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
//!
//! ## integration note (integration-agent)
//!
//! per agent reports, several methods here are STUBS that panic with
//! `@panic("TODO: integration agent")`. that's deliberate — the
//! minimum-viable smoke (`PushNil;Return`, `LoadConst;LoadConst;Send;
//! Return`) doesn't exercise them. fill in as later phases need them.

const std = @import("std");

const value = @import("value.zig");
const Value = value.Value;

const form = @import("form.zig");
const FormId = form.FormId;
const Form = form.Form;

const sym_mod = @import("sym.zig");
const SymTable = sym_mod.SymTable;
pub const SymId = u32;

const heap_mod = @import("heap.zig");
const Heap = heap_mod.Heap;

const protos_mod = @import("protos.zig");
pub const Protos = protos_mod.Protos;

const vm_mod = @import("vm.zig");

const gc_mod = @import("gc.zig");
pub const GcStats = gc_mod.GcStats;

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

    /// thin shim so callers (intrinsics' :callIn:withSelf:) can write
    /// `world.vm.runMethod(...)`. delegates to the free-function in
    /// vm.zig.
    pub fn runMethod(
        self: *Vm,
        world: *World,
        chunk: FormId,
        env: FormId,
        self_v: Value,
        defining_proto: FormId,
    ) anyerror!Value {
        _ = self;
        return vm_mod.runMethod(world, chunk, env, self_v, defining_proto);
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

/// payload stashed in `far_ref_table` per V4 spec §10.4. resolution
/// is lazy — populated by image-load, dereferenced on first VM hit.
pub const FarRef = struct {
    target_vat_id: [16]u8,
    target_form_id: u32,
};

/// result of `lookupHandler` — the method-Form Value plus the proto
/// it was found on (needed by SuperSend's "lookup-above" semantics
/// and by the IC's `cached_defining` slot).
pub const HandlerHit = struct {
    handler: Value,
    defining: FormId,
};

/// the substrate's per-vat root.
///
/// owns the heap, sym table, proto cache, chunk side-tables, native-fn
/// registry, the `$here` form, the `Macros` form, the VM, and the
/// allocator.
///
/// hash maps are all `AutoArrayHashMapUnmanaged` for two reasons:
///   1. insertion-order iteration (determinism law D5 — replicas must
///      agree on iteration order even for substrate-internal tables),
///   2. FormId is a packed u32 → unmanaged variant uses the bit
///      pattern as the hash key without us writing a custom Context.
///
/// the `Unmanaged` choice (per agent-report flag #1+#2) means every
/// `.put` / `.get` / `.deinit` call takes the world's allocator as
/// its first argument. we hold one on `World.allocator`.
pub const World = struct {
    heap: Heap,
    syms: SymTable,
    protos: Protos,
    allocator: std.mem.Allocator,

    /// optional std.Io handle for natives that need filesystem access
    /// (e.g. `:serializeTo:`). zig 0.16 routes all fs through std.Io.Dir;
    /// natives running deep in the dispatch tree don't have it in scope
    /// unless we stash it here. nullable so default-constructed Worlds
    /// (tests) still work — natives that need io must check.
    io: ?std.Io = null,

    /// the `$transporter` cap's resolved lib root — `MOOF_LIB` env var,
    /// `<exe>/../lib`, or `./lib`. set by the host when launching `moof
    /// run`; natives consult it via `transporter:load:` / `:loadAll:`.
    /// owned by the world's allocator; freed in deinit.
    transporter_root: ?[]u8 = null,

    /// mirror of `crates/substrate/src/world.rs::use_moof_compiler`.
    /// when `true`, `$compiler` is in-image — every compile routes
    /// through `[Compiler compileTop: form]`. zig substrate has no
    /// native compiler, so this MUST be true for any compile to
    /// happen. defaulted false; flipped by `[$compiler useMoof]`.
    use_moof_compiler: bool = false,

    /// mirror of `crates/substrate/src/world.rs::use_moof_reader`.
    /// when `true`, `$reader` is in-image — every parse routes through
    /// `[Parser parse: src]`. zig has no native reader, so this MUST
    /// be true. defaulted false; flipped by `[$reader useMoof]`.
    use_moof_reader: bool = false,

    /// chunk-FormId → byte-encoded bytecode (owned). V4 spec §4.3:
    /// chunks are serializable as `:body` Bytes.
    chunk_bytecode: std.AutoArrayHashMapUnmanaged(FormId, []u8),
    /// chunk-FormId → constant pool, indexed by LoadConst.idx.
    chunk_consts: std.AutoArrayHashMapUnmanaged(FormId, []Value),
    /// chunk-FormId → IC slot table, one entry per Send-variant op.
    chunk_ics: std.AutoArrayHashMapUnmanaged(FormId, []ICache),
    /// chunk-FormId → param-sym list (image.zig loads these).
    chunk_params: std.AutoArrayHashMapUnmanaged(FormId, []u32),

    /// method-FormId → native function pointer.
    native_fns: std.AutoArrayHashMapUnmanaged(FormId, NativeFn),

    /// FormId(.far_ref) → FarRef. populated by image-load (V4 §10.4).
    far_ref_table: std.AutoArrayHashMapUnmanaged(FormId, FarRef),

    /// proto-FormId → handler-table generation. incremented when a
    /// handler is rewritten via `set-handler!`. ICs compare to detect
    /// staleness (law L10). missing key implies generation 0.
    proto_generation: std.AutoArrayHashMapUnmanaged(FormId, u32),

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

    /// the bytecode interpreter's per-vat state.
    vm: Vm,

    /// phase 1 GC controls. when `gc_enabled` is true, `runTop`
    /// triggers a mark-sweep cycle on exit (the "turn boundary
    /// stand-in" — see `gc.zig` and spec §3.5 option A). when
    /// `gc_stats_enabled` is true, each cycle prints a one-line
    /// summary to stderr. both flipped by main.zig via env vars /
    /// CLI flags.
    gc_enabled: bool = true,
    gc_stats_enabled: bool = false,

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

    /// `'does-not-understand:with:` — the canonical dnu selector.
    dnu_sym: SymId,
    /// `'body` — slot on method-Forms holding the chunk FormId.
    body_sym: SymId,
    /// `'env` — slot on method/closure-Forms holding the captured env.
    env_sym: SymId,
    /// `'params` — slot on method/closure-Forms holding the param-list.
    params_sym: SymId,
    /// `'car` — slot 0 of a Cons.
    symCar: SymId,
    /// `'cdr` — slot 1 of a Cons.
    symCdr: SymId,
    /// `'body` — alias of body_sym for intrinsics naming.
    symBody: SymId,
    /// `'parent` — alias of parent_sym for intrinsics naming.
    symParent: SymId,
    /// `'name` — meta key for proto display names.
    symName: SymId,
    /// `'self` — slot on closures holding captured receiver.
    self_sym: SymId,

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
        const dnu_sym = try syms.intern("does-not-understand:with:");
        const body_sym = try syms.intern("body");
        const env_sym = try syms.intern("env");
        const params_sym = try syms.intern("params");
        const car_sym = try syms.intern("car");
        const cdr_sym = try syms.intern("cdr");
        const self_sym = try syms.intern("self");

        // allocate the here_form: proto = Env, meta.parent = Nil
        // (it's the root of the env chain for this vat).
        var here_form_init = Form.withProto(Value{ .form = protos.env });
        try here_form_init.meta.put(allocator, parent_sym, Value.nil);
        const here_form = try heap.alloc(here_form_init);

        // allocate the Macros form: proto = Object, meta.name = Sym("Macros")
        // so reflection shows the name.
        var macros_form_init = Form.withProto(Value{ .form = protos.object });
        const macros_sym = try syms.intern("Macros");
        try macros_form_init.meta.put(allocator, name_meta, Value{ .sym = macros_sym });
        const macros_form = try heap.alloc(macros_form_init);

        // bind $here self-referentially inside here_form.slots —
        // moof code reaches its own globals env via this binding;
        // also lets reflection list path-bound names.
        const here_form_ref = heap.getMut(here_form);
        try here_form_ref.slots.put(allocator, here_sym, Value{ .form = here_form });

        return World{
            .heap = heap,
            .syms = syms,
            .protos = protos,
            .allocator = allocator,
            .chunk_bytecode = .empty,
            .chunk_consts = .empty,
            .chunk_ics = .empty,
            .chunk_params = .empty,
            .native_fns = .empty,
            .far_ref_table = .empty,
            .proto_generation = .empty,
            .here_form = here_form,
            .macros_form = macros_form,
            .vm = Vm.init(),
            .view_target_sym = view_target_sym,
            .parent_sym = parent_sym,
            .dnu_sym = dnu_sym,
            .body_sym = body_sym,
            .env_sym = env_sym,
            .params_sym = params_sym,
            .symCar = car_sym,
            .symCdr = cdr_sym,
            .symBody = body_sym,
            .symParent = parent_sym,
            .symName = name_meta,
            .self_sym = self_sym,
        };
    }

    /// initialize a "bare" World — no protos, no `$here`, no `Macros`.
    ///
    /// used by image-load (V4 §10). the image carries the canonical
    /// FormIds for here_form / macros_form / all 18 protos in its
    /// Header; `image.loadVatImage` fills them in after deserializing
    /// the FormSection. allocating them here would conflict with the
    /// FormIds the image expects.
    ///
    /// the hot-path SymIds (parent, view-target, etc.) are still
    /// interned — the env-walker assumes they exist. image hydration
    /// overwrites them via `clearAndKeepCapacity` + intern-loop, so
    /// the syms re-intern in image order. **after load** the cached
    /// SymId fields on World may be stale; callers that exercise V3
    /// env semantics on an image-loaded World should re-cache them.
    /// for V4 phase α (just load + inspect) this is fine.
    pub fn initBare(allocator: std.mem.Allocator) !World {
        var heap = try Heap.init(allocator);
        errdefer heap.deinit();

        var syms = try SymTable.init(allocator);
        errdefer syms.deinit();

        // intern the hot-path syms so env-walker / intrinsics that
        // touch them on a bare-but-not-yet-loaded world don't NPE.
        // image-load will clearAndKeepCapacity these and re-intern
        // from its own table; the cached SymIds below become stale
        // at that point — see doc note above.
        const view_target_sym = try syms.intern("view-target");
        const parent_sym = try syms.intern("parent");
        const dnu_sym = try syms.intern("does-not-understand:with:");
        const body_sym = try syms.intern("body");
        const env_sym = try syms.intern("env");
        const params_sym = try syms.intern("params");
        const car_sym = try syms.intern("car");
        const cdr_sym = try syms.intern("cdr");
        const self_sym = try syms.intern("self");
        const name_sym = try syms.intern("name");

        // every proto FormId starts at NONE; image's header populates.
        const none_protos: Protos = .{
            .object = FormId.NONE,
            .nil = FormId.NONE,
            .bool_ = FormId.NONE,
            .integer = FormId.NONE,
            .char = FormId.NONE,
            .sym = FormId.NONE,
            .cons = FormId.NONE,
            .string = FormId.NONE,
            .bytes = FormId.NONE,
            .method = FormId.NONE,
            .chunk = FormId.NONE,
            .closure = FormId.NONE,
            .env = FormId.NONE,
            .foreign_handle = FormId.NONE,
            .table = FormId.NONE,
            .frame = FormId.NONE,
            .macros = FormId.NONE,
            .opcode = FormId.NONE,
        };

        return World{
            .heap = heap,
            .syms = syms,
            .protos = none_protos,
            .allocator = allocator,
            .chunk_bytecode = .empty,
            .chunk_consts = .empty,
            .chunk_ics = .empty,
            .chunk_params = .empty,
            .native_fns = .empty,
            .far_ref_table = .empty,
            .proto_generation = .empty,
            .here_form = FormId.NONE,
            .macros_form = FormId.NONE,
            .vm = Vm.init(),
            .view_target_sym = view_target_sym,
            .parent_sym = parent_sym,
            .dnu_sym = dnu_sym,
            .body_sym = body_sym,
            .env_sym = env_sym,
            .params_sym = params_sym,
            .symCar = car_sym,
            .symCdr = cdr_sym,
            .symBody = body_sym,
            .symParent = parent_sym,
            .symName = name_sym,
            .self_sym = self_sym,
        };
    }

    /// free everything owned by this World.
    ///
    /// `chunk_bytecode` values are slices owned by World; free those
    /// individually before deiniting the map. `chunk_consts` / `chunk_ics`
    /// / `chunk_params` slices are likewise owned. NativeFn entries are
    /// function pointers (no ownership).
    pub fn deinit(self: *World) void {
        // free owned slices in side tables.
        var it_bytes = self.chunk_bytecode.iterator();
        while (it_bytes.next()) |entry| {
            self.allocator.free(entry.value_ptr.*);
        }
        self.chunk_bytecode.deinit(self.allocator);

        var it_consts = self.chunk_consts.iterator();
        while (it_consts.next()) |entry| {
            self.allocator.free(entry.value_ptr.*);
        }
        self.chunk_consts.deinit(self.allocator);

        var it_ics = self.chunk_ics.iterator();
        while (it_ics.next()) |entry| {
            self.allocator.free(entry.value_ptr.*);
        }
        self.chunk_ics.deinit(self.allocator);

        var it_params = self.chunk_params.iterator();
        while (it_params.next()) |entry| {
            self.allocator.free(entry.value_ptr.*);
        }
        self.chunk_params.deinit(self.allocator);

        self.native_fns.deinit(self.allocator);
        self.far_ref_table.deinit(self.allocator);
        self.proto_generation.deinit(self.allocator);
        self.vm.deinit(self.allocator);
        self.syms.deinit();
        self.heap.deinit();
        if (self.transporter_root) |root| self.allocator.free(root);
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
                try fm.slots.put(self.allocator, name, val);
                return true;
            }

            // V3 view-target write-through.
            if (f.meta.get(self.view_target_sym)) |target_v| {
                if (target_v == .form) {
                    const target_id = target_v.form;
                    const tf = self.heap.get(target_id);
                    if (tf.slots.contains(name)) {
                        const tfm = self.heap.getMut(target_id);
                        try tfm.slots.put(self.allocator, name, val);
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
        try fm.slots.put(self.allocator, name, val);
    }

    // ---- proto / form access -------------------------------------

    /// the proto Value for any Value (tagged-immediate or Form).
    /// tagged immediates resolve to their canonical proto-Form per V0.
    pub fn protoOf(self: *const World, v: Value) Value {
        return switch (v) {
            .nil => .{ .form = self.protos.nil },
            .bool_ => .{ .form = self.protos.bool_ },
            .int => .{ .form = self.protos.integer },
            .sym => .{ .form = self.protos.sym },
            .char => .{ .form = self.protos.char },
            .float => .{ .form = self.protos.object }, // Float proto deferred (phase γ)
            .form => |id| self.heap.get(id).proto,
        };
    }

    /// the effective FormId for any Value, where defined. tagged
    /// immediates currently have no per-instance singleton FormId
    /// (returned as null); future singletons (#true / #false) will
    /// fill this in.
    pub fn effectiveFormId(self: *const World, v: Value) ?FormId {
        _ = self;
        return switch (v) {
            .form => |id| id,
            else => null,
        };
    }

    /// read `slot_name` on `id`, walking only the Form's own slots
    /// (no proto-chain). returns nil if absent.
    pub fn formSlot(self: *const World, id: FormId, slot_name: SymId) Value {
        const f = self.heap.get(id);
        return f.slot(slot_name);
    }

    /// write `val` to slot `slot_name` on `id`. errors on frozen or
    /// out-of-memory.
    pub fn formSlotSet(self: *World, id: FormId, slot_name: SymId, val: Value) !void {
        const fm = self.heap.getMut(id);
        if (fm.frozen) return error.FrozenForm;
        try fm.slots.put(self.allocator, slot_name, val);
    }

    /// read `key` on `id.meta`. nil if absent.
    pub fn formMeta(self: *const World, id: FormId, key: SymId) Value {
        const f = self.heap.get(id);
        return f.metaAt(key);
    }

    /// `[a become: b]` — record a heap-level indirection. wraps
    /// `Heap.become_` and bumps any relevant proto generations so
    /// stale ICs re-resolve.
    pub fn become_(self: *World, a: FormId, b: FormId) !void {
        if (a.eql(b)) return; // self-become is a no-op
        try self.heap.become_(a, b);
        // bump generation on a's slot if it was a proto — cheap to
        // do unconditionally since proto_generation is just a u32.
        try self.bumpGeneration(a);
    }

    /// bump `proto`'s handler-generation counter. called by become_,
    /// set-handler!, etc. — anything that could stale an IC.
    pub fn bumpGeneration(self: *World, proto: FormId) !void {
        const gop = try self.proto_generation.getOrPut(self.allocator, proto);
        if (!gop.found_existing) gop.value_ptr.* = 0;
        gop.value_ptr.* +%= 1;
    }

    /// look up `proto`'s current handler-generation. missing → 0.
    pub fn protoGeneration(self: *const World, proto: FormId) u32 {
        return self.proto_generation.get(proto) orelse 0;
    }

    // ---- handler lookup (proto-chain walk) -----------------------

    /// walk the proto chain starting AT `start_proto` looking for a
    /// handler for `selector`. used by lookupHandler and lookupHandlerSuper
    /// (with different starting points).
    fn walkChain(self: *const World, start: FormId, selector: SymId) ?HandlerHit {
        var cur = start;
        var hops: usize = 0;
        const MAX_HOPS: usize = 256;
        while (hops < MAX_HOPS) : (hops += 1) {
            const f = self.heap.get(cur);
            if (f.handler(selector)) |h| {
                return .{ .handler = h, .defining = cur };
            }
            switch (f.proto) {
                .form => |id| cur = id,
                else => return null,
            }
        }
        return null;
    }

    /// resolve `selector` for `receiver`: checks the receiver's OWN
    /// handler table first (so proto-as-receiver and singleton-method
    /// sends dispatch correctly), then walks the proto chain. returns
    /// the matched handler + defining proto, or null on miss.
    /// mirrors rust crates/substrate/src/world.rs::lookup_handler.
    pub fn lookupHandler(self: *const World, receiver: Value, selector: SymId) ?HandlerHit {
        // 1. receiver's own handlers (singleton / proto-as-receiver).
        if (self.effectiveFormId(receiver)) |id| {
            const f = self.heap.get(id);
            if (f.handler(selector)) |h| {
                return .{ .handler = h, .defining = id };
            }
        }
        // 2. walk the proto chain.
        const proto_v = self.protoOf(receiver);
        return switch (proto_v) {
            .form => |id| self.walkChain(id, selector),
            else => null,
        };
    }

    /// super-send lookup: start the walk ABOVE `defining_proto`.
    /// used by SuperSend (V4 spec §6.3).
    pub fn lookupHandlerSuper(self: *const World, defining: FormId, selector: SymId) ?HandlerHit {
        const d = self.heap.get(defining);
        return switch (d.proto) {
            .form => |id| self.walkChain(id, selector),
            else => null,
        };
    }

    /// `method` → native function pointer, if any.
    pub fn nativeFn(self: *const World, method: FormId) ?NativeFn {
        return self.native_fns.get(method);
    }

    // ---- VM helpers (called by vm.zig + intrinsics) --------------

    /// allocate a new Env-Form with `parent` linked via meta.
    pub fn allocEnv(self: *World, parent: FormId) !FormId {
        var f = Form.withProto(.{ .form = self.protos.env });
        try f.meta.put(self.allocator, self.parent_sym, .{ .form = parent });
        return self.heap.alloc(f);
    }

    /// allocate a Closure-Form. captures (chunk, env, self) for
    /// later invocation via `:call*`.
    pub fn allocClosure(
        self: *World,
        chunk: FormId,
        env: FormId,
        captured_self: Value,
    ) !FormId {
        var f = Form.withProto(.{ .form = self.protos.closure });
        try f.slots.put(self.allocator, self.body_sym, .{ .form = chunk });
        try f.slots.put(self.allocator, self.env_sym, .{ .form = env });
        // canonical slot name is `:captured-self` (matches rust
        // substrate and intrinsics Method:call). previous bug: this
        // wrote to `:self`, so Method:call dispatched with nil receiver.
        const captured_self_sym = try self.syms.intern("captured-self");
        try f.slots.put(self.allocator, captured_self_sym, captured_self);

        // V4: also bind :params slot so prepareInvoke knows the arity.
        // the side-table stores SymIds; we build a Value-list.
        if (self.chunk_params.get(chunk)) |p_syms| {
            var params_vals = try self.allocator.alloc(Value, p_syms.len);
            defer self.allocator.free(params_vals);
            for (p_syms, 0..) |s, i| params_vals[i] = .{ .sym = s };
            const params_v = try self.makeList(params_vals);
            try f.slots.put(self.allocator, self.params_sym, params_v);
        }

        return self.heap.alloc(f);
    }

    /// send `selector` to `receiver` with `args`. wraps the slow
    /// dispatch path; mostly called by intrinsics that need to
    /// re-enter the VM ("option α" per spec §4.5).
    ///
    /// for bytecode methods this pushes a new frame and drives the
    /// dispatch loop until that frame returns — one level of
    /// host-stack recursion per nested native→moof call, bounded
    /// by native count, not moof depth.
    ///
    /// for native methods (or no-handler fall-through to dnu) this
    /// returns the result directly.
    pub fn send(self: *World, receiver: Value, selector: SymId, args: []const Value) !Value {
        // resolve dispatch via the shared slow-send machinery. since
        // there's no current bytecode frame to anchor on (this is
        // called from outside the dispatch loop, or from a native),
        // we pass `shrink_to` = current stack length so prepareInvoke
        // won't touch existing operand stack contents.
        //
        // note: there's no IC for `World.send` — no chunk context to
        // key against. always slow path.
        const start_stack = self.vm.stack.items.len;
        const start_depth = self.vm.frames.items.len;
        const action = try vm_mod.prepareSlowSend(self, receiver, selector, args, start_stack);
        switch (action) {
            .native_done => |result| return result,
            .bytecode_pushed => {
                // sub-loop: drive the outer dispatch until our pushed
                // frame's Return brings frames.len back to start_depth.
                // the new frame's stack_base = start_stack; Return
                // truncates the stack to start_stack and pushes the
                // result. we pop it.
                try vm_mod.runUntilFrameReturns(self, start_depth);
                if (self.vm.stack.items.len <= start_stack) return .nil;
                return self.vm.stack.pop().?;
            },
        }
    }

    /// canonical "raise" — for V4 phase α we just return an error.
    /// the rust seed has a structured error-Form; the zig substrate
    /// will follow once condition-handling lands.
    pub fn raise(self: *World, kind: []const u8, msg: []const u8) anyerror {
        _ = self;
        _ = kind;
        _ = msg;
        return error.DispatchError;
    }

    // ---- list / string helpers (stubs for now) -------------------

    /// walk a cons-chain into a heap-allocated slice of Values.
    /// caller owns the slice (free with `freeSlice` or `allocator.free`).
    /// nil terminates; non-Cons / non-nil mid-chain raises type-error.
    pub fn listToSlice(self: *World, list: Value) ![]Value {
        // count first
        var n: usize = 0;
        var cur = list;
        while (true) {
            switch (cur) {
                .nil => break,
                .form => |id| {
                    const f = self.heap.get(id);
                    // a Cons has slots {car, cdr}; if not, treat as
                    // terminator (matches rust seed leniency).
                    if (!f.slotPresent(self.symCar)) break;
                    n += 1;
                    cur = f.slot(self.symCdr);
                },
                else => break,
            }
        }
        const out = try self.allocator.alloc(Value, n);
        cur = list;
        var i: usize = 0;
        while (i < n) : (i += 1) {
            const id = cur.asFormId().?;
            const f = self.heap.get(id);
            out[i] = f.slot(self.symCar);
            cur = f.slot(self.symCdr);
        }
        return out;
    }

    /// alias of `listToSlice` matching the rust naming.
    pub fn listToVec(self: *World, list: Value) ![]Value {
        return self.listToSlice(list);
    }

    /// free a slice returned by `listToSlice` / `listToVec`.
    pub fn freeSlice(self: *World, slice: []Value) void {
        self.allocator.free(slice);
    }

    /// build a cons-chain from `values`. returns the head (or nil).
    pub fn makeList(self: *World, values: []const Value) !Value {
        var acc: Value = .nil;
        var i: usize = values.len;
        while (i > 0) {
            i -= 1;
            var f = Form.withProto(.{ .form = self.protos.cons });
            try f.slots.put(self.allocator, self.symCar, values[i]);
            try f.slots.put(self.allocator, self.symCdr, acc);
            const id = try self.heap.alloc(f);
            acc = .{ .form = id };
        }
        return acc;
    }

    /// build a String-Form from `text`. minimum-viable: allocates a
    /// Form with proto=String and a single `:bytes` slot holding the
    /// text as a cons-chain of Char codepoints. matches the
    /// ocaml-seed lift convention (see build_seed_cmd.ml's
    /// build_form_for) and what the moof parser expects on `:bytes`.
    pub fn makeString(self: *World, text: []const u8) !Value {
        const bytes_sym = try self.syms.intern("bytes");
        // decode utf-8 into Value.char per codepoint.
        var chars: std.ArrayList(Value) = .empty;
        defer chars.deinit(self.allocator);
        var it = std.unicode.Utf8Iterator{ .bytes = text, .i = 0 };
        while (it.nextCodepoint()) |cp| {
            try chars.append(self.allocator, .{ .char = @intCast(cp) });
        }
        const chain = try self.makeList(chars.items);
        var f = Form.withProto(.{ .form = self.protos.string });
        try f.slots.put(self.allocator, bytes_sym, chain);
        const id = try self.heap.alloc(f);
        return .{ .form = id };
    }

    /// set the transporter root. takes ownership of `root` (caller
    /// passes a heap slice or arena-allocated bytes; world frees in
    /// deinit). prior root, if any, is freed.
    pub fn setTransporterRoot(self: *World, root: []const u8) !void {
        if (self.transporter_root) |old| self.allocator.free(old);
        self.transporter_root = try self.allocator.dupe(u8, root);
    }

    /// trigger a mark-sweep GC cycle. callable from any quiescent
    /// point (no mid-turn invariant violations) — phase 1's intended
    /// caller is `vm.runTop` on exit of the outermost frame.
    ///
    /// returns the cycle's stats; printing to stderr is gated on
    /// `world.gc_stats_enabled`. cycles are skipped (and `null`
    /// returned) when `world.gc_enabled` is false (the `--no-gc`
    /// diagnostic path).
    pub fn collect(self: *World) !?GcStats {
        if (!self.gc_enabled) return null;
        const stats = try gc_mod.collect(self);
        if (self.gc_stats_enabled) gc_mod.printStats(stats);
        return stats;
    }

    /// look up a named native in the process intrinsics table.
    /// image-load (image.zig::readNativeRefs) uses this to rebind
    /// natives on freshly-deserialized methods. backed by the
    /// comptime REGISTRY in intrinsics.zig — names match the rust
    /// v4_export's NativeRefsSection format ("ProtoName:selector").
    pub fn lookupNativeByName(self: *const World, name: []const u8) ?NativeFn {
        _ = self;
        // late import to avoid a top-level cycle (intrinsics imports
        // world). zig comptime @import returns a struct; this works
        // because we only access REGISTRY at call time.
        const intrinsics = @import("intrinsics.zig");
        return intrinsics.REGISTRY.get(name);
    }
};
