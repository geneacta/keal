//! Just enough JSON for the language server to speak the protocol.
//!
//! Keal has no dependencies and this is not the place to acquire the first
//! one: what the Language Server Protocol needs is a value tree, a parser
//! and a writer, and all three fit in a page. Nothing here is a general
//! JSON library and it does not pretend to be — it reads what an editor
//! sends and writes what an editor reads.

use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Ordered, so that what this writes is stable from one run to the next
    /// — which is what makes a transcript worth diffing.
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    pub fn int(n: i64) -> Json {
        Json::Num(n as f64)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Num(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    /// `a.b.c`, for reaching into a request without four `match`es.
    pub fn at(&self, path: &str) -> Option<&Json> {
        let mut here = self;
        for part in path.split('.') {
            here = here.get(part)?;
        }
        Some(here)
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                // A whole number is written without a fraction: the protocol
                // is full of counts and offsets, and `3.0` where an editor
                // expects `3` reads as a mistake even where it is not.
                if n.fract() == 0.0 && n.is_finite() && n.abs() < 9e15 {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{}", n);
                }
            }
            Json::Str(s) => write_string(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn to_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Everything below a space has to be escaped; everything above
            // goes out as UTF-8, which is what the protocol asks for.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn parse(text: &str) -> Option<Json> {
    let bytes: Vec<char> = text.chars().collect();
    let mut p = Parser { s: &bytes, i: 0 };
    p.ws();
    let v = p.value()?;
    Some(v)
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> char {
        *self.s.get(self.i).unwrap_or(&'\0')
    }

    fn ws(&mut self) {
        while matches!(self.peek(), ' ' | '\t' | '\n' | '\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == c {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match self.peek() {
            '{' => self.object(),
            '[' => self.array(),
            '"' => Some(Json::Str(self.string()?)),
            't' => self.word("true", Json::Bool(true)),
            'f' => self.word("false", Json::Bool(false)),
            'n' => self.word("null", Json::Null),
            _ => self.number(),
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Option<Json> {
        for c in w.chars() {
            if !self.eat(c) {
                return None;
            }
        }
        Some(v)
    }

    fn object(&mut self) -> Option<Json> {
        self.i += 1;
        let mut map = BTreeMap::new();
        self.ws();
        if self.eat('}') {
            return Some(Json::Obj(map));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            if !self.eat(':') {
                return None;
            }
            let value = self.value()?;
            map.insert(key, value);
            self.ws();
            if self.eat(',') {
                continue;
            }
            return self.eat('}').then_some(Json::Obj(map));
        }
    }

    fn array(&mut self) -> Option<Json> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat(']') {
            return Some(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            if self.eat(',') {
                continue;
            }
            return self.eat(']').then_some(Json::Arr(items));
        }
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat('"') {
            return None;
        }
        let mut out = String::new();
        loop {
            let c = self.peek();
            self.i += 1;
            match c {
                '"' => return Some(out),
                '\0' => return None,
                '\\' => {
                    let e = self.peek();
                    self.i += 1;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let hi = self.hex4()?;
                            // A surrogate pair is two escapes, and an editor
                            // sending an emoji in a document name will send
                            // exactly that.
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                if !(self.eat('\\') && self.eat('u')) {
                                    return None;
                                }
                                let lo = self.hex4()?;
                                let combined =
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                char::from_u32(combined)?
                            } else {
                                char::from_u32(hi)?
                            };
                            out.push(ch);
                        }
                        _ => return None,
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.peek();
            self.i += 1;
            v = v * 16 + c.to_digit(16)?;
        }
        Some(v)
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.i;
        if self.peek() == '-' {
            self.i += 1;
        }
        while matches!(self.peek(), '0'..='9' | '.' | 'e' | 'E' | '+' | '-') {
            self.i += 1;
        }
        if start == self.i {
            return None;
        }
        let text: String = self.s[start..self.i].iter().collect();
        text.parse::<f64>().ok().map(Json::Num)
    }
}
