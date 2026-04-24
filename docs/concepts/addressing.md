# addressing

**type:** concept
**specializes:** throughline 3 (walks), throughline 5 (canonical form)

> moof has ONE universal reference scheme. every persistent
> value has a URL. every URL is a walk through the same
> namespace. `/caps`, `/vats`, `/services` are conventions vat-0
> establishes — not hardcoded categories with special
> resolvers. a URL is a promise; resolution is how the promise
> is kept.

---

## the commitment

**every persistent value should have a universal, permanent
reference.** two commitments:

- **universal**: one URL scheme covers everything. local,
  persistent, live, federated, content-addressed, user-bound —
  all one vocabulary.
- **permanent**: the URL survives process restarts, image
  reloads, and (eventually) peer-to-peer migration. holding a
  URL today is holding the same URL tomorrow; what it resolves
  to might change, but the URL doesn't.

this isn't a hierarchy of special cases where `/caps/X` has
magic caps-logic and `/vats/N` has magic vat-logic. it's one
tree. the apparent categories are conventions.

---

## the model

there's one **namespace**: a tree of objects rooted at `/`. every
node is an object. every edge is a name. walking `/foo/bar/baz`
means: start at root, do `[root at: 'foo]`, do `[that at: 'bar]`,
do `[that at: 'baz]`. plain `at:` sends at each step.

a URL is a serialization of a walk. nothing more. the URL doesn't
know what it's going to find; the namespace's current state
decides.

```
moof:/foo/bar/baz    ; walk root's /foo/bar/baz
moof:/caps/console   ; walk root's /caps/console
moof:/my-things/thing3  ; the same kind of walk, just user-bound
```

there's no special handler for `/caps` vs `/my-things`. both are
segments whose resolution is decided by whoever maintains the
parent object. vat-0's root has `caps`, `vats`, `services`,
`protos` among other things; you can add more; you can
(carefully) take them away.

---

## content-addressing is one subtree

immutable values are addressed by content hash: `moof:<hash>`.
structurally this is a walk too — through a **hash-indexed
subtree** of the namespace, where the step is "hash → blob"
rather than "symbol → child."

```
moof:<hash>     ; walk the content-addressed subtree by hash
```

conceptually that's:

```
moof:/@content/<hash>
```

with `@content` being the content-addressed resolver-subtree.
the bare `<hash>` form is shorthand because this subtree is
very common. the resolver is different (BLAKE3 lookup into LMDB,
not `at:`-send into a table), but the pattern — "walk a
graph, return a value" — is the same.

**same URL scheme, different resolver-per-subtree.** no
hierarchy of special cases — just one tree with heterogeneous
resolvers at well-known points.

---

## federation is another subtree

cross-machine references use a `peer/<id>/...` prefix:

```
moof:peer/alice/caps/console    ; alice's console cap
moof:peer/alice/vats/12         ; alice's vat 12
```

this is a **network-backed subtree**: the resolver for `peer/alice`
is "make a request to alice's machine." but it's STILL a walk,
still the same URL scheme, still composable with the rest.

you could theoretically have `/peer/alice/my-things/thing3` and
it would resolve if alice has a `my-things/thing3` in her
namespace and has granted you read. no special federation
vocabulary beyond "this subtree goes over the wire."

---

## resolvers, not categories

the structure is:

- **one namespace**, tree of objects.
- **one URL scheme**, `moof:<walk>`.
- **multiple resolvers** at well-known subtrees, each choosing
  how to fetch its children:
  - **local object resolver** — `at:` sends through live objects.
    default; most of the tree.
  - **content-addressed resolver** — `<hash>` → LMDB lookup →
    deserialize. used for `@content` / bare-hash URLs.
  - **peer resolver** — network hop to another machine's
    namespace. used for `peer/<id>/...`.
  - **more resolvers are possible** — a `@git/<commit>` subtree
    could mount a git repo; a `@ipfs/<cid>` subtree could mount
    IPFS; a `@http/<url>` subtree could fetch web content. all
    just different ways to resolve a walk.

the resolver is a property of the subtree; when resolution
crosses into a new subtree, the relevant resolver takes over.

this is plan 9's union-mount model, applied to objects. you
mount a subtree under a prefix; the prefix's resolver handles
what's below.

---

## a URL is a promise

a URL names a walk. the walk might succeed (resolve to a
value) or fail (no such path, network down, capability
revoked). **the URL itself carries no content** — it's a
stable handle.

this is how URLs work on the web. `https://example.com/foo` is
a promise that the server at `example.com` might return
something for `/foo`. sometimes it does; sometimes 404; sometimes
500. the URL is the question; resolution is the answer.

