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
    // verify proto itself is frozen.
    let proto_frozen = moof::eval_program(&mut w, "[proto frozen?]").unwrap();
    assert_eq!(proto_frozen, Value::Bool(true));
    // and the method-Form on the handler table is also frozen — that's
    // the sealed-vs-default distinction: plain freezeRecursive walks
    // slots only, freezeRecursiveSealed also walks handlers.
    let method_frozen = moof::eval_program(
        &mut w,
        "[[Heap handlerOf: proto at: 'm] frozen?]",
    )
    .unwrap();
    assert_eq!(method_frozen, Value::Bool(true));
}

// V2 task-12 — final integration coverage.

#[test]
fn raise_in_eval_program_rolls_back_freeze() {
    // a freeze that happens inside an eval_program turn that then
    // aborts via raise must roll back, just like any other journaled
    // mutation. the canonical x must remain mutable.
    let mut w = moof::new_world();
    let src = r#"
        (def x [Object new])
        (slotSet! x 'k 1)
    "#;
    moof::eval_program(&mut w, src).unwrap();
    // grab the canonical FormId for x.
    let x_id = moof::eval_program(&mut w, "x")
        .unwrap()
        .as_form_id()
        .unwrap();
    assert!(!w.heap.get(x_id).frozen);

    // freeze x and then raise — same eval_program turn.
    let raise_src = r#"
        [x freeze]
        (raise: 'boom "rolling back the freeze")
    "#;
    let r = moof::eval_program(&mut w, raise_src);
    assert!(r.is_err());
    // canonical x should NOT be frozen — turn aborted, freeze rolled back.
    assert!(!w.heap.get(x_id).frozen);
}

#[test]
#[ignore = "FromBoot + FrozenByDefault may not be lib-compatible in V2 — see spec §11"]
fn from_boot_frozen_by_default_smoke() {
    // attempt to construct a fully-frozen-from-boot world. may panic
    // if standard lib mutates a form post-:initialize during bootstrap.
    // this test is ignored by default; manually run it to track lib's
    // FromBoot-readiness over time.
    let _ = moof::new_world_with_mode_scoped(
        VatMode::FrozenByDefault,
        ModeScope::FromBoot,
    );
}

#[test]
fn commit_emits_freezings_for_pre_existing_form() {
    // construct a form, commit, then freeze in a fresh turn —
    // commit_turn's TurnDiff.freezings should list the FormId.
    let mut w = moof::new_world_bare();
    w.start_turn();
    let id = w.heap.alloc(moof::form::Form::default());
    let _ = w.commit_turn();    // form is now canonical, watermark advanced

    w.start_turn();
    w.freeze(id).unwrap();
    let diff = w.commit_turn();
    assert!(diff.freezings.contains(&id));
    assert!(w.heap.get(id).frozen);
}

#[test]
fn ic_dispatches_distinct_handlers_on_singleton_pair() {
    // regression: Bool(true) and Bool(false) share proto `Bool`,
    // so the IC must distinguish them by `cached_singleton`. before
    // the V2-task-11 fix, populating the cache via Bool(true) would
    // serve Bool(false) the wrong handler.
    let mut w = moof::new_world();
    // install a singleton handler on each of #true and #false that
    // returns a distinguishable Symbol.
    let src = r#"
        (defmethod #true (mark) 'true-mark)
        (defmethod #false (mark) 'false-mark)
        ;; force IC populate on #true, then dispatch on #false.
        ;; both invocations share the same call site if expressed
        ;; via the same chunk — eval_program runs each top-level
        ;; expression as a fresh chunk, so we wrap in a do-form.
        (do
          [#true mark]
          [#false mark])
    "#;
    let r = moof::eval_program(&mut w, src).unwrap();
    // the do-form returns the LAST expression — which is [#false mark].
    let false_mark_sym = w.intern("false-mark");
    assert_eq!(r, Value::Sym(false_mark_sym));
    // and confirm separately:
    let r_true = moof::eval_program(&mut w, "[#true mark]").unwrap();
    let true_mark_sym = w.intern("true-mark");
    assert_eq!(r_true, Value::Sym(true_mark_sym));
}
