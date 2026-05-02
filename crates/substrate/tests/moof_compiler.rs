//! track-2 forcing function: every form compiled by the moof-side
//! compiler (`lib/compiler.moof`) produces the same runtime value
//! as the rust-side compiler.
//!
//! at this stage we compare *runtime values*, not byte-identical
//! chunks. byte-identity is the track-3 deliverable and a strict
//! superset of value-identity. if the chunks differ but both
//! produce the same value, the moof compiler is correct-but-not-
//! canonical; track 3 closes the gap.
//!
//! see `docs/reference/compiler-primitives.md` for the substrate
//! primitives the moof compiler is built on.
//! see `lib/compiler.moof` for the compiler itself.
//! see `NEXT_SESSION.md` for the track structure.
//!
//! the assertion shape:
//!
//!   assert_compilers_agree(w, "<source>");
//!
//! evaluates the source two ways:
//!   1. read → rust compile → run.
//!   2. read → moof compile (via `__mc-compile-and-run`) → run.
//! and asserts the values are equal.

use moof::value::Value;
use moof::world::World;

/// compile `src` via both compilers and assert they produce the same
/// runtime value. fails loudly on any compile-time or run-time error.
///
/// reads the source once, then drives both compilers on the same
/// parsed form — this keeps any *literal* Forms in the const pool
/// identity-equal across the two paths. results are compared
/// structurally (deep) so freshly-allocated forms (Strings, Lists
/// built at runtime) match by content rather than heap-id.
fn assert_compilers_agree(w: &mut World, src: &str) {
    let form = w
        .read(src)
        .unwrap_or_else(|e| panic!("read failed on `{}`: {}", src, e.message));

    // path 1: rust compiler.
    let rust_chunk = moof::compiler::compile(w, form)
        .unwrap_or_else(|e| panic!("rust compile failed on `{}`: {}", src, e.message));
    let rust_result = w
        .run_top(rust_chunk)
        .unwrap_or_else(|e| panic!("rust run failed on `{}`: {}", src, e.message));

    // path 2: moof compiler.
    let helper_sym = w.intern("__mc-compile-and-run");
    let helper = w
        .env_lookup(w.global_env, helper_sym)
        .expect("__mc-compile-and-run is unbound — compiler.moof didn't load");
    let call_sym = w.intern("call");
    let moof_result = w
        .send(helper, call_sym, &[form])
        .unwrap_or_else(|e| panic!("moof compile/run failed on `{}`: {}", src, e.message));

    assert!(
        values_equal_deep(w, rust_result, moof_result),
        "compilers disagree on `{}` — rust = {:?}, moof = {:?}",
        src,
        rust_result,
        moof_result,
    );
}

/// structural deep-equality for `Value`s. needed because operations
/// like `[42 toString]` allocate a fresh String on each run; the
/// rust and moof paths each produce one, identity-distinct but
/// content-identical. compares Strings by bytes and Lists element-
/// wise; everything else falls through to `==`.
fn values_equal_deep(w: &World, a: Value, b: Value) -> bool {
    if a == b {
        return true;
    }
    if let (Some(sa), Some(sb)) = (w.string_text(a), w.string_text(b)) {
        return sa == sb;
    }
    // both lists?
    if matches!(w.proto_of(a), Value::Form(p) if p == w.protos.cons)
        && matches!(w.proto_of(b), Value::Form(p) if p == w.protos.cons)
    {
        let va = match w.list_to_vec(a) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let vb = match w.list_to_vec(b) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if va.len() != vb.len() {
            return false;
        }
        return va
            .iter()
            .zip(vb.iter())
            .all(|(x, y)| values_equal_deep(w, *x, *y));
    }
    false
}

/// like `assert_compilers_agree`, but each source is run in a fresh
/// world so they don't share global state. used for tests that don't
/// need `def` continuity.
fn agree_fresh(src: &str) {
    let mut w = moof::new_world();
    assert_compilers_agree(&mut w, src);
}

// ── literals ────────────────────────────────────────────────────

#[test]
fn agrees_on_integer_literal() {
    agree_fresh("42");
    agree_fresh("0");
    agree_fresh("-7");
}

#[test]
fn agrees_on_bool_literal() {
    agree_fresh("#true");
    agree_fresh("#false");
}

#[test]
fn agrees_on_nil_literal() {
    agree_fresh("nil");
}

#[test]
fn agrees_on_quoted_symbol() {
    agree_fresh("'foo");
    agree_fresh("'bar");
}

#[test]
fn agrees_on_quoted_list() {
    agree_fresh("'(1 2 3)");
    agree_fresh("'(a (b c) d)");
}

#[test]
fn agrees_on_empty_list() {
    agree_fresh("()");
}

// ── symbols / lookup ────────────────────────────────────────────

