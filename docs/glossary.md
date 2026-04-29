# glossary

> **every distinctive term moof uses, in one place. when you encounter
> a word and aren't sure what it means here, look here first.**

---

**applicative.** a callable whose arguments are evaluated *before*
being passed in. ordinary functions and methods are applicatives.
contrast: operative. (kernel: shutt 2010.)

**attenuation.** producing a smaller / more restricted version of a
capability by sending it a message. preserves the cap's protocol.
(`concepts/capabilities.md`.)

**block.** a closure literal. syntax: `|args| body`. body is a
single expression. (`concepts/blocks-and-patterns.md`.)

**bytecode.** internal representation of a method, derived from its
source-form. cached for performance. canonical truth is always
the source. (`concepts/forms.md` L5.)

**canonical encoding.** the deterministic byte representation of a
Form. used for persistence, hashing, content-addressing. same form ⇒
same bytes. (`reference/canonical-encoding.md` when written.)

**cap / capability.** an unforgeable Form authorizing some side-
effect. parameters whose names start with `$` are caps by convention.
(`concepts/capabilities.md`.)

**cascade.** a smalltalk-style chain of sends to the same receiver,
separated by `;`. returns the receiver. (`concepts/sends-and-calls.md`.)

**checkpoint.** writing a vat's current state to its store and
truncating its journal. periodic; concurrent with operation.
(`concepts/persistence.md`.)

**closure.** a Form with `proto: Closure` capturing a body and a
lexical environment. responds to `:call`. (`concepts/blocks-and-patterns.md`.)

**compiled object (`.mco`).** a one-object binary file enabling rust-
implemented methods. *only* used for the rust→moof bridge.
(`concepts/compiled-objects.md`.)

**cons-cell.** a Form representing one node of a List: head + tail.
(`concepts/lists.md`.)

**data source.** the universal i/o protocol. sources of values over
time. (`concepts/data-sources.md`.)

**datalog.** a declarative logic-programming subset focused on
relations and rules. moof's query language draws from it.
(`concepts/queries.md`.)

**defop.** define an operative. user-defined "special form."

**defproto.** define a proto. introduces a new type / class-equivalent.

**delegation.** the proto-chain mechanism by which an object
inherits methods and slots from its proto. (`concepts/objects-and-protos.md`.)

**doesNotUnderstand.** smalltalk-style fallback selector invoked when
a sent selector finds no handler in the proto chain. extension
hook. (`concepts/objects-and-protos.md`.)

**environment.** a Form representing a lexical scope; has bindings
and parent. closures capture environments.

**far-ref.** a reference that crosses a vat boundary. async-only;
returns a promise on send. `(vat-id, form-id, cap-token)` tuple.
(`concepts/references.md`.)

**fexpr.** lisp-historical name for an operative; a function whose
arguments are not evaluated.

**form.** the universal substrate primitive. one heap kind with four
faces: structure, identity, liveness, history. (`concepts/forms.md`.)

**forwarding.** redirection of a Form's identity to another, via
`become:` or proxy. (`concepts/objects-and-protos.md`.)

**frame.** a Form representing a stack frame during execution. has
locals, self, method-ref, pc. (`laws/reflection-contract.md` R3.)

**handler.** a method in a proto's handler table; the implementation
of a selector. (`concepts/objects-and-protos.md`.)

**id-ref.** a reference to a Form by its vat-local heap-id. only
meaningful within one vat. (`concepts/references.md`.)

**identity.** the heap-id of a Form, vat-local, stable.
(`laws/substrate-laws.md` L11.)

**image.** the persistent state of the world (or of a single vat).
moof v4: per-vat directories, not a single monolithic file.
(`concepts/image-and-world.md`.)

**inline cache.** a substrate-level cache at every send-site,
recording the resolved (proto, handler) for fast re-dispatch.
(`concepts/sends-and-calls.md`.)

**isolation.** the property that vats do not share mutable state.
crossing requires far-ref. (`laws/isolation-laws.md`.)

**journal.** per-vat write-ahead-log of mutations. append-only data
source. (`concepts/persistence.md`, `concepts/time-and-journal.md`.)

