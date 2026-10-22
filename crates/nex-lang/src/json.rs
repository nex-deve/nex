//! JSON reading and writing for the `Json` type.
//!
//! JSON values map onto VM values: objects are maps with `Str` keys,
//! arrays are arrays, numbers are `Int` when they are whole and fit in 64
//! bits (otherwise `Float`), and `null` is `Unit`.

use crate::vm::{Key, Value};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::rc::Rc;

/// Deepest nesting `parse` accepts, so hostile input cannot exhaust the
/// native stack.
pub const MAX_DEPTH: usize = 128;

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("unexpected text after the JSON value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> String {
        format!("json_parse: {msg} at byte {}", self.i)
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting is too deep"));
        }
        match self.s.get(self.i) {
            None => Err(self.err("unexpected end of input")),
            Some(b'{') => {
                self.i += 1;
                let mut map = BTreeMap::new();
                self.ws();
                if self.eat("}") {
                    return Ok(Value::Map(Rc::new(map)));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(self.err("expected a string key"));
                    }
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return Err(self.err("expected `:`"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    map.insert(Key::Str(Rc::from(key)), v);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Ok(Value::Map(Rc::new(map)));
                    }
                    return Err(self.err("expected `,` or `}`"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Ok(Value::Array(Rc::new(items)));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Value::Array(Rc::new(items)));
                    }
                    return Err(self.err("expected `,` or `]`"));
                }
            }
            Some(b'"') => Ok(Value::Str(Rc::from(self.string()?))),
            Some(b't') if self.eat("true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Value::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Value::Unit),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        let mut is_float = false;
        if self.s[self.i] == b'-' {
            self.i += 1;
        }
        let digits = |p: &mut Self| {
            let from = p.i;
            while p.i < p.s.len() && p.s[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i > from
        };
        if !digits(self) {
            return Err(self.err("invalid number"));
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            is_float = true;
            if !digits(self) {
                return Err(self.err("invalid number"));
            }
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            is_float = true;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                return Err(self.err("invalid number"));
            }
        }
        let text = &self.text[start..self.i];
        if !is_float {
            if let Ok(n) = text.parse::<i64>() {
                return Ok(Value::Int(n));
            }
        }
        match text.parse::<f64>() {
            Ok(x) if x.is_finite() => Ok(Value::Float(x)),
            _ => Err(self.err("number is out of range")),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .text
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len() && !matches!(self.s[self.i], b'"' | b'\\' | 0..=0x1f) {
                self.i += 1;
            }
            out.push_str(&self.text[start..self.i]);
            match self.s.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = *self.s.get(self.i).ok_or_else(|| self.err("bad escape"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.eat("\\u") {
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                                }
                            }
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
            }
        }
    }
}

/// Write `v` as compact JSON.
pub fn stringify(v: &Value) -> Result<String, String> {
    let mut out = String::new();
    write_value(&mut out, v, 0)?;
    Ok(out)
}

fn write_str(out: &mut String, s: &str) {
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

fn write_value(out: &mut String, v: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("to_json: nesting is too deep".into());
    }
    match v {
        Value::Unit => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Float(x) if x.is_finite() => {
            let _ = write!(out, "{x:?}");
        }
        Value::Float(x) => return Err(format!("to_json: {x} cannot be written as JSON")),
        Value::Str(s) => write_str(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item, depth + 1)?;
            }
            out.push(']');
        }
        Value::Map(map) => {
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let key = match k {
                    Key::Str(s) => s.to_string(),
                    Key::Int(n) => n.to_string(),
                    Key::Bool(b) => b.to_string(),
                };
                write_str(out, &key);
                out.push(':');
                write_value(out, item, depth + 1)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let text = r#"{"a":[1,2.5,-3e2,true,false,null],"b":{"c":"x\"y\n\u00e9\ud83d\ude00"},"n":12345678901234567890}"#;
        let v = parse(text).unwrap();
        let out = stringify(&v).unwrap();
        assert_eq!(
            out,
            r#"{"a":[1,2.5,-300.0,true,false,null],"b":{"c":"x\"y\né😀"},"n":1.2345678901234567e19}"#
        );
        assert_eq!(parse(&out).unwrap(), v);
    }

    #[test]
    fn rejects_bad_json() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "tru",
            "01x",
            "\"abc",
            "[1] 2",
            "{1:2}",
            "\"\\x\"",
            "-",
            "1.",
            "\"a\nb\"",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should fail");
        }
        let deep = "[".repeat(MAX_DEPTH + 5) + &"]".repeat(MAX_DEPTH + 5);
        assert!(parse(&deep).unwrap_err().contains("too deep"));
    }

    #[test]
    fn whitespace_and_empty_containers() {
        assert_eq!(
            stringify(&parse(" { \"x\" : [ ] , \"y\" : { } } ").unwrap()).unwrap(),
            r#"{"x":[],"y":{}}"#
        );
        assert!(stringify(&Value::Float(f64::NAN)).is_err());
    }
}
