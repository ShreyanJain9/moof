use moof_core::native;
use moof_core::heap::*;
use moof_core::object::HeapObject;
use moof_core::value::Value;

pub struct CorePlugin;

impl moof_core::Plugin for CorePlugin {
    fn name(&self) -> &str { "core" }

    fn register(&self, heap: &mut Heap) {
        // pre-intern symbols used by the compiler's defmethod
        heap.intern("self");

        // -- Object prototype (root of all delegation) --
        let object_proto = heap.make_object(Value::NIL);
        heap.type_protos[PROTO_OBJ] = object_proto;
        let obj_id = object_proto.as_any_object().unwrap();


        // fix up root environment's VM-internal proto to Object (was NIL at
        // allocation time — before Object existed). the env's `parent` slot is
        // the semantic outer-scope pointer (NIL at root) and stays untouched.
        let env_id = heap.env;
        heap.get_mut(env_id).set_proto(object_proto);

        // Object: slotAt: — real slots plus virtual slots contributed
        // by foreign payloads (Pair's car/cdr, Vec3's x/y/z, etc.
        // all flow through Heap::slot_of).
        native(heap, obj_id, "slotAt:", |heap, receiver, args| {
            let name = args.first().and_then(|v| v.as_symbol()).ok_or("slotAt: arg must be a symbol")?;
            if let Some(id) = receiver.as_any_object() {
                Ok(heap.slot_of(id, name).unwrap_or(Value::NIL))
            } else {
                Ok(Value::NIL) // primitives have no slots
            }
        });

        // slotAt:put: used to live here as a primitive mutation handler;
        // it is deliberately NOT registered now. the only sanctioned way
        // to change state is a server's Update return. the VM's internal
        // slot_set is used by process_handler_result to apply Update deltas,
        // but there is no userland-callable message for in-place slot writes.

        // Object: with: — non-destructive slot update. returns a new object.
        native(heap, obj_id, "with:", |heap, receiver, args| {
            let overrides = args.first().copied().ok_or("with: needs an object")?;
            let override_id = overrides.as_any_object().ok_or("with: arg must be an object")?;

            // get the original object's slots + handlers
            let recv_id = receiver.as_any_object().ok_or("with: receiver must be an object")?;
            let recv_obj = heap.get(recv_id);
            let orig_proto = recv_obj.proto;
            let orig_names = recv_obj.slot_names.clone();
            let orig_vals = recv_obj.slot_values.clone();
            let orig_handlers = recv_obj.handlers.clone();
            let orig_foreign = recv_obj.foreign.clone();

            let override_names_syms: Vec<u32> = heap.get(override_id).slot_names();
            let override_vals: Vec<Value> = override_names_syms.iter()
                .map(|n| heap.get(override_id).slot_get(*n).unwrap_or(Value::NIL))
                .collect();

            let mut new_names = orig_names;
            let mut new_vals = orig_vals;
            for (name, val) in override_names_syms.iter().zip(override_vals.iter()) {
                if let Some(i) = new_names.iter().position(|n| *n == *name) {
                    new_vals[i] = *val;
                } else {
                    new_names.push(*name);
                    new_vals.push(*val);
                }
            }

            let new_obj = heap.alloc_val(HeapObject {
                proto: orig_proto,
                slot_names: new_names,
                slot_values: new_vals,
                handlers: orig_handlers,
                foreign: orig_foreign,
            });

            // closures install a self-referential `call:` handler at
            // construction so the VM's `[closure call: args]` unpack
            // path triggers (handler == recv). cloning copies that
            // handler verbatim — but it now points at the ORIGINAL,
            // not the clone, so the unpack path is bypassed and the
            // original closure ends up invoked with the clone as
            // self. fix it: if the clone has a `call:` handler that
            // pointed at the original, rewrite it to point at the
            // clone. preserves callability of [fn with: { ... }].
            let new_id = new_obj.as_any_object().unwrap();
            let call_sym = heap.sym_call;
            if let Some(h) = heap.get(new_id).handler_get(call_sym) {
                if h == receiver {
                    heap.get_mut(new_id).handler_set(call_sym, new_obj);
                }
            }
            Ok(new_obj)
        });

        // Object: parent — works for ALL types (primitives, optimized variants, general objects)
        native(heap, obj_id, "parent", |heap, receiver, _args| {
            Ok(heap.prototype_of(receiver))
        });

        // Object: content-hash — content-addressable identity. Same
        // content → same hex string everywhere, always. Stable across
        // runs, processes, machines. The primitive underneath the
        // URL/resolver story (see docs/persistence.md,
        // docs/addressing.md). Distinct from `hash` (Hashable protocol,
        // fnv-ish i64 used by collections for bucketing).
        native(heap, obj_id, "content-hash", |heap, receiver, _args| {
            let h = heap.hash_value(receiver);
            Ok(heap.alloc_string(&moof_core::hash_hex(&h)))
        });

        // Object: slotNames — real slots plus foreign virtual-slot names.
        native(heap, obj_id, "slotNames", |heap, receiver, _args| {
            if let Some(id) = receiver.as_any_object() {
                let names = heap.slot_names_of(id);
                let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
                Ok(heap.list(&syms))
            } else {
                Ok(Value::NIL) // primitives have no slots
            }
        });

        // Object: handlerNames — walks the full prototype chain for ALL types
        native(heap, obj_id, "handlerNames", |heap, receiver, _args| {
            let names = heap.all_handler_names(receiver);
            let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
            Ok(heap.list(&syms))
        });

        // Object: __form-text — verbatim source text for any
        // parsed form (cons cell or other heap value), looked up
        // in heap.form_locations. returns nil if the value
        // wasn't recorded by the parser (literals, runtime-built
        // forms, foreign values without a parsed origin).
        //
        // populating definers (defmethod, defprotocol's provide,
        // etc.) call this on subforms to bundle precise verbatim
        // text — including comments, whitespace, and the user's
        // bracket choices — into the :source slot.
        native(heap, obj_id, "__form-text", |heap, receiver, _args| {
            let Some(id) = receiver.as_any_object() else {
                return Ok(Value::NIL);
            };
            match heap.form_locations.get(&id) {
                Some(loc) => {
                    let s = loc.slice().to_string();
                    Ok(heap.alloc_string(&s))
                }
                None => Ok(Value::NIL),
            }
        });

        // Object: handle:with:
        native(heap, obj_id, "handle:with:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("handle:with: receiver is not a mutable object")?;
            let sel = args.first().and_then(|v| v.as_symbol()).ok_or("handle:with: selector must be a symbol")?;
            let handler = args.get(1).copied().ok_or("handle:with: need handler value")?;
            heap.get_mut(id).handler_set(sel, handler);
            // flush send cache — any previously-cached (proto, sel) entries
            // might now be stale, since the new handler could shadow one
            // higher in the chain. crude but correct; in practice handler_set
            // is rare after boot, so the resulting cache warm-up is cheap.
            heap.send_cache.clear();
            Ok(receiver)
        });

