# phase 1 substrate optimization — gc + dispatch refactor + image
# compression — design

> **status:** brainstormed 2026-05-11. ready for writing-plans.
>
> **scope:** three substrate refactors that together unblock full
> self-host and rust deletion: (1) a mark-sweep GC at turn boundaries,
> (2) a single-loop VM dispatch eliminating rust-stack recursion, and
> (3) zstd compression on V4 vat-images. all three live in
> `crates/zig-substrate/`. ocaml-seed gets a one-line change
> (compression flag in header). rust substrate is untouched — it's
> deletable shortly, and these are all forward-looking.
>
> **prior reading:** `2026-05-10-self-host-and-rust-deletion-design.md`
> (the overall self-host arc), `2026-05-10-vm-V4-opcodes-design.md` §10
> (image format — where compression slots in), `2026-05-04-vats-and-
> references-protocol-design.md` §22 (V1 nursery design that V2/V3
> already shipped in rust and that this GC partly resurrects in zig),
> `laws/substrate-laws.md` (esp. L10/L11), `laws/determinism-laws.md`
> (esp. D5/D6).

## 1. context — why this package, what it unblocks

### 1.1 where we are

state at HEAD `4b21407` (2026-05-11):

- the V4 polyglot substrate works: rust at build time produces a 21.6
  MB `system.vat`; zig at runtime loads it; the world is alive but
  largely inert (`crates/zig-substrate/src/{world.zig, image.zig}`).
- the V4 byte-tagged ISA is implemented in zig (`vm.zig`, 678 LoC)
  with 24 opcodes per §3 of the V4 opcode spec.
- rust still owns the runtime CLI (`crates/substrate/src/{vm.rs,
  main.rs}`) for backwards compatibility and as a build-time oracle.

self-host requires three more pieces beyond what's currently working
(per the self-host design §5 W1–W5): `compiler.moof` V4 audit,
`parser.moof`, and `moof-zig` proof-of-life executing chunks.
**before any of those land safely, the substrate has three sharp
edges that need filing**:

1. **no GC.** task #13 measured 3.8M closure-Forms with `:source`
   meta after a moderate compile workload. every `moof-reader` send
   allocates an Env-form; every `PushClosure` allocates a closure-
   Form. without reclamation these persist for the life of the vat.
   any meaningful compile-execute-compile loop OOMs.

2. **rust-stack recursion in dispatch.** task #12 documents that
   `run_method → step → send_via_ic → invoke → run_method` adds ~5
   rust frames per non-tail moof Send. ~26 deep nested moof sends
   blow the default 8MB stack. `crates/substrate/src/main.rs:16-30`
   ships a 128MB worker thread as the interim workaround:

   ```rust
   // background: the rust VM dispatches Op::Send via real rust
   // recursion (run_method → step → send_via_ic → invoke →
   // run_method). every non-tail moof send adds ~5 rust frames.
   // moof code with deep nesting (esp. recursive parsers,
   // compilers, transducers) trivially exceeds the default 8 MB
   // main-thread stack. proper fix: refactor the VM to a single
   // dispatch loop that pushes/pops frames in a Vec without
   // rust-stack recursion.
   std::thread::Builder::new()
       .name("moof-main".into())
       .stack_size(128 * 1024 * 1024)
       .spawn(real_main)
   ```

   the zig substrate at `crates/zig-substrate/src/vm.zig:494-548`
   inherited the same shape (`invokeMethod → runMethod → step → …`).
   without fixing this, deeply-recursive moof code blows the zig
   stack too (and the workaround is uglier in zig — `Thread.spawn`
   with a custom stack size requires more ceremony).

3. **vat-images are big.** `system.vat` is 21.6 MB today; with full
   stdlib + macros + workspace state we project 200-360 MB. shipped
   `system.vat` is embedded in the moof binary (per V4 §10.6) and
   cold-loaded at every startup. zstd-class compression typically
   buys 5-10× on this shape (lots of repeated bytecode patterns,
   const-pool strings, FormId u32s with regular high bits). a 22 MB
   binary that decompresses in ~30ms is much better than a 22 MB
   binary that already takes 30ms to read off disk.

### 1.2 the unblock

with these three changes shipped, the self-host milestones in
`2026-05-10-self-host-and-rust-deletion-design.md` §5 become
achievable without contortion:

| self-host step | what this package gives it |
|---|---|
| W1 (compiler.moof V4) | compile-test loop doesn't OOM; deep recursion in the compiler doesn't blow stack |
| W2 (parser.moof) | parser is the heaviest allocator in moof — without GC, even smoke tests leak megabytes |
| W4 (moof exec) | chunks loaded from system.vat plus on-the-fly compiled chunks coexist without bloat |
| W5 (rust deletion) | rust's 128MB stack workaround no longer carries us — zig has to stand on its own |

phase 1 is the **make zig substrate production-viable** package.
rust deletion (W5) is the deliverable that depends on it.

### 1.3 what this package is NOT

- not a JIT, not type-specialized arithmetic, not PICs (those are
  phase 2/3 — §10).
- not generational GC (also phase 2 — the V1-nursery in the vats spec
  §22 is the natural next step; phase 1 ships a simpler design and
  upgrades later).
- not vat-image schema changes beyond a compression flag.
- not a rust-side change. rust's vm.rs survives untouched and rust
  deletes wholesale post-self-host. interim rust users keep their
  128MB workaround.

## 2. goals + exit criteria

### 2.1 goals

- **G1** — bounded heap. running `[parser parse: large-file]`
  followed by 1000 invocations of the result, without GC, OOMs;
  with GC, peak heap stabilizes (within ~2× the live set).
- **G2** — bounded stack. recursive moof code (say, recursive parser
  combinators on a deeply-nested AST) runs on a default-stack-size
  thread without blowing the rust/zig host stack.
- **G3** — small images. `system.vat` for the current stdlib
  shrinks from ~22 MB to ~2-4 MB on disk; load wall-clock either
  improves or stays within 10% of uncompressed.
- **G4** — all three changes preserve L10 (IC invalidation), L11
  (FormId stability within a vat), D4 (deterministic allocation
  order — applies when replication mode is on), D5 (insertion-order
  iteration), D6 (GC at turn boundaries only in replicated mode).

### 2.2 exit criteria

- [ ] **E1** — `zig build test` passes; substrate smoke (the
  `PushNil;Return` and `LoadConst;LoadConst;Send;Return` patterns
  from `vm.zig` doc comments) still works.
- [ ] **E2** — `moof exec system.vat <chunk-id>` runs without
  worker-thread stack-size workaround; default thread stack (~8MB)
  is sufficient for the deepest stdlib code path.
- [ ] **E3** — under a synthetic allocation-heavy workload (run
  `[1 + 2]` in a hot loop for 1M iterations), the heap stabilizes
  within 2× live set; no monotonic growth.
- [ ] **E4** — `system.vat` produced with `--compress=zstd` is ≤25%
  of the uncompressed size; load wall-clock is within 10%.
- [ ] **E5** — rust-built `system.vat` (no compression) and
  zig-loaded works; zig-built `system.vat` (with compression) and
  zig-loaded works; the round-trip is byte-deterministic per D5/D9.
- [ ] **E6** — every change passes a "law audit" (a manual pass
  through `substrate-laws.md` + `determinism-laws.md`, documenting
  no regression).

## 3. GC design (mark-sweep at turn boundaries)

### 3.1 algorithm choice

three serious candidates:

