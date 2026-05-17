//! moof-zig — primordial native methods.
//!
//! V4 task A.6. installed at `World.init()`, before any moof source
//! loads. ports the minimal-viable-subset (~30 natives) from
//! `players/rust/src/intrinsics.rs`; the rest are derived in moof
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
const ICache = world_mod.ICache;
const SymId = world_mod.SymId;
const image_mod = @import("image.zig");
const opcodes_mod = @import("opcodes.zig");
const Op = opcodes_mod.Op;
const bytecode_mod = @import("bytecode.zig");
const vm_mod = @import("vm.zig");

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

/// install the primordial caps — `$transporter`, `$compiler`, `$reader`.
/// each is an anonymous Object-proto-Form with the listed handlers,
/// bound in `world.here_form`'s slots under the dollared name.
///
/// **call AFTER image-load** (or after `World.init`). image-load
/// doesn't preserve anonymous-proto natives across the image
/// boundary — the v4_export side labels them `<anon-N>:useMoof`
/// where N varies, so they can't ride the NativeRefsSection rebind
/// path. instead, the host calls this helper at run-time to wire
/// the caps in place. mirrors rust install_compiler_cap /
/// install_reader_cap / transporter::install.
pub fn installCaps(world: *World) !void {
    if (world.protos.object.isNone()) return error.NoObjectProto;
    if (world.here_form.isNone()) return error.NoHereForm;

    const dollar_transporter = try world.syms.intern("$transporter");
    const dollar_compiler = try world.syms.intern("$compiler");
    const dollar_reader = try world.syms.intern("$reader");
    const dollar_layout = try world.syms.intern("$layout");
    const name_meta = try world.syms.intern("name");
    const transporter_name = try world.syms.intern("Transporter");
    const compiler_name = try world.syms.intern("Compiler");
    const reader_name = try world.syms.intern("Reader");
    const layout_name = try world.syms.intern("Layout");

    // $transporter
    {
        var proto = Form.withProto(.{ .form = world.protos.object });
        // tag with :name so v4_export-style introspection sees
        // "Transporter:load:" (matches the REGISTRY keys below).
        try proto.meta.put(world.allocator, name_meta, .{ .sym = transporter_name });
        const proto_id = try world.heap.alloc(proto);
        try installNative(world, proto_id, "load:", transporterLoad);
        try installNative(world, proto_id, "loadAll:", transporterLoadAll);
        try world.envBind(world.here_form, dollar_transporter, .{ .form = proto_id });
    }

    // $compiler
    {
        var proto = Form.withProto(.{ .form = world.protos.object });
        try proto.meta.put(world.allocator, name_meta, .{ .sym = compiler_name });
        const proto_id = try world.heap.alloc(proto);
        try installNative(world, proto_id, "useMoof", compilerUseMoof);
        try installNative(world, proto_id, "useSeed", compilerUseSeed);
        try world.envBind(world.here_form, dollar_compiler, .{ .form = proto_id });
    }

    // $reader
    {
        var proto = Form.withProto(.{ .form = world.protos.object });
        try proto.meta.put(world.allocator, name_meta, .{ .sym = reader_name });
        const proto_id = try world.heap.alloc(proto);
        try installNative(world, proto_id, "useMoof", readerUseMoof);
        try installNative(world, proto_id, "useSeed", readerUseSeed);
        try world.envBind(world.here_form, dollar_reader, .{ .form = proto_id });
    }

    // $layout — exposes World.registerLayout to moof code. defproto
    // calls `[$layout register: Counter slots: '(count)]` so user
    // protos get inline-slot storage (the Layout fast path) without
    // any per-instance map traffic. anonymous-proto same caveat as
    // $compiler / $reader: host installs at runtime, not via image
    // NativeRefsSection.
    {
        var proto = Form.withProto(.{ .form = world.protos.object });
        try proto.meta.put(world.allocator, name_meta, .{ .sym = layout_name });
        const proto_id = try world.heap.alloc(proto);
        try installNative(world, proto_id, "register:slots:", layoutRegisterSlots);
        try world.envBind(world.here_form, dollar_layout, .{ .form = proto_id });
    }
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

// port of players/rust/src/intrinsics.rs::install_integer_methods `:+`
fn intPlus(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "+ expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "+ expected a numeric rhs");
    return .{ .int = a +% b };
}

// port of players/rust/src/intrinsics.rs::install_integer_methods `:-`
fn intMinus(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "- expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "- expected a numeric rhs");
    return .{ .int = a -% b };
}

// port of players/rust/src/intrinsics.rs::install_integer_methods `:*`
fn intMultiply(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "* expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "* expected a numeric rhs");
    return .{ .int = a *% b };
}

// port of players/rust/src/intrinsics.rs::install_integer_methods `:/`
fn intDivide(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "/ expected an Integer");
    const b = args[0].asInt() orelse return typeError(world, "/ expected a numeric rhs");
    if (b == 0) return raise(world, "division-by-zero", "integer division by zero");
    // wrapping_div: i48 has the same MIN/-1 quirk as i64, but moof
    // accepts it as wrapping per substrate.rs.
    return .{ .int = @divTrunc(a, b) };
}

// port of players/rust/src/intrinsics.rs::install_integer_methods `:=`
fn intEq(_: *World, self_: Value, args: []const Value) anyerror!Value {
    // defensive against proto-Form receivers (see rust comment at
    // intrinsics.rs:1371): if self isn't actually an Int, fall back
    // to identity comparison.
    const a_opt = self_.asInt();
    const b_opt = args[0].asInt();
    if (a_opt) |a| if (b_opt) |b| return .{ .bool_ = a == b };
    return .{ .bool_ = self_.equals(args[0]) };
}

// port of players/rust/src/intrinsics.rs::install_integer_methods `:<`
fn intLt(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "< expected an Integer receiver");
    const b = args[0].asInt() orelse return typeError(world, "< expected a numeric rhs");
    return .{ .bool_ = a < b };
}

// port of players/rust/src/intrinsics.rs::install_integer_methods `:>`
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

// port of players/rust/src/intrinsics.rs::install_if_dispatch `:!!` on Object
fn objBangBang(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .{ .bool_ = true };
}

// port of players/rust/src/intrinsics.rs::install_if_dispatch `:!!` on Nil
fn nilBangBang(_: *World, _: Value, _: []const Value) anyerror!Value {
    return .{ .bool_ = false };
}

// port of players/rust/src/intrinsics.rs::install_if_dispatch `:!!` on Bool
fn boolBangBang(_: *World, self_: Value, _: []const Value) anyerror!Value {
    return self_;
}

// ─────────────────────────────────────────────────────────────────
// Object reflection / identity primitives.
// ─────────────────────────────────────────────────────────────────

// port of players/rust/src/intrinsics.rs::install_object_reflection `:is`
// identity equality (same heap-id or same tagged-immediate).
fn objIs(_: *World, self_: Value, args: []const Value) anyerror!Value {
    return .{ .bool_ = self_.equals(args[0]) };
}

// port of players/rust/src/intrinsics.rs (Heap singleton `protoOf:` /
// the moof `:proto` defmethod that delegates there). returns the proto
// Value of a Form receiver. tagged immediates fall through to their
// proto-Form (e.g. Int → Integer-proto) via world.protoOf — matches
// rust's `proto_of` helper at world.rs:556.
fn objProto(world: *World, self_: Value, _: []const Value) anyerror!Value {
    return world.protoOf(self_);
}

// port of players/rust/src/intrinsics.rs (Heap singleton `heapIdOf:` /
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

// port of players/rust/src/intrinsics.rs `(slot v 'name)`
fn objSlot(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const sym = args[0].asSym() orelse return typeError(world, "slot: name must be a Symbol");
    const id = self_.asFormId() orelse return .nil;
    return world.formSlot(id, sym);
}

// port of players/rust/src/intrinsics.rs `(slotSet! v 'name v)`
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

// §5.8d — Cons:car fast path. when the Form has a layout (every
// allocFlatCons cell does, post-§5.8d step 2), read inline_slots[0]
// directly — that's where allocFlatCons stores the canonical car.
// general-Form Cons cells (rare, only from image-load before
// reflatten or from synthetic test allocations) fall through to
// formSlot, which still works via the SlotMap.
fn consCar(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return typeError(world, "car on non-Cons");
    const f = world.heap.get(id);
    if (f.layout != null) return f.inline_slots[0];
    return world.formSlot(id, world.symCar);
}

// §5.8d — Cons:cdr fast path. mirrors consCar.
fn consCdr(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return typeError(world, "cdr on non-Cons");
    const f = world.heap.get(id);
    if (f.layout != null) return f.inline_slots[1];
    return world.formSlot(id, world.symCdr);
}

// ─────────────────────────────────────────────────────────────────
// Env methods — port of intrinsics.rs::install_env_proto_methods.
// V3 spec §4.1 — non-walking `:bind:to:`, walking `:set:to:` (raises
// 'unbound on miss), walking `:lookup:` (nil on miss). plus `:parent`
// and the `[Env current]` class-method-style accessor.
// ─────────────────────────────────────────────────────────────────

// port of players/rust/src/intrinsics.rs::install_env_proto_methods `:bind:to:`
fn envBindTo(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":bind:to: receiver must be an Env Form");
    const name = args[0].asSym() orelse return typeError(world, ":bind:to: name must be a Symbol");
    const val = args[1];
    try world.envBind(env, name, val);
    return val;
}

// port of players/rust/src/intrinsics.rs::install_env_proto_methods `:set:to:`
fn envSetTo(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":set:to: receiver must be an Env Form");
    const name = args[0].asSym() orelse return typeError(world, ":set:to: name must be a Symbol");
    const val = args[1];
    const found = try world.envSet(env, name, val);
    if (!found) return raise(world, "unbound", "set!: name is unbound");
    return val;
}

