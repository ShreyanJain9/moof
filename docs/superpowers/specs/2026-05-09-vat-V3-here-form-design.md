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

V3 also lands two new ergonomic primitives that make Ruby-rich envs real in moof:

- **`Closure:callIn:withSelf:`** — a substrate primitive that runs a closure body with an explicit `call_env` and `self`. the irreducible bytecode-level escape hatch.
- **`Object:eval:`** — written in moof on top of the primitive. allocates a body env that "views" the receiver via a `view-target` meta marker (no mutation of receiver — works on frozen objects). lookups AND mutations in the closure body propagate live, capturing from BOTH the receiver's slots and the closure's captured env.

V3 also purifies `if` along the same line as `def` and `set!` — three opcode/special-form duplicates removed in one phase. a compile-time **peephole optimizer** recognizes the if-macro's expanded shape and emits Jump-based bytecode (no closure allocations) when the args are syntactic `(fn () …)` literals — purity preserved at the source level, performance preserved at the bytecode level.

V3 does **not** include: lexical-env-as-first-class-Form for vau / fexpr support (V8); cross-vat closure travel and `$here` rebinding (V5); persistence inheritance (V9).

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

## 6. `Object:eval:` — full live forwarding via view-env (Ruby instance_eval semantics)

V3 ships true Ruby `instance_eval` — lookups and mutations BOTH propagate live to BOTH the receiver and the closure's captured env, **without mutating the receiver**. works on frozen receivers. implementation splits cleanly: one rust primitive, the rest is moof.

### 6.1. the rust primitive: `Closure:callIn:withSelf:`

Installed as a native on `protos.closure`:

```rust
world.install_native(world.protos.closure, "callIn:withSelf:", |w, self_, args| {
    let closure_id = self_.as_form_id().ok_or_else(|| {
        RaiseError::new(w.intern("type-error"), ":callIn:withSelf: on non-closure")
    })?;
    let env_id = args[0].as_form_id().ok_or_else(|| {
        RaiseError::new(w.intern("type-error"), ":callIn: requires a Form env")
    })?;
    let new_self = args[1];
    let body = w.form_slot(closure_id, w.body_sym).as_form_id().ok_or_else(|| {
        RaiseError::new(w.intern("type-error"), "closure has no :body")
    })?;
    // Run the closure body with the explicit env and self.
    // closure's own :env slot is ignored — caller controls scope.
    run_method(w, body, env_id, new_self, FormId::NONE)
})?;
```

This is the irreducible bytecode-level escape hatch: "run this closure's body in this env, with this self." Everything else (eval, instance_eval, vau-flavored composition) builds on it.

### 6.2. the view-env mechanism: `view-target` meta key

Rather than mutating the receiver's parent (the splice approach), V3 introduces a non-mutating env walker extension. A new substrate-internal meta key `view-target` is recognized by `World::env_lookup` and `World::env_set`:

```rust
// New entry in world.rs's reserved-meta-symbol caches:
view_target_sym: SymId,    // interned at boot from "view-target"
```

`env_lookup` is extended (additively) to also check the view-target's slots when walking:

```rust
pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
    let mut cur = env;
    loop {
        let f = self.heap.get(cur);
        // 1. own slots
        if let Some(v) = f.slots.get(&name).copied() {
            return Some(v);
        }
        // 2. view-target's slots, if set (V3: live forwarding into another Form
        //    without mutating its parent chain)
        if let Some(target_v) = f.meta.get(&self.view_target_sym).copied() {
            if let Some(target_id) = target_v.as_form_id() {
                let tf = self.heap.get(target_id);
                if let Some(v) = tf.slots.get(&name).copied() {
                    return Some(v);
                }
            }
        }
        // 3. walk parent
        let parent = self.form_meta(cur, self.parent_sym);
        match parent {
            Value::Form(id) => cur = id,
            _ => return None,
        }
    }
}
```

`env_set` (the chain-walker for `:set:to:`) gains the same check:

```rust
pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> Result<bool, RaiseError> {
    let mut cur = env;
    loop {
        // 1. own slots — direct hit
        let bound_in_self = self.heap.get(cur).slots.contains_key(&name)
            || self.nursery_deltas.get(&cur).map_or(false, |d| d.slots.contains_key(&name));
        if bound_in_self {
            self.form_slot_set(cur, name, value)?;
            return Ok(true);
        }
        // 2. view-target's slots — if hit, write to view-target LIVE
        if let Some(target_v) = self.form_meta(cur, self.view_target_sym).as_form_id() {
            let bound_in_view = self.heap.get(target_v).slots.contains_key(&name)
                || self.nursery_deltas.get(&target_v).map_or(false, |d| d.slots.contains_key(&name));
            if bound_in_view {
                self.form_slot_set(target_v, name, value)?;
                return Ok(true);
            }
        }
        // 3. walk parent
        let parent = self.form_meta(cur, self.parent_sym);
        match parent {
            Value::Form(id) => cur = id,
            _ => return Ok(false),
        }
    }
}
```

