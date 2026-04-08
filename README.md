# moof: a living objectspace

> *"clarus the dogcow lives again"*

moof is a place, not a language. you open it and you're *somewhere* — a
persistent, introspectable objectspace where objects are clickable,
editable, and alive. an AI agent lives inside the image as a co-inhabitant.
the REPL is the escape hatch, not the front door.

**[SYNTHESIS.md](SYNTHESIS.md)** — what v1 was, what went right, what went wrong
**[VISION.md](VISION.md)** — the full v2 design: browser-first, agent-native, LMDB-persistent

the v1 codebase is preserved on `archive/v1` (tagged `v1-final`).

## the idea in 30 seconds

- **three heap types**: Object, Cons, Blob. that's it.
- **one operation**: `send(receiver, selector, args)`. everything is messaging.
- **six kernel forms**: `vau`, `send`, `def`, `quote`, `cons`, `eq`. everything else is derived.
- **LMDB persistence**: the heap IS the database. no save, no load, no bootstrap after first run.
- **browser-first**: egui spatial graph of objects. click, inspect, edit. the environment IS the IDE.
- **agent-native**: an LLM lives in a vat with faceted capabilities. its actions are visible and revocable.
- **capability security**: vats, membranes, facets. a reference is a capability.

## status

v2 is in the vision/design phase. no code yet — we're getting the
architecture right before writing a line. see VISION.md for the full plan.

## license

MIT
