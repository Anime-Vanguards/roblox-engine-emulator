//! JSON decoding with Lune's number semantics. The Luau runtime reads JSON (recorded clocks,
//! `--actions`, `--controls`, order dumps, session commands) through Lune's `serde.decode`, whose
//! serde_json parses floats with the fast path (`f64_from_parts`: the decimal significand as a u64,
//! extra digits dropped, then one multiply / divide by a power of ten), not the correctly rounded
//! one this crate enables (`float_roundtrip`). A 17-digit value such as Studio's
//! `24490.894583499998` lands one ulp apart, and a recorded clock's times with it: this decoder
//! reproduces Lune's values. Structure and strings are plain JSON (serde_json::Value, objects
//! without duplicate keys: the last one wins, as in a Luau table).

use serde_json::{Map, Number, Value as Json};

/// Decodes `text` as Lune's `serde.decode("json", text)` sees its numbers.
pub fn decode(text: &str) -> Result<Json, String> {
    let mut parser = Parser { bytes: text.as_bytes(), index: 0 };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.index != parser.bytes.len() {
        return Err(parser.error("trailing characters"));
    }
    Ok(value)
}

/// serde_json's `f64_from_parts` without `float_roundtrip`.
pub fn f64_from_parts(positive: bool, significand: u64, mut exponent: i32) -> Option<f64> {
    let mut f = significand as f64;
    loop {
        let index = exponent.unsigned_abs() as usize;
        if index <= 308 {
            let pow = pow10(index);
            if exponent >= 0 {
                f *= pow;
                if f.is_infinite() {
                    return None;
                }
            } else {
                f /= pow;
            }
            break;
        }
        if f == 0.0 {
            break;
        }
        if exponent >= 0 {
            return None;
        }
        f /= 1e308;
        exponent += 308;
    }
    Some(if positive { f } else { -f })
}

