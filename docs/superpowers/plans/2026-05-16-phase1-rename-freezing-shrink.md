# Phase 1 — Rename, Freezing, Intrinsic Shrink Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land phase 1 of the vats+substrate carve: rename `crates/` to clean structure, complete the freezing primitive surface (vat-mode + auto-freeze + let-mutable), and remove redundant zig intrinsics whose moof equivalents are canonical.

**Architecture:** Three parallel-safe workstreams. **A (rename)** runs first and is mechanical. **B (freezing)** and **C (intrinsic shrink)** can run in parallel after A — they touch different parts of `intrinsics.zig` but localized; merge conflicts unlikely. Final integration task validates full polyglot bootstrap.

**Tech Stack:** Zig 0.16, OCaml (build-only, dune), Rust 2021 (workspace tooling), moof (lib/ source tree).

**Reference spec:** `docs/superpowers/specs/2026-05-16-vats-substrate-and-image-design.md` §1, §4, §12.2.

---

## File Structure After Phase 1

```
players/
  zig/           ← was crates/zig-substrate
    build.zig
    src/
    zig-out/
  rust/          ← was crates/substrate (still operational, safety net)
    Cargo.toml
    src/
seed/
  ocaml/         ← was crates/ocaml-seed
    dune-project
    bin/
    src/
tools/
  abi/           ← was crates/abi
  abi-rust/      ← was crates/abi-rust
  mco-pack/      ← was crates/mco-pack
lib/             (unchanged)
docs/            (unchanged)
tests/           (unchanged)
Cargo.toml       (workspace member paths updated)
README.md        (path references updated)
NEXT_SESSION.md  (path references updated)
```

**Substrate code changes (after rename, in `players/zig/src/`):**

| File | What changes |
|---|---|
| `world.zig` | Adds `vat_mode: VatMode` field; helper to determine auto-freeze policy; `VatMode` enum at top of file |
| `intrinsics.zig` | Adds `let-mutable` runtime support + auto-freeze hook; removes Cons/Integer/Float/String derived natives that have moof equivalents |
| `form.zig` | Adds `:freezable?` guard for vat-Forms / live faces (currently only checks already-frozen) |

**Moof code changes (in `lib/`):**

| File | What changes |
|---|---|
| `lib/early/12-vat-mode.moof` (NEW) | `let-mutable` macro; vat-mode helpers |
| `lib/main.moof` | Loads `12-vat-mode.moof` after defmethod |
| `lib/stdlib/freezing.moof` | Verify `freezeRecursive` semantics; expand cycle/live-boundary tests |
| `lib/stdlib/{cons,integer,float,string,char,object,method}.moof` | Verify all derived methods present + canonical (not shadowed); add any missing |

**Test surfaces:**

| Test file | What it covers |
|---|---|
| `players/zig/src/test_freeze.zig` (NEW) | Freeze locks slots/handlers/meta; mutation guard raises; dispatch on frozen walks proto |
| `lib/stdlib/test-freezing.moof` (NEW) | freezeRecursive cycle-safe; live-boundary skipped; vat-mode behavior; let-mutable scoping |
| `tests/conformance/freezing.json` (NEW) | image+message+expected triples for freezing |

---

## Workstream A: Directory Rename

Mechanical. Do this first, in one atomic commit per move where feasible. After this workstream, all subsequent tasks use new paths.

### Task A1: Survey current layout + baseline test pass

**Files:**
- Read: `Cargo.toml`, `crates/zig-substrate/build.zig`, `crates/ocaml-seed/dune-project`

- [ ] **Step 1: Read root Cargo.toml workspace members**

Run: `cat Cargo.toml`
Expected: lists `crates/abi`, `crates/abi-rust`, `crates/mco-pack`, `crates/substrate`.

- [ ] **Step 2: Baseline build the rust safety net**

Run: `cargo build --release -p moof --bin moof-rs`
Expected: builds successfully. Note the path of the resulting binary for later.

- [ ] **Step 3: Baseline build zig substrate**

Run: `cd crates/zig-substrate && zig build && cd -`
Expected: produces `crates/zig-substrate/zig-out/bin/moof`. Confirm exists.

- [ ] **Step 4: Baseline build ocaml-seed**

Run: `eval $(opam env --switch=wasm-mco) && dune build --root crates/ocaml-seed`
Expected: clean build.

- [ ] **Step 5: Baseline produce seed.vat + run polyglot bootstrap**

Run:
```bash
eval $(opam env --switch=wasm-mco)
dune exec --root crates/ocaml-seed bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./crates/zig-substrate/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -5
```
Expected: 22 stdlib files load; final error `UnboundName: Console` (per NEXT_SESSION). This is the **baseline integration check**. Save the output to confirm post-rename behavior matches.

- [ ] **Step 6: Note any extra files in crates/ to handle**

Run: `ls crates/`
Expected: directories `abi`, `abi-rust`, `mco-pack`, `ocaml-seed`, `substrate`, `zig-substrate`. Anything else surfaces here.

### Task A2: Create new top-level directories

**Files:**
- Create: `players/`, `seed/`, `tools/` (empty)

- [ ] **Step 1: Create new top-level dirs**

Run:
```bash
mkdir -p players seed tools
ls -ld players seed tools
```
Expected: three new empty directories.

- [ ] **Step 2: Commit the empty structure as a marker**

