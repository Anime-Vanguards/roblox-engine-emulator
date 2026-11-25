//! LocalizationTable, Translator and LocalizationService's translators, as a playing client has them.
//!
//! Measured in roblox-sim's 0.747.0.7471481 play sessions, a client and a server at script identity 2 alike (the parity probes
//! tools/parity/probes/localization_entries.luau, localization_templates.luau, localization_lookup.luau and
//! localization_formats.luau; docs/engine.md, Log 2026-11-25):
//!
//! - A table's entries: SetEntries takes an array of dictionaries with string Key, Source, Context and Example (absent ones
//!   empty) and a Values dictionary of strings by locale, and refuses the whole call at the first bad entry with the client's
//!   texts ("entries was not an array.", "Entry at index 2 was not a table.", "... had was missing \"Values\"." and the rest:
//!   [`entries_from_lua`]). An entry is named by its Key, or with no Key by its Source and Context; a second keyed entry of a
//!   key raises, a second keyless one of a Source and Context is dropped. Locale ids are kept lower case with `-` (`ES_es` is
//!   `es-es`), the first of two that end up equal, in key order, kept. Sources are templates and are checked
//!   ([`parse_template`]); `{}` alone is an empty Source. GetEntries lists the entries grouped by Source, the groups in the
//!   order they were made (a group stays when its last entry goes), each sorted by Context, then Key: [`Entries`].
//! - The entry methods (SetEntry, SetEntryValue, SetEntryContext / Key / Source / Example, RemoveEntry, RemoveEntryValue,
//!   RemoveKey, RemoveTargetLocale, GetString) find an entry as above and, but for SetEntryExample, make it when it is
//!   missing; a change that would give an entry another's identity raises "The modified entry would conflict with an existing
//!   entry.", one that leaves it with no Key and no Source loses it ("Failed to add entry to index after modifying.").
//! - A table keeps its entries in its Contents (hidden from scripts, as on a client), the JSON GetContents returns; SetContents
//!   reads that JSON with RapidJSON's errors, a copy (Clone) reads its own, losing the Examples a client's reader drops too.
//!   Writes fire no Changed. ree keeps the entries a write made beside the Contents it wrote ([`Localization`]).
//! - A Translator is a new instance each call, named "Instance", its Parent locked by class, its LocaleId read-only; Clone
//!   gives nil. A table's translator (GetTranslator) reads the table and then the tables it is in, up to the first ancestor
//!   that is no table; LocalizationService's (GetTranslatorForLocaleAsync, GetTranslatorForPlayer) the tables in the service
//!   that only tables hold (one in a Folder or a Model is not read), each table's own entries after the tables in it. Both
//!   read their tables as they are at each call.
//! - FormatByKey takes the first table with a translation of the key for the translator's locale ([`translation`]: the
//!   locale's, its language's, a paired locale's, another of its language's), or whose source locale has the translator's
//!   language and a Source for it; else "Key K not found for locale L". The text is then formatted with the arguments
//!   ([`render`]: `{1}` from an array, `{name}` from a dictionary, the formats of `{1:int}` and the rest, with the client's
//!   errors). Translate(context, text) answers the translation of the entry with that Source whose Context best fits the
//!   context's full name, else the text.
//! - LocalizationService: RobloxLocaleId and SystemLocaleId "en-us"; GetTranslatorForPlayer gives the player's LocaleId (""
//!   in an offline client, as in ree); GetTranslatorForPlayerAsync raises "Publishing the game is required to use
//!   GetTranslatorForPlayerAsync API." in a place with no GameId; GetTranslatorForLocaleAsync takes two characters or more.
//!   GetTableEntries lists the service's tables' entries and an empty list for the experience's own table. ree answers the
//!   Async calls at once (a client offline waits ten seconds for the experience's table).
//! - Roblox's own members (RobloxOnlyTranslate, the CSV prompts, the text scraper, ...) raise the capability error.

use std::{cell::OnceCell, rc::Rc};

use mlua::prelude::*;
use rustc_hash::FxHashMap;

use super::{Behaviors, StringParam, method, strings, values};
use crate::{
    ClassId, Engine, InstanceId, NameId, Reflection, State,
    instance as core,
    lua::{Args, Flow, Ret, bindable::Refused, instance_value, none, raise, typeof_value},
    value::{Str, Value},
};

type Bytes = Vec<u8>;

/// One entry of a table: what GetEntries lists (`Example` is `examples` in the table's contents)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub key: Bytes,
    pub source: Bytes,
    pub context: Bytes,
    pub example: Bytes,
    /// (locale, text), by locale
    pub values: Vec<(Bytes, Bytes)>,
}

impl Entry {
    fn new(key: &[u8], source: &[u8], context: &[u8]) -> Entry {
        Entry { key: key.to_vec(), source: source.to_vec(), context: context.to_vec(), ..Entry::default() }
    }

    pub fn value(&self, locale: &[u8]) -> Option<&[u8]> {
        self.values.binary_search_by(|(held, _)| held.as_slice().cmp(locale)).ok().map(|at| self.values[at].1.as_slice())
    }

    fn set_value(&mut self, locale: Bytes, text: Bytes) {
        match self.values.binary_search_by(|(held, _)| held.as_slice().cmp(&locale)) {
            Ok(at) => self.values[at].1 = text,
            Err(at) => self.values.insert(at, (locale, text)),
        }
    }

    fn remove_value(&mut self, locale: &[u8]) {
        if let Ok(at) = self.values.binary_search_by(|(held, _)| held.as_slice().cmp(locale)) {
            self.values.remove(at);
        }
    }

    /// Where the entry sorts in its group: by Context, then Key
    fn order(&self) -> (&[u8], &[u8]) {
        (&self.context, &self.key)
    }
}

/// A table's entries in GetEntries' order: groups of one Source each, in the order the groups were made (a group stays when
/// its last entry goes), each group's entries sorted by Context, then Key.
#[derive(Clone, Debug, Default)]
pub struct Entries {
    groups: Vec<Group>,
}

#[derive(Clone, Debug)]
struct Group {
    source: Bytes,
    entries: Vec<Entry>,
}

/// Where an entry is: (group, place in it)
type At = (usize, usize);

impl Entries {
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.groups.iter().flat_map(|group| group.entries.iter())
    }

    fn len(&self) -> usize {
        self.groups.iter().map(|group| group.entries.len()).sum()
    }

    fn at(&self, (group, place): At) -> &Entry {
        &self.groups[group].entries[place]
    }

    fn at_mut(&mut self, (group, place): At) -> &mut Entry {
        &mut self.groups[group].entries[place]
    }

    /// The entry with this key
    fn find_key(&self, key: &[u8]) -> Option<At> {
        for (g, group) in self.groups.iter().enumerate() {
            if let Some(p) = group.entries.iter().position(|entry| entry.key == key) {
                return Some((g, p));
            }
        }
        None
    }

    /// The entry a (key, source, context) names: the one with the key when the key is not empty, else the one of that source
    /// and context that has no key
    fn find(&self, key: &[u8], source: &[u8], context: &[u8]) -> Option<At> {
        if !key.is_empty() {
            return self.find_key(key);
        }
        let g = self.groups.iter().position(|group| group.source == source)?;
        let p = self.groups[g].entries.iter().position(|entry| entry.key.is_empty() && entry.context == context)?;
        Some((g, p))
    }

    /// The entries whose Source is `text`, in their order
    fn with_source(&self, text: &[u8]) -> &[Entry] {
        self.groups.iter().find(|group| group.source == text).map_or(&[], |group| group.entries.as_slice())
    }

    /// Files `entry` in its source's group (made last when there is none) at its (context, key) place
    fn insert(&mut self, entry: Entry) -> At {
        let g = match self.groups.iter().position(|group| group.source == entry.source) {
            Some(g) => g,
            None => {
                self.groups.push(Group { source: entry.source.clone(), entries: Vec::new() });
                self.groups.len() - 1
            }
        };
        let entries = &mut self.groups[g].entries;
        let p = entries.partition_point(|held| held.order() < entry.order());
        entries.insert(p, entry);
        (g, p)
    }

    fn remove(&mut self, (group, place): At) -> Entry {
        self.groups[group].entries.remove(place)
    }

    /// An entry changed (`edit`) and filed again where its new Source, Context and Key put it; Err, the entry dropped, when it is
    /// left with no key and no Source. A Source no group has yet takes over the entry's old group when the entry was all it
    /// held (measured: the entry keeps its group's place); else the entry goes to its Source's group, made last when there is
    /// none
    fn refile(&mut self, at: At, edit: impl FnOnce(&mut Entry)) -> Result<(), ()> {
        let mut entry = self.remove(at);
        edit(&mut entry);
        if entry.key.is_empty() && entry.source.is_empty() {
            return Err(());
        }
        let (group, _) = at;
        if self.groups[group].entries.is_empty() && self.groups[group].source != entry.source && !self.groups.iter().any(|held| held.source == entry.source) {
            self.groups[group].source = entry.source.clone();
        }
        self.insert(entry);
        Ok(())
    }
}

/// A table's entries as its contents last held them, and its key index (made on the first lookup by key)
pub struct Table {
    contents: Str,
    pub entries: Entries,
    keys: OnceCell<FxHashMap<Bytes, At>>,
}

impl Table {
    fn new(contents: Str, entries: Entries) -> Table {
        Table { contents, entries, keys: OnceCell::new() }
    }

    pub fn by_key(&self, key: &[u8]) -> Option<&Entry> {
        let keys = self.keys.get_or_init(|| {
            let mut keys = FxHashMap::default();
            for (g, group) in self.entries.groups.iter().enumerate() {
                for (p, entry) in group.entries.iter().enumerate() {
                    if !entry.key.is_empty() {
                        keys.entry(entry.key.clone()).or_insert((g, p));
                    }
                }
            }
            keys
        });
        keys.get(key).map(|&at| self.entries.at(at))
    }
}

/// What a Translator translates from: one table (and the tables it is in), or the LocalizationService's tables
#[derive(Clone, Copy, Debug)]
enum Scope {
    Table(InstanceId),
    Service,
}

