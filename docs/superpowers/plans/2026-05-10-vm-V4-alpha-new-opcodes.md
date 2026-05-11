# VM V4 phase α — new opcodes (enum-based) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** introduce the 9 new V4 opcodes (`LoadHere`, `JumpIfTrue`, `SendDynamic`, `SendSelf`, `SendHere`, `TailSendSelf`, `TailSendHere`, `Suspend`, `Resume`) into the rust substrate, with VM dispatch + reflection + compiler emission. **Pure additive change** — no encoding migration, no opcode removals, no semantic changes to existing behavior. Sets up V4-β (byte encoding) + V4-γ (zig host) + V4-δ (OCaml seed compiler) as separate follow-on plans.

**Architecture:** the rust `Op` enum gains 9 variants. each variant gets a VM handler in `vm.rs::step`. each gets encode/decode entries in `intrinsics.rs`'s op-form tables. the rust seed compiler (`compiler.rs`) and moof Compiler (`lib/compiler/`) detect their target patterns at emission time and emit the fused/specialized op instead of the equivalent multi-op sequence. existing Send/LoadSelf/etc paths continue to work as fallback — the fused ops are pure optimizations that the compiler picks when applicable.

**Tech Stack:** Rust 2021, `cargo build --workspace`. Moof code in `lib/compiler/*.moof`. CLI smokes for verification (no rust tests — per project policy).

**Project state (HEAD):** `c896436` — VM V4 spec committed. V3 features (here_form, view-target, become:, perform:withArgs:, IC invalidation, doesNotUnderstand:) all landed. const-fold peephole working. cycle-safe inspect working.

---

## File Structure

| file | role |
|---|---|
| `crates/substrate/src/opcodes.rs` | add 9 enum variants + `pushes()` updates |
| `crates/substrate/src/vm.rs` | add 9 match arms in `step()` (each calls existing helpers like `send_via_ic`) |
| `crates/substrate/src/intrinsics.rs` | add 9 encode + 9 decode entries in the op-form tables (reflection round-trip) |
| `crates/substrate/src/compiler.rs` | rust seed: rewrite `compile_load_name` to emit `LoadHere`; rewrite `compile_send` to emit `SendSelf`/`SendHere`/`TailSendSelf`/`TailSendHere`/`SendDynamic`; rewrite if-emission to use `JumpIfTrue` where shorter |
| `lib/compiler/01-dispatch.moof` | moof Compiler `compileLoadName:` emits `LoadHere` for `$here` |
| `lib/compiler/02-special.moof` | moof Compiler `compileSend:` emits `SendSelf`/`SendHere`/`SendDynamic`; existing if-peephole gains `JumpIfTrue` emission |

---

## Task 1: add 9 enum variants + `pushes()` updates

**Files:**
- Modify: `crates/substrate/src/opcodes.rs`

Pure declaration. No VM behavior change yet — just types.

- [ ] **Step 1: add the variants to `Op`**

In `crates/substrate/src/opcodes.rs`, add after the existing variants (and before the `impl Op` block):

```rust
    /// V4 — push `Value::Form(world.here_form)` directly. saves the
    /// most common env walk (LoadName('$here')). substrate-internal:
    /// uses the canonical here_form FormId, bypasses any user-level
    /// rebinding of the `$here` symbol.
    LoadHere,

    /// V4 — dual of JumpIfFalse. pop a value; if truthy, jump.
    /// saves a `:!` send when the if-peephole generates the inverse
    /// branch shape.
    JumpIfTrue(i16),

    /// V4 — dynamic-selector send. selector is on the stack (top),
    /// then args, then receiver (top-1-argc). pops sel + args + recv;
    /// pushes result. used by `:perform:withArgs:` and (future) vau.
    /// `ic_idx` caches the dispatched proto+method as usual.
    SendDynamic { argc: u8, ic_idx: u16 },

    /// V4 — fused `LoadSelf;Send`. receiver is `current_frame.self_`;
    /// no stack pop for receiver. pure dispatch optimization;
    /// reflection round-trips through a distinct op-name.
    SendSelf { selector: SymId, argc: u8, ic_idx: u16 },

    /// V4 — fused `LoadHere;Send`. receiver is
    /// `Value::Form(world.here_form)`. same caveat as `LoadHere`:
    /// bypasses user-level `$here` rebinding.
    SendHere { selector: SymId, argc: u8, ic_idx: u16 },

    /// V4 — tail-position SendSelf. replaces current frame instead
    /// of pushing a new one. no IC field (matches TailSend; flagged
    /// as future work in spec §6.2).
    TailSendSelf { selector: SymId, argc: u8 },

    /// V4 — tail-position SendHere. same caveats as SendHere +
    /// TailSendSelf.
    TailSendHere { selector: SymId, argc: u8 },

    /// V4 — yield current frame to a promise's wait queue. phase D
    /// semantics; in V4-α this op is a placeholder that raises
    /// 'unimplemented when executed (the encoding/decoding round-
    /// trip works; the scheduler doesn't exist yet).
    Suspend { promise_ic: u16 },

    /// V4 — install a saved Frame-Form's state. phase D continuation
    /// primitive. same placeholder status as Suspend.
    Resume { frame_ic: u16 },
```

- [ ] **Step 2: update `pushes()` for the new ops**

