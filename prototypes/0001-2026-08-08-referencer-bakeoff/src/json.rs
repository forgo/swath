//! Minimal, dependency-free JSON reader (sufficient for this prototype's manifest schema).
//!
//! This is intentionally small — a correct recursive-descent parser for standard JSON
//! (objects, arrays, strings with escapes, numbers, booleans, null). It is not a general-purpose
//! library; it exists so the harness stays std-only. Writing is done directly by the manifest module.

#[derive(Debug, Clone)]
#[allow(dead_code)] // Bool/Null are parsed for completeness though the manifest schema doesn't read them
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kvs) => kvs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    /// Byte offsets/lengths fit well within f64's exact-integer range (< 2^53) for real granules.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().map(|n| n as u64)
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

pub fn parse(s: &str) -> Result<Json, String> {
    let mut p = Parser {
        b: s.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!("trailing input at byte {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn peek(&self) -> Result<u8, String> {
        self.b
            .get(self.i)
            .copied()
            .ok_or_else(|| "unexpected end of input".to_string())
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Json::Str(self.string()?)),
            b't' | b'f' => self.boolean(),
            b'n' => self.null(),
            _ => self.number(),
        }
    }
    fn object(&mut self) -> Result<Json, String> {
        self.i += 1; // consume '{'
        let mut kvs = Vec::new();
        self.ws();
        if self.peek()? == b'}' {
            self.i += 1;
            return Ok(Json::Obj(kvs));
        }
        loop {
            self.ws();
            if self.peek()? != b'"' {
                return Err(format!("expected key string at byte {}", self.i));
            }
            let k = self.string()?;
            self.ws();
            if self.peek()? != b':' {
                return Err(format!("expected ':' at byte {}", self.i));
            }
            self.i += 1;
            let v = self.value()?;
            kvs.push((k, v));
            self.ws();
            match self.peek()? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    break;
                }
                c => {
                    return Err(format!(
                        "expected ',' or '}}' at byte {}, got '{}'",
                        self.i, c as char
                    ));
                }
            }
        }
        Ok(Json::Obj(kvs))
    }
    fn array(&mut self) -> Result<Json, String> {
        self.i += 1; // consume '['
        let mut a = Vec::new();
        self.ws();
        if self.peek()? == b']' {
            self.i += 1;
            return Ok(Json::Arr(a));
        }
        loop {
            let v = self.value()?;
            a.push(v);
            self.ws();
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    break;
                }
                c => {
                    return Err(format!(
                        "expected ',' or ']' at byte {}, got '{}'",
                        self.i, c as char
                    ));
                }
            }
        }
        Ok(Json::Arr(a))
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // consume opening quote
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    match e {
                        b'"' => buf.push(b'"'),
                        b'\\' => buf.push(b'\\'),
                        b'/' => buf.push(b'/'),
                        b'n' => buf.push(b'\n'),
                        b't' => buf.push(b'\t'),
                        b'r' => buf.push(b'\r'),
                        b'b' => buf.push(0x08),
                        b'f' => buf.push(0x0c),
                        b'u' => {
                            if self.i + 4 > self.b.len() {
                                return Err("truncated \\u escape".to_string());
                            }
                            let hex = std::str::from_utf8(&self.b[self.i..self.i + 4])
                                .map_err(|_| "bad \\u escape".to_string())?;
                            let cp = u32::from_str_radix(hex, 16)
                                .map_err(|_| "bad \\u hex".to_string())?;
                            self.i += 4;
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            let mut tmp = [0u8; 4];
                            buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                        }
                        _ => return Err(format!("bad escape '\\{}'", e as char)),
                    }
                }
                _ => buf.push(c), // raw UTF-8 byte, preserved
            }
        }
        String::from_utf8(buf).map_err(|_| "invalid UTF-8 in string".to_string())
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek()? == b'-' {
            self.i += 1;
        }
        while self.i < self.b.len()
            && matches!(
                self.b[self.i],
                b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
            )
        {
            self.i += 1;
        }
        let tok =
            std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad number".to_string())?;
        tok.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("bad number '{tok}'"))
    }
    fn boolean(&mut self) -> Result<Json, String> {
        if self.b[self.i..].starts_with(b"true") {
            self.i += 4;
            Ok(Json::Bool(true))
        } else if self.b[self.i..].starts_with(b"false") {
            self.i += 5;
            Ok(Json::Bool(false))
        } else {
            Err(format!("bad literal at byte {}", self.i))
        }
    }
    fn null(&mut self) -> Result<Json, String> {
        if self.b[self.i..].starts_with(b"null") {
            self.i += 4;
            Ok(Json::Null)
        } else {
            Err(format!("bad literal at byte {}", self.i))
        }
    }
}
