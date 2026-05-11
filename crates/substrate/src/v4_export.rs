//! V4 byte-tagged bytecode export. Walks the rust World produced
//! by `new_world()` and serializes it as a V4 vat-image per the
//! spec at `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`
//! (§3 op encoding, §4 byte layout, §10.3 per-vat image format).
//!
//! Strategy D from the 2026-05-10 C.3 brainstorm: rust is the
//! build-time oracle. This is throwaway code that will be deleted
//! once parser.moof + compiler.moof self-host (phase A-self-host).
//!
//! ## bypasses / known gaps (intentional for V4 minimum viable)
//!
//! - V3 has fewer opcodes than V4. We emit `LoadSelf` + `Send` (not
//!   the fused `SendSelf`); same for `LoadHere`/`SendHere`. moof-zig
//!   handles both shapes; the fused emissions are phase-A-self-host's
//!   job.
//! - `Value::Foreign(_)` has no V4 inline tag — we encode it as Nil
//!   with a TODO. Foreign handles are vat-local and never serialize
//!   (per `value.rs` doc); reflection-side this is acceptable.
//! - Strings/Bytes/Cons are already heap-allocated as Forms in rust,
//!   so they emit as `Value::Form(_)` tags. No special inline-blob
//!   path needed.
//! - Protos table: V4 spec asks for 18 entries; rust has 17 (+ Float,
//!   – macros, – opcode). We emit FormId(0) for absent ones and slot
//!   `world.macros_form` into the macros position.
//! - Footer image hash is zeros (moof-zig stubs verification).
//! - `params_count` for each chunk reads the chunk-Form's `:params`
//!   slot list — already canonical for closures via `compiler.rs`.

use crate::form::FormId;
use crate::opcodes::Op;
use crate::sym::SymId;
use crate::value::Value;
use crate::world::World;

// ─────────────────────────────────────────────────────────────────
// op size + encoding (spec §3 / §4)

/// Byte width of one Op when encoded in V4 form.
pub fn op_byte_size(op: &Op) -> usize {
    match op {
        Op::PushNil | Op::PushTrue | Op::PushFalse | Op::Pop | Op::Dup
        | Op::LoadSelf | Op::Return => 1,
        Op::LoadConst(_) => 3,                  // tag + u16
        Op::LoadName(_) => 5,                   // tag + u32
        Op::Send { .. } => 8,                   // tag + u32 + u8 + u16
        Op::TailSend { .. } => 6,               // tag + u32 + u8
        Op::SuperSend { .. } => 8,
        Op::Jump(_) | Op::JumpIfFalse(_) => 3,  // tag + i16
        Op::PushClosure { .. } => 5,            // tag + u32
    }
}

/// Encode a single Op to V4 byte-tagged bytecode, appending to `buf`.
///
/// `byte_positions[i]` is the byte-offset of op `i` within the chunk
/// body. Used to convert rust's op-index-relative `Jump`/`JumpIfFalse`
/// offsets into V4's byte-relative offsets (spec §3.4).
pub fn encode_op(op: &Op, buf: &mut Vec<u8>, op_idx: usize, byte_positions: &[usize]) {
    match op {
        Op::PushNil => buf.push(0x01),
        Op::PushTrue => buf.push(0x02),
        Op::PushFalse => buf.push(0x03),
        Op::LoadConst(idx) => {
            buf.push(0x04);
            buf.extend_from_slice(&idx.to_be_bytes());
        }
        Op::LoadSelf => buf.push(0x05),
        Op::LoadName(sym) => {
            buf.push(0x07);
            buf.extend_from_slice(&(sym.0 as u32).to_be_bytes());
        }
        Op::Pop => buf.push(0x10),
        Op::Dup => buf.push(0x11),
        Op::Send { selector, argc, ic_idx } => {
            buf.push(0x20);
            buf.extend_from_slice(&(selector.0 as u32).to_be_bytes());
            buf.push(*argc);
            buf.extend_from_slice(&ic_idx.to_be_bytes());
        }
        Op::TailSend { selector, argc } => {
            buf.push(0x21);
            buf.extend_from_slice(&(selector.0 as u32).to_be_bytes());
            buf.push(*argc);
        }
        Op::SuperSend { selector, argc, ic_idx } => {
            buf.push(0x22);
            buf.extend_from_slice(&(selector.0 as u32).to_be_bytes());
            buf.push(*argc);
            buf.extend_from_slice(&ic_idx.to_be_bytes());
        }
        Op::Jump(op_offset) => {
            buf.push(0x30);
            let byte_offset = compute_byte_offset(op_idx, *op_offset, byte_positions);
            buf.extend_from_slice(&byte_offset.to_be_bytes());
        }
        Op::JumpIfFalse(op_offset) => {
            buf.push(0x31);
            let byte_offset = compute_byte_offset(op_idx, *op_offset, byte_positions);
            buf.extend_from_slice(&byte_offset.to_be_bytes());
        }
        Op::Return => buf.push(0x33),
        Op::PushClosure { chunk } => {
            buf.push(0x40);
            buf.extend_from_slice(&chunk.0.to_be_bytes());
        }
    }
}

