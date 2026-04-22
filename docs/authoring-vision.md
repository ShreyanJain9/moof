# authoring-vision

> the long view. moof in the tradition of kay, engelbart,
> atkinson — a personal dynamic medium where the user is the
> author, the tools are the things, and there is no priesthood.

VISION.md covers the runtime-shape of moof. this doc is about
the *why* — what kind of tool we're trying to build, who it's
for, and the specific design commitments that follow from
taking those ancestors seriously.

---

## three ancestors, one insight

**Alan Kay / Smalltalk / Dynabook.** the big idea wasn't
objects. it was that the system itself was made of objects you
could inspect and modify *while it ran*. the class browser was
written in smalltalk; you opened it up and saw smalltalk. no
privileged layer. the commitment was late-binding all the way
down: behavior was decided at message-send time, not compile
time, so anything could be extended by anyone at any moment.
this is what made the dynabook *dynamic*.

**Doug Engelbart / NLS / "the mother of all demos" (1968).**
the big idea wasn't the mouse. it was *augmentation*: tools for
amplifying human intellect, with special focus on structured
collaboration — multiple people editing the same document live,
hypertext linking thoughts across documents, view-control so the
same data could be seen at different granularities. engelbart
said the goal was "to bootstrap": the tools would be used to
build better tools, recursively, by the same people using them.

**Bill Atkinson / HyperCard.** the big idea wasn't stacks of
cards. it was that **every affordance carried its script next to
it**. click a button; hold a modifier; see its script; edit it.
the same gesture worked on the most trivial "go to next card"
button and on a card that ran a whole adventure game. authoring
and using were the *same* mode of interaction, distinguished
only by how deep you chose to go. tens of thousands of
non-programmers wrote hypercard stacks. some of them made art.
some of them made businesses.

what all three shared — and what almost no system today
inherits — is that **the substrate, the tool, and the document
are the same material accessible at the same level.** you don't
download an app to edit your document; the document *is* an
authoring environment. you don't learn a separate IDE to extend
the system; the system *is* its IDE.

moof's bet is that this is still the right direction, and that
the reason modern systems don't work this way is accident, not
necessity.

---

## the continuous ladder

the design tension all three systems had to navigate was **low
floor vs. high ceiling**. let low-floor mean "a beginner can do
something meaningful in under an hour." let high-ceiling mean
"an expert can build arbitrary new abstractions without leaving
the system." nobody solved both:

- smalltalk had the highest ceiling. the floor was in the
  clouds — everything required mental commitment.
- hypercard had the lowest floor. the ceiling was also low —
  hypertalk topped out fast.
- NLS had a high ceiling but required weeks of training before
  you could do anything at all.

the goal is a **continuous ladder**: no mode boundary between
"i'm using this" and "i'm extending it." every rung of the
ladder offers a gesture that's more powerful than the one below
but uses the same primitives.

a user journey i'd like to be possible:

1. someone shares a workspace with you. you open it and read it
   like a document.
2. you notice a code block has an interesting result. you click
   it. you see its source.
3. you modify the source. it re-runs. the result updates.
4. you realize the block calls a function you didn't write. you
   navigate to that function. same interface as navigating to a
   different block.
5. you realize the function's a method on a prototype. you
   navigate to the prototype. you see all its handlers.
6. you realize you can add a new handler. you click "add". you
   write moof code. the new handler is live in your image.
7. you share your forked workspace back.

at no point does the user switch contexts. no "developer mode."
no "export to code." no "publish." the ladder is the same
material all the way down.

---

## authoring is the UI

the single principle that follows:

> **authoring is the UI. and the UI is the substrate. and the
> substrate is the document.**

design commitments implied:

### no privileged layer

the inspector, debugger, canvas, notebook, repl, and eventually
the ui framework all live in the image as moof objects. they
can be inspected. they can be modified. they can be inherited,
conformed, extended. the user can build their own inspector by
inheriting from the default one and changing one handler. this
is the smalltalk / croquet / lively.next commitment.

