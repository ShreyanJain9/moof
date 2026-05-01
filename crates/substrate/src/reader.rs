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

use crate::foreign::{ForeignHandle, ForeignTable};
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
    quasiquote_sym: SymId,
    unquote_sym: SymId,
    unquote_splicing_sym: SymId,
    send_sym: SymId,
    self_sym: SymId,
    bytes_sym: SymId,
    table_marker_sym: SymId,
    entry_marker_sym: SymId,
    obj_marker_sym: SymId,
    obj_slot_sym: SymId,
    obj_method_sym: SymId,
    /// the proto FormId to assign to every cons cell. typically
    /// `Value::Form(list_proto)`. phase-A clients that don't yet
    /// have a List proto pass `Value::Nil`.
    pub list_proto: Value,
    /// the proto FormId to assign to every String literal. when
    /// `Nil` (e.g., bare reader unit tests), `"…"` falls back to
    /// interning as a Sym.
    pub string_proto: Value,
    pub heap: &'a mut Heap,
    pub syms: &'a mut SymTable,
    pub foreign: &'a mut ForeignTable,
}

impl<'a> ReadCtx<'a> {
    pub fn new(
        heap: &'a mut Heap,
        syms: &'a mut SymTable,
        foreign: &'a mut ForeignTable,
        list_proto: Value,
        string_proto: Value,
    ) -> Self {
        let head_sym = syms.intern("head");
        let tail_sym = syms.intern("tail");
        let quote_sym = syms.intern("quote");
        let quasiquote_sym = syms.intern("quasiquote");
        let unquote_sym = syms.intern("unquote");
        let unquote_splicing_sym = syms.intern("unquote-splicing");
        let send_sym = syms.intern("__send__");
        let self_sym = syms.intern("self");
        let bytes_sym = syms.intern("bytes");
        let table_marker_sym = syms.intern("__table__");
        let entry_marker_sym = syms.intern("__entry__");
        let obj_marker_sym = syms.intern("__obj__");
        let obj_slot_sym = syms.intern("__slot__");
        let obj_method_sym = syms.intern("__method__");
        ReadCtx {
            head_sym,
            tail_sym,
            quote_sym,
            quasiquote_sym,
            unquote_sym,
            unquote_splicing_sym,
            send_sym,
            self_sym,
            bytes_sym,
            table_marker_sym,
            entry_marker_sym,
            obj_marker_sym,
            obj_slot_sym,
            obj_method_sym,
            list_proto,
            string_proto,
            heap,
            syms,
            foreign,
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
    matches!(
        c,
        b'(' | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'\''
            | b'"'
            | b';'
            | b'`'
            | b','
    ) || c.is_ascii_whitespace()
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

/// skip whitespace and comments.
///
/// per `docs/syntax/literals.md`:
/// - `;;` line comment
/// - `;:` doc comment (treated as line comment for now)
/// - `;~` scratch / fixme annotation
///
/// a bare `;` is *not* a comment — it's reserved as a syntactic
/// separator (cascade in `[…]`, etc). so we only consume `;` when
/// followed by `;`, `:`, or `~`. otherwise the `;` is left for
/// the caller.
fn skip_trivia(c: &mut Cursor) {
    loop {
        match c.peek() {
            Some(b) if b.is_ascii_whitespace() => {
                c.advance();
            }
            Some(b';') => {
                // is this a comment? requires `;` followed by `;`,
                // `:`, or `~`.
                let next = c.bytes.get(c.pos + 1).copied();
                if !matches!(next, Some(b';') | Some(b':') | Some(b'~')) {
                    return;
                }
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
        Some(b'{') => read_object_literal(c, ctx),
        Some(b'}') => Err(ReadError::at(c, "unexpected `}`")),
        Some(b'\'') => read_quote(c, ctx),
        Some(b'`') => read_quasiquote(c, ctx),
        Some(b',') => read_unquote(c, ctx),
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

/// `` `expr `` ⇒ `(quasiquote expr)`.
fn read_quasiquote(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'`'));
    c.advance();
    let inner = read_form(c, ctx)?;
    let qq_sym_v = Value::Sym(ctx.quasiquote_sym);
    Ok(build_list(ctx, &[qq_sym_v, inner]))
}

/// `,expr` ⇒ `(unquote expr)`. `,@expr` ⇒ `(unquote-splicing expr)`.
fn read_unquote(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b','));
    c.advance();
    let splicing = matches!(c.peek(), Some(b'@'));
    if splicing {
        c.advance();
    }
    let inner = read_form(c, ctx)?;
    let head_sym_v = Value::Sym(if splicing {
        ctx.unquote_splicing_sym
    } else {
        ctx.unquote_sym
    });
    Ok(build_list(ctx, &[head_sym_v, inner]))
}

/// `[recv sel args…]` — smalltalk-flavored send, optionally with
/// cascades.
///
/// shapes (`docs/syntax/brackets.md`):
/// - `[recv]` — error: too few elements.
/// - `[recv selector]` — unary send.
/// - `[recv OP arg]` — binary send (OP is operator-chars only).
/// - `[recv selector arg arg …]` — positional send.
/// - `[recv kw1: arg1 kw2: arg2 …]` — multi-keyword send.
/// - `[recv a; b; c: x]` — cascade: send `:a`, `:b`, `:c: x` to
///   `recv` in order. the cascade returns `recv` itself.
///
/// for non-cascade sends, emits `(__send__ recv 'selector arg…)`.
/// for cascades, emits `(__cascade__ recv (sel args…) …)`.
fn read_send_bracket(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'['));
    let start_line = c.line;
    let start_col = c.col;
    c.advance();

    // first element is the receiver.
    skip_trivia(c);
    if c.peek() == Some(b']') {
        return Err(ReadError {
            message: "empty send bracket `[]`".into(),
            line: start_line,
            col: start_col,
        });
    }
    let receiver = read_form(c, ctx)?;

    // collect segments (selector + args), separated by `;`. one
    // segment = the rest of a normal send; multiple = cascade.
    let mut segments: Vec<(SymId, Vec<Value>)> = Vec::new();
    loop {
        let mut elems: Vec<Value> = Vec::new();
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
                Some(b']') => break,
                Some(b';') => {
                    c.advance();
                    break;
                }
                _ => {
                    elems.push(read_form(c, ctx)?);
                }
            }
        }
        if elems.is_empty() {
            if segments.is_empty() {
                return Err(ReadError {
                    message: "send needs at least a selector".into(),
                    line: start_line,
                    col: start_col,
                });
            } else {
                return Err(ReadError {
                    message: "empty cascade segment after `;`".into(),
                    line: start_line,
                    col: start_col,
                });
            }
        }
        let (sel, args) = decode_send_segment(ctx, &elems, start_line, start_col)?;
        segments.push((sel, args));

