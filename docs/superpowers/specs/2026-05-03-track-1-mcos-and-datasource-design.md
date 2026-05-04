# track 1 — richer mco abi, datasource as universal protocol, polyglot std lib

> **session mission: take wasm-mcos from "proof of life" to "production-
> shaped." richer signatures (ints, bytes, strings, raise), real
> moof-namespace imports, the `$mco` cap proper, content-addressed
> cache, an embedded-bytes-as-trust hash bootstrap, the std lib's first
> wave of mcos integrated as cap-mediated artifacts (not as
> `examples/` curiosities), the DataSource protocol extended for
> infinite-source-shaped values (Clock, Random), and four working
> language toolchains targeting the same content-addressed wasm
> artifact format.**

---

## the load-bearing decisions

### LB-1 — mcos are pure behavior

**mcos do not participate in the moof heap.** their only interaction
with moof is through method args and method returns. the wasm import
surface lets the mco ask the moof world to allocate things (a String,
a Bytes buffer, a Symbol) or signal events (raise an error); it does
*not* let the mco read or mutate moof Forms.

an mco's state-as-it-sees-it lives entirely in its own wasm linear
memory. an mco's state-as-moof-sees-it lives in the slots of the
proto-Form moof binds it to — and that slot management is moof's job,
not the mco's. this is what justifies keeping the import surface
minimal *forever*, not merely for first-wave: there is never a reason
for the import surface to grow into Form mutation.

deferred imports (`moof_slot`, `moof_slot_set`, `moof_send`,
`moof_proto_of`) are deferred not because they're hard but because LB-1
says they're the wrong shape.

### LB-2 — datasource has an infinite-source subclass

`docs/concepts/data-sources.md` describes pull-based, advancing-
cursor sources (files, sockets, query results). Clock and Random fit
oddly: neither has `:done?`, neither has eof, neither advances along a
prerecorded sequence. the right amendment is to add the **infinite
source** subclass:

> **an infinite source is a DataSource that never terminates.**
> `:done?` always returns false. `:close` is a no-op. `:next` always
> succeeds.
>
> two flavors share the contract:
>
> - **polled** (Clock-like): `:next` reads environment state.
>   `:peek == :next` (idempotent, no internal state).
> - **generator** (Random-like): `:next` advances internal state and
>   returns. `:peek` stashes one value to return on next `:next` (or
>   computes-one-step-ahead semantics — implementation chooses).

both pass the same protocol-conformance test except for the `:peek`
discipline. combinators (`:take:`, `:for-each:`, `:throttle:`,
`:ticks:`) work on either flavor uniformly.

### LB-3 — symbolic names resolve through a moof-side index

the substrate handles bytes. `lib/mcos/index.moof` is a moof Table
mapping symbolic names (`"core/clock"`) to content-hashes
(`"7f3a2c…"`). `[$mco load: "core/clock"]` does the lookup in moof,
fetches bytes by hash from `.moof/mcos/<hash>.mco`, verifies hash,
hands bytes to the substrate's primitive `__instantiate-mco-bytes`.

zero substrate code knows about symbolic names or about file paths.
that's a moof concern.

### LB-4 — embedded-bytes-as-trust for the hash bootstrap

content-addressing requires hashing. the canonical hash is blake3.
blake3 ships *as an mco* (`core/hash`, written in zig from scratch).
to break the chicken-and-egg, the substrate `include_bytes!`'s the
canonical Hash mco at compile time and instantiates it on boot
*without verification* — its provenance is the substrate binary's own
provenance.

from that moment, `$hash` exists. every other mco load uses
`[$hash of: bytes]` to verify the cache filename matches its
contents. there is exactly one privileged moment in the mco lifecycle
(the first instantiate-from-embedded-bytes); everything else is hash-
verified.

this generalizes: any "load-bearing for the loader itself" mco we
later identify can use the same pattern. for now Hash is the only
one.

### LB-5 — no eager binding

`lib/main.moof` does not eager-bind any mco. `Clock`, `Random`,
`Base64`, `Utf8`, `Url`, `Date` are not in the global env unless user
code (or a world manifest in phase B+) does
`(def Clock [$mco load: "core/clock"])` itself.

three reasons:
- **cap-discipline**: making Clock a bare global is the same category
  error as making `$out` a bare global.
- **uniformity** with moof-bytecode mcos: stdlib moof code wants
  load-on-demand; one rule should cover both wasm and bytecode mcos.
