// moof — dispatch to an interface.
//
// The rust side does one thing: pick a moof source file + argv and
// hand control to the script runner (or the repl for no-args).
// The actual behaviors live as moof code in lib/bin/*.moof, so
// anyone can inspect, edit, or replace them.
//
//   moof                      → REPL
//   moof <file.moof> args...  → run that file with argv = [args...]
//   moof -e "<expr>"          → run lib/bin/eval.moof with argv = [expr]
//   moof --help | -h          → usage
//
// Adding a new top-level command means adding a moof file under
// lib/bin/, not a new Rust file.

mod shell;

/// Path to the bundled eval script. Loaded from the current working
/// directory's lib/bin so it's editable as part of the source tree.
/// Future: honor a search path (~/.moof/bin, plugin dirs, etc.).
const EVAL_SCRIPT: &str = "lib/bin/eval.moof";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => shell::repl::run(),
        [flag] if flag == "-h" || flag == "--help" => {
            print_usage();
        }
        [flag, expr] if flag == "-e" => {
            std::process::exit(shell::script::run(EVAL_SCRIPT, vec![expr.clone()]));
        }
        [path, rest @ ..] if !path.starts_with('-') => {
            std::process::exit(shell::script::run(path, rest.to_vec()));
        }
        _ => {
            print_usage();
            std::process::exit(2);
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  moof                      — interactive repl");
    eprintln!("  moof <file.moof> args...  — run file with argv=[args]");
    eprintln!("  moof -e \"<expr>\"          — evaluate the expression (via lib/bin/eval.moof)");
    eprintln!("  moof -h | --help          — this message");
}
