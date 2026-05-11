# Self-Host + Rust Deletion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to dispatch parallel workers across W1–W4 in stage 1, then one sequential agent for W5. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the rust runtime with `moof` (zig substrate) + `moof-seed` (ocaml build tool), via a new `parser.moof` and a V4-aware `compiler.moof`. End-state: rust is gone; users run `moof`; developers build `system.vat` with `moof-seed` + `moof`.

**Architecture:** Five workstreams. W1 (compiler.moof V4 audit), W2 (parser.moof), W3 (ocaml-seed minimization), W4 (zig substrate proof-of-life + serialize) run in parallel in stage 1 — 6 subagents total. W5 (integration + rust deletion + rename) is sequential in stage 2 and gates on all stage-1 streams.

**Tech Stack:** Zig 0.16.0+ (substrate host), OCaml 5.x + dune (seed compiler, build-time only), moof (everything else — minimal subset for bootstrap files, full surface for post-bootstrap), Rust (build-only during W1–W4; deleted in W5).

**Authoritative references (every subagent must read these first):**
- `docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md` — design
- `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` — V4 ISA + image format contract
- `NEXT_SESSION.md` — current state snapshot (HEAD `380203b`)

---

## File Structure

| file | role | task |
|---|---|---|
| `lib/compiler/00-helpers.moof` | Compiler singleton + helpers (V4 audit) | W1 modify |
| `lib/compiler/01-dispatch.moof` | form dispatcher | W1 modify |
| `lib/compiler/02-special.moof` | special forms (def, fn, if, set!, let, do, quote) | W1 modify |
| `lib/compiler/03-control.moof` | control flow + jumps | W1 modify |
| `crates/substrate/src/bin/byte_differ.rs` | rust corpus diff tool | W1 create |
| `tests/byte-corpus/*.moof` | small forms to compile + diff | W1 create |
| `lib/parser/00-lexer.moof` | char-stream → token-stream | W2 create |
| `lib/parser/01-tokens.moof` | token utils + FormLoc construction | W2 create |
| `lib/parser/02-parser.moof` | recursive-descent token-stream → Form-tree | W2 create |
| `lib/parser/03-bootstrap.moof` | Parser singleton + `[$reader useMoof]` flip | W2 create |
| `lib/main.moof` | add parser load before compiler load | W2 modify |
| `tests/parser-corpus/*.moof` | strings to parse via both readers and diff | W2 create |
| `crates/ocaml-seed/src/{ast,reader,opcodes,bytecode,compiler,image}.ml` | minimize to subset | W3 modify |
| `crates/ocaml-seed/bin/seed.ml` | `build-seed` subcommand | W3 modify |
| `crates/zig-substrate/src/main.zig` | `exec` + `serialize` CLI subcommands | W4 modify |
| `crates/zig-substrate/src/intrinsics.zig` | `:serialize-world` native | W4 modify |
| `crates/zig-substrate/src/image.zig` | world-to-bytes serializer | W4 modify |
| `crates/zig-substrate/src/vm.zig` | bug-fixes uncovered by exec smokes | W4 modify |
| `crates/zig-substrate/build.zig` | rename binary `moof-zig` → `moof` | W5 modify |
| `Cargo.toml` (workspace) | remove `substrate` member | W5 modify |
| `crates/substrate/src/*.rs` | DELETE runtime files (reader, compiler, vm, opcodes, intrinsics, nursery, transporter) | W5 delete |

---

## Stage 1 — Parallel Workstreams

Dispatch all 6 agents simultaneously. Each is self-contained against the V4 spec + design doc.

---

### W1 — `compiler.moof` V4 audit

Stream owner: **agent α**. Effort: ~3–4h. Risk: low (mechanical).

#### Task W1.1: Inventory current emissions

**Files:** none yet (research task)

- [ ] **Step 1:** Read V4 spec §2 + §3 + §4 + §5 (opcode set, byte tags, operand layout, emission rules).
- [ ] **Step 2:** Read `lib/compiler/00-helpers.moof`, `01-dispatch.moof`, `02-special.moof`, `03-control.moof` end to end.
- [ ] **Step 3:** For each `(setHandler! Compiler 'compileX:...)` clause, record:
  - Which opcode(s) it emits (e.g., `[Opcode pushNil]`, `[Opcode send: ...]`)
  - What operands it passes
  - Which form-shapes it handles
- [ ] **Step 4:** Save the inventory at `tmp/w1-emission-inventory.md` (not committed; reference for next steps).

#### Task W1.2: Write byte-differ tool

**Files:**
- Create: `crates/substrate/src/bin/byte_differ.rs`
- Modify: `crates/substrate/Cargo.toml` (add `[[bin]]` entry)

- [ ] **Step 1:** Add bin entry to `crates/substrate/Cargo.toml`:

```toml
[[bin]]
name = "byte-differ"
path = "src/bin/byte_differ.rs"
```

- [ ] **Step 2:** Create `crates/substrate/src/bin/byte_differ.rs`. The tool: takes a `.moof` source path as argv[1]; loads it; compiles once with `World::use_moof_compiler = false` (rust path); compiles again with `World::use_moof_compiler = true` (moof path); hex-dumps both byte streams + computes the first divergent offset.

Skeleton:

