//! moof — phase 2 cli.
//!
//! invocations:
//!   moof '<expr>'        evaluate a single expression, print result.
//!   moof <file.moof>     load file, evaluate top-level forms in order,
//!                        print value of last expression.
//!   moof --no-bootstrap '<expr>'   skip stdlib (for substrate testing).
//!
//! the world boots with `lib/bootstrap.moof` loaded automatically. user
//! files run on top of that.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    let mut load_bootstrap = true;
    if !args.is_empty() && args[0] == "--no-bootstrap" {
        load_bootstrap = false;
        args.remove(0);
    }

    // build the world (with or without the bootstrap stdlib).
    let mut world = if load_bootstrap {
        match moof::new_world() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("moof: bootstrap failed: {}", e);
                return ExitCode::from(1);
            }
        }
    } else {
        moof::World::new()
    };

    // no args → enter the moof-defined REPL.
    if args.is_empty() {
        return match moof::eval_str(&mut world, "(repl)") {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("moof: {}", e);
                ExitCode::from(1)
            }
        };
    }

    let target = &args[0];
    let input = if target.ends_with(".moof") || std::path::Path::new(target).is_file() {
        match std::fs::read_to_string(target) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("moof: cannot read {}: {}", target, e);
                return ExitCode::from(1);
            }
        }
    } else {
        target.clone()
    };

    match moof::eval_program(&mut world, &input) {
        Ok(v) => {
            println!("{}", moof::print::show(&world, v));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("moof: {}", e);
            ExitCode::from(1)
        }
    }
}