// port of players/rust/src/intrinsics.rs::install_env_proto_methods `:lookup:`
fn envLookupTo(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":lookup: receiver must be an Env Form");
    const name = args[0].asSym() orelse return typeError(world, ":lookup: name must be a Symbol");
    return world.envLookup(env, name) orelse .nil;
}

// port of players/rust/src/intrinsics.rs::install_env_proto_methods `:parent`
fn envParent(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const env = self_.asFormId() orelse return typeError(world, ":parent receiver must be a Form");
    return world.formMeta(env, world.symParent);
}

// port of players/rust/src/intrinsics.rs::install_env_proto_methods `:current`
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

// port of players/rust/src/intrinsics.rs::install_closure_proto_methods `:callIn:withSelf:`
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

// port of players/rust/src/intrinsics.rs::install_if_dispatch `:become:`
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

// port of players/rust/src/intrinsics.rs::install_object_reflection `:doesNotUnderstand:with:`
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

// port of players/rust/src/intrinsics.rs::install_if_dispatch `:perform:withArgs:`
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

// port of players/rust/src/intrinsics.rs::install_if_dispatch `:ifTrue:ifFalse:`
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

// port of players/rust/src/intrinsics.rs::install_object_reflection `:toString` for Int receivers
fn intToString(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const a = self_.asInt() orelse return typeError(world, "toString expected an Integer");
    var buf: [32]u8 = undefined;
    const text = try std.fmt.bufPrint(&buf, "{d}", .{a});
    return world.makeString(text);
}

// port of players/rust/src/intrinsics.rs::install_object_reflection `:toString`
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
// port of players/rust/src/intrinsics.rs::install_compiler_primitives.
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

fn opcodeSendSelfArgcIc(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 3) return raise(world, "arity", "[Opcode sendSelf: argc: ic:] takes 3 args");
    return mkOpForm(world, "SendSelf", args);
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
// port of players/rust/src/intrinsics.rs::install_chunks_singleton.
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
    // TODO(phase2): this reflection primitive bypasses `vat_mode` auto-freeze
    // (per design spec §4.1). in a `frozen_default` vat, `[$heap
    // allocFormWithProto: SomeProto]` would yield a mutable form, diverging
    // from `allocInstance` behavior. impact is currently zero (default mode
    // is mutable); revisit when frozen-default vats arrive. same gap exists
    // in `world.allocFlatCons`.
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

fn heapSlotKeysOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    const id = args[0].asFormId() orelse return .nil;
    const f = world.heap.get(id);
    var keys: std.ArrayList(Value) = .empty;
    defer keys.deinit(world.allocator);
    // §5.8d — for Layout-backed Forms, surface layout slot names in
    // declaration order BEFORE the extras map (matches canonical
    // insertion order at allocation time).
    if (f.layout) |lay| {
        var i: u8 = 0;
        while (i < lay.inline_size) : (i += 1) {
            try keys.append(world.allocator, .{ .sym = lay.slot_names[i] });
        }
    }
    var it = f.slots.iterator();
    while (it.next()) |entry| {
        try keys.append(world.allocator, .{ .sym = entry.key_ptr.* });
    }
    return world.makeList(keys.items);
}

fn heapHandlerKeysOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    const id = args[0].asFormId() orelse return .nil;
    const f = world.heap.get(id);
    var keys: std.ArrayList(Value) = .empty;
    defer keys.deinit(world.allocator);
    var it = f.handlers.iterator();
    while (it.next()) |entry| {
        try keys.append(world.allocator, .{ .sym = entry.key_ptr.* });
    }
    return world.makeList(keys.items);
}

fn heapMetaKeysOf(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .nil;
    const id = args[0].asFormId() orelse return .nil;
    const f = world.heap.get(id);
    var keys: std.ArrayList(Value) = .empty;
    defer keys.deinit(world.allocator);
    var it = f.meta.iterator();
    while (it.next()) |entry| {
        try keys.append(world.allocator, .{ .sym = entry.key_ptr.* });
    }
    return world.makeList(keys.items);
}


// ─────────────────────────────────────────────────────────────────
// Method:call — invoke a method/closure Form with args. wraps the
// substrate's send-path so the closure's captured-self is honored.
//
// port of players/rust/src/intrinsics.rs::install_call_on_method.
// ─────────────────────────────────────────────────────────────────

fn methodCall(world: *World, self_: Value, args: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return raise(world, "dispatch", "receiver of :call is not a Form");
    // native? free-function globals bound on here_form (e.g.
    // `setHandler!`, `cons`, `list`) are method-Forms with a registered
    // native_fn but NO body chunk. dispatch them directly — mirrors
    // rust World::invoke (vm.rs:250) which checks native_fns first.
    if (world.nativeFn(id)) |native| {
        const captured_sym = try world.syms.intern("captured-self");
        const captured = world.formSlot(id, captured_sym);
        return native(world, captured, args);
    }
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
// `:freeze` flips the `frozen` bit; `:frozen?` / `:freezable?`
// query it. `:initialize` was a trivial `return self` stub — now
// lives in stdlib/object.moof as a defmethod.
// ─────────────────────────────────────────────────────────────────

fn objEq(_: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .{ .bool_ = false };
    return .{ .bool_ = self_.equals(args[0]) };
}

fn objNew(world: *World, self_: Value, _: []const Value) anyerror!Value {
    // [Proto new] — allocate a fresh form whose proto is the receiver.
    //
    // §5.8d — consult the proto_layouts registry; if the receiver
    // proto has a Layout registered, `allocInstance` produces a Form
    // whose `layout` pointer is set and whose `inline_slots` are
    // zero-initialized. all subsequent `slotSet!` calls on a canonical
    // slot land on `inline_slots` via `formSlotSet`'s layoutTrySet
    // fast path. proto without a Layout → general Form (SlotMap).
    //
    // §5.8b legacy: Cons specifically still routes through
    // `allocFlatCons` because consCar/consCdr in this file still
    // read `f.car_inline` / `f.cdr_inline`. step 5 retires those
    // reads and migrates Cons to the unified Layout path; step 8
    // deletes the FlatCons fields entirely.
    if (self_.asFormId()) |proto_id| {
        if (proto_id.eql(world.protos.cons)) {
            const id = try world.allocFlatCons(Value.nil, Value.nil);
            return .{ .form = id };
        }
        const id = try world.allocInstance(proto_id);
        return .{ .form = id };
    }
    var f = Form.withProto(self_);
    const id = try world.heap.alloc(f);
    _ = &f;
    return .{ .form = id };
}

fn objFreeze(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return self_;
    const fm = world.heap.get(id);
    // already frozen: idempotent no-op (calling freeze twice is fine).
    if (fm.frozen) return self_;
    // live-face forms (ForeignHandle in V0): cannot-freeze-live per spec §4.5.
    if (!world.isFreezable(id)) {
        return world.raise("cannot-freeze-live", "form is a live face and cannot be frozen");
    }
    world.heap.getMut(id).frozen = true;
    return self_;
}

fn objFrozen(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const id = self_.asFormId() orelse return .{ .bool_ = false };
    return .{ .bool_ = world.heap.get(id).frozen };
}

fn objFreezable(world: *World, self_: Value, _: []const Value) anyerror!Value {
    // spec §4.5: report false for live-face forms (ForeignHandle in V0).
    // tagged immediates are conceptually already-frozen but not "live",
    // so report true for them (they can be "frozen" trivially).
    const id = self_.asFormId() orelse return .{ .bool_ = true };
    return .{ .bool_ = world.isFreezable(id) };
}

// ── phase1/B: vat-mode intrinsics ────────────────────────────────

/// `(__vat-mode__)` — returns the current world's vat mode as a Symbol.
/// returns 'mutable-by-default or 'frozen-by-default.
fn globalVatMode(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "(__vat-mode__) takes no args");
    const sym_name: []const u8 = switch (world.vat_mode) {
        .mutable_default => "mutable-by-default",
        .frozen_default => "frozen-by-default",
    };
    const sym_id = try world.syms.intern(sym_name);
    return .{ .sym = sym_id };
}

/// `(__alloc-mutable__ proto)` — allocate a fresh form that is mutable
/// regardless of vat-mode. used by the let-mutable macro to bypass
/// auto-freeze for scoped construct-then-seal idioms.
fn globalAllocMutable(world: *World, _: Value, args: []const Value) anyerror!Value {
    const proto_id = if (args.len > 0)
        args[0].asFormId() orelse return typeError(world, "__alloc-mutable__: proto must be a Form")
    else
        world.protos.object;
    const id = try world.allocMutableBypass(proto_id);
    return .{ .form = id };
}

// ─────────────────────────────────────────────────────────────────
// Cons basics — port of intrinsics.rs::install_cons_and_nil_primitives.
//
// `:cons:` builds `(cdr cons: car)` — i.e. self IS the tail, arg is
// the new head. `:empty?` / `:null?` / `:nonEmpty?` are obvious.
// ─────────────────────────────────────────────────────────────────

