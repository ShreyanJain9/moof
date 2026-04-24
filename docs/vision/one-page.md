# moof in one page

**type:** vision

---

moof is a **persistent, concurrent objectspace** — one computational
substrate where the tools you use, the code you write, and the data
you produce are all made of the same material, and none of them
vanish when you close your laptop.

## the one operation

`[obj selector: arg]` — send a message to an object. that's it. the
only operation moof has. function calls, arithmetic, control flow,
slot access, IO, concurrency — all message sends.

`(f x)` desugars to `[f call: x]`. `[3 + 4]` is a message to an
integer. `obj.x` is `[obj slotAt: 'x]`. `(if c a b)` is a message to
a boolean. even `def` is a message to an environment.

## the material

an **object** has:
- **slots** — public, fixed-shape data
- **handlers** — open, overridable behavior
- a **prototype** it delegates to when a handler isn't found locally

that's all. no classes. no metaclasses. no second-class citizens.
integers, strings, cons cells, tables, vats, the canvas — everything
is an object. the VM has optimizations but the semantics are uniform.

## the commitments

- **the image persists.** close moof, reopen it, everything is still
  there — not because you saved, because objects just exist.
- **references are capabilities.** if you don't hold a reference to
  the Console, you can't print. no ambient authority. no global state.
- **vats are isolation.** a vat is a single-threaded actor with its
  own heap. cross-vat messages are async and return Acts. nothing
  crosses a vat boundary except a message.
- **protocols are the type system.** a protocol is a contract:
  "implement `fold:with:` and get 50 collection methods free." no
  classes; conformance is nominal + structural.
- **every value is addressable.** immutable values by content hash
  (`moof:<hash>`). live references by path (`moof:/vats/7/objs/42`).
  you can send a friend a URL to a vat.
- **authoring is the UI.** there is no separate "developer mode."
  you see a rendered thing, you can see its source. you edit the
  source, it re-runs. the repl, inspector, and (eventually) canvas
  are all moof objects.

## the lineage

moof inherits on purpose:

- **smalltalk** — the image, prototype delegation, `doesNotUnderstand:`
- **plan 9** — namespaces as values, everything addressable by path
- **erlang / BEAM** — vats, let-it-crash, supervision, async messages
- **alan kay / engelbart / atkinson** — the continuous ladder: no
  mode boundary between using a tool and building one
- **E language** — capability security, far-refs, promise pipelining
- **git / IPLD** — content-addressing, merkle DAGs, federation

## what moof is NOT

not a scripting language. not an operating system. not a database.
not an IDE. it's one substrate trying to be all of those from one
object model. when it works, there's no distinction between your
editor and your workspace and your notes and your programs. they're
all objects in the image.

## where to read next

- full vision → [manifesto.md](manifesto.md)
- the model → [../concepts/objects.md](../concepts/objects.md)
- the horizon → [horizons.md](horizons.md)
