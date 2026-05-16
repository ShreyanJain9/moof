//! moof-zig — VM dispatch loop. per V4 spec §3 + §6.
//!
//! one switch-based interpreter; one shared operand stack; one
//! shared frame stack (so tail-sends and recursive send-from-native
//! stay in bounded memory). this is the zig port of the rust seed's
//! `crates/substrate/src/vm.rs`, updated for the V4 opcode set —
//! 24 opcodes across 6 categories (V4 spec §2).
//!
//! per `laws/substrate-laws.md` L3, *every* method call goes through
//! Send. there is no privileged ABI for built-in operations;
//! arithmetic, reflection, even global lookups go through send.
//!
//! per L10, ICs check the cached proto's generation; mismatch
//! (because `set-handler!` rewrote the table) triggers re-resolution
//! and re-populates the slot. per L11, FormIds are stable; the
//! interpreter never compacts or renumbers heap addresses.
//!
//! tail-position sends (`TailSend`, `TailSendSelf`, `TailSendHere`)
//! replace the current frame; non-tail sends push a new frame. per
//! V4 spec §6.2, tail-send variants currently do not consult ICs
//! (flagged as future work).
//!
//! `Suspend` / `Resume` are reserved-and-defined in V4 (spec §3.6)
//! but the promise / scheduler machinery is phase D+. they raise
//! `SuspendUnimplemented` / `ResumeUnimplemented` until then.
//!
//! ## dispatch architecture (post-2026-05-11 phase 1 §4 refactor)
//!
//! the dispatch loop is single-level: `runTop` pushes a frame, then
//! calls `step` in a `while (frames.len > start_depth)` loop. each
//! op handler returns to that loop after one op. **non-tail sends
//! push a new frame onto `world.vm.frames` and return**; the outer
//! loop picks up the new top frame on the next iteration. when the
//! method's `Return` fires, it pops the frame and pushes the result
//! onto the caller's operand stack.
//!
//! this means moof→moof send depth is bounded by the heap (frames
//! ArrayList) — **not** the host stack. previously every non-tail
//! Send walked `runMethod → step → sendViaIC → invokeMethod →
//! runMethod`, ~5 host-stack frames per moof send, blowing the
//! default 8 MB stack at ~26 levels deep. the rust seed has the
//! same shape and a 128 MB worker-thread workaround
//! (`crates/substrate/src/main.rs:16-30`); the zig substrate fixes
//! it structurally.
//!
//! native re-entry (e.g. `:perform:withArgs:` calling `World.send`)
//! is "option α" per spec §4.5: `World.send` runs an inner sub-loop
//! `runUntilFrameReturns` that drives the outer loop until that
//! specific frame returns, then unwinds. one level of host-stack
//! recursion per nested native→moof call; bounded by native count
//! (~50 in the stdlib), not moof depth.

const std = @import("std");
const opcodes = @import("opcodes.zig");
const bytecode = @import("bytecode.zig");
const value = @import("value.zig");
const Value = value.Value;
const form = @import("form.zig");
const FormId = form.FormId;
const world_mod = @import("world.zig");
const World = world_mod.World;
const Frame = world_mod.Frame;
const ICache = world_mod.ICache;
const SymId = world_mod.SymId;

/// VM-level errors. send-dispatch + native-method errors flow
/// through `anyerror` so individual native handlers can raise their
/// own error sets; the VM only adds the structural ones below.
///
/// `SuspendUnimplemented` / `ResumeUnimplemented` are placeholders
/// for phase D scheduling (V4 spec §3.6).
pub const VmError = error{
    UnknownChunk,
    NoChunkConsts,
    UnboundName,
    StackUnderflow,
    SendArgcOverflow,
    SuperFromNonMethodFrame,
    SuperHandlerMissing,
    SendDynamicRequiresSymbol,
    HandlerNotAMethod,
    MethodBodyNotAChunk,
    BadParam,
    Arity,
    JumpNegative,
    PcOutOfBounds,
    SuspendUnimplemented,
    ResumeUnimplemented,
    DispatchError,
    UnhandledDnu,
} || std.mem.Allocator.Error;

