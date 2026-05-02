# Transporter + radical std-lib modularization — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the two `include_str!`-bound moof source files with a Self-style `$transporter` capability that loads a tree of small thematic files at runtime; in the process move ~1.2k LoC of derivable methods from `intrinsics.rs` into Moof, and fix the REPL bug that suppresses `nil` in displayed output.

**Architecture:** Three layered changes. (1) a primordial `$transporter` cap implemented in a new `transporter.rs`, plus a `$compiler` cap to replace the `use_moof_compiler` flag-flip. (2) the rust seed compiler now loads exactly one file (`lib/main.moof`); main.moof orchestrates the rest of bootstrap via `[$transporter load: …]`. (3) the moof compiler internally calls primitive free functions (`__list-length` etc.) instead of method-sending on `Cons`, which removes the circularity that previously forced `Cons:length` to be a Rust method.

**Tech Stack:** Rust 2021, the existing `crates/substrate` workspace, `cargo test --workspace` for verification.

**Spec:** [`docs/superpowers/specs/2026-05-02-transporter-and-stdlib-modularization-design.md`](../specs/2026-05-02-transporter-and-stdlib-modularization-design.md)

---

## File Structure

**Files to create:**

```
crates/substrate/src/transporter.rs          ;; new — the $transporter cap

lib/main.moof                                ;; new — single rust entry point
lib/compiler/00-helpers.moof                 ;; split out of lib/compiler.moof
lib/compiler/01-dispatch.moof
lib/compiler/02-special.moof
lib/compiler/03-control.moof
lib/early/00-cons.moof                       ;; split out of lib/bootstrap.moof
lib/early/01-nil.moof
lib/early/02-bool.moof
lib/early/03-string-essentials.moof
lib/early/04-symbol.moof
lib/early/05-quasiquote.moof
lib/early/06-control-macros.moof
lib/early/07-modules.moof
lib/early/08-match-defn-proto.moof
lib/early/09-defmethod.moof
lib/stdlib/object.moof                       ;; split out of lib/bootstrap.moof
lib/stdlib/bool.moof
lib/stdlib/nil.moof
lib/stdlib/cons.moof
lib/stdlib/integer.moof
lib/stdlib/float.moof
lib/stdlib/string.moof
lib/stdlib/char.moof
lib/stdlib/table.moof
lib/stdlib/method.moof
crates/substrate/tests/transporter.rs        ;; new — transporter unit tests
crates/substrate/tests/repl_nil.rs           ;; new — REPL nil-display test
```

**Files to modify:**

```
crates/substrate/src/lib.rs                  ;; new_world() rewrite
crates/substrate/src/main.rs                 ;; remove nil gates
crates/substrate/src/intrinsics.rs           ;; add free-fn primitives,
                                             ;; install $compiler cap, then
                                             ;; remove migrated methods
crates/substrate/src/world.rs                ;; transporter_root field
crates/substrate/src/compiler.rs             ;; if needed for use_moof_compiler
                                             ;; flag access pattern
```

**Files to delete (at end):**

```
lib/bootstrap.moof                           ;; fully split into early/ + stdlib/
lib/compiler.moof                            ;; fully split into compiler/
```

---

## Phase 1 — Foundation

Phase 1 ships a working build with the Transporter cap, REPL fix, and the `$compiler` cap. The existing `lib/bootstrap.moof` and `lib/compiler.moof` files stay as single files; only the **mechanism** for loading them changes (from `include_str!` to runtime `fs::read_to_string` via `$transporter`). After Phase 1 every test should still pass and the REPL prints `nil`.

### Task 1: REPL nil-display test (failing)

**Files:**
- Create: `crates/substrate/tests/repl_nil.rs`

- [ ] **Step 1: Write the failing test**

