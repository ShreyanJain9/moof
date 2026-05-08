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
