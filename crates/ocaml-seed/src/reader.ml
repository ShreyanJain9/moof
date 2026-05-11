(* moof seed reader.
   port of crates/substrate/src/reader.rs into OCaml.

   produces Ast.form values from source text. mirrors the rust
   reader's surface: s-expressions, send-brackets `[…]`, object
   literals `{…}`, table literals `#[…]`, char literals `#\…`,
   quote/quasiquote/unquote sugar, `|args| body` blocks, `.foo`
   self-send shorthand, and the moof-flavored numeric grammar.

   the reader desugars at the parse layer (matching the rust
   reader): send brackets emit `(__send__ recv 'sel args…)`,
   object literals emit `(__obj__ Proto entries…)`, etc. the
   compiler consumes these marker symbols. *)

exception ReadError of string

(* --- cursor --- *)

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

let read_error (c : cursor) (msg : string) : 'a =
  raise (ReadError (Printf.sprintf "read error at %d:%d: %s" c.line c.col msg))

(* --- character classes --- *)

let is_ascii_whitespace (b : char) : bool =
  match b with ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' -> true | _ -> false

let is_delim (b : char) : bool =
  match b with
  | '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"' | ';' | '`' | ',' -> true
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

(* --- skip whitespace + comments ---
   per the rust reader: `;;` / `;:` / `;~` introduce a line comment.
   a bare `;` is reserved for cascade separation inside brackets. *)