1. **mark-sweep, sparse heap, no compaction**
   - pros: trivial w.r.t. L11 (FormIds never change); minimal code;
     no write barriers needed; piggybacks on the existing
     `Heap.forms: ArrayList(Form)` layout (`heap.zig:48`).
   - cons: heap stays sparse over time — payload values fragment but
     `forms.len()` only grows. eventually need compaction (phase 2).

2. **semispace copying, with forwarding-pointer table**
   - pros: compacts; ideal cache locality; common in modern runtimes.
   - cons: violates L11 *unless* every old FormId installs a
     forwarding indirection. that indirection is exactly what
     `Heap.redirects` already does for `become_`, so the mechanism
     exists — but every read becomes a chase-the-chain.
     `heap.zig:35` already caps chains at MAX_BECOME_HOPS=32, so
     adding GC indirections would dilute that budget.

3. **generational (nursery + old-gen), matches V1 design**
   - pros: amortizes well; matches the per-turn nursery pattern in
     `vats-spec §22 V1`.
   - cons: bigger lift; nursery needs its own scope tag in FormId
     (the `11…` reserved range — `vats-spec §5`); write barriers on
     old→new pointers. phase 2 territory.

**decision: option 1 (mark-sweep, no compaction).** rationale:

- minimum-viable; gets us past the leak; doesn't paint us into a
  corner for option 3 later (generational *uses* a mark-sweep
  collector for the old generation).
- L11 falls out for free: we don't reuse FormIds within a vat
  lifetime within a session, we just tombstone dead slots in
  `forms` and let the slot stay vacant.
- when fragmentation pressure builds (likely never in phase 1; only
  in long-running federation vats), we add option 3.

a sparse heap means the `forms` ArrayList has holes. **a tombstone
form** (proto = NONE, empty slots/handlers/meta, marked `:dead`)
occupies the dead slot. allocators must skip tombstones (or, easier,
just keep extending — fragmentation isn't a real problem until
gigabytes).

future option: a free-list of dead slot indices, reusable by
`Heap.alloc`. **but FormId is stable** — reusing a FormId for a new
form after the old form dies would only work if nothing on disk or
in another vat still holds a reference to the old id. for phase 1
we say no: tombstones stay tombstones; allocator appends.

### 3.2 roots — what to mark

a Form is **live** iff reachable from a root. roots in the zig
substrate:

| root | where | lives in code |
|---|---|---|
| `world.here_form` | the vat's `$here` Form | `world.zig:239` |
| `world.macros_form` | macro registry | `world.zig:244` |
| all 18 proto-Forms | `world.protos.*` | `world.zig:184`, `protos.zig` |
| `world.transporter_root` if set | string, not a Form | n/a |
| every entry in `world.vm.frames[].chunk` | the chunk Form being executed | `vm.zig` Frame.chunk |
| every entry in `world.vm.frames[].env` | the env Form for that frame | Frame.env |
| every entry in `world.vm.frames[].self_` (if .form) | the receiver | Frame.self_ |
| every entry in `world.vm.frames[].defining_proto` | for super-send | Frame.defining_proto |
| every Value on `world.vm.stack` (if .form) | operand stack | Vm.stack |
| every IC entry's `cached_proto / cached_method / cached_defining / cached_singleton` | inline caches | `world.zig:126` ICache |
| every key/value in `world.far_ref_table` keys (FormIds with .far_ref scope) | far-ref-targeted Forms | `world.zig:227` |
| every entry in `world.native_fns` keys (method-FormIds) | native bindings | `world.zig:224` |
| every chunk-id key in `world.chunk_bytecode` / `chunk_consts` / `chunk_ics` / `chunk_params` | the chunk-Form itself | `world.zig:215-221` |
| every Value in `world.chunk_consts[*]` (if .form) | constant pool | same |
| every entry in `world.proto_generation` keys | proto-FormIds tracked for IC invalidation | `world.zig:232` |
| `become_` redirects keys + values | aliased FormIds | `heap.zig` redirects |
| pending phase D promise queue (future) | placeholder | not yet |
| mailbox / outbox (future, vats phase) | placeholder | not yet |

every Form-typed Value (i.e. tag = `.form`) reached from any of
these roots is **live**. transitive reachability follows:

- a Form's `proto` Value (if .form) → live.
- every value in slots / handlers / meta (if .form) → live.

method-Forms link to their `:body` chunk-Form via a slot containing
a `Value{ .form = chunk_id }`. closure-Forms link to their `:env`
env-Form similarly. so the chunk-Form is reachable from the method-
Form, and `world.chunk_bytecode[chunk_form_id]` is the chunk's
storage. **the chunk side-table is keyed by FormId, indexed
post-mark.**

### 3.3 the marking pass

a single mark bit per Form. tri-color in implementation, but pause
behavior is **stop-the-world at turn boundaries** — we don't need
incremental for phase 1.

```
fn gc_mark_phase(world: *World) !void {
    // 1. clear marks on every form.
    for (world.heap.forms.items) |*f| f.gc_mark = false;

    // 2. seed worklist with roots.
    var worklist: ArrayList(FormId) = ...;
    add_if_form(world, &worklist, world.here_form);
    add_if_form(world, &worklist, world.macros_form);
    for (world.protos.fields()) |proto_id| add_if_form(...);
    for (world.vm.frames.items) |frame| {
        add_if_form(..., frame.chunk);
        add_if_form(..., frame.env);
        if (frame.self_ == .form) add_if_form(..., frame.self_.form);
        add_if_form(..., frame.defining_proto);
    }
    for (world.vm.stack.items) |v| if (v == .form) add_if_form(...);
    for (world.chunk_consts.values()) |consts|
        for (consts) |c| if (c == .form) add_if_form(...);
    for (world.chunk_ics.values()) |ics|
        for (ics) |ic| {
            add_if_form(..., ic.cached_proto);
            add_if_form(..., ic.cached_method);
            add_if_form(..., ic.cached_defining);
            add_if_form(..., ic.cached_singleton);
        }
    for (world.chunk_bytecode.keys()) |chunk_id| add_if_form(..., chunk_id);
    for (world.far_ref_table.keys()) |fr_id| add_if_form(..., fr_id);
    for (world.native_fns.keys()) |method_id| add_if_form(..., method_id);
    for (world.proto_generation.keys()) |proto_id| add_if_form(..., proto_id);
    var redir_it = world.heap.redirects.iterator();
    while (redir_it.next()) |e| {
        add_if_form(..., e.key_ptr.*);
        add_if_form(..., e.value_ptr.*);
    }

    // 3. drain worklist.
    while (worklist.pop()) |fid| {
        const f = world.heap.get(fid);
        if (f.gc_mark) continue;
        f.gc_mark = true;
        if (f.proto == .form) try worklist.append(f.proto.form);
        var sit = f.slots.iterator();
        while (sit.next()) |e| if (e.value_ptr.* == .form)
            try worklist.append(e.value_ptr.form);
        var hit = f.handlers.iterator();
        while (hit.next()) |e| if (e.value_ptr.* == .form)
            try worklist.append(e.value_ptr.form);
        var mit = f.meta.iterator();
        while (mit.next()) |e| if (e.value_ptr.* == .form)
            try worklist.append(e.value_ptr.form);
    }
}
```

implementation note: the existing `Form` struct in `form.zig` needs
one bit. either repurpose padding or add `gc_mark: bool` (one byte
of bloat per Form — negligible).

