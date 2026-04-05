/// Persistence layer for MOOF.
///
/// The image is a directory of .moof source files + a manifest:
///
///   .moof/
///     manifest.moof     — load order, per-module hashes, global hash
///     modules/
///       bootstrap.moof  — full source, comments and all
///       collections.moof
///       ...
///
/// Source files are the canonical representation. The manifest provides
/// integrity checking and deterministic load ordering.

pub mod image;
