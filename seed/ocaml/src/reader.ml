(* moof seed reader - minimal subset.
   spec: docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md.

   produces Ast.form values from source text. handles ONLY the
   minimal-bootstrap subset:

   - atoms: Int, Char, Sym, String, #true / #false, nil
   - (...) cons lists
   - 'foo quote sugar
   - [recv msg: arg ...] send (single-clause; no cascade); desugars to
     (__send__ recv 'sel arg ...)
   - #\char char literals + named chars
   - #true / #false hash bool literals
   - string escapes (backslash-n, backslash-t, backslash-backslash,
     backslash-doublequote, backslash-r, backslash-0, backslash-xHH)

   BANNED (handled by parser.moof at runtime, not by the seed):
   - quasiquote, unquote, unquote-splice
   - vector / table literals #[...]
   - object literals {...}
   - block syntax |args| body
   - send-cascade ;
   - self-send shorthand .foo
   - => arrow inside table literals *)

exception ReadError of string

(* cursor *)

type cursor = {
  mutable pos : int;
  mutable line : int;
  mutable col : int;
  bytes : bytes;
  len : int;
}

let make_cursor (s : string) : cursor =
  { pos = 0; line = 1; col = 1; bytes = Bytes.of_string s; len = String.length s }

let cur_peek (c : cursor) : char option =
  if c.pos >= c.len then None else Some (Bytes.get c.bytes c.pos)

let cur_peek_at (c : cursor) (off : int) : char option =
  let p = c.pos + off in
  if p >= c.len then None else Some (Bytes.get c.bytes p)

let cur_advance (c : cursor) : char option =
  match cur_peek c with
  | None -> None
  | Some b ->
      c.pos <- c.pos + 1;
      if b = '\n' then (c.line <- c.line + 1; c.col <- 1)
      else c.col <- c.col + 1;
      Some b

let cur_at_end (c : cursor) : bool = c.pos >= c.len

(* character classes *)

let is_ascii_whitespace (b : char) : bool =
  match b with ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' -> true | _ -> false

let is_delim (b : char) : bool =
  match b with
  | '(' | ')' | '[' | ']' | '\'' | '"' | ';' -> true
  | c -> is_ascii_whitespace c

let is_binary_op_char (b : char) : bool =
  match b with
  | '+' | '-' | '*' | '/' | '<' | '>' | '=' | '!' | '?' | '|' | '&' | '~'
  | '^' | '%' -> true
  | _ -> false

let is_binary_op (s : string) : bool =
  String.length s > 0
  && (let ok = ref true in
      String.iter (fun c -> if not (is_binary_op_char c) then ok := false) s;
      !ok)

let is_ascii_digit (b : char) : bool = b >= '0' && b <= '9'
let is_ascii_hexdigit (b : char) : bool =
  is_ascii_digit b || (b >= 'a' && b <= 'f') || (b >= 'A' && b <= 'F')

(* skip whitespace + comments.
   ;; / ;: / ;~ introduce line comments. *)
let rec skip_trivia (c : cursor) : unit =
  match cur_peek c with
  | Some b when is_ascii_whitespace b ->
      let _ = cur_advance c in
      skip_trivia c
  | Some ';' ->
      (match cur_peek_at c 1 with
       | Some (';' | ':' | '~') ->
           let rec eat () =
             match cur_peek c with
             | None -> ()
             | Some '\n' -> ()
             | Some _ -> let _ = cur_advance c in eat ()
           in
           eat ();
           skip_trivia c
       | _ -> ())
  | _ -> ()

(* forward decls (mutual recursion via refs) *)
let read_form_ref : (cursor -> Ast.form) ref =
  ref (fun _ -> failwith "read_form not initialized")

let read_form (c : cursor) : Ast.form = !read_form_ref c

(* numeric parsing *)
let try_parse_number (text : string) : Ast.form option =
  let cleaned =
    let buf = Buffer.create (String.length text) in
    String.iter (fun c -> if c <> '_' then Buffer.add_char buf c) text;
    Buffer.contents buf
  in
  if cleaned = "" then None
  else
    let sign_int, sign_f, rest =
      match cleaned.[0] with
      | '-' -> (-1, -1.0, String.sub cleaned 1 (String.length cleaned - 1))
      | '+' -> (1, 1.0, String.sub cleaned 1 (String.length cleaned - 1))
      | _ -> (1, 1.0, cleaned)
    in
    if rest = "" then None
    else
      let starts_with p s =
        let lp = String.length p and ls = String.length s in
        ls >= lp && String.sub s 0 lp = p
      in
      let strip_prefix p s =
        let lp = String.length p in
        String.sub s lp (String.length s - lp)
      in
      if starts_with "0x" rest || starts_with "0X" rest then
        (try Some (Ast.Int (sign_int * int_of_string ("0x" ^ strip_prefix "0x" (String.lowercase_ascii rest))))
         with _ -> None)
      else if starts_with "0b" rest || starts_with "0B" rest then
        (try Some (Ast.Int (sign_int * int_of_string ("0b" ^ strip_prefix "0b" (String.lowercase_ascii rest))))
         with _ -> None)
      else if starts_with "0o" rest || starts_with "0O" rest then
        (try Some (Ast.Int (sign_int * int_of_string ("0o" ^ strip_prefix "0o" (String.lowercase_ascii rest))))
         with _ -> None)
      else
        let first = rest.[0] in
        if not (is_ascii_digit first) && first <> '.' then None
        else
          let is_float =
            let f = ref false in
            String.iter (fun b ->
              if b = '.' || b = 'e' || b = 'E' then f := true) rest;
            !f
          in
          if is_float then
            (try Some (Ast.Float (sign_f *. float_of_string rest)) with _ -> None)
          else
            (try Some (Ast.Int (sign_int * int_of_string rest)) with _ -> None)

(* bare atom *)
let read_atom (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let buf = Buffer.create 16 in
  let rec eat () =
    match cur_peek c with
    | Some b when is_delim b -> ()
    | Some _ ->
        (match cur_advance c with
         | Some b -> Buffer.add_char buf b
         | None -> ());
        eat ()
    | None -> ()
  in
  eat ();
  let text = Buffer.contents buf in
  if text = "" then
    raise (ReadError (Printf.sprintf "read error at %d:%d: expected atom"
                        start_line start_col));
  if text = "nil" then Ast.Nil
  else
    match try_parse_number text with
    | Some v -> v
    | None -> Ast.Sym text

(* string literal.
   escapes: backslash-n, backslash-t, backslash-backslash,
   backslash-doublequote, backslash-r, backslash-0, backslash-xHH *)
let read_string_lit (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let _ = cur_advance c in
  let buf = Buffer.create 16 in
  let rec eat () =
    match cur_peek c with
    | None ->
        raise (ReadError (Printf.sprintf "read error at %d:%d: unterminated string"
                            start_line start_col))
    | Some '"' -> let _ = cur_advance c in ()
    | Some '\\' ->
        let _ = cur_advance c in
        (match cur_advance c with
         | Some 'n' -> Buffer.add_char buf '\n'; eat ()
         | Some 't' -> Buffer.add_char buf '\t'; eat ()
         | Some '\\' -> Buffer.add_char buf '\\'; eat ()
         | Some '"' -> Buffer.add_char buf '"'; eat ()
         | Some 'r' -> Buffer.add_char buf '\r'; eat ()
         | Some '0' -> Buffer.add_char buf '\x00'; eat ()
         | Some 'x' ->
             let h1 = cur_advance c in
             let h2 = cur_advance c in
             (match h1, h2 with
              | Some a, Some b when is_ascii_hexdigit a && is_ascii_hexdigit b ->
                  let v = int_of_string (Printf.sprintf "0x%c%c" a b) in
                  Buffer.add_char buf (Char.chr (v land 0xff));
                  eat ()
              | _ ->
                  raise (ReadError
                           (Printf.sprintf "read error at %d:%d: malformed hex escape"
                              c.line c.col)))
         | Some other ->
             raise (ReadError
                      (Printf.sprintf "read error at %d:%d: unknown escape: \\%c"
                         c.line c.col other))
         | None ->
             raise (ReadError
                      (Printf.sprintf "read error at %d:%d: unterminated escape"
                         c.line c.col)))
    | Some b ->
        Buffer.add_char buf b;
        let _ = cur_advance c in
        eat ()
  in
  eat ();
  Ast.Str (Buffer.contents buf)

(* char literal - entered with cursor after #\ *)
let read_char_literal (c : cursor) (start_line : int) (start_col : int) : Ast.form =
  let first =
    match cur_advance c with
    | Some b -> b
    | None ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unterminated char literal"
                    start_line start_col))
  in
  let buf = Buffer.create 4 in
  Buffer.add_char buf first;
  let rec eat () =
    match cur_peek c with
    | Some b when is_delim b -> ()
    | Some b -> Buffer.add_char buf b; let _ = cur_advance c in eat ()
    | None -> ()
  in
  eat ();
  let text = Buffer.contents buf in
  if String.length text = 1 then Ast.Char (Char.code first)
  else
    match text with
    | "space" -> Ast.Char (Char.code ' ')
    | "newline" -> Ast.Char (Char.code '\n')
    | "tab" -> Ast.Char (Char.code '\t')
    | "return" -> Ast.Char (Char.code '\r')
    | "null" -> Ast.Char 0
    | "backspace" -> Ast.Char 0x08
    | "delete" -> Ast.Char 0x7f
    | "escape" -> Ast.Char 0x1b
    | _ ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unknown char name"
                    start_line start_col))

