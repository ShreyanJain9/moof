// Dynamic plugin loading: load a .dylib/.so at runtime.
//
// Plugin ABI (C-compatible):
//
// A plugin shared library must export these two functions:
//
//   const char* moof_plugin_name()
//     Returns the capability name (e.g. "db", "redis").
//     The REPL binds a FarRef to this name.
//
//   void moof_plugin_setup(MoofSetupCtx* ctx)
//     Called once to set up handlers on the capability's root object.
//     Uses the ctx pointer to call back into moof:
//       moof_register_handler(ctx, "selector", handler_fn)
//       moof_make_string(ctx, "text")  → value handle
//       etc.
//
// The handler function signature:
//   uint64_t handler(MoofCallCtx* ctx, uint64_t receiver, uint64_t* args, uint32_t nargs)
//
// Values are passed as raw u64 (NaN-boxed). The plugin uses helper
// functions to decode/encode them.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

use crate::heap::Heap;
use crate::value::Value;
use crate::scheduler::Vat;
use crate::object::HeapObject;
use super::CapabilityPlugin;

/// Opaque context passed to plugin setup function.
/// Contains the vat being set up and the root object id.
#[repr(C)]
pub struct MoofSetupCtx {
    vat: *mut Vat,
    root_obj_id: u32,
}

/// The C function signature for a native handler in a plugin.
/// Returns a NaN-boxed Value as u64 (0 = nil, nonzero = value).
/// On error, sets *error to a non-null string (plugin must keep it alive).
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

// ── C API for plugins to call back into moof ────────────

/// Register a handler on the root object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_register_handler(
    ctx: *mut MoofSetupCtx,
    selector: *const c_char,
    handler: PluginHandlerFn,
) {
    let ctx = &mut *ctx;
    let vat = &mut *ctx.vat;
    let sel_str = CStr::from_ptr(selector).to_str().unwrap_or("?");
    let sym = vat.heap.intern(sel_str);

    let h = vat.heap.register_native(sel_str, move |heap, receiver, args| {
        let mut call_ctx = MoofCallCtx { heap: heap as *mut Heap };
        let raw_args: Vec<u64> = args.iter().map(|v| v.to_bits()).collect();
        let result = unsafe {
            handler(
                &mut call_ctx,
                receiver.to_bits(),
                raw_args.as_ptr(),
                raw_args.len() as u32,
            )
        };
        Ok(Value::from_bits(result))
    });
    vat.heap.get_mut(ctx.root_obj_id).handler_set(sym, h);
}

/// Create a string in the heap. Returns a NaN-boxed Value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_make_string(ctx: *mut MoofCallCtx, s: *const c_char) -> u64 {
    let ctx = &mut *ctx;
    let heap = &mut *ctx.heap;
    let text = CStr::from_ptr(s).to_str().unwrap_or("");
    heap.alloc_string(text).to_bits()
}

/// Create an integer Value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_make_integer(n: i64) -> u64 {
    Value::integer(n).to_bits()
}

/// Create a float Value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_make_float(n: f64) -> u64 {
    Value::float(n).to_bits()
}

/// Create nil.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_nil() -> u64 {
    Value::NIL.to_bits()
}

/// Extract an integer from a NaN-boxed value. Returns 0 if not an integer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_as_integer(val: u64) -> i64 {
    Value::from_bits(val).as_integer().unwrap_or(0)
}

/// Extract a float from a NaN-boxed value. Returns 0.0 if not numeric.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_as_float(val: u64) -> f64 {
    Value::from_bits(val).as_float().unwrap_or(0.0)
}

/// Get string content. Returns null if not a string. Caller must NOT free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_as_string(ctx: *mut MoofCallCtx, val: u64) -> *const c_char {
    let ctx = &*ctx;
    let heap = &*ctx.heap;
    let v = Value::from_bits(val);
    if let Some(id) = v.as_any_object() {
        if let HeapObject::Text(s) = heap.get(id) {
            // return a pointer to the string content
            // WARNING: only valid while the heap object lives
            return s.as_ptr() as *const c_char;
        }
    }
    std::ptr::null()
}

// ── Dynamic loading ─────────────────────────────────────

type NameFn = unsafe extern "C" fn() -> *const c_char;
type SetupFn = unsafe extern "C" fn(*mut MoofSetupCtx);

/// A dynamically loaded capability plugin.
pub struct DynCapabilityPlugin {
    cap_name: String,
    _lib: libloading::Library,  // must stay alive while plugin is in use
    setup_fn: SetupFn,
}

impl DynCapabilityPlugin {
    /// Load a plugin from a shared library path.
    /// The library must export `moof_plugin_name` and `moof_plugin_setup`.
    pub fn load(path: &Path) -> Result<Self, String> {
        unsafe {
            let lib = libloading::Library::new(path)
                .map_err(|e| format!("failed to load plugin: {e}"))?;

            let name_fn: libloading::Symbol<NameFn> = lib.get(b"moof_plugin_name")
                .map_err(|e| format!("plugin missing moof_plugin_name: {e}"))?;
            let name_ptr = name_fn();
            let name = CStr::from_ptr(name_ptr).to_str()
                .map_err(|e| format!("plugin name not UTF-8: {e}"))?
                .to_string();

            let setup_fn: libloading::Symbol<SetupFn> = lib.get(b"moof_plugin_setup")
                .map_err(|e| format!("plugin missing moof_plugin_setup: {e}"))?;
            let setup_fn = *setup_fn;

            Ok(DynCapabilityPlugin { cap_name: name, _lib: lib, setup_fn })
        }
    }
}

impl CapabilityPlugin for DynCapabilityPlugin {
    fn name(&self) -> &str { &self.cap_name }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let root_obj = vat.heap.make_object(Value::NIL);
        let root_id = root_obj.as_any_object().unwrap();

        let mut ctx = MoofSetupCtx {
            vat: vat as *mut Vat,
            root_obj_id: root_id,
        };

        unsafe { (self.setup_fn)(&mut ctx) };

        root_id
    }
}
