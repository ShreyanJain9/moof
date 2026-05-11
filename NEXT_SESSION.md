# next session — V4 polyglot, getting toward rust deletion

## what just shipped (HEAD `380203b`)

**V4 polyglot substrate. the stdlib boots end-to-end through zig + ocaml.**

over the last two sessions we went from "rust substrate only, V3 done"
to a working polyglot stack where:

- **rust at build time** produces `system.vat` (21.6 MB) — a V4 vat-image
  capturing the fully-bootstrapped moof World (596K forms, 685 syms,
  1153 chunks, 50+ native methods).
- **zig at runtime** loads `system.vat` and reconstructs the World in
  memory. heap populated, native methods re-bound by name, here_form
  + macros_form resolved.

```
$ cargo run -p moof --bin moof-rs -- export-v4 --output /tmp/system.vat
  wrote /tmp/system.vat (21,645,729 bytes)

$ ./crates/zig-substrate/zig-out/bin/moof load /tmp/system.vat
  loaded /tmp/system.vat (21,645,729 bytes)
    heap.len    = 596,905
    syms.len    = 685
    chunks      = 1,153
    natives     = 21 (29 in registry; rest are stdlib-internal)
    here_form   = FormId(scope=vat_local, payload=18)
    macros_form = FormId(scope=vat_local, payload=19)
  V4 vat-image alive ٩(◕‿◕｡)۶
```

### what exists

