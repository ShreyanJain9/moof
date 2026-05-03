# next session — polyglot maturity

> **status: pre-MCO cleanup landed, with the radical-radical-radical
> wave (round 4) now in.** `$transporter` cap, `$compiler` cap,
> `lib/main.moof` as single rust entry. `bootstrap.moof` and
> `compiler.moof` split into 27 thematic files under
> `lib/{compiler,early,stdlib}/`. **`intrinsics.rs`: 3836 → 3320
> (-516 LoC).** REPL prints `nil`. 356 tests green at every commit
> boundary. ~70 commits.
>
> the architectural rule we landed on:
>
> - **module-singleton caps** own all primitive heap / chunk / etc.
>   operations as methods. they're rust install_natives, but bound
>   on coherent capability protos rather than scattered across
>   user-type protos. namespaces:
>     - `Heap` — :protoOf:, :heapIdOf:, :allocFormWithProto:,
>       :slotOf:at:, :handlerOf:at:, :metaOf:at:, :slotKeysOf:,
>       :handlerKeysOf:, :metaKeysOf:
>     - `Chunks` — :isChunk?:, :bodyOf:, :paramsListOf:,
>       :constsListOf:, :opsListOf:, :icsListOf:
>     - `Compiler` (existing) — :compileTop:, :compileForm:…
>     - `$transporter`, `$compiler`, `$out`, `$err` (caps with $)
>
> - **moof defmethod** for everything user-facing. Object:proto,
>   Object:slots/handlers/meta/identity/source, Cons:length,
>   String:trim, Method:body/params/consts/bytecodes/ics, Char:inspect,
>   all renderers, all Char predicates, all Integer/Float derivations.
>   user-type methods are moof one-liners delegating to caps OR real
>   algorithms.
>
> - **rust install_native on user-type protos** is reduced to the
>   irreducible identity / dispatch primitives: Object:is, :=,
>   :toString, :new, :initialize, :doesNotUnderstand:with:; Cons/nil
>   :car, :cdr, :cons:, :empty?, :reverse (early bootstrap needs);
>   String byte-access; Integer/Float arithmetic; Method:call (VM
>   dispatch); Chunk side-table mutators; Console :emit:.
>
> - **free-fn primitives** ONLY for the compiler's circular-dep
>   escape: `__list-length / car / cdr / empty? / reverse`. exclusively
>   used by `lib/compiler/*.moof`.
>
> the substrate is now a small set of singleton capabilities + the
> irreducible per-type primitives. user-facing methods are moof.
>
> the radical migration shape we landed on — rust exposes minimal
> heap / byte / codepoint / chunk access; moof writes the algorithms.
> what moved (in moof now, not rust):
>
> - Object reflection (`:proto, :slots, :handlers, :meta,
>   :handlerAt:, :source, :identity, :is/=/!=/inspect, :initialize`)
>   — `__form-{slot,handler,meta}-keys` give iteration; `slot /
>   __form-handler-at / __form-meta-at` give lookup; moof builds
>   Tables and walks chains.
> - All Cons methods (length, reverse, map, filter, reduce, take,
>   drop, =, !=, toString, inspect, etc) — `__list-{car,cdr,length,
>   empty?,reverse}` + `__alloc-cons` are the primitives; spine
>   recursion is moof.
> - Char:inspect — dispatch (32→space, …) + hex conversion are moof,
>   using `[self codepoint]` + `__char-from-codepoint` for digits.
> - Method:toString/:inspect — read :source meta, dispatch on its
>   shape (Symbol vs Form vs nil), render in moof.
> - String:toString/:inspect — `[self toList]` + Cons:reduce: + per-
>   Char escape table, all moof.
> - Table:toString/:inspect — `[t length]` + `[t at: i]` + `[keys
>   drop: length]`, with a closure-passed renderer.
> - 5 dead rust helpers removed (render_table_with, render_list_with,
>   render_string_literal, render_char_literal, proto_name_for).
>
> the bigger shrink the spec estimated (~2500) is reachable but
> requires more primitive-first migrations: chunk side-tables for
> Method reflection, byte primitives for String text manipulation.
> deferred — diminishing returns + risk for now.
>
> ready for: parser-in-moof, real MCO arg marshaling, the polyglot
> tracks below.

