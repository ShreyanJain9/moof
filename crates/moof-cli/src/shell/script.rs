// Script runner — an Interface. Sibling of the REPL.
//
// The user says `moof file.moof arg1 arg2`; main dispatches here;
// System hands us a vat with the allowed caps + an `argv` binding
// containing the trailing args; we eval the file.
//
// No boot orchestration; no capability authority. Also used as the
// engine for `moof -e EXPR` (via lib/bin/eval.moof + argv=[EXPR]).

use moof::system::{Interface, System};
use moof::manifest::Manifest;
use std::path::Path;

const MANIFEST_PATH: &str = "moof.toml";

/// Run a moof source file with the given argv.
pub fn run(path: &str, argv: Vec<String>) -> i32 {
    let Ok(source) = std::fs::read_to_string(path) else {
        eprintln!("  ~ cannot read {path}");
        return 1;
    };

    let manifest = match Path::new(MANIFEST_PATH).exists() {
        true => Manifest::load(Path::new(MANIFEST_PATH)).unwrap_or_else(|_| Manifest::default()),
        false => Manifest::default(),
    };

    let mut sys = System::boot(manifest);
    let mut script = ScriptInterface {
        path: path.to_string(),
        source,
        argv,
    };
    sys.run(&mut script)
}

struct ScriptInterface {
    #[allow(dead_code)]  // future: use as origin label
    path: String,
    source: String,
    argv: Vec<String>,
}

impl Interface for ScriptInterface {
    fn name(&self) -> &str { "script" }

    fn required_caps(&self) -> Vec<&str> {
        // same default ask as the repl; manifest's [grants.script]
        // decides what's actually granted.
        vec!["console", "clock", "file", "random", "system", "evaluator"]
    }

    fn argv(&self) -> Vec<String> { self.argv.clone() }

    fn run(&mut self, sys: &mut System, vat_id: u32) -> i32 {
        // Scripts own their own output. UNIX-style: if you want
        // something printed, call `[console println:]`. No auto-echo
        // of the final expression's value — that was a repl habit
        // that clashed with Acts (which look ugly when stringified).
        match sys.eval(vat_id, &self.source) {
            Ok(_) => 0,
            Err(e) => { eprintln!("  ~ {e}"); 1 }
        }
    }
}
