use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(arr) => Some(arr.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, JsonValue>> {
        match self {
            JsonValue::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        self.as_object()?.get(key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub position: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "json error at byte {}: {}", self.position, self.message)
    }
}

impl std::error::Error for JsonError {}

fn hex_nibble(byte: u8) -> Option<u16> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u16),
        b'a'..=b'f' => Some((byte - b'a' + 10) as u16),
        b'A'..=b'F' => Some((byte - b'A' + 10) as u16),
        _ => None,
    }
}

pub fn parse_hex4_bytes(b: [u8; 4]) -> Option<u16> {
    let d0 = hex_nibble(b[0])?;
    let d1 = hex_nibble(b[1])?;
    let d2 = hex_nibble(b[2])?;
    let d3 = hex_nibble(b[3])?;
    Some((d0 << 12) | (d1 << 8) | (d2 << 4) | d3)
}

pub fn decode_utf16_surrogate_pair(hi: u16, lo: u16) -> Option<char> {
    if (0xD800..=0xDBFF).contains(&hi) && (0xDC00..=0xDFFF).contains(&lo) {
        let scalar = 0x10000 + ((((hi as u32) - 0xD800) << 10) | ((lo as u32) - 0xDC00));
        char::from_u32(scalar)
    } else {
        None
    }
}

pub fn validate_json_number_bytes(bytes: &[u8]) -> bool {
    let len = bytes.len();
    let mut i = 0;

    if i < len && bytes[i] == b'-' {
        i += 1;
    }

    if i >= len {
        return false;
    }

    match bytes[i] {
        b'0' => {
            i += 1;
        }
        b'1'..=b'9' => {
            i += 1;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        _ => return false,
    }

    if i < len && bytes[i] == b'.' {
        i += 1;
        if i >= len || !bytes[i].is_ascii_digit() {
            return false;
        }
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }

    if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        if i >= len || !bytes[i].is_ascii_digit() {
            return false;
        }
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }

    i == len
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn err(&self, message: impl Into<String>) -> JsonError {
        JsonError {
            position: self.pos,
            message: message.into(),
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn read_hex4(&mut self) -> Result<u16, JsonError> {
        if self.pos + 4 > self.bytes.len() {
            return Err(self.err("incomplete unicode escape"));
        }
        let hex4 = [
            self.bytes[self.pos],
            self.bytes[self.pos + 1],
            self.bytes[self.pos + 2],
            self.bytes[self.pos + 3],
        ];
        let unit = parse_hex4_bytes(hex4).ok_or_else(|| self.err("invalid hex in unicode escape"))?;
        self.pos += 4;
        Ok(unit)
    }

    fn parse_value(&mut self) -> Result<JsonValue, JsonError> {
        self.skip_ws();
        match self.peek() {
            Some(b'n') => self.parse_literal(b"null", JsonValue::Null),
            Some(b't') => self.parse_literal(b"true", JsonValue::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", JsonValue::Bool(false)),
            Some(b'"') => self.parse_string().map(JsonValue::String),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            _ => Err(self.err("unexpected token")),
        }
    }

    fn parse_literal(&mut self, lit: &[u8], val: JsonValue) -> Result<JsonValue, JsonError> {
        if self.bytes[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        if self.peek() != Some(b'"') {
            return Err(self.err("expected '\"'"));
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                0x00..=0x1f => return Err(self.err("unescaped control character in string")),
                b'\\' => {
                    if self.pos >= self.bytes.len() {
                        return Err(self.err("unterminated escape"));
                    }
                    let esc = self.bytes[self.pos];
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let unit = self.read_hex4()?;
                            if (0xD800..=0xDBFF).contains(&unit) {
                                if self.pos + 2 > self.bytes.len()
                                    || self.bytes[self.pos] != b'\\'
                                    || self.bytes[self.pos + 1] != b'u'
                                {
                                    return Err(self.err("unpaired high surrogate"));
                                }
                                self.pos += 2;
                                let lo = self.read_hex4()?;
                                let ch = decode_utf16_surrogate_pair(unit, lo)
                                    .ok_or_else(|| self.err("invalid low surrogate"))?;
                                out.push(ch);
                            } else if (0xDC00..=0xDFFF).contains(&unit) {
                                return Err(self.err("unpaired low surrogate"));
                            } else {
                                let ch = char::from_u32(unit as u32)
                                    .ok_or_else(|| self.err("invalid unicode scalar"))?;
                                out.push(ch);
                            }
                        }
                        _ => return Err(self.err("unknown escape character")),
                    }
                }
                _ => {
                    let start = self.pos - 1;
                    if b < 0x80 {
                        out.push(b as char);
                    } else {
                        while self.pos < self.bytes.len() && (self.bytes[self.pos] & 0xc0) == 0x80 {
                            self.pos += 1;
                        }
                        let s = std::str::from_utf8(&self.bytes[start..self.pos])
                            .map_err(|_| self.err("invalid utf8 in string"))?;
                        out.push_str(s);
                    }
                }
            }
        }
        Err(self.err("unterminated string"))
    }

    fn parse_number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            if matches!(b, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let slice = &self.bytes[start..self.pos];
        if !validate_json_number_bytes(slice) {
            return Err(self.err("invalid number"));
        }
        let s = std::str::from_utf8(slice).map_err(|_| self.err("invalid number bytes"))?;
        let n: f64 = s.parse().map_err(|_| self.err("invalid number"))?;
        if !n.is_finite() {
            return Err(self.err("non-finite number"));
        }
        Ok(JsonValue::Number(n))
    }

    fn parse_array(&mut self) -> Result<JsonValue, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, JsonError> {
        self.pos += 1;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(map));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':'"));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(map));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }
}

pub fn parse_json(input: &str) -> Result<JsonValue, JsonError> {
    let mut parser = Parser::new(input);
    let val = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(parser.err("trailing characters after json value"));
    }
    Ok(val)
}

