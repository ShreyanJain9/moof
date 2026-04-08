use crate::heap::Heap;
use crate::lang::lexer;
use crate::lang::parser::Parser;
use crate::lang::compiler::Compiler;
use crate::vm::VM;

fn eval_source(vm: &mut VM, heap: &mut Heap, source: &str) -> Result<(), String> {
    let tokens = lexer::tokenize(source).map_err(|e| format!("lex: {e}"))?;
    let mut parser = Parser::new(&tokens, heap);
    let exprs = parser.parse_all().map_err(|e| format!("parse: {e}"))?;
    for expr in &exprs {
        let result = Compiler::compile_toplevel(heap, *expr)
            .map_err(|e| format!("compile: {e}"))?;
        vm.eval_result(heap, result)
            .map_err(|e| format!("eval: {e}"))?;
    }
    Ok(())
}

fn load_bootstrap(vm: &mut VM, heap: &mut Heap) {
    // try loading lib/bootstrap.moof
    let paths = ["lib/bootstrap.moof", "bootstrap.moof"];
    for path in &paths {
        if let Ok(source) = std::fs::read_to_string(path) {
            match eval_source(vm, heap, &source) {
                Ok(()) => {
                    eprintln!("  loaded {path}");
                    return;
                }
                Err(e) => {
                    eprintln!("  ~ bootstrap error in {path}: {e}");
                    return;
                }
            }
        }
    }
    // no bootstrap file found — that's ok, run with just builtins
}

const IMAGE_PATH: &str = ".moof/image.bin";

pub fn run() {
    println!();
    println!("  .  *  .        m o o f        .  *  .");
    println!("       ~ a living objectspace ~");
    println!("    clarus the dogcow lives again");

    // try loading saved image, fall back to fresh bootstrap
    let mut heap = Heap::new();
    let mut vm = VM::new();
    crate::lang::compiler::register_type_protos(&mut heap);
    load_bootstrap(&mut vm, &mut heap);
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

    // save the image
    let _ = std::fs::create_dir_all(".moof");
    match heap.save_image(IMAGE_PATH) {
        Ok(()) => eprintln!("  image saved ({} objects)", heap.object_count()),
        Err(e) => eprintln!("  ~ could not save image: {e}"),
    }

    println!("\n  the circle closes. moof.\n");
}
