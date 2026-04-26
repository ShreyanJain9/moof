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

        // -- Env prototype (context-linked namespace) --
        //
        // Every env is an Object with two slots: `parent` (the outer
        // scope, or nil at the root) and `bindings` (a Table holding
        // name → value). Closures' captures are slots on the closure
        // itself, not in an env; closures still walk env.parent for
        // free-variable lookup. Env is the value-shape of "the
        // namespace tree" — what `addressing.md` calls a node walked
        // via `at:`.
        let env_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_ENV] = env_proto;
        let env_proto_id = env_proto.as_any_object().unwrap();

        // [env at: name] — binding lookup, walking the parent chain.
        // returns nil if not bound. lets `[Env at: 'foo]` work and
        // makes envs walkable via plain at: / walk:.
        native(heap, env_proto_id, "at:", |heap, receiver, args| {
            let env_id = receiver.as_any_object().ok_or("at:: not an env")?;
            let key = args.first().copied().ok_or("at:: needs a name")?;
            let sym = key.as_symbol()
                .or_else(|| {
                    key.as_any_object()
                        .and_then(|id| heap.get_string(id).map(|s| s.to_string()))
                        .and_then(|s| heap.find_symbol(&s))
                })
                .ok_or("at:: name must be a symbol or string")?;
            let mut cur = env_id;
            loop {
                let bind_sym = heap.find_symbol("bindings");
                let par_sym  = heap.find_symbol("parent");
                let bindings = bind_sym
                    .and_then(|s| heap.get(cur).slot_get(s))
                    .unwrap_or(Value::NIL);
                if let Some(bid) = bindings.as_any_object() {
                    if let Some(t) = heap.foreign_ref::<Table>(Value::nursery(bid)) {
                        if let Some(v) = t.map.get(&Value::symbol(sym)).copied() {
                            return Ok(v);
                        }
                    }
                }
                let parent = par_sym
                    .and_then(|s| heap.get(cur).slot_get(s))
                    .unwrap_or(Value::NIL);
                let Some(next) = parent.as_any_object() else { return Ok(Value::NIL); };
                cur = next;
            }
        });

        // [env has?: name] — true iff name is bound somewhere in the
        // scope chain. mirrors at: but returns Bool.
        native(heap, env_proto_id, "has?:", |heap, receiver, args| {
            let env_id = receiver.as_any_object().ok_or("has?:: not an env")?;
            let key = args.first().copied().ok_or("has?:: needs a name")?;
            let sym = key.as_symbol()
                .or_else(|| {
                    key.as_any_object()
                        .and_then(|id| heap.get_string(id).map(|s| s.to_string()))
                        .and_then(|s| heap.find_symbol(&s))
                })
                .ok_or("has?:: name must be a symbol or string")?;
            let mut cur = env_id;
            loop {
                let bind_sym = heap.find_symbol("bindings");
                let par_sym  = heap.find_symbol("parent");
                let bindings = bind_sym
                    .and_then(|s| heap.get(cur).slot_get(s))
                    .unwrap_or(Value::NIL);
                if let Some(bid) = bindings.as_any_object() {
                    if let Some(t) = heap.foreign_ref::<Table>(Value::nursery(bid)) {
                        if t.map.contains_key(&Value::symbol(sym)) {
                            return Ok(Value::TRUE);
                        }
                    }
                }
                let parent = par_sym
                    .and_then(|s| heap.get(cur).slot_get(s))
                    .unwrap_or(Value::NIL);
                let Some(next) = parent.as_any_object() else { return Ok(Value::FALSE); };
                cur = next;
            }
        });

        // [env names] — symbols bound directly in THIS env (not the
        // parent chain). use [env at:] to traverse parents.
        native(heap, env_proto_id, "names", |heap, receiver, _args| {
            let env_id = receiver.as_any_object().ok_or("names: not an env")?;
            let bind_sym = heap.find_symbol("bindings").ok_or("names: env has no bindings table")?;
            let bindings = heap.get(env_id).slot_get(bind_sym).unwrap_or(Value::NIL);
            let bid = bindings.as_any_object().ok_or("names: bindings slot empty")?;
            let t = heap.foreign_ref::<Table>(Value::nursery(bid))
                .ok_or("names: bindings is not a Table")?;
            let keys: Vec<Value> = t.map.keys().copied().collect();
            Ok(heap.list(&keys))
        });

        // [env count] — number of locally-bound names.
        native(heap, env_proto_id, "count", |heap, receiver, _args| {
            let env_id = receiver.as_any_object().ok_or("count: not an env")?;
            let bind_sym = heap.find_symbol("bindings").ok_or("count: env has no bindings")?;
            let bindings = heap.get(env_id).slot_get(bind_sym).unwrap_or(Value::NIL);
            let bid = bindings.as_any_object().ok_or("count: bindings empty")?;
            let t = heap.foreign_ref::<Table>(Value::nursery(bid))
                .ok_or("count: bindings is not a Table")?;
            Ok(Value::integer(t.map.len() as i64))
        });

        // [env typeName] — 'Env
        let env_typename_sym = heap.intern("Env");
        native(heap, env_proto_id, "typeName", move |_heap, _r, _a| {
            Ok(Value::symbol(env_typename_sym))
        });

        // [env describe] — short shape summary
        native(heap, env_proto_id, "describe", |heap, receiver, _args| {
            let env_id = receiver.as_any_object().ok_or("describe: not an env")?;
            let bind_sym = heap.find_symbol("bindings");
            let count = bind_sym
                .and_then(|s| heap.get(env_id).slot_get(s))
                .and_then(|b| b.as_any_object())
                .and_then(|bid| heap.foreign_ref::<Table>(Value::nursery(bid)))
                .map(|t| t.map.len())
                .unwrap_or(0);
            Ok(heap.alloc_string(&format!("<Env {count} bindings>")))
        });

        // [Env new] — construct a fresh empty Env with no parent.
        // optional argument: the parent env (so [Env new: outer]
        // builds an env whose `at:` falls through to outer).
        // bindings live in a fresh Table, mutable like the root env's.
        native(heap, env_proto_id, "new", |heap, _r, _args| {
            let bindings = heap.alloc_empty_table();
            let par_sym = heap.intern("parent");
            let bind_sym = heap.intern("bindings");
            let proto = heap.type_protos[PROTO_ENV];
            Ok(heap.make_object_with_slots(
                proto,
                vec![par_sym, bind_sym],
                vec![Value::NIL, bindings],
            ))
        });
        native(heap, env_proto_id, "new:", |heap, _r, args| {
            let parent = args.first().copied().unwrap_or(Value::NIL);
            let bindings = heap.alloc_empty_table();
            let par_sym = heap.intern("parent");
            let bind_sym = heap.intern("bindings");
            let proto = heap.type_protos[PROTO_ENV];
            Ok(heap.make_object_with_slots(
                proto,
                vec![par_sym, bind_sym],
                vec![parent, bindings],
            ))
        });

        // [env bind: name to: val] — set a binding in THIS env's
        // bindings table. mutates the env (matching env_def
        // semantics). returns self so chains work.
        native(heap, env_proto_id, "bind:to:", |heap, receiver, args| {
            let key = args.first().copied().ok_or("bind:to:: needs a name")?;
            let val = args.get(1).copied().unwrap_or(Value::NIL);
            let sym = key.as_symbol().ok_or("bind:to:: name must be a symbol")?;
            if !heap.bind_in_env(receiver, sym, val) {
                return Err("bind:to:: not an env (no bindings table)".into());
            }
            Ok(receiver)
        });

        // [env union: other] — copy every binding from other's
        // bindings table (just its own, not its parents) into self.
        // mutates self; returns self. last write wins on collision.
        native(heap, env_proto_id, "union:", |heap, receiver, args| {
            let other = args.first().copied().ok_or("union:: needs an env")?;
            let other_id = other.as_any_object().ok_or("union:: arg not an env")?;
            let bind_sym = heap.intern("bindings");
            let other_bindings = heap.get(other_id).slot_get(bind_sym).unwrap_or(Value::NIL);
            let other_bid = other_bindings.as_any_object()
                .ok_or("union:: other has no bindings")?;
            // collect (name, value) pairs from other
            let pairs: Vec<(u32, Value)> = {
                let t = heap.foreign_ref::<Table>(Value::nursery(other_bid))
                    .ok_or("union:: other.bindings not a Table")?;
                t.map.iter()
                    .filter_map(|(k, v)| k.as_symbol().map(|s| (s, *v)))
                    .collect()
            };
            for (sym, val) in pairs {
                heap.bind_in_env(receiver, sym, val);
            }
            Ok(receiver)
        });

        // [env walk: path] — plan-9 walk over the env tree. handles
        // leading slash; nil on miss. uses dispatch::lookup_handler +
        // call_native to send `at:` at each step, so any proto with
        // an `at:` handler participates.
        // can't use defmethod from moof because `Env` is bound to
        // the global env singleton, not the proto.
        native(heap, env_proto_id, "walk:", |heap, receiver, args| {
            let path_val = args.first().copied().ok_or("walk:: needs a path")?;
            let path_id = path_val.as_any_object().ok_or("walk:: path must be a string")?;
            let path = heap.get_string(path_id).ok_or("walk:: path must be a string")?.to_string();
            let stripped = path.strip_prefix('/').unwrap_or(&path);
            let at_sym = heap.sym_at;
            let mut cur = receiver;
            for seg in stripped.split('/') {
                if seg.is_empty() { continue; }
                let sym = heap.intern(seg);
                // dispatch [cur at: sym]
                let (handler, _) = moof_core::dispatch::lookup_handler(heap, cur, at_sym)?;
                if handler.is_nil() { return Ok(Value::NIL); }
                if !moof_core::dispatch::is_native(heap, handler) {
                    return Ok(Value::NIL);  // would need full VM dispatch for moof handlers
                }
                let result = moof_core::dispatch::call_native(
                    heap, handler, cur, &[Value::symbol(sym)],
                ).unwrap_or(Value::NIL);
                if result.is_nil() { return Ok(Value::NIL); }
                cur = result;
            }
            Ok(cur)
        });

        // root env is now an Env-shaped value. fix its proto.
        heap.get_mut(heap.env).set_proto(env_proto);

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

        // bind `Env` to the env PROTOTYPE (the Type), not to the
        // vat's root scope value. matches Object/Cons/Set/etc. —
        // the name refers to the type, defmethod Env x works.
        // there's no user-facing name for the vat's root scope;
        // top-level defs land in it implicitly via the runtime,
        // and Bundle.apply (no arg) targets it.
        let env_sym = heap.intern("Env");
        heap.env_def(env_sym, env_proto);
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
