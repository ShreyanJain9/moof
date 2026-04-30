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
pub mod opcodes;
pub mod protos;
pub mod reader;
pub mod sym;
pub mod table;
pub mod value;
pub mod vm;
pub mod world;

/// the moof-side bootstrap stdlib. embedded in the substrate
/// binary so the seed boots self-contained.
pub const BOOTSTRAP_SOURCE: &str = include_str!("../../../lib/bootstrap.moof");

/// build a fresh world with the phase-A intrinsics + bootstrap
/// stdlib loaded.
pub fn new_world() -> world::World {
    let mut w = world::World::new();
    intrinsics::install(&mut w);
    // load the moof-side bootstrap. failures here are substrate
    // bugs, not user errors — bootstrap.moof ships with the seed.
    if let Err(e) = eval_program(&mut w, BOOTSTRAP_SOURCE) {
        panic!("bootstrap.moof failed to load: {}", e.message);
    }
    w
}

/// build a fresh world *without* loading bootstrap.moof. used by
/// tests that exercise raw substrate behavior without the moof-side
/// stdlib.
pub fn new_world_bare() -> world::World {
    let mut w = world::World::new();
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
/// of the last. used to load multi-form scripts (incl. bootstrap.moof).
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
