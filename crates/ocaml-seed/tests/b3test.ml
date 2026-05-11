(* BLAKE3 regression smoke. Checks several BLAKE3 official test vectors
   against the pure-OCaml implementation in src/blake3.ml. Run via:
     dune exec ./tests/b3test.exe
   Exits non-zero on any mismatch. *)

let want_eq label actual expected =
  if actual = expected then
    Printf.printf "OK   %s: %s\n" label actual
  else begin
    Printf.printf "FAIL %s\n  expected: %s\n  actual:   %s\n"
      label expected actual;
    exit 1
  end

let h b = Moof_seed.Blake3.hex (Moof_seed.Blake3.hash b)

let cycle n =
  let b = Bytes.create n in
  for i = 0 to n - 1 do
    Bytes.set b i (Char.chr (i mod 251))
  done;
  b

let () =
  (* official BLAKE3 test vectors *)
  want_eq "empty"
    (h (Bytes.of_string ""))
    "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
  want_eq "abc"
    (h (Bytes.of_string "abc"))
    "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
  want_eq "IETF"
    (h (Bytes.of_string "IETF"))
    "83a2de1ee6f4e6ab686889248f4ec0cf4cc5709446a682ffd1cbb4d6165181e2";
  (* boundary cases for the chunk-tree path *)
  want_eq "cycle 1023" (h (cycle 1023))
    "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11";
  want_eq "cycle 1024" (h (cycle 1024))
    "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7";
  want_eq "cycle 1025" (h (cycle 1025))
    "9b4dec86204c5fe533c456aaf35debfc8cfb7d569f2f4be8bf616dd5144054c1";
  print_endline "all BLAKE3 vectors match"