/// 10^index as the literal `1e<index>` (serde_json's POW10 table).
fn pow10(index: usize) -> f64 {
    format!("1e{index}").parse().unwrap()
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.index)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.index += 1;
        }
    }

    fn expect(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.bytes[self.index..].starts_with(literal) {
            self.index += literal.len();
            Ok(())
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 128 {
            return Err(self.error("recursion limit exceeded"));
        }
        match self.peek() {
            Some(b'{') => {
                self.index += 1;
                let mut map = Map::new();
                self.skip_whitespace();
                if self.peek() == Some(b'}') {
                    self.index += 1;
                    return Ok(Json::Object(map));
                }
                loop {
                    self.skip_whitespace();
                    let key = self.string()?;
                    self.skip_whitespace();
                    self.expect(b":")?;
                    self.skip_whitespace();
                    let value = self.value(depth + 1)?;
                    map.insert(key, value);
                    self.skip_whitespace();
                    match self.peek() {
                        Some(b',') => self.index += 1,
                        Some(b'}') => {
                            self.index += 1;
                            return Ok(Json::Object(map));
                        }
                        _ => return Err(self.error("expected , or }")),
                    }
                }
            }
            Some(b'[') => {
                self.index += 1;
                let mut items = Vec::new();
                self.skip_whitespace();
                if self.peek() == Some(b']') {
                    self.index += 1;
                    return Ok(Json::Array(items));
                }
                loop {
                    self.skip_whitespace();
                    items.push(self.value(depth + 1)?);
                    self.skip_whitespace();
                    match self.peek() {
                        Some(b',') => self.index += 1,
                        Some(b']') => {
                            self.index += 1;
                            return Ok(Json::Array(items));
                        }
                        _ => return Err(self.error("expected , or ]")),
                    }
                }
            }
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.expect(b"true").map(|_| Json::Bool(true)),
            Some(b'f') => self.expect(b"false").map(|_| Json::Bool(false)),
            Some(b'n') => self.expect(b"null").map(|_| Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("expected value")),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.bytes.get(self.index..self.index + 4).ok_or_else(|| self.error("bad escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.error("bad escape"))?;
        let value = u32::from_str_radix(text, 16).map_err(|_| self.error("bad escape"))?;
        self.index += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(self.error("expected string"));
        }
        self.index += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(byte) = self.peek() else { return Err(self.error("unterminated string")) };
            self.index += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let Some(escape) = self.peek() else { return Err(self.error("bad escape")) };
                    self.index += 1;
                    match escape {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) && self.bytes[self.index..].starts_with(b"\\u") {
                                self.index += 2;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let character = char::from_u32(code).unwrap_or('\u{FFFD}');
                            let mut buffer = [0u8; 4];
                            out.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
                        }
                        _ => return Err(self.error("bad escape")),
                    }
                }
                other => out.push(other),
            }
        }
        String::from_utf8(out).map_err(|_| self.error("invalid UTF-8"))
    }

    fn digit(&self) -> Option<u64> {
        self.peek().filter(u8::is_ascii_digit).map(|c| (c - b'0') as u64)
    }

    fn number(&mut self) -> Result<Json, String> {
        let positive = if self.peek() == Some(b'-') {
            self.index += 1;
            false
        } else {
            true
        };
        let Some(first) = self.digit() else { return Err(self.error("invalid number")) };
        self.index += 1;
        let mut significand = first;
        let mut exponent: i32 = 0;
        let mut float = false;
        let mut overflowed = false;
        if first != 0 {
            while let Some(digit) = self.digit() {
                self.index += 1;
                if overflowed {
                    exponent += 1;
                    continue;
                }
                match significand.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                    Some(next) => significand = next,
                    None => {
                        // parse_long_integer: further digits only scale
                        overflowed = true;
                        float = true;
                        exponent += 1;
                    }
                }
            }
        } else if self.digit().is_some() {
            return Err(self.error("invalid number"));
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            float = true;
            if self.digit().is_none() {
                return Err(self.error("invalid number"));
            }
            while let Some(digit) = self.digit() {
                self.index += 1;
                if overflowed {
                    continue;
                }
                match significand.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                    Some(next) => {
                        significand = next;
                        exponent -= 1;
                    }
                    // parse_decimal_overflow: the rest of the digits are dropped
                    None => overflowed = true,
                }
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            float = true;
            let positive_exp = match self.peek() {
                Some(b'+') => {
                    self.index += 1;
                    true
                }
                Some(b'-') => {
                    self.index += 1;
                    false
                }
                _ => true,
            };
            let Some(first) = self.digit() else { return Err(self.error("invalid number")) };
            self.index += 1;
            let mut exp = first as i32;
            let mut exp_overflow = false;
            while let Some(digit) = self.digit() {
                self.index += 1;
                match exp.checked_mul(10).and_then(|v| v.checked_add(digit as i32)) {
                    Some(next) => exp = next,
                    None => exp_overflow = true,
                }
            }
            if exp_overflow {
                // parse_exponent_overflow: 0 for a zero significand or a negative exponent
                if significand == 0 || !positive_exp {
                    return Ok(Json::Number(Number::from_f64(if positive { 0.0 } else { -0.0 }).unwrap()));
                }
                return Err(self.error("number out of range"));
            }
            exponent = if positive_exp { exponent.saturating_add(exp) } else { exponent.saturating_sub(exp) };
        }
        if !float {
            if positive {
                return Ok(Json::Number(Number::from(significand)));
            }
            let negative = (significand as i64).wrapping_neg();
            if negative >= 0 {
                return Ok(Json::Number(Number::from_f64(-(significand as f64)).ok_or_else(|| self.error("invalid number"))?));
            }
            return Ok(Json::Number(Number::from(negative)));
        }
        let value = f64_from_parts(positive, significand, exponent).ok_or_else(|| self.error("number out of range"))?;
        Number::from_f64(value).map(Json::Number).ok_or_else(|| self.error("invalid number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lune_float_parsing() {
        // 17 significant digits: the u64 significand is not exact in f64 (Lune's value, not the
        // correctly rounded one)
        let value = decode("[24490.894583499998, 24490.894913599997, 0.016666666666666666, 1e-3, -2, 5, 1.5e3]").unwrap();
        let numbers: Vec<f64> = value.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        assert_eq!(numbers[0], 24490894583499998u64 as f64 / 1e12);
        assert_ne!(numbers[0], 24490.894583499998f64);
        assert_eq!(-(numbers[1] - numbers[0]) / 2.0, -0.00016504999803146347);
        assert_eq!(numbers[2], 16666666666666666u64 as f64 / 1e18);
        assert_eq!(&numbers[3..], &[0.001, -2.0, 5.0, 1500.0]);
        let text = decode(r#"{"a":"xé\n","b":[true,false,null],"c":{}}"#).unwrap();
        assert_eq!(text["a"], "xé\n");
        assert_eq!(text["b"][2], Json::Null);
    }
}
