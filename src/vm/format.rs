use crate::runtime::value::{Value, HeapObject};
use super::exec::VM;

impl VM {
    /// Format a value for display.
    pub fn format_value(&self, val: Value) -> String {
        match val {
            Value::Nil => "nil".to_string(),
            Value::True => "true".to_string(),
            Value::False => "false".to_string(),
            Value::Integer(n) => n.to_string(),
            Value::Float(f) => format!("{}", f),
            Value::Symbol(id) => format!("'{}", self.heap.symbol_name(id)),
            Value::Object(id) => {
                match self.heap.get(id) {
                    HeapObject::Cons { .. } => self.format_list(val),
                    HeapObject::MoofString(s) => format!("\"{}\"", s),
                    HeapObject::GeneralObject { .. } => format!("<object #{}>", id),
                    HeapObject::BytecodeChunk(_) => "<bytecode>".to_string(),
                    HeapObject::Operative { .. } => "<operative>".to_string(),
                    HeapObject::Lambda { .. } => "<lambda>".to_string(),
                    HeapObject::Environment(_) => "<environment>".to_string(),
                    HeapObject::NativeFunction { name } => {
                        format!("<native {}>", name)
                    }
                }
            }
        }
    }

    /// Format a cons-list for display.
    fn format_list(&self, val: Value) -> String {
        let mut parts = Vec::new();
        let mut current = val;
        loop {
            match current {
                Value::Nil => break,
                Value::Object(id) => {
                    match self.heap.get(id) {
                        HeapObject::Cons { car, cdr } => {
                            parts.push(self.format_value(*car));
                            current = *cdr;
                        }
                        _ => {
                            parts.push(format!(". {}", self.format_value(current)));
                            break;
                        }
                    }
                }
                other => {
                    parts.push(format!(". {}", self.format_value(other)));
                    break;
                }
            }
        }
        format!("({})", parts.join(" "))
    }
}
