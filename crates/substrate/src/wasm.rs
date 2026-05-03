//! the wasm mco loader — the polyglot heart of the substrate.
//!
//! per `docs/reference/mco-format.md`, mcos are wasm modules
//! (with optional moof custom sections). loading an mco
//! instantiates the wasm module, allocates a fresh proto-Form,
//! and installs each declared method as a handler that wraps the
//! corresponding wasm export.
//!
//! this is the **minimum-viable polyglot** version. it skips:
//! - custom-section parsing (manifest comes from inferred exports
//!   for now; the `.mco` format with manifest+signature lands in
//!   the next iteration)
//! - signature verification
//! - dependency resolution
//! - linear-memory marshaling for non-scalar values
//!
//! what it DOES support:
//! - load a `.wasm` file from disk
//! - instantiate it via wasmtime
//! - allocate a fresh proto-Form
//! - install handlers for each function-export with `() -> i64`
//!   shape (the smallest useful method shape — clock-style)
//! - return the proto to moof
//!
//! a moof program does:
//! ```moof
//! (def Hello [$mco load: "examples/wasm-mcos/hello.wasm"])
//! [[Hello new] answer]   ;; → 42
//! ```
//!
//! load-time anonymity holds: the substrate doesn't know what the
//! mco's proto is "called". the moof program names it by `def`.

use std::sync::Arc;

use wasmtime::{Engine, Instance, Linker, Module, Store};
use wasmtime_wasi::preview1::WasiP1Ctx;
use wasmtime_wasi::WasiCtxBuilder;

use crate::form::Form;
use crate::value::Value;
use crate::world::{RaiseError, World};

/// Per-dispatch handle table. wasm-side u32 indexes into this Vec.
/// Allocated at dispatch entry; dropped at dispatch exit (including
/// via raise/trap). NEVER cached across dispatches.
pub struct HandleTable {
    slots: Vec<Value>,
}

impl HandleTable {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }
    pub fn push(&mut self, v: Value) -> u32 {
        let idx = self.slots.len() as u32;
        self.slots.push(v);
        idx
    }
    pub fn get(&self, h: u32) -> Option<&Value> {
        self.slots.get(h as usize)
    }
    pub fn take(&mut self, h: u32) -> Option<Value> {
        // Replace with a placeholder so handle indices stay valid.
        self.slots.get_mut(h as usize).map(|slot| std::mem::replace(slot, Value::Nil))
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn clear(&mut self) {
        self.slots.clear();
    }
}

impl Default for HandleTable {
    fn default() -> Self { Self::new() }
}

/// per-mco state: the wasmtime engine + instantiated module +
/// store. the store carries a WasiP1Ctx so mcos compiled for
/// `wasm32-wasi` can access standard system services (time, fs,
/// stdin/stdout, etc) through `wasi_snapshot_preview1` imports.
///
/// parking the whole shape in a single Form's `:wasm-instance`
/// foreign-handle slot would be cleaner (per L6, "nothing the
/// substrate knows is hidden"); for now we keep a side table on
/// the World, indexed by proto-FormId. graduates to ForeignHandle
/// when the mco-format pipeline lands properly.
pub struct WasmInstance {
    pub _engine: Arc<Engine>,
    pub _module: Module,
    pub instance: Instance,
    pub store: Store<WasiP1Ctx>,
}

/// load a `.wasm` file from disk, instantiate it, return a fresh
/// proto-Form whose handlers wrap the wasm exports.
///
/// this is the substrate-internal entry. moof code reaches it via
/// the `[$mco load: path]` cap (see intrinsics.rs).
pub fn load_wasm_mco(world: &mut World, path: &str) -> Result<Value, RaiseError> {
    let bytes = std::fs::read(path).map_err(|e| {
        RaiseError::new(
            world.intern("io-error"),
            format!("could not read mco at `{}`: {}", path, e),
        )
    })?;
    load_wasm_bytes(world, &bytes, path)
}

