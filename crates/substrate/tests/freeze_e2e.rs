//! V2 freezing — end-to-end tests.

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