        // Object: handlerAt: — read a handler value by selector.
        // Handlers are always Block-proto heap objects: user-defined
        // closures are bytecode + captures; native handlers use the
        // same shape with a `native_idx` slot. Either way the value
        // is callable, describable, and carries a stable identity —
        // so handlerAt: just returns it.
        native(heap, obj_id, "handlerAt:", |heap, receiver, args| {
            let sel = args.first().and_then(|v| v.as_symbol())
                .ok_or("handlerAt: arg must be a symbol")?;
            Ok(lookup_handler_by_sel(heap, receiver, sel))
        });

        // Object: responds: — moved to moof (types.moof)

        // Object: hasOwnHandler: — check if THIS object has the handler directly (no chain walk)
        native(heap, obj_id, "hasOwnHandler:", |heap, receiver, args| {
            let sel = args.first().and_then(|v| v.as_symbol()).ok_or("hasOwnHandler: arg must be a symbol")?;
            if let Some(id) = receiver.as_any_object() {
                Ok(Value::boolean(heap.get(id).handler_get(sel).is_some()))
            } else {
                Ok(Value::FALSE)
            }
        });

        // Object: clone — shallow copy
        native(heap, obj_id, "clone", |heap, receiver, _args| {
            if let Some(id) = receiver.as_any_object() {
                let cloned = heap.get(id).clone();
                Ok(heap.alloc_val(cloned))
            } else {
                Ok(receiver) // primitives are immutable, return self
            }
        });