/// result of a dispatch "prepare" step. either the call ran to
/// completion synchronously (native: result is returned in-band)
/// or a new frame was pushed onto `world.vm.frames` (bytecode:
/// outer loop picks up the new frame next iteration; its eventual
/// `Return` pushes the result onto the caller's stack).
///
/// per spec §4.4: native_done means "shrink + push result yourself";
/// bytecode_pushed means "do nothing — Return will push for you".
pub const DispatchAction = union(enum) {
    /// native finished; caller must push `result` onto the operand
    /// stack. the stack has already been shrunk to the dispatch's
    /// `shrink_to` argument before this is returned.
    native_done: Value,
    /// a bytecode frame has been pushed onto `world.vm.frames`. the
    /// stack has been shrunk to the dispatch's `shrink_to` argument
    /// (which is the new frame's `stack_base`). caller does nothing
    /// more — outer loop will drive the new frame.
    bytecode_pushed,
};

// =====================================================================
// step + top-level run
// =====================================================================

/// execute one bytecode op of the topmost frame.
///
/// per V4 spec §6.7, the read+advance is atomic with respect to the
/// executing thread (single-threaded substrate); operand layout is
/// fixed-size, no parsing ambiguity.
pub fn step(world: *World) !void {
    const frame_idx = world.vm.frames.items.len - 1;
    const chunk = world.vm.frames.items[frame_idx].chunk;
    const pc = world.vm.frames.items[frame_idx].pc;
    const bytes = world.chunk_bytecode.get(chunk) orelse return error.UnknownChunk;
    const decoded = try bytecode.decodeOp(bytes, pc);
    world.vm.frames.items[frame_idx].pc = pc + decoded.advance;
    try dispatchOp(world, decoded.op);
}

/// run a chunk to completion, returning its top-of-stack on Return.
///
/// pushes a fresh frame, runs the loop until *that* frame returns,
/// pops the result. equivalent to invoking a zero-arg method whose
/// body is `chunk`. `defining_proto` is `FormId.NONE` because no
/// method dispatch led here.
pub fn runTop(world: *World, chunk: FormId) !Value {
    const starting_depth = world.vm.frames.items.len;
    try world.vm.frames.append(world.allocator, Frame{
        .chunk = chunk,
        .pc = 0,
        .env = world.here_form,
        .self_ = .nil,
        .stack_base = @intCast(world.vm.stack.items.len),
        .defining_proto = FormId.NONE,
    });
    while (world.vm.frames.items.len > starting_depth) {
        try step(world);
    }
    // the popped frame's last `Return` left its result on the stack.
    if (world.vm.stack.items.len == 0) return .nil;
    return world.vm.stack.pop().?;
}

/// drive the dispatch loop until `world.vm.frames.items.len` falls
/// back to `target_depth`. used by `World.send` and other native
/// re-entry paths ("option α" per spec §4.5): when a native needs to
/// synchronously call into moof code, it pushes a frame and calls
/// this to run that frame to completion without further unwinding
/// the host stack.
///
/// caller is responsible for popping the result off `world.vm.stack`
/// after this returns.
pub fn runUntilFrameReturns(world: *World, target_depth: usize) !void {
    while (world.vm.frames.items.len > target_depth) {
        try step(world);
    }
}

/// legacy name kept for backward compatibility with intrinsics that
/// still call `world.vm.runMethod(...)`. new code should use
/// `prepareDispatch` + `runUntilFrameReturns` (or `World.send`).
///
/// pushes a fresh frame, drives the outer loop until that frame
/// returns, pops the result. one level of host-stack recursion if
/// called from a native; if called from `runTop` directly, no extra
/// recursion (it's just the outer loop).
pub fn runMethod(
    world: *World,
    chunk: FormId,
    env: FormId,
    self_v: Value,
    defining_proto: FormId,
) anyerror!Value {
    const starting_depth = world.vm.frames.items.len;
    try world.vm.frames.append(world.allocator, Frame{
        .chunk = chunk,
        .pc = 0,
        .env = env,
        .self_ = self_v,
        .stack_base = @intCast(world.vm.stack.items.len),
        .defining_proto = defining_proto,
    });
    try runUntilFrameReturns(world, starting_depth);
    if (world.vm.stack.items.len == 0) return .nil;
    return world.vm.stack.pop().?;
}

// =====================================================================
// dispatch
// =====================================================================