let rec skip_trivia (c : cursor) : unit =
  match cur_peek c with
  | Some b when is_ascii_whitespace b ->
      let _ = cur_advance c in
      skip_trivia c
  | Some ';' ->
      (match cur_peek_at c 1 with
       | Some (';' | ':' | '~') ->
           (* skip until newline *)
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

(* --- forward decls (mutual recursion via refs) --- *)
let read_form_ref : (cursor -> Ast.form) ref =
  ref (fun _ -> failwith "read_form not initialized")

let read_form (c : cursor) : Ast.form = !read_form_ref c

(* --- numeric parsing --- *)

(* strip underscores, then parse with explicit base prefixes.
   returns None if not a numeric literal. *)
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

(* --- bare atom ---

   pipe-character special: when we land on `|` here, the caller's
   `looks_like_block` discriminator already determined this isn't
   a block, so consume a maximal run of `|` chars as a binary
   operator selector (matches the rust reader). *)
let read_atom (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  match cur_peek c with
  | Some '|' ->
      let buf = Buffer.create 4 in
      let rec eat () =
        match cur_peek c with
        | Some '|' -> Buffer.add_char buf '|'; let _ = cur_advance c in eat ()
        | _ -> ()
      in
      eat ();
      Ast.Sym (Buffer.contents buf)
  | _ ->
      let buf = Buffer.create 16 in
      let rec eat () =
        match cur_peek c with
        | Some b when is_delim b || b = '|' -> ()
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
        | None ->
            if String.length text > 1 && text.[0] = '.' && text <> "." then
              let rest = String.sub text 1 (String.length text - 1) in
              if rest <> "" && rest <> "." then
                Ast.forms_to_list
                  [Ast.Sym "__send__"; Ast.Sym "self"; Ast.Sym rest]
              else Ast.Sym text
            else Ast.Sym text

(* --- string literal --- *)

let read_string_lit (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  (* consume opening double-quote *)
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

(* --- char literal — entered with cursor *after* `#\` --- *)

let read_char_literal (c : cursor) (start_line : int) (start_col : int) : Ast.form =
  (* unicode escape: u{HEX} *)
  if cur_peek c = Some 'u' && cur_peek_at c 1 = Some '{' then begin
    let _ = cur_advance c in
    let _ = cur_advance c in
    let buf = Buffer.create 4 in
    let rec eat () =
      match cur_peek c with
      | Some '}' -> let _ = cur_advance c in ()
      | Some b when is_ascii_hexdigit b ->
          Buffer.add_char buf b;
          let _ = cur_advance c in eat ()
      | _ ->
          raise (ReadError
                   (Printf.sprintf "read error at %d:%d: malformed `#\\u{…}` char literal"
                      start_line start_col))
    in
    eat ();
    let hex = Buffer.contents buf in
    let cp = try int_of_string ("0x" ^ hex) with _ ->
      raise (ReadError
               (Printf.sprintf "read error at %d:%d: invalid hex codepoint #\\u{%s}"
                  start_line start_col hex)) in
    Ast.Char cp
  end else begin
    let first =
      match cur_advance c with
      | Some b -> b
      | None ->
          raise (ReadError
                   (Printf.sprintf "read error at %d:%d: unterminated #\\ char literal"
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
                   (Printf.sprintf "read error at %d:%d: unknown char name #\\%s"
                      start_line start_col text))
  end

(* --- list `(…)` --- *)

let read_list (c : cursor) : Ast.form =
  let _ = cur_advance c in  (* consume `(` *)
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

(* --- quote sugar --- *)

let read_quote (c : cursor) : Ast.form =
  let _ = cur_advance c in
  let inner = read_form c in
  Ast.forms_to_list [Ast.Sym "quote"; inner]

let read_quasiquote (c : cursor) : Ast.form =
  let _ = cur_advance c in
  let inner = read_form c in
  Ast.forms_to_list [Ast.Sym "quasiquote"; inner]

let read_unquote (c : cursor) : Ast.form =
  let _ = cur_advance c in
  let splicing = cur_peek c = Some '@' in
  if splicing then (let _ = cur_advance c in ());
  let inner = read_form c in
  let head = if splicing then "unquote-splicing" else "unquote" in
  Ast.forms_to_list [Ast.Sym head; inner]

(* --- send bracket `[…]` ---

   shapes (match the rust reader):
   - `[recv selector]`         — unary send
   - `[recv OP arg]`           — binary send (OP = operator-chars only)
   - `[recv selector args…]`   — positional send
   - `[recv kw1: a kw2: b …]`  — keyword send, selector = concat of kw:'s
   - `[recv a; b; c: x]`       — cascade

   non-cascade => `(__send__ recv 'sel arg…)`
   cascade     => `(__cascade__ recv (sel args…) …)` *)

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
      (* binary: 2 elements, first is operator-only *)
      if is_binary_op first_text && len = 2 then
        (first_text, [List.nth elements 1])
      (* keyword: first ends in `:` *)
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
                            "read error at %d:%d: keyword send: expected `kw:` symbol"
                            line col))
          in
          if String.length kw = 0 || kw.[String.length kw - 1] <> ':' then
            raise (ReadError
                     (Printf.sprintf "read error at %d:%d: keyword `%s` must end with `:`"
                        line col kw));
          Buffer.add_string sel_buf kw;
          incr i;
          if !i >= n then
            raise (ReadError
                     (Printf.sprintf "read error at %d:%d: keyword `%s` needs an argument"
                        line col kw));
          args := arr.(!i) :: !args;
          incr i
        done;
        (Buffer.contents sel_buf, List.rev !args)
      end else
        (first_text, rest)

let emit_send (recv : Ast.form) (sel : string) (args : Ast.form list) : Ast.form =
  Ast.forms_to_list ([Ast.Sym "__send__"; recv; Ast.Sym sel] @ args)

let emit_cascade (recv : Ast.form) (segments : (string * Ast.form list) list)
    : Ast.form =
  let seg_forms =
    List.map (fun (sel, args) ->
      Ast.forms_to_list (Ast.Sym sel :: args)) segments
  in
  Ast.forms_to_list ([Ast.Sym "__cascade__"; recv] @ seg_forms)

let read_send_bracket (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let _ = cur_advance c in  (* consume `[` *)
  skip_trivia c;
  if cur_peek c = Some ']' then
    raise (ReadError (Printf.sprintf "read error at %d:%d: empty send bracket `[]`"
                        start_line start_col));
  let receiver = read_form c in
  let segments = ref [] in
  let rec loop_segments () =
    let elems = ref [] in
    let rec eat_elems () =
      skip_trivia c;
      match cur_peek c with
      | None ->
          raise (ReadError
                   (Printf.sprintf "read error at %d:%d: unterminated send bracket"
                      start_line start_col))
      | Some ']' -> ()
      | Some ';' -> let _ = cur_advance c in ()
      | Some _ -> elems := read_form c :: !elems; eat_elems ()
    in
    eat_elems ();
    let elem_list = List.rev !elems in
    if elem_list = [] then begin
      if !segments = [] then
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: send needs at least a selector"
                    start_line start_col))
      else
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: empty cascade segment after `;`"
                    start_line start_col))
    end;
    let seg = decode_send_segment elem_list start_line start_col in
    segments := seg :: !segments;
    if cur_peek c = Some ']' then (let _ = cur_advance c in ())
    else loop_segments ()
  in
  loop_segments ();
  let segs = List.rev !segments in
  match segs with
  | [(sel, args)] -> emit_send receiver sel args
  | _ -> emit_cascade receiver segs