/// The engine's localization state: each table's entries, read from its contents and kept while they are its contents, and
/// what each Translator translates from
#[derive(Default)]
pub struct Localization {
    tables: FxHashMap<InstanceId, Rc<Table>>,
    translators: FxHashMap<InstanceId, Scope>,
}

/// The classes and members the rules read, interned once
#[derive(Clone, Copy)]
pub struct Names {
    table: ClassId,
    translator: ClassId,
    service: ClassId,
    player: ClassId,
    contents: NameId,
    source_locale: NameId,
    locale_id: NameId,
}

impl Names {
    pub fn new(reflection: &Reflection) -> Option<Names> {
        Some(Names {
            table: reflection.class_id("LocalizationTable")?,
            translator: reflection.class_id("Translator")?,
            service: reflection.class_id("LocalizationService")?,
            player: reflection.class_id("Player")?,
            contents: reflection.name_id("Contents")?,
            source_locale: reflection.name_id("SourceLocaleId")?,
            locale_id: reflection.name_id("LocaleId")?,
        })
    }
}

/// A locale id as a client keeps it: ASCII letters lower-cased and `_` made `-` (`EN_us` is `en-us`), nothing else checked
pub fn locale(text: &[u8]) -> Bytes {
    text.iter().map(|&byte| if byte == b'_' { b'-' } else { byte.to_ascii_lowercase() }).collect()
}

// Contents: the JSON a table keeps its entries in (GetContents, SetContents, a copy's entries)

fn json_string(out: &mut Bytes, text: &[u8]) {
    out.push(b'"');
    for &byte in text {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            0..0x20 => out.extend_from_slice(format!("\\u{byte:04X}").as_bytes()),
            _ => out.push(byte),
        }
    }
    out.push(b'"');
}

/// The contents a client writes for `entries` (measured: `[{"key":"k","context":"c","examples":"e","source":"s","values":
/// {"es-es":"v"}},{"source":"free","values":{}}]`, an empty field left out, the values by locale)
pub fn contents_json(entries: &Entries) -> Bytes {
    let mut out = b"[".to_vec();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            out.push(b',');
        }
        out.push(b'{');
        for (name, text) in [(&b"key"[..], &entry.key), (b"context", &entry.context), (b"examples", &entry.example), (b"source", &entry.source)] {
            if !text.is_empty() {
                json_string(&mut out, name);
                out.push(b':');
                json_string(&mut out, text);
                out.push(b',');
            }
        }
        out.extend_from_slice(b"\"values\":{");
        for (place, (locale, text)) in entry.values.iter().enumerate() {
            if place > 0 {
                out.push(b',');
            }
            json_string(&mut out, locale);
            out.push(b':');
            json_string(&mut out, shown(text));
        }
        out.extend_from_slice(b"}}");
    }
    out.push(b']');
    out
}

/// A JSON text's value, or the parse error a client names (RapidJSON's texts: measured, an empty text, one followed by more, and
/// any other fault as "Unspecific syntax error.")
fn parse_json(text: &[u8]) -> Result<serde_json::Value, &'static str> {
    if text.iter().all(u8::is_ascii_whitespace) {
        return Err("The document is empty.");
    }
    let mut stream = serde_json::Deserializer::from_slice(text).into_iter::<serde_json::Value>();
    match stream.next() {
        Some(Ok(value)) if text[stream.byte_offset()..].iter().all(u8::is_ascii_whitespace) => Ok(value),
        Some(Ok(_)) => Err("The document root must not be followed by other values."),
        _ => Err("Unspecific syntax error."),
    }
}

/// The entries of a table's contents as SetContents reads them (a copy's are read the same way), with the client's errors
/// (measured): an array of objects whose string `key`, `source`, `context` and `example` it takes (anything else in them, the
/// `examples` GetContents writes among them, is dropped) and whose `values` object of strings by locale it takes (locale ids
/// made lower case with `-`, the first of those that end up equal kept, an empty one too), the entries filed as SetEntries
/// files them
pub fn parse_contents(text: &[u8]) -> Result<Entries, String> {
    let json = parse_json(text).map_err(|problem| format!("JSON parse error: {problem}"))?;
    let serde_json::Value::Array(items) = json else { return Err("Root is not an array".into()) };
    let mut entries = Builder::default();
    for (index, item) in items.into_iter().enumerate() {
        let serde_json::Value::Object(fields) = item else { return Err("Entry is not an object".into()) };
        let text = |name: &str| match fields.get(name) {
            Some(serde_json::Value::String(text)) => text.as_bytes().to_vec(),
            _ => Vec::new(),
        };
        let values = match fields.get("values") {
            None => return Err("Entry does not have \"values\"".into()),
            Some(serde_json::Value::Object(values)) => values,
            Some(_) => return Err("Entry \"values\" is not an object".into()),
        };
        let mut entry = Entry::new(&text("key"), &text("source"), &text("context"));
        entry.example = text("example");
        if entry.key.is_empty() && entry.source.is_empty() {
            return Err("Cannot create entry with empty source text and no key.".into());
        }
        if let Err(problem) = parse_template(&entry.source) {
            return Err(format!("Entry had invalid source \"{}\": {}", lossy(&entry.source), problem.text(&entry.source)));
        }
        // (a JSON object's members by name, as a client's map keeps them)
        for (name, value) in values {
            let serde_json::Value::String(value) = value else { return Err("Entry values value is not a string".into()) };
            let id = locale(name.as_bytes());
            if entry.value(&id).is_none() {
                entry.set_value(id, value.as_bytes().to_vec());
            }
        }
        entries.add(entry, index + 1)?;
    }
    Ok(entries.done())
}

/// A table's entries as SetEntries and SetContents file a list of them, each in a step (keys and Sources looked up by hash: a
/// list of thousands is read in one pass): a second entry with a key another has is refused, a second keyless entry of the same
/// Source and Context is dropped
#[derive(Default)]
struct Builder {
    groups: Vec<Group>,
    by_source: FxHashMap<Bytes, usize>,
    keys: rustc_hash::FxHashSet<Bytes>,
    free: rustc_hash::FxHashSet<(Bytes, Bytes)>,
}

impl Builder {
    /// Files `entry`, the call's entry `index`, or the client's error for it
    fn add(&mut self, entry: Entry, index: usize) -> Result<(), String> {
        if entry.key.is_empty() {
            if !self.free.insert((entry.source.clone(), entry.context.clone())) {
                return Ok(());
            }
        } else if !self.keys.insert(entry.key.clone()) {
            return Err(format!(
                "Entry at index {index} has the same (key) or (key,source,context) tuple as another entry. key: \"{}\", source: \"{}\", context: \"{}\"",
                lossy(&entry.key),
                lossy(&entry.source),
                lossy(&entry.context)
            ));
        }
        let group = *self.by_source.entry(entry.source.clone()).or_insert_with(|| {
            self.groups.push(Group { source: entry.source.clone(), entries: Vec::new() });
            self.groups.len() - 1
        });
        self.groups[group].entries.push(entry);
        Ok(())
    }

    /// The entries, each group sorted by Context, then Key
    fn done(mut self) -> Entries {
        for group in &mut self.groups {
            crate::sort::by(&mut group.entries, |a, b| a.order().cmp(&b.order()));
        }
        Entries { groups: self.groups }
    }
}

// Parameter templates: a Source or a translation with `{1}` / `{name}` parameters, `{1:int}` formats and `{{` / `}}` escapes

/// A parameter's place in the arguments
#[derive(Clone, Debug, PartialEq)]
enum Param {
    /// `{1}`: the arguments' first (an array's)
    Index(usize),
    /// `{name}`: a dictionary's `name`
    Name(Bytes),
}

/// How a parameter's value is written (`{1:int}`)
#[derive(Clone, Copy, Debug, PartialEq)]
enum Format {
    Default,
    Int,
    Fixed,
    Num,
    Hex,
    HexUpper,
    DateTime,
    ShortDateTime,
    ShortDate,
    ShortTime,
    Iso8601,
    Translate,
}

