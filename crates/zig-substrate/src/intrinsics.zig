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
const image_mod = @import("image.zig");

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

    // :serializeTo: — write current World as a V4 vat-image to a path.
    // installed on Object so [$here serializeTo: "/tmp/out.vat"] works
    // (here_form's proto chain bottoms out at Object). image-load
    // re-binds this by name via NativeRefsSection: "Object:serializeTo:".
    try installNative(world, world.protos.object, "serializeTo:", objSerializeTo);
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
    try world.native_fns.put(world.allocator, method_id, native_fn);
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
    return .{ .bool_ = self_.equals(args[0]) };
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
    return .{ .bool_ = self_.equals(args[0]) };
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

// ─────────────────────────────────────────────────────────────────
// :serializeTo: — serialize the current World as a V4 vat-image to a
// path. used by moof code that wants to write its current state out
// for later loading: `[$here serializeTo: "/tmp/out.vat"]`.
//
// arg[0] must be a String-Form whose :bytes slot is a moof string-of-
// chars (cons-chain of Char values) — the moof convention until a
// real String storage lands. for now we ALSO accept a path encoded as
// a single Sym value, which avoids the cons-chain construction during
// boot-time tests.
//
// note that calling :serializeTo: on a half-bootstrapped world will
// write a half-bootstrapped image; the caller chooses the moment.
// ─────────────────────────────────────────────────────────────────

fn objSerializeTo(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return raise(world, "arity", ":serializeTo: expects 1 arg (path)");
    // path extraction: prefer Sym (boot-time convenience), fall back to
    // a String-Form whose :bytes is a cons-chain of chars.
    var path_buf: [4096]u8 = undefined;
    const path: []const u8 = switch (args[0]) {
        .sym => |s| world.syms.resolve(s),
        .form => |id| blk: {
            // String-Form heuristic: walk the cons-chain in slots,
            // collecting char codepoints into path_buf as UTF-8.
            const f = world.heap.get(id);
            const bytes_sym_id = lookupSymByName(world, "bytes") orelse {
                // no :bytes slot — assume the slot itself IS the chain
                // by using car/cdr directly. fall through with empty.
                break :blk @as([]const u8, "");
            };
            const chain_v = f.slot(bytes_sym_id);
            break :blk valueCharsToBuffer(world, chain_v, &path_buf) catch "";
        },
        else => return typeError(world, ":serializeTo: path must be a Sym or String"),
    };
    if (path.len == 0) return typeError(world, ":serializeTo: path is empty");

    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(world.allocator);
    try image_mod.serializeVat(world, &buf, world.allocator);

    // need a std.Io handle to open the file in zig 0.16. natives that
    // run from inside dispatch don't carry one unless the host stashed
    // it on the World at boot (see World.io). raise a clear error
    // rather than crashing when it's null.
    const io = world.io orelse return raise(world, "no-io", ":serializeTo: requires world.io to be set by host");

    try std.Io.Dir.cwd().writeFile(io, .{
        .sub_path = path,
        .data = buf.items,
        .flags = .{ .truncate = true },
    });
    return .nil;
}

/// linear-scan helper used by serializeTo to find a SymId by name.
/// avoids the implicit intern that `world.syms.intern` would do
/// (which would mutate the world during a serialize call).
fn lookupSymByName(world: *World, name: []const u8) ?u32 {
    const total = world.syms.len();
    var i: u32 = 1;
    while (i <= total) : (i += 1) {
        if (std.mem.eql(u8, world.syms.resolve(i), name)) return i;
    }
    return null;
}

