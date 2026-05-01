//! doc-alignment regression suite.
//!
//! these tests lock in surfaces the docs in `docs/concepts/` and
//! `docs/laws/` promise but the impl had silently lacked. each test
//! cites the doc paragraph it pins down.
//!
//! anything that would be a "the docs are aspirational here" caveat
//! should ideally fail until either the impl catches up or the docs
//! retract. for now, this file holds the *kept* promises.

use moof::value::Value;

// ─────────────────────────────────────────────────────────────────
// laws/reflection-contract.md R1 — every Form responds to the basic
// reflection protocol. R7 — meta is extensible (so it must be a
// Table, not a list-of-pairs).
// ─────────────────────────────────────────────────────────────────

#[test]
fn protos_returns_full_chain() {
    // R1: `[v protos]` returns the full delegation chain.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[5 protos]").unwrap();
    // [5 protos] → (Integer Object). Integer's proto is Object;
    // Object's proto is nil so the chain stops there.
    let chain = w.list_to_vec(v).unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0], Value::Form(w.protos.integer));
    assert_eq!(chain[1], Value::Form(w.protos.object));
}

#[test]
fn protos_terminates_at_object() {
    // Object's :protos is the empty list — its proto is nil.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "[Object protos]").unwrap();
    assert_eq!(v, Value::Nil);
}

#[test]
fn slots_returns_a_table() {
    // forms.md, reflection-contract.md R7: :slots is a Table
    // keyed by slot-name. mutating the returned Table doesn't
    // affect the receiver (snapshot semantics in phase A; live
    // views are phase C).
    let mut w = moof::new_world();
    moof::eval(&mut w, "(defproto Counter (slots count step))").unwrap();
    let _ = moof::eval(&mut w, "(def c [Counter new])").unwrap();
    moof::eval(&mut w, "(slotSet! c 'count 5)").unwrap();
    moof::eval(&mut w, "(slotSet! c 'step 1)").unwrap();
    let v = moof::eval(&mut w, "[c slots]").unwrap();
    // it's a Table; sending :at: works.
    assert_eq!(
        moof::eval(&mut w, "[[c slots] at: 'count]").unwrap(),
        Value::Int(5),
    );
    assert_eq!(
        moof::eval(&mut w, "[[c slots] at: 'step]").unwrap(),
        Value::Int(1),
    );
    // it's a Table-proto Form.
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
}

#[test]
fn handlers_returns_a_table() {
    // reflection-contract.md R1: `[proto handlers]` is a Table.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(defproto Greeter (handlers (greet) 'hello))",
    )
    .unwrap();
    let v = moof::eval(&mut w, "[Greeter handlers]").unwrap();
    let id = v.as_form_id().unwrap();
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
    // the :greet handler is in there.
    let greet_present =
        moof::eval(&mut w, "[[Greeter handlers] containsKey?: 'greet]").unwrap();
    assert_eq!(greet_present, Value::Bool(true));
}

// ─────────────────────────────────────────────────────────────────
// concepts/lists.md — `[xs at: i]`, structural :=, :count, :zip:.
// ─────────────────────────────────────────────────────────────────

#[test]
fn list_indexing_is_o_n_but_works() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 'a 'b 'c) at: 0]").unwrap(),
        Value::Sym(w.intern("a")),
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 'a 'b 'c) at: 2]").unwrap(),
        Value::Sym(w.intern("c")),
    );
}

#[test]
fn list_equality_is_structural() {
    // distinct cons-cells with the same content compare equal.
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) = (list 1 2 3)]").unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) = (list 1 2 4)]").unwrap(),
        Value::Bool(false),
    );
    // empty matches empty.
    assert_eq!(
        moof::eval(&mut w, "[nil = nil]").unwrap(),
        Value::Bool(true),
    );
    // length-mismatched lists are unequal.
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2) = (list 1 2 3)]").unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        moof::eval(&mut w, "[(list 1 2 3) = (list 1 2)]").unwrap(),
        Value::Bool(false),
    );
}

#[test]
fn list_count_and_zip() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[(list 'a 'b 'c) count]").unwrap(),
        Value::Int(3),
    );
    // zip stops at the shorter list.
    let v = moof::eval(
        &mut w,
        "[(list 1 2 3) zip: (list 'a 'b)]",
    )
    .unwrap();
    let pairs = w.list_to_vec(v).unwrap();
    assert_eq!(pairs.len(), 2);
}

#[test]
fn list_scan_is_running_fold() {
    let mut w = moof::new_world();
    let v = moof::eval(
        &mut w,
        "[(list 1 2 3 4) scan: (fn (a b) [a + b]) from: 0]",
    )
    .unwrap();
    let xs = w.list_to_vec(v).unwrap();
    assert_eq!(xs, vec![Value::Int(1), Value::Int(3), Value::Int(6), Value::Int(10)]);
}