- **image-phase neutrality**: once images land, eager-vs-lazy
  distinction dissolves — once-loaded protos are heap values that
  persist. only fresh-world first-boot is affected.

REPL ergonomics: a separate `lib/repl-init.moof` (or `~/.moofrc`) does
the eager-binds for interactive sessions. that's a shell concern,
distinct from the substrate.

### LB-6 — language slate by toolchain ergonomics

prefer languages where you can throw a few files together and run a
compile script. avoid languages whose toolchains demand
package-manager-and-lockfile ceremony for what should be a direct
compilation. session slate:

| lang | toolchain | install | role this session |
|---|---|---|---|
| zig | `zig build-exe -target wasm32-freestanding` | already on system | majority of mcos |
| c | `clang --target=wasm32-freestanding -nostdlib` | system clang | one independent-impl mco |
| ocaml | `wasm_of_ocaml` via opam | `opam install wasm_of_ocaml-compiler` (after `binaryen` dep) | one mco |
| haskell | ghc-wasm via ghcup-cross | best-effort, may defer | one mco if toolchain cooperates |

**explicitly excluded this session:** rust. the `cargo`+crates ceremony
fights the "files + compile script" criterion. `crates/abi-rust/`
remains for users who want it; no rust mco ships in this session's
slate.

future polyglot candidates (not this session, named for trajectory):
Grain, TinyGo, Nim, AssemblyScript.

---

## the three-tier mco model

every method on every mco proto has an implementation that lives in
exactly one of three tiers. the .mco container format is designed to
carry methods of any tier (and mixes); **this session ships only
tier 2**, but the format must not paint us into a tier-2-only corner.

### tier 1 — inline natives

source-level `(native LANG …)` forms compiled into the substrate
binary itself. not artifacts. not loaded — built-in. used for the
primitives the substrate can't live without: `Cons:car`, `Integer:+`,
`Method:call`, the Heap / Chunks singletons. these exist today; this
session leaves them alone.

### tier 2 — wasm mcos (this session)

canonical case. wasm bytecode in the .mco's wasm preamble; methods
exported by name; manifest's `methods.[].impl` is `#wasm-export
"name"`. **platform-independent**. wasmtime AOT-compiles to native at
install or first-load time (cached as `cwasm` next to the `.mco` in
`.moof/mcos/cache/`); same artifact runs at near-native speed
everywhere with zero platform variants. AOT is a pure cache
optimization — no manifest changes, no artifact changes; just an
opt-in flag in the cache layout.

### tier 3 — native dylib mcos (deferred — format accommodates now)

reach for it only when AOT-compiled wasm is genuinely insufficient:
gpu shaders, mmap shared memory with another process, syscalls wasm
can't make. the .mco container shape stays the same (manifest,
content-hash, signature, deps) but the payload is platform-specific.

**format implications, designed now, implemented later:**

- the .mco's wasm preamble may be a near-empty wasm module (still
  legal wasm; exports nothing). the wasm portion is a placeholder for
  tier-3-only mcos.
- a new `moof.native` custom section carries:
  - `target-platforms`: list of host triples the artifact supports
    (eg `["aarch64-apple-darwin", "x86_64-linux-gnu"]`)
  - either embedded payload bytes (whole dylib in a custom section)
    OR a sibling-file reference (`<hash>.dylib` next to `<hash>.mco`
    in the cache directory; signed under the same `moof.signature`)
- manifest's `methods.[].impl` grows a third option:
  - `#wasm-export "name"` (tier 2)
  - `#moof-bytecode <chunk-id>` (pure-moof, track 3)
  - `#native-symbol "name"` (tier 3 — `dlsym` in the loaded dylib)
- manifest gains optional `tier`: `'wasm | 'native | 'mixed`.
  **mixed mcos** blend wasm and native methods on the same proto —
  useful for "fast path in native, derived methods in wasm" hybrids.
- loader checks `target-platforms` for tier-3 mcos at load; raises
  `'platform-unsupported` if the host isn't covered.
- content-hash for tier-3 mcos differs per platform variant. that's
  honest: `core/blake3-gpu@<arm64-mac-hash>.mco` and
  `core/blake3-gpu@<x86-linux-hash>.mco` are *different artifacts*
  with different hashes, even though they implement the same proto.
  symbolic-name resolution (LB-3) handles this transparently — the
  index can list multiple hashes per name, the loader picks the one
  matching the current platform.

