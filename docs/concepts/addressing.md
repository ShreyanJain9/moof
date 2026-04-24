# addressing

**type:** concept
**specializes:** throughline 3 (walks), throughline 5 (canonical form)

> every value in moof has a URL. addressing is the surface of
> the walks throughline: every identity is a path through some
> graph of objects. content-addressed URLs walk the hash DAG;
> path URLs walk the namespace tree; FarRef resolution walks
> across vat boundaries; federation walks across machines. same
> pattern, different graphs.

---

## the deeper view

if you've read [throughlines.md](../throughlines.md), addressing
is the concrete face of throughline 3 (walks). every moof URL
is a SERIALIZED WALK through some graph:

| URL shape | graph walked | step |
|-----------|-------------|------|
| `moof:<hash>` | content-addressed DAG | hash → blob |
| `moof:/caps/X` | namespace tree | `[table at: segment]` |
| `moof:/vats/N` | vat registry | vat id → Vat |
| `moof:/protos/X` | prototype registry | name → proto |
| `moof:peer/alice/...` | peer federation graph | network hop |

and for comparison, walks that DON'T get their own URLs (yet)
but are structurally the same:

| walk | graph | step |
|------|-------|------|
| message dispatch | proto chain | `obj.proto` |
| env lookup | env parent chain | `env.parent` |
| delegation | same as dispatch | `obj.proto` |

URLs name the walks that cross persistence or trust boundaries.
internal walks (dispatch, env lookup) don't need URLs because
they never leave the current image/vat. but the structure is
shared — moof is full of walks, and addressing names some of
them for durability.

---

## URLs everywhere

moof URLs have the `moof:` scheme:

- `moof:<hash>` — immutable, content-addressed value.
- `moof:/caps/console` — the console capability in this image.
- `moof:/vats/7/objs/42` — object 42 in vat 7.
- `moof:/services/clock` — the clock service.
- `moof:/protos/Integer` — the Integer prototype.
- `moof:peer/alice/vats/12` — alice's vat 12 (federated, future).

URLs are moof values. they have a `URL` prototype. they carry a
scheme and a path. they round-trip through serialization.

```moof
(def u (URL "moof:/caps/console"))
u.scheme       ; 'moof
u.path         ; "/caps/console"
[system resolve: u]   ; → a FarRef to the console capability
```

---

## two kinds of addressing

**content addressing** — identity by hash. every immutable value
has one.
- `moof:bafy...` (256-bit BLAKE3 hash, base32-encoded)
- two identical values (same contents) have the same URL, always.
- you can hand someone a content URL; if they have a cache, they
  resolve it locally with no network.
- used for: serialized values, shared data, workspaces that travel.

**path addressing** — identity by location in a live namespace.
- `moof:/caps/console`, `moof:/vats/7`
- the same path can refer to different objects over time (the
  console capability is one vat today, possibly different after
  restart — but the URL still names "the console").
- paths are resolved through the System's namespace tree.
- used for: live references, FarRefs, capabilities.

together they cover both "this specific value frozen in time" and
"this current role in the live system."

---

## the namespace

moof's live namespace is a tree rooted at `/`, hosted by vat 0's
System:

```
/
├── caps/
│   ├── console
│   ├── clock
│   ├── file
│   ├── random
│   ├── system
│   └── evaluator
├── vats/
│   ├── 0/       ; vat 0 (init)
│   ├── 7/       ; your repl vat
│   │   ├── objs/42
│   │   └── namespace
│   └── 12/      ; some server
├── services/
│   ├── clock
│   ├── console
│   └── ...
└── protos/
    ├── Integer
    ├── String
    ├── Cons
    └── ...
```

navigation is a walk:

```moof
[system root]
  ; → { caps: {...} vats: {...} services: {...} protos: {...} }

[[system root] walk: "/caps/console"]
  ; → a FarRef to the console capability

[system resolve: (URL "moof:/caps/console")]
  ; → the same FarRef, via URL
```

the namespace is a value — a nested Table. anything that responds
to `at:` can participate. `walk:` is a generic method on Table.

---

## plan 9's commitments, applied

plan 9 said: everything is a file; every file has a path; per-
process namespaces assemble the file tree the process sees. moof
generalizes:

- everything is an object; every object has a path in the
  namespace tree.
- each vat has its own namespace, handed to it at spawn time (or
  will, once wave 10+ makes namespaces per-vat rather than
  global).
- namespaces compose via **mount** (plan 9 bind): `[ns mount:
  other at: 'remote]` — other's tree now appears under
  `/remote/*`.
