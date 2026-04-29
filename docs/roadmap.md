# roadmap

> **what we build, in what order. each phase has a single forcing
> function — a thing that, when it works, signals readiness for the
> next phase.**

we explicitly resist building anything before its phase. earlier
phases are not allowed to assume later phases. when tempted to
"prepare for" a later phase, write the temptation in
`process/open-questions.md` and move on.

## phase 0 — vision and docs

**status:** in progress (this folder).

**deliverables:**
- vision/manifesto, lineage, one-page.
- concepts/* covering every substrate primitive.
- syntax/* covering the surface.
- laws/* with formal guarantees.
- process/docs-driven, open-questions.

**done when:** the docs read coherently to a fresh reader. external
adversarial reviews (codex, gemini) raise no foundation-level
objections.

**we do not write code in this phase.**

## phase 1 — substrate seed

**forcing function:** `(+ 1 2)` runs in a fresh moof process,
prints `3`, exits cleanly.

**deliverables:**
- rust crate: form heap + GC.
- rust crate: bytecode interpreter (~30 opcodes), send primitive,
  inline cache slot allocation.
- rust crate: bootstrap parser (minimal s-expressions).
- rust crate: the world boot sequence — read manifest, allocate
  root vat, dispatch initial message.
- a tiny bootstrap.moof that defines `+`, `-`, `*`, `/` and a
  println via a `$out` cap stub.
- a `moof` cli binary that runs a single expression.

**ratio:** ~1500 lines of rust + 100 lines of moof, give or take.

**not in scope:** persistence, datalog, type system, real parser,
real compiler, vats-as-actors, far-refs.

## phase 2 — vats and persistence

**forcing function:** a vat saves to disk per turn; quit and reboot
restores its state; `(println "hi")` works through a `$out` cap that
survives reboot.

**deliverables:**
- rust crate: per-vat lmdb store + journal.
- rust crate: vat scheduler with mailbox (single-process; no
  cross-process yet).
- rust crate: canonical encoding for Forms (binary).
- rust crate: bootloader that mmaps the store and replays journal-tail.
- moof: the proper parser (replaces bootstrap parser; written in moof,
  loaded as source on first boot, bytecode-cached).
- moof: a small stdlib of List, Tab, String, Number protos.
- moof: defproto / defop operatives.

**ratio:** another ~1500 lines of rust, ~2000 lines of moof.

**not in scope:** distribution, datalog, types, GUI, debugger.

## phase 3 — moldability features

**forcing function:** a user can edit a method via the inspector,
hit save, and the next call uses the new code.

**deliverables:**
- moof: inspector — a vat that renders forms with per-proto views.
- moof: live-editor — text editor for method source, recompile on save.
- moof: debugger — pause-on-error, frame inspector, edit-and-continue.
- moof: `become:` substrate primitive (rust support).
- moof: doesNotUnderstand mechanism + cap-attenuation primitives.
- moof: pattern-match library (in moof; substrate calls user code).

**ratio:** ~5000 lines of moof.

## phase 4 — query, types, capabilities

**forcing function:** `(query (?obj proto: Counter where: [?obj count > 100]))`
returns a data source streaming matching objects from the live world.

**deliverables:**
- moof: datalog rule and query operatives.
- moof: type system (`Type` proto, refinement / structural / dependent).
- moof: analyzer (effect inference, exhaustiveness checks).
- moof: capability machinery formalized — `$out`, `$clock`, `$random`,
  `$fs`, `$keyboard`, `$screen` caps with concrete leaves in rust.

**ratio:** ~5000 lines of moof, modest rust additions for new caps.

## phase 5 — distribution

**forcing function:** alice's vat at machine A sends a message to
bob's vat at machine B; bob's vat replies; alice sees the result.

**deliverables:**
- rust crate: cross-process transport (unix-socket).
- rust crate: cross-machine transport (tcp + websocket).
- rust crate: routing table maintenance.
- substrate: serialization promotion (auto-far-ref at boundary).
- moof: discovery (whatever we end up with — gossip, registry,
  bonjour).
- moof: shared workspaces / collaboration UI.

**ratio:** ~1500 lines of rust + ~3000 lines of moof.

## phase 6 — tooling and culture

**forcing function:** a stranger can read the docs, install moof,
and within a day have a useful environment customized to their work.

**deliverables:**
- moof: package system (whatever shape).
- moof: testing framework.
- moof: profiling and observability.
- moof: more inspector views, more morphic primitives.
- documentation polish, more examples, tutorials.

**ratio:** ongoing.

## phase 7 — sunset preceding attempts

archive any remaining v3 dependencies / scripts. v3 is permanently
preserved at branch `v3` and tag `v3-final`. master is v4 only.

**not a phase, more a cleanup.**

## guidelines for living with this roadmap

- **no skipping ahead.** writing rust for distribution before
  persistence is unsafe — the substrate hasn't paid for the safety
  invariants yet. resist.
- **no skipping back.** if phase 2 forces a redesign of phase 1,
  that's normal — fix phase 1's docs and rust. don't ship phase 2 on
  a broken phase 1.
- **forcing functions are not optional.** "phase n is done" means
  the forcing function works. nothing else suffices.
- **the docs lead.** if we discover a phase requires something
  undocumented, the docs go first. (`process/docs-driven.md`.)

## inspirations

- the rust 2015 / 2018 / 2021 edition rollout: phased substrate
  evolution with explicit cutover points.
- the smalltalk-80 bootstrap: a tiny image that loaded the rest of
  the world.
- the maru posture (piumarta): tiny seed; world grows itself.

## see also

- `vision/manifesto.md` — the why.
- `process/docs-driven.md` — the discipline.
- `process/open-questions.md` — what's still undecided.
