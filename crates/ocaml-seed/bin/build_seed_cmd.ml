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
        contents (gather_from_file recurses). These chunks are
        pre-compiled into seed.vat so the parser+compiler are
        available the moment the image loads.
     2. a transporter load whose path is anything else (early/,
        stdlib/, mcos.moof, etc.) - we DO NOT pre-compile, but we
        INCLUDE the raw `[$transporter load: "X"]` form in main's
        chunk. At runtime, moof parser+compiler (now loaded) drive
        the load. This is the point: exercise the runtime path.
     3. a `__cascade__` form - cascade segments are expanded into
        individual sends, then each is processed by the rules above.
        (Cascades in main.moof are exclusively load-cascades —
        `[$transporter load: "a" ; load: "b" ; ...]`.)
     4. anything else (e.g. `[$compiler useMoof]`) - include the
        form in main's chunk. The moof runtime executes it.

   Files reached via (1) are scanned the same way - if a parser file
   loads another parser file, we follow.
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

(* Match (__cascade__ recv (sel args...) (sel2 args2...) ...) — emitted
   by the OCaml reader for any send-bracket containing `;`. We only
   care about load-cascades on $transporter, where each segment is
   (load: "path"). Returns Some [path1; path2; ...] on match, None
   otherwise. *)
let match_transporter_load_cascade (form : Ast.form) : string list option =
  match form with
  | Ast.Cons (Ast.Sym "__cascade__",
      Ast.Cons (Ast.Sym "$transporter", segments)) ->
      let rec each segs acc =
        match segs with
        | Ast.Nil -> Some (List.rev acc)
        | Ast.Cons (seg, rest) ->
            (match seg with
             | Ast.Cons (Ast.Sym "load:",
                 Ast.Cons (Ast.Str path, Ast.Nil)) ->
                 each rest (path :: acc)
             | _ -> None)
        | _ -> None
      in
      each segments []
  | _ -> None

(* Re-emit a bare transporter load form: (__send__ $transporter load: path). *)
let mk_transporter_load (path : string) : Ast.form =
  Ast.forms_to_list [
    Ast.Sym "__send__";
    Ast.Sym "$transporter";
    Ast.Sym "load:";
    Ast.Str path;
  ]

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

