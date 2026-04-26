// Vat scheduler: cooperative concurrency with fuel-based preemption.
//
// Architecture:
//   - vat 0 is the init vat (the rust runtime). it spawns everything.
//   - the REPL is just another vat — not privileged.
//   - capability vats (Console, Clock, etc.) are also just vats.
//   - all cross-vat sends return Acts.
//   - the scheduler drains outboxes and delivers messages.
//
// Vat itself (the per-context execution environment) lives in vat.rs.

use moof_core::heap::{Heap, OutgoingMessage, SpawnRequest};
use moof_core::value::Value;
use crate::vat::{Vat, Message};
use crate::capability::CapabilityPlugin;

/// Merge two delta objects: base slots + override slots → new object.
fn merge_deltas(heap: &mut Heap, base: Value, overrides: Value) -> Value {
    let base_id = match base.as_any_object() {
        Some(id) => id,
        None => return overrides,
    };
    let over_id = match overrides.as_any_object() {
        Some(id) => id,
        None => return base,
    };

    // collect all slots from base
    let base_names = heap.get(base_id).slot_names();
    let mut names: Vec<u32> = base_names.clone();
    let mut vals: Vec<Value> = base_names.iter()
        .map(|n| heap.get(base_id).slot_get(*n).unwrap_or(Value::NIL))
        .collect();

    // apply overrides
    let over_names = heap.get(over_id).slot_names();
    for name in &over_names {
        let val = heap.get(over_id).slot_get(*name).unwrap_or(Value::NIL);
        if let Some(i) = names.iter().position(|n| *n == *name) {
            vals[i] = val;
        } else {
            names.push(*name);
            vals.push(val);
        }
    }

    heap.make_object_with_slots(Value::NIL, names, vals)
}

/// A pending Act resolution: result from a cross-vat send.
struct ActResolution {
    vat_id: u32,       // which vat the Act lives in
    act_id: u32,       // Act object ID
    result: Value,     // the resolved value (in the Act's vat heap)
    is_error: bool,
}

/// Info about a loaded dynamic plugin.
pub struct LoadedPlugin {
    pub name: String,
    pub path: std::path::PathBuf,
    pub vat_id: u32,
    pub obj_id: u32,
}

/// The scheduler: manages vats and runs them.
pub struct Scheduler {
    pub vats: Vec<Vat>,
    pub fuel_per_turn: u64,
    next_vat_id: u32,
    _plugin_handles: Vec<Box<dyn CapabilityPlugin>>,
    pub loaded_plugins: Vec<LoadedPlugin>,

    /// Type plugins registered on every new vat's heap at spawn
    /// time. Populated by moof-cli from the manifest (both builtins
    /// and dylib-loaded plugins go here). Dylib-loaded plugins own
    /// their `libloading::Library` internally, so the dylib stays
    /// resident as long as the Box<dyn Plugin> does.
    type_plugins: Vec<Box<dyn moof_core::Plugin>>,

    /// Source snippets eval'd on each new vat after plugin registration.
    /// Lets every cross-vat spawn have the same stdlib loaded.
    pub bootstrap_sources: Vec<String>,

    /// Origin labels paired with `bootstrap_sources` (same length).
    /// Used so closures compiled from bootstrap files carry their
    /// actual file path as origin, not "<eval>". Stays empty when
    /// sources are set without paths.
    pub bootstrap_source_labels: Vec<String>,
}

// Drop order: vats → capability plugin handles → type plugins.
// Vats' heaps hold Box<dyn Fn> closures and foreign payloads whose
// drop code lives in the plugin dylibs, so the dylibs must outlive
// every vat. Rust drops struct fields in declaration order — vats
// is declared before _plugin_handles which is declared before
// type_plugins, so default order already matches, but we keep this
// explicit Drop impl so the invariant is documented and any future
// field-order shuffling doesn't silently become a segfault.
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.vats.clear();
        self._plugin_handles.clear();
        self.type_plugins.clear();
    }
}

impl Scheduler {
    pub fn new(fuel_per_turn: u64) -> Self {
        Scheduler {
            vats: Vec::new(),
            fuel_per_turn,
            next_vat_id: 0,
            _plugin_handles: Vec::new(),
            loaded_plugins: Vec::new(),
            type_plugins: Vec::new(),
            bootstrap_sources: Vec::new(),
            bootstrap_source_labels: Vec::new(),
        }
    }

    /// Install the list of type plugins that every new vat will get.
    /// moof-cli calls this at startup with the resolved manifest types
    /// (builtin or dylib-loaded). Replaces the whole list — call once
    /// after manifest parsing.
    pub fn install_type_plugins(&mut self, plugins: Vec<Box<dyn moof_core::Plugin>>) {
        self.type_plugins = plugins;
    }

    /// Ordered list of source files to eval on every newly-spawned vat
    /// after type plugins register. moof-cli populates from manifest's
    /// [sources.files]. Passed as snippets, not paths, so vats spawned
    /// from other threads don't need filesystem access.
    pub fn set_bootstrap_sources(&mut self, sources: Vec<String>) {
        self.bootstrap_sources = sources;
        self.bootstrap_source_labels.clear();  // unlabelled
    }

