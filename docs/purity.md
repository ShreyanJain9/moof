# purity: the immutable objectspace

## the commitment

all values in moof are immutable. objects, tables, cons lists,
strings, integers, symbols — once created, they never change.
the only mutable state lives inside servers, behind vat
boundaries, mediated by message sends that return Acts.

this is not a convention. it's an architectural guarantee.
the runtime enforces it. the language has no mutation operators.
referential transparency is the default, not the exception.

## what referential transparency buys you

same expression → same value → always. this unlocks:

- **memoization.** pure function + same args = cached result.
- **parallelism.** no shared mutable state = no races.
- **replay.** record inputs, replay computation deterministically.
- **content-addressing.** same value = same hash. cache everything.
- **serialization.** immutable values are trivially serializable.
  no mutable references to worry about.
- **cross-vat passing.** immutable values can be copied freely
  across vat boundaries. no aliasing, no deep-clone surprises.
- **time-travel debugging.** record message history + state
  snapshots. replay any historical state.
- **equational reasoning.** you can substitute equals for equals
  in your head (and in the optimizer). code means what it says.

## the two worlds

```
PURE (referentially transparent)     STATEFUL (server-mediated)
────────────────────────────────     ──────────────────────────
object literals                      server slots (via update)
tables (persistent, immutable)       server state transitions
cons lists, strings, numbers         cross-vat message sends
let bindings (immutable)             capability IO (→ Act)
function application
protocol dispatch
pattern matching
```

everything on the left is pure. the runtime can prove it —
no Acts, no FarRefs, no vat boundary crossings. everything
on the right returns an Act or goes through a server.

the vat boundary is the purity boundary. inside a handler,
everything is pure computation. effects and state changes are
return values, not side effects.

## what goes away

### `:=` (rebinding)

gone. `let` binds once. function params bind once.

```moof
; old (dead)
(let ((x 1))
  (:= x 2)
  x)

; new — no rebinding. compute forward.
(let ((x 1)
      (y [x + 1]))
  y)
```

if you need accumulation, use recursion or fold:

```moof
; old
(let ((sum 0))
  [items each: |x| (:= sum [sum + x])]
  sum)

; new
[items fold: 0 with: |sum x| [sum + x]]
```

### mutable `at:put:` on tables

`at:put:` returns a NEW table. the original is unchanged.

```moof
(let ((t #[1 2 3])
      (t2 [t at: 0 put: 99]))
  t    ; → #[1 2 3]  (unchanged)
  t2)  ; → #[99 2 3] (new table, structural sharing)
```

same for map tables:

```moof
(let ((m #[name => "moof"  version => 2])
      (m2 [m at: 'version put: 3]))
  m    ; → #[name => "moof"  version => 2]
  m2)  ; → #[name => "moof"  version => 3]
```

under the hood, tables use persistent data structures (hash
array mapped tries) for structural sharing. creating a "new"
table with one element changed is O(log n), not O(n).

### mutable object slots

object literal slots are set at creation and never change.
there is no `slotAt:put:` that mutates in place.

```moof
(def p { x: 1 y: 2 })
; p is frozen. forever.
```

to "update" an object, create a new one:

```moof
(def p2 [p with: { x: 10 }])
; p  → { x: 1  y: 2 }
; p2 → { x: 10 y: 2 }
```

`with:` creates a new object that clones the parent's slots
with overrides. non-destructive. the old object is unchanged.

## what stays

### `def` at the REPL

```moof
(def x 1)
(def x 2)  ; re-defines x in the REPL env
```

the REPL is a server — it's a vat with mutable state. `def`
is a state transition on the REPL server's environment. this
is explicitly stateful and that's fine. the REPL is the one
place where interactive mutation is expected.

inside functions and handlers, `def` creates a binding in the
local environment. it doesn't mutate — it shadows.

### `let` bindings

immutable within scope:

```moof
(let ((x 1)
      (y [x + 1]))
  [x + y])  ; → 3
```

### recursion and higher-order functions

the replacement for loops + mutation:

```moof
; counting
(defn count-up (n max)
  (if [n >= max] n
    (count-up [n + 1] max)))

; accumulation
[items fold: 0 with: |acc x| [acc + x]]

; transformation
[items map: |x| [x * 2]]

; filtering
[items filter: |x| [x > 0]]
```

## servers: the state boundary

servers are the ONLY place where state changes. a server is an
object in its own vat. handlers are pure functions that return
values describing what should happen:

```moof
(defserver Counter (console)
  "A counter with logging."

  count: 0

  [increment]
    (update { count: [@count + 1] }
            [@count + 1])

  [decrement]
    (update { count: [@count - 1] }
            [@count - 1])

  [get] @count

  [reset] (update { count: 0 })

  [log]
    [console println: (str "count: " [@count describe])])
```

### handler return types

a handler returns one of four things:

**plain value** — query result, no state change.

