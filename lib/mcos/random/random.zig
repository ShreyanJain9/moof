//! lib/mcos/random/random.zig — xoshiro256++ PRNG, DataSource generator.
//!
//! exports:
//!   seedFrom:(seed: i64) -> void   — seed the PRNG via SplitMix64 expansion
//!   next() -> i64                  — return the next u64 as a signed i64

const moof = @import("moof");

// ── prng state in linear memory ──────────────────────────────────
// Xoshiro256++ has 256 bits of state (4 × u64).

var state: [4]u64 = .{ 0, 0, 0, 0 };
var initialized: bool = false;

inline fn rotl(x: u64, k: u6) u64 {
    return (x << k) | (x >> @intCast(64 - @as(u7, k)));
}

fn next_u64() u64 {
    const result = rotl(state[0] +% state[3], 23) +% state[0];
    const t = state[1] << 17;
    state[2] ^= state[0];
    state[3] ^= state[1];
    state[1] ^= state[2];
    state[0] ^= state[3];
    state[2] ^= t;
    state[3] = rotl(state[3], 45);
    return result;
}

fn seed_with(s: u64) void {
    // SplitMix64 to expand seed into 4 u64s.
    var z: u64 = s;
    var i: usize = 0;
    while (i < 4) : (i += 1) {
        z +%= 0x9E3779B97F4A7C15;
        var x = z;
        x = (x ^ (x >> 30)) *% 0xBF58476D1CE4E5B9;
        x = (x ^ (x >> 27)) *% 0x94D049BB133111EB;
        x = x ^ (x >> 31);
        state[i] = x;
    }
    initialized = true;
}

// ── exports ───────────────────────────────────────────────────────
// wasm export names = moof selectors. colons are valid in wasm
// function names; zig uses @"..." syntax for identifiers with
// special characters.

export fn @"seedFrom:"(seed: i64) void {
    seed_with(@bitCast(seed));
}

export fn next() i64 {
    if (!initialized) {
        // default seed if user never calls seedFrom:
        seed_with(0);
    }
    return @bitCast(next_u64());
}
