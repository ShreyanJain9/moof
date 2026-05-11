//! symbol interning.
//!
//! every symbol literal moof reads (`'foo`, `'+`, `'at:put:`) is
//! interned through this table. interning has two purposes:
//!
//! 1. `[a is b]` for two symbols with the same text returns `#true`
//!    in O(1) — comparing SymIds is comparing two `u32`s.
//! 2. send dispatch's hash key is small and cache-friendly.
//!
//! per `laws/substrate-laws.md` L11, symbol identity is stable for
//! the lifetime of a vat. the same symbol text always interns to
//! the same SymId.
//!
//! the symbol *text* is preserved verbatim — case, dashes, colons,
//! everything. there is no normalization. `'Foo` and `'foo` are
//! different symbols.
//!
//! v4-spec correspondence: interning order is the canonical SymId
//! order serialized into V4 §10.3's SymTableSection. on load the
//! table is re-built by reading symbol names in order; SymId N maps
//! to the Nth entry. determinism is per the V0 spec + the V4 §9
//! "same source → same bytecode" rule.
//!
//! NONE (SymId 0) is reserved as the "absent symbol" sentinel; the
//! sentinel entry holds an empty string so `resolve(NONE)` returns
//! `""` instead of crashing.

const std = @import("std");

/// the reserved "absent symbol" sentinel. `intern` never returns
/// this value for a non-empty input.
pub const NONE: u32 = 0;

/// the interning table. owns its allocator (held in the underlying
/// StringHashMap; the entries list shares it on every operation).
pub const SymTable = struct {
    /// SymId → text. `entries[0]` is the empty-string sentinel for
    /// SymId.NONE; user-interned syms start at index 1.
    entries: std.ArrayList([]const u8),
    /// text → SymId. managed StringHashMap (keeps an allocator
    /// reference). keys point into the same byte storage as
    /// `entries` — both share the duplicated copy made at intern
    /// time.
    index: std.StringHashMap(u32),
    /// the allocator used for entry byte storage AND the entries
    /// ArrayList. the StringHashMap stashes its own reference.
    allocator: std.mem.Allocator,

    /// build an empty SymTable. allocates the sentinel entry so
    /// that `resolve(NONE)` is well-defined.
    pub fn init(allocator: std.mem.Allocator) !SymTable {
        var entries: std.ArrayList([]const u8) = .empty;
        // sentinel — `intern("")` would map to 0, but we never want
        // the empty string to round-trip through the user API. the
        // entry stays at index 0 unconditionally.
        try entries.append(allocator, "");
        return .{
            .entries = entries,
            .index = std.StringHashMap(u32).init(allocator),
            .allocator = allocator,
        };
    }

    /// release entry storage + the index. frees every duplicated
    /// symbol-name copy made by `intern`.
    pub fn deinit(self: *SymTable) void {
        // skip index 0 (the empty sentinel which we never dupe'd).
        var i: usize = 1;
        while (i < self.entries.items.len) : (i += 1) {
            self.allocator.free(self.entries.items[i]);
        }
        self.entries.deinit(self.allocator);
        self.index.deinit();
        self.* = undefined;
    }

    /// intern a symbol by name. same text ⇒ same SymId forever.
    ///
    /// allocates a private copy of `name`; the caller does not have
    /// to keep the input slice alive. returns SymId (u32).
    pub fn intern(self: *SymTable, name: []const u8) !u32 {
        if (self.index.get(name)) |id| return id;

        // first-time intern — dupe the bytes into our own storage
        // so we own them for the lifetime of the table.
        const owned = try self.allocator.dupe(u8, name);
        errdefer self.allocator.free(owned);

        const id: u32 = @intCast(self.entries.items.len);
        try self.entries.append(self.allocator, owned);
        errdefer _ = self.entries.pop();

        try self.index.put(owned, id);
        return id;
    }

    /// recover the text for an interned SymId. returns "" for
    /// `NONE`. panics on out-of-range — that indicates a SymId from
    /// a different table or a fabricated one.
    pub fn resolve(self: *const SymTable, id: u32) []const u8 {
        return self.entries.items[id];
    }

    /// `true` if `name` has ever been interned. (constant-time;
    /// useful for tests / reflection.)
    pub fn contains(self: *const SymTable, name: []const u8) bool {
        return self.index.contains(name);
    }

    /// number of interned symbols (excluding the NONE sentinel).
    pub fn len(self: *const SymTable) usize {
        return self.entries.items.len - 1;
    }

    /// drop every interned symbol except the NONE sentinel at
    /// index 0. used by image-load (V4 §10) — the image's sym
    /// table is canonical: SymIds in the image's chunks index
    /// into it, so we must REPLACE the World's table, not append.
    ///
    /// frees the duped name bytes for each non-sentinel entry,
    /// then truncates entries + clears the index hashmap. retains
    /// allocator capacity for the about-to-arrive image syms.
    pub fn clearAndKeepCapacity(self: *SymTable) void {
        // free the duped name bytes for every non-sentinel entry.
        var i: usize = 1;
        while (i < self.entries.items.len) : (i += 1) {
            self.allocator.free(self.entries.items[i]);
        }
        // drop entries down to just the sentinel at index 0.
        self.entries.shrinkRetainingCapacity(1);
        // clear the text → id map (sentinel was never inserted).
        self.index.clearRetainingCapacity();
    }
};
