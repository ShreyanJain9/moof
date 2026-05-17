//! vat-mode unit tests — covers VatMode field, auto-freeze hook,
//! isFreezable predicate, and __alloc-mutable__ bypass.

const std = @import("std");
const testing = std.testing;
const World = @import("world.zig").World;
const VatMode = @import("world.zig").VatMode;

// ── B2: VatMode field default + settability ──────────────────────

test "vat_mode defaults to mutable_default" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    try testing.expect(world.vat_mode == .mutable_default);
}

test "vat_mode is settable to frozen_default" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.vat_mode = .frozen_default;
    try testing.expect(world.vat_mode == .frozen_default);
}

// ── B3: auto-freeze on alloc when vat_mode is frozen_default ─────

test "alloc in frozen_default mode yields frozen form" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.vat_mode = .frozen_default;
    const id = try world.allocInstance(world.protos.object);
    try testing.expect(world.heap.get(id).frozen);
}

test "alloc in mutable_default mode yields mutable form" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.vat_mode = .mutable_default;
    const id = try world.allocInstance(world.protos.object);
    try testing.expect(!world.heap.get(id).frozen);
}

// ── B4: isFreezable predicate ─────────────────────────────────────

test "isFreezable returns true for fresh mutable form" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const id = try world.allocInstance(world.protos.object);
    try testing.expect(world.isFreezable(id));
}

test "isFreezable returns false for already-frozen form" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const id = try world.allocInstance(world.protos.object);
    world.heap.getMut(id).frozen = true;
    try testing.expect(!world.isFreezable(id));
}

test "isFreezable returns false for foreign-handle forms" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    // ForeignHandle forms have proto = world.protos.foreign_handle.
    const fh_id = try world.allocInstance(world.protos.foreign_handle);
    try testing.expect(!world.isFreezable(fh_id));
}

// ── B5: allocMutableBypass ignores vat_mode ───────────────────────

test "allocMutableBypass in frozen_default mode still yields mutable form" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.vat_mode = .frozen_default;
    const id = try world.allocMutableBypass(world.protos.object);
    try testing.expect(!world.heap.get(id).frozen);
}
