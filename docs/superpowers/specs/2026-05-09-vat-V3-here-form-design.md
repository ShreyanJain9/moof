# vat phase V3 — env-chain / `$here` unification — design

> **status:** brainstormed 2026-05-09. ready for plan.
>
> **prior art:** V0 (FormId scope-tagging, shipped) + V1 (per-turn nursery + diff, shipped) + V2 (freezing, shipped). V3 unifies the global env into the moof object model — `$here` becomes a first-class Form reachable from moof, `def` and `set!` collapse from rust special forms with dedicated opcodes into pure method dispatch.
>
> **spec reference:** `2026-05-04-vats-and-references-protocol-design.md` §8 (the environment model) is the user-facing spec; this document is the substrate-side implementation design.

## 1. scope and motivation

V3 makes the env-as-Form picture real at the user layer and collapses two duplicate code paths:

- **`Op::DefineGlobal`** vs the moof-side concept of "bind a name in the global env" — same operation, two implementations (rust opcode + redundant compile path).
- **`Op::StoreName`** vs the moof-side concept of "mutate the binding I can find in the lexical chain" — same operation.

Both opcodes are removed. `def` and `set!` become method dispatch via a small Env proto API + a `Frame` proto exposing the current lexical env. `$here` is exposed as a moof binding pointing to the global env Form (renamed `here_form` on `World`). User code can introspect, bind into, walk, and (eventually, V4+) freeze envs through the same surface as any other Form.

V3 also lands a single new ergonomic affordance — `Object:eval:` — that runs a closure with a receiver-Form's slots spliced into its lookup chain (the practical "instance_eval" rubyism). Lookups in the closure body find names from BOTH the receiver's slots and the closure's captured env.

V3 does **not** include: full Ruby `instance_eval` with live-forwarding mutations (V8+); `if` purification to method-dispatch (V3.5 — same shape as def/set! but more invasive, deferred); lexical-env-as-first-class-Form for vau / fexpr support (V8); cross-vat closure travel and `$here` rebinding (V5).

## 2. the rust field rename: `global_env` → `here_form`

`World.global_env` was a substrate-internal name reflecting a now-outdated mental model ("the global env"). Post-V3, it becomes `here_form`:

```rust
pub struct World {
    // ...
    pub here_form: FormId,    // was: pub global_env: FormId,
    // ...
}
```

32 call sites across the substrate update mechanically. The rename is forward-looking: V4 will migrate `here_form` from `World` to `Vat` (per the V4 vat-as-Form structure in `2026-05-04-vats-and-references-protocol-design.md` §9). Doing the rename now means V4's plan is a structural lift rather than "rename + lift."

## 3. `$here` boot binding — self-reference is fine

`intrinsics::install` (inside the V1 boot turn, already wrapped in `lib.rs::new_world`) binds the symbol `$here` in `here_form`'s slots:

```rust
let dollar_here = world.intern("$here");
world.form_slot_set(here_form, dollar_here, Value::Form(here_form))?;
```

The binding is self-referential — `here_form.slots['$here] = Value::Form(here_form)`. moof reflection (`[Heap slotKeysOf: $here]`) will list `$here` itself as one of its own slots. This is fine in moof's "everything is a Form" model — the same shape as `[$out :proto]` walking out and back.

When V9 persistence lands, the self-reference survives serialize/deserialize round-trips because FormIds are stable identifiers — `$here` deserializes to point at whichever FormId the deserialized `here_form` lives under.

## 4. the Env proto API

`protos.env` already exists. V3 installs four methods on it as natives in `intrinsics::install`:

### 4.1. `:bind:to:` — non-walking bind

```moof
[env bind: 'name to: 42]    ;; → 42 (the bound value)
```

Native body wraps `world.form_slot_set(env, name, value)` directly. Returns the value. No parent-chain walk — binds in `env` itself, regardless of whether `name` is shadowed elsewhere.

### 4.2. `:set:to:` — walks chain, raises on miss

```moof
[env set: 'name to: 42]    ;; → 42 (or raises 'unbound)
```

Native body wraps `world.env_set(env, name, value)`. Returns the value on success. Raises `'unbound` if `name` is not bound anywhere in the parent chain rooted at `env`. The raise is the V3-tightened semantic vs today's `set!` which silently falls back to creating a global binding (a footgun the V1 turn-aware raise machinery now lets us close).