In the existing `impl Op { pub fn pushes(self) -> bool { ... } }`, add the new variants to the list of "pushes 1 element" patterns:

```rust
    pub fn pushes(self) -> bool {
        matches!(
            self,
            Op::LoadConst(_)
                | Op::PushNil
                | Op::PushTrue
                | Op::PushFalse
                | Op::Dup
                | Op::LoadName(_)
                | Op::LoadSelf
                | Op::LoadHere                              // V4
                | Op::PushClosure { .. }
        )
    }
```

Note: `Send`-like variants (including all the new ones) push 1 (the result) AFTER popping. They're "pushes" in the sense of leaving net +1 (counting from before the receiver+args). Skip them in `pushes()` — that's a compile-time-balance check helper, not a runtime accounting.

Actually the cleanest reading: `pushes()` returns "this op increases the stack by exactly 1 in isolation, not counting any pops it does." LoadHere fits this (no pops, +1). Send variants do not (they pop receiver+args and push 1, net `-argc` or `-argc-1`). So just adding `LoadHere` is correct.

- [ ] **Step 3: build check**

```bash
cargo build -p moof 2>&1 | tail -5
```

Expected: clean build. The new variants are declared but not yet executed by the VM (no match arms for them). Rust will warn `unused variant Op::LoadHere` etc. — that's expected through Task 2.

- [ ] **Step 4: commit**

```bash
git add crates/substrate/src/opcodes.rs
git commit -m "$(cat <<'EOF'
opcodes: declare V4 opcodes (LoadHere, JumpIfTrue, SendDynamic, ...)

V4 phase α task 1. Pure declaration of 9 new variants:

- LoadHere: direct here_form push (eliminates the env walk)
- JumpIfTrue: dual of JumpIfFalse
- SendDynamic: selector-on-stack send (replaces :perform:withArgs:)
- SendSelf / SendHere: fused LoadSelf/LoadHere + Send
- TailSendSelf / TailSendHere: fused tail variants
- Suspend / Resume: phase-D promise+continuation primitives
  (placeholders in V4-α; full semantics in phase D)

No VM dispatch arms yet (task 2). No compiler emission yet (tasks
3-6). Build will warn about unused variants until then.

Spec: docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md
EOF
)"
```

---

## Task 2: VM dispatch arms for the 9 new opcodes

**Files:**
- Modify: `crates/substrate/src/vm.rs`

Add match arms in `step()` for each new variant. Most reuse existing helpers (`send_via_ic`, `lookup_handler`, `is_truthy`).

- [ ] **Step 1: locate `vm.rs::step()`'s match block**

```bash
grep -n "Op::LoadConst\|Op::Send {" crates/substrate/src/vm.rs | head -5
```

- [ ] **Step 2: add match arms**

In `vm.rs::step()`, add arms inside the existing `match op` block. paste these near the related-op groups:

```rust
        // V4 — direct here_form push
        Op::LoadHere => {
            world.vm.stack.push(Value::Form(world.here_form));
        }

        // V4 — JumpIfTrue (dual of JumpIfFalse)
        Op::JumpIfTrue(offset) => {
            let v = pop(world)?;
            if is_truthy(v) {
                // pc advance happens via the offset; matching
                // JumpIfFalse's `pc = (pc as isize + offset as isize) as usize`
                let frame = world.vm.frames.last_mut().unwrap();
                frame.pc = (frame.pc as isize + offset as isize - 1) as usize;
                // -1 because the outer step loop already advanced pc past Op::JumpIfTrue
                // (verify: check how JumpIfFalse computes; mirror exactly)
            }
        }

        // V4 — dynamic-selector send
        Op::SendDynamic { argc, ic_idx } => {
            let sel = pop(world)?.as_sym().ok_or_else(|| {
                RaiseError::new(
                    world.intern("type-error"),
                    "SendDynamic: selector on stack must be a Symbol",
                )
            })?;
            let (receiver, args) = pop_call_args(world, argc as usize)?;
            let result = world.send_via_ic(receiver, sel, &args, chunk, ic_idx)?;
            world.vm.stack.push(result);
        }

        // V4 — SendSelf (fused LoadSelf+Send)
        Op::SendSelf { selector, argc, ic_idx } => {
            let receiver = world.vm.frames[frame_idx].self_;
            let args = pop_n(world, argc as usize)?;
            let result = world.send_via_ic(receiver, selector, &args, chunk, ic_idx)?;
            world.vm.stack.push(result);
        }

        // V4 — SendHere (fused LoadHere+Send)
        Op::SendHere { selector, argc, ic_idx } => {
            let receiver = Value::Form(world.here_form);
            let args = pop_n(world, argc as usize)?;
            let result = world.send_via_ic(receiver, selector, &args, chunk, ic_idx)?;
            world.vm.stack.push(result);
        }

        // V4 — TailSendSelf (tail-position fused)
        Op::TailSendSelf { selector, argc } => {
            let receiver = world.vm.frames[frame_idx].self_;
            let args = pop_n(world, argc as usize)?;
            // tail-call replacement — same shape as TailSend
            return tail_send_dispatch(world, receiver, selector, args);
        }

        // V4 — TailSendHere (tail-position fused)
        Op::TailSendHere { selector, argc } => {
            let receiver = Value::Form(world.here_form);
            let args = pop_n(world, argc as usize)?;
            return tail_send_dispatch(world, receiver, selector, args);
        }

        // V4 — Suspend placeholder (phase D)
        Op::Suspend { promise_ic: _ } => {
            return Err(RaiseError::new(
                world.intern("unimplemented"),
                "Op::Suspend requires the phase-D promise scheduler",
            ));
        }

        // V4 — Resume placeholder (phase D)
        Op::Resume { frame_ic: _ } => {
            return Err(RaiseError::new(
                world.intern("unimplemented"),
                "Op::Resume requires the phase-D continuation machinery",
            ));
        }
```

