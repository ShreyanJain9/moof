# the mco format

> **an `.mco` file is the universal moof artifact: behavior packaged
> for delivery. it's a wasm module with moof-specific custom
> sections; it carries one proto's worth of methods (which may be
> wasm-native, pure-moof bytecode, or both); it doesn't name itself;
> the loader hands the caller a fresh proto-Form to bind however
> they like. one container, three possible payloads, infinite
> language flexibility, no platform variants.**

this supersedes the dylib-based design in
`docs/concepts/compiled-objects.md` for the canonical mco model.
dylib-mcos remain a tier-3 escape hatch for genuine perf-cliff
cases (gpu hot path, shared-memory mmap); wasm-mcos are tier-2,
the canonical case; inline `(native LANG …)` natives are tier-1
for hot-loop primitives. most things are tier 2.

## the file shape

```
┌──────────────────────────────────────────────┐
│ wasm preamble (magic 0x6d736100, version 1)  │
│ ──────────────────────────────────────────── │
│ types, imports, functions, exports, code     │  ← any wasm tool reads this
│   (may be empty for pure-moof mcos)          │
│ ──────────────────────────────────────────── │
│ custom section "moof.manifest"               │
│   { abi-version, parent, methods,            │
│     imports-cap, imports-mco }               │
│ ──────────────────────────────────────────── │
│ custom section "moof.bytecode"   (optional)  │
│   serialized moof chunks (consts, ops, ics)  │
│ ──────────────────────────────────────────── │
│ custom section "moof.source"     (optional)  │
│   the canonical moof source, for L5 honesty  │
│ ──────────────────────────────────────────── │
│ custom section "moof.deps"                   │
│   list of (local-name → content-hash)        │
│ ──────────────────────────────────────────── │
│ custom section "moof.signature"              │
│   ed25519 over preceding sections            │
└──────────────────────────────────────────────┘
```

an .mco file is *valid wasm* — `wasmtime my-clock.mco` runs the
wasm portion (which may do nothing); `wasm-objdump` lists its
exports; any wasm tool eats it without choking. the moof-specific
bits live in custom sections, which are wasm's standard mechanism
for "data the runtime ignores but tools may read."

extension is `.mco` because the contents are wasm-plus-conventions;
calling it `.wasm` would be a half-truth. precedent: `.deb` is `ar`,
`.docx` is `zip`, `.elf` is generic. descriptive name beats truthful-
but-confusing.

## load-time anonymity

**the load-bearing design rule.**

> an mco does not name the proto it provides. loading returns a
> fresh proto-Form; the caller binds it. the substrate never auto-
> installs into the global env.

why this matters:

- **no naming collisions.** two vendors ship clocks? bind them to
  different names: `(def Clock1 [$mco load: "vendor-a/clock.mco"])`
  / `(def Clock2 [$mco load: "vendor-b/clock.mco"])`. done.
- **loading is composable.** the result is a value. pass it around,
  store it in slots, build compound objects from it.
- **the artifact is pure behavior.** namespace policy is a host
  concern, not an artifact concern. the .mco file says "here are
  my methods"; the loading user says "i'll call this `TimeSource`"
  (or doesn't — `(map mco-paths [$mco load:])` returns a list of
  unnamed protos).
- **rename is free.** vendor's "Clock" can be your "Watch" or
  "$clock-ng" without coordinating.

this matches smalltalk's file-in/file-out (loading a class file
yields a class object you assign), common lisp's `(load …)` (returns
the package), and scheme's r7rs library system (modules are values
to import-and-rename). moof inherits all three and keeps the
discipline at the substrate level.

## loading semantics

loading is an effect (filesystem access + executing untrusted code),
so it routes through a cap:

```moof
(def TimeSource [$mco load: "core/clock.mco"])
(def $clock [TimeSource new])
[$clock now]   ;; → 1735689600000000
```

`[$mco load: path]` does:

1. **read** the .mco file — verify wasm magic, parse custom sections.
2. **verify** signature (`moof.signature`), content-hash, abi-version
   compatibility. mismatches refuse to load.
3. **resolve dependencies** declared in `moof.deps` — for each
   `(local-name . content-hash)` entry, locate or fetch the dep
   mco; install its proto into a private env keyed by `local-name`.
4. **instantiate** the wasm module (if non-empty) — substrate
   provides imports declared in `moof.manifest.imports-cap` (the
   substrate's MoofApi vtable: heap access, send-back, foreign
   handle, raise, etc).
5. **allocate** a fresh proto-Form. its `proto` field is the
   manifest's `parent:` (default `Object`).
6. **install handlers** for each method in `moof.manifest.methods`:
   - if the method is in `moof.bytecode`, install the chunk-based
     handler (a Method-Form whose body is the embedded chunk).
   - if the method is a wasm export, install a native handler
     that wraps the wasm function via the substrate's wasm bridge.
