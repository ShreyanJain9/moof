/// Bytecode opcodes for the MOOF VM.
///
/// Stack-based. The bytecode is the canonical representation — what gets
/// serialized in the image, what introspection operates on (§9.2).
///
/// The 6 kernel forms (vau, send, def, quote, cons, eq) each have opcodes.
/// Everything else compiles to combinations of these.

// ── Stack manipulation ──
/// Push a constant from the chunk's constant pool. Arg: u16 index.
pub const OP_CONST: u8 = 0x01;
/// Push nil.
pub const OP_NIL: u8 = 0x02;
/// Push true.
pub const OP_TRUE: u8 = 0x03;
/// Push false.
pub const OP_FALSE: u8 = 0x04;
/// Pop and discard top of stack.
pub const OP_POP: u8 = 0x05;

// ── Environment ──
/// Look up a name in the current environment and push it. Arg: u16 constant index (symbol).
pub const OP_LOOKUP: u8 = 0x10;
/// Bind a value in the current environment. Arg: u16 constant index (symbol).
/// Pops value from stack.
pub const OP_DEF: u8 = 0x11;
/// Push the current environment as a value.
pub const OP_GET_ENV: u8 = 0x12;

// ── The six kernel primitives ──

/// `send` — the VM's dispatch instruction.
/// Arg: u16 constant index (selector symbol), u8 arg count.
/// Stack: [receiver, arg1, arg2, ...argN] → [result]
pub const OP_SEND: u8 = 0x20;

/// `cons` — construct a pair.
/// Stack: [car, cdr] → [cons-cell]
pub const OP_CONS: u8 = 0x21;

/// `eq` — identity comparison.
/// Stack: [a, b] → [boolean]
pub const OP_EQ: u8 = 0x22;

/// `quote` — push a quoted value (literal AST).
/// Arg: u16 constant index.
pub const OP_QUOTE: u8 = 0x23;

/// `vau` — create an operative.
/// Arg: u16 constant index for params, u16 constant index for env_param symbol,
///      u16 constant index for body chunk object id, u16 constant index for source AST.
/// Captures the current environment.
pub const OP_VAU: u8 = 0x24;

// ── Control flow ──
/// Call a callable. Arg: u8 arg count.
/// Stack: [callable, arg1, ...argN] → [result]
pub const OP_CALL: u8 = 0x30;

/// Return from the current frame. Pops the return value.
pub const OP_RETURN: u8 = 0x31;

/// Jump forward. Arg: u16 offset.
pub const OP_JUMP: u8 = 0x32;

/// Jump forward if top of stack is falsey. Arg: u16 offset. Pops condition.
pub const OP_JUMP_IF_FALSE: u8 = 0x33;

/// Backward jump for loops. Arg: u16 distance to subtract from current ip.
pub const OP_LOOP_BACK: u8 = 0x34;

// ── Operatives: raw (unevaluated) call ──
/// Call an operative with unevaluated arguments.
/// Arg: u8 arg count.
pub const OP_CALL_OPERATIVE: u8 = 0x40;

// ── Generic apply ──
/// Generic apply: checks at runtime whether target is operative or applicative.
/// Stack: [callable, quoted_args_list] → [result]
pub const OP_APPLY: u8 = 0x41;

/// Tail-call variant of OP_APPLY. Replaces current frame instead of pushing new one.
pub const OP_TAIL_APPLY: u8 = 0x42;

/// Tail-call variant of OP_CALL. Replaces current frame for known-lambda calls.
pub const OP_TAIL_CALL: u8 = 0x35;

// ── Built-in operations ──
/// Evaluate an expression in the current environment.
pub const OP_EVAL: u8 = 0x50;
/// Print a value to the transcript.
pub const OP_PRINT: u8 = 0x51;
/// List operations (car, cdr) as opcodes for efficiency.
pub const OP_CAR: u8 = 0x52;
pub const OP_CDR: u8 = 0x53;
/// Type checking. Stack: [value] → [symbol]
pub const OP_TYPE_OF: u8 = 0x54;
/// Load and evaluate a file. Stack: [path_string] → [result]
pub const OP_LOAD: u8 = 0x55;
/// Get the source AST of a lambda/operative. Stack: [callable] → [ast_or_nil]
pub const OP_SOURCE: u8 = 0x56;

// ── Object construction ──
/// Create a new GeneralObject. Arg: u8 slot_count.
/// Stack: [parent, key1, val1, key2, val2, ...] → [object]
pub const OP_MAKE_OBJECT: u8 = 0x60;

/// Add a handler to an object.
/// Stack: [object, selector_symbol, handler_lambda] → [object]
pub const OP_HANDLE: u8 = 0x61;

/// Direct slot access on an object.
/// Stack: [object, symbol] → [value]
pub const OP_SLOT_GET: u8 = 0x62;

/// Direct slot mutation on an object.
/// Stack: [object, symbol, value] → [value]
pub const OP_SLOT_SET: u8 = 0x63;

/// Primitive-only send — bypasses handler lookup, goes directly to fast path.
/// Used by native handler lambdas to avoid infinite recursion.
/// Same encoding as OP_SEND: u16 selector constant, u8 argc.
pub const OP_PRIM_SEND: u8 = 0x64;

/// Open a native library. Stack: [name_string] → [library_object]
pub const OP_FFI_OPEN: u8 = 0x70;
/// Bind a foreign function via the native registry. Stack: [lib, name_string, arg_types_list, ret_type_sym] → [native_fn]
pub const OP_FFI_BIND: u8 = 0x71;

/// Read a u16 from bytecode at the given offset (big-endian).
pub fn read_u16(code: &[u8], offset: usize) -> u16 {
    ((code[offset] as u16) << 8) | (code[offset + 1] as u16)
}
