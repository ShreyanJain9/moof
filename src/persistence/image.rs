/// Directory-based image — the canonical persistence format.
///
/// The image is a directory of .moof source files + a manifest:
///
///   .moof/
///     manifest.moof     — load order, per-module hashes, global hash
///     modules/
///       bootstrap.moof  — full source, comments and all
///       collections.moof
///       ...
///       workspace.moof  — REPL-defined stuff, autosaved
///
/// Each module file IS the source code. The manifest provides integrity
/// checking and deterministic load ordering.

use std::path::{Path, PathBuf};
use std::fs;
use std::collections::BTreeMap;
use sha2::{Sha256, Digest};

/// Manifest entry for one module.
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub name: String,
    pub source_hash: String,
    pub provides_count: usize,
}

/// The image manifest — integrity anchor for the directory-based image.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// Modules in topological load order
    pub modules: Vec<ManifestEntry>,
    /// SHA-256 of all source hashes concatenated in order
    pub global_hash: String,
}

/// Compute SHA-256 hash of bytes, return hex string.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Compute the global hash from per-module hashes in order.
fn compute_global_hash(entries: &[ManifestEntry]) -> String {
    let concatenated: String = entries.iter()
        .map(|e| format!("{}:{}", e.name, e.source_hash))
        .collect::<Vec<_>>()
        .join("\n");
    sha256_hex(concatenated.as_bytes())
}

/// The image directory layout.
pub fn modules_dir(image_dir: &Path) -> PathBuf {
    image_dir.join("modules")
}

pub fn manifest_path(image_dir: &Path) -> PathBuf {
    image_dir.join("manifest.moof")
}

/// Check if a directory-based image exists.
pub fn image_exists(image_dir: &Path) -> bool {
    manifest_path(image_dir).exists()
}

/// Save the manifest to disk.
pub fn save_manifest(image_dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let path = manifest_path(image_dir);
    let mut lines = Vec::new();

    lines.push(format!("; MOOF image manifest — do not edit by hand"));
    lines.push(format!("; global hash: {}", manifest.global_hash));
    lines.push(format!(""));
    lines.push(format!("(image"));
    lines.push(format!("  (hash \"{}\")", manifest.global_hash));
    lines.push(format!("  (modules"));
    for entry in &manifest.modules {
        lines.push(format!("    ({} hash: \"{}\" provides: {})",
            entry.name, entry.source_hash, entry.provides_count));
    }
    lines.push(format!("  ))"));

    fs::write(&path, lines.join("\n"))
        .map_err(|e| format!("cannot write manifest: {}", e))
}

/// Load and verify the manifest from disk.
/// Returns the manifest and verifies the global hash matches.
pub fn load_manifest(image_dir: &Path) -> Result<Manifest, String> {
    let path = manifest_path(image_dir);
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("cannot read manifest: {}", e))?;

    // Parse the manifest — it's a simple format we can parse with string ops
    let mut stored_hash = String::new();
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("; ") || line.is_empty() { continue; }

        // (hash "xxx")
        if line.starts_with("(hash ") {
            stored_hash = extract_quoted(line).unwrap_or_default();
        }

        // (name hash: "xxx" provides: N)
        if line.starts_with('(') && line.contains("hash:") {
            if let Some(entry) = parse_manifest_entry(line) {
                entries.push(entry);
            }
        }
    }

    // Verify global hash
    let computed = compute_global_hash(&entries);
    if computed != stored_hash {
        return Err(format!(
            "manifest hash mismatch: stored={}, computed={} — image may be corrupted",
            &stored_hash[..12], &computed[..12]
        ));
    }

    // Verify each module source exists and its hash matches
    let mod_dir = modules_dir(image_dir);
    for entry in &entries {
        let mod_path = mod_dir.join(format!("{}.moof", entry.name));
        if !mod_path.exists() {
            return Err(format!("module file missing: {}.moof", entry.name));
        }
        let source = fs::read_to_string(&mod_path)
            .map_err(|e| format!("cannot read {}.moof: {}", entry.name, e))?;
        let actual_hash = sha256_hex(source.as_bytes());
        if actual_hash != entry.source_hash {
            return Err(format!(
                "hash mismatch for {}: stored={}, actual={} — module was modified externally",
                entry.name, &entry.source_hash[..12], &actual_hash[..12]
            ));
        }
    }

    Ok(Manifest {
        modules: entries,
        global_hash: stored_hash,
    })
}

