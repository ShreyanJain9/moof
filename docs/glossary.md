# glossary

**type:** reference

> every term moof uses, defined once. if a concept doc
> introduces a term, this file has the short form. if a term
> here conflicts with a concept doc, the concept doc wins — tell
> us so we can fix this file.

---

## A

**Act** — a first-class effect descriptor. returned from a cross-
vat send. has a state (`pending`/`resolved`) and eventually a
value. composable via `then:` and `(do ...)`. see
[concepts/effects.md](concepts/effects.md).

**aspect** — one view of an object (e.g., as text, as JSON, as
prototype chain). an aspect is a handler you call on the object.
the canvas stacks aspects visually.

**Atom** — a defserver wrapping a single mutable cell of state.
`[atom get]`, `[atom swap: new]`. see `lib/flow/reactive.moof`.

**Awaitable** — protocol for values that resolve over time. Act
conforms. Cons doesn't. (from the wave-jubilee Thenable split.)

---

## B

**bag** — multiset. `lib/data/bag.moof`. conforms to Iterable.

**BigInt** — heap-backed arbitrary-precision integer. NOT a
distinct moof type — part of the unified `Integer` type (wave 9.3).

**blob** — content-addressed bytes in the blob store. every
immutable value can be one.

**blobstore** — the LMDB-backed persistent store at `.moof/store`.
three tables: blobs (hash→bytes), refs (name→hash), meta.

**Block proto** — the prototype for closures. both moof-compiled
closures and wrapped native handlers are Block-proto objects.

**boot** — loading the image and starting vat 0. today: rust-
driven with source replay. wave 10+: moof-driven from a seed
image.

**bootstrap** — the source files replayed into every vat at
startup (`moof.toml` `[sources].files`). scheduled for replacement
by seed-image hydration.

---

## C

**call:** — the selector applicative calls desugar to. `(f x)` →
`[f call: x]`. any object with a `call:` handler is callable.

**Callable** — protocol for invokable values. required: `call:`.

**canvas** — the (future) zoomable visual UI. every object renders
itself via the Renderable protocol; halos expose per-object verbs.
wave 13.

**capability** — an object reference that grants authority to do
something effectful. moof's security model: authority =
reachability. see [concepts/capabilities.md](concepts/capabilities.md).

**capability plugin** — a rust cdylib that spawns a capability
vat at startup (Console, Clock, File, Random, System, Evaluator).

**cdylib** — C-style dynamic library. moof plugins are cdylibs
loaded via `libloading`.

**closure** — a function with captured environment. a Block-proto
heap object with a code_idx slot and captured values as named
slots.

**Comparable** — protocol for orderable values. required: `<`.

**Cons** — linked-list cell. `(cons head tail)`. `car`/`cdr`
accessors. conforms to Iterable.

**content-addressing** — identity by hash of canonical
serialization. the same value has the same hash always, on any
machine. see [concepts/persistence.md](concepts/persistence.md).

---

## D

**defmethod** — adds a handler to a prototype.
`(defmethod Integer isEven? () (eq [self % 2] 0))`.

**defn** — defines a named function. `(defn name (args) body)`.

**defprotocol** — declares a protocol with required and provided
methods. see [concepts/protocols.md](concepts/protocols.md).

**defserver** — declares a server vat. `(defserver Name (init-args)
body)`. see `lib/tools/server.moof`.

**delta** — a slot-change set carried in an Update. the scheduler
applies the delta atomically between server messages.

**doesNotUnderstand:** (DNU) — the message sent when dispatch
fails to find a handler. override it to create proxies, DSLs,
auto-generators.

**do-notation** — syntactic sugar for chaining Monadic values.
`(do (x <- expr) body)` is bind. `(do e1 e2)` is sequencing.

**drain** — the scheduler operation that processes all pending
outbox messages and resolves ready Acts. called implicitly by the
REPL between inputs.

---

## E

**Equatable** — protocol for value equality. required: `equal:`.

**effect** — anything that crosses a vat boundary. all effects
return Acts. pure code has none.

**env** (environment) — the binding table for variable lookup. a
moof object with a `parent` slot (outer scope) and a `bindings`
slot (name → value table).