concretely: every "shell feature" that lives in rust today
(`:notebook`, the repl loop, bindings in startup) is a future
thing to pull into moof-side code. the rust layer should
eventually be just enough to bootstrap the moof image and then
get out of the way.

### every value has a surface, and surfaces are messages

a value doesn't have "a UI" or "a widget" attached. it has a
protocol conformance — `view-as-text`, `view-as-card`,
`view-as-grid`, `view-as-graph`. the same object renders
differently in different contexts because different protocols
are called on it. a view is a message, not a widget.

this means:
- making an object look a certain way is done by conforming it
  to a view protocol — an authoring gesture the user performs
- an inspector *just* calls `view-as-card` on whatever it holds
- the user can write their own view protocol and conform their
  own types

in hypercard you drew a button by choosing the button tool. in
moof you draw a view by conforming your object. the gesture is
declarative, not procedural. the medium is modal at no layer
but the user's — the system itself is pluri-modal.

### conformance as an authoring gesture

today `(conform X SomeProtocol)` is a programmer gesture. i want
it to also be a user gesture. the inspector has a "conform"
button that opens a picker of known protocols. clicking installs
the required handlers (with stubs the user can fill in). this
matches hypercard's "new button" as a one-click gesture that
opens the script editor pre-populated.

### the script is next to the affordance

hypercard's deepest insight. when you see a rendered value, you
should be one gesture away from its source. not "open in
editor." not "view source." the source is a handler on a
prototype; the gesture is "show me the handler that produced
this view."

### halos

smalltalk/morphic's halo: you click-and-hold on any pixel and
get a ring of verbs — inspect, edit, duplicate, grab, resize,
open-handler-menu. the halo is polymorphic; each object provides
its own verb set. on moof, a halo is just a `halo-verbs`
protocol returning a list of actions.

halos are the universal interface. a user who has learned the
halo has learned all of moof's UI in one gesture.

---

## time as view-control

engelbart's deepest idea was that you're never looking at the
document, you're looking at a *view of* the document. in moof,
the image is the document and every inspector shows a view of
it. "the view you're currently in" includes:

- what slice of objects are visible
- what protocol is being used to render them
- what moment in time you're viewing

the last one matters: **time is a view axis**. the user can
scrub back to last tuesday's image state the same way they can
filter to "only show objects in this workspace." time-travel
isn't a debugger feature; it's a built-in navigation gesture.

this requires the persistence layer to record enough to
reconstruct past states cheaply (see foundations.md), and the UI
layer to expose it as a timeline affordance. ideally: a little
handle at the edge of every view, draggable backward to see
this thing as it was.

---

## federation as the social fabric

engelbart's "bootstrap" was inherently collaborative — his team
used NLS to build NLS. hypercard's distribution was social —
stacks traveled on floppies, got remixed, reappeared in
magazines. smalltalk's images were individual but the culture
around them was deeply sharing.

moof's federation story should make this structural:

- **content-addressed values dedupe automatically across
  machines.** your shared list and my shared list are stored
  once, globally, regardless of how many people hold it.
- **sharing is URI-exchange.** "here's my workspace" is a URI.
  you paste it, your client resolves what it needs over the
  network, the rest is local.
- **you can subscribe to someone else's objectspace.** their
  changes propagate to you (opt-in). think rss for objects, or
  the activitypub model for objectspaces.
- **conflict isn't a database problem, it's a conversation.**
  when two people change the same defserver, the reconciliation
  is a message the users see. no silent last-write-wins. no
  CRDT for everything — just for the things where it's
  semantically ok.

the vat model already gives us most of this: vats are isolated,
cross-vat sends are the only interaction, FarRef is location-
transparent. we extend FarRef to cross machines. we add the
content-addressed cache. we add subscription as a protocol.