impl Format {
    fn named(name: &[u8]) -> Option<Format> {
        Some(match name {
            b"int" => Format::Int,
            b"fixed" => Format::Fixed,
            b"num" => Format::Num,
            b"hex" => Format::Hex,
            b"HEX" => Format::HexUpper,
            b"datetime" => Format::DateTime,
            b"shortdatetime" => Format::ShortDateTime,
            b"shortdate" => Format::ShortDate,
            b"shorttime" => Format::ShortTime,
            b"iso8601" => Format::Iso8601,
            b"translate" => Format::Translate,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Piece {
    Text(Bytes),
    Param(Param, Format),
}

/// Why a template does not parse: a client writes the template before some of its messages (`"a{b". Incomplete parameter
/// token at end of string.`) and not before others
#[derive(Debug, PartialEq)]
enum TemplateError {
    Plain(&'static str),
    Quoted(String),
}

impl TemplateError {
    fn text(&self, template: &[u8]) -> String {
        match self {
            TemplateError::Plain(message) => message.to_string(),
            TemplateError::Quoted(message) => format!("\"{}\". {message}", String::from_utf8_lossy(template)),
        }
    }

    /// The message alone (SetEntryValue's)
    fn message(&self) -> String {
        match self {
            TemplateError::Plain(message) => message.to_string(),
            TemplateError::Quoted(message) => message.clone(),
        }
    }
}

/// A template's pieces as a client reads it (measured, tools/parity/probes/localization_templates.luau)
fn parse_template(text: &[u8]) -> Result<Vec<Piece>, TemplateError> {
    use TemplateError::{Plain, Quoted};
    // (`{}` alone is an empty text; anywhere else it is an empty token: measured, `a{}b` and `{}{1}` are refused)
    if text == b"{}" {
        return Ok(Vec::new());
    }
    let mut pieces = Vec::new();
    let mut literal = Vec::new();
    // (whether the parameters so far are numbered or named: a template takes one kind)
    let mut numbered: Option<bool> = None;
    let mut at = 0;
    while at < text.len() {
        match text[at] {
            b'{' if text.get(at + 1) == Some(&b'{') => {
                literal.push(b'{');
                at += 2;
            }
            b'{' => {
                if at + 1 == text.len() {
                    return Err(Plain("Incomplete parameter or unescaped '{' at end of string."));
                }
                let Some(length) = text[at + 1..].iter().position(|&byte| byte == b'}') else {
                    return Err(Quoted("Incomplete parameter token at end of string.".into()));
                };
                let close = at + 1 + length;
                let token = &text[at + 1..close];
                if token.is_empty() {
                    return Err(Quoted(format!("Empty parameter token at [{close}]")));
                }
                let (name, format) = match token.iter().position(|&byte| byte == b':') {
                    Some(colon) => {
                        let (name, format) = (&token[..colon], &token[colon + 1..]);
                        if name.is_empty() {
                            return Err(Quoted(format!("Empty parameter name at [{}]", at + 1 + colon)));
                        }
                        if format.is_empty() {
                            return Err(Quoted(format!("Empty parameter format specifier at [{close}]")));
                        }
                        (name, Some(format))
                    }
                    None => (token, None),
                };
                let shown = || String::from_utf8_lossy(name).into_owned();
                let param = if name[0].is_ascii_digit() {
                    if !name.iter().all(u8::is_ascii_digit) {
                        return Err(Quoted(format!("Invalid parameter index \"{}\".", shown())));
                    }
                    let index = name.iter().fold(0usize, |sum, &digit| sum.saturating_mul(10).saturating_add((digit - b'0') as usize));
                    if index == 0 {
                        return Err(Quoted(format!("Parameter index \"{}\" out of range. Parameter indexes must be > 0.", shown())));
                    }
                    Param::Index(index)
                } else {
                    if !name.iter().all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                        return Err(Quoted(format!(
                            "Invalid parameter name \"{}\". Parameter names can only contain letters (A-Z, a-z), digits (0-9), and underscores (_).",
                            shown()
                        )));
                    }
                    Param::Name(name.to_vec())
                };
                let format = match format {
                    Some(format) => Format::named(format).ok_or_else(|| Quoted(format!("Unknown format specifier \"{}\"", String::from_utf8_lossy(format))))?,
                    None => Format::Default,
                };
                let is_numbered = matches!(param, Param::Index(_));
                match numbered {
                    Some(true) if !is_numbered => {
                        return Err(Quoted(format!("Inconsistent parameter types. Found named parameter \"{}\" after numeric parameter.", shown())));
                    }
                    Some(false) if is_numbered => {
                        return Err(Quoted(format!("Inconsistent parameter types. Found numeric parameter \"{}\" after named parameter.", shown())));
                    }
                    _ => numbered = Some(is_numbered),
                }
                if !literal.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut literal)));
                }
                pieces.push(Piece::Param(param, format));
                at = close + 1;
            }
            b'}' if text.get(at + 1) == Some(&b'}') => {
                literal.push(b'}');
                at += 2;
            }
            b'}' => return Err(Plain("Unescaped '}' at end of string, not part of a parameter token.")),
            byte => {
                literal.push(byte);
                at += 1;
            }
        }
    }
    if !literal.is_empty() {
        pieces.push(Piece::Text(literal));
    }
    Ok(pieces)
}

// FormatByKey's arguments and how each parameter's value is written

/// FormatByKey's arguments as a client converts them: none, an array (a table whose border is not 0, its values 1 to the
/// border; an empty table is an empty array) or a dictionary (its keys made strings: lua/bindable.rs key_name)
enum Arguments {
    None,
    NotTable,
    Array(Vec<LuaValue>),
    Named(FxHashMap<Bytes, LuaValue>),
}

fn arguments(lua: &Lua, engine: &Engine, value: &LuaValue) -> Result<Arguments, Refused> {
    let table = match value {
        LuaValue::Nil => return Ok(Arguments::None),
        LuaValue::Table(table) => table,
        _ => return Ok(Arguments::NotTable),
    };
    let border = table.raw_len();
    if border > 0 {
        let mut items = Vec::with_capacity(border.min(1 << 16));
        for index in 1..=border {
            items.push(table.raw_get::<LuaValue>(index).map_err(|error| error.to_string())?);
        }
        return Ok(Arguments::Array(items));
    }
    let mut named = FxHashMap::default();
    for pair in table.clone().pairs::<LuaValue, LuaValue>() {
        let (key, item) = pair.map_err(|error| error.to_string())?;
        named.insert(crate::lua::bindable::key_name(lua, engine, table, &key)?, item);
    }
    if named.is_empty() {
        return Ok(Arguments::Array(Vec::new()));
    }
    Ok(Arguments::Named(named))
}

impl Arguments {
    /// The value a parameter names, or the client's error
    fn get(&self, param: &Param) -> Result<&LuaValue, String> {
        let range = |index: usize, items: &[LuaValue]| {
            if items.is_empty() {
                format!("Parameter index \"{index}\" was out of range of empty array")
            } else {
                format!("Parameter index \"{index}\" was out of range [1-{}]", items.len())
            }
        };
        match (self, param) {
            (Arguments::None, _) => Err("Found parameter token, but had no parameters".into()),
            (Arguments::NotTable, _) => Err("Found parameter token but parameters was not a table".into()),
            (Arguments::Array(items), Param::Index(index)) => items.get(index - 1).ok_or_else(|| range(*index, items)),
            // (a named parameter reads an array's item 0)
            (Arguments::Array(items), Param::Name(_)) => Err(range(0, items)),
            (Arguments::Named(_), Param::Index(_)) => Err("Expected array table, but table contained named arguments.".into()),
            (Arguments::Named(named), Param::Name(name)) => {
                named.get(name).ok_or_else(|| format!("Parameter name \"{}\" not found in table", String::from_utf8_lossy(name)))
            }
        }
    }
}

/// A number as C's printf writes it with `precision` decimals (`%.2f`): exact digits, halves to even, and MSVC's `-nan(ind)`
/// for the NaN a 0/0 makes
fn fixed(number: f64, precision: usize) -> String {
    if number.is_nan() {
        let bits = number.to_bits();
        let negative = bits >> 63 == 1;
        let quiet = bits & (1 << 51) != 0;
        let payload = bits & ((1 << 51) - 1);
        return match (negative, quiet, payload == 0) {
            (true, true, true) => "-nan(ind)",
            (false, true, _) => "nan",
            (true, true, false) => "-nan",
            (true, false, _) => "-nan(snan)",
            (false, false, _) => "nan(snan)",
        }
        .into();
    }
    if number.is_infinite() {
        return if number > 0.0 { "inf" } else { "-inf" }.into();
    }
    format!("{number:.precision$}")
}

/// A parameter's value as a number, where a format needs one: a number, or a boolean as 1 or 0
fn numeric(value: &LuaValue) -> Option<f64> {
    match value {
        LuaValue::Number(number) => Some(*number),
        LuaValue::Integer(number) => Some(*number as f64),
        LuaValue::Boolean(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// A parameter's value written as `format` says, for a translator of `locale` (`translate` reads it through `translator`)
fn format_value(lua: &Lua, engine: &Engine, value: &LuaValue, format: Format, locale: &[u8], translate: &dyn Fn(&[u8]) -> Bytes) -> Result<Bytes, String> {
    let needs_number = |name: &str| format!("Format type {name} requires numerical parameter.\n");
    let cast = |value: &LuaValue| crate::lua::convert::cast_text(lua, engine, value).map_err(|error| error.to_string());
    Ok(match format {
        Format::Default => match value {
            LuaValue::String(text) => text.as_bytes().to_vec(),
            LuaValue::Number(_) | LuaValue::Integer(_) => fixed(numeric(value).unwrap_or(0.0), 2).into_bytes(),
            LuaValue::Boolean(flag) => if *flag { b"1".to_vec() } else { b"0".to_vec() },
            LuaValue::Nil => b"nil".to_vec(),
            other => cast(other)?,
        },
        Format::Int => fixed(numeric(value).ok_or_else(|| needs_number("int"))?, 0).into_bytes(),
        Format::Fixed => fixed(numeric(value).ok_or_else(|| needs_number("fixed"))?, 2).into_bytes(),
        Format::Num => formats::number(locale, numeric(value).ok_or_else(|| needs_number("num"))?).into_bytes(),
        Format::Hex | Format::HexUpper => {
            let number = numeric(value).ok_or_else(|| needs_number("hex/HEX"))?;
            if number.is_nan() {
                return Err("Format type hex/HEX received nan parameter\n".into());
            }
            // (measured: below -2^63 and above 2^64 refused; a negative number as its 64-bit integer's bits, toward zero; 2^63 to
            // 2^64 as the unsigned integer, 2^64 itself as the conversion's overflow, 8000000000000000)
            if !(-9_223_372_036_854_775_808.0..=18_446_744_073_709_551_616.0).contains(&number) {
                return Err(format!("Number out of range for hex/HEX format ({})\n", fixed(number, 6)));
            }
            let bits = if number < 0.0 {
                number as i64 as u64
            } else if number < 18_446_744_073_709_551_616.0 {
                number as u64
            } else {
                1 << 63
            };
            if format == Format::Hex { format!("{bits:x}") } else { format!("{bits:X}") }.into_bytes()
        }
        Format::DateTime | Format::ShortDateTime | Format::ShortDate | Format::ShortTime | Format::Iso8601 => {
            let moment = match value {
                LuaValue::String(text) => formats::iso_seconds(&text.as_bytes())?,
                other => match numeric(other) {
                    Some(number) => formats::epoch_seconds(number)?,
                    None => return Err("Format string date/time parameters are encoded as numbers (seconds since Epoch) or iso8601 strings.\n".into()),
                },
            };
            formats::date(format, locale, moment).into_bytes()
        }
        // (the value's text, translated as Translate translates a text with no context: measured, `{1:translate}` of "Hello"
        // is the table's "Hola")
        Format::Translate => translate(&cast(value)?),
    })
}

/// A template written with its arguments (FormatByKey's text, its arguments already read: [`arguments`]), and what writing
/// them cost a client beyond the call (cost.rs LOCALIZATION_ARGUMENTS, at the first parameter, and after)
fn render(lua: &Lua, engine: &Engine, pieces: &[Piece], arguments: &Arguments, locale: &[u8], translate: &dyn Fn(&[u8]) -> Bytes) -> (Result<Bytes, String>, f64) {
    let mut out = Vec::new();
    let mut read = false;
    let mut work = 0.0;
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.extend_from_slice(text),
            Piece::Param(param, format) => {
                if !read {
                    work += crate::cost::LOCALIZATION_ARGUMENTS;
                    read = true;
                }
                work += match param {
                    Param::Index(_) => crate::cost::LOCALIZATION_POSITIONAL,
                    Param::Name(_) => crate::cost::LOCALIZATION_NAMED,
                };
                let value = match arguments.get(param) {
                    Ok(value) => value,
                    Err(problem) => return (Err(problem), work),
                };
                if matches!(value, LuaValue::Number(_) | LuaValue::Integer(_)) {
                    work += crate::cost::LOCALIZATION_NUMBER;
                }
                match format_value(lua, engine, value, *format, locale, translate) {
                    Ok(written) => out.extend(written),
                    Err(problem) => return (Err(problem), work),
                }
            }
        }
    }
    (Ok(out), work)
}

// Lookups

/// A locale id's language: what comes before its first `-` (`en` of `en-us`)
fn language(locale: &[u8]) -> &[u8] {
    locale.split(|&byte| byte == b'-').next().unwrap_or_default()
}

/// The text a translator of `locale` finds in an entry (measured, tools/parity/probes/localization_lookup.luau): its value for
/// the locale, else for the locale's language alone (`es` for `es-mx`), else for the locale a client pairs it with (`es-mx` for
/// `es-es`, `fr-ca` for `fr-fr`, `zh-tw` for `zh-hk`), else for another locale of the language, the first by locale id. An
/// empty value is none. Chinese looks past its own locale only from `zh-cn`, `zh-tw`, `zh-hk` and `zh-sg` (`zh-mo` and `zh`
/// find nothing else).
pub fn translation<'a>(entry: &'a Entry, locale: &[u8]) -> Option<&'a [u8]> {
    let found = |locale: &[u8]| entry.value(locale).filter(|text| !text.is_empty());
    if let Some(text) = found(locale) {
        return Some(text);
    }
    let lang = language(locale);
    if lang.is_empty() || lang == b"zh" && !matches!(locale, b"zh-cn" | b"zh-tw" | b"zh-hk" | b"zh-sg") {
        return None;
    }
    if let Some(text) = found(lang) {
        return Some(text);
    }
    let paired: &[u8] = match locale {
        b"es-es" => b"es-mx",
        b"fr-fr" => b"fr-ca",
        b"zh-hk" => b"zh-tw",
        _ => b"",
    };
    if let Some(text) = Some(paired).filter(|paired| !paired.is_empty()).and_then(found) {
        return Some(text);
    }
    entry
        .values
        .iter()
        .find(|(held, text)| !text.is_empty() && held.len() > lang.len() && held.starts_with(lang) && held[lang.len()] == b'-')
        .map(|(_, text)| text.as_slice())
}

