//! Small strict JSON value/parser for evidence. Reject duplicate keys, invalid UTF-8,
//! non-JSON numbers and trailing input. No dependency on a package registry.
use crate::{Result, refuse};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }
    pub fn number(value: impl ToString) -> Self {
        Self::Number(value.to_string())
    }
    pub fn object(pairs: impl IntoIterator<Item = (String, Value)>) -> Self {
        Self::Object(pairs.into_iter().collect())
    }
    pub fn array(values: impl IntoIterator<Item = Value>) -> Self {
        Self::Array(values.into_iter().collect())
    }
    pub fn get(&self, key: &str) -> Result<&Value> {
        match self {
            Self::Object(map) => map
                .get(key)
                .ok_or_else(|| format!("missing JSON field: {key}")),
            _ => refuse("expected JSON object"),
        }
    }
    pub fn str(&self) -> Result<&str> {
        match self {
            Self::String(s) => Ok(s),
            _ => refuse("expected JSON string"),
        }
    }
    pub fn integer(&self) -> Result<i64> {
        match self {
            Self::Number(n) => n
                .parse()
                .map_err(|_| format!("expected JSON integer, got {n}")),
            _ => refuse("expected JSON integer"),
        }
    }
    pub fn bool(&self) -> Result<bool> {
        match self {
            Self::Bool(v) => Ok(*v),
            _ => refuse("expected JSON boolean"),
        }
    }
    pub fn array_ref(&self) -> Result<&[Value]> {
        match self {
            Self::Array(v) => Ok(v),
            _ => refuse("expected JSON array"),
        }
    }
    pub fn entries_mut(&mut self) -> Result<&mut BTreeMap<String, Value>> {
        match self {
            Self::Object(v) => Ok(v),
            _ => refuse("expected JSON object"),
        }
    }
    pub fn entries(&self) -> Result<&BTreeMap<String, Value>> {
        match self {
            Self::Object(v) => Ok(v),
            _ => refuse("expected JSON object"),
        }
    }
    pub fn canonical(&self) -> String {
        // Preserve the canonical byte contract while avoiding the temporary
        // String and Vec allocations at every nested array/object level.
        let mut output = String::new();
        self.write_canonical(&mut output);
        output
    }

    fn write_canonical(&self, output: &mut String) {
        match self {
            Self::Null => output.push_str("null"),
            Self::Bool(v) => output.push_str(if *v { "true" } else { "false" }),
            Self::Number(v) => output.push_str(v),
            Self::String(v) => write_quoted(v, output),
            Self::Array(values) => {
                output.push('[');
                for (i, value) in values.iter().enumerate() {
                    if i != 0 {
                        output.push(',');
                    }
                    value.write_canonical(output);
                }
                output.push(']');
            }
            Self::Object(fields) => {
                output.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i != 0 {
                        output.push(',');
                    }
                    write_quoted(key, output);
                    output.push(':');
                    value.write_canonical(output);
                }
                output.push('}');
            }
        }
    }
    pub fn sha256(&self) -> String {
        crate::digest::sha256(self.canonical().as_bytes())
    }
}

pub fn obj(pairs: &[(&str, Value)]) -> Value {
    Value::object(pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())))
}