fn consConsInto(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "cons: takes 1 arg");
    // §5.8b — flat-cons. car = args[0] (new head), cdr = self_
    // (prior tail). zero SlotMap traffic.
    const id = try world.allocFlatCons(args[0], self_);
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
                // §5.8b — new node is a flat-cons.
                const nid = try world.allocFlatCons(car, acc);
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
// $compiler / $reader cap flag-flip natives.
//
// the rust intrinsics.rs::install_compiler_cap / install_reader_cap
// install these on **anonymous** proto-Forms so v4_export labels
// them `<anon-N>:useMoof` where N changes per-image — no canonical
// REGISTRY key. but the SEED image-builder doesn't go through
// v4_export; it builds named protos (or skips natives entirely for
// the cap, since seed-vat's `$compiler` / `$reader` natives are
// re-installed on the in-image cap by intrinsics-install at boot).
//
// for the zig substrate's view: we want these flip-flops callable
// from moof code at boot time. installed on the Object proto (which
// is also where the `$compiler` / `$reader` caps' protos chain to)
// keyed by canonical "Compiler:useMoof" / "Reader:useMoof" etc.
// names — the seed.vat may not actually use these names, but our
// caller (the host) will install the cap separately and bind
// directly via env_bind.
//
// when the host has installed `$compiler` / `$reader` caps, sends
// to them route to these natives, which toggle the world flag.
// note that for zig, with no native parser/compiler, useMoof MUST
// be set or eval-string-in-world raises. useSeed is purely API parity.
// ─────────────────────────────────────────────────────────────────

fn compilerUseMoof(world: *World, _: Value, _: []const Value) anyerror!Value {
    world.use_moof_compiler = true;
    return .nil;
}

fn compilerUseSeed(world: *World, _: Value, _: []const Value) anyerror!Value {
    world.use_moof_compiler = false;
    return .nil;
}

fn readerUseMoof(world: *World, _: Value, _: []const Value) anyerror!Value {
    world.use_moof_reader = true;
    return .nil;
}

fn readerUseSeed(world: *World, _: Value, _: []const Value) anyerror!Value {
    world.use_moof_reader = false;
    return .nil;
}

// ─────────────────────────────────────────────────────────────────
// $transporter — Self-style file ↔ image bridge.
//
// port of players/rust/src/transporter.rs (~160 LoC). subset:
//
//   [$transporter load: rel]     — read file, parse, compile, run
//   [$transporter loadAll: list] — same on each path in a cons-list
//
// :root and :dump:toFile: deferred — :root is diagnostics-only and
// :dump:toFile: was already TODO in rust.
//
// implementation skeleton:
//
//   1. extract path string from Value (sym or String-Form)
//   2. refuse absolute / `..`-traversing paths (rust:transporter.rs:124)
//   3. resolve against world.transporter_root
//   4. read file via world.io
//   5. delegate to evalStringInWorld — parse + compile + run via the
//      in-image Parser / Compiler. requires use_moof_reader and
//      use_moof_compiler to be true.
//
// **CAVEAT** (V4 alpha): the OCaml seed image lifts `Str "..."` into
// an empty placeholder Form (build_seed_cmd.ml:284). zig's image-load
// preserves that placeholder. so a path-string from inside a
// seed.vat chunk arrives as an empty Form — no :bytes, no content.
// `extractPath` raises a clear error for this case. fix is to teach
// ocaml-seed to populate :bytes on Str forms (next session task).
// ─────────────────────────────────────────────────────────────────

/// extract a path string from `v`. accepts:
///   1. `.sym` — symbol text is the path (boot-test convenience)
///   2. `.form` — String-Form with a `:bytes` slot that's either a
///      foreign byte handle (rust-side) or a cons-chain of Char values
///      (zig-side stand-in).
///
/// the caller owns the returned slice and must free with `world.allocator`.
/// raises if the value isn't a recognized String shape OR the form has
/// no usable :bytes content (the seed.vat placeholder case).
fn extractPath(world: *World, v: Value) ![]u8 {
    switch (v) {
        .sym => |s| {
            const text = world.syms.resolve(s);
            return world.allocator.dupe(u8, text);
        },
        .form => |id| {
            const f = world.heap.get(id);
            // try :bytes slot — same convention as :serializeTo:.
            const bytes_sym = lookupSymByName(world, "bytes") orelse {
                return raise(world, "tx-bad-arg", ":load: path-Form has no :bytes slot (seed.vat string placeholder?)");
            };
            const chain = f.slot(bytes_sym);
            // walk the chain into a growable buffer. unknown maximum
            // length, so we use ArrayList rather than the 4 KiB stack
            // buffer used by :serializeTo:.
            var buf: std.ArrayList(u8) = .empty;
            errdefer buf.deinit(world.allocator);
            var cur = chain;
            var saw_any = false;
            while (true) {
                switch (cur) {
                    .nil => break,
                    .form => |cid| {
                        const cf = world.heap.get(cid);
                        if (!cf.slotPresent(world.symCar)) break;
                        const car_v = cf.slot(world.symCar);
                        const cdr_v = cf.slot(world.symCdr);
                        switch (car_v) {
                            .char => |cp| {
                                saw_any = true;
                                const cp_u21 = std.math.cast(u21, cp) orelse return raise(world, "tx-bad-arg", ":load: path contains invalid char");
                                var tmp: [4]u8 = undefined;
                                const n = std.unicode.utf8Encode(cp_u21, &tmp) catch return raise(world, "tx-bad-arg", ":load: path contains un-encodable char");
                                try buf.appendSlice(world.allocator, tmp[0..n]);
                            },
                            else => break,
                        }
                        cur = cdr_v;
                    },
                    else => break,
                }
            }
            if (!saw_any) {
                return raise(world, "tx-bad-arg", ":load: path-Form's :bytes is empty (likely the seed.vat string placeholder — ocaml-seed needs to lift Str payload)");
            }
            return buf.toOwnedSlice(world.allocator);
        },
        else => return typeError(world, ":load: expects a String path (Sym or String-Form)"),
    }
}

/// guard against absolute paths and `..`-traversal — mirror of
/// rust transporter.rs::load_relative path validation.
fn isUnsafePath(rel: []const u8) bool {
    if (rel.len == 0) return true;
    if (std.fs.path.isAbsolute(rel)) return true;
    // contains ".." segment?
    var it = std.mem.tokenizeAny(u8, rel, "/\\");
    while (it.next()) |seg| {
        if (std.mem.eql(u8, seg, "..")) return true;
    }
    return false;
}

/// parse + compile + run `source` against the in-image Parser / Compiler.
/// **requires** `use_moof_reader == true` and `use_moof_compiler == true`
/// because zig has no native parser/compiler. raises otherwise.
///
/// looks up `Parser` and `Compiler` by name in `world.here_form`'s slots
/// (the canonical bindings established by `lib/parser/03-bootstrap.moof`
/// and `lib/compiler/00-helpers.moof`). returns the last form's result.
pub fn evalStringInWorld(world: *World, source_val: Value) anyerror!Value {
    if (!world.use_moof_reader) {
        std.debug.print("evalStringInWorld: no moof reader, skipping\n", .{});
        return raise(world, "no-reader", "zig has no native reader; flip [$reader useMoof] first");
    }
    if (!world.use_moof_compiler) {
        std.debug.print("evalStringInWorld: no moof compiler, skipping\n", .{});
        return raise(world, "no-compiler", "zig has no native compiler; flip [$compiler useMoof] first");
    }

    // look up Parser + Compiler from $here.slots.
    const parser_sym = lookupSymByName(world, "Parser") orelse {
        std.debug.print("evalStringInWorld: Parser unbound\n", .{});
        return raise(world, "no-parser", "Parser symbol not interned; lib/parser/03-bootstrap.moof has not loaded");
    };
    const compiler_sym = lookupSymByName(world, "Compiler") orelse {
        std.debug.print("evalStringInWorld: Compiler unbound\n", .{});
        return raise(world, "no-compiler", "Compiler symbol not interned; lib/compiler/00-helpers.moof has not loaded");
    };
    const parser_v = world.envLookup(world.here_form, parser_sym) orelse {
        std.debug.print("evalStringInWorld: Parser lookup failed\n", .{});
        return raise(world, "no-parser", "Parser is unbound in $here — expected after parser/03-bootstrap.moof");
    };
    const compiler_v = world.envLookup(world.here_form, compiler_sym) orelse {
        std.debug.print("evalStringInWorld: Compiler lookup failed\n", .{});
        return raise(world, "no-compiler", "Compiler is unbound in $here — expected after compiler/00-helpers.moof");
    };

    const parse_sel = try world.syms.intern("parse:");
    const compile_top_sel = try world.syms.intern("compileTop:");

    // lukewarm: fires per eval. gated behind MOOF_TRACE per phase 2 §4.9.
    if (world.trace_enabled) std.debug.print("evalStringInWorld: parsing...\n", .{});
    // [Parser parse: source] → cons-chain of Forms.
    const forms_v = try world.send(parser_v, parse_sel, &.{source_val});

    // GC-anchor: each form's compile+run ends in a `runTop` which runs
    // a collect cycle. the parsed forms list lives in this zig local
    // and is NOT on the VM stack or VM frames, so the mid-eval-loop
    // GC would tombstone every cons-cell past the one we've already
    // pulled `:car` from — yielding `proto=.nil, slots=0` decoys whose
    // `:car` lookup returns nil and the loop quietly bails after one
    // form per file (root cause of why only the first defmacro per
    // file got registered before the polyglot self-host fix landed).
    //
    // pinning `forms_v` to the VM stack for the loop's duration adds
    // it to the GC's stack-root set; popping at the end restores the
    // pre-eval stack state.
    try world.vm.stack.append(world.allocator, forms_v);
    defer _ = world.vm.stack.pop();

    // iterate the forms, compile + runTop each. last result wins.
    var last: Value = .nil;
    var cur = forms_v;
    while (true) {
        switch (cur) {
            .nil => break,
            .form => |cid| {
                const cf = world.heap.get(cid);
                if (!cf.slotPresent(world.symCar)) break;
                const form_v = cf.slot(world.symCar);
                const cdr_v = cf.slot(world.symCdr);
                // [Compiler compileTop: form] → chunk-Form
                const chunk_v = try world.send(compiler_v, compile_top_sel, &.{form_v});
                const chunk_id = chunk_v.asFormId() orelse return raise(world, "no-chunk", "[Compiler compileTop:] did not return a chunk-Form");
                last = try @import("vm.zig").runTop(world, chunk_id);
                cur = cdr_v;
            },
            else => break,
        }
    }
    return last;
}

