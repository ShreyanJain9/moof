# state of the implementation (post-phase 2 commit)

> **the docs describe a substrate. the code, today, simulates the
> easy-to-implement parts of that substrate while violating most of
> its load-bearing laws. this doc is the honest accounting and the
> plan to fix it.**

written *after* the first push (commit `67ae0da`). no point being
polite — the architecture is flimsy and we know it. naming the gaps
is how we close them.

## the headline

what we have is **a small lispy interpreter with a smalltalk-shaped
veneer over it**. it satisfies the surface examples in the README. it
does *not* satisfy the substrate-laws.md guarantees. the four-faces-
of-Form vision is essentially aspirational at the implementation
level.

we're at "simulation of the basics" rather than "real substrate v0."
that's fine — but we shouldn't pretend otherwise.

## laws violated, in order of severity

### L3 — send dispatch is the universal verb (severely violated)

the docs claim every operation routes through `vm::send_dispatch`.
in reality:

- **env access is direct rust code.** `env_lookup`, `env_define`,
  `env_set` walk the env chain by hand. there is no `[env lookup:
  name]` send.
- **special forms are a hardcoded `match` on string names** in the
  rust compiler. a user defining their own `if` would be silently
  ignored.
- **`slot` / `slot-set!` are global builtin functions**, not sends.
  `[obj slot: 'name]` doesn't exist; you write `(slot obj 'name)`.
- **lexical name lookup is not a send.** the compiler emits
  `LoadName(sym)`; the VM looks it up directly.
- **`new` is a global function**, not a method on a proto. classical
  smalltalk has `[Proto new]` send to the class. we have `(new Proto)`
  fn-call.

what this costs us: the entire moldability claim. you cannot, from
inside moof, override how envs lookup names, how `if` dispatches, how
slot access works, or how `new` constructs. the substrate is not
moldable in the way the docs promise.

### L4 — eval is itself a send (entirely violated)

the docs (`concepts/forms.md`) claim:

```
eval(form, env) := handler := lookup(form.proto, :eval); handler(form, env)
```

we don't do this anywhere. the rust compiler hardcodes how each kind
of form evaluates. there is no `:eval` handler on any proto. user
code cannot redefine evaluation for any value-kind.

### L1 — one Form heap kind (partially violated)

`Value` is a tagged enum: `Nil | Bool(bool) | Int(i64) | Sym(SymId) |
Form(FormId)`. the immediates have *implicit* protos via
`World::dispatch_start`. the docs allow this as an internal
optimization — *but* require the conceptual model to be honest:
every value has a proto, slots, handlers, meta.

we don't honor that. you cannot:
- attach a slot to the integer `5`.
- read the meta of a symbol.
- introspect anything about an immediate beyond what `type-of` reveals.

NaN-boxing was the docs' eventual answer. we have an honest tagged
union pretending to be NaN-boxed. close enough for now, but the
pretense is showing.

### L5 — source is canonical, bytecode derived (mostly aspirational)

we *do* keep `chunk.source: Option<Value>`. but:

- nothing reads it. there is no `[m source]` accessor.
- editing a method's source does not invalidate or regenerate
  bytecode. once compiled, edits are ignored.
- methods aren't Forms (see L6 below) so the source-form isn't
  reachable from moof anyway.

### L6 — reflection is total (severely violated)

zero of the substrate-promised reflection methods exist as actual
sendable methods:

| substrate-laws.md / reflection-contract.md says | reality |
|---|---|
| `[v proto]` | doesn't exist; only `(type-of v)` returns a name-symbol |
| `[v slots]` | doesn't exist; `(slot v name)` reads one |
| `[v handlers]` | doesn't exist |
| `[v meta]` | doesn't exist; meta field is unused |
| `[v source]` | doesn't exist |
| `[v identity]` | doesn't exist |
| `[m bytecodes]` / `[m disassemble]` | doesn't exist |
| `[m caps-required]` / `[m purity]` | doesn't exist |
| `[frame locals]` / `[frame resume!]` / etc. | frames aren't Forms |

`Object` (the root proto) has zero handlers installed. nothing
inherits anything. the proto chain is real but empty above each leaf
type.

### L7–L16 (vats, isolation, purity, journaling) — N/A

we don't have vats, persistence, or capability discipline. nothing
to violate yet, but also nothing to honor. the docs describe these
in detail; the implementation is silent on all of them.

## docs/code mismatches by file

documents that describe features the code does not have at all:

- `docs/concepts/tables.md` — Tables (lua+APL hybrid). **not implemented.**
- `docs/concepts/types.md` — types as Forms with `:satisfies?`. **not implemented.**
- `docs/concepts/queries.md` — datalog rules + queries. **not implemented.**
- `docs/concepts/data-sources.md` — universal i/o protocol. **not implemented.**
- `docs/concepts/persistence.md` — per-vat lmdb store + journal. **not implemented.**
- `docs/concepts/vats.md` — vats, mailboxes, supervisors. **not implemented.**
- `docs/concepts/references.md` — slot/id/far/path taxonomy, far-refs. **only id-refs (form-ids) and (informally) slot-refs exist.**
- `docs/concepts/capabilities.md` — `$cap` discipline. **`println` is a global; nothing is unforgeable.**
- `docs/concepts/time-and-journal.md` — journaling, undo, time-travel. **not implemented.**
- `docs/concepts/moldability.md` — promised user-overridable inspector views, frame edit-and-continue. **not implemented.**

documents that describe surfaces the reader doesn't accept:

- `docs/syntax/object-literals.md` — `{Counter count: 0 [incr] body}`.
  **`{}` brackets aren't parsed at all.**
- `docs/syntax/literals.md` — `#[1 2 3]` Tables, `#Date "..."` tagged
  literals, `1.5` floats, `1/3` rationals, `3+4i` complex,
  triple-quoted strings, raw strings, char literals (`#\h`),
  underscores in number literals. **none of these are implemented.**
