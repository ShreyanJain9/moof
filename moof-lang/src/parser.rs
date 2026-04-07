/// The MOOF parser — turns tokens into cons-cell ASTs.
///
/// Three bracket species (§3.1):
///   (f a b c)           → applicative call: list in cons cells
///   [obj sel: a]        → message send: (%send obj sel: a)
///   { Parent x: 10 }   → object literal: (%object-literal ...)
///
/// Sugar:
///   obj.field           → (%dot obj 'field)
///   @field              → (%dot self 'field)
///   'x                  → (quote x)
///
/// The AST is just cons cells. Code is data.

use crate::lexer::Token;
use moof_fabric::{Value, Heap};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    /// Parse all expressions until end of input.
    pub fn parse_all(&mut self, heap: &mut Heap) -> Result<Vec<Value>, String> {
        let mut exprs = Vec::new();
        while self.pos < self.tokens.len() {
            exprs.push(self.parse_expr(heap)?);
        }
        Ok(exprs)
    }

    /// Parse an expression, then check for postfix dot-access chains.
    pub fn parse_expr(&mut self, heap: &mut Heap) -> Result<Value, String> {
        let mut expr = self.parse_primary(heap)?;

        // Check for dot-access chains: obj.x.y.z (tight dots only)
        while self.pos < self.tokens.len() && self.tokens[self.pos] == Token::DotAccess {
            self.pos += 1; // skip .
            if self.pos < self.tokens.len() {
                if let Token::Symbol(ref name) = self.tokens[self.pos] {
                    let name = name.clone();
                    self.pos += 1;
                    let dot_sym = Value::Symbol(heap.intern("%dot"));
                    let quote_sym = Value::Symbol(heap.intern("quote"));
                    let field_sym = Value::Symbol(heap.intern(&name));
                    let quoted_field = heap.list(&[quote_sym, field_sym]);
                    expr = heap.list(&[dot_sym, expr, quoted_field]);
                } else {
                    return Err("Expected field name after '.'".into());
                }
            } else {
                return Err("Expected field name after '.'".into());
            }
        }

        Ok(expr)
    }

    /// Parse a primary expression (atom, list, bracket, brace, quote, etc).
    fn parse_primary(&mut self, heap: &mut Heap) -> Result<Value, String> {
        if self.pos >= self.tokens.len() {
            return Err("Unexpected end of input".into());
        }

        match self.tokens[self.pos].clone() {
            Token::LParen => self.parse_paren(heap),
            Token::LBracket => self.parse_bracket(heap),
            Token::LBrace => self.parse_brace(heap),
            Token::Quote => self.parse_quote(heap),
            Token::Quasiquote => {
                self.pos += 1;
                let expr = self.parse_expr(heap)?;
                let qq_sym = Value::Symbol(heap.intern("quasiquote"));
                Ok(heap.list(&[qq_sym, expr]))
            }
            Token::Unquote => {
                self.pos += 1;
                let expr = self.parse_expr(heap)?;
                let uq_sym = Value::Symbol(heap.intern("unquote"));
                Ok(heap.list(&[uq_sym, expr]))
            }
            Token::UnquoteSplice => {
                self.pos += 1;
                let expr = self.parse_expr(heap)?;
                let uqs_sym = Value::Symbol(heap.intern("unquote-splicing"));
                Ok(heap.list(&[uqs_sym, expr]))
            }
            Token::Integer(n) => { self.pos += 1; Ok(Value::Integer(n)) }
            Token::Float(f) => { self.pos += 1; Ok(Value::Float(f)) }
            Token::StringLit(s) => { self.pos += 1; Ok(heap.alloc_string(&s)) }
            Token::Symbol(ref name) => {
                let name = name.clone();
                self.pos += 1;
                match name.as_str() {
                    "nil" => Ok(Value::Nil),
                    "true" => Ok(Value::True),
                    "false" => Ok(Value::False),
                    _ => Ok(Value::Symbol(heap.intern(&name))),
                }
            }
            Token::AtField(ref name) => {
                // @field → (%dot self 'field)
                let name = name.clone();
                self.pos += 1;
                let dot_sym = Value::Symbol(heap.intern("%dot"));
                let self_sym = Value::Symbol(heap.intern("self"));
                let quote_sym = Value::Symbol(heap.intern("quote"));
                let field_sym = Value::Symbol(heap.intern(&name));
                let quoted_field = heap.list(&[quote_sym, field_sym]);
                Ok(heap.list(&[dot_sym, self_sym, quoted_field]))
            }
            Token::DollarSymbol(ref name) => {
                let name = name.clone();
                self.pos += 1;
                Ok(Value::Symbol(heap.intern(&format!("${}", name))))
            }
            Token::Keyword(ref kw) => {
                let kw = kw.clone();
                self.pos += 1;
                Ok(Value::Symbol(heap.intern(&kw)))
            }
            Token::Colon => {
                Err("Unexpected ':'".into())
            }
            t => Err(format!("Unexpected token: {:?}", t)),
        }
    }

    /// Parse (f a b c) or (a . b) — applicative call or dotted pair
    fn parse_paren(&mut self, heap: &mut Heap) -> Result<Value, String> {
        self.pos += 1; // skip (
        let mut elements = Vec::new();
        let mut dotted_cdr = None;

        while self.pos < self.tokens.len() && self.tokens[self.pos] != Token::RParen {
            if self.tokens[self.pos] == Token::Dot {
                // Dotted pair: (a b . c)
                self.pos += 1; // skip .
                dotted_cdr = Some(self.parse_expr(heap)?);
                break;
            }
            elements.push(self.parse_expr(heap)?);
        }
        if self.pos >= self.tokens.len() || self.tokens[self.pos] != Token::RParen {
            return Err("Unclosed '('".into());
        }
        self.pos += 1; // skip )

        // Build the cons list
        let mut result = dotted_cdr.unwrap_or(Value::Nil);
        for &v in elements.iter().rev() {
            result = heap.cons(v, result);
        }
        Ok(result)
    }

    /// Parse [obj sel: a sel2: b] — message send
    /// Produces: (%send obj selector arg1 arg2 ...)
    fn parse_bracket(&mut self, heap: &mut Heap) -> Result<Value, String> {
        self.pos += 1; // skip [
        if self.pos >= self.tokens.len() {
            return Err("Unclosed '['".into());
        }

        // Parse receiver
        let receiver = self.parse_expr(heap)?;

        if self.pos < self.tokens.len() && self.tokens[self.pos] == Token::RBracket {
            self.pos += 1;
            return Ok(receiver);
        }

        // Check for eventual send: [obj <- selector: arg]
        let eventual = match &self.tokens[self.pos] {
            Token::Symbol(name) if name == "<-" => {
                self.pos += 1; // consume <-
                true
            }
            _ => false,
        };

        let mut selector_parts = Vec::new();
        let mut args = Vec::new();

        // After consuming optional <-, parse selector + args normally
        if self.pos < self.tokens.len() && self.tokens[self.pos] != Token::RBracket {
            match &self.tokens[self.pos] {
                Token::Keyword(_kw) => {
                    while self.pos < self.tokens.len() {
                        match &self.tokens[self.pos] {
                            Token::Keyword(kw) => {
                                selector_parts.push(kw.clone());
                                self.pos += 1;
                                args.push(self.parse_expr(heap)?);
                            }
                            Token::RBracket => break,
                            _ => {
                                args.push(self.parse_expr(heap)?);
                            }
                        }
                    }
                }
                Token::Symbol(name) if is_binary_operator(name) => {
                    let op = name.clone();
                    self.pos += 1;
                    selector_parts.push(op);
                    while self.pos < self.tokens.len() && self.tokens[self.pos] != Token::RBracket {
                        args.push(self.parse_expr(heap)?);
                    }
                }
                _ => {
                    let sel = self.parse_expr(heap)?;
                    match sel {
                        Value::Symbol(sym_id) => {
                            selector_parts.push(heap.symbol_name(sym_id).to_string());
                        }
                        _ => return Err("Expected selector symbol in message send".into()),
                    }
                }
            }
        }

        if self.pos >= self.tokens.len() || self.tokens[self.pos] != Token::RBracket {
            return Err("Unclosed '['".into());
        }
        self.pos += 1; // skip ]

        let head = if eventual {
            Value::Symbol(heap.intern("%eventual-send"))
        } else {
            Value::Symbol(heap.intern("%send"))
        };
        let selector_str = selector_parts.join("");
        let selector = Value::Symbol(heap.intern(&selector_str));

        let mut elements = vec![head, receiver, selector];
        elements.extend(args);
        Ok(heap.list(&elements))
    }

    /// Parse { ... } — object literal OR block (closure)
    ///
    /// Block syntax (§3.4):
    ///   { :x [x * 2] }       → object with call: handler taking one arg
    ///   { :x :y [x + y] }    → object with call: handler taking two args
    ///
    /// Object literal syntax:
    ///   { Parent key: value key: (params) body... }
    ///
    /// Detection: if first token is Colon (`:x` block param), it's a block.
    /// Otherwise it's an object literal.
    ///
    /// Blocks produce: (lambda (params...) body)
    fn parse_brace(&mut self, heap: &mut Heap) -> Result<Value, String> {
        self.pos += 1; // skip {

        if self.pos >= self.tokens.len() {
            return Err("Unclosed '{'".into());
        }

        // Check for empty object
        if self.tokens[self.pos] == Token::RBrace {
            self.pos += 1;
            let tag = Value::Symbol(heap.intern("%object-literal"));
            return Ok(heap.list(&[tag, Value::Nil]));
        }

        // Detect block syntax: { :param ... }
        if self.tokens[self.pos] == Token::Colon {
            return self.parse_block(heap);
        }

        // Determine parent: if first token is not a Keyword, parse as parent expression
        let parent = if !matches!(self.tokens[self.pos], Token::Keyword(_)) {
            self.parse_expr(heap)?
        } else {
            Value::Nil
        };

        let slot_sym = Value::Symbol(heap.intern("%slot"));
        let method_sym = Value::Symbol(heap.intern("%method"));
        let mut entries = Vec::new();

        while self.pos < self.tokens.len() && self.tokens[self.pos] != Token::RBrace {
            // Expect a keyword
            let keyword = match &self.tokens[self.pos] {
                Token::Keyword(kw) => {
                    let kw = kw.clone();
                    self.pos += 1;
                    kw
                }
                t => return Err(format!("Expected keyword in object literal, got {:?}", t)),
            };

            // Parse the first expression after the keyword
            let first_expr = self.parse_expr(heap)?;

            // Peek: if next is Keyword or RBrace, this was a slot.
            // Otherwise, first_expr is params and remaining exprs are the method body.
            let next_is_boundary = self.pos >= self.tokens.len()
                || self.tokens[self.pos] == Token::RBrace
                || matches!(self.tokens[self.pos], Token::Keyword(_));

            if next_is_boundary {
                // Slot: key (without trailing colon) → value
                let key_name = keyword.trim_end_matches(':');
                let key_sym = Value::Symbol(heap.intern(key_name));
                entries.push(heap.list(&[slot_sym, key_sym, first_expr]));
            } else {
                // Method: first_expr is the param list, collect body expressions
                let mut body_exprs = Vec::new();
                while self.pos < self.tokens.len()
                    && self.tokens[self.pos] != Token::RBrace
                    && !matches!(self.tokens[self.pos], Token::Keyword(_))
                {
                    body_exprs.push(self.parse_expr(heap)?);
                }

                // Determine selector: if params is empty (Nil), strip colon (unary).
                // Otherwise keep keyword as-is (it has the colon).
                let params_vec = heap.list_to_vec(first_expr);
                let selector_name = if params_vec.is_empty() {
                    keyword.trim_end_matches(':').to_string()
                } else {
                    keyword.clone()
                };

                let sel_sym = Value::Symbol(heap.intern(&selector_name));
                let mut method_entry = vec![method_sym, sel_sym, first_expr];
                method_entry.extend(body_exprs);
                entries.push(heap.list(&method_entry));
            }
        }

        if self.pos >= self.tokens.len() || self.tokens[self.pos] != Token::RBrace {
            return Err("Unclosed '{'".into());
        }
        self.pos += 1; // skip }

        let tag = Value::Symbol(heap.intern("%object-literal"));
        let mut elements = vec![tag, parent];
        elements.extend(entries);
        Ok(heap.list(&elements))
    }

    /// Parse block syntax: { :x :y body... }
    /// Produces: (lambda (x y) body) — a closure, which is an object with call:.
    /// "blocks are closures are objects" (§3.4)
    fn parse_block(&mut self, heap: &mut Heap) -> Result<Value, String> {
        let mut params = Vec::new();

        // Collect :param declarations
        while self.pos < self.tokens.len() && self.tokens[self.pos] == Token::Colon {
            self.pos += 1; // skip :
            match &self.tokens[self.pos] {
                Token::Symbol(name) => {
                    params.push(Value::Symbol(heap.intern(name)));
                    self.pos += 1;
                }
                t => return Err(format!("Expected parameter name after ':', got {:?}", t)),
            }
        }

        // Parse body expressions until }
        let mut body_exprs = Vec::new();
        while self.pos < self.tokens.len() && self.tokens[self.pos] != Token::RBrace {
            body_exprs.push(self.parse_expr(heap)?);
        }

        if self.pos >= self.tokens.len() || self.tokens[self.pos] != Token::RBrace {
            return Err("Unclosed '{' in block".into());
        }
        self.pos += 1; // skip }

        if body_exprs.is_empty() {
            return Err("Block requires at least one body expression".into());
        }

        // Wrap multiple body exprs in (do ...)
        let body = if body_exprs.len() == 1 {
            body_exprs.into_iter().next().unwrap()
        } else {
            let do_sym = Value::Symbol(heap.intern("do"));
            let mut do_form = vec![do_sym];
            do_form.extend(body_exprs);
            heap.list(&do_form)
        };

        // Produce (lambda (params...) body)
        let lambda_sym = Value::Symbol(heap.intern("lambda"));
        let params_list = heap.list(&params);
        Ok(heap.list(&[lambda_sym, params_list, body]))
    }

    /// Parse 'x → (quote x)
    fn parse_quote(&mut self, heap: &mut Heap) -> Result<Value, String> {
        self.pos += 1; // skip '
        let expr = self.parse_expr(heap)?;
        let quote_sym = Value::Symbol(heap.intern("quote"));
        Ok(heap.list(&[quote_sym, expr]))
    }
}

fn is_binary_operator(name: &str) -> bool {
    matches!(name, "+" | "-" | "*" | "/" | "%" | "<" | ">" | "=" | "++"
        | "<=" | ">=" | "!=" | "==" | "&&" | "||")
}
