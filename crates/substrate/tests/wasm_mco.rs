//! end-to-end test: load a hand-rolled wasm mco and call it from
//! moof.
//!
//! see `examples/wasm-mcos/hello.zig` for the source — a tiny zig
//! program exporting `answer() i64` returning 42. compiled to
//! `examples/wasm-mcos/hello.wasm` via:
//!
//!   zig build-exe -target wasm32-freestanding -O ReleaseSmall \
//!     -fno-entry --export=answer hello.zig
//!
//! per `docs/reference/mco-format.md`, loading returns a fresh
//! proto-Form. moof code names it (load-time anonymity); methods
//! are dispatched normally.

use moof::value::Value;

/// embed the wasm bytes at test-build time. avoids fragile
/// path-from-cargo-test-cwd issues.
const HELLO_WASM: &[u8] = include_bytes!("../../../examples/wasm-mcos/hello.wasm");

#[test]
fn load_hello_wasm_and_call_answer() {
    // load wasm bytes directly (skipping the disk → readback step).
    // result is a proto-Form; the substrate doesn't auto-bind it to
    // any name.
    let mut w = moof::new_world();
    let proto = moof::wasm::load_wasm_bytes(&mut w, HELLO_WASM, "hello.wasm")
        .expect("load");
    // bind it manually under a name moof code can see.
    let hello_sym = w.intern("Hello");
    w.env_bind(w.global_env, hello_sym, proto);
    // [Hello answer] sends `:answer` to the proto. dispatches to
    // the wasm trampoline, which calls the wasm `answer` export.
    let r = moof::eval(&mut w, "[Hello answer]").expect("call");
    assert_eq!(r, Value::Int(42));
}

#[test]
fn loaded_proto_is_anonymous() {
    // load-time anonymity: the loaded proto has no `:name` meta
    // (the user supplies the name via `def`).
    let mut w = moof::new_world();
    let v = moof::wasm::load_wasm_bytes(&mut w, HELLO_WASM, "hello.wasm").unwrap();
    let id = v.as_form_id().expect("loaded mco should be a Form");
    let name_meta = w.intern("name");
    let name = w.heap.get(id).meta_at(name_meta);
    assert!(
        name.is_nil(),
        "wasm mco should not self-name; got {:?}",
        name,
    );
}

#[test]
fn loaded_proto_can_be_instantiated_and_called() {
    // [Hello new] gets an instance whose proto is Hello. sending
    // `:answer` dispatches up the proto chain to the wasm method.
    let mut w = moof::new_world();
    let proto = moof::wasm::load_wasm_bytes(&mut w, HELLO_WASM, "hello.wasm").unwrap();
    let hello_sym = w.intern("Hello");
    w.env_bind(w.global_env, hello_sym, proto);
    let r = moof::eval(&mut w, "(do (def h [Hello new]) [h answer])").unwrap();
    assert_eq!(r, Value::Int(42));
}