- `docs/syntax/string-interpolation.md` — `"hi #{name}"` ruby-style.
  **interpolation isn't implemented.** strings are dumb literals.
- `docs/syntax/methods-and-handlers.md` — `[header] body` method
  shape using send-brackets. **we use `(name (params) body)` parens
  syntax for handlers.**
- `docs/syntax/binding-and-defs.md` — multi-clause pattern-matched
  defs. **single-clause only.**
- `docs/syntax/sigils.md` — `,` `,@` (unquote-splice), `\`` (quasiquote),
  `?foo` (logic vars), `$foo` (caps). **none of these are implemented.**
- `docs/concepts/blocks-and-patterns.md` — patterns everywhere
  (literals, type guards, predicate guards, list/Table destructuring).
  **no pattern matching anywhere.**

## architectural flaws (independent of the docs)

these would bother me even if the docs didn't exist.

1. **`Form` is structurally fat for tiny use-cases.** every Form has
   three HashMaps (slots, handlers, meta) plus an Option<Box<str>>.
   each cons-cell allocates ~200 bytes for what should be 24. a
   1000-element list is ~200kB. fine for now; will not survive any
   real workload.

2. **Methods are not Forms.** `MethodImpl` is a rust enum
   (`Native(fn)` | `Bytecode { chunk, captured_env, params }`). it
   lives in the heap's `handlers` HashMap. you cannot:
   - send a message to a method
   - introspect a method's source from moof
   - replace a method-impl with another value
   this contradicts the four-faces-of-Form vision directly. methods
   *should* be Forms (closures with proto = Method).

3. **Chunks are not Forms.** `Chunk` is a separate `struct` with `Vec<Op>`,
   `Vec<Value>`, `Vec<ICache>`, `Vec<ChunkId>`. they live in
   `world.chunks: Vec<Chunk>`, indexed by `ChunkId`. completely off
   the heap. invisible to reflection. cannot be introspected,
   serialized, or moved.

4. **Closures store env+params twice.** once as `slots` for
   "reflection" (which doesn't actually work — see L6), once inside
   `MethodImpl::Bytecode { captured_env, params }` for actual
   dispatch. two sources of truth = bugs waiting to happen.

5. **No tail-call optimization.** recursive moof functions blow the
   rust stack. `(factorial 5000)` overflows. the iter-fib in
   examples works only because it's `(< n 2)`-bounded shallow.

6. **Heap is append-only.** never reclaimed. every parsed s-expr
   keeps allocating cons-cells. every closure invocation allocates a
   new env. multi-minute programs leak indefinitely.

7. **Inline cache invalidation is unimplemented.** ICs are populated
   on first dispatch, never invalidated. if user code mutates a
   proto's handler table at runtime (which they can do via
   `proto-set-handler!`), existing IC slots silently keep returning
   the old method. **this is a correctness bug, not just a perf
   gap.**

8. **No `become:`.** identity swap is foundational for live editing.

9. **No `does-not-understand:` mechanism.** missing handlers raise a
   substrate error. user code can't intercept. proxies, smart
   wrappers, and lazy-loaded objects are all impossible.

10. **`new` doesn't invoke `:initialize`.** a smalltalk
    `[Counter new]` should send `:new`, which sends `:initialize`.
    we do neither. user code does `(slot-set! c 'count 0)` manually.

