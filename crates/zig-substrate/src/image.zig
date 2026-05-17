//! per-vat image deserializer for the V4 substrate.
//!
//! reads a sealed `.vat` binary blob (V4 spec §10.3) into a fresh
//! `World`, mutating it in place. one parser handles both regular
//! vat-images and the special `manifest.vat` — there is no second
//! format. see V4 spec §10 for the full design rationale.
//!
//! the loader is byte-shuffling only — no compilation. for the
//! stdlib-system vat this replaces the ~1300ms re-compile with a
//! ~10ms cold boot (per spec §10.3 step 10).
//!
//! ## sections in disk order (per §10.3)
//!   1. Magic(4) "MVAT"
//!   2. Version(u16 BE)  — currently 0x0004
//!   3. Header              — vat_id, counts, here/macros, protos, external vat refs
//!   4. SymTableSection
//!   5. FormSection         — alloc each Form; FormId payload = position
//!   6. ChunkSection        — body bytes, consts, ic slots, params
//!   7. NativeRefsSection   — rebind named natives onto methods
//!   8. McoBindingsSection  — load wasm from mcos/<hash>; instantiate (stubbed)
//!   9. FarRefsSection      — populate far_ref_table (lazy resolution)
//!  10. Footer(32 bytes)    — blake3 of everything above (skipped in V4 MVP)
//!
//! ## ambiguities flagged (TODO into spec)
//!   - the spec §4 documents OPCODE tag bytes but not Value tag bytes
//!     for in-image encoding. this loader uses 0xC0..0xC7 per the
//!     agent-task spec; add an explicit §4.5 "Value byte encoding"
//!     section to the spec to canonicalize.
//!   - footer hash verification is stubbed (TODO when crypto is wired).
//!
//! V4 spec: docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md §10

const std = @import("std");
const value = @import("value.zig");
const Value = value.Value;
const form = @import("form.zig");
const FormId = form.FormId;
const Form = form.Form;
const world_mod = @import("world.zig");
const World = world_mod.World;

/// `"MVAT"` — the four magic bytes at the head of every vat-image.
/// per spec §10.3.
pub const MAGIC: [4]u8 = .{ 'M', 'V', 'A', 'T' };

/// schema version (`0x0004` = V4). see spec §10.10.
pub const VERSION: u16 = 0x0004;

/// upper bound on `become` redirect-chain length when chasing FormIds.
/// not used by the loader itself but exposed here because image-load
/// is the canonical spot where this constant gets shared across the
/// substrate.
pub const MAX_BECOME_HOPS: usize = 32;

/// errors that can surface from `loadVatImage`. all are recoverable —
/// no panics in the loader; bad bytes are user data.
pub const ImageError = error{
    /// the first four bytes weren't `"MVAT"`.
    BadMagic,
    /// the version field doesn't match `VERSION`.
    UnsupportedVersion,
    /// recomputed blake3 differs from the declared footer (TODO V4 MVP).
    HashMismatch,
    /// the byte slice is shorter than the declared sections need.
    TruncatedImage,
    /// a NativeRefsSection entry names a native not in the process table.
    UnknownNative,
    /// a McoBindingsSection entry references an mco hash we don't have.
    UnknownMco,
    /// the byte-tagged Value tag isn't one of the recognized variants.
    BadValueTag,
};

// ---------------------------------------------------------------------------
// Value byte encoding (§4 ambiguity — documented above)
// ---------------------------------------------------------------------------

/// tag bytes for the in-image `Value` encoding. the canonical V4 spec
/// §4 talks about op tags; these are the Value tags used inside
/// FormSection/ChunkSection. flagged for promotion into the spec.
const VTAG_NIL: u8 = 0xC0;
const VTAG_BOOL_FALSE: u8 = 0xC1;
const VTAG_BOOL_TRUE: u8 = 0xC2;
const VTAG_INT: u8 = 0xC3;
const VTAG_SYM: u8 = 0xC4;
const VTAG_CHAR: u8 = 0xC5;
const VTAG_FLOAT: u8 = 0xC6;
const VTAG_FORM: u8 = 0xC7;

// ---------------------------------------------------------------------------
// Header types (mirrors spec §10.3)
// ---------------------------------------------------------------------------

/// 18 standard boot protos in canonical order (matches spec §10.3 and
/// `protos.zig::Protos`). all entries are FormId payloads (u32 BE on
/// the wire); scope is always `.vat_local`.
pub const ProtoTable = struct {
    object: u32,
    nil: u32,
    bool_: u32,
    integer: u32,
    char: u32,
    sym: u32,
    cons: u32,
    string: u32,
    bytes: u32,
    method: u32,
    chunk: u32,
    closure: u32,
    env: u32,
    foreign_handle: u32,
    table: u32,
    frame: u32,
    macros: u32,
    opcode: u32,
};

