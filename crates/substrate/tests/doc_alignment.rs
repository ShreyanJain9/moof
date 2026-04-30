//! doc-alignment regression suite.
//!
//! these tests lock in surfaces the docs in `docs/concepts/` and
//! `docs/laws/` promise but the impl had silently lacked. each test
//! cites the doc paragraph it pins down.
//!
//! anything that would be a "the docs are aspirational here" caveat
//! should ideally fail until either the impl catches up or the docs
//! retract. for now, this file holds the *kept* promises.

use moof::value::Value;

// ─────────────────────────────────────────────────────────────────
// laws/reflection-contract.md R1 — every Form responds to the basic
// reflection protocol. R7 — meta is extensible (so it must be a
// Table, not a list-of-pairs).
// ─────────────────────────────────────────────────────────────────

#[test]
fn protos_returns_full_chain() {
    // R1: `[v protos]` returns the full delegation chain.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[5 protos]").unwrap();
    // [5 protos] → (Integer Object). Integer's proto is Object;
    // Object's proto is nil so the chain stops there.
    let chain = w.list_to_vec(v).unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0], Value::Form(w.protos.integer));
    assert_eq!(chain[1], Value::Form(w.protos.object));
}

#[test]
fn protos_terminates_at_object() {
    // Object's :protos is the empty list — its proto is nil.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[Object protos]").unwrap();
    assert_eq!(v, Value::Nil);
}

#[test]
fn slots_returns_a_table() {
    // forms.md, reflection-contract.md R7: :slots is a Table
    // keyed by slot-name. mutating the returned Table doesn't
    // affect the receiver (snapshot semantics in phase A; live
    // views are phase C).
    let mut w = moof::new_world();
    moof::eval(&mut w, "(defproto Counter (slots count step))").unwrap();
    let _ = moof::eval(&mut w, "(def c [Counter new])").unwrap();
    moof::eval(&mut w, "(slotSet! c 'count 5)").unwrap();
    moof::eval(&mut w, "(slotSet! c 'step 1)").unwrap();
    let v = moof::eval(&mut w, "[c slots]").unwrap();
    // it's a Table; sending :at: works.
    assert_eq!(
        moof::eval(&mut w, "[[c slots] at: 'count]").unwrap(),
        Value::Int(5),
    );
    assert_eq!(
        moof::eval(&mut w, "[[c slots] at: 'step]").unwrap(),
        Value::Int(1),
    );
    // it's a Table-proto Form.
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
}

#[test]
fn handlers_returns_a_table() {
    // reflection-contract.md R1: `[proto handlers]` is a Table.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(defproto Greeter (handlers (greet) 'hello))",
    )
    .unwrap();
    let v = moof::eval(&mut w, "[Greeter handlers]").unwrap();
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
    // the :greet handler is in there.
    let greet_present =
        moof::eval(&mut w, "[[Greeter handlers] containsKey?: 'greet]").unwrap();
    assert_eq!(greet_present, Value::Bool(true));
}

// ─────────────────────────────────────────────────────────────────
// concepts/lists.md — `[xs at: i]`, structural :=, :count, :zip:.
// ─────────────────────────────────────────────────────────────────

#[test]
fn list_indexing_is_o_n_but_works() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 'a 'b 'c) at: 0]").unwrap(),
        Value::Sym(w.intern("a")),
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 'a 'b 'c) at: 2]").unwrap(),
        Value::Sym(w.intern("c")),
    );
}

#[test]
fn list_equality_is_structural() {
    // distinct cons-cells with the same content compare equal.
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) = (list 1 2 3)]").unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) = (list 1 2 4)]").unwrap(),
        Value::Bool(false),
    );
    // empty matches empty.
    assert_eq!(
        moof::eval(&mut w, "[nil = nil]").unwrap(),
        Value::Bool(true),
    );
    // length-mismatched lists are unequal.
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2) = (list 1 2 3)]").unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) = (list 1 2)]").unwrap(),
        Value::Bool(false),
    );
}

#[test]
fn list_count_and_zip() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 'a 'b 'c) count]").unwrap(),
        Value::Int(3),
    );
    // zip stops at the shorter list.
    let v = moof::eval(
        &mut w,
        "[(list 1 2 3) zip: (list 'a 'b)]",
    )
    .unwrap();
    let pairs = w.list_to_vec(v).unwrap();
    assert_eq!(pairs.len(), 2);
}

#[test]
fn list_scan_is_running_fold() {
    let mut w = moof::new_world();
    let v = moof::eval(
        &mut w,
        "[(list 1 2 3 4) scan: (fn (a b) [a + b]) from: 0]",
    )
    .unwrap();
    let xs = w.list_to_vec(v).unwrap();
    assert_eq!(xs, vec![Value::Int(1), Value::Int(3), Value::Int(6), Value::Int(10)]);
}

