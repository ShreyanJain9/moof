// Display formatting for Heap values.
//
// - format_value: unambiguous round-trippable representation
//   (matches object's describe handler where possible).
// - display_value: rich REPL presentation, with type annotations
//   and length counts.

use crate::object::HeapObject;
use crate::value::Value;
use super::Heap;

/// Hard cap on format depth as a safety net; in practice cycle detection
/// catches Env/self-ref loops earlier.
const FORMAT_DEPTH_LIMIT: usize = 32;

impl Heap {
    /// Format a value for display — unambiguous, round-trippable.
    pub fn format_value(&self, val: Value) -> String {
        self.format_value_at(val, &mut Vec::new())
    }

    pub(crate) fn format_value_at(&self, val: Value, visiting: &mut Vec<u32>) -> String {
        if visiting.len() >= FORMAT_DEPTH_LIMIT { return "...".into(); }
        if val.is_nil() { return "nil".into(); }
        if val.is_true() { return "true".into(); }
        if val.is_false() { return "false".into(); }
        if let Some(n) = val.as_integer() { return n.to_string(); }
        if val.is_float() { return format!("{}", f64::from_bits(val.to_bits())); }
        if let Some(id) = val.as_symbol() {
            return format!("'{}", self.symbol_name(id));
        }
        if let Some(id) = val.as_any_object() {
            // cycle detection: if we're already mid-format on this id
            // somewhere up the stack, bail — Env containing its own
            // binding is the normal case.
            if visiting.contains(&id) {
                return format!("<cycle#{id}>");
            }
            return self.format_object_at(id, visiting);
        }
        format!("?{:#018x}", val.to_bits())
    }

    fn format_object_at(&self, id: u32, visiting: &mut Vec<u32>) -> String {
        // closures are Generals with a code_idx slot — detect + format specially.
        if let Some((_, is_op)) = self.as_closure(Value::nursery(id)) {
            let arity = self.closure_arity(Value::nursery(id)).unwrap_or(0);
            return if is_op { format!("<operative arity:{arity}>") }
                   else { format!("<fn arity:{arity}>") };
        }
        visiting.push(id);
        let result = match self.get(id) {
            HeapObject::Pair(_, _) => self.format_list_at(id, visiting),
            HeapObject::Text(s) => format!("\"{}\"", s.replace('"', "\\\"")),
            HeapObject::Buffer(b) => format!("<bytes:{}>", b.len()),
            HeapObject::Table { seq, map } => {
                let mut parts = Vec::new();
                for v in seq { parts.push(self.format_value_at(*v, visiting)); }
                for (k, v) in map {
                    parts.push(format!("{} => {}",
                        self.format_value_at(*k, visiting),
                        self.format_value_at(*v, visiting)));
                }
                format!("#[{}]", parts.join(" "))
            }
            HeapObject::General { slot_names, slot_values, foreign, .. } => {
                // foreign payload takes precedence — its describe() is
                // the type author's intended presentation.
                if let Some(fd) = foreign.as_ref()
                    .and_then(|fd| self.foreign_registry().vtable(fd.type_id).map(|vt| (fd, vt)))
                {
                    (fd.1.describe)(&*fd.0.payload)
                } else if slot_names.is_empty() {
                    format!("<object#{id}>")
                } else {
                    let slots: Vec<_> = slot_names.iter().zip(slot_values.iter())
                        .map(|(n, v)| format!("{}: {}",
                            self.symbol_name(*n),
                            self.format_value_at(*v, visiting)))
                        .collect();
                    format!("{{ {} }}", slots.join(" "))
                }
            }
        };
        visiting.pop();
        result
    }

    fn format_object(&self, id: u32) -> String {
        self.format_object_at(id, &mut Vec::new())
    }

    fn format_list_at(&self, mut id: u32, visiting: &mut Vec<u32>) -> String {
        let mut items = Vec::new();
        let mut tail = Value::NIL;
        loop {
            match self.get(id) {
                HeapObject::Pair(car, cdr) => {
                    items.push(self.format_value_at(*car, visiting));
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
            format!("({} . {})", items.join(" "), self.format_value_at(tail, visiting))
        }
    }

    /// Rich display for the REPL — shows type annotations + counts.
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
        if let Some((_, is_op)) = self.as_closure(Value::nursery(id)) {
            let arity = self.closure_arity(Value::nursery(id)).unwrap_or(0);
            return if is_op { format!("<operative arity:{arity}>") }
                   else { format!("<fn arity:{arity}>") };
        }
        match self.get(id) {
            HeapObject::Pair(_, _) => {
                let formatted = self.format_list_at(id, &mut Vec::new());
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
            HeapObject::General { slot_names, slot_values, handlers, .. } => {
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