**ADAPT NOTES:**

- Verify `pop_call_args(world, argc)` exists (used by existing Send) — yes per grep. It pops `argc + 1` (args + receiver) and returns (receiver, args).
- For `SendSelf`/`SendHere`, we need to pop only `argc` (no receiver in stack). May need a new helper `pop_n(world, n) -> Vec<Value>` that just pops N. Or inline:
  ```rust
  let stack = &mut world.vm.stack;
  let split_at = stack.len() - argc as usize;
  let args: Vec<Value> = stack.split_off(split_at);
  ```
- For `TailSendSelf`/`TailSendHere`, the tail-call logic in existing `Op::TailSend` is the template. Extract a helper `tail_send_dispatch(world, receiver, selector, args)` from the existing arm. The body does: lookup_handler, replace current frame's chunk/pc/env/self/defining_proto, install param bindings.
- For `JumpIfTrue`, the pc-offset logic: read the existing `JumpIfFalse` arm and mirror exactly — moof's interpreter loop advances pc AFTER reading the op, so the offset is relative to the post-op pc. The `-1` adjustment in my snippet may or may not be needed depending on how the existing arm handles this. **Read JumpIfFalse and copy its shape verbatim, only flipping the truthiness condition.**

- [ ] **Step 3: build check**

```bash
cargo build -p moof 2>&1 | tail -10
```

Expected: clean build. Warnings about unused variants should be GONE now (every variant has a match arm).

- [ ] **Step 4: hand-test that Suspend/Resume raise correctly**

```bash
# We can't easily construct a Suspend op from the CLI yet (compiler
# doesn't emit it). Skip this test — verified by code review only.
```

- [ ] **Step 5: regression smoke**

```bash
cargo run --quiet -p moof -- '[1 + 2]' 2>&1 | tail -3      # → 3
cargo run --quiet -p moof -- '(if #true 1 2)' 2>&1 | tail -3  # → 1
cargo run --quiet -p moof -- '$here' 2>&1 | tail -3        # → { Env ... }
cargo run --quiet -p moof -- '(do (def x 42) x)' 2>&1 | tail -3  # → 42
```

Existing functionality must be unchanged.

- [ ] **Step 6: commit**

```bash
git add crates/substrate/src/vm.rs
git commit -m "$(cat <<'EOF'
vm: dispatch arms for V4 opcodes

V4 phase α task 2. Adds match arms in vm.rs::step() for all 9 new
opcodes:

- LoadHere: push Value::Form(world.here_form)
- JumpIfTrue: mirror of JumpIfFalse
- SendDynamic: pop sel from stack, pop args+receiver, send via IC
- SendSelf / SendHere: receiver from frame.self_ or world.here_form;
  pop args; send via IC; push result
- TailSendSelf / TailSendHere: tail-call equivalents; reuse current
  frame
- Suspend / Resume: placeholder raise 'unimplemented (phase D fills in)

Regression smokes confirm existing ops unchanged. New opcodes have
zero emission sites yet (tasks 3-6); the dispatch arms are reachable
only via hand-constructed chunks for now.
EOF
)"
```

---

## Task 3: rust seed `compile_load_name` emits LoadHere for $here

**Files:**
- Modify: `crates/substrate/src/compiler.rs`

Single-line specialization: when the name being loaded is `$here`, emit `LoadHere` instead of `LoadName('$here')`.

- [ ] **Step 1: locate `compile_load_name` or equivalent**

```bash
grep -n "fn compile_load_name\|Op::LoadName" crates/substrate/src/compiler.rs | head -5
```

The rust seed compiler emits `LoadName` directly in the lookup-load path. Find the site.

- [ ] **Step 2: rewrite to detect `$here`**

In the relevant function, before emitting `LoadName(sym)`, check if the symbol resolves to `"$here"`:

```rust
// V4 — fused load: $here resolves to a known FormId.
let here_str = self.world.resolve(sym);
if here_str == "$here" {
    self.emit(Op::LoadHere);
} else {
    self.emit(Op::LoadName(sym));
}
```

- [ ] **Step 3: build + smoke**

```bash
cargo build -p moof 2>&1 | tail -3
cargo run --quiet -p moof -- '$here' 2>&1 | head -3
```

Should still render the full env (the LoadHere path returns the same FormId that LoadName('$here') would have resolved to).

- [ ] **Step 4: verify via reflection**

```bash
cargo run --quiet -p moof -- '(do (def f (fn () $here)) [[f bytecodes] inspect])' 2>&1 | tail -5
```

Look for `{ Opcode op: LoadHere ... }` instead of `{ Opcode op: LoadName operands: #[$here] }`. That confirms the emission path is hit.

- [ ] **Step 5: commit**

