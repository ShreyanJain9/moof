//! reader — phase 1 minimum s-expression parser.
//!
//! parses one expression from text, allocating Forms via the World's
//! heap. List literals are chains of cons-cell Forms (proto: List;
//! head: car; args: rest-as-Form-or-Nil).
//!
//! the *real* moof parser is moof code (phase 2). this rust reader
//! is the bootstrap parser only — small enough to load the real one.

use crate::form::Form;
use crate::value::Value;
use crate::world::World;

pub fn read(world: &mut World, input: &str) -> Result<Value, String> {
    let mut p = Parser::new(input, world);
    p.skip_trivia();
    let v = p.read_expr()?;
    p.skip_trivia();
    if p.pos < p.bytes.len() {
        return Err(format!(
            "unexpected trailing input at byte {}: {:?}",
            p.pos,
            p.peek_char()
        ));
    }
    Ok(v)
}

/// read every top-level expression in `input`. returns them in order.
pub fn read_all(world: &mut World, input: &str) -> Result<Vec<Value>, String> {
    let mut p = Parser::new(input, world);
    let mut out = Vec::new();
    loop {
        p.skip_trivia();
        if p.pos >= p.bytes.len() {
            return Ok(out);
        }
        out.push(p.read_expr()?);
    }
}

struct Parser<'a, 'w> {
    bytes: &'a [u8],
    pos: usize,
    world: &'w mut World,
}

