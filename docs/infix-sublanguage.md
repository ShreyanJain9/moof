# the infix sublanguage

moof has two souls: smalltalk (everything is a message, objects all the way down)
and haskell (types are contracts, effects are tracked, functions compose).
the s-expr syntax serves the smalltalk soul. this document explores what
serves the haskell soul — and how both can coexist.

## the tension

smalltalk: the OBJECT is the protagonist. `[list map: |x| [x * 2]]`.
the receiver is there, you're talking to it, the method name reads like english.

haskell: the FUNCTION is the protagonist. `map (*2) xs`.
you're composing transformations. the data flows through.

moof says both are true. objects respond to messages AND functions
are first-class composable values. the question is whether the
surface syntax can serve both modes of thought without forcing
you to pick one.

## what the sublanguage looks like

```haskell
-- arithmetic is infix with precedence
1 + 2 * 3            -- 7, not 9.  → [[2 * 3] + 1]... wait, [1 + [2 * 3]]
x * 2 + 1            -- → [[x * 2] + 1]

-- ranges are literals
1..5                  -- → [1 to: 5]
1..100 by 3           -- → [1 to: 100 by: 3]
'a'..'z'              -- → ['a' to: 'z']  (if Char exists)

-- comprehensions (haskell list comprehensions)
[x * 2 | x <- xs]                     -- → [xs map: |x| [x * 2]]
[x * 2 | x <- xs, x > 3]             -- → [[xs select: |x| [x > 3]] map: |x| [x * 2]]
[x + y | x <- xs, y <- ys]            -- → nested flatMap
[(x, y) | x <- xs, y <- ys, x /= y]  -- cartesian product with filter

-- function application is juxtaposition
f x                   -- → (f x) → [f call: x]
map f xs              -- → (map f xs)
not (x > 3)           -- → (not [x > 3])

-- composition
f . g                 -- → [f compose: g]
xs |> map (*2) |> sum -- → [[xs map: |x| [x * 2]] sum]  (pipe)

-- let/where
let x = 1 + 2
    y = x * 3
in x + y

-- or trailing where:
hypotenuse a b = sqrt (a^2 + b^2)
  where sqrt = |x| [x toFloat] . [_ sqrt]

-- pattern matching with guards
factorial 0 = 1
factorial n = n * factorial (n - 1)

abs x
  | x >= 0    = x
  | otherwise  = negate x

-- case expressions
case command of
  "quit" -> exit
  "help" -> showHelp
  _      -> print ("unknown: " ++ command)

-- type/protocol annotations (not checked yet, documentation)
sort :: Comparable a => [a] -> [a]
```

## how it maps to moof

every infix expression compiles to the same AST as moof proper.
the sublanguage is a second parser, not a second semantics.

```
sublanguage              moof s-expr                    bytecode
─────────────────────    ──────────────────             ────────
1 + 2                    [1 + 2]                        Send 1 '+ 2
x * 2 + 1               [[x * 2] + 1]                  Send (Send x '* 2) '+ 1
1..5                     [1 to: 5]                      Send 1 'to: 5
[x*2 | x <- xs]         [xs map: |x| [x * 2]]         Send xs 'map: (Closure ...)
f x                      (f x)                          Send f 'call: x
f . g                    [f compose: g]                  Send f 'compose: g
```

the key insight: moof's `[receiver selector: arg]` IS infix.
`[a + b]` is already `a + b` with brackets. the sublanguage
just drops the brackets and adds precedence.

## operator precedence

haskell-style, adapted for moof's message sends:

```
level   operators           associativity    moof equivalent
──────  ──────────────────  ──────────────   ──────────────────
9       . (compose)         right            [f compose: g]
8       ^ (power)           right            [a pow: b]
7       * / %               left             [a * b] [a / b] [a % b]
6       + - ++              left             [a + b] [a - b] [a ++ b]
5       .. (range)          none             [a to: b]
4       == /= < > <= >=     none             [a = b] [a < b] etc.
3       &&                  right            (and a b)
2       ||                  right            (or a b)
1       |> (pipe)           left             chained sends
0       $ (apply)           right            reduce parens
```

