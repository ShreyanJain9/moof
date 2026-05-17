//! the substrate's per-vat root. owns the heap, sym table, protos,
//! chunk side-tables, native-fn registry, and the bytecode interpreter
//! state. one World per vat (V4 polyglot-substrate plan, Track A).
//!
//! the rust seed equivalent is `players/rust/src/world.rs`; this
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
//! V4 carries this verbatim.
//!
//! ## V1 per-turn nursery + diff
//!
//! per `docs/superpowers/specs/2026-05-06-vat-V1-nursery-diff-
//! design.md`, mutations during a turn buffer in `nursery_deltas`
//! for forms whose FormId payload predates the turn's watermark.
//! forms allocated this turn live in the canonical heap directly
//! (above `turn_watermark`) — they're new, so mutations to them
//! ARE the canonical value. `formSlot` / `formSlotSet` route
//! through this. `startTurn` / `commitTurn` / `abortTurn` bracket
//! the unit of atomicity; outermost `runTop` wraps automatically.
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

const nursery_mod = @import("nursery.zig");
pub const Delta = nursery_mod.Delta;
pub const FaceKind = nursery_mod.FaceKind;
pub const TurnDiff = nursery_mod.TurnDiff;

/// the bytecode interpreter's per-vat state.
///
/// `stack` is the operand stack; frames are pushed/popped on call /
/// return. `last_send_sel` tracks the most recently dispatched
/// selector for error-message hygiene (used by raise paths in the
/// rust seed; carried forward for parity).
///
/// `args_scratch` is a per-Vm bump-allocator buffer for native-call
/// argument slices (per phase 2 §4.2). zig's `prepareInvoke` previously
/// did `world.allocator.alloc(Value, argc)` + memcpy + defer free per
/// Send → ~80 ns/Send at smp_allocator, far more at DebugAllocator.
/// the scratch buffer is reused across calls; only natives that
/// re-enter the VM (`World.send`) extend it. when capacity is
/// exceeded, the underlying ArrayList grows once and never shrinks.
pub const Vm = struct {
    stack: std.ArrayList(Value),
    frames: std.ArrayList(Frame),
    /// scratch buffer for native-call args. each `prepareInvoke`
    /// native-path bumps `args_scratch.items.len` up by argc, copies
    /// from the operand stack, hands a `[]const Value` slice to the
    /// native, and on return truncates back. re-entrant `World.send`
    /// just bumps further into the buffer — bump-allocator discipline.
    args_scratch: std.ArrayList(Value),
    last_send_sel: ?SymId,

    pub fn init() Vm {
        return Vm{
            .stack = .empty,
            .frames = .empty,
            .args_scratch = .empty,
            .last_send_sel = null,
        };
    }

    pub fn deinit(self: *Vm, allocator: std.mem.Allocator) void {
        self.stack.deinit(allocator);
        self.frames.deinit(allocator);
        self.args_scratch.deinit(allocator);
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
///
/// **chunk side-table slices are cached on the Frame** (per phase 2
/// §4.3): when a frame is pushed, the relevant `chunk_bytecode`,
/// `chunk_consts`, and `chunk_ics` slices are looked up once and
/// stored here. each `step()` reads `frame.bytecode[pc]` directly —
/// no per-op hashmap lookup. **safety:** the slices are owned by the
/// side-table; live frames keep their chunk marked as a GC root (see
/// `gc.zig` seedRoots), so the side-table entry (and thus the
/// underlying allocation) stays alive for the lifetime of any frame
/// holding the cached slice. side-tables are populated at compile /
/// image-load time and value pointers don't change after insertion;
/// `fetchSwapRemove` during sweep only fires on tombstoned (unreachable)
/// chunks, which by definition no live frame can hold.
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
    /// **cached slice of `world.chunk_bytecode.get(chunk)`** — one
    /// hashmap lookup amortized over every op this frame executes.
    bytecode: []const u8,
    /// **cached slice of `world.chunk_consts.get(chunk)`** — used by
    /// LoadConst without a per-op hashmap lookup.
    consts: []const Value,
    /// **cached slice of `world.chunk_ics.get(chunk)`** — used by the
    /// Send IC fast path without a per-op hashmap lookup. mutable —
    /// IC slots are written in place on cache miss.
    ics: []ICache,
};

/// build a Frame for `chunk`, looking up the chunk's side-table slices
/// once and caching them on the frame (per phase 2 §4.3). returns
/// `error.UnknownChunk` if `chunk_bytecode` has no entry (the chunk-
/// Form exists but no bytecode is registered). `chunk_consts` /
/// `chunk_ics` default to empty slices when absent — a chunk may
/// legitimately have no consts / no ICs.
pub fn makeFrame(
    world: *World,
    chunk: FormId,
    pc: usize,
    env: FormId,
    self_v: Value,
    stack_base: u32,
    defining_proto: FormId,
) !Frame {
    const bytes = world.chunk_bytecode.get(chunk) orelse return error.UnknownChunk;
    const consts: []const Value = if (world.chunk_consts.get(chunk)) |c| c else &.{};
    const ics: []ICache = if (world.chunk_ics.get(chunk)) |i| i else &.{};
    return Frame{
        .chunk = chunk,
        .pc = pc,
        .env = env,
        .self_ = self_v,
        .stack_base = stack_base,
        .defining_proto = defining_proto,
        .bytecode = bytes,
        .consts = consts,
        .ics = ics,
    };
}

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

/// vat-mode controls the default mutability for newly-allocated forms.
/// per design spec §4.1, this is set at vat-spawn time and immutable for
/// the vat's life. in V0 (single-vat), this is held on World; in V4
/// (multi-vat), it moves to per-Vat struct.
pub const VatMode = enum {
    /// new forms are born mutable; [form freeze] is explicit.
    mutable_default,
    /// new forms auto-freeze at end of their allocation expression.
    /// internal building during alloc is mutable; on alloc-expr exit,
    /// the form locks. for parsers, compilers, computation kernels.
    frozen_default,
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

    /// vat-mode for this world. defaults to mutable_default for
    /// backward-compat with existing workspaces. moof code can set
    /// this at world-creation time via a yet-to-be-added intrinsic
    /// or via direct world.vat_mode assignment in tests.
    vat_mode: VatMode,

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

    /// mirror of `players/rust/src/world.rs::use_moof_compiler`.
    /// when `true`, `$compiler` is in-image — every compile routes
    /// through `[Compiler compileTop: form]`. zig substrate has no
    /// native compiler, so this MUST be true for any compile to
    /// happen. defaulted false; flipped by `[$compiler useMoof]`.
    use_moof_compiler: bool = false,

    /// mirror of `players/rust/src/world.rs::use_moof_reader`.
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
    ///
    /// `AutoHashMapUnmanaged` (not insertion-ordered) per phase 2 §4.7.
    /// this table is never iterated for user-observable output:
    ///   - `nativeFn(method)` point lookups (vm.zig dispatch fast path)
    ///   - `contains(mid)` checks during image serialization (the
    ///     iteration there walks `handlers`, not `native_fns`)
    ///   - GC mark/sweep walks (internal; order doesn't leak)
    /// safe to swap per D5 (no observable iteration → no order to
    /// preserve). gains O(1) amortized lookup vs the insertion-order
    /// table's two-step (hash → index → array load).
    native_fns: std.AutoHashMapUnmanaged(FormId, NativeFn),

    /// cached `Method:call` native function pointer (post-2026-05-16
    /// tail-call peephole). populated by `lookupNativeByName` the
    /// first time someone resolves "Method:call" — image-load takes
    /// that path during NativeRefsSection rebind, so the field is
    /// already warm by the time any moof code dispatches.
    ///
    /// the VM's tail-send dispatcher consults this to short-circuit
    /// `TailSend → Method:call → runMethod` recursion: when the
    /// resolved native equals this pointer AND the receiver is itself
    /// a method-Form with a `:body` chunk, frame-replace directly
    /// instead of entering the native (which would push a fresh host
    /// stack frame per tail-iteration and blow the stack on tail
    /// recursion). see vm.zig::replaceFrameWithTailCall.
    method_call_native: ?NativeFn = null,

    /// FormId(.far_ref) → FarRef. populated by image-load (V4 §10.4).
    far_ref_table: std.AutoArrayHashMapUnmanaged(FormId, FarRef),

    /// proto-FormId → handler-table generation. incremented when a
    /// handler is rewritten via `set-handler!`. ICs compare to detect
    /// staleness (law L10). missing key implies generation 0.
    proto_generation: std.AutoArrayHashMapUnmanaged(FormId, u32),

    /// **phase 2 §5.8d — proto → Layout registry.**
    ///
    /// when `Object:new` (or any constructor) allocates an instance,
    /// it looks up the proto's FormId here; if a Layout is registered,
    /// the new Form is allocated via `Form.withLayout`, putting its
    /// canonical slots inline. otherwise, a general Form is allocated.
    ///
    /// the Layouts themselves are owned by `layout_arena` — allocated
    /// once per proto, never freed (lifetime = the World). registration
    /// is one-way: a proto can grow a Layout but can't shed one (would
    /// break L11 stability for existing instances).
    ///
    /// `AutoHashMapUnmanaged` (not insertion-ordered) per phase 2 §4.7
    /// — internal table, no user-observable iteration order.
    proto_layouts: std.AutoHashMapUnmanaged(FormId, *const form.Layout) = .{},

    /// **§5.8d — arena holding Layouts and their `slot_names` slices.**
    ///
    /// each `registerLayout` allocation comes out of this arena and
    /// lives until World.deinit. arena ownership keeps the
    /// `*const Layout` pointers in `proto_layouts` stable for the
    /// World's lifetime.
    layout_arena: std.heap.ArenaAllocator,

    /// tagged-immediate → singleton FormId. lazy: populated only
    /// when user code asks for a per-instance handler (e.g.
    /// `[#true ifTrue:ifFalse: t f]`). mirrors rust's
    /// `World::tagged_storage`. needed because moof code does
    /// `(setHandler! #true 'ifTrue:ifFalse: …)` etc., where the
    /// receiver isn't a Form. resolution path:
    ///   - `effectiveFormId(v)` returns the cached singleton (if any)
    ///     and falls back to `v.asFormId()`.
    ///   - `ensureWritableFormId(v)` allocates on demand.
    tagged_storage: std.AutoArrayHashMapUnmanaged(u64, FormId),

    /// V1 — per-form mutation deltas for the active turn. keyed
    /// by the canonical FormId of pre-existing forms (payload
    /// `< turn_watermark`). forms allocated THIS turn are
    /// canonical-direct and do NOT have an entry here. cleared
    /// at commit/abort. iteration order = insertion order (D5).
    nursery_deltas: std.AutoArrayHashMapUnmanaged(FormId, Delta),

    /// V1 — the FormId payload below which forms are canonical
    /// (committed in a prior turn or at boot). forms at payloads
    /// `>= turn_watermark` are this-turn allocations. set by
    /// `startTurn` to `heap.forms.items.len`; advanced by
    /// `commitTurn` to the post-turn high-water; unchanged by
    /// `abortTurn` (which instead truncates the heap back to it).
    turn_watermark: u32,

    /// V1 — `true` iff a turn is currently active. `startTurn`
    /// flips on; `commitTurn` / `abortTurn` flip off. nested
    /// `startTurn` panics: V1 supports exactly one active turn
    /// at a time. V4 will lift this to per-vat state when
    /// multi-vat lands.
    in_turn: bool,

    /// V3 — the "here" Form for this vat. exposed as `$here` in
    /// moof code (self-referential binding in here_form.slots).
    /// LoadHere / SendHere / TailSendHere refer to this FormId
    /// directly (bypassing any user-level `$here` rebinding per
    /// V4 spec §6.5).
    here_form: FormId,

    /// **phase 2 §5.8a — String char-materialization cache.**
    ///
    /// Strings are V0-shape Forms whose `:bytes` slot holds a
    /// cons-chain of Char codepoints. naive `:at:` / `:length` walks
    /// that chain per call — O(N) per access, O(N²) to scan a string.
    /// the parser reads every codepoint via `:at:` during lex, so this
    /// dominates real-workload time.
    ///
    /// fix: lazily materialize the chain into a `[]u32` slice the
    /// first time `:at:` / `:length` / `:slice:length:` is called on
    /// a given String FormId. subsequent calls are O(1) indexed reads.
    ///
    /// invariants:
    /// - **L1 (everything is a Form) preserved** — the cache lives on
    ///   the World, not in any Form slot; moof-side reflection
    ///   (`[s :bytes]`) still walks the cons-chain.
    /// - **invalidation:** on any `formSlotSet` to a String's `:bytes`
    ///   slot (the only mutation path in zig-substrate). also evicted
    ///   on GC sweep when the String FormId is tombstoned.
    /// - **GC safety:** the cache does NOT keep Strings alive — sweep
    ///   evicts entries whose key is unmarked before tombstoning.
    /// - **AutoHashMap** (not insertion-ordered) per phase 2 §4.7 —
    ///   internal cache, no user-observable iteration order.
    string_cache: std.AutoHashMapUnmanaged(FormId, []u32) = .{},

    /// **phase 2 §5.8c — String→Sym intern cache.**
    ///
    /// `(intern str)` walks `str.:bytes` into a UTF-8 buffer then
    /// calls `syms.intern`. profile shows 267K calls / 653 unique
    /// syms = 410× redundancy. since FormId is stable for the lifetime
    /// of a vat (L11) and a String's content is conventionally
    /// immutable after parse, caching FormId → SymId is safe.
    ///
    /// invariants:
    /// - **soft cache** — a miss falls through to the slow path. an
    ///   evicted entry just re-walks once next call.
    /// - **GC safety:** sweep removes entries whose String key is
    ///   tombstoned. without this, the cache would leak SymIds
    ///   indefinitely (small leak — SymIds are 4 bytes — but
    ///   unbounded).
    /// - **mutation safety:** if `formSlotSet` rewrites a String's
    ///   `:bytes`, we evict the intern cache entry alongside the
    ///   string_cache entry. Strings becoming a different identifier
    ///   would otherwise lie to callers.
    intern_cache: std.AutoHashMapUnmanaged(FormId, SymId) = .{},

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

    /// **adaptive GC trigger (post-§5.8c perf).** without a threshold,
    /// `runTop` collects on every outermost exit. for a bootstrap with
    /// 27 transporter loads that's 27 cycles, each walking a mostly-
    /// live heap (~99% live per spec §3.5). most cycles are wasted
    /// work — the heap hasn't grown enough to be worth a full walk.
    ///
    /// fix: only collect when the heap has grown by `gc_threshold_min`
    /// allocations AND by `gc_threshold_pct` percent since the last
    /// collection. AND-semantics: pct alone fires every cycle on a
    /// small heap; min alone defers a big heap too long. their
    /// intersection halves the cycle count without leaving large
    /// garbage uncollected.
    ///
    /// tunable via `MOOF_GC_THRESHOLD_MIN` / `MOOF_GC_THRESHOLD_PCT`
    /// env vars (read in main.zig).
    ///
    /// `last_gc_heap_size` is the value of `heap.forms.items.len` at
    /// the end of the most recent collection cycle (after sweep —
    /// unchanged because we don't compact, but the metric is "how
    /// big the heap was when we last decided to GC"). zero means
    /// "no collection has run yet"; only the min floor gates that
    /// first cycle.
    last_gc_heap_size: usize = 0,
    /// trigger threshold — percent growth since last GC. 50 means
    /// "collect once heap is 1.5× its post-last-GC size."
    gc_threshold_pct: u32 = 50,
    /// trigger threshold — absolute alloc count since last GC. 50_000
    /// means "collect once at least 50k new forms exist." this is
    /// the floor: small absolute growth is never worth the walk.
    gc_threshold_min: usize = 50_000,

    /// when true, vm.zig / intrinsics.zig surface diagnostic messages
    /// (UnboundName, UnhandledDnu, prepareInvoke arity mismatch dumps,
    /// evalStringInWorld parse-stage prints, etc.). default false —
    /// these are slow unbuffered fprintln calls; spec 2026-05-16-phase2
    /// §4.9 removed them from hot paths. flipped by `MOOF_TRACE=1` in
    /// main.zig.
    trace_enabled: bool = false,

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
    /// `'bytes` — slot on String-Forms holding the cons-chain of
    /// Char codepoints. cached so `formSlotSet` can fast-check
    /// "is this a String mutation?" without a re-intern.
    symBytes: SymId,

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
        const bytes_sym = try syms.intern("bytes");

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

        // V1 — turn_watermark advances past every Form already
        // allocated by bootstrap (protos, here_form, macros_form,
        // their proto/meta wiring). post-init: in_turn = false,
        // nursery_deltas empty, watermark = heap.len(). first
        // user-driven turn sees the entire bootstrap heap as
        // canonical pre-existing state.
        //
        // NB: master uses `var world` (not `return`) because we
        // register the Cons Layout against the fresh World below.
        const watermark: u32 = @intCast(heap.forms.items.len);

        var world = World{
            .heap = heap,
            .syms = syms,
            .protos = protos,
            .allocator = allocator,
            .vat_mode = .mutable_default,
            .chunk_bytecode = .empty,
            .chunk_consts = .empty,
            .chunk_ics = .empty,
            .chunk_params = .empty,
            .native_fns = .empty,
            .far_ref_table = .empty,
            .proto_generation = .empty,
            .proto_layouts = .{},
            .layout_arena = std.heap.ArenaAllocator.init(allocator),
            .tagged_storage = .empty,
            .nursery_deltas = .empty,
            .turn_watermark = watermark,
            .in_turn = false,
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
            .symBytes = bytes_sym,
        };
        // §5.8d — register the Cons layout (car, cdr). lives in the
        // layout_arena; pointer stable for the World's lifetime.
        // FlatCons fast-path remains in place; this just ensures the
        // Layout-aware dispatch can find Cons too.
        _ = try world.registerLayout(protos.cons, &.{ car_sym, cdr_sym });
        return world;
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
        const bytes_sym = try syms.intern("bytes");

        // every proto FormId starts at NONE; image's header populates.
        // NB: Cons layout is registered post-load by `loadVatImage`,
        // since here the proto FormIds are all NONE — the header
        // populates them later.
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

        // V1 — bare world: heap.len() is just the sentinel (1).
        // image-load will append forms; after load, callers should
        // update turn_watermark to heap.len() if they want loaded
        // forms treated as canonical for the first turn.
        // initBareForImage handles that; raw initBare leaves the
        // watermark at 1 so even the sentinel sits in canonical-land.
        const watermark: u32 = @intCast(heap.forms.items.len);

        return World{
            .heap = heap,
            .syms = syms,
            .protos = none_protos,
            .allocator = allocator,
            .vat_mode = .mutable_default,
            .chunk_bytecode = .empty,
            .chunk_consts = .empty,
            .chunk_ics = .empty,
            .chunk_params = .empty,
            .native_fns = .empty,
            .far_ref_table = .empty,
            .proto_generation = .empty,
            .proto_layouts = .{},
            .layout_arena = std.heap.ArenaAllocator.init(allocator),
            .tagged_storage = .empty,
            .nursery_deltas = .empty,
            .turn_watermark = watermark,
            .in_turn = false,
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
            .symBytes = bytes_sym,
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
        self.proto_layouts.deinit(self.allocator);
        // §5.8d — frees every Layout + its slot_names slice in one go.
        self.layout_arena.deinit();
        self.tagged_storage.deinit(self.allocator);

        // §5.8a — free the cached `[]u32` slices, then the map itself.
        // §5.8c — intern_cache values are scalars; just deinit the map.
        {
            var it = self.string_cache.iterator();
            while (it.next()) |entry| {
                self.allocator.free(entry.value_ptr.*);
            }
            self.string_cache.deinit(self.allocator);
        }
        self.intern_cache.deinit(self.allocator);

        // V1 — release per-form Delta storage. each Delta owns
        // three FaceMaps (slots / handlers / meta). callers are
        // expected to commit / abort cleanly before deinit, but
        // we walk defensively in case a panic interrupted a turn.
        {
            var it = self.nursery_deltas.iterator();
            while (it.next()) |entry| entry.value_ptr.deinit(self.allocator);
            self.nursery_deltas.deinit(self.allocator);
        }

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
            vm_mod.PROFILE.load_name_walk_hops += 1;
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
        vm_mod.PROFILE.env_bind_calls += 1;
        const fm = self.heap.getMut(env);
        try fm.slots.put(self.allocator, name, val);
    }

    // ---- proto / form access -------------------------------------

    /// the proto Value for any Value (tagged-immediate or Form).
    /// tagged immediates resolve to their canonical proto-Form per V0.
    pub fn protoOf(self: *const World, v: Value) Value {
        vm_mod.PROFILE.proto_of_calls += 1;
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

    /// pack a Value into a 64-bit key for `tagged_storage`. distinct
    /// tagged immediates hash to distinct keys; Forms aren't keyed
    /// here (their FormId is the canonical identity).
    fn valueKey(v: Value) u64 {
        return switch (v) {
            .nil => 0,
            .bool_ => |b| if (b) 1 else 2,
            .int => |n| (@as(u64, 3) << 56) | (@as(u64, @bitCast(n)) & 0x00ff_ffff_ffff_ffff),
            .sym => |s| (@as(u64, 4) << 56) | @as(u64, s),
            .char => |c| (@as(u64, 5) << 56) | @as(u64, c),
            .float => |f| (@as(u64, 6) << 56) | (@as(u64, @bitCast(f)) >> 8),
            .form => |id| (@as(u64, 7) << 56) | @as(u64, @as(u32, @bitCast(id))),
        };
    }

    /// the effective FormId for any Value, where defined. for Forms
    /// returns the FormId; for tagged immediates with a cached
    /// singleton-Form (allocated via `ensureWritableFormId`),
    /// returns that singleton's id; otherwise `null`.
    pub fn effectiveFormId(self: *const World, v: Value) ?FormId {
        return switch (v) {
            .form => |id| id,
            else => self.tagged_storage.get(valueKey(v)),
        };
    }

    /// ensure `v` has a writable Form identity. for Forms returns
    /// the FormId. for tagged immediates allocates a singleton-Form
    /// on first call (proto = the value's natural proto, e.g.
    /// `Bool` for `#true`) and caches it in `tagged_storage`. used
    /// by `setHandler!` / `slotSet!` / `metaSet!` so user code
    /// can pin per-instance handlers on `#true` etc.
    pub fn ensureWritableFormId(self: *World, v: Value) !FormId {
        if (v.asFormId()) |id| return id;
        const key = valueKey(v);
        if (self.tagged_storage.get(key)) |id| return id;
        // allocate a fresh singleton-Form whose proto is v's natural proto.
        const proto_v = self.protoOf(v);
        var f = Form.withProto(proto_v);
        const singleton_meta = try self.syms.intern("singleton-of");
        try f.meta.put(self.allocator, singleton_meta, v);
        const id = try self.heap.alloc(f);
        try self.tagged_storage.put(self.allocator, key, id);
        return id;
    }

    /// read `slot_name` on `id`, walking only the Form's own slots
    /// (no proto-chain). returns nil if absent.
    ///
    /// V1 nursery-aware: when `in_turn` is set AND `id` is a
    /// pre-existing form (payload < watermark), checks the
    /// `nursery_deltas` entry first. otherwise (new-alloc form
    /// or no active turn) reads canonical directly. matches rust
    /// `World::form_slot`.
    pub fn formSlot(self: *const World, id: FormId, slot_name: SymId) Value {
        vm_mod.PROFILE.form_slot_lookups += 1;
        // V1 nursery-aware: turn-buffered writes shadow canonical.
        // delta only ever holds explicit overrides; a miss here
        // falls through to the canonical Form's `slot`, which
        // honors Layout / FlatCons fast paths on the Form itself.
        if (self.in_turn and id.payload < self.turn_watermark) {
            if (self.nursery_deltas.get(id)) |delta| {
                if (delta.slots.get(slot_name)) |v| return v;
            }
        }
        const f = self.heap.get(id);
        return f.slot(slot_name);
    }

    /// write `val` to slot `slot_name` on `id`. errors on frozen or
    /// out-of-memory.
    ///
    /// V1 nursery-aware: when `in_turn` is set AND `id` is a
    /// pre-existing form (payload < watermark), buffers the write
    /// in `nursery_deltas` (a plain SlotMap face) and returns —
    /// the Layout / FlatCons fast paths and cache invalidation
    /// defer until `commitTurn` replays the delta against
    /// canonical. otherwise (new-alloc this turn, or no turn at
    /// all) writes canonical now, honoring all fast paths. matches
    /// rust `World::form_slot_set` minus the rust-side mutation-
    /// outside-turn panic (zig allows direct writes at boot, where
    /// intrinsics still write via `heap.getMut(...)` for proto
    /// wiring).
    ///
    /// **§5.8a/c invalidation:** if `slot_name == :bytes` and we
    /// hit the canonical path, evict any string_cache / intern_cache
    /// entry for `id`. cheap (two hashmap removes; both no-op if
    /// not present). nursery-buffered writes don't invalidate
    /// caches inline — `commitTurn` re-routes through this same
    /// function (via the canonical path) at commit time.
    ///
    /// **§5.8d Layout:** if the Form has a layout and `slot_name`
    /// matches one of its canonical slots, write the inline storage
    /// directly (skip the SlotMap). otherwise fall through.
    ///
    /// **§5.8b FlatCons (legacy):** if `id` is a flat-cons Form and the
    /// slot name is `:car` / `:cdr`, write the inline field directly.
    /// other slot names fall through to the SlotMap path.
    pub fn formSlotSet(self: *World, id: FormId, slot_name: SymId, val: Value) !void {
        const fm = self.heap.getMut(id);
        if (fm.frozen) return error.FrozenForm;
        if (self.in_turn and id.payload < self.turn_watermark) {
            // pre-existing form during an active turn — buffer the
            // write into the per-form Delta. commitTurn replays
            // these via the canonical path below (which honors
            // Layout / FlatCons / cache invalidation).
            const gop = try self.nursery_deltas.getOrPut(self.allocator, id);
            if (!gop.found_existing) gop.value_ptr.* = .{};
            try gop.value_ptr.slots.put(self.allocator, slot_name, val);
            return;
        }
        // new-alloc within turn, or no turn — write canonical.
        if (fm.layoutTrySet(slot_name, val)) return;
        try fm.slots.put(self.allocator, slot_name, val);
        if (slot_name == self.symBytes) self.invalidateStringCaches(id);
    }

    /// allocate a fresh Cons cell using the registered Layout.
    /// inline-fields only, no SlotMap traffic. callers: `consConsInto`,
    /// `consReverse`, `globalCons`, `World.makeList`.
    ///
    /// post-§5.8d: a thin wrapper around `Form.withLayout(cons_proto,
    /// cons_layout)` with `inline_slots[0..1]` pre-populated. on-disk
    /// image format treats this as a Form-with-(car,cdr)-slots
    /// synthesized at serialize time; the loader's `reflatLoadedLayouts`
    /// re-hoists them on read.
    /// TODO(phase2): this path does NOT consult `vat_mode` for auto-freeze.
    /// Cons cells allocated here in a `frozen_default` vat are born MUTABLE,
    /// which diverges from spec §4.1's "every alloc-expression result is born
    /// frozen" semantics. impact is currently zero because `vat_mode` defaults
    /// to `.mutable_default` and no code sets `.frozen_default` yet. when
    /// `frozen_default` vats land in phase 2+, add the same auto-freeze hook
    /// as `allocInstance` (or factor a shared `maybeFreeze(id)` helper).
    /// the same gap exists in `heapAllocFormWithProto` (intrinsics.zig).
    pub fn allocFlatCons(self: *World, car: Value, cdr: Value) !FormId {
        const cons_proto_v = Value{ .form = self.protos.cons };
        const lay = self.proto_layouts.get(self.protos.cons) orelse {
            // safety net for image-load paths where the Cons layout
            // hasn't been registered yet (init order / partial image).
            // fall back to a general Form-with-cons-proto + slots.
            var general = Form.withProto(cons_proto_v);
            try general.slots.put(self.allocator, self.symCar, car);
            try general.slots.put(self.allocator, self.symCdr, cdr);
            return self.heap.alloc(general);
        };
        var f = Form.withLayout(cons_proto_v, lay);
        f.inline_slots[0] = car;
        f.inline_slots[1] = cdr;
        return self.heap.alloc(f);
    }

    /// **§5.8d — generic layout-aware allocation.** if the proto has
    /// a registered Layout, allocate a Form whose inline_slots are
    /// the canonical schema. otherwise, allocate a general Form.
    ///
    /// respects vat_mode: when frozen_default, the returned Form is
    /// immediately frozen (per design spec §4.1). use `allocMutableBypass`
    /// when you need to build-then-seal within a frozen-default vat.
    ///
    /// callers: `Object:new` and any constructor that wants to honor
    /// the proto's declared shape. caller may populate inline slots
    /// before passing to alloc via `f.layoutTrySet`.
    pub fn allocInstance(self: *World, proto_id: FormId) !FormId {
        const proto_v = Value{ .form = proto_id };
        const id = if (self.proto_layouts.get(proto_id)) |lay|
            try self.heap.alloc(Form.withLayout(proto_v, lay))
        else
            try self.heap.alloc(Form.withProto(proto_v));
        // vat-mode auto-freeze: spec §4.1. applied post-alloc so the
        // form's initial state is mutable during construction (inside
        // allocInstance callers). freezes on return to moof code.
        if (self.vat_mode == .frozen_default) {
            self.heap.getMut(id).frozen = true;
        }
        return id;
    }

    /// allocate a fresh mutable Form regardless of vat_mode.
    ///
    /// used by the `let-mutable` macro's `__alloc-mutable__` intrinsic
    /// to bypass auto-freeze for build-then-seal idioms in frozen-default
    /// vats. the caller is responsible for explicitly freezing the form
    /// at scope exit (let-mutable does this automatically).
    pub fn allocMutableBypass(self: *World, proto_id: FormId) !FormId {
        const proto_v = Value{ .form = proto_id };
        if (self.proto_layouts.get(proto_id)) |lay| {
            return self.heap.alloc(Form.withLayout(proto_v, lay));
        }
        return self.heap.alloc(Form.withProto(proto_v));
    }

    /// per spec §4.5: returns false for already-frozen forms OR for
    /// "live face" forms that cannot be frozen. in V0, the only live
    /// face category is ForeignHandle (the wasm mco ABI handle). V4
    /// adds vat-Forms, mailboxes, cap-tokens; those land later.
    ///
    /// **semantic of `freezable?`**: this is a "will calling freeze do
    /// useful work without raising" predicate, not a pure category check.
    /// already-frozen → false (no useful work — freeze is a no-op).
    /// live-face → false (raises 'cannot-freeze-live).
    /// fresh mutable → true.
    /// rationale: most user code asks `freezable?` to decide "should i
    /// call freeze here?" — the current-state interpretation is more
    /// useful than a pure-category one. category-only callers can check
    /// `not [form is-live-face?]` (future API) instead.
    ///
    /// note: `Object:freeze` also uses this to detect live-face forms.
    /// `Object:freeze` on an already-frozen form is a silent no-op
    /// (idempotent); `Object:freeze` on a live-face raises
    /// `'cannot-freeze-live`. the distinction is carried by the
    /// caller: `objFreeze` checks `fm.frozen` first, then `isFreezable`.
    pub fn isFreezable(self: *const World, id: FormId) bool {
        const fm = self.heap.get(id);
        if (fm.frozen) return false;
        // V0 live face: ForeignHandle. identified by proto == protos.foreign_handle.
        if (fm.proto.asFormId()) |proto_id| {
            if (proto_id.eql(self.protos.foreign_handle)) return false;
        }
        return true;
    }

    /// evict cached `[]u32` materialization and intern result for `id`.
    /// callable from any String-mutating native; also called by
    /// `formSlotSet` whenever the `:bytes` slot is written.
    pub fn invalidateStringCaches(self: *World, id: FormId) void {
        if (self.string_cache.fetchRemove(id)) |kv| {
            self.allocator.free(kv.value);
        }
        _ = self.intern_cache.remove(id);
    }

    /// **§5.8a — fetch (or lazily build) the cached `[]u32`
    /// codepoint slice for a String FormId.**
    ///
    /// fast path: cache hit returns the slice in one hashmap probe.
    /// slow path: walks `id.:bytes` once, allocates a `[]u32`,
    /// inserts into the cache, returns the slice. subsequent calls
    /// at this site become O(1).
    ///
    /// returns null if `id`'s `:bytes` chain is malformed (missing,
    /// non-Cons mid-chain, non-Char car). callers fall back to
    /// raising an error.
    pub fn getStringChars(self: *World, id: FormId) !?[]const u32 {
        if (self.string_cache.get(id)) |cached| return cached;
        // miss — walk the :bytes chain into a buffer.
        var buf: std.ArrayList(u32) = .empty;
        defer buf.deinit(self.allocator);
        var cur = self.formSlot(id, self.symBytes);
        while (true) {
            switch (cur) {
                .nil => break,
                .form => |cid| {
                    const cf = self.heap.get(cid);
                    if (!cf.slotPresent(self.symCar)) break;
                    const car_v = cf.slot(self.symCar);
                    const cdr_v = cf.slot(self.symCdr);
                    switch (car_v) {
                        .char => |cp| try buf.append(self.allocator, cp),
                        else => return null,
                    }
                    cur = cdr_v;
                },
                else => return null,
            }
        }
        const owned = try self.allocator.dupe(u32, buf.items);
        try self.string_cache.put(self.allocator, id, owned);
        return owned;
    }

    /// read `key` on `id.handlers`. returns `null` if absent —
    /// callers walking the proto chain rely on `null` to keep
    /// walking. V1 nursery-aware analogous to `formSlot`.
    pub fn formHandler(self: *const World, id: FormId, key: SymId) ?Value {
        if (self.in_turn and id.payload < self.turn_watermark) {
            if (self.nursery_deltas.get(id)) |delta| {
                if (delta.handlers.get(key)) |v| return v;
            }
        }
        const f = self.heap.get(id);
        return f.handler(key);
    }

    /// write `val` to handler `key` on `id`. V1 nursery-aware.
    /// like `formSlotSet`: pre-existing forms during a turn
    /// buffer in the delta; new-allocs / no-turn writes go
    /// straight to canonical.
    pub fn formHandlerSet(self: *World, id: FormId, key: SymId, val: Value) !void {
        const fm = self.heap.getMut(id);
        if (fm.frozen) return error.FrozenForm;
        if (self.in_turn and id.payload < self.turn_watermark) {
            const gop = try self.nursery_deltas.getOrPut(self.allocator, id);
            if (!gop.found_existing) gop.value_ptr.* = .{};
            try gop.value_ptr.handlers.put(self.allocator, key, val);
            return;
        }
        try fm.handlers.put(self.allocator, key, val);
    }

    /// read `key` on `id.meta`. nil if absent. V1 nursery-aware.
    pub fn formMeta(self: *const World, id: FormId, key: SymId) Value {
        if (self.in_turn and id.payload < self.turn_watermark) {
            if (self.nursery_deltas.get(id)) |delta| {
                if (delta.meta.get(key)) |v| return v;
            }
        }
        const f = self.heap.get(id);
        return f.metaAt(key);
    }

    /// write `val` to meta `key` on `id`. V1 nursery-aware.
    pub fn formMetaSet(self: *World, id: FormId, key: SymId, val: Value) !void {
        const fm = self.heap.getMut(id);
        if (fm.frozen) return error.FrozenForm;
        if (self.in_turn and id.payload < self.turn_watermark) {
            const gop = try self.nursery_deltas.getOrPut(self.allocator, id);
            if (!gop.found_existing) gop.value_ptr.* = .{};
            try gop.value_ptr.meta.put(self.allocator, key, val);
            return;
        }
        try fm.meta.put(self.allocator, key, val);
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

    // ---- §5.8d Layout registration ------------------------------

    /// register a Layout for `proto`. allocates a `Layout` (and a
    /// duped slot-names slice) out of `layout_arena`; the pointer is
    /// stable for the World's lifetime. callable from boot
    /// (initBare/init/loadVatImage) and later from `defproto` once
    /// user-defined layouts land.
    ///
    /// idempotent on identical schemas: a second call for the same
    /// proto with the same slot_names returns the existing Layout
    /// pointer. mismatched re-registration is rejected — the schema
    /// has to stay stable for instance L11.
    pub fn registerLayout(self: *World, proto: FormId, slot_names: []const SymId) !*const form.Layout {
        std.debug.assert(slot_names.len <= form.INLINE_CAPACITY);
        if (self.proto_layouts.get(proto)) |existing| {
            // require schema match (defensive — registrations should
            // happen once per proto).
            if (existing.inline_size != slot_names.len) return error.LayoutMismatch;
            for (slot_names, 0..) |s, i| {
                if (existing.slot_names[i] != s) return error.LayoutMismatch;
            }
            return existing;
        }
        const arena = self.layout_arena.allocator();
        const names_copy = try arena.dupe(SymId, slot_names);
        const lay_ptr = try arena.create(form.Layout);
        lay_ptr.* = .{
            .slot_names = names_copy,
            .inline_size = @intCast(slot_names.len),
        };
        try self.proto_layouts.put(self.allocator, proto, lay_ptr);
        return lay_ptr;
    }

    /// look up the Layout registered for `proto`. `null` if the proto
    /// is general (no inline schema). callers (Object:new, constructors,
    /// image-load post-pass) consult this to decide allocation shape.
    pub fn layoutForProto(self: *const World, proto: FormId) ?*const form.Layout {
        return self.proto_layouts.get(proto);
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

    // ---- V1 turn lifecycle ---------------------------------------
    //
    // mirrors `players/rust/src/world.rs::{start_turn,
    // commit_turn, abort_turn, in_turn}`. zig deviates from rust in
    // one place: write-outside-turn does NOT panic — boot-time
    // intrinsics still poke `heap.getMut(...)` directly, and we
    // don't want to force a migration audit in this PR. read-paths
    // and write-paths through `formSlot*` are nursery-aware
    // when `in_turn` is true, and degrade to direct r/w otherwise.

    /// `true` iff a turn is currently active.
    pub fn inTurn(self: *const World) bool {
        return self.in_turn;
    }

    /// begin a turn. panics if a turn is already active —
    /// V1 supports exactly one active turn at a time. matches
    /// rust `World::start_turn`.
    ///
    /// records `turn_watermark = heap.forms.items.len`. all
    /// allocations after this point have payload >= watermark
    /// and are "new" — mutations to them write canonically.
    /// mutations to pre-existing forms buffer in
    /// `nursery_deltas`.
    pub fn startTurn(self: *World) void {
        if (self.in_turn) std.debug.panic("startTurn called while a turn is already active", .{});
        std.debug.assert(self.nursery_deltas.count() == 0);
        self.in_turn = true;
        self.turn_watermark = @intCast(self.heap.forms.items.len);
    }

    /// commit the active turn. computes and returns a
    /// `TurnDiff`; applies nursery deltas to canonical heap;
    /// advances `turn_watermark` to the post-turn high-water;
    /// clears `nursery_deltas`; flips `in_turn` off. panics
    /// if no turn is active.
    ///
    /// caller owns the returned TurnDiff (zig has no Drop —
    /// call `diff.deinit(world.allocator)`). zig substrate
    /// doesn't yet journal the diff (V9 persistence work);
    /// callers that don't need it can `defer diff.deinit(...)`
    /// immediately.
    pub fn commitTurn(self: *World) !TurnDiff {
        if (!self.in_turn) std.debug.panic("commitTurn called outside a turn", .{});

        var diff: TurnDiff = .{};
        errdefer diff.deinit(self.allocator);

        // process deltas: read canonical prior, emit diff
        // entry, apply mutation. order is the nursery_deltas
        // insertion order (D5 — AutoArrayHashMap preserves it).
        // we drain by iterating, then clearing+freeing at the
        // end (we can't shrink the map during iteration).
        var it = self.nursery_deltas.iterator();
        while (it.next()) |entry| {
            const form_id = entry.key_ptr.*;
            const delta = entry.value_ptr;
            const canonical = self.heap.getMut(form_id);

            // slots
            var sit = delta.slots.iterator();
            while (sit.next()) |sentry| {
                const key = sentry.key_ptr.*;
                const new_value = sentry.value_ptr.*;
                const prior: Value = if (canonical.slots.get(key)) |v| v else Value.nil;
                try diff.mutations.put(self.allocator, .{
                    .form_id = form_id,
                    .face = .slots,
                    .key = key,
                }, .{ .prior = prior, .new = new_value });
                try canonical.slots.put(self.allocator, key, new_value);
            }
            // handlers
            var hit = delta.handlers.iterator();
            while (hit.next()) |hentry| {
                const key = hentry.key_ptr.*;
                const new_value = hentry.value_ptr.*;
                const prior: Value = if (canonical.handlers.get(key)) |v| v else Value.nil;
                try diff.mutations.put(self.allocator, .{
                    .form_id = form_id,
                    .face = .handlers,
                    .key = key,
                }, .{ .prior = prior, .new = new_value });
                try canonical.handlers.put(self.allocator, key, new_value);
            }
            // meta
            var mit = delta.meta.iterator();
            while (mit.next()) |mentry| {
                const key = mentry.key_ptr.*;
                const new_value = mentry.value_ptr.*;
                const prior: Value = if (canonical.meta.get(key)) |v| v else Value.nil;
                try diff.mutations.put(self.allocator, .{
                    .form_id = form_id,
                    .face = .meta,
                    .key = key,
                }, .{ .prior = prior, .new = new_value });
                try canonical.meta.put(self.allocator, key, new_value);
            }

            // V2 — frozen-bit transition. only emit a freezings
            // entry for pre-existing forms (below the pre-commit
            // watermark, which we still hold). zig doesn't yet
            // expose freeze() but we honor the field for parity.
            if (delta.frozen and !canonical.frozen) {
                canonical.frozen = true;
                if (form_id.payload < self.turn_watermark) {
                    try diff.freezings.append(self.allocator, form_id);
                }
            }
        }

        // drain the deltas: deinit each Delta's owned FaceMaps,
        // then clear the outer map.
        var dit = self.nursery_deltas.iterator();
        while (dit.next()) |entry| entry.value_ptr.deinit(self.allocator);
        self.nursery_deltas.clearRetainingCapacity();

        // collect new-alloc FormIds (allocations during this
        // turn sit at `heap.forms[turn_watermark..]`).
        const new_high: u32 = @intCast(self.heap.forms.items.len);
        var p: u32 = self.turn_watermark;
        while (p < new_high) : (p += 1) {
            try diff.new_allocs.append(self.allocator, FormId.vatLocal(@intCast(p)));
        }

        // advance watermark to include this turn's allocs.
        self.turn_watermark = new_high;
        self.in_turn = false;

        return diff;
    }

    /// abort the active turn. truncates `heap.forms` to
    /// `turn_watermark` (drops this-turn allocations). clears
    /// `nursery_deltas` (drops buffered mutations). flips
    /// `in_turn` off. watermark unchanged. panics if no turn
    /// is active.
    ///
    /// NOTE: truncating `heap.forms` is the rollback for
    /// allocations — newly allocated Forms vanish with their
    /// slot/handler/meta backing storage. callers must NOT
    /// retain raw `*Form` pointers across `abortTurn`
    /// boundaries (they'd dangle); this matches the rust
    /// discipline.
    ///
    /// NB: `become_` redirects are NOT yet rolled back —
    /// zig substrate hasn't implemented `turn_redirect_originals`.
    /// `become_` happens through `Heap.become_` outside the
    /// nursery. tracked alongside V1 follow-ups in
    /// `players/rust/src/world.rs::turn_redirect_originals`.
    pub fn abortTurn(self: *World) void {
        if (!self.in_turn) std.debug.panic("abortTurn called outside a turn", .{});

        // drop new-alloc forms by truncating Forms vec to
        // watermark. before truncating, deinit each Form's
        // owned FaceMaps so we don't leak their storage.
        const watermark_usz: usize = @intCast(self.turn_watermark);
        var i: usize = watermark_usz;
        while (i < self.heap.forms.items.len) : (i += 1) {
            self.heap.forms.items[i].deinit(self.allocator);
        }
        self.heap.forms.shrinkRetainingCapacity(watermark_usz);

        // drop buffered mutations (no canonical writes occurred).
        var it = self.nursery_deltas.iterator();
        while (it.next()) |entry| entry.value_ptr.deinit(self.allocator);
        self.nursery_deltas.clearRetainingCapacity();

        self.in_turn = false;
    }

    // ---- handler lookup (proto-chain walk) -----------------------

    /// walk the proto chain starting AT `start_proto` looking for a
    /// handler for `selector`. used by lookupHandler and lookupHandlerSuper
    /// (with different starting points).
    fn walkChain(self: *const World, start: FormId, selector: SymId) ?HandlerHit {
        vm_mod.PROFILE.proto_chain_walks += 1;
        var cur = start;
        var hops: usize = 0;
        const MAX_HOPS: usize = 256;
        while (hops < MAX_HOPS) : (hops += 1) {
            vm_mod.PROFILE.proto_chain_hops += 1;
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
    /// mirrors rust players/rust/src/world.rs::lookup_handler.
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
        vm_mod.PROFILE.envs_allocated += 1;
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
        vm_mod.PROFILE.list_to_slice_calls += 1;
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
        vm_mod.PROFILE.list_to_slice_total_items += n;
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
    /// §5.8b — flat-cons fast path; no SlotMap traffic per cell.
    pub fn makeList(self: *World, values: []const Value) !Value {
        var acc: Value = .nil;
        var i: usize = values.len;
        while (i > 0) {
            i -= 1;
            const id = try self.allocFlatCons(values[i], acc);
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
    ///
    /// updates `last_gc_heap_size` after a successful cycle so the
    /// adaptive trigger (`collectIfNeeded`) can compare against it.
    pub fn collect(self: *World) !?GcStats {
        if (!self.gc_enabled) return null;
        const stats = try gc_mod.collect(self);
        if (self.gc_stats_enabled) gc_mod.printStats(stats);
        // record the heap-size watermark. mark-sweep is sparse — we
        // never compact — so `heap.forms.items.len` is unchanged by
        // sweep; this is just "what was the size when we last GC'd?"
        self.last_gc_heap_size = self.heap.forms.items.len;
        return stats;
    }

    /// **adaptive GC trigger.** call from `runTop` on outermost exit.
    /// only invokes `collect` when the heap has grown enough since
    /// the last cycle to justify the walk. returns the cycle's stats
    /// when one fires; `null` when the threshold wasn't met OR when
    /// `gc_enabled` is false.
    ///
    /// threshold: collect when growth since the last cycle exceeds
    /// BOTH
    ///   - `gc_threshold_min` absolute allocations (floor: don't walk
    ///     for trivial growth), AND
    ///   - `gc_threshold_pct` percent of the post-last-GC size (ceiling:
    ///     don't let a large heap quietly double before we collect).
    ///
    /// AND-semantics matches the comment "collect when heap grew …
    /// **but at least** N allocations since last." pct alone fires
    /// every cycle on a small heap; min alone defers a big heap past
    /// useful collection. their intersection halves the cycle count
    /// without leaving large garbage uncollected.
    ///
    /// special case: when `last_gc_heap_size == 0` (no cycle has
    /// fired yet) the pct floor is bypassed; only the min floor
    /// gates. this means the very first cycle waits for
    /// `gc_threshold_min` allocs — a fresh world doesn't pay for a
    /// GC walk on its first runTop exit.
    ///
    /// the bootstrap baseline ran ~23 cycles each averaging ~70 ms
    /// (walking ~99%-live heaps). with defaults of 50k / 50%, this
    /// drops to ~3-5 cycles total, saving ~80% of the GC wall time.
    pub fn collectIfNeeded(self: *World) !?GcStats {
        if (!self.gc_enabled) return null;
        const cur = self.heap.forms.items.len;
        const grew = cur -| self.last_gc_heap_size;
        if (grew < self.gc_threshold_min) return null;
        const pct_target: usize = (self.last_gc_heap_size *| @as(usize, self.gc_threshold_pct)) / 100;
        if (self.last_gc_heap_size > 0 and grew < pct_target) return null;
        return self.collect();
    }

    /// look up a named native in the process intrinsics table.
    /// image-load (image.zig::readNativeRefs) uses this to rebind
    /// natives on freshly-deserialized methods. backed by the
    /// comptime REGISTRY in intrinsics.zig — names match the rust
    /// v4_export's NativeRefsSection format ("ProtoName:selector").
    ///
    /// side-effect: caches the `Method:call` pointer on the World
    /// when it shows up, so the tail-send dispatcher can compare
    /// against it without re-resolving by name on the hot path.
    pub fn lookupNativeByName(self: *World, name: []const u8) ?NativeFn {
        // late import to avoid a top-level cycle (intrinsics imports
        // world). zig comptime @import returns a struct; this works
        // because we only access REGISTRY at call time.
        const intrinsics = @import("intrinsics.zig");
        const fp = intrinsics.REGISTRY.get(name) orelse return null;
        if (std.mem.eql(u8, name, "Method:call")) {
            self.method_call_native = fp;
        }
        return fp;
    }
};

// ─────────────────────────────────────────────────────────────────
// §5.8b FlatCons contract tests at the World level: formSlot /
// formSlotSet / allocFlatCons preserve every Form-face the spec
// requires.
// ─────────────────────────────────────────────────────────────────

const testing = std.testing;

test "World.allocFlatCons: car/cdr accessible via formSlot" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const cell = try world.allocFlatCons(Value{ .int = 1 }, Value{ .int = 2 });
    try testing.expect(world.formSlot(cell, world.symCar).equals(Value{ .int = 1 }));
    try testing.expect(world.formSlot(cell, world.symCdr).equals(Value{ .int = 2 }));
    // proto chain points at the Cons proto.
    const proto_v = world.protoOf(.{ .form = cell });
    try testing.expect(proto_v.equals(.{ .form = world.protos.cons }));
}

test "World.formSlotSet: :car / :cdr write inline; extras lazy-init" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const cell = try world.allocFlatCons(Value{ .int = 1 }, Value{ .int = 2 });
    // overwrite car inline.
    try world.formSlotSet(cell, world.symCar, Value{ .int = 99 });
    try testing.expect(world.formSlot(cell, world.symCar).equals(Value{ .int = 99 }));
    // an extras slot goes to SlotMap; canonical slots still read inline.
    const foo_sym = try world.syms.intern("foo");
    try world.formSlotSet(cell, foo_sym, Value{ .int = 42 });
    try testing.expect(world.formSlot(cell, foo_sym).equals(Value{ .int = 42 }));
    try testing.expect(world.formSlot(cell, world.symCar).equals(Value{ .int = 99 }));
    // SlotMap holds only the extra (not car/cdr).
    const fm = world.heap.get(cell);
    try testing.expectEqual(@as(usize, 1), fm.slots.count());
    try testing.expect(fm.slots.contains(foo_sym));
    try testing.expect(!fm.slots.contains(world.symCar));
}

test "World.formSlotSet: frozen FlatCons raises" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const cell = try world.allocFlatCons(Value{ .int = 1 }, Value{ .int = 2 });
    world.heap.getMut(cell).frozen = true;
    const got = world.formSlotSet(cell, world.symCar, Value{ .int = 99 });
    try testing.expectError(error.FrozenForm, got);
    // cell unchanged.
    try testing.expect(world.formSlot(cell, world.symCar).equals(Value{ .int = 1 }));
}

test "World.become_: FlatCons can be the target of a redirect" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const a = try world.allocFlatCons(Value{ .int = 1 }, Value.nil);
    const b = try world.allocFlatCons(Value{ .int = 7 }, Value.nil);
    try world.become_(a, b);
    // reads via `a` see b's content.
    try testing.expect(world.formSlot(a, world.symCar).equals(Value{ .int = 7 }));
}

// ─────────────────────────────────────────────────────────────────
// §5.8d Layout registration tests.
// ─────────────────────────────────────────────────────────────────

test "Layout: World.init pre-registers Cons layout" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const lay = world.layoutForProto(world.protos.cons) orelse return error.NoLayout;
    try testing.expectEqual(@as(u8, 2), lay.inline_size);
    try testing.expectEqual(world.symCar, lay.slot_names[0]);
    try testing.expectEqual(world.symCdr, lay.slot_names[1]);
}

test "Layout: registerLayout idempotent on identical schema" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const lay1 = try world.registerLayout(world.protos.cons, &.{ world.symCar, world.symCdr });
    const lay2 = world.layoutForProto(world.protos.cons).?;
    try testing.expect(lay1 == lay2);
}

test "Layout: registerLayout rejects schema mismatch" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    // Cons already has (car, cdr); try registering (cdr, car) — mismatch.
    const got = world.registerLayout(world.protos.cons, &.{ world.symCdr, world.symCar });
    try testing.expectError(error.LayoutMismatch, got);
}

test "Layout: register Counter-like proto, alloc + read inline" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    // make a fresh proto.
    const counter_proto = try world.heap.alloc(form.Form.withProto(.{ .form = world.protos.object }));
    const count_sym = try world.syms.intern("count");
    _ = try world.registerLayout(counter_proto, &.{count_sym});
    // allocate an instance via Form.withLayout directly (Object:new wiring is step 4).
    const lay = world.layoutForProto(counter_proto).?;
    var f = form.Form.withLayout(.{ .form = counter_proto }, lay);
    f.inline_slots[0] = Value{ .int = 7 };
    const id = try world.heap.alloc(f);
    try testing.expect(world.formSlot(id, count_sym).equals(Value{ .int = 7 }));
}

test "makeList: builds FlatCons cells" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const vals = [_]Value{ .{ .int = 10 }, .{ .int = 20 }, .{ .int = 30 } };
    const list = try world.makeList(&vals);
    // first cell carries the Cons Layout (post-§5.8d).
    const head_id = list.asFormId().?;
    try testing.expect(world.heap.get(head_id).layout != null);
    // car / cdr work
    try testing.expect(world.formSlot(head_id, world.symCar).equals(Value{ .int = 10 }));
    // listToSlice round-trips
    const slice = try world.listToSlice(list);
    defer world.freeSlice(slice);
    try testing.expectEqual(@as(usize, 3), slice.len);
    try testing.expect(slice[0].equals(Value{ .int = 10 }));
    try testing.expect(slice[1].equals(Value{ .int = 20 }));
    try testing.expect(slice[2].equals(Value{ .int = 30 }));
}