(* list (...) *)
let read_list (c : cursor) : Ast.form =
  let _ = cur_advance c in
  let items = ref [] in
  let rec eat () =
    skip_trivia c;
    match cur_peek c with
    | None ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unterminated list"
                    c.line c.col))
    | Some ')' -> let _ = cur_advance c in ()
    | Some _ ->
        items := read_form c :: !items;
        eat ()
  in
  eat ();
  Ast.forms_to_list (List.rev !items)

(* quote sugar *)
let read_quote (c : cursor) : Ast.form =
  let _ = cur_advance c in
  let inner = read_form c in
  Ast.forms_to_list [Ast.Sym "quote"; inner]

(* send bracket - single clause, no cascade.
   shapes:
   - [recv selector]         unary send
   - [recv OP arg]           binary send (OP = operator chars only)
   - [recv selector args...] positional send
   - [recv kw1: a kw2: b]    keyword send, selector = concat of kw:'s
   desugars to (__send__ recv 'sel arg...) *)

let decode_send_segment (elements : Ast.form list)
    (line : int) (col : int) : string * Ast.form list =
  match elements with
  | [] ->
      raise (ReadError
               (Printf.sprintf "read error at %d:%d: send needs at least a selector" line col))
  | first :: rest ->
      let first_text =
        match first with
        | Ast.Sym s -> s
        | _ ->
            raise (ReadError
                     (Printf.sprintf "read error at %d:%d: selector must be a symbol" line col))
      in
      let len = List.length elements in
      if is_binary_op first_text && len = 2 then
        (first_text, [List.nth elements 1])
      else if String.length first_text > 0
              && first_text.[String.length first_text - 1] = ':' then begin
        let sel_buf = Buffer.create 16 in
        let args = ref [] in
        let arr = Array.of_list elements in
        let n = Array.length arr in
        let i = ref 0 in
        while !i < n do
          let kw =
            match arr.(!i) with
            | Ast.Sym s -> s
            | _ ->
                raise (ReadError
                         (Printf.sprintf
                            "read error at %d:%d: keyword send: expected kw: symbol"
                            line col))
          in
          if String.length kw = 0 || kw.[String.length kw - 1] <> ':' then
            raise (ReadError
                     (Printf.sprintf "read error at %d:%d: keyword must end with colon"
                        line col));
          Buffer.add_string sel_buf kw;
          incr i;
          if !i >= n then
            raise (ReadError
                     (Printf.sprintf "read error at %d:%d: keyword needs an argument"
                        line col));
          args := arr.(!i) :: !args;
          incr i
        done;
        (Buffer.contents sel_buf, List.rev !args)
      end else
        (first_text, rest)

let emit_send (recv : Ast.form) (sel : string) (args : Ast.form list) : Ast.form =
  Ast.forms_to_list ([Ast.Sym "__send__"; recv; Ast.Sym sel] @ args)

let read_send_bracket (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let _ = cur_advance c in
  skip_trivia c;
  if cur_peek c = Some ']' then
    raise (ReadError (Printf.sprintf "read error at %d:%d: empty send bracket"
                        start_line start_col));
  let receiver = read_form c in
  (* Accumulate segments separated by `;`. Each segment is its own
     list of elements that decode_send_segment turns into (sel, args).
     - 1 segment: emit (__send__ recv 'sel args...) — single send.
     - 2+ segments: emit (__cascade__ recv (sel1 args1...) (sel2 args2...) ...)
       This matches lib/early/06-control-macros.moof::__cascade__ shape. *)
  let segments = ref [] in
  let current = ref [] in
  let done_ = ref false in
  while not !done_ do
    skip_trivia c;
    match cur_peek c with
    | None ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unterminated send bracket"
                    start_line start_col))
    | Some ']' ->
        let _ = cur_advance c in
        segments := List.rev !current :: !segments;
        done_ := true
    | Some ';' ->
        let _ = cur_advance c in
        segments := List.rev !current :: !segments;
        current := []
    | Some _ ->
        current := read_form c :: !current
  done;
  let seg_lists = List.rev !segments in
  if List.length seg_lists = 0 then
    raise (ReadError
             (Printf.sprintf "read error at %d:%d: send needs at least a selector"
                start_line start_col));
  (match seg_lists with
   | [elem_list] ->
       if elem_list = [] then
         raise (ReadError
                  (Printf.sprintf "read error at %d:%d: send needs at least a selector"
                     start_line start_col));
       let sel, args = decode_send_segment elem_list start_line start_col in
       emit_send receiver sel args
   | _ ->
       (* 2+ segments → cascade. Each segment becomes (sel args...). *)
       let seg_forms = List.map (fun elem_list ->
         if elem_list = [] then
           raise (ReadError
                    (Printf.sprintf
                       "read error at %d:%d: empty cascade segment"
                       start_line start_col));
         let sel, args = decode_send_segment elem_list start_line start_col in
         Ast.forms_to_list (Ast.Sym sel :: args)
       ) seg_lists in
       Ast.forms_to_list (Ast.Sym "__cascade__" :: receiver :: seg_forms))

(* hash forms - minimal subset: #true, #false, #\... *)
let read_hash (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let _ = cur_advance c in
  match cur_peek c with
  | Some '\\' ->
      let _ = cur_advance c in
      read_char_literal c start_line start_col
  | _ ->
      let buf = Buffer.create 8 in
      let rec eat () =
        match cur_peek c with
        | Some b when is_delim b -> ()
        | Some b -> Buffer.add_char buf b; let _ = cur_advance c in eat ()
        | None -> ()
      in
      eat ();
      let word = Buffer.contents buf in
      match word with
      | "true" -> Ast.Bool true
      | "false" -> Ast.Bool false
      | _ ->
          raise (ReadError
                   (Printf.sprintf
                      "read error at %d:%d: unknown hash form (minimal: #true #false #\\)"
                      start_line start_col))

(* main dispatch *)

let read_form_impl (c : cursor) : Ast.form =
  skip_trivia c;
  match cur_peek c with
  | None ->
      raise (ReadError
               (Printf.sprintf "read error at %d:%d: unexpected end of input"
                  c.line c.col))
  | Some '(' -> read_list c
  | Some ')' ->
      raise (ReadError (Printf.sprintf "read error at %d:%d: unexpected close paren"
                          c.line c.col))
  | Some '[' -> read_send_bracket c
  | Some ']' ->
      raise (ReadError (Printf.sprintf "read error at %d:%d: unexpected close bracket"
                          c.line c.col))
  | Some '{' ->
      raise (ReadError
               (Printf.sprintf
                  "read error at %d:%d: object-literal is banned in minimal subset"
                  c.line c.col))
  | Some '}' ->
      raise (ReadError (Printf.sprintf "read error at %d:%d: unexpected close brace"
                          c.line c.col))
  | Some '\'' -> read_quote c
  | Some '`' ->
      raise (ReadError
               (Printf.sprintf
                  "read error at %d:%d: quasiquote is banned in minimal subset"
                  c.line c.col))
  | Some ',' ->
      raise (ReadError
               (Printf.sprintf
                  "read error at %d:%d: unquote is banned in minimal subset"
                  c.line c.col))
  | Some '"' -> read_string_lit c
  | Some '#' -> read_hash c
  | Some _ -> read_atom c

let () = read_form_ref := read_form_impl

(* public API *)

let read_string (text : string) : Ast.form =
  let c = make_cursor text in
  skip_trivia c;
  let v = read_form c in
  skip_trivia c;
  if not (cur_at_end c) then
    raise (ReadError
             (Printf.sprintf "read error at %d:%d: unexpected trailing content"
                c.line c.col));
  v

let read_all (text : string) : Ast.form list =
  let c = make_cursor text in
  let out = ref [] in
  let rec loop () =
    skip_trivia c;
    if cur_at_end c then ()
    else begin
      out := read_form c :: !out;
      loop ()
    end
  in
  loop ();
  List.rev !out

let read_file (path : string) : Ast.form list =
  let ic = open_in path in
  let n = in_channel_length ic in
  let buf = Bytes.create n in
  really_input ic buf 0 n;
  close_in ic;
  read_all (Bytes.to_string buf)
