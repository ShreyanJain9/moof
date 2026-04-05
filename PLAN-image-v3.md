# Plan: Image v3, Source/State Correspondence, and Getting MOOF Back On-Model

## Why this plan exists

The current system is off the design doc's axis in a fundamental way.

We have:

- a bytecode VM
- a source loader
- a directory-backed module store
- a bit of source introspection

What we do **not** have is the thing the design doc is actually about:

- a living image that is the canonical runtime truth
- source as a projection of image-resident program objects
- objects that carry their own provenance and explanation
- a coherent answer to "what survives?" and "where does this come from?"

The hard problem is not heap serialization.

The hard problem is maintaining a principled relationship between:

- authored source text
- compiled definitions
- live object identity
- mutable runtime state
- module organization
- reload and evolution over time

If we do not solve that relationship, adding `image.bin` only makes the system faster, not truer.

---

## The actual problem

Right now MOOF still behaves like a source-first language runtime. The design doc calls for an image-first objectspace.

### Current mismatches

1. **Truth is split**
   - Runtime truth lives in the heap.
   - Resting truth lives in `.moof/modules/*.moof`.
   - Some REPL truth is smuggled back into source by text append/dedup.
   - These can drift.

2. **Environment is carrying too much semantic weight**
   - Environments currently mix imports, definitions, transient locals, persistent bindings, and accidental runtime junk.
   - That makes "module = environment = source file" unstable and underspecified.

3. **Definitions lose provenance**
   - After evaluating `(def foo ...)`, we get a binding and maybe a source string somewhere, but not a durable first-class Definition object that owns the meaning of `foo`.

4. **Runtime mutation has no principled home**
   - Slot mutation, method redefinition, and runtime-created objects are real state changes.
   - Today some of them are exported to source, some are not, and the rule is not structural.

5. **Modules conflate multiple roles**
   - They currently serve as dependency units, persistence units, source files, and partial runtime compartments.
   - These should be related, but not identical.

6. **Reload has no identity model**
   - Re-evaluating source is not the same thing as evolving a living object graph.
   - The system needs a story for preserving object identity while changing definitions.

---

## The core design move

Stop treating raw environments as the unit of authorship.

They are not rich enough, stable enough, or structured enough.

Instead, make three layers explicit and first-class:

### 1. Definition layer

This is the canonical source-level meaning of the system.

A Definition object should represent a durable, named program element:

- owner module
- exported name
- canonical source
- parsed AST
- compiled chunk or compiled artifact
- referenced definitions
- doc / metadata / provenance
- patch history or generation

This is what projects to source text.

### 2. State layer

This is the live object graph:

- instances
- mutable slots
- runtime-created objects
- module singletons
- environments
- caches
- tool state

This is image state.

This is what `image.bin` should capture.

### 3. Projection layer

These are views of the image:

- `.moof/modules/*.moof`
- manifest metadata
- browser/editor views
- inspector output
- export-to-lib
- git diffs

These are not canonical truth. They are deterministic projections from image-resident objects.

---

## The new invariants

Image v3 should enforce these invariants:

1. **The image is canonical for runtime state**
   - If the runtime can observe it, the image can persist it.

2. **Definition objects are canonical for source meaning**
   - Every durable top-level definition has a stable heap identity and source provenance.

3. **Source files are deterministic projections of Definition objects**
   - Not handwritten append logs.
   - Not the primary truth layer.

4. **Modules are objectspace objects first, files second**
   - Files are exports of module state, not the only place modules "exist."

5. **Environments are execution contexts, not persistence units**
   - We may persist them, but we do not confuse them with authored structure.

6. **Not all persistent state must be source-projectable**
   - Only source-backed definitions must round-trip to text.
   - Arbitrary runtime objects may remain image-only unless explicitly projected.

7. **Reload is identity-preserving where possible**
   - Updating a prototype or module should patch living objects when semantically appropriate, not blindly replace everything.

---

## The key architectural trick

Do **not** attempt a naive 1:1 mapping between "source file" and "entire environment contents."

That is the wrong unit.

Instead, define a `ModuleImage` as the persistence/authorship unit.

## `ModuleImage`

Each module in the image should have a first-class representation that separates code from state:

```rust
pub struct ModuleImage {
    pub name: String,
    pub requires: Vec<String>,
    pub provides: Vec<String>,

    // Canonical code objects
    pub definitions: Vec<DefinitionId>,

    // Persistent module-owned runtime objects
    pub durable_objects: Vec<ObjectId>,

    // Execution context
    pub env_id: u32,

    // Projection metadata
    pub source_projection: ModuleSourceProjection,
    pub unrestricted: bool,
}
```

This lets us say:

- The **module file** corresponds to the module's `definitions`.
- The **image** corresponds to the module's full runtime state.
- The **environment** is just the module's execution context.

That is the missing separation.

---

## Definition objects

Every durable top-level `def`, `defmethod`, class/prototype declaration, and module-level handler install should become a Definition object.