11. **No `super`.** override-and-delegate is impossible.

12. **Reader and compiler are rust monoliths.** the docs describe
    a tiny "bootstrap parser" that loads the *real* moof parser. we
    have a 297-line rust reader and a 480-line rust compiler that do
    everything. no moof reader, no moof compiler.

13. **Bootstrap order is brittle.** `lib/bootstrap.moof` loads top
    to bottom. forward references between defs would crash. ranges
    are defined before length, length before reverse, etc. — by
    careful manual ordering. a real module system would prevent this.

14. **No way to extend a built-in proto from moof.** you cannot, from
    moof, add a method to Integer. `proto-set-handler!` would let
    you, but the proto's handler-table is keyed on `MethodImpl`,
    which is a rust type — moof code only produces closures, and the
    builtin `proto-set-handler!` extracts the closure's
    `MethodImpl::Bytecode` to install. so it works, but only by an
    accidental coincidence of the implementation. the *moof-level
    contract* is unspecified.

15. **The compiler accepts ambiguous syntax.** `(set! x y)` works
    because we hardcoded `set!` as a special form. but `(my-set! x y)`
    where `my-set!` is a user-defined fn also works. visually they
    are nearly the same; the compiler's behavior is invisible. user
    code has no way to know whether a name is "magic" without reading
    the rust source.

16. **`self` is a regular name, shadow-able by accident.** a
    `(let ((self ...)) ...)` quietly shadows the auto-bound self.
    not catastrophic but should be a documented restriction or a
    distinct syntactic form (`.foo` per the docs is the intended
    answer; not implemented).

17. **No quasiquote / unquote.** macros are practically impossible to
    write. without macros, defop is impractical. without defop,
    defproto can't move to moof. without that, the compiler stays
    rust-only.

18. **17 tests is not enough.** no tests for the message-send paths
    (binary, multi-keyword, cascade — cascade isn't even
    implemented). no tests for proto mutation. no tests for error
    propagation. no tests for edge cases.

## what would a *real* phase-2 substrate look like

(ordered by impact-per-effort.)

### tier 1 — making the substrate honest

these would close most of the L1–L6 violations. without them, every
later phase compounds the dishonesty.

#### 1.1 — methods as Forms

replace `MethodImpl` with: a method is a Form. `proto.handlers` maps
selector → `Value::Form(closure_id)`. dispatch sends `:invoke` (or
`:call`) to the method-Form with `(receiver, args...)` as args.
native methods become Forms whose proto carries a `:invoke` handler
that's a rust trampoline reading the form's slot for a
`NativeFnId`.