### 3.4 the sweep pass

```
fn gc_sweep_phase(world: *World) !void {
    var i: usize = 1; // skip sentinel
    while (i < world.heap.forms.items.len) : (i += 1) {
        const f = &world.heap.forms.items[i];
        if (!f.gc_mark) {
            // tombstone: free slot/handler/meta maps; mark as dead.
            f.slots.deinit(world.allocator);
            f.handlers.deinit(world.allocator);
            f.meta.deinit(world.allocator);
            // re-init empty so the slot's invariants stay clean;
            // distinguishable from "live form with no slots" by
            // .proto == .nil and .frozen = false. consumers must
            // never reach a tombstoned id via a live root.
            f.* = Form.init();
            f.proto = .nil;
            // remove from associated side-tables.
            const fid = FormId.vatLocal(@intCast(i));
            _ = world.chunk_bytecode.swapRemove(fid); // also frees the body slice — see deinit
            _ = world.chunk_consts.swapRemove(fid);
            _ = world.chunk_ics.swapRemove(fid);
            _ = world.chunk_params.swapRemove(fid);
            _ = world.native_fns.swapRemove(fid);
            _ = world.proto_generation.swapRemove(fid);
        }
    }
}
```

side-tables (chunk_bytecode, chunk_consts, chunk_ics, chunk_params,
native_fns, proto_generation, far_ref_table) hold owned heap bytes
per `world.zig:456-489`. **sweep must free them when the keyed form
is collected**, otherwise GC leaks the side-table contents even
though the Form is gone.

`far_ref_table` is special: the FormId there is a `.far_ref`-scope
id, never tombstoned by this collector (which scans only vat-local
scope). but if no live form references the far-ref FormId, the
entry can be dropped — same logic, just keyed differently.

post-sweep, `world.heap.forms.items.len` is unchanged. fragmentation
accumulates. that's fine for phase 1; phase 2 (compaction) is the
fix when it becomes a real problem.

### 3.5 when to run — scheduling

three candidate triggers:

- **A. turn-boundary (default).** between `[vat receive: msg]` and
  the next message dispatch, the heap is quiescent (per
  `vats-spec §15`). this is exactly D6 ("GC runs at turn boundaries
  only"). solo vats don't have turn boundaries today, so we synthesize
  one at every `runTop` exit. this is the **recommended trigger**.

- **B. allocation-threshold (heuristic).** after every N allocations
  (say N = 100,000), run a GC. simple; doesn't depend on vats; but
  D6 forbids GC mid-turn for replicated vats. so this is only valid
  for solo mode + when no replication is active. we'd guard it.

- **C. on-demand `[$gc collect]`.** moof code triggers manually.
  useful for benchmarks / debugger; not load-bearing.

**decision:** ship A as the primary mechanism. ship C as a moof-
visible intrinsic (`Gc:collect`) for testing and explicit control.
defer B until we observe heaps growing within a single turn (which
happens with the compile-heavy load that motivated this work; we
may need this sooner than expected).

solo-mode adapter: the zig substrate doesn't have a vat scheduler
yet (that's phase D+). until then, "turn boundary" means "after
`runTop` returns" in `vm.zig:92`. add a `gc_after_run_top: bool =
true` flag on World; flip to disable for tests.

for moof-zig's CLI:
- `moof exec <vat> <chunk>` — runs once; no need for inter-run GC.
- `moof repl <vat>` — runs many times; GC after each top-level eval.
- `moof load <vat>` — never runs; no GC needed.

**replicated vats (future, D6 strict):** GC must happen between turns
*and* be deterministic. mark order is determined by root iteration
order; if all the side-tables iterate in insertion order (D5: they
do — they're `AutoArrayHashMapUnmanaged`), then mark traversal is
deterministic. sweep walks `forms` in order. **all the iteration is
already determinism-shaped.** good.

### 3.6 compaction policy — not now

phase 1: no compaction. tombstones accumulate. `forms.items.len`
grows monotonically.

quantitative bound: a single compile of `lib/` allocates ~600K
Forms (per NEXT_SESSION.md heap.len = 596,905 after stdlib boot).
if GC runs every 100K allocs, we have 100K live + 500K dead = 600K
total. with 5 GC cycles, we have 100K live + 2.5M dead = 2.6M
total. **for a long-running session this is a problem.** phase 2
(compaction) is the fix; phase 1 just notes the wart.

interim mitigation: if `forms.items.len` exceeds some threshold
(say 10M), refuse further allocation with an error. crude but
honest — better than silent memory bloat.

### 3.7 the `:source` meta question

task #13 measured 3.8M closure-Forms with `:source` meta after a
moderate workload. **is `:source` actually load-bearing on
closures?**

per L5: "every closure / method has a `:source` slot containing the
*actual source-form* it was compiled from. bytecode is derived."
this is for *introspection* — `[m source]` returns the form,
debuggers show the call site. it's not used for *execution*.

a closure created by `PushClosure` (V4 spec §3.5) doesn't strictly
need `:source` to *run* — the chunk's `:source` slot is enough. the
closure's `:source` is `(fn (params...) body...)` — the wrapping
form — which is convenient but redundant given chunk-Form's source.

**proposal:** make closure `:source` lazy. `PushClosure` does NOT
populate `:source` on the closure-Form. when `[closure source]` is
called (an intrinsic), it computes the source from the chunk's
`:source` plus the closure's `:params`. eliminates 3.8M Form-slot
writes in the measured workload.

this is a separate (smaller) change, but lands inside the same
package because it materially reduces GC pressure. it's also
backward-compatible: `[closure source]` still returns a Form;
reflection still works.

deferred to §10 (future work) if it slips: the GC alone gets us
80% of the way; the `:source` lazy refactor is bonus.

## 4. dispatch refactor design (single loop, no rust-stack recursion)

### 4.1 the problem in code

`crates/zig-substrate/src/vm.zig:494-548`:

```zig
fn invokeMethod(world, method, self_v, call_args, defining_proto) {
    if (world.nativeFn(method)) |native| return native(...);
    // bytecode method
    const call_env = try world.allocEnv(captured_env);
    // ... bind params ...
    return runMethod(world, chunk_id, call_env, self_v, defining_proto);
}

fn runMethod(world, chunk, env, self_v, defining_proto) {
    const starting_depth = world.vm.frames.items.len;
    try world.vm.frames.append(world.allocator, Frame{...});
    while (world.vm.frames.items.len > starting_depth) {
        try step(world);                           // ← rust call
    }
    return world.vm.stack.pop();
}

pub fn step(world) {
    // ... decode op ...
    try dispatchOp(world, decoded.op);             // ← rust call
}

pub fn dispatchOp(world, op) {
    switch (op) {
        .send => |args| {
            const result = try sendViaIC(...);     // ← rust call
            // ...
        },
        ...
    }
}

fn sendViaIC(world, receiver, sel, args, ic) {
    // ... IC fast path or slow lookup ...
    return invokeMethod(...);                      // ← rust call → recursion
}
```

every non-tail moof Send walks:
`runMethod → step → dispatchOp → sendViaIC → invokeMethod → runMethod → …`

that's 5 rust frames per moof send. **on a default 8MB rust/zig
stack, ~26 nested moof sends overflow.** the 128MB workaround in
`crates/substrate/src/main.rs:28` is the duct tape.

### 4.2 the target shape — one outer loop

```
pub fn runTop(world: *World, chunk: FormId) !Value {
    const starting_depth = world.vm.frames.items.len;
    try push_frame(world, chunk, world.here_form, .nil, FormId.NONE);
    while (world.vm.frames.items.len > starting_depth) {
        try step_one(world);
    }
    return if (world.vm.stack.items.len > 0) world.vm.stack.pop().? else .nil;
}

pub fn step_one(world: *World) !void {
    // decode one op of the topmost frame.
    const frame = top_frame(world);
    const decoded = try bytecode.decodeOp(...);
    frame.pc = pc + decoded.advance;
    try dispatch_op(world, decoded.op);  // ← does NOT recurse for sends
}
```

the key change: **`dispatch_op` returns to the outer loop after
every op**, including Send. Send's job becomes "push a new frame
and return"; the outer loop's next iteration steps the new top
frame.

```
.send => |args| {
    // dispatch: lookup, maybe IC, maybe native.
    const lookup_result = try resolve_send(world, ...);
    switch (lookup_result) {
        .native_call => |nc| {
            // native runs to completion in one outer-loop tick.
            // pop args/receiver; push native's return value.
            const result = try nc.native(world, nc.receiver, nc.args);
            pop_send_args(world, ...);
            try push(world, result);
        },
        .bytecode_call => |bc| {
            // bind params into a fresh env; push a new frame; return.
            // the outer loop will pick up the new top frame next tick.
            // current frame's pc has already advanced past Send,
            // so when the new frame's Return pops it, dispatch
            // resumes after the Send.
            const call_env = try world.allocEnv(bc.captured_env);
            try bind_params(world, call_env, bc.params, bc.args);
            pop_send_args(world, ...);
            try push_frame(world, bc.chunk_id, call_env, bc.receiver,
                            bc.defining_proto);
            // (no recursion! no `return invoke()`!)
        },
    }
}
```

**`Return` op** still pops a frame and pushes the result onto the
caller frame's operand stack (same semantics as today; the change
is just that the outer loop now picks up the caller). semantically
identical to recursion; mechanically, no rust stack used.

### 4.3 frame representation — no structural change

the existing `Frame` struct in `world.zig:100-114`:

```zig
pub const Frame = struct {
    chunk: FormId,
    pc: usize,
    env: FormId,
    self_: Value,
    stack_base: u32,
    defining_proto: FormId,
};
```

is already sufficient. it already supports:
- multiple frames stacked
- saved pc per frame
- saved stack_base per frame for `Return` to truncate to

we just stop using rust recursion to *push* frames. Frame layout
unchanged. **L11 and binary compatibility with rust's Frame
unchanged.**

new helpers needed:

```zig
fn push_frame(world, chunk, env, self_, defining) !void;
fn pop_frame(world) Frame;  // returns popped frame; caller uses
                            // popped.stack_base to truncate
fn top_frame(world) *Frame;
```

trivially derived from existing code.

### 4.4 the Send op — what to do

current `vm.zig:166-177` (`.send => |args| { ... }`) does:

```zig
const result = try sendViaIC(world, receiver, args.selector, call_args, args.ic_idx);
world.vm.stack.shrinkRetainingCapacity(receiver_idx);
try world.vm.stack.append(world.allocator, result);
```

— it gets a result back synchronously, because `sendViaIC` recurses
through `invokeMethod → runMethod` and unwinds all the way back.

new shape: `sendViaIC` becomes `prepare_send_dispatch`, which
*either*:

- **(a) determines the call is to a native fn.** runs the native
  inline, returns the result Value. (natives don't recurse into
  moof code — or if they do, they call `world.send(...)` which
  pushes onto the same VM stack. natives that recurse become a
  concern; see §4.5.)

- **(b) determines the call is to a bytecode method.** allocates
  the call env, binds params, **pushes a new Frame**, returns. the
  caller (the dispatch_op `.send` case) does NOT push a result —
  the new frame will do that via its eventual `Return`.

`.send`'s handler in dispatch_op:

```zig
.send => |args| {
    const argc: usize = args.argc;
    if (world.vm.stack.items.len < argc + 1) return error.SendArgcOverflow;
    const receiver_idx = world.vm.stack.items.len - argc - 1;
    const receiver = world.vm.stack.items[receiver_idx];
    // we DO NOT pop yet — the dispatch may need args still in place
    // for native_call, and bytecode_call moves them to env bindings.
    const action = try resolve_dispatch(world, receiver, args.selector,
                                         receiver_idx + 1, argc, args.ic_idx);
    switch (action) {
        .native_done => |result| {
            // shrink off args + receiver; push result.
            world.vm.stack.shrinkRetainingCapacity(receiver_idx);
            try world.vm.stack.append(world.allocator, result);
        },
        .bytecode_pushed => {
            // shrink off args + receiver; frame already pushed.
            // the new frame's eventual Return pushes the result.
            world.vm.stack.shrinkRetainingCapacity(receiver_idx);
        },
    }
},
```

**TailSend** stays simple — frame replacement is already non-
recursive. just bypass the new-frame-push entirely; reuse
`replaceFrameWithTailCall` from `vm.zig:566`.

### 4.5 native calls — when natives re-enter the VM

several natives in `intrinsics.zig` need to invoke moof methods
themselves. e.g. `Object:eval:`, `Closure:callIn:withSelf:`,
mapping/iteration helpers (`List:map:`).

today these call `World.send(...)` or `vm_mod.runMethod(...)`
which **re-enters the dispatch loop recursively**. that's the
exact pattern we're trying to eliminate.

two options:

- **option α: native re-entry is allowed.** natives that need to
  call moof code call `World.runUntilFrameReturns(...)` which pushes
  a frame, runs the outer loop until *just that frame's depth*
  returns, then returns the result. this *is* still rust-stack
  recursion if the native is mid-call when it does this, but only
  one level deep per nested native call. since natives don't form
  deep recursive chains the way moof code does, this is acceptable.

- **option β: native re-entry forbidden.** natives return a
  "continuation" Value that the outer loop handles. ugly; requires
  rewriting every re-entrant native.

**decision: α.** natives keep working. one level of rust recursion
on nested native→moof→native is fine — the depth is bounded by
natives-in-the-stdlib (~50). it's the *unbounded* moof→moof
recursion that the package fixes.

implementation: rename `runMethod` to `runUntilFrameReturns`;
clarify its role in doc comments; tighten its signature
(`World.send`-callers should clearly be entering a sub-VM).

### 4.6 error flow

zig errors via `try` already unwind cleanly through the outer loop
— the loop is `try step_one(world)`. errors raised by a native or
by VM machinery propagate up to `runTop` and back to the caller.

what about errors raised mid-bytecode (e.g. `'unbound-name`)? same
— `step_one` returns the error; the outer loop returns; everything
unwinds.

what about errors raised by a native that the native catches? same
as today: the native handles it locally, returns the result.

**no change in error semantics.** the change is purely structural.

### 4.7 tail-call threaded dispatch (future)

zig 0.16 supports `@call(.always_tail, fn, args)` which compiles
to a direct jump rather than a call+return. with one handler per
opcode and tail-calls between handlers, dispatch overhead drops
~2-3× (the classic luajit/wasm3 design).

shape:

```zig
fn op_send(world: *World) anyerror!void {
    // ... handle send ...
    const next_op = decode_next(world);
    return @call(.always_tail, dispatch_table[next_op.tag], .{world});
}
```

**deferred to phase 2 (§10).** the single-loop refactor in phase 1
is the structural prerequisite — once dispatch isn't recursing into
itself, swapping `switch` for `@call(.always_tail)` is mechanical.

### 4.8 mirror in rust?

rust substrate is deletable per the self-host design. **we don't
refactor rust's vm.rs.** the 128MB worker thread workaround stays;
it ships for as long as rust ships. when rust deletes, the
workaround deletes with it.

argument for mirroring: rust is still used at build time
(`v4_export.rs` calls into the rust VM to build `system.vat`). but
build-time is one-shot, not a live runtime — the 128MB worker
thread is fine.

argument against: every hour spent refactoring rust's vm.rs is
deferred from deleting rust. **deletion wins.**

## 5. image compression design (zstd)

### 5.1 what to compress

three options:

1. **whole image post-header.** simplest. one zstd frame; load reads
   header (fixed-size, plaintext), decompresses everything after.
2. **per-section.** Forms-section gets its own frame, Chunks-section
   gets its own, etc. allows partial / lazy decompression.
3. **specific large sections only.** Forms (largest) compressed;
   header / symtable / footer plaintext.

**decision: option 1 (whole image post-header).** rationale:

- the header is ≤ ~200 bytes — compression buys nothing.
- per-section compression is more complex (more frame boundaries,
  more decode calls). zstd's dictionary doesn't span frames, so
  per-section compression compresses worse.
- option 3 (large sections only) is a half-measure; option 1 is
  cleaner with the same compression ratio.

**exception:** the footer (32-byte blake3 hash) stays uncompressed
*outside* the compressed blob. the hash covers the compressed bytes
or the decompressed bytes (§5.6 — decision below).

### 5.2 algorithm choice

three serious candidates:

| algorithm | typical ratio on this data | decode speed | encode speed |
|---|---|---|---|
| zstd (level 3) | 5-8× | ~1500 MB/s | ~500 MB/s |
| zstd (level 19) | 7-10× | ~1500 MB/s | ~20 MB/s |
| lz4 | 3-4× | ~3000 MB/s | ~700 MB/s |
| brotli (level 11) | 8-11× | ~400 MB/s | ~1 MB/s |
| zlib (level 9) | 5-7× | ~400 MB/s | ~50 MB/s |

**decision: zstd at level 3 for default, level 19 for `--release`.**
rationale:

- zstd is the consensus "best general-purpose compressor" of the
  last decade; in zig 0.16 it's `std.compress.zstd`.
- level 3 encodes fast enough for dev mode; level 19 takes seconds
  but compresses tighter for shipped builds.
- decode is the hot path (every user runs `moof load` at startup;
  almost no one runs `moof build-image`), and decode is the same
  speed regardless of encode level.
- lz4 is faster on decode but the 2× compression ratio penalty is
  the wrong tradeoff for an embedded-in-binary image.
- brotli is tighter but slower to decode; not a win on 22MB.

### 5.3 header flag

add one byte to the V4 header **after `Version` and before
`vat_id`**. for backward compatibility this byte is zero in
existing V4 (no compression) images.

```
Magic   := "MVAT" (4 bytes)
Version := u16 (0x0004)
Compression := u8                  // NEW
    0 = none (default; matches existing images)
    1 = zstd
    2 = lz4    (reserved; not implemented in phase 1)
    3 = brotli (reserved; not implemented in phase 1)
DecompressedSize := u64 BE         // NEW — only present if Compression != 0
                                   //   needed for streaming decode allocation
Header := { vat_id, num_forms, num_syms, ... }  // unchanged
... (rest of sections; compressed if Compression != 0)
```

with `Compression = 0`, the header bytes from `vat_id` onward read
unchanged — **existing V4 images load with no code change** beyond
the loader reading one extra byte that's 0. for compressed images,
the loader reads the byte, then `DecompressedSize`, then decompresses
the rest into a scratch buffer and continues parsing as before.

**backward compatibility shim:** if the existing files in the wild
have `Compression` == something else (because we added the byte
post-hoc and a stale image lacks the byte), we'd need a flag. but
images are produced fresh from rust today and shipped from CI;
there's no in-wild fleet. we can change the format without breakage
as part of phase 1.

a minor variant: **bump Version to 0x0005**. cleaner; signals "new
format." downside: rust's existing emitter writes 0x0004; we'd
either (a) bump rust's emitter too (small change to `v4_export.rs`)
or (b) make the loader accept 0x0004 and 0x0005.

**decision:** bump Version to 0x0005. mirror in rust's
`v4_export.rs` (one-line constant change). loader rejects 0x0004
images with a clear "rebuild your system.vat" error.

### 5.4 API design

**writer side** (`serializeVat` in `image.zig:269`):

```zig
pub const SerializeOptions = struct {
    compression: Compression = .none,
    zstd_level: i32 = 3,
};

pub fn serializeVat(
    world: *const World,
    out: *std.ArrayList(u8),
    allocator: std.mem.Allocator,
    options: SerializeOptions,
) !void {
    // 1. emit Magic + Version + Compression byte.
    // 2. if compression == .none:
    //      emit the sections + footer as today.
    //    if compression == .zstd:
    //      a. serialize the sections to a scratch buffer.
    //      b. compute footer hash over scratch (decompressed) bytes.
    //      c. write DecompressedSize:u64.
    //      d. zstd.compress(scratch) into `out`.
    //      e. append footer hash (uncompressed).
    // 3. done.
}
```

**reader side** (`loadVatImage` in `image.zig:149`):

```zig
pub fn loadVatImage(
    world: *World,
    bytes: []const u8,
    allocator: std.mem.Allocator,
) !void {
    // 1. verify magic + version.
    // 2. read compression byte.
    // 3. if compression == .none:
    //      use `bytes` as-is (same as today).
    //    if compression == .zstd:
    //      a. read DecompressedSize:u64.
    //      b. allocate scratch buffer of size DecompressedSize.
    //      c. zstd.decompress(bytes[pos..pos+compressed_len], scratch).
    //      d. continue parsing as today, using scratch instead of bytes.
    //      e. defer allocator.free(scratch);
    // 4. emit per-section parses + footer verification.
}
```

zig 0.16's `std.compress.zstd`:

- `std.compress.zstd.Decompressor` — streaming reader, suitable for
  the size we have (decompresses into our pre-allocated scratch
  buffer in one shot).
- `std.compress.zstd` does not currently ship an encoder. for the
  writer side we have options:
    - use a wrapped C zstd (vendor zstd as a build-time dep)
    - shell out to `zstd` CLI at build time
    - keep writer uncompressed in dev mode, use a CI step to
      compress shipped images

**decision:** **always-uncompressed writes from zig substrate** for
phase 1; compression at the build-step level via the `zstd` CLI tool
or a wrapped library. zig's decoder is stable; encoder lands later.

ocaml-seed (`crates/ocaml-seed/`) similarly writes uncompressed
images; a build-time post-processing step compresses for shipping.

this avoids the C-dep question and lets us land the loader-side
change cleanly. the writer-side encode is a follow-on (§10).

### 5.5 determinism

zstd with a fixed dictionary, single-frame, default parameters, is
**deterministic** — same input bytes produce same output bytes.
this matters for D9 (canonical hash) when we want hash-equivalence
of the compressed image.

caveat: zstd's compression level affects output bytes (different
levels = different bytes). for D9 invariance we'd freeze a level
(say 19 for shipped images) and require it for canonical-hash
comparisons.

**recommendation:** D9 (canonical-state-hash) is computed over the
**decompressed** bytes, not the compressed image. this way:

- compression is a transport optimization, not a semantic property.
- two vats with same logical content but different compression
  levels still hash equal.
- the footer's content-hash (currently 32 zero bytes per
  `image.zig:457`) becomes the decompressed-bytes hash, computed
  before compression on write and after decompression on read.