    /// Install bootstrap sources paired with their origin labels
    /// (typically file paths from the manifest). Closures compiled
    /// from these sources carry their file path as origin, so
    /// inspectors can point users back at the source file.
    pub fn set_bootstrap_sources_with_labels(&mut self, sources: Vec<(String, String)>) {
        self.bootstrap_sources = sources.iter().map(|(s, _)| s.clone()).collect();
        self.bootstrap_source_labels = sources.into_iter().map(|(_, l)| l).collect();
    }

    /// Spawn a new vat and initialize it: register installed type
    /// plugins, eval bootstrap sources. Returns the vat id.
    pub fn spawn_vat(&mut self) -> u32 {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        let mut vat = Vat::new(id);
        for plugin in &self.type_plugins {
            plugin.register(&mut vat.heap);
        }
        for (i, source) in self.bootstrap_sources.iter().enumerate() {
            let label = self.bootstrap_source_labels.get(i)
                .map(|s| s.as_str())
                .unwrap_or("<bootstrap>");
            let _ = vat.eval_source_with_origin(source, label);
        }
        self.vats.push(vat);
        id
    }

    /// Spawn a bare vat with no plugins or bootstrap. Used for the
    /// init vat that doesn't need userland.
    pub fn spawn_bare_vat(&mut self) -> u32 {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        self.vats.push(Vat::new(id));
        id
    }

    /// Spawn a capability vat from a `CapabilityPlugin`. Returns
    /// `(vat_id, root_object_id)`. Capability vats are bare: they
    /// don't get userland plugins or bootstrap — just the heap +
    /// whatever the capability's setup installs.
    ///
    /// **Critical**: the plugin Box is MOVED into `_plugin_handles`
    /// and kept alive for the scheduler's lifetime. The cap's setup()
    /// installed native closures in the vat's heap whose vtable
    /// pointers live in the plugin's dylib — if we dropped the Box
    /// now, the dylib would unload and those closures would become
    /// dangling, crashing at drop time (or any dispatch before then).
    pub fn spawn_capability(&mut self, cap: Box<dyn crate::capability::CapabilityPlugin>) -> (u32, u32) {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        let mut vat = Vat::new(id);
        let obj_id = cap.setup(&mut vat);
        self.vats.push(vat);
        self._plugin_handles.push(cap);
        (id, obj_id)
    }

    /// Load a dynamic plugin from a shared library and spawn it as a capability vat.
    /// Returns (capability_name, vat_id, root_object_id).
    pub fn load_plugin(&mut self, path: &std::path::Path) -> Result<(String, u32, u32), String> {
        // check if already loaded
        if let Some(existing) = self.loaded_plugins.iter().find(|p| p.path == path) {
            return Err(format!("plugin '{}' already loaded from {:?}", existing.name, existing.path));
        }

        let plugin = crate::dynload::DynCapabilityPlugin::load(path)?;
        let name = plugin.name().to_string();
        let plugin_path = plugin.path().to_path_buf();
        // spawn_capability moves the Box into _plugin_handles — so
        // the dylib stays loaded while its natives are in use.
        let (vat_id, obj_id) = self.spawn_capability(Box::new(plugin));

        self.loaded_plugins.push(LoadedPlugin {
            name: name.clone(),
            path: plugin_path,
            vat_id,
            obj_id,
        });
        Ok((name, vat_id, obj_id))
    }

    /// Get paths of all loaded dynamic plugins (for image persistence).
    pub fn plugin_paths(&self) -> Vec<&std::path::Path> {
        self.loaded_plugins.iter().map(|p| p.path.as_path()).collect()
    }

    /// Reload all plugins from saved paths. Used on image restore.
    pub fn reload_plugins(&mut self, paths: &[std::path::PathBuf], repl_vat_id: u32) {
        for path in paths {
            match self.load_plugin(path) {
                Ok((name, vat_id, obj_id)) => {
                    let url = format!("moof:/caps/{name}");
                    let farref = self.create_farref(repl_vat_id, vat_id, obj_id, Some(&url));
                    let sym = self.vat_mut(repl_vat_id).heap.intern(&name);
                    self.vat_mut(repl_vat_id).heap.env_def(sym, farref);
                    eprintln!("  reloaded plugin '{name}'");
                }
                Err(e) => eprintln!("  ~ failed to reload plugin from {path:?}: {e}"),
            }
        }
    }

