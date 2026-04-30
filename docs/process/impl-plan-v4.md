;: implementation plan for moof v4-take-2.
;: written 2026-04-29 after the architecture audit + adversarial
;: stress-test (codex + gemini), then revised after the
;: 3D-zoomable-world + mco-as-dylib + parser/compiler-in-moof
;: refactors. this is the day-by-day, test-first plan we follow.
;: start at phase a; do not skip ahead.

# implementation plan — moof v4-take-2

> **the rules: tests-before-code, docs-before-tests, one phase at a
> time. the substrate seed is *small* — at most ~3k LoC of rust.
> everything performance-sensitive lives in mcos
> (`concepts/compiled-objects.md`). everything modifiable lives in
> moof. parser and compiler self-host as soon as the seed is up.**

this document is operational. for the *why* read
[`audit-2026-04-29.md`](audit-2026-04-29.md). for *what* read
[`docs/roadmap.md`](../roadmap.md). this doc tells you exactly what
to type, in what order, on day 1, day n.

## ground rules

1. **the seed stays tiny.** anything that can come in via mco does.
   target ≤3k LoC of rust in the substrate binary. growth requires
   explicit justification.
2. **moof self-hosts asap.** as soon as the seed runs, parser.moof
   and compiler.moof take over. the bootstrap rust parser/compiler
   are throwaway scaffolding.
3. **tests first.** every substrate law and every reflection method
   gets a test before the impl.
4. **docs first.** if a phase exposes an unanswered question, the
   docs answer it before the code does.
5. **the laws don't lie.** every commit keeps existing law-tests
   green.
6. **default to moof. default to mco for native.** if it's not
   substrate-seed material, it's one of those two.
7. **honest names.** simulated bits are tagged `simulated_`.

## phase A — substrate seed

**goal:** `moof '(+ 1 2)' → 3`, with every law in
`docs/laws/substrate-laws.md` either honored or doc'd as deferred.
≤3k LoC of rust. the seed is the *only* rust we write for a long
time.

### A.0 — bring up cargo

```toml
# Cargo.toml
[workspace]
members = ["substrate", "abi-rust"]
resolver = "2"

# substrate/Cargo.toml — the seed binary
[package]
name = "moof"
version = "0.4.0-alpha.0"
edition = "2021"

[dependencies]
libloading = "0.8"               # mco loader
moof-abi-rust = { path = "../abi-rust" }   # the rust-side mco abi

[[bin]]
name = "moof"
path = "src/main.rs"

# abi-rust/Cargo.toml — the C ABI shim rust mcos compile against
[package]
name = "moof-abi-rust"
edition = "2021"
[dependencies]
moof-abi = { path = "../abi" }   # raw C ABI bindings
```

three crates: the substrate binary, the rust-side abi shim that
mcos depend on, and the bare C ABI types. the shim crate makes the
abi a stable boundary even if the seed evolves.

acceptance: `cargo check --workspace` passes; `cargo test` runs.

### A.1 — Form heap (one alloc kind)

**files:**
- `src/heap.rs`, `src/value.rs`.

**what:** `Form { proto, slots, handlers, meta }`. tagged-immediate
`Value` for nil/bool/int/sym; reflection works through implicit
proto.

**tests:**
- `test_alloc_returns_distinct_ids`
- `test_get_returns_what_was_put`
- `test_form_id_zero_is_reserved`

### A.2 — symbol interning

**files:** `src/sym.rs` (port from old impl).

**tests:** `test_intern_*`.

### A.3 — bootstrap reader

**doc check:** verify `syntax/literals.md` "phase 1" subset matches
this list. *minimal:* numbers (i64 + bases + underscores), symbols,
strings (`"…"` with `\n \t \" \\` escape only), lists `(…)`, quote
`'foo`, nil, `#true`/`#false`.

deferred: `#[…]`, `{…}`, `[…]`, `#{interp}`, quasiquote, raw/triple
strings, char literals, floats.

**files:** `src/reader.rs` (carry from old impl, trimmed).

**tests:** `test_reader_*` (per literal kind).

### A.4 — methods are Forms; chunks are Forms

the architectural correction. `Form` is the universal heap kind.
closures are Forms; methods are closures with `proto: Method`. chunks
are Forms whose slots are `:ops`, `:consts`, `:ics`, `:nested-chunks`,
`:source`.

