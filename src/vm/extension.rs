/// Extension trait for hooking native I/O into the MOOF runtime.
///
/// Extensions are event sources. Each extension can poll for events
/// (non-blocking), and the scheduler delivers them to vats as messages.
/// GUI, terminal, MCP, network — each extension follows this protocol.

use std::time::Duration;
use super::exec::VM;
use crate::runtime::value::Value;

/// An event produced by an extension for delivery to a vat.
#[derive(Debug)]
pub struct ExtensionEvent {
    /// Which vat should receive this event
    pub target_vat: u32,
    /// The receiver object in that vat
    pub receiver: u32,
    /// The selector symbol
    pub selector: u32,
    /// Arguments
    pub args: Vec<Value>,
}

/// Implement this trait to plug native I/O into the MOOF runtime.
/// Extensions run on the scheduler thread (poll is non-blocking).
/// For blocking I/O, use a background thread and channel internally.
pub trait MoofExtension {
    /// Name for logging/debugging.
    fn name(&self) -> &str;

    /// Startup: register natives, create heap objects.
    fn register(&mut self, vm: &mut VM, root_env: u32);

    /// Event loop: poll for events (non-blocking).
    /// Called once per scheduler round. Return empty vec if nothing ready.
    fn poll(&mut self, timeout: Duration) -> Vec<ExtensionEvent>;

    /// Lifecycle: called on checkpoint/save.
    fn on_checkpoint(&mut self, _vm: &VM) {}

    /// Lifecycle: called on image resume (re-register non-serializable state).
    fn on_resume(&mut self, _vm: &mut VM, _root_env: u32) {}

    /// Roots: return heap object IDs that this extension keeps alive.
    fn gc_roots(&self) -> Vec<u32> { Vec::new() }
}
