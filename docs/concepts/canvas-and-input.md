# canvas and input

> **`$canvas` and `$pointer` are the i/o leaves moofpaint stands on.
> a canvas is a 1-bit pixmap with rust-backed primitives. a pointer
> is a DataSource of input events. both are per-replica ambient
> capabilities, not part of replicated state.**

## $canvas

a `$canvas` cap is a write-side render surface (3D-aware; see
`concepts/world-and-space.md`). its protocol:

```moof
[$canvas dimensions]                     ; → #[width: … height: …]
[$canvas clear]                          ; → all-blank
[$canvas frame-begin: camera]            ; → start a new render frame
[$canvas frame-end]                      ; → flush to screen
[$canvas textured-quad-at: pose
                     size: #[w h]
                  texture: bits
                   camera: cam]          ; bit-pixmap as a 3D quad
[$canvas mesh-at: pose mesh: m]          ; arbitrary triangle mesh
[$canvas text-extrude: str at: pose
                  depth: d font: f]      ; 3D text
[$canvas line-3d: p1 to: p2 weight: w]   ; 3D line
[$canvas refresh]                        ; commit
```

the cap is delivered by a render mco. the moof side sees an
ordinary Form whose proto carries native methods.

### renderers

phase E ships **`render/terminal`** as the canonical first
renderer:

- a software 3D rasterizer to braille / half-block characters
  (`▀`, `▄`, `█`, `⠿`). depth-buffered. 30fps target on a
  reasonable terminal.
- low-resolution (~80×40 character cells; ~160×80 pixels via
  half-block); plenty for a moof-shaped demo, awful for "real"
  3D — that's by design.

phase G ships **`render/wgpu`** for gpu acceleration on desktop,
and **`render/web`** for browser canvas via wasm. all conform to
the same `$canvas` protocol; moof code never asks which is in use.

### 1-bit as a content choice, not a renderer constraint

the renderer is free-floating; pixmap content (`concepts/pixmap.md`)
defaults to 1-bit because that's the macpaint aesthetic, but the
canvas itself takes any color. RGBA pixmaps and 3D meshes both
render through the same `$canvas` protocol; the canvas paints
whatever pose+geometry+texture it gets.

## $pointer

a `$pointer` cap is a read-side input source. its protocol is
standard `DataSource` (`concepts/data-sources.md`), yielding screen-
space events:

```moof
{PointerEvent
  kind: 'down | 'move | 'up | 'cancel
  x: <integer>                 ; screen pixels
  y: <integer>                 ; screen pixels
  buttons: <set>
  modifiers: <set>}
```

the wrapper vat consumes these, ray-casts via the local viewport's
camera (`concepts/world-and-space.md`), and translates hits into
world-space input envelopes (`{PointerOnForm form-id: … hit-uv: …}`)
sent to the world-vat.

the cap is delivered by an input mco. terminal mode reads
xterm-mouse-protocol; web mode reads via a JS shim's
`pointer{down,move,up,cancel}` events; sdl mode reads sdl2.

### rate

`$pointer` events arrive at "device rate" — typically 60–240 Hz
depending on the input device. the wrapper vat batches them into
moofpaint's tick rate (`concepts/replication.md`) — typically 20Hz —
and submits a `StrokeAddPoint` envelope per batch.

raw events are *not* replicated. only the batched
`StrokeAddPoint`/`StrokeEnd` envelopes go into the replicated input
log.

## $keyboard

a `$keyboard` cap reads keystrokes:

```moof
{KeyEvent
  kind: 'press | 'release | 'repeat
  key: <symbol>           ; 'a, 'space, 'shift-a
  modifiers: <set>
  text: <string>}         ; the textual representation, if any
```

terminal: stdin parsed with mintaka-style escape decoding. web:
JS shim listens to `keydown`/`keyup`.

moofpaint uses keyboard for tool-switching (P for pencil, E for
eraser, etc.) and undo (`Cmd+Z` / `Ctrl+Z`).

## why these are caps and not part of the substrate

moof's discipline (`concepts/capabilities.md`): every i/o capability
is unforgeable, requested at vat birth from a supervisor. `$canvas`
and `$pointer` are ordinary caps in the same shape as `$out` and
`$fs`. nothing about them is special at the substrate level.

their *implementations* live in **mcos**
(`concepts/compiled-objects.md`), not in the substrate seed:

- `render/terminal.mco` — software 3D rasterizer to half-blocks.
- `render/wgpu.mco` — gpu renderer (phase G+).
- `input/xterm-mouse.mco`, `input/xterm-keys.mco` — terminal input.
- `input/sdl-pointer.mco` — sdl2 input (phase G+).

the seed itself only knows about the mco loader. the renderer and
input drivers are loaded at world boot like any other proto.

## per-replica binding

each replica's local supervisor binds its own `$canvas` to its own
local screen, and `$pointer` to its own local input. the
*replicated* canvas vat doesn't hold these caps; the local *wrapper
vat* does, and forwards events into the replicated vat as input
envelopes.

```
alice's machine:                bob's machine:
                                
  $canvas-alice                   $canvas-bob
   ↓                              ↓
  wrapper-alice                   wrapper-bob
   ↓ (sends inputs)               ↓ (sends inputs)
  reflector  <───────────────────  reflector
   ↓ (broadcasts envelopes)      
  canvas vat (replica-A)          canvas vat (replica-B)
  identical heap                  identical heap
   ↑ (read for render)             ↑ (read for render)
  wrapper-alice                   wrapper-bob
   ↓ ([$canvas-alice paint:])     ↓ ([$canvas-bob paint:])
  alice's screen                  bob's screen
```

each replica's render loop is *not* deterministic — alice may render
at 60fps, bob at 30fps, but both are reading the same heap. the
*replicated state* is identical; the *render output* is per-machine.

## intent/receipt for canvas

per `concepts/effect-intents.md`, cap calls in a replicated vat
become intents. but the canvas vat *doesn't* hold `$canvas`; the
wrapper does. so:

- replicated canvas vat: never makes a `$canvas` intent. it just
  mutates the stroke log; the wrapper polls the stroke log and
  re-renders.
- wrapper vat (solo): `[$canvas paint: …]` is a normal sync cap call,
  no intent needed. it's a per-replica side effect.

the indirection means the canvas vat stays purely deterministic and
the rendering is "outside" the replicated state machine. clean.

## inspirations

- **macpaint** (atkinson 1984) — visual model: 1-bit, tools, palette.
- **smalltalk's BitBlt** — the original "blit pixels around" primitive
  (kay et al.).
- **morphic** (maloney, smith 1995) — direct-manipulation event
  model.
- **rust's pixels crate / wgpu** — the modern rust toolchain for
  pixel-level rendering.
- **tldraw / excalidraw** — collaborative drawing UI patterns.
- **xterm mouse protocol** — terminal-side mouse input
  (https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Mouse-Tracking).

## see also

- `concepts/moofpaint.md` — the demo that uses these.
- `concepts/data-sources.md` — `$pointer` is a DataSource.
- `concepts/capabilities.md` — generic cap discipline.
- `concepts/compiled-objects.md` — how rust-backed methods land.
- `concepts/replication.md` — why these are wrapper-vat-bound.
