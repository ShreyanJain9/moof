# phase 2 substrate — moof performance — design

> **status:** profiled + brainstormed 2026-05-16; **real-workload
> measured + re-ordered 2026-05-16** (see §3.5). ready for plan.
>
> **scope:** make the zig substrate fast enough to (a) self-host the
> moof parser/compiler at sustained workloads, and (b) ship the
> phase E 3D-world demo without users noticing that the VM is
> interpreted. concretely: get `[1 is nil]` parsing — currently 90s
> via the in-image Parser at ~5.5 sends/sec — down under 100 ms,
> and put us on a roadmap to BEAM-interpreted parity (1-10M
> reductions/sec) for tier-1-2 work and BEAMJIT parity (100M+) for
> tier-3 work.
>
> **prior reading:**
> - `2026-05-11-phase1-gc-dispatch-compression-design.md` — phase 1
>   shipped GC + single-loop dispatch + image compression. that
>   removed the structural blockers; this spec attacks the
>   throughput blockers.
> - `2026-05-10-vm-V4-opcodes-design.md` — V4 opcode set, IC contract
>   (§6.1), fused-send rationale (§6).
> - `2026-05-10-self-host-and-rust-deletion-design.md` — why
>   `moof Parser is canonical` matters and what gates rust deletion.
> - `NEXT_SESSION.md` — V4 polyglot state at HEAD `4b21407`.
> - `laws/substrate-laws.md` — L3 (send is universal), L5 (source is
>   canonical), L10 (IC invalidation), L11 (FormId stability).
> - `laws/determinism-laws.md` — D5 (insertion-order iteration; bounds
>   what we can do with hash-table swaps).

## 1. context — the 5.5 sends/sec finding and why it blocks everything

### 1.1 the headline number

after the phase 1 refactor landed (single-loop dispatch + mark-sweep
GC + still-uncompressed images), the zig substrate runs `moof Parser`
end-to-end. it just runs *very slowly*. measurement against the
representative workload (parsing the literal expression `[1 is nil]`
through `[Parser parse:]` from `lib/parser/02-parser.moof`):

- **wall clock:** ~90 seconds
- **estimated sends:** ~500 (small expression, but the parser
  recursive-descents through every char + every dispatch state)
- **throughput:** ~5.5 sends/sec

a healthy bytecode interpreter dispatches at 1-10M sends/sec (BEAM,
CPython, SpiderMonkey baseline). we are **200,000× to 20,000,000×
slower than even a slow interpreter**.

### 1.2 why this blocks phase A.5 self-host

per `2026-05-10-self-host-and-rust-deletion-design.md` §5, the rust
deletion is gated on:

- W1 — compiler.moof V4 audit (compile-test loop must complete in
  reasonable time)
- W2 — parser.moof works against moof Compiler output (likewise)
- W4 — `moof exec` against a vat-image runs stdlib code

every one of these breaks at ~5.5 sends/sec. just *parsing* one
stdlib file is hours, not seconds. compiling all of `lib/` (with
parser running in-image) would be days.

### 1.3 why this blocks phase E (the demo)

the demo is interactive — a user types, the parser runs, a value
returns. at 5.5 sends/sec, every keystroke is a coffee break. demo
is unshippable without 10000× speedup on this workload.

### 1.4 the prior decision context

phase 1 was structural: stop bleeding (GC), stop crashing (dispatch
non-recursive), compress the image. **none of phase 1 touched the
per-Send cost.** that's phase 2.

we have two complementary places to gain: (a) reduce work-per-send,
(b) reduce sends-per-source-expression. this spec attacks both.

## 2. goals + exit criteria

### 2.1 goals

- **G1** — `[1 is nil]` parses end-to-end in **under 100 ms** (~1000×
  speedup vs. baseline 90 s).
- **G2** — `parser.moof` parsing a 1000-line stdlib file completes
  in **under 5 s** (rough proxy: 100k sends @ 50k sends/sec = 2 s).
- **G3** — sustained dispatch throughput **≥ 1M sends/sec** under
  realistic workloads (BEAM-interpreted parity). aspirationally 5-10M.
- **G4** — no regression in any phase-1 law audit (L10/L11 + D5/D6
  preserved). new techniques (PICs, inline arith, frame pool) are
  semantically transparent to moof code.
- **G5** — exit posture allows phase 3 (JIT) to land without
  re-doing tier-1 work.

### 2.2 exit criteria

- [ ] **E1** — `MOOF_FAST_ALLOC=1 moof eval system.vat "[1 is nil]"`
  completes in ≤ 100 ms wall (excluding image load + parser/compiler
  load) on the dev machine; reproducible to ±20 %.
- [ ] **E2** — `bench-parser-like 1000000` reports ≥ 5M sends/sec
  with smp_allocator and ≥ 500K sends/sec with the default debug
  allocator.
- [ ] **E3** — `stress-recursion 100000` reports ≥ 1M sends/sec
  (smp_allocator); proxy for sustained method-dispatch throughput.
- [ ] **E4** — IC hit rate under bytecode workloads is ≥ 95 %; under
  parser-like polymorphic workloads (PICs land) ≥ 90 %.
- [ ] **E5** — peak heap growth under sustained workload reduced
  ≥ 50 % vs. phase 1 baseline (the frame-pool + lazy-source + flat-
  env wins compound).
- [ ] **E6** — phase 1 tests pass unchanged.

### 2.3 explicit non-goals (for this phase)