```bash
git add crates/substrate/src/compiler.rs
git commit -m "$(cat <<'EOF'
compiler: rust seed emits LoadHere for \$here

V4 phase α task 3. When the rust seed compiler sees a name lookup
for the symbol `\$here`, emit the V4 LoadHere op instead of the
generic LoadName. Eliminates the env-chain walk for the most common
global access.

Reflection: `[chunk bytecodes]` for `(fn () \$here)` now shows
`{ Opcode op: LoadHere operands: #[] }` in place of
`{ Opcode op: LoadName operands: #[\$here] }`.

Semantically equivalent — both resolve to world.here_form — but
LoadHere is one op + one cycle vs an env walk + slot lookup.
EOF
)"
```

---

## Task 4: moof Compiler `compileLoadName:` emits LoadHere

**Files:**
- Modify: `lib/compiler/01-dispatch.moof`

Mirror Task 3 in the moof Compiler, where it lives in moof code.

- [ ] **Step 1: locate `compileLoadName:chunk:`**

```bash
grep -n "compileLoadName:chunk:" lib/compiler/01-dispatch.moof
```

- [ ] **Step 2: rewrite to detect `$here`**

Find:

```moof
(setHandler! Compiler 'compileLoadName:chunk:
  (fn (sym chunk)
    (if [sym is 'self]
        [chunk emit: [Opcode loadSelf]]
        [chunk emit: [Opcode loadName: sym]])))
```

Replace with:

```moof
(setHandler! Compiler 'compileLoadName:chunk:
  (fn (sym chunk)
    (if [sym is 'self]
        [chunk emit: [Opcode loadSelf]]
        (if [sym is '$here]
            ;; V4: direct here_form push — eliminates env walk
            [chunk emit: [Opcode loadHere]]
            [chunk emit: [Opcode loadName: sym]]))))
```

VERIFY: the opcode reflection helper `Opcode loadHere` exists in `intrinsics.rs`'s op-form decode table — which Task 7 will add. **For now, this may fail at compile time** (no decode entry yet). Either complete Task 7 first, OR comment out this change temporarily until Task 7 lands.

**Order resolution:** swap Task 4 and Task 7 — do Task 7 first.

(Rewriting the plan: tasks should land in dependency order. Let me re-flag below.)

- [ ] **Step 3: smoke + commit (after Task 7)**

```bash
cargo run --quiet -p moof -- '(do (def f (fn () $here)) [[f bytecodes] inspect])' 2>&1 | tail -3
```

Same as Task 3 verification.

```bash
git add lib/compiler/01-dispatch.moof
git commit -m "$(cat <<'EOF'
compiler: moof Compiler emits LoadHere for \$here

V4 phase α task 4. Mirrors task 3's rust seed change. After this
commit, both compilers emit LoadHere for `\$here` references.
EOF
)"
```

---

## Task 5: rust seed `compile_send` emits SendSelf / SendHere

**Files:**
- Modify: `crates/substrate/src/compiler.rs`

Specialize `compile_send` to detect self/$here receivers and emit the fused variants.

- [ ] **Step 1: locate `compile_send` (which already does V3 work)**

```bash
grep -n "fn compile_send" crates/substrate/src/compiler.rs
```

- [ ] **Step 2: add the receiver-source check**

At the head of `compile_send`, before the existing receiver-compile step, check the receiver's symbolic identity:

```rust
fn compile_send(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
    // ... existing if-peephole + setup ...
    let receiver = elems[1];
    let selector_sym = elems[2].as_sym()
        .ok_or_else(|| self.err("__send__ selector must be a symbol"))?;
    let argc = elems.len() - 3;  // minus __send__, receiver, selector

    // V4 — fused send for `self` and `$here` receivers
    let receiver_kind = match receiver {
        Value::Sym(s) if self.world.resolve(s) == "self" => Some("self"),
        Value::Sym(s) if self.world.resolve(s) == "$here" => Some("here"),
        _ => None,
    };

    if let Some(kind) = receiver_kind {
        // compile args (no receiver — implicit)
        for &arg in &elems[3..] {
            self.compile_expr(arg, false)?;
        }
        let argc_u8 = argc as u8;
        let ic_idx = self.next_ic();
        let op = match (kind, tail) {
            ("self", false) => Op::SendSelf {
                selector: selector_sym, argc: argc_u8, ic_idx,
            },
            ("self", true) => Op::TailSendSelf {
                selector: selector_sym, argc: argc_u8,
            },
            ("here", false) => Op::SendHere {
                selector: selector_sym, argc: argc_u8, ic_idx,
            },
            ("here", true) => Op::TailSendHere {
                selector: selector_sym, argc: argc_u8,
            },
            _ => unreachable!(),
        };
        self.emit(op);
        return Ok(());
    }

    // existing path: compile receiver, then args, then emit Send/TailSend
    // ... unchanged ...
}
```

Note: the `self` symbol's specialization to `LoadSelf` already happens in `compile_load_name`. Without this fusion, the bytecode would be `LoadSelf; <args>; Send`. With it, `<args>; SendSelf`. one fewer op.

- [ ] **Step 3: build + smoke**

```bash
cargo build -p moof 2>&1 | tail -3
cargo run --quiet -p moof -- '[$here parent]' 2>&1 | tail -3   # → nil (or "<root>")
cargo run --quiet -p moof -- '(do (def m (defmethod (Object foo) [self proto])) [Object foo])' 2>&1 | tail -3   # → Object
```

Both should still work — semantically equivalent paths.

- [ ] **Step 4: verify reflection**

```bash
cargo run --quiet -p moof -- '(do (def f (fn () [$here parent])) [[f bytecodes] inspect])' 2>&1 | tail -3
```

Look for `{ Opcode op: SendHere ... }` instead of `LoadHere + Send`.

- [ ] **Step 5: commit**

```bash
git add crates/substrate/src/compiler.rs
git commit -m "$(cat <<'EOF'
compiler: rust seed emits SendSelf/SendHere fused variants

V4 phase α task 5. compile_send detects receivers `self` and `\$here`
at emission time and emits the fused V4 opcodes:

- `[self foo …]`  → SendSelf  (or TailSendSelf in tail position)
- `[\$here foo …]` → SendHere  (or TailSendHere in tail position)

Eliminates the LoadSelf/LoadHere op + a dispatch cycle per fused
send. Reflection round-trips through distinct op-names.

Receivers that aren't statically `self` or `\$here` continue to use
the existing receiver-then-Send path unchanged.
EOF
)"
```

---

## Task 6: moof Compiler `compileSend:` emits SendSelf / SendHere

**Files:**
- Modify: `lib/compiler/02-special.moof`

Mirror Task 5 in moof.

- [ ] **Step 1: locate `compileSend:chunk:tail:`**

```bash
grep -n "compileSend:chunk:tail:" lib/compiler/02-special.moof | head
```

- [ ] **Step 2: insert fused-send detection before the existing flow**

Find the section that compiles the receiver. Add the recv-source check BEFORE compiling the receiver:

```moof
(setHandler! Compiler 'compileSend:chunk:tail:
  (fn (rest chunk tail)
    (let ((receiver [Heap slotOf: rest at: 'car])
          (selector [Heap slotOf: [Heap slotOf: rest at: 'cdr] at: 'car])
          (args [Heap slotOf: [Heap slotOf: rest at: 'cdr] at: 'cdr]))

      ;; V4 — fused send for `self` and `$here` receivers
      (let ((recvKind (if [self symbol?: receiver]
                          (if [receiver is 'self] 'self
                          (if [receiver is '$here] 'here
                          #false))
                          #false)))
        (if [recvKind = #false]
            ;; not fused — fall through to existing const-fold +
            ;; if-peephole + standard path (unchanged)
            <existing body wrapped here>
            ;; fused — compile args only, emit fused op
            (do
              [self compileArgs: args chunk: chunk]
              (let ((argc [self argc: args])
                    (ic [chunk addIc]))
                (if [recvKind = 'self]
                    (if tail
                        [chunk emit: [Opcode tailSendSelf: selector argc: argc]]
                        [chunk emit: [Opcode sendSelf: selector argc: argc ic: ic]])
                    (if tail
                        [chunk emit: [Opcode tailSendHere: selector argc: argc]]
                        [chunk emit: [Opcode sendHere: selector argc: argc ic: ic]])))))))))
```

**ADAPT NOTES:**
- The exact Opcode constructor names (`sendSelf:argc:ic:` vs `sendSelf:argc:ic:`) depend on the moof-side Opcode helper module. The convention in V3 was `[Opcode send: sel argc: n ic: ic]`. So: `[Opcode sendSelf: sel argc: n ic: ic]`. These need to be defined in the moof `Opcode` module — Task 7 will add them when it sets up the op-form encode/decode side.
- The body of "fall through" is the existing compileSend: code (the const-fold check + if-peephole + standard send path). Use a `let` to bind the original body via a helper method or just inline the existing structure as the else-branch. Either way works.

Verify: the change is purely additive — the existing structure is preserved as the `else` branch.

- [ ] **Step 3: build + smoke + commit**

(After Task 7 lands the Opcode helpers.)

```bash
cargo run --quiet -p moof -- '[$here parent]' 2>&1 | tail -3
```

```bash
git add lib/compiler/02-special.moof
git commit -m "$(cat <<'EOF'
compiler: moof Compiler emits SendSelf/SendHere fused variants

V4 phase α task 6. Mirrors task 5 in moof code. The moof Compiler
now detects `self` and `\$here` receivers at compileSend: emission
and produces the fused V4 opcodes.
EOF
)"
```

---

## Task 7: reflection — encode/decode for the 9 new opcodes

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs`

`intrinsics.rs` has op-form encode/decode tables that translate `Op` enum variants ↔ moof `Opcode` Forms (for `[chunk bytecodes]` reflection + the moof Compiler's `[chunk emit: [Opcode ...]]` API). Each new variant needs entries.

- [ ] **Step 1: locate the encode/decode tables**

```bash
grep -n "Op::DefineGlobal\|Op::Send {\|fn op_to_form\|fn form_to_op\|mk_op_form" crates/substrate/src/intrinsics.rs | head -10
```

There should be two paths:
- **encode** (Op → Form): converts an in-memory Op variant to a `{ Opcode op: 'X operands: ... }` Form for reflection.
- **decode** (Form → Op): converts a Form (from `[chunk emit: ...]` calls in the moof Compiler) back to an in-memory Op.

- [ ] **Step 2: add encode entries**

For each new variant, add a case in the encode mapping. Example for `LoadHere`:

```rust
Op::LoadHere => mk_op_form(world, "LoadHere", vec![]),
Op::JumpIfTrue(offset) => mk_op_form(world, "JumpIfTrue", vec![Value::Int(offset as i64)]),
Op::SendDynamic { argc, ic_idx } => mk_op_form(world, "SendDynamic", vec![
    Value::Int(argc as i64),
    Value::Int(ic_idx as i64),
]),
Op::SendSelf { selector, argc, ic_idx } => mk_op_form(world, "SendSelf", vec![
    Value::Sym(selector),
    Value::Int(argc as i64),
    Value::Int(ic_idx as i64),
]),
Op::SendHere { selector, argc, ic_idx } => mk_op_form(world, "SendHere", vec![
    Value::Sym(selector),
    Value::Int(argc as i64),
    Value::Int(ic_idx as i64),
]),
Op::TailSendSelf { selector, argc } => mk_op_form(world, "TailSendSelf", vec![
    Value::Sym(selector),
    Value::Int(argc as i64),
]),
Op::TailSendHere { selector, argc } => mk_op_form(world, "TailSendHere", vec![
    Value::Sym(selector),
    Value::Int(argc as i64),
]),
Op::Suspend { promise_ic } => mk_op_form(world, "Suspend", vec![
    Value::Int(promise_ic as i64),
]),
Op::Resume { frame_ic } => mk_op_form(world, "Resume", vec![
    Value::Int(frame_ic as i64),
]),
```

- [ ] **Step 3: add decode entries**

Mirror each in the decode table (the function the moof Compiler's `[chunk emit: [Opcode ...]]` calls):

```rust
"LoadHere" => Op::LoadHere,
"JumpIfTrue" => {
    let offset = need_int(world, "JumpIfTrue", &operands, 0)? as i16;
    Op::JumpIfTrue(offset)
}
"SendDynamic" => {
    let argc = need_int(world, "SendDynamic", &operands, 0)? as u8;
    let ic_idx = need_int(world, "SendDynamic", &operands, 1)? as u16;
    Op::SendDynamic { argc, ic_idx }
}
"SendSelf" => {
    let selector = need_sym(world, "SendSelf", &operands, 0)?;
    let argc = need_int(world, "SendSelf", &operands, 1)? as u8;
    let ic_idx = need_int(world, "SendSelf", &operands, 2)? as u16;
    Op::SendSelf { selector, argc, ic_idx }
}
"SendHere" => {
    let selector = need_sym(world, "SendHere", &operands, 0)?;
    let argc = need_int(world, "SendHere", &operands, 1)? as u8;
    let ic_idx = need_int(world, "SendHere", &operands, 2)? as u16;
    Op::SendHere { selector, argc, ic_idx }
}
"TailSendSelf" => {
    let selector = need_sym(world, "TailSendSelf", &operands, 0)?;
    let argc = need_int(world, "TailSendSelf", &operands, 1)? as u8;
    Op::TailSendSelf { selector, argc }
}
"TailSendHere" => {
    let selector = need_sym(world, "TailSendHere", &operands, 0)?;
    let argc = need_int(world, "TailSendHere", &operands, 1)? as u8;
    Op::TailSendHere { selector, argc }
}
"Suspend" => {
    let promise_ic = need_int(world, "Suspend", &operands, 0)? as u16;
    Op::Suspend { promise_ic }
}
"Resume" => {
    let frame_ic = need_int(world, "Resume", &operands, 0)? as u16;
    Op::Resume { frame_ic }
}
```

- [ ] **Step 4: moof Opcode helpers**

The moof Compiler's `[Opcode loadHere]` etc. constructors need to exist. They're typically in a moof file that defines the Opcode "module" (or proto). Search:

```bash
grep -rn "Opcode loadName\|Opcode send:\|Opcode pop\|Opcode return" lib/ --include="*.moof" | head -5
```

This will surface where the Opcode constructors are defined. Add new constructors mirroring the existing ones:

```moof
(setHandler! Opcode 'loadHere
  (fn () [Opcode new: 'LoadHere operands: nil]))

(setHandler! Opcode 'jumpIfTrue:
  (fn (offset) [Opcode new: 'JumpIfTrue operands: (list offset)]))

(setHandler! Opcode 'sendSelf:argc:ic:
  (fn (sel argc ic) [Opcode new: 'SendSelf operands: (list sel argc ic)]))

(setHandler! Opcode 'sendHere:argc:ic:
  (fn (sel argc ic) [Opcode new: 'SendHere operands: (list sel argc ic)]))

(setHandler! Opcode 'tailSendSelf:argc:
  (fn (sel argc) [Opcode new: 'TailSendSelf operands: (list sel argc)]))

(setHandler! Opcode 'tailSendHere:argc:
  (fn (sel argc) [Opcode new: 'TailSendHere operands: (list sel argc)]))

(setHandler! Opcode 'sendDynamic:ic:
  (fn (argc ic) [Opcode new: 'SendDynamic operands: (list argc ic)]))

(setHandler! Opcode 'suspend:
  (fn (promise-ic) [Opcode new: 'Suspend operands: (list promise-ic)]))

(setHandler! Opcode 'resume:
  (fn (frame-ic) [Opcode new: 'Resume operands: (list frame-ic)]))
```

**ADAPT**: the actual `Opcode` constructor pattern depends on local convention. Read the existing `[Opcode loadName: sym]` definition first. The shape `[Opcode <name>: <ops>]` is typical.

- [ ] **Step 5: build + smoke**

```bash
cargo build -p moof 2>&1 | tail -3
cargo run --quiet -p moof -- '(do (def f (fn () $here)) [[f bytecodes] inspect])' 2>&1 | tail -3
```

After Tasks 3+7, this should show `{ Opcode op: LoadHere ... }`. After Tasks 5+7, this should show `{ Opcode op: SendHere ... }` for `[$here ...]` calls.

- [ ] **Step 6: commit**

```bash
git add crates/substrate/src/intrinsics.rs lib/
git commit -m "$(cat <<'EOF'
intrinsics+moof: reflection encode/decode for V4 opcodes

V4 phase α task 7. Adds encode (Op → Form) and decode (Form → Op)
entries for the 9 new opcodes in intrinsics.rs's op-form tables.

Also adds moof-side Opcode constructors (`[Opcode loadHere]`,
`[Opcode sendSelf: argc: ic:]`, etc.) so the moof Compiler can emit
the new ops via `[chunk emit: [Opcode ...]]`.

After this, `[chunk bytecodes]` correctly decodes V4 ops, and tasks
4 + 6 (moof Compiler emission of LoadHere + SendSelf/SendHere) can
proceed.
EOF
)"
```

---

## Task 8: if-peephole emits JumpIfTrue when shorter

**Files:**
- Modify: `crates/substrate/src/compiler.rs` (rust seed if-peephole)
- Modify: `lib/compiler/02-special.moof` (moof Compiler if-peephole)

The V3 if-peephole emits `<compile c>; Send :!! argc=0; JumpIfFalse else_label; ...`. With `JumpIfTrue`, certain inverse shapes save a `:!` send. The transformation is opportunistic.

**Honest scope:** the V3 if-shape (`[c !!]; JumpIfFalse ...`) is the canonical shape and rarely benefits from `JumpIfTrue` (it would need an inverted source like `(if (not c) t e)`). For V4-α, this task is mostly about WIRING UP `JumpIfTrue` so the substrate has it available. The compiler may not have a hot pattern that uses it yet.

- [ ] **Step 1: add JumpIfTrue emission point**

In the if-peephole (rust seed), wherever we emit `JumpIfFalse`, allow the inverse shape:

```rust
// V4 — when the source is (if (not c) t e), the compiler could
// recognize this and emit JumpIfTrue instead of inverting via :!.
// For now, we just have it available as an op. Future const-fold +
// shape detection can use it.
```

This task is mostly a no-op for the rust seed in V4-α. **Skip the rust change** for now; revisit when const-fold or peephole gains support for `(if (not c) ...)`.

- [ ] **Step 2: same for moof Compiler**

Same: no-op for V4-α.

- [ ] **Step 3: smoke test that the op is reachable**

Manually construct a chunk that uses JumpIfTrue via the moof Opcode API:

```bash
cargo run --quiet -p moof -- '(do
  (def c [Chunk new: nil source: (quote (test))])
  [c emit: [Opcode pushTrue]]
  [c emit: [Opcode jumpIfTrue: 3]]
  [c emit: [Opcode pushNil]]
  [c emit: [Opcode return]]
  [c emit: [Opcode pushConst: 1]]
  [c emit: [Opcode return]]
  ;; [c run]  ;; if there's a way to invoke a chunk directly
)' 2>&1 | tail -3
```

If we can invoke the chunk and it returns the post-JumpIfTrue value, the op works. (This may not work directly via the CLI — could be a no-op for this task.)

- [ ] **Step 4: commit (with note about future use)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
compiler: JumpIfTrue is available; future peepholes will use it

V4 phase α task 8. JumpIfTrue is declared in task 1, dispatched in
task 2, and reflectable via task 7. The compiler does not yet have
a peephole pattern that emits JumpIfTrue (the V3 if-peephole always
uses JumpIfFalse for the standard if-shape).

Future work: a peephole on `(if (not c) ...)` shapes could emit
JumpIfTrue to avoid the `:!` inversion send.

The op is wired and reachable; just not yet emitted by the canonical
compiler paths.
EOF
)"
```

---

## Task 9: SendDynamic — replace `:perform:withArgs:` overhead

**Files:**
- Modify: `crates/substrate/src/compiler.rs` (rust seed)
- Modify: `lib/compiler/02-special.moof` (moof Compiler)

When `compileSend:` sees the selector `perform:withArgs:`, it can emit `SendDynamic` instead. The runtime then does the dynamic-selector dispatch directly, avoiding the moof-level `:perform:withArgs:` native (which packages args + calls `World::send`).

- [ ] **Step 1: detection in moof Compiler `compileSend:`**

Add to the head of `compileSend:` (or just before standard emission):

```moof
;; V4 — replace [recv perform: sel withArgs: args] with SendDynamic
(if (if [selector is 'perform:withArgs:]
        [args count = 2]   ;; sel + args-list
        #false)
    ;; emit SendDynamic
    (let ((selArg [args car])
          (argsArg [[args cdr] car]))
      [self compileForm: receiver chunk: chunk tail: #false]
      [self compileForm: argsArg chunk: chunk tail: #false]
      ;; the args are a runtime list; we need to "splice" them onto
      ;; the stack. SendDynamic expects sel + receiver + args on stack.
      ;; Hmm — this is tricky because the args-arg is a *list*, not
      ;; pre-spread args. We'd need a runtime helper to spread.
      ...
      ;; ... actually this is more involved than expected.
      ...)
    ;; otherwise, normal path
    ...)
```

**Wait — the encoding mismatch makes this nontrivial.**

`SendDynamic` expects args to be ON THE STACK individually, not as a list. `:perform:withArgs:` takes args as a list (because the caller might pass a runtime-computed args list).

For the FUSED case where the args list is a syntactic literal (`[obj perform: 'foo withArgs: '(a b)]`), we COULD spread at compile time and emit SendDynamic with the actual ops. For the dynamic case, we still need the list-walking helper.

**Decision for V4-α:** skip the SendDynamic fusion for now. The op exists for future use (e.g., wave C vau when we want eval-as-dispatch). The current `:perform:withArgs:` native is fine; not a perf bottleneck.

- [ ] **Step 2: commit (acknowledging skip)**

Nothing to commit for this task. SendDynamic is wired (task 1-2-7) but unused by the canonical compiler. Future plans will add emission when the use case is concrete.

---

## Task 10: final integration + benchmark + push

- [ ] **Step 1: build clean**

```bash
cargo build --workspace 2>&1 | tail -5
```

Expected: no warnings, no errors.

- [ ] **Step 2: full smoke battery**

```bash
echo "=== arithmetic ==="
cargo run --quiet -p moof -- '[1 + 2]' 2>&1 | tail -1
cargo run --quiet -p moof -- '[3 * [4 + 5]]' 2>&1 | tail -1

echo "=== control flow ==="
cargo run --quiet -p moof -- '(if #true 1 2)' 2>&1 | tail -1
cargo run --quiet -p moof -- '(if #false 1 2)' 2>&1 | tail -1

echo "=== def/set! ==="
cargo run --quiet -p moof -- '(do (def x 1) (set! x 99) x)' 2>&1 | tail -1

echo "=== live edit ==="
cargo run --quiet -p moof -- '(do
  (setHandler! Object (quote g) (fn () "v1"))
  [$out say: [Object g]]
  (setHandler! Object (quote g) (fn () "v2"))
  [Object g])' 2>&1 | tail -2

echo "=== become: ==="
cargo run --quiet -p moof -- '(do
  (def a [Object new])
  (def b [Object new])
  (setHandler! a (quote m) (fn () "a"))
  (setHandler! b (quote m) (fn () "b"))
  [a become: b]
  [a m])' 2>&1 | tail -1

echo "=== \$here ==="
cargo run --quiet -p moof -- '[$here parent]' 2>&1 | tail -1

echo "=== reflection ==="
cargo run --quiet -p moof -- '(do (def f (fn () [self proto])) [[f bytecodes] inspect])' 2>&1 | tail -1
```

Each should produce the expected output without error.

- [ ] **Step 3: verify new opcodes are firing**

```bash
echo "=== LoadHere ==="
cargo run --quiet -p moof -- '(do (def f (fn () $here)) [[f bytecodes] inspect])' 2>&1 | tail -1
# should contain "LoadHere"

echo "=== SendSelf ==="
cargo run --quiet -p moof -- '(do (defmethod (Object zo) [self proto]) [[[Object handlerAt: (quote zo)] body] bytecodes])' 2>&1 | tail -1
# should contain "SendSelf" or "TailSendSelf"

echo "=== SendHere ==="
cargo run --quiet -p moof -- '(do (def g (fn () [$here parent])) [[g bytecodes] inspect])' 2>&1 | tail -1
# should contain "SendHere"
```

- [ ] **Step 4: benchmark — bootstrap time**

```bash
time cargo run --quiet --release -p moof -- '(do (def x 42) x)' 2>&1 | tail -3
```

Compare to pre-V4-α baseline (1.3s release per the polyglot-plan profile). If V4-α reduces dispatch overhead 5-10%, expect 1.2s-ish. Larger drops will come from byte-encoding (V4-β) and zig dispatch (V4-γ).

- [ ] **Step 5: push**

```bash
git push 2>&1 | tail -3
```

- [ ] **Step 6: write up the wave**

Update memory or the docs to note V4-α landed. Plan V4-β (byte encoding) as the next plan to write.

---

## Out of scope for V4-α

deferred to V4-β:
- byte-tagged encoding (replace `Vec<Op>` with `Vec<u8>`)
- content-addressed chunk hashes
- deterministic-encoding tests

deferred to V4-γ:
- zig substrate core (the host migration)
- tail-call-threaded dispatch loop

deferred to V4-δ:
- OCaml seed reader+compiler
- removal of rust seed reader+compiler

deferred to phase D:
- `Suspend`/`Resume` full semantics (placeholders raise 'unimplemented for now)
- promise scheduler
- continuation operatives

deferred to phase G+:
- JIT
- type-specialized arithmetic (Smalltalk primitives + fallback)
- polymorphic IC (4-entry hash on miss)
- mid-bytecode rewriting

---

## see also

- spec: `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` — the canonical opcode set + byte encoding design.
- V3 plan: `docs/superpowers/plans/2026-05-09-vat-V3-here-form.md` — the predecessor (V3 here-form unification).
- substrate-laws.md — L3, L5, L10, L11 (the invariants this VM upholds).
- the 2026-05-10 polyglot conversation: rust → zig migration plan + OCaml seed compiler plan (these are V4-γ and V4-δ).
