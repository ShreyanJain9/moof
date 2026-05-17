(* bytecode.ml — V4 byte encoder + decoder.

   spec: docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md §3-§4.

   layout (all multi-byte operands big-endian):

     op                          | tag  | operand bytes        | total
     --------------------------- | ---- | -------------------- | -----
     PushNil                     | 0x01 | -                    | 1
     PushTrue                    | 0x02 | -                    | 1
     PushFalse                   | 0x03 | -                    | 1
     LoadConst {idx}             | 0x04 | u16                  | 3
     LoadSelf                    | 0x05 | -                    | 1
     LoadHere                    | 0x06 | -                    | 1
     LoadName {name}             | 0x07 | u32                  | 5
     Pop                         | 0x10 | -                    | 1
     Dup                         | 0x11 | -                    | 1
     Send {sel,argc,ic}          | 0x20 | u32 + u8 + u16       | 8
     TailSend {sel,argc}         | 0x21 | u32 + u8             | 6
     SuperSend {sel,argc,ic}     | 0x22 | u32 + u8 + u16       | 8
     SendDynamic {argc,ic}       | 0x23 | u8 + u16             | 4
     SendSelf {sel,argc,ic}      | 0x24 | u32 + u8 + u16       | 8
     SendHere {sel,argc,ic}      | 0x25 | u32 + u8 + u16       | 8
     TailSendSelf {sel,argc}     | 0x26 | u32 + u8             | 6
     TailSendHere {sel,argc}     | 0x27 | u32 + u8             | 6
     Jump {off}                  | 0x30 | i16                  | 3
     JumpIfFalse {off}           | 0x31 | i16                  | 3
     JumpIfTrue {off}            | 0x32 | i16                  | 3
     Return                      | 0x33 | -                    | 1
     PushClosure {chunk}         | 0x40 | u32                  | 5
     Suspend {promise-ic}        | 0x50 | u16                  | 3
     Resume {frame-ic}           | 0x51 | u16                  | 3
*)

open Opcodes

(* ===========================================================
   write helpers — append to a Buffer.t in big-endian byte order
   =========================================================== *)

let write_u8 (b : Buffer.t) (v : int) : unit =
  Buffer.add_uint8 b (v land 0xff)

let write_u16_be (b : Buffer.t) (v : int) : unit =
  Buffer.add_uint16_be b (v land 0xffff)

let write_u32_be (b : Buffer.t) (v : int) : unit =
  (* OCaml [Buffer.add_int32_be] takes an [int32]; we mask to 32 bits
     so a host-int caller can pass any non-negative int up to 2^32-1. *)
  let lo = Int32.of_int (v land 0xffff) in
  let hi = Int32.of_int ((v lsr 16) land 0xffff) in
  let n = Int32.logor (Int32.shift_left hi 16) lo in
  Buffer.add_int32_be b n

(* i16: caller may pass any int in [-32768, 32767]. we mask to 16 bits
   so the on-disk byte pattern is two's-complement and unambiguous;
   reading sign-extends back. *)
let write_i16_be (b : Buffer.t) (v : int) : unit =
  Buffer.add_uint16_be b (v land 0xffff)

(* =========================================================
   read helpers — read big-endian operands from a bytes buffer
   ========================================================= *)

let read_u8 (bs : bytes) (pos : int) : int =
  Bytes.get_uint8 bs pos

let read_u16_be (bs : bytes) (pos : int) : int =
  Bytes.get_uint16_be bs pos

let read_u32_be (bs : bytes) (pos : int) : int =
  let n = Bytes.get_int32_be bs pos in
  (* widen to OCaml int (>=63-bit on 64-bit hosts). avoid sign-extension
     so the result is in [0, 2^32 - 1]. *)
  Int32.to_int n land 0xffffffff

(* sign-extend a 16-bit unsigned reading into a host int in
   [-32768, 32767]. *)
let read_i16_be (bs : bytes) (pos : int) : int =
  let u = Bytes.get_uint16_be bs pos in
  if u >= 0x8000 then u - 0x10000 else u

(* ====================
   per-op tag constants
   ==================== *)

let tag_PushNil       = 0x01
let tag_PushTrue      = 0x02
let tag_PushFalse     = 0x03
let tag_LoadConst     = 0x04
let tag_LoadSelf      = 0x05
let tag_LoadHere      = 0x06
let tag_LoadName      = 0x07
let tag_Pop           = 0x10
let tag_Dup           = 0x11
let tag_Send          = 0x20
let tag_TailSend      = 0x21
let tag_SuperSend     = 0x22
let tag_SendDynamic   = 0x23
let tag_SendSelf      = 0x24
let tag_SendHere      = 0x25
let tag_TailSendSelf  = 0x26
let tag_TailSendHere  = 0x27
let tag_Jump          = 0x30
let tag_JumpIfFalse   = 0x31
let tag_JumpIfTrue    = 0x32
let tag_Return        = 0x33
let tag_PushClosure   = 0x40
let tag_Suspend       = 0x50
let tag_Resume        = 0x51

(* ==========
   encode_op
   ========== *)

