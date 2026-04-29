# moof v4, in one page

**moof is an environment, not a language.** the language is a feature
of the environment, not the other way around.

a moof world is a persistent, live, multi-actor place that you inhabit
and reshape from inside it. it boots from disk, runs continuously,
and saves itself as you work. you do not "run a program" — you wake
the world, change it, and let it sleep.

## what is in the world

every value is a **Form**: one heap kind that wears four faces
simultaneously — *structure* (head + args, like a lisp list),
*identity* (proto + slots + handlers, like a smalltalk object),
*liveness* (mailbox + behavior, like an erlang process), and
*history* (meta + provenance, like a database row). most values lean
on one or two faces, but the four are always available.

**vats** are the unit of concurrency, persistence, isolation, and
distribution. a vat is a process with a mailbox, a heap of forms, and
a journal on disk. within a vat, message-sends are synchronous. across
vats, sends are asynchronous and return promises. cross-vat references
are a substrate primitive from day one — federation falls out for free.

**data sources** are the universal i/o protocol. files, sockets,
keyboards, mailboxes, journals, query results — anything producing or
consuming values over time speaks the same protocol and composes with
the same combinators.

**Tables** (lua-style hybrid arrays + maps, with APL-flavored
operations) are the everyday data structure. **Lists** (cons-cell
linked sequences) are the substrate of code-as-data. **Strings**,
**Numbers**, **Symbols** are leaf types that participate in the
universal protocols.

**types** are Forms with `:satisfies?` handlers. they are optional,
gradual, and infinitely composable — refinement, structural,
dependent, intersection, union — all from one substrate hook. types
are values; the type system is moldable like everything else.

## what makes it moldable

every substrate concern that does not *have* to live in rust lives in
moof: the parser proper, the compiler, the analyzer, the type system,
the inspector, the canvas, the editor, the package system, the query
engine. the rust line provides only what cannot be expressed above
itself: heap, GC, bytecode interpreter, scheduler, transport leaves,
the `.mco` loader for rust-bridge methods, and a bootstrap parser.

this is the **maru/piumarta posture**: the substrate is a tiny seed;
the world grows itself.

## what we take from where

smalltalk's image and message-passing; self's prototypes and morphic;
kernel's $vau and operative/applicative split; io's everything-is-a-
message tree; erlang's processes, mailboxes, and let-it-crash;
e's vats and capabilities; croquet's distributed-determinism; lua's
table; APL's rank-polymorphism; datalog's rules-and-queries; haskell's
pattern-matching shape; ruby's friendliness; common lisp's homoiconic
heart. every one is cited in `vision/lineage.md`.

## non-goals

- **not** another scripting language with a repl bolted on.
- **not** a research vehicle for a single idea.
- **not** a compete-with-c performance target.
- **not** a generic "platform" with no opinions.

## the test

> can a person inside moof, using only moof, redefine the compiler,
> rewrite the inspector to suit their work, query the world's history,
> spawn a new collaborator, watch them edit a method live, and save
> the result so tomorrow's wake-up restores it all?

if yes, moof is what we wanted. if no, the substrate has lied.

see `vision/manifesto.md` for the longer version, and `concepts/`
for the specifics.
