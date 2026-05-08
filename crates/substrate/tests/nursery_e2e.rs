//! end-to-end tests for the V1 nursery + diff machinery.
//! exercises the public turn API + the implicit-turn wrapping
//! of eval_program / eval, including rollback semantics.

use moof::nursery::FaceKind;
use moof::value::Value;

#[test]
fn explicit_turn_alloc_mutate_commit() {
    let mut w = moof::new_world_bare();
    let initial_watermark = w.turn_watermark;

    w.start_turn();

    // alloc a form (above watermark — new-alloc path).
    let id = w.heap.alloc(moof::form::Form::default());
    let key = w.intern("hello");
    w.form_slot_set(id, key, Value::Int(42)).unwrap();

    // mid-turn: read sees the value (direct canonical for new alloc).
    assert_eq!(w.form_slot(id, key), Value::Int(42));

    let diff = w.commit_turn();

    // post-commit: heap is canonical-updated.
    assert_eq!(w.heap.get(id).slot(key), Value::Int(42));
    // diff lists the new alloc.
    assert!(diff.new_allocs.contains(&id));
    // new alloc's mutations don't appear in diff.mutations
    // (it had no prior state).
    assert!(diff.mutations.is_empty());
    // watermark advanced.
    assert_eq!(w.turn_watermark, initial_watermark + 1);
}

#[test]
fn explicit_turn_mutate_pre_existing_emits_diff_entry() {
    let mut w = moof::new_world_bare();

    // alloc and commit one form first.
    w.start_turn();
    let id = w.heap.alloc(moof::form::Form::default());
    let key = w.intern("count");
    w.form_slot_set(id, key, Value::Int(0)).unwrap();
    let _ = w.commit_turn();

    // now mutate the pre-existing form in a new turn.
    w.start_turn();
    w.form_slot_set(id, key, Value::Int(99)).unwrap();
    let diff = w.commit_turn();

    // diff has the (id, slots, key) entry with prior=0, new=99.
    let entry = diff
        .mutations
        .get(&(id, FaceKind::Slots, key))
        .copied();
    assert_eq!(entry, Some((Value::Int(0), Value::Int(99))));
    assert_eq!(w.heap.get(id).slot(key), Value::Int(99));
}

#[test]
fn explicit_turn_abort_rolls_back_alloc_and_mutation() {
    let mut w = moof::new_world_bare();

    // first turn: alloc and commit a form.
    w.start_turn();
    let id = w.heap.alloc(moof::form::Form::default());
    let key = w.intern("count");
    w.form_slot_set(id, key, Value::Int(0)).unwrap();
    let _ = w.commit_turn();
    let watermark_after_first_commit = w.turn_watermark;

    // second turn: alloc another form, mutate the first, then abort.
    w.start_turn();
    let _id2 = w.heap.alloc(moof::form::Form::default());
    w.form_slot_set(id, key, Value::Int(99)).unwrap();
    w.abort_turn();

    // canonical state preserved.
    assert_eq!(w.heap.get(id).slot(key), Value::Int(0));
    // watermark unchanged (abort doesn't advance).
    assert_eq!(w.turn_watermark, watermark_after_first_commit);
    // heap was truncated — _id2 no longer exists in the Vec.
    assert_eq!(w.heap.len() as u32, watermark_after_first_commit);
}

#[test]
fn raise_in_eval_program_aborts_implicit_turn() {
    let mut w = moof::new_world_bare();
    let env_id = w.global_env;
    let foo_sym = w.intern("foo");
    assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Nil);

    let result = moof::eval_program(
        &mut w,
        "(def foo 5) (raise: 'boom \"x\")",
    );
    assert!(result.is_err());

    // foo binding rolled back; canonical env unchanged.
    assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Nil);
}

#[test]
fn successful_eval_program_commits_state_visibly() {
    let mut w = moof::new_world_bare();
    let env_id = w.global_env;
    let foo_sym = w.intern("foo");

    let result = moof::eval_program(&mut w, "(def foo 42) foo");
    assert_eq!(result.unwrap(), Value::Int(42));

    // post-commit: canonical env has the binding.
    assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Int(42));
}

#[test]
fn mutation_outside_turn_panics() {
    let mut w = moof::new_world_bare();
    let id = w.heap.alloc(moof::form::Form::default());

    let x_sym = w.intern("x");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // panic fires from form_slot_set's `assert!(in_turn)` before
        // the Result is constructed — `let _ =` for the silenced Result
        // shape on the rare path it survives.
        let _ = w.form_slot_set(id, x_sym, Value::Int(1));
    }));
    assert!(panicked.is_err(), "expected panic on mutation outside turn");
}

#[test]
fn diff_handles_handlers_and_meta_faces() {
    let mut w = moof::new_world_bare();

    // alloc and commit.
    w.start_turn();
    let id = w.heap.alloc(moof::form::Form::default());
    let _ = w.commit_turn();

    // mutate all three faces.
    w.start_turn();
    let k = w.intern("k");
    w.form_slot_set(id, k, Value::Int(1)).unwrap();
    w.form_handler_set(id, k, Value::Int(2)).unwrap();
    w.form_meta_set(id, k, Value::Int(3)).unwrap();
    let diff = w.commit_turn();

    assert_eq!(diff.mutations.len(), 3);
    assert!(diff.mutations.contains_key(&(id, FaceKind::Slots, k)));
    assert!(diff.mutations.contains_key(&(id, FaceKind::Handlers, k)));
    assert!(diff.mutations.contains_key(&(id, FaceKind::Meta, k)));
}