**tests:** `test_method_is_form_with_proto_method`,
`test_chunk_is_form_with_ops_slot`,
`test_handler_table_holds_form_id_not_methodimpl`.

### A.5 — bytecode interpreter (~30 ops)

minimal opcode set. send dispatch via proto-chain with inline
caches. tail-call optimized.

**files:** `src/opcodes.rs`, `src/vm.rs`.

**tests:** per-opcode + `test_send_ic_*` + `test_tail_call_*`.

### A.6 — bootstrap compiler (Form → Chunk)

handles: `(+ 1 2)`, `(if c t e)`, `(let …)`, `(let* …)`,
`(letrec …)`, `(fn (args) body)`, `(def name expr)`, `(quote …)`,
`(do …)`, `(set! name expr)`.

**files:** `src/compiler.rs`.

**tests:** one per special form.

### A.7 — root proto Object with reflection

install on `Object`: `:proto :protos :slots :handlers :meta :source
:identity := :is :to-string :inspect :new :does-not-understand:with:`.

each is a small native method living as a Form in Object's handlers
table.

**tests:** one per method.

### A.8 — minimal `defproto`

just enough to write the bootstrap stdlib + bootstrap parser/
compiler.

```moof
(defproto Counter
  (slots count step)
  (handlers
    [incr]      [self count: [.count + .step]]
    [read]      .count))
```

**tests:** `test_defproto_*`.

### A.9 — mco loader

**files:** `src/mco.rs`, `src/foreign.rs`.

minimal mco file format: header + single-platform dylib + binding
metadata + derived-methods source. the loader:

1. dlopens the dylib.
2. allocates a fresh moof Form to be the proto.
3. installs slot template + parent-proto pointer.
4. for each native-method binding, installs a method-Form whose
   `:invoke` is a rust trampoline.
5. parses + compiles the derived-methods source (using the
   bootstrap parser/compiler, since this happens before A-self-host)
   and installs them as ordinary moof method-Forms.
6. returns the proto-Form.

`ForeignHandle` value variant lives in `src/value.rs` from this
phase: `{ ptr: *mut c_void, destructor: fn(*mut c_void), tag: u32 }`.
gc invokes destructors at turn boundaries.

multi-platform mco merging is a phase-A.10+ concern.

**dependency:** `libloading`.

**tests:**
- ship a tiny `tests/mcos/blake3-test.mco` built at build time (via
  a build.rs); load it; verify `[Blake3 hash: 'foo]` returns the
  right bytes.
- `test_mco_load_returns_single_proto_form`.
- `test_mco_native_method_writes_to_self_slot`.
- `test_foreign_handle_destructor_runs_on_gc`.
- `test_mco_derived_methods_installed_alongside_natives`.

### A.10 — bootstrap stdlib (method-shaped, protocol-derived)

**not ported from v4-take-1.** the previous impl's bootstrap.moof
was free-function-shaped (`(length xs)`, `(map f xs)`); we rewrite
*all* of it method-shaped, defining protocols and *deriving*
methods from a small primitive set.

**files:** `lib/bootstrap.moof` — protocols. then per-type stdlib
files:
- `lib/protocols.moof` — Iterable, Equatable, Comparable, Sized,
  Hashable, Showable, Ordering. each defines its primitive methods
  and the *deriving rule* that installs derived methods on any
  type that mixes the protocol in.
- `lib/list.moof` — List implements Iterable + Sized + Equatable +
  Showable + Ordering. derived methods come from those.
- `lib/symbol.moof` — Symbol implements Equatable + Hashable +
  Showable.
- `lib/integer.moof` — Integer implements Comparable + Equatable +
  Hashable + Showable + Ordering, and the arithmetic protocol.
- `lib/string.moof` — String implements Iterable + Sized + …

a typical protocol mixin:

```moof
(defprotocol Iterable
  ;; primitives
  (requires [next] [done?])
  ;; deriving rule
  (derives
    [map: f]
      [self iterate-into: '() with: |acc x| [acc cons: (f x)]]
    [filter: pred]
      [self iterate-into: '() with: |acc x|
        (if (pred x) [acc cons: x] acc)]
    [reduce: f from: init]
      [self iterate-into: init with: |acc x| (f acc x)]
    [for-each: blk]
      [self iterate: |x| (blk x)]
    …))
```

