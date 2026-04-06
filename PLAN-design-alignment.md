# The Shape of Moof

## what the philosophy actually says

strip away the feature lists, the standard library sketch, the milestone roadmap.
the design doc makes ONE claim:

> "the vm's single privileged operation is `send`. everything reduces to it."

not "send is important." not "send is the primary mechanism." EVERYTHING reduces to it.

if you take that seriously — really seriously — it dissolves the six kernel forms:

- **def** is a send: `[env define: 'name to: value]`
- **cons** is a send: `[Pair new: a with: b]`
- **eq** is a send: `[a identicalTo: b]`
- **quote** isn't an operation at all — it's the *absence* of evaluation
- **vau** is... the one genuine primitive. it creates the thing that can receive
  unevaluated sends. without it there are no handlers. it's the bootstrap spark.

so the kernel is really: **send** + **vau** + a handful of primordial objects to
send messages TO (Object, Environment, Pair, Boolean). that's it. that's the bottom
turtle.

and then `(f a b c)` is `[f call: a b c]`. and `@name` is `[self slotAt: 'name]`.
and `(if cond then else)` is `[cond ifTrue: then ifFalse: else]`. ALL sends.

## where the implementation breaks from this

### the HeapObject enum is eight parallel worlds

```rust
enum HeapObject {
    Cons { car, cdr },
    MoofString(String),
    GeneralObject { parent, slots, handlers },
    BytecodeChunk(BytecodeChunk),
    Operative { params, env_param, body, def_env, source },
    Lambda { params, body, def_env, source },
    Environment(Environment),
    NativeFunction { name },
}
```

eight variants. eight kinds of objects with DIFFERENT representations and
DIFFERENT dispatch paths. the Rust code is full of `match self.heap.get(id)` arms
that treat each variant specially.

but the philosophy says: there is ONE kind of thing. an object with slots and handlers.

a Cons is an object with `car` and `cdr` slots.
a Lambda is an object with a `call:` handler and `params`, `body`, `env` slots.
an Environment is an object with a `bindings` slot and `lookup:`, `define:to:` handlers.
a String is an object with byte data and `length`, `at:`, `++` handlers.

they're all objects. the IMPLEMENTATION can store them differently for performance
(unboxed cons, compact strings), but the SEMANTIC MODEL is: object responds to messages.

### message_send has four dispatch paths

```
1. GeneralObject handler lookup (delegation chain)
2. Type prototype handler lookup (NativeRegistry closures)
3. primitive_send fallback (250 lines of hardcoded Rust)
4. doesNotUnderstand:
```

the philosophy says: there is ONE dispatch path. look up the handler and call it.
if a type prototype has a handler, that's just... handler lookup. delegation.
if a primitive needs special behavior, register it as a handler.

primitive_send is a bypass of the object model. it's the VM saying "i know better
than the handler table." it should not exist.

### the compiler has 18+ special forms

`if`, `lambda`, `let`, `car`, `cdr`, `eval`, `object`, `handle!`, `quasiquote`...

the doc says: "user code has identical expressive power to 'compiler' code."
but a user can't define a new control flow form that compiles to jump instructions.
the compiler recognizes `if` and gives it special treatment. user-defined operatives
get the generic (slower) path.

this is the hardest tension. the honest answer: `if` as a compiler special form is
an OPTIMIZATION, not a privilege. the SEMANTICS are: `if` is an operative. the
COMPILER happens to recognize it and emit fast code. this is fine as long as:
- the operative version also exists and works
- user operatives can EVENTUALLY get the same treatment (inline caching)
- the optimization is transparent to the language level

### OP_SLOT_GET breaks encapsulation

`@name` compiles to `OP_SLOT_GET` which reads the internal `Vec<(u32, Value)>` directly.
no message send. no handler invocation. the object CANNOT intercept its own slot reads.

this breaks the entire capability security model. a Membrane wrapping an object should
be able to intercept `slotAt:` reads. but OP_SLOT_GET bypasses the membrane because
it never sends a message.

### OP_CALL is not a send

`(f x y)` compiles to `OP_CALL` which does `call_value()` — a direct match on
HeapObject variant (is it a Lambda? a NativeFunction? an Operative?). it falls back
to `[f call: x y]` only if none of those match. but the primary path is NOT a send.

this means: if you wrap a function in a Membrane, `(f x)` syntax will bypass the
membrane and call the function directly. only `[f call: x]` goes through the membrane.
the syntax and the semantics disagree.

## what radical coherence looks like

### one dispatch path