/// the dispatch table. one branch per opcode tag. semantics per V4
/// spec §3.
pub fn dispatchOp(world: *World, op: opcodes.Op) !void {
    switch (op) {
        // ===== value-load ops (V4 spec §3.1) =====

        .push_nil => try world.vm.stack.append(world.allocator, .nil),
        .push_true => try world.vm.stack.append(world.allocator, .{ .bool_ = true }),
        .push_false => try world.vm.stack.append(world.allocator, .{ .bool_ = false }),

        .load_const => |args| {
            const frame_idx = world.vm.frames.items.len - 1;
            const chunk = world.vm.frames.items[frame_idx].chunk;
            const consts = world.chunk_consts.get(chunk) orelse return error.NoChunkConsts;
            if (args.idx >= consts.len) return error.PcOutOfBounds;
            try world.vm.stack.append(world.allocator, consts[args.idx]);
        },

        .load_self => {
            const frame_idx = world.vm.frames.items.len - 1;
            try world.vm.stack.append(world.allocator, world.vm.frames.items[frame_idx].self_);
        },

        // [NEW in V4] — bypasses any user-level $here rebinding;
        // pushes the substrate-canonical here_form directly. see
        // V4 spec §6.5.
        .load_here => try world.vm.stack.append(world.allocator, .{ .form = world.here_form }),

        .load_name => |args| {
            const frame_idx = world.vm.frames.items.len - 1;
            const env = world.vm.frames.items[frame_idx].env;
            const v = world.envLookup(env, args.name) orelse return error.UnboundName;
            try world.vm.stack.append(world.allocator, v);
        },

        // ===== stack ops (V4 spec §3.2) =====

        .pop => {
            if (world.vm.stack.items.len == 0) return error.StackUnderflow;
            _ = world.vm.stack.pop();
        },

        .dup => {
            if (world.vm.stack.items.len == 0) return error.StackUnderflow;
            const top = world.vm.stack.items[world.vm.stack.items.len - 1];
            try world.vm.stack.append(world.allocator, top);
        },

        // ===== send ops (V4 spec §3.3) =====
        //
        // single-loop dispatch (post-2026-05-11 §4 refactor): each
        // Send op handler either invokes a native and pushes its
        // result, or pushes a new bytecode frame and returns. the
        // outer loop in `runTop` / `runUntilFrameReturns` picks up
        // the new top frame. no host-stack recursion.

        // Send {sel, argc, ic}: pop receiver + argc args; dispatch
        // via IC; either invoke native + push result, or push a new
        // frame whose Return pushes the result. (stack effect
        // -(1+argc)/+1, eventually.)
        .send => |args| {
            const argc: usize = args.argc;
            if (world.vm.stack.items.len < argc + 1) return error.SendArgcOverflow;
            const receiver_idx = world.vm.stack.items.len - argc - 1;
            const receiver = world.vm.stack.items[receiver_idx];
            const call_args = world.vm.stack.items[receiver_idx + 1 ..];
            const action = try prepareSendDispatch(world, receiver, args.selector, call_args, args.ic_idx, receiver_idx);
            switch (action) {
                .native_done => |result| try world.vm.stack.append(world.allocator, result),
                .bytecode_pushed => {}, // new frame's Return will push
            }
        },

        // TailSend {sel, argc}: pop receiver + argc args; replace
        // current frame with the dispatched method's frame. per
        // V4 spec §6.2, tail-send variants currently lack ICs (full
        // lookup every time; future work). tail-position frame
        // replacement is already non-recursive (no new host frame).
        .tail_send => |args| {
            const argc: usize = args.argc;
            if (world.vm.stack.items.len < argc + 1) return error.SendArgcOverflow;
            const split = world.vm.stack.items.len - argc;
            const receiver = world.vm.stack.items[split - 1];
            try replaceFrameWithTailCall(world, receiver, args.selector, split, split - 1);
        },

        // SuperSend {sel, argc, ic}: receiver = current frame's
        // self_; lookup walks *above* frame.defining_proto. per
        // V4 spec §6.3, SuperSend uses self as receiver implicitly
        // — there's no SuperSendSelf.
        .super_send => |args| {
            try doSuperSend(world, args.selector, args.argc, args.ic_idx);
        },

        // SendDynamic {argc, ic}: selector is on the stack top
        // (pop'd first); then receiver; then argc args. [NEW in V4]
        // — compiles down `:perform:withArgs:` directly.
        .send_dynamic => |args| {
            const argc: usize = args.argc;
            if (world.vm.stack.items.len < argc + 2) return error.SendArgcOverflow;
            const sel_v = world.vm.stack.pop().?;
            const sel = sel_v.asSym() orelse return error.SendDynamicRequiresSymbol;
            const receiver_idx = world.vm.stack.items.len - argc - 1;
            const receiver = world.vm.stack.items[receiver_idx];
            const call_args = world.vm.stack.items[receiver_idx + 1 ..];
            const action = try prepareSendDispatch(world, receiver, sel, call_args, args.ic_idx, receiver_idx);
            switch (action) {
                .native_done => |result| try world.vm.stack.append(world.allocator, result),
                .bytecode_pushed => {},
            }
        },

        // SendSelf {sel, argc, ic}: receiver = current frame's
        // self_ (no receiver pop). [NEW in V4] — equivalent to
        // LoadSelf;Send fused. per V4 spec §6.6, top-level
        // dispatches with self_ = Nil; cleanly defined.
        .send_self => |args| {
            const frame_idx = world.vm.frames.items.len - 1;
            const self_v = world.vm.frames.items[frame_idx].self_;
            const argc: usize = args.argc;
            if (world.vm.stack.items.len < argc) return error.SendArgcOverflow;
            const args_start = world.vm.stack.items.len - argc;
            const call_args = world.vm.stack.items[args_start..];
            const action = try prepareSendDispatch(world, self_v, args.selector, call_args, args.ic_idx, args_start);
            switch (action) {
                .native_done => |result| try world.vm.stack.append(world.allocator, result),
                .bytecode_pushed => {},
            }
        },

        // SendHere {sel, argc, ic}: receiver = Value::Form(world.here_form).
        // [NEW in V4] — equivalent to LoadHere;Send fused. uses
        // substrate's canonical here_form; bypasses user-level
        // $here rebinding (V4 spec §6.5).
        .send_here => |args| {
            const argc: usize = args.argc;
            if (world.vm.stack.items.len < argc) return error.SendArgcOverflow;
            const args_start = world.vm.stack.items.len - argc;
            const call_args = world.vm.stack.items[args_start..];
            const receiver: Value = .{ .form = world.here_form };
            const action = try prepareSendDispatch(world, receiver, args.selector, call_args, args.ic_idx, args_start);
            switch (action) {
                .native_done => |result| try world.vm.stack.append(world.allocator, result),
                .bytecode_pushed => {},
            }
        },

        // TailSendSelf {sel, argc}: tail-position variant of SendSelf.
        // receiver = current frame's self_; replace frame. [NEW in V4]
        .tail_send_self => |args| {
            const frame_idx = world.vm.frames.items.len - 1;
            const self_v = world.vm.frames.items[frame_idx].self_;
            const argc: usize = args.argc;
            if (world.vm.stack.items.len < argc) return error.SendArgcOverflow;
            const split = world.vm.stack.items.len - argc;
            try replaceFrameWithTailCall(world, self_v, args.selector, split, split);
        },

        // TailSendHere {sel, argc}: tail-position variant of SendHere.
        // [NEW in V4]
        .tail_send_here => |args| {
            const argc: usize = args.argc;
            if (world.vm.stack.items.len < argc) return error.SendArgcOverflow;
            const split = world.vm.stack.items.len - argc;
            const receiver: Value = .{ .form = world.here_form };
            try replaceFrameWithTailCall(world, receiver, args.selector, split, split);
        },

        // ===== control flow ops (V4 spec §3.4) =====

        .jump => |args| {
            const frame_idx = world.vm.frames.items.len - 1;
            const cur_pc = world.vm.frames.items[frame_idx].pc;
            const new_pc = @as(isize, @intCast(cur_pc)) + @as(isize, args.offset);
            if (new_pc < 0) return error.JumpNegative;
            world.vm.frames.items[frame_idx].pc = @intCast(new_pc);
        },

        .jump_if_false => |args| {
            if (world.vm.stack.items.len == 0) return error.StackUnderflow;
            const v = world.vm.stack.pop().?;
            if (!v.isTruthy()) {
                const frame_idx = world.vm.frames.items.len - 1;
                const cur_pc = world.vm.frames.items[frame_idx].pc;
                const new_pc = @as(isize, @intCast(cur_pc)) + @as(isize, args.offset);
                if (new_pc < 0) return error.JumpNegative;
                world.vm.frames.items[frame_idx].pc = @intCast(new_pc);
            }
        },

        // [NEW in V4] — dual of JumpIfFalse. lets the if-peephole
        // emit direct-branch shape without inverting via `:!`.
        .jump_if_true => |args| {
            if (world.vm.stack.items.len == 0) return error.StackUnderflow;
            const v = world.vm.stack.pop().?;
            if (v.isTruthy()) {
                const frame_idx = world.vm.frames.items.len - 1;
                const cur_pc = world.vm.frames.items[frame_idx].pc;
                const new_pc = @as(isize, @intCast(cur_pc)) + @as(isize, args.offset);
                if (new_pc < 0) return error.JumpNegative;
                world.vm.frames.items[frame_idx].pc = @intCast(new_pc);
            }
        },

        // Return: pop top-of-stack; pop current frame; truncate to
        // caller's stack_base; push result onto caller's stack.
        // if no caller (this was the outermost frame), the outer
        // loop's `frames.len > start_depth` check breaks and the
        // result sits on top of the stack for runTop/runUntilFrameReturns
        // to harvest. per spec §4.2 — Return semantics unchanged
        // from the recursive design; it's the *outer* loop that
        // changed.
        .return_op => {
            if (world.vm.stack.items.len == 0) return error.StackUnderflow;
            const result = world.vm.stack.pop().?;
            const popped = world.vm.frames.pop().?;
            world.vm.stack.shrinkRetainingCapacity(popped.stack_base);
            try world.vm.stack.append(world.allocator, result);
        },

        // ===== closure ops (V4 spec §3.5) =====

        // PushClosure {chunk}: alloc closure-Form with proto =
        // protos.closure; capture current env + current self_
        // (so let-induced closures see the enclosing method's
        // receiver via [m call:…]). when installed as a handler
        // and dispatched, World::invoke overrides self_ with the
        // runtime receiver.
        .push_closure => |args| {
            const frame_idx = world.vm.frames.items.len - 1;
            const env = world.vm.frames.items[frame_idx].env;
            const captured_self = world.vm.frames.items[frame_idx].self_;
            const closure_id = try world.allocClosure(args.chunk, env, captured_self);
            try world.vm.stack.append(world.allocator, .{ .form = closure_id });
        },

        // ===== scheduling ops (V4 spec §3.6) — phase D+ =====

        .suspend_op => return error.SuspendUnimplemented,
        .resume_op => return error.ResumeUnimplemented,
    }
}

