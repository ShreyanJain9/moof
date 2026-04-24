# manifesto

**type:** vision

> moof is a personal dynamic medium — a living, accretive, shareable
> objectspace where the tools and the content are made of the same
> material, and everyone is an author.

---

## 1. the problem

modern software eats itself. your 2018 workflow broke in 2020. your
2020 workflow broke in 2022. the SaaS treadmill demands constant
adaptation to vendor choices you didn't make. the divide between
users and programmers is wide and growing — most people's
relationship to computing is one of consumption, a series of pre-made
apps you can only modify within the narrow parameters someone else
chose.

this wasn't always the case. smalltalk had a system made entirely of
objects you could inspect and modify *while it ran*. hypercard let
tens of thousands of non-programmers build working applications on
their own terms. emacs let you reshape your editor with a few lines
of lisp. plan-9 treated every system resource — files, processes,
network connections — as a tree of named values you could compose.
unix, at its weird best, was a workshop.

all of these systems shared a property modern software has almost
completely lost: **the substrate, the tool, and the document are the
same material, accessible at the same level.** no app/document
divide. no developer/user divide. no "press ctrl-shift-alt-f12 to
open the developer console." the system is its own authoring
environment.

moof's bet is that this is still the right direction, and that the
reason modern systems don't work this way is accident, not necessity.
we have better concurrency primitives than smalltalk did. we have
content addressing that git made practical. we have cheap disks that
let us keep every state we ever computed. we have LLMs that can sit
in a vat and collaborate with us on the substrate itself. the
ingredients for the dynabook are finally here. nobody's assembled
them.

moof is an attempt to assemble them.

---

## 2. the thesis

moof's thesis fits in a sentence:

> **one substrate, made of objects, persistent by default, where
> sending a message is the only operation and holding a reference
> is the only authority.**

unpacked:

- **one substrate.** not a language with a runtime and libraries
  and an IDE. one thing, inseparable. the code, the data, and the
  tools are the same material. you can't hold "just the code" —
  it comes with its runtime state, its dependencies, its history.
- **made of objects.** the object is the unit of meaning. integers,
  text, functions, lists, maps, vats, the canvas, the agent — all
  objects. they have slots (data) and handlers (behavior) and they
  delegate to prototypes. there are no special cases. there is one
  semantic type: Object.
- **persistent by default.** when you create a value, it persists.
  not because you saved it — because persistence is the default.
  close your laptop, open it, everything is still there, exactly
  where you left it, including what was in flight.
- **message sends are the one operation.** `[3 + 4]` is a message
  send. `(f x)` desugars to `[f call: x]`. `obj.x` is
  `[obj slotAt: 'x]`. control flow, arithmetic, IO, concurrency,
  introspection — all sends. there is no second operation.
- **references are capabilities.** if you hold a reference to the
  Console, you can print. if you don't, you can't. there is no
  ambient global namespace from which permissions leak. spawning
  a vat means deciding exactly what it can do by deciding what
  references to hand it.

these commitments together define the substrate. everything else —
syntax, stdlib, canvas, federation — is the surface we build on top.

---

## 3. the continuous ladder

the design tension all authoring-oriented systems have to navigate is
**low floor vs. high ceiling**.

- *low floor* means "a beginner can do something meaningful in
  under an hour."
- *high ceiling* means "an expert can build arbitrary new
  abstractions without leaving the system."

smalltalk had the highest ceiling but a very high floor. hypercard
had the lowest floor but a low ceiling. emacs reached both at
different times but forced you to cross a steep learning chasm
between them.

moof's design principle, borrowed directly from bill atkinson and
doug engelbart: **the ladder should be continuous.** no mode
boundary between "i'm using this" and "i'm extending it." every rung
of the ladder offers a gesture more powerful than the one below, but
uses the same primitives.

a journey i'd like to make possible:

1. someone shares a workspace URL with you. you click it. you see
   a document — text, code, images, tables.
2. you notice a code block has an interesting result. you click it.
   you see its source.