`$` is haskell's "apply" — it's a low-precedence function application
that replaces parens: `f $ g $ h x` = `f (g (h x))`.

`|>` is elixir's pipe — data flows left to right:
`xs |> map f |> filter g |> sum` = `[[[xs map: f] filter: g] sum]`

## the key design questions

### 1. where does infix live?

**option A: file-level mode**
`.moof` files are s-expr. `.moo` files are infix. both compile to
the same bytecode. the REPL defaults to one and can switch.

**option B: inline escape via `#{...}`**
inside s-expr moof, `#{...}` enters infix mode:
```moof
(def doubled #{[x * 2 | x <- my-list]})
(def r #{1..5})
```
inside infix mode, `$(...)` escapes back to s-expr:
```haskell
let r = 1..5
let handler = $(vau (args) $e (eval [args car] $e))
```

**option C: dual personality**
the WHOLE language supports both syntaxes everywhere. the parser
auto-detects based on context. risky — ambiguity.

leaning toward **A + B**: files pick a mode, and escape hatches
let you reach the other. like how haskell has Template Haskell
for metaprogramming that the normal syntax can't express.

### 2. how do keyword messages work in infix?

smalltalk's killer feature: `[obj moveTo: 100 by: 200]` reads like english.
how does this look in infix?

```haskell
-- option A: dot syntax for sends
obj.moveTo 100 200          -- positional? loses the keyword names
obj.moveTo(x: 100, y: 200)  -- named args? new syntax

-- option B: backtick for keyword messages (like haskell's infix functions)
obj `moveTo:by:` 100 200

-- option C: keep bracket syntax for keyword messages
[obj moveTo: 100 by: 200]   -- just use moof syntax when you need keywords
1 + 2                        -- infix for operators

-- option D: colon syntax
obj moveTo: 100 by: 200     -- colons as in smalltalk, no brackets
```

option C is the pragmatic answer: operators are infix (`+`, `*`, `..`),
keyword messages keep brackets. you're not forced into one mode.
this is what Self does — Self has infix arithmetic but keyword message syntax.

option D is more radical: drop brackets entirely, use periods to end
statements (smalltalk-style). `obj moveTo: 100 by: 200.` the period
resolves ambiguity about where the send ends.

### 3. how do comprehensions interact with the object model?

haskell comprehensions desugar to monadic operations:
```haskell
[x * 2 | x <- xs, x > 3]
-- desugars to:
xs >>= \x -> guard (x > 3) >> return (x * 2)
```

moof comprehensions could desugar to protocol sends:
```moof
[xs select: |x| [x > 3]] map: |x| [x * 2]]
```

but if we have a Monad protocol (or Flatmappable), comprehensions
could work for ANY conforming type — not just lists:

```haskell
-- Option monad
[x + y | x <- maybeA, y <- maybeB]
-- → [maybeA flatMap: |x| [maybeB map: |y| [x + y]]]

-- Promise monad
[x + y | x <- fetchA, y <- fetchB]
-- → [fetchA flatMap: |x| [fetchB map: |y| [x + y]]]

-- Table rows
[p.name | p <- people, p.age > 30]
-- → [[people select: |p| [p.age > 30]] map: |p| [p.name]]
```

the comprehension desugars to `flatMap:` and `map:` sends.
ANY object that conforms to Flatmappable gets comprehension syntax.
this is exactly haskell's do-notation / list comprehension generalization.

### 4. where do effects go?

moof's vision: effects are capabilities (object refs).
haskell's insight: effects should be visible in types.

the sublanguage could combine both:

```haskell
-- protocol annotations show effects
greet :: Console -> String -> ()
greet console name = console.println ("hello, " ++ name)

-- a pure function has no capability args
double :: Int -> Int
double x = x * 2

-- the type tells you: this needs Console to run
main :: Console -> ()
main console = do
  let name = "moof"
  greet console name
```

the `::` annotations aren't enforced by the compiler (yet), but they
document which capabilities a function needs. this is moof's
"effects are capabilities" made visible. haskell uses the IO monad;
moof uses capability passing. the annotations make it legible.

### 5. do-notation?

