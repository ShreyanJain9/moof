//! REPL: read-eval-print loop.

use std::path::Path;

use moof_fabric::Store;
use moof_lang::lexer::Lexer;
use moof_lang::parser::Parser;

/// Run the REPL.
pub fn run(store_path: &Path) -> Result<(), String> {
    let mut store = Store::open(store_path)?;

    println!("MOOF v2 — Moof Open Objectspace Fabric");
    println!("clarus the dogcow lives again");
    println!("Type expressions to evaluate. Ctrl-D to exit.\n");

    let mut rl = rustyline::DefaultEditor::new().map_err(|e| format!("readline: {e}"))?;

    loop {
        let line = match rl.readline("moof> ") {
            Ok(line) => {
                let _ = rl.add_history_entry(&line);
                line
            }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(e) => return Err(format!("readline: {e}")),
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // lex
        let mut lexer = Lexer::new(trimmed);
        let tokens = match lexer.tokenize() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("!! lex error: {e}");
                continue;
            }
        };

        // parse
        let mut parser = Parser::new(&tokens, &mut store);
        let exprs = match parser.parse_all() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("!! parse error: {e}");
                continue;
            }
        };

        // for now, just print the parsed AST values
        for expr in &exprs {
            println!("=> {expr:?}");
        }
    }

    println!("\nmoof.");
    Ok(())
}
