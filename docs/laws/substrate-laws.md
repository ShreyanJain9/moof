# substrate laws

> **the inviolate guarantees the rust line provides. these are
> *promises*: if any of these breaks, the whole moldable claim
> breaks.**

these are tested in the substrate's own test suite (when written).
violating any of them is a substrate bug, not user error.

## L1 — one allocator, one Form kind

every value moof allocates is a Form. there are no second-class
"primitive" values that bypass the form-allocator. small-int
optimization (NaN-boxing) and similar perf tricks are *internal
representation*; conceptually every value is a Form with a proto.

## L2 — proto chain bottoms out at Object

every Form's transitive proto chain reaches `Object`, whose proto
is `nil`. there are no orphan Forms. there are no cycles in proto
chains (the substrate detects and refuses cyclic delegation).

## L3 — message dispatch is the universal verb

every operation that produces a value or causes an effect is
implemented as `send(receiver, selector, args)`. there is no
privileged ABI for built-in operations. internally, hot paths are
optimized via inline caches; semantically, every call is a send.

## L4 — eval is itself a send

`(eval form env)` is `send(form, :eval, [env])`. user code can
override `:eval` on a proto and the substrate will use it. there is
no "real evaluator" hidden in rust that bypasses this.

## L5 — source is canonical

every closure / method has a `:source` slot containing the *actual
source-form* it was compiled from. bytecode is derived. editing
source invalidates bytecode caches. the substrate must regenerate
bytecode from source when needed; it must never lose source.

## L6 — reflection is total

every Form exposes `:proto`, `:slots`, `:handlers`, `:meta`,
`:source` (when applicable), `:identity`. closures additionally
expose `:bytecodes`, `:caps-required`, `:purity`. running frames
expose `:locals`, `:stack`, `:method`, `:pc`, `:self`. (formal list:
`laws/reflection-contract.md`.)

## L7 — vat boundaries are absolute

raw form-ids do not cross vat boundaries. any value leaving its
origin vat is auto-promoted to a far-ref by the substrate's
serialization layer. user code cannot bypass this. `laws/isolation-
laws.md`.

## L8 — mutation is journaled

every committed mutation appends to the vat's journal. there is no
substrate-level "stealth mutation" that bypasses the journal. user
code can flag mutations as `:redacted` (omits payload, keeps
seq-id), but the seq-id is still written.

## L9 — capabilities are unforgeable

the substrate provides exactly two ways to obtain a capability:
1. be the root supervisor at boot, granted primordial caps.
2. receive an attenuated cap from a holder.

there is no constructor for caps that pure-moof code can call.
there is no rust escape hatch in `.mco` natives that bypasses cap
verification (the rust trampoline registry enforces this).

## L10 — proto mutation is observable

editing a proto's handler table fires a substrate-level event:
existing inline caches at sites resolved against the modified
proto are invalidated; the journal records the proto-edit; observers
of the proto receive `:proto-changed` notifications.

## L11 — identity is stable within a vat

a Form's `:identity` (heap-id) is stable for the lifetime of the
Form within its vat. it survives serialization round-trips:
saving and loading a vat preserves form-ids. (cross-vat identity is
*not* preserved; far-refs use vat-local form-ids of the target.)

## L12 — `become:` swaps cleanly

`[a become: b]` swaps the heap contents of `a` and `b` such that
every existing reference behaves as if a/b had been the other all
along. no orphaned references; no torn state. `become:` is a
substrate primitive that takes effect at message-turn boundaries.

## L13 — message-turn ACID

within a vat, each message-turn is atomic, consistent, isolated, and
durable:
- atomic: all mutations of the turn either commit or none do.
- consistent: invariants enforced by the substrate (proto-chain
  acyclicity, isolation, etc.) hold at turn boundary.
- isolated: no other vat sees mid-turn state.
- durable: once turn-end is signalled, mutations are on disk.

## L14 — the inbox is ordered

messages within a vat's inbox are processed in arrival order.
selective receive (skipping a message to take a later one) is a vat-
behavior concern, not a substrate guarantee. but the substrate
guarantees the order in which messages are *delivered* to the inbox.

## L15 — pure means pure

if the analyzer marks a method `#pure`, the substrate is free to
memoize, reorder, or parallelize it. for this to be sound, `#pure`
must mean exactly:
- no `$cap` argument received.
- no message sent to a far-ref.
- no slot-write of a Form not allocated within this call.
- no effect on the vat's mailbox or supervisor.

`#pure` may not be ascribed to a method that violates these. the
analyzer must conservatively label as `#unknown` when it cannot
prove purity.

## L16 — the journal is append-only

journal entries, once written, are not modified or deleted. the
substrate does not mutate past entries. compaction creates a
*new* snapshot that supersedes a journal *prefix*; the prefix is then
retired but its existence and seq-id range are preserved in metadata.

## breaking these is the only thing that matters

any substrate change that risks breaking one of these laws requires
a documented audit and explicit decision. if you find yourself
arguing about whether to bend one, you are touching foundations and
should be deliberate.

## see also

- `laws/reflection-contract.md` — formal reflection guarantees.
- `laws/isolation-laws.md` — vat-boundary rules.
- `laws/purity-and-effects.md` — formal purity rules.
- `laws/determinism-laws.md` — what replicated vats observe and
  refuse.
- `concepts/forms.md` — what L1, L6 protect.
- `concepts/vats.md` — what L7, L13, L14 protect.
- `concepts/replication.md` — replicated-vat mode that depends on
  L13 + determinism-laws.
- `concepts/effect-intents.md` — how L8's "mutation is journaled"
  splits into mutation-log + input-log + effect-log.