/// load wasm bytes (already in memory) and instantiate. used by
/// `load_wasm_mco` and tests that embed wasm via `include_bytes!`.
pub fn load_wasm_bytes(
    world: &mut World,
    bytes: &[u8],
    label: &str,
) -> Result<Value, RaiseError> {
    let engine = Arc::new(Engine::default());
    let module = Module::from_binary(&engine, bytes).map_err(|e| {
        RaiseError::new(
            world.intern("wasm-load-error"),
            format!("`{}` is not a valid wasm module: {}", label, e),
        )
    })?;

    // build a WASI ctx — the wasm-side `clock_time_get`, `fd_write`,
    // etc resolve through here. mcos compiled for `wasm32-wasi`
    // get standard system access "for free." moof's own imports
    // are namespaced separately under the "moof" wasm module.
    let wasi = WasiCtxBuilder::new()
        .inherit_stderr() // dev: inherit so panics show up
        .build_p1();
    let mut store: Store<WasiP1Ctx> = Store::new(&engine, wasi);

    // build the import linker. wasi first (the standard), then
    // moof-specific imports (substrate-native primitives).
    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    wasmtime_wasi::preview1::add_to_linker_sync(&mut linker, |c| c).map_err(|e| {
        RaiseError::new(
            world.intern("wasm-link-error"),
            format!("wasi linker setup failed: {}", e),
        )
    })?;
    install_moof_imports(&mut linker).map_err(|e| {
        RaiseError::new(
            world.intern("wasm-link-error"),
            format!("substrate imports failed: {}", e),
        )
    })?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| {
            RaiseError::new(
                world.intern("wasm-instantiate-error"),
                format!("instantiating `{}` failed: {}", label, e),
            )
        })?;

    // allocate a fresh proto-Form. parent = Object. no `:name` meta:
    // load-time anonymity per `docs/reference/mco-format.md`. the
    // moof code that called `[$mco load:]` decides what to bind it
    // as.
    let proto_form = Form::with_proto(Value::Form(world.protos.object));
    let proto_id = world.alloc(proto_form);

    // ── manifest parsing ────────────────────────────────────────
    //
    // per `docs/reference/mco-format.md`, an `.mco` file is a
    // wasm module with custom sections holding moof-specific
    // metadata. the `moof.manifest` section, when present, is
    // moof source-text — parseable by the substrate's reader.
    // it declares which methods this mco exposes, the parent
    // proto, the abi version, etc.
    //
    // when no manifest is present (raw `.wasm` dev case), we fall
    // back to inferring from wasm exports (the MVP behavior).
    let manifest = parse_manifest_section(world, bytes, label)?;

    // discover exported functions. each function with `() -> i64`
    // shape is eligible. the manifest (if present) cross-checks:
    // declared methods MUST be a subset of the exports, with
    // matching shape.
    let all_exports: Vec<(String, ExportShape)> = module
        .exports()
        .filter_map(|exp| match exp.ty() {
            wasmtime::ExternType::Func(ft) => {
                let params = ft.params().collect::<Vec<_>>();
                let results = ft.results().collect::<Vec<_>>();
                if params.is_empty()
                    && results.len() == 1
                    && matches!(results[0], wasmtime::ValType::I64)
                {
                    Some((exp.name().to_string(), ExportShape::NoArgsToI64))
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();

    // if a manifest was found, only install methods it declared.
    // cross-validate: every declared method must exist as a wasm
    // export with the expected shape.
    let exports: Vec<(String, ExportShape)> = if let Some(m) = &manifest {
        let mut chosen = Vec::with_capacity(m.methods.len());
        for declared in &m.methods {
            match all_exports.iter().find(|(n, _)| n == declared) {
                Some((n, shape)) => chosen.push((n.clone(), *shape)),
                None => {
                    return Err(RaiseError::new(
                        world.intern("mco-manifest-mismatch"),
                        format!(
                            "`{}` declares method `{}` but the wasm \
                             module has no matching export",
                            label, declared
                        ),
                    ));
                }
            }
        }
        chosen
    } else {
        all_exports
    };

    // stash the wasm instance in a side table indexed by proto-id.
    // each handler closure captures `proto_id` and looks up the
    // instance at dispatch time.
    world.wasm_instances.insert(
        proto_id,
        WasmInstance {
            _engine: engine,
            _module: module,
            instance,
            store,
        },
    );

    // install handlers for each export. each handler is a *generic*
    // native fn — it receives the selector via the dispatched
    // selector at call time. but install_native takes a fn pointer,
    // so we install a single trampoline keyed by the selector name.
    //
    // since we can't capture per-export name in a fn pointer, we
    // store a (proto_id, selector) → export-name map in the world,
    // and the trampoline looks up its own (proto_id, selector) pair.
    for (export_name, shape) in &exports {
        // selector = export name (verbatim). the user calls
        // `[instance answer]` → selector "answer" → wasm export
        // "answer".
        let sel_id = world.intern(export_name);
        world
            .wasm_export_map
            .insert((proto_id, sel_id), (export_name.clone(), *shape));
        world.install_native(proto_id, export_name, wasm_method_trampoline);
    }

    Ok(Value::Form(proto_id))
}

/// walk a wasm binary's section headers; if a custom section
/// named `name` is found, return its payload (the bytes AFTER the
/// name). returns None if absent or if parsing fails partway —
/// the loader treats that the same as "no manifest" and falls
/// back to inferring exports.
///
/// wasm format reminder:
///   header:    [0x00, 0x61, 0x73, 0x6d]  ("\0asm")
///              [0x01, 0x00, 0x00, 0x00]  (version 1)
///   sections:  [id: u8][size: ULEB128][...size bytes...]
///   custom:    id=0; payload starts with [name_len: ULEB128][name]
fn find_custom_section<'a>(wasm: &'a [u8], target_name: &str) -> Option<&'a [u8]> {
    if wasm.len() < 8 || &wasm[..4] != b"\0asm" {
        return None;
    }
    let mut i = 8usize;
    while i < wasm.len() {
        let section_id = *wasm.get(i)?;
        i += 1;
        let (section_size, consumed) = read_uleb128(&wasm[i..])?;
        i += consumed;
        let section_end = i.checked_add(section_size as usize)?;
        if section_end > wasm.len() {
            return None;
        }
        if section_id == 0 {
            // custom section.
            let body = &wasm[i..section_end];
            let (name_len, name_consumed) = read_uleb128(body)?;
            let name_end = name_consumed.checked_add(name_len as usize)?;
            if name_end > body.len() {
                return None;
            }
            let name_bytes = &body[name_consumed..name_end];
            let payload = &body[name_end..];
            if name_bytes == target_name.as_bytes() {
                return Some(payload);
            }
        }
        i = section_end;
    }
    None
}

/// read an unsigned LEB128 from the start of `bytes`. returns
/// `(value, bytes-consumed)` or None on overrun / overflow.
fn read_uleb128(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    let mut i = 0usize;
    loop {
        let byte = *bytes.get(i)?;
        i += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i));
        }
        shift = shift.checked_add(7)?;
        if shift > 63 {
            return None;
        }
    }
}

