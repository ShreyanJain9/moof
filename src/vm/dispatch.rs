/// Message dispatch and call methods for the MOOF VM.
///
/// Extracted from exec.rs — these are the core dispatch operations:
/// call_value, call_lambda, call_operative, bind_params,
/// message_send, lookup_handler, primitive_send, eval.

use crate::runtime::value::{Value, HeapObject};
use super::exec::{VM, VMResult};

impl VM {
    /// Call a value as a function: [callable call: args...]
    pub(crate) fn call_value(&mut self, callable: Value, args: &[Value]) -> VMResult {
        match callable {
            Value::Object(id) => {
                match self.heap.get(id).clone() {
                    HeapObject::Lambda { params, body, def_env, .. } => {
                        self.call_lambda(params, body, def_env, args)
                    }
                    HeapObject::Operative { .. } => {
                        Err("Cannot call an operative with evaluated arguments — use vau syntax".into())
                    }
                    HeapObject::NativeFunction { name } => {
                        let name = name.clone();
                        self.call_native(&name, args)
                    }
                    _ => {
                        // Try message send: [callable call: args...]
                        self.message_send(callable, self.sym_call, args)
                    }
                }
            }
            _ => Err(format!("Cannot call {:?}", callable)),
        }
    }

    /// Call a lambda: create a new environment, bind params, execute body.
    /// Handles both positional params (a b c) and rest params (args) and
    /// dotted rest (a b . rest).
    pub(crate) fn call_lambda(&mut self, params: Value, body: u32, def_env: u32, args: &[Value]) -> VMResult {
        let new_env_id = self.heap.alloc_env(Some(def_env));
        // Use bind_params for proper destructuring (handles rest params)
        let args_list = self.heap.list(args);
        self.bind_params(new_env_id, params, args_list);
        self.execute(body, new_env_id)
    }

    /// Call an operative with unevaluated args and the caller's environment.
    pub(crate) fn call_operative(&mut self, operative: Value, args_list: Value, caller_env: u32) -> VMResult {
        match operative {
            Value::Object(id) => {
                match self.heap.get(id).clone() {
                    HeapObject::Operative { params, env_param, body, def_env, .. } => {
                        let new_env_id = self.heap.alloc_env(Some(def_env));
                        // Bind the parameter list to the unevaluated args
                        self.bind_params(new_env_id, params, args_list);
                        // Bind the environment parameter to the caller's env
                        self.env_define(new_env_id, env_param, Value::Object(caller_env));
                        self.execute(body, new_env_id)
                    }
                    _ => Err("call_operative: not an operative".into()),
                }
            }
            _ => Err(format!("call_operative: expected object, got {:?}", operative)),
        }
    }

    /// Bind parameters to arguments (destructuring cons lists).
    pub(crate) fn bind_params(&mut self, env_id: u32, params: Value, args: Value) {
        match params {
            Value::Symbol(sym) => {
                // Rest parameter — bind the whole list
                self.env_define(env_id, sym, args);
            }
            Value::Nil => {} // no params
            Value::Object(pid) => {
                match self.heap.get(pid).clone() {
                    HeapObject::Cons { car, cdr } => {
                        let arg_car = self.heap.car(args);
                        let arg_cdr = self.heap.cdr(args);
                        self.bind_params(env_id, car, arg_car);
                        self.bind_params(env_id, cdr, arg_cdr);
                    }
                    _ => {} // ignore non-cons
                }
            }
            _ => {} // ignore non-bindable
        }
    }

