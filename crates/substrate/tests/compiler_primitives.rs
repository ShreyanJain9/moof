//! the `__list-*` and `__symbol-*` free-fn primitives that this
//! file used to test are all gone. the moof compiler walks lists
//! via `[Heap slotOf: x at: 'cdr]` (Heap singleton, rust install_native)
//! and `[v is nil]` (Object identity), and counts via self-recursion
//! on a Compiler:argc: helper. nothing rust-side is left to test
//! here directly — the equivalent coverage is in tests that exercise
//! the moof compiler end-to-end.
