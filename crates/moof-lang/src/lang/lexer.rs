/// Tokenizer for the moof surface syntax.
///
/// Moof has a lisp-shaped surface with three bracket species, keyword
/// syntax (trailing colon), tight/loose dot disambiguation, and the
/// usual quasiquote sugar.

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
    // Decimal literal whose magnitude exceeds i48's payload. Carried
    // as a string so the parser can hand it to Heap::alloc_integer
    // (which sits above this crate and knows about BigInt).
    BigInteger(std::string::String),
    Float(f64),
    String(std::string::String),
    Symbol(std::string::String),
    Keyword(std::string::String), // includes trailing colon, e.g. "name:"

    // sugar
    Quote,
    Backtick,
    Comma,
    CommaAt,
    DotAccess, // tight dot — no preceding whitespace
    Dot,       // loose dot — whitespace before
    At,
    Pipe,
    Arrow, // <-
    Hash,  // #

    Eof,
}

pub struct Lexer {
    chars: Vec<char>,
    /// Byte offset of each char (parallel to `chars`). Length is
    /// `chars.len() + 1` — the extra entry at the end is the byte
    /// length of the input, used as the end-of-input offset.
    char_byte_offsets: Vec<u32>,
    pos: usize,
    /// true when the character immediately before `pos` was whitespace
    /// (or we are at the start of input)
    prev_was_space: bool,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let mut char_byte_offsets = Vec::with_capacity(chars.len() + 1);
        let mut byte = 0u32;
        for c in &chars {
            char_byte_offsets.push(byte);
            byte += c.len_utf8() as u32;
        }
        char_byte_offsets.push(byte);
        Lexer {
            chars,
            char_byte_offsets,
            pos: 0,
            prev_was_space: true,
        }
    }

    /// Convert a char position to a byte offset. Clamps to the
    /// end-of-input offset for positions past the last char.
    fn byte_at(&self, char_pos: usize) -> u32 {
        if char_pos < self.char_byte_offsets.len() {
            self.char_byte_offsets[char_pos]
        } else {
            *self.char_byte_offsets.last().unwrap_or(&0)
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        self.tokenize_with_spans().0
    }

    /// Tokenize, returning both the tokens and a parallel Vec of
    /// (byte_start, byte_end) spans for each token. The parser
    /// uses spans to record per-form locations on the heap so
    /// `[v __form-text]` can return verbatim source for any
    /// parsed sub-form.
    pub fn tokenize_with_spans(&mut self) -> (Vec<Token>, Vec<(u32, u32)>) {
        let mut tokens = Vec::new();
        let mut spans: Vec<(u32, u32)> = Vec::new();
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];

            // whitespace — skip, mark prev_was_space
            if c.is_ascii_whitespace() {
                self.prev_was_space = true;
                self.pos += 1;
                continue;
            }

            // line comment
            if c == ';' {
                while self.pos < self.chars.len() && self.chars[self.pos] != '\n' {
                    self.pos += 1;
                }
                // don't clear prev_was_space — comment acts like whitespace
                self.prev_was_space = true;
                continue;
            }

            // record start position before emitting
            let start_char = self.pos;

            // from here on we will emit a token
            match c {
                '(' => { tokens.push(Token::LParen); self.pos += 1; }
                ')' => { tokens.push(Token::RParen); self.pos += 1; }
                '[' => { tokens.push(Token::LBracket); self.pos += 1; }
                ']' => { tokens.push(Token::RBracket); self.pos += 1; }
                '{' => { tokens.push(Token::LBrace); self.pos += 1; }
                '}' => { tokens.push(Token::RBrace); self.pos += 1; }
                '\'' => { tokens.push(Token::Quote); self.pos += 1; }
                '`' => { tokens.push(Token::Backtick); self.pos += 1; }
                ',' => {
                    self.pos += 1;
                    if self.pos < self.chars.len() && self.chars[self.pos] == '@' {
                        tokens.push(Token::CommaAt);
                        self.pos += 1;
                    } else {
                        tokens.push(Token::Comma);
                    }
                }
                '@' => { tokens.push(Token::At); self.pos += 1; }
                '#' => { tokens.push(Token::Hash); self.pos += 1; }
                '|' => { tokens.push(Token::Pipe); self.pos += 1; }
                '.' => {
                    if self.prev_was_space {
                        tokens.push(Token::Dot);
                    } else {
                        tokens.push(Token::DotAccess);
                    }
                    self.pos += 1;
                }
                '"' => {
                    tokens.push(self.read_string());
                }
                _ => {
                    // negative number: - immediately followed by digit
                    if c == '-' && self.peek_next().map_or(false, |n| n.is_ascii_digit()) {
                        tokens.push(self.read_number(true));
                    } else if c.is_ascii_digit() {
                        tokens.push(self.read_number(false));
                    } else if c == '<' && self.peek_next() == Some('-') {
                        tokens.push(Token::Arrow);
                        self.pos += 2;
                    } else {
                        tokens.push(self.read_symbol());
                    }
                }
            }

            // record span for the just-emitted token
            spans.push((self.byte_at(start_char), self.byte_at(self.pos)));

            self.prev_was_space = false;
        }
        (tokens, spans)
    }

    fn peek_next(&self) -> Option<char> {
        if self.pos + 1 < self.chars.len() {
            Some(self.chars[self.pos + 1])
        } else {
            None
        }
    }

    fn read_string(&mut self) -> Token {
        self.pos += 1; // skip opening "
        let mut s = std::string::String::new();
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == '"' {
                self.pos += 1;
                return Token::String(s);
            }
            if c == '\\' {
                self.pos += 1;
                if self.pos < self.chars.len() {
                    match self.chars[self.pos] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        other => {
                            s.push('\\');
                            s.push(other);
                        }
                    }
                }
            } else {
                s.push(c);
            }
            self.pos += 1;
        }
        // unterminated string — return what we have
        Token::String(s)
    }

    fn read_number(&mut self, negative: bool) -> Token {
        let start = self.pos;
        if negative {
            self.pos += 1; // skip -
        }
        while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        // check for float
        if self.pos < self.chars.len()
            && self.chars[self.pos] == '.'
            && self.pos + 1 < self.chars.len()
            && self.chars[self.pos + 1].is_ascii_digit()
        {
            self.pos += 1; // skip .
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let text: std::string::String = self.chars[start..self.pos].iter().collect();
            Token::Float(text.parse().unwrap_or(f64::NAN))
        } else {
            let text: std::string::String = self.chars[start..self.pos].iter().collect();
            // fits in i48? fast path. else promote to BigInteger, which
            // the parser will realize as a foreign BigInt on the heap.
            // still one moof-level Integer type either way.
            const I48_MAX: i64 = (1 << 47) - 1;
            const I48_MIN: i64 = -(1 << 47);
            match text.parse::<i64>() {
                Ok(n) if n >= I48_MIN && n <= I48_MAX => Token::Integer(n),
                _ => Token::BigInteger(text),
            }
        }
    }

    fn read_symbol(&mut self) -> Token {
        let start = self.pos;
        while self.pos < self.chars.len() && is_symbol_char(self.chars[self.pos]) {
            self.pos += 1;
        }
        let text: std::string::String = self.chars[start..self.pos].iter().collect();
        // keyword: ends with colon
        if text.ends_with(':') {
            Token::Keyword(text)
        } else {
            Token::Symbol(text)
        }
    }
}

