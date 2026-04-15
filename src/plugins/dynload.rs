// Dynamic plugin loading: load a .dylib/.so at runtime.
//
// Two plugin ABIs supported:
//
// 1. Rust ABI (recommended for Rust plugins):
//    Export: fn moof_create_plugin() -> Box<dyn CapabilityPlugin>
//    The plugin links against libmoof and uses the full Rust API.
//    Crate type: cdylib. Depends on moof.
//
// 2. C ABI (for C/C++/Zig/etc plugins):
//    Export: moof_plugin_name() and moof_plugin_setup()
//    Uses callback functions to register handlers.
//
// The loader tries Rust ABI first, falls back to C ABI.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};

use crate::heap::Heap;
use crate::value::Value;
use crate::scheduler::Vat;
use crate::object::HeapObject;
use super::CapabilityPlugin;

// ═══════════════════════════════════════════════════════════
// C ABI types and callbacks
// ═══════════════════════════════════════════════════════════

/// Opaque context passed to plugin setup function.
#[repr(C)]
pub struct MoofSetupCtx {
    vat: *mut Vat,
    root_obj_id: u32,
}

/// C handler signature.
pub type PluginHandlerFn = unsafe extern "C" fn(
    ctx: *mut MoofCallCtx,
    receiver: u64,
    args: *const u64,
    nargs: u32,
) -> u64;

/// Opaque context passed to plugin handlers at call time.
#[repr(C)]
pub struct MoofCallCtx {
    heap: *mut Heap,
}

// C API callbacks for plugins

#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_register_handler(
    ctx: *mut MoofSetupCtx,
    selector: *const c_char,
    handler: PluginHandlerFn,
) {
    let ctx = unsafe { &mut *ctx };
    let vat = unsafe { &mut *ctx.vat };
    let sel_str = unsafe { CStr::from_ptr(selector) }.to_str().unwrap_or("?");
    let sym = vat.heap.intern(sel_str);
    let h = vat.heap.register_native(sel_str, move |heap, receiver, args| {
        let mut call_ctx = MoofCallCtx { heap: heap as *mut Heap };
        let raw_args: Vec<u64> = args.iter().map(|v| v.to_bits()).collect();
        let result = unsafe {
            handler(&mut call_ctx, receiver.to_bits(), raw_args.as_ptr(), raw_args.len() as u32)
        };
        Ok(Value::from_bits(result))
    });
    vat.heap.get_mut(ctx.root_obj_id).handler_set(sym, h);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_make_string(ctx: *mut MoofCallCtx, s: *const c_char) -> u64 {
    let heap = unsafe { &mut *(*ctx).heap };
    let text = unsafe { CStr::from_ptr(s) }.to_str().unwrap_or("");
    heap.alloc_string(text).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn moof_make_integer(n: i64) -> u64 { Value::integer(n).to_bits() }

#[unsafe(no_mangle)]
pub extern "C" fn moof_make_float(n: f64) -> u64 { Value::float(n).to_bits() }

#[unsafe(no_mangle)]
pub extern "C" fn moof_nil() -> u64 { Value::NIL.to_bits() }

#[unsafe(no_mangle)]
pub extern "C" fn moof_as_integer(val: u64) -> i64 { Value::from_bits(val).as_integer().unwrap_or(0) }

#[unsafe(no_mangle)]
pub extern "C" fn moof_as_float(val: u64) -> f64 { Value::from_bits(val).as_float().unwrap_or(0.0) }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_as_string(ctx: *mut MoofCallCtx, val: u64) -> *const c_char {
    let heap = unsafe { &*(*ctx).heap };
    let v = Value::from_bits(val);
    if let Some(id) = v.as_any_object() {
        if let HeapObject::Text(s) = heap.get(id) {
            return s.as_ptr() as *const c_char;
        }
    }
    std::ptr::null()
}

// ═══════════════════════════════════════════════════════════
// Dynamic loader
// ═══════════════════════════════════════════════════════════

type CNameFn = unsafe extern "C" fn() -> *const c_char;
type CSetupFn = unsafe extern "C" fn(*mut MoofSetupCtx);
type RustCreateFn = fn() -> Box<dyn CapabilityPlugin>;

/// A dynamically loaded capability plugin.
/// Supports both Rust ABI (moof_create_plugin) and C ABI (moof_plugin_name + moof_plugin_setup).
pub struct DynCapabilityPlugin {
    cap_name: String,
    path: PathBuf,
    _lib: libloading::Library,
    inner: DynPluginInner,
}

enum DynPluginInner {
    /// Rust ABI: the plugin returned a trait object
    Rust(Box<dyn CapabilityPlugin>),
    /// C ABI: we call the setup function directly
    C(CSetupFn),
}

impl DynCapabilityPlugin {
    /// Load a plugin from a shared library path.
    /// Tries Rust ABI first (moof_create_plugin), falls back to C ABI.
    pub fn load(path: &Path) -> Result<Self, String> {
        let lib = unsafe { libloading::Library::new(path) }
            .map_err(|e| format!("failed to load plugin: {e}"))?;

        // try Rust ABI first
        if let Ok(create_fn) = unsafe { lib.get::<RustCreateFn>(b"moof_create_plugin") } {
            let plugin = create_fn();
            let name = plugin.name().to_string();
            return Ok(DynCapabilityPlugin {
                cap_name: name,
                path: path.to_path_buf(),
                _lib: lib,
                inner: DynPluginInner::Rust(plugin),
            });
        }

        // fall back to C ABI
        unsafe {
            let name_fn: libloading::Symbol<CNameFn> = lib.get(b"moof_plugin_name")
                .map_err(|e| format!("plugin missing both moof_create_plugin and moof_plugin_name: {e}"))?;
            let name_ptr = name_fn();
            let name = CStr::from_ptr(name_ptr).to_str()
                .map_err(|e| format!("plugin name not UTF-8: {e}"))?
                .to_string();

            let setup_fn: libloading::Symbol<CSetupFn> = lib.get(b"moof_plugin_setup")
                .map_err(|e| format!("plugin missing moof_plugin_setup: {e}"))?;
            let setup_fn = *setup_fn;

            Ok(DynCapabilityPlugin {
                cap_name: name,
                path: path.to_path_buf(),
                _lib: lib,
                inner: DynPluginInner::C(setup_fn),
            })
        }
    }

    /// The path this plugin was loaded from.
    pub fn path(&self) -> &Path { &self.path }
}

impl CapabilityPlugin for DynCapabilityPlugin {
    fn name(&self) -> &str { &self.cap_name }

    fn setup(&self, vat: &mut Vat) -> u32 {
        match &self.inner {
            DynPluginInner::Rust(plugin) => plugin.setup(vat),
            DynPluginInner::C(setup_fn) => {
                let root_obj = vat.heap.make_object(Value::NIL);
                let root_id = root_obj.as_any_object().unwrap();
                let mut ctx = MoofSetupCtx {
                    vat: vat as *mut Vat,
                    root_obj_id: root_id,
                };
                unsafe { setup_fn(&mut ctx) };
                root_id
            }
        }
    }
}