```rust
use moof::{World, Form, Compiler};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path: PathBuf = std::env::args().nth(1).expect("usage: byte-differ <file.moof>").into();
    let src = std::fs::read_to_string(&path)?;

    let mut world = World::new_for_compile_diff();
    let form = world.read_one(&src)?;

    // rust compiler path
    world.set_use_moof_compiler(false);
    let chunk_rust = world.compile_top(&form)?;
    let bytes_rust = world.chunk_bytecode(chunk_rust);

    // moof compiler path
    world.set_use_moof_compiler(true);
    let chunk_moof = world.compile_top(&form)?;
    let bytes_moof = world.chunk_bytecode(chunk_moof);

    if bytes_rust == bytes_moof {
        println!("MATCH ({} bytes)", bytes_rust.len());
        return Ok(());
    }

    // find first divergence
    let mut div = 0;
    while div < bytes_rust.len().min(bytes_moof.len()) && bytes_rust[div] == bytes_moof[div] {
        div += 1;
    }
    println!("DIFF at offset {}:", div);
    println!("  rust: {}", hex(&bytes_rust));
    println!("  moof: {}", hex(&bytes_moof));
    std::process::exit(1);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")
}
```

(Adjust names: `set_use_moof_compiler`, `chunk_bytecode`, `compile_top` may already exist on `World` under slightly different names — match the existing rust API.)

- [ ] **Step 3:** Build it: `cargo build -p moof --bin byte-differ`. Expected: clean build, possibly with a few mismatched-name errors to fix by reading `crates/substrate/src/world.rs`.

- [ ] **Step 4:** Commit:

```bash
git add crates/substrate/src/bin/byte_differ.rs crates/substrate/Cargo.toml
git commit -m "tooling: byte-differ — diff rust vs moof compiler emission"
```

#### Task W1.3: Build corpus + run initial diff

**Files:**
- Create: `tests/byte-corpus/*.moof` (~30 files)

- [ ] **Step 1:** Create `tests/byte-corpus/` directory.

- [ ] **Step 2:** Add corpus files (one form per file, named descriptively). Minimum set:

```
01-int-literal.moof:        42
02-bool-literal.moof:       #true
03-nil-literal.moof:        nil
04-sym-literal.moof:        'foo
05-send-binary.moof:        [1 + 2]
06-send-no-arg.moof:        [obj foo]
07-send-3-arg.moof:         [a b: 1 c: 2 d: 3]
08-if.moof:                 (if #true 1 2)
09-if-peephole.moof:        (if [x is nil] 1 2)
10-def.moof:                (def x 42)
11-set.moof:                (do (def x 1) (set! x 2))
12-fn-noargs.moof:          (fn () 42)
13-fn-args.moof:            (fn (a b) [a + b])
14-let.moof:                (let ((x 1)) x)
15-do.moof:                 (do 1 2 3)
16-quote-list.moof:         '(1 2 3)
17-send-self.moof:          [self foo]
18-send-here.moof:          [$here lookup: 'x]
19-tail-send.moof:           ;; from fn body to confirm TailSend
                            (fn () [obj foo])
20-super-send.moof:         (fn () [super foo])
21-nested-fn.moof:          (fn () (fn () 42))
22-load-name.moof:          x
23-const-fold-plus.moof:    [1 + 2]      ;; should peephole-fold to LoadConst 3
24-string-literal.moof:     "hello"
25-char-literal.moof:       #\a
26-cascade.moof:            [a foo ; bar]  ;; if compiler.moof handles `;`
27-multiclause-send.moof:   [a foo: 1 ; bar: 2]
28-load-self-receiver.moof: [self foo: 1]
29-send-dynamic.moof:       [a perform: 'foo withArgs: '(1)]
30-nested-send.moof:        [[a foo] bar]
```

- [ ] **Step 3:** Run the differ over the corpus:

```bash
for f in tests/byte-corpus/*.moof; do
  echo "=== $f ==="
  cargo run -q -p moof --bin byte-differ -- "$f" || true
done | tee tmp/w1-initial-diff-report.txt
```

- [ ] **Step 4:** Commit corpus:

```bash
git add tests/byte-corpus/
git commit -m "test: byte-corpus — V4 emission diff between rust and moof compilers"
```

#### Task W1.4: Fix divergences (iterate)

For each corpus file that reports DIFF:

- [ ] **Step 1:** Identify the failing op in the diff report.
- [ ] **Step 2:** Find the relevant `(setHandler! Compiler 'compileX:...)` clause in `lib/compiler/*.moof`.
- [ ] **Step 3:** Compare its emission to V4 spec §3 (byte tags) / §4 (operand layout) / §5 (emission rules).
- [ ] **Step 4:** Edit the offending clause. Most common fixes:
  - opcode byte tag (V3 had different tags than V4)
  - operand width (some V3 ops used u16, V4 may want u32)
  - peephole optimization missing (esp. const-fold and if-jump)
  - Send-variant choice (SendSelf vs Send-with-self-receiver)
- [ ] **Step 5:** Rerun differ on that one file:

```bash
cargo run -q -p moof --bin byte-differ -- tests/byte-corpus/05-send-binary.moof
```

Expected: `MATCH (N bytes)`.

- [ ] **Step 6:** Once entire corpus passes, commit:

```bash
git add lib/compiler/
git commit -m "compiler.moof: V4 emission — match rust byte-for-byte on test corpus"
```

#### Task W1.5: Exit-criterion smoke

- [ ] **Step 1:** Run the full corpus diff loop. All entries must report MATCH:

```bash
for f in tests/byte-corpus/*.moof; do
  result=$(cargo run -q -p moof --bin byte-differ -- "$f" 2>&1)
  if [[ "$result" != MATCH* ]]; then
    echo "FAIL: $f"; echo "$result"; exit 1
  fi
done
echo "W1 done."
```

- [ ] **Step 2:** Confirm exit. W1 is green.

---

### W2 — `parser.moof`

Stream owners: **agents β1, β2, β3** (subdivide). Effort: ~6–8h total. Risk: medium (largest stream; Form-tree equivalence is fiddly).

#### Task W2.1: Inventory `reader.rs` semantics

**Files:** none yet (research task — single agent does this before β1/β2/β3 split)

