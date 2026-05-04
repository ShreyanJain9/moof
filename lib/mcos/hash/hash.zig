//! lib/mcos/hash/hash.zig — blake3 (arbitrary-length ≤ 65536 bytes).
//!
//! implements the BLAKE3 hash function per the spec:
//!   https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf
//!
//! strategy: collect all chunk CVs, then iteratively merge pairs bottom-up
//! (BLAKE3's binary subtree structure), applying ROOT only on the final merge.
//! supports up to MAX_CHUNKS = 64 chunks (65536 bytes).
//!
//! exports:
//!   of:(bytes_handle: u32) -> u32   reads Bytes, returns Bytes (32-byte hash)

const moof = @import("moof");

// ── constants ──────────────────────────────────────────────────────

const OUT_LEN:    usize = 32;
const BLOCK_LEN:  usize = 64;
const CHUNK_LEN:  usize = 1024;
const MAX_CHUNKS: usize = 64;   // 64 × 1024 = 65536 bytes max

const IV: [8]u32 = .{
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
};

const MSG_PERMUTATION: [16]usize = .{
    2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8,
};

const CHUNK_START: u32 = 1;
const CHUNK_END:   u32 = 2;
const PARENT:      u32 = 4;
const ROOT:        u32 = 8;

// ── compression ────────────────────────────────────────────────────

inline fn rotr(x: u32, n: u5) u32 {
    return (x >> n) | (x << @intCast(32 - @as(u6, n)));
}

fn g(s: *[16]u32, a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) void {
    s[a] = s[a] +% s[b] +% mx;  s[d] = rotr(s[d] ^ s[a], 16);
    s[c] = s[c] +% s[d];        s[b] = rotr(s[b] ^ s[c], 12);
    s[a] = s[a] +% s[b] +% my;  s[d] = rotr(s[d] ^ s[a],  8);
    s[c] = s[c] +% s[d];        s[b] = rotr(s[b] ^ s[c],  7);
}

fn round_fn(s: *[16]u32, m: *const [16]u32) void {
    g(s, 0, 4,  8, 12, m[0],  m[1]);
    g(s, 1, 5,  9, 13, m[2],  m[3]);
    g(s, 2, 6, 10, 14, m[4],  m[5]);
    g(s, 3, 7, 11, 15, m[6],  m[7]);
    g(s, 0, 5, 10, 15, m[8],  m[9]);
    g(s, 1, 6, 11, 12, m[10], m[11]);
    g(s, 2, 7,  8, 13, m[12], m[13]);
    g(s, 3, 4,  9, 14, m[14], m[15]);
}

fn permute(m: *[16]u32) void {
    var p: [16]u32 = undefined;
    for (MSG_PERMUTATION, 0..) |src, dst| p[dst] = m[src];
    m.* = p;
}

fn compress(
    cv:          *const [8]u32,
    block_words: *const [16]u32,
    counter:     u64,
    block_len:   u32,
    flags:       u32,
) [16]u32 {
    var s: [16]u32 = .{
        cv[0], cv[1], cv[2], cv[3],
        cv[4], cv[5], cv[6], cv[7],
        IV[0], IV[1], IV[2], IV[3],
        @truncate(counter), @truncate(counter >> 32),
        block_len, flags,
    };
    var m: [16]u32 = block_words.*;
    inline for (0..6) |_| { round_fn(&s, &m); permute(&m); }
    round_fn(&s, &m);
    for (0..8) |i| { s[i] ^= s[i + 8]; s[i + 8] ^= cv[i]; }
    return s;
}

// ── helpers ────────────────────────────────────────────────────────

fn le_to_words(bytes: []const u8, w: *[16]u32) void {
    for (w, 0..) |*ww, i| {
        const s = i * 4;
        ww.* = @as(u32, bytes[s])
             | (@as(u32, bytes[s+1]) << 8)
             | (@as(u32, bytes[s+2]) << 16)
             | (@as(u32, bytes[s+3]) << 24);
    }
}

fn cv_to_output(cv: *const [8]u32, out: *[OUT_LEN]u8) void {
    for (0..8) |i| {
        out[i*4+0] = @truncate(cv[i]);
        out[i*4+1] = @truncate(cv[i] >>  8);
        out[i*4+2] = @truncate(cv[i] >> 16);
        out[i*4+3] = @truncate(cv[i] >> 24);
    }
}

// ── chunk → chaining value ─────────────────────────────────────────

