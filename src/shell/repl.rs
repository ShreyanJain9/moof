use crate::heap::Heap;
use crate::lang::lexer;
use crate::lang::parser::Parser;
use crate::lang::compiler::Compiler;
use crate::vm::VM;

pub fn run() {
    let mut heap = Heap::new();
    let mut vm = VM::new();
    crate::lang::compiler::register_type_protos(&mut heap);

    println!();
    println!("  .  *  .        m o o f        .  *  .");
    println!("       ~ a living objectspace ~");
    println!("    clarus the dogcow lives again");
    println!();

    let mut rl = match rustyline::DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => {
            eprintln!("the scrying glass cracks: {e}");
            return;
        }
    };

    loop {
        let line = match rl.readline("\u{2728} ") {
            Ok(line) => {
                let _ = rl.add_history_entry(&line);
                line
            }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(e) => {
                eprintln!("the circle breaks: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        // lex
        let tokens = match lexer::tokenize(trimmed) {
            Ok(t) => t,
            Err(e) => { eprintln!("  ~ lex: {e}"); continue; }
        };

        // parse
        let mut parser = Parser::new(&tokens, &mut heap);
        let exprs = match parser.parse_all() {
            Ok(e) => e,
            Err(e) => { eprintln!("  ~ parse: {e}"); continue; }
        };

        // compile and eval each expression
        for expr in &exprs {
            match Compiler::compile_toplevel(&heap, *expr) {
                Ok(result) => {
                    match vm.eval_result(&mut heap, result) {
                        Ok(val) => println!("  {}", heap.display_value(val)),
                        Err(e) => eprintln!("  ~ {e}"),
                    }
                }
                Err(e) => eprintln!("  ~ compile: {e}"),
            }
        }
    }

    println!("\n  the circle closes. moof.\n");
}