**eventual send** — a cross-vat message send. syntax `[obj <-
selector: args]` or implicit when `obj` is a FarRef. returns an
Act immediately; handler runs later.

**exemption** — a named, time-boxed violation of a law.
documented in `docs/exemptions.md` with owner and wave target.

---

## F

**Fallible** — protocol for values that can fail. required: `ok?`.
Ok, Err, Some, None conform.

**FarRef** — a proxy to an object in another vat. carries
`__target_vat`, `__target_obj`, `url`. sends through it become
outbox messages.

**federation** — moof's cross-machine plan: FarRefs that span the
network, content-addressed cache, signed peer identities. wave
15.

**Floating** — informally: `Float` (IEEE 754 double). `Float` is
its own proto; not part of `Integer`.

**foreign type** — a rust type exposed as a first-class moof
value via the `ForeignType` trait. BigInt, Vec3, JsonValue, etc.

**fuel** — the scheduler's budget-per-vat-per-turn. a vat runs
until its fuel is exhausted, then yields.

---

## G

**GC** — garbage collection. moof uses mark-and-sweep, triggered
by a budget. roots: env, VM frames, ready Acts, persistent refs.

**grants** — which capabilities each interface gets. defined in
`moof.toml` under `[grants]`. planned to be visible as a moof
object in wave 9.6.

---

## H

**halo** — the click-and-hold-to-see-verbs UI gesture. per-object
via `halo-verbs` handler. wave 13+.

**handler** — a named piece of behavior on an object. handlers
respond to messages. `handler_set` installs one.

**Hashable** — protocol for values that produce a stable hash
Integer. required: `hash`.

**heap** — a vat's private arena for objects. values in one heap
can't reference values in another; cross-vat transfer copies.

---

## I

**identical:** — identity check: `[a identical: b]` → true iff
same heap cell.

**image** — moof's single persistent artifact. everything lives
in the image: objects, protos, bindings, services, blob store.

**Indexable** — protocol for random-access sequences. required:
`at:`, `count`.

**Integer** — the unified integer type. i48 primitive internally
for small values; BigInt foreign for large. user sees one type.

**Interface** — a moof "interface" is a peer consumer of System
that asks for capabilities (repl, script, eval). also, an old
protocol in `system/system.moof` slated for deletion.

**Iterable** — protocol for walkable sequences. required:
`fold:with:`. provides ~40 methods.

---

## K

**keyword selector** — a selector whose parts end in colons:
`at:put:`. common for multi-argument message sends.

---

## L

**let-it-crash** — erlang's reliability pattern: don't defend
against every failure; isolate failures and supervise restarts.

**LMDB** — the key-value store underlying moof's blob store.
crash-safe, concurrent readers, mmap-fast.

---

## M

**membrane** — a proxy that intercepts every cross-boundary
message. logs, allows, denies, or transforms. key primitive for
capability attenuation and revocation. planned.

**Monadic** — protocol for bind-able values. required: `then:`
and class-side `pure:`. Act, Cons, Option, Result conform.

**mount** — composes namespaces. `[ns mount: other at: 'prefix]`
makes `other` accessible under `/prefix/*` in `ns`.

---

## N

**namespace** — a tree of named values, per-vat, plan-9-shaped.
today mostly represented as nested Tables.

**native** — a rust-implemented handler. indistinguishable at the
moof level from a moof closure.

**Numeric** — protocol for arithmetic values. required:
`+`, `-`, `*`, `=`, `<`.

**nil** — the empty value. tagged immediate. not an object in the
same sense as others; it has no handlers.

---

## O

**Object** — the root prototype. everything ultimately delegates
to Object.

**Ok / Err** — Result constructors. `(Ok v)` for success,
`(Err msg)` for failure. Monadic + Fallible + Showable.

**Option** — Some/None value type for optional presence.

**outbox** — a vat's queue of outgoing messages (to other vats).
drained by the scheduler into target mailboxes.

---

## P

**Pair** — the rust struct backing Cons cells.

**peer** — another moof instance, remote. accessed via URLs like
`moof:peer/alice/...`. federation-future.

**protocol** — a contract: "implement these required methods,
get these provided methods free." moof's type system.

**prototype** — another object that this one delegates to for
message dispatch. every object has one. no classes.

**pure** — code with no cross-vat references. guaranteed
deterministic, replayable, cacheable.

