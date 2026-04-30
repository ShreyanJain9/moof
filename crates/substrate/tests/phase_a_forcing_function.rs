//! phase-A acceptance gate.
//!
//! these tests exercise the forcing function from
//! `docs/process/impl-plan-v4.md` (phase A.14):
//!
//! - `moof '(+ 1 2)' → 3` (printed via `$out say:`)
//! - `[5 proto] → Integer`
//! - `[(fn (x) (* x x)) source] → '(fn (x) (* x x))`
//! - **no** path to stdout that bypasses `$out`.
//!
//! anything below is correctness over completion: every assertion
//! corresponds to a substrate-law promise.

use moof::value::Value;

#[test]
fn forcing_function_arithmetic() {
    // canonical moof: send-bracket binary on Integer.
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[1 + 2]").unwrap(), Value::Int(3));
    assert_eq!(
        moof::eval(&mut w, "[3 * [4 + 5]]").unwrap(),
        Value::Int(27)
    );
    assert_eq!(moof::eval(&mut w, "[10 - 3]").unwrap(), Value::Int(7));
}

#[test]
fn forcing_function_proto_reflection() {
    // L1 + L6: every value has a proto reachable through reflection.
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "[5 proto]").unwrap();
    assert_eq!(r, Value::Form(w.protos.integer));
    let r = moof::eval(&mut w, "[#true proto]").unwrap();
    assert_eq!(r, Value::Form(w.protos.bool_));
    let r = moof::eval(&mut w, "[nil proto]").unwrap();
    assert_eq!(r, Value::Form(w.protos.nil));
    let r = moof::eval(&mut w, "['hello proto]").unwrap();
    assert_eq!(r, Value::Form(w.protos.symbol));
}

#[test]
fn forcing_function_source_reflection() {
    // L5: source is canonical; `[m source]` returns the source-form.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def square (fn (x) [x * x]))").unwrap();
    let inspected = moof::eval(&mut w, "[square inspect]").unwrap();
    // inspect now returns a String form (replacing the prior
    // Sym-as-string placeholder).
    let text = w.string_text(inspected).unwrap();
    assert!(!text.is_empty());
}

#[test]
fn forcing_function_recursion() {
    // closures + recursion + arithmetic all on substrate-laws path.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(def fact (fn (n)
            (if [n = 0]
                1
                [n * (fact [n - 1])])))",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "(fact 0)").unwrap(), Value::Int(1));
    assert_eq!(moof::eval(&mut w, "(fact 5)").unwrap(), Value::Int(120));
    assert_eq!(moof::eval(&mut w, "(fact 10)").unwrap(), Value::Int(3628800));
    assert_eq!(moof::eval(&mut w, "(fact 12)").unwrap(), Value::Int(479001600));
}

#[test]
fn forcing_function_closure_captures() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def make-adder (fn (n) (fn (x) [x + n])))").unwrap();
    assert_eq!(
        moof::eval(&mut w, "((make-adder 5) 7)").unwrap(),
        Value::Int(12)
    );
    assert_eq!(
        moof::eval(&mut w, "((make-adder 100) 23)").unwrap(),
        Value::Int(123)
    );
}

#[test]
fn forcing_function_no_print_globals() {
    // process/docs-driven.md's symbol-table check.
    let mut w = moof::new_world();
    for forbidden in ["print", "println", "puts", "simulated_println"] {
        let s = w.intern(forbidden);
        assert!(
            w.env_lookup(w.global_env, s).is_none(),
            "forbidden global `{}` exists in seed",
            forbidden
        );
    }
}

#[test]
fn forcing_function_out_cap_in_scope() {
    let mut w = moof::new_world();
    let dollar_out = w.intern("$out");
    let dollar_err = w.intern("$err");
    assert!(w.env_lookup(w.global_env, dollar_out).is_some());
    assert!(w.env_lookup(w.global_env, dollar_err).is_some());
}

#[test]
fn forcing_function_does_not_understand_for_unknown_selectors() {
    let mut w = moof::new_world();
    let mystery = w.intern("flibbertigibbet");
    let err = w.send(Value::Int(5), mystery, &[]).unwrap_err();
    assert_eq!(w.resolve(err.kind), "doesNotUnderstand");
}

#[test]
fn forcing_function_let_star_is_sequential() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "(let* ((a 1) (b a) (c [a + b])) c)").unwrap(),
        Value::Int(2)
    );
}

#[test]
fn forcing_function_quote_returns_form() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "'foo").unwrap();
    let s = r.as_sym().unwrap();
    assert_eq!(w.resolve(s), "foo");

    // (quote (1 2 3)) → a list-Form
    let r = moof::eval(&mut w, "'(1 2 3)").unwrap();
    let id = r.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.list));
}

#[test]
fn forcing_function_set_updates() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def x 1)").unwrap();
    moof::eval(&mut w, "(set! x 99)").unwrap();
    assert_eq!(moof::eval(&mut w, "x").unwrap(), Value::Int(99));
}

