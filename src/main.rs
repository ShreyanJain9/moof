mod value;
mod object;
mod heap;
mod dispatch;
mod opcodes;
mod vm;
mod store;
mod lang;
mod shell;

fn main() {
    shell::repl::run();
}
