# next steps

what we've built, what's next, and in what order.

## where we are (april 2026)

the core language works. the object model is solid. protocols
compose. monadic do-notation desugars via vau (no compiler
hardcoding). the environment is a real object. quasiquote
splicing works. seven protocols are self-documenting and
queryable at runtime.

### what's done

**runtime + VM**
- frame-based register VM with TCO and fuel-based preemption
- NaN-boxed values (integers, floats, symbols, objects — 8 bytes each)
- HeapObject variants: General, Environment, Closure, Pair, Text, Table, Buffer
- environment-as-object (HeapObject::Environment with HashMap bindings)
- native handler system with unique symbol-per-registration
- LMDB persistence (save/load image on exit/startup)

**language**
- four kernel forms: vau, send, def, quote
- compiler stability analysis (rebound forms fall back to vau)
- quasiquote with unquote-splicing (`,@`)
- object literals with methods, parent cloning, @x sugar, do blocks
- pipe blocks `|x| expr`
- multi-line REPL input (bracket balancing)
- monadic do-notation (vau-based, backwards-compatible)
- vau rest params

**protocols + stdlib**
- defprotocol vau with runtime-queryable docs
- Comparable (7 methods from <)
- Numeric (5 methods from +, -, *, negate)
- Iterable (40 methods from each:)
- Indexable (7 methods from at: + length, includes Iterable)
- Callable (4 methods from call:)
- Showable (4 methods from describe)
- Flatmappable (3 methods from flatMap: + map:)
- Result type: Ok/Err prototypes inheriting from Result
- identity monad defaults on Object (then:, flatMap:, map:)
- Range as pure moof object literal
- extensive Integer, String, Cons, Table, Nil, Boolean methods

**vat infrastructure (partially done)**
- scheduler.rs: Vat struct, Scheduler, spawn, run_turn, run_all
- FarRef prototype with doesNotUnderstand: (transparent proxying)
- Promise prototype (pending state)
- heap.outbox for outgoing messages
- Act placeholder type (data description of an effect)

---

## what's next

### phase 1: the effect system (vats + Acts)

the design is in docs/effects-and-vats.md. the core claim:
the vat is the universal effect boundary. all effects are
cross-vat sends returning Acts. pure code has no vat refs.

**1a. basic vat wiring**
- `[Vat spawn: block]` — creates a vat, runs block, returns Act
- REPL integration: scheduler drains vat work after each eval
- cross-vat send returns Act (replaces Promise)
- Act resolution: when target vat completes, Act resolves
- proof of life: spawn a vat, send a message, get a result

**1b. Act as first-class effect type**
- Act replaces Promise everywhere
- Act conforms to Flatmappable (real implementation, not placeholder)
- do-notation chains Acts: `(do (x <- [vat <- msg]) [process x])`
- Act combinators: `[Act all: acts]`, `[Act race: acts]`
- Act is inspectable data: target, selector, args

**1c. capability vats**
- Console, Clock, Store as capability vats
- native code wrapped behind vat interfaces
- the REPL is a vat holding capability refs
- the init vat (rust runtime) spawns capability vats
- spawn with explicit capability grants

**1d. remove try/catch**
- errors propagate through Act/Result monadic chains
- `recover:` handler for explicit error handling
- remove TryCatch and Throw opcodes
- pure code uses Result values + match

### phase 2: the pure/impure split

once vats and Acts work, we can enforce the separation:

**2a. purity detection**
- the runtime knows which closures capture vat refs
- closures with no vat refs are provably pure
- pure closures can be memoized, parallelized, serialized

**2b. reactive recomputation**
- Observable protocol: slot mutation notifies watchers
- dependency tracking within a vat (which bindings read which)
- change an input → recompute downstream → re-render
- foundation for the notebook/canvas model

**2c. content-addressed computation**
- pure function + same inputs = same hash
- cache bytecode compilation results
- cache execution results for pure calls
- connects to the vision's content-addressed storage

### phase 3: the surface

the design work is in docs/ — these are the three surface
experiments. all compile to the same AST.

**3a. generic sugar interface (docs/sugar-interface.md)**
- `#{tag content...}` — one syntax form, extensible via moof
- relaxed parsing inside braces (| and , as literal symbols)
- handlers are moof functions that take AST, return AST
- enables range literals, comprehensions, regex, DSLs

