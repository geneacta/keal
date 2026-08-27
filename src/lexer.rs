//! Hand-written lexer.
//!
//! Newlines are not tokens. Instead we do Go-style automatic semicolon
//! insertion: a newline becomes a `;` when the previous token is one that can
//! legally end a statement. That keeps the grammar newline-insensitive while
//! letting users omit semicolons.

use crate::span::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(Vec<StrPart>),
    Ident(String),

    // keywords
    Val,
    Var,
    Fun,
    /// Declares a procedure: something run for its effect, which returns
    /// nothing. `fun` is the counterpart that must return a value.
    Proc,
    Return,
    If,
    /// `unless (c)` is `if (not c)`, and takes an `else` just the same.
    Unless,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Class,
    This,
    Null,
    True,
    False,
    When,
    Is,
    Import,

    // The logical connectives. These are the spelling the language
    // recommends; `&&`, `||`, `!` and `^` are accepted as aliases.
    KwNot,
    KwAnd,
    KwOr,
    KwXor,
    KwXnor,
    KwNand,
    KwNor,
    KwImplies,

    // operators & punctuation
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    EqEq,
    Bang,
    BangEq,
    /// `!!` — asserts that a nullable value is not null.
    BangBang,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    Caret,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    /// `++` and `--`, the statement-level increment and decrement.
    PlusPlus,
    MinusMinus,
    /// `**` — power — and `^/` — root — with their compound assignments.
    StarStar,
    StarStarEq,
    RootOp,
    RootEq,
    Dot,
    SafeDot,
    Elvis,
    Question,
    Comma,
    Colon,
    Semi,
    Arrow,
    DotDot,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    Eof,
}

/// One piece of a string literal: either literal text or an interpolation
/// hole whose source is re-lexed and parsed by the parser.
#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Lit(String),
    Interp(String, Span),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

impl Tok {
    /// True when a newline directly after this token should become a `;`.
    pub fn ends_statement(&self) -> bool {
        matches!(
            self,
            Tok::Int(_)
                | Tok::Float(_)
                | Tok::Str(_)
                | Tok::Ident(_)
                | Tok::Return
                | Tok::Break
                | Tok::Continue
                | Tok::Null
                | Tok::True
                | Tok::False
                | Tok::This
                | Tok::RParen
                | Tok::RBrace
                | Tok::RBracket
                | Tok::Question
                | Tok::BangBang
                | Tok::PlusPlus
                | Tok::MinusMinus
        )
    }

    pub fn describe(&self) -> String {
        match self {
            Tok::Int(n) => format!("integer `{}`", n),
            Tok::Float(n) => format!("float `{}`", n),
            Tok::Str(_) => "string literal".to_string(),
            Tok::Ident(s) => format!("`{}`", s),
            Tok::Eof => "end of file".to_string(),
            Tok::Semi => "end of statement".to_string(),
            other => format!("`{}`", other.symbol()),
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Tok::Val => "val",
            Tok::Var => "var",
            Tok::Fun => "fun",
            Tok::Proc => "proc",
            Tok::Return => "return",
            Tok::If => "if",
            Tok::Unless => "unless",
            Tok::Else => "else",
            Tok::While => "while",
            Tok::For => "for",
            Tok::In => "in",
            Tok::Break => "break",
            Tok::Continue => "continue",
            Tok::Class => "class",
            Tok::This => "this",
            Tok::Null => "null",
            Tok::True => "true",
            Tok::False => "false",
            Tok::When => "when",
            Tok::Is => "is",
            Tok::Import => "import",
            Tok::KwNot => "not",
            Tok::KwAnd => "and",
            Tok::KwOr => "or",
            Tok::KwXor => "xor",
            Tok::KwXnor => "xnor",
            Tok::KwNand => "nand",
            Tok::KwNor => "nor",
            Tok::KwImplies => "implies",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Assign => "=",
            Tok::EqEq => "==",
            Tok::Bang => "!",
            Tok::BangEq => "!=",
            Tok::BangBang => "!!",
            Tok::Lt => "<",
            Tok::LtEq => "<=",
            Tok::Gt => ">",
            Tok::GtEq => ">=",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::Caret => "^",
            Tok::PlusEq => "+=",
            Tok::PlusPlus => "++",
            Tok::MinusMinus => "--",
            Tok::StarStar => "**",
            Tok::StarStarEq => "**=",
            Tok::RootOp => "^/",
            Tok::RootEq => "^/=",
            Tok::MinusEq => "-=",
            Tok::StarEq => "*=",
            Tok::SlashEq => "/=",
            Tok::PercentEq => "%=",
            Tok::Dot => ".",
            Tok::SafeDot => "?.",
            Tok::Elvis => "?:",
            Tok::Question => "?",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Semi => ";",
            Tok::Arrow => "->",
            Tok::DotDot => "..",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            _ => "?",
        }
    }
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
    file: u32,
    out: Vec<Token>,
    /// True for a whole file, where a leading `#!` line is the shebang and
    /// not part of the program. False for an interpolation fragment.
    allow_shebang: bool,
    /// Open brackets, innermost last. A newline inside `(` or `[` never
    /// becomes a `;`, so call arguments and list literals may span lines.
    /// A `{` re-enables insertion, so lambda bodies still work normally.
    brackets: Vec<u8>,
}

