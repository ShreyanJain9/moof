//! V2 freezing — end-to-end tests.

use moof::value::Value;
use moof::{ModeScope, VatMode};

#[test]
fn new_world_defaults_to_mutable_by_default() {
    let w = moof::new_world_bare();
    assert_eq!(w.vat_mode, VatMode::MutableByDefault);
}

#[test]
fn new_world_with_mode_post_bootstrap_sets_mode_after_construction() {
    let w = moof::new_world_bare_with_mode(VatMode::FrozenByDefault);
    assert_eq!(w.vat_mode, VatMode::FrozenByDefault);
}

#[test]
fn mode_scope_post_bootstrap_runs_bootstrap_mutable() {
    // we can't directly observe "boot ran in mutable mode", but we
    // can confirm the world boots without panicking under
    // FrozenByDefault + PostBootstrap (lib bootstrap is allowed
    // to mutate regardless).
    let _ = moof::new_world_with_mode_scoped(
        VatMode::FrozenByDefault,
        ModeScope::PostBootstrap,
    );
    // reaching this line means bootstrap completed.
}

#[test]
fn mutable_by_default_new_returns_mutable_form() {
    let mut w = moof::new_world_bare_with_mode(VatMode::MutableByDefault);
    let result = moof::eval_program(&mut w, "[Object new]").unwrap();
    let id = result.as_form_id().unwrap();
    assert!(!w.heap.get(id).frozen);
}

#[test]
fn frozen_by_default_new_returns_frozen_form() {
    let mut w = moof::new_world_bare_with_mode(VatMode::FrozenByDefault);
    let result = moof::eval_program(&mut w, "[Object new]").unwrap();
    let id = result.as_form_id().unwrap();
    assert!(w.heap.get(id).frozen);
}

// V2 task-10 — :freeze / :frozen? / :freezable? bound on Object.

#[test]
fn freeze_method_bound_on_object_freezes() {
    let mut w = moof::new_world_bare();
    // local def returns nil here, so split into two top-level forms
    // and let the program result be the [p freeze] expression.
    let result = moof::eval_program(
        &mut w,
        "(def p [Object new]) [p freeze]",
    )
    .unwrap();
    let id = result.as_form_id().unwrap();
    assert!(w.heap.get(id).frozen);
}

#[test]
fn frozen_query_returns_bool() {
    let mut w = moof::new_world_bare();
    let unfrozen = moof::eval_program(&mut w, "[[Object new] frozen?]").unwrap();
    assert_eq!(unfrozen, Value::Bool(false));
    let frozen = moof::eval_program(&mut w, "[[[Object new] freeze] frozen?]").unwrap();
    assert_eq!(frozen, Value::Bool(true));
}

#[test]
fn freezable_query_returns_bool() {
    let mut w = moof::new_world_bare();
    let unfrozen = moof::eval_program(&mut w, "[[Object new] freezable?]").unwrap();
    assert_eq!(unfrozen, Value::Bool(true));
    let frozen = moof::eval_program(&mut w, "[[[Object new] freeze] freezable?]").unwrap();
    assert_eq!(frozen, Value::Bool(false));
}

#[test]
fn freeze_on_cap_raises_cannot_freeze_live() {
    let mut w = moof::new_world(); // full lib so $out exists
    let r = moof::eval(&mut w, "[$out freeze]");
    assert!(r.is_err());
    assert_eq!(w.resolve(r.unwrap_err().kind), "cannot-freeze-live");
}

#[test]
fn freeze_on_tagged_immediate_is_noop() {
    // tagged immediates are inherently immutable; :freeze returns
    // self unchanged rather than raising. matches :frozen? which
    // returns true and :freezable? which returns false on the same.
    let mut w = moof::new_world_bare();
    let r = moof::eval_program(&mut w, "[42 freeze]").unwrap();
    assert_eq!(r, Value::Int(42));
    let r2 = moof::eval_program(&mut w, "[#true freeze]").unwrap();
    assert_eq!(r2, Value::Bool(true));
}

// V2 task-11 — :freezeRecursive / :freezeRecursiveSealed /
// :freezeRecursiveWalking: live in lib/stdlib/freezing.moof.

#[test]
fn freeze_recursive_walks_slots_default() {
    let mut w = moof::new_world();
    let src = r#"
        (def parent [Object new])
        (def child [Object new])
        (slotSet! parent 'c child)
        [parent freezeRecursive]
    "#;
    moof::eval_program(&mut w, src).unwrap();
    let p_frozen = moof::eval_program(&mut w, "[parent frozen?]").unwrap();
    let c_frozen = moof::eval_program(&mut w, "[child frozen?]").unwrap();
    assert_eq!(p_frozen, Value::Bool(true));
    assert_eq!(c_frozen, Value::Bool(true));
}

#[test]
fn freeze_recursive_handles_cycle_via_already_frozen() {
    let mut w = moof::new_world();
    // a → b → a back-edge. we freeze self before recursing, so the
    // back-edge's :frozen? check terminates the walk.
    let src = r#"
        (def a [Object new])
        (def b [Object new])
        (slotSet! a 'next b)
        (slotSet! b 'next a)
        [a freezeRecursive]
    "#;
    moof::eval_program(&mut w, src).unwrap();
    let a_frozen = moof::eval_program(&mut w, "[a frozen?]").unwrap();
    let b_frozen = moof::eval_program(&mut w, "[b frozen?]").unwrap();
    assert_eq!(a_frozen, Value::Bool(true));
    assert_eq!(b_frozen, Value::Bool(true));
}


#[test]
fn freeze_recursive_stops_at_live_boundary() {
    // a parent slot points at $out (a live cap). the walk freezes
    // parent, then encounters $out (not freezable), and silently
    // bails without raising.
    let mut w = moof::new_world();
    let src = r#"
        (def parent [Object new])
        (slotSet! parent 'cap $out)
        [parent freezeRecursive]
    "#;
    moof::eval_program(&mut w, src).unwrap();
    let p_frozen = moof::eval_program(&mut w, "[parent frozen?]").unwrap();
    let cap_frozen = moof::eval_program(&mut w, "[$out frozen?]").unwrap();
    assert_eq!(p_frozen, Value::Bool(true));
    assert_eq!(cap_frozen, Value::Bool(false));
}

#[test]
fn freeze_recursive_sealed_walks_handlers() {
    let mut w = moof::new_world();
    // freezeRecursiveSealed walks slots + handlers. install a
    // handler on a fresh proto, then deep-freeze it sealed.
    let src = r#"
        (def proto [Object new])
        (defmethod proto (m) 42)
        [proto freezeRecursiveSealed]
    "#;
    let r = moof::eval_program(&mut w, src);
    assert!(r.is_ok());
    let proto_frozen = moof::eval_program(&mut w, "[proto frozen?]").unwrap();
    assert_eq!(proto_frozen, Value::Bool(true));
}