        // Object: describe
        native(heap, obj_id, "describe", |heap, receiver, _args| {
            let s = heap.format_value(receiver);
            Ok(heap.alloc_string(&s))
        });

        // Object: type — returns a symbol for the type. Integer is
        // "Integer" for both primitive i48 and BigInt foreign backings
        // — users never see the bignum as a distinct type.
        native(heap, obj_id, "type", |heap, receiver, _args| {
            let name = if receiver.is_nil() { "Nil" }
                else if receiver.is_bool() { "Boolean" }
                else if heap.is_any_integer(receiver) { "Integer" }
                else if receiver.is_float() { "Float" }
                else if receiver.is_symbol() { "Symbol" }
                else if receiver.as_any_object().is_some() {
                    if let Some((_, is_op)) = heap.as_closure(receiver) {
                        if is_op { "Operative" } else { "Fn" }
                    } else if heap.is_pair(receiver) { "Cons" }
                    else if heap.is_text(receiver) { "String" }
                    else if heap.is_bytes(receiver) { "Bytes" }
                    else if heap.is_table(receiver) { "Table" }
                    else { "Object" }
                } else { "Unknown" };
            Ok(Value::symbol(heap.intern(name)))
        });

        // Object: identical: — bit-level identity test
        native(heap, obj_id, "identical:", |_heap, receiver, args| {
            let other = args.first().copied().unwrap_or(Value::NIL);
            Ok(Value::boolean(receiver == other))
        });

        // Object: equal: — content equality
        native(heap, obj_id, "equal:", |heap, receiver, args| {
            let other = args.first().copied().unwrap_or(Value::NIL);
            Ok(Value::boolean(heap.values_equal(receiver, other)))
        });

        // Object: print — outputs describe WITHOUT newline, returns self
        // (chainable for tap-style pipelines).
        native(heap, obj_id, "print", |heap, receiver, _args| {
            use std::io::Write;
            print!("{}", heap.format_value(receiver));
            let _ = std::io::stdout().flush();
            Ok(receiver)
        });

        // Object: println — outputs describe WITH newline, returns self
        // (chainable). Matches `[x print] [\"\\n\" print]` semantically
        // and keeps return-value uniformity.
        native(heap, obj_id, "println", |heap, receiver, _args| {
            println!("{}", heap.format_value(receiver));
            Ok(receiver)
        });

        // Object: show — default display for REPL (Showable protocol base)
        native(heap, obj_id, "show", |heap, receiver, _args| {
            let s = heap.format_value(receiver);
            Ok(heap.alloc_string(&s))
        });

