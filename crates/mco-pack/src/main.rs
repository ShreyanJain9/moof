//! mco-pack — multi-purpose mco tooling.
//!
//! subcommands:
//!   mco-pack pack <input.wasm> <output.mco> <manifest-path>
//!   mco-pack index-update <name> <hash>
//!
//! pack: reads the manifest from a file path (not a literal string),
//! appends it as a `moof.manifest` custom wasm section, writes the
//! resulting .mco.
//!
//! index-update: appends (or no-ops if already present) an entry to
//! lib/mcos/index.moof: [$mco-index at: NAME put: HASH]

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <subcommand> [args...]", args[0]);
        eprintln!("subcommands: pack, index-update");
        return ExitCode::from(2);
    }
    match args[1].as_str() {
        "pack" => cmd_pack(&args[2..]),
        "index-update" => cmd_index_update(&args[2..]),
        sub => {
            eprintln!("unknown subcommand: {}", sub);
            ExitCode::from(2)
        }
    }
}

fn cmd_pack(args: &[String]) -> ExitCode {
    if args.len() != 3 {
        eprintln!("usage: pack <input.wasm> <output.mco> <manifest-path>");
        return ExitCode::from(2);
    }
    let in_path = &args[0];
    let out_path = &args[1];
    let manifest_path = &args[2];

    let mut wasm = match fs::read(in_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {}: {}", in_path, e);
            return ExitCode::from(74);
        }
    };

    // sanity check that this looks like a wasm file.
    if wasm.len() < 8 || &wasm[..4] != b"\0asm" {
        eprintln!("{} doesn't have a wasm magic; refusing to pack", in_path);
        return ExitCode::from(65);
    }

    let manifest_src = match fs::read(manifest_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read manifest {}: {}", manifest_path, e);
            return ExitCode::from(74);
        }
    };

    append_custom_section(&mut wasm, "moof.manifest", &manifest_src);

    if let Err(e) = fs::write(out_path, &wasm) {
        eprintln!("write {}: {}", out_path, e);
        return ExitCode::from(74);
    }
    println!("packed {} → {} ({} bytes)", in_path, out_path, wasm.len());
    ExitCode::SUCCESS
}

fn cmd_index_update(args: &[String]) -> ExitCode {
    if args.len() != 2 {
        eprintln!("usage: index-update <name> <hash>");
        return ExitCode::from(2);
    }
    let name = &args[0];
    let hash = &args[1];

    // locate index.moof relative to cwd — the convention is that
    // build scripts run from the repo root.
    let index_path = "lib/mcos/index.moof";

    let content = match fs::read_to_string(index_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {}", index_path, e);
            return ExitCode::from(74);
        }
    };

    // idempotency: if the exact name is already present, skip.
    let entry_marker = format!("\"{}\"", name);
    if content.contains(&entry_marker) {
        println!("index-update: {} already present, skipping", name);
        return ExitCode::SUCCESS;
    }

    // append a new entry line at the end of the file.
    let entry = format!("[$mco-index at: \"{}\" put: \"{}\"]\n", name, hash);
    let new_content = format!("{}{}", content, entry);

    if let Err(e) = fs::write(index_path, &new_content) {
        eprintln!("write {}: {}", index_path, e);
        return ExitCode::from(74);
    }
    println!("index-update: added {} → {}", name, hash);
    ExitCode::SUCCESS
}

/// append a custom section (id=0) with `name` and `payload` to a
/// wasm binary. wasm spec allows custom sections at any position;
/// the end is fine.
///
/// section format:
///
///   [0x00]                         section id (custom)
///   [size: ULEB128]                bytes that follow
///     [name_len: ULEB128]          length of name
///     [name: utf-8]                name string
///     [payload: bytes]             arbitrary
fn append_custom_section(out: &mut Vec<u8>, name: &str, payload: &[u8]) {
    let mut body: Vec<u8> = Vec::new();
    write_uleb128(&mut body, name.len() as u64);
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(payload);

    out.push(0); // custom section id
    write_uleb128(out, body.len() as u64);
    out.extend_from_slice(&body);
}

/// write an unsigned 64-bit integer in little-endian base-128
/// (LEB128) encoding. wasm uses LEB128 for all variable-length
/// integers in its binary format.
fn write_uleb128(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let mut byte = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uleb128_basic() {
        let mut out = Vec::new();
        write_uleb128(&mut out, 0);
        assert_eq!(out, vec![0]);

        out.clear();
        write_uleb128(&mut out, 127);
        assert_eq!(out, vec![127]);

        out.clear();
        write_uleb128(&mut out, 128);
        assert_eq!(out, vec![0x80, 0x01]);

        out.clear();
        write_uleb128(&mut out, 624485);
        assert_eq!(out, vec![0xe5, 0x8e, 0x26]);
    }

    #[test]
    fn append_section_shape() {
        // start with the minimum-valid wasm header.
        let mut wasm: Vec<u8> = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        append_custom_section(&mut wasm, "x", b"hi");
        // expected: header (8) + [0x00, size, name-len(1), 'x', 'h', 'i']
        assert_eq!(wasm.len(), 8 + 1 + 1 + 1 + 1 + 2);
        assert_eq!(&wasm[8..], &[0x00, 0x04, 0x01, b'x', b'h', b'i']);
    }
}
