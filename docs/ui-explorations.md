# ui-explorations

> opinionated survey of GUI substrates we could bind when the
> time comes. not urgent — persistence, purity, addressability,
> time-travel come first. but parked here so when we pick, we
> pick deliberately.

---

## the principle first

moof's UI has to honor the authoring commitments from
`authoring-vision.md`:

- **no privileged layer.** whatever substrate we bind, moof
  code has to be able to extend it, compose it, and in the
  limit replace it. if the UI framework assumes widget
  identities only it controls, it's the wrong framework.

- **object-identity-per-widget (retained mode).** immediate-mode
  frameworks (egui, imgui) rebuild the tree every frame; there's
  no "this widget is *that* moof object." retained-mode
  frameworks (xilem, iced, qt, gtk, flutter) keep a tree with
  stable nodes; we can bind each node to a moof object.

- **pixel-level escape hatch.** for a canvas / morphic surface,
  we need the ability to draw primitives directly — paths,
  glyphs, custom layouts. anything that forces us through a
  fixed widget taxonomy caps us at "another notebook app."

- **hot reload.** the image is persistent, the authoring is
  live. any substrate that requires a rebuild/restart to see
  changes breaks the authoring loop.

- **typographic honesty.** we are building a grimoire, not a
  dashboard. text rendering has to be beautiful — proper font
  shaping, variable fonts, ligatures, hinting. many UI libraries
  get this wrong in subtle ways. check first.

these principles rule out a lot — and that's useful. let's
walk through contenders.

---

## rust-native, short list

### xilem + vello (linebender)

my top pick.

- **xilem** is linebender's reactive retained-mode UI framework
  built on **masonry** (widget tree) and **vello** (GPU path +
  text renderer). everything is typed, composable, and tree-
  based. adding a new widget is writing a Rust type and impl.
- **vello** is the renderer *alone* — GPU-accelerated 2D paths,
  layers, clipping, text. it's what a smalltalk morphic core
  would use today if it were written from scratch.
- **parley** is their text shaper — proper typography including
  variable fonts.

why it fits: linebender's explicit design goal is "UIs as data"
— the widget tree is a value, reactive updates replace nodes,
there's no hidden scene graph managed imperatively. aesthetically
and architecturally it's the closest thing in rust to smalltalk's
morphic.

why to worry: it's pre-1.0. API churn likely. you're
inheriting a young ecosystem with fewer batteries than egui.

ideal use: moof binds masonry+vello as the UI capability. the
inspector, canvas, and notebook are all moof objects that
produce vello scenes. every "widget" is a moof object with a
`render-scene` handler.

### gpui (zed)

zed's in-house UI framework. recently more public-ish.

- GPU-rendered, retained, reactive
- optimized for rich text editor-style UIs
- produces genuinely fast, good-looking apps

why it fits: zed feels snappy. the framework behind it could
power a really good moof inspector.

why to worry: still co-evolving with zed. API shape changes.
community outside zed is small. the dependency surface is
large (zed pulls in livekit, webrtc, etc.).

ideal use: watch for maturity. revisit in a year.

### makepad

live-coding DSL + GPU renderer. very demo-friendly.

- hot reload is first-class
- visual expression is wild — explicitly pitched for creative /
  live coding work
- has its own language layered on rust

why it fits: makepad's ethos is close to moof's in spirit —
live, visual, creative.

why to worry: weird deps, smaller community. layering moof on
top of another DSL creates two living languages in one project.

### slint

commercial-ish declarative UI framework. QML-like DSL.

- retained-mode, tree of widgets
- cross-platform, including embedded
- polished defaults, good docs
- commercial license for closed-source (open source for
  open projects, but worth reading)

why it fits: stable, mature-ish for rust GUI. declarative shape
pairs well with moof's "ui as data."

why to worry: widget taxonomy is app-shaped, not morphic-shaped.
escape hatch to custom pixel-level drawing exists but feels
bolted-on.

### floem