pub fn lex(src: &str, file: u32) -> Result<Vec<Token>, Diag> {
    Lexer {
        src: src.as_bytes(),
        pos: 0,
        line: 1,
        col: 1,
        file,
        out: Vec::new(),
        allow_shebang: true,
        brackets: Vec::new(),
    }
    .run()
}

/// Lexes an interpolation hole, whose text starts at `span` in the outer file.
pub fn lex_fragment(src: &str, span: Span) -> Result<Vec<Token>, Diag> {
    Lexer {
        src: src.as_bytes(),
        pos: 0,
        line: span.line,
        col: span.col,
        file: span.file,
        out: Vec::new(),
        allow_shebang: false,
        brackets: Vec::new(),
    }
    .run()
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> u8 {
        *self.src.get(self.pos).unwrap_or(&0)
    }

    fn peek2(&self) -> u8 {
        *self.src.get(self.pos + 1).unwrap_or(&0)
    }

    fn bump(&mut self) -> u8 {
        let c = self.peek();
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn span(&self) -> Span {
        Span::new(self.file, self.line, self.col)
    }

    fn push(&mut self, tok: Tok, span: Span) {
        self.out.push(Token { tok, span });
    }

    /// Emits a virtual `;` if the last real token can end a statement and we
    /// are not inside a parenthesised or bracketed group.
    fn maybe_semi(&mut self) {
        if matches!(self.brackets.last(), Some(b'(') | Some(b'[')) {
            return;
        }
        let insert = match self.out.last() {
            Some(t) => t.tok.ends_statement(),
            None => false,
        };
        if insert {
            let span = self.span();
            self.push(Tok::Semi, span);
        }
    }

    fn err(&self, span: Span, msg: impl Into<String>) -> Diag {
        Diag::new(span, msg)
    }

    fn run(mut self) -> Result<Vec<Token>, Diag> {
        // `#!/usr/bin/env keal` on the first line makes a script executable.
        // The line is skipped rather than removed, so every span still points
        // where the file says it does.
        if self.allow_shebang && self.src.starts_with(b"#!") {
            while self.peek() != b'\n' && self.peek() != 0 {
                self.bump();
            }
        }
        loop {
            // Whitespace and comments, tracking newlines for semicolon insertion.
            loop {
                match self.peek() {
                    b' ' | b'\t' | b'\r' => {
                        self.bump();
                    }
                    b'\n' => {
                        self.bump();
                        self.maybe_semi();
                    }
                    b'/' if self.peek2() == b'/' => {
                        while self.peek() != b'\n' && self.peek() != 0 {
                            self.bump();
                        }
                    }
                    b'/' if self.peek2() == b'*' => {
                        let start = self.span();
                        self.bump();
                        self.bump();
                        let mut depth = 1;
                        while depth > 0 {
                            match self.peek() {
                                0 => return Err(self.err(start, "unterminated block comment")),
                                b'/' if self.peek2() == b'*' => {
                                    self.bump();
                                    self.bump();
                                    depth += 1;
                                }
                                b'*' if self.peek2() == b'/' => {
                                    self.bump();
                                    self.bump();
                                    depth -= 1;
                                }
                                _ => {
                                    self.bump();
                                }
                            }
                        }
                    }
                    _ => break,
                }
            }

            let span = self.span();
            let c = self.peek();
            if c == 0 {
                self.maybe_semi();
                self.push(Tok::Eof, span);
                return Ok(self.out);
            }

            if c.is_ascii_digit() {
                self.number(span)?;
                continue;
            }
            if c.is_ascii_alphabetic() || c == b'_' {
                self.word(span);
                continue;
            }
            if c == b'"' {
                // `"""` opens a raw string: newlines welcome, no escapes,
                // no interpolation — text meant to be passed through whole,
                // like the C in a `native` block.
                if self.peek2() == b'"' && *self.src.get(self.pos + 2).unwrap_or(&0) == b'"' {
                    let text = self.raw_string(span)?;
                    self.push(Tok::Str(vec![StrPart::Lit(text)]), span);
                    continue;
                }
                let parts = self.string(span)?;
                self.push(Tok::Str(parts), span);
                continue;
            }

            self.bump();
            let tok = match c {
                b'+' => {
                    if self.peek() == b'+' {
                        self.bump();
                        Tok::PlusPlus
                    } else {
                        self.pick(b'=', Tok::PlusEq, Tok::Plus)
                    }
                }
                b'-' => {
                    if self.peek() == b'=' {
                        self.bump();
                        Tok::MinusEq
                    } else if self.peek() == b'>' {
                        self.bump();
                        Tok::Arrow
                    } else if self.peek() == b'-' {
                        self.bump();
                        Tok::MinusMinus
                    } else {
                        Tok::Minus
                    }
                }
                b'*' => {
                    if self.peek() == b'*' {
                        self.bump();
                        self.pick(b'=', Tok::StarStarEq, Tok::StarStar)
                    } else {
                        self.pick(b'=', Tok::StarEq, Tok::Star)
                    }
                }
                b'/' => self.pick(b'=', Tok::SlashEq, Tok::Slash),
                b'%' => self.pick(b'=', Tok::PercentEq, Tok::Percent),
                b'=' => self.pick(b'=', Tok::EqEq, Tok::Assign),
                b'!' => {
                    if self.peek() == b'=' {
                        self.bump();
                        Tok::BangEq
                    } else if self.peek() == b'!' {
                        self.bump();
                        Tok::BangBang
                    } else {
                        Tok::Bang
                    }
                }
                b'<' => self.pick(b'=', Tok::LtEq, Tok::Lt),
                b'>' => self.pick(b'=', Tok::GtEq, Tok::Gt),
                b'&' => {
                    if self.peek() == b'&' {
                        self.bump();
                        Tok::AndAnd
                    } else {
                        return Err(self.err(span, "unexpected `&` (did you mean `&&`?)"));
                    }
                }
                b'|' => {
                    if self.peek() == b'|' {
                        self.bump();
                        Tok::OrOr
                    } else {
                        return Err(self.err(span, "unexpected `|` (did you mean `||`?)"));
                    }
                }
                b'?' => {
                    if self.peek() == b'.' {
                        self.bump();
                        Tok::SafeDot
                    } else if self.peek() == b':' {
                        self.bump();
                        Tok::Elvis
                    } else {
                        Tok::Question
                    }
                }
                b'^' => {
                    if self.peek() == b'/' {
                        self.bump();
                        self.pick(b'=', Tok::RootEq, Tok::RootOp)
                    } else {
                        Tok::Caret
                    }
                }
                b'.' => self.pick(b'.', Tok::DotDot, Tok::Dot),
                b',' => Tok::Comma,
                b':' => Tok::Colon,
                b';' => Tok::Semi,
                b'(' | b'{' | b'[' => {
                    self.brackets.push(c);
                    match c {
                        b'(' => Tok::LParen,
                        b'{' => Tok::LBrace,
                        _ => Tok::LBracket,
                    }
                }
                b')' | b'}' | b']' => {
                    self.brackets.pop();
                    match c {
                        b')' => Tok::RParen,
                        b'}' => Tok::RBrace,
                        _ => Tok::RBracket,
                    }
                }
                other => {
                    return Err(self.err(span, format!("unexpected character `{}`", other as char)))
                }
            };
            self.push(tok, span);
        }
    }

    fn pick(&mut self, next: u8, yes: Tok, no: Tok) -> Tok {
        if self.peek() == next {
            self.bump();
            yes
        } else {
            no
        }
    }

    fn number(&mut self, span: Span) -> Result<(), Diag> {
        let start = self.pos;
        while self.peek().is_ascii_digit() || self.peek() == b'_' {
            self.bump();
        }
        // `1..10` is a range, not `1.` followed by `.10`.
        let is_float = self.peek() == b'.' && self.peek2().is_ascii_digit();
        if is_float {
            self.bump();
            while self.peek().is_ascii_digit() || self.peek() == b'_' {
                self.bump();
            }
        }
        if matches!(self.peek(), b'e' | b'E') {
            let save = self.pos;
            self.bump();
            if matches!(self.peek(), b'+' | b'-') {
                self.bump();
            }
            if self.peek().is_ascii_digit() {
                while self.peek().is_ascii_digit() {
                    self.bump();
                }
                let text: String =
                    String::from_utf8_lossy(&self.src[start..self.pos]).replace('_', "");
                let n: f64 = text
                    .parse()
                    .map_err(|_| self.err(span, format!("invalid float literal `{}`", text)))?;
                self.push(Tok::Float(n), span);
                return Ok(());
            }
            self.pos = save; // not an exponent after all
        }

        let text: String = String::from_utf8_lossy(&self.src[start..self.pos]).replace('_', "");
        if is_float {
            let n: f64 = text
                .parse()
                .map_err(|_| self.err(span, format!("invalid float literal `{}`", text)))?;
            self.push(Tok::Float(n), span);
        } else {
            let n: i64 = text.parse().map_err(|_| {
                self.err(span, format!("integer literal `{}` does not fit in Int", text))
            })?;
            self.push(Tok::Int(n), span);
        }
        Ok(())
    }

    fn word(&mut self, span: Span) {
        let start = self.pos;
        while self.peek().is_ascii_alphanumeric() || self.peek() == b'_' {
            self.bump();
        }
        let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        let tok = match text.as_str() {
            "val" => Tok::Val,
            "var" => Tok::Var,
            "fun" => Tok::Fun,
            "proc" => Tok::Proc,
            "return" => Tok::Return,
            "if" => Tok::If,
            "unless" => Tok::Unless,
            "else" => Tok::Else,
            "while" => Tok::While,
            "for" => Tok::For,
            "in" => Tok::In,
            "break" => Tok::Break,
            "continue" => Tok::Continue,
            "class" => Tok::Class,
            "this" => Tok::This,
            "null" => Tok::Null,
            "true" => Tok::True,
            "false" => Tok::False,
            "when" => Tok::When,
            "is" => Tok::Is,
            "import" => Tok::Import,
            "not" => Tok::KwNot,
            "and" => Tok::KwAnd,
            "or" => Tok::KwOr,
            "xor" => Tok::KwXor,
            "xnor" => Tok::KwXnor,
            "nand" => Tok::KwNand,
            "nor" => Tok::KwNor,
            "implies" => Tok::KwImplies,
            _ => Tok::Ident(text),
        };
        self.push(tok, span);
    }

    fn raw_string(&mut self, span: Span) -> Result<String, Diag> {
        self.bump();
        self.bump();
        self.bump();
        let start = self.pos;
        loop {
            if self.peek() == 0 {
                return Err(self.err(span, "unterminated raw string"));
            }
            if self.peek() == b'"'
                && self.peek2() == b'"'
                && *self.src.get(self.pos + 2).unwrap_or(&0) == b'"'
            {
                let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                self.bump();
                self.bump();
                self.bump();
                return Ok(text);
            }
            self.bump();
        }
    }

    fn string(&mut self, span: Span) -> Result<Vec<StrPart>, Diag> {
        self.bump(); // opening quote
        let mut parts: Vec<StrPart> = Vec::new();
        let mut buf = String::new();
        loop {
            match self.peek() {
                0 | b'\n' => return Err(self.err(span, "unterminated string literal")),
                b'"' => {
                    self.bump();
                    break;
                }
                b'\\' => {
                    self.bump();
                    let esc_span = self.span();
                    let e = self.bump();
                    buf.push(match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'0' => '\0',
                        b'\\' => '\\',
                        b'"' => '"',
                        b'$' => '$',
                        b'u' => {
                            if self.peek() != b'{' {
                                return Err(self.err(esc_span, "expected `{` after `\\u`"));
                            }
                            self.bump();
                            let mut hex = String::new();
                            while self.peek() != b'}' {
                                if self.peek() == 0 {
                                    return Err(self.err(esc_span, "unterminated `\\u{...}` escape"));
                                }
                                hex.push(self.bump() as char);
                            }
                            self.bump();
                            let code = u32::from_str_radix(&hex, 16)
                                .map_err(|_| self.err(esc_span, "invalid hex in `\\u{...}`"))?;
                            char::from_u32(code)
                                .ok_or_else(|| self.err(esc_span, "invalid unicode scalar value"))?
                        }
                        other => {
                            return Err(self
                                .err(esc_span, format!("unknown escape `\\{}`", other as char)))
                        }
                    });
                }
                b'$' => {
                    let dollar = self.span();
                    self.bump();
                    if self.peek() == b'{' {
                        self.bump();
                        let inner_span = self.span();
                        let start = self.pos;
                        let mut depth = 1;
                        loop {
                            match self.peek() {
                                0 => return Err(self.err(dollar, "unterminated `${...}`")),
                                b'{' => {
                                    depth += 1;
                                    self.bump();
                                }
                                b'}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                    self.bump();
                                }
                                _ => {
                                    self.bump();
                                }
                            }
                        }
                        let text =
                            String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                        self.bump(); // closing }
                        if !buf.is_empty() {
                            parts.push(StrPart::Lit(std::mem::take(&mut buf)));
                        }
                        parts.push(StrPart::Interp(text, inner_span));
                    } else if self.peek().is_ascii_alphabetic() || self.peek() == b'_' {
                        let inner_span = self.span();
                        let start = self.pos;
                        while self.peek().is_ascii_alphanumeric() || self.peek() == b'_' {
                            self.bump();
                        }
                        let text =
                            String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                        if !buf.is_empty() {
                            parts.push(StrPart::Lit(std::mem::take(&mut buf)));
                        }
                        parts.push(StrPart::Interp(text, inner_span));
                    } else {
                        buf.push('$');
                    }
                }
                _ => {
                    // Copy one whole UTF-8 sequence so multibyte chars survive.
                    let start = self.pos;
                    let len = utf8_len(self.peek());
                    for _ in 0..len {
                        self.bump();
                    }
                    buf.push_str(&String::from_utf8_lossy(&self.src[start..self.pos]));
                }
            }
        }
        if !buf.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(buf));
        }
        Ok(parts)
    }
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}
