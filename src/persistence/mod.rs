/// Persistence layer for MOOF.
///
/// The heap IS the image. "Serialize the slab" is literally the strategy.
/// Content-addressed snapshots + write-ahead log for crash recovery.
///
/// Directory layout:
///   .moof/
///     image.bin       — serialized heap (Vec<HeapObject>)
///     symbols.bin     — symbol intern table (Vec<String>)
///     wal.bin         — write-ahead log (mutations since last snapshot)
///     image.sha256    — hash of current snapshot

pub mod snapshot;
pub mod wal;