// ─────────────────────────────────────────────────────────────────
// concepts/numbers.md — :negate, :integer?, :rational?, :real?,
// :mod:, :clamp:to:.
// ─────────────────────────────────────────────────────────────────

#[test]
fn integer_tower_predicates() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[5 integer?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[5 rational?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[5 real?]").unwrap(), Value::Bool(true));
    assert_eq!(moof::eval(&mut w, "[5 nan?]").unwrap(), Value::Bool(false));
    assert_eq!(moof::eval(&mut w, "[5 finite?]").unwrap(), Value::Bool(true));
}

#[test]
fn float_tower_predicates() {
    let mut w = moof::new_world();
    // a Float is real but not integer or rational.
    assert_eq!(moof::eval(&mut w, "[1.5 integer?]").unwrap(), Value::Bool(false));
    assert_eq!(moof::eval(&mut w, "[1.5 rational?]").unwrap(), Value::Bool(false));
    assert_eq!(moof::eval(&mut w, "[1.5 real?]").unwrap(), Value::Bool(true));
}

#[test]
fn negate_works_on_both_kinds() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[5 negate]").unwrap(), Value::Int(-5));
    let v = moof::eval(&mut w, "[1.5 negate]").unwrap();
    assert_eq!(v.as_float().unwrap(), -1.5);
}

#[test]
fn integer_mod_and_clamp() {
    let mut w = moof::new_world();
    assert_eq!(moof::eval(&mut w, "[7 mod: 2]").unwrap(), Value::Int(1));
    assert_eq!(moof::eval(&mut w, "[10 mod: 3]").unwrap(), Value::Int(1));
    // clamp:to: returns lo if below, hi if above, self otherwise.
    assert_eq!(moof::eval(&mut w, "[5 clamp: 0 to: 10]").unwrap(), Value::Int(5));
    assert_eq!(moof::eval(&mut w, "[-3 clamp: 0 to: 10]").unwrap(), Value::Int(0));
    assert_eq!(moof::eval(&mut w, "[42 clamp: 0 to: 10]").unwrap(), Value::Int(10));
}

// ─────────────────────────────────────────────────────────────────
// laws/reflection-contract.md R2 — methods expose :arity, :purity,
// :caps-required, :parameters.
// ─────────────────────────────────────────────────────────────────

#[test]
fn method_reflection_arity() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def f (fn (a b c) [a + b]))").unwrap();
    assert_eq!(moof::eval(&mut w, "[f arity]").unwrap(), Value::Int(3));
    moof::eval(&mut w, "(def g (fn () 42))").unwrap();
    assert_eq!(moof::eval(&mut w, "[g arity]").unwrap(), Value::Int(0));
}

#[test]
fn method_reflection_purity_default() {
    // until the analyzer lands (phase C), :purity defaults to
    // 'unknown — but the contract is honored.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def f (fn () 42))").unwrap();
    assert_eq!(
        moof::eval(&mut w, "[f purity]").unwrap(),
        Value::Sym(w.intern("unknown")),
    );
    // :caps-required defaults to nil (empty list).
    assert_eq!(
        moof::eval(&mut w, "[f caps-required]").unwrap(),
        Value::Nil,
    );
}

// ─────────────────────────────────────────────────────────────────
// concepts/tables.md — :reduce: (no init), :at-or:default:.
// ─────────────────────────────────────────────────────────────────

#[test]
fn table_reduce_no_init_uses_first() {
    let mut w = moof::new_world();
    let v = moof::eval(
        &mut w,
        "[#[1 2 3 4] reduce: (fn (a b) [a + b])]",
    )
    .unwrap();
    assert_eq!(v, Value::Int(10));
    // empty table → nil.
    assert_eq!(
        moof::eval(&mut w, "[#[] reduce: (fn (a b) [a + b])]").unwrap(),
        Value::Nil,
    );
}

#[test]
fn table_at_or_default() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def t #[1 2 3 'name => 'ada])").unwrap();
    assert_eq!(
        moof::eval(&mut w, "[t at-or: 'name default: 'unknown]").unwrap(),
        Value::Sym(w.intern("ada")),
    );
    assert_eq!(
        moof::eval(&mut w, "[t at-or: 'missing default: 'fallback]").unwrap(),
        Value::Sym(w.intern("fallback")),
    );
}

// ─────────────────────────────────────────────────────────────────
// substrate-laws.md L3 dispatch + the proto-receiver edge case.
// ─────────────────────────────────────────────────────────────────

