//! the substrate's per-vat root. owns the heap, sym table, protos,
//! chunk side-tables, native-fn registry, and the bytecode interpreter
//! state. one World per vat (V4 polyglot-substrate plan, Track A).
//!
//! the rust seed equivalent is `crates/substrate/src/world.rs`; this
//! is the zig port. concepts ported, not lines — the rust file is
//! ~1500 lines of accumulated phase work, while zig-substrate is
//! starting fresh with V4 semantics from the start.
//!
//! ## V3 env semantics (lookup / set / bind)
//!
//! per `docs/superpowers/specs/2026-05-09-vat-V3-here-form-design.md`
//! §6, env-Forms chain via the `parent` meta key; lookup walks:
//!
//! 1. the current Form's slots,
//! 2. if `:meta at: 'view-target` is set, the target Form's slots,
//! 3. the Form's `parent` meta (recurse).
//!
//! V4 carries this verbatim. nursery-delta interleaving (V1) is NOT
//! yet present in zig-substrate; it will land as a separate pass.
//!
//! ## V4 references
//!
//! - opcode + image format: `2026-05-10-vm-V4-opcodes-design.md`
//! - phase plan: `2026-05-10-vm-V4-polyglot-substrate.md` Track A.4

const std = @import("std");

const value = @import("value.zig");
const Value = value.Value;

const form = @import("form.zig");
const FormId = form.FormId;
const Form = form.Form;

const sym_mod = @import("sym.zig");
const SymTable = sym_mod.SymTable;
const SymId = u32;

const heap_mod = @import("heap.zig");
const Heap = heap_mod.Heap;

const protos_mod = @import("protos.zig");
pub const Protos = protos_mod.Protos;

/// the bytecode interpreter's per-vat state.
///
/// `stack` is the operand stack; frames are pushed/popped on call /
/// return. `last_send_sel` tracks the most recently dispatched
/// selector for error-message hygiene (used by raise paths in the
/// rust seed; carried forward for parity).
pub const Vm = struct {
    stack: std.ArrayList(Value),
    frames: std.ArrayList(Frame),
    last_send_sel: ?SymId,

    pub fn init() Vm {
        return Vm{
            .stack = .empty,
            .frames = .empty,
            .last_send_sel = null,
        };
    }

    pub fn deinit(self: *Vm, allocator: std.mem.Allocator) void {
        self.stack.deinit(allocator);
        self.frames.deinit(allocator);
    }
};

/// a single activation record on the VM call stack.
///
/// V4 frames carry a byte-offset `pc` (chunks are byte-tagged streams
/// per spec §4) and the defining_proto needed for `SuperSend`'s
/// "lookup-starting-above" semantics (§3.3).
pub const Frame = struct {
    /// chunk-FormId currently executing.
    chunk: FormId,
    /// byte offset into chunk.body.
    pc: usize,
    /// the env-Form active for this frame.
    env: FormId,
    /// the receiver `self` for this frame.
    self_: Value,
    /// stack depth at entry — pops on Return restore to here.
    stack_base: u32,
    /// the proto on which this frame's method was defined. used by
    /// `SuperSend` to start the lookup ABOVE this proto in the chain.
    defining_proto: FormId,
};

/// inline-cache slot for one Send site. monomorphic.
///
/// invalidation per `laws/substrate-laws.md` L10: when a proto's
/// generation bumps (handler mutated), ICs whose cached_generation
/// no longer matches re-resolve on next dispatch.
///
/// `cached_singleton` is the per-instance-Form for tagged immediates
/// (e.g. Bool(true)'s singleton vs Bool(false)'s singleton). when set,
/// the IC hit must check effective-receiver-id == cached_singleton so
/// we don't reuse Bool(true)'s handler for Bool(false).
pub const ICache = struct {
    cached_proto: FormId,
    cached_method: FormId,
    cached_defining: FormId,
    cached_generation: u32,
    cached_singleton: FormId,

    pub const empty: ICache = .{
        .cached_proto = FormId.NONE,
        .cached_method = FormId.NONE,
        .cached_defining = FormId.NONE,
        .cached_generation = 0,
        .cached_singleton = FormId.NONE,
    };
};

/// the signature of a native method installed by a phase-A intrinsic
/// (or, later, by a wasm mco binding). receives the World plus the
/// receiver and slice of args; returns a Value or any error.
pub const NativeFn = *const fn (
    world: *World,
    self_: Value,
    args: []const Value,
) anyerror!Value;