Run:
```bash
touch players/.gitkeep seed/.gitkeep tools/.gitkeep
git add players/.gitkeep seed/.gitkeep tools/.gitkeep
git commit -m "phase1/A: create players/, seed/, tools/ scaffolding"
```
Expected: commit succeeds. (We'll remove the .gitkeep files as real content lands.)

### Task A3: Move crates/zig-substrate → players/zig

**Files:**
- Move: `crates/zig-substrate/` → `players/zig/`

- [ ] **Step 1: Use git mv to preserve history**

Run:
```bash
git mv crates/zig-substrate players/zig
rm -f players/.gitkeep
ls players/
```
Expected: `players/zig/` exists; `players/zig/build.zig` exists; `players/zig/src/` exists. `.gitkeep` removed.

- [ ] **Step 2: Verify zig build still works from new location**

Run: `cd players/zig && zig build && cd -`
Expected: produces `players/zig/zig-out/bin/moof`. Build artifacts go to new location.

- [ ] **Step 3: Commit**

Run:
```bash
git add -A
git commit -m "phase1/A: crates/zig-substrate → players/zig"
```

### Task A4: Move crates/substrate → players/rust

**Files:**
- Move: `crates/substrate/` → `players/rust/`
- Modify: `Cargo.toml` (workspace member path)

- [ ] **Step 1: Use git mv**

Run:
```bash
git mv crates/substrate players/rust
```

- [ ] **Step 2: Update Cargo.toml workspace member**

Modify `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "tools/abi",
    "tools/abi-rust",
    "tools/mco-pack",
    "players/rust",
]
```

(Note: tools/* paths anticipated; we'll move those in subsequent tasks. For now the build will be broken until tasks A5-A7 land. That's fine within this commit boundary if we batch — but to keep each task self-checking, we'll update incrementally.)

For this task, update only `crates/substrate` → `players/rust`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/abi",
    "crates/abi-rust",
    "crates/mco-pack",
    "players/rust",
]
```

- [ ] **Step 3: Verify rust workspace still builds**

Run: `cargo build --release -p moof --bin moof-rs`
Expected: builds successfully. The crate name `moof` is unchanged; only the path changed.

- [ ] **Step 4: Commit**

Run:
```bash
git add -A
git commit -m "phase1/A: crates/substrate → players/rust"
```

### Task A5: Move crates/ocaml-seed → seed/ocaml

**Files:**
- Move: `crates/ocaml-seed/` → `seed/ocaml/`

- [ ] **Step 1: Use git mv**

Run:
```bash
git mv crates/ocaml-seed seed/ocaml
rm -f seed/.gitkeep
ls seed/
```
Expected: `seed/ocaml/` with `dune-project`, `bin/`, `src/`.

- [ ] **Step 2: Verify ocaml builds from new location**

Run:
```bash
eval $(opam env --switch=wasm-mco)
dune build --root seed/ocaml
```
Expected: clean build.

- [ ] **Step 3: Commit**

Run:
```bash
git add -A
git commit -m "phase1/A: crates/ocaml-seed → seed/ocaml"
```

### Task A6: Move crates/abi, abi-rust, mco-pack → tools/

**Files:**
- Move: `crates/abi/` → `tools/abi/`
- Move: `crates/abi-rust/` → `tools/abi-rust/`
- Move: `crates/mco-pack/` → `tools/mco-pack/`
- Modify: `Cargo.toml` (workspace member paths)

- [ ] **Step 1: Use git mv for each crate**

Run:
```bash
git mv crates/abi tools/abi
git mv crates/abi-rust tools/abi-rust
git mv crates/mco-pack tools/mco-pack
rm -f tools/.gitkeep
ls tools/
```
Expected: `tools/abi/`, `tools/abi-rust/`, `tools/mco-pack/` with their Cargo.toml files.

- [ ] **Step 2: Update Cargo.toml workspace members**

Modify `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "tools/abi",
    "tools/abi-rust",
    "tools/mco-pack",
    "players/rust",
]

[workspace.package]
version = "0.4.0-alpha.0"
edition = "2021"
authors = ["shreyan jain"]
description = "moof — a moldable, persistent, multi-actor environment"

[workspace.dependencies]
indexmap = "2"
libloading = "0.8"

[profile.release]
opt-level = 3
lto = "thin"
```

- [ ] **Step 3: Verify rust workspace still builds**

Run: `cargo build --workspace`
Expected: builds. All workspace crates resolve from new paths.

- [ ] **Step 4: Verify crates/ is empty (or only contains stragglers)**

Run: `ls crates/ 2>/dev/null`
Expected: empty or directory doesn't exist.

- [ ] **Step 5: Remove the now-empty crates/ directory**

Run: `rmdir crates 2>/dev/null && echo "removed" || echo "still has content"`
If "still has content", investigate `ls crates/` and handle each remaining item.

- [ ] **Step 6: Commit**

Run:
```bash
git add -A
git commit -m "phase1/A: crates/{abi,abi-rust,mco-pack} → tools/; remove crates/"
```

### Task A7: Update build paths, scripts, and docs

**Files:**
- Modify: `README.md` (any path refs)
- Modify: `NEXT_SESSION.md` (path refs)
- Modify: Any `.sh` scripts under `lib/mcos/*/build.sh` referencing `crates/`
- Modify: Any CI/workflow files referencing `crates/`

- [ ] **Step 1: Find all references to crates/**

Run: `git grep -l "crates/" -- ':(exclude)docs/superpowers/'`
Expected: list of files still containing `crates/` references.

- [ ] **Step 2: For each file in the list, audit and update**

For each file, run `git grep -n "crates/" -- <file>` and replace with the new path. Common patterns:
- `crates/zig-substrate` → `players/zig`
- `crates/substrate` → `players/rust`
- `crates/ocaml-seed` → `seed/ocaml`
- `crates/mco-pack` → `tools/mco-pack`
- `crates/abi` → `tools/abi`
- `crates/abi-rust` → `tools/abi-rust`

Exclude `docs/superpowers/` (historical specs / plans — those are time-stamped artifacts; don't rewrite history).

- [ ] **Step 3: Update README.md**

Modify the relevant sections of `README.md` to reflect new structure. Replace any `crates/zig-substrate` / `crates/ocaml-seed` etc. references with the new paths. (Read first to determine extent.)

- [ ] **Step 4: Update NEXT_SESSION.md path references**

Modify `NEXT_SESSION.md` table at the top (the `| crate | role | status |` block):

```markdown
| dir | role | status |
|---|---|---|
| `players/zig/` | THE runtime — heap, vm, gc, image, intrinsics, nursery, layout | substantial; 4000+ LoC zig |
| `players/rust/` | rust build-time oracle | WORKS but slated for deletion (W5e) once polyglot complete |
| `seed/ocaml/` | minimal bootstrap compiler | works; produces seed.vat (~91 KB, 305 chunks, 77 natives) |
| `lib/` | stdlib + parser + compiler + early macros | unchanged structurally; defproto auto-flatten added |
```

Also update the "starting the next session" steps to use new paths:
```
3. `cd players/zig && zig build && cd -` — produces `players/zig/zig-out/bin/moof`
4. `eval $(opam env --switch=wasm-mco)` then `dune build --root seed/ocaml`
5. `dune exec --root seed/ocaml bin/seed.exe -- ...`
6. `MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat`
```

- [ ] **Step 5: Update mco build scripts**

For each `lib/mcos/*/build.sh`, check for `crates/` references and update. Typical pattern: scripts reference `tools/mco-pack` (was `crates/mco-pack`). Replace as needed.

Run: `git grep -l "crates/" lib/mcos/`
For each, update.

- [ ] **Step 6: Verify everything still builds end-to-end**

Run the full baseline cycle:
```bash
cargo build --workspace
cd players/zig && zig build && cd -
eval $(opam env --switch=wasm-mco)
dune build --root seed/ocaml
dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -5
```
Expected: matches baseline from Task A1, Step 5 (final error `UnboundName: Console`).

- [ ] **Step 7: Commit**

Run:
```bash
git add -A
git commit -m "phase1/A: update build paths, scripts, README, NEXT_SESSION for new layout"
```

### Task A8: Verification — workstream A complete

- [ ] **Step 1: Final structure check**

Run:
```bash
ls -d players/zig players/rust seed/ocaml tools/abi tools/abi-rust tools/mco-pack
ls crates 2>&1 | head -3
```
Expected: all six new paths exist; `crates/` reports "No such file or directory" or is empty.

- [ ] **Step 2: All test/build commands work from new paths**

Run the full baseline cycle from Task A7 Step 6. Expected: works.

- [ ] **Step 3: Commit any final cleanup**

If there are stray artifacts, commit them with `phase1/A: cleanup post-rename`.

**Workstream A exit criteria met:** new directory structure in place; all builds work; baseline test (22-stdlib-load + UnboundName: Console) reproduces from new paths.

---

## Workstream B: Freezing Primitive Completions

Substrate already has freeze/frozen?/freezable? intrinsics, frozen bit on Form, and mutation guards in world.zig. **Phase 1 completes the surface** by adding:

1. vat-mode field on World (precursor to V4 per-Vat carve)
2. Auto-freeze policy when vat-mode is frozen-by-default
3. let-mutable runtime support + macro
4. Verify cannot-freeze-live for vat-Forms / live faces
5. Tests for the full semantics

### Task B1: Add VatMode enum + vat_mode field to World

**Files:**
- Modify: `players/zig/src/world.zig`

- [ ] **Step 1: Read the current World struct definition**

Run: `grep -n "pub const World" players/zig/src/world.zig | head -5`
Locate the struct definition (likely around line 30-50). Read 50 lines around it to understand current fields.

- [ ] **Step 2: Add VatMode enum at top of world.zig**

After the existing imports and constants, add:

```zig
/// vat-mode controls the default mutability for newly-allocated forms.
/// per design spec §4.1, this is set at vat-spawn time and immutable for
/// the vat's life. in V0 (single-vat), this is held on World; in V4
/// (multi-vat), it moves to per-Vat struct.
pub const VatMode = enum {
    /// new forms are born mutable; [form freeze] is explicit.
    mutable_default,
    /// new forms auto-freeze at end of their allocation expression.
    /// internal building during alloc is mutable; on alloc-expr exit,
    /// the form locks. for parsers, compilers, computation kernels.
    frozen_default,
};
```

- [ ] **Step 3: Add vat_mode field to World struct**

Locate `pub const World = struct {` and add this field near the top (after `gpa: std.mem.Allocator,` or equivalent — match existing conventions):

```zig
    /// vat-mode for this world. defaults to mutable_default for
    /// backward-compat with existing workspaces. moof code can set
    /// this at world-creation time via a yet-to-be-added intrinsic
    /// or via direct world.vat_mode assignment in tests.
    vat_mode: VatMode,
```

- [ ] **Step 4: Initialize vat_mode in World.init()**

Locate `World.init` (or equivalent constructor). Add to the initializer block:

```zig
            .vat_mode = .mutable_default,
```

(Adjust syntax to match existing field initializer style — likely matches comma-separated struct init.)

- [ ] **Step 5: Verify it compiles**

Run: `cd players/zig && zig build && cd -`
Expected: clean build. No new warnings or errors.

- [ ] **Step 6: Commit**

Run:
```bash
git add players/zig/src/world.zig
git commit -m "phase1/B: add VatMode enum and vat_mode field to World"
```

### Task B2: Add a vat-mode getter intrinsic

Expose `__vat-mode__` as a global that returns the current mode as a symbol.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`

- [ ] **Step 1: Locate the intrinsics registration table**

Run: `grep -n "INTRINSICS\|install.*intrinsic\|registerNative" players/zig/src/intrinsics.zig | head -20`
Find the table that maps intrinsic names to function pointers.

- [ ] **Step 2: Write the failing test first**

Create `players/zig/src/test_vat_mode.zig`:

```zig
const std = @import("std");
const testing = std.testing;
const World = @import("world.zig").World;
const VatMode = @import("world.zig").VatMode;

test "vat_mode defaults to mutable_default" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    try testing.expect(world.vat_mode == .mutable_default);
}

test "vat_mode is settable to frozen_default" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.vat_mode = .frozen_default;
    try testing.expect(world.vat_mode == .frozen_default);
}
```

- [ ] **Step 3: Wire the test file into build.zig**

Modify `players/zig/build.zig` to include `src/test_vat_mode.zig` in the test step. (If the build already auto-discovers tests under `src/`, this is a no-op; verify by reading `build.zig`.)

- [ ] **Step 4: Run the tests, confirm they pass**

Run: `cd players/zig && zig build test 2>&1 | tail -20`
Expected: two new tests pass (plus existing tests still green).

- [ ] **Step 5: Add the __vat-mode__ intrinsic**

In `players/zig/src/intrinsics.zig`, add a new intrinsic function and register it. Find where `__here__` or similar globals are registered. Add:

```zig
/// `__vat-mode__` — returns the current world's vat mode as a Symbol.
/// returns 'mutable-by-default or 'frozen-by-default.
fn vatModeIntrinsic(world: *World, _: []const Value) anyerror!Value {
    const sym_name: []const u8 = switch (world.vat_mode) {
        .mutable_default => "mutable-by-default",
        .frozen_default => "frozen-by-default",
    };
    const sym_id = try world.intern(sym_name);
    return .{ .sym = sym_id };
}
```

Register it in the intrinsics table:

```zig
    .{ "__vat-mode__", vatModeIntrinsic },
```

(Adjust function signature to match the existing intrinsic convention. Look at how `__here__` or another zero-arg intrinsic is defined and follow that pattern.)

- [ ] **Step 6: Verify build + smoke test**

Run: `cd players/zig && zig build && cd -`
Expected: clean build.

Smoke from moof: rebuild seed.vat, run, then in any moof file (or via stdin if REPL works) evaluate `(__vat-mode__)` — should return `'mutable-by-default`.

- [ ] **Step 7: Commit**

Run:
```bash
git add players/zig/src/intrinsics.zig players/zig/src/test_vat_mode.zig players/zig/build.zig
git commit -m "phase1/B: __vat-mode__ intrinsic + vat-mode tests"
```

### Task B3: Add auto-freeze hook for new form allocation

When `world.vat_mode == .frozen_default`, freshly allocated forms get their `frozen` bit set immediately on return from the alloc primitive. This is the simplest auto-freeze semantics: every alloc-expression result is born frozen.

The spec's let-mutable form (§4.1) lets you build mutably first then freeze — but in this simplest version, the alloc returns frozen, and let-mutable will need an alloc-with-mutable-window variant. We'll handle let-mutable in B5.

**Files:**
- Modify: `players/zig/src/intrinsics.zig` (alloc paths)
- Modify: `players/zig/src/world.zig` (form alloc helper if applicable)

- [ ] **Step 1: Locate the alloc entry point used by moof code**

Run: `grep -n "Object:new\|objNew\|allocForm\|form_alloc" players/zig/src/intrinsics.zig | head -10`
Expected: locates the intrinsic that moof's `[Proto new]` ultimately calls.

Also: `grep -n "pub fn alloc\|pub fn formAlloc\|pub fn allocRaw" players/zig/src/world.zig players/zig/src/heap.zig`
to find the alloc API hierarchy.

The goal is to identify the **single chokepoint** where a fresh Form gets created and returned to moof. That's where the vat-mode hook goes.

- [ ] **Step 2: Write the failing test**

Add to `players/zig/src/test_vat_mode.zig` (use whatever alloc API Step 1 surfaced; the example below uses `world.allocForm(.{})` as a placeholder):

```zig
test "alloc in frozen_default mode yields frozen form" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.vat_mode = .frozen_default;

    const id = try world.allocForm(.{});  // use actual API from Step 1
    try testing.expect(world.heap.get(id).frozen);
}

test "alloc in mutable_default mode yields mutable form" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.vat_mode = .mutable_default;

    const id = try world.allocForm(.{});
    try testing.expect(!world.heap.get(id).frozen);
}
```

- [ ] **Step 3: Run the test, confirm one fails**

Run: `cd players/zig && zig build test 2>&1 | grep -A 2 "frozen_default"`
Expected: the `alloc in frozen_default` test FAILS (form returns with `.frozen = false`).

- [ ] **Step 4: Implement the auto-freeze hook at the chokepoint**

In the alloc chokepoint from Step 1, add the vat-mode check **post-allocation, pre-return**:

```zig
    pub fn allocForm(self: *World, opts: AllocOpts) anyerror!FormId {
        const id = try self.heap.alloc(opts);  // existing call
        if (self.vat_mode == .frozen_default) {
            self.heap.getMut(id).frozen = true;
        }
        return id;
    }
```

If the chokepoint is in an intrinsic (e.g., `objNew`), put the hook there instead. Don't refactor the Heap API — just add the check at the layer that knows about World.

- [ ] **Step 5: Run the tests, confirm they pass**

Run: `cd players/zig && zig build test 2>&1 | tail -10`
Expected: both new tests pass; existing tests still green.

- [ ] **Step 6: Verify polyglot bootstrap still works**

Run the bootstrap cycle (build seed.vat, run via zig). Default vat_mode is mutable_default, so behavior is unchanged.
```bash
eval $(opam env --switch=wasm-mco)
dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -5
```
Expected: 22 stdlib files load, UnboundName: Console at the end (baseline).

- [ ] **Step 7: Commit**

Run:
```bash
git add players/zig/src/world.zig players/zig/src/test_vat_mode.zig
git commit -m "phase1/B: auto-freeze on alloc when vat_mode is frozen_default"
```

### Task B4: Verify cannot-freeze-live for vat-Forms / live faces

Per spec §4.5, `freeze` must raise `'cannot-freeze-live` on vat-Forms, mailboxes, cap-tokens, foreign-handles. Currently `:freezable?` returns true if not-already-frozen; we need to add the live-face check.

In V0/V1 we don't have vat-Forms yet — but we have foreign-handles via the wasm mco ABI. Make `:freezable?` and `freeze` check for these.

**Files:**
- Modify: `players/zig/src/intrinsics.zig` (the `objFreeze` and `objFreezable` impls)
- Modify: `players/zig/src/form.zig` (add liveness check helper)

- [ ] **Step 1: Read current objFreeze and objFreezable implementations**

Run: `grep -n "objFreeze\|objFrozen\|objFreezable" players/zig/src/intrinsics.zig`
Find their definitions. Read ~30 lines around each.

- [ ] **Step 2: Identify which Forms are "live" in V0**

Currently the only live thing is foreign-handles (per the mcos design — `moof_foreign_handle` Forms cannot cross vat boundaries per L7). In V0 there are no vat-Forms or mailbox-Forms yet (those come in V4). So the check needs to detect: **is this form a ForeignHandle?**

Check `players/zig/src/form.zig` for how ForeignHandle Forms are tagged. Likely via meta slot or special proto.

Run: `grep -n "ForeignHandle\|foreign_handle\|foreignHandle" players/zig/src/form.zig players/zig/src/intrinsics.zig`
Expected: locates how foreign-handles are represented.

- [ ] **Step 3: Write the failing test using the discovered API**

Based on Step 2's grep output, you now know the foreign-handle representation. Two possible forms:
- **Marker meta slot**: foreign-handles set a specific meta key (e.g., `__foreign-handle__`). Create a Form, set that meta key, check.
- **Dedicated alloc**: there's a `world.heap.allocForeignHandle(...)` or similar. Call it directly.

Pick the simpler path. Add to `players/zig/src/test_vat_mode.zig`:

```zig
test "freezable? returns false for foreign-handle forms" {
    var world = try World.init(testing.allocator);
    defer world.deinit();

    // Construct a foreign-handle form using whatever API Step 2 surfaced.
    // For example, if the marker is a meta slot:
    //   const fh_id = try world.heap.alloc(.{});
    //   const marker_sym = try world.intern("__foreign-handle__");
    //   try world.heap.getMut(fh_id).meta.put(testing.allocator, marker_sym, .nil);
    //
    // Then assert:
    try testing.expect(!world.isFreezable(fh_id));
}
```

Use the actual API names from Step 2. If a test helper exists (search for `test_helpers.zig` or similar), prefer it.

- [ ] **Step 4: Add a `World.isFreezable(id)` method**

In `players/zig/src/world.zig`, add:

```zig
    /// per spec §4.5, certain "live" forms cannot be frozen:
    /// vat-Forms, mailboxes, cap-tokens, foreign-handles.
    /// in V0 only foreign-handles exist as a category; the other
    /// categories land in V4.
    pub fn isFreezable(self: *const World, id: FormId) bool {
        const fm = self.heap.get(id);
        if (fm.frozen) return false;  // already frozen: not re-freezable
        if (fm.isForeignHandle()) return false;  // V0 live face
        return true;
    }
```

Add `isForeignHandle()` method to Form in `form.zig` if not present. The exact predicate depends on Step 2's findings. Common shape:

```zig
    pub fn isForeignHandle(self: *const Form, marker_sym_id: SymId) bool {
        // Foreign-handles are typically marked by a meta key. The
        // marker_sym_id should be interned at world startup and cached.
        return self.meta.contains(marker_sym_id);
    }
```

If the project already has a `Heap.isForeignHandle(id)` or similar, use it directly. Either way, `World.isFreezable` calls into this predicate.

- [ ] **Step 5: Wire objFreezable + objFreeze through World.isFreezable**

Modify `objFreezable` to call `world.isFreezable(id)`. Modify `objFreeze` to raise `'cannot-freeze-live` if not freezable:

```zig
fn objFreeze(world: *World, args: []const Value) anyerror!Value {
    const id = args[0].form;  // adjust to actual API
    if (!world.isFreezable(id)) {
        return raiseError(world, "cannot-freeze-live", "form is a live face");
    }
    world.heap.getMut(id).frozen = true;
    return args[0];
}

fn objFreezable(world: *World, args: []const Value) anyerror!Value {
    const id = args[0].form;
    return .{ .bool_ = world.isFreezable(id) };
}
```

(Adapt to actual zig idioms in this file.)

- [ ] **Step 6: Run the tests**

Run: `cd players/zig && zig build test 2>&1 | tail -10`
Expected: new test passes.

- [ ] **Step 7: Verify polyglot bootstrap still works**

```bash
eval $(opam env --switch=wasm-mco)
dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -5
```
Expected: baseline behavior.

- [ ] **Step 8: Commit**

Run:
```bash
git add players/zig/src/world.zig players/zig/src/form.zig players/zig/src/intrinsics.zig players/zig/src/test_vat_mode.zig
git commit -m "phase1/B: cannot-freeze-live for foreign-handle forms; World.isFreezable helper"
```

### Task B5: Implement let-mutable form

`let-mutable` lets you allocate a form, mutate it, then have it freeze automatically at scope exit. Useful in frozen-by-default vats for "build it then seal it" idioms.

Syntactic sketch:

```moof
(let-mutable form-binding
   body...)
```

evaluates: allocates a fresh mutable form (overriding vat-mode), binds it to `form-binding`, evaluates `body` (which can set slots), then freezes the form at scope exit.

**Files:**
- Create: `lib/early/12-vat-mode.moof`
- Modify: `lib/main.moof` (load 12-vat-mode.moof after defmethod)
- Modify: `players/zig/src/intrinsics.zig` (add `__alloc-mutable__` intrinsic that ignores vat-mode)

- [ ] **Step 1: Add `__alloc-mutable__` intrinsic**

In `players/zig/src/intrinsics.zig`, find the existing alloc intrinsic (probably `__alloc__` or `Object:new`). Add a sibling that explicitly does NOT apply vat-mode auto-freeze:

```zig
/// `__alloc-mutable__` — allocate a fresh form that is mutable regardless
/// of vat-mode. used by the let-mutable macro to bypass auto-freeze for
/// scoped construct-then-seal idioms.
fn allocMutableIntrinsic(world: *World, args: []const Value) anyerror!Value {
    // proto is args[0]; or nil for bare alloc
    const proto = if (args.len > 0) args[0] else Value.nil;
    const id = try world.heap.allocRaw(.{ .proto = proto });
    // explicitly DO NOT consult vat_mode; the form stays mutable.
    return .{ .form = id };
}
```

Register it:

```zig
    .{ "__alloc-mutable__", allocMutableIntrinsic },
```

- [ ] **Step 2: Add the `let-mutable` macro in moof**

Create `lib/early/12-vat-mode.moof`:

```moof
;; lib/early/12-vat-mode.moof
;;
;; vat-mode helpers. let-mutable lets you allocate a form mutably
;; (bypassing vat-mode auto-freeze), mutate it inside the body,
;; then auto-freeze it at scope exit.

;; usage:
;;
;;   (let-mutable (p [Point new])
;;     [p setX: 1]
;;     [p setY: 2]
;;     p)
;;   ; → returns p, which is now frozen
;;
;; the binding may be either a single (name expr) pair, allocating
;; whatever expr returns; the form returned by expr is what gets
;; frozen at exit. typical: expr is __alloc-mutable__ + initial setup.

(defmacro let-mutable (binding . body)
  (let ((name (car binding))
        (init-expr (car (cdr binding))))
    `(let ((,name ,init-expr))
       (let ((__lm-result__ (do ,@body)))
         [,name freeze]
         __lm-result__))))

;; convenience: allocate a fresh form (proto optional), bypassing vat-mode.
(def alloc-mutable
  (fn (proto)
    (__alloc-mutable__ proto)))
```

- [ ] **Step 3: Wire 12-vat-mode.moof into the boot order**

Modify `lib/main.moof`. Find the line that loads `lib/early/09-defmethod.moof` (or the last `early/` load before `if-macro`). After it, add:

```moof
[$transporter load: "early/12-vat-mode.moof"]
```

Place AFTER `defmethod` is loaded (since `let-mutable` may want it) but BEFORE stdlib loads (since stdlib code may want let-mutable).

(Note: the file is named `12-vat-mode.moof` to leave room for future early/* files. We're putting it at position 12 to come after `11-if-macro` and any other current early files.)

Run: `ls lib/early/` to see current numbering. Then place this file appropriately.

If you need to renumber other files to keep ordering, do that as a separate commit before this one to keep diffs clean.

- [ ] **Step 4: Hand-verify moof-side semantics**

A persistent moof-side test runner doesn't exist yet in phase 1; coverage at this layer is:
- Substrate-level correctness: zig tests in Task B3 + B4
- Cross-cutting durable test: conformance corpus in Task B6 (a scaffold; a future phase ships the runner)

For phase 1, hand-verify the macro in a REPL or one-shot eval:

```bash
echo '
(let-mutable (p [Object new])
  [p slot: (quote x) put: 1]
  [p slot: (quote y) put: 2])
' | MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof eval -
```

Expected: returns the form `p`; subsequent `[p frozen?]` returns `#true`.

If the existing stdlib has a `test` macro convention (check `grep -r "defmacro test" lib/`), you can optionally author `lib/stdlib/test-freezing.moof` to match. Not required for phase 1.

- [ ] **Step 5: Verify polyglot bootstrap still loads everything**

```bash
eval $(opam env --switch=wasm-mco)
dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -10
```
Expected: `12-vat-mode.moof` loads cleanly along with the others. The bootstrap may surface new errors if let-mutable has bugs — fix iteratively.

- [ ] **Step 6: Run the moof-side tests**

Test the let-mutable macro expansion manually if no test runner exists yet. Evaluate in a REPL or via direct moof eval:

```moof
[$transporter load: "stdlib/test-freezing.moof"]
```

Expected: tests pass (or report failures; fix until they pass).

- [ ] **Step 7: Commit**

Run:
```bash
git add lib/early/12-vat-mode.moof lib/main.moof players/zig/src/intrinsics.zig
git commit -m "phase1/B: let-mutable macro + __alloc-mutable__ intrinsic"
```

### Task B6: Conformance tests for freezing

Add image+message+expected triples per the conformance discipline (spec §13.10). These are the durable check that all players agree on freezing semantics.

**Files:**
- Create: `tests/conformance/freezing.json`
- Create: `tests/conformance/freezing/` (directory for the .vat fixtures + setup script)

- [ ] **Step 1: Look at existing conformance structure if any**

Run: `ls tests/conformance/ 2>/dev/null`
Expected: may not exist yet. If so, this task scaffolds it (and is OK to be minimal — full corpus comes in a later phase per the spec's §16 deferrals).

- [ ] **Step 2: Create the directory**

Run: `mkdir -p tests/conformance/freezing`

- [ ] **Step 3: Author the manifest**

Create `tests/conformance/freezing.json`:

```json
{
  "name": "freezing-v1",
  "description": "freeze locks state; mutation raises; let-mutable scoped",
  "triples": [
    {
      "name": "freeze-and-frozen-query",
      "image": "tests/conformance/freezing/empty.vat",
      "send": ["((let ((p [Point new])) [p freeze] [p frozen?]))"],
      "expect-value": "#true",
      "expect-stdout": ""
    },
    {
      "name": "mutation-after-freeze-raises",
      "image": "tests/conformance/freezing/empty.vat",
      "send": ["((let ((p [Point new])) [p freeze] (try [p slot: 'x put: 1] catch: |e| [e :kind])))"],
      "expect-value": "'frozen-form",
      "expect-stdout": ""
    },
    {
      "name": "let-mutable-result-is-frozen",
      "image": "tests/conformance/freezing/empty.vat",
      "send": ["((let-mutable (p [Point new]) p) frozen?)"],
      "expect-value": "#true",
      "expect-stdout": ""
    },
    {
      "name": "freezable-on-foreign-handle-returns-false",
      "image": "tests/conformance/freezing/with-foreign.vat",
      "send": ["[$hash freezable?]"],
      "expect-value": "#false",
      "expect-stdout": ""
    }
  ]
}
```

(Adjust syntax to match actual moof; the test runner will need a way to evaluate raw expressions against an image. If the runner doesn't exist yet, document this as a "next phase: build conformance runner" deferral.)

- [ ] **Step 4: Add a README in tests/conformance/**

Create `tests/conformance/README.md`:

```markdown
# Conformance Test Corpus

Per the design spec §13.10, every player must pass these triples.

## Structure

- `<name>.json` — manifest of (image, send, expect-value, expect-stdout) triples
- `<name>/` — directory with .vat fixtures referenced by the manifest

## Running

(Pending implementation; tracked as a future-phase concern. For now,
hand-verify each triple via REPL.)

The eventual runner: `moof conform <manifest.json>` per spec §1.3.
```

- [ ] **Step 5: Commit**

Run:
```bash
git add tests/conformance/
git commit -m "phase1/B: freezing conformance corpus scaffold + 4 triples"
```

### Task B7: Workstream B verification

- [ ] **Step 1: Run all zig tests**

Run: `cd players/zig && zig build test 2>&1 | tail -20`
Expected: all tests pass, including new freezing tests.

- [ ] **Step 2: Run full polyglot bootstrap**

```bash
eval $(opam env --switch=wasm-mco)
dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -10
```
Expected: 22+ stdlib files load (now includes `early/12-vat-mode.moof`); final error `UnboundName: Console` (still baseline; Console install is task #46 from NEXT_SESSION, out of scope for phase 1).

- [ ] **Step 3: Hand-verify freezing semantics**

Evaluate the conformance corpus triples from `tests/conformance/freezing.json` by hand. For each triple, construct the equivalent expression and run via:

```bash
echo '<expression>' | MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof eval -
```

Expected results match the `expect-value` field in the JSON.

Until a `moof conform <manifest.json>` runner exists (future phase per spec §1.3 / §16), this is the hand-check standard.

- [ ] **Step 4: Update NEXT_SESSION.md**

Modify `NEXT_SESSION.md` to add a line under "what just shipped" noting freezing primitive complete + vat-mode + let-mutable.

- [ ] **Step 5: Commit verification**

Run:
```bash
git add NEXT_SESSION.md
git commit -m "phase1/B: NEXT_SESSION — freezing primitive surface complete"
```

**Workstream B exit criteria met:** freezing primitive surface complete (freeze, frozen?, freezable?, cannot-freeze-live, vat-mode, auto-freeze, let-mutable). Polyglot bootstrap intact.

---

## Workstream C: Intrinsic Shrink First Pass

Goal: remove zig-side native implementations of methods that have canonical moof equivalents in `lib/stdlib/`. Target: `intrinsics.zig` 2506 → ~1750 LoC (30% shrink for first pass; full shrink to ~1500 LoC happens in later phases as more methods migrate).

**Strategy:** for each proto, survey what's in zig vs what's in moof. If moof has the canonical impl, remove the zig duplicate. If neither has it, either add to moof (preferred) or leave zig (rare).

**Bootstrap safety:** any native used by `lib/early/*.moof` before the corresponding stdlib file loads must stay in zig. Check load order in `lib/main.moof`.

Each task in this workstream handles one proto. They can be done in parallel by different agents.

### Task C1: Survey current intrinsics.zig native registrations

**Files:**
- Read: `players/zig/src/intrinsics.zig` (the registration table)

- [ ] **Step 1: Extract the list of all registered natives**

Run:
```bash
grep -E '^\s*\.\{ "[A-Z][^"]*:[a-z][^"]*"' players/zig/src/intrinsics.zig | sed -E 's/.*"([^"]+)".*/\1/' | sort | uniq > /tmp/native-methods.txt
wc -l /tmp/native-methods.txt
cat /tmp/native-methods.txt | head -40
```

This gives a list of all `Proto:method` pairs that are zig-side natives. Save the full list as a working document.

- [ ] **Step 2: Cross-reference with stdlib/**

For each method in `/tmp/native-methods.txt`, check if `lib/stdlib/<proto-lowercase>.moof` defines it via `defmethod`:

Run:
```bash
while IFS=: read proto method; do
  proto_lower=$(echo "$proto" | tr '[:upper:]' '[:lower:]')
  if grep -q "defmethod $proto ($method)" "lib/stdlib/$proto_lower.moof" 2>/dev/null; then
    echo "MOOF_HAS: $proto:$method"
  else
    echo "ZIG_ONLY: $proto:$method"
  fi
done < /tmp/native-methods.txt > /tmp/native-classification.txt
grep -c MOOF_HAS /tmp/native-classification.txt
grep -c ZIG_ONLY /tmp/native-classification.txt
```

(Note: this is a heuristic — the actual `defmethod` form is `(defmethod Proto (method ...)`. The grep might miss multi-line or differently-spaced forms. Treat as starting point, not final.)

- [ ] **Step 3: For each `MOOF_HAS` method, decide if zig version is shadow**

A "shadow" means the zig native is redundant — moof's version supersedes it. To verify a shadow:
1. The moof method does NOT call back into the zig native.
2. The moof method's behavior matches the zig native's spec.

Some natives are NOT shadows — they're foundational primitives that moof's defmethod actually wraps (`Cons:car`, `Integer:+`). Those stay.

Build a working axe-list at `/tmp/axe-list.txt` of native methods to remove. Reference §12.2 of the design spec for the recommended shrink table.

- [ ] **Step 4: Commit the working document**

```bash
mkdir -p docs/superpowers/working/
cp /tmp/native-classification.txt docs/superpowers/working/2026-05-16-native-classification.txt
git add docs/superpowers/working/
git commit -m "phase1/C: native intrinsic classification working doc"
```

### Task C2: Shrink Cons natives

Per §12.2: stays — `Cons:car`, `:cdr`, `:cons:`. Moves to moof — `:length`, `:reverse`, `:map:`, `:filter:`, `:reduce:`, `:forEach:`, `:take:`, `:drop:`, `:any?:`, `:all?:`, `:contains?:`, `:append:`, `:zip:`, `:scan:`, `:at:`, etc.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/cons.moof` has all the derived methods

- [ ] **Step 1: Verify lib/stdlib/cons.moof has the derived methods**

Run: `grep -n "defmethod Cons" lib/stdlib/cons.moof`
Expected: defmethod entries for length, reverse, map, filter, reduce, forEach, take, drop, any?, all?, contains?, append, zip, scan, at, =, !=, toString, inspect.

For any missing, **add them** to `lib/stdlib/cons.moof` before deleting the zig version. Follow the existing patterns in the file.

- [ ] **Step 2: Identify zig natives to remove for Cons**

In `players/zig/src/intrinsics.zig`, find the registration table entries:

```zig
.{ "Cons:length", consLength },
.{ "Cons:reverse", consReverse },
.{ "Cons:map:", consMap },
...
```

And their function bodies. Build a precise list.

- [ ] **Step 3: Remove a single zig native, test, then move to next (TDD-style)**

Pick one method, e.g., `Cons:length`. 

1. Note the registration line and the function body line range.
2. Delete the registration line.
3. Delete the function body.
4. Run `cd players/zig && zig build` — confirm clean build.
5. Run the polyglot bootstrap:
   ```bash
   eval $(opam env --switch=wasm-mco)
   dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
   MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -10
   ```
   Expected: still reaches the baseline `UnboundName: Console`. If it fails earlier, the moof version of the method has a bug or a different signature. Fix moof side, then retry.
6. Commit:
   ```bash
   git add players/zig/src/intrinsics.zig
   git commit -m "phase1/C: remove zig Cons:length; canonical is stdlib/cons.moof"
   ```

- [ ] **Step 4: Repeat step 3 for each derivable Cons method**

For each of: `Cons:reverse`, `Cons:map:`, `Cons:filter:`, `Cons:reduce:`, `Cons:forEach:`, `Cons:take:`, `Cons:drop:`, `Cons:any?:`, `Cons:all?:`, `Cons:contains?:`, `Cons:append:`, `Cons:zip:`, `Cons:scan:`, `Cons:at:`, `Cons:=`, `Cons:!=`, `Cons:toString`, `Cons:inspect`, `Cons:empty?`, `Cons:null?`.

One commit per method (or batch by 2-3 if they share dependencies). Each commit must leave the bootstrap green.

- [ ] **Step 5: Verify Cons natives remaining**

Run: `grep -E '"Cons:[a-z]' players/zig/src/intrinsics.zig | sort`
Expected: only `Cons:car`, `Cons:cdr`, `Cons:cons:` remain (plus any other truly-primitive ones).

- [ ] **Step 6: Track LoC change**

Run: `wc -l players/zig/src/intrinsics.zig`
Note: should be smaller than the baseline 2506.

### Task C3: Shrink Integer natives

Per §12.2: stays — `:+`, `:-`, `:*`, `:/`, `:=`, `:<`, `:>`, `:asFloat`. Moves to moof — `:abs`, `:even?`, `:odd?`, `:between?:`, `:max:`, `:min:`, `:<=`, `:>=`, `:!=`.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/integer.moof` has the derived methods

- [ ] **Step 1: Verify lib/stdlib/integer.moof has derived methods**

Run: `grep -n "defmethod Integer" lib/stdlib/integer.moof`
Expected: abs, even?, odd?, between?, max:, min:, <=, >=, !=. Add any missing.

- [ ] **Step 2: For each redundant Integer native, remove + verify**

Follow the same one-at-a-time pattern from Task C2 Step 3. For each of: `Integer:abs`, `Integer:even?`, `Integer:odd?`, `Integer:between?:`, `Integer:max:`, `Integer:min:`, `Integer:<=`, `Integer:>=`, `Integer:!=`.

Commit per method.

- [ ] **Step 3: Verify**

Run: `grep -E '"Integer:[a-z]' players/zig/src/intrinsics.zig | sort`
Expected: only `Integer:+`, `:-`, `:*`, `:/`, `:=`, `:<`, `:>`, `:asFloat` remain.

### Task C4: Shrink Float natives

Per §12.2: stays — arithmetic primitives. Moves to moof — `:abs`, `:max:`, `:min:`, `:asInteger`, `:round`, `:floor`, `:ceil`, `:<=`, `:>=`, `:!=`.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/float.moof`

- [ ] **Step 1: Verify lib/stdlib/float.moof has derived methods**

Run: `grep -n "defmethod Float" lib/stdlib/float.moof`
Expected: abs, max:, min:, asInteger, round, floor, ceil, <=, >=, !=. Add any missing.

(`:floor` and `:ceil` may need to stay native if they wrap actual zig stdlib floor/ceil; check whether the moof version reimplements via integer truncation or calls a zig primitive.)

- [ ] **Step 2: Remove redundant natives**

Per the C2 pattern. Commit per method.

- [ ] **Step 3: Verify**

Run: `grep -E '"Float:[a-z]' players/zig/src/intrinsics.zig | sort`
Expected: arithmetic primitives only.

### Task C5: Shrink String natives

Per §12.2: stays — `:byteAt:`, `:length`, `:byteEq`, `:concat`, `:at:`, `:slice:length:`, `:as:`, `:toList`, `:contains?:`. Moves to moof — `:trim`, `:indexOf:`, `:replace:with:`, `:split:`, `:lines`, `:toString`, `:inspect`, `:asTable`, `:startsWith?:`, `:endsWith?:`, `:reverse`.

**Bootstrap dependency caveat**: `:endsWith?:` is needed by `Symbol:endsWithColon?` which is needed by `__decode-header` inside the `defmethod` macro. If `:endsWith?:` lives in stdlib/string.moof (which loads after early/), defmethod can't use it. Check `lib/early/03-string-essentials.moof` — that file holds the early-required string subset.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/early/03-string-essentials.moof` + `lib/stdlib/string.moof`

- [ ] **Step 1: Audit which String methods are "early required"**

Run: `cat lib/early/03-string-essentials.moof`
This tells you what string ops the early bootstrap needs before stdlib loads.

Methods that appear here MUST stay in zig OR be derivable from other early-required primitives. Methods that DO NOT appear here are safe to fully move to stdlib.

- [ ] **Step 2: For each derivable String native, follow the C2 pattern**

For each of: `String:trim`, `String:indexOf:`, `String:replace:with:`, `String:split:`, `String:lines`, `String:toString`, `String:inspect`, `String:asTable`, `String:reverse`.

For `:startsWith?:` and `:endsWith?:` — check that the early bootstrap doesn't need them at the zig level; if early/ uses these via moof methods (i.e., moof code itself calls `[sym endsWithColon?]` which then calls `[str endsWith?: ":"]`), then we need to ensure they're defined in early/ before defmethod runs.

If unclear, leave `:startsWith?:` and `:endsWith?:` as zig natives for now and revisit in the next phase.

Commit per method.

- [ ] **Step 3: Verify**

Run: `grep -E '"String:[a-z]' players/zig/src/intrinsics.zig | sort`
Expected: only the truly-primitive subset remains.

### Task C6: Shrink Char natives

Per §12.2: stays — `:codepoint`, `:<`. Moves to moof — `:inspect`, `:toString`, `:digit?`, `:letter?`, `:uppercase`, `:lowercase`.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/char.moof`

- [ ] **Step 1: Verify lib/stdlib/char.moof has derived methods**

Run: `grep -n "defmethod Char" lib/stdlib/char.moof`
Expected: inspect, toString, digit?, letter?, uppercase, lowercase. Add any missing.

- [ ] **Step 2: Remove redundant natives per C2 pattern**

Commit per method.

- [ ] **Step 3: Verify**

Run: `grep -E '"Char:[a-z]' players/zig/src/intrinsics.zig | sort`
Expected: `Char:codepoint` and `Char:<` only.

### Task C7: Shrink Object derived methods

Per §12.2: stays — `:proto`, `:slots`, `:handlers`, `:meta`, `:freeze`, `:identity`, `:source`. Moves to moof — `:=`, `:!=`, `:satisfies?`, `:is-fallback`, `:toString-name-fallback`, `:initialize`.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/object.moof`

- [ ] **Step 1: Verify lib/stdlib/object.moof has derived methods**

Run: `grep -n "defmethod Object" lib/stdlib/object.moof`
Expected: =, !=, satisfies?, is-fallback, toString-name-fallback, initialize. Add any missing.

- [ ] **Step 2: Remove redundant natives per C2 pattern**

Commit per method.

- [ ] **Step 3: Verify**

Run: `grep -E '"Object:[a-z]' players/zig/src/intrinsics.zig | sort`
Expected: proto, slots, handlers, meta, freeze, frozen?, freezable?, identity, source, new, dnu, handlerAt: remain.

### Task C8: Shrink Method derived methods

Per §12.2: stays — `:body`, `:source`, `:params`, `:consts`, `:bytecodes`, `:ics`, `:call`. Moves to moof — `:toString`, `:inspect`.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/method.moof`

- [ ] **Step 1: Verify lib/stdlib/method.moof has toString + inspect**

Run: `grep -n "defmethod Method" lib/stdlib/method.moof`
Expected: toString, inspect.

- [ ] **Step 2: Remove redundant natives per C2 pattern**

Commit per method.

- [ ] **Step 3: Verify**

Run: `grep -E '"Method:[a-z]' players/zig/src/intrinsics.zig | sort`
Expected: only primitives remain.

### Task C9: Shrink Table derived methods

Per §12.2: stays — `:new`, `:length`, `:at:`, `:at:put:`, `:push:`, `:pop`, `:keys`, `:values`, `:remove:`, `:containsKey?:`. Moves to moof — `:size`, `:empty?`, `:nonEmpty?`, `:asString`, `:toString`, `:inspect`, `:=`, `:as:`, `:forEach:`.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/table.moof`

- [ ] **Step 1: Verify lib/stdlib/table.moof has derived methods**

Run: `grep -n "defmethod Table" lib/stdlib/table.moof`
Expected: size, empty?, nonEmpty?, asString, toString, inspect, =, as:, forEach:. Add any missing.

- [ ] **Step 2: Remove redundant natives per C2 pattern**

Commit per method.

- [ ] **Step 3: Verify**

### Task C10: Shrink nil derived methods

The transporter spec called out that `nil` has shadow installations in both rust (the original) and bootstrap.moof. With the zig port, check whether zig has the redundant `nil` natives. If so, remove.

**Files:**
- Modify: `players/zig/src/intrinsics.zig`
- Verify: `lib/stdlib/nil.moof`

- [ ] **Step 1: Find nil registrations in zig**

Run: `grep -E '"nil:[a-z]' players/zig/src/intrinsics.zig | sort`

- [ ] **Step 2: For each redundant nil method, follow C2 pattern**

Commit per method.

### Task C11: Final shrink verification

- [ ] **Step 1: Measure intrinsics.zig LoC**

Run: `wc -l players/zig/src/intrinsics.zig`
Expected: significantly smaller than the baseline 2506. Target: ~1750 or below (~30% reduction). If we're not there yet, look at the classification working doc (Task C1 Step 4) for additional candidates.

- [ ] **Step 2: Run zig tests**

Run: `cd players/zig && zig build test 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 3: Run full polyglot bootstrap**

```bash
eval $(opam env --switch=wasm-mco)
dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -10
```
Expected: 23 stdlib files load (now including `12-vat-mode.moof`); `UnboundName: Console` at the end (baseline).

- [ ] **Step 4: Update NEXT_SESSION.md**

Modify `NEXT_SESSION.md` to note intrinsic shrink first pass complete and report new intrinsics.zig LoC.

- [ ] **Step 5: Commit verification**

Run:
```bash
git add NEXT_SESSION.md
git commit -m "phase1/C: NEXT_SESSION — intrinsic shrink first pass complete"
```

**Workstream C exit criteria met:** intrinsics.zig shrunk by ~30%; bootstrap still passes baseline; canonical method implementations are in stdlib/.

---

## Final Integration Task: Phase 1 verification

### Task Z1: End-to-end phase 1 verification

- [ ] **Step 1: Clean rebuild from scratch**

Run:
```bash
cd players/zig && zig build -Doptimize=ReleaseSafe && cd -
cargo build --release -p moof --bin moof-rs --quiet
eval $(opam env --switch=wasm-mco) && dune build --root seed/ocaml
```
Expected: all builds clean.

- [ ] **Step 2: Run all test suites**

Run:
```bash
cd players/zig && zig build test 2>&1 | tail -10 && cd -
cargo test --workspace --quiet 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 3: Polyglot bootstrap baseline**

```bash
eval $(opam env --switch=wasm-mco)
dune exec --root seed/ocaml bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof run /tmp/seed.vat 2>&1 | tail -10
```
Expected: 23 stdlib files load (early/* count increased by 1 for 12-vat-mode.moof; stdlib/* count unchanged). Final error `UnboundName: Console` as before.

- [ ] **Step 4: Measure shrink metrics**

Run:
```bash
wc -l players/zig/src/intrinsics.zig
wc -l players/zig/src/*.zig
```
Note: total LoC should be lower than the 10702 baseline; intrinsics.zig specifically should be in the ~1500-1800 range.

- [ ] **Step 5: Final NEXT_SESSION.md update**

Modify `NEXT_SESSION.md` to reflect phase 1 completion:
- Directory rename: done
- Freezing primitive surface: done
- Intrinsic shrink first pass: done with measurements
- Next: phase 2 (vat carve)

- [ ] **Step 6: Final commit**

Run:
```bash
git add NEXT_SESSION.md
git commit -m "phase 1 complete: rename + freezing + intrinsic shrink first pass"
```

- [ ] **Step 7: Tag the milestone**

Run:
```bash
git tag -a phase-1-complete -m "phase 1: substrate housekeeping + freezing + intrinsic shrink"
```

**Phase 1 exit criteria:**
- [x] `crates/` → `players/`, `seed/`, `tools/` rename complete
- [x] Freezing primitive surface complete (freeze, frozen?, freezable?, cannot-freeze-live, vat-mode, auto-freeze, let-mutable)
- [x] `intrinsics.zig` shrunk by ~30%
- [x] Polyglot bootstrap still reaches the baseline `UnboundName: Console`
- [x] All zig tests pass; all rust workspace tests pass

**Ready for phase 2: vat carve (`World` → `World + Vat`).**