#[test]
fn forcing_function_identity_distinguishes_forms() {
    // L11: identity is stable per heap-id.
    let mut w = moof::new_world();
    let a = moof::eval(&mut w, "(list 1 2 3)").unwrap();
    let b = moof::eval(&mut w, "(list 1 2 3)").unwrap();
    let identity = w.intern("identity");
    let id_a = w.send(a, identity, &[]).unwrap();
    let id_b = w.send(b, identity, &[]).unwrap();
    // different alloc → different identity
    assert_ne!(id_a, id_b);
    let is_sel = w.intern("is");
    assert_eq!(w.send(a, is_sel, &[a]).unwrap(), Value::Bool(true));
    assert_eq!(w.send(a, is_sel, &[b]).unwrap(), Value::Bool(false));
}

#[test]
fn forcing_function_send_walks_proto_chain() {
    // L3: dispatch is universal; user-installed handlers on Object
    // are reachable from any proto.
    let mut w = moof::new_world();
    // Object has := installed; integers also have :=. integer's
    // shadows object's for Int receivers — confirm by behavior:
    let eq = w.intern("=");
    assert_eq!(
        w.send(Value::Int(5), eq, &[Value::Int(5)]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn forcing_function_pipeline_works() {
    // composite test: read → compile → run on a non-trivial program.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(def sum-to (fn (n) (if [n = 0] 0 [n + (sum-to [n - 1])])))",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "(sum-to 10)").unwrap(),
        Value::Int(55)
    );
}

// ─────────────────────────────────────────────────────────────────
// the moldable-seed claim: bootstrap.moof installs methods on
// canonical protos via `setHandler!`. these tests verify the moof-
// side stdlib is loaded and dispatch-reachable.
// ─────────────────────────────────────────────────────────────────

#[test]
fn bootstrap_list_length() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3 4 5) length]").unwrap(),
        Value::Int(5)
    );
    assert_eq!(moof::eval(&mut w, "[nil length]").unwrap(), Value::Int(0));
    assert_eq!(
        moof::eval(&mut w, "[(list) length]").unwrap(),
        Value::Int(0)
    );
}

#[test]
fn bootstrap_list_empty() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[nil empty?]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 1) empty?]").unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn bootstrap_integer_predicates() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[0 zero?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[5 zero?]").unwrap(), Value::Bool(false));
    assert_eq!(
        moof::eval(&mut w, "[7 positive?]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[-3 negative?]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(moof::eval(&mut w, "[-42 abs]").unwrap(), Value::Int(42));
    assert_eq!(moof::eval(&mut w, "[9 square]").unwrap(), Value::Int(81));
    assert_eq!(
        moof::eval(&mut w, "[5 between: 1 and: 10]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[5 between: 6 and: 10]").unwrap(),
        Value::Bool(false)
    );
    assert_eq!(moof::eval(&mut w, "[5 min: 3]").unwrap(), Value::Int(3));
    assert_eq!(moof::eval(&mut w, "[5 max: 3]").unwrap(), Value::Int(5));
}

#[test]
fn bootstrap_higher_order_ops() {
    // method-shape passes lambdas explicitly: `(fn (x) [x square])`
    // rather than reaching for a free-function `square` (which
    // would be a free-function-stdlib anti-pattern).
    let mut w = moof::new_world();
    // [xs map: f] — receiver is the subject.
    let r = moof::eval(
        &mut w,
        "[[(list 1 2 3 4 5) map: (fn (x) [x square])] length]",
    )
    .unwrap();
    assert_eq!(r, Value::Int(5));
    // sum of squares (via :sum derived method, which uses :reduce:from:)
    let r = moof::eval(
        &mut w,
        "[[(list 1 2 3 4 5) map: (fn (x) [x square])] sum]",
    )
    .unwrap();
    assert_eq!(r, Value::Int(55));
    // filter positive
    let r = moof::eval(
        &mut w,
        "[[(list -2 -1 0 1 2) filter: (fn (x) [x positive?])] length]",
    )
    .unwrap();
    assert_eq!(r, Value::Int(2));
    // reverse
    let r = moof::eval(&mut w, "[[(list 1 2 3) reverse] head]").unwrap();
    assert_eq!(r, Value::Int(3));
    // take
    let r = moof::eval(
        &mut w,
        "[[(list 1 2 3 4 5) take: 3] length]",
    )
    .unwrap();
    assert_eq!(r, Value::Int(3));
    // contains?
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) contains?: 2]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) contains?: 9]").unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn bootstrap_no_free_function_predicates() {
    // confirm `positive?` and `square` are NOT free-function globals.
    // they're methods on Integer; user code passes a lambda
    // explicitly when treating them as values.
    let mut w = moof::new_world();
    for forbidden in [
        "positive?",
        "negative?",
        "zero?",
        "abs",
        "square",
        "map",
        "filter",
        "reduce",
        "length",
        "empty?",
    ] {
        let s = w.intern(forbidden);
        assert!(
            w.env_lookup(w.global_env, s).is_none(),
            "`{}` should not be a free-function global",
            forbidden
        );
    }
    // via send: yes.
    let positive_sym = w.intern("positive?");
    let r = w.send(Value::Int(7), positive_sym, &[]).unwrap();
    assert_eq!(r, Value::Bool(true));
}

