//! `moof` cli — phase A.14 + REPL.
//!
//! two modes:
//! - `moof '<expr>'` — eval one expression, print via `$out say:`.
//! - `moof` (no args) — drop into a REPL.
//!
//! per `process/docs-driven.md`'s capability rule, the *only* path
//! to stdout from moof code is `$out`. the cli pipes results
//! through `[$out say:]` accordingly. no `print`, `println`,
//! `puts` exist as moof-side bindings.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 {
        repl()
    } else if args.len() == 2 {
        eval_one_shot(&args[1])
    } else {
        usage();
        ExitCode::from(2)
    }
}

fn usage() {
    eprintln!("moof v{} — substrate seed", env!("CARGO_PKG_VERSION"));
    eprintln!();
    eprintln!("usage:");
    eprintln!("  moof              # repl");
    eprintln!("  moof '<expr>'     # eval one expression, print result");
}

fn eval_one_shot(source: &str) -> ExitCode {
    let mut world = moof::new_world();
    match moof::eval(&mut world, source) {
        Ok(value) => {
            // skip printing nil — matches lisp convention and the
            // REPL's behavior; programs that want explicit nil
            // output can `[$out say: nil]` themselves.
            if value.is_nil() {
                return ExitCode::SUCCESS;
            }
            match print_via_out(&mut world, value) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("moof: {}", e.message);
                    ExitCode::from(70)
                }
            }
        }
        Err(err) => {
            let _ = print_via_err(&mut world, &format!("error: {}", err.message));
            ExitCode::from(1)
        }
    }
}

fn repl() -> ExitCode {
    let mut world = moof::new_world();
    print_banner(&mut world);
    let stdin = io::stdin();
    loop {
        // prompt — write through $out so the cap discipline holds.
        let _ = print_prompt(&mut world);
        let _ = io::stdout().flush();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                // EOF (ctrl-d).
                let _ = print_via_out_text(&mut world, "\ngoodbye.\n");
                return ExitCode::SUCCESS;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("moof: stdin read failed: {}", e);
                return ExitCode::from(74);
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "(quit)" || trimmed == ":quit" {
            let _ = print_via_out_text(&mut world, "goodbye.\n");
            return ExitCode::SUCCESS;
        }
        match moof::eval(&mut world, trimmed) {
            Ok(value) => {
                if !value.is_nil() {
                    // REPL prints via :inspect (re-readable) rather
                    // than :toString (display-friendly). matches the
                    // smalltalk `printNl` convention. one-shot
                    // `moof '<expr>'` keeps :toString — there the
                    // user is producing output, not debugging.
                    let _ = print_via_out_inspect(&mut world, value);
                }
            }
            Err(err) => {
                let _ = print_via_err(&mut world, &format!("! {}", err.message));
            }
        }
    }
}

/// REPL print path: `[v inspect]` then emit + newline. distinct from
/// `[$out say:]` (which uses :toString) so the REPL can show
/// re-readable output: `"hello"` not `hello`, `#\a` not `a`.
fn print_via_out_inspect(
    world: &mut moof::world::World,
    value: moof::value::Value,
) -> Result<(), moof::world::RaiseError> {
    let inspect = world.intern("inspect");
    let str_v = world.send(value, inspect, &[])?;
    let text = world
        .string_text(str_v)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "<inspect: not a String>".to_string());
    print_via_out_text(world, &text)?;
    print_via_out_text(world, "\n")
}

fn print_banner(world: &mut moof::world::World) {
    let banner = format!(
        "moof v{} — substrate seed\ntype expressions; ctrl-d or :quit to exit.\n",
        env!("CARGO_PKG_VERSION")
    );
    let _ = print_via_out_text(world, &banner);
}

fn print_prompt(world: &mut moof::world::World) -> Result<(), moof::world::RaiseError> {
    print_via_out_text(world, "> ")
}

fn print_via_out(
    world: &mut moof::world::World,
    value: moof::value::Value,
) -> Result<(), moof::world::RaiseError> {
    let dollar_out = world.intern("$out");
    let say = world.intern("say:");
    let out = world.env_lookup(world.global_env, dollar_out).ok_or_else(|| {
        moof::world::RaiseError::new(world.intern("missing-cap"), "$out unbound")
    })?;
    world.send(out, say, &[value]).map(|_| ())
}

fn print_via_out_text(
    world: &mut moof::world::World,
    text: &str,
) -> Result<(), moof::world::RaiseError> {
    let dollar_out = world.intern("$out");
    let emit = world.intern("emit:");
    let out = world.env_lookup(world.global_env, dollar_out).ok_or_else(|| {
        moof::world::RaiseError::new(world.intern("missing-cap"), "$out unbound")
    })?;
    let payload = world.make_string(text);
    world.send(out, emit, &[payload]).map(|_| ())
}

fn print_via_err(
    world: &mut moof::world::World,
    text: &str,
) -> Result<(), moof::world::RaiseError> {
    let dollar_err = world.intern("$err");
    let emit = world.intern("emit:");
    let err = match world.env_lookup(world.global_env, dollar_err) {
        Some(v) => v,
        None => {
            // shouldn't happen — $err is a primordial cap.
            eprintln!("{}", text);
            return Ok(());
        }
    };
    let payload = world.make_string(&format!("{}\n", text));
    world.send(err, emit, &[payload]).map(|_| ())
}
