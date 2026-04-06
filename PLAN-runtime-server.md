# Plan: Runtime Server Architecture

## Why this plan exists

MOOF is currently a single-threaded REPL. The design doc envisions a living computational environment where multiple frontends (REPL, GUI browser, AI agent, MCP server) interact concurrently with the same objectspace. Getting there requires a runtime server architecture: the heap is a shared resource, vats provide isolation and scheduling, extensions provide I/O, and the event loop ties it together.

This plan was informed by a three-way brainstorm (Codex, Gemini, Claude) and grounded in the current codebase.

---

## The architecture

```
┌─────────────────────────────────────────────┐
│ Runtime                                      │
│  ┌──────────┐ ┌───────────┐ ┌────────────┐  │
│  │   Heap   │ │ Scheduler │ │ Extensions │  │
│  │ (shared) │ │           │ │            │  │
│  └──────────┘ └─────┬─────┘ └──────┬─────┘  │
│                     │              │         │
│         ┌───────────┼──────────────┼───┐     │
│         │           │              │   │     │
│    ┌────┴───┐  ┌────┴───┐   ┌─────┴┐  │     │
│    │ Vat 0  │  │ Vat 1  │   │Vat 2 │  │     │
│    │ REPL   │  │  GUI   │   │Agent │  │     │
│    │        │  │        │   │      │  │     │
│    │ stack  │  │ stack  │   │stack │  │     │
│    │ frames │  │ frames │   │frames│  │     │
│    │ root   │  │ root   │   │root  │  │     │
│    │ mailbox│  │ mailbox│   │mailbx│  │     │
│    └────────┘  └────────┘   └──────┘  │     │
│         │           │              │   │     │
│         └───────────┼──────────────┘   │     │
│                     │                  │     │
│              ┌──────┴──────┐           │     │
│              │  Event Loop │◄──────────┘     │
│              └─────────────┘                 │
└─────────────────────────────────────────────┘
```

### Core principles (from brainstorm consensus)

1. **Single scheduler thread for heap access.** The heap is a `Vec<HeapObject>` with no synchronization. I/O extensions run on their own threads and produce events. The scheduler thread runs vat turns and processes events. Node.js model: single-threaded execution, multi-threaded I/O.
2. **Vats are worlds.** Each vat has its own stack, frames, root env, mailbox, and capabilities. A vat is the boundary between pure computation and effects — the IO monad, enforced at runtime.
3. **Fuel-based yielding.** The VM executes N bytecode instructions then yields control to the scheduler. No mid-instruction preemption, no data races.
4. **Extensions are event sources.** GUI, terminal, MCP, network — each extension runs on its own thread, produces events for the scheduler, and consumes output (render trees, responses).
5. **Owned-shared heap.** All vats can READ any object (zero-cost, no copying). Only the owning vat can MUTATE. Cross-vat mutation goes through eventual sends and returns promises.

### The concurrency philosophy: vats as monads

The middle ground between Erlang (separate heaps, message copying) and E (shared heap, sequential turns):

```
read  = always immediate, any vat, shared heap, zero cost
write = only if you own it, or eventual send to the owner
I/O   = only if your vat holds the capability
```

**Pure code transcends vats.** `(fn (x) [x + 1])` has no effects — it can run in any vat or no vat. It's just computation on the shared heap.

**Effects require a vat.** Mutation, I/O, cross-vat sends — these are effects. The vat determines what effects are permitted via its capability set.

This gives us:
- **Erlang's isolation** — no uncoordinated mutation, no data races, supervisors restart crashed vats
- **E's shared objects** — the GUI reads the same objects the REPL writes, no copying, no serialization
- **Moof's own model** — delegation IS cross-vat reading (walking the prototype chain is just pointer chasing through the shared heap). Slot reads are safe. Slot writes are effects.
- **Capability security** — a vat's capabilities determine what it can do. The agent vat has `[filesystem read:]` but not `[filesystem write:]` because its membrane only grants read facets.
- **Testability** — run code in a vat with no I/O capabilities. Pure functions don't need mocking.
- **Agent speculation** — the agent runs in a sandboxed vat, tries mutations. If approved, mutations commit. If not, the vat is discarded. No shadow copies needed — the vat IS the sandbox.