/// per-method shape — used by the trampoline to know how to
/// marshal args and results. starts minimal; grows as we add
/// signatures.
#[derive(Copy, Clone, Debug)]
pub enum ExportShape {
    /// fn() -> i64. result becomes Value::Int.
    NoArgsToI64,
}

/// parsed `moof.manifest` custom-section contents. fields grow as
/// the manifest schema does; for now we extract just the bits the
/// loader cross-validates with: the abi-version and the method
/// names. `parent` is captured but defaults to Object regardless
/// (proper resolution comes with the dep-resolution pipeline).
#[derive(Debug, Default)]
pub struct McoManifest {
    pub abi_version: i64,
    pub methods: Vec<String>,
}

/// extract and parse the `moof.manifest` custom section, if any.
/// returns `None` when no manifest is present (the dev/raw-.wasm
/// case); returns `Some` after a successful parse; raises if a
/// manifest IS present but malformed.
///
/// wasmtime 26 doesn't expose custom sections by name on `Module`,
/// so we walk the raw wasm bytes ourselves. the format is
/// straightforward — each section begins with `id (u8) + size
/// (ULEB128)`; custom sections have id=0 and start with `name_len
/// (ULEB128) + name (utf-8)` then arbitrary payload bytes.
fn parse_manifest_section(
    world: &mut World,
    wasm_bytes: &[u8],
    label: &str,
) -> Result<Option<McoManifest>, RaiseError> {
    let payload = match find_custom_section(wasm_bytes, "moof.manifest") {
        Some(p) => p,
        None => return Ok(None),
    };
    let text = std::str::from_utf8(payload).map_err(|e| {
        RaiseError::new(
            world.intern("mco-manifest-parse-error"),
            format!("`{}` moof.manifest is not utf-8: {}", label, e),
        )
    })?;
    // parse the manifest as moof source-text. it's a list of
    // (key value) pairs:
    //   ((abi-version 1)
    //    (parent Object)
    //    (methods (now monotonic)))
    let form = world.read(text).map_err(|e| {
        RaiseError::new(
            world.intern("mco-manifest-parse-error"),
            format!("`{}` moof.manifest read error: {}", label, e.message),
        )
    })?;
    decode_manifest_form(world, form, label)
}