// ─────────────────────────────────────────────────────────────────
// concepts/numbers.md — :negate, :integer?, :rational?, :real?,
// :mod:, :clamp:to:.
// ─────────────────────────────────────────────────────────────────

#[test]
fn integer_tower_predicates() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[5 integer?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[5 rational?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[5 real?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[5 nan?]").unwrap(), Value::Bool(false));
    assert_eq!(moof::eval(&mut w, "[5 finite?]").unwrap(), Value::Bool(true));
}

#[test]
fn float_tower_predicates() {
    let mut w = moof::new_world();
    // a Float is real but not integer or rational.
    assert_eq!(moof::eval(&mut w, "[1.5 integer?]").unwrap(), Value::Bool(false));
    assert_eq!(moof::eval(&mut w, "[1.5 rational?]").unwrap(), Value::Bool(false));
    assert_eq!(moof::eval(&mut w, "[1.5 real?]").unwrap(), Value::Bool(true));
}

#[test]
fn negate_works_on_both_kinds() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[5 negate]").unwrap(), Value::Int(-5));
    let v = moof::eval(&mut w, "[1.5 negate]").unwrap();
    assert_eq!(v.as_float().unwrap(), -1.5);
}

#[test]
fn integer_mod_and_clamp() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[7 mod: 2]").unwrap(), Value::Int(1));
    assert_eq!(moof::eval(&mut w, "[10 mod: 3]").unwrap(), Value::Int(1));
    // clamp:to: returns lo if below, hi if above, self otherwise.
    assert_eq!(moof::eval(&mut w, "[5 clamp: 0 to: 10]").unwrap(), Value::Int(5));
    assert_eq!(moof::eval(&mut w, "[-3 clamp: 0 to: 10]").unwrap(), Value::Int(0));
    assert_eq!(moof::eval(&mut w, "[42 clamp: 0 to: 10]").unwrap(), Value::Int(10));
}

// ─────────────────────────────────────────────────────────────────
// laws/reflection-contract.md R2 — methods expose :arity, :purity,
// :caps-required, :parameters.
// ─────────────────────────────────────────────────────────────────

#[test]
fn method_reflection_arity() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def f (fn (a b c) [a + b]))").unwrap();
    assert_eq!(moof::eval(&mut w, "[f arity]").unwrap(), Value::Int(3));
    moof::eval(&mut w, "(def g (fn () 42))").unwrap();
    assert_eq!(moof::eval(&mut w, "[g arity]").unwrap(), Value::Int(0));
}

#[test]
fn method_reflection_purity_default() {
    // until the analyzer lands (phase C), :purity defaults to
    // 'unknown — but the contract is honored.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def f (fn () 42))").unwrap();
    assert_eq!(
        moof::eval(&mut w, "[f purity]").unwrap(),
        Value::Sym(w.intern("unknown")),
    );
    // :caps-required defaults to nil (empty list).
    assert_eq!(
        moof::eval(&mut w, "[f caps-required]").unwrap(),
        Value::Nil,
    );
}

// ─────────────────────────────────────────────────────────────────
// concepts/tables.md — :reduce: (no init), :at-or:default:.
// ─────────────────────────────────────────────────────────────────

#[test]
fn table_reduce_no_init_uses_first() {
    let mut w = moof::new_world();
    let v = moof::eval(
        &mut w,
        "[#[1 2 3 4] reduce: (fn (a b) [a + b])]",
    )
    .unwrap();
    assert_eq!(v, Value::Int(10));
    // empty table → nil.
    assert_eq!(
        moof::eval(&mut w, "[#[] reduce: (fn (a b) [a + b])]").unwrap(),
        Value::Nil,
    );
}

#[test]
fn table_at_or_default() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def t #[1 2 3 'name => 'ada])").unwrap();
    assert_eq!(
        moof::eval(&mut w, "[t at-or: 'name default: 'unknown]").unwrap(),
        Value::Sym(w.intern("ada")),
    );
    assert_eq!(
        moof::eval(&mut w, "[t at-or: 'missing default: 'fallback]").unwrap(),
        Value::Sym(w.intern("fallback")),
    );
}

// ─────────────────────────────────────────────────────────────────
// substrate-laws.md L3 dispatch + the proto-receiver edge case.
// ─────────────────────────────────────────────────────────────────

#[test]
fn equality_on_proto_forms_does_not_panic() {
    // sending `=` to a proto-Form (e.g. Integer itself, not an
    // Integer instance) used to panic because Integer's `=` was
    // assuming `self.as_int()` succeeded. now it falls through to
    // identity.
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[Integer = Integer]").unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        moof::eval(&mut w, "[Integer = String]").unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        moof::eval(&mut w, "[Integer = nil]").unwrap(),
        Value::Bool(false),
    );
}

