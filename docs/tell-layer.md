# the tell layer

a human-friendly scripting surface for moof's live object world.
not a replacement for moof proper — a translation membrane between
human intent and message sends.

## architecture

```
canvas / direct manipulation        pointing, dragging, gesturing
tell layer                           end-user scripting (this doc)
moof proper                          system language (s-exprs, sends, operatives)
bytecode / VM                        execution
```

the tell layer parses to moof s-exprs. zero changes to compiler, VM,
or runtime. purely a second parser: `parse_tell(source) -> Value`.

## the one operation

everything is a message send. the tell layer makes that readable.

```
tell the circle to hide.
```

desugars to `[circle hide]`. that's the whole idea.

## grammar

```
program       = statement*
statement     = tell-stmt | set-stmt | if-stmt | for-stmt | query | expr "."

tell-stmt     = "tell" target "to" message-chain "."
set-stmt      = "set" (slot-path | name) "to" expr "."
if-stmt       = "if" expr "then" block ("otherwise" block)? "end"
for-stmt      = "for each" name "in" expr "," block "end"
query         = quantifier? type-name ("where" predicate)? sort-clause? limit-clause? "."
expr          = target | literal | get-expr | binop | "(" expr ")" | bracket-send

message-chain = message ("then" message)*
message       = selector | selector ":" arg-list
arg-list      = expr ("," expr)*

target        = name | "the" name | get-expr
get-expr      = "the" slot "of" target
slot-path     = "the" slot "of" target

quantifier    = "every" | "all" | "any"
sort-clause   = "sorted by:" selector
limit-clause  = "first" integer | "last" integer

block         = statement+
bracket-send  = "[" expr selector (":" expr)* "]"    -- escape hatch to moof proper
```

roughly fifteen rules. no NLP. no fuzzy matching.
a rigid grammar with english word order, like SQL.

## examples

### messages

```
-- unary (no args)
tell the circle to hide.
tell the document to archive.

-- keyword (args after colons)
tell the circle to move to: 100, 200.
tell the label to set text: "hello".

-- chaining with 'then'
tell my inbox to filter: unread then sort by: date then take: 10.
```

### desugaring

```
tell the circle to hide.
  --> [circle hide]

tell the circle to move to: 100, 200.
  --> [circle moveTo: 100 200]

tell my inbox to filter: unread then sort by: date.
  --> [[inbox filter: unread] sortBy: date]
```

### slots

```
-- get
the color of the circle.
  --> [circle slotAt: 'color]

the name of the document.
  --> [document slotAt: 'name]

-- set
set the color of the circle to red.
  --> [circle slotAt: 'color put: red]

-- simple assignment
set x to 42.
  --> (def x 42)

set x to x + 1.
  --> (:= x [x + 1])
```

### control flow

```
if the count of items > 10 then
  tell the list to paginate.
otherwise
  tell the list to show all.
end

  --> (if [[items count] > 10]
        [list paginate]
        [list showAll])
```

```
for each item in the inbox,
  tell item to archive.
end

  --> [inbox each: |item| [item archive]]
```

### queries (sugar over Iterable)

```
every document where modified > yesterday.
  --> [documents select: |d| [[d modified] > yesterday]]

the first 3 notes sorted by: date.
  --> [[[notes sortBy: |n| [n date]] take: 3]

every task where done = false sorted by: priority.
  --> [[tasks select: |t| [[t done] equal: false]] sortBy: |t| [t priority]]
```

## design rules

1. **every statement ends with a period.** smalltalk's rule. no ambiguity.
2. **`tell X to Y` is the one operation.** everything desugars into sends.
3. **`the X of Y` is slot access.** always. predictable.
4. **`then` chains sends.** replaces nested brackets.
5. **colons mark arguments.** same as moof proper.
6. **barewords resolve against the environment.** `the circle` looks up `circle`. `red` looks up `red`.
7. **bracket syntax is the escape hatch.** `[circle moveTo: 100 200]` works anywhere.
8. **objects define the vocabulary.** what you can say to a thing = its handlers. `tell X to Y` works iff X responds to Y.

## what this is NOT

- not a general-purpose language. no closures, no vau, no protocols.
  for that, drop into moof proper.
- not NLP. the grammar is fixed and learnable.
  "move the circle right 50" does NOT work.
  "tell the circle to move by: 50, 0" does.
- not a replacement for moof. the system language stays lispy and precise.
  this is the command bar. the voice input. the first thing a new user types.

## implementation

a recursive descent parser that emits moof s-exprs (cons cells).
lives alongside the existing parser. could be:
- `src/lang/tell.rs` — `parse_tell(source: &str, heap: &mut Heap) -> Vec<Value>`
- REPL detects tell-mode by leading keyword (`tell`, `set`, `if`, `for each`, `every`)
  or by a mode toggle
- output feeds into the same `Compiler::compile_toplevel` path

## open questions

- how does `the X` resolve? is it `Env slotAt: 'X`? canvas spatial lookup?
  probably: check the focused/selected context first, then Env, then canvas.
- multi-word names: `the address book` vs `the addressBook`?
  could allow quoted names: `the "address book"`.
- how do blocks/closures surface? `for each` covers iteration,
  but what about `map:` with a transform? maybe:
  `tell the list to map: (each item -> item name).`
- event binding: `when X happens, do Y.` — needs the event/vat system.
- should the tell layer be a separate file extension? `.tell` vs `.moof`?
