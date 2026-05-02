//! `$transporter` capability tests — happy path plus every defined
//! error symbol (`tx-bad-arg`, `tx-bad-path`, `tx-no-root`,
//! `tx-not-found`, `tx-unimplemented`). `tx-read-error` is reachable
//! only via fragile filesystem permission setups and is left untested.
//! tests assume `MOOF_LIB` is set in the test harness via env (handled
//! by setting it in each test).

use moof::value::Value;
use std::path::PathBuf;

fn lib_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib")
}

fn fresh_world() -> moof::world::World {
    std::env::set_var("MOOF_LIB", lib_root());
    moof::new_world()
}

#[test]
fn root_returns_a_string() {
    let mut w = fresh_world();
    let v = moof::eval(&mut w, "[$transporter root]").unwrap();
    let s = w.string_text(v).expect(":root must return a String").to_string();
    assert!(
        s.ends_with("/lib"),
        "expected root to end with /lib, got {:?}",
        s
    );
}

#[test]
fn load_known_file_succeeds() {
    let mut w = fresh_world();
    // bootstrap.moof exists at this point (file split happens later).
    let v = moof::eval(&mut w, "[$transporter load: \"bootstrap.moof\"]");
    assert!(v.is_ok(), "load: should succeed for an existing file: {:?}", v);
}

#[test]
fn load_missing_file_raises_not_found() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: \"nope-does-not-exist.moof\"]")
        .expect_err("load: should fail for missing file");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-not-found", "wrong error kind: {}", kind_str);
}

#[test]
fn load_absolute_path_is_rejected() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: \"/etc/passwd\"]")
        .expect_err("load: should reject absolute paths");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-path");
}

#[test]
fn load_traversal_path_is_rejected() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: \"../../../etc/passwd\"]")
        .expect_err("load: should reject ..-traversing paths");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-path");
}

#[test]
fn load_non_string_arg_raises_bad_arg() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: 42]")
        .expect_err("load: should reject non-String arg");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-arg");
}

#[test]
fn load_all_walks_a_list() {
    let mut w = fresh_world();
    // empty list — returns nil, no error.
    let v = moof::eval(&mut w, "[$transporter loadAll: '()]").unwrap();
    assert!(matches!(v, Value::Nil));
}

#[test]
fn load_all_non_string_element_raises() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter loadAll: '(42)]")
        .expect_err(":loadAll: should reject non-String element");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-arg");
}

#[test]
fn dump_to_file_is_unimplemented() {
    let mut w = fresh_world();
    let err = moof::eval(
        &mut w,
        "[$transporter dump: 1 toFile: \"x\"]",
    )
    .expect_err(":dump:toFile: stub should raise");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-unimplemented");
}

#[test]
fn root_raises_no_root_when_no_root_configured() {
    // use World::new() directly — new_world_bare() now sets transporter_root
    // (so it resolves MOOF_LIB / exe/../lib / ./lib the same way new_world
    // does). we need a world with no root at all to hit tx-no-root.
    let mut w = moof::world::World::new();
    moof::intrinsics::install(&mut w);
    let err = moof::eval(&mut w, "[$transporter root]")
        .expect_err(":root should raise tx-no-root when no root configured");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-no-root");
}

#[test]
fn load_all_non_list_arg_raises_bad_arg() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter loadAll: 42]")
        .expect_err(":loadAll: should reject non-list arg");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-arg");
}

#[test]
fn compiler_cap_flips_flag_in_both_directions() {
    // bootstrapped world starts with the moof compiler in charge
    // (new_world flips the flag after compiler.moof loads). flip
    // back to seed via $compiler, then forward again via $compiler —
    // both directions get exercised end-to-end without touching
    // the field directly.
    let mut w = fresh_world();
    assert!(
        w.use_moof_compiler,
        "new_world should leave the flag true after bootstrap"
    );
    moof::eval(&mut w, "[$compiler useSeed]").unwrap();
    assert!(!w.use_moof_compiler, "useSeed should flip the flag to false");
    moof::eval(&mut w, "[$compiler useMoof]").unwrap();
    assert!(w.use_moof_compiler, "useMoof should flip the flag back to true");
}
