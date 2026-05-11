(* moof source reader. produces Ast.form values.

   Handles a small subset of moof syntax — enough for the seed CLI to
   parse `[1 + 2]`, `(do (def x 42) x)`, etc. The OCAML-2 agent will
   overwrite with the full reader.

   Supported tokens:
     - integers, floats
     - #true / #false / nil
     - symbols (any non-bracket/whitespace run starting with non-digit)
     - strings "..."
     - lists ( ... )
     - sends [ ... ] — read as (__send__ receiver sel args...)
       where `[recv sel arg1 arg2]` becomes `(__send__ recv 'sel arg1 arg2)`
       for unary, `[recv :sel: a b]` → `(__send__ recv 'sel: a b)`
   This is a simplification, but matches enough of moof for the smoke. *)

open Ast

exception Read_error of string

(* tokenizer state. *)
type tokenizer = {
  src : string;
  mutable pos : int;
}

let mk_tok s = { src = s; pos = 0 }

let peek t =
  if t.pos >= String.length t.src then None
  else Some t.src.[t.pos]

let advance t =
  if t.pos < String.length t.src then t.pos <- t.pos + 1

let skip_ws t =
  let rec loop () =
    match peek t with
    | Some c when c = ' ' || c = '\t' || c = '\n' || c = '\r' ->
        advance t; loop ()
    | Some ';' ->
        (* line comment to EOL *)
        while (match peek t with Some c -> c <> '\n' | None -> false) do
          advance t
        done;
        loop ()
    | _ -> ()
  in
  loop ()

let is_sym_char c =
  match c with
  | ' ' | '\t' | '\n' | '\r' | '(' | ')' | '[' | ']' | '"' | ';' -> false
  | _ -> true

let read_token t : string =
  let start = t.pos in
  let rec loop () =
    match peek t with
    | Some c when is_sym_char c -> advance t; loop ()
    | _ -> ()
  in
  loop ();
  String.sub t.src start (t.pos - start)

let read_string t : form =
  (* skip opening quote *)
  advance t;
  let buf = Buffer.create 16 in
  let rec loop () =
    match peek t with
    | None -> raise (Read_error "unterminated string")
    | Some '"' -> advance t
    | Some '\\' ->
        advance t;
        (match peek t with
         | None -> raise (Read_error "unterminated escape")
         | Some 'n' -> Buffer.add_char buf '\n'; advance t
         | Some 't' -> Buffer.add_char buf '\t'; advance t
         | Some '\\' -> Buffer.add_char buf '\\'; advance t
         | Some '"' -> Buffer.add_char buf '"'; advance t
         | Some c -> Buffer.add_char buf c; advance t);
        loop ()
    | Some c -> Buffer.add_char buf c; advance t; loop ()
  in
  loop ();
  Str (Buffer.contents buf)

let try_parse_number (s : string) : form option =
  if s = "" then None
  else
    let c0 = s.[0] in
    if not (c0 = '-' || c0 = '+' || (c0 >= '0' && c0 <= '9')) then None
    else
      try Some (Int (int_of_string s))
      with _ ->
        try Some (Float (float_of_string s))
        with _ -> None

let classify_atom (tok : string) : form =
  if tok = "nil" then Nil
  else if tok = "#true" then Bool true
  else if tok = "#false" then Bool false
  else
    match try_parse_number tok with
    | Some f -> f
    | None -> Sym tok

let rec read_form t : form =
  skip_ws t;
  match peek t with
  | None -> raise (Read_error "unexpected EOF")
  | Some '(' ->
      advance t;
      read_list t ')'
  | Some '[' ->
      advance t;
      read_send t
  | Some '"' -> read_string t
  | Some ')' | Some ']' ->
      raise (Read_error (Printf.sprintf "unexpected '%c'" (Option.get (peek t))))
  | Some '\'' ->
      (* quote sugar: 'x → (quote x) *)
      advance t;
      let inner = read_form t in
      forms_to_list [Sym "quote"; inner]
  | Some _ ->
      let tok = read_token t in
      classify_atom tok

and read_list t closer : form =
  let rec loop acc =
    skip_ws t;
    match peek t with
    | None -> raise (Read_error "unterminated list")
    | Some c when c = closer -> advance t; List.rev acc
    | _ ->
        let f = read_form t in
        loop (f :: acc)
  in
  let elems = loop [] in
  forms_to_list elems

and read_send t : form =
  (* [recv sel args...] → (__send__ recv 'sel args...) *)
  let elems =
    let rec loop acc =
      skip_ws t;
      match peek t with
      | None -> raise (Read_error "unterminated send")
      | Some ']' -> advance t; List.rev acc
      | _ ->
          let f = read_form t in
          loop (f :: acc)
    in
    loop []
  in
  match elems with
  | [] -> raise (Read_error "empty send")
  | recv :: rest ->
      (* The selector is the *next* sym, unless it's an operator-shaped
         message [a + b]. For seed-CLI scope we just take the second elem
         as the selector when it's a Sym, otherwise emit unary. *)
      match rest with
      | [] ->
          (* unary message — but we have only the receiver. shouldn't
             happen; treat as (__send__ recv). *)
          forms_to_list [Sym "__send__"; recv]
      | (Sym sel) :: args ->
          forms_to_list (Sym "__send__" :: recv :: forms_to_list [Sym "quote"; Sym sel] :: args)
      | _other ->
          (* receiver-only-then-non-sym — degenerate; emit raw. *)
          forms_to_list (Sym "__send__" :: recv :: rest)

(* read all top-level forms in a string. *)
let read_all (src : string) : form list =
  let t = mk_tok src in
  let rec loop acc =
    skip_ws t;
    if t.pos >= String.length t.src then List.rev acc
    else
      let f = read_form t in
      loop (f :: acc)
  in
  loop []

let read_file (path : string) : form list =
  let ic = open_in path in
  let n = in_channel_length ic in
  let buf = Bytes.create n in
  really_input ic buf 0 n;
  close_in ic;
  read_all (Bytes.to_string buf)