- not a JIT (that's tier 3 / phase 3; this spec scopes it).
- not bignum / unboxed floats (V5+).
- not BigDecimal or specialized numeric types (separate effort).
- not GPU dispatch, not SIMD, not parallel scheduling. single-thread
  cooperative is the V4 model.

## 3. PROFILING RESULTS — measured 2026-05-16

instrumentation: `crates/zig-substrate/src/vm.zig` was extended with
a `Profile` struct of u64 counters at every hot path (send sites, IC
hit/miss, IC singleton check, frame push, env alloc, env-walk hops,
proto-chain walks, list-to-slice expansions, form alloc). `step()`
itself increments `ops_executed`. monotonic ns timing in
`monotonicNs()` (calls `std.c.clock_gettime(.MONOTONIC, …)`). all
benches are `-Doptimize=ReleaseFast`. measurements taken on
Apple Silicon (Darwin 25.2.0).

three benchmarks were added behind subcommands:

- `bench-loop N` — `N` iterations of `LoadConst; Pop` (no Send).
  measures pure dispatch floor.
- `bench-natives N` — `N` Sends of `:+` to an Integer (IC hits a
  native handler). measures native-Send fast path.
- `bench-parser-like N` — `N` × 4 sends across `:<`, `:>`, `:=`, `:+`
  on Integers. approximates lexer/parser sends.
- `bench-polymorphic N` — alternates `:!!` between true and nil
  receivers; every Send is an IC miss. measures slow-path cost.
- `bench-deep-env D` — fixed 100k LoadName ops in an env chain of
  depth `D`. measures env-walk linear cost.
- `stress-recursion D` — recursive `:rec:` self-send going `D` levels
  deep (bytecode method, env alloc per call, params binding,
  listToSlice).

a `MOOF_FAST_ALLOC=1` env var was added to flip from the
zig-default `DebugAllocator` to `std.heap.smp_allocator`. (the
DebugAllocator is the only meaningful "default": it's what `gpa.allocator()`
returns when built without options. it does corruption-check pages
+ stack-trace bookkeeping per alloc/free, which is **catastrophically
expensive at the per-Send granularity**.)

### 3.1 raw numbers (sends/sec)

| benchmark               | allocator | ns/send | ns/op | throughput   |
|-------------------------|-----------|---------|-------|--------------|
| bench-loop 1M           | smp       | n/a     | 17.7  | n/a          |
| bench-loop 1M           | debug     | n/a     | 29.4  | n/a          |
| bench-natives 1M        | smp       | 123.6   | 30.9  | 8.09 M/s     |
| bench-natives 1M        | debug     | 7,533.7 | 1,883 | 132 K/s      |
| bench-parser-like 100k×4| smp       | 146.7   | 36.7  | 6.82 M/s     |
| bench-parser-like 100k×4| debug     | 5,540.6 | 1,385 | 180 K/s      |
| bench-polymorphic 1M    | smp       | 127.1   | 42.4  | 7.87 M/s     |
| bench-polymorphic 1M    | debug     | 125.8   | 41.9  | 7.95 M/s     |
| bench-deep-env D=1      | smp       | n/a     | 63.2  | n/a (2 hops) |
| bench-deep-env D=10     | smp       | n/a     | 124.7 | n/a (11 hops)|
| bench-deep-env D=50     | smp       | n/a     | 472.4 | n/a (51 hops)|
| bench-deep-env D=50     | debug     | n/a     | 1,050 | n/a          |
| stress-recursion 100k   | smp       | 197.0   | 65.7  | 5.08 M/s     |
| stress-recursion 100k   | debug     | 6,216.7 | 2,072 | 161 K/s      |

### 3.2 the top three bottlenecks

#### 3.2.1 bottleneck #1: per-Send args_buf alloc (60× speedup available)

every call to `prepareInvoke` whose dispatch resolves to a *native*
fn with argc > 0 executes:

```zig
// crates/zig-substrate/src/vm.zig:780-786 (prepareInvoke native path)
const args_buf = try world.allocator.alloc(Value, argc);
defer world.allocator.free(args_buf);
@memcpy(args_buf, call_args);
world.vm.stack.shrinkRetainingCapacity(shrink_to);
const result = try native(world, self_v, args_buf);
return .{ .native_done = result };
```

— a heap alloc + memcpy + free per native Send. with DebugAllocator,
this single line dominates: bench-natives drops from 8.1M sends/sec
(smp) to 132K sends/sec (debug), a **61× slowdown**. with
smp_allocator the cost is still ~80ns/Send out of 124ns total
(estimate from ratio of bench-natives 124ns vs bench-polymorphic
127ns which has 0 allocs and similar work — wait, polymorphic does
allocate args_buf too if argc>0. let me re-read… polymorphic uses
argc=0 because `:!!` is unary. so the gap is real: ~80ns / Send of
the 124ns is the alloc + free.)

reason this happens: zig's "natives accept a `[]const Value` slice"
contract. the slice into the operand stack would be valid for the
duration of the native call **if natives didn't mutate the stack** —
but several natives (`Method:call`, `Closure:callIn:withSelf:`,
`Object:perform:withArgs:`) re-enter the VM via `world.send`, which
mutates the operand stack underneath. so we defensively copy. the
fix is to either (a) preserve the slice by establishing a no-stack-
mutation contract, or (b) reuse a per-vat scratch buffer.

#### 3.2.2 bottleneck #2: env alloc + listToSlice + envBind per bytecode call

every bytecode method dispatch (`prepareInvoke` taking the bytecode
branch) executes:

```zig
// crates/zig-substrate/src/vm.zig:780-810 (prepareInvoke bytecode path)
const body_v = world.formSlot(method, world.body_sym);          // slot lookup
const captured_env_v = world.formSlot(method, world.env_sym);   // slot lookup
const params_v = world.formSlot(method, world.params_sym);      // slot lookup
const params = world.listToSlice(params_v);                     // walk cons-chain, alloc slice
defer world.freeSlice(params);
const call_env = try world.allocEnv(captured_env);              // alloc Form + meta map
for (params, call_args) |param_v, arg_v| {
    const param_sym = param_v.asSym() orelse return error.BadParam;
    try world.envBind(call_env, param_sym, arg_v);              // hashmap put per param
}
world.vm.frames.append(...);                                    // frame push
```

per call: 3 slot lookups + 1 listToSlice (which walks the
params-list cons-chain, doing `n` slot reads and 1 slice alloc) + 1
allocEnv (which allocates a new Form with `meta.parent` set) + `n`
envBind (each a hashmap put) + 1 frame append. for a 1-arg method
that's roughly 6 hash operations + 2 allocations per call.

**stress-recursion 100k (smp_allocator) measures 197 ns / send for
bytecode methods**, vs 124 ns / send for the native path. the
difference is ~70 ns of bytecode-only overhead — env alloc dominates.

evidence the listToSlice path is wasteful: chunks already carry
`chunk_params: []u32` in their side table (`world.zig:224`). we
shouldn't be walking a cons-chain on the method-Form to recover what
we already have indexed by chunk-FormId. proof: `world.allocClosure`
in `world.zig:807-812` populates `:params` from `chunk_params` —
**we round-trip cons → slice on every call when the slice was the
source.**

#### 3.2.3 bottleneck #3: dispatch-loop overhead (~20-30ns / op floor)

`bench-loop` measures 17.7 ns/op on smp_allocator with optimize=ReleaseFast.
the inner loop is `step(world)` → `bytecode.decodeOp(bytes, pc)` →
`switch (decoded.op)`. each iteration:

1. read top frame from `world.vm.frames.items[len-1]`
2. read chunk bytes from `world.chunk_bytecode.get(chunk)` (hashmap lookup!)
3. decode the byte stream at `pc`
4. switch on the decoded op
5. for value-load / control: append to stack

the **per-step `chunk_bytecode.get(chunk)` hashmap lookup is wasted**
— the chunk doesn't change between steps. likewise `chunk_consts` is
looked up afresh in LoadConst's handler.

this is small (17 ns) but it adds up. with 1.2M ops in
stress-recursion 100k, this dispatch overhead is ~24 ms — small
fraction of the 2.5 s total, but a real fraction in tight loops
without bytecode-Send dominance.

### 3.3 secondary observations

- **IC hit ratio is 100%** in monomorphic micro-benches and ≥
  99.99% in stress-recursion. when the workload is polymorphic (the
  `bench-polymorphic` case alternating between Bool and Nil
  receivers), hit ratio collapses to 0%. **PICs would help here**;
  for fully monomorphic code (most of the stdlib *inside* a single
  send-site) the existing IC is already saturated.
- **load_name walk hops scale linearly** with env depth at ~9 ns /
  hop (smp). real moof code rarely has > 10-level chains except in
  pathological cases (deeply nested let-in-fn). but every LoadName
  pays at least 1 hop.
- **DebugAllocator is the dominant cost in EVERY allocation-heavy
  benchmark**. switching to smp_allocator buys 30-60× on Send-heavy
  workloads without code changes. **this is the lowest-hanging
  fruit and should ship in tier 0**.
- **forms_allocated tracks bytecode_dispatches almost 1:1** (100,056
  forms in stress-recursion 100k with 100,001 bytecode calls). each
  bytecode call allocates exactly 1 env-Form. **frame pool plus
  per-frame env caching would near-zero this.**
- **proto-chain walks are rare** when IC hits (only 5 walks in
  100,001 bytecode sends; the slow path is exercised only on the
  first call to each method, then cached). when the IC misses, the
  walk is fast (~1 hop in the polymorphic bench → 127 ns / Send).
- **no DNU dispatches** in any benchmark — surface checks out clean.

### 3.4 extrapolating to the `[1 is nil]` workload

we don't have the parser running in-image (anon-natives blocked),
but we can extrapolate. the parser does, per character:

- 5-10 sends for `:isWhitespace:` / `:isLetter:` / `:isDigit:` /
  delimiter checks (each calls `:<`, `:>` — `2 sends each, all
  polymorphic across `Char` vs `Integer` proto chains)
- 2-4 sends for cons construction / list manipulation
- 1-3 env walks per send (parser's recursive descent uses nested
  closures)
- ~5 form allocations per token

for `[1 is nil]` (8 chars) that's ~50-100 sends just at the lexer
level, then another ~50 sends in the parser proper, then ~30 in
the compiler. call it 150-200 sends.

at the *measured* **180K sends/sec with DebugAllocator on
parser-like workload**, 200 sends takes **1.1 ms**. that's nowhere
near 90 s. so the reported 5.5 sends/sec implies a 200× factor we
haven't isolated in the microbenchmarks — likely:

1. **deep env chains in the parser** (every nested closure adds a
   level). every LoadName per send pays ≥5 hops → +50 ns per
   LoadName × 5 LoadNames per send × 200 sends = 50,000 ns ≈ 0.05 s.
   not enough alone.
2. **listToSlice on the parser's intermediate cons lists** (the
   tokens list, the form-list output, the call-args slices for
   `Cons:map:` etc.). each list-walk pays N hashmap reads.
3. **IC misses in polymorphic code**. the lexer's `:<` site sees
   Integer x Integer most of the time, but Char x Integer too —
   each transition is an IC miss + slow-path resolve.
4. **the moof Compiler's many sends per emit-op** (each emit is a
   `[chunk push: byte]` cascade through several Closures).
5. **garbage collection mid-parse**. phase 1 GCs only at runTop
   exit, but every Sub-eval through `evalStringInWorld` calls
   runTop. so GC fires per form parsed. with a heap that grew
   transiently to 100k forms, marking 100k forms takes maybe
   ~10ms. over 100 forms parsed, that's 1 s. not enough alone but
   compounding.
6. **PRINTF DEBUGGING IN HOT LOOP.** `vm.zig:264` does
   `std.debug.print("UnboundName: ...")` and `vm.zig:709-716`
   prints arity-mismatch diagnostics including iterating method
   slots. if the parser sends `:doesNotUnderstand:with:` flowing
   through these paths even on success branches, the debug print
   alone costs ms per send. **check this immediately as a tier-0
   wart.**

the 90s number won't be one bottleneck but ~5-10 compounding ones
each costing ~3× — that's how interpreter perf works. each tier-1
fix unstacks a multiplier.

## 3.5 real-workload findings — measured 2026-05-16 (post tier-1)

with tier-1A/B already shipped (smp_allocator, args scratch, chunk
side-tables on Frame, chunk_params direct), microbenches report
6.8-8.1 M sends/sec. but the real bootstrap workload — `moof
profile-run /tmp/seed.vat` walking `lib/early/*.moof` files through
the in-image Parser + Compiler — measures at **45,400 sends/sec**:
a **178× gap from microbench**.

instrumentation added in commit `917e297` (the `profile-run`
subcommand, plus eight new Profile counters). methodology:

```
$ MOOF_LIB=$(pwd)/lib MOOF_PROFILE=1 \
    ./crates/zig-substrate/zig-out/bin/moof profile-run /tmp/seed.vat
  # ... killed after 60s; the run organically hits UnboundName
  # 'emit:' (issue #31, defmethod machinery) mid-bootstrap, ~39s
  # into the workload. profile dumps on graceful error.
```

### 3.5.1 measured counters (39.33 s run, killed organically at #31)

| metric                         | value          |
|--------------------------------|----------------|
| wall elapsed                   | 39.33 s        |
| ops executed                   | 4,820,757      |
| sends total                    | 1,785,432      |
|   native dispatches            | 1,250,318      |
|   bytecode dispatches          | 212,798        |
|   tail sends                   | 322,338        |
| throughput                     | 45,396 send/s  |
| ns/send                        | 22,029         |
| ns/op                          | 8,159          |
| IC hits                        | 1,454,350      |
| IC misses                      | 8,738          |
|   cold (empty slot)            | 995            |
|   **thrash (proto flip)**      | **7,702 (88%)**|
|   generation stale             | 41             |
| IC hit ratio                   | 81.46 %        |
| envs allocated                 | 388,986        |
| env_bind calls                 | 556,223        |
| listToSlice calls              | 106,442        |
|   total items walked           | 121,973        |
| load_name lookups              | 1,350,091      |
|   total env hops               | 1,854,112      |
|   avg hops per LoadName        | **1.373**      |
| formSlot lookups               | 1,274,030      |
| protoOf calls                  | 1,683,165      |
| **sym intern calls**           | **267,878**    |
| proto-chain walks              | 218,523        |
| **forms allocated**            | **746,071**    |
| runTop invocations             | 11             |
| GC runs (mid-workload)         | 9              |
| GC wall-ns                     | 257.3 ms       |
| GC share of wall               | 0.65 %         |

env-walk depth histogram (per LoadName resolve depth):

```
depth  1: 1,016,536   (75.3%)
depth  2:   244,769   (18.1%)
depth  3:    42,928   ( 3.2%)
depth  4:    25,234   ( 1.9%)
depth  5:     6,112   ( 0.5%)
depth  6:    13,844   ( 1.0%)
depth  7:       667   ( 0.05%)
```

### 3.5.2 the top 3 real-workload bottlenecks

#### bottleneck R1 — native intrinsic O(N) overheads (parser-internal)

**finding:** ns/op = 8,159 in real workload vs ~36 ns/op in
bench-parser-like — a **226× gap that the microbench cannot
detect because it only exercises tagged-int arithmetic natives**.

real native dispatches (1.25 M of them) include `String:at:`,
`String:length`, `String:=`, `Cons:car`, `Cons:cdr`, `(intern str)`
etc. several have **O(N) walks that hide behind a single Send**:

- `stringWalk` (helper for `String:at:` and `String:length`) walks
  the entire :bytes cons-chain every call. parser parses files
  thousands of chars long; each char read is O(file-position).
- `lookupSymByName(world, "bytes")` in **every string operation**
  does a linear scan of the 653-entry sym table — `653 × memcmp(~6
  ns)` ≈ **4 µs of pure-CPU waste per call**, just to recover a
  SymId that's already known at compile time. this single helper
  function may dominate string-native dispatch time.
- `(intern str)` (`globalIntern`) walks the input String's
  cons-chain into a UTF-8 buffer, then hits `syms.intern`. parser
  invokes per identifier read. correlates with the 267,878
  intern-calls counter — interning 267K syms when only 653 are
  unique means **410 redundant intern lookups per unique sym**.

evidence pinning intrinsics (not the dispatch loop): `protoOf`
calls (1.68 M) exceed `sends_total` (1.79 M, near-parity), which
is expected — every dispatch consults `protoOf`. but `formSlot
lookups` (1.27 M) are **separate from dispatch**: every `.car`
/ `.cdr` / `:body` / `:env` read in a native is one. 1.27 M
of these means the natives are reading cons-chain slots at a
~1:1 ratio with sends. that's high for a dispatch-bound
workload, low for a data-walking workload — and matches the
"natives traverse cons-chains" hypothesis.

**microbench miss reason:** bench-natives uses Integer `:+` which
is a literal `int1 + int2` — zero internal work. real natives do
data-structure traversals microbench never touches.

**tier-2 plan adequacy:** existing items §5.1 (PIC), §5.2 (inline
arith), §5.3 (threaded dispatch), §5.4 (flat env) all target the
*dispatch loop*. they leave R1 untouched. **§5.7 + §5.8 added
below** target it.

#### bottleneck R2 — form allocation churn (746K forms per ~40 s of parse)

**finding:** 746,071 Forms allocated, 388,986 of which are env-Forms
(52% of allocation budget is "calls that bind params"). GC swept
only ~750 of these total over 9 cycles (the rest survived because
the parser/compiler retains intermediate AST). **mean alloc rate:
19 K forms / second.**

each Form allocation costs:
- one `forms.append` (ArrayList growth, occasionally a realloc),
- three lazy `AutoArrayHashMapUnmanaged` slots (slots / handlers /
  meta — though only slots typically gets populated for env-Forms),
- one or more `slots.put` calls (envBind: 556K of these).

at smp_allocator's ~50 ns / put + ~5 ns / Form-init, the alloc-
fraction-of-wall is roughly:
- 746K Forms × 5 ns = 3.7 ms (init)
- 556K env_binds × 50 ns = 27.8 ms (puts)
- 388K env allocs × ~10 ns = 3.9 ms (env-specific extra)
- **total ~35 ms** — only **0.09 %** of wall time.

so the alloc cost itself isn't the time-killer — but the
**consequent memory pressure** is. 746K Forms × ~200 bytes / Form
(slots + handlers + meta + bookkeeping) = ~150 MB transient heap.
this pressures the allocator's caches and the L1/L2 hit rate of
every subsequent access. the 19K forms/sec rate also implies
constant heap-grow events for `forms: std.ArrayList(Form)`.

evidence: the 746K Forms accumulate net even after 9 GCs swept
~750. that means **the parser/compiler hangs on to >99.9% of
allocations** as live state (parsed AST + compiler IR). nothing
to sweep until the parsed file's chunk-Form gets evicted, which
only happens if no future code references it. for `lib/early/*.moof`
which **defines protos that the rest of bootstrap uses**, none of
that AST can be swept.

**microbench miss reason:** bench-parser-like and stress-recursion
both create 0-100K forms in their lifetime — small enough that the
allocator pool stays hot. real workload **shifts allocation patterns
into "ArrayList grows past every cache size"** territory.

**tier-2 plan adequacy:** §5.5 (Closure flat repr) helps for
closures, §4.5 (env-form pool) helps for envs. **but neither
addresses the parser/compiler AST-Form churn**. they're the bulk
of the 746K. **§5.9 added below** (Cons + AST pool) targets this.

#### bottleneck R3 — sym intern churn (267K intern calls; 410 per unique sym)

**finding:** `world.syms.intern` was called 267,878 times during
the run. only ~653 symbols are unique in the table. **the parser
is interning the same name hundreds of times each.**

mechanism: `(intern str)` in moof code (parsing `'foo`) walks
the String into UTF-8, then calls `syms.intern`. `syms.intern`
does `index.get(name)` (a StringHashMap lookup with hash + memcmp).
at ~150 ns per StringHashMap lookup × 267K calls = **40 ms** —
small fraction of 39 s but a smoking-gun pattern.

**worse:** several intrinsics intern static strings on every
invocation:
- `stringAt` → `lookupSymByName("bytes")` → linear scan, NOT
  intern. same issue (worst kind).
- `globalIntern` → `lookupSymByName("bytes")` → linear scan + ⌐
  `syms.intern` on the result name. so the parser's per-sym-read
  path costs `653 × memcmp + 1 hashmap-lookup`.
- `closureCallIn` / `methodSlot` / dozens of others — every native
  that needs a SymId calls `syms.intern("name")` and gets the same
  SymId every time.

the **400× redundancy ratio** (interns / unique syms) confirms:
every per-name native is re-interning. zero-cost in microbench (no
strings touched) but real cost in parser.

**microbench miss reason:** microbenches use pre-interned SymIds
(intern at chunk-build time, then dispatch on the SymId). real
parser converts source-strings to SymIds at parse-time, with
intermediate cons-chain traversals.

**tier-2 plan adequacy:** none of §5.1-§5.5 addresses this.
**§5.10 added below** caches sym-by-name in a static comptime
table on the World.

### 3.5.3 NOT-bottlenecks (microbench was right)

- **IC hit ratio = 81.5%** — better than feared. **but 88% of
  the misses are polymorphism thrash**, not cold or generation-
  stale. PICs (§5.1) directly target the 8,738 misses; sizing-
  by-measurement: 4-way PIC would catch ~7,400 of those
  (rough JS-engine literature ratio).
- **env walk depth: avg 1.37 hops, p99 ≤ 6 hops.** parser is NOT
  using deep nested closures — this kills the hypothesis that
  env-chain depth dominates. `bench-deep-env` measured this
  correctly: at 1-2 hops, env walks are cheap.
- **GC cost: 0.65% of wall time** (257 ms across 9 cycles).
  the "GC fires per evalStringInWorld sub-form" hypothesis is
  *true* but the cost is negligible — phase 1 mark-sweep on a
  ~700K-Form heap takes ~30 ms.
- **No DNU dispatches** across the entire bootstrap workload.
  the proto-chain lookup machinery is clean.
- **Tail-send count (322K) > bytecode dispatch (213K)** — the
  compiler's tail-call optimization is firing aggressively.
  more frames are *replaced* than *pushed*. good. **but tail-send
  has no IC** (per V4 spec §6.2); 322K uncached lookups is real
  waste. flagged for §5.11.

### 3.5.4 the 178× gap, decomposed

```
microbench send (bench-parser-like): 147 ns
   ├─ dispatch loop:            ~17 ns
   ├─ IC fast path:             ~5  ns
   ├─ prepareInvoke (native):   ~120 ns (mostly args copy)
   └─ Integer + Integer:        ~5  ns

real-workload send: 22,029 ns / send (NB. average over all sends,
  including native sends with heavy bodies)
   ├─ dispatch loop:            ~17 ns       (unchanged)
   ├─ IC overhead:              ~10 ns       (81% hit, 19% miss)
   ├─ prepareInvoke:            ~120 ns      (unchanged)
   ├─ native body (avg):        ~10,000 ns  ← R1: string walks +
   │                                            sym name scans +
   │                                            cons chain reads
   ├─ env alloc + bind (21%):   ~30 ns       (213K bytecode of
   │                                          1.79M total)
   ├─ form allocs (~0.42/send): ~10 ns       (cache pressure
   │                                          aside, alloc itself
   │                                          is cheap)
   └─ misc:                     ~50 ns
```

so **R1 (native bodies) is responsible for ~90% of the gap.**
R2 + R3 are real but small. **the dispatch loop is fast; the
intrinsics are not.**

this reorders the tier-2 priorities significantly. §5.1 / §5.2 /
§5.3 / §5.4 still ship — they raise the dispatch floor — but
**the biggest single-line speedup is fixing R1 first**.

## 4. tier 1 design (immediate, weeks → 100-1000× speedup target)

each tier-1 fix below has a measured or extrapolated speedup
estimate. all are *structural code changes* to
`crates/zig-substrate/`; none require new dependencies.

### 4.1 fast allocator by default (smp_allocator)

**mechanism:** flip `main.zig:60` so the default allocator is
`std.heap.smp_allocator` (single-thread fast path; multi-thread-safe
but not bookkeeping-heavy). gate `DebugAllocator` behind
`MOOF_DEBUG_ALLOC=1` for leak-hunting only.

**measured speedup:** 30-60× on Send-heavy code (bench-natives
132K → 8.1M sends/sec; stress-recursion 161K → 5.1M sends/sec).

**risk:** low. smp_allocator has been in zig std since 0.13. doesn't
do bounds-check pages, so a use-after-free would corrupt rather than
trap. but with phase-1 GC owning Form lifetimes and no exposed raw
pointers in the moof surface, the corruption risk is bounded to
substrate bugs.

**effort:** ≤ 1 hour. ship same day.

### 4.2 stack-resident args buffer (no per-Send alloc)

**mechanism:** allocate one `[]Value` scratch buffer per `Vm` (sized
generously — say 32 slots, expandable on overflow). natives take a
`[]const Value` slice into this buffer. on entry to `prepareInvoke`,
copy from the operand stack into the scratch buffer once, then
truncate the stack. **no heap alloc per Send.**

if a native re-enters the VM (option α per phase 1 §4.5), the
inner Send's `prepareInvoke` claims its own slice further into the
scratch buffer (bump-allocator style). on return, free back. the
scratch buffer is per-Vm so single-thread-safe by construction.

```zig
// world.zig: Vm gets a scratch buffer.
pub const Vm = struct {
    stack: std.ArrayList(Value),
    frames: std.ArrayList(Frame),
    args_scratch: std.ArrayList(Value),  // NEW
    args_scratch_top: usize = 0,         // NEW (bump pointer)
    last_send_sel: ?SymId,
    ...
};

// prepareInvoke: claim a region of args_scratch.
const args_start = world.vm.args_scratch_top;
try world.vm.args_scratch.ensureUnusedCapacity(world.allocator, argc);
world.vm.args_scratch.items.len = args_start + argc;
@memcpy(world.vm.args_scratch.items[args_start..][0..argc], call_args);
world.vm.args_scratch_top += argc;
defer world.vm.args_scratch_top = args_start;
const args_buf = world.vm.args_scratch.items[args_start..][0..argc];
world.vm.stack.shrinkRetainingCapacity(shrink_to);
const result = try native(world, self_v, args_buf);
```

**expected speedup:** measured native-Send native dispatch cost
drops from ~124 ns → ~50 ns at smp_allocator (allocator overhead
goes to ~0), and from ~7500 ns → ~150 ns at DebugAllocator (60×
on debug builds; ~2.5× on release).

**risk:** low-medium. natives mustn't hold onto the args slice
across re-entrant `world.send`. that contract already holds in
practice (natives copy values out before sending), but it's now
*load-bearing*. document it; add a debug-mode "args_scratch_top
must equal args_start" assertion at native return.

**effort:** 1-2 days including audit of all 21+ natives in
`intrinsics.zig`.

### 4.3 chunk side-table caching on the Frame

**mechanism:** when a Frame is pushed, also cache its `bytecode_bytes:
[]const u8`, `consts: []const Value`, `ics: []ICache`, and `params_slice:
[]const u32` directly on the Frame struct. eliminates per-step
`chunk_bytecode.get(chunk)` hashmap lookups and per-`LoadConst`
`chunk_consts.get(chunk)` lookups.

```zig
pub const Frame = struct {
    chunk: FormId,
    pc: usize,
    env: FormId,
    self_: Value,
    stack_base: u32,
    defining_proto: FormId,
    // NEW — cached side-table slices (read-only borrow of side-table)
    bytecode: []const u8,
    consts: []const Value,
    ics: []ICache,
    params_slice: []const u32,
};
```

**expected speedup:** removes ~3 hashmap lookups per op (~5-10 ns
each at smp_allocator). on bench-loop floor of 17.7 ns/op, this
could cut it to ~10 ns/op (40% off). cumulative across a 1.2M-op
workload: ~6-12 ms. moderate win.

**risk:** the cached slices must invalidate if the side-tables grow
(causing ArrayList realloc). but in practice, side-tables are
populated at compile time and not extended during execution. the
phase-1 GC swaps entries out of the side-tables on sweep — if a
chunk in the frame stack is collected (it shouldn't be, since
frame.chunk is a GC root per phase 1 §3.2), the slices would
dangle. assertion: frame.chunk in `gcMark` ⇒ side-tables stay live.

**effort:** 1 day.

### 4.4 closure :params lazy / use chunk_params direct

**mechanism:** today, `world.allocClosure` (called by `PushClosure`)
builds a cons-list of params from `chunk_params[chunk]` and stores
it as the `:params` slot on the closure. `prepareInvoke` then walks
that cons-list back into a slice via `listToSlice`. **eliminate the
round-trip:**

- on `PushClosure`: do NOT populate `:params`. (reflection that
  needs `:params` reconstructs it lazily from chunk_params.)
- in `prepareInvoke`: bind params by reading `world.chunk_params.get(
  chunk_id)` directly — a single hashmap lookup, no walk, no
  allocation.

```zig
// prepareInvoke (post-change):
const params_syms = world.chunk_params.get(chunk_id) orelse &[_]u32{};
if (params_syms.len != call_args.len) return error.Arity;
const call_env = try world.allocEnv(captured_env);
for (params_syms, call_args) |param_sym, arg_v| {
    try world.envBind(call_env, param_sym, arg_v);
}
// no listToSlice; no freeSlice; no defer; no cons walk.
```

**measured potential:** listToSlice in stress-recursion 100k did
100k calls walking 100k items (no allocs since the params list is
length 1). 1 ns per item walk + 1 alloc per call ≈ 50-100 ns / call.
**eliminating this is ~30-50 ns / Send (15-25% of bytecode-Send
cost).**

**risk:** anything reading `[closure :params]` for reflection now
sees nil. fix: reflection intrinsic reads chunk_params and builds
the list on demand. acceptable.

**method-Form has `:params` already populated by image-load (per
v4_export.rs).** in the `[method :params]` reflection path, the
list-walk is still needed. **only PushClosure-allocated closures
skip the cons.** that's the hot path; the reflection path is cold.

**effort:** 0.5 day.

### 4.5 frame pool (free-list)

**mechanism:** maintain a free-list of `Frame` structs on the Vm.
when a frame is popped (`Return`), push it onto the free list. on
next `prepareInvoke` bytecode path, reuse a free Frame instead of
appending. the `ArrayList(Frame)` becomes a stack-discipline list of
live frames; the free-list is parallel.

actually simpler: since `frames` is an ArrayList, the existing
`shrinkRetainingCapacity` already does this — frames after the live
top remain allocated. we don't actually need a separate free list.
**verify:** confirm `frames.append` after a `frames.pop` reuses the
same slot. zig's ArrayList does, so this is already correct.

what's NOT free-listed: the **per-frame env-Form** is still allocated
on every call. that's the real allocation cost (~50 ns at smp,
~5000 ns at debug).

**better mechanism: env-Form pool.** maintain a free-list of empty
env-Forms on the Vm. on `allocEnv(parent)`, take from the pool,
reset `slots` and set `meta.parent = parent`. on Return, push the
just-popped frame's env-Form back onto the pool **but ONLY if no
moof-side reference escapes** (e.g. via a `PushClosure` capturing
this env, or `Object:eval:` view-target binding, or a let-form
holding onto the env as a Value).

the "escape" tracking is non-trivial. one option: maintain a "may
escape" flag on the Frame, set by `PushClosure` / `Object:eval:`,
and only recycle if false. but this is fragile.

**alternative: post-GC compaction frees them anyway.** since phase
1 GC tombstones unreachable Forms, env-Forms that don't escape *are
already* swept on the next runTop boundary. the speedup is then in
*alloc cost*, not lifetime. allocEnv allocates a Form (which is a
struct of pointers to maps; the maps are lazy-init). making the
form-alloc itself cheap is the win.

**proposed concrete fix: Form payload pool.**

- the Form struct holds 3 `AutoArrayHashMapUnmanaged` (slots,
  handlers, meta) — pure-pointer structs. allocating is just
  appending to `Heap.forms`.
- the cost isn't *alloc Form* (~5 ns); it's that `envBind`'s
  subsequent `slots.put` may realloc the hashmap's backing storage.

per-Vm hashmap pool: when an env-Form is freed (via GC), instead of
deallocing its inner maps' storage, return the storage to a pool
keyed by capacity. on next `envBind` that triggers a put, prefer
pool storage over fresh alloc. saves the dominant cost.

**implementation:** this is more involved. defer to tier 1.5 if
allocator switch (4.1) hasn't fully closed the gap.

**expected speedup combined with 4.1, 4.2, 4.4:** stress-recursion
should drop from 197 ns/send → ~80 ns/send (4× on bytecode path).

**risk:** medium — pool correctness is easy to get wrong.

**effort:** 1-2 days if needed.

### 4.6 IC hit-rate audit + telemetry

**mechanism:** ship the `Profile` counters from the perf-investigation
branch as an optional `MOOF_PROFILE=1` mode. at every `runTop` exit,
dump counters to stderr (or a JSON file). collect IC hit rate over
parser/compiler real workloads.

current profile data shows ICs hit 100% on monomorphic micros.
parser-real-world is presumably worse. *measure* before optimizing.

if hit rate < 90 %, ship PICs (§5.1).

**risk:** zero; observability only.

**effort:** 0.5 day to refactor the profile patch out of the bench
codepath.

### 4.7 hash-table swap: ArrayHashMap → AutoHashMap for handlers + native_fns

**mechanism:** `Form.handlers` is currently `AutoArrayHashMapUnmanaged
(u32, Value)` — preserves insertion order. **D5 only requires this for
the iteration-visible substrate-internal tables** (slot iteration,
meta iteration). `handlers` is rarely iterated; it's lookup-heavy.

swap to `AutoHashMapUnmanaged(u32, Value)`:

- `handlers.get(selector)` becomes a regular open-addressing hash
  lookup; no insertion-order overhead. expected 1.5-3× faster get.
- a related table: `world.native_fns: AutoArrayHashMapUnmanaged
  (FormId, NativeFn)`. swap to AutoHashMap. **but** D5 says image
  serialization needs deterministic iteration. fix: when serializing,
  iterate sorted by FormId payload. not insertion-order, but
  deterministic.

**risk:** **medium**. D5 audit needed. `slots`, `meta`, and the
side-tables (chunk_bytecode/consts/ics/params) likely must stay
ArrayHashMap because:
- slots/meta: reflection iterates them; users see insertion order;
- side-tables: image-load (image.zig) iterates them in load order
  for byte-deterministic re-serialize.

`handlers` is the *least* affected — users rarely iterate handlers
(it's a method table, not a struct-like slot map). PROPOSE switching
ONLY `handlers` and `native_fns`, leaving the rest. measure carefully.

**expected speedup:** small but measurable (~5-10 ns / Send via the
faster `handler.get` on slow-path; cumulative with PICs).

**effort:** 1 day including the determinism audit.

### 4.8 const-fold + peephole in compiler.moof (out of zig)

**mechanism:** `[1 is nil]` should compile to a single `PushFalse`,
not a Send. check whether the compiler.moof peephole already does
this. if not, audit `lib/compiler/02-special.moof` and add the
fold.

**measured potential:** if a 200-send parser/compiler trace contains
50 fold-able sends (`1 + 2`, `is nil`, etc.) at ~100 ns / Send,
that's 5 µs. small in raw terms but lands "compile-time work cuts
runtime work" as a permanent invariant.

**risk:** zero (compile-time only; runtime semantics unchanged).

**effort:** 0.5-1 day.

### 4.9 the immediate "wart hunt"

before any structural fix, **search for hot-path `std.debug.print`
calls**:

- `vm.zig:264` — `print("UnboundName: …")` in LoadName's miss path.
  fires every time a name isn't bound. **but unbound names are
  errors** — if this fires routinely in the parser, the parser is
  doing something wrong. either way, the print is hot. wrap in
  `if (world.gc_stats_enabled) …` or gate behind a debug flag.
- `vm.zig:640-642` — `print("UnhandledDnu: …")` similar story.
- `vm.zig:703-716` — arity mismatch in `prepareInvoke`. fires only
  on bug, not hot path. but it iterates method slots in the
  diagnostic print — if the slot count is in the hundreds, that's
  a slow print. probably fine to leave.
- `intrinsics.zig:1394` — `print("transporterLoad: …")` on every
  file load. cold; leave.
- `intrinsics.zig:1360` — `print("evalStringInWorld: parsing…")`
  every eval. lukewarm; could gate.

**effort:** 0.5 hour.

### 4.10 expected tier-1 cumulative speedup

stacking 4.1 (smp_allocator), 4.2 (no args alloc), 4.3 (frame
caching), 4.4 (no params cons), 4.7 (handlers AutoHashMap), 4.9
(wart hunt):

- baseline (DebugAllocator): 132 K sends/sec (bench-natives)
- 4.1 alone: 8.1 M sends/sec (60×)
- + 4.2: ~15 M sends/sec (1.5-2× on top — args copy is gone)
- + 4.3, 4.4, 4.7: ~20 M sends/sec (1.3× — sundry overheads gone)
- 4.9 catches any debug-print regression

**realistic landed target:** 5-10 M sends/sec sustained on
realistic mixed workloads. interpolating to the `[1 is nil]` case:
from 90 s @ 5.5 sends/sec → **100-200 ms** at 5-10 M sends/sec
**if** the per-Send work doesn't compound (i.e. if the 200-send
estimate is accurate). likely needs PIC (§5.1) for the deeply
polymorphic parser to hit the lower bound.

**E1 should be reachable with tier-1 alone**, modulo PICs as a
safety net.

## 5. tier 2 design (months, 5-10× on top)

once tier 1 is in, the floor is dispatch cost (~20-30 ns/op) +
per-op work (env walk, hashmap lookups). tier 2 targets the floor.

### 5.1 polymorphic inline caches (PICs)

**mechanism:** extend the IC slot from monomorphic to
**N-way polymorphic** (default N=4). cached entry becomes a small
array of `(cached_proto, cached_method, cached_defining, cached_singleton)`
quadruples, plus an LRU counter or first-fit-on-miss policy.

```zig
pub const PICache = struct {
    entries: [4]ICEntry,
    n_entries: u8,  // 0..4
    last_generation: u32,
};
```

on dispatch:
```
for entry in entries[0..n_entries]:
    if entry.proto == receiver_proto and entry.generation == ...:
        return entry.method  // hit
// miss → resolve + insert at first empty slot, or evict LRU
```

**measured potential:** in the polymorphic bench (alternating
Bool/Nil), monomorphic IC hits 0%. PIC-4 would hit 100% on
2-way alternation. per-Send cost stays at ~50 ns (still fast path),
vs slow-path ~127 ns. **2.5× speedup on polymorphic-heavy code**.

real-world hit rates from JS engines suggest 2-way PIC catches
~80% of polymorphic sites and 4-way catches ~95%. moof's stdlib has
a similar shape (most call sites are 1-3 receiver types).

**risk:** low-medium. PIC entry layout grows the IC slot from
24 bytes to ~96 bytes; chunk side-tables grow proportionally.
trivial.

**effort:** 2-3 days. straightforward extension of phase 1's IC.

### 5.2 inline arithmetic + comparison

**mechanism:** at the dispatch site for `Send` (and fused variants),
check the receiver-selector pair against a small hardcoded fast-set:

```zig
// before resolving via IC: fast-path Integer x Integer arithmetic.
if (receiver == .int and args.argc == 1) {
    const rhs = stack[top];
    if (rhs == .int) {
        switch (args.selector) {
            world.plus_sym  => { push(int(a +% b)); return; },
            world.minus_sym => { push(int(a -% b)); return; },
            world.eq_sym    => { push(bool(a == b)); return; },
            world.lt_sym    => { push(bool(a < b)); return; },
            world.gt_sym    => { push(bool(a > b)); return; },
            else => {},
        }
    }
}
```

caveat: this **bypasses** the proto chain — if the user installed a
custom `:+` on the Integer proto via `setHandler!`, the inline fast
path silently ignores it. **L3 violation.** mitigation: only take
the fast path if `proto_generation[integer_proto] == 0` (no user
override has happened). when an override fires, the generation
bumps, the IC invalidates, and the fast path stops firing.

**measured potential:** intArithmetic is the most-used native (the
lexer's `:<` and `:>` are called per char per delimiter check). at
~120 ns/Send for native dispatch, eliminating the dispatch overhead
in favor of a direct switch could drop these to ~20 ns/op. for
parser-heavy workloads where ~50% of sends are int-int ops, **2-4×
speedup**.

**risk:** medium. the override-via-set-handler escape hatch must be
preserved per L3. proto-generation gating handles this but needs
test coverage.

**effort:** 2 days including override-detection tests.

### 5.3 tail-call threaded dispatch

**mechanism:** zig 0.16's `@call(.always_tail, ...)` compiles to a
direct jump. dispatch becomes:

```zig
const dispatch_table = [256]fn(*World) anyerror!void {
    [0x01] = op_push_nil,
    [0x02] = op_push_true,
    ...
};

fn op_push_nil(world: *World) anyerror!void {
    try world.vm.stack.append(world.allocator, .nil);
    return @call(.always_tail, decodeAndDispatchNext, .{world});
}

fn decodeAndDispatchNext(world: *World) anyerror!void {
    const frame = top_frame(world);
    if (frame_done(world)) return;  // outer-loop break
    const tag = frame.bytecode[frame.pc];
    frame.pc += 1;
    return @call(.always_tail, dispatch_table[tag], .{world});
}
```

**expected speedup:** classic luajit / wasm3 design buys 2-3× over a
switch-based interpreter. our 17 ns/op floor would drop to ~7-10
ns/op.

**risk:** medium. tail-call ABI compatibility is sensitive to
optimizer behavior; small changes can defeat the tail call. needs
disasm-level verification.

**effort:** 3-4 days including verification and re-validating the
op-by-op semantics.

### 5.4 flat env representation

**mechanism:** today an env-Form is a Form with a `meta.parent`
link and `slots: AutoArrayHashMapUnmanaged`. lookups iterate slots
(O(1) per frame, hashmap overhead) then walk the parent chain.

flat representation: each frame's locals are a fixed `[]Value`
indexed by **a compile-time-assigned local-slot number**. the
compiler tracks which let-bindings live where; LoadName at parse
time resolves to either `LoadLocal{n: u8}` (fast) or `LoadName{sym}`
(slow, for module-level / dynamic).

**measured potential:** LoadName at depth=1 costs ~63 ns; depth=10
costs ~125 ns. with a per-frame `[]Value`, LoadLocal is one array
read — ~3-5 ns. **20-25× speedup on local var loads**.

**risk:** high. requires a new opcode (LoadLocal / StoreLocal),
compiler changes, and a hybrid env model (still need
hashmap-style binding for `(def name val)` at module level).

**effort:** 5-7 days. compiler-side work.

### 5.5 closure flat representation

**mechanism:** today a Closure is a Form with `:body`, `:env`,
`:captured-self`, `:params` slots. that's 4 hashmap entries per
closure for fields that are always present and always read.

flat: a `Closure` struct with named fields (chunk_id, env_id,
captured_self, params_slice). reflection still exposes them as
slots via a synthetic-slot reader (the reflection contract per
R7 doesn't require flat-form storage).

**expected speedup:** PushClosure allocation drops from 4 hashmap
puts + Form alloc → 1 fixed-size struct alloc. ~2-3× on closure
creation.

**risk:** low — Closure-Form is read-only after construction in
99% of cases.

**effort:** 1-2 days.

### 5.7 native intrinsic body audit + cached SymIds (R1, R3)

**mechanism:** the §3.5 profile shows that *most native dispatches
spend their time inside the native body, not in dispatch*. two
recurring sins:

1. **`lookupSymByName(world, "bytes")`** in nine intrinsics — a
   linear scan of the 653-sym table per call (~4 µs of pure CPU).
   includes `stringWalk` (used by `:at:` / `:length`), `stringEq`,
   `stringSlice`, `globalIntern`, etc.
2. **`syms.intern("captured-self")` / `syms.intern("op")` / ...**
   in dispatch-helper natives — pays one StringHashMap lookup per
   call where the result is constant.

both are fixable trivially: **cache hot SymIds on the World**
at boot time, in the same shape as the existing `body_sym`,
`env_sym`, `params_sym`, `symCar`, `symCdr` fields. add:

```zig
// world.zig — cached SymIds populated at init/initBare boot.
bytes_sym:           SymId,   // for String:bytes traversals
captured_self_sym:   SymId,   // for closure dispatch
op_sym:              SymId,   // for Opcode:op
operands_sym:        SymId,   // for Opcode:operands
// ... plus the ~6 other hot ones identified in the audit
```

then every `lookupSymByName(world, "bytes")` becomes a direct
`world.bytes_sym` read — **5 ns vs ~4 µs (800× speedup on that
call site)**.

**measured potential:** 1.27 M `formSlot` calls in the bootstrap.
many go through `stringWalk` → `lookupSymByName`. extrapolating:
if 200K of those 1.27M paid the linear-scan tax at 4 µs each, that's
**800 ms = 2 % of wall time**. small in *percent*, large in
absolute, and **trivial to fix**.

paired with this: **eliminate `lookupSymByName` from the codebase**.
the helper exists "to avoid the implicit intern that `syms.intern`
would do" (per its docstring) — but at the cost of a 653× linear
scan. swap to caching the SymId at boot.

**risk:** zero. cached SymIds are already a pattern in the codebase
(`body_sym`, `params_sym`, etc.). symbol identity is stable
across the World's lifetime (L11).

**effort:** 0.5-1 day. mostly mechanical — find every
`lookupSymByName` + `intern("static")` call, add a cached field,
swap.

**ordering:** ship FIRST in tier 2. zero-risk, expected to recover
2-5 % of wall time outright, and clears the way for the more
invasive fixes below.

### 5.8 hot-native rewrite: stringWalk, Cons traversal, intern (R1)

**mechanism:** with cached SymIds in hand (§5.7), audit the
remaining body cost of the top-3 hot natives:

1. **`String:at:`** — currently O(N) walks the :bytes cons-chain
   per call. parser uses this to read chars; for a 1000-char file
   that's 500K cons-cell hops to read every position. **fix:** for
   String-Forms specifically, materialize the chars as a `[]u32`
   slice cached in a meta slot on first :at: call. subsequent calls
   are O(1) indexed lookup. **but** that violates L1 ("everything
   is a Form") — Strings are Forms-of-conses by canonical V0
   shape. mitigation: cache lazily in `world.string_cache`
   (a FormId → []u32 map); invalidate on `:setCharAt:` /
   `:append:` etc. (these aren't on the hot path; cache hit rate
   is near-100% for read-only parser input).
2. **`Cons:car` / `Cons:cdr`** — slot lookups. fast (~5 ns) but
   1.27 M of them. if Cons becomes a **flat struct** (parallel to
   §5.5 Closure flat repr), `car` and `cdr` become direct field
   reads; **~10× speedup** (5 ns → 0.5 ns). reflection still works
   via a synthetic-slot reader.
3. **`(intern str)`** — walks String :bytes into UTF-8 buffer +
   `syms.intern`. fix: **a String→SymId cache** on World. once a
   String identifier has been interned, the FormId → SymId mapping
   is stable (L11). cache hit rate near-100% for repeated parses
   of the same identifier text. cuts `(intern "foo")` from O(n) +
   memcmp + alloc to O(1) FormId lookup.

**measured potential:** the 22,029 ns/send average is the
weighted-mean cost across a workload where (1.25M / 1.79M = 70 %)
are native dispatches. each native body averages ~22-30 µs of
in-body work (excluding dispatch). cutting that to ~3 µs via
the cache wins **~7×** on the native-dispatch slice = **5×
overall**.

**risk:** medium. caches need invalidation discipline; misses on
"setCharAt:" / "append:" mutations need to evict. moof-side
reflection (`[s :bytes]`) must continue to work — easy if cache
is hidden behind a substrate-internal field, not a Form slot.

**effort:** 2-3 days, plus a regression-test pass against the
String/Cons stdlib tests.

### 5.9 cons-pool + AST-Form sharing (R2)

**mechanism:** the bootstrap allocates 746K Forms in 40 s, almost
none of which the GC can sweep (parser/compiler AST is live). of
those, ~389K are env-Forms (already targeted by §4.5's env-Form
pool, but that pool only helps short-lived ones — env-Forms
captured by closures escape and stay live).

the **remaining 357K** are predominantly cons cells (parser builds
list-of-lists AST) and Form-Forms (intermediate compiler IR).
neither can be addressed by pooling — they're net retained.

two complementary mitigations:

1. **cons-pool with eager init:** pre-allocate cons cells in batches
   of 4096 at boot. on heap-grow, fewer ArrayList reallocs (each
   realloc bursts cache cost across the whole heap). doesn't
   reduce *count*, but reduces *cache-miss frequency* on alloc.
2. **compiler IR sharing via interning:** common AST shapes
   (e.g. `(+ x y)` for various x, y) generate identical-shape
   sub-trees. a content-addressed cache for cons-cells (keyed
   by (car-FormId, cdr-FormId)) deduplicates. **but** L1
   (everything is a Form) requires distinct FormIds for
   user-visible identity; this only applies to compiler-internal
   shapes. mitigation: gate behind a substrate `dedupe-cons`
   flag.

**measured potential:** alloc cost itself is ~35 ms in the 40 s
workload (per §3.5.2 R2 calc). pre-allocation buys most of that
back. cache pressure from 150 MB transient heap is the bigger
deal; harder to measure pre-fix, target post-fix is a 30 % drop in
peak L2 misses (proxy: `perf stat -e LLC-load-misses` before /
after).

**risk:** medium. dedup-cons changes the heap's L11 invariant for
deduplicated cells — careful audit needed. without dedup,
pre-allocation alone is low-risk.

**effort:** 2 days for pre-alloc; 4-5 days for dedup-cons w/ tests.

### 5.10 sym-intern cache for parser hot path (R3)

**mechanism:** 267K intern calls for 653 unique syms = 410×
redundancy. the parser parses every identifier (e.g. `'foo`) via
`(intern (string-of-chars …))` — a String → SymId call that
hashes the String's bytes and looks it up in `syms.index`.

simpler fix: **memoize at the parser level** via a substrate-
exposed `[$reader internCached:]` cap, backed by a `World`-side
`FormId(string) → SymId` map. once the parser has interned `'foo`
once, future `(intern foo-str)` calls hit the map in ~3 ns vs
~150 ns.

**measured potential:** 267K × ~150 ns = ~40 ms saved. small in
absolute terms. cumulative with §5.8's hot-natives fix.

**risk:** zero. the cache is one-way (string FormId is immutable
post-parse); evictions are unnecessary.

**effort:** 0.5 day.

### 5.11 IC for tail-sends (R4)

**mechanism:** tail-sends (`TailSend`, `TailSendSelf`, `TailSendHere`)
currently have **no IC**. per V4 spec §6.2, the operand layout
doesn't include `ic_idx`. the profile measured 322K tail-sends in
the bootstrap, each a full `lookupHandler` walk.

extend the V4 byte encoding: tail-send variants gain a 2-byte
`ic_idx` operand, mirroring the regular Send forms. the IC slot
type is unchanged; the dispatch path is the same as regular Send
minus the frame-push (frame-replace stays).

**measured potential:** if tail-send hit rates parallel
regular Send (81 %), 322K × 0.81 × ~120 ns saved (slow→fast) ≈
31 ms. cumulative.

**risk:** medium — wire-format change. need a V4 spec amendment +
ocaml-seed compiler update + image-format migration. but
backward-compatible: chunks compiled before the change have no
ic_idx; substrate falls back to the no-IC path.

**effort:** 2-3 days including ocaml-seed sync.

### 5.6 expected tier-2 cumulative

stacking tier-2 on top of tier-1: maybe 3-5× more. lands us at
~30-50 M sends/sec on micro-benches; ~5-10 M sustained on real
workloads. **BEAM-interpreted parity**.

**revised tier-2 cumulative incorporating §5.7-§5.11:**

| item                            | gain (real workload) | risk    |
|---------------------------------|----------------------|---------|
| §5.7 cached SymIds              | 2-5 %                | low     |
| §5.8 hot-natives + caches       | **5×**               | medium  |
| §5.9 cons-pool + IR sharing     | 1.3×                 | medium  |
| §5.10 sym-intern cache          | 0.1 %                | low     |
| §5.11 tail-send IC              | 1.1×                 | medium  |
| §5.1 4-way PIC                  | 1.3×                 | low-med |
| §5.2 inline Int+Int             | 1.5×                 | medium  |
| §5.3 threaded dispatch          | 2×                   | medium  |
| §5.4 flat env                   | 1.5×                 | high    |
| §5.5 flat closure               | 1.2×                 | low     |

**stacked (not multiplicative — overlapping fixes):** ~10-15×
on real workloads from current baseline. real `[1 is nil]` at
**~150-300 ms** (E1 target ≤ 100 ms reachable, requires
§5.7/§5.8 first).

## 6. tier 3 design (months → year, BEAMJIT-rivaling)

tier 3 is the big ambition. three viable approaches; each is a
distinct project.

### 6.1 copy-and-patch compilation

**concept:** for each opcode (or fused multi-op block), pre-compile
a "stencil" — a small chunk of native machine code with placeholder
slots. at chunk-compile time, copy stencils end-to-end into a fresh
executable buffer and patch in the literal values (immediates,
ic-slot pointers, etc.).

origin: Truffle (Oracle), then [Copy-and-Patch Compilation by
Xu & Kjolstad (PLDI '21)](https://fredrikbk.com/publications/copy-and-patch.pdf).
JuliaPy uses a variant. WebKit ships a copy-and-patch tier of JSC.

**pros:**
- near-native dispatch speed (5-10× over interpreted).
- per-stencil compile time is sub-microsecond.
- no need for full machine-code generation infrastructure (no
  LLVM / cranelift / etc.).
- portable across architectures via per-arch stencil libraries.

**cons:**
- stencils are tied to specific opcodes and value layouts.
  every opcode + immediate-shape combination needs a stencil.
- generating stencils requires a working toolchain (zig compiler
  output, or hand-tuned asm).
- code-cache eviction (when methods are redefined) requires
  careful state tracking.

**effort:** 4-6 weeks for a minimal viable tier (stencils for
the 10 hottest opcodes; fallback to interpreter for the rest).

**speedup target:** 10-50× on hot code. approaches BEAM-JIT
performance for ~80% of the workload.

### 6.2 tracing JIT (PyPy-style)

**concept:** detect hot loops in moof code (loop trace, recursive
trace). compile the trace as a flat sequence of guarded operations
(if the receiver's proto isn't Integer at this point, deopt). emit
specialized machine code. fall back to interpreter on guard failure.

**pros:**
- handles polymorphic code well (specializes per-trace).
- self-optimizing — workload determines what gets compiled.
- excellent on numeric-heavy and tight-loop code.

**cons:**
- enormous engineering complexity. PyPy is 20+ years of work.
- compilation pauses are a UX risk (mitigated by lazy compile).
- deopt machinery is the hard part — managing speculation-failure
  fallback is most of the bug surface.

**effort:** 6+ months for a minimal tracing tier.

**speedup target:** 30-100× on hot code.

### 6.3 cranelift / LLVM backend

**concept:** use cranelift or LLVM as an IR-to-machine-code engine.
emit chunk bytecode as cranelift IR; let cranelift do the heavy
lifting.

**pros:**
- near-native speed (cranelift competes with LLVM on simple code).
- well-supported, well-documented backends.
- inheritances large body of optimization work.

**cons:**
- ~10-30 MB binary footprint (cranelift), 100+ MB (LLVM).
- compile times are real (seconds for cranelift on a moderate chunk;
  minutes for LLVM).
- not portable to wasm (we have wasm in our future via mco).

**effort:** 3-4 months for cranelift integration; LLVM is 2× that.

**speedup target:** 50-100×.

### 6.4 recommendation: copy-and-patch first

based on the moof philosophy (small substrate seed; minimal
dependencies; portable across mco/wasm/native):

- **copy-and-patch** wins on simplicity + portability. each
  stencil is "the bytecode handler with the immediate inlined" —
  a moof-aware engineer can write each by hand or generate via a
  zig comptime macro. no dep on LLVM/cranelift/wasmtime.

- **tracing JIT** is too expensive in engineering effort given our
  team size. revisit in 2-3 years if needed.

- **cranelift** is plausible if we accept the dep size. defer until
  we've maxed out tier 1+2 and need the last 5×.

**proposed sequence:**
1. tier 1 ships first (this spec).
2. tier 2 ships next quarter.
3. tier 3 evaluation: profile real moof workloads after tier 2;
   only commit to a JIT if the floor isn't acceptable.

## 7. BEAM comparison

erlang's BEAM gets to ~1-10 M reductions/sec interpreted, ~100M+
with BEAMJIT (since OTP 24).

**what BEAM does that moof currently doesn't:**

1. **register-machine bytecode**, not stack-machine. eliminates
   stack-push/pop per intermediate. moof is stack-machine — sticking
   with it makes reflection easier (the operand stack is observable),
   but at a 10-15% perf cost.
2. **process-local heaps + lightweight scheduler**. moof is already
   single-vat single-thread; phase D will add a scheduler. BEAM's
   advantage here is parallelism, not per-process throughput.
3. **threaded interpreter dispatch** (BEAM uses computed-goto via
   labels-as-values in C). tier 2.3 puts us in the same league.
4. **tagged-pointer values**, no separate boxing. moof has tagged
   immediates per V0 — we're parity.
5. **BIFs (built-in functions)** that bypass dispatch entirely for
   hot operations. tier 2.2 (inline arithmetic) does this.
6. **BEAMJIT (asmjit-based copy-and-patch)** since OTP 24. tier 3
   ships the moof equivalent.

**summary:** with tier 1+2 we should reach BEAM-interpreted parity
(~1-10 M reductions/sec). with tier 3 (copy-and-patch), we should
reach BEAMJIT parity. there's no architectural reason moof can't
go there — it's all engineering effort.

## 8. sequencing — what ships first, what gates on what

```
tier 0 (today; ≤ 1 day):
    4.1 smp_allocator default
    4.9 wart hunt (debug-print removal)
    ──── unblocks E1 to ~10ms / sends, alone

tier 1A (week 1; ≤ 3 days):
    4.2 stack-resident args buffer
    4.4 closure :params lazy
    4.3 chunk side-tables on Frame
    ──── unblocks E1 (sub-100ms for [1 is nil])

tier 1B (week 2; ≤ 2 days):
    4.7 hash-table swap (handlers + native_fns)
    4.6 IC hit-rate telemetry
    ──── measures readiness for PICs

tier 1.5 (week 2-3; if needed):
    4.5 env-form pool
    ──── only if stress-recursion still slow

tier 2-pre (week 3; ≤ 2 days, REVISED 2026-05-16 per §3.5):
    5.7 cached SymIds in World        — kills R3 + parts of R1
    5.10 sym-intern cache             — small but trivial
    ──── unblocks R1/R3; expected 2-5 % wall-time recovery; near-zero risk

tier 2A (month 1, REVISED — R1 first):
    5.8 hot-natives + caches          — biggest single win (~5×)
    5.1 PICs                          — depends on 4.6 telemetry
    5.5 closure flat repr             — independent

tier 2B (month 2):
    5.2 inline arithmetic             — depends on L3 generation gate
    5.9 cons-pool + IR sharing        — independent; alloc churn fix
    5.11 tail-send IC (V4 amendment)  — wire-format change

tier 2C (month 3):
    5.3 tail-call threaded dispatch   — depends on 5.1 / 5.2 stable
    5.4 flat env (compiler change)    — separate compiler track

tier 3 (quarter 2+):
    profile → choose between 6.1/6.2/6.3
    proposed: 6.1 copy-and-patch
```

each tier-1 step is independently shippable. each tier-2 step
depends on tier 1 to have shaken out hot-path bugs (PICs on top of
unstable monomorphic ICs would mask each other's regressions).

**revised ordering rationale (post §3.5 measurement):** the original
sequencing put PICs / inline arith / threaded dispatch first because
the assumption was "the dispatch loop is the bottleneck." the
real-workload profile (§3.5) showed the dispatch loop is fast and
**the natives themselves are the bottleneck**. tier 2-pre + 2A now
ship the §5.7 / §5.8 work first — they're cheaper and bigger wins.
the dispatch-floor work (PICs / inline arith / threaded dispatch)
stays on the roadmap but **after** the natives audit; otherwise the
PIC improvements get drowned out by the native body cost they don't
address.

## 9. risks + open questions

### 9.1 risks

1. **L3 violation via inline arithmetic** (§5.2). proto-generation
   gating is correct in principle, but needs careful test coverage.
   mitigation: ship inline arith last in tier 2; run the full
   `lib/stdlib/integer.moof` test suite against it.
2. **D5 regression from hash-table swap** (§4.7). if anything outside
   `handlers`/`native_fns` mistakenly switches to AutoHashMap, replay
   determinism breaks. mitigation: keep ArrayHashMap for everything
   reflection-visible; audit-document the choice.
3. **DebugAllocator users (CI / leak hunting)**. shipping
   smp_allocator as default means `cargo run`-style smoke tests
   lose leak detection. mitigation: keep `MOOF_DEBUG_ALLOC=1` flag;
   document in NEXT_SESSION.md.
4. **tail-call threaded dispatch may not stabilize across zig
   versions**. zig's `@call(.always_tail)` ABI is still evolving.
   mitigation: hold tier 2.3 until tier 2.1 + 2.2 land cleanly;
   pin zig version.
5. **stencil compilation (tier 3) requires careful ABI design**.
   subtle bugs (incorrect immediate sign-extension, etc.) silently
   miscompile. mitigation: tier 3 is a separate project with its
   own test suite; don't ship into the substrate until 100% smoke
   coverage.

### 9.2 open questions

1. **how much does PIC actually help in real parser workloads?**
   need profile data from the real parser (post tier-1) to
   answer. tier 1 ships the telemetry (4.6).
2. **does the closure-flat-repr break any reflection contracts?**
   need to grep `[closure :params]` callers in `lib/` to verify.
3. **what's the right N for PIC?** literature suggests 4 is plenty
   for JS; moof's stdlib may want different. measure first.
4. **should we ship a stack-allocated frame for `runUntilFrameReturns`
   sub-loops?** native re-entry pushes 1 frame; we could allocate
   it on the host stack. minor speedup, minor risk.
5. **what's the GC interaction with the args_scratch buffer (§4.2)?**
   if a native captures an argument into a heap form, the value
   was Borrowed from the operand stack but is now lifetime-bound
   to the heap form. nothing changes — Values are POD. but if the
   args_scratch is later overwritten, the captured *FormId* still
   resolves correctly (form is on heap). flagged: no actual problem,
   but document.
6. **should image-load eagerly resolve native by FormId?** today
   `world.native_fns: AutoArrayHashMap(FormId, NativeFn)` is queried
   per dispatch. caching the native-fn-pointer on the IC slot (when
   the method has one) eliminates this. small win; flag for tier 1.5.

## 10. future work (post BEAM parity)

once tier 1+2+3 are landed, the next steps are research-grade:

1. **per-vat bytecode dedup / content-addressed chunks**. when a
   chunk's hash matches one already in the shared segment, point
   to the shared copy. saves memory; enables cross-vat code
   sharing. depends on V4 §10.9 (canonical state hash).

2. **specialization** (Truffle-style). per-receiver-shape compiled
   stubs. requires runtime profiling + recompilation. lands after
   tier 3 stabilizes.

3. **incremental / concurrent GC**. phase 1 stop-the-world is fine
   at this heap size, but a 100k-form vat pauses ~50ms per cycle.
   mitigation: defer until visible.

4. **multi-vat parallel scheduling**. phase D's promise/scheduler
   work. once vats can run in parallel, dispatch perf compounds —
   8M sends/sec × N cores.

5. **AOT-compile hot kernels to mcos**. the parser, the compiler,
   and the type-checker are obvious candidates. compile-once via
   tier 3 toolchain; ship as content-addressed binaries. matches
   the V4 mco shape.

6. **mco-isolated user code** (sandbox). user-supplied moof code
   runs in a wasm mco; substrate has bounded blast radius.
   orthogonal to perf but enabled by tier 3's compile pipeline.

## see also

- `docs/superpowers/specs/2026-05-11-phase1-gc-dispatch-compression-design.md` —
  phase 1; the prerequisite.
- `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` — V4
  opcode set; the substrate this perf works on top of.
- `docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md` —
  why we need this (rust deletion gated on self-host workload perf).
- `crates/zig-substrate/src/vm.zig` — dispatch + Profile counters.
- `crates/zig-substrate/src/world.zig` — World, IC, frame state.
- `crates/zig-substrate/src/intrinsics.zig` — native fns (args alloc
  site lives here too via `world.allocator.alloc(Value, argc)` in
  prepareInvoke).
- `crates/zig-substrate/src/main.zig` — benchmark entry points
  (`bench-loop`, `bench-natives`, `bench-parser-like`,
  `bench-polymorphic`, `bench-deep-env`, `stress-recursion`,
  `profile-run` for real-workload diagnosis per §3.5).
- `crates/zig-substrate/src/sym.zig` — SymTable; `intern` counter
  wired here per §3.5 instrumentation.
- `NEXT_SESSION.md` — state at HEAD `4b21407`.
- `laws/substrate-laws.md` L3, L10, L11.
- `laws/determinism-laws.md` D5.
- BEAM ref: `The BEAM Book` ch. 9 (scheduler, dispatch) — for
  comparison.
- Copy-and-patch paper: Xu, Kjolstad. PLDI '21.