fn write_quoted(s: &str, out: &mut String) {
    use std::fmt::Write as _;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => {
                write!(out, "\\u{:04x}", c as u32).expect("writing a String cannot fail");
            }
            c if (c as u32) > 0x7f && (c as u32) <= 0xffff => {
                write!(out, "\\u{:04x}", c as u32).expect("writing a String cannot fail");
            }
            c if (c as u32) > 0xffff => {
                let u = (c as u32) - 0x10000;
                write!(out, "\\u{:04x}\\u{:04x}", 0xd800 + (u >> 10), 0xdc00 + (u & 0x3ff))
                    .expect("writing a String cannot fail");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
}
impl<'a> Parser<'a> {
    fn next(&mut self) -> Result<u8> {
        if self.at >= self.input.len() {
            return refuse("unexpected end of JSON");
        }
        let b = self.input[self.at];
        self.at += 1;
        Ok(b)
    }
    fn peek(&self) -> Option<u8> {
        self.input.get(self.at).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
    }
    fn exact(&mut self, b: u8) -> Result<()> {
        if self.next()? == b {
            Ok(())
        } else {
            refuse("unexpected JSON token")
        }
    }
    fn literal(&mut self, rest: &[u8]) -> Result<()> {
        for &b in rest {
            self.exact(b)?;
        }
        Ok(())
    }
    fn hex4(&mut self) -> Result<u16> {
        let mut n = 0u16;
        for _ in 0..4 {
            let c = self.next()?;
            n = n.checked_mul(16).ok_or("unicode overflow")?
                + match c {
                    b'0'..=b'9' => u16::from(c - b'0'),
                    b'a'..=b'f' => u16::from(c - b'a' + 10),
                    b'A'..=b'F' => u16::from(c - b'A' + 10),
                    _ => return refuse("invalid \\u escape"),
                };
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<String> {
        self.exact(b'"')?;
        let mut bytes = Vec::<u8>::new();
        loop {
            let c = self.next()?;
            match c {
                b'"' => break,
                b'\\' => match self.next()? {
                    b'"' => bytes.push(b'"'),
                    b'\\' => bytes.push(b'\\'),
                    b'/' => bytes.push(b'/'),
                    b'b' => bytes.push(8),
                    b'f' => bytes.push(12),
                    b'n' => bytes.push(b'\n'),
                    b'r' => bytes.push(b'\r'),
                    b't' => bytes.push(b'\t'),
                    b'u' => {
                        let high = self.hex4()?;
                        let scalar = if (0xd800..=0xdbff).contains(&high) {
                            self.exact(b'\\')?;
                            self.exact(b'u')?;
                            let low = self.hex4()?;
                            if !(0xdc00..=0xdfff).contains(&low) {
                                return refuse("invalid low surrogate");
                            }
                            0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00)
                        } else if (0xdc00..=0xdfff).contains(&high) {
                            return refuse("unpaired low surrogate");
                        } else {
                            u32::from(high)
                        };
                        let mut utf8 = [0u8; 4];
                        let ch = char::from_u32(scalar).ok_or("invalid unicode scalar")?;
                        bytes.extend_from_slice(ch.encode_utf8(&mut utf8).as_bytes());
                    }
                    _ => return refuse("invalid JSON escape"),
                },
                0..=31 => return refuse("unescaped JSON control character"),
                _ => bytes.push(c),
            }
        }
        String::from_utf8(bytes).map_err(|_| "invalid UTF-8 in JSON".to_string())
    }
    fn number(&mut self) -> Result<Value> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                }
            }
            _ => return refuse("invalid JSON number"),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return refuse("malformed JSON fraction");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return refuse("malformed JSON exponent");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        Ok(Value::Number(
            std::str::from_utf8(&self.input[start..self.at])
                .map_err(|_| "invalid number bytes")?
                .to_owned(),
        ))
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > 128 {
            return refuse("JSON nesting too deep");
        }
        self.ws();
        match self.peek() {
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'{') => {
                self.at += 1;
                self.ws();
                let mut out = BTreeMap::new();
                if self.peek() == Some(b'}') {
                    self.at += 1;
                    return Ok(Value::Object(out));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    self.exact(b':')?;
                    self.ws();
                    let value = self.value(depth + 1)?;
                    if out.insert(key, value).is_some() {
                        return refuse("duplicate JSON object key");
                    }
                    self.ws();
                    match self.next()? {
                        b',' => continue,
                        b'}' => break,
                        _ => return refuse("malformed JSON object"),
                    }
                }
                Ok(Value::Object(out))
            }
            Some(b'[') => {
                self.at += 1;
                self.ws();
                let mut out = Vec::new();
                if self.peek() == Some(b']') {
                    self.at += 1;
                    return Ok(Value::Array(out));
                }
                loop {
                    out.push(self.value(depth + 1)?);
                    self.ws();
                    match self.next()? {
                        b',' => continue,
                        b']' => break,
                        _ => return refuse("malformed JSON array"),
                    }
                }
                Ok(Value::Array(out))
            }
            Some(b't') => {
                self.at += 1;
                self.literal(b"rue")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.at += 1;
                self.literal(b"alse")?;
                Ok(Value::Bool(false))
            }
            Some(b'n') => {
                self.at += 1;
                self.literal(b"ull")?;
                Ok(Value::Null)
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => refuse("invalid JSON value"),
        }
    }
}

