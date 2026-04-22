// moof binary — just runs the REPL.
// the runtime, plugins, and core are in lib.rs.

mod shell;

fn main() {
    shell::repl::run();
}
