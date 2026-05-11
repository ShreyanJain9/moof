//! V4 opcode set — tagged-union representation in zig.
//!
//! the source of truth is `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`,
//! specifically §2 (overview), §3 (per-opcode reference), and §4 (byte encoding).
//! every tag byte, operand ordering, and operand width here matches that
//! spec verbatim.
//!
//! the in-memory `Op` is a tagged union — convenient to construct and
//! pattern-match. the on-disk encoding (per spec §4) is fixed-width
//! big-endian and lives in `bytecode.zig`.

const std = @import("std");
const form = @import("form.zig");

/// FormId — re-exported for convenience. owned by `form.zig`.
pub const FormId = form.FormId;

/// symbol-table index. always a u32 (spec §3 operand types).
pub const SymId = u32;

/// V4 opcode tag byte. ranges per spec §4.1:
///   0x01–0x0F  value-load
///   0x10–0x1F  stack
///   0x20–0x2F  sends
///   0x30–0x3F  control flow
///   0x40–0x4F  closures
///   0x50–0x5F  scheduling
///   0x60–0xFF  reserved
pub const Tag = enum(u8) {
    // value-load (spec §3.1)
    push_nil = 0x01,
    push_true = 0x02,
    push_false = 0x03,
    load_const = 0x04,
    load_self = 0x05,
    load_here = 0x06,
    load_name = 0x07,

    // stack (spec §3.2)
    pop = 0x10,
    dup = 0x11,

    // sends (spec §3.3)
    send = 0x20,
    tail_send = 0x21,
    super_send = 0x22,
    send_dynamic = 0x23,
    send_self = 0x24,
    send_here = 0x25,
    tail_send_self = 0x26,
    tail_send_here = 0x27,

    // control flow (spec §3.4)
    jump = 0x30,
    jump_if_false = 0x31,
    jump_if_true = 0x32,
    return_op = 0x33,

    // closures (spec §3.5)
    push_closure = 0x40,

    // scheduling (spec §3.6)
    suspend_op = 0x50,
    resume_op = 0x51,
};

/// LoadConst {idx: u16} — spec §3.1.
pub const LoadConst = struct { idx: u16 };

/// LoadName {name: SymId} — spec §3.1.
pub const LoadName = struct { name: SymId };

/// Send {selector: SymId, argc: u8, ic_idx: u16} — spec §3.3.
/// shared layout for Send / SuperSend / SendSelf / SendHere.
pub const Send = struct { selector: SymId, argc: u8, ic_idx: u16 };

/// SendDynamic {argc: u8, ic_idx: u16} — spec §3.3.
/// no selector field; selector is taken from the stack.
pub const SendDynamic = struct { argc: u8, ic_idx: u16 };

/// TailSend {selector: SymId, argc: u8} — spec §3.3.
/// shared layout for TailSend / TailSendSelf / TailSendHere.
/// note: tail variants currently lack ICs (spec §6.2 — flagged wart).
pub const TailSend = struct { selector: SymId, argc: u8 };

/// Jump / JumpIfFalse / JumpIfTrue — spec §3.4. signed i16 offset.
pub const Jump = struct { offset: i16 };

/// PushClosure {chunk: FormId} — spec §3.5.
pub const PushClosure = struct { chunk: FormId };

/// Suspend {promise_ic: u16} — spec §3.6.
pub const Suspend = struct { promise_ic: u16 };

/// Resume {frame_ic: u16} — spec §3.6.
pub const Resume = struct { frame_ic: u16 };

/// the in-memory opcode representation. one variant per V4 op.
///
/// the variant tag exactly matches the on-disk byte tag (`Tag` enum)
/// so we can use `@intFromEnum` / `@enumFromInt` interchangeably with
/// the byte encoder/decoder in `bytecode.zig`.
pub const Op = union(Tag) {
    // value-load
    push_nil,
    push_true,
    push_false,
    load_const: LoadConst,
    load_self,
    load_here,
    load_name: LoadName,

    // stack
    pop,
    dup,

    // sends
    send: Send,
    tail_send: TailSend,
    super_send: Send,
    send_dynamic: SendDynamic,
    send_self: Send,
    send_here: Send,
    tail_send_self: TailSend,
    tail_send_here: TailSend,

    // control flow
    jump: Jump,
    jump_if_false: Jump,
    jump_if_true: Jump,
    return_op,

    // closures
    push_closure: PushClosure,

    // scheduling
    suspend_op: Suspend,
    resume_op: Resume,
};