---

## R

**Range** — a numeric interval. `(range 0 10)`. Iterable.

**Reactive** / **Signal** — reactive values that notify
subscribers on change. see `lib/flow/reactive.moof`.

**REPL** — moof's read-eval-print loop. a moof `Interface` that
receives input, evaluates, prints, drains. currently implemented
in rust (`crates/moof-cli/src/shell/repl.rs`).

**Registry** — object holding the service table. wave 9.4: plain
proto. wave 9.6+: defserver.

**Result** — Ok/Err sum type. Monadic + Fallible.

**reveal** — not yet a formal moof concept; used in halos: "reveal
this object's halo verbs."

---

## S

**schema_version** — a ForeignType's current serialization
version. images with mismatched schemas require migrators.

**scheduler** — the fuel-based preemptive runner for vats. rust
struct today (law 1 targets this).

**selector** — a symbol naming a message. `'+'`, `'at:put:'`,
`'describe'`.

**send** — the one moof operation. `[obj sel: arg]` invokes the
handler chain-found for `sel` on `obj`.

**Service** — a declared service in System's registry. has a
name, description, spawner, restart policy, depends list.

**Set** — unordered unique collection. `lib/data/set.moof`.

**Showable** — protocol for human-renderable values. required:
`show`.

**slot** — a public named data entry on an object. fixed-shape.
accessed via `obj.name` or `[obj slotAt: 'name]`.

**Stream** — a lazy sequence with a `next:` producer. Iterable.

**Symbol** — interned string. tagged immediate value. used for
names (selector, slot name, etc).

**System** — the vat-0 capability that exposes services, grants,
vats, and the namespace root. `[system root]`, `[system services]`.

---

## T

**Table** — a key-value + ordered-seq hybrid. `#[ a: 1 b: 2 ]`.
works as a map and a list simultaneously (Lua-style).

**Text** — the rust struct backing String.

**Thenable** — the old fused protocol with then: + map: + recover:
+ ok?. being split into Monadic + Fallible + Awaitable per the
jubilee.

**time-travel** — navigating past image states. content-
addressing enables it; UI exposure pending.

**transducer** — a composable reducer transformation.
`lib/flow/transducer.moof`. moof's primary pipeline primitive.

**type plugin** — a rust cdylib that registers a new ForeignType
on every vat's heap.

---

## U

**unary selector** — a selector with no colon and no args:
`'describe'`, `'abs'`.

**union mount** — plan-9 style: multiple namespaces mounted at the
same point, lookups try each in order.

**Update** — a slot-change delta + a reply value. returned from
server handlers. the scheduler applies the delta between
messages.

**URL** — moof's universal identifier. content or path-based.
`moof:<hash>` or `moof:/caps/console` or `moof:/vats/7/objs/42`.

---

## V

**vat** — a single-threaded isolated actor with its own heap,
message queue, and event loop. moof's concurrency unit.

**vau** — a primitive that creates an operative: a function that
receives unevaluated args and the caller's environment. lets user
code define its own special forms.

**Vec3** — a plugin-provided 3D vector type.

---

## W

**Workspace** — a defserver in `lib/tools/workspace.moof`. holds
a title, a list of blocks, and a binding table. a candidate
user-facing container for the eventual canvas.

**with:** — non-destructive slot update. `[obj with: { x: 99 }]`
returns a new object like `obj` but with `x` replaced.

---

## common abbreviations

- **DNU** — `doesNotUnderstand:`
- **FFI** — foreign function interface
- **VM** — virtual machine (moof-lang bytecode runtime)
- **HAMT** — hash array mapped trie (planned table impl)
- **BLAKE3** — the hash function used for content addressing
- **E language** — mark miller's capability-secure language,
  moof's influence for capabilities

---

## what's NOT in moof

some terms you might expect:

- **"class"** — we have prototypes, not classes.
- **"method"** — every method is a handler; every handler is a
  method. no distinction.
- **"function"** — every callable is an object with a `call:`
  handler.
- **"exception"** — replaced by Result/Err.
- **"statement"** — everything is an expression.
- **"variable"** — we have bindings in environments, immutable
  by default.

these terms appear in some older docs; the concept they name is
covered by a different word in the above glossary.