The change is purely additive: forms without `view-target` meta behave identically to before V3. Forms WITH `view-target` get the live-forwarding semantic.

For env-Forms with view-target set:
- **Lookup order:** own slots → view-target's slots → parent's chain (which itself may have view-targets — recursive).
- **Mutation order (`:set:to:`):** own slots → view-target's slots (writes to view-target LIVE) → parent's chain. First match wins.
- **Bind (`:bind:to:`):** writes to receiver's own slots only — never to view-target. body-local semantics.

### 6.3. `Object:eval:` — moof implementation

Lives in `lib/stdlib/object.moof`:

```moof
;; [obj eval: closure] — runs `closure` with obj's slots as a "view"
;; into the lookup chain. lookups and mutations propagate LIVE to both
;; obj and closure.env. obj is NOT mutated (no splice — uses the
;; substrate's view-target meta key for non-mutating delegation).
(defmethod (Object eval: closure)
  (let ((captured-env [Heap slotOf: closure at: 'env])
        (body-env [Env new]))
    ;; build the body env with parent = closure.captured_env, view-target = self
    [Heap metaSet: body-env at: 'parent to: captured-env]
    [Heap metaSet: body-env at: 'view-target to: self]
    ;; run the closure body in body-env with self bound to obj
    [closure callIn: body-env withSelf: self]))
```

Lookup chain inside the closure body:

```
body-env's own slots          (let-bindings declared in body)
   ↓ via view-target
self's own slots              (obj's instance state — LIVE)
   ↓ via body-env.parent
closure.captured_env          (closure's lexical chain)
   ↓ ... → globals
```

Both lookups and mutations are live:

- **Lookups** find names from body-locals, then obj's slots, then closure's captured chain, then globals. True capture from both.
- **`(set! name value)`** walks the chain via `:set:to:`. If `name` is body-local: writes to body-env. If in obj: writes to obj LIVE (via view-target's `env_set` hit). If in captured-env: writes there LIVE. Raises `'unbound` if not reachable.
- **`[$here bind: …]`** (i.e. `def`) targets `$here` (global) as always — orthogonal to eval scope.
- **`[obj :bind:to:]`** within body — mutates obj directly. Live.

### 6.4. why view-env over splice

V3 considered both:

- **Splice** (mutate receiver.parent temporarily, save/restore): simpler, but requires receiver to be mutable — frozen objects raise `'frozen-form`. multi-threaded futures (V8+) would have visibility concerns.
- **View-env** (non-mutating, via the new `view-target` meta key): walker change is small (~10 lines per accessor). works on frozen receivers. clean for future multi-actor.

V3 ships view-env. The walker change is a one-time substrate cost; the payoff is symmetric semantics across frozen and mutable receivers, plus future-proofing for V5 cross-vat scenarios.

### 6.5. forward implication: vau / fexpr come almost for free in V8

The `Closure:callIn:withSelf:` primitive is exactly what a vau / fexpr uses to evaluate code in a captured caller env. V8's vau syntax can reduce to this primitive plus `[Env current]` (V3 already ships) — no new substrate machinery. V3 establishes the foundation; V8 adds surface syntax.

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

### 10.4. peephole optimization (in V3)

V3 ships a compile-time peephole optimizer that recognizes the if-macro's expanded shape and emits Jump-based bytecode — recovering the perf without sacrificing macro purity at the source level.

**The optimizer's trigger:** in the compiler's send-emission path (rust seed `compile_send`, moof Compiler `compileSend:`), when about to emit `Send :ifTrue:ifFalse: arity=2`, check the AST shape of the args:

- if both args are syntactic `(fn () body)` literals (i.e. zero-arg `fn` forms with no captures from outside their own body's let-bindings — though for V3's recognition pass, we just check the syntactic shape, not closure analysis), AND
- the receiver is the result of a `Send :!!` (the if-macro signature),

then **emit Jump-based bytecode inline**:

```
<compile c>
Send :!! arity=0           ;; coerce to Bool (still dispatches normally — Bool's :!! is identity, etc.)
JumpIfFalse else_label
<inline-compile t-body>
Jump end_label
else_label:
<inline-compile e-body>
end_label:
```

The closure-Form allocations are skipped entirely. The branches' bodies are inlined into the parent chunk. Same bytecode shape as the pre-V3 special-form `compile_if`.

If either arg is NOT a syntactic closure (e.g., a let-bound variable holding a closure, or a method-call result), the optimizer **does not trigger** — falls back to standard send dispatch. This preserves user-overridability: anyone can write `(let ((t-thunk (fn () "hi"))) [c ifTrue: t-thunk ifFalse: e-thunk])` and get standard method dispatch through whatever `:ifTrue:ifFalse:` Bool resolves to.

Note: the optimizer works on the **post-macro-expansion** form. The if-macro emits the standard `[[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]` shape; the optimizer just recognizes that shape and short-circuits to Jump-based bytecode. Users who redefine the if macro to expand differently get the new expansion compiled normally — no special-casing.

Same optimizer pattern can land in V3.5+ for other duplicate-recognized shapes if profiling shows them. For V3, it's just `:ifTrue:ifFalse:` with closure args.

### 10.5. fallback when `:!!` isn't trivially Bool-returning

The optimizer assumes `Send :!!` produces a Bool that the JumpIfFalse can branch on. Bool's `:!!` is identity (`#true → #true`, `#false → #false`). For Nil, `:!!` returns `#false`. For everything else, `:!!` returns `#true` (per `lib/early/02-bool.moof`'s coercion rules).

