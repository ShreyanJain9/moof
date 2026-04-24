# stdlib at a glance

> one page. the rule, the protocols, the decisions. if you only read
> one doc before touching `lib/`, read this.

## the rule

**every generic operation lives on a protocol. every protocol has 1–3
required methods and many derived ones. every type gets generic
behavior by conforming. no protocol without 3+ conformers.**

## the 9 protocols

```
Showable       show                         → a String for humans
Equatable      equal:                       → value equality
Hashable       hash                          → stable Integer for keys
Comparable     <                             → total order
Numeric        + - * = <                     → arithmetic values
Iterable       fold:with:                   → walkable sequence
Indexable      at:, count                   → random-access sequence
Callable       call:                         → invokable value
Thenable       then: + class-side pure:     → compose (do-notation)
                 provides: map:, recover: — NO ok? NO pending?
```

every other generic operation belongs inside one of these.

`Thenable` is moof's composition contract and the backbone of
do-notation. Cons, Option, Result, Act, Update, Stream conform.
Err/None override `recover:` to express failure; everyone else
uses the default (`self`, "nothing to recover from").

**Thenable is deliberately opaque.** no `ok?`, no `pending?`,
no probing. you compose via `then:` or `recover:`, and the
scheduler handles resolution. acts never expose "is it done?" —
you bind through them instead.

## the deletion list

- **`Reference`** (1 conformer) → delete
- **`Buildable`** (0 conformers) → delete
- **`Interface`** (0 conformers) → move to docs/
- **do NOT split Thenable.** earlier doctrine proposed splitting
  into Monadic + Fallible + Awaitable; reverted. Thenable stays
  fused with defaulted provides.
- **`Query`** (duplicates Transducer) → decide: delete or rewrite as sugar

## the rules of addition

before you add to `lib/`:

1. does it fit one of the 18 stdlib categories? (see full doctrine)
2. is it a provide on an existing protocol, or a method on a type?
3. if new protocol: do 3+ types conform TODAY? if not, stop.
4. does it duplicate something? replace or abandon — never parallel.
5. in doubt: write the specific method, not the protocol. promote later.

## the 18 categories

```
1.  value identity    Showable / Equatable / Hashable / Comparable
2.  numbers           Numeric + Range
3.  text              (no protocol yet — owed)
4.  sequences         Iterable / Indexable
5.  associations      Table (native + Indexable)
6.  sets + bags       Set / Bag
7.  option / result   Option / Result (both Thenable)
8.  time              (no types yet — owed)
9.  bytes             native
10. callables         Callable
11. concurrency       Act / Update / vat / Thenable
12. patterns          Pattern matching (fix match-constructor)
13. pipelines         Transducer (primary) / Stream (lazy)
14. reactivity        Atom / Signal
15. persistence       blobstore + save-image
16. namespaces        URL + Namespace walk
17. system            System / Registry / Service
18. introspection     typeName / prototypes / Inspector
```

## owed (not yet built)

- **Time types**: Duration, Timestamp. every real program needs these.
- **Textual protocol**: unify String / URL / Bytes-as-utf8.
- **Log protocol**: structured diagnostics layered over console cap.
- **Test harness expansion**: JUnit-style output, runnable dirs.

## the contract

PRs against `lib/` are reviewed against the [full doctrine](stdlib-doctrine.md).
violations get rejected on principle, not opinion. the cost of saying
no is cheap.
