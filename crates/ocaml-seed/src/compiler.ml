(* moof V4 compiler — produces byte-encoded chunks per spec §3-§5.

   STATUS: stub for parallel agent dev. OCAML-3 owns the canonical version.

   This stub handles enough of the language for the seed CLI to demonstrate
   end-to-end: integer literals, `[a + b]`-style sends, and the const-fold
   peephole on `[N + M]`. Anything beyond that falls into `compile_call`
   which emits a generic Send. *)

open Ast
open Opcodes

(* a chunk-builder. accumulates ops + consts + sym refs + ic count.
   `next_ic`: the next IC slot to assign. monotonically increasing.
   `next_sym`: monotonically-assigned SymId for `intern`. *)
type chunk_builder = {
  mutable ops : op list;             (* in reverse — emitted backward *)
  mutable consts : Ast.form list;     (* in reverse *)
  mutable num_consts : int;
  mutable ics : int;
  mutable params : int list;
  mutable source : Ast.form;
  (* symbol-interning state, threaded through compilation: *)
  mutable sym_table : (string, int) Hashtbl.t;
  mutable sym_order : string list;     (* in interning order, reverse *)
}

let new_chunk_builder source = {
  ops = [];
  consts = [];
  num_consts = 0;
  ics = 0;
  params = [];
  source;
  sym_table = Hashtbl.create 32;
  sym_order = [];
}

let intern_sym (cb : chunk_builder) (name : string) : int =
  match Hashtbl.find_opt cb.sym_table name with
  | Some i -> i
  | None ->
      let i = Hashtbl.length cb.sym_table in
      Hashtbl.add cb.sym_table name i;
      cb.sym_order <- name :: cb.sym_order;
      i

let emit (cb : chunk_builder) (op : op) : unit =
  cb.ops <- op :: cb.ops

let add_const (cb : chunk_builder) (v : Ast.form) : int =
  (* deduplicate by structural equality (per spec §9 determinism). *)
  let rec find i = function
    | [] -> -1
    | x :: _ when x = v -> i
    | _ :: rest -> find (i - 1) rest
  in
  let n = cb.num_consts in
  match find (n - 1) cb.consts with
  | -1 ->
      cb.consts <- v :: cb.consts;
      cb.num_consts <- n + 1;
      n
  | i -> i

let alloc_ic (cb : chunk_builder) : int =
  let i = cb.ics in
  cb.ics <- i + 1;
  i

(* Compile a single form into the current chunk builder.
   Returns unit; ops accumulated on cb. *)
let rec compile_form (cb : chunk_builder) (f : Ast.form) : unit =
  match f with
  | Nil -> emit cb PushNil
  | Bool true -> emit cb PushTrue
  | Bool false -> emit cb PushFalse
  | Int _ | Float _ | Char _ | Str _ | Bytes _ ->
      let idx = add_const cb f in
      emit cb (LoadConst idx)
  | Sym "self" -> emit cb LoadSelf
  | Sym "$here" -> emit cb LoadHere
  | Sym s ->
      let sid = intern_sym cb s in
      emit cb (LoadName sid)
  | Vec _ ->
      (* not implemented at seed level; just emit nil. *)
      emit cb PushNil
  | Cons (Sym "__send__", rest) ->
      compile_send cb rest
  | Cons (Sym "quote", Cons (q, Nil)) ->
      (* compile-time constant. *)
      let idx = add_const cb q in
      emit cb (LoadConst idx)
  | Cons (Sym "do", body) ->
      compile_do cb body
  | Cons (head, args) ->
      (* fallback: treat as a call — emit head, args, then a Send with sel='__call__. *)
      compile_form cb head;
      let arg_list = list_to_forms args in
      List.iter (compile_form cb) arg_list;
      let sid = intern_sym cb "__call__" in
      let argc = List.length arg_list in
      let ic = alloc_ic cb in
      emit cb (Send (sid, argc, ic))

and compile_send (cb : chunk_builder) (args_form : Ast.form) : unit =
  (* args_form is (receiver 'sel arg1 arg2 ...) *)
  let items = list_to_forms args_form in
  match items with
  | [] -> failwith "compile_send: empty"
  | [_recv] -> failwith "compile_send: missing selector"
  | recv :: sel_form :: rest ->
      let sel = match sel_form with
        | Cons (Sym "quote", Cons (Sym s, Nil)) -> s
        | Sym s -> s
        | _ -> failwith "compile_send: selector not a sym"
      in
      (* peephole: const-fold [N + M] where N,M are Int literals. *)
      (match recv, sel, rest with
       | Int a, "+", [Int b] ->
           let idx = add_const cb (Int (a + b)) in
           emit cb (LoadConst idx)
       | Int a, "-", [Int b] ->
           let idx = add_const cb (Int (a - b)) in
           emit cb (LoadConst idx)
       | Int a, "*", [Int b] ->
           let idx = add_const cb (Int (a * b)) in
           emit cb (LoadConst idx)
       | _ ->
           let sid = intern_sym cb sel in
           let argc = List.length rest in
           let ic = alloc_ic cb in
           (* V4 emission rules §5.1: fused-send variants. *)
           (match recv with
            | Sym "self" ->
                List.iter (compile_form cb) rest;
                emit cb (SendSelf (sid, argc, ic))
            | Sym "$here" ->
                List.iter (compile_form cb) rest;
                emit cb (SendHere (sid, argc, ic))
            | _ ->
                compile_form cb recv;
                List.iter (compile_form cb) rest;
                emit cb (Send (sid, argc, ic))))

and compile_do (cb : chunk_builder) (body : Ast.form) : unit =
  let items = list_to_forms body in
  match items with
  | [] -> emit cb PushNil
  | [x] -> compile_form cb x
  | _ ->
      let rec loop = function
        | [] -> ()
        | [x] -> compile_form cb x
        | x :: rest ->
            compile_form cb x;
            emit cb Pop;
            loop rest
      in
      loop items

(* compile a top-level form — produces a fresh chunk builder
   ending with an explicit Return. *)
let compile_top (form : Ast.form) : chunk_builder =
  let cb = new_chunk_builder form in
  compile_form cb form;
  emit cb Return;
  cb

(* finalize chunk_builder → concrete chunk-bytes for serialization. *)
type final_chunk = {
  source : Ast.form;
  body : bytes;
  consts : Ast.form list;
  ic_count : int;
  params : int list;
  sym_order : string list;  (* in interning order — for sym table assembly *)
}

let finalize (cb : chunk_builder) : final_chunk =
  let ops = List.rev cb.ops in
  let body = Bytecode.encode_ops ops in
  {
    source = cb.source;
    body;
    consts = List.rev cb.consts;
    ic_count = cb.ics;
    params = cb.params;
    sym_order = List.rev cb.sym_order;
  }
