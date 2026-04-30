//! the bootstrap sexpr reader.
//!
//! parses the *minimum* surface that lets us bootstrap the rest of
//! moof: numbers (decimal + hex/binary/octal + underscore grouping),
//! symbols (kebab-case + operator + keyword-with-trailing-colon),
//! strings (`"…"` with `\n \t \\ \"` escapes), lists, quote sugar
//! (`'foo`), and the literal `nil`, `#true`, `#false`.
//!
//! deferred to phase A.10+ (when parser.moof takes over):
//! - tables (`#[…]`), object literals (`{…}`), send brackets (`[…]`)
//! - string interpolation (`#{…}`), char literals (`#\…`)
//! - quasiquote / unquote / splice
//! - raw and triple-quoted strings
//! - floats, rationals, complex, scientific notation
//! - tagged literals (`#Tag …`)
//!
//! per `process/docs-driven.md`, this reader is *throwaway
//! scaffolding*. once `parser.moof` lands, the bootstrap reader is
//! quarantined behind a debug flag, used only to load
//! parser.moof itself.

use crate::form::Form;
use crate::heap::Heap;
use crate::sym::{SymId, SymTable};
use crate::value::Value;

/// reader error with a human-friendly position (1-based line + col).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl ReadError {
    fn at(c: &Cursor, message: impl Into<String>) -> Self {
        ReadError {
            message: message.into(),
            line: c.line,
            col: c.col,
        }
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "read error at {}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ReadError {}

/// the reader's slot symbols. cached as ids so we aren't
/// re-interning on every cons-cell allocation.
pub struct ReadCtx<'a> {
    head_sym: SymId,
    tail_sym: SymId,
    quote_sym: SymId,
    send_sym: SymId,
    self_sym: SymId,
    /// the proto FormId to assign to every cons cell. typically
    /// `Value::Form(list_proto)`. phase-A clients that don't yet
    /// have a List proto pass `Value::Nil`.
    pub list_proto: Value,
    pub heap: &'a mut Heap,
    pub syms: &'a mut SymTable,
}

impl<'a> ReadCtx<'a> {
    pub fn new(heap: &'a mut Heap, syms: &'a mut SymTable, list_proto: Value) -> Self {
        let head_sym = syms.intern("head");
        let tail_sym = syms.intern("tail");
        let quote_sym = syms.intern("quote");
        let send_sym = syms.intern("__send__");
        let self_sym = syms.intern("self");
        ReadCtx {
            head_sym,
            tail_sym,
            quote_sym,
            send_sym,
            self_sym,
            list_proto,
            heap,
            syms,
        }
    }
}

/// position-tracking iterator over the input.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Cursor {
            bytes: text.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(b)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }
}

/// `true` if `c` is a delimiter that terminates an atom.
fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'[' | b']' | b'\'' | b'"' | b';') || c.is_ascii_whitespace()
}

/// `true` if every byte of `name` is a binary-operator character
/// per `docs/syntax/sends-and-calls.md`. `name` must be non-empty.
fn is_binary_op(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            matches!(
                b,
                b'+' | b'-'
                    | b'*'
                    | b'/'
                    | b'<'
                    | b'>'
                    | b'='
                    | b'!'
                    | b'?'
                    | b'|'
                    | b'&'
                    | b'~'
                    | b'^'
                    | b'%'
            )
        })
}

/// skip whitespace and `;`-line-comments.
fn skip_trivia(c: &mut Cursor) {
    loop {
        match c.peek() {
            Some(b) if b.is_ascii_whitespace() => {
                c.advance();
            }
            Some(b';') => {
                // skip until end-of-line
                while let Some(b) = c.peek() {
                    if b == b'\n' {
                        break;
                    }
                    c.advance();
                }
            }
            _ => return,
        }
    }
}

/// read a single moof Form from `text`. ignores trailing whitespace
/// and comments. fails if there's a non-whitespace remainder.
pub fn read(text: &str, ctx: &mut ReadCtx<'_>) -> Result<Value, ReadError> {
    let mut c = Cursor::new(text);
    skip_trivia(&mut c);
    let v = read_form(&mut c, ctx)?;
    skip_trivia(&mut c);
    if !c.at_end() {
        return Err(ReadError::at(&c, "unexpected trailing content"));
    }
    Ok(v)
}

