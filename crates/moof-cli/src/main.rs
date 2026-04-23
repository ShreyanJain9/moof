// moof — dispatch to an interface.
//
// Interfaces are siblings:
//   moof                 → REPL
//   moof <file.moof>     → script (eval file, print result, exit)
//   moof -e "expr"       → eval the argument, print, exit
//   moof --help | -h     → usage
//
// The REPL is not privileged. Adding a new interface means a new
// sibling under `shell::`, not a modification of the REPL.

mod shell;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => shell::repl::run(),
        [flag] if flag == "-h" || flag == "--help" => {
            print_usage();
        }
        [flag, expr] if flag == "-e" => {
            std::process::exit(shell::eval::run(expr.clone()));
        }
        [path] if !path.starts_with('-') => {
            std::process::exit(shell::script::run(path));
        }
        _ => {
            print_usage();
            std::process::exit(2);
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  moof                — interactive repl");
    eprintln!("  moof <file.moof>    — evaluate a file, print result, exit");
    eprintln!("  moof -e \"<expr>\"    — evaluate the expression, print, exit");
    eprintln!("  moof -h | --help    — this message");
}
