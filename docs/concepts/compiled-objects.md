# compiled objects

> **a `.mco` file is a runtime-loadable native module: a platform-
> tagged dylib + metadata describing which dylib symbols implement
> which moof methods on which proto. mcos are how the moof world
> brings in performant rust and c — wgpu, lmdb, websocket, blake3,
> the works — without bloating the substrate seed. the seed stays
> tiny; mcos do the heavy lifting; everything that *can* live in
> moof does.**

this is the maru / piumarta posture (substrate-as-tiny-seed)
extended with modern dylib loading. the substrate's rust line is
roughly 3k LoC. *everything else* with a foot in the OS — disk,
network, gpu, signing, hashing, encoding — comes in as an mco.

## the file

a `.mco` file is one logical unit, on disk:

```
header
  magic                "MOOF"
  version              u32
  content-hash         blake3 of body
  proto-name           string         ; "wgpu/Surface", "lmdb/Env"
  deps                 list of (proto-name, content-hash)

variants                              ; multi-platform
  for each platform in supported:
    platform-tag       e.g. "darwin-aarch64", "linux-x86_64"
    dylib-bytes        the actual .dylib / .so / .dll bytes
    dylib-hash         blake3 of dylib-bytes (for cache invalidation)

binding
  proto-info           proto's slots, parent-proto reference
  methods              table:
                         selector → {dylib-symbol-name, signature, purity}
  source-fallback      optional: pure-moof reference impl
                       (for documentation, audits, or platforms
                       without a dylib build)

signature
  signer-public-key    ed25519
  signature            over header + variants + binding
```

content-addressable. signable. shippable. the mco file format is
`reference/mco-format.md` (when written).

## loading

an `.mco` load returns *exactly one Form* — the proto.

the substrate's mco loader, on first proto reference:

1. find the `.mco` file (resolved by proto-name through a search
   path; a moof world ships its own mcos in `<world>/.moof/mcos/`).
2. verify content-hash and signature.
3. select the variant matching the host platform.
4. extract the dylib bytes to a cache directory; `dlopen` it (via
   the `libloading`-style mechanism).
5. allocate a fresh Form in the moof heap to be *the proto*.
   populate its slot-template (the slot names instances will have)
   and parent-proto reference from the binding metadata.
6. for each method in `binding.methods`: look up the dylib symbol
   by name; allocate a method-Form whose proto is `Method` and
   whose `:invoke` handler is a native trampoline pointing at the
   dylib symbol; install in the proto-Form's handler table.
7. install the proto-Form under `proto-name` in the world's
   namespace.
8. return the proto-Form. **that's the entire load result.**

if any step fails — wrong platform, missing symbol, bad signature
— **fail loudly**. a partially-bound proto is unsafe.

## state stays in moof; only methods are in rust

**this is the load-bearing rule of the mco model.** instances of
an mco-loaded proto are *ordinary moof Forms in the moof heap* —
slots, handlers, meta, reflection, persistence, journaling all
behave exactly the same as for a moof-defined proto. the rust dylib
contributes *only* the bodies of certain methods; it does not own
state, hold instance pools, or hide anything from reflection.

when a native method runs:

```c
MoofResult lmdb_env_open(
    MoofContext *ctx,
    MoofValue   self,         // a moof Form whose proto is LmdbEnv
    MoofValue  *args,
    size_t      argc,
    MoofValue  *out
) {
    // read the path argument from moof
    const char *path = moof_string(ctx, args[0]);

    // open the rust-side resource
    let env: lmdb::Environment = lmdb::Environment::new().open(path)?;

    // store the env handle as a slot value on `self`. the slot value
    // is a ForeignHandle: a tagged opaque pointer + a destructor
    // function that the moof GC will call when `self` is collected.
    moof_set_slot(ctx, self, "handle",
        moof_foreign_handle(ctx, env, lmdb_env_destructor));

    // also store moof-visible state in moof slots
    moof_set_slot(ctx, self, "path", args[0]);

    *out = self;
    return MOOF_OK;
}
```

