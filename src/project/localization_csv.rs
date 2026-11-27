//! Rojo's localization CSV files (`*.csv`, a directory's `init.csv`): a LocalizationTable whose Contents is the JSON of the
//! file's entries, as `rojo build` 7.6.1 and 7.7.0 write it (Rojo's snapshot_middleware/csv.rs, which reads the file with the
//! csv crate's default reader). Measured against both releases' builds of a fixture of 39 tables (every Contents byte for
//! byte) and of the files they refuse (each error text):
//!
//! - The reader: a comma between fields; a record ends at `\r`, `\n` or `\r\n`; empty lines make no record; a field that starts
//!   with `"` is quoted until the next lone `"` (`""` inside is one `"`, delimiters and line breaks are text), and what follows
//!   its closing quote up to the next delimiter is text too (`"ab"cd` reads `abcd`); a `"` anywhere else is text; a quoted field
//!   the file ends in reads to the end. A UTF-8 byte order mark at the start is skipped. Nothing is trimmed.
//! - The first record names the columns, and every record must have as many fields (`CSV error: record 1 (line: 2, byte: 14):
//!   found record with 2 fields, but the previous record has 3 fields`); every field must be UTF-8 (`CSV parse error: record 1
//!   (line 2, field: 1, byte: 14): invalid utf-8: invalid UTF-8 in field 1 near byte index 0`). A record's position is where
//!   the reader stood when it started it: after the line break that ended the record before (after the `\r` of a `\r\n`, before
//!   any empty lines), the header at byte 0 with or without a byte order mark; lines count from 1.
//! - An entry per record: the columns `Key`, `Source`, `Context` and `Example` (exactly so spelled) give its fields, any other
//!   column (`en`, `es-ES`, `Notes`, `key`) a value under that name; an empty field or a column with no name is left out, and a
//!   later column of one name wins. A record with neither a Key nor a Source makes no entry.
//! - Contents is `[{"key":..,"context":..,"example":..,"source":..,"values":{..}}, ...]` with the absent fields left out, the
//!   values by name in byte order, and serde_json's string escapes (`\u0001`, `\t`, non-ASCII kept as it is).

/// A record as the reader read it: its fields' bytes, and where it started
struct Record {
    fields: Vec<Vec<u8>>,
    byte: usize,
    line: usize,
}

/// The records of a CSV file, as the csv crate's default reader reads them (it refuses nothing at this level)
fn records(bytes: &[u8]) -> Vec<Record> {
    #[derive(Clone, Copy, PartialEq)]
    enum At {
        FieldStart,
        Unquoted,
        Quoted,
        AfterQuote,
    }
    let mut records = Vec::new();
    let mut line = 1;
    // a byte order mark is skipped, after the header's position (byte 0)
    let mut pos = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
    let mut first = true;
    loop {
        let start = (if first { 0 } else { pos }, line);
        first = false;
        // the `\n` of a `\r\n` that ended the last record, then empty lines
        while pos < bytes.len() && matches!(bytes[pos], b'\r' | b'\n') {
            line += (bytes[pos] == b'\n') as usize;
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let mut fields = Vec::new();
        let mut field = Vec::new();
        let mut at = At::FieldStart;
        loop {
            let Some(&byte) = bytes.get(pos) else {
                fields.push(std::mem::take(&mut field));
                break;
            };
            pos += 1;
            if byte == b'\n' {
                line += 1;
            }
            match (at, byte) {
                (At::FieldStart, b'"') => at = At::Quoted,
                (At::Quoted, b'"') => at = At::AfterQuote,
                (At::Quoted, _) => field.push(byte),
                (At::AfterQuote, b'"') => {
                    field.push(b'"');
                    at = At::Quoted;
                }
                (_, b',') => {
                    fields.push(std::mem::take(&mut field));
                    at = At::FieldStart;
                }
                (_, b'\r' | b'\n') => {
                    fields.push(std::mem::take(&mut field));
                    break;
                }
                _ => {
                    field.push(byte);
                    at = At::Unquoted;
                }
            }
        }
        records.push(Record { fields, byte: start.0, line: start.1 });
    }
    records
}

/// One entry of the table, as Rojo's LocalizationEntry serializes it
#[derive(Default)]
struct Entry<'a> {
    key: Option<&'a str>,
    context: Option<&'a str>,
    example: Option<&'a str>,
    source: Option<&'a str>,
    values: std::collections::BTreeMap<&'a str, &'a str>,
}