/// the per-vat header. `external_vat_refs` borrows from the source
/// byte slice (no copy) — fine because the slice outlives loading.
pub const Header = struct {
    vat_id: [16]u8,
    num_forms: u32,
    num_syms: u32,
    num_chunks: u32,
    here_form_id: u32,
    macros_form_id: u32,
    protos: ProtoTable,
    num_external_vat_refs: u16,
    external_vat_refs: []const [16]u8, // borrows from bytes
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// load a serialized vat-image into the given fresh `world`.
/// mutates `world` in place per V4 spec §10.3 step-list:
///   - allocates one Form per FormSection entry (FormId payload = index+1)
///   - interns each symbol in SymTable order
///   - populates chunk bytecode/consts/ics side-tables
///   - re-binds named natives onto method-form handler tables
///   - stubs mco instantiation (logged; real wasm wiring is separate)
///   - populates the far-ref table (resolution happens at first use)
///   - wires up `world.here_form`, `world.macros_form`, `world.protos.*`
///
/// the caller must pass a `world` whose heap/syms are empty (or at
/// least where the FormIds in the image don't collide). the loader
/// does not reset the world for you.
pub fn loadVatImage(world: *World, bytes: []const u8, allocator: std.mem.Allocator) !void {
    var pos: usize = 0;

    // 1. magic
    if (bytes.len < 4) return ImageError.TruncatedImage;
    if (!std.mem.eql(u8, bytes[0..4], &MAGIC)) return ImageError.BadMagic;
    pos = 4;

    // 2. version (big-endian u16)
    try requireBytes(bytes, pos, 2);
    const version = std.mem.readInt(u16, bytes[pos..][0..2], .big);
    pos += 2;
    if (version != VERSION) return ImageError.UnsupportedVersion;

    // 3. header
    const header = try readHeader(bytes, &pos);

    // 4. SymTableSection
    try readSymTable(world, bytes, &pos, header.num_syms);

    // 5. FormSection — alloc each Form in order; populate slots/handlers/meta
    try readForms(world, bytes, &pos, header.num_forms);

    // 6. ChunkSection — populate world.chunk_bytecode + chunk_consts + chunk_ics + chunk_params
    try readChunks(world, bytes, &pos, header.num_chunks, allocator);

    // 7. NativeRefsSection — look up by name in the process intrinsics table
    try readNativeRefs(world, bytes, &pos);

    // 8. McoBindingsSection — would load wasm; stubbed for V4 MVP
    try readMcoBindings(world, bytes, &pos);

    // 9. FarRefsSection — populate the far-ref table; lazy resolution at first use
    try readFarRefs(world, bytes, &pos);

    // 10. footer (32 bytes blake3). TODO: recompute + compare when crypto wired.
    if (bytes.len < 32 or pos > bytes.len - 32) return ImageError.TruncatedImage;
    const footer_pos = bytes.len - 32;
    _ = bytes[footer_pos..]; // declared_hash; ignored in V4 MVP
    // TODO(crypto): recompute blake3 over bytes[0..footer_pos];
    // if (!std.mem.eql(u8, &recomputed, declared_hash)) return ImageError.HashMismatch;

    // 11. wire up here_form, macros_form, and all 18 protos from the header.
    world.here_form = vatLocalId(header.here_form_id);
    world.macros_form = vatLocalId(header.macros_form_id);
    world.protos.object = vatLocalId(header.protos.object);
    world.protos.nil = vatLocalId(header.protos.nil);
    world.protos.bool_ = vatLocalId(header.protos.bool_);
    world.protos.integer = vatLocalId(header.protos.integer);
    world.protos.char = vatLocalId(header.protos.char);
    world.protos.sym = vatLocalId(header.protos.sym);
    world.protos.cons = vatLocalId(header.protos.cons);
    world.protos.string = vatLocalId(header.protos.string);
    world.protos.bytes = vatLocalId(header.protos.bytes);
    world.protos.method = vatLocalId(header.protos.method);
    world.protos.chunk = vatLocalId(header.protos.chunk);
    world.protos.closure = vatLocalId(header.protos.closure);
    world.protos.env = vatLocalId(header.protos.env);
    world.protos.foreign_handle = vatLocalId(header.protos.foreign_handle);
    world.protos.table = vatLocalId(header.protos.table);
    world.protos.frame = vatLocalId(header.protos.frame);
    world.protos.macros = vatLocalId(header.protos.macros);
    world.protos.opcode = vatLocalId(header.protos.opcode);

    // 12. re-cache the hot-path SymIds (gotcha #5 from NEXT_SESSION.md).
    //
    // initBare interned `parent`, `view-target`, etc. into the world's
    // sym table, then readSymTable's clearAndKeepCapacity wiped them and
    // re-interned the image's symbols. the SymIds we cached on the
    // World struct at init time are now stale — re-resolve each by
    // looking up its name in the freshly-loaded sym table.
    //
    // any name missing from the image is fine for V4 phase α (this only
    // happens when the rust v4_export never interned that symbol); we
    // leave the cached SymId at 0 (NONE), which env-walker treats as
    // "no parent slot", "no view-target slot" — i.e. the meta-key path
    // is effectively disabled rather than corrupted.
    world.parent_sym = lookupSym(world, "parent");
    world.view_target_sym = lookupSym(world, "view-target");
    world.dnu_sym = lookupSym(world, "does-not-understand:with:");
    world.body_sym = lookupSym(world, "body");
    world.env_sym = lookupSym(world, "env");
    world.params_sym = lookupSym(world, "params");
    world.symCar = lookupSym(world, "car");
    world.symCdr = lookupSym(world, "cdr");
    world.symBody = world.body_sym;
    world.symParent = world.parent_sym;
    world.symName = lookupSym(world, "name");
    world.self_sym = lookupSym(world, "self");
    // §5.8a — re-cache `:bytes` so World.getStringChars finds the
    // String content slot. initBare's interned-at-init SymId is
    // invalidated by clearAndKeepCapacity on the sym table.
    world.symBytes = lookupSym(world, "bytes");

    // §5.8b — install (car, cdr) SymIds for FlatCons accessors. must
    // happen AFTER the sym table is loaded; sets the form-module
    // globals that `Form.slot` / `Form.slotPresent` read.
    form.setConsSyms(world.symCar, world.symCdr);

    // §5.8d — register the Cons layout against the just-loaded
    // Cons proto FormId. happens BEFORE `reflatLoadedCons` so the
    // reflattened cells can carry the layout pointer. soft-skip if
    // (car, cdr) syms weren't in the image (degenerate test images).
    if (!world.protos.cons.isNone() and world.symCar != 0 and world.symCdr != 0) {
        _ = try world.registerLayout(world.protos.cons, &.{ world.symCar, world.symCdr });
    }

    // §5.8b — post-load re-flatten pass. on-disk format is unchanged
    // (a Cons cell serializes as a Form with :car / :cdr in slots),
    // so the loader must scan once after protos are wired to detect
    // cons-Forms and migrate their canonical slots into the inline
    // fields. all newly-allocated Cons cells (from `world.allocFlatCons`
    // / friends) are already flat — this pass only fires on entries
    // that came in via the image's FormSection.
    try reflatLoadedCons(world);
}

/// §5.8b — walk every loaded Form; for those with proto == cons_proto
/// and the canonical `:car` / `:cdr` slot bindings, move them into the
/// inline FlatCons fields and clear from the SlotMap. callers that
/// previously did `f.slots.put(car_sym, …)` see the slot via
/// `formSlot` (now flat-cons-aware), preserving the Form contract.
///
/// non-canonical extra slots on a cons-Form (rare but legal —
/// `[cell slotSet: 'foo to: 42]`) stay in the SlotMap. the post-pass
/// preserves them.
fn reflatLoadedCons(world: *World) !void {
    if (world.protos.cons.isNone()) return;
    const cons_id = world.protos.cons;
    const car_sym = world.symCar;
    const cdr_sym = world.symCdr;
    if (car_sym == 0 or cdr_sym == 0) return; // no sym table → skip

    // walk heap. skip sentinel (index 0). resolve each form's proto
    // Value and compare to cons proto FormId.
    var i: usize = 1;
    while (i < world.heap.len()) : (i += 1) {
        const fid = FormId.vatLocal(@intCast(i));
        const f_const = world.heap.get(fid);
        // skip tombstones and Forms whose proto isn't Cons.
        if (f_const.gc_tombstone) continue;
        if (f_const.is_flat_cons) continue; // shouldn't happen post-load, but defensive
        switch (f_const.proto) {
            .form => |pid| if (!pid.eql(cons_id)) continue,
            else => continue,
        }
        // hit. extract car/cdr.
        const fm = world.heap.getMut(fid);
        const car = if (fm.slots.get(car_sym)) |v| v else Value.nil;
        const cdr = if (fm.slots.get(cdr_sym)) |v| v else Value.nil;
        // remove from SlotMap.
        _ = fm.slots.swapRemove(car_sym);
        _ = fm.slots.swapRemove(cdr_sym);
        fm.car_inline = car;
        fm.cdr_inline = cdr;
        fm.is_flat_cons = true;
    }
}

/// linear-scan the sym table for `name`. zero (NONE) if not present.
/// used at the tail of `loadVatImage` to re-cache the hot-path SymIds
/// after `clearAndKeepCapacity` invalidated the ones cached at init.
fn lookupSym(world: *World, name: []const u8) u32 {
    const total = world.syms.len();
    var i: u32 = 1;
    while (i <= total) : (i += 1) {
        const text = world.syms.resolve(i);
        if (std.mem.eql(u8, text, name)) return i;
    }
    return 0; // NONE — name not interned in this image.
}

// ---------------------------------------------------------------------------
// Public serializer (W4 Piece 2)
// ---------------------------------------------------------------------------

/// serialize the given `world` as a V4 vat-image, appending bytes to
/// `out` (an ArrayList(u8)) using `allocator` for growth.
///
/// mirrors the byte layout produced by `crates/substrate/src/v4_export.rs`
/// exactly so a round-trip (rust→zig→rust→…) yields bit-identical bytes
/// modulo: insertion order of native methods (we walk proto handler
/// tables in heap order, matching rust's `collect_native_methods`), and
/// the footer hash is zeros in both implementations until phase 9.
///
/// the ArrayList+allocator interface mirrors `bytecode.encodeOp` —
/// zig 0.16's std.ArrayList lost its Managed `.writer()` shim, and the
/// project's convention is to thread the allocator through explicitly.
pub fn serializeVat(world: *const World, out: *std.ArrayList(u8), allocator: std.mem.Allocator) !void {
    // ── Magic + Version ────────────────────────────────────────
    try out.appendSlice(allocator, &MAGIC);
    try appendU16(out, allocator, VERSION);

    // ── Header ─────────────────────────────────────────────────
    // vat_id: 16 zero bytes (TODO: real ULID, matches rust stub).
    const vat_id: [16]u8 = .{0} ** 16;
    try out.appendSlice(allocator, &vat_id);

    // num_forms excludes the FormId(0) sentinel.
    const num_forms: u32 = @intCast(world.heap.len() - 1);
    const num_syms: u32 = @intCast(world.syms.len());
    const num_chunks: u32 = @intCast(world.chunk_bytecode.count());
    try appendU32(out, allocator, num_forms);
    try appendU32(out, allocator, num_syms);
    try appendU32(out, allocator, num_chunks);

    // here_form, macros_form
    try appendU32(out, allocator, @bitCast(world.here_form));
    try appendU32(out, allocator, @bitCast(world.macros_form));

    // 18 protos in canonical order — matches rust v4_export and
    // image.readHeader's ProtoTable layout.
    try appendU32(out, allocator, @bitCast(world.protos.object));
    try appendU32(out, allocator, @bitCast(world.protos.nil));
    try appendU32(out, allocator, @bitCast(world.protos.bool_));
    try appendU32(out, allocator, @bitCast(world.protos.integer));
    try appendU32(out, allocator, @bitCast(world.protos.char));
    try appendU32(out, allocator, @bitCast(world.protos.sym));
    try appendU32(out, allocator, @bitCast(world.protos.cons));
    try appendU32(out, allocator, @bitCast(world.protos.string));
    try appendU32(out, allocator, @bitCast(world.protos.bytes));
    try appendU32(out, allocator, @bitCast(world.protos.method));
    try appendU32(out, allocator, @bitCast(world.protos.chunk));
    try appendU32(out, allocator, @bitCast(world.protos.closure));
    try appendU32(out, allocator, @bitCast(world.protos.env));
    try appendU32(out, allocator, @bitCast(world.protos.foreign_handle));
    try appendU32(out, allocator, @bitCast(world.protos.table));
    try appendU32(out, allocator, @bitCast(world.protos.frame));
    try appendU32(out, allocator, @bitCast(world.protos.macros));
    try appendU32(out, allocator, @bitCast(world.protos.opcode));

    // external_vat_refs count = 0 (single-vat).
    try appendU16(out, allocator, 0);

    // ── SymTableSection ────────────────────────────────────────
    try appendU32(out, allocator, num_syms);
    var sym_i: u32 = 1;
    while (sym_i <= num_syms) : (sym_i += 1) {
        const text = world.syms.resolve(sym_i);
        try appendU16(out, allocator, @intCast(text.len));
        try out.appendSlice(allocator, text);
    }

    // ── FormSection ────────────────────────────────────────────
    //
    // §5.8b: FlatCons cells synthesize :car / :cdr in the slots
    // section when serializing so the on-disk image format stays
    // unchanged. the loader's `reflatLoadedCons` pass re-hoists them
    // into the inline fields after load. canonical car/cdr SymIds
    // are taken from `world.symCar` / `world.symCdr`, which are set
    // by World.init / loadVatImage's sym-cache refresh.
    try appendU32(out, allocator, num_forms);
    var i: usize = 1;
    while (i < world.heap.len()) : (i += 1) {
        const fid = FormId.vatLocal(@intCast(i));
        const f = world.heap.get(fid);
        // proto
        try appendValue(out, allocator, f.proto);
        // slots — observable slot count includes synthesized :car/:cdr
        // for FlatCons (yielded BEFORE the extras-map, matching the
        // canonical insertion order users see at allocation time).
        const slot_count: u16 = @intCast(f.slotCount());
        try appendU16(out, allocator, slot_count);
        if (f.is_flat_cons) {
            try appendU32(out, allocator, world.symCar);
            try appendValue(out, allocator, f.car_inline);
            try appendU32(out, allocator, world.symCdr);
            try appendValue(out, allocator, f.cdr_inline);
        }
        var slot_it = f.slots.iterator();
        while (slot_it.next()) |entry| {
            try appendU32(out, allocator, entry.key_ptr.*);
            try appendValue(out, allocator, entry.value_ptr.*);
        }
        // handlers
        try appendU16(out, allocator, @intCast(f.handlers.count()));
        var h_it = f.handlers.iterator();
        while (h_it.next()) |entry| {
            try appendU32(out, allocator, entry.key_ptr.*);
            try appendValue(out, allocator, entry.value_ptr.*);
        }
        // meta
        try appendU16(out, allocator, @intCast(f.meta.count()));
        var m_it = f.meta.iterator();
        while (m_it.next()) |entry| {
            try appendU32(out, allocator, entry.key_ptr.*);
            try appendValue(out, allocator, entry.value_ptr.*);
        }
        // frozen
        try out.append(allocator, if (f.frozen) @as(u8, 1) else @as(u8, 0));
    }

    // ── ChunkSection ───────────────────────────────────────────
    try appendU32(out, allocator, num_chunks);
    var ch_it = world.chunk_bytecode.iterator();
    while (ch_it.next()) |entry| {
        const chunk_id = entry.key_ptr.*;
        const body = entry.value_ptr.*;
        try appendU32(out, allocator, @bitCast(chunk_id));
        try appendU32(out, allocator, @intCast(body.len));
        try out.appendSlice(allocator, body);

        // consts
        const consts = world.chunk_consts.get(chunk_id) orelse &[_]value.Value{};
        try appendU16(out, allocator, @intCast(consts.len));
        for (consts) |c| {
            try appendValue(out, allocator, c);
        }

        // ic_count
        const ics_len: u16 = if (world.chunk_ics.get(chunk_id)) |ics| @intCast(ics.len) else 0;
        try appendU16(out, allocator, ics_len);

        // params
        const params = world.chunk_params.get(chunk_id) orelse &[_]u32{};
        try appendU16(out, allocator, @intCast(params.len));
        for (params) |p| {
            try appendU32(out, allocator, p);
        }
    }

    // ── NativeRefsSection ──────────────────────────────────────
    //
    // matches rust's `collect_native_methods` shape: walk every proto
    // Form's handlers table; emit one (method_form_id, "ProtoName:selector")
    // entry per method-FormId that lives in native_fns.
    //
    // first pass: count. second pass: emit (ArrayList is forward-only).
    var native_count: u32 = 0;
    {
        var hi: usize = 1;
        while (hi < world.heap.len()) : (hi += 1) {
            const proto_id = FormId.vatLocal(@intCast(hi));
            const proto_form = world.heap.get(proto_id);
            if (proto_form.handlers.count() == 0) continue;
            var hit = proto_form.handlers.iterator();
            while (hit.next()) |entry| {
                const method_v = entry.value_ptr.*;
                if (method_v.asFormId()) |mid| {
                    if (world.native_fns.contains(mid)) native_count += 1;
                }
            }
        }
    }
    try appendU32(out, allocator, native_count);
    {
        var hi: usize = 1;
        while (hi < world.heap.len()) : (hi += 1) {
            const proto_id = FormId.vatLocal(@intCast(hi));
            const proto_form = world.heap.get(proto_id);
            if (proto_form.handlers.count() == 0) continue;
            // best-effort proto name from :name meta sym; fall back to
            // "<anon-N>" mirroring rust's collect_native_methods.
            const name_sym = world.symName;
            const proto_name_v = proto_form.metaAt(name_sym);
            var proto_name_buf: [64]u8 = undefined;
            const proto_name: []const u8 = if (proto_name_v.asSym()) |s| world.syms.resolve(s) else blk: {
                break :blk std.fmt.bufPrint(&proto_name_buf, "<anon-{d}>", .{hi}) catch unreachable;
            };
            var h_it2 = proto_form.handlers.iterator();
            while (h_it2.next()) |entry| {
                const sel_sym = entry.key_ptr.*;
                const method_v = entry.value_ptr.*;
                const mid = method_v.asFormId() orelse continue;
                if (!world.native_fns.contains(mid)) continue;
                const sel_text = world.syms.resolve(sel_sym);

                // "ProtoName:selector" — same shape rust emits.
                var name_buf: [256]u8 = undefined;
                const full_name = try std.fmt.bufPrint(&name_buf, "{s}:{s}", .{ proto_name, sel_text });
                const trunc_len = @min(full_name.len, 255);
                const truncated = full_name[0..trunc_len];

                try appendU32(out, allocator, @bitCast(mid));
                try out.append(allocator, @intCast(trunc_len));
                try out.appendSlice(allocator, truncated);
            }
        }
    }

    // ── McoBindingsSection ─────────────────────────────────────
    // empty stub (TODO phase D — wasm wiring).
    try appendU32(out, allocator, 0);

    // ── FarRefsSection ─────────────────────────────────────────
    // empty stub (single-vat for now).
    try appendU32(out, allocator, 0);

    // ── Footer ─────────────────────────────────────────────────
    // 32-byte image hash. TODO: real blake3; zeros for now (matches
    // rust v4_export — both stubs in sync until phase 9).
    const zeros: [32]u8 = .{0} ** 32;
    try out.appendSlice(allocator, &zeros);
}

// ---------------------------------------------------------------------------
// Value byte encoder (used by serializeVat)
// ---------------------------------------------------------------------------

/// encode a Value into `out` using the same VTAG_* scheme as the
/// loader's `readValue`. tags 0xC0–0xC7 per spec ambiguity flagged at
/// top of file.
fn appendValue(out: *std.ArrayList(u8), allocator: std.mem.Allocator, v: value.Value) !void {
    switch (v) {
        .nil => try out.append(allocator, VTAG_NIL),
        .bool_ => |b| try out.append(allocator, if (b) VTAG_BOOL_TRUE else VTAG_BOOL_FALSE),
        .int => |n| {
            try out.append(allocator, VTAG_INT);
            // wire is i64 BE per V4 §10.3; cast from i48.
            try appendI64(out, allocator, @as(i64, n));
        },
        .sym => |s| {
            try out.append(allocator, VTAG_SYM);
            try appendU32(out, allocator, s);
        },
        .char => |cp| {
            try out.append(allocator, VTAG_CHAR);
            try appendU32(out, allocator, cp);
        },
        .float => |f| {
            try out.append(allocator, VTAG_FLOAT);
            const raw: u64 = @bitCast(f);
            try appendU64(out, allocator, raw);
        },
        .form => |id| {
            try out.append(allocator, VTAG_FORM);
            try appendU32(out, allocator, @bitCast(id));
        },
    }
}

// ---------------------------------------------------------------------------
// big-endian append helpers
// ---------------------------------------------------------------------------

fn appendU16(out: *std.ArrayList(u8), allocator: std.mem.Allocator, v: u16) !void {
    var buf: [2]u8 = undefined;
    std.mem.writeInt(u16, &buf, v, .big);
    try out.appendSlice(allocator, &buf);
}

fn appendU32(out: *std.ArrayList(u8), allocator: std.mem.Allocator, v: u32) !void {
    var buf: [4]u8 = undefined;
    std.mem.writeInt(u32, &buf, v, .big);
    try out.appendSlice(allocator, &buf);
}

fn appendU64(out: *std.ArrayList(u8), allocator: std.mem.Allocator, v: u64) !void {
    var buf: [8]u8 = undefined;
    std.mem.writeInt(u64, &buf, v, .big);
    try out.appendSlice(allocator, &buf);
}

fn appendI64(out: *std.ArrayList(u8), allocator: std.mem.Allocator, v: i64) !void {
    var buf: [8]u8 = undefined;
    std.mem.writeInt(i64, &buf, v, .big);
    try out.appendSlice(allocator, &buf);
}

// ---------------------------------------------------------------------------
// Header reader
// ---------------------------------------------------------------------------

/// read the fixed-size header per spec §10.3. advances `pos`.
fn readHeader(bytes: []const u8, pos: *usize) !Header {
    // vat_id: [16]u8
    try requireBytes(bytes, pos.*, 16);
    var vat_id: [16]u8 = undefined;
    @memcpy(&vat_id, bytes[pos.*..][0..16]);
    pos.* += 16;

    // num_forms, num_syms, num_chunks, here_form_id, macros_form_id (5 × u32 BE)
    const num_forms = try readU32(bytes, pos);
    const num_syms = try readU32(bytes, pos);
    const num_chunks = try readU32(bytes, pos);
    const here_form_id = try readU32(bytes, pos);
    const macros_form_id = try readU32(bytes, pos);

    // 18 proto FormIds in canonical order (spec §10.3)
    const protos = ProtoTable{
        .object = try readU32(bytes, pos),
        .nil = try readU32(bytes, pos),
        .bool_ = try readU32(bytes, pos),
        .integer = try readU32(bytes, pos),
        .char = try readU32(bytes, pos),
        .sym = try readU32(bytes, pos),
        .cons = try readU32(bytes, pos),
        .string = try readU32(bytes, pos),
        .bytes = try readU32(bytes, pos),
        .method = try readU32(bytes, pos),
        .chunk = try readU32(bytes, pos),
        .closure = try readU32(bytes, pos),
        .env = try readU32(bytes, pos),
        .foreign_handle = try readU32(bytes, pos),
        .table = try readU32(bytes, pos),
        .frame = try readU32(bytes, pos),
        .macros = try readU32(bytes, pos),
        .opcode = try readU32(bytes, pos),
    };

    // external_vat_refs_count: u16 BE
    const num_external_vat_refs = try readU16(bytes, pos);

    // external_vat_refs: [count × [16]u8] — borrowed slice
    const ext_bytes_len = @as(usize, num_external_vat_refs) * 16;
    try requireBytes(bytes, pos.*, ext_bytes_len);
    const ext_slice = std.mem.bytesAsSlice([16]u8, bytes[pos.*..][0..ext_bytes_len]);
    pos.* += ext_bytes_len;

    return .{
        .vat_id = vat_id,
        .num_forms = num_forms,
        .num_syms = num_syms,
        .num_chunks = num_chunks,
        .here_form_id = here_form_id,
        .macros_form_id = macros_form_id,
        .protos = protos,
        .num_external_vat_refs = num_external_vat_refs,
        .external_vat_refs = ext_slice,
    };
}

// ---------------------------------------------------------------------------
// Section readers
// ---------------------------------------------------------------------------

/// SymTableSection — `count:u32 [for each: len:u16 bytes:[len]]`.
/// per spec §10.3. interns each name in order; the resulting SymId
/// equals 1 + the index into the section (the NONE=0 sentinel always
/// occupies slot 0 in the World's sym table; rust's serializer writes
/// only the non-sentinel symbols).
///
/// V4 §10 hydration: REPLACE the World's sym table with the image's.
/// SymIds inside the image's chunks/forms/handlers are indices into
/// THIS table; appending to a pre-populated World would shift them.
/// per Gemini's brainstorm finding.
fn readSymTable(world: *World, bytes: []const u8, pos: *usize, expected_count: u32) !void {
    const count = try readU32(bytes, pos);
    if (count != expected_count) return ImageError.TruncatedImage;
    // clear the World's syms (keeps the NONE sentinel at index 0).
    world.syms.clearAndKeepCapacity();
    var i: u32 = 0;
    while (i < count) : (i += 1) {
        const len = try readU16(bytes, pos);
        try requireBytes(bytes, pos.*, len);
        const name = bytes[pos.*..][0..len];
        pos.* += len;
        _ = try world.syms.intern(name);
    }
}

/// FormSection — alloc each Form in order. FormId payload = index+1
/// (FormId 0 is the sentinel; first allocation lands at 1). populates
/// proto/slots/handlers/meta/frozen per spec §10.3.
fn readForms(world: *World, bytes: []const u8, pos: *usize, expected_count: u32) !void {
    const count = try readU32(bytes, pos);
    if (count != expected_count) return ImageError.TruncatedImage;
    var i: u32 = 0;
    while (i < count) : (i += 1) {
        const proto_val = try readValue(bytes, pos);

        // allocate a fresh form with the parsed proto. the heap's
        // alloc() returns the next FormId (payload monotonically
        // increases from 1; #0 is the sentinel per spec §10.5).
        const fid = try world.heap.alloc(Form.withProto(proto_val));
        const form_ptr = world.heap.getMut(fid);

        // slots: count:u16 [name_sym:u32 val:Value]
        const slots_count = try readU16(bytes, pos);
        var s: u16 = 0;
        while (s < slots_count) : (s += 1) {
            const name_sym = try readU32(bytes, pos);
            const val = try readValue(bytes, pos);
            try form_ptr.slots.put(world.allocator, name_sym, val);
        }

        // handlers: count:u16 [sel_sym:u32 method:Value]
        const handlers_count = try readU16(bytes, pos);
        var h: u16 = 0;
        while (h < handlers_count) : (h += 1) {
            const sel_sym = try readU32(bytes, pos);
            const method = try readValue(bytes, pos);
            try form_ptr.handlers.put(world.allocator, sel_sym, method);
        }

        // meta: count:u16 [key_sym:u32 val:Value]
        const meta_count = try readU16(bytes, pos);
        var m: u16 = 0;
        while (m < meta_count) : (m += 1) {
            const key_sym = try readU32(bytes, pos);
            const val = try readValue(bytes, pos);
            try form_ptr.meta.put(world.allocator, key_sym, val);
        }

        // frozen: u8 (0 or 1)
        try requireBytes(bytes, pos.*, 1);
        form_ptr.frozen = bytes[pos.*] != 0;
        pos.* += 1;
    }
}

/// read one chunk from `bytes` at `pos`, allocate a fresh Form for it,
/// and populate the world's chunk side-tables. returns the new FormId.
/// per spec §10.3 ChunkSection entry layout.
pub fn loadChunk(world: *World, bytes: []const u8, pos: *usize, allocator: std.mem.Allocator) !FormId {
    _ = try readU32(bytes, pos); // source_form (ignored for fresh chunks)

    const chunk_fid = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));

    const body_len = try readU32(bytes, pos);
    try requireBytes(bytes, pos.*, body_len);
    const body = try allocator.dupe(u8, bytes[pos.*..][0..body_len]);
    pos.* += body_len;
    try world.chunk_bytecode.put(allocator, chunk_fid, body);

    const consts_count = try readU16(bytes, pos);
    const consts = try allocator.alloc(Value, consts_count);
    var c: u16 = 0;
    while (c < consts_count) : (c += 1) {
        consts[c] = try readValue(bytes, pos);
    }
    try world.chunk_consts.put(allocator, chunk_fid, consts);

    const ic_count = try readU16(bytes, pos);
    const ics = try allocator.alloc(world_mod.ICache, ic_count);
    var ic_i: u16 = 0;
    while (ic_i < ic_count) : (ic_i += 1) ics[ic_i] = world_mod.ICache.empty;
    try world.chunk_ics.put(allocator, chunk_fid, ics);

    const params_count = try readU16(bytes, pos);
    const params = try allocator.alloc(u32, params_count);
    var p: u16 = 0;
    while (p < params_count) : (p += 1) {
        params[p] = try readU32(bytes, pos);
    }
    try world.chunk_params.put(allocator, chunk_fid, params);

    return chunk_fid;
}

