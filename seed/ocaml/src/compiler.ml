(* compiler.ml — moof source Form → V4 byte-tagged bytecode.

   ports players/rust/src/compiler.rs (the rust seed) to OCaml,
   with V4 emission rules layered on per the V4 opcode design spec
   (docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md, §5).

   responsibilities (same as the rust seed — sized to compile
   lib/compiler/*.moof and nothing more):

   | form                           | emits                          |
   |--------------------------------|--------------------------------|
   | (def name expr)                | SendHere :bind:to: (V4 fused)  |
   | (set! name expr)               | LoadName Env; Send :current;   |
   |                                |   Send :set:to:                |
   | (fn (params…) body…)           | sub-chunk + PushClosure        |
   | (if cond then [else])          | (Send-shape with peephole →    |
   |                                |  JumpIfFalse/Jump inline)      |
   | (let ((n v)…) body…)           | ((fn (n…) body) v…) desugar    |
   | (do e1 … eN)                   | sequence + Pop intermediates   |
   | (quote v)                      | LoadConst                      |
   | (__send__ recv 'sel args…)     | Send / SendSelf / SendHere /   |
   |                                |   SuperSend / TailSend variants|
   | (callable args…)               | Send :call (call dispatch)     |

   V4 fusion rules (spec §5.1):
   - receiver is the symbol `self`  → SendSelf  / TailSendSelf
   - receiver is the symbol `$here` → SendHere  / TailSendHere
   - selector is :perform:withArgs: → SendDynamic
   - otherwise                      → Send / TailSend / SuperSend

   the V3 const-fold + if-peephole optimizations are deferred to the
   moof Compiler (lib/compiler/02-special.moof) — the seed compiler
   emits the straightforward shape. exception: `if` is lowered to
   Jump-based bytecode directly (skipping the :ifTrue:ifFalse: Send),
   matching the rust seed's pre-V3-task-13 behavior. this is faster
   and simpler for the seed; the moof Compiler (loaded later) emits
   the full Send-based shape for user code.

   per "Bytecode from day one" — the API surface is the byte-encoded
   chunk, not the in-memory Op list. internally we build an op-list
   for ease of jump-patching, then encode to bytes at finalize. *)

open Ast
open Opcodes
(* Bytecode is referenced qualified (Bytecode.encode_op) only — no open. *)

(* ─────────────────────────────────────────────────────────────────
   compile errors.
   ───────────────────────────────────────────────────────────────── *)

exception Compile_error of string

let err msg = raise (Compile_error msg)

(* ─────────────────────────────────────────────────────────────────
   the SymId model. moof's symbol table interning is per-vat at
   runtime; here at compile time we use a process-wide table that
   the image serializer (OCAML-4) will canonicalize.
   ───────────────────────────────────────────────────────────────── *)

let sym_table : (string, int) Hashtbl.t = Hashtbl.create 256
let sym_names : string Dynarray.t = Dynarray.create ()

(** intern a string into the symbol table, returning its SymId.

    SymIds are 1-based to match the zig substrate's convention
    (players/zig/src/sym.zig — entries[0] = NONE sentinel,
    first user-interned sym lands at SymId 1). The image's
    SymTableSection serializes the entries in encounter order, so
    OCaml's `intern("foo") = N` lines up with zig's
    `intern("foo") = N` after image-load. *)
let intern (name : string) : int =
  match Hashtbl.find_opt sym_table name with
  | Some id -> id
  | None ->
      let id = Dynarray.length sym_names + 1 in
      Hashtbl.add sym_table name id;
      Dynarray.add_last sym_names name;
      id

(** resolve a SymId back to its name. raises if unknown. *)
let sym_name (id : int) : string =
  Dynarray.get sym_names (id - 1)

(** all interned symbols, in interning order (canonical for the
    SymTableSection of the image). *)
let all_syms () : string list =
  Dynarray.to_list sym_names

type chunk_builder = {
  ops : op Dynarray.t;
  op_bytepos : int Dynarray.t;  (* byte position of each op, parallel to ops *)
  consts : form Dynarray.t;
  mutable ic_count : int;
  mutable next_pc : int;
  params : int list;
  source : form;
  id : int;
}

(* ─────────────────────────────────────────────────────────────────
   global chunk registry.

   chunks are assigned monotonic FormIds at construction time. nested
   chunks (from compile_fn / compile_let / compile_if thunks) register
   before their parent finalizes, so the parent's PushClosure can
   reference them. depth-first order — deterministic (V4 spec §9).

   the image serializer (OCAML-4) reads `all_chunks ()` after the
   top-level compile to produce the ChunkSection.
   ───────────────────────────────────────────────────────────────── *)

let chunk_registry : chunk_builder Dynarray.t = Dynarray.create ()

let register_chunk (b : chunk_builder) : unit =
  Dynarray.add_last chunk_registry b

let all_chunks () : chunk_builder list =
  Dynarray.to_list chunk_registry

(** reset the global state — useful between independent compiles
    (e.g. tests, or per-source-file build-image runs). *)
let reset_globals () : unit =
  Hashtbl.clear sym_table;
  Dynarray.clear sym_names;
  Dynarray.clear chunk_registry

(** import a list of symbols into the table, preserving their order.
    useful for aligning with a hydrated vat's sym table. *)
let import_syms (names : string list) : unit =
  reset_globals ();
  List.iter (fun n -> ignore (intern n)) names

(* ─────────────────────────────────────────────────────────────────
   byte-size of each opcode (V4 spec §3). used by `emit` to advance
   the byte cursor and by `patch_jump_to_here` to compute byte-deltas.
   ───────────────────────────────────────────────────────────────── *)

let byte_size_of_op (o : op) : int =
  match o with
  | PushNil | PushTrue | PushFalse -> 1
  | LoadConst _ -> 3  (* tag + u16 *)
  | LoadSelf | LoadHere -> 1
  | LoadName _ -> 5  (* tag + u32 *)
  | Pop | Dup -> 1
  | Send _ -> 8                (* tag + u32 sel + u8 argc + u16 ic *)
  | TailSend _ -> 6            (* tag + u32 sel + u8 argc *)
  | SuperSend _ -> 8
  | SendDynamic _ -> 4         (* tag + u8 argc + u16 ic *)
  | SendSelf _ -> 8
  | SendHere _ -> 8
  | TailSendSelf _ -> 6
  | TailSendHere _ -> 6
  | Jump _ | JumpIfFalse _ | JumpIfTrue _ -> 3  (* tag + i16 *)
  | Return -> 1
  | PushClosure _ -> 5         (* tag + u32 chunk-id *)
  | Suspend _ -> 3             (* tag + u16 promise-ic *)
  | Resume _ -> 3              (* tag + u16 frame-ic *)

(* ─────────────────────────────────────────────────────────────────
   public chunk-builder API.
   ───────────────────────────────────────────────────────────────── *)

(** allocate a fresh chunk-builder. registers it in the global chunk
    registry, assigning a FormId-like monotonic chunk id. *)
let make ?(params : int list = []) (source : form) : chunk_builder =
  let id = Dynarray.length chunk_registry in
  let b = {
    ops = Dynarray.create ();
    op_bytepos = Dynarray.create ();
    consts = Dynarray.create ();
    ic_count = 0;
    next_pc = 0;
    params;
    source;
    id;
  } in
  register_chunk b;
  b

(** emit an op into the builder. returns the byte position at which
    the op was emitted — useful for jump patching (see
    `patch_jump_to_here`). *)
let emit (b : chunk_builder) (o : op) : int =
  let pos = b.next_pc in
  Dynarray.add_last b.ops o;
  Dynarray.add_last b.op_bytepos pos;
  b.next_pc <- pos + byte_size_of_op o;
  pos

(** ensure every Sym contained in this form has been interned into the
    global sym table. constants reach the image-write step as Ast.form
    values; any Sym name they reference must be in the sym table so the
    image serializer can resolve it via build_sym_lookup.

    walks Cons trees structurally. Other compound forms (Vec) shouldn't
    appear in the minimal subset but are walked defensively. *)
let rec intern_syms_in_form (v : form) : unit =
  match v with
  | Sym s -> let _ = intern s in ()
  | Cons (h, t) -> intern_syms_in_form h; intern_syms_in_form t
  | Vec xs -> List.iter intern_syms_in_form xs
  | _ -> ()

(** add a Form to the constant pool, returning its u16 index.
    constants are deduplicated by structural equality so the pool
    stays small (V4 spec §9: same source → same chunk bytes).

    Every Sym leaf inside the constant is also interned into the
    global sym table - constants travel through the image as Ast.form
    values, and the image serializer must be able to resolve their
    Sym names via the SymTableSection. *)
let add_const (b : chunk_builder) (v : form) : int =
  intern_syms_in_form v;
  let n = Dynarray.length b.consts in
  let rec find i =
    if i >= n then None
    else if Dynarray.get b.consts i = v then Some i
    else find (i + 1)
  in
  match find 0 with
  | Some i -> i
  | None ->
      if n >= 65536 then err "constant pool overflow (>65535)";
      Dynarray.add_last b.consts v;
      n

(** reserve a fresh IC slot index. *)
let next_ic (b : chunk_builder) : int =
  let idx = b.ic_count in
  if idx >= 65536 then err "IC pool overflow (>65535)";
  b.ic_count <- idx + 1;
  idx

(* ─────────────────────────────────────────────────────────────────
   jump patching.

   the protocol: emit a placeholder jump with operand 0, save the
   returned byte-pos, compile the branch, then call
   `patch_jump_to_here` with that byte-pos. the op's operand is
   rewritten to the byte-delta from the START of the jump op to the
   current next_pc (the position the op after the jump WILL occupy).

   convention matches V4 spec §3.4: `pc += offset` where pc is at
   the START of the jump op when offset is read — so a forward jump
   has positive offset = (target_byte - jump_byte). this matches the
   rust seed's patching (op_list_target_index - op_list_jump_index in
   op-list space; here it's the byte-position equivalent).
   ───────────────────────────────────────────────────────────────── *)

(** find the op-list index of the op emitted at byte-position `bytepos`.
    walks the (small) op_bytepos array linearly — O(n) but acceptable
    for the seed compiler's chunk sizes. *)
let op_index_at (b : chunk_builder) (bytepos : int) : int =
  let n = Dynarray.length b.op_bytepos in
  let rec find i =
    if i >= n then err (Printf.sprintf "no op at bytepos %d" bytepos)
    else if Dynarray.get b.op_bytepos i = bytepos then i
    else find (i + 1)
  in
  find 0

(** patch a previously-emitted jump to land at the current next_pc.
    `jump_bytepos` is the value returned by `emit` when the jump was
    emitted.

    V4 spec §3.4: jump offsets are measured from the byte AFTER the
    jump's operand bytes (i.e. from `jump_bytepos + 3`, since
    Jump/JumpIfFalse/JumpIfTrue are 3 bytes). To land at
    `target_bytepos`, encode `off = target - jump_bytepos - 3` so that
    runtime's `pc += offset` (with pc already at jump_bytepos + 3)
    arrives at target. matches players/rust/src/v4_export.rs's
    compute_byte_offset. *)
let patch_jump_to_here (b : chunk_builder) (jump_bytepos : int) : unit =
  let idx = op_index_at b jump_bytepos in
  let target_bytepos = b.next_pc in
  let off = target_bytepos - jump_bytepos - 3 in
  if off < -32768 || off > 32767 then
    err (Printf.sprintf "jump offset out of i16 range: %d" off);
  let new_op =
    match Dynarray.get b.ops idx with
    | Jump _ -> Jump off
    | JumpIfFalse _ -> JumpIfFalse off
    | JumpIfTrue _ -> JumpIfTrue off
    | _ ->
        err (Printf.sprintf "patch_jump_to_here: not a jump op (idx=%d)" idx)
  in
  Dynarray.set b.ops idx new_op

(* ─────────────────────────────────────────────────────────────────
   helpers — list/cons accessors used heavily by compile_form.
   ───────────────────────────────────────────────────────────────── *)

(** is this form a cons-list with `head` as its symbol head? *)
let head_is_sym (f : form) (head : string) : bool =
  match f with
  | Cons (Sym s, _) -> s = head
  | _ -> false

(** the cdr of a form, or Nil if the form is Nil. *)
let safe_cdr (f : form) : form =
  match f with
  | Nil -> Nil
  | Cons (_, t) -> t
  | _ -> err "safe_cdr: not a cons"

(** extract the symbol from a form, or err. *)
let sym_of (f : form) : string =
  match f with
  | Sym s -> s
  | _ -> err "expected a symbol"

(* ─────────────────────────────────────────────────────────────────
   the main entrypoint — compile_form.

   `tail`: whether this form is in tail position. propagates through
   `if`, `do`, `let` to the last expression; resets to false inside
   args. enables TailSend / TailSendSelf / TailSendHere.
   ───────────────────────────────────────────────────────────────── *)

let rec compile_form (b : chunk_builder) (f : form) ~(tail : bool) : unit =
  match f with
  | Nil -> ignore (emit b PushNil)
  | Bool true -> ignore (emit b PushTrue)
  | Bool false -> ignore (emit b PushFalse)
  | Int _ | Float _ | Char _ | Str _ | Bytes _ ->
      ignore (emit b (LoadConst (add_const b f)))
  | Sym "self" -> ignore (emit b LoadSelf)
  | Sym "$here" ->
      (* V4 fusion: `$here` as a value bypasses the env walk. spec §5.3. *)
      ignore (emit b LoadHere)
  | Sym s -> ignore (emit b (LoadName (intern s)))
  | Cons _ as c -> compile_list b c ~tail
  | Vec _ ->
      (* table-literal staging — moof Compiler handles this. seed
         shouldn't see it in compiler.moof; if it does, treat as
         a quoted constant. *)
      ignore (emit b (LoadConst (add_const b f)))
  | FormRef _ ->
      (* FormRef is a transient produced by the image builder's
         non-scalar const lifter; the compiler should never see it
         in source. defensively, treat it as a constant. *)
      ignore (emit b (LoadConst (add_const b f)))

(* ─────────────────────────────────────────────────────────────────
   compile_list — dispatch on the head of a cons-list.

   the seven special forms exactly: def, set!, if, fn, do, let, quote.
   plus __send__ (parser-desugared send) and a catch-all "treat as
   call" branch. matches the minimal subset (self-host design §4).

   note: the seed does NOT consult any user-macro table. compiler.moof
   uses zero macros; bootstrap.moof (which has macros) loads through
   the moof Compiler post-flip.
   ───────────────────────────────────────────────────────────────── *)

and compile_list (b : chunk_builder) (f : form) ~(tail : bool) : unit =
  let elems = list_to_forms f in
  match elems with
  | [] -> ignore (emit b PushNil)
  | (Sym "__send__") :: rest -> compile_send b rest ~tail
  | (Sym "quote") :: rest -> compile_quote b rest
  | (Sym "set!") :: rest -> compile_set b rest
  | (Sym "def") :: rest -> compile_def b rest
  | (Sym "if") :: rest -> compile_if b rest ~tail
  | (Sym "fn") :: rest -> compile_fn b rest
  | (Sym "do") :: rest -> compile_do b rest ~tail
  | (Sym "let") :: rest -> compile_let b rest ~tail
  | _ -> compile_call b elems ~tail

(* ─────────────────────────────────────────────────────────────────
   compile_send — (__send__ receiver 'selector args…)

   the most important compile-* function. V4 fusion lives here:
   - receiver `self`   → SendSelf / TailSendSelf  (spec §3.3, §5.1)
   - receiver `$here`  → SendHere / TailSendHere
   - receiver `super`  → SuperSend (uses self_ implicitly, §3.3)
   - selector :perform:withArgs: → SendDynamic    (spec §5.4)
   - otherwise         → Send / TailSend

   the V3 if-shape peephole + const-fold peephole are deferred to the
   moof Compiler — the seed emits straightforward shapes.
   ───────────────────────────────────────────────────────────────── *)

and compile_send (b : chunk_builder) (elems : form list) ~(tail : bool) : unit =
  match elems with
  | receiver :: selector_form :: args ->
      let selector_str = sym_of selector_form in
      let selector = intern selector_str in
      let argc = List.length args in
      if argc > 255 then err "send: too many args (max 255)";

      (* SendDynamic — :perform:withArgs: short-circuit. spec §5.4. *)
      if selector_str = "perform:withArgs:" then begin
        compile_form b receiver ~tail:false;
        List.iter (fun a -> compile_form b a ~tail:false) args;
        let ic = next_ic b in
        ignore (emit b (SendDynamic { argc; ic_idx = ic }))
      end
      (* SuperSend — super receiver is implicit (uses self_). spec §3.3. *)
      else if (match receiver with Sym "super" -> true | _ -> false) then begin
        List.iter (fun a -> compile_form b a ~tail:false) args;
        let ic = next_ic b in
        ignore (emit b (SuperSend { selector; argc; ic_idx = ic }))
      end
      (* SendSelf / TailSendSelf — V4 fusion for self receiver. spec §3.3. *)
      else if (match receiver with Sym "self" -> true | _ -> false) then begin
        List.iter (fun a -> compile_form b a ~tail:false) args;
        if tail then
          ignore (emit b (TailSendSelf { selector; argc }))
        else
          let ic = next_ic b in
          ignore (emit b (SendSelf { selector; argc; ic_idx = ic }))
      end
      (* SendHere / TailSendHere — V4 fusion for $here receiver. spec §3.3. *)
      else if (match receiver with Sym "$here" -> true | _ -> false) then begin
        List.iter (fun a -> compile_form b a ~tail:false) args;
        if tail then
          ignore (emit b (TailSendHere { selector; argc }))
        else
          let ic = next_ic b in
          ignore (emit b (SendHere { selector; argc; ic_idx = ic }))
      end
      (* normal Send / TailSend. *)
      else begin
        compile_form b receiver ~tail:false;
        List.iter (fun a -> compile_form b a ~tail:false) args;
        if tail then
          ignore (emit b (TailSend { selector; argc }))
        else
          let ic = next_ic b in
          ignore (emit b (Send { selector; argc; ic_idx = ic }))
      end
  | _ -> err "malformed __send__ form (expected receiver selector args…)"

(* ─────────────────────────────────────────────────────────────────
   compile_quote — (quote v) → LoadConst of the verbatim form.
   ───────────────────────────────────────────────────────────────── *)

and compile_quote (b : chunk_builder) (rest : form list) : unit =
  match rest with
  | [ v ] -> ignore (emit b (LoadConst (add_const b v)))
  | _ -> err "quote requires 1 arg: (quote v)"

(* ─────────────────────────────────────────────────────────────────
   compile_set — (set! name expr)

   per lib/compiler/02-special.moof + V3 task semantics — desugars to
   [[Env current] set: 'name to: expr]. emission:

     LoadName Env
     Send :current argc=0
     LoadConst 'name
     <compile expr>
     Send :set:to: argc=2

   note: not a V4 fused variant — the receiver is `Env`, a runtime
   value, not `self`/`$here`. user can override Env :current to
   redirect set! to a custom env.
   ───────────────────────────────────────────────────────────────── *)

and compile_set (b : chunk_builder) (rest : form list) : unit =
  match rest with
  | [ name_form; expr ] ->
      let name = sym_of name_form in
      ignore (emit b (LoadName (intern "Env")));
      let ic_cur = next_ic b in
      ignore (emit b (Send { selector = intern "current"; argc = 0; ic_idx = ic_cur }));
      ignore (emit b (LoadConst (add_const b (Sym name))));
      compile_form b expr ~tail:false;
      let ic_set = next_ic b in
      ignore (emit b (Send { selector = intern "set:to:"; argc = 2; ic_idx = ic_set }))
  | _ -> err "set! requires 2 args: (set! name expr)"

(* ─────────────────────────────────────────────────────────────────
   compile_def — (def name expr)

   V3+V4: emits Send-based bytecode equivalent to
     (do [$here bind: 'name to: expr] 'name)

   V4 fusion: receiver `$here` → SendHere {sel='bind:to:, argc=2}.
   no LoadHere needed — SendHere carries the receiver implicitly
   (spec §3.3, §5.3). emission:

     LoadConst 'name
     <compile expr>
     SendHere :bind:to: argc=2
     Pop
     LoadConst 'name

   the single-binding-only shape — multi-clause defs are a moof-side
   concern (defn macro). compiler.moof uses only single-binding.
   ───────────────────────────────────────────────────────────────── *)

and compile_def (b : chunk_builder) (rest : form list) : unit =
  match rest with
  | [ name_form; expr ] ->
      let name = sym_of name_form in
      let name_sym_val = Sym name in
      ignore (emit b (LoadConst (add_const b name_sym_val)));
      compile_form b expr ~tail:false;
      let ic = next_ic b in
      ignore (emit b (SendHere { selector = intern "bind:to:"; argc = 2; ic_idx = ic }));
      ignore (emit b Pop);
      ignore (emit b (LoadConst (add_const b name_sym_val)))
  | _ -> err "seed compiler: def requires 2 args (multi-clause is moof-only)"

(* ─────────────────────────────────────────────────────────────────
   compile_if — (if cond then [else])

   the seed emits Jump-based bytecode directly (skipping the
   :ifTrue:ifFalse: Send-shape that the moof Compiler emits). this
   is faster + simpler for the seed; the moof Compiler (loaded
   later) emits the full Send-based shape for user code that may
   override `:ifTrue:ifFalse:`.

   emission:
     <compile cond>
     Send :!! argc=0          ; coerce to Bool (user-overridable !!)
     JumpIfFalse else_target
     <compile then, tail-iff-outer-tail>
     Jump end_target
   else_target:
     <compile else, tail-iff-outer-tail>
   end_target:

   we still emit the `:!!` send (matches rust seed's
   compile_if_inline) — preserves user-level truthiness overrides
   at the cond layer. JumpIfFalse itself uses the substrate's
   built-in is_truthy (Nil/false → false; everything else → true).
   ───────────────────────────────────────────────────────────────── *)

and compile_if (b : chunk_builder) (rest : form list) ~(tail : bool) : unit =
  let (cond, then_branch, else_branch) =
    match rest with
    | [ c; t ] -> (c, t, Nil)
    | [ c; t; e ] -> (c, t, e)
    | _ -> err "if takes 2 or 3 args: (if cond then [else])"
  in
  (* compile cond, non-tail. *)
  compile_form b cond ~tail:false;
  (* Send :!! argc=0 — coerce to Bool, preserves user override of :!!. *)
  let ic_bang = next_ic b in
  ignore (emit b (Send { selector = intern "!!"; argc = 0; ic_idx = ic_bang }));
  (* JumpIfFalse else (placeholder; patched after then compiles). *)
  let jif = emit b (JumpIfFalse 0) in
  (* compile then-branch; tail iff `if` was tail. *)
  compile_form b then_branch ~tail;
  (* Jump end (placeholder; patched after else compiles). *)
  let jmp = emit b (Jump 0) in
  (* patch JumpIfFalse to land here (start of else). *)
  patch_jump_to_here b jif;
  (* compile else-branch; tail iff `if` was tail. *)
  compile_form b else_branch ~tail;
  (* patch unconditional Jump to land here (after else). *)
  patch_jump_to_here b jmp

(* ─────────────────────────────────────────────────────────────────
   compile_fn — (fn (params…) body…)

   compiles the body into a fresh sub-chunk (registered in the
   global chunk_registry, which assigns its FormId). emits a
   PushClosure referencing the sub-chunk.

   multi-expression bodies are wrapped in (do …) per the rust seed.
   ───────────────────────────────────────────────────────────────── *)

and compile_fn (b : chunk_builder) (rest : form list) : unit =
  match rest with
  | params_form :: body_forms when body_forms <> [] ->
      let params_list = list_to_forms params_form in
      let params = List.map (fun p -> intern (sym_of p)) params_list in
      let body =
        match body_forms with
        | [ single ] -> single
        | many -> Cons (Sym "do", forms_to_list many)
      in
      let inner_b = make ~params body in
      compile_form inner_b body ~tail:true;
      ignore (emit inner_b Return);
      ignore (emit b (PushClosure inner_b.id))
  | _ -> err "fn requires params list and body"

(* ─────────────────────────────────────────────────────────────────
   compile_let — (let ((n v) …) body…)

   desugars to ((fn (n …) body) v …). bindings evaluate in PARALLEL
   in the current env, then a single new env binds them all before
   body runs. matches the rust seed.

   note: bootstrap.moof installs `let` as a macro that does the same
   expansion at source level; this handler is the fallback for when
   the macro isn't yet registered (i.e., during compiler.moof's own
   load).
   ───────────────────────────────────────────────────────────────── *)

and compile_let (b : chunk_builder) (rest : form list) ~(tail : bool) : unit =
  match rest with
  | bindings_form :: body_forms when body_forms <> [] ->
      let bindings = list_to_forms bindings_form in
      let (params, values) =
        List.fold_right
          (fun binding (ps, vs) ->
            match list_to_forms binding with
            | [ Sym n; v ] -> (intern n :: ps, v :: vs)
            | _ -> err "let: each binding is (name value)")
          bindings ([], [])
      in
      let body =
        match body_forms with
        | [ single ] -> single
        | many -> Cons (Sym "do", forms_to_list many)
      in
      (* compile sub-chunk for the body. *)
      let inner_b = make ~params body in
      compile_form inner_b body ~tail:true;
      ignore (emit inner_b Return);
      (* outer: PushClosure; <eval each value>; Send :call argc=N. *)
      ignore (emit b (PushClosure inner_b.id));
      List.iter (fun v -> compile_form b v ~tail:false) values;
      let argc = List.length values in
      if argc > 255 then err "let: too many bindings (max 255)";
      let call_sel = intern "call" in
      if tail then
        ignore (emit b (TailSend { selector = call_sel; argc }))
      else
        let ic = next_ic b in
        ignore (emit b (Send { selector = call_sel; argc; ic_idx = ic }))
  | _ -> err "let requires bindings + body"

(* ─────────────────────────────────────────────────────────────────
   compile_do — (do e1 … eN)

   sequence: each form except the last is non-tail + followed by Pop;
   the last carries the form's value, tail iff outer tail. empty
   body is PushNil (matches rust seed + moof Compiler).
   ───────────────────────────────────────────────────────────────── *)

and compile_do (b : chunk_builder) (rest : form list) ~(tail : bool) : unit =
  match rest with
  | [] -> ignore (emit b PushNil)
  | body ->
      let n = List.length body in
      List.iteri
        (fun i expr ->
          let last = i = n - 1 in
          compile_form b expr ~tail:(tail && last);
          if not last then ignore (emit b Pop))
        body

(* ─────────────────────────────────────────────────────────────────
   compile_call — (callable arg…)

   the fallback when no special form matches. lowers to
   [callable call: arg…] — Send :call to the callable. matches rust
   seed's compile_call.

   the callable is the head of the form, ARGS are the tail. evaluation
   order: callable first (non-tail), then each arg (non-tail), then
   Send/TailSend :call.
   ───────────────────────────────────────────────────────────────── *)

and compile_call (b : chunk_builder) (elems : form list) ~(tail : bool) : unit =
  match elems with
  | callable :: args ->
      compile_form b callable ~tail:false;
      List.iter (fun a -> compile_form b a ~tail:false) args;
      let argc = List.length args in
      if argc > 255 then err "call: too many args (max 255)";
      let call_sel = intern "call" in
      if tail then
        ignore (emit b (TailSend { selector = call_sel; argc }))
      else
        let ic = next_ic b in
        ignore (emit b (Send { selector = call_sel; argc; ic_idx = ic }))
  | [] -> err "compile_call: empty list"

(* ─────────────────────────────────────────────────────────────────
   top-level entry — compile_top.

   allocates a fresh chunk, compiles the form in tail position,
   emits Return, and returns the chunk_builder. caller can then
   `finalize` to get the byte-encoded body or hand the builder to
   the image serializer.
   ───────────────────────────────────────────────────────────────── *)

let compile_top (form : form) : chunk_builder =
  let b = make form in
  compile_form b form ~tail:true;
  ignore (emit b Return);
  b

(* ─────────────────────────────────────────────────────────────────
   finalize — produce the byte-encoded chunk body.

   walks the op list and encodes each op per V4 spec §4 (big-endian
   fixed-width operands). delegates per-op encoding to
   Bytecode.encode_op (from OCAML-2).

   the returned record matches the V4 chunk shape (spec §10.3
   ChunkSection): body bytes, const pool, IC count, and (implicitly)
   the params already stored in the builder.
   ───────────────────────────────────────────────────────────────── *)

type finalized = {
  body : bytes;
  consts : form list;
  ic_count : int;
  params : int list;
  source : form;
  chunk_id : int;
}

let finalize (b : chunk_builder) : finalized =
  let buf = Buffer.create (b.next_pc) in
  Dynarray.iter (fun op -> Bytecode.encode_op op buf) b.ops;
  let body = Buffer.to_bytes buf in
  if Bytes.length body <> b.next_pc then
    err (Printf.sprintf
      "finalize: encoded body length %d != next_pc %d (size_of_op disagrees with encode_op)"
      (Bytes.length body) b.next_pc);
  {
    body;
    consts = Dynarray.to_list b.consts;
    ic_count = b.ic_count;
    params = b.params;
    source = b.source;
    chunk_id = b.id;
  }