the env handle is *in a slot of the moof Form*. `[env slots]`
returns it. the moof gc tracks it. on heap-snapshot, the substrate
serializes the moof slots; foreign handles serialize as "broken"
(re-open required on load); other slots roundtrip normally. any
state the rust code wants to remember between calls *must* be in a
slot (or, for genuinely-global rust state, in a moof Form named in
the proto's namespace). nothing is hidden.

### foreign handles

a `ForeignHandle` is a moof Value variant containing:

- a `*mut c_void` opaque pointer (rust-allocated memory).
- a `fn(ptr: *mut c_void)` destructor.
- an optional content-tag for safety checks across mco boundaries.

the substrate guarantees:

- **destructors run** when the moof gc collects the holding Form
  (or when the slot is overwritten with a non-handle value).
- **destructors run at turn boundaries**, not mid-turn (matches
  `laws/determinism-laws.md` D6).
- **foreign handles cannot cross vat boundaries.** sending a Form
  with a ForeignHandle slot to another vat triggers a substrate
  error. (the rust pointer would be meaningless there.)
- **foreign handles cannot serialize.** persistence sees them as
  "broken"; the cap that owns them must re-open on load.

(this is BEAM's NIF resource pattern, with explicit moof reflection
of the handle-bearing slot.)

### consequence: protos written in moof, methods in mco

a typical mco-backed proto in moof world looks like:

```moof
;; LmdbEnv proto, slot template, *and most methods*, are moof.
;; only the leaf operations that need libmdb live in the mco.

(defproto LmdbEnv
  (slots path handle map-size)             ; just slot names
  (handlers
    [open: p]                              ; native (in mco)
    [close]                                ; native
    [read-txn: blk]                        ; native
    [write-txn: blk]                       ; native
    
    ;; derived methods, in moof:
    [get: k]
      [self read-txn: |txn| [txn get: k]]
    [put: k value: v]
      [self write-txn: |txn| [txn put: k value: v]]
    [contains?: k]
      [[self get: k] is-not-nil?]))
```

the mco supplies `:open:`, `:close`, `:read-txn:`, `:write-txn:`
because they're rust-shaped. everything derivable from those
primitives is moof code, written *in* the proto, where users can
read and modify it.

this is the `:satisfies?` / "derive all from a few" pattern from
`concepts/types.md`, applied at the mco interface.

## the dylib's ABI

every native method exported by an mco's dylib has the same C-ABI
signature:

```c
typedef int32_t MoofResult;       // 0 = ok; nonzero = error code

MoofResult moof_method(
    MoofContext *ctx,             // opaque substrate handle
    MoofValue   self,             // receiver Form
    MoofValue  *args,             // argument array
    size_t      argc,
    MoofValue  *out               // result, written by callee
);
```

the `MoofContext *` exposes a small, stable C API for:

- heap access: `moof_slot(ctx, form, "name")`,
  `moof_set_slot(ctx, form, "name", value)`,
  `moof_meta(ctx, form, "name")`.
- form construction: `moof_alloc_form(ctx, proto)`, then populate
  slots.
- foreign-handle wrapping: `moof_foreign_handle(ctx, ptr,
  destructor)`.
- value primitives: `moof_int_value(ctx, value)`,
  `moof_string(ctx, value)`, etc.
- sending messages back into moof: `moof_send(ctx, receiver,
  selector, args, argc, &result)`.
- raising errors: `moof_raise(ctx, sym, message)`.
- capability resolution: `moof_cap(ctx, name)`.

this is the **substrate native ABI**. it is the only ABI a native
method ever talks to. the substrate enforces capability discipline
through the api — there is no rust escape hatch.

the substrate native ABI is *the* ABI commitment. mcos compile
against a specific substrate-abi-version; the substrate refuses to
load mcos targeting incompatible versions.

(rust mco authors use a `moof-abi-rust` crate that wraps the C ABI
in safe rust types. similarly `moof-abi-c`, etc.)

## what gets shipped as mcos

substrate-adjacent things that *would* otherwise live in the rust
binary:

| mco | provides |
|---|---|
| `core/blake3` | hashing (used by canonical encoding) |
| `core/ed25519` | signing (used by reflector envelopes) |
| `core/canonical-encoder` | binary form serialization |
| `store/lmdb` | persistent kv store |
| `transport/websocket` | replicated session transport |
| `transport/webrtc` | low-latency p2p transport |
| `render/wgpu` | gpu-backed `:render-with:` surface |
| `render/terminal` | half-block + braille terminal renderer |
| `render/software` | cpu-only software 3D rasterizer |
| `input/xterm-mouse` | terminal mouse-event source |
| `input/sdl-pointer` | sdl2 mouse + keyboard source |
| `os/fs` | filesystem cap implementation |
| `os/net` | tcp/udp socket cap implementation |
| `os/clock` | wall-clock + monotonic timer |
| `os/random` | csprng |
| `format/json` | json read/write |
| `format/png` | png read/write |
| `compress/zstd` | compression for snapshot transfer |

a moof distribution ships with a *bundle* of canonical mcos; users
can add more or replace.

## what stays in the substrate seed

a small, fixed set:

- the form heap + GC.
- bytecode interpreter + send dispatch.
- inline cache machinery.
- the **bootstrap** sexpr reader (just enough to parse `parser.moof`).
- the **bootstrap** compiler (just enough to produce the bytecode
  for `compiler.moof`).
- the scheduler (single-vat in early phases).
- the mco loader.
- the moof binary's argv processing.

*everything* else — including the eventual production parser and
compiler — is moof code or mco-delivered native code, all of it
modifiable from inside moof.

ratio target: substrate seed is ≤3k LoC of rust. any growth
requires explicit justification.

## the rust→moof author experience

writing an mco from rust:

```rust
// lib/mcos/blake3-rs/src/lib.rs
use moof_abi_rust::*;

moof_object! {
    proto: "core/blake3",
    parent: "Object",
    slots: ["state"],            // declares slot template
    methods: {
        // primitives in rust:
        "hash:"             => fn(ctx, self_, args) { /* … */ },
        "incremental"       => fn(ctx, self_, args) { /* … */ },
        "update:"           => fn(ctx, self_, args) { /* … */ },
        "finalize"          => fn(ctx, self_, args) { /* … */ },
    },
    // derived methods stay in moof; the mco ships them as source
    // alongside the dylib, and the substrate compiles them at load.
    derived: r#"
        (handlers
          [hash-string: s]
            [self hash: [s as: Bytes]]
          [hash-file: path with: $fs]
            [self hash: [$fs read-all: path]])
    "#,
}
```

the `moof_object!` macro generates:
- the dylib symbols with the right C ABI for the rust primitives.
- a build-script step that emits `core/blake3.mco` containing the
  compiled dylib + binding metadata + the derived-methods source.

`cargo build` produces the mco. cross-compiling for other targets
produces additional variants which can be merged into one
multi-platform mco. derived methods are platform-independent and
identical across variants.

## multi-platform mcos

an mco can carry several `variants`, each tagged with a platform.
on load, the substrate picks the variant matching its host. a
multi-platform mco is built by composing single-platform mcos:

```bash
moof mco merge core/blake3-darwin-aarch64.mco \
                core/blake3-linux-x86_64.mco \
                core/blake3-windows-x86_64.mco \
                -o core/blake3.mco
```

distribution-by-default: the canonical mco bundle ships multi-
platform; user-built mcos start single-platform and gain variants
as users compile for new targets.

## hot-loading new mcos at runtime

a moof world can load an mco at runtime, not just at boot:

```moof
[$mco load: #Path "/users/shreyan/wgpu-experimental.mco"]
```

becomes an `EffectIntent` (`concepts/effect-intents.md`); the
authority loads the dylib, registers the proto, returns. user code
can immediately use the new proto.

unloading is *not* supported. a loaded dylib is loaded for the
process lifetime. (dlclose is fragile; we don't try to be clever.)

## hot-replacing methods on a loaded mco

an mco's methods are bound to dylib symbols at load time. if you
want to change behavior without rebuilding, you have two paths:

1. **override at the proto level.** add a moof closure as a handler
   on the same selector with higher priority than the native method.
   user code wins; native fallback is reachable via `super`.
2. **rebuild + reload as a new mco.** the old mco's methods stay
   accessible on existing instances; the new mco's proto gets a new
   name (`core/blake3-v2`); user code can `become:` instances over
   to the new proto.

(literal hot-patching of running native code is out of scope.)

## security

three layers:

### load-time

- **content-hash**: every mco carries blake3 of its body. mismatch
  ⇒ refuse load.
- **signature**: an mco is signed by its author's ed25519 key. the
  substrate maintains a list of trusted public keys. unsigned mcos
  load only with `--allow-unsigned-mcos` flag.
- **dependencies**: each mco lists its required mcos by `(proto-
  name, content-hash)`. a mismatch ⇒ refuse load.

### runtime

- **capability discipline**: native methods can only access caps
  passed to them. there is no rust escape hatch that bypasses cap
  verification (the `MoofContext *` API enforces this).
- **isolation**: native methods run in the calling vat's heap;
  cannot reach into other vats.

### sandbox (future)

- **wasm wrappers**: an mco may optionally provide its dylib as a
  wasm module instead of a native dylib. the substrate runs wasm
  variants in a sandbox (wasmtime). wasm mcos are slower but safer.
  v0 ships native-only; wasm support is a phase G+ addition.

## what mcos do NOT do

| thing | done by |
|---|---|
| save world state | per-vat database storage, itself mco-delivered |
| ship moof source code | source files (`.moof`) plus bytecode caches |
| serialize a far-ref | `concepts/references.md` |
| provide a programming environment | the running moof world |

mcos are exactly: "the bridge from rust/c-shaped libraries into
moof's send-shaped object world." that's their whole job. anything
broader is a different mechanism.

## the mco contract from moof's side

calling a method whose handler is a native mco binding is
*indistinguishable* from calling a moof closure. inline caches work
the same way; reflection works the same way; the proto chain looks
the same. the only difference is that `[m source]` returns the
mco's source-fallback (if provided) or a synthesized form like
`(native "core/blake3" :hash:)` describing the binding.

## what makes this maru-flavored

piumarta and warth's *open extensible object models* (vpri 2007)
showed that a substrate of ~200 lines of c can bootstrap a full
language. moof's seed is bigger (we have a heap GC, bytecode
interp, scheduler — c didn't), but the *posture* is identical:

> **the seed contains only what cannot be expressed above itself.
> everything else is delivered through one uniform mechanism that
> is itself moldable from within.**

mcos are that uniform mechanism for native libraries. moof source
is that uniform mechanism for high-level code. the mco loader is
itself accessible from moof (as the `$mco` cap, internally an mco
on top of `libloading`). the world grows itself.

## inspirations

- **maru / cola**: ian piumarta, alessandro warth (vpri 2007).
  substrate-as-tiny-seed.
- **erlang BEAM NIFs**: native implemented functions; symbol-name
  binding registered at module load. moof's mcos are NIFs +
  packaging.
- **dynamic loading in unix**: thompson/ritchie's `dlopen` is
  ancient; moof inherits the model and adds content-addressing.
- **wasm component model** (bytecodealliance): the future wasm
  variant of mco draws here.
- **content-addressed storage**: ipfs (juan benet et al.); nix
  (eelco dolstra).
- **plugin systems with stable abis**: postgres extensions, redis
  modules, vim plugins built with `:cdll`.
- **the linker as a value**: c. p. wadsworth's "the linker is the
  module system." moof's mco loader is a linker that lives inside
  the world.

## see also

- `process/docs-driven.md` — when to write rust-via-mco vs moof.
- `concepts/persistence.md` — itself mco-delivered.
- `concepts/transport.md` — transport leaves are mcos.
- `concepts/canvas-and-input.md` — `$canvas`, `$pointer` are mcos.
- `concepts/world-and-space.md` — wgpu renderer is an mco.
- `reference/mco-format.md` — binary format spec (when written).
- `reference/native-abi.md` — the C ABI mcos compile against (when
  written).
