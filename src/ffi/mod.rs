/// FFI layer for MOOF — dynamic loading of native libraries.
///
/// Uses libloading for dlopen/dlsym. Supports a fixed set of calling
/// signatures dispatched at runtime. No libffi dependency needed.
///
/// From moof:
///   (def lib (ffi-open "libm"))
///   (def sin (ffi-bind lib "sin" '(f64) 'f64))
///   (sin 3.14159)

pub mod bridge;