/// read all forms from `text`. each form is a top-level expression.
pub fn read_all(text: &str, ctx: &mut ReadCtx<'_>) -> Result<Vec<Value>, ReadError> {
    let mut c = Cursor::new(text);
    let mut out = Vec::new();
    loop {
        skip_trivia(&mut c);
        if c.at_end() {
            return Ok(out);
        }
        out.push(read_form(&mut c, ctx)?);
    }
}

fn read_form(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    skip_trivia(c);
    match c.peek() {
        None => Err(ReadError::at(c, "unexpected end of input")),
        Some(b'(') => read_list(c, ctx),
        Some(b')') => Err(ReadError::at(c, "unexpected `)`")),
        Some(b'[') => read_send_bracket(c, ctx),
        Some(b']') => Err(ReadError::at(c, "unexpected `]`")),
        Some(b'\'') => read_quote(c, ctx),
        Some(b'"') => read_string(c, ctx),
        Some(b'#') => read_hash(c, ctx),
        _ => read_atom(c, ctx),
    }
}

fn read_list(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'('));
    c.advance(); // consume `(`
    let mut elements = Vec::new();
    loop {
        skip_trivia(c);
        match c.peek() {
            None => return Err(ReadError::at(c, "unterminated list")),
            Some(b')') => {
                c.advance();
                return Ok(build_list(ctx, &elements));
            }
            _ => {
                elements.push(read_form(c, ctx)?);
            }
        }
    }
}

/// `'expr` ⇒ `(quote expr)`.
fn read_quote(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'\''));
    c.advance();
    let inner = read_form(c, ctx)?;
    let quote_sym_v = Value::Sym(ctx.quote_sym);
    Ok(build_list(ctx, &[quote_sym_v, inner]))
}

/// `[recv sel args…]` — smalltalk-flavored send.
///
/// shapes (`docs/syntax/brackets.md`):
/// - `[recv]` — error: too few elements.
/// - `[recv selector]` — unary send.
/// - `[recv OP arg]` — binary send (OP is operator-chars only).
/// - `[recv selector arg arg …]` — positional send (selector is a
///   bareword without trailing `:`).
/// - `[recv kw1: arg1 kw2: arg2 …]` — multi-keyword send; selector
///   is the concatenation of the keyword markers.
///
/// the result is a list `(__send__ recv 'selector arg…)` that the
/// compiler recognizes and lowers to a `Send` opcode.
fn read_send_bracket(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'['));
    let start_line = c.line;
    let start_col = c.col;
    c.advance();
    let mut elements = Vec::new();
    loop {
        skip_trivia(c);
        match c.peek() {
            None => {
                return Err(ReadError {
                    message: "unterminated send bracket".into(),
                    line: start_line,
                    col: start_col,
                });
            }
            Some(b']') => {
                c.advance();
                return build_send_form(ctx, &elements, start_line, start_col);
            }
            _ => {
                elements.push(read_form(c, ctx)?);
            }
        }
    }
}