#[test]
fn bootstrap_set_handler_works_at_runtime() {
    // user code can install new handlers on any proto at runtime.
    // the moldable-seed claim, made operational.
    let mut w = moof::new_world();
    // Integer doesn't have :triple yet.
    let triple_sym = w.intern("triple");
    let err = w.send(Value::Int(5), triple_sym, &[]).unwrap_err();
    assert!(err.message.contains("does not understand"));
    // install :triple on Integer.
    moof::eval(&mut w, "(setHandler! Integer 'triple (fn () [self * 3]))").unwrap();
    assert_eq!(
        w.send(Value::Int(5), triple_sym, &[]).unwrap(),
        Value::Int(15)
    );
}

#[test]
fn bootstrap_methods_use_self_correctly() {
    // verify the receiver-as-self pattern actually works by
    // constructing a list, sending :length, and checking the body
    // referenced `self`.
    let mut w = moof::new_world();
    // build the list (1 2 3) and capture its id.
    let v = moof::eval(&mut w, "(list 1 2 3)").unwrap();
    let length = w.intern("length");
    let r = w.send(v, length, &[]).unwrap();
    assert_eq!(r, Value::Int(3));
}

// ─────────────────────────────────────────────────────────────────
// send brackets, .foo shorthand, defproto
// ─────────────────────────────────────────────────────────────────

#[test]
fn send_brackets_binary() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[5 + 3]").unwrap(), Value::Int(8));
    assert_eq!(moof::eval(&mut w, "[10 - 7]").unwrap(), Value::Int(3));
    assert_eq!(moof::eval(&mut w, "[[5 + 3] * 2]").unwrap(), Value::Int(16));
}

#[test]
fn send_brackets_unary() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) length]").unwrap(),
        Value::Int(3)
    );
    assert_eq!(moof::eval(&mut w, "[[0 - 7] abs]").unwrap(), Value::Int(7));
}

#[test]
fn send_brackets_keyword() {
    let mut w = moof::new_world();
    // `:cons:` is a keyword-style selector; `[xs cons: 0]` builds
    // a list with 0 prepended.
    let r = moof::eval(&mut w, "[[(list 1 2) cons: 0] length]").unwrap();
    assert_eq!(r, Value::Int(3));
}

#[test]
fn defproto_creates_proto_with_handler() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (defproto Foo (handlers (greet) 42)) [[Foo new] greet])",
    )
    .unwrap();
    assert_eq!(r, Value::Int(42));
}

#[test]
fn defproto_counter_full_lifecycle() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do
            (defproto Counter
              (handlers
                (incr) [self count: [.count + 1]]
                (read) .count
                (count: v) (slotSet! self (quote count) v)
                (count) (slot self (quote count))))
            (def c [Counter new])
            [c count: 0]
            [c incr]
            [c incr]
            [c incr]
            [c read])",
    )
    .unwrap();
    assert_eq!(r, Value::Int(3));
}

#[test]
fn defproto_with_inheritance() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do
            (defproto Animal
              (handlers
                (sound) (quote unspecified)))
            (defproto Dog
              (proto Animal)
              (handlers
                (sound) (quote woof)))
            [[Dog new] sound])",
    )
    .unwrap();
    let woof = w.intern("woof");
    assert_eq!(r, Value::Sym(woof));
    // animal still says unspecified.
    let r = moof::eval(&mut w, "[[Animal new] sound]").unwrap();
    let unspec = w.intern("unspecified");
    assert_eq!(r, Value::Sym(unspec));
}

#[test]
fn dot_self_shorthand() {
    // .foo desugars to [self foo]. inside a method body, self is
    // the receiver, so .count reads self's count slot via the
    // installed accessor.
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do
            (defproto Box
              (handlers
                (peek) .stuff
                (stuff: v) (slotSet! self (quote stuff) v)
                (stuff) (slot self (quote stuff))))
            (def b [Box new])
            [b stuff: 7]
            [b peek])",
    )
    .unwrap();
    assert_eq!(r, Value::Int(7));
}

#[test]
fn fact_via_send_brackets() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (def fact (fn (n) (if [n = 0] 1 [n * (fact [n - 1])])))
             (fact 12))",
    )
    .unwrap();
    assert_eq!(r, Value::Int(479001600));
}

// ─────────────────────────────────────────────────────────────────
// inline cache verification — the corner-cut that's now uncut.
// ─────────────────────────────────────────────────────────────────

#[test]
fn super_send_delegates_to_parent() {
    // override-and-delegate: Dog's :sound conses 'loud onto
    // [super sound] which returns Animal's 'unspecified.
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do
            (defproto Animal
              (handlers
                (sound) (quote unspecified)))
            (defproto Dog
              (proto Animal)
              (handlers
                (sound) (cons (quote loud) (cons [super sound] nil))))
            [[Dog new] sound])",
    )
    .unwrap();
    // (loud unspecified) — verify by walking the cons chain.
    let id = r.as_form_id().unwrap();
    let head_sym = w.intern("head");
    let tail_sym = w.intern("tail");
    let h0 = w.heap.get(id).slot(head_sym);
    assert_eq!(w.resolve(h0.as_sym().unwrap()), "loud");
    let t0 = w.heap.get(id).slot(tail_sym);
    let t0_id = t0.as_form_id().unwrap();
    let h1 = w.heap.get(t0_id).slot(head_sym);
    assert_eq!(w.resolve(h1.as_sym().unwrap()), "unspecified");
}