/// ChunkSection — populate world.chunk_bytecode / chunk_consts /
/// chunk_ics / chunk_params keyed by source-FormId per spec §10.3.
///
/// `allocator` is the World's allocator (also the heap's). all owned
/// slices freed by `World.deinit` come from this allocator.
fn readChunks(world: *World, bytes: []const u8, pos: *usize, expected_count: u32, allocator: std.mem.Allocator) !void {
    const count = try readU32(bytes, pos);
    if (count != expected_count) return ImageError.TruncatedImage;
    var i: u32 = 0;
    while (i < count) : (i += 1) {
        const source_form = try readU32(bytes, pos);
        const chunk_id = vatLocalId(source_form);

        // body_len:u32 body:[body_len]
        const body_len = try readU32(bytes, pos);
        try requireBytes(bytes, pos.*, body_len);
        // dup the body bytes so the World owns them (the input slice
        // may be a mmap and the World may outlive it; safer to copy).
        const body = try allocator.dupe(u8, bytes[pos.*..][0..body_len]);
        pos.* += body_len;
        try world.chunk_bytecode.put(allocator, chunk_id, body);

        // consts_count:u16 [Value × n]
        const consts_count = try readU16(bytes, pos);
        const consts = try allocator.alloc(Value, consts_count);
        var c: u16 = 0;
        while (c < consts_count) : (c += 1) {
            consts[c] = try readValue(bytes, pos);
        }
        try world.chunk_consts.put(allocator, chunk_id, consts);

        // ic_count:u16 — ICs are zero-initialized at load (spec §10.3 step 6).
        // we allocate the slice now so vm.zig's fast-path can index in.
        const ic_count = try readU16(bytes, pos);
        const ics = try allocator.alloc(world_mod.ICache, ic_count);
        var ic_i: u16 = 0;
        while (ic_i < ic_count) : (ic_i += 1) ics[ic_i] = world_mod.ICache.empty;
        try world.chunk_ics.put(allocator, chunk_id, ics);

        // params_count:u16 [u32 sym × n]
        const params_count = try readU16(bytes, pos);
        const params = try allocator.alloc(u32, params_count);
        var p: u16 = 0;
        while (p < params_count) : (p += 1) {
            params[p] = try readU32(bytes, pos);
        }
        try world.chunk_params.put(allocator, chunk_id, params);
    }
}

