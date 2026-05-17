//! V4 bytecode encoder / decoder.
//!
//! the byte format is defined by spec §4 of
//! `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`. all
//! multi-byte operands are big-endian (network order), fixed-width.
//!
//! per-op byte sizes (spec §3):
//!   PushNil/PushTrue/PushFalse/Pop/Dup/LoadSelf/LoadHere/Return    1 byte
//!   LoadConst                                                       3 bytes
//!   LoadName                                                        5 bytes
//!   Send/SuperSend/SendSelf/SendHere                                8 bytes
//!   TailSend/TailSendSelf/TailSendHere                              6 bytes
//!   SendDynamic                                                     4 bytes
//!   Jump/JumpIfFalse/JumpIfTrue                                     3 bytes
//!   PushClosure                                                     5 bytes
//!   Suspend                                                         3 bytes
//!   Resume                                                          3 bytes

const std = @import("std");
const opcodes = @import("opcodes.zig");
const form = @import("form.zig");

const Tag = opcodes.Tag;
const Op = opcodes.Op;
const FormId = form.FormId;

/// errors raised by `decodeOp`.
pub const DecodeError = error{
    /// the buffer ended mid-op.
    Truncated,
    /// the tag byte didn't match any V4 opcode (spec §4.1: 0x00 and
    /// 0x60+ are reserved / never emitted).
    UnknownTag,
};

// ============================================================
// primitive writers (big-endian, per spec §4.2)
// ============================================================

pub fn writeU8(buf: *std.ArrayList(u8), allocator: std.mem.Allocator, v: u8) !void {
    try buf.append(allocator, v);
}

pub fn writeU16BE(buf: *std.ArrayList(u8), allocator: std.mem.Allocator, v: u16) !void {
    try buf.append(allocator, @intCast((v >> 8) & 0xFF));
    try buf.append(allocator, @intCast(v & 0xFF));
}

pub fn writeU32BE(buf: *std.ArrayList(u8), allocator: std.mem.Allocator, v: u32) !void {
    try buf.append(allocator, @intCast((v >> 24) & 0xFF));
    try buf.append(allocator, @intCast((v >> 16) & 0xFF));
    try buf.append(allocator, @intCast((v >> 8) & 0xFF));
    try buf.append(allocator, @intCast(v & 0xFF));
}

pub fn writeI16BE(buf: *std.ArrayList(u8), allocator: std.mem.Allocator, v: i16) !void {
    // i16 → u16 via bit-cast preserves the two's-complement bit pattern,
    // which is exactly what the BE encoding requires.
    const u: u16 = @bitCast(v);
    try writeU16BE(buf, allocator, u);
}

// ============================================================
// primitive readers (big-endian, per spec §4.2)
// ============================================================

pub fn readU8(bytes: []const u8, pc: usize) DecodeError!u8 {
    if (pc + 1 > bytes.len) return DecodeError.Truncated;
    return bytes[pc];
}

pub fn readU16BE(bytes: []const u8, pc: usize) DecodeError!u16 {
    if (pc + 2 > bytes.len) return DecodeError.Truncated;
    return (@as(u16, bytes[pc]) << 8) | @as(u16, bytes[pc + 1]);
}

pub fn readU32BE(bytes: []const u8, pc: usize) DecodeError!u32 {
    if (pc + 4 > bytes.len) return DecodeError.Truncated;
    return (@as(u32, bytes[pc]) << 24) |
        (@as(u32, bytes[pc + 1]) << 16) |
        (@as(u32, bytes[pc + 2]) << 8) |
        @as(u32, bytes[pc + 3]);
}

pub fn readI16BE(bytes: []const u8, pc: usize) DecodeError!i16 {
    const u = try readU16BE(bytes, pc);
    return @bitCast(u);
}

// ============================================================
// encode
// ============================================================