once methods are Forms:
- `[m source]` works (via the form's slot)
- `[m bytecodes]` works
- user code can construct method Forms from moof and install them
- there is *one* substrate concept (Form) instead of two (Form +
  MethodImpl)

cost: substantial refactor of `vm::send_dispatch`, `make_closure`,
`proto-set-handler!`. ~300 LoC of rust changed.

#### 1.2 — chunks as Forms

replace `Chunk: Vec<Op> + Vec<Value> + Vec<ICache>` with: a chunk is
a Form whose slots are `:ops`, `:consts`, `:ics`, `:nested`,
`:source`. opcodes are themselves small Forms (`{Op-Send sel: 'foo
arity: 2 ic-idx: 0}`).

the bytecode interpreter loads ops by reading the chunk-Form's
`:ops` Table. chunks become first-class moof values: serializable,
inspectable, modifiable.

cost: significant. heap traffic increases (every op is a Form). the
interpreter slows down by some constant factor. ~400 LoC changed.

we can defer this if the perf hit is intolerable, but the docs claim
chunks are reflection-visible. they aren't. fix or revise.

#### 1.3 — reflection methods on Object

install on the `Object` proto (so all Forms inherit):
- `:proto`, `:slots`, `:handlers`, `:meta`, `:source`, `:identity`,
  `:to-string`, `:inspect`, `:=`, `:is`

each is a small native method. the slots/handlers/meta accessors
return Forms wrapping the underlying HashMap (read-only views).

cost: ~150 LoC of rust + a way to expose HashMaps as Tables.

after this, **anything in the world is introspectable from moof**.
this is the moldable promise made real for the first time.

#### 1.4 — eval-via-`:eval`

every Form's proto has an `:eval` handler. the compiler dispatches
via send rather than hardcoded `match`. user-defined operatives
become possible by defining new protos with custom `:eval`.

this is the kernel-pure move (shutt 2010, maru). it's the most
important architectural shift available.

cost: ~200 LoC of rust + a careful refactor of compile_form.
performance hit (every eval is a send) is recoverable via inline
caching of the proto's `:eval` resolution.

### tier 2 — closing the surface gaps

- **`{Proto …}` object literals** in the reader and compiler.
  ~100 LoC.
- **`:to-string` send dispatch in the printer.** so user types print
  themselves their way. ~30 LoC.
- **multi-clause pattern-matched defs.** the substrate's biggest
  ergonomic win. ~300 LoC of compiler work.
- **quasiquote `\`` `,` `,@`** in the reader, with semantic support
  in the compiler. ~150 LoC.
- **`defop` / macros** — operatives that receive unevaluated forms
  and return new forms. requires quasiquote. ~200 LoC.
- **proper string interpolation `#{expr}`** in the reader. ~80 LoC.
- **`super` send.** ~50 LoC of compiler + 50 of vm.
- **`become:`** at the heap level. ~80 LoC, but cascades through
  every place that holds a FormId.
- **`does-not-understand:` extension hook.** ~30 LoC.
- **inline cache invalidation on proto edit.** ~80 LoC.
- **tail call optimization.** ~150 LoC of vm work; substantial care
  required for closure environments.

### tier 3 — vision features (each is a project)

- **Tables** (concepts/tables.md) — would need ~600 LoC for the
  basic positional+keyed type plus operations.
- **Types as Forms with `:satisfies?`** — ~400 LoC for substrate
  + analyzer in moof later.
- **Datalog queries** — ~800 LoC for a basic relational + rule
  engine.
- **Data sources** — protocol + a few rust leaves (file, stdin,
  timer, channel). ~500 LoC.
- **Vats + scheduler + mailboxes** — the big one. ~1500 LoC for a
  single-process scheduler with cross-vat far-refs.
- **Per-vat persistence** (lmdb + WAL) — ~800 LoC, requires vats.
- **Capabilities** — once `$cap` discipline is enforced, ~200 LoC.
- **Distribution / federation** — much later, builds on vats +
  far-refs.

## proposed plan for next session

**phase 2.5 — substrate honesty.** before adding any vision-tier
features, fix the foundations. forcing function: every claim in
`laws/substrate-laws.md` is either upheld or has a docs note saying
"deferred to phase X."

deliverables (rough order):

1. **install reflection methods on Object** (tier 1.3). closes L6 to
   a respectable degree. one afternoon.

2. **methods as Forms** (tier 1.1). closes a chunk of L1, L3, L6.
   one to two days.

3. **eval-via-`:eval`** (tier 1.4). closes L4. one to two days.

4. **inline cache invalidation** (architectural flaw #7). closes a
   correctness bug. half a day.

5. **`does-not-understand:` hook** (architectural flaw #9). half a
   day.

6. **multi-clause pattern-matched defs** (tier 2). this single
   feature is the moof "feel" the docs promise — closing it makes
   moof code look like the snippets in the docs. two to three days.

7. **rewrite test suite** to cover send-brackets, defproto, proto
   mutation, error paths. half a day.

after phase 2.5:

- the substrate is honest about what it is.
- moof code can introspect itself.
- user types can override `:eval`, `:does-not-understand`, etc.
- the gap between what the docs say and what the code does is
  manageable.

then, **phase 3 — moldable from outside** can begin: object
literals, quasiquote, defop, real macros. with the substrate honest,
these become small additions rather than substrate-shaking changes.

## what we should *not* do next session

- **vats / persistence / distribution.** the substrate isn't ready
  to host them. they would compound on top of an unsteady foundation.
  see them as phase 4+.
- **Tables / Types / Queries.** these are tier 3 features that
  *also* depend on a substrate that's honest first.
- **performance work.** the heap is wasteful, dispatch is slow, ICs
  don't invalidate. but optimizing a substrate we'll restructure is
  wasted work. honest first, fast later.
- **more stdlib.** the stdlib is fine; adding more functions doesn't
  fix anything underneath.

## stop sugarcoating

honest summary, for future-us reading this in the morning:

we shipped a working lispy interpreter with smalltalk-flavored sugar
and called it "phase 2." the docs in `docs/` describe a substrate
this implementation does not, in fact, embody. we know which bits
are aspirational; we should make the docs reflect that, OR fix the
implementation. doing both — letting the docs claim things the code
doesn't deliver — is the v3 mistake all over again.

phase 2.5 is "make the docs honest *or* fix the code." pick. then
the rest of the roadmap can proceed without continual debt.

`>.<` softly. and apologetically. ૮ ◞ ﻌ ◟ ა
