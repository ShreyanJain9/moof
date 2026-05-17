# substrate v2 — architecture design

> **status:** drafted 2026-05-17. ready for review + ratification.
> **the load-bearing rewrite spec.** defines a from-scratch substrate
> built spec-driven with: vats-first, eat-from-beginning (moof code
> for everything composable), JIT from line 1, Self-style shape
> tables, NaN-boxed FormIds, methods-not-free-fns, perf baked from
> the start, short cold boot. **throws out the current rust + zig
> players.** depends on (and supersedes design choices in)
> `2026-05-16-vats-substrate-and-image-design.md` for vat / image /
> federation semantics.
>
> **prior reading:**
> - `2026-05-16-vats-substrate-and-image-design.md` — the umbrella design (vats, merkle store, scheduler, etc.)
> - `2026-05-16-phase3-cohesive-vision-design.md` §6 — the perf path
> - `2026-05-16-phase2-moof-performance-design.md` — tier-1/2/3 perf playbook
> - `2026-05-10-vm-V4-opcodes-design.md` — V4 ISA + image format §10
> - `2026-05-03-track-1-mcos-and-datasource-design.md` — mco ABI + LB-1
> - `docs/concepts/forms.md` — the four faces
> - `docs/laws/substrate-laws.md` — L1-L16

---

## table of contents

§0  the picture in one shot
§1  what the substrate is — and isn't
§2  target performance + compactness budget
§3  the Form / Shape / FormId layout
§4  the bytecode dispatch ABI
§5  the JIT — copy-and-patch design
§6  the interpreter (fallback for cold + wasm host)
§7  inline caches — ICs baked into JIT'd code
§8  heap, nursery, generational gc
§9  vats, scheduler, mailbox (MPMC)
§10 shared segment + intern table
§11 image i/o — V4 byte layer + merkle object store
§12 mco runtime + mandatory serialize/restore
§13 the native primitives registry
§14 the minimal ocaml-seed v2
§15 cold boot path — what loads in what order
§16 swap-in plan — when do we delete current players
§17 open questions + risks
§18 see also

---

## §0 — the picture in one shot

