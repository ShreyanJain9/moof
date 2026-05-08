//! Regression: `World::send` from rust callers (REPL, eval_one_shot,
//! embedders) outside an active turn must not panic. The dispatched
//! method body may call `env_bind` (any moof-defined method with
//! params or let-bindings), which goes through `form_slot_set` and
//! asserts `in_turn`.
//!
//! Pre-fix: `moof '(list 1 2)'` panics with "form_slot_set called
//! outside a turn" because eval commits its turn before returning,
//! and the cli's `print_via_out_inspect` then calls `world.send`
//! outside any turn — Cons:inspect uses env_bind in its body.
//!
//! Post-fix: `World::send` defensively wraps a turn (was_in_turn
//! pattern, mirroring `run_top` and `lib::eval`).

use moof::value::Value;

#[test]
fn send_outside_turn_dispatches_moof_defined_method() {
    let mut w = moof::new_world();
    // boot turn already committed; we are not in a turn.
    assert!(!w.in_turn());
    let inspect = w.intern("inspect");

    // Cons:inspect is moof-defined and recursively binds via env_bind.
    // Pre-fix: panics inside invoke -> env_bind -> form_slot_set.
    let cons = moof::eval(&mut w, "(list 1 2 3)").unwrap();
    // back outside a turn after eval committed.
    assert!(!w.in_turn());
    let s = w.send(cons, inspect, &[]);
    assert!(s.is_ok(), "World::send must defensively wrap a turn — got {:?}", s.err());
    let text = w.string_text(s.unwrap()).unwrap().to_string();
    assert_eq!(text, "(1 2 3)");
}

#[test]
fn send_outside_turn_for_tagged_immediate_already_works() {
    // Sanity check — tagged immediates dispatch to natives that
    // don't env_bind, so they always worked. Locks in the baseline.
    let mut w = moof::new_world();
    assert!(!w.in_turn());
    let inspect = w.intern("inspect");
    let s = w.send(Value::Int(42), inspect, &[]).unwrap();
    let text = w.string_text(s).unwrap().to_string();
    assert_eq!(text, "42");
}

#[test]
fn send_inside_active_turn_does_not_double_wrap() {
    // When the caller is already inside a turn, send should not
    // start a new one (would panic — nested turns are forbidden).
    let mut w = moof::new_world();
    w.start_turn();
    let inspect = w.intern("inspect");
    let cons = moof::value::Value::Nil;
    let s = w.send(cons, inspect, &[]);
    assert!(s.is_ok());
    let _ = w.commit_turn();
}
