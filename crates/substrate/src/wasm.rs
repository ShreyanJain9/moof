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

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use wasmtime::{Engine, Instance, Linker, Module, Store};
use wasmtime_wasi::preview1::WasiP1Ctx;
use wasmtime_wasi::WasiCtxBuilder;

use crate::form::Form;
use crate::sym::SymId;
use crate::value::Value;
use crate::world::{RaiseError, World};

thread_local! {
    /// pinned World pointer for the duration of an mco dispatch.
    /// SAFETY: substrate is single-threaded per vat; pin ↔ unpin
    /// via DispatchGuard ensures the pointer is only dereferenced
    /// while the World is alive and not otherwise borrowed.
    static DISPATCH_WORLD: Cell<*mut World> = Cell::new(std::ptr::null_mut());

    /// per-dispatch handle table. allocated on dispatch entry,
    /// cleared on dispatch exit (including raise paths via Drop).
    /// pub so that tests and the trampoline (C4) can inspect handles
    /// after a wasm call returns.
    pub static DISPATCH_HANDLE_TABLE: RefCell<HandleTable> = RefCell::new(HandleTable::new());
}

/// RAII guard for an mco dispatch's thread-local state.
/// pin a World and clear the handle table on construction;
/// unpin and clear on Drop (including panic-unwind / wasmtime trap).
pub struct DispatchGuard {
    _private: (),
}

impl DispatchGuard {
    /// begin an mco dispatch: pin the world pointer, clear any
    /// stale handles. returns a guard that cleans up on drop.
    ///
    /// panics if called while another DispatchGuard is live —
    /// nested dispatch is not supported (the raw pointer would be
    /// silently overwritten, corrupting the outer dispatch).
    pub fn begin(world: &mut World) -> Self {
        DISPATCH_WORLD.with(|cell| {
            assert!(
                cell.get().is_null(),
                "nested dispatch not supported — DispatchGuard::begin called while another is live"
            );
            cell.set(world as *mut World);
        });
        DISPATCH_HANDLE_TABLE.with(|t| t.borrow_mut().clear());
        Self { _private: () }
    }
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        DISPATCH_WORLD.with(|cell| cell.set(std::ptr::null_mut()));
        DISPATCH_HANDLE_TABLE.with(|t| t.borrow_mut().clear());
    }
}

/// look up the current dispatch's World. panics if called outside dispatch.
fn current_world() -> *mut World {
    let p = DISPATCH_WORLD.with(|cell| cell.get());
    assert!(!p.is_null(), "moof import called outside dispatch");
    p
}

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
        let idx = u32::try_from(self.slots.len()).expect("HandleTable overflow");
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
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
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

