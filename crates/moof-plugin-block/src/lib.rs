use moof_core::{Plugin, native};
use moof_core::heap::*;
use moof_core::value::Value;

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn name(&self) -> &str { "block" }

    fn register(&self, heap: &mut Heap) {
        let object_proto = heap.type_protos[PROTO_OBJ];

        // -- Block/Closure prototype (type_protos[PROTO_CLOSURE]) --
        let block_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_CLOSURE] = block_proto;
        let block_id = block_proto.as_any_object().unwrap();

        // Block: arity — return the arity of the closure
        native(heap, block_id, "arity", |heap, receiver, _args| {
            heap.closure_arity(receiver)
                .map(|n| Value::integer(n as i64))
                .ok_or_else(|| "arity: not a closure".into())
        });

        // Block: pure? — true if no FarRef captures (safe to memoize/parallelize)
        native(heap, block_id, "pure?", |heap, receiver, _args| {
            if heap.as_closure(receiver).is_none() {
                return Err("pure?: not a closure".into());
            }
            Ok(Value::boolean(heap.closure_is_pure(receiver)))
        });

        // Block: operative? — structural: true iff this closure has no
        // `__underlying` slot (i.e. it's not wrapping a target).
        // as_closure returns (code_idx, is_operative) where is_operative
        // is derived as `!has_underlying`.
        native(heap, block_id, "operative?", |heap, receiver, _args| {
            let (_, is_operative) = heap.as_closure(receiver).ok_or("operative?: not a closure")?;
            Ok(Value::boolean(is_operative))
        });

        // Block: applicative? — true iff this closure has __underlying
        // (it's a wrap output around some other combiner).
        native(heap, block_id, "applicative?", |heap, receiver, _args| {
            let (_, is_operative) = heap.as_closure(receiver).ok_or("applicative?: not a closure")?;
            Ok(Value::boolean(!is_operative))
        });

        // Block: __make-applicative — Kernel's wrap, structural form.
        //   target.__make-applicative => closure with __underlying = target
        // shares target's bytecode/captures/arity, just adds the marker
        // slot. compile-time call sites query has-__underlying to decide
        // whether to eval args before passing. moof-level (def wrap …)
        // in the prelude is the user-facing form; this is the one-line
        // primitive that does the slot construction.
        //
        // wrap-of-wrap stacks: the new wrap-output's __underlying points
        // at the previous wrap-output, and unwrap walks the chain.
        native(heap, block_id, "__make-applicative", |heap, receiver, _args| {
            let (code_idx, _) = heap.as_closure(receiver).ok_or("__make-applicative: not a closure")?;
            let arity = heap.closure_arity(receiver).unwrap_or(0);
            let captures = heap.closure_captures(receiver);
            let new_closure = heap.make_closure(code_idx, arity, &captures);
            heap.set_closure_underlying(new_closure, receiver);
            Ok(new_closure)
        });

        // Block: unwrap — return the target this applicative wraps,
        // or self if it's already an operative. inverse of wrap.
        native(heap, block_id, "unwrap", |heap, receiver, _args| {
            let underlying = heap.closure_underlying(receiver);
            if underlying.is_nil() { Ok(receiver) } else { Ok(underlying) }
        });

        // Block: __source-text — low-level accessor for the raw source
        // text of the form that compiled this closure. Returns the
        // text recorded at parse time, or nil if none.
        //
        // The user-facing accessor is [v source], which returns the
        // canonical authored form (a structured value with `text`
        // and `form` slots). definers populate that during
        // installation; this primitive is what they call to get the
        // text portion. see lib/kernel/bootstrap.moof's defmethod.
        native(heap, block_id, "__source-text", |heap, receiver, _args| {
            let Some((code_idx, _)) = heap.as_closure(receiver) else {
                return Ok(Value::NIL);
            };
            match heap.closure_source(code_idx) {
                Some(src) => {
                    let text = src.text.clone();
                    Ok(heap.alloc_string(&text))
                }
                None => Ok(Value::NIL),
            }
        });

        // Block: origin — provenance record as a plain object:
        // { label: <string> byte-start: <int> byte-end: <int> } or nil.
        native(heap, block_id, "origin", |heap, receiver, _args| {
            let (code_idx, _) = heap.as_closure(receiver).ok_or("origin: not a closure")?;
            let Some(src) = heap.closure_source(code_idx) else { return Ok(Value::NIL); };
            let label = src.origin.label.clone();
            let byte_start = src.origin.byte_start as i64;
            let byte_end = src.origin.byte_end as i64;

            let label_val = heap.alloc_string(&label);
            let label_sym = heap.intern("label");
            let start_sym = heap.intern("byte-start");
            let end_sym = heap.intern("byte-end");
            // inherit from Object so slotAt: / dot-access works naturally.
            let object_proto = heap.type_protos[PROTO_OBJ];
            Ok(heap.make_object_with_slots(
                object_proto,
                vec![label_sym, start_sym, end_sym],
                vec![label_val, Value::integer(byte_start), Value::integer(byte_end)],
            ))
        });

        // Block: describe — human-readable description
        native(heap, block_id, "describe", |heap, receiver, _args| {
            let s = heap.format_value(receiver);
            Ok(heap.alloc_string(&s))
        });

        // register Block as global
        let block_sym = heap.intern("Block");
        heap.env_def(block_sym, block_proto);

        // (Env binding is owned by plugin-core, which binds it to
        // the env PROTOTYPE — not to the vat's root scope value.
        // there's no user-facing name for the singleton anymore;
        // top-level defs land in the runtime's current scope
        // implicitly, and Bundle.apply targets it without a name.)
    }
}

/// Entry point for dylib loading. moof-cli's manifest loader
/// calls this via `libloading` when a `[types]` entry points
/// at this crate's cdylib.
#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn moof_core::Plugin> {
    Box::new(BlockPlugin)
}
