(* moof-seed CLI.

   Subcommands:
     compile <file.moof>
       reads, parses each top-level form, compiles to V4 bytecode,
       prints disassembly + hex dump.

     build-image --root <dir> --entry <file> --output <out.vat>
       reads entry, compiles it into a single-form vat image, serializes.
       NOTE: transitive [$transporter load: ...] following is OUT OF SCOPE
       for V4 minimum viable — flagged as TODO.

   See docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md §10
   for the image-format contract. *)

open Moof_seed

(* ---------------- compile ---------------- *)

let cmd_compile (file : string) : unit =
  let forms = Reader.read_file file in
  List.iteri (fun i form ->
    Printf.printf "=== form %d ===\n" i;
    Printf.printf "source: %s\n" (Ast.to_string form);
    let cb = Compiler.compile_top form in
    let final = Compiler.finalize cb in
    let ops = Bytecode.decode_ops final.body in
    List.iter (fun op -> print_endline ("  " ^ Opcodes.show_op op)) ops;
    Printf.printf "bytes (%d): %s\n"
      (Bytes.length final.body) (Bytecode.to_hex final.body);
    Printf.printf "consts (%d):\n" (List.length final.consts);
    List.iteri (fun j c ->
      Printf.printf "  [%d] %s\n" j (Ast.to_string c)
    ) final.consts;
    Printf.printf "syms (%d):\n" (List.length final.sym_order);
    List.iteri (fun j s ->
      Printf.printf "  [%d] %s\n" j s
    ) final.sym_order;
    Printf.printf "ic_count: %d\n" final.ic_count
  ) forms

(* ---------------- build-image ---------------- *)

(* generate a 16-byte vat id from a string seed. for the seed CLI, we use
   blake3(entry-path) truncated to 16. ulid-style; matches manifest entry. *)
let gen_vat_id (seed : string) : bytes =
  let h = Blake3.hash (Bytes.of_string seed) in
  Bytes.sub h 0 16

(* For the V4 minimum-viable build-image:
     - compile just the entry file (no [$transporter load:] recursion — TODO)
     - emit one Form (a stub "Program" proto holding the compiled chunk)
     - emit one chunk per top-level form, sharing one sym table
     - assign vat_id from blake3(entry-name)

   This is enough to produce a non-empty .vat file the zig deserializer
   can parse, while leaving room for the integration agent to iterate. *)

