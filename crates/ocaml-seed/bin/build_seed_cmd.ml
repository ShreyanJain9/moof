(* build_seed_cmd.ml - moof-seed build-seed subcommand.

   Reads <root>/main.moof, statically resolves transporter loads under
   parser/ and compiler/ (everything else is left to the moof runtime),
   parses + compiles each minimal-bootstrap file with the stripped seed,
   and serializes the result as a V4 vat-image to <output>.

   spec: docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md §5 W3
   image layout: docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md §10.3

   Output: <output> is a V4 .vat file with magic MVAT + version 4 BE,
   containing the parser + compiler chunks only. Boot-stdlib + early/*
   + stdlib/* + mcos are NOT included; the moof runtime handles those
   via its in-image transporter once seed.vat is loaded.

   Determinism (V4 spec §9, D5): same lib/ source produces byte-
   identical seed.vat. Achieved by:
     - reading files in the order they appear in main.moof
     - compiling forms in source order
     - global sym table interns in encounter order
     - chunk registry assigns monotonic chunk-ids in depth-first order
*)

open Moof_seed

(* ----------------------------------------------------------------
   argument parsing
   ---------------------------------------------------------------- *)

type opts = {
  root : string;
  output : string;
}

let usage_msg = "moof-seed build-seed --root <lib-dir> --output <seed.vat>"

let parse_args (args : string array) : opts =
  let root = ref None in
  let output = ref None in
  let i = ref 0 in
  let n = Array.length args in
  while !i < n do
    let a = args.(!i) in
    (match a with
     | "--root" ->
         incr i;
         if !i >= n then (prerr_endline usage_msg; exit 1);
         root := Some args.(!i)
     | "--output" ->
         incr i;
         if !i >= n then (prerr_endline usage_msg; exit 1);
         output := Some args.(!i)
     | _ ->
         prerr_endline ("unknown arg: " ^ a);
         prerr_endline usage_msg;
         exit 1);
    incr i
  done;
  match !root, !output with
  | Some r, Some o -> { root = r; output = o }
  | _ ->
      prerr_endline usage_msg;
      exit 1

(* ----------------------------------------------------------------
   file IO
   ---------------------------------------------------------------- *)

let read_file (path : string) : string =
  let ic = open_in path in
  let len = in_channel_length ic in
  let buf = Bytes.create len in
  really_input ic buf 0 len;
  close_in ic;
  Bytes.to_string buf

(* ----------------------------------------------------------------
   static transporter resolution

   A "transporter load" is a top-level Form of shape:
     (__send__ (Sym "$transporter") (Sym "load:") (Str "path"))

   We scan main.moof's top-level forms. Each form is one of:
     1. a transporter load whose path starts with "parser/" or
        "compiler/" - we follow it and recursively include its
        contents.
     2. a transporter load whose path is anything else (early/,
        stdlib/, mcos.moof, etc.) - we SKIP it. The moof runtime
        will execute these at boot time via its in-image transporter.
     3. anything else (e.g. [$compiler useMoof]) - we SKIP at the seed
        stage. The moof runtime executes these.

   Files reached via (1) are also scanned the same way - if a parser
   file loads another parser file, we follow.
   ---------------------------------------------------------------- *)

let str_starts_with (prefix : string) (s : string) : bool =
  let lp = String.length prefix in
  String.length s >= lp && String.sub s 0 lp = prefix

let is_minimal_bootstrap_path (path : string) : bool =
  str_starts_with "parser/" path || str_starts_with "compiler/" path

(* Match (__send__ (Sym "$transporter") (Sym "load:") (Str path)).
   Returns Some path if it matches, None otherwise. *)
let match_transporter_load (form : Ast.form) : string option =
  match form with
  | Ast.Cons (Ast.Sym "__send__",
      Ast.Cons (Ast.Sym "$transporter",
        Ast.Cons (Ast.Sym "load:",
          Ast.Cons (Ast.Str path, Ast.Nil)))) ->
      Some path
  | _ -> None

(* A "load step" gathers the forms we actually compile, in order, and
   tracks (filename, form) for diagnostics. *)
type load_step = {
  source_path : string;       (* path relative to lib/ (or "<main>") *)
  form : Ast.form;
}

(* Read top-level forms from a file. For main.moof specifically, we
   tolerate parse failures (some lines may use banned syntax like
   send-cascade for non-minimal stdlib loads, which we skip anyway).
   For any minimal-bootstrap file (parser-slash-star, compiler-slash-
   star), any parse failure is a real bug we want to surface. *)
let read_top_forms (full : string) (tolerant : bool) : Ast.form list =
  let src = read_file full in
  if not tolerant then
    Reader.read_all src
  else begin
    (* Tolerant mode: walk the source, parsing one form at a time. On
       a read error, skip ahead to the next plausible top-level start
       (the next blank-line-preceded `[` or `(`). For main.moof's
       cascade-using lines this works because each cascade block sits
       at the top level on its own. *)
    let cursor = Reader.make_cursor src in
    let acc = ref [] in
    let rec loop () =
      Reader.skip_trivia cursor;
      if Reader.cur_at_end cursor then ()
      else begin
        let saved_pos = cursor.pos in
        (try
          let f = Reader.read_form cursor in
          acc := f :: !acc
        with Reader.ReadError _ ->
          (* Reader errored partway. Recover by advancing to the next
             newline-then-toplevel position. Conservative: scan to the
             next "\n[" or "\n(" boundary. *)
          cursor.pos <- saved_pos;
          let len = cursor.len in
          let p = ref cursor.pos in
          let saw_newline = ref false in
          let done_ = ref false in
          while not !done_ && !p < len do
            let b = Bytes.get cursor.bytes !p in
            if !saw_newline && (b = '[' || b = '(') then begin
              cursor.pos <- !p;
              done_ := true
            end else begin
              if b = '\n' then saw_newline := true;
              incr p
            end
          done;
          if not !done_ then cursor.pos <- len);
        loop ()
      end
    in
    loop ();
    List.rev !acc
  end

let rec gather_from_file (root : string) (rel_path : string)
                         (acc : load_step list ref) : unit =
  let full = Filename.concat root rel_path in
  let tolerant = (rel_path = "main.moof") in
  let forms = read_top_forms full tolerant in
  List.iter (fun form ->
    match match_transporter_load form with
    | Some sub_path when is_minimal_bootstrap_path sub_path ->
        (* recurse: follow parser/* and compiler/* transitively *)
        gather_from_file root sub_path acc
    | Some _ ->
        (* transporter load of non-minimal path - skip; the moof runtime
           handles these at boot via the in-image transporter. *)
        ()
    | None ->
        (* not a transporter load - if this is main.moof, skip the form
           (e.g. [$compiler useMoof]); the runtime handles it. Otherwise
           it's a real compilable form from a parser/* or compiler/*
           file - include it. *)
        if rel_path <> "main.moof" then
          acc := { source_path = rel_path; form } :: !acc
        else
          ()
  ) forms

(* Top-level entry: gather every form from parser/ + compiler/ files,
   recursively, starting from main.moof's transporter chain. *)
let gather_steps (root : string) : load_step list =
  let acc = ref [] in
  gather_from_file root "main.moof" acc;
  List.rev !acc

(* ----------------------------------------------------------------
   non-scalar const lifting

   The V4 image format encodes only scalar Values inline
   (Nil/Bool/Int/Float/Char/Sym/FormRef per spec §10.3). Compound
   values - String / Bytes / Cons - must be allocated as Forms in
   the FormSection, then referenced via FormRef tags.

   Compiler.ml emits these compound values into chunk consts (e.g.
   [quote (a b c)] produces a Cons constant). We pre-allocate one
   Form per unique compound value, populate the FormSection, and
   rewrite every chunk's consts list to use FormRef tags.

   Form layout for lifted compounds (matching the in-image runtime's
   structure that the zig substrate will recognize):
     - Str s   -> { proto: Nil, slots: [], handlers: [], meta: [], frozen: 1 }
                  (W4/W5 will refine: store the raw bytes in a :raw slot)
     - Bytes b -> same
     - Cons (a, b) -> { proto: Nil,
                        slots: [('car, a-encoded), ('cdr, b-encoded)],
                        ... }

   For the V4-alpha seed image, the placeholder slot population
   (with sym-resolved 'car / 'cdr) is sufficient because the moof
   runtime will rewrite this representation during its boot. The
   ZIG SUBSTRATE'S loader just needs to walk the FormSection
   bytes consistently with the spec.

   ---------------------------------------------------------------- *)

(* Hashable representation of an Ast.form for memoization. We use
   the OCaml polymorphic compare via Hashtbl - works because Ast.form
   has no functions / closures and structural equality is defined. *)
let form_table : (Ast.form, int) Hashtbl.t = Hashtbl.create 256
(* The next FormId to allocate. Starts at 1 - FormId(0) is the
   image-format sentinel (per spec §10.3, "first non-sentinel is
   FormId(1)"). *)
let next_form_id = ref 1
(* The lifted forms, in allocation order. Position i in this list
   corresponds to FormId (i+1). *)
let lifted_forms : Image.vat_form list ref = ref []

(* Reset the lifter's state. Call between independent build-seed runs
   to keep things deterministic. *)
let reset_lifter () =
  Hashtbl.clear form_table;
  next_form_id := 1;
  lifted_forms := []

(* Lift a single Ast.form into the FormSection, returning a scalar
   replacement (FormRef for non-scalars, or the value itself for
   scalars). Recursive: nested compounds inside a Cons are lifted
   first.

   Memoized by structural equality - same value gets the same
   FormId. This is good for determinism and image size. *)
let rec lift_value (v : Ast.form) : Ast.form =
  match v with
  (* scalars pass through unchanged *)
  | Ast.Nil | Ast.Bool _ | Ast.Int _ | Ast.Float _
  | Ast.Char _ | Ast.Sym _ | Ast.FormRef _ -> v
  (* compounds lift to FormRef *)
  | Ast.Str _ | Ast.Bytes _ | Ast.Cons _ | Ast.Vec _ ->
      (match Hashtbl.find_opt form_table v with
       | Some id -> Ast.FormRef id
       | None ->
           let id = !next_form_id in
           incr next_form_id;
           Hashtbl.add form_table v id;
           let form = build_form_for v in
           lifted_forms := form :: !lifted_forms;
           Ast.FormRef id)

(* Build a vat_form record for a non-scalar Ast.form. For Cons cells
   we emit :car and :cdr slots (which means 'car / 'cdr must be in
   the sym table - we call Compiler.intern to ensure that). *)
and build_form_for (v : Ast.form) : Image.vat_form =
  match v with
  | Ast.Str _ | Ast.Bytes _ ->
      (* W4/W5: store the actual bytes via a :raw slot. For seed.vat
         the moof runtime treats String / Bytes Forms as opaque -
         their identity matters more than payload at this stage
         (since these are mostly error-message strings inside
         compiler chunks). *)
      Image.{
        proto = Ast.Nil;
        slots = [];
        handlers = [];
        meta = [];
        frozen = true;
      }
  | Ast.Cons (head, tail) ->
      let car_sym = Compiler.intern "car" in
      let cdr_sym = Compiler.intern "cdr" in
      let head' = lift_value head in
      let tail' = lift_value tail in
      Image.{
        proto = Ast.Nil;
        slots = [(car_sym, head'); (cdr_sym, tail')];
        handlers = [];
        meta = [];
        frozen = true;
      }
  | Ast.Vec xs ->
      let items_sym = Compiler.intern "items" in
      let items' = List.map lift_value xs in
      (* encode as a chained Cons within a single :items slot.
         this is a placeholder - vector literals are banned in
         the minimal subset so this should never fire. *)
      let rec to_list_form = function
        | [] -> Ast.Nil
        | x :: rest -> Ast.Cons (x, to_list_form rest)
      in
      let items_form = lift_value (to_list_form items') in
      Image.{
        proto = Ast.Nil;
        slots = [(items_sym, items_form)];
        handlers = [];
        meta = [];
        frozen = true;
      }
  | _ -> failwith "build_form_for: unexpected scalar"

(* ----------------------------------------------------------------
   compile + serialize

   We follow the same V4-alpha shape that cmd_build_image already uses
   in seed.ml: no NativeRefs, no Mcos, no FarRefs. The ChunkSection
   + SymTable are real. seed.vat is the "minimal frame" that gets
   fleshed out at boot by the moof runtime (it allocates protos,
   registers natives, calls into our chunks).

   TODO(W4/W5): once the zig runtime can boot from seed.vat directly,
   we may need to pre-allocate boot protos here. For now the structure
   is "structurally valid V4 image" which the zig loader can parse.
   ---------------------------------------------------------------- *)

let run (args : string array) : unit =
  let opts = parse_args args in
  Compiler.reset_globals ();
  reset_lifter ();
  let steps = gather_steps opts.root in
  if steps = [] then begin
    Printf.eprintf "build-seed: no minimal-bootstrap forms found under %s\n"
      opts.root;
    Printf.eprintf "  (expected parser/* and/or compiler/* loaded from %s/main.moof)\n"
      opts.root;
    exit 1
  end;
  (* Compile every form. Each compile_top registers its chunk + nested
     closures into the global chunk registry. *)
  List.iter (fun step ->
    try
      let cb = Compiler.compile_top step.form in
      let _ = Compiler.finalize cb in
      ()
    with
    | Compiler.Compile_error msg ->
        Printf.eprintf "compile error in %s: %s\n" step.source_path msg;
        exit 1
  ) steps;
  (* Lift non-scalar consts into FormRefs. We walk every chunk's
     consts in registry order (deterministic) and rewrite each
     non-scalar to a FormRef pointing at a pre-allocated Form. *)
  let chunks =
    List.map (fun (cb : Compiler.chunk_builder) ->
      let f = Compiler.finalize cb in
      let lifted_consts = List.map lift_value f.consts in
      Image.{
        source_form_id = 0;   (* W4/W5: real source Form pre-alloc *)
        body = f.body;
        consts = lifted_consts;
        ic_count = f.ic_count;
        params = f.params;
      }
    ) (Compiler.all_chunks ())
  in
  let forms = List.rev !lifted_forms in
  let vat = Image.{
    vat_id = Bytes.make 16 '\x00';       (* deterministic placeholder *)
    syms = Compiler.all_syms ();
    forms;
    chunks;
    natives = [];
    mcos = [];
    far_refs = [];
    external_vat_refs = [];
    here_form_id = 0;                    (* runtime alloc *)
    macros_form_id = 0;
    protos = Image.empty_protos;         (* runtime alloc *)
  } in
  let bytes = Image.serialize vat in
  let oc = open_out_bin opts.output in
  output_bytes oc bytes;
  close_out oc;
  Printf.printf "wrote %s (%d bytes, %d chunks, %d syms, %d forms, %d files)\n"
    opts.output
    (Bytes.length bytes)
    (List.length chunks)
    (List.length vat.syms)
    (List.length forms)
    (List.length (List.sort_uniq compare
                    (List.map (fun s -> s.source_path) steps)))
