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

// V3 Task 14 — Object :eval: implements Ruby instance_eval semantics
// in pure moof on top of `:callIn:withSelf:` (Task 6) and the
// `view-target` meta key (Tasks 2-3).

#[test]
fn obj_eval_lookups_find_obj_slots() {
    let mut w = moof::new_world();
    // bind a slot on obj, then [obj eval: (fn () foo)] should find it
    // via the view-target hop into obj's slots.
    moof::eval(&mut w, "(def obj [Object new])").unwrap();
    moof::eval(&mut w, "(slotSet! obj 'foo 42)").unwrap();
    let r = moof::eval(&mut w, "[obj eval: (fn () foo)]").unwrap();
    assert_eq!(r, Value::Int(42));
}

#[test]
fn obj_eval_lookups_also_find_closure_captured_names() {
    let mut w = moof::new_world();
    // captured-env names still resolve — view-target augments, not
    // replaces, the lexical chain.
    moof::eval(&mut w, "(def obj [Object new])").unwrap();
    moof::eval(&mut w, "(slotSet! obj 'foo 42)").unwrap();
    moof::eval(&mut w, "(def captured 99)").unwrap();
    let r = moof::eval(&mut w, "[obj eval: (fn () captured)]").unwrap();
    assert_eq!(r, Value::Int(99));
}

#[test]
fn obj_eval_set_propagates_live_to_obj_via_view_target() {
    let mut w = moof::new_world();
    // (set! counter 100) inside the closure body should walk the
    // chain via env_set, hit view-target = obj, and write LIVE.
    moof::eval(&mut w, "(def obj [Object new])").unwrap();
    moof::eval(&mut w, "(slotSet! obj 'counter 0)").unwrap();
    moof::eval(&mut w, "[obj eval: (fn () (set! counter 100))]").unwrap();
    let r = moof::eval(&mut w, "(slot obj 'counter)").unwrap();
    assert_eq!(r, Value::Int(100));
}

#[test]
fn obj_eval_works_on_frozen_obj() {
    // V3's view-env doesn't mutate receiver — so frozen obj is fine
    // for read-only :eval: bodies.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def obj [Object new])").unwrap();
    moof::eval(&mut w, "(slotSet! obj 'foo 42)").unwrap();
    moof::eval(&mut w, "[obj freeze]").unwrap();
    let r = moof::eval(&mut w, "[obj eval: (fn () foo)]").unwrap();
    assert_eq!(r, Value::Int(42));
}

// V3 Task 15 — final integration coverage. these round out the
// user-facing behaviors that span multiple V3 features.

#[test]
fn here_lookup_works() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def myValue 12345)").unwrap();
    let r = moof::eval(&mut w, "[$here lookup: 'myValue]").unwrap();
    assert_eq!(r, Value::Int(12345));
}

#[test]
fn here_bind_to_works() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "[$here bind: 'newName to: 'newValue]").unwrap();
    let r = moof::eval(&mut w, "newName").unwrap();
    let new_value = w.intern("newValue");
    assert_eq!(r, Value::Sym(new_value));
}

#[test]
fn here_parent_returns_nil() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "[$here parent]").unwrap();
    assert_eq!(r, Value::Nil);
}

#[test]
fn here_self_reference_is_value_form_of_here() {
    let mut w = moof::new_world();
    let here_v = moof::eval(&mut w, "$here").unwrap();
    assert_eq!(here_v, Value::Form(w.here_form));
}