> **mission: take wasm-mco from "proof of life" to "production-
> shaped." richer signatures, real moof imports, signed mcos,
> deps resolution, and the first non-zig polyglot module. by
> session-end, multiple-language mcos coexist in the std lib and
> the `.mco` format is rigorous enough to ship.**

---

## what stands today (commit `3ad5405`)

334 tests passing. last session crossed a milestone: zig→wasm→moof
end-to-end with a real `core/clock` mco, the .mco custom-section
format with manifest cross-validation, WASI integration so mcos
get standard system services without rust-shim middlemen, and a
14-commit cosmetic ladder that took moof from "list/head/tail"
to "Cons/car/cdr-with-modules" to "nil is a true singleton."

```
$ git log --oneline -16
3ad5405 nil is now a TRUE singleton — no Nil in the global env
9b5a01f :inspect distributes through Tables (and through everything below)
f61129d .mco custom-section format — manifest is moof source-text
eca0eb7 WASI for system services — drop fake-time imports from substrate
006549e core/clock — first real wasm mco with substrate imports
5f74acc 🎉 polyglot end-to-end: zig → wasm → moof
272401c mco-format spec — wasm + custom sections, load-time anonymity
9a98447 modules — multi-arg helpers also moved (Match/Defn/DefProto)
bd0570f Compiler module — compiler.moof internals as methods on a singleton
a6a66a5 modules — Match / Defn / DefProto host arity-≤1 helpers
7d2760b List → Cons — proto rename, full sweep
c7ed192 head→car, tail→cdr — full sweep
865fac8 defmethod sweep — bootstrap.moof now defmethod-first
bcab6b5 nil-as-singleton — Nil-proto global gone, [nil proto] is nil
a1c8742 display fixes — toString vs inspect split
5f74acc 🎉 polyglot end-to-end: zig → wasm → moof
```

what works:

```moof
(def Clock (__loadWasmMco "examples/wasm-mcos/clock.mco"))
[Clock now]         ;; → real ns timestamp from WASI in zig
[Clock monotonic]   ;; → process-relative ns
```