/// `[$transporter load: rel]` — resolve `rel` against transporter_root,
/// read the file, parse + compile + run it.
fn transporterLoad(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return raise(world, "arity", ":load: expects 1 arg (path)");
    const rel = try extractPath(world, args[0]);
    defer world.allocator.free(rel);

    // gated behind MOOF_TRACE (phase 2 §4.9 wart hunt — unbuffered
    // stderr writes per file load are non-trivial on cold disk caches;
    // keep available for diagnostic mode only).
    if (world.trace_enabled) std.debug.print("transporterLoad: {s}\n", .{rel});

    if (isUnsafePath(rel)) return raise(world, "tx-bad-path", ":load: refuses absolute or `..`-traversing paths");

    const root = world.transporter_root orelse {
        if (world.trace_enabled) std.debug.print("transporterLoad: no root configured\n", .{});
        return raise(world, "tx-no-root", "transporter has no root configured (set MOOF_LIB or place lib/ next to the binary)");
    };

    const io = world.io orelse return raise(world, "no-io", ":load: requires world.io to be set by host");

    // resolve rel against root, then read via world.io.
    const abs = try std.fs.path.join(world.allocator, &.{ root, rel });
    defer world.allocator.free(abs);

    const source = std.Io.Dir.cwd().readFileAlloc(io, abs, world.allocator, .limited(64 * 1024 * 1024)) catch |err| {
        std.debug.print("transporter load: read failed for {s}: {s}\n", .{ abs, @errorName(err) });
        return raise(world, "tx-read-error", ":load: file read failed");
    };
    defer world.allocator.free(source);

    return evalStringInWorld(world, try world.makeString(source));
}

/// `[$transporter loadAll: list]` — walk a cons of String paths,
/// `:load:` each. returns last result.
fn transporterLoadAll(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return raise(world, "arity", ":loadAll: expects 1 arg (cons of paths)");
    const list = args[0];
    var last: Value = .nil;
    var cur = list;
    while (true) {
        switch (cur) {
            .nil => break,
            .form => |id| {
                const f = world.heap.get(id);
                if (!f.slotPresent(world.symCar)) break;
                const car_v = f.slot(world.symCar);
                const cdr_v = f.slot(world.symCdr);
                last = try transporterLoad(world, self_, &.{car_v});
                cur = cdr_v;
            },
            else => return typeError(world, ":loadAll: expects a Cons"),
        }
    }
    return last;
}

// ─────────────────────────────────────────────────────────────────
// Free-function globals — bound on here_form's slots by NAME.
//
// the rust seed installs these via install_global (intrinsics.rs:2938):
// allocates a method-Form with proto=Method, registers a NativeFn,
// binds the form on here_form under the global's name. moof code
// calls them as `(name arg…)` — the parser produces a plain cons,
// the compiler lowers to `LoadName name` + `Send :call argc`.
//
// for the zig substrate, ocaml-seed's `build_seed_cmd.ml::wire_natives`
// pre-allocates one method-Form per `Global:name` REGISTRY entry,
// binds the form on here_form, and emits a matching NativeRefsSection
// entry. image-load rebinds the NativeFn via lookupNativeByName.
//
// dispatch path: send :call lands on the receiver method-Form;
// lookup walks proto chain to Method, finds methodCall handler;
// methodCall checks native_fns first (free-fn case) and delegates
// to the NativeFn directly with the call args.
// ─────────────────────────────────────────────────────────────────

// port of players/rust/src/intrinsics.rs::install_globals `setHandler!`.
// (setHandler! Proto 'sel fn) — install `fn` as handler for `sel` on
// `Proto`. bumps proto's generation so existing ICs invalidate (L10).
//
// receivers that are tagged immediates (e.g. `#true`, `#false`,
// `nil`, an Integer) route through `ensureWritableFormId` which
// lazily allocates a per-instance singleton-Form whose proto is the
// value's natural proto. matches rust's target_form_id flow —
// `(setHandler! #true 'ifTrue:ifFalse: …)` writes onto the #true
// singleton, not the Bool proto.
fn globalSetHandler(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 3) return raise(world, "arity", "(setHandler! Proto 'sel fn)");
    const proto_id = try world.ensureWritableFormId(args[0]);
    const sel = args[1].asSym() orelse return typeError(world, "setHandler!: selector must be a Symbol");
    const proto_form = world.heap.getMut(proto_id);
    try proto_form.handlers.put(world.allocator, sel, args[2]);
    try world.bumpGeneration(proto_id);
    return args[2];
}

// ─────────────────────────────────────────────────────────────────
// String / Char / Integer (char-related) primitives.
//
// the parser (lib/parser/00-lexer.moof) walks source one char at a
// time via `[src at: i]` + `[c codepoint]`. all moof Strings inside
// the V4 alpha image are stored as `{ proto=String, slots={bytes:
// cons-chain-of-Char} }` (see ocaml-seed's build_form_for/Str).
// the natives below honor that shape; once a real String storage
// layer lands they can be rewritten without changing the moof side.
// ─────────────────────────────────────────────────────────────────

/// walk a String-form's :bytes cons-chain. returns the count of
/// codepoints traversed, and (if `target >= 0`) the codepoint at the
/// 0-based index `target` (or -1 if past end).
// **§5.8a** — get the cached `[]u32` for a String FormId, bumping
// hit/miss counters. wraps `world.getStringChars` to centralize the
// profile instrumentation. returns null on malformed `:bytes`.
fn cachedStringChars(world: *World, id: form.FormId) !?[]const u32 {
    if (world.string_cache.contains(id)) {
        vm_mod.PROFILE.string_cache_hits += 1;
    } else {
        vm_mod.PROFILE.string_cache_misses += 1;
    }
    return world.getStringChars(id);
}

// port of players/rust/src/intrinsics.rs::install_string_methods `:length`
fn stringLength(world: *World, self_: Value, _: []const Value) anyerror!Value {
    if (self_ != .form) return typeError(world, "length: receiver must be a String");
    const id = self_.asFormId().?;
    const chars = (try cachedStringChars(world, id)) orelse return typeError(world, "length: malformed String");
    return .{ .int = @intCast(chars.len) };
}

// port of players/rust/src/intrinsics.rs::install_string_methods `:at:`
// returns the Char at the given index, or raises 'index-out-of-bounds.
fn stringAt(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return raise(world, "arity", "at: takes 1 arg");
    const idx = args[0].asInt() orelse return typeError(world, "at: index must be an Integer");
    if (idx < 0) return raise(world, "index-out-of-bounds", "[String at: i] negative index");
    if (self_ != .form) return typeError(world, "at: receiver must be a String");
    const id = self_.asFormId().?;
    const chars = (try cachedStringChars(world, id)) orelse return typeError(world, "at: malformed String");
    if (idx >= @as(i64, @intCast(chars.len))) {
        return raise(world, "index-out-of-bounds", "[String at: i] out of range");
    }
    return .{ .char = chars[@intCast(idx)] };
}

// port of players/rust/src/intrinsics.rs::install_string_methods `:=`
// structural equality — compare the two :bytes cons-chains element by
// element. accepts Strings or anything String-shaped (.bytes cons-chain
// of Chars); mismatched shape → false (never raises).
fn stringEq(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return .{ .bool_ = false };
    const bytes_sym = lookupSymByName(world, "bytes") orelse return .{ .bool_ = false };
    if (self_ != .form or args[0] != .form) return .{ .bool_ = false };
    const a_id = self_.asFormId().?;
    const b_id = args[0].asFormId().?;
    var a_cur = world.formSlot(a_id, bytes_sym);
    var b_cur = world.formSlot(b_id, bytes_sym);
    while (true) {
        const a_done = (a_cur == .nil);
        const b_done = (b_cur == .nil);
        if (a_done and b_done) return .{ .bool_ = true };
        if (a_done or b_done) return .{ .bool_ = false };
        if (a_cur != .form or b_cur != .form) return .{ .bool_ = false };
        const af = world.heap.get(a_cur.asFormId().?);
        const bf = world.heap.get(b_cur.asFormId().?);
        if (!af.slotPresent(world.symCar) or !bf.slotPresent(world.symCar)) return .{ .bool_ = false };
        const a_car = af.slot(world.symCar);
        const b_car = bf.slot(world.symCar);
        const a_cp: u32 = switch (a_car) { .char => |c| c, else => return .{ .bool_ = false } };
        const b_cp: u32 = switch (b_car) { .char => |c| c, else => return .{ .bool_ = false } };
        if (a_cp != b_cp) return .{ .bool_ = false };
        a_cur = af.slot(world.symCdr);
        b_cur = bf.slot(world.symCdr);
    }
}

