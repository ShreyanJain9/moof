# next session — substrate is essentially complete; pivot to conformance + Console cap

## what just shipped (phase 1 complete: rename + freezing + intrinsic shrink)

phase 1 of the vats+substrate carve. design at `docs/superpowers/specs/2026-05-16-vats-substrate-and-image-design.md`. plan at `docs/superpowers/plans/2026-05-16-phase1-rename-freezing-shrink.md`. three workstreams, all merged.

### workstream A — directory rename

- `crates/zig-substrate/` → `players/zig/`
- `crates/substrate/` → `players/rust/`
- `crates/ocaml-seed/` → `seed/ocaml/`
- `crates/{abi,abi-rust,mco-pack}/` → `tools/`
- `crates/` directory removed.
- Cargo.toml workspace updated; all builds (rust workspace, zig, ocaml, polyglot bootstrap) green from new paths.
- 7 commits; history preserved via `git mv`.

### workstream B — freezing primitive surface

- **VatMode enum + vat_mode field on World** — `mutable_default` | `frozen_default`. V4 precursor; moves to per-Vat in multi-vat carve.
- **auto-freeze on alloc** — when `vat_mode == .frozen_default`, every `allocInstance` result is born frozen. `allocMutableBypass` for explicit bypass. (Known gap: `allocFlatCons` and `heapAllocFormWithProto` don't yet honor vat-mode; TODO(phase2) comments in place. Zero impact today since default mode is mutable.)
- **`isFreezable` predicate** — returns false for already-frozen forms OR ForeignHandle forms (live-face per spec §4.5). `Object:freeze` raises `'cannot-freeze-live` for live faces; is idempotent on already-frozen.
- **`__vat-mode__` intrinsic** — returns `'mutable-by-default` or `'frozen-by-default` as a Symbol.
- **`__alloc-mutable__` intrinsic** — allocates a fresh mutable form regardless of vat-mode.
- **`let-mutable` macro** — `lib/early/12-vat-mode.moof`. build-then-seal pattern for frozen-default vats.
- **freezing conformance corpus** — `tests/conformance/freezing.json` with 5 triples; pending runner (future phase).
- **36 zig tests pass** (8 new vat-mode tests + 28 existing).

### workstream C — intrinsic shrink first pass

**1 method removed:** `Object:initialize` from zig intrinsics; canonical is now `(defmethod Object (initialize) self)` in stdlib/object.moof. starting intrinsics.zig LoC: 2506, ending: 2503.

**Important finding:** the zig substrate was already designed minimally. the methods the plan expected to remove (Cons:length, Cons:map:, Integer:abs, String:trim, etc.) were never added to zig intrinsics — the zig port started conservative. only `Object:initialize` (trivial `return self` stub) was safely removable without modifying the OCaml seed.

**LoC budget status:** 2503 (vs 2506 start; original target was ~1750). The 30% target was based on rust's `intrinsics.rs` as a reference; reality is the zig port never had those redundancies. Future shrink approaches (move chunk/opcode infrastructure to moof, move transporter internals) require deeper redesign and aren't "first pass" work.

**bootstrap warning:** `image-load: unknown native 'Object:initialize' for form_id=87 — skipping` — harmless. seed.vat still references it; the moof defmethod shadows it. warning disappears when OCaml seed is updated.

**classification working doc:** `docs/superpowers/working/2026-05-16-native-classification.txt` — full audit of all 112 registry entries with KEEP/AXE classification + reasoning per entry. useful reference for future shrink work.

### phase 1 baseline preserved

bootstrap: 23 stdlib files load (12 early + 11 stdlib including `12-vat-mode.moof`); final error `UnboundName: Console` — same baseline as pre-phase-1.

### next: phase 2 — vat carve (W4)

per the spec §14 implementation phasing: carve `World` into `World + Vat`. single scheduler initially. spawn / mailbox / supervisor primitives. ~1-2 weeks. unblocks V5 (references), V6 (shared segment), V7 (promises), V8 (supervision).

---

## what just shipped (HEAD `041f8fd`)

a massive session. went from "polyglot is an idea but not in service of a goal" to:

- **substrate-level functional self-host** — moof drives its own bootstrap through 22 stdlib files
- **5.87× real-workload speedup** plus 64× microbench from §4.x tier-1 perf
- **Layout mechanism** — generalizable flat-rep; FlatCons hack removed; foundation for user-proto auto-flatten
- **V1 per-turn nursery + diff** ported from rust to zig (preserves turn semantics pre-rust-deletion)
- **GC adaptive trigger + free-list reuse** — 23→2 cycles per bootstrap, GC overhead 22%→12%
- **VM dispatch refactor** + TailSend Method:call peephole — no host-stack recursion (216,670 peephole hits during bootstrap)
- **String/intern caches** in zig; **defproto auto-flatten** wired via `$layout` cap
- **phase 3 cohesive vision spec** (2096 lines) — image-as-canon polyglot reframe + native laziness + compaction + MCO hooks + Erlang vats + 1B-sends/sec path

### the bootstrap journey this session

```
moof run /tmp/seed.vat
↓
all 11 early/* ✅       (cons, nil, bool, string-ess, symbol,
                         quasiquote, control-macros, modules,
                         match-defn-proto, defmethod, if-macro)
↓
stdlib/* ✅              (object, bool, nil, cons, freezing, integer,
                         float, string, char, table)
↓
stdlib/console.moof → UnboundName: Console (substrate cap not installed)
```

**22 stdlib files load in ~22.5s.** the polyglot self-host stack works end-to-end at the substrate level — what remains is mostly missing cap wiring.

### what exists now

| dir | role | status |
|---|---|---|
| `players/zig/` | THE runtime — heap, vm, gc, image, intrinsics, nursery, layout | substantial; 4000+ LoC zig |
| `players/rust/` | rust build-time oracle | WORKS but slated for deletion (W5e) once polyglot complete |
| `seed/ocaml/` | minimal bootstrap compiler | works; produces seed.vat (~91 KB, 305 chunks, 77 natives) |
| `lib/` | stdlib + parser + compiler + early macros | unchanged structurally; defproto auto-flatten added |

### perf snapshot

| metric | value | path-to-target |
|---|---|---|
| bench-natives microbench | 14.9M sends/sec | tier 2: PICs + threaded dispatch → 50M+ |
| bench-parser-like microbench | 10.7M sends/sec | tier 2 → 30M+ |
| real-workload (parser+compiler on stdlib) | 532K sends/sec | tier 2 → 5M; tier 3 JIT → 100M; specialization → 1B |
| GC overhead | 12% of wall time | generational compaction → ~3% |
| bootstrap wall time | 22.5s (was: hung) | tier 2/3 + Console fix → <5s |
| `[1 is nil]` parse (isolated) | ~100ms range | tier 2 → <10ms |

we are well within striking distance of BEAM-interpreted parity (5-10M sends/sec real). tier 3 copy-and-patch JIT gets to BEAMJIT-class.

---

## what's next (this session's queued tasks)

### immediate unblocker (1-2 hours)

**Console cap install in zig substrate (task #46).** the 23rd stdlib file blocks because `Console` global isn't installed by `installCaps`. mirror rust's `install_console_cap` in `players/rust/src/intrinsics.rs`. after this, bootstrap likely reaches even further into stdlib OR completes end-to-end.

### tier-2 perf (next big push, ~2-3 weeks)

phase 2 spec at `docs/superpowers/specs/2026-05-16-phase2-moof-performance-design.md`:

- §5.1 4-way PIC (polymorphic inline caches) — 1.3×
- §5.2 inline Int+Int fast path — 1.5×
- §5.3 tail-call threaded dispatch (zig 0.16 `@call(.always_tail)`) — 2×
- §5.4 flat env representation — 1.5× (high risk; touches L1)
- §5.5 closure flat representation — 1.3×
- §5.10 parser-level intern memoization — small, was 0% hit (re-dispatch)
- §5.11 tail-send IC (wire-format amendment) — 1.1×

cumulative target: 10-15× → 5-10M sends/sec real workload (BEAM-interpreted parity).

### handoff session priorities (recommendation from #45 design agent)

> **"design the v0.5 conformance test corpus — 50-100 (image, message, expected-result) triples + the `moof conform manifest.json` command per spec §1.3."**

this pivots polyglot from "described in a spec" to "tested on every commit." enabling work for: wasm-browser player, byte-format freeze, cross-player parity.

### vats-V4 onward (months of work, BEAM-rivaling)

per `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md` §22:

- V4 multi-vat container — round-robin scheduler, per-vat heap isolation
- V5 references protocol — far-refs, membrane translation, cap-tokens
- V6 shared segment — content-addressed cross-vat immutable
- V7 eventual sends — `<-` operator, Promise Form
- V8 supervision + spawn — `[$vat spawn:]`, let-it-crash
- V9 persistence — per-vat lmdb + journal
- V10 capabilities + intents
- V11 replication + CRDT hooks

V1 nursery just landed in zig → V4 multi-vat is unblocked.

### tier-3 perf (months out)

per phase 3 spec §6:

- copy-and-patch JIT (~4-6w MVP) → BEAMJIT-class
- Self/Truffle-style shape specialization → near-native (1B sends/sec hot)

---

## known gotchas + open questions

### immediate

1. **moof eval / run requires MOOF_LIB env set** when called without lib/ in cwd. Set `MOOF_LIB=/path/to/lib` before running.
2. **The polyglot path requires ocaml-seed + zig moof + lib/ all in sync.** rebuild seed.vat after any lib/ change.
3. **Console + emit + a few other caps** missing on zig substrate's installCaps — small ports each.
4. **`(set! count ...)` style still routes through env-binding**, not slot. defproto auto-flatten registers the Layout but full ergonomics require §11 `.foo`/env-lookup work (`docs/superpowers/specs/2026-05-10-dot-slots-and-pipes-design.md`).

### phase 3 spec risks (load-bearing)

1. **wasm-browser players cannot JIT** — interpreted-only, no top-tier parity. decision deadline: v2.0.
2. **.vat format frozen at v1.0 is binding** — byte-level spec + validator + migration framework needed AT v1.0, not after.
3. **mco security audit before v2.0** — moof_call / moof_form_slot_set ABI is the cap surface; needs cap-token enforcement before public mcos.

### deferred items

| task | what | why |
|---|---|---|
| #34 §5.7 cached SymIds | small mechanical port | overlapped with §5.8 work; may already be partly done |
| #35 §5.10 parser-level intern memoization | parser-side cache | small win; re-dispatch with tight scope |
| #43 follow-ups | `become_` rollback in nursery; TurnDiff serialization | V9 persistence work |
| Env / Method / Closure layouts | one-liner each post-#41 + sweep alloc sites | small but tedious |
| §5.4 flat env | high risk, touches L1 | needs design care |
| §5.5 flat closure | medium risk, many alloc sites | sweep work |
| §11 `.foo` slot read + pipes implementation | spec ready at 2026-05-10-dot-slots-and-pipes-design.md | unblocks Counter-style ergonomics |

---

## starting the next session

1. `git pull` — confirm at `041f8fd` or later
2. `cargo build --release -p moof --bin moof-rs` — produces rust safety net
3. `cd players/zig && zig build && cd -` — produces `players/zig/zig-out/bin/moof`
4. `eval $(opam env --switch=wasm-mco)` then `dune build --root seed/ocaml` — ocaml-seed builds
5. `dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat` — produces 91 KB seed.vat
6. `MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat` — should reach `UnboundName: Console`
7. Read `docs/superpowers/specs/2026-05-16-phase3-cohesive-vision-design.md` for the next several months' direction
8. Read this file's "what's next" sections + recommended first dispatch
9. Pick a workstream:
   - immediate: Console cap install (#46) + remaining cap wirings
   - cleanup: §5.10 intern + Env/Method layouts (§5.5)
   - strategic: tier-2 perf push, OR conformance test corpus, OR vats-V4 multi-vat
10. Dispatch agents in parallel where files don't overlap

if all 6 build/setup steps pass, you're ready to dispatch.

---

## the moof philosophy hasn't changed

still:
- environment, not language
- the maru posture (tiny substrate seed)
- the four faces of Form
- moldable, reflective, expressive, pure

we've made the substrate dramatically faster + added turn semantics + generalized flat reps + clarified the vision. moof is a real shape now, not just an idea.

next session: **make a moof image a real artifact**. ٩(◕‿◕｡)۶

---

## see also

- `docs/superpowers/specs/2026-05-16-phase3-cohesive-vision-design.md` — THE roadmap doc
- `docs/superpowers/specs/2026-05-16-phase2-moof-performance-design.md` — perf design + real-workload measurements
- `docs/superpowers/specs/2026-05-11-phase1-gc-dispatch-compression-design.md` — substrate optimization (mostly shipped)
- `docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md` — self-host design
- `docs/superpowers/specs/2026-05-10-dot-slots-and-pipes-design.md` — `.foo` + pipes language design
- `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md` — vats roadmap V4-V11
- `docs/roadmap.md` — overall phase plan
