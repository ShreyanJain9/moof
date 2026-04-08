//! Register-based bytecode instruction set.
//!
//! Each instruction is 4 bytes:
//!   byte 0: opcode
//!   bytes 1-3: operands (register indices or constant pool indices)
//!
//! Registers are u8 indices into the current frame's register file.
//! Constants are u16 indices into the chunk's constant pool.

/// Bytecode opcodes for the register-based VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    /// LOAD_CONST dst, const_hi, const_lo
    /// Load constant pool[const] into register dst.
    LoadConst = 0,

    /// LOAD_NIL dst, _, _
    LoadNil = 1,

    /// LOAD_TRUE dst, _, _
    LoadTrue = 2,

    /// LOAD_FALSE dst, _, _
    LoadFalse = 3,

    /// MOVE dst, src, _
    /// Copy register src to dst.
    Move = 4,

    /// LOAD_LOCAL dst, frame_depth, slot
    /// Load from enclosing environment: frame_depth levels up, slot index.
    LoadLocal = 5,

    /// STORE_LOCAL frame_depth, slot, src
    /// Store register src into environment frame.
    StoreLocal = 6,

    /// SEND dst, recv, selector_const
    /// [recv selector] → dst. args in registers recv+1..recv+N.
    /// N is encoded in the next byte after selector_const.
    Send = 7,

    /// SEND_N dst, recv, nargs
    /// Extended send: selector is in register recv+nargs+1.
    /// Used for dynamic selectors.
    SendN = 8,

    /// CALL dst, func, nargs
    /// (func arg1 arg2 ...) → dst.
    /// Args in registers func+1..func+nargs.
    Call = 9,

    /// TAIL_CALL func, nargs, _
    /// Like CALL but replaces the current frame.
    TailCall = 10,

    /// RETURN src, _, _
    /// Return register src from the current frame.
    Return = 11,

    /// JUMP offset_hi, offset_lo, _
    /// Unconditional jump (signed 16-bit offset from current PC).
    Jump = 12,

    /// JUMP_IF_FALSE test, offset_hi, offset_lo
    /// Jump if register test is falsy.
    JumpIfFalse = 13,

    /// JUMP_IF_TRUE test, offset_hi, offset_lo
    JumpIfTrue = 14,

    /// CONS dst, car, cdr
    Cons = 15,

    /// CAR dst, pair, _
    Car = 16,

    /// CDR dst, pair, _
    Cdr = 17,

    /// EQ dst, a, b
    /// Identity comparison.
    Eq = 18,

    /// DEF name_const_hi, name_const_lo, src
    /// Bind a name in the current environment.
    Def = 19,

    /// MAKE_OBJECT dst, parent, _
    /// Create a new empty object with the given parent.
    MakeObject = 20,

    /// SET_SLOT obj, slot_const, value
    /// Set a slot on an object.
    SetSlot = 21,

    /// SET_HANDLER obj, selector_const, handler
    /// Set a handler on an object.
    SetHandler = 22,

    /// CLOSURE dst, code_const, _
    /// Create a closure capturing the current environment.
    Closure = 23,

    /// HALT _, _, _
    Halt = 255,
}

impl Op {
    pub fn from_u8(b: u8) -> Option<Op> {
        match b {
            0 => Some(Op::LoadConst),
            1 => Some(Op::LoadNil),
            2 => Some(Op::LoadTrue),
            3 => Some(Op::LoadFalse),
            4 => Some(Op::Move),
            5 => Some(Op::LoadLocal),
            6 => Some(Op::StoreLocal),
            7 => Some(Op::Send),
            8 => Some(Op::SendN),
            9 => Some(Op::Call),
            10 => Some(Op::TailCall),
            11 => Some(Op::Return),
            12 => Some(Op::Jump),
            13 => Some(Op::JumpIfFalse),
            14 => Some(Op::JumpIfTrue),
            15 => Some(Op::Cons),
            16 => Some(Op::Car),
            17 => Some(Op::Cdr),
            18 => Some(Op::Eq),
            19 => Some(Op::Def),
            20 => Some(Op::MakeObject),
            21 => Some(Op::SetSlot),
            22 => Some(Op::SetHandler),
            23 => Some(Op::Closure),
            255 => Some(Op::Halt),
            _ => None,
        }
    }
}

/// A compiled bytecode chunk.
pub struct Chunk {
    pub code: Vec<u8>,
    pub constants: Vec<u64>, // Value bits
    pub name: String,
    pub arity: u8,
    pub num_registers: u8,
}

impl Chunk {
    pub fn new(name: impl Into<String>, arity: u8) -> Self {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            name: name.into(),
            arity,
            num_registers: 0,
        }
    }

    /// Add a constant, return its index.
    pub fn add_constant(&mut self, val: u64) -> u16 {
        let idx = self.constants.len() as u16;
        self.constants.push(val);
        idx
    }

    /// Emit a 4-byte instruction.
    pub fn emit(&mut self, op: Op, a: u8, b: u8, c: u8) {
        self.code.push(op as u8);
        self.code.push(a);
        self.code.push(b);
        self.code.push(c);
    }

    /// Current code offset (for jump patching).
    pub fn offset(&self) -> usize {
        self.code.len()
    }

    /// Patch a jump instruction's offset.
    pub fn patch_jump(&mut self, instr_offset: usize, target: usize) {
        let delta = (target as i16) - (instr_offset as i16) - 4; // 4 bytes per instruction
        let bytes = delta.to_be_bytes();
        // the offset is in bytes 2 and 3 of the instruction (or 1 and 2 for JUMP)
        self.code[instr_offset + 1] = bytes[0];
        self.code[instr_offset + 2] = bytes[1];
    }
}