pub fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
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
    out
}

pub fn to_json_string(val: &JsonValue) -> String {
    match val {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        JsonValue::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        JsonValue::String(s) => escape_json_string(s),
        JsonValue::Array(items) => {
            let parts: Vec<String> = items.iter().map(to_json_string).collect();
            format!("[{}]", parts.join(","))
        }
        JsonValue::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}:{}", escape_json_string(k), to_json_string(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16_surrogate_pair_emojis_and_bmp_escapes() {
        let rocket = parse_json(r#""\uD83D\uDE80""#).unwrap();
        assert_eq!(rocket.as_str(), Some("🚀"));

        let mixed = parse_json(r#""ship \uD83D\uDE80 + \u00e9 + \uD83C\uDF89""#).unwrap();
        assert_eq!(mixed.as_str(), Some("ship 🚀 + é + 🎉"));

        assert_eq!(decode_utf16_surrogate_pair(0xD83D, 0xDE80), Some('🚀'));
        assert_eq!(decode_utf16_surrogate_pair(0xD800, 0xDC00), Some('\u{10000}'));
        assert_eq!(decode_utf16_surrogate_pair(0xDBFF, 0xDFFF), Some('\u{10FFFF}'));
    }

    #[test]
    fn rejects_lone_and_malformed_surrogates() {
        assert!(parse_json(r#""\uD83D""#).is_err());
        assert!(parse_json(r#""\uD83Dabc""#).is_err());
        assert!(parse_json(r#""\uD83D\n""#).is_err());
        assert!(parse_json(r#""\uD83D\u0041""#).is_err());
        assert!(parse_json(r#""\uD83D\uD83D""#).is_err());
        assert!(parse_json(r#""\uDE80""#).is_err());
        assert!(parse_json(r#""\uDE80\uD83D""#).is_err());
        assert!(parse_json(r#""\uD83D\uDE8""#).is_err());
    }

    #[test]
    fn rejects_unescaped_control_characters_in_strings() {
        assert!(parse_json("\"hello\nworld\"").is_err());
        assert!(parse_json("\"hello\tworld\"").is_err());
        assert!(parse_json("\"hello\rworld\"").is_err());
        assert!(parse_json("\"hello\u{0000}world\"").is_err());
        assert!(parse_json("\"hello\u{001f}world\"").is_err());

        let escaped = parse_json(r#""hello\n\t\r\u0000\u001f\/world""#).unwrap();
        assert_eq!(escaped.as_str(), Some("hello\n\t\r\u{0000}\u{001f}/world"));
    }

    #[test]
    fn enforces_rfc8259_number_grammar() {
        for valid in [
            "0", "-0", "42", "-42", "0.5", "-0.5", "1.2e-6", "1.2e-06", "1E+10", "0e0",
        ] {
            assert!(
                validate_json_number_bytes(valid.as_bytes()),
                "expected valid: {valid}"
            );
            assert!(parse_json(valid).is_ok(), "expected parse ok: {valid}");
        }

        for invalid in [
            "01", "-01", "00", "1.", ".5", "1.e2", "1e", "1e+", "1e-", "-", "+1", "1.2.3", "1e2e3",
        ] {
            assert!(
                !validate_json_number_bytes(invalid.as_bytes()),
                "expected invalid: {invalid}"
            );
            assert!(parse_json(invalid).is_err(), "expected parse err: {invalid}");
        }
    }

    #[test]
    fn parses_real_openrouter_and_pi_jsonl_payload() {
        let payload = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Deploying \uD83D\uDE80 to https:\/\/openrouter.ai\/api\/v1","partial":{"role":"assistant","provider":"openrouter","model":"anthropic\/claude-sonnet-4","usage":{"input":1240,"output":85,"cost":{"input":1.2e-06,"output":0.000425,"total":4.262e-4}}}}}"#;

        let val = parse_json(payload).unwrap();
        assert_eq!(val.get("type").and_then(JsonValue::as_str), Some("message_update"));

        let evt = val.get("assistantMessageEvent").unwrap();
        assert_eq!(evt.get("type").and_then(JsonValue::as_str), Some("text_delta"));
        assert_eq!(
            evt.get("delta").and_then(JsonValue::as_str),
            Some("Deploying 🚀 to https://openrouter.ai/api/v1")
        );

        let cost = evt
            .get("partial")
            .and_then(|p| p.get("usage"))
            .and_then(|u| u.get("cost"))
            .unwrap();
        match cost.get("input") {
            Some(JsonValue::Number(n)) => assert!((n - 1.2e-6).abs() < 1e-12),
            other => panic!("expected numeric input cost, got {other:?}"),
        }
        match cost.get("total") {
            Some(JsonValue::Number(n)) => assert!((n - 4.262e-4).abs() < 1e-12),
            other => panic!("expected numeric total cost, got {other:?}"),
        }
    }
}