#[test]
fn equality_on_proto_forms_does_not_panic() {
    // sending `=` to a proto-Form (e.g. Integer itself, not an
    // Integer instance) used to panic because Integer's `=` was
    // assuming `self.as_int()` succeeded. now it falls through to
    // identity.
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[Integer = Integer]").unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        moof::eval(&mut w, "[Integer = String]").unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        moof::eval(&mut w, "[Integer = nil]").unwrap(),
        Value::Bool(false),
    );
}

// ─────────────────────────────────────────────────────────────────
// vm.rs PushClosure — captured-self regression.
// ─────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────
// "everything is a Form" (forms.md L1) — control-flow sugar like
// `when` / `unless` / `let*` / `let-rec` / `defmethod` lives as
// moof macros (lib/bootstrap.moof), not as hardcoded special-cases
// in the rust compiler. that means: the user can inspect them,
// re-define them, and override their expansion from inside moof.
// ─────────────────────────────────────────────────────────────────

#[test]
fn when_unless_letstar_letrec_are_user_inspectable_macros() {
    let mut w = moof::new_world();
    // each is a real Method-Form bound in the global env, with
    // `'macro` set in its meta.
    for name in &[
        // control-flow + binding sugar
        "when", "unless", "let*", "let-rec",
        // proto + method definers
        "defmethod", "defproto",
        // source-to-source special forms now in moof
        "quasiquote",
        // reader-emitted markers, also moof macros now
        "__cascade__", "__table__", "__obj__",
    ] {
        let sym = w.intern(name);
        let v = w.env_lookup(w.global_env, sym).unwrap_or_else(|| {
            panic!("expected macro `{}` bound in global env", name)
        });
        let id = v
            .as_form_id()
            .unwrap_or_else(|| panic!("`{}` is not a Form", name));
        // the macro is registered in the canonical Macros Form.
        assert!(
            w.macro_at(sym).is_some(),
            "`{}` must be registered as a macro",
            name
        );
        // it's a Method-Form (closure).
        let proto = w.heap.get(id).proto;
        assert_eq!(proto, Value::Form(w.protos.method));
    }
}