// port of players/rust/src/intrinsics.rs::install_string_methods `:slice:length:`
// substring by char-index. allocates a new String-Form with a fresh
// :bytes cons-chain.
//
// **§5.8a** — sources its codepoints from the char-cache, so the
// `start`-char skip + `len`-char collect is O(start+len) array reads
// instead of O(start+len) cons-walks. cache populates lazily on
// first access.
fn stringSlice(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len < 2) return raise(world, "arity", "slice:length: takes 2 args");
    const start = args[0].asInt() orelse return typeError(world, "slice:length: needs Integer start");
    const len_arg = args[1].asInt() orelse return typeError(world, "slice:length: needs Integer length");
    if (start < 0 or len_arg < 0) return raise(world, "index-out-of-bounds", "slice:length: negative start or length");
    if (self_ != .form) return typeError(world, "slice:length: receiver must be a String");
    const id = self_.asFormId().?;
    const chars = (try cachedStringChars(world, id)) orelse return typeError(world, "slice:length: malformed String");
    const start_u: usize = @intCast(start);
    const len_u: usize = @intCast(len_arg);
    // clamp end at chars.len (matches the cons-walk's nil-terminated stop).
    const end_u = @min(chars.len, start_u +| len_u);
    const begin_u = @min(chars.len, start_u);
    var collected: std.ArrayList(Value) = .empty;
    defer collected.deinit(world.allocator);
    var i: usize = begin_u;
    while (i < end_u) : (i += 1) {
        try collected.append(world.allocator, .{ .char = chars[i] });
    }
    const chain = try world.makeList(collected.items);
    var f = Form.withProto(.{ .form = world.protos.string });
    const bytes_sym = world.symBytes;
    try f.slots.put(world.allocator, bytes_sym, chain);
    const new_id = try world.heap.alloc(f);
    return .{ .form = new_id };
}

// port of players/rust/src/intrinsics.rs::install_string_methods `:+`
// concatenation. accepts a String on the right (cons-chain shape);
// for Sym/Char rhs we coerce via :toString-like inline handling.
fn stringPlus(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return raise(world, "arity", "+ takes 1 arg");
    if (self_ != .form) return typeError(world, "+: receiver must be a String");
    const bytes_sym = lookupSymByName(world, "bytes") orelse return raise(world, "no-bytes-sym", "+: no bytes sym");
    // collect chars from self.
    var collected: std.ArrayList(Value) = .empty;
    defer collected.deinit(world.allocator);
    {
        var cur = world.formSlot(self_.asFormId().?, bytes_sym);
        while (true) {
            switch (cur) {
                .nil => break,
                .form => |cid| {
                    const cf = world.heap.get(cid);
                    if (!cf.slotPresent(world.symCar)) break;
                    const car_v = cf.slot(world.symCar);
                    switch (car_v) {
                        .char => |cp| try collected.append(world.allocator, .{ .char = cp }),
                        else => break,
                    }
                    cur = cf.slot(world.symCdr);
                },
                else => break,
            }
        }
    }
    // append chars from rhs. accepts a String-Form with :bytes, or a
    // single Char (treated as one-element string).
    const rhs = args[0];
    switch (rhs) {
        .char => |cp| try collected.append(world.allocator, .{ .char = cp }),
        .form => |rid| {
            // walk rhs's :bytes (Char chain).
            var cur = world.formSlot(rid, bytes_sym);
            while (true) {
                switch (cur) {
                    .nil => break,
                    .form => |cid| {
                        const cf = world.heap.get(cid);
                        if (!cf.slotPresent(world.symCar)) break;
                        const car_v = cf.slot(world.symCar);
                        switch (car_v) {
                            .char => |cp| try collected.append(world.allocator, .{ .char = cp }),
                            else => break,
                        }
                        cur = cf.slot(world.symCdr);
                    },
                    else => break,
                }
            }
        },
        .sym => |s| {
            // append each codepoint of the symbol's text.
            const text = world.syms.resolve(s);
            var it = std.unicode.Utf8Iterator{ .bytes = text, .i = 0 };
            while (it.nextCodepoint()) |cp| {
                try collected.append(world.allocator, .{ .char = @intCast(cp) });
            }
        },
        else => return typeError(world, "+: rhs must be a String, Char, or Sym"),
    }
    const chain = try world.makeList(collected.items);
    var f = Form.withProto(.{ .form = world.protos.string });
    try f.slots.put(world.allocator, bytes_sym, chain);
    const new_id = try world.heap.alloc(f);
    return .{ .form = new_id };
}

// ─────────────────────────────────────────────────────────────────
// Char primitives.
// ─────────────────────────────────────────────────────────────────

// port of players/rust/src/intrinsics.rs::install_char_methods `:codepoint`
fn charCodepoint(world: *World, self_: Value, _: []const Value) anyerror!Value {
    return switch (self_) {
        .char => |cp| .{ .int = @intCast(cp) },
        else => typeError(world, "codepoint on non-Char"),
    };
}

// port of players/rust/src/intrinsics.rs::install_char_methods `:<`
fn charLt(_: *World, self_: Value, args: []const Value) anyerror!Value {
    return switch (self_) {
        .char => |a| switch (args[0]) {
            .char => |b| .{ .bool_ = a < b },
            else => .{ .bool_ = false },
        },
        else => .{ .bool_ = false },
    };
}

// `:toString` on Char returns a one-character String. The Object
// fallback renders a Char as its single utf-8 character (used for
// e.g. say:), but the moof Lexer wants a String-Form here so it can
// concat via :+. allocate a one-char String.
fn charToString(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const cp = switch (self_) {
        .char => |c| c,
        else => return typeError(world, "toString on non-Char"),
    };
    const bytes_sym = try world.syms.intern("bytes");
    const one = [_]Value{ .{ .char = cp } };
    const chain = try world.makeList(&one);
    var f = Form.withProto(.{ .form = world.protos.string });
    try f.slots.put(world.allocator, bytes_sym, chain);
    const new_id = try world.heap.alloc(f);
    return .{ .form = new_id };
}

// ─────────────────────────────────────────────────────────────────
// Integer:asChar — coerce an Integer to a Char (codepoint).
// used by the Lexer's escape table: `[10 asChar]` → newline-Char.
// ─────────────────────────────────────────────────────────────────

// port of players/rust/src/intrinsics.rs (Integer `:asChar`)
fn intAsChar(world: *World, self_: Value, _: []const Value) anyerror!Value {
    const n = self_.asInt() orelse return typeError(world, "asChar receiver must be an Integer");
    if (n < 0 or n > 0x10_FFFF) return raise(world, "index-out-of-bounds", "asChar: codepoint out of range");
    return .{ .char = @intCast(n) };
}

// ─────────────────────────────────────────────────────────────────
// `intern` — global free function. `(intern "name")` returns the
// interned Symbol. accepts a String-Form (cons-chain of chars) or
// a Symbol (already interned, identity return).
// ─────────────────────────────────────────────────────────────────

// `(cons head tail)` — alloc a cons cell. §5.8b — flat-cons fast path.
fn globalCons(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "(cons h tail) takes 2 args");
    const id = try world.allocFlatCons(args[0], args[1]);
    return .{ .form = id };
}

// `(list a b c …)` — variadic list constructor.
fn globalList(world: *World, _: Value, args: []const Value) anyerror!Value {
    return world.makeList(args);
}

// `(slot v 'name)` — direct slot access (no proto-walk).
fn globalSlot(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "(slot v 'name)");
    const name = args[1].asSym() orelse return typeError(world, "slot: name must be a Symbol");
    const id = args[0].asFormId() orelse return .nil;
    return world.formSlot(id, name);
}

// `(slotSet! v 'name value)` — write a slot. tagged-immediate receivers
// allocate a singleton-Form via ensureWritableFormId (same flow as
// setHandler!).
fn globalSlotSet(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 3) return raise(world, "arity", "(slotSet! v 'name value)");
    const id = try world.ensureWritableFormId(args[0]);
    const name = args[1].asSym() orelse return typeError(world, "slotSet!: name must be a Symbol");
    try world.formSlotSet(id, name, args[2]);
    return args[2];
}

// `(metaSet! v 'name value)` — write a meta slot.
fn globalMetaSet(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 3) return raise(world, "arity", "(metaSet! v 'name value)");
    const id = try world.ensureWritableFormId(args[0]);
    const name = args[1].asSym() orelse return typeError(world, "metaSet!: name must be a Symbol");
    const fm = world.heap.getMut(id);
    try fm.meta.put(world.allocator, name, args[2]);
    return args[2];
}

// `(globalEnv)` — return the canonical $here Form.
fn globalGlobalEnv(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "(globalEnv) takes no args");
    return .{ .form = world.here_form };
}

// `(getOrCreateProto 'Name Parent)` — defproto reopen helper.
// returns the existing Form if `Name` is already bound to one;
// otherwise alloc fresh with proto = Parent, bind, return.
fn globalGetOrCreateProto(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "(getOrCreateProto 'Name Parent)");
    const name_sym = args[0].asSym() orelse return typeError(world, "getOrCreateProto: name must be a Symbol");
    if (world.envLookup(world.here_form, name_sym)) |existing| {
        if (existing == .form) return existing;
    }
    var f = Form.withProto(args[1]);
    const name_meta = try world.syms.intern("name");
    try f.meta.put(world.allocator, name_meta, .{ .sym = name_sym });
    const id = try world.heap.alloc(f);
    try world.envBind(world.here_form, name_sym, .{ .form = id });
    return .{ .form = id };
}

// `(append xs ys …)` — concatenate cons-lists left-to-right.
fn globalAppend(world: *World, _: Value, args: []const Value) anyerror!Value {
    var out: std.ArrayList(Value) = .empty;
    defer out.deinit(world.allocator);
    for (args) |arg| {
        var cur = arg;
        while (cur == .form) {
            const fid = cur.asFormId().?;
            const f = world.heap.get(fid);
            // accept any cons-shaped form (has :car slot).
            if (!f.slotPresent(world.symCar)) break;
            const head = f.slot(world.symCar);
            const tail = f.slot(world.symCdr);
            try out.append(world.allocator, head);
            cur = tail;
        }
    }
    return world.makeList(out.items);
}

