# lineage

**type:** vision

> moof is not a new idea. it's an old idea, reassembled with
> materials that weren't available the first time around. this
> doc names the debts: who invented what, what moof takes, what
> moof leaves behind.

---

## smalltalk

**what we take**
- the image: the running system persists as a single artifact.
- prototype delegation: objects inherit behavior from other
  objects. no separate class/metaclass layer.
- the browser as an object in the image. smalltalk's class
  browser was written in smalltalk. you could open it up and see.
- `doesNotUnderstand:`. a message-not-found is a MESSAGE, not a
  crash. enables proxies, DSLs, and reflection.
- late binding all the way down. behavior is decided at send-time,
  not compile-time.
- the commitment to no privileged layer.

**what we leave behind**
- classes-and-metaclasses. moof uses plain prototypes. every
  object is a first-class thing; the "class" is just another object
  pointed at by `proto`.
- the single-process assumption. smalltalk images run in one heap,
  one thread (classically). moof has vats.

**what we complicate, not abandon**
- morphic's UI. moof keeps the **halo gesture** and the **live-
  editing commitment** wholesale. but the visual style is an open
  question, not a closed one. morphic was a specific aesthetic —
  big rounded pixelated morphs, physicality emphasized. moof's
  canvas can be more like that (skeuomorphic, physical
  affordances that communicate purpose) OR more typographic
  (quiet, text-forward) — whichever a view commits to. rendering
  is per-protocol (see [horizons.md](horizons.md)), so a recipe
  renders one way, a file system renders another. the moof
  commitment is "views are messages"; the aesthetic menu is
  open.

---

## plan 9

**what we take**
- **everything is addressable by path.** `/vats/42/mailbox`, not
  "get mailbox from vat 42 via API." paths are first-class values.
- per-process namespaces. each vat gets a namespace assembled at
  spawn time; no global authority.
- union mounts. compose namespaces by mounting one under another.
- 9P in spirit: send a path, receive a value; remote and local
  look the same.

**what we generalize**
- the file abstraction. moof's unit of addressability is the
  object, not the byte stream. we generalize plan 9's insight from
  files to arbitrary values. "everything is a file" becomes
  "everything is an object at some path." we keep the
  stream-oriented composition idea (see unix, below); we just
  apply it to typed values instead of byte-streams.

**what we leave behind**
- the C-level machinery. plan 9's implementation is deep in the
  kernel; moof's namespace is a moof object.

---

## erlang / BEAM

**what we take**
- **processes everywhere.** vats are erlang processes. cheap,
  isolated, no shared memory, communicate by async messages.
- **immutable-by-default, wholesale.** erlang values are
  immutable; moof's are immutable too. this is a commitment,
  not an optimization — no in-place mutation anywhere. servers
  exist as the *structured exception*: mutable state behind a vat
  boundary, accessed only via Update deltas. see
  [../concepts/effects.md](../concepts/effects.md).
- preemptive scheduling with fuel-based reductions. long-running
  vats can't starve others.
