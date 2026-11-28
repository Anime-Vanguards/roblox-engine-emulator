//! Rojo's data files, read with the readers `rojo build` 7.6.1 uses (its Cargo.lock: jsonc-parser 0.27.0, toml 0.5.11,
//! yaml-rust2 0.10.3), and the Luau source it writes for a data module (its src/lua_ast.rs). Measured against 7.6.1's builds of
//! a fixture of 154 files that build (data files, meta files and what they sit beside) and 114 one-file builds, 90 of which
//! Rojo refuses (each value as a client's require returns it, in pairs() order, and each error text; 7.7.0 differs only where
//! said):
//!
//! - **JSON** (`.json` modules, `.model.json`, meta files, project files) is read as lenient JSONC (jsonc-parser's defaults):
//!   `//` and `/* */` comments, trailing commas, missing commas, unquoted keys that are words or numbers (`a`, `a-b`, `é`, `1`,
//!   `2.5`; not `$a`, `1a` or `true`), single-quoted strings, raw control characters and line breaks in strings, hex numbers
//!   (`0x10`, `-0x10`, `+0x1F`; one past i64 is the string of its text, `"0xFFFFFFFFFFFFFFFFF"`), a leading `+`, and a number
//!   serde_json can't hold as the string of its text (`1e400` is `"1e400"`). Refused: `NaN`, `Infinity`, `.5`, `1.`, `01`,
//!   `[,]`, `[1,,2]`, two values, a byte order mark, an empty file or one of comments alone ("File contains no JSON value"),
//!   and any `\u` escape of a UTF-16 surrogate, a valid pair too (`"\ud83d\ude00"`; 7.7.0 reads pairs). Objects read as
//!   serde_json's map: a later key wins, keys in byte order.
//! - **TOML** (`.toml`) is toml 0.5.11's `Value`: an integer is a number (`i64 as f64`: 2^53 + 1 reads 2^53), `nan` / `inf`,
//!   a date or time the text toml's `Datetime` prints (`1979-05-27 07:32:00Z` is `"1979-05-27T07:32:00Z"`), tables by key in
//!   byte order, mixed arrays allowed.
//! - **YAML** (`.yaml`, `.yml`) is yaml-rust2's one document (none, or more than one, is refused): an integer above 2^53 is
//!   refused ("cannot be losslessly converted") while one below -2^53 rounds; reals by yaml-rust2's reading (`.nan`, `.inf`,
//!   `1e3`, `.5`); `~` / `null` / nothing is nil; anchors are copied where an alias stands (`<<` is a key like any other);
//!   mappings keep the document's key order and any key (`1`, `true`, `[1, 2]`, `{a: 1}`), but a null key crashes Rojo; a
//!   value an explicit tag can't read (`!!int abc`) is refused; a byte order mark is part of the first key.
//! - **The module's source** is `return <expression>`: a table one entry a line, tab-indented, `key = value,` for a key that
//!   is a Luau identifier and not a keyword (`["end"] = `, `["a-b"] = ` otherwise, `[1] = ` for a number, `[{...}] = ` for a
//!   table), an array `{1, 2, 3}` on one line (a null in it is `nil`, a hole), numbers as Rust prints them (all digits, no
//!   exponent), `0/0`, `math.huge`, `-math.huge`, strings with only `"`, `\`, CR, LF and tab escaped. So a string holding
//!   NUL makes source no client compiles ("Malformed string"), a NaN key fails the require ("table index is NaN"), and
//!   nesting past 255 levels runs out of registers: the module errors at require, as a published place's does.

use std::fmt::Write as _;

use yaml_rust2::Yaml;

/// How deeply a data file may nest. `rojo build` 7.6.1 builds every file this deep (a client compiles none past 255 levels)
/// and overflows its stack from about 1,300 levels (objects) to 1,700 (TOML arrays); ree refuses a file past this.
pub(crate) const MAX_DEPTH: usize = 1000;

/// Nesting past this is parsed on a thread of its own, with the stack the deepest file needs
const SHALLOW: usize = 128;

/// A JSON module's value, or None when the module runs the source Rojo writes for it instead (a string holding NUL, or
/// nesting past [`SHALLOW`]): that source is what a client runs, and it doesn't compile ([`json_module_source`]).
pub(crate) fn json_module(bytes: &[u8]) -> Result<Option<serde_json::Value>, String> {
    let value = read_json(bytes)?;
    // (a NUL is `\u0000` in strict JSON, a raw byte or that escape in JSONC: no source without either holds one)
    let maybe_nul = bytes.contains(&0) || memchr_find(bytes, b"\\u0000");
    if (maybe_nul && json_has_nul(&value)) || json_depth(&value) > SHALLOW {
        drop_deep(value);
        return Ok(None);
    }
    Ok(Some(value))
}

