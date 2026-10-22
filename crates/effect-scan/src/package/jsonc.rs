//! JSONC with the error contract of `jsonc-parser`'s `parseTree(text, errors,
//! { allowTrailingComma: true, disallowComments: false })`: either a node tree whose nodes carry
//! UTF-16 offsets, or the *first* parse error (code name + UTF-16 token offset). The scanner and
//! the recursive-descent order are those of jsonc-parser, so the first error is the same one.
//! Text is processed as UTF-16 units because offsets are observable (line numbers).
use serde_json::{Map, Number, Value};

#[derive(Debug, Clone)]
pub enum Kind {
    Object(Vec<(String, Node)>),
    Array(Vec<Node>),
    Literal(Value),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub offset: usize,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub code: &'static str,
    pub offset: usize,
}

impl Node {
    pub fn value(&self) -> Value {
        match &self.kind {
            Kind::Literal(v) => v.clone(),
            Kind::Array(items) => Value::Array(items.iter().map(Node::value).collect()),
            Kind::Object(props) => {
                let mut map = Map::new();
                for (key, node) in props {
                    map.insert(key.clone(), node.value());
                }
                Value::Object(map)
            }
        }
    }

    pub fn at(&self, path: &[Segment]) -> Option<&Node> {
        let mut node = self;
        for segment in path {
            node = match (segment, &node.kind) {
                (Segment::Key(key), Kind::Object(props)) => &props.iter().find(|(k, _)| k == key)?.1,
                (Segment::Index(i), Kind::Array(items)) => items.get(*i)?,
                _ => return None,
            };
        }
        Some(node)
    }
}

#[derive(Debug, Clone)]
pub enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Comma,
    Colon,
    Null,
    True,
    False,
    String,
    Number,
    LineComment,
    BlockComment,
    Trivia,
    LineBreak,
    Unknown,
    Eof,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanError {
    None,
    UnexpectedEndOfComment,
    UnexpectedEndOfString,
    UnexpectedEndOfNumber,
    InvalidUnicode,
    InvalidEscapeCharacter,
    InvalidCharacter,
}

struct Scanner {
    text: Vec<u16>,
    pos: usize,
    value: Vec<u16>,
    token_offset: usize,
    token: Token,
    error: ScanError,
}

fn is_digit(c: i32) -> bool {
    (48..=57).contains(&c)
}

fn is_line_break(c: i32) -> bool {
    c == 10 || c == 13
}

impl Scanner {
    fn ch(&self, at: usize) -> i32 {
        self.text.get(at).map_or(-1, |c| *c as i32)
    }

    fn hex_digits(&mut self, count: usize) -> i32 {
        let mut digits = 0;
        let mut value = 0i32;
        while digits < count {
            let c = self.ch(self.pos);
            let d = match c {
                48..=57 => c - 48,
                65..=70 => c - 65 + 10,
                97..=102 => c - 97 + 10,
                _ => break,
            };
            value = value * 16 + d;
            self.pos += 1;
            digits += 1;
        }
        if digits < count {
            -1
        } else {
            value
        }
    }

    fn scan_number(&mut self) -> Vec<u16> {
        let start = self.pos;
        if self.ch(self.pos) == 48 {
            self.pos += 1;
        } else {
            self.pos += 1;
            while is_digit(self.ch(self.pos)) {
                self.pos += 1;
            }
        }
        if self.ch(self.pos) == 46 {
            self.pos += 1;
            if is_digit(self.ch(self.pos)) {
                while is_digit(self.ch(self.pos)) {
                    self.pos += 1;
                }
            } else {
                self.error = ScanError::UnexpectedEndOfNumber;
                return self.text[start..self.pos].to_vec();
            }
        }
        let mut end = self.pos;
        if self.ch(self.pos) == 69 || self.ch(self.pos) == 101 {
            self.pos += 1;
            if self.ch(self.pos) == 43 || self.ch(self.pos) == 45 {
                self.pos += 1;
            }
            if is_digit(self.ch(self.pos)) {
                while is_digit(self.ch(self.pos)) {
                    self.pos += 1;
                }
                end = self.pos;
            } else {
                self.error = ScanError::UnexpectedEndOfNumber;
            }
        }
        self.text[start..end].to_vec()
    }

    fn scan_string(&mut self) -> Vec<u16> {
        let mut result = Vec::new();
        let mut start = self.pos;
        let len = self.text.len();
        loop {
            if self.pos >= len {
                result.extend_from_slice(&self.text[start..self.pos]);
                self.error = ScanError::UnexpectedEndOfString;
                break;
            }
            let c = self.ch(self.pos);
            if c == 34 {
                result.extend_from_slice(&self.text[start..self.pos]);
                self.pos += 1;
                break;
            }
            if c == 92 {
                result.extend_from_slice(&self.text[start..self.pos]);
                self.pos += 1;
                if self.pos >= len {
                    self.error = ScanError::UnexpectedEndOfString;
                    break;
                }
                let c2 = self.ch(self.pos);
                self.pos += 1;
                match c2 {
                    34 => result.push(34),
                    92 => result.push(92),
                    47 => result.push(47),
                    98 => result.push(8),
                    102 => result.push(12),
                    110 => result.push(10),
                    114 => result.push(13),
                    116 => result.push(9),
                    117 => {
                        let c3 = self.hex_digits(4);
                        if c3 >= 0 {
                            result.push(c3 as u16);
                        } else {
                            self.error = ScanError::InvalidUnicode;
                        }
                    }
                    _ => self.error = ScanError::InvalidEscapeCharacter,
                }
                start = self.pos;
                continue;
            }
            if (0..=0x1f).contains(&c) {
                if is_line_break(c) {
                    result.extend_from_slice(&self.text[start..self.pos]);
                    self.error = ScanError::UnexpectedEndOfString;
                    break;
                }
                self.error = ScanError::InvalidCharacter;
            }
            self.pos += 1;
        }
        result
    }