/// walk a String-form's cons-chain of Char values into a utf-8 buffer.
/// fails silently on malformed inputs (returns whatever was decoded so
/// far). limit is the buffer's len.
fn valueCharsToBuffer(world: *World, chain: Value, buf: []u8) ![]const u8 {
    var len: usize = 0;
    var cur = chain;
    while (true) {
        switch (cur) {
            .nil => break,
            .form => |id| {
                const f = world.heap.get(id);
                const car_v = f.slot(world.symCar);
                const cdr_v = f.slot(world.symCdr);
                switch (car_v) {
                    .char => |cp| {
                        const cp_u21 = std.math.cast(u21, cp) orelse return error.BadChar;
                        const encoded = std.unicode.utf8Encode(cp_u21, buf[len..]) catch return error.BadChar;
                        len += encoded;
                    },
                    else => break,
                }
                cur = cdr_v;
            },
            else => break,
        }
    }
    return buf[0..len];
}

// ─────────────────────────────────────────────────────────────────
// Opcode constructors — class-side on the `Opcode` singleton.
//
// the rust intrinsics.rs install_compiler_primitives builds 15 of
// these as `[Opcode foo:]` constructors. each returns a fresh Form
// with proto = `Opcode`, slot `:op` = a Symbol naming the variant,
// slot `:operands` = a moof-cons list of operand values.
//
// **shape note:** the rust impl uses a `Table` for `:operands`. zig
// substrate doesn't have first-class Table storage at the substrate
// layer yet, so we use a cons-chain instead. positional access
// (`[ops at: 0]`) is replaced by `car` / `cdr` walks. the moof
// Compiler may need a small adapter if/when it reaches this path —
// but for the bootstrap, these constructors are mostly invoked then
// re-decoded via `[chunk emit:]`, which we mirror in `chunkEmit`
// below using the same cons-shape contract.
//
// port of crates/substrate/src/intrinsics.rs::install_compiler_primitives.
// ─────────────────────────────────────────────────────────────────

/// build an opcode-Form `{Opcode :op 'name :operands (cons-list operands...)}`.
/// shared by every Opcode:foo constructor below. operands list is
/// already in dispatch order — head = first operand.
fn mkOpForm(world: *World, name: []const u8, operands: []const Value) !Value {
    const op_sym = try world.syms.intern("op");
    const operands_sym = try world.syms.intern("operands");
    var op_form = Form.withProto(.{ .form = world.protos.opcode });
    const name_sym = try world.syms.intern(name);
    try op_form.slots.put(world.allocator, op_sym, .{ .sym = name_sym });
    // operands as a cons-chain. note: rust uses a Table; zig uses
    // a moof list. callers iterate via car/cdr.
    const operands_list = try world.makeList(operands);
    try op_form.slots.put(world.allocator, operands_sym, operands_list);
    const id = try world.heap.alloc(op_form);
    return .{ .form = id };
}

fn opcodePushNil(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[Opcode pushNil] takes no args");
    return mkOpForm(world, "PushNil", &.{});
}

fn opcodePushTrue(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[Opcode pushTrue] takes no args");
    return mkOpForm(world, "PushTrue", &.{});
}

fn opcodePushFalse(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[Opcode pushFalse] takes no args");
    return mkOpForm(world, "PushFalse", &.{});
}

fn opcodePop(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[Opcode pop] takes no args");
    return mkOpForm(world, "Pop", &.{});
}

fn opcodeDup(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[Opcode dup] takes no args");
    return mkOpForm(world, "Dup", &.{});
}

fn opcodeLoadSelf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[Opcode loadSelf] takes no args");
    return mkOpForm(world, "LoadSelf", &.{});
}

fn opcodeReturn(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[Opcode return] takes no args");
    return mkOpForm(world, "Return", &.{});
}

fn opcodeLoadConst(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "[Opcode loadConst: x] takes 1 arg");
    return mkOpForm(world, "LoadConst", args);
}

fn opcodeLoadName(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "[Opcode loadName: n] takes 1 arg");
    return mkOpForm(world, "LoadName", args);
}

fn opcodePushClosure(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "[Opcode pushClosure: c] takes 1 arg");
    return mkOpForm(world, "PushClosure", args);
}

fn opcodeJump(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "[Opcode jump: o] takes 1 arg");
    return mkOpForm(world, "Jump", args);
}

