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

All three opcodes (`Op::DefineGlobal`, `Op::StoreName`, plus the seed/moof-Compiler special-form handling for `if`) are removed. `def`, `set!`, and `if` become method dispatch via a small Env proto API. `$here` is exposed as a moof binding pointing to the global env Form (renamed `here_form` on `World`). The current lexical env is reachable as `[Env current]`. User code can introspect, bind into, walk, and (eventually, V4+) freeze envs through the same surface as any other Form.

V3 also lands a new ergonomic affordance — `Object:eval:` — that runs a closure with a receiver-Form's parent temporarily spliced to the closure's captured env (true Ruby `instance_eval` with live forwarding). Lookups AND mutations in the closure body propagate live to BOTH the receiver's slots and the closure's captured env.

V3 also purifies `if` along the same line as `def` and `set!` — three opcode/special-form duplicates removed in one phase.

V3 does **not** include: lexical-env-as-first-class-Form for vau / fexpr support (V8); cross-vat closure travel and `$here` rebinding (V5); persistence inheritance (V9); a non-mutating "read-only" eval variant for frozen receivers (V8+).

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

## 5. lexical-env-as-Form via `Env :current`

Lexical-env-as-Form reachability for `set!`'s macro form. **No new proto is added** — the existing `Env` proto (already bound globally to `protos.env`) gains a `:current` class-style method. The existing `Frame` proto stays as-is for its debugger-style snapshot machinery (`(currentFrame)` global, `frame_snapshot`, `frame_stack_snapshot` in `world.rs:752`+ unchanged). No naming overlap.

```rust
world.install_native(world.protos.env, "current", |w, _self_, _args| {
    // natives don't push a VM frame; frames.last() IS the caller's
    // lexical env. ignore self_ — `:current` is class-method-style:
    // [Env current], [$here current], [someEnv current] all return
    // the current frame's env.
    let env = w.vm.frames.last()
        .map(|f| f.env)
        .ok_or_else(|| RaiseError::new(
            w.intern("env-out-of-scope"),
            "[Env current] called outside any active method dispatch",
        ))?;
    Ok(Value::Form(env))
})?;
```

`[Env current]` returns the FormId of the caller's lexical env. The set!-macro uses this to resolve "the env at the call site of set!" at run time. Since native fns don't push a VM frame (verified at vm.rs:258, the native-dispatch path), `frames.last()` IS the caller's env — exactly what we want.

class-method-style note: `:current` ignores its receiver. Whether you call `[Env current]`, `[$here current]`, or `[someUserEnv current]`, the result is the same: the current dynamic frame's env. Useful as a uniform reflection primitive; surprising-but-harmless if invoked on a specific env instance.

## 6. `Object:eval:` — full live forwarding (Ruby instance_eval semantics)

Installed as a native on `protos.object`. Implements true Ruby `instance_eval` — lookups and mutations BOTH propagate live, capturing from both obj and the closure's captured env.

```moof
[obj eval: closure]
```

Semantics (parent-splice with save/restore):

