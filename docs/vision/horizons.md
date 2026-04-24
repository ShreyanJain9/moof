# horizons

**type:** vision

> what moof is aiming at beyond the current substrate. these are
> commitments in direction, not plans with dates. each section says
> what the horizon is and what the current gap is.

---

## the canvas

**the horizon.** a zoomable, inspectable, infinite UI where every
object renders itself, every rendering has halo-accessible source,
and editing code looks the same as arranging documents. morphic
reread through moof's commitments: vector-first, deeply
authored. no single aesthetic — the medium supports both
typographic minimalism (plan-9 adjacent) and rich skeuomorphism
(hypercard adjacent) per view protocol. no windows in the
overlapping-rectangles sense; just objects, views, and space.

**the specific pieces**

- **rendering is a protocol.** a value conforms to `Renderable` by
  implementing `render: medium`. the same object renders
  differently in `text`, `canvas`, `dot-graph`, `json-tree`
  because different mediums are different message targets. views
  aren't widgets; they're messages.
- **skeuomorphism where it communicates.** a switchboard *should*
  look like switches. a notebook *should* look like paper if
  that's what the content wants. a file stack *should* look like
  stacked cards. these are authoring choices per type, not a
  house style. hypercard's lovingly-drawn buttons were part of
  what made it feel authorable. moof lets a view be detailed and
  physical, or quiet and typographic — the principle is that
  every visual element should signal purpose, not decorate.
- **halos are verbs.** click-and-hold on any pixel; a polymorphic
  ring of verbs appears, contributed by the object itself through
  `halo-verbs`. inspect, edit, duplicate, conform, stash. the halo
  is the universal interface.
- **layers of aspects.** one object, many views stacked. you see
  a recipe as a rendered document; drag the aspect handle; now
  you see it as JSON; drag again; now you see the prototype
  chain. the aspect is a handler you call on the object.
- **spatial memory.** objects you place on the canvas stay where
  you put them. the canvas persists like any other part of the
  image. close moof, reopen, your workspace is still arranged.
- **direct-manipulation editing of handlers.** find a view, hold
  a modifier, see its source. edit it. live.

**current gap.** the stdlib has `Showable` and `describe`. there is
no Renderable protocol, no canvas, no halo. the UI today is a
terminal REPL. the substrate supports everything described above
— it just hasn't been built on top.

**what gets us there.** wave 10 (image-first boot) unblocks vat-0
owning live-reload. wave 11-ish (canvas protocol + vello renderer)
makes rendering a first-class thing. then progressively: halo
handlers, aspects, the inspector-as-canvas-object, direct handler
edit.

---

## the agent

**the horizon.** an LLM that lives in a vat, holds membrane-filtered
capabilities, participates in the substrate as a peer. the agent
reads moof, writes moof, collaborates on your workspaces. you can
share the agent a URL to a problem and it shows up. you can watch
the agent type. you can reject or accept its edits. you can give
the agent narrower or broader capabilities.

**the specific pieces**

- **a vat with an LLM inside.** the vat receives messages, composes
  prompts, calls out to the model API through a capability, parses
  responses into moof-values, sends messages back. the model is a
  FarRef target just like any other vat.
- **membranes control what the agent can do.** you grant it
  narrow capabilities: read a specific workspace, write to a
  specific scratch vat, call specific APIs. membranes intercept
  every cross-boundary send and log, allow, deny, or transform.
- **the agent's reasoning is visible.** its prompts, responses,
  tool calls, rejections — all are objects in the image. you can
  inspect them. you can fork them.
- **tools are moof objects.** an MCP tool is a moof handler the
  agent is allowed to call. adding a tool is adding a handler
  with a description. the image is the tool registry.

**current gap.** there's no agent integration today. an earlier
moof version had MCP over stdio; that's v1 and archived. the
capability and membrane primitives that make the agent safe exist
in design but not in implementation.

**what gets us there.** capability/membrane plumbing (post-wave-10).
agent vat with a specific LLM capability. tool registration as
handler registration. UX for inspecting agent thought.

---

## federation

**the horizon.** the web of objectspaces. you share a URL; someone's
client resolves it over the network; they see your thing. shared
objects dedupe via content-addressing — your list and my list are
stored once globally. changes propagate through opt-in subscription.
conflicts are conversations.