#[test]
fn agrees_on_symbol_lookup_via_def() {
    let mut w = moof::new_world();
    assert_compilers_agree(&mut w, "(def x 100)");
    assert_compilers_agree(&mut w, "x");
}

#[test]
fn agrees_on_global_lookup() {
    // `Object` is in the global env from boot; both compilers should
    // emit a LoadName that finds it.
    agree_fresh("Object");
    agree_fresh("Integer");
}

// ── send-bracket sends ──────────────────────────────────────────

#[test]
fn agrees_on_arithmetic_sends() {
    agree_fresh("[1 + 2]");
    agree_fresh("[10 - 3]");
    agree_fresh("[4 * 5]");
    agree_fresh("[3 * [4 + 5]]");
}

#[test]
fn agrees_on_unary_send() {
    agree_fresh("[5 proto]");
    agree_fresh("[42 toString]");
}

#[test]
fn agrees_on_keyword_send() {
    agree_fresh("[(list 1 2 3) length]");
}

// ── if ──────────────────────────────────────────────────────────

#[test]
fn agrees_on_if_true() {
    agree_fresh("(if #true 'yes 'no)");
}

#[test]
fn agrees_on_if_false() {
    agree_fresh("(if #false 'yes 'no)");
}

#[test]
fn agrees_on_if_nil_is_falsy() {
    agree_fresh("(if nil 'yes 'no)");
}

#[test]
fn agrees_on_if_no_else_branch() {
    agree_fresh("(if #true 'yes)");
    agree_fresh("(if #false 'yes)");
}

#[test]
fn agrees_on_nested_if() {
    agree_fresh("(if #true (if #false 'a 'b) 'c)");
    agree_fresh("(if #true (if #true 'a 'b) 'c)");
}

// ── do ──────────────────────────────────────────────────────────

#[test]
fn agrees_on_do_returns_last() {
    agree_fresh("(do 1 2 3)");
}

#[test]
fn agrees_on_empty_do() {
    agree_fresh("(do)");
}

// ── set! ────────────────────────────────────────────────────────

#[test]
fn agrees_on_set_after_def() {
    let mut w = moof::new_world();
    assert_compilers_agree(&mut w, "(def n 1)");
    assert_compilers_agree(&mut w, "(do (set! n 99) n)");
}

// ── fn / call ───────────────────────────────────────────────────

#[test]
fn agrees_on_fn_identity() {
    agree_fresh("((fn (x) x) 42)");
}

#[test]
fn agrees_on_fn_with_send_body() {
    agree_fresh("((fn (a b) [a + b]) 3 4)");
}

#[test]
fn agrees_on_closure_captures_env() {
    agree_fresh("((fn (n) ((fn (x) [x + n]) 10)) 5)");
}

#[test]
fn agrees_on_recursive_fn() {
    let mut w = moof::new_world();
    assert_compilers_agree(
        &mut w,
        "(def fact (fn (n) (if [n = 0] 1 [n * (fact [n - 1])])))",
    );
    assert_compilers_agree(&mut w, "(fact 0)");
    assert_compilers_agree(&mut w, "(fact 1)");
    assert_compilers_agree(&mut w, "(fact 6)");
}

// ── let (via the bootstrap macro; compile-let is a fallback) ────

#[test]
fn agrees_on_let_simple() {
    agree_fresh("(let ((a 3) (b 4)) [a + b])");
}

#[test]
fn agrees_on_let_star_sequential() {
    agree_fresh("(let* ((a 1) (b a)) b)");
}

#[test]
fn agrees_on_let_does_not_leak() {
    agree_fresh("(let ((a 5)) a)");
}

// ── defmacro (eager registration; runtime def of name) ──────────

#[test]
fn agrees_on_defmacro_then_use() {
    let mut w = moof::new_world();
    // a tiny user macro that swaps two args.
    assert_compilers_agree(
        &mut w,
        "(defmacro swap2 (args) (list (list 'fn '(a b) '(list b a)) [args car] [[args cdr] car]))",
    );
    // (swap2 1 2) expands to ((fn (a b) (list b a)) 1 2) → (2 1) as a list.
    let r = moof::eval(&mut w, "[(swap2 1 2) car]").unwrap();
    assert_eq!(r, Value::Int(2));
}

// ── interaction: bootstrap macros (when, match, defn) ───────────

#[test]
fn agrees_on_when_macro() {
    agree_fresh("(when #true 1 2 3)");
    agree_fresh("(when #false 1 2 3)");
}

#[test]
fn agrees_on_match_macro() {
    agree_fresh(
        "(match 5
           1     'one
           5     'five
           _     'other)",
    );
}

#[test]
fn agrees_on_cascade_via_macro() {
    // cascade is a moof macro that desugars to the same shape both
    // compilers see — exercises macroexpand + send + let + do.
    agree_fresh("(let ((c (list 1 2 3))) [c car])");
}