this session: **tier 2 only**. the loader's first-wave behavior:
sees a `moof.native` custom section ⇒ raises
`'tier-3-not-supported`. format reservations land in `manifest.moof`
schema (the `tier` field accepts only `'wasm` for now; the `impl`
field accepts only `#wasm-export` and `#moof-bytecode`). when tier 3
lands as its own session, the format already supports it; only
loader code (dlopen path + dlsym dispatch) and the mixed-tier
trampoline plumbing need to change.

formal schema amendment to `docs/reference/mco-format.md` for tier 3
lands with that session — this section is the forward-looking
sketch.

---

## architecture

three artifacts, one pipeline, three substrate amendments.

```
┌──────────────────┐     ┌────────────────────┐     ┌──────────────────┐
│ lib/mcos/<name>/ │     │ .moof/mcos/cache/  │     │ user code        │
│  source files    │ ──▶ │  <hash>.mco        │ ──▶ │  (def Clock      │
│  manifest.moof   │     │  (content-         │     │   [$mco load:    │
│  build.sh        │     │   addressed)       │     │    "core/clock"])│
└──────────────────┘     └────────────────────┘     └──────────────────┘
   build phase                cache phase                  load phase
   (per-mco script)           (substrate-managed)          (cap-mediated)
```

substrate amendments:

1. **wasm trampoline (`crates/substrate/src/wasm.rs`)** — grows from
   `() → i64` to handle int args, byte-buffer in/out, string in/out,
   raise propagation, signature introspection. ~+250 LoC.
2. **`$mco` cap** — replaces `__loadWasmMco`. moof-side singleton
   cap with `:load:`, `:loadByHash:`, `:describe:`. backed by a
   single rust primitive: `__instantiate-mco-bytes`.
3. **embedded Hash mco** — `include_bytes!` in substrate; instantiated
   pre-bootstrap.moof; provides `$hash` as a primordial cap.

moof-side additions:

1. **`lib/mcos/<name>/`** dirs for each mco (sources, manifest, build script, tests).
2. **`lib/mcos/index.moof`** — symbolic name → content-hash table.
3. **`lib/stdlib/data-source.moof`** — moof-side default methods for
   the infinite-source subclass.
4. **`docs/concepts/data-sources.md`** — amendment for infinite source.
5. **`docs/reference/native-abi.md`** (new) — language-neutral ABI spec.

---

## components

### C-1 — wasm trampoline

three subcomponents:

**handle table** — `Vec<Value>` per-instantiation. wasm-side u32
indexes in. allocated on dispatch entry; drained on dispatch exit
(including on raise paths via rust drop guards). Forms always cross
as handles. *no callbacks back into wasm allowed in first wave* —
this is what makes lifetimes trivial.

**arg/return marshaler** — introspects wasm function signature post-
`Instance::get_func`. shapes covered first wave:
- `i32` / `i64` ↔ Integer (auto-promoting if BigInt range needed)
- `u32 handle` ↔ Form / String / Bytes / Symbol

return-value: a single u32 handle. byte-buffer or string returns are
constructed inside wasm (via `moof_make_string` or `moof_make_bytes`
imports), then the handle is returned.

signature mismatch raises `'arity-mismatch` at load time, not at
call time — the trampoline introspects at instantiate-time.

**import surface (first-wave)** — six imports, all under the `moof`
wasm namespace:

```
moof_raise(kind_handle: u32, msg_ptr: u32, msg_len: u32) -> noreturn
moof_make_string(ptr: u32, len: u32) -> u32
moof_make_bytes(ptr: u32, len: u32) -> u32
moof_string_text(handle: u32, buf: u32, cap: u32) -> u32
moof_bytes_data(handle: u32, buf: u32, cap: u32) -> u32
moof_intern(ptr: u32, len: u32) -> u32
```

semantics:
- `moof_make_string`/`moof_make_bytes`: copy bytes from wasm linmem
  into a moof-heap String/Bytes. returns a handle.
- `moof_string_text`/`moof_bytes_data`: copy bytes from a moof-heap
  String/Bytes into wasm linmem at `buf`, capped at `cap`. returns
  actual length. wasm allocates the buffer.
- `moof_raise`: trap with payload `(kind, msg)`. trampoline catches
  and converts to a moof RaiseError.
- `moof_intern`: register a Symbol from utf-8 bytes; return handle.

**deferred (LB-1):** `moof_slot`, `moof_slot_set`, `moof_proto_of`,
`moof_send`. mcos that need these are themselves the wrong shape.

### C-2 — `$mco` cap

