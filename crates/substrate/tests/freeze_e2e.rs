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
