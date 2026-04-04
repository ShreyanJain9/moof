/// FFI bridge — load native libraries and call their functions from MOOF.
///
/// Uses libloading for dlopen/dlsym. Supports a fixed set of calling
/// signatures dispatched at runtime based on type descriptors.

use libloading::{Library, Symbol as LibSymbol};
use std::collections::HashMap;
use std::ffi::{CString, CStr, c_void};

use crate::runtime::value::{Value, HeapObject};
use crate::runtime::heap::Heap;

/// A loaded native library.
pub struct NativeLibrary {
    _lib: Library, // kept alive for symbol validity
    name: String,
}

/// A bound foreign function, ready to call.
pub struct ForeignFunction {
    /// Raw function pointer
    fn_ptr: *mut c_void,
    /// Argument types as moof symbols: 'i64, 'f64, 'string, 'void, 'pointer
    arg_types: Vec<FfiType>,
    /// Return type
    ret_type: FfiType,
    /// Human-readable name
    name: String,
}

#[derive(Clone, Copy, Debug)]
pub enum FfiType {
    Void,
    I64,
    F64,
    String,
    Pointer,
}

impl FfiType {
    pub fn from_symbol_name(name: &str) -> Option<Self> {
        match name {
            "void" => Some(FfiType::Void),
            "i64" | "int" | "integer" => Some(FfiType::I64),
            "f64" | "float" | "double" => Some(FfiType::F64),
            "string" | "str" => Some(FfiType::String),
            "pointer" | "ptr" => Some(FfiType::Pointer),
            _ => None,
        }
    }
}

/// Open a native library by name.
pub fn open_library(name: &str) -> Result<NativeLibrary, String> {
    // Try the name as-is, then with platform-specific extensions
    let candidates = if name.contains('.') {
        vec![name.to_string()]
    } else {
        if cfg!(target_os = "macos") {
            vec![
                format!("lib{}.dylib", name),
                format!("{}.dylib", name),
                name.to_string(),
            ]
        } else if cfg!(target_os = "linux") {
            vec![
                format!("lib{}.so", name),
                format!("{}.so", name),
                name.to_string(),
            ]
        } else {
            vec![name.to_string()]
        }
    };

    for candidate in &candidates {
        match unsafe { Library::new(candidate) } {
            Ok(lib) => return Ok(NativeLibrary {
                _lib: lib,
                name: name.to_string(),
            }),
            Err(_) => continue,
        }
    }

    Err(format!("Cannot load library: {}", name))
}

/// Look up a function in a library and bind it with type information.
pub fn bind_function(
    lib: &NativeLibrary,
    func_name: &str,
    arg_types: Vec<FfiType>,
    ret_type: FfiType,
) -> Result<ForeignFunction, String> {
    let fn_ptr = unsafe {
        let sym: LibSymbol<*mut c_void> = lib._lib.get(func_name.as_bytes())
            .map_err(|e| format!("Cannot find symbol '{}': {}", func_name, e))?;
        *sym
    };

    Ok(ForeignFunction {
        fn_ptr,
        arg_types,
        ret_type,
        name: format!("{}:{}", lib.name, func_name),
    })
}

/// Call a foreign function with MOOF values.
/// This is the unsafe core — dispatches based on type signature.
pub fn call_foreign(
    ff: &ForeignFunction,
    args: &[Value],
    heap: &mut crate::runtime::heap::Heap,
) -> Result<Value, String> {
    if args.len() != ff.arg_types.len() {
        return Err(format!("{}: expected {} args, got {}",
            ff.name, ff.arg_types.len(), args.len()));
    }

    // Dispatch based on signature pattern
    // We support common patterns without libffi by transmuting function pointers
    unsafe {
        match (ff.arg_types.as_slice(), ff.ret_type) {
            // () -> void
            (&[], FfiType::Void) => {
                let f: extern "C" fn() = std::mem::transmute(ff.fn_ptr);
                f();
                Ok(Value::Nil)
            }
            // () -> i64
            (&[], FfiType::I64) => {
                let f: extern "C" fn() -> i64 = std::mem::transmute(ff.fn_ptr);
                Ok(Value::Integer(f()))
            }
            // () -> f64
            (&[], FfiType::F64) => {
                let f: extern "C" fn() -> f64 = std::mem::transmute(ff.fn_ptr);
                Ok(Value::Float(f()))
            }
            // (f64) -> f64  — the most common math library pattern
            (&[FfiType::F64], FfiType::F64) => {
                let a = args[0].as_float().ok_or("expected float arg")?;
                let f: extern "C" fn(f64) -> f64 = std::mem::transmute(ff.fn_ptr);
                Ok(Value::Float(f(a)))
            }
            // (f64, f64) -> f64
            (&[FfiType::F64, FfiType::F64], FfiType::F64) => {
                let a = args[0].as_float().ok_or("expected float arg 1")?;
                let b = args[1].as_float().ok_or("expected float arg 2")?;
                let f: extern "C" fn(f64, f64) -> f64 = std::mem::transmute(ff.fn_ptr);
                Ok(Value::Float(f(a, b)))
            }
            // (i64) -> i64
            (&[FfiType::I64], FfiType::I64) => {
                let a = args[0].as_integer().ok_or("expected integer arg")?;
                let f: extern "C" fn(i64) -> i64 = std::mem::transmute(ff.fn_ptr);
                Ok(Value::Integer(f(a)))
            }
            // (i64, i64) -> i64
            (&[FfiType::I64, FfiType::I64], FfiType::I64) => {
                let a = args[0].as_integer().ok_or("expected integer arg 1")?;
                let b = args[1].as_integer().ok_or("expected integer arg 2")?;
                let f: extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ff.fn_ptr);
                Ok(Value::Integer(f(a, b)))
            }
            // (string) -> i64 (e.g., strlen, atoi)
            (&[FfiType::String], FfiType::I64) => {
                let s = get_string_arg(&args[0], heap)?;
                let cs = CString::new(s).map_err(|e| format!("invalid C string: {}", e))?;
                let f: extern "C" fn(*const i8) -> i64 = std::mem::transmute(ff.fn_ptr);
                Ok(Value::Integer(f(cs.as_ptr())))
            }
            // (string) -> string (rare but useful)
            (&[FfiType::String], FfiType::String) => {
                let s = get_string_arg(&args[0], heap)?;
                let cs = CString::new(s).map_err(|e| format!("invalid C string: {}", e))?;
                let f: extern "C" fn(*const i8) -> *const i8 = std::mem::transmute(ff.fn_ptr);
                let result = f(cs.as_ptr());
                if result.is_null() {
                    Ok(Value::Nil)
                } else {
                    let result_str = CStr::from_ptr(result).to_string_lossy().to_string();
                    Ok(heap.alloc_string(&result_str))
                }
            }
            _ => Err(format!("{}: unsupported signature {:?} -> {:?}",
                ff.name, ff.arg_types, ff.ret_type)),
        }
    }
}

fn get_string_arg(val: &Value, heap: &crate::runtime::heap::Heap) -> Result<String, String> {
    match val {
        Value::Object(id) => match heap.get(*id) {
            HeapObject::MoofString(s) => Ok(s.clone()),
            _ => Err("expected string argument".into()),
        },
        _ => Err("expected string argument".into()),
    }
}

/// The name of a foreign function (for display).
pub fn foreign_name(ff: &ForeignFunction) -> &str {
    &ff.name
}