// `(macroexpand '(foo a b …))` — expand the macro registered as `foo`.
// raises if `foo` isn't a registered macro.
fn globalMacroexpand(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "(macroexpand 'form)");
    const fid = args[0].asFormId() orelse return raise(world, "macroexpand", "form must be a cons-list");
    const f = world.heap.get(fid);
    if (!f.slotPresent(world.symCar)) return raise(world, "macroexpand", "empty form");
    const head = f.slot(world.symCar);
    const head_sym = head.asSym() orelse return raise(world, "macroexpand", "form head is not a Symbol");
    // look up the macro on world.macros_form's slots.
    const macro_v = world.formSlot(world.macros_form, head_sym);
    if (macro_v == .nil) return raise(world, "macroexpand", "not a macro");
    const mid = macro_v.asFormId() orelse return raise(world, "macroexpand", "macro entry is not a Form");
    // macro is invoked with one arg = the args-list (the form's cdr).
    const args_list = f.slot(world.symCdr);
    // dispatch via Method:call so captured-self / env are honored.
    const call_sym = try world.syms.intern("call");
    return world.send(.{ .form = mid }, call_sym, &.{args_list});
}

// `(raise: kind message)` — raise a moof-level error.
fn globalRaise(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "(raise: kind message)");
    const kind_sym = args[0].asSym() orelse return typeError(world, "raise:: kind must be a Symbol");
    _ = args[1]; // message: not yet propagated through the raise structure.
    // V4 phase α: substrate uses a simple error.DispatchError + stderr
    // log. propagate the kind symbol's name so callers know what's up.
    const kind_name = world.syms.resolve(kind_sym);
    return world.raise(kind_name, "raised from moof");
}

// **§5.8c** — String→Sym intern cache. profile shows 267K intern
// calls / 653 unique syms = 410× redundancy when the parser re-uses a
// pre-interned String identity (e.g. compiler-internal identifier
// tables that hold onto the parsed-name String-Form across passes).
// caching String FormId → SymId reduces those redundant calls to a
// single hashmap probe.
//
// the per-call walk (cons → UTF-8 buffer → `syms.intern`) is still on
// the miss path, but now uses the §5.8a char-cache for the cons walk,
// so each unique String is materialized into a `[]u32` exactly once.
//
// safety: the cache is in `world.intern_cache`; invalidation lives on
// `world.formSlotSet` (when `:bytes` rewrites) and `gc.sweepSideTables`
// (when a String FormId tombstones). by L11 the FormId itself is
// stable for the lifetime of the vat, so a hit always points at the
// canonical SymId for this string's content.
//
// note: when the parser builds a fresh String per identifier (cf.
// lib/parser/00-lexer.moof), the FormId is unique per call → all
// misses. that's the §5.10 "memoize at the parser level" territory;
// this cache only helps when the caller holds onto a String. cheap
// to install regardless — the miss path is no slower than the
// pre-cache code.
fn globalIntern(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len < 1) return raise(world, "arity", "(intern name) takes 1 arg");
    const arg = args[0];
    // Symbol identity passthrough.
    if (arg == .sym) return arg;
    // String-Form: cache lookup first.
    const id = arg.asFormId() orelse return typeError(world, "intern: arg must be a String or Sym");
    if (world.intern_cache.get(id)) |cached_sym| {
        vm_mod.PROFILE.intern_cache_hits += 1;
        return .{ .sym = cached_sym };
    }
    vm_mod.PROFILE.intern_cache_misses += 1;

    // miss — use the §5.8a char-cache to grab the codepoints (one
    // cons-walk per unique String FormId), then UTF-8 encode into a
    // local buffer.
    const chars = (try cachedStringChars(world, id)) orelse return raise(world, "intern", "malformed String");
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(world.allocator);
    try buf.ensureTotalCapacity(world.allocator, chars.len);
    for (chars) |cp| {
        const cp_u21 = std.math.cast(u21, cp) orelse return raise(world, "intern", "bad char");
        var tmp: [4]u8 = undefined;
        const n = std.unicode.utf8Encode(cp_u21, &tmp) catch return raise(world, "intern", "un-encodable char");
        try buf.appendSlice(world.allocator, tmp[0..n]);
    }
    const sid = try world.syms.intern(buf.items);
    // populate cache. errors here are non-fatal — the cache is a soft
    // optimization, and we already have a valid SymId to return.
    world.intern_cache.put(world.allocator, id, sid) catch {};
    return .{ .sym = sid };
}

// ─────────────────────────────────────────────────────────────────
// Chunk class- and instance-side methods — port of
// players/rust/src/intrinsics.rs::install_compiler_primitives'
// chunk subset.
//
// the moof Compiler (lib/compiler/*.moof) builds chunks by sending:
//   [Chunk new: params source: src]
//   [chunk emit: opcode-form]
//   [chunk addConst: v]
//   [chunk addIc]
//   [chunk jumpTarget]
//   [chunk patchJump: pos to: tgt]
//   [chunk asClosure]
//
// rust stores per-chunk `chunk_ops` (Vec<Op>) and finalizes to bytes
// only on serialization. zig has only `chunk_bytecode` (encoded
// bytes). we eagerly encode each emit:'d op into the bytecode
// buffer; patchJump:to: rewrites the i16 offset bytes in place at
// the recorded position (jumpTarget / emit return *byte positions*,
// not op-list indices, so the encoded layout is the authoritative
// addressing model).
//
// the moof Compiler's patchJump:to: signature passes `pos` (returned
// by the earlier emit Jump 0) and `tgt` (returned by jumpTarget at
// the patching point), then expects the substrate to compute the
// V4-spec offset `tgt - pos - 3`. mirrors rust's patchJump:to: post
// the byte-offset refactor in ocaml-seed.
// ─────────────────────────────────────────────────────────────────

/// decode an opcode-Form (slots `:op` Sym + `:operands` cons-list)
/// into an Op. mirrors players/rust/src/intrinsics.rs::decode_op_form.
fn decodeOpForm(world: *World, v: Value) anyerror!Op {
    const id = v.asFormId() orelse return typeError(world, "chunk-emit: opcode must be a Form");
    const op_sym = try world.syms.intern("op");
    const operands_sym = try world.syms.intern("operands");
    const name_v = world.formSlot(id, op_sym);
    const name_sid = name_v.asSym() orelse return raise(world, "compile-error", "opcode :op must be a Symbol");
    const name = world.syms.resolve(name_sid);
    // operands as a cons-chain; walk into a slice for indexed access.
    const operands_v = world.formSlot(id, operands_sym);
    var operands: []Value = &.{};
    if (operands_v != .nil) {
        operands = try world.listToSlice(operands_v);
    }
    defer if (operands.len > 0) world.freeSlice(operands);

    // Each operand-typed helper, inline.
    if (std.mem.eql(u8, name, "PushNil")) return Op.push_nil;
    if (std.mem.eql(u8, name, "PushTrue")) return Op.push_true;
    if (std.mem.eql(u8, name, "PushFalse")) return Op.push_false;
    if (std.mem.eql(u8, name, "Pop")) return Op.pop;
    if (std.mem.eql(u8, name, "Dup")) return Op.dup;
    if (std.mem.eql(u8, name, "LoadSelf")) return Op.load_self;
    if (std.mem.eql(u8, name, "Return")) return Op.return_op;

    if (std.mem.eql(u8, name, "LoadConst")) {
        if (operands.len < 1) return raise(world, "compile-error", "LoadConst needs 1 operand");
        const idx_i = operands[0].asInt() orelse return raise(world, "compile-error", "LoadConst operand must be Integer");
        if (idx_i < 0 or idx_i > std.math.maxInt(u16)) return raise(world, "range-error", "LoadConst idx out of u16 range");
        return Op{ .load_const = .{ .idx = @intCast(idx_i) } };
    }
    if (std.mem.eql(u8, name, "LoadName")) {
        if (operands.len < 1) return raise(world, "compile-error", "LoadName needs 1 operand");
        const sym = operands[0].asSym() orelse return raise(world, "compile-error", "LoadName operand must be Symbol");
        return Op{ .load_name = .{ .name = sym } };
    }
    if (std.mem.eql(u8, name, "PushClosure")) {
        if (operands.len < 1) return raise(world, "compile-error", "PushClosure needs 1 operand");
        const fid = operands[0].asFormId() orelse return raise(world, "compile-error", "PushClosure operand must be Form");
        return Op{ .push_closure = .{ .chunk = fid } };
    }
    if (std.mem.eql(u8, name, "Jump")) {
        if (operands.len < 1) return raise(world, "compile-error", "Jump needs 1 operand");
        const off_i = operands[0].asInt() orelse return raise(world, "compile-error", "Jump operand must be Integer");
        if (off_i < std.math.minInt(i16) or off_i > std.math.maxInt(i16)) return raise(world, "range-error", "Jump offset out of i16 range");
        return Op{ .jump = .{ .offset = @intCast(off_i) } };
    }
    if (std.mem.eql(u8, name, "JumpIfFalse")) {
        if (operands.len < 1) return raise(world, "compile-error", "JumpIfFalse needs 1 operand");
        const off_i = operands[0].asInt() orelse return raise(world, "compile-error", "JumpIfFalse operand must be Integer");
        if (off_i < std.math.minInt(i16) or off_i > std.math.maxInt(i16)) return raise(world, "range-error", "JumpIfFalse offset out of i16 range");
        return Op{ .jump_if_false = .{ .offset = @intCast(off_i) } };
    }
    if (std.mem.eql(u8, name, "Send")) {
        if (operands.len < 3) return raise(world, "compile-error", "Send needs 3 operands");
        const sel = operands[0].asSym() orelse return raise(world, "compile-error", "Send selector must be Symbol");
        const argc_i = operands[1].asInt() orelse return raise(world, "compile-error", "Send argc must be Integer");
        const ic_i = operands[2].asInt() orelse return raise(world, "compile-error", "Send ic must be Integer");
        if (argc_i < 0 or argc_i > std.math.maxInt(u8)) return raise(world, "range-error", "Send argc out of u8 range");
        if (ic_i < 0 or ic_i > std.math.maxInt(u16)) return raise(world, "range-error", "Send ic out of u16 range");
        return Op{ .send = .{ .selector = sel, .argc = @intCast(argc_i), .ic_idx = @intCast(ic_i) } };
    }
    if (std.mem.eql(u8, name, "TailSend")) {
        if (operands.len < 2) return raise(world, "compile-error", "TailSend needs 2 operands");
        const sel = operands[0].asSym() orelse return raise(world, "compile-error", "TailSend selector must be Symbol");
        const argc_i = operands[1].asInt() orelse return raise(world, "compile-error", "TailSend argc must be Integer");
        if (argc_i < 0 or argc_i > std.math.maxInt(u8)) return raise(world, "range-error", "TailSend argc out of u8 range");
        return Op{ .tail_send = .{ .selector = sel, .argc = @intCast(argc_i) } };
    }
    if (std.mem.eql(u8, name, "SuperSend")) {
        if (operands.len < 3) return raise(world, "compile-error", "SuperSend needs 3 operands");
        const sel = operands[0].asSym() orelse return raise(world, "compile-error", "SuperSend selector must be Symbol");
        const argc_i = operands[1].asInt() orelse return raise(world, "compile-error", "SuperSend argc must be Integer");
        const ic_i = operands[2].asInt() orelse return raise(world, "compile-error", "SuperSend ic must be Integer");
        if (argc_i < 0 or argc_i > std.math.maxInt(u8)) return raise(world, "range-error", "SuperSend argc out of u8 range");
        if (ic_i < 0 or ic_i > std.math.maxInt(u16)) return raise(world, "range-error", "SuperSend ic out of u16 range");
        return Op{ .super_send = .{ .selector = sel, .argc = @intCast(argc_i), .ic_idx = @intCast(ic_i) } };
    }
    if (std.mem.eql(u8, name, "SendSelf")) {
        if (operands.len < 3) return raise(world, "compile-error", "SendSelf needs 3 operands");
        const sel = operands[0].asSym() orelse return raise(world, "compile-error", "SendSelf selector must be Symbol");
        const argc_i = operands[1].asInt() orelse return raise(world, "compile-error", "SendSelf argc must be Integer");
        const ic_i = operands[2].asInt() orelse return raise(world, "compile-error", "SendSelf ic must be Integer");
        if (argc_i < 0 or argc_i > std.math.maxInt(u8)) return raise(world, "range-error", "SendSelf argc out of u8 range");
        if (ic_i < 0 or ic_i > std.math.maxInt(u16)) return raise(world, "range-error", "SendSelf ic out of u16 range");
        return Op{ .send_self = .{ .selector = sel, .argc = @intCast(argc_i), .ic_idx = @intCast(ic_i) } };
    }
    return raise(world, "compile-error", "decodeOpForm: unknown op name");
}