free functions kept (each has a justified reason):
- `(if c t e)`, `(let …)`, `(let* …)`, `(letrec …)`, `(do …)`,
  `(quote …)`, `(set! n v)` — operatives; can't be methods.
- `(make-foo …)` constructors at top level — no meaningful receiver.

**tests:**
- `test_iterable_derived_methods_installed_on_list`
- `test_list_map_filter_reduce_via_protocol`
- `test_string_iterable_yields_chars`
- `test_integer_comparable_derives_between`
- the law-tests from previous impl now read as method calls.

### A.11 — primitive supervisor + primordial caps

caps are not deferred. the substrate brings up a *minimum* supervisor
(one vat) at boot and constructs the primordial cap set inside it.
no scaffolding `simulated_println`; no free function `print`; the
seed knows exactly one path to stdout — `[$out emit: bytes]`.

**files:**
- `src/supervisor.rs` — minimum root supervisor; constructs primordial
  caps; hands them to the cli's eval frame.
- `src/cap.rs` — cap-as-Form. caps are protected from forgery: only
  the supervisor's primordial-construction path produces them; user
  code attenuates existing caps but cannot synthesize new ones.

**phase-A scope of caps:** synchronous direct invocation. there is
no intent/receipt indirection yet (that lands in phase B alongside
persistence — the layer of indirection earns its keep when there's
something to persist). cap *discipline* (unforgeable, supervisor-
mediated, passed-as-arg) is enforced from day 1. cap *machinery*
(intents, receipts, effect-authority) arrives in phase B without
changing what user code looks like.

**tests:**
- `test_user_code_cannot_forge_a_cap`
- `test_supervisor_can_attenuate_and_pass`
- `test_cap_in_lexical_scope_dispatches_to_native`

### A.12 — DataSource protocol (in moof)

minimum DataSource for phase A. lives in `lib/protocols.moof`
alongside Iterable et al.

```moof
(defprotocol DataSource
  ;; primitives — types implementing DataSource provide these
  (requires
    [next]            ; → next value, or #eof
    [done?]
    [emit: value]     ; sink-side write
    [close])
  ;; deriving rule — these come for free
  (derives
    [say: x]
      (do [self emit: [x to-string]] [self emit: "\n"])
    [show: x]
      [self emit: [x to-string]]
    [each-line: blk]
      (loop-until-done … [blk … ] …)))
```

