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

use indexmap::IndexMap;

use crate::foreign::ForeignTable;
use crate::form::{Form, FormId};
use crate::heap::Heap;
use crate::opcodes::Op;
use crate::protos::Protos;
use crate::reader::{self, ReadCtx, ReadError};
use crate::sym::{SymId, SymTable};
use crate::value::Value;
use crate::vm::Vm;

/// destructor for a `Box<Vec<u8>>` minted by `make_string`. runs
/// when the gc collects the holding String form.
unsafe extern "C" fn string_bytes_dtor(ptr: *mut std::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr was minted by `Box::into_raw(Box<Vec<u8>>)` in
    // `World::make_string`. consume it back into a Box and let
    // it drop.
    let _ = unsafe { Box::from_raw(ptr as *mut Vec<u8>) };
}

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
    /// the proto on which `cached_method` was found — used by
    /// `super` sends from the method's body.
    pub cached_defining: FormId,
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

    /// chunk-FormId → its bytecode op vector. `IndexMap` so that
    /// any iteration is deterministic per `laws/determinism-laws.md`
    /// D5 — replicas must agree on iteration order even for
    /// substrate-internal tables.
    pub chunk_ops: IndexMap<FormId, Vec<Op>>,
    /// chunk-FormId → its constant pool.
    pub chunk_consts: IndexMap<FormId, Vec<Value>>,
    /// chunk-FormId → its inline-cache slot table (one per Send op).
    pub chunk_ics: IndexMap<FormId, Vec<ICache>>,

    /// method-FormId → native function pointer.
    pub native_fns: IndexMap<FormId, NativeFn>,

    /// per-tagged-immediate singleton-Forms (ruby/Self style).
    /// when moof code first writes to a tagged immediate (`(slotSet!
    /// 5 'foo 42)` / `(setHandler! 5 'sel fn)` / etc.), the
    /// substrate lazily allocates a fresh Form whose proto is the
    /// value's class-level proto (Integer / Bool / Char / …) and
    /// records it here, keyed by the Value. subsequent reads and
    /// writes target that Form. dispatch on the immediate also
    /// consults its per-instance handler table first before
    /// walking the proto chain.
    ///
    /// **the ruby move**: writing to `5` doesn't mutate `Integer`;
    /// it mutates `5`'s singleton-Form. `5` and `7` are different
    /// objects with different per-instance state, even though
    /// they share the Integer class for inherited methods.
    /// matches `5.define_singleton_method(:foo) { … }` semantics.
    ///
    /// memory note: allocated lazily, never garbage-collected
    /// (phase G+ adds proper gc). user discipline needed if
    /// large numbers of distinct Ints get singleton state.
    pub tagged_storage: IndexMap<Value, FormId>,

    /// proto-FormId → its instantiated wasm module + store. set by
    /// the wasm mco loader (see `crate::wasm`). a moof-method
    /// dispatch on this proto's handler table routes through the
    /// wasm trampoline, which looks the proto up here.
    pub wasm_instances: IndexMap<FormId, crate::wasm::WasmInstance>,

    /// (proto-FormId, selector-SymId) → (export-name, shape). lets
    /// the wasm trampoline figure out which wasm function to call
    /// for the dispatched selector.
    pub wasm_export_map:
        IndexMap<(FormId, SymId), (String, crate::wasm::ExportShape)>,

    /// the `Macros` Form — canonical store of registered macros.
    /// each slot is `name -> method-Form`. exposed as a global so
    /// moof code can do `[Macros slots]` to list all macros,
    /// `[Macros at: 'when]` to fetch one, `[[Macros at: 'when]
    /// source]` to read its source. honors reflection-contract R6:
    /// the macro registry IS a Form, not a rust hash.
    ///
    /// the `:macro?` helper exists in moof; the rust line just owns
    /// the slot table as the canonical lookup table — same shape as
    /// any other Form's slots.
    pub macros_form: FormId,

    // proto generation counters live on each proto Form's `:meta`
    // table under the `generation` key. honors reflection-contract
    // R6 ("if the rust line stores state about a Form, it must be
    // exposed through reflection"). reads via `proto_generation`,
    // writes via `bump_proto_generation`.

    /// the world's global environment Form.
    pub global_env: FormId,

    /// when `true`, [`crate::compiler::compile`] delegates to the
    /// moof-side `compile-top` (defined in `lib/compiler.moof`).
    /// when `false`, the rust compiler runs.
    ///
    /// the bootstrap dance: starts `false`, rust compiler compiles
    /// `compiler.moof` into the world, then `lib.rs` flips this to
    /// `true`, then `bootstrap.moof` loads via the moof compiler.
    /// **after that, every compile in this world routes through
    /// moof.** the rust compiler is dead code post-flip.
    ///
    /// see `docs/process/self-hosted-compiler.md` for the full
    /// dance. the rust compiler's residual surface is exactly
    /// what compiler.moof itself uses (def, fn, if, let, do,
    /// quote, __send__) — minimal seed.
    pub use_moof_compiler: bool,

    /// Resolved root for [$transporter load: ...] calls. Populated at
    /// `new_world()` via `transporter::resolve_lib_root`. None means
    /// the transporter cap will raise 'tx-no-root on every call —
    /// used by `new_world_bare` for tests that don't need bootstrap.
    pub transporter_root: Option<std::path::PathBuf>,

    /// the bytecode interpreter's per-vat state.
    pub vm: Vm,

    // cached SymIds for hot paths.
    pub car_sym: SymId,
    pub cdr_sym: SymId,
    pub body_sym: SymId,
    pub source_sym: SymId,
    pub params_sym: SymId,
    pub env_sym: SymId,
    pub parent_sym: SymId,
    pub bindings_sym: SymId,
    pub dnu_sym: SymId,
    pub quote_sym: SymId,
    pub bytes_sym: SymId,
    pub rep_sym: SymId,
    pub generation_sym: SymId,
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

        // the canonical macro registry: a plain Form (proto: Object)
        // whose slots are macro-name -> method-Form. exposed as the
        // `Macros` global so user code can introspect it.
        let macros_form = heap.alloc(Form::with_proto(Value::Form(protos.object)));

        let car_sym = syms.intern("car");
        let cdr_sym = syms.intern("cdr");
        let body_sym = syms.intern("body");
        let source_sym = syms.intern("source");
        let params_sym = syms.intern("params");
        let env_sym = syms.intern("env");
        let bindings_sym = syms.intern("bindings");
        let dnu_sym = syms.intern("doesNotUnderstand:with:");
        let quote_sym = syms.intern("quote");
        let bytes_sym = syms.intern("bytes");
        let rep_sym = syms.intern("rep");
        let generation_sym = syms.intern("generation");

        World {
            heap,
            syms,
            foreign,
            protos,
            chunk_ops: IndexMap::new(),
            chunk_consts: IndexMap::new(),
            chunk_ics: IndexMap::new(),
            native_fns: IndexMap::new(),
            tagged_storage: IndexMap::new(),
            wasm_instances: IndexMap::new(),
            wasm_export_map: IndexMap::new(),
            macros_form,
            global_env,
            transporter_root: None,
            use_moof_compiler: false,
            vm: Vm::default(),
            car_sym,
            cdr_sym,
            body_sym,
            source_sym,
            params_sym,
            env_sym,
            parent_sym,
            bindings_sym,
            dnu_sym,
            quote_sym,
            bytes_sym,
            rep_sym,
            generation_sym,
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

    /// allocate a fresh String form with the given UTF-8 bytes.
    /// the bytes are owned by a ForeignHandle whose destructor
    /// frees the underlying `Vec<u8>` on gc.
    pub fn make_string(&mut self, text: &str) -> Value {
        use crate::foreign::ForeignHandle;
        let boxed: Box<Vec<u8>> = Box::new(text.as_bytes().to_vec());
        let ptr = Box::into_raw(boxed) as *mut std::ffi::c_void;
        let handle_id = self.foreign.alloc(ForeignHandle {
            ptr,
            destructor: Some(string_bytes_dtor),
            tag: crate::foreign::TAG_STRING_BYTES,
        });
        let mut form = Form::with_proto(Value::Form(self.protos.string));
        form.slots.insert(self.bytes_sym, Value::Foreign(handle_id));
        Value::Form(self.alloc(form))
    }

    /// borrow a String form's UTF-8 bytes. returns `None` if
    /// `value` isn't a well-formed String.
    pub fn string_bytes(&self, value: Value) -> Option<&[u8]> {
        let id = value.as_form_id()?;
        let f = self.heap.get(id);
        if f.proto != Value::Form(self.protos.string) {
            return None;
        }
        let fid = match f.slot(self.bytes_sym) {
            Value::Foreign(fid) => fid,
            _ => return None,
        };
        let handle = self.foreign.get(fid);
        if handle.tag != crate::foreign::TAG_STRING_BYTES {
            return None;
        }
        // SAFETY: tag-check confirms make_string minted this; cast
        // back. the pointer outlives the holding Form (gc invariant).
        unsafe {
            let v: &Vec<u8> = &*(handle.ptr as *const Vec<u8>);
            Some(v.as_slice())
        }
    }

    /// borrow a String form's text as `&str`. `None` if not a
    /// String or if bytes aren't valid UTF-8.
    pub fn string_text(&self, value: Value) -> Option<&str> {
        std::str::from_utf8(self.string_bytes(value)?).ok()
    }

    /// allocate a fresh empty Table form. the `:rep` slot holds a
    /// ForeignHandle wrapping a `Box<TableRepr>`.
    pub fn make_table(&mut self) -> Value {
        use crate::foreign::{ForeignHandle, TAG_TABLE_REPR};
        use crate::table::{table_repr_dtor, TableRepr};
        let boxed: Box<TableRepr> = Box::new(TableRepr::new());
        let ptr = Box::into_raw(boxed) as *mut std::ffi::c_void;
        let handle_id = self.foreign.alloc(ForeignHandle {
            ptr,
            destructor: Some(table_repr_dtor),
            tag: TAG_TABLE_REPR,
        });
        let mut form = Form::with_proto(Value::Form(self.protos.table));
        form.slots.insert(self.rep_sym, Value::Foreign(handle_id));
        Value::Form(self.alloc(form))
    }

    /// borrow a Table form's `TableRepr`. returns `None` if `value`
    /// isn't a well-formed Table.
    pub fn table_repr(&self, value: Value) -> Option<&crate::table::TableRepr> {
        use crate::foreign::TAG_TABLE_REPR;
        use crate::table::TableRepr;
        let id = value.as_form_id()?;
        let f = self.heap.get(id);
        if f.proto != Value::Form(self.protos.table) {
            return None;
        }
        let fid = match f.slot(self.rep_sym) {
            Value::Foreign(fid) => fid,
            _ => return None,
        };
        let handle = self.foreign.get(fid);
        if handle.tag != TAG_TABLE_REPR {
            return None;
        }
        // SAFETY: tag confirms make_table minted this; cast back.
        unsafe { Some(&*(handle.ptr as *const TableRepr)) }
    }

    /// mutable borrow of a Table form's `TableRepr`. analogous to
    /// `table_repr`.
    pub fn table_repr_mut(
        &mut self,
        value: Value,
    ) -> Option<&mut crate::table::TableRepr> {
        use crate::foreign::TAG_TABLE_REPR;
        use crate::table::TableRepr;
        let id = value.as_form_id()?;
        if self.heap.get(id).proto != Value::Form(self.protos.table) {
            return None;
        }
        let fid = match self.heap.get(id).slot(self.rep_sym) {
            Value::Foreign(fid) => fid,
            _ => return None,
        };
        let handle = self.foreign.get(fid);
        if handle.tag != TAG_TABLE_REPR {
            return None;
        }
        // SAFETY: same; we have exclusive access via &mut self.
        unsafe { Some(&mut *(handle.ptr as *mut TableRepr)) }
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
            Value::Float(_) => Value::Form(self.protos.float),
            Value::Sym(_) => Value::Form(self.protos.symbol),
            Value::Char(_) => Value::Form(self.protos.char_),
            Value::Form(id) => self.heap.get(id).proto,
            Value::Foreign(_) => Value::Form(self.protos.foreign),
        }
    }

    /// build a moof list from a slice of values. `head`/`tail`
    /// cons-cells, terminated by `nil`. matches the reader's
    /// canonical shape.
    pub fn make_list(&mut self, values: &[Value]) -> Value {
        let mut tail = Value::Nil;
        let list_proto = Value::Form(self.protos.cons);
        for &v in values.iter().rev() {
            let mut cell = Form::with_proto(list_proto);
            cell.slots.insert(self.car_sym, v);
            cell.slots.insert(self.cdr_sym, tail);
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
                    out.push(f.slot(self.car_sym));
                    cur = f.slot(self.cdr_sym);
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
                    cur = self.heap.get(id).slot(self.cdr_sym);
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

    /// `set!` semantics: walk the parent chain looking for an
    /// existing binding of `name`; if found, mutate it in place and
    /// return `true`. if not found anywhere, return `false` —
    /// the caller decides whether to define-locally or raise.
    ///
    /// matches scheme's classical set! (where set!ing an unbound
    /// name is undefined-behavior / error). load-bearing for
    /// closures-capture-env-by-reference: the environment frame
    /// where a name *was originally bound* is what `set!` must
    /// touch, not whatever frame happens to be active at the call
    /// site (which may shadow the original).
    pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> bool {
        let mut cur = env;
        loop {
            if self.heap.get(cur).slots.contains_key(&name) {
                self.heap.get_mut(cur).slots.insert(name, value);
                return true;
            }
            let parent = self
                .heap
                .get(cur)
                .meta
                .get(&self.parent_sym)
                .copied()
                .unwrap_or(Value::Nil);
            match parent {
                Value::Form(id) => cur = id,
                _ => return false,
            }
        }
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

    /// reader entry — uses the canonical List + String protos.
    pub fn read(&mut self, text: &str) -> Result<Value, ReadError> {
        let list_proto = Value::Form(self.protos.cons);
        let string_proto = Value::Form(self.protos.string);
        let mut ctx = ReadCtx::new(
            &mut self.heap,
            &mut self.syms,
            &mut self.foreign,
            list_proto,
            string_proto,
        );
        reader::read(text, &mut ctx)
    }

    /// reader-all entry.
    pub fn read_all(&mut self, text: &str) -> Result<Vec<Value>, ReadError> {
        let list_proto = Value::Form(self.protos.cons);
        let string_proto = Value::Form(self.protos.string);
        let mut ctx = ReadCtx::new(
            &mut self.heap,
            &mut self.syms,
            &mut self.foreign,
            list_proto,
            string_proto,
        );
        reader::read_all(text, &mut ctx)
    }

    /// materialize the rust `Vm::Frame` at index `idx` as a Form
    /// snapshot. honors reflection-contract.md R3 — the moof view
    /// of a frame is a Form with proto `Frame` carrying slots for
    /// `chunk`, `pc`, `env`, `self`, `stack-base`, `defining-proto`.
    ///
    /// this is a snapshot (parallels the existing `:slots` /
    /// `:handlers` / `:meta` pattern). live-views are a phase-C
    /// follow-up. reading the snapshot tells you what the frame
    /// looked like at the moment of materialization; mutations to
    /// the snapshot do not propagate to the running frame.
    ///
    /// returns `None` if `idx` is out of bounds.
    pub fn frame_snapshot(&mut self, idx: usize) -> Option<Value> {
        let frame = self.vm.frames.get(idx)?.clone();
        let chunk_sym = self.intern("chunk");
        let pc_sym = self.intern("pc");
        let env_sym = self.intern("env");
        let self_sym = self.intern("self");
        let stack_base_sym = self.intern("stack-base");
        let defining_sym = self.intern("defining-proto");
        let mut snap = Form::with_proto(Value::Form(self.protos.frame));
        snap.slots.insert(chunk_sym, Value::Form(frame.chunk));
        snap.slots.insert(pc_sym, Value::Int(frame.pc as i64));
        snap.slots.insert(env_sym, Value::Form(frame.env));
        snap.slots.insert(self_sym, frame.self_);
        snap.slots
            .insert(stack_base_sym, Value::Int(frame.stack_base as i64));
        let defining = if frame.defining_proto.is_none() {
            Value::Nil
        } else {
            Value::Form(frame.defining_proto)
        };
        snap.slots.insert(defining_sym, defining);
        Some(Value::Form(self.heap.alloc(snap)))
    }

    /// snapshot the entire VM frame stack as a List of frame-Forms.
    /// outermost (index 0) frame first. returns Nil for an empty
    /// stack.
    pub fn frame_stack_snapshot(&mut self) -> Value {
        let n = self.vm.frames.len();
        if n == 0 {
            return Value::Nil;
        }
        let mut snaps = Vec::with_capacity(n);
        for i in 0..n {
            // frame_snapshot can't fail: i < n.
            snaps.push(self.frame_snapshot(i).unwrap());
        }
        self.make_list(&snaps)
    }

    /// look up a macro by name. returns the method-Form Value
    /// (a `Value::Form`), or `None` if no macro is registered.
    ///
    /// reads from the canonical `Macros` Form's slots — the same
    /// table moof code reads via `[Macros at: name]`.
    pub fn macro_at(&self, name: SymId) -> Option<Value> {
        let f = self.heap.get(self.macros_form);
        if f.slot_present(name) {
            Some(f.slot(name))
        } else {
            None
        }
    }

    /// register a macro: install `method` under `name` in the
    /// canonical `Macros` Form.
    pub fn macro_register(&mut self, name: SymId, method: Value) {
        self.heap
            .get_mut(self.macros_form)
            .slots
            .insert(name, method);
    }

    /// the current generation for `proto_id`. zero is the default
    /// for a never-mutated proto.
    ///
    /// stored in the proto Form's `:meta` table under the
    /// `generation` key (so reflection-contract R6 holds: this is
    /// state-about-a-Form, exposed via the reflection protocol —
    /// `[proto meta at: 'generation]` works from inside moof).
    pub fn proto_generation(&self, proto_id: FormId) -> u32 {
        match self.heap.get(proto_id).meta_at(self.generation_sym) {
            Value::Int(n) => n as u32,
            _ => 0,
        }
    }

    /// bump a proto's generation counter. call after any handler-
    /// table mutation so existing inline caches invalidate.
    ///
    /// writes to the proto Form's `:meta at: 'generation` slot.
    pub fn bump_proto_generation(&mut self, proto_id: FormId) {
        let cur = self.proto_generation(proto_id);
        let next = cur.wrapping_add(1);
        self.heap
            .get_mut(proto_id)
            .meta
            .insert(self.generation_sym, Value::Int(next as i64));
    }

    /// the FormId where this value's per-instance state lives, if
    /// it has any. for `Value::Form(id)`, that's `id` directly.
    /// for tagged immediates, that's the lazily-allocated
    /// singleton-Form recorded in `tagged_storage`, if it exists.
    /// otherwise `None` — which means "no per-instance state yet."
    ///
    /// the read path (slot, slots, handlers, meta, dispatch's "own
    /// handlers" check) consults this and returns empty/nil/falls-
    /// through-to-proto when None. matches Ruby's distinction:
    /// `5.instance_variables` is `[]` until you set one; reflection
    /// shows the singleton's state, not the class's.
    pub fn effective_form_id(&self, v: Value) -> Option<FormId> {
        if let Value::Form(id) = v {
            return Some(id);
        }
        // short-circuit: most programs never write to immediates.
        if self.tagged_storage.is_empty() {
            return None;
        }
        self.tagged_storage.get(&v).copied()
    }

    /// the FormId where this value's per-instance state should be
    /// written. for `Value::Form(id)`, returns `id`. for tagged
    /// immediates without a singleton-Form, **lazily allocates one**
    /// — its proto is the value's class-level proto (Integer,
    /// Bool, etc), so dispatch from the singleton walks: singleton
    /// → class → Object. matches Ruby `define_singleton_method`.
    ///
    /// allocation is intentional and unbounded — phase A has no gc.
    /// large numbers of singleton-Form'd Ints will accumulate.
    /// phase G+ tackles gc.
    pub fn ensure_writable_form_id(&mut self, v: Value) -> FormId {
        if let Value::Form(id) = v {
            return id;
        }
        if let Some(&id) = self.tagged_storage.get(&v) {
            return id;
        }
        // allocate a fresh singleton-Form whose proto is `v`'s
        // class-level proto.
        let proto = self.proto_of(v);
        let mut f = Form::with_proto(proto);
        // tag the singleton-Form so reflection / debugging can
        // recognize it. user code mostly doesn't care; the meta
        // is informational.
        let singleton_meta = self.intern("singleton-of");
        f.meta.insert(singleton_meta, v);
        let id = self.heap.alloc(f);
        self.tagged_storage.insert(v, id);
        id
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
    ///
    /// per `docs/laws/substrate-laws.md` L2, the proto chain must
    /// be acyclic. this method aborts cleanly after `MAX_PROTO_DEPTH`
    /// hops. with no `set-proto!` primitive at phase A, cycles can
    /// only arise from rust-side mistakes; bound is purely defensive.
    ///
    /// when receiver is a tagged immediate that has a singleton-
    /// Form (lazily allocated by past mutations), dispatch starts
    /// at the singleton-Form — handlers installed via `(setHandler!
    /// 5 …)` shadow inherited Integer methods. matches Ruby's
    /// singleton-class lookup.
    pub fn lookup_handler(
        &self,
        receiver: Value,
        selector: SymId,
    ) -> Option<(Value, FormId)> {
        // 1. receiver's own (or singleton's own) handler table.
        let own_id = self.effective_form_id(receiver);
        if let Some(id) = own_id {
            if let Some(handler) = self.heap.get(id).handler(selector) {
                return Some((handler, id));
            }
        }
        // 2. walk the proto chain. starts from `own_id`'s proto
        //    when it exists (so the singleton's class chain is
        //    respected), else from `proto_of(receiver)` (the
        //    classic tagged-immediate case).
        let mut proto = match own_id {
            Some(id) => self.heap.get(id).proto,
            None => self.proto_of(receiver),
        };
        const MAX_PROTO_DEPTH: usize = 256;
        for _ in 0..MAX_PROTO_DEPTH {
            match proto {
                Value::Form(proto_id) => {
                    let f = self.heap.get(proto_id);
                    if let Some(handler) = f.handler(selector) {
                        return Some((handler, proto_id));
                    }
                    proto = f.proto;
                }
                _ => return None,
            }
        }
        None
    }

    /// look up a handler starting *above* `defining_proto` —
    /// implements `super` send. used when a method that lives on
    /// `defining_proto` wants to delegate to its parent's method.
    pub fn lookup_handler_super(
        &self,
        defining_proto: FormId,
        selector: SymId,
    ) -> Option<(Value, FormId)> {
        let mut proto = self.heap.get(defining_proto).proto;
        const MAX_PROTO_DEPTH: usize = 256;
        for _ in 0..MAX_PROTO_DEPTH {
            match proto {
                Value::Form(proto_id) => {
                    let f = self.heap.get(proto_id);
                    if let Some(handler) = f.handler(selector) {
                        return Some((handler, proto_id));
                    }
                    proto = f.proto;
                }
                _ => return None,
            }
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
        for id in [p.object, p.nil, p.bool_, p.integer, p.symbol, p.cons,
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
        let cons = w.make_list(&xs);
        let back = w.list_to_vec(cons).unwrap();
        assert_eq!(xs, back);
    }

    #[test]
    fn list_len_works() {
        let mut w = World::new();
        let xs = vec![Value::Int(1), Value::Int(2), Value::Int(3)];
        let cons = w.make_list(&xs);
        assert_eq!(w.list_len(cons).unwrap(), 3);
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
        assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.cons));
    }
}