fn opcodeJumpIfFalse(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "[Opcode jumpIfFalse: o] takes 1 arg");
    return mkOpForm(world, "JumpIfFalse", args);
}

fn opcodeSendArgcIc(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 3) return raise(world, "arity", "[Opcode send: argc: ic:] takes 3 args");
    return mkOpForm(world, "Send", args);
}

fn opcodeTailSendArgc(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "[Opcode tailSend: argc:] takes 2 args");
    return mkOpForm(world, "TailSend", args);
}

fn opcodeSuperSendArgcIc(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 3) return raise(world, "arity", "[Opcode superSend: argc: ic:] takes 3 args");
    return mkOpForm(world, "SuperSend", args);
}

// ─────────────────────────────────────────────────────────────────
// Opcode instance methods — `:op`, `:operands`, `:toString`.
//
// the rust ensure_opcode_proto installs these as instance-side
// slot-getters so opcode-Forms can be introspected. port: identical
// shape, returning the corresponding slot.
// ─────────────────────────────────────────────────────────────────

fn opcodeOp(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return typeError(world, "op: receiver not a Form");
    const op_sym = try world.syms.intern("op");
    return world.formSlot(id, op_sym);
}

fn opcodeOperands(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return typeError(world, "operands: receiver not a Form");
    const operands_sym = try world.syms.intern("operands");
    return world.formSlot(id, operands_sym);
}

fn opcodeToString(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return typeError(world, "toString: receiver not a Form");
    const op_sym = try world.syms.intern("op");
    const name_v = world.formSlot(id, op_sym);
    var buf: [128]u8 = undefined;
    const text = switch (name_v) {
        .sym => |s| try std.fmt.bufPrint(&buf, "<{s}>", .{world.syms.resolve(s)}),
        else => try std.fmt.bufPrint(&buf, "<?>", .{}),
    };
    return world.makeString(text);
}

// ─────────────────────────────────────────────────────────────────
// Chunks singleton reflection methods.
//
// the rust install_chunks_singleton exposes the chunk side-tables
// (chunk_ops, chunk_consts, chunk_ics, chunk_params, chunk_bytecode)
// to moof code. each method takes a chunk-or-closure and returns
// a Cons of the relevant entries.
//
// **bytecode-vs-ops asymmetry:** rust stores per-chunk `chunk_ops`
// (Vec<Op> structured), while zig stores `chunk_bytecode` (raw
// V4-encoded bytes). `opsListOf:` would need to decode ops on the
// fly. for now we return nil with a TODO — the moof Method's
// `:bytecodes` reflection method can fall back to nil.
//
// port of crates/substrate/src/intrinsics.rs::install_chunks_singleton.
// ─────────────────────────────────────────────────────────────────

/// helper: extract a chunk-FormId from a value that might be a chunk,
/// a method (with `:body` slot), or a closure. nil otherwise.
fn chunkIdOf(world: *World, v: Value) ?FormId {
    const id = v.asFormId() orelse return null;
    if (world.chunk_bytecode.contains(id)) return id;
    const body_v = world.formSlot(id, world.body_sym);
    if (body_v.asFormId()) |bid| {
        if (world.chunk_bytecode.contains(bid)) return bid;
    }
    return null;
}

fn chunksIsChunk(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .{ .bool_ = false };
    if (args[0].asFormId()) |id| {
        return .{ .bool_ = world.chunk_bytecode.contains(id) };
    }
    return .{ .bool_ = false };
}

fn chunksParamsListOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    const id = args[0].asFormId() orelse return .nil;
    const p = world.formSlot(id, world.params_sym);
    if (p != .nil) return p;
    const cid = chunkIdOf(world, args[0]) orelse return .nil;
    // chunk_params is a slice of SymIds — rebuild as a Sym cons-list.
    const params = world.chunk_params.get(cid) orelse return .nil;
    var vals: std.ArrayList(Value) = .empty;
    defer vals.deinit(world.allocator);
    for (params) |sid| {
        try vals.append(world.allocator, .{ .sym = sid });
    }
    return world.makeList(vals.items);
}

