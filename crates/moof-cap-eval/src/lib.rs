// Eval capability — parse moof source into AST forms.
//
// The capability has no authority of its own; its handler is a pure
// parser that turns a source string into a list of top-level AST
// forms. Those forms cross back to the caller as cons lists + symbols
// + literals (cross-vat-copyable moof values), and the caller evaluates
// each with the built-in `(eval form)` operative (an Op::Eval in the
// caller's vat).
//
// This keeps evaluation in the caller's heap — exactly where defs
// should land — while moving the impure parsing step into a
// capability. Canonical usage:
//
//   (defn reload (path)
//     (do (src   <- [file read: path])
//         (forms <- [evaluator parse: src])
//         (each forms |f| (eval f))))

use moof_core::{Heap, Value, native};
use moof_runtime::{CapabilityPlugin, Vat};

pub struct EvalCapability;

impl CapabilityPlugin for EvalCapability {
    fn name(&self) -> &str { "eval" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj = vat.heap.make_object(Value::NIL).as_any_object().unwrap();
        let heap = &mut vat.heap;

        // [evaluator parse: source-string]
        //   → list of top-level AST forms (cons cells, symbols, literals)
        //
        // Forms cross back to the caller vat by the scheduler's
        // standard value-copy path. The caller evaluates each form in
        // its own vat via `(eval form)`, keeping defs in the right heap.
        native(heap, obj, "parse:", parse_handler);

        // [evaluator describe] — human-readable summary
        native(heap, obj, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Evaluator>"))
        });

        let type_sym = heap.intern("Evaluator");
        native(heap, obj, "typeName", move |_, _, _| Ok(Value::symbol(type_sym)));

        obj
    }
}

/// Parse a source string into a list of top-level AST forms.
/// Returns the list as a cons-chain in the cap's heap; the scheduler
/// copies it across to the caller on reply.
fn parse_handler(heap: &mut Heap, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let src_val = args.first().copied().ok_or("parse: need a source string")?;
    let src_id = src_val.as_any_object().ok_or("parse: arg must be a String")?;
    let source = heap.get_string(src_id).ok_or("parse: arg must be a String")?.to_string();

    let tokens = moof_lang::lang::lexer::tokenize(&source)
        .map_err(|e| format!("parse: lex: {e}"))?;
    let mut parser = moof_lang::lang::parser::Parser::new(&tokens, heap);
    let exprs = parser.parse_all()
        .map_err(|e| format!("parse: {e}"))?;
    Ok(heap.list(&exprs))
}

#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(EvalCapability)
}
