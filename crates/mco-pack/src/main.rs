//! mco-pack — append a moof manifest custom section to a wasm
//! file, producing a .mco.
//!
//! per `docs/reference/mco-format.md`, an `.mco` is a `.wasm` file
//! plus moof-specific custom sections. this tool appends the
//! `moof.manifest` section, which holds the manifest as moof
//! source-text (parseable by the substrate's reader, no new
//! format).
//!
//! usage:
//!
//!   mco-pack <input.wasm> <output.mco> <manifest-source>
//!
//! the manifest source is the literal moof source-text:
//!
//!   ((abi-version 1)
//!    (parent Object)
//!    (methods (now monotonic)))
//!
//! that gets embedded into the wasm file as a custom section
//! named "moof.manifest". the substrate's wasm loader parses
//! it back via the same reader that parses every other moof
//! source. zero new format, zero new parser.

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: {} <input.wasm> <output.mco> <manifest-source>", args[0]);
        return ExitCode::from(2);
    }
    let in_path = &args[1];
    let out_path = &args[2];
    let manifest_src = &args[3];

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

    append_custom_section(&mut wasm, "moof.manifest", manifest_src.as_bytes());

    if let Err(e) = fs::write(out_path, &wasm) {
        eprintln!("write {}: {}", out_path, e);
        return ExitCode::from(74);
    }
    println!("packed {} → {} ({} bytes)", in_path, out_path, wasm.len());
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