// ─────────────────────────────────────────────────────────────────
// vm.rs PushClosure — captured-self regression.
// ─────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────
// "everything is a Form" (forms.md L1) — control-flow sugar like
// `when` / `unless` / `let*` / `let-rec` / `defmethod` lives as
// moof macros (lib/bootstrap.moof), not as hardcoded special-cases
// in the rust compiler. that means: the user can inspect them,
// re-define them, and override their expansion from inside moof.
// ─────────────────────────────────────────────────────────────────

#[test]
fn when_unless_letstar_letrec_are_user_inspectable_macros() {
    let mut w = moof::new_world();
    // each is a real Method-Form bound in the global env, with
    // `'macro` set in its meta.
    for name in &["when", "unless", "let*", "let-rec", "defmethod"] {
        let sym = w.intern(name);
        let v = w.env_lookup(w.global_env, sym).unwrap_or_else(|| {
            panic!("expected macro `{}` bound in global env", name)
        });
        let id = v
            .as_form_id()
            .unwrap_or_else(|| panic!("`{}` is not a Form", name));
        // the macro is registered on the world's macro table.
        assert!(
            w.macros.contains_key(&sym),
            "`{}` must be registered as a macro",
            name
        );
        // it's a Method-Form (closure).
        let proto = w.heap.get(id).proto;
        assert_eq!(proto, Value::Form(w.protos.method));
    }
}

#[test]
fn user_can_redefine_when() {
    // because `when` is a moof macro, redefining it from user code
    // works. (this is the key moldability promise — "everything
    // modifiable lives in moof".)
    let mut w = moof::new_world();
    // re-define `when` to ALWAYS evaluate body, ignoring the cond.
    moof::eval(
        &mut w,
        "(defmacro when (args)
           (let ((body [args tail]))
             `(do ,@body)))",
    )
    .unwrap();
    // now `(when #false 42)` evaluates the body anyway.
    assert_eq!(
        moof::eval(&mut w, "(when #false 42)").unwrap(),
        Value::Int(42),
    );
}

#[test]
fn macroexpand_uses_single_list_convention() {
    // (macroexpand 'form) returns the expansion. matches the new
    // calling convention: macros take one arg = the list of
    // source-arg-Forms.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(macroexpand '(when c a b))").unwrap();
    // expansion: (if c (do a b) nil)
    let elems = w.list_to_vec(v).unwrap();
    assert_eq!(elems.len(), 4);
    assert_eq!(elems[0], Value::Sym(w.intern("if")));
    assert_eq!(elems[1], Value::Sym(w.intern("c")));
    // elems[2] is (do a b)
    let then_branch = w.list_to_vec(elems[2]).unwrap();
    assert_eq!(then_branch[0], Value::Sym(w.intern("do")));
    assert_eq!(elems[3], Value::Nil);
}

#[test]
fn defmethod_via_macro_installs_handler() {
    let mut w = moof::new_world();
    // single-symbol header.
    moof::eval(
        &mut w,
        "(defmethod Integer (squared) [self * self])",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "[5 squared]").unwrap(), Value::Int(25));

    // binary-operator header.
    moof::eval(
        &mut w,
        "(defmethod Integer (~ other) [self + [other * 100]])",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[3 ~ 5]").unwrap(),
        Value::Int(503),
    );

    // keyword header — the single-keyword case.
    moof::eval(
        &mut w,
        "(defmethod Integer (multipliedBy: n) [self * n])",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[7 multipliedBy: 3]").unwrap(),
        Value::Int(21),
    );

    // keyword header — multi-keyword needs intern at expand time.
    moof::eval(
        &mut w,
        "(defmethod Integer (between?: lo and: hi)
           (if [self < lo] #false [self <= hi]))",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[5 between?: 0 and: 10]").unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        moof::eval(&mut w, "[15 between?: 0 and: 10]").unwrap(),
        Value::Bool(false),
    );
}

// ─────────────────────────────────────────────────────────────────
// the `(intern …)` primitive — required by macros that build
// keyword selectors at expansion time.
// ─────────────────────────────────────────────────────────────────

#[test]
fn intern_constructs_symbols_from_strings() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(intern \"foo\")").unwrap();
    assert_eq!(v, Value::Sym(w.intern("foo")));
    // idempotent on Symbol input.
    let v = moof::eval(&mut w, "(intern 'bar)").unwrap();
    assert_eq!(v, Value::Sym(w.intern("bar")));
    // joining two symbols' text and re-interning.
    let v = moof::eval(
        &mut w,
        "(intern [[(intern \"at:\") toString] + [(intern \"put:\") toString]])",
    )
    .unwrap();
    assert_eq!(v, Value::Sym(w.intern("at:put:")));
}

#[test]
fn let_inside_method_sees_self() {
    // `let` desugars to a closure-call. without PushClosure
    // capturing self_, the closure ran with self = nil. fix in
    // place; method bodies that wrap state in `(let ...)` still
    // see the receiver.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(setHandler! Integer 'doubled-via-let
           (fn ()
             (let ((n self)) [n * 2])))",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[5 doubled-via-let]").unwrap(),
        Value::Int(10),
    );
}