/// load wasm bytes (already in memory) and instantiate. used by
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

    // discover exported functions. any function export is eligible;
    // the trampoline introspects the signature at dispatch time.
    // we tag `() -> i64` exports for backward-compat metadata but
    // all shapes are accepted and installed.
    let all_exports: Vec<(String, ExportShape)> = module
        .exports()
        .filter_map(|exp| match exp.ty() {
            wasmtime::ExternType::Func(ft) => {
                let params = ft.params().collect::<Vec<_>>();
                let results = ft.results().collect::<Vec<_>>();
                // tag the legacy no-args-i64 shape for metadata; everything
                // else is AnyFunc. the trampoline handles all via introspection.
                let shape = if params.is_empty()
                    && results.len() == 1
                    && matches!(results[0], wasmtime::ValType::I64)
                {
                    ExportShape::NoArgsToI64
                } else {
                    ExportShape::AnyFunc
                };
                Some((exp.name().to_string(), shape))
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

    // apply meta entries declared in the manifest to the proto-Form.
    // DataSource defaults (done?, take:, forEach:) key off meta slots
    // like :infinite-source and :infinite-source-flavor.
    if let Some(m) = &manifest {
        for (k, v) in &m.meta {
            world.heap.get_mut(proto_id).meta.insert(*k, *v);
        }
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

/// per-method shape — used to tag exported functions at load time.
/// the trampoline now introspects signatures directly at dispatch
/// time via `func.ty()`, so this enum is kept minimal. it records
/// the "detected at load" category but the trampoline ignores it.
#[derive(Copy, Clone, Debug)]
pub enum ExportShape {
    /// fn() -> i64. the original MVP-only shape; still accepted.
    NoArgsToI64,
    /// any other function export. shape is determined at dispatch
    /// time by introspecting `func.ty()`.
    AnyFunc,
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
    /// meta entries declared in the manifest's `(meta ...)` section.
    /// each entry is a (key, value) pair where value is a pre-parsed
    /// moof Value. applied to the proto-Form immediately after load.
    pub meta: Vec<(crate::sym::SymId, Value)>,
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
    let meta_sym = world.intern("meta");
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
        } else if key == meta_sym {
            // meta section: the remaining elements of the pair list are
            // (key value) pairs, e.g.:
            //   (meta
            //     (infinite-source #true)
            //     (infinite-source-flavor 'generator))
            // pair = [sym:meta, (infinite-source #true), (infinite-source-flavor 'generator)]
            // so pair[1..] are the individual meta entries.
            let quote_sym = world.intern("quote");
            for mp_v in &pair[1..] {
                let mp = match world.list_to_vec(*mp_v) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if mp.len() < 2 {
                    continue;
                }
                let mk = match mp[0].as_sym() {
                    Some(s) => s,
                    None => continue,
                };
                // unwrap (quote sym) → sym. reader produces
                // 'foo as a (quote foo) pair.
                let mv = {
                    let raw = mp[1];
                    if let Ok(inner) = world.list_to_vec(raw) {
                        // check if it's (quote X)
                        if inner.len() == 2 && inner[0].as_sym() == Some(quote_sym) {
                            inner[1]
                        } else {
                            raw
                        }
                    } else {
                        raw
                    }
                };
                manifest.meta.push((mk, mv));
            }
        }
        // parent: ignored for now — defaults to Object. proper
        // dep-resolution comes in a later pass.
    }
    Ok(Some(manifest))
}

/// install substrate-provided functions into a wasmtime Linker.
/// every wasm mco that imports something in the "moof" wasm namespace
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
///   "moof" namespace  → moof-specific (raise, make_string, make_bytes,
///                        string_text, bytes_data, intern)
///
/// all closures read/write the World and HandleTable through the
/// DISPATCH_WORLD / DISPATCH_HANDLE_TABLE thread-locals set up by
/// DispatchGuard::begin. no WasiP1Ctx-specific state is used, so
/// the function is generic over the store context T.
pub fn install_moof_imports<T: 'static>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    use wasmtime::{Caller, Extern};

    // helper: read a slice from wasm linear memory.
    fn read_linmem<T>(
        caller: &mut Caller<'_, T>,
        ptr: u32,
        len: u32,
    ) -> wasmtime::Result<Vec<u8>> {
        let mem = caller
            .get_export("memory")
            .and_then(Extern::into_memory)
            .ok_or_else(|| wasmtime::Error::msg("wasm module has no memory export"))?;
        let data = mem.data(caller);
        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or_else(|| wasmtime::Error::msg("moof import: ptr+len overflow"))?;
        if end > data.len() {
            return Err(wasmtime::Error::msg("moof import: ptr+len out of bounds"));
        }
        Ok(data[start..end].to_vec())
    }

    // helper: write bytes into wasm linear memory at ptr (capped at cap).
    // returns the total (uncapped) length so the caller can size-check.
    fn write_linmem<T>(
        caller: &mut Caller<'_, T>,
        ptr: u32,
        cap: u32,
        bytes: &[u8],
    ) -> wasmtime::Result<usize> {
        let to_write = bytes.len().min(cap as usize);
        let mem = caller
            .get_export("memory")
            .and_then(Extern::into_memory)
            .ok_or_else(|| wasmtime::Error::msg("wasm module has no memory export"))?;
        let start = ptr as usize;
        let end = start
            .checked_add(to_write)
            .ok_or_else(|| wasmtime::Error::msg("moof import: write ptr+cap overflow"))?;
        let data = mem.data_mut(caller);
        if end > data.len() {
            return Err(wasmtime::Error::msg(
                "moof import: write ptr+cap out of bounds",
            ));
        }
        data[start..end].copy_from_slice(&bytes[..to_write]);
        Ok(bytes.len())
    }

    // moof_raise(kind_handle, msg_ptr, msg_len) — traps with a structured
    // error message that the trampoline (C4) will decode into a RaiseError.
    linker.func_wrap(
        "moof",
        "moof_raise",
        |mut caller: Caller<'_, T>,
         kind_handle: u32,
         msg_ptr: u32,
         msg_len: u32|
         -> wasmtime::Result<()> {
            let msg_bytes = read_linmem(&mut caller, msg_ptr, msg_len)?;
            let msg = String::from_utf8_lossy(&msg_bytes).into_owned();
            // encode kind as the raw SymId u32 — colon-free by construction,
            // so colon-bearing symbol names like `at:put:` survive intact.
            // 0 is the SymId::NONE sentinel; the trampoline maps it to
            // the generic 'wasm-error' fallback.
            let kind_id: u32 = DISPATCH_HANDLE_TABLE
                .with(|t| {
                    let table = t.borrow();
                    table.get(kind_handle).and_then(|v| {
                        if let Value::Sym(sid) = v {
                            Some(sid.0)
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or(0);
            // encode kind+msg into the trap message so C4's trampoline
            // can reconstruct the RaiseError without an extra side-channel.
            // format: __moof_raise_id__:U32:MSG  (U32 has no colons)
            Err(wasmtime::Error::msg(format!(
                "__moof_raise_id__:{}:{}",
                kind_id, msg
            )))
        },
    )?;

    // moof_make_string(ptr, len) -> handle
    linker.func_wrap(
        "moof",
        "moof_make_string",
        |mut caller: Caller<'_, T>, ptr: u32, len: u32| -> wasmtime::Result<u32> {
            let bytes = read_linmem(&mut caller, ptr, len)?;
            let s = String::from_utf8_lossy(&bytes).into_owned();
            // SAFETY: pointer valid within dispatch (see DispatchGuard).
            let world: &mut World = unsafe { &mut *current_world() };
            let v = world.make_string(&s);
            Ok(DISPATCH_HANDLE_TABLE.with(|t| t.borrow_mut().push(v)))
        },
    )?;

    // moof_make_bytes(ptr, len) -> handle
    linker.func_wrap(
        "moof",
        "moof_make_bytes",
        |mut caller: Caller<'_, T>, ptr: u32, len: u32| -> wasmtime::Result<u32> {
            let bytes = read_linmem(&mut caller, ptr, len)?;
            // SAFETY: pointer valid within dispatch (see DispatchGuard).
            let world: &mut World = unsafe { &mut *current_world() };
            let v = world.make_bytes(&bytes);
            Ok(DISPATCH_HANDLE_TABLE.with(|t| t.borrow_mut().push(v)))
        },
    )?;

    // moof_string_text(handle, buf, cap) -> u32 (actual byte length)
    // writes min(actual, cap) bytes into buf; returns actual length
    // so the caller can detect truncation or pre-size a second call.
    linker.func_wrap(
        "moof",
        "moof_string_text",
        |mut caller: Caller<'_, T>,
         handle: u32,
         buf: u32,
         cap: u32|
         -> wasmtime::Result<u32> {
            let bytes_owned = DISPATCH_HANDLE_TABLE
                .with(|t| {
                    let table = t.borrow();
                    let v = table.get(handle).copied()?;
                    // SAFETY: pointer valid within dispatch (see DispatchGuard).
                    let world: &World = unsafe { &*current_world() };
                    world.string_bytes(v).map(|b| b.to_vec())
                })
                .ok_or_else(|| {
                    wasmtime::Error::msg("moof_string_text: handle is not a String")
                })?;
            let actual = write_linmem(&mut caller, buf, cap, &bytes_owned)?;
            Ok(actual as u32)
        },
    )?;

    // moof_bytes_data(handle, buf, cap) -> u32 (actual byte length)
    linker.func_wrap(
        "moof",
        "moof_bytes_data",
        |mut caller: Caller<'_, T>,
         handle: u32,
         buf: u32,
         cap: u32|
         -> wasmtime::Result<u32> {
            let bytes_owned = DISPATCH_HANDLE_TABLE
                .with(|t| {
                    let table = t.borrow();
                    let v = table.get(handle).copied()?;
                    // SAFETY: pointer valid within dispatch (see DispatchGuard).
                    let world: &World = unsafe { &*current_world() };
                    world.bytes_data(v).map(|b| b.to_vec())
                })
                .ok_or_else(|| {
                    wasmtime::Error::msg("moof_bytes_data: handle is not a Bytes value")
                })?;
            let actual = write_linmem(&mut caller, buf, cap, &bytes_owned)?;
            Ok(actual as u32)
        },
    )?;

    // moof_intern(ptr, len) -> handle (Symbol)
    linker.func_wrap(
        "moof",
        "moof_intern",
        |mut caller: Caller<'_, T>, ptr: u32, len: u32| -> wasmtime::Result<u32> {
            let bytes = read_linmem(&mut caller, ptr, len)?;
            let s = std::str::from_utf8(&bytes)
                .map_err(|_| wasmtime::Error::msg("moof_intern: invalid utf-8"))?;
            // SAFETY: pointer valid within dispatch (see DispatchGuard).
            let world: &mut World = unsafe { &mut *current_world() };
            let sid: SymId = world.intern(s);
            Ok(DISPATCH_HANDLE_TABLE.with(|t| t.borrow_mut().push(Value::Sym(sid))))
        },
    )?;

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

    // find which selector was just dispatched. the VM stashes it in
    // `last_send_sel` before calling the native handler.
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

    // pre-intern error symbols before any borrow-checker conflicts arise.
    let wasm_err_sym = world.intern("wasm-error");
    let arity_sym = world.intern("arity-mismatch");
    let type_sym = world.intern("type-mismatch");

    // introspect the wasm function signature BEFORE pinning world.
    // we collect param/result types as owned Vecs so we can drop
    // the inst borrow before acquiring the DispatchGuard.
    let (param_tys, result_tys): (Vec<wasmtime::ValType>, Vec<wasmtime::ValType>) = {
        let inst = world.wasm_instances.get_mut(&proto_id).unwrap();
        let func = inst
            .instance
            .get_func(&mut inst.store, &export_name)
            .ok_or_else(|| {
                RaiseError::new(
                    wasm_err_sym,
                    format!("export `{}` not found", export_name),
                )
            })?;
        let ty = func.ty(&mut inst.store);
        (ty.params().collect(), ty.results().collect())
    };

    if param_tys.len() != args.len() {
        return Err(RaiseError::new(
            arity_sym,
            format!(
                "wasm export `{}` expects {} args, got {}",
                export_name,
                param_tys.len(),
                args.len()
            ),
        ));
    }

    // Pin world + clear handle table. The guard's Drop handles cleanup
    // on all exit paths (including error returns and panics).
    //
    // SAFETY: `world` is mutably borrowed by us for the lifetime of
    // this function. The guard holds a raw *mut World. The wasm
    // imports that dereference it run synchronously inside func.call —
    // they cannot race with our outer &mut World accesses. We must
    // not hold a Rust &mut borrow of `world` across func.call itself
    // (the imports need the same pointer), but since the imports only
    // touch `make_string`, `make_bytes`, `intern`, `string_bytes`,
    // `bytes_data`, and `resolve` — none of which touch
    // `wasm_instances` — the aliasing is safe in practice.
    let _guard = DispatchGuard::begin(world);

    // Marshal moof args → wasm Val.
    //
    // ValType::I32 with a non-Int arg: push to handle table → pass
    // as a u32 slot index cast to i32. The wasm side interprets the
    // signed-looking bit pattern as an unsigned slot index (u32 and
    // i32 share the same bits in wasm; the guest code casts back).
    //
    // ValType::I64 requires Value::Int (no implicit coercion).
    let mut wasm_args: Vec<wasmtime::Val> = Vec::with_capacity(args.len());
    for (ty, arg) in param_tys.iter().zip(args.iter()) {
        let wval = match ty {
            wasmtime::ValType::I32 => match arg {
                Value::Int(n)
                    if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 =>
                {
                    wasmtime::Val::I32(*n as i32)
                }
                _ => {
                    // non-Int or out-of-range: treat as a handle slot.
                    let h = DISPATCH_HANDLE_TABLE
                        .with(|t| t.borrow_mut().push(*arg));
                    wasmtime::Val::I32(h as i32)
                }
            },
            wasmtime::ValType::I64 => match arg {
                Value::Int(n) => wasmtime::Val::I64(*n),
                _ => {
                    drop(_guard);
                    return Err(RaiseError::new(
                        type_sym,
                        format!(
                            "wasm export `{}`: i64 param requires Int, got {:?}",
                            export_name, arg
                        ),
                    ));
                }
            },
            _ => {
                drop(_guard);
                return Err(RaiseError::new(
                    type_sym,
                    format!(
                        "wasm export `{}`: unsupported wasm param type {:?}",
                        export_name, ty
                    ),
                ));
            }
        };
        wasm_args.push(wval);
    }

    // Prepare results buffer.
    let mut wasm_results: Vec<wasmtime::Val> =
        vec![wasmtime::Val::I32(0); result_tys.len()];

    // Call wasm. The DispatchGuard is active: the 6 moof imports can
    // dereference DISPATCH_WORLD safely. We must NOT hold a Rust &mut
    // reference to the top-level `world` across this call (the raw
    // pointer alias would be observable). Instead we access world only
    // through the inst sub-borrow, which the imports don't touch.
    let call_result = {
        let inst = world.wasm_instances.get_mut(&proto_id).unwrap();
        let func = inst
            .instance
            .get_func(&mut inst.store, &export_name)
            .unwrap(); // we verified this above
        func.call(&mut inst.store, &wasm_args, &mut wasm_results)
    };

    // Marshal return value.
    match call_result {
        Ok(()) => {
            let ret = if result_tys.is_empty() {
                Value::Nil
            } else {
                match result_tys[0] {
                    wasmtime::ValType::I64 => {
                        Value::Int(wasm_results[0].i64().unwrap_or(0))
                    }
                    wasmtime::ValType::I32 => {
                        // Treat as a handle slot: take the Value out of the
                        // table before the guard drops and clears it.
                        let h = wasm_results[0].i32().unwrap_or(0) as u32;
                        DISPATCH_HANDLE_TABLE
                            .with(|t| t.borrow_mut().take(h))
                            .unwrap_or(Value::Nil)
                    }
                    _ => Value::Nil,
                }
            };
            // Guard drops here: clears any remaining table entries and
            // unpins world. The return value has already been extracted.
            Ok(ret)
        }
        Err(trap_err) => {
            // Try to extract a structured moof raise from anywhere in
            // the error chain. The moof_raise import encodes errors as:
            //   "__moof_raise_id__:KIND_ID:MSG"
            // where KIND_ID is the SymId as a u32 decimal (colon-free by
            // construction, so keyword selectors like `at:put:` are safe).
            // MSG is arbitrary text (may contain colons); splitn(2) takes
            // it verbatim as the tail.
            fn extract_moof_raise(
                err: &(dyn std::error::Error + 'static),
            ) -> Option<(u32, String)> {
                let s = err.to_string();
                if let Some(payload) = s.strip_prefix("__moof_raise_id__:") {
                    // payload = "KIND_ID:MSG"
                    let parts: Vec<&str> = payload.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        if let Ok(kind_id) = parts[0].parse::<u32>() {
                            return Some((kind_id, parts[1].to_string()));
                        }
                    }
                }
                // walk the error source chain (wasmtime wraps host errors).
                err.source().and_then(extract_moof_raise)
            }

            let trap_ref: &(dyn std::error::Error + 'static) =
                trap_err.as_ref();
            // Drop the guard BEFORE re-borrowing world for intern —
            // the guard holds the raw pointer; we want clean Rust
            // borrow semantics for the intern call.
            drop(_guard);
            if let Some((kind_id, msg)) = extract_moof_raise(trap_ref) {
                // Convert kind_id back to SymId. 0 is the NONE sentinel
                // (means kind wasn't found in handle table); fall back to
                // the pre-interned wasm_err_sym in that case.
                let kind_sym = if kind_id == 0 {
                    wasm_err_sym
                } else {
                    SymId(kind_id)
                };
                Err(RaiseError::new(kind_sym, msg))
            } else {
                Err(RaiseError::new(
                    wasm_err_sym,
                    format!("wasm `{}` trapped: {}", export_name, trap_err),
                ))
            }
        }
    }
}