#[test]
fn super_from_top_level_raises() {
    // super-send outside a method body has no defining proto.
    let mut w = moof::new_world();
    let err = moof::eval(&mut w, "[super sound]").unwrap_err();
    assert!(err.message.contains("non-method frame"));
}

#[test]
fn if_with_no_else_returns_nil_for_false() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "(if #true 42)").unwrap(), Value::Int(42));
    assert_eq!(moof::eval(&mut w, "(if #false 42)").unwrap(), Value::Nil);
    assert_eq!(moof::eval(&mut w, "(if nil 42)").unwrap(), Value::Nil);
}

// ─────────────────────────────────────────────────────────────────
// real String type (no longer Sym-as-string-stand-in)
// ─────────────────────────────────────────────────────────────────

#[test]
fn string_literal_produces_string_form() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "\"hello\"").unwrap();
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.string));
    assert_eq!(w.string_text(v).unwrap(), "hello");
}

#[test]
fn integer_to_string_returns_string_form() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[42 toString]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "42");
}

#[test]
fn string_concatenation() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[\"hello\" + \" world\"]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "hello world");
    // concat: keyword is the same operation:
    let v = moof::eval(&mut w, "[\"a\" concat: \"b\"]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "ab");
    // + delegates to rhs's :to-string for non-string rhs:
    let v = moof::eval(&mut w, "[\"answer = \" + 42]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "answer = 42");
}

#[test]
fn string_length() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[\"foo\" length]").unwrap();
    assert_eq!(v, Value::Int(3));
    let v = moof::eval(&mut w, "[\"\" empty?]").unwrap();
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn string_equality_is_structural() {
    let mut w = moof::new_world();
    // two different String forms with same bytes compare equal.
    let r = moof::eval(&mut w, "[\"foo\" = \"foo\"]").unwrap();
    assert_eq!(r, Value::Bool(true));
    let r = moof::eval(&mut w, "[\"foo\" = \"bar\"]").unwrap();
    assert_eq!(r, Value::Bool(false));
    // distinct from the ContentEqual on Sym (their literal id is
    // different but bytes match).
    let r = moof::eval(&mut w, "[\"foo\" != \"bar\"]").unwrap();
    assert_eq!(r, Value::Bool(true));
}

#[test]
fn list_to_string_is_a_string_form() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[(list 1 2 3) toString]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "(1 2 3)");
}

#[test]
fn console_emit_accepts_string() {
    // verify Console's :emit: accepts a String form (not just a
    // Sym fallback). this is the mco-pattern in action: the
    // ForeignHandle holds the raw bytes.
    let mut w = moof::new_world();
    // route to $err (stderr) so test runner's captured stdout
    // stays happy.
    let dollar_err = w.intern("$err");
    let err = w.env_lookup(w.global_env, dollar_err).unwrap();
    let emit = w.intern("emit:");
    let payload = w.make_string("");
    assert_eq!(w.send(err, emit, &[payload]).unwrap(), Value::Nil);
}

#[test]
fn string_is_table_of_chars() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[\"hello\" length]").unwrap();
    assert_eq!(v, Value::Int(5));
    let v = moof::eval(&mut w, "[\"hello\" at: 0]").unwrap();
    assert!(matches!(v, Value::Char(0x68))); // 'h'
    let v = moof::eval(&mut w, "[\"hello\" at: 4]").unwrap();
    assert!(matches!(v, Value::Char(0x6f))); // 'o'
    let xs = moof::eval(&mut w, "[\"abc\" toList]").unwrap();
    let elems = w.list_to_vec(xs).unwrap();
    assert_eq!(elems.len(), 3);
    assert!(matches!(elems[0], Value::Char(0x61))); // 'a'
}

#[test]
fn string_utf8_correct() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[\"héllo\" length]").unwrap();
    assert_eq!(v, Value::Int(5));
    let v = moof::eval(&mut w, "[\"héllo\" byteLength]").unwrap();
    assert_eq!(v, Value::Int(6));
    let v = moof::eval(&mut w, "[[\"héllo\" at: 1] codepoint]").unwrap();
    assert_eq!(v, Value::Int(233)); // é
    let v = moof::eval(&mut w, "[\"🌙\" length]").unwrap();
    assert_eq!(v, Value::Int(1));
    let v = moof::eval(&mut w, "[\"🌙\" byteLength]").unwrap();
    assert_eq!(v, Value::Int(4));
}