7. **return** the proto-Form. the substrate does not touch the
   global env.

## the manifest

`moof.manifest` is a serialized moof Form (canonical encoding).
schema:

```moof
{ abi-version    1
  parent         'Object                ;; or another local-name
  methods
    [{ sel        'now
       impl       #wasm-export "now"    ;; or #moof-bytecode <chunk-id>
       arity      0 }
     { sel        'monotonicNow
       impl       #wasm-export "monotonicNow"
       arity      0 }]
  imports-cap                            ;; what the wasm imports from substrate
    [{ name       "moof_value_int"
       sig        :i64-to-handle }
     { name       "moof_raise"
       sig        :handle-handle-to-result }]
  imports-mco                            ;; what other mcos this depends on
    []                                   ;; (none for clock; resolves at load)
  caps-required                          ;; substrate-provided caps the mco uses
    []                                   ;; (none for clock; would list e.g. $time)
}
```

note the manifest **does not** declare a name for the proto
itself. it describes its parent, its methods, its dependencies,
its imports — *behavior*. naming is the loader's call.

## three payload tiers

an `.mco` can carry any combination of:

| payload | what it means | example |
|---|---|---|
| **wasm only** | native code; no moof bytecode | `core/clock.mco` (zig→wasm) |
| **moof bytecode only** | pure-moof library; empty wasm | `core/cons-utils.mco` |
| **both** | hybrid; native fast path + derived moof methods | `core/blake3.mco` (hash: native, hashFile:: moof) |

the manifest's `methods` table tells the loader which of the
methods are wasm-implemented vs moof-bytecode-implemented. each
method has exactly one impl, but different methods on the same
proto can use different mechanisms. this matches the `derived:`
field in the original (dylib) compiled-objects design — except
both kinds of methods are *uniformly addressable* through the
mco file format.

## single-proto-per-mco

constraint enforced by the loader: **one mco provides one proto.**

why:

- forces good factoring. shipping `core/clock+stopwatch+timer.mco`
  is wrong; ship three separate mcos.
- naming-anonymity is much cleaner with one proto per mco —
  loading returns a single value.
- enables better caching, dep-resolution, and content-addressing.
  `core/clock@<hash>` identifies behavior unambiguously.
- mco bundles (multiple related mcos shipped together) live above
  the single-mco level: a `.bundle` is a directory of `.mco`s.

if your library wants three protos, ship three mcos. if you want
to ship them together as a unit, bundle them.

## content addressing

content-hash is the canonical name. an mco at content-hash `7f3a2c…`
is `7f3a2c…` *forever*. you reference it by hash; you fetch it by
hash; you cache it by hash; you sign over its hash; replicas
exchange it by hash.

```bash
moof install core/clock@7f3a2c.mco
moof show core/clock@7f3a2c.mco       # describe manifest, no instantiation
moof verify core/clock@7f3a2c.mco
```

this gives you nix-store-grade reproducibility for moof natives.
content-hash includes the wasm bytes, the manifest, the bytecode
section, the source — but not the signature (which is *over* all
the rest).

## platform independence

**no platform variants.** ever.

the original (dylib) compiled-objects design needed `darwin-aarch64`,
`linux-x86_64`, `windows-x86_64` variants packaged together. that
was a workaround for native-binary platform-tie. wasm bytecode is
platform-independent by definition. moof bytecode is platform-
independent by definition. the .mco format ships *one* artifact;
it runs on:

- arm64 mac
- x86_64 linux
- aarch64 linux
- aarch64 mobile (ios / android, eventually)
- wasm32 in a browser running moof-on-wasm
- whatever else exists by phase G

without recompilation. this is the holy grail of portable
software: *compile once, run everywhere actually means it now*.

## dependency resolution

`moof.deps` lists what other mcos this mco needs:

```moof
[ {local: 'CONS_PROTO  hash: <hash-of-core/cons.mco>}
  {local: 'METHOD_PROTO hash: <hash-of-core/method.mco>} ]
```

`local:` names are private to this mco — they exist only inside
its own manifest, used to refer to the dep in `parent:` /
`imports-mco:` / `methods.[].chunk-references`. NO global naming
contamination.

at load time, the loader:

1. for each dep, locates the mco at `<hash>` (cache, peer fetch,
   or refuse if missing).
2. recursively loads it.
3. installs the resulting proto-Form in this mco's private env
   under the local name.

the loaded mco's bytecode (if any) compiles its `LoadName` of
`'CONS_PROTO` against this private env, NOT the user's global env.
this is the **referential transparency** an mco needs: it sees
the world it declared, not whatever the loading user happens to
have in scope.

primordial protos (`Object`, `Cons`, `Method`, `Chunk`, `Closure`,
`Integer`, `Float`, `Symbol`, `Char`, `Bool`, `Nil`, `Frame`,
`String`, `Table`, `ForeignHandle`) are implicitly available
under their canonical names in every mco's env. they're the
substrate's primordial bindings; mcos don't have to declare them
in `moof.deps`.

