//! moof v4 substrate seed.
//!
//! the load-bearing rust crate. ≤3k LoC budgeted; everything else
//! ships as moof code or via mcos (`docs/concepts/compiled-objects.md`).
//!
//! ## modules
//!
//! - [`value`] — the runtime value enum (Form-with-tagged-immediates).
//! - [`sym`] — symbol interning.
//! - [`form`] — the universal heap kind.
//! - [`heap`] — the form allocator.
//! - [`foreign`] — opaque rust-side handles, slotted into Forms.
//! - [`reader`] — the bootstrap sexpr parser.
//! - [`opcodes`] — bytecode op set.
//! - [`vm`] — the bytecode interpreter + send dispatch.
//! - [`compiler`] — bootstrap Form → Chunk compiler.
//! - [`world`] — the substrate's per-vat root.
//!
//! see `docs/laws/substrate-laws.md` for the load-bearing
//! invariants this crate must uphold.

pub mod compiler;
pub mod foreign;
pub mod form;
pub mod heap;
pub mod intrinsics;
pub mod nursery;
pub mod opcodes;
pub mod protos;
pub mod reader;
pub mod sym;
pub mod table;
pub mod transporter;
pub mod value;
pub mod vm;
pub mod wasm;
pub mod world;

/// the Hash mco bytes, baked in at compile time.
/// built by `lib/mcos/hash/build.sh`; path resolved by `crates/substrate/build.rs`.
const HASH_MCO_BYTES: &[u8] = include_bytes!(env!("MOOF_HASH_MCO_PATH"));

/// V2 — vat mode. controls whether `:new` (Object, Table) returns
/// born-mutable or born-frozen instances. lives on `World` for V2;
/// will move to per-`Vat` in V4.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VatMode {
    MutableByDefault,
    FrozenByDefault,
}

/// V2 — when does `vat_mode` take effect? `PostBootstrap` (default)
/// runs lib bootstrap in mutable regardless, then flips to `mode`
/// for user code. `FromBoot` applies `mode` from the very first
/// allocation — opt-in expert path; standard lib may not load
/// cleanly under `FromBoot + FrozenByDefault`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModeScope {
    PostBootstrap,
    FromBoot,
}

