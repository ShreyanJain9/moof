// System — the sole authority.
//
// Every vat is spawned by System. Every capability is held by System
// and delegated from there. Interfaces (repl, script, future network
// listeners, future morph editors) are peer consumers that ask System
// for a user vat with specific capabilities granted, and use whatever
// System hands back.
//
// This is the rust-side realization of docs/system.md. Today System
// is a rust struct; phase 1 of the system roadmap promotes it to a
// defserver in lib/system/system.moof and the rust wrapper shrinks
// to "rehydrate image, send [System boot: manifest], run interface."
//
// Design commitment: no external consumer touches the scheduler
// directly. scheduler, capability list, and the spawned-vats list
// are all private. Every operation goes through a System method.
// This mirrors the final moof architecture where anything-outside-
// vat-0 can only influence the system by sending vat 0 messages.

use crate::manifest::Manifest;
use crate::plugins;
use crate::scheduler::Scheduler;
use crate::vat::Vat;
use moof_core::Value;
use std::path::Path;

/// A capability registered with System. Private to this module — the
/// only way to use a capability is to ask System to grant it.
struct CapEntry {
    name: String,
    vat_id: u32,
    obj_id: u32,
}

/// A record of a vat System spawned on behalf of an interface.
/// Private; used for the roster and future supervision.
struct UserVatEntry {
    id: u32,
    #[allow(dead_code)]  // surfaced in future phases for supervision
    owner_label: String,
    granted_caps: Vec<String>,
}

/// The root-of-authority handle. Owns the scheduler, the capability
/// registry, and the vat roster. No other module touches any of
/// these directly.
pub struct System {
    scheduler: Scheduler,
    manifest: Manifest,
    capabilities: Vec<CapEntry>,
    user_vats: Vec<UserVatEntry>,
    init_vat_id: u32,
    /// Location of the `system` capability's own heap object, if
    /// spawned. Used by `push_state` to mirror System's rust-side
    /// state into the capability's slots so moof can read it.
    system_cap_loc: Option<(u32, u32)>,
}

impl System {
    /// Boot: resolve plugins, create scheduler, spawn vat 0 (init),
    /// spawn capability vats declared by the manifest. Interfaces
    /// are NOT started here — callers must explicitly [run] one.
    pub fn boot(manifest: Manifest) -> Self {
        let mut scheduler = Scheduler::new(100_000);

        let type_plugins = plugins::resolve_type_plugins(&manifest.types);
        let bootstrap_sources: Vec<(String, String)> = manifest.sources.files.iter()
            .filter_map(|p| {
                std::fs::read_to_string(p).ok().map(|s| (s, p.clone()))
            })
            .collect();

        scheduler.install_type_plugins(type_plugins);
        scheduler.set_bootstrap_sources_with_labels(bootstrap_sources);

        // vat 0: reserved as System's home. today bare; phase 1 of
        // the system.md plan installs a full System defserver here.
        let init_vat_id = scheduler.spawn_bare_vat();

        let mut capabilities = Vec::new();
        let mut system_cap_loc: Option<(u32, u32)> = None;
        for (name, spec) in &manifest.capabilities {
            match plugins::resolve_capability(spec) {
                Ok(cap) => {
                    let (vat_id, obj_id) = scheduler.spawn_capability(cap.as_ref());
                    eprintln!("  loaded capability '{}' from {spec}", cap.name());
                    if cap.name() == "system" {
                        system_cap_loc = Some((vat_id, obj_id));
                    }
                    capabilities.push(CapEntry { name: name.clone(), vat_id, obj_id });
                }
                Err(e) => eprintln!("  ~ capability '{name}' failed: {e}"),
            }
        }

        for source_path in &manifest.sources.files {
            if Path::new(source_path).exists() {
                eprintln!("  loaded {source_path}");
            } else {
                eprintln!("  ~ source not found: {source_path}");
            }
        }

        let mut sys = System {
            scheduler,
            manifest,
            capabilities,
            user_vats: Vec::new(),
            init_vat_id,
            system_cap_loc,
        };
        sys.push_state();
        sys
    }

    // ─────────── accessors (read-only views) ───────────