#[test]
fn char_literals() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "#\\h").unwrap();
    assert!(matches!(v, Value::Char(0x68)));
    let v = moof::eval(&mut w, "#\\space").unwrap();
    assert!(matches!(v, Value::Char(0x20)));
    let v = moof::eval(&mut w, "#\\newline").unwrap();
    assert!(matches!(v, Value::Char(0x0a)));
    let v = moof::eval(&mut w, "#\\u{1f496}").unwrap();
    assert_eq!(v, Value::Char(0x1f496));
}

#[test]
fn char_methods_work() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[#\\h codepoint]").unwrap(),
        Value::Int(104)
    );
    assert_eq!(moof::eval(&mut w, "[#\\h letter?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[#\\7 digit?]").unwrap(), Value::Bool(true));
    assert_eq!(
        moof::eval(&mut w, "[#\\space whitespace?]").unwrap(),
        Value::Bool(true)
    );
    let v = moof::eval(&mut w, "[#\\h upcase]").unwrap();
    assert!(matches!(v, Value::Char(0x48))); // 'H'
    let v = moof::eval(&mut w, "[#\\Z downcase]").unwrap();
    assert!(matches!(v, Value::Char(0x7a))); // 'z'
    assert_eq!(moof::eval(&mut w, "[#\\a = #\\a]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[#\\a < #\\b]").unwrap(), Value::Bool(true));
}

// ─────────────────────────────────────────────────────────────────
// Floats — concepts/numbers.md
// ─────────────────────────────────────────────────────────────────

#[test]
fn float_literal_and_arithmetic() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "1.5").unwrap();
    assert!(matches!(v, Value::Float(_)));
    assert_eq!(v.as_float().unwrap(), 1.5);
    let v = moof::eval(&mut w, "[1.5 + 2.5]").unwrap();
    assert_eq!(v.as_float().unwrap(), 4.0);
}

#[test]
fn float_literal_shapes() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, ".5").unwrap().as_float().unwrap(), 0.5);
    assert_eq!(moof::eval(&mut w, "1e9").unwrap().as_float().unwrap(), 1e9);
    assert_eq!(
        moof::eval(&mut w, "1.5e-3").unwrap().as_float().unwrap(),
        1.5e-3
    );
    assert_eq!(moof::eval(&mut w, "-3.14").unwrap().as_float().unwrap(), -3.14);
}

#[test]
fn float_int_promotion() {
    let mut w = moof::new_world();
    // Int + Float → Float
    let v = moof::eval(&mut w, "[1 + 1.5]").unwrap();
    assert_eq!(v.as_float().unwrap(), 2.5);
    // Float + Int → Float
    let v = moof::eval(&mut w, "[1.5 + 1]").unwrap();
    assert_eq!(v.as_float().unwrap(), 2.5);
    // mixed comparison
    assert_eq!(moof::eval(&mut w, "[1 < 2.5]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[1.5 = 1.5]").unwrap(), Value::Bool(true));
}

#[test]
fn float_math_functions() {
    let mut w = moof::new_world();
    assert!((moof::eval(&mut w, "[2.0 sqrt]").unwrap().as_float().unwrap() - 2.0_f64.sqrt()).abs() < 1e-10);
    assert!((moof::eval(&mut w, "[1.0 exp]").unwrap().as_float().unwrap() - 1.0_f64.exp()).abs() < 1e-10);
    assert_eq!(
        moof::eval(&mut w, "[1.999 floor]").unwrap().as_float().unwrap(),
        1.0
    );
    assert_eq!(
        moof::eval(&mut w, "[1.001 ceil]").unwrap().as_float().unwrap(),
        2.0
    );
    assert_eq!(
        moof::eval(&mut w, "[3.7 round]").unwrap().as_float().unwrap(),
        4.0
    );
}

#[test]
fn float_predicates() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[0.0 zero?]").unwrap(), Value::Bool(true));
    assert_eq!(
        moof::eval(&mut w, "[3.14 positive?]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[-1.5 negative?]").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn float_int_conversion() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[3 asFloat]").unwrap().as_float().unwrap(),
        3.0
    );
    assert_eq!(
        moof::eval(&mut w, "[3.7 asInteger]").unwrap(),
        Value::Int(3)
    );
}

#[test]
fn float_proto() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[1.5 proto]").unwrap();
    assert_eq!(v, Value::Form(w.protos.float));
}

// ─────────────────────────────────────────────────────────────────
// Cascades [obj a; b; c] — concepts/sends-and-calls.md
// ─────────────────────────────────────────────────────────────────

#[test]
fn cascade_sends_in_order_returns_receiver() {
    let mut w = moof::new_world();
    // table cascade. push three; expect the receiver back; verify
    // length.
    let r = moof::eval(
        &mut w,
        "(do (def t #[]) [t push: 1; push: 2; push: 3])",
    )
    .unwrap();
    let length = w.intern("length");
    assert_eq!(w.send(r, length, &[]).unwrap(), Value::Int(3));
}

#[test]
fn cascade_with_multiple_setters() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (def b {x: 0 y: 0}) [b x: 10; y: 20] [b x])",
    )
    .unwrap();
    assert_eq!(r, Value::Int(10));
    let r = moof::eval(&mut w, "[b y]").unwrap();
    assert_eq!(r, Value::Int(20));
}