#[test]
fn user_can_redefine_when() {
    // because `when` is a moof macro, redefining it from user code
    // works. (this is the key moldability promise — "everything
    // modifiable lives in moof".)
    let mut w = moof::new_world();
    // re-define `when` to ALWAYS evaluate body, ignoring the cond.
    moof::eval(
        &mut w,
        "(defmacro when (args)
           (let ((body [args tail]))
             `(do ,@body)))",
    )
    .unwrap();
    // now `(when #false 42)` evaluates the body anyway.
    assert_eq!(
        moof::eval(&mut w, "(when #false 42)").unwrap(),
        Value::Int(42),
    );
}

#[test]
fn macroexpand_uses_single_list_convention() {
    // (macroexpand 'form) returns the expansion. matches the new
    // calling convention: macros take one arg = the list of
    // source-arg-Forms.
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(macroexpand '(when c a b))").unwrap();
    // expansion: (if c (do a b) nil)
    let elems = w.list_to_vec(v).unwrap();
    assert_eq!(elems.len(), 4);
    assert_eq!(elems[0], Value::Sym(w.intern("if")));
    assert_eq!(elems[1], Value::Sym(w.intern("c")));
    // elems[2] is (do a b)
    let then_branch = w.list_to_vec(elems[2]).unwrap();
    assert_eq!(then_branch[0], Value::Sym(w.intern("do")));
    assert_eq!(elems[3], Value::Nil);
}

#[test]
fn defproto_via_macro_full_grammar() {
    let mut w = moof::new_world();
    // (proto P) + (slots …) + (handlers (h1) body1 (h2) body2)
    moof::eval(
        &mut w,
        "(defproto Counter
           (slots count step)
           (handlers (incr) [self count: [.count + .step]]
                     (read) .count))",
    )
    .unwrap();
    moof::eval(&mut w, "(def c [Counter new])").unwrap();
    moof::eval(&mut w, "[c count: 0]").unwrap();
    moof::eval(&mut w, "[c step: 5]").unwrap();
    moof::eval(&mut w, "[c incr]").unwrap();
    moof::eval(&mut w, "[c incr]").unwrap();
    assert_eq!(moof::eval(&mut w, "[c read]").unwrap(), Value::Int(10));
    // auto-getter and `slot:`-setter are installed.
    assert_eq!(moof::eval(&mut w, "[c count]").unwrap(), Value::Int(10));
    assert_eq!(moof::eval(&mut w, "[c step]").unwrap(), Value::Int(5));

    // flat header/body form.
    moof::eval(
        &mut w,
        "(defproto Greeter (greet) 'hello (greet: name) name)",
    )
    .unwrap();
    let v = moof::eval(&mut w, "[[Greeter new] greet]").unwrap();
    assert_eq!(v, Value::Sym(w.intern("hello")));
    let v = moof::eval(&mut w, "[[Greeter new] greet: 'world]").unwrap();
    assert_eq!(v, Value::Sym(w.intern("world")));

    // (proto P) clause — inheritance.
    moof::eval(
        &mut w,
        "(defproto BoundedCounter
           (proto Counter)
           (slots max)
           (handlers (incr-by: n)
                     (if [[.count + n] > .max]
                         'overflow
                         (do [self count: [.count + n]] .count))))",
    )
    .unwrap();
    moof::eval(&mut w, "(def b [BoundedCounter new])").unwrap();
    moof::eval(&mut w, "[b count: 0]").unwrap();
    moof::eval(&mut w, "[b step: 1]").unwrap();
    moof::eval(&mut w, "[b max: 10]").unwrap();
    assert_eq!(
        moof::eval(&mut w, "[b incr-by: 5]").unwrap(),
        Value::Int(5),
    );
    assert_eq!(
        moof::eval(&mut w, "[b incr-by: 100]").unwrap(),
        Value::Sym(w.intern("overflow")),
    );
    // inherited :read still works (delegates up to Counter).
    assert_eq!(moof::eval(&mut w, "[b read]").unwrap(), Value::Int(5));
}

#[test]
fn defproto_reopen_preserves_identity() {
    // `(defproto Name …)` twice must reopen the SAME proto-Form.
    // matches getOrCreateProto's spec — used by smalltalk-style
    // class-extend.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(defproto Foo (greet) 1)").unwrap();
    let id1 = moof::eval(&mut w, "[Foo identity]")
        .unwrap()
        .as_int()
        .unwrap();
    moof::eval(&mut w, "(defproto Foo (greet) 2 (extra) 'new)").unwrap();
    let id2 = moof::eval(&mut w, "[Foo identity]")
        .unwrap()
        .as_int()
        .unwrap();
    assert_eq!(id1, id2, "defproto should reopen, not replace");
    // the new method is reachable.
    assert_eq!(
        moof::eval(&mut w, "[[Foo new] extra]").unwrap(),
        Value::Sym(w.intern("new")),
    );
    // the redefined method picks up the new body.
    assert_eq!(
        moof::eval(&mut w, "[[Foo new] greet]").unwrap(),
        Value::Int(2),
    );
}

#[test]
fn defmethod_via_macro_installs_handler() {
    let mut w = moof::new_world();
    // single-symbol header.
    moof::eval(
        &mut w,
        "(defmethod Integer (squared) [self * self])",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "[5 squared]").unwrap(), Value::Int(25));

    // binary-operator header.
    moof::eval(
        &mut w,
        "(defmethod Integer (~ other) [self + [other * 100]])",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[3 ~ 5]").unwrap(),
        Value::Int(503),
    );

    // keyword header — the single-keyword case.
    moof::eval(
        &mut w,
        "(defmethod Integer (multipliedBy: n) [self * n])",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[7 multipliedBy: 3]").unwrap(),
        Value::Int(21),
    );

    // keyword header — multi-keyword needs intern at expand time.
    moof::eval(
        &mut w,
        "(defmethod Integer (between?: lo and: hi)
           (if [self < lo] #false [self <= hi]))",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[5 between?: 0 and: 10]").unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        moof::eval(&mut w, "[15 between?: 0 and: 10]").unwrap(),
        Value::Bool(false),
    );
}

// ─────────────────────────────────────────────────────────────────
// the `(intern …)` primitive — required by macros that build
// keyword selectors at expansion time.
// ─────────────────────────────────────────────────────────────────

#[test]
fn quasiquote_macro_round_trips_basic_shapes() {
    // quasiquote is now a moof macro. test that the basic
    // expander still works for atoms, lists, unquotes, splices.
    let mut w = moof::new_world();
    // atom → (quote atom)
    let v = moof::eval(&mut w, "`5").unwrap();
    assert_eq!(v, Value::Int(5));
    // list with unquote
    let v = moof::eval(&mut w, "(let ((x 7)) `(a ,x b))").unwrap();
    let xs = w.list_to_vec(v).unwrap();
    assert_eq!(xs.len(), 3);
    assert_eq!(xs[0], Value::Sym(w.intern("a")));
    assert_eq!(xs[1], Value::Int(7));
    assert_eq!(xs[2], Value::Sym(w.intern("b")));
    // splice
    let v = moof::eval(
        &mut w,
        "(let ((xs '(1 2 3))) `(a ,@xs b))",
    )
    .unwrap();
    let ys = w.list_to_vec(v).unwrap();
    assert_eq!(ys.len(), 5);
    assert_eq!(ys[0], Value::Sym(w.intern("a")));
    assert_eq!(ys[1], Value::Int(1));
    assert_eq!(ys[2], Value::Int(2));
    assert_eq!(ys[3], Value::Int(3));
    assert_eq!(ys[4], Value::Sym(w.intern("b")));
}

#[test]
fn cascade_table_obj_literals_are_macros() {
    // make sure the surface syntaxes still work after their
    // markers became macros instead of compiler special-forms.
    let mut w = moof::new_world();

    // cascade: returns the receiver after each segment-send.
    let v = moof::eval(
        &mut w,
        "(let ((t #[]))
           (do [t push: 1 ; push: 2 ; push: 3]
               [t length]))",
    )
    .unwrap();
    assert_eq!(v, Value::Int(3));

    // table literal — both positional and keyed entries.
    let v = moof::eval(
        &mut w,
        "[#[1 2 3 'name => 'ada] size]",
    )
    .unwrap();
    assert_eq!(v, Value::Int(4));
    let v = moof::eval(
        &mut w,
        "[#['name => 'ada 'age => 30] at: 'name]",
    )
    .unwrap();
    assert_eq!(v, Value::Sym(w.intern("ada")));

    // object literal — slots + auto-accessors + custom method.
    moof::eval(
        &mut w,
        "(defproto Box (slots contents))",
    )
    .unwrap();
    let v = moof::eval(
        &mut w,
        "(let ((b {Box contents: 42 [doubled] [.contents * 2]}))
           (do [b doubled]))",
    )
    .unwrap();
    assert_eq!(v, Value::Int(84));
    // auto-accessor for the slot.
    let v = moof::eval(
        &mut w,
        "(let ((b {Box contents: 'hi})) [b contents])",
    )
    .unwrap();
    assert_eq!(v, Value::Sym(w.intern("hi")));
}

