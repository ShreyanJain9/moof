//! the substrate's bytecode op set.
//!
//! ~30 opcodes total. each op is one [`Op`] variant. the
//! interpreter (`vm`) walks `Vec<Op>` from a chunk and keeps a
//! stack of `Value`s.
//!
//! per `laws/substrate-laws.md` L5, bytecode is *derived* from a
//! source-form. the chunk owns the `source` so re-derivation works
//! after a method's source is edited.
//!
//! per `laws/reflection-contract.md` R2, `[m bytecodes]` exposes a
//! decoded view of the bytecode. moof code can read it, but cannot
//! edit it directly — edit source, the substrate re-derives.

use crate::form::FormId;
use crate::sym::SymId;

/// the substrate's bytecode op set. each op is a single variant;
/// `vm` matches on this directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
    /// push the constant at `consts[idx]` onto the stack.
    LoadConst(u16),

    /// push `nil`.
    PushNil,

    /// push `#true`.
    PushTrue,

    /// push `#false`.
    PushFalse,

    /// pop and discard the top of stack.
    Pop,

    /// duplicate the top of stack.
    Dup,

    /// look up `name` in the current frame's lexical environment;
    /// push its value.
    LoadName(SymId),

    /// push the receiver (`self`) of the current method frame.
    LoadSelf,

    /// pop `argc` args and a receiver; send `selector` to the
    /// receiver. `ic_idx` is this site's inline-cache slot.
    Send {
        selector: SymId,
        argc: u8,
        ic_idx: u16,
    },

    /// like `Send`, but in tail position — the current frame is
    /// reused. no stack growth.
    TailSend { selector: SymId, argc: u8 },

    /// `[super selector args…]` — dispatch to the proto-chain
    /// position *above* the current frame's defining proto, with
    /// `self` as the receiver. lets a method delegate to the
    /// inherited implementation it overrode.
    SuperSend {
        selector: SymId,
        argc: u8,
        ic_idx: u16,
    },

    /// allocate a new closure capturing the current env, with the
    /// chunk identified by `chunk` (a Form-id pointing to a
    /// chunk-Form in the heap).
    PushClosure { chunk: FormId },

    /// jump to `pc + offset` (signed; `offset` may be negative).
    Jump(i16),

    /// pop a value; if falsy, jump to `pc + offset`.
    JumpIfFalse(i16),

    /// pop the top of stack and return it from this frame.
    Return,
}

impl Op {
    /// `true` if executing this op leaves the stack with one more
    /// element than before. used by the compiler's stack-balance
    /// checker (when written).
    pub fn pushes(self) -> bool {
        matches!(
            self,
            Op::LoadConst(_)
                | Op::PushNil
                | Op::PushTrue
                | Op::PushFalse
                | Op::Dup
                | Op::LoadName(_)
                | Op::LoadSelf
                | Op::PushClosure { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_is_copy_and_small() {
        // ops are passed by value through the interpreter loop;
        // they need to be cheap to copy.
        assert!(std::mem::size_of::<Op>() <= 16);
    }

    #[test]
    fn pushes_classifies_correctly() {
        assert!(Op::PushNil.pushes());
        assert!(Op::LoadConst(0).pushes());
        assert!(Op::LoadName(SymId(1)).pushes());
        assert!(Op::LoadSelf.pushes());
        assert!(!Op::Pop.pushes());
        assert!(!Op::Return.pushes());
    }

    #[test]
    fn equality_and_hashing_work() {
        // ops compare by value, useful for snapshot tests on
        // compiled chunks.
        let a = Op::Send {
            selector: SymId(7),
            argc: 2,
            ic_idx: 0,
        };
        let b = Op::Send {
            selector: SymId(7),
            argc: 2,
            ic_idx: 0,
        };
        assert_eq!(a, b);
        let c = Op::Send {
            selector: SymId(7),
            argc: 3,
            ic_idx: 0,
        };
        assert_ne!(a, c);
    }
}
