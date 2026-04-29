# moof

> a moldable environment. fourth attempt. docs-driven.

## what is this

moof is an attempt at a moldable, persistent, multi-actor environment
in the lineage of smalltalk, self, erlang, e, croquet, and the
glamorous toolkit. it is *not* a language-with-a-repl; it is a world
you wake, change, and let sleep. the language is a feature of the
environment, not the other way around.

start here: [`docs/vision/one-page.md`](docs/vision/one-page.md).

## status

**phase 1: substrate seed.** complete.

forcing function:

```
$ cargo run --quiet -- '(+ 1 2)'
3
```

what works:
- s-expression reader allocating real Forms (proto + slots + handlers + meta).
- bytecode compiler (Form → Chunk).
- bytecode interpreter with inline-cache slots at every send-site.
- send dispatch via proto-chain walk (substrate-laws.md L3).
- root protos: Object, Nil, Bool, Integer, Symbol, List, Builtin.
- native methods on Integer for `:+`, `:-`, `:*`, `:/`.
- global callables `+`, `-`, `*`, `/`, `println`.
- `moof` cli that evaluates one expression and prints the result.

what does not yet exist (each in its own phase, see
[`docs/roadmap.md`](docs/roadmap.md)):

- phase 2: persistence, vats, real parser-in-moof, GC, defs, lambdas.
- phase 3: inspector, debugger, become:, doesNotUnderstand:.
- phase 4: queries (datalog), types, capabilities.
- phase 5: distribution.
- phase 6+: tooling, package system, ecosystem.

## docs

[`docs/`](docs/) is the source of truth. start with:

1. [`docs/vision/one-page.md`](docs/vision/one-page.md) — pitch.
2. [`docs/vision/manifesto.md`](docs/vision/manifesto.md) — thesis.
3. [`docs/vision/lineage.md`](docs/vision/lineage.md) — every inspiration, attributed.
4. [`docs/concepts/forms.md`](docs/concepts/forms.md) — the substrate primitive.
5. [`docs/roadmap.md`](docs/roadmap.md) — phases, in order.

40 docs, ~7,260 lines. citations everywhere. the implementation
follows the docs (see
[`docs/process/docs-driven.md`](docs/process/docs-driven.md)).

## history

- `archive/v1`, `v1-final` — the first attempt.
- `v3`, `v3-final` — the third attempt (preserved for reference).
- `master` — v4. clean room since `dcdf6ce`.

## try it

```
cargo run --quiet -- '(* 3 (+ 4 5))'         # → 27
cargo run --quiet -- '(- 10 3 2)'            # → 5
cargo run --quiet -- '(println (+ 1 2))'     # → 3, then ()
```

## license

unspecified for now. assume "personal use, eventually free." we'll
pick something proper before any external sharing.