fn chunksConstsListOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    const cid = chunkIdOf(world, args[0]) orelse return .nil;
    const consts = world.chunk_consts.get(cid) orelse return .nil;
    return world.makeList(consts);
}

/// **stub** — zig stores chunk bodies as raw V4-encoded bytes
/// (`chunk_bytecode`), not decoded `Op` structs. a proper port
/// would decode each op via `bytecode.decodeOp` and rebuild an
/// opcode-Form via `mkOpForm`. for the bootstrap path that doesn't
/// introspect bytecodes, returning nil is acceptable; the moof
/// Method:bytecodes reflection method will fall back.
fn chunksOpsListOf(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .nil;
}

fn chunksIcsListOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    const cid = chunkIdOf(world, args[0]) orelse return .nil;
    const ics = world.chunk_ics.get(cid) orelse return .nil;
    // each IC becomes a small Form with slots :cached-proto,
    // :cached-method, :cached-defining, :cached-generation.
    const cached_proto_sym = try world.syms.intern("cached-proto");
    const cached_method_sym = try world.syms.intern("cached-method");
    const cached_defining_sym = try world.syms.intern("cached-defining");
    const cached_generation_sym = try world.syms.intern("cached-generation");
    var entries: std.ArrayList(Value) = .empty;
    defer entries.deinit(world.allocator);
    for (ics) |ic| {
        var entry = Form.withProto(.{ .form = world.protos.object });
        const proto_v: Value = if (ic.cached_proto.isNone()) .nil else .{ .form = ic.cached_proto };
        const method_v: Value = if (ic.cached_method.isNone()) .nil else .{ .form = ic.cached_method };
        const defining_v: Value = if (ic.cached_defining.isNone()) .nil else .{ .form = ic.cached_defining };
        try entry.slots.put(world.allocator, cached_proto_sym, proto_v);
        try entry.slots.put(world.allocator, cached_method_sym, method_v);
        try entry.slots.put(world.allocator, cached_defining_sym, defining_v);
        try entry.slots.put(world.allocator, cached_generation_sym, .{ .int = @as(i48, @intCast(ic.cached_generation)) });
        const eid = try world.heap.alloc(entry);
        try entries.append(world.allocator, .{ .form = eid });
    }
    return world.makeList(entries.items);
}

fn chunksBodyOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    const id = args[0].asFormId() orelse return .nil;
    const body_v = world.formSlot(id, world.body_sym);
    if (body_v.asFormId()) |bid| {
        if (world.chunk_bytecode.contains(bid)) return .{ .form = bid };
    }
    if (world.chunk_bytecode.contains(id)) return .{ .form = id };
    return .nil;
}

// ─────────────────────────────────────────────────────────────────
// Heap singleton — six reflection primitives moof code uses to
// introspect Forms (slots / handlers / meta / heap-id / proto).
//
// the rust install_heap_singleton wires these as `[Heap protoOf: v]`
// style class-method sends; image-rebind matches them by canonical
// "Heap:selector" name.
// ─────────────────────────────────────────────────────────────────

fn heapProtoOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    return world.protoOf(args[0]);
}

fn heapHeapIdOf(_: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .{ .int = 0 };
    return switch (args[0]) {
        .form => |fid| .{ .int = @as(i48, @intCast(@as(u32, @bitCast(fid)))) },
        else => .{ .int = 0 },
    };
}

fn heapAllocFormWithProto(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return typeError(world, "allocFormWithProto: needs 1 arg");
    const proto_v = args[0];
    if (proto_v != .form) return typeError(world, "allocFormWithProto: proto must be a Form");
    var f = Form.withProto(proto_v);
    const id = try world.heap.alloc(f);
    _ = &f;
    return .{ .form = id };
}

