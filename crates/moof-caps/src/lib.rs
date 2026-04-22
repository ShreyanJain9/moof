// moof-caps — built-in capability plugins.
//
// Capabilities are privileged vats that own mutable rust state —
// file handles, clock, stdin/stdout, random. They impl
// `moof_runtime::CapabilityPlugin`, so this crate depends on
// moof-runtime (not just moof-core, unlike type plugins).

pub mod capabilities;

pub use capabilities::{ConsoleCapability, ClockCapability, FileCapability, RandomCapability};

use moof_runtime::CapabilityPlugin;

/// Look up a built-in capability plugin by name.
pub fn builtin_capability(name: &str) -> Option<Box<dyn CapabilityPlugin>> {
    match name {
        "console" => Some(Box::new(ConsoleCapability)),
        "clock"   => Some(Box::new(ClockCapability)),
        "file"    => Some(Box::new(FileCapability)),
        "random"  => Some(Box::new(RandomCapability)),
        _ => None,
    }
}
