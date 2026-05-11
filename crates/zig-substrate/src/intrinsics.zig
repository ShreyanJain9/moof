//! moof-zig — primordial native methods.
//!
//! V4 task A.6. installed at `World.init()`, before any moof source
//! loads. ports the minimal-viable-subset (~30 natives) from
//! `crates/substrate/src/intrinsics.rs`; the rest are derived in moof
//! (lib/bootstrap.moof + friends) once the seed-emitted bytecode runs.
//!
//! installed surface (see plan §A.6 step 1):
//!
//! - Integer arithmetic: `:+ :- :* :/` (i48, overflow wraps; div-by-zero raises)
//! - Integer comparison: `:= :< :>`
//! - truthiness: `:!!` on Object / Nil / Bool
//! - identity / reflection: `:is :proto :identity` on Object
//! - slot access: `:slot: :slotSet!:` on Object
//! - cons: `:car :cdr` on Cons
//! - env (V3): `:bind:to: :set:to: :lookup: :parent :current` on Env
//! - closure (V3): `:callIn:withSelf:` — bypass closure's stored env
//! - `:become:` — heap-level indirection
//! - `:doesNotUnderstand:with:` — raises by default
//! - `:perform:withArgs:` — dynamic dispatch escape hatch
//! - `:ifTrue:ifFalse:` on Bool — branch via :call on chosen thunk
//! - `:toString` minimal — Integer + Object fallback
//!
//! everything else (List protocol, String/Table primitives, Char,
//! Float math, Console caps, Compiler primitives, etc.) is deferred
//! to phase β / phase γ tasks or to moof bootstrap.

const std = @import("std");
const value = @import("value.zig");
const Value = value.Value;
const form = @import("form.zig");
const FormId = form.FormId;
const Form = form.Form;
const world_mod = @import("world.zig");
const World = world_mod.World;
const NativeFn = world_mod.NativeFn;

// ─────────────────────────────────────────────────────────────────
// install — top-level entry. idempotent: safe to call once at world
// init. mirrors the structure of intrinsics.rs::install but keeps
// the call sites inline (no per-category helper fns) because the
// surface is small.
// ─────────────────────────────────────────────────────────────────

pub fn install(world: *World) !void {
    // arithmetic on Integer
    try installNative(world, world.protos.integer, "+", intPlus);
    try installNative(world, world.protos.integer, "-", intMinus);
    try installNative(world, world.protos.integer, "*", intMultiply);
    try installNative(world, world.protos.integer, "/", intDivide);

    // comparison on Integer
    try installNative(world, world.protos.integer, "=", intEq);
    try installNative(world, world.protos.integer, "<", intLt);
    try installNative(world, world.protos.integer, ">", intGt);

    // truthiness — everything-is-truthy default, with Nil/Bool overrides.
    try installNative(world, world.protos.object, "!!", objBangBang);
    try installNative(world, world.protos.nil, "!!", nilBangBang);
    try installNative(world, world.protos.bool_, "!!", boolBangBang);

    // identity / reflection on Object
    try installNative(world, world.protos.object, "is", objIs);
    try installNative(world, world.protos.object, "proto", objProto);
    try installNative(world, world.protos.object, "identity", objIdentity);

    // slot access on Object
    try installNative(world, world.protos.object, "slot:", objSlot);
    try installNative(world, world.protos.object, "slotSet!:", objSlotSet);

    // cons
    try installNative(world, world.protos.cons, "car", consCar);
    try installNative(world, world.protos.cons, "cdr", consCdr);

    // env (V3 task 6, port of substrate.rs::install_env_proto_methods)
    try installNative(world, world.protos.env, "bind:to:", envBindTo);
    try installNative(world, world.protos.env, "set:to:", envSetTo);
    try installNative(world, world.protos.env, "lookup:", envLookupTo);
    try installNative(world, world.protos.env, "parent", envParent);
    try installNative(world, world.protos.env, "current", envCurrent);

    // closure :callIn:withSelf: (V3 — explicit env + self)
    try installNative(world, world.protos.closure, "callIn:withSelf:", closureCallInWithSelf);

    // become: — heap indirection. self-become is a no-op.
    try installNative(world, world.protos.object, "become:", objBecome);

    // dispatch fallbacks
    try installNative(world, world.protos.object, "doesNotUnderstand:with:", objDoesNotUnderstand);
    try installNative(world, world.protos.object, "perform:withArgs:", objPerformWithArgs);

    // ifTrue:ifFalse: on Bool — branch + :call the chosen thunk.
    try installNative(world, world.protos.bool_, "ifTrue:ifFalse:", boolIfTrueIfFalse);

    // toString minimal — Integer + Object fallback.
    try installNative(world, world.protos.integer, "toString", intToString);
    try installNative(world, world.protos.object, "toString", objToString);
}