    /// The core message send: look up a handler and invoke it.
    /// "the vm's single privileged operation is `send`" (§0)
    ///
    /// Dispatch order:
    /// 1. Handler on the receiver itself (GeneralObject delegation chain)
    /// 2. Handler on the type prototype (Integer, Boolean, String, etc.)
    /// 3. Universal introspection protocol (handlerNames, parent, handlerAt:)
    /// 4. doesNotUnderstand:
    ///
    /// There is no fallback, no hidden substrate. If the handler isn't in
    /// the table, it's doesNotUnderstand. One path. One mechanism.
    pub fn message_send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> VMResult {
        // 1. For GeneralObjects, check user-defined handlers (§4.2)
        if let Value::Object(id) = receiver {
            if let HeapObject::GeneralObject { .. } = self.heap.get(id) {
                if let Some(handler) = self.lookup_handler(id, selector) {
                    let mut full_args = vec![receiver];
                    full_args.extend_from_slice(args);
                    return self.call_value(handler, &full_args);
                }
            }
        }

        // 2. Check type prototype handlers
        if let Some(proto_id) = self.type_prototype(receiver) {
            if let Some(handler) = self.lookup_handler(proto_id, selector) {
                let mut full_args = vec![receiver];
                full_args.extend_from_slice(args);
                return self.call_value(handler, &full_args);
            }
        }

        // 3. Universal introspection protocol
        //    These work on any value — primitives route to their type prototype,
        //    GeneralObjects introspect their own structure.
        let sel_name = self.heap.symbol_name(selector).to_string();
        match sel_name.as_str() {
            "handlerNames" => {
                if let Value::Object(id) = receiver {
                    if let HeapObject::GeneralObject { handlers, .. } = self.heap.get(id) {
                        let names: Vec<Value> = handlers.iter()
                            .map(|(k, _)| Value::Symbol(*k)).collect();
                        return Ok(self.heap.list(&names));
                    }
                }
                if let Some(proto_id) = self.type_prototype(receiver) {
                    if let HeapObject::GeneralObject { handlers, .. } = self.heap.get(proto_id) {
                        let names: Vec<Value> = handlers.iter()
                            .map(|(k, _)| Value::Symbol(*k)).collect();
                        return Ok(self.heap.list(&names));
                    }
                }
                return Ok(Value::Nil);
            }
            "parent" => {
                if let Value::Object(id) = receiver {
                    if let HeapObject::GeneralObject { parent, .. } = self.heap.get(id) {
                        return Ok(*parent);
                    }
                }
                if let Some(proto_id) = self.type_prototype(receiver) {
                    return Ok(Value::Object(proto_id));
                }
                return Ok(Value::Nil);
            }
            "handlerAt:" => {
                let key = args.first().and_then(|v| v.as_symbol())
                    .ok_or("handlerAt: expects a symbol")?;
                if let Value::Object(id) = receiver {
                    if let HeapObject::GeneralObject { handlers, .. } = self.heap.get(id) {
                        let handler = handlers.iter()
                            .find(|(k, _)| *k == key)
                            .map(|(_, v)| *v).unwrap_or(Value::Nil);
                        return Ok(handler);
                    }
                }
                if let Some(proto_id) = self.type_prototype(receiver) {
                    if let HeapObject::GeneralObject { handlers, .. } = self.heap.get(proto_id) {
                        let handler = handlers.iter()
                            .find(|(k, _)| *k == key)
                            .map(|(_, v)| *v).unwrap_or(Value::Nil);
                        return Ok(handler);
                    }
                }
                return Ok(Value::Nil);
            }
            _ => {}
        }

        // 4. doesNotUnderstand:
        if selector != self.sym_does_not_understand {
            if let Value::Object(id) = receiver {
                if let Some(dnu_handler) = self.lookup_handler(id, self.sym_does_not_understand) {
                    let sel_sym = Value::Symbol(selector);
                    let args_list = self.heap.list(args);
                    let full_args = vec![receiver, sel_sym, args_list];
                    return self.call_value(dnu_handler, &full_args);
                }
            }
            if let Some(proto_id) = self.type_prototype(receiver) {
                if let Some(dnu_handler) = self.lookup_handler(proto_id, self.sym_does_not_understand) {
                    let sel_sym = Value::Symbol(selector);
                    let args_list = self.heap.list(args);
                    let full_args = vec![receiver, sel_sym, args_list];
                    return self.call_value(dnu_handler, &full_args);
                }
            }
        }

        Err(format!("doesNotUnderstand: {} on {:?}", sel_name, receiver))
    }

    /// Look up a handler in the delegation chain (§4.2).
    pub(crate) fn lookup_handler(&self, obj_id: u32, selector: u32) -> Option<Value> {
        let mut current = Some(obj_id);
        while let Some(id) = current {
            match self.heap.get(id) {
                HeapObject::GeneralObject { parent, handlers, .. } => {
                    for &(sel, handler) in handlers {
                        if sel == selector {
                            return Some(handler);
                        }
                    }
                    // Delegate to parent
                    match parent {
                        Value::Object(pid) => current = Some(*pid),
                        _ => current = None,
                    }
                }
                _ => return None,
            }
        }
        None
    }

    // primitive_send is dead. all dispatch goes through handler lookup.
    // "the vm's single privileged operation is send." — §0

    /// Evaluate an expression in an environment (used by eval:, the REPL, etc).
    /// This is the tree-walking fallback for when we need to eval AST directly.
    pub fn eval(&mut self, expr: Value, env_id: u32) -> VMResult {
        use crate::compiler::compile::Compiler;
        let mut compiler = Compiler::new();
        let chunk = compiler.compile_expr(&mut self.heap, expr)?;
        let chunk_id = self.heap.alloc_chunk(chunk);
        self.execute(chunk_id, env_id)
    }
}