#[test]
fn intern_constructs_symbols_from_strings() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(intern \"foo\")").unwrap();
    assert_eq!(v, Value::Sym(w.intern("foo")));
    // idempotent on Symbol input.
    let v = moof::eval(&mut w, "(intern 'bar)").unwrap();
    assert_eq!(v, Value::Sym(w.intern("bar")));
    // joining two symbols' text and re-interning.
    let v = moof::eval(
        &mut w,
        "(intern [[(intern \"at:\") toString] + [(intern \"put:\") toString]])",
    )
    .unwrap();
    assert_eq!(v, Value::Sym(w.intern("at:put:")));
}

// ─────────────────────────────────────────────────────────────────
// concepts/blocks-and-patterns.md — `|args| body` block-sugar.
// reader-side: `|x| body` lowers to `(fn (x) body)`. discriminates
// from `[a | b]` binary-op via lookahead (no closing `|` before
// `]` ⇒ binary-op interpretation).
// ─────────────────────────────────────────────────────────────────

#[test]
fn block_sugar_unary() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def f |x| [x + 1])").unwrap();
    assert_eq!(moof::eval(&mut w, "(f 5)").unwrap(), Value::Int(6));
}

#[test]
fn block_sugar_two_args() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def add |x y| [x + y])").unwrap();
    assert_eq!(moof::eval(&mut w, "(add 3 4)").unwrap(), Value::Int(7));
}

#[test]
fn block_sugar_nullary() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def answer || 42)").unwrap();
    assert_eq!(moof::eval(&mut w, "(answer)").unwrap(), Value::Int(42));
}

#[test]
fn block_sugar_does_not_eat_binary_pipe() {
    // `[a | b]` is a binary-op send (selector `|`), not a block.
    // the lookahead must not trigger block-parse here.
    let mut w = moof::new_world();
    // install `|` on Integer so the send actually resolves.
    moof::eval(
        &mut w,
        "(setHandler! Integer '| (fn (rhs) [self + rhs]))",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "[3 | 4]").unwrap(), Value::Int(7));
}

// ─────────────────────────────────────────────────────────────────
// concepts/blocks-and-patterns.md — patterns v2:
// `|n :: Type|` type-guard, `|n where pred|` predicate-guard.
// ─────────────────────────────────────────────────────────────────

#[test]
fn match_typed_pattern() {
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(def kind
           |n :: Integer| 'an-int
           |s :: String|  'a-string
           |_|            'other)",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "(kind 5)").unwrap(),
        Value::Sym(w.intern("an-int"))
    );
    assert_eq!(
        moof::eval(&mut w, "(kind \"hi\")").unwrap(),
        Value::Sym(w.intern("a-string"))
    );
    assert_eq!(
        moof::eval(&mut w, "(kind 'sym)").unwrap(),
        Value::Sym(w.intern("other"))
    );
}

#[test]
fn match_where_pattern() {
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(def sign
           |n where [n > 0]| 'positive
           |n where [n < 0]| 'negative
           |_|               'zero)",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "(sign 5)").unwrap(),
        Value::Sym(w.intern("positive"))
    );
    assert_eq!(
        moof::eval(&mut w, "(sign -3)").unwrap(),
        Value::Sym(w.intern("negative"))
    );
    assert_eq!(
        moof::eval(&mut w, "(sign 0)").unwrap(),
        Value::Sym(w.intern("zero"))
    );
}