// ─────────────────────────────────────────────────────────────────
// installNative — alloc a method-Form (proto = protos.method),
// record the rust-side NativeFn in world.native_fns keyed by the
// method-Form's FormId, and install it on `proto`'s handlers table
// under `selector_name`.
//
// the "method-form-with-native-callback" trick mirrors substrate.rs's
// `World::install_native`: a method-Form whose body is implicit
// (lookup in `native_fns`) instead of bytecode. lets natives and
// moof methods share a single dispatch path.
// ─────────────────────────────────────────────────────────────────

fn installNative(
    world: *World,
    proto: FormId,
    selector_name: []const u8,
    native_fn: NativeFn,
) !void {
    const sel = try world.syms.intern(selector_name);
    var method_form = Form.init();
    method_form.proto = .{ .form = world.protos.method };
    const method_id = try world.heap.alloc(method_form);
    try world.native_fns.put(method_id, native_fn);
    // install on proto's handlers table.
    var proto_form = world.heap.getMut(proto);
    try proto_form.handlers.put(world.allocator, sel, .{ .form = method_id });
}

// ─────────────────────────────────────────────────────────────────
// error helpers — keep raise sites concise.
// ─────────────────────────────────────────────────────────────────

/// raise a type-error. mirrors intrinsics.rs::type_error.
fn typeError(world: *World, comptime msg: []const u8) anyerror {
    return world.raise("type-error", msg);
}

/// raise a generic error with `kind`. mirrors intrinsics.rs::raise.
fn raise(world: *World, comptime kind: []const u8, comptime msg: []const u8) anyerror {
    return world.raise(kind, msg);
}

// ─────────────────────────────────────────────────────────────────
// Integer arithmetic — port of intrinsics.rs::install_integer_methods.
//
// receiver is i48 (moof Integer); rhs must be Int (overflow wraps
// — the rust impl auto-promotes to Float when rhs is Float, but
// V4 phase α defers Float to lib/bootstrap.moof). div-by-zero raises.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_integer_methods `:+`
fn intPlus(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "+ expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "+ expected a numeric rhs");
    return .{ .int = a +% b };
}

// port of crates/substrate/src/intrinsics.rs::install_integer_methods `:-`
fn intMinus(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "- expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "- expected a numeric rhs");
    return .{ .int = a -% b };
}

// port of crates/substrate/src/intrinsics.rs::install_integer_methods `:*`
fn intMultiply(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "* expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "* expected a numeric rhs");
    return .{ .int = a *% b };
}

// port of crates/substrate/src/intrinsics.rs::install_integer_methods `:/`
fn intDivide(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "/ expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "/ expected a numeric rhs");
    if (b == 0) return raise(world, "division-by-zero", "integer division by zero");
    // wrapping_div: i48 has the same MIN/-1 quirk as i64, but moof
    // accepts it as wrapping per substrate.rs.
    return .{ .int = @divTrunc(a, b) };
}

// port of crates/substrate/src/intrinsics.rs::install_integer_methods `:=`
fn intEq(_: *World, self_: Value, args: []const Value) anyerror!Value {
    // defensive against proto-Form receivers (see rust comment at
    // intrinsics.rs:1371): if self isn't actually an Int, fall back
    // to identity comparison.
    const a_opt = self_.asInt();
    const b_opt = args[0].asInt();
    if (a_opt) |a| if (b_opt) |b| return .{ .bool_ = a == b };
    return .{ .bool_ = value.equals(self_, args[0]) };
}

// port of crates/substrate/src/intrinsics.rs::install_integer_methods `:<`
fn intLt(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "< expected an Integer receiver");
    const b = args[0].asInt() orelse return typeError(world, "< expected a numeric rhs");
    return .{ .bool_ = a < b };
}