/// Whether a translator of `locale` reads a table of `source` locale's Source texts: the same language (`en-gb` reads an
/// `en-us` table's)
fn reads_source(locale: &[u8], source: &[u8]) -> bool {
    language(locale) == language(source)
}

/// A table's source locale (its SourceLocaleId)
fn source_locale(state: &State, names: &Names, id: InstanceId) -> Bytes {
    match state.dom.raw_get(id, names.source_locale) {
        Some(Value::String(text)) => text.as_bytes().to_vec(),
        _ => b"en-us".to_vec(),
    }
}

impl Localization {
    /// A table's entries: the ones its last write made while those are its contents, else its contents read again (a copy's,
    /// a model's)
    pub fn table(state: &mut State, names: &Names, id: InstanceId) -> Rc<Table> {
        let contents = match state.dom.raw_get(id, names.contents) {
            Some(Value::String(text)) => text.clone(),
            _ => Str::default(),
        };
        if let Some(held) = state.localization.tables.get(&id)
            && (held.contents.ptr_eq(&contents) || held.contents.is_empty() && contents.is_empty())
        {
            return held.clone();
        }
        let entries = if contents.is_empty() { Entries::default() } else { parse_contents(contents.as_bytes()).unwrap_or_default() };
        let table = Rc::new(Table::new(contents, entries));
        state.localization.tables.insert(id, table.clone());
        table
    }

    /// A table's entries made `entries`: its contents written (silently: no Changed, measured) and the entries kept with them
    fn store(state: &mut State, reflection: &Reflection, names: &Names, id: InstanceId, entries: Entries) {
        let contents = Str::new(&contents_json(&entries));
        core::raw_set_silent(state, reflection, id, names.contents, Value::String(contents.clone()));
        state.localization.tables.insert(id, Rc::new(Table::new(contents, entries)));
    }
}

/// The tables a translator reads, in the order it reads them: a table's, the table and the tables it is in (up to the first
/// ancestor that is no table); the LocalizationService's, the tables in it that only tables hold, each table's own after the
/// tables in it (measured: a table in a Folder or a Model is not read; GetTableEntries lists the same tables in the same order)
fn tables_of(state: &State, names: &Names, scope: Scope, game: InstanceId) -> Vec<InstanceId> {
    let is_table = |id: InstanceId| state.dom.get(id).is_some_and(|instance| instance.class == names.table);
    match scope {
        Scope::Table(table) => {
            let mut chain = Vec::new();
            let mut current = Some(table).filter(|&id| is_table(id));
            while let Some(id) = current {
                chain.push(id);
                current = state.dom.at(id).parent.filter(|&parent| is_table(parent));
            }
            chain
        }
        Scope::Service => {
            let mut found = Vec::new();
            let Some(service) = service_of(state, names, game) else { return found };
            // (post-order through tables alone, with a stack: a script can nest tables deeper than a thread's stack holds frames for)
            let mut stack: Vec<(InstanceId, bool)> = state.dom.children_of(service).iter().rev().filter(|&&child| is_table(child)).map(|&child| (child, false)).collect();
            while let Some((id, expanded)) = stack.pop() {
                if expanded {
                    found.push(id);
                    continue;
                }
                stack.push((id, true));
                stack.extend(state.dom.children_of(id).iter().rev().filter(|&&child| is_table(child)).map(|&child| (child, false)));
            }
            found
        }
    }
}

/// The DataModel's LocalizationService, if it has one (found, never made)
fn service_of(state: &State, names: &Names, game: InstanceId) -> Option<InstanceId> {
    state.dom.get(game)?;
    state.dom.children_of(game).iter().copied().find(|&child| state.dom.at(child).class == names.service)
}

/// The text FormatByKey formats for `key`: the first table that has a translation of the key for the locale, or whose source
/// locale's language is the locale's and has a Source for it (measured: a table lacking both is passed over)
fn text_for_key(state: &mut State, names: &Names, tables: &[InstanceId], key: &[u8], locale: &[u8]) -> Option<Bytes> {
    for &id in tables {
        let table = Localization::table(state, names, id);
        let Some(entry) = table.by_key(key) else { continue };
        if let Some(text) = translation(entry, locale) {
            return Some(text.to_vec());
        }
        if !entry.source.is_empty() && reads_source(locale, &source_locale(state, names, id)) {
            return Some(entry.source.clone());
        }
    }
    None
}

/// How well an entry's Context fits a context instance's full name: how many of their last dot-separated names agree, ASCII
/// case aside, then how many of those agree in case too (measured: `label` fits `Workspace.Gui.Label`, `A.Label` fits it by
/// one name, an empty Context by none; for a lone `Label`, `Gui.Label` fits better than `label`)
fn context_score(context: &[u8], path: &[u8]) -> (usize, usize) {
    if context.is_empty() {
        return (0, 0);
    }
    let agreeing = context.rsplit(|&byte| byte == b'.').zip(path.rsplit(|&byte| byte == b'.')).take_while(|(a, b)| a.eq_ignore_ascii_case(b));
    agreeing.fold((0, 0), |(names, exact), (a, b)| (names + 1, exact + usize::from(a == b)))
}

/// Translate's text: of the first table with an entry of this Source that has a translation for the locale, the translation
/// of the one whose Context best fits the context (the last of the best), else the text itself (measured)
fn translate_text(state: &mut State, names: &Names, tables: &[InstanceId], text: &[u8], path: &[u8], locale: &[u8]) -> Bytes {
    // (an empty text is never looked up: measured, an entry with a key and no Source does not answer it)
    if text.is_empty() {
        return Vec::new();
    }
    for &id in tables {
        let table = Localization::table(state, names, id);
        let candidates = table.entries.with_source(text);
        if candidates.is_empty() {
            continue;
        }
        let mut best: Option<((usize, usize), &[u8])> = None;
        for entry in candidates {
            // (a translation that is no template is passed over: the Source is the answer then, measured)
            if let Some(found) = translation(entry, locale).filter(|found| parse_template(found).is_ok()) {
                let score = context_score(&entry.context, path);
                if best.is_none_or(|(held, _)| score >= held) {
                    best = Some((score, found));
                }
            }
        }
        if let Some((_, found)) = best {
            return found.to_vec();
        }
        if reads_source(locale, &source_locale(state, names, id)) {
            return text.to_vec();
        }
    }
    text.to_vec()
}

/// The `num`, date and time formats (measured on a 0.747 client, tools/parity/probes/localization_formats.luau: a translator's
/// locale picks one of a few patterns, and every locale it does not know gets the default one)
mod formats {
    use super::Format;
    use chrono::{Datelike, TimeZone, Timelike};