/// The Luau source `rojo build` writes for a JSON module (`return ...`)
pub(crate) fn json_module_source(bytes: &[u8]) -> Result<String, String> {
    let value = read_json(bytes)?;
    on_deep_stack(json_depth(&value) > SHALLOW, move || {
        let source = source_of(&json_expression(&value))?;
        drop(value);
        Ok(source)
    })
}

/// A JSON file read as Rojo's src/json.rs reads every JSON file (`parse_value`): its value, or Rojo's reason it refuses it.
/// Strict JSON reads through serde_json at its speed, as jsonc-parser would read it; anything else, and any `\u` escape of
/// a surrogate, through jsonc-parser itself.
pub(crate) fn read_json(bytes: &[u8]) -> Result<serde_json::Value, String> {
    // (strict JSON jsonc-parser reads alike: the same numbers through serde_json's Number, the same strings, a later key
    // winning; but for a surrogate escape, which it refuses. Nesting past 128 serde_json refuses.)
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if !has_surrogate_escape(bytes) {
            return Ok(value);
        }
    }
    let text = std::str::from_utf8(bytes).map_err(|error| format!("File is not valid UTF-8 ({error})"))?;
    let depth = jsonc_depth(text);
    if depth > MAX_DEPTH {
        return Err(too_deep());
    }
    on_deep_stack(depth > SHALLOW, || match jsonc_parser::parse_to_serde_value(text, &Default::default()) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err("File contains no JSON value".to_string()),
        Err(error) => Err(format!("JSONC parse error ({error})")),
    })
}

/// A JSON file read as [`read_json`] reads it, as strict JSON text that keeps its keys in the file's order (a reader that
/// takes them in that order: a project file's tree, rojo.rs): the file's own text when it is strict JSON, else the value
/// jsonc-parser read written out (each string and number as jsonc-parser converts it)
pub(crate) fn read_json_text(bytes: &[u8]) -> Result<std::borrow::Cow<'_, str>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| format!("File is not valid UTF-8 ({error})"))?;
    if serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok() && !has_surrogate_escape(bytes) {
        return Ok(std::borrow::Cow::Borrowed(text));
    }
    let depth = jsonc_depth(text);
    if depth > MAX_DEPTH {
        return Err(too_deep());
    }
    on_deep_stack(depth > SHALLOW, || {
        let collect = jsonc_parser::CollectOptions { comments: jsonc_parser::CommentCollectionStrategy::Off, tokens: false };
        let parsed = jsonc_parser::parse_to_ast(text, &collect, &Default::default()).map_err(|error| format!("JSONC parse error ({error})"))?;
        let value = parsed.value.ok_or("File contains no JSON value")?;
        let mut out = String::with_capacity(text.len());
        write_strict(&value, &mut out);
        Ok(std::borrow::Cow::Owned(out))
    })
}

/// A JSONC value as strict JSON text, keys in their order
fn write_strict(value: &jsonc_parser::ast::Value, out: &mut String) {
    use jsonc_parser::ast::{ObjectPropName, Value};
    match value {
        Value::Object(object) => {
            out.push('{');
            for (index, property) in object.properties.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let name = match &property.name {
                    ObjectPropName::String(literal) => literal.value.as_ref(),
                    ObjectPropName::Word(literal) => literal.value,
                };
                out.push_str(&serde_json::Value::String(name.to_string()).to_string());
                out.push(':');
                write_strict(&property.value, out);
            }
            out.push('}');
        }
        Value::Array(array) => {
            out.push('[');
            for (index, element) in array.elements.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_strict(element, out);
            }
            out.push(']');
        }
        leaf => out.push_str(&serde_json::Value::from(leaf.clone()).to_string()),
    }
}

/// A JSON file of Rojo's that deserializes into `T` (a meta file, a JSON model, a project): Rojo's `from_slice` (a value
/// read by [`read_json`], then serde_json's `from_value`: "Invalid JSON structure" when it does not fit).
pub(crate) fn read_json_as<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let value = read_json(bytes)?;
    serde_json::from_value(value).map_err(|error| format!("Invalid JSON structure ({error})"))
}