```rust
//! REPL must display `nil` when the user evaluates `nil`. The previous
//! "lisp convention" of suppressing nil from REPL output was an
//! ergonomic bug — moof has its own conventions and `(defmethod nil
//! (inspect) "nil")` is canonical.

use std::process::Command;

#[test]
fn repl_displays_nil_on_nil_input() {
    // one-shot mode is a sufficient proxy for the REPL print path —
    // both share `print_via_out_inspect` (after Task 2) and the same
    // `is_nil()` gate. (full pty-driving would need expectrl; one-shot
    // is enough for this regression.)
    let out = Command::new(env!("CARGO_BIN_EXE_moof"))
        .arg("nil")
        .output()
        .expect("failed to spawn moof");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout, "nil\n",
        "expected `nil\\n` on stdout for `moof nil`, got: {:?}",
        stdout
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --test repl_nil -- --nocapture
```

Expected: FAIL — `assertion 'left == right' failed: left: ""`. The current binary returns success but emits nothing for nil.

### Task 2: Remove the REPL nil gates

**Files:**
- Modify: `crates/substrate/src/main.rs` (lines 35-58, 60-106)

- [ ] **Step 1: Replace `eval_one_shot` with the un-gated version**

Replace the body of `fn eval_one_shot(source: &str) -> ExitCode { … }` with:

```rust
fn eval_one_shot(source: &str) -> ExitCode {
    let mut world = moof::new_world();
    match moof::eval(&mut world, source) {
        Ok(value) => {
            // print every value via :inspect — including nil.
            // moof's `:inspect` on nil returns `"nil"`. the REPL is
            // the user's view into the running image; suppressing
            // results would lie about what the expression yielded.
            match print_via_out_inspect(&mut world, value) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("moof: {}", e.message);
                    ExitCode::from(70)
                }
            }
        }
        Err(err) => {
            let _ = print_via_err(&mut world, &format!("error: {}", err.message));
            ExitCode::from(1)
        }
    }
}
```

Note: this also unifies the print path between one-shot and REPL — both now use `print_via_out_inspect`. The previous one-shot used `print_via_out` (which routes through `[$out say:]` → `:toString`, NOT `:inspect`). After this change, `moof '"hello"'` prints `"hello"` (re-readable) instead of `hello` (display-friendly). That's the right call: `moof '<expr>'` is most often used for debugging and pipe-to-other-tools, where re-readable output is more useful.

- [ ] **Step 2: Replace the REPL loop's nil-gate**

In `fn repl()`, change:

```rust
        match moof::eval(&mut world, trimmed) {
            Ok(value) => {
                if !value.is_nil() {
                    let _ = print_via_out_inspect(&mut world, value);
                }
            }
```

to:

```rust
        match moof::eval(&mut world, trimmed) {
            Ok(value) => {
                let _ = print_via_out_inspect(&mut world, value);
            }
```

- [ ] **Step 3: Remove the now-unused `print_via_out` function**

`print_via_out` was only used by `eval_one_shot`; both call sites now use `print_via_out_inspect`. Delete the function (lines 137-147 in the current file).

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo test --test repl_nil -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Confirm no other tests regressed**

```bash
cargo test --workspace
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/main.rs crates/substrate/tests/repl_nil.rs
git commit -m "REPL displays nil — drop the lisp-convention gate"
```

### Task 3: Scaffold `transporter.rs` with the Transporter proto and stub

**Files:**
- Create: `crates/substrate/src/transporter.rs`
- Modify: `crates/substrate/src/lib.rs` (add `pub mod transporter;`)
- Modify: `crates/substrate/src/world.rs` (add `transporter_root: Option<PathBuf>` field)

- [ ] **Step 1: Add `transporter_root` field to `World`**

In `crates/substrate/src/world.rs`, find the `pub struct World` definition. Add:

```rust
    /// Resolved root for [$transporter load: ...] calls. Populated at
    /// `new_world()` via `transporter::resolve_lib_root`. None means
    /// the transporter cap will raise 'tx-no-root on every call —
    /// used by `new_world_bare` for tests that don't need bootstrap.
    pub transporter_root: Option<std::path::PathBuf>,
```

In `World::new()` (around line 265), add to the field initializers:

```rust
            transporter_root: None,
```

- [ ] **Step 2: Create `transporter.rs` with the resolve helper**

```rust
//! `$transporter` — Self-style file ↔ image bridge.
//!
//! files are *transport*. the canonical home of moof code is the
//! image (the live runtime objects). $transporter ferries source
//! text into the image (`:load:`, `:loadAll:`) and — eventually —
//! ferries in-image objects back out as files (`:dump:toFile:`,
//! reserved for a future session).
//!
//! the cap is a primordial — installed by intrinsics.rs at world
//! creation, bound to `$transporter` in the global env. it is the
//! only path through which moof code reads files. the substrate
//! itself uses it directly (in `new_world()`) to load `lib/main.moof`.
//!
//! see `docs/superpowers/specs/2026-05-02-transporter-and-stdlib-
//! modularization-design.md` for the full design.

use crate::value::Value;
use crate::world::{RaiseError, World};
use std::path::{Path, PathBuf};

/// resolve the lib root, in order:
///   1. `MOOF_LIB` env var (if set and is a directory)
///   2. `<dir of std::env::current_exe()>/../lib` (if a directory)
///   3. `./lib` relative to cwd (if a directory)
///   4. None — caller raises `'tx-no-root`.
pub fn resolve_lib_root() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("MOOF_LIB") {
        let p = PathBuf::from(env_path);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent().and_then(|p| p.parent()) {
            let candidate = parent.join("lib");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    let cwd_lib = PathBuf::from("./lib");
    if cwd_lib.is_dir() {
        return Some(cwd_lib);
    }
    None
}

/// Build the `$transporter` proto-Form. The proto-Form is the cap
/// itself — there is exactly one Transporter, so the proto and the
/// "instance" are identical, like primordial $out / $err.
pub fn install(w: &mut World) {
    use crate::form::Form;

    let proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    // :load: — minimum surface this session.
    w.install_native(proto, "load:", |w, _self, args| {
        let path_val = args.first().copied().unwrap_or(Value::Nil);
        let rel = w
            .string_text(path_val)
            .map(|s| s.to_string())
            .ok_or_else(|| {
                RaiseError::new(w.intern("tx-bad-arg"), ":load: expects a String path")
            })?;
        load_relative(w, &rel)
    });

    // :loadAll: — walks a Cons of Strings, calls :load: on each.
    w.install_native(proto, "loadAll:", |w, _self, args| {
        let list = args.first().copied().unwrap_or(Value::Nil);
        let paths = w.list_to_vec(list).map_err(|_| {
            RaiseError::new(w.intern("tx-bad-arg"), ":loadAll: expects a Cons")
        })?;
        let mut last = Value::Nil;
        for (i, v) in paths.iter().enumerate() {
            let rel = w.string_text(*v).map(|s| s.to_string()).ok_or_else(|| {
                RaiseError::new(
                    w.intern("tx-bad-arg"),
                    format!(":loadAll: element {} is not a String", i),
                )
            })?;
            last = load_relative(w, &rel)?;
        }
        Ok(last)
    });

    // :root — diagnostic; returns the resolved root as a String.
    w.install_native(proto, "root", |w, _self, _args| {
        match &w.transporter_root {
            Some(p) => {
                let s = p.display().to_string();
                Ok(w.make_string(&s))
            }
            None => Err(RaiseError::new(
                w.intern("tx-no-root"),
                "transporter has no root configured",
            )),
        }
    });

    // :dump:toFile: — RESERVED. The Transporter's name promises a
    // round-trip; the second half lands in a future session that
    // walks a Form's :handlers / :slots / :meta and reconstructs
    // source text using the per-method :source slot.
    w.install_native(proto, "dump:toFile:", |w, _self, _args| {
        Err(RaiseError::new(
            w.intern("tx-unimplemented"),
            ":dump:toFile: is reserved — the file→image direction lands in a future session",
        ))
    });

    // bind the proto-Form as the `$transporter` global. that's the
    // cap itself; receiving methods sends to it.
    let global = w.global_env;
    let dollar = w.intern("$transporter");
    w.env_bind(global, dollar, Value::Form(proto));
}

/// shared implementation for `:load:` and `:loadAll:`. resolves rel
/// against the world's transporter_root, reads the file, and
/// `eval_program`'s its contents.
fn load_relative(w: &mut World, rel: &str) -> Result<Value, RaiseError> {
    if Path::new(rel).is_absolute() || rel.contains("..") {
        return Err(RaiseError::new(
            w.intern("tx-bad-path"),
            format!(
                ":load: refuses absolute or `..`-traversing paths: {:?}",
                rel
            ),
        ));
    }
    let root = w.transporter_root.clone().ok_or_else(|| {
        RaiseError::new(
            w.intern("tx-no-root"),
            "transporter has no root configured",
        )
    })?;
    let abs = root.join(rel);
    if !abs.exists() {
        return Err(RaiseError::new(
            w.intern("tx-not-found"),
            format!("not found: {} (resolved as {})", rel, abs.display()),
        ));
    }
    let source = std::fs::read_to_string(&abs).map_err(|e| {
        RaiseError::new(
            w.intern("tx-read-error"),
            format!("{}: {}", abs.display(), e),
        )
    })?;
    crate::eval_program(w, &source).map_err(|e| {
        // wrap inner errors with the file path for diagnosis. preserve
        // the inner symbol so callers can still pattern-match by kind.
        RaiseError::new(
            e.kind,
            format!("{}: {}", abs.display(), e.message),
        )
    })
}
```

- [ ] **Step 3: Wire `transporter` into `lib.rs`**

In `crates/substrate/src/lib.rs`, near the other `pub mod` lines, add:

```rust
pub mod transporter;
```

- [ ] **Step 4: Verify the crate still builds**

```bash
cargo build --workspace
```

Expected: success. (Not yet wired into `new_world()` or intrinsics — just the module exists and compiles.)

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/transporter.rs crates/substrate/src/lib.rs crates/substrate/src/world.rs
git commit -m "transporter — scaffold module with load/loadAll/root + dump stub"
```

### Task 4: Wire `$transporter` into intrinsics + `new_world` to populate root

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs` (extend `pub fn install`)
- Modify: `crates/substrate/src/lib.rs` (`new_world` to set `transporter_root`)

- [ ] **Step 1: Add transporter install to intrinsics**

In `crates/substrate/src/intrinsics.rs`, find `pub fn install(w: &mut World) {`. Add at the top of the function body, before `install_call_on_method`:

```rust
    crate::transporter::install(w);
```

- [ ] **Step 2: Set the transporter root in `new_world`**

In `crates/substrate/src/lib.rs`, replace `pub fn new_world()` with:

```rust
pub fn new_world() -> world::World {
    let mut w = world::World::new();
    w.transporter_root = transporter::resolve_lib_root();
    intrinsics::install(&mut w);
    // step 2 — compile compiler.moof via the rust seed compiler.
    if let Err(e) = eval_program(&mut w, COMPILER_SOURCE) {
        panic!("compiler.moof failed to load: {}", e.message);
    }
    // step 3 — flip. all subsequent compiles go through moof.
    w.use_moof_compiler = true;
    // step 4 — compile bootstrap.moof via the moof compiler.
    if let Err(e) = eval_program(&mut w, BOOTSTRAP_SOURCE) {
        panic!("bootstrap.moof failed to load: {}", e.message);
    }
    w
}
```

(BOOTSTRAP_SOURCE / COMPILER_SOURCE include_str!s stay for now — they go away in Task 8.)

- [ ] **Step 3: Verify build + tests**

```bash
cargo test --workspace
```

Expected: still 334 passing. Transporter cap exists but isn't yet driving the boot.

- [ ] **Step 4: Commit**

```bash
git add crates/substrate/src/intrinsics.rs crates/substrate/src/lib.rs
git commit -m "transporter — install \$transporter cap at boot, populate root"
```

### Task 5: Transporter unit tests

**Files:**
- Create: `crates/substrate/tests/transporter.rs`

- [ ] **Step 1: Write the unit tests**

```rust
//! `$transporter` capability tests. exercises the four error symbols
//! plus the happy path. tests assume `MOOF_LIB` is set in the test
//! harness via env (handled by setting it in each test).

use moof::value::Value;
use std::path::PathBuf;

fn lib_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib")
}

fn fresh_world() -> moof::world::World {
    std::env::set_var("MOOF_LIB", lib_root());
    moof::new_world()
}

#[test]
fn root_returns_a_string() {
    let mut w = fresh_world();
    let v = moof::eval(&mut w, "[$transporter root]").unwrap();
    let s = w.string_text(v).expect(":root must return a String").to_string();
    assert!(
        s.ends_with("/lib"),
        "expected root to end with /lib, got {:?}",
        s
    );
}

#[test]
fn load_known_file_succeeds() {
    let mut w = fresh_world();
    // bootstrap.moof exists at this point (file split happens later).
    let v = moof::eval(&mut w, "[$transporter load: \"bootstrap.moof\"]");
    assert!(v.is_ok(), "load: should succeed for an existing file: {:?}", v);
}

#[test]
fn load_missing_file_raises_not_found() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: \"nope-does-not-exist.moof\"]")
        .expect_err("load: should fail for missing file");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-not-found", "wrong error kind: {}", kind_str);
}

#[test]
fn load_absolute_path_is_rejected() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: \"/etc/passwd\"]")
        .expect_err("load: should reject absolute paths");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-path");
}

#[test]
fn load_traversal_path_is_rejected() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: \"../../../etc/passwd\"]")
        .expect_err("load: should reject ..-traversing paths");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-path");
}

#[test]
fn load_non_string_arg_raises_bad_arg() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter load: 42]")
        .expect_err("load: should reject non-String arg");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-arg");
}

#[test]
fn load_all_walks_a_list() {
    let mut w = fresh_world();
    // empty list — returns nil, no error.
    let v = moof::eval(&mut w, "[$transporter loadAll: '()]").unwrap();
    assert!(matches!(v, Value::Nil));
}

#[test]
fn load_all_non_string_element_raises() {
    let mut w = fresh_world();
    let err = moof::eval(&mut w, "[$transporter loadAll: '(\"bootstrap.moof\" 42)]")
        .expect_err(":loadAll: should reject non-String element");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-bad-arg");
}

#[test]
fn dump_to_file_is_unimplemented() {
    let mut w = fresh_world();
    let err = moof::eval(
        &mut w,
        "[$transporter dump: 1 toFile: \"x\"]",
    )
    .expect_err(":dump:toFile: stub should raise");
    let kind_str = w.resolve(err.kind).to_string();
    assert_eq!(kind_str, "tx-unimplemented");
}
```

- [ ] **Step 2: Run the transporter tests**

```bash
cargo test --test transporter
```

Expected: all PASS. (Note: tests share global env, so `MOOF_LIB` is set to the same value across them — fine.)

- [ ] **Step 3: Commit**

```bash
git add crates/substrate/tests/transporter.rs
git commit -m "transporter tests — load, loadAll, root, error symbols"
```

### Task 6: Add `$compiler` cap with `:useMoof` and `:useSeed`

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs` (add `install_compiler_cap`)

- [ ] **Step 1: Write the failing test inline (in transporter.rs unit test file is fine)**

Append to `crates/substrate/tests/transporter.rs`:

```rust
#[test]
fn compiler_use_moof_flips_flag() {
    // build a bare world (no bootstrap), flip via moof, observe.
    let mut w = moof::new_world_bare();
    assert!(!w.use_moof_compiler, "bare world starts with seed compiler");
    // intrinsics::install needs to run for $compiler to exist.
    moof::intrinsics::install(&mut w);
    moof::eval(&mut w, "[$compiler useMoof]").unwrap();
    assert!(w.use_moof_compiler, "useMoof should flip the flag");
    moof::eval(&mut w, "[$compiler useSeed]").unwrap();
    assert!(!w.use_moof_compiler, "useSeed should flip back");
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --test transporter compiler_use_moof_flips_flag
```

Expected: FAIL — `$compiler` is unbound.

- [ ] **Step 3: Add `install_compiler_cap` to intrinsics.rs**

After `install_console_proto_and_caps` in `intrinsics.rs`, add:

```rust
fn install_compiler_cap(w: &mut World) {
    // `$compiler` — primordial cap that controls which compiler is
    // canonical. one proto-Form, two methods. flipping useMoof
    // routes every subsequent compile through the Compiler singleton
    // defined in lib/compiler/. useSeed flips back (mostly a
    // diagnostics knob).
    let proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    w.install_native(proto, "useMoof", |w, _self, _args| {
        w.use_moof_compiler = true;
        Ok(Value::Nil)
    });
    w.install_native(proto, "useSeed", |w, _self, _args| {
        w.use_moof_compiler = false;
        Ok(Value::Nil)
    });

    let global = w.global_env;
    let dollar = w.intern("$compiler");
    w.env_bind(global, dollar, Value::Form(proto));
}
```

