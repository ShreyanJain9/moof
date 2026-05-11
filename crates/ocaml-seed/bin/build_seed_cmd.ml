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

(* Allocate a fresh Form at the next FormId, append to the FormSection
   list, return the assigned FormId. NOT memoized — caller is responsible
   for de-duplication when appropriate. Used by proto / here_form /
   macros_form / per-chunk-source allocation. *)
let alloc_form_raw (f : Image.vat_form) : int =
  let id = !next_form_id in
  incr next_form_id;
  lifted_forms := f :: !lifted_forms;
  id

(* ----------------------------------------------------------------
   boot proto allocation

   Mirror crates/substrate/src/protos.rs::Protos::bootstrap and
   crates/zig-substrate/src/protos.zig::bootstrap. The 18 canonical
   protos are allocated in a fixed order so the image's Header.protos
   table can name them by FormId.

   Each proto Form gets a :name meta slot (sym) so reflection works
   from the moment the image loads. Inheritance:
     - Object proto:Nil    (root)
     - Closure proto:Method
     - everything else proto:Object
   ---------------------------------------------------------------- *)

(* The boot proto FormIds, captured after allocation. Order in this
   record matches the V4 image header's proto-table order (spec §10.3
   and image.ml::proto_table). *)
type boot_protos = {
  object_id   : int;
  nil_id      : int;
  bool_id     : int;
  integer_id  : int;
  char_id     : int;
  sym_id      : int;
  cons_id     : int;
  string_id   : int;
  bytes_id    : int;
  method_id   : int;
  chunk_id    : int;
  closure_id  : int;
  env_id      : int;
  foreign_id  : int;
  table_id    : int;
  frame_id    : int;
  macros_id   : int;
  opcode_id   : int;
}

(* Allocate one empty proto Form. `parent` is the parent proto's value
   (Nil for Object, Form(object_id) for normal protos, Form(method_id)
   for Closure). `name` is the proto's reflection name — populated into
   the :name meta slot. *)
let alloc_proto (parent : Ast.form) (name : string) : int =
  let name_meta = Compiler.intern "name" in
  let name_sym = Compiler.intern name in
  alloc_form_raw Image.{
    proto = parent;
    slots = [];
    handlers = [];
    meta = [(name_meta, Ast.Sym name)];
    frozen = false;
  } |> fun id ->
  let _ = name_sym in
  id

let bootstrap_protos () : boot_protos =
  (* Object is the root; its proto is Nil so the chain terminates. *)
  let object_id = alloc_proto Ast.Nil "Object" in
  let object_ref = Ast.FormRef object_id in
  let p name = alloc_proto object_ref name in
  let nil_id = p "Nil" in
  let bool_id = p "Bool" in
  let integer_id = p "Integer" in
  let char_id = p "Char" in
  let sym_id = p "Sym" in
  let cons_id = p "Cons" in
  let string_id = p "String" in
  let bytes_id = p "Bytes" in
  let method_id = p "Method" in
  let chunk_id = p "Chunk" in
  (* Closure proto = Method (closures inherit Method dispatch). *)
  let closure_id = alloc_proto (Ast.FormRef method_id) "Closure" in
  let env_id = p "Env" in
  let foreign_id = p "ForeignHandle" in
  let table_id = p "Table" in
  let frame_id = p "Frame" in
  let macros_id = p "Macros" in
  let opcode_id = p "Opcode" in
  {
    object_id; nil_id; bool_id; integer_id; char_id; sym_id; cons_id;
    string_id; bytes_id; method_id; chunk_id; closure_id; env_id;
    foreign_id; table_id; frame_id; macros_id; opcode_id;
  }

(* ----------------------------------------------------------------
   here_form / macros_form allocation

   Mirror crates/substrate/src/world.rs::World::new (lines ~330-340)
   and crates/zig-substrate/src/world.zig::init (lines ~311-328). The
   image carries the canonical FormIds for both; zig's image-load reads
   them from the header and populates world.here_form / world.macros_form.

   here_form is the env-Form serving as the vat's globals. Its proto
   is Env, its :meta.parent is Nil (root of env chain), and one of its
   slots binds `$here` to itself (so [Env current] / `$here`-lookup
   resolves). We also seed bindings for the 18 proto names so user
   code can reach `Object`, `Cons`, etc. by name.

   macros_form is the canonical macro registry — a plain Object-proto
   Form whose slots will hold macro-name → method-Form once macros
   load. Empty here; the runtime populates as user code defines.
   ---------------------------------------------------------------- *)