/// Save a single module's source to the image directory.
pub fn save_module_source(image_dir: &Path, name: &str, source: &str) -> Result<String, String> {
    let mod_dir = modules_dir(image_dir);
    fs::create_dir_all(&mod_dir)
        .map_err(|e| format!("cannot create modules dir: {}", e))?;

    let path = mod_dir.join(format!("{}.moof", name));
    fs::write(&path, source)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;

    Ok(sha256_hex(source.as_bytes()))
}

/// Read a module's source from the image directory.
pub fn read_module_source(image_dir: &Path, name: &str) -> Result<String, String> {
    let path = modules_dir(image_dir).join(format!("{}.moof", name));
    fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))
}

/// Build a manifest from the current loader state.
pub fn build_manifest(
    load_order: &[String],
    source_hashes: &BTreeMap<String, String>,
    provides_counts: &BTreeMap<String, usize>,
) -> Manifest {
    let entries: Vec<ManifestEntry> = load_order.iter().map(|name| {
        ManifestEntry {
            name: name.clone(),
            source_hash: source_hashes.get(name).cloned().unwrap_or_default(),
            provides_count: provides_counts.get(name).copied().unwrap_or(0),
        }
    }).collect();

    let global_hash = compute_global_hash(&entries);

    Manifest {
        modules: entries,
        global_hash,
    }
}

/// Seed: copy all .moof files from a source directory into the image.
/// Returns the list of files copied.
pub fn seed_from_directory(source_dir: &Path, image_dir: &Path) -> Result<Vec<String>, String> {
    let mod_dir = modules_dir(image_dir);
    fs::create_dir_all(&mod_dir)
        .map_err(|e| format!("cannot create {}: {}", mod_dir.display(), e))?;

    let mut copied = Vec::new();

    let entries = fs::read_dir(source_dir)
        .map_err(|e| format!("cannot read {}: {}", source_dir.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("readdir: {}", e))?;
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "moof") {
            continue;
        }

        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        let dest = mod_dir.join(&filename);
        fs::copy(&path, &dest)
            .map_err(|e| format!("cannot copy {} to {}: {}", path.display(), dest.display(), e))?;

        copied.push(filename);
    }

    Ok(copied)
}

/// Export: write all module sources from the image to a target directory.
pub fn export_to_directory(image_dir: &Path, target_dir: &Path) -> Result<usize, String> {
    let mod_dir = modules_dir(image_dir);
    fs::create_dir_all(target_dir)
        .map_err(|e| format!("cannot create {}: {}", target_dir.display(), e))?;

    let entries = fs::read_dir(&mod_dir)
        .map_err(|e| format!("cannot read {}: {}", mod_dir.display(), e))?;

    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("readdir: {}", e))?;
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "moof") {
            continue;
        }
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        let dest = target_dir.join(&filename);
        fs::copy(&path, &dest)
            .map_err(|e| format!("cannot copy: {}", e))?;
        count += 1;
    }

    Ok(count)
}

// ── helpers ────────────────────────────────────────────────

fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')? + 1;
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_string())
}

fn parse_manifest_entry(line: &str) -> Option<ManifestEntry> {
    // Format: (name hash: "xxx" provides: N)
    let inner = line.trim_start_matches('(').trim_end_matches(')').trim();
    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() < 4 { return None; }

    let name = parts[0].to_string();

    let mut source_hash = String::new();
    let mut provides_count = 0usize;

    let mut i = 1;
    while i < parts.len() {
        match parts[i] {
            "hash:" if i + 1 < parts.len() => {
                source_hash = parts[i + 1].trim_matches('"').to_string();
                i += 2;
            }
            "provides:" if i + 1 < parts.len() => {
                provides_count = parts[i + 1].trim_matches(')').parse().unwrap_or(0);
                i += 2;
            }
            _ => { i += 1; }
        }
    }

    Some(ManifestEntry {
        name,
        source_hash,
        provides_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest_entry() {
        let entry = parse_manifest_entry("(bootstrap hash: \"abc123\" provides: 57)").unwrap();
        assert_eq!(entry.name, "bootstrap");
        assert_eq!(entry.source_hash, "abc123");
        assert_eq!(entry.provides_count, 57);
    }

    #[test]
    fn test_global_hash_deterministic() {
        let entries = vec![
            ManifestEntry { name: "a".into(), source_hash: "aaa".into(), provides_count: 1 },
            ManifestEntry { name: "b".into(), source_hash: "bbb".into(), provides_count: 2 },
        ];
        let h1 = compute_global_hash(&entries);
        let h2 = compute_global_hash(&entries);
        assert_eq!(h1, h2);
        assert!(!h1.is_empty());
    }
}