fn json(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

/// The Contents Rojo gives the LocalizationTable of a CSV file of these bytes, or the reader's error, as Rojo reports it
/// ("File was not a valid LocalizationTable CSV file" and this text)
pub fn contents(bytes: &[u8]) -> Result<String, String> {
    let records = records(bytes);
    let mut texts: Vec<Vec<&str>> = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        if let Some(first) = records.first().filter(|first| first.fields.len() != record.fields.len()) {
            return Err(format!(
                "CSV error: record {index} (line: {}, byte: {}): found record with {} fields, but the previous record has {} fields",
                record.line,
                record.byte,
                record.fields.len(),
                first.fields.len()
            ));
        }
        let mut fields = Vec::with_capacity(record.fields.len());
        for (number, field) in record.fields.iter().enumerate() {
            match std::str::from_utf8(field) {
                Ok(text) => fields.push(text),
                Err(error) => {
                    return Err(format!(
                        "CSV parse error: record {index} (line {}, field: {number}, byte: {}): invalid utf-8: invalid UTF-8 in field {number} near byte index {}",
                        record.line,
                        record.byte,
                        error.valid_up_to()
                    ));
                }
            }
        }
        texts.push(fields);
    }
    let mut out = String::from("[");
    let Some((headers, rows)) = texts.split_first() else { return Ok("[]".into()) };
    for row in rows {
        let mut entry = Entry::default();
        for (&header, &value) in headers.iter().zip(row) {
            if header.is_empty() || value.is_empty() {
                continue;
            }
            match header {
                "Key" => entry.key = Some(value),
                "Source" => entry.source = Some(value),
                "Context" => entry.context = Some(value),
                "Example" => entry.example = Some(value),
                _ => {
                    entry.values.insert(header, value);
                }
            }
        }
        if entry.key.is_none() && entry.source.is_none() {
            continue;
        }
        if out.len() > 1 {
            out.push(',');
        }
        out.push('{');
        for (name, field) in [("key", entry.key), ("context", entry.context), ("example", entry.example), ("source", entry.source)] {
            if let Some(field) = field {
                out.push_str(&format!("\"{name}\":{},", json(field)));
            }
        }
        out.push_str("\"values\":{");
        for (place, (name, value)) in entry.values.iter().enumerate() {
            if place > 0 {
                out.push(',');
            }
            out.push_str(&format!("{}:{}", json(name), json(value)));
        }
        out.push_str("}}");
    }
    out.push(']');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::contents;

    /// Files and the Contents `rojo build` 7.6.1 and 7.7.0 wrote for them (the same bytes from both)
    #[test]
    fn csv_files_read_as_rojo_writes_their_tables() {
        let cases: &[(&[u8], &str)] = &[
            (b"", "[]"),
            (b"Key,Source,Context,Example,en\n", "[]"),
            (b"\n\nKey,Source,en\nHB,hb,v\n", r#"[{"key":"HB","source":"hb","values":{"en":"v"}}]"#),
            (
                b"Key,Source,Context,Example,en,es\nHello,Hello,,,Hello,Hola\nBye,Goodbye,ctx,an example,Bye,Adios\n",
                r#"[{"key":"Hello","source":"Hello","values":{"en":"Hello","es":"Hola"}},{"key":"Bye","context":"ctx","example":"an example","source":"Goodbye","values":{"en":"Bye","es":"Adios"}}]"#,
            ),
            (
                b"Key,Source,Context,Example,en\n\"Q1\",\"a, b\",,,\"say \"\"hi\"\"\"\nQ2,\"two\nlines\",,,\"x\r\ny\"\n",
                r#"[{"key":"Q1","source":"a, b","values":{"en":"say \"hi\""}},{"key":"Q2","source":"two\nlines","values":{"en":"x\r\ny"}}]"#,
            ),
            (b"Key,Source,Context,Example,en\r\nC1,Crlf,,,c\r\n", r#"[{"key":"C1","source":"Crlf","values":{"en":"c"}}]"#),
            (b"Key,Source,en\rC,c,v\rD,d,w\r", r#"[{"key":"C","source":"c","values":{"en":"v"}},{"key":"D","source":"d","values":{"en":"w"}}]"#),
            (b"\xEF\xBB\xBFKey,Source,Context,Example,en\nB1,Bom,,,b\n", r#"[{"key":"B1","source":"Bom","values":{"en":"b"}}]"#),
            (b"\xEF\xBB\xBF\"Key\",Source,en\nBQ,s,v\n", r#"[{"key":"BQ","source":"s","values":{"en":"v"}}]"#),
            (b"Key,Source,Context,Example,en\n,,ctx,ex,v\nK,,,,v\n,S,,,v\n", r#"[{"key":"K","values":{"en":"v"}},{"source":"S","values":{"en":"v"}}]"#),
            (b"Key,Source,Notes,en-us,ES_es\nU,Src,note,a,b\n", r#"[{"key":"U","source":"Src","values":{"ES_es":"b","Notes":"note","en-us":"a"}}]"#),
            (b"Key,Source,,en\nB,Src,dropped,v\n", r#"[{"key":"B","source":"Src","values":{"en":"v"}}]"#),
            (b"Key,Source,Context,Example,en\n K , S ,,, v \n", r#"[{"key":" K ","source":" S ","values":{"en":" v "}}]"#),
            (b"Key,Source,en\nM,a\"b,v\n", r#"[{"key":"M","source":"a\"b","values":{"en":"v"}}]"#),
            (b"Key,Source,en\nT,t,v", r#"[{"key":"T","source":"t","values":{"en":"v"}}]"#),
            (b"Key,Source,en\n\nL,l,v\n\n", r#"[{"key":"L","source":"l","values":{"en":"v"}}]"#),
            (b"Source,en\nOnly,o\n", r#"[{"source":"Only","values":{"en":"o"}}]"#),
            (b"Key,Source,zh-cn,de,fr,en,pt-br\nML,ml,1,2,3,4,5\n", r#"[{"key":"ML","source":"ml","values":{"de":"2","en":"4","fr":"3","pt-br":"5","zh-cn":"1"}}]"#),
            (b"Key,Source,en\nD,one,1\nD,two,2\n", r#"[{"key":"D","source":"one","values":{"en":"1"}},{"key":"D","source":"two","values":{"en":"2"}}]"#),
            (b"Key,Source,en\nA,\"ab\"cd,v\n", r#"[{"key":"A","source":"abcd","values":{"en":"v"}}]"#),
            (b"Key,Source,en\nU,u,\"open\nstill", r#"[{"key":"U","source":"u","values":{"en":"open\nstill"}}]"#),
            (b"Key,Source,en\nL, \"a\",v\n", r#"[{"key":"L","source":" \"a\"","values":{"en":"v"}}]"#),
            (b"Key,Source,en\n\"\",S,\"\"\n", r#"[{"source":"S","values":{}}]"#),
            (b"Key,Source,en,en,Key\nK1,S,first,second,K2\n", r#"[{"key":"K2","source":"S","values":{"en":"second"}}]"#),
            ("Key,Source,en\nE,\"tab\there/slash\x08ack\x01ctl\",\"日本 😀 é\"\n".as_bytes(), r#"[{"key":"E","source":"tab\there/slash\back\u0001ctl","values":{"en":"日本 😀 é"}}]"#),
            (b"key,Source,en\nlk,s,v\n", r#"[{"source":"s","values":{"en":"v","key":"lk"}}]"#),
            (b"Key,Source,en\n\"a,b\",\"c\"\"d\",\"\"\"\"\n", r#"[{"key":"a,b","source":"c\"d","values":{"en":"\""}}]"#),
            (b"Key,Source,en\n,,\nR,r,v\n", r#"[{"key":"R","source":"r","values":{"en":"v"}}]"#),
        ];
        for (bytes, expected) in cases {
            assert_eq!(contents(bytes).as_deref(), Ok(*expected), "{:?}", String::from_utf8_lossy(bytes));
        }
    }

    /// The files Rojo refuses, with the reader's error it gives (the text after "Caused by:")
    #[test]
    fn csv_files_rojo_refuses_give_its_reasons() {
        let cases: &[(&[u8], &str)] = &[
            (b"Key,Source,en\nA,a\nB,b,c,d\n", "CSV error: record 1 (line: 2, byte: 14): found record with 2 fields, but the previous record has 3 fields"),
            (b"Key,Source,en\nS,s,v\n   \n", "CSV error: record 2 (line: 3, byte: 20): found record with 1 fields, but the previous record has 3 fields"),
            (b"\xEF\xBB\xBFKey,Source\r\nA,a,b\r\n", "CSV error: record 1 (line: 1, byte: 14): found record with 3 fields, but the previous record has 2 fields"),
            (b"Key,Source,en\n\"multi\nline\",a,b\nX,y\n", "CSV error: record 2 (line: 4, byte: 31): found record with 2 fields, but the previous record has 3 fields"),
            (b"Key\n\xFF\xFE\n", "CSV parse error: record 1 (line 2, field: 0, byte: 4): invalid utf-8: invalid UTF-8 in field 0 near byte index 0"),
            (b"Key,Source,en\nK,\xFF,v\n", "CSV parse error: record 1 (line 2, field: 1, byte: 14): invalid utf-8: invalid UTF-8 in field 1 near byte index 0"),
            (b"Key,\xC3,en\nK,S,v\n", "CSV parse error: record 0 (line 1, field: 1, byte: 0): invalid utf-8: invalid UTF-8 in field 1 near byte index 0"),
            (b"\xEF\xBB\xBFKey,Source,\xFF\nK,S,v\n", "CSV parse error: record 0 (line 1, field: 2, byte: 0): invalid utf-8: invalid UTF-8 in field 2 near byte index 0"),
            (b"Key,Source,en\nK,\xC3,\xA9\n", "CSV parse error: record 1 (line 2, field: 1, byte: 14): invalid utf-8: invalid UTF-8 in field 1 near byte index 0"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(contents(bytes).err().as_deref(), Some(*expected), "{:?}", String::from_utf8_lossy(bytes));
        }
    }
}
