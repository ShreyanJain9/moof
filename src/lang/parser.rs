// Parser: tokens → cons-cell ASTs in the heap.
//
// The AST is cons lists all the way down. No separate AST type.
// Code is data — the lisp heritage.
//
// Special forms emitted:
//   (send receiver 'selector args...)   — from [obj sel: arg]
//   (%dot obj 'field)                   — from obj.field
//   (%block (params) body)              — from |x| expr
//   (%object-literal items...)          — from { Parent x: 10 }

use crate::heap::Heap;
use crate::lang::lexer::Token;
use crate::value::Value;

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    heap: &'a mut Heap,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token], heap: &'a mut Heap) -> Self {
        Parser { tokens, pos: 0, heap }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        let tok = self.advance();
        if &tok == expected { Ok(()) }
        else { Err(format!("expected {expected:?}, got {tok:?}")) }
    }

    fn intern(&mut self, name: &str) -> Value {
        Value::symbol(self.heap.intern(name))
    }

    fn quoted(&mut self, val: Value) -> Value {
        let q = self.intern("quote");
        let inner = self.heap.cons(val, Value::NIL);
        self.heap.cons(q, inner)
    }

    pub fn parse_expr(&mut self) -> Result<Value, String> {
        match self.peek().clone() {
            Token::LParen => self.parse_list(),
            Token::LBracket => self.parse_send(),
            Token::LBrace => self.parse_object_literal(),
            Token::Pipe => self.parse_block(),

            Token::Quote => { self.advance(); let e = self.parse_expr()?; Ok(self.quoted(e)) }

            Token::Integer(n) => { self.advance(); Ok(Value::integer(n)) }
            Token::Float(f) => { self.advance(); Ok(Value::float(f)) }

            Token::String(ref s) => {
                let s = s.clone(); self.advance();
                Ok(self.heap.alloc_string(&s))
            }

            Token::Symbol(ref name) => {
                let name = name.clone(); self.advance();
                // resolve well-known literals
                let val = match name.as_str() {
                    "nil" => Value::NIL,
                    "true" => Value::TRUE,
                    "false" => Value::FALSE,
                    _ => self.intern(&name),
                };
                self.parse_dot_chain(val)
            }

            Token::At => {
                self.advance();
                match self.peek().clone() {
                    Token::Symbol(ref field) => {
                        let field = field.clone(); self.advance();
                        let dot = self.intern("%dot");
                        let self_sym = self.intern("self");
                        let field_sym = self.intern(&field);
                        let qfield = self.quoted(field_sym);
                        let args = self.heap.list(&[dot, self_sym, qfield]);
                        self.parse_dot_chain(args)
                    }
                    _ => Err("expected field name after @".into()),
                }
            }

            Token::Arrow => {
                self.advance();
                Ok(self.intern("<-"))
            }

            Token::Eof => Err("unexpected end of input".into()),
            ref tok => Err(format!("unexpected token: {tok:?}")),
        }
    }

    fn parse_dot_chain(&mut self, mut result: Value) -> Result<Value, String> {
        while self.peek() == &Token::DotAccess {
            self.advance();
            match self.peek().clone() {
                Token::Symbol(ref field) => {
                    let field = field.clone(); self.advance();
                    let dot = self.intern("%dot");
                    let field_sym = self.intern(&field);
                        let qfield = self.quoted(field_sym);
                    result = self.heap.list(&[dot, result, qfield]);
                }
                _ => return Err("expected field name after dot".into()),
            }
        }
        Ok(result)
    }

    fn parse_list(&mut self) -> Result<Value, String> {
        self.advance(); // (
        let mut items = Vec::new();
        let mut dotted_tail = None;

        loop {
            match self.peek() {
                Token::RParen => { self.advance(); break; }
                Token::Dot => {
                    self.advance();
                    dotted_tail = Some(self.parse_expr()?);
                    self.expect(&Token::RParen)?;
                    break;
                }
                Token::Eof => return Err("unterminated list".into()),
                _ => items.push(self.parse_expr()?),
            }
        }

        let mut result = dotted_tail.unwrap_or(Value::NIL);
        for item in items.into_iter().rev() {
            result = self.heap.cons(item, result);
        }
        Ok(result)
    }

    fn parse_send(&mut self) -> Result<Value, String> {
        self.advance(); // [
        let receiver = self.parse_expr()?;

        // check for eventual send: [obj <- sel: arg]
        let eventual = if let Token::Arrow = self.peek() {
            self.advance(); true
        } else {
            false
        };

        let mut sel_parts = Vec::new();
        let mut args = Vec::new();

        loop {
            match self.peek().clone() {
                Token::RBracket => { self.advance(); break; }
                Token::Keyword(ref k) => {
                    let k = k.clone(); self.advance();
                    sel_parts.push(k);
                    if self.peek() != &Token::RBracket {
                        args.push(self.parse_expr()?);
                    }
                }
                Token::Symbol(ref s) if sel_parts.is_empty() && args.is_empty() => {
                    let s = s.clone(); self.advance();
                    if is_operator(&s) && self.peek() != &Token::RBracket {
                        sel_parts.push(s);
                        args.push(self.parse_expr()?);
                    } else {
                        sel_parts.push(s);
                    }
                }
                Token::Eof => return Err("unterminated send".into()),
                _ => args.push(self.parse_expr()?),
            }
        }

        if sel_parts.is_empty() {
            return Err("empty message send".into());
        }

        let selector = sel_parts.join("");
        let send_sym = self.intern(if eventual { "%eventual-send" } else { "send" });
        let sel_sym = self.intern(&selector);
        let sel_val = self.quoted(sel_sym);

        let mut all = vec![send_sym, receiver, sel_val];
        all.extend(args);
        Ok(self.heap.list(&all))
    }

    fn parse_object_literal(&mut self) -> Result<Value, String> {
        self.advance(); // {
        let obj_sym = self.intern("%object-literal");
        let mut items = Vec::new();

        loop {
            match self.peek().clone() {
                Token::RBrace => { self.advance(); break; }
                Token::Eof => return Err("unterminated object literal".into()),
                _ => items.push(self.parse_expr()?),
            }
        }

        let mut all = vec![obj_sym];
        all.extend(items);
        Ok(self.heap.list(&all))
    }

    fn parse_block(&mut self) -> Result<Value, String> {
        self.advance(); // |

        // parse params until closing |
        let mut params = Vec::new();
        loop {
            match self.peek().clone() {
                Token::Pipe => { self.advance(); break; }
                Token::Symbol(ref name) => {
                    let name = name.clone(); self.advance();
                    params.push(self.intern(&name));
                }
                Token::Eof => return Err("unterminated block params".into()),
                ref tok => return Err(format!("unexpected {tok:?} in block params")),
            }
        }

        // parse body (single expression)
        let body = self.parse_expr()?;

        let block_sym = self.intern("%block");
        let param_list = self.heap.list(&params);
        Ok(self.heap.list(&[block_sym, param_list, body]))
    }

    pub fn parse_all(&mut self) -> Result<Vec<Value>, String> {
        let mut exprs = Vec::new();
        loop {
            if self.peek() == &Token::Eof { break; }
            exprs.push(self.parse_expr()?);
        }
        Ok(exprs)
    }
}