/// walk a parsed manifest-Form and extract the typed fields.
/// expected shape: a list of (key value) pairs.
fn decode_manifest_form(
    world: &mut World,
    form: Value,
    label: &str,
) -> Result<Option<McoManifest>, RaiseError> {
    let pairs = world.list_to_vec(form).map_err(|_| {
        RaiseError::new(
            world.intern("mco-manifest-parse-error"),
            format!("`{}` moof.manifest must be a list of (key value) pairs", label),
        )
    })?;
    let mut manifest = McoManifest::default();
    let abi_version_sym = world.intern("abi-version");
    let methods_sym = world.intern("methods");
    for pair_v in pairs {
        let pair = world.list_to_vec(pair_v).map_err(|_| {
            RaiseError::new(
                world.intern("mco-manifest-parse-error"),
                format!("`{}` manifest entry isn't a list", label),
            )
        })?;
        if pair.len() < 2 {
            continue;
        }
        let key = match pair[0].as_sym() {
            Some(s) => s,
            None => continue,
        };
        if key == abi_version_sym {
            if let Some(n) = pair[1].as_int() {
                manifest.abi_version = n;
            }
        } else if key == methods_sym {
            let method_list = world.list_to_vec(pair[1]).unwrap_or_default();
            for m in method_list {
                if let Some(s) = m.as_sym() {
                    manifest.methods.push(world.resolve(s).to_string());
                }
            }
        }
        // parent: ignored for now — defaults to Object. proper
        // dep-resolution comes in a later pass.
    }
    Ok(Some(manifest))
}

/// install substrate-provided functions into a wasmtime Linker.
/// every wasm mco that imports something `extern "moof" fn …`
/// resolves through here.
///
/// the names + signatures form a stable abi surface the substrate
/// commits to. **only moof-specific primitives go here** — things
/// that have no POSIX equivalent. system services like clocks,
/// filesystems, randomness, network: those are WASI's job; mcos
/// import them through `wasi_snapshot_preview1` directly.
///
/// the substrate doesn't fake-shim POSIX. clear separation:
///   "wasi" namespace  → standard system services
///   "moof" namespace  → moof-specific (slot, send, raise, …)
///
/// planned moof imports (coming as richer methods need them):
/// - `intern(ptr, len) -> sym-handle`
/// - `make_string(ptr, len) -> form-handle`
/// - `slot(form-handle, sym-handle) -> value-handle`
/// - `slot_set(form-handle, sym-handle, value-handle)`
/// - `send(receiver, sel, args-ptr, argc) -> value-handle`
/// - `raise(kind-sym, msg-ptr, msg-len) -> traps`
///
/// the function is currently a no-op. left as a hook so the
/// import-pipeline plumbing is in place; the first real moof-
/// import lands when an mco needs slot access.
fn install_moof_imports(_linker: &mut Linker<WasiP1Ctx>) -> wasmtime::Result<()> {
    Ok(())
}

