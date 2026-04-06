/// ReplExtension — the REPL as a MoofExtension.
///
/// For now, the REPL is synchronous: main.rs blocks on stdin and evals
/// directly. The extension exists to own REPL-related setup and lifecycle.
/// When we move to async I/O, this will poll a channel fed by a stdin thread.

use std::time::Duration;
use super::extension::{MoofExtension, ExtensionEvent};
use super::exec::VM;

pub struct ReplExtension {
    /// The vat id that the REPL runs in
    pub vat_id: u32,
}

impl ReplExtension {
    pub fn new(vat_id: u32) -> Self {
        ReplExtension { vat_id }
    }
}

impl MoofExtension for ReplExtension {
    fn name(&self) -> &str {
        "repl"
    }

    fn register(&mut self, _vm: &mut VM, _root_env: u32) {
        // REPL natives (io:read-line, etc.) are registered in natives.rs.
        // When we refactor those to be extension-owned, they move here.
    }

    fn poll(&mut self, _timeout: Duration) -> Vec<ExtensionEvent> {
        // Synchronous REPL: input handled in main.rs loop, not via poll.
        // Future: read from a channel fed by a stdin background thread.
        Vec::new()
    }

    fn on_checkpoint(&mut self, _vm: &VM) {
        // Nothing to do — REPL has no persistent state beyond the image.
    }

    fn on_resume(&mut self, _vm: &mut VM, _root_env: u32) {
        // Re-register REPL natives if needed (currently handled by natives.rs).
    }

    fn gc_roots(&self) -> Vec<u32> {
        Vec::new()
    }
}