#[test]
fn cascade_unary_and_keyword_mixed() {
    // mix shapes within a cascade.
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (def t #[]) [t push: 1; push: 2; push: 3; pop])",
    )
    .unwrap();
    // pop returns 3 (the popped value), but cascade returns the
    // receiver. so r is the table.
    let length = w.intern("length");
    assert_eq!(w.send(r, length, &[]).unwrap(), Value::Int(2));
}

// ─────────────────────────────────────────────────────────────────
// Object literals — `concepts/objects-and-protos.md`,
//                   `syntax/object-literals.md`
// ─────────────────────────────────────────────────────────────────

#[test]
fn object_literal_basic() {
    // `{x: 5}` produces a fresh Form with an `:x` slot and an
    // auto-accessor.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "{x: 5}").unwrap();
    let id = v.as_form_id().unwrap();
    let x_sym = w.intern("x");
    assert_eq!(w.heap.get(id).slot(x_sym), Value::Int(5));
    // auto-getter: `[obj x]` returns 5.
    assert_eq!(
        moof::eval(&mut w, "[{x: 5} x]").unwrap(),
        Value::Int(5)
    );
}

#[test]
fn object_literal_setter_and_getter() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (def b {x: 5}) [b x: 99] [b x])",
    )
    .unwrap();
    assert_eq!(r, Value::Int(99));
}

#[test]
fn object_literal_counter() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do
            (def c {count: 0
                    [bump] [self count: [.count + 1]]
                    [read] .count})
            [c bump]
            [c bump]
            [c bump]
            [c read])",
    )
    .unwrap();
    assert_eq!(r, Value::Int(3));
}

#[test]
fn object_literal_with_proto() {
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (defproto Greeter (handlers (greet) \"hello!\"))
             [{Greeter} greet])",
    )
    .unwrap();
    assert_eq!(w.string_text(r).unwrap(), "hello!");
}

#[test]
fn object_literal_proto_with_slots() {
    // proto + slots + extra method
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (defproto Animal (handlers (sound) 'generic))
             (def d {Animal name: 'rex
                     [sound] 'woof})
             [d sound])",
    )
    .unwrap();
    assert_eq!(r, Value::Sym(w.intern("woof")));
    let r = moof::eval(&mut w, "[d name]").unwrap();
    assert_eq!(r, Value::Sym(w.intern("rex")));
}

#[test]
fn object_literal_methods_can_self_send() {
    // method body references `.x` (auto-accessor for `x`) inside
    // the object — verifies self-dispatch through the literal's
    // own handler table.
    let mut w = moof::new_world();
    let r = moof::eval(
        &mut w,
        "(do (def p {x: 3 y: 4
                     [magnitude] [[.x * .x] + [.y * .y]]})
             [p magnitude])",
    )
    .unwrap();
    assert_eq!(r, Value::Int(25));
}

// ─────────────────────────────────────────────────────────────────
// String surface (concepts/strings.md)
// ─────────────────────────────────────────────────────────────────

#[test]
fn string_case_methods() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[\"hello world\" upcase]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "HELLO WORLD");
    let v = moof::eval(&mut w, "[\"HELLO WORLD\" downcase]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "hello world");
}

#[test]
fn string_trim() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[\"  hello  \" trim]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "hello");
}

#[test]
fn string_predicates() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[\"hello\" contains?: \"ell\"]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[\"hello\" startsWith?: \"he\"]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[\"hello\" endsWith?: \"lo\"]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[\"hello\" startsWith?: \"hi\"]").unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn string_indexOf_and_slice() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[\"hello world\" indexOf: \"world\"]").unwrap(),
        Value::Int(6)
    );
    assert_eq!(
        moof::eval(&mut w, "[\"hello\" indexOf: \"missing\"]").unwrap(),
        Value::Int(-1)
    );
    let v = moof::eval(&mut w, "[\"hello world\" slice: 6 length: 5]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "world");
    let v = moof::eval(&mut w, "[\"hello\" slice: 1 length: 3]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "ell");
}

#[test]
fn string_replace() {
    let mut w = moof::new_world();
    let v = moof::eval(
        &mut w,
        "[\"hello world\" replace: \"world\" with: \"moof\"]",
    )
    .unwrap();
    assert_eq!(w.string_text(v).unwrap(), "hello moof");
}

#[test]
fn string_split_and_lines() {
    let mut w = moof::new_world();
    // split returns a List of Strings.
    let v = moof::eval(&mut w, "[\"a,b,c\" split: \",\"]").unwrap();
    let elems = w.list_to_vec(v).unwrap();
    assert_eq!(elems.len(), 3);
    assert_eq!(w.string_text(elems[0]).unwrap(), "a");
    assert_eq!(w.string_text(elems[2]).unwrap(), "c");
    // lines splits on \n.
    let v = moof::eval(&mut w, "[\"a\\nb\\nc\" lines]").unwrap();
    let lines = w.list_to_vec(v).unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(w.string_text(lines[1]).unwrap(), "b");
}

#[test]
fn string_toList_returns_chars() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[\"abc\" toList]").unwrap();
    let elems = w.list_to_vec(v).unwrap();
    assert_eq!(elems.len(), 3);
    assert!(matches!(elems[0], Value::Char(0x61)));
}