    /// Create a FarRef in a vat pointing to an object in another vat.
    /// The `url` argument is the canonical identity of the target
    /// resource — a string in the `moof:<path>` form. Every FarRef
    /// the runtime hands out carries a URL: capabilities use
    /// `moof:/caps/<name>`, user defservers use
    /// `moof:/vats/<id>/objs/<id>`. Image restart re-resolves the
    /// URL to refresh `(vat_id, obj_id)`, which are session-local.
    ///
    /// Pass `None` only for legacy / ad-hoc FarRefs that don't
    /// belong to a stable resource; those won't survive restart.
    pub fn create_farref(
        &mut self,
        in_vat: u32,
        target_vat: u32,
        target_obj: u32,
        url: Option<&str>,
    ) -> Value {
        let vat = self.vat_mut(in_vat);
        let farref_proto = vat.heap.lookup_type("FarRef");
        let tgt_vat_sym = vat.heap.intern("__target_vat");
        let tgt_obj_sym = vat.heap.intern("__target_obj");
        let mut names = vec![tgt_vat_sym, tgt_obj_sym];
        let mut values = vec![
            Value::integer(target_vat as i64),
            Value::integer(target_obj as i64),
        ];
        if let Some(u) = url {
            let url_sym = vat.heap.intern("url");
            let url_val = vat.heap.alloc_string(u);
            names.push(url_sym);
            values.push(url_val);
        }
        vat.heap.make_object_with_slots(farref_proto, names, values)
    }

    /// Get a reference to a vat by ID.
    pub fn vat(&self, id: u32) -> &Vat {
        &self.vats[id as usize]
    }

    /// Get a mutable reference to a vat by ID.
    pub fn vat_mut(&mut self, id: u32) -> &mut Vat {
        &mut self.vats[id as usize]
    }

    /// Evaluate source in a specific vat, then drain all pending work.
    pub fn eval_in_vat(&mut self, vat_id: u32, source: &str) -> Result<Value, String> {
        let result = self.vat_mut(vat_id).eval_source(source)?;
        self.drain();
        Ok(result)
    }