        // closing `]` ends the bracket.
        if c.peek() == Some(b']') {
            c.advance();
            break;
        }
        // else we just consumed a `;`; loop reads the next segment.
    }

    if segments.len() == 1 {
        let (sel, args) = segments.pop().unwrap();
        Ok(emit_send(ctx, receiver, sel, &args))
    } else {
        Ok(emit_cascade(ctx, receiver, &segments))
    }
}

/// decode a flat element list into a (selector, args) pair using
/// the same shape rules as send-brackets.
fn decode_send_segment(
    ctx: &mut ReadCtx,
    elements: &[Value],
    line: usize,
    col: usize,
) -> Result<(SymId, Vec<Value>), ReadError> {
    let first = elements[0];
    let first_sym = first.as_sym().ok_or_else(|| ReadError {
        message: "selector must be a symbol".into(),
        line,
        col,
    })?;
    let first_text = ctx.syms.resolve(first_sym).to_string();

    // binary: exactly 2 elements, first is operator-only.
    if is_binary_op(&first_text) && elements.len() == 2 {
        return Ok((first_sym, vec![elements[1]]));
    }

    // keyword: first ends in `:`.
    if first_text.ends_with(':') {
        let mut sel_text = String::new();
        let mut args: Vec<Value> = Vec::new();
        let mut i = 0;
        while i < elements.len() {
            let kw = elements[i].as_sym().ok_or_else(|| ReadError {
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
            if i >= elements.len() {
                return Err(ReadError {
                    message: format!("keyword `{}` needs an argument", kw_text),
                    line,
                    col,
                });
            }
            args.push(elements[i]);
            i += 1;
        }
        let sel = ctx.syms.intern(&sel_text);
        return Ok((sel, args));
    }

    // unary or positional.
    Ok((first_sym, elements[1..].to_vec()))
}

/// emit a cascade marker: `(__cascade__ recv (sel arg…) …)`.
fn emit_cascade(
    ctx: &mut ReadCtx,
    receiver: Value,
    segments: &[(SymId, Vec<Value>)],
) -> Value {
    let cascade_sym = ctx.syms.intern("__cascade__");
    let mut parts: Vec<Value> = Vec::with_capacity(2 + segments.len());
    parts.push(Value::Sym(cascade_sym));
    parts.push(receiver);
    for (sel, args) in segments {
        let mut seg_parts = Vec::with_capacity(args.len() + 1);
        seg_parts.push(Value::Sym(*sel));
        seg_parts.extend_from_slice(args);
        let seg = build_list(ctx, &seg_parts);
        parts.push(seg);
    }
    build_list(ctx, &parts)
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

/// read `#…` — hash-prefixed forms. supported:
///   `#true` `#false`           — boolean literals
///   `#\…`                      — char literal
///   `#[ … ]`                   — Table literal
///
/// tagged literals (`#Date "..."`, `#Url "..."`, etc.) come later.
fn read_hash(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'#'));
    let start_line = c.line;
    let start_col = c.col;
    c.advance();
    // char literal? `#\<rest>`
    if c.peek() == Some(b'\\') {
        c.advance();
        return read_char_literal(c, start_line, start_col);
    }
    // table literal? `#[ … ]`
    if c.peek() == Some(b'[') {
        return read_table_literal(c, ctx, start_line, start_col);
    }
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
                "unknown hash form `#{}` (supports `#true`, `#false`, `#\\…`, `#[…]`)",
                other
            ),
            line: start_line,
            col: start_col,
        }),
    }
}