substrate v2 is the smallest possible thing that runs moof bytecode
fast. it provides: V4 bytecode dispatch (interpreted + JIT'd), the
heap data structure, the wasm mco runtime, lock-free mailbox /
intern primitives, and the ~40 native methods that all moof code
ultimately bottoms out into. **everything else is moof code in the
image, or mcos.**

the substrate is **single-language (zig 0.16), ~6500 LoC, ~5 MB
binary, ~50ms cold boot.** every line earns its place via a perf
gate; nothing is "we'll optimize later." the JIT is in the
architecture from line 1 because moof-in-moof requires it to be
fast.

four mantras hold the design:

1. **methods, not free fns.** every primitive is a method on its
   proto, dispatched through the same `OP_SEND` path as any moof
   method send. there are no "internal" calling conventions
   parallel to moof's.
2. **eat from beginning.** anything that composes from primitives
   lives in moof or an mco, not in zig. the substrate has ~40
   natives, fixed; the temptation to add "just one more" has
   nowhere to land.
3. **shape-tabled forms.** Self-style hidden classes; O(1) slot
   access via shape → offset → direct array index; ICs cache
   shape transitions; ~64-byte typical form.
4. **JIT from line 1.** copy-and-patch JIT; per-arch stencil
   libraries; ~100M sends/sec hot path; persistent cache for
   short cold boot.

---

## §1 — what the substrate is — and isn't

### 1.1 what it is

```
moof binary  =  bytecode interpreter
            +   JIT compiler (copy-and-patch)
            +   heap + shape tables + nursery + gc
            +   mailbox MPMC primitive
            +   intern table (sym + shape + shared-segment)
            +   wasm mco runtime
            +   image i/o (V4 byte format + merkle store)
            +   the ~40 native primitives that moof bottoms into
            +   ~150 LoC of cli / bootstrap glue
```

that's the whole substrate. ~6500 LoC zig. one binary, ~5 MB
released, ~50ms cold boot.

### 1.2 what it isn't

the substrate does **not** contain:

- the parser (moof code, in the image)
- the compiler (moof code, in the image)
- the transporter (moof code + a few primitives)
- the supervisor / vat ergonomics layer (moof code)
- the scheduler **logic** — substrate spawns N threads, each calls
  a moof primitive `[$scheduler nextRunnable]` to pick the next
  vat. the policy is moof.
- the gc walker — substrate provides `[Heap iterate: blk]`; moof
  walks and marks. (substrate sweeps unmarked.)
- the merkle store **organization** — substrate writes bytes by
  hash; moof orchestrates refs, journal, packfiles.
- the freeze policy walker (`freezeRecursive`) — moof code.
- the stdlib — entirely moof code.
- the mco protos installed at runtime — moof loads them.
- specific mcos like hash / utf8 / base64 / clock — wasm modules
  loaded at runtime.

**the substrate is a player. it does not own truth. it loads truth.**

### 1.3 the eat-from-beginning principle

every previous "we'll move this to moof later" plan has decayed
into "we still have it in zig because the user can't notice it's
fast." this time the principle is enforced **structurally**:

- the substrate exposes **only the ~40 natives in §13**. there is
  nowhere to put more. adding a native requires a spec amendment.
- moof methods that today are natives (`Cons:length`,
  `Integer:abs`, `String:trim`, etc.) **do not exist** as natives
  in v2. they live only in moof code (`lib/stdlib/*.moof`).
- if a moof method is too slow, the answer is JIT, not "move it
  to zig." the JIT compiles `Cons:length` to native code that is
  as fast as today's native impl — at the cost of needing the
  JIT to be real, which it is, from line 1.

### 1.4 the maru posture, dead serious

maru's c kernel is ~250 LoC; everything else is maru. we're
targeting the same shape with a slightly larger substrate
(~6500 LoC) because V4 bytecode + shape tables + the JIT +
wasm host + the V4 image format add real machinery. but the
**discipline is identical**: the substrate is a CACHE of the
language, not the language itself.

---

## §2 — target performance + compactness budget

### 2.1 binary + memory targets

| metric | target | how |
|---|---|---|
| substrate LoC (zig) | **~6500** | discipline + per-module budget (§2.3) |
| stripped release binary | **3-5 MB** | zig std + wasmtime; no LLVM/cranelift |
| seed.vat (minimal ocaml-seed v2 output) | **~50 KB** | smaller native-list; tighter chunks |
| typical workspace .moof/store/ | **50-200 MB** | merkle dedup; reflog retention |
| cold boot (settled image) | **~50-100 ms** | image-as-canon + lazy materialization (§15) |
| cold boot (fresh from .vat) | **<1 s** | unpack + boot |

### 2.2 runtime perf targets

| metric | target | how |
|---|---|---|
| empty turn (yield, no message) | **<1 μs** | tight scheduler loop |
| typical turn (1-3 mutations) | **10-100 μs** | nursery + diff + hash recompute |
| turn rate per scheduler (interpreted) | **~5M turns/sec** | shape ICs + threaded dispatch |
| turn rate per scheduler (JIT'd hot) | **~100M ops/sec** | copy-and-patch |
| hot opcode dispatch (interpreted) | **<2 ns/op** | tail-call threaded |
| hot opcode dispatch (JIT'd) | **<1 ns/op** | one indirect jump per stencil |
| MPMC mailbox enqueue | **<100 ns** | michael-scott + CAS |
| intern lookup (sym, shape, shared-seg) | **<100 ns** | atomic acquire-load |
| intern install (CAS) | **<1 μs** | hash + slot probe + atomic exchange |
| shape transition (cold) | **<200 ns** | lookup + alloc + register |
| shape transition (cached) | **<10 ns** | direct hash hit |
| Form alloc (nursery, fast path) | **<100 ns** | bump-pointer |
| form hash recompute (typical) | **~10 ns** | blake3 on ~64 bytes |

### 2.3 per-module LoC budget

```
module                            loc   purpose
──────────────────────────────────────────────────────────────────
main.zig                           50   cli dispatch
bootstrap.zig                     100   image load + entry
value.zig                         100   NaN-boxed Value (= FormId)
form.zig                          400   shape-based Form layout
shape.zig                         300   ShapeId, interner, transitions
sym.zig                           150   symbol intern table
heap.zig                          300   per-vat arena; allocation
vm.zig                            600   interpreter (threaded dispatch fallback)
mailbox.zig                       250   MPMC + epoch reclamation
mco.zig                           600   wasm host + serialize/restore plumbing
image.zig                         500   V4 byte layer + merkle objects + refs
posix.zig                         150   syscall wrappers (capped)
prims/                            600   ~40 native methods, by domain
jit/
  stencils/arm64.zig              600   pre-compiled stencil library
  stencils/x86_64.zig             600   pre-compiled stencil library
  compiler.zig                    800   bytecode → stencils → patches → RWX
  loader.zig                      300   mprotect, code cache mgmt
  ic.zig                          200   inline-cache slot layout in JIT'd code
──────────────────────────────────────────────────────────────────
total                            6500
```

each module has a hard LoC budget. if the implementation exceeds
budget by >20%, design is revisited. **the budget is the discipline.**

### 2.4 perf is a conformance contract

per spec §13.10 of the umbrella design, every phase ships
microbenchmarks for its targets. perf regressions block merge.
**the substrate v2 conformance suite includes perf oracles**, not
just behavioral oracles. tolerance bands per metric. drift triggers
investigation.

---

## §3 — the Form / Shape / FormId layout

### 3.1 FormId: 4-bit scope + 28-bit payload

```zig
pub const FormId = packed struct(u32) {
    payload: u28,
    scope: u4,
};
```

**16 scopes** (only 0x0-0x9 in use; 0xA-0xF reserved):

| scope | hex | semantics | payload |
|---|---|---|---|
| `vat_local` | 0x0 | index into the current vat's `heap.forms` | 28-bit index (~268M forms) |
| `shared_segment` | 0x1 | index into process-wide shared frozen arena | 28-bit index |
| `far_ref` | 0x2 | index into vat's far-ref table | 28-bit index |
| `imm_int` | 0x3 | payload IS a small int | i28 (-2^27..2^27-1, ~±134M) |
| `imm_char` | 0x4 | payload IS a codepoint | 28-bit u-codepoint |
| `imm_bool` | 0x5 | payload = 1 for #true, 0 for #false | 1 bit |
| `imm_nil` | 0x6 | payload always 0 | singleton |
| `imm_sym` | 0x7 | payload IS a sym-id | 28-bit u-id (~268M syms) |
| `string_pool` | 0x8 | index into shared string pool | 28-bit index |
| `bigint_pool` | 0x9 | index into shared bigint pool | 28-bit index |
| `float_pool` | 0xA | index into shared float pool (for f64) | 28-bit index |
| `shape_id` | 0xB | the form IS a Shape (reflection) | 28-bit ShapeId |
| `reserved` | 0xC-0xF | future use | — |

**every Value in the VM is exactly one u32.** this is the single
biggest perf simplification: operand stack, frame slots, IC slots,
slot table values are all `u32`. no tagged union unwrap. no
indirect dispatch on Value's tag.

floats live in a shared pool keyed by content. accessing a float
is one indirect load (`float_pool[id.payload]`). uncommon enough
in moof workloads that this is the right trade.

bigints similarly. small-int overflow auto-promotes: at int+int
fast path, if result overflows i28, allocate a bigint, return a
FormId with scope=bigint_pool.

`Value = FormId` is a literal type alias. no other type.

### 3.2 Form: shape-tabled, packed

```zig
pub const Form = struct {
    shape: ShapeId,                          // 4 bytes
    inline_slots: [INLINE_SLOT_COUNT]Value,  // 16 bytes (4 × u32)
    overflow: ?[*]Value,                     // 8 bytes (null if ≤4 slots)
    handlers: ?*HandlerTable,                // 8 bytes (null if no native+moof handlers)
    meta: ?*MetaTable,                       // 8 bytes (null if no meta)
    hash_cache: [8]u8,                       // 8 bytes (truncated blake3)
    flags: Flags,                            // 1 byte
    _pad: [3]u8,                             // 3 bytes alignment
};
// total: 64 bytes typical
```

```zig
pub const INLINE_SLOT_COUNT: usize = 4;
```

**flags byte:**
```
bit 0  frozen                  (mutation guard)
bit 1  hash_dirty               (recompute hash at next save)
bit 2  remembered               (mature → young pointer; gc remembered-set)
bit 3  is_live_face             (cannot be frozen; cap / vat / mailbox)
bit 4  is_far_ref_entry         (the form IS a far-ref entry; routed accordingly)
bit 5  reserved
bit 6  reserved
bit 7  reserved
```

**target sizes:**
- empty form (no shape, no overflow, no handlers, no meta): allocator may compact to ~32 bytes via per-size-class slabs. (§8.2)
- typical form with 1-4 slots: 64 bytes
- form with >4 slots: 64 + (overflow slot count × 4) bytes

### 3.3 Shape: hidden class

```zig
pub const ShapeId = u32;

pub const Shape = struct {
    proto: FormId,                              // proto chain root
    parent: ?ShapeId,                           // shape this transitioned from
    added_sym: ?u32,                            // the sym added in this transition
    slot_count: u8,                             // total slots in this shape
    slot_syms: [INLINE_SHAPE_SIZE]u32,          // sym-id → offset mapping (inline)
    slot_overflow: ?[]u32,                      // additional slot syms (heap'd)
    transitions: AutoHashMapUnmanaged(u32, ShapeId),  // sym-id → next ShapeId
    flags: ShapeFlags,
};

pub const INLINE_SHAPE_SIZE: usize = 4;
```

**shape lookup is content-addressed.** the shape interner is a hash
table keyed by `(proto, slot_syms[])` (canonical sorted form). same
slot-set + same proto = same ShapeId, process-wide.

**slot offset lookup**: given a `(Shape, slot_sym)`, find the index
in `slot_syms`. that index is the offset into the Form's
inline_slots (if offset < INLINE_SLOT_COUNT) or overflow (offset >=
INLINE_SLOT_COUNT). **two array reads, no hashing.**

**transition**: when moof code sets a slot that isn't in the
current shape, look up `current_shape.transitions[new_sym]`:
- hit (cached): use the existing target ShapeId; update form's
  shape pointer; write to the new slot.
- miss: synthesize a new Shape (extend the current one's
  `slot_syms` with `new_sym`); content-address it via the
  shape interner; cache the transition.

### 3.4 the shape interner

```zig
pub const ShapeInterner = struct {
    // content-addressed hash table
    // key: (proto, slot_syms_sorted)
    // value: ShapeId
    by_signature: AutoHashMap(ShapeSignature, ShapeId),
    shapes: ArrayListUnmanaged(Shape),
    // for gc — see §8.5
    refcount: ArrayListUnmanaged(u32),
};

pub const ShapeSignature = struct {
    proto: FormId,
    slot_syms_hash: u64,  // blake3 of sorted syms
};
```

shape interner is process-wide (not per-vat). **shapes are shared
across vats** without copying.

### 3.5 dispatch on shape

method dispatch goes:
1. operand stack has the receiver (a Value = FormId)
2. for non-immediate scopes, look up `form.shape`
3. for immediate scopes (int, char, bool, nil, sym), use a hardcoded
   shape based on scope (e.g., `imm_int` has shape `int_shape`)
4. look up `[shape, selector]` in the IC slot at the call site
5. if IC hit: direct call to the cached handler-fn-pointer
6. if IC miss: full proto-chain walk via `shape.proto`; update IC

**most sends hit the IC.** §7 covers IC structure.

### 3.6 garbage collection of shapes

shapes outlive their last form instance (because they're in
transitions tables of other shapes). a shape becomes unreachable
when:
- its parent shape is unreachable
- it has no live instances
- no live transitions point to it

shape gc runs alongside heap gc (§8.5). low-frequency event.

### 3.7 mutation guard via Form.flags

`Object:slotAt:put:` checks `form.flags.frozen` before write. raises
`'frozen-form` immediately if set. **single byte read; one branch.**

mutation also sets `form.flags.hash_dirty` so the next save
recomputes the hash.

### 3.8 design choices, locked

- `INLINE_SLOT_COUNT = 4` (covers ~80% of forms; tunable later)
- `INLINE_SHAPE_SIZE = 4` (same rationale)
- shape interner is process-wide
- shape gc deferred to heap gc cycles
- transition cache is part of Shape, not separate
- floats live in float_pool (Value stays u32)

---

## §4 — the bytecode dispatch ABI

### 4.1 V4 opcodes (24-op ISA, unchanged)

substrate v2 keeps the V4 opcode set per
`docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`. no
new opcodes invented here. listed for reference:

```
NOP, LOAD_CONST, LOAD_LOCAL, STORE_LOCAL, LOAD_GLOBAL, STORE_GLOBAL,
LOAD_SLOT, STORE_SLOT, PUSH_NIL, PUSH_TRUE, PUSH_FALSE, JUMP,
JUMP_IF_FALSE, RETURN, MAKE_CLOSURE, CALL_NATIVE,
SEND, SEND_DYNAMIC, SEND_SUPER, SEND_HERE, SEND_SELF,
TAIL_SEND, EVAL_FORM, BECOME
```

24 opcodes, reserved tags up to 0x1F. add new opcodes only via
major image-format version bump.

### 4.2 dispatch handler signature (interpreter)

```zig
pub const DispatchHandler = *const fn (
    ctx: *VmContext,
    pc: [*]const u8,
    sp: [*]Value,
    fp: [*]Value,
) callconv(.@"inline") void;
```

threaded dispatch via `@call(.always_tail)`:

```zig
pub fn opLoadLocal(ctx: *VmContext, pc: [*]const u8, sp: [*]Value, fp: [*]Value) callconv(.@"inline") void {
    const offset = readU8(pc);
    const v = fp[offset];
    sp[0] = v;
    return @call(.always_tail, dispatch[pc[1]], .{ ctx, pc + 2, sp + 1, fp });
}
```

every handler ends in a tail call to the next opcode's handler.
**no return overhead. no switch. one register spill per op.**

### 4.3 dispatch table

```zig
pub const dispatch: [256]DispatchHandler = blk: {
    var tbl: [256]DispatchHandler = undefined;
    @setEvalBranchQuota(10000);
    inline for (0..256) |op| {
        tbl[op] = comptime opHandler(op);
    }
    break :blk tbl;
};
```

generated at compile-time. comptime metaprogramming covers the
24 real opcodes + 232 traps (each a "unknown opcode" handler).
**zero runtime overhead for the table itself.**

### 4.4 the SEND ABI

`OP_SEND` semantics:
```
SEND <ic_slot:u16> <selector_sym:u32> <argc:u8>
```

at dispatch:
1. read IC slot (16-byte struct: shape, handler_ptr, hit_count, padding)
2. if IC.shape == receiver.shape (mono case): call IC.handler_ptr directly
3. else: full proto-chain walk; update IC; call handler
4. handler signature: `fn(ctx, receiver, args[]) Value`
5. handler returns; result on operand stack

**IC hit hot path: 1 load + 1 compare + 1 indirect call. ~3-5 ns.**

### 4.5 frames

```zig
pub const Frame = struct {
    chunk: ChunkId,       // which chunk we're in
    pc: [*]const u8,      // bytecode pointer
    fp: [*]Value,         // frame pointer (into operand stack)
    callee: FormId,       // the form we're executing (closure / method)
    saved_sp: [*]Value,   // return target on stack
};
```

frame stack is `ArrayListUnmanaged(Frame)`. push on `SEND`, pop on
`RETURN`. tail `TAIL_SEND` replaces top frame in place.

**target: <1 KB frame stack overhead for typical depth (~32
frames).**

### 4.6 chunk representation

```zig
pub const Chunk = struct {
    bytecode: []const u8,        // V4 byte-tagged ops
    consts: []const Value,        // constant pool
    ics: []ICSlot,                // per-call-site inline caches
    params: []const u32,          // param sym-ids
    arity: u8,
    has_rest: bool,
    // JIT integration:
    jit_entry: ?*const fn() void, // null if not yet JIT'd
    jit_cache_key: [16]u8,        // content-hash for cache lookup
};
```

stored in `world.chunks: AutoArrayHashMap(ChunkId, Chunk)`. lookup
by ChunkId (which is itself a FormId with scope reserved for chunks
— actually just an ordinary vat_local FormId pointing to a chunk
Form whose proto is Chunk).

---

## §5 — the JIT — copy-and-patch design

### 5.1 why copy-and-patch

copy-and-patch JIT (haberman 2021, "compiling python to x86 with
copy and patch") is the right tradeoff for moof:

- ~10-50× speedup over interpreted (matches phase 3 §6.2 target)
- much simpler than full register-allocating JIT (cranelift / LLVM)
- per-arch stencil libraries, no architecture-specific compiler
- ~2-3K LoC total (versus 10-30 MB for cranelift)
- works for moof's small ISA (24 opcodes × ~4-8 shapes each = ~150 stencils per arch)

### 5.2 stencils

a stencil is a small native function with patch points. each
(opcode, operand-shape) pair gets one stencil. example:

```assembly
; arm64 stencil for: LOAD_LOCAL offset
; assumes: x28 = operand stack pointer, x29 = frame pointer,
;          x19 = vm context pointer

stencil_load_local:
    ldr w0, [x29, #__OFFSET_PATCH__]    ; patch: 12-bit offset
    str w0, [x28], #4                    ; push to operand stack
    b __NEXT_PATCH__                     ; patch: 28-bit branch target
```

**patch points**:
- `__OFFSET_PATCH__` — runtime-resolved operand (here, the local slot offset)
- `__NEXT_PATCH__` — address of the next stencil's first instruction
- `__IC_PATCH__` — for SEND, the IC slot address

stencil generation:
1. write each opcode-shape as a zig function with `@hot` attribute
2. compile with `-O3 -fno-stack-check`
3. extract the function's machine bytes via objcopy / per-arch tool
4. record patch points by symbol name → offset into the bytes
5. embed both bytes + patch table as constant data in the substrate binary

embedded format:
```zig
pub const Stencil = struct {
    bytes: []const u8,
    patches: []const PatchPoint,
};

pub const PatchPoint = struct {
    offset: u16,
    width: u8,        // 8, 12, 16, 28, 32, or 64 bits
    kind: PatchKind,  // .imm_offset, .next_handler, .ic_slot, .constant_ref
};

pub const STENCILS_ARM64: [STENCIL_COUNT]Stencil = blk: {
    @embedFile("stencils/arm64.bin")
    // + decoded patch table
};
```

### 5.3 JIT compilation algorithm

```
fn jit_compile_chunk(world: *World, chunk_id: ChunkId, arch: Arch) !void {
    const chunk = world.chunks.get(chunk_id);
    
    // check the cache first
    if (jit_cache_lookup(chunk.jit_cache_key, arch)) |cached_bytes| {
        chunk.jit_entry = install_jit_cached(cached_bytes);
        return;
    }
    
    // allocate enough RWX memory for the JIT'd chunk
    // (estimate: avg stencil ~64 bytes; chunk has N ops; budget 200 bytes/op)
    const estimate = chunk.bytecode.len * 200;
    const code_buf = try jit_alloc_rwx(estimate);
    
    var pos: usize = 0;
    var op_starts = std.AutoHashMap(usize, usize).init(...);  // bytecode pos → jit pos
    
    // pass 1: emit stencils + collect branch fixups
    var pc: usize = 0;
    while (pc < chunk.bytecode.len) {
        const op = chunk.bytecode[pc];
        op_starts.put(pc, pos);
        const stencil = STENCILS[arch][op];
        
        // copy stencil bytes
        @memcpy(code_buf[pos..pos + stencil.bytes.len], stencil.bytes);
        
        // patch
        for (stencil.patches) |patch| {
            switch (patch.kind) {
                .imm_offset => patch_imm(code_buf, pos + patch.offset, patch.width, read_operand(chunk.bytecode, pc + 1)),
                .next_handler => patch_branch_to(code_buf, pos + patch.offset, pos + stencil.bytes.len),
                .ic_slot => patch_imm(code_buf, pos + patch.offset, patch.width, &chunk.ics[ic_index]),
                .constant_ref => patch_const_ptr(code_buf, pos + patch.offset, &chunk.consts[const_index]),
            }
        }
        
        pos += stencil.bytes.len;
        pc += opcode_length(op);
    }
    
    // pass 2: fix up branches (JUMP, JUMP_IF_FALSE) — they need
    // forward references resolved
    fix_up_branches(code_buf, &op_starts, &chunk);
    
    // flush instruction cache (arm64 requires it; x86 doesn't)
    arch_specific_icache_flush(code_buf);
    
    // remap RWX → RX (drop write permissions)
    try jit_finalize_rx(code_buf);
    
    chunk.jit_entry = @ptrCast(code_buf.ptr);
    
    // persist to cache for next boot
    try jit_cache_write(chunk.jit_cache_key, code_buf, arch);
}
```

### 5.4 JIT cache (for short cold boot)

JIT'd code is **content-addressed by chunk-hash**. cached on disk:

```
.moof/store/
  jit-cache/
    arm64/
      <chunk-hash>.bin     ← raw native code bytes
      <chunk-hash>.meta    ← patch positions, branch targets, ic refs
    x86_64/
      ...
```

at chunk-load:
1. compute chunk content-hash
2. check `jit-cache/<arch>/<hash>.bin`
3. if present, mmap + remap-RX + install as jit_entry
4. if absent, JIT compile + cache for next time

**cache key includes substrate version + arch** (stencil layout
changes between versions). version mismatch invalidates the cache.

### 5.5 when do chunks get JIT'd

three policies, configurable per workload:

- **lazy (default)**: chunk is JIT'd on first dispatch. interpreter
  serves first call; JIT serves subsequent. ~100μs per chunk.
- **eager**: at image-load, JIT every chunk in the image. trades
  ~10ms cold boot for zero warmup.
- **AOT** (server mode): build script JIT's everything and ships
  jit-cache populated. cold boot loads cached bytes directly.

### 5.6 deoptimization

JIT'd code can hit guard failures:
- IC miss (shape doesn't match cached shape) → bail to slow IC walk
- frozen guard failure → raise `'frozen-form` (caught in interp)
- bigint overflow → bail to bigint path (interp)
- become-target redirect → bail; re-jit eventually

**deopt strategy**: bail to interpreter for that single send;
return to JIT'd code at the SEND's bytecode position post-handler.
**no on-stack replacement**; the interpreter and JIT share the
same operand stack + frame stack layout, so transition is trivial.

if a JIT'd chunk experiences >50% deopt rate, mark it invalid and
re-JIT (probably with different ICs).

### 5.7 inline ICs in JIT'd code

ICs are stored **outside** the JIT'd bytes (in `chunk.ics: []ICSlot`).
the JIT'd code holds a pointer to the IC slot; reads + writes go
through that pointer.

```assembly
; arm64 stencil for: SEND with 1 arg
; assumes operand stack has [..., receiver, arg]

stencil_send_1:
    ldr x0, [x28, #-8]                       ; receiver
    ldr x1, [x0, #__SHAPE_OFFSET__]          ; receiver.shape
    ldr x2, =__IC_SLOT_PATCH__               ; IC slot address (patch)
    ldr w3, [x2, #0]                         ; IC.shape
    cmp w1, w3
    b.ne __SLOW_PATH_PATCH__                 ; cache miss
    ldr x4, [x2, #8]                         ; IC.handler_ptr
    blr x4                                    ; tail-call handler
    str x0, [x28], #4                        ; push result
    b __NEXT_PATCH__
```

IC hit: ~5 cycles (load shape, compare, branch, load handler,
call). **~3-5 ns on modern arm64.**

### 5.8 wasm fallback

wasm host cannot patch code at runtime (mostly). for the wasm
player:
- stencils for wasm32 are pre-compiled wasm sequences
- at chunk-load, **concatenate stencils into a wasm module** (no
  patching; operands as immediate constants in the wasm code)
- use wasmtime's runtime-compile capability (or pre-compile per
  image-bind)
- expected slowdown: ~2× vs native JIT, but still ~5-10× over
  interpreter

(in-browser wasm host: no runtime wasm-from-wasm compilation; use
interpreter only. browser perf is bounded by §9.5 of the umbrella
design.)

### 5.9 stencil generation pipeline

at substrate build-time:
1. zig source file `players/v2/src/jit/stencils_arm64.zig` declares
   each stencil as a `@hot` function with placeholders
2. zig-build runs a stencil-extraction tool (zig-host CLI):
   - compile stencils_arm64.zig to a temp .o
   - run a per-arch parser (using zig stdlib's `std.macho` /
     `std.elf`) to extract symbol offsets
   - identify patches by symbol-prefix convention
     (`__OFFSET_PATCH_<n>__`, `__NEXT_PATCH__`, etc.)
   - emit `stencils_arm64.bin` (raw bytes) + `stencils_arm64.meta`
     (patch table as zig comptime data)
3. these get `@embedFile`'d into the substrate
4. JIT compiler reads them at runtime

per-arch generation:
- macos arm64 (M1+)
- linux x86_64
- linux arm64
- (wasm32 — sequence pre-build; no per-platform variants needed)

### 5.10 design choices, locked

- copy-and-patch over cranelift / LLVM (smaller; portable; faster compile)
- lazy JIT by default (eager + AOT are knobs)
- IC slots external to JIT'd code (allows IC invalidation without re-JIT)
- deopt via bail-to-interpreter (no OSR machinery)
- arch coverage: arm64-darwin, x86_64-linux, arm64-linux at v1
- wasm: interpreter-only for browser; AOT for wasmtime
- jit-cache content-addressed; survives across boots

---

## §6 — the interpreter (fallback)

even with JIT, the interpreter has a job:

1. **wasm host execution** — chunks that the wasm player runs
2. **JIT cold path** — chunks not yet JIT'd
3. **deopt landing zone** — JIT'd code bails here on guard failure
4. **debugging mode** — single-step requires interpreted dispatch
5. **mco interaction** — wasm import calls path through the interpreter

interpreter signature matches JIT'd code (operand stack + frame
stack identical layout). transition between interp and JIT is
zero-cost.

threaded dispatch via `@call(.always_tail)` (§4.2). target: **<2
ns/op** for hot ops; ~5x slower than JIT'd; still ~10x faster than
naive switch dispatch.

---

## §7 — inline caches

### 7.1 IC slot structure

```zig
pub const ICSlot = struct {
    shape: ShapeId,                      // cached receiver shape
    handler: *const HandlerFn,           // resolved handler fn pointer
    hit_count: u32,                      // for poly-promotion + perf telemetry
};
// 16 bytes per IC slot
```

**4-way polymorphic IC** (planned at line 1 even if v1 ships mono):
the per-call-site IC is actually 4 slots:

```zig
pub const PolyIC = [4]ICSlot;
```

on send:
- check slot 0; if hit, dispatch
- if miss, check slots 1-3
- on hit in slot n (n > 0), promote: swap slot 0 ↔ slot n
- on full miss, evict least-recently-hit slot; fill with new shape

target: **~95% IC hit rate** on real moof workloads (per V8 / Self
research, hits this with 4-way).

### 7.2 IC invalidation

ICs are invalidated when:
- a proto's handler table changes (proto editing — moldability per L10)
- a form becomes (BECOME redirect)
- a shape is gc'd (transitive)

invalidation strategy: bump a global `proto_generation` counter on
any proto edit. each IC slot stores the generation at install time.
on dispatch, if `IC.gen != current.gen`, treat as miss.

**alternative considered + rejected**: per-proto generation
counters (more precise but more bookkeeping). global counter wins
for the substrate-side because IC invalidations are infrequent and
the comparison is cheap.

### 7.3 IC slot encoding in bytecode

each `SEND` op carries a 16-bit IC slot index. the chunk's
`ics: []ICSlot` array is the table. JIT'd code patches the address
of the IC slot directly (§5.7).

### 7.4 IC + shape gc

if a shape is reclaimed (§3.6), all ICs referencing it must
invalidate. handled by the global generation bump + lazy check.

---

## §8 — heap, nursery, generational gc

### 8.1 heap layout

per-vat:
```zig
pub const Heap = struct {
    forms: ArrayListUnmanaged(Form),        // primary storage
    free_list: ArrayListUnmanaged(u28),     // reclaimed slots (tombstones)
    redirects: AutoHashMapUnmanaged(FormId, FormId),  // become: targets
    forwarding: AutoHashMapUnmanaged(FormId, FormId), // post-compaction
    remembered_set: AutoHashMapUnmanaged(FormId, void),  // mature → young
};
```

allocation:
1. if `free_list` non-empty, reuse top entry
2. else append to `forms`
3. nursery alloc: a separate small heap with `bump_pointer`

### 8.2 size classes

allocations are bucketed by Form size:
- **class 0**: ≤32 bytes (empty form, no shape committed)
- **class 1**: ≤64 bytes (1-4 inline slots, no overflow)
- **class 2**: ≤128 bytes (5-12 slots with overflow)
- **class 3**: >128 bytes (big forms)

each class has its own slab allocator. saves space + improves cache
locality.

### 8.3 nursery (young gen)

per spec §3 umbrella + V1 work already shipped:

- bump-pointer; ~64KB initial size
- all turn-allocations land here
- on turn-commit: promote live → mature; reset nursery
- promotion: copy form to mature; rewrite FormId; record in
  forwarding map for any in-flight references

**bump-pointer alloc is ~100 ns** (atomic bump + zero out).

### 8.4 generational gc

- **young-only gc**: every turn-boundary. cheap (drop nursery
  after promotion).
- **mature mark-sweep**: every ~20 turns. ~30 ms @ 1M forms.
- **mature compaction**: every ~200 turns or at fragmentation
  threshold. ~100 ms @ 1M forms. forwarding table preserves L11.

**moof-side gc walker**: the substrate provides `[Heap iterate:
blk]` which calls `blk` for each (FormId, Form). moof's gc orchestrator
uses this to mark reachable forms via a moof-side mark set. the
substrate's role is **iteration and sweep**; the **walking is
moof.**

```moof
;; lib/early/gc.moof — the gc walker (moof code!)
(defmethod $gc (mark-roots)
  [$heap iterate: |id form|
    (if [form should-mark?]
        [self mark: id]
        nil)])
```

substrate then sweeps unmarked. **the gc policy is moof.**

### 8.5 shape gc

shapes are reachable iff any reachable form has them OR a reachable
shape transitions to them. shape gc runs alongside mature mark-sweep.

shape interner walks: for each shape, check refcount. if refcount
== 0 AND no parent has it in transitions, drop.

### 8.6 write barrier

on slot/handler/meta-mutation that writes a Form-typed value:
```
if target is in mature gen and value is in young gen:
    add target.id to remembered_set
```

at next young-only gc, remembered_set entries are roots.

cost: ~5 ns per mutation. amortized fine. if it bites, lazy-batch
into a thread-local buffer.

### 8.7 design choices, locked

- 4 size classes
- per-turn young-only gc (cheap)
- mark-sweep mature (every 20 turns)
- compaction (every 200 turns)
- moof-side gc walker; substrate iterates + sweeps
- shape gc piggy-backs on mature gc
- write barrier on Form-typed mutations only

---

## §9 — vats, scheduler, mailbox (MPMC)

### 9.1 Vat struct

```zig
pub const Vat = struct {
    id: VatId,                           // UUIDv7
    mode: VatMode,                       // .frozen_default | .mutable_default
    heap: Heap,                          // per-vat (00… scope)
    nursery: ?Nursery,                   // lazy: null until first alloc
    mailbox: ?*Mailbox,                  // lazy: null until first incoming msg
    outbox: Outbox,                      // pending cross-vat sends + intents
    here: FormId,                        // $here — root env, path-table seg
    behavior: FormId,                    // method-form for receive loop
    supervisor: ?FarRefId,
    caps: FormId,                        // cap-bag
    journal: ?JournalHandle,             // lazy: null until first mutation
    far_ref_table: ArrayListUnmanaged(FarRefEntry),  // 10… scope
    forwarding: AutoHashMapUnmanaged(FormId, FormId),
    scheduler_id: u8,                    // which thread owns me
    turn_counter: u64,                   // monotonic
};
```

**target idle size**: ~1 KB (most pointers null until first use).

**target spawn rate**: 100K/sec.

### 9.2 World struct

```zig
pub const World = struct {
    vats: AutoArrayHashMap(VatId, *Vat),
    shared: SharedSegment,           // 01… scope
    path_table: FormId,              // root path-table vat
    schedulers: []Scheduler,         // N pinned threads
    shape_interner: ShapeInterner,
    sym_pool: SymPool,
    chunks: AutoArrayHashMap(ChunkId, Chunk),
    proto_generation: AtomicU64,     // for IC invalidation
    image_store: ImageStore,         // merkle store handle
    mco_runtime: McoRuntime,         // wasm host
};
```

### 9.3 Scheduler

```zig
pub const Scheduler = struct {
    id: u8,
    thread: std.Thread,
    vats: ArrayListUnmanaged(*Vat),    // pinned to this scheduler
    runqueue: AtomicQueue(*Vat),       // ready-to-run
    current: ?*Vat,
    state: SchedulerState,             // .running | .idle | .stopping
};
```

each scheduler runs in a dedicated zig thread:
```zig
fn scheduler_loop(s: *Scheduler) void {
    while (s.state == .running) {
        // moof picks the next vat — substrate is the executor
        const next = call_moof_primitive("[$scheduler nextRunnable]");
        if (next == null) {
            wait_for_message();
            continue;
        }
        run_one_turn(next);
    }
}
```

**substrate runs threads; moof picks which vat is next.** the
substrate exposes `__pop_runnable_vat`, `__push_runnable_vat`,
`__wait_for_message`; the moof `$scheduler` form composes them.

### 9.4 Mailbox: michael-scott MPMC

```zig
pub const Node = struct {
    next: AtomicValue(?*Node),
    envelope: Envelope,
};

pub const Mailbox = struct {
    head: AtomicValue(*Node),
    tail: AtomicValue(*Node),
};
```

**enqueue** (lock-free, many producers):
```zig
pub fn enqueue(self: *Mailbox, e: Envelope, alloc: Allocator) !void {
    const node = try alloc.create(Node);
    node.envelope = e;
    node.next.store(null, .release);
    
    while (true) {
        const last = self.tail.load(.acquire);
        const next = last.next.load(.acquire);
        if (last == self.tail.load(.acquire)) {  // tail still consistent?
            if (next == null) {
                if (last.next.cmpxchgWeak(null, node, .release, .acquire) == null) {
                    _ = self.tail.cmpxchgWeak(last, node, .release, .acquire);
                    return;
                }
            } else {
                _ = self.tail.cmpxchgWeak(last, next.?, .release, .acquire);
            }
        }
    }
}
```

**dequeue** (single consumer per vat):
```zig
pub fn dequeue(self: *Mailbox, alloc: Allocator) ?Envelope {
    while (true) {
        const first = self.head.load(.acquire);
        const last = self.tail.load(.acquire);
        const next = first.next.load(.acquire);
        if (first == self.head.load(.acquire)) {
            if (first == last) {
                if (next == null) return null;  // empty
                _ = self.tail.cmpxchgWeak(last, next.?, .release, .acquire);
            } else {
                const env = next.?.envelope;
                if (self.head.cmpxchgWeak(first, next.?, .release, .acquire) == null) {
                    schedule_free(alloc, first);
                    return env;
                }
            }
        }
    }
}
```

### 9.5 memory reclamation

michael-scott has ABA + use-after-free risks. solution: **epoch-based
reclamation**.

```zig
pub const Epoch = struct {
    global: AtomicU64,
    per_thread: []AtomicU64,
    free_lists: [][]*Node,
};
```

each scheduler thread:
1. on entry to mailbox op: bump local epoch to global epoch
2. on exit: bump local epoch past current op
3. nodes freed get put on per-thread free-list with epoch stamp
4. lazy reclamation: every ~1000 frees, walk free-lists; reclaim
   nodes whose epoch is ≤ min(per_thread.epoch) - 2

cost: a few atomics per mailbox op. negligible.

### 9.6 cross-scheduler send

within-vat send: direct dispatch (no mailbox).
within-process-cross-vat send: `target.mailbox.enqueue(...)`.
cross-process send: serialize → network → remote enqueue.

**all paths use the same `Mailbox` primitive.**

### 9.7 design choices, locked

- N pinned schedulers (default N=cores)
- michael-scott MPMC mailbox
- epoch-based reclamation
- moof picks runnable vats (`[$scheduler nextRunnable]`)
- vat substructures lazy-allocated
- no work-stealing in v1 (deferred per umbrella §7.3)

---

## §10 — shared segment + intern table

### 10.1 shared segment

process-wide arena of frozen forms, indexed by `01…` scope.
**lock-free reads** for any vat; **CAS install** for promotion.

```zig
pub const SharedSegment = struct {
    arena: AtomicArrayList(Form),       // append-only
    intern: AtomicHashMap(Hash, u28),  // hash → arena index
    refcount: AtomicArrayList(u32),    // per-entry refcount
};
```

### 10.2 promotion

per spec §5 umbrella: lazy promotion on first cross-vat send of a
frozen form. canonical bytes → blake3 → intern table lookup. CAS
install if miss; reuse if hit.

forwarding pointer left in source vat's heap.

### 10.3 atomic intern install

```zig
pub fn intern_install(self: *SharedSegment, hash: Hash, bytes: []const u8) u28 {
    while (true) {
        const slot = self.intern.find_or_insert_slot(hash);
        const existing = slot.load(.acquire);
        if (existing != 0) {
            atomic_inc(self.refcount.items[existing - 1]);
            return existing - 1;
        }
        // allocate arena slot
        const idx = atomic_arena_append(self.arena, form_from_bytes(bytes));
        if (slot.cmpxchgWeak(0, idx + 1, .release, .acquire) == null) {
            atomic_set(self.refcount.items[idx], 1);
            return idx;
        }
        // CAS lost; another thread won; retry lookup
    }
}
```

### 10.4 design choices, locked

- single process-wide segment (not per-scheduler)
- intern lookup is lock-free; install is CAS
- refcount atomic per entry
- shared-segment gc: refcount=0 → reclaim
- cycles impossible (frozen forms only reference frozen + immediates)

---

## §11 — image i/o — V4 byte layer + merkle object store

### 11.1 V4 image format (byte level)

per `2026-05-10-vm-V4-opcodes-design.md` §10. **format unchanged**
in v2; only the storage layout (merkle) is new.

### 11.2 merkle object store

per umbrella §8:

```
.moof/store/
  objects/
    ab/cdef…              ← content-addressed Form blob
  refs/
    world/current
    world/turn-NNNN
    vats/<vat-id>
    scratch/<name>
  journal/
    <vat-id>/inputs.log
    <vat-id>/effects.log
  packs/
    pack-<sha>.pack
    pack-<sha>.idx
  jit-cache/
    arm64/<chunk-hash>.bin
    x86_64/<chunk-hash>.bin
```

### 11.3 save algorithm

per umbrella §9. all forms whose `hash_dirty` flag is set get
re-hashed at turn-commit; new objects accumulate in flush buffer;
batched i/o on flush trigger.

target: **<10 ms** typical flush; **<100 KB** written per flush.

### 11.4 load algorithm

per umbrella §8.4 + §15 below. lazy materialization; mmap-based;
forms paged-in on access.

### 11.5 substrate's role vs moof's role

- substrate: byte-level read/write of objects keyed by hash; ref
  atomic update; journal append/fsync
- moof: merkle walk; ref management; journal pruning; pack
  organization; gc

### 11.6 design choices, locked

- merkle store layout per umbrella §8.1
- substrate provides byte primitives only
- moof orchestrates everything above (merkle walk, refs, journal)
- jit-cache is per-arch + content-addressed

---

## §12 — mco runtime + mandatory serialize/restore

### 12.1 wasm host

wasmtime (or wasmer) embedded. one wasm engine per substrate
process; one instance per loaded mco.

abi per umbrella §11 + `2026-05-03-track-1-mcos-and-datasource-design.md`.
the import surface (the moof_* functions wasm calls back into):

```
moof_raise(kind_handle: u32, msg_ptr: u32, msg_len: u32) -> noreturn
moof_make_string(ptr: u32, len: u32) -> u32
moof_make_bytes(ptr: u32, len: u32) -> u32
moof_string_text(handle: u32, buf: u32, cap: u32) -> u32
moof_bytes_data(handle: u32, buf: u32, cap: u32) -> u32
moof_intern(ptr: u32, len: u32) -> u32
```

handle table per-instantiation, drained on dispatch exit (per LB-1).

### 12.2 mandatory serialize/restore

every mco implements:

```
mco_serialize(ctx: *MoofCtx, out_handle: *u32) MoofResult
mco_restore(ctx: *MoofCtx, bytes_handle: u32) MoofResult
```

per umbrella §11.1. manifest declares serializability category:
`'pure | 'linmem-only | 'rebind-handles | 'ephemeral-warn`.

substrate's role: at image-save, walk every mco-bound proto; call
`mco_serialize`; store the resulting bytes as a hash-addressed
object (same merkle store as forms). at image-load: fetch bytes by
hash; call `mco_restore`.

### 12.3 design choices, locked

- wasmtime as the wasm runtime (proven; ~5 MB embedded; covers our
  targets)
- per-instantiation handle table; drop guards on dispatch exit
- mandatory serialize/restore; no escape hatch
- mco bytecode loaded by hash from `.moof/store/objects/`
  (just like forms — unified store)
- per-arch AOT cache for instantiated mcos
  (`.moof/store/jit-cache/<arch>/<mco-hash>.cwasm`)

---

## §13 — the native primitives registry

### 13.1 full registry

these ~40 natives are the **only** native methods in the
substrate. anything else is moof or mco.

```zig
pub const PRIMITIVES = [_]NativeBinding{
    // ── Object — universal reflection + lifecycle ────────────────
    .{ "Object:slot:",           objSlotAt           },  // [form slot: sym]
    .{ "Object:slotAt:put:",     objSlotAtPut        },  // [form slotAt: sym put: val]
    .{ "Object:handlerAt:",      objHandlerAt        },
    .{ "Object:handlerAt:put:",  objHandlerAtPut     },
    .{ "Object:metaAt:",         objMetaAt           },
    .{ "Object:metaAt:put:",     objMetaAtPut        },
    .{ "Object:proto",           objProto            },
    .{ "Object:shape",           objShape            },
    .{ "Object:freeze",          objFreeze           },
    .{ "Object:frozen?",         objFrozen           },
    .{ "Object:identity",        objIdentity         },
    .{ "Object:perform:withArgs:", objPerformWithArgs },
    .{ "Object:become:",         objBecome           },
    
    // ── Cons — irreducible (the JIT inlines these to direct slot reads) ───
    .{ "Cons:car",               consCar             },
    .{ "Cons:cdr",               consCdr             },
    .{ "Cons:cons:",             consConsCdr         },
    
    // ── Integer — arithmetic primitives ──────────────────────────
    .{ "Integer:+",              intPlus             },
    .{ "Integer:-",              intMinus            },
    .{ "Integer:*",              intTimes            },
    .{ "Integer:/",              intDiv              },
    .{ "Integer:%",              intMod              },
    .{ "Integer:=",              intEq               },
    .{ "Integer:<",              intLt               },
    .{ "Integer:>",              intGt               },
    .{ "Integer:asFloat",        intAsFloat          },
    
    // ── Float — arithmetic primitives ────────────────────────────
    .{ "Float:+",                floatPlus           },
    .{ "Float:-",                floatMinus          },
    .{ "Float:*",                floatTimes          },
    .{ "Float:/",                floatDiv            },
    .{ "Float:=",                floatEq             },
    .{ "Float:<",                floatLt             },
    .{ "Float:>",                floatGt             },
    .{ "Float:asInteger",        floatAsInt          },
    
    // ── Char — byte-comparison primitives ────────────────────────
    .{ "Char:codepoint",         charCodepoint       },
    .{ "Char:<",                 charLt              },
    
    // ── String — byte access only (everything else is moof / utf8 mco) ───
    .{ "String:byteAt:",         strByteAt           },
    .{ "String:byteLen",         strByteLen          },
    .{ "String:byteEq:",         strByteEq           },
    .{ "String:concat:",         strConcat           },
    
    // ── Mailbox — concurrent primitives ──────────────────────────
    .{ "Mailbox:enqueue:",       mailboxEnqueue      },
    .{ "Mailbox:dequeue",        mailboxDequeue      },
    
    // ── Heap — iteration + intern + hashing ──────────────────────
    .{ "Heap:hashBlake3:",       heapHashBlake3      },
    .{ "Heap:internAt:put:",     heapInternAtPut     },  // CAS install
    .{ "Heap:iterate:",          heapIterate         },  // for moof-side gc
    .{ "Heap:atomicCas:slot:old:new:", heapAtomicCas },
    
    // ── Mco — load + dispatch + serialize/restore ────────────────
    .{ "Mco:load:",              mcoLoad             },
    .{ "Mco:call:args:",         mcoCallArgs         },
    .{ "Mco:serialize",          mcoSerialize        },
    .{ "Mco:restore:",           mcoRestore          },
    
    // ── $io (capability) — posix i/o ─────────────────────────────
    .{ "$io:readBytes:",         ioReadBytes         },
    .{ "$io:writeBytes:to:",     ioWriteBytesTo      },
    .{ "$io:mkdir:",             ioMkdir             },
    
    // ── $image (capability) — merkle store + journal primitives ──
    .{ "$image:writeObject:bytes:", imageWriteObject },
    .{ "$image:readObject:",     imageReadObject     },
    .{ "$image:setRef:to:",      imageSetRef         },
    .{ "$image:getRef:",         imageGetRef         },
    .{ "$image:appendJournal:vat:", imageAppendJournal },
    
    // ── $time / $entropy — capability primitives ─────────────────
    .{ "$time:now",              timeNow             },
    .{ "$entropy:bytes:",        entropyBytes        },
    
    // ── $scheduler — substrate-side scheduler hooks ──────────────
    .{ "$scheduler:nextRunnable", schedNextRunnable  },
    .{ "$scheduler:waitForMessage", schedWaitForMsg  },
};
```

**count: 51 natives.** every one is a method on its proto.

### 13.2 dispatch from JIT'd code

each native has a stable C ABI fn pointer. the JIT'd code calls
through this pointer directly — no overhead beyond the call itself.

```zig
pub fn nativeDispatchEntry(ctx: *VmContext, receiver: Value, args: []const Value, native_id: u16) callconv(.C) Value {
    const fn_ptr = PRIMITIVES[native_id].fn;
    return fn_ptr(ctx, receiver, args);
}
```

target: **<50 ns per native call** including IC dispatch.

### 13.3 native handler signature

```zig
pub const HandlerFn = *const fn(
    ctx: *VmContext,
    receiver: Value,
    args: []const Value,
) callconv(.C) Value;
```

handlers return a Value (the result of the message send). raise via
`ctx.raise(kind, msg)` which longjmps to the nearest catch.

### 13.4 design choices, locked

- ~51 native primitives, fixed at substrate v2 ship
- additions require spec amendment
- handler signature: `(ctx, receiver, args) -> Value`
- dispatch via stable native_id (compile-time enum)

---

## §14 — the minimal ocaml-seed v2

### 14.1 what shrinks

current ocaml-seed produces seed.vat with 104 natives listed +
~305 chunks for parser+compiler+transporter. substrate v2's 51
natives means seed.vat's native-list shrinks.

```
seed.vat v2:
  natives declared:           51   (from 104)
  chunks (parser+compiler+
   transporter+main):        ~280   (from 305; smaller compiler with
                                     fewer special cases)
  total size:               ~50 KB   (from ~92 KB)
```

### 14.2 what stays

ocaml-seed v2 still:
- parses the minimal-subset (per `2026-05-10-self-host-and-rust-deletion-design.md` §4.1)
- compiles to V4 bytecode (exact byte-format match with moof
  Compiler.compileTop:)
- emits seed.vat with chunks, syms, natives, here_form

### 14.3 what drops

- redundant native declarations (Cons:length, Integer:abs, String:trim, etc.)
- V3 image format support (substrate v2 only reads V4)
- legacy debug machinery
- experimental opcode variants from V3-era

### 14.4 estimated ocaml LoC

current: ~3000 LoC (ocaml-seed)
target: **~2000 LoC** (~33% smaller)

### 14.5 design choices, locked

- still ocaml, dune, build-time only
- emits seed.vat (~50 KB)
- declares 51 natives
- no runtime requirements beyond substrate v2

---

## §15 — cold boot path

### 15.1 the boot sequence (target: <100 ms for settled image)

```
─ phase ────────────────────────  cost ──  cumulative
1. process startup (zig binary)   ~5 ms      5 ms
   - libc init
   - argv parsing
2. resolve image path             <1 ms      6 ms
   - $MOOF_IMAGE or .moof/store
3. mmap .moof/store/objects/      <1 ms      7 ms
   - kernel page table setup
4. read refs/world/current        <1 ms      8 ms
   - single file open + read
5. parse world manifest object    <1 ms      9 ms
   - which vats, scheduler config
6. init core data structures     ~10 ms     19 ms
   - sym pool, intern, alloc
   - shape interner re-hydrate from image
7. spawn N scheduler threads     ~10 ms     29 ms
   - pinned to N cores
   - thread-local epoch counters
8. lazy materialize root vat(s)  ~10 ms     39 ms
   - vat structs alloc
   - first-page-in via mmap
9. JIT cache scan + load          ~5 ms     44 ms
   - load .moof/store/jit-cache/<arch>/<chunks-touched-on-boot>
10. restore first-frame mcos     ~10 ms     54 ms
   - mco_restore for each bound proto
11. workspace runnable           ≤100 ms     ✓
```

### 15.2 what loads when

**eager** (always at boot):
- world manifest
- sym pool (interned strings the substrate needs)
- shape interner snapshot
- scheduler config
- first vat (the workspace root) — its struct + first-frame forms

**lazy** (paged in on access):
- other vats — materialize when first message arrives
- forms within a vat — kernel pages in via mmap as accessed
- chunks — load when first send routes through them
- mcos — restore when first call comes in
- jit-cache entries — load when first hot chunk runs

### 15.3 image-as-canon means no re-bootstrap

**critical**: cold boot does NOT re-parse source files, re-compile
moof, or re-evaluate stdlib loads. all of that happened the LAST
time the image was saved. boot reads the saved state.

(re-bootstrap from source happens once, ever, when the user runs
`moof bootstrap` — produces system.vat for the first time.)

### 15.4 short cold boot enablers

every architectural choice contributes:
- merkle store (mmap'd; unchanged objects = same page)
- content-addressed forms (no per-form deserialize)
- shape interner snapshot (no shape-rebuild)
- sym pool snapshot (no re-intern)
- jit cache (no per-chunk JIT compile)
- inline cache state in image (no warmup)
- lazy vat substructures (no eager allocation)
- tight binary (fast process startup)

### 15.5 design choices, locked

- target: 50-100 ms cold boot, settled image
- eager: only world manifest + first vat + interners
- lazy: everything else
- no re-bootstrap on cold boot

---

## §16 — swap-in plan

### 16.1 sequencing

1. **architecture spec ratified** (this doc, after review)
2. **conformance corpus + runner authored** (~1 week)
   - ~100 image+message+expected triples
   - `moof conform <manifest.json>` runnable command in current substrate
3. **substrate v2 implementation** (~8-12 weeks)
   - module by module per §2.3 budget
   - each module passes its perf gate before merge
4. **conformance crosscheck** — v2 runs the same corpus; results
   byte-identical to current substrate (modulo perf metric drift)
5. **delete `players/zig/` and `players/rust/`** — full atomic
   replacement with `players/v2/` (renamed from new substrate dir)
6. **ocaml-seed v2 ships** — declares the new 51-native list

### 16.2 risk during the transition

- **conformance is the contract**. v2 must pass before old is
  deleted. no "we'll fix it later."
- **the rewrite happens in `players/v2/` worktree.** master keeps
  current substrate running.
- **rolling tests**: each module merge runs the conformance suite.
  if it regresses, that module's design is revisited.

### 16.3 design choices, locked

- atomic swap when conformance passes
- old players deleted, not deprecated-and-left
- conformance suite is the verifier

---

## §17 — open questions + risks

### 17.1 NaN-boxing scheme detail

current §3.1 plan: `Value = u32 = FormId`. floats live in
`float_pool`. **risk**: every float op pays an extra indirect load.
**mitigation**: floats are uncommon in moof workloads (most numeric
work is integer). measure first; if it bites, consider u64 Value
with NaN-boxed f64.

### 17.2 stencil per-arch maintenance burden

each (arm64, x86_64, x86_64-windows, arm64-windows, linux variants)
needs a stencil library. **risk**: stencil libraries diverge in
subtle ways; bugs catchable only by conformance.
**mitigation**: minimal arch set at v2 ship (arm64-darwin,
x86_64-linux, arm64-linux); add more as users demand.

### 17.3 scheduler ↔ moof boundary perf

`[$scheduler nextRunnable]` is called every turn — that's the
hottest path through the moof / substrate boundary. **risk**: if
this primitive is slow, every turn pays the cost.
**mitigation**: inline-fast-path the primitive in JIT'd code;
substrate provides `__pop_runnable_vat_inline` (memory-mapped
queue, no method dispatch); moof's `nextRunnable` wraps it.

### 17.4 mailbox memory reclamation under contention

epoch-based reclamation is simpler than hazard pointers but can
delay frees indefinitely if one thread parks. **risk**: memory
leaks in pathological cases.
**mitigation**: time-bound epoch advance; force GC if no thread
progress for >100 ms. tune in measurement.

### 17.5 shape gc complexity

shapes form a DAG via parent + transitions pointers. **risk**:
cycles in the shape graph could prevent gc.
**mitigation**: by construction, shape transitions only ADD slots
(forward direction); no cycles possible. shape gc is a topological
sweep.

### 17.6 mco serialize/restore for stateful protos

wgpu canvas, lmdb, websocket — these can't fully serialize their
external state. **risk**: image save with these in flight loses
state.
**mitigation**: per umbrella §11.5 — mco declares
`'ephemeral-warn` if it can't recover; user-visible warning at
save. consumer (workspace vat) handles.

### 17.7 jit-cache invalidation across substrate versions

substrate v2 → v3 may change stencil layouts. **risk**: stale
jit-cache crashes or misbehaves.
**mitigation**: cache key includes substrate version hash. version
mismatch invalidates cache. fresh JIT on next boot.

### 17.8 the ~6500 LoC budget

current rust substrate is ~10K. zig substrate is ~10.7K. v2 target
is ~6500. **risk**: budget overrun.
**mitigation**: per-module budget enforced; refactor when bloated;
optionally push more to mcos (utf8 walking, hash variants, etc.)
to keep substrate slim.

### 17.9 conformance suite authoring effort

~100 triples is ~2-4 weeks of careful work. **risk**: corpus is
incomplete; v2 ships with undiscovered bugs.
**mitigation**: incremental authoring. start with the ~30 most
critical (dispatch, freezing, gc, mailbox); add as needed.

### 17.10 ocaml-seed v2 effort

reducing from 3000 LoC ocaml → 2000 LoC is a real refactor.
**risk**: ocaml work is its own task.
**mitigation**: parallel to substrate v2 implementation; ocaml-seed
v2 is its own ~2-week effort.

---

## §18 — see also

- `2026-05-16-vats-substrate-and-image-design.md` — umbrella design
- `2026-05-16-phase3-cohesive-vision-design.md` — perf path, vat ergonomics
- `2026-05-16-phase2-moof-performance-design.md` — tier-2/3 perf playbook
- `2026-05-10-vm-V4-opcodes-design.md` — V4 ISA + image format
- `2026-05-10-self-host-and-rust-deletion-design.md` — self-host arc
- `2026-05-03-track-1-mcos-and-datasource-design.md` — mco ABI + LB-1
- `docs/concepts/forms.md`, `vats.md`, `references.md`,
  `replication.md`, `compiled-objects.md`, `capabilities.md`
- `docs/laws/substrate-laws.md`, `determinism-laws.md`
- haberman, k. "copy and patch compilation: a fast compilation
  algorithm for high-level languages and bytecode." (2021)
- chambers, c & ungar, d. "an efficient implementation of Self,
  a dynamically-typed object-oriented language based on prototypes."
  (1991) — the shape-table foundation

---

`٩(◕‿◕｡)۶` — methods not free fns, eat from beginning, JIT from line 1, ~6500 LoC, ~50 ms cold boot. **the spec we build to.**