(* --- table literal `#[ … ]` ---

   each entry is either a bare positional form or
   `(__entry__ key val)` if separated by `=>`. *)

let peek_arrow (c : cursor) : bool =
  if c.pos + 1 >= c.len then false
  else
    Bytes.get c.bytes c.pos = '=' && Bytes.get c.bytes (c.pos + 1) = '>'
    && (c.pos + 2 >= c.len || is_delim (Bytes.get c.bytes (c.pos + 2)))

let consume_arrow (c : cursor) : unit =
  let _ = cur_advance c in
  let _ = cur_advance c in
  ()

let read_table_literal (c : cursor) (start_line : int) (start_col : int) : Ast.form =
  let _ = cur_advance c in  (* consume `[` *)
  let entries = ref [] in
  let rec loop () =
    skip_trivia c;
    match cur_peek c with
    | None ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unterminated `#[…]` table literal"
                    start_line start_col))
    | Some ']' -> let _ = cur_advance c in ()
    | Some _ ->
        let elem = read_form c in
        skip_trivia c;
        if peek_arrow c then begin
          consume_arrow c;
          skip_trivia c;
          (match cur_peek c with
           | None | Some ']' ->
               raise (ReadError
                        (Printf.sprintf "read error at %d:%d: `=>` expects a value after it"
                           start_line start_col))
           | _ -> ());
          let v = read_form c in
          entries := Ast.forms_to_list [Ast.Sym "__entry__"; elem; v] :: !entries
        end else
          entries := elem :: !entries;
        loop ()
  in
  loop ();
  Ast.forms_to_list (Ast.Sym "__table__" :: List.rev !entries)

(* --- object literal `{ Proto k: v [sel] body … }` --- *)

let decode_method_header (tokens : Ast.form list)
    (line : int) (col : int) : string * Ast.form list =
  match tokens with
  | [] ->
      raise (ReadError
               (Printf.sprintf "read error at %d:%d: empty method header" line col))
  | first :: rest ->
      let first_text =
        match first with
        | Ast.Sym s -> s
        | _ ->
            raise (ReadError
                     (Printf.sprintf
                        "read error at %d:%d: method header: selector must be a symbol"
                        line col))
      in
      let len = List.length tokens in
      if is_binary_op first_text && len = 2 then
        (first_text, [List.nth tokens 1])
      else if String.length first_text > 0
              && first_text.[String.length first_text - 1] = ':' then begin
        let sel_buf = Buffer.create 16 in
        let params = ref [] in
        let arr = Array.of_list tokens in
        let n = Array.length arr in
        let i = ref 0 in
        while !i < n do
          let kw =
            match arr.(!i) with
            | Ast.Sym s -> s
            | _ ->
                raise (ReadError
                         (Printf.sprintf
                            "read error at %d:%d: keyword method header: expected `kw:` symbol"
                            line col))
          in
          if String.length kw = 0 || kw.[String.length kw - 1] <> ':' then
            raise (ReadError
                     (Printf.sprintf "read error at %d:%d: keyword `%s` must end with `:`"
                        line col kw));
          Buffer.add_string sel_buf kw;
          incr i;
          if !i >= n then
            raise (ReadError
                     (Printf.sprintf "read error at %d:%d: keyword `%s` needs a parameter"
                        line col kw));
          params := arr.(!i) :: !params;
          incr i
        done;
        (Buffer.contents sel_buf, List.rev !params)
      end else
        (first_text, rest)

let read_method_header_tokens (c : cursor) : Ast.form list =
  let tokens = ref [] in
  let rec eat () =
    skip_trivia c;
    match cur_peek c with
    | None ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unterminated method header"
                    c.line c.col))
    | Some ']' -> let _ = cur_advance c in ()
    | Some _ -> tokens := read_form c :: !tokens; eat ()
  in
  eat ();
  List.rev !tokens

