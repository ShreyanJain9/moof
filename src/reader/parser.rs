/// The MOOF parser — turns tokens into cons-cell ASTs.
///
/// Three bracket species (§3.1):
///   (f a b c)       → applicative call: (f a b c) as a list
///   [obj sel: a]    → message send: (%send obj sel: a) — tagged with %send
///   { :x [x * 2] } → block: (%block (x) body)
///
/// The AST is just cons cells. Code is data.

use crate::reader::lexer::Token;
use crate::runtime::value::Value;
use crate::runtime::heap::Heap;

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

    /// Parse a single expression.
    pub fn parse_expr(&mut self, heap: &mut Heap) -> Result<Value, String> {
        if self.pos >= self.tokens.len() {
            return Err("Unexpected end of input".into());
        }

        match self.tokens[self.pos].clone() {
            Token::LParen => self.parse_paren(heap),
            Token::LBracket => self.parse_bracket(heap),
            Token::LBrace => self.parse_brace(heap),
            Token::Quote => self.parse_quote(heap),
            Token::Integer(n) => { self.pos += 1; Ok(Value::Integer(n)) }
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
            Token::HashSymbol(ref name) => {
                // #foo → (quote foo) — a literal symbol value, not a variable lookup
                let name = name.clone();
                self.pos += 1;
                let quote_sym = Value::Symbol(heap.intern("quote"));
                let sym_val = Value::Symbol(heap.intern(&name));
                Ok(heap.list(&[quote_sym, sym_val]))
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
                // Standalone colon — shouldn't appear here
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
    /// where selector is the concatenated keyword string "sel:sel2:"
    fn parse_bracket(&mut self, heap: &mut Heap) -> Result<Value, String> {
        self.pos += 1; // skip [
        if self.pos >= self.tokens.len() {
            return Err("Unclosed '['".into());
        }

        // Parse receiver
        let receiver = self.parse_expr(heap)?;

        // Now parse the message
        // Could be:
        //   [obj slot]                — unary
        //   [obj + 5]                 — binary
        //   [obj at: k]              — keyword
        //   [obj at: k put: v]       — multi-keyword
        if self.pos < self.tokens.len() && self.tokens[self.pos] == Token::RBracket {
            // No message — just return the receiver (parenthesized expression)
            self.pos += 1;
            return Ok(receiver);
        }

        let mut selector_parts = Vec::new();
        let mut args = Vec::new();

        // Check what kind of message this is
        match &self.tokens[self.pos] {
            Token::Keyword(_kw) => {
                // Keyword message: [obj at: k put: v]
                while self.pos < self.tokens.len() {
                    match &self.tokens[self.pos] {
                        Token::Keyword(kw) => {
                            selector_parts.push(kw.clone());
                            self.pos += 1;
                            args.push(self.parse_expr(heap)?);
                        }
                        Token::RBracket => break,
                        _ => {
                            // Could be more args for the current keyword
                            args.push(self.parse_expr(heap)?);
                        }
                    }
                }
            }
            Token::Symbol(name) if is_binary_operator(name) => {
                // Binary message: [obj + 5]
                let op = name.clone();
                self.pos += 1;
                selector_parts.push(op);
                // Parse all args until ]
                while self.pos < self.tokens.len() && self.tokens[self.pos] != Token::RBracket {
                    args.push(self.parse_expr(heap)?);
                }
            }
            _ => {
                // Unary message: [obj negate]
                let sel = self.parse_expr(heap)?;
                match sel {
                    Value::Symbol(sym_id) => {
                        selector_parts.push(heap.symbol_name(sym_id).to_string());
                    }
                    _ => return Err("Expected selector symbol in message send".into()),
                }
            }
        }

        if self.pos >= self.tokens.len() || self.tokens[self.pos] != Token::RBracket {
            return Err("Unclosed '['".into());
        }
        self.pos += 1; // skip ]

        // Build the AST: (%send receiver selector arg1 arg2 ...)
        let send_sym = Value::Symbol(heap.intern("%send"));
        let selector_str = selector_parts.join("");
        let selector = Value::Symbol(heap.intern(&selector_str));

        let mut elements = vec![send_sym, receiver, selector];
        elements.extend(args);
        Ok(heap.list(&elements))
    }

    /// Parse { :x :y [x + y] } — block / object literal
    /// Produces: (%block (x y) body)
    fn parse_brace(&mut self, heap: &mut Heap) -> Result<Value, String> {
        self.pos += 1; // skip {

        // Collect block parameters (:x :y etc.)
        let mut params = Vec::new();
        while self.pos < self.tokens.len() {
            if self.tokens[self.pos] == Token::Colon {
                self.pos += 1; // skip :
                match &self.tokens[self.pos] {
                    Token::Symbol(name) => {
                        params.push(Value::Symbol(heap.intern(name)));
                        self.pos += 1;
                    }
                    _ => return Err("Expected parameter name after ':'".into()),
                }
            } else {
                break;
            }
        }

        // Parse body expressions
        let mut body_exprs = Vec::new();
        while self.pos < self.tokens.len() && self.tokens[self.pos] != Token::RBrace {
            body_exprs.push(self.parse_expr(heap)?);
        }
        if self.pos >= self.tokens.len() {
            return Err("Unclosed '{'".into());
        }
        self.pos += 1; // skip }

        // Body: if multiple expressions, wrap in (%do expr1 expr2 ...)
        let body = if body_exprs.len() == 1 {
            body_exprs.into_iter().next().unwrap()
        } else {
            let do_sym = Value::Symbol(heap.intern("%do"));
            let mut elems = vec![do_sym];
            elems.extend(body_exprs);
            heap.list(&elems)
        };

        let block_sym = Value::Symbol(heap.intern("%block"));
        let param_list = heap.list(&params);
        let elements = vec![block_sym, param_list, body];
        Ok(heap.list(&elements))
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