/// the trampoline that bridges a moof method-call to a wasm
/// function-call. installed once per export; looks up its own
/// (proto, selector) pair in the world's wasm-export map at
/// dispatch time to know which wasm function to call.
///
/// this dance is necessary because `install_native` accepts a fn
/// pointer (`NativeFn`), not a closure with captured state. so
/// per-export state lives in side-tables on World keyed by the
/// dispatch site.
pub fn wasm_method_trampoline(
    world: &mut World,
    self_: Value,
    args: &[Value],
) -> Result<Value, RaiseError> {
    // recover the proto-id from `self_` — for an instance, that's
    // `proto_of(self_)`; for a class-side send (`[Proto answer]`),
    // self_ IS the proto.
    let proto_id = match world.proto_of(self_) {
        Value::Form(p) => {
            // is THIS form a registered mco-loaded proto?
            if world.wasm_instances.contains_key(&p) {
                p
            } else if let Some(p_id) = self_.as_form_id() {
                // class-side: receiver is the proto-Form itself.
                if world.wasm_instances.contains_key(&p_id) {
                    p_id
                } else {
                    return Err(RaiseError::new(
                        world.intern("dispatch-error"),
                        "wasm-method called on non-wasm-mco receiver",
                    ));
                }
            } else {
                return Err(RaiseError::new(
                    world.intern("dispatch-error"),
                    "wasm-method called on non-wasm-mco receiver",
                ));
            }
        }
        _ => {
            return Err(RaiseError::new(
                world.intern("dispatch-error"),
                "wasm-method called on tagged-immediate receiver",
            ));
        }
    };

    // the dispatcher routed us here because it found the selector
    // on the proto's handler table — but we need to know WHICH
    // selector. NativeFn doesn't tell us. so we reconstruct from
    // a thread-local-ish convention: the world stashes the
    // currently-dispatching selector in `world.vm.last_send_sel`
    // before calling the handler. simpler approach for the MVP:
    // walk the export map for this proto, finding the entry whose
    // installed handler is THIS function pointer. since there's
    // (currently) only one shape, we can punt and just take the
    // first one. (proper fix: dispatch passes selector to native.
    // tracked.)
    //
    // even simpler MVP: since we're only handling `() -> i64`,
    // ANY method call with no args must hit one of these
    // exports. find the matching one by iterating.

    if !args.is_empty() {
        return Err(RaiseError::new(
            world.intern("arity"),
            "wasm method (mvp) takes no args",
        ));
    }

    // find this proto's export map entry — but which selector?
    // we don't have it. for the MVP, require world.vm to expose
    // the most-recent selector (added below). fallback: error.
    let sel = world.vm.last_send_sel.ok_or_else(|| {
        RaiseError::new(
            world.intern("dispatch-error"),
            "no current send-selector available",
        )
    })?;
    let (export_name, _shape) = world
        .wasm_export_map
        .get(&(proto_id, sel))
        .cloned()
        .ok_or_else(|| {
            RaiseError::new(
                world.intern("dispatch-error"),
                "no wasm export registered for this selector",
            )
        })?;

    // pre-intern error symbols so we don't fight the borrow
    // checker once the wasm-instance mut-borrow is live.
    let wasm_err_sym = world.intern("wasm-error");

    // call the wasm function.
    let inst = world.wasm_instances.get_mut(&proto_id).unwrap();
    let func = inst
        .instance
        .get_typed_func::<(), i64>(&mut inst.store, &export_name)
        .map_err(|e| {
            RaiseError::new(
                wasm_err_sym,
                format!("export `{}` lookup failed: {}", export_name, e),
            )
        })?;
    let result = func.call(&mut inst.store, ()).map_err(|e| {
        RaiseError::new(
            wasm_err_sym,
            format!("wasm `{}` trapped: {}", export_name, e),
        )
    })?;
    Ok(Value::Int(result))
}
