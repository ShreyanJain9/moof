// The nursery: in-memory arena for all heap objects.
//
// Objects are allocated here and indexed by u32 ID.
// Value::nursery(id) references objects in this arena.
//
// Eventually, persistent objects will be promoted to LMDB (Value::object).
// For now, everything lives in the nursery.
//
// display/format code lives in heap/format.rs; save/load_image in
// heap/image.rs. both are additional `impl Heap` blocks in sibling files.

mod format;
mod image;
mod gc;
mod pair;
mod text;
mod bytes;
mod table;

pub use image::HeapImage;
pub use gc::GcStats;
pub use pair::Pair;
pub use text::Text;
pub use bytes::Bytes;
pub use table::Table;

use crate::object::HeapObject;
use crate::value::Value;
use crate::foreign::{ForeignData, ForeignType, ForeignTypeId, ForeignTypeRegistry};
use indexmap::IndexMap;
use std::sync::Arc;

// type prototype indices — named constants instead of magic numbers
pub const PROTO_NIL: usize = 0;
pub const PROTO_BOOL: usize = 1;
pub const PROTO_INT: usize = 2;
pub const PROTO_FLOAT: usize = 3;
pub const PROTO_SYM: usize = 4;
pub const PROTO_OBJ: usize = 5;
pub const PROTO_CONS: usize = 6;
pub const PROTO_STR: usize = 7;
pub const PROTO_BYTES: usize = 8;
pub const PROTO_TABLE: usize = 9;
pub const PROTO_NUMBER: usize = 10;
pub const PROTO_CLOSURE: usize = 11;
pub const PROTO_ERR: usize = 12;
pub const PROTO_FARREF: usize = 13;
pub const PROTO_ACT: usize = 14;
pub const PROTO_UPDATE: usize = 15;
pub const PROTO_OK: usize = 16;

/// What to run in a spawned vat.
#[derive(Debug)]
pub enum SpawnPayload {
    Source(String),              // source code to eval
    Closure(Value),              // closure to copy + call (no args)
    ClosureWithArgs(Value, Vec<Value>),  // closure + args to pass
}

/// A spawn request: queued by [Vat spawn: block/source], processed by scheduler.
#[derive(Debug)]
pub struct SpawnRequest {
    pub payload: SpawnPayload,
    pub act_id: u32,             // Act in this vat to resolve with the result
    pub serve: bool,             // true = return FarRef (server vat), false = copy result (compute vat)
}

/// An outgoing message from a vat (queued by FarRef's doesNotUnderstand:).
#[derive(Debug)]
pub struct OutgoingMessage {
    pub target_vat_id: u32,
    pub target_obj_id: u32,
    pub selector: u32,
    pub args: Vec<Value>,
    pub act_id: u32,  // local Act to resolve with the result
}

pub struct Heap {
    objects: Vec<HeapObject>,
    /// Indexes into `objects` that have been freed by GC and can
    /// be reused on alloc. Swept objects are tombstoned in place
    /// (replaced with an empty General) so the slot is safe to
    /// read — but callers shouldn't be holding refs to freed
    /// slots in the first place.
    free_list: Vec<u32>,
    /// Set by moof code (e.g. [Vat requestGc]) when a GC is
    /// desired. The scheduler / REPL loop polls this at safe
    /// points and runs the actual collection. never run GC
    /// directly from a native handler — VM frames are live.
    pub gc_requested: bool,
    /// Allocation counter since the last completed GC. when it
    /// crosses `alloc_budget`, alloc flips gc_requested so the
    /// next safepoint triggers a collection.
    allocs_since_gc: usize,
    /// Ratio-based GC budget: after each gc, set to live*2 (but
    /// no less than MIN_GC_BUDGET). this adapts to working set —
    /// steady-state programs get rare GCs, allocation-heavy ones
    /// get frequent ones.
    alloc_budget: usize,
    symbols: Vec<String>,
    sym_reverse: std::collections::HashMap<String, u32>,
    pub env: u32,                                      // root environment object ID
    pub rebound: std::collections::HashSet<u32>,       // symbols that have been reassigned
    pub vat_id: u32,                                   // which vat this heap belongs to
    pub outbox: Vec<OutgoingMessage>,                  // pending messages to other vats
    pub spawn_queue: Vec<SpawnRequest>,                // pending vat spawn requests
    pub ready_acts: Vec<u32>,                          // Acts with ready continuations

    // well-known symbols (interned at startup)
    pub sym_car: u32,
    pub sym_cdr: u32,
    pub sym_call: u32,
    pub sym_slot_at: u32,
    pub sym_slot_at_put: u32,
    pub sym_slot_names: u32,
    pub sym_handler_names: u32,
    pub sym_parent: u32,
    pub sym_describe: u32,
    pub sym_dnu: u32,  // doesNotUnderstand:
    pub sym_length: u32,
    pub sym_at: u32,
    pub sym_at_put: u32,
    pub sym_message: u32,     // message — Error slot

    // type prototypes: indexed by PROTO_* constants
    pub type_protos: Vec<Value>,

    // native handlers: name_sym → Rust closure
    pub natives: Vec<(u32, NativeFn)>,
    /// Parallel index: sym → position in `natives`. replaces the
    /// linear scan that `find_native` used to do — every native
    /// call previously walked the whole natives Vec.
    native_idx: std::collections::HashMap<u32, usize>,

