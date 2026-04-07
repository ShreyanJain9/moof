/// The MOOF lexer — tokenizes the three bracket species and reader sugar.
///
/// Three structural forms (§3.1):
///   (f a b c)        — applicative call
///   [obj sel: a]     — message send
///   { Parent x: 10 } — object literal

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Delimiters
    LParen,     // (
    RParen,     // )
    LBracket,   // [
    RBracket,   // ]
    LBrace,     // {
    RBrace,     // }

    // Literals
    Integer(i64),
    Float(f64),
    StringLit(String),
    Symbol(String),     // regular identifier
    Keyword(String),    // identifier ending with : (e.g., "at:", "put:")

    // Sugar
    Quote,          // '
    Quasiquote,     // `
    Unquote,        // ,
    UnquoteSplice,  // ,@
    Colon,       // : alone (for block params like :x)
    Dot,         // . (loose — used for dotted pairs)
    DotAccess,   // . (tight — no preceding whitespace, used for field access)
    AtField(String), // @name — self-field access sugar

    // Special
    DollarSymbol(String), // $e — environment parameter for vau
}

pub struct Lexer<'a> {
    #[allow(dead_code)] // kept for future error reporting
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
        while self.pos < self.chars.len() {
            self.skip_whitespace();
            if self.pos >= self.chars.len() { break; }

            let ch = self.chars[self.pos];
            match ch {
                // Comments: ; to end of line
                ';' => {
                    while self.pos < self.chars.len() && self.chars[self.pos] != '\n' {
                        self.pos += 1;
                    }
                }

                '(' => { tokens.push(Token::LParen); self.pos += 1; }
                ')' => { tokens.push(Token::RParen); self.pos += 1; }
                '[' => { tokens.push(Token::LBracket); self.pos += 1; }
                ']' => { tokens.push(Token::RBracket); self.pos += 1; }
                '{' => { tokens.push(Token::LBrace); self.pos += 1; }
                '}' => { tokens.push(Token::RBrace); self.pos += 1; }
                '\'' => { tokens.push(Token::Quote); self.pos += 1; }
                '`' => { tokens.push(Token::Quasiquote); self.pos += 1; }
                ',' => {
                    self.pos += 1;
                    if self.pos < self.chars.len() && self.chars[self.pos] == '@' {
                        self.pos += 1;
                        tokens.push(Token::UnquoteSplice);
                    } else {
                        tokens.push(Token::Unquote);
                    }
                }
                '.' => {
                    // Tight dot (no whitespace before) = field access: obj.x
                    // Loose dot (whitespace before) = dotted pair: (a . b)
                    let tight = self.pos > 0 && !self.chars[self.pos - 1].is_whitespace();
                    if tight {
                        tokens.push(Token::DotAccess);
                    } else {
                        tokens.push(Token::Dot);
                    }
                    self.pos += 1;
                }

                '@' => {
                    self.pos += 1;
                    let name = self.read_symbol_chars();
                    if name.is_empty() {
                        return Err("Expected field name after @".to_string());
                    }
                    tokens.push(Token::AtField(name));
                }

                '$' => {
                    self.pos += 1;
                    let name = self.read_symbol_chars();
                    if name.is_empty() {
                        return Err("Expected name after $".to_string());
                    }
                    tokens.push(Token::DollarSymbol(name));
                }

                '"' => {
                    tokens.push(self.read_string()?);
                }

                ':' => {
                    self.pos += 1;
                    // Check if this is :name (block param) or standalone colon
                    if self.pos < self.chars.len() && is_symbol_char(self.chars[self.pos]) {
                        let name = self.read_symbol_chars();
                        tokens.push(Token::Colon);
                        tokens.push(Token::Symbol(name));
                    } else {
                        tokens.push(Token::Colon);
                    }
                }

                _ if ch == '-' && self.pos + 1 < self.chars.len() && self.chars[self.pos + 1].is_ascii_digit() => {
                    tokens.push(self.read_number()?);
                }

                _ if ch.is_ascii_digit() => {
                    tokens.push(self.read_number()?);
                }

                _ if is_symbol_start(ch) || is_operator_char(ch) => {
                    if is_operator_char(ch) && !is_symbol_start(ch) {
                        // Operator like +, -, *, /
                        let op = self.read_operator();
                        // Check if followed by : to make it a keyword
                        if self.pos < self.chars.len() && self.chars[self.pos] == ':' {
                            self.pos += 1;
                            tokens.push(Token::Keyword(format!("{}:", op)));
                        } else {
                            tokens.push(Token::Symbol(op));
                        }
                    } else {
                        let name = self.read_symbol_chars();
                        // Check if followed by : to make it a keyword selector
                        if self.pos < self.chars.len() && self.chars[self.pos] == ':' {
                            self.pos += 1;
                            tokens.push(Token::Keyword(format!("{}:", name)));
                        } else {
                            tokens.push(Token::Symbol(name));
                        }
                    }
                }

                _ => return Err(format!("Unexpected character: '{}'", ch)),
            }
        }
        Ok(tokens)
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn read_symbol_chars(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.chars.len() && is_symbol_char(self.chars[self.pos]) {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn read_operator(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.chars.len() && is_operator_char(self.chars[self.pos]) {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn read_number(&mut self) -> Result<Token, String> {
        let start = self.pos;
        if self.chars[self.pos] == '-' { self.pos += 1; }
        while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        // Check for decimal point → float
        if self.pos < self.chars.len() && self.chars[self.pos] == '.'
            && self.pos + 1 < self.chars.len() && self.chars[self.pos + 1].is_ascii_digit()
        {
            self.pos += 1; // skip .
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let s: String = self.chars[start..self.pos].iter().collect();
            let f: f64 = s.parse().map_err(|_| format!("Invalid float: {}", s))?;
            return Ok(Token::Float(f));
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        let n: i64 = s.parse().map_err(|_| format!("Invalid number: {}", s))?;
        Ok(Token::Integer(n))
    }

    fn read_string(&mut self) -> Result<Token, String> {
        self.pos += 1; // skip opening "
        let mut s = String::new();
        while self.pos < self.chars.len() && self.chars[self.pos] != '"' {
            if self.chars[self.pos] == '\\' {
                self.pos += 1;
                if self.pos >= self.chars.len() {
                    return Err("Unterminated string escape".to_string());
                }
                match self.chars[self.pos] {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    '\\' => s.push('\\'),
                    '"' => s.push('"'),
                    c => { s.push('\\'); s.push(c); }
                }
            } else {
                s.push(self.chars[self.pos]);
            }
            self.pos += 1;
        }
        if self.pos >= self.chars.len() {
            return Err("Unterminated string".to_string());
        }
        self.pos += 1; // skip closing "
        Ok(Token::StringLit(s))
    }
}

fn is_symbol_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_' || ch == '-' || ch == '?' || ch == '!'
}

fn is_symbol_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '?' || ch == '!'
}

fn is_operator_char(ch: char) -> bool {
    matches!(ch, '+' | '-' | '*' | '/' | '%' | '<' | '>' | '=' | '&' | '|' | '~' | '^')
}