fn build_send_form(
    ctx: &mut ReadCtx,
    elements: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, ReadError> {
    if elements.is_empty() {
        return Err(ReadError {
            message: "empty send bracket `[]`".into(),
            line,
            col,
        });
    }
    if elements.len() == 1 {
        return Err(ReadError {
            message: "send needs at least a selector".into(),
            line,
            col,
        });
    }
    let receiver = elements[0];
    let rest = &elements[1..];

    let first = rest[0];
    let first_sym = first.as_sym().ok_or_else(|| ReadError {
        message: "selector must be a symbol".into(),
        line,
        col,
    })?;
    let first_text = ctx.syms.resolve(first_sym).to_string();

    // binary: `[a OP b]` — exactly 3 elements, middle is operator-only.
    if is_binary_op(&first_text) && rest.len() == 2 {
        return Ok(emit_send(ctx, receiver, first_sym, &rest[1..]));
    }

    // keyword: first selector ends in `:` — parse pairs.
    if first_text.ends_with(':') {
        return parse_keyword_send(ctx, receiver, rest, line, col);
    }

    // unary or positional: first sym is the selector, rest are args.
    Ok(emit_send(ctx, receiver, first_sym, &rest[1..]))
}

fn parse_keyword_send(
    ctx: &mut ReadCtx,
    receiver: Value,
    rest: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, ReadError> {
    let mut sel_text = String::new();
    let mut args = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let kw = rest[i].as_sym().ok_or_else(|| ReadError {
            message: "keyword send: expected `kw:` symbol".into(),
            line,
            col,
        })?;
        let kw_text = ctx.syms.resolve(kw).to_string();
        if !kw_text.ends_with(':') {
            return Err(ReadError {
                message: format!("keyword `{}` must end with `:`", kw_text),
                line,
                col,
            });
        }
        sel_text.push_str(&kw_text);
        i += 1;
        if i >= rest.len() {
            return Err(ReadError {
                message: format!("keyword `{}` needs an argument", kw_text),
                line,
                col,
            });
        }
        args.push(rest[i]);
        i += 1;
    }
    let sel_sym = ctx.syms.intern(&sel_text);
    Ok(emit_send(ctx, receiver, sel_sym, &args))
}

/// build the marker-tagged list: `(__send__ recv 'sel args…)`.
fn emit_send(
    ctx: &mut ReadCtx,
    receiver: Value,
    selector: SymId,
    args: &[Value],
) -> Value {
    let mut entries = Vec::with_capacity(args.len() + 3);
    entries.push(Value::Sym(ctx.send_sym));
    entries.push(receiver);
    entries.push(Value::Sym(selector));
    entries.extend_from_slice(args);
    build_list(ctx, &entries)
}

/// build a moof list from a slice of values. constructs cons cells
/// in the heap, terminated by `nil`. each cell's slots are
/// `{head: v, tail: rest}`.
fn build_list(ctx: &mut ReadCtx, elements: &[Value]) -> Value {
    let mut tail = Value::Nil;
    for &v in elements.iter().rev() {
        let mut cell = Form::with_proto(ctx.list_proto);
        cell.slots.insert(ctx.head_sym, v);
        cell.slots.insert(ctx.tail_sym, tail);
        let id = ctx.heap.alloc(cell);
        tail = Value::Form(id);
    }
    tail
}

/// read `#…` — hash-prefixed forms. phase A handles only `#true`,
/// `#false`. tables/tagged-literals/chars come later.
fn read_hash(c: &mut Cursor, _ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'#'));
    let start_line = c.line;
    let start_col = c.col;
    c.advance();
    // peek the rest until a delimiter; that's the bareword.
    let mut word = String::new();
    while let Some(b) = c.peek() {
        if is_delim(b) {
            break;
        }
        word.push(b as char);
        c.advance();
    }
    match word.as_str() {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        other => Err(ReadError {
            message: format!(
                "unknown hash form `#{}` (phase-A reader supports `#true` and `#false` only)",
                other
            ),
            line: start_line,
            col: start_col,
        }),
    }
}

/// read a `"…"` string with `\n \t \\ \"` escapes.
fn read_string(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'"'));
    let start_line = c.line;
    let start_col = c.col;
    c.advance();
    let mut s = String::new();
    loop {
        match c.peek() {
            None => {
                return Err(ReadError {
                    message: "unterminated string".into(),
                    line: start_line,
                    col: start_col,
                });
            }
            Some(b'"') => {
                c.advance();
                // strings are interned as symbols at phase A — we
                // don't have a String proto yet. once String lands
                // we'll allocate a String Form here. for now,
                // intern-as-symbol is the honest placeholder; tests
                // assert this.
                let id = ctx.syms.intern(&s);
                return Ok(Value::Sym(id));
            }
            Some(b'\\') => {
                c.advance();
                match c.advance() {
                    Some(b'n') => s.push('\n'),
                    Some(b't') => s.push('\t'),
                    Some(b'\\') => s.push('\\'),
                    Some(b'"') => s.push('"'),
                    Some(other) => {
                        return Err(ReadError::at(
                            c,
                            format!("unknown escape: \\{}", other as char),
                        ));
                    }
                    None => {
                        return Err(ReadError::at(c, "unterminated escape"));
                    }
                }
            }
            Some(b) => {
                s.push(b as char);
                c.advance();
            }
        }
    }
}

