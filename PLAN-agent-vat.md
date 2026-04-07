# Plan: Agent Vat — Claude as a Moof Citizen

## The vision

Claude (or any AI agent) lives INSIDE the moof image as a first-class participant.
Not "Claude talks to moof over a pipe." Claude IS a vat. Its tools are moof objects.
Its memory is moof objects. Its actions are message sends, auditable through the
same infrastructure as any other vat.

```
┌─────────────────────────────────────────────┐
│ Runtime                                      │
│                                              │
│  ┌────────┐  ┌────────┐  ┌──────────────┐  │
│  │ Vat 0  │  │ Vat 1  │  │   Vat 2      │  │
│  │ REPL   │  │  MCP   │  │  Agent/Claude │  │
│  │        │  │        │  │              │  │
│  │ stdin  │  │ jsonrpc│  │  tools =     │  │
│  │ stdout │  │ stdio  │  │   facets of  │  │
│  │        │  │        │  │   image objs │  │
│  └────────┘  └────────┘  └──────────────┘  │
│       ↕           ↕             ↕           │
│  ┌──────────────────────────────────────┐   │
│  │              Shared Heap              │   │
│  └──────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

## The architecture

### Layer 1: MCP Extension (Rust)

An `McpExtension` implementing `MoofExtension`. Runs the JSON-RPC protocol over
stdio (or later, HTTP). When a tool call arrives:

1. The extension receives the JSON-RPC request
2. Parses it, looks up the tool name in a registry
3. Creates a Message for the agent vat: `[tool call: args]`
4. Enqueues it on the agent vat's mailbox
5. The scheduler delivers it on the agent vat's next turn
6. The result resolves a Promise
7. The extension serializes the result as JSON-RPC response

### Layer 2: Agent Vat (Moof)

A vat created at startup (or on first MCP connection). It has:
- Its own root environment (child of the global root)
- Facet references to image objects (not direct references — sandboxed)
- A set of registered tools, each mapping to a moof handler

```moof
(def agent-vat (spawn (fn ()
  ; The agent's world: tools are facets of real objects
  (def tools (Assoc))
  [tools set: "eval" to: (Facet wrap: root-env allow: '(eval:))]
  [tools set: "modules" to: (Facet wrap: Modules allow: '(list named:))]
  [tools set: "inspect" to: (Facet wrap: root-env allow: '(lookup:))]
  ; ... more tools
)))
```

### Layer 3: Tool Registry (Moof)

Tools are moof objects with `describe`, `interface`, and `call:` handlers.
The MCP server walks the tool registry and exposes them as MCP tools.
When a tool call arrives, it's a message send to the tool object.

```moof
(def EvalTool { Object
  name: "eval"
  describe: () "Evaluate a moof expression in the image"
  interface: () { input: { expression: "string" } }
  call: (args) [root-env eval: (read [args get: "expression"])]
})
```

### Layer 4: Claude Code Integration

Claude Code connects to the moof MCP server. The skill we just built
(`/moof`) currently pipes expressions through cargo run. With the MCP server:

1. Claude Code connects to the moof MCP server as a client
2. Tool calls go directly to the agent vat
3. Responses come back as structured data, not grep'd text
4. The agent vat can maintain state across calls (it's a vat!)
5. The image records every tool call in the ChangeLog

## What needs to be built

### Step 1: MCP Extension (Rust)

New file: `src/vm/mcp_ext.rs`

```rust
pub struct McpExtension {
    vat_id: Option<u32>,
    // stdin/stdout handles for JSON-RPC
    // pending requests (waiting for vat turn results)
}

impl MoofExtension for McpExtension {
    fn name(&self) -> &str { "mcp" }

    fn register(&mut self, vm: &mut VM, root_env: u32) {
        // Create the agent vat
        // Register default tools
    }

    fn poll(&mut self, timeout: Duration) -> Vec<ExtensionEvent> {
        // Read JSON-RPC from stdin (non-blocking)
        // Parse requests
        // Create events targeting the agent vat
    }
}
```

### Step 2: JSON-RPC Protocol (Rust)

Parse MCP JSON-RPC messages. We already have a moof-level JSON parser,
but for the MCP extension we want Rust-level parsing (serde_json) for
reliability and performance.

Messages:
- `initialize` → return capabilities
- `tools/list` → walk tool registry, return tool schemas
- `tools/call` → enqueue message on agent vat, return result

### Step 3: Tool Objects (Moof)

Define tool prototypes in moof:

```moof
(def Tool { Object
  name: nil
  description: nil
  inputSchema: nil
  call: (args) nil  ; override in concrete tools
})

(def ToolRegistry { Object
  tools: (Assoc)
  register: (tool) [tools set: [tool slotAt: 'name] to: tool]
  list: () [tools values]
  get: (name) [tools get: name]
})
```

### Step 4: Default Tools

The tools Claude gets out of the box:

- **moof_eval** — evaluate a moof expression, return the result
- **moof_modules** — list modules and their definitions
- **moof_inspect** — inspect an object (describe, interface, slots)
- **moof_source** — get the source of a definition or module
- **moof_define** — define a new binding (or update with <-)
- **moof_send** — send a message to an object
- **moof_spawn** — create a new vat

### Step 5: Capability Wrapping

The agent vat gets Facets, not direct references:

```moof
; Agent can read anything but only write to designated objects
(def agent-image-facet
  (Facet wrap: root-env
    allow: '(lookup: eval:)))  ; read + eval, no define:to: or set:to:

; Agent can list and inspect modules but not modify them
(def agent-modules-facet
  (Facet wrap: Modules
    allow: '(list named:)))
```

Writes go through a review mechanism:
1. Agent proposes a change (eventual send)
2. Change is queued as a pending action
3. Human reviews in the REPL or Browser
4. Human approves → change commits
5. Human denies → change discarded

### Step 6: Claude Code MCP Client Config

In `.claude/settings.json` or similar:

```json
{
  "mcpServers": {
    "moof": {
      "command": "cargo",
      "args": ["run", "--", "--mcp"],
      "cwd": "/path/to/moof"
    }
  }
}
```

Then Claude Code has direct tool access to the moof image.
No more `printf | cargo run | grep`. Just tool calls.

## The beautiful part

Once this exists:
- Claude is a moof citizen. Its tool calls are message sends.
- The ChangeLog records everything Claude does.
- Capability security limits what Claude can access.
- Claude can spawn sub-vats for speculation.
- The human watches in the Browser and approves/denies.
- It's the §8 vision from the design doc, realized.

## Build order

1. Rust MCP extension (McpExtension + JSON-RPC parsing)
2. Tool objects in moof (Tool, ToolRegistry prototypes)
3. Default tools (eval, modules, inspect, source, define, send)
4. Capability wrapping (Facets on agent vat)
5. Claude Code MCP client config
6. Review/approval mechanism for writes

Steps 1-3 are one session. Steps 4-6 are a second session.
