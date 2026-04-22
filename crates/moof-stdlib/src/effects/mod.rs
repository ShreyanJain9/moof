// Effects plugin — types that touch the outside world or cross vat
// boundaries. Each type gets its own module.

use moof_core::Plugin;
use moof_core::heap::Heap;

mod result;
mod farref;
mod act;
mod vat;
mod update;

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn name(&self) -> &str { "effects" }

    fn register(&self, heap: &mut Heap) {
        // order matters only for Result (must exist before
        // anything that might return Err values). the rest are
        // independent siblings.
        result::register(heap);
        farref::register(heap);
        act::register(heap);
        vat::register(heap);
        update::register(heap);
    }
}
