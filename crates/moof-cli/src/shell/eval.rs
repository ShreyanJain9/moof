// Eval runner — an Interface. Sibling of repl + script.
//
// `moof -e "(+ 1 2)"` — parse, compile, evaluate, print, exit.
// Same boot path + caps as script; the only difference is the
// source comes from argv rather than a file.

use moof::system::{Interface, System};
use moof::manifest::Manifest;
use std::path::Path;

const MANIFEST_PATH: &str = "moof.toml";

pub fn run(source: String) -> i32 {
    let manifest = match Path::new(MANIFEST_PATH).exists() {
        true => Manifest::load(Path::new(MANIFEST_PATH)).unwrap_or_else(|_| Manifest::default()),
        false => Manifest::default(),
    };
    let mut sys = System::boot(manifest);
    let mut iface = EvalInterface { source };
    sys.run(&mut iface)
}

struct EvalInterface {
    source: String,
}

impl Interface for EvalInterface {
    fn name(&self) -> &str { "eval" }

    fn required_caps(&self) -> Vec<&str> {
        // same ask as script; manifest's [grants.eval] (falling back
        // to [grants.script] if absent — handled by grants_for default).
        vec!["console", "clock", "file", "random", "system", "evaluator"]
    }

    fn run(&mut self, sys: &mut System, vat_id: u32) -> i32 {
        match sys.eval(vat_id, &self.source) {
            Ok(v) => {
                let vat = sys.vat(vat_id);
                // emit `show` form for readability.
                println!("{}", vat.heap.display_value(v));
                0
            }
            Err(e) => { eprintln!("  ~ {e}"); 1 }
        }
    }
}
