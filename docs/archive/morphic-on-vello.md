# morphic-on-vello

> design for moof's eventual canonical authoring surface:
> a morphic-family direct-manipulation environment, rebuilt
> on linebender's vello, with every morph as a moof object.

**status**: parked design, not yet implementation. prerequisite
waves live in `foundations.md`: content-addressing, log +
snapshot, URIs, history, source preservation, basic method
browsing. morphic-on-vello starts only after those land.

this doc exists so that when we're ready, we know what we're
building. no code yet; shape, protocol, and open questions.

---

## why morphic, why vello, why rebuild

**why morphic.** it's the only mature UI paradigm that matches
moof's authoring philosophy. every visible thing is an object.
the object renders itself. events route to the object
underneath the cursor. there's no separate widget framework
mediating — the morph *is* the widget. smalltalk's squeak,
self's ui2, lively kernel, pharo morphic are all
thirty-year-tested proof that this model works for
authoring-for-all.

**why vello.** it's a rust-native, GPU-accelerated 2D scene
renderer built by the linebender group (raph levien et al),
explicitly designed with "UIs as data" in mind. it handles
paths, gradients, clipping, layers, and ships parley for proper
text shaping (variable fonts, ligatures, RTL). it's the
closest thing in any ecosystem to "squeak's balloon 2D
engine, if we were building it today."

**why rebuild rather than bind.** we considered embedding
lively.next in a webview and driving pharo as a subprocess
(see `ui-explorations.md`). both work. both introduce a second
objectspace running beside moof's, with its own GC, its own
persistence, its own identity system. maintaining the seam is
forever. in contrast, rebuilding morphic on vello gives us a
system where **every morph is a moof object** — inspectable,
conformable, persistable, federatable, all the way down. the
ladder stays continuous. that's worth the months of work.

one compromise: we *start* by reading the squeak blue book's
morphic chapter and porting its ideas literally. morphic's
*core* is small (about 30 pages of smalltalk for the essential
classes). we're not inventing morphic; we're porting it.

---

## the core protocol

a morph is anything that conforms to `Morph`. the protocol is
intentionally small:

```moof
(defprotocol Morph
  "a visible, interactive, addressable object."
  (require (extent)
    "{width height} as integers. determines layout and hit-test box.")
  (require (draw: ctx)
    "emit vello primitives into the rendering context.")
  (provide (submorphs) nil
    "child morphs contained in this one. defaults to none.")
  (provide (handle: event) nil
    "respond to an input event. return an Act or nil."))
```

that's it. everything else is built up.

extensions, each their own protocol so a morph opts in:

```moof
(defprotocol Layouter
  "a morph that arranges its submorphs."
  (require (layoutIn: rect)
    "compute submorph positions + extents given my bounding box."))

(defprotocol Haloable
  "a morph that offers a set of action buttons when selected."
  (require (haloVerbs)
    "list of {label action} records. actions are closures."))

(defprotocol Steppable
  "a morph that animates. the world ticks it every frame."
  (require (stepAt: ticks)
    "advance internal state by ticks milliseconds."))

(defprotocol Draggable
  "a morph that can be picked up."
  (provide (pickedUpBy: hand) self)
  (provide (droppedOn: target) self))
```

no morph is forced to implement any of these. a pure display
morph implements `Morph` and nothing else. interactive morphs
conform to the protocols they need. this is the continuous
ladder: **a simple shape is simple; an interactive widget pays
for what it uses.**

---

## the world

the world is a distinguished morph that contains all other
morphs. it owns:

- the submorph list (top-level morphs on the screen)
- the hand (mouse state, current drag payload)
- the step list (morphs conforming to `Steppable`)
- the active halo (if any)

the world is a moof defserver — mutable state, captured in
updates, accessed via FarRef. every move/resize/pickup is an
Act that mutates the world through a delta.

```moof
(defserver World ()
  "the root morph. contains all top-level morphs,
   the hand, and the step list."

  { root-morphs: nil
    hand: (Hand)
    halos: nil
    step-list: nil

    [addMorph: m]  ...
    [step: ticks]  ...
    [dispatch: event] ...
    [draw: ctx]    ...
  })
```

the capability layer (see below) drives the world by sending
`[world step: dt]` every frame and `[world dispatch: evt]` on
input.

---

## events

events arrive from the capability as simple tagged records:

```moof
{ kind: 'mouse-down  pos: {x: 100 y: 200}  button: 0 }
{ kind: 'mouse-move  pos: {x: 101 y: 200}  delta: {x: 1 y: 0} }
{ kind: 'mouse-up    pos: {x: 102 y: 205}  button: 0 }
{ kind: 'key-down    key: 'a  mods: {shift: true} }
{ kind: 'scroll      pos: {x: 100 y: 200}  delta: {x: 0 y: -40} }
```

