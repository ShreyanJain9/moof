# MOOF: Moof's Open Objectspace Fabric
## Design & Philosophy Document

> *"clarus the dogcow lives again"*

---

## 0. the north star

MOOF is not a programming language. it is a **living computational environment** — a persistent, introspectable, AI-native objectspace where usage and programming are the same activity. the file is a lie. the filesystem is a 1969 compromise. MOOF replaces both with a unified fabric of objects that know what they are, remember what they were, and can explain themselves to humans and AI agents alike.

three governing principles underpin every design decision:

1. **turtles all the way down** — a tiny, fixed kernel of primitives. everything else, including the standard library, the IDE, the AI integration layer, and the persistence system, is written in MOOF and lives in the image.
2. **objects and messaging are the only abstraction** — not a layer on top of something else. the vm's single privileged operation is `send`. everything reduces to it.
3. **radical openness** — the image is a first-class participant in the broader computing ecosystem. FFI, MCP, HTTP, federation: MOOF objects speak every protocol, mediated by the same message-passing model.

---

## 1. the name

**MOOF: Moof's Open Objectspace Fabric** — recursive backronym, in the tradition of GNU. named after Clarus the dogcow, the beloved Apple mascot. "moof" was her sound. she was always half one thing and half another. so is this environment.

---

## 2. the kernel

the entire kernel consists of exactly these primitives, implemented in Rust. nothing else gets vm privilege. everything else is derived.

### 2.1 the six kernel forms

```
vau     ; operative constructor. THE primitive abstraction.
        ; (vau (params) $env body) — receives unevaluated args and the caller's environment
send    ; the vm's dispatch instruction. [obj selector args...] compiles to this.
def     ; bind a name in the current environment
quote   ; ' sugar. (quote x) → the symbol x, unevaluated
cons    ; construct a pair. the ast is cons cells all the way down
eq      ; identity comparison. the only equality at the kernel level
```

everything else — `lambda`, `if`, `let`, `cond`, `loop`, `match`, `object`, `do` — is derived from these in the image bootstrap.

### 2.2 wrap and lambda

`wrap` is derived immediately from `vau`:

```scheme
; wrap: takes an operative, returns one that evaluates its args first
(def wrap (vau (operative) $e
  (vau args $caller
    (send operative call:
      (map (lambda (a) (eval a $caller)) args)))))

; lambda is now just: vau + wrap
(def lambda (vau (params . body) $e
  (wrap (eval (cons vau (cons params body)) $e))))
```

### 2.3 why vau and not macros

macros require compiler privilege. vau does not. an operative receives its arguments *unevaluated* and the calling *environment* as a first-class value. this means:

- `if`, `define`, `lambda` are library functions, not special forms
- user code has identical expressive power to "compiler" code
- the evaluator is expressible in the language itself
- the reflective tower (see §7) falls out naturally

the cost is naive optimization difficulty. the answer is inline caching and operative specialization at the JIT level (see §9), not compiler privilege.

---

## 3. syntax

### 3.1 the three bracket species

MOOF has exactly three structural forms at the reader level:

| syntax | meaning | desugars to |
|---|---|---|
| `(f a b c)` | applicative call | `[f call: a b c]` |
| `[obj sel: a sel2: b]` | message send | vm `send` instruction |
| `{ x: 10 y: 20 }` | object literal | anonymous object with slots |

**applicative call is message send.** `(f a b c)` is syntactic sugar for `[f call: a b c]`. any object with a `call:` handler is callable with `()` syntax. there is no separate "function type" at the semantic level.

### 3.2 message send syntax

```scheme
[obj slot]              ; unary message
[obj negate]            ; unary message
[obj + 5]               ; binary message — selector is "+"
[obj at: k]             ; keyword message — selector is "at:"
[obj at: k put: v]      ; multi-keyword — selector is "at:put:"
[[obj foo] bar: baz]    ; chained sends
```

multi-keyword selectors are concatenated: `at:put:` is a single selector string. this is the smalltalk convention and it reads beautifully.

### 3.3 reader sugar