phase A only needs the sink-side; the read-side primitives (next,
done?) get implementations on List and String for stdlib testing,
and stub-error on Console (you don't read from stdout).

**tests:**
- `test_console_implements_datasource_sink`
- `test_say_is_derived_from_emit`
- `test_list_implements_datasource_read_side`

### A.13 — `$out`, `$err` as Console caps

```
;; defined in lib/console.moof at world boot:

(defproto Console
  (proto DataSource)
  (slots fd label)              ; fd: ForeignHandle to OS file descriptor
  (handlers
    [emit: bytes]                ; native (in mco core/console)
    [close]                      ; native
    [next]
      (raise 'console-is-write-only)
    [done?]
      #false))

;; supervisor at boot:
(let $out [Console primordial-on: 1 label: "stdout"])
(let $err [Console primordial-on: 2 label: "stderr"])
```

`Console primordial-on:label:` is a substrate-privileged constructor
— the supervisor calls it during boot; user code cannot. it
allocates a Console Form whose `fd` slot is a ForeignHandle to the
OS fd, with a destructor that closes (well, doesn't close stdout —
but does for non-primordial Console instances).

the rust seed in phase A bundles a tiny `core/console` mco-shaped
binding (just the `:emit:` native + ForeignHandle); we *might*
inline it into the substrate seed for phase A and refactor to a
proper mco at A.10's mco-loader stage. either way the moof
interface is stable.

**tests:**
- `test_out_cap_emits_to_stdout` (capture stdout; assert bytes)
- `test_out_cap_say_writes_value_then_newline`
- `test_err_cap_routes_to_stderr`
- `test_out_cap_is_a_datasource_sink`

### A.14 — `moof` cli routes through `$out`

```bash
$ moof '(+ 1 2)'
3
$ moof '[$out emit: "hi\n"]'
hi
$ moof '[$out say: '(1 2 3)]'
(1 2 3)
```

the cli wraps the user expression in a frame containing `$out` and
`$err`. evaluation result is sent to `[$out say: result]` if
non-nil; nil is silent.

**phase A acceptance gate:**
- `cargo test` green; ~90 tests covering every substrate law plus
  caps + DataSource.
- `moof '(+ 1 2)' → 3` (printed via `$out say:`).
- `[5 proto] → Integer`. `[(fn (x) (* x x)) source] → '(fn (x) (* x x))`.
- the rust binary is ≤3k LoC.
- there is *no* path to stdout from inside moof code that bypasses
  `$out`. (test by inspecting the substrate's symbol table: no
  `print`, no `println`, no `puts`, no anything.)

approx **~1.5 weeks** of work (slightly longer than before because
caps + DataSource are real now, not deferred).

### what the smalltalk-y Transcript becomes (later, phase G)

a `Transcript` proto wraps a `$out` cap and adds:
- buffered output with periodic flush.
- per-line prefixes (timestamps, tags).
- multiple sinks (a Transcript can fan out to console + log file +
  in-world inspector view).
- the same `:say:`, `:show:`, `:cr`, `:tab` vocabulary smalltalk-80
  used.

it's a moof-side Form composing existing pieces — no substrate
addition. mentioned here so the substrate doesn't accidentally bake
in a print api that we have to deprecate later.

## phase A-self-host — parser and compiler in moof

**this phase is small but load-bearing.** once it passes, the rust
parser and compiler are *retired*. they boot the world's first
wake; thereafter, parser.moof and compiler.moof do the work, and
they are user-modifiable like everything else.

### As.1 — parser.moof

the production parser, written in moof. parses the *full* moof
surface (literals, lists, tables, send-brackets, object literals,
sigils, string interpolation, quasiquote/unquote, char literals).
emits Form-trees identical to what the bootstrap parser would
produce for the subset they share.

**tests:**
- `test_parser_moof_parses_substrate_law_corpus` — feed it the
  bootstrap reader's test corpus; assert tree equality.
- `test_parser_moof_parses_itself` — `(parse parser-source) →
  parser-form-tree`. roundtrip-able.

### As.2 — compiler.moof

the production compiler, written in moof. takes Form-trees from
parser.moof; produces chunks. supports all special forms the
bootstrap compiler did, plus:
- `defproto` with full grammar (slots, handlers, multi-clause
  patterns).
- `defop` (user-defined operatives — but use carefully; bytecode
  caching invalidates aggressively).
- pattern destructuring in `let` and method headers.
- `super` sends.

**tests:**
- `test_compiler_moof_compiles_stdlib` — feed it lib/bootstrap.moof;
  resulting chunks behave identically to what bootstrap compiled.
- `test_compiler_moof_compiles_itself` — `(compile compiler-source)
  → compiler-chunk` works.

### As.3 — switch to self-hosted on next boot

after this phase: the seed loads parser.moof and compiler.moof at
boot, *uses them for everything*. the rust parser and compiler are
quarantined behind a `--use-bootstrap-parser` debug flag, used only
for diagnosing parser.moof bugs.

**phase A-self-host acceptance gate:**
- `parse(parser-source)` and `compile(compiler-source)` both work.
- the world boots normally without the bootstrap parser/compiler
  participating.
- live-editing parser.moof or compiler.moof and re-saving works
  (modulo the chicken-and-egg: editing the *running* compiler
  requires care; documented).

approx **~1 week** of work. ~1500 LoC of moof; rust is unchanged.

## phase B — single-vat persistence

**goal:** save state per turn; reboot restores. cap effects via the
intent/receipt model. mcos for store + signing + canonical encoding.

### B.1 — vat scaffold

**files:** `src/vat.rs`, `src/scheduler.rs`. single-vat run loop.

### B.2 — message-turn ACID

journal slot mutations + input envelopes + fsync. effect-authority
reads new outbox entries.

**tests:** `test_message_turn_is_atomic`, `test_journal_round_trip`.

### B.3 — canonical encoding (as an mco)

**doc:** write `docs/reference/canonical-encoding.md` first.

**mco:** `core/canonical-encoder.mco` — rust crate compiled to
dylib. invariant: `forms_equal(a, b) ⇒ canonical_bytes(a) ==
canonical_bytes(b)`.

**tests:** `test_canonical_*` (deterministic, round-trip).

### B.4 — store (lmdb mco)

**mco:** `store/lmdb.mco` — wraps `lmdb-rkv`. exposes `Env`,
`Txn`, `Db` protos.

### B.5 — boot from store

**files:** `src/boot.rs`. read snapshot; replay input log tail;
re-fire un-receipted intents.

**tests:** `test_boot_*`.

### B.6 — first capabilities (mco-delivered)

**mcos:**
- `os/clock.mco` — `$clock` cap.
- `os/random.mco` — `$random` cap.
- `os/console.mco` — `$out`/`$err` cap.

each cap's methods become `EffectIntent`s; the rust effect-authority
in the seed reads them; calls into the mco's native methods; emits
`EffectReceipt`s.

**tests:** per-cap.

### B.7 — mark-sweep gc at turn boundaries

**files:** `src/gc.rs`. simple tracing collector.

**tests:** `test_gc_*`.

**phase B acceptance:**
- `moof '(println "hi")'` outputs `hi`, persists across reboot.
- two cold boots produce same heap-hash.
- 100 random turns + crash + reboot = exact recovery.

approx **~2 weeks**. seed grows by ≤500 LoC (vat scaffold, intent
authority, gc, scheduler). everything else is mcos and moof.

## phase C — moldability

### C.1 — proto handler mutation + IC invalidation

generation counters; IC slots store `(proto-id, gen, handler)`.

**tests:** `test_set_handler_invalidates_existing_ic`.

### C.2 — does-not-understand: hook

unrecognized selector falls through to `:does-not-understand:with:`.
default raises; user override intercepts.

**tests:** `test_dnu_*`.

### C.3 — multi-clause pattern-matched defs (in compiler.moof)

since the compiler is now in moof, this is a moof-side change.
patterns: literal, variable, wildcard, list-cons, table-positional,
table-keyed, type-guard, predicate-guard.

**tests:** `test_pattern_*`.

### C.4 — `become:`

with id-indirection: every FormId resolves through a per-vat
forwarding table. `become:` swaps two entries.

**tests:** `test_become_*`.

### C.5 — text inspector (in moof)

`[obj inspect] → string`. proto, slots, handlers reflected.

**phase C acceptance:**
- live-edit a method; next call uses new code.
- pattern destructuring works in stdlib.
- inspector produces useful output.

approx **~2 weeks**. ~+200 LoC rust (id indirection); ~1.5k LoC
moof.

## phase D — replicated vats (in-process)

**this is the load-bearing phase.** if D's gate passes, the
substrate is honest about croquet-style determinism.

### D.1 — vat mode at birth

```rust
pub enum VatMode {
  Solo,
  ReplicatedLeader  { session: SessionId, … },
  ReplicatedFollower { session: SessionId, … },
}
```

immutable.

### D.2 — determinism enforcement

implement `laws/determinism-laws.md` D3: a replicated turn cannot
read OS-bound caps. deterministic alloc order (D4): `FormId =
(turn-seq << 32) | local-counter`. ordered hashmap iteration (D5)
via `IndexMap`. gc at turn boundaries (D6). deterministic promise
ids (D7).

**tests:** `test_replicated_*`.

### D.3 — turn envelope

```rust
pub struct TurnEnvelope {
    session: SessionId,
    epoch: u32,
    turn_seq: u64,
    author: VatId,
    logical_now: i64,
    seed: u64,
    input_event: CanonicalBytes,
    signature: Signature,
}
```

inside a replicated turn, the envelope is reachable via `[turn now]`,
`[turn seed]`, `[turn author]`, `[turn input]`.

### D.4 — in-process reflector

**files:** `src/reflector.rs`. orders inputs, batches per-tick,
broadcasts envelopes, signs them.

**mco:** `core/ed25519.mco` for signing.

### D.5 — canonical hash (mco-delivered)

**mco:** `core/blake3.mco`. canonical-hash over canonical-bytes.

### D.6 — intent/receipt round trip

**tests:** `test_intent_emits_to_outbox_only`,
`test_receipt_envelope_resolves_promise_on_all_replicas`.

### D.7 — proto-edit-as-input

`{ProtoEdit target: P selector: :foo source: '(...)}` envelopes
trigger on every replica; bytecode regenerates locally via
compiler.moof.

**tests:** `test_proto_edit_envelope_recompiles_on_all_replicas`.

### D.8 — fault injection

drop a replica; rejoin from snapshot; catch up via input log.

**phase D gate:**
- 10000 random envelopes, two in-process replicas, hash-equal at
  every turn.
- no test relies on wall-clock or os entropy.
- effect intents have stable ids across replicas.

approx **~3 weeks**. seed grows by ~+800 LoC (mode logic,
determinism enforcement, in-process reflector); ~1k LoC moof.

## phase E — world-and-space single-user

**goal:** one user, one terminal, navigates a 3D world; pixmaps,
counters, scratchpads as inhabitants. canvas + pointer caps via
mcos. live-edit via inspecting forms in the world.

### E.1 — world-vat in moof

**moof:** `worlds/test-world/init.moof` — a script that creates
the root frame and a few canonical inhabitants (a Pixmap, a
Counter, a Cube).

### E.2 — Frame, Placement, Pose protos (in moof)

**moof:** `lib/world/frame.moof`, `lib/world/placement.moof`,
`lib/world/pose.moof`. quaternion + vec3 math via `core/math3d.mco`.

### E.3 — `:render-with: ctx` protocol

every form-with-a-view answers it. the wrapper vat produces
RenderContext per frame.

### E.4 — render mco (terminal half-block + braille 3D)

**mco:** `render/terminal.mco` — software 3D rasterizer outputting
braille / half-block characters. cpu-only (no gpu). good for
terminal moofpaint with low resolution.

shipping with phase E because it's the smallest renderer that
exercises the protocol.

(`render/wgpu.mco` comes phase F+ for gpu rendering.)

### E.5 — input mcos

**mcos:**
- `input/xterm-mouse.mco` — terminal mouse-event source.
- `input/xterm-keys.mco` — terminal keyboard event source.

ray-casts happen in the wrapper vat (in moof, via math3d.mco).

### E.6 — wrapper vat in moof

solo, per-replica. holds local caps, viewport, camera, render
loop. forwards pointer events to world-vat as input envelopes.

### E.7 — Pixmap proto

**moof:** `lib/inhabitants/pixmap.moof`. tools (Pencil/Eraser/...).

**mco:** `pixel-bits.mco` for fast bit-vector ops.

### E.8 — Counter, Scratchpad, Cube protos

**moof:** `lib/inhabitants/counter.moof`,
`lib/inhabitants/scratchpad.moof`, `lib/inhabitants/cube.moof`.

### E.9 — `moof world` cli

**files:** `src/main.rs` extended.

**phase E acceptance:**
- `moof world ./worlds/test-world/` opens; user navigates; edits a
  pixmap, counter, scratchpad; saves; restarts; observes restored
  state. all 3D.

approx **~3 weeks**. seed grows by ≤+200 LoC (CLI + boot); ~3k
LoC moof + ~3 new mcos (~700 LoC rust each).

## phase F — multi-user world over websocket

**goal:** alice and bob share a world. websocket transport.
presence (cursors). leader failover. live-edit propagation.

### F.1 — transport mco

**mco:** `transport/websocket.mco` — wraps `tungstenite`.

### F.2 — handshake / authentication

ed25519 signatures per `concepts/transport.md`.

### F.3 — snapshot transfer

http endpoint or chunked websocket frames.

### F.4 — reconnect with epoch

### F.5 — leader failover

### F.6 — Cursor inhabitant in moof

`lib/inhabitants/cursor.moof`. presence as a first-class Form.

### F.7 — `moof world join wss://...`

**phase F gate:**
- two terminals, two processes. alice draws on a pixmap; bob sees
  within 50ms. bob disconnects mid-stroke, reconnects, converges.
- alice live-edits the Pencil; bob's pencil changes the very next
  send.
- close both, reopen; both wake to the same world.

approx **~3 weeks**. seed unchanged; ~+1 mco (transport); ~1.5k
LoC moof.

## phase G — gpu rendering, web canvas, polish

- **`render/wgpu.mco`** — gpu-backed renderer. same `:render-with:
  ctx` protocol; targets a wgpu surface.
- **`render/web.mco`** — runs in browser via wasm. web canvas as a
  second viewport.
- **`format/png.mco`** — pixmap export.
- **session persistence as on-disk artifact** — shareable worlds.
- **user identity** — long-lived ed25519 keys, identity-as-form.

## phase H+ — beyond

deferred until needed:

- types (nominal + structural in moof; refinement deferred).
- datalog (list-comprehensions only for a long time).
- APL Tables (Table is `Vec + IndexMap` for now).
- federation across worlds.
- package format.
- physics inhabitants.
- VR/gamepad input mcos.

## perf budget for the moofpaint demo

| operation | budget | notes |
|---|---|---|
| send dispatch (IC hit) | < 100 ns | seed fast path |
| canvas refresh (full) | < 16 ms | 60Hz target |
| reflector tick | 50 ms | configurable |
| cold boot | < 500 ms | including snapshot mmap |
| input → render latency | < 100 ms | input → reflector → broadcast → render |
| journal fsync per turn | < 5 ms | lmdb write txn |
| mco load (warm) | < 10 ms | dylib already cached |
| mco load (cold) | < 100 ms | first dlopen |

if any budget is exceeded by 2×, profile + optimize *or* adjust
budget + document why.

## risks to watch

1. **seed creep.** every "this would be easier in rust" is a step
   back from the maru posture. require justification.
2. **canonical encoding determinism.** every refactor of the
   `core/canonical-encoder` mco needs adversarial tests.
3. **bytecode invalidation correctness.** when source changes, all
   dependent bytecodes must regenerate. cache key: `(form-id,
   source-content-hash)`.
4. **mco abi stability.** breaking the native ABI requires
   rebuilding all mcos. version the abi.
5. **leader failover on a busy session.** in-flight intents +
   uncommitted state interacts hairily. prefer "let recent intents
   replay; idempotence saves us."
6. **the reflector becoming a trust bottleneck.** keep it small.
   if it's > 500 LoC of rust, something has crept in.
7. **ambient cap leakage.** an os-bound cap captured in a closure
   sent into a replicated vat is silent corruption. detect and
   refuse.
8. **self-hosting bug.** if compiler.moof has a bug that miscompiles
   itself, you can't fix it from inside. mitigation: keep the
   bootstrap rust compiler buildable behind a flag forever.

## tooling for the substrate sprint

- **a "law tester" binary**: takes a vat directory, runs each
  substrate-law assertion.
- **a reproduction harness**: given a session-id, replay the input
  log on a fresh vat, verify hash matches recorded. for ci.
- **a fuzzer** against the canonical encoder + the vm.
- **an mco builder** (`moof mco build`) that produces multi-platform
  mcos from a rust crate.

## rough overall budget

| phase | wall-clock | rust LoC delta | moof LoC delta | mcos added |
|---|---|---|---|---|
| A | 1 wk | +2.5k | +0.3k | (1 demo) |
| A-self-host | 1 wk | 0 | +1.5k | 0 |
| B | 2 wk | +0.5k | +0.5k | 4 (encoder, lmdb, clock, random, console) |
| C | 2 wk | +0.2k | +1.5k | 0 |
| D | 3 wk | +0.8k | +1k | 2 (blake3, ed25519) |
| E | 3 wk | +0.2k | +3k | 4 (terminal-render, xterm-mouse, xterm-keys, pixel-bits, math3d) |
| F | 3 wk | 0 | +1.5k | 1 (websocket) |
| **total** | **~15 wk** | **+4.2k** | **+9.3k** | **~12 mcos** |

note: the rust line stops growing significantly after phase D. all
phase E/F/G work happens in moof + new mcos. the seed converges to
its terminal size.

## see also

- `roadmap.md` — phase summary.
- `audit-2026-04-29.md` — why this plan looks the way it does.
- `state-of-the-implementation.md` — what we left behind.
- `concepts/world-and-space.md` — the 3D world primitive.
- `concepts/pixmap.md` — the canonical inhabitant.
- `concepts/compiled-objects.md` — what mcos do.
- `laws/determinism-laws.md` — the test gate at phase D.