    /// Drain all pending cross-vat work: spawn requests, outbox messages,
    /// Act resolutions. Runs until quiescent.
    pub fn drain(&mut self) {
        // loop until no more work
        for _ in 0..100_000 {  // safety bound
            let mut did_work = false;

            // 0. process ready Acts (continuations on already-resolved Acts)
            let mut ready: Vec<(u32, u32)> = Vec::new();  // (vat_id, act_id)
            for vat in &mut self.vats {
                for act_id in vat.heap.ready_acts.drain(..) {
                    ready.push((vat.id, act_id));
                }
            }
            for (vat_id, act_id) in ready {
                did_work = true;
                let vat = self.vat_mut(vat_id);
                let cont_fn_sym = vat.heap.intern("__cont_fn");
                let cont_val_sym = vat.heap.intern("__cont_val");
                let cont_fn = vat.heap.get(act_id).slot_get(cont_fn_sym);
                let cont_val = vat.heap.get(act_id).slot_get(cont_val_sym);
                if let (Some(f), Some(val)) = (cont_fn, cont_val) {
                    // check if this ready-act has a merge_delta (from Update then:)
                    let merge_delta_sym = vat.heap.intern("__merge_delta");
                    let merge_delta = vat.heap.get(act_id).handler_get(merge_delta_sym);

                    match vat.vm.call_value(&mut vat.heap, f, &[val]) {
                        Ok(result) => {
                            if let Some(our_delta) = merge_delta {
                                // wrap result: if result is an Update, merge deltas.
                                // if result is a plain value, create Update with our delta.
                                let vat = self.vat_mut(vat_id);
                                let update_proto = vat.heap.lookup_type("Update");
                                let is_update = !update_proto.is_nil()
                                    && vat.heap.prototype_of(result) == update_proto;

                                let final_val = if is_update {
                                    // merge: our delta + result's delta
                                    let result_id = result.as_any_object().unwrap();
                                    let delta_sym = vat.heap.intern("__delta");
                                    let reply_sym = vat.heap.intern("__reply");
                                    let result_delta = vat.heap.get(result_id)
                                        .slot_get(delta_sym).unwrap_or(Value::NIL);
                                    let result_reply = vat.heap.get(result_id)
                                        .slot_get(reply_sym).unwrap_or(Value::NIL);
                                    // merge: start with our_delta slots, override with result_delta slots
                                    let merged = merge_deltas(&mut vat.heap, our_delta, result_delta);
                                    vat.heap.make_object_with_slots(
                                        update_proto,
                                        vec![delta_sym, reply_sym],
                                        vec![merged, result_reply],
                                    )
                                } else {
                                    // plain value — wrap with our delta
                                    let delta_sym = vat.heap.intern("__delta");
                                    let reply_sym = vat.heap.intern("__reply");
                                    vat.heap.make_object_with_slots(
                                        update_proto,
                                        vec![delta_sym, reply_sym],
                                        vec![our_delta, result],
                                    )
                                };
                                self.resolve_act(vat_id, act_id, final_val, false);
                            } else {
                                // check for __wrap_ok (from Ok map:)
                                let vat = self.vat_mut(vat_id);
                                let wrap_ok_sym = vat.heap.intern("__wrap_ok");
                                let should_wrap = vat.heap.get(act_id).handler_get(wrap_ok_sym).is_some();
                                if should_wrap {
                                    let ok_proto = vat.heap.lookup_type("Ok");
                                    let val_sym = vat.heap.intern("value");
                                    let wrapped = vat.heap.make_object_with_slots(
                                        ok_proto, vec![val_sym], vec![result]);
                                    self.resolve_act(vat_id, act_id, wrapped, false);
                                } else {
                                    self.resolve_act(vat_id, act_id, result, false);
                                }
                            }
                        }
                        Err(e) => {
                            let err_val = self.vat_mut(vat_id).heap.make_error(&e);
                            self.resolve_act(vat_id, act_id, err_val, true);
                        }
                    }
                }
            }

            // 1. collect spawn requests from all vats
            let mut spawns: Vec<(u32, SpawnRequest)> = Vec::new();
            for vat in &mut self.vats {
                for req in vat.heap.spawn_queue.drain(..) {
                    spawns.push((vat.id, req));
                }
            }

            // 2. process spawn requests
            for (parent_vat_id, req) in spawns {
                did_work = true;

                // create new vat with bootstrap
                let child_id = self.spawn_vat();

                let result = match req.payload {
                    moof_core::heap::SpawnPayload::Source(ref source) => {
                        self.vat_mut(child_id).eval_source(source)
                    }
                    moof_core::heap::SpawnPayload::Closure(closure_val) => {
                        self.run_closure_in_vat(closure_val, parent_vat_id, child_id, &[])
                    }
                    moof_core::heap::SpawnPayload::ClosureWithArgs(closure_val, ref args) => {
                        let args = args.clone();
                        self.run_closure_in_vat(closure_val, parent_vat_id, child_id, &args)
                    }
                };

                // resolve the Act in the parent vat
                match result {
                    Ok(val) => {
                        if req.serve {
                            // serve mode: return a FarRef to the object in the child vat
                            if let Some(obj_id) = val.as_any_object() {
                                // user-defserver FarRef — URL addresses the specific
                                // (vat, obj) pair. resolver will hand back a live
                                // handle on restart (if the defserver's been spawned).
                                let url = format!("moof:/vats/{child_id}/objs/{obj_id}");
                                let farref = self.create_farref(parent_vat_id, child_id, obj_id, Some(&url));
                                self.resolve_act(parent_vat_id, req.act_id, farref, false);
                            } else {
                                let err = self.vat_mut(parent_vat_id).heap
                                    .make_error("serve: closure must return an object");
                                self.resolve_act(parent_vat_id, req.act_id, err, true);
                            }
                        } else {
                            // compute mode: copy the result value
                            let copied_val = self.copy_value_across(val, child_id, parent_vat_id);
                            self.resolve_act(parent_vat_id, req.act_id, copied_val, false);
                        }
                    }
                    Err(e) => {
                        let err_val = self.vat_mut(parent_vat_id).heap.make_error(&e);
                        self.resolve_act(parent_vat_id, req.act_id, err_val, true);
                    }
                }
            }

            // 3. collect outgoing messages from all vats
            let mut outgoing: Vec<(u32, OutgoingMessage)> = Vec::new();
            for vat in &mut self.vats {
                for msg in vat.heap.outbox.drain(..) {
                    outgoing.push((vat.id, msg));
                }
            }

            // 4. deliver messages and collect resolutions
            let mut resolutions: Vec<ActResolution> = Vec::new();
            for (source_vat_id, out_msg) in outgoing {
                did_work = true;
                let target_vat_id = out_msg.target_vat_id;

                // re-intern selector from source vat into target vat
                let sel_name = self.vat(source_vat_id).heap.symbol_name(out_msg.selector).to_string();
                let target_sel = self.vat_mut(target_vat_id).heap.intern(&sel_name);

                // copy args from source heap to target heap
                let copied_args: Vec<Value> = out_msg.args.iter()
                    .map(|v| self.copy_value_across(*v, source_vat_id, target_vat_id))
                    .collect();

                let msg = Message {
                    receiver_id: out_msg.target_obj_id,
                    selector: target_sel,
                    args: copied_args,
                    reply_vat_id: source_vat_id,
                    reply_act_id: out_msg.act_id,
                };

                let result = self.vat_mut(target_vat_id).dispatch_message(&msg);
                match result {
                    Ok(val) => {
                        // check what the handler returned
                        let vat = self.vat_mut(target_vat_id);
                        let is_act = Self::is_act(&vat.heap, val);
                        let reply = if is_act {
                            // handler returned an Act (IO + maybe Update).
                            // we need to wait for the Act to resolve, then
                            // check for Update and apply the delta.
                            // store the receiver_id and caller info on the Act
                            // so the drain loop can finalize it later.
                            let act_id = val.as_any_object().unwrap();
                            let srv_recv_sym = vat.heap.intern("__server_recv");
                            let srv_caller_vat_sym = vat.heap.intern("__server_caller_vat");
                            let srv_caller_act_sym = vat.heap.intern("__server_caller_act");
                            vat.heap.get_mut(act_id).handler_set(srv_recv_sym,
                                Value::integer(msg.receiver_id as i64));
                            vat.heap.get_mut(act_id).handler_set(srv_caller_vat_sym,
                                Value::integer(source_vat_id as i64));
                            vat.heap.get_mut(act_id).handler_set(srv_caller_act_sym,
                                Value::integer(out_msg.act_id as i64));
                            // don't resolve the caller's Act yet — the drain loop
                            // will handle it when this Act resolves
                            continue;
                        } else {
                            self.process_handler_result(target_vat_id, msg.receiver_id, val)
                        };
                        // copy the reply to the source vat
                        let copied = self.copy_value_across(reply, target_vat_id, source_vat_id);
                        resolutions.push(ActResolution {
                            vat_id: source_vat_id,
                            act_id: out_msg.act_id,
                            result: copied,
                            is_error: false,
                        });
                    }
                    Err(e) => {
                        let err_val = self.vat_mut(source_vat_id).heap.make_error(&e);
                        resolutions.push(ActResolution {
                            vat_id: source_vat_id,
                            act_id: out_msg.act_id,
                            result: err_val,
                            is_error: true,
                        });
                    }
                }
            }

            // 5. resolve Acts
            for res in resolutions {
                did_work = true;
                self.resolve_act(res.vat_id, res.act_id, res.result, res.is_error);
            }

            if !did_work { break; }
        }
    }

