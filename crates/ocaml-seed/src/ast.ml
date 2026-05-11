(* moof source Form. mirrors what the reader produces and what the
   compiler consumes. matches the structure of crates/substrate/src/reader.rs's
   Value enum at the read layer.

   note on integer width: moof's source-level Integer is i48 ∪ BigInt
   on the runtime side, but at the read layer we model it as an
   OCaml int (guaranteed >= 63 bits on a 64-bit host). values that
   would overflow are read as Float for now; the compiler agent can
   refine later if BigInt literals show up in source. *)

type form =
  | Nil
  | Bool of bool
  | Int of int
  | Float of float
  | Char of int  (* codepoint *)
  | Sym of string  (* interned at compile-time; here just a string *)
  | Str of string
  | Bytes of bytes
  | Cons of form * form
  | Vec of form list  (* reserved for #[...] table-literal staging if needed *)

(* construct a proper cons-list from an OCaml list. terminator is Nil. *)
let rec forms_to_list (xs : form list) : form =
  match xs with
  | [] -> Nil
  | x :: rest -> Cons (x, forms_to_list rest)

(* walk a cons-list into an OCaml list. an improper tail (anything
   not Nil) is appended as the final element verbatim. *)
let list_to_forms (f : form) : form list =
  let rec go acc cur =
    match cur with
    | Nil -> List.rev acc
    | Cons (h, t) -> go (h :: acc) t
    | other -> List.rev (other :: acc)
  in
  go [] f

let car (f : form) : form =
  match f with
  | Cons (h, _) -> h
  | _ -> invalid_arg "Ast.car: not a Cons"

let cdr (f : form) : form =
  match f with
  | Cons (_, t) -> t
  | _ -> invalid_arg "Ast.cdr: not a Cons"

let is_nil (f : form) : bool = match f with Nil -> true | _ -> false
let is_cons (f : form) : bool = match f with Cons _ -> true | _ -> false
let is_sym (f : form) : bool = match f with Sym _ -> true | _ -> false

(* simple printer — for debugging/tests. doesn't try to be smart
   about quoted forms; just dumps the s-expression structure. *)
let rec to_string (f : form) : string =
  match f with
  | Nil -> "nil"
  | Bool true -> "#true"
  | Bool false -> "#false"
  | Int n -> string_of_int n
  | Float x -> Printf.sprintf "%g" x
  | Char cp -> Printf.sprintf "#\\u{%x}" cp
  | Sym s -> s
  | Str s -> Printf.sprintf "%S" s
  | Bytes b -> Printf.sprintf "#bytes(%d)" (Bytes.length b)
  | Vec xs -> "#[" ^ String.concat " " (List.map to_string xs) ^ "]"
  | Cons _ as c ->
      (* render as list if proper; else dotted pair. *)
      let rec collect acc = function
        | Cons (h, t) -> collect (h :: acc) t
        | Nil -> (List.rev acc, None)
        | other -> (List.rev acc, Some other)
      in
      let xs, tail = collect [] c in
      let body = String.concat " " (List.map to_string xs) in
      match tail with
      | None -> "(" ^ body ^ ")"
      | Some t -> "(" ^ body ^ " . " ^ to_string t ^ ")"
