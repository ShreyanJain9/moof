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
