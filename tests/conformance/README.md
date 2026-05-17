# Conformance Test Corpus

Per the design spec §13.10, every player must pass these triples.

## Structure

- `<name>.json` — manifest of (image, send, expect-value, expect-stdout) triples
- `<name>/` — directory with .vat fixtures referenced by the manifest

## Running

Pending implementation; tracked as a future-phase concern. For now,
hand-verify each triple via REPL.

The eventual runner: `moof conform <manifest.json>` per spec §1.3.

## Current manifests

### freezing.json

Four triples covering the freeze primitive surface (phase 1/B):

1. `freeze-and-frozen-query` — freeze then frozen? returns #true
2. `mutation-after-freeze-raises` — slot mutation after freeze raises 'frozen-form
3. `let-mutable-result-is-frozen` — let-mutable auto-freezes at scope exit
4. `freezable-on-foreign-handle-returns-false` — ForeignHandle is a live face

To hand-verify triple 1 via REPL:

```
MOOF_LIB=$PWD/lib ./players/zig/zig-out/bin/moof eval -
(let ((p [Object new])) [p freeze] [p frozen?])
```

Expected: `#true`
