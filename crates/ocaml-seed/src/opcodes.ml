(* V4 opcodes — byte tags and structured ops.

   Matches spec §3 / §4 (docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md).
   Tag bytes and operand layout are LOAD-BEARING — the zig deserializer reads
   the same bytes.

   STATUS: stub for parallel agent dev. OCAML-2 owns the canonical version. *)

(* op tag bytes per spec §4.1. *)
let tag_push_nil       = 0x01
let tag_push_true      = 0x02
let tag_push_false     = 0x03
let tag_load_const     = 0x04
let tag_load_self      = 0x05
let tag_load_here      = 0x06
let tag_load_name      = 0x07

let tag_pop            = 0x10
let tag_dup            = 0x11

let tag_send           = 0x20
let tag_tail_send      = 0x21
let tag_super_send     = 0x22
let tag_send_dynamic   = 0x23
let tag_send_self      = 0x24
let tag_send_here      = 0x25
let tag_tail_send_self = 0x26
let tag_tail_send_here = 0x27

let tag_jump           = 0x30
let tag_jump_if_false  = 0x31
let tag_jump_if_true   = 0x32
let tag_return         = 0x33

let tag_push_closure   = 0x40

let tag_suspend        = 0x50
let tag_resume         = 0x51

(* structured op — used by the assembler/disassembler. selectors and
   names are SymId (u32) — at the OCaml-seed level we resolve them lazily
   when serializing chunks; the structured op carries the *string* (the sym
   name) and the bytecode encoder substitutes the sym id at serialize time.
   for the standalone seed CLI, the disassembler reads u32s and prints them
   as `sym#N` if the symbol table isn't known. *)
type op =
  | PushNil
  | PushTrue
  | PushFalse
  | LoadConst of int          (* u16 idx *)
  | LoadSelf
  | LoadHere
  | LoadName of int           (* u32 SymId *)
  | Pop
  | Dup
  | Send of int * int * int          (* sel:u32 SymId, argc:u8, ic:u16 *)
  | TailSend of int * int            (* sel:u32 SymId, argc:u8 *)
  | SuperSend of int * int * int     (* sel:u32 SymId, argc:u8, ic:u16 *)
  | SendDynamic of int * int         (* argc:u8, ic:u16 *)
  | SendSelf of int * int * int      (* sel:u32, argc:u8, ic:u16 *)
  | SendHere of int * int * int      (* sel:u32, argc:u8, ic:u16 *)
  | TailSendSelf of int * int        (* sel:u32, argc:u8 *)
  | TailSendHere of int * int        (* sel:u32, argc:u8 *)
  | Jump of int               (* i16 *)
  | JumpIfFalse of int        (* i16 *)
  | JumpIfTrue of int         (* i16 *)
  | Return
  | PushClosure of int        (* u32 FormId *)
  | Suspend of int            (* u16 promise-ic *)
  | Resume of int             (* u16 frame-ic *)

(* smalltalk-style disassembly — per spec §7.2. *)
let show_op (op : op) : string =
  match op with
  | PushNil -> "PushNil"
  | PushTrue -> "PushTrue"
  | PushFalse -> "PushFalse"
  | LoadConst idx -> Printf.sprintf "LoadConst idx=%d" idx
  | LoadSelf -> "LoadSelf"
  | LoadHere -> "LoadHere"
  | LoadName s -> Printf.sprintf "LoadName name=sym#%d" s
  | Pop -> "Pop"
  | Dup -> "Dup"
  | Send (s, argc, ic) ->
      Printf.sprintf "Send :sym#%d argc=%d ic=%d" s argc ic
  | TailSend (s, argc) ->
      Printf.sprintf "TailSend :sym#%d argc=%d" s argc
  | SuperSend (s, argc, ic) ->
      Printf.sprintf "SuperSend :sym#%d argc=%d ic=%d" s argc ic
  | SendDynamic (argc, ic) ->
      Printf.sprintf "SendDynamic argc=%d ic=%d" argc ic
  | SendSelf (s, argc, ic) ->
      Printf.sprintf "SendSelf :sym#%d argc=%d ic=%d" s argc ic
  | SendHere (s, argc, ic) ->
      Printf.sprintf "SendHere :sym#%d argc=%d ic=%d" s argc ic
  | TailSendSelf (s, argc) ->
      Printf.sprintf "TailSendSelf :sym#%d argc=%d" s argc
  | TailSendHere (s, argc) ->
      Printf.sprintf "TailSendHere :sym#%d argc=%d" s argc
  | Jump o -> Printf.sprintf "Jump offset=%d" o
  | JumpIfFalse o -> Printf.sprintf "JumpIfFalse offset=%d" o
  | JumpIfTrue o -> Printf.sprintf "JumpIfTrue offset=%d" o
  | Return -> "Return"
  | PushClosure c -> Printf.sprintf "PushClosure chunk=%d" c
  | Suspend ic -> Printf.sprintf "Suspend promise-ic=%d" ic
  | Resume ic -> Printf.sprintf "Resume frame-ic=%d" ic