/// read a bare atom: number, symbol, or `.foo` self-send.
fn read_atom(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    let start_line = c.line;
    let start_col = c.col;
    let mut text = String::new();
    while let Some(b) = c.peek() {
        if is_delim(b) {
            break;
        }
        text.push(b as char);
        c.advance();
    }
    if text.is_empty() {
        return Err(ReadError {
            message: "expected atom".into(),
            line: start_line,
            col: start_col,
        });
    }
    if text == "nil" {
        return Ok(Value::Nil);
    }
    // `.foo` shorthand: substitutes `[self foo]`. lowered to a
    // send form like `[…]` does — `(__send__ self 'foo)`.
    // (`docs/syntax/sigils.md`.)
    if let Some(rest) = text.strip_prefix('.') {
        if !rest.is_empty() && rest != "." {
            let foo_sym = ctx.syms.intern(rest);
            return Ok(emit_send(ctx, Value::Sym(ctx.self_sym), foo_sym, &[]));
        }
    }
    // numeric? either pure decimal/+/- prefix, or 0x/0b/0o.
    if let Some(v) = try_parse_number(&text) {
        return Ok(v);
    }
    Ok(Value::Sym(ctx.syms.intern(&text)))
}

/// try to parse `text` as a moof integer literal. returns `None`
/// if it doesn't look like a number, in which case the caller
/// treats it as a symbol.
fn try_parse_number(text: &str) -> Option<Value> {
    // strip underscores from anywhere; they're for readability only.
    // (`syntax/literals.md`.)
    let cleaned: String = text.chars().filter(|&c| c != '_').collect();
    let cleaned = cleaned.as_str();
    // empty after stripping? (e.g., user wrote `"_"`, which we'd
    // never see because `_` isn't a delim, but defensive.)
    if cleaned.is_empty() {
        return None;
    }

    // sign-aware base prefix detection.
    let (sign, rest): (i64, &str) = match cleaned.as_bytes()[0] {
        b'-' => (-1, &cleaned[1..]),
        b'+' => (1, &cleaned[1..]),
        _ => (1, cleaned),
    };
    if rest.is_empty() {
        return None;
    }

    // decimal-only fast path (no base prefix).
    if let Some(stripped) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        return i64::from_str_radix(stripped, 16).ok().map(|n| Value::Int(sign * n));
    }
    if let Some(stripped) = rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")) {
        return i64::from_str_radix(stripped, 2).ok().map(|n| Value::Int(sign * n));
    }
    if let Some(stripped) = rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")) {
        return i64::from_str_radix(stripped, 8).ok().map(|n| Value::Int(sign * n));
    }
    // pure decimal — must start with a digit. (a leading `-` sign
    // alone would have been caught above; what's left here must
    // start with `0..9`.)
    if !rest.bytes().next().map_or(false, |b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse::<i64>().ok().map(|n| Value::Int(sign * n))
}

