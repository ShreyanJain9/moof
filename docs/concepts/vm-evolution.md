# VM Evolution, Elegance, and Hot-Swapping: Deep Implementation Plan

> **Goal: A 6-instruction, Form-serialized, hot-swappable MCO engine.**

This document provides the exact C-ABI, the memory layouts, and the checkpoint protocol to decouple the VM.

## 1. The Decoupled VM ABI and Form Layouts

The substrate (`src/vm.rs`) must be deleted. It is replaced by an MCO loaded at boot.

### Phase 1.A: The C-ABI Definition
**Files:** `crates/abi/src/engine.rs`
```rust
#[repr(u32)]
pub enum VmStatus {
    Yielded = 0,    // Turn complete or async wait
    Trapped = 1,    // Error state
    Finished = 2,   // Root execution complete
}

#[repr(C)]
pub struct VmState {
    pub instruction_pointer: usize,
    pub call_frame_id: u32, // FormId of current frame
}

extern "C" {
    fn moof_engine_step(state: *mut VmState, max_ops: u64) -> VmStatus;
}
```

### Phase 1.B: The `CallFrame` Form Memory Layout
The MCO engine accesses Moof memory. The `CallFrame` must be a rigorous Form.
```rust
// Layout conceptually enforced by `lib/early/vm-structs.moof`
// Slots:
// 0: Method FormId
// 1: Receiver FormId
// 2: Args List FormId
// 3: Locals Table FormId
// 4: Caller CallFrame FormId
// 5: IP Integer FormId
// 6: Stack List FormId (or dedicated Stack Form)
```
The VM MCO uses capability imports (e.g., `moof_slot_read(frame_id, 3)`) to access locals.

## 2. Shrinking the Opcode Set

A smaller set pushes macro expansion and lexical scoping firmly into `compiler.moof`.

### Phase 2.A: The 6-Opcode Definition
**Files:** `crates/mco-engine-v1/src/opcodes.rs`
The byte stream in a `Chunk` is strictly these 8-byte aligned instructions.
| Opcode (1 byte) | Arg 1 (1 byte) | Arg 2 (2 bytes) | Arg 3 (4 bytes) | Description |
|-----------------|----------------|-----------------|-----------------|-------------|
| `0x01` (LDC)    | -              | -               | `ConstIdx`      | Push `Chunk.consts[Idx]` |
| `0x02` (LDV)    | `EnvDepth`     | `SlotIdx`       | -               | Push local variable |
| `0x03` (STV)    | `EnvDepth`     | `SlotIdx`       | -               | Pop and store to local |
| `0x04` (DISP)   | `Arity`        | `IC_Idx`        | `SelectorIdx`   | Pop `Arity` args + receiver. Dispatch. |
| `0x05` (RET)    | -              | -               | -               | Pop frame, push return val |
| `0x06` (JMPF)   | -              | -               | `Offset`        | Pop bool, jump if false |

### Phase 2.B: Inline Caching (IC) Forms
1. The compiler allocates an `IC` Table Form for every `DISP` instruction. It is stored in the `Chunk.ics` slot.
2. The VM MCO executes `DISP`:
   ```rust
   let receiver_proto = moof_get_proto(receiver);
   let ic_table = moof_slot_read(chunk, Chunk::ICS);
   let ic_entry = moof_table_get(ic_table, ic_idx);

   if ic_entry.proto == receiver_proto {
       // Fast path
       setup_new_frame(ic_entry.method);
   } else {
       // Slow path: moof_lookup_handler walks proto chain
       let method = moof_lookup_handler(receiver_proto, selector);
       moof_table_put(ic_table, ic_idx, {proto: receiver_proto, method});
       setup_new_frame(method);
   }
   ```

## 3. The Hot-Swap Lifecycle

Swapping the VM engine mid-execution.

### Phase 3.A: The Checkpoint-and-Resume Protocol
**Files:** `lib/stdlib/vm-hotswap.moof`
1. **The Request:** An agent invokes `[$engine swapWith: "core/vm-v2.mco"]`.
2. **The Yield:** The method sets a flag `yield_requested = true` in the substrate. The MCO engine checks this flag loop, saves the `ip` to `CallFrame.slots[5]`, and returns `VmStatus::Yielded`.
3. **The Substrate Intervention:**
   ```rust
   // src/scheduler.rs
   let current_state = engine_state;
   let new_engine = mco_loader.load("core/vm-v2.mco");
   // Pass the exact same FormId for the call frame
   let status = new_engine.moof_engine_step(&mut current_state, u64::MAX);
   ```
4. **The Resume:** The new engine reads the `CallFrame`, reads `CallFrame.slots[5]` (the IP), jumps to that offset in the chunk, and continues exactly where it left off.

**Tests:** `test_vm_state_serializes_to_forms`, `test_vm_hotswap_maintains_stack_integrity`.