haskell's `do` notation desugars `>>=` chains:
```haskell
do x <- getLine
   y <- getLine
   putStrLn (x ++ y)
-- is sugar for:
getLine >>= \x -> getLine >>= \y -> putStrLn (x ++ y)
```

moof could have similar notation for promise chains / sequential effects:

```haskell
do x <- fetch "/api/name"
   y <- fetch ("/api/greeting/" ++ x)
   console.println y
-- desugars to:
[fetch "/api/name" then: |x|
  [fetch ["/api/greeting/" ++ x] then: |y|
    [console println: y]]]
```

or more generally, for any Flatmappable:
```haskell
do x <- action1
   y <- action2 x
   return (x + y)
-- desugars to:
[action1 flatMap: |x| [action2 x map: |y| [x + y]]]
```

this makes async code, option chaining, and collection operations
all use the same syntax. that's the haskell gift.

## the two-soul architecture

```
                    ┌──────────────────────┐
                    │   infix sublanguage   │  ← haskell soul
                    │   .moo files          │     precedence, comprehensions,
                    │   or #{...} escapes   │     pattern match, do-notation
                    ├──────────────────────┤
                    │   s-expr moof         │  ← smalltalk soul
                    │   .moof files         │     messages, objects, operatives,
                    │   or $(...) escapes   │     vau, live environment
                    ├──────────────────────┤
                    │   shared AST          │  ← cons cells, symbols, values
                    │   (code is data)      │     homoiconic in both syntaxes
                    ├──────────────────────┤
                    │   compiler + VM       │  ← one bytecode, one runtime
                    │   protocols, objects   │     everything is a send
                    └──────────────────────┘
```

the s-expr layer is the foundation. it's homoiconic — code is data,
operatives manipulate syntax, vau gives user code compiler power.
the infix sublanguage compiles to the same AST. it's NOT homoiconic
(you can't easily manipulate infix code as data), but it's more
readable for the 90% of code that's just computation.

the escape hatches connect them:
- `#{...}` enters infix from s-expr
- `$(...)` enters s-expr from infix

## what we'd need to build

### the infix parser
a pratt parser (operator precedence parsing) that produces moof
AST (cons cells). handles:
- binary operators with precedence
- unary prefix operators (-, not)
- function application by juxtaposition
- `..` range literals
- `[... | ...]` comprehensions
- `let`/`where`/`case`/`do` expressions
- `$` and `|>` for low-precedence application and piping
- indentation sensitivity (layout rule) or explicit delimiters

### the Flatmappable protocol
```
requires: flatMap:, map:, pure:
provides: comprehension desugaring, do-notation, sequence:, traverse:
```

any type that conforms gets comprehensions and do-notation.
List, Option, Promise, Table — all candidates.

### `..` as a send
the parser translates `a..b` to `[a to: b]`. since `to:` already
exists on Integer (returns a Range), this works today. extending it
to other types = define `to:` on their prototype.

### pattern matching integration
the infix parser can have richer pattern syntax:
```haskell
case point of
  { x: 0, y: 0 } -> "origin"
  { x: x, y: 0 } -> "on x-axis at " ++ show x
  { x: x, y: y } -> show x ++ ", " ++ show y
```
this compiles to the same `match` form as s-expr moof.

## open questions (not yet answered)

- **naming**: what's the sublanguage called? "moo"? "moof-h"? just "moof" with a different file extension?
- **layout rule**: significant whitespace (haskell-style) or `end` keywords (ruby-style)?
- **partial application**: `map (*2) xs` — haskell curries by default. moof doesn't. do we add auto-curry in infix mode? or use explicit `_` placeholders: `map (_ * 2) xs`?
- **type annotations**: `::` syntax — purely documentation, or eventually checked? the protocol system could support it.
- **do-notation scope**: just for promises/async? or general monadic (any Flatmappable)?
- **REPL default**: which mode does the REPL start in? can you switch mid-session?
- **interop story**: how does infix code call s-expr code and vice versa? (answer: they compile to the same AST, so it's seamless — the question is how imports/modules work)
- **the `#{}` syntax**: does the generic sugar form survive alongside this? or does the infix sublanguage subsume it?
- **what comes first**: we could implement just infix operators + `..` ranges as a small first step, then add comprehensions, then do-notation. incremental path.