```rust
fn message_send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> VMResult {
    // 1. look up handler on receiver (or its delegation chain)
    if let Some(handler) = self.lookup_handler_for(receiver, selector) {
        return self.invoke_handler(handler, receiver, args);
    }
    // 2. doesNotUnderstand:
    ...
}
```

ONE function. no special cases. `lookup_handler_for` works on any value — immediates
route to their type prototype, heap objects check their own handlers then delegate.
primitive_send doesn't exist. there's no fallback. if the handler isn't in the table,
it's doesNotUnderstand.

this means: Float gets its own prototype with its own handlers. Boolean conditionals
are handlers on True/False. Environment operations are handlers on Environment.
GeneralObject introspection (slotAt:, slotNames) are handlers on Object (inherited
by everything).

### OP_CALL → OP_SEND 'call:

`(f x y)` compiles to `[f call: x y]` which is `OP_SEND 'call: 2`. the VM can
OPTIMIZE this (if it sees the receiver is a Lambda, skip handler lookup), but the
semantic path is always send. membranes work on function calls. doesNotUnderstand:
works on function calls.

### OP_SLOT_GET → OP_SEND 'slotAt:

`@name` compiles to `[self slotAt: 'name]`. slot access goes through the handler table.
objects can intercept slot reads. membranes can intercept slot reads. the VM can
optimize the common case (direct slot read on a plain GeneralObject), but the
semantic model is: it's a send.

### the type hierarchy is real

```
Object
  Number
    Integer
    Float
  Collection
    Cons
    String
  Boolean
  Symbol
  Environment
  Operative
  Lambda
  NativeFunction
```

these are real GeneralObject instances in the heap with real parent delegation.
Integer's parent is Number. Number's parent is Object. when you send `describe` to
an Integer, it walks: Integer proto → Number proto → Object proto → found handler.

this is pure moof work — define the prototypes, set the parent slots. the delegation
mechanism already works perfectly.

### block syntax `{ :x [x * 2] }`

this is the missing piece that makes the smalltalk-style control flow work:

```scheme
[condition ifTrue: { do-this } ifFalse: { do-that }]
(list 1 2 3) map: { :x [x * 2] }
```

a block is an object with a `call:` handler. `{ expr }` is a zero-arg block.
`{ :x expr }` is a one-arg block. the reader produces them as object literals
with a generated `call:` handler.

this unifies closures, blocks, and objects. a block IS an object. a closure IS
an object. `fn` is sugar for `vau` + `wrap`. block syntax is sugar for object literal
with call: handler. turtles all the way down.

## the concrete path

### phase A: unify dispatch (make primitive_send die)

1. give Float its own proto_float (not shared with Integer)
2. move all Float ops from primitive_send → Float proto handlers
3. move Boolean ifTrue:/ifFalse:/and:/or: → True/False proto handlers (VM intercepts for call_value)
4. move Environment eval:/lookup:/set:to:/define:to:/remove: → Environment proto handlers (VM intercepts)
5. move GeneralObject slotAt:/slotAt:put:/slotNames/handlerNames → Object proto handlers
6. move Lambda/NativeFunction call: → proto handlers
7. delete primitive_send

after this: message_send has ONE path. look up handler, invoke it.

### phase B: make call and slot-access into sends

1. OP_CALL compiles as OP_SEND with selector call: (with fast-path optimization)
2. OP_SLOT_GET compiles as OP_SEND with selector slotAt: (with fast-path optimization)
3. OP_SLOT_SET compiles as OP_SEND with selector slotAt:put:

the optimization: the VM recognizes the common case and shortcuts. but the SEMANTIC
path is send. membranes and DNU work uniformly.

### phase C: build the real hierarchy

pure moof work in the image:
1. define Number prototype with shared numeric operations
2. set Integer.parent = Number, Float.parent = Number
3. define Collection prototype with shared iteration
4. set Cons.parent = Collection, String.parent = Collection
5. checkpoint

### phase D: block syntax

parser change:
- `{ expr }` → object with zero-arg call: handler
- `{ :x expr }` → object with one-arg call: handler
- `{ :x :y expr }` → etc

compiler change:
- emit OP_MAKE_OBJECT with a call: handler

### phase E: reseed

1. export all module sources from the image
2. rebuild lib/ from those sources
3. delete image.bin, run --seed
4. remove legacy opcode shims
5. checkpoint clean image

## what this gives us

after all this, moof's runtime is:
- one dispatch mechanism (send)
- one kind of thing (objects with handlers)
- one kind of control flow (message passing)
- real prototype hierarchy with delegation
- uniform syntax (parens for call, brackets for send, braces for objects/blocks)
- capability security that actually works (membranes intercept everything)

and the codebase reflects the philosophy instead of contradicting it.
