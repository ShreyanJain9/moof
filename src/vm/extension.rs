/// Extension trait for hooking native Rust code into MOOF.
///
/// All native operations — type primitives, FFI bindings, user extensions —
/// go through the same NativeRegistry. One path, one mechanism.

use super::exec::VM;

/// Implement this trait to register native functions with a MOOF VM.
pub trait MoofExtension {
    fn register(&self, vm: &mut VM, root_env: u32);
}