singleton cap installed via `lib/mcos.moof` (new). backed by *one*
new rust primitive on `intrinsics.rs`:

- `__instantiate-mco-bytes(bytes) → proto-Form` — verifies wasm magic,
  parses custom sections, instantiates wasm + WASI, allocates fresh
  proto-Form, installs handlers from manifest. roughly today's
  `load_wasm_mco` rebound to take bytes instead of path.

three moof-side methods on `$mco`:

- `:load: name` — name → hash via index lookup, hash → bytes via
  `[$io readBytes: ".moof/mcos/<hash>.mco"]`, verifies
  `[$hash of: bytes]` matches expected, calls
  `[__instantiate-mco-bytes bytes]`. caches loaded protos by hash for
  identity-on-second-load.
- `:loadByHash: hash` — bypasses the resolver.
- `:describe: name-or-hash` — instantiates only the manifest custom
  section; returns the manifest as a moof Form. for tooling.

retires `__loadWasmMco` entirely.

### C-3 — Hash bootstrap

substrate compile-time machinery:

```rust
// in crates/substrate/src/lib.rs (or a new bootstrap.rs)
const HASH_MCO_BYTES: &'static [u8] =
    include_bytes!("../../../lib/mcos/hash/hash.mco");

// during world::new, before bootstrap.moof eval:
let hash_proto = instantiate_mco_bytes(world, HASH_MCO_BYTES)?;
world.set_global("$hash", hash_proto);
```

build orchestration: `lib/mcos/hash/build.sh` produces
`lib/mcos/hash/hash.mco`. cargo's `build.rs` for the substrate crate
checks the file exists AND that its hash matches a build-time-
recorded expected hash (stored in
`lib/mcos/hash/hash.expected-hash`, updated by `build.sh` at every
successful build). failing either check fails the substrate build.

**the very-first-build path** (no substrate yet, no `$hash` mco
to verify itself): `build.sh` uses an **external blake3 utility**
(homebrew package `b3sum`) for hash computation. this is the only
piece of build-side hashing that doesn't go through the moof Hash
mco. once substrate is built, runtime `$hash` takes over for
verification at every load. blake3 being deterministic guarantees
external-tool hash and mco hash always agree on identical bytes; a
tier-3 test asserts this on a corpus.

one-time bootstrap when first cloning:

```bash
brew install b3sum                      # if not already
./lib/mcos/hash/build.sh                # produces hash.mco + hash.expected-hash
cargo build --workspace                 # substrate embeds hash.mco
```

if substrate ships pre-built (in a release tarball), the embedded
bytes are already there and the user never needs zig or b3sum.

### C-4 — DataSource infinite-source

three pieces:

1. **doc amendment** at `docs/concepts/data-sources.md`: a new
   "infinite sources" section after "laziness". defines the subclass,
   the polled/generator flavors, the conformance test signature.

2. **moof-side default methods** at `lib/stdlib/data-source.moof`
   (new):
   - `:done?` defaults to `#false` for any proto declaring infinite-
     source membership (a meta-slot, e.g., `:infinite-source #true` on
     the proto)
   - `:peek` default for polled flavor delegates to `:next`
   - `:peek` default for generator flavor uses a one-element stash
     (lives in instance state)
   - `:close` default is no-op
   - `:take: n` consumes n values into a Cons
   - `:for-each: blk` infinite-loops; the consumer's job to break out
   - `:ticks: dur` (specced; deferred-implementation if `[$scheduler …]`
     not yet available)

3. **`:ticks:` combinator semantics** (specced even if deferred): wraps
   any infinite source in a sampler that emits the source's `:next`
   value every `dur`. used as `[Clock ticks: 1s]`.

### C-5 — per-mco directory shape

every `lib/mcos/<name>/` has exactly four kinds of file:

- **source** (`<name>.zig` / `<name>.c` / `<name>.ml` / `<name>.hs`)
- **`manifest.moof`** — moof Form: parent, methods, imports-cap,
  imports-mco, arity per method
- **`build.sh`** — compiles, packs, hashes, caches, indexes
- **`<name>.test.moof`** (optional but expected) — moof unit tests

uniformity here is what makes adding a 5th language mechanical: same
four files, just a different toolchain inside `build.sh`.

---

## data flow

### DF-1 — build flow (per mco)

