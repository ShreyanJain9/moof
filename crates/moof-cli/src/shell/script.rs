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
        //
        // Auto-commit is on by default: each top-level form's eval is
        // followed by `sys.save_image(vat_id)`. Continuous persistence
        // — a script that crashes mid-stream still has its pre-crash
        // state durable in lmdb. The merkle cache (stage A) + merged
        // reachability walk + known_stored set make each per-form
        // commit cheap (~65ms warm). Set MOOF_NO_AUTO_COMMIT=1 to
        // restore the old single-save-at-end behavior (e.g. for
        // benchmarking, or for a script you specifically don't want
        // partially-committed).
        if std::env::var("MOOF_NO_AUTO_COMMIT").is_ok() {
            return match sys.eval(vat_id, &self.source) {
                Ok(_) => 0,
                Err(e) => { eprintln!("  ~ {e}"); 1 }
            };
        }
        self.run_with_auto_commit(sys, vat_id)
    }
}

impl ScriptInterface {
    fn run_with_auto_commit(&self, sys: &mut System, vat_id: u32) -> i32 {
        let forms = moof_core::source::split_top_level_forms(&self.source);
        for (form_text, _range) in forms {
            if let Err(e) = sys.eval(vat_id, &form_text) {
                eprintln!("  ~ {e}");
                // best-effort commit even on error so the pre-error
                // state persists.
                let _ = sys.save_image(vat_id);
                return 1;
            }
            if let Err(e) = sys.save_image(vat_id) {
                eprintln!("  ~ commit: {e}");
                return 1;
            }
        }
        0
    }
}
