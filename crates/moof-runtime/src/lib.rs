// moof-runtime — Vat, Scheduler, manifest, image store.
//
// Binds moof-core + moof-lang into a runnable objectspace:
// a Vat owns a Heap and a VM; the Scheduler spawns/drives vats
// and routes cross-vat messages. Image serialization (LMDB-
// backed store) and dylib plugin loading live here too — they
// straddle the runtime/plugin boundary.
//
// External capability-plugin crates depend on this crate (for
// `CapabilityPlugin` + `Vat`). Pure type-plugin crates depend
// only on moof-core.

pub mod vat;
pub mod scheduler;
pub mod manifest;
pub mod store;
pub mod blobstore;
pub mod dynload;
pub mod capability;

pub use vat::Vat;
pub use scheduler::Scheduler;
pub use manifest::Manifest;
pub use store::{Store, LoadedImage, SerializableClosureDesc};
pub use blobstore::BlobStore;
pub use dynload::{DynTypePlugin, DynCapabilityPlugin};
pub use capability::CapabilityPlugin;
