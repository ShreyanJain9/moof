(* test_bytecode.ml — roundtrip the V4 opcodes through encode/decode.

   covers every opcode variant + boundary values for the operand widths:
   - LoadConst 0 and 65535 (u16 boundary)
   - i16 jump offsets: positive, max-positive, zero, negative, max-negative
   - u32 selectors / SymIds / FormIds: small + large

   if this passes, the OCaml encoder is byte-identical for any sequence we
   put through it. cross-encoder identity with the zig agent is checked
   separately (a few golden-byte fixtures wired up later). *)

open Moof_seed.Opcodes
open Moof_seed.Bytecode

let test_roundtrip () =
  let ops = [
    PushNil; PushTrue; PushFalse;
    LoadConst 0; LoadConst 65535;
    LoadSelf; LoadHere;
    LoadName 12345;
    LoadName 0xdeadbeef;
    Pop; Dup;
    Send { selector = 100; argc = 2; ic_idx = 5 };
    TailSend { selector = 100; argc = 2 };
    SuperSend { selector = 200; argc = 0; ic_idx = 7 };
    SendDynamic { argc = 1; ic_idx = 3 };
    SendSelf { selector = 50; argc = 1; ic_idx = 9 };
    SendHere { selector = 60; argc = 2; ic_idx = 4 };
    TailSendSelf { selector = 70; argc = 0 };
    TailSendHere { selector = 80; argc = 3 };
    Jump 100;
    Jump 32767;       (* i16 max-positive *)
    Jump 0;
    JumpIfFalse (-50);
    JumpIfFalse (-32768);  (* i16 max-negative *)
    JumpIfTrue 200;
    JumpIfTrue (-1);
    Return;
    PushClosure 9999;
    PushClosure 0xfeedface;
    Suspend 0; Resume 1;
    Suspend 65535;
  ] in
  let bytes = encode_ops ops in
  let decoded = decode_ops bytes in
  assert (decoded = ops);
  Printf.printf "Roundtrip OK: %d ops, %d bytes\n"
    (List.length ops) (Bytes.length bytes)

(* spot-check the exact byte layout from spec §4.2:
     Send {selector=0x1234abcd, argc=2, ic_idx=4} =
     0x20  0x12 0x34 0xab 0xcd  0x02  0x00 0x04 *)
let test_send_layout () =
  let b = Buffer.create 8 in
  encode_op (Send { selector = 0x1234abcd; argc = 2; ic_idx = 4 }) b;
  let got = Buffer.to_bytes b in
  let want = Bytes.of_string "\x20\x12\x34\xab\xcd\x02\x00\x04" in
  assert (Bytes.equal got want);
  Printf.printf "Send layout matches spec §4.2 example\n"

(* spot-check i16 negative encoding: Jump (-1) should encode as
   0x30 0xff 0xff (two's-complement). *)
let test_jump_negative_layout () =
  let b = Buffer.create 3 in
  encode_op (Jump (-1)) b;
  let got = Buffer.to_bytes b in
  let want = Bytes.of_string "\x30\xff\xff" in
  assert (Bytes.equal got want);
  let op, consumed = decode_op got 0 in
  assert (consumed = 3);
  assert (op = Jump (-1));
  Printf.printf "Jump (-1) i16 two's-complement OK\n"

let () =
  test_roundtrip ();
  test_send_layout ();
  test_jump_negative_layout ();
  print_endline "All bytecode tests passed."
