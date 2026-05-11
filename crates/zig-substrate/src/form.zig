//! form.zig — Form heap + FormId.
//!
//! NOTE (V4 polyglot migration): the canonical owner of this file is
//! another track-A agent. this is a minimal stub mirroring main.zig's
//! current FormId shape so that opcodes.zig / bytecode.zig compile and
//! their roundtrip tests run self-contained. when the form-heap agent
//! lands their version, this file should be overwritten — `FormId`'s
//! wire shape (a 32-bit value with the V0 2-bit scope tag in the top
//! bits) is the integration contract.

const std = @import("std");

/// the universal heap-id. matches the rust `FormId` layout: 2-bit
/// scope tag in the top, 30-bit payload below. derived from the V0
/// scope-tagging design.
pub const FormId = packed struct(u32) {
    payload: u30,
    scope: Scope,

    pub const Scope = enum(u2) {
        vat_local = 0b00,
        shared = 0b01,
        far_ref = 0b10,
        reserved = 0b11,
    };

    pub const NONE: FormId = .{ .payload = 0, .scope = .vat_local };

    pub fn isNone(self: FormId) bool {
        return self.payload == 0 and self.scope == .vat_local;
    }

    pub fn vatLocal(payload: u30) FormId {
        return .{ .payload = payload, .scope = .vat_local };
    }

    /// reinterpret the FormId as its packed u32 representation.
    /// big-endian byte encoders use this for the on-disk form per
    /// spec §4.2 (FormId as u32).
    pub fn toU32(self: FormId) u32 {
        return @bitCast(self);
    }

    /// inverse of toU32 — reinterpret a u32 as a FormId.
    pub fn fromU32(x: u32) FormId {
        return @bitCast(x);
    }
};