- [ ] **Step 1:** Read `crates/substrate/src/reader.rs` end to end.
- [ ] **Step 2:** Read `lib/parser/` (does not exist yet) and `docs/syntax/*` to confirm moof's surface syntax.
- [ ] **Step 3:** Enumerate every syntactic form rust reader produces:
  - atoms: `Int`, `Float`, `Bool`, `Nil`, `Char`, `Str`, `Sym`
  - cons-lists: `(...)`
  - send syntax: `[recv msg: arg ...]`
  - quote: `'foo` → `(quote foo)`
  - quasiquote: `` `foo `` → `(quasiquote foo)`
  - unquote: `,foo` → `(unquote foo)`
  - unquote-splice: `,@foo` → `(unquote-splice foo)`
  - vector literal: `#[...]`
  - char literal: `#\char`
  - bool literals: `#true`, `#false`
  - send-cascade: `[a foo ; bar]`
  - FormLoc meta on every node
- [ ] **Step 4:** For each, record the AST shape produced (exact cons-tree).
- [ ] **Step 5:** Save inventory at `tmp/w2-syntax-inventory.md`.

#### Task W2.2: Form-tree differ tool

**Files:**
- Create: `crates/substrate/src/bin/form_differ.rs`
- Modify: `crates/substrate/Cargo.toml` (add bin)

- [ ] **Step 1:** Add bin entry:

```toml
[[bin]]
name = "form-differ"
path = "src/bin/form_differ.rs"
```

- [ ] **Step 2:** Create `crates/substrate/src/bin/form_differ.rs`. Reads source from argv[1]; parses via rust reader → tree_a; parses via moof reader (`[Parser parse: src]`) → tree_b; structural-equal check (insertion-order-aware, including `:source-loc` meta).

Skeleton:

```rust
use moof::{World, Form};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path: std::path::PathBuf = std::env::args().nth(1).expect("usage").into();
    let src = std::fs::read_to_string(&path)?;

    let mut world = World::new_for_compile_diff();

    // rust reader
    world.set_use_moof_reader(false);
    let tree_rust = world.read_one(&src)?;

    // moof reader
    world.set_use_moof_reader(true);
    let tree_moof = world.read_one(&src)?;

    if forms_structurally_equal(&world, tree_rust, tree_moof) {
        println!("MATCH");
        return Ok(());
    }

    println!("DIFF");
    println!("  rust: {}", world.form_to_string(tree_rust));
    println!("  moof: {}", world.form_to_string(tree_moof));
    std::process::exit(1);
}

// (helper that recursively compares Form heads + slots + meta)
fn forms_structurally_equal(world: &World, a: Form, b: Form) -> bool { /* … */ }
```

- [ ] **Step 3:** Build + commit:

```bash
cargo build -p moof --bin form-differ
git add crates/substrate/src/bin/form_differ.rs crates/substrate/Cargo.toml
git commit -m "tooling: form-differ — diff rust vs moof reader output"
```

#### Task W2.3 (β1): `lib/parser/00-lexer.moof`

**Files:**
- Create: `lib/parser/00-lexer.moof`

The lexer produces a list of tokens. Each token is a cons `(type . value)` plus a position int (use a 3-cons `(type value pos)` for now). Token types: `'lparen`, `'rparen`, `'lbracket`, `'rbracket`, `'lbrace`, `'rbrace`, `'quote`, `'quasiquote`, `'unquote`, `'unquote-splice`, `'int`, `'float`, `'char`, `'string`, `'sym`, `'true`, `'false`, `'nil`, `'colon-sym` (for keyword args ending in `:`), `'cascade` (for `;`), `'hash-vec` (for `#[`).

- [ ] **Step 1:** Create file with header:

```moof
;; lib/parser/00-lexer.moof — char-stream → token-stream.
;;
;; tokens are cons cells: (type value position).
;; loaded by main.moof BEFORE compiler/*.moof.
;;
;; written in the minimal-subset moof: raw setHandler!, no macros,
;; no quasiquote, no defmethod sugar, no pattern-match params.
;; see docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md §4.

(def Lexer [Object new])
```

- [ ] **Step 2:** Add the entry point — `[Lexer tokenize: src]` returns a list of tokens.

```moof
(setHandler! Lexer 'tokenize:
  (fn (src)
    ;; src is a String. iterate by index.
    (let ((len [src length])
          (i 0)
          (tokens '()))
      ;; loop body — append tokens, advance i.
      ;; use a recursive helper since we don't have while.
      [self tokenLoop: src len: len i: i acc: tokens])))
```

