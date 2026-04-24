# image round-trip gap

> diagnostic finding while preparing source preservation:
> `Store::load_all` exists but is never called. image save works;
> image load doesn't. every boot starts from a fresh heap and
> re-runs bootstrap. parking this for a dedicated wave.

---

## what's there

`crates/moof-runtime/src/store.rs` has `save_all` and `load_all`
fully implemented:

- `save_all(heap, closure_descs)` writes the whole heap (objects +
  symbols + env_id + closure descs) to an lmdb store.
- `load_all(heap)` reads the same data back into `LoadedImage`.

save is called on interface exit from `System::save_image`. load
is called from… nowhere. `grep -n 'load_all\|store.load\|load_image'
crates/moof-*/src/` turns up only the method definition.

## what this means

- interactive repl state (defs, workspace state) does not survive
  restart. the "image" you see on boot is whatever the bootstrap
  sources produce, not whatever you last quit with.
- capability-vat state similarly doesn't persist; they're reborn
  on each boot from the plugin's setup() call.
- source preservation, just landed in wave 7.2, writes source
  records into the store via `SerializableClosureDesc.source`,
  but without load the data sits there.

save works. load is unwired. the whole persistence story is a
mirage until we close the loop.

## why it was never wired

i suspect: loading the image while *also* re-running plugin
registration and bootstrap would double-register everything and
likely corrupt the heap — plugins create new prototype objects
every boot, while the loaded image has its own prototypes with
symbol-id-encoded handler references.

making this work requires decisions:

- **symbol preservation.** if we load persisted symbols FIRST,
  then run plugin registration, `intern(&unique)` reuses matching
  ids and natives land at stable indices. handlers that reference
  native symbols from the loaded image remain valid.
- **prototype deduplication.** plugin register() creates fresh
  type-proto heap objects; the loaded image has its own. one
  has to win. simplest: load overrides — the persisted image is
  authoritative for prototypes; plugin registration only fills in
  native handler symbols.
- **closure desc continuity.** loaded descs have code_idx values
  compiled against the old desc pool. rebuilding a VM that honors
  those indices is doable (restore closure_descs from the image,
  don't re-create at startup).
- **capability vat ids.** FarRefs in the saved repl heap point
  to caps by `(vat_id, obj_id)`. if capability spawn order is
  deterministic, ids match — but that's fragile. the url/resolver
  work from `docs/addressing.md` fixes this properly.

## what we do instead, for now

- source preservation writes records into descs so the save
  format is correct *for when we wire load*.
- the repl experience is "bootstrap every time" — fine for
  development, not yet a persistent medium.
- `reload:` (moof-side helper in `lib/system/source.moof`) lets
  users re-evaluate a file's contents without restarting, which
  covers the daily authoring loop pending real persistence.

## sequencing

wire load in its own dedicated wave (probably wave 8), with these
sub-steps:

1. **load path bypasses plugin register() for prototype
   construction**, but runs it for native registration only. may
   require a split of `Plugin::register` into `register_natives`
   + `register_prototypes` or a flag.
2. **load symbols before prototype restoration**, so
   natively-registered symbols align with handler values.
3. **reconstruct closure_descs from image** instead of starting
   empty.
4. **verify bootstrap can be skipped entirely for loaded images**,
   or design bootstrap to be idempotent (defining something that
   already exists should be a no-op).
5. **integration test**: boot fresh → add state → quit → boot
   again → verify state.

once content-addressing lands (`docs/foundations.md` phase 1),
the capability-FarRef-restart story also gets cleaner: caps are
named URIs, not raw ids.

## decision

not blocking source preservation or eval-cap work. parked as
wave 8.0 with this doc as the reminder.