moof URLs are the same. holding a URL doesn't mean holding the
value. holding a URL means being able to ASK for the value.
what you get depends on who maintains the namespace at that
point and what their resolver decides.

this is important for **capability security**: even if someone
knows the URL `moof:/caps/admin-power`, asking for it doesn't
mean getting it. the resolver — vat-0's System — decides
whether to return the capability. a URL isn't a bearer token;
a FarRef obtained by resolving a URL is a bearer token.

---

## conventions vat-0 establishes

vat-0's root starts with these conventional subtrees. none are
special-cased by the URL scheme; all are just bindings vat-0
makes at startup:

```
/                — root, maintained by vat 0
/caps            — capability FarRefs, named by symbol
/vats            — live vats indexed by id
/services        — registered services (wave 9.4+)
/protos          — named prototypes (Integer, String, etc.)
/env             — vat-0's environment bindings
/users           — per-user subtrees (future)
/peers           — federated peer subtrees (future)
/@content        — content-addressed resolver (internal, shorthand moof:<hash>)
/@snapshots      — historical root hashes (future, wave 11+)
```

these can be inspected, rebound, extended. a user can bind
their own `/my-things` at the root. another image might have
different top-level names. there's no "moof protocol says caps
must be at /caps." it's convention.

---

## what URLs guarantee

- **syntactic uniformity** — same scheme, same parse.
- **compositional walks** — longer paths walk further.
- **restart stability** — URLs don't change across restarts;
  the underlying (vat, obj) ids might, but FarRefs re-resolve
  via the URL on load.
- **federation readiness** — `peer/<id>/...` is a drop-in prefix.

---

## what URLs do NOT guarantee

- **resolution success** — the namespace might not have bound
  your path. 404.
- **permission** — the resolver might refuse. not a 404; a
  `permission denied`. same URL, different outcome.
- **stability over time** — the bindings at a path can change.
  `moof:/caps/console` today might not be `moof:/caps/console`
  in six months if you rebuild your system. content-addressed
  URLs DO guarantee stability (they're hashes), but live URLs
  are stable against accident, not against deliberate rebinding.

for unchanging identity, use content-addressing. for current
identity, use a path. different tools for different jobs — but
SAME SCHEME.

---

## URLs as values

URLs are first-class moof values. they have a `URL` prototype.

```moof
(def u (URL "moof:/caps/console"))
u.scheme       ; 'moof
u.path         ; "/caps/console"
[system resolve: u]   ; → FarRef, or Err if unresolvable
```

you can store URLs in slots, pass them as arguments, serialize
them, compare them, hash them. every persistent moof value
that wants to refer to another without "owning" it stores a URL
rather than a direct reference.

this is the foundation for **FarRefs carrying URLs**: a FarRef
stores both the current `(vat, obj)` coordinates AND the URL
that identifies its target. on image load, the URL is
re-resolved; the stale (vat, obj) is replaced with the fresh
one. to the user, nothing happened — the FarRef kept its
meaning.

---

## dispatch, env lookup, walks — same mechanism, different label

a message send walks the proto chain. env lookup walks the
parent chain. URL resolution walks the namespace. these share
structure; they differ in what graph and what edges:

| walk | graph | edges |
|------|-------|-------|
| dispatch | proto chain | `obj.proto` |
| env lookup | scope chain | `env.parent` |
| URL resolve | namespace tree | `[obj at: segment]` |
| content-addressed fetch | hash DAG | `hash → blob` |
| federation | peer graph | network hop |

walks are universal (throughline 3). URLs are the **serialized**
form of walks we want to persist, share, or transmit. walks we
don't persist (dispatch, env lookup) don't need URLs. the URL
scheme names a subset of walks — the ones that cross persistence
or trust boundaries.

---

## what you need to know

- one namespace, tree of objects, rooted at `/`.
- one URL scheme: `moof:<walk>`.
- `/caps`, `/vats`, `/services`, `/protos` are vat-0 conventions,
  not hardcoded special cases.
- content-addressing and federation are **resolver subtrees**,
  not parallel schemes.
- a URL is a promise; resolution is how it's kept.
- FarRefs carry URLs so they survive restart.
- URLs are values — first-class, composable, serializable.
- same URL, different resolver → different subtree behavior. the
  scheme doesn't care.

---

## next

- [../throughlines.md](../throughlines.md) — walks + canonical
  form, the patterns this specializes
- [persistence.md](persistence.md) — content-addressed resolver
  + blob store
- [capabilities.md](capabilities.md) — why holding the URL
  isn't the same as holding the authority
- [messages.md](messages.md) — dispatch, the unlabeled walk
  through the proto graph
- [vats.md](vats.md) — FarRefs and how URLs re-resolve