// =====================================================================
// send via inline cache (V4 spec §6.1 + L10)
// =====================================================================

/// dispatch with an inline-cache fast path. used by `Send`,
/// `SendSelf`, `SendHere`, `SendDynamic`. returns a `DispatchAction`
/// indicating whether the call ran inline (native) or pushed a new
/// frame onto `world.vm.frames` (bytecode).
///
/// `call_args` is a slice into `world.vm.stack.items`; it must be
/// fully read before the stack is shrunk. `shrink_to` is the stack
/// position to truncate to after extracting args (the receiver's
/// index for Send / SendDynamic; `args_start` for SendSelf / SendHere
/// where the receiver was implicit).
///
/// per V4 spec §6.1, the IC caches `(proto, method, generation,
/// defining, singleton)` and the receiver source doesn't affect
/// the IC layout. different sites use different `ic_idx` values;
/// no collision.
///
/// per `laws/substrate-laws.md` L10, generation-mismatch (because
/// `set-handler!` rewrote the table since we cached) triggers
/// re-resolution.
pub fn prepareSendDispatch(
    world: *World,
    receiver: Value,
    selector: SymId,
    call_args: []const Value,
    ic_idx: u16,
    shrink_to: usize,
) anyerror!DispatchAction {
    const frame_idx = world.vm.frames.items.len - 1;
    const chunk = world.vm.frames.items[frame_idx].chunk;

    // resolve the receiver's proto first. tagged-immediate values
    // (Nil, Bool, Int, …) have substrate-installed protos; Form
    // values delegate via the heap's proto chain.
    const receiver_proto_v = world.protoOf(receiver);
    const receiver_proto: FormId = switch (receiver_proto_v) {
        .form => |id| id,
        else => {
            // tagged-immediate proto chain bottoms unexpectedly —
            // fall back to the slow path which will dnu.
            return prepareSlowSend(world, receiver, selector, call_args, shrink_to);
        },
    };

    // attempt IC fast-path. ics array per-chunk; bounds-check.
    if (world.chunk_ics.get(chunk)) |ics| {
        if (ic_idx < ics.len) {
            const cached: ICache = ics[ic_idx];
            // when the cached handler came from a singleton (per-
            // instance state, e.g. #true's :toString), we must verify
            // the receiver's effective singleton matches too —
            // otherwise we'd hand Bool(true)'s :toString to Bool(false).
            // for proto-chain handlers (cached_singleton == NONE),
            // proto+generation alone is sufficient.
            const singleton_ok = blk: {
                if (cached.cached_singleton.isNone()) break :blk true;
                const eff = world.effectiveFormId(receiver) orelse break :blk false;
                break :blk eff.eql(cached.cached_singleton);
            };
            if (!cached.cached_proto.isNone() and
                cached.cached_proto.eql(receiver_proto) and
                cached.cached_generation == world.protoGeneration(receiver_proto) and
                singleton_ok)
            {
                // cache hit
                world.vm.last_send_sel = selector;
                return prepareInvoke(
                    world,
                    cached.cached_method,
                    receiver,
                    call_args,
                    cached.cached_defining,
                    shrink_to,
                );
            }
        }
    }

    // cache miss or stale — slow lookup + populate.
    const lookup = world.lookupHandler(receiver, selector);
    if (lookup) |hit| {
        const handler = hit.handler;
        const defining = hit.defining;
        const method = handler.asFormId() orelse return error.HandlerNotAMethod;
        // read generation first to avoid double-borrow patterns.
        const gen = world.protoGeneration(receiver_proto);
        // if the handler was found on the receiver's own singleton,
        // record so the IC distinguishes (e.g. Bool(true) vs Bool(false)).
        const cached_singleton: FormId = blk: {
            const eff = world.effectiveFormId(receiver) orelse break :blk FormId.NONE;
            if (eff.eql(defining)) break :blk eff;
            break :blk FormId.NONE;
        };
        // populate the IC slot
        if (world.chunk_ics.getPtr(chunk)) |ics_ptr| {
            const ics = ics_ptr.*;
            if (ic_idx < ics.len) {
                ics[ic_idx] = .{
                    .cached_proto = receiver_proto,
                    .cached_method = method,
                    .cached_defining = defining,
                    .cached_generation = gen,
                    .cached_singleton = cached_singleton,
                };
            }
        }
        world.vm.last_send_sel = selector;
        return prepareInvoke(world, method, receiver, call_args, defining, shrink_to);
    }
    // no handler — dispatch :does-not-understand:with:
    return prepareDispatchDnu(world, receiver, selector, call_args, shrink_to);
}