The architecture:

```
[Extension threads]        [Scheduler thread]
  terminal  ──events──►   ┌──────────────────┐
  network   ──events──►   │ event loop       │
  AI API    ──events──►   │  ↓               │
                           │ dispatch to vats │
                           │  ↓               │     ┌──────┐
  terminal  ◄──render──   │ run vat turn     │◄───►│ Heap │
  network   ◄──response── │  ↓               │     │shared│
                           │ check GC        │     └──────┘
                           └──────────────────┘
```

---

## Phase 0: Foundation — GC + Error handling

Before any concurrency work. These are blocking.

### 0a: Compacting GC

Bring back mark-and-compact from §7, updated for the current architecture.

**Roots:** root_env + all ModuleImage envs + all Definition source objects + all vat roots (later) + extension-registered roots.

**When:** between turns (scheduler safepoint). Triggered by allocation count threshold or explicit `(gc)`.

**How:** Codex's "epoch heap" model — build a new arena by tracing from roots, copy reachable objects, rewrite all u32 IDs through a forwarding table, atomically swap. This reuses the existing arena/ID model and avoids mid-turn relocation hazards.

**Interaction with persistence:** `save_image` is just "trace + serialize the live set" — same machinery as GC.

**Files:** `src/runtime/heap.rs` (add `compact(roots: &[u32]) -> ForwardingTable`), `src/vm/exec.rs` (apply forwarding table to VM state).

### 0b: Error containment (try/catch)

**Phase 1 (minimal):** A `try` vau operative that catches `Err(String)` from `execute()`.

```moof
(try
  (do dangerous-thing)
  (fn (err) (println (str "caught: " err))))
```

Implemented as a VM-level native `__try` that wraps `execute()` in Rust error handling.

**Phase 2 (future):** First-class error objects (not strings). Suspended continuations for fix-and-proceed. This requires reifying `CallFrame` as a heap object, which is a big change — defer until needed for the debugger.

**Files:** `src/vm/exec.rs` (add `__try` intercept), `src/vm/natives.rs` (register), bootstrap.moof (add `try` vau).

---

## Phase 1: Runtime + Vats + Event Loop

The big architectural change. Split the monolithic `main()` loop into a proper runtime.

### 1a: Split VM into Runtime + VatState

Currently `VM` owns everything. Split it:

```rust
struct Runtime {
    heap: Heap,
    native_registry: NativeRegistry,
    scheduler: Scheduler,
    extensions: Vec<Box<dyn MoofExtension>>,
    ffi_libs: HashMap<String, NativeLibrary>,
}

struct VatState {
    id: VatId,
    stack: Vec<Value>,
    frames: Vec<CallFrame>,
    root_env: u32,
    mailbox: VecDeque<Message>,
    status: VatStatus, // Running, Suspended, Waiting(PromiseId)
    fuel: u32,
}

enum VatStatus {
    Ready,
    Running,
    Suspended { error: Value },
    Waiting { promise: u32 },
}

struct Scheduler {
    vats: Vec<VatState>,
    active_vat: Option<VatId>,
    default_fuel: u32, // instructions per turn
}
```

The `VM` struct becomes a transient handle: `Runtime` + `&mut VatState` for the currently executing vat. All heap access goes through `Runtime.heap`. Stack/frame access goes through `VatState`.

### 1b: Fuel-based execution

Modify `VM::run()` to count instructions and yield after `fuel` is exhausted:

