# addressing

> how moof names things. urls for identity, resolvers for reach,
> farrefs for the runtime cache. this doc parks the design for
> the next real wave.

---

## the trilemma we're escaping

moof has had three ways to refer to things, each bad alone:

1. **FarRefs** — transient `(vat_id, obj_id)` pairs. they're real
   runtime handles (the thing a message send uses), but they don't
   survive restart. the same semantic capability gets a new
   `(vat_id, obj_id)` every boot. you can't persist a FarRef in
   the image and expect it to mean anything later.

2. **Symbol names** — `"console"`, `"clock"`. stable across restart
   because the manifest pins them, but they're ambient global
   lookup. anyone with a string can try to reach anything. it's
   the "import antigravity" problem: names imply a namespace that
   isn't explicitly owned by anyone.

3. **Content hashes** — `sha256:abc123...`. stable, globally unique,
   pure. great for immutable values. useless for live, mutable
   resources that have ongoing state (a capability vat, a service,
   a peer).

we need all three jobs done by something that composes: **stable
identity**, **explicit reach** (no ambient lookup), **works for
both immutable values and mutable resources**.

---

## the proposal: moof URLs + a Resolver capability

two url shapes, one resolution protocol.

### shape 1: content-addressed immutable values

```
moof:<hash>
moof:b2/4fa8a2c1...
```

the url *is* the content hash. resolution is deterministic:
given the hash, look up the bytes, deserialize, return the value.
no network round-trip needed if the bytes are local; network
fetch if not. dedup is automatic — same bytes everywhere = same
url everywhere.

this is what makes "here's my workspace" a 70-character string.

### shape 2: namespaced mutable resources

```
moof:<path>
moof:/caps/console
moof:/vats/5
moof:/services/clock
moof:/peers/alice/caps/clock
moof:/image/snapshots/2026-04-22-18:30:12
```

paths are hierarchical (plan-9-style) and navigable. the root `/`
is owned by System. well-known subtrees:

| path                    | what lives there                                 |
|-------------------------|--------------------------------------------------|
| `/caps/<name>`          | capability FarRefs                               |
| `/vats/<id>`            | user vats spawned by System                      |
| `/services/<name>`      | long-running services (phase 2 of system.md)     |
| `/interfaces/<name>`    | registered interfaces (repl, script, ...)        |
| `/image/snapshots/<id>` | historical image snapshots                       |
| `/peers/<id>/*`         | federated remote tree (plan-9's union mount)     |
| `/env/<symbol>`         | current env bindings in some reference scope     |

this is a *notational* tree, not a filesystem. `[resolver resolve:
"moof:/caps/console"]` is the primitive. but inspectors render it
as a tree and users navigate it the same way.

### shape 3 (implied): transient runtime handles

```
ref:<token>
```

urls resolve to FarRefs (or immutable values) at runtime. the
resolver maintains the mapping; callers don't see `(vat_id,
obj_id)` unless they ask. when an image restarts, urls re-resolve
through the resolver; the FarRef they hand out is new but the
url hasn't changed. persistence stores urls. the runtime holds
FarRefs transparently.

a FarRef's rust representation can optionally carry its source
url; printing a FarRef could show the url rather than the raw
ids. the rendering becomes `<FarRef moof:/caps/console>` instead
of `<far-ref vat:1 obj:2>`. much friendlier.

---

## resolvers are capabilities, not ambient

the worst part of name-based lookup is that any code can lookup
any name. urls fix this only if **resolution requires a capability**.

```moof
; the root resolver knows about /caps, /vats, /services, /peers
[resolver resolve: "moof:/caps/console"]  → Act<FarRef>

; a sandbox resolver knows only what it was given
(let ((sandbox [resolver subTree: "/caps/clock"]))
  [sandbox resolve: "moof:/caps/console"]) ; → Err, not visible