If a user installs a non-standard `:!!` that returns something else, the JumpIfFalse falls through to the truthy branch (because anything non-#false is truthy at the bytecode level). This is the same semantic as without the optimizer — `:ifTrue:ifFalse:` would also dispatch on whatever Bool the receiver is.

Edge case: if a user installs `:!!` to raise, the optimizer triggers the raise (good). If `:!!` returns nil-ish but not exactly Nil, the JumpIfFalse's "is this #false or Nil" check (existing bytecode semantic) handles it.

No edge case requires special handling in the optimizer. The optimizer is semantically transparent: same observable behavior with or without it, modulo the closure-allocation perf difference.

## 11. boot order

`intrinsics::install` (already inside the V1 boot turn) is extended:

1. Existing: install all current natives (Object, Cons, Nil, Bool, Integer, Float, Symbol, Char, String, Bytes, Cons, Table, Method, Chunk, Closure, Env, Foreign, Frame snapshot machinery, $out / $err caps, $compiler cap, $transporter, $hash).
2. **V3 additions:**
   - intern `view-target` symbol on `World` and cache as `view_target_sym`.
   - extend `World::env_lookup` and `World::env_set` to consult `view-target` meta key (additive — backward-compatible).
   - install Env proto methods: `:bind:to:`, `:set:to:`, `:lookup:`, `:parent`, `:current` on `protos.env`.
   - install Closure proto method: `:callIn:withSelf:` on `protos.closure` (rust native — the irreducible "run closure with explicit env+self" primitive).
   - bind `$here` in `here_form.slots` (self-reference).
   - `lib/stdlib/object.moof`: add `(defmethod (Object eval: closure) …)` using `:callIn:withSelf:` and the view-target meta marker. moof code, user-overridable.
   - rust seed `compile_send` and moof Compiler `compileSend:`: add the if-shape peephole optimizer for `Send :ifTrue:ifFalse:` with syntactic-closure args.

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
3. Env proto has `:bind:to:`, `:set:to:`, `:lookup:`, `:parent`, `:current` methods (5 new rust natives).
4. Closure proto has `:callIn:withSelf:` method (1 new rust native) — the irreducible primitive for `:eval:` and future vau / fexpr.
5. Object proto has `:eval:` method (moof, in `lib/stdlib/object.moof`) — view-env-based; works on frozen receivers; live forwarding for both lookups and mutations.
6. `World::env_lookup` and `World::env_set` extended to consult `view-target` meta key (additive — backward-compatible with all V0/V1/V2 invariants).
7. `Op::DefineGlobal` and `Op::StoreName` removed from `crates/substrate/src/opcodes.rs`. VM handlers and encode/decode entries also removed.
8. Rust seed `compile_def` emits Send-based bytecode (no `Op::DefineGlobal`).
9. Rust seed `compile_if` emits Send-based bytecode (`[[c !!] ifTrue: ... ifFalse: ...]` pattern via PushClosure).
10. Moof Compiler `compileDef:`, `compileSet:`, `compileIf:` rewritten to emit Send-based bytecode.
11. **Peephole optimizer** in rust seed `compile_send` AND moof Compiler `compileSend:`: recognize `Send :ifTrue:ifFalse:` with syntactic-closure args and emit Jump-based bytecode inline (no closure allocations). triggers on the if-macro's expansion shape; falls back to standard dispatch on other shapes.
12. `lib/early/06-control-macros.moof` has `(defmacro def …)` and `(defmacro set! …)` at the top of the file. `lib/early/10-if-macro.moof` is unchanged (already correct).
13. `lib/compiler/01-dispatch.moof::compileForm:chunk:` checks macros before special-form dispatch.
14. All 481 V2 tests still pass; new V3 tests cover: `(def x 42)` then `x` → 42 (regression for the macro path); `(set! foo 0)` inside `(let ((foo 5)) ...)` mutates the lexical foo; `(set! unbound-name 0)` raises `'unbound`; `(set! obj-slot 5)` inside `[obj eval: …]` mutates obj's slot LIVE via view-target; `[obj eval: (fn () [self bar])]` returns obj's `bar`; `[obj eval: closure]` finds names from both obj and closure's captured env; **eval against FROZEN obj works** (no mutation of obj); `[Env current]` returns the lexical env of the caller; `[$here lookup: 'foo]`, `[$here parent]`, `[$here bind: 'baz to: 99]` work; `[Heap slotKeysOf: $here]` lists global bindings (including `$here` itself); `(if c t e)` post-peephole compiles to Jump-based bytecode (matches pre-V3 efficiency for the macro pattern); user-defined `[c ifTrue: tThunk ifFalse: eThunk]` with non-literal closures uses standard Send dispatch.

## 13. test plan (sketch)

Unit tests in `crates/substrate/src/world.rs::tests` and `crates/substrate/src/vm.rs::tests`:
- `here_form_is_self_referential_at_boot`
- `env_bind_to_writes_slot_no_walk`
- `env_set_to_walks_chain_returns_value`
- `env_set_to_raises_unbound_when_not_in_chain`
- `env_lookup_walks_chain_nil_on_miss`
- `env_parent_returns_parent_form_or_nil`
- `env_current_returns_caller_env`
- `env_lookup_consults_view_target_after_own_slots`
- `env_lookup_view_target_does_not_recurse_into_target_parent`  (view-target lookup is one-level, doesn't walk target's chain)
- `env_lookup_without_view_target_unchanged`  (regression: V1/V2 invariants preserved when meta key absent)
- `env_set_writes_to_view_target_when_name_found_there`
- `closure_call_in_with_self_runs_body_with_explicit_env`
- `closure_call_in_ignores_closures_own_env_slot`
- `peephole_if_with_syntactic_closures_emits_jump_based`  (verify chunk's bytecode contains JumpIfFalse, not PushClosure-then-Send)
- `peephole_if_with_non_syntactic_args_falls_back_to_send_dispatch`

Integration tests in `crates/substrate/tests/here_e2e.rs`:
- `def_macro_binds_in_here`
- `set_macro_walks_lexical_chain`
- `set_macro_raises_unbound`
- `here_self_reference_works_in_reflection`
- `obj_eval_capture_from_both`
- `obj_eval_set_propagates_live_to_obj_via_view_target`
- `obj_eval_on_frozen_obj_works_no_mutation`  (regression: view-env doesn't require obj mutability)
- `if_macro_post_peephole_runs_efficiently_in_tight_loop`  (sanity: 100k iterations finish in reasonable time — the peephole pays off)
- `user_overridden_if_macro_still_works`  (regression: redefining the if macro changes expansion; peephole gracefully degrades)

Plus regression tests for the existing test suite (V0 / V1 / V2).

## 14. out of scope (deferred)

- **Lexical-env-as-first-class for vau / fexpr**: V8 — adds surface syntax like `(vau (args env) ...)` that captures the caller's env directly at the macro level. V3 already ships the substrate primitive (`Closure:callIn:withSelf:` + `[Env current]`) that vau will compose against — V8 just adds the parser/macro layer.
- **Cross-vat closure travel + `$here` rebinding**: V5 — when a closure travels to another vat, its captured `$here` rebinds to the receiving vat's `$here`.
- **`$here` persistence + journal**: V9 — the env Form persists naturally; the journal records bindings as ordinary slot mutations.
- **Mutation-via-`:bind:to:` on frozen forms**: V2's `'frozen-form` raise applies; if `$here` is ever frozen (V4+ replicated vats might do this), `[$here bind: …]` raises. V3 doesn't change this behavior — just inherits it.
- **`let`, `fn`, `do`, `defmacro`, `quote` purification**: out of V3's scope. These construct envs / frames / macro machinery — substrate primitives, harder to eliminate. Future phase if motivation arises.
- **Recursive view-target chains**: V3's `env_lookup` checks one view-target per env in the chain. If the view-target itself has a view-target meta, V3 doesn't recurse into that. Adding "view-target chains" or "view-target-of-view-target" recursion is a future enhancement if a use case appears (currently doesn't seem useful).
- **More peephole optimizations**: V3 ships ONE peephole — the if-pattern. Future ones (e.g. `[Bool not]` chains, common arithmetic shapes) can land incrementally without changing the V3 architecture.

## see also

- `2026-05-04-vats-and-references-protocol-design.md` §8 — the user-facing spec for the env model V3 implements.
- `2026-05-06-vat-V1-nursery-diff-design.md` — V1's per-turn nursery, which V3 inherits (env mutations journal through the nursery as usual).
- `2026-05-07-vat-V2-freezing-design.md` §4 — V2's freezing model; envs stay freezable (not in `live_protos`).