- union mounts: `[ns mount: a at: 'x] mount: b at: 'x]` means
  lookups at `/x/foo` try `a` first, then `b`.

the namespace is a first-class moof value. you can save it, fork
it, send one to a friend. "here's the setup i use" becomes a
namespace URL you paste.

---

## resolution

**resolving a URL**: given a URL, produce a value (usually a
FarRef or a cached immutable value).

today:
- `moof:<hash>` — look up in the blob store. if present, return
  the hydrated value. if not, fail (until wave 11+ fetches from
  peers).
- `moof:/caps/<name>` — pattern-match against the System's
  capability registry. return the current FarRef.
- `moof:/vats/<id>/objs/<obj>` — construct a FarRef to that
  specific (vat, obj) pair, if the vat is alive.
- `moof:/services/<name>` — look up in the service registry (wave
  9.4+), return the service's FarRef.
- `moof:/protos/<name>` — look up in the prototype registry.

resolution is an operation on System (`[system resolve: url]`).
failure is explicit: an Err value if the path doesn't point at
anything.

---

## URLs and persistence

URLs survive restart because they're not pointers — they're names.

example: a capability FarRef has a URL stored on it:

```moof
a FarRef {
  __target_vat: 3       ; session-local
  __target_obj: 17      ; session-local
  url: "moof:/caps/console"
}
```

the (vat, obj) pair changes every restart (vats get new IDs). the
URL is stable. when the image reloads, we walk every FarRef and
re-resolve its URL to get fresh (vat, obj) coordinates. the user
sees no change; the wiring re-establishes.

this is what lets moof promise "your references survive reboot."
the URL is the identity; the (vat, obj) is cache.

see [persistence.md](persistence.md) for the full save/load cycle.

---

## federation (future)

one URL scheme, local or remote:

```
moof:/caps/console              ; in THIS image
moof:peer/alice/caps/console    ; in alice's image
moof:peer/alice/vats/12/objs/7  ; a specific object in alice's vat 12
```

the `peer` segment names a peer by trust-anchored identity. sending
a message to `alice/vats/12` routes the message over the network;
the reply comes back; FarRef semantics are preserved.

content-addressed URLs are easier: `moof:<hash>` resolves the same
way regardless of source. when alice shares `moof:<hash>`, and i
don't have it locally, i fetch it from alice (or from any peer who
has it — it's the same hash either way).

federation plumbing is designed but not implemented. the URL scheme
and FarRef architecture already support it.

---

## what "capability" means in URLs

every capability FarRef carries a URL at grant time. that URL is
the capability's **stable name**:

```
a Console FarRef {
  url: "moof:/caps/console"
}
```

if your vat holds this FarRef, you have the capability. the URL is
a bearer token in a weak sense (anyone who holds the object holds
the permission).

revocation happens by severing the name's binding: System removes
`console` from its `/caps/` table. existing FarRefs now fail to
resolve. this is the capability security model expressed through
namespace mutation.

see [capabilities.md](capabilities.md) for the full security
picture.

---

## why unify

three distinct concepts collapse into one vocabulary:

- **lookup** (find me this thing) — walk the namespace.
- **persistence** (identify this thing for later) — URL in an
  object's slot.
- **federation** (refer to something elsewhere) — same URL syntax,
  with a `peer` prefix.

one vocabulary means: the same code that looks up a local
capability can look up a remote peer's capability. the same code
that identifies a value for caching can identify it for sharing.
the same gesture that navigates the local namespace (future: in
the canvas) navigates a remote peer's namespace.

this is plan 9's gift applied an abstraction layer up, with git's
content-addressing doing the dedup.

---

## what you need to know

- every moof value has a URL: content-addressed or path-addressed.
- the namespace is a tree rooted at `/`, hosted by System in vat 0.
- navigation is a `walk:` on a Table.
- FarRefs carry URLs; URLs survive restart via re-resolution.
- federation extends the scheme with `peer` segments; same
  semantics.
- one vocabulary unifies local lookup, persistence, and sharing.

---

## next

- [../throughlines.md](../throughlines.md) — walks + canonical
  form, the patterns this specializes
- [persistence.md](persistence.md) — how URLs and content-
  addressing intertwine with the blob store
- [capabilities.md](capabilities.md) — the security model built
  on URL-carrying FarRefs (reachability as a constraint)
- [messages.md](messages.md) — dispatch, the unlabeled walk
  through the proto chain
- [vats.md](vats.md) — what a FarRef really is