    fn unknown_content(c: i32) -> bool {
        !matches!(c, 32 | 9 | 10 | 13 | 125 | 93 | 123 | 91 | 34 | 58 | 44 | 47)
    }

    fn scan(&mut self) -> Token {
        self.value.clear();
        self.error = ScanError::None;
        self.token_offset = self.pos;
        let len = self.text.len();
        if self.pos >= len {
            self.token_offset = len;
            return self.set(Token::Eof);
        }
        let mut code = self.ch(self.pos);
        if code == 32 || code == 9 {
            while code == 32 || code == 9 {
                self.pos += 1;
                code = self.ch(self.pos);
            }
            return self.set(Token::Trivia);
        }
        if is_line_break(code) {
            self.pos += 1;
            if code == 13 && self.ch(self.pos) == 10 {
                self.pos += 1;
            }
            return self.set(Token::LineBreak);
        }
        let single = |s: &mut Scanner, t: Token| {
            s.pos += 1;
            s.set(t)
        };
        match code {
            123 => single(self, Token::OpenBrace),
            125 => single(self, Token::CloseBrace),
            91 => single(self, Token::OpenBracket),
            93 => single(self, Token::CloseBracket),
            58 => single(self, Token::Colon),
            44 => single(self, Token::Comma),
            34 => {
                self.pos += 1;
                self.value = self.scan_string();
                self.set(Token::String)
            }
            47 => {
                let start = self.pos;
                self.pos += 1;
                if self.ch(self.pos) == 47 {
                    self.pos += 1;
                    while self.pos < len && !is_line_break(self.ch(self.pos)) {
                        self.pos += 1;
                    }
                    return self.set(Token::LineComment);
                }
                if self.ch(self.pos) == 42 {
                    self.pos += 1;
                    let safe = len.saturating_sub(1);
                    let mut closed = false;
                    while self.pos < safe {
                        let c = self.ch(self.pos);
                        if c == 42 && self.ch(self.pos + 1) == 47 {
                            self.pos += 2;
                            closed = true;
                            break;
                        }
                        self.pos += 1;
                        if is_line_break(c) && c == 13 && self.ch(self.pos) == 10 {
                            self.pos += 1;
                        }
                    }
                    if !closed {
                        self.pos += 1;
                        self.error = ScanError::UnexpectedEndOfComment;
                    }
                    let _ = start;
                    return self.set(Token::BlockComment);
                }
                self.set(Token::Unknown)
            }
            45 | 48..=57 => {
                let mut prefix = Vec::new();
                if code == 45 {
                    prefix.push(45u16);
                    self.pos += 1;
                    if self.pos == len || !is_digit(self.ch(self.pos)) {
                        self.value = prefix;
                        return self.set(Token::Unknown);
                    }
                }
                let number = self.scan_number();
                prefix.extend(number);
                self.value = prefix;
                self.set(Token::Number)
            }
            _ => {
                while self.pos < len && Self::unknown_content(code) {
                    self.pos += 1;
                    code = self.ch(self.pos);
                }
                if self.token_offset != self.pos {
                    let text = String::from_utf16_lossy(&self.text[self.token_offset..self.pos]);
                    return self.set(match text.as_str() {
                        "true" => Token::True,
                        "false" => Token::False,
                        "null" => Token::Null,
                        _ => Token::Unknown,
                    });
                }
                self.pos += 1;
                self.set(Token::Unknown)
            }
        }
    }

    fn set(&mut self, token: Token) -> Token {
        self.token = token;
        token
    }
}

/// Containers nest recursively in the parser, in `Node::value` and in drop; a bound keeps hostile input
/// (a config of `[[[[...`) an error instead of a stack overflow. Real wrangler configs nest < 10 deep.
const MAX_DEPTH: usize = 256;

struct Parser {
    scanner: Scanner,
    depth: usize,
}

type Res<T> = Result<T, ParseError>;

impl Parser {
    fn fail<T>(&self, code: &'static str) -> Res<T> {
        Err(ParseError { code, offset: self.scanner.token_offset })
    }

