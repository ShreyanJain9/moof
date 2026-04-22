// Vat: an isolated single-threaded execution context.
//
// Each vat has its own heap (objects, symbol table, natives),
// its own VM, and its own mailbox of pending messages. Vats never
// share heap references directly — all cross-vat communication
// goes through Messages, whose args are copied into the target
// vat's heap by the scheduler.

use std::collections::VecDeque;
use crate::heap::Heap;
use crate::vm::VM;
use crate::value::Value;
use crate::lang::compiler::Compiler;

/// A message queued for delivery to a vat.
pub struct Message {
    pub receiver_id: u32,   // object ID in the target vat
    pub selector: u32,      // method selector (symbol in target vat)
    pub args: Vec<Value>,   // values in the target vat's heap
    pub reply_vat_id: u32,  // which vat to resolve the Act in
    pub reply_act_id: u32,  // Act object ID in the reply vat
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VatStatus {
    Running,
    Idle,
    Dead,
}

pub struct Vat {
    pub id: u32,
    pub heap: Heap,
    pub vm: VM,
    pub mailbox: VecDeque<Message>,
    pub status: VatStatus,
}

const BOOTSTRAP_FILES: &[&str] = &[
    "lib/kernel/bootstrap.moof",
    "lib/kernel/protocols.moof",
    "lib/kernel/identity.moof",
    "lib/kernel/types.moof",
    "lib/kernel/error.moof",
    "lib/kernel/showable.moof",
    "lib/data/comparable.moof",
    "lib/data/numeric.moof",
    "lib/data/iterable.moof",
    "lib/data/indexable.moof",
    "lib/data/callable.moof",
    "lib/data/range.moof",
    "lib/data/act.moof",
];

impl Vat {
    /// Create a new vat with default type plugins registered on its heap.
    pub fn new(id: u32) -> Self {
        let mut heap = Heap::new();
        heap.vat_id = id;
        crate::plugins::register_all(&mut heap);
        Vat {
            id,
            heap,
            vm: VM::new(),
            mailbox: VecDeque::new(),
            status: VatStatus::Idle,
        }
    }

    /// Create a bare vat (no type plugins). Plugins registered separately.
    pub fn new_bare(id: u32) -> Self {
        let mut heap = Heap::new();
        heap.vat_id = id;
        Vat {
            id,
            heap,
            vm: VM::new(),
            mailbox: VecDeque::new(),
            status: VatStatus::Idle,
        }
    }

    /// Load the core stdlib files into this vat. Used when spawning
    /// vats that should have the full language available (e.g. REPL).
    pub fn load_bootstrap(&mut self) {
        for path in BOOTSTRAP_FILES {
            if let Ok(source) = std::fs::read_to_string(path) {
                match self.eval_source(&source) {
                    Ok(_) => eprintln!("  loaded {path}"),
                    Err(e) => { eprintln!("  ~ error in {path}: {e}"); return; }
                }
            }
        }
    }

    /// Evaluate source code in this vat's heap + VM.
    pub fn eval_source(&mut self, source: &str) -> Result<Value, String> {
        let tokens = crate::lang::lexer::tokenize(source).map_err(|e| format!("lex: {e}"))?;
        let mut parser = crate::lang::parser::Parser::new(&tokens, &mut self.heap);
        let exprs = parser.parse_all().map_err(|e| format!("parse: {e}"))?;
        let mut last = Value::NIL;
        for expr in &exprs {
            let result = Compiler::compile_toplevel(&self.heap, *expr)
                .map_err(|e| format!("compile: {e}"))?;
            last = self.vm.eval_result(&mut self.heap, result)
                .map_err(|e| format!("eval: {e}"))?;
        }
        Ok(last)
    }

    /// Dispatch a message to a receiver object in this vat.
    pub fn dispatch_message(&mut self, msg: &Message) -> Result<Value, String> {
        let receiver = Value::nursery(msg.receiver_id);
        // symbols are per-heap, so re-intern the selector name locally
        let sel_name = self.heap.symbol_name(msg.selector).to_string();
        let local_sel = self.heap.intern(&sel_name);
        self.vm.send_message(&mut self.heap, receiver, local_sel, &msg.args)
    }
}