**list.** linked cons-cell sequence. `'(1 2 3)`. distinct from Table.
(`concepts/lists.md`.)

**logic variable.** a name beginning with `?` interpreted by `(query)`
and `(rule)` operatives as a placeholder for unification. outside
those contexts, it's a regular identifier. (`concepts/queries.md`.)

**mailbox.** a vat's inbox (and optionally outbox), implemented as a
data source. (`concepts/vats.md`.)

**meta.** the metadata Table on a Form. holds source-loc, doc,
journal-id, type, etc. extensible by user code.
(`laws/reflection-contract.md` R7.)

**moldable.** the property that the substrate's tools (inspector,
debugger, browser) are themselves objects in the world,
inspectable and modifiable. (`concepts/moldability.md`.)

**moof.** this project. lowercase. friendly. fourth attempt.

**multi-clause.** a definition with multiple `|pattern| body`
clauses, dispatched by pattern matching.
(`concepts/blocks-and-patterns.md`.)

**operative.** a callable whose arguments are passed *unevaluated*.
used for special forms, macros, fexprs. (kernel: shutt 2010.)

**path / path-ref.** a named address in the world's namespace.
resolves to id-ref or far-ref. `#Path "/users/shreyan/notes/today"`.

**polymorphic inline cache (PIC).** a multi-entry inline cache
handling sites that see multiple proto-types. (self: hölzle et al.)

**promise.** a Form representing a not-yet-resolved value. returned
by every cross-vat send. (`concepts/references.md`.)

**proto.** the immediate delegation parent of a Form.
(`concepts/objects-and-protos.md`.)

**purity.** a function is pure iff it receives no `$cap` and uses
no cap-requiring operations. (`laws/purity-and-effects.md`.)

**quasiquote.** a quoted form with the ability to splice values via
`,` (unquote) and `,@` (unquote-splice). (`syntax/literals.md`.)

**quote.** evaluate to the form itself, not its evaluation. `'foo`
or `(quote foo)`.

**reflection.** introspection of Forms. universal, total, guaranteed
by the substrate. (`laws/reflection-contract.md`.)

**routing table.** the substrate's vat-id → location map. enables
distribution. (`laws/isolation-laws.md` I9.)

**selector.** the name of a message. always a symbol. for keyword
sends, the concatenation of keyword markers (e.g., `:at:put:`).

**self.** the receiver in a method body. an implicit name. accessed
via `.foo` shorthand for `[self foo]`.

**send.** the universal verb. invoke a handler on a receiver with
args. `[recv selector args]`. (`concepts/sends-and-calls.md`.)

**slot.** a named binding in a Form's slot-table. mutable.

**slot-ref.** a reference to a specific slot of a specific Form.
within one vat. (`concepts/references.md`.)

**special form.** a syntactic construct with non-applicative
semantics (if, let, def, quote, …). in moof, all special forms are
operatives — first-class, user-extensible.

**super.** in a method body, `super` denotes "the proto-chain start
above the proto where this method was found." for delegating to
parent's behavior.

**supervisor.** a vat managing the lifecycle of other vats.
(`concepts/vats.md`.)

**substrate.** the rust line: heap, GC, scheduler, bytecode
interpreter, primitives. tiny by intent. (`laws/substrate-laws.md`.)

**table.** moof's universal collection: array + map hybrid, APL-
flavored. `#[1 2 3 'name => "ada"]`. distinct from List.
(`concepts/tables.md`.)

**tagged literal.** a literal preceded by `#Tag` that the proto
interprets via its `:read-literal` handler. extensible.
(`syntax/literals.md`.)

**type.** a Form with `:satisfies?` handler. optional, gradual,
composable. (`concepts/types.md`.)

**vat.** the unit of concurrency, isolation, persistence, and
distribution. one heap, one mailbox, one journal, one supervisor.
(`concepts/vats.md`.)

**world.** the totality of vats running together (per process or
distributed). has a root supervisor and a path-table.
(`concepts/image-and-world.md`.)

**WAL (write-ahead log).** the journal. append-only record of
mutations.

---

terms encountered in implementation but not used at the user level
(e.g., specific opcodes, lmdb internals) live in `reference/` files
when those exist.