**the specific pieces**

- **URLs extend across machines.** `moof:peer/alice/vats/12` is
  alice's vat 12, reachable through a FarRef that does network
  sends under the hood. the syntax matches local: `[alice-vat
  msg]`. a send to a FarRef returns an Act whether the FarRef
  points next door or across the internet.
- **content-addressed cache.** when you receive a value, its hash
  resolves to local bytes if you have them. federation cost is
  proportional to novelty, not size.
- **subscription as protocol.** you subscribe to a peer's
  workspace. their changes arrive as a stream of messages,
  applied (opt-in) to your image. think RSS for objects.
- **conflict resolution is visible.** when two peers change the
  same server, both versions arrive; the reconciliation is a
  message the user sees. sometimes auto-merge (CRDT-amenable
  types); sometimes user choice.
- **signatures and trust.** values can be signed. trust is
  per-signer and configurable. no "verified checkmarks" — each
  user decides what signer identities they trust.

**current gap.** moof runs locally only. FarRefs work across vats
in one process. there's no network layer, no peer discovery, no
sync.

**what gets us there.** content-addressing is already wired (wave
8). the protocol-over-socket work lives in the "way later" bucket.
the primitives are ready; the delivery isn't prioritized until the
local substrate is rock-solid.

---

## the personal database

**the horizon.** your image is your database. no imports, no
exports, no "add a data source." you put recipes in it; you search
them; you query them; you build reactive dashboards on top. the
same image hosts your notes, your code, your tasks, your tools.

**the specific pieces**

- **collections as tables.** any Iterable of same-shaped objects is
  a table. you can `[coll where: |x| ...]`, `[coll groupBy: ...]`,
  `[coll join: other on: |a b| ...]`. queries are messages.
- **full-text search as an index.** a standard index is a server
  that watches new values and maintains searchability. you
  compose new kinds of indexes by writing servers.
- **reactive views.** a view is a server that watches its sources
  and emits updates. your dashboard is alive.
- **schema-emergent, not schema-first.** you build objects and
  the shapes show up. later, you can write a defprotocol that
  describes what you've been building and conform to it.

**current gap.** collections exist, basic `where:` and `map:` work.
indexing and reactive views exist in the flow/ directory but aren't
wired to real query planning. full-text search is a plugin
integration not yet shipped.

**what gets us there.** stable Iterable/Indexable (done), unified
transducer pipeline (wave jubilee), index server framework, then
reactive-views, then search integration.

---

## authoring-for-all

**the horizon.** users who don't write code still extend moof,
through gestures. conform-button. halo-edit. view-protocol-picker.
the ladder is genuinely continuous. a non-programmer authors their
own tools and shares them.

**the specific pieces**

- **conformance as click.** the inspector offers "make this
  renderable" as a button. click it; a protocol picker; install
  stubs; the user fills in behavior with a WYSIWYG gesture or a
  tiny expression.
- **hypercard-style scripts attached to affordances.** a button on
  the canvas has a script slot; click to edit; the script is a
  handler; the gesture is direct.
- **sharing is one click.** "share this workspace" generates a
  URL; pasting it anywhere opens it in the receiver's moof. the
  workspace carries its dependencies via content-addressing.
- **simple tutorials live as workspaces.** the user opens them
  and the tutorial IS the thing they're learning.

**current gap.** there is no GUI. there are no gestures. authoring
today is text at a REPL. every commitment described here assumes
the canvas exists.

**what gets us there.** everything above, built in sequence. the
canvas is the gating horizon; once it lands, authoring-for-all is
an emergent property of doing it right.

---

## the test for a horizon

a horizon isn't a feature we might add. it's a direction we
committed to. we test candidate changes against this list:

- **does this get us closer to the canvas horizon?** (if no, do
  we still want it?)
- **does this fit with the agent-in-a-vat model?** (or will we
  have to rewrite when the agent lands?)
- **does this work federated?** (or is it baking in a single-
  machine assumption?)
- **does it preserve personal-database properties?** (or is it a
  silo?)
- **does it make authoring-for-all easier or harder?**

some changes trade one horizon against another. we accept that.
but a change that hurts every horizon is a change we refuse.