```moof
[get] @count
; vat sends @count as reply. state unchanged.
```

**Update** — state transition, optional reply.

```moof
[increment]
  (update { count: [@count + 1] }   ; delta
          [@count + 1])              ; reply
; vat applies delta, sends reply.

[reset] (update { count: 0 })
; vat applies delta, sends nil as reply.
```

`update` is a function that creates an Update value — just like
Act is a value describing an effect. the vat inspects the return
and acts accordingly.

**Act** — effect, no state change.

```moof
[log]
  [console println: @count]
; cross-vat send → Act. vat executes effect, sends result.
```

**Act resolving to Update** — effectful state transition.

```moof
[save] (do
  [store save: 'count value: @count]
  (update { saved: true }))
; effect first, then state change.
```

the `do` chains the effect (Act) and the update. the scheduler
runs the effect, then the vat applies the delta.

### the Update type

Update is a value, like Act:

```moof
Update
  delta:   { count: 1 }    ; slots to change
  reply:   1                ; value to send back to caller
  inspect  ...              ; introspectable
  describe ...
```

created by `(update delta)` or `(update delta reply)`. the vat
recognizes it by type and applies the delta. the delta is an
object literal — slot names and new values. only declared slots
can be updated (typo protection).

### snapshot semantics

inside a handler, `@slot` reads from an immutable snapshot of
the server's current state. the snapshot is taken when the
handler starts. the handler cannot observe its own state changes
— deltas are applied AFTER the handler returns.

this means:

```moof
[increment]
  (update { count: [@count + 1] }
          [@count + 1])
```

`@count` always refers to the pre-increment value within this
handler invocation. the handler is a pure function from
(current-state, message-args) → (Update | value | Act).

### state transitions are atomic

the delta from one handler is applied atomically before the
next message is processed. there's no intermediate state visible
to other messages:

```
state₀ = { count: 0 }
→ [increment] → handler returns Update{ count: 1 }
state₁ = { count: 1 }  (delta applied)
→ [increment] → handler returns Update{ count: 2 }
state₂ = { count: 2 }  (delta applied)
```

no interleaving. no partial updates. the vat's single-threaded
processing guarantees this.

## objects as values

with immutability, objects become true values. two objects
with the same slots and handlers are equal:

```moof
(let ((a { x: 1 y: 2 })
      (b { x: 1 y: 2 }))
  [a equal: b])  ; → true (structural equality)
```

this enables:
- objects as hash keys
- content-addressed storage of objects
- deduplication
- structural sharing in persistent data structures

### `with:` for non-destructive update

```moof
(def point { x: 0 y: 0 })
(def moved [point with: { x: 5 }])
; point → { x: 0 y: 0 }
; moved → { x: 5 y: 0 }
```

`with:` is the primary "update" mechanism for pure objects.
it creates a new object with the specified slots overridden.
unmentioned slots carry over. handlers carry over via
prototype delegation.

for deeply nested updates:

```moof
(def game { player: { pos: { x: 0 y: 0 } hp: 100 } })

; update nested structure
(def game2
  [game with: { player:
    [[@game player] with: { pos:
      [[[@game player] pos] with: { x: 5 }] }] }])
```

this is verbose. a path-update helper improves ergonomics:

```moof
(def game2 [game update-in: '(player pos x) with: 5])
```

design of path-update syntax is future work but the
architecture supports it — objects are values, paths are
lists of symbols, updates create new objects.

## tables as persistent data structures

tables are moof's array and dictionary type. they must be
immutable to preserve referential transparency.

### implementation: hash array mapped tries (HAMT)

sequential tables (`#[1 2 3]`) and map tables
(`#[x => 1 y => 2]`) use persistent data structures
internally. "modifying" a table returns a new table that
shares structure with the original:

```moof
(def big-table [Range new: 1 to: 1000000])
(def modified [big-table at: 500000 put: 0])
; modified shares 99.999% of big-table's memory
; creation is O(log n), not O(n)
```

### table operations (all return new tables)

```moof
[t at: i put: v]        ; new table with element changed
[t append: v]           ; new table with element added at end
[t remove: i]           ; new table with element removed
[t concat: other]       ; new table combining both
[t map: f]              ; new table with f applied to each
[t filter: pred]        ; new table with matching elements
[t sort: cmp]           ; new table in sorted order
```

every operation returns a new table. no mutation. the original
is always unchanged.

## cross-vat value passing

immutability makes cross-vat copying trivial:

1. **immediate values** (int, float, bool, nil, symbol):
   bitwise copy. always safe.

2. **immutable heap objects** (string, cons, table, object):
   deep copy on first transfer, or content-addressed dedup.
   no aliasing concerns — the copy is indistinguishable from
   the original because neither can change.

3. **FarRefs**: copy the target coordinates
   (vat_id, object_id). the ref is a value pointing at a
   remote, the remote is a server.