impl<'a, 'w> Parser<'a, 'w> {
    fn new(input: &'a str, world: &'w mut World) -> Self {
        Parser {
            bytes: input.as_bytes(),
            pos: 0,
            world,
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.bytes.get(self.pos).map(|b| *b as char)
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek_char() {
                Some(c) if c.is_whitespace() => self.advance(),
                Some(';') => {
                    while let Some(c) = self.peek_char() {
                        self.advance();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn read_expr(&mut self) -> Result<Value, String> {
        self.skip_trivia();
        match self.peek_char() {
            None => Err("unexpected end of input".into()),
            Some('(') => self.read_list(),
            Some(')') => Err("unexpected `)`".into()),
            Some('\'') => {
                self.advance();
                let inner = self.read_expr()?;
                // `'x` desugars to `(quote x)`.
                let quote_sym = self.world.syms.intern("quote");
                let cdr = self.alloc_cons(inner, Value::Nil);
                Ok(self.alloc_cons(Value::Sym(quote_sym), cdr))
            }
            Some('#') => self.read_hash_form(),
            Some('"') => self.read_string(),
            Some('[') => self.read_send_form(),
            Some(']') => Err("unexpected `]`".into()),
            Some(c) if is_atom_start(c) => self.read_atom(),
            Some(c) => Err(format!("unexpected character {:?}", c)),
        }
    }

    /// `[recv selector args...]` — message send.
    /// supports three reading patterns (concepts/sends-and-calls.md):
    ///
    /// - **unary / positional**: `[obj msg]`, `[obj method arg1 arg2]`.
    ///   selector is the second token; remaining tokens are args.
    /// - **multi-keyword**: `[dict at: 'name put: 5]`. tokens after
    ///   the receiver alternate marker/value; the marker symbols (each
    ///   ending in `:`) concatenate into the selector.
    /// - **binary** (a special case of positional): `[5 + 3]` — the
    ///   selector `+` is just a symbol; one positional arg follows.
    ///
    /// produces a Form with proto = `send_form_proto`. structure-face:
    /// `head` = receiver, `args` = (selector . positional-args).
    /// the compiler dispatches on the proto to emit a Send opcode.
    fn read_send_form(&mut self) -> Result<Value, String> {
        debug_assert_eq!(self.peek_char(), Some('['));
        self.advance(); // consume '['
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia();
            match self.peek_char() {
                None => return Err("unterminated send `[…]`".into()),
                Some(']') => {
                    self.advance();
                    break;
                }
                _ => tokens.push(self.read_expr()?),
            }
        }
        if tokens.len() < 2 {
            return Err("send `[…]` must have receiver + selector".into());
        }
        let recv = tokens[0];

        // determine if this is keyword-style (any token after the
        // receiver is a keyword marker — symbol ending in `:`).
        let is_keyword = matches!(tokens[1], Value::Sym(s)
                                if self.world.syms.name(s).ends_with(':'));

        let (selector, args) = if is_keyword {
            // alternating marker / value pairs.
            let mut selector_str = String::new();
            let mut args = Vec::new();
            let mut i = 1;
            while i < tokens.len() {
                let marker = match tokens[i] {
                    Value::Sym(s)
                        if self.world.syms.name(s).ends_with(':') =>
                    {
                        self.world.syms.name(s).to_string()
                    }
                    _ => {
                        return Err(format!(
                            "send `[…]`: expected keyword marker at position {i}"
                        ))
                    }
                };
                if i + 1 >= tokens.len() {
                    return Err(format!(
                        "send `[…]`: keyword `{marker}` has no value"
                    ));
                }
                selector_str.push_str(&marker);
                args.push(tokens[i + 1]);
                i += 2;
            }
            let sel = self.world.syms.intern(&selector_str);
            (Value::Sym(sel), args)
        } else {
            // unary / positional / binary. selector is tokens[1].
            let selector = tokens[1];
            if !matches!(selector, Value::Sym(_)) {
                return Err("send `[…]`: selector must be a symbol".into());
            }
            let args: Vec<Value> = tokens[2..].to_vec();
            (selector, args)
        };

        // build the inner list (selector . args)
        let mut inner_tail = Value::Nil;
        for v in args.iter().rev() {
            inner_tail = self.alloc_cons(*v, inner_tail);
        }
        let inner = self.alloc_cons(selector, inner_tail);

        // wrap as a send-form.
        let proto = self.world.send_form_proto;
        let id = self
            .world
            .heap
            .alloc(crate::form::Form::cons(proto, recv, inner));
        Ok(Value::Form(id))
    }

    fn read_string(&mut self) -> Result<Value, String> {
        debug_assert_eq!(self.peek_char(), Some('"'));
        self.advance(); // consume opening "
        let mut out = String::new();
        loop {
            match self.peek_char() {
                None => return Err("unterminated string literal".into()),
                Some('"') => {
                    self.advance();
                    let id = self.world.alloc_string(&out);
                    return Ok(Value::Form(id));
                }
                Some('\\') => {
                    self.advance();
                    match self.peek_char() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('r') => out.push('\r'),
                        Some('\\') => out.push('\\'),
                        Some('"') => out.push('"'),
                        Some('#') => out.push('#'),
                        Some(c) => return Err(format!("unknown string escape: \\{c}")),
                        None => return Err("string ended after backslash".into()),
                    }
                    self.advance();
                }
                Some(c) => {
                    out.push(c);
                    self.advance();
                }
            }
        }
    }

    /// `#`-prefixed forms. phase 2 supports the boolean literals
    /// `#true` and `#false`. later phases add `#[...]` (Tables),
    /// `#Tag (...)` (tagged literals), `#\h` (chars).
    fn read_hash_form(&mut self) -> Result<Value, String> {
        debug_assert_eq!(self.peek_char(), Some('#'));
        // capture the full token starting from `#` so we can dispatch.
        let start = self.pos;
        self.advance(); // consume '#'
        // peek next character and decide.
        match self.peek_char() {
            Some(c) if c.is_alphabetic() => {
                // read a tag name (alphanumeric / -)
                let tag_start = self.pos;
                while let Some(ch) = self.peek_char() {
                    if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
                let tag = std::str::from_utf8(&self.bytes[tag_start..self.pos])
                    .map_err(|e| format!("invalid utf8 in #form: {e}"))?;
                match tag {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    "nil" => Ok(Value::Nil),
                    other => Err(format!(
                        "unknown #-form: #{other} (phase 2 only knows #true, #false, #nil)"
                    )),
                }
            }
            _ => Err(format!(
                "unexpected character after `#` at byte {}: {:?}",
                start,
                self.peek_char()
            )),
        }
    }

    fn read_list(&mut self) -> Result<Value, String> {
        debug_assert_eq!(self.peek_char(), Some('('));
        self.advance();
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            match self.peek_char() {
                None => return Err("unterminated list".into()),
                Some(')') => {
                    self.advance();
                    let mut tail = Value::Nil;
                    for v in items.into_iter().rev() {
                        tail = self.alloc_cons(v, tail);
                    }
                    return Ok(tail);
                }
                Some(_) => {
                    items.push(self.read_expr()?);
                }
            }
        }
    }

    fn read_atom(&mut self) -> Result<Value, String> {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if is_atom_continue(c) {
                self.advance();
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|e| format!("invalid utf8 in atom: {e}"))?;

        if let Some(n) = parse_integer(text) {
            return Ok(Value::Int(n));
        }
        let id = self.world.syms.intern(text);
        Ok(Value::Sym(id))
    }

    /// allocate a cons-cell-shaped Form on the heap.
    fn alloc_cons(&mut self, car: Value, cdr: Value) -> Value {
        let proto = self.world.list_proto;
        let id = self.world.heap.alloc(Form::cons(proto, car, cdr));
        Value::Form(id)
    }
}

fn is_atom_start(c: char) -> bool {
    !c.is_whitespace()
        && c != '('
        && c != ')'
        && c != '['
        && c != ']'
        && c != '{'
        && c != '}'
        && c != '"'
        && c != '\''
        && c != ';'
        && c != '`'
        && c != ','
}

fn is_atom_continue(c: char) -> bool {
    is_atom_start(c)
}

fn parse_integer(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let (sign, rest) = match s.as_bytes()[0] {
        b'-' => (-1i64, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (1, s),
    };
    if rest.is_empty() {
        return None;
    }
    let mut acc: i64 = 0;
    for b in rest.bytes() {
        match b {
            b'0'..=b'9' => {
                acc = acc.checked_mul(10)?.checked_add((b - b'0') as i64)?;
            }
            b'_' => continue,
            _ => return None,
        }
    }
    Some(sign.checked_mul(acc)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_int() {
        let mut w = World::new();
        match read(&mut w, "42").unwrap() {
            Value::Int(42) => {}
            v => panic!("got {v:?}"),
        }
    }

    #[test]
    fn read_neg_int() {
        let mut w = World::new();
        match read(&mut w, "-7").unwrap() {
            Value::Int(-7) => {}
            v => panic!("got {v:?}"),
        }
    }

    #[test]
    fn read_sym() {
        let mut w = World::new();
        let plus = w.syms.intern("+");
        match read(&mut w, "+").unwrap() {
            Value::Sym(s) => assert_eq!(s, plus),
            v => panic!("got {v:?}"),
        }
    }

    #[test]
    fn read_nested_list() {
        let mut w = World::new();
        let v = read(&mut w, "(* 3 (+ 4 5))").unwrap();
        assert!(matches!(v, Value::Form(_)));
    }
}