// port of crates/substrate/src/intrinsics.rs::install_integer_methods `:>`
fn intGt(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "> expected an Integer receiver");
    const b = args[0].asInt() orelse return typeError(world, "> expected a numeric rhs");
    return .{ .bool_ = a > b };
}

// ─────────────────────────────────────────────────────────────────
// Truthiness — `:!!` coerces any Value to a Bool.
//
// Object → #true (default truthy).
// nil    → #false (the only non-Bool falsy value).
// Bool   → self  (#true and #false are their own coercions).
//
// the moof-side `lib/early/02-bool.moof` re-installs these via
// `setHandler!`. they're rust-side at boot because the seed-emitted
// `if` bytecode (lowered to `[c !!] ifTrue: t-thunk ifFalse: e-thunk`)
// dispatches `:!!` *before* early/02 loads.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_if_dispatch `:!!` on Object
fn objBangBang(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .{ .bool_ = true };
}

// port of crates/substrate/src/intrinsics.rs::install_if_dispatch `:!!` on Nil
fn nilBangBang(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .{ .bool_ = false };
}

// port of crates/substrate/src/intrinsics.rs::install_if_dispatch `:!!` on Bool
fn boolBangBang(_: *World, self_: Value, _: []const Value) anyerror!Value {
    return self_;
}

// ─────────────────────────────────────────────────────────────────
// Object reflection / identity primitives.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_object_reflection `:is`
// identity equality (same heap-id or same tagged-immediate).
fn objIs(_: *World, self_: Value, args: []const Value) anyerror!Value {
    return .{ .bool_ = value.equals(self_, args[0]) };
}

// port of crates/substrate/src/intrinsics.rs (Heap singleton `protoOf:` /
// the moof `:proto` defmethod that delegates there). returns the proto
// Value of a Form receiver. tagged immediates fall through to their
// proto-Form (e.g. Int → Integer-proto) via world.protoOf — matches
// rust's `proto_of` helper at world.rs:556.
fn objProto(world: *World, self_: Value, _: []const Value) anyerror!Value {
    return world.protoOf(self_);
}

// port of crates/substrate/src/intrinsics.rs (Heap singleton `heapIdOf:` /
// moof `:identity`). returns the FormId's raw payload as an Int — the
// stable identity number. for non-Forms (tagged immediates) returns 0,
// matching `Heap heapIdOf:`.
fn objIdentity(_: *World, self_: Value, _: []const Value) anyerror!Value {
    return switch (self_) {
        .form => |fid| .{ .int = @as(i48, @intCast(@as(u32, @bitCast(fid)))) },
        else => .{ .int = 0 },
    };
}

// ─────────────────────────────────────────────────────────────────
// Slot access — `:slot:` and `:slotSet!:` on Object.
//
// the rust impl exposes these as the (slot v 'name) and (slotSet! v
// 'name v) globals (intrinsics.rs:2395+) rather than proto methods —
// the moof side defines :slot: / :slotSet!: as defmethods that
// delegate. plan §A.6 step 1 puts them on Object directly to skip a
// layer of boot-time indirection.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs `(slot v 'name)`
fn objSlot(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const sym = args[0].asSym() orelse return typeError(world, "slot: name must be a Symbol");
    const id = self_.asFormId() orelse return .nil;
    return world.formSlot(id, sym);
}

// port of crates/substrate/src/intrinsics.rs `(slotSet! v 'name v)`
fn objSlotSet(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const sym = args[0].asSym() orelse return typeError(world, "slotSet!: name must be a Symbol");
    const id = self_.asFormId() orelse return typeError(world, "slotSet!: receiver must be a Form");
    const val = args[1];
    // form_slot_set raises `'frozen` if the form is sealed. just
    // propagate; the caller's error path handles it.
    try world.formSlotSet(id, sym, val);
    return val;
}

// ─────────────────────────────────────────────────────────────────
// Cons primitives — irreducible heap reads. List protocol (length,
// map, filter, reduce, …) is moof-only in stdlib/cons.moof.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_cons_and_nil_primitives `:car`
fn consCar(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return typeError(world, "car on non-Cons");
    return world.formSlot(id, world.symCar);
}

