//! lib/mcos/hash/hash.zig — blake3 from scratch (single-chunk, ≤1024 bytes).
//!
//! implements the BLAKE3 hash function for inputs up to CHUNK_LEN (1024 bytes).
//! single-chunk mode is sufficient for all moof mcos (which are a few hundred
//! bytes). chunk-tree for larger inputs is deferred (raises 'blake3-too-long).
//!
//! reference: https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf
//!
//! exports:
//!   of_(bytes_handle: u32) -> u32   reads Bytes input, returns Bytes (32-byte hash)

const moof = @import("moof");

// ── blake3 constants ───────────────────────────────────────────────

const OUT_LEN: usize = 32;
const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

/// blake3 IV — first 8 words of SHA-256 initialization constants.
const IV: [8]u32 = .{
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
};

/// message schedule permutation applied between rounds.
/// permuted[dst] = m[MSG_PERMUTATION[dst]]
const MSG_PERMUTATION: [16]usize = .{ 2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8 };

// ── domain flags ──────────────────────────────────────────────────

const CHUNK_START: u32 = 1;
const CHUNK_END: u32 = 2;
// PARENT = 4 (not needed for single-chunk)
const ROOT: u32 = 8;
// KEYED_HASH = 16, DERIVE_KEY_CONTEXT = 32, DERIVE_KEY_MATERIAL = 64 — not needed

// ── compression function ───────────────────────────────────────────

inline fn rotr(x: u32, n: u5) u32 {
    return (x >> n) | (x << @intCast(32 - @as(u6, n)));
}

/// the BLAKE3 quarter-round (G function). mixes two message words mx, my
/// into the state columns at positions a, b, c, d.
fn g(state: *[16]u32, a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) void {
    state[a] = state[a] +% state[b] +% mx;
    state[d] = rotr(state[d] ^ state[a], 16);
    state[c] = state[c] +% state[d];
    state[b] = rotr(state[b] ^ state[c], 12);
    state[a] = state[a] +% state[b] +% my;
    state[d] = rotr(state[d] ^ state[a], 8);
    state[c] = state[c] +% state[d];
    state[b] = rotr(state[b] ^ state[c], 7);
}

/// one round: 4 column mixes + 4 diagonal mixes.
fn round(state: *[16]u32, m: *const [16]u32) void {
    // columns
    g(state, 0, 4, 8,  12, m[0],  m[1]);
    g(state, 1, 5, 9,  13, m[2],  m[3]);
    g(state, 2, 6, 10, 14, m[4],  m[5]);
    g(state, 3, 7, 11, 15, m[6],  m[7]);
    // diagonals
    g(state, 0, 5, 10, 15, m[8],  m[9]);
    g(state, 1, 6, 11, 12, m[10], m[11]);
    g(state, 2, 7, 8,  13, m[12], m[13]);
    g(state, 3, 4, 9,  14, m[14], m[15]);
}

/// apply the message permutation table to a block of 16 words.
fn permute(m: *[16]u32) void {
    var permuted: [16]u32 = undefined;
    for (MSG_PERMUTATION, 0..) |src, dst| {
        permuted[dst] = m[src];
    }
    m.* = permuted;
}

/// the BLAKE3 compression function. takes:
///   chaining_value — 8 u32s (the running CV, or IV for the first block)
///   block_words    — 16 u32s (64-byte block, little-endian decoded)
///   counter        — u64 (chunk counter; 0 for single-chunk)
///   block_len      — u32 (number of message bytes in this block; ≤64)
///   flags          — u32 (combination of domain separation flags above)
/// returns the 16-word output state.
fn compress(
    chaining_value: *const [8]u32,
    block_words: *const [16]u32,
    counter: u64,
    block_len: u32,
    flags: u32,
) [16]u32 {
    var state: [16]u32 = .{
        chaining_value[0], chaining_value[1], chaining_value[2], chaining_value[3],
        chaining_value[4], chaining_value[5], chaining_value[6], chaining_value[7],
        IV[0], IV[1], IV[2], IV[3],
        @truncate(counter),
        @truncate(counter >> 32),
        block_len,
        flags,
    };
    var m: [16]u32 = block_words.*;
    // 7 rounds (6 permute + final round without permute)
    inline for (0..6) |_| {
        round(&state, &m);
        permute(&m);
    }
    round(&state, &m);
    // XOR upper half into lower half and CV into upper half
    for (0..8) |i| {
        state[i] ^= state[i + 8];
        state[i + 8] ^= chaining_value[i];
    }
    return state;
}

