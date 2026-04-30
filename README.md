# moof

> a moldable environment. fourth attempt, second take. docs-driven.
> correctness-first.

## what is this

moof is an attempt at a moldable, persistent, multi-actor
environment in the lineage of smalltalk, self, erlang, e, croquet,
and the glamorous toolkit. it is *not* a language-with-a-repl; it
is a world you wake, change, and let sleep. the language is a
feature of the environment, not the other way around.

start here: [`docs/vision/one-page.md`](docs/vision/one-page.md).

## status

**phase 0 — vision and docs:** complete.

**phase A — substrate seed (rewrite):** about to begin. the
previous v4-take-1 attempt is summarized in
[`docs/process/state-of-the-implementation.md`](docs/process/state-of-the-implementation.md);
the new approach is in
[`docs/process/impl-plan-v4.md`](docs/process/impl-plan-v4.md).

what changed since the previous push (commit `67ae0da`):

- the docs took a stress-testing pass — see
  [`docs/process/audit-2026-04-29.md`](docs/process/audit-2026-04-29.md).
  external adversarial reviews (codex + gemini) ran in parallel;
  both independently arrived at the same load-bearing
  recommendation: separate replicated input logs from
  per-vat mutation journals; treat cap effects as data
  (intent + receipt) rather than out-of-band side effects.
- the previous src/ has been reset. its honest-but-incomplete
  interpreter would have compounded debt to clear. the new plan
  starts the substrate from scratch with substrate-laws as test
  gates, and with **moofpaint** — a multi-user, croquet-style,
  macpaint-inspired collaborative drawing app — as the named
  forcing function.

forcing function for the next milestone (phase F):

```
$ moof world ./worlds/test-world/             # alice; hosts.
$ moof world join wss://localhost:7878        # bob; joins.
# both inhabit the same 3D zoomable space.
# both fly, zoom, click. some inhabitants are pixmaps; some are
# counters; some are scratchpads. each is editable in place.
# both see each other's cursors. both see strokes within 50ms.
# both see live-edits to a tool proto propagate immediately.
# close + reopen: world state restored from the input log.
```

passing this test means the substrate honors its own laws —
replicated determinism, persistent input log, ambient per-replica
caps, live-editable protos, real 3D i/o.

three deliberate choices that shape the work:

- **3D from day one.** the world is a continuous 3D zoomable space
  ([`docs/concepts/world-and-space.md`](docs/concepts/world-and-space.md));
  every spatially-placed Form has a Pose; rendering is a recursive
  `:render-with: ctx` send. zoom = fly. inspect = ray-cast. macpaint-
  style pixmaps are textured planes floating in space.
- **MCO-as-dylib.** the substrate seed is ~3k LoC of rust. *every*
  performance-sensitive thing — lmdb, blake3, ed25519, wgpu,
  websocket, terminal renderer — ships as an `.mco` file:
  platform-tagged dylib + binding metadata, dlopened at runtime.
  the rust line stops growing after phase D
  ([`docs/concepts/compiled-objects.md`](docs/concepts/compiled-objects.md)).
- **parser + compiler self-host immediately.** phase A's bootstrap
  parser/compiler in rust are throwaway scaffolding; phase A-self-
  host loads `parser.moof` and `compiler.moof` and uses them
  thereafter. moof code is the canonical compiler, fully user-
  modifiable.

## docs

[`docs/`](docs/) is the source of truth. start with:

1. [`docs/vision/one-page.md`](docs/vision/one-page.md) — pitch.
2. [`docs/vision/manifesto.md`](docs/vision/manifesto.md) — thesis.
3. [`docs/vision/lineage.md`](docs/vision/lineage.md) — every
   inspiration, attributed.
4. [`docs/concepts/forms.md`](docs/concepts/forms.md) — the
   substrate primitive.
5. [`docs/concepts/replication.md`](docs/concepts/replication.md) —
   croquet-style replicated vats. **new for v4-take-2.**
6. [`docs/concepts/effect-intents.md`](docs/concepts/effect-intents.md) —
   intent/receipt model. **new.**
7. [`docs/concepts/moofpaint.md`](docs/concepts/moofpaint.md) — the
   forcing-function spec. **new.**
8. [`docs/laws/determinism-laws.md`](docs/laws/determinism-laws.md) —
   what replicated vats must observe and refuse. **new.**
9. [`docs/roadmap.md`](docs/roadmap.md) — phases A–G+, in order.
10. [`docs/process/impl-plan-v4.md`](docs/process/impl-plan-v4.md) —
   day-by-day plan for phases A–F.

40+ docs, ~10k lines. citations everywhere. the implementation
follows the docs (see
[`docs/process/docs-driven.md`](docs/process/docs-driven.md)).

## history

- `archive/v1`, `v1-final` — the first attempt.
- `v3`, `v3-final` — the third attempt.
- `master` — v4. clean room since `dcdf6ce`. reset again at the
  v4-take-2 commit (this commit, or its successor).

## license

unspecified for now. assume "personal use, eventually free." we'll
pick something proper before any external sharing.
