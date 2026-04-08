use crate::heap::Heap;
use crate::lang::lexer;
use crate::lang::parser::Parser;
use crate::lang::compiler::Compiler;
use crate::vm::VM;

pub fn run() {
    let mut heap = Heap::new();
    let mut vm = VM::new();
    crate::lang::compiler::register_type_protos(&mut heap);

    println!("moof v2 — Moof Open Objectspace Fabric");
    println!("clarus the dogcow lives again");
    println!("type expressions to evaluate. ctrl-d to exit.\n");

    let mut rl = match rustyline::DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => {
            eprintln!("readline init failed: {e}");
            return;
        }
    };

    loop {
        let line = match rl.readline("moof> ") {
            Ok(line) => {
                let _ = rl.add_history_entry(&line);
                line
            }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(e) => {
                eprintln!("readline error: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        // lex
        let tokens = match lexer::tokenize(trimmed) {
            Ok(t) => t,
            Err(e) => { eprintln!("!! {e}"); continue; }
        };

        // parse
        let mut parser = Parser::new(&tokens, &mut heap);
        let exprs = match parser.parse_all() {
            Ok(e) => e,
            Err(e) => { eprintln!("!! {e}"); continue; }
        };

        // compile and eval each expression
        for expr in &exprs {
            match Compiler::compile_toplevel(&heap, *expr) {
                Ok(result) => {
                    match vm.eval_result(&mut heap, result) {
                        Ok(val) => println!("=> {}", heap.format_value(val)),
                        Err(e) => eprintln!("!! {e}"),
                    }
                }
                Err(e) => eprintln!("!! compile: {e}"),
            }
        }
    }

    println!("\nmoof.");
}
