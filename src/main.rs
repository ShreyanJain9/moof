mod value;
mod object;
mod heap;
mod dispatch;
mod opcodes;
mod vm;
mod store;
mod runtime;
mod lang;
mod shell;

fn main() {
    shell::repl::run();
}
