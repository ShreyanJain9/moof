# moldability

> **moof is moldable from within. anything not requiring rust lives
> as moof code, modifiable at runtime, with full reflection. tools
> are objects you can rewrite. the editor is in moof. the inspector
> is in moof. the compiler is in moof.**

moldability is a culture (gîrba et al., glamorous toolkit) more than
a feature. the substrate provides the conditions; the environment is
where moldability actually happens.

## the principle

> if it can live above the rust line, it does.

the rust line provides the irreducibly-substrate things: heap, GC,
bytecode interpreter, scheduler, transport leaves, `.mco` loader,
and the bootstrap parser. *every other concern is moof code.*

the parser proper, the compiler, the analyzer, the type system, the
inspector, the canvas, the morphic UI, the editor, the package
system, the query engine — moof. all of it. modifiable while running.

## the maru posture

we credit ian piumarta and alessandro warth: their *open extensible
object models* paper (VPRI 2007) articulated the substrate-as-tiny-
seed posture. piumarta's maru (~200 lines of C bootstrapping a full
lisp) is the concrete demonstration.

we add: the seed is rust (memory-safe, OS-portable, performant), and
the world above it is structured around four substrate primitives
(Forms, Vats, DataSources, Compiled-Objects-for-rust-bridge).

## what makes moldability *real*

three things:

### 1. reflection is total

every Form exposes its proto, slots, handlers, source, bytecode,
caps, journal-id (`concepts/reflection.md`). nothing is hidden.

### 2. mutability is live

editing a method updates the proto's handler table. existing
instances pick up the change on next dispatch (modulo in-flight
frames). inline caches invalidate. you don't need to "restart."

### 3. tools are objects

the inspector is a vat with a render-on-canvas behavior. the
debugger is a vat that intercepts a frame's resume. the editor is a
vat that watches keypresses and renders text. each tool is a Form.
each can be inspected, edited, replaced.

## the test

> can a person, working only from inside moof, redefine a special
> form (the language is theirs), rewrite the inspector for a domain
> object (the tools are theirs), query the world's history
> relationally (the past is theirs), spawn a collaborator on another
> machine and watch them edit a method live (the social fabric is
> theirs), and find everything as they left it tomorrow (the
> persistence is real)?

this is the success criterion (`vision/manifesto.md`). every
substrate decision is judged against it.

## what living-inside-it looks like

a typical day:

- you wake the world. last week's open inspector is back; the long-
  running training vat resumed where it left off; your scratchpad has
  the half-thought from friday.
- you click on an object. the inspector that opens is custom for
  that object's type. you edit a method via the inline editor.
- you ask a relational question. a query opens; results stream;
  you save the search.
- you spawn a friend's far-ref into your workspace. they show up
  in the canvas; their cursor is visible; you both edit.
- you rewind a vat by 5 minutes to undo a bad change. the world is
  consistent.
- you write a small rust function for a hot loop, build a `.mco`,
  load it. it appears as a new method on your object. you keep going.

none of this is unusual. all of it is what smalltalk-80 + erlang +
glamorous toolkit + croquet would have been if those projects had
been one project.

## inspirations

- maru / COLA: piumarta & warth. the substrate-is-tiny posture.
- glamorous toolkit: gîrba et al. moldable development as culture.
- smalltalk-80: kay, ingalls, goldberg, robson. the live image.
- self / morphic: ungar, smith, maloney. direct manipulation of code.
- emacs: stallman, et al. the editor is itself programmable.
- the conviction that *the user owns the system* — anywhere in
  programming where this has been taken seriously.

## see also

- `vision/manifesto.md` — why moldability is the thesis.
- `concepts/reflection.md` — the substrate guarantee.
- `concepts/forms.md` — what makes everything modifiable.
- `process/docs-driven.md` — the rule for choosing rust vs moof.