    pub fn manifest(&self) -> &Manifest { &self.manifest }
    pub fn init_vat_id(&self) -> u32 { self.init_vat_id }

    /// Names of capabilities System is currently holding. Interfaces
    /// read this to know what they're allowed to ask for.
    pub fn capability_names(&self) -> Vec<&str> {
        self.capabilities.iter().map(|c| c.name.as_str()).collect()
    }

    /// The manifest's `[grants]` entry for a given interface label.
    /// Authoritative: System intersects an interface's request with
    /// this list before granting.
    pub fn grants_for(&self, interface: &str) -> Vec<String> {
        self.manifest.grants.get(interface).cloned().unwrap_or_default()
    }

    /// The vats that System has spawned on behalf of interfaces.
    /// Returns (vat_id, caps-granted) pairs. Used by the repl's
    /// `(plugins)` inspection command; will be the backbone of a
    /// `[System vats]` handler in phase 1.
    pub fn user_vats(&self) -> Vec<(u32, Vec<String>)> {
        self.user_vats.iter()
            .map(|v| (v.id, v.granted_caps.clone()))
            .collect()
    }

    // ─────────── the authority surface ───────────

    /// Spawn a user vat for an interface and grant it the named caps.
    /// Returns the vat id — the sole handle the interface gets back.
    /// If a requested cap isn't in System's registry, the grant is
    /// skipped with a warning. Nothing else can spawn a user vat;
    /// nothing else can grant caps.
    pub fn spawn_for_interface(&mut self, label: &str, cap_names: &[String]) -> u32 {
        let id = self.scheduler.spawn_vat();
        self.grant_internal(id, cap_names, label);
        self.user_vats.push(UserVatEntry {
            id,
            owner_label: label.to_string(),
            granted_caps: cap_names.to_vec(),
        });
        self.push_state();
        id
    }

    /// Grant additional caps to a previously-spawned user vat.
    /// Revocation is future work (phase 3 of system.md).
    pub fn grant(&mut self, target_vat: u32, cap_names: &[String]) {
        let exists = self.user_vats.iter().any(|v| v.id == target_vat);
        if !exists {
            eprintln!("  ~ grant refused: vat {target_vat} is not a user vat");
            return;
        }
        self.grant_internal(target_vat, cap_names, "additional-grant");
        if let Some(v) = self.user_vats.iter_mut().find(|v| v.id == target_vat) {
            for name in cap_names {
                if !v.granted_caps.contains(name) { v.granted_caps.push(name.clone()); }
            }
        }
        self.push_state();
    }

    fn grant_internal(&mut self, target_vat: u32, cap_names: &[String], label: &str) {
        let grants: Vec<(String, u32, u32)> = self.capabilities.iter()
            .filter(|c| cap_names.contains(&c.name))
            .map(|c| (c.name.clone(), c.vat_id, c.obj_id))
            .collect();
        for name in cap_names {
            if !grants.iter().any(|(n, _, _)| n == name) {
                eprintln!("  ~ {label}: capability '{name}' unknown; skipping");
            }
        }
        for (name, src_vat, src_obj) in grants {
            let farref = self.scheduler.create_farref(target_vat, src_vat, src_obj);
            let sym = self.scheduler.vat_mut(target_vat).heap.intern(&name);
            self.scheduler.vat_mut(target_vat).heap.env_def(sym, farref);
        }
    }

    // ─────────── execution surface — interfaces use these ───────────

    /// Evaluate source in a vat and drain until quiescent.
    pub fn eval(&mut self, vat_id: u32, source: &str) -> Result<Value, String> {
        self.scheduler.eval_in_vat(vat_id, source)
    }

    /// Drain pending cross-vat work.
    pub fn drain(&mut self) { self.scheduler.drain(); }

    /// Save the given vat's heap to the image store.
    pub fn save_image(&self, vat_id: u32) -> Result<usize, String> {
        use crate::store::Store;
        let store_path = &self.manifest.image.path;
        let store = Store::open(Path::new(store_path))
            .map_err(|e| format!("open store: {e}"))?;
        let vat = self.scheduler.vat(vat_id);
        store.save_all(&vat.heap, vat.vm.closure_descs_ref())
            .map_err(|e| format!("save: {e}"))?;
        Ok(vat.heap.object_count())
    }