/// decode 64 bytes (little-endian) into 16 u32 words.
fn words_from_le_bytes(bytes: []const u8, words: []u32) void {
    for (words, 0..) |*w, i| {
        const s = i * 4;
        w.* = @as(u32, bytes[s])
            | (@as(u32, bytes[s + 1]) << 8)
            | (@as(u32, bytes[s + 2]) << 16)
            | (@as(u32, bytes[s + 3]) << 24);
    }
}

// ── single-chunk hashing ───────────────────────────────────────────

/// compute blake3(input) for inputs ≤ CHUNK_LEN (1024 bytes).
/// writes 32 bytes to out.
fn hash_single_chunk(input: []const u8, out: *[OUT_LEN]u8) void {
    // the chaining value starts as the IV.
    var cv: [8]u32 = IV;
    var block_words: [16]u32 = undefined;

    // iterate over 64-byte blocks. the last block is padded with zeros.
    var i: usize = 0;
    while (i < input.len) : (i += 64) {
        // flags for this block.
        var flags: u32 = 0;
        if (i == 0) flags |= CHUNK_START;

        // is this the last block?
        const remaining = input.len - i;
        const is_last = remaining <= 64;

        if (is_last) {
            // last (possibly partial) block: pad with zeros.
            var last_block: [64]u8 = .{0} ** 64;
            const to_copy = if (remaining < 64) remaining else 64;
            @memcpy(last_block[0..to_copy], input[i..i + to_copy]);
            words_from_le_bytes(&last_block, &block_words);
            flags |= CHUNK_END | ROOT;
            const result = compress(&cv, &block_words, 0, @intCast(to_copy), flags);
            for (0..8) |k| cv[k] = result[k];
            break;
        } else {
            // full 64-byte block.
            words_from_le_bytes(input[i..i + 64], &block_words);
            const result = compress(&cv, &block_words, 0, 64, flags);
            for (0..8) |k| cv[k] = result[k];
        }
    }

    // special case: empty input — one zero block with CHUNK_START|CHUNK_END|ROOT.
    if (input.len == 0) {
        var empty_block: [16]u32 = .{0} ** 16;
        const flags: u32 = CHUNK_START | CHUNK_END | ROOT;
        const result = compress(&IV, &empty_block, 0, 0, flags);
        for (0..8) |k| cv[k] = result[k];
    }

    // serialize output: cv[0..8] each as 4 little-endian bytes.
    for (0..8) |w| {
        out[w * 4 + 0] = @truncate(cv[w]);
        out[w * 4 + 1] = @truncate(cv[w] >> 8);
        out[w * 4 + 2] = @truncate(cv[w] >> 16);
        out[w * 4 + 3] = @truncate(cv[w] >> 24);
    }
}

// ── io buffer ─────────────────────────────────────────────────────

var io_buf: [2048]u8 = undefined;

// ── exported mco method ───────────────────────────────────────────

/// [Hash of: bytes] → Bytes (32-byte blake3 hash).
/// raises 'blake3-too-long for inputs > CHUNK_LEN (1024 bytes).
export fn @"of:"(bytes_handle: u32) u32 {
    const input = moof.readBytes(bytes_handle, &io_buf);
    if (input.len > CHUNK_LEN) {
        moof.raise("blake3-too-long", "blake3 chunk-tree not implemented; input exceeds 1024 bytes");
    }
    var out: [OUT_LEN]u8 = undefined;
    hash_single_chunk(input, &out);
    return moof.makeBytes(&out);
}