pub fn parse(input: &[u8]) -> Result<Value> {
    let mut p = Parser { input, at: 0 };
    let value = p.value(0)?;
    p.ws();
    if p.at != input.len() {
        return refuse("trailing data after JSON value");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_canonical() {
        let v = parse(br#"{"z":1,"a":[true,null,"\uD83D\uDE00"]}"#).expect("valid");
        assert_eq!(v.canonical(), r#"{"a":[true,null,"\ud83d\ude00"],"z":1}"#);
        assert_eq!(parse(v.canonical().as_bytes()).expect("round trip"), v);
    }
    // Retain a structurally independent copy of the previous serializer in
    // tests. The optimized writer must preserve historical view hashes even
    // for nested records and non-ASCII characters.
    fn reference_canonical(value: &Value) -> String {
        fn quote(value: &str) -> String {
            let mut out = String::from("\"");
            for c in value.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\u{8}' => out.push_str("\\b"),
                    '\u{c}' => out.push_str("\\f"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c if (c as u32) > 0x7f && (c as u32) <= 0xffff => {
                        out.push_str(&format!("\\u{:04x}", c as u32))
                    }
                    c if (c as u32) > 0xffff => {
                        let u = (c as u32) - 0x10000;
                        out.push_str(&format!(
                            "\\u{:04x}\\u{:04x}",
                            0xd800 + (u >> 10),
                            0xdc00 + (u & 0x3ff)
                        ));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        }
        match value {
            Value::Null => "null".into(),
            Value::Bool(v) => v.to_string(),
            Value::Number(v) => v.clone(),
            Value::String(v) => quote(v),
            Value::Array(v) => format!(
                "[{}]",
                v.iter().map(reference_canonical).collect::<Vec<_>>().join(",")
            ),
            Value::Object(v) => format!(
                "{{{}}}",
                v.iter().map(|(k, v)| format!("{}:{}", quote(k), reference_canonical(v)))
                    .collect::<Vec<_>>().join(",")
            ),
        }
    }

    #[test]
    fn canonical_matches_preoptimization_bytes_and_hashes() {
        let records = [
            Value::Null,
            Value::Bool(true),
            Value::number("-1.250e+4"),
            Value::string("plain"),
            Value::string("\"\\\n\r\t\u{8}\u{c}\0\u{1f}\u{7f}\u{80}\u{e9}\u{20ac}\u{ffff}\u{10000}😀"),
            obj(&[
                ("z", Value::array([])),
                ("é", Value::string("astral 🎼")),
                ("nested", Value::array([
                    obj(&[("b", Value::Bool(false)), ("a", Value::number("0.000"))]),
                    Value::Null,
                ])),
            ]),
        ];
        for record in &records {
            let expected = reference_canonical(record);
            assert_eq!(record.canonical(), expected);
            assert_eq!(record.sha256(), crate::digest::sha256(expected.as_bytes()));
        }
        let mut sample = Value::String("a".into());
        for i in 0..95 {
            sample = Value::array([
                sample,
                obj(&[
                    ("🧪", Value::number(i)),
                    ("slashes", Value::string("\\/\"")),
                    ("a", Value::array([])),
                ]),
            ]);
        }
        let old = reference_canonical(&sample);
        assert_eq!(sample.canonical(), old);
        assert_eq!(crate::json::parse(old.as_bytes()).unwrap(), sample);
    }

    #[test]
    fn strict_refusals() {
        for s in [
            r#"{"a":1,"a":2}"#,
            "{\"x\":01}",
            "[1,]",
            "true false",
            "\"\\uD800\"",
            "\"\\uDC00\"",
            "{\"a\":NaN}",
        ] {
            assert!(parse(s.as_bytes()).is_err(), "accepted {s}");
        }
    }
    #[test]
    fn ascii_python_canonical() {
        assert_eq!(
            obj(&[("z", Value::number(1)), ("a", Value::Bool(false))]).canonical(),
            r#"{"a":false,"z":1}"#
        );
    }
}