- [ ] **Step 3:** Add `tokenLoop:len:i:acc:` which dispatches based on the current char:
  - whitespace → skip
  - `;` to end-of-line → comment, skip
  - `(`, `)`, `[`, `]`, `{`, `}`, `'`, `` ` ``, `,`, `@` → punctuation token
  - digit or `-digit` → `readNumber:`
  - `"` → `readString:`
  - `#` → `readHash:` (handles `#\`, `#true`, `#false`, `#[`)
  - identifier-start → `readSymbol:`
  - else → raise error

(Write each `read*:` helper as its own `setHandler!`. ~40 lines each.)

- [ ] **Step 4:** Add `[Lexer charAt:str index:]`, `[Lexer isWhitespace:c]`, `[Lexer isDigit:c]`, `[Lexer isIdentStart:c]` predicates.

- [ ] **Step 5:** Write a smoke test in moof (run via `moof '[Lexer tokenize: "(+ 1 2)"]'` once the runtime supports it; for now compile + manually inspect under the rust runtime by adding a debug print).

- [ ] **Step 6:** Verify by tokenizing every corpus file from W1:

```bash
for f in tests/byte-corpus/*.moof; do
  cargo run -q -p moof -- "[Lexer tokenize: [$transporter slurp: \"$f\"]]"
done
```

(Adjust to use whatever moof primitive reads file → string. If none, add `[Object slurp:path]` as a temporary helper.)

- [ ] **Step 7:** Commit:

```bash
git add lib/parser/00-lexer.moof
git commit -m "parser.moof: lexer (W2 β1)"
```

#### Task W2.4 (β2): `lib/parser/01-tokens.moof`

**Files:**
- Create: `lib/parser/01-tokens.moof`

Token utility singleton — predicates and FormLoc construction. Used by parser.

- [ ] **Step 1:** Create file:

```moof
;; lib/parser/01-tokens.moof — token-stream utilities + FormLoc.
;;
;; depends on: 00-lexer.moof.

(def Tokens [Object new])

;; tokens are (type value position). these are field accessors.
(setHandler! Tokens 'typeOf:    (fn (t) [t car]))
(setHandler! Tokens 'valueOf:   (fn (t) [[t cdr] car]))
(setHandler! Tokens 'posOf:     (fn (t) [[[t cdr] cdr] car]))

;; constructor
(setHandler! Tokens 'make:value:pos:
  (fn (type value pos)
    (cons type (cons value (cons pos '())))))
```

- [ ] **Step 2:** Add FormLoc construction. Matches rust `FormLoc { file: Sym, byte_offset: Int, line: Int, col: Int }`:

```moof
(setHandler! Tokens 'formLoc:line:col:
  (fn (file line col)
    ;; cons-based record: (file line col).
    (cons file (cons line (cons col '())))))
```

- [ ] **Step 3:** Add helpers for `tokens` cars/cdrs/null-checks used by the parser. Smoke: instantiate a token, read back its fields, assert equal.

- [ ] **Step 4:** Commit:

```bash
git add lib/parser/01-tokens.moof
git commit -m "parser.moof: token utils + FormLoc (W2 β2)"
```

#### Task W2.5 (β2 cont'd): `lib/parser/02-parser.moof`

**Files:**
- Create: `lib/parser/02-parser.moof`

Recursive-descent. The big one — ~400 LoC of moof.

- [ ] **Step 1:** Create file with header + singleton:

```moof
;; lib/parser/02-parser.moof — token-stream → Form tree.
;;
;; depends on: 00-lexer.moof, 01-tokens.moof.
;;
;; recursive-descent. one method per syntactic category.
;; minimal-subset; no macros, no quasiquote, raw setHandler!.

(def ParserCore [Object new])
```

- [ ] **Step 2:** Add `[ParserCore parseAll: tokens]` — returns a list of top-level Forms. Drives the loop calling `parseOne:` until tokens exhausted.

- [ ] **Step 3:** Add `parseOne:`. Dispatches on token type:
  - `'lparen` → `parseList:` (consume `(`, recursively read until `)`)
  - `'lbracket` → `parseSend:` (consume `[`, parse receiver, then `(:keyword arg)+`, then `]`; desugar to `(__send__ recv 'sel arg ...)`)
  - `'quote` → `parseQuoted:`
  - `'quasiquote`, `'unquote`, `'unquote-splice` → analogous
  - `'int` → `Int` Form
  - `'float` → `Float` Form
  - `'char` → `Char` Form
  - `'string` → `String` Form
  - `'sym` → `Sym` Form (interned)
  - `'true`/`'false`/`'nil` → atom literals

- [ ] **Step 4:** Each handler returns `(form . remaining-tokens)`. Caller threads remaining-tokens.

- [ ] **Step 5:** FormLoc: every constructed Form gets `:source-loc` attached via `[form setMeta: 'source-loc to: [Tokens formLoc: ...]]`.

- [ ] **Step 6:** Smoke (manual; we don't have moof-zig executing yet): under the rust runtime, run a corpus file's source through `[ParserCore parseAll: [Lexer tokenize: src]]` and verify the output Form prints identically to the rust reader.

- [ ] **Step 7:** Commit:

```bash
git add lib/parser/02-parser.moof
git commit -m "parser.moof: recursive-descent parser (W2 β2)"
```

#### Task W2.6 (β3): `lib/parser/03-bootstrap.moof`

**Files:**
- Create: `lib/parser/03-bootstrap.moof`

The Parser singleton + flip.

- [ ] **Step 1:** Create file:

```moof
;; lib/parser/03-bootstrap.moof — Parser singleton + [$reader useMoof] flip.
;;
;; depends on: 00-lexer.moof, 01-tokens.moof, 02-parser.moof.
;; after this file loads + the flip fires, every subsequent parse
;; (including all of early/*, stdlib/*, mcos.moof, REPL input)
;; routes through [Parser parse: src].

(def Parser [Object new])

(setHandler! Parser 'parse:
  (fn (src)
    (let ((tokens [Lexer tokenize: src]))
      [ParserCore parseAll: tokens])))

(setHandler! Parser 'parseOne:
  (fn (src)
    ;; convenience: parse a single top-level form.
    (let ((tokens [Lexer tokenize: src]))
      [[ParserCore parseAll: tokens] car])))

;; flip!
[$reader useMoof]
```

- [ ] **Step 2:** Commit:

```bash
git add lib/parser/03-bootstrap.moof
git commit -m "parser.moof: Parser singleton + useMoof flip (W2 β3)"
```

#### Task W2.7 (β3 cont'd): wire into `lib/main.moof`

**Files:**
- Modify: `lib/main.moof`

- [ ] **Step 1:** Edit `lib/main.moof`. Insert parser load **before** compiler load:

```moof
;; phase 0: the moof parser — compiled by the rust seed (minimal subset).
[$transporter load: "parser/00-lexer.moof"]
[$transporter load: "parser/01-tokens.moof"]
[$transporter load: "parser/02-parser.moof"]
[$transporter load: "parser/03-bootstrap.moof"]

;; flip already fires inside 03-bootstrap.moof — every subsequent
;; parse routes through Parser.

;; phase 1: the moof compiler — compiled by the moof parser now.
[$transporter load: "compiler/00-helpers.moof"]
...
```

- [ ] **Step 2:** Verify it builds: `cargo run -p moof --bin moof -- export-v4 --output /tmp/system.vat`. Expected: clean build; possibly some parser bugs surface that didn't with corpus tests.

- [ ] **Step 3:** If errors surface, capture them, fix the parser, retry.

- [ ] **Step 4:** Commit:

```bash
git add lib/main.moof
git commit -m "lib/main.moof: load parser before compiler (W2 β3)"
```

#### Task W2.8: Exit-criterion smoke — Form-tree differ

- [ ] **Step 1:** For each corpus file from W1, run form-differ. All must MATCH:

```bash
for f in tests/byte-corpus/*.moof; do
  result=$(cargo run -q -p moof --bin form-differ -- "$f")
  if [[ "$result" != "MATCH" ]]; then echo "FAIL: $f"; exit 1; fi
done
```

- [ ] **Step 2:** Run form-differ on EVERY file in `lib/`. All must MATCH. This catches edge cases the corpus didn't:

```bash
find lib -name "*.moof" | while read f; do
  result=$(cargo run -q -p moof --bin form-differ -- "$f")
  if [[ "$result" != "MATCH" ]]; then echo "FAIL: $f"; exit 1; fi
done
echo "W2 done."
```

- [ ] **Step 3:** Confirm exit. W2 is green.

---

### W3 — `moof-seed` minimization

Stream owner: **agent γ**. Effort: ~4–5h. Risk: low–medium.

**OCaml environment:** ocaml-seed lives in `opam switch wasm-mco`. Before any `dune` command:

```bash
eval $(opam env --switch=wasm-mco)
```

#### Task W3.1: Inventory + V4 audit

- [ ] **Step 1:** Read `crates/ocaml-seed/src/{ast,reader,opcodes,bytecode,compiler,image}.ml` end to end.
- [ ] **Step 2:** Read V4 spec §3 (opcode bytes) + §4 (operand layout) + §10 (image format).
- [ ] **Step 3:** Identify divergences between current ocaml-seed and V4 spec. Save at `tmp/w3-divergences.md`.
- [ ] **Step 4:** Identify code paths to delete (anything supporting macros, quasiquote, defmethod, send-cascade, pattern-match params, defproto, cond/when). Save at `tmp/w3-deletions.md`.

#### Task W3.2: Strip parser

**Files:**
- Modify: `crates/ocaml-seed/src/reader.ml`

- [ ] **Step 1:** Delete handlers for:
  - `` ` `` (quasiquote)
  - `,` (unquote)
  - `,@` (unquote-splice)
  - `#[` (vector literals)
  - `;` (send-cascade) — treat `;` as comment-to-EOL only
- [ ] **Step 2:** Retain:
  - `(...)`, `[...]`, `'foo`, atoms, `#\char`, `#true`, `#false`, `nil`
  - basic string escapes `\n \t \\ \" \0 \x`
- [ ] **Step 3:** Rebuild:

```bash
eval $(opam env --switch=wasm-mco)
dune build --root crates/ocaml-seed
```

Expected: clean build. If any module references the deleted handlers, delete those references too.

- [ ] **Step 4:** Commit:

```bash
git add crates/ocaml-seed/src/reader.ml
git commit -m "moof-seed: strip reader to minimal subset (W3)"
```

#### Task W3.3: Strip compiler

**Files:**
- Modify: `crates/ocaml-seed/src/compiler.ml`

- [ ] **Step 1:** Delete code paths for:
  - macro expansion (any `expand_macro` or similar)
  - `defmethod` / `defproto` / `defmacro` special-form handlers
  - pattern-match parameter destructuring in `fn`
  - `cond` / `when` / `match` form handlers
- [ ] **Step 2:** Retain dispatch for exactly: `def`, `set!`, `if`, `fn`, `do`, `let`, `quote`, and the catch-all "treat as call" branch.
- [ ] **Step 3:** Rebuild:

```bash
dune build --root crates/ocaml-seed
```

- [ ] **Step 4:** Commit:

```bash
git add crates/ocaml-seed/src/compiler.ml
git commit -m "moof-seed: strip compiler to 7 special forms (W3)"
```

#### Task W3.4: V4 byte emission audit

**Files:**
- Modify: `crates/ocaml-seed/src/{opcodes,bytecode}.ml`

- [ ] **Step 1:** For each opcode in `opcodes.ml`, confirm its byte tag matches V4 spec §3 exactly. Spec lists 24 opcodes; record tag values.
- [ ] **Step 2:** For each operand in `bytecode.ml`, confirm big-endian fixed-width encoding per spec §4:
  - SymId: u32 big-endian
  - Chunk const idx: u16 big-endian
  - IC idx: u16 big-endian
  - Jump offset: i16 big-endian (signed)
  - argc: u8
- [ ] **Step 3:** Where ocaml-seed diverges, edit to match spec.
- [ ] **Step 4:** Rebuild. Commit:

```bash
git add crates/ocaml-seed/src/opcodes.ml crates/ocaml-seed/src/bytecode.ml
git commit -m "moof-seed: V4 byte emission audit — match spec §3/§4 (W3)"
```

#### Task W3.5: `build-seed` subcommand

**Files:**
- Modify: `crates/ocaml-seed/bin/seed.ml`

The new `moof-seed build-seed --root lib/ --output seed.vat` command:
1. Reads `lib/main.moof`.
2. Statically resolves `[$transporter load: "..."]` calls — but ONLY for paths under `parser/`, `compiler/`, and `main.moof` itself. Everything else (`early/*`, `stdlib/*`, `mcos`) is NOT loaded; moof runtime handles those later.
3. Parses each file with the stripped reader.
4. Compiles each form with the stripped compiler.
5. Allocates Parser + Compiler singletons in a fresh World (matching rust's `new_world` shape minimally).
6. Serializes via `image.ml` to seed.vat.

- [ ] **Step 1:** Add command dispatch in `seed.ml`:

```ocaml
let () =
  let argv = Sys.argv in
  if Array.length argv < 2 then (prerr_endline "usage"; exit 1);
  match argv.(1) with
  | "compile" -> Compile_cmd.run (Array.sub argv 2 (Array.length argv - 2))
  | "build-seed" -> Build_seed_cmd.run (Array.sub argv 2 (Array.length argv - 2))
  | _ -> prerr_endline "unknown subcommand"; exit 1
```

- [ ] **Step 2:** Create `crates/ocaml-seed/bin/build_seed_cmd.ml` (and add to `bin/dune`). Implements the 6-step pipeline above.

- [ ] **Step 3:** Build:

```bash
dune build --root crates/ocaml-seed
```

- [ ] **Step 4:** Smoke: build a seed.vat:

```bash
dune exec --root crates/ocaml-seed bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
ls -la /tmp/seed.vat
```

Expected: file exists, ~50-500 KB (much smaller than full system.vat). Header bytes start with `MVAT` magic + version 4.

- [ ] **Step 5:** Reproducibility check (determinism law D5):

```bash
dune exec --root crates/ocaml-seed bin/seed.exe -- build-seed --root lib/ --output /tmp/seed-a.vat
dune exec --root crates/ocaml-seed bin/seed.exe -- build-seed --root lib/ --output /tmp/seed-b.vat
diff /tmp/seed-a.vat /tmp/seed-b.vat
```

Expected: no diff.

- [ ] **Step 6:** Commit:

```bash
git add crates/ocaml-seed/
git commit -m "moof-seed: build-seed command — produces seed.vat from minimal subset (W3)"
```

#### Task W3.6: Exit-criterion

- [ ] **Step 1:** Confirm: ocaml-seed builds clean, produces deterministic seed.vat, contains exactly Parser + Compiler + transporter chunks (no stdlib).

- [ ] **Step 2:** W3 is green.

---

### W4 — `moof` (zig) proof-of-life + `:serialize-world`

Stream owner: **agent δ**. Effort: ~4–5h. Risk: HIGH (bug-rich; unknown unknowns).

#### Task W4.1: Read state + plan

- [ ] **Step 1:** Read `crates/zig-substrate/src/{main,vm,intrinsics,image,heap}.zig` end to end.
- [ ] **Step 2:** Read V4 spec §2 + §3 (ops) and §10 (image format).
- [ ] **Step 3:** Identify what's missing for `vm.runTop`:
  - frame allocation
  - PC threading through chunks
  - dispatch handlers per opcode
  - native binding lookup for sends
- [ ] **Step 4:** Save analysis at `tmp/w4-analysis.md`.

#### Task W4.2: Add `exec` subcommand

**Files:**
- Modify: `crates/zig-substrate/src/main.zig`

- [ ] **Step 1:** Add CLI dispatch for `moof-zig exec <vat-file> <chunk-id>`:

```zig
} else if (std.mem.eql(u8, command, "exec")) {
    const vat_path = args.next() orelse return error.MissingArg;
    const chunk_id_str = args.next() orelse return error.MissingArg;
    const chunk_id = try std.fmt.parseInt(u32, chunk_id_str, 10);
    try cmd_exec(allocator, vat_path, chunk_id);
}
```

- [ ] **Step 2:** Implement `cmd_exec`:

```zig
fn cmd_exec(allocator: std.mem.Allocator, vat_path: []const u8, chunk_id: u32) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    const bytes = try std.fs.cwd().readFileAlloc(allocator, vat_path, 100 * 1024 * 1024);
    defer allocator.free(bytes);
    try image.loadVatImage(&world, bytes);

    const chunk_fid = world.fromU32(chunk_id);
    var vm_inst = vm.Vm.init(allocator);
    defer vm_inst.deinit();
    const result = try vm_inst.runTop(&world, chunk_fid);
    std.debug.print("=> {}\n", .{result});
}
```

- [ ] **Step 3:** Build:

```bash
cd crates/zig-substrate && zig build && cd ..
```

- [ ] **Step 4:** Smoke (will likely fail — that's W4.3):

```bash
./crates/zig-substrate/zig-out/bin/moof-zig exec /tmp/system.vat 42
```

(Use whichever chunk-id corresponds to a known-trivial chunk; find one by inspecting the system.vat's chunk table.)

- [ ] **Step 5:** Commit (skeleton even if exec fails):

```bash
git add crates/zig-substrate/src/main.zig
git commit -m "zig-substrate: exec subcommand skeleton (W4)"
```

#### Task W4.3: Fix exec bugs (iterate)

This is the bug-hunt task. Expect to iterate.

- [ ] **Step 1:** Construct a known-trivial chunk by hand: `LoadConst 0 (where const[0] = Int 3); Return`. Save bytes to `/tmp/trivial.bc`.
- [ ] **Step 2:** Build a tiny vat-image containing just this chunk; load via moof-zig; exec it. Expected output: `=> Int(3)`.
- [ ] **Step 3:** Each bug surfaces: chunk side-tables not aligned, IC slot init, sym alignment, dispatch table missing op. Fix one at a time. Commit each fix:

```bash
git add crates/zig-substrate/src/vm.zig
git commit -m "zig-substrate: fix <specific bug> (W4)"
```

- [ ] **Step 4:** Once trivial works, escalate: run a chunk that does `LoadConst 1; LoadConst 2; Send :+: argc=1 ic=0; Return`. Expected: `=> Int(3)` via real native dispatch.

- [ ] **Step 5:** Once that works, run a chunk from the actual system.vat — pick one that should be deterministic (`[Object new]` for example). Verify.

#### Task W4.4: `:serialize-world` intrinsic

**Files:**
- Modify: `crates/zig-substrate/src/intrinsics.zig`
- Modify: `crates/zig-substrate/src/image.zig`

- [ ] **Step 1:** In `image.zig`, add `fn serializeVat(world: *World, writer: anytype) !void`. Mirrors the rust `v4_export` byte layout exactly (V4 spec §10.3).

The layout per §10.3:
- Header: magic `"MVAT"` (4 bytes) + version u16 BE = `0x0004` + flags u16 + section count u16
- SymTableSection
- FormSection
- ChunkSection
- NativeRefsSection
- McoBindingsSection (can be empty for now)
- FarRefsSection (can be empty for now)
- Footer: 32-byte blake3 hash of preceding bytes (or zeros for stub)

- [ ] **Step 2:** Add native binding in `intrinsics.zig`:

```zig
fn serializeWorld(world: *World, self_: Value, args: []const Value) anyerror!Value {
    _ = self_;
    if (args.len < 1) return error.ArgcMismatch;
    const path_str = try valueToString(world, args[0]);
    defer world.allocator.free(path_str);
    var file = try std.fs.cwd().createFile(path_str, .{ .truncate = true });
    defer file.close();
    try image.serializeVat(world, file.writer());
    return Value.nil;
}
```

- [ ] **Step 3:** Register the native under selector `:serializeTo:` on the `$here` form (so `[$here serializeTo: "/tmp/out.vat"]` from moof code dispatches to this native). Use the existing intrinsic-binding pattern from `intrinsics.zig`.

- [ ] **Step 4:** Add CLI subcommand `moof-zig serialize <input.vat> <output.vat>`:

```zig
} else if (std.mem.eql(u8, command, "serialize")) {
    const in = args.next() orelse return error.MissingArg;
    const out = args.next() orelse return error.MissingArg;
    try cmd_serialize(allocator, in, out);
}
```

`cmd_serialize`: load input vat, immediately re-serialize to output. (Roundtrip — used to verify byte-equivalence with rust's export.)

- [ ] **Step 5:** Smoke roundtrip:

```bash
./crates/zig-substrate/zig-out/bin/moof-zig serialize /tmp/system.vat /tmp/system-rt.vat
diff /tmp/system.vat /tmp/system-rt.vat
```

Expected: identical OR documented divergence (footer hash, native re-binding order — fix if real bug).

- [ ] **Step 6:** Commit:

```bash
git add crates/zig-substrate/src/{intrinsics,image,main}.zig
git commit -m "zig-substrate: :serialize-world intrinsic + serialize CLI (W4)"
```

#### Task W4.5: Exit-criterion

- [ ] **Step 1:** Smokes pass:
  - `moof-zig exec /tmp/system.vat <(1+2 chunk)>` → `Int(3)`
  - `moof-zig serialize /tmp/system.vat /tmp/system-rt.vat` → byte-identical (or only footer differs)
- [ ] **Step 2:** W4 is green.

---

## Stage 2 — Integration + Rust Deletion (Sequential)

Dispatches after W1+W2+W3+W4 all green.

### W5 — integration + rename + rust deletion

Stream owner: **agent ε**. Effort: ~3–4h. Risk: medium.

#### Task W5.1: End-to-end build cycle

- [ ] **Step 1:** Build seed.vat with ocaml-seed:

```bash
eval $(opam env --switch=wasm-mco)
dune exec --root crates/ocaml-seed bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
```

- [ ] **Step 2:** Load seed.vat in moof-zig, run main (transporter chain fills stdlib), serialize world:

```bash
./crates/zig-substrate/zig-out/bin/moof-zig run /tmp/seed.vat --serialize-to /tmp/system-polyglot.vat
```

`moof-zig run` is a new subcommand added in this task (W5.1). It boots a vat-image, runs its `main` chunk to completion (transporter loads early/*, stdlib/*, mcos), and (if `--serialize-to` given) calls `:serializeTo:` on `$here` at the end. Add to `main.zig` alongside `exec`.

- [ ] **Step 3:** Compare to rust-built system.vat:

```bash
cargo run -p moof --bin moof -- export-v4 --output /tmp/system-rust.vat
```

Hash compare:

```bash
shasum /tmp/system-polyglot.vat /tmp/system-rust.vat
```

- [ ] **Step 4:** If hashes differ, dispatch a fix-agent against the divergence (insertion order? FormId allocation order? sym order?).

- [ ] **Step 5:** Once functionally equivalent (REPL smokes pass on both), commit:

```bash
git add -- (any integration scripts)
git commit -m "integration: full polyglot bootstrap cycle works (W5)"
```

#### Task W5.2: Smoke against polyglot-built system.vat

- [ ] **Step 1:** REPL-level smokes against `/tmp/system-polyglot.vat`:
  - `[1 + 2]` → `3`
  - `(if #true 1 2)` → `1`
  - `(do (def x 42) x)` → `42`
  - `[Object new]` → non-nil
- [ ] **Step 2:** Each must return the expected result via `moof-zig run /tmp/system-polyglot.vat <(echo '<expr>')`.

- [ ] **Step 3:** If any fails, fix the underlying bug, rebuild, retry. Do NOT proceed to rename/deletion until smokes pass.

#### Task W5.3: Rename binaries

**Files:**
- Modify: `crates/zig-substrate/build.zig`
- Modify: `crates/substrate/Cargo.toml`
- Modify: `NEXT_SESSION.md`, `README.md`, any scripts that reference `moof-zig` or `moof`

- [ ] **Step 1:** In `crates/zig-substrate/build.zig`, change executable name from `moof-zig` to `moof`. Rebuild + smoke:

```bash
cd crates/zig-substrate && zig build && ls zig-out/bin/
```

Expected: `zig-out/bin/moof` exists.

- [ ] **Step 2:** In `crates/substrate/Cargo.toml`, rename `[[bin]]` from `moof` to `moof-rs`:

```toml
[[bin]]
name = "moof-rs"
path = "src/main.rs"
```

Rebuild + smoke:

```bash
cargo build -p moof --bin moof-rs
```

- [ ] **Step 3:** Update `NEXT_SESSION.md` references: `moof-zig` → `moof`, `moof` (rust) → `moof-rs`.

- [ ] **Step 4:** Commit:

```bash
git add crates/zig-substrate/build.zig crates/substrate/Cargo.toml NEXT_SESSION.md
git commit -m "binary: rename moof-zig→moof and moof(rust)→moof-rs (W5)"
```

#### Task W5.4: Delete rust runtime

**Files:**
- Delete: `crates/substrate/src/{reader.rs, compiler.rs, vm.rs, opcodes.rs, intrinsics.rs, nursery.rs, transporter.rs, ...}`

Identify deletable files: anything that implements the runtime. Keep: anything that mco-related crates depend on (audit by cargo dependency graph).

- [ ] **Step 1:** List runtime files to delete:

```bash
ls crates/substrate/src/*.rs
```

Audit each: is it runtime or shared? Save list at `tmp/w5-to-delete.txt`.

- [ ] **Step 2:** Delete in batch:

```bash
git rm crates/substrate/src/{reader,compiler,vm,opcodes,intrinsics,nursery,transporter}.rs
```

(Adjust file list per audit.)

- [ ] **Step 3:** Try to build:

```bash
cargo build --workspace
```

Expected: errors in any remaining substrate files that imported deleted modules. Fix by deleting their consumers OR by stubbing.

- [ ] **Step 4:** Iterate until `cargo build --workspace` succeeds (with rust runtime gone but mco utilities intact).

- [ ] **Step 5:** Commit:

```bash
git add -A
git commit -m "delete: rust runtime — substrate is now zig + ocaml-seed (W5)"
```

#### Task W5.5: Remove substrate from workspace (optional, depending on residual)

**Files:**
- Modify: root `Cargo.toml`

- [ ] **Step 1:** If `crates/substrate/` is now empty (only mco helpers remain), consider whether to keep it as a workspace member or merge its contents into `crates/abi-rust/`.

- [ ] **Step 2:** If keeping: trim `crates/substrate/Cargo.toml` dependencies (remove wasmtime, indexmap, etc., that the runtime needed).

- [ ] **Step 3:** Commit:

```bash
git add Cargo.toml crates/substrate/Cargo.toml
git commit -m "workspace: trim substrate to mco-only residual (W5)"
```

#### Task W5.6: Final exit smoke

- [ ] **Step 1:** Clean build from scratch:

```bash
cargo clean
cargo build --workspace
cd crates/zig-substrate && zig build && cd ../..
eval $(opam env --switch=wasm-mco)
dune build --root crates/ocaml-seed
```

All three must succeed.

- [ ] **Step 2:** Full build cycle:

```bash
dune exec --root crates/ocaml-seed bin/seed.exe -- build-seed --root lib/ --output /tmp/seed.vat
./crates/zig-substrate/zig-out/bin/moof run /tmp/seed.vat --serialize-to /tmp/system.vat
./crates/zig-substrate/zig-out/bin/moof run /tmp/system.vat -- '[1 + 2]'
```

Expected last command: prints `3`.

- [ ] **Step 3:** Update `NEXT_SESSION.md` with new state (mention self-host shipped, rust deleted, vats-V4 is next).

- [ ] **Step 4:** Final commit + push:

```bash
git add NEXT_SESSION.md
git commit -m "NEXT_SESSION: self-host shipped; rust deleted; vats-V4 next"
git push
```

#### Task W5.7: Save a `rust-fallback` branch (safety net)

- [ ] **Step 1:** Before any push, save the pre-deletion commit as a branch:

```bash
git branch rust-fallback HEAD~$N  # N = number of commits in W5
git push -u origin rust-fallback
```

This is the regret-window mitigation from spec §7 risk 7.

---

## Risks reminder (from spec §7)

1. V4 byte mismatch invisible → byte-differ (W1.2) catches it.
2. parser.moof source-loc tracking → enforced in W2.5 step 5.
3. Minimal-subset violations → linter implied by W2/W3 exits.
4. W4 unknown unknowns → highest fix-loop budget; start earliest.
5. System.vat byte-equivalence → W5.1 step 3 hash-compares.
6. ocaml-seed build-image stubbed → replaced by `build-seed` (W3.5).
7. Rust deletion regret → `rust-fallback` branch (W5.7).

---

## Exit Criteria (consolidated)

- [ ] W1: corpus diff 100% MATCH (W1.5)
- [ ] W2: form-diff 100% MATCH on `lib/**/*.moof` (W2.8)
- [ ] W3: seed.vat builds deterministically; only minimal subset (W3.6)
- [ ] W4: exec smokes pass; `:serialize-world` round-trips (W4.5)
- [ ] W5: polyglot system.vat functionally equivalent to rust system.vat; rust deleted; binary renamed to `moof` (W5.6)

---

## See Also

- `docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md` — design
- `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` — V4 ISA + image format
- `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md` — vats roadmap (V4 multi-vat container follows this plan)
- `NEXT_SESSION.md` — current state snapshot
- `docs/roadmap.md` — phase A-self-host context