1. Read `closure.:env` (the captured env from the closure's definition site).
2. **Save** `obj.:meta at: 'parent` (call it `saved_parent`).
3. **Splice**: set `obj.:meta at: 'parent` = `closure.:env` (the captured env). This is a turn-mutation (journals through V1's nursery; rolls back if the outer turn aborts).
4. Allocate `body_env` with `parent = obj`.
5. Invoke the closure's body chunk with `call_env = body_env` and `self = obj` (via `World::run_method`).
6. **Restore** `obj.:meta at: 'parent` = `saved_parent` (also a turn-mutation; net effect of splice + restore is invisible from outside the eval).
7. Return whatever the closure body returned.

Lookups inside the closure body walk the env chain:

```
body_env (let-locals declared inside the closure body)
   → obj (its slots — LIVE; reflects any mutation during eval)
   → obj's pre-splice parent chain... no, wait — see below.
```

The splice **temporarily replaces** `obj`'s parent for the duration of `eval:`. So during eval, `obj`'s effective parent is the closure's captured env, not whatever obj's original parent was. The chain becomes:

```
body_env → obj → closure.captured_env → captured_env's parents → ... → globals
```

Both lookups and mutations are live:

- **Lookups** find names from BOTH obj's slots AND closure's captured chain (and globals beyond). True "capture from both."
- **Mutations** via `(set! name value)` walk `[Env current]` (which is body_env). `:set:to:` walks the chain — if `name` is in body_env: body-local; if in obj: writes to obj LIVE; if in captured_env: writes to that env LIVE (visible to other holders of that closure). Raises `'unbound` only if name isn't reachable anywhere in the chain.
- **New bindings** via `[$here bind: ...]` (i.e. `def`) target `$here` (global) as always — orthogonal to the eval scope.
- **Body-local bindings** (let-style) bind in body_env via the let macro's normal frame allocation.

The save/restore pattern keeps obj's parent change confined to the eval's dynamic extent. If the closure body raises and the outer turn aborts, the splice rolls back via V1's nursery abort — no special handling needed. If the closure body succeeds, the explicit restore (step 6) brings obj.parent back to `saved_parent`. From outside the eval, obj's parent is unchanged.

**Constraint: `obj` must not be frozen.** The splice in step 3 calls `form_meta_set` on obj, which raises `'frozen-form` if obj's frozen bit is set. This is an inherited consequence of V2's freeze invariant. Documented; user-recoverable. If you want to eval against a frozen form, you can't with V3's :eval:. Future enhancement could provide a non-mutating variant that uses a custom env-walker (e.g. `:peekEval:` or `:evalReadOnly:`), but that's V8+ work.

**Multi-threaded note (forward-looking):** the parent splice mutates obj for the eval's dynamic extent. In V3 (single-threaded vat), this is invariant-safe. V8+ multi-actor designs that share obj across vats need to think about this — but that's a far-future concern; the V5 cross-vat reference protocol already requires explicit far-refs for shared mutable state, so the issue is naturally bounded.

**Implementation requires a new `World::run_method`-shaped helper** that takes the closure-Form and an explicit `call_env`, since the existing `World::invoke` always allocates its own call_env from the method's `:env` slot. The new helper extracts the closure's `:body`, `:params`, and binds args into `call_env` (rather than alloc'ing fresh). `run_method` itself (the lower-level VM entry) already takes the env explicitly; :eval:'s native body composes around it.

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
;; (set! name value) → [[Env current] set: 'name to: value]
;; raises 'unbound if name isn't reachable from the lexical chain.
(defmacro set! (args)
  (let ((name [args car])
        (value [[args cdr] car]))
    `[[Env current] set: ',name to: ,value]))
```

The macro form references `[Env current]` — at runtime, this resolves to the FormId of the env at the call site of `set!`. `:set:to:` on Env walks that env's parent chain, raises `'unbound` on miss.

### 8.2. moof Compiler `compileSet:` rewrite

`lib/compiler/03-control.moof::compileSet:chunk:` is rewritten to emit the macro-equivalent Send-based bytecode:

```
LoadName Env
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

## 10. `if` purification — included in V3

Same shape as def/set!. The existing `lib/early/10-if-macro.moof` already defines the macro:

```moof
;; (if c t e) → [[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]
(defmacro if (args)
  (let ((c [args car])
        (t [[args cdr] car])
        (rest [[args cdr] cdr]))
    (let ((e [[rest is nil]
              ifTrue:  (fn () nil)
              ifFalse: (fn () [rest car])]))
      `[[,c !!] ifTrue: (fn () ,t) ifFalse: (fn () ,e)])))
```

V3 doesn't change this macro — it's already correct. What V3 changes:

### 10.1. rust seed `compile_if` rewrite

Currently emits `Op::JumpIfFalse` / `Op::Jump`. V3 rewrites to emit Send-based bytecode equivalent to the macro expansion:

```rust
fn compile_if(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
    // (if c t [e])
    let c = elems[1];
    let t = elems[2];
    let e = if elems.len() >= 4 { elems[3] } else { Value::Nil };
    
    let bang_bang = self.world.intern("!!");
    let if_true_if_false = self.world.intern("ifTrue:ifFalse:");
    
    // compile c, then Send :!! to coerce to Bool
    self.compile_form(c, false)?;
    let ic_idx = self.fresh_ic();
    self.emit(Op::Send { selector: bang_bang, argc: 0, ic_idx });
    
    // compile t into a fresh chunk-Form; PushClosure over it
    let t_chunk = self.compile_thunk(t)?;     // helper: returns FormId of new chunk
    self.emit(Op::PushClosure(t_chunk));
    
    // compile e into a fresh chunk-Form; PushClosure over it
    let e_chunk = self.compile_thunk(e)?;
    self.emit(Op::PushClosure(e_chunk));
    
    // Send :ifTrue:ifFalse: arity=2
    let ic_idx2 = self.fresh_ic();
    let send_op = if tail {
        Op::TailSend { selector: if_true_if_false, argc: 2 }
    } else {
        Op::Send { selector: if_true_if_false, argc: 2, ic_idx: ic_idx2 }
    };
    self.emit(send_op);
    Ok(())
}
```

`compile_thunk(form)` is a new helper that compiles `form` into a fresh chunk-Form (proto = Chunk) with no params and returns its FormId. `Op::PushClosure(chunk_id)` allocates a closure-Form at run time wrapping the chunk + the current call env.

### 10.2. moof Compiler `compileIf:` rewrite

`lib/compiler/03-control.moof::compileIf:chunk:` is rewritten to emit the same Send-based bytecode pattern. Same equivalence rule as def/set!: the special-form handler and the macro produce bytecode-identical output.

### 10.3. `Op::JumpIfFalse` and `Op::Jump` — keep

These opcodes are still emitted by other compile paths — primarily `compile_let` for tail-position branching, and any future user macro that compiles to jump-based control flow. They are NOT duplicates of method dispatch (no method emits them); they're substrate primitives for sequential bytecode flow. Keep both.

### 10.4. performance note

Each `if` now allocates 2 closure-Forms at run time (one per branch) plus 3 Send dispatches (`:!!`, `:ifTrue:ifFalse:`, plus the receiver-evaluation Send if non-trivial). vs the prior 2-3 jump opcodes. Roughly an order of magnitude more bytecode for a primitive control-flow construct.

We accept this for V3 in the name of purity. A future peephole optimizer could detect the pattern `[[c !!] ifTrue: <static-closure> ifFalse: <static-closure>]` and emit the original Jump-based bytecode — gives back the perf without losing the user-overridable macro semantics. Flagged as V3.5 follow-up if profiling shows real impact in hot paths.

## 11. boot order

`intrinsics::install` (already inside the V1 boot turn) is extended:

1. Existing: install all current natives (Object, Cons, Nil, Bool, Integer, Float, Symbol, Char, String, Bytes, Cons, Table, Method, Chunk, Closure, Env, Foreign, Frame snapshot machinery, $out / $err caps, $compiler cap, $transporter, $hash).
2. **V3 additions:**
   - install Env proto methods: `:bind:to:`, `:set:to:`, `:lookup:`, `:parent`, `:current` on `protos.env`.
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
3. Env proto has `:bind:to:`, `:set:to:`, `:lookup:`, `:parent`, `:current` methods (5 new natives).
4. Object proto has `:eval:` method (1 new native) — full live-forwarding semantics via parent splice + save/restore.
5. `Op::DefineGlobal` and `Op::StoreName` are removed from `crates/substrate/src/opcodes.rs`. Their VM handlers and encode/decode entries are also removed.
6. Rust seed `compile_def` emits Send-based bytecode (no `Op::DefineGlobal`).
7. Rust seed `compile_if` emits Send-based bytecode (`[[c !!] ifTrue: ... ifFalse: ...]` pattern via PushClosure).
8. Moof Compiler `compileDef:`, `compileSet:`, `compileIf:` rewritten to emit Send-based bytecode.
9. `lib/early/06-control-macros.moof` has `(defmacro def …)` and `(defmacro set! …)` at the top of the file. `lib/early/10-if-macro.moof` is unchanged (already correct).
10. `lib/compiler/01-dispatch.moof::compileForm:chunk:` checks macros before special-form dispatch.
11. All 481 V2 tests still pass; new V3 tests cover: `(def x 42)` then `x` → 42 (regression for the macro path); `(set! foo 0)` inside `(let ((foo 5)) ...)` mutates the lexical foo; `(set! unbound-name 0)` raises `'unbound`; `(set! obj-slot 5)` inside `[obj eval: …]` mutates obj's slot LIVE; `[obj eval: (fn () [self bar])]` returns obj's `bar`; `[obj eval: closure]` finds names from both obj and closure's captured env; eval against frozen obj raises `'frozen-form`; `[Env current]` returns the lexical env of the caller; `[$here lookup: 'foo]`, `[$here parent]`, `[$here bind: 'baz to: 99]` work; `[Heap slotKeysOf: $here]` lists global bindings (including `$here` itself); `(if c t e)` produces correct bytecode shape (compiles to Send pattern, not Jump-based).

## 13. test plan (sketch)

Unit tests in `crates/substrate/src/world.rs::tests` and `crates/substrate/src/vm.rs::tests`:
- `here_form_is_self_referential_at_boot`
- `env_bind_to_writes_slot_no_walk`
- `env_set_to_walks_chain_returns_value`
- `env_set_to_raises_unbound_when_not_in_chain`
- `env_lookup_walks_chain_nil_on_miss`
- `env_parent_returns_parent_form_or_nil`
- `env_current_returns_caller_env`
- `object_eval_runs_closure_with_obj_self`
- `object_eval_lookups_find_obj_slots`
- `object_eval_lookups_also_find_closure_captured_names`
- `object_eval_set_in_obj_propagates_live`
- `object_eval_on_frozen_obj_raises_frozen_form`
- `object_eval_save_restores_obj_parent_on_success`
- `object_eval_save_restores_obj_parent_on_raise`

Integration tests in `crates/substrate/tests/here_e2e.rs`:
- `def_macro_binds_in_here`
- `set_macro_walks_lexical_chain`
- `set_macro_raises_unbound`
- `here_self_reference_works_in_reflection`
- `obj_eval_capture_from_both`
- `obj_eval_set_propagates_live_to_obj`
- `if_compiles_to_send_pattern_not_jump`

Plus regression tests for the existing test suite (V0 / V1 / V2).

## 14. out of scope (deferred)

- **Lexical-env-as-first-class for vau / fexpr**: V8 — adds primitives like `(vau (args env) ...)` that capture the caller's env directly at the macro level. V3's `[Env current]` is a runtime primitive sufficient for set!; V8 adds the compile-time/macro-time variants.
- **Read-only eval for frozen receivers**: V8+ — `[obj peekEval: closure]` or similar that uses a custom env walker to chain into obj without mutating its parent. V3's `:eval:` requires obj to be mutable (raises `'frozen-form` on attempt against frozen obj).
- **Cross-vat closure travel + `$here` rebinding**: V5 — when a closure travels to another vat, its captured `$here` rebinds to the receiving vat's `$here`.
- **`$here` persistence + journal**: V9 — the env Form persists naturally; the journal records bindings as ordinary slot mutations.
- **Mutation-via-`:bind:to:` on frozen forms**: V2's `'frozen-form` raise applies; if `$here` is ever frozen (V4+ replicated vats might do this), `[$here bind: …]` raises. V3 doesn't change this behavior — just inherits it.
- **`let`, `fn`, `do`, `defmacro`, `quote` purification**: out of V3's scope. These construct envs / frames / macro machinery — substrate primitives, harder to eliminate. Future phase if motivation arises.
- **Peephole optimizer for the if-as-Send pattern**: V3.5 follow-up if profiling shows `if`'s 2-closure-per-execution overhead matters in hot paths. Restores Jump-based bytecode without losing macro semantics.

## see also

- `2026-05-04-vats-and-references-protocol-design.md` §8 — the user-facing spec for the env model V3 implements.
- `2026-05-06-vat-V1-nursery-diff-design.md` — V1's per-turn nursery, which V3 inherits (env mutations journal through the nursery as usual).
- `2026-05-07-vat-V2-freezing-design.md` §4 — V2's freezing model; envs stay freezable (not in `live_protos`).
