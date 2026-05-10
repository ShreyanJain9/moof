//! V3 here-form / env unification — end-to-end tests.

use moof::value::Value;

#[test]
fn def_macro_binds_via_here() {
    let mut w = moof::new_world();
    // (def x 42) should bind x in $here. verify via direct env_lookup.
    moof::eval(&mut w, "(def x 42)").unwrap();
    let x_sym = w.intern("x");
    assert_eq!(w.env_lookup(w.here_form, x_sym), Some(Value::Int(42)));
}

#[test]
fn def_returns_the_symbol() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "(def y 99)").unwrap();
    let y_sym = w.intern("y");
    assert_eq!(r, Value::Sym(y_sym));
}

// V3 Task 10 — set! now compiles to Send-based bytecode
// ([[Env current] set: 'name to: rhs]) instead of Op::StoreName.
// Two semantic changes flow from this:
//   (a) (set! x v) evaluates to v (per :set:to:'s return), not nil.
//   (b) (set! unbound v) raises 'unbound rather than silently
//       creating a global.

#[test]
fn set_walks_lexical_chain_via_env_current() {
    let mut w = moof::new_world();
    // bind foo in a let-frame, then set! it from inside.
    let r = moof::eval(&mut w, "(let ((foo 5)) (set! foo 99) foo)").unwrap();
    assert_eq!(r, Value::Int(99));
}

#[test]
fn set_raises_unbound_when_name_not_in_chain() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "(set! definitelyNotBound 0)");
    assert!(r.is_err());
    assert_eq!(w.resolve(r.unwrap_err().kind), "unbound");
}

// V3 Task 13 — peephole optimizer recognizes the if-macro's
// post-expansion shape `(__send__ (__send__ c '!!) 'ifTrue:ifFalse:
// (fn () t) (fn () e))` and emits Jump-based bytecode inline,
// recovering pre-V3 if-perf. Behavioral tests below — the peephole
// is internal, so we verify behavior, not the bytecode.

#[test]
fn if_macro_post_peephole_compiles_to_jump_based() {
    let mut w = moof::new_world();
    // matched shape: peephole fires; result must still be correct.
    let r2 = moof::eval(&mut w, "(if #true 1 2)").unwrap();
    assert_eq!(r2, Value::Int(1));
    let r3 = moof::eval(&mut w, "(if #false 1 2)").unwrap();
    assert_eq!(r3, Value::Int(2));
    // nested ifs — both branches go through the peephole.
    let r4 = moof::eval(&mut w, "(if #true (if #false 'a 'b) 'c)").unwrap();
    let b_sym = w.intern("b");
    assert_eq!(r4, Value::Sym(b_sym));
}

#[test]
fn if_with_non_syntactic_closure_args_uses_send_dispatch() {
    let mut w = moof::new_world();
    // user code that builds the closures explicitly via a `let`-
    // binding — peephole does NOT trigger (args aren't syntactic
    // (fn () body) literals at the call site). verifies the
    // user-overridable :ifTrue:ifFalse: dispatch path still works.
    let r = moof::eval(
        &mut w,
        "(let ((tThunk (fn () 'yes)) (eThunk (fn () 'no))) \
           [#true ifTrue: tThunk ifFalse: eThunk])",
    )
    .unwrap();
    let yes = w.intern("yes");
    assert_eq!(r, Value::Sym(yes));
}
