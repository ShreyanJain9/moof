//! Parser: tokens → cons-cell AST in the fabric store.
//!
//! The AST is cons lists all the way down. No separate AST type.
//! This is the lisp heritage: code is data.

use moof_fabric::{Store, Value};

use crate::lexer::Token;

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    store: &'a mut Store,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token], store: &'a mut Store) -> Self {
        Parser {
            tokens,
            pos: 0,
            store,
        }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> &Token {
        let tok = self.tokens.get(self.pos).unwrap_or(&Token::Eof);
        self.pos += 1;
        tok
    }

    /// Parse a single expression, returning it as a Value.
    pub fn parse_expr(&mut self) -> Result<Value, String> {
        match self.peek().clone() {
            Token::LParen => self.parse_list(),
            Token::LBracket => self.parse_send(),
            Token::LBrace => self.parse_object_literal(),
            Token::Quote => {
                self.advance();
                let expr = self.parse_expr()?;
                let quote_sym = self.store.intern("quote")?;
                // (quote expr)
                let inner = self.store.cons(expr, Value::NIL)?;
                let outer = self.store.cons(Value::symbol(quote_sym), Value::object(inner))?;
                Ok(Value::object(outer))
            }
            Token::Integer(n) => {
                let n = n;
                self.advance();
                Ok(Value::integer(n))
            }
            Token::Float(f) => {
                let f = f;
                self.advance();
                Ok(Value::float(f))
            }
            Token::Str(ref s) => {
                let s = s.clone();
                self.advance();
                let id = self.store.alloc_string(&s)?;
                Ok(Value::object(id))
            }
            Token::Symbol(ref name) => {
                let name = name.clone();
                self.advance();
                // check for dot access chain
                let sym_id = self.store.intern(&name)?;
                let mut result = Value::symbol(sym_id);
                while self.peek() == &Token::DotAccess {
                    self.advance(); // consume dot
                    if let Token::Symbol(ref field) = self.peek().clone() {
                        let field = field.clone();
                        self.advance();
                        let dot_sym = self.store.intern("%dot")?;
                        let field_sym = self.store.intern(&field)?;
                        // (%dot obj 'field)
                        let quote_sym = self.store.intern("quote")?;
                        let quoted_field = {
                            let inner = self.store.cons(Value::symbol(field_sym), Value::NIL)?;
                            let outer = self.store.cons(Value::symbol(quote_sym), Value::object(inner))?;
                            Value::object(outer)
                        };
                        let args = self.store.cons(quoted_field, Value::NIL)?;
                        let args = self.store.cons(result, Value::object(args))?;
                        let call = self.store.cons(Value::symbol(dot_sym), Value::object(args))?;
                        result = Value::object(call);
                    } else {
                        return Err("expected field name after dot".into());
                    }
                }
                Ok(result)
            }
            Token::At => {
                self.advance();
                // @field → (%dot self 'field)
                if let Token::Symbol(ref field) = self.peek().clone() {
                    let field = field.clone();
                    self.advance();
                    let dot_sym = self.store.intern("%dot")?;
                    let self_sym = self.store.intern("self")?;
                    let field_sym = self.store.intern(&field)?;
                    let quote_sym = self.store.intern("quote")?;
                    let quoted_field = {
                        let inner = self.store.cons(Value::symbol(field_sym), Value::NIL)?;
                        let outer = self.store.cons(Value::symbol(quote_sym), Value::object(inner))?;
                        Value::object(outer)
                    };
                    let args = self.store.cons(quoted_field, Value::NIL)?;
                    let args = self.store.cons(Value::symbol(self_sym), Value::object(args))?;
                    let call = self.store.cons(Value::symbol(dot_sym), Value::object(args))?;
                    Ok(Value::object(call))
                } else {
                    Err("expected field name after @".into())
                }
            }
            Token::Keyword(ref k) => {
                Err(format!("unexpected keyword {k} outside of message send"))
            }
            Token::Eof => Err("unexpected end of input".into()),
            ref tok => Err(format!("unexpected token: {tok:?}")),
        }
    }

    /// Parse a paren-delimited list: (f a b c) or (a . b)
    fn parse_list(&mut self) -> Result<Value, String> {
        self.advance(); // consume (
        let mut items = Vec::new();
        let mut dotted_tail = None;

        loop {
            match self.peek() {
                Token::RParen => {
                    self.advance();
                    break;
                }
                Token::Dot => {
                    self.advance();
                    dotted_tail = Some(self.parse_expr()?);
                    if self.peek() != &Token::RParen {
                        return Err("expected ) after dotted tail".into());
                    }
                    self.advance();
                    break;
                }
                Token::Eof => return Err("unterminated list".into()),
                _ => items.push(self.parse_expr()?),
            }
        }

        // build the cons list from right to left
        let mut result = dotted_tail.unwrap_or(Value::NIL);
        for item in items.into_iter().rev() {
            let cell = self.store.cons(item, result)?;
            result = Value::object(cell);
        }
        Ok(result)
    }

    /// Parse a bracket-delimited message send: [obj sel: arg ...]
    fn parse_send(&mut self) -> Result<Value, String> {
        self.advance(); // consume [
        let receiver = self.parse_expr()?;

        // collect selector parts and arguments
        let mut selector_parts = Vec::new();
        let mut args = Vec::new();

        loop {
            match self.peek().clone() {
                Token::RBracket => {
                    self.advance();
                    break;
                }
                Token::Keyword(ref k) => {
                    let k = k.clone();
                    self.advance();
                    selector_parts.push(k);
                    // the next token is the argument for this keyword
                    if self.peek() != &Token::RBracket {
                        args.push(self.parse_expr()?);
                    }
                }
                Token::Symbol(ref s) if selector_parts.is_empty() && args.is_empty() => {
                    // could be a unary message or a binary operator
                    let s = s.clone();
                    self.advance();
                    // check if it looks like a binary operator followed by an arg
                    if is_operator(&s) && self.peek() != &Token::RBracket {
                        selector_parts.push(s);
                        args.push(self.parse_expr()?);
                    } else {
                        selector_parts.push(s);
                    }
                }
                Token::Eof => return Err("unterminated message send".into()),
                _ => {
                    // unexpected token in send context
                    let expr = self.parse_expr()?;
                    args.push(expr);
                }
            }
        }

        if selector_parts.is_empty() {
            return Err("empty message send".into());
        }

        // build selector string
        let selector = selector_parts.join("");
        let send_sym = self.store.intern("send")?;
        let sel_sym = self.store.intern(&selector)?;

        // build: (send receiver 'selector arg1 arg2 ...)
        let quote_sym = self.store.intern("quote")?;
        let quoted_sel = {
            let inner = self.store.cons(Value::symbol(sel_sym), Value::NIL)?;
            let outer = self.store.cons(Value::symbol(quote_sym), Value::object(inner))?;
            Value::object(outer)
        };

        let mut list = Value::NIL;
        for arg in args.into_iter().rev() {
            let cell = self.store.cons(arg, list)?;
            list = Value::object(cell);
        }
        let list = self.store.cons(quoted_sel, list)?;
        let list = self.store.cons(receiver, Value::object(list))?;
        let list = self.store.cons(Value::symbol(send_sym), Value::object(list))?;
        Ok(Value::object(list))
    }

    /// Parse an object literal: { Parent key: val ... }
    fn parse_object_literal(&mut self) -> Result<Value, String> {
        self.advance(); // consume {

        // the first element might be a parent expression
        // for now, collect all key:value pairs
        let obj_lit_sym = self.store.intern("%object-literal")?;
        let mut items = Vec::new();

        // check for block syntax: { :x body } or { body }
        // vs object syntax: { Parent key: val }
        // heuristic: if first token after { is a keyword, it's an object or block

        loop {
            match self.peek().clone() {
                Token::RBrace => {
                    self.advance();
                    break;
                }
                Token::Eof => return Err("unterminated object literal".into()),
                _ => items.push(self.parse_expr()?),
            }
        }

        // build: (%object-literal item1 item2 ...)
        let mut list = Value::NIL;
        for item in items.into_iter().rev() {
            let cell = self.store.cons(item, list)?;
            list = Value::object(cell);
        }
        let result = self.store.cons(Value::symbol(obj_lit_sym), list)?;
        Ok(Value::object(result))
    }

    /// Parse all expressions in the token stream.
    pub fn parse_all(&mut self) -> Result<Vec<Value>, String> {
        let mut exprs = Vec::new();
        loop {
            if self.peek() == &Token::Eof {
                break;
            }
            exprs.push(self.parse_expr()?);
        }
        Ok(exprs)
    }
}

fn is_operator(s: &str) -> bool {
    matches!(
        s,
        "+" | "-" | "*" | "/" | "%" | "=" | "<" | ">" | "<=" | ">="
            | "!=" | "==" | "++" | "**"
    )
}