    /// A locale's digit separators and short date and time patterns (`d`/`dd` day, `M`/`MM` month, `yyyy` year, `h`/`H` hour of 12 or
    /// 24, `hh`/`HH` with two digits, `mm` minutes, `tt` AM or PM)
    struct Pattern {
        group: &'static str,
        decimal: &'static str,
        date: &'static str,
        time: &'static str,
    }

    const NBSP: &str = "\u{a0}";
    const DEFAULT: Pattern = Pattern { group: ",", decimal: ".", date: "yyyy-MM-dd", time: "HH:mm" };

    /// The locales with patterns of their own: by the whole locale id, else (the second list) by its language
    const LOCALES: &[(&str, Pattern)] = &[
        ("en-us", Pattern { group: ",", decimal: ".", date: "M/d/yyyy", time: "h:mm tt" }),
        ("en-gb", Pattern { group: ",", decimal: ".", date: "dd/MM/yyyy", time: "HH:mm" }),
        ("en-au", Pattern { group: ",", decimal: ".", date: "dd/MM/yyyy", time: "h:mm tt" }),
        ("en-ca", Pattern { group: ",", decimal: ".", date: "yyyy-MM-dd", time: "h:mm tt" }),
        ("es-es", Pattern { group: ".", decimal: ",", date: "dd/MM/yyyy", time: "HH:mm" }),
        ("es-mx", Pattern { group: ",", decimal: ".", date: "dd/MM/yyyy", time: "h:mm tt" }),
        ("pt-br", Pattern { group: ".", decimal: ",", date: "dd/MM/yyyy", time: "HH:mm" }),
        ("pt-pt", Pattern { group: NBSP, decimal: ",", date: "dd/MM/yyyy", time: "HH:mm" }),
        ("it-it", Pattern { group: ".", decimal: ",", date: "dd/MM/yyyy", time: "HH:mm" }),
        ("fr-fr", Pattern { group: NBSP, decimal: ",", date: "dd/MM/yyyy", time: "HH:mm" }),
        ("fr-ca", Pattern { group: NBSP, decimal: ",", date: "yyyy-MM-dd", time: "HH:mm" }),
        ("de-de", Pattern { group: ".", decimal: ",", date: "dd.MM.yyyy", time: "HH:mm" }),
        ("ru-ru", Pattern { group: NBSP, decimal: ",", date: "dd.MM.yyyy", time: "HH:mm" }),
        ("ja-jp", Pattern { group: ",", decimal: ".", date: "yyyy/MM/dd", time: "HH:mm" }),
        ("zh-cn", Pattern { group: ",", decimal: ".", date: "yyyy/M/d", time: "HH:mm" }),
        ("zh-tw", Pattern { group: ",", decimal: ".", date: "yyyy/M/d", time: "HH:mm" }),
        ("zh-hk", Pattern { group: ",", decimal: ".", date: "d/M/yyyy", time: "HH:mm" }),
        ("zh-sg", Pattern { group: ",", decimal: ".", date: "d/M/yyyy", time: "HH:mm" }),
        ("zh-cjv", DEFAULT),
        ("ar-001", Pattern { group: ",", decimal: ".", date: "d/M/yyyy", time: "HH:mm tt" }),
    ];
    const LANGUAGES: &[(&str, &str)] = &[
        ("en", "en-us"),
        ("es", "es-es"),
        ("fr", "fr-fr"),
        ("de", "de-de"),
        ("ar", "ar-001"),
        ("pt", "pt-br"),
        ("it", "it-it"),
        ("ja", "ja-jp"),
        ("ru", "ru-ru"),
        ("zh", "zh-cn"),
    ];

    fn pattern(locale: &[u8]) -> &'static Pattern {
        let exact = |id: &[u8]| LOCALES.iter().find(|(name, _)| name.as_bytes() == id).map(|(_, pattern)| pattern);
        if let Some(found) = exact(locale) {
            return found;
        }
        let language = super::language(locale);
        LANGUAGES.iter().find(|(name, _)| name.as_bytes() == language).and_then(|(_, id)| exact(id.as_bytes())).unwrap_or(&DEFAULT)
    }

    /// `{1:num}`: two decimals as `%.2f` writes them, the whole part grouped by three, with the locale's separators
    pub fn number(locale: &[u8], number: f64) -> String {
        let fixed = super::fixed(number, 2);
        if !number.is_finite() {
            return fixed;
        }
        let pattern = pattern(locale);
        let (sign, digits) = match fixed.strip_prefix('-') {
            Some(rest) => ("-", rest),
            None => ("", fixed.as_str()),
        };
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        let mut out = String::from(sign);
        for (index, digit) in whole.chars().enumerate() {
            if index > 0 && (whole.len() - index) % 3 == 0 {
                out.push_str(pattern.group);
            }
            out.push(digit);
        }
        out.push_str(pattern.decimal);
        out.push_str(fraction);
        out
    }

    /// A date and time as UTC fields, as a parameter gave them (an ISO text's are kept as written: `2023-02-30` stays)
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Moment {
        year: i64,
        month: i64,
        day: i64,
        hour: i64,
        minute: i64,
        second: i64,
    }

    /// The latest number of seconds a date parameter takes (measured: 1700000000 taken, 2147483647 refused; the client died on
    /// the values between, so the line is the 32-bit one)
    const LATEST: i64 = 2_147_483_646;

    /// A number parameter's moment: the seconds since 1970 rounded half away from zero (none, 0, for what no 64-bit integer
    /// holds), refused before 1970 and after [`LATEST`] with the low 32 bits of the seconds in the message (measured)
    pub fn epoch_seconds(number: f64) -> Result<Moment, String> {
        let rounded = number.round();
        let seconds = if rounded.is_finite() && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&rounded) { rounded as i64 } else { 0 };
        if !(0..=LATEST).contains(&seconds) {
            return Err(format!("Error interpreting number as a date/time ({}) out of range.\n", seconds as i32));
        }
        Ok(utc(seconds))
    }

    fn utc(seconds: i64) -> Moment {
        let days = seconds.div_euclid(86_400);
        let time = seconds.rem_euclid(86_400);
        // (days to a civil date: Howard Hinnant's algorithm)
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
        Moment { year, month, day, hour: time / 3600, minute: time / 60 % 60, second: time % 60 }
    }

    /// The seconds since 1970 of UTC fields, days past a month's end running into the next (as timegm counts them)
    fn epoch(moment: &Moment) -> i64 {
        let (year, month) = if moment.month <= 2 { (moment.year - 1, moment.month + 9) } else { (moment.year, moment.month - 3) };
        let era = year.div_euclid(400);
        let yoe = year - era * 400;
        let doy = (153 * month + 2) / 5;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468 + moment.day - 1;
        days * 86_400 + moment.hour * 3600 + moment.minute * 60 + moment.second
    }

    /// An ISO text parameter's moment: exactly `YYYY-mm-ddTHH:MM:SSZ`, its fields in bounds (years 1970 to 2037: 1969 and 9999 refused, measured), else the client's error
    pub fn iso_seconds(text: &[u8]) -> Result<Moment, String> {
        let shown = String::from_utf8_lossy(text);
        let form = || format!("Error interpreting string as a date/time. ({shown})  Must be of the form YYYY-mm-ddTHH:MM:SSZ\n");
        let shape = b"dddd-dd-ddTdd:dd:ddZ";
        if text.len() != shape.len() || !text.iter().zip(shape).all(|(&byte, &want)| if want == b'd' { byte.is_ascii_digit() } else { byte == want }) {
            return Err(form());
        }
        let field = |from: usize, to: usize| text[from..to].iter().fold(0i64, |sum, &digit| sum * 10 + (digit - b'0') as i64);
        let moment = Moment { year: field(0, 4), month: field(5, 7), day: field(8, 10), hour: field(11, 13), minute: field(14, 16), second: field(17, 19) };
        let bounded = (1970..=2037).contains(&moment.year)
            && (1..=12).contains(&moment.month)
            && (1..=31).contains(&moment.day)
            && moment.hour < 24
            && moment.minute < 60
            && moment.second < 60;
        if !bounded {
            return Err(format!("Error interpreting string as a date/time. ({shown})  Date/time fields failed bound check\n"));
        }
        Ok(moment)
    }

    /// A moment written as `format` says: `datetime` and `iso8601` in UTC as given, the short ones in the host's local time with
    /// the locale's patterns
    pub fn date(format: Format, locale: &[u8], moment: Moment) -> String {
        let Moment { year, month, day, hour, minute, second } = moment;
        match format {
            Format::DateTime => return format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"),
            Format::Iso8601 => return format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"),
            _ => {}
        }
        let local = match chrono::Local.timestamp_opt(epoch(&moment), 0) {
            chrono::LocalResult::Single(local) | chrono::LocalResult::Ambiguous(local, _) => local,
            chrono::LocalResult::None => return String::new(),
        };
        let fields = Local { year: local.year() as i64, month: local.month() as i64, day: local.day() as i64, hour: local.hour() as i64, minute: local.minute() as i64 };
        let pattern = pattern(locale);
        match format {
            Format::ShortDate => written(pattern.date, &fields),
            Format::ShortTime => written(pattern.time, &fields),
            _ => format!("{} {}", written(pattern.date, &fields), written(pattern.time, &fields)),
        }
    }

    struct Local {
        year: i64,
        month: i64,
        day: i64,
        hour: i64,
        minute: i64,
    }

    fn written(pattern: &str, at: &Local) -> String {
        let mut out = String::new();
        let bytes = pattern.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let letter = bytes[index];
            let run = bytes[index..].iter().take_while(|&&byte| byte == letter).count();
            let twelve = if at.hour % 12 == 0 { 12 } else { at.hour % 12 };
            let text = match (letter, run) {
                (b'y', _) => format!("{:04}", at.year),
                (b'M', 1) => at.month.to_string(),
                (b'M', _) => format!("{:02}", at.month),
                (b'd', 1) => at.day.to_string(),
                (b'd', _) => format!("{:02}", at.day),
                (b'h', 1) => twelve.to_string(),
                (b'h', _) => format!("{twelve:02}"),
                (b'H', 1) => at.hour.to_string(),
                (b'H', _) => format!("{:02}", at.hour),
                (b'm', _) => format!("{:02}", at.minute),
                (b't', _) => if at.hour < 12 { "AM" } else { "PM" }.to_string(),
                _ => String::from_utf8_lossy(&bytes[index..index + run]).into_owned(),
            };
            out.push_str(&text);
            index += run;
        }
        out
    }

}

