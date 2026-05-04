# VM Evolution, Elegance, and Hot-Swapping: Implementation Plan

> **Building the greatest VM ever: Decoupled, Meta-Circular, and Swappable.**

This document outlines the architectural transition from the current hardcoded Rust bytecode interpreter (`src/vm.rs`) to a decoupled, polyglot, hot-swappable MCO engine with a highly reduced opcode set.

## 1. The Decoupled VM MCO ABI

The VM must no longer be statically compiled into the `moof` binary. The substrate seed merely provides memory management and capability routing.

### Phase 1.A: The `moof-engine` ABI
Define a strict C ABI for VM MCOs.
```rust
// in crates/abi/src/engine.rs
#[repr(C)]
pub struct VmState {
    pub instruction_pointer: usize,
    pub frame_pointer: FormId, // Pointer to the current CallFrame Form
    pub stack_pointer: usize,
    pub status: VmStatus,
}

extern "C" {
    // The core entrypoint the substrate calls to advance execution
    fn moof_engine_step(state: *mut VmState, max_steps: u64) -> VmStatus;
}
```

### Phase 1.B: The `$engine` Capability
The supervisor loads the VM from an MCO:
`(def Engine [$mco load: "core/vm-v1.mco"])`
The substrate simply loops `moof_engine_step` until it yields or completes.

## 2. Meta-Circular State Serialization (Forms all the way down)

To achieve hot-swapping, the VM cannot hold opaque native state (like Rust `Vec` stacks). All execution state must be Forms.

### Phase 2.A: The CallFrame Proto
```moof
(defproto CallFrame
  (slots
    method      ;; Method Form being executed
    receiver    ;; Self
    args        ;; List Form of arguments
    locals      ;; Table Form of local variables
    caller      ;; Previous CallFrame Form
    ip          ;; Integer: current instruction index
    stack))     ;; List Form: operand stack
```

When the VM traps, or yields at a message turn boundary, the `frame_pointer` in the ABI points directly to a valid, garbage-collected `CallFrame` Form in the heap.

## 3. Hot-Swapping the Execution Engine

Because state is entirely represented as Forms, upgrading the VM is trivial and safe.

### Phase 3.A: The Checkpoint-and-Resume Swap
1. **Pause:** At a turn boundary, the current engine returns `VmStatus::Yielded`.
2. **State Capture:** The substrate holds the root `CallFrame` Form.
3. **Load New Engine:** The system evaluates `(def NewEngine [$mco load: "core/vm-v2-jit.mco"])`.
4. **Resume:** The substrate calls `moof_engine_step` on the *new* engine, passing it the *existing* `CallFrame` Form pointer. The new engine reads the Form, understands the `ip` and `stack`, and resumes execution flawlessly.

## 4. Shrinking the Opcode Set

The current opcode set is large. We will shrink it to a minimalist set, pushing complexity to the Moof compiler.

### Phase 4.A: The Reduced Instruction Set
The MCO engine only needs to understand these core primitives:
1. `LDC <const_idx>`: Load constant onto stack.
2. `LDV <local_idx>`: Load local variable onto stack.
3. `STV <local_idx>`: Store top of stack to local variable.
4. `DISPATCH <selector_idx> <arity>`: The universal call mechanism. Pops `arity` args and a receiver. Looks up `selector` in receiver's `proto` chain. Pushes new CallFrame.
5. `RET`: Pop current frame, return top of stack to caller.
6. `JMP / JMPF <offset>`: Control flow.

### Phase 4.B: Inline Caching as Forms
The `DISPATCH` opcode is slow if it traverses the proto chain every time.
1. The Moof compiler attaches an `IC` (Inline Cache) Table Form to every `DISPATCH` call site.
2. The VM MCO checks the `IC` Table: `(receiver_proto_id -> method_form)`.
3. If hit, it jumps directly to `method_form`.
4. If miss, it walks the chain, updates the `IC` Table Form, and proceeds.
Because the IC is a Form, `[proto setHandler:...]` can easily traverse and clear relevant ICs without needing native Rust VM hooks.
