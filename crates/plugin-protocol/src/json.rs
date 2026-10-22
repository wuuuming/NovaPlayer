//! Shared JSON reader/writer with the original host/kit Value API.
//! Parsing accepts strict JSON, Unicode scalar strings, and finite f64 numbers, with
//! at most 64 nested values. Object keys are sorted; duplicate keys keep the last value.
//! Numbers retain the historical f64 representation (integers above 2^53 may round).

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, k: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.get(k),
            _ => None,
        }
    }
    /// `a.b.c` navigation; missing links are `None`.
    pub fn at(&self, path: &[&str]) -> Option<&Value> {
        let mut v = self;
        for k in path {
            v = v.get(k)?;
        }
        Some(v)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n < 18446744073709551616.0 && n.fract() == 0.0)
            .map(|n| n as u64)
    }
    /// A number that some APIs return as a string.
    pub fn as_num_loose(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn as_str_loose(&self) -> Option<String> {
        match self {
            Value::Str(s) => Some(s.clone()),
            Value::Num(n) => Some(format!("{}", *n as i64)),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
    /// `Str` from anything string-like.
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }
}

/// An object from (key, value) pairs.
pub fn obj(fields: Vec<(&str, Value)>) -> Value {
    Value::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// One line of JSON (no newlines: strings escape them).
pub fn write(v: &Value) -> String {
    let mut out = String::new();
    put(v, &mut out);
    out
}

fn put(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) if n.is_finite() => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                out.push_str(&format!("{}", *n as i64));
            } else {
                out.push_str(&format!("{n}"));
            }
        }
        Value::Num(_) => out.push_str("null"),
        Value::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                put(x, out);
            }
            out.push(']');
        }
        Value::Obj(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                put(&Value::Str(k.clone()), out);
                out.push(':');
                put(x, out);
            }
            out.push('}');
        }
    }
}

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = P { s: text.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing content at {}", p.i));
    }
    Ok(v)
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at {}", self.i))
    }
    fn lit(&mut self, w: &str, v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            self.err("unrecognised value")
        }
    }
    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        self.depth += 1;
        if self.depth > 64 {
            return self.err("nested too deep");
        }
        let v = match self.s.get(self.i) {
            None => self.err("unexpected end"),
            Some(b'{') => self.obj(),
            Some(b'[') => self.arr(),
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(_) => self.num(),
        };
        self.depth -= 1;
        v
    }
    fn obj(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut m = BTreeMap::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Value::Obj(m));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return self.err("string key expected");
            }
            let k = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return self.err("':' expected");
            }
            self.i += 1;
            let v = self.value()?;
            m.insert(k, v);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Obj(m));
                }
                _ => return self.err("',' or '}' expected"),
            }
        }
    }
    fn arr(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Value::Arr(a));
        }
        loop {
            a.push(self.value()?);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Arr(a));
                }
                _ => return self.err("',' or ']' expected"),
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.s.get(self.i..self.i + 4).ok_or("incomplete escape")?;
        if !h.iter().all(u8::is_ascii_hexdigit) {
            return self.err("bad escape");
        }
        let h = std::str::from_utf8(h).map_err(|_| "bad escape")?;
        self.i += 4;
        u32::from_str_radix(h, 16).map_err(|_| "bad escape".to_string())
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return self.err("unterminated string") };
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| "string is not UTF-8".into()),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return self.err("incomplete escape") };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut u = self.hex4()?;
                            if (0xD800..0xDC00).contains(&u) {
                                if !self.s[self.i..].starts_with(b"\\u") {
                                    return self.err("missing low surrogate");
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.err("invalid low surrogate");
                                }
                                u = 0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00);
                            }
                            char::from_u32(u).ok_or_else(|| format!("invalid Unicode scalar at {}", self.i))?
                        }
                        _ => return self.err("unknown escape"),
                    };
                    let mut b = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                c if c < 0x20 => return self.err("control character in string"),
                c => out.push(c),
            }
        }
    }
    fn digits(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
    }
    fn num(&mut self) -> Result<Value, String> {
        let st = self.i;
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return self.err("number expected"),
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            let start = self.i;
            self.digits();
            if self.i == start {
                return self.err("fraction expected");
            }
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let start = self.i;
            self.digits();
            if self.i == start {
                return self.err("exponent expected");
            }
        }
        std::str::from_utf8(&self.s[st..self.i])
            .ok()
            .and_then(|t| t.parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .map(Value::Num)
            .ok_or_else(|| format!("unrecognised value at {st}"))
    }
}