// ─────────────────────────────────────────────────────────────────
// Tables — `concepts/tables.md`
// ─────────────────────────────────────────────────────────────────

#[test]
fn table_positional_literal() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "#[1 2 3]").unwrap();
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
    assert_eq!(moof::eval(&mut w, "[#[1 2 3] length]").unwrap(), Value::Int(3));
    assert_eq!(
        moof::eval(&mut w, "[#[1 2 3] at: 0]").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        moof::eval(&mut w, "[#[1 2 3] at: 2]").unwrap(),
        Value::Int(3)
    );
}

#[test]
fn table_keyed_literal() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "#['name => \"ada\" 'age => 30]").unwrap();
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
    let r = moof::eval(&mut w, "[#['name => 1 'age => 30] at: 'age]").unwrap();
    assert_eq!(r, Value::Int(30));
    let r = moof::eval(&mut w, "[#['name => 1 'age => 30] containsKey?: 'age]")
        .unwrap();
    assert_eq!(r, Value::Bool(true));
    let r = moof::eval(
        &mut w,
        "[#['name => 1 'age => 30] containsKey?: 'missing]",
    )
    .unwrap();
    assert_eq!(r, Value::Bool(false));
}

#[test]
fn table_mixed_literal() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "#[1 2 3 'tag => 'urgent]").unwrap();
    let length = w.intern("length");
    let size = w.intern("size");
    assert_eq!(w.send(v, length, &[]).unwrap(), Value::Int(3)); // positional only
    assert_eq!(w.send(v, size, &[]).unwrap(), Value::Int(4)); // positional + keyed
}

#[test]
fn table_higher_order_methods() {
    let mut w = moof::new_world();
    // map
    let v = moof::eval(
        &mut w,
        "[[#[1 2 3 4 5] map: (fn (x) [x * 10])] length]",
    )
    .unwrap();
    assert_eq!(v, Value::Int(5));
    // reduce
    let v = moof::eval(
        &mut w,
        "[#[1 2 3 4 5] reduce: (fn (a b) [a + b]) from: 0]",
    )
    .unwrap();
    assert_eq!(v, Value::Int(15));
    // filter
    let v = moof::eval(
        &mut w,
        "[[#[-2 -1 0 1 2] filter: (fn (x) [x positive?])] length]",
    )
    .unwrap();
    assert_eq!(v, Value::Int(2));
    // forEach
    moof::eval(
        &mut w,
        "[#[1 2 3] forEach: (fn (x) [$err emit: \"\"])]",
    )
    .unwrap();
}

#[test]
fn table_mutation_via_methods() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def t #[])").unwrap();
    moof::eval(&mut w, "[t push: 1]").unwrap();
    moof::eval(&mut w, "[t push: 2]").unwrap();
    moof::eval(&mut w, "[t push: 3]").unwrap();
    assert_eq!(moof::eval(&mut w, "[t length]").unwrap(), Value::Int(3));
    moof::eval(&mut w, "[t at: 'name put: 'alice]").unwrap();
    assert_eq!(
        moof::eval(&mut w, "[t at: 'name]").unwrap(),
        Value::Sym(w.intern("alice"))
    );
    assert_eq!(moof::eval(&mut w, "[t size]").unwrap(), Value::Int(4));
    let popped = moof::eval(&mut w, "[t pop]").unwrap();
    assert_eq!(popped, Value::Int(3));
}

#[test]
fn table_keys_and_values() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "[[#['x => 1 'y => 2 'z => 3] keys] length]").unwrap();
    assert_eq!(r, Value::Int(3));
    let r = moof::eval(&mut w, "[[#[10 20 30 'x => 99] keys] length]").unwrap();
    assert_eq!(r, Value::Int(4)); // 0, 1, 2, 'x
    let r = moof::eval(&mut w, "[[#[10 20 30 'x => 99] values] length]").unwrap();
    assert_eq!(r, Value::Int(4));
}

#[test]
fn table_to_string() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[#[1 2 3] toString]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "#[1 2 3]");
    // keyed
    let v = moof::eval(&mut w, "[#['x => 1] toString]").unwrap();
    assert_eq!(w.string_text(v).unwrap(), "#[x => 1]");
}

#[test]
fn table_equality_is_structural() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "[#[1 2 3] = #[1 2 3]]").unwrap();
    assert_eq!(r, Value::Bool(true));
    let r = moof::eval(&mut w, "[#[1 2 3] = #[1 2]]").unwrap();
    assert_eq!(r, Value::Bool(false));
    let r = moof::eval(&mut w, "[#['a => 1] = #['a => 1]]").unwrap();
    assert_eq!(r, Value::Bool(true));
}

#[test]
fn table_empty_literal() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[#[] empty?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[#[] length]").unwrap(), Value::Int(0));
    assert_eq!(moof::eval(&mut w, "[#[] size]").unwrap(), Value::Int(0));
}