```rust
enum TurnResult {
    Completed(Value),     // turn finished normally
    Yielded,              // fuel exhausted, resume later
    Error(String),        // unhandled error
    Suspended(Value),     // error caught at vat level, awaiting fix
}
```

The scheduler loops:
```rust
loop {
    // 1. Poll extensions for events
    for ext in &mut self.extensions {
        for event in ext.poll(timeout) {
            let target_vat = event.target_vat;
            self.scheduler.enqueue(target_vat, event.as_message());
        }
    }
    
    // 2. Run each ready vat for one turn
    for vat in self.scheduler.ready_vats() {
        match self.run_turn(vat) {
            TurnResult::Completed(val) => { /* deliver result */ }
            TurnResult::Yielded => { /* re-queue */ }
            TurnResult::Error(e) => { vat.status = Suspended { error: e }; }
            TurnResult::Suspended(e) => { /* log, notify debugger */ }
        }
    }
    
    // 3. GC if needed
    if self.heap.should_gc() {
        self.gc();
    }
}
```

### 1c: Cross-vat sends and promises

Immediate send: `[obj foo]` — synchronous, same-vat. No change from current behavior.

Eventual send: `[obj <- foo]` — new syntax. Enqueues message in target vat's mailbox. Returns a Promise object.

```rust
struct Message {
    receiver: u32,   // heap object id
    selector: u32,   // symbol id
    args: Vec<Value>,
    resolver: Option<u32>, // Promise object to resolve with result
}
```

Promise is a moof object:
```moof
(def Promise { Object
  value: nil
  resolved: false
  waiters: nil
  
  then: (callback)
    (if @resolved
      (callback @value)
      (<- @waiters (cons callback @waiters)))
    self
})
```

