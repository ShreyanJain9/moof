(* opcodes.ml — V4 opcode ADT.

   spec: docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md §2-§3.
   the byte tags below MUST match what the zig agent uses; the encoder in
   bytecode.ml is the byte-boundary contract. *)

type op =
  (* value load *)
  | PushNil               (* tag 0x01 *)
  | PushTrue              (* tag 0x02 *)
  | PushFalse             (* tag 0x03 *)
  | LoadConst of int      (* tag 0x04; u16 idx *)
  | LoadSelf              (* tag 0x05 *)
  | LoadHere              (* tag 0x06 *)
  | LoadName of int       (* tag 0x07; u32 SymId *)

  (* stack *)
  | Pop                   (* tag 0x10 *)
  | Dup                   (* tag 0x11 *)

  (* sends *)
  | Send of {             (* tag 0x20 *)
      selector : int;
      argc : int;
      ic_idx : int;
    }
  | TailSend of {         (* tag 0x21 *)
      selector : int;
      argc : int;
    }
  | SuperSend of {        (* tag 0x22 *)
      selector : int;
      argc : int;
      ic_idx : int;
    }
  | SendDynamic of {      (* tag 0x23 *)
      argc : int;
      ic_idx : int;
    }
  | SendSelf of {         (* tag 0x24 *)
      selector : int;
      argc : int;
      ic_idx : int;
    }
  | SendHere of {         (* tag 0x25 *)
      selector : int;
      argc : int;
      ic_idx : int;
    }
  | TailSendSelf of {     (* tag 0x26 *)
      selector : int;
      argc : int;
    }
  | TailSendHere of {     (* tag 0x27 *)
      selector : int;
      argc : int;
    }

  (* control flow *)
  | Jump of int           (* tag 0x30; i16 offset *)
  | JumpIfFalse of int    (* tag 0x31; i16 offset *)
  | JumpIfTrue of int     (* tag 0x32; i16 offset *)
  | Return                (* tag 0x33 *)

  (* closures *)
  | PushClosure of int    (* tag 0x40; u32 chunk-FormId *)

  (* scheduling (phase D placeholders) *)
  | Suspend of int        (* tag 0x50; u16 promise-ic *)
  | Resume of int         (* tag 0x51; u16 frame-ic *)

(* [pushes op] is [true] iff the op leaves +1 on the operand stack in
   isolation (ignoring stack-replacing tail variants).

   used by the compiler's running stack-balance check (spec §6.4).

   notes:
   - send variants pop receiver+args and push the result, net +1 only if
     argc < (1 + receiver-pop); we report the spec's `pushes:+1` here.
   - tail variants replace the frame; they don't add to *this* frame's
     stack, so they report [false].
   - Pop is -1, Return ends the frame; both [false].
   - Jump/JumpIfFalse/JumpIfTrue don't push (the cond-jumps pop). [false]. *)
let pushes : op -> bool = function
  | PushNil | PushTrue | PushFalse
  | LoadConst _ | LoadSelf | LoadHere | LoadName _
  | Dup
  | Send _ | SuperSend _ | SendDynamic _
  | SendSelf _ | SendHere _
  | PushClosure _ -> true
  | Pop
  | TailSend _ | TailSendSelf _ | TailSendHere _
  | Jump _ | JumpIfFalse _ | JumpIfTrue _
  | Return
  | Suspend _ | Resume _ -> false
