/// Sandboxed environment construction for module loading.
///
/// Each module evaluates in a restricted env that has:
/// - The six kernel primitives (compiler builtins, always available)
/// - Pure derived forms from bootstrap (fn, let, if, cond, etc.)
/// - Only the specific bindings listed in `requires`
/// - NO io, no print, no load, no ffi, no eval-string

use crate::vm::exec::VM;
use crate::runtime::value::Value;
use std::collections::HashMap;

/// Create a sandboxed environment for loading a module.
///
/// `imports` maps symbol names to their values — these come from
/// the `provides` of each required module.
pub fn create_sandbox_env(
    vm: &mut VM,
    imports: &HashMap<String, Value>,
) -> u32 {
    // Fresh env with no parent — isolated from root
    let env_id = vm.heap.alloc_env(None);

    // Bind nil, true, false
    let nil_sym = vm.heap.intern("nil");
    vm.env_define_helper(env_id, nil_sym, Value::Nil);
    let true_sym = vm.heap.intern("true");
    vm.env_define_helper(env_id, true_sym, Value::True);
    let false_sym = vm.heap.intern("false");
    vm.env_define_helper(env_id, false_sym, Value::False);

    // Bind all imported symbols
    for (name, &value) in imports {
        let sym = vm.heap.intern(name);
        vm.env_define_helper(env_id, sym, value);
    }

    env_id
}

/// Create an unrestricted environment that inherits from root.
/// Used for modules marked (unrestricted) that need access to natives.
pub fn create_unrestricted_env(
    vm: &mut VM,
    imports: &HashMap<String, Value>,
) -> u32 {
    let root = vm.vat.root_env.unwrap_or(0);
    let env_id = vm.heap.alloc_env(Some(root));

    // Bind all imported symbols (override root bindings if needed)
    for (name, &value) in imports {
        let sym = vm.heap.intern(name);
        vm.env_define_helper(env_id, sym, value);
    }

    env_id
}