```

the resolver is an object capability. if you don't hold one, you
can't turn a url into a FarRef. if you hold a *restricted*
resolver, you can only reach what it exposes.

the root resolver is owned by System. when System grants a
capability to an interface, the interface receives a resolver
handle whose reach is scoped to what System approved. sandboxing
becomes handing out a different resolver; no special code needed.

### composition

resolvers compose. a peer resolver is "the resolver alice
published, plus my local fallback." a federation mount is a
resolver that resolves `/peers/alice/*` through alice's published
resolver. the union-mount pattern becomes:

```moof
[myResolver mount: aliceResolver at: "/peers/alice"]
```

---

## implications for persistence

- **the image stores urls, not FarRefs.** a serialized handle is
  `moof:/caps/console`; deserialization goes through the root
  resolver, producing a live FarRef.
- **immutable values are content-addressed at storage time.**
  `moof:<hash>` round-trips without loss. the foreign-type
  `schema_version` story from `foundations.md` plus canonical
  serialization get us there.
- **federation is free.** a remote value at `moof:peer/alice/...`
  resolves on demand through the peer resolver, fetching bytes
  across the network. distributed references are "just" urls
  with a different resolver path.

---

## implications for capability security

the current model: each vat has capabilities bound by name in its
env. `env_def('console, console_farref)`. the env is the
capability namespace; shadowing one in env shadows it for that
vat's code.

the url model:
- each vat has access to a **resolver**, which it was granted
- the resolver exposes a subset of paths
- `[resolver resolve: "moof:/caps/console"]` fetches the FarRef
- if the path isn't in the resolver's reach, resolve fails
- `env_def` still binds *handles* (the FarRef after resolution),
  but the source of authority is the resolver, not the env

this also gives us **capability provenance**: every FarRef that's
handed out came from some resolver; we can audit "how did this
vat come to have this reference?"

---

## implications for the grants table

today:
```toml
[grants]
repl = ["console", "clock", "file", "random", "system"]
```

url form:
```toml
[grants]
repl = [
  "moof:/caps/console",
  "moof:/caps/clock",
  "moof:/caps/file",
  "moof:/caps/random",
  "moof:/caps/system",
]
```

more verbose on the page. but now the grants table can *also*
talk about non-cap resources:

```toml
[grants]
observer = [
  "moof:/vats/*",             # read any vat's public state
  "moof:/services/clock",     # the clock service itself
  "moof:peer/alice/canvas",   # alice's shared canvas, federated
]
```

the manifest becomes a capability graph, not just a name list.

---

## how we get there (sequenced)

### phase a: url value type

- a `URL` first-class value wrapping a parsed url (scheme, path,
  fragment, query)
- parser extends to read `moof:/...` as a literal producing a
  `URL` value
- `[url scheme]`, `[url path]`, `[url join: subpath]` handlers

moof-plugin or moof-core addition. ~1 week.

### phase b: resolver capability

- `moof-cap-resolver` — a capability with:
  - `resolve: url` → `Act<value>`
  - `mount: resolver at: path` → `Act<nil>`
  - `subTree: path` → a new bounded resolver
- the root resolver knows the well-known paths above
- System creates the root resolver and grants it to interfaces
  by default

~2 weeks.

### phase c: farref source tracking

- FarRef struct remembers the url it came from (when applicable)
- `[farref source]` → `URL` or nil
- inspector renders `<FarRef moof:/caps/console>`

~3 days.

### phase d: manifest uses urls

- manifest grants entries are urls, not names
- System.grants_for uses the resolver to convert urls to
  FarRefs; interfaces no longer see the raw name list

~1 week.

### phase e: image serialization uses urls

- FarRefs in persisted state store their source url, not
  `(vat_id, obj_id)`
- on image load, urls re-resolve through the root resolver,
  producing fresh FarRefs
- handles cap identity surviving restart

this is the payoff. ~2 weeks once content-addressing lands.

### phase f: federation

- peer urls (`moof:peer/alice/...`) route through a peer
  resolver over the network
- content-addressed values dedup across peers automatically
- subscription primitives on top (opt-in, service-level)

open-ended. phase 6 of system.md.

---

## open questions

- **query parameters.** do we need `moof:/services/clock?since=N`
  semantics? probably yes for log/event streams; defer until we
  hit a use case.

- **fragment identifiers.** `moof:<hash>#/slots/title` navigates
  inside a content-addressed value. useful for referencing part
  of a document. borrow the web's semantics.

- **case sensitivity.** be case-sensitive throughout. paths are
  symbols; symbols are. done.

- **versioning of schemas.** foreign-type serialization includes
  `schema_version`; the url hash includes those bytes too, so a
  v2 value has a different hash from a v1 even if "content"
  overlaps. correct behavior.

- **which resolver scopes resolve `/env/*`?** envs are per-vat;
  a resolver query of `/env/console` has to specify which vat.
  probably: `/env/<vat-id>/<symbol>` for explicit, or an
  implicit "my own vat" shortcut.

- **encoding in urls.** paths can contain arbitrary symbol
  characters; need percent-encoding or similar. sorrowfully yes.

- **what about non-moof systems?** we might want to treat http
  urls uniformly. probably: `http://...` is just another scheme
  the resolver knows about, resolving through a network capability.
  moof's object model is bigger than one scheme.

- **why not just use the web's url machinery?** honestly
  considered. the answer: we want resolvers to be capabilities,
  not ambient. the web's url resolution happens via a process's
  ambient network stack; that's the opposite of our model. we
  adopt url *shape* and *syntax* (standard enough) but wire
  resolution through our capability discipline.

---

## what this isn't

- **not a DNS replacement.** DNS maps names to ip addresses;
  moof urls map names to values. different layer.
- **not a filesystem.** paths are navigational, not stored on
  disk. the store is content-addressed blobs; the path tree is
  an overlay rendered by the resolver.
- **not a typed uri scheme registry.** `moof:` is our scheme;
  we don't try to standardize what anyone else does with urls.

---

## today vs. the future

today (as of wave 7.1): the `system` capability is our toe-dip.
its handlers read from slots that rust-side System writes; moof
code sends it messages to get caps + vats. it works but the
references it hands back are name-based strings inside value
structures, not urls.

future (waves 8+): urls replace names. the system cap *is* a
resolver. granting a capability to an interface is the act of
handing it a resolver whose reach includes that url. farrefs
become an implementation detail most users never see directly.

when we get there, the capability story is tight, the
persistence story is solved, and federation is nearly free
because we've been building on the right primitive from the
beginning.
