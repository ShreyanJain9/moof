//! the world — the substrate's per-vat root.
//!
//! holds the heap, the symbol table, the foreign-handle table, the
//! proto registry, and the bytecode side-tables (chunk ops/consts/
//! ics, native-method function pointers).
//!
//! phase A is single-vat; the `World` *is* the lone vat. phase B
//! splits this into per-vat instances.
//!
//! ## chunks and methods, the side-table model
//!
//! per `docs/concepts/forms.md`, chunks and methods are *Forms*.
//! reflection (`[m bytecodes]`, `[chunk source]`) reads through
//! ordinary slot access. but for the interpreter to be tolerable
//! at phase A — without paying allocation per opcode — we keep the
//! `Vec<Op>` in a side table indexed by chunk-FormId, and analogously
//! for native-method function pointers indexed by method-FormId.
//!
//! both are conceptually `:ops` / `:native-fn` slots on the
//! corresponding Forms; the side tables are the substrate's *cache*
//! for them. phase G+ may migrate the canonical storage into
//! `ForeignHandle` slot values, but the moof interface
//! (`[m bytecodes]`, etc.) stays the same.

use std::collections::HashMap;

use crate::foreign::ForeignTable;
use crate::form::{Form, FormId};
use crate::heap::Heap;
use crate::opcodes::Op;
use crate::protos::Protos;
use crate::reader::{self, ReadCtx, ReadError};
use crate::sym::{SymId, SymTable};
use crate::value::Value;
use crate::vm::Vm;

/// the signature of a native method bound by a phase-A intrinsic
/// or, later, by an mco.
///
/// returns a new value or a [`RaiseError`] (caught by the dispatcher
/// or propagated out of the world).
pub type NativeFn =
    fn(&mut World, /* self */ Value, /* args */ &[Value]) -> Result<Value, RaiseError>;

/// an inline-cache slot for one `Op::Send` site. monomorphic only
/// at phase A; polymorphic ICs (a small array of entries) come in
/// phase G if hot-path measurements demand it.
///
/// invalidation is per `docs/laws/substrate-laws.md` L10: when the
/// substrate's `bump_proto_generation` runs (via `set-handler!`),
/// every IC whose `cached_generation` no longer matches the proto's
/// current generation re-resolves on next dispatch.
#[derive(Copy, Clone, Default, Debug)]
pub struct ICache {
    /// the proto-FormId this site last resolved against, or
    /// [`FormId::NONE`] if unresolved.
    pub cached_proto: FormId,
    /// the resolved method-FormId, or [`FormId::NONE`].
    pub cached_method: FormId,
    /// the proto's generation at the time of caching.
    pub cached_generation: u32,
}

/// a raised error — propagated up the call stack until caught.
#[derive(Clone, Debug)]
pub struct RaiseError {
    pub kind: SymId,
    pub message: String,
    pub data: Value,
}

impl RaiseError {
    pub fn new(kind: SymId, message: impl Into<String>) -> Self {
        RaiseError {
            kind,
            message: message.into(),
            data: Value::Nil,
        }
    }

    pub fn from_reader(syms: &mut SymTable, e: ReadError) -> Self {
        let kind = syms.intern("read-error");
        RaiseError::new(kind, e.message)
    }
}

impl std::fmt::Display for RaiseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RaiseError {}

/// the substrate's per-vat root. owns everything.
pub struct World {
    pub heap: Heap,
    pub syms: SymTable,
    pub foreign: ForeignTable,
    pub protos: Protos,

    /// chunk-FormId → its bytecode op vector.
    pub chunk_ops: HashMap<FormId, Vec<Op>>,
    /// chunk-FormId → its constant pool.
    pub chunk_consts: HashMap<FormId, Vec<Value>>,
    /// chunk-FormId → its inline-cache slot table (one per Send op).
    pub chunk_ics: HashMap<FormId, Vec<ICache>>,

    /// method-FormId → native function pointer.
    pub native_fns: HashMap<FormId, NativeFn>,

    /// per-proto generation counters. bumped on `set-handler!` to
    /// invalidate inline caches. ICs check the cached generation
    /// against the current value; mismatch triggers re-resolution.
    pub proto_generations: HashMap<FormId, u32>,

    /// the world's global environment Form.
    pub global_env: FormId,

    /// the bytecode interpreter's per-vat state.
    pub vm: Vm,

    // cached SymIds for hot paths.
    pub head_sym: SymId,
    pub tail_sym: SymId,
    pub body_sym: SymId,
    pub source_sym: SymId,
    pub params_sym: SymId,
    pub env_sym: SymId,
    pub parent_sym: SymId,
    pub bindings_sym: SymId,
    pub dnu_sym: SymId,
    pub quote_sym: SymId,
}

