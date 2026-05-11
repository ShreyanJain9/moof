(* smoke tests for the reader. checked into the project so the
   integration agent has a baseline; not intended to be exhaustive. *)

open Moof_seed

let failures = ref 0

let check name expected actual =
  if expected = actual then
    Printf.printf "  ok   %s\n" name
  else begin
    Printf.printf "  FAIL %s\n    expected: %s\n    actual:   %s\n"
      name expected actual;
    incr failures
  end

let parse s = Ast.to_string (Reader.read_string s)

let () =
  print_endline "== reader smoke tests ==";

  (* literals *)
  check "nil" "nil" (parse "nil");
  check "true" "#true" (parse "#true");
  check "false" "#false" (parse "#false");
  check "integer" "42" (parse "42");
  check "negative integer" "-7" (parse "-7");
  check "hex" "255" (parse "0xff");
  check "binary" "10" (parse "0b1010");
  check "underscored" "1000000" (parse "1_000_000");
  check "symbol" "foo" (parse "foo");
  check "kebab symbol" "foo-bar" (parse "foo-bar");
  check "keyword sym" "key:" (parse "key:");
  check "multi-keyword sym" "a:b:c:" (parse "a:b:c:");

  (* lists *)
  check "empty list" "nil" (parse "()");  (* () is read as Nil — proper-list terminator *)
  check "list" "(a b c)" (parse "(a b c)");
  check "nested list" "(a (b c) d)" (parse "(a (b c) d)");

  (* quotes *)
  check "quote" "(quote x)" (parse "'x");
  check "quasiquote" "(quasiquote x)" (parse "`x");
  check "unquote" "(unquote x)" (parse ",x");
  check "splice" "(unquote-splicing x)" (parse ",@x");

  (* send brackets — the load-bearing test *)
  check "unary send" "(__send__ a method)" (parse "[a method]");
  check "binary send" "(__send__ a + b)" (parse "[a + b]");
  check "binary send 1+2" "(__send__ 1 + 2)" (parse "[1 + 2]");
  check "keyword send" "(__send__ a at: 1)" (parse "[a at: 1]");
  check "multi-keyword send" "(__send__ a at:put: 1 2)" (parse "[a at: 1 put: 2]");
  check "positional send" "(__send__ a method 1 2)" (parse "[a method 1 2]");

  (* cascade *)
  check "cascade"
    "(__cascade__ a (foo) (bar 1))"
    (parse "[a foo; bar 1]");

  (* hash + char *)
  check "char literal" "#\\u{61}" (parse "#\\a");
  check "named char space" "#\\u{20}" (parse "#\\space");
  check "unicode char" "#\\u{1f496}" (parse "#\\u{1f496}");

  (* table literal *)
  check "table" "(__table__ 1 2 3)" (parse "#[1 2 3]");
  check "table with arrow"
    "(__table__ (__entry__ k 1))"
    (parse "#[k => 1]");

  (* object literal — params list is `nil` for nullary methods *)
  check "object literal"
    "(__obj__ Foo (__slot__ a 1) (__method__ ping nil body))"
    (parse "{Foo a: 1 [ping] body}");

  (* block syntax — params list is `nil` for nullary blocks *)
  check "nullary block" "(fn nil body)" (parse "|| body");
  check "unary block"
    "(fn (x) (__send__ x + 1))"
    (parse "|x| [x + 1]");

  (* .foo self-send shorthand *)
  check "self shorthand" "(__send__ self foo)" (parse ".foo");

  (* strings *)
  check "string" "\"hello\"" (parse "\"hello\"");
  check "string with escape"
    "\"line1\\nline2\""
    (parse "\"line1\\nline2\"");

  (* comments *)
  check "line comment" "x" (parse ";; comment\nx");

  (* multi-form read_all *)
  let forms = Reader.read_all "1 2 3" in
  check "read_all count" "3" (string_of_int (List.length forms));

  Printf.printf "\n%d failures\n" !failures;
  if !failures > 0 then exit 1