### 4.3. `:lookup:` — walks chain, returns Nil on miss

```moof
[env lookup: 'name]         ;; → value or Nil
```

Native body wraps `world.env_lookup(env, name)`. Same semantics as today's substrate-internal `env_lookup`: walks the parent chain, returns the first hit, returns `Nil` if no hit reached.

### 4.4. `:parent` — explicit parent access

```moof
[env parent]                ;; → parent env Form, or Nil at chain root
```

Equivalent to `[Heap metaOf: env at: 'parent]` — convenience method exposing the parent slot directly. Returns `Nil` for `here_form` (root of the chain).

### 4.5. reflection — `:keys` is free

`[Heap slotKeysOf: $here]` lists the bindings in scope at `$here` directly via the V1 task-8 keys helpers. No dedicated `:keys` method needed on Env proto for V3 — the existing reflection ergonomic suffices.

## 5. the Frame proto + `:current`

Lexical-env-as-Form reachability for `set!`'s macro form. Frame is a new proto allocated at boot and bound globally:

```rust
let frame_proto = world.alloc(Form::with_proto(Value::Form(world.protos.object)));
let frame_sym = world.intern("Frame");
world.form_slot_set(here_form, frame_sym, Value::Form(frame_proto))?;
world.install_native(frame_proto, "current", |w, _self_, _args| {
    // natives don't push a VM frame; frames.last() IS the caller's lexical env.
    let env = w.vm.frames.last()
        .map(|f| f.env)
        .ok_or_else(|| RaiseError::new(
            w.intern("frame-out-of-scope"),
            "[Frame current] called outside any active method dispatch",
        ))?;
    Ok(Value::Form(env))
})?;
```

`[Frame current]` returns the FormId of the caller's lexical env. The set!-macro uses this to resolve "the env at the call site of set!" at run time.