/// Convert rust's op-index-relative offset to V4's byte-relative
/// offset. Rust's `Jump(n)` means "set pc to op_idx + n"; V4's
/// `Jump(n)` means "set byte_pc to current_byte + 3 + n" (offset is
/// measured from the byte AFTER the jump's operand bytes).
///
/// In V4 spec §3.4 the offset semantics: `pc += offset` where pc is
/// the byte AFTER the jump op (3 bytes ahead of jump's tag). So:
///   target_byte = post_jump_byte + offset
///   target_byte = current_byte + 3 + offset
///   offset = target_byte - current_byte - 3
fn compute_byte_offset(op_idx: usize, op_offset: i16, byte_positions: &[usize]) -> i16 {
    let target_op = (op_idx as isize + op_offset as isize) as usize;
    let target_byte = byte_positions[target_op] as isize;
    let current_byte = byte_positions[op_idx] as isize;
    // V4 jump offset is from the byte AFTER the 3-byte jump op.
    (target_byte - current_byte - 3) as i16
}

/// Encode a whole chunk's `Vec<Op>` to V4 byte-tagged bytecode.
/// Two-pass: compute byte positions, then emit bytes (with byte-
/// based jump offsets).
pub fn encode_chunk_ops(ops: &[Op]) -> Vec<u8> {
    // pass 1: compute byte position of each op.
    let mut byte_positions = Vec::with_capacity(ops.len() + 1);
    let mut cursor = 0;
    for op in ops {
        byte_positions.push(cursor);
        cursor += op_byte_size(op);
    }
    byte_positions.push(cursor); // sentinel for "past end"

    // pass 2: emit bytes.
    let mut buf = Vec::with_capacity(cursor);
    for (i, op) in ops.iter().enumerate() {
        encode_op(op, &mut buf, i, &byte_positions);
    }
    buf
}

// ─────────────────────────────────────────────────────────────────
// value encoding (spec §10.3 — inline byte-tagged Value)