| crate | LoC | role | status |
|---|---|---|---|
| `crates/zig-substrate/` | ~3500 zig | the runtime (heap, vm, intrinsics, image-load) | builds clean; loads stdlib; runs hand-constructed bytecode |
| `crates/substrate/` | ~8000 rust | the build-time oracle (runs `new_world()` + serializes) | builds clean; existing runtime CLI still works (REPL, `moof '<expr>'`); deletion target after polyglot proves itself |
| `crates/ocaml-seed/` | ~2500 ocaml | future compiler (will replace rust when parser.moof self-hosts) | builds clean; parses + compiles simple programs; produces V4-spec bytes; cross-stack verified at the byte boundary |
| `lib/` | ~5000 moof | the stdlib — Compiler.moof + early/* + stdlib/* + mcos | unchanged; compiled by rust runtime, serialized into system.vat |

### V4 ops set (24 total)

19 op-tag bytes used; 5 reserved for phase D (Suspend/Resume) and future
fusions. byte-tagged, big-endian, fixed-width operands. content-hashable
chunk bytecode. spec at `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`.

### key strategic move from session N

option D — use rust at BUILD time as a compiler+serializer, not RUNTIME.
this skipped the bootstrap chicken-and-egg entirely (parser.moof doesn't
exist yet; macro expansion in OCaml would be huge). rust runtime keeps
working as the safety net; rust deletion happens LATER once polyglot
fully self-hosts.

dual-brainstorm contributed two gold nuggets:
- gemini caught: sym-table hydration semantics (image replaces world.syms,
  doesn't append)
- gemini caught: `std.StaticStringMap` for comptime native registry

both folded in. resulting load is robust.

---

## what's next (this session)

three tracks, roughly in dependency order. each is ~half a session.

### track A — execute moof code against the loaded image

**right now:** moof LOADS the vat-image but doesn't RUN moof code
against it. the world is "alive" but inert.

**what we want:** `moof run /tmp/system.vat <chunk-id>` or similar —
invoke a specific chunk, observe the result. or: `moof exec /tmp/system.vat
<bytecode-file>` — load image, then run a hand-constructed bytecode against
its world.

**why it matters:** proves end-to-end. proves IC dispatch works against
re-bound native methods. proves chunk_consts / chunk_ics / heap references
all align across the rust-emit / zig-load boundary.

**concrete plan:**
1. add `moof exec <vat> <bytecode-file>` — instantiate a top-level
   chunk from raw V4 bytes, run via `vm.runTop` against the loaded world.
2. smoke: `moof-seed bytes /tmp/test.moof | moof exec /tmp/system.vat -`
   for `[1 + 2]` → `Int 3` via real IC dispatch through stdlib's protos.
3. probable bug-hunt: chunk side-tables, IC slot initialization, dispatch
   against re-bound natives.

**risk:** the stdlib's chunks reference SymIds that may not match what
fresh bytecode (compiled at runtime by moof-seed) uses. solution: have
moof-seed READ the loaded image's sym table first, intern symbols against
that canonical numbering. or: post-process moof-seed's output to remap
SymIds.

### track B — grow zig's intrinsic registry

**right now:** zig REGISTRY has 29 natives. rust v4_export emits ~50.
20+ are warned-and-skipped at load time. these are mostly:
- `Opcode:loadSelf`, `Opcode:return`, `Opcode:loadConst:`, etc. — chunk-
  reflection methods used by the moof Compiler when introspecting its
  own output.
- `Chunks:bodyOf:`, `Chunks:opsListOf:`, `Chunks:icsListOf:`, etc. —
  same family.
- `Compiler.useMoof` / `Compiler.useSeed` — the flag-flip methods.
- `$transporter.of:` — the file-load cap.
- ~6 anonymous singleton methods (the `<anon-N>:foo` warnings).

**what to do:** port each missing native from rust intrinsics.rs to zig
intrinsics.zig + add to the comptime REGISTRY. mostly mechanical. some
will need wasm runtime support (for `$mco`).

**scope:** ~3-6 hours of porting. completes before track A's smokes
have any chance of running stdlib code that needs these natives.

### track C — runtime source parsing (the parser problem)

**right now:** moof consumes bytecode but can't parse source. once
track A works, the natural ask is `moof "(+ 1 2)"` — eval a source
string. needs a reader at runtime.

**three approaches:**
1. **port `reader.rs` → `reader.zig`.** ~1000 LoC of zig. self-contained;
   no dependencies. preferred long-term.
2. **call ocaml-seed as a subprocess.** `moof` shells out to
   `moof-seed bytes -` to compile. simple; introduces a subprocess
   dependency at runtime.
3. **embed ocaml-seed in moof via wasm.** compile ocaml-seed to wasm
   (with wasm_of_ocaml if it works), call it via the wasm runtime that's
   already in zig substrate. fits the philosophy ("OCaml as mco"); requires
   solving the OCaml→wasm story.

**recommend:** option 1 for V4 maturity, option 2 for tonight if we want
to ship something running. option 3 is the elegant end state.

once any of these works, plus track A + B, we have **moof replacing
moof entirely** for ordinary use. rust deletion is one step away.

---

## the bigger arc (post this session)

| step | what | unlocks |
|---|---|---|
| this session | tracks A + B + C | moof is feature-complete |
| next session | rust deletion | only crates/zig-substrate + crates/ocaml-seed remain |
| then | parser.moof + compiler.moof self-host | phase A-self-host complete; ocaml seed deletion follows |
| then | phase B (persistence) | save vat per turn; restore from disk; live image is real |
| then | phase D (replication) | multi-vat in-process; canonical-hash convergence |
| then | phase E (3D world demo) | the moofpaint forcing function |
| then | phase F (federation) | multi-user 3D world via websocket |

the V4 image format ALREADY IS the phase-B persistence format. designing
it well now (per-vat structure, content-addressable, deterministic) pays
back across B/D/F all at once.

---

## known gotchas + TODOs

these are flagged-but-not-blocking. fix when they become hot:

### from track 1 (rust v4_export)

1. **non-scalar Value serialization.** rust may have inline `Value::Str` /
   `Value::Bytes` somewhere; current export treats only the 7 scalar
   variants (Nil/Bool/Int/Sym/Char/Float/Form). If a non-scalar Value
   shows up in a chunk's const-pool or a Form's slot, it's emitted as
   Nil with a panic-on-encounter. **investigate:** does `lib/` ever hit
   this? unclear. mitigation: scan for inline String values during
   export; allocate them as Forms first if found.
2. **Source form linking.** every chunk has a `:source` slot referencing
   the source-Form it was compiled from. currently the export writes
   `chunk_id` as `source_form_id` (placeholder). L5 (source is canonical,
   bytecode derived) is violated until this is fixed. **fix:** look up
   chunk's `:source` meta slot, find that Form's FormId, emit.
3. **Footer hash is zeros.** zig stubs verification. real blake3 footer
   needed when content-addressing actually matters. phase 9 work.
4. **Proto count mismatch.** spec §10.3 wants 18 protos; rust has 17 +
   `macros_form` slotted in. `Opcode` proto FormId is emitted as NONE.
   moof handles NONE gracefully; long-term, add Opcode proto on rust
   side OR shrink spec to 17.

### from track 2 (zig image-load)

5. **sym-table cached symbol IDs in `World.initBare`.** zig caches hot
   syms (parent, view-target, etc.) at init; after image-load's
   `clearAndKeepCapacity`, those SymIds are stale. fine for load-and-
   inspect, broken for active env walks. **fix:** re-intern after load,
   or read SymIds from the loaded image's table.
6. **wasm mco loading is stubbed.** moof logs "would load mco" and
   continues. for stdlib methods that genuinely need mcos (Hash, Utf8,
   Clock, etc.), they'll fail at dispatch. acceptable for boot-and-inspect;
   blocks "run stdlib code that hashes things."
7. **20+ unknown natives skipped at load.** see track B above.

### across both

8. **OCaml seed didn't ship a working build-image command.** the
   image+CLI agent in session N stubbed it; it produces a SKELETON .vat
   but not a full bootstrap image. rust's v4_export is the canonical
   path until ocaml-seed catches up.
9. **stale worktree branches in `git branch`.** 9+ zombie branches from
   parallel-subagent work. `git branch -D worktree-agent-*` cleanup is
   safe; not urgent.
10. **OCaml lives in `opam switch wasm-mco`.** undocumented for CI.
    `eval $(opam env --switch=wasm-mco)` before any `dune` command.

---

## starting the next session

1. `git pull` — confirm at `380203b` or later.
2. `cargo build -p moof --release --bin moof-rs` — produces `target/release/moof-rs`.
3. `cd crates/zig-substrate && zig build && cd ..` — produces
   `crates/zig-substrate/zig-out/bin/moof`.
4. `cargo run -p moof --bin moof-rs -- export-v4 --output /tmp/system.vat`
   — 21 MB image.
5. `./crates/zig-substrate/zig-out/bin/moof load /tmp/system.vat`
   — should print `V4 vat-image alive`.
6. read `docs/superpowers/plans/2026-05-10-vm-V4-C3-stdlib-bootstrap.md`
   if you need plan context.
7. read `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` for
   the byte format / image format contract.

if all 6 pass, pick a track (A/B/C above) and dispatch parallel agents.

---

## the moof philosophy hasn't changed

still:
- environment, not language
- the maru posture (tiny substrate seed)
- the four faces of Form
- moldable, reflective, expressive, pure

we've just made the substrate **smaller** and **polyglot**. zig owns the
runtime; ocaml will own the compiler; rust is the build-tool that goes
away. moof code keeps doing what it does. ٩(◕‿◕｡)۶