- let-it-crash. a vat can die without taking down the image.
- **OTP's vocabulary.** erlang's Open Telecom Platform isn't just
  telecoms — it's a naming system and a set of idioms for
  supervision. we steal the vocabulary directly:
  - `supervisor` — a vat that watches child vats, restarts on
    policy.
  - restart strategies: `always`, `on-failure`, `never`,
    `escalate`. (ok rest_for_one / one_for_all / one_for_one
    mostly not — those are specific to erlang's linked-pair
    model; we adapt what's idiomatically moof.)
  - `application` — a bundle of supervised services that
    starts/stops together.
  - named patterns like gen_server (our `defserver`), gen_event
    (our reactive signals), gen_statem (our finite-state
    defservers).
  - orderly startup/shutdown sequences.
  OTP's real gift is that decades of production systems refined
  these patterns; we'd be foolish to reinvent them.
- the supervision tree as a first-class design concept.
- hot code swapping. change a handler on a prototype, every
  delegating object gets the new behavior, no restart.

**what we leave behind**
- the syntax. erlang's is terse and unusual. moof's is lisp-shaped
  because the reader is a different concern.
- erlang's *linked-pair* restart semantics. the more complex
  multi-process restart strategies (rest_for_one, one_for_all)
  are specific to erlang's process-linking model. moof's
  supervision uses the same vocabulary but adapts the specifics
  to the vat model.

---

## kay / engelbart / atkinson

the authoring-vision triumvirate. the *why* of moof.

**alan kay** — the dynabook. personal dynamic medium. the system
is made of objects you inspect and modify while it runs. "doing with
images makes symbols." programming is a form of literacy that should
be available to everyone.

**doug engelbart** — NLS and "the mother of all demos" (1968).
augmentation of human intellect. tools that amplify individual and
collaborative thought. view-control: the document is one thing, the
view is another. bootstrapping: use the tools to build better tools.

**bill atkinson** — hypercard. the script is next to the affordance.
authoring and using are the same gesture, distinguished only by how
deep you choose to go. tens of thousands of non-programmers built
working applications because the ladder was continuous.

**what we take from all three**
- the continuous ladder: no mode boundary between using and
  extending.
- the commitment to authoring-for-all: moof is not for
  programmers, it's for anyone willing to click "conform."
- the substrate/tool/document identity: one material at one level.
- view-control as a first-class concept: the object is the thing,
  views are how you look at it, and looking differently is a
  gesture you perform.
- bootstrapping: moof is built with moof, progressively. the goal
  is to get rust out of the way.

**what we update**
- the canvas is computationally richer. not bitmap morphs;
  vector-first, zoomable infinite space with smooth LOD.
- the agent is a first-class participant. kay didn't anticipate
  LLMs; hypercard didn't have them; we do, and the agent-in-a-vat
  is part of the design now.
- federation is assumed. engelbart dreamed of it; we have the
  plumbing (content-addressing, FarRefs, signatures).

---

## E language (mark miller)

**what we take**
- **capabilities.** a reference *is* permission. holding the
  reference means you can send it messages. no ambient authority.
  no global namespace of resources.
- near-refs and far-refs. same-vat sends are synchronous;
  cross-vat sends are eventual and return promises.
- promise pipelining. `[[a <- b] <- c]` pipelines across the
  network without explicit .then() chaining.
- membranes and facets. intercept all messages crossing a trust
  boundary. transform, log, allow, deny.

**what we leave behind**
- the specific syntax. E's is its own thing.
- the standalone distribution. moof's network layer is a thin
  delta over vat-local sends, not a separate system.

---

## haskell

**what we take**
- **typeclasses become protocols.** `Eq`, `Ord`, `Functor`,
  `Foldable` — the pattern of "required methods, many derived
  methods" maps to moof's `defprotocol` with `require` + `provide`.
- effects as values. haskell's IO monad as object references:
  effects are capability sends that return Acts (moof's version of
  effect descriptors).
- pattern matching. moof's `match` is a derived form; destructure
  by shape.
- laziness where you want it. streams are objects with a `next`
  handler that computes on demand.

**what we complicate, not abandon**
- static type-checking is not present as a core commitment —
  moof's protocols and the vat boundary do most of what haskell's
  types do, without foreclosing the dynamic floor. but we don't
  *rule out* an optional typed layer. a haskell-style or
  typescript-style type system layered on moof — where you can
  annotate, infer, and check — is a viable future wave, and the
  protocol machinery is already shaped to accept it. see
  [horizons.md](horizons.md) and the archived
  [type-system.md](../archive/type-system.md) for the sketch.
  the floor stays low; the ceiling can still rise.
- purity enforcement at the type level. moof enforces purity
  through the vat boundary: if you hold no effectful references,
  you can't have effects. pragmatic, not theoretical. a typed
  layer could add algebraic-effect or IO-monad-style reasoning
  on top, but the substrate commitment stays capability-based.

---

## ruby

**what we take**
- **everything is an object.** no primitives. `3.times {}` works
  because Integer is an object.
- blocks. `|x| [x + 1]` is a closure; pass it to a method; the
  method sends `call:` to it. blocks are values.
- `method_missing` → `doesNotUnderstand:`. the same idea with
  fewer apologies.
- open prototypes. add a handler to Integer and every integer
  gains that behavior. smalltalk said this first; ruby made it
  ergonomic.

**what we leave behind**
- mutation-everywhere. ruby's default is mutable; moof's is
  immutable.
- the syntax. we're lisp-shaped for uniformity.

---

## SQL / relational

**what we take**
- **objects as rows.** fixed-shape objects with public slots *are*
  rows. a collection of same-shaped objects is a table.
- query operations as message sends. `where:`, `select:`,
  `groupBy:`, `orderBy:`, `join:on:`, `aggregate:` on collections.
  the object model IS the query language.
- indexes as computed views. maintained live by the reactive
  layer.
- **schemas when you want them.** SQL made schemas mandatory.
  noSQL swung to schema-free. moof wants both: the default is
  schema-emergent (you build objects, shapes coalesce), but you
  can declare explicit schemas — a prototype with required
  slot-name-and-type contracts, queryable at runtime — whenever
  the rigor pays off. see
  [../concepts/schemas.md](../concepts/schemas.md).

**what we leave behind**
- schema-first as the *only* mode. mandatory up-front schemas
  break fluidity for exploratory work.
- the wire protocol. SQL's text-based query language is an
  accident of history. moof queries are compositions of message
  sends.

---

## git / IPLD

**what we take**
- **content-addressed storage.** every immutable value has a hash
  derived from canonical serialization. identical content = same
  hash. history is a chain of hashes.
- merkle DAGs for sync. two images compare root hashes. if they
  differ, recurse into children. exchange only what's missing.
  this is git fetch; it's also moof sync.
- snapshots as tags. "save a checkpoint" = record a root hash.
  "restore" = reconstruct.

**what we leave behind**
- git's user model. branches and commits as a primary interface.
  moof treats the content-addressing as plumbing; the UX is object
  inspection and time-travel, not commit logs.
- the blob-of-bytes assumption. git tracks files; moof tracks
  typed values with known shapes.

---

## unix

**what we take**
- composability. small tools that do one thing well, combined via
  pipelines.
- **stream-centric design.** unix's real gift isn't text streams —
  it's treating *everything as a flow of values*: files, input
  events, sensor data, log output, mailboxes, rendered frames.
  moof embraces streams as a first-class pattern. see
  [../concepts/streams.md](../concepts/streams.md). the flow/
  directory — transducers, streams, reactive signals — is
  where moof operationalizes this.
- textual representation as a fallback. every moof value has a
  human-readable `describe` — text is the universal interop
  surface, even when typed values are primary.
- the philosophy of making the substrate into a workshop.

**what we generalize**
- the text-stream assumption. moof streams carry typed values, not
  just bytes. but the *stream as composition primitive* stays —
  a mailbox is a stream, a sensor is a stream, a canvas event
  source is a stream. pipelines apply to all of them.

**what we leave behind**
- the process-as-isolation model. moof uses vats, which are both
  cheaper and better-integrated than OS processes.

---

## what moof adds

these are the things none of the above had at once that moof tries
to combine:

1. **cheap fork-from-history.** git gave us content-addressing but
   git doesn't run programs. smalltalk's image was live but not
   content-addressed.
2. **LLM as substrate participant.** none of our ancestors had this.
   the agent lives in a vat, bounded by capabilities, with full
   awareness of the image.
3. **vats + capabilities + image all at once.** erlang has vats
   but no image. smalltalk has an image but no vats. E has
   capabilities but no persistence. moof combines all three.
4. **authoring-for-all.** kay, engelbart, and atkinson each got
   part of this. moof is a second attempt to get the whole thing,
   with better materials.

---

## the point of the list

moof is not novel. it's a specific reassembly. each piece is
battle-tested elsewhere. the risk isn't "will it work?" — each
piece works. the risk is "can these pieces be combined in one
coherent substrate without the seams becoming friction?"

that's the design problem. this doc names the pieces so we can
argue clearly about them.