/// the substrate's per-vat root.
///
/// owns the heap, sym table, proto cache, chunk side-tables, native-fn
/// registry, the `$here` form, the `Macros` form, the VM, and the
/// allocator.
///
/// `chunk_bytecode` / `chunk_consts` / `chunk_ics` are `AutoArrayHashMap`
/// for two reasons:
///   1. insertion-order iteration (determinism law D5 — replicas must
///      agree on iteration order even for substrate-internal tables),
///   2. FormId is a packed u32 → `AutoArrayHashMap` uses the bit pattern
///      as the hash key without us writing a custom Context.
pub const World = struct {
    heap: Heap,
    syms: SymTable,
    protos: Protos,
    allocator: std.mem.Allocator,

    /// chunk-FormId → byte-encoded bytecode (owned). V4 spec §4.3:
    /// chunks are serializable as `:body` Bytes.
    chunk_bytecode: std.AutoArrayHashMap(FormId, []u8),
    /// chunk-FormId → constant pool, indexed by LoadConst.idx.
    chunk_consts: std.AutoArrayHashMap(FormId, []Value),
    /// chunk-FormId → IC slot table, one entry per Send-variant op.
    chunk_ics: std.AutoArrayHashMap(FormId, []ICache),

    /// method-FormId → native function pointer.
    native_fns: std.AutoArrayHashMap(FormId, NativeFn),

    /// V3 — the "here" Form for this vat. exposed as `$here` in
    /// moof code (self-referential binding in here_form.slots).
    /// LoadHere / SendHere / TailSendHere refer to this FormId
    /// directly (bypassing any user-level `$here` rebinding per
    /// V4 spec §6.5).
    here_form: FormId,

    /// the canonical macro registry: a plain Form (proto: Object)
    /// whose slots are macro-name → method-Form. exposed as the
    /// `Macros` global so user code can introspect.
    macros_form: FormId,

    // wasm mco instances — stub for now. zig wasmtime integration
    // (per V4 §10.2's mcos/ + McoBindingsSection) lands in a later
    // pass. left commented to flag the slot.
    //
    // wasm_instances: std.AutoArrayHashMap(FormId, WasmInstance),

    /// the bytecode interpreter's per-vat state.
    vm: Vm,

    // ---- cached SymIds for hot paths (V3 env walker + boot) ----

    /// V3 — meta key recognized by envLookup / envSet. when an
    /// env-Form has `:meta at: 'view-target` set to another Form,
    /// the walker consults that Form's slots after its own (one
    /// level — does not recurse into target's parent chain). used
    /// by `Object:eval:` to splice an obj's slots into the lookup
    /// chain without mutating obj.
    view_target_sym: SymId,
    /// V3 — meta key for env-chain parent linkage. an env-Form
    /// chains to its enclosing scope via `meta at: 'parent`.
    parent_sym: SymId,

    /// initialize a fresh, empty world.
    ///
    /// allocates: heap, sym table, all canonical protos, the
    /// `$here` form, the `Macros` form. binds `$here` self-
    /// referentially inside here_form.slots so user code can
    /// reach it via env-lookup.
    pub fn init(allocator: std.mem.Allocator) !World {
        var heap = try Heap.init(allocator);
        errdefer heap.deinit();

        var syms = try SymTable.init(allocator);
        errdefer syms.deinit();

        const protos = try protos_mod.bootstrap(&heap, &syms, allocator);

        // intern hot-path syms first; the env walker uses these.
        const view_target_sym = try syms.intern("view-target");
        const parent_sym = try syms.intern("parent");
        const here_sym = try syms.intern("$here");
        const name_meta = try syms.intern("name");

        // allocate the here_form: proto = Env, meta.parent = Nil
        // (it's the root of the env chain for this vat).
        var here_form_init = Form.withProto(Value{ .form = protos.env });
        try here_form_init.meta.put(parent_sym, Value.nil);
        const here_form = try heap.alloc(here_form_init);

        // allocate the Macros form: proto = Object, meta.name = Sym("Macros")
        // so reflection shows the name.
        var macros_form_init = Form.withProto(Value{ .form = protos.object });
        const macros_sym = try syms.intern("Macros");
        try macros_form_init.meta.put(name_meta, Value{ .sym = macros_sym });
        const macros_form = try heap.alloc(macros_form_init);

        // bind $here self-referentially inside here_form.slots —
        // moof code reaches its own globals env via this binding;
        // also lets reflection list path-bound names.
        const here_form_ref = heap.getMut(here_form);
        try here_form_ref.slots.put(here_sym, Value{ .form = here_form });

        return World{
            .heap = heap,
            .syms = syms,
            .protos = protos,
            .allocator = allocator,
            .chunk_bytecode = std.AutoArrayHashMap(FormId, []u8).init(allocator),
            .chunk_consts = std.AutoArrayHashMap(FormId, []Value).init(allocator),
            .chunk_ics = std.AutoArrayHashMap(FormId, []ICache).init(allocator),
            .native_fns = std.AutoArrayHashMap(FormId, NativeFn).init(allocator),
            .here_form = here_form,
            .macros_form = macros_form,
            .vm = Vm.init(),
            .view_target_sym = view_target_sym,
            .parent_sym = parent_sym,
        };
    }

    /// free everything owned by this World.
    ///
    /// `chunk_bytecode` values are slices owned by World; free those
    /// individually before deiniting the map. `chunk_consts` / `chunk_ics`
    /// slices are likewise owned. NativeFn entries are function pointers
    /// (no ownership).
    pub fn deinit(self: *World) void {
        // free owned slices in side tables.
        var it_bytes = self.chunk_bytecode.iterator();
        while (it_bytes.next()) |entry| {
            self.allocator.free(entry.value_ptr.*);
        }
        self.chunk_bytecode.deinit();

        var it_consts = self.chunk_consts.iterator();
        while (it_consts.next()) |entry| {
            self.allocator.free(entry.value_ptr.*);
        }
        self.chunk_consts.deinit();

        var it_ics = self.chunk_ics.iterator();
        while (it_ics.next()) |entry| {
            self.allocator.free(entry.value_ptr.*);
        }
        self.chunk_ics.deinit();

        self.native_fns.deinit();
        self.vm.deinit(self.allocator);
        self.syms.deinit();
        self.heap.deinit();
    }

    // ---- env machinery (V3 spec §6) ------------------------------

    /// look up `name` in `env` and its parent chain.
    ///
    /// at each frame: check slots first; if absent, consult
    /// `:meta at: 'view-target` (V3 — one-level splice); if still
    /// absent, recurse via `:meta at: 'parent`. returns null when
    /// the chain terminates without a binding.
    ///
    /// note we distinguish `Some(Nil)` (bound to nil) from `None`
    /// (unbound) — required for `set!`-on-nil-bound vs unbound.
    pub fn envLookup(self: *const World, env: FormId, name: SymId) ?Value {
        var cur = env;
        // defensive bound — env chains shouldn't be deep, but a
        // mis-wired :parent could cycle. matches MAX_PROTO_DEPTH in
        // the rust seed (256). cheap insurance.
        var hops: usize = 0;
        const MAX_HOPS: usize = 256;
        while (hops < MAX_HOPS) : (hops += 1) {
            const f = self.heap.get(cur);

            // 1. current Form's slots.
            if (f.slots.get(name)) |v| return v;

            // 2. V3 — view-target consultation (one-level).
            if (f.meta.get(self.view_target_sym)) |target_v| {
                if (target_v == .form) {
                    const tf = self.heap.get(target_v.form);
                    if (tf.slots.get(name)) |v| return v;
                }
            }

            // 3. walk parent.
            const parent_v = f.meta.get(self.parent_sym) orelse return null;
            switch (parent_v) {
                .form => |id| cur = id,
                else => return null, // Nil or non-Form terminates the chain.
            }
        }
        // exhausted hop budget — treat as unbound rather than loop.
        return null;
    }

    /// `set!` semantics: walk the chain looking for an EXISTING
    /// binding of `name`; if found, mutate it in place and return
    /// true. if no binding is found anywhere in the chain, return
    /// false (caller decides whether to define-locally or raise).
    ///
    /// view-target writes are LIVE: if the current frame's
    /// view-target has `name` bound, the write goes there (per V3
    /// spec §6 — `Object:eval:` mutations propagate to the spliced
    /// obj). matches the rust seed.
    pub fn envSet(self: *World, env: FormId, name: SymId, val: Value) !bool {
        var cur = env;
        var hops: usize = 0;
        const MAX_HOPS: usize = 256;
        while (hops < MAX_HOPS) : (hops += 1) {
            // present-in-current-frame? contains_key, not "lookup +
            // is-nil" — we must hit bindings-to-Nil.
            const f = self.heap.get(cur);
            if (f.slots.contains(name)) {
                const fm = self.heap.getMut(cur);
                try fm.slots.put(name, val);
                return true;
            }

            // V3 view-target write-through.
            if (f.meta.get(self.view_target_sym)) |target_v| {
                if (target_v == .form) {
                    const target_id = target_v.form;
                    const tf = self.heap.get(target_id);
                    if (tf.slots.contains(name)) {
                        const tfm = self.heap.getMut(target_id);
                        try tfm.slots.put(name, val);
                        return true;
                    }
                }
            }

            const parent_v = f.meta.get(self.parent_sym) orelse return false;
            switch (parent_v) {
                .form => |id| cur = id,
                else => return false,
            }
        }
        return false;
    }

    /// bind `name` in `env`'s local scope. does not walk parents.
    /// equivalent to `def` in scheme — establishes a new local
    /// binding (or overwrites an existing local one).
    pub fn envBind(self: *World, env: FormId, name: SymId, val: Value) !void {
        const fm = self.heap.getMut(env);
        try fm.slots.put(name, val);
    }
};
