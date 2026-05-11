(* V4 bytecode encoder/decoder.

   Byte layout per spec §4. All multi-byte operands big-endian.
   This produces the byte sequence the zig substrate executes
   directly off byte indices.

   STATUS: stub for parallel agent dev. OCAML-2 owns the canonical version. *)

open Opcodes

(* helpers — write big-endian integers into a Buffer. *)
let put_u8 buf v = Buffer.add_char buf (Char.chr (v land 0xff))

let put_u16 buf v =
  Buffer.add_char buf (Char.chr ((v lsr 8) land 0xff));
  Buffer.add_char buf (Char.chr (v land 0xff))

let put_i16 buf v =
  (* OCaml int → two-byte big-endian, signed. *)
  let v = if v < 0 then v + 0x10000 else v in
  put_u16 buf v

let put_u32 buf v =
  Buffer.add_char buf (Char.chr ((v lsr 24) land 0xff));
  Buffer.add_char buf (Char.chr ((v lsr 16) land 0xff));
  Buffer.add_char buf (Char.chr ((v lsr 8) land 0xff));
  Buffer.add_char buf (Char.chr (v land 0xff))

let put_i64 buf v =
  for i = 7 downto 0 do
    Buffer.add_char buf (Char.chr ((v lsr (i * 8)) land 0xff))
  done

let put_f64 buf (f : float) =
  let bits = Int64.bits_of_float f in
  for i = 7 downto 0 do
    let b = Int64.to_int (Int64.logand (Int64.shift_right_logical bits (i * 8)) 0xffL) in
    Buffer.add_char buf (Char.chr b)
  done

(* readers — for the decoder. *)
let get_u8 b off = Char.code (Bytes.get b off)
let get_u16 b off = (get_u8 b off lsl 8) lor get_u8 b (off + 1)
let get_i16 b off =
  let v = get_u16 b off in
  if v >= 0x8000 then v - 0x10000 else v
let get_u32 b off =
  (get_u8 b off lsl 24)
  lor (get_u8 b (off + 1) lsl 16)
  lor (get_u8 b (off + 2) lsl 8)
  lor get_u8 b (off + 3)

(* encode one op to a buffer. *)
let encode_op (buf : Buffer.t) (op : op) : unit =
  match op with
  | PushNil -> put_u8 buf tag_push_nil
  | PushTrue -> put_u8 buf tag_push_true
  | PushFalse -> put_u8 buf tag_push_false
  | LoadConst idx ->
      put_u8 buf tag_load_const;
      put_u16 buf idx
  | LoadSelf -> put_u8 buf tag_load_self
  | LoadHere -> put_u8 buf tag_load_here
  | LoadName s ->
      put_u8 buf tag_load_name;
      put_u32 buf s
  | Pop -> put_u8 buf tag_pop
  | Dup -> put_u8 buf tag_dup
  | Send (s, argc, ic) ->
      put_u8 buf tag_send;
      put_u32 buf s;
      put_u8 buf argc;
      put_u16 buf ic
  | TailSend (s, argc) ->
      put_u8 buf tag_tail_send;
      put_u32 buf s;
      put_u8 buf argc
  | SuperSend (s, argc, ic) ->
      put_u8 buf tag_super_send;
      put_u32 buf s;
      put_u8 buf argc;
      put_u16 buf ic
  | SendDynamic (argc, ic) ->
      put_u8 buf tag_send_dynamic;
      put_u8 buf argc;
      put_u16 buf ic
  | SendSelf (s, argc, ic) ->
      put_u8 buf tag_send_self;
      put_u32 buf s;
      put_u8 buf argc;
      put_u16 buf ic
  | SendHere (s, argc, ic) ->
      put_u8 buf tag_send_here;
      put_u32 buf s;
      put_u8 buf argc;
      put_u16 buf ic
  | TailSendSelf (s, argc) ->
      put_u8 buf tag_tail_send_self;
      put_u32 buf s;
      put_u8 buf argc
  | TailSendHere (s, argc) ->
      put_u8 buf tag_tail_send_here;
      put_u32 buf s;
      put_u8 buf argc
  | Jump off ->
      put_u8 buf tag_jump;
      put_i16 buf off
  | JumpIfFalse off ->
      put_u8 buf tag_jump_if_false;
      put_i16 buf off
  | JumpIfTrue off ->
      put_u8 buf tag_jump_if_true;
      put_i16 buf off
  | Return -> put_u8 buf tag_return
  | PushClosure c ->
      put_u8 buf tag_push_closure;
      put_u32 buf c
  | Suspend ic ->
      put_u8 buf tag_suspend;
      put_u16 buf ic
  | Resume ic ->
      put_u8 buf tag_resume;
      put_u16 buf ic