reactive, uses stylo (servo's CSS engine) for layout.

- familiar CSS mental model
- fast, small
- used by lapce editor

why it fits: css gives you serious layout without us
reimplementing. reactive is good.

why to worry: css as an abstraction layer is a choice with
tradeoffs; you're inheriting web's layout quirks. is that the
aesthetic we want?

### iced

elm architecture, beautiful defaults.

- retained, typed, reactive
- good cross-platform support

why it fits: the elm architecture is well-understood. decent
starting point for app-shaped UIs.

why to worry: layout is rigid for morphic-style direct
manipulation. extending iced to be a canvas is fighting the
framework.

### blitz

servo-derived layout engine, *without* the browser.

- HTML/CSS layout primitives you can drive from rust
- no JS engine, no fetch, just the layout + render parts

why it fits: if we decide html semantics are useful, this gives
us browser-grade layout without the browser.

why to worry: you're still thinking in the web's DOM/CSS mental
model. that's a choice. maybe not the one we want.

---

## FFI options

### flutter via flutter_rust_bridge

- **skia** is the renderer — probably the most polished 2D
  stack on earth right now
- hot reload is genuinely good
- proper typography, proper gestures, proper everything on
  every platform

why it fits: if "the look" is decisive, this is the look.

why to worry: dart is on the other side of the bridge. the
model has widgets defined in dart; your moof code is
instructing dart to build widgets. it's a wide boundary.

ideal use: moof-cap-flutter as a capability that exposes a
minimal widget DSL, with dart-side glue that just translates.
possibly overkill; possibly exactly right.

### qt via cxx-qt

- QML is genuinely smalltalk-adjacent: live objects, property
  binding, imperative-on-declarative
- huge, mature, every platform
- batteries included — you get a real text editor widget, a
  real code editor widget, a real tree view, etc.

why it fits: qml's object-property-binding model is close to
what moof wants to project.

why to worry: qt is *big*. the license is permissive-ish but
real (LGPL with static-linking caveats for commercial). dep
weight is significant.

### gtk via gtk-rs

- stable, linux-native
- retained-mode, tree-based
- less pretty by default, fewer platforms

why it fits: rock-solid, decades of work.

why to worry: gtk4's direction doesn't feel aligned with
creative UI work; it's app-shaped.

### cxx-skia

- just the renderer, no widget layer
- same reliability as flutter's backend
- much larger build surface than vello

why it fits: if we want skia's polish without flutter's dart.

why to worry: we'd be building our own widget layer on top.
vello is the moral equivalent with a rust-native build story.

---

## wildcards

### lively.next in a webview

the jaw-dropper. **lively.next** is the current incarnation of
lively kernel — a smalltalk-family live environment written in
javascript, by dan ingalls (yes, *that* ingalls) and team. it
runs in a browser. it has halos. it has a class browser. it has
everything.

binding: moof runs a local webview capability. the webview
hosts lively.next. moof and lively talk via postMessage /
WebSocket. moof is the persistent kernel; lively is the morphic
surface.

why it fits: you're bolting a finished morphic environment onto
moof. all the authoring gestures already exist. halos, direct
manipulation, the class browser — all there, all working, all
in javascript which is extensible.

why to worry: it's a huge dependency. the impedance match
between moof's vat model and lively's js runtime could be
rough. two objectspaces glued together. if it works, it's
genius; if it doesn't, it's a maintenance nightmare.

### vello alone (we build our own morphic)

pure version of the xilem path: bind just vello + parley, build
the entire widget layer in moof. every morph is a moof object
with a `render` method that emits vello primitives.

why it fits: this is the *honest* authoring-for-all path.
everything is moof all the way down. no privileged widget layer.

why to worry: it's a *lot* of work. probably a year of
smalltalk-grade effort before we catch up to what xilem gives us
out of the box. and we'd be solving problems (text input, IME,
accessibility, clipboard) that are already solved elsewhere.