/// append the byte-encoded form of `op` to `buf`. layout per spec §3
/// / §4 (tag byte, then operands in declared order, big-endian).
pub fn encodeOp(
    op: Op,
    buf: *std.ArrayList(u8),
    allocator: std.mem.Allocator,
) !void {
    // tag first — derived from the union's active variant.
    try writeU8(buf, allocator, @intFromEnum(@as(Tag, op)));

    switch (op) {
        // 1-byte ops: just the tag, no operands.
        .push_nil,
        .push_true,
        .push_false,
        .pop,
        .dup,
        .load_self,
        .load_here,
        .return_op,
        => {},

        // LoadConst: tag + u16 idx = 3 bytes (spec §3.1).
        .load_const => |p| {
            try writeU16BE(buf, allocator, p.idx);
        },

        // LoadName: tag + u32 SymId = 5 bytes (spec §3.1).
        .load_name => |p| {
            try writeU32BE(buf, allocator, p.name);
        },

        // Send / SuperSend / SendSelf / SendHere: tag + u32 sel + u8 argc + u16 ic = 8 bytes (spec §3.3).
        .send, .super_send, .send_self, .send_here => |p| {
            try writeU32BE(buf, allocator, p.selector);
            try writeU8(buf, allocator, p.argc);
            try writeU16BE(buf, allocator, p.ic_idx);
        },

        // TailSend / TailSendSelf / TailSendHere: tag + u32 sel + u8 argc = 6 bytes (spec §3.3).
        .tail_send, .tail_send_self, .tail_send_here => |p| {
            try writeU32BE(buf, allocator, p.selector);
            try writeU8(buf, allocator, p.argc);
        },

        // SendDynamic: tag + u8 argc + u16 ic = 4 bytes (spec §3.3).
        .send_dynamic => |p| {
            try writeU8(buf, allocator, p.argc);
            try writeU16BE(buf, allocator, p.ic_idx);
        },

        // Jump / JumpIfFalse / JumpIfTrue: tag + i16 offset = 3 bytes (spec §3.4).
        .jump, .jump_if_false, .jump_if_true => |p| {
            try writeI16BE(buf, allocator, p.offset);
        },

        // PushClosure: tag + u32 FormId = 5 bytes (spec §3.5).
        .push_closure => |p| {
            try writeU32BE(buf, allocator, p.chunk.toU32());
        },

        // Suspend: tag + u16 promise-ic = 3 bytes (spec §3.6).
        .suspend_op => |p| {
            try writeU16BE(buf, allocator, p.promise_ic);
        },

        // Resume: tag + u16 frame-ic = 3 bytes (spec §3.6).
        .resume_op => |p| {
            try writeU16BE(buf, allocator, p.frame_ic);
        },
    }
}

// ============================================================
// decode
// ============================================================

/// decode one op starting at `bytes[pc]`. returns the op + the number
/// of bytes consumed (always equal to that op's encoded size per
/// spec §3).
///
/// raises `Truncated` if the buffer ends mid-op or `UnknownTag` if
/// the tag byte doesn't match a V4 opcode.
/// safely convert a tag byte to a `Tag` enum value, returning
/// `UnknownTag` if the byte isn't a defined V4 opcode. zig 0.16's
/// std no longer ships `meta.intToEnum`, so we do the check by
/// walking the declared enum fields once.
fn tagFromByte(b: u8) DecodeError!Tag {
    inline for (@typeInfo(Tag).@"enum".fields) |f| {
        if (f.value == b) return @field(Tag, f.name);
    }
    return DecodeError.UnknownTag;
}

pub fn decodeOp(
    bytes: []const u8,
    pc: usize,
) (DecodeError)!struct { op: Op, advance: usize } {
    const tag_byte = try readU8(bytes, pc);
    const tag = try tagFromByte(tag_byte);

    return switch (tag) {
        .push_nil => .{ .op = .push_nil, .advance = 1 },
        .push_true => .{ .op = .push_true, .advance = 1 },
        .push_false => .{ .op = .push_false, .advance = 1 },
        .pop => .{ .op = .pop, .advance = 1 },
        .dup => .{ .op = .dup, .advance = 1 },
        .load_self => .{ .op = .load_self, .advance = 1 },
        .load_here => .{ .op = .load_here, .advance = 1 },
        .return_op => .{ .op = .return_op, .advance = 1 },

        .load_const => blk: {
            const idx = try readU16BE(bytes, pc + 1);
            break :blk .{ .op = .{ .load_const = .{ .idx = idx } }, .advance = 3 };
        },

        .load_name => blk: {
            const name = try readU32BE(bytes, pc + 1);
            break :blk .{ .op = .{ .load_name = .{ .name = name } }, .advance = 5 };
        },

        .send, .super_send, .send_self, .send_here => blk: {
            const sel = try readU32BE(bytes, pc + 1);
            const argc = try readU8(bytes, pc + 5);
            const ic = try readU16BE(bytes, pc + 6);
            const payload: opcodes.Send = .{ .selector = sel, .argc = argc, .ic_idx = ic };
            const op: Op = switch (tag) {
                .send => .{ .send = payload },
                .super_send => .{ .super_send = payload },
                .send_self => .{ .send_self = payload },
                .send_here => .{ .send_here = payload },
                else => unreachable,
            };
            break :blk .{ .op = op, .advance = 8 };
        },

        .tail_send, .tail_send_self, .tail_send_here => blk: {
            const sel = try readU32BE(bytes, pc + 1);
            const argc = try readU8(bytes, pc + 5);
            const payload: opcodes.TailSend = .{ .selector = sel, .argc = argc };
            const op: Op = switch (tag) {
                .tail_send => .{ .tail_send = payload },
                .tail_send_self => .{ .tail_send_self = payload },
                .tail_send_here => .{ .tail_send_here = payload },
                else => unreachable,
            };
            break :blk .{ .op = op, .advance = 6 };
        },

        .send_dynamic => blk: {
            const argc = try readU8(bytes, pc + 1);
            const ic = try readU16BE(bytes, pc + 2);
            break :blk .{
                .op = .{ .send_dynamic = .{ .argc = argc, .ic_idx = ic } },
                .advance = 4,
            };
        },

        .jump, .jump_if_false, .jump_if_true => blk: {
            const off = try readI16BE(bytes, pc + 1);
            const payload: opcodes.Jump = .{ .offset = off };
            const op: Op = switch (tag) {
                .jump => .{ .jump = payload },
                .jump_if_false => .{ .jump_if_false = payload },
                .jump_if_true => .{ .jump_if_true = payload },
                else => unreachable,
            };
            break :blk .{ .op = op, .advance = 3 };
        },

        .push_closure => blk: {
            const raw = try readU32BE(bytes, pc + 1);
            break :blk .{
                .op = .{ .push_closure = .{ .chunk = FormId.fromU32(raw) } },
                .advance = 5,
            };
        },

        .suspend_op => blk: {
            const v = try readU16BE(bytes, pc + 1);
            break :blk .{ .op = .{ .suspend_op = .{ .promise_ic = v } }, .advance = 3 };
        },

        .resume_op => blk: {
            const v = try readU16BE(bytes, pc + 1);
            break :blk .{ .op = .{ .resume_op = .{ .frame_ic = v } }, .advance = 3 };
        },
    };
}