/// read `#[ a b 'name => "ada" c ]` — Table literal.
///
/// emits a `(__table__ <entry…>)` form. each entry is either:
/// - a bare expression form (positional)
/// - `(__entry__ <key-expr> <value-expr>)` (keyed)
///
/// `=>` between two expressions promotes the previous element from
/// positional to keyed. expressions are evaluated by the compiler;
/// the reader produces literal cons cells.
fn read_table_literal(
    c: &mut Cursor,
    ctx: &mut ReadCtx,
    start_line: usize,
    start_col: usize,
) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'['));
    c.advance();
    let mut entries: Vec<Value> = Vec::new();
    loop {
        skip_trivia(c);
        match c.peek() {
            None => {
                return Err(ReadError {
                    message: "unterminated `#[…]` table literal".into(),
                    line: start_line,
                    col: start_col,
                });
            }
            Some(b']') => {
                c.advance();
                let mut form_elems = vec![Value::Sym(ctx.table_marker_sym)];
                form_elems.extend(entries);
                return Ok(build_list(ctx, &form_elems));
            }
            _ => {
                let elem = read_form(c, ctx)?;
                // peek for `=>` to upgrade to keyed entry.
                skip_trivia(c);
                if peek_arrow(c) {
                    consume_arrow(c);
                    skip_trivia(c);
                    if c.peek() == Some(b']') || c.peek().is_none() {
                        return Err(ReadError {
                            message: "`=>` expects a value after it".into(),
                            line: start_line,
                            col: start_col,
                        });
                    }
                    let val = read_form(c, ctx)?;
                    let entry = build_list(
                        ctx,
                        &[Value::Sym(ctx.entry_marker_sym), elem, val],
                    );
                    entries.push(entry);
                } else {
                    entries.push(elem);
                }
            }
        }
    }
}