fn is_symbol_char(c: char) -> bool {
    !c.is_ascii_whitespace()
        && !matches!(c, '(' | ')' | '[' | ']' | '{' | '}')
        && !matches!(c, '\'' | '`' | ',' | ';' | '@' | '|' | '.' | '"' | '#')
}

/// Convenience: tokenize a string in one call.
pub fn tokens(input: &str) -> Vec<Token> {
    Lexer::new(input).tokenize()
}

/// Tokenize, returning Result for the REPL. Appends Eof.
pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut toks = Lexer::new(input).tokenize();
    toks.push(Token::Eof);
    Ok(toks)
}

/// Tokenize with parallel byte spans. Used by the parser to
/// record per-form locations for verbatim source retrieval.
/// Spans is the same length as the returned token vec; the Eof
/// is paired with a zero-length span at end-of-input.
pub fn tokenize_with_spans(input: &str) -> Result<(Vec<Token>, Vec<(u32, u32)>), String> {
    let (mut toks, mut spans) = Lexer::new(input).tokenize_with_spans();
    let eof_byte = input.len() as u32;
    toks.push(Token::Eof);
    spans.push((eof_byte, eof_byte));
    Ok((toks, spans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Token::*;

    #[test]
    fn basic_tokens() {
        assert_eq!(
            tokens("(def x 42)"),
            vec![
                LParen,
                Symbol("def".into()),
                Symbol("x".into()),
                Integer(42),
                RParen,
            ]
        );
    }

    #[test]
    fn brackets_and_keywords() {
        assert_eq!(
            tokens("[obj at: k put: v]"),
            vec![
                LBracket,
                Symbol("obj".into()),
                Keyword("at:".into()),
                Symbol("k".into()),
                Keyword("put:".into()),
                Symbol("v".into()),
                RBracket,
            ]
        );
    }

    #[test]
    fn string_with_escapes() {
        assert_eq!(
            tokens(r#""hello\nworld""#),
            vec![String("hello\nworld".into())]
        );
        assert_eq!(
            tokens(r#""tab\there""#),
            vec![String("tab\there".into())]
        );
        assert_eq!(
            tokens(r#""escaped\\slash""#),
            vec![String("escaped\\slash".into())]
        );
        assert_eq!(
            tokens(r#""say \"hi\"""#),
            vec![String("say \"hi\"".into())]
        );
    }

    #[test]
    fn float_literal() {
        assert_eq!(tokens("3.14"), vec![Float(3.14)]);
        assert_eq!(tokens("-0.5"), vec![Float(-0.5)]);
    }

    #[test]
    fn dot_disambiguation() {
        // tight dot: no space before .
        assert_eq!(
            tokens("obj.x"),
            vec![
                Symbol("obj".into()),
                DotAccess,
                Symbol("x".into()),
            ]
        );
        // loose dot: space before .
        assert_eq!(
            tokens("(a . b)"),
            vec![
                LParen,
                Symbol("a".into()),
                Dot,
                Symbol("b".into()),
                RParen,
            ]
        );
    }

    #[test]
    fn block_syntax() {
        assert_eq!(
            tokens("|x| [x + 1]"),
            vec![
                Pipe,
                Symbol("x".into()),
                Pipe,
                LBracket,
                Symbol("x".into()),
                Symbol("+".into()),
                Integer(1),
                RBracket,
            ]
        );
    }

    #[test]
    fn eventual_send() {
        assert_eq!(
            tokens("[obj <- sel: arg]"),
            vec![
                LBracket,
                Symbol("obj".into()),
                Arrow,
                Keyword("sel:".into()),
                Symbol("arg".into()),
                RBracket,
            ]
        );
    }

    #[test]
    fn negative_numbers() {
        assert_eq!(tokens("-7"), vec![Integer(-7)]);
        assert_eq!(tokens("-42"), vec![Integer(-42)]);
        // minus as operator (space after -)
        assert_eq!(
            tokens("(- 3 1)"),
            vec![
                LParen,
                Symbol("-".into()),
                Integer(3),
                Integer(1),
                RParen,
            ]
        );
    }

    #[test]
    fn comments() {
        assert_eq!(
            tokens("(def x 1) ; this is a comment\n(def y 2)"),
            vec![
                LParen,
                Symbol("def".into()),
                Symbol("x".into()),
                Integer(1),
                RParen,
                LParen,
                Symbol("def".into()),
                Symbol("y".into()),
                Integer(2),
                RParen,
            ]
        );
    }

    #[test]
    fn quote_sugar() {
        assert_eq!(
            tokens("'(a b)"),
            vec![
                Quote,
                LParen,
                Symbol("a".into()),
                Symbol("b".into()),
                RParen,
            ]
        );
    }

    #[test]
    fn quasiquote_sugar() {
        assert_eq!(
            tokens("`(a ,b ,@rest)"),
            vec![
                Backtick,
                LParen,
                Symbol("a".into()),
                Comma,
                Symbol("b".into()),
                CommaAt,
                Symbol("rest".into()),
                RParen,
            ]
        );
    }

    #[test]
    fn at_sign() {
        assert_eq!(
            tokens("@slot"),
            vec![At, Symbol("slot".into())]
        );
    }

    #[test]
    fn braces() {
        assert_eq!(
            tokens("{a 1 b 2}"),
            vec![
                LBrace,
                Symbol("a".into()),
                Integer(1),
                Symbol("b".into()),
                Integer(2),
                RBrace,
            ]
        );
    }

    #[test]
    fn operators_as_symbols() {
        assert_eq!(
            tokens("(+ * <= !=)"),
            vec![
                LParen,
                Symbol("+".into()),
                Symbol("*".into()),
                Symbol("<=".into()),
                Symbol("!=".into()),
                RParen,
            ]
        );
    }
}