the world's `dispatch:` handler:

1. finds the deepest morph containing `pos` (`pickMorphAt:`)
2. forwards the event to that morph via `handle:`
3. if the morph returns an Act, schedules it
4. if no morph handles it, the world handles it (start drag,
   open halo on right-click, etc.)

bubbling: if a morph's `handle:` returns `'pass`, the event
bubbles up to the owner. this gives us the classic "click an
empty area of a card to reach the card itself" behavior.

---

## layout

layout is a protocol, not a property. a container morph
conforms to `Layouter` and decides where its submorphs go. we
ship a small set of standard layouters:

```moof
(def NoLayout      { Morph  [layoutIn: rect] self })
(def TableLayout   { Morph  ... })    ; row/column with gaps
(def ProportionalLayout { Morph ... }) ; resizes by ratios
(def FlowLayout    { Morph  ... })    ; text-like wrapping
```

`TableLayout` is morphic's workhorse — it handles 90% of UI
layout needs (windows, toolbars, lists). `ProportionalLayout`
is for resizable panes. `FlowLayout` is for textual rendering
of morphs.

users can conform their own layouters. "spiral layout,"
"physics-based layout," "graph layout" are all handler-swaps
away. this is one of morphic's best gestures.

---

## halos

halos are smalltalk's killer interaction primitive. you click
and hold (or right-click) on any morph; a ring of verb buttons
appears around it. each button is a verb: inspect, grab,
duplicate, delete, resize, open-script.

in moof-morphic, halos are a protocol. any morph that conforms
to `Haloable` returns its verb list. base moof types get a
standard halo (via a default conformance). user morphs can
extend or replace.

```moof
(defmethod SomeMorph haloVerbs ()
  (list
    { label: "i" action: || [Inspector on: self] }
    { label: "x" action: || [world removeMorph: self] }
    { label: "⌷" action: || [Duplicator duplicate: self] }
    { label: "↕" action: || [Resizer on: self] }
    { label: "✎" action: || [SourceEditor editHandler: 'draw: on: self] }))
```

halos are themselves morphs. they're drawn in the world's halo
layer, above normal content. they disappear when you click
elsewhere.

**the core move:** halos are the universal interface. a user
who has learned halos has learned how to do everything —
inspect, modify, duplicate, script, resize, move — on *any*
object. no per-widget menus to remember.

---

## reference morphs (v0 shipping list)

**shape morphs** — basic visible primitives:

- `RectangleMorph` — filled rectangle with optional border
- `EllipseMorph` — filled ellipse
- `PathMorph` — arbitrary vello path
- `ImageMorph` — raster image (PNG, JPG via a decoder capability)
- `TextMorph` — a styled run of text via parley

**container morphs** — hold other morphs:

- `StackMorph` — submorphs overlaid
- `RowMorph` — horizontal TableLayout
- `ColumnMorph` — vertical TableLayout
- `PaneMorph` — resizable ProportionalLayout
- `ScrollMorph` — scrollable clipping viewport
- `WindowMorph` — title bar + close button + draggable + contained content

**control morphs** — interactive primitives:

- `ButtonMorph` — tap to fire action
- `TextFieldMorph` — editable text
- `CheckBoxMorph` — bool toggle
- `SliderMorph` — numeric scrub
- `MenuMorph` — pop-up list of items

**authoring morphs** — the tools that make moof smalltalk-grade:

- `InspectorMorph` — shows slots + handlers of any value,
  slot click to drill in, handler click to edit source
- `BrowserMorph` — prototype browser (analogous to smalltalk's
  class browser). left pane: list of protos. right pane: list
  of handlers. bottom: source for selected handler.
- `TranscriptMorph` — scrolling log output. `[Transcript show: x]`
  appends. this replaces rust-side println for development.
- `WorkspaceMorph` — scratchpad. paste code, select, cmd-D to
  evaluate. output appears inline. this is our "notebook."
- `HaloMorph` — the ring of verbs drawn around a selected morph.
- `DebuggerMorph` — activates on an Act failure. shows the
  stack, lets you inspect frames, edit the failing method,
  resume.
- `ClockMorph` — time display. not critical but traditional.

v0 is "enough to inspect and edit moof objects." shapes +
containers + buttons + text + inspector + browser + workspace.
~70% of what morphic users actually touched.

---

## the capability shape

a single capability owns the render window + input. it's the
only thing rust-side; everything above it is moof.

```moof
; in the capability's proto:

[openWindow: title size: {w h}]       → Act<WindowRef>
[render: scene on: windowRef]         → Act<nil>
[inputEvents: windowRef]              → Act<list of events>
[closeWindow: windowRef]              → Act<nil>
```

rust-side crate: `crates/moof-cap-canvas/`

- runs one or more vello-backed windows via winit
- maintains a pool of open windows
- per frame: asks the moof side for a `scene` (an opaque value
  representing a set of vello draw calls), renders it, returns
  input events
- the scene is constructed moof-side by calling `draw:` on the
  world morph, which recursively emits vello primitives into a
  scene-builder foreign type

**the draw: loop (conceptually):**

```
loop:
  evs = [canvas inputEvents: window]
  [world dispatch: evs]
  [world step: dt]
  scene = [SceneBuilder new]
  [world draw: scene]
  [canvas render: scene on: window]
```

this runs as moof code inside a dedicated canvas vat. at 60fps
that's 60 `step:` and `draw:` calls per second, driving maybe a
few hundred morph `draw:` invocations — well within moof's
performance envelope given bytecode dispatch + vello's GPU
acceleration.

the scene-builder is a foreign type. its handlers (`rect:`,
`ellipse:`, `path:`, `text:`, `clip:`, `pushLayer:`) emit into
a vello `Scene` that the capability flushes. moof morphs call
these from their `draw:` methods.

---

## purity + morphic

traditional morphic mutates morph state freely. `morph
position: 10@20` is destructive. moof can't do that.

the fix, staying inside moof's immutability rules:

- each morph is a **defserver**. its state (position, extent,
  color, submorphs) lives as slots; mutations are `update`
  deltas.
- `[morph setPosition: p]` returns `Act<nil>` that applies a
  `{position: p}` delta.
- animation (`step:`) returns an Act that applies the frame's
  delta.
- dragging: `mouse-move` event → compute new position → `Act`
  updating position → scheduler applies delta before next frame.

this is *cleaner* than original morphic — every visual change
is an audited, replayable, undoable event. the scene-builder
captures deltas implicitly; "undo last drag" becomes a real
gesture for free.

performance concern: sixty updates per second per dragged
morph is fine (thousands of Acts). if we ever have thousands
of morphs animating simultaneously, we revisit — maybe
a "direct-mutation mode" for the world's inner loop where
step:/dispatch: get to bypass the Update machinery for
hot-path morphs. don't optimize yet.

---

## content-addressed morphs, federated worlds

because morphs are moof values (mostly immutable shape + a
mutable defserver cell for state), they round-trip through the
image, content-address, federate cleanly.

this means:

- **share a morph by URI.** "here's my inspector, try it" is a
  hash.
- **save a scene as a value.** your workspace layout is a
  value in your image, loadable years later.
- **remote morphs.** a morph whose state lives in someone
  else's vat can still be drawn locally — the `draw:` method
  returns an Act; the local world awaits it and renders when
  resolved. collaborative editing falls out.

this is the kay "objects are biological cells" and engelbart
"we bootstrap together" idea made concrete. we're not adding
collaboration on top of a local UI; the UI is *already*
location-transparent because moof's object model already is.

---

## authoring as direct manipulation

the experience we're designing for:

1. you have a workspace open. you see some code, a result.
2. you click the result. a halo appears. you click "i"
   (inspect). an InspectorMorph opens, showing the value's
   slots and handlers.
3. you click a handler. its source text appears below.
4. you edit it. you hit cmd-D. the method recompiles live.
   future sends of that selector use the new source.
5. you click the inspector's window title halo, click "x" to
   close it.
6. you drag a value from one workspace into another. it's the
   same value, referenced from two places.
7. you conform a plain object to `Morph` by clicking "conform"
   in the halo and picking `Morph`. the inspector helps you
   fill in the required handlers with stubs.
8. you drag your new morph into the world. it's drawn.

at no point is there a mode shift between "i'm using moof" and
"i'm programming moof." this is the target UX. it's the
hypercard authoring gesture combined with smalltalk's live
method editing.

---

## open questions

things i don't know the right answer to yet. parked for
discussion when we get closer.

- **typography stack.** parley is the right text shaper. but do
  we ship our own font bundle or use system fonts? grimoire
  aesthetic wants a chosen palette; platform integration wants
  system fonts. probably: ship a small set of carefully-chosen
  fonts as content-addressed values in the image, users can
  override.

- **ime input.** how do we handle CJK/IME input for
  TextFieldMorph? winit provides the events; we need to pipe
  them into text morphs correctly. needs an expert's attention.

- **accessibility.** screen readers, high contrast, keyboard-
  only navigation. morphic famously got this wrong. we should
  design for it from the start — every morph has an
  `a11y-label` handler, the accessibility tree is derived from
  morph hierarchy. details TBD.

- **multi-window.** one window = one world? or one world with
  multiple window morphs? multi-window is simpler for OS
  integration; single-world is cleaner conceptually. leaning
  single-world with window morphs as a special class that the
  capability maps to actual OS windows.

- **retained vs immediate.** we've committed to retained (every
  morph is identity-preserving). but the draw: loop emits a
  fresh vello scene every frame — that's immediate at the
  render-command level, retained at the morph level. this is
  correct but worth spelling out: moof is retained; vello is
  immediate; the morph hierarchy is the bridge.

- **animation model.** smalltalk morphic animates via `step:` on
  Steppable. is that enough? or do we want explicit timeline
  animations (à la Core Animation, CSS transitions)? probably
  both — `Steppable` for imperative, a declarative
  `AnimationMorph` for "animate this property from a to b over
  n ms with this curve."

- **performance envelope.** moof's current bytecode VM dispatch
  rate is plenty for dozens of morphs at 60fps. thousands of
  morphs might stress us. mitigations: frame-level partial
  update (redraw only dirty regions), a "static morph"
  optimization that caches vello scene fragments, eventually a
  C-side hot path for rectangle/text morphs. don't do any of
  this until we see the profile.

- **input routing for nested drag.** when morph A is being
  dragged inside morph B which is being dragged inside the
  world, which morph gets the mouse-move? morphic's answer:
  whoever the hand is attached to. moof follows that.

- **interoperation with the existing workspace/CodeBlock
  types.** the workspace types already in `lib/tools/workspace.moof`
  are value-shaped (blocks are immutable). do we turn them into
  morphs (make them visualizable) or do morphs reference them
  (workspace is data; morphic is view)? probably the latter —
  data and view stay distinct, but `WorkspaceMorph` knows how
  to render a workspace value. model-view discipline.

- **script editor integration.** when you click "edit handler"
  on a morph, what text editor comes up? a simple TextFieldMorph
  works for v0. a proper code editor (syntax-aware, with
  completion) is its own subproject. probably a later morph
  called `CodeEditorMorph` that embeds tree-sitter.

---

## sequencing

rough order of operations when we start:

1. **phase 0: vello hello-world capability.** tiny
   `crates/moof-sketch/` that opens a window and moof code can
   put a single rectangle on it. proves the vello binding, the
   capability shape, the frame loop. ~1 week.
2. **phase 1: core protocol + world + hand + basic events.**
   `Morph` protocol defined in moof. World and Hand as
   defservers. One hardcoded `RectangleMorph`. Mouse events
   route to it. ~2–3 weeks.
3. **phase 2: reference morph kit.** shape morphs + container
   morphs + layout protocols + button + textfield. ~1 month.
4. **phase 3: inspector morph.** the first authoring morph.
   changes everything — once we can inspect objects from a
   moof UI, we stop using the rust-side repl for the hard
   cases. ~1 month.
5. **phase 4: browser + transcript + workspace morphs.** now
   moof has its own IDE, written in moof. ~1 month.
6. **phase 5: halos, drag-drop, duplicate, conform-from-ui.**
   the authoring gestures that make morphic *feel* like
   morphic. ~3 weeks.
7. **phase 6: polish.** typography, theming, animation,
   accessibility, performance. ~2 months minimum, open-ended.

total: ~6 months to a living morphic environment from a cold
start. but we don't start cold — we have bytecode, persistence
(eventually), a working protocol system. realistically: **~4
months of focused work after foundations land, for a v0 that's
genuinely usable**, then open-ended improvement as the canonical
authoring surface.

---

## what this gets us

a moof environment where:

- every value has a visible, interactive surface
- the tools are made of the same stuff as the things
- new morphs are authored *from inside* using the same
  primitives that authored the existing ones
- sharing a workspace means sending a hash; your friend opens
  it and sees exactly what you see
- scrubbing time is a gesture, not a debugger feature
- a non-programmer can click-inspect-modify without ever
  writing moof syntax, and a programmer can drop into source
  editing without leaving the environment
- the ladder is continuous from "i opened a workspace" to "i
  wrote a new protocol"

this is the authoring-for-all vision made concrete. it's what
smalltalk *almost* was, what hypercard *almost* was, what
engelbart *partially* achieved. no one has done it all together
in a way that's modern, federated, and free. that's the slot
moof is aiming for.

*the work is months away. but the shape is clear. we will know
it's time to build this when foundations are boring-reliable
and we reach for the morphic-shaped hole in our daily use of
moof and find that we keep reaching.*