impl World {
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let mut syms = SymTable::new();
        let foreign = ForeignTable::new();

        let protos = Protos::bootstrap(&mut heap);

        // an env-Form serves as the world's globals.
        let mut global_env_form = Form::with_proto(Value::Form(protos.env));
        let parent_sym = syms.intern("parent");
        global_env_form.meta.insert(parent_sym, Value::Nil);
        let global_env = heap.alloc(global_env_form);

        let head_sym = syms.intern("head");
        let tail_sym = syms.intern("tail");
        let body_sym = syms.intern("body");
        let source_sym = syms.intern("source");
        let params_sym = syms.intern("params");
        let env_sym = syms.intern("env");
        let bindings_sym = syms.intern("bindings");
        let dnu_sym = syms.intern("does-not-understand:with:");
        let quote_sym = syms.intern("quote");

        World {
            heap,
            syms,
            foreign,
            protos,
            chunk_ops: HashMap::new(),
            chunk_consts: HashMap::new(),
            chunk_ics: HashMap::new(),
            native_fns: HashMap::new(),
            proto_generations: HashMap::new(),
            global_env,
            vm: Vm::default(),
            head_sym,
            tail_sym,
            body_sym,
            source_sym,
            params_sym,
            env_sym,
            parent_sym,
            bindings_sym,
            dnu_sym,
            quote_sym,
        }
    }

    /// intern a symbol.
    pub fn intern(&mut self, name: &str) -> SymId {
        self.syms.intern(name)
    }

    /// resolve a symbol to its text.
    pub fn resolve(&self, sym: SymId) -> &str {
        self.syms.resolve(sym)
    }

    /// allocate a Form.
    pub fn alloc(&mut self, form: Form) -> FormId {
        self.heap.alloc(form)
    }

    /// the proto of any value — substrate-laws.md L1's tagged-
    /// immediate-with-implicit-proto reading.
    pub fn proto_of(&self, value: Value) -> Value {
        match value {
            Value::Nil => Value::Form(self.protos.nil),
            Value::Bool(_) => Value::Form(self.protos.bool_),
            Value::Int(_) => Value::Form(self.protos.integer),
            Value::Sym(_) => Value::Form(self.protos.symbol),
            Value::Form(id) => self.heap.get(id).proto,
            Value::Foreign(_) => Value::Form(self.protos.foreign),
        }
    }

    /// build a moof list from a slice of values. `head`/`tail`
    /// cons-cells, terminated by `nil`. matches the reader's
    /// canonical shape.
    pub fn make_list(&mut self, values: &[Value]) -> Value {
        let mut tail = Value::Nil;
        let list_proto = Value::Form(self.protos.list);
        for &v in values.iter().rev() {
            let mut cell = Form::with_proto(list_proto);
            cell.slots.insert(self.head_sym, v);
            cell.slots.insert(self.tail_sym, tail);
            let id = self.heap.alloc(cell);
            tail = Value::Form(id);
        }
        tail
    }

    /// walk a list-Form into a `Vec<Value>`. errors if `value`
    /// isn't a well-formed list.
    pub fn list_to_vec(&self, value: Value) -> Result<Vec<Value>, &'static str> {
        let mut out = Vec::new();
        let mut cur = value;
        loop {
            match cur {
                Value::Nil => return Ok(out),
                Value::Form(id) => {
                    let f = self.heap.get(id);
                    out.push(f.slot(self.head_sym));
                    cur = f.slot(self.tail_sym);
                }
                _ => return Err("not a list"),
            }
        }
    }

    /// `len` of a list-Form. errors if `value` isn't a list.
    pub fn list_len(&self, value: Value) -> Result<usize, &'static str> {
        let mut n = 0;
        let mut cur = value;
        loop {
            match cur {
                Value::Nil => return Ok(n),
                Value::Form(id) => {
                    n += 1;
                    cur = self.heap.get(id).slot(self.tail_sym);
                }
                _ => return Err("not a list"),
            }
        }
    }

    /// look up a name in an env chain.
    pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
        let mut cur = env;
        loop {
            let f = self.heap.get(cur);
            if let Some(v) = f.slots.get(&name).copied() {
                return Some(v);
            }
            // walk parent
            let parent = f.meta.get(&self.parent_sym).copied().unwrap_or(Value::Nil);
            match parent {
                Value::Nil => return None,
                Value::Form(id) => cur = id,
                _ => return None,
            }
        }
    }

    /// bind a name in an env's local scope (does not walk parents).
    pub fn env_bind(&mut self, env: FormId, name: SymId, value: Value) {
        self.heap.get_mut(env).slots.insert(name, value);
    }

    /// allocate a fresh env-Form chained off `parent` (or `Nil`
    /// for a root env).
    pub fn alloc_env(&mut self, parent: Option<FormId>) -> FormId {
        let mut f = Form::with_proto(Value::Form(self.protos.env));
        let parent_v = parent.map_or(Value::Nil, Value::Form);
        f.meta.insert(self.parent_sym, parent_v);
        self.heap.alloc(f)
    }

    /// install a native method on `proto` under `selector`.
    /// allocates a method-Form whose proto is `Method`, records
    /// the function pointer in `native_fns`, and inserts the
    /// method-Form into `proto`'s handler table.
    pub fn install_native(
        &mut self,
        proto: FormId,
        selector: &str,
        native_fn: NativeFn,
    ) -> FormId {
        let sel_id = self.intern(selector);
        let method_form = Form::with_proto(Value::Form(self.protos.method));
        let method_id = self.heap.alloc(method_form);
        // tag the method with its name in :source so reflection has
        // *something* to show. proper natives carry a real source
        // form; the bare-rust intrinsics installed at boot use the
        // selector symbol as a placeholder.
        let sym_v = Value::Sym(sel_id);
        self.heap
            .get_mut(method_id)
            .meta
            .insert(self.source_sym, sym_v);
        self.native_fns.insert(method_id, native_fn);
        self.heap
            .get_mut(proto)
            .handlers
            .insert(sel_id, Value::Form(method_id));
        method_id
    }

    /// reader entry — uses the canonical List proto.
    pub fn read(&mut self, text: &str) -> Result<Value, ReadError> {
        let list_proto = Value::Form(self.protos.list);
        let mut ctx = ReadCtx::new(&mut self.heap, &mut self.syms, list_proto);
        reader::read(text, &mut ctx)
    }

    /// reader-all entry.
    pub fn read_all(&mut self, text: &str) -> Result<Vec<Value>, ReadError> {
        let list_proto = Value::Form(self.protos.list);
        let mut ctx = ReadCtx::new(&mut self.heap, &mut self.syms, list_proto);
        reader::read_all(text, &mut ctx)
    }

    /// the current generation for `proto_id`. zero is the default
    /// for a never-mutated proto.
    pub fn proto_generation(&self, proto_id: FormId) -> u32 {
        self.proto_generations.get(&proto_id).copied().unwrap_or(0)
    }

    /// bump a proto's generation counter. call after any handler-
    /// table mutation so existing inline caches invalidate.
    pub fn bump_proto_generation(&mut self, proto_id: FormId) {
        let entry = self.proto_generations.entry(proto_id).or_insert(0);
        *entry = entry.wrapping_add(1);
    }

    /// look up a handler by walking the proto chain. returns the
    /// (method-Form, defining-proto-FormId) pair, or `None` if no
    /// handler is found before the chain bottoms out.
    ///
    /// per `docs/concepts/objects-and-protos.md`, lookup checks the
    /// receiver's *own* handler table first (so one-off
    /// object-literal overrides shadow inherited methods, and so
    /// proto-Forms — like `Object` itself — dispatch off their own
    /// handler table since their `proto` is `nil`). then walks
    /// `receiver.proto`, `receiver.proto.proto`, …
    pub fn lookup_handler(
        &self,
        receiver: Value,
        selector: SymId,
    ) -> Option<(Value, FormId)> {
        // 1. receiver's own handlers (Form receivers only — tagged
        //    immediates have no own table).
        if let Value::Form(id) = receiver {
            if let Some(handler) = self.heap.get(id).handler(selector) {
                return Some((handler, id));
            }
        }
        // 2. walk the proto chain.
        let mut proto = self.proto_of(receiver);
        while let Value::Form(proto_id) = proto {
            let f = self.heap.get(proto_id);
            if let Some(handler) = f.handler(selector) {
                return Some((handler, proto_id));
            }
            proto = f.proto;
        }
        None
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_boots_with_protos() {
        let w = World::new();
        // every proto resolves to a real Form.
        let p = w.protos;
        for id in [p.object, p.nil, p.bool_, p.integer, p.symbol, p.list,
                   p.method, p.chunk, p.closure, p.env, p.foreign] {
            assert!(!id.is_none());
        }
    }

    #[test]
    fn proto_of_tagged_immediates() {
        let w = World::new();
        let p = w.protos;
        assert_eq!(w.proto_of(Value::Nil), Value::Form(p.nil));
        assert_eq!(w.proto_of(Value::Bool(true)), Value::Form(p.bool_));
        assert_eq!(w.proto_of(Value::Int(42)), Value::Form(p.integer));
        assert_eq!(w.proto_of(Value::Sym(SymId(1))), Value::Form(p.symbol));
    }

    #[test]
    fn proto_of_form_returns_form_proto() {
        let mut w = World::new();
        // an Integer-proto-instance Form
        let f = Form::with_proto(Value::Form(w.protos.integer));
        let id = w.alloc(f);
        assert_eq!(w.proto_of(Value::Form(id)), Value::Form(w.protos.integer));
    }

    #[test]
    fn make_list_and_list_to_vec_roundtrip() {
        let mut w = World::new();
        let xs = vec![Value::Int(1), Value::Int(2), Value::Int(3)];
        let list = w.make_list(&xs);
        let back = w.list_to_vec(list).unwrap();
        assert_eq!(xs, back);
    }

    #[test]
    fn list_len_works() {
        let mut w = World::new();
        let xs = vec![Value::Int(1), Value::Int(2), Value::Int(3)];
        let list = w.make_list(&xs);
        assert_eq!(w.list_len(list).unwrap(), 3);
        assert_eq!(w.list_len(Value::Nil).unwrap(), 0);
    }

    #[test]
    fn env_lookup_walks_parents() {
        let mut w = World::new();
        let outer = w.alloc_env(None);
        let inner = w.alloc_env(Some(outer));
        let foo = w.intern("foo");
        let bar = w.intern("bar");
        w.env_bind(outer, foo, Value::Int(1));
        w.env_bind(inner, bar, Value::Int(2));
        // outer's foo is reachable from inner.
        assert_eq!(w.env_lookup(inner, foo), Some(Value::Int(1)));
        // inner's bar is reachable from inner.
        assert_eq!(w.env_lookup(inner, bar), Some(Value::Int(2)));
        // bar is *not* visible from outer.
        assert_eq!(w.env_lookup(outer, bar), None);
        // unbound name is None.
        let baz = w.intern("baz");
        assert_eq!(w.env_lookup(inner, baz), None);
    }

    #[test]
    fn env_inner_shadows_outer() {
        let mut w = World::new();
        let outer = w.alloc_env(None);
        let inner = w.alloc_env(Some(outer));
        let x = w.intern("x");
        w.env_bind(outer, x, Value::Int(10));
        w.env_bind(inner, x, Value::Int(20));
        assert_eq!(w.env_lookup(inner, x), Some(Value::Int(20)));
        assert_eq!(w.env_lookup(outer, x), Some(Value::Int(10)));
    }

    #[test]
    fn lookup_handler_walks_proto_chain() {
        let mut w = World::new();
        let foo = w.intern("foo");
        // install foo on Object as a handler-stub Form
        let stub = w.alloc(Form::with_proto(Value::Form(w.protos.method)));
        w.heap
            .get_mut(w.protos.object)
            .handlers
            .insert(foo, Value::Form(stub));
        // lookup from any Integer instance reaches Object's foo.
        let result = w.lookup_handler(Value::Int(5), foo);
        let (handler, defining) = result.unwrap();
        assert_eq!(handler, Value::Form(stub));
        assert_eq!(defining, w.protos.object);
    }

    #[test]
    fn lookup_handler_misses_return_none() {
        let w = World::new();
        let mystery = SymId(9999); // not interned in this world
        // dispatch on Integer for a selector with no handler anywhere
        // returns None.
        assert!(w.lookup_handler(Value::Int(5), mystery).is_none());
    }

    #[test]
    fn install_native_records_function_and_handler() {
        let mut w = World::new();
        fn echo(_: &mut World, self_: Value, args: &[Value]) -> Result<Value, RaiseError> {
            // returns its first arg or self if no args.
            Ok(args.first().copied().unwrap_or(self_))
        }
        let method_id = w.install_native(w.protos.integer, "test-echo", echo);
        // method-Form is a Method.
        assert_eq!(
            w.heap.get(method_id).proto,
            Value::Form(w.protos.method)
        );
        // it's installed on Integer's handler table.
        let sel = w.intern("test-echo");
        assert_eq!(
            w.heap.get(w.protos.integer).handler(sel),
            Some(Value::Form(method_id))
        );
        // the function pointer is in the side table.
        assert!(w.native_fns.contains_key(&method_id));
    }

    #[test]
    fn world_reader_uses_list_proto() {
        let mut w = World::new();
        let v = w.read("(1 2 3)").unwrap();
        let id = v.as_form_id().unwrap();
        assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.list));
    }
}