// ============================================================
// tests
// ============================================================

/// helper: encode one op, decode it back, expect identical operand bytes
/// and the declared advance count for that op (spec §3).
fn expectRoundtrip(op: Op, expected_size: usize) !void {
    const allocator = std.testing.allocator;
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);

    try encodeOp(op, &buf, allocator);
    try std.testing.expectEqual(expected_size, buf.items.len);

    const decoded = try decodeOp(buf.items, 0);
    try std.testing.expectEqual(expected_size, decoded.advance);

    // compare via canonical-byte-encoding: re-encode the decoded op
    // and check byte-identical. this dodges union-equality
    // complications and is the strongest possible roundtrip check.
    var buf2: std.ArrayList(u8) = .empty;
    defer buf2.deinit(allocator);
    try encodeOp(decoded.op, &buf2, allocator);
    try std.testing.expectEqualSlices(u8, buf.items, buf2.items);
}

test "primitive BE read/write roundtrip" {
    const allocator = std.testing.allocator;
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);

    try writeU8(&buf, allocator, 0xAB);
    try writeU16BE(&buf, allocator, 0x1234);
    try writeU32BE(&buf, allocator, 0xDEADBEEF);
    try writeI16BE(&buf, allocator, -100);

    // check the actual byte layout is big-endian
    try std.testing.expectEqualSlices(u8, &.{
        0xAB,
        0x12, 0x34,
        0xDE, 0xAD, 0xBE, 0xEF,
        0xFF, 0x9C, // -100 in two's-complement big-endian
    }, buf.items);

    try std.testing.expectEqual(@as(u8, 0xAB), try readU8(buf.items, 0));
    try std.testing.expectEqual(@as(u16, 0x1234), try readU16BE(buf.items, 1));
    try std.testing.expectEqual(@as(u32, 0xDEADBEEF), try readU32BE(buf.items, 3));
    try std.testing.expectEqual(@as(i16, -100), try readI16BE(buf.items, 7));
}

test "encoded Send matches spec §4.2 example" {
    // spec §4.2: Send {selector=0x1234abcd, argc=2, ic_idx=4} encodes as
    //   0x20  0x12 0x34 0xab 0xcd  0x02  0x00 0x04
    const allocator = std.testing.allocator;
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);

    try encodeOp(.{ .send = .{ .selector = 0x1234abcd, .argc = 2, .ic_idx = 4 } }, &buf, allocator);
    try std.testing.expectEqualSlices(u8, &.{
        0x20, 0x12, 0x34, 0xab, 0xcd, 0x02, 0x00, 0x04,
    }, buf.items);
}