/// resolve a Chunk receiver — `chunk` or a Closure pointing at a chunk.
fn chunkIdSelf(world: *World, self_: Value) anyerror!FormId {
    const id = self_.asFormId() orelse return typeError(world, "chunk method: receiver must be a Form");
    if (world.chunk_bytecode.contains(id)) return id;
    const body_v = world.formSlot(id, world.body_sym);
    if (body_v.asFormId()) |bid| {
        if (world.chunk_bytecode.contains(bid)) return bid;
    }
    return typeError(world, "chunk method: receiver is not a chunk");
}

// [Chunk new: params source: source]
fn chunkNewSource(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "[Chunk new: ps source: src] takes 2 args");
    const params_v = args[0];
    const source_v = args[1];
    // validate + collect param sym ids.
    const params_slice = try world.listToSlice(params_v);
    defer world.freeSlice(params_slice);
    var param_syms = try world.allocator.alloc(u32, params_slice.len);
    errdefer world.allocator.free(param_syms);
    for (params_slice, 0..) |p, i| {
        const ps = p.asSym() orelse return typeError(world, "Chunk new:source:: each param must be a Symbol");
        param_syms[i] = ps;
    }
    // allocate the chunk-Form with :params + :source meta.
    const source_sym = try world.syms.intern("source");
    var chunk_form = Form.withProto(.{ .form = world.protos.chunk });
    try chunk_form.slots.put(world.allocator, world.params_sym, params_v);
    try chunk_form.meta.put(world.allocator, source_sym, source_v);
    const chunk_id = try world.heap.alloc(chunk_form);
    // register side tables.
    const empty_bytes = try world.allocator.alloc(u8, 0);
    try world.chunk_bytecode.put(world.allocator, chunk_id, empty_bytes);
    const empty_consts = try world.allocator.alloc(Value, 0);
    try world.chunk_consts.put(world.allocator, chunk_id, empty_consts);
    const empty_ics = try world.allocator.alloc(ICache, 0);
    try world.chunk_ics.put(world.allocator, chunk_id, empty_ics);
    try world.chunk_params.put(world.allocator, chunk_id, param_syms);
    return .{ .form = chunk_id };
}

// [chunk emit: op-form] — encode op + append to bytecode; return byte position.
fn chunkEmit(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "[chunk emit: op] takes 1 arg");
    const chunk_id = try chunkIdSelf(world, self_);
    const op = try decodeOpForm(world, args[0]);
    // load existing bytecode into an ArrayList, append, store back.
    const existing = world.chunk_bytecode.get(chunk_id).?;
    var buf: std.ArrayList(u8) = .empty;
    try buf.appendSlice(world.allocator, existing);
    const pos = buf.items.len;
    try bytecode_mod.encodeOp(op, &buf, world.allocator);
    world.allocator.free(existing);
    const owned = try buf.toOwnedSlice(world.allocator);
    try world.chunk_bytecode.put(world.allocator, chunk_id, owned);
    return .{ .int = @intCast(pos) };
}

// [chunk addConst: v] — append to consts pool; return idx.
fn chunkAddConst(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 1) return raise(world, "arity", "[chunk addConst: v] takes 1 arg");
    const chunk_id = try chunkIdSelf(world, self_);
    const existing = world.chunk_consts.get(chunk_id).?;
    if (existing.len >= std.math.maxInt(u16)) return raise(world, "range-error", "addConst:: pool exceeds 65535");
    const new = try world.allocator.alloc(Value, existing.len + 1);
    @memcpy(new[0..existing.len], existing);
    new[existing.len] = args[0];
    world.allocator.free(existing);
    try world.chunk_consts.put(world.allocator, chunk_id, new);
    return .{ .int = @intCast(existing.len) };
}

// [chunk addIc] — reserve IC slot; return idx.
fn chunkAddIc(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[chunk addIc] takes no args");
    const chunk_id = try chunkIdSelf(world, self_);
    const existing = world.chunk_ics.get(chunk_id).?;
    if (existing.len >= std.math.maxInt(u16)) return raise(world, "range-error", "addIc: pool exceeds 65535");
    const new = try world.allocator.alloc(ICache, existing.len + 1);
    @memcpy(new[0..existing.len], existing);
    new[existing.len] = ICache.empty;
    world.allocator.free(existing);
    try world.chunk_ics.put(world.allocator, chunk_id, new);
    return .{ .int = @intCast(existing.len) };
}

// [chunk jumpTarget] — current bytecode length (byte position of next emit).
fn chunkJumpTarget(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[chunk jumpTarget] takes no args");
    const chunk_id = try chunkIdSelf(world, self_);
    const bytes = world.chunk_bytecode.get(chunk_id).?;
    return .{ .int = @intCast(bytes.len) };
}

// [chunk patchJump: pos to: tgt] — overwrite the offset bytes at the
// jump op located at byte `pos`. V4 spec §3.4: offset is relative to
// the byte AFTER the 3-byte jump op, so we encode `tgt - pos - 3`.
fn chunkPatchJump(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "[chunk patchJump: pos to: tgt] takes 2 args");
    const chunk_id = try chunkIdSelf(world, self_);
    const pos_i = args[0].asInt() orelse return typeError(world, "patchJump:to:: pos must be Integer");
    const tgt_i = args[1].asInt() orelse return typeError(world, "patchJump:to:: target must be Integer");
    if (pos_i < 0) return raise(world, "range-error", "patchJump:to:: pos must be non-negative");
    const off = tgt_i - pos_i - 3;
    if (off < std.math.minInt(i16) or off > std.math.maxInt(i16)) return raise(world, "range-error", "patchJump:to:: offset doesn't fit i16");
    const bytes = world.chunk_bytecode.getPtr(chunk_id).?;
    const pos: usize = @intCast(pos_i);
    if (pos + 3 > bytes.*.len) return raise(world, "range-error", "patchJump:to:: pos out of range");
    // bytes at [pos] should be a Jump/JumpIfFalse/JumpIfTrue tag.
    const tag = bytes.*[pos];
    if (tag != 0x30 and tag != 0x31 and tag != 0x32) return raise(world, "compile-error", "patchJump:to:: op at pos is not a jump");
    const off_i16: i16 = @intCast(off);
    bytes.*[pos + 1] = @intCast((@as(u16, @bitCast(off_i16)) >> 8) & 0xff);
    bytes.*[pos + 2] = @intCast(@as(u16, @bitCast(off_i16)) & 0xff);
    return .nil;
}