let cmd_build_image ~(root : string) ~(entry : string) ~(output : string) : unit =
  let entry_path = Filename.concat root entry in
  let forms = Reader.read_file entry_path in
  (* compile each top-level form, threading sym/const state into the image
     via per-form chunks. *)
  let chunks = ref [] in
  (* Build a vat-level symbol table by accumulating each chunk's sym_order.
     Simple deduplication via Hashtbl. *)
  let sym_table = Hashtbl.create 32 in
  let sym_order = ref [] in
  let intern_vat_sym s =
    match Hashtbl.find_opt sym_table s with
    | Some i -> i
    | None ->
        let i = Hashtbl.length sym_table in
        Hashtbl.add sym_table s i;
        sym_order := s :: !sym_order;
        i
  in
  (* sentinel form (#0) — required so first non-sentinel is FormId(1). *)
  let sentinel : Image.vat_form = {
    proto = Ast.Nil;
    slots = [];
    handlers = [];
    meta = [];
    frozen = true;
  } in
  let forms_alloc = ref [sentinel] in
  let next_form_id = ref 1 in
  let alloc_form (f : Image.vat_form) : int =
    let id = !next_form_id in
    forms_alloc := f :: !forms_alloc;
    incr next_form_id;
    id
  in
  (* Pre-intern proto name symbols (for the proto-table). *)
  (* Create proto-Form stubs (each just a stand-in with a :name slot
     holding the proto's symbol). Realistic boot would populate handlers
     via NativeRefsSection; for V4 minimum-viable we ship empty proto-forms
     and let the integration agent wire up natives. *)
  let name_slot = intern_vat_sym "name" in
  let mk_proto_form (name : string) : int =
    let _ = intern_vat_sym name in
    alloc_form {
      proto = Ast.Nil;
      slots = [(name_slot, Ast.Sym name)];
      handlers = [];
      meta = [];
      frozen = true;
    }
  in
  (* Allocate proto forms in canonical order so we can populate the
     proto_table. The names match spec §10.3 protos. *)
  let object_id = mk_proto_form "Object" in
  let nil_id = mk_proto_form "Nil" in
  let bool_id = mk_proto_form "Bool" in
  let integer_id = mk_proto_form "Integer" in
  let char_id = mk_proto_form "Char" in
  let sym_id = mk_proto_form "Sym" in
  let cons_id = mk_proto_form "Cons" in
  let string_id = mk_proto_form "String" in
  let bytes_id = mk_proto_form "Bytes" in
  let method_id = mk_proto_form "Method" in
  let chunk_id = mk_proto_form "Chunk" in
  let closure_id = mk_proto_form "Closure" in
  let env_id = mk_proto_form "Env" in
  let foreign_handle_id = mk_proto_form "ForeignHandle" in
  let table_id = mk_proto_form "Table" in
  let frame_id = mk_proto_form "Frame" in
  let macros_id = mk_proto_form "Macros" in
  let opcode_id = mk_proto_form "Opcode" in

  (* $here form — the vat's globals registry. *)
  let here_form_id = alloc_form {
    proto = Ast.Nil;
    slots = [];
    handlers = [];
    meta = [];
    frozen = false;
  } in
  let macros_form_id = alloc_form {
    proto = Ast.Nil;
    slots = [];
    handlers = [];
    meta = [];
    frozen = false;
  } in

  (* Compile each top-level form. For each, allocate a Form-id for its
     `source` Form (a stub holding the raw source as a :text slot), then
     emit a chunk whose source_form_id points there. *)
  List.iter (fun form ->
    let source_text_sym = intern_vat_sym "source-text" in
    let source_form_id = alloc_form {
      proto = Ast.Nil;
      slots = [(source_text_sym, Ast.Nil)]; (* string body is non-scalar; TODO *)
      handlers = [];
      meta = [];
      frozen = true;
    } in
    let cb = Compiler.compile_top form in
    let final = Compiler.finalize cb in
    (* re-map the chunk's sym refs from local sym ids to vat-level ones. *)
    (* The seed compiler emits SymIds local to its chunk_builder. We need
       to map them through cb.sym_order → vat-level sym ids. *)
    let local_to_vat = Array.make (List.length final.sym_order) 0 in
    List.iteri (fun i s ->
      local_to_vat.(i) <- intern_vat_sym s
    ) final.sym_order;
    (* re-encode the chunk body, substituting sym ids. *)
    let ops = Bytecode.decode_ops final.body in
    let remapped = List.map (fun op ->
      let open Opcodes in
      match op with
      | LoadName s -> LoadName local_to_vat.(s)
      | Send (s, argc, ic) -> Send (local_to_vat.(s), argc, ic)
      | TailSend (s, argc) -> TailSend (local_to_vat.(s), argc)
      | SuperSend (s, argc, ic) -> SuperSend (local_to_vat.(s), argc, ic)
      | SendSelf (s, argc, ic) -> SendSelf (local_to_vat.(s), argc, ic)
      | SendHere (s, argc, ic) -> SendHere (local_to_vat.(s), argc, ic)
      | TailSendSelf (s, argc) -> TailSendSelf (local_to_vat.(s), argc)
      | TailSendHere (s, argc) -> TailSendHere (local_to_vat.(s), argc)
      | other -> other
    ) ops in
    let body = Bytecode.encode_ops remapped in
    let params = List.map (fun s ->
      intern_vat_sym (List.nth final.sym_order s)
    ) final.params in
    chunks := {
      Image.source_form_id;
      body;
      consts = final.consts;
      ic_count = final.ic_count;
      params;
    } :: !chunks
  ) forms;

  let chunks = List.rev !chunks in
  (* Drop sentinel from output: spec §10.3 says num_forms is the count
     EXCLUDING the sentinel; but the file actually lists every form
     starting at index 1. We keep the sentinel in the list head and emit
     only forms after it. *)
  let forms_list = List.rev !forms_alloc in
  let forms_no_sentinel = List.tl forms_list in

  let img : Image.vat_image = {
    vat_id = gen_vat_id entry;
    syms = List.rev !sym_order;
    forms = forms_no_sentinel;
    chunks;
    natives = [];
    mcos = [];
    far_refs = [];
    external_vat_refs = [];
    here_form_id;
    macros_form_id;
    protos = {
      object_id; nil_id; bool_id; integer_id; char_id; sym_id; cons_id;
      string_id; bytes_id; method_id; chunk_id; closure_id; env_id;
      foreign_handle_id; table_id; frame_id; macros_id; opcode_id;
    };
  } in
  let bytes = Image.serialize img in
  let oc = open_out_bin output in
  output_bytes oc bytes;
  close_out oc;
  Printf.printf "Wrote %s (%d bytes)\n" output (Bytes.length bytes);
  Printf.printf "vat_id: %s\n" (Bytecode.to_hex img.vat_id);
  Printf.printf "num_forms: %d, num_syms: %d, num_chunks: %d\n"
    (List.length img.forms) (List.length img.syms) (List.length img.chunks);
  Printf.printf "TODO: transitive [$transporter load: ...] following.\n";
  Printf.printf "TODO: string/bytes/cons Form allocation for source-text slot.\n"

(* ---------------- argv dispatch ---------------- *)

let usage () =
  prerr_endline "Usage:";
  prerr_endline "  moof-seed compile <file.moof>";
  prerr_endline "  moof-seed build-image --root <dir> --entry <file> --output <out.vat>";
  exit 1

let parse_build_image_args (args : string array) =
  let root = ref "" in
  let entry = ref "" in
  let output = ref "" in
  let i = ref 2 in
  while !i < Array.length args do
    (match args.(!i) with
     | "--root"   -> incr i; root   := args.(!i)
     | "--entry"  -> incr i; entry  := args.(!i)
     | "--output" -> incr i; output := args.(!i)
     | other ->
         prerr_endline (Printf.sprintf "unknown arg: %s" other);
         usage ());
    incr i
  done;
  if !root = "" || !entry = "" || !output = "" then usage ();
  (!root, !entry, !output)

let main () =
  let argv = Sys.argv in
  if Array.length argv < 2 then usage ();
  match argv.(1) with
  | "compile" ->
      if Array.length argv < 3 then usage ();
      cmd_compile argv.(2)
  | "build-image" ->
      let (root, entry, output) = parse_build_image_args argv in
      cmd_build_image ~root ~entry ~output
  | _ -> usage ()

let () = main ()
