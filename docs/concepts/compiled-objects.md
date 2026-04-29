# compiled objects

> **a `.mco` file is one object, serialized to bytes. you load it,
> bind it to a variable, and use it. its sole purpose is letting
> rust code appear as moof methods on that object.**

compiled objects do exactly one thing: bring rust-implemented
methods into moof. they are *not* used for image persistence (that's
per-vat database storage; `concepts/persistence.md`), nor for
distribution (that's data sources + far-refs), nor for stdlib
delivery (that's source files + bytecode caches).

making `.mco` narrow is the v3-lesson: any single mechanism you
overload with four jobs starts breaking at the seams.

## the file

a `.mco` file describes one object:

```
header
  magic         "MOOF"
  version       u32
  body-hash     blake3 of body
  deps          list of (path, content-hash) — required external protos

body
  proto         reference to the parent proto (by dep + form-id)
  slots         initial slot values (canonical encoding)
  methods       table: selector → method-body
                  method-body := bytecode | native-ref(symbol-name)
  source        optional source forms (for round-trip)
  type-info     optional protocols/effect rows
  doc           optional
```

it's just bytes. content-addressable. signable. shippable.

## loading

```moof
(let RustyVec (load-compiled "/lib/RustyVec.mco"))

;; RustyVec is now a moof object. methods backed by rust are
;; indistinguishable from native moof methods at the call site.
(let v [RustyVec new])
[v push: 5]
[v push: 3]
[v length]                       ; → 2
```

load procedure:
1. read header; verify dep hashes against locally-available protos.
2. resolve native symbol-names against the in-process registry.
3. materialize the object's Form into the current vat's heap.
4. return the form-id.

if any required native symbol is missing, *fail loudly*. a
partially-alive compiled object is unsafe.

## the rust side

a rust crate exporting one moof object:

```rust
moof_object! {
    name:  "RustyVec",
    proto: Object,
    slots: { len: 0 },
    methods: {
        "push:"     => native push_impl,
        "at:"       => native at_impl,
        "length"    => native length_impl,
        "as-string" => bytecode r#"
            (pipe self
              [for-each: |x| (build x)]
              [build finalize])
        "#,
    }
}

fn push_impl(state: &mut Heap, this: FormId, args: &[Value]) -> Value { … }
fn at_impl(state: &mut Heap, this: FormId, args: &[Value]) -> Value { … }
fn length_impl(state: &mut Heap, this: FormId, args: &[Value]) -> Value { … }
```

build → emits `RustyVec.mco`. the running rust process's static
registry has fn-pointers under the names `"RustyVec::push:"` etc.
loading `RustyVec.mco` finds those pointers and links them.

## native trampoline registry

every native method is referenced in the `.mco` by *symbol-name*. at
process init, the rust crate registers `(name → fn-ptr)` in a
static `NATIVE_REGISTRY`. the loader resolves symbol-names against
this registry.

cross-process boundary: a `.mco` only loads in a process that has
the matching rust binary's natives registered. cross-machine sharing
of `.mco` requires the same rust code on both ends. (this is the
realistic constraint; we don't pretend `.mco` is platform-neutral.)

## what `.mco` does NOT do

| thing | done by |
|---|---|
| save a vat's state | per-vat database storage (`concepts/persistence.md`) |
| share code across machines at runtime | source files + recompile, OR data-source-streamed forms |
| package a library | a directory of `.moof` source files (+ optional `.mco` for rust bits) |
| distribute the stdlib | source files in `lib/`, bytecode-cached on first load |
| serialize a far-ref | `concepts/references.md` |
| journal mutations | `concepts/persistence.md` |

## the rust→moof contract

when you write a rust crate that exports moof methods:

1. you *register* native symbols at process init via `moof_object!` macros.
2. the build emits `.mco` files describing the methods.
3. moof loads the `.mco` files at runtime; natives bind by symbol-name.
4. moof code calls the methods normally; they happen to be implemented
   in rust.

from moof's side, calling a native method is calling a method.
inline caches, reflection, types, pattern-matching, all work
identically.

## what natives can and can't do

natives are passed:
- `state` (the heap, scheduler, etc., behind a controlled handle).
- `this` (the receiver's form-id).
- `args` (a slice of Values).

natives can:
- read/write the receiver's slots through the heap handle.
- send messages via the heap handle (synchronous within the vat).
- allocate new Forms.
- invoke other native or moof methods.

natives *cannot*:
- bypass capability checks (the rust handle enforces them).
- access another vat's heap directly.
- escape the substrate's invariants (like leaking form-ids across
  vat boundaries — `laws/isolation-laws.md`).

the rust→moof contract is small and disciplined. natives are not
"a way to do whatever you want in rust." they're "a way to make a
moof method's body run faster or interface with the os."

## inspirations

- erlang BEAM's NIFs (native implemented functions) — same model:
  named-symbol bindings, registered at module load, called as
  ordinary erlang functions.
- the discipline of "the file format IS the abi" comes from beam,
  java's `.class`, and python's `.pyc` traditions — none of which
  use dylib loading at runtime, all of which package code as data.
- the conviction that we *don't* try to make `.mco` do everything
  comes from watching v3's plugin-dylib system grow into a
  multi-headed monster.

## see also

- `concepts/persistence.md` — image storage (different mechanism).
- `concepts/data-sources.md` — code distribution (different mechanism).
- `process/docs-driven.md` — when to write rust vs moof.