/// compute the chaining value (or root output) of one chunk (≤ CHUNK_LEN bytes).
/// counter: 0-based chunk index.
/// root_flag: ROOT for single-chunk root; 0 for all other chunks.
///            ROOT is applied only on the final (CHUNK_END) block of the chunk.
fn chunk_cv(input: []const u8, counter: u64, root_flag: u32, out: *[8]u32) void {
    var cv: [8]u32 = IV;
    var bw: [16]u32 = undefined;

    if (input.len == 0) {
        // empty chunk: single zero-padded block, all flags.
        var blk: [64]u8 = .{0} ** 64;
        le_to_words(&blk, &bw);
        const r = compress(&IV, &bw, counter, 0, CHUNK_START | CHUNK_END | root_flag);
        for (0..8) |k| cv[k] = r[k];
        out.* = cv;
        return;
    }

    var pos: usize = 0;
    while (pos < input.len) {
        // flags for this block (ROOT only on last block with CHUNK_END).
        var flags: u32 = 0;
        if (pos == 0) flags |= CHUNK_START;
        const rem = input.len - pos;
        if (rem <= 64) {
            // last (possibly partial) block.
            var blk: [64]u8 = .{0} ** 64;
            @memcpy(blk[0..rem], input[pos..]);
            le_to_words(&blk, &bw);
            flags |= CHUNK_END | root_flag;
            const r = compress(&cv, &bw, counter, @intCast(rem), flags);
            for (0..8) |k| cv[k] = r[k];
            break;
        } else {
            // full 64-byte non-final block: no CHUNK_END, no ROOT.
            le_to_words(input[pos .. pos + 64], &bw);
            const r = compress(&cv, &bw, counter, 64, flags);
            for (0..8) |k| cv[k] = r[k];
            pos += 64;
        }
    }
    out.* = cv;
}

/// compute parent chaining value (or root output).
/// extra_flags: pass ROOT for the root parent; 0 for interior parents.
fn parent_cv(left: *const [8]u32, right: *const [8]u32, extra_flags: u32, out: *[8]u32) void {
    var bw: [16]u32 = undefined;
    for (0..8) |k| bw[k]   = left[k];
    for (0..8) |k| bw[8+k] = right[k];
    const r = compress(&IV, &bw, 0, BLOCK_LEN, PARENT | extra_flags);
    for (0..8) |k| out[k] = r[k];
}

// ── iterative tree reduction ───────────────────────────────────────
//
// we have n chunk CVs. we want to merge them into a BLAKE3 tree root.
// BLAKE3's tree is a COMPLETE binary tree where internal (parent) nodes
// combine pairs of children. for n leaves:
//   round 1: merge pairs (0,1), (2,3), ... — ceil(n/2) nodes
//   ...
//   final:   merge remaining 2 into 1 with ROOT
//
// we perform this with two static arrays (ping-pong buffers) to avoid
// allocation. ROOT is applied only on the final 2→1 merge.

var cv_buf_a: [MAX_CHUNKS][8]u32 = undefined;
var cv_buf_b: [MAX_CHUNKS][8]u32 = undefined;

/// in-place iterative tree merge. `a` is a slice of CVs (len = n).
/// on return, `out` is written with the single root CV.
fn reduce_to_root(n_in: usize, out: *[8]u32) void {
    // copy chunk_cvs into cv_buf_a first (done by caller).
    var src: *[MAX_CHUNKS][8]u32 = &cv_buf_a;
    var dst: *[MAX_CHUNKS][8]u32 = &cv_buf_b;
    var count: usize = n_in;

    while (count > 2) {
        var half: usize = 0;
        var i: usize = 0;
        while (i + 1 < count) : (i += 2) {
            // interior parent: no ROOT
            parent_cv(&src[i], &src[i+1], 0, &dst[half]);
            half += 1;
        }
        if (count & 1 == 1) {
            // odd: last element carries over unpaired.
            dst[half] = src[count - 1];
            half += 1;
        }
        // swap buffers.
        const tmp = src; src = dst; dst = tmp;
        count = half;
    }

    // count == 1 (single chunk was handled earlier; here n_in >= 2 always).
    // count == 2: final merge with ROOT.
    parent_cv(&src[0], &src[1], ROOT, out);
}

// ── full blake3 ────────────────────────────────────────────────────

fn blake3(input: []const u8, out: *[OUT_LEN]u8) void {
    // fast path: single chunk.
    if (input.len <= CHUNK_LEN) {
        var cv: [8]u32 = undefined;
        chunk_cv(input, 0, ROOT, &cv);
        cv_to_output(&cv, out);
        return;
    }

    // collect chunk CVs into cv_buf_a.
    var num_chunks: usize = 0;
    var pos: usize = 0;
    var counter: u64 = 0;
    while (pos < input.len) {
        const end = @min(pos + CHUNK_LEN, input.len);
        chunk_cv(input[pos..end], counter, 0, &cv_buf_a[num_chunks]);
        num_chunks += 1;
        counter += 1;
        pos = end;
    }

    // reduce to root with ROOT on the final merge.
    var root_cv: [8]u32 = undefined;
    reduce_to_root(num_chunks, &root_cv);
    cv_to_output(&root_cv, out);
}

// ── io buffer ─────────────────────────────────────────────────────

var io_buf: [65536]u8 = undefined;

// ── export ────────────────────────────────────────────────────────

export fn @"of:"(bytes_handle: u32) u32 {
    const input = moof.readBytes(bytes_handle, &io_buf);
    var out: [OUT_LEN]u8 = undefined;
    blake3(input, &out);
    return moof.makeBytes(&out);
}
