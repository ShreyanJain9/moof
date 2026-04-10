// Vat scheduler: cooperative concurrency with fuel-based preemption.
//
// Each vat is an isolated execution context with its own Heap and VM.
// Vats communicate via eventual sends (messages queued to mailboxes).
// The scheduler runs vats round-robin, giving each a fuel budget per turn.

use std::collections::VecDeque;
use crate::heap::Heap;
use crate::vm::VM;
use crate::value::Value;
use crate::lang::compiler::Compiler;

/// A message queued for delivery to a vat.
pub struct Message {
    pub selector: u32,
    pub args: Vec<Value>,        // already deep-copied into target vat's heap
    pub reply_vat_id: u32,       // which vat to resolve the promise in
    pub reply_promise_id: u32,   // promise object ID in the reply vat
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VatStatus {
    Running,
    Idle,
    Dead,
}

/// A vat: an isolated single-threaded execution context.
pub struct Vat {
    pub id: u32,
    pub heap: Heap,
    pub vm: VM,
    pub mailbox: VecDeque<Message>,
    pub status: VatStatus,
}

impl Vat {
    /// Create a new vat with a fresh Heap and VM. Runs runtime initialization.
    pub fn new(id: u32) -> Self {
        let mut heap = Heap::new();
        let vm = VM::new();
        crate::runtime::register_type_protos(&mut heap);
        Vat {
            id,
            heap,
            vm,
            mailbox: VecDeque::new(),
            status: VatStatus::Idle,
        }
    }

    /// Evaluate source code in this vat.
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

    /// Process one message from the mailbox. Returns the result value.
    pub fn process_message(&mut self, msg: Message) -> Result<Value, String> {
        // for now, messages are evaluated as source code strings
        // TODO: proper message dispatch to receiver objects
        Ok(Value::NIL)
    }
}

/// The scheduler: manages vats and runs them round-robin.
pub struct Scheduler {
    pub vats: Vec<Vat>,
    pub ready_queue: VecDeque<u32>,
    pub fuel_per_turn: u64,
    next_vat_id: u32,
}

impl Scheduler {
    pub fn new(fuel_per_turn: u64) -> Self {
        Scheduler {
            vats: Vec::new(),
            ready_queue: VecDeque::new(),
            fuel_per_turn,
            next_vat_id: 0,
        }
    }

    /// Create the root vat (id=0). Used by the REPL.
    pub fn create_root_vat(&mut self) -> &mut Vat {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        self.vats.push(Vat::new(id));
        self.vats.last_mut().unwrap()
    }

    /// Spawn a new vat. Returns the vat ID.
    pub fn spawn(&mut self) -> u32 {
        let id = self.next_vat_id;
        self.next_vat_id += 1;
        let mut vat = Vat::new(id);
        // load bootstrap into the new vat
        let bootstrap_files = [
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
        for path in &bootstrap_files {
            if let Ok(source) = std::fs::read_to_string(path) {
                let _ = vat.eval_source(&source);
            }
        }
        self.vats.push(vat);
        self.ready_queue.push_back(id);
        id
    }

    /// Get a reference to a vat by ID.
    pub fn get_vat(&self, id: u32) -> Option<&Vat> {
        self.vats.iter().find(|v| v.id == id)
    }

    /// Get a mutable reference to a vat by ID.
    pub fn get_vat_mut(&mut self, id: u32) -> Option<&mut Vat> {
        self.vats.iter_mut().find(|v| v.id == id)
    }

    /// Enqueue a message to a vat's mailbox.
    pub fn enqueue(&mut self, vat_id: u32, msg: Message) {
        if let Some(vat) = self.get_vat_mut(vat_id) {
            vat.mailbox.push_back(msg);
            if vat.status == VatStatus::Idle {
                vat.status = VatStatus::Running;
                self.ready_queue.push_back(vat_id);
            }
        }
    }

    /// Run one turn for a vat: process messages with fuel budget.
    pub fn run_turn(&mut self, vat_id: u32) -> Result<(), String> {
        let fuel = self.fuel_per_turn;
        let vat = self.get_vat_mut(vat_id).ok_or("vat not found")?;
        vat.vm.fuel = fuel;
        vat.status = VatStatus::Running;

        // process messages from mailbox
        while let Some(msg) = vat.mailbox.pop_front() {
            match vat.process_message(msg) {
                Ok(_) => {}
                Err(e) => {
                    if VM::is_yield_error(&e) {
                        // fuel exhausted — re-enqueue
                        vat.status = VatStatus::Running;
                        self.ready_queue.push_back(vat_id);
                        return Ok(());
                    }
                    // real error — log and continue
                    eprintln!("  ~ vat {} error: {e}", vat_id);
                }
            }
        }

        // no more messages — idle
        let vat = self.get_vat_mut(vat_id).unwrap();
        vat.status = VatStatus::Idle;
        Ok(())
    }

    /// Run all ready vats until all are idle.
    pub fn run_all(&mut self) -> Result<(), String> {
        while let Some(vat_id) = self.ready_queue.pop_front() {
            self.run_turn(vat_id)?;
        }
        Ok(())
    }
}
