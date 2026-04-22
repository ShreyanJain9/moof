// Boot orchestration.
//
// Sets up the scheduler, plugins, vat 0 (init), and capability vats
// from a manifest. Returns a `BootedSystem` handle that any interface
// — REPL, script runner, headless service — can use.
//
// This is intentionally thin: nothing here is specific to the REPL.
// The REPL is just one consumer; a --script runner or a network
// service would be siblings. This is the first step of deprivileging
// the REPL in service of `docs/system.md` — moving toward a world
// where vat 0 owns the boot story in moof code rather than rust.
//
// Today, vat 0 is still created as a bare init vat; the rust-side
// helpers below do the work. As the `System` prototype in
// `lib/system/system.moof` grows, more of this will migrate into
// vat 0's moof code. For now, this module is the rust seam.

use crate::manifest::Manifest;
use crate::plugins;
use crate::scheduler::Scheduler;
use moof_core::Value;
use std::path::Path;

/// A capability that's been spawned into its own vat. Collected here
/// so any interface (REPL, script runner, service) can grant it to
/// whichever vat needs it.
pub struct CapRef {
    pub name: String,
    pub vat_id: u32,
    pub obj_id: u32,
}

/// The post-boot runtime handle. The scheduler owns all vats; cap_refs
/// is the capability registry; init_vat_id is vat 0 (currently inert
/// but reserved as the future System vat).
pub struct BootedSystem {
    pub scheduler: Scheduler,
    pub manifest: Manifest,
    pub cap_refs: Vec<CapRef>,
    pub init_vat_id: u32,
}

impl BootedSystem {
    /// Resolve + spawn everything declared by the manifest. Prints
    /// progress to stderr (capability-managed terminal output is a
    /// future refactor).
    pub fn boot(manifest: Manifest) -> Self {
        let mut scheduler = Scheduler::new(100_000);

        let type_plugins = plugins::resolve_type_plugins(&manifest.types);
        let bootstrap_sources: Vec<String> = manifest.sources.files.iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect();

        scheduler.install_type_plugins(type_plugins);
        scheduler.set_bootstrap_sources(bootstrap_sources);

        // vat 0: the init vat. today it's bare (no plugins, no
        // bootstrap). the system design calls for it to own the
        // boot sequence in moof code — but that refactor is phase 1
        // of the system.md plan. for now it just exists.
        let init_vat_id = scheduler.spawn_bare_vat();

        // spawn capability vats from the manifest. each is a dylib;
        // `builtin:X` shorthand resolves to target/<profile>/libmoof_cap_X.
        let mut cap_refs: Vec<CapRef> = Vec::new();
        for (name, spec) in &manifest.capabilities {
            match plugins::resolve_capability(spec) {
                Ok(cap) => {
                    let (vat_id, obj_id) = scheduler.spawn_capability(cap.as_ref());
                    eprintln!("  loaded capability '{}' from {spec}", cap.name());
                    cap_refs.push(CapRef { name: name.clone(), vat_id, obj_id });
                }
                Err(e) => eprintln!("  ~ capability '{name}' failed: {e}"),
            }
        }

        // log bootstrap source load (the actual eval happens inside
        // spawn_vat via the generic spawn path).
        for source_path in &manifest.sources.files {
            if Path::new(source_path).exists() {
                eprintln!("  loaded {source_path}");
            } else {
                eprintln!("  ~ source not found: {source_path}");
            }
        }

        BootedSystem { scheduler, manifest, cap_refs, init_vat_id }
    }

    /// Spawn a vat with plugins + bootstrap applied, and grant it the
    /// named capabilities. Returns the new vat's id. Used by any
    /// interface that needs its own execution context (REPL, script
    /// runner, service).
    pub fn spawn_with_caps(&mut self, cap_names: &[String]) -> u32 {
        let vat_id = self.scheduler.spawn_vat();
        self.grant_caps(vat_id, cap_names);
        vat_id
    }

    /// Grant capabilities by name to a target vat. Binds each as a
    /// FarRef in the target vat's env under the capability's name.
    pub fn grant_caps(&mut self, target_vat: u32, cap_names: &[String]) {
        // clone the cap tuples so we don't hold a borrow on self during
        // the mutable scheduler calls.
        let grants: Vec<(String, u32, u32)> = self.cap_refs.iter()
            .filter(|c| cap_names.contains(&c.name))
            .map(|c| (c.name.clone(), c.vat_id, c.obj_id))
            .collect();
        for (name, src_vat, src_obj) in grants {
            let farref = self.scheduler.create_farref(target_vat, src_vat, src_obj);
            let sym = self.scheduler.vat_mut(target_vat).heap.intern(&name);
            self.scheduler.vat_mut(target_vat).heap.env_def(sym, farref);
        }
    }

    /// Drain pending work (Acts, messages, spawns) in the scheduler.
    pub fn drain(&mut self) { self.scheduler.drain(); }

    /// Save the given vat's heap to the image store declared by the
    /// manifest. Returns Ok(object_count) on success.
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

    /// Evaluate source in the given vat and drain until quiescent.
    /// Returns the resolved value.
    pub fn eval(&mut self, vat_id: u32, source: &str) -> Result<Value, String> {
        self.scheduler.eval_in_vat(vat_id, source)
    }
}