this is *bigger* than "moof has a network feature." it's what
makes moof part of a *medium* rather than a single-user
environment.

---

## the grimoire aesthetic

a word about feel.

smalltalk's ImageFileContent, HyperCard's .stk files, emacs's
.el init — these were *personal grimoires*. you accreted them
over years. they were handwritten in the sense that mattered.
tools you built last winter still worked this winter, and
probably will next winter too.

modern software eats its own tail: your 2018 workflow broke in
2020, your 2020 workflow broke in 2022. the SaaS death march.
the platform-update treadmill. this isn't a law of nature; it's
a cultural choice made by an industry optimized for churn.

moof's persistent image is a commitment in the other direction.
your image is *yours*. your handlers work forever. your
protocols don't get deprecated by a vendor. when moof itself
upgrades, migrators handle it, and if a migrator isn't possible,
the image tells you honestly — not "you must upgrade now or lose
everything." stability is a feature.

the aesthetic that follows:

- **low-contrast, typographic, quiet.** no chrome. no flashy
  animations. the interface fades; the content is foreground.
- **hand-made, not templated.** a workspace should feel like
  your room, not like a template.
- **accretive, not replaceable.** the "delete everything and
  start over" button is hard to find for a reason.
- **quiet bootstrapping.** moof boots in under a second. loads
  your image. shows you where you left off. no splash, no
  onboarding, no tour. it trusts you to know where you are.

this is the opposite of most modern software. it's much closer
to plan9, to emacs, to well-worn unix tools. it's the aesthetic
of the grimoire you inherited from your teacher and are adding
chapters to.

---

## anti-patterns we explicitly reject

- **the app/document divide.** there is no "app"; there are
  workspaces. there is no "document"; everything is an object.
  we never build a moof where the user imports/exports between
  a "notebook format" and "the real data."

- **the developer/user divide.** no "developer tools" separate
  from "user tools." no "moof pro" vs "moof lite." the full
  primitives are always available. the ladder is continuous.

- **JSON walls.** moof values don't serialize to JSON for
  anything internal. JSON is a capability-mediated interop
  format when talking to non-moof systems. inside moof, values
  move as values.

- **hidden state.** no "this feature only works if you configure
  X." no global settings dictionaries. state is objects;
  objects are visible; visibility is discoverable through the
  inspector.

- **mandatory network.** moof runs locally first. every feature
  must work offline. federation is an addition, not a
  requirement.

- **accounts.** moof doesn't have a user account system. your
  image is your identity. sharing uses URIs + optionally
  signatures, not a platform account.

- **privileged tools.** the inspector is not "the debugging UI"
  — it's *a* UI, made of the same parts anyone can use. the
  repl is *a* surface, not *the* surface. anyone can build a
  different one.

- **lock-in.** images are portable. content is
  content-addressed so it can be exported wholesale. moof's
  value is what it lets you do, not what it keeps you from
  leaving with.

---

## what this means practically

every design decision we make — syntax, ui, capability model,
effect system — should be tested against:

1. **does this preserve the continuous ladder?** could a user
   climb this rung from the one below without a mode shift?
2. **does this fit the image?** can this live as a persistent
   moof object, or does it introduce a parallel state system?
3. **does this respect addressability?** can i send someone a
   URI for this, and would it make sense on their machine?
4. **does this age well?** will this feature still be here in
   five years, on the user's terms?
5. **is this authoring-for-all?** can a non-programmer engage
   with this, at some level of depth, without learning to
   program?

sometimes the answer is "not yet" — we're building a substrate,
not a finished product. but the trajectory has to preserve
these properties. we don't get to cash in the long-term design
for a short-term win.

---

## what we're building, in one sentence

> **moof is a personal dynamic medium — a living, accretive,
> shareable objectspace where the tools and the content are made
> of the same material, and everyone is an author.**

that's the north star. foundations, effect system, protocols,
canvas, federation — every wave either gets us closer to that
or it was the wrong wave.