/// `{ proto-ref? ( name : value | [ method-header ] body )* }` —
/// object literal per `docs/syntax/object-literals.md`.
///
/// emits `(__obj__ <proto-sym> <entry…>)` where each entry is one of:
/// - `(__slot__ <key-sym> <value-expr>)`
/// - `(__method__ <selector-sym> <params-list> <body-expr>)`
///
/// proto defaults to `Object` if omitted (i.e., the first non-`[`
/// item is a slot binding `name:`).
///
/// inside `{…}`, `[…]` is a *method header*, not a send. the
/// header tokens are parsed and translated to a (selector, params)
/// pair using the same shape rules as send-brackets:
/// - `[name]`           — unary
/// - `[+ other]`        — binary
/// - `[name x y …]`     — positional
/// - `[at: i put: v]`   — keyword
fn read_object_literal(c: &mut Cursor, ctx: &mut ReadCtx) -> Result<Value, ReadError> {
    debug_assert_eq!(c.peek(), Some(b'{'));
    let start_line = c.line;
    let start_col = c.col;
    c.advance();

    let object_sym = ctx.syms.intern("Object");
    let mut proto = Value::Sym(object_sym);
    let mut entries: Vec<Value> = Vec::new();
    let mut has_proto = false;

    loop {
        skip_trivia(c);
        match c.peek() {
            None => {
                return Err(ReadError {
                    message: "unterminated `{…}` object literal".into(),
                    line: start_line,
                    col: start_col,
                });
            }
            Some(b'}') => {
                c.advance();
                let mut form_elems = vec![Value::Sym(ctx.obj_marker_sym), proto];
                form_elems.extend(entries);
                return Ok(build_list(ctx, &form_elems));
            }
            Some(b'[') => {
                // method definition.
                c.advance();
                let header_tokens = read_method_header_tokens(c, ctx)?;
                skip_trivia(c);
                let body = read_form(c, ctx)?;
                let (sel, params) = decode_method_header(
                    ctx,
                    &header_tokens,
                    start_line,
                    start_col,
                )?;
                let params_list = build_list(ctx, &params);
                let entry = build_list(
                    ctx,
                    &[
                        Value::Sym(ctx.obj_method_sym),
                        Value::Sym(sel),
                        params_list,
                        body,
                    ],
                );
                entries.push(entry);
            }
            _ => {
                let form = read_form(c, ctx)?;
                if let Value::Sym(s) = form {
                    let text = ctx.syms.resolve(s).to_string();
                    if text.ends_with(':') {
                        // slot binding: `name: value`. strip the
                        // trailing colon for the slot's name.
                        let key_text = &text[..text.len() - 1];
                        let key_sym = ctx.syms.intern(key_text);
                        skip_trivia(c);
                        let value = read_form(c, ctx)?;
                        let entry = build_list(
                            ctx,
                            &[Value::Sym(ctx.obj_slot_sym), Value::Sym(key_sym), value],
                        );
                        entries.push(entry);
                        continue;
                    }
                    if !has_proto && entries.is_empty() {
                        // first bare symbol — proto.
                        proto = Value::Sym(s);
                        has_proto = true;
                        continue;
                    }
                }
                return Err(ReadError {
                    message: "object literal: expected `name:` slot binding, `[…]` method, or proto symbol"
                        .into(),
                    line: start_line,
                    col: start_col,
                });
            }
        }
    }
}

/// read tokens up to (and consuming) `]`, returning the tokens.
/// used inside object-literal method headers where `[…]` is a
/// header, not a send.
fn read_method_header_tokens(
    c: &mut Cursor,
    ctx: &mut ReadCtx,
) -> Result<Vec<Value>, ReadError> {
    let mut tokens = Vec::new();
    loop {
        skip_trivia(c);
        match c.peek() {
            None => {
                return Err(ReadError::at(c, "unterminated method header"));
            }
            Some(b']') => {
                c.advance();
                return Ok(tokens);
            }
            _ => {
                tokens.push(read_form(c, ctx)?);
            }
        }
    }
}

