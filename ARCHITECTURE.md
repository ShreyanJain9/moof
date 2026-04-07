# Architecture

moof is a shared data substrate. not a programming language.

the objectspace is a server. languages are frontends. every connection is
a vat. every operation is a send.

## Crates

```
moof-fabric/    the objectspace kernel
moof-server/    generic fabric host + IO system vats
moof-lang/      the moof language (one extension among many)
src/bin/         server + repl binaries
```

### moof-fabric (~900 lines)

the kernel. knows about objects, messaging, scheduling. no language.

- **value.rs** — `Value` (Nil/True/False/Integer/Float/Symbol/Object) and `HeapObject` (Object/Cons/String/Bytes/Environment)
- **heap.rs** — arena allocator, symbol table, slot/handler/env operations
- **dispatch.rs** — `send()`: handler lookup → delegation → type protos → universal protocol → DNU. `HandlerInvoker` trait for pluggable handler execution.
- **vat.rs** — `Vat`, `Message`, `Scheduler`. round-robin dispatch.
- **native.rs** — `NativeInvoker`: handlers as named Rust closures.
- **wire.rs** — binary protocol for fabric operations (connect/send/create/slot-get/etc)
- **persist.rs** — bincode save/load of the heap

### moof-server (~300 lines)

a running fabric instance. frontends connect as vats.

- **lib.rs** — `Server` struct, system vats, auth tokens, connection management, virtual vats for external interfaces.
- **extension.rs** — dylib loading via `moof_extension_init` C ABI entry point.
- **io.rs** — Console/Filesystem/Clock system vat handlers.

the server is generic. it loads language extensions as dylibs at runtime.
`moof-lang` is one such dylib. the server doesn't know about any language.

### moof-lang (~2700 lines)

the moof language shell. compiles as both rlib and cdylib.

- **lexer.rs** — tokenizer (s-expressions, brackets, braces, keywords)
- **parser.rs** — cons-cell AST builder, block syntax `{:x expr}`
- **compiler.rs** — AST → bytecode, stored as Object{code: Bytes, constants: list}
- **opcodes.rs** — bytecode instruction set
- **interpreter.rs** — `BytecodeInvoker` implements `HandlerInvoker`. stateless — all execution state on the Rust stack.
- **conventions.rs** — type prototypes (Integer +, String length, etc.)

exports `moof_extension_init` for dylib loading. when loaded:
1. registers BytecodeInvoker + NativeInvoker with type conventions
2. binds system vat capabilities (Console, etc.) into root env
3. loads bootstrap from lib/bootstrap.moof
4. registers eval hook for remote evaluation

### Binaries

- **moof-server** — boots fabric, loads dylib extensions, listens on unix socket. operator approves connections.
- **moof-repl** — thin client. connects to server socket, sends eval: messages, prints results.

## The protocol

clients talk to the server over a binary wire protocol:

```
client → server:
  Connect { token }
  Send { receiver, selector, args }
  Create { parent }
  SlotGet { object, slot }
  SlotSet { object, slot, value }
  Intern { name }
  Disconnect

server → client:
  Connected { vat_id, capabilities }
  Ok(value)
  Error(message)
  Created(id)
  Interned(id)
```

values are binary-encoded: tag byte + payload. strings in Send args
are `WireArg::Str` — the server allocates them in the heap on arrival.

## Flow

```
Terminal 1:                     Terminal 2:
$ moof-server                   $ moof-repl
  loading libmoof_lang.dylib      connecting to /tmp/moof.sock...
  bootstrap loaded                [waiting for approval]
  listening: /tmp/moof.sock
  approve? [y/n] y                Connected as vat 3
                                  (Console, Filesystem, Clock)
  [vat 3] eval => 3
                                  moof> [1 + 2]
                                  => 3
```

## Capability model

a capability IS a reference to an object. if you have the reference, you
can send messages to it. if you don't, the object doesn't exist in your
world. no permissions, no roles — just references.

system vats (Console, Filesystem, Clock) are objects. the server decides
which references to hand to a connecting vat based on auth tokens.

## Extension model

extensions are cdylib files exporting `moof_extension_init(server: *mut Server)`.
loaded at startup via `--load path.dylib`. each extension can:
- register HandlerInvokers (language shells)
- create system vats (IO, FFI, network)
- register native handlers on type prototypes
- load bootstrap code
- register an eval hook