/// Encode a Value as V4 byte-tagged inline. Per spec §10.3 Value
/// byte tags 0xC0-0xC7.
///
/// `Value::Foreign(_)` has no V4 inline tag; we emit Nil as a stub
/// with a TODO trail (the spec considers foreign handles vat-local
/// and non-serializable).
pub fn encode_value(v: Value, buf: &mut Vec<u8>) {
    match v {
        Value::Nil => buf.push(0xC0),
        Value::Bool(false) => buf.push(0xC1),
        Value::Bool(true) => buf.push(0xC2),
        Value::Int(n) => {
            buf.push(0xC3);
            buf.extend_from_slice(&n.to_be_bytes());
        }
        Value::Sym(s) => {
            buf.push(0xC4);
            buf.extend_from_slice(&(s.0 as u32).to_be_bytes());
        }
        Value::Char(cp) => {
            buf.push(0xC5);
            buf.extend_from_slice(&cp.to_be_bytes());
        }
        Value::Float(bits) => {
            buf.push(0xC6);
            buf.extend_from_slice(&bits.to_be_bytes());
        }
        Value::Form(id) => {
            buf.push(0xC7);
            buf.extend_from_slice(&id.0.to_be_bytes());
        }
        Value::Foreign(_) => {
            // TODO(phase-A-self-host): foreign handles can't cross
            // the image boundary. Emit Nil; consumer treats it as
            // "this slot held a non-portable handle, re-bind via
            // intrinsics on load." String/Bytes/Table reps will
            // be re-allocated by the moof-zig side from their
            // owning Form's structure.
            buf.push(0xC0);
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// native methods collection

/// Walk every proto Form's handler table; for each method-Form whose
/// FormId is a key in `world.native_fns`, emit
/// `(method_form_id, "ProtoName:selectorName")`.
///
/// The proto name comes from the proto Form's `:name` meta slot
/// (populated by `intrinsics::install_proto_globals`). Protos with no
/// `:name` meta — singletons, anonymous protos — are labelled
/// `"<anon-N>:selector"` where N is the proto's payload, so they
/// remain identifiable for debugging but won't match any zig-side
/// REGISTRY entry (which is fine; only canonical protos have native
/// methods at this stage).
pub fn collect_native_methods(world: &World) -> Vec<(FormId, String)> {
    let name_sym = world.syms.contains("name");
    let name_sym = if name_sym {
        // best-effort: we don't have a non-mut intern; instead, the
        // ":name" symbol is interned by install_proto_globals at boot,
        // so by the time we serialize, it's present. Re-resolve via
        // a linear scan of the sym table. SymTable has no public
        // iter, so we walk SymId(1..syms.len()+1) and find "name".
        find_sym(world, "name").expect("'name' sym must exist after boot")
    } else {
        // Pathological: bare world without proto_globals; no natives
        // to collect.
        return Vec::new();
    };

    let mut out = Vec::new();
    // iterate every Form in the heap (skip sentinel index 0).
    let heap_len = world.heap.len();
    for i in 1..heap_len {
        let proto_id = FormId::vat_local(i as u32);
        let proto_form = world.heap.get(proto_id);
        if proto_form.handlers.is_empty() {
            continue;
        }
        let proto_name = match proto_form.meta_at(name_sym) {
            Value::Sym(s) => world.resolve(s).to_string(),
            _ => format!("<anon-{}>", i),
        };
        for (sel, method_v) in proto_form.handlers.iter() {
            if let Value::Form(method_id) = method_v {
                if world.native_fns.contains_key(method_id) {
                    let sel_name = world.resolve(*sel);
                    out.push((*method_id, format!("{}:{}", proto_name, sel_name)));
                }
            }
        }
    }
    out
}

/// Linear scan for a SymId by name. SymTable has no public iter, so
/// we probe SymId(1..len+1). Used only by `collect_native_methods`
/// for the `:name` meta key — one-shot at serialization time.
fn find_sym(world: &World, target: &str) -> Option<SymId> {
    let total = world.syms.len();
    for i in 1..=total as u32 {
        let id = SymId(i);
        if world.resolve(id) == target {
            return Some(id);
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────
// full image serialization (spec §10.3)

/// Serialize the World as a V4 vat-image. Output layout:
///
/// ```text
/// Magic ("MVAT") | Version (u16) | Header | Sections | Footer
/// ```
///
/// See spec §10.3 for the full byte layout.
pub fn serialize_world(world: &World) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1024 * 1024);

    // ── Magic + Version ─────────────────────────────────────────
    buf.extend_from_slice(b"MVAT");
    buf.extend_from_slice(&0x0004_u16.to_be_bytes());

    // ── Header ──────────────────────────────────────────────────
    // vat_id: 16 bytes of zeros for now (TODO: real ULID).
    let vat_id = [0u8; 16];
    buf.extend_from_slice(&vat_id);

    // num_forms excludes the sentinel at FormId(0).
    let num_forms = (world.heap.len() as u32).saturating_sub(1);
    let num_syms = world.syms.len() as u32;
    let num_chunks = world.chunk_ops.len() as u32;
    buf.extend_from_slice(&num_forms.to_be_bytes());
    buf.extend_from_slice(&num_syms.to_be_bytes());
    buf.extend_from_slice(&num_chunks.to_be_bytes());

    buf.extend_from_slice(&world.here_form.0.to_be_bytes());
    buf.extend_from_slice(&world.macros_form.0.to_be_bytes());

    // Protos table — 18 entries per spec §10.3.
    // Order: object, nil, bool, integer, char, sym, cons, string,
    //        bytes, method, chunk, closure, env, foreign_handle,
    //        table, frame, macros, opcode.
    // Rust has no opcode proto; emit FormId(0). macros proto is
    // approximated by `world.macros_form` (which IS the canonical
    // Macros registry Form, just not a "proto" in rust's typology).
    let p = &world.protos;
    let proto_ids: [FormId; 18] = [
        p.object,
        p.nil,
        p.bool_,
        p.integer,
        p.char_,
        p.symbol,
        p.cons,
        p.string,
        p.bytes,
        p.method,
        p.chunk,
        p.closure,
        p.env,
        p.foreign,
        p.table,
        p.frame,
        world.macros_form,
        FormId::NONE, // opcode — absent in rust seed
    ];
    for id in proto_ids {
        buf.extend_from_slice(&id.0.to_be_bytes());
    }

    // external_vat_refs count = 0 (single-vat for now).
    buf.extend_from_slice(&0_u16.to_be_bytes());

    // ── SymTableSection ─────────────────────────────────────────
    buf.extend_from_slice(&num_syms.to_be_bytes());
    // SymIds 1..=num_syms in interning order (0 is the sentinel).
    for i in 1..=num_syms {
        let s = world.resolve(SymId(i));
        let bytes = s.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(bytes);
    }

    // ── FormSection ─────────────────────────────────────────────
    buf.extend_from_slice(&num_forms.to_be_bytes());
    for i in 1..world.heap.len() {
        let form_id = FormId::vat_local(i as u32);
        let form = world.heap.get(form_id);
        // proto
        encode_value(form.proto, &mut buf);
        // slots
        buf.extend_from_slice(&(form.slots.len() as u16).to_be_bytes());
        for (sym, val) in form.slots.iter() {
            buf.extend_from_slice(&(sym.0 as u32).to_be_bytes());
            encode_value(*val, &mut buf);
        }
        // handlers
        buf.extend_from_slice(&(form.handlers.len() as u16).to_be_bytes());
        for (sel, method) in form.handlers.iter() {
            buf.extend_from_slice(&(sel.0 as u32).to_be_bytes());
            encode_value(*method, &mut buf);
        }
        // meta
        buf.extend_from_slice(&(form.meta.len() as u16).to_be_bytes());
        for (key, val) in form.meta.iter() {
            buf.extend_from_slice(&(key.0 as u32).to_be_bytes());
            encode_value(*val, &mut buf);
        }
        // frozen flag
        buf.push(if form.frozen { 1 } else { 0 });
    }

    // ── ChunkSection ────────────────────────────────────────────
    buf.extend_from_slice(&num_chunks.to_be_bytes());
    // Pre-fetch the :params sym for chunk param-list inspection.
    let params_sym = find_sym(world, "params");
    let car_sym = find_sym(world, "car");
    let cdr_sym = find_sym(world, "cdr");
    for (chunk_id, ops) in world.chunk_ops.iter() {
        // source_form: chunks store their source in `:source` meta.
        // For V4 minimum viable we emit the chunk's own FormId — the
        // spec calls for the "source Form id"; rust's source is a
        // moof Form deep in the heap. Using the chunk-id itself is
        // a placeholder (moof-zig doesn't dispatch on this yet).
        buf.extend_from_slice(&chunk_id.0.to_be_bytes());
        // body: encoded ops
        let body = encode_chunk_ops(ops);
        buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
        buf.extend_from_slice(&body);
        // consts
        let consts = world.chunk_consts.get(chunk_id);
        let consts_len = consts.map(|v| v.len()).unwrap_or(0);
        buf.extend_from_slice(&(consts_len as u16).to_be_bytes());
        if let Some(cs) = consts {
            for c in cs {
                encode_value(*c, &mut buf);
            }
        }
        // ic_count
        let ic_count = world.chunk_ics.get(chunk_id).map(|v| v.len()).unwrap_or(0);
        buf.extend_from_slice(&(ic_count as u16).to_be_bytes());
        // params: walk the chunk-Form's :params slot list.
        let params_syms: Vec<SymId> = match (params_sym, car_sym, cdr_sym) {
            (Some(ps), Some(carp), Some(cdrp)) => {
                let chunk_form = world.heap.get(*chunk_id);
                let mut cur = chunk_form.slot(ps);
                let mut out = Vec::new();
                while let Value::Form(cell_id) = cur {
                    let cell = world.heap.get(cell_id);
                    if let Value::Sym(s) = cell.slot(carp) {
                        out.push(s);
                    }
                    cur = cell.slot(cdrp);
                }
                out
            }
            _ => Vec::new(),
        };
        buf.extend_from_slice(&(params_syms.len() as u16).to_be_bytes());
        for s in params_syms {
            buf.extend_from_slice(&(s.0 as u32).to_be_bytes());
        }
    }

    // ── NativeRefsSection ───────────────────────────────────────
    let natives = collect_native_methods(world);
    buf.extend_from_slice(&(natives.len() as u32).to_be_bytes());
    for (method_id, name) in natives {
        buf.extend_from_slice(&method_id.0.to_be_bytes());
        let bytes = name.as_bytes();
        // u8 length per spec; truncate names > 255 bytes (none exist
        // in practice — selectors max ~30 chars).
        let len = bytes.len().min(255) as u8;
        buf.push(len);
        buf.extend_from_slice(&bytes[..len as usize]);
    }

    // ── McoBindingsSection ──────────────────────────────────────
    // TODO(phase-D): re-emit wasm mco bindings. For V4 MVP, moof-zig
    // skips wasm at load (the rust runtime carries Hash mco bytes
    // embedded; zig will bootstrap its own copies).
    buf.extend_from_slice(&0_u32.to_be_bytes());

    // ── FarRefsSection ──────────────────────────────────────────
    // Single-vat — no far-refs.
    buf.extend_from_slice(&0_u32.to_be_bytes());

    // ── Footer: image hash ──────────────────────────────────────
    // TODO(phase-9): real blake3 hash of the bytes above. moof-zig
    // currently stubs verification.
    let hash = [0u8; 32];
    buf.extend_from_slice(&hash);

    buf
}