fn is_operator(s: &str) -> bool {
    matches!(s, "+" | "-" | "*" | "/" | "%" | "=" | "<" | ">" | "<=" | ">="
        | "!=" | "==" | "++" | "**" | "<-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::lexer;

    fn parse_one(src: &str) -> (Heap, Value) {
        let tokens = lexer::tokenize(src).unwrap();
        let mut heap = Heap::new();
        let mut parser = Parser::new(&tokens, &mut heap);
        let expr = parser.parse_expr().unwrap();
        (heap, expr)
    }

    #[test]
    fn parse_integer() {
        let (_, val) = parse_one("42");
        assert_eq!(val.as_integer(), Some(42));
    }

    #[test]
    fn parse_list() {
        let (heap, val) = parse_one("(def x 42)");
        let items = heap.list_to_vec(val);
        assert_eq!(items.len(), 3);
        assert!(items[0].is_symbol()); // def
        assert!(items[1].is_symbol()); // x
        assert_eq!(items[2].as_integer(), Some(42));
    }

    #[test]
    fn parse_send() {
        let (heap, val) = parse_one("[3 + 4]");
        let items = heap.list_to_vec(val);
        assert_eq!(items.len(), 4); // (send 3 '+ 4)
        assert!(items[0].is_symbol()); // send
        assert_eq!(items[1].as_integer(), Some(3));
    }

    #[test]
    fn parse_block() {
        let (heap, val) = parse_one("|x| [x + 1]");
        let items = heap.list_to_vec(val);
        assert_eq!(items.len(), 3); // (%block (x) [x + 1])
    }
}
