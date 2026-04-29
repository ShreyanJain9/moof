//! root proto installation.
//!
//! at world boot, allocate the pre-defined type-protos and the
//! global env. each proto is a Form chained up through `Object`.
//! `Object`'s proto is `FormId::NONE`.
//!
//! later phases:
//! - move method bodies to bytecode (compiled from moof source).
//! - add `defproto` operative so user code can extend.
//! - the inspector / type system / analyzer query these protos.

use crate::form::{Form, FormId, MethodImpl};
use crate::value::Value;
use crate::world::World;

pub fn install(w: &mut World) {
    // ── Object ── the root. proto: NONE. (substrate-laws.md L2.)
    w.object = w.heap.alloc(Form::with_proto(FormId::NONE));

    // ── primitive type-protos. all chain up to Object.
    w.nil_proto = w.heap.alloc(Form::with_proto(w.object));
    w.bool_proto = w.heap.alloc(Form::with_proto(w.object));
    w.integer_proto = w.heap.alloc(Form::with_proto(w.object));
    w.symbol_proto = w.heap.alloc(Form::with_proto(w.object));
    w.list_proto = w.heap.alloc(Form::with_proto(w.object));
    w.builtin_proto = w.heap.alloc(Form::with_proto(w.object));
    w.closure_proto = w.heap.alloc(Form::with_proto(w.object));
    w.env_proto = w.heap.alloc(Form::with_proto(w.object));
    w.string_proto = w.heap.alloc(Form::with_proto(w.object));
    w.send_form_proto = w.heap.alloc(Form::with_proto(w.object));

    // ── allocate the global env. its parent is Nil (root of chain).
    w.global_env = w.alloc_env(Value::Nil);

    // ── install native methods on Integer.
    install_integer_methods(w);
    install_string_methods(w);

    // ── install global callables: arithmetic, comparison, list ops,
    // and the println stub. these are the substrate-provided
    // primitives — anything user code defines later via `def` and
    // `fn` lives above this line in moof.
    install_global_callables(w);
}

fn install_integer_methods(w: &mut World) {
    use crate::builtins::*;
    let p = w.integer_proto;

    // selectors are bare symbols. trailing `:` only marks keyword-arg
    // positions in `[...]` sends (concepts/sends-and-calls.md); there
    // is no leading `:`. so Integer's `+` method is keyed on the symbol
    // `+`, not `:+`.
    let plus = w.syms.intern("+");
    let minus = w.syms.intern("-");
    let times = w.syms.intern("*");
    let div = w.syms.intern("/");
    let eq = w.syms.intern("=");
    let lt = w.syms.intern("<");
    let gt = w.syms.intern(">");
    let le = w.syms.intern("<=");
    let ge = w.syms.intern(">=");

    let h = &mut w.heap.get_mut(p).handlers;
    h.insert(plus, MethodImpl::Native(int_plus));
    h.insert(minus, MethodImpl::Native(int_minus));
    h.insert(times, MethodImpl::Native(int_times));
    h.insert(div, MethodImpl::Native(int_div));
    h.insert(eq, MethodImpl::Native(int_eq));
    h.insert(lt, MethodImpl::Native(int_lt));
    h.insert(gt, MethodImpl::Native(int_gt));
    h.insert(le, MethodImpl::Native(int_le));
    h.insert(ge, MethodImpl::Native(int_ge));
}

fn install_string_methods(w: &mut World) {
    use crate::builtins::*;
    let p = w.string_proto;
    let length = w.syms.intern("length");
    let byte_length = w.syms.intern("byte-length");
    let concat = w.syms.intern("++");
    let eq = w.syms.intern("=");
    let h = &mut w.heap.get_mut(p).handlers;
    h.insert(length, MethodImpl::Native(str_length));
    h.insert(byte_length, MethodImpl::Native(str_byte_length));
    h.insert(concat, MethodImpl::Native(str_concat));
    h.insert(eq, MethodImpl::Native(str_eq));
}

fn install_global_callables(w: &mut World) {
    use crate::builtins::*;

    let entries: &[(&str, crate::form::NativeFn)] = &[
        // arithmetic — forward via send to receiver's :OP method
        ("+", fn_plus),
        ("-", fn_minus),
        ("*", fn_times),
        ("/", fn_div),
        // comparison
        ("=", fn_eq),
        ("<", fn_lt),
        (">", fn_gt),
        ("<=", fn_le),
        (">=", fn_ge),
        // list ops — Lists are heap Forms; head/tail/cons live in rust
        ("cons", fn_cons),
        ("head", fn_head),
        ("tail", fn_tail),
        ("null?", fn_null_q),
        ("list?", fn_list_q),
        ("list", fn_list),
        // booleans
        ("not", fn_not),
        // identity
        ("identity", fn_identity),
        // io (stub for $out cap; concepts/capabilities.md proper later)
        ("println", fn_println),
        ("print", fn_print),
        // slot access
        ("slot", fn_slot),
        ("slot-set!", fn_slot_set),
        ("has-slot?", fn_has_slot_q),
        // proto / object construction (used by defproto, plus user code)
        ("make-proto", fn_make_proto),
        ("proto-set-handler!", fn_proto_set_handler),
        ("set-default-slot!", fn_set_default_slot),
        ("new", fn_new),
        // strings
        ("str", fn_str),
        ("show", fn_show),
        // REPL primitives (read / parse / eval — the loop itself lives
        // in lib/bootstrap.moof)
        ("read-line", fn_read_line),
        ("parse", fn_parse),
        ("parse-all", fn_parse_all),
        ("eval", fn_eval),
        ("try-eval", fn_try_eval),
        // reflection
        ("type-of", fn_type_of),
    ];

    for (name, native) in entries {
        let sym = w.syms.intern(name);
        let cb = w.alloc_native_callable(*native);
        w.define_global(sym, Value::Form(cb));
    }
}