Cross-vat boundary detection: either by vat ownership tag on objects (Codex's suggestion — add `vat_id: u16` to GeneralObject), or by membrane wrapping (existing architecture). Start with membranes (simpler, already partially implemented), add ownership tags later if performance requires.

### 1d: Extension trait

```rust
trait MoofExtension: Send {
    /// Name for logging/debugging
    fn name(&self) -> &str;
    
    /// Startup: register natives, create heap objects
    fn register(&mut self, runtime: &mut Runtime, root_env: u32);
    
    /// Event loop: poll for events (non-blocking)
    fn poll(&mut self, timeout: Duration) -> Vec<Event>;
    
    /// Lifecycle: called on checkpoint/save
    fn on_checkpoint(&mut self, runtime: &Runtime);
    
    /// Lifecycle: called on image resume (re-register non-serializable state)
    fn on_resume(&mut self, runtime: &mut Runtime, root_env: u32);
    
    /// Roots: return heap object IDs that this extension keeps alive
    fn gc_roots(&self) -> Vec<u32> { Vec::new() }
    
    /// Render: consume a render tree from a vat (for GUI extensions)
    fn render(&mut self, _tree: Value) {}
}
```

Concrete extensions:
- **ReplExtension** — polls stdin, produces "line_ready" events, renders to stdout
- **TerminalExtension** — polls crossterm events, renders widget trees via ratatui
- **McpExtension** — polls JSON-RPC over stdio, produces "tool_call" events
- **AgentExtension** — polls AI API, produces "agent_action" events

---

## Phase 2: Render Protocol + GUI

Once the event loop exists, build the visual layer.

### 2a: Widget tree protocol

`[obj render]` returns a tree of moof objects:

```moof
{ VBox children: (list
    { Label text: "hello" }
    { HBox children: (list
        { Button text: "click" on-click: my-handler }
        { TextInput value: "" on-change: my-handler }
    )}
)}
```

Widget prototypes (VBox, HBox, Label, Button, TextInput, etc.) defined in a `ui.moof` module. They're plain moof objects with slots — the renderer walks the tree and draws.

### 2b: Declarative rendering (Elm-style)

Each GUI vat has a `model` (state) and a `view` function:

```moof
(def my-app { Object
  count: 0
  
  view: ()
    { VBox children: (list
        { Label text: (str "Count: " @count) }
        { Button text: "+" on-click: (fn () (<- @count [@count + 1])) }
    )}
})
```

The terminal extension calls `[app view]`, diffs against the previous tree, and draws only the changes.

### 2c: Browser and Inspector as moof objects

The Browser is a moof object in a GUI vat:
```moof
(def Browser { Object
  selected: nil
  
  view: ()
    { VBox children: (list
        { Label text: "Object Browser" }
        [self module-list-view]
        [self detail-view]
    )}
    
  module-list-view: ()
    { VBox children: (map (fn (name)
        { Button text: name on-click: (fn () (<- @selected [Modules named: name])) })
      [Modules list]) }
      
  detail-view: ()
    (if (null? @selected) { Label text: "(select a module)" }
      { VBox children: (map (fn (d)
          { Label text: [d slotAt: 'name] })
        [@selected slotAt: 'definitions]) })
})
```

---

## Phase 3: Identity + Live Patching

Make reload preserve object identity. When a Definition's source changes:

1. Recompile the new source
2. Find the existing binding (the old value)
3. If it's a prototype: patch handlers in-place (existing instances automatically get new behavior via delegation)
4. If it's a function: replace the binding (old closures keep old behavior, new calls get new behavior)

This is what makes live coding work — you change a render handler and the GUI updates immediately without losing state.

---

## Phase 4: Milestone 1 deliverables

With phases 0-3 done:
- Documents as objects with `render` handlers ✓
- Tasks as objects that message each other ✓
- Browser/Inspector as moof objects in the image ✓
- Persistent, living, self-modifying objectspace ✓
- No files — everything is objects ✓

---

## Implementation order

```
Phase 0a: GC (epoch compact)          — 1 session
Phase 0b: try/catch                   — 1 session  
Phase 1a: Split VM → Runtime+VatState — 2 sessions (biggest refactor)
Phase 1b: Fuel-based execution        — 1 session
Phase 1c: Promises + eventual sends   — 1 session
Phase 1d: Extension trait              — 1 session
Phase 2a: Widget prototypes           — 1 session
Phase 2b: Terminal renderer           — 1 session
Phase 2c: Browser/Inspector           — 2 sessions
Phase 3:  Live patching               — 1 session
Phase 4:  Document/task objects       — 1 session
```

Total: ~13 sessions to milestone 1.

---

## Design doc updates needed

- §6 "source files are canonical" → heap is canonical, source is projection
- §6 add binary image persistence as the primary format
- §5 add vat scheduling model (single-threaded, fuel-based, E-style turns)
- §9 add Runtime/VatState split
- §10 add render protocol (widget tree) and extension model
- §7 add continuation-based error model (future)
- New §: extension system as the Rust/moof boundary

---

## Resolved questions

1. **Vat ownership:** Mediated by membranes/facets, not object tags. Fits the capability model. Tags are a future optimization if profiling shows membrane overhead.

2. **Eventual send syntax:** `[obj <- foo]` works. `(<- x y)` for mutation is a vau in moof-land. `[obj <- foo]` is a message send with selector `<-`. Different dispatch paths, no collision.

3. **Widget tree diffing:** Rust extension implements the diff natively, but exposed through a moof interface (`[Renderer diff: old new]`). All Rust-level functionality bound as extensions, never as invisible magic. Moof-level diff as a future replacement.

4. **GUI backend:** Native GUI via Rust extension (iced or similar for custom styling — egui too limited for skeuomorphic). Terminal (ratatui) is the first target to prove the architecture. GUI extension hooks into the event loop like any other extension. Multiple GUI vats are possible with native windowing.

5. **Vat persistence:** Each vat saves separately. Suspended vats survive checkpoint — their stack/frames serialize into the image. On resume, the vat picks up where it left off. Ephemeral vats (one-shot request handlers) can opt out.
