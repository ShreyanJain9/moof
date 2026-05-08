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

use std::collections::HashSet;

use indexmap::IndexMap;

use crate::foreign::ForeignTable;
use crate::form::{Form, FormId};
use crate::heap::Heap;
use crate::nursery::{Delta, FaceKind, TurnDiff};
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

/// destructor for a `Box<Vec<u8>>` minted by `make_bytes`. mirrors
/// `string_bytes_dtor` — same payload type, different tag.
unsafe extern "C" fn bytes_dtor(ptr: *mut std::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr was minted by `Box::into_raw(Box<Vec<u8>>)` in
    // `World::make_bytes`. consume it back into a Box and let it drop.
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
    /// the bootstrap dance: starts `false`, the rust seed compiler
    /// compiles `compiler.moof` (loaded via `$transporter` from
    /// `lib/main.moof`), then `lib/main.moof` sends `[$compiler useMoof]`
    /// to flip this to `true`, then `bootstrap.moof` loads via the
    /// moof compiler.
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
    /// None only when a `World` is constructed via `World::new()` directly
    /// without going through `new_world` or `new_world_bare` — the
    /// `tx-no-root` test path.
    pub transporter_root: Option<std::path::PathBuf>,

    /// the bytecode interpreter's per-vat state.
    pub vm: Vm,

    /// the current turn's mutation deltas, keyed by FormId of
    /// pre-existing forms (payload < `turn_watermark`). forms
    /// allocated this turn are NOT in this map — they're at
    /// `heap.forms[i]` for `i >= turn_watermark`. cleared on
    /// commit and abort.
    pub nursery_deltas: IndexMap<FormId, Delta>,

    /// the FormId payload below which forms are canonical
    /// (committed in a prior turn or at boot). forms with
    /// payload `>= turn_watermark` are this-turn allocations
    /// during an active turn. advanced on commit; unchanged on
    /// abort.
    pub turn_watermark: u32,

    /// `true` iff a turn is currently active. `start_turn`
    /// flips on; `commit_turn` and `abort_turn` flip off.
    /// nested `start_turn` calls panic — V1 supports exactly
    /// one active turn at a time.
    pub in_turn: bool,

    /// V2 — protos whose forms refuse `world.freeze` and raise
    /// `'cannot-freeze-live`. liveness is a property of the proto
    /// chain (vat-Forms have Vat proto, mailbox-Forms have Mailbox
    /// proto, etc.) — `world.freeze` walks the chain and refuses
    /// if any ancestor is in this set. populated at boot in
    /// `intrinsics.rs::install` with cap-bearing protos. V4+ phases
    /// add Vat / Mailbox / DataSource protos.
    pub live_protos: HashSet<FormId>,

    /// V2 — current vat mode. `:new` consults this in
    /// `intrinsics.rs::install` to decide whether to seal-after-
    /// initialize. defaults to `MutableByDefault`.
    pub vat_mode: crate::VatMode,

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

        let mut world = World {
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
            nursery_deltas: IndexMap::new(),
            turn_watermark: 0,
            in_turn: false,
            live_protos: HashSet::new(),
            vat_mode: crate::VatMode::MutableByDefault,
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
        };

        // boot turn auto-commit: all allocations during World::new
        // are treated as committed canonical state. turn_watermark
        // reflects this. (the equivalent of "start_turn → bootstrap
        // → commit_turn" with the diff discarded.)
        world.turn_watermark = world.heap.len() as u32;

        world
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

    /// allocate a Bytes form with the given raw byte buffer.
    /// no utf-8 invariant — any byte sequence is valid. mirrors
    /// `make_string` exactly, using `TAG_BYTES` and the Bytes proto.
    pub fn make_bytes(&mut self, data: &[u8]) -> Value {
        use crate::foreign::ForeignHandle;
        let boxed: Box<Vec<u8>> = Box::new(data.to_vec());
        let ptr = Box::into_raw(boxed) as *mut std::ffi::c_void;
        let handle_id = self.foreign.alloc(ForeignHandle {
            ptr,
            destructor: Some(bytes_dtor),
            tag: crate::foreign::TAG_BYTES,
        });
        let mut form = Form::with_proto(Value::Form(self.protos.bytes));
        form.slots.insert(self.bytes_sym, Value::Foreign(handle_id));
        Value::Form(self.alloc(form))
    }

    /// borrow a Bytes form's raw byte buffer. returns `None` if
    /// `value` isn't a well-formed Bytes form.
    pub fn bytes_data(&self, value: Value) -> Option<&[u8]> {
        let id = value.as_form_id()?;
        let f = self.heap.get(id);
        if f.proto != Value::Form(self.protos.bytes) {
            return None;
        }
        let fid = match f.slot(self.bytes_sym) {
            Value::Foreign(fid) => fid,
            _ => return None,
        };
        let handle = self.foreign.get(fid);
        if handle.tag != crate::foreign::TAG_BYTES {
            return None;
        }
        // SAFETY: tag-check confirms make_bytes minted this; cast
        // back. the pointer outlives the holding Form (gc invariant).
        unsafe {
            let v: &Vec<u8> = &*(handle.ptr as *const Vec<u8>);
            Some(v.as_slice())
        }
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
    ///
    /// nursery-aware: when in a turn, checks the per-form delta
    /// before canonical, since `env_set` / `env_bind` write to the
    /// delta. note we can't just use `form_slot` here — `env_lookup`
    /// must distinguish `Some(Value::Nil)` (bound to nil) from
    /// `None` (unbound), and `form_slot` collapses both to
    /// `Value::Nil`. so we do the dual check explicitly.
    pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
        let mut cur = env;
        loop {
            // delta first (only meaningful for pre-existing forms in a turn).
            if self.in_turn && cur.payload() < self.turn_watermark {
                if let Some(delta) = self.nursery_deltas.get(&cur) {
                    if let Some(v) = delta.slots.get(&name).copied() {
                        return Some(v);
                    }
                }
            }
            let f = self.heap.get(cur);
            if let Some(v) = f.slots.get(&name).copied() {
                return Some(v);
            }
            // walk parent — nursery-aware via form_meta.
            let parent = self.form_meta(cur, self.parent_sym);
            match parent {
                Value::Nil => return None,
                Value::Form(id) => cur = id,
                _ => return None,
            }
        }
    }

    /// bind a name in an env's local scope (does not walk parents).
    pub fn env_bind(&mut self, env: FormId, name: SymId, value: Value) -> Result<(), RaiseError> {
        self.form_slot_set(env, name, value)
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
    pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> Result<bool, RaiseError> {
        let mut cur = env;
        loop {
            // contains_key semantics: present in delta OR canonical.
            // form_slot collapses absent and bound-to-nil; we need
            // explicit dual-check here so set! on a nil-bound name
            // hits, but set! on an unbound name walks parent.
            let bound_in_delta = self
                .nursery_deltas
                .get(&cur)
                .map(|d| d.slots.contains_key(&name))
                .unwrap_or(false);
            let bound_in_canonical = self.heap.get(cur).slots.contains_key(&name);
            if bound_in_delta || bound_in_canonical {
                self.form_slot_set(cur, name, value)?;
                return Ok(true);
            }
            // walk parent — nursery-aware.
            let parent = self.form_meta(cur, self.parent_sym);
            match parent {
                Value::Form(id) => cur = id,
                _ => return Ok(false),
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
    ) -> Result<FormId, RaiseError> {
        let sel_id = self.intern(selector);
        let method_form = Form::with_proto(Value::Form(self.protos.method));
        let method_id = self.heap.alloc(method_form);
        // tag the method with its name in :source so reflection has
        // *something* to show. proper natives carry a real source
        // form; the bare-rust intrinsics installed at boot use the
        // selector symbol as a placeholder.
        let sym_v = Value::Sym(sel_id);
        // method_id is freshly allocated this turn (above the
        // watermark) — form_meta_set writes directly to canonical.
        self.form_meta_set(method_id, self.source_sym, sym_v)?;
        self.native_fns.insert(method_id, native_fn);
        // proto may be pre-existing — form_handler_set buffers in
        // the delta when so, writes directly when above watermark.
        self.form_handler_set(proto, sel_id, Value::Form(method_id))?;
        Ok(method_id)
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
        // dual-check like env_lookup: must distinguish absent from
        // bound-to-nil (slot_present semantics), so we check the
        // delta's contains_key first, then canonical's slot_present.
        let id = self.macros_form;
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.slots.get(&name).copied() {
                    return Some(v);
                }
            }
        }
        let f = self.heap.get(id);
        if f.slot_present(name) {
            Some(f.slot(name))
        } else {
            None
        }
    }

    /// register a macro: install `method` under `name` in the
    /// canonical `Macros` Form.
    pub fn macro_register(&mut self, name: SymId, method: Value) -> Result<(), RaiseError> {
        self.form_slot_set(self.macros_form, name, method)
    }

    /// the current generation for `proto_id`. zero is the default
    /// for a never-mutated proto.
    ///
    /// stored in the proto Form's `:meta` table under the
    /// `generation` key (so reflection-contract R6 holds: this is
    /// state-about-a-Form, exposed via the reflection protocol —
    /// `[proto meta at: 'generation]` works from inside moof).
    pub fn proto_generation(&self, proto_id: FormId) -> u32 {
        match self.form_meta(proto_id, self.generation_sym) {
            Value::Int(n) => n as u32,
            _ => 0,
        }
    }

    /// bump a proto's generation counter. call after any handler-
    /// table mutation so existing inline caches invalidate.
    ///
    /// writes to the proto Form's `:meta at: 'generation` slot.
    pub fn bump_proto_generation(&mut self, proto_id: FormId) -> Result<(), RaiseError> {
        let cur = self.proto_generation(proto_id);
        let next = cur.wrapping_add(1);
        self.form_meta_set(proto_id, self.generation_sym, Value::Int(next as i64))
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

    /// `true` iff a turn is currently active.
    pub fn in_turn(&self) -> bool {
        self.in_turn
    }

    /// begin a turn. panics if a turn is already active —
    /// V1 supports exactly one active turn at a time.
    pub fn start_turn(&mut self) {
        assert!(
            !self.in_turn,
            "start_turn called while a turn is already active"
        );
        self.in_turn = true;
        // nursery_deltas should already be empty (clear on commit/abort);
        // assert defensively.
        debug_assert!(self.nursery_deltas.is_empty());
    }

    /// commit the active turn. computes and returns the
    /// `TurnDiff`. applies nursery deltas to canonical heap.
    /// advances `turn_watermark` to current heap length.
    /// clears `nursery_deltas`. flips `in_turn` off.
    /// panics if no turn is active.
    pub fn commit_turn(&mut self) -> TurnDiff {
        assert!(
            self.in_turn,
            "commit_turn called outside a turn"
        );

        let mut diff = TurnDiff::default();

        // process deltas: read canonical prior, emit diff entry,
        // apply mutation. order is `IndexMap` insertion order,
        // which is deterministic per `laws/determinism-laws.md` D5.
        for (form_id, delta) in std::mem::take(&mut self.nursery_deltas) {
            let canonical = self.heap.get_mut(form_id);

            for (key, new_value) in delta.slots {
                let prior = canonical
                    .slots
                    .get(&key)
                    .copied()
                    .unwrap_or(Value::Nil);
                diff.mutations.insert(
                    (form_id, FaceKind::Slots, key),
                    (prior, new_value),
                );
                canonical.slots.insert(key, new_value);
            }
            for (key, new_value) in delta.handlers {
                let prior = canonical
                    .handlers
                    .get(&key)
                    .copied()
                    .unwrap_or(Value::Nil);
                diff.mutations.insert(
                    (form_id, FaceKind::Handlers, key),
                    (prior, new_value),
                );
                canonical.handlers.insert(key, new_value);
            }
            for (key, new_value) in delta.meta {
                let prior = canonical
                    .meta
                    .get(&key)
                    .copied()
                    .unwrap_or(Value::Nil);
                diff.mutations.insert(
                    (form_id, FaceKind::Meta, key),
                    (prior, new_value),
                );
                canonical.meta.insert(key, new_value);
            }

            // V2 — frozen-bit transition. only emit a freezings entry
            // for pre-existing forms (below the *previous* watermark);
            // forms allocated AND frozen in the same turn are already
            // captured by new_allocs. note: the watermark advance
            // happens after this loop, so `self.turn_watermark` here
            // still reads the pre-turn value.
            if delta.frozen {
                if !canonical.frozen {
                    canonical.frozen = true;
                }
                if form_id.payload() < self.turn_watermark {
                    diff.freezings.push(form_id);
                }
            }
        }

        // collect new-alloc FormIds (allocations during this turn
        // sit at `heap.forms[turn_watermark..]`).
        let new_high = self.heap.len() as u32;
        diff.new_allocs = (self.turn_watermark..new_high)
            .map(FormId::vat_local)
            .collect();

        // advance watermark to include this turn's allocs.
        self.turn_watermark = new_high;
        self.in_turn = false;

        diff
    }

    /// abort the active turn. truncates `heap.forms` to
    /// `turn_watermark` (drops this-turn allocations). clears
    /// `nursery_deltas` (drops buffered mutations). flips
    /// `in_turn` off. watermark unchanged. panics if no turn
    /// is active.
    pub fn abort_turn(&mut self) {
        assert!(
            self.in_turn,
            "abort_turn called outside a turn"
        );

        // drop new-alloc forms by truncating Vec to watermark.
        // this is the rollback for allocations.
        self.heap.forms.truncate(self.turn_watermark as usize);

        // drop buffered mutations (no canonical writes occurred).
        self.nursery_deltas.clear();

        self.in_turn = false;
    }

    /// read a form's slot value, nursery-aware. checks nursery
    /// delta first when the form is pre-existing and a turn is
    /// active; falls through to canonical heap otherwise.
    /// returns `Value::Nil` if the slot is absent in both
    /// nursery delta (if any) and canonical (matching `Form::slot`'s
    /// behavior).
    pub fn form_slot(&self, id: FormId, key: SymId) -> Value {
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.slots.get(&key).copied() {
                    return v;
                }
            }
        }
        self.heap.get(id).slot(key)
    }

    /// read a form's handler entry, nursery-aware. returns
    /// `None` if absent in both nursery delta and canonical
    /// (matching `Form::handler`'s behavior — callers walking
    /// the proto chain rely on `None` to keep walking).
    pub fn form_handler(&self, id: FormId, key: SymId) -> Option<Value> {
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.handlers.get(&key).copied() {
                    return Some(v);
                }
            }
        }
        self.heap.get(id).handler(key)
    }

    /// read a form's meta entry, nursery-aware. returns
    /// `Value::Nil` if absent in both nursery delta and
    /// canonical (matching `Form::meta_at`'s behavior).
    pub fn form_meta(&self, id: FormId, key: SymId) -> Value {
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.meta.get(&key).copied() {
                    return v;
                }
            }
        }
        self.heap.get(id).meta_at(key)
    }

    /// query the frozen bit on a form, nursery-aware.
    /// returns `true` if the canonical `Form.frozen` is `true`,
    /// OR (during a turn, for pre-existing forms below the
    /// watermark) if the form's nursery `Delta.frozen` is `true`.
    /// V2's mutation guard inside `form_*_set` calls this to
    /// decide whether to raise `'frozen-form`.
    pub fn is_frozen(&self, id: FormId) -> bool {
        if self.heap.get(id).frozen {
            return true;
        }
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if delta.frozen {
                    return true;
                }
            }
        }
        false
    }

    /// query liveness — walks the proto chain from `id` upward
    /// and returns `true` if any ancestor proto is in
    /// `live_protos`. used by `freeze` to refuse vat-Forms /
    /// mailbox-Forms / DataSource handles / cap-tokens.
    pub fn is_live(&self, id: FormId) -> bool {
        const MAX_PROTO_DEPTH: usize = 256;
        let mut cur = Value::Form(id);
        for _ in 0..MAX_PROTO_DEPTH {
            match cur {
                Value::Form(fid) => {
                    if self.live_protos.contains(&fid) {
                        return true;
                    }
                    cur = self.heap.get(fid).proto;
                }
                _ => return false,
            }
        }
        // depth-exceeded — conservative "not live"; pathological
        // proto chains are a rust-side bug, not a substrate-policy
        // signal. mirrors the discipline of `lookup_handler` /
        // `lookup_handler_super` in this file.
        false
    }

    /// query "can this form be frozen?" — `true` iff the form is
    /// neither already frozen nor live. lets policy code branch
    /// without try / raise / catch.
    pub fn freezable(&self, id: FormId) -> bool {
        !self.is_frozen(id) && !self.is_live(id)
    }

    /// freeze a form — set its `frozen` bit, journaling through the
    /// nursery as a turn-mutation. one-way; there is no thaw.
    /// raises `'cannot-freeze-live` (FormId in `data`) if the form's
    /// proto chain crosses any registered `live_protos` proto
    /// (vat-Forms, mailbox-Forms, cap-tokens). idempotent:
    /// already-frozen forms return `Ok(())` without re-checking
    /// liveness — so a form frozen long ago whose chain has since
    /// crossed a live proto doesn't suddenly start raising.
    pub fn freeze(&mut self, id: FormId) -> Result<(), RaiseError> {
        assert!(self.in_turn, "freeze called outside a turn");
        // already frozen — idempotent no-op (also avoids a bogus
        // 'cannot-freeze-live raise on a form that's already frozen
        // and happens to inherit from a now-mutable proto).
        if self.is_frozen(id) {
            return Ok(());
        }
        // V2 task-5: refuse forms whose proto chain hits live_protos.
        if self.is_live(id) {
            let kind = self.intern("cannot-freeze-live");
            let mut err = RaiseError::new(
                kind,
                "cannot freeze form: proto chain includes a live (mutable-by-design) proto",
            );
            err.data = Value::Form(id);
            return Err(err);
        }
        if id.payload() >= self.turn_watermark {
            // new alloc — write directly to canonical (analogous to
            // form_*_set's fast path for above-watermark forms).
            self.heap.get_mut(id).frozen = true;
        } else {
            // pre-existing — buffer in the nursery delta.
            self.nursery_deltas
                .entry(id)
                .or_default()
                .frozen = true;
        }
        Ok(())
    }

    /// set a slot value on a form, nursery-aware. for
    /// pre-existing forms (payload < turn_watermark) during an
    /// active turn, writes to the nursery delta. for new-alloc
    /// forms (payload >= turn_watermark), writes directly to
    /// canonical heap (they're already nursery-semantic).
    /// panics if `!in_turn` — substrate disallows mutation
    /// outside a turn (V1 invariant: turn = unit of atomicity).
    pub fn form_slot_set(&mut self, id: FormId, key: SymId, value: Value) -> Result<(), RaiseError> {
        assert!(
            self.in_turn,
            "form_slot_set called outside a turn"
        );
        // V2 task-7 — frozen guard. raise immediately at call site
        // per spec §4. FormId travels in `data` for diagnostic /
        // pattern-match use.
        if self.is_frozen(id) {
            let kind = self.intern("frozen-form");
            let mut err = RaiseError::new(kind, "mutation on frozen form (slots)");
            err.data = Value::Form(id);
            return Err(err);
        }
        if id.payload() >= self.turn_watermark {
            // new alloc — write directly to canonical.
            self.heap.get_mut(id).slots.insert(key, value);
        } else {
            // pre-existing — buffer in nursery delta.
            self.nursery_deltas
                .entry(id)
                .or_default()
                .slots
                .insert(key, value);
        }
        Ok(())
    }

    /// set a handler entry on a form, nursery-aware. semantics
    /// mirror `form_slot_set`.
    pub fn form_handler_set(&mut self, id: FormId, key: SymId, value: Value) -> Result<(), RaiseError> {
        assert!(
            self.in_turn,
            "form_handler_set called outside a turn"
        );
        // V2 task-7 — frozen guard. raise immediately at call site
        // per spec §4. FormId travels in `data` for diagnostic /
        // pattern-match use.
        if self.is_frozen(id) {
            let kind = self.intern("frozen-form");
            let mut err = RaiseError::new(kind, "mutation on frozen form (handlers)");
            err.data = Value::Form(id);
            return Err(err);
        }
        if id.payload() >= self.turn_watermark {
            self.heap.get_mut(id).handlers.insert(key, value);
        } else {
            self.nursery_deltas
                .entry(id)
                .or_default()
                .handlers
                .insert(key, value);
        }
        Ok(())
    }

    /// set a meta entry on a form, nursery-aware. semantics
    /// mirror `form_slot_set`.
    pub fn form_meta_set(&mut self, id: FormId, key: SymId, value: Value) -> Result<(), RaiseError> {
        assert!(
            self.in_turn,
            "form_meta_set called outside a turn"
        );
        // V2 task-7 — frozen guard. raise immediately at call site
        // per spec §4. FormId travels in `data` for diagnostic /
        // pattern-match use.
        if self.is_frozen(id) {
            let kind = self.intern("frozen-form");
            let mut err = RaiseError::new(kind, "mutation on frozen form (meta)");
            err.data = Value::Form(id);
            return Err(err);
        }
        if id.payload() >= self.turn_watermark {
            self.heap.get_mut(id).meta.insert(key, value);
        } else {
            self.nursery_deltas
                .entry(id)
                .or_default()
                .meta
                .insert(key, value);
        }
        Ok(())
    }

    /// list slot keys for a form, nursery-aware. union of canonical's
    /// slot keys and the nursery delta's slot keys (during a turn,
    /// for pre-existing forms only). preserves insertion order:
    /// canonical first, then delta keys not already in canonical
    /// (D5 determinism).
    pub fn form_slot_keys(&self, id: FormId) -> Vec<SymId> {
        let mut keys: Vec<SymId> = self.heap.get(id).slots.keys().copied().collect();
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                for k in delta.slots.keys() {
                    if !keys.contains(k) {
                        keys.push(*k);
                    }
                }
            }
        }
        keys
    }

    /// handler keys, nursery-aware. analogous to `form_slot_keys`.
    pub fn form_handler_keys(&self, id: FormId) -> Vec<SymId> {
        let mut keys: Vec<SymId> = self.heap.get(id).handlers.keys().copied().collect();
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                for k in delta.handlers.keys() {
                    if !keys.contains(k) {
                        keys.push(*k);
                    }
                }
            }
        }
        keys
    }

    /// meta keys, nursery-aware. analogous to `form_slot_keys`.
    pub fn form_meta_keys(&self, id: FormId) -> Vec<SymId> {
        let mut keys: Vec<SymId> = self.heap.get(id).meta.keys().copied().collect();
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                for k in delta.meta.keys() {
                    if !keys.contains(k) {
                        keys.push(*k);
                    }
                }
            }
        }
        keys
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
        //    nursery-aware via form_handler.
        let own_id = self.effective_form_id(receiver);
        if let Some(id) = own_id {
            if let Some(handler) = self.form_handler(id, selector) {
                return Some((handler, id));
            }
        }
        // 2. walk the proto chain. starts from `own_id`'s proto
        //    when it exists (so the singleton's class chain is
        //    respected), else from `proto_of(receiver)` (the
        //    classic tagged-immediate case). proto is a struct
        //    field on Form, not a slot — never buffered through
        //    the nursery, so direct heap reads are correct.
        let mut proto = match own_id {
            Some(id) => self.heap.get(id).proto,
            None => self.proto_of(receiver),
        };
        const MAX_PROTO_DEPTH: usize = 256;
        for _ in 0..MAX_PROTO_DEPTH {
            match proto {
                Value::Form(proto_id) => {
                    if let Some(handler) = self.form_handler(proto_id, selector) {
                        return Some((handler, proto_id));
                    }
                    proto = self.heap.get(proto_id).proto;
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
        // proto is a struct field, not a slot — direct heap read.
        let mut proto = self.heap.get(defining_proto).proto;
        const MAX_PROTO_DEPTH: usize = 256;
        for _ in 0..MAX_PROTO_DEPTH {
            match proto {
                Value::Form(proto_id) => {
                    if let Some(handler) = self.form_handler(proto_id, selector) {
                        return Some((handler, proto_id));
                    }
                    proto = self.heap.get(proto_id).proto;
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
        // env_bind goes through form_slot_set which requires in_turn.
        w.start_turn();
        let outer = w.alloc_env(None);
        let inner = w.alloc_env(Some(outer));
        let foo = w.intern("foo");
        let bar = w.intern("bar");
        w.env_bind(outer, foo, Value::Int(1)).expect("env_bind in mutable test");
        w.env_bind(inner, bar, Value::Int(2)).expect("env_bind in mutable test");
        // outer's foo is reachable from inner.
        assert_eq!(w.env_lookup(inner, foo), Some(Value::Int(1)));
        // inner's bar is reachable from inner.
        assert_eq!(w.env_lookup(inner, bar), Some(Value::Int(2)));
        // bar is *not* visible from outer.
        assert_eq!(w.env_lookup(outer, bar), None);
        // unbound name is None.
        let baz = w.intern("baz");
        assert_eq!(w.env_lookup(inner, baz), None);
        let _ = w.commit_turn();
    }

    #[test]
    fn env_inner_shadows_outer() {
        let mut w = World::new();
        // env_bind goes through form_slot_set which requires in_turn.
        w.start_turn();
        let outer = w.alloc_env(None);
        let inner = w.alloc_env(Some(outer));
        let x = w.intern("x");
        w.env_bind(outer, x, Value::Int(10)).expect("env_bind in mutable test");
        w.env_bind(inner, x, Value::Int(20)).expect("env_bind in mutable test");
        assert_eq!(w.env_lookup(inner, x), Some(Value::Int(20)));
        assert_eq!(w.env_lookup(outer, x), Some(Value::Int(10)));
        let _ = w.commit_turn();
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
        // install_native writes :source meta and a handler entry —
        // both go through nursery-aware setters that require in_turn.
        w.start_turn();
        let method_id = w.install_native(w.protos.integer, "test-echo", echo).expect("install_native in mutable test");
        let _ = w.commit_turn();
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

    #[test]
    fn boot_turn_commits_cleanly() {
        let w = World::new();
        // not in a turn (boot turn committed).
        assert!(!w.in_turn());
        // watermark advanced past all bootstrap allocations.
        assert!(
            w.turn_watermark > 1,
            "watermark must include at least the bootstrap forms (got {})",
            w.turn_watermark
        );
        // nursery is empty (boot turn drained on commit).
        assert!(w.nursery_deltas.is_empty());
    }

    #[test]
    fn start_turn_flips_in_turn_on_and_commit_flips_off() {
        let mut w = World::new();
        assert!(!w.in_turn());
        w.start_turn();
        assert!(w.in_turn());
        let _diff = w.commit_turn();
        assert!(!w.in_turn());
    }

    #[test]
    fn start_turn_then_abort_flips_in_turn_off() {
        let mut w = World::new();
        w.start_turn();
        assert!(w.in_turn());
        w.abort_turn();
        assert!(!w.in_turn());
    }

    #[test]
    #[should_panic(expected = "start_turn called while a turn is already active")]
    fn nested_start_turn_panics() {
        let mut w = World::new();
        w.start_turn();
        w.start_turn();
    }

    #[test]
    #[should_panic(expected = "commit_turn called outside a turn")]
    fn commit_turn_outside_a_turn_panics() {
        let mut w = World::new();
        w.commit_turn();
    }

    #[test]
    #[should_panic(expected = "abort_turn called outside a turn")]
    fn abort_turn_outside_a_turn_panics() {
        let mut w = World::new();
        w.abort_turn();
    }

    #[test]
    fn empty_turn_commit_returns_empty_diff() {
        let mut w = World::new();
        w.start_turn();
        let diff = w.commit_turn();
        assert!(diff.mutations.is_empty());
        assert!(diff.new_allocs.is_empty());
    }

    #[test]
    fn turn_watermark_advances_on_commit_for_new_allocs() {
        use crate::form::Form;
        let mut w = World::new();
        let mark_before = w.turn_watermark;
        w.start_turn();
        w.heap.alloc(Form::default());
        let diff = w.commit_turn();
        assert_eq!(diff.new_allocs.len(), 1);
        // watermark moved up by 1 to include the new alloc.
        assert_eq!(w.turn_watermark, mark_before + 1);
    }

    #[test]
    fn turn_abort_truncates_new_allocs() {
        use crate::form::Form;
        let mut w = World::new();
        let mark_before = w.turn_watermark;
        let len_before = w.heap.len();
        w.start_turn();
        let _ = w.heap.alloc(Form::default());
        let _ = w.heap.alloc(Form::default());
        assert_eq!(w.heap.len(), len_before + 2);
        w.abort_turn();
        // after abort, heap is back to pre-turn state.
        assert_eq!(w.heap.len(), len_before);
        // watermark unchanged.
        assert_eq!(w.turn_watermark, mark_before);
    }

    #[test]
    fn form_slot_reads_canonical_when_not_in_turn() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        // not in a turn: form_slot reads canonical directly.
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(99));
    }

    #[test]
    fn form_slot_falls_through_to_canonical_when_no_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        // advance watermark so id is "pre-existing" relative to the next turn.
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // no delta; falls through to canonical.
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(99));
        w.commit_turn();
    }

    #[test]
    fn form_slot_reads_delta_when_seeded() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // seed nursery_deltas manually for the test.
        let mut d = Delta::default();
        d.slots.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        // form_slot should see the delta value, not canonical's 99.
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(77));
        w.abort_turn();
    }

    #[test]
    fn form_slot_falls_through_when_key_not_in_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        f.slots.insert(SymId(8), Value::Int(88));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // delta touches slot 7 but not 8.
        let mut d = Delta::default();
        d.slots.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(77));
        // slot 8 falls through to canonical.
        assert_eq!(w.form_slot(id, SymId(8)), Value::Int(88));
        w.abort_turn();
    }

    #[test]
    fn form_handler_reads_canonical_when_not_in_turn() {
        let mut w = World::new();
        let mut f = Form::default();
        f.handlers.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        assert_eq!(w.form_handler(id, SymId(7)), Some(Value::Int(99)));
        assert_eq!(w.form_handler(id, SymId(99)), None);
    }

    #[test]
    fn form_handler_reads_delta_when_seeded() {
        let mut w = World::new();
        let mut f = Form::default();
        f.handlers.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.handlers.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        assert_eq!(w.form_handler(id, SymId(7)), Some(Value::Int(77)));
        w.abort_turn();
    }

    #[test]
    fn form_meta_reads_canonical_when_not_in_turn() {
        let mut w = World::new();
        let mut f = Form::default();
        f.meta.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        assert_eq!(w.form_meta(id, SymId(7)), Value::Int(99));
        // missing key returns nil (matches Form::meta_at behavior).
        assert_eq!(w.form_meta(id, SymId(99)), Value::Nil);
    }

    #[test]
    fn form_meta_reads_delta_when_seeded() {
        let mut w = World::new();
        let mut f = Form::default();
        f.meta.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.meta.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        assert_eq!(w.form_meta(id, SymId(7)), Value::Int(77));
        w.abort_turn();
    }

    #[test]
    fn is_frozen_reads_canonical_unfrozen() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        // canonical fresh form is not frozen.
        assert!(!w.is_frozen(id));
    }

    #[test]
    fn is_frozen_reads_canonical_frozen() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.heap.get_mut(id).frozen = true;   // direct write — bypasses nursery for test setup
        assert!(w.is_frozen(id));
    }

    #[test]
    fn is_frozen_reads_delta_when_seeded() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        // simulate "pre-existing form, frozen this turn" by parking
        // the form below watermark and seeding the delta.
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.frozen = true;
        w.nursery_deltas.insert(id, d);
        assert!(w.is_frozen(id));
        w.abort_turn();   // canonical was never touched
        assert!(!w.is_frozen(id));
    }

    #[test]
    fn is_frozen_ignores_delta_when_not_in_turn() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        // craft a stale delta entry without entering a turn.
        let mut d = Delta::default();
        d.frozen = true;
        w.nursery_deltas.insert(id, d);
        // outside a turn, deltas are ignored — defensive against bugs.
        assert!(!w.is_frozen(id));
        // tidy up so other tests don't see the orphan delta.
        w.nursery_deltas.clear();
    }

    #[test]
    fn form_slot_keys_unions_canonical_and_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(1), Value::Int(10));
        f.slots.insert(SymId(2), Value::Int(20));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // delta adds key 3 and overwrites key 2.
        let mut d = Delta::default();
        d.slots.insert(SymId(3), Value::Int(30));
        d.slots.insert(SymId(2), Value::Int(99));
        w.nursery_deltas.insert(id, d);
        let keys = w.form_slot_keys(id);
        // canonical keys 1, 2 first; then delta's new key 3.
        // key 2 is in canonical so no duplicate from delta.
        assert_eq!(keys, vec![SymId(1), SymId(2), SymId(3)]);
        w.abort_turn();
    }

    #[test]
    fn form_handler_keys_unions_canonical_and_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.handlers.insert(SymId(1), Value::Int(10));
        f.handlers.insert(SymId(2), Value::Int(20));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.handlers.insert(SymId(3), Value::Int(30));
        d.handlers.insert(SymId(2), Value::Int(99));
        w.nursery_deltas.insert(id, d);
        let keys = w.form_handler_keys(id);
        assert_eq!(keys, vec![SymId(1), SymId(2), SymId(3)]);
        w.abort_turn();
    }

    #[test]
    fn form_meta_keys_unions_canonical_and_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.meta.insert(SymId(1), Value::Int(10));
        f.meta.insert(SymId(2), Value::Int(20));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.meta.insert(SymId(3), Value::Int(30));
        d.meta.insert(SymId(2), Value::Int(99));
        w.nursery_deltas.insert(id, d);
        let keys = w.form_meta_keys(id);
        assert_eq!(keys, vec![SymId(1), SymId(2), SymId(3)]);
        w.abort_turn();
    }

    #[test]
    #[should_panic(expected = "form_slot_set called outside a turn")]
    fn form_slot_set_outside_turn_panics() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        let _ = w.form_slot_set(id, SymId(1), Value::Int(42));
    }

    #[test]
    #[should_panic(expected = "form_handler_set called outside a turn")]
    fn form_handler_set_outside_turn_panics() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        let _ = w.form_handler_set(id, SymId(1), Value::Int(42));
    }

    #[test]
    #[should_panic(expected = "form_meta_set called outside a turn")]
    fn form_meta_set_outside_turn_panics() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        let _ = w.form_meta_set(id, SymId(1), Value::Int(42));
    }

    #[test]
    fn form_slot_set_buffers_in_delta_for_pre_existing_form() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        // mark id as pre-existing.
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(42)).expect("form_slot_set in mutable test");
        // canonical heap still has empty slots for this form.
        assert!(w.heap.get(id).slots.is_empty());
        // delta should have the entry.
        let delta = w.nursery_deltas.get(&id).unwrap();
        assert_eq!(delta.slots.get(&SymId(1)).copied(), Some(Value::Int(42)));
        // read-your-writes via form_slot.
        assert_eq!(w.form_slot(id, SymId(1)), Value::Int(42));
        w.abort_turn();
    }

    #[test]
    fn form_slot_set_writes_canonical_directly_for_new_alloc() {
        let mut w = World::new();
        // set a watermark; alloc above it.
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let id = w.heap.alloc(Form::default());
        // id.payload() >= turn_watermark, so the form is new-alloc.
        w.form_slot_set(id, SymId(1), Value::Int(42)).expect("form_slot_set in mutable test");
        // canonical heap has the value directly (no delta needed).
        assert_eq!(w.heap.get(id).slot(SymId(1)), Value::Int(42));
        // delta is empty (new-alloc forms don't use the delta map).
        assert!(w.nursery_deltas.get(&id).is_none() || w.nursery_deltas.get(&id).unwrap().is_empty());
        w.commit_turn();
    }

    #[test]
    fn commit_applies_delta_and_emits_diff() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(1), Value::Int(10));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(20)).expect("form_slot_set in mutable test");
        w.form_slot_set(id, SymId(2), Value::Int(30)).expect("form_slot_set in mutable test");
        let diff = w.commit_turn();
        // canonical now has updated values.
        assert_eq!(w.heap.get(id).slot(SymId(1)), Value::Int(20));
        assert_eq!(w.heap.get(id).slot(SymId(2)), Value::Int(30));
        // diff has both entries with correct prior/new.
        let e1 = diff.mutations.get(&(id, FaceKind::Slots, SymId(1))).copied();
        assert_eq!(e1, Some((Value::Int(10), Value::Int(20))));
        let e2 = diff.mutations.get(&(id, FaceKind::Slots, SymId(2))).copied();
        // SymId(2) was absent before — prior is Nil.
        assert_eq!(e2, Some((Value::Nil, Value::Int(30))));
    }

    #[test]
    fn last_write_wins_within_a_turn() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(1)).expect("form_slot_set in mutable test");
        w.form_slot_set(id, SymId(1), Value::Int(2)).expect("form_slot_set in mutable test");
        w.form_slot_set(id, SymId(1), Value::Int(3)).expect("form_slot_set in mutable test");
        let diff = w.commit_turn();
        // diff has the final value 3, with prior nil (key was absent).
        let e = diff.mutations.get(&(id, FaceKind::Slots, SymId(1))).copied();
        assert_eq!(e, Some((Value::Nil, Value::Int(3))));
    }

    #[test]
    fn abort_drops_delta_no_canonical_change() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(1), Value::Int(10));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(99)).expect("form_slot_set in mutable test");
        // mid-turn: read sees new value via delta.
        assert_eq!(w.form_slot(id, SymId(1)), Value::Int(99));
        w.abort_turn();
        // post-abort: canonical untouched.
        assert_eq!(w.heap.get(id).slot(SymId(1)), Value::Int(10));
    }

    #[test]
    fn diff_handles_all_three_faces() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(11)).expect("form_slot_set in mutable test");
        w.form_handler_set(id, SymId(2), Value::Int(22)).expect("form_handler_set in mutable test");
        w.form_meta_set(id, SymId(3), Value::Int(33)).expect("form_meta_set in mutable test");
        let diff = w.commit_turn();
        assert_eq!(diff.mutations.len(), 3);
        assert!(diff.mutations.contains_key(&(id, FaceKind::Slots, SymId(1))));
        assert!(diff.mutations.contains_key(&(id, FaceKind::Handlers, SymId(2))));
        assert!(diff.mutations.contains_key(&(id, FaceKind::Meta, SymId(3))));
    }

    #[test]
    fn raise_in_eval_program_aborts_implicit_turn_no_state_leak() {
        let mut w = crate::new_world_bare();
        let env_id = w.global_env;
        let foo_sym = w.intern("foo");
        let snapshot_before = w.heap.get(env_id).slot(foo_sym);
        assert_eq!(snapshot_before, Value::Nil);

        let result = crate::eval_program(
            &mut w,
            "(def foo 5) (raise: 'boom \"oh no\")",
        );
        assert!(result.is_err());
        // env state preserved post-abort.
        assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Nil);
    }

    #[test]
    fn eval_program_in_turn_state_post_eval_is_not_in_turn() {
        let mut w = crate::new_world_bare();
        // implicit turn wraps the body; on completion, in_turn is false.
        let result = crate::eval_program(&mut w, "(def x 42) x");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Value::Int(42));
        assert!(!w.in_turn());
    }

    #[test]
    fn eval_program_returning_error_leaves_in_turn_false() {
        let mut w = crate::new_world_bare();
        let result = crate::eval_program(&mut w, "(raise: 'boom \"x\")");
        assert!(result.is_err());
        assert!(!w.in_turn());
    }

    #[test]
    fn nested_eval_program_calls_use_outer_turn_idempotently() {
        let mut w = crate::new_world_bare();
        w.start_turn();
        // outer caller already in a turn; eval_program should NOT
        // open a nested turn (idempotent via was_in_turn).
        let _ = crate::eval_program(&mut w, "(def x 1)");
        // still in the outer turn — eval_program didn't commit.
        assert!(w.in_turn());
        w.commit_turn();
        assert!(!w.in_turn());
    }

    #[test]
    fn freeze_new_alloc_writes_canonical_directly() {
        let mut w = World::new();
        w.start_turn();
        let id = w.heap.alloc(Form::default());
        // new alloc — above watermark — freeze writes to canonical.
        let r = w.freeze(id);
        assert!(r.is_ok());
        assert!(w.heap.get(id).frozen);
        // delta should be empty (no entry for this id).
        assert!(!w.nursery_deltas.contains_key(&id));
        let _ = w.commit_turn();
    }

    #[test]
    fn freeze_pre_existing_in_turn_writes_delta() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let r = w.freeze(id);
        assert!(r.is_ok());
        // canonical untouched until commit.
        assert!(!w.heap.get(id).frozen);
        // delta records the freeze.
        assert!(w.nursery_deltas.get(&id).map(|d| d.frozen).unwrap_or(false));
        let _ = w.commit_turn();
    }

    #[test]
    fn freeze_then_commit_lands_in_canonical_and_freezings() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let _ = w.freeze(id).unwrap();
        let diff = w.commit_turn();
        // canonical now frozen.
        assert!(w.heap.get(id).frozen);
        // diff records the transition.
        assert!(diff.freezings.contains(&id));
    }

    #[test]
    fn freeze_then_abort_unfreezes() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let _ = w.freeze(id).unwrap();
        // mid-turn, is_frozen sees true via delta.
        assert!(w.is_frozen(id));
        w.abort_turn();
        // post-abort, canonical was never touched and delta is gone.
        assert!(!w.heap.get(id).frozen);
        assert!(!w.is_frozen(id));
    }

    #[test]
    fn freeze_new_alloc_then_commit_no_freezings_entry() {
        // forms allocated AND frozen in the same turn appear in
        // new_allocs but NOT freezings (the new-alloc list already
        // implies their final state).
        let mut w = World::new();
        w.start_turn();
        let id = w.heap.alloc(Form::default());
        let _ = w.freeze(id).unwrap();
        let diff = w.commit_turn();
        assert!(diff.new_allocs.contains(&id));
        assert!(!diff.freezings.contains(&id));
    }

    #[test]
    fn is_live_returns_false_for_unregistered_proto() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::with_proto(Value::Form(w.protos.object)));
        assert!(!w.is_live(id));
    }

    #[test]
    fn is_live_returns_true_when_proto_in_live_set() {
        let mut w = World::new();
        let custom = w.heap.alloc(Form::default());
        w.live_protos.insert(custom);
        let inst = w.heap.alloc(Form::with_proto(Value::Form(custom)));
        assert!(w.is_live(inst));
    }

    #[test]
    fn is_live_walks_proto_chain() {
        let mut w = World::new();
        let live = w.heap.alloc(Form::default());
        w.live_protos.insert(live);
        // intermediate proto inherits from `live`.
        let mid = w.heap.alloc(Form::with_proto(Value::Form(live)));
        let inst = w.heap.alloc(Form::with_proto(Value::Form(mid)));
        assert!(w.is_live(inst));
    }

    #[test]
    fn freezable_unfrozen_unlive_returns_true() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::with_proto(Value::Form(w.protos.object)));
        assert!(w.freezable(id));
    }

    #[test]
    fn freezable_frozen_returns_false() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.heap.get_mut(id).frozen = true;
        assert!(!w.freezable(id));
    }

    #[test]
    fn freezable_live_returns_false() {
        let mut w = World::new();
        let live = w.heap.alloc(Form::default());
        w.live_protos.insert(live);
        let inst = w.heap.alloc(Form::with_proto(Value::Form(live)));
        assert!(!w.freezable(inst));
    }

    #[test]
    fn freeze_on_live_proto_raises_cannot_freeze_live() {
        let mut w = World::new();
        let live = w.heap.alloc(Form::default());
        w.live_protos.insert(live);
        let inst = w.heap.alloc(Form::with_proto(Value::Form(live)));
        w.start_turn();
        let r = w.freeze(inst);
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert_eq!(w.resolve(err.kind), "cannot-freeze-live");
        // FormId of the offending form travels in `data`.
        assert_eq!(err.data, Value::Form(inst));
        w.abort_turn();
    }

    #[test]
    fn frozen_slot_set_raises_frozen_form() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.start_turn();
        w.freeze(id).unwrap();
        let key = w.intern("x");
        let r = w.form_slot_set(id, key, Value::Int(42));
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert_eq!(w.resolve(err.kind), "frozen-form");
        assert_eq!(err.data, Value::Form(id));
        let _ = w.commit_turn();
    }

    #[test]
    fn frozen_handler_set_raises_frozen_form() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.start_turn();
        w.freeze(id).unwrap();
        let sel = w.intern("foo:");
        let r = w.form_handler_set(id, sel, Value::Nil);
        assert!(r.is_err());
        assert_eq!(w.resolve(r.unwrap_err().kind), "frozen-form");
        let _ = w.commit_turn();
    }

    #[test]
    fn frozen_meta_set_raises_frozen_form() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.start_turn();
        w.freeze(id).unwrap();
        let k = w.intern("source");
        let r = w.form_meta_set(id, k, Value::Nil);
        assert!(r.is_err());
        assert_eq!(w.resolve(r.unwrap_err().kind), "frozen-form");
        let _ = w.commit_turn();
    }

    #[test]
    fn same_turn_freeze_then_mutate_raises_immediately() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.freeze(id).unwrap();
        let key = w.intern("x");
        // before commit, mid-turn — already raises.
        let r = w.form_slot_set(id, key, Value::Int(1));
        assert!(r.is_err());
        w.abort_turn();
        // after abort, the freeze is gone — mutation works again.
        w.start_turn();
        w.form_slot_set(id, key, Value::Int(1)).unwrap();
        let _ = w.commit_turn();
        assert_eq!(w.heap.get(id).slot(key), Value::Int(1));
    }

    #[test]
    fn frozen_form_in_turn_then_abort_can_mutate_after() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.freeze(id).unwrap();
        w.abort_turn();   // freeze rolled back
        // canonical was never frozen.
        assert!(!w.heap.get(id).frozen);
        w.start_turn();
        let key = w.intern("x");
        let r = w.form_slot_set(id, key, Value::Int(7));
        assert!(r.is_ok());
        let _ = w.commit_turn();
        assert_eq!(w.heap.get(id).slot(key), Value::Int(7));
    }
}