And register it from `pub fn install(w: &mut World)` (just below the existing `install_compiler_primitives(w);`):

```rust
    install_compiler_cap(w);
```

- [ ] **Step 4: Make `intrinsics::install` accessible from tests**

Confirm `pub mod intrinsics;` is already in `lib.rs` (it is, around line 26). No change needed.

- [ ] **Step 5: Run tests**

```bash
cargo test --test transporter
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/intrinsics.rs crates/substrate/tests/transporter.rs
git commit -m "\$compiler cap — useMoof / useSeed replaces direct flag access"
```

### Task 7: Create `lib/main.moof` orchestration

**Files:**
- Create: `lib/main.moof`

- [ ] **Step 1: Write `lib/main.moof` (transitional content)**

This version of main.moof loads the existing single bootstrap.moof and compiler.moof files. After Phase 3 the file list grows; for now we just exercise the new mechanism.

```moof
;; lib/main.moof — the only file the rust seed compiles directly.
;;
;; everything else flows through `$transporter`. files are transport,
;; the image is canonical.
;;
;; Transitional during Phase 1: this file just chains the two existing
;; bootstrap files. Phase 2-3 split them into compiler/, early/, and
;; stdlib/ subtrees and grow the load list accordingly.

;; phase 1: the moof compiler — compiled by the rust seed.
[$transporter load: "compiler.moof"]

;; flip — every subsequent compile routes through the Compiler singleton.
[$compiler useMoof]

;; phase 2-3: bootstrap stdlib + macros — compiled by the moof compiler.
[$transporter load: "bootstrap.moof"]
```

- [ ] **Step 2: Verify the file is valid moof syntax**

```bash
ls -la lib/main.moof
```

Expected: file exists, ~10 lines.

- [ ] **Step 3: Commit**

```bash
git add lib/main.moof
git commit -m "lib/main.moof — Phase 1 orchestration via \$transporter"
```

### Task 8: Switch `new_world` to load `lib/main.moof` via the transporter

**Files:**
- Modify: `crates/substrate/src/lib.rs` (delete `BOOTSTRAP_SOURCE`, `COMPILER_SOURCE`; rewrite `new_world`)

- [ ] **Step 1: Replace `new_world` and remove the `include_str!` constants**

In `crates/substrate/src/lib.rs`, replace the entire file body **below** `pub mod world;` (i.e., from line 37 onward) with:

```rust
/// build a fresh world with the phase-A intrinsics, the $transporter
/// cap populated, and `lib/main.moof` loaded — which itself orchestrates
/// loading the rest of the std lib.
///
/// the boot dance, per `docs/process/self-hosted-compiler.md`:
///
/// 1. rust intrinsics (heap, OS i/o, arithmetic primitives, the
///    chunk-construction api, the `$transporter` and `$compiler` caps).
/// 2. resolve the lib root via `transporter::resolve_lib_root` and
///    bind it on `World.transporter_root`.
/// 3. read `<root>/main.moof`. main.moof drives:
///    a. `[$transporter load: "compiler.moof"]` — seed-compiled.
///    b. `[$compiler useMoof]` — flag flip.
///    c. `[$transporter load: "bootstrap.moof"]` — moof-compiled.
///
/// failures at any step are substrate bugs (lib/ ships with the
/// substrate), so we panic.
pub fn new_world() -> world::World {
    let mut w = world::World::new();
    w.transporter_root = transporter::resolve_lib_root();
    intrinsics::install(&mut w);

    let root = w.transporter_root.clone().unwrap_or_else(|| {
        panic!(
            "could not resolve moof lib root. tried MOOF_LIB env, \
             <exe>/../lib, and ./lib. set MOOF_LIB to point at the \
             moof lib directory."
        )
    });
    let main_path = root.join("main.moof");
    let main_source = std::fs::read_to_string(&main_path).unwrap_or_else(|e| {
        panic!("failed to read {}: {}", main_path.display(), e)
    });
    if let Err(e) = eval_program(&mut w, &main_source) {
        panic!("lib/main.moof failed to load: {}", e.message);
    }
    w
}

/// build a fresh world *without* loading any moof code. used by
/// tests that exercise raw substrate behavior without the moof-side
/// stdlib.
pub fn new_world_bare() -> world::World {
    let mut w = world::World::new();
    w.transporter_root = transporter::resolve_lib_root();
    intrinsics::install(&mut w);
    w
}

/// evaluate a single expression in the world's global env.
pub fn eval(w: &mut world::World, source: &str) -> Result<value::Value, world::RaiseError> {
    let form = w
        .read(source)
        .map_err(|e| world::RaiseError::from_reader(&mut w.syms, e))?;
    let chunk = compiler::compile(w, form)?;
    w.run_top(chunk)
}

/// evaluate every top-level form in `source`, returning the value
/// of the last. used to load multi-form scripts (incl. lib/main.moof
/// and the files it transitively loads).
pub fn eval_program(
    w: &mut world::World,
    source: &str,
) -> Result<value::Value, world::RaiseError> {
    let forms = w
        .read_all(source)
        .map_err(|e| world::RaiseError::from_reader(&mut w.syms, e))?;
    let mut last = value::Value::Nil;
    for form in forms {
        let chunk = compiler::compile(w, form)?;
        last = w.run_top(chunk)?;
    }
    Ok(last)
}
```

The `BOOTSTRAP_SOURCE` and `COMPILER_SOURCE` `pub const` declarations are gone.

- [ ] **Step 2: Find and update any other code that referenced `BOOTSTRAP_SOURCE` / `COMPILER_SOURCE`**

```bash
grep -rn 'BOOTSTRAP_SOURCE\|COMPILER_SOURCE' crates/
```

Expected output: probably some test files. For each, if it imports the constant, replace with reading from disk (paralleling the new `new_world` logic) or — better — switch the test to call `moof::new_world()` and exercise the loaded behavior. List of likely call sites: `tests/moof_compiler.rs`, `tests/doc_alignment.rs`, `tests/phase_a_forcing_function.rs`. Inspect each match and apply the minimal fix.

- [ ] **Step 3: Run tests**

```bash
cargo test --workspace
```

Expected: all green. The build now reads `lib/main.moof` at runtime; from the user's perspective nothing changed.

- [ ] **Step 4: Commit**

```bash
git add crates/substrate/src/lib.rs crates/substrate/tests/
git commit -m "new_world boots via lib/main.moof — \`include_str!\`s gone"
```

### Phase 1 checkpoint

At the end of Phase 1: `cargo test --workspace` passes, `moof nil` prints `"nil\n"`, the `$transporter` cap exists with all four methods (`load:`, `loadAll:`, `root`, `dump:toFile:`-stub), and `$compiler` exposes `useMoof` / `useSeed`. The `include_str!` constants are gone; `lib/main.moof` is the entry. Existing `lib/bootstrap.moof` and `lib/compiler.moof` are still single files. Stop here, run the tests, take a breath before Phase 2.

---

## Phase 2 — Free-function primitives + `compiler.moof` split

The radicality unlock. The moof compiler currently does method sends like `[args length]` on Cons and Symbol values. Those sends require `Cons:length` / `Symbol:endsWithColon?` to exist as Moof methods. But those are exactly the methods we want to install **using** the moof compiler. Circular.

Fix: replace those sends inside the moof compiler with primitive **free functions** installed by Rust intrinsics — `__list-length`, `__list-empty?`, etc. After this swap, the moof compiler's internal plumbing depends only on free-function primitives + method-sends to its own Compiler singleton. `Cons` and `Symbol` methods can then live entirely in moof.

### Task 9: Add free-function primitives to intrinsics

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs` (extend `install_globals`)

- [ ] **Step 1: Add primitive free functions**

Inside `install_globals`, after the existing primitives but still inside the function body, add:

```rust
    // ─────────────────────────────────────────────────────────────
    // moof-compiler primitives — free-function escape hatches that
    // the moof compiler uses internally so it doesn't need
    // `Cons:length` / `Symbol:endsWithColon?` etc. to exist as
    // *methods* before the file that defines them can be compiled.
    //
    // these are NOT user-facing. they live as `__`-prefixed
    // globals; convention is "rust-side compiler plumbing".
    // ─────────────────────────────────────────────────────────────

    install_global(w, "__list-length", |world, _self, args| {
        if args.len() != 1 {
            return Err(raise(world, "arity", "(__list-length list)"));
        }
        let n = world
            .list_len(args[0])
            .map_err(|_| type_error(world, "__list-length: not a list"))?;
        Ok(Value::Int(n as i64))
    });

    install_global(w, "__list-empty?", |_world, _self, args| {
        // empty? is true for nil only — Cons cells are never empty.
        Ok(Value::Bool(matches!(args.first(), Some(Value::Nil))))
    });

    install_global(w, "__list-car", |world, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        match v {
            Value::Nil => Ok(Value::Nil),
            Value::Form(id) => {
                let car_sym = world.car_sym;
                Ok(world.heap.get(id).slot(car_sym))
            }
            _ => Err(type_error(world, "__list-car: not a Cons or nil")),
        }
    });

    install_global(w, "__list-cdr", |world, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        match v {
            Value::Nil => Ok(Value::Nil),
            Value::Form(id) => {
                let cdr_sym = world.cdr_sym;
                Ok(world.heap.get(id).slot(cdr_sym))
            }
            _ => Err(type_error(world, "__list-cdr: not a Cons or nil")),
        }
    });

    install_global(w, "__list-reverse", |world, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let elems = world
            .list_to_vec(v)
            .map_err(|_| type_error(world, "__list-reverse: not a list"))?;
        let rev: Vec<Value> = elems.into_iter().rev().collect();
        Ok(world.make_list(&rev))
    });

    install_global(w, "__symbol-ends-with-colon?", |world, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let sym = v.as_sym().ok_or_else(|| {
            type_error(world, "__symbol-ends-with-colon?: not a Symbol")
        })?;
        let text = world.resolve(sym);
        Ok(Value::Bool(text.ends_with(':')))
    });
```

- [ ] **Step 2: Add unit tests for the primitives**

Append to `crates/substrate/tests/transporter.rs` (or create `crates/substrate/tests/compiler_primitives.rs` if you prefer file isolation):

```rust
#[test]
fn list_length_primitive() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "(__list-length '(1 2 3))").unwrap(),
        moof::value::Value::Int(3)
    );
    assert_eq!(
        moof::eval(&mut w, "(__list-length nil)").unwrap(),
        moof::value::Value::Int(0)
    );
}

