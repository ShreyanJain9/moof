//! tests for the rust-side free-function primitives that the moof
//! compiler uses internally. these break the circular dep between
//! `Cons:length` (a moof method) and the moof compiler's own
//! `compileSend` (which needs to know list length).

use moof::value::Value;

fn fresh_world() -> moof::world::World {
    moof::new_world()
}

#[test]
fn list_length_primitive() {
    let mut w = fresh_world();
    assert_eq!(
        moof::eval(&mut w, "(__list-length '(1 2 3))").unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        moof::eval(&mut w, "(__list-length nil)").unwrap(),
        Value::Int(0)
    );
}

#[test]
fn list_empty_primitive() {
    let mut w = fresh_world();
    assert_eq!(
        moof::eval(&mut w, "(__list-empty? nil)").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "(__list-empty? '(1))").unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn list_car_cdr_primitives() {
    let mut w = fresh_world();
    assert_eq!(
        moof::eval(&mut w, "(__list-car '(1 2 3))").unwrap(),
        Value::Int(1)
    );
    let v = moof::eval(&mut w, "(__list-cdr '(1 2 3))").unwrap();
    assert_eq!(w.list_len(v).unwrap(), 2);
}

#[test]
fn list_car_cdr_on_nil() {
    let mut w = fresh_world();
    assert!(matches!(
        moof::eval(&mut w, "(__list-car nil)").unwrap(),
        Value::Nil
    ));
    assert!(matches!(
        moof::eval(&mut w, "(__list-cdr nil)").unwrap(),
        Value::Nil
    ));
}

#[test]
fn list_reverse_primitive() {
    let mut w = fresh_world();
    let v = moof::eval(&mut w, "(__list-reverse '(1 2 3))").unwrap();
    let elems = w.list_to_vec(v).unwrap();
    assert_eq!(
        elems,
        vec![Value::Int(3), Value::Int(2), Value::Int(1),]
    );
}

// __symbol-ends-with-colon? was removed — Symbol:endsWithColon? in
// early/04-symbol.moof uses [[self toString] endsWith?: ":"] which
// dispatches through methods. no compiler-internal primitive needed.
