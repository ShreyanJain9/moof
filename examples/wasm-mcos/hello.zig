// minimum-viable polyglot smoke test.
// compiles to wasm32-freestanding; exports a single `answer()`
// returning 42. the substrate's wasm loader instantiates this and
// exposes `answer` as a method on a fresh proto-Form.

export fn answer() i64 {
    return 42;
}