```
$ ./lib/mcos/random/build.sh
  │
  ├─ <toolchain> source → random.wasm        # only language-specific line
  ├─ moof mco pack random.wasm \             # shell tool, calls mco-pack crate
  │       --manifest manifest.moof \
  │       --output random.mco
  ├─ HASH=$(b3sum random.mco | cut -d' ' -f1)  # external blake3 utility (always)
  ├─ mv random.mco $MOOF_CACHE/$HASH.mco
  └─ moof mco index-update "core/random" $HASH
```

15 lines per script; the only language-specific line is the first.

### DF-2 — load flow

```
substrate process starts
  │
  ├─ wasmtime engine + WASI ctx initialized
  ├─ embedded Hash mco bytes instantiated     (privileged, no verify)
  │       └─ $hash bound as a global before bootstrap.moof runs
  ├─ bootstrap.moof eval'd                    (existing flow)
  │       └─ at end, lib/main.moof eval'd
  │               └─ (def $mco …) installs the cap
  │               └─ NO mcos eager-bound (LB-5)
  │
REPL ready
```

per-load:

```
[$mco load: "core/random"]
  │
  ├─ name → hash       lookup in lib/mcos/index.moof
  ├─ hash → bytes      [$io readBytes: ".moof/mcos/<hash>.mco"]
  ├─ verify hash       [$hash of: bytes] == hash, else 'hash-mismatch
  ├─ check load cache  if hash already loaded, return cached proto
  └─ instantiate       [__instantiate-mco-bytes bytes] → proto-Form
                       └─ side effect: cache hash → proto for re-use
```

### DF-3 — dispatch flow

```
[Base64 encode: bytes-value]
  │
  ├─ Base64 proto resolves :encode: handler
  ├─ handler is a wasm-trampoline closure (instance, fn-name, sig)
  │
  ├─ trampoline entry:
  │   ├─ allocate per-call handle table
  │   ├─ marshal args: bytes-value → handle table → u32
  │   ├─ wasmtime call instance->encode(handle)
  │
  │   wasm side runs:
  │     ├─ moof_bytes_data(handle, scratch, cap)     # read input into linmem
  │     ├─ <encoding>                                 # pure compute in linmem
  │     ├─ moof_make_string(out_ptr, out_len)        # alloc result in moof heap
  │     └─ return new u32
  │
  │   ├─ unmarshal return: u32 → handle table → moof Value
  │   ├─ drain handle table (drop temp handles, keep returned)
  │
  └─ return moof Value
```

handle table opens on dispatch entry, closes on dispatch exit. no
callbacks-into-wasm allowed.

### DF-4 — error flow

```
wasm calls moof_raise(kind, msg_ptr, msg_len)
  │
  ├─ rust:
  │   ├─ resolve kind handle → Symbol
  │   ├─ copy msg bytes from linmem → moof String
  │   └─ wasmtime::Trap with custom payload (kind, msg)
  │
  ├─ wasmtime unwinds wasm stack
  ├─ trampoline catches Trap, drains handle table
  ├─ trampoline converts payload → moof RaiseError
  └─ propagates to nearest [try …] / [catch: …]
```

failure shape is exactly today's moof raise. wasm just gets a way to
participate.

---

## implementation order

**ordering principle**: each step bounded by prior step's risk. ABI
surface grows naturally with consumer demand.

1. **frontload**: 1-page ABI skeleton at
   `docs/reference/native-abi.md` listing first-wave imports with
   signatures. paragraph-level addition to `data-sources.md` for
   infinite source. ~30 minutes; iterated as we go.

2. **Random (zig)** — pure compute. tests new format / loader / cache
   / resolver / hash bootstrap end-to-end with minimum ABI
   (`(seed: u64) → u64`, `(state: u64) → u64`). retires
   `__loadWasmMco` for `[$mco load: "core/random"]`. *forcing
   function: cache + resolver + bootstrap all work.*

3. **Clock (zig, migrate)** — drives DataSource state-source spec.
   WASI calls only; no new moof imports. `[Clock now]`,
   `[Clock monotonic]`, `:next`, `:peek`, `:done?`, `:ticks: 1s`
   (specced, deferred-impl). *forcing function: infinite-source
   semantics in moof.*

4. **Base64 (zig)** — exercises byte-buffer marshaling both
   directions; raises on bad input. `[Base64 encode: bytes]`,
   `[Base64 decode: string]`. *forcing function: bytes ABI half +
   `moof_raise` import.*

5. **Utf8 (c)** — second language. `[Utf8 valid?: bytes]`,
   `[Utf8 codepoints: bytes]`, `[Utf8 length: bytes]`. *forcing
   function: ABI doc validates against an independent C
   implementation.*