#[test]
fn satisfies_walks_proto_chain() {
    // [T satisfies?: v] — v's proto chain reaches T.
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "[Integer satisfies?: 5]").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "[Integer satisfies?: \"hi\"]").unwrap(),
        Value::Bool(false)
    );
    // Object satisfies everything (every proto chain bottoms there).
    assert_eq!(
        moof::eval(&mut w, "[Object satisfies?: 5]").unwrap(),
        Value::Bool(true)
    );
}

// ─────────────────────────────────────────────────────────────────
// concepts/blocks-and-patterns.md / syntax/binding-and-defs.md —
// `defn` for multi-clause pattern-matched function definitions.
// ─────────────────────────────────────────────────────────────────

#[test]
fn def_itself_supports_multi_clause() {
    // syntax/binding-and-defs.md says `def` is multi-clause:
    //   (def fact |0| 1 |n| [n * (fact [n - 1])])
    // we implement this by detecting the multi-clause shape in
    // the rust compiler and rerouting to the moof `defn` macro.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(def fact
           |0| 1
           |n| [n * (fact [n - 1])])",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "(fact 7)").unwrap(), Value::Int(5040));
    // single-binding `def` still works (the unchanged path).
    moof::eval(&mut w, "(def x 42)").unwrap();
    assert_eq!(moof::eval(&mut w, "x").unwrap(), Value::Int(42));
}

#[test]
fn defn_factorial() {
    // the canonical forcing function (concepts/blocks-and-patterns.md):
    //   (defn fact |0| 1 |n| [n * (fact [n - 1])])
    // → (fact 5) returns 120.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(defn fact
           |0| 1
           |n| [n * (fact [n - 1])])",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "(fact 0)").unwrap(), Value::Int(1));
    assert_eq!(moof::eval(&mut w, "(fact 5)").unwrap(), Value::Int(120));
    assert_eq!(moof::eval(&mut w, "(fact 6)").unwrap(), Value::Int(720));
}

#[test]
fn defn_two_arg_safe_divide() {
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(defn safe-divide
           |a 0|  nil
           |a b|  [a / b])",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "(safe-divide 10 0)").unwrap(),
        Value::Nil
    );
    assert_eq!(
        moof::eval(&mut w, "(safe-divide 10 2)").unwrap(),
        Value::Int(5),
    );
}

#[test]
fn defn_list_destructure() {
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(defn length
           |'()|         0
           |'(_ …rest)| [1 + (length rest)])",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "(length nil)").unwrap(), Value::Int(0));
    assert_eq!(
        moof::eval(&mut w, "(length (list 1 2 3 4 5))").unwrap(),
        Value::Int(5),
    );
}

#[test]
fn block_sugar_with_match_yields_factorial_via_match() {
    // bigger forcing function: a closure literal whose body uses
    // match, used in a fn-call context. proves `|n| (match n …)`
    // round-trips through the new sugar plus existing match.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(def fact
           |n| (match n
                 0 1
                 n [n * (fact [n - 1])]))",
    )
    .unwrap();
    assert_eq!(moof::eval(&mut w, "(fact 6)").unwrap(), Value::Int(720));
}

// ─────────────────────────────────────────────────────────────────
// concepts/blocks-and-patterns.md — `match` as a moof macro.
// ─────────────────────────────────────────────────────────────────

#[test]
fn match_literal_int() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(match 5 5 'matched 0 'zero)").unwrap();
    assert_eq!(v, Value::Sym(w.intern("matched")));
    let v = moof::eval(&mut w, "(match 0 5 'matched 0 'zero)").unwrap();
    assert_eq!(v, Value::Sym(w.intern("zero")));
}

#[test]
fn match_variable_binds_subject() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(match 5 n [n + 1])").unwrap();
    assert_eq!(v, Value::Int(6));
}

#[test]
fn match_wildcard_falls_through() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(match 7 0 'zero _ 'nonzero)").unwrap();
    assert_eq!(v, Value::Sym(w.intern("nonzero")));
}

#[test]
fn match_quoted_symbol_is_identity_compare() {
    let mut w = moof::new_world();
    let v = moof::eval(
        &mut w,
        "(match 'foo 'bar 'matched-bar 'foo 'matched-foo _ 'other)",
    )
    .unwrap();
    assert_eq!(v, Value::Sym(w.intern("matched-foo")));
}

#[test]
fn match_empty_list_pattern() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(match nil '() 'empty _ 'cons)").unwrap();
    assert_eq!(v, Value::Sym(w.intern("empty")));
    let v = moof::eval(&mut w, "(match (list 1) '() 'empty _ 'cons)").unwrap();
    assert_eq!(v, Value::Sym(w.intern("cons")));
}

