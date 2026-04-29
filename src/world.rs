//! the World — phase 2's runtime context.
//!
//! the World owns the heap, the symtab, the chunk table, the root
//! protos, and the global env (a Form).
//!
//! later phases (concepts/vats.md): per-vat Worlds, persistence,
//! mailboxes, supervisors. for now it's "the world" — single instance.

use crate::form::{Form, FormId, MethodImpl};
use crate::heap::Heap;
use crate::opcodes::{Chunk, ChunkId};
use crate::sym::{SymId, SymTab};
use crate::value::Value;

pub struct World {
    pub heap: Heap,
    pub syms: SymTab,
    pub chunks: Vec<Chunk>,

    /// the global env Form. top-level bindings live here. closures
    /// at the top level capture this as their parent env.
    pub global_env: FormId,

    /// pre-allocated root protos (populated by `protos::install`).
    pub object: FormId,
    pub nil_proto: FormId,
    pub bool_proto: FormId,
    pub integer_proto: FormId,
    pub symbol_proto: FormId,
    pub list_proto: FormId,
    pub builtin_proto: FormId,
    pub closure_proto: FormId,
    pub env_proto: FormId,
    pub string_proto: FormId,
    /// proto used by `[…]` send-form literals produced by the reader.
    /// the compiler dispatches on this to emit a Send opcode rather
    /// than a fn-call.
    pub send_form_proto: FormId,

    /// interned sentinel symbols.
    pub parent_sym: SymId,
    pub call_sym: SymId,
}

impl World {
    pub fn new() -> Self {
        let mut w = World {
            heap: Heap::new(),
            syms: SymTab::new(),
            chunks: Vec::new(),
            global_env: FormId::NONE,
            object: FormId::NONE,
            nil_proto: FormId::NONE,
            bool_proto: FormId::NONE,
            integer_proto: FormId::NONE,
            symbol_proto: FormId::NONE,
            list_proto: FormId::NONE,
            builtin_proto: FormId::NONE,
            closure_proto: FormId::NONE,
            env_proto: FormId::NONE,
            string_proto: FormId::NONE,
            send_form_proto: FormId::NONE,
            parent_sym: SymId::NONE,
            call_sym: SymId::NONE,
        };
        // the parent-symbol used in env Forms to point to the outer
        // env. chosen to not collide with any user-level name.
        w.parent_sym = w.syms.intern("__parent__");
        w.call_sym = w.syms.intern("call");
        crate::protos::install(&mut w);
        w
    }

    /// determine the *proto* of a value (for IC keys / introspection).
    pub fn proto_of(&self, v: Value) -> FormId {
        match v {
            Value::Nil => self.nil_proto,
            Value::Bool(_) => self.bool_proto,
            Value::Int(_) => self.integer_proto,
            Value::Sym(_) => self.symbol_proto,
            Value::Form(id) => self.heap.get(id).proto,
        }
    }

    /// where the proto-chain walk should *start* for a given receiver
    /// during message dispatch.
    pub fn dispatch_start(&self, v: Value) -> FormId {
        match v {
            Value::Nil => self.nil_proto,
            Value::Bool(_) => self.bool_proto,
            Value::Int(_) => self.integer_proto,
            Value::Sym(_) => self.symbol_proto,
            Value::Form(id) => id,
        }
    }

    /// store a chunk, returning its id.
    pub fn add_chunk(&mut self, chunk: Chunk) -> ChunkId {
        let id = self.chunks.len() as u32;
        self.chunks.push(chunk);
        ChunkId(id)
    }

    /// allocate a callable Form whose `:call` handler is the given
    /// rust function. used by `protos::install` for builtins.
    pub fn alloc_native_callable(&mut self, native: crate::form::NativeFn) -> FormId {
        let mut form = Form::with_proto(self.builtin_proto);
        form.handlers
            .insert(self.call_sym, MethodImpl::Native(native));
        self.heap.alloc(form)
    }

    /// allocate a String Form with the given UTF-8 contents.
    /// (concepts/strings.md: String is a leaf type with Tab-like
    /// interface, internally a UTF-8 byte buffer.)
    pub fn alloc_string(&mut self, s: &str) -> FormId {
        let mut form = Form::with_proto(self.string_proto);
        form.bytes = Some(Box::from(s));
        self.heap.alloc(form)
    }

    /// returns the UTF-8 contents if this value is a String Form.
    pub fn as_str<'a>(&'a self, v: Value) -> Option<&'a str> {
        match v {
            Value::Form(id) => self.heap.get(id).bytes.as_deref(),
            _ => None,
        }
    }

    // ── env helpers ──────────────────────────────────────────────

    /// allocate a fresh env Form with the given parent.
    /// pass `Value::Nil` for a root env.
    pub fn alloc_env(&mut self, parent: Value) -> FormId {
        let mut form = Form::with_proto(self.env_proto);
        form.slots.insert(self.parent_sym, parent);
        self.heap.alloc(form)
    }

    /// look up a name in the env chain, walking `__parent__` slots.
    pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
        let mut cur = env;
        loop {
            let f = self.heap.get(cur);
            if let Some(v) = f.slots.get(&name) {
                return Some(*v);
            }
            match f.slots.get(&self.parent_sym) {
                Some(Value::Form(parent_id)) => cur = *parent_id,
                _ => return None,
            }
        }
    }

    /// define (or rebind) a name in the *innermost* env. shadows
    /// any same-named binding in outer envs.
    pub fn env_define(&mut self, env: FormId, name: SymId, value: Value) {
        self.heap.get_mut(env).slots.insert(name, value);
    }

    /// set an *existing* binding in the env chain. errors if no env
    /// in the chain already binds the name. used by `set!`.
    pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> Result<(), String> {
        let mut cur = env;
        loop {
            if self.heap.get(cur).slots.contains_key(&name) {
                self.heap.get_mut(cur).slots.insert(name, value);
                return Ok(());
            }
            let parent = match self.heap.get(cur).slots.get(&self.parent_sym) {
                Some(Value::Form(parent_id)) => *parent_id,
                _ => return Err(format!("set!: undefined: {}", self.syms.name(name))),
            };
            cur = parent;
        }
    }

    /// convenience: define a top-level binding in `global_env`.
    pub fn define_global(&mut self, name: SymId, value: Value) {
        self.env_define(self.global_env, name, value);
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
