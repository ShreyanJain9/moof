# review protocol

**type:** law

> how PRs and design proposals are reviewed. short doc. the point
> is to be firm and consistent, not ceremonial.

---

## every change answers three questions

1. **does this violate a [substrate law](substrate-laws.md)?**
2. **does this violate the [stdlib doctrine](stdlib-doctrine.md)?**
3. **does this rot the docs?** (add "wave N TBD" comments, stub
   protocols, etc.)

if any answer is yes, the change needs either fixing or an
exemption. no third path.

---

## classes of change

### additive, law-compliant

new feature that satisfies all laws, doesn't duplicate existing
work, fits doctrine categories. **accept quickly.** just review
for style and test coverage.

### additive, partial violation

new feature that needs an exemption (e.g., wave 9.4's Registry is
a plain proto not a defserver — violating law 1 briefly).

**review:**
- is the exemption necessary? (often yes — wave ordering)
- is the wave target named?
- is the boundary clear?

**accept** with the exemption added to `docs/exemptions.md` (to
be created). **reject** if the exemption has no removal plan.

### subtractive / cleanup

deleting dead code, removing wave apology comments, fixing an
admitted bug. **accept by default.** line counts going down is
almost always good.

### rewriting a law

a proposal to change a substrate law or doctrine rule. these get
a separate design doc (not a PR) with:
- what the current law is
- why it's wrong
- what the new law should be
- what breaks / needs migrating

**review asynchronously.** law changes are rare and expensive.

---

## style rules that don't reject PRs

these are conventions, not laws:

- lowercase filenames (mostly)
- kebab-case moof identifiers (mostly)
- comment density in stdlib files
- specific function sizes

call these out in review, don't block on them.

---

## style rules that DO reject PRs

- violating a law without an exemption
- violating doctrine (adding a protocol with <3 conformers, etc.)
- leaving a "wave X TBD" comment in source instead of a backlog
  issue
- adding parallel abstractions for things already covered
- inline TODO/FIXME without a named resolution

these are structural, not aesthetic.

---

## exemption format

to be placed in `docs/exemptions.md` (created during the jubilee):

```markdown
## [short name] — violates law N

**location**: file:line or commit range
**rationale**: why the exemption is needed
**boundary**: what this exemption allows (and does NOT allow)
**wave target**: which wave resolves it
**owner**: who is responsible for removing it
**added**: date

details...
```

example:

```markdown
## Registry-as-plain-proto — violates law 1 + 6

**location**: lib/system/registry.moof
**rationale**: bootstrap replays source into every vat, so
  instantiating defserver at load-time recurses infinitely.
**boundary**: allows Registry to be a plain object with
  rebind-for-mutation. does NOT allow this pattern for any
  other new type.
**wave target**: 10 (image-first) removes the bootstrap replay;
  Registry becomes a defserver then.
**owner**: whoever picks up wave 10
**added**: 2026-04-23
```

no "eventual" without a wave. no exemption stays forever.

---

## doc rot

this is the one we're most likely to fail at. the rules:

- **no wave-N comments in source.** wave apologies go in
  `exemptions.md` or in the roadmap, not in .moof files.
- **no TODO without a wave.** if you can't name when it gets
  done, it's not a TODO, it's a dream — delete or move to
  exemptions.
- **no "for now" comments.** either it works or it doesn't.
- **no aspirational protocols.** if a protocol has <3 conformers,
  it doesn't ship. aspirations go in horizons, not in lib/.

docs that describe the CURRENT state must actually describe the
current state. aspirations live in vision/ with clear "this is
not yet true" labeling.

---

## the one-sentence rule

**when you can't decide: the change that preserves every law AND
doctrine rule wins.** preservation is the tiebreaker. adding
features is cheap; removing unused code is cheap; undoing a law
violation later is expensive.

---

## what this protocol is trying to prevent

- **wave apology accumulation.** source comments that say "wave
  9.4 this is a hack, wave 9.6 will fix" — these rot in place
  and become load-bearing documentation for bugs.
- **parallel abstractions.** Query AND Transducer AND Builder,
  each local-optimal, together a mess.
- **orphan protocols.** Reference, Interface, Buildable — each
  with <2 conformers, each pretending to be infrastructure.
- **hidden state.** rust-side grant matrices, rust-side
  schedulers, anything not visible through moof introspection.
- **docs that lie.** a concept doc saying "moof does X" when
  moof doesn't actually do X yet.

every one of these has bitten moof. the review protocol is how
we stop getting bitten.

---

## next

- [substrate-laws.md](substrate-laws.md) — the six laws being
  enforced.
- [stdlib-doctrine.md](stdlib-doctrine.md) — the stdlib rules.
- [../roadmap.md](../roadmap.md) — wave ordering for exemption
  expiration.