// The scripts' API

/// The members a game script may not use (Roblox's own scripts'): each raises the capability error (measured, every one)
const ROBLOX_ONLY: &[(&str, &str)] = &[
    ("Translator", "RobloxOnlyTranslate"),
    ("LocalizationTable", "SetIsExemptFromUGCAnalytics"),
    ("LocalizationService", "GetIsLoadingInternalTranslations"),
    ("LocalizationService", "IsLoadingInternalTranslationsSettingChanged"),
    ("LocalizationService", "SetRobloxLocaleId"),
    ("LocalizationService", "StartTextScraper"),
    ("LocalizationService", "StopTextScraper"),
    ("LocalizationService", "PromptDownloadGameTableToCSV"),
    ("LocalizationService", "PromptExportToCSVs"),
    ("LocalizationService", "PromptImportFromCSVs"),
    ("LocalizationService", "PromptUploadCSVToGameTable"),
];

/// The locale a playing client's LocalizationService reports (RobloxLocaleId, SystemLocaleId: measured, a 0.747 client and
/// server alike)
pub const ROBLOX_LOCALE: &str = "en-us";

const ONE: &[StringParam] = &[StringParam::at(1)];
const TWO: &[StringParam] = &[StringParam::at(1), StringParam::at(2)];
const THREE: &[StringParam] = &[StringParam::at(1), StringParam::at(2), StringParam::at(3)];
const FOUR: &[StringParam] = &[StringParam::at(1), StringParam::at(2), StringParam::at(3), StringParam::at(4)];
const FIVE: &[StringParam] = &[StringParam::at(1), StringParam::at(2), StringParam::at(3), StringParam::at(4), StringParam::at(5)];
/// Translate's text, the second argument
const SECOND: &[StringParam] = &[StringParam::at(2)];
/// GetTranslatorForLocaleAsync's locale: cast without the warning (a method that yields)
const QUIET: &[StringParam] = &[StringParam::quiet(1)];

/// The string argument at `index` (cast already: [`strings`])
fn text_arg(args: &Args, index: usize) -> Bytes {
    args.string(index).map(|text| text.as_bytes().to_vec()).unwrap_or_default()
}

/// A Source as the entry methods keep it: `{}` alone is an empty text (measured: SetEntries, SetEntryValue)
fn source_text(text: &[u8]) -> &[u8] {
    if text == b"{}" { b"" } else { text }
}

/// A translation as GetEntries, GetString and GetContents show it: `{}` alone as an empty text (it still translates, to an
/// empty text, where an empty translation is none: measured)
fn shown(text: &[u8]) -> &[u8] {
    if text == b"{}" { b"" } else { text }
}

fn lossy(text: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(text)
}

/// Whether a Lua table holds nothing (its array part or hash part)
fn is_empty(table: &LuaTable) -> bool {
    table.clone().pairs::<LuaValue, LuaValue>().next().is_none()
}

/// SetEntries' entries as a client reads them (measured, tools/parity/probes/localization_entries.luau): an array of
/// dictionaries, each with string `Key`, `Source`, `Context` and `Example` fields (absent ones empty) and a `Values`
/// dictionary of strings by locale (locale ids made lower case with `-`, the first of those that end up equal in key order
/// kept, an empty one dropped); the whole call refused at the first bad entry
fn entries_from_lua(lua: &Lua, engine: &Engine, value: &LuaValue) -> Result<Entries, Refused> {
    let LuaValue::Table(list) = value else { return Err(Refused::from("entries was not an array.".to_string())) };
    let border = list.raw_len();
    if border == 0 && !is_empty(list) {
        return Err(Refused::from("entries was not an array.".to_string()));
    }
    let mut entries = Builder::default();
    for index in 1..=border {
        let item: LuaValue = list.raw_get(index).map_err(|error| error.to_string())?;
        let fields = match item {
            LuaValue::Table(fields) if fields.raw_len() == 0 && !is_empty(&fields) => fields,
            _ => return Err(Refused::from(format!("Entry at index {index} was not a table."))),
        };
        let invalid = |message: String| format!("Entry at index {index} had invalid Key/Source/Context: {message}");
        let field = |name: &str| -> Result<Option<Bytes>, String> {
            match fields.raw_get::<LuaValue>(name).map_err(|error| error.to_string())? {
                LuaValue::Nil => Ok(Some(Vec::new())),
                LuaValue::String(text) => Ok(Some(text.as_bytes().to_vec())),
                _ => Ok(None),
            }
        };
        let mut texts = Vec::with_capacity(3);
        for name in ["Key", "Source", "Context"] {
            match field(name)? {
                Some(text) => texts.push(text),
                None => return Err(Refused::from(invalid(format!("Entry at index {index} \"{name}\" was not a string.")))),
            }
        }
        let (key, source, context) = (&texts[0], source_text(&texts[1]), &texts[2]);
        if key.is_empty() && source.is_empty() {
            return Err(Refused::from(invalid("Cannot create entry with empty source text and no key.".into())));
        }
        if let Err(problem) = parse_template(source) {
            return Err(Refused::from(invalid(problem.text(source))));
        }
        let mut entry = Entry::new(key, source, context);
        entry.example = field("Example")?.ok_or_else(|| format!("Entry at index {index} \"Example\" was not a string."))?;
        let values = match fields.raw_get::<LuaValue>("Values").map_err(|error| error.to_string())? {
            LuaValue::Nil => return Err(Refused::from(format!("Entry at index {index} had was missing \"Values\"."))),
            LuaValue::Table(values) if values.raw_len() == 0 => values,
            _ => return Err(Refused::from(format!("Entry at index {index} [\"Values\"] was not a table."))),
        };
        let mut pairs = Vec::new();
        for pair in values.pairs::<LuaValue, LuaValue>() {
            let (name, text) = pair.map_err(|error| error.to_string())?;
            pairs.push((crate::lua::bindable::key_name(lua, engine, &values, &name)?, text));
        }
        crate::sort::by(&mut pairs, |a, b| a.0.cmp(&b.0));
        for (name, text) in pairs {
            let LuaValue::String(text) = text else { return Err(Refused::from(format!("Entry at index {index} \"{}\" was not a string.", lossy(&name)))) };
            let id = locale(&name);
            if !id.is_empty() && entry.value(&id).is_none() {
                entry.set_value(id, text.as_bytes().to_vec());
            }
        }
        entries.add(entry, index)?;
    }
    Ok(entries.done())
}

/// GetEntries' list: a new table of new entry tables each call
fn entries_to_lua(lua: &Lua, entries: &Entries) -> LuaResult<LuaTable> {
    let list = lua.create_table_with_capacity(entries.len(), 0)?;
    for entry in entries.iter() {
        let item = lua.create_table_with_capacity(0, 5)?;
        item.raw_set("Key", lua.create_string(&entry.key)?)?;
        item.raw_set("Source", lua.create_string(&entry.source)?)?;
        item.raw_set("Context", lua.create_string(&entry.context)?)?;
        item.raw_set("Example", lua.create_string(&entry.example)?)?;
        let values = lua.create_table_with_capacity(0, entry.values.len())?;
        for (id, text) in &entry.values {
            values.raw_set(lua.create_string(id)?, lua.create_string(shown(text))?)?;
        }
        item.raw_set("Values", values)?;
        list.raw_push(item)?;
    }
    Ok(list)
}

/// Makes a Translator (named "Instance", parent-locked, as a client's: measured) of `locale` reading `scope`. A table's translator
/// does not keep its table: once the table is collected its translator raises (measured: [`gone`])
fn new_translator(engine: &Engine, names: &Names, scope: Scope, locale: Bytes) -> InstanceId {
    let mut state = engine.state.borrow_mut();
    let id = core::create_named(&mut state, &engine.reflection, names.translator, "Instance");
    core::raw_set_silent(&mut state, &engine.reflection, id, names.locale_id, Value::String(Str::new(&locale)));
    state.dom.at_mut(id).set_flag(crate::dom::flags::PARENT_LOCKED, true);
    state.localization.translators.insert(id, scope);
    id
}

/// The error of a table's translator whose table was collected (measured: a client's FormatByKey and Translate raise it once
/// no script holds the table, which a translator does not keep)
fn gone(state: &State, scope: Scope) -> Option<&'static str> {
    match scope {
        Scope::Table(table) if state.dom.get(table).is_none() => Some("LocalizationTable no longer exists"),
        _ => None,
    }
}

/// Charges `seconds` of a call's work beyond its price (cost.rs method)
fn charge(engine: &Engine, seconds: f64) {
    if seconds > 0.0 {
        engine.state.borrow().sched.charge(seconds);
        crate::boundary::add_cost(seconds);
    }
}

/// What reading or writing these entries costs: per entry and per value
fn entries_work(entries: &Entries, entry: f64, value: f64) -> f64 {
    entries.iter().map(|held| entry + held.values.len() as f64 * value).sum()
}

/// A string property's text
fn string_of(state: &State, id: InstanceId, name: NameId) -> Bytes {
    match state.dom.raw_get(id, name) {
        Some(Value::String(text)) => text.as_bytes().to_vec(),
        _ => Vec::new(),
    }
}

/// A player argument (GetTranslatorForPlayer's and the like): an Instance, or None for nil and what a client casts to nil; a
/// missing argument raises "Argument 1 missing or nil", another value "Unable to cast value to Object"
fn object_arg(engine: &Engine, args: &Args) -> Result<Option<InstanceId>, Flow> {
    if args.len() < 2 {
        return raise("Argument 1 missing or nil");
    }
    if let Some(id) = args.instance(1) {
        return Ok(Some(id));
    }
    match args.get(1) {
        LuaValue::Nil | LuaValue::Thread(_) => Ok(None),
        other => match typeof_value(engine, other).as_str() {
            "Enum" | "Enums" | "userdata" => Ok(None),
            _ => raise("Unable to cast value to Object"),
        },
    }
}

