//! A JSON parser and compact serializer covering the site-list files.
//! Numbers are kept as their original text so status codes stay exact.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(raw) => {
                if let Ok(v) = raw.parse::<i64>() {
                    return Some(v);
                }
                let v = raw.parse::<f64>().ok()?;
                if v.fract() == 0.0 && v >= i64::MIN as f64 && v <= i64::MAX as f64 {
                    Some(v as i64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(pairs) => Some(pairs),
            _ => None,
        }
    }
}

pub fn parse(input: &str) -> Result<Value, String> {
    let mut p = Parser { s: input, i: 0 };
    let value = p.parse_value()?;
    p.skip();
    if p.i != p.s.len() {
        return Err(format!("trailing data at byte {}", p.i));
    }
    Ok(value)
}

pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(raw) => out.push_str(raw),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, k);
                out.push(':');
                write_value(out, v);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip();
        let c = self.peek().ok_or_else(|| self.err("unexpected end"))?;
        match c {
            'n' => self.consume_lit("null", Value::Null),
            't' => self.consume_lit("true", Value::Bool(true)),
            'f' => self.consume_lit("false", Value::Bool(false)),
            '"' => Ok(Value::String(self.parse_string()?)),
            '[' => self.parse_array(),
            '{' => self.parse_object(),
            '-' | '0'..='9' => self.parse_number(),
            _ => Err(self.err("expected a value")),
        }
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.bump();
        let mut items = Vec::new();
        self.skip();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some(']') => {
                    self.bump();
                    break;
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.bump();
        let mut pairs = Vec::new();
        self.skip();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::Object(pairs));
        }
        loop {
            self.skip();
            if self.peek() != Some('"') {
                return Err(self.err("expected a string key"));
            }
            let key = self.parse_string()?;
            self.skip();
            if self.peek() != Some(':') {
                return Err(self.err("expected ':'"));
            }
            self.bump();
            let value = self.parse_value()?;
            pairs.push((key, value));
            self.skip();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some('}') => {
                    self.bump();
                    break;
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
        Ok(Value::Object(pairs))
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.bump();
        }
        match self.peek() {
            Some('0') => {
                self.bump();
            }
            Some('1'..='9') => {
                while matches!(self.peek(), Some('0'..='9')) {
                    self.bump();
                }
            }
            _ => return Err(self.err("invalid number")),
        }
        if self.peek() == Some('.') {
            self.bump();
            if !matches!(self.peek(), Some('0'..='9')) {
                return Err(self.err("invalid fraction"));
            }
            while matches!(self.peek(), Some('0'..='9')) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if !matches!(self.peek(), Some('0'..='9')) {
                return Err(self.err("invalid exponent"));
            }
            while matches!(self.peek(), Some('0'..='9')) {
                self.bump();
            }
        }
        Ok(Value::Number(self.s[start..self.i].to_string()))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.bump();
        let mut out = String::new();
        loop {
            let c = self.bump().ok_or_else(|| self.err("unterminated string"))?;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.bump().ok_or_else(|| self.err("bad escape"))?;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => out.push(self.parse_unicode_escape()?),
                        _ => return Err(self.err("bad escape")),
                    }
                }
                c if (c as u32) < 0x20 => return Err(self.err("raw control in string")),
                c => out.push(c),
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let unit = self.hex4()?;
        if (0xD800..=0xDBFF).contains(&unit) {
            if self.peek() == Some('\\') {
                let save = self.i;
                self.bump();
                if self.peek() == Some('u') {
                    self.bump();
                    let low = self.hex4()?;
                    if (0xDC00..=0xDFFF).contains(&low) {
                        let cp =
                            0x10000 + (((unit as u32) - 0xD800) << 10) + ((low as u32) - 0xDC00);
                        return char::from_u32(cp).ok_or_else(|| self.err("bad surrogate"));
                    }
                }
                self.i = save;
            }
            return Err(self.err("lone surrogate"));
        }
        char::from_u32(unit as u32).ok_or_else(|| self.err("bad codepoint"))
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let mut n = 0u16;
        for _ in 0..4 {
            let c = self.bump().ok_or_else(|| self.err("short \\u escape"))?;
            let d = c.to_digit(16).ok_or_else(|| self.err("bad hex"))?;
            n = (n << 4) | d as u16;
        }
        Ok(n)
    }

    fn consume_lit(&mut self, lit: &str, value: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            Ok(value)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(' ' | '\n' | '\r' | '\t')) {
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }

    fn err(&self, msg: &str) -> String {
        format!("{msg} at byte {}", self.i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_and_escapes() {
        let v = parse(r#"{"a":[1,true,null,"hi\n\"\u0041"],"b":{"c":-2.5}}"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[0].as_i64(), Some(1));
        assert_eq!(
            v.get("a").unwrap().as_array().unwrap()[3].as_str(),
            Some("hi\n\"A")
        );
        assert_eq!(v.get("b").unwrap().get("c").unwrap().as_i64(), None);
        let again = parse(&stringify(&v)).unwrap();
        assert_eq!(
            again.get("a").unwrap().as_array().unwrap()[3].as_str(),
            Some("hi\n\"A")
        );
    }

    #[test]
    fn surrogate_pair() {
        let v = parse(r#""\uD83D\uDC0D""#).unwrap();
        assert_eq!(v.as_str(), Some("🐍"));
    }
}