`Frame` is not the same kind of object as the `Frame` slot snapshot used by the existing `:source` / `frame_snapshot` machinery — those are debugger-style snapshots of the call stack. V3's `Frame` is just a proto exposing the substrate's "the current call's env" primitive. (The naming overlap is fine; future cleanup can rename one of the two if it becomes confusing — flagged as a Minor note for the V3 plan's review pass.)

## 6. `Object:eval:` — capture from both

Installed as a native on `protos.object`. Implements the practical Ruby `instance_eval` shape:

```moof
[obj eval: closure]
```

Semantics:

1. Read `closure.:env` (the captured env from the closure's definition site).
2. Allocate `merge_env` with `parent = captured_env`.
3. Shallow-copy `obj.slots` into `merge_env.slots` (Values are shared by reference; the `Value` type is `Copy`).
4. Invoke the closure body with `call_env = merge_env` and `self = obj`.

Lookups inside the closure body walk:

```
call_env (locals from let-bindings inside body)
   → merge_env (obj's snapshot ∪ closure's captured names if same name)
   → captured_env (closure's lexical chain)
   → captured_env's parents → ... → globals
```

This delivers "capture from both" for **lookups** — instance vars from `obj` AND lexical free vars from where the closure was defined are both findable. Mutations made inside the body (via `:bind:to:` on `merge_env`, or via let-bindings) stay in `merge_env`; they don't propagate back to `obj` or to `captured_env`. That's Ruby's block-local-ish behavior and lets V3 ship a tractable implementation. Live-forwarding mutations (true Ruby `instance_eval` where `@x = 5` actually writes to obj) are V8+ work — would require a custom env-walker that delegates lookups *and* writes through to obj without copying.

The shallow-copy approach has one observable consequence worth documenting: if `obj` is mutated mid-eval (by some other code path), the copy in `merge_env` doesn't update. Practical impact in V3: minimal, because moof is single-threaded within a vat. V8+ multi-actor changes can revisit.

## 7. `def` becomes a moof macro — purifying `Op::DefineGlobal`

### 7.1. the macro

Added to the top of `lib/early/06-control-macros.moof` (the file's existing mission is exactly this — converting compiler-level special forms into user-modifiable macros):

```moof
;; (def name value) → (do [$here bind: 'name to: value] 'name)
(defmacro def (args)
  (let ((name [args car])
        (value [[args cdr] car]))
    `(do
       [$here bind: ',name to: ,value]
       ',name)))
```

### 7.2. dispatch reorder

`lib/compiler/01-dispatch.moof::compileForm:chunk:` is reordered to check macros BEFORE special-form dispatch. After the def-macro registers (post-load of early/06), every subsequent `(def …)` form goes through macro expansion. The moof Compiler's `compileDef:` is shadowed but stays in place — emits equivalent bytecode if ever reached (used during the bootstrap window, before early/06 loads).

### 7.3. rust seed `compile_def` rewrite

The rust seed compiles `lib/main.moof` and `lib/compiler/*.moof` directly (pre-flip). It still has `def_sym` as a recognized special form. Its `compile_def` is rewritten to emit Send-based bytecode equivalent to the macro expansion:

```rust
fn compile_def(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
    // (def name value)
    let name = elems[1].as_sym().ok_or(...)?;
    let here_sym = self.world.intern("$here");
    let bind_to_sym = self.world.intern("bind:to:");
    
    // LoadName $here
    self.emit(Op::LoadName(here_sym));
    // LoadConst 'name
    let const_idx = self.add_const(Value::Sym(name));
    self.emit(Op::LoadConst(const_idx));
    // compile rhs
    self.compile_form(elems[2], false)?;
    // Send :bind:to: arity=2
    let ic_idx = self.fresh_ic();
    self.emit(Op::Send { selector: bind_to_sym, argc: 2, ic_idx });
    // Pop the bind result (the value); push 'name as the def's return
    self.emit(Op::Pop);
    let name_const = self.add_const(Value::Sym(name));
    self.emit(Op::LoadConst(name_const));
    Ok(())
}
```

After the rewrite, the rust seed never emits `Op::DefineGlobal`. Compiler.moof's own `(def …)` uses compile to Send dispatch — runnable because `$here` and `:bind:to:` are bound by `intrinsics::install` BEFORE compiler.moof loads.

### 7.4. moof Compiler `compileDef:` rewrite

`lib/compiler/02-special.moof::compileDef:chunk:` is rewritten to emit the same Send-based bytecode pattern. It survives as a bootstrap fallback for early/00–05 files (which load before the def macro registers). Once early/06 loads and the macro is registered, dispatch checks macros first — `compileDef:` is shadowed but harmless.

The moof Compiler's `compileDef:` and the def-macro's expansion produce **bytecode-identical** output. There is no longer a duplicate operation at the runtime level — both paths are method dispatch. This is what "no opcode/macro split" means: at the bytecode layer, there is exactly one way to bind a name into `$here`, and it goes through `:bind:to:`.

### 7.5. `Op::DefineGlobal` removed

Once the seed and moof Compiler stop emitting it:

- Remove from `crates/substrate/src/opcodes.rs`.
- Remove handler from `crates/substrate/src/vm.rs`.
- Remove encode/decode entries from `crates/substrate/src/intrinsics.rs` (the `mk_op_form` and reverse-decode tables).
- Update any tests that assert on `Op::DefineGlobal` directly.

## 8. `set!` becomes a moof macro — purifying `Op::StoreName`

### 8.1. the macro

Added to `lib/early/06-control-macros.moof` alongside the def macro:

```moof
;; (set! name value) → [[Frame current] set: 'name to: value]
;; raises 'unbound if name isn't reachable from the lexical chain.
(defmacro set! (args)
  (let ((name [args car])
        (value [[args cdr] car]))
    `[[Frame current] set: ',name to: ,value]))
```

The macro form references `[Frame current]` — at runtime, this resolves to the FormId of the env at the call site of `set!`. `:set:to:` on Env walks that env's parent chain, raises `'unbound` on miss.

### 8.2. moof Compiler `compileSet:` rewrite

`lib/compiler/03-control.moof::compileSet:chunk:` is rewritten to emit the macro-equivalent Send-based bytecode:

```
LoadName Frame
Send :current arity=0
LoadConst 'name
<compile rhs>
Send :set:to: arity=2
```

(The rust seed compiler doesn't compile `set!` — `set_sym` was "deliberately absent" per the existing comment at compiler.rs:105. So no seed change needed.)

### 8.3. semantic shift: silent-on-miss → raise

Today's `Op::StoreName` walks the lexical chain via `env_set`; if no binding is found, it falls back to `env_bind` on `world.global_env` (silently creating a new global). V3 tightens this: `[env set: 'foo to: value]` raises `'unbound` if `foo` isn't in scope.

This is a **behavioral change**. Any user code relying on `(set! undeclared-name value)` to create a global will break. Migration: change to `(def undeclared-name value)`. The plan's verification gate will run the existing test suite — if any test relies on the silent fall-through, it gets fixed during V3.

### 8.4. `Op::StoreName` removed

After the rewrite:

- Remove from `crates/substrate/src/opcodes.rs`.
- Remove handler from `crates/substrate/src/vm.rs`.
- Remove encode/decode entries.

## 9. dispatch reorder in `lib/compiler/01-dispatch.moof`

`compileForm:chunk:` currently dispatches in roughly this order:

1. is the head a known special form (`'if`, `'def`, `'defmacro`, `'set!`, etc.)? → call the corresponding `compileX:chunk:` handler.
2. is the head a registered macro? → expand and recurse.
3. otherwise → compile as function call.

V3 swaps (1) and (2):

1. is the head a registered macro? → expand and recurse.
2. is the head a known special form? → call the corresponding handler.
3. otherwise → compile as function call.

This makes the def, set!, and (future) if macros take precedence over their special-form fallbacks. The fallbacks stay in place for the bootstrap window (before each macro is registered) and for any future macro override (a user can `(defmacro def …)` and replace the default).

## 10. why `if` is deferred to V3.5

The same purification shape applies to `if`: rewrite `compile_if` (rust seed) and `compileIf:` (moof Compiler) to emit `[[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]` Send pattern, remove `Op::JumpIfFalse` / `Op::Jump` if they become unused (they probably stay — used by `let` and other env-construction primitives).

Why deferred:
1. The compile-time rewrite for `if` requires constructing closures on the fly for the true/false branches — significantly more bytecode than the current jump-based form. Performance impact is real (every `if` becomes 2 closure allocations + 3 sends). V3.5 should benchmark before committing.
2. The existing `lib/early/10-if-macro.moof` already provides the macro path for user-level `if`. Files loaded after early/10 use the macro; files before use the special form. V3 doesn't worsen this; V3.5 closes it.
3. V3's scope is already substantial (rename + new field bindings + Env/Frame/Object methods + def/set! migration + 2 opcode removals). Holding `if` off keeps V3 reviewable.

## 11. boot order

`intrinsics::install` (already inside the V1 boot turn) is extended:

1. Existing: install all current natives (Object, Cons, Nil, Bool, Integer, Float, Symbol, Char, String, Bytes, Cons, Table, Method, Chunk, Closure, Env, Foreign, Frame snapshot machinery, $out / $err caps, $compiler cap, $transporter, $hash).
2. **V3 additions:**
   - install Env proto methods: `:bind:to:`, `:set:to:`, `:lookup:`, `:parent` on `protos.env`.
   - allocate Frame proto, install `:current` on it, bind `Frame` global.
   - install Object `:eval:` method on `protos.object`.
   - bind `$here` in `here_form.slots` (self-reference).

After `intrinsics::install` returns, the boot turn commits, then `lib/main.moof` loads:

- `lib/compiler/*.moof` — compiled by rust seed, uses `def` via the rewritten `compile_def` (Send-based, no `Op::DefineGlobal`).
- `[$compiler useMoof]` — flip.
- `lib/early/00-cons.moof` through `lib/early/05-quasiquote.moof` — compiled by moof Compiler. `def` via `compileDef:` (rewritten Send-based fallback). Macros not yet checked first.
- `lib/early/06-control-macros.moof` — registers def, set! macros. Dispatch is also reordered here (the dispatch reorder lives in `lib/compiler/01-dispatch.moof` so it's already in effect from the moof Compiler's first compile, but is a no-op until macros register).
- All subsequent files use def, set! via the macro path.

## 12. exit criteria

V3 lands when:

1. `World.global_env` field is renamed to `here_form` everywhere (32 call sites updated).
2. `intrinsics::install` binds `$here` self-referentially in `here_form.slots`.
3. Env proto has `:bind:to:`, `:set:to:`, `:lookup:`, `:parent` methods (4 new natives).
4. Frame proto exists with `:current` method; `Frame` is bound globally.
5. Object proto has `:eval:` method (1 new native).
6. `Op::DefineGlobal` and `Op::StoreName` are removed from `crates/substrate/src/opcodes.rs`. Their VM handlers and encode/decode entries are also removed.
7. Rust seed `compile_def` emits Send-based bytecode (no `Op::DefineGlobal`).
8. Moof Compiler `compileDef:` and `compileSet:` rewritten to emit Send-based bytecode.
9. `lib/early/06-control-macros.moof` has `(defmacro def …)` and `(defmacro set! …)`.
10. `lib/compiler/01-dispatch.moof::compileForm:chunk:` checks macros before special-form dispatch.
11. All 481 V2 tests still pass; new V3 tests cover: `(def x 42)` then `x` → 42 (regression for the macro path); `(set! foo 0)` inside `(let ((foo 5)) ...)` mutates the lexical foo; `(set! unbound-name 0)` raises `'unbound`; `[obj eval: (fn () [self bar])]` returns obj's `bar`; `[obj eval: closure]` finds names from both obj and closure's captured env; `[Frame current]` returns the lexical env of the caller; `[$here lookup: 'foo]`, `[$here parent]`, `[$here bind: 'baz to: 99]` work; `[Heap slotKeysOf: $here]` lists global bindings (including `$here` itself).

## 13. test plan (sketch)

Unit tests in `crates/substrate/src/world.rs::tests` and `crates/substrate/src/vm.rs::tests`:
- `here_form_is_self_referential_at_boot`
- `env_bind_to_writes_slot_no_walk`
- `env_set_to_walks_chain_returns_value`
- `env_set_to_raises_unbound_when_not_in_chain`
- `env_lookup_walks_chain_nil_on_miss`
- `env_parent_returns_parent_form_or_nil`
- `frame_current_returns_caller_env`
- `object_eval_runs_closure_with_obj_self`
- `object_eval_lookups_find_obj_slots`
- `object_eval_lookups_also_find_closure_captured_names`

Integration tests in `crates/substrate/tests/here_e2e.rs`:
- `def_macro_binds_in_here`
- `set_macro_walks_lexical_chain`
- `set_macro_raises_unbound`
- `here_self_reference_works_in_reflection`
- `obj_eval_capture_from_both`
- `obj_eval_mutations_stay_local`

Plus regression tests for the existing test suite (V0 / V1 / V2).

## 14. out of scope (deferred)

- **`if` purification**: V3.5 — same shape as def/set! but with closure-construction overhead concerns; benchmark-driven decision.
- **Lexical-env-as-first-class for vau / fexpr**: V8 — exposes `[Frame current]` ergonomically and adds primitives for capturing dynamic environments at the macro level.
- **Live-forwarding `Object:eval:`**: V8+ — true Ruby `instance_eval` where mutations propagate back to obj. Requires custom env walker.
- **Cross-vat closure travel + `$here` rebinding**: V5 — when a closure travels to another vat, its captured `$here` rebinds to the receiving vat's `$here`.
- **`$here` persistence + journal**: V9 — the env Form persists naturally; the journal records bindings as ordinary slot mutations.
- **Mutation-via-`:bind:to:` on frozen forms**: V2's `'frozen-form` raise applies; if `$here` is ever frozen (V4+ replicated vats might do this), `[$here bind: …]` raises. V3 doesn't change this behavior — just inherits it.
- **`let`, `fn`, `do`, `defmacro`, `quote` purification**: out of V3's scope. These construct envs / frames / macro machinery — substrate primitives, harder to eliminate. Future phase if motivation arises.

## see also

- `2026-05-04-vats-and-references-protocol-design.md` §8 — the user-facing spec for the env model V3 implements.
- `2026-05-06-vat-V1-nursery-diff-design.md` — V1's per-turn nursery, which V3 inherits (env mutations journal through the nursery as usual).
- `2026-05-07-vat-V2-freezing-design.md` §4 — V2's freezing model; envs stay freezable (not in `live_protos`).
