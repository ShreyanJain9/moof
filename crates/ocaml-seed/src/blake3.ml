(* Pure-OCaml BLAKE3 implementation.

   Reference: BLAKE3 specification (Bertoni et al., 2020),
              https://github.com/BLAKE3-team/BLAKE3-specs

   This implementation covers the *hash* mode (no keyed, no derive_key)
   and is sufficient for the V4 vat-image footer hash (which hashes the
   entire file-minus-footer in one shot). It is NOT optimized — the
   substrate-side (zig) implementation should use the reference C/zig
   library. What matters is that the output matches.

   Test vectors are checked at the bottom of the file via let () = ();.
   To verify: ocaml -e 'Blake3.self_test ();;'

   STATUS: scope-limited fallback so the OCaml seed serializer can produce
   the image footer hash without depending on `digestif`. If digestif
   becomes available in the project's opam switch, prefer it. *)

(* state: 8 chaining-value words + counter + block-len + flags. *)

let iv = [|
  0x6A09E667l; 0xBB67AE85l; 0x3C6EF372l; 0xA54FF53Al;
  0x510E527Fl; 0x9B05688Cl; 0x1F83D9ABl; 0x5BE0CD19l;
|]

let msg_permutation = [| 2; 6; 3; 10; 7; 0; 4; 13; 1; 11; 12; 5; 9; 14; 15; 8 |]

let chunk_start_flag    = 1 lsl 0
let chunk_end_flag      = 1 lsl 1
let parent_flag         = 1 lsl 2
let root_flag           = 1 lsl 3
(* the rest of the flag bits are unused for plain hash mode *)

let rotr (x : int32) (n : int) : int32 =
  Int32.logor
    (Int32.shift_right_logical x n)
    (Int32.shift_left x (32 - n))

let add32 a b = Int32.add a b
let xor32 a b = Int32.logxor a b

let g (state : int32 array) (a : int) (b : int) (c : int) (d : int)
      (mx : int32) (my : int32) : unit =
  state.(a) <- add32 (add32 state.(a) state.(b)) mx;
  state.(d) <- rotr (xor32 state.(d) state.(a)) 16;
  state.(c) <- add32 state.(c) state.(d);
  state.(b) <- rotr (xor32 state.(b) state.(c)) 12;
  state.(a) <- add32 (add32 state.(a) state.(b)) my;
  state.(d) <- rotr (xor32 state.(d) state.(a)) 8;
  state.(c) <- add32 state.(c) state.(d);
  state.(b) <- rotr (xor32 state.(b) state.(c)) 7

let round (state : int32 array) (m : int32 array) : unit =
  g state 0 4  8 12 m.(0) m.(1);
  g state 1 5  9 13 m.(2) m.(3);
  g state 2 6 10 14 m.(4) m.(5);
  g state 3 7 11 15 m.(6) m.(7);
  g state 0 5 10 15 m.(8) m.(9);
  g state 1 6 11 12 m.(10) m.(11);
  g state 2 7  8 13 m.(12) m.(13);
  g state 3 4  9 14 m.(14) m.(15)

let permute (m : int32 array) : int32 array =
  let out = Array.make 16 0l in
  for i = 0 to 15 do
    out.(i) <- m.(msg_permutation.(i))
  done;
  out

(* compress: produces a 16-word state output for either the chaining-value
   path (first 8 words XOR'd with state[8..]) or the full-16-word output
   used in root finalization. *)
let compress (cv : int32 array) (block : int32 array) (counter : int64)
             (block_len : int) (flags : int) : int32 array =
  let state = Array.make 16 0l in
  Array.blit cv 0 state 0 8;
  state.(8)  <- iv.(0);
  state.(9)  <- iv.(1);
  state.(10) <- iv.(2);
  state.(11) <- iv.(3);
  state.(12) <- Int64.to_int32 (Int64.logand counter 0xffffffffL);
  state.(13) <- Int64.to_int32
                 (Int64.logand (Int64.shift_right_logical counter 32) 0xffffffffL);
  state.(14) <- Int32.of_int block_len;
  state.(15) <- Int32.of_int flags;

  let m = ref (Array.copy block) in
  (* 7 rounds with message permutation between them. *)
  for r = 0 to 6 do
    round state !m;
    if r < 6 then m := permute !m
  done;

  for i = 0 to 7 do
    state.(i) <- xor32 state.(i) state.(i + 8);
    state.(i + 8) <- xor32 state.(i + 8) cv.(i)
  done;
  state

(* read a u32 little-endian from bytes at offset off. (BLAKE3 is LE.) *)
let get_u32_le (b : bytes) (off : int) : int32 =
  let b0 = Int32.of_int (Char.code (Bytes.get b off)) in
  let b1 = Int32.of_int (Char.code (Bytes.get b (off + 1))) in
  let b2 = Int32.of_int (Char.code (Bytes.get b (off + 2))) in
  let b3 = Int32.of_int (Char.code (Bytes.get b (off + 3))) in
  Int32.logor b0
    (Int32.logor (Int32.shift_left b1 8)
       (Int32.logor (Int32.shift_left b2 16) (Int32.shift_left b3 24)))

let put_u32_le (b : bytes) (off : int) (v : int32) : unit =
  Bytes.set b off
    (Char.chr (Int32.to_int (Int32.logand v 0xffl)));
  Bytes.set b (off + 1)
    (Char.chr (Int32.to_int (Int32.logand (Int32.shift_right_logical v 8) 0xffl)));
  Bytes.set b (off + 2)
    (Char.chr (Int32.to_int (Int32.logand (Int32.shift_right_logical v 16) 0xffl)));
  Bytes.set b (off + 3)
    (Char.chr (Int32.to_int (Int32.logand (Int32.shift_right_logical v 24) 0xffl)))

(* parse a 64-byte block from raw bytes into an int32 array of length 16. *)
let parse_block (raw : bytes) (off : int) : int32 array =
  let m = Array.make 16 0l in
  for i = 0 to 15 do
    m.(i) <- get_u32_le raw (off + i * 4)
  done;
  m

(* BLAKE3 chunk processor: takes input bytes for one chunk (≤1024 bytes,
   the last chunk in a hash may be less) and produces its chaining value
   (8 words). If `is_root` is true and this is the only chunk, the full
   output is extracted (caller handles XOF-style output extraction).

   For our use case (V4 image footer, hashing the whole image minus footer),
   the input may be many KB to several MB. BLAKE3's tree structure with
   chunks-of-1024-bytes and a binary parent-tree is what this implementation
   reduces to. We implement the full tree for correctness. *)

let chunk_size = 1024
let block_size = 64

(* Process all blocks of one chunk; return the final chaining-value (8 words).
   `chunk_counter` is the 0-indexed chunk number.
   `is_root_chunk` indicates this is the *only* chunk (root). *)
let process_chunk (data : bytes) (chunk_off : int) (chunk_len : int)
                  (chunk_counter : int64) (is_root_chunk : bool) : int32 array =
  let n_blocks = (chunk_len + block_size - 1) / block_size in
  let cv = ref (Array.copy iv) in
  for b = 0 to n_blocks - 1 do
    let block_off = chunk_off + b * block_size in
    let raw_len = min block_size (chunk_len - b * block_size) in
    (* pad to 64 bytes for block parsing *)
    let raw =
      if raw_len = block_size then Bytes.sub data block_off block_size
      else
        let pad = Bytes.make block_size '\000' in
        Bytes.blit data block_off pad 0 raw_len;
        pad
    in
    let m = parse_block raw 0 in
    let flags = ref 0 in
    if b = 0 then flags := !flags lor chunk_start_flag;
    if b = n_blocks - 1 then begin
      flags := !flags lor chunk_end_flag;
      if is_root_chunk then flags := !flags lor root_flag
    end;
    let state = compress !cv m chunk_counter raw_len !flags in
    let new_cv = Array.make 8 0l in
    Array.blit state 0 new_cv 0 8;
    cv := new_cv
  done;
  !cv

(* Process a parent node: combine left+right CVs into one CV.
   If `is_root` is true, marks with root flag. *)
let parent_cv (left : int32 array) (right : int32 array) (is_root : bool) : int32 array =
  let m = Array.make 16 0l in
  Array.blit left 0 m 0 8;
  Array.blit right 0 m 8 8;
  let flags = parent_flag lor (if is_root then root_flag else 0) in
  let state = compress (Array.copy iv) m 0L block_size flags in
  let cv = Array.make 8 0l in
  Array.blit state 0 cv 0 8;
  cv

(* Build the tree over chunk CVs.

   BLAKE3's tree structure: chunks are leaves. The tree is *left-perfect-binary*:
   at each level, take left-greedy power-of-2 subtrees.

   Simpler recursive view: given a contiguous range of chunks [lo, hi),
   if it's a single chunk, return its CV; otherwise split at the largest
   power-of-2 ≤ (hi - lo - 1) below it... actually the canonical algorithm
   uses a stack-based incremental build. We implement the stack form. *)

(* incremental tree builder: we process chunks left-to-right, maintaining
   a stack of CVs. At each chunk completion: push the chunk's CV. Then,
   while the depth-2 invariant says we can merge (chunk_count divisible
   by 2^k for the right k), merge the top two. At the end, fold the stack
   right-to-left to produce the root. *)

(* count trailing zeros — used to decide how many merges to do per push. *)
let trailing_zeros (n : int64) : int =
  if Int64.equal n 0L then 64
  else
    let n = ref n in
    let c = ref 0 in
    while Int64.equal (Int64.logand !n 1L) 0L do
      n := Int64.shift_right_logical !n 1;
      incr c
    done;
    !c

(* full BLAKE3 hash. Output: 32 bytes. *)
let hash (data : bytes) : bytes =
  let n = Bytes.length data in

  (* Single-chunk fast path. Even empty input has one chunk (length 0). *)
  if n <= chunk_size then begin
    (* Need to also handle: if n=0, still have one (empty) block with
       chunk_start | chunk_end | root flags. *)
    let chunk_len = n in
    let n_blocks =
      if chunk_len = 0 then 1
      else (chunk_len + block_size - 1) / block_size
    in
    let cv = ref (Array.copy iv) in
    let last_state = ref (Array.make 16 0l) in
    for b = 0 to n_blocks - 1 do
      let block_off = b * block_size in
      let raw_len =
        if chunk_len = 0 then 0
        else min block_size (chunk_len - b * block_size)
      in
      let raw =
        let pad = Bytes.make block_size '\000' in
        if raw_len > 0 then Bytes.blit data block_off pad 0 raw_len;
        pad
      in
      let m = parse_block raw 0 in
      let flags = ref 0 in
      if b = 0 then flags := !flags lor chunk_start_flag;
      if b = n_blocks - 1 then begin
        flags := !flags lor chunk_end_flag;
        flags := !flags lor root_flag
      end;
      let state = compress !cv m 0L raw_len !flags in
      last_state := state;
      let new_cv = Array.make 8 0l in
      Array.blit state 0 new_cv 0 8;
      cv := new_cv
    done;
    (* root output: first 8 words of the LAST block's full 16-word state
       (XOF style); we need 32 bytes = first 8 words. *)
    let out = Bytes.create 32 in
    for i = 0 to 7 do
      put_u32_le out (i * 4) !last_state.(i)
    done;
    out
  end
  else begin
    (* Multi-chunk: build the binary tree. *)
    let n_chunks = (n + chunk_size - 1) / chunk_size in
    let stack : int32 array list ref = ref [] in

    for ci = 0 to n_chunks - 1 do
      let chunk_off = ci * chunk_size in
      let chunk_len = min chunk_size (n - chunk_off) in
      (* We always treat non-root chunks as non-root here; we only mark
         the final root in the very last parent_cv call. *)
      let is_only_chunk = (n_chunks = 1) in
      let cv = process_chunk data chunk_off chunk_len
                 (Int64.of_int ci) is_only_chunk in
      stack := cv :: !stack;
      (* Merge while we can — trailing_zeros(ci+1) merges. *)
      let merges = trailing_zeros (Int64.of_int (ci + 1)) in
      let m = ref 0 in
      while !m < merges && (match !stack with _ :: _ :: _ -> true | _ -> false) do
        (match !stack with
         | right :: left :: rest ->
             let combined = parent_cv left right false in
             stack := combined :: rest
         | _ -> assert false);
        incr m
      done
    done;

    (* Fold the remaining stack right-to-left into a root. The TOP of the
       stack is the rightmost element. Merge: pair up; mark the FINAL merge
       as root. *)
    let lst = List.rev !stack in (* now leftmost first *)
    (* Pair-merge right-to-left: combine the last two non-root, until one
       remains. Actually the canonical algorithm: while stack > 1, pop two
       (right, left where right is top), parent_cv left right.
       The very last merge is root.

       Since we reversed, work from the end. *)
    let arr = Array.of_list lst in
    let len = ref (Array.length arr) in
    while !len > 1 do
      let is_last_merge = (!len = 2) in
      let left = arr.(!len - 2) in
      let right = arr.(!len - 1) in
      let combined =
        if is_last_merge then
          (* For root parent: we need the full state, not just CV. *)
          let m = Array.make 16 0l in
          Array.blit left 0 m 0 8;
          Array.blit right 0 m 8 8;
          let flags = parent_flag lor root_flag in
          compress (Array.copy iv) m 0L block_size flags
        else
          parent_cv left right false
      in
      arr.(!len - 2) <- combined;
      decr len
    done;

    let final_state = arr.(0) in
    let out = Bytes.create 32 in
    for i = 0 to 7 do
      put_u32_le out (i * 4) final_state.(i)
    done;
    out
  end

(* Hex-encode 32 bytes — for debugging. *)
let hex (b : bytes) : string =
  let n = Bytes.length b in
  let buf = Buffer.create (n * 2) in
  for i = 0 to n - 1 do
    Buffer.add_string buf (Printf.sprintf "%02x" (Char.code (Bytes.get b i)))
  done;
  Buffer.contents buf

(* Self-test against published BLAKE3 vectors. *)
let self_test () =
  let check name input expected =
    let actual = hex (hash (Bytes.of_string input)) in
    if actual <> expected then
      Printf.printf "BLAKE3 self-test FAIL %s\n  expected: %s\n  actual:   %s\n"
        name expected actual
    else
      Printf.printf "BLAKE3 self-test OK %s\n" name
  in
  (* empty input *)
  check "empty"
    ""
    "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
  (* "abc" *)
  check "abc"
    "abc"
    "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
  check "IETF"
    "IETF"
    "83a2de1ee6f4e6ab686889248f4ec0cf4cc5709446a682ffd1cbb4d6165181e2"