```scheme
'x              → (quote x)
`(a ,b ,@c)     → quasiquote / unquote / unquote-splicing
obj.slot        → [obj slot]       ; dot access, optional
{ :x [x * 2] } → anonymous block / closure object
```

### 3.4 blocks

`{ :x [x * 2] }` is an anonymous object with a `call:` handler. blocks are closures are objects. they capture their lexical environment. `{ expr }` is a zero-argument block. blocks are how `if`, `while`, `map:`, etc. take "deferred code" — they are not macros, they are first-class values.

```scheme
; if is just a message to a boolean object:
[condition ifTrue: { do-this } ifFalse: { do-that }]

; true and false are singletons in the object model:
; True >> ifTrue: block ifFalse: _ → [block call]
; False >> ifTrue: _ ifFalse: block → [block call]
```

### 3.5 the code-is-data invariant

the reader produces cons cells. the AST is cons lists. `(f a b)` reads as `(cons f (cons a (cons b nil)))`. `vau` receives this tree unevaluated. quasiquote manipulates it. the evaluator walks it. there is no separate "AST type" — it's just data. this is the lisp heritage, fully preserved.

---

## 4. the object model

### 4.1 the fundamental split: slots vs handlers

objects have **two distinct namespaces**:

```
object = {
  slots:    { name → value }          ; storage. private by default.
  handlers: { selector → operative }  ; behavior. the public interface.
}
```

this is the critical design choice distinguishing MOOF from Self. in Self, everything is a slot. in MOOF:

- **slots** are storage. they are accessed *only* via explicit slot messages (`slotAt:`, `slotAt:put:`). they are never inherited.
- **handlers** are behavior. they are the public message interface. they *are* inherited through prototype delegation. they are defined with `handle!`, not assigned with `=`.

```scheme
; defining a handler — behavior
(handle! my-point
  [distanceTo: other]
  (let ((dx [[my-point slotAt: #x] - [other slotAt: #x]])
        (dy [[my-point slotAt: #y] - [other slotAt: #y]]))
    [[dx * dx] + [dy * dy] sqrt]))

; accessing a slot — storage
[my-point slotAt: #x]          ; → 10
[my-point slotAt: #x put: 20]  ; mutate
```

the payoff: encapsulation is structural, not conventional. an agent or user can only reach what the handler table exposes.

### 4.2 prototype delegation

objects have a `parent` slot (a normal slot, not a handler). handler lookup walks the delegation chain:

1. look in receiver's handler table
2. if not found, recurse on `[receiver slotAt: #parent]`
3. if chain exhausted, fire `doesNotUnderstand:` on the receiver

`doesNotUnderstand:` is a handler like any other. proxy objects, remote objects, dsl builders, and dynamic dispatch all live here.

### 4.3 the object hierarchy

all primitive types are full members of the object model. there is no "primitive layer" separate from the object system. the vm has fast paths (unboxed arithmetic, inline caches) but the *semantic model* is always message dispatch:

```
Object          ; root of the delegation chain
  Nil           ; the nil singleton
  Boolean
    True        ; the true singleton
    False       ; the false singleton
  Magnitude
    Number
      Integer   ; arbitrary precision
      Float
      Ratio     ; exact rational, always
    Character
    Symbol      ; interned, identity-comparable
  Collection
    String      ; byte or char, protocol-unified
    Cons        ; the pair — foundation of lists and ASTs
    List        ; singly-linked, built on Cons
    Vector      ; contiguous, O(1) index
    HashMap
    HashSet
    Stream      ; lazy, potentially infinite
  Block         ; closure / anonymous object with call:
  Environment   ; first-class, [env eval: expr]
  Operative     ; produced by vau
  Vat           ; capability security domain
  Membrane      ; message interception wrapper
  Mirror        ; safe reflection handle
```

### 4.4 `call:` and the unified callable protocol

`(f a b c)` always desugars to `[f call: a b c]`. this means:

- lambdas: objects with a `call:` handler that evaluates args then runs body
- operatives (vau results): objects with a `call:` handler that receives raw args + env
- any user object with `call:`: callable with `()` syntax
- partial application, decorators, proxies: just objects with `call:`

there is no privileged "function" concept in the vm. `send` is the only primitive operation. `call:` is just the conventional selector for invocation.

### 4.5 protocols

selectors are namespaced through **protocol objects** to avoid global collision:

```scheme
(def Numeric (Protocol {
  name: "Numeric"
  requires: '(+ - * / negate abs)
  doc: "anything that supports arithmetic"
}))

; Integer conforms to Numeric
(conform! Integer Numeric)

; dispatch checks protocol conformance
; [myVec + otherVec] routes to Geometry/+ not Numeric/+
```

protocols are first-class objects. the MCP server can query "what protocols does this object conform to" as a reflection surface.

---

## 5. capability security

### 5.1 the axiom

**a reference is a capability. capabilities are unforgeable.** the only way to interact with an object is to hold a direct reference to it. there is no global lookup, no ambient namespace access, no way to conjure a reference from a name string.

### 5.2 vats

a **Vat** is a security domain. every object has a home vat.

- **same-vat sends**: synchronous, fast, no overhead
- **cross-vat sends**: asynchronous, go through an explicit channel, auditable

the vm enforces vat boundaries. you cannot synchronously call into another vat.

### 5.3 membranes

a **Membrane** wraps an object or vat and intercepts every message send:

```scheme
(def logged-obj
  (Membrane wrap: real-obj
    on-send: (lambda (sel args)
      [AuditLog record: sel args: args timestamp: [Clock now]]
      :proceed)          ; :proceed, :deny, or a replacement result
    on-receive: (lambda (result) result)))
```

membranes are the unified mechanism for: sandboxing AI agents, sandboxing user code, auditing, rate limiting, access logging. no special cases.

### 5.4 facets

a **Facet** is a restricted view of an object exposing only named selectors:

```scheme
; give an agent read-only filesystem access
(def agent-view
  [real-filesystem facet: '(read: list: stat: exists:)])
```

facets compose with membranes:

```scheme
(def safe-agent-view
  (Membrane wrap: agent-view
    on-send: (lambda (sel args)
      [RateLimiter check: sel])))
```

### 5.5 the AI security story

an AI agent lives in its own Vat. it receives Facets of image objects, wrapped in Membranes. it can do exactly what the facets allow. the ChangeLog records every action with author, timestamp, and old/new state. a human can revoke any facet at any time. this is the complete security story.

---

## 6. persistence

### 6.1 orthogonal persistence

things just survive. you never "save." the image is the truth. programs do not distinguish between memory and storage.

### 6.2 the four-layer stack

**layer 1 — write-ahead log (WAL)**
every slot mutation is logged before it happens. crash recovery is WAL replay from the last snapshot.

**layer 2 — content-addressed object store**
each snapshot is hashed (like a git object). provides:
- time travel: `[obj history]` is a lazy stream of past snapshots
- cheap diffing: `[obj diff: otherSnapshot]`
- structural sharing: unchanged subgraphs share storage

**layer 3 — per-object durability policy**
set via a slot on every object:
- `ephemeral` — in-memory only, gone on restart
- `durable` — WAL-protected (default)
- `versioned` — full history in the content store
- `synced` — mirrors to an external store (sqlite, s3, etc.)

**layer 4 — CRDTs for federation**
objects marked `shared` use CRDT semantics for merge. `Counter`, `GrowSet`, `ObservedRemoveSet`, `LastWriteWinsMap`. not a day-one concern but the slot model must not preclude it.

### 6.3 key persistence API

```scheme
[Image checkpoint]               ; manual snapshot
[Image rollback-to: timestamp]   ; restore
[obj history]                    ; lazy stream of past states
[obj diff: other]                ; changeset object
[ChangeLog on-change: obj do: h] ; live subscription
```

---

## 7. reflection

### 7.1 the three standard slots

every object inherits from `Object` and gets these handlers by default:

- `describe` — human/AI readable string. used by the Browser, the MCP server, and agents.
- `interface` — map of selector → `{args doc: returns:}`. the schema the MCP server exposes as tools.
- `source` — the live s-expression tree of each handler, stored in the image. not a filename. the actual code, queryable and modifiable.

### 7.2 Mirror

`[Mirror on: obj]` returns a safe reflective handle. read-only by default. used by the debugger, browser, and AI agents to introspect without side effects.

### 7.3 the reflective tower

because environments are first-class objects and `eval` is `[env eval: expr]`, the evaluator itself is reifiable. an operative that intercepts evaluation at any level can be installed. this is the 3-Lisp / Black tradition: the meta-level is just more objects. not a day-one implementation priority but must not be architecturally foreclosed.

---

## 8. AI integration

### 8.1 the image as MCP server

at startup, `MCPServer` walks the image and registers:
- every object with a non-nil `describe` as an MCP **resource**
- every object with a non-nil `interface` as a set of MCP **tools**
- the live object graph is the tool registry. no separate config.

an AI agent calling `tools/list` gets the live objectspace. calling a tool is a membrane'd message send into the image.

### 8.2 Agent objects

```scheme
(def my-agent
  (Agent spawn: :claude-sonnet
         with-capabilities: (list
           [filesystem facet: '(read: list:)]
           [Namespace current facet: '(lookup: register:)])))
```

the agent has its own Vat. its memory is objects in that Vat. its tools are the facets it was given. the ChangeLog records everything it does. any action is revocable by revoking a facet.

### 8.3 the vision

the AI agent is not a chatbot bolted onto the outside. it is a **participant in the objectspace** — another user, with limited permissions, whose actions are visible, auditable, and reversible. non-programmers interact with the image through the agent. the agent explores, creates, modifies. the human watches in the Browser and approves.

---

## 9. the runtime

### 9.1 implementation language

**Rust.** the borrow checker is a free proof of memory safety at the GC boundary, which is the hardest and most dangerous code in the VM. worth the verbosity.

### 9.2 VM architecture

- **tagged pointer object representation** — objects are indices into a typed arena slab, not raw pointers. image serialization is "serialize the slab." gc is safe (no pointer chasing outside the arena).
- **bytecode interpreter** — the "truth layer." what gets serialized in the image. introspection operates on bytecode.
- **LLVM ORC JIT** — on-demand compilation of hot paths via `inkwell`. the jit is a cache; cold code never pays compilation cost. bytecode is the canonical form.
- **MMTk** — pluggable, generational, moving GC. plays well with image serialization since object layout is controlled.

### 9.3 performance: inline caches

for a prototypal message-passing language, inline caching is where speed lives — not compiler magic. every message send site caches the last N receiver handler-table shapes → handler lookups. most real code is monomorphic; you get near-static dispatch with zero type annotations.

**operative specialization**: if a `vau` call site always passes the same operative, the JIT specializes and inlines it. this is how the vau-is-hard-to-optimize problem is handled.

### 9.4 FFI

- **libffi** as the runtime layer — call arbitrary C functions by signature, no compilation step
- foreign C structs become MOOF proxy objects — fields are slots, pointer arithmetic is hidden behind handler dispatch
- a cffi-style declaration syntax in MOOF for describing C interfaces

```scheme
; declaring a C function
(ffi-define sqlite3-open
  lib: "libsqlite3"
  args: '(String)
  returns: 'Pointer)

; it's just an object with call:
(def db (sqlite3-open "/tmp/data.db"))
```

### 9.5 external world as objects

every external thing is a proxy object in the same message-passing model:

| thing | how it appears |
|---|---|
| C library | namespace of FFI proxy objects |
| MCP server | object whose send goes over the wire |
| HTTP endpoint | object responding to `get:` `post:` etc |
| OS process | object responding to `send:` `kill` `wait` |
| filesystem | object responding to `read:` `write:` `list:` |

same syntax everywhere. no special-casing.

---

## 10. the live environment

the IDE is not a separate application. it is a set of objects living in the image, written in MOOF.

**Browser** — the object explorer. every object is a clickable node. sends `[obj describe]` on click. slot values are live-editable. implements in MOOF.

**Inspector** — single-object deep view. all handlers, all slots, delegation chain, ChangeLog for this object, scratchpad that evals in the object's own environment.

**Workspace** — the REPL's successor. evaluation results persist as named objects in the image, not printed strings. you evaluate `[3 + 4]` and get a `7` object you can click, inspect, and wire up to other things.

**Transcript** — shared output surface. users, agents, and background processes all write here. a unified log of what is happening in the image.

**Debugger** — when an unhandled error reaches the top of a Vat, execution is **suspended, not terminated**. you inspect the live continuation (a first-class object), fix the handler, and resume. this is Smalltalk's "fix and proceed" and it is one of the greatest UX ideas in computing history.

---

## 11. standard library sketch

written entirely in MOOF, living in the image. approximately 3000 lines at full bootstrap.

### bootstrap (from vau + send + def + quote + cons + eq)
`lambda` `if` `let` `let*` `letrec` `cond` `when` `unless` `do` `loop` `while` `and` `or` `not` `match` `->` (threading) `quasiquote`

### object model
`Object` `Nil` `Boolean` `True` `False` `Magnitude` `Number` `Integer` `Float` `Ratio` `Character` `Symbol` `String` `Cons` `List` `Vector` `HashMap` `HashSet` `Stream` `Block` `Environment` `Operative`

### protocols
`Numeric` `Comparable` `Iterable` `Printable` `Serializable` `Callable`

### capability
`Vat` `Membrane` `Facet` `AuditLog` `RateLimiter`

### persistence
`WAL` `Snapshot` `ContentStore` `VersionedObject` `ChangeLog` `CRDT` `Counter` `GrowSet` `LastWriteWinsMap`

### reflection
`Mirror` `MethodDictionary` `ProtocolRegistry`

### AI integration
`MCPServer` `Agent` `Tool` `ToolRegistry`

### external world
`FFI` `IO` `HTTP` `Process` `Socket` `FileSystem` `ModuleLoader`

### environment
`Browser` `Inspector` `Workspace` `Transcript` `Debugger` `Profiler` `TestRunner`

---

## 12. bigger goals / milestones

**milestone 1 — the personal objectspace**
a single-user MOOF image that replaces your notes app, your scratchpad, your code editor. documents are objects with `render` handlers. tasks are objects that message each other. no files.

**milestone 2 — the AI-native environment**
an AI agent living *inside* the image as a first-class participant. collaboration feels like pair programming, not chatting with a tool. the agent's actions are visible and reversible.

**milestone 3 — the open fabric**
federated MOOF images. you grant a Facet reference to an object in your image; someone else can message it from theirs. object-capability-secure distributed computing. the original internet vision, without the HTTP+JSON+REST flattening.

**milestone 4 — new computing literacy**
because the agent can explore, explain, and modify the objectspace in natural language, and because the objectspace is maximally inspectable, MOOF becomes the first programming environment where a non-programmer can genuinely understand what their computer is doing. not "no-code" — *readable code in a living context.*

---

## 13. prior art and debts

| source | what we steal |
|---|---|
| Kernel language (Shutt) | vau / operative model — the actual foundation |
| Self | prototype object model, live environment IDE |
| Squeak / Pharo | image persistence, browser, fix-and-proceed debugger |
| E language | vat/membrane/facet capability security model |
| Io language | message-passing purity, radical simplicity |
| 3-Lisp / Black | reflective tower, reified evaluator |
| Racket | language-oriented layering on a minimal core |
| Julia | bytecode-as-truth + LLVM ORC JIT layering |
| Git | content-addressed object storage for persistence |
| MMTk | pluggable GC |

---

## 14. what MOOF is not

- **not a Lisp** — homoiconic and cons-based, but the object model and vau semantics are not Common Lisp, Scheme, or Clojure
- **not Smalltalk** — image-based and message-passing, but no class/metaclass distinction, and vau gives user code compiler-level power
- **not Self** — prototype-based, but slots and handlers are distinct namespaces; inheritance is handler-only
- **not a language with an IDE bolted on** — the environment is primary; the language is what the environment is made of

---

*this document describes the design intent. implementation details — especially around the JIT, GC integration, and bootstrap sequence — will evolve. the kernel primitive set (§2.1) is fixed by design. everything else is negotiable.*

*clarus lives. moof.*