    /// Send-site monomorphic cache. key: (starting_proto_id, selector).
    /// value: the handler value (native-sym or closure) resolved by
    /// chain walking from that prototype. every lookup_handler call
    /// checks this first — a hit skips the chain walk entirely.
    /// flushed on handler_set via moof-level [obj handle:with:].
    pub send_cache: std::collections::HashMap<(u32, u32), Value>,

    /// Foreign type registry — Ruby-style rust-value wrapping.
    /// Per-heap for vat isolation: cross-vat copies translate
    /// through the registered `ForeignTypeName`. Immutable by
    /// construction: `foreign_ref` but no `foreign_mut`.
    foreign_registry: ForeignTypeRegistry,
}

pub type NativeFn = Box<dyn Fn(&mut Heap, Value, &[Value]) -> Result<Value, String>>;

impl Heap {
    pub fn new() -> Self {
        let mut h = Heap {
            objects: Vec::new(),
            free_list: Vec::new(),
            gc_requested: false,
            allocs_since_gc: 0,
            alloc_budget: Self::MIN_GC_BUDGET,
            symbols: Vec::new(),
            sym_reverse: std::collections::HashMap::new(),
            env: 0,
            rebound: std::collections::HashSet::new(),
            vat_id: 0,
            outbox: Vec::new(),
            spawn_queue: Vec::new(),
            ready_acts: Vec::new(),
            sym_car: 0, sym_cdr: 0, sym_call: 0,
            sym_slot_at: 0, sym_slot_at_put: 0,
            sym_slot_names: 0, sym_handler_names: 0,
            sym_parent: 0, sym_describe: 0, sym_dnu: 0,
            sym_length: 0, sym_at: 0, sym_at_put: 0,
            sym_message: 0,
            type_protos: vec![Value::NIL; 17],
            natives: Vec::new(),
            native_idx: std::collections::HashMap::new(),
            send_cache: std::collections::HashMap::new(),
            foreign_registry: ForeignTypeRegistry::new(),
        };

        // intern well-known symbols
        h.sym_car = h.intern("car");
        h.sym_cdr = h.intern("cdr");
        h.sym_call = h.intern("call:");
        h.sym_slot_at = h.intern("slotAt:");
        h.sym_slot_at_put = h.intern("slotAt:put:");
        h.sym_slot_names = h.intern("slotNames");
        h.sym_handler_names = h.intern("handlerNames");
        h.sym_parent = h.intern("parent");
        h.sym_describe = h.intern("describe");
        h.sym_dnu = h.intern("doesNotUnderstand:");
        h.sym_length = h.intern("length");
        h.sym_at = h.intern("at:");
        h.sym_at_put = h.intern("at:put:");
        h.sym_message = h.intern("message");
        let bindings_sym = h.intern("bindings");

        // Register the core foreign types before anything else
        // allocates through the foreign pipeline. Pair is the
        // first — cons cells are foreign objects now. The sym IDs
        // for `car` / `cdr` were interned above and we propagate
        // them to the Pair vtable's slot cache.
        h.register_foreign_type::<Pair>().expect("register Pair");
        h.register_foreign_type::<Text>().expect("register Text");
        h.register_foreign_type::<Bytes>().expect("register Bytes");
        h.register_foreign_type::<Table>().expect("register Table");
        pair::PAIR_SYMS.store(pair::PairSyms { car: h.sym_car, cdr: h.sym_cdr });

        // allocate the root env's bindings Table first.
        let bindings_table = h.alloc_table(Vec::new(), IndexMap::new());

        // root env: just a General. `parent` here is a real slot —
        // the scope chain for variable lookup, which env_get walks.
        // Not the VM proto — that's the `proto` field (fixed up to
        // the Env prototype once the plugin registers it, or stays
        // whatever we set here as the default Object proto later).
        // `bindings` holds the actual name→value mappings.
        h.env = h.alloc(HeapObject::new_general(
            Value::NIL,                              // proto (set later)
            vec![h.sym_parent, bindings_sym],        // real user-facing slots
            vec![Value::NIL, bindings_table],        // parent scope (nil = root), bindings table
        ));

        h
    }