/// translate a method-header token sequence into a (selector,
/// params-list) pair. mirrors the four send-shapes:
/// - unary:    `[name]` → selector=`name`, params=[]
/// - binary:   `[OP other]` → selector=OP, params=[other]
/// - keyword:  `[kw1: a kw2: b]` → selector="kw1:kw2:", params=[a, b]
/// - positional: `[name x y]` → selector="name", params=[x, y]
fn decode_method_header(
    ctx: &mut ReadCtx,
    tokens: &[Value],
    line: usize,
    col: usize,
) -> Result<(SymId, Vec<Value>), ReadError> {
    if tokens.is_empty() {
        return Err(ReadError {
            message: "empty method header".into(),
            line,
            col,
        });
    }
    let first = tokens[0];
    let first_sym = first.as_sym().ok_or_else(|| ReadError {
        message: "method header: selector must be a symbol".into(),
        line,
        col,
    })?;
    let first_text = ctx.syms.resolve(first_sym).to_string();

    // binary
    if is_binary_op(&first_text) && tokens.len() == 2 {
        return Ok((first_sym, vec![tokens[1]]));
    }

    // keyword
    if first_text.ends_with(':') {
        let mut sel_text = String::new();
        let mut params = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            let kw_sym = tokens[i].as_sym().ok_or_else(|| ReadError {
                message: "keyword method header: expected `kw:` symbol".into(),
                line,
                col,
            })?;
            let kw_text = ctx.syms.resolve(kw_sym).to_string();
            if !kw_text.ends_with(':') {
                return Err(ReadError {
                    message: format!("keyword `{}` must end with `:`", kw_text),
                    line,
                    col,
                });
            }
            sel_text.push_str(&kw_text);
            i += 1;
            if i >= tokens.len() {
                return Err(ReadError {
                    message: format!("keyword `{}` needs a parameter", kw_text),
                    line,
                    col,
                });
            }
            params.push(tokens[i]);
            i += 1;
        }
        let sel = ctx.syms.intern(&sel_text);
        return Ok((sel, params));
    }

    // unary or positional
    Ok((first_sym, tokens[1..].to_vec()))
}

/// peek for `=>` followed by whitespace / delim. doesn't consume.
fn peek_arrow(c: &Cursor) -> bool {
    let pos = c.pos;
    if pos + 2 > c.bytes.len() {
        return false;
    }
    if c.bytes[pos] != b'=' || c.bytes[pos + 1] != b'>' {
        return false;
    }
    if pos + 2 == c.bytes.len() {
        return true;
    }
    is_delim(c.bytes[pos + 2])
}

/// consume a 2-byte `=>`.
fn consume_arrow(c: &mut Cursor) {
    debug_assert_eq!(c.peek(), Some(b'='));
    c.advance();
    debug_assert_eq!(c.peek(), Some(b'>'));
    c.advance();
}

/// read a char literal starting *after* the `#\` prefix.
///
/// supported shapes:
/// - `#\h`             — single ASCII char
/// - `#\space`         — named char (space, newline, tab, return)
/// - `#\u{1f496}`      — hex codepoint
fn read_char_literal(
    c: &mut Cursor,
    start_line: usize,
    start_col: usize,
) -> Result<Value, ReadError> {
    // unicode escape: #\u{HEX}
    if c.peek() == Some(b'u') {
        // peek further to confirm `u{...}` (otherwise might be the
        // bareword `#\u`-as-char, but that's ambiguous). we require
        // the `{` to disambiguate.
        if c.bytes.get(c.pos + 1).copied() == Some(b'{') {
            c.advance(); // u
            c.advance(); // {
            let mut hex = String::new();
            loop {
                match c.peek() {
                    Some(b'}') => {
                        c.advance();
                        break;
                    }
                    Some(b) if (b as char).is_ascii_hexdigit() => {
                        hex.push(b as char);
                        c.advance();
                    }
                    _ => {
                        return Err(ReadError {
                            message: format!("malformed `#\\u{{{}…}}` char literal", hex),
                            line: start_line,
                            col: start_col,
                        });
                    }
                }
            }
            let cp = u32::from_str_radix(&hex, 16).map_err(|_| ReadError {
                message: format!("invalid hex codepoint `#\\u{{{}}}`", hex),
                line: start_line,
                col: start_col,
            })?;
            // reject surrogates and out-of-range.
            if char::from_u32(cp).is_none() {
                return Err(ReadError {
                    message: format!("`#\\u{{{}}}` is not a Unicode scalar value", hex),
                    line: start_line,
                    col: start_col,
                });
            }
            return Ok(Value::Char(cp));
        }
    }
    // single char or named.
    let first = match c.advance() {
        Some(b) => b,
        None => {
            return Err(ReadError {
                message: "unterminated `#\\` char literal".into(),
                line: start_line,
                col: start_col,
            });
        }
    };
    // try to read more if the next char is alphabetic — for named
    // char literals like #\space.
    let mut buf = String::new();
    buf.push(first as char);
    while let Some(b) = c.peek() {
        if is_delim(b) {
            break;
        }
        buf.push(b as char);
        c.advance();
    }
    if buf.len() == 1 {
        return Ok(Value::Char(first as u32));
    }
    // named char
    let cp = match buf.as_str() {
        "space" => Some(b' ' as u32),
        "newline" => Some(b'\n' as u32),
        "tab" => Some(b'\t' as u32),
        "return" => Some(b'\r' as u32),
        "null" => Some(0u32),
        "backspace" => Some(0x08),
        "delete" => Some(0x7f),
        "escape" => Some(0x1b),
        _ => None,
    };
    match cp {
        Some(c) => Ok(Value::Char(c)),
        None => Err(ReadError {
            message: format!("unknown char name `#\\{}`", buf),
            line: start_line,
            col: start_col,
        }),
    }
}