### A Definition should know:

- `id`
- `module`
- `kind`
  - value binding
  - method definition
  - object/prototype definition
  - import/export declaration
- `name`
- `selector` for methods when applicable
- `source`
- `ast`
- `compiled_artifact`
- `dependencies`
- `target object identity` if this definition patches an existing object
- `last_applied_generation`

### Why this matters

Once definitions have stable identity:

- introspection can point to a real owner
- `source` becomes authoritative
- reload can update specific definitions instead of smashing whole modules
- file projection is structural, not heuristic

---

## Source-backed vs image-only objects

MOOF needs to stop pretending every persistent object must have pretty source.

Instead, define two categories explicitly:

### Source-backed objects

These are objects whose durable identity is tied to source definitions:

- module prototypes
- classes
- methods
- named singletons
- explicitly declared persistent module objects

These must support deterministic source projection.

### Image-only objects

These are persistent but not necessarily source-projected:

- ad hoc REPL objects
- caches
- runtime-generated data structures
- temporary graphs users still want persisted

These survive in `image.bin` but may have no textual projection beyond inspector/export tooling.

This is not a compromise. It is the correct model for an image system.

---

## How source projection should work

Source should be regenerated from Definition objects, not patched textually.

### Current bad model

- Append raw source text to `workspace.moof`
- Dedup by pattern matching on `(def name ...)`
- Rewrite `(provides ...)`

This is brittle and semantically weak.

### Target model

Each module file is rendered from:

- module header object
- ordered Definition objects
- source formatting policy

Projection should be:

- deterministic
- stable enough for git diff
- reversible enough to preserve authored intent where reasonable

### Important clarification

We do **not** need perfect token-for-token source preservation as the main invariant.

We need:

- semantic fidelity
- stable structure
- preserved comments/docs when possible
- durable definition identity

If exact text preservation is desired later, add concrete syntax trees or source spans. It should not block the architecture.

---

## What reload should mean

Reload should no longer mean "reparse file and re-eval module body into an env."

That is still compiler-runner thinking.

### Reload should become:

1. Parse projected source into candidate Definition objects
2. Diff against existing module definitions by stable identity / name / selector
3. Recompile only changed definitions
4. Reapply them to the live image
5. Preserve object identity when patching existing prototypes or module singletons
6. Recompute exports
7. Regenerate source projection

### Identity-preserving patch rules

These are the important cases:

- **Value definitions**
  - may replace binding value directly

- **Prototype/class definitions**
  - patch slots/handlers in place when possible
  - preserve the prototype object's identity

- **Method definitions**
  - replace handler implementation on the target object/prototype

- **Durable instances**
  - preserve identity
  - optionally migrate shape if their prototype changed

This is the real "living system" behavior the design doc is gesturing toward.

---

## Workspace should change meaning

The workspace should stop being a text append file with special treatment.

### Current workspace

- a module file
- REPL autosave target
- append-only-ish source sink

### Better workspace

The workspace is a first-class module with:

- its own `ModuleImage`
- Definition objects for every durable top-level REPL definition
- optional durable objects the user chooses to keep
- a projected `workspace.moof` as a readable view

Then REPL evaluation splits into two cases:

- **durable top-level forms**
  - create/update Definition objects
  - update module source projection

- **ephemeral expressions**
  - just evaluate
  - no source projection required

That gives a principled answer to "what from the REPL becomes part of the image?"

---

## Image v3 storage design

The binary image should persist the runtime truth and the definition/source metadata needed to reconstruct projections.

```text
.moof/
  image.bin
  image.sha256
  manifest.moof          ; optional projected metadata
  modules/
    *.moof               ; projected source view
```

### `Image` should contain

```rust
pub struct Image {
    pub version: u32,
    pub objects: Vec<HeapObject>,
    pub symbol_names: Vec<String>,
    pub root_env_id: u32,
    pub module_registry: ModuleRegistry,
    pub definition_registry: DefinitionRegistry,
}
```

### `ModuleRegistry` should contain

- module identity
- dependency metadata
- env id
- exported symbols
- definition ids in projection order
- durable object ids
- unrestricted flag

### `DefinitionRegistry` should contain

- all durable definition objects
- source text
- AST or compiled references
- ownership metadata

We do not have to fully normalize this on day one, but the model should point this way.

---

## Startup model

### Fast path

If `image.bin` exists:

1. deserialize image
2. reconstruct heap and symbol table
3. restore root env
4. re-register native functions
5. restore prototype caches
6. reconstruct `ModuleLoader` / module registry from image metadata
7. project source files only if needed
8. enter REPL immediately

### Rebuild path

If `image.bin` does not exist:

1. read `.moof/modules/*.moof`
2. parse into module headers plus definitions
3. build initial image objects
4. compile/apply definitions
5. create `image.bin`
6. continue from image model afterward

### Seed path

Only for first boot:

1. seed `.moof/modules/` from `lib/`
2. rebuild initial image
3. never treat `lib/` as canonical runtime truth again

---

## What to keep from the current system

Not everything should be thrown away.

### Keep

- the current parser/compiler/vm core
- `.moof/modules/` as a human-readable projection directory
- `ModuleLoader` as the shell around discovery / dependency order / exports
- `Modules` and `Module` moof-level objects
- current introspection surfaces where they work

### Replace or demote

- directory image as canonical persistence
- append-and-dedup workspace persistence
- "module environment = durable source truth" assumption
- full-module reload as the only update granularity

---

## Concrete implementation phases

## Phase 0: Make the target model explicit

Before coding heavily:

- document invariants from this file in `DESIGN.md` or a follow-up architecture doc
- decide what counts as a Definition in v1 of this design
- decide what counts as a durable module-owned object

Without this, implementation will drift back into ad hoc persistence.

## Phase 1: Bring back binary image persistence

Goal:

- make runtime state persist in one binary artifact
- leave current source module flow operational as fallback

Work:

- add `src/persistence/snapshot.rs`
- implement `Image { objects, symbol_names, root_env_id, module_registry }`
- add `Heap::from_image(...)` and read-only heap export helpers
- update `main.rs` to try image fast path first
- on checkpoint, write `image.bin` and project source

This phase fixes canonical runtime persistence, but not full source/state correspondence.

## Phase 2: Introduce first-class ModuleImage records

Goal:

- stop using raw environments as the persistence boundary

Work:

- extend module metadata in Rust to include durable module records
- track module-owned source text as heap objects
- track persistent module-owned objects separately from env bindings
- reconstruct module loader directly from image records

This phase gives modules a stable image identity.

## Phase 3: Introduce Definition objects

Goal:

- make top-level authored program elements first-class

Work:

- create a definition registry
- record top-level `def`, `defmethod`, and similar durable forms structurally
- store source and ownership on definitions
- update introspection to point at definitions rather than ad hoc source

This phase is the turning point.

## Phase 4: Replace text-append workspace persistence

Goal:

- eliminate heuristic source mutation

Work:

- REPL durable forms create/update Definition objects in the workspace module
- `workspace.moof` is rendered from those definitions
- ephemeral expressions remain image-only unless promoted

## Phase 5: Structural projection

Goal:

- generate source files deterministically from module/definition objects

Work:

- build a renderer for module headers and definitions
- preserve comments/docs where feasible
- remove `dedup_defs` / text surgery from the write path

## Phase 6: Identity-preserving reload and live patching

Goal:

- make the system feel like a living image, not a rebuild machine

Work:

- diff old/new definitions
- patch methods/prototypes in place
- preserve durable object identities
- add shape migration hooks for instances when needed

## Phase 7: Compacting GC for images

Goal:

- stop persisting garbage

Work:

- mark from root env, module records, definition registry, durable objects
- compact heap
- rewrite ids in image metadata

Do this after the metadata model is correct, not before.

---

## Verification criteria

Image v3 is only successful if these hold:

1. **Cold restart preserves live state**
   - mutate a slot
   - restart
   - state survives

2. **Definitions retain provenance**
   - ask where a binding came from
   - the system can point to a Definition object and its module

3. **Source projection is deterministic**
   - repeated checkpoints produce stable `.moof/modules/*.moof`

4. **Workspace is principled**
   - durable REPL definitions become workspace definitions
   - ephemeral computations do not pollute source projection

5. **Reload preserves identity**
   - editing a prototype updates its behavior without replacing every instance

6. **Image and source are recoverable from each other in the intended direction**
   - no image: rebuild from projected source
   - image present: startup from image without source re-eval

---

## Immediate code implications

### New or heavily changed files

| File | Change |
|------|--------|
| `src/persistence/snapshot.rs` | new binary image format and load/save path |
| `src/persistence/mod.rs` | expose both `image` and `snapshot` layers |
| `src/runtime/heap.rs` | image import/export helpers |
| `src/modules/loader.rs` | move toward image-backed module reconstruction |
| `src/main.rs` | image-first startup and checkpoint flow |

### Later-phase files

| File | Change |
|------|--------|
| `src/compiler/compile.rs` | optionally emit durable definition metadata for top-level forms |
| `src/vm/exec.rs` | support live patch / reload semantics where needed |
| `.moof/modules/modules.moof` | align moof-level module API with image-backed module objects |
| workspace handling in `src/main.rs` and `src/modules/loader.rs` | replace append-and-dedup with definition objects |

---

## Final position

The right answer is:

- **binary image as canonical runtime truth**
- **Definition objects as canonical source truth**
- **source files as projections**
- **environments as runtime scopes, not authorship units**

If we do only "serialize the heap + keep the current module/source model", we will get faster startup and better persistence, but we will still be philosophically off.

If we add Definition objects and ModuleImage records, the system starts to line up with the design doc's actual worldview.
