(* moof-seed CLI entry point.

   spec: docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md
   plan: docs/superpowers/plans/2026-05-10-vm-V4-polyglot-substrate.md (Track B.6)

   subcommands:
     moof-seed parse <file.moof>
       parse + print AST forms.

     moof-seed compile <file.moof>
       parse + compile each top-level form. for each, print disassembly,
       hex byte dump, and const pool.

     moof-seed bytes <file.moof>
       parse + compile, emit ONLY the raw V4 bytecode bytes to stdout
       (binary). for multi-form sources, all chunk bodies are concatenated.
       intended for piping into moof-zig.

     moof-seed build-image <file.moof> <out.vat>
       parse + compile + serialize as a per-vat V4 image. V4-α scope
       (Path B): produces a minimum image — empty Forms / Natives / Mcos /
       FarRefs sections, but a real SymTable + ChunkSection. won't
       bootstrap on its own; useful for round-trip testing the serializer
       against moof-zig's deserializer. *)

open Moof_seed

(* ── file io ────────────────────────────────────────────────────── *)

let read_file (path : string) : string =
  let ic = open_in path in
  let n = in_channel_length ic in
  let buf = Bytes.create n in
  really_input ic buf 0 n;
  close_in ic;
  Bytes.to_string buf

(* ── op pretty-printer ─────────────────────────────────────────────
   Opcodes module exposes no show_op; provide a local disassembler
   that names selectors / consts where possible.  spec §3 byte layout
   names are preserved verbatim. *)

let show_sym (sid : int) : string =
  (* sym ids come from Compiler.sym_table; lookup may fail if the
     compiler state was reset between compile and disasm, so guard. *)
  try Printf.sprintf ":%s" (Compiler.sym_name sid)
  with _ -> Printf.sprintf "sym#%d" sid

let show_op (op : Opcodes.op) : string =
  match op with
  | PushNil    -> "PushNil"
  | PushTrue   -> "PushTrue"
  | PushFalse  -> "PushFalse"
  | LoadConst idx -> Printf.sprintf "LoadConst   const[%d]" idx
  | LoadSelf   -> "LoadSelf"
  | LoadHere   -> "LoadHere"
  | LoadName sid -> Printf.sprintf "LoadName    %s" (show_sym sid)
  | Pop -> "Pop"
  | Dup -> "Dup"
  | Send { selector; argc; ic_idx } ->
      Printf.sprintf "Send        %s argc=%d ic=%d"
        (show_sym selector) argc ic_idx
  | TailSend { selector; argc } ->
      Printf.sprintf "TailSend    %s argc=%d" (show_sym selector) argc
  | SuperSend { selector; argc; ic_idx } ->
      Printf.sprintf "SuperSend   %s argc=%d ic=%d"
        (show_sym selector) argc ic_idx
  | SendDynamic { argc; ic_idx } ->
      Printf.sprintf "SendDynamic argc=%d ic=%d" argc ic_idx
  | SendSelf { selector; argc; ic_idx } ->
      Printf.sprintf "SendSelf    %s argc=%d ic=%d"
        (show_sym selector) argc ic_idx
  | SendHere { selector; argc; ic_idx } ->
      Printf.sprintf "SendHere    %s argc=%d ic=%d"
        (show_sym selector) argc ic_idx
  | TailSendSelf { selector; argc } ->
      Printf.sprintf "TailSendSelf %s argc=%d" (show_sym selector) argc
  | TailSendHere { selector; argc } ->
      Printf.sprintf "TailSendHere %s argc=%d" (show_sym selector) argc
  | Jump off        -> Printf.sprintf "Jump        %+d" off
  | JumpIfFalse off -> Printf.sprintf "JumpIfFalse %+d" off
  | JumpIfTrue off  -> Printf.sprintf "JumpIfTrue  %+d" off
  | Return -> "Return"
  | PushClosure chunk_id ->
      Printf.sprintf "PushClosure chunk#%d" chunk_id
  | Suspend ic -> Printf.sprintf "Suspend     promise-ic=%d" ic
  | Resume ic  -> Printf.sprintf "Resume      frame-ic=%d"   ic

let hex_dump (b : bytes) : string =
  let buf = Buffer.create (Bytes.length b * 3) in
  Bytes.iter (fun c -> Buffer.add_string buf (Printf.sprintf "%02x " (Char.code c))) b;
  Buffer.contents buf

(* ── subcommands ───────────────────────────────────────────────── *)

let cmd_parse (path : string) : unit =
  let text = read_file path in
  let forms = Reader.read_all text in
  List.iter (fun f -> print_endline (Ast.to_string f)) forms

let cmd_compile (path : string) : unit =
  Compiler.reset_globals ();
  let text = read_file path in
  let forms = Reader.read_all text in
  List.iter (fun form ->
    let cb = Compiler.compile_top form in
    let final = Compiler.finalize cb in
    Printf.printf "=== form: %s ===\n" (Ast.to_string form);
    Printf.printf "chunk_id: %d  ic_count: %d  bytes: %d\n"
      final.chunk_id final.ic_count (Bytes.length final.body);
    (* disassemble *)
    let ops = Bytecode.decode_ops final.body in
    List.iter (fun op -> print_endline ("  " ^ show_op op)) ops;
    Printf.printf "hex: %s\n" (hex_dump final.body);
    if final.consts <> [] then begin
      print_endline "consts:";
      List.iteri (fun i c -> Printf.printf "  [%d] %s\n" i (Ast.to_string c)) final.consts
    end;
    print_newline ()
  ) forms

let cmd_bytes (path : string) : unit =
  Compiler.reset_globals ();
  let text = read_file path in
  let forms = Reader.read_all text in
  (* dump raw bytecode bytes to stdout — caller pipes to moof-zig.
     for multi-form sources we concatenate top-chunk bodies. (V4-α;
     phase ε may want a per-form header.) *)
  set_binary_mode_out stdout true;
  List.iter (fun form ->
    let cb = Compiler.compile_top form in
    let final = Compiler.finalize cb in
    output_bytes stdout final.body
  ) forms

let cmd_build_image (path : string) (output : string) : unit =
  Compiler.reset_globals ();
  let text = read_file path in
  let forms = Reader.read_all text in
  (* compile every top-level form. the compile_top calls implicitly
     register their (and any nested) chunks in the global chunk
     registry — Compiler.all_chunks () returns them in registration
     order. *)
  List.iter (fun form ->
    let cb = Compiler.compile_top form in
    let _ = Compiler.finalize cb in
    ()
  ) forms;
  (* convert each registered chunk_builder → vat_chunk by finalizing
     it. note: top-level builders are finalized twice (once above, once
     here) — finalize is pure, so that's safe.  V4-α: source_form_id
     is set to 0 (no Form pre-allocated). *)
  let chunks =
    List.map (fun (cb : Compiler.chunk_builder) ->
      let f = Compiler.finalize cb in
      Image.{
        source_form_id = 0;       (* TODO(phase-ε): real source Form *)
        body = f.body;
        consts = f.consts;
        ic_count = f.ic_count;
        params = f.params;
      }
    ) (Compiler.all_chunks ())
  in
  (* TODO(phase-ε): proto stubs + here_form + macros_form. for V4-α
     Path B we emit an empty FormSection and zero proto/form ids —
     produces a structurally-valid image (deserializer can parse it)
     but it isn't bootstrappable on its own. *)
  let vat = Image.{
    vat_id = Bytes.make 16 '\x00';   (* TODO: real ulid *)
    syms = Compiler.all_syms ();
    forms = [];
    chunks;
    natives = [];
    mcos = [];
    far_refs = [];
    external_vat_refs = [];
    here_form_id = 0;                (* TODO: pre-alloc Form *)
    macros_form_id = 0;
    protos = empty_protos;
  } in
  let bytes = Image.serialize vat in
  let oc = open_out_bin output in
  output_bytes oc bytes;
  close_out oc;
  Printf.printf "wrote %s (%d bytes, %d chunks, %d syms)\n"
    output (Bytes.length bytes) (List.length chunks)
    (List.length vat.syms)

(* ── dispatch ──────────────────────────────────────────────────── *)

let usage () =
  prerr_endline "usage:";
  prerr_endline "  moof-seed parse <file.moof>";
  prerr_endline "  moof-seed compile <file.moof>";
  prerr_endline "  moof-seed bytes <file.moof>";
  prerr_endline "  moof-seed build-image <file.moof> <output.vat>";
  prerr_endline "  moof-seed build-seed --root <lib-dir> --output <seed.vat>";
  exit 1

let () =
  try
    let argv = Sys.argv in
    if Array.length argv < 2 then usage ();
    match argv.(1) with
    | "parse" ->
        if Array.length argv <> 3 then usage ();
        cmd_parse argv.(2)
    | "compile" ->
        if Array.length argv <> 3 then usage ();
        cmd_compile argv.(2)
    | "bytes" ->
        if Array.length argv <> 3 then usage ();
        cmd_bytes argv.(2)
    | "build-image" ->
        if Array.length argv <> 4 then usage ();
        cmd_build_image argv.(2) argv.(3)
    | "build-seed" ->
        Build_seed_cmd.run (Array.sub argv 2 (Array.length argv - 2))
    | _ -> usage ()
  with
  | Reader.ReadError msg ->
      prerr_endline ("read error: " ^ msg); exit 1
  | Compiler.Compile_error msg ->
      prerr_endline ("compile error: " ^ msg); exit 1
  | Sys_error msg ->
      prerr_endline ("io error: " ^ msg); exit 1