fn heapSlotOfAt(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 2) return typeError(world, "slotOf:at: needs 2 args");
    const id = args[0].asFormId() orelse return .nil;
    const sym = args[1].asSym() orelse return typeError(world, "slotOf:at: key must be a Symbol");
    return world.formSlot(id, sym);
}

fn heapHandlerOfAt(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 2) return typeError(world, "handlerOf:at: needs 2 args");
    const id = args[0].asFormId() orelse return .nil;
    const sym = args[1].asSym() orelse return typeError(world, "handlerOf:at: key must be a Symbol");
    const f = world.heap.get(id);
    return f.handler(sym) orelse .nil;
}

fn heapMetaOfAt(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 2) return typeError(world, "metaOf:at: needs 2 args");
    const id = args[0].asFormId() orelse return .nil;
    const sym = args[1].asSym() orelse return typeError(world, "metaOf:at: key must be a Symbol");
    return world.formMeta(id, sym);
}

// ─────────────────────────────────────────────────────────────────
// Method:call — invoke a method/closure Form with args. wraps the
// substrate's send-path so the closure's captured-self is honored.
//
// port of crates/substrate/src/intrinsics.rs::install_call_on_method.
// ─────────────────────────────────────────────────────────────────

fn methodCall(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return raise(world, "dispatch", "receiver of :call is not a Form");
    // captured-self for closures created by PushClosure.
    const captured_sym = try world.syms.intern("captured-self");
    const captured = world.formSlot(id, captured_sym);
    // dispatch via the body chunk. mirror the inline-arg-binding done
    // by World.send for a method-Form, but with an arbitrary receiver
    // (the captured-self) and no super-context.
    const body_v = world.formSlot(id, world.body_sym);
    const chunk_id = body_v.asFormId() orelse return raise(world, "dispatch", ":call: method has no :body chunk");
    const captured_env_v = world.formSlot(id, world.env_sym);
    const captured_env = captured_env_v.asFormId() orelse world.here_form;
    const params_v = world.formSlot(id, world.params_sym);
    const params = try world.listToSlice(params_v);
    defer world.freeSlice(params);
    if (params.len != args.len) return raise(world, "arity", ":call: argc mismatch");
    const call_env = try world.allocEnv(captured_env);
    for (params, args) |p, a| {
        const ps = p.asSym() orelse return raise(world, "type-error", ":call: bad param");
        try world.envBind(call_env, ps, a);
    }
    return world.vm.runMethod(world, chunk_id, call_env, captured, FormId.NONE);
}

// ─────────────────────────────────────────────────────────────────
// Object basics — port of intrinsics.rs::install_object_reflection.
//
// `:=` is identity equality (Object default; protos like Integer
// override). `:new` allocates a fresh form with this proto.
// `:initialize` is a hook returning self. `:freeze` flips the
// `frozen` bit; `:frozen?` / `:freezable?` query it.
// ─────────────────────────────────────────────────────────────────

fn objEq(_: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .{ .bool_ = false };
    return .{ .bool_ = self_.equals(args[0]) };
}

fn objNew(world: *World, self_: Value, _: []const Value) anyerror!Value {
    // [Proto new] — allocate a fresh form whose proto is the receiver.
    var f = Form.withProto(self_);
    const id = try world.heap.alloc(f);
    _ = &f;
    return .{ .form = id };
}

fn objInitialize(_: *World, self_: Value, _: []const Value) anyerror!Value {
    return self_;
}

fn objFreeze(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return self_;
    const fm = world.heap.getMut(id);
    fm.frozen = true;
    return self_;
}

fn objFrozen(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return .{ .bool_ = false };
    return .{ .bool_ = world.heap.get(id).frozen };
}

fn objFreezable(_: *World, self_: Value, _: []const Value) anyerror!Value {
    // every Form is freezable; tagged immediates are conceptually
    // already-frozen. report true uniformly (matches rust default).
    _ = self_;
    return .{ .bool_ = true };
}

