// opcodes.rs — register-based bytecode for the moof objectspace
//
// instruction format: 4 bytes = [opcode: u8, a: u8, b: u8, c: u8]
//
// the vm's only real primitive is SEND. everything else is sugar or
// infrastructure to set up sends. (f a b c) desugars to [f call: a b c].
//
// lexical access uses de bruijn indices (depth, slot) — no name lookup
// at runtime. closures capture the environment chain.
//
// tail calls are mandatory. recursive loops depend on them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    LoadConst    = 0x01, // dst, const_hi, const_lo
    LoadNil      = 0x02, // dst, _, _
    LoadTrue     = 0x03, // dst, _, _
    LoadFalse    = 0x04, // dst, _, _
    Move         = 0x05, // dst, src, _
    GetLocal     = 0x06, // dst, depth, slot
    SetLocal     = 0x07, // depth, slot, src
    GetUpval     = 0x08, // dst, depth, slot  (alias for GetLocal semantics)
    Send         = 0x10, // dst, recv, sel_const  (nargs in next instruction byte)
    Call         = 0x11, // dst, func, nargs
    TailCall     = 0x12, // func, nargs, _
    Return       = 0x13, // src, _, _
    Jump         = 0x20, // offset_hi, offset_lo, _
    JumpIfFalse  = 0x21, // test, offset_hi, offset_lo
    JumpIfTrue   = 0x22, // test, offset_hi, offset_lo
    Cons         = 0x30, // dst, car, cdr
    Eq           = 0x31, // dst, a, b
    MakeObj      = 0x40, // dst, parent, nslots
    SetSlot      = 0x41, // obj, name_const, val
    SetHandler   = 0x42, // obj, sel_const, handler
    MakeClosure  = 0x50, // dst, code_const_hi, code_const_lo
    LoadInt      = 0x51, // dst, value_hi, value_lo
    MakeTable    = 0x52, // dst, nseq, nmap — followed by register lists
    GetGlobal    = 0x60, // dst, name_hi, name_lo  (name is symbol constant index)
    DefGlobal    = 0x61, // name_hi, name_lo, src  (bind name to register value)
    Eval         = 0x70, // dst, src, _  — compile and execute src as AST, result in dst
}

