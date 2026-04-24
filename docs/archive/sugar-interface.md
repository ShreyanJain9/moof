# Generic Sugar Interface: `#{tag content...}`

a single extensible syntax form for user-defined sugar.
range literals, comprehensions, regex, custom DSLs — all through one gateway.

## the one form

```moof
#{range 1 5}
#{for [x * 2] | x <- list, [x > 3]}
#{regex "\\d+"}
#{color red}
```

`#{...}` — hash + braces. first symbol inside is the **tag**.
rest is **content**, parsed in a relaxed mode where `|` and `,`
become literal marker symbols instead of their normal moof meaning
(block-start / unquote).

## pipeline

```
source → lexer → parser → expand_sugar → compiler → VM
                              ↑
                    calls sugar handlers (moof functions)
                    that take AST tokens, return AST
```

## parser behavior

`#{` triggers sugar mode:
1. advance past `{`
2. read first symbol → tag name
3. read remaining expressions until `}` in **relaxed mode**:
   - normal moof expression parsing (lists, sends, objects, strings, numbers)
   - `|` → literal symbol `|` (not block-start)
   - `,` → literal symbol `,` (not unquote)
   - `<-` → literal symbol `<-`
4. produce: `(%sugar tag-sym (content...))`

### parse examples

```
#{range 1 5}
→ (%sugar range (1 5))

#{for [x * 2] | x <- list, [x > 3]}
→ (%sugar for ([x * 2] | x <- list , [x > 3]))

#{regex "\\d+"}
→ (%sugar regex ("\\d+"))
```

content must be valid moof tokens.
for raw content, use strings: `#{regex "\\d+"}` not `#{regex \d+}`.

## expansion pass

new function: `expand_sugar(vm, heap, ast) -> Result<Value>`

walks the AST tree depth-first. when it finds `(%sugar tag content)`:
1. look up `tag` in the environment
2. call the handler function with `content` as argument
3. replace the `(%sugar ...)` node with the return value
4. recursively expand the result (handlers can return more sugar)
5. depth limit to prevent infinite loops

runs between parsing and compilation — returned AST goes through
the compiler normally, so local variables, closures, everything resolves.

## sugar handlers

plain moof functions. take a list of AST tokens (with `|`, `,`, `<-`
as marker symbols). return moof AST using quasiquote.

### range

```moof
(def range (fn (tokens)
  (let ((start [tokens car])
        (end [[tokens cdr] car]))
    `(send ,start 'to: ,end))))

; #{range 1 5} → [1 to: 5]
; #{range 1 n} → [1 to: n] (n resolved by compiler later)
```

### for (comprehension)

```moof
(def for (fn (tokens)
  ; split tokens on '| → (output-section . binding-sections)
  ; split binding-sections on ', → individual clauses
  ; each clause: (var <- collection) or (filter-expr)
  ; construct: [[collection select: |var| filter] map: |var| output]
  ...))

; #{for [x * 2] | x <- list, [x > 3]}
; → [[list select: |x| [x > 3]] map: |x| [x * 2]]
```

### def-sugar convenience

```moof
(def-sugar range (start end)
  `(send ,start 'to: ,end))

; macro that generates the handler function with
; destructuring from the token list
```

### helper functions

```moof
; split a list on a marker symbol
(defn split-on (lst marker) ...)

; convert (a op b) into (send a 'op b)
(defn infix (a op b) `(send ,a ',op ,b))
```

## open questions

**naming collisions**: `#{for ...}` looks up `for` in the env.
if the user hasn't defined a `for` handler, it errors.
if something else is bound to `for`, it tries to call it.
- option A: accept this. users are responsible.
- option B: namespace: look up `sugar/for` or `sugar-for` instead.
- option C: dedicated Sugar table: `(def Sugar #[])`, handlers registered there.

**infix inside sugar**: `#{for x * 2 | ...}` — the `x * 2` parses as
three flat tokens, not a send. the handler must reconstruct `[x * 2]`.
- option A: handlers deal with it (use helper functions)
- option B: require brackets inside sugar: `#{for [x * 2] | ...}`
- option C: add mini infix parser for sugar content (bigger change)

**single-expression sugar**: should `#tag expr` (no braces) also work
for simple cases? `#range(1 5)` instead of `#{range 1 5}`.
- brace form is universal and consistent
- paren form is lighter for simple cases
- could support both

**comprehension complexity**: the `for` handler needs to parse bindings,
filters, and output expressions from a flat token list. non-trivial.
- implement in v1 or defer?
- could be a good proof-of-concept for the sugar system's power

**interaction with tell layer**: the tell layer (docs/tell-layer.md)
targets end-users. sugar targets programmers. do they share infrastructure?
- tell layer could be implemented AS a sugar handler: `#{tell ...}`
- or they stay separate (different parsing, different audience)

## implementation sketch

files to create/modify:
- `src/lang/parser.rs` — add `parse_sugar()`, handle `#{`
- `src/lang/sugar.rs` — NEW: `expand_sugar` tree walker
- `src/lang/mod.rs` — add `pub mod sugar;`
- `src/shell/repl.rs` — call `expand_sugar` in `eval_source`
- `src/lang/compiler.rs` — error on unexpanded `%sugar` (safety net)
- `lib/sugar.moof` — NEW: handlers, helpers, def-sugar