(* Allocate here_form + macros_form, returning their FormIds. Must be
   called after bootstrap_protos so we know Env / Object FormIds. *)
let alloc_here_and_macros (bp : boot_protos) : int * int =
  let parent_sym = Compiler.intern "parent" in
  let name_meta = Compiler.intern "name" in
  let macros_name = Compiler.intern "Macros" in
  (* macros_form: proto=Object, :meta name=Macros, no slots. *)
  let macros_id = alloc_form_raw Image.{
    proto = Ast.FormRef bp.object_id;
    slots = [];
    handlers = [];
    meta = [(name_meta, Ast.Sym "Macros")];
    frozen = false;
  } in
  let _ = macros_name in
  (* here_form: proto=Env, :meta parent=Nil. Slots populated below. *)
  let here_id = alloc_form_raw Image.{
    proto = Ast.FormRef bp.env_id;
    slots = [];          (* filled in by patch_here_form below *)
    handlers = [];
    meta = [(parent_sym, Ast.Nil)];
    frozen = false;
  } in
  (macros_id, here_id)

(* After here_form is allocated, build its slot list (binding `$here`
   to itself + each proto name to its Form). Since lifted_forms is
   built by prepending, we splice the patched here_form record back
   into its original position.

   We don't have a "mutate-by-id" API; instead we filter+replace.
   Allocation order is preserved because we don't change FormIds. *)
let patch_here_form (here_id : int) (bp : boot_protos) : unit =
  let here_sym = Compiler.intern "$here" in
  let object_s = Compiler.intern "Object" in
  let nil_s = Compiler.intern "Nil" in
  let bool_s = Compiler.intern "Bool" in
  let integer_s = Compiler.intern "Integer" in
  let char_s = Compiler.intern "Char" in
  let sym_s = Compiler.intern "Sym" in
  let cons_s = Compiler.intern "Cons" in
  let string_s = Compiler.intern "String" in
  let bytes_s = Compiler.intern "Bytes" in
  let method_s = Compiler.intern "Method" in
  let chunk_s = Compiler.intern "Chunk" in
  let closure_s = Compiler.intern "Closure" in
  let env_s = Compiler.intern "Env" in
  let foreign_s = Compiler.intern "ForeignHandle" in
  let table_s = Compiler.intern "Table" in
  let frame_s = Compiler.intern "Frame" in
  let macros_s = Compiler.intern "Macros" in
  let opcode_s = Compiler.intern "Opcode" in
  let here_slots = [
    (here_sym,    Ast.FormRef here_id);
    (object_s,    Ast.FormRef bp.object_id);
    (nil_s,       Ast.FormRef bp.nil_id);
    (bool_s,      Ast.FormRef bp.bool_id);
    (integer_s,   Ast.FormRef bp.integer_id);
    (char_s,      Ast.FormRef bp.char_id);
    (sym_s,       Ast.FormRef bp.sym_id);
    (cons_s,      Ast.FormRef bp.cons_id);
    (string_s,    Ast.FormRef bp.string_id);
    (bytes_s,     Ast.FormRef bp.bytes_id);
    (method_s,    Ast.FormRef bp.method_id);
    (chunk_s,     Ast.FormRef bp.chunk_id);
    (closure_s,   Ast.FormRef bp.closure_id);
    (env_s,       Ast.FormRef bp.env_id);
    (foreign_s,   Ast.FormRef bp.foreign_id);
    (table_s,     Ast.FormRef bp.table_id);
    (frame_s,     Ast.FormRef bp.frame_id);
    (macros_s,    Ast.FormRef bp.macros_id);
    (opcode_s,    Ast.FormRef bp.opcode_id);
  ] in
  (* Walk lifted_forms (stored reverse-allocation-order) and rewrite
     the entry whose FormId == here_id. lifted_forms is a list whose
     head is the LAST allocated form; position from the head is
     (next_form_id - 1 - here_id). *)
  let cur_top = !next_form_id - 1 in
  let target_offset = cur_top - here_id in
  let rec patch_at i acc = function
    | [] -> List.rev acc
    | f :: rest when i = target_offset ->
        let patched = Image.{ f with slots = here_slots } in
        List.rev_append acc (patched :: rest)
    | f :: rest -> patch_at (i + 1) (f :: acc) rest
  in
  lifted_forms := patch_at 0 [] !lifted_forms

(* Append a single (sym, value) pair to an already-allocated Form's
   slots list. Like patch_here_form but additive — preserves existing
   slots and appends a new one. Used to bind `main` on here_form once
   we know the boot chunk's FormId. *)
let patch_form_slots (target_id : int) (slot : int * Ast.form) : unit =
  let cur_top = !next_form_id - 1 in
  let target_offset = cur_top - target_id in
  let rec patch_at i acc = function
    | [] ->
        failwith (Printf.sprintf
          "patch_form_slots: FormId %d not in lifted_forms" target_id)
    | f :: rest when i = target_offset ->
        let patched = Image.{ f with slots = f.slots @ [slot] } in
        List.rev_append acc (patched :: rest)
    | f :: rest -> patch_at (i + 1) (f :: acc) rest
  in
  lifted_forms := patch_at 0 [] !lifted_forms

(* Convert boot_protos -> Image.proto_table for the image header. *)
let protos_table (bp : boot_protos) : Image.proto_table =
  Image.{
    object_id = bp.object_id;
    nil_id = bp.nil_id;
    bool_id = bp.bool_id;
    integer_id = bp.integer_id;
    char_id = bp.char_id;
    sym_id = bp.sym_id;
    cons_id = bp.cons_id;
    string_id = bp.string_id;
    bytes_id = bp.bytes_id;
    method_id = bp.method_id;
    chunk_id = bp.chunk_id;
    closure_id = bp.closure_id;
    env_id = bp.env_id;
    foreign_handle_id = bp.foreign_id;
    table_id = bp.table_id;
    frame_id = bp.frame_id;
    macros_id = bp.macros_id;
    opcode_id = bp.opcode_id;
  }

(* The active proto table for lifting. Set by run() once
   bootstrap_protos completes; consulted by build_form_for to assign
   the right :proto for String / Bytes / Cons Forms.

   This is a ref rather than a parameter because lift_value /
   build_form_for are mutually recursive and called from inside
   List.map / List.iter contexts that we don't thread an explicit
   `bp` through. *)
let active_protos : boot_protos option ref = ref None

let get_protos () : boot_protos =
  match !active_protos with
  | Some p -> p
  | None -> failwith "build-seed: active_protos unset (bootstrap_protos must run first)"

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

(* Build a Cons-chain Form representing a list of Char codepoints.
   This is the canonical String-payload representation zig's
   transporter expects (see intrinsics.zig::extractPath): a sequence
   of FormRef-linked Cons cells whose :car is a Char value and :cdr
   is the next cell (or Nil at the tail).

   Each cons cell is allocated as a fresh Form here (NOT memoized via
   form_table — two strings with overlapping suffixes still get
   distinct cells because identity matters for cons chains we build
   by hand). Returns the FormRef of the head cell, or Nil for empty. *)
and build_char_chain (codepoints : int list) : Ast.form =
  let bp = get_protos () in
  let car_sym = Compiler.intern "car" in
  let cdr_sym = Compiler.intern "cdr" in
  let rec go = function
    | [] -> Ast.Nil
    | cp :: rest ->
        let tail = go rest in
        let cell = Image.{
          proto = Ast.FormRef bp.cons_id;
          slots = [(car_sym, Ast.Char cp); (cdr_sym, tail)];
          handlers = [];
          meta = [];
          frozen = true;
        } in
        let id = alloc_form_raw cell in
        Ast.FormRef id
  in
  go codepoints

(* Build a vat_form record for a non-scalar Ast.form. For Cons cells
   we emit :car and :cdr slots (which means 'car / 'cdr must be in
   the sym table - we call Compiler.intern to ensure that). *)
and build_form_for (v : Ast.form) : Image.vat_form =
  let bp = get_protos () in
  match v with
  | Ast.Str s ->
      (* moof String layout: proto=String, :bytes slot = cons-chain
         of Char codepoints. This is what zig's transporter expects;
         see crates/zig-substrate/src/intrinsics.zig::extractPath
         (it walks :bytes cell-by-cell, expecting .char car values).

         We use UTF-8 codepoints: decode each byte sequence to a
         single codepoint per char. OCaml's String is byte-indexed;
         use Uutf-style manual decoding to avoid external deps. *)
      let bytes_sym = Compiler.intern "bytes" in
      let codepoints = utf8_codepoints s in
      let chain = build_char_chain codepoints in
      Image.{
        proto = Ast.FormRef bp.string_id;
        slots = [(bytes_sym, chain)];
        handlers = [];
        meta = [];
        frozen = true;
      }
  | Ast.Bytes b ->
      (* Bytes follow the same shape — :bytes slot is a cons-chain
         of Int values (raw byte values, 0..255), proto=Bytes. *)
      let bytes_sym = Compiler.intern "bytes" in
      let car_sym = Compiler.intern "car" in
      let cdr_sym = Compiler.intern "cdr" in
      let rec build_int_chain pos =
        if pos >= Bytes.length b then Ast.Nil
        else
          let byte_v = Char.code (Bytes.get b pos) in
          let tail = build_int_chain (pos + 1) in
          let cell = Image.{
            proto = Ast.FormRef bp.cons_id;
            slots = [(car_sym, Ast.Int byte_v); (cdr_sym, tail)];
            handlers = [];
            meta = [];
            frozen = true;
          } in
          let id = alloc_form_raw cell in
          Ast.FormRef id
      in
      let chain = build_int_chain 0 in
      Image.{
        proto = Ast.FormRef bp.bytes_id;
        slots = [(bytes_sym, chain)];
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
        proto = Ast.FormRef bp.cons_id;
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
        proto = Ast.FormRef bp.object_id;
        slots = [(items_sym, items_form)];
        handlers = [];
        meta = [];
        frozen = true;
      }
  | _ -> failwith "build_form_for: unexpected scalar"

(* Decode a UTF-8 string into a list of Unicode codepoints. Used by
   build_form_for to lift String payloads into cons-chains of Char.

   Hand-rolled decoder: each leading byte determines the sequence
   length (1-4 bytes). Trailing bytes are masked + shifted. Invalid
   sequences fall back to byte-for-byte (treats invalid as Latin-1) —
   this is permissive enough for the minimum-viable boot; lossless
   round-trip will need a proper validator. *)
and utf8_codepoints (s : string) : int list =
  let len = String.length s in
  let rec go i acc =
    if i >= len then List.rev acc
    else
      let b0 = Char.code s.[i] in
      let (cp, next) =
        if b0 < 0x80 then
          (b0, i + 1)
        else if b0 < 0xC0 then
          (* stray continuation byte — treat as Latin-1 *)
          (b0, i + 1)
        else if b0 < 0xE0 && i + 1 < len then
          let b1 = Char.code s.[i + 1] in
          (((b0 land 0x1F) lsl 6) lor (b1 land 0x3F), i + 2)
        else if b0 < 0xF0 && i + 2 < len then
          let b1 = Char.code s.[i + 1] in
          let b2 = Char.code s.[i + 2] in
          (((b0 land 0x0F) lsl 12) lor
           ((b1 land 0x3F) lsl 6) lor
           (b2 land 0x3F), i + 3)
        else if b0 < 0xF8 && i + 3 < len then
          let b1 = Char.code s.[i + 1] in
          let b2 = Char.code s.[i + 2] in
          let b3 = Char.code s.[i + 3] in
          (((b0 land 0x07) lsl 18) lor
           ((b1 land 0x3F) lsl 12) lor
           ((b2 land 0x3F) lsl 6) lor
           (b3 land 0x3F), i + 4)
        else
          (b0, i + 1)
      in
      go next (cp :: acc)
  in
  go 0 []

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
  (* Pre-allocate boot protos so zig's installCaps + dispatch find
     them at known FormIds. Must happen BEFORE chunk compilation +
     lifting so the protos sit at the low FormIds the header expects.
     See bootstrap_protos doc. *)
  let bp = bootstrap_protos () in
  active_protos := Some bp;
  (* Allocate here_form + macros_form. patch_here_form populates the
     here_form's slots once we know its FormId. *)
  let (macros_form_id, here_form_id) = alloc_here_and_macros bp in
  patch_here_form here_form_id bp;
  (* Compile a single synthetic "main" top-level chunk wrapping every
     gathered step in `(do step1 step2 ...)`. zig's runRun expects one
     entry-point chunk reachable via the `main` slot on here_form; we
     bind it below.

     Why one chunk rather than N: each step is one top-level form like
     [$transporter load: "..."] that, when run, performs a side effect
     (load file, define proto, etc.). We need them all to run in order
     when the vat boots. compile_top on a single (do ...) form produces
     a chunk that executes them sequentially.

     Sub-chunks (closures, fn bodies) registered during compilation
     still land in the chunk registry normally. *)
  let main_form =
    let steps_forms = List.map (fun s -> s.form) steps in
    Ast.Cons (Ast.Sym "do", Ast.forms_to_list steps_forms)
  in
  let main_cb =
    try Compiler.compile_top main_form
    with Compiler.Compile_error msg ->
      Printf.eprintf "compile error in synthetic main: %s\n" msg;
      exit 1
  in
  let _ = Compiler.finalize main_cb in
  (* Allocate a fresh source-form Form per compiled chunk. zig's
     image-loader keys world.chunk_bytecode by source_form_id, so each
     chunk needs a distinct FormId or all chunks collapse into one
     entry (Bug 3). The chunk's compiler-id (cb.id) is the operand
     PushClosure emits — to keep that operand valid as a FormId after
     load, we rewrite the operand to point at the matching source-form
     FormId below.

     Each source-form Form is a minimal stub: proto=Chunk, no slots,
     no handlers. Eventually we'll lift the actual parsed source Form
     here (for reflection), but the empty-stub keeps dispatch happy. *)
  let all_cbs = Compiler.all_chunks () in
  let chunk_form_ids = List.map (fun (_ : Compiler.chunk_builder) ->
    alloc_form_raw Image.{
      proto = Ast.FormRef bp.chunk_id;
      slots = [];
      handlers = [];
      meta = [];
      frozen = true;
    }
  ) all_cbs in
  (* Map: chunk's compiler-id (cb.id) → its allocated FormId.
     We use this both for source_form_id on emission AND for rewriting
     PushClosure operands inside each chunk's op list. *)
  let id_map : (int, int) Hashtbl.t = Hashtbl.create 256 in
  List.iter2 (fun (cb : Compiler.chunk_builder) fid ->
    Hashtbl.add id_map cb.id fid
  ) all_cbs chunk_form_ids;
  let map_chunk_id (cid : int) : int =
    match Hashtbl.find_opt id_map cid with
    | Some f -> f
    | None ->
        failwith (Printf.sprintf
          "build-seed: PushClosure references unknown chunk-id %d" cid)
  in
  (* Rewrite every chunk's op list, substituting PushClosure operands.
     finalize re-encodes from b.ops on each call, so this mutation is
     picked up. Op sizes don't change (PushClosure is fixed at 5 bytes
     regardless of operand value), so byte positions / jump offsets
     are unaffected. *)
  List.iter (fun (cb : Compiler.chunk_builder) ->
    Dynarray.iteri (fun i op ->
      match op with
      | Opcodes.PushClosure cid ->
          Dynarray.set cb.ops i (Opcodes.PushClosure (map_chunk_id cid))
      | _ -> ()
    ) cb.ops
  ) all_cbs;
  (* Lift non-scalar consts into FormRefs. We walk every chunk's
     consts in registry order (deterministic) and rewrite each
     non-scalar to a FormRef pointing at a pre-allocated Form. *)
  let chunks =
    List.map2 (fun (cb : Compiler.chunk_builder) source_fid ->
      let f = Compiler.finalize cb in
      let lifted_consts = List.map lift_value f.consts in
      Image.{
        source_form_id = source_fid;
        body = f.body;
        consts = lifted_consts;
        ic_count = f.ic_count;
        params = f.params;
      }
    ) all_cbs chunk_form_ids
  in
  (* Bind `main` on here_form to the synthetic main chunk's FormId.
     zig's runRun looks here for the boot entry point. *)
  let main_fid = map_chunk_id main_cb.id in
  patch_form_slots here_form_id
    (Compiler.intern "main", Ast.FormRef main_fid);
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
    here_form_id;
    macros_form_id;
    protos = protos_table bp;
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