impl Op {
    pub fn from_u8(byte: u8) -> Option<Op> {
        match byte {
            0x01 => Some(Op::LoadConst),
            0x02 => Some(Op::LoadNil),
            0x03 => Some(Op::LoadTrue),
            0x04 => Some(Op::LoadFalse),
            0x05 => Some(Op::Move),
            0x06 => Some(Op::GetLocal),
            0x07 => Some(Op::SetLocal),
            0x08 => Some(Op::GetUpval),
            0x10 => Some(Op::Send),
            0x11 => Some(Op::Call),
            0x12 => Some(Op::TailCall),
            0x13 => Some(Op::Return),
            0x20 => Some(Op::Jump),
            0x21 => Some(Op::JumpIfFalse),
            0x22 => Some(Op::JumpIfTrue),
            0x30 => Some(Op::Cons),
            0x31 => Some(Op::Eq),
            0x40 => Some(Op::MakeObj),
            0x41 => Some(Op::SetSlot),
            0x42 => Some(Op::SetHandler),
            0x50 => Some(Op::MakeClosure),
            0x51 => Some(Op::LoadInt),
            0x52 => Some(Op::MakeTable),
            0x60 => Some(Op::GetGlobal),
            0x61 => Some(Op::DefGlobal),
            0x70 => Some(Op::Eval),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<u8>,
    pub constants: Vec<u64>,
    pub arity: u8,
    pub num_regs: u8,
    pub name: String,
}

impl Chunk {
    pub fn new(name: impl Into<String>, arity: u8, num_regs: u8) -> Self {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            arity,
            num_regs,
            name: name.into(),
        }
    }

    pub fn add_constant(&mut self, value: u64) -> u16 {
        // dedup: reuse existing slot if the bits match
        if let Some(idx) = self.constants.iter().position(|&v| v == value) {
            return idx as u16;
        }
        let idx = self.constants.len();
        assert!(idx <= u16::MAX as usize, "constant pool overflow");
        self.constants.push(value);
        idx as u16
    }

    pub fn emit(&mut self, op: Op, a: u8, b: u8, c: u8) -> usize {
        let offset = self.code.len();
        self.code.push(op as u8);
        self.code.push(a);
        self.code.push(b);
        self.code.push(c);
        offset
    }

    pub fn emit_wide(&mut self, op: Op, a: u8, wide: u16) -> usize {
        let [hi, lo] = wide.to_be_bytes();
        self.emit(op, a, hi, lo)
    }

    pub fn offset(&self) -> usize {
        self.code.len()
    }

    // emit a jump with a placeholder offset, returns the position to patch
    pub fn emit_jump(&mut self, op: Op, test: u8) -> usize {
        match op {
            Op::Jump => self.emit(op, 0xFF, 0xFF, 0),
            Op::JumpIfFalse | Op::JumpIfTrue => self.emit(op, test, 0xFF, 0xFF),
            _ => panic!("emit_jump called with non-jump opcode"),
        }
    }

    // patch a previously emitted jump to land at the current offset
    pub fn patch_jump(&mut self, site: usize) {
        let op = Op::from_u8(self.code[site]).expect("invalid opcode at patch site");
        let target = self.code.len() as isize;
        let origin = (site + 4) as isize; // instruction after the jump
        let delta = target - origin;
        assert!(
            delta >= i16::MIN as isize && delta <= i16::MAX as isize,
            "jump offset out of i16 range"
        );
        let [hi, lo] = (delta as i16).to_be_bytes();
        match op {
            Op::Jump => {
                self.code[site + 1] = hi;
                self.code[site + 2] = lo;
            }
            Op::JumpIfFalse | Op::JumpIfTrue => {
                self.code[site + 2] = hi;
                self.code[site + 3] = lo;
            }
            _ => panic!("patch_jump on non-jump opcode"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_basic_instruction() {
        let mut chunk = Chunk::new("test", 0, 4);
        let off = chunk.emit(Op::LoadNil, 0, 0, 0);
        assert_eq!(off, 0);
        assert_eq!(chunk.code.len(), 4);
        assert_eq!(Op::from_u8(chunk.code[0]), Some(Op::LoadNil));
    }

    #[test]
    fn emit_wide_constant() {
        let mut chunk = Chunk::new("test", 0, 4);
        let idx = chunk.add_constant(0xDEAD);
        let off = chunk.emit_wide(Op::LoadConst, 0, idx);
        assert_eq!(off, 0);
        assert_eq!(chunk.code[0], Op::LoadConst as u8);
        assert_eq!(chunk.code[1], 0); // dst
        // constant index 0 as big-endian u16
        assert_eq!(chunk.code[2], 0);
        assert_eq!(chunk.code[3], 0);
    }

    #[test]
    fn add_constant_dedup() {
        let mut chunk = Chunk::new("test", 0, 4);
        let a = chunk.add_constant(42);
        let b = chunk.add_constant(99);
        let c = chunk.add_constant(42);
        assert_eq!(a, c);
        assert_ne!(a, b);
        assert_eq!(chunk.constants.len(), 2);
    }

    #[test]
    fn jump_patch_forward() {
        let mut chunk = Chunk::new("test", 0, 4);
        let site = chunk.emit_jump(Op::JumpIfFalse, 0);
        chunk.emit(Op::LoadNil, 1, 0, 0); // filler
        chunk.emit(Op::LoadTrue, 2, 0, 0); // filler
        chunk.patch_jump(site);

        // delta = 12 - 4 = 8
        let hi = chunk.code[site + 2];
        let lo = chunk.code[site + 3];
        let delta = i16::from_be_bytes([hi, lo]);
        assert_eq!(delta, 8);
    }

    #[test]
    fn opcode_roundtrip() {
        for byte in 0..=0xFF {
            if let Some(op) = Op::from_u8(byte) {
                assert_eq!(op as u8, byte);
            }
        }
    }

    #[test]
    fn emit_send_sequence() {
        // [obj foo: arg1 arg2] =>
        //   SEND dst=r0, recv=r1, sel_const=idx
        //   (nargs encoded separately by compiler)
        let mut chunk = Chunk::new("test-send", 0, 8);
        let sel = chunk.add_constant(0xF00);
        chunk.emit(Op::Send, 0, 1, sel as u8);
        chunk.emit(Op::Return, 0, 0, 0);
        assert_eq!(chunk.code.len(), 8);
    }
}