    /// Copy a closure from the parent vat and run it in the child vat.
    /// Copies the ClosureDesc (bytecode + constants) and any captured values.
    fn run_closure_in_vat(&mut self, closure_val: Value, from_vat_id: u32, to_vat_id: u32, args: &[Value]) -> Result<Value, String> {
        let new_closure = self.copy_closure_across(closure_val, from_vat_id, to_vat_id)
            .ok_or("spawn: failed to migrate closure")?;

        // copy args across heaps
        let mut new_args: Vec<Value> = Vec::new();
        for arg in args {
            new_args.push(self.copy_value_across(*arg, from_vat_id, to_vat_id));
        }

        // call the closure with args
        let vat = self.vat_mut(to_vat_id);
        vat.vm.call_value(&mut vat.heap, new_closure, &new_args)
    }

    /// Migrate a closure from one vat to another: copy its ClosureDescs,
    /// remap captures, install descs into the target VM, and build a new
    /// closure value there. Returns None if the value isn't a closure or
    /// migration fails. Used both for spawning child vats and for crossing
    /// closures as message args / captured values.
    fn copy_closure_across(&mut self, closure_val: Value, from_vat_id: u32, to_vat_id: u32) -> Option<Value> {
        let from_vat = &self.vats[from_vat_id as usize];
        let (code_idx, _) = from_vat.heap.as_closure(closure_val)?;
        if code_idx >= from_vat.vm.closure_descs_ref().len() {
            return None;
        }

        let src_desc = &from_vat.vm.closure_descs_ref()[code_idx];
        let src_chunk_arity = src_desc.chunk.arity;
        // structurally check: was the source closure an applicative?
        // (has __underlying slot.) we copy that bit so the target heap
        // builds a structurally-equivalent closure.
        let src_is_applicative = !from_vat.heap.as_closure(closure_val)
            .map(|(_, is_op)| is_op).unwrap_or(true);
        let src_desc_base = src_desc.desc_base;

        // clone all descs from desc_base onwards
        let src_descs: Vec<_> = from_vat.vm.closure_descs_ref()[src_desc_base..]
            .iter()
            .map(|d| {
                let const_vals: Vec<Value> = d.chunk.constants.iter()
                    .map(|&bits| Value::from_bits(bits))
                    .collect();
                (d.chunk.clone(), d.param_names.clone(),
                 d.capture_names.clone(), d.capture_parent_regs.clone(),
                 d.capture_local_regs.clone(), d.capture_values.clone(),
                 d.rest_param_reg, const_vals, d.source.clone())
            })
            .collect();

        let captures: Vec<(String, Value)> = from_vat.heap.closure_captures(closure_val)
            .iter()
            .map(|(sym, val)| {
                (from_vat.heap.symbol_name(*sym).to_string(), *val)
            })
            .collect();

        // remap captured values into target heap
        let mut new_captures: Vec<(u32, Value)> = Vec::new();
        for (sym_name, val) in &captures {
            let new_sym = self.vat_mut(to_vat_id).heap.intern(sym_name);
            let new_val = self.copy_value_across(*val, from_vat_id, to_vat_id);
            new_captures.push((new_sym, new_val));
        }

        // build new descs with remapped constants
        let target_base = self.vat(to_vat_id).vm.closure_descs_ref().len();
        let new_code_idx = target_base + (code_idx - src_desc_base);

        for (mut chunk, param_names, cap_names, cap_parent, cap_local, cap_vals, rest_reg, const_vals, src) in src_descs {
            chunk.constants = const_vals.iter()
                .map(|v| self.copy_value_across(*v, from_vat_id, to_vat_id).to_bits())
                .collect();

            // register the source record in the target vat's heap at
            // the code_idx this desc will land at; keeps [closure source]
            // functional across migrations.
            if let Some(s) = src.clone() {
                let target_idx = self.vat(to_vat_id).vm.closure_descs_ref().len();
                self.vat_mut(to_vat_id).heap.register_closure_source(target_idx, s);
            }

            let desc = moof_lang::lang::compiler::ClosureDesc {
                chunk,
                param_names,
                capture_names: cap_names,
                capture_parent_regs: cap_parent,
                capture_local_regs: cap_local,
                capture_values: cap_vals,
                desc_base: target_base,
                rest_param_reg: rest_reg,
                source: src,
            };
            self.vat_mut(to_vat_id).vm.add_closure_desc(desc);
        }

        let new_closure = self.vat_mut(to_vat_id).heap.make_closure(
            new_code_idx,
            src_chunk_arity,
            &new_captures,
        );
        // restore applicative status if the source was wrapped.
        if src_is_applicative {
            self.vat_mut(to_vat_id).heap.set_closure_underlying(new_closure, new_closure);
        }
        Some(new_closure)
    }