let encode_ops (ops : op list) : bytes =
  let buf = Buffer.create 64 in
  List.iter (encode_op buf) ops;
  Buffer.to_bytes buf

(* decode a byte-encoded chunk body into structured ops. used by
   `seed compile` for disassembly. *)
let decode_ops (body : bytes) : op list =
  let n = Bytes.length body in
  let rec go off acc =
    if off >= n then List.rev acc
    else
      let tag = get_u8 body off in
      let off = off + 1 in
      if tag = tag_push_nil then go off (PushNil :: acc)
      else if tag = tag_push_true then go off (PushTrue :: acc)
      else if tag = tag_push_false then go off (PushFalse :: acc)
      else if tag = tag_load_const then
        let idx = get_u16 body off in
        go (off + 2) (LoadConst idx :: acc)
      else if tag = tag_load_self then go off (LoadSelf :: acc)
      else if tag = tag_load_here then go off (LoadHere :: acc)
      else if tag = tag_load_name then
        let s = get_u32 body off in
        go (off + 4) (LoadName s :: acc)
      else if tag = tag_pop then go off (Pop :: acc)
      else if tag = tag_dup then go off (Dup :: acc)
      else if tag = tag_send then
        let s = get_u32 body off in
        let argc = get_u8 body (off + 4) in
        let ic = get_u16 body (off + 5) in
        go (off + 7) (Send (s, argc, ic) :: acc)
      else if tag = tag_tail_send then
        let s = get_u32 body off in
        let argc = get_u8 body (off + 4) in
        go (off + 5) (TailSend (s, argc) :: acc)
      else if tag = tag_super_send then
        let s = get_u32 body off in
        let argc = get_u8 body (off + 4) in
        let ic = get_u16 body (off + 5) in
        go (off + 7) (SuperSend (s, argc, ic) :: acc)
      else if tag = tag_send_dynamic then
        let argc = get_u8 body off in
        let ic = get_u16 body (off + 1) in
        go (off + 3) (SendDynamic (argc, ic) :: acc)
      else if tag = tag_send_self then
        let s = get_u32 body off in
        let argc = get_u8 body (off + 4) in
        let ic = get_u16 body (off + 5) in
        go (off + 7) (SendSelf (s, argc, ic) :: acc)
      else if tag = tag_send_here then
        let s = get_u32 body off in
        let argc = get_u8 body (off + 4) in
        let ic = get_u16 body (off + 5) in
        go (off + 7) (SendHere (s, argc, ic) :: acc)
      else if tag = tag_tail_send_self then
        let s = get_u32 body off in
        let argc = get_u8 body (off + 4) in
        go (off + 5) (TailSendSelf (s, argc) :: acc)
      else if tag = tag_tail_send_here then
        let s = get_u32 body off in
        let argc = get_u8 body (off + 4) in
        go (off + 5) (TailSendHere (s, argc) :: acc)
      else if tag = tag_jump then
        let o = get_i16 body off in
        go (off + 2) (Jump o :: acc)
      else if tag = tag_jump_if_false then
        let o = get_i16 body off in
        go (off + 2) (JumpIfFalse o :: acc)
      else if tag = tag_jump_if_true then
        let o = get_i16 body off in
        go (off + 2) (JumpIfTrue o :: acc)
      else if tag = tag_return then go off (Return :: acc)
      else if tag = tag_push_closure then
        let c = get_u32 body off in
        go (off + 4) (PushClosure c :: acc)
      else if tag = tag_suspend then
        let ic = get_u16 body off in
        go (off + 2) (Suspend ic :: acc)
      else if tag = tag_resume then
        let ic = get_u16 body off in
        go (off + 2) (Resume ic :: acc)
      else
        failwith (Printf.sprintf "Bytecode.decode_ops: unknown tag 0x%02x at off %d" tag (off - 1))
  in
  go 0 []

(* hex dump of a byte string — utility for the CLI. *)
let to_hex (b : bytes) : string =
  let n = Bytes.length b in
  let buf = Buffer.create (n * 2) in
  for i = 0 to n - 1 do
    Buffer.add_string buf (Printf.sprintf "%02x" (Char.code (Bytes.get b i)))
  done;
  Buffer.contents buf