let encode_op (op : op) (b : Buffer.t) : unit =
  match op with
  | PushNil   -> write_u8 b tag_PushNil
  | PushTrue  -> write_u8 b tag_PushTrue
  | PushFalse -> write_u8 b tag_PushFalse
  | LoadConst idx ->
    write_u8 b tag_LoadConst;
    write_u16_be b idx
  | LoadSelf -> write_u8 b tag_LoadSelf
  | LoadHere -> write_u8 b tag_LoadHere
  | LoadName name ->
    write_u8 b tag_LoadName;
    write_u32_be b name
  | Pop -> write_u8 b tag_Pop
  | Dup -> write_u8 b tag_Dup
  | Send { selector; argc; ic_idx } ->
    write_u8 b tag_Send;
    write_u32_be b selector;
    write_u8 b argc;
    write_u16_be b ic_idx
  | TailSend { selector; argc } ->
    write_u8 b tag_TailSend;
    write_u32_be b selector;
    write_u8 b argc
  | SuperSend { selector; argc; ic_idx } ->
    write_u8 b tag_SuperSend;
    write_u32_be b selector;
    write_u8 b argc;
    write_u16_be b ic_idx
  | SendDynamic { argc; ic_idx } ->
    write_u8 b tag_SendDynamic;
    write_u8 b argc;
    write_u16_be b ic_idx
  | SendSelf { selector; argc; ic_idx } ->
    write_u8 b tag_SendSelf;
    write_u32_be b selector;
    write_u8 b argc;
    write_u16_be b ic_idx
  | SendHere { selector; argc; ic_idx } ->
    write_u8 b tag_SendHere;
    write_u32_be b selector;
    write_u8 b argc;
    write_u16_be b ic_idx
  | TailSendSelf { selector; argc } ->
    write_u8 b tag_TailSendSelf;
    write_u32_be b selector;
    write_u8 b argc
  | TailSendHere { selector; argc } ->
    write_u8 b tag_TailSendHere;
    write_u32_be b selector;
    write_u8 b argc
  | Jump off ->
    write_u8 b tag_Jump;
    write_i16_be b off
  | JumpIfFalse off ->
    write_u8 b tag_JumpIfFalse;
    write_i16_be b off
  | JumpIfTrue off ->
    write_u8 b tag_JumpIfTrue;
    write_i16_be b off
  | Return -> write_u8 b tag_Return
  | PushClosure chunk ->
    write_u8 b tag_PushClosure;
    write_u32_be b chunk
  | Suspend ic ->
    write_u8 b tag_Suspend;
    write_u16_be b ic
  | Resume ic ->
    write_u8 b tag_Resume;
    write_u16_be b ic

let encode_ops (ops : op list) : bytes =
  let b = Buffer.create (List.length ops * 4) in
  List.iter (fun op -> encode_op op b) ops;
  Buffer.to_bytes b

(* ==========
   decode_op
   ========== *)

exception Bad_opcode of int * int  (* (tag, position) *)

let decode_op (bs : bytes) (pos : int) : op * int =
  let tag = read_u8 bs pos in
  match tag with
  | 0x01 -> PushNil,   1
  | 0x02 -> PushTrue,  1
  | 0x03 -> PushFalse, 1
  | 0x04 ->
    let idx = read_u16_be bs (pos + 1) in
    LoadConst idx, 3
  | 0x05 -> LoadSelf, 1
  | 0x06 -> LoadHere, 1
  | 0x07 ->
    let name = read_u32_be bs (pos + 1) in
    LoadName name, 5
  | 0x10 -> Pop, 1
  | 0x11 -> Dup, 1
  | 0x20 ->
    let selector = read_u32_be bs (pos + 1) in
    let argc     = read_u8     bs (pos + 5) in
    let ic_idx   = read_u16_be bs (pos + 6) in
    Send { selector; argc; ic_idx }, 8
  | 0x21 ->
    let selector = read_u32_be bs (pos + 1) in
    let argc     = read_u8     bs (pos + 5) in
    TailSend { selector; argc }, 6
  | 0x22 ->
    let selector = read_u32_be bs (pos + 1) in
    let argc     = read_u8     bs (pos + 5) in
    let ic_idx   = read_u16_be bs (pos + 6) in
    SuperSend { selector; argc; ic_idx }, 8
  | 0x23 ->
    let argc   = read_u8     bs (pos + 1) in
    let ic_idx = read_u16_be bs (pos + 2) in
    SendDynamic { argc; ic_idx }, 4
  | 0x24 ->
    let selector = read_u32_be bs (pos + 1) in
    let argc     = read_u8     bs (pos + 5) in
    let ic_idx   = read_u16_be bs (pos + 6) in
    SendSelf { selector; argc; ic_idx }, 8
  | 0x25 ->
    let selector = read_u32_be bs (pos + 1) in
    let argc     = read_u8     bs (pos + 5) in
    let ic_idx   = read_u16_be bs (pos + 6) in
    SendHere { selector; argc; ic_idx }, 8
  | 0x26 ->
    let selector = read_u32_be bs (pos + 1) in
    let argc     = read_u8     bs (pos + 5) in
    TailSendSelf { selector; argc }, 6
  | 0x27 ->
    let selector = read_u32_be bs (pos + 1) in
    let argc     = read_u8     bs (pos + 5) in
    TailSendHere { selector; argc }, 6
  | 0x30 ->
    let off = read_i16_be bs (pos + 1) in
    Jump off, 3
  | 0x31 ->
    let off = read_i16_be bs (pos + 1) in
    JumpIfFalse off, 3
  | 0x32 ->
    let off = read_i16_be bs (pos + 1) in
    JumpIfTrue off, 3
  | 0x33 -> Return, 1
  | 0x40 ->
    let chunk = read_u32_be bs (pos + 1) in
    PushClosure chunk, 5
  | 0x50 ->
    let ic = read_u16_be bs (pos + 1) in
    Suspend ic, 3
  | 0x51 ->
    let ic = read_u16_be bs (pos + 1) in
    Resume ic, 3
  | _ -> raise (Bad_opcode (tag, pos))

let decode_ops (bs : bytes) : op list =
  let len = Bytes.length bs in
  let rec loop pos acc =
    if pos >= len then List.rev acc
    else
      let op, consumed = decode_op bs pos in
      loop (pos + consumed) (op :: acc)
  in
  loop 0 []