#[test]
fn match_cons_destructure() {
    let mut w = moof::new_world();
    // factorial via match — the canonical forcing function for
    // pattern-matched defs (concepts/blocks-and-patterns.md).
    moof::eval(
        &mut w,
        "(def fact
           (fn (n)
             (match n
               0 1
               n [n * (fact [n - 1])])))",
    )
    .unwrap();
    let v = moof::eval(&mut w, "(fact 5)").unwrap();
    assert_eq!(v, Value::Int(120));
    // cons destructure: head/tail bindings.
    let v = moof::eval(
        &mut w,
        "(match (list 1 2 3)
           '() 'empty
           '(h …rest) [h * 10])",
    )
    .unwrap();
    assert_eq!(v, Value::Int(10));
}

#[test]
fn match_no_clause_raises() {
    let mut w = moof::new_world();
    let res = moof::eval(&mut w, "(match 5 0 'zero 1 'one)");
    assert!(res.is_err(), "match with no matching clause should raise");
}

#[test]
fn user_can_introspect_match_macro() {
    // moldability: `match` is a regular macro registered in the
    // canonical Macros Form. user code can introspect it via the
    // substrate's slot-lookup primitive.
    let mut w = moof::new_world();
    // sanity: it's a registered macro.
    let match_sym = w.intern("match");
    assert!(w.macro_at(match_sym).is_some());
    // …and reachable from inside moof through `(slot Macros 'match)`.
    let v = moof::eval(&mut w, "(slot Macros 'match)").unwrap();
    let id = v.as_form_id().expect("match macro should be a Form");
    // it's a method-Form (closure).
    let proto = w.heap.get(id).proto;
    assert_eq!(proto, Value::Form(w.protos.method));
}

#[test]
fn method_ics_are_reflectable() {
    // reflection-contract.md R6: "an inline cache at a send-site
    // is substrate-level state, but it's exposed as `[send-site
    // cache-stats]` for inspection." we expose it via `[m ics]`
    // returning a Table of cache-snapshot Forms — one entry per
    // Send opcode in the chunk.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(setHandler! Integer 'incr-twice (fn () [[self + 1] + 1]))",
    )
    .unwrap();
    // run it once so each Send opcode populates its cache.
    let v = moof::eval(&mut w, "[5 incr-twice]").unwrap();
    assert_eq!(v, Value::Int(7));
    // fetch the method-Form, then [m ics].
    moof::eval(&mut w, "(def m [Integer handlerAt: 'incr-twice])").unwrap();
    let ics = moof::eval(&mut w, "[m ics]").unwrap();
    // it's a Table.
    let id = ics.as_form_id().expect("[m ics] should return a Form");
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.table));
    // there are two `+` Send sites in the body. each should have
    // cached against Integer (after the test run resolved them).
    let n_ics = moof::eval(&mut w, "[[m ics] length]").unwrap();
    assert_eq!(n_ics, Value::Int(2));
    // each entry is a Form with the four cache slots.
    moof::eval(&mut w, "(def ic0 [[m ics] at: 0])").unwrap();
    let cached_proto =
        moof::eval(&mut w, "[[ic0 slots] at: 'cached-proto]").unwrap();
    // `cached_proto` for `[self + 1]` resolved against Integer.
    assert_eq!(cached_proto, Value::Form(w.protos.integer));
}

#[test]
fn current_frame_is_a_form_with_r3_slots() {
    // reflection-contract.md R3: a frame is a Form (proto: Frame)
    // exposing :chunk :pc :env :self :stack-base :defining-proto.
    // we materialize a snapshot mid-method, walk its slots, and
    // confirm the proto is `Frame`.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(defproto Counter (slots count))").unwrap();
    moof::eval(
        &mut w,
        "(setHandler! Counter 'snap (fn () (currentFrame)))",
    )
    .unwrap();
    let v = moof::eval(&mut w, "[[Counter new] snap]").unwrap();
    let id = v.as_form_id().expect("currentFrame should return a Form");
    // proto: Frame.
    assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.frame));
    // R3: :chunk, :pc, :env, :self, :stack-base, :defining-proto
    // are all populated.
    let chunk_sym = w.intern("chunk");
    let pc_sym = w.intern("pc");
    let env_sym = w.intern("env");
    let self_sym = w.intern("self");
    let stack_base_sym = w.intern("stack-base");
    let defining_sym = w.intern("defining-proto");
    let f = w.heap.get(id);
    assert!(f.slot_present(chunk_sym), ":chunk slot must be present");
    assert!(f.slot_present(pc_sym), ":pc slot must be present");
    assert!(f.slot_present(env_sym), ":env slot must be present");
    assert!(f.slot_present(self_sym), ":self slot must be present");
    assert!(
        f.slot_present(stack_base_sym),
        ":stack-base slot must be present"
    );
    assert!(
        f.slot_present(defining_sym),
        ":defining-proto slot must be present"
    );
    // :pc and :stack-base are Integers.
    assert!(matches!(f.slot(pc_sym), Value::Int(_)));
    assert!(matches!(f.slot(stack_base_sym), Value::Int(_)));
    // :defining-proto resolves to the Counter proto, since the
    // method was found there.
    let counter = moof::eval(&mut w, "Counter").unwrap();
    let f = w.heap.get(id);
    assert_eq!(f.slot(defining_sym), counter);
}