// port of crates/substrate/src/intrinsics.rs::install_cons_and_nil_primitives `:cdr`
fn consCdr(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return typeError(world, "cdr on non-Cons");
    return world.formSlot(id, world.symCdr);
}

// ─────────────────────────────────────────────────────────────────
// Env methods — port of intrinsics.rs::install_env_proto_methods.
// V3 spec §4.1 — non-walking `:bind:to:`, walking `:set:to:` (raises
// 'unbound on miss), walking `:lookup:` (nil on miss). plus `:parent`
// and the `[Env current]` class-method-style accessor.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_env_proto_methods `:bind:to:`
fn envBindTo(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":bind:to: receiver must be an Env Form");
    const name = args[0].asSym() orelse return typeError(world, ":bind:to: name must be a Symbol");
    const val = args[1];
    try world.envBind(env, name, val);
    return val;
}

// port of crates/substrate/src/intrinsics.rs::install_env_proto_methods `:set:to:`
fn envSetTo(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":set:to: receiver must be an Env Form");
    const name = args[0].asSym() orelse return typeError(world, ":set:to: name must be a Symbol");
    const val = args[1];
    const found = try world.envSet(env, name, val);
    if (!found) return raise(world, "unbound", "set!: name is unbound");
    return val;
}

// port of crates/substrate/src/intrinsics.rs::install_env_proto_methods `:lookup:`
fn envLookupTo(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":lookup: receiver must be an Env Form");
    const name = args[0].asSym() orelse return typeError(world, ":lookup: name must be a Symbol");
    return world.envLookup(env, name) orelse .nil;
}

// port of crates/substrate/src/intrinsics.rs::install_env_proto_methods `:parent`
fn envParent(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":parent receiver must be a Form");
    return world.formMeta(env, world.symParent);
}

// port of crates/substrate/src/intrinsics.rs::install_env_proto_methods `:current`
// the LIVE current frame's env. natives don't push a VM frame, so
// frames.last().env IS the caller's lexical env. used by `set!` macro.
fn envCurrent(world: *World, _: Value, _: []const Value) anyerror!Value {
    const frames = world.vm.frames.items;
    std.debug.assert(frames.len > 0); // substrate invariant
    return .{ .form = frames[frames.len - 1].env };
}

// ─────────────────────────────────────────────────────────────────
// Closure :callIn:withSelf: — V3 task 6.
// run the closure body with `env` as the frame env and `self` as
// the receiver. bypasses the closure's own :env slot (which :call
// uses for lexical scope). used by Object:eval: and future vau/fexpr.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_closure_proto_methods `:callIn:withSelf:`
fn closureCallInWithSelf(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const closure_id = self_.asFormId() orelse return typeError(world, ":callIn:withSelf: on non-closure");
    if (args.len != 2) return raise(world, "arity", ":callIn:withSelf: expects 2 args (env, self)");
    const call_env = args[0].asFormId() orelse return typeError(world, ":callIn: requires a Form env");
    const new_self = args[1];
    const body_v = world.formSlot(closure_id, world.symBody);
    const chunk_id = body_v.asFormId() orelse return typeError(world, "closure has no :body chunk");
    // defining_proto is FormId::NONE — not a method dispatch, so a
    // super-send from within the body will raise the usual "no
    // defining proto" error.
    return world.vm.runMethod(world, chunk_id, call_env, new_self, FormId.NONE);
}

// ─────────────────────────────────────────────────────────────────
// :become: — heap-level indirection. at the next dereference of `a`
// (and forever), the substrate resolves to `b`. used for live proto
// migration. returns the receiver (chainable). nursery-aware:
// aborting the turn restores the pre-turn redirect mapping.
// self-become is a no-op (handled inside world.become_).
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_if_dispatch `:become:`
fn objBecome(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asFormId() orelse return typeError(world, ":become: receiver must be a Form");
    const b = args[0].asFormId() orelse return typeError(world, ":become: argument must be a Form");
    try world.become_(a, b);
    return self_;
}

// ─────────────────────────────────────────────────────────────────
// :doesNotUnderstand:with: — default raises. user code can override
// on any proto. arg[0] is the missed selector (Symbol); arg[1] is
// the args-list (cons-chain).
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_object_reflection `:doesNotUnderstand:with:`
fn objDoesNotUnderstand(world: *World, _: Value, args: []const Value) anyerror!Value {
    // include the missed selector in the message if we can resolve
    // it. format_short-style detail is deferred — the rust impl
    // pretty-prints the receiver too, but for V4 phase α a simple
    // "doesNotUnderstand `sel`" is enough.
    _ = args;
    return raise(world, "doesNotUnderstand", "doesNotUnderstand");
}