future optimization: with content-addressing, cross-vat
"copying" becomes "verify you have the same hash, skip the
copy." immutability makes this sound.

## the REPL as a server

the REPL is a vat — already true. now it's also explicitly
a server. `def` is a state transition:

```moof
(def x 1)     ; update REPL state: bind x to 1
(def x 2)     ; update REPL state: rebind x to 2
x             ; query: return current binding of x
```

this is consistent. the REPL is the one place where
interactive re-definition is expected. it's the "top-level
server" that holds your working environment.

inside functions and let-bindings, everything is immutable.
the REPL's `def` mutability is a server behavior, not a
language feature.

## runtime implications

### what changes in the VM

- **remove `:=` opcode.** (SetLocal rebinding gone.)
- **remove mutable `slotAt:put:`.** the handler returns a new
  object. (or: keep the handler name, change semantics to
  return new object.)
- **add `with:` handler** on Object prototype. creates a new
  object with slot overrides.
- **add `update` function.** creates Update values for server
  handlers.
- **add Update type.** new heap object variant, or General
  with a specific prototype.
- **persistent table implementation.** replace Vec-based Table
  with HAMT or similar persistent structure.

### what changes in the scheduler

- **compute vats auto-close.** after result is copied, mark Dead.
- **server vats apply deltas.** when a handler returns Update,
  the scheduler applies the delta to the server object between
  message deliveries.
- **server vats stay alive.** until [stop] or supervision
  decision.

### what changes in the heap

- **HeapObject becomes immutable after allocation.** no more
  `slot_set` on General objects (except during construction).
- **`handler_set` still works** — handlers are set during object
  construction (by the compiler and bootstrap).
- **structural equality** for objects. two objects with the same
  slots, values, and handlers are equal.
- **content-addressed objects** (future). hash the structure,
  dedup identical objects.

## migration path

we don't have to do everything at once. the order:

1. **add `with:` on Object.** non-destructive slot update.
   code can start using it immediately.

2. **make `at:put:` non-destructive on tables.** returns new
   table. existing code that does `[t at: k put: v]` without
   using the return value breaks — but that code was wrong
   anyway (the "mutation" was unobservable).

3. **add Update type.** just a prototype with delta + reply
   slots. `(update delta reply)` creates one.

4. **implement server state application.** scheduler recognizes
   Update return values, applies deltas between messages.

5. **remove `:=` from the compiler.** flag as error. existing
   code must migrate to `let`, `fold`, `with:`, or servers.

6. **freeze object slots after construction.** `slot_set`
   errors after the object is fully constructed.

7. **persistent tables.** replace internal Vec with HAMT.
   this is a perf concern, not a semantics concern — the
   API doesn't change.

steps 1-4 are additive — nothing breaks. step 5 is the
breaking change. steps 6-7 are internal optimizations.

## examples

### before (imperative)

```moof
(def scores #[])
(defn add-score (name val)
  (:= scores [scores at: name put: val]))
(add-score 'alice 100)
(add-score 'bob 85)
scores  ; → #[alice => 100  bob => 85]
```

### after (pure + server)

```moof
; pure computation — no state
(let ((scores #[alice => 100  bob => 85])
      (updated [scores at: 'charlie put: 92]))
  [updated at: 'charlie])  ; → 92

; stateful service — server
(defserver Leaderboard ()
  scores: #[]

  [add: name score: val]
    (update { scores: [@scores at: name put: val] }
            val)

  [get: name]
    [@scores at: name]

  [top: n]
    [[@scores entries] sort: |a b| [[b value] > [a value]]]
    ; returns sorted list, no state change
)

(do (lb <- (Leaderboard {}))
    [lb add: 'alice score: 100]
    [lb add: 'bob score: 85]
    (top <- [lb top: 10])
    top)
```

### before (mutable object)

```moof
(def player { hp: 100 pos: { x: 0 y: 0 } })
(:= @hp [@hp - 10])
```

### after (immutable value)

```moof
(def player { hp: 100 pos: { x: 0 y: 0 } })
(def hurt-player [player with: { hp: [[@player hp] - 10] }])
; player unchanged. hurt-player is the new version.
```

## summary

1. **all values are immutable.** objects, tables, lists, strings.
   once created, never changed.
2. **no `:=`.** let binds once. functions take args. fold and
   recurse instead of loop and mutate.
3. **`with:` for object updates.** returns a new object.
   `at:put:` on tables returns a new table.
4. **servers are the state boundary.** the only place where
   "state changes" — via Update values returned from handlers.
5. **handlers are pure.** (state, message) → (Update | value | Act).
   the vat applies the Update between messages.
6. **referential transparency everywhere.** same inputs, same
   outputs, always. the runtime can prove it.
7. **cross-vat passing is trivial.** immutable values copy freely.
8. **the REPL is a server.** `def` is the one interactive
   mutation, explicitly stateful.