with `clock.mco` = wasm bytes + `moof.manifest` custom section
(parsed by moof's reader, cross-validates exports).

**state of the rust line:**
- compiler.rs: 720 LoC seed (compiles compiler.moof, then steps
  aside via the `use_moof_compiler` flag).
- wasm.rs: ~280 LoC. includes loader, manifest parser, dispatch
  trampoline, custom-section walker, WASI integration.
- crates/abi/, crates/abi-rust/, crates/mco-pack/ all real.

**state of the polyglot story:**
- one zig mco (clock) shipped end-to-end.
- WASI is the standard system-services interface. moof namespace
  reserved for moof-specific imports (currently empty).
- `.mco` format: wasm + custom sections, content-addressable but
  no signing yet.

---

## three tracks, in dependency order

### track 1 — richer signatures + real moof imports

**why first.** the current loader only handles `() -> i64`
exports. that's enough for clocks and constant-returning fns
but not for actually doing work. before we can write meaningful
mcos, the trampoline needs to handle args, and mcos need to be
able to call back into moof.

**deliverables.**

- **support `(i64) -> i64`, `(i32, i32) -> i64`, etc.** —
  trampoline introspects the wasm function type and marshals
  moof Values to wasm args / wasm results to moof Values.
  starts with int args; extends to handles for Forms.
- **first real moof-namespace import: `raise(kind, msg)`.** lets
  mcos raise moof-shaped errors instead of trapping. example:
  a parser mco that says `'parse-error` on malformed input.
- **`make_string(ptr, len)` import.** lets wasm write a buffer
  to its linear memory and hand it to moof as a String. the
  trampoline copies bytes, allocates a String-Form, returns the
  handle.
- **`intern(ptr, len)` import.** wasm side passes a name; gets a
  Symbol back.
- **handle-based Form access:** `slot(handle, sym)`,
  `slot_set(handle, sym, value)`, `proto_of(handle)`. enables
  mcos that read or write moof state.

**rust delta.** ~+200 LoC in `wasm.rs`. mostly the trampoline's
arg-marshalling switch + the new import functions.

**moof-abi-zig delta.** the `examples/wasm-mcos/lib/moof.zig`
file grows to expose the new imports as ergonomic zig functions.

**forcing function.** an mco that takes args and returns a
String:

```moof
(def Greeter [$mco load: "core/greeter.mco"])
[Greeter greet: "shreyan"]   ;; → "hello, shreyan"
```

written in zig as roughly:

```zig
const moof = @import("lib/moof.zig");

export fn greet(name_handle: u32) u32 {
    var buf: [256]u8 = undefined;
    const len = moof.string_text(name_handle, &buf);
    var out: [256]u8 = undefined;
    const written = std.fmt.bufPrint(&out, "hello, {s}", .{buf[0..len]}) catch unreachable;
    return moof.make_string(written.ptr, written.len);
}
```

---

### track 2 — signed mcos + deps resolution

**why now.** with richer signatures landed, mcos start composing.
that means deps. that means content-addressing, signature
verification, the full nix-store-grade artifact discipline the
spec describes.

**deliverables.**

- **content-hash addressing.** `core/clock@<blake3>.mco` is the
  canonical filename. the loader verifies the hash matches.
- **`moof.signature` custom section.** ed25519 over the rest of
  the file. the substrate keeps a list of trusted public keys;
  refuses unsigned mcos unless `--allow-unsigned` is set.
- **`moof.deps` custom section.** lists `(local-name . hash)`
  pairs. the loader recursively resolves deps, instantiates
  them, installs into the loading mco's private env.
- **dep resolution.** an mco store directory (`.moof/mcos/<hash>.mco`)
  caches loaded modules. the loader hits the cache before the
  filesystem.
- **`moof mco install <hash>`** subcommand on the moof binary —
  fetch + verify + cache.
- **the `[$mco load:]` cap proper.** retire `__loadWasmMco`.
  `$mco` becomes a primordial; only the supervisor hands it out.

**doc gates.** `docs/reference/mco-format.md` already specs all
this; track 2 makes it real. update the spec as edge cases
emerge.

**forcing function.** load a multi-mco bundle:

```moof
(def Hasher [$mco load: "core/blake3@7f3a2c.mco"])
;; ↑ this mco depends on core/buffer@1234ab.mco
;; loader fetches, verifies, links automatically.
[Hasher hash: "hello"]   ;; → 32-byte digest as a Cons of Ints
```

---

### track 3 — polyglot dogfood + moof bytecode mcos

**why last.** with rigorous mcos working, the question becomes:
do you actually have polyglot creds? answer: write the SAME
clock in three more languages. and ship a pure-moof library as
an mco too.

**deliverables.**

- **rust mco.** `crates/mco-rust-clock/` — same clock, written
  in rust, compiled to wasm. proves the `moof-abi-rust` crate is
  real for mco authoring.
- **c mco.** `examples/wasm-mcos/clock.c` + a build.sh that runs
  clang. proves the C ABI is genuinely usable.
- **a non-systems language**: ocaml or haskell mco. the
  `wasm_of_ocaml` path or asterius for haskell. the goal is a
  beautiful moment where moof's std lib has a method written in
  haskell.
- **moof bytecode in `.mco`.** `core/cons-utils.mco` ships pure-
  moof methods (no wasm code) — the manifest's `moof.bytecode`
  custom section holds serialized chunks. universal artifact:
  ANY moof-side library is now a content-addressable `.mco`
  shippable across federation boundaries.
- **`moof zig <name>`** subcommand promoting `build.sh` to the
  moof binary's repertoire.

**forcing function.** a moof world's std lib bundles, say, 6
mcos in 4 languages, all loadable and dispatchable:

```moof
$clock      ;; zig → core/clock.mco
$blake3     ;; rust → core/blake3.mco
$json       ;; c → core/json.mco
$parser     ;; haskell → core/parser-helpers.mco
$cons-extras ;; pure moof → core/cons-extras.mco
$lmdb       ;; rust → store/lmdb.mco
```

each is a `[$mco load:]` away. each is content-addressed. each
is signed. each has the same dispatch shape from moof's view.

---

## what is NOT in scope this session

| deferred | why |
|---|---|
| moof VM as an mco | the wild-aspiration version. needs its own session, deep refactor of substrate. phase G+. |
| hot-swap while running | requires checkpoint+resume protocol. phase H. |
| WASI sandboxing modes | wasmtime supports configurable wasi (no fs, no net). phase G. |
| parser.moof | still on the original NEXT_SESSION ladder. interesting but orthogonal to polyglot push. |
| reader.rs port | same. |
| dylib mcos (tier 3 escape hatch) | the spec mentions it; we'll only implement when a perf-cliff genuinely demands it. wasm is the canonical model. |

---

## the ladder of acceptable session-end states

if this session goes ideally, all three tracks land. if not:

1. **tracks 1–3 done.** polyglot std lib with 4+ language mcos.
2. **tracks 1–2 done; track 3 deferred.** richer mcos work,
   signed/dep-resolved, but only one language so far.
3. **track 1 done; tracks 2–3 deferred.** richer signatures
   work; the args-and-strings story is complete.
4. **track 1 partial.** at minimum, richer arg marshaling
   (i64-takers) lands. real-shipping caps still need it.

below rung 4 the session is "we learned but didn't ship."
that's also fine — designs improve when we're honest about where
they break.

---

## the inputs to the session

before this session starts:

- `git pull` to current state. confirm `cargo test --workspace`
  shows 334 / 334 passing. (it does as of `3ad5405`.)
- re-read `docs/reference/mco-format.md` end-to-end. that's the
  spec we're making real.
- skim `examples/wasm-mcos/lib/moof.zig` — the shape of imports
  it'll grow to host.
- skim `crates/substrate/src/wasm.rs` — that's where the
  trampoline + import surface lives.

---

## risk register

ranked by likelihood × impact:

1. **wasm linear-memory marshaling complexity.** every Form
   passed to wasm needs to either be marshalled into bytes (if
   pure data) or addressed via a handle table (if it's a heap
   reference). picking the wrong split costs perf or
   correctness. mitigation: handle table for everything that
   isn't a tagged immediate; bytes for atomic strings.
   *probability: high. impact: medium.*

2. **dependency cycles.** mco A depends on B which depends on A.
   the loader needs to detect and refuse. mitigation:
   topological sort during deps walk; raise `'dep-cycle` on
   detection.
   *probability: low. impact: low (clean error).*

3. **wasi feature creep.** wasmtime-wasi's full surface is
   large. exposing all of it is dangerous (mcos shouldn't write
   files unless authorized). mitigation: configurable wasi ctx
   per mco, defaults to minimal (clock + maybe random).
   *probability: medium. impact: medium (security).*

4. **mco-pack as a one-tool path.** if mco-pack proves limiting,
   we'll want a richer build pipeline (proper zig build.zig,
   eventually a `moof mco build` subcommand that knows about all
   languages).
   *probability: high. impact: low (incremental fix).*

