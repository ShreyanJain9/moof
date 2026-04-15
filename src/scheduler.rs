// Vat scheduler: cooperative concurrency with fuel-based preemption.
//
// Architecture:
//   - vat 0 is the init vat (the rust runtime). it spawns everything.
//   - the REPL is just another vat — not privileged.
//   - capability vats (Console, Clock, etc.) are also just vats.
//   - all cross-vat sends return Acts.
//   - the scheduler drains outboxes and delivers messages.

use std::collections::VecDeque;
use crate::heap::{Heap, OutgoingMessage, SpawnRequest};
use crate::vm::VM;
use crate::value::Value;
use crate::lang::compiler::Compiler;

/// A message queued for delivery to a vat.
pub struct Message {
    pub receiver_id: u32,        // object ID in the target vat
    pub selector: u32,           // method selector (symbol in target vat)
    pub args: Vec<Value>,        // values in the target vat's heap
    pub reply_vat_id: u32,       // which vat to resolve the Act in
    pub reply_act_id: u32,       // Act object ID in the reply vat
}

/// A pending Act resolution: result from a cross-vat send.
struct ActResolution {
    vat_id: u32,       // which vat the Act lives in
    act_id: u32,       // Act object ID
    result: Value,     // the resolved value (in the Act's vat heap)
    is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VatStatus {
    Running,
    Idle,
    Dead,
}

/// A vat: an isolated single-threaded execution context.
pub struct Vat {
    pub id: u32,
    pub heap: Heap,
    pub vm: VM,
    pub mailbox: VecDeque<Message>,
    pub status: VatStatus,
}

const BOOTSTRAP_FILES: &[&str] = &[
    "lib/bootstrap.moof",
    "lib/protocols.moof",
    "lib/comparable.moof",
    "lib/numeric.moof",
    "lib/iterable.moof",
    "lib/indexable.moof",
    "lib/callable.moof",
    "lib/types.moof",
    "lib/error.moof",
    "lib/showable.moof",
    "lib/range.moof",
    "lib/act.moof",
];

impl Vat {
    /// Create a new vat with a fresh Heap and VM.
    pub fn new(id: u32) -> Self {
        let mut heap = Heap::new();
        heap.vat_id = id;
        let vm = VM::new();
        crate::runtime::register_type_protos(&mut heap);
        Vat {
            id,
            heap,
            vm,
            mailbox: VecDeque::new(),
            status: VatStatus::Idle,
        }
    }

    /// Load bootstrap libraries into this vat.
    pub fn load_bootstrap(&mut self) {
        for path in BOOTSTRAP_FILES {
            if let Ok(source) = std::fs::read_to_string(path) {
                match self.eval_source(&source) {
                    Ok(_) => eprintln!("  loaded {path}"),
                    Err(e) => { eprintln!("  ~ error in {path}: {e}"); return; }
                }
            }
        }
    }

    /// Evaluate source code in this vat.
    pub fn eval_source(&mut self, source: &str) -> Result<Value, String> {
        let tokens = crate::lang::lexer::tokenize(source).map_err(|e| format!("lex: {e}"))?;
        let mut parser = crate::lang::parser::Parser::new(&tokens, &mut self.heap);
        let exprs = parser.parse_all().map_err(|e| format!("parse: {e}"))?;
        let mut last = Value::NIL;
        for expr in &exprs {
            let result = Compiler::compile_toplevel(&self.heap, *expr)
                .map_err(|e| format!("compile: {e}"))?;
            last = self.vm.eval_result(&mut self.heap, result)
                .map_err(|e| format!("eval: {e}"))?;
        }
        Ok(last)
    }

    /// Dispatch a message to a receiver object in this vat.
    pub fn dispatch_message(&mut self, msg: &Message) -> Result<Value, String> {
        let receiver = Value::nursery(msg.receiver_id);
        // look up selector name from this vat's symbol table
        let sel_name = self.heap.symbol_name(msg.selector).to_string();
        let local_sel = self.heap.intern(&sel_name);
        self.vm.send_message(&mut self.heap, receiver, local_sel, &msg.args)
    }
}

/// The scheduler: manages vats and runs them.
pub struct Scheduler {
    pub vats: Vec<Vat>,
    pub fuel_per_turn: u64,
    next_vat_id: u32,
}

impl Scheduler {
    pub fn new(fuel_per_turn: u64) -> Self {
        Scheduler {
            vats: Vec::new(),
            fuel_per_turn,
            next_vat_id: 0,
        }
    }

