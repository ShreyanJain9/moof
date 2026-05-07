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
    let mut w = world::World::new();
    w.transporter_root = transporter::resolve_lib_root();
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
        let global = w.global_env;
        w.env_bind(global, dollar_hash, hash_instance);
    }

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
