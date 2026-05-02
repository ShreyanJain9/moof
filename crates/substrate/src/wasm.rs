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

    // discover exported functions. for the MVP we only handle
    // exports with shape `() -> i64`. richer signatures need
    // linear-memory marshaling and come with the manifest spec.
    let exports: Vec<(String, ExportShape)> = module
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

/// per-method shape — used by the trampoline to know how to
/// marshal args and results. starts minimal; grows as we add
/// signatures.
#[derive(Copy, Clone, Debug)]
pub enum ExportShape {
    /// fn() -> i64. result becomes Value::Int.
    NoArgsToI64,
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
