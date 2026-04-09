use crate::heap::Heap;
use crate::lang::lexer;
use crate::lang::parser::Parser;
use crate::lang::compiler::{Compiler, ClosureDesc};
use crate::store::{Store, SerializableClosureDesc};
use crate::opcodes::Chunk;
use crate::vm::VM;
use crate::value::Value;
use std::path::Path;

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
    let files = [
        "lib/bootstrap.moof",
        "lib/protocols.moof",
        "lib/comparable.moof",
        "lib/numeric.moof",
        "lib/iterable.moof",
        "lib/indexable.moof",
        "lib/callable.moof",
        "lib/types.moof",
        "lib/error.moof",
        "lib/showable.moof",
        "lib/range.moof",
    ];
    for path in &files {
        if let Ok(source) = std::fs::read_to_string(path) {
            match eval_source(vm, heap, &source) {
                Ok(()) => eprintln!("  loaded {path}"),
                Err(e) => { eprintln!("  ~ error in {path}: {e}"); return; }
            }
        }
    }
}

const STORE_PATH: &str = ".moof/store";

pub fn run() {
    println!();
    println!("  .  *  .        m o o f        .  *  .");
    println!("       ~ a living objectspace ~");
    println!("    clarus the dogcow lives again");

    let store = match Store::open(Path::new(STORE_PATH)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ~ could not open store: {e}");
            eprintln!("  running without persistence");
            run_without_store();
            return;
        }
    };

    // try loading from LMDB
    let (mut heap, mut vm) = if let Some(image) = store.load_all() {
        let mut heap = Heap::restore(
            image.objects,
            image.symbols,
            image.globals,
            image.operatives,
        );
        // re-register native handlers (rust closures can't be serialized)
        crate::lang::compiler::register_type_protos(&mut heap);
        // restore closure descs
        let mut vm = VM::new();
        for desc in image.closure_descs {
            vm.add_closure_desc(ClosureDesc {
                chunk: Chunk {
                    code: desc.code,
                    constants: desc.constants,
                    arity: desc.arity,
                    num_regs: desc.num_regs,
                    name: String::new(),
                },
                param_names: desc.param_names,
                is_operative: desc.is_operative,
                capture_names: desc.capture_names,
                capture_parent_regs: desc.capture_parent_regs,
                capture_local_regs: desc.capture_local_regs,
                capture_values: Vec::new(),
                desc_base: desc.desc_base,
                rest_param_reg: desc.rest_param_reg,
            });
        }
        // reload stdlib (adds methods that can't be serialized, like error.moof)
        load_bootstrap(&mut vm, &mut heap);
        eprintln!("  restored from image ({} objects)", heap.object_count());
        (heap, vm)
    } else {
        // fresh start
        let mut heap = Heap::new();
        let mut vm = VM::new();
        crate::lang::compiler::register_type_protos(&mut heap);
        load_bootstrap(&mut vm, &mut heap);
        (heap, vm)
    };
    println!();

    let mut rl = match rustyline::DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => { eprintln!("readline: {e}"); return; }
    };

    loop {
        let line = match rl.readline("\u{2728} ") {
            Ok(line) => { let _ = rl.add_history_entry(&line); line }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(e) => { eprintln!("readline: {e}"); break; }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        let tokens = match lexer::tokenize(trimmed) {
            Ok(t) => t,
            Err(e) => { eprintln!("  ~ lex: {e}"); continue; }
        };

        let mut parser = Parser::new(&tokens, &mut heap);
        let exprs = match parser.parse_all() {
            Ok(e) => e,
            Err(e) => { eprintln!("  ~ parse: {e}"); continue; }
        };

        for expr in &exprs {
            match Compiler::compile_toplevel(&heap, *expr) {
                Ok(result) => {
                    match vm.eval_result(&mut heap, result) {
                        Ok(val) => {
                            // try [val show] for display, fall back to display_value
                            let show_sym = heap.intern("show");
                            let displayed = match vm.send_message(&mut heap, val, show_sym, &[]) {
                                Ok(show_val) => {
                                    if let Some(id) = show_val.as_any_object() {
                                        if let crate::object::HeapObject::Text(s) = heap.get(id) {
                                            s.clone()
                                        } else {
                                            heap.display_value(val)
                                        }
                                    } else {
                                        heap.display_value(val)
                                    }
                                }
                                Err(_) => heap.display_value(val),
                            };
                            println!("  {displayed}");
                        }
                        Err(e) => eprintln!("  ~ {e}"),
                    }
                }
                Err(e) => eprintln!("  ~ compile: {e}"),
            }
        }
    }

    // save to LMDB
    match store.save_all(
        heap.objects_ref(),
        heap.symbols_ref(),
        &heap.globals,
        &heap.operatives,
        vm.closure_descs_ref(),
    ) {
        Ok(()) => eprintln!("  image saved to LMDB ({} objects)", heap.object_count()),
        Err(e) => eprintln!("  ~ save failed: {e}"),
    }

    println!("\n  the circle closes. moof.\n");
}

fn run_without_store() {
    let mut heap = Heap::new();
    let mut vm = VM::new();
    crate::lang::compiler::register_type_protos(&mut heap);
    load_bootstrap(&mut vm, &mut heap);
    println!();

    let mut rl = rustyline::DefaultEditor::new().unwrap();
    loop {
        let line = match rl.readline("\u{2728} ") {
            Ok(line) => { let _ = rl.add_history_entry(&line); line }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        let tokens = match lexer::tokenize(trimmed) {
            Ok(t) => t, Err(e) => { eprintln!("  ~ {e}"); continue; }
        };
        let mut parser = Parser::new(&tokens, &mut heap);
        let exprs = match parser.parse_all() {
            Ok(e) => e, Err(e) => { eprintln!("  ~ {e}"); continue; }
        };
        for expr in &exprs {
            match Compiler::compile_toplevel(&heap, *expr) {
                Ok(r) => match vm.eval_result(&mut heap, r) {
                    Ok(val) => {
                        let show_sym = heap.intern("show");
                        let displayed = match vm.send_message(&mut heap, val, show_sym, &[]) {
                            Ok(sv) => if let Some(id) = sv.as_any_object() {
                                if let crate::object::HeapObject::Text(s) = heap.get(id) {
                                    s.clone()
                                } else { heap.display_value(val) }
                            } else { heap.display_value(val) },
                            Err(_) => heap.display_value(val),
                        };
                        println!("  {displayed}");
                    }
                    Err(e) => eprintln!("  ~ {e}"),
                },
                Err(e) => eprintln!("  ~ {e}"),
            }
        }
    }
    println!("\n  the circle closes. moof.\n");
}