/// The Luau source `rojo build` writes for a TOML file, or its reason to refuse it (toml's error)
pub(crate) fn toml_module_source(bytes: &[u8]) -> Result<String, String> {
    if bracket_depth(bytes) > MAX_DEPTH {
        return Err(too_deep());
    }
    on_deep_stack(true, || {
        let value: toml::Value = toml::from_slice(bytes).map_err(|error| error.to_string())?;
        source_of(&toml_expression(value))
    })
}

/// The Luau source `rojo build` writes for a YAML file, or its reason to refuse it (as Rojo says it)
pub(crate) fn yaml_module_source(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "File was not valid UTF-8".to_string())?;
    let text = text.to_string();
    on_deep_stack(true, move || {
        let mut documents = yaml_rust2::YamlLoader::load_from_str(&text).map_err(|error| error.to_string())?;
        let document = documents.pop().ok_or("all YAML documents must contain a document")?;
        if !documents.is_empty() {
            return Err("Rojo does not currently support multiple documents in a YAML file".to_string());
        }
        if yaml_depth(&document) > MAX_DEPTH {
            return Err(too_deep());
        }
        source_of(&yaml_expression(document)?)
    })
}

/// Whether a reason this module gives is ree's nesting limit ([`MAX_DEPTH`]) rather than one of Rojo's
pub(crate) fn is_too_deep(reason: &str) -> bool {
    reason.starts_with("it nests more than")
}

fn too_deep() -> String {
    format!("it nests more than {MAX_DEPTH} levels deep, past what a client compiles; rojo build overflows its stack from about 1,300")
}

/// Runs `body` on a thread with a stack for [`MAX_DEPTH`] levels when `deep`, else here (the readers and the source writer
/// recurse once a level; so does dropping what they made)
fn on_deep_stack<R: Send>(deep: bool, body: impl FnOnce() -> R + Send) -> R {
    if !deep {
        return body();
    }
    std::thread::scope(|scope| std::thread::Builder::new().name("rojo data".into()).stack_size(64 << 20).spawn_scoped(scope, body).expect("a thread").join().expect("a data file read"))
}

/// Drops a value that may nest deeply where its recursive drop has the stack for it
fn drop_deep(value: serde_json::Value) {
    let deep = json_depth(&value) > SHALLOW;
    on_deep_stack(deep, move || drop(value));
}

