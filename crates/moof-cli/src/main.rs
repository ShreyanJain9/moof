// moof — dispatch to an interface.
//
// Interfaces are siblings:
//   moof              → REPL
//   moof <file.moof>  → script (eval file, print result, exit)
//
// The REPL is not privileged. Adding a new interface means a new
// sibling under `shell::`, not a modification of the REPL.

mod shell;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => shell::repl::run(),
        [path] if path != "--help" && path != "-h" => {
            std::process::exit(shell::script::run(path));
        }
        _ => {
            eprintln!("usage:");
            eprintln!("  moof              — interactive repl");
            eprintln!("  moof <file.moof>  — evaluate a file, print result, exit");
            std::process::exit(2);
        }
    }
}
