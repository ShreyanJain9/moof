// Runtime initialization: thin wrapper over the plugin system.
//
// The actual type prototypes and native handlers are defined in
// src/plugins/. This module just provides the entry point that
// the scheduler and vat code call.

use crate::heap::Heap;

/// Register type prototypes and native handlers on the heap.
/// Delegates to the plugin system.
pub fn register_type_protos(heap: &mut Heap) {
    crate::plugins::register_all(heap);
}