let read_object_literal (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let _ = cur_advance c in  (* consume `{` *)
  let proto = ref (Ast.Sym "Object") in
  let entries = ref [] in
  let has_proto = ref false in
  let rec loop () =
    skip_trivia c;
    match cur_peek c with
    | None ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unterminated `{…}` object literal"
                    start_line start_col))
    | Some '}' -> let _ = cur_advance c in ()
    | Some '[' ->
        (* method header *)
        let _ = cur_advance c in
        let header = read_method_header_tokens c in
        skip_trivia c;
        let body = read_form c in
        let sel, params = decode_method_header header start_line start_col in
        let params_list = Ast.forms_to_list params in
        let entry =
          Ast.forms_to_list
            [Ast.Sym "__method__"; Ast.Sym sel; params_list; body]
        in
        entries := entry :: !entries;
        loop ()
    | Some _ ->
        let form = read_form c in
        (match form with
         | Ast.Sym s when String.length s > 0 && s.[String.length s - 1] = ':' ->
             (* slot binding *)
             let key = String.sub s 0 (String.length s - 1) in
             skip_trivia c;
             let value = read_form c in
             entries :=
               Ast.forms_to_list
                 [Ast.Sym "__slot__"; Ast.Sym key; value] :: !entries;
             loop ()
         | Ast.Sym _ when not !has_proto && !entries = [] ->
             proto := form;
             has_proto := true;
             loop ()
         | _ ->
             raise (ReadError
                      (Printf.sprintf
                         "read error at %d:%d: object literal: expected `name:` slot binding, `[…]` method, or proto symbol"
                         start_line start_col)))
  in
  loop ();
  Ast.forms_to_list ([Ast.Sym "__obj__"; !proto] @ List.rev !entries)

(* --- hash forms `#…` --- *)

let read_hash (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let _ = cur_advance c in
  match cur_peek c with
  | Some '\\' ->
      let _ = cur_advance c in
      read_char_literal c start_line start_col
  | Some '[' -> read_table_literal c start_line start_col
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
                      "read error at %d:%d: unknown hash form `#%s` (supports `#true`, `#false`, `#\\…`, `#[…]`)"
                      start_line start_col word))

(* --- block `|args| body` ---

   discriminate from binary `|` operator: scan ahead from the
   opening `|`; if there's a closing `|` at the current nesting
   level before any structural terminator, it's a block. *)

let looks_like_block (c : cursor) : bool =
  (* c.pos points at the opening `|`; scan from c.pos+1 *)
  let depth = ref 0 in
  let i = ref (c.pos + 1) in
  let found = ref false in
  let done_ = ref false in
  while not !done_ && !i < c.len do
    let b = Bytes.get c.bytes !i in
    (match b with
     | '|' when !depth = 0 -> found := true; done_ := true
     | '(' | '[' | '{' -> incr depth
     | ')' | ']' | '}' ->
         if !depth = 0 then done_ := true
         else decr depth
     | ';' when !depth = 0 -> done_ := true
     | _ -> ());
    incr i
  done;
  !found

let read_block (c : cursor) : Ast.form =
  let start_line = c.line and start_col = c.col in
  let _ = cur_advance c in  (* consume opening `|` *)
  let params = ref [] in
  let rec loop () =
    skip_trivia c;
    match cur_peek c with
    | Some '|' -> let _ = cur_advance c in ()
    | None ->
        raise (ReadError
                 (Printf.sprintf "read error at %d:%d: unterminated `|args|` block params"
                    start_line start_col))
    | Some _ -> params := read_form c :: !params; loop ()
  in
  loop ();
  skip_trivia c;
  let body = read_form c in
  let params_list = Ast.forms_to_list (List.rev !params) in
  Ast.forms_to_list [Ast.Sym "fn"; params_list; body]

(* --- main dispatch --- *)

let read_form_impl (c : cursor) : Ast.form =
  skip_trivia c;
  match cur_peek c with
  | None ->
      raise (ReadError
               (Printf.sprintf "read error at %d:%d: unexpected end of input"
                  c.line c.col))
  | Some '(' -> read_list c
  | Some ')' ->
      raise (ReadError (Printf.sprintf "read error at %d:%d: unexpected `)`"
                          c.line c.col))
  | Some '[' -> read_send_bracket c
  | Some ']' ->
      raise (ReadError (Printf.sprintf "read error at %d:%d: unexpected `]`"
                          c.line c.col))
  | Some '{' -> read_object_literal c
  | Some '}' ->
      raise (ReadError (Printf.sprintf "read error at %d:%d: unexpected `}`"
                          c.line c.col))
  | Some '\'' -> read_quote c
  | Some '`' -> read_quasiquote c
  | Some ',' -> read_unquote c
  | Some '"' -> read_string_lit c
  | Some '#' -> read_hash c
  | Some '|' when looks_like_block c -> read_block c
  | Some _ -> read_atom c

let () = read_form_ref := read_form_impl

(* --- public API --- *)

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