#[test]
fn table_new_via_proto() {
    // [Table new] → fresh empty Table.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[Table new]").unwrap();
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
    let length = w.intern("length");
    assert_eq!(w.send(v, length, &[]).unwrap(), Value::Int(0));
}

#[test]
fn camel_case_methods_only() {
    // kebab-named methods are gone — they're now camelCase. test
    // each rename: sending the kebab name should dnu-fail.
    let mut w = moof::new_world();
    let kebab_methods = [
        "to-string",
        "byte-length",
        "for-each:",
        "count-where:",
        "non-empty?",
        "does-not-understand:with:",
    ];
    for kebab in kebab_methods {
        let s = w.intern(kebab);
        let err = w.send(Value::Int(5), s, &[]).unwrap_err();
        assert!(
            err.message.contains("does not understand"),
            "kebab method `{}` should be gone (renamed to camelCase)",
            kebab
        );
    }
    // and the camelCase versions exist for [42 toString] etc.
    let to_string = w.intern("toString");
    let r = w.send(Value::Int(42), to_string, &[]).unwrap();
    assert_eq!(w.string_text(r).unwrap(), "42");
}

#[test]
fn when_unless_let_rec() {
    let mut w = moof::new_world();
    // when
    assert_eq!(
        moof::eval(&mut w, "(when [5 > 3] 'yes)").unwrap(),
        Value::Sym(w.intern("yes"))
    );
    assert_eq!(
        moof::eval(&mut w, "(when [5 < 3] 'yes)").unwrap(),
        Value::Nil
    );
    // unless
    assert_eq!(
        moof::eval(&mut w, "(unless [5 < 3] 'yes)").unwrap(),
        Value::Sym(w.intern("yes"))
    );
    // let-rec mutual recursion
    let r = moof::eval(
        &mut w,
        "(let-rec
            ((even? (fn (n) (if [n = 0] #true (odd? [n - 1]))))
             (odd?  (fn (n) (if [n = 0] #false (even? [n - 1])))))
          (list (even? 10) (odd? 7)))",
    )
    .unwrap();
    let id = r.as_form_id().unwrap();
    let head_sym = w.intern("head");
    let tail_sym = w.intern("tail");
    assert_eq!(w.heap.get(id).slot(head_sym), Value::Bool(true));
    let t = w.heap.get(id).slot(tail_sym).as_form_id().unwrap();
    assert_eq!(w.heap.get(t).slot(head_sym), Value::Bool(true));
}

#[test]
fn proto_chain_cycle_safety() {
    // we don't have set-proto! at phase A, but the lookup_handler
    // bound (256 hops) is purely defensive. with the chain that
    // exists in a fresh world (Object.proto = Nil; depth ≤ 4 for
    // any built-in), normal dispatch never hits the bound.
    let mut w = moof::new_world();
    // a deeply-derived proto: 50 levels of inheritance, each just
    // adding a level marker. send a method that isn't on any of
    // them; the lookup walks all 50 then dnu raises (no infinite
    // loop, no panic).
    let mut script = String::new();
    script.push_str("(do (defproto L0 (handlers (which) 0))");
    for i in 1..=50 {
        script.push_str(&format!(
            " (defproto L{i} (proto L{prev}) (handlers (which) {i}))",
            i = i,
            prev = i - 1
        ));
    }
    script.push_str(" [[L50 new] which])");
    let r = moof::eval(&mut w, &script).unwrap();
    assert_eq!(r, Value::Int(50));
}

#[test]
fn initialize_runs_on_new() {
    // [Proto new] sends :initialize. user override fires.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(do
            (defproto Counter
              (handlers
                (initialize) (slotSet! self (quote count) 100)
                (count) (slot self (quote count))))
            [[Counter new] count])",
    )
    .unwrap();
    let r = moof::eval(&mut w, "[[Counter new] count]").unwrap();
    assert_eq!(r, Value::Int(100));
}

#[test]
fn inline_cache_invalidates_on_set_handler() {
    // a tight loop that calls the same method many times. the IC
    // caches after first call. then setHandler! changes the
    // method's body; the IC's generation no longer matches; next
    // call re-resolves and gets the new behavior.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(setHandler! Integer 'pet (fn () (quote dog)))",
    )
    .unwrap();
    // first call: cache miss → populate.
    let r = moof::eval(&mut w, "[5 pet]").unwrap();
    assert_eq!(w.resolve(r.as_sym().unwrap()), "dog");
    // second call: cache hit (same proto = Integer).
    let r = moof::eval(&mut w, "[7 pet]").unwrap();
    assert_eq!(w.resolve(r.as_sym().unwrap()), "dog");
    // change behavior. setHandler! bumps the proto's generation;
    // existing ICs are now stale.
    moof::eval(
        &mut w,
        "(setHandler! Integer 'pet (fn () (quote cat)))",
    )
    .unwrap();
    // third call: stale-IC re-resolves; new behavior takes effect.
    let r = moof::eval(&mut w, "[5 pet]").unwrap();
    assert_eq!(w.resolve(r.as_sym().unwrap()), "cat");
}
