//! Tokenizer for the moof surface syntax.
//!
//! Three bracket species: () [] {}
//! Sugar: 'symbol, `quasiquote, ,unquote, ,@splice
//! Dot: tight (obj.x) vs loose (a . b)

/// A source position.
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub offset: usize,
    pub len: usize,
}

/// Token types.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // delimiters
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // literals
    Integer(i64),
    Float(f64),
    Str(String),
    Symbol(String),     // bare identifier
    Keyword(String),    // trailing colon: "name:"

    // sugar
    Quote,              // '
    Backtick,           // `
    Comma,              // ,
    CommaAt,            // ,@
    DotAccess,          // tight dot (no preceding whitespace)
    Dot,                // loose dot (whitespace before)
    At,                 // @

    Eof,
}

pub struct Lexer<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            source,
            chars: source.chars().collect(),
            pos: 0,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            if tok == Token::Eof {
                tokens.push(tok);
                break;
            }
            tokens.push(tok);
        }
        Ok(tokens)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        Some(c)
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else if c == ';' {
                // line comment
                while let Some(c) = self.advance() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn had_whitespace_before(&self) -> bool {
        if self.pos == 0 {
            return true;
        }
        self.chars
            .get(self.pos - 1)
            .map(|c| c.is_whitespace() || *c == '(' || *c == '[' || *c == '{')
            .unwrap_or(true)
    }

    fn next_token(&mut self) -> Result<Token, String> {
        self.skip_whitespace_and_comments();

        let Some(c) = self.peek() else {
            return Ok(Token::Eof);
        };

        // remember if we had whitespace before this token
        // (used for tight vs loose dot)
        let ws_before = self.pos == 0
            || self
                .chars
                .get(self.pos.wrapping_sub(1))
                .map(|c| c.is_whitespace() || *c == '(' || *c == '[' || *c == '{')
                .unwrap_or(true);

        match c {
            '(' => {
                self.advance();
                Ok(Token::LParen)
            }
            ')' => {
                self.advance();
                Ok(Token::RParen)
            }
            '[' => {
                self.advance();
                Ok(Token::LBracket)
            }
            ']' => {
                self.advance();
                Ok(Token::RBracket)
            }
            '{' => {
                self.advance();
                Ok(Token::LBrace)
            }
            '}' => {
                self.advance();
                Ok(Token::RBrace)
            }
            '\'' => {
                self.advance();
                Ok(Token::Quote)
            }
            '`' => {
                self.advance();
                Ok(Token::Backtick)
            }
            ',' => {
                self.advance();
                if self.peek() == Some('@') {
                    self.advance();
                    Ok(Token::CommaAt)
                } else {
                    Ok(Token::Comma)
                }
            }
            '@' => {
                self.advance();
                Ok(Token::At)
            }
            '.' => {
                self.advance();
                if ws_before {
                    Ok(Token::Dot)
                } else {
                    Ok(Token::DotAccess)
                }
            }
            '"' => self.read_string(),
            _ if c.is_ascii_digit() || (c == '-' && self.is_number_ahead()) => self.read_number(),
            _ if Self::is_symbol_char(c) => self.read_symbol(),
            _ => {
                self.advance();
                Err(format!("unexpected character: {c}"))
            }
        }
    }

    fn is_number_ahead(&self) -> bool {
        self.chars
            .get(self.pos + 1)
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    }

    fn is_symbol_char(c: char) -> bool {
        !c.is_whitespace()
            && c != '('
            && c != ')'
            && c != '['
            && c != ']'
            && c != '{'
            && c != '}'
            && c != '\''
            && c != '`'
            && c != ','
            && c != '"'
            && c != ';'
            && c != '@'
    }

    fn read_string(&mut self) -> Result<Token, String> {
        self.advance(); // skip opening "
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err("unterminated string".into()),
                Some('"') => return Ok(Token::Str(s)),
                Some('\\') => match self.advance() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some(c) => {
                        s.push('\\');
                        s.push(c);
                    }
                    None => return Err("unterminated escape".into()),
                },
                Some(c) => s.push(c),
            }
        }
    }

    fn read_number(&mut self) -> Result<Token, String> {
        let mut s = String::new();
        if self.peek() == Some('-') {
            s.push('-');
            self.advance();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        // check for float
        if self.peek() == Some('.')
            && self
                .chars
                .get(self.pos + 1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        {
            s.push('.');
            self.advance();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            let f: f64 = s.parse().map_err(|e| format!("bad float: {e}"))?;
            Ok(Token::Float(f))
        } else {
            let n: i64 = s.parse().map_err(|e| format!("bad integer: {e}"))?;
            Ok(Token::Integer(n))
        }
    }

    fn read_symbol(&mut self) -> Result<Token, String> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if Self::is_symbol_char(c) && c != '.' {
                s.push(c);
                self.advance();
            } else if c == '.' {
                // tight dot ends the symbol
                break;
            } else {
                break;
            }
        }
        // check for keyword (trailing colon)
        if s.ends_with(':') {
            Ok(Token::Keyword(s))
        } else {
            Ok(Token::Symbol(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_tokens() {
        let mut l = Lexer::new("(def x 42)");
        let toks = l.tokenize().unwrap();
        assert_eq!(toks[0], Token::LParen);
        assert_eq!(toks[1], Token::Symbol("def".into()));
        assert_eq!(toks[2], Token::Symbol("x".into()));
        assert_eq!(toks[3], Token::Integer(42));
        assert_eq!(toks[4], Token::RParen);
    }

    #[test]
    fn brackets_and_keywords() {
        let mut l = Lexer::new("[obj at: k put: v]");
        let toks = l.tokenize().unwrap();
        assert_eq!(toks[0], Token::LBracket);
        assert_eq!(toks[1], Token::Symbol("obj".into()));
        assert_eq!(toks[2], Token::Keyword("at:".into()));
        assert_eq!(toks[3], Token::Symbol("k".into()));
        assert_eq!(toks[4], Token::Keyword("put:".into()));
        assert_eq!(toks[5], Token::Symbol("v".into()));
        assert_eq!(toks[6], Token::RBracket);
    }

    #[test]
    fn string_literal() {
        let mut l = Lexer::new("\"hello\\nworld\"");
        let toks = l.tokenize().unwrap();
        assert_eq!(toks[0], Token::Str("hello\nworld".into()));
    }

    #[test]
    fn float_literal() {
        let mut l = Lexer::new("3.14");
        let toks = l.tokenize().unwrap();
        assert_eq!(toks[0], Token::Float(3.14));
    }
}