fn is_player(engine: &Engine, names: &Names, id: InstanceId) -> bool {
    engine.state.borrow().dom.get(id).is_some_and(|instance| instance.class == names.player)
}

fn string(lua: &Lua, text: &[u8]) -> LuaResult<LuaValue> {
    Ok(LuaValue::String(lua.create_string(text)?))
}

/// A table method that changes its entries: `edit` works on a copy, which is stored when it returns Ok (and when it fails
/// part way, as `Err((message, true))` says: a client's SetEntryKey to no key and no Source loses the entry, measured)
fn edit_entries(engine: &Engine, names: &Names, id: InstanceId, edit: impl FnOnce(&mut Entries) -> Result<(), (String, bool)>) -> Ret {
    let mut state = engine.state.borrow_mut();
    let mut entries = Localization::table(&mut state, names, id).entries.clone();
    let outcome = edit(&mut entries);
    if !matches!(outcome, Err((_, false))) {
        Localization::store(&mut state, &engine.reflection, names, id, entries);
    }
    drop(state);
    match outcome {
        Ok(()) => Ok(none()),
        Err((message, _)) => raise(message),
    }
}

/// The entry a (key, source, context) names, made when there is none (SetEntryValue, SetEntryContext, SetEntryKey,
/// SetEntrySource: measured; SetEntryExample makes none)
fn find_or_make(entries: &mut Entries, key: &[u8], source: &[u8], context: &[u8]) -> Result<At, (String, bool)> {
    if let Some(at) = entries.find(key, source, context) {
        return Ok(at);
    }
    let source = source_text(source);
    if key.is_empty() && source.is_empty() {
        return Err(("Cannot create entry with empty source text and no key.".into(), false));
    }
    parse_template(source).map_err(|problem| (problem.text(source), false))?;
    Ok(entries.insert(Entry::new(key, source, context)))
}

