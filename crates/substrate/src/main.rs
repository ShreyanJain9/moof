//! `moof` cli — phase A.14.
//!
//! evaluates a single moof expression supplied on argv and prints
//! its result to stdout via the `$out` cap. this is the *only*
//! path the substrate seed offers to write text from moof code —
//! `process/docs-driven.md` enforces this.
//!
//! later: `moof world <dir>` and `moof world join <url>` cli
//! commands; `moof repl` for an interactive session; flag-driven
//! diagnostic modes (disassemble, profile, etc).

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "moof v{} — substrate seed",
            env!("CARGO_PKG_VERSION")
        );
        eprintln!("usage: moof '<expr>'");
        eprintln!();
        eprintln!("examples:");
        eprintln!("  moof '(+ 1 2)'");
        eprintln!("  moof '(let ((a 3) (b 4)) (+ a b))'");
        eprintln!("  moof '[$out emit: \\'hi\\\\n]'");
        return ExitCode::from(2);
    }
    let source = &args[1];
    let mut world = moof::new_world();
    match moof::eval(&mut world, source) {
        Ok(value) => {
            // pipe the result through `$out say:` per the cap rule.
            // no path to stdout that bypasses `$out` —
            // `process/docs-driven.md` guarantees this.
            let dollar_out = world.intern("$out");
            let say = world.intern("say:");
            let out = match world.env_lookup(world.global_env, dollar_out) {
                Some(v) => v,
                None => {
                    eprintln!("moof: $out cap is unbound (substrate boot failed)");
                    return ExitCode::from(70);
                }
            };
            if let Err(e) = world.send(out, say, &[value]) {
                eprintln!("moof: error printing result: {}", e.message);
                return ExitCode::from(70);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            // routing the error through $err to stay on-discipline.
            let dollar_err = world.intern("$err");
            let say = world.intern("say:");
            let err_cap = world.env_lookup(world.global_env, dollar_err);
            // fall back to plain eprintln if even $err is broken.
            let payload = world.intern(&format!("error: {}", err.message));
            if let Some(cap) = err_cap {
                let _ = world.send(cap, say, &[moof::value::Value::Sym(payload)]);
            } else {
                eprintln!("error: {}", err.message);
            }
            ExitCode::from(1)
        }
    }
}
