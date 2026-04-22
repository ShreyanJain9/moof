// CapabilityPlugin trait.
//
// A capability is a privileged vat that owns mutable rust state
// (a file handle, a clock, a socket, etc.). It's distinct from
// a pure `Plugin` (moof-core) because it needs `Vat` access —
// capabilities spawn per-vat at scheduler init, and their setup
// function installs native handlers that close over the
// capability's mutable state.
//
// External capability-plugin dylibs depend on this crate
// (moof-runtime) rather than just moof-core.

use crate::vat::Vat;

pub trait CapabilityPlugin {
    /// The name used to bind the FarRef in the REPL (e.g. "console").
    fn name(&self) -> &str;

    /// Set up native handlers on a root object in the given vat.
    /// Returns the root object ID (for FarRef creation).
    fn setup(&self, vat: &mut Vat) -> u32;
}