/// NativeRefsSection — re-bind native methods by name. names are
/// canonical "proto:selector" strings like `"Object:+:"` or
/// `"Env:bind:to:"`. per spec §10.3 step 7.
fn readNativeRefs(world: *World, bytes: []const u8, pos: *usize) !void {
    const count = try readU32(bytes, pos);
    var i: u32 = 0;
    while (i < count) : (i += 1) {
        const method_form_id = try readU32(bytes, pos);
        try requireBytes(bytes, pos.*, 1);
        const name_len = bytes[pos.*];
        pos.* += 1;
        try requireBytes(bytes, pos.*, name_len);
        const name = bytes[pos.*..][0..name_len];
        pos.* += name_len;

        // look up the named native in the process-wide intrinsics
        // table and install it on world.native_fns[method_form_id].
        // log + SKIP if missing — zig ships a 29-native MVS REGISTRY;
        // the rust v4_export's World may have more (~50). methods
        // that need missing natives will fail at dispatch time
        // ("method body not callable") but the world LOADS. additions
        // to zig's REGISTRY are how the gap is closed long-term.
        if (world.lookupNativeByName(name)) |native_fn| {
            try world.native_fns.put(world.allocator, vatLocalId(method_form_id), native_fn);
        } else {
            std.log.warn("image-load: unknown native '{s}' for form_id={d} — skipping (method will fail at dispatch)", .{ name, method_form_id });
        }
    }
}