3. you modify the source. it re-runs. the result updates.
4. the block calls a function you didn't write. you navigate to that
   function. same interface, same gestures.
5. the function is a method on a prototype. you navigate to the
   prototype. you see all its handlers.
6. you realize you can add a new handler. you click "add". you
   write moof. the new handler is live in your image.
7. you share your forked workspace back.

at no point does the user change mode. no "export to code". no
"publish". no "switch to developer view". the ladder is the same
material all the way down.

this is the north star. every design decision is tested against it.
"does this preserve the ladder?" is the question. "yes, but only if
you enable developer mode" is the answer we refuse.

---

## 4. authoring is the UI

the principle that follows:

> **authoring is the UI. and the UI is the substrate. and the
> substrate is the document.**

design commitments implied:

### no privileged layer

the inspector, debugger, repl, canvas, notebook — every tool moof
uses to look at itself is a moof object. it can be inspected. it can
be modified. it can be inherited from. you can build your own
inspector by inheriting from the default one and overriding a
handler. the rust layer beneath moof is a substrate that gets out of
the way as soon as the image boots. we consider it a smell when
something *has* to be rust.

### every value has a surface; surfaces are messages

a value doesn't have "a UI" attached. it has protocol conformances.
`view-as-text`, `view-as-card`, `view-as-grid`, `view-as-graph`. the
same object renders differently in different contexts because
different view-protocols are called on it. a view is a message, not
a widget.

making something look a certain way is a declarative gesture the
user performs: "conform this to that protocol." the inspector is just
something that calls `view-as-card` on whatever it holds. anyone can
write a new view protocol. anyone can conform their types.

### the script is next to the affordance

hypercard's deepest insight. when you see a rendered value, you are
one gesture away from its source. not "open in editor." not "view
source." the source *is* the handler. the gesture is "show me the
handler that produced this view."

### conformance as an authoring gesture

today `(conform X SomeProtocol)` is a programmer gesture. we want it
to be a *user* gesture too. the inspector has a "conform" button that
opens a protocol picker. clicking installs the required handlers
(with stubs) and opens the editor on them. this matches hypercard's
"new button" — a one-click gesture that pre-populates the script
editor.

### halos

a halo is a polymorphic ring of verbs that appears when you
click-and-hold on any object. each object provides its own verb set
via a `halo-verbs` handler. halos are the universal interface: a user
who has learned the halo has learned all of moof's UI in one
gesture.

---

## 5. the grimoire aesthetic

a word about feel. this matters more than it sounds.

smalltalk's ImageFileContent, hypercard's .stk files, emacs's .emacs
init — these were **personal grimoires**. you accreted them over
years. they were handwritten in the sense that mattered. tools you
built last winter still worked this winter, and probably will next
winter too.

modern software treats this as a bug to be fixed by an update cycle.
moof treats it as a first-order feature.

- **your image is yours.** your handlers work forever. your
  protocols don't get deprecated by a vendor. schema migrations
  happen through migrator objects *you* write (or accept
  explicitly from upstream).
- **stability is a feature.** when moof itself upgrades, migrators
  handle it, and if a migrator isn't possible, the image tells you
  honestly — not "you must upgrade now or lose everything."
- **accretive, not replaceable.** "delete everything and start
  over" is hard to find for a reason. the default workflow is
  layering, not restart.
- **low-contrast, typographic, quiet.** the UI fades; the content
  is foreground. no splash screens. no onboarding tours. boot in
  under a second and land where you left off.

this is the aesthetic of the grimoire you inherited from your
teacher and are adding chapters to. it's the opposite of most
modern software. it's closer to plan9, to emacs, to well-worn unix
tools. it's what makes moof part of *you*, not part of a vendor's
roadmap.

---

## 6. what moof explicitly rejects

the commitments above imply rejections. we call these out so they
aren't accidentally re-litigated:

- **the app/document divide.** there is no "app." there are
  workspaces. there is no "document." everything is an object.
  we will not build a moof where you import/export between a
  "notebook format" and "the real data."