**3b. infix sublanguage (docs/infix-sublanguage.md)**
- operator precedence parsing (pratt parser)
- `1..5` range literals, `[x * 2 | x <- xs]` comprehensions
- `.moo` files or `#{...}` escape from s-exprs
- haskell-style let/where/case/do
- serves the haskell soul of moof

**3c. tell layer (docs/tell-layer.md)**
- `tell the circle to hide.` — end-user scripting
- `set the color of the circle to red.`
- `every document where modified > yesterday.`
- ~15 grammar rules, rigid structure, english word order
- the command bar / voice input / agent interface

### phase 4: stdlib expansion

**4a. capabilities**
- Console: println:, readLine, print:, flush
- Clock: now, measure:, sleep:
- Random: next, nextIn:, shuffle:, seed:
- File: read:, write:, exists:, delete:
- Network: fetch:, listen:

**4b. data**
- JSON: parse:, stringify:
- Pattern matching: match with object destructuring
- Option type (Some/None) conforming to Flatmappable
- Stream type (lazy iteration via next handler)

**4c. testing**
- (test "name" body) form
- assertions: assert:, assertEqual:to:, assertError:
- test runner: (runTests), reports pass/fail
- tests are just vats with mock capabilities

### phase 5: the canvas

from the vision doc. the spatial browser where objects
render themselves.

- Renderable protocol: render:, bounds, position
- egui-based zoomable infinite canvas
- every object renders itself (Renderable conformance)
- direct manipulation: click, drag, inspect, modify
- the REPL lives in the canvas
- the tell layer is the command bar

### phase 6: the agent

an LLM in a vat with a membrane.

- the agent is a vat with controlled capabilities
- membrane intercepts all sends: log, allow, deny, transform
- the agent uses protocols to discover what objects can do
- [obj protocols], [Protocol docs] — the agent reads these
- tool use = message sends to capability objects

---

## implementation priorities

what to do first, and why:

1. **vats** — everything downstream depends on the effect
   system working. without vats, no Acts, no capabilities,
   no pure/impure split.

2. **sugar/infix** — makes moof pleasant to write. the
   current s-expr syntax is powerful but noisy. a surface
   language brings in users.

3. **stdlib** — testing framework first (we need tests),
   then capabilities (Console, Clock), then data (JSON,
   streams, Option).

4. **canvas** — the defining feature. moof without the
   canvas is a language. moof with the canvas is an
   environment.

5. **agent** — the killer app. but needs canvas + capabilities
   + membranes first.

---

## key files

```
src/
  vm.rs              frame-based VM, TCO, fuel
  runtime.rs         native handlers via native() helper
  scheduler.rs       Vat, Scheduler, Message (partially wired)
  heap.rs            Environment object, outbox, type_protos
  object.rs          HeapObject variants
  dispatch.rs        message send dispatch
  value.rs           NaN-boxed values
  store.rs           LMDB persistence
  lang/
    compiler.rs      AST → bytecode, stability analysis
    parser.rs        s-exprs, sends, object literals, blocks
    lexer.rs         tokenizer

lib/
  bootstrap.moof     kernel: if, fn, do, cons, eq, and, or, when, unless, defn, defmethod
  protocols.moof     protocol infrastructure: defprotocol, conform, conforms?
  comparable.moof    Comparable protocol
  numeric.moof       Numeric protocol + Integer methods
  iterable.moof      Iterable protocol + Cons conformance
  indexable.moof     Indexable protocol (includes Iterable)
  callable.moof      Callable protocol
  showable.moof      Showable protocol
  types.moof         type-specific methods (Object, Nil, Boolean, String, Cons, Table)
  error.moof         error handling extensions
  range.moof         Range type (pure moof object literal)
  act.moof           Flatmappable, Result (Ok/Err), monadic do, Act placeholder

docs/
  effects-and-vats.md    the unified effect model (design)
  infix-sublanguage.md   haskell-style surface syntax (design)
  sugar-interface.md     generic #{...} sugar form (design)
  tell-layer.md          end-user scripting layer (design)
```