    // ─────────── low-level vat access for interfaces ───────────
    //
    // the repl needs VM-level eval (rustyline-driven parse → compile
    // → eval → show). exposing vat access through narrow methods
    // keeps the interface surface tighter than a raw scheduler borrow
    // without demanding interfaces write all their moof as eval_source.

    pub fn vat(&self, id: u32) -> &Vat { self.scheduler.vat(id) }
    pub fn vat_mut(&mut self, id: u32) -> &mut Vat { self.scheduler.vat_mut(id) }

    // ─────────── mirror state into the `system` capability ───────────

    /// Write the current capability list, user-vat list, and grants
    /// table into the system cap's slots so moof code can read them
    /// via `[system capabilities]`, `[system vats]`, `[system grants]`.
    /// Called on boot and after any mutation. No-op if no `system`
    /// capability is registered.
    fn push_state(&mut self) {
        let Some((sys_vat, sys_obj)) = self.system_cap_loc else { return; };

        // Build everything into the cap vat's heap so it lives next
        // to the slots we're about to write into.
        let heap = &mut self.scheduler.vat_mut(sys_vat).heap;

        // capabilities: list of symbols
        let cap_syms: Vec<Value> = self.capabilities.iter()
            .map(|c| {
                let s = heap.intern(&c.name);
                Value::symbol(s)
            })
            .collect();
        let caps_list = heap.list(&cap_syms);
        let caps_slot = heap.intern("capability-names");
        heap.get_mut(sys_obj).slot_set(caps_slot, caps_list);

        // user-vats: list of integers
        let vat_ids: Vec<Value> = self.user_vats.iter()
            .map(|v| Value::integer(v.id as i64))
            .collect();
        let vats_list = heap.list(&vat_ids);
        let vats_slot = heap.intern("user-vats");
        heap.get_mut(sys_obj).slot_set(vats_slot, vats_list);

        // grants: list of (interface-sym . list-of-cap-syms) pairs
        let grants_entries: Vec<Value> = self.manifest.grants.iter()
            .map(|(iface, caps)| {
                let iface_sym = heap.intern(iface);
                let cap_vals: Vec<Value> = caps.iter()
                    .map(|c| {
                        let s = heap.intern(c);
                        Value::symbol(s)
                    })
                    .collect();
                let cap_list = heap.list(&cap_vals);
                heap.cons(Value::symbol(iface_sym), cap_list)
            })
            .collect();
        let grants_list = heap.list(&grants_entries);
        let grants_slot = heap.intern("grants-table");
        heap.get_mut(sys_obj).slot_set(grants_slot, grants_list);
    }

    // ─────────── running an interface ───────────

    /// Run a registered interface. System spawns the interface's
    /// vat, grants the manifest-allowed caps, runs to completion,
    /// then saves the image. Returns the interface's exit code.
    pub fn run(&mut self, iface: &mut dyn Interface) -> i32 {
        let label = iface.name().to_string();
        let requested: Vec<String> = iface.required_caps()
            .into_iter().map(String::from).collect();
        let allowed = self.grants_for(&label);
        let caps: Vec<String> = requested.into_iter()
            .filter(|c| allowed.contains(c))
            .collect();
        let vat_id = self.spawn_for_interface(&label, &caps);
        let code = iface.run(self, vat_id);
        if let Err(e) = self.save_image(vat_id) {
            eprintln!("  ~ save: {e}");
        }
        code
    }
}

/// An interface is a peer consumer of System. It declares its name
/// (matches a `[grants]` entry) and the caps it wants. System hands
/// it a vat id with the allowed caps granted; the interface runs
/// until it's done. REPL, script runner, future network listener,
/// future morph-backed editor — all siblings.
pub trait Interface {
    /// Stable label, matched against the manifest's `[grants]` table.
    fn name(&self) -> &str;

    /// Caps this interface wants. System intersects with grants_for
    /// and silently drops anything not explicitly allowed.
    fn required_caps(&self) -> Vec<&str>;

    /// Run until done. Returns a process exit code.
    fn run(&mut self, sys: &mut System, vat_id: u32) -> i32;
}