/// McoBindingsSection — instantiate cached wasm mcos.
/// **V4 MVP stub**: log and continue. real wasm-runtime wiring is a
/// separate concern (track for phase γ+). per spec §10.3 step 8.
fn readMcoBindings(world: *World, bytes: []const u8, pos: *usize) !void {
    _ = world;
    const count = try readU32(bytes, pos);
    var i: u32 = 0;
    while (i < count) : (i += 1) {
        try requireBytes(bytes, pos.*, 32);
        const mco_hash = bytes[pos.*..][0..32];
        pos.* += 32;
        const proto_form_id = try readU32(bytes, pos);

        // TODO(wasm): read mcos/<hex(mco_hash)> from the bundle's
        // mco cache, instantiate via wasmtime/wasmer, bind to the
        // proto FormId. for V4 MVP: noop log (don't try to render
        // the hash — zig 0.16 dropped std.fmt.fmtSliceHexLower).
        _ = mco_hash;
        std.log.info("would load mco for proto form_id={}", .{proto_form_id});
    }
}

/// FarRefsSection — populate the far-ref table. resolution is lazy:
/// when the VM dereferences a FormId with `scope = .far_ref`, it
/// consults `world.far_ref_table` to find the target vat+form pair.
/// per spec §10.4.
fn readFarRefs(world: *World, bytes: []const u8, pos: *usize) !void {
    const count = try readU32(bytes, pos);
    var i: u32 = 0;
    while (i < count) : (i += 1) {
        const local_form_id = try readU32(bytes, pos);
        try requireBytes(bytes, pos.*, 16);
        var target_vat_id: [16]u8 = undefined;
        @memcpy(&target_vat_id, bytes[pos.*..][0..16]);
        pos.* += 16;
        const target_form_id = try readU32(bytes, pos);

        // store with scope=.far_ref so heap.get() routes through the
        // far-ref table on first dereference (handled by the heap;
        // we just populate the entry).
        const local: FormId = .{ .payload = @intCast(local_form_id), .scope = .far_ref };
        try world.far_ref_table.put(world.allocator, local, .{
            .target_vat_id = target_vat_id,
            .target_form_id = target_form_id,
        });
    }
}