/// SetEntryContext, SetEntryKey and SetEntrySource: the entry (made when there is none) changed by `edit` and filed again,
/// unless another entry has the identity it would take ("The modified entry would conflict with an existing entry."); an
/// entry left with no key and no Source is lost ("Failed to add entry to index after modifying.", measured)
fn modify(entries: &mut Entries, key: &[u8], source: &[u8], context: &[u8], edit: impl Fn(&mut Entry)) -> Result<(), (String, bool)> {
    let at = find_or_make(entries, key, source, context)?;
    let mut changed = entries.at(at).clone();
    edit(&mut changed);
    if entries.find(&changed.key, &changed.source, &changed.context).is_some_and(|other| other != at) {
        return Err(("The modified entry would conflict with an existing entry.".into(), true));
    }
    if entries.refile(at, |entry| edit(entry)).is_err() {
        return Err(("Failed to add entry to index after modifying.".into(), true));
    }
    Ok(())
}

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = &engine.reflection;
    let Some(names) = Names::new(reflection) else { return };
    let member = |name: &str| reflection.name_id(name);
    let table = names.table;

    for &(class, name) in ROBLOX_ONLY {
        if let (Some(class), Some(id)) = (reflection.class_id(class), member(name)) {
            behaviors.add_method(class, id, method(move |_, _, _, _| raise(format!("The current thread cannot call '{name}' (lacking capability RobloxScript)"))));
        }
    }

    // LocalizationTable
    if let Some(id) = member("SetEntries") {
        behaviors.add_method(table, id, method(move |lua, engine, this, args| {
            if args.len() < 2 || args.get(1).is_nil() {
                return raise("Argument 1 missing or nil");
            }
            let entries = match entries_from_lua(lua, engine, args.get(1)) {
                Ok(entries) => entries,
                Err(refused) => return refused.raise(),
            };
            charge(engine, entries_work(&entries, crate::cost::LOCALIZATION_ENTRY_SET, crate::cost::LOCALIZATION_VALUE_SET));
            Localization::store(&mut engine.state.borrow_mut(), &engine.reflection, &names, this, entries);
            Ok(none())
        }));
    }
    if let Some(id) = member("GetEntries") {
        behaviors.add_method(table, id, method(move |lua, engine, this, _| {
            let held = Localization::table(&mut engine.state.borrow_mut(), &names, this);
            charge(engine, entries_work(&held.entries, crate::cost::LOCALIZATION_ENTRY_LISTED, crate::cost::LOCALIZATION_VALUE_LISTED));
            Ok(values([LuaValue::Table(entries_to_lua(lua, &held.entries)?)]))
        }));
    }
    if let Some(id) = member("GetContents") {
        behaviors.add_method(table, id, method(move |lua, engine, this, _| {
            let held = Localization::table(&mut engine.state.borrow_mut(), &names, this);
            let json = contents_json(&held.entries);
            charge(engine, json.len() as f64 * crate::cost::LOCALIZATION_CONTENTS_BYTE);
            Ok(values([string(lua, &json)?]))
        }));
    }
    if let Some(id) = member("SetContents") {
        behaviors.add_method(table, id, strings(ONE, method(move |_, engine, this, args| {
            let entries = match parse_contents(&text_arg(args, 1)) {
                Ok(entries) => entries,
                Err(message) => return raise(message),
            };
            charge(engine, entries_work(&entries, crate::cost::LOCALIZATION_ENTRY_SET, crate::cost::LOCALIZATION_VALUE_SET));
            Localization::store(&mut engine.state.borrow_mut(), &engine.reflection, &names, this, entries);
            Ok(none())
        })));
    }
    if let Some(id) = member("GetString") {
        behaviors.add_method(table, id, strings(TWO, method(move |lua, engine, this, args| {
            let (wanted, key) = (locale(&text_arg(args, 1)), text_arg(args, 2));
            let held = Localization::table(&mut engine.state.borrow_mut(), &names, this);
            let Some(entry) = held.by_key(&key) else { return raise(format!("LocalizationTable does not contain key \"{}\"", lossy(&key))) };
            let Some(text) = entry.value(&wanted) else {
                return raise(format!("LocalizationTable entry with key \"{}\" does not contain locale \"{}\"", lossy(&key), lossy(&wanted)));
            };
            Ok(values([string(lua, shown(text))?]))
        })));
    }
    if let Some(id) = member("SetEntry") {
        behaviors.add_method(table, id, strings(THREE, method(move |_, engine, this, args| {
            let (key, id, text) = (text_arg(args, 1), locale(&text_arg(args, 2)), text_arg(args, 3));
            if let Err(problem) = parse_template(&text) {
                return raise(problem.message());
            }
            edit_entries(engine, &names, this, |entries| {
                let at = find_or_make(entries, &key, b"", b"")?;
                if !id.is_empty() {
                    entries.at_mut(at).set_value(id, text);
                }
                Ok(())
            })
        })));
    }
    if let Some(id) = member("SetEntryValue") {
        behaviors.add_method(table, id, strings(FIVE, method(move |_, engine, this, args| {
            let (key, source, context, id, text) = (text_arg(args, 1), text_arg(args, 2), text_arg(args, 3), locale(&text_arg(args, 4)), text_arg(args, 5));
            // (a text that is no template makes no entry: measured)
            if let Err(problem) = parse_template(&text) {
                return raise(problem.message());
            }
            edit_entries(engine, &names, this, |entries| {
                let at = find_or_make(entries, &key, &source, &context)?;
                if !id.is_empty() {
                    entries.at_mut(at).set_value(id, text);
                }
                Ok(())
            })
        })));
    }
    for (name, field) in [("SetEntryContext", 0), ("SetEntryKey", 1), ("SetEntrySource", 2)] {
        let Some(id) = member(name) else { continue };
        behaviors.add_method(table, id, strings(FOUR, method(move |_, engine, this, args| {
            let (key, source, context, new) = (text_arg(args, 1), text_arg(args, 2), text_arg(args, 3), text_arg(args, 4));
            if field == 2
                && let Err(problem) = parse_template(&new)
            {
                return raise(problem.text(&new));
            }
            edit_entries(engine, &names, this, |entries| {
                // (an entry SetEntrySource makes takes the new Source as SetEntryValue takes one: measured, `{}` is empty there)
                if field == 2 && entries.find(&key, &source, &context).is_none() {
                    find_or_make(entries, &key, &new, &context)?;
                    return Ok(());
                }
                modify(entries, &key, &source, &context, |entry| match field {
                    0 => entry.context = new.clone(),
                    1 => entry.key = new.clone(),
                    _ => entry.source = new.clone(),
                })
            })
        })));
    }
    if let Some(id) = member("SetEntryExample") {
        behaviors.add_method(table, id, strings(FOUR, method(move |_, engine, this, args| {
            let (key, source, context, example) = (text_arg(args, 1), text_arg(args, 2), text_arg(args, 3), text_arg(args, 4));
            edit_entries(engine, &names, this, |entries| {
                if let Some(at) = entries.find(&key, &source, &context) {
                    entries.at_mut(at).example = example;
                }
                Ok(())
            })
        })));
    }
    if let Some(id) = member("RemoveEntry") {
        behaviors.add_method(table, id, strings(THREE, method(move |_, engine, this, args| {
            let (key, source, context) = (text_arg(args, 1), text_arg(args, 2), text_arg(args, 3));
            edit_entries(engine, &names, this, |entries| {
                if let Some(at) = entries.find(&key, &source, &context) {
                    entries.remove(at);
                }
                Ok(())
            })
        })));
    }
    if let Some(id) = member("RemoveEntryValue") {
        behaviors.add_method(table, id, strings(FOUR, method(move |_, engine, this, args| {
            let (key, source, context, id) = (text_arg(args, 1), text_arg(args, 2), text_arg(args, 3), locale(&text_arg(args, 4)));
            edit_entries(engine, &names, this, |entries| {
                if let Some(at) = entries.find(&key, &source, &context) {
                    entries.at_mut(at).remove_value(&id);
                }
                Ok(())
            })
        })));
    }
    if let Some(id) = member("RemoveKey") {
        behaviors.add_method(table, id, strings(ONE, method(move |_, engine, this, args| {
            let key = text_arg(args, 1);
            edit_entries(engine, &names, this, |entries| {
                if let Some(at) = Some(&key).filter(|key| !key.is_empty()).and_then(|key| entries.find_key(key)) {
                    entries.remove(at);
                }
                Ok(())
            })
        })));
    }
    // (its locale is not made lower case: measured, RemoveTargetLocale("ES_ES") leaves es-es)
    if let Some(id) = member("RemoveTargetLocale") {
        behaviors.add_method(table, id, strings(ONE, method(move |_, engine, this, args| {
            let id = text_arg(args, 1);
            edit_entries(engine, &names, this, |entries| {
                for group in &mut entries.groups {
                    for entry in &mut group.entries {
                        entry.remove_value(&id);
                    }
                }
                Ok(())
            })
        })));
    }
    if let Some(id) = member("GetTranslator") {
        behaviors.add_method(table, id, strings(ONE, method(move |lua, engine, this, args| {
            let translator = new_translator(engine, &names, Scope::Table(this), locale(&text_arg(args, 1)));
            Ok(values([instance_value(lua, engine, translator)?]))
        })));
    }
    // SourceLocaleId keeps its locale lower case with `-`; DevelopmentLanguage reads and writes it (measured)
    if let Some(info) = reflection.class_property(table, names.source_locale).cloned() {
        let set_locale: super::SetterFn = Rc::new(move |lua, engine, id, value| {
            let coerced = crate::lua::convert::coerce(lua, engine, &info, &value)?;
            let coerced = match coerced {
                Value::String(text) => Value::String(Str::new(&locale(text.as_bytes()))),
                other => other,
            };
            crate::lua::instance::set_property(engine, id, names.source_locale, coerced);
            Ok(())
        });
        behaviors.add_setter(table, names.source_locale, set_locale.clone());
        if let Some(development) = member("DevelopmentLanguage") {
            behaviors.add_setter(table, development, set_locale);
            behaviors.add_getter(table, development, super::getter(move |lua, engine, id| string(lua, &string_of(&engine.state.borrow(), id, names.source_locale)).map_err(Flow::from)));
        }
    }

    // Translator
    let translator = names.translator;
    if let Some(id) = member("FormatByKey") {
        behaviors.add_method(translator, id, strings(ONE, method(move |lua, engine, this, args| {
            let key = text_arg(args, 1);
            // (the arguments are read first, as a client reads a call's arguments: a dictionary key's failing __tostring raises
            // even when the key is not found; measured, tools/parity/probes/table_keys.luau)
            let given = match arguments(lua, engine, args.get(2)) {
                Ok(given) => given,
                Err(refused) => return refused.raise(),
            };
            let (text, wanted, tables) = {
                let mut state = engine.state.borrow_mut();
                let wanted = string_of(&state, this, names.locale_id);
                let scope = state.localization.translators.get(&this).copied().unwrap_or(Scope::Service);
                let tables = tables_of(&state, &names, scope, engine.world().game);
                if let Some(message) = gone(&state, scope) {
                    drop(state);
                    return raise(message);
                }
                match text_for_key(&mut state, &names, &tables, &key, &wanted) {
                    Some(text) => (text, wanted, tables),
                    None => {
                        drop(state);
                        charge(engine, crate::cost::LOCALIZATION_NOT_FOUND);
                        return raise(format!("Key {} not found for locale {}", lossy(&key), lossy(&wanted)));
                    }
                }
            };
            let pieces = match parse_template(&text) {
                Ok(pieces) => pieces,
                Err(problem) => return raise(problem.message()),
            };
            let translate = |text: &[u8]| translate_text(&mut engine.state.borrow_mut(), &names, &tables, text, b"", &wanted);
            let (written, work) = render(lua, engine, &pieces, &given, &wanted, &translate);
            charge(engine, work + text.len() as f64 * crate::cost::LOCALIZATION_TEXT_BYTE);
            match written {
                Ok(written) => Ok(values([string(lua, &written)?])),
                Err(message) => raise(message),
            }
        })));
    }
    if let Some(id) = member("Translate") {
        behaviors.add_method(translator, id, strings(SECOND, method(move |lua, engine, this, args| {
            let context = match args.instance(1) {
                Some(context) => context,
                None => match args.get(1) {
                    LuaValue::Nil | LuaValue::Thread(_) => return raise("Context must be an Instance."),
                    other => match typeof_value(engine, other).as_str() {
                        "Enum" | "Enums" | "userdata" => return raise("Context must be an Instance."),
                        _ => return raise("Unable to cast value to Object"),
                    },
                },
            };
            let text = text_arg(args, 2);
            let mut state = engine.state.borrow_mut();
            let wanted = string_of(&state, this, names.locale_id);
            let scope = state.localization.translators.get(&this).copied().unwrap_or(Scope::Service);
            if let Some(message) = gone(&state, scope) {
                drop(state);
                return raise(message);
            }
            let tables = tables_of(&state, &names, scope, engine.world().game);
            let path = state.dom.full_name(&engine.reflection, context);
            let translated = translate_text(&mut state, &names, &tables, &text, path.as_bytes(), &wanted);
            drop(state);
            Ok(values([string(lua, &translated)?]))
        })));
    }
    // (LocaleId as it was made, at what a client's read costs)
    behaviors.add_getter(translator, names.locale_id, super::getter(move |lua, engine, id| {
        charge(engine, crate::cost::LOCALIZATION_LOCALE_READ);
        let text = string_of(&engine.state.borrow(), id, names.locale_id);
        string(lua, &text).map_err(Flow::from)
    }));
    // (a Translator is not copied: Clone gives nil, measured)
    if let Some(id) = member("Clone") {
        behaviors.add_method(translator, id, method(|_, _, _, _| Ok(values([LuaValue::Nil]))));
    }

    // LocalizationService
    let service = names.service;
    for name in ["RobloxLocaleId", "SystemLocaleId"] {
        if let Some(id) = member(name) {
            behaviors.add_getter(service, id, super::getter(|lua, engine, _| {
                charge(engine, crate::cost::LOCALIZATION_SERVICE_LOCALE_READ);
                string(lua, ROBLOX_LOCALE.as_bytes()).map_err(Flow::from)
            }));
        }
    }
    if let Some(id) = member("GetTranslatorForPlayer") {
        behaviors.add_method(service, id, method(move |lua, engine, _, args| {
            let Some(player) = object_arg(engine, args)? else { return raise("Player cannot be nil") };
            if !is_player(engine, &names, player) {
                return raise("Instance was not a player");
            }
            let wanted = string_of(&engine.state.borrow(), player, names.locale_id);
            let made = new_translator(engine, &names, Scope::Service, wanted);
            Ok(values([instance_value(lua, engine, made)?]))
        }));
    }
    if let Some(id) = member("GetTranslatorForPlayerAsync") {
        behaviors.add_method(service, id, method(move |lua, engine, _, args| {
            let Some(player) = object_arg(engine, args)? else { return raise("Argument 1 missing or nil") };
            if engine.options.game_id == 0.0 {
                return raise("Publishing the game is required to use GetTranslatorForPlayerAsync API.");
            }
            if !is_player(engine, &names, player) {
                return raise("Instance was not a player");
            }
            let wanted = string_of(&engine.state.borrow(), player, names.locale_id);
            let made = new_translator(engine, &names, Scope::Service, wanted);
            Ok(values([instance_value(lua, engine, made)?]))
        }));
    }
    if let Some(id) = member("GetTranslatorForLocaleAsync") {
        behaviors.add_method(service, id, strings(QUIET, method(move |lua, engine, _, args| {
            let given = text_arg(args, 1);
            if given.len() < 2 {
                return raise("Too few characters in locale id!");
            }
            let made = new_translator(engine, &names, Scope::Service, locale(&given));
            Ok(values([instance_value(lua, engine, made)?]))
        })));
    }
    if let Some(id) = member("GetTableEntries") {
        behaviors.add_method(service, id, method(move |lua, engine, _, args| {
            if !args.get(1).is_nil() && args.instance(1).is_none() {
                object_arg(engine, args)?;
            }
            let held: Vec<Rc<Table>> = {
                let mut state = engine.state.borrow_mut();
                let tables = tables_of(&state, &names, Scope::Service, engine.world().game);
                tables.into_iter().map(|id| Localization::table(&mut state, &names, id)).collect()
            };
            let list = lua.create_table_with_capacity(held.len() + 1, 0)?;
            for table in &held {
                charge(engine, entries_work(&table.entries, crate::cost::LOCALIZATION_ENTRY_LISTED, crate::cost::LOCALIZATION_VALUE_LISTED));
                list.raw_push(entries_to_lua(lua, &table.entries)?)?;
            }
            // (and the experience's own table, which a client offline has empty)
            list.raw_push(lua.create_table()?)?;
            Ok(values([LuaValue::Table(list)]))
        }));
    }
    if let Some(id) = member("GetCorescriptLocalizations") {
        behaviors.add_method(service, id, method(|lua, _, _, _| Ok(values([LuaValue::Table(lua.create_table()?)]))));
    }
    // (a client's answer is its player's country, which no offline run has: "US" stands in, as before)
    if let Some(id) = member("GetCountryRegionForPlayerAsync") {
        behaviors.add_method(service, id, method(move |lua, engine, _, args| {
            let Some(player) = object_arg(engine, args)? else { return raise("Argument 1 missing or nil") };
            if !is_player(engine, &names, player) {
                return raise("Instance was not a player");
            }
            Ok(values([string(lua, b"US")?]))
        }));
    }

    // A freed table or translator leaves the caches
    engine.hooks.borrow_mut().instance_freed.push(Box::new(|state, id| {
        state.localization.tables.remove(&id);
        state.localization.translators.remove(&id);
    }));
}

#[cfg(test)]
#[path = "localization_tests.rs"]
mod tests;