### 5.6 footer hash — covers decompressed bytes

current: `image.zig:184-189` stubs the footer as 32 bytes (zero in
both rust and zig until phase 9 / wire-up).

new: footer = blake3(bytes_in_image_before_compression). on read,
after decompression, recompute and verify. on write, compute
before compression.

this matches the "the hash is a content-address" semantic from V4
§10.9; the compressed image is *a representation* of the canonical
state; the hash is the canonical state's identity.

### 5.7 streaming vs whole-image

for the 22 MB → 360 MB image-size range, **whole-image decode** is
fine:

- pre-allocate a `DecompressedSize`-byte scratch buffer.
- zstd into it in one call.
- parse the scratch buffer as before.

streaming would save peak RAM (the compressed image bytes can be
freed before parse), but the parse already needs the full
decompressed bytes in memory (FormSection, ChunkSection cross-
reference SymTableSection by index). saving the compressed bytes is
~22 MB; not worth the complexity.

streaming is an option later (phase 2) if image sizes grow past a
few GB. unlikely for the foreseeable future.

## 6. interactions between the three changes

### 6.1 GC + dispatch refactor

**dependency:** GC needs to walk frames; the dispatch refactor
changes how frames are pushed/popped but not their structure.
**no conflict, but ordering matters:**

- if dispatch refactor lands first, GC implementation is cleaner
  (one place to scan frames, one shape for "what's a live frame").