// [chunk asClosure] — wrap a chunk in a closure-Form ready to call.
fn chunkAsClosure(world: *World, self_: Value, args: []const Value) anyerror!Value {
    if (args.len != 0) return raise(world, "arity", "[chunk asClosure] takes no args");
    const chunk_id = try chunkIdSelf(world, self_);
    var f = Form.withProto(.{ .form = world.protos.closure });
    try f.slots.put(world.allocator, world.body_sym, .{ .form = chunk_id });
    try f.slots.put(world.allocator, world.env_sym, .{ .form = world.here_form });
    const captured_self_sym = try world.syms.intern("captured-self");
    try f.slots.put(world.allocator, captured_self_sym, .nil);
    const params_v = world.formSlot(chunk_id, world.params_sym);
    try f.slots.put(world.allocator, world.params_sym, params_v);
    const source_sym = try world.syms.intern("source");
    const source_v = world.formMeta(chunk_id, source_sym);
    if (source_v != .nil) {
        try f.meta.put(world.allocator, source_sym, source_v);
    }
    return .{ .form = try world.heap.alloc(f) };
}

// ─────────────────────────────────────────────────────────────────
// $layout cap — register a Layout for a user proto.
//
// `[$layout register: Proto slots: '(s1 s2 ...)]`
//
// walks the cons-list of slot-syms, interns nothing (the syms are
// already interned by the reader / quasiquote expansion) and calls
// world.registerLayout. nil-return — moof code typically discards.
//
// idempotent on identical schemas (World.registerLayout returns the
// existing Layout pointer); raises LayoutMismatch on schema change.
// the slot count must be ≤ form.INLINE_CAPACITY (currently 4).
//
// installed by installCaps under `$layout` on here_form. used by
// the defproto macro to give every user-declared proto inline-slot
// storage automatically.
// ─────────────────────────────────────────────────────────────────

fn layoutRegisterSlots(world: *World, _: Value, args: []const Value) anyerror!Value {
    if (args.len != 2) return raise(world, "arity", "[$layout register: Proto slots: '(...)] takes 2 args");
    const proto_id = args[0].asFormId() orelse return typeError(world, "register:slots: proto must be a Form");
    var slot_names: std.ArrayList(SymId) = .empty;
    defer slot_names.deinit(world.allocator);
    var cur = args[1];
    while (true) {
        switch (cur) {
            .nil => break,
            .form => |fid| {
                const f = world.heap.get(fid);
                if (!f.slotPresent(world.symCar)) break;
                const car = f.slot(world.symCar);
                const sym = car.asSym() orelse
                    return typeError(world, "register:slots: slot name must be a Symbol");
                try slot_names.append(world.allocator, sym);
                cur = f.slot(world.symCdr);
            },
            else => return typeError(world, "register:slots: slot list must be a Cons or nil"),
        }
    }
    _ = try world.registerLayout(proto_id, slot_names.items);
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
    .{ "Opcode:sendSelf:argc:ic:", opcodeSendSelfArgcIc },

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
    .{ "Heap:slotKeysOf:", heapSlotKeysOf },
    .{ "Heap:handlerKeysOf:", heapHandlerKeysOf },
    .{ "Heap:metaKeysOf:", heapMetaKeysOf },

    // Method:call — invoke a method/closure form.
    .{ "Method:call", methodCall },

    // Object basics.
    .{ "Object:=", objEq },
    .{ "Object:new", objNew },
    // Object:initialize removed — canonical is stdlib/object.moof (defmethod Object (initialize) self)
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

    // $transporter (W5b — port of players/rust/src/transporter.rs).
    // names match what rust would emit if its anonymous-proto issue
    // were resolved: in rust the transporter proto is anonymous, so
    // v4_export emits `<anon-N>:load:` which won't match here. these
    // entries serve as the canonical zig-side surface that the host
    // installs via the install-cap helper below (intrinsics.install).
    .{ "Transporter:load:", transporterLoad },
    .{ "Transporter:loadAll:", transporterLoadAll },

    // $compiler / $reader flag-flip caps (W5b — flag-flip primitives
    // mirror rust install_compiler_cap / install_reader_cap). same
    // anonymous-proto caveat as $transporter; host wires these via
    // a `setCompilerCap` helper at boot rather than the image's
    // NativeRefsSection.
    .{ "Compiler:useMoof", compilerUseMoof },
    .{ "Compiler:useSeed", compilerUseSeed },
    .{ "Reader:useMoof", readerUseMoof },
    .{ "Reader:useSeed", readerUseSeed },

    // Free-function globals — bound on here_form by name (no proto).
    // ocaml-seed allocates one method-Form per entry and emits a
    // NativeRefs binding under the `Global:NAME` key. moof code calls
    // these as `(name arg…)` which lowers to LoadName + Send :call.
    .{ "Global:setHandler!", globalSetHandler },
    .{ "Global:intern", globalIntern },
    .{ "Global:cons", globalCons },
    .{ "Global:list", globalList },
    .{ "Global:raise:", globalRaise },
    .{ "Global:slot", globalSlot },
    .{ "Global:slotSet!", globalSlotSet },
    .{ "Global:metaSet!", globalMetaSet },
    .{ "Global:globalEnv", globalGlobalEnv },
    .{ "Global:getOrCreateProto", globalGetOrCreateProto },
    .{ "Global:append", globalAppend },
    .{ "Global:macroexpand", globalMacroexpand },

    // phase1/B — vat-mode intrinsics.
    .{ "Global:__vat-mode__", globalVatMode },
    .{ "Global:__alloc-mutable__", globalAllocMutable },

    // String primitives — parser uses these heavily.
    .{ "String:length", stringLength },
    .{ "String:at:", stringAt },
    .{ "String:=", stringEq },
    .{ "String:slice:length:", stringSlice },
    .{ "String:+", stringPlus },

    // Char primitives.
    .{ "Char:codepoint", charCodepoint },
    .{ "Char:<", charLt },
    .{ "Char:toString", charToString },

    // Integer:asChar — coerce Int → Char.
    .{ "Integer:asChar", intAsChar },

    // Chunk class- + instance-side methods — moof Compiler primitives.
    .{ "Chunk:new:source:", chunkNewSource },
    .{ "Chunk:emit:", chunkEmit },
    .{ "Chunk:addConst:", chunkAddConst },
    .{ "Chunk:addIc", chunkAddIc },
    .{ "Chunk:jumpTarget", chunkJumpTarget },
    .{ "Chunk:patchJump:to:", chunkPatchJump },
    .{ "Chunk:asClosure", chunkAsClosure },
});

// ─────────────────────────────────────────────────────────────────
// $layout cap tests. validate the cons-list-walker on the native
// against the World.registerLayout API. eval-level smoke through
// defproto is blocked on the unrelated pre-existing #31 bootstrap
// gap (compiler `emit:` UnboundName during main), so we test the
// native directly here.
// ─────────────────────────────────────────────────────────────────

const testing = std.testing;

test "layoutRegisterSlots: walks cons-list and registers layout" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    // alloc a fresh proto.
    const counter_proto = try world.heap.alloc(Form.withProto(.{ .form = world.protos.object }));
    const count_sym = try world.syms.intern("count");
    // build a moof list '(count) — a single FlatCons cell.
    const slots_list = try world.makeList(&.{.{ .sym = count_sym }});
    const result = try layoutRegisterSlots(&world, .nil, &.{ .{ .form = counter_proto }, slots_list });
    try testing.expect(result == .nil);
    // layout should now be registered.
    const lay = world.layoutForProto(counter_proto) orelse return error.NoLayout;
    try testing.expectEqual(@as(u8, 1), lay.inline_size);
    try testing.expectEqual(count_sym, lay.slot_names[0]);
}

test "layoutRegisterSlots: empty slot list registers zero-size layout" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const proto = try world.heap.alloc(Form.withProto(.{ .form = world.protos.object }));
    _ = try layoutRegisterSlots(&world, .nil, &.{ .{ .form = proto }, .nil });
    const lay = world.layoutForProto(proto) orelse return error.NoLayout;
    try testing.expectEqual(@as(u8, 0), lay.inline_size);
}

test "layoutRegisterSlots: raises on non-Symbol slot name" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const proto = try world.heap.alloc(Form.withProto(.{ .form = world.protos.object }));
    // build a list with an int instead of a sym — should type-error.
    const bad_list = try world.makeList(&.{.{ .int = 42 }});
    const got = layoutRegisterSlots(&world, .nil, &.{ .{ .form = proto }, bad_list });
    try testing.expectError(error.DispatchError, got);
}

test "layoutRegisterSlots: end-to-end with Object:new picks up inline layout" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    try install(&world);
    // alloc a fresh user proto, register a Layout for it, then send :new.
    const counter_proto = try world.heap.alloc(Form.withProto(.{ .form = world.protos.object }));
    const count_sym = try world.syms.intern("count");
    const slots_list = try world.makeList(&.{.{ .sym = count_sym }});
    _ = try layoutRegisterSlots(&world, .nil, &.{ .{ .form = counter_proto }, slots_list });
    // [Counter new] — Object:new (objNew above) calls world.allocInstance,
    // which checks proto_layouts and returns a Form with inline_slots.
    const new_v = try objNew(&world, .{ .form = counter_proto }, &.{});
    const instance_id = new_v.asFormId() orelse return error.NotAForm;
    const f = world.heap.get(instance_id);
    try testing.expect(f.layout != null);
    try testing.expectEqual(@as(u8, 1), f.layout.?.inline_size);
    // count starts nil; setting it via slotSet! should land inline.
    try world.formSlotSet(instance_id, count_sym, .{ .int = 7 });
    const f2 = world.heap.get(instance_id);
    try testing.expect(f2.slots.count() == 0); // didn't spill to SlotMap
    try testing.expect(f2.inline_slots[0].equals(.{ .int = 7 }));
}
