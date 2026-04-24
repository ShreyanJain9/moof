# capabilities

**type:** concept

> moof's security model. a reference IS a capability. if you hold
> a reference to an object, you can send it messages; if you don't,
> you can't touch it. no ambient authority. no global namespace.

---

## the rule

**authority equals reachability.** a vat can only do what its
references let it do. if the vat has no FarRef to Console, it
cannot print — not "shouldn't not," cannot, by construction.
there's no global `Console.println()` to call. there's only the
Console vat, and you reach it by holding a reference.

this is the **object-capability model** (E language's legacy). it's
how moof does security.

---

## why this is different from permissions

classical security models have **ambient authority**:

- unix processes inherit their parent's file access.
- browsers have origin policies with implicit cross-origin defaults.
- most languages have global imports: any code can `open("/etc/passwd")`.

the "permission" model asks: "should this code be allowed to do
X?" and trusts a central registry to answer. the central registry
inevitably grows bugs.

moof's capability model asks: "does this code HAVE the reference
required to do X?" and the only way to get the reference is to be
given it. the security property is structural, not decisional.
less to configure, more to reason about.

---

## in practice

when a vat spawns, it receives exactly the references its spawner
hands it. nothing more.

```moof
(def worker-vat
  (Vat spawn: (fn (console clock)
    [console println: (str "hi, the time is " [clock now])])
    with: (list console clock)))
```

the worker has `console` and `clock` references. it cannot touch
`file` — it was never given a reference. it doesn't even know
about `file`. it can't "escape" to look for file because there's
no global namespace to traverse.

if the worker spawns a child, the child gets exactly what the
worker hands it — usually a subset (**principle of least
authority**).

---

## capabilities vs regular objects

there's no type-level distinction. a "capability" is just an object
someone treats as authority. the Console vat is a capability
because:

- it's a vat (so referenced via FarRef)
- its messages cause effects in the real world (stdout)
- possessing the FarRef is the only way to trigger those effects

a Map object isn't a capability — nothing in it is effectful, so
holding a reference doesn't grant anything meaningful. but the
same object model applies to both.

this uniformity means: any user-defined vat with a FarRef handed
out is itself a capability for whatever it does. your defserver
vats naturally obey the same rules. you don't build security on
top of moof; moof IS security.

---

## capability vats

moof ships with capability vats for common effects:

| capability | what it does | granted to |
|------------|--------------|------------|
| `console` | stdout, stderr, println | repl, script, eval |
| `clock` | time, duration, monotonic | repl, script, eval |
| `file` | read/write filesystem | repl, script, eval |
| `random` | PRNG, entropy | repl, script, eval |
| `system` | introspect vats, caps, services | repl, script, eval |
| `evaluator` | parse + eval moof source | repl, script, eval |

each is a vat loaded from a dylib at startup. the System grants
them to interface vats (repl, script runner) based on a manifest.

a user-defined server (defserver) is also a capability vat — its
owner hands out FarRefs; holders can use it.

---

## URLs are capabilities (with a caveat)

a FarRef carries a URL:

```
{ __target_vat: 3 __target_obj: 17 url: "moof:/caps/console" }
```

in a sense the URL **is** the capability. if you know the URL,
and the System will resolve it for you, you can reach the
capability.

but the URL is a NAME. you still need the System's cooperation to
resolve it. the System's `resolve:` handler can refuse ("you don't
have permission to name this"). today resolution is liberal — any
caller can resolve any URL they name. in a hardened future, the
System consults an ACL.

this is why we say "capability ≈ reference" rather than "capability
= URL." the URL tells you HOW to reach it; the reference is the
proof that you ARE reached. today those collapse; tomorrow they'll
diverge cleanly.

---

## membranes

a **membrane** is a proxy object that sits between a caller and a
capability, intercepting every message. it can:

- **log** — record all sends for audit
- **allow** — pass through
- **deny** — return an error, don't forward
- **transform** — rewrite the message (change args, restrict
  selectors)
- **revoke** — one-shot: the membrane unbinds, future sends fail

```moof
(def safe-file
  (membrane-around file
    allow-selectors: (list 'read:)     ; only reads, no writes
    log-to: audit-log))
```

`safe-file` responds like `file` but only to read selectors. any
other send is rejected. a vat holding `safe-file` cannot write to
disk even if it tries.

membranes are how moof does **attenuation** — hand someone a
narrower version of a capability. you grant the agent a
filesystem reference that only sees `/tmp`, for instance.

membranes don't exist in the implementation yet (wave TBD). the
design is well-understood.

---

## revocation

some capabilities need to be takeable-back. E's answer: wrap the
capability in a one-shot proxy that unbinds its inner reference
when revoked. any send through it goes to the proxy, which checks
the unbound flag and errors if so.

moof will use the same pattern via membranes. revocation becomes:

```moof
(def t (revocable file))      ; t.cap is the wrapped ref
                              ; t.revoke is the unbinder
; ... give `t.cap` to a vat ...
; ... later ...
[t revoke]                    ; t.cap stops working immediately
```

today this is design, not implementation. when membranes land,
revocation lands with them.

---

## the audit trail

every grant is (or will be) an event the System records:

```
at T: vat 7 (repl) was granted 'console' (moof:/caps/console)
at T: vat 7 was granted 'clock'
at T+1h: vat 7 spawned vat 9, granted subset {'console'}
```

this log is **the** security artifact — an append-only record of
who was given what, when, by whom. revocations are also events.
the capability history is as much a first-class artifact as the
value history.

today the grant matrix lives rust-side in System. wave 10+ moves
it into moof: System becomes a proper defserver with a grant
table as a slot, grants are Updates, the log is a message stream.

---

## the trust-anchored identity model (future)

for federation, we need a way to say "this URL came from alice."
the answer: **signatures**.

- every peer generates a keypair on first use.
- values (or URLs) can be signed.
- trust is per-signer, configurable by the user: "i trust alice
  to name moof:/caps/* on her machine."

this isn't "verified checkmarks." each user decides whose signer
identities to trust. capability delegation across the network
works by "alice signs a URL, you trust alice, so you accept the
URL as legitimate."

also not implemented. also well-understood.

---

## what you need to know

- authority = reference. hold the FarRef, you can send; don't
  hold it, you can't.
- capability vats wrap native effects (console, clock, file,
  random, system, evaluator).
- user-defined servers with external references are capabilities
  for whatever they do.
- membranes attenuate: narrower wrappers around broader
  capabilities.
- revocation: one-shot proxies that unbind on command.
- federation extends this across peers via signed URLs.

---

## next

- [vats.md](vats.md) — the isolation boundary capabilities are
  built on.
- [addressing.md](addressing.md) — URLs and the namespace tree.
- [effects.md](effects.md) — what "doing something" actually
  looks like when you hold a capability.