/// helper for tests + downstream: walk a moof list-Form into a
/// `Vec<Value>`. returns `Err` if `value` isn't `nil` or a
/// well-formed cons-cell chain.
pub fn list_to_vec(value: Value, ctx: &ReadCtx<'_>) -> Result<Vec<Value>, &'static str> {
    let mut out = Vec::new();
    let mut cur = value;
    loop {
        match cur {
            Value::Nil => return Ok(out),
            Value::Form(id) => {
                let f = ctx.heap.get(id);
                let head = f.slot(ctx.head_sym);
                let tail = f.slot(ctx.tail_sym);
                out.push(head);
                cur = tail;
            }
            _ => return Err("not a list"),
        }
    }
}

/// `head` selector for the cons-cell convention. exposed for
/// the compiler's convenience.
pub fn head_sym(syms: &mut SymTable) -> SymId {
    syms.intern("head")
}

/// `tail` selector for the cons-cell convention.
pub fn tail_sym(syms: &mut SymTable) -> SymId {
    syms.intern("tail")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (Heap, SymTable) {
        (Heap::new(), SymTable::new())
    }

    fn ctx<'a>(heap: &'a mut Heap, syms: &'a mut SymTable) -> ReadCtx<'a> {
        ReadCtx::new(heap, syms, Value::Nil)
    }

    #[test]
    fn nil_literal() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        assert_eq!(read("nil", &mut c).unwrap(), Value::Nil);
    }

    #[test]
    fn boolean_literals() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        assert_eq!(read("#true", &mut c).unwrap(), Value::Bool(true));
        assert_eq!(read("#false", &mut c).unwrap(), Value::Bool(false));
    }

    #[test]
    fn integer_literals_decimal() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        assert_eq!(read("0", &mut c).unwrap(), Value::Int(0));
        assert_eq!(read("42", &mut c).unwrap(), Value::Int(42));
        assert_eq!(read("-7", &mut c).unwrap(), Value::Int(-7));
        assert_eq!(read("+7", &mut c).unwrap(), Value::Int(7));
    }

    #[test]
    fn integer_literals_bases() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        assert_eq!(read("0xff", &mut c).unwrap(), Value::Int(255));
        assert_eq!(read("0xFF", &mut c).unwrap(), Value::Int(255));
        assert_eq!(read("0b1010", &mut c).unwrap(), Value::Int(10));
        assert_eq!(read("0o17", &mut c).unwrap(), Value::Int(15));
        assert_eq!(read("-0xff", &mut c).unwrap(), Value::Int(-255));
    }

    #[test]
    fn integer_literals_underscore_grouping() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        assert_eq!(read("1_000_000", &mut c).unwrap(), Value::Int(1_000_000));
        assert_eq!(read("0xDEAD_BEEF", &mut c).unwrap(), Value::Int(0xDEAD_BEEF));
        assert_eq!(read("0b1100_0011", &mut c).unwrap(), Value::Int(0b1100_0011));
    }

    #[test]
    fn symbols() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let foo = read("foo", &mut c).unwrap();
        let foo_again = read("foo", &mut c).unwrap();
        assert_eq!(foo, foo_again, "interning means same name = same id");
        // operator-as-symbol
        let plus = read("+", &mut c).unwrap();
        assert!(plus.as_sym().is_some());
        assert_ne!(plus, foo);
        // keyword-style selector
        let kw = read("at:put:", &mut c).unwrap();
        assert!(kw.as_sym().is_some());
        assert_eq!(c.syms.resolve(kw.as_sym().unwrap()), "at:put:");
    }

    #[test]
    fn empty_list_is_nil() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        assert_eq!(read("()", &mut c).unwrap(), Value::Nil);
    }

    #[test]
    fn list_three_ints() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("(1 2 3)", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        assert_eq!(elems, vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
    }

    #[test]
    fn list_nested() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("(foo (bar baz) qux)", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        assert_eq!(elems.len(), 3);
        assert_eq!(c.syms.resolve(elems[0].as_sym().unwrap()), "foo");
        let inner = list_to_vec(elems[1], &c).unwrap();
        assert_eq!(inner.len(), 2);
        assert_eq!(c.syms.resolve(inner[0].as_sym().unwrap()), "bar");
    }

    #[test]
    fn quote_sugar() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("'foo", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        // ('quote foo)
        assert_eq!(elems.len(), 2);
        assert_eq!(c.syms.resolve(elems[0].as_sym().unwrap()), "quote");
        assert_eq!(c.syms.resolve(elems[1].as_sym().unwrap()), "foo");
    }

    #[test]
    fn quote_of_list() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("'(1 2 3)", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        assert_eq!(elems.len(), 2);
        assert_eq!(c.syms.resolve(elems[0].as_sym().unwrap()), "quote");
        let inner = list_to_vec(elems[1], &c).unwrap();
        assert_eq!(
            inner,
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    #[test]
    fn strings_basic() {
        // strings intern-as-symbols in phase A; tests assert this
        // honestly so the placeholder is visible.
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("\"hello\"", &mut c).unwrap();
        let s = v.as_sym().unwrap();
        assert_eq!(c.syms.resolve(s), "hello");
    }

    #[test]
    fn strings_with_escapes() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("\"a\\nb\\tc\\\"d\\\\e\"", &mut c).unwrap();
        let s = v.as_sym().unwrap();
        assert_eq!(c.syms.resolve(s), "a\nb\tc\"d\\e");
    }

    #[test]
    fn comments_are_ignored() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read(
            "; a comment\n  42  ; trailing comment\n",
            &mut c,
        ).unwrap();
        assert_eq!(v, Value::Int(42));
    }

    #[test]
    fn read_all_top_level_forms() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let forms = read_all("1 2 3", &mut c).unwrap();
        assert_eq!(
            forms,
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    #[test]
    fn errors_have_position() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let err = read("(foo bar", &mut c).unwrap_err();
        assert_eq!(err.message, "unterminated list");
        // unterminated list is reported at end-of-input.

        let err = read("(foo )) extra", &mut c).unwrap_err();
        assert!(err.message.contains("unexpected"));
    }

    #[test]
    fn errors_unterminated_string() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let err = read("\"hello", &mut c).unwrap_err();
        assert_eq!(err.message, "unterminated string");
    }

    #[test]
    fn errors_unknown_hash_form() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let err = read("#nope", &mut c).unwrap_err();
        assert!(err.message.contains("`#nope`"));
        // tagged literals come later — error is informative.
    }

    #[test]
    fn cons_cells_use_list_proto() {
        // when a list-proto is provided, cons cells inherit it.
        let mut heap = Heap::new();
        let mut syms = SymTable::new();
        let list_proto = heap.alloc(Form::default());
        let mut c = ReadCtx::new(&mut heap, &mut syms, Value::Form(list_proto));
        let v = read("(1)", &mut c).unwrap();
        let id = v.as_form_id().unwrap();
        assert_eq!(c.heap.get(id).proto, Value::Form(list_proto));
    }

    #[test]
    fn list_iteration_is_in_order() {
        // determinism: read produces canonical, in-order cons cells.
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("(1 2 3 4 5)", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        assert_eq!(
            elems,
            vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
            ]
        );
    }

    #[test]
    fn underscores_alone_dont_parse_as_number() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        // `_foo` is a valid symbol name; not a number.
        let v = read("_foo", &mut c).unwrap();
        assert!(v.as_sym().is_some());
    }

    #[test]
    fn malformed_numbers_become_symbols() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        // `12abc` is not a valid number; phase-A treats as symbol.
        // (deliberate: lets users define `12abc` as a name.
        // parser.moof's later reader will reject more strictly.)
        let v = read("12abc", &mut c).unwrap();
        assert!(v.as_sym().is_some());
    }

    #[test]
    fn negative_numbers_versus_minus_symbol() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        // `-` alone is the subtraction symbol, not a number.
        let v = read("-", &mut c).unwrap();
        assert!(v.as_sym().is_some());
        assert_eq!(c.syms.resolve(v.as_sym().unwrap()), "-");

        // `-5` is a number.
        let v = read("-5", &mut c).unwrap();
        assert_eq!(v, Value::Int(-5));
    }

    #[test]
    fn whitespace_is_flexible() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("  (\n  foo\n  bar\n  )  ", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        assert_eq!(elems.len(), 2);
    }

    #[test]
    fn deeply_nested_lists() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let v = read("(((a)))", &mut c).unwrap();
        let level1 = list_to_vec(v, &c).unwrap();
        assert_eq!(level1.len(), 1);
        let level2 = list_to_vec(level1[0], &c).unwrap();
        assert_eq!(level2.len(), 1);
        let level3 = list_to_vec(level2[0], &c).unwrap();
        assert_eq!(level3.len(), 1);
        assert_eq!(c.syms.resolve(level3[0].as_sym().unwrap()), "a");
    }

    #[test]
    fn trailing_garbage_is_an_error() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let err = read("42 garbage", &mut c).unwrap_err();
        assert!(err.message.contains("trailing"));
    }

    #[test]
    fn empty_input_is_an_error() {
        let (mut heap, mut syms) = fresh();
        let mut c = ctx(&mut heap, &mut syms);
        let err = read("   ; just a comment\n", &mut c).unwrap_err();
        assert!(err.message.contains("end of input"));
    }
}
