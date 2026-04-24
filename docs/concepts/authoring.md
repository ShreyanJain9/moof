# authoring

**type:** concept

> the image isn't a product you use; it's a workshop you live in.
> this doc is about what that means concretely — how the UI, the
> live editing, the inspector, and the canvas fit together.

---

## the continuous ladder

moof's UX commitment: **no mode boundary between "i'm using this"
and "i'm extending it."** every rung of the ladder uses the same
primitives as the rung below. the ladder is the interface.

the journey we intend:

1. **open something shared with you.** a workspace URL, dragged in
   or typed. it opens. you read it.
2. **see a result; click it to see its source.** the rendering is
   just a view; the source is a handler; the gesture to switch is
   uniform.
3. **edit the source; it re-runs.** live. no save-and-reload.
4. **navigate to a function it calls.** same gesture, same
   interface.
5. **add behavior.** click "conform"; pick a protocol; fill in
   stubs. new methods appear.
6. **share your fork.** a URL. paste it to a friend; they open
   yours on their moof.

at no point do you change mode. no "switch to developer view." no
"export to code." no "publish." the ladder is one material all the
way down.

---

## authoring is the UI

single principle:

> **authoring is the UI. and the UI is the substrate. and the
> substrate is the document.**

corollaries:

### no privileged layer

the inspector, debugger, repl, canvas, notebook — every tool moof
uses to look at itself is a moof object. it can be inspected. it
can be inherited from. you can build your own inspector by
inheriting from the default one and overriding a handler.

the rust layer underneath moof is substrate. it gets out of the
way as soon as the image boots. any "shell feature" that lives in
rust today is a debt to pay off — a move toward moof-side code.

### every value has a surface; surfaces are messages

a value doesn't have "a UI" attached. it has protocol
conformances:

- `[obj render: 'text]` → a String
- `[obj render: 'canvas]` → a drawing
- `[obj render: 'tree]` → a node graph
- `[obj render: 'card]` → a card widget
- `[obj render: 'inspector]` → a multi-aspect view

the same object renders differently in different contexts because
different view-protocols are called on it. a view is a message,
not a widget. the renderer is whoever chose the medium.

### the script is next to the affordance

hypercard's deepest insight. when you see a rendered value, you
are one gesture away from its source. not "open in editor." the
SOURCE is a handler on a prototype; the GESTURE is "show me the
handler that produced this view." halos expose it.

### conformance as an authoring gesture

today `(conform X SomeProtocol)` is a programmer gesture. we want
it to be a *user* gesture. the inspector has a "conform" button
that opens a protocol picker. clicking installs required handler
stubs and opens the editor on them.

this matches hypercard's "new button" — a one-click gesture that
pre-populates the script editor. the user sees an affordance, the
affordance has a stub, the user fills it in.

---

## halos

a **halo** is a polymorphic ring of verbs that appears when you
click-and-hold on any object. each object provides its own verb
set via a `halo-verbs` handler returning a list of actions.

```moof
(defmethod Integer halo-verbs ()
  (list 'inspect 'stash 'convert-to-float 'copy-value))

(defmethod Cons halo-verbs ()
  (list 'inspect 'reverse 'sort 'stash 'render-as-table))
```

the canvas widget draws the ring. user clicks a verb. verb runs.

halos are **the universal interface**. a user who has learned the
halo gesture has learned all of moof's UI in one gesture. every
object is reachable; every object exposes its verbs; verbs are
moof code.

---

## aspects

an **aspect** is one view of a value. one object can have many
aspects stacked visually:

- a recipe object as a rendered page
- drag the aspect handle; now it's a JSON-tree
- drag again; now it's the prototype chain
- drag again; now it's the raw slot table
- drag again; now it's the handlers list

aspects are handlers. `[obj aspect-as: 'json]` returns a view. the
canvas offers aspect switching as a UI affordance.

this is engelbart's view-control — the same data, different
perspectives — made first-class. the document is one object; the
perspective is a choice.

---

## the canvas

the canvas is the eventual home for moof's authoring experience.
it's:

- **zoomable infinite space.** objects placed on it persist in
  their spatial arrangement.
- **vector-first.** smooth LOD. no pixelation on zoom.
- **typographic.** low-chrome, quiet, text-forward. the UI fades;
  the content is foreground.
- **inspector native.** every object on the canvas exposes the
  halo. clicking through to source is the default gesture.
- **direct-manipulation-editable.** you edit handlers in place;
  they take effect live.

this does not yet exist. the REPL is the current surface. the
canvas lands after wave 10 (image-first boot) stabilizes.

see [../vision/horizons.md](../vision/horizons.md) for the
canvas horizon and its timing.

---

## the inspector

the inspector is a vat that holds read-only faceted references to
other vats. from it, you navigate:

- **slots** of the current object.
- **handlers** of the current object, each with its source.
- **prototypes** up the delegation chain.
- **references** from this object (via slots) and TO this object
  (reverse index — "what else knows this?").

every navigation is a new view. you can pin an object, split the
view, open a second inspector on a different object. the
inspector is a moof object — you can subclass it, change its
layout, add aspects.

current status: a simple terminal inspector exists. the canvas-
based inspector is horizon work.

---

## the repl

the REPL is a single interface over System. you type an
expression; it evaluates in your vat; the result renders via
`show`; the image saves on exit.

the REPL's key insight: **every eval produces a persistent
object.** `(def x 42)` binds x in your vat's env, and that env
persists. your session isn't text history; it's an object history.
you can `[inspector open: x]` and see 42 as a first-class value.

the REPL is not privileged. it's one Interface. future interfaces
(canvas, script, headless service) are different ways to reach
System. they all get the same capabilities subject to the same
grant matrix.

---

## time as view-control

engelbart's deepest idea: you're never looking at the document,
you're looking at a **view of** the document. in moof, the image
is the document, and every view includes:

- what slice of objects is visible
- what aspect is being used to render them
- **what moment in time you're viewing**

time is a view axis. you scrub back to last tuesday's state the
same way you filter to "only show objects in this workspace."
time-travel isn't a debugger feature; it's navigation.

this requires the persistence layer to retain past states cheaply
(via content-addressing + structural sharing — see
[persistence.md](persistence.md)) and the UI to expose time as a
draggable handle. ideally: every view has a timeline. default
position = now. drag left = see this thing as it was.

---

## federation as authoring

when you share a workspace URL with a friend, you're not exporting
data. you're inviting them into a view of your objectspace. their
client resolves the URL, fetches what it needs via content-
addressing, renders your workspace on their canvas.

if they edit, they fork — their changes live in their image. if
they want to propose a merge back, they send you a delta (a set
of Updates); you see them as pending proposals; you accept or
reject per Update.

this is authoring-for-all applied to collaboration. no "git pull";
there's one substrate and you're collaborating in it.

see [../vision/horizons.md](../vision/horizons.md) for the
federation horizon.

---

## what you need to know

- no mode boundary between use and extension.
- views are messages, not widgets; same object renders differently
  in different mediums.
- halos expose per-object verbs; learning the halo is learning
  moof's UI.
- aspects stack multiple views of one object.
- the canvas is the goal; the REPL is where we are.
- time is a view axis — you scrub past states as naturally as you
  filter slices.
- federation is authoring-across-peers, built on content-
  addressing and FarRefs.

---

## next

- [../vision/manifesto.md](../vision/manifesto.md) — why we
  commit to this.
- [../vision/horizons.md](../vision/horizons.md) — canvas,
  agent, federation — what's next.
- [protocols.md](protocols.md) — how a type says "i render as a
  card."