#[test]
fn call_stack_returns_a_list_of_frames() {
    // R3: `(callStack)` materializes the entire frame stack.
    // mid-method, the stack has at least one frame.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(defproto Recorder (slots))").unwrap();
    moof::eval(
        &mut w,
        "(setHandler! Recorder 'depth (fn () [(callStack) length]))",
    )
    .unwrap();
    let v = moof::eval(&mut w, "[[Recorder new] depth]").unwrap();
    // at least one frame: the depth-method itself. (the [length]
    // send adds another, but that frame returns before our
    // snapshot — :length is computed *over* the snapshot list.)
    let n = match v {
        Value::Int(n) => n,
        _ => panic!("(callStack) length should be an Int"),
    };
    assert!(n >= 1, "call stack should have ≥1 frame, got {}", n);
}

#[test]
fn macros_form_is_introspectable_from_moof() {
    // reflection-contract.md R6: the macro registry lives on a Form,
    // not in a rust HashMap. moof code can reach it through the
    // global `Macros` and ask: "is X a macro?" "what's its source?"
    let mut w = moof::new_world();
    // `when` is shipped as a bootstrap macro.
    let when_sym = w.intern("when");
    // looking it up via `Macros at: …` returns the macro's
    // method-Form. (the slot key is the macro's name symbol;
    // `at:` on a Table-shaped Form goes through slot lookup —
    // here we test the Form-as-slot-table path directly.)
    assert!(
        w.macro_at(when_sym).is_some(),
        "`when` macro should be registered"
    );
    // the binding is also visible as a global named `Macros` —
    // the same Form id as `world.macros_form`.
    let macros_global = moof::eval(&mut w, "Macros").unwrap();
    assert_eq!(macros_global, Value::Form(w.macros_form));
    // and the slots of that Form contain `when` (and friends).
    let macros_form = w.heap.get(w.macros_form);
    assert!(
        macros_form.slot_present(when_sym),
        "Macros Form should have a `when` slot"
    );
}

#[test]
fn proto_generation_lives_in_meta() {
    // reflection-contract.md R6: state-about-a-Form must be reflected.
    // proto-generation counters used to live in a rust-side HashMap on
    // World; they now live in each proto Form's :meta table under the
    // 'generation key, so `[proto meta at: 'generation]` works from
    // moof. set-handler! bumps the counter.
    let mut w = moof::new_world();
    moof::eval(&mut w, "(defproto Counter (slots count))").unwrap();
    // defproto itself installs methods under the hood, so the
    // counter has already advanced. capture the current value as
    // the baseline.
    let before = moof::eval(&mut w, "[[Counter meta] at: 'generation]").unwrap();
    let n0 = match before {
        Value::Int(n) => n,
        Value::Nil => 0,
        _ => panic!("generation should be Int or Nil, got {:?}", before),
    };
    // installing a handler bumps the counter by exactly one.
    moof::eval(
        &mut w,
        "(setHandler! Counter 'incr (fn () [.count + 1]))",
    )
    .unwrap();
    let after = moof::eval(&mut w, "[[Counter meta] at: 'generation]").unwrap();
    assert_eq!(after, Value::Int(n0 + 1));
    // a second mutation increments again.
    moof::eval(
        &mut w,
        "(setHandler! Counter 'decr (fn () [.count - 1]))",
    )
    .unwrap();
    let after2 = moof::eval(&mut w, "[[Counter meta] at: 'generation]").unwrap();
    assert_eq!(after2, Value::Int(n0 + 2));
}

#[test]
fn let_inside_method_sees_self() {
    // `let` desugars to a closure-call. without PushClosure
    // capturing self_, the closure ran with self = nil. fix in
    // place; method bodies that wrap state in `(let ...)` still
    // see the receiver.
    let mut w = moof::new_world();
    moof::eval(
        &mut w,
        "(setHandler! Integer 'doubled-via-let
           (fn ()
             (let ((n self)) [n * 2])))",
    )
    .unwrap();
    assert_eq!(
        moof::eval(&mut w, "[5 doubled-via-let]").unwrap(),
        Value::Int(10),
    );
}
