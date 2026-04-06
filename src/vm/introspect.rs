use crate::runtime::value::{Value, HeapObject};
use super::exec::VM;

impl VM {
    /// Get the type prototype for a value (if registered).
    pub(crate) fn type_prototype(&self, val: Value) -> Option<u32> {
        match val {
            Value::Integer(_) => self.proto_integer,
            Value::Float(_) => self.proto_float.or(self.proto_integer), // prefer Float proto, fall back to Integer
            Value::True | Value::False => self.proto_boolean,
            Value::Nil => self.proto_nil,
            Value::Symbol(_) => self.proto_symbol,
            Value::Object(id) => match self.heap.get(id) {
                HeapObject::MoofString(_) => self.proto_string,
                HeapObject::Cons { .. } => self.proto_cons,
                HeapObject::Lambda { .. } => self.proto_lambda,
                HeapObject::Operative { .. } => self.proto_operative,
                HeapObject::Environment(_) => self.proto_environment,
                _ => None,
            },
        }
    }

    /// Get all registered prototypes as a Vec of (name, id).
    pub fn get_protos(&self) -> Vec<(String, u32)> {
        let mut result = Vec::new();
        let list = [
            ("integer", self.proto_integer),
            ("float", self.proto_float),
            ("boolean", self.proto_boolean),
            ("string", self.proto_string),
            ("cons", self.proto_cons),
            ("nil", self.proto_nil),
            ("symbol", self.proto_symbol),
            ("lambda", self.proto_lambda),
            ("operative", self.proto_operative),
            ("environment", self.proto_environment),
        ];
        for (name, opt) in list {
            if let Some(id) = opt {
                result.push((name.to_string(), id));
            }
        }
        result
    }

    /// Restore prototypes from a Vec of (name, id).
    pub fn set_protos(&mut self, protos: Vec<(String, u32)>) {
        for (name, id) in protos {
            match name.as_str() {
                "integer" => self.proto_integer = Some(id),
                "float" => self.proto_float = Some(id),
                "boolean" => self.proto_boolean = Some(id),
                "string" => self.proto_string = Some(id),
                "cons" => self.proto_cons = Some(id),
                "nil" => self.proto_nil = Some(id),
                "symbol" => self.proto_symbol = Some(id),
                "lambda" => self.proto_lambda = Some(id),
                "operative" => self.proto_operative = Some(id),
                "environment" => self.proto_environment = Some(id),
                _ => {}
            }
        }
    }

    /// Find the Modules singleton object.
    pub fn find_module_registry(&self) -> Option<u32> {
        let root = self.vat.root_env?;
        let sym = self.heap.symbol_lookup_only("Modules")?;
        match self.env_lookup(root, sym) {
            Ok(Value::Object(id)) => Some(id),
            _ => None,
        }
    }

    /// Find a ModuleImage object by name.
    pub fn find_module(&mut self, name: &str) -> Option<u32> {
        let root = self.vat.root_env?;
        let sym_modules = self.heap.intern("Modules");
        let modules_obj = match self.env_lookup(root, sym_modules) {
            Ok(Value::Object(id)) => id,
            _ => return None,
        };

        // Send 'named: name' to Modules
        let sel_named = self.heap.intern("named:");
        let name_val = self.heap.alloc_string(name);
        match self.message_send(Value::Object(modules_obj), sel_named, &[name_val]) {
            Ok(Value::Object(id)) => Some(id),
            _ => None,
        }
    }

    /// Read a named slot from a GeneralObject (non-mutating).
    pub fn read_slot(&self, obj_id: u32, slot_name: &str) -> Value {
        let sym = match self.heap.symbol_lookup_only(slot_name) {
            Some(s) => s,
            None => return Value::Nil,
        };
        match self.heap.get(obj_id) {
            HeapObject::GeneralObject { slots, .. } => {
                slots.iter().find(|(k, _)| *k == sym).map(|(_, v)| *v).unwrap_or(Value::Nil)
            }
            _ => Value::Nil,
        }
    }

    /// Read a Value that should be a MoofString as a Rust String.
    pub fn read_string(&self, val: Value) -> Option<String> {
        match val {
            Value::Object(id) => match self.heap.get(id) {
                HeapObject::MoofString(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Read a Value that should be a list of strings as Vec<String>.
    pub fn read_string_list(&self, val: Value) -> Vec<String> {
        self.heap.list_to_vec(val).iter()
            .filter_map(|v| self.read_string(*v))
            .collect()
    }

    /// Read a Value that should be a list of symbols as Vec<String>.
    pub fn read_symbol_list(&self, val: Value) -> Vec<String> {
        self.heap.list_to_vec(val).iter()
            .filter_map(|v| match v {
                Value::Symbol(s) => Some(self.heap.symbol_name(*s).to_string()),
                _ => self.read_string(*v),
            })
            .collect()
    }

    /// Get all (name, mod_id) pairs from the Modules registry.
    /// Walks Modules._registry which is an Assoc with a `data` slot
    /// containing a list of (key . value) pairs.
    pub fn all_module_ids(&self) -> Vec<(String, u32)> {
        let root = match self.vat.root_env {
            Some(r) => r,
            None => return Vec::new(),
        };
        let sym_modules = match self.heap.symbol_lookup_only("Modules") {
            Some(s) => s,
            None => return Vec::new(),
        };
        let modules_obj = match self.env_lookup(root, sym_modules) {
            Ok(Value::Object(id)) => id,
            _ => return Vec::new(),
        };

        // Read _registry slot
        let registry_id = match self.read_slot(modules_obj, "_registry") {
            Value::Object(id) => id,
            _ => return Vec::new(),
        };

        // Assoc stores entries in "data" slot as list of (key . value) cons cells.
        // The stub registry uses "entries" instead.
        let data_val = self.read_slot(registry_id, "data");
        let entries_val = if data_val == Value::Nil {
            self.read_slot(registry_id, "entries")
        } else {
            data_val
        };

        let mut result = Vec::new();
        let entries = self.heap.list_to_vec(entries_val);
        for entry in entries {
            if let Value::Object(pair_id) = entry {
                if let HeapObject::Cons { car, cdr } = self.heap.get(pair_id) {
                    if let Some(name) = self.read_string(*car) {
                        if let Value::Object(mod_id) = cdr {
                            result.push((name, *mod_id));
                        }
                    }
                }
            }
        }
        result
    }

    /// Get Definition object ids from a ModuleImage's definitions list.
    pub fn definitions_list(&self, mod_id: u32) -> Vec<u32> {
        let defs_val = self.read_slot(mod_id, "definitions");
        self.heap.list_to_vec(defs_val).iter()
            .filter_map(|v| match v {
                Value::Object(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Read the source text from a Definition object.
    pub fn definition_source(&self, def_id: u32) -> Option<String> {
        let source_val = self.read_slot(def_id, "source");
        self.read_string(source_val)
    }

    /// Read the name from a Definition object.
    pub fn definition_name(&self, def_id: u32) -> Option<String> {
        let name_val = self.read_slot(def_id, "name");
        self.read_string(name_val)
    }
}