    /// Spawn a new vat with bootstrap loaded. Returns the vat ID.
    pub fn spawn_vat(&mut self) -> u32 {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        let mut vat = Vat::new(id);
        vat.load_bootstrap();
        self.vats.push(vat);
        id
    }

    /// Spawn a bare vat (no bootstrap). Used for the init vat.
    pub fn spawn_bare_vat(&mut self) -> u32 {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        let vat = Vat::new(id);
        self.vats.push(vat);
        id
    }

    /// Spawn a capability vat from a CapabilityPlugin.
    /// Returns (vat_id, root_object_id).
    pub fn spawn_capability(&mut self, cap: &dyn crate::plugins::CapabilityPlugin) -> (u32, u32) {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        let mut vat = Vat::new(id);
        let obj_id = cap.setup(&mut vat);
        self.vats.push(vat);
        (id, obj_id)
    }

    /// Create a FarRef in a vat pointing to an object in another vat.
    pub fn create_farref(&mut self, in_vat: u32, target_vat: u32, target_obj: u32) -> Value {
        let vat = self.vat_mut(in_vat);
        let farref_proto = vat.heap.type_protos[crate::heap::PROTO_FARREF];
        let tgt_vat_sym = vat.heap.intern("__target_vat");
        let tgt_obj_sym = vat.heap.intern("__target_obj");
        vat.heap.make_object_with_slots(
            farref_proto,
            vec![tgt_vat_sym, tgt_obj_sym],
            vec![Value::integer(target_vat as i64), Value::integer(target_obj as i64)],
        )
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
        for _ in 0..1000 {  // safety bound
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
                    match vat.vm.call_value(&mut vat.heap, f, &[val]) {
                        Ok(result) => {
                            self.resolve_act(vat_id, act_id, result, false);
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
                    crate::heap::SpawnPayload::Source(ref source) => {
                        self.vat_mut(child_id).eval_source(source)
                    }
                    crate::heap::SpawnPayload::Closure(closure_val) => {
                        self.run_closure_in_vat(closure_val, parent_vat_id, child_id, &[])
                    }
                    crate::heap::SpawnPayload::ClosureWithArgs(closure_val, ref args) => {
                        let args = args.clone();
                        self.run_closure_in_vat(closure_val, parent_vat_id, child_id, &args)
                    }
                };

                // resolve the Act in the parent vat
                match result {
                    Ok(val) => {
                        let copied_val = self.copy_value_across(val, child_id, parent_vat_id);
                        self.resolve_act(parent_vat_id, req.act_id, copied_val, false);
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
                        resolutions.push(ActResolution {
                            vat_id: source_vat_id,
                            act_id: out_msg.act_id,
                            result: val,
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
        // --- phase 1: extract everything from source vat (no borrows of target) ---
        let from_vat = &self.vats[from_vat_id as usize];
        let (code_idx, _) = from_vat.heap.as_closure(closure_val)
            .ok_or("spawn: not a closure")?;

        if code_idx >= from_vat.vm.closure_descs_ref().len() {
            return Err("spawn: closure code_idx out of bounds".into());
        }

        let src_desc = &from_vat.vm.closure_descs_ref()[code_idx];
        let src_chunk_arity = src_desc.chunk.arity;
        let src_is_operative = src_desc.is_operative;
        let src_desc_base = src_desc.desc_base;

        // clone all descs from desc_base onwards
        let src_descs: Vec<_> = from_vat.vm.closure_descs_ref()[src_desc_base..]
            .iter()
            .map(|d| {
                // clone constant values for remapping
                let const_vals: Vec<Value> = d.chunk.constants.iter()
                    .map(|&bits| Value::from_bits(bits))
                    .collect();
                (d.chunk.clone(), d.param_names.clone(), d.is_operative,
                 d.capture_names.clone(), d.capture_parent_regs.clone(),
                 d.capture_local_regs.clone(), d.capture_values.clone(),
                 d.rest_param_reg, const_vals)
            })
            .collect();

        // extract capture data from the closure heap object
        let captures: Vec<(String, Value)> = from_vat.heap.closure_captures(closure_val)
            .iter()
            .map(|(sym, val)| {
                (from_vat.heap.symbol_name(*sym).to_string(), *val)
            })
            .collect();

        // --- phase 2: remap and install into target vat ---
        // remap captured values
        let mut new_captures: Vec<(u32, Value)> = Vec::new();
        for (sym_name, val) in &captures {
            let new_sym = self.vat_mut(to_vat_id).heap.intern(sym_name);
            let new_val = self.copy_value_across(*val, from_vat_id, to_vat_id);
            new_captures.push((new_sym, new_val));
        }

        // build new descs with remapped constants
        let target_base = self.vat(to_vat_id).vm.closure_descs_ref().len();
        let new_code_idx = target_base + (code_idx - src_desc_base);

        for (mut chunk, param_names, is_op, cap_names, cap_parent, cap_local, cap_vals, rest_reg, const_vals) in src_descs {
            // remap constants
            chunk.constants = const_vals.iter()
                .map(|v| self.copy_value_across(*v, from_vat_id, to_vat_id).to_bits())
                .collect();

            let desc = crate::lang::compiler::ClosureDesc {
                chunk,
                param_names,
                is_operative: is_op,
                capture_names: cap_names,
                capture_parent_regs: cap_parent,
                capture_local_regs: cap_local,
                capture_values: cap_vals,
                desc_base: target_base,
                rest_param_reg: rest_reg,
            };
            self.vat_mut(to_vat_id).vm.add_closure_desc(desc);
        }

        // create the closure in the target heap
        let new_closure = self.vat_mut(to_vat_id).heap.make_closure(
            new_code_idx,
            src_chunk_arity,
            src_is_operative,
            &new_captures,
        );

        // copy args across heaps
        let mut new_args: Vec<Value> = Vec::new();
        for arg in args {
            new_args.push(self.copy_value_across(*arg, from_vat_id, to_vat_id));
        }

        // call the closure with args
        let vat = self.vat_mut(to_vat_id);
        vat.vm.call_value(&mut vat.heap, new_closure, &new_args)
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
        // heap objects: for now, copy strings
        if let Some(obj_id) = val.as_any_object() {
            let from_heap = &self.vat(_from_vat).heap;
            match from_heap.get(obj_id) {
                crate::object::HeapObject::Text(s) => {
                    let s = s.clone();
                    return self.vat_mut(to_vat).heap.alloc_string(&s);
                }
                crate::object::HeapObject::General { parent: _, slot_names, slot_values, .. } => {
                    // copy General objects (including FarRefs) by cloning slots
                    let names: Vec<String> = slot_names.iter()
                        .map(|s| from_heap.symbol_name(*s).to_string())
                        .collect();
                    let vals: Vec<Value> = slot_values.clone();
                    // check if this is a FarRef (has __target_vat slot)
                    let is_farref = names.iter().any(|n| n == "__target_vat");
                    // re-intern names and copy values in target heap
                    let new_names: Vec<u32> = names.iter()
                        .map(|n| self.vat_mut(to_vat).heap.intern(n))
                        .collect();
                    let new_vals: Vec<Value> = vals.iter()
                        .map(|v| self.copy_value_across(*v, _from_vat, to_vat))
                        .collect();
                    let parent = if is_farref {
                        self.vat(to_vat).heap.type_protos[crate::heap::PROTO_FARREF]
                    } else {
                        self.vat(to_vat).heap.type_protos[crate::heap::PROTO_OBJ]
                    };
                    return self.vat_mut(to_vat).heap.make_object_with_slots(
                        parent, new_names, new_vals,
                    );
                }
                _ => {
                    eprintln!("  ~ warning: cannot copy heap object across vats (yet)");
                    return Value::NIL;
                }
            }
        }
        val
    }

    /// Check if a value is an Act (has PROTO_ACT as prototype).
    fn is_act(heap: &Heap, val: Value) -> bool {
        let act_proto = heap.type_protos[crate::heap::PROTO_ACT];
        if act_proto.is_nil() { return false; }
        let proto = heap.prototype_of(val);
        proto == act_proto
    }

    /// Resolve an Act: set state to resolved, store result, run continuations.
    /// If the final value is itself an Act (monadic bind), set up forwarding.
    fn resolve_act(&mut self, vat_id: u32, act_id: u32, result: Value, is_error: bool) {
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
                                for r_cont in remaining.iter().rev() {
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

        // check if this Act has a forward link (inner Act → outer Act)
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
