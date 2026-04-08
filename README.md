# MOOF v2: Moof Open Objectspace Fabric

> *"clarus the dogcow lives again"*

a ground-up rewrite of the moof objectspace. same soul, better bones.

see [SYNTHESIS.md](SYNTHESIS.md) for what v1 was and [VISION.md](VISION.md) for what v2 aims to be.

the v1 codebase is preserved on the `archive/v1` branch and tagged `v1-final`.

## architecture

```
crates/
  fabric/    the substrate: objects, messaging, LMDB persistence (~800 lines)
  lang/      the moof language: lexer, parser, compiler, VM (~1500 lines)
  shell/     the interactive surface: repl, inspector, mcp (~400 lines)
```

the fabric is language-agnostic. moof-lang is one frontend. the fabric doesn't know about bytecode, ASTs, or s-expressions.

## status

v2 is in early development. the skeleton compiles, the NaN-boxed value type works, the lexer and parser produce cons-cell ASTs, and the register-based VM executes basic bytecode. the LMDB-backed object store is functional.

what's next: wiring the compiler to the VM, bootstrapping the kernel forms, and reaching REPL parity with v1.

## quick start

```bash
cargo run
```

## license

MIT
