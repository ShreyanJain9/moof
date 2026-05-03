# the wasm mco abi

> **language-neutral contract every wasm mco speaks. binding files
> in zig / c / ocaml / haskell / future-langs target THIS doc as
> source-of-truth, not each other. host substrate's
> `crates/substrate/src/wasm.rs` implements the host side.**

## abi version

current: 1. mco manifests declare `(abi-version 1)`. substrate
refuses to load mcos whose abi-version it doesn't support.

## handle layout

every value crossing the wasm boundary that isn't a primitive
(int / float) is represented as a `u32` handle indexing into a
per-call **handle table** maintained on the rust side. the handle
table is allocated on dispatch entry and drained on dispatch exit
(including via raise). wasm code MUST NOT cache handles across
dispatches; doing so is undefined behavior.

## imports surface (`moof` namespace)

### `moof_raise(kind_handle: u32, msg_ptr: u32, msg_len: u32) -> noreturn`

raise a moof-shape error. `kind_handle` is a Symbol handle (typically
obtained via `moof_intern`). `msg_ptr`/`msg_len` is a utf-8 byte
slice in wasm linear memory; copied into a moof String. control does
not return to wasm.

### `moof_make_string(ptr: u32, len: u32) -> u32`

allocate a moof-heap String from utf-8 bytes at `ptr`/`len` in wasm
linear memory. returns a handle. the bytes are copied during the
import call; wasm may free its buffer immediately after.

### `moof_make_bytes(ptr: u32, len: u32) -> u32`

allocate a moof-heap Bytes from raw bytes at `ptr`/`len`. returns a
handle. byte ordering and meaning is opaque to moof — Bytes is a
transparent byte-buffer type.

### `moof_string_text(handle: u32, buf: u32, cap: u32) -> u32`

copy the utf-8 bytes of a moof String (referenced by `handle`) into
wasm linear memory at `buf`, capped at `cap` bytes. returns the
ACTUAL length (which may exceed `cap`; if so, only `cap` bytes were
written and the wasm side should re-allocate and retry).

### `moof_bytes_data(handle: u32, buf: u32, cap: u32) -> u32`

same as `moof_string_text` but for Bytes handles.

### `moof_intern(ptr: u32, len: u32) -> u32`

intern a Symbol from utf-8 bytes. returns a Symbol handle.

## exports

each method on the mco's proto is a wasm export named `<selector>`
(with selector colons replaced by underscores; e.g., `seedFrom:`
exports as `seedFrom_`). signature shape:

- arg types: `i32`, `i64`, `u32` (handle)
- return type: `u32` (handle) for non-primitive returns; `i64` for int
  returns; `void` for procedures

signature mismatch (more args declared than the wasm function
accepts, or wrong return type) raises `'arity-mismatch` at load time.

## error model

`moof_raise` traps wasmtime with a structured payload. the substrate's
trampoline catches the trap, drains the handle table, and converts to
a moof RaiseError. user code sees it as a normal `[try …]` /
`[catch: …]` candidate.

## per-language bindings

- **zig**: `lib/mcos/_lib/moof.zig` — extern declarations + ergonomic helpers
- **c**: `lib/mcos/_lib/moof.c` (header + tiny static inline)
- **ocaml**: `lib/mcos/_lib/moof.ml` — uses wasm_of_ocaml's externs
- **haskell**: `lib/mcos/_lib/moof.hs` (when ghc-wasm is functioning)

each binding implements the imports/exports surface defined above. the
binding is what mco authors `import`/`require`; this doc is what the
binding implements against.