    /// Copy a value from one vat's heap to another.
    /// For now, only handles immediate values (int, symbol, bool, nil, float).
    /// Heap objects will need deep copy later.
    fn copy_value_across(&mut self, val: Value, _from_vat: u32, to_vat: u32) -> Value {
        // immediate values (int, bool, nil, float) are bitwise-identical across heaps
        if val.is_nil() || val.is_true() || val.is_false()
            || val.as_integer().is_some() || val.is_float() {
            return val;
        }
        // symbols need re-interning in the target heap
        if let Some(sym_id) = val.as_symbol() {
            let name = self.vat(_from_vat).heap.symbol_name(sym_id).to_string();
            let new_sym = self.vat_mut(to_vat).heap.intern(&name);
            return Value::symbol(new_sym);
        }
        // heap objects
        if let Some(obj_id) = val.as_any_object() {
            // closures live as Generals now — detect via __code_idx slot and
            // route through copy_closure_across so the bytecode desc migrates
            // too (the slot's integer code_idx is meaningless in the target
            // vat's closure_descs list without it).
            if self.vat(_from_vat).heap.as_closure(val).is_some() {
                if let Some(new_closure) = self.copy_closure_across(val, _from_vat, to_vat) {
                    return new_closure;
                }
                eprintln!("  ~ warning: failed to migrate closure across vats");
                return Value::NIL;
            }
            let from_heap = &self.vat(_from_vat).heap;
            // Text is a foreign type now, but it's by far the most common
            // cross-vat payload (println: args, error messages, source
            // for spawn). Handle it directly for speed — skips the full
            // foreign-registry round-trip since we have a native path.
            if let Some(s) = from_heap.get_string(obj_id) {
                let s = s.to_string();
                return self.vat_mut(to_vat).heap.alloc_string(&s);
            }
            let src_obj = from_heap.get(obj_id);
            let names: Vec<String> = src_obj.slot_names.iter()
                .map(|s| from_heap.symbol_name(*s).to_string())
                .collect();
            let vals: Vec<Value> = src_obj.slot_values.clone();
            let foreign = src_obj.foreign.clone();  // Arc + id, cheap

            // Translate foreign payload: clone across using the
            // source vtable, resolve the target type_id by name,
            // and pick up the target's prototype by prototype_name
            // (session-local proto Values never match across
            // heaps; the semantic name is the stable link).
            let foreign_translated: Option<(moof_core::foreign::ForeignData, Option<Value>)> = match foreign {
                Some(fd) => {
                    let src_vt = from_heap.foreign_registry().vtable(fd.type_id)
                        .expect("source foreign type_id has no vtable");
                    let src_name = src_vt.id.clone();
                    let proto_name: &str = (src_vt.prototype_name)();
                    let clone_across_fn = src_vt.clone_across;
                    let new_payload: std::sync::Arc<dyn std::any::Any + Send + Sync> =
                        clone_across_fn(&*fd.payload, &mut |v| {
                            self.copy_value_across(v, _from_vat, to_vat)
                        });
                    let target_id = self.vat(to_vat).heap.foreign_registry()
                        .resolve(&src_name);
                    match target_id {
                        Ok(tid) => {
                            let target_proto = self.vat(to_vat).heap.lookup_type(proto_name);
                            let proto_opt = if target_proto.is_nil() { None } else { Some(target_proto) };
                            Some((moof_core::foreign::ForeignData {
                                type_id: tid,
                                payload: new_payload,
                            }, proto_opt))
                        }
                        Err(e) => {
                            eprintln!("  ~ cross-vat foreign copy failed: {e}");
                            return Value::NIL;
                        }
                    }
                }
                None => None,
            };

            let is_farref = names.iter().any(|n| n == "__target_vat");
            let new_names: Vec<u32> = names.iter()
                .map(|n| self.vat_mut(to_vat).heap.intern(n))
                .collect();
            let new_vals: Vec<Value> = vals.iter()
                .map(|v| self.copy_value_across(*v, _from_vat, to_vat))
                .collect();
            let (foreign_data, foreign_proto) = match foreign_translated {
                Some((fd, p)) => (Some(fd), p),
                None => (None, None),
            };
            let proto = foreign_proto.unwrap_or_else(|| {
                if is_farref {
                    self.vat(to_vat).heap.lookup_type("FarRef")
                } else {
                    self.vat(to_vat).heap.type_protos[moof_core::heap::PROTO_OBJ]
                }
            });
            let to_heap = &mut self.vat_mut(to_vat).heap;
            let new_val = to_heap.alloc_val(moof_core::object::HeapObject {
                proto,
                slot_names: new_names,
                slot_values: new_vals,
                handlers: Vec::new(),
                foreign: foreign_data,
            });
            return new_val;
        }
        val
    }

