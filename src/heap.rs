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
    pub globals: Vec<(String, u64)>, // name → value bits
    pub operatives: Vec<String>,     // operative symbol names
}

pub struct Heap {
    objects: Vec<HeapObject>,
    symbols: Vec<String>,
    sym_reverse: std::collections::HashMap<String, u32>,
    pub globals: std::collections::HashMap<u32, Value>, // top-level defs
    pub operatives: std::collections::HashSet<u32>,    // symbols bound to vau operatives

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
    pub sym_code_idx: u32,    // __code_idx — closure code index
    pub sym_arity: u32,       // __arity
    pub sym_operative: u32,   // __operative

    // type prototypes: indexed by Value::type_tag()
    // 0=nil, 1=bool, 2=int, 3=float, 4=symbol, 5=object
    // plus: 6=cons, 7=string, 8=bytes, 9=array, 10=map, 11=block
    pub type_protos: [Value; 12],

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
            globals: std::collections::HashMap::new(),
            operatives: std::collections::HashSet::new(),
            sym_car: 0, sym_cdr: 0, sym_call: 0,
            sym_slot_at: 0, sym_slot_at_put: 0,
            sym_slot_names: 0, sym_handler_names: 0,
            sym_parent: 0, sym_describe: 0, sym_dnu: 0,
            sym_length: 0, sym_at: 0, sym_at_put: 0,
            sym_code_idx: 0, sym_arity: 0, sym_operative: 0,
            type_protos: [Value::NIL; 12],
            natives: Vec::new(),
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
        h.sym_code_idx = h.intern("__code_idx");
        h.sym_arity = h.intern("__arity");
        h.sym_operative = h.intern("__operative");

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
        let sym = self.intern(name);
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
        let mut slot_names = vec![self.sym_code_idx, self.sym_arity, self.sym_operative];
        let mut slot_values: Vec<Value> = vec![
            Value::integer(code_idx as i64),
            Value::integer(arity as i64),
            Value::boolean(is_operative),
        ];
        // add captured values as named slots
        for &(name, val) in captures {
            slot_names.push(name);
            slot_values.push(val);
        }
        let id = self.alloc(HeapObject::new_general(Value::NIL, slot_names, slot_values));
        let val = Value::nursery(id);
        // set call: handler to self — dispatch recognizes closures by __code_idx
        let call_sym = self.sym_call;
        self.get_mut(id).handler_set(call_sym, val);
        val
    }

    /// Check if a value is a closure object. Returns (code_idx, is_operative) if so.
    pub fn as_closure(&self, val: Value) -> Option<(usize, bool)> {
        let id = val.as_any_object()?;
        let code_idx_val = self.get(id).slot_get(self.sym_code_idx)?;
        let code_idx = code_idx_val.as_integer()?;
        if code_idx < 0 { return None; }
        let is_operative = self.get(id).slot_get(self.sym_operative)
            .map(|v| v.is_true()).unwrap_or(false);
        Some((code_idx as usize, is_operative))
    }

    /// Get the captured values from a closure object (slots beyond __code_idx, __arity, __operative).
    pub fn closure_captures(&self, val: Value) -> Vec<(u32, Value)> {
        let Some(id) = val.as_any_object() else { return Vec::new(); };
        match self.get(id) {
            HeapObject::General { slot_names, slot_values, .. } => {
                // skip the first 3 slots (__code_idx, __arity, __operative)
                slot_names.iter().zip(slot_values.iter())
                    .skip(3)
                    .map(|(&n, &v)| (n, v))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Save the heap to a file.
    pub fn save_image(&self, path: &str) -> Result<(), String> {
        let globals: Vec<(String, u64)> = self.globals.iter()
            .map(|(&sym, &val)| (self.symbol_name(sym).to_string(), val.to_bits()))
            .collect();
        let operatives: Vec<String> = self.operatives.iter()
            .map(|&sym| self.symbol_name(sym).to_string())
            .collect();
        let image = HeapImage {
            objects: self.objects.clone(),
            symbols: self.symbols.clone(),
            globals,
            operatives,
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
        // rebuild sym_reverse
        h.sym_reverse.clear();
        for (i, name) in h.symbols.iter().enumerate() {
            h.sym_reverse.insert(name.clone(), i as u32);
        }
        // restore globals
        for (name, bits) in &image.globals {
            let sym = *h.sym_reverse.get(name.as_str())?;
            h.globals.insert(sym, Value::from_bits(*bits));
        }
        // restore operatives
        for name in &image.operatives {
            if let Some(&sym) = h.sym_reverse.get(name.as_str()) {
                h.operatives.insert(sym);
            }
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
        h.sym_code_idx = *h.sym_reverse.get("__code_idx")?;
        h.sym_arity = *h.sym_reverse.get("__arity")?;
        h.sym_operative = *h.sym_reverse.get("__operative")?;
        Some(h)
    }

    /// Get the prototype for any value (including primitives and optimized types).
    pub fn prototype_of(&self, val: Value) -> Value {
        // for heap objects, check the variant first
        if let Some(id) = val.as_any_object() {
            match self.get(id) {
                HeapObject::General { parent, .. } => return *parent,
                HeapObject::Pair(_, _) => return self.type_protos.get(6).copied().unwrap_or(Value::NIL),
                HeapObject::Text(_) => return self.type_protos.get(7).copied().unwrap_or(Value::NIL),
                HeapObject::Buffer(_) => return self.type_protos.get(8).copied().unwrap_or(Value::NIL),
                HeapObject::Table { .. } => return self.type_protos.get(9).copied().unwrap_or(Value::NIL),
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

    /// Total object count (for stats).
    pub fn object_count(&self) -> usize { self.objects.len() }

    pub fn objects_ref(&self) -> &[HeapObject] { &self.objects }
    pub fn symbols_ref(&self) -> &[String] { &self.symbols }

    /// Restore a heap from saved data.
    pub fn restore(
        objects: Vec<HeapObject>,
        symbols: Vec<String>,
        globals: std::collections::HashMap<u32, Value>,
        operatives: std::collections::HashSet<u32>,
    ) -> Self {
        let mut h = Heap::new();
        h.objects = objects;
        h.symbols = symbols;
        h.sym_reverse.clear();
        for (i, name) in h.symbols.iter().enumerate() {
            h.sym_reverse.insert(name.clone(), i as u32);
        }
        h.globals = globals;
        h.operatives = operatives;
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
        h.sym_code_idx = h.sym_reverse.get("__code_idx").copied().unwrap_or(0);
        h.sym_arity = h.sym_reverse.get("__arity").copied().unwrap_or(0);
        h.sym_operative = h.sym_reverse.get("__operative").copied().unwrap_or(0);
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
        // check if it's a closure object before generic object formatting
        if self.as_closure(val).is_some() { return "<fn>".into(); }
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
            HeapObject::General { parent, slot_names, slot_values, .. } => {
                if slot_names.is_empty() {
                    return format!("<object#{id}>");
                }
                let slots: Vec<_> = slot_names.iter().zip(slot_values.iter())
                    .map(|(n, v)| format!("{}: {}", self.symbol_name(*n), self.format_value(*v)))
                    .collect();
                format!("{{ {} }}", slots.join(" "))
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
        if self.as_closure(val).is_some() { return "<fn>".into(); }
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
