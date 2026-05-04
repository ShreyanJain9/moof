//! lib/mcos/base64/base64.zig — RFC 4648 standard-alphabet base64.
//!
//! exports:
//!   encode:(bytes_handle: u32) -> u32   reads Bytes, returns String
//!   decode:(str_handle: u32)   -> u32   reads String, returns Bytes
//!                                       raises 'base64-decode on malformed input

const moof = @import("moof");

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

// single 64 KiB buffer: first half = input staging, second half = output.
var io_buf: [65536]u8 = undefined;

// ── exports ───────────────────────────────────────────────────────

export fn @"encode:"(bytes_handle: u32) u32 {
    const input = moof.readBytes(bytes_handle, io_buf[0..32768]);
    const output = io_buf[32768..];
    const out_len = encode_into(input, output);
    return moof.makeString(output[0..out_len]);
}

export fn @"decode:"(str_handle: u32) u32 {
    const input = moof.readString(str_handle, io_buf[0..32768]);
    const output = io_buf[32768..];
    const out_len = decode_into(input, output) catch {
        moof.raise("base64-decode", "malformed base64 input");
    };
    return moof.makeBytes(output[0..out_len]);
}

// ── RFC 4648 standard encode ──────────────────────────────────────

fn encode_into(input: []const u8, output: []u8) usize {
    var i: usize = 0;
    var o: usize = 0;
    while (i + 3 <= input.len) : (i += 3) {
        const b0 = input[i];
        const b1 = input[i + 1];
        const b2 = input[i + 2];
        output[o + 0] = ALPHABET[(b0 >> 2) & 0x3F];
        output[o + 1] = ALPHABET[((b0 << 4) | (b1 >> 4)) & 0x3F];
        output[o + 2] = ALPHABET[((b1 << 2) | (b2 >> 6)) & 0x3F];
        output[o + 3] = ALPHABET[b2 & 0x3F];
        o += 4;
    }
    const rem = input.len - i;
    if (rem == 1) {
        const b0 = input[i];
        output[o + 0] = ALPHABET[(b0 >> 2) & 0x3F];
        output[o + 1] = ALPHABET[(b0 << 4) & 0x3F];
        output[o + 2] = '=';
        output[o + 3] = '=';
        o += 4;
    } else if (rem == 2) {
        const b0 = input[i];
        const b1 = input[i + 1];
        output[o + 0] = ALPHABET[(b0 >> 2) & 0x3F];
        output[o + 1] = ALPHABET[((b0 << 4) | (b1 >> 4)) & 0x3F];
        output[o + 2] = ALPHABET[(b1 << 2) & 0x3F];
        output[o + 3] = '=';
        o += 4;
    }
    return o;
}

// ── RFC 4648 standard decode ──────────────────────────────────────

fn decode_into(input: []const u8, output: []u8) !usize {
    if (input.len % 4 != 0) return error.BadLength;
    var i: usize = 0;
    var o: usize = 0;
    while (i < input.len) : (i += 4) {
        const c0 = decode_char(input[i]) orelse return error.BadChar;
        const c1 = decode_char(input[i + 1]) orelse return error.BadChar;
        const c2: u8 = if (input[i + 2] == '=') 0 else decode_char(input[i + 2]) orelse return error.BadChar;
        const c3: u8 = if (input[i + 3] == '=') 0 else decode_char(input[i + 3]) orelse return error.BadChar;
        output[o + 0] = (c0 << 2) | (c1 >> 4);
        if (input[i + 2] != '=') {
            output[o + 1] = (c1 << 4) | (c2 >> 2);
        }
        if (input[i + 3] != '=') {
            output[o + 2] = (c2 << 6) | c3;
        }
        const pad: usize = if (input[i + 2] == '=') 2 else if (input[i + 3] == '=') 1 else 0;
        o += 3 - pad;
    }
    return o;
}

fn decode_char(c: u8) ?u8 {
    return switch (c) {
        'A'...'Z' => c - 'A',
        'a'...'z' => c - 'a' + 26,
        '0'...'9' => c - '0' + 52,
        '+' => 62,
        '/' => 63,
        else => null,
    };
}