fn memchr_find(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Whether the text has a `\u` escape of a UTF-16 surrogate (`\uD800`-`\uDFFF`), or what looks like one (an escaped backslash
/// before `ud800` is reported too, which only costs the lenient reader's time)
fn has_surrogate_escape(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|window| window[0] == b'\\' && window[1] == b'u' && matches!(window[2], b'd' | b'D') && matches!(window[3], b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F'))
}

/// How deeply JSONC text nests, by jsonc-parser's own scanner (no recursion), up to the first token it can't read: the
/// parser stops there too, so its recursion never goes deeper
fn jsonc_depth(text: &str) -> usize {
    use jsonc_parser::tokens::Token;
    let mut scanner = jsonc_parser::Scanner::new(text);
    let (mut depth, mut deepest) = (0usize, 0usize);
    while let Ok(Some(token)) = scanner.scan() {
        match token {
            Token::OpenBrace | Token::OpenBracket => {
                depth += 1;
                deepest = deepest.max(depth);
                if deepest > MAX_DEPTH {
                    break;
                }
            }
            Token::CloseBrace | Token::CloseBracket => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// How deeply a TOML file's brackets and braces nest outside its strings and comments (a table header's brackets open and
/// close on its line): an upper bound of how deeply its values nest
fn bracket_depth(bytes: &[u8]) -> usize {
    let (mut depth, mut deepest, mut i) = (0usize, 0usize, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            quote @ (b'"' | b'\'') => {
                let triple = bytes[i..].starts_with(if quote == b'"' { b"\"\"\"" } else { b"'''" });
                i += if triple { 3 } else { 1 };
                while i < bytes.len() {
                    if quote == b'"' && bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote && (!triple || bytes[i..].starts_with(if quote == b'"' { b"\"\"\"" } else { b"'''" })) {
                        i += if triple { 2 } else { 0 };
                        break;
                    }
                    if !triple && bytes[i] == b'\n' {
                        break;
                    }
                    i += 1;
                }
            }
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    deepest
}

/// How deeply a JSON value nests (a scalar is 0), without recursion
fn json_depth(value: &serde_json::Value) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(value, 0usize)];
    while let Some((value, depth)) = stack.pop() {
        match value {
            serde_json::Value::Array(items) => {
                deepest = deepest.max(depth + 1);
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            serde_json::Value::Object(map) => {
                deepest = deepest.max(depth + 1);
                stack.extend(map.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    deepest
}

/// Whether any string of a JSON value (a key or a value) holds NUL, without recursion
fn json_has_nul(value: &serde_json::Value) -> bool {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            serde_json::Value::String(text) if text.contains('\0') => return true,
            serde_json::Value::Array(items) => stack.extend(items),
            serde_json::Value::Object(map) => {
                if map.keys().any(|key| key.contains('\0')) {
                    return true;
                }
                stack.extend(map.values());
            }
            _ => {}
        }
    }
    false
}

/// How deeply a YAML document nests, without recursion
fn yaml_depth(value: &Yaml) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(value, 0usize)];
    while let Some((value, depth)) = stack.pop() {
        match value {
            Yaml::Array(items) => {
                deepest = deepest.max(depth + 1);
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Yaml::Hash(map) => {
                deepest = deepest.max(depth + 1);
                stack.extend(map.iter().flat_map(|(key, item)| [(key, depth + 1), (item, depth + 1)]));
            }
            _ => {}
        }
    }
    deepest
}

/// A data value as Rojo's lua_ast writes it
enum Lua {
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
    Table(Vec<(Lua, Lua)>),
    Array(Vec<Lua>),
}

/// Rojo's json_to_lua_value. (Keys in byte order, as serde_json's map holds them in Rojo's build, whatever this build's
/// serde_json features)
fn json_expression(value: &serde_json::Value) -> Lua {
    match value {
        serde_json::Value::Null => Lua::Nil,
        serde_json::Value::Bool(value) => Lua::Bool(*value),
        serde_json::Value::Number(value) => Lua::Number(value.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(value) => Lua::String(value.clone()),
        serde_json::Value::Array(items) => Lua::Array(items.iter().map(json_expression).collect()),
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
            Lua::Table(entries.into_iter().map(|(key, value)| (Lua::String(key.clone()), json_expression(value))).collect())
        }
    }
}

/// Rojo's toml_to_lua_value (tables by key in byte order, as toml's map holds them in Rojo's build)
fn toml_expression(value: toml::Value) -> Lua {
    match value {
        toml::Value::Datetime(value) => Lua::String(value.to_string()),
        toml::Value::Boolean(value) => Lua::Bool(value),
        toml::Value::Float(value) => Lua::Number(value),
        toml::Value::Integer(value) => Lua::Number(value as f64),
        toml::Value::String(value) => Lua::String(value),
        toml::Value::Array(items) => Lua::Array(items.into_iter().map(toml_expression).collect()),
        toml::Value::Table(map) => {
            let mut entries: Vec<(String, toml::Value)> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
            Lua::Table(entries.into_iter().map(|(key, value)| (Lua::String(key), toml_expression(value))).collect())
        }
    }
}

/// Rojo's yaml_to_luau, its errors in its words (the first in document order, a key before its value)
fn yaml_expression(value: Yaml) -> Result<Lua, String> {
    const MAX_FLOAT_INT: i64 = 1 << 53;
    Ok(match value {
        Yaml::String(text) => Lua::String(text),
        Yaml::Boolean(value) => Lua::Bool(value),
        Yaml::Integer(value) if value <= MAX_FLOAT_INT => Lua::Number(value as f64),
        Yaml::Integer(value) => return Err(format!("the integer '{value}' cannot be losslessly converted into a Luau number")),
        real @ Yaml::Real(_) => Lua::Number(real.as_f64().unwrap_or(f64::NAN)),
        Yaml::Null => Lua::Nil,
        Yaml::Array(items) => Lua::Array(items.into_iter().map(yaml_expression).collect::<Result<_, _>>()?),
        Yaml::Hash(map) => Lua::Table(map.into_iter().map(|(key, value)| Ok((yaml_expression(key)?, yaml_expression(value)?))).collect::<Result<_, String>>()?),
        Yaml::Alias(_) => return Err("Rojo cannot convert YAML aliases to Luau".to_string()),
        Yaml::BadValue => return Err("Rojo cannot convert YAML to Luau because of a parsing error".to_string()),
    })
}

/// `return <value>`, written as Rojo's LuaStream writes it; Err for a nil key, where Rojo panics
fn source_of(value: &Lua) -> Result<String, String> {
    let mut stream = Stream { out: String::new(), indent: 0, line_start: true };
    stream.write("return ");
    stream.expression(value)?;
    Ok(stream.out)
}

/// Rojo's LuaStream: text written a piece at a time, each line's indentation (tabs) written before its first text
struct Stream {
    out: String,
    indent: usize,
    line_start: bool,
}

impl Stream {
    fn write(&mut self, text: &str) {
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.out.push('\n');
                self.line_start = true;
            }
            if !line.is_empty() {
                if self.line_start {
                    self.line_start = false;
                    for _ in 0..self.indent {
                        self.out.push('\t');
                    }
                }
                self.out.push_str(line);
            }
        }
    }

    fn expression(&mut self, value: &Lua) -> Result<(), String> {
        match value {
            Lua::Nil => self.write("nil"),
            Lua::Bool(value) => self.write(if *value { "true" } else { "false" }),
            Lua::Number(value) => self.number(*value),
            Lua::String(text) => {
                self.write("\"");
                self.escaped(text);
                self.write("\"");
            }
            Lua::Array(items) => {
                self.write("{");
                for (index, item) in items.iter().enumerate() {
                    self.expression(item)?;
                    if index < items.len() - 1 {
                        self.write(", ");
                    }
                }
                self.write("}");
            }
            Lua::Table(entries) => {
                self.write("{\n");
                self.indent += 1;
                for (key, value) in entries {
                    self.key(key)?;
                    self.write(" = ");
                    self.expression(value)?;
                    self.write(",\n");
                }
                self.indent -= 1;
                self.write("}");
            }
        }
        Ok(())
    }

    fn key(&mut self, key: &Lua) -> Result<(), String> {
        match key {
            Lua::Nil => return Err("a mapping key is null (Rojo crashes: nil cannot be a table key)".to_string()),
            Lua::String(text) if is_identifier(text) => self.write(text),
            other => {
                self.write("[");
                self.expression(other)?;
                self.write("]");
            }
        }
        Ok(())
    }

    /// Rust's float Display (the shortest digits that read back, never an exponent), NaN as `0/0` and the infinities as
    /// `math.huge`
    fn number(&mut self, value: f64) {
        if value.is_nan() {
            self.write("0/0");
        } else if value.is_infinite() {
            self.write(if value > 0.0 { "math.huge" } else { "-math.huge" });
        } else {
            let mut text = String::new();
            let _ = write!(text, "{value}");
            self.write(&text);
        }
    }

    /// Lua's escapes for `"`, CR, LF, tab and `\` alone; every other character as it is
    fn escaped(&mut self, text: &str) {
        let mut escaped = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '"' => escaped.push_str("\\\""),
                '\r' => escaped.push_str("\\r"),
                '\n' => escaped.push_str("\\n"),
                '\t' => escaped.push_str("\\t"),
                '\\' => escaped.push_str("\\\\"),
                c => escaped.push(c),
            }
        }
        self.write(&escaped);
    }
}

/// Rojo's is_valid_ident: ASCII letters, digits and `_`, not starting with a digit, and none of Lua 5.1's keywords
/// (`continue`, `type`, `export` are identifiers)
fn is_identifier(text: &str) -> bool {
    const KEYWORDS: [&str; 21] = ["and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"];
    let mut chars = text.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && !KEYWORDS.contains(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_written_as_rojo_writes_them() {
        // (each source as `rojo build` 7.6.1 wrote it into the module's Source)
        let toml = toml_module_source(b"b = [1, 2.5, \"x\"]\na = { \"end\" = 1, ok = true, \"\" = nan }\nc = []\nd = [{ k = 1 }, { j = -inf }]\n").unwrap();
        assert_eq!(toml, "return {\n\ta = {\n\t\t[\"\"] = 0/0,\n\t\t[\"end\"] = 1,\n\t\tok = true,\n\t},\n\tb = {1, 2.5, \"x\"},\n\tc = {},\n\td = {{\n\t\tk = 1,\n\t}, {\n\t\tj = -math.huge,\n\t}},\n}");
        assert_eq!(toml_module_source(b"").unwrap(), "return {\n}");
        let yaml = yaml_module_source(b"z: 1\n1: [~, \"q\\\"\\t\"]\n{a: 1}: x\n").unwrap();
        assert_eq!(yaml, "return {\n\tz = 1,\n\t[1] = {nil, \"q\\\"\\t\"},\n\t[{\n\t\ta = 1,\n\t}] = \"x\",\n}");
        assert_eq!(json_module_source(b"[1e300, -0, 0.1, \"a\\u0000b\"]").unwrap(), format!("return {{{}, -0, 0.1, \"a\0b\"}}", 1e300));
        assert_eq!(json_module_source(b"null").unwrap(), "return nil");
    }

    #[test]
    fn json_reads_as_rojo_reads_it_and_refuses_what_it_refuses() {
        let read = |text: &str| read_json(text.as_bytes());
        assert_eq!(read("// c\n{a: 1, 'b': 'x', c: [1 2,], d: 0x10, e: +1, f: 1e400,} /* end */").unwrap(), serde_json::json!({"a": 1, "b": "x", "c": [1, 2], "d": 16, "e": 1, "f": "1e400"}));
        assert_eq!(read("{\"a\": \"x\ty\"}").unwrap(), serde_json::json!({"a": "x\ty"}));
        assert_eq!(read("").unwrap_err(), "File contains no JSON value");
        assert_eq!(read("// only\n").unwrap_err(), "File contains no JSON value");
        assert_eq!(read("{\"a\": NaN}").unwrap_err(), "JSONC parse error (Unexpected token on line 1 column 7)");
        assert_eq!(read("1 2").unwrap_err(), "JSONC parse error (Text cannot contain more than one JSON value on line 1 column 3)");
        assert_eq!(read("\u{feff}{}").unwrap_err(), "JSONC parse error (Unexpected token on line 1 column 1)");
        // (strict JSON serde_json reads, which jsonc-parser 0.27.0 refuses)
        assert_eq!(read("{\"a\": \"\\ud83d\\ude00\"}").unwrap_err(), "JSONC parse error (Invalid unicode escape sequence. 'd83d' is not a valid UTF8 character on line 1 column 8)");
        assert!(read_json(&[b'"', 0xff, b'"']).unwrap_err().starts_with("File is not valid UTF-8"));
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(read(&deep).unwrap_err().contains("nests more than 1000 levels"));
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert_eq!(json_depth(&read(&deep).unwrap()), MAX_DEPTH);
        assert!(json_module(deep.as_bytes()).unwrap().is_none());
        assert!(json_module(b"{\"a\": \"x\\u0000\"}").unwrap().is_none());
        assert!(json_module(b"{\"a\": \"x\\\\u0000\"}").unwrap().is_some());
        // (a project file's keys in the file's order)
        assert_eq!(read_json_text(b"{\"z\": 1, \"a\": 2}").unwrap(), "{\"z\": 1, \"a\": 2}");
        assert_eq!(read_json_text(b"{z: 0x10, 'a': [1e400, +1,], /* c */}").unwrap(), "{\"z\":16,\"a\":[\"1e400\",1]}");
    }

    #[test]
    fn toml_and_yaml_refuse_what_rojo_refuses_in_its_words() {
        assert_eq!(toml_module_source(b"a = 9223372036854775808\n").unwrap_err(), "invalid number at line 1 column 5");
        assert_eq!(toml_module_source(b"a = 1\na = 2\n").unwrap_err(), "duplicate key: `a` at line 1 column 1");
        assert_eq!(yaml_module_source(b"").unwrap_err(), "all YAML documents must contain a document");
        assert_eq!(yaml_module_source(b"a: 1\n---\nb: 2\n").unwrap_err(), "Rojo does not currently support multiple documents in a YAML file");
        assert_eq!(yaml_module_source(b"a: 9007199254740993\n").unwrap_err(), "the integer '9007199254740993' cannot be losslessly converted into a Luau number");
        assert_eq!(yaml_module_source(b"a: -9007199254740993\n").unwrap(), "return {\n\ta = -9007199254740992,\n}");
        assert_eq!(yaml_module_source(b"a: !!int abc\n").unwrap_err(), "Rojo cannot convert YAML to Luau because of a parsing error");
        assert_eq!(yaml_module_source(b"~: x\n").unwrap_err(), "a mapping key is null (Rojo crashes: nil cannot be a table key)");
        assert_eq!(yaml_module_source(b"a: 1\nb: 2\na: 3\n").unwrap_err(), "String(\"a\"): duplicated key in mapping at byte 13 line 3 column 4");
        assert_eq!(yaml_module_source(&[b'a', b':', b' ', 0xff]).unwrap_err(), "File was not valid UTF-8");
    }
}