/// slow-path send (no IC). used when the receiver's proto-of
/// resolves to a tagged-immediate (which can't appear in the IC's
/// cached_proto FormId slot), and as the fall-through for dnu
/// dispatch.
pub fn prepareSlowSend(
    world: *World,
    receiver: Value,
    selector: SymId,
    call_args: []const Value,
    shrink_to: usize,
) anyerror!DispatchAction {
    const lookup = world.lookupHandler(receiver, selector);
    if (lookup) |hit| {
        const method = hit.handler.asFormId() orelse return error.HandlerNotAMethod;
        world.vm.last_send_sel = selector;
        return prepareInvoke(world, method, receiver, call_args, hit.defining, shrink_to);
    }
    return prepareDispatchDnu(world, receiver, selector, call_args, shrink_to);
}

/// fall-through when no handler is found anywhere on the proto chain.
/// constructs `(does-not-understand:with: <selector> <args>)` and
/// re-dispatches. if `:does-not-understand:with:` itself is missing,
/// raises UnhandledDnu.
fn prepareDispatchDnu(
    world: *World,
    receiver: Value,
    selector: SymId,
    call_args: []const Value,
    shrink_to: usize,
) anyerror!DispatchAction {
    const dnu = world.dnu_sym;
    if (selector == dnu) {
        // we got here from a previous dnu fall-through — there's
        // no handler to escalate to.
        return error.UnhandledDnu;
    }
    // makeList allocates Forms but doesn't touch the operand stack;
    // call_args may still be a stack slice — copy by value into the
    // new list. then we re-enter slow-send with the dnu selector.
    const args_list = try world.makeList(call_args);
    const dnu_args = [_]Value{ .{ .sym = selector }, args_list };
    return prepareSlowSend(world, receiver, dnu, &dnu_args, shrink_to);
}

