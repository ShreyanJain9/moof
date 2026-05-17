// crates/substrate/build.rs — verify Hash mco exists at compile time,
// emit its path as an env var for include_bytes!.
//
// runs from the crates/substrate directory; repo root is two levels up.

use std::path::Path;

fn main() {
    let hash_dir = "../../lib/mcos/hash";
    println!("cargo:rerun-if-changed={}/hash.zig", hash_dir);
    println!("cargo:rerun-if-changed={}/manifest.moof", hash_dir);
    println!("cargo:rerun-if-changed={}/hash.expected-hash", hash_dir);

    let expected_path = Path::new(hash_dir).join("hash.expected-hash");
    if !expected_path.exists() {
        panic!(
            "lib/mcos/hash/hash.expected-hash missing — run lib/mcos/hash/build.sh first"
        );
    }
    let hash_str = std::fs::read_to_string(&expected_path)
        .unwrap()
        .trim()
        .to_string();
    if hash_str.len() != 64 {
        panic!(
            "hash.expected-hash has unexpected length {} (expected 64-char hex)",
            hash_str.len()
        );
    }
    // build.rs runs from `crates/substrate/`; abs path makes include_bytes! work
    // regardless of which directory rustc resolves relative paths from.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = Path::new(&manifest_dir).join("../..").canonicalize().unwrap();
    let cache_file = repo_root
        .join(".moof/mcos/cache")
        .join(format!("{}.mco", hash_str));
    if !cache_file.exists() {
        panic!(
            "Hash mco missing from cache: {} — run lib/mcos/hash/build.sh first",
            cache_file.display()
        );
    }
    println!(
        "cargo:rustc-env=MOOF_HASH_MCO_PATH={}",
        cache_file.display()
    );
}