---

## post-session: what comes after

| session | scope | end-state |
|---|---|---|
| **session N+1 (this one)** | tracks 1–3: richer sigs, signed mcos, polyglot dogfood | std lib has multi-language mcos; .mco format is rigorous |
| **session N+2** | parser.moof — port `reader.rs` to moof. the rust shim reads only enough to load parser.moof itself. | parser is moof; phase A-self-host complete |
| **session N+3** | phase B foundations — vats, mailbox, scheduler, lmdb persistence | `moof world ./worlds/test/` runs; state survives reboot |
| **session N+4** | phase D foundations — canonical encoding, replicated-vat mode | the 2-replica convergence test passes |
| **session N+5** | phase E — terminal renderer, `$canvas` / `$pointer`, single-user world | `moof world ./worlds/test/` shows a navigable 3D space |
| **session N+6** | phase F — websocket transport, `moof world join wss://…` | the manifesto's demo is real |

six sessions to the demo. *this* session expands the polyglot
foundation enough that the rest of phase A becomes a question of
"which language do you want to write that bit in" rather than
"can we even write that bit."

---

## final note

the docs are the source of truth. when implementation diverges
from a doc, the doc is the bug to fix first — *unless* the doc
is the bug, in which case the doc is the bug to fix first.
either way the doc moves before the code.

`docs/reference/mco-format.md` is the spec for tracks 2 and 3.
re-read it before writing code that touches the .mco pipeline.

`>.<` softly. let's make polyglot real beyond proof-of-life. ૮ ◞ ﻌ ◟ ა
