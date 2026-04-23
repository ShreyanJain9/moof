// Script runner — an Interface. Sibling of the REPL.
//
// The user says `moof file.moof`; main dispatches here; System hands
// us a vat with the allowed caps; we eval the file, print the result,
// return. No boot orchestration; no capability authority.

use moof::system::{Interface, System};
use moof::manifest::Manifest;
use std::path::Path;

const MANIFEST_PATH: &str = "moof.toml";

pub fn run(path: &str) -> i32 {
    let Ok(source) = std::fs::read_to_string(path) else {
        eprintln!("  ~ cannot read {path}");
        return 1;
    };

    let manifest = match Path::new(MANIFEST_PATH).exists() {
        true => Manifest::load(Path::new(MANIFEST_PATH)).unwrap_or_else(|_| Manifest::default()),
        false => Manifest::default(),
    };

    let mut sys = System::boot(manifest);
    let mut script = ScriptInterface { source };
    sys.run(&mut script)
}

struct ScriptInterface {
    source: String,
}

impl Interface for ScriptInterface {
    fn name(&self) -> &str { "script" }

    fn required_caps(&self) -> Vec<&str> {
        // same default ask as the repl; manifest's [grants.script]
        // decides what's actually granted.
        vec!["console", "clock", "file", "random", "system"]
    }

    fn run(&mut self, sys: &mut System, vat_id: u32) -> i32 {
        match sys.eval(vat_id, &self.source) {
            Ok(v) => {
                let vat = sys.vat(vat_id);
                println!("{}", vat.heap.display_value(v));
                0
            }
            Err(e) => { eprintln!("  ~ {e}"); 1 }
        }
    }
}