test "roundtrip every opcode" {
    // sample operands per the task brief: SymId 12345, argc 2, ic_idx 5,
    // offset -100. add a few extras to keep coverage real.
    const sel: opcodes.SymId = 12345;
    const argc: u8 = 2;
    const ic: u16 = 5;
    const off: i16 = -100;

    // 1-byte ops (8)
    try expectRoundtrip(.push_nil, 1);
    try expectRoundtrip(.push_true, 1);
    try expectRoundtrip(.push_false, 1);
    try expectRoundtrip(.load_self, 1);
    try expectRoundtrip(.load_here, 1);
    try expectRoundtrip(.pop, 1);
    try expectRoundtrip(.dup, 1);
    try expectRoundtrip(.return_op, 1);

    // LoadConst (3 bytes)
    try expectRoundtrip(.{ .load_const = .{ .idx = 0xBEEF } }, 3);

    // LoadName (5 bytes)
    try expectRoundtrip(.{ .load_name = .{ .name = sel } }, 5);

    // 8-byte sends (4 variants)
    const send_payload: opcodes.Send = .{ .selector = sel, .argc = argc, .ic_idx = ic };
    try expectRoundtrip(.{ .send = send_payload }, 8);
    try expectRoundtrip(.{ .super_send = send_payload }, 8);
    try expectRoundtrip(.{ .send_self = send_payload }, 8);
    try expectRoundtrip(.{ .send_here = send_payload }, 8);

    // 6-byte tail sends (3 variants)
    const tail_payload: opcodes.TailSend = .{ .selector = sel, .argc = argc };
    try expectRoundtrip(.{ .tail_send = tail_payload }, 6);
    try expectRoundtrip(.{ .tail_send_self = tail_payload }, 6);
    try expectRoundtrip(.{ .tail_send_here = tail_payload }, 6);

    // SendDynamic (4 bytes — no selector)
    try expectRoundtrip(.{ .send_dynamic = .{ .argc = argc, .ic_idx = ic } }, 4);

    // jumps (3 bytes each) — exercise negative + positive + extremes
    try expectRoundtrip(.{ .jump = .{ .offset = off } }, 3);
    try expectRoundtrip(.{ .jump_if_false = .{ .offset = off } }, 3);
    try expectRoundtrip(.{ .jump_if_true = .{ .offset = 32767 } }, 3);
    try expectRoundtrip(.{ .jump = .{ .offset = -32768 } }, 3);

    // PushClosure (5 bytes) — exercise non-default scope to confirm
    // the packed-struct → u32 roundtrip preserves the scope tag.
    try expectRoundtrip(
        .{ .push_closure = .{ .chunk = FormId.vatLocal(0x123456) } },
        5,
    );
    try expectRoundtrip(
        .{ .push_closure = .{ .chunk = .{ .payload = 7, .scope = .far_ref } } },
        5,
    );

    // scheduling (3 bytes each)
    try expectRoundtrip(.{ .suspend_op = .{ .promise_ic = 42 } }, 3);
    try expectRoundtrip(.{ .resume_op = .{ .frame_ic = 99 } }, 3);
}

test "decode catches unknown tag" {
    const bad = [_]u8{0x00};
    try std.testing.expectError(DecodeError.UnknownTag, decodeOp(&bad, 0));
    const reserved = [_]u8{0x6F};
    try std.testing.expectError(DecodeError.UnknownTag, decodeOp(&reserved, 0));
}

test "decode catches truncated buffer" {
    // a Send op needs 8 bytes; give it 4.
    const truncated = [_]u8{ 0x20, 0x12, 0x34, 0xab };
    try std.testing.expectError(DecodeError.Truncated, decodeOp(&truncated, 0));
    // bare LoadConst tag with nothing after.
    const bare = [_]u8{0x04};
    try std.testing.expectError(DecodeError.Truncated, decodeOp(&bare, 0));
}

test "decodeOp respects pc offset (stream decoding)" {
    // encode a small stream: PushTrue ; LoadConst 7 ; Return
    const allocator = std.testing.allocator;
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);

    try encodeOp(.push_true, &buf, allocator);
    try encodeOp(.{ .load_const = .{ .idx = 7 } }, &buf, allocator);
    try encodeOp(.return_op, &buf, allocator);

    var pc: usize = 0;
    const a = try decodeOp(buf.items, pc);
    try std.testing.expectEqual(Tag.push_true, @as(Tag, a.op));
    pc += a.advance;
    const b = try decodeOp(buf.items, pc);
    try std.testing.expectEqual(Tag.load_const, @as(Tag, b.op));
    try std.testing.expectEqual(@as(u16, 7), b.op.load_const.idx);
    pc += b.advance;
    const c = try decodeOp(buf.items, pc);
    try std.testing.expectEqual(Tag.return_op, @as(Tag, c.op));
    pc += c.advance;
    try std.testing.expectEqual(buf.items.len, pc);
}
