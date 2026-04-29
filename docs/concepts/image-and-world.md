# image and world

> **the world is the program. boot is wake. quit is sleep. there is
> no "outside the system." every artifact you create is a citizen of
> the same persistent place.**

we deliberately use the smalltalk-80 word: *image*. but we adapt it
for federation: the world is *many* small images (vats), each saving
itself, in conversation.

## the world

a *world* is the set of vats running together on one process (or
distributed across many). it has:

- a **root supervisor** — the topmost vat, parent of all others.
- a **path-table** — the namespace for named addresses
  (`/users/shreyan/notes/today`).
- a **registry** — a built-in vat that exposes `[$registry vats]`,
  `[$registry paths]`, etc.
- a set of **primordial caps** held by the root supervisor and
  granted downward.

worlds boot from the contents of `.moof/` on disk:

```
.moof/
  manifest.toml         which root supervisor proto, which startup vats
  vats/
    <vat-id-1>/         per-vat directory (concepts/persistence.md)
    <vat-id-2>/
    …
  registry/             world-level registries
  cache/                bytecode caches and other derived artifacts
```

## boot

booting a world:

1. read `manifest.toml`. instantiate root supervisor.
2. supervisor reads its own state (its vat directory).
3. supervisor reads the manifest's "auto-start" vats and brings them
   up in dependency order.
4. each vat boots independently (mmap + journal-replay).
5. vats reconnect via far-refs (lazily, on first message).
6. once root supervisor signals "ready," the user-facing canvas / UI
   appears.

elapsed: typically under a second for a moderate-size world. lazy
page-faults via mmap mean *most state isn't loaded until referenced*.

## quit / sleep

quitting:

1. user signals quit (close window, signal, command).
2. root supervisor broadcasts `:prepare-shutdown` to its children.
3. each vat finishes its current message-turn, commits journal, signals
   ready.
4. root supervisor commits its own state.
5. process exits.

a forced quit (kill -9) loses any uncommitted turn-state, but
already-committed turns persist. on next boot, every vat replays its
journal tail and resumes.

## the manifest

```toml
# .moof/manifest.toml
[world]
name = "shreyan-personal"
version = 4

[supervisor]
proto = "RootSupervisor"
caps = ["$clock", "$random", "$out", "$err", "$fs", "$keyboard", "$screen", "$net"]

[auto-start]
workspace = { proto = "Workspace", caps = ["$out"] }
clipboard = { proto = "Clipboard", caps = [] }
inspector-default = { proto = "Inspector", caps = ["$screen"] }
```

human-readable, edit-by-hand recoverable. the world's "config" is
itself queryable inside the running world.

## first launch

a fresh world (no `.moof/` directory) bootstraps:

1. create `.moof/manifest.toml` with default contents.
2. create root supervisor vat, blank state.
3. spawn a default `Workspace` vat with an empty canvas.
4. show the canvas.
5. user starts working. the world saves itself per-turn.

(this is the user-facing "first 60 seconds" target —
`vision/manifesto.md`.)

## the world IS the program

we mean this literally. there is no "build artifact you ship."
there is no "deploy." there is no "configure." you have a world; you
inhabit it.

distribution is "share a vat directory" (or its `.mco`-derived
sibling for rust-bound objects). collaboration is "give your friend
a far-ref to your vat." backup is "copy the directory."

this is the smalltalk image philosophy, federated: many small images
in conversation rather than one giant image alone.

## inspirations

- smalltalk-80 image: kay et al.
- erlang/OTP applications and supervision trees: armstrong et al.
- emacs: a single, persistent, configurable text-environment that
  is also its own programming environment.
- nix / direnv / project-as-directory disciplines: the world *is* a
  directory.

## see also

- `concepts/vats.md` — what's inside the world.
- `concepts/persistence.md` — per-vat on-disk shape.
- `concepts/moldability.md` — what living inside the world feels like.
- `vision/manifesto.md` — the thesis.
