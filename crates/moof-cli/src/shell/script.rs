// Script runner: a sibling interface to the REPL.
//
// Boots the system the same way the REPL does, spawns a user vat
// with the [grants] for `script` (falling back to repl's grants),
// evaluates the file, drains, prints the final result, and exits.
//
// The point of this module is not that anyone needs to run scripts
// right now — it's that the REPL isn't special. Boot is shared;
// the interface is a thin consumer; you can build a new one in a
// hundred lines.

use moof::boot::BootedSystem;
use moof::manifest::Manifest;
use moof::store::Store;
use std::path::Path;

const MANIFEST_PATH: &str = "moof.toml";

pub fn run(path: &str) -> i32 {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("  ~ read {path}: {e}"); return 1; }
    };

    let manifest = match Path::new(MANIFEST_PATH).exists() {
        true => Manifest::load(Path::new(MANIFEST_PATH)).unwrap_or_else(|_| Manifest::default()),
        false => Manifest::default(),
    };

    let mut sys = BootedSystem::boot(manifest);
    // prefer a `script` grants entry, fall back to `repl`'s list.
    let grants = sys.manifest.grants.get("script")
        .or_else(|| sys.manifest.grants.get("repl"))
        .cloned().unwrap_or_default();
    let vat_id = sys.spawn_with_caps(&grants);

    let code = match sys.eval(vat_id, &source) {
        Ok(v) => {
            let vat = sys.scheduler.vat(vat_id);
            println!("{}", vat.heap.display_value(v));
            0
        }
        Err(e) => { eprintln!("  ~ {e}"); 1 }
    };

    // save on exit, same as the REPL — scripts build up the image too.
    let store_path = &sys.manifest.image.path;
    if let Ok(store) = Store::open(Path::new(store_path)) {
        let vat = sys.scheduler.vat(vat_id);
        if let Err(e) = store.save_all(&vat.heap, vat.vm.closure_descs_ref()) {
            eprintln!("  ~ save failed: {e}");
        }
    }

    code
}