## dev tier vs prod tier

during development, you write `clock.zig`, compile to `clock.wasm`,
and want to load it without ceremony. the substrate's loader is
*permissive*:

```bash
moof world ./my-world --allow-unsigned
[$mco load: "clock.wasm"]      # raw .wasm; loader infers manifest
                                # from a `_moof_manifest_json` export
                                # or a sidecar `clock.manifest.json`
```

for shipping:

```bash
moof mco pack clock.wasm \
  --parent Object \
  --methods now,monotonicNow \
  --sign \
  --output core/clock.mco
```

producing a fully-signed, manifested `.mco`. the substrate's loader
is *rigorous* in prod:

- requires `moof.manifest`, `moof.signature` custom sections.
- verifies signature against trusted public keys.
- verifies content-hash matches filename (when convention is
  `<purpose>@<hash>.mco`).
- refuses unsigned mcos unless `--allow-unsigned` is set.
- refuses abi-version mismatches.

## the moof-as-os framing

**moof world is a kernel; mcos are programs; moofscript is a shell.**

- the seed = wasm runtime + heap + scheduler + capability router.
- everything else = mcos. every cap, every proto, every native
  primitive, every datasource, every renderer, every transport,
  every parser — all mcos.
- the moof VM itself is an mco. swap it. `core/vm-rust.mco` →
  `core/vm-cranelift.mco` → `core/vm-trace.mco` → wasm-sandbox.
- moofscript-the-language is delivered through mcos too: the
  parser is `core/parser.mco`, the compiler is `core/compiler.mco`.
  someday they're rewritten in zig or haskell — same artifact
  format, no churn.

this is *moof's polyglot story*: one container, any language that
targets wasm, no platform variants, no privileged "host language."
the polyglot creds are the substrate, not a feature.

## what mcos do NOT do

- **do not name themselves.** the manifest has no `proto:` field.
  the caller of `[$mco load: …]` decides the binding.
- **do not auto-install into the global env.** the loader returns
  a Form. users / supervisors do the binding.
- **do not provide multiple protos.** one mco, one proto. ship a
  bundle if you need related units.
- **do not platform-vary.** wasm + moof-bytecode are both portable.
- **do not break wasm validity.** any wasm tool can still parse
  and validate the file. moof-specific custom sections never
  affect the wasm semantics.
- **do not capture state from the loading world.** mcos are pure
  *behavior*. instances created from their proto live in the
  loading world's heap; the mco file is a value-less template.

## tooling

```bash
# inspect
moof mco describe core/clock.mco       # manifest + deps + sig
moof mco extract core/clock.mco        # extract pure wasm part
moof mco verify  core/clock.mco        # sig + hash + deps

# build
moof mco pack <wasm-file> [<bytecode-file>] \
  --parent <proto-name>      \
  --methods <sel>,<sel>,...  \
  --imports <import-spec>    \
  --deps <dep-hash>,...      \
  --sign

# distribute
moof mco install <hash>                # fetch + verify + cache
moof mco share core/clock.mco          # publish to peers (phase F+)
```

## doc gates

- `docs/concepts/compiled-objects.md` — narrative motivation (was
  dylib-focused; treat *this* file as the canonical format spec).
- `docs/laws/substrate-laws.md` L5 (source canonical) — the
  `moof.source` section honors this for moof-bytecode payloads.
- `docs/laws/substrate-laws.md` L9 (capabilities unforgeable) —
  `[$mco load:]` is a cap-mediated effect; only the supervisor
  hands `$mco` out.
- `docs/laws/reflection-contract.md` — loaded protos are real
  Forms; `[mco-loaded-proto handlers]` etc all work normally.

## see also

- `docs/concepts/compiled-objects.md` — the prior (dylib) design.
- `docs/reference/native-abi.md` — the wasm import surface
  (when written: the substrate's MoofApi vtable in concrete C ABI).
- `docs/reference/wire-protocol.md` — for the deferred process-as-mco
  alternative (if ever needed).
- `docs/concepts/canonical-encoding.md` — used by `moof.manifest`
  serialization (when written).
- `concepts/capabilities.md` — `$mco` as a primordial cap.

## inspirations

- **smalltalk file-in/file-out** — a class file, when filed in,
  yields a class. moof inherits the discipline.
- **scheme r7rs libraries** — modules are first-class values, named
  by the importer.
- **erlang BEAM modules** — code modules as artifacts; hot-swap
  is mechanical.
- **wasm component model** — moves toward exactly this design at
  the wasm-spec level. moof gets there ahead of the standard.
- **nix store** — content-addressable artifacts as the source of
  truth; rebinding is host-policy.
- **plan 9** — "everything is a file"; here, "everything is an
  mco" is the analogous unifying claim.

`>.<` softly. one container, all languages. ૮ ◞ ﻌ ◟ ა
