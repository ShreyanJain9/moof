(* V4 per-vat image serializer.

   Layout per spec §10.3 (docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md).
   File structure:
     Magic "MVAT" (4B) + Version u16 BE (= 0x0004)
     Header (vat_id + counts + here_form_id + macros_form_id + 18-proto-table
             + external_vat_refs)
     SymTableSection + FormSection + ChunkSection + NativeRefsSection
       + McoBindingsSection + FarRefsSection
     Footer: 32-byte BLAKE3 hash of everything above

   Value encoding inside FormSection (tag bytes per spec §4):
     0xC0 Nil      (1B)
     0xC1 Bool false (1B)
     0xC2 Bool true  (1B)
     0xC3 Int      (1+8B; i64 BE)
     0xC4 Sym      (1+4B; u32 BE SymId)
     0xC5 Char     (1+4B; u32 BE codepoint)
     0xC6 Float    (1+8B; f64 BE)
     0xC7 Form     (1+4B; u32 BE FormId)

   STRINGS, BYTES, CONS: encoded as Form-references — the vat heap holds the
     actual Form, and the value slot contains a Form(FormId) pointing to it.
     The caller (build-image path) is responsible for pre-allocating those Forms
     in the FormSection. *)

type proto_table = {
  object_id: int;
  nil_id: int;
  bool_id: int;
  integer_id: int;
  char_id: int;
  sym_id: int;
  cons_id: int;
  string_id: int;
  bytes_id: int;
  method_id: int;
  chunk_id: int;
  closure_id: int;
  env_id: int;
  foreign_handle_id: int;
  table_id: int;
  frame_id: int;
  macros_id: int;
  opcode_id: int;
}

let empty_protos : proto_table = {
  object_id = 0; nil_id = 0; bool_id = 0; integer_id = 0; char_id = 0;
  sym_id = 0; cons_id = 0; string_id = 0; bytes_id = 0; method_id = 0;
  chunk_id = 0; closure_id = 0; env_id = 0; foreign_handle_id = 0;
  table_id = 0; frame_id = 0; macros_id = 0; opcode_id = 0;
}

type vat_form = {
  proto: Ast.form;                       (* a Value — encoded inline *)
  slots: (int * Ast.form) list;          (* (sym_id, value); insertion order *)
  handlers: (int * Ast.form) list;
  meta: (int * Ast.form) list;
  frozen: bool;
}

type vat_chunk = {
  source_form_id: int;
  body: bytes;                            (* V4 byte-tagged bytecode *)
  consts: Ast.form list;
  ic_count: int;
  params: int list;                       (* sym ids *)
}

type native_ref = {
  method_form_id: int;
  native_name: string;                    (* e.g. "Object:+:" *)
}

type mco_binding = {
  mco_hash: bytes;                        (* 32 bytes blake3 *)
  proto_form_id: int;
}

type far_ref = {
  local_form_id: int;
  target_vat_id: bytes;                   (* 16 bytes *)
  target_form_id: int;
}

type vat_image = {
  vat_id: bytes;                          (* 16 bytes *)
  syms: string list;                      (* in interning order *)
  forms: vat_form list;                   (* in alloc order; first non-sentinel is FormId(1) *)
  chunks: vat_chunk list;
  natives: native_ref list;
  mcos: mco_binding list;
  far_refs: far_ref list;
  external_vat_refs: bytes list;          (* 16-byte vat ids referenced via far-refs *)
  here_form_id: int;
  macros_form_id: int;
  protos: proto_table;
}

(* ------- buffer helpers (mirror Bytecode.put_X) ------- *)

let put_bytes buf (b : bytes) = Buffer.add_bytes buf b

let put_u8  = Bytecode.write_u8
let put_u16 = Bytecode.write_u16_be
let put_u32 = Bytecode.write_u32_be
(* i64 + f64 helpers — not in Bytecode (which only deals with op
   operands up to u32). define locally here.

   Note: OCaml's native int is 63 bits (one bit stolen for the GC tag).
   We use `asr` (arithmetic shift right) rather than `lsr` so the sign
   bit propagates correctly through all eight emitted bytes — `lsr`
   would zero-fill from bit 63 and turn -1 into 0x7FFF_FFFF_FFFF_FFFF
   on the wire (which the zig loader then rejects as out-of-i48-range,
   because Value::Int is i48 ∪ BigInt and i64::MAX overflows i48).
   For positive values `asr` and `lsr` are identical, so the change is
   safe across the whole int range. *)
let put_i64 (b : Buffer.t) (v : int) : unit =
  for i = 7 downto 0 do
    Buffer.add_char b (Char.chr ((v asr (i * 8)) land 0xff))
  done