// =====================================================================
// prepare invocation — push frame or call native (no host recursion)
// =====================================================================

/// dispatch a known method to `(receiver, call_args)`. **does not
/// run** the method — for bytecode methods it allocates the env,
/// binds the params, shrinks the stack to `shrink_to`, and pushes a
/// new `Frame` onto `world.vm.frames`. for native methods it copies
/// args out, shrinks the stack, and invokes the native inline,
/// returning the result in `native_done`.
///
/// the caller (the dispatchOp Send-family handler, or the outer
/// `World.send`) reads the return value:
///   - `native_done(v)`: push `v` onto the stack.
///   - `bytecode_pushed`: do nothing — the new frame is the new top
///     and will be driven by the outer loop. its `Return` will push
///     the result onto the caller's stack (which has already been
///     truncated to `shrink_to` = the new frame's `stack_base`).
pub fn prepareInvoke(
    world: *World,
    method: FormId,
    self_v: Value,
    call_args: []const Value,
    defining_proto: FormId,
    shrink_to: usize,
) anyerror!DispatchAction {
    // native? copy args, shrink stack, run inline; no frame push.
    if (world.nativeFn(method)) |native| {
        const argc = call_args.len;
        if (argc == 0) {
            world.vm.stack.shrinkRetainingCapacity(shrink_to);
            const result = try native(world, self_v, &.{});
            return .{ .native_done = result };
        }
        const args_buf = try world.allocator.alloc(Value, argc);
        defer world.allocator.free(args_buf);
        @memcpy(args_buf, call_args);
        world.vm.stack.shrinkRetainingCapacity(shrink_to);
        const result = try native(world, self_v, args_buf);
        return .{ .native_done = result };
    }

    // bytecode: build call env, bind params, push a frame.
    const body_v = world.formSlot(method, world.body_sym);
    const chunk_id = body_v.asFormId() orelse return error.MethodBodyNotAChunk;
    const captured_env_v = world.formSlot(method, world.env_sym);
    const captured_env = captured_env_v.asFormId() orelse world.here_form;
    const params_v = world.formSlot(method, world.params_sym);

    const params = world.listToSlice(params_v) catch |err| {
        std.debug.print("prepareInvoke: listToSlice failed: {s}\n", .{@errorName(err)});
        return err;
    };
    defer world.freeSlice(params);

    if (params.len != call_args.len) {
        std.debug.print("prepareInvoke: Arity mismatch: method has {d} params, called with {d} args\n", .{ params.len, call_args.len });
        // debug: list slots of method
        const mf = world.heap.get(method);
        std.debug.print("method Form {d} slots:\n", .{method.payload});
        var it = mf.slots.iterator();
        while (it.next()) |entry| {
            std.debug.print("  {s} -> \n", .{world.syms.resolve(entry.key_ptr.*)});
        }
        return error.Arity;
    }

    // bind params from the (still-live) call_args slice into the
    // new env BEFORE shrinking the stack — call_args may point
    // into the operand stack.
    const call_env = try world.allocEnv(captured_env);
    for (params, call_args) |param_v, arg_v| {
        const param_sym = param_v.asSym() orelse return error.BadParam;
        try world.envBind(call_env, param_sym, arg_v);
    }

    // params bound — now safe to shrink and push frame.
    world.vm.stack.shrinkRetainingCapacity(shrink_to);
    try world.vm.frames.append(world.allocator, Frame{
        .chunk = chunk_id,
        .pc = 0,
        .env = call_env,
        .self_ = self_v,
        .stack_base = @intCast(shrink_to),
        .defining_proto = defining_proto,
    });
    return .bytecode_pushed;
}

