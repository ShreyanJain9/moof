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

pub use image::HeapImage;
pub use gc::GcStats;

use crate::object::HeapObject;
use crate::value::Value;

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
        };

        // allocate root environment object (gets ID 0)
        h.env = h.alloc(HeapObject::Environment {
            parent: Value::NIL, // fixed up in register_type_protos
            bindings: std::collections::HashMap::new(),
            handlers: Vec::new(),
        });

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

    /// Look up a symbol ID by name without interning. Returns None if not found.
    pub fn find_symbol(&self, name: &str) -> Option<u32> {
        self.sym_reverse.get(name).copied()
    }

    // -- environment access --

    pub fn env_get(&self, sym: u32) -> Option<Value> {
        self.get(self.env).slot_get(sym)
    }

    /// Look up a type by name from the environment. Used by Rust code
    /// to find moof-defined types (Ok, Err, Act, etc.) without needing
    /// hardcoded PROTO_* constants.
    pub fn lookup_type(&self, name: &str) -> Value {
        self.find_symbol(name)
            .and_then(|sym| self.env_get(sym))
            .unwrap_or(Value::NIL)
    }

    pub fn env_def(&mut self, sym: u32, val: Value) {
        self.get_mut(self.env).slot_set(sym, val);
    }

    pub fn env_remove(&mut self, sym: u32) {
        if let HeapObject::Environment { bindings, .. } = self.get_mut(self.env) {
            bindings.remove(&sym);
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

    pub fn make_object(&mut self, parent: Value) -> Value {
        self.alloc_val(HeapObject::new_empty(parent))
    }

    pub fn make_object_with_slots(&mut self, parent: Value, slot_names: Vec<u32>, slot_values: Vec<Value>) -> Value {
        self.alloc_val(HeapObject::new_general(parent, slot_names, slot_values))
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
        self.alloc_val(HeapObject::Pair(car, cdr))
    }

    pub fn alloc_string(&mut self, s: &str) -> Value {
        self.alloc_val(HeapObject::Text(s.to_string()))
    }

    pub fn alloc_bytes(&mut self, data: Vec<u8>) -> Value {
        self.alloc_val(HeapObject::Buffer(data))
    }

    pub fn alloc_table_seq(&mut self, items: Vec<Value>) -> Value {
        self.alloc_val(HeapObject::Table { seq: items, map: Vec::new() })
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
        match self.get(id) {
            HeapObject::Pair(car, _) => *car,
            _ => Value::NIL,
        }
    }

    pub fn cdr(&self, id: u32) -> Value {
        match self.get(id) {
            HeapObject::Pair(_, cdr) => *cdr,
            _ => Value::NIL,
        }
    }

    pub fn get_string(&self, id: u32) -> Option<&str> {
        match self.get(id) {
            HeapObject::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn get_bytes(&self, id: u32) -> Option<&[u8]> {
        match self.get(id) {
            HeapObject::Buffer(b) => Some(b),
            _ => None,
        }
    }

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
            match self.get(id) {
                HeapObject::Pair(car, cdr) => {
                    result.push(*car);
                    list = *cdr;
                }
                _ => break,
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
        // content equality for strings
        if let (Some(aid), Some(bid)) = (a.as_any_object(), b.as_any_object()) {
            match (self.get(aid), self.get(bid)) {
                (HeapObject::Text(sa), HeapObject::Text(sb)) => return sa == sb,
                _ => {}
            }
        }
        false
    }

    /// Create a closure object: a General with PROTO_CLOSURE parent and a
    /// standard set of metadata slots (__code_idx, __arity, __is_operative,
    /// __is_pure) followed by the captures as regular named slots. A `call:`
    /// handler pointing to self is installed so dispatch finds it; the VM's
    /// is-closure fast path reads the __code_idx slot to invoke bytecode.
    pub fn make_closure(&mut self, code_idx: usize, arity: u8, is_operative: bool, captures: &[(u32, Value)]) -> Value {
        let parent = self.type_protos.get(PROTO_CLOSURE).copied()
            .unwrap_or_else(|| self.type_protos[PROTO_OBJ]);

        // compute is_pure before borrowing self mutably for intern.
        let farref_proto = self.lookup_type("FarRef");
        let is_pure = if farref_proto.is_nil() {
            true
        } else {
            !captures.iter().any(|(_, val)| self.prototype_of(*val) == farref_proto)
        };

        // metadata slots first, then captures. slot names are NORMAL (no __
        // prefix) — they show up in slotNames and via dot access, which is
        // the point: a closure's metadata is just data on the object.
        // predicate handlers like operative?/pure? on the Block prototype
        // read these slots; the slot itself holds the raw data.
        let code_idx_sym = self.intern("code_idx");
        let arity_sym = self.intern("arity");
        let is_op_sym = self.intern("is_operative");
        let is_pure_sym = self.intern("is_pure");

        let mut slot_names: Vec<u32> = Vec::with_capacity(4 + captures.len());
        let mut slot_values: Vec<Value> = Vec::with_capacity(4 + captures.len());
        slot_names.push(code_idx_sym);  slot_values.push(Value::integer(code_idx as i64));
        slot_names.push(arity_sym);     slot_values.push(Value::integer(arity as i64));
        slot_names.push(is_op_sym);     slot_values.push(Value::boolean(is_operative));
        slot_names.push(is_pure_sym);   slot_values.push(Value::boolean(is_pure));
        for (sym, val) in captures {
            slot_names.push(*sym);
            slot_values.push(*val);
        }

        let id = self.alloc(HeapObject::General {
            parent,
            slot_names,
            slot_values,
            handlers: Vec::new(),
        });
        let val = Value::nursery(id);
        // set call: handler to self — VM dispatch looks up call: on the
        // receiver, finds this self-reference, sees a closure-shaped General
        // (has __code_idx slot), and jumps to bytecode.
        let call_sym = self.sym_call;
        self.get_mut(id).handler_set(call_sym, val);
        val
    }

    /// Check if a value is a closure object. Returns (code_idx, is_operative)
    /// if so. Detection is parent-based: closures delegate to PROTO_CLOSURE.
    /// Slot access uses fixed INDEXES (not names) to stay robust against
    /// cross-vat symbol-id mismatch during migration.
    pub fn as_closure(&self, val: Value) -> Option<(usize, bool)> {
        let id = val.as_any_object()?;
        let HeapObject::General { parent, slot_values, slot_names, .. } = self.get(id) else {
            return None;
        };
        // closure detection: parent must be PROTO_CLOSURE
        let closure_proto = self.type_protos.get(PROTO_CLOSURE).copied().unwrap_or(Value::NIL);
        if *parent != closure_proto || slot_names.len() < Self::CLOSURE_META_SLOTS {
            return None;
        }
        let code_idx = slot_values[0].as_integer()? as usize;
        let is_op = slot_values[2].is_true();
        Some((code_idx, is_op))
    }

    /// Get the captured values from a closure object — every slot whose name
    /// doesn't start with `__` (the metadata prefix).
    /// Metadata slots are always the first 4 entries in a closure's slot
    /// list: __code_idx, __arity, __is_operative, __is_pure. Captures
    /// follow. Skipping by INDEX (not by name) keeps this cross-vat-safe:
    /// migrated closures may carry source-heap symbol ids until the next
    /// intern pass, so we avoid symbol_name on slot_names entirely.
    const CLOSURE_META_SLOTS: usize = 4;

    pub fn closure_captures(&self, val: Value) -> Vec<(u32, Value)> {
        let Some(id) = val.as_any_object() else { return Vec::new(); };
        let HeapObject::General { slot_names, slot_values, .. } = self.get(id) else {
            return Vec::new();
        };
        if slot_names.len() <= Self::CLOSURE_META_SLOTS {
            return Vec::new();
        }
        slot_names.iter().skip(Self::CLOSURE_META_SLOTS)
            .zip(slot_values.iter().skip(Self::CLOSURE_META_SLOTS))
            .map(|(s, v)| (*s, *v))
            .collect()
    }

    /// Check if a closure is "pure" (no FarRef captures, safe to memoize).
    pub fn closure_is_pure(&self, val: Value) -> bool {
        let Some(id) = val.as_any_object() else { return false; };
        let HeapObject::General { slot_values, slot_names, .. } = self.get(id) else {
            return false;
        };
        if slot_names.len() < Self::CLOSURE_META_SLOTS { return false; }
        slot_values[3].is_true()
    }

    /// Return a closure's arity, if it is one.
    pub fn closure_arity(&self, val: Value) -> Option<u8> {
        let id = val.as_any_object()?;
        let HeapObject::General { slot_values, slot_names, .. } = self.get(id) else {
            return None;
        };
        if slot_names.len() < Self::CLOSURE_META_SLOTS { return None; }
        Some(slot_values[1].as_integer()? as u8)
    }

    /// Override the is_pure metadata flag on a closure. Needed because
    /// the compiler's post-construction FarRef scan may discover impurity
    /// after make_closure has already set the default.
    pub fn set_closure_pure(&mut self, val: Value, is_pure: bool) {
        let Some(id) = val.as_any_object() else { return; };
        if let HeapObject::General { slot_values, slot_names, .. } = self.get_mut(id) {
            if slot_names.len() >= Self::CLOSURE_META_SLOTS {
                slot_values[3] = Value::boolean(is_pure);
            }
        }
    }

    /// Get the prototype for any value (including primitives and optimized types).
    pub fn prototype_of(&self, val: Value) -> Value {
        // for heap objects, check the variant first
        if let Some(id) = val.as_any_object() {
            match self.get(id) {
                HeapObject::General { parent, .. } |
                HeapObject::Environment { parent, .. } => return *parent,
                HeapObject::Pair(_, _) => return self.type_protos.get(PROTO_CONS).copied().unwrap_or(Value::NIL),
                HeapObject::Text(_) => return self.type_protos.get(PROTO_STR).copied().unwrap_or(Value::NIL),
                HeapObject::Buffer(_) => return self.type_protos.get(PROTO_BYTES).copied().unwrap_or(Value::NIL),
                HeapObject::Table { .. } => return self.type_protos.get(PROTO_TABLE).copied().unwrap_or(Value::NIL),
            }
        }
        // for primitives, use type_protos by tag
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
            match self.get(id) {
                HeapObject::General { handlers, .. }
                | HeapObject::Environment { handlers, .. } => {
                    for &(sel, _) in handlers {
                        if seen.insert(sel) { names.push(sel); }
                    }
                }
                _ => {}
            }
        }

        // walk the prototype chain — accept any variant that carries handlers.
        let mut proto = self.prototype_of(val);
        for _ in 0..256 {
            if proto.is_nil() { break; }
            let Some(id) = proto.as_any_object() else { break; };
            let (handlers, next) = match self.get(id) {
                HeapObject::General { handlers, parent, .. }
                | HeapObject::Environment { handlers, parent, .. } => (handlers, *parent),
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
                match self.get(id) {
                    HeapObject::Pair(car, cdr) => {
                        if let Some(sym) = car.as_symbol() {
                            positional.push(sym);
                        }
                        current = *cdr;
                    }
                    _ => break,
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
    fn fixed_shape_slots() {
        let mut h = Heap::new();
        let x = h.intern("x");
        let y = h.intern("y");
        let obj = h.make_object_with_slots(Value::NIL, vec![x, y], vec![Value::integer(3), Value::integer(4)]);
        let id = obj.as_any_object().unwrap();

        // can read slots
        assert_eq!(h.get(id).slot_get(x), Some(Value::integer(3)));
        assert_eq!(h.get(id).slot_get(y), Some(Value::integer(4)));

        // can write existing slots
        assert!(h.get_mut(id).slot_set(x, Value::integer(99)));
        assert_eq!(h.get(id).slot_get(x), Some(Value::integer(99)));

        // cannot add new slots
        let z = h.intern("z");
        assert!(!h.get_mut(id).slot_set(z, Value::integer(0)));
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