    /// Process a handler's return value. If it's an Update, apply the delta
    /// to the receiver and return the reply. Otherwise return the value as-is.
    fn process_handler_result(&mut self, vat_id: u32, receiver_id: u32, val: Value) -> Value {
        let vat = self.vat_mut(vat_id);
        let update_proto = vat.heap.lookup_type("Update");
        if update_proto.is_nil() { return val; }

        // check if val is an Update
        let proto = vat.heap.prototype_of(val);
        if proto != update_proto { return val; }

        // it's an Update — extract delta and reply
        let delta_sym = vat.heap.intern("__delta");
        let reply_sym = vat.heap.intern("__reply");
        let val_id = val.as_any_object().unwrap();
        let delta = vat.heap.get(val_id).slot_get(delta_sym).unwrap_or(Value::NIL);
        let reply = vat.heap.get(val_id).slot_get(reply_sym).unwrap_or(Value::NIL);

        // apply delta: use with: to create merged state, then replace slots
        if let Some(delta_id) = delta.as_any_object() {
            let delta_names = vat.heap.get(delta_id).slot_names();
            let delta_vals: Vec<Value> = delta_names.iter()
                .map(|n| vat.heap.get(delta_id).slot_get(*n).unwrap_or(Value::NIL))
                .collect();
            for (name, val) in delta_names.iter().zip(delta_vals.iter()) {
                vat.heap.get_mut(receiver_id).slot_set(*name, *val);
            }
        }
        // note: slot_set is used here as an internal scheduler mechanism,
        // not as a user-facing mutation. the server object is only modified
        // between message deliveries, atomically.

        reply
    }

    /// Check if a value is an Act (has PROTO_ACT as prototype).
    fn is_act(heap: &Heap, val: Value) -> bool {
        let act_proto = heap.lookup_type("Act");
        if act_proto.is_nil() { return false; }
        let proto = heap.prototype_of(val);
        proto == act_proto
    }