// =====================================================================
// tail-send (replace-frame; non-recursive by construction)
// =====================================================================

/// replace the current frame with a tail-call to `(receiver
/// selector args…)`. used by `TailSend`, `TailSendSelf`,
/// `TailSendHere`.
///
/// args occupy stack indices `[args_start, args_start + argc)`.
/// `discard_from` is the stack index from which we should truncate
/// after copying args (for TailSend it's the receiver index since
/// the receiver was on the stack; for TailSendSelf / TailSendHere
/// it's `args_start` since the receiver was implicit).
///
/// per V4 spec §6.2, tail-sends currently lack ICs — full lookup
/// every time. flagged as future work.
///
/// tail-sends are already non-recursive (the frame is replaced in
/// place, no new host-stack frame). the §4 refactor leaves this
/// path unchanged; we just need to make sure the dnu fall-through
/// (which goes through prepareDispatchDnu) integrates cleanly.
fn replaceFrameWithTailCall(
    world: *World,
    receiver: Value,
    selector: SymId,
    args_start: usize,
    discard_from: usize,
) anyerror!void {
    // copy args out before mutating the stack.
    const argc = world.vm.stack.items.len - args_start;
    const args_buf = try world.allocator.alloc(Value, argc);
    defer world.allocator.free(args_buf);
    @memcpy(args_buf, world.vm.stack.items[args_start..]);

    // dispatch.
    const lookup = world.lookupHandler(receiver, selector);
    const hit = lookup orelse {
        // no handler — fall through to dnu. no TCO opportunity
        // (dnu dispatch is itself a non-tail call); route through
        // the shared prepareDispatchDnu machinery, which will
        // either push a frame (bytecode dnu handler) or run a
        // native dnu handler inline and return the result.
        //
        // truncate to discard_from first so dispatch's `shrink_to`
        // matches.
        world.vm.stack.shrinkRetainingCapacity(discard_from);
        const action = try prepareDispatchDnu(world, receiver, selector, args_buf, discard_from);
        switch (action) {
            .native_done => |result| try world.vm.stack.append(world.allocator, result),
            .bytecode_pushed => {},
        }
        return;
    };
    const method = hit.handler.asFormId() orelse return error.HandlerNotAMethod;
    const defining = hit.defining;

    // native? same as Send's native path; pop args and push result;
    // no frame replacement.
    if (world.nativeFn(method)) |native| {
        world.vm.last_send_sel = selector;
        world.vm.stack.shrinkRetainingCapacity(discard_from);
        const result = try native(world, receiver, args_buf);
        try world.vm.stack.append(world.allocator, result);
        return;
    }

    // bytecode: replace the current frame.
    const body_v = world.formSlot(method, world.body_sym);
    const chunk_id = body_v.asFormId() orelse return error.MethodBodyNotAChunk;
    const captured_env_v = world.formSlot(method, world.env_sym);
    const captured_env = captured_env_v.asFormId() orelse world.here_form;
    const params_v = world.formSlot(method, world.params_sym);
    const params = try world.listToSlice(params_v);
    defer world.freeSlice(params);
    if (params.len != args_buf.len) return error.Arity;

    const call_env = try world.allocEnv(captured_env);
    for (params, args_buf) |param_v, arg_v| {
        const param_sym = param_v.asSym() orelse return error.BadParam;
        try world.envBind(call_env, param_sym, arg_v);
    }

    // truncate stack to the current frame's stack_base — discard
    // any leftover scratch + the args + (for TailSend) the
    // receiver. the new frame starts clean.
    const frame_idx = world.vm.frames.items.len - 1;
    const base = world.vm.frames.items[frame_idx].stack_base;
    world.vm.stack.shrinkRetainingCapacity(base);
    world.vm.frames.items[frame_idx] = Frame{
        .chunk = chunk_id,
        .pc = 0,
        .env = call_env,
        .self_ = receiver,
        .stack_base = base,
        .defining_proto = defining,
    };
}

