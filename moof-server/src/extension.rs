/// Extension loading: the server loads .dylib/.so files at startup.
/// Each extension exports a C-ABI init function that receives a Server pointer.
///
/// Extensions can:
/// - Register HandlerInvokers (language shells)
/// - Create system vats (IO, FFI bridges)
/// - Register native handlers on type prototypes
/// - Load bootstrap code
/// - Anything the Server API allows

use crate::Server;

/// The function signature every extension must export.
/// Name: `moof_extension_init`
pub type ExtensionInitFn = unsafe extern "C" fn(server: *mut Server);

/// Load an extension from a dylib path.
pub fn load_extension(server: &mut Server, path: &str) -> Result<(), String> {
    unsafe {
        let lib = libloading::Library::new(path)
            .map_err(|e| format!("cannot load {}: {}", path, e))?;

        let init: libloading::Symbol<ExtensionInitFn> = lib.get(b"moof_extension_init")
            .map_err(|e| format!("{}: missing moof_extension_init: {}", path, e))?;

        init(server as *mut Server);

        // Leak the library so it stays loaded for the process lifetime.
        // Extensions register closures that reference code in the dylib,
        // so the dylib must outlive the fabric.
        std::mem::forget(lib);

        Ok(())
    }
}
