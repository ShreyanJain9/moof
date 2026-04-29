# manifesto

> *"the best way to predict the future is to invent it."* — alan kay

## the thesis

moof v4 is an environment, not a language. this is a load-bearing
distinction. previous attempts (v1–v3) drifted toward
language-thinking — what does this expression mean, what's the
type, what's the evaluation strategy — and accumulated infrastructure
(plugin abis, merkle persistence, jit) that served the language
but not the *experience of inhabiting a place*.

we are starting v4 from the other direction. we ask:

> *what does it feel like to live inside this world for ten thousand hours?*

every architectural decision — heap, dispatch, persistence,
concurrency, syntax, distribution — is judged by that question.

## the lineage we are honoring

three families of system have asked this question seriously:

1. **the smalltalk family** — smalltalk-72/76/80 (kay, ingalls,
   goldberg, robson), self (ungar, smith), morphic (maloney, smith),
   pharo, and most recently glamorous toolkit (gîrba). their answer:
   the world is the program. the image is what you save. every
   tool you use is just an object you can inspect and rewrite.

2. **the actor / message-passing family** — hewitt's actor model,
   erlang/OTP (armstrong, virding, williams), e (miller, close,
   stiegler), croquet (kay, reed, lombardi, ducasse, smith),
   ambienttalk (van cutsem, dedecker), pony (clebsch). their answer:
   a world is many small lives in conversation. concurrency, fault
   tolerance, and distribution are one design problem, not three.

3. **the lisp / homoiconic family** — mccarthy's lisp, scheme
   (steele, sussman), common lisp (steele et al.), maru (piumarta),
   shutt's kernel, clojure (hickey). their answer: the program is
   data the program can read. the substrate is a tiny seed; the
   world grows itself.

each family is incomplete on its own. smalltalks were lonely
single-image worlds; actors had no live-image culture; lisps lacked
the moldable inspector culture. **moof v4 is the synthesis we
believe is possible now that wasn't in 1980 — and that nobody else
has assembled.**

we also lean on:

- **HyperCard** (atkinson 1987) for direct-manipulation accessibility,
- **Genera** (symbolics) for "every thing on screen is the actual thing,"
- **APL/J/K** (iverson, hui, whitney) for rank-polymorphic data,
- **datalog/prolog** (colmerauer; ullman) for declarative relations,
- **datomic** (hickey) for time-as-a-first-class-axis,
- **lua** (ierusalimschy) for the table as universal collection,
- **ruby** (matsumoto) for friendliness without compromise,
- **haskell** (peyton-jones et al.) for pattern-matched-clauses-as-rhythm.

(see `vision/lineage.md` for full attributions.)

## the four faces of Form

we believe the substrate primitive is a single thing — the **Form** —
that simultaneously presents:

- a **structure** face (head + args; what lisp made central),
- an **identity** face (proto + slots + handlers; what smalltalk and
  self made central),
- a **liveness** face (mailbox + behavior; what erlang and e made central),
- a **history** face (meta + provenance + journal; what datomic and
  modern databases made central).

every value has all four faces. most use one or two. the four are
not arbitrary — they correspond to the four traditions above. the
synthesis claim is that you can hold them in *one cell* without losing
the character of any one. this is what we are testing.

## what we believe about software

a few opinions, stated plainly:

**you are inside the world, not outside it.** "running a program" is
the wrong metaphor. you wake a world. you change it. you let it
sleep. on next wake it remembers.

**every artifact is a citizen.** a function, a window position, a
scratchpad note, a half-thought, an open inspector — they all live
in the same persistent place, on equal footing. there is no
"transient" register that loses things on quit.

**your tools should be made of the same stuff as your work.** the
inspector that shows you an object, the editor that lets you change
its method, the debugger that pauses at a frame — these are objects
in the world you inhabit. they obey the same protocols, are
introspectable the same way, and you can rewrite them.

**concurrency is not a library.** it is the shape of the world. the
keyboard is an actor. the screen is an actor. each window is an
actor. the file you opened is an actor (proxying an external
resource). this is not a performance optimization; it is the model.

**isolation is a feature, not a constraint.** the per-vat
boundary is what makes "let it crash" sane, what makes
distribution a small extension, and what makes time-travel
implementable. we accept the cost (no shared mutable memory across
vats) because the benefit is "every interesting hard problem
becomes tractable."

**source is canonical.** code that defines behavior lives as
human-readable forms. derived artifacts (bytecode, caches, indexes)
are derived. you can always edit a method by editing its source.
the substrate guarantees this.

**reflection is total.** every Form exposes its proto, slots,
handlers, source, bytecode (if any), frame state (if running),
mailbox (if a vat), journal (if persistent). nothing is hidden in
the rust line. if a piece of state cannot be inspected from inside
moof, that piece of state is in the wrong place.

## what we are not building

- a competitor to general-purpose programming languages.
- a research vehicle for a single neat idea.
- a "platform" you build on without taking a position.
- a fast-as-c runtime. (we aim for "fast enough for an environment.")

## the test

we will know we have succeeded if a person, working only from inside
moof, can:

- redefine a special form (the language is theirs).
- rewrite the inspector for a domain object (the tools are theirs).
- query the world's history relationally (the past is theirs).
- spawn a collaborator on another machine and watch them edit a method
  live (the social fabric is theirs).
- close the laptop, open it tomorrow, and find everything as they
  left it, plus the collaborator's changes (the persistence is real).

if we make this real, moof v4 is a place worth living in.

if not, we have learned, again, what does not work. and that is also
fine. but: this is the fourth attempt. the fourth time is when you
trust your taste and make something real.

`>.<` softly, but with intent.