(* gather_from_file recurses into parser/* and compiler/* files,
   appending their forms to `acc` as load_steps. For main.moof
   specifically, we ALSO accumulate a "main forms" list:
   forms-as-AST that go directly into the synthetic main chunk
   for runtime execution (no pre-compilation). This list mixes
   parser/+compiler/ forms (which get pre-compiled — represented
   inside main as... well, currently they're emitted via the
   pre-compile path; main re-references them only insofar as the
   chunks land in the registry and the seed's main `(do ...)`
   wrapper includes the pre-compiled load_steps).

   The contract:
     - `acc` accumulates forms to PRE-COMPILE (parser/+compiler/
       internals). These chunks land in seed.vat as compiled
       bytecode; main runs them via standard sequencing.
     - `main_forms` accumulates forms in main.moof order to RUN
       AT RUNTIME via the moof parser+compiler. This includes:
         - non-load forms like [$compiler useMoof]
         - transporter loads with non-minimal paths (early/, stdlib/)
       Parser/+compiler/ loads in main.moof are RESOLVED at seed
       build time — their internals get pre-compiled, so there's
       no need to keep the [$transporter load: "parser/..."] in
       main_forms.

   The final synthetic main chunk wraps `(acc-as-do-sequence ++
   main_forms)`. The pre-compiled parser/+compiler/ steps execute
   first (no transporter calls — just their compiled bodies in
   order), then [$compiler useMoof] flips, then the early/+stdlib/
   loads each run through the moof parser+compiler at runtime. *)
let rec gather_from_file (root : string) (rel_path : string)
                         (acc : load_step list ref)
                         (main_forms : Ast.form list ref) : unit =
  let full = Filename.concat root rel_path in
  let tolerant = (rel_path = "main.moof") in
  let forms = read_top_forms full tolerant in
  List.iter (fun form ->
    let process_in_main path =
      (* This branch only fires for main.moof's top-level forms. *)
      if is_minimal_bootstrap_path path then
        (* parser/+compiler/ load: recurse to pre-compile its
           contents. Do NOT add to main_forms — the recursive
           pre-compile + main's `(do ...)` over acc handles
           sequencing. *)
        gather_from_file root path acc main_forms
      else
        (* non-minimal load: keep as a bare load form in main
           for the moof runtime to execute via parser+compiler. *)
        main_forms := mk_transporter_load path :: !main_forms
    in
    if rel_path = "main.moof" then begin
      match match_transporter_load form with
      | Some path -> process_in_main path
      | None ->
          (match match_transporter_load_cascade form with
           | Some paths ->
               (* expand the cascade: each segment becomes its own
                  load form, processed by the same rules. *)
               List.iter process_in_main paths
           | None ->
               (* not a load — include in main_forms so the runtime
                  executes it (e.g. [$compiler useMoof]). *)
               main_forms := form :: !main_forms)
    end else begin
      (* parser/* or compiler/* file: every form is a real
         compilable definition. Includes nested transporter loads,
         which we follow. *)
      match match_transporter_load form with
      | Some sub_path when is_minimal_bootstrap_path sub_path ->
          gather_from_file root sub_path acc main_forms
      | Some _ ->
          (* parser/+compiler/ shouldn't load early/+stdlib/, but
             if they did, the moof runtime can't help (we're still
             in the pre-flip world). Bail out with a clear error. *)
          failwith (Printf.sprintf
            "build-seed: %s loads non-minimal path; only parser/+compiler/ \
             loads allowed inside the pre-compiled prefix" rel_path)
      | None ->
          acc := { source_path = rel_path; form } :: !acc
    end
  ) forms

(* Top-level entry: returns (precompile_steps, runtime_main_forms).
   - precompile_steps: forms from parser/+compiler/, in load order,
     to be compiled into seed.vat's chunk table.
   - runtime_main_forms: forms left in main.moof (the [$compiler
     useMoof] flip + the early/+stdlib/+mcos load chain) that the
     moof runtime will execute via its in-image parser+compiler. *)
let gather_steps (root : string) : load_step list * Ast.form list =
  let acc = ref [] in
  let main_forms = ref [] in
  gather_from_file root "main.moof" acc main_forms;
  (List.rev !acc, List.rev !main_forms)

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
(* The lifted forms, indexed by FormId (1-based). We use a Dynarray
   keyed by FormId so each form lands at the correct wire position
   regardless of allocation interleaving. (List-based prepend caused
   a wire-id mismatch when nested forms — e.g. char-cells inside a
   String — got allocated AFTER their parent's id was reserved but
   appended to the list before the parent.) *)
let lifted_forms : Image.vat_form option Dynarray.t = Dynarray.create ()

(* Reset the lifter's state. Call between independent build-seed runs
   to keep things deterministic. *)
let reset_lifter () =
  Hashtbl.clear form_table;
  next_form_id := 1;
  Dynarray.clear lifted_forms

(* Reserve a fresh FormId, ensuring the indexed array has a slot for
   it (filled with None until the form is populated). Returns the
   reserved id. *)
let reserve_form_id () : int =
  let id = !next_form_id in
  incr next_form_id;
  (* slot index = id - 1 (1-based wire id). *)
  Dynarray.add_last lifted_forms None;
  id

(* Fill a previously-reserved slot with the actual form. *)
let put_form (id : int) (f : Image.vat_form) : unit =
  Dynarray.set lifted_forms (id - 1) (Some f)

(* Allocate a fresh Form at the next FormId, append to the FormSection
   in id-order, return the assigned FormId. NOT memoized — caller is
   responsible for de-duplication when appropriate. Used by proto /
   here_form / macros_form / per-chunk-source allocation. *)
let alloc_form_raw (f : Image.vat_form) : int =
  let id = reserve_form_id () in
  put_form id f;
  id

(* Snapshot the final FormSection contents in id-order. Raises if any
   reserved id was never filled (indicates a lift bug). *)
let finalize_lifted_forms () : Image.vat_form list =
  Dynarray.to_seq lifted_forms
  |> Seq.mapi (fun i o ->
       match o with
       | Some f -> f
       | None ->
           failwith (Printf.sprintf
             "build-seed: FormId %d was reserved but never populated" (i + 1)))
  |> List.of_seq

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

(* The boot proto FormIds, captured after allocation. Field order
   roughly tracks V4 image header proto-table order (spec §10.3 and
   image.ml::proto_table) — but the wire table is locked at 18
   canonical entries, so `float_id` lives outside it. Float is
   allocated like any other proto and bound by name on here_form, but
   omitted from `protos_table` / header. Once zig substrate grows a
   `Protos.float` field (today its `protoOf(.float)` returns Object —
   see crates/zig-substrate/src/world.zig:715), the V4 header layout
   will need a wire-format bump to carry the id; for now we just need
   `Float` resolvable as a name so stdlib/float.moof can `defmethod`
   onto it. *)
type boot_protos = {
  object_id   : int;
  nil_id      : int;
  bool_id     : int;
  integer_id  : int;
  float_id    : int;
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
  (* Float: proto chain is Object (mirrors crates/substrate/src/protos.rs:75
     and Integer). Not part of the V4 image proto-table (header carries
     18 entries; Float is the 19th), but needs a Form so user-level
     `defmethod Float ...` resolves. *)
  let float_id = p "Float" in
  let char_id = p "Char" in
  let sym_id = p "Symbol" in
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
    object_id; nil_id; bool_id; integer_id; float_id; char_id; sym_id;
    cons_id; string_id; bytes_id; method_id; chunk_id; closure_id;
    env_id; foreign_id; table_id; frame_id; macros_id; opcode_id;
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
   resolves). We also seed bindings for the 18 canonical proto names
   (Object, Cons, …, Opcode) plus Float so user code can reach them
   by name.

   macros_form is the canonical macro registry — a plain Object-proto
   Form whose slots will hold macro-name → method-Form once macros
   load. Empty here; the runtime populates as user code defines.
   ---------------------------------------------------------------- *)

(* Allocate here_form, returning (macros_form_id, here_form_id). Must
   be called after bootstrap_protos so we know Env / Object FormIds.

   IMPORTANT: macros_form_id == bp.macros_id (single Form serves as
   both the canonical macro registry — what `Macros` symbol resolves
   to in here_form, what world.macros_form points to, what defmacro
   slotSet!s into — AND the V4-image proto-table's "macros" entry).
   rust's v4_export.rs emits world.macros_form as both
   header.macros_form_id and protos[16]; we mirror that aliasing. *)
let alloc_here_and_macros (bp : boot_protos) : int * int =
  let parent_sym = Compiler.intern "parent" in
  (* here_form: proto=Env, :meta parent=Nil. Slots populated below. *)
  let here_id = alloc_form_raw Image.{
    proto = Ast.FormRef bp.env_id;
    slots = [];          (* filled in by patch_here_form below *)
    handlers = [];
    meta = [(parent_sym, Ast.Nil)];
    frozen = false;
  } in
  (bp.macros_id, here_id)

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
  let float_s = Compiler.intern "Float" in
  let char_s = Compiler.intern "Char" in
  (* canonical user-facing name is "Symbol" — matches rust substrate
     and what moof code uses (e.g. early/04-symbol.moof's `Symbol`
     binding sends). zig substrate's proto field is .sym for brevity. *)
  let sym_s = Compiler.intern "Symbol" in
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
    (float_s,     Ast.FormRef bp.float_id);
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
  (* lifted_forms is indexed by FormId-1; just overwrite the slot.
     Raises if the id was never reserved (caller bug). *)
  match Dynarray.get lifted_forms (here_id - 1) with
  | None ->
      failwith (Printf.sprintf
        "patch_here_form: FormId %d never populated" here_id)
  | Some f ->
      Dynarray.set lifted_forms (here_id - 1)
        (Some Image.{ f with slots = here_slots })

(* Append a single (sym, value) pair to an already-allocated Form's
   slots list. Like patch_here_form but additive — preserves existing
   slots and appends a new one. Used to bind `main` on here_form once
   we know the boot chunk's FormId. *)
let patch_form_slots (target_id : int) (slot : int * Ast.form) : unit =
  match Dynarray.get lifted_forms (target_id - 1) with
  | None ->
      failwith (Printf.sprintf
        "patch_form_slots: FormId %d not in lifted_forms" target_id)
  | Some f ->
      Dynarray.set lifted_forms (target_id - 1)
        (Some Image.{ f with slots = f.slots @ [slot] })

(* Append a single (sym, value) pair to an already-allocated Form's
   handlers list. Mirrors patch_form_slots but writes to .handlers.
   Used by wire_natives to install method-Forms on proto handler
   tables (same shape rust's `form_handler_set` produces). *)
let patch_form_handlers (target_id : int) (handler : int * Ast.form) : unit =
  match Dynarray.get lifted_forms (target_id - 1) with
  | None ->
      failwith (Printf.sprintf
        "patch_form_handlers: FormId %d not in lifted_forms" target_id)
  | Some f ->
      Dynarray.set lifted_forms (target_id - 1)
        (Some Image.{ f with handlers = f.handlers @ [handler] })

(* ----------------------------------------------------------------
   native handler wiring (NativeRefsSection)

   Mirror of crates/zig-substrate/src/intrinsics.zig REGISTRY: a list
   of "ProtoName:selector" keys that the zig host's image-load step
   rebinds to NativeFn pointers via World.lookupNativeByName.

   For each entry:
     1. Look up (or allocate) the target proto Form by name.
     2. Alloc a fresh method-Form (proto=Method, no body, no slots).
     3. Append (selector-sym-id, FormRef method-id) to the proto's
        handlers list.
     4. Emit (method-id, "ProtoName:selector") into NativeRefsSection.

   At load time, zig's readNativeRefs does:
        world.native_fns.put(method_id, REGISTRY.get(name).?)
   so dispatch finds the native via the same path moof methods use
   (lookup on proto.handlers → method-Form → native_fns).

   Protos handled:
     - The 18 canonical boot protos (Object, Cons, Nil, ..., Opcode):
       allocated by bootstrap_protos already.
     - Two singletons created here: `Heap`, `Chunks`. These are
       Object-proto Forms with :name meta + bound on here_form so
       moof code can write `[Heap slotOf: x at: 'car]`.
     - Skipped: Transporter, Compiler, Reader. zig's host calls
       intrinsics.installCaps AFTER image-load to wire those — they
       carry $-prefixed env bindings (`$transporter` etc.), not
       canonical proto names, so seed.vat doesn't pre-bind them.

   ---------------------------------------------------------------- *)

(* The list of native REGISTRY keys, hardcoded to mirror zig's
   intrinsics.zig::REGISTRY. Order doesn't matter for correctness —
   readNativeRefs is order-independent — but matching the zig file's
   order is easier to audit. Keep this in sync manually when zig adds
   natives; future cleanup could read a shared manifest file. *)
let zig_registry_keys : string list = [
  (* Integer arithmetic *)
  "Integer:+";
  "Integer:-";
  "Integer:*";
  "Integer:/";
  "Integer:=";
  "Integer:<";
  "Integer:>";
  "Integer:toString";
  (* truthiness *)
  "Object:!!";
  "Nil:!!";
  "Bool:!!";
  (* Object basics — identity / reflection *)
  "Object:is";
  "Object:proto";
  "Object:identity";
  "Object:slot:";
  "Object:slotSet!:";
  (* Cons accessors *)
  "Cons:car";
  "Cons:cdr";
  (* Env API *)
  "Env:bind:to:";
  "Env:set:to:";
  "Env:lookup:";
  "Env:parent";
  "Env:current";
  (* Closure invocation *)
  "Closure:callIn:withSelf:";
  (* Object meta *)
  "Object:become:";
  "Object:doesNotUnderstand:with:";
  "Object:perform:withArgs:";
  "Bool:ifTrue:ifFalse:";
  "Object:toString";
  "Object:serializeTo:";
  (* Opcode constructors *)
  "Opcode:pushNil";
  "Opcode:pushTrue";
  "Opcode:pushFalse";
  "Opcode:pop";
  "Opcode:dup";
  "Opcode:loadSelf";
  "Opcode:return";
  "Opcode:loadConst:";
  "Opcode:loadName:";
  "Opcode:pushClosure:";
  "Opcode:jump:";
  "Opcode:jumpIfFalse:";
  "Opcode:send:argc:ic:";
  "Opcode:tailSend:argc:";
  "Opcode:superSend:argc:ic:";
  "Opcode:sendSelf:argc:ic:";
  (* Opcode reflection *)
  "Opcode:op";
  "Opcode:operands";
  "Opcode:toString";
  (* Chunks singleton *)
  "Chunks:isChunk?:";
  "Chunks:paramsListOf:";
  "Chunks:constsListOf:";
  "Chunks:opsListOf:";
  "Chunks:icsListOf:";
  "Chunks:bodyOf:";
  (* Heap singleton *)
  "Heap:protoOf:";
  "Heap:heapIdOf:";
  "Heap:allocFormWithProto:";
  "Heap:slotOf:at:";
  "Heap:handlerOf:at:";
  "Heap:metaOf:at:";
  (* Method invocation *)
  "Method:call";
  (* Object equality + lifecycle *)
  "Object:=";
  "Object:new";
  "Object:initialize";
  "Object:freeze";
  "Object:frozen?";
  "Object:freezable?";
  (* Cons / Nil *)
  "Cons:cons:";
  "Nil:cons:";
  "Cons:empty?";
  "Cons:null?";
  "Cons:nonEmpty?";
  "Nil:empty?";
  "Nil:proto";
  "Cons:reverse";
  (* Transporter / Compiler / Reader are SKIPPED — host installCaps
     wires those AFTER image-load (anonymous-proto names + $-env
     binding diverge from canonical NativeRefs path). *)

  (* Free-function globals — bound on here_form by NAME (not on any
     proto's handler table). port of rust install_global. moof code
     calls these as `(name arg…)` which lowers to LoadName + Send :call,
     so the method-Form must:
       - have proto = Method (so Send :call finds Method:call handler)
       - sit in here_form's slots under the unprefixed name
       - have its FormId recorded in NativeRefsSection so zig binds
         the NativeFn at load time
     wire_natives recognizes the `Global:` prefix and routes through a
     different allocation path — see wire_global_native there. *)
  "Global:setHandler!";
  "Global:intern";
  "Global:cons";
  "Global:list";
  "Global:raise:";
  "Global:slot";
  "Global:slotSet!";
  "Global:metaSet!";
  "Global:globalEnv";
  "Global:getOrCreateProto";
  "Global:append";
  "Global:macroexpand";
  (* String primitives — parser uses these heavily. *)
  "String:length";
  "String:at:";
  "String:=";
  "String:slice:length:";
  "String:+";
  (* Char primitives. *)
  "Char:codepoint";
  "Char:<";
  "Char:toString";
  (* Integer:asChar — coerce Int → Char. *)
  "Integer:asChar";
  (* Chunk class- + instance-side methods — moof Compiler primitives. *)
  "Chunk:new:source:";
  "Chunk:emit:";
  "Chunk:addConst:";
  "Chunk:addIc";
  "Chunk:jumpTarget";
  "Chunk:patchJump:to:";
  "Chunk:asClosure";
]

(* Split "ProtoName:rest" into (proto, selector). Selector keeps any
   internal colons (e.g. "bind:to:" stays as a single selector). *)
let split_native_key (key : string) : string * string =
  match String.index_opt key ':' with
  | None -> failwith (Printf.sprintf "native key missing colon: %S" key)
  | Some i ->
      let proto = String.sub key 0 i in
      let sel = String.sub key (i + 1) (String.length key - i - 1) in
      (proto, sel)

(* Build a name → FormId map for the 18 canonical boot protos. Used to
   resolve REGISTRY keys; entries for Heap / Chunks are added on
   demand once those singletons are allocated. *)
let canonical_proto_map (bp : boot_protos) : (string, int) Hashtbl.t =
  let h = Hashtbl.create 32 in
  Hashtbl.add h "Object"        bp.object_id;
  Hashtbl.add h "Nil"           bp.nil_id;
  Hashtbl.add h "Bool"          bp.bool_id;
  Hashtbl.add h "Integer"       bp.integer_id;
  Hashtbl.add h "Float"         bp.float_id;
  Hashtbl.add h "Char"          bp.char_id;
  Hashtbl.add h "Symbol"        bp.sym_id;
  Hashtbl.add h "Cons"          bp.cons_id;
  Hashtbl.add h "String"        bp.string_id;
  Hashtbl.add h "Bytes"         bp.bytes_id;
  Hashtbl.add h "Method"        bp.method_id;
  Hashtbl.add h "Chunk"         bp.chunk_id;
  Hashtbl.add h "Closure"       bp.closure_id;
  Hashtbl.add h "Env"           bp.env_id;
  Hashtbl.add h "ForeignHandle" bp.foreign_id;
  Hashtbl.add h "Table"         bp.table_id;
  Hashtbl.add h "Frame"         bp.frame_id;
  Hashtbl.add h "Macros"        bp.macros_id;
  Hashtbl.add h "Opcode"        bp.opcode_id;
  h

(* Allocate an Object-proto singleton named `name`. Returns the
   FormId. Used for Heap / Chunks singletons that aren't in the 18
   canonical protos but still need a proto-Form with handlers + a
   `:name` meta tag so reflection works. *)
let alloc_singleton (bp : boot_protos) (name : string) : int =
  let name_meta = Compiler.intern "name" in
  alloc_form_raw Image.{
    proto = Ast.FormRef bp.object_id;
    slots = [];
    handlers = [];
    meta = [(name_meta, Ast.Sym name)];
    frozen = false;
  }

(* Wire every REGISTRY native onto its target proto. Returns the
   list of (method_form_id, "ProtoName:selector") pairs that will
   populate NativeRefsSection.

   Side effects:
     - allocates Heap + Chunks singleton Forms
     - binds Heap / Chunks on here_form
     - allocates one method-Form per REGISTRY entry
     - patches proto handlers tables with the method-Forms

   The here_form binding for Heap / Chunks is APPENDED (via
   patch_form_slots) so we don't disturb the proto bindings that
   patch_here_form set up earlier. *)
let wire_natives (bp : boot_protos) (here_form_id : int)
                 : Image.native_ref list =
  let proto_ids = canonical_proto_map bp in
  (* Allocate singletons + bind them on here_form. *)
  let heap_id = alloc_singleton bp "Heap" in
  let chunks_id = alloc_singleton bp "Chunks" in
  Hashtbl.add proto_ids "Heap" heap_id;
  Hashtbl.add proto_ids "Chunks" chunks_id;
  patch_form_slots here_form_id
    (Compiler.intern "Heap", Ast.FormRef heap_id);
  patch_form_slots here_form_id
    (Compiler.intern "Chunks", Ast.FormRef chunks_id);
  (* For each REGISTRY key: alloc method-Form, patch proto handlers (or
     bind on here_form for `Global:` keys), accrue the NativeRefs entry. *)
  List.fold_left (fun acc key ->
    let (proto_name, selector) = split_native_key key in
    if proto_name = "Global" then begin
      (* Free-function global. Bind on here_form's slots under the
         unprefixed name. The method-Form has proto=Method so a
         Send :call from `(name arg…)` walks to Method:call's handler,
         which checks native_fns and dispatches. *)
      let name_sym = Compiler.intern selector in
      let method_id = alloc_form_raw Image.{
        proto = Ast.FormRef bp.method_id;
        slots = [];
        handlers = [];
        meta = [];
        frozen = false;
      } in
      patch_form_slots here_form_id (name_sym, Ast.FormRef method_id);
      Image.{ method_form_id = method_id; native_name = key } :: acc
    end else
      match Hashtbl.find_opt proto_ids proto_name with
      | None ->
          Printf.eprintf
            "build-seed: warning: REGISTRY key %S has no proto target; skipping\n"
            key;
          acc
      | Some proto_id ->
          let sel_sym = Compiler.intern selector in
          (* alloc method-Form: proto = Method, otherwise empty. *)
          let method_id = alloc_form_raw Image.{
            proto = Ast.FormRef bp.method_id;
            slots = [];
            handlers = [];
            meta = [];
            frozen = false;
          } in
          patch_form_handlers proto_id (sel_sym, Ast.FormRef method_id);
          Image.{ method_form_id = method_id; native_name = key } :: acc
  ) [] zig_registry_keys
  |> List.rev

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
  (* compounds lift to FormRef. We reserve the parent's id BEFORE
     building its content so any child allocations (in build_form_for
     → build_char_chain / build_int_chain / lift_value) end up with
     wire ids AFTER the parent's. This keeps wire-position(form) ==
     internal-id(form) across the recursive lift. *)
  | Ast.Str _ | Ast.Bytes _ | Ast.Cons _ | Ast.Vec _ ->
      (match Hashtbl.find_opt form_table v with
       | Some id -> Ast.FormRef id
       | None ->
           let id = reserve_form_id () in
           Hashtbl.add form_table v id;
           let form = build_form_for v in
           put_form id form;
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
  (* Allocate cells in HEAD-FIRST order so wire-id(head) < wire-id(tail).
     We reserve each cell's id before recursing so child allocations
     pick up the next-larger id naturally. *)
  let rec go = function
    | [] -> Ast.Nil
    | cp :: rest ->
        let id = reserve_form_id () in
        let tail = go rest in
        let cell = Image.{
          proto = Ast.FormRef bp.cons_id;
          slots = [(car_sym, Ast.Char cp); (cdr_sym, tail)];
          handlers = [];
          meta = [];
          frozen = true;
        } in
        put_form id cell;
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
         of Int values (raw byte values, 0..255), proto=Bytes.
         Head-first id allocation, same reasoning as build_char_chain. *)
      let bytes_sym = Compiler.intern "bytes" in
      let car_sym = Compiler.intern "car" in
      let cdr_sym = Compiler.intern "cdr" in
      let rec build_int_chain pos =
        if pos >= Bytes.length b then Ast.Nil
        else
          let byte_v = Char.code (Bytes.get b pos) in
          let id = reserve_form_id () in
          let tail = build_int_chain (pos + 1) in
          let cell = Image.{
            proto = Ast.FormRef bp.cons_id;
            slots = [(car_sym, Ast.Int byte_v); (cdr_sym, tail)];
            handlers = [];
            meta = [];
            frozen = true;
          } in
          put_form id cell;
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
  let (steps, runtime_forms) = gather_steps opts.root in
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
  (* Wire native methods onto proto handler tables + emit a matching
     NativeRefsSection. This unblocks the moof source from doing
     [Object new] / [cons :car ...] / etc. after image-load — the
     handlers point at method-Forms whose `:body` is implicit, and
     zig's readNativeRefs binds REGISTRY[name] to each method's id.
     Also allocates Heap + Chunks singletons and binds them on
     here_form so parser/01-tokens.moof's `[Heap slotOf: ...]` works. *)
  let native_refs = wire_natives bp here_form_id in
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
  (* Synthetic main: precompiled steps (parser/+compiler/ forms) run
     first; then the runtime_forms (the [$compiler useMoof] flip plus
     the early/+stdlib/+mcos transporter-load chain). The runtime
     forms execute via the moof parser+compiler that the precompiled
     prefix just installed. *)
  let main_form =
    let steps_forms = List.map (fun s -> s.form) steps in
    Ast.Cons (Ast.Sym "do",
              Ast.forms_to_list (steps_forms @ runtime_forms))
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
  let forms = finalize_lifted_forms () in
  let vat = Image.{
    vat_id = Bytes.make 16 '\x00';       (* deterministic placeholder *)
    syms = Compiler.all_syms ();
    forms;
    chunks;
    natives = native_refs;
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
  Printf.printf "wrote %s (%d bytes, %d chunks, %d syms, %d forms, %d natives, %d files, %d runtime-forms)\n"
    opts.output
    (Bytes.length bytes)
    (List.length chunks)
    (List.length vat.syms)
    (List.length forms)
    (List.length native_refs)
    (List.length (List.sort_uniq compare
                    (List.map (fun s -> s.source_path) steps)))
    (List.length runtime_forms)
