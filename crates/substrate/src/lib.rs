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
pub mod wasm;
pub mod world;

/// the moof-side bootstrap stdlib. embedded in the substrate
/// binary so the seed boots self-contained.
pub const BOOTSTRAP_SOURCE: &str = include_str!("../../../lib/bootstrap.moof");

/// the moof-side bytecode compiler. loaded after bootstrap; defines
/// `compile-form` and per-special-form helpers. track 3 will flip
/// the `World::use_moof_compiler` flag so this becomes canonical.
/// see `docs/reference/compiler-primitives.md`.
pub const COMPILER_SOURCE: &str = include_str!("../../../lib/compiler.moof");

/// build a fresh world with the phase-A intrinsics, the moof-side
/// compiler, and the bootstrap stdlib loaded — in that order.
///
/// the boot dance, per `docs/process/self-hosted-compiler.md`:
///
/// 1. rust intrinsics (heap, OS i/o, arithmetic primitives, the
///    chunk-construction api, etc).
/// 2. **rust compiler compiles `compiler.moof`.** the rust
///    compiler is sized to handle exactly the special forms that
///    file uses: `def`, `fn`, `if`, `let`, `do`, `quote`,
///    `__send__`. nothing else.
/// 3. flip `use_moof_compiler` — from this point, every compile
///    routes through moof's `compile-top`.
/// 4. **moof compiler compiles `bootstrap.moof`.** all the macros
///    (`when`, `match`, `defn`, `defmethod`, `defproto`, …) and
///    method installations land via the canonical compiler.
///
/// failures at any step are substrate bugs (both files ship with
/// the seed), so we panic.
pub fn new_world() -> world::World {
    let mut w = world::World::new();
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