// ---------------------------------------------------------------------------
// Value byte decoder (in-image, V4 §4 ambiguity — see top-of-file)
// ---------------------------------------------------------------------------

/// decode one in-image `Value`. tag byte + variable operand, all BE.
/// see VTAG_* constants above. flagged for spec promotion.
fn readValue(bytes: []const u8, pos: *usize) !Value {
    try requireBytes(bytes, pos.*, 1);
    const tag = bytes[pos.*];
    pos.* += 1;
    return switch (tag) {
        VTAG_NIL => Value{ .nil = {} },
        VTAG_BOOL_FALSE => Value{ .bool_ = false },
        VTAG_BOOL_TRUE => Value{ .bool_ = true },
        VTAG_INT => blk: {
            try requireBytes(bytes, pos.*, 8);
            const raw = std.mem.readInt(i64, bytes[pos.*..][0..8], .big);
            pos.* += 8;
            // wire pads to i64; Value also uses i64 in phase A.
            break :blk Value{ .int = raw };
        },
        VTAG_SYM => blk: {
            const raw = try readU32(bytes, pos);
            break :blk Value{ .sym = raw };
        },
        VTAG_CHAR => blk: {
            const raw = try readU32(bytes, pos);
            break :blk Value{ .char = raw };
        },
        VTAG_FLOAT => blk: {
            try requireBytes(bytes, pos.*, 8);
            const raw = std.mem.readInt(u64, bytes[pos.*..][0..8], .big);
            pos.* += 8;
            break :blk Value{ .float = @bitCast(raw) };
        },
        VTAG_FORM => blk: {
            const raw = try readU32(bytes, pos);
            // the raw u32 already encodes the 2-bit scope tag in its
            // top bits per V0 FormId layout — bitcast preserves it.
            const fid: FormId = @bitCast(raw);
            break :blk Value{ .form = fid };
        },
        else => ImageError.BadValueTag,
    };
}

// ---------------------------------------------------------------------------
// Primitive readers (big-endian, bounds-checked)
// ---------------------------------------------------------------------------

inline fn requireBytes(bytes: []const u8, pos: usize, need: usize) !void {
    if (pos + need > bytes.len) return ImageError.TruncatedImage;
}

fn readU16(bytes: []const u8, pos: *usize) !u16 {
    try requireBytes(bytes, pos.*, 2);
    const v = std.mem.readInt(u16, bytes[pos.*..][0..2], .big);
    pos.* += 2;
    return v;
}

fn readU32(bytes: []const u8, pos: *usize) !u32 {
    try requireBytes(bytes, pos.*, 4);
    const v = std.mem.readInt(u32, bytes[pos.*..][0..4], .big);
    pos.* += 4;
    return v;
}

/// build a vat-local FormId from a raw u32. extracts payload bits;
/// the scope is set to `.vat_local`. for far-ref-scope FormIds the
/// caller should bitcast directly (see readValue and readFarRefs).
inline fn vatLocalId(raw: u32) FormId {
    return .{ .payload = @intCast(raw & 0x3FFF_FFFF), .scope = .vat_local };
}