- if GC lands first, the GC root-walker has to handle two shapes
  for frames (the current rust-recursing one and the future
  single-loop one) — fine but extra work.

**recommendation:** dispatch refactor first.

### 6.2 GC + compression

**independent.** compression touches image-load and image-serialize.
GC touches heap. they don't share state.

post-load, GC may collect Forms allocated during decompression
(scratch buffers, etc.). but those are owned by the loader, not the
heap; they don't escape. **no interaction.**

### 6.3 dispatch refactor + compression

**independent.** dispatch refactor touches vm.zig. compression
touches image.zig. zero overlap.

### 6.4 all three + downstream self-host

self-host's W1-W5 (per `2026-05-10-self-host-and-rust-deletion-
design.md` §5) depend on phase 1:

- W1 (compiler.moof V4 audit) — needs GC (compile-test loop OOMs
  without it); doesn't need dispatch refactor or compression
  strictly, but benefits from both.
- W2 (parser.moof) — needs GC (parser allocates heavily); needs
  dispatch refactor (parsing recursive descent is recursive).
- W3 (ocaml-seed minimization) — independent of phase 1.
- W4 (moof exec) — needs dispatch refactor (otherwise repeats rust's
  128MB workaround). benefits from GC + compression.
- W5 (rust deletion) — gated by all of phase 1 plus W1-W4.

phase 1 is on the critical path. all three changes ship before W5.

## 7. migration plan

### 7.1 sequencing within phase 1

recommended order:

1. **dispatch refactor first.** lowest risk; pure structural change;
   smallest blast radius (one file: `vm.zig`). easy to verify with
   existing smokes. **estimated effort: 1-2 days.**

2. **GC second.** depends on dispatch refactor for cleanest frame
   walking. moderate risk (root enumeration is easy to get wrong;
   missing a root crashes; over-conservatively keeping roots leaks).
   **estimated effort: 3-5 days** including soak/leak tests.

3. **compression last.** independent of the other two. low risk
   (zstd is well-tested; format change is one byte). **estimated
   effort: 1-2 days.**

total: ~5-9 days of focused work, plus ~2-3 days of integration
testing.

### 7.2 backward compatibility

- **existing system.vat (V4 v0x0004) files:** loader detects
  version mismatch, emits clear error: "system.vat is V4 v0x0004;
  rebuild with `moof-rs export-v4` to get v0x0005 (with optional
  compression byte)." rebuild is one command; no user-data loss.
- **rust-built images:** rust's `v4_export.rs` needs to bump
  Version to 0x0005 and emit the Compression byte = 0. one-line
  change.
- **ocaml-seed:** same one-line change.
- **shipped binaries:** during phase 1, ship two binaries: `moof`
  (zig, new) and `moof-rs` (rust, old). `moof-rs` produces v0x0004
  files; `moof` (zig) produces v0x0005. they don't load each
  other's files. **for the brief overlap period this is fine** —
  rust deletes shortly after.

### 7.3 feature flags

- **`GC_DEBUG`** (zig comptime) — extra checks in GC: mark every
  Form unmarked at sweep start, verify no live form was reached via
  a tombstoned id.
- **`GC_ENABLED`** (world flag) — `world.gc_enabled: bool = true`.
  set false for benchmarks and bisection. **the test suite must
  pass with `GC_ENABLED = true` AND `GC_ENABLED = false`.** GC bugs
  hide easily; this is the bisection knob.
- **`COMPRESSION_DEFAULT`** (compile-time constant in serializer) —
  default `.none` during dev; `.zstd` for release builds. set via
  build flag.
- **`DISPATCH_SINGLE_LOOP`** — not a flag, just the new code. once
  shipped, the recursive shape is gone.

### 7.4 staged rollout

1. land dispatch refactor; ship `moof exec` with default stack
   size; verify against the recursive-parser corpus (build a few
   500-deep test forms and parse them through moof).
2. land GC behind `GC_ENABLED=true`; flip to default-on after a
   week of soak.
3. land compression byte in header (uncompressed default); update
   rust + ocaml-seed emitters.
4. flip compression to default-on for `moof-rs build-image` (with
   `--compress=zstd` flag controlling).
5. ship a fresh `system.vat` (compressed, default).

### 7.5 measurement

baseline (HEAD `4b21407`):

- `system.vat` size: 21.6 MB
- cold load time: ~ 100ms (per zig substrate load smoke)
- compile-and-execute leak: 3.8M closures per task #13's workload
- stack required for deep recursion: 128 MB workaround

phase 1 targets:

- `system.vat` size: ≤ 5 MB (compressed)
- cold load time: ≤ 120ms (with decompression)
- compile-and-execute leak: 0 (bounded heap; GC reclaims)
- stack required for deep recursion: ≤ 8 MB default

each is a measurable success criterion mapped to exit criteria
§2.2.

## 8. risks

### 8.1 GC risks

1. **missed root.** if a root isn't enumerated, GC collects a live
   form; next dereference is a use-after-free / dispatches against
   tombstoned data. **mitigation:** GC_DEBUG mode that scans every
   currently-tracked Value site at every GC and warns if any
   `Value{.form = id}` exists where `id` resolves to a tombstoned
   form post-sweep. exhaustive root-table in §3.2 above.

2. **iteration during mutation.** if a native fn modifies
   `chunk_consts` or `slots` mid-sweep, sweep crashes. **mitigation:**
   GC is single-threaded and runs only between turns; nothing else
   is mutating concurrently. easy invariant; documented.

3. **side-table desync.** GC must remove form-ids from side tables
   (chunk_bytecode, native_fns, etc.). missing one leaks; doing it
   wrong dangles. **mitigation:** side-table list is hardcoded in
   `sweep_phase`; new side tables added to `World` must be
   registered there. add a comptime assertion or a `// GC SWEEP:
   update on add!` comment on each side-table field in `world.zig`.

4. **moof-visible heap-id stability.** L11 says FormIds are stable
   within a vat. mark-sweep preserves this (no IDs reused). but if
   we ever reuse tombstone slots, we'd violate L11. **mitigation:**
   document "no reuse" in phase 1; phase 2 compaction must add a
   forwarding mechanism.

5. **freezing + GC interaction.** frozen forms are still GCable
   (they can be unreachable). per V2 (`2026-05-07-vat-V2-freezing-
   design.md`) freezing is a state bit, not a lifetime bit. no
   special-case needed.

### 8.2 dispatch refactor risks

1. **return-flow ordering.** native calls return synchronously; the
   dispatch handler must push the result and advance pc as today.
   bytecode calls push a frame and *don't* push a result (the new
   frame's Return will). easy to confuse. **mitigation:** clear
   types — `enum DispatchAction { native_done(Value), bytecode_pushed }`.
   one match site, one place to get it wrong.

2. **stack-base bookkeeping.** the new frame's `stack_base` must be
   set such that the new frame's eventual Return truncates the
   stack to the caller's "after-Send-pop" state, then pushes the
   result. **mitigation:** clarify the contract in code comments
   and tests; existing `Frame.stack_base` semantics are correct as-
   is; just need to maintain them through the new push-frame helper.

3. **TailSend interaction.** TailSend replaces the current frame;
   the new code must not push a fresh frame on top. current
   `replaceFrameWithTailCall` already does this; we just have to
   confirm the new helper structure preserves it. **mitigation:**
   existing TailSend tests still pass; add tests for tail-recursive
   moof functions running on default stack size (this is the
   primary regression check).

4. **native-from-native recursion.** option α in §4.5 lets natives
   recurse through `World.send`; this is rust-stack recursion. if a
   native calls a moof method that calls another native that calls
   another moof method... we're back to the original problem at a
   lower bound. **mitigation:** ban native-from-native deep
   recursion in the stdlib audit; bound by native count (~50).

### 8.3 compression risks

1. **format-version churn.** bumping V4 to 0x0005 means all
   `system.vat` files in wild become invalid. **mitigation:** there
   are no in-wild files; the only persistence is build-time. ship a
   migration script (rebuild). communicate clearly.

2. **zstd dependency.** zig 0.16 has decoder built-in; encoder is
   not present. **mitigation:** ship uncompressed writes from zig
   substrate; compression at build-step level. revisit when zig
   ships encoder or we vendor zstd-c.

3. **footer hash semantics change.** the footer now covers
   decompressed bytes (canonical state) rather than image bytes.
   different hash for same logical content vs same image bytes.
   **mitigation:** D9 prefers decompressed-bytes hash; document.
   the image-bytes hash (if anyone wants it) can be computed
   independently.

4. **decompression OOM.** a malicious image could declare a huge
   `DecompressedSize`. **mitigation:** cap `DecompressedSize` at,
   say, 1GB; reject larger. (a 1GB stdlib is implausible.)

### 8.4 cross-cutting

1. **measurement debt.** without a benchmark harness, "did this
   help?" is qualitative. **mitigation:** §9 testing strategy
   includes specific measurements with baseline + post numbers.

2. **rust + zig parity drift.** rust is deletable. but during the
   overlap, two substrates means two places for bugs. **mitigation:**
   accept the drift; rust is one-way out the door.

3. **moldability tax.** GC may impose pause times that affect
   interactive performance. **mitigation:** turn-boundary GC pauses
   should be sub-100ms for the foreseeable heap sizes; benchmark
   and add incremental GC in phase 2 only if needed.

## 9. testing strategy

### 9.1 GC tests

**unit-style smokes** (in `crates/zig-substrate/src/tests/` or
inline `test "..."` blocks):

1. allocate a Form not reachable from any root; trigger GC; verify
   `forms[fid]` is tombstoned.
2. allocate a Form reachable only via chunk_consts; trigger GC;
   verify it persists.
3. allocate a Form reachable only via an IC `cached_method`;
   trigger GC; verify it persists.
4. allocate a Form reachable only via the redirects table (a
   `become_` target); trigger GC; verify it persists.
5. allocate a closure-form whose env is reachable from current vm
   stack; trigger GC; verify env + closure both live.
6. tombstone-then-re-mark: allocate, drop ref, GC, allocate again;
   verify second alloc gets a fresh FormId (not the tombstoned one)
   — phase 1 invariant.

**integration**:

1. **leak test.** run `[1 + 2]` in a hot loop for 1M iterations.
   measure `world.heap.forms.items.len` periodically. assert
   plateau (within 2× of initial). regression check for §2.2 E3.
2. **soak test.** run the full stdlib compile loop 100× back-to-
   back. assert no OOM, no leaks.
3. **determinism test.** run the same workload twice; collect
   `world.heap.forms.items.len` time series; assert byte-identical
   (post GC). validates D5 / D4 / D9 isn't broken.

### 9.2 dispatch refactor tests

**correctness**:

1. existing smokes still pass: `PushNil;Return`, `LoadConst;LoadConst;
   Send;Return`, `(if #true 1 2)`, etc.
2. **deep-recursion test.** moof factorial of 1000 (a recursive
   non-tail-call) on default thread stack. assert no overflow.
3. **tail-call test.** `(loop forever)` tail-recursive moof; let
   run for 1M iterations; assert constant memory.
4. **native re-entry.** `Object:eval:` (which re-enters the VM)
   doesn't crash; recursion depth equals native depth, not moof
   depth.
5. **error flow.** raise mid-bytecode (e.g. unbound name); verify
   error propagates to `runTop`'s caller; verify frames clean up.

**performance**:

1. micro-benchmark: 1M `[1 + 2]` evals. compare pre- vs post-
   refactor wall-clock. expect ±10% (no perf regression).
2. macro-benchmark: full stdlib compile. compare wall-clock.

### 9.3 compression tests

1. **round-trip.** serialize world with `compression = .none`;
   load; assert byte-identical heap. serialize with `.zstd`; load;
   assert byte-identical heap.
2. **footer-hash.** load → modify one byte of compressed image →
   load → assert HashMismatch error.
3. **version-mismatch.** load a 0x0004 image with a 0x0005 loader;
   assert clear `UnsupportedVersion` error.
4. **size ratio.** measure `system.vat` size at `--compression=.none`
   vs `.zstd` levels 3, 9, 19. assert ≥4× at level 19.
5. **decode speed.** measure load wall-clock at `.none` vs `.zstd`.
   assert decode adds ≤10% to total load time.

### 9.4 cross-change tests

1. **all-three enabled.** load a compressed image, run a deep-
   recursive workload with GC enabled, verify all three play nice.
2. **stress.** parse `lib/` end-to-end via parser.moof (once W2
   ships), compile via compiler.moof (W1), run; measure heap, time,
   peak memory.

### 9.5 law-audit pass

manual pass through:

- `substrate-laws.md` L1-L16; document each as preserved or audited.
- `determinism-laws.md` D1-D12; same.
- `reflection-contract.md` — GC must not affect reflection (live
  forms reflect normally; tombstoned forms are unreachable).
- `isolation-laws.md` — phase 1 is single-vat; vat boundary rules
  are vacuous but should be preserved for phase D.

target: a `phase1-law-audit.md` document with a one-line statement
per law.

## 10. future work (phase 2/3)

### 10.1 generational GC with V1 nursery

next obvious GC refinement: split the heap into young (turn-scoped
nursery) and old (vat-scoped) generations. matches the V1 spec
exactly (`vats-spec §22 V1`). per-turn collections are cheap;
old-gen collections rare.

requires: write barrier on old→young pointer creation; a third
FormId scope tag (the `11…` reserved range in the V0 spec).

estimated effort: ~1-2 weeks. unblocks per-turn diff replay (phase
B persistence) cleanly.

### 10.2 heap compaction

reclaim tombstoned slots by compacting `forms`. requires either:
- forwarding indirection table (every old id → new id), violating
  L11 mildly (FormIds reshuffle but live references still work).
- a single point-in-time compaction at vat-save (image-write
  rewrites FormIds; live process doesn't touch).

estimated effort: ~1 week.

### 10.3 fused V4 opcodes (LoadName + Send, etc.)

V4 already fuses `LoadSelf + Send` → `SendSelf` and `LoadHere +
Send` → `SendHere`. similar fusions for `LoadName + Send` (a single
`SendNamed { name: SymId, argc, ic }` op) could compact bytecode
~15-20% based on the rust seed's chunk corpus. **deferred to phase
2**; cosmetic perf win until profiling demands it.

### 10.4 content-addressed form dedup (phase 9)

per V4 §10.9, the canonical-state-hash can identify equivalent
vats. extending this to *form-level* dedup (across vats in the
process-wide shared segment) is `vats-spec §5` "shared segment +
content-addressed promotion." requires LMDB-backed persistent
store; cross-vat coordination. **phase 9.**

### 10.5 tail-call threaded dispatch (zig 0.16 `@call(.always_tail)`)

once the single-loop dispatch from phase 1 is stable, each op-
handler can tail-call the next op-handler directly (no return-to-
loop). expected ~2-3× dispatch speedup. **phase 2.**

### 10.6 polymorphic ICs (PICs)

V4 spec §13 lists this as deferred. current ICs are monomorphic; a
miss invalidates and re-caches. polymorphic (4-entry hash on miss)
buys ~10-15% in highly polymorphic dispatch (e.g. message-passing-
heavy stdlib code). **phase 2.**

### 10.7 flat env arrays

env-Forms are AutoArrayHashMaps today (`world.zig:582`). flat
arrays would be ~2× faster for hot lookups. requires resolving
"how does `(def x 1)` add to a flat array" — probably via a
spill-to-map for late additions. **phase 2.**

### 10.8 source-attached-on-demand for closures

per §3.7 above: closure-Forms shouldn't eagerly carry `:source`;
reflection should reconstruct it from chunk + params. saves ~3.8M
form-slot writes per task #13's measured workload. **phase 1 if it
slips in cheaply; otherwise phase 2.**

### 10.9 zstd encoder via vendored C lib or wasm-of-ocaml

phase 1 ships only the decoder. encoder lives in build steps. when
zig ships a `std.compress.zstd.Compressor` (likely 0.17+), or when
we vendor zstd-c, switch encoder to zig-native too. **phase 2-ish.**

### 10.10 JIT

very far. only once profiling clearly shows interpreter dispatch
is the bottleneck even with tail-call threading. **phase G+.**

## 11. open questions

these are tagged for discussion before writing the implementation
plan; I couldn't resolve them with the context available.

1. **does the zig substrate ever observe FormId reuse?** phase 1's
   GC explicitly never reuses tombstoned ids. but the `forms`
   ArrayList grows unboundedly. **at what threshold do we flag
   this?** answer probably "1M live + tombstoned forms" but the
   correct number depends on workload data we don't have yet.

2. **does compression need to interact with the `mcos/` directory
   layout** (V4 spec §10.2)? mcos are content-addressed binaries
   shipped alongside `system.vat`. they're already individually
   compressed at the wasm level. compressing them again with zstd
   buys ~20-30% on average. **probably out of scope for phase 1**
   but worth flagging.

3. **GC + `become_` interaction.** `[a become: b]` registers a→b in
   `Heap.redirects`. after the become, references to `a` resolve to
   `b`. is `a` itself still a root? if `b` is reachable from some
   root and `a` only appears in the redirects-table key, is `a`
   live? **answer probably "yes, `a` is live as long as it's a
   redirects key, because anyone holding the old FormId still
   indexes through it."** but the GC root list in §3.2 above does
   include redirects keys; need to confirm this is the right
   semantics.

4. **the `:source` meta question on closures (§3.7).** is anything
   in the stdlib actually reading `[closure source]`? quick grep
   over `lib/` should answer. if no, removing the eager `:source`
   on closures is a free win.

5. **does ocaml-seed implement zstd?** if not, phase 1's
   `compression = .none` default keeps it working as today.
   eventually ocaml-seed needs to either:
   - emit uncompressed and rely on a post-step,
   - link a zstd binding (ocaml has them),
   - emit a flag saying "I'm raw, post-compress me." **deferred.**

6. **GC in solo mode without a vat scheduler.** zig substrate
   doesn't have a vat scheduler yet; "turn boundary" is synthesized
   at `runTop` exit. when phase D (real vats) lands, GC moves to
   the real turn boundary. **what's the migration story?** probably
   nothing — the trigger just moves; GC code stays.

7. **how does GC interact with native fn lifetime?** native fns are
   function pointers held in `world.native_fns`. they're zig code,
   not allocated forms. but the *method-Form* they're keyed by IS
   a heap-allocated Form. if that method-Form gets collected (no
   live reference to it), the native_fns entry should drop. §3.4
   does this. **edge case:** what if a native fn is being called
   when GC happens? same as "stop-the-world at turn boundaries"
   answer: GC doesn't happen mid-native. enforced by trigger
   choice (§3.5 A).

8. **compression for individual chunks** (cross-vat code shipping).
   when phase F ships and vats start sending chunks across the
   wire, per-chunk compression makes sense. **distinct from
   image-level compression and deferred.**

9. **dispatch refactor vs zig 0.16 async** (if zig ever ships
   stack-saving async). phase D's promise/scheduling ops (Suspend
   / Resume) might want zig async. **out of scope; flag for phase
   D design.**

## see also

- `2026-05-10-self-host-and-rust-deletion-design.md` — overall arc.
- `2026-05-10-vm-V4-opcodes-design.md` §10 — image format.
- `2026-05-04-vats-and-references-protocol-design.md` §22 V1 —
  nursery, the future generational layer.
- `2026-05-06-vat-V1-nursery-diff-design.md` — V1 nursery design,
  shipped in rust as the per-turn nursery; the generational-GC
  building block for phase 2.
- `2026-05-07-vat-V2-freezing-design.md` — freezing; orthogonal
  to GC.
- `laws/substrate-laws.md` — esp. L10/L11.
- `laws/determinism-laws.md` — esp. D4-D6.
- `crates/zig-substrate/src/vm.zig` — current dispatch.
- `crates/zig-substrate/src/world.zig` — heap, frames, side-tables.
- `crates/zig-substrate/src/image.zig` — serializer/deserializer.
- `crates/zig-substrate/src/heap.zig` — heap + redirects + L11 doc.
- `crates/substrate/src/vm.rs` — rust dispatch (deletion target).
- `crates/substrate/src/main.rs:16-30` — 128MB worker thread.
- `NEXT_SESSION.md` — state snapshot at HEAD `4b21407`.