/// read a `"…"` string with `\n \t \\ \"` escapes.
///
/// produces a `String` Form (proto = ctx.string_proto) whose
/// `:bytes` slot is a ForeignHandle wrapping the UTF-8 bytes. for
/// reader unit tests that don't have a String proto wired (legacy
/// phase-A behavior), `string_proto = Nil` falls back to interning
/// as a Sym so older tests still pass.
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
                return Ok(make_string_value(ctx, &s));
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
            Some(b) if b < 0x80 => {
                // ASCII fast path.
                s.push(b as char);
                c.advance();
            }
            Some(_) => {
                // multi-byte UTF-8 sequence — decode the next
                // codepoint and advance by its byte length.
                let remaining = &c.bytes[c.pos..];
                let decoded = std::str::from_utf8(remaining)
                    .map_err(|_| ReadError::at(c, "invalid UTF-8 in string"))?;
                let ch = decoded
                    .chars()
                    .next()
                    .ok_or_else(|| ReadError::at(c, "unexpected end of string"))?;
                let len = ch.len_utf8();
                s.push(ch);
                for _ in 0..len {
                    c.advance();
                }
            }
        }
    }
}

/// allocate a String form (or fall back to Sym if no String proto
/// is provided — used by bare reader unit tests).
fn make_string_value(ctx: &mut ReadCtx, text: &str) -> Value {
    if ctx.string_proto.is_nil() {
        // legacy fallback for bare reader tests.
        return Value::Sym(ctx.syms.intern(text));
    }
    // tag is the canonical substrate constant from foreign.rs —
    // both world.rs's `make_string` and this path mint string
    // forms with the same tag, ensuring the heap looks identical
    // regardless of which gateway built it.
    let boxed: Box<Vec<u8>> = Box::new(text.as_bytes().to_vec());
    let ptr = Box::into_raw(boxed) as *mut std::ffi::c_void;
    unsafe extern "C" fn dtor(ptr: *mut std::ffi::c_void) {
        if !ptr.is_null() {
            let _ = unsafe { Box::from_raw(ptr as *mut Vec<u8>) };
        }
    }
    let handle_id = ctx.foreign.alloc(ForeignHandle {
        ptr,
        destructor: Some(dtor),
        tag: crate::foreign::TAG_STRING_BYTES,
    });
    let mut form = Form::with_proto(ctx.string_proto);
    form.slots.insert(ctx.bytes_sym, Value::Foreign(handle_id));
    Value::Form(ctx.heap.alloc(form))
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
    // numeric first (so `.5`, `1.5`, `1e9` parse as Float, not as
    // `.foo` shorthand or symbol).
    if let Some(v) = try_parse_number(&text) {
        return Ok(v);
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
    Ok(Value::Sym(ctx.syms.intern(&text)))
}

/// try to parse `text` as a moof numeric literal — Integer or
/// Float. returns `None` if it doesn't look like a number, in
/// which case the caller treats it as a symbol.
///
/// shapes (`docs/syntax/literals.md`):
/// - decimal Integer: `42`, `-42`, `1_000_000`
/// - hex/bin/oct Integer: `0xff`, `0b1010`, `0o17`
/// - Float: `1.5`, `.5`, `1.`, `1e9`, `1.5e-3`, `-3.14`
///   (always parsed as f64 when `.` or `e/E` appears outside a
///   base prefix).
fn try_parse_number(text: &str) -> Option<Value> {
    // strip underscores from anywhere; they're for readability only.
    let cleaned: String = text.chars().filter(|&c| c != '_').collect();
    let cleaned = cleaned.as_str();
    if cleaned.is_empty() {
        return None;
    }

    // sign-aware base prefix detection.
    let (sign_int, sign_f, rest): (i64, f64, &str) = match cleaned.as_bytes()[0] {
        b'-' => (-1, -1.0, &cleaned[1..]),
        b'+' => (1, 1.0, &cleaned[1..]),
        _ => (1, 1.0, cleaned),
    };
    if rest.is_empty() {
        return None;
    }

    // base prefixes are integer-only.
    if let Some(stripped) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        return i64::from_str_radix(stripped, 16)
            .ok()
            .map(|n| Value::Int(sign_int * n));
    }
    if let Some(stripped) = rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")) {
        return i64::from_str_radix(stripped, 2)
            .ok()
            .map(|n| Value::Int(sign_int * n));
    }
    if let Some(stripped) = rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")) {
        return i64::from_str_radix(stripped, 8)
            .ok()
            .map(|n| Value::Int(sign_int * n));
    }

    // first char is a digit OR a `.` (for `.5`).
    let first = rest.bytes().next()?;
    if !first.is_ascii_digit() && first != b'.' {
        return None;
    }

    // does it look like a float? (contains `.` or `e`/`E`)
    let is_float = rest.bytes().any(|b| b == b'.' || b == b'e' || b == b'E');
    if is_float {
        // rust's f64::from_str doesn't accept a leading `.5`-style
        // — wait, it actually does. let's just try.
        return rest.parse::<f64>().ok().map(|f| Value::float(sign_f * f));
    }

    rest.parse::<i64>().ok().map(|n| Value::Int(sign_int * n))
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

    fn fresh() -> (Heap, SymTable, ForeignTable) {
        (Heap::new(), SymTable::new(), ForeignTable::new())
    }

    fn ctx<'a>(
        heap: &'a mut Heap,
        syms: &'a mut SymTable,
        foreign: &'a mut ForeignTable,
    ) -> ReadCtx<'a> {
        ReadCtx::new(heap, syms, foreign, Value::Nil, Value::Nil)
    }

    #[test]
    fn nil_literal() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        assert_eq!(read("nil", &mut c).unwrap(), Value::Nil);
    }

    #[test]
    fn boolean_literals() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        assert_eq!(read("#true", &mut c).unwrap(), Value::Bool(true));
        assert_eq!(read("#false", &mut c).unwrap(), Value::Bool(false));
    }

    #[test]
    fn integer_literals_decimal() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        assert_eq!(read("0", &mut c).unwrap(), Value::Int(0));
        assert_eq!(read("42", &mut c).unwrap(), Value::Int(42));
        assert_eq!(read("-7", &mut c).unwrap(), Value::Int(-7));
        assert_eq!(read("+7", &mut c).unwrap(), Value::Int(7));
    }

    #[test]
    fn integer_literals_bases() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        assert_eq!(read("0xff", &mut c).unwrap(), Value::Int(255));
        assert_eq!(read("0xFF", &mut c).unwrap(), Value::Int(255));
        assert_eq!(read("0b1010", &mut c).unwrap(), Value::Int(10));
        assert_eq!(read("0o17", &mut c).unwrap(), Value::Int(15));
        assert_eq!(read("-0xff", &mut c).unwrap(), Value::Int(-255));
    }

    #[test]
    fn integer_literals_underscore_grouping() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        assert_eq!(read("1_000_000", &mut c).unwrap(), Value::Int(1_000_000));
        assert_eq!(read("0xDEAD_BEEF", &mut c).unwrap(), Value::Int(0xDEAD_BEEF));
        assert_eq!(read("0b1100_0011", &mut c).unwrap(), Value::Int(0b1100_0011));
    }

    #[test]
    fn symbols() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
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
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        assert_eq!(read("()", &mut c).unwrap(), Value::Nil);
    }

    #[test]
    fn list_three_ints() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let v = read("(1 2 3)", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        assert_eq!(elems, vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
    }

    #[test]
    fn list_nested() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
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
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let v = read("'foo", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        // ('quote foo)
        assert_eq!(elems.len(), 2);
        assert_eq!(c.syms.resolve(elems[0].as_sym().unwrap()), "quote");
        assert_eq!(c.syms.resolve(elems[1].as_sym().unwrap()), "foo");
    }

    #[test]
    fn quote_of_list() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
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
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let v = read("\"hello\"", &mut c).unwrap();
        let s = v.as_sym().unwrap();
        assert_eq!(c.syms.resolve(s), "hello");
    }

    #[test]
    fn strings_with_escapes() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let v = read("\"a\\nb\\tc\\\"d\\\\e\"", &mut c).unwrap();
        let s = v.as_sym().unwrap();
        assert_eq!(c.syms.resolve(s), "a\nb\tc\"d\\e");
    }

    #[test]
    fn comments_are_ignored() {
        // `;;` is the line-comment marker now (bare `;` is reserved
        // for cascade separators inside `[…]`). docs/syntax/literals.md.
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let v = read(
            ";; a comment\n  42  ;; trailing comment\n",
            &mut c,
        )
        .unwrap();
        assert_eq!(v, Value::Int(42));
    }

    #[test]
    fn read_all_top_level_forms() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let forms = read_all("1 2 3", &mut c).unwrap();
        assert_eq!(
            forms,
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    #[test]
    fn errors_have_position() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let err = read("(foo bar", &mut c).unwrap_err();
        assert_eq!(err.message, "unterminated list");
        // unterminated list is reported at end-of-input.

        let err = read("(foo )) extra", &mut c).unwrap_err();
        assert!(err.message.contains("unexpected"));
    }

    #[test]
    fn errors_unterminated_string() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let err = read("\"hello", &mut c).unwrap_err();
        assert_eq!(err.message, "unterminated string");
    }

    #[test]
    fn errors_unknown_hash_form() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let err = read("#nope", &mut c).unwrap_err();
        assert!(err.message.contains("`#nope`"));
        // tagged literals come later — error is informative.
    }

    #[test]
    fn cons_cells_use_list_proto() {
        // when a list-proto is provided, cons cells inherit it.
        let mut heap = Heap::new();
        let mut syms = SymTable::new();
        let mut foreign = ForeignTable::new();
        let list_proto = heap.alloc(Form::default());
        let mut c = ReadCtx::new(
            &mut heap,
            &mut syms,
            &mut foreign,
            Value::Form(list_proto),
            Value::Nil,
        );
        let v = read("(1)", &mut c).unwrap();
        let id = v.as_form_id().unwrap();
        assert_eq!(c.heap.get(id).proto, Value::Form(list_proto));
    }

    #[test]
    fn list_iteration_is_in_order() {
        // determinism: read produces canonical, in-order cons cells.
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
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
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        // `_foo` is a valid symbol name; not a number.
        let v = read("_foo", &mut c).unwrap();
        assert!(v.as_sym().is_some());
    }

    #[test]
    fn malformed_numbers_become_symbols() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        // `12abc` is not a valid number; phase-A treats as symbol.
        // (deliberate: lets users define `12abc` as a name.
        // parser.moof's later reader will reject more strictly.)
        let v = read("12abc", &mut c).unwrap();
        assert!(v.as_sym().is_some());
    }

    #[test]
    fn negative_numbers_versus_minus_symbol() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
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
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let v = read("  (\n  foo\n  bar\n  )  ", &mut c).unwrap();
        let elems = list_to_vec(v, &c).unwrap();
        assert_eq!(elems.len(), 2);
    }

    #[test]
    fn deeply_nested_lists() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
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
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let err = read("42 garbage", &mut c).unwrap_err();
        assert!(err.message.contains("trailing"));
    }

    #[test]
    fn empty_input_is_an_error() {
        let (mut heap, mut syms, mut foreign) = fresh();
        let mut c = ctx(&mut heap, &mut syms, &mut foreign);
        let err = read("   ;; just a comment\n", &mut c).unwrap_err();
        assert!(err.message.contains("end of input"));
    }
}