// =====================================================================
// super-send (lookup above defining_proto)
// =====================================================================

/// `[super selector args…]` — receiver is the current frame's self;
/// lookup walks *above* frame.defining_proto. per V4 spec §6.3,
/// SuperSend uses self as receiver implicitly (no SuperSendSelf
/// variant needed).
///
/// the `ic_idx` operand exists in the byte encoding (V4 spec §3.3,
/// 0x22 SuperSend) but the current rust seed doesn't consult it for
/// super-sends; we mirror that — full lookup. flagged as future
/// work alongside tail-send ICs.
///
/// post-§4 refactor: routes through `prepareInvoke` so a bytecode
/// super-send pushes a new frame and returns to the outer loop,
/// matching the bounded-host-stack invariant.
fn doSuperSend(
    world: *World,
    selector: SymId,
    argc: u8,
    ic_idx: u16,
) anyerror!void {
    _ = ic_idx; // reserved per V4 spec §3.3; not yet consulted

    const argc_u: usize = argc;
    if (world.vm.stack.items.len < argc_u) return error.SendArgcOverflow;
    const args_start = world.vm.stack.items.len - argc_u;
    const call_args = world.vm.stack.items[args_start..];

    const frame_idx = world.vm.frames.items.len - 1;
    const self_v = world.vm.frames.items[frame_idx].self_;
    const defining = world.vm.frames.items[frame_idx].defining_proto;
    if (defining.isNone()) return error.SuperFromNonMethodFrame;

    const lookup = world.lookupHandlerSuper(defining, selector);
    const hit = lookup orelse return error.SuperHandlerMissing;
    const method = hit.handler.asFormId() orelse return error.HandlerNotAMethod;
    world.vm.last_send_sel = selector;
    const action = try prepareInvoke(world, method, self_v, call_args, hit.defining, args_start);
    switch (action) {
        .native_done => |result| try world.vm.stack.append(world.allocator, result),
        .bytecode_pushed => {},
    }
}
