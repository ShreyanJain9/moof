//! bytecode opcodes and Chunks.
//!
//! a Chunk is a compiled method body (or a top-level expression).
//! it contains: the opcode stream, a constants table, a symbols
//! table (for selectors and global names), and inline-cache slots.
//!
//! phase 2 grows the opcode set to support special forms (def, if,
//! let, fn, do, quote) and lexical scope via env-Forms. names
//! resolve through the current frame's env chain (LoadName), not
//! through a flat globals hashmap.

use crate::form::{FormId, MethodImpl};
use crate::sym::SymId;
use crate::value::Value;

/// stable identity for a Chunk in the world's chunk table.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ChunkId(pub u32);

/// an inline-cache slot for a Send opcode.
///
/// substrate-laws.md L3 + concepts/sends-and-calls.md: every
/// send-site has one of these. on first dispatch, the resolved
/// (proto, method) is recorded; subsequent sends with the same
/// receiver-proto skip the proto-chain walk.
#[derive(Clone, Default)]
pub struct ICache {
    pub cached_proto: Option<FormId>,
    pub cached_method: Option<MethodImpl>,
}

/// the opcode set.
///
/// kept small. each one corresponds to one explicit operation; we
/// favor a few primitives over many specialized ops. the IC slot
/// indices are u16 — 65k cache slots per chunk is plenty.
#[derive(Clone, Debug)]
pub enum Op {
    // ── stack literals ─────────────────────────────────────────────
    LoadNil,
    LoadBool(bool),
    LoadInt(i32),
    LoadConst(u16),
    LoadSym(SymId),

    // ── env access ────────────────────────────────────────────────
    /// look up a name in the current frame's env chain. errors if not
    /// found. (concepts/forms.md: env is itself a Form; lookup walks
    /// its `:__parent` chain.)
    LoadName(SymId),
    /// define a *new* binding in the current frame's env. used by
    /// `def` and `let`. shadows any binding of the same name in
    /// outer envs.
    DefineName(SymId),
    /// set an *existing* binding in the env chain. errors if not
    /// found. used by `set!` (later phase).
    #[allow(dead_code)]
    SetName(SymId),

    // ── dispatch ──────────────────────────────────────────────────
    /// pop `arity + 1` values: receiver and args. dispatch the send;
    /// push the result. uses inline-cache slot at `ic_idx`.
    Send {
        sel: SymId,
        arity: u8,
        ic_idx: u16,
    },

    // ── control flow ─────────────────────────────────────────────
    /// unconditional relative jump from the next instruction.
    Branch(i16),
    /// pop one value; if falsy (Nil or #false), jump.
    BranchIfFalse(i16),

    // ── scope ────────────────────────────────────────────────────
    /// push a new env Form on top of the current frame's env. used by
    /// `let` to introduce a fresh scope.
    PushScope,
    /// pop back to the env's parent. used by `let` after body.
    PopScope,

    // ── closures ─────────────────────────────────────────────────
    /// allocate a Closure Form whose body is the chunk at index
    /// `chunk_idx` in the chunk's nested-chunks table. captures the
    /// current frame's env. the closure's params come from a List
    /// constant at `params_idx`.
    MakeClosure {
        chunk_idx: u16,
        params_idx: u16,
    },

    // ── stack management ─────────────────────────────────────────
    Pop,
    Return,
}

/// a compiled chunk of bytecode.
pub struct Chunk {
    pub ops: Vec<Op>,
    pub consts: Vec<Value>,
    pub ics: Vec<ICache>,
    /// nested chunks for closures defined inside this chunk.
    /// MakeClosure references one of these.
    pub nested: Vec<ChunkId>,
    /// optional: source-form this chunk was compiled from.
    /// substrate-laws.md L5: source is canonical, bytecode derived.
    pub source: Option<Value>,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            ops: Vec::new(),
            consts: Vec::new(),
            ics: Vec::new(),
            nested: Vec::new(),
            source: None,
        }
    }

    /// allocate a fresh inline-cache slot, returning its index.
    pub fn alloc_ic(&mut self) -> u16 {
        let idx = self.ics.len() as u16;
        self.ics.push(ICache::default());
        idx
    }

    /// add a constant, returning its index.
    pub fn add_const(&mut self, v: Value) -> u16 {
        let idx = self.consts.len() as u16;
        self.consts.push(v);
        idx
    }

    /// add a nested chunk reference, returning its index.
    pub fn add_nested(&mut self, chunk_id: ChunkId) -> u16 {
        let idx = self.nested.len() as u16;
        self.nested.push(chunk_id);
        idx
    }

    /// emit an op and return the position where it was placed
    /// (so callers can later patch a branch target).
    pub fn emit(&mut self, op: Op) -> usize {
        let pos = self.ops.len();
        self.ops.push(op);
        pos
    }

    /// patch a Branch / BranchIfFalse at `pos` so it jumps to the
    /// current end of the op stream.
    pub fn patch_branch_to_here(&mut self, pos: usize) -> Result<(), String> {
        let target = self.ops.len();
        // offset is from the instruction *after* the branch
        let offset_isize: isize = (target as isize) - (pos as isize) - 1;
        let offset: i16 = offset_isize
            .try_into()
            .map_err(|_| format!("branch offset out of range: {offset_isize}"))?;
        match &mut self.ops[pos] {
            Op::Branch(o) => *o = offset,
            Op::BranchIfFalse(o) => *o = offset,
            other => return Err(format!("not a branch op at {pos}: {other:?}")),
        }
        Ok(())
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}
