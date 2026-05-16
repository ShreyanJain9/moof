//! the bootstrap sexpr reader, ported from reader.rs.
//!
//! provides native source-parsing to the zig substrate, avoiding the
//! heap inflation of the moof-coded parser in a no-GC environment.

const std = @import("std");
const value = @import("value.zig");
const Value = value.Value;
const form = @import("form.zig");
const FormId = form.FormId;
const Form = form.Form;
const world_mod = @import("world.zig");
const World = world_mod.World;
const SymId = world_mod.SymId;

pub const ReadError = struct {
    message: []const u8,
    line: usize,
    col: usize,

    pub fn at(cursor: *const Cursor, message: []const u8) ReadError {
        return .{
            .message = message,
            .line = cursor.line,
            .col = cursor.col,
        };
    }
};

const Cursor = struct {
    bytes: []const u8,
    pos: usize,
    line: usize,
    col: usize,

    fn init(text: []const u8) Cursor {
        return .{
            .bytes = text,
            .pos = 0,
            .line = 1,
            .col = 1,
        };
    }

    fn peek(self: *const Cursor) ?u8 {
        if (self.pos >= self.bytes.len) return null;
        return self.bytes[self.pos];
    }

    fn advance(self: *Cursor) ?u8 {
        const b = self.peek() orelse return null;
        self.pos += 1;
        if (b == '\n') {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        return b;
    }

    fn atEnd(self: *const Cursor) bool {
        return self.pos >= self.bytes.len;
    }
};

fn isDelim(c: u8) bool {
    return switch (c) {
        '(', ')', '[', ']', '{', '}', '\'', '"', ';', '`', ',' => true,
        else => std.ascii.isWhitespace(c),
    };
}

pub fn readOne(world: *World, text: []const u8) !Value {
    var cursor = Cursor.init(text);
    try skipTrivia(&cursor);
    if (cursor.atEnd()) return .nil;
    return try readExpr(world, &cursor);
}

pub fn readAll(world: *World, text: []const u8, allocator: std.mem.Allocator) ![]const Value {
    var cursor = Cursor.init(text);
    var list: std.ArrayList(Value) = .empty;
    errdefer list.deinit(allocator);

    while (true) {
        try skipTrivia(&cursor);
        if (cursor.atEnd()) break;
        try list.append(allocator, try readExpr(world, &cursor));
    }
    return list.toOwnedSlice(allocator);
}

fn skipTrivia(cursor: *Cursor) !void {
    while (cursor.peek()) |c| {
        if (std.ascii.isWhitespace(c)) {
            _ = cursor.advance();
        } else if (c == ';') {
            // comment: ; to end-of-line
            _ = cursor.advance();
            if (cursor.peek()) |c2| {
                if (c2 == ';' or c2 == ':' or c2 == '~') {
                    while (cursor.advance()) |cc| {
                        if (cc == '\n') break;
                    }
                } else {
                    // standalone ; is not a comment
                    return;
                }
            } else return;
        } else {
            break;
        }
    }
}

fn readExpr(world: *World, cursor: *Cursor) anyerror!Value {
    try skipTrivia(cursor);
    const c = cursor.peek() orelse return error.UnexpectedEof;

    return switch (c) {
        '(' => try readList(world, cursor),
        '[' => try readSend(world, cursor),
        '\'' => {
            _ = cursor.advance();
            const inner = try readExpr(world, cursor);
            const quote_sym = try world.syms.intern("quote");
            return try world.makeList(&.{ .{ .sym = quote_sym }, inner });
        },
        '"' => try readString(world, cursor),
        '#' => try readHash(world, cursor),
        ')', ']', '}', ';', '`', ',' => {
            _ = cursor.advance();
            return error.UnexpectedDelimiter;
        },
        else => try readAtom(world, cursor),
    };
}

fn readSend(world: *World, cursor: *Cursor) anyerror!Value {
    _ = cursor.advance(); // [
    var list: std.ArrayList(Value) = .empty;
    defer list.deinit(world.allocator);

    while (true) {
        try skipTrivia(cursor);
        if (cursor.peek()) |c| {
            if (c == ']') {
                _ = cursor.advance();
                break;
            }
        } else return error.UnterminatedSend;

        try list.append(world.allocator, try readExpr(world, cursor));
    }

    const items = list.items;
    if (items.len < 1) return error.EmptySend;

    // [recv sel arg...]
    const recv = items[0];
    if (items.len == 1) return error.SendMissingSelector;

    const first_arg = items[1];
    if (first_arg == .sym) {
        const sel_name = world.syms.resolve(first_arg.sym);
        if (sel_name.len > 0 and sel_name[sel_name.len - 1] == ':') {
            // keyword send: concat keywords, alternating args.
            var sel_buf: std.ArrayList(u8) = .empty;
            defer sel_buf.deinit(world.allocator);
            var args: std.ArrayList(Value) = .empty;
            defer args.deinit(world.allocator);

            var i: usize = 1;
            while (i < items.len) : (i += 2) {
                const kw = items[i];
                if (kw != .sym) return error.ExpectedKeyword;
                const kw_name = world.syms.resolve(kw.sym);
                if (kw_name.len == 0 or kw_name[kw_name.len - 1] != ':') return error.ExpectedKeyword;
                try sel_buf.appendSlice(world.allocator, kw_name);
                if (i + 1 >= items.len) return error.KeywordMissingArg;
                try args.append(world.allocator, items[i + 1]);
            }
            const sel_sym = try world.syms.intern(sel_buf.items);
            var send_list: std.ArrayList(Value) = .empty;
            defer send_list.deinit(world.allocator);
            try send_list.append(world.allocator, .{ .sym = try world.syms.intern("__send__") });
            try send_list.append(world.allocator, recv);
            try send_list.append(world.allocator, .{ .sym = sel_sym });
            try send_list.appendSlice(world.allocator, args.items);
            return try world.makeList(send_list.items);
        } else {
            // positional send: [recv sel arg...]
            var send_list: std.ArrayList(Value) = .empty;
            defer send_list.deinit(world.allocator);
            try send_list.append(world.allocator, .{ .sym = try world.syms.intern("__send__") });
            try send_list.append(world.allocator, recv);
            try send_list.appendSlice(world.allocator, items[1..]);
            return try world.makeList(send_list.items);
        }
    }

    return error.ExpectedSelector;
}

fn readList(world: *World, cursor: *Cursor) anyerror!Value {
    _ = cursor.advance(); // (
    var list: std.ArrayList(Value) = .empty;
    defer list.deinit(world.allocator);

    while (true) {
        try skipTrivia(cursor);
        if (cursor.peek()) |c| {
            if (c == ')') {
                _ = cursor.advance();
                break;
            }
        } else return error.UnterminatedList;

        try list.append(world.allocator, try readExpr(world, cursor));
    }

    return try world.makeList(list.items);
}

fn readAtom(world: *World, cursor: *Cursor) anyerror!Value {
    const start = cursor.pos;
    while (cursor.peek()) |c| {
        if (isDelim(c)) break;
        _ = cursor.advance();
    }
    const text = cursor.bytes[start..cursor.pos];
    if (text.len == 0) return error.EmptyAtom;

    if (std.mem.eql(u8, text, "nil")) return .nil;
    if (std.mem.eql(u8, text, "#true")) return .{ .bool_ = true };
    if (std.mem.eql(u8, text, "#false")) return .{ .bool_ = false };

    // try parse as int
    if (std.fmt.parseInt(i64, text, 0)) |n| {
        return .{ .int = n };
    } else |_| {}

    // try parse as float
    if (std.fmt.parseFloat(f64, text)) |f| {
        return .{ .float = f };
    } else |_| {}

    // else it's a symbol
    return .{ .sym = try world.syms.intern(text) };
}

fn readString(world: *World, cursor: *Cursor) anyerror!Value {
    _ = cursor.advance(); // "
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(world.allocator);

    while (cursor.advance()) |c| {
        if (c == '"') break;
        if (c == '\\') {
            const esc = cursor.advance() orelse return error.UnterminatedString;
            switch (esc) {
                'n' => try buf.append(world.allocator, '\n'),
                't' => try buf.append(world.allocator, '\t'),
                'r' => try buf.append(world.allocator, '\r'),
                '\\' => try buf.append(world.allocator, '\\'),
                '"' => try buf.append(world.allocator, '"'),
                '0' => try buf.append(world.allocator, 0),
                else => return error.InvalidEscape,
            }
        } else {
            try buf.append(world.allocator, c);
        }
    } else return error.UnterminatedString;

    return try world.makeString(buf.items);
}

fn readHash(_: *World, cursor: *Cursor) anyerror!Value {
    _ = cursor.advance(); // #
    const c = cursor.peek() orelse return error.UnexpectedEof;

    if (c == '\\') {
        _ = cursor.advance(); // \
        const start = cursor.pos;
        while (cursor.peek()) |cc| {
            if (isDelim(cc)) break;
            _ = cursor.advance();
        }
        const name = cursor.bytes[start..cursor.pos];
        if (name.len == 1) return .{ .char = @as(u21, name[0]) };
        if (std.mem.eql(u8, name, "space")) return .{ .char = ' ' };
        if (std.mem.eql(u8, name, "newline")) return .{ .char = '\n' };
        if (std.mem.eql(u8, name, "tab")) return .{ .char = '\t' };
        return error.UnknownCharName;
    }

    // fallback to atom (handles #true, #false)
    const start = cursor.pos;
    while (cursor.peek()) |cc| {
        if (isDelim(cc)) break;
        _ = cursor.advance();
    }
    const text = cursor.bytes[start..cursor.pos];
    if (std.mem.eql(u8, text, "true")) return .{ .bool_ = true };
    if (std.mem.eql(u8, text, "false")) return .{ .bool_ = false };

    return error.InvalidHashLiteral;
}