// ─────────────────────────────────────────────────────────────────
// :perform:withArgs: — dynamic dispatch escape hatch. sends `sel` to
// receiver with argList's elements as args. honors regular dispatch:
// walks proto chain, hits any user override, raises 'doesNotUnderstand
// on miss — i.e. observationally identical to `[receiver sel args…]`
// when sel and the args are known at parse time.
//
// selector must be a Symbol; argList must be a proper list (cons-
// chain terminating in nil).
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_if_dispatch `:perform:withArgs:`
fn objPerformWithArgs(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const sel = args[0].asSym() orelse return typeError(world, ":perform:withArgs: selector must be a Symbol");
    const arg_list = args[1];
    // listToVec walks the cons-chain; raises type-error on improper
    // list. caller is responsible for cleanup of the returned slice.
    const arg_vec = try world.listToVec(arg_list);
    defer world.allocator.free(arg_vec);
    return world.send(self_, sel, arg_vec);
}

// ─────────────────────────────────────────────────────────────────
// :ifTrue:ifFalse: on Bool — branch on self, dispatch :call on the
// chosen thunk. moof-side `lib/early/02-bool.moof` re-installs per-
// singleton on #true / #false; this Bool-proto fallback lets the
// seed's `if` bytecode run during phase-1 boot.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_if_dispatch `:ifTrue:ifFalse:`
fn boolIfTrueIfFalse(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const chosen = switch (self_) {
        .bool_ => |b| if (b) args[0] else args[1],
        else => return typeError(world, ":ifTrue:ifFalse: receiver must be a Bool"),
    };
    const call_sym = try world.syms.intern("call");
    return world.send(chosen, call_sym, &.{});
}

// ─────────────────────────────────────────────────────────────────
// :toString minimal — Integer + Object fallback.
//
// the rust impl puts Integer's toString on Object (which already
// stringifies Int/Float/Bool/Sym/Char tagged immediates). we install
// an explicit Integer:toString here for clarity at the substrate
// layer; the Object fallback handles every other shape.
// ─────────────────────────────────────────────────────────────────

// port of crates/substrate/src/intrinsics.rs::install_object_reflection `:toString` for Int receivers
fn intToString(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "toString expected an Integer");
    var buf: [32]u8 = undefined;
    const text = try std.fmt.bufPrint(&buf, "{d}", .{a});
    return world.makeString(text);
}

// port of crates/substrate/src/intrinsics.rs::install_object_reflection `:toString`
// default rendering: `<Form#N>` for heap forms; tagged immediates render
// using their natural type. forms carrying a `:name` meta render as that
// name — so `[Integer toString]` → `Integer`.
fn objToString(world: *World, self_: Value, _: []const Value) anyerror!Value {
    var buf: [64]u8 = undefined;
    const text = switch (self_) {
        .nil => "nil",
        .bool_ => |b| if (b) "#true" else "#false",
        .int => |n| try std.fmt.bufPrint(&buf, "{d}", .{n}),
        .sym => |s| world.syms.resolve(s),
        .char => |cp| blk: {
            // single-codepoint utf-8 encode. fall back to a hex escape
            // for invalid codepoints.
            const cp_u21 = std.math.cast(u21, cp) orelse break :blk try std.fmt.bufPrint(&buf, "<bad-char:{x}>", .{cp});
            const len = std.unicode.utf8Encode(cp_u21, &buf) catch break :blk try std.fmt.bufPrint(&buf, "<bad-char:{x}>", .{cp});
            break :blk buf[0..len];
        },
        .float => |f| try std.fmt.bufPrint(&buf, "{d}", .{f}),
        .form => |id| blk: {
            // look up :name meta; render as the name symbol if set.
            const meta = world.formMeta(id, world.symName);
            if (meta.asSym()) |s| break :blk world.syms.resolve(s);
            break :blk try std.fmt.bufPrint(&buf, "<Form#{d}>", .{@as(u32, @bitCast(id))});
        },
    };
    return world.makeString(text);
}