    /// Resolve an Act: set state to resolved, store result, run continuations.
    /// If the final value is itself an Act (monadic bind), set up forwarding.
    fn resolve_act(&mut self, vat_id: u32, act_id: u32, result: Value, is_error: bool) {
        // if we're asked to resolve with another Act, don't drain our chain
        // yet — forward to the inner Act so our chain runs with the inner's
        // actual resolved value. without this, the chain would see the
        // pending Act object as current_val.
        {
            let vat = self.vat_mut(vat_id);
            if !is_error && Self::is_act(&vat.heap, result) {
                let inner_act_id = result.as_any_object().unwrap();
                if inner_act_id != act_id {
                    self.setup_forwarding(vat_id, inner_act_id, act_id);
                    return;
                }
            }
        }
        let vat = self.vat_mut(vat_id);
        let state_sym = vat.heap.intern("__state");
        let result_sym = vat.heap.intern("__result");
        let chain_sym = vat.heap.intern("__chain");

        let mut current_val = result;
        let mut is_err = is_error;

        // run continuation chain (stored in reverse — built by cons prepending)
        let chain = vat.heap.get(act_id).slot_get(chain_sym).unwrap_or(Value::NIL);
        if !chain.is_nil() && !is_error {
            // clear the chain FIRST — prevents re-execution on forwarded resolution
            let conts: Vec<Value> = vat.heap.list_to_vec(chain);
            vat.heap.get_mut(act_id).slot_set(chain_sym, Value::NIL);
            let conts: Vec<Value> = conts.into_iter().rev().collect();

            for (i, cont) in conts.iter().enumerate() {
                let vat = self.vat_mut(vat_id);
                match vat.vm.call_value(&mut vat.heap, *cont, &[current_val]) {
                    Ok(val) => {
                        let vat = self.vat_mut(vat_id);
                        if Self::is_act(&vat.heap, val) {
                            // continuation returned an Act — forward remaining
                            let inner_act_id = val.as_any_object().unwrap();
                            let remaining = &conts[i+1..];
                            if !remaining.is_empty() {
                                let inner_chain = vat.heap.get(inner_act_id)
                                    .slot_get(chain_sym).unwrap_or(Value::NIL);
                                let mut new_chain = inner_chain;
                                // drain order matches chain-list-reversed. to get drain
                                // order [r0, r1, r2] we want chain = (r2 . r1 . r0 . rest),
                                // i.e. cons in forward order so the earliest-to-drain ends
                                // up deepest in the cons spine.
                                for r_cont in remaining.iter() {
                                    new_chain = vat.heap.cons(*r_cont, new_chain);
                                }
                                vat.heap.get_mut(inner_act_id).slot_set(chain_sym, new_chain);
                            }
                            // set up forwarding: inner → outer
                            self.setup_forwarding(vat_id, inner_act_id, act_id);
                            return; // outer Act stays pending until inner resolves
                        }
                        current_val = val;
                    }
                    Err(e) => {
                        eprintln!("  ~ act continuation error: {e}");
                        let vat = self.vat_mut(vat_id);
                        current_val = vat.heap.make_error(&e);
                        is_err = true;
                        break;
                    }
                }
            }
        }

        // check if the final value is itself an Act (even without a chain)
        {
            let vat = self.vat_mut(vat_id);
            if !is_err && Self::is_act(&vat.heap, current_val) {
                let inner_act_id = current_val.as_any_object().unwrap();
                self.setup_forwarding(vat_id, inner_act_id, act_id);
                return;
            }
        }

        // resolve: set state + result
        let vat = self.vat_mut(vat_id);
        let resolved_sym = if is_err {
            vat.heap.intern("failed")
        } else {
            vat.heap.intern("resolved")
        };
        vat.heap.get_mut(act_id).slot_set(state_sym, Value::symbol(resolved_sym));
        vat.heap.get_mut(act_id).slot_set(result_sym, current_val);

        // check if this Act is from a server handler (has __server_recv metadata).
        // if so, process the resolved value as a handler result (check for Update,
        // apply delta), then resolve the caller's Act with the reply.
        let srv_recv_sym = vat.heap.intern("__server_recv");
        let srv_info = vat.heap.get(act_id).handler_get(srv_recv_sym)
            .and_then(|v| v.as_integer())
            .map(|recv_id| {
                let caller_vat_sym = vat.heap.intern("__server_caller_vat");
                let caller_act_sym = vat.heap.intern("__server_caller_act");
                let caller_vat = vat.heap.get(act_id).handler_get(caller_vat_sym)
                    .and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                let caller_act = vat.heap.get(act_id).handler_get(caller_act_sym)
                    .and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                (recv_id as u32, caller_vat, caller_act)
            });
        if let Some((recv_id, caller_vat, caller_act)) = srv_info {
            // process the resolved value — check for Update
            let reply = self.process_handler_result(vat_id, recv_id, current_val);
            // copy reply to the caller's vat and resolve their Act
            let copied = self.copy_value_across(reply, vat_id, caller_vat);
            self.resolve_act(caller_vat, caller_act, copied, is_err);
            return;
        }

        // check if this Act has a forward link (inner Act → outer Act)
        let vat = self.vat_mut(vat_id);
        let fwd_sym = vat.heap.intern("__forward_to");
        let fwd_info = vat.heap.get(act_id).handler_get(fwd_sym)
            .and_then(|v| v.as_integer())
            .map(|outer_id| (outer_id as u32, current_val, is_err));
        if let Some((outer_id, val, err)) = fwd_info {
            self.resolve_act(vat_id, outer_id, val, err);
        }
    }

    /// Set up forwarding: when inner_act resolves, resolve outer_act too.
    fn setup_forwarding(&mut self, vat_id: u32, inner_act_id: u32, outer_act_id: u32) {
        let vat = self.vat_mut(vat_id);
        let state_sym = vat.heap.intern("__state");
        let result_sym = vat.heap.intern("__result");
        let pending_sym = vat.heap.intern("pending");
        let fwd_sym = vat.heap.intern("__forward_to");

        // set outer Act back to pending
        vat.heap.get_mut(outer_act_id).slot_set(state_sym, Value::symbol(pending_sym));
        vat.heap.get_mut(outer_act_id).slot_set(result_sym, Value::NIL);

        // check if inner is already resolved
        let resolved_sym = vat.heap.intern("resolved");
        let failed_sym = vat.heap.intern("failed");
        let inner_state = vat.heap.get(inner_act_id).slot_get(state_sym);
        let inner_resolved = inner_state == Some(Value::symbol(resolved_sym));
        let inner_failed = inner_state == Some(Value::symbol(failed_sym));

        if inner_resolved || inner_failed {
            // inner already done — resolve outer immediately
            let inner_result = vat.heap.get(inner_act_id).slot_get(result_sym).unwrap_or(Value::NIL);
            self.resolve_act(vat_id, outer_act_id, inner_result, inner_failed);
        } else {
            // inner still pending — set forwarding link
            vat.heap.get_mut(inner_act_id).handler_set(fwd_sym, Value::integer(outer_act_id as i64));
        }
    }
}