6. **Hash (zig)** — blake3 from scratch. dogfood: substrate uses our
   own implementation. enables the LB-4 bootstrap. *forcing function:
   embedded-bytes-as-trust path is real.*

7. **Url (ocaml)** — third language. RFC 3986 parser; returns Forms.
   `[Url parse: "https://example.com/foo?bar"]` →
   `{scheme: 'https, host: "example.com", path: "/foo", query: …}`.
   *forcing function: ABI validates against a non-c-family lang.*

8. **Date (haskell)** — fourth language, IF toolchain. takes ns
   timestamp, returns date Form. consumer of Clock. *forcing function:
   ABI validates against pure-functional lang.*

ladder rungs by completion (counting languages: zig=Random/Clock/Base64/Hash, c=Utf8, ocaml=Url, haskell=Date):

| through step | session classification |
|---|---|
| 1–4 | track-1 ABI complete; one language (zig) |
| 1–5 | + Utf8 (c) — two-language polyglot start |
| 1–6 | + Hash (zig) — content-addressing dogfood; still two langs |
| 1–7 | + Url (ocaml) — three-language polyglot |
| 1–8 | + Date (haskell) — four-language polyglot, the demo shape |

below rung 4 the session is "we learned but didn't ship" — also fine.

---

## error handling

four error origins, one propagation path.

### load-time errors

| kind | meaning | recovery |
|---|---|---|
| `'unknown-mco` | name not in `lib/mcos/index.moof` | typo or mco not built |
| `'mco-not-cached` | hash listed but no cache file | run `lib/mcos/<name>/build.sh` |
| `'hash-mismatch` | cache file's hash ≠ index's hash | tampering or stale cache |
| `'bad-mco` | wasm validation failed / manifest malformed | corrupted artifact |
| `'abi-mismatch` | manifest abi-version > substrate's | substrate too old, mco too new |

### dispatch-time errors

| kind | meaning |
|---|---|
| `'arity-mismatch` | moof passes wrong arg count (caught at marshal time) |
| `'type-mismatch` | wrong type for arg slot |
| `<user-defined>` | mco called `moof_raise` with its own kind |
| `'wasm-trap` | unreachable / memory fault (mco bug) |

handle table is drained whether dispatch returns normally OR via raise
— rust's drop semantics on a guard struct ensure no leaks. *test (L3):
induce raise mid-call; assert handle table empty afterward.*

### build-time errors

`build.sh` exits nonzero. nothing else happens — no cache mutation,
no index mutation. dev sees toolchain output verbatim.

### bootstrap errors

embedded Hash mco fails to instantiate ⇒ substrate panics with clear
"shipped substrate is broken" message. unrecoverable; user
re-installs moof. cargo's `build.rs` guards against the most common
form (missing `lib/mcos/hash/hash.mco`).

---

## testing

three levels, mirror the components. (avoiding "tier" here since it
means "mco implementation strategy" elsewhere in the doc.)

### L1 — per-mco unit tests

`lib/mcos/<name>/<name>.test.moof`:

```moof
;; lib/mcos/random/random.test.moof
(def Random [$mco load: "core/random"])
(def $rng [Random seedFrom: 42])
(test "deterministic from seed"
  (let a [$rng next] b [$rng next] c [$rng next])
  (assert= a 12345678901234N)
  (assert!= a b)
  (assert!= b c))
(test "take: returns n values"
  (let stream [$rng take: 5])
  (assert= [stream length] 5))
```

run by `cargo test --workspace` via an integration runner that walks
`lib/mcos/*/<name>.test.moof` and evals each. *forcing function: every
mco must have at least one L1 test.*

### L2 — protocol conformance

`lib/stdlib/data-source.test.moof` runs against any infinite source:

```moof
(defn assert-infinite-source-polled [ds]
  (assert (not [ds done?]))
  (assert= [ds peek] [ds peek]))   ; peek idempotent for polled

(defn assert-infinite-source-generator [ds]
  (assert (not [ds done?]))
  (let p [ds peek])
  (assert= p [ds next]))           ; peek's value comes back on next

(test "Clock conforms to polled infinite source"
  (assert-infinite-source-polled Clock))
(test "Random conforms to generator infinite source"
  (assert-infinite-source-generator [Random seedFrom: 0]))
```

new mcos that claim a protocol must pass conformance. *forcing
function: protocol changes are caught here, not at the use-site of
one specific source.*

### L3 — substrate integration

extends `crates/substrate/tests/wasm_mco.rs`:

- pack / load / hash-roundtrip
- corrupted-bytes load fails with `'bad-mco`
- tampered-cache load fails with `'hash-mismatch`
- handle-table balance assertion (instrument trampoline with debug
  counter; assert zero after every test)
- ABI doc coverage: a parser of `docs/reference/native-abi.md`
  extracts every import name; assert each has at least one mco using
  it (catches doc-vs-reality drift)

**test count target:** existing 351 + ~30 (5 new mcos × ~5 tests at L1) +
~10 (L2 protocol + L3 integration) ≈ **~390 tests passing** by session-end.

---

## the abi reference doc

`docs/reference/native-abi.md` (new) is the language-neutral spec.
**every language binding** (`moof.zig`, `moof-abi-rust`, `moof.ml`,
`moof.hs`, future `moof.c` / `moof.go` / `moof.gr`) references this
doc as source-of-truth, *not* each other.

structure:
- preamble: "this is the contract every wasm mco speaks. four
  languages currently bind it."
- imports surface (one section per import, each with: name, signature,
  semantics, error semantics, example wasm-text)
- export conventions (function names, argument shapes, return shapes)
- handle layout (u32 indices, lifetime rules, "no callbacks back into
  wasm")
- error model (raise → trap → moof RaiseError)
- abi-version negotiation (manifest field, substrate compatibility
  range)
- per-language binding pointers (each binding gets one paragraph
  pointing at its source location and noting any language-specific
  ergonomics)

doc-vs-reality drift is caught by the L3 ABI-coverage test.

---

## what is NOT in scope this session

| deferred | why |
|---|---|
| `moof_slot` / `moof_slot_set` / `moof_send` imports | LB-1 says wrong shape for any current mco; revisit only if a real consumer demands it |
| Form mutation from wasm | same |
| ed25519 signing of mcos | track 2; not blocking polyglot |
| `moof.deps` resolution | track 2; first-wave mcos have no deps |
| `moof mco install <hash>` (peer fetch) | track 2 / phase F |
| dylib mcos (tier 3 implementation) | format accommodates them per the three-tier model section; loader + trampoline plumbing waits for its own session |
| sandboxed wasi modes | phase G |
| parser.moof | N+2 |
| moof-bytecode-only mcos (pure-moof shipped as .mco) | track 3; this session is wasm-payload mcos |
| `[Clock ticks: 1s]` actual implementation | requires `[$scheduler after: …]`; specced but deferred-impl |

---

## risk register

ranked by likelihood × impact:

1. **haskell-wasm toolchain rabbit hole.** ghcup-cross is experimental;
   nix path requires installing nix; tweag/ghc-wasm-meta is a flake.
   *probability: high. impact: medium.* mitigation: drop haskell from
   slate; ship 1–7 (three-language polyglot). spec doc keeps haskell
   nominated for N+1.

2. **`build.sh` orchestration drift across languages.** each script
   has the same shape but four different toolchains. inconsistencies
   creep. *probability: medium. impact: low.* mitigation: a shared
   `lib/mcos/_lib/pack-and-cache.sh` helper that the per-language
   `build.sh` files invoke after producing `<name>.wasm`.

3. **handle-table lifecycle bugs on raise paths.** if rust's drop
   guard ever doesn't run, handles leak. *probability: low. impact:
   high.* mitigation: L3 balance-assertion test runs after every
   integration test.

4. **DataSource infinite-source spec is shaped to Clock+Random
   only.** the polled/generator distinction may not generalize.
   *probability: medium. impact: medium.* mitigation: the spec
   amendment is deliberately small (~50 lines); easy to revise when a
   third infinite-source candidate (id-mints, fibonacci sequences)
   teaches us what generalizes.

---

## session-end deliverables checklist

(in roughly the order they land)

- [x] `docs/reference/native-abi.md` exists (frontloaded; iterated)
- [x] `docs/concepts/data-sources.md` has infinite-source amendment
- [x] `crates/substrate/src/wasm.rs` trampoline grows arg/return marshaling + 6 imports
- [x] `lib/mcos/index.moof` exists with name → hash entries (schema accommodates multi-hash-per-name for future tier-3 platform variants)
- [x] `lib/mcos.moof` defines `$mco` cap with `:load:`, `:loadByHash:`, `:describe:`
- [ ] manifest schema reserves the `tier` field (accepts `'wasm` only this session) and the `methods.[].impl` field accepts `#wasm-export` and `#moof-bytecode`; tier-3 variants are reserved for future expansion — **deferred to N+2; manifests ship without explicit `tier` field (loader is wasm-only by construction)**
- [ ] loader rejects `moof.native` custom section presence with `'tier-3-not-supported` — **deferred to N+2; loader silently ignores unknown custom sections (harmless for now; no tier-3 mcos exist)**
- [x] `crates/mco-pack` extends with `pack`, `index-update` subcommands invoked by per-mco `build.sh` scripts
- [x] `__loadWasmMco` retired
- [x] `lib/mcos/hash/{hash.zig, manifest.moof, build.sh, hash.test.moof}` ships; bytes embedded in substrate via content-hash cache path (`.moof/mcos/cache/<hash>.mco`); `build.rs` guards the embed
- [x] `lib/mcos/random/` (zig) + DataSource test conformance
- [x] `lib/mcos/clock/` (zig, migrated) + DataSource test conformance
- [x] `lib/mcos/base64/` (zig) + raise-on-bad-input test
- [x] `lib/mcos/utf8/` (c) + clang→wasm32 build script
- [ ] `lib/mcos/url/` (ocaml) — **deferred: wasm_of_ocaml uses GC-wasm (js_of_ocaml lineage); incompatible with linear-memory ABI. needs deliberate toolchain choice (wasm32-wasi target via custom GC-off build or switch to a different FFI surface)**
- [ ] `lib/mcos/date/` (haskell) — **deferred: ghc-wasm-meta flake / tweag path required; best-effort attempt not completed in session**
- [x] `lib/stdlib/data-source.moof` (new) with infinite-source defaults
- [x] `lib/repl-init.moof` (new) eager-binds for REPL ergonomics
- [ ] cargo test --workspace ≈ 390 tests passing — **actual: 368 tests passing** (target was ~390; gap is OCaml + Haskell mcos + their tests not landing)
- [x] all changes committed in atomic commits with passing tests

**summary: 15/20 delivered. 2 deferred to N+2 (tier-3 format guards); 2 language mcos deferred (OCaml, Haskell — toolchain ABI mismatch); test count 368 vs ~390 target (gap tracks missing mcos).**

---

## post-session trajectory

| session | scope | end-state |
|---|---|---|
| **N+1 (this one)** | track 1 ABI + DataSource + 4-lang polyglot | std lib has 6 mcos in 3-4 langs |
| **N+2** | track 2: ed25519 signing, `moof.deps`, peer-fetch | mcos are nix-store-grade artifacts |
| **N+3** | parser.moof — port `reader.rs` to moof | parser is moof; phase A self-host done |
| **N+4** | track 3: more polyglot (Grain, TinyGo, Nim) + moof-bytecode `.mco`s | 6+ langs; pure-moof `.mco`s ship |

four sessions to "the polyglot story is real and prod-shaped."

---

## appendix — committed positions and deferred questions

decisions made during brainstorming, locked here so the implementation
plan doesn't re-litigate them:

1. **`[$io readBytes:]` is rust-direct**, not WASI-routed. the
   substrate's cache management is privileged and shouldn't share an
   i/o model with sandboxed mcos. WASI is for user mcos that want
   filesystem access; the substrate's own cache reads bypass it.

2. **wasm `_start` runs at instantiation**, not at first dispatch.
   mcos are stateful protos; init-once is the right discipline.
   Random's PRNG state seed lives in linmem from instantiation
   forward; methods like `:seedFrom:` mutate the linmem state.

3. **byte-buffer ownership in wasm linmem is wasm-side responsibility.**
   each language's binding can use a bump-allocator, malloc, or a
   designated scratch region — its choice. the ABI contract is
   uniform: import calls take `(ptr, len)` pairs from wasm-side
   allocations; rust copies bytes during the import call and never
   holds a pointer past the import return.

deferred to later tracks:

4. **manifest canonical encoding.** today the manifest is "moof
   source-text in a custom section." moving toward "canonical binary
   encoding" for content-addressing reproducibility. **defer to track
   2** (when content-hashing reproducibility actually matters across
   substrate builds).

5. **moof bytecode mcos.** when a manifest declares method impl as
   `#moof-bytecode <chunk-id>`, the `moof.bytecode` custom section
   serializes a chunk. format? the existing in-memory chunk is
   `Vec<Value>`-style; serialization needs work. **defer to track 3**
   (when pure-moof libraries want to ship as `.mco`s).

---

`>.<` softly. four languages, one container, no platform variants.
૮ ◞ ﻌ ◟ ા