        // -- Number prototype (shared parent for Integer and Float) --
        let number_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_NUMBER] = number_proto;

        // -- Symbol prototype --
        let sym_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_SYM] = sym_proto;
        let sym_proto_id = sym_proto.as_any_object().unwrap();

        // Symbol: name — the string name of the symbol
        native(heap, sym_proto_id, "name", |heap, receiver, _args| {
            let sym_id = receiver.as_symbol().ok_or("name: not a symbol")?;
            let name = heap.symbol_name(sym_id).to_string();
            Ok(heap.alloc_string(&name))
        });

        // Symbol: toString — alias for name
        let name_sym = heap.intern("name");
        let to_string_sym = heap.intern("toString");
        let name_handler = heap.get(sym_proto_id).handler_get(name_sym).unwrap();
        heap.get_mut(sym_proto_id).handler_set(to_string_sym, name_handler);

        // Symbol: describe
        native(heap, sym_proto_id, "describe", |heap, receiver, _args| {
            let sym_id = receiver.as_symbol().ok_or("describe: not a symbol")?;
            let name = heap.symbol_name(sym_id).to_string();
            Ok(heap.alloc_string(&name))
        });

        // Symbol: show
        native(heap, sym_proto_id, "show", |heap, receiver, _args| {
            let sym_id = receiver.as_symbol().ok_or("show: not a symbol")?;
            let name = heap.symbol_name(sym_id).to_string();
            Ok(heap.alloc_string(&format!("'{name}")))
        });

        // Symbol: hash — interned id (stable within a heap; content-
        // stable across heaps because equal symbols have equal names).
        // mix through fnv1a of the name to make cross-heap hashes agree.
        native(heap, sym_proto_id, "hash", |heap, receiver, _args| {
            let sym_id = receiver.as_symbol().ok_or("hash: not a symbol")?;
            let name = heap.symbol_name(sym_id);
            Ok(Value::integer(moof_core::fnv1a_64(name.as_bytes()) as i64))
        });

        // -- Nil prototype --
        let nil_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_NIL] = nil_proto;
        let nil_id = nil_proto.as_any_object().unwrap();

        native(heap, nil_id, "describe", |heap, _receiver, _args| {
            Ok(heap.alloc_string("nil"))
        });

        // Nil: ifTrue:ifFalse: — nil is falsy, always returns false branch
        native(heap, nil_id, "ifTrue:ifFalse:", |_heap, _receiver, args| {
            let false_val = args.get(1).copied().unwrap_or(Value::NIL);
            Ok(false_val)
        });

        // Nil: hash — FNV-1a over the NaN-box bits of Nil. the
        // type tag is in the high bits, so this hash is distinct
        // from Integer 0 / Float 0.0 / Boolean false.
        native(heap, nil_id, "hash", |_heap, receiver, _args| {
            let bits = receiver.to_bits().to_le_bytes();
            Ok(Value::integer(moof_core::fnv1a_64(&bits) as i64))
        });

        // -- Boolean prototype --
        let bool_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_BOOL] = bool_proto;
        let bool_id = bool_proto.as_any_object().unwrap();

        native(heap, bool_id, "not", |_heap, receiver, _args| {
            Ok(Value::boolean(!receiver.is_truthy()))
        });
        native(heap, bool_id, "describe", |heap, receiver, _args| {
            let s = if receiver.is_true() { "true" } else { "false" };
            Ok(heap.alloc_string(s))
        });
        native(heap, bool_id, "ifTrue:ifFalse:", |_heap, receiver, args| {
            let true_val = args.first().copied().unwrap_or(Value::NIL);
            let false_val = args.get(1).copied().unwrap_or(Value::NIL);
            Ok(if receiver.is_truthy() { true_val } else { false_val })
        });

        // Boolean: hash — FNV-1a over the NaN-box bits. distinct
        // from Integer 1 / Integer 2 / Nil / Float bits.
        native(heap, bool_id, "hash", |_heap, receiver, _args| {
            let bits = receiver.to_bits().to_le_bytes();
            Ok(Value::integer(moof_core::fnv1a_64(&bits) as i64))
        });

        // -- Register all prototypes as globals --
        let obj_sym = heap.intern("Object");
        heap.env_def(obj_sym, object_proto);
        let nil_s = heap.intern("Nil");
        heap.env_def(nil_s, nil_proto);
        let bool_s = heap.intern("Boolean");
        heap.env_def(bool_s, bool_proto);
        let symbol_s = heap.intern("Symbol");
        heap.env_def(symbol_s, sym_proto);
        let number_s = heap.intern("Number");
        heap.env_def(number_s, number_proto);

        // expose the root environment object as 'Env'
        let env_sym = heap.intern("Env");
        heap.env_def(env_sym, Value::nursery(heap.env));
    }
}

/// Look up a handler by selector on a value, walking the proto
/// chain. Returns the raw handler value (a symbol for native
/// handlers, a closure Value for moof-defined handlers, or NIL).
/// Factored out so handlerAt: doesn't need to inline the walk.
fn lookup_handler_by_sel(heap: &Heap, receiver: Value, sel: u32) -> Value {
    // instance-local handlers win first.
    if let Some(id) = receiver.as_any_object() {
        if let Some(h) = heap.get(id).handler_get(sel) {
            return h;
        }
    }
    // then the prototype chain.
    let proto = heap.prototype_of(receiver);
    let Some(pid) = proto.as_any_object() else { return Value::NIL; };
    let mut current = pid;
    for _ in 0..256 {
        if let Some(h) = heap.get(current).handler_get(sel) {
            return h;
        }
        match heap.get(current).proto().as_any_object() {
            Some(next) => current = next,
            None => break,
        }
    }
    Value::NIL
}

/// Entry point for dylib loading. moof-cli's manifest loader
/// calls this via `libloading` when a `[types]` entry points
/// at this crate's cdylib.
#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn moof_core::Plugin> {
    Box::new(CorePlugin)
}