- **the developer/user divide.** no developer tools separate from
  user tools. no "moof pro" vs "moof lite." the full primitives
  are always available.
- **JSON walls.** moof values don't serialize to JSON for anything
  internal. JSON is a capability-mediated interop format for
  non-moof systems. inside moof, values move as values.
- **hidden state.** no "this feature only works if you configure
  X." state is objects; objects are visible; visibility is
  discoverable through the inspector.
- **mandatory network.** moof runs locally first. every feature
  must work offline. federation is an addition, not a requirement.
- **accounts.** moof doesn't have a user account system. your
  image is your identity. sharing uses URIs + optional
  signatures, not a platform account.
- **privileged tools.** the inspector is not "the debugging UI."
  it's *a* UI, made of the same parts anyone can use.
- **lock-in.** images are portable. content is content-addressed
  so it can be exported wholesale. moof's value is what it lets
  you do, not what it keeps you from leaving with.

---

## 7. federation as the social fabric

engelbart's "bootstrap" was inherently collaborative — his team used
NLS to build NLS. hypercard's distribution was social — stacks
traveled on floppies, got remixed, reappeared in magazines.
smalltalk's culture was deeply sharing even when the tools were
single-user.

moof's federation story makes this structural:

- **content-addressed values dedupe automatically across
  machines.** your shared list and my shared list are stored once,
  globally, regardless of how many people hold it.
- **sharing is URI-exchange.** "here's my workspace" is a URI. you
  paste it, your client resolves what it needs over the network,
  the rest is local.
- **subscription is a protocol.** you can subscribe to someone
  else's objectspace; their changes propagate (opt-in). think RSS
  for objects, or ActivityPub for objectspaces.
- **conflict is a conversation.** when two people change the same
  defserver, reconciliation is a message the users see. no silent
  last-write-wins. no CRDT-for-everything — just for the things
  where it's semantically ok.

the vat model already gives us most of this infrastructure: vats are
isolated, cross-vat sends are the only interaction, FarRef is
location-transparent. federation extends FarRef across machines,
adds content-addressed caching, adds subscription. it's a delta, not
a rewrite.

this is what makes moof a *medium* rather than a single-user
environment.

---

## 8. the test

every design decision is tested against five questions:

1. **does this preserve the continuous ladder?** could a user climb
   this rung from the one below without a mode shift?
2. **does this fit the image?** can it live as a persistent moof
   object, or does it introduce a parallel state system?
3. **does this respect addressability?** can i send someone a URI
   for this, and would it make sense on their machine?
4. **does this age well?** will this feature still be here in five
   years, on the user's terms?
5. **is this authoring-for-all?** can a non-programmer engage with
   this, at some level of depth, without learning to program?

sometimes the answer is "not yet" — we're building a substrate, not
a finished product. but the trajectory has to preserve these
properties. we don't cash in the long-term design for a short-term
win.

---

## 9. where this is going

the roadmap, in broad strokes:

- **now: the substrate.** kernel VM, object model, vats, image,
  capabilities. most of this exists. gaps being closed in waves 9–10.
- **next: the canvas.** a zoomable, inspectable UI where every
  object renders itself and the halo exposes authoring gestures.
  morphic, re-read through moof's commitments.
- **next: the agent.** an LLM in a vat with membraned capabilities.
  the agent collaborates on the substrate with you; doesn't replace
  your judgment.
- **next: federation.** FarRef extended across machines.
  content-addressed cache for cheap sharing. subscription protocol.
  the web of objectspaces.
- **eventually: the whole medium.** authoring-for-all. workspaces
  shared as URIs. users who don't code still authoring their own
  tools through conformance and halo gestures. the dynabook as
  promised.

none of this is fast. the timeline is measured in years, not
sprints. the point is the trajectory, not the milestones.

---

## 10. in one sentence

> **moof is a personal dynamic medium — a living, accretive,
> shareable objectspace where the tools and the content are made of
> the same material, and everyone is an author.**

that's the north star. foundations, effects, protocols, canvas,
federation — every wave gets us closer to that, or it was the wrong
wave.