// ─────────────────────────────────────────────────────────────────
// Cons basics — port of intrinsics.rs::install_cons_and_nil_primitives.
//
// `:cons:` builds `(cdr cons: car)` — i.e. self IS the tail, arg is
// the new head. `:empty?` / `:null?` / `:nonEmpty?` are obvious.
// ─────────────────────────────────────────────────────────────────

fn consConsInto(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "cons: takes 1 arg");
    var f = Form.withProto(.{ .form = world.protos.cons });
    try f.slots.put(world.allocator, world.symCar, args[0]);
    try f.slots.put(world.allocator, world.symCdr, self_);
    const id = try world.heap.alloc(f);
    return .{ .form = id };
}

fn consEmptyFalse(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .{ .bool_ = false };
}

fn consEmptyTrue(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .{ .bool_ = true };
}

fn nilProto(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .nil;
}

fn consReverse(world: *World, self_: Value, _: []const Value) anyerror!Value {
    var acc: Value = .nil;
    var cur = self_;
    while (true) {
        switch (cur) {
            .nil => break,
            .form => |id| {
                const f = world.heap.get(id);
                if (!f.slotPresent(world.symCar)) break;
                const car = f.slot(world.symCar);
                const cdr = f.slot(world.symCdr);
                var node = Form.withProto(.{ .form = world.protos.cons });
                try node.slots.put(world.allocator, world.symCar, car);
                try node.slots.put(world.allocator, world.symCdr, acc);
                const nid = try world.heap.alloc(node);
                acc = .{ .form = nid };
                cur = cdr;
            },
            else => break,
        }
    }
    return acc;
}

// ─────────────────────────────────────────────────────────────────
// Compiler / Reader cap flag toggles.
//
// the rust install_compiler_cap / install_reader_cap allocate
// **anonymous** proto-Forms (no `:name` meta) and bind them under
// `$compiler` / `$reader` globally. when v4_export collects native
// methods it labels them `<anon-N>:useMoof` / `<anon-N>:useSeed`
// where N is the heap index — and N varies across boots.
//
// because the canonical name in the image's NativeRefsSection
// changes per-image, a static REGISTRY entry cannot reliably
// re-bind these. we keep the zig-side functions here so the rust
// runtime can install them at world-init via `install`, but they
// are **NOT** added to the REGISTRY. when track-A self-host moves
// these flags into in-image state (post wave-W3), the issue goes
// away.
//
// (the no-op stub matches rust shape: world.use_moof_compiler /
// world.use_moof_reader don't exist in zig — there's no
// rust-versus-moof compiler split here. natives are still defined
// to keep the surface complete.)
// ─────────────────────────────────────────────────────────────────

fn capNoOp(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .nil;
}

// ─────────────────────────────────────────────────────────────────
// REGISTRY — comptime name → NativeFn map (V4 Track C.3 Task 2.1).
//
// keyed by canonical "ProtoName:selector" strings. image-load
// (image.zig::readNativeRefs) queries this via
// World.lookupNativeByName to re-bind native methods after
// deserialization. names must agree with what the rust v4_export
// emits in its NativeRefsSection — cross-stack contract.
//
// std.StaticStringMap is comptime-built: a typo in any key or fn
// reference is a build-time error. zero runtime registration code.
// ─────────────────────────────────────────────────────────────────