/// shared body for [`new_world`] / [`new_world_with_mode_scoped`] —
/// see [`new_world`] for the full boot sequence.
fn build_world_with_initial_mode(initial_mode: VatMode) -> world::World {
    let mut w = world::World::new();
    w.vat_mode = initial_mode;
    w.transporter_root = transporter::resolve_lib_root();

    // wrap intrinsics::install + the $hash bootstrap in an explicit
    // turn so the nursery-aware mutation paths (form_slot_set,
    // form_handler_set, form_meta_set) satisfy their in_turn
    // invariant. these run between World::new (which auto-commits
    // its own boot turn) and the first eval_program (which opens
    // its own implicit turn).
    w.start_turn();
    intrinsics::install(&mut w);

    // bootstrap $hash from embedded Hash mco bytes — BEFORE lib/main.moof
    // loads lib/mcos.moof, which calls [$hash of: bytes] for hash verification.
    // we bind $hash as an instance (not the proto directly) so that
    // [$hash of: bytes] dispatches without an explicit [hash-proto new].
    {
        let hash_proto = wasm::load_wasm_bytes(&mut w, HASH_MCO_BYTES, "embedded-hash")
            .unwrap_or_else(|e| {
                panic!("Hash mco bootstrap failed — substrate is broken: {}", e.message)
            });
        // call [hash_proto new] to get an instance: method dispatch walks
        // hash_proto → object proto → finds "new" there.
        let new_sel = w.intern("new");
        let hash_instance = w
            .send(hash_proto, new_sel, &[])
            .unwrap_or_else(|e| {
                panic!("Hash mco [new] failed during bootstrap: {}", e.message)
            });
        let dollar_hash = w.intern("$hash");
        let global = w.here_form;
        w.env_bind(global, dollar_hash, hash_instance)
            .expect("env_bind at boot — substrate bug");
    }

    let _ = w.commit_turn();

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

/// build a fresh world with the phase-A intrinsics, the $transporter
/// cap populated, and `lib/main.moof` loaded — which itself orchestrates
/// loading the rest of the std lib.
///
/// the boot dance, per `docs/process/self-hosted-compiler.md`:
///
/// 1. rust intrinsics (heap, OS i/o, arithmetic primitives, the
///    chunk-construction api, the `$transporter` and `$compiler` caps).
/// 2. bootstrap the embedded Hash mco and bind `$hash` before any
///    moof code runs — lib/mcos.moof's $mco cap calls `[$hash of: ...]`.
/// 3. resolve the lib root via `transporter::resolve_lib_root` and
///    bind it on `World.transporter_root`.
/// 4. read `<root>/main.moof`. main.moof drives:
///    a. `[$transporter load: "compiler.moof"]` — seed-compiled.
///    b. `[$compiler useMoof]` — flag flip.
///    c. `[$transporter load: "bootstrap.moof"]` — moof-compiled.
///
/// failures at any step are substrate bugs (lib/ ships with the
/// substrate), so we panic.
pub fn new_world() -> world::World {
    build_world_with_initial_mode(VatMode::MutableByDefault)
}

/// build a world with a specified [`VatMode`] and [`ModeScope`].
/// see [`ModeScope`] for the difference between `PostBootstrap`
/// (default; safe with standard lib) and `FromBoot` (mode applies
/// from the very first allocation; opt-in expert path).
pub fn new_world_with_mode_scoped(
    mode: VatMode,
    scope: ModeScope,
) -> world::World {
    let initial = match scope {
        ModeScope::PostBootstrap => VatMode::MutableByDefault,
        ModeScope::FromBoot => mode,
    };
    let mut w = build_world_with_initial_mode(initial);
    w.vat_mode = mode; // either no-op (FromBoot) or post-flip (PostBootstrap)
    w
}

/// shorthand for [`new_world_with_mode_scoped`] with
/// [`ModeScope::PostBootstrap`] — the safe default. lib bootstrap
/// runs in mutable regardless of `mode`; the requested mode applies
/// to user code that runs after this returns.
pub fn new_world_with_mode(mode: VatMode) -> world::World {
    new_world_with_mode_scoped(mode, ModeScope::PostBootstrap)
}

/// shared body for [`new_world_bare`] / [`new_world_bare_with_mode_scoped`] —
/// see [`new_world_bare`] for what "bare" means.
fn build_world_bare_with_initial_mode(initial_mode: VatMode) -> world::World {
    let mut w = world::World::new();
    w.vat_mode = initial_mode;
    w.transporter_root = transporter::resolve_lib_root();
    // same turn-wrap as new_world — intrinsics::install needs in_turn.
    w.start_turn();
    intrinsics::install(&mut w);
    let _ = w.commit_turn();
    w
}

/// build a fresh world *without* loading any moof code. used by
/// tests that exercise raw substrate behavior without the moof-side
/// stdlib.
pub fn new_world_bare() -> world::World {
    build_world_bare_with_initial_mode(VatMode::MutableByDefault)
}

/// bare-world variant of [`new_world_with_mode_scoped`] — same
/// [`VatMode`] / [`ModeScope`] semantics, but skips loading
/// `lib/main.moof` (no stdlib). see [`new_world_bare`] for the
/// no-mode-args version.
pub fn new_world_bare_with_mode_scoped(
    mode: VatMode,
    scope: ModeScope,
) -> world::World {
    let initial = match scope {
        ModeScope::PostBootstrap => VatMode::MutableByDefault,
        ModeScope::FromBoot => mode,
    };
    let mut w = build_world_bare_with_initial_mode(initial);
    w.vat_mode = mode;
    w
}

/// bare-world shorthand for [`new_world_bare_with_mode_scoped`]
/// with [`ModeScope::PostBootstrap`] — the safe default for tests
/// that want a specific [`VatMode`] without loading the stdlib.
pub fn new_world_bare_with_mode(mode: VatMode) -> world::World {
    new_world_bare_with_mode_scoped(mode, ModeScope::PostBootstrap)
}

/// evaluate a single expression in the world's global env.
pub fn eval(w: &mut world::World, source: &str) -> Result<value::Value, world::RaiseError> {
    let was_in_turn = w.in_turn();
    if !was_in_turn {
        w.start_turn();
    }
    let result = eval_inner(w, source);
    if !was_in_turn {
        match &result {
            Ok(_) => { let _ = w.commit_turn(); }
            Err(_) => { w.abort_turn(); }
        }
    }
    result
}

fn eval_inner(
    w: &mut world::World,
    source: &str,
) -> Result<value::Value, world::RaiseError> {
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
    let was_in_turn = w.in_turn();
    if !was_in_turn {
        w.start_turn();
    }
    let result = eval_program_inner(w, source);
    if !was_in_turn {
        match &result {
            Ok(_) => { let _ = w.commit_turn(); }
            Err(_) => { w.abort_turn(); }
        }
    }
    result
}

fn eval_program_inner(
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
