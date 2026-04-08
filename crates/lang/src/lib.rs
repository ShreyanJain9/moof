//! moof-lang: the moof language shell.
//!
//! lexer → parser → analyzer → compiler → bytecode interpreter.
//! registers a BytecodeInvoker with the fabric's dispatch system.

pub mod lexer;
pub mod parser;
pub mod compiler;
pub mod opcodes;
pub mod vm;

// TODO: pub mod analyze; (vau-aware analysis pass)