(* For Float we want the *raw* u64 bit pattern, no sign-extension —
   so go through Int64 directly rather than reusing put_i64 (which
   could only handle the 63-bit OCaml-int range anyway). *)
let put_f64 (b : Buffer.t) (v : float) : unit =
  let bits = Int64.bits_of_float v in
  for i = 7 downto 0 do
    let byte = Int64.to_int
                 (Int64.logand
                   (Int64.shift_right_logical bits (i * 8))
                   0xffL)
    in
    Buffer.add_char b (Char.chr byte)
  done

(* ------- Value (inline byte-tagged) encoder per spec §4 ------- *)

(* Encode a Form-as-Value. Strings/Bytes/Cons are NOT inline; they
   should be pre-allocated as Form entries in the FormSection, and
   the caller passes Form-references via the inline encoding. For
   strict correctness in this serializer's value path, we accept
   only "scalar" Ast.form values: Nil/Bool/Int/Float/Char/Sym.

   If a Cons/Str/Bytes/Vec sneaks in, we raise — it indicates the
   build-image pass didn't allocate it as a Form first. *)

(* SymId resolver: indexed by sym-table position (caller built it
   from `image.syms`). We use a hashtable for O(1) lookup.

   IMPORTANT: zig's sym table reserves SymId 0 as the NONE sentinel
   (see crates/zig-substrate/src/sym.zig — entries[0] = ""). After
   image-load, the first user-interned sym lands at SymId 1, the
   second at SymId 2, etc. So a sym at OCaml position `i` (0-based
   in `image.syms`) becomes SymId `i + 1` on the wire. *)
type sym_lookup = (string, int) Hashtbl.t

let build_sym_lookup (syms : string list) : sym_lookup =
  let h = Hashtbl.create (max 16 (List.length syms)) in
  List.iteri (fun i s -> Hashtbl.replace h s (i + 1)) syms;
  h

let resolve_sym (lookup : sym_lookup) (name : string) : int =
  match Hashtbl.find_opt lookup name with
  | Some i -> i
  | None ->
      failwith (Printf.sprintf
        "Image.encode_value: unknown symbol %S (not in vat sym table)" name)

(* Encode a scalar value with its byte tag. *)
let encode_value (buf : Buffer.t) (lookup : sym_lookup) (v : Ast.form) : unit =
  match v with
  | Ast.Nil -> put_u8 buf 0xC0
  | Ast.Bool false -> put_u8 buf 0xC1
  | Ast.Bool true -> put_u8 buf 0xC2
  | Ast.Int n ->
      put_u8 buf 0xC3;
      put_i64 buf n
  | Ast.Sym s ->
      put_u8 buf 0xC4;
      put_u32 buf (resolve_sym lookup s)
  | Ast.Char cp ->
      put_u8 buf 0xC5;
      put_u32 buf cp
  | Ast.Float f ->
      put_u8 buf 0xC6;
      put_f64 buf f
  | Ast.FormRef id ->
      put_u8 buf 0xC7;
      put_u32 buf id
  | Ast.Str _ | Ast.Bytes _ | Ast.Cons _ | Ast.Vec _ ->
      failwith "Image.encode_value: non-scalar Value must be allocated as Form first"

(* Form-reference value (caller already resolved to FormId). *)
let encode_value_form_ref (buf : Buffer.t) (form_id : int) : unit =
  put_u8 buf 0xC7;
  put_u32 buf form_id

(* ------- sections ------- *)

let write_header (buf : Buffer.t) (img : vat_image) : unit =
  (* vat_id [16]u8 *)
  if Bytes.length img.vat_id <> 16 then
    failwith (Printf.sprintf "Image.write_header: vat_id must be 16 bytes, got %d"
                (Bytes.length img.vat_id));
  put_bytes buf img.vat_id;
  put_u32 buf (List.length img.forms);
  put_u32 buf (List.length img.syms);
  put_u32 buf (List.length img.chunks);
  put_u32 buf img.here_form_id;
  put_u32 buf img.macros_form_id;
  (* protos: 18 × u32 in canonical order *)
  let p = img.protos in
  put_u32 buf p.object_id;
  put_u32 buf p.nil_id;
  put_u32 buf p.bool_id;
  put_u32 buf p.integer_id;
  put_u32 buf p.char_id;
  put_u32 buf p.sym_id;
  put_u32 buf p.cons_id;
  put_u32 buf p.string_id;
  put_u32 buf p.bytes_id;
  put_u32 buf p.method_id;
  put_u32 buf p.chunk_id;
  put_u32 buf p.closure_id;
  put_u32 buf p.env_id;
  put_u32 buf p.foreign_handle_id;
  put_u32 buf p.table_id;
  put_u32 buf p.frame_id;
  put_u32 buf p.macros_id;
  put_u32 buf p.opcode_id;
  (* external_vat_refs *)
  put_u16 buf (List.length img.external_vat_refs);
  List.iter (fun vid ->
    if Bytes.length vid <> 16 then
      failwith "Image.write_header: external_vat_ref id must be 16 bytes";
    put_bytes buf vid
  ) img.external_vat_refs

let write_sym_table (buf : Buffer.t) (syms : string list) : unit =
  put_u32 buf (List.length syms);
  List.iter (fun s ->
    let n = String.length s in
    if n > 0xffff then
      failwith (Printf.sprintf "Image.write_sym_table: symbol too long (%d bytes)" n);
    put_u16 buf n;
    Buffer.add_string buf s
  ) syms

let write_form_section (buf : Buffer.t) (lookup : sym_lookup)
                       (forms : vat_form list) : unit =
  put_u32 buf (List.length forms);
  List.iter (fun (f : vat_form) ->
    encode_value buf lookup f.proto;
    put_u16 buf (List.length f.slots);
    List.iter (fun (sid, v) ->
      put_u32 buf sid;
      encode_value buf lookup v
    ) f.slots;
    put_u16 buf (List.length f.handlers);
    List.iter (fun (sid, v) ->
      put_u32 buf sid;
      encode_value buf lookup v
    ) f.handlers;
    put_u16 buf (List.length f.meta);
    List.iter (fun (sid, v) ->
      put_u32 buf sid;
      encode_value buf lookup v
    ) f.meta;
    put_u8 buf (if f.frozen then 1 else 0)
  ) forms

let write_chunk_section (buf : Buffer.t) (lookup : sym_lookup)
                        (chunks : vat_chunk list) : unit =
  put_u32 buf (List.length chunks);
  List.iter (fun (c : vat_chunk) ->
    put_u32 buf c.source_form_id;
    put_u32 buf (Bytes.length c.body);
    put_bytes buf c.body;
    put_u16 buf (List.length c.consts);
    List.iter (fun v -> encode_value buf lookup v) c.consts;
    put_u16 buf c.ic_count;
    put_u16 buf (List.length c.params);
    List.iter (fun sid -> put_u32 buf sid) c.params
  ) chunks

let write_native_refs (buf : Buffer.t) (natives : native_ref list) : unit =
  put_u32 buf (List.length natives);
  List.iter (fun (n : native_ref) ->
    put_u32 buf n.method_form_id;
    let len = String.length n.native_name in
    if len > 0xff then
      failwith (Printf.sprintf "Image.write_native_refs: name too long (%d bytes)" len);
    put_u8 buf len;
    Buffer.add_string buf n.native_name
  ) natives

let write_mco_bindings (buf : Buffer.t) (mcos : mco_binding list) : unit =
  put_u32 buf (List.length mcos);
  List.iter (fun (m : mco_binding) ->
    if Bytes.length m.mco_hash <> 32 then
      failwith "Image.write_mco_bindings: mco_hash must be 32 bytes";
    put_bytes buf m.mco_hash;
    put_u32 buf m.proto_form_id
  ) mcos

let write_far_refs (buf : Buffer.t) (fars : far_ref list) : unit =
  put_u32 buf (List.length fars);
  List.iter (fun (f : far_ref) ->
    put_u32 buf f.local_form_id;
    if Bytes.length f.target_vat_id <> 16 then
      failwith "Image.write_far_refs: target_vat_id must be 16 bytes";
    put_bytes buf f.target_vat_id;
    put_u32 buf f.target_form_id
  ) fars

(* ------- top-level serialize ------- *)

let compute_image_hash (without_footer : bytes) : bytes =
  Blake3.hash without_footer

let serialize (img : vat_image) : bytes =
  let lookup = build_sym_lookup img.syms in
  let buf = Buffer.create 4096 in
  (* Magic + Version *)
  Buffer.add_string buf "MVAT";
  put_u16 buf 0x0004;
  (* Header *)
  write_header buf img;
  (* Sections *)
  write_sym_table buf img.syms;
  write_form_section buf lookup img.forms;
  write_chunk_section buf lookup img.chunks;
  write_native_refs buf img.natives;
  write_mco_bindings buf img.mcos;
  write_far_refs buf img.far_refs;
  (* Footer = hash of everything above *)
  let so_far = Buffer.to_bytes buf in
  let h = compute_image_hash so_far in
  Buffer.add_bytes buf h;
  Buffer.to_bytes buf
