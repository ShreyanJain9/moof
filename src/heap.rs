// The nursery: in-memory arena for all heap objects.
//
// Objects are allocated here and indexed by u32 ID.
// Value::nursery(id) references objects in this arena.
//
// Eventually, persistent objects will be promoted to LMDB (Value::object).
// For now, everything lives in the nursery.

use serde::{Serialize, Deserialize};
use crate::object::HeapObject;
use crate::value::Value;

#[derive(Serialize, Deserialize)]
pub struct HeapImage {
    pub objects: Vec<HeapObject>,
    pub symbols: Vec<String>,
    pub env_id: u32,                 // root environment object ID
}

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
pub const PROTO_ERROR: usize = 12;
pub const PROTO_FARREF: usize = 13;
pub const PROTO_ACT: usize = 14;

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
}

pub type NativeFn = Box<dyn Fn(&mut Heap, Value, &[Value]) -> Result<Value, String>>;

impl Heap {
    pub fn new() -> Self {
        let mut h = Heap {
            objects: Vec::new(),
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
            type_protos: vec![Value::NIL; 15],
            natives: Vec::new(),
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

    pub fn env_def(&mut self, sym: u32, val: Value) {
        self.get_mut(self.env).slot_set(sym, val);
    }

    pub fn env_remove(&mut self, sym: u32) {
        if let HeapObject::Environment { bindings, .. } = self.get_mut(self.env) {
            bindings.remove(&sym);
        }
    }

    // -- object allocation --

    pub fn alloc(&mut self, obj: HeapObject) -> u32 {
        let id = self.objects.len() as u32;
        self.objects.push(obj);
        id
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

    /// Create an Error object with a message string.
    pub fn make_error(&mut self, msg: &str) -> Value {
        let msg_val = self.alloc_string(msg);
        let parent = self.type_protos[PROTO_ERROR];
        let parent = if parent.is_nil() { self.type_protos[PROTO_OBJ] } else { parent };
        self.make_object_with_slots(parent, vec![self.sym_message], vec![msg_val])
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
        let act_proto = self.type_protos[PROTO_ACT];
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
        let act_proto = self.type_protos[PROTO_ACT];
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
        Value::symbol(sym) // the handler value IS the symbol — dispatch looks it up
    }

    pub fn find_native(&self, sym: u32) -> Option<usize> {
        self.natives.iter().position(|(s, _)| *s == sym)
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

    /// Create a closure object. Returns a nursery Value.
    pub fn make_closure(&mut self, code_idx: usize, arity: u8, is_operative: bool, captures: &[(u32, Value)]) -> Value {
        // closures delegate to Block prototype (→ Object)
        let parent = self.type_protos.get(PROTO_CLOSURE).copied()
            .unwrap_or_else(|| self.type_protos[PROTO_OBJ]);
        let id = self.alloc(HeapObject::Closure {
            parent,
            code_idx,
            arity,
            is_operative,
            captures: captures.to_vec(),
            handlers: Vec::new(),
        });
        let val = Value::nursery(id);
        // set call: handler to self — dispatch uses this to invoke the closure
        let call_sym = self.sym_call;
        self.get_mut(id).handler_set(call_sym, val);
        val
    }

    /// Check if a value is a closure object. Returns (code_idx, is_operative) if so.
    pub fn as_closure(&self, val: Value) -> Option<(usize, bool)> {
        let id = val.as_any_object()?;
        match self.get(id) {
            HeapObject::Closure { code_idx, is_operative, .. } => Some((*code_idx, *is_operative)),
            _ => None,
        }
    }

    /// Get the captured values from a closure object.
    pub fn closure_captures(&self, val: Value) -> Vec<(u32, Value)> {
        let Some(id) = val.as_any_object() else { return Vec::new(); };
        match self.get(id) {
            HeapObject::Closure { captures, .. } => captures.clone(),
            _ => Vec::new(),
        }
    }

    /// Save the heap to a file.
    pub fn save_image(&self, path: &str) -> Result<(), String> {
        let image = HeapImage {
            objects: self.objects.clone(),
            symbols: self.symbols.clone(),
            env_id: self.env,
        };
        let bytes = bincode::serialize(&image).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(path, bytes).map_err(|e| format!("write: {e}"))?;
        Ok(())
    }

    /// Load a heap from a file. Returns None if file doesn't exist.
    pub fn load_image(path: &str) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let image: HeapImage = bincode::deserialize(&bytes).ok()?;

        let mut h = Heap::new();
        h.objects = image.objects;
        h.symbols = image.symbols;
        h.env = image.env_id;
        // rebuild sym_reverse
        h.sym_reverse.clear();
        for (i, name) in h.symbols.iter().enumerate() {
            h.sym_reverse.insert(name.clone(), i as u32);
        }
        // re-intern well-known symbols (they should already exist)
        h.sym_car = *h.sym_reverse.get("car")?;
        h.sym_cdr = *h.sym_reverse.get("cdr")?;
        h.sym_call = *h.sym_reverse.get("call:")?;
        h.sym_slot_at = *h.sym_reverse.get("slotAt:")?;
        h.sym_slot_at_put = *h.sym_reverse.get("slotAt:put:")?;
        h.sym_slot_names = *h.sym_reverse.get("slotNames")?;
        h.sym_handler_names = *h.sym_reverse.get("handlerNames")?;
        h.sym_parent = *h.sym_reverse.get("parent")?;
        h.sym_describe = *h.sym_reverse.get("describe")?;
        h.sym_dnu = *h.sym_reverse.get("doesNotUnderstand:")?;
        h.sym_length = *h.sym_reverse.get("length")?;
        h.sym_at = *h.sym_reverse.get("at:")?;
        h.sym_at_put = *h.sym_reverse.get("at:put:")?;
        h.sym_message = h.sym_reverse.get("message").copied().unwrap_or_else(|| h.intern("message"));
        Some(h)
    }

    /// Get the prototype for any value (including primitives and optimized types).
    pub fn prototype_of(&self, val: Value) -> Value {
        // for heap objects, check the variant first
        if let Some(id) = val.as_any_object() {
            match self.get(id) {
                HeapObject::General { parent, .. } |
                HeapObject::Closure { parent, .. } |
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

        // for heap general objects, start with own handlers
        if let Some(id) = val.as_any_object() {
            if let HeapObject::General { handlers, .. } = self.get(id) {
                for &(sel, _) in handlers {
                    if seen.insert(sel) { names.push(sel); }
                }
            }
        }

        // walk the prototype chain
        let mut proto = self.prototype_of(val);
        for _ in 0..256 {
            if proto.is_nil() { break; }
            if let Some(id) = proto.as_any_object() {
                if let HeapObject::General { handlers, parent, .. } = self.get(id) {
                    for &(sel, _) in handlers {
                        if seen.insert(sel) { names.push(sel); }
                    }
                    proto = *parent;
                } else {
                    break;
                }
            } else {
                break;
            }
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

    /// Total object count (for stats).
    pub fn object_count(&self) -> usize { self.objects.len() }

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

// -- printing support --

impl Heap {
    /// Format a value for display, resolving symbols and walking cons lists.
    pub fn format_value(&self, val: Value) -> String {
        if val.is_nil() { return "nil".into(); }
        if val.is_true() { return "true".into(); }
        if val.is_false() { return "false".into(); }
        if let Some(n) = val.as_integer() { return n.to_string(); }
        if val.is_float() { return format!("{}", f64::from_bits(val.to_bits())); }
        if let Some(id) = val.as_symbol() {
            return format!("'{}", self.symbol_name(id));
        }
        if let Some(id) = val.as_any_object() {
            return self.format_object(id);
        }
        format!("?{:#018x}", val.to_bits())
    }

    fn format_object(&self, id: u32) -> String {
        match self.get(id) {
            HeapObject::Closure { is_operative, arity, .. } => {
                if *is_operative { format!("<operative arity:{arity}>") }
                else { format!("<fn arity:{arity}>") }
            }
            HeapObject::Pair(_, _) => self.format_list(id),
            HeapObject::Text(s) => format!("\"{}\"", s.replace('"', "\\\"")),
            HeapObject::Buffer(b) => format!("<bytes:{}>", b.len()),
            HeapObject::Table { seq, map } => {
                let mut parts = Vec::new();
                for v in seq { parts.push(self.format_value(*v)); }
                for (k, v) in map {
                    parts.push(format!("{} => {}", self.format_value(*k), self.format_value(*v)));
                }
                format!("#[{}]", parts.join(" "))
            }
            HeapObject::General { slot_names, slot_values, .. } => {
                if slot_names.is_empty() {
                    return format!("<object#{id}>");
                }
                let slots: Vec<_> = slot_names.iter().zip(slot_values.iter())
                    .map(|(n, v)| format!("{}: {}", self.symbol_name(*n), self.format_value(*v)))
                    .collect();
                format!("{{ {} }}", slots.join(" "))
            }
            HeapObject::Environment { bindings, .. } => {
                format!("<environment: {} bindings>", bindings.len())
            }
        }
    }

    fn format_list(&self, mut id: u32) -> String {
        let mut items = Vec::new();
        let mut tail = Value::NIL;
        loop {
            match self.get(id) {
                HeapObject::Pair(car, cdr) => {
                    items.push(self.format_value(*car));
                    if cdr.is_nil() {
                        break;
                    } else if let Some(next) = cdr.as_any_object() {
                        if matches!(self.get(next), HeapObject::Pair(_, _)) {
                            id = next;
                            continue;
                        }
                    }
                    // dotted pair
                    tail = *cdr;
                    break;
                }
                _ => break,
            }
        }
        if tail.is_nil() {
            format!("({})", items.join(" "))
        } else {
            format!("({} . {})", items.join(" "), self.format_value(tail))
        }
    }

    /// Rich display for the REPL — shows the nature of things.
    pub fn display_value(&self, val: Value) -> String {
        if val.is_nil() { return "nil".into(); }
        if val.is_true() { return "true".into(); }
        if val.is_false() { return "false".into(); }
        if let Some(n) = val.as_integer() { return format!("{n}  : Integer"); }
        if val.is_float() {
            return format!("{}  : Float", f64::from_bits(val.to_bits()));
        }
        if let Some(id) = val.as_symbol() {
            return format!("'{}", self.symbol_name(id));
        }
        if let Some(id) = val.as_any_object() {
            return self.display_object(id);
        }
        format!("?{:#018x}", val.to_bits())
    }

    fn display_object(&self, id: u32) -> String {
        match self.get(id) {
            HeapObject::Closure { is_operative, arity, .. } => {
                if *is_operative { format!("<operative arity:{arity}>") }
                else { format!("<fn arity:{arity}>") }
            }
            HeapObject::Pair(_, _) => {
                let formatted = self.format_list(id);
                let len = self.list_len(id);
                format!("{formatted}  : Cons ({len} elements)")
            }
            HeapObject::Text(s) => {
                if s.len() > 60 {
                    format!("\"{}...\"  : String ({} chars)", &s[..57], s.len())
                } else {
                    format!("\"{s}\"  : String")
                }
            }
            HeapObject::Buffer(b) => format!("<{} bytes>  : Bytes", b.len()),
            HeapObject::Table { seq, map } => {
                let mut parts = Vec::new();
                for v in seq { parts.push(self.format_value(*v)); }
                for (k, v) in map {
                    parts.push(format!("{} => {}", self.format_value(*k), self.format_value(*v)));
                }
                format!("#[{}]  : Table ({} seq, {} map)", parts.join(" "), seq.len(), map.len())
            }
            HeapObject::General { parent: _, slot_names, slot_values, handlers } => {
                if slot_names.is_empty() && handlers.is_empty() {
                    return format!("<object#{id}>");
                }
                let nslots = slot_names.len();
                let nhandlers = handlers.len();

                // compact display for small objects
                if nslots <= 4 && nhandlers == 0 {
                    let slots: Vec<_> = slot_names.iter().zip(slot_values.iter())
                        .map(|(n, v)| format!("{}: {}", self.symbol_name(*n), self.format_value(*v)))
                        .collect();
                    return format!("{{ {} }}", slots.join(", "));
                }

                // rich multi-line display
                let mut lines = Vec::new();
                for (n, v) in slot_names.iter().zip(slot_values.iter()) {
                    lines.push(format!("    {}: {}", self.symbol_name(*n), self.format_value(*v)));
                }
                let handler_names: Vec<_> = handlers.iter()
                    .map(|(s, _)| self.symbol_name(*s).to_string())
                    .collect();

                let handler_info = if nhandlers == 0 {
                    String::new()
                } else if nhandlers <= 6 {
                    format!("\n    responds to: {}", handler_names.join(", "))
                } else {
                    format!("\n    responds to: {}, ... ({nhandlers} total)",
                        handler_names[..4].join(", "))
                };

                format!("  {{ {nslots} slots, {nhandlers} handlers{handler_info}\n{}\n  }}", lines.join("\n"))
            }
            HeapObject::Environment { bindings, .. } => {
                format!("<environment: {} bindings>", bindings.len())
            }
        }
    }

    fn list_len(&self, mut id: u32) -> usize {
        let mut count = 0;
        loop {
            match self.get(id) {
                HeapObject::Pair(_, cdr) => {
                    count += 1;
                    if let Some(next) = cdr.as_any_object() {
                        if matches!(self.get(next), HeapObject::Pair(_, _)) {
                            id = next;
                            continue;
                        }
                    }
                    break;
                }
                _ => break,
            }
        }
        count
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
