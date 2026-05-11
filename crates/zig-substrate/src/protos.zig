//! the canonical primordial protos for a moof vat.
//!
//! every fresh moof world allocates these proto Forms at boot; their
//! `FormId`s live in [`Protos`] and feed dispatch for tagged immediates
//! (Integer, Char, Sym, Bool, Nil — see V0 scope-tagging design) and
//! per-Form proto delegation.
//!
//! protos are *empty* at allocation (no handlers, no slots); the
//! intrinsics installer wires their handler tables later. each proto's
//! `:name` meta is populated here so reflection (`[Integer toString]`
//! → `"Integer"`) works from the moment the heap is live.
//!
//! ## the chain
//!
//! ```text
//! Object              proto: Nil
//!  ├── Nil-proto       proto: Object  (the proto of nil-the-value)
//!  ├── Bool            proto: Object
//!  ├── Integer         proto: Object
//!  ├── Char            proto: Object
//!  ├── Sym             proto: Object
//!  ├── Cons            proto: Object
//!  ├── String          proto: Object
//!  ├── Bytes           proto: Object
//!  ├── Method          proto: Object
//!  ├── Chunk           proto: Object
//!  ├── Closure         proto: Method
//!  ├── Env             proto: Object
//!  ├── ForeignHandle   proto: Object
//!  ├── Table           proto: Object
//!  ├── Frame           proto: Object   (R3 — running computation)
//!  ├── Macros          proto: Object   (canonical macro registry)
//!  └── Opcode          proto: Object   (reflection R2 — decoded ops)
//! ```
//!
//! note: `Nil-proto` is the *proto* used by the `nil` value. it is
//! distinct from `Value::Nil` itself. the `Nil-proto` Form's own
//! `proto` field is `Object`.
//!
//! the proto field set here matches the V4 image-format Header proto
//! table (see `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`
//! §10.3 — "Header := { ..., protos: { ... } }").

const std = @import("std");

const value = @import("value.zig");
const Value = value.Value;

const form = @import("form.zig");
const FormId = form.FormId;
const Form = form.Form;

const sym_mod = @import("sym.zig");
const SymTable = sym_mod.SymTable;

const heap_mod = @import("heap.zig");
const Heap = heap_mod.Heap;

/// the canonical proto FormIds. populated by [`bootstrap`].
///
/// field set + order matches the V4 image Header.protos table per
/// `2026-05-10-vm-V4-opcodes-design.md` §10.3 so save/restore is a
/// straight field copy.
pub const Protos = struct {
    object: FormId,
    nil: FormId,
    bool_: FormId,
    integer: FormId,
    char: FormId,
    sym: FormId,
    cons: FormId,
    string: FormId,
    bytes: FormId,
    method: FormId,
    chunk: FormId,
    closure: FormId,
    env: FormId,
    foreign_handle: FormId,
    table: FormId,
    frame: FormId,
    macros: FormId,
    opcode: FormId,
};

/// allocate empty proto Forms for each canonical kind.
///
/// - `Object` is the root: its proto is `Value.nil` (it has no parent).
/// - every other proto has proto = `Value{ .form = object }`, except
///   `Closure` whose proto is `Method` (per V4 spec — closures inherit
///   the dispatch machinery of methods).
/// - each proto's `:name` meta slot is populated with a Symbol whose
///   text matches the field name (capitalized — `Integer`, `Cons`,
///   `Macros`, etc.) so reflection has something to show before
///   intrinsics run.
///
/// handler tables stay empty here; phase-A intrinsics installs them.
pub fn bootstrap(
    heap: *Heap,
    syms: *SymTable,
    allocator: std.mem.Allocator,
) !Protos {
    _ = allocator; // present for API parity / future per-proto deinit hooks

    // intern the `name` meta key once.
    const name_meta = try syms.intern("name");

    // Object is the root; its proto is Nil so the chain has a terminator.
    const object = try heap.alloc(Form.withProto(Value.nil));

    // helper: allocate a Form whose proto is Object.
    const objectProto = Value{ .form = object };

    const nil_p = try heap.alloc(Form.withProto(objectProto));
    const bool_p = try heap.alloc(Form.withProto(objectProto));
    const integer_p = try heap.alloc(Form.withProto(objectProto));
    const char_p = try heap.alloc(Form.withProto(objectProto));
    const sym_p = try heap.alloc(Form.withProto(objectProto));
    const cons_p = try heap.alloc(Form.withProto(objectProto));
    const string_p = try heap.alloc(Form.withProto(objectProto));
    const bytes_p = try heap.alloc(Form.withProto(objectProto));
    const method_p = try heap.alloc(Form.withProto(objectProto));
    const chunk_p = try heap.alloc(Form.withProto(objectProto));
    // Closure proto = Method (closures inherit Method dispatch).
    const closure_p = try heap.alloc(Form.withProto(Value{ .form = method_p }));
    const env_p = try heap.alloc(Form.withProto(objectProto));
    const foreign_p = try heap.alloc(Form.withProto(objectProto));
    const table_p = try heap.alloc(Form.withProto(objectProto));
    const frame_p = try heap.alloc(Form.withProto(objectProto));
    const macros_p = try heap.alloc(Form.withProto(objectProto));
    const opcode_p = try heap.alloc(Form.withProto(objectProto));

    const protos = Protos{
        .object = object,
        .nil = nil_p,
        .bool_ = bool_p,
        .integer = integer_p,
        .char = char_p,
        .sym = sym_p,
        .cons = cons_p,
        .string = string_p,
        .bytes = bytes_p,
        .method = method_p,
        .chunk = chunk_p,
        .closure = closure_p,
        .env = env_p,
        .foreign_handle = foreign_p,
        .table = table_p,
        .frame = frame_p,
        .macros = macros_p,
        .opcode = opcode_p,
    };

    // populate `:name` meta on every proto so `[Integer toString]`
    // renders `Integer`, not `<Form#3>`. matches the rust seed's
    // intrinsics::install boot dance for proto-name globals.
    const NamedProto = struct { id: FormId, name: []const u8 };
    const named = [_]NamedProto{
        .{ .id = object,     .name = "Object" },
        .{ .id = nil_p,      .name = "Nil" },
        .{ .id = bool_p,     .name = "Bool" },
        .{ .id = integer_p,  .name = "Integer" },
        .{ .id = char_p,     .name = "Char" },
        .{ .id = sym_p,      .name = "Sym" },
        .{ .id = cons_p,     .name = "Cons" },
        .{ .id = string_p,   .name = "String" },
        .{ .id = bytes_p,    .name = "Bytes" },
        .{ .id = method_p,   .name = "Method" },
        .{ .id = chunk_p,    .name = "Chunk" },
        .{ .id = closure_p,  .name = "Closure" },
        .{ .id = env_p,      .name = "Env" },
        .{ .id = foreign_p,  .name = "ForeignHandle" },
        .{ .id = table_p,    .name = "Table" },
        .{ .id = frame_p,    .name = "Frame" },
        .{ .id = macros_p,   .name = "Macros" },
        .{ .id = opcode_p,   .name = "Opcode" },
    };

    for (named) |np| {
        const sym_id = try syms.intern(np.name);
        const f = heap.getMut(np.id);
        try f.meta.put(name_meta, Value{ .sym = sym_id });
    }

    return protos;
}