reasonable compromise: bind masonry (xilem's widget layer) as
the starting widget kit, expose vello directly for canvas/
morphic work, let moof code define new widgets over time —
either by subclassing masonry widgets or by dropping to vello
directly. gradient from "just use the widgets" to "draw your
own" matches the continuous-ladder principle.

### humble UI (clojure, nikita prokopov)

- skia-based, rust-interop possible via FFI
- explicitly designed with a smalltalk-y philosophy
- tiny, opinionated, alive

why it fits: spirit alignment. prokopov cares about the same
things we do.

why to worry: clojure on the other side. interop is real work.

### terminal + kitty graphics protocol

worth mentioning even though it's weird.

- kitty, wezterm, ghostty, iterm2 all speak image / graphics
  protocols
- you can draw images inside terminal cells, blend with text
- proper typography is free (terminal renderer)
- works over ssh, works on a potato laptop

ideal use: a "lite" moof ui that's text-first with inline
graphics. the canvas happens in a real window; the inspector
lives in the terminal. works today.

why to worry: you can't zoom, can't free-form place things.
text-first caps you at text-first.

### custom wayland compositor

moof windows are wayland surfaces. moof *is* your window
manager on linux.

why it fits: ultimate "the system is the UI" play. the screen
is the image.

why to worry: wayland is a universe of its own. platform-
locked to linux.

### dynamicland-adjacent: projector + camera + paper

the bret-victor / dynamicland wild route.

- a short-throw projector points at a table
- a usb camera watches the table
- paper with printed aruco markers is tracked
- moof objects are projected next to the markers, projected
  output responds to paper arrangement

why it fits: the purest "authoring is physical" story. total
bret-victor. real dynabook vibes.

why to worry: hardware setup is ~$1-2k. only works in a
dedicated space. engineering is nontrivial (opencv +
calibration + gpu compositor).

worth building as a long-term moonshot demo. not the primary UI.

### croquet SDK / multisynq

alan kay's later collaborative-objects project. deterministic
replicated computation across clients. vats across machines.

why it fits: philosophically the closest existing system to
moof's federation story.

why to worry: SDK is js/ts-shaped. commercial licensing. we'd
be adopting their replication model.

worth studying even if we don't bind it. the replicated
deterministic computation idea is a better model for federation
than most CRDT work.

### godot

the game engine. node tree, editor, scripting, hot reload.

- every node is an identity-preserving object with properties
- the editor is built in and customizable
- 2D and 3D both
- open source, cross-platform

why it fits: godot's scene tree is morphic-adjacent. it's a
live object environment with a built-in editor. you could ship
moof's canvas as a godot project.

why to worry: godot's mental model is game-shaped. users who
see "godot" think games. the aesthetic doesn't quite say
"grimoire." and integrating moof with gdscript would be a real
bridge.

### rebol/red view dialect

rebol's UI was a *dialect* — a DSL expressed in rebol values.
you wrote `[size 400x300 button "click" [print "hi"]]` and got
a window. the ui was *literally* data.

why it fits: this is the cleanest "UI is data" model that
ships in any language. exact spiritual alignment with moof.

why to worry: rebol/red are low-traction ecosystems. we'd be
bridging into a niche-of-a-niche. but the *concept* is worth
stealing.

### mathematica notebooks

proprietary but: wolfram's notebook interface is probably the
closest thing in commercial software to moof's grimoire idea.
live evaluation, rich typeset output, expressions are values,
the notebook IS the program, graphics inline with everything.

why it fits: aesthetic + semantic inspiration.

why to worry: proprietary, can't bind to, but it's worth
playing with one for a week to understand what polished
living-document UX feels like.

---

## my picks

### short-term (experimental, in parallel with foundations)

**terminal + kitty graphics protocol, via a small moof-cap-kitty
capability.** easy to build, works today, gives us inline
graphics in the repl. lets us iterate on the object-viewing
protocols (`view-as-text`, `view-as-card`) without committing
to a big UI framework. emacs-native, ssh-native.

expected size: ~500 lines of rust. ~1 week of work.

### medium-term (post-foundations, first real UI)

**masonry (xilem's widget layer) + vello + parley, via
moof-cap-canvas.** this gives us the morphic surface for real
work. the inspector, notebook, and canvas all become moof
objects rendering into a vello scene graph.

this is a multi-month commitment. the payoff is a UI that's
coherent, all-moof-on-top, performant, and won't feel dated in
three years.

### long-term (the moonshot)

**grow the vello + masonry layer until moof *is* a morphic
system.** every widget is a moof object. the inspector is a
workspace. the canvas is a workspace. the repl is a workspace.
the user can conform any of their own objects to the `Morph`
protocol and drop them into the canvas. halos everywhere. the
ladder is continuous.

### the dark horse to watch

**lively.next embedding.** if the vello + masonry path takes too
long, or we want a jump-start on authoring gestures we don't
know how to design from scratch, lively.next is a 30-year-deep
reservoir of morphic design decisions by people who literally
invented morphic. worth a prototype even if we don't ship it.

---

## what's explicitly off-limits

- **any framework whose authoring story is "edit these XML/YAML
  files to configure the UI."** moof authoring is done in moof,
  not in a sidecar config language.
- **any framework that mandates a separate IDE.** no "use this
  editor to build your UI." the editor is moof.
- **any framework that charges per-seat commercial licenses on
  open-source usage.** fine for their business, not fine for
  the accretive grimoire model where users share workspaces
  freely.
- **any framework without working accessibility (screen readers,
  high contrast, keyboard-only).** moof is a medium; media are
  for everyone.

---

## decision process when we get there

when it's time to pick (probably after foundations + initial
inspector design), the process should be:

1. **prototype three** — pick three from the shortlist, build
   the same small thing in each (the inspector showing a moof
   object's slots with click-to-drill-down). a week each.
2. **use them.** for a week after each prototype, *actually*
   use that UI to navigate our own image. feel the latency,
   the typography, the mental overhead.
3. **decide based on feel, not on feature lists.** the right
   answer is whichever one *disappears* — the one where you
   stop noticing the framework and just work.

and: keep the binding narrow. whatever we choose, wrap it
behind a capability interface that moof code sees as a generic
"scene" / "morph" protocol. if we pick wrong, we switch the
capability, not rewrite moof code.

---

## addendum: what we're not optimizing for

- **cross-platform 100% parity.** mac-first is fine. linux
  second. windows when someone needs it. we're building a
  grimoire, not salesforce.
- **accessibility compliance.** we care about accessibility
  (see above), but we're not certifying against WCAG. we're
  building toward emergent accessibility because the underlying
  model is inspectable-everywhere.
- **60fps on every hardware.** smooth is nice. correctness is
  required. typographic quality is required. smoothness
  follows.
- **web deployment.** moof can be shipped to the web someday,
  but "runs in a browser" is not a constraint that gets to
  shape the UI model. the browser is a target, not a frame.

---

*worth re-reading authoring-vision.md before making any UI
decision. the framework is the one that makes those principles
easier, not the one with the best demo reel.*