    fn scan_next(&mut self) -> Res<Token> {
        loop {
            let token = self.scanner.scan();
            match self.scanner.error {
                ScanError::InvalidUnicode => return self.fail("InvalidUnicode"),
                ScanError::InvalidEscapeCharacter => return self.fail("InvalidEscapeCharacter"),
                ScanError::UnexpectedEndOfNumber => return self.fail("UnexpectedEndOfNumber"),
                ScanError::UnexpectedEndOfComment => return self.fail("UnexpectedEndOfComment"),
                ScanError::UnexpectedEndOfString => return self.fail("UnexpectedEndOfString"),
                ScanError::InvalidCharacter => return self.fail("InvalidCharacter"),
                ScanError::None => {}
            }
            match token {
                Token::LineComment | Token::BlockComment | Token::Trivia | Token::LineBreak => {}
                Token::Unknown => return self.fail("InvalidSymbol"),
                _ => return Ok(token),
            }
        }
    }

    /// `parseLiteral`/`parseString`/`parseArray`/`parseObject`: Ok(None) when the current token starts no value.
    fn parse_value(&mut self) -> Res<Option<Node>> {
        let offset = self.scanner.token_offset;
        let literal = |value: Value| Node { offset, kind: Kind::Literal(value) };
        let node = match self.scanner.token {
            Token::OpenBracket | Token::OpenBrace => {
                if self.depth >= MAX_DEPTH {
                    return self.fail("MaxDepthExceeded");
                }
                self.depth += 1;
                let node = if self.scanner.token == Token::OpenBracket { self.parse_array() } else { self.parse_object() };
                self.depth -= 1;
                return node.map(Some);
            }
            Token::String => literal(Value::String(String::from_utf16_lossy(&self.scanner.value))),
            Token::Number => {
                let text = String::from_utf16_lossy(&self.scanner.value);
                let Ok(n) = text.parse::<f64>() else { return self.fail("InvalidNumberFormat") };
                literal(json_number(n))
            }
            Token::Null => literal(Value::Null),
            Token::True => literal(Value::Bool(true)),
            Token::False => literal(Value::Bool(false)),
            _ => return Ok(None),
        };
        self.scan_next()?;
        Ok(Some(node))
    }

    fn parse_object(&mut self) -> Res<Node> {
        let offset = self.scanner.token_offset;
        self.scan_next()?;
        let mut props: Vec<(String, Node)> = Vec::new();
        let mut needs_comma = false;
        while !matches!(self.scanner.token, Token::CloseBrace | Token::Eof) {
            if self.scanner.token == Token::Comma {
                if !needs_comma {
                    return self.fail("ValueExpected");
                }
                self.scan_next()?;
                if self.scanner.token == Token::CloseBrace {
                    break;
                }
            } else if needs_comma {
                return self.fail("CommaExpected");
            }
            if self.scanner.token != Token::String {
                return self.fail("PropertyNameExpected");
            }
            let key = String::from_utf16_lossy(&self.scanner.value);
            self.scan_next()?;
            if self.scanner.token != Token::Colon {
                return self.fail("ColonExpected");
            }
            self.scan_next()?;
            let Some(value) = self.parse_value()? else { return self.fail("ValueExpected") };
            props.push((key, value));
            needs_comma = true;
        }
        if self.scanner.token != Token::CloseBrace {
            return self.fail("CloseBraceExpected");
        }
        self.scan_next()?;
        Ok(Node { offset, kind: Kind::Object(props) })
    }

    fn parse_array(&mut self) -> Res<Node> {
        let offset = self.scanner.token_offset;
        self.scan_next()?;
        let mut items = Vec::new();
        let mut needs_comma = false;
        while !matches!(self.scanner.token, Token::CloseBracket | Token::Eof) {
            if self.scanner.token == Token::Comma {
                if !needs_comma {
                    return self.fail("ValueExpected");
                }
                self.scan_next()?;
                if self.scanner.token == Token::CloseBracket {
                    break;
                }
            } else if needs_comma {
                return self.fail("CommaExpected");
            }
            let Some(value) = self.parse_value()? else { return self.fail("ValueExpected") };
            items.push(value);
            needs_comma = true;
        }
        if self.scanner.token != Token::CloseBracket {
            return self.fail("CloseBracketExpected");
        }
        self.scan_next()?;
        Ok(Node { offset, kind: Kind::Array(items) })
    }
}

fn json_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::Number(Number::from(n as i64))
    } else {
        Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

pub fn parse_tree(text: &str) -> Result<Node, ParseError> {
    let mut parser = Parser {
        depth: 0,
        scanner: Scanner { text: text.encode_utf16().collect(), pos: 0, value: Vec::new(), token_offset: 0, token: Token::Unknown, error: ScanError::None },
    };
    parser.scan_next()?;
    if parser.scanner.token == Token::Eof {
        return parser.fail("ValueExpected");
    }
    let Some(root) = parser.parse_value()? else { return parser.fail("ValueExpected") };
    if parser.scanner.token != Token::Eof {
        return parser.fail("EndOfFileExpected");
    }
    Ok(root)
}

/// 1-based line of a UTF-16 offset (`lineForOffset`: count of `\n` in the first `offset` units).
pub fn line_for_offset(text: &str, offset: usize) -> u32 {
    1 + text.encode_utf16().take(offset).filter(|c| *c == 10).count() as u32
}