#[test]
fn list_empty_primitive() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "(__list-empty? nil)").unwrap(),
        moof::value::Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "(__list-empty? '(1))").unwrap(),
        moof::value::Value::Bool(false)
    );
}

#[test]
fn list_car_cdr_primitives() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "(__list-car '(1 2 3))").unwrap(),
        moof::value::Value::Int(1)
    );
    let v = moof::eval(&mut w, "(__list-cdr '(1 2 3))").unwrap();
    assert_eq!(w.list_len(v).unwrap(), 2);
}

#[test]
fn list_reverse_primitive() {
    let mut w = moof::new_world();
    let v = moof::eval(&mut w, "(__list-reverse '(1 2 3))").unwrap();
    let elems = w.list_to_vec(v).unwrap();
    assert_eq!(
        elems,
        vec![
            moof::value::Value::Int(3),
            moof::value::Value::Int(2),
            moof::value::Value::Int(1),
        ]
    );
}

#[test]
fn symbol_ends_with_colon_primitive() {
    let mut w = moof::new_world();
    assert_eq!(
        moof::eval(&mut w, "(__symbol-ends-with-colon? 'foo:)").unwrap(),
        moof::value::Value::Bool(true)
    );
    assert_eq!(
        moof::eval(&mut w, "(__symbol-ends-with-colon? 'foo)").unwrap(),
        moof::value::Value::Bool(false)
    );
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS, including the new primitive tests.

- [ ] **Step 4: Commit**

```bash
git add crates/substrate/src/intrinsics.rs crates/substrate/tests/
git commit -m "moof-compiler primitives — __list-length/__list-empty?/etc."
```

### Task 10: Update `compiler.moof` to use free-function primitives

**Files:**
- Modify: `lib/compiler.moof`

- [ ] **Step 1: Audit the moof compiler for method sends that need replacement**

```bash
grep -n '\[args length\]\|\[forms car\]\|\[forms cdr\]\|\[rest car\]\|\[rest cdr\]\|\[rest is nil\]\|endsWithColon?\|\[bindings car\]\|\[bindings cdr\]\|\[values length\]' lib/compiler.moof
```

Each match is a candidate for replacement. The rule: any send to `:length`, `:car`, `:cdr`, `:empty?`, `:reverse`, `:is nil` (where the receiver is a list/symbol that we want to be moof-side post-migration) gets replaced with the free-function form.

NOT every method send needs replacement — only those whose receivers will (post-migration) require the moof compiler to be running in order to dispatch. Practically: `[chunk emit: …]`, `[Compiler …]`, `[Method …]`, `[Opcode …]`, `[Chunk …]` stay as method sends because Chunk/Opcode/Method primitives are kept in Rust.

- [ ] **Step 2: Replace identified sends with primitives**

Apply these replacements throughout `lib/compiler.moof`:

| Before | After |
|---|---|
| `[args length]` | `(__list-length args)` |
| `[values length]` | `(__list-length values)` |
| `[args is nil]` | `(__list-empty? args)` |
| `[forms car]` | `(__list-car forms)` |
| `[forms cdr]` | `(__list-cdr forms)` |
| `[rest car]` | `(__list-car rest)` |
| `[rest cdr]` | `(__list-cdr rest)` |
| `[bindings car]` | `(__list-car bindings)` |
| `[bindings cdr]` | `(__list-cdr bindings)` |
| `[clauses car]` | `(__list-car clauses)` |
| `[clauses cdr]` | `(__list-cdr clauses)` |
| `[form car]` (where `form` is a moof source list) | `(__list-car form)` |
| `[form cdr]` | `(__list-cdr form)` |
| `[head is 'fn]` etc — STAYS (this is `Object:is`, kept primitive) | unchanged |
| `[head endsWithColon?]` — N/A (the moof compiler doesn't actually call this; it's only `__decode-header` in bootstrap that does) | unchanged |

Use `Edit` (or `sed`) to apply each replacement. Be careful: `[c is nil]` should stay (since `is` lives on Object as a primitive that survives the migration). Only the listed sends to `:length`/`:car`/`:cdr`/`:empty?` change.

- [ ] **Step 3: Run tests after substitution**

```bash
cargo test --workspace
```

Expected: all PASS. The moof compiler now has a clean dependency on rust primitives only.

- [ ] **Step 4: Commit**

```bash
git add lib/compiler.moof
git commit -m "compiler.moof — internal plumbing uses free-fn primitives"
```

### Task 11: Split `compiler.moof` into `lib/compiler/00-03-*.moof`

**Files:**
- Create: `lib/compiler/00-helpers.moof`, `01-dispatch.moof`, `02-special.moof`, `03-control.moof`
- Modify: `lib/main.moof`
- Delete: `lib/compiler.moof`

- [ ] **Step 1: Map content of `compiler.moof` to the four output files**

Open `lib/compiler.moof`. Use the spec's section breakdown:

- `00-helpers.moof`: file header comment, `(def Compiler [Object new])`, plus the helpers `cons?:`, `symbol?:`, `bool?:`, `macroAt:`, `wrapBody:` (lines roughly 1-71 in the existing file).
- `01-dispatch.moof`: `compileTop:`, `compileForm:chunk:tail:`, `compileConst:chunk:`, `compileLoadName:chunk:`, `compileList:chunk:tail:`, `compileSpecialOrCall:chunk:tail:` (lines ~73-157).
- `02-special.moof`: `compileQuote:chunk:`, `compileSet:chunk:`, `compileDef:chunk:` and helpers (`multiClauseDef?:`, `allFnForms?:`), `compileSend:chunk:tail:`, `compileArgs:chunk:` (lines ~159-275).
- `03-control.moof`: `compileIf:chunk:tail:`, `compileFn:chunk:`, `compileDo:chunk:tail:`, `compileDoLoop:chunk:tail:`, `compileLet:chunk:tail:`, `letParams:`, `letValues:`, `compileDefmacro:chunk:`, `compileCall:chunk:tail:`, plus the `__mc-compile-and-run` test helper at the bottom (lines ~277-end).

Each output file should start with its own brief header comment naming the file's responsibility. The original file-level comment (the long one at the top of `compiler.moof`) goes into `00-helpers.moof` since that's the first file loaded.

- [ ] **Step 2: Create the four new files with copied content**

For each split, read the section from `compiler.moof` and write it to the corresponding new file. Don't reformat or reword — straight copy.

```bash
# you can do this manually with Edit/Write, or use Read+Write
# carefully. either way, verify line counts add up to the original
# afterwards.
wc -l lib/compiler.moof lib/compiler/*.moof
```

Expected: sum of split files ≈ `lib/compiler.moof` + ~10 lines for new headers.

- [ ] **Step 3: Update `lib/main.moof`**

Replace the `[$transporter load: "compiler.moof"]` line with:

```moof
;; phase 1: the moof compiler — compiled by the rust seed.
[$transporter load: "compiler/00-helpers.moof"]
[$transporter load: "compiler/01-dispatch.moof"]
[$transporter load: "compiler/02-special.moof"]
[$transporter load: "compiler/03-control.moof"]
```

- [ ] **Step 4: Delete `lib/compiler.moof`**

```bash
rm lib/compiler.moof
```

- [ ] **Step 5: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS. The compiler is identical, just split across 4 files now.

- [ ] **Step 6: Commit**

```bash
git add lib/compiler/ lib/main.moof
git rm lib/compiler.moof
git commit -m "compiler.moof split into compiler/{00..03}-*.moof"
```

### Phase 2 checkpoint

`cargo test --workspace` green. The moof compiler is split into four files; its internal plumbing uses free-function primitives. No method has migrated yet — `intrinsics.rs` is unchanged in size. Stop, breathe.

---

## Phase 3 — `bootstrap.moof` split into `early/` + `stdlib/`

This is the big mechanical phase. We carve the existing 1211-line `bootstrap.moof` into 16 small files, in dependency order, **preserving every method's current behavior**. The methods that are currently in Rust intrinsics stay there for now — we don't migrate yet, just move moof code around.

The pattern for every task here is identical:
1. Read the section from `bootstrap.moof`.
2. Write it to the target `early/Nx-foo.moof` or `stdlib/proto.moof` with a brief header.
3. Add the corresponding `[$transporter load: …]` line to `lib/main.moof` (in dependency order).
4. Remove the section from `bootstrap.moof`.
5. Run `cargo test --workspace`.
6. Commit.

Tasks 12-21 cover `early/`. Tasks 22-31 cover `stdlib/`. After Task 31, `bootstrap.moof` is empty; Task 32 deletes it.

### Task 12: Extract `early/00-cons.moof`

**Files:**
- Create: `lib/early/00-cons.moof`
- Modify: `lib/bootstrap.moof`, `lib/main.moof`

- [ ] **Step 1: Identify the Cons-related setHandler! / def lines in bootstrap.moof**

There are no setHandler!-driven Cons methods in bootstrap.moof currently — Cons methods all use `defmethod` (defined later) or live in Rust. **For Phase 3, this file starts EMPTY** (just a header comment), and we'll populate it during Phase 4 when we start migrating Cons methods out of Rust.

- [ ] **Step 2: Create the placeholder file**

```moof
;; lib/early/00-cons.moof
;;
;; setHandler!-driven Cons primitive methods. Phase 3 (file split)
;; leaves this as a placeholder; Phase 4 (rust→moof migration) fills
;; it in with `:length`, `:reverse`, `:empty?`, `:null?`, `:toString`,
;; `:inspect`, plus any other Cons primitives that move out of
;; intrinsics.rs.
;;
;; load order rationale: Cons methods are needed by `__qq-walk-elems`
;; (uses `[forms car]` and `[forms cdr]` — free-function primitives,
;; not method sends, so technically not blocking), and by the entire
;; rest of bootstrap. installing them first keeps the load-order story
;; clean for readers.

;; (intentionally empty in Phase 3.)
```

- [ ] **Step 3: Add the `[$transporter load: …]` line to main.moof**

Insert into `lib/main.moof` between `[$compiler useMoof]` and `[$transporter load: "bootstrap.moof"]`:

```moof
;; early stage — primitives + macros. setHandler!-driven, predates defmethod.
[$transporter load: "early/00-cons.moof"]
```

- [ ] **Step 4: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS. (Empty file loads as a no-op.)

- [ ] **Step 5: Commit**

```bash
git add lib/early/00-cons.moof lib/main.moof
git commit -m "early/00-cons.moof — empty placeholder for Phase 4 Cons migration"
```

### Task 13: Extract `early/01-nil.moof`

**Files:**
- Create: `lib/early/01-nil.moof`
- Modify: `lib/main.moof`

- [ ] **Step 1: Create the placeholder file**

```moof
;; lib/early/01-nil.moof
;;
;; setHandler!-driven nil primitive methods. Phase 3 placeholder;
;; Phase 4 fills in `:length, :car, :cdr, :empty?, :reverse, :append:,
;; :proto, :toString, :inspect, :cons:` — the canonical nil methods
;; that supersede both the rust shadow installs in intrinsics.rs AND
;; the bootstrap.moof defmethod shadows further down.

;; (intentionally empty in Phase 3.)
```

- [ ] **Step 2: Add load to main.moof**

Insert after the cons load:

```moof
[$transporter load: "early/01-nil.moof"]
```

- [ ] **Step 3: Test + commit**

```bash
cargo test --workspace
git add lib/early/01-nil.moof lib/main.moof
git commit -m "early/01-nil.moof — empty placeholder for Phase 4 nil migration"
```

### Task 14: Extract `early/02-bool.moof` (placeholder)

**Files:**
- Create: `lib/early/02-bool.moof`
- Modify: `lib/main.moof`

- [ ] **Step 1: Create the placeholder file**

```moof
;; lib/early/02-bool.moof
;;
;; Phase 3 placeholder. Bool methods (:not, :and:, :or:, :toString)
;; currently live as `defmethod`s in bootstrap.moof and migrate to
;; `stdlib/bool.moof` (NOT here) — this file stays empty unless a
;; Phase 4 dependency forces a migration to setHandler! form.

;; (intentionally empty.)
```

- [ ] **Step 2-3: Add load + test + commit**

```moof
[$transporter load: "early/02-bool.moof"]
```

```bash
cargo test --workspace
git add lib/early/02-bool.moof lib/main.moof
git commit -m "early/02-bool.moof — empty placeholder"
```

### Task 15: Extract `early/03-string-essentials.moof` (placeholder)

**Files:**
- Create: `lib/early/03-string-essentials.moof`
- Modify: `lib/main.moof`

- [ ] **Step 1: Create**

```moof
;; lib/early/03-string-essentials.moof
;;
;; Phase 3 placeholder. The small set of String methods that
;; `__decode-header` transitively needs: :endsWith?:, :+, :=, :length,
;; :contains?:, :all?:, :toString. Phase 4 ports these from
;; intrinsics.rs into setHandler! form here.

;; (intentionally empty in Phase 3.)
```

- [ ] **Step 2-3: Add load + test + commit**

```moof
[$transporter load: "early/03-string-essentials.moof"]
```

```bash
cargo test --workspace
git add lib/early/03-string-essentials.moof lib/main.moof
git commit -m "early/03-string-essentials.moof — empty placeholder"
```

### Task 16: Extract `early/04-symbol.moof`

**Files:**
- Create: `lib/early/04-symbol.moof`
- Modify: `lib/bootstrap.moof`, `lib/main.moof`

- [ ] **Step 1: Identify the section in bootstrap.moof**

```bash
grep -n 'Symbol\|operator-chars\|endsWithColon' lib/bootstrap.moof
```

Move lines 585-597 (the `setHandler! Symbol 'endsWithColon?` block, the `__operator-chars` def, and the `setHandler! Symbol 'operatorOnly?` block) into the new file.

- [ ] **Step 2: Create the file**

```moof
;; lib/early/04-symbol.moof
;;
;; Symbol primitives needed by __decode-header (and thus by the
;; defmethod macro). these stay as setHandler! because defmethod
;; doesn't exist yet at this point in the boot.

(setHandler! Symbol 'endsWithColon?
  (fn () [[self toString] endsWith?: ":"]))

;; the operator-character set — used by header-decoding to spot
;; binary operator headers like `(+ other)`. defined as a String
;; we test membership against; cheaper to extend than a chain of
;; nested `if`s.
(def __operator-chars "+-*/<>=!?|&~^%")

(setHandler! Symbol 'operatorOnly?
  (fn ()
    [[self toString] all?:
      (fn (c) [__operator-chars contains?: [c toString]])]))
```

- [ ] **Step 3: Remove the same lines from bootstrap.moof**

Delete lines 585-597 from `lib/bootstrap.moof`.

- [ ] **Step 4: Add load to main.moof, in order (after string-essentials)**

```moof
[$transporter load: "early/04-symbol.moof"]
```

- [ ] **Step 5: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS — the Symbol methods are installed at the same point in boot; `Symbol:operatorOnly?` still works because `String:contains?:` is a Rust method (kept in Phase 3).

- [ ] **Step 6: Commit**

```bash
git add lib/early/04-symbol.moof lib/main.moof lib/bootstrap.moof
git commit -m "early/04-symbol.moof — extract Symbol primitives from bootstrap"
```

### Task 17: Extract `early/05-quasiquote.moof`

**Files:**
- Create: `lib/early/05-quasiquote.moof`
- Modify: `lib/bootstrap.moof`, `lib/main.moof`

- [ ] **Step 1: Identify the section**

Lines 65-119 in current `bootstrap.moof`: the `__qq-list?`, `__qq-marker?`, `__qq-walk-elems`, `__qq-expand` defs plus the `(defmacro quasiquote (args) …)`. Plus the surrounding header comment about quasiquote.

- [ ] **Step 2: Create the new file** with the section content + a brief header. Header text:

```moof
;; lib/early/05-quasiquote.moof
;;
;; quasiquote — pure source-to-source expansion. lives here (not in
;; the rust seed compiler) so user code can read it, reason about it,
;; and override it. defined first because every macro below uses it.

;; [original __qq-* def block + (defmacro quasiquote …) here]
```

(Copy the original lines 65-119 verbatim into the body.)

- [ ] **Step 3: Remove from bootstrap.moof**

Delete lines 65-119 from `lib/bootstrap.moof`.

- [ ] **Step 4: Add load to main.moof**

```moof
[$transporter load: "early/05-quasiquote.moof"]
```

- [ ] **Step 5: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add lib/early/05-quasiquote.moof lib/main.moof lib/bootstrap.moof
git commit -m "early/05-quasiquote.moof — extract quasiquote macro"
```

### Task 18: Extract `early/06-control-macros.moof`

**Files:**
- Create: `lib/early/06-control-macros.moof`
- Modify: `lib/bootstrap.moof`, `lib/main.moof`

- [ ] **Step 1: Identify the section**

The `__cascade__`, `__table__`, `__obj__`, `when`, `unless`, `let*`, `let-rec` macros — currently lines 146-284 (approximately) in `bootstrap.moof`. Use `grep -n '^(defmacro' lib/bootstrap.moof` to confirm exact lines.

- [ ] **Step 2: Create the file**

```moof
;; lib/early/06-control-macros.moof
;;
;; control-flow + binding sugar — each was once a hardcoded special
;; form in the rust compiler; they live here as moof macros so the
;; "everything is a Form" claim isn't a lie about what the user wrote.

;; [content from bootstrap.moof]
```

- [ ] **Step 3-6: Same pattern — remove from bootstrap, add load, test, commit**

```bash
cargo test --workspace
git add lib/early/06-control-macros.moof lib/main.moof lib/bootstrap.moof
git commit -m "early/06-control-macros.moof — when, unless, let*, let-rec, cascade, table, obj"
```

### Task 19: Extract `early/07-modules.moof`

**Files:**
- Create: `lib/early/07-modules.moof`
- Modify: `lib/bootstrap.moof`, `lib/main.moof`

- [ ] **Step 1: Identify**

The `(def DefProto …)`, `(def Defn …)`, `(def Match …)` block plus all their `setHandler!` installations. Lines 286-541-ish; use `grep` to confirm.

- [ ] **Step 2: Create** with this header:

```moof
;; lib/early/07-modules.moof
;;
;; module-singleton helpers — DefProto, Defn, Match. these host the
;; multi-arg helper methods that Match/Defn/DefProto macros need at
;; expansion time. setHandler! shape because defmethod isn't here yet.
```

- [ ] **Step 3-6: Remove, load, test, commit**

```bash
cargo test --workspace
git add lib/early/07-modules.moof lib/main.moof lib/bootstrap.moof
git commit -m "early/07-modules.moof — DefProto, Defn, Match singletons"
```

### Task 20: Extract `early/08-match-defn-proto.moof`

**Files:**
- Create: `lib/early/08-match-defn-proto.moof`
- Modify: `lib/bootstrap.moof`, `lib/main.moof`

- [ ] **Step 1: Identify**

The `(defmacro match …)`, `(defmacro defn …)`, `(defmacro defproto …)` macro defs. Lines roughly 461 (`defmacro match`), 553 (`defmacro defn`), and 1132 (`defmacro defproto`) — note `defproto` lives near the **end** of bootstrap.moof. All three should move into this single `early/08` file (they're peers semantically).

- [ ] **Step 2: Create**

```moof
;; lib/early/08-match-defn-proto.moof
;;
;; the three macros that turn the Match / Defn / DefProto modules
;; from helper data into language surfaces. each desugars to setHandler!
;; or an ordinary def using the helpers in early/07-modules.moof.
```

- [ ] **Step 3-6: Remove from bootstrap, load, test, commit**

```bash
cargo test --workspace
git add lib/early/08-match-defn-proto.moof lib/main.moof lib/bootstrap.moof
git commit -m "early/08-match-defn-proto.moof — match, defn, defproto macros"
```

### Task 21: Extract `early/09-defmethod.moof`

**Files:**
- Create: `lib/early/09-defmethod.moof`
- Modify: `lib/bootstrap.moof`, `lib/main.moof`

- [ ] **Step 1: Identify**

`__decode-header`, `__decode-keyword`, and `(defmacro defmethod (args) …)` — lines 599-637. (`__operator-chars` already moved to `early/04`; verify it's not duplicated here.)

- [ ] **Step 2: Create**

```moof
;; lib/early/09-defmethod.moof
;;
;; defmethod — the canonical method-installation macro. lowers to
;; setHandler!. once this loads, every subsequent file in stdlib/
;; can use the prettier defmethod surface.
;;
;; depends on early/04-symbol.moof (Symbol :endsWithColon?,
;; :operatorOnly?, __operator-chars).
```

- [ ] **Step 3-6: Remove, load, test, commit**

```bash
cargo test --workspace
git add lib/early/09-defmethod.moof lib/main.moof lib/bootstrap.moof
git commit -m "early/09-defmethod.moof — defmethod macro and __decode-header"
```

### Task 22-31: Extract `stdlib/` files (one per proto)

After Phase 3 Tasks 12-21, `bootstrap.moof` should have only `(defmethod …)` blocks left (plus a few `(def …)` at the top of bootstrap for Object protos like `__satisfies-walk`, `Macros`, etc.). Now we split per proto.

**Pattern (identical for each task):**

1. `grep -n '^(defmethod <ProtoName>' lib/bootstrap.moof` — find lines.
2. Create `lib/stdlib/<protoname>.moof` with a header comment + a copy of those defmethod blocks.
3. Delete those blocks from `bootstrap.moof`.
4. Add `[$transporter load: "stdlib/<protoname>.moof"]` to `lib/main.moof` after all the `early/` loads.
5. `cargo test --workspace`.
6. Commit.

The 10 protos in dependency-friendly order (Object first since others may inherit; nil before Cons since Cons:reverse uses nil:append:):

### Task 22: `stdlib/object.moof`

- [ ] Extract every `(defmethod Object …)` and supporting `def __satisfies-walk` from bootstrap.moof.
- [ ] Header comment:

```moof
;; lib/stdlib/object.moof
;;
;; Object — derived defaults that flow through every proto chain.
;; primitives like :proto, :slots, :handlers, :meta, :identity,
;; :source, :dnu, :new live in rust intrinsics; this file derives
;; the rest.
```

- [ ] Test + commit:

```bash
cargo test --workspace
git add lib/stdlib/object.moof lib/main.moof lib/bootstrap.moof
git commit -m "stdlib/object.moof — Object derivations split from bootstrap"
```

### Task 23: `stdlib/bool.moof`

- [ ] Extract `(defmethod Bool …)` blocks. Header:

```moof
;; lib/stdlib/bool.moof
;;
;; Bool — convenience derived from the conditional primitive `if`.
;; rust no longer carries Bool methods; all of them are derived here.
```

- [ ] Test + commit per pattern. Commit message:

```
stdlib/bool.moof — Bool methods split from bootstrap
```

### Task 24: `stdlib/nil.moof`

- [ ] Extract `(defmethod nil …)` blocks. Header:

```moof
;; lib/stdlib/nil.moof
;;
;; nil — the singleton empty-list and absence value. Phase 3 of the
;; cleanup leaves both these defmethods AND the rust intrinsics as
;; shadow installs. Phase 4 removes the rust copies; the moof
;; defmethods become canonical.
```

- [ ] Test + commit. Message: `stdlib/nil.moof — nil methods split from bootstrap`.

### Task 25: `stdlib/cons.moof`

- [ ] Extract `(defmethod Cons …)` blocks (length, reverse, map, filter, reduce, etc.). Header:

```moof
;; lib/stdlib/cons.moof
;;
;; Cons — the singly-linked list. primitives :car, :cdr, :cons: live
;; in rust; everything else (length, reverse, map, filter, reduce,
;; forEach, take, drop, any?, all?, contains?, sum, product, count,
;; zip, scan, at, =, !=, toString, inspect, append) is derived here.
```

- [ ] Note: some `(defmethod nil (…))` definitions are interleaved with the Cons ones in bootstrap.moof for nil-as-empty-list cases (e.g. `(defmethod nil (zip: ys) nil)` at line 787). Move those into `stdlib/nil.moof` (Task 24's file) rather than `stdlib/cons.moof`. Use `grep -n '^(defmethod nil' lib/bootstrap.moof` to make sure all nil cases land in the right file.

- [ ] Test + commit. Message: `stdlib/cons.moof — Cons methods split from bootstrap`.

### Task 26: `stdlib/integer.moof`

- [ ] Extract `(defmethod Integer …)` blocks. Header:

```moof
;; lib/stdlib/integer.moof
;;
;; Integer — derived methods. arithmetic primitives (+ - * / = < >),
;; :asFloat live in rust; this file derives :abs, :even?, :odd?,
;; :between?, :max:, :min:, comparison shortcuts (!=, <=, >=), etc.
```

- [ ] Test + commit. Message: `stdlib/integer.moof — Integer methods split from bootstrap`.

### Task 27: `stdlib/float.moof`

- [ ] Extract `(defmethod Float …)` blocks. Header:

```moof
;; lib/stdlib/float.moof
;;
;; Float — derived methods. arithmetic primitives live in rust;
;; this file derives :abs, :max:, :min:, :!=, :<=, :>=, :asInteger,
;; rounding helpers.
```

- [ ] Test + commit. Message: `stdlib/float.moof — Float methods split from bootstrap`.

### Task 28: `stdlib/string.moof`

- [ ] Extract `(defmethod String …)` blocks. Header:

```moof
;; lib/stdlib/string.moof
;;
;; String — derived methods. byte-level primitives (:length,
;; :byteLength, :byteAt:, :at:, :+, :=, :slice:length:, :as:,
;; :toList, :contains?:) live in rust; this file derives the rest.
;;
;; Note: the small set in `early/03-string-essentials.moof` (Phase 4)
;; needs to exist before defmethod runs. those duplicate-shadow this
;; file by design — same pattern as the rust :endsWith?: shadow used
;; to.
```

- [ ] Test + commit. Message: `stdlib/string.moof — String methods split from bootstrap`.

### Task 29: `stdlib/char.moof`

- [ ] Extract `(defmethod Char …)` blocks (if any exist in bootstrap; if none, create with the header below as a Phase 4 placeholder).

```moof
;; lib/stdlib/char.moof
;;
;; Char — small helpers. :codepoint and :< stay rust primitives;
;; everything else (:inspect, :digit?, :letter?, :uppercase, :lowercase)
;; lives here.
```

- [ ] Test + commit. Message: `stdlib/char.moof — Char methods placeholder`.

### Task 30: `stdlib/table.moof`

- [ ] Extract `(defmethod Table …)` blocks. Header:

```moof
;; lib/stdlib/table.moof
;;
;; Table — derived methods. heap primitives (:new, :length, :at:,
;; :at:put:, :push:, :pop, :keys, :values, :remove:, :containsKey?:)
;; live in rust; this file derives :size, :empty?, :nonEmpty?,
;; :forEach:, structural :=, :as:, :toString, :inspect, :asString.
```

- [ ] Test + commit. Message: `stdlib/table.moof — Table methods split from bootstrap`.

### Task 31: `stdlib/method.moof`

- [ ] Extract `(defmethod Method …)` blocks (placeholder if none in bootstrap.moof — Phase 4 will fill).

```moof
;; lib/stdlib/method.moof
;;
;; Method — reflection rendering. :body, :source, :params, :consts,
;; :bytecodes, :ics, :call live in rust; :toString and :inspect
;; (which walk slots) live here.
```

- [ ] Test + commit. Message: `stdlib/method.moof — Method methods placeholder`.

### Task 32: Delete `bootstrap.moof`

**Files:**
- Delete: `lib/bootstrap.moof`
- Modify: `lib/main.moof` (remove the now-redundant final load)

- [ ] **Step 1: Verify bootstrap.moof is empty (or only comments)**

```bash
cat lib/bootstrap.moof
```

Expected: only the file-header comment block remains, or it's truly empty after the per-proto extractions.

- [ ] **Step 2: Remove the `[$transporter load: "bootstrap.moof"]` line from main.moof**

- [ ] **Step 3: Delete the file**

```bash
rm lib/bootstrap.moof
```

- [ ] **Step 4: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add lib/main.moof
git rm lib/bootstrap.moof
git commit -m "bootstrap.moof — gone; replaced by early/ + stdlib/"
```

### Phase 3 checkpoint

`bootstrap.moof` and `compiler.moof` no longer exist as single files. The std lib is a tree of ~16 files, loaded in dependency order by `lib/main.moof`. **No method has migrated yet** — `intrinsics.rs` is still 3745 LoC. `cargo test --workspace` is green. Stop, breathe.

---

## Phase 4 — Method migration (Rust → Moof)

This is the actual radicality. For each proto, we add the missing moof implementations to `early/` (if needed before defmethod) or `stdlib/` (if defmethod-driven), then remove the corresponding Rust intrinsic. After each proto's migration, `cargo test --workspace` should be green.

The order is rough-dependency-first: things other migrations depend on go first (Object, Bool, nil), then leaf protos (Char, Method, etc.), then the bigger-derivation cases (String, Cons, Table).

### Task 33: Migrate Object's derivable methods

**Files:**
- Modify: `lib/stdlib/object.moof` (add migrated method bodies)
- Modify: `crates/substrate/src/intrinsics.rs` (remove migrated installs)

- [ ] **Step 1: Add the migrated methods to `stdlib/object.moof`**

Append:

```moof
;; ─────────────────────────────────────────────────────────────
;; Migrated from intrinsics.rs (was install_object_reflection).
;; Identity, equality, and the toString/initialize defaults.
;; Stays in rust: :proto, :slots, :handlers, :meta, :identity,
;; :source, :dnu, :new, :handlerAt:.
;; ─────────────────────────────────────────────────────────────

;; identity equality. takes two values, returns #true iff they're the
;; same heap object (or two equal tagged immediates). does NOT walk
;; protos; specifically NOT :=.
(defmethod Object (is other) (__identity-eq self other))

;; structural equality: defaults to identity. protos override (e.g.,
;; Cons walks pairs, String compares bytes). subclasses see this as
;; the "no override" answer.
(defmethod Object (= other) [self is other])

;; not-equal — derived.
(defmethod Object (!= other) [[self = other] not])

;; default :initialize — returns self unchanged. user protos override.
(defmethod Object (initialize) self)

;; default :toString — falls through to :name (the proto-Form's own
;; name slot, set by :proto-naming machinery). subclass-specific
;; toString overrides this.
;;
;; Note: tagged-immediate toString (Int, Float, Bool, Sym, Char, Nil)
;; is handled by their per-proto methods; Object's toString is for
;; proto-Forms and user-defined objects.
(defmethod Object (toString) (__form-name self))
```

The implementation requires two new free-function primitives in Rust:

- `__identity-eq` — compares two `Value`s by `==` (matches the existing rust `:is` impl).
- `__form-name` — returns the `:name` slot on the proto-Form, or a generic fallback string.

Add these to `install_globals` in `intrinsics.rs`:

```rust
    install_global(w, "__identity-eq", |_world, _self, args| {
        if args.len() != 2 {
            return Err(raise(_world, "arity", "(__identity-eq a b)"));
        }
        Ok(Value::Bool(args[0] == args[1]))
    });

    install_global(w, "__form-name", |world, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let name_sym = world.intern("name");
        let name_v = match v {
            Value::Form(id) => world.heap.get(id).meta(name_sym).unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        if let Some(s) = world.string_text(name_v) {
            return Ok(world.make_string(&s.to_string()));
        }
        if let Value::Sym(sym) = name_v {
            return Ok(world.make_string(&world.resolve(sym).to_string()));
        }
        // fallback — generic Object label.
        Ok(world.make_string("Object"))
    });
```

(If `__form-name` semantics need refinement to match the existing rust :toString name-fallback exactly, do so. The intent is that `[Cons toString]` returns `"Cons"`.)

- [ ] **Step 2: Remove the migrated installs from `intrinsics.rs`**

In `install_object_reflection`, delete the install_native blocks for `:is`, `:=`, `:!=`, `:initialize`, `:toString`. Keep `:proto`, `:slots`, `:handlers`, `:meta`, `:identity`, `:source`, `:handlerAt:`, `:new`, `:dnu`.

- [ ] **Step 3: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS. If anything fails, the most likely culprit is `__form-name`'s exact fallback behavior — match the rust impl for `Object:toString` carefully (especially for proto-Form receivers vs ordinary instances).

- [ ] **Step 4: Commit**

```bash
git add lib/stdlib/object.moof crates/substrate/src/intrinsics.rs
git commit -m "Object — migrate :is, :=, :!=, :initialize, :toString to moof"
```

### Task 34: Migrate Bool methods

**Files:**
- Modify: `lib/stdlib/bool.moof` (already has the methods from Phase 3)
- Modify: `crates/substrate/src/intrinsics.rs` (remove install_bool_methods)

- [ ] **Step 1: Verify `stdlib/bool.moof` already has :not, :and:, :or:, :toString**

These were defmethods in bootstrap.moof, so Phase 3 Task 23 already moved them.

- [ ] **Step 2: Remove `install_bool_methods` from intrinsics.rs**

It's already empty (`fn install_bool_methods(_: &mut World) {}` at line 1787) — just delete the function and its call site in `pub fn install`.

- [ ] **Step 3: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/substrate/src/intrinsics.rs
git commit -m "Bool — drop empty install_bool_methods (moof has it all)"
```

### Task 35: Migrate nil methods

**Files:**
- Modify: `lib/early/01-nil.moof` (fill in the migrated methods that need to exist before defmethod)
- Modify: `lib/stdlib/nil.moof` (already has the defmethod versions from Phase 3)
- Modify: `crates/substrate/src/intrinsics.rs` (remove install_nil_methods)

- [ ] **Step 1: Add the early-needed nil methods to `early/01-nil.moof`**

The `__decode-header` macro and quasiquote helpers reach `[v is nil]` and `(__list-empty? v)` only — they don't method-send on nil. So the early stage probably doesn't need ANY nil method installs. Confirm:

```bash
grep -n 'nil' lib/early/0[5-9]*.moof
```

If anything calls `[nil :foo]`, port that method to `early/01-nil.moof`. Otherwise leave it empty (the defmethods in `stdlib/nil.moof` cover all cases).

The Cons machinery (`(defmethod Cons (reverse) [[.cdr reverse] append: (cons .car nil)])`) bottoms out on `[nil append: …]` and `[nil reverse]`. These are method sends; they need to dispatch correctly when `Cons:reverse` runs. After Phase 3 these have been moved to `stdlib/nil.moof`; that file loads BEFORE `stdlib/cons.moof` per the alphabetical convention — so the dependency is satisfied.

(In summary: after Phase 3, all nil methods live in `stdlib/nil.moof` and load before any code that needs them. `early/01-nil.moof` stays empty.)

- [ ] **Step 2: Remove `install_nil_methods` from intrinsics.rs**

Delete the entire function (lines 1794-1833) and remove its call site in `pub fn install`. The `make_cons_method` helper (line 1835) stays — it's used by Cons too.

- [ ] **Step 3: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS. If tests fail, the failure pattern points at which nil method needs to live in `early/` instead of just `stdlib/`. The most likely candidates are `:cons:` (used during quasiquote expansion before `stdlib/nil.moof` loads). If quasiquote breaks, port `(setHandler! nil 'cons: (fn (x) (__alloc-cons x self)))` to `early/01-nil.moof`.

- [ ] **Step 4: Commit**

```bash
git add lib/early/01-nil.moof crates/substrate/src/intrinsics.rs
git commit -m "nil — drop install_nil_methods; stdlib/nil.moof is canonical"
```

### Task 36: Migrate Cons methods

**Files:**
- Modify: `lib/early/00-cons.moof` (port primitives that need to exist before defmethod, if any)
- Modify: `lib/stdlib/cons.moof` (already has defmethod versions of length/reverse/etc. from Phase 3)
- Modify: `crates/substrate/src/intrinsics.rs` (remove install_list_methods, keep car/cdr/cons:)

- [ ] **Step 1: Audit which Cons methods need to exist BEFORE defmethod runs**

Things `__decode-header` / quasiquote / module helpers do that involve Cons:

- `[forms car]`, `[forms cdr]` — handled by `__list-car` / `__list-cdr` primitives (Task 9).
- `[args length]` — handled by `__list-length`.
- `[xs is nil]` — `Object:is` (kept rust).
- `(cons head tail)` — free function in rust (kept).

So in fact NO Cons method-send happens during early/ stage IF we routed everything through the free-function primitives. Confirm:

```bash
grep -nE '\[[a-zA-Z]+ (length|reverse|empty\?|null\?|car|cdr|toString|inspect|cons:|append:|map:|filter:)' lib/early/*.moof | grep -v 'String\|Int\|Float\|Bool\|Object\|Symbol\|Match\|Defn\|DefProto\|Compiler\|chunk\|Macros\|Method\|Opcode\|Chunk'
```

If any of those matches a list-receiver call, port that method to `early/00-cons.moof`. Otherwise the file stays empty.

- [ ] **Step 2: Remove `install_list_methods` from intrinsics.rs**

KEEP the `:car`, `:cdr`, `:cons:` install blocks (heap primitives). REMOVE `:null?`, `:empty?`, `:reverse`, `:length`, `:toString`, `:inspect`. Reorganize the function so the kept lines stay coherent (e.g., rename to `install_cons_primitives` and only have the three primitive installs).

Also delete the `render_list_with` and `push_string_value` helpers if no other rust intrinsic still uses them.

- [ ] **Step 3: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add lib/early/00-cons.moof lib/stdlib/cons.moof crates/substrate/src/intrinsics.rs
git commit -m "Cons — keep :car/:cdr/:cons: in rust; rest migrate to moof"
```

### Task 37: Migrate Char :inspect

**Files:**
- Modify: `lib/stdlib/char.moof`
- Modify: `crates/substrate/src/intrinsics.rs` (remove `:inspect` install)

- [ ] **Step 1: Add the migrated :inspect to `stdlib/char.moof`**

The current rust impl produces `#\X` for printable codepoints, with escape rules for special characters (newline → `#\newline`, space → `#\space`, etc.). Port that logic:

```moof
;; :inspect — re-readable form. derives via :codepoint + the
;; substrate's char-name table. matches the existing rust
;; intrinsic exactly (newline, space, tab, return get long names;
;; everything else uses the literal char).
(defmethod Char (inspect)
  (let ((cp [self codepoint]))
    (if [cp = 32]   "#\\space"
    (if [cp = 9]    "#\\tab"
    (if [cp = 10]   "#\\newline"
    (if [cp = 13]   "#\\return"
        ["#\\" + [self toString]]))))))
```

(Adjust the cases to match exactly the cases the existing rust impl supports — if it has more, port them.)

- [ ] **Step 2: Remove the rust `:inspect` install from `install_char_methods`**

In `intrinsics.rs`, delete the `w.install_native(w.protos.char_, "inspect", …)` block (around lines 534-540).

- [ ] **Step 3: Run tests**

```bash
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add lib/stdlib/char.moof crates/substrate/src/intrinsics.rs
git commit -m "Char :inspect — migrate to moof"
```

### Task 38: Migrate Method :toString and :inspect

**Files:**
- Modify: `lib/stdlib/method.moof`
- Modify: `crates/substrate/src/intrinsics.rs` (remove the migrated installs)

- [ ] **Step 1: Inspect the rust impls**

```bash
grep -A20 'install_native(w\.protos\.method, "toString"\|install_native(w\.protos\.method, "inspect"' crates/substrate/src/intrinsics.rs
```

The rust impls walk the `:body`, `:params`, `:source` slots. Replicate in moof using the kept primitives `:body`, `:params`, `:source`:

```moof
;; lib/stdlib/method.moof additions:

(defmethod Method (toString)
  (let ((src [self source]))
    (if [src is nil]
        "<method>"
        ["#<method " + [src toString] + ">"])))

(defmethod Method (inspect)
  ;; re-readable: include the params list and a compact body shape.
  (let ((p [self params])
        (s [self source]))
    ["#<method params=" + [p inspect] + " source=" + [s inspect] + ">"]))
```

(Match the actual existing rust output shape exactly — read the rust impl first and replicate.)

- [ ] **Step 2: Remove the rust :toString and :inspect from `install_method_methods`**

Keep `:body`, `:source`, `:params`, `:consts`, `:bytecodes`, `:ics`, `:call`. Remove `:toString`, `:inspect`.

- [ ] **Step 3: Run tests + commit**

```bash
cargo test --workspace
git add lib/stdlib/method.moof crates/substrate/src/intrinsics.rs
git commit -m "Method :toString and :inspect — migrate to moof"
```

### Task 39: Migrate Float comparisons + :asInteger

**Files:**
- Modify: `lib/stdlib/float.moof`
- Modify: `crates/substrate/src/intrinsics.rs` (remove `:!=`, `:<=`, `:>=`, `:asInteger` installs if they exist)

- [ ] **Step 1: Verify what's currently in rust**

```bash
grep -n 'protos\.float, *"' crates/substrate/src/intrinsics.rs
```

`:asInteger` is at line 1730. `:!=, :<=, :>=` may also be present.

- [ ] **Step 2: Add moof implementations**

Append to `stdlib/float.moof`:

```moof
(defmethod Float (asInteger)
  (let ((s (if [self < 0.0] -1 1))
        (a [self abs]))
    [s * (__float-truncate a)]))

(defmethod Float (!= other) [[self = other] not])
(defmethod Float (<= other) (if [self < other] #true [self = other]))
(defmethod Float (>= other) (if [self > other] #true [self = other]))
```

Add `__float-truncate` as a free-function primitive in `intrinsics.rs::install_globals`:

```rust
    install_global(w, "__float-truncate", |world, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let f = v.as_float().ok_or_else(|| {
            type_error(world, "__float-truncate: not a Float")
        })?;
        Ok(Value::Int(f.trunc() as i64))
    });
```

- [ ] **Step 3: Remove the migrated installs from `install_float_methods`**

Delete the `:asInteger`, `:!=`, `:<=`, `:>=` blocks.

- [ ] **Step 4: Test + commit**

```bash
cargo test --workspace
git add lib/stdlib/float.moof crates/substrate/src/intrinsics.rs
git commit -m "Float — migrate :asInteger and comparison shortcuts to moof"
```

### Task 40: Migrate String derived methods

**Files:**
- Modify: `lib/early/03-string-essentials.moof` (the methods needed before defmethod runs)
- Modify: `lib/stdlib/string.moof` (the rest)
- Modify: `crates/substrate/src/intrinsics.rs` (remove migrated installs)

- [ ] **Step 1: Move `:endsWith?:` (and other Symbol-dependent methods) to `early/03-string-essentials.moof`**

```moof
;; early/03-string-essentials.moof body:

;; the small set of String methods __decode-header transitively needs.
;; setHandler! shape — defmethod isn't here yet.

(setHandler! String 'endsWith?:
  (fn (suffix)
    (let ((nself [self length])
          (nsuf  [suffix length]))
      (if [nsuf > nself]
          #false
          [[self slice: [nself - nsuf] length: nsuf] = suffix]))))

(setHandler! String 'startsWith?:
  (fn (prefix)
    (let ((np [prefix length]))
      (if [np > [self length]]
          #false
          [[self slice: 0 length: np] = prefix]))))

;; :all?: derived from :toList + Cons:all?: — but Cons:all?: lives
;; in stdlib/cons.moof (loaded later). Therefore at this point we
;; have to inline a recursive walk:
(setHandler! String 'all?:
  (fn (pred)
    (__string-all-bytes self pred)))
```

Add `__string-all-bytes` as a primitive (if it doesn't already exist) — or rephrase the `:all?:` impl to use `byteAt:` + `byteLength` purely:

```moof
(setHandler! String 'all?:
  (fn (pred)
    (let ((n [self byteLength]))
      (__string-all-bytes-loop self pred 0 n))))

(def __string-all-bytes-loop
  (fn (s pred i n)
    (if [i = n]
        #true
        (if (pred [s at: i])
            (__string-all-bytes-loop s pred [i + 1] n)
            #false))))
```

(`:contains?:` is similar — port carefully or rely on it staying as a Rust primitive if the dependency analysis demands.)

- [ ] **Step 2: Move the rest of String to `stdlib/string.moof` using `defmethod` shape**

Methods to migrate: `:trim`, `:indexOf:`, `:replace:with:`, `:split:`, `:lines`, `:toString`, `:inspect`, `:asTable`, `:reverse`. Each derives from byte primitives (`:byteAt:`, `:byteLength`, `:slice:length:`, `:+`, `:=`, `:toList`). Port one at a time; run tests after each.

For each method:
- Read the rust impl in `intrinsics.rs`'s `install_string_methods`.
- Write the moof equivalent in `stdlib/string.moof` using `(defmethod String (...) ...)`.
- Remove the corresponding `install_native` block from `install_string_methods`.

- [ ] **Step 3: Test + commit (one commit per method, or one combined commit at end if they're tightly coupled)**

```bash
cargo test --workspace
```

Expected: all PASS after each individual method migration. Commit each individually for bisect discipline:

```bash
git commit -m "String :trim — migrate to moof"
git commit -m "String :indexOf: — migrate to moof"
# … etc.
```

Or, if you do one big commit at the end:

```bash
git commit -m "String — migrate all derived methods (:trim, :split:, :replace:, :lines, :toString, :inspect, :asTable, :reverse) to moof"
```

### Task 41: Migrate Table derived methods

**Files:**
- Modify: `lib/stdlib/table.moof`
- Modify: `crates/substrate/src/intrinsics.rs`

- [ ] **Step 1: Migrate `:toString`, `:inspect`, `:asString`, `:=`, `:as:`, `:forEach:`**

For each, port the rust impl to moof, using the kept primitives (`:length`, `:at:`, `:keys`, `:values`).

Example for `:toString`:

```moof
(defmethod Table (toString)
  (let ((entries (__table-render-entries self)))
    [["{" + entries] + "}"]))
```

Implement `__table-render-entries` in moof using the kept primitives (`:keys`, `:values`, `:length`) — or, if too complex, keep `:toString` in rust and migrate only the simpler methods.

- [ ] **Step 2: Remove the migrated installs**

Delete from `install_table_methods` in `intrinsics.rs`.

- [ ] **Step 3: Test + commit**

```bash
cargo test --workspace
git commit -m "Table — migrate :toString, :inspect, :asString, :=, :as:, :forEach: to moof"
```

### Task 42: Final cleanup and verification

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs` (final cleanup pass)

- [ ] **Step 1: Re-audit `intrinsics.rs` for dead code**

```bash
wc -l crates/substrate/src/intrinsics.rs
grep -c 'install_native' crates/substrate/src/intrinsics.rs
```

Expected: from 3745 LoC → ~2400-2500 LoC. From ~85 install_natives → ~50.

- [ ] **Step 2: Remove unused helper functions**

```bash
cargo check 2>&1 | grep 'warning: function .* is never used'
```

For each warning, decide whether to delete or `#[allow(dead_code)]`. Almost always: delete.

- [ ] **Step 3: Final test run**

```bash
cargo test --workspace
```

Expected: all PASS, ideally ≥334 (the original count).

- [ ] **Step 4: Commit**

```bash
git add crates/substrate/src/intrinsics.rs
git commit -m "intrinsics — drop dead code after Rust→Moof migration"
```

### Task 43: Update NEXT_SESSION.md

**Files:**
- Modify: `NEXT_SESSION.md`

- [ ] **Step 1: Add a "completed pre-MCO cleanup" note**

Append a section describing what's now in place:

```markdown
## what stands today (post-cleanup)

- `$transporter` cap loads the std lib at runtime; `MOOF_LIB`
  override works.
- `$compiler useMoof` / `useSeed` replaces direct flag access.
- `lib/main.moof` is the single rust entry point; it orchestrates
  `compiler/`, `early/`, and `stdlib/` (16 files total).
- `intrinsics.rs` is ~2.5k LoC (down from 3.7k); only true
  primitives remain.
- REPL prints `nil` when the user evaluates `nil`. previously
  suppressed.

with this foundation, parser-in-moof becomes "just another
stdlib/ extraction" — the load-bearing infrastructure (transporter,
free-fn primitives, file split) is already there.
```

- [ ] **Step 2: Commit**

```bash
git add NEXT_SESSION.md
git commit -m "NEXT_SESSION.md — note pre-MCO cleanup completion"
```

---

## Final verification

After all 43 tasks:

- [ ] `cargo test --workspace` passes (all 334+ green).
- [ ] `cargo build --workspace` produces a working `moof` binary.
- [ ] `target/debug/moof nil` prints `nil\n`.
- [ ] `target/debug/moof '(+ 1 2)'` prints `3\n`.
- [ ] `target/debug/moof '[$transporter root]'` prints the lib path.
- [ ] `MOOF_LIB=/some/other/path target/debug/moof` works (or fails with `'tx-no-root` if path is invalid — that's correct).
- [ ] `wc -l crates/substrate/src/intrinsics.rs` shows ≤ ~2500 LoC.
- [ ] `lib/bootstrap.moof` and `lib/compiler.moof` no longer exist.
- [ ] `ls lib/` shows: `main.moof`, `compiler/`, `early/`, `stdlib/`.

If all of the above hold, the cleanup is complete and we can move on to the polyglot/MCO work in NEXT_SESSION.md.

---

## Self-review

**1. Spec coverage:** Each spec section maps to tasks:
- "$transporter cap" → Tasks 3-5
- "Lib root resolution" → Task 3 (resolve_lib_root) + Task 4 (population)
- "Boot dance" → Tasks 7-8 (main.moof + new_world rewrite)
- "$compiler useMoof" → Task 6
- "File structure" → Tasks 11 (compiler/), 12-21 (early/), 22-31 (stdlib/), 32 (delete bootstrap)
- "Radicality unlock — primitive free-functions" → Tasks 9-10
- "Method migration manifest" → Tasks 33-41 (one per proto)
- "REPL nil fix" → Tasks 1-2
- "Rust module structure" → Task 3 creates `transporter.rs`; Task 8 reshapes `lib.rs`
- "Testing strategy" → embedded throughout (TDD per task) plus Task 42 final verification.

**2. Placeholder scan:** No "TBD/TODO" left intentionally. Some Phase 3 tasks (12-15) create empty placeholder files — that's deliberate (Phase 4 fills them); the empty file IS the deliverable for that task and the task explains why.

**3. Type consistency:** Method names used consistently: `:load:` (with the colon), `:loadAll:` (camelCase plus colon), `:root` (no colon), `:dump:toFile:` (multi-keyword). `useMoof` / `useSeed` (camelCase, no colons — single-arg-less methods on $compiler). Free-function primitives all `__hyphen-case` per moof convention.

**4. Ambiguity check:** Two spots flagged — Task 33's `__form-name` exact behavior and Task 38's Method:toString shape — both call out "match the rust impl exactly" and direct the implementer to read the rust source first. The risk is small because behavior is testable via existing tests.

The plan is implementable as-is. Bite-sized commits, frequent test runs.