    // -- symbol table --

    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.sym_reverse.get(name) {
            return id;
        }
        let id = self.symbols.len() as u32;
        self.symbols.push(name.to_string());
        self.sym_reverse.insert(name.to_string(), id);
        id
    }

    pub fn symbol_name(&self, id: u32) -> &str {
        &self.symbols[id as usize]
    }

    /// Canonicalize a Table/HashMap key: intern String content as a symbol
    /// so two equal strings land in the same bucket. Other Values pass
    /// through (bit-hashing is correct for everything else).
    pub fn canonicalize_key(&mut self, key: Value) -> Value {
        if let Some(s) = key.as_any_object().and_then(|id| self.get_string(id)) {
            let s = s.to_string();
            return Value::symbol(self.intern(&s));
        }
        key
    }

    /// Look up a symbol ID by name without interning. Returns None if not found.
    pub fn find_symbol(&self, name: &str) -> Option<u32> {
        self.sym_reverse.get(name).copied()
    }

    // -- environment access --
    //
    // An env is a General with two user-facing slots:
    //   `parent`   — the outer scope (another env, or nil at the root).
    //                Scope lookup climbs this when a name isn't here.
    //   `bindings` — a Table (IndexMap-backed) mapping symbol → value
    //                for locals defined in this scope.
    //
    // `parent` is a real slot you can read from moof as `env.parent`. It
    // is NOT the VM-internal proto (which governs message dispatch); scope
    // chain and prototype chain are different concepts with different
    // walk paths.

    fn env_bindings_id(&self, env_id: u32) -> Option<u32> {
        let bindings_sym = self.sym_reverse.get("bindings").copied()?;
        self.get(env_id).slot_get(bindings_sym)?.as_any_object()
    }

    fn env_parent(&self, env_id: u32) -> Value {
        self.get(env_id).slot_get(self.sym_parent).unwrap_or(Value::NIL)
    }

    pub fn env_get(&self, sym: u32) -> Option<Value> {
        // walk the scope chain via the `parent` slot — NOT the proto chain.
        let mut cur = self.env;
        loop {
            if let Some(bid) = self.env_bindings_id(cur) {
                if let Some(t) = self.foreign_ref::<Table>(Value::nursery(bid)) {
                    if let Some(v) = t.map.get(&Value::symbol(sym)).copied() {
                        return Some(v);
                    }
                }
            }
            let Some(next) = self.env_parent(cur).as_any_object() else { return None; };
            cur = next;
        }
    }

    /// Look up a type by name from the environment.
    pub fn lookup_type(&self, name: &str) -> Value {
        self.find_symbol(name)
            .and_then(|sym| self.env_get(sym))
            .unwrap_or(Value::NIL)
    }

    /// Get a mutable borrow of the foreign payload at `id`, if it's
    /// the sole owner of the Arc. This is a *crate-internal* escape
    /// hatch used only for env bindings (a Table that legitimately
    /// grows as bindings accumulate). User-facing moof code cannot
    /// reach this — ForeignType stays immutable from moof's side.
    ///
    /// Falls back to clone-and-replace when the Arc is shared
    /// (unusual for env bindings but handled safely).
    pub(crate) fn foreign_payload_mut<T>(&mut self, val: Value) -> Option<&mut T>
    where T: ForeignType + 'static
    {
        let id = val.as_any_object()?;
        let obj = self.get_mut(id);
        let fd = match obj {
            HeapObject::General { foreign: Some(fd), .. } => fd,
            _ => return None,
        };
        // If shared, clone out and replace so we get unique ownership.
        if std::sync::Arc::strong_count(&fd.payload) > 1 {
            let cloned: T = fd.payload.downcast_ref::<T>()?.clone();
            fd.payload = std::sync::Arc::new(cloned);
        }
        let arc = &mut fd.payload;
        std::sync::Arc::get_mut(arc)?.downcast_mut::<T>()
    }

    pub fn env_def(&mut self, sym: u32, val: Value) {
        // define in the root env's bindings.
        if let Some(bid) = self.env_bindings_id(self.env) {
            if let Some(t) = self.foreign_payload_mut::<Table>(Value::nursery(bid)) {
                t.map.insert(Value::symbol(sym), val);
            }
        }
    }

    pub fn env_remove(&mut self, sym: u32) {
        if let Some(bid) = self.env_bindings_id(self.env) {
            if let Some(t) = self.foreign_payload_mut::<Table>(Value::nursery(bid)) {
                t.map.shift_remove(&Value::symbol(sym));
            }
        }
    }

    // -- object allocation --

    /// Floor for the GC budget — keeps GCs from firing during the
    /// first few allocations at program start.
    pub const MIN_GC_BUDGET: usize = 2048;
    /// Growth factor after a GC: next_budget = live_count * this.
    /// 2x means ~50% collector-amortized overhead in the worst
    /// case; tighten to 1.5 for more-frequent-smaller collections.
    pub const GC_GROWTH_FACTOR: usize = 2;

    pub fn alloc(&mut self, obj: HeapObject) -> u32 {
        self.allocs_since_gc += 1;
        if self.allocs_since_gc >= self.alloc_budget {
            self.gc_requested = true;
        }
        // prefer freelist (reuse) over append (grow)
        if let Some(id) = self.free_list.pop() {
            self.objects[id as usize] = obj;
            id
        } else {
            let id = self.objects.len() as u32;
            self.objects.push(obj);
            id
        }
    }

    /// Called by gc() after a collection completes to reset the
    /// budget based on the new live count.
    pub(crate) fn set_alloc_budget_from_live(&mut self, live: usize) {
        self.alloc_budget = (live * Self::GC_GROWTH_FACTOR).max(Self::MIN_GC_BUDGET);
        self.allocs_since_gc = 0;
    }

    pub fn alloc_val(&mut self, obj: HeapObject) -> Value {
        Value::nursery(self.alloc(obj))
    }

    pub fn get(&self, id: u32) -> &HeapObject {
        &self.objects[id as usize]
    }

    pub fn get_mut(&mut self, id: u32) -> &mut HeapObject {
        &mut self.objects[id as usize]
    }

    // -- convenience allocators --

    pub fn make_object(&mut self, proto: Value) -> Value {
        self.alloc_val(HeapObject::new_empty(proto))
    }

    pub fn make_object_with_slots(&mut self, proto: Value, slot_names: Vec<u32>, slot_values: Vec<Value>) -> Value {
        self.alloc_val(HeapObject::new_general(proto, slot_names, slot_values))
    }

    /// Create an Err value with a message string.
    pub fn make_error(&mut self, msg: &str) -> Value {
        let msg_val = self.alloc_string(msg);
        let parent = self.lookup_type("Err");
        let parent = if parent.is_nil() { self.type_protos[PROTO_OBJ] } else { parent };
        self.make_object_with_slots(parent, vec![self.sym_message], vec![msg_val])
    }

    /// Build an Ok(val) result. Uses the Ok prototype from the effects plugin.
    pub fn make_ok(&mut self, val: Value) -> Value {
        let parent = self.lookup_type("Ok");
        let parent = if parent.is_nil() { self.type_protos[PROTO_OBJ] } else { parent };
        let value_sym = self.intern("value");
        self.make_object_with_slots(parent, vec![value_sym], vec![val])
    }

    pub fn cons(&mut self, car: Value, cdr: Value) -> Value {
        // Pairs are foreign objects: proto is Cons (user-visible
        // prototype that carries list-protocol handlers), payload
        // is `Pair { car, cdr }`. car/cdr appear as virtual slots.
        let proto = self.type_protos.get(PROTO_CONS).copied().unwrap_or(Value::NIL);
        self.alloc_foreign(proto, Pair { car, cdr })
            .expect("Pair foreign type must be registered")
    }

    /// If `id` is a cons-pair General, return (car, cdr). This is
    /// the replacement for the old `HeapObject::Pair(a, b)` match —
    /// post-wave-5.1 pairs don't have a dedicated enum variant.
    pub fn pair_of(&self, id: u32) -> Option<(Value, Value)> {
        self.foreign_ref::<Pair>(Value::nursery(id)).map(|p| (p.car, p.cdr))
    }

    /// True iff `val` is an object whose foreign payload is a Pair.
    pub fn is_pair(&self, val: Value) -> bool {
        self.foreign_ref::<Pair>(val).is_some()
    }

    pub fn alloc_string(&mut self, s: &str) -> Value {
        let proto = self.type_protos.get(PROTO_STR).copied().unwrap_or(Value::NIL);
        self.alloc_foreign(proto, Text(s.to_string()))
            .expect("Text foreign type must be registered")
    }

    pub fn alloc_bytes(&mut self, data: Vec<u8>) -> Value {
        let proto = self.type_protos.get(PROTO_BYTES).copied().unwrap_or(Value::NIL);
        self.alloc_foreign(proto, Bytes(data))
            .expect("Bytes foreign type must be registered")
    }

    pub fn alloc_table_seq(&mut self, items: Vec<Value>) -> Value {
        let proto = self.type_protos.get(PROTO_TABLE).copied().unwrap_or(Value::NIL);
        self.alloc_foreign(proto, Table { seq: items, map: indexmap::IndexMap::new() })
            .expect("Table foreign type must be registered")
    }

    pub fn alloc_table(&mut self, seq: Vec<Value>, map: indexmap::IndexMap<Value, Value>) -> Value {
        let proto = self.type_protos.get(PROTO_TABLE).copied().unwrap_or(Value::NIL);
        self.alloc_foreign(proto, Table { seq, map })
            .expect("Table foreign type must be registered")
    }

    /// Create an Act for a cross-vat send (pending, with target info).
    pub fn make_act(&mut self, target_vat: u32, target_obj: u32, selector: u32) -> Value {
        let act_proto = self.lookup_type("Act");
        let state_sym = self.intern("__state");
        let pending_sym = self.intern("pending");
        let chain_sym = self.intern("__chain");
        let tgt_vat_sym = self.intern("__target_vat");
        let tgt_obj_sym = self.intern("__target_obj");
        let sel_sym = self.intern("__selector");
        let result_sym = self.intern("__result");
        self.make_object_with_slots(
            act_proto,
            vec![state_sym, chain_sym, tgt_vat_sym, tgt_obj_sym, sel_sym, result_sym],
            vec![Value::symbol(pending_sym), Value::NIL,
                 Value::integer(target_vat as i64), Value::integer(target_obj as i64),
                 Value::symbol(selector), Value::NIL],
        )
    }

    /// Create a pending Act with no target (for continuation-derived Acts).
    pub fn make_pending_act(&mut self) -> Value {
        let act_proto = self.lookup_type("Act");
        let state_sym = self.intern("__state");
        let pending_sym = self.intern("pending");
        let chain_sym = self.intern("__chain");
        let result_sym = self.intern("__result");
        let cont_fn_sym = self.intern("__cont_fn");
        let cont_val_sym = self.intern("__cont_val");
        self.make_object_with_slots(
            act_proto,
            vec![state_sym, chain_sym, result_sym, cont_fn_sym, cont_val_sym],
            vec![Value::symbol(pending_sym), Value::NIL, Value::NIL, Value::NIL, Value::NIL],
        )
    }

    // -- object access helpers --

    pub fn car(&self, id: u32) -> Value {
        self.pair_of(id).map(|(a, _)| a).unwrap_or(Value::NIL)
    }

    pub fn cdr(&self, id: u32) -> Value {
        self.pair_of(id).map(|(_, d)| d).unwrap_or(Value::NIL)
    }

    pub fn get_string(&self, id: u32) -> Option<&str> {
        self.foreign_ref::<Text>(Value::nursery(id)).map(|t| t.0.as_str())
    }

    pub fn get_bytes(&self, id: u32) -> Option<&[u8]> {
        self.foreign_ref::<Bytes>(Value::nursery(id)).map(|b| b.0.as_slice())
    }

    pub fn get_table(&self, id: u32) -> Option<&Table> {
        self.foreign_ref::<Table>(Value::nursery(id))
    }

    pub fn is_text(&self, val: Value) -> bool { self.foreign_ref::<Text>(val).is_some() }
    pub fn is_bytes(&self, val: Value) -> bool { self.foreign_ref::<Bytes>(val).is_some() }
    pub fn is_table(&self, val: Value) -> bool { self.foreign_ref::<Table>(val).is_some() }

    /// Build a moof list from a slice of values: (a b c) as nested cons cells.
    pub fn list(&mut self, items: &[Value]) -> Value {
        let mut result = Value::NIL;
        for item in items.iter().rev() {
            result = self.cons(*item, result);
        }
        result
    }

    /// Collect a cons list into a Vec.
    pub fn list_to_vec(&self, mut list: Value) -> Vec<Value> {
        let mut result = Vec::new();
        while let Some(id) = list.as_any_object() {
            match self.pair_of(id) {
                Some((car, cdr)) => {
                    result.push(car);
                    list = cdr;
                }
                None => break,
            }
        }
        result
    }

    // -- native handler registration --

    pub fn register_native(&mut self, name: &str, f: impl Fn(&mut Heap, Value, &[Value]) -> Result<Value, String> + 'static) -> Value {
        let idx = self.natives.len();
        let unique = format!("{name}#{idx}");
        let sym = self.intern(&unique);
        self.natives.push((sym, Box::new(f)));
        self.native_idx.insert(sym, idx);
        Value::symbol(sym) // the handler value IS the symbol — dispatch looks it up
    }

    pub fn find_native(&self, sym: u32) -> Option<usize> {
        // O(1) hashmap lookup. previously this was a linear scan over
        // the natives Vec — a real cost when every native call paid
        // for it. the map is maintained in lockstep with the Vec by
        // register_native.
        self.native_idx.get(&sym).copied()
    }

    /// Value equality (like Ruby's eql?). Compares content for strings.
    pub fn values_equal(&self, a: Value, b: Value) -> bool {
        if a == b { return true; } // identity match (covers ints, symbols, bools, nil, same obj)
        if let (Some(sa), Some(sb)) = (
            a.as_any_object().and_then(|id| self.get_string(id)),
            b.as_any_object().and_then(|id| self.get_string(id)),
        ) {
            return sa == sb;
        }
        false
    }

    /// Create a closure object: a General with PROTO_CLOSURE proto plus
    /// metadata slots (code_idx / arity / is_operative / is_pure) and
    /// captures as regular named slots. Metadata occupies slot indices
    /// 0..4; captures follow starting at CLOSURE_FIXED_SLOTS. A `call:`
    /// handler pointing to self is installed so dispatch finds it.
    pub fn make_closure(&mut self, code_idx: usize, arity: u8, is_operative: bool, captures: &[(u32, Value)]) -> Value {
        let proto = self.type_protos.get(PROTO_CLOSURE).copied()
            .unwrap_or_else(|| self.type_protos[PROTO_OBJ]);

        let farref_proto = self.lookup_type("FarRef");
        let is_pure = if farref_proto.is_nil() {
            true
        } else {
            !captures.iter().any(|(_, val)| self.prototype_of(*val) == farref_proto)
        };

        let code_idx_sym = self.intern("code_idx");
        let arity_sym = self.intern("arity");
        let is_op_sym = self.intern("is_operative");
        let is_pure_sym = self.intern("is_pure");

        let mut names: Vec<u32> = Vec::with_capacity(4 + captures.len());
        let mut values: Vec<Value> = Vec::with_capacity(4 + captures.len());
        names.push(code_idx_sym);  values.push(Value::integer(code_idx as i64));
        names.push(arity_sym);     values.push(Value::integer(arity as i64));
        names.push(is_op_sym);     values.push(Value::boolean(is_operative));
        names.push(is_pure_sym);   values.push(Value::boolean(is_pure));
        for (sym, val) in captures {
            names.push(*sym);
            values.push(*val);
        }

        let id = self.alloc(HeapObject::new_general(proto, names, values));
        let val = Value::nursery(id);
        let call_sym = self.sym_call;
        self.get_mut(id).handler_set(call_sym, val);
        val
    }

    /// Check if a value is a closure object. Returns (code_idx, is_operative)
    /// if so. Detection is proto-based: closures delegate to PROTO_CLOSURE.
    /// Slot access uses fixed INDEXES (not names) to stay robust against
    /// cross-vat symbol-id mismatch during migration.
    pub fn as_closure(&self, val: Value) -> Option<(usize, bool)> {
        let id = val.as_any_object()?;
        let HeapObject::General { proto, slot_values, slot_names, .. } = self.get(id) else {
            return None;
        };
        let closure_proto = self.type_protos.get(PROTO_CLOSURE).copied().unwrap_or(Value::NIL);
        if slot_names.len() < Self::CLOSURE_FIXED_SLOTS || *proto != closure_proto {
            return None;
        }
        let code_idx = slot_values[Self::SLOT_CODE_IDX].as_integer()? as usize;
        let is_op = slot_values[Self::SLOT_IS_OPERATIVE].is_true();
        Some((code_idx, is_op))
    }

    /// Fixed slot positions on every closure. Metadata occupies 0..4;
    /// captures start at CLOSURE_FIXED_SLOTS.
    const SLOT_CODE_IDX: usize = 0;
    const SLOT_ARITY: usize = 1;
    const SLOT_IS_OPERATIVE: usize = 2;
    const SLOT_IS_PURE: usize = 3;
    const CLOSURE_FIXED_SLOTS: usize = 4;

    pub fn closure_captures(&self, val: Value) -> Vec<(u32, Value)> {
        let Some(id) = val.as_any_object() else { return Vec::new(); };
        let HeapObject::General { slot_names, slot_values, .. } = self.get(id) else {
            return Vec::new();
        };
        if slot_names.len() <= Self::CLOSURE_FIXED_SLOTS {
            return Vec::new();
        }
        slot_names.iter().skip(Self::CLOSURE_FIXED_SLOTS)
            .zip(slot_values.iter().skip(Self::CLOSURE_FIXED_SLOTS))
            .map(|(s, v)| (*s, *v))
            .collect()
    }

    /// Check if a closure is "pure" (no FarRef captures, safe to memoize).
    pub fn closure_is_pure(&self, val: Value) -> bool {
        let Some(id) = val.as_any_object() else { return false; };
        let HeapObject::General { slot_values, slot_names, .. } = self.get(id) else {
            return false;
        };
        if slot_names.len() < Self::CLOSURE_FIXED_SLOTS { return false; }
        slot_values[Self::SLOT_IS_PURE].is_true()
    }

    /// Return a closure's arity, if it is one.
    pub fn closure_arity(&self, val: Value) -> Option<u8> {
        let id = val.as_any_object()?;
        let HeapObject::General { slot_values, slot_names, .. } = self.get(id) else {
            return None;
        };
        if slot_names.len() < Self::CLOSURE_FIXED_SLOTS { return None; }
        Some(slot_values[Self::SLOT_ARITY].as_integer()? as u8)
    }

    /// Override the is_pure metadata flag on a closure.
    pub fn set_closure_pure(&mut self, val: Value, is_pure: bool) {
        let Some(id) = val.as_any_object() else { return; };
        if let HeapObject::General { slot_values, slot_names, .. } = self.get_mut(id) {
            if slot_names.len() >= Self::CLOSURE_FIXED_SLOTS {
                slot_values[Self::SLOT_IS_PURE] = Value::boolean(is_pure);
            }
        }
    }

    /// Get the prototype for any value (including primitives and optimized types).
    pub fn prototype_of(&self, val: Value) -> Value {
        if let Some(id) = val.as_any_object() {
            if let HeapObject::General { proto, .. } = self.get(id) {
                return *proto;
            }
        }
        let tag = val.type_tag() as usize;
        self.type_protos.get(tag).copied().unwrap_or(Value::NIL)
    }

    /// Get all handler names for any value (walks the full delegation chain).
    pub fn all_handler_names(&self, val: Value) -> Vec<u32> {
        let mut names = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // start with the receiver's own handlers — General and Environment
        // both carry per-instance handler tables (closures are Generals).
        if let Some(id) = val.as_any_object() {
            if let HeapObject::General { handlers, .. } = self.get(id) {
                for &(sel, _) in handlers {
                    if seen.insert(sel) { names.push(sel); }
                }
            }
        }

        // walk the prototype chain — accept any variant that carries handlers.
        let mut proto = self.prototype_of(val);
        for _ in 0..256 {
            if proto.is_nil() { break; }
            let Some(id) = proto.as_any_object() else { break; };
            let (handlers, next) = match self.get(id) {
                HeapObject::General { handlers, proto, .. } => (handlers, *proto),
                _ => break,
            };
            for &(sel, _) in handlers {
                if seen.insert(sel) { names.push(sel); }
            }
            proto = next;
        }
        names
    }

    /// Extract params from a possibly-dotted list.
    /// Returns (positional_params, optional_rest_param).
    /// (a b c) → ([a, b, c], None)
    /// (a b . rest) → ([a, b], Some(rest))
    /// just-a-symbol → ([], Some(symbol))
    pub fn extract_params(&self, form: Value) -> (Vec<u32>, Option<u32>) {
        // single symbol = all-capturing rest param
        if let Some(sym) = form.as_symbol() {
            return (Vec::new(), Some(sym));
        }
        let mut positional = Vec::new();
        let mut current = form;
        loop {
            if current.is_nil() { break; }
            if let Some(sym) = current.as_symbol() {
                // dotted tail — rest param
                return (positional, Some(sym));
            }
            if let Some(id) = current.as_any_object() {
                match self.pair_of(id) {
                    Some((car, cdr)) => {
                        if let Some(sym) = car.as_symbol() {
                            positional.push(sym);
                        }
                        current = cdr;
                    }
                    None => break,
                }
            } else {
                break;
            }
        }
        (positional, None)
    }

    /// Total object count (includes freelist slots).
    pub fn object_count(&self) -> usize { self.objects.len() }

    /// Live object count (heap size minus freelist).
    pub fn live_count(&self) -> usize {
        self.objects.len() - self.free_list.len()
    }

    pub fn objects_ref(&self) -> &[HeapObject] { &self.objects }
    pub fn symbols_ref(&self) -> &[String] { &self.symbols }

    // ============================================================
    // Foreign type registry — Ruby-style rust value wrapping.
    // ============================================================

    pub fn foreign_registry(&self) -> &ForeignTypeRegistry { &self.foreign_registry }

    /// User-facing slot read: consults both the real slots vec AND
    /// the foreign payload's virtual-slot hook (if any). Pair's
    /// car/cdr, Vec3's x/y/z, etc. flow through this.
    pub fn slot_of(&self, id: u32, name: u32) -> Option<Value> {
        let obj = self.get(id);
        if let Some(v) = obj.slot_get(name) { return Some(v); }
        if let Some(fd) = obj.foreign() {
            if let Some(vt) = self.foreign_registry.vtable(fd.type_id) {
                if let Some(vfn) = vt.virtual_slot {
                    return vfn(&*fd.payload, name);
                }
            }
        }
        None
    }

    /// All slot names on this object — real slots + any virtual
    /// slots contributed by its foreign payload.
    pub fn slot_names_of(&self, id: u32) -> Vec<u32> {
        let obj = self.get(id);
        let mut names = obj.slot_names();
        if let Some(fd) = obj.foreign() {
            if let Some(vt) = self.foreign_registry.vtable(fd.type_id) {
                if let Some(vfn) = vt.virtual_slot_names {
                    names.extend(vfn(&*fd.payload));
                }
            }
        }
        names
    }

    /// Register a `ForeignType` impl. Returns the session-local
    /// type id. Idempotent — re-registering the same type name
    /// with a matching schema hash is a no-op; mismatch errors.
    pub fn register_foreign_type<T: ForeignType>(&mut self) -> Result<ForeignTypeId, String> {
        self.foreign_registry.register::<T>()
    }

    /// Allocate a General with a foreign payload attached. Proto
    /// determines message dispatch; the payload is immutable.
    pub fn alloc_foreign<T: ForeignType>(&mut self, proto: Value, payload: T) -> Result<Value, String> {
        let type_id = self.foreign_registry.lookup(T::type_name())
            .ok_or_else(|| format!("foreign type '{}' not registered", T::type_name()))?;
        let fd = ForeignData {
            type_id,
            payload: Arc::new(payload),
        };
        Ok(self.alloc_val(HeapObject::new_foreign(proto, fd)))
    }

    /// Borrow the foreign payload of `val` as `&T`, if any. Returns
    /// None if `val` isn't an object, has no foreign payload, or
    /// holds a different type. This is the ONLY access path —
    /// there is no `foreign_mut`.
    pub fn foreign_ref<T: ForeignType>(&self, val: Value) -> Option<&T> {
        let id = val.as_any_object()?;
        let fd = self.get(id).foreign()?;
        fd.payload.downcast_ref::<T>()
    }

    /// Clone the foreign payload out if it matches `T`. Convenient
    /// when a handler needs owned data to pair with `&mut Heap`.
    pub fn foreign_clone<T: ForeignType>(&self, val: Value) -> Option<T> {
        self.foreign_ref::<T>(val).cloned()
    }

    // ============================================================
    // Image restore — used by load_image path to rehydrate state
    // after construction. new() already ran, so the registry and
    // root env exist; this replaces the object arena + symbol
    // table + env pointer wholesale.
    // ============================================================

    pub fn restore_objects(&mut self, objects: Vec<HeapObject>, symbols: Vec<String>, env_id: u32) {
        self.objects = objects;
        self.symbols = symbols;
        self.env = env_id;
        self.sym_reverse.clear();
        for (i, name) in self.symbols.iter().enumerate() {
            self.sym_reverse.insert(name.clone(), i as u32);
        }
        // re-resolve well-known symbols
        self.sym_car = self.sym_reverse.get("car").copied().unwrap_or(0);
        self.sym_cdr = self.sym_reverse.get("cdr").copied().unwrap_or(0);
        self.sym_call = self.sym_reverse.get("call:").copied().unwrap_or(0);
        self.sym_slot_at = self.sym_reverse.get("slotAt:").copied().unwrap_or(0);
        self.sym_slot_at_put = self.sym_reverse.get("slotAt:put:").copied().unwrap_or(0);
        self.sym_slot_names = self.sym_reverse.get("slotNames").copied().unwrap_or(0);
        self.sym_handler_names = self.sym_reverse.get("handlerNames").copied().unwrap_or(0);
        self.sym_parent = self.sym_reverse.get("parent").copied().unwrap_or(0);
        self.sym_describe = self.sym_reverse.get("describe").copied().unwrap_or(0);
        self.sym_dnu = self.sym_reverse.get("doesNotUnderstand:").copied().unwrap_or(0);
        self.sym_length = self.sym_reverse.get("length").copied().unwrap_or(0);
        self.sym_at = self.sym_reverse.get("at:").copied().unwrap_or(0);
        self.sym_at_put = self.sym_reverse.get("at:put:").copied().unwrap_or(0);
        self.sym_message = self.sym_reverse.get("message").copied().unwrap_or_else(|| self.intern("message"));
    }

    /// Restore a heap from saved data.
    pub fn restore(
        objects: Vec<HeapObject>,
        symbols: Vec<String>,
        env_id: u32,
    ) -> Self {
        let mut h = Heap::new();
        h.objects = objects;
        h.symbols = symbols;
        h.env = env_id;
        h.sym_reverse.clear();
        for (i, name) in h.symbols.iter().enumerate() {
            h.sym_reverse.insert(name.clone(), i as u32);
        }
        // re-resolve well-known symbols
        h.sym_car = h.sym_reverse.get("car").copied().unwrap_or(0);
        h.sym_cdr = h.sym_reverse.get("cdr").copied().unwrap_or(0);
        h.sym_call = h.sym_reverse.get("call:").copied().unwrap_or(0);
        h.sym_slot_at = h.sym_reverse.get("slotAt:").copied().unwrap_or(0);
        h.sym_slot_at_put = h.sym_reverse.get("slotAt:put:").copied().unwrap_or(0);
        h.sym_slot_names = h.sym_reverse.get("slotNames").copied().unwrap_or(0);
        h.sym_handler_names = h.sym_reverse.get("handlerNames").copied().unwrap_or(0);
        h.sym_parent = h.sym_reverse.get("parent").copied().unwrap_or(0);
        h.sym_describe = h.sym_reverse.get("describe").copied().unwrap_or(0);
        h.sym_dnu = h.sym_reverse.get("doesNotUnderstand:").copied().unwrap_or(0);
        h.sym_length = h.sym_reverse.get("length").copied().unwrap_or(0);
        h.sym_at = h.sym_reverse.get("at:").copied().unwrap_or(0);
        h.sym_at_put = h.sym_reverse.get("at:put:").copied().unwrap_or(0);
        h.sym_message = h.sym_reverse.get("message").copied().unwrap_or_else(|| h.intern("message"));
        h
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_and_lookup() {
        let mut h = Heap::new();
        let a = h.intern("hello");
        let b = h.intern("world");
        let c = h.intern("hello");
        assert_eq!(a, c);
        assert_ne!(a, b);
        assert_eq!(h.symbol_name(a), "hello");
    }

    #[test]
    fn alloc_and_get() {
        let mut h = Heap::new();
        let obj = h.make_object(Value::NIL);
        assert!(obj.is_nursery());
        let id = obj.as_nursery().unwrap();
        assert!(matches!(h.get(id), HeapObject::General { .. }));
    }

    #[test]
    fn cons_and_list() {
        let mut h = Heap::new();
        let list = h.list(&[Value::integer(1), Value::integer(2), Value::integer(3)]);
        let vec = h.list_to_vec(list);
        assert_eq!(vec.len(), 3);
        assert_eq!(vec[0].as_integer(), Some(1));
        assert_eq!(vec[2].as_integer(), Some(3));
    }

    #[test]
    fn foreign_type_roundtrip() {
        use crate::foreign::ForeignType;

        #[derive(Clone, PartialEq, Debug)]
        struct Point2D { x: i64, y: i64 }

        impl ForeignType for Point2D {
            fn type_name() -> &'static str { "test.Point2D" }
            fn serialize(&self) -> Vec<u8> {
                let mut b = Vec::with_capacity(16);
                b.extend_from_slice(&self.x.to_le_bytes());
                b.extend_from_slice(&self.y.to_le_bytes());
                b
            }
            fn deserialize(bytes: &[u8]) -> Result<Self, String> {
                let mut x_bytes = [0u8; 8]; x_bytes.copy_from_slice(&bytes[0..8]);
                let mut y_bytes = [0u8; 8]; y_bytes.copy_from_slice(&bytes[8..16]);
                Ok(Point2D { x: i64::from_le_bytes(x_bytes), y: i64::from_le_bytes(y_bytes) })
            }
            fn equal(&self, other: &Self) -> bool { self == other }
            fn describe(&self) -> String { format!("Point2D({},{})", self.x, self.y) }
        }

        let mut h = Heap::new();
        h.register_foreign_type::<Point2D>().unwrap();

        let proto = h.type_protos[PROTO_OBJ];
        let v = h.alloc_foreign(proto, Point2D { x: 3, y: 4 }).unwrap();

        // borrow out as &Point2D
        let borrowed = h.foreign_ref::<Point2D>(v).unwrap();
        assert_eq!(borrowed, &Point2D { x: 3, y: 4 });

        // serialize one object and deserialize it back
        let obj = h.get(v.as_any_object().unwrap()).clone();
        let bytes = h.serialize_object(&obj).unwrap();
        let back = h.deserialize_object(&bytes).unwrap();

        // reconstructed object's foreign payload should roundtrip.
        let fd = back.foreign().unwrap();
        let p = fd.payload.downcast_ref::<Point2D>().unwrap();
        assert_eq!(p, &Point2D { x: 3, y: 4 });
    }

    #[test]
    fn open_slots() {
        let mut h = Heap::new();
        let x = h.intern("x");
        let y = h.intern("y");
        let obj = h.make_object_with_slots(Value::NIL, vec![x, y], vec![Value::integer(3), Value::integer(4)]);
        let id = obj.as_any_object().unwrap();

        assert_eq!(h.get(id).slot_get(x), Some(Value::integer(3)));
        assert_eq!(h.get(id).slot_get(y), Some(Value::integer(4)));

        // can overwrite existing slots
        assert!(h.get_mut(id).slot_set(x, Value::integer(99)));
        assert_eq!(h.get(id).slot_get(x), Some(Value::integer(99)));

        // slots are open — adding a new one grows the object
        let z = h.intern("z");
        assert!(h.get_mut(id).slot_set(z, Value::integer(0)));
        assert_eq!(h.get(id).slot_get(z), Some(Value::integer(0)));
    }

    #[test]
    fn open_handlers() {
        let mut h = Heap::new();
        let obj = h.make_object(Value::NIL);
        let id = obj.as_any_object().unwrap();
        let sel = h.intern("foo");

        assert!(h.get(id).handler_get(sel).is_none());

        h.get_mut(id).handler_set(sel, Value::integer(42));
        assert_eq!(h.get(id).handler_get(sel), Some(Value::integer(42)));

        // overwrite
        h.get_mut(id).handler_set(sel, Value::integer(99));
        assert_eq!(h.get(id).handler_get(sel), Some(Value::integer(99)));
    }

    #[test]
    fn format_values() {
        let mut h = Heap::new();
        assert_eq!(h.format_value(Value::NIL), "nil");
        assert_eq!(h.format_value(Value::integer(42)), "42");

        let s = h.alloc_string("hello");
        assert_eq!(h.format_value(s), "\"hello\"");

        let list = h.list(&[Value::integer(1), Value::integer(2)]);
        assert_eq!(h.format_value(list), "(1 2)");
    }
}
