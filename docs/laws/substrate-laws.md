# substrate laws

**type:** law

> six invariants the moof substrate is committed to. laws are
> rules we enforce, not wishes we aspire to. every code change is
> reviewed against them.

---

## why laws

without global invariants, every wave adds local wins and global
incoherence. moof's early waves shipped features without rules; the
result was duplication (multiple ways to thread state), protocol
orphans (declared but unused), and subtle constraint violations
(registry can't be a defserver because bootstrap recurses).

laws close this. each law:
- names a property we commit to maintaining.
- lists what CURRENT moof satisfies vs what's in violation.
- explains what the law unlocks.
- names the exception mechanism (if any) and when exceptions
  expire.

when a PR violates a law with no exemption, the PR is rejected.
the cost of saying no is cheap; the cost of drift isn't.

---

## law 1 — vats are objects

**vats are first-class moof objects.** you spawn, inspect, and
message them through the moof object model. there is no special
"vat API" accessible only from rust.

**what this enforces**
- the Scheduler is a moof-level capability, not a rust-only
  entrypoint.
- `spawn`, `kill`, `introspect`, `supervise` are message sends to
  a Scheduler FarRef.
- vat metadata (id, status, mailbox depth) is inspectable via
  messages.

**current gap**
- `Scheduler::spawn_vat` is a rust method today. no moof-level
  Scheduler capability.
- spawning is done by moof-cli or by FarRefs via the runtime's
  outbox machinery, bypassing moof.

**unblocks**
- fork-a-vat, ship-a-vat, diff-two-vats at moof level.
- moof-side supervision policies.
- the "vat 0 owns boot" model (wave 9.6).

**wave target**: 9.5.

---

## law 2 — all mutable state lives in a vat and changes only via Update

**no in-place mutation anywhere.** the only way existing state
changes is: a server vat receives a message, returns an Update,
and the scheduler applies the delta between messages.

**what this enforces**
- `slotAt:put:` doesn't exist (correctly — already rejected).
- `env_def` from moof code only creates new bindings; never
  rewrites existing ones without going through Update.
- rust-side native handlers don't mutate Heap-owned objects
  behind moof's back.
- atoms, signals, and defserver state all express changes as
  Updates.

**current gap**
- there are a few rust-side escape hatches (registry.rs, system.rs
  mutating rust state invisible to moof).
- register_native mutates the heap (builds a block object) —
  arguable, since registration isn't "state the user sees."

**unblocks**
- time travel (every state change is a message, every state is a
  snapshot).
- rollback (an Update is a value you can invert).
- replay (message log + initial state = deterministic
  reconstruction).

**wave target**: ongoing — this is mostly honored; we audit
against it during the jubilee.

---

## law 3 — every live value has a URL

**every value in the running image is addressable by a URL.**
content-addressed URLs for immutable values (`moof:<hash>`).
path-addressed URLs for live ones (`moof:/vats/7/objs/42`).

**what this enforces**
- the namespace tree is comprehensive — no "hidden" objects
  unreachable by path.
- every FarRef carries a URL.
- the blob store hashes every persisted immutable value.
- federation uses the same URL scheme (with a `peer` prefix).

**current gap**
- not every object has an addressable path today. prototypes
  reachable via env, but not cleanly via `/protos/<name>`.
- intermediate Acts (pending) are hidden in vat internals, not
  URL-addressable.

**unblocks**
- federation (send a URL, receiver resolves).
- deep linking (a permalink to any live object).
- visual navigation on the canvas (URLs as addresses the halo
  exposes).

**wave target**: 9.0–9.6 (in progress); 10+ (full coverage).

---

## law 4 — reachability equals authority

**if you can't reach it from a root you hold, you can't call it.**
no ambient authority. no global namespace. no implicit grants.

**what this enforces**
- capabilities are FarRefs; holding one is the permission.
- the System capability mediates all grants; grants are events.
- no `env_def` that hands a capability to a vat that wasn't
  explicitly given it.
- every grant is recorded (audit trail).

**current gap**
- the grant matrix is rust-side (`moof-cli/src/system.rs`); it
  works but isn't a moof-level record.
- no real membrane support — we grant the full FarRef.
- no revocation — once granted, always granted.

**unblocks**
- principle of least authority for real (attenuated FarRefs via
  membranes).
- capability revocation.
- visual permission editors (ACL UI as a canvas gesture).

**wave target**: 9.6 (grants in moof); wave 11+ (membranes).

---

## law 5 — the image is the program

**no source code at runtime.** the kernel, the stdlib, the
prototypes, the handlers are all part of an image built at build
time. `moof build` produces a seed image; `moof run` hydrates it.

**what this enforces**
- no bootstrap source replay on every `moof run` startup.
- plugin ABI verification is at build time, not at shutdown.
- every runtime artifact is traceable to the seed image or to
  user mutations recorded in the image.

**current gap**
- today moof replays every .moof file in the bootstrap list into
  every vat at startup.
- this is why defserver-at-bootstrap recurses infinitely.
- this is why plugin ABI drift segfaults at runtime instead of
  compile time.

**unblocks**
- fast boot (no parsing at startup).
- defserver Registry (no recursion trap).
- plugin ABI checked at build.
- hot reload (as image deltas).
- federated deployment (ship an image, hydrate on arrival).

**wave target**: 10.

---

## law 6 — one dispatch, one effect

**message send is the only dispatch mechanism. Act is the only
effect type.** no exceptions. no try/catch. no separate
result-type ecosystem.

**what this enforces**
- every effectful operation returns an Act (directly or as a
  resolved value that might become one via `then:`).
- do-notation is the only composition mechanism for effects.
- Result (Ok/Err) is a Monadic value, not a separate control
  flow primitive.
- all message sends go through one dispatch path: local receiver,
  walk proto chain, invoke handler.

**what this removes**
- try/catch (already removed from VM).
- throw (already removed).
- ambient error channels.
- multiple "kinds" of handler dispatch.

**current gap**
- Thenable conflates Monadic + Fallible + Awaitable — the
  stdlib-doctrine splits this.
- Query + Transducer + Builder are three dispatch-ish state
  threadings; pending consolidation.
- scheduler's merge_deltas (for Update composition) is rust-only;
  should be derivable from Monadic composition.

**unblocks**
- one mental model. you understand send + Act, you understand
  moof.
- one visual representation (halo shows message; Acts show as
  pending).
- simpler VM (fewer opcodes, less special-case code).
- easier morphic rendering (one composition primitive).

**wave target**: jubilee; continues into 10+.

---

## exception mechanism

a law can be temporarily violated only with:

1. a **named exemption** — the specific code location + the reason.
2. a **wave target** — the wave that removes the violation.
3. a **boundary** — what the exemption allows and what it doesn't.

exemptions live in `docs/exemptions.md` (to be created at
jubilee). every exemption has an owner (who's responsible for
removing it) and a deadline (which wave).

an exemption without a wave target is either upgraded (becomes a
commitment, with a plan) or becomes a law rewrite (we admit the
original law was wrong). unresolved violations are bugs, not
exemptions.

---

## review protocol

every PR is reviewed for law compliance by the maintainer. when a
PR violates a law:

- if the PR is short and the law violation accidental → fix.
- if the PR introduces a needed exemption → add to exemptions.md
  with owner + target.
- if the PR is trying to rewrite the law → a separate proposal
  first, with rationale.

see [review-protocol.md](review-protocol.md) for the full review
flow.

---

## what's NOT a law

things that are rules of thumb but not laws:

- naming conventions (lowercase, kebab-case)
- comment density
- function size preferences
- documentation style

these are preferences. they're in the conventions doc; they don't
reject PRs.

laws are about SUBSTRATE INVARIANTS, not code-style aesthetics.

---

## the full set, one page

1. **vats are objects.** spawn/kill/inspect via moof.
2. **mutation via Update.** no in-place changes.
3. **URL for everything.** content or path addressed.
4. **reachability = authority.** no ambient power.
5. **the image is the program.** no source replay.
6. **one dispatch, one effect.** Act for effects, send for calls.

print this. pin it to the wall. every PR is a question: does
this preserve all six?

---

## next

- [stdlib-doctrine.md](stdlib-doctrine.md) — the parallel
  constitution for lib/.
- [review-protocol.md](review-protocol.md) — how these get
  enforced.
- [../roadmap.md](../roadmap.md) — when each law's current
  violations get resolved.