pub const REGISTRY = std.StaticStringMap(NativeFn).initComptime(.{
    .{ "Integer:+", intPlus },
    .{ "Integer:-", intMinus },
    .{ "Integer:*", intMultiply },
    .{ "Integer:/", intDivide },
    .{ "Integer:=", intEq },
    .{ "Integer:<", intLt },
    .{ "Integer:>", intGt },
    .{ "Integer:toString", intToString },
    .{ "Object:!!", objBangBang },
    .{ "Nil:!!", nilBangBang },
    .{ "Bool:!!", boolBangBang },
    .{ "Object:is", objIs },
    .{ "Object:proto", objProto },
    .{ "Object:identity", objIdentity },
    .{ "Object:slot:", objSlot },
    .{ "Object:slotSet!:", objSlotSet },
    .{ "Cons:car", consCar },
    .{ "Cons:cdr", consCdr },
    .{ "Env:bind:to:", envBindTo },
    .{ "Env:set:to:", envSetTo },
    .{ "Env:lookup:", envLookupTo },
    .{ "Env:parent", envParent },
    .{ "Env:current", envCurrent },
    .{ "Closure:callIn:withSelf:", closureCallInWithSelf },
    .{ "Object:become:", objBecome },
    .{ "Object:doesNotUnderstand:with:", objDoesNotUnderstand },
    .{ "Object:perform:withArgs:", objPerformWithArgs },
    .{ "Bool:ifTrue:ifFalse:", boolIfTrueIfFalse },
    .{ "Object:toString", objToString },
    .{ "Object:serializeTo:", objSerializeTo },

    // W10 (track B) additions — Opcode constructors, ~15 entries.
    .{ "Opcode:pushNil", opcodePushNil },
    .{ "Opcode:pushTrue", opcodePushTrue },
    .{ "Opcode:pushFalse", opcodePushFalse },
    .{ "Opcode:pop", opcodePop },
    .{ "Opcode:dup", opcodeDup },
    .{ "Opcode:loadSelf", opcodeLoadSelf },
    .{ "Opcode:return", opcodeReturn },
    .{ "Opcode:loadConst:", opcodeLoadConst },
    .{ "Opcode:loadName:", opcodeLoadName },
    .{ "Opcode:pushClosure:", opcodePushClosure },
    .{ "Opcode:jump:", opcodeJump },
    .{ "Opcode:jumpIfFalse:", opcodeJumpIfFalse },
    .{ "Opcode:send:argc:ic:", opcodeSendArgcIc },
    .{ "Opcode:tailSend:argc:", opcodeTailSendArgc },
    .{ "Opcode:superSend:argc:ic:", opcodeSuperSendArgcIc },

    // Opcode instance methods — opcode-Form reflection.
    .{ "Opcode:op", opcodeOp },
    .{ "Opcode:operands", opcodeOperands },
    .{ "Opcode:toString", opcodeToString },

    // Chunks singleton reflection methods.
    .{ "Chunks:isChunk?:", chunksIsChunk },
    .{ "Chunks:paramsListOf:", chunksParamsListOf },
    .{ "Chunks:constsListOf:", chunksConstsListOf },
    .{ "Chunks:opsListOf:", chunksOpsListOf },
    .{ "Chunks:icsListOf:", chunksIcsListOf },
    .{ "Chunks:bodyOf:", chunksBodyOf },

    // Heap singleton.
    .{ "Heap:protoOf:", heapProtoOf },
    .{ "Heap:heapIdOf:", heapHeapIdOf },
    .{ "Heap:allocFormWithProto:", heapAllocFormWithProto },
    .{ "Heap:slotOf:at:", heapSlotOfAt },
    .{ "Heap:handlerOf:at:", heapHandlerOfAt },
    .{ "Heap:metaOf:at:", heapMetaOfAt },

    // Method:call — invoke a method/closure form.
    .{ "Method:call", methodCall },

    // Object basics.
    .{ "Object:=", objEq },
    .{ "Object:new", objNew },
    .{ "Object:initialize", objInitialize },
    .{ "Object:freeze", objFreeze },
    .{ "Object:frozen?", objFrozen },
    .{ "Object:freezable?", objFreezable },

    // Cons / Nil basics.
    .{ "Cons:cons:", consConsInto },
    .{ "Nil:cons:", consConsInto },
    .{ "Cons:empty?", consEmptyFalse },
    .{ "Cons:null?", consEmptyFalse },
    .{ "Cons:nonEmpty?", consEmptyTrue }, // and conversely Nil:empty? is true
    .{ "Nil:empty?", consEmptyTrue },
    .{ "Nil:proto", nilProto },
    .{ "Cons:reverse", consReverse },
});