/// returns true iff `op` leaves +1 on the operand stack in isolation
/// (no pops besides what its declared stack effect already accounts for
/// as a *net* push).
///
/// per spec §3, the pure-pushers are:
///   PushNil, PushTrue, PushFalse, LoadConst, LoadSelf, LoadHere,
///   LoadName, Dup, PushClosure.
///
/// send variants are NOT in this set: they pop receiver+args then push
/// one result. their *net* effect is +1 but in isolation they consume
/// inputs from the stack, so they don't fit the "pure push" notion the
/// stack-balance checker (spec §6.4) needs.
pub fn pushes(op: Op) bool {
    return switch (op) {
        .push_nil,
        .push_true,
        .push_false,
        .load_const,
        .load_self,
        .load_here,
        .load_name,
        .dup,
        .push_closure,
        => true,
        else => false,
    };
}

test "Tag bytes match spec §2" {
    try std.testing.expectEqual(@as(u8, 0x01), @intFromEnum(Tag.push_nil));
    try std.testing.expectEqual(@as(u8, 0x04), @intFromEnum(Tag.load_const));
    try std.testing.expectEqual(@as(u8, 0x07), @intFromEnum(Tag.load_name));
    try std.testing.expectEqual(@as(u8, 0x10), @intFromEnum(Tag.pop));
    try std.testing.expectEqual(@as(u8, 0x20), @intFromEnum(Tag.send));
    try std.testing.expectEqual(@as(u8, 0x23), @intFromEnum(Tag.send_dynamic));
    try std.testing.expectEqual(@as(u8, 0x27), @intFromEnum(Tag.tail_send_here));
    try std.testing.expectEqual(@as(u8, 0x30), @intFromEnum(Tag.jump));
    try std.testing.expectEqual(@as(u8, 0x33), @intFromEnum(Tag.return_op));
    try std.testing.expectEqual(@as(u8, 0x40), @intFromEnum(Tag.push_closure));
    try std.testing.expectEqual(@as(u8, 0x50), @intFromEnum(Tag.suspend_op));
    try std.testing.expectEqual(@as(u8, 0x51), @intFromEnum(Tag.resume_op));
}

test "pushes flags pure-push ops" {
    try std.testing.expect(pushes(.push_nil));
    try std.testing.expect(pushes(.push_true));
    try std.testing.expect(pushes(.push_false));
    try std.testing.expect(pushes(.load_self));
    try std.testing.expect(pushes(.load_here));
    try std.testing.expect(pushes(.dup));
    try std.testing.expect(pushes(.{ .load_const = .{ .idx = 3 } }));
    try std.testing.expect(pushes(.{ .load_name = .{ .name = 99 } }));
    try std.testing.expect(pushes(.{ .push_closure = .{ .chunk = FormId.vatLocal(1) } }));

    // sends + control flow + scheduling + stack-pop are NOT pure pushes
    try std.testing.expect(!pushes(.pop));
    try std.testing.expect(!pushes(.return_op));
    try std.testing.expect(!pushes(.{ .send = .{ .selector = 1, .argc = 0, .ic_idx = 0 } }));
    try std.testing.expect(!pushes(.{ .send_self = .{ .selector = 1, .argc = 0, .ic_idx = 0 } }));
    try std.testing.expect(!pushes(.{ .jump = .{ .offset = 0 } }));
    try std.testing.expect(!pushes(.{ .jump_if_true = .{ .offset = 0 } }));
    try std.testing.expect(!pushes(.{ .suspend_op = .{ .promise_ic = 0 } }));
}
