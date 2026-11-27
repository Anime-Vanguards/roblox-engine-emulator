//! TextService:GetTextSize without a text layer (a host layer, roblox-ui-renderer's, measures with real fonts and replaces this): a script
//! that reads `.X` of it ran into nil in a plain `ree run` (a settings window). The answer comes from Studio's own
//! measurements, per Enum.Font (data/text-advances.json, from a text_advances probe): Studio hints its fonts, so a glyph's advance
//! is a whole number of pixels at the size asked and does not scale down from a large size. A least-squares line per character over 23
//! sizes (8-100), `round(a x size + c)` summed over the string, reproduces 76 % of 3,744 sample strings exactly (mean error 0.66 px, 95th
//! percentile 3). A line is `floor(ratio x size)` high (Legacy 1.5, every other font 1.0). FredokaOne at the integer sizes 1-100 is a
//! client's own table (data/fredoka-sizes.json: advances, kerns, ligatures). Lines break as a client breaks them ([`measure`]).
//!
//! TextService:GetTextBoundsAsync (a GetTextBoundsParams: Text, Font, Size, Width, RichText) is the same measurement of a Font face
//! (measured in a playing client, Studio 0.747, tools/parity/probes/text_bounds.luau): the bounds of an Enum.Font face are what
//! GetTextSize answers for it, with Width as the frame's width (none when not above 0), but for SourceSansItalic, which GetTextSize
//! measures upright. A face no Enum.Font names measures as the one data/text-faces.json says (tools/text-faces.mjs: a family without the
//! weight or style asked picks a face it has); a face of its own the tables lack, an unknown family and rich text markup are measured
//! as the nearest face, SourceSans and plain text, and reported missing. A client loads a face on its first call, which yields (the
//! faces it starts with do not); a server has no typesetting, so GetTextBoundsAsync raises and GetTextSize answers (0, 0).

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::OnceLock,
};

use mlua::prelude::*;
use serde::Deserialize;

use crate::{
    Engine, K, NameId, Value,
    lua::{Args, Flow, Ret, convert, raise, types::font_to_enum, values},
    scheduler::JobTarget,
    services::{Behaviors, FIRST, method, strings},
    value::FontValue,
};

#[derive(Deserialize)]
struct Font {
    height: f64,
    a: Vec<f64>,
    c: Vec<f64>,
}

fn fonts() -> &'static HashMap<String, Font> {
    static FONTS: OnceLock<HashMap<String, Font>> = OnceLock::new();
    FONTS.get_or_init(|| serde_json::from_slice(include_bytes!("../data/text-advances.json")).expect("data/text-advances.json"))
}

/// FredokaOne, a font games measure their UI with, size by size (data/fredoka-sizes.json, measured in a playing client, Studio 0.747):
/// the advance of each printable ASCII glyph at every integer size `first`.. (a whole number of pixels), a line's height, the kerns
/// of glyph pairs within a text, and the font's ligatures with their kerns. A string's width is its clusters' advances and the
/// kerns between them.
#[derive(Deserialize)]
struct SizedFile {
    first: usize,
    heights: Vec<f64>,
    advances: Vec<Vec<f64>>,
    /// per size: `[first code, second code, kern]` for each pair that moves (whole pixels; a space kerns with nothing). A ligature's
    /// pair holds what the ligature changes: its width is the two advances and that kern.
    kerns: Vec<Vec<(u32, u32, f64)>>,
    /// the pairs the font draws as one glyph, which a line never breaks
    ligatures: Vec<String>,
    /// per size: `[left, right, kern]` between a ligature and a glyph or another ligature (a ligature kerns as a glyph of its own,
    /// not as the letters it joins)
    ligature_kerns: Vec<Vec<(String, String, f64)>>,
}

struct Sized {
    first: usize,
    heights: Vec<f64>,
    advances: Vec<Vec<f64>>,
    /// per size, by cluster ([`Sized::cluster`]): the kern between two clusters next to each other in a text
    kerns: Vec<HashMap<(u32, u32), f64>>,
    ligatures: Vec<(char, char)>,
}

/// A ligature's cluster id: past every char's
const LIGATURE: u32 = 0x11_0000;

impl Sized {
    /// The cluster id of a char or of a ligature named by its two chars
    fn cluster(&self, text: &str) -> u32 {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(one), None) => one as u32,
            (Some(a), Some(b)) => LIGATURE + self.ligatures.iter().position(|&pair| pair == (a, b)).expect("a ligature of data/fredoka-sizes.json") as u32,
            _ => panic!("an empty cluster in data/fredoka-sizes.json"),
        }
    }
}

fn sized() -> &'static Sized {
    static SIZED: OnceLock<Sized> = OnceLock::new();
    SIZED.get_or_init(|| {
        let file: SizedFile = serde_json::from_slice(include_bytes!("../data/fredoka-sizes.json")).expect("data/fredoka-sizes.json");
        let ligatures = file.ligatures.iter().map(|pair| {
            let mut chars = pair.chars();
            (chars.next().expect("a ligature's first char"), chars.next().expect("a ligature's second char"))
        }).collect();
        let mut table = Sized { first: file.first, heights: file.heights, advances: file.advances, kerns: Vec::new(), ligatures };
        let kerns = file
            .kerns
            .iter()
            .zip(&file.ligature_kerns)
            .map(|(pairs, ligature)| {
                let mut kerns: HashMap<(u32, u32), f64> = pairs.iter().map(|&(a, b, kern)| ((a, b), kern)).collect();
                kerns.extend(ligature.iter().map(|(a, b, kern)| ((table.cluster(a), table.cluster(b)), *kern)));
                kerns
            })
            .collect();
        table.kerns = kerns;
        table
    })
}

/// data/text-faces.json (tools/text-faces.mjs, a sweep of every weight and style of the families Enum.Font names, in a playing
/// client): per family file, `"<FontWeight> <FontStyle>"` of a face no Enum.Font names and the Enum.Font face it measures as (`same`),
/// or the one nearest in its widths when it measures as none (`near`, which also holds SourceSansItalic's face: the tables hold
/// GetTextSize's upright answer for it).
#[derive(Deserialize)]
struct FaceFile {
    same: HashMap<String, HashMap<String, String>>,
    near: HashMap<String, HashMap<String, String>>,
}

fn face_file() -> &'static FaceFile {
    static FACES: OnceLock<FaceFile> = OnceLock::new();
    FACES.get_or_init(|| serde_json::from_slice(include_bytes!("../data/text-faces.json")).expect("data/text-faces.json"))
}

/// How GetTextBoundsAsync measures a Font face: with the tables of the Enum.Font face `table`; `exact` when the face measures as that
/// one (else it is the nearest, or SourceSans for a family the tables do not know), and `load`, the face the client loads for it.
pub struct Face {
    pub table: String,
    pub exact: bool,
    pub load: String,
}

pub fn face(engine: &Engine, font: &FontValue) -> Face {
    let reflection = &engine.reflection;
    let family = font.family.to_str_lossy();
    let key = format!("{} {}", reflection.enum_item_info(font.weight).name, reflection.enum_item_info(font.style).name);
    let own = || format!("{family} {key}");
    let file = face_file();
    if let Some(name) = file.near.get(&*family).and_then(|faces| faces.get(&key)) {
        return Face { table: name.clone(), exact: false, load: own() };
    }
    if let Some(item) = font_to_enum(engine, font) {
        let name = &*reflection.enum_item_info(item).name;
        if fonts().contains_key(name) {
            return Face { table: name.to_string(), exact: true, load: own() };
        }
    }
    if let Some(name) = file.same.get(&*family).and_then(|faces| faces.get(&key)) {
        // (the client loads the face it measures as: FredokaOne Bold needs no load once FredokaOne has one)
        let load = match reflection.font_enum_faces.get(name.as_str()) {
            Some((family, weight, style)) => format!("{family} {weight} {style}"),
            None => own(),
        };
        return Face { table: name.clone(), exact: true, load };
    }
    Face { table: "SourceSans".to_string(), exact: false, load: own() }
}

/// The faces a playing client has loaded: the Enum.Font faces it starts with (a call from a metamethod, which cannot yield, measured
/// them; every other face raised "thread is not yieldable"), then those calls loaded.
struct LoadedFaces(RefCell<HashSet<String>>);

impl crate::Extension for LoadedFaces {}

const LOADED_AT_START: &[&str] = &["SourceSans", "SourceSansBold", "SourceSansItalic", "SourceSansLight", "SourceSansSemibold", "Arial", "ArialBold", "Arimo", "ArimoBold"];

/// Marks the face `load` loaded; whether it was already.
fn loaded(engine: &Engine, load: &str) -> bool {
    let faces = engine.extension_or_insert_with(|| {
        let faces = &engine.reflection.font_enum_faces;
        LoadedFaces(RefCell::new(LOADED_AT_START.iter().filter_map(|name| faces.get(*name)).map(|(family, weight, style)| format!("{family} {weight} {style}")).collect()))
    });
    !faces.0.borrow_mut().insert(load.to_string())
}

/// The column of FredokaOne's table for `size`, when the font is FredokaOne and the size is a whole number the table covers.
fn column(font: &str, size: f64) -> Option<usize> {
    let table = sized();
    if font != "FredokaOne" || size.fract() != 0.0 || size < table.first as f64 {
        return None;
    }
    let column = size as usize - table.first;
    (column < table.heights.len()).then_some(column)
}

/// The marks that kern with the glyph after them only once a letter came before them in the text: before the text's first letter,
/// a client shapes them apart from it (".T" alone, after a space or a digit or at a paragraph's start is unkerned; "a.T", "a .T",
/// "a1.T" and "a\n.T" kern). They are the only glyphs before a letter that kern.
const MARKS: [char; 4] = ['"', '\'', ',', '.'];

/// Lays `paragraph` out on one line: `pens` gets each cluster's pen position and then the line's end (one more than the clusters),
/// `spaces` whether each cluster is a space. A cluster is a glyph, or one of FredokaOne's ligatures (`column`: its table's size),
/// which a line never breaks; FredokaOne's clusters kern. `lettered`: whether a letter came before, in the text's earlier
/// paragraphs too ([`MARKS`]).
fn shape(font: &Font, column: Option<usize>, paragraph: &str, size: f64, lettered: &mut bool, pens: &mut Vec<f64>, spaces: &mut Vec<bool>) {
    pens.clear();
    spaces.clear();
    let mut pen = 0.0;
    let Some(column) = column else {
        let count = font.a.len() as f64;
        let (mean_a, mean_c) = (font.a.iter().sum::<f64>() / count, font.c.iter().sum::<f64>() / count);
        for character in paragraph.chars() {
            let (a, c) = match (character as u32).checked_sub(32).filter(|index| *index < 95) {
                Some(index) => (font.a[index as usize], font.c[index as usize]),
                None => (mean_a, mean_c),
            };
            pens.push(pen);
            spaces.push(character == ' ');
            pen += (a * size + c + 0.5).floor().max(0.0);
        }
        pens.push(pen);
        return;
    };
    let table = sized();
    let advance = |character: char| match (character as u32).checked_sub(32).filter(|index| *index < 95) {
        Some(index) => table.advances[index as usize][column],
        None => table.advances.iter().map(|row| row[column]).sum::<f64>() / table.advances.len() as f64,
    };
    let kerns = &table.kerns[column];
    let mut chars = paragraph.chars().peekable();
    let mut before: Option<(u32, char)> = None;
    while let Some(character) = chars.next() {
        let ligature = chars.peek().and_then(|&next| table.ligatures.iter().position(|&pair| pair == (character, next)).map(|index| (index, next)));
        let (cluster, width) = match ligature {
            Some((index, next)) => {
                chars.next();
                let kern = kerns.get(&(character as u32, next as u32)).copied().unwrap_or(0.0);
                (LIGATURE + index as u32, advance(character) + advance(next) + kern)
            }
            None => (character as u32, advance(character)),
        };
        if let Some((left, last)) = before {
            if *lettered || !MARKS.contains(&last) {
                pen += kerns.get(&(left, cluster)).copied().unwrap_or(0.0);
            }
        }
        pens.push(pen);
        spaces.push(character == ' ');
        pen += width;
        *lettered |= character.is_alphabetic();
        before = Some((cluster, if ligature.is_some() { '\0' } else { character }));
    }
    pens.push(pen);
}

/// (width, height) of `text` at `size` in `font` (an Enum.Font item's name) inside a frame `limit` wide, as a playing client lays it
/// out (Studio 0.747, roblox-sim: 24,000 texts of FredokaOne, and sweeps of SourceSans, GothamBold, Arial and Legacy). Each paragraph
/// (a newline ends one) is laid out on one line, kerned, and broken greedily: a line's width is the distance from its first cluster's
/// pen position to the next line's, so the kern across a break counts on the line before it. A cluster that does not fit goes to the
/// next line, after the spaces before it when the clusters from there to it fit a line of their own (the spaces before the break do
/// not count in the line's width), else right before it (the spaces count: a word wider than the frame breaks between clusters, after
/// what fits on the line it starts on). A paragraph whose first cluster is wider than the frame starts with an empty line. The last
/// line's trailing spaces count when they fit, or when its other clusters do not fit either, or when it holds only spaces.
pub fn measure(font: &str, text: &str, size: f64, limit: f64) -> (f64, f64) {
    let table = fonts();
    let column = column(font, size);
    let Some(font) = table.get(font).or_else(|| table.get("SourceSans")) else { return (0.0, size) };
    let line_height = match column {
        Some(column) => sized().heights[column],
        None => (font.height * size + 1e-6).floor(),
    };
    let (mut pens, mut spaces) = (Vec::new(), Vec::new());
    let mut lettered = false;
    let mut widest: f64 = 0.0;
    let mut lines = 0.0;
    for paragraph in text.split('\n') {
        lines += 1.0;
        shape(font, column, paragraph, size, &mut lettered, &mut pens, &mut spaces);
        let count = spaces.len();
        if count == 0 {
            continue;
        }
        let width = |from: usize, to: usize| pens[to] - pens[from];
        let trimmed = |from: usize, mut to: usize| {
            while to > from && spaces[to - 1] {
                to -= 1;
            }
            width(from, to)
        };
        if !spaces[0] && width(0, 1) > limit {
            lines += 1.0;
        }
        let mut start = 0;
        // (where the line could break: after its last run of spaces)
        let mut opening = 0;
        for index in 0..count {
            if spaces[index] {
                opening = index + 1;
                continue;
            }
            if index > start && width(start, index + 1) > limit {
                if opening > start && width(opening, index + 1) <= limit {
                    widest = widest.max(trimmed(start, opening));
                    start = opening;
                } else {
                    widest = widest.max(width(start, index));
                    start = index;
                }
                lines += 1.0;
            }
        }
        let (full, content) = (width(start, count), trimmed(start, count));
        let only_spaces = spaces[start..].iter().all(|space| *space);
        widest = widest.max(if full <= limit || content > limit || only_spaces { full } else { content });
    }
    ((widest + 0.5).floor(), lines * line_height)
}

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let Some(class) = reflection.class_id("TextService") else { return };
    if let Some(name) = reflection.name_id("GetTextBoundsAsync") {
        behaviors.add_method(class, name, method(get_text_bounds));
    }
    let Some(name) = reflection.name_id("GetTextSize") else { return };
    // (its text read as a client reads a string parameter, on a server too: services::strings)
    behaviors.add_method(class, name, strings(FIRST, method(|lua, engine, _, args| {
        if engine.options.server() {
            // (a server has no typesetting: every size is (0, 0), measured)
            return Ok(values([convert::to_lua(lua, engine, &Value::Vector2(rbx_dom_weak::types::Vector2::new(0.0, 0.0)))?]));
        }
        let text = args.string(1).map(|text| text.to_string_lossy()).unwrap_or_default();
        let size = args.number(2).unwrap_or(14.0);
        let font = match convert::from_lua(args.get(3)) {
            Value::EnumItem(item) => engine.reflection.enum_item_info(item).name.to_string(),
            _ => "SourceSans".to_string(),
        };
        let limit = match convert::from_lua(args.get(4)) {
            Value::Vector2(frame) if frame.x > 0.0 => frame.x as f64,
            _ => f64::INFINITY,
        };
        engine.state.borrow().sched.charge(crate::cost::TEXT_MEASURED);
        let (x, y) = measure(&font, &text, size, limit);
        let size = Value::Vector2(rbx_dom_weak::types::Vector2::new(x as f32, y as f32));
        Ok(values([convert::to_lua(lua, engine, &size)?]))
    })));
}

/// TextService:GetTextBoundsAsync (measured in a playing client and a server, Studio 0.747: tools/parity/probes/text_bounds.luau).
fn get_text_bounds(lua: &Lua, engine: &Rc<Engine>, service: crate::InstanceId, args: &Args) -> Ret {
    // (`TextService.GetTextBoundsAsync(params)`: the params in the service's place)
    if !engine.reflection.is_a(engine.state.borrow().dom.at(service).class, "TextService") {
        return raise("Expected ':' not '.' calling member function GetTextBoundsAsync");
    }
    let params = match args.get(1) {
        LuaValue::Nil => return raise("Argument 1 missing or nil"),
        value => match crate::lua::instance_id(value) {
            Some(params) => params,
            None => return raise("Unable to cast value to Object"),
        },
    };
    let class = engine.state.borrow().dom.at(params).class;
    if !engine.reflection.is_a(class, "GetTextBoundsParams") {
        return raise("Argument 1 is invalid: expected 'GetTextBoundsParams' instance type");
    }
    if engine.options.server() {
        return raise("Typesetting is not enabled in this DataModel");
    }
    let (text, font, size, width, rich) = {
        let state = engine.state.borrow();
        let get = |name: NameId| state.dom.raw_get(params, name);
        let text = get(K::Text as NameId).and_then(Value::as_str).map(|text| text.to_str_lossy().into_owned()).unwrap_or_default();
        let font = match get(K::Font as NameId) {
            Some(Value::Font(font)) => (**font).clone(),
            _ => crate::lua::types::font_from_enum(engine, engine.reflection.enum_item("Font", "SourceSans").unwrap_or(0)),
        };
        let size = get(K::Size as NameId).and_then(Value::as_number).unwrap_or(20.0);
        let width = engine.reflection.name_id("Width").and_then(get).and_then(Value::as_number).unwrap_or(0.0);
        (text, font, size, width, matches!(get(K::RichText as NameId), Some(Value::Bool(true))))
    };
    if font.family.as_bytes().is_empty() {
        return raise("Font with empty asset ID passed to TextService:GetTextBoundsAsync()");
    }
    let face = face(engine, &font);
    if !face.exact {
        let missing = format!("GetTextBoundsAsync of {} (a face the text tables lack; measured as {})", face.load, face.table);
        engine.missing(Some(lua), "TextService", &missing);
    }
    if rich && text.contains(['<', '&']) {
        // (rich text needs a host layer's parser: Studio reads the tags, entities and <br/>, and raises on markup that does not parse)
        engine.missing(Some(lua), "TextService", "GetTextBoundsAsync of rich text markup (measured as plain text)");
    }
    engine.state.borrow().sched.charge(crate::cost::TEXT_MEASURED);
    // (Size stores 1 to 100, whole: convert::number_for; a value from elsewhere measures within them)
    let size = if size.is_nan() { 1.0 } else { size.trunc().clamp(1.0, 100.0) };
    let limit = if width > 0.0 { width } else { f64::INFINITY };
    let (x, y) = measure(&face.table, &text, size, limit);
    let bounds = Value::Vector2(rbx_dom_weak::types::Vector2::new(x as f32, y as f32));
    if loaded(engine, &face.load) {
        return Ok(values([convert::to_lua(lua, engine, &bounds)?]));
    }
    // The first call of a face waits for it to load (a call that cannot yield raises, and the load goes on: a later call finds it
    // loaded). The client resumes the call when the load is done, which depends on the machine; here at the next resumption point.
    let thread = lua.current_thread();
    crate::lua::refuse_unyieldable(&thread, crate::lua::NOT_YIELDABLE)?;
    engine.state.borrow_mut().sched.defer(JobTarget::Thread(thread), vec![bounds]);
    Err(Flow::Yield)
}

#[cfg(test)]
mod tests {
    use super::{face_file, fonts, measure};

    /// data/text-faces.json names only faces the tables hold, and no face twice; FredokaOne has one face (every weight and style
    /// measures as it), GothamSSm's SemiBold is a face of its own (measured as no Enum.Font face).
    #[test]
    fn text_faces_measure_as_faces_the_tables_hold() {
        let file = face_file();
        for (family, faces) in file.same.iter().chain(&file.near) {
            for (face, name) in faces {
                assert!(fonts().contains_key(name), "{family} {face}: {name}");
            }
        }
        for (family, faces) in &file.same {
            for face in faces.keys() {
                assert!(!file.near.get(family).is_some_and(|near| near.contains_key(face)), "{family} {face} is in both");
            }
        }
        let fredoka = &file.same["rbxasset://fonts/families/FredokaOne.json"];
        assert_eq!(fredoka.len(), 17);
        assert!(fredoka.values().all(|name| name == "FredokaOne"));
        assert_eq!(file.near["rbxasset://fonts/families/GothamSSm.json"]["SemiBold Normal"], "GothamBold");
        assert_eq!(file.near["rbxasset://fonts/families/SourceSansPro.json"]["Regular Italic"], "SourceSansLight");
    }

    /// Studio's answers (an empty test place, 2026-10-23, the text_advances probe): single lines at sizes 14 and 20 in an unbounded
    /// frame, and the 62 characters of "The quick brown fox jumps over the lazy dog and keeps running" in frames 40-220 px wide.
    /// `(font, text, size, frame, width, height)`.
    const STUDIO: &[(&str, &str, f64, f64, f64, f64)] = &[
        ("FredokaOne", "Hello", 14.0, 1e5, 31.0, 14.0),
        ("FredokaOne", "Hello", 20.0, 1e5, 43.0, 20.0),
        ("FredokaOne", "WWW", 14.0, 1e5, 36.0, 14.0),
        ("FredokaOne", "iii", 14.0, 1e5, 9.0, 14.0),
        ("FredokaOne", "1,234,567", 20.0, 1e5, 76.0, 20.0),
        ("FredokaOne", "Level 12 - Wave 30/40", 14.0, 1e5, 135.0, 14.0),
        ("SourceSans", "Hello", 14.0, 1e5, 26.0, 14.0),
        ("Legacy", "Hello", 14.0, 1e5, 44.0, 21.0),
        ("Legacy", "Hello", 20.0, 1e5, 62.0, 30.0),
        ("SourceSans", "The quick brown fox jumps over the lazy dog and keeps running", 14.0, 40.0, 40.0, 154.0),
        ("SourceSans", "The quick brown fox jumps over the lazy dog and keeps running", 14.0, 100.0, 98.0, 56.0),
        ("SourceSans", "The quick brown fox jumps over the lazy dog and keeps running", 20.0, 100.0, 82.0, 120.0),
        ("SourceSans", "The quick brown fox jumps over the lazy dog and keeps running", 14.0, 220.0, 216.0, 28.0),
        ("FredokaOne", "The quick brown fox jumps over the lazy dog and keeps running", 20.0, 150.0, 138.0, 80.0),
        ("FredokaOne", "The quick brown fox jumps over the lazy dog and keeps running", 14.0, 100.0, 98.0, 56.0),
        ("GothamBold", "The quick brown fox jumps over the lazy dog and keeps running", 20.0, 220.0, 216.0, 60.0),
        ("SourceSans", "Hello world again and again", 20.0, 100.0, 79.0, 60.0),
    ];

    /// TextService:GetTextSize returned nil without a host's text layer (a script reading `.X` failed: the lobby's settings window); it
    /// answers from Studio's own measurements now. Widths within 3 px (the fit's 95th percentile) and the same number of lines.
    #[test]
    fn text_service_measures_text_as_studio_does() {
        for &(font, text, size, frame, width, height) in STUDIO {
            let (x, y) = measure(font, text, size, frame);
            assert!((x - width).abs() <= 3.0, "{font} {text:?} {size} in {frame}: width {x}, Studio {width}");
            assert_eq!(y, height, "{font} {text:?} {size} in {frame}: height {y}, Studio {height}");
        }
    }

    /// FredokaOne at the integer sizes UI uses, from Studio's own table (data/fredoka-sizes.json, the fredoka_sizes probe in an
    /// empty test place, 2026-10-23; the same in a playing client, 2026-11-27): a glyph's advance is a whole number of pixels and pairs
    /// kern by whole pixels. The per-glyph fit (data/text-advances.json) read 57 % of these samples exactly and was up to 8 px wide on
    /// "The quick brown fox..." at 48; the table read 694 of 704 exactly, the 10 others ("Sett", "All") 1 px off where a ligature (tt,
    /// ll) met a glyph it kerns with as a letter but not as a ligature. `(text, size, width, height)`, all pinned exactly.
    const FREDOKA: &[(&str, f64, f64, f64)] = &[
        ("Hello", 10.0, 22.0, 10.0),
        ("Hello", 13.0, 29.0, 13.0),
        ("Hello", 15.0, 33.0, 15.0),
        ("Hello", 16.0, 35.0, 16.0),
        ("Hello", 24.0, 51.0, 24.0),
        ("Hello", 36.0, 76.0, 36.0),
        ("Hello", 48.0, 100.0, 48.0),
        ("The quick brown fox jumps over the lazy dog", 10.0, 191.0, 10.0),
        ("The quick brown fox jumps over the lazy dog", 13.0, 252.0, 13.0),
        ("The quick brown fox jumps over the lazy dog", 15.0, 284.0, 15.0),
        ("The quick brown fox jumps over the lazy dog", 16.0, 306.0, 16.0),
        ("The quick brown fox jumps over the lazy dog", 24.0, 443.0, 24.0),
        ("The quick brown fox jumps over the lazy dog", 36.0, 662.0, 36.0),
        ("The quick brown fox jumps over the lazy dog", 48.0, 869.0, 48.0),
        ("Level 12 - Wave 30/40", 10.0, 98.0, 10.0),
        ("Level 12 - Wave 30/40", 13.0, 125.0, 13.0),
        ("Level 12 - Wave 30/40", 15.0, 140.0, 15.0),
        ("Level 12 - Wave 30/40", 16.0, 155.0, 16.0),
        ("Level 12 - Wave 30/40", 24.0, 222.0, 24.0),
        ("Level 12 - Wave 30/40", 36.0, 331.0, 36.0),
        ("Level 12 - Wave 30/40", 48.0, 434.0, 48.0),
        ("Wa Yo Ty AV", 10.0, 52.0, 10.0),
        ("Wa Yo Ty AV", 13.0, 71.0, 13.0),
        ("Wa Yo Ty AV", 15.0, 78.0, 15.0),
        ("Wa Yo Ty AV", 16.0, 85.0, 16.0),
        ("Wa Yo Ty AV", 24.0, 123.0, 24.0),
        ("Wa Yo Ty AV", 36.0, 184.0, 36.0),
        ("Wa Yo Ty AV", 48.0, 244.0, 48.0),
        ("Join Match", 10.0, 48.0, 10.0),
        ("Join Match", 13.0, 61.0, 13.0),
        ("Join Match", 15.0, 70.0, 15.0),
        ("Join Match", 16.0, 75.0, 16.0),
        ("Join Match", 24.0, 109.0, 24.0),
        ("Join Match", 36.0, 163.0, 36.0),
        ("Join Match", 48.0, 213.0, 48.0),
        ("Unit Limit: 6/6", 10.0, 61.0, 10.0),
        ("Unit Limit: 6/6", 13.0, 78.0, 13.0),
        ("Unit Limit: 6/6", 15.0, 88.0, 15.0),
        ("Unit Limit: 6/6", 16.0, 97.0, 16.0),
        ("Unit Limit: 6/6", 24.0, 141.0, 24.0),
        ("Unit Limit: 6/6", 36.0, 210.0, 36.0),
        ("Unit Limit: 6/6", 48.0, 275.0, 48.0),
    ];

    /// A playing client's FredokaOne layouts (Studio 0.747, roblox-sim, GetTextSize), each one ree measured otherwise before:
    /// sizes 1-3 (the table started at 4), ligatures (never broken, kerned as glyphs of their own), the marks before a text's first
    /// letter (unkerned), a word wider than the frame breaking after what fits and never inside a ligature, a line's width up to the
    /// next line's pen position, the empty first line of a frame narrower than the first glyph, and trailing spaces.
    /// `(text, size, frame, width, height)`.
    const CLIENT: &[(&str, f64, f64, f64, f64)] = &[
        ("Hello", 1.0, 1e5, 4.0, 1.0),
        ("The quick brown fox jumps over the lazy dog", 1.0, 1e5, 33.0, 1.0),
        ("Hello", 2.0, 1e5, 6.0, 2.0),
        ("Hello", 3.0, 1e5, 8.0, 3.0),
        ("All", 3.0, 1e5, 5.0, 3.0),
        ("Sett", 3.0, 1e5, 7.0, 3.0),
        ("lll", 20.0, 1e5, 17.0, 20.0),
        ("Alll", 20.0, 1e5, 30.0, 20.0),
        ("affluent", 20.0, 1e5, 68.0, 20.0),
        ("Office", 48.0, 1e5, 117.0, 48.0),
        ("fft", 48.0, 1e5, 51.0, 48.0),
        (".T", 20.0, 1e5, 17.0, 20.0),
        ("..T", 20.0, 1e5, 21.0, 20.0),
        ("1.T", 20.0, 1e5, 24.0, 20.0),
        ("\"A", 48.0, 1e5, 48.0, 48.0),
        ("\"\"A", 20.0, 1e5, 27.0, 20.0),
        (".\"A", 20.0, 1e5, 24.0, 20.0),
        ("(.T", 48.0, 1e5, 56.0, 48.0),
        ("\n.T", 20.0, 1e5, 17.0, 40.0),
        ("Hello", 20.0, 21.0, 21.0, 60.0),
        ("Hello", 20.0, 28.0, 22.0, 40.0),
        ("Hello", 20.0, 33.0, 33.0, 40.0),
        ("Hello", 20.0, 16.0, 12.0, 80.0),
        ("ll", 20.0, 9.0, 11.0, 40.0),
        ("T.T", 20.0, 11.0, 13.0, 40.0),
        ("Wa.", 20.0, 14.0, 16.0, 60.0),
        ("WWWWW", 20.0, 1.0, 17.0, 120.0),
        ("WWWWW", 20.0, 16.0, 17.0, 120.0),
        ("iiiiiiiiii", 20.0, 3.0, 4.0, 220.0),
        ("e", 20.0, 9.0, 10.0, 40.0),
        ("Ho", 20.0, 11.0, 12.0, 60.0),
        ("a b", 20.0, 9.0, 14.0, 60.0),
        ("W i", 20.0, 3.0, 21.0, 60.0),
        ("W i", 20.0, 4.0, 17.0, 60.0),
        ("Hello ", 20.0, 9.0, 14.0, 100.0),
        ("Hello ", 20.0, 10.0, 12.0, 100.0),
        ("Hello ", 20.0, 22.0, 22.0, 40.0),
        ("Hello ", 20.0, 25.0, 25.0, 40.0),
        (" ", 20.0, 1.0, 4.0, 20.0),
        ("  ", 14.0, 3.0, 6.0, 14.0),
        ("  Hello  ", 20.0, 12.0, 12.0, 100.0),
        ("  Hello  ", 20.0, 20.0, 20.0, 80.0),
        ("  Hello  ", 20.0, 30.0, 30.0, 40.0),
        ("ab cd", 20.0, 8.0, 14.0, 100.0),
        ("ab cd", 20.0, 9.0, 10.0, 100.0),
        ("The quick brown fox jumps over the lazy dog and keeps running", 14.0, 1.0, 11.0, 714.0),
        ("The quick brown fox jumps over the lazy dog and keeps running", 14.0, 12.0, 12.0, 616.0),
    ];

    #[test]
    fn fredoka_one_lays_out_as_a_client_does() {
        for &(text, size, frame, width, height) in CLIENT {
            let (x, y) = measure("FredokaOne", text, size, frame);
            assert_eq!((x, y), (width, height), "FredokaOne {text:?} at {size} in {frame}: {x} x {y}, a client {width} x {height}");
        }
        // a letter in an earlier paragraph lets a mark kern: "a\n.T" is 14 wide where "\n.T" is 17
        assert_eq!(measure("FredokaOne", "a\n.T", 20.0, 1e5), (14.0, 40.0));
    }

    /// The table names only ligatures whose pairs it measures, and each size holds every ligature's kerns (the cluster ids resolve).
    #[test]
    fn the_fredoka_table_covers_sizes_1_to_100_with_its_ligatures() {
        let table = super::sized();
        assert_eq!((table.first, table.heights.len()), (1, 100));
        assert!(table.advances.iter().all(|row| row.len() == 100));
        assert_eq!(table.ligatures.iter().map(|(a, b)| format!("{a}{b}")).collect::<Vec<_>>(), ["ff", "fi", "fl", "ft", "jj", "ll", "tt"]);
        assert_eq!(table.heights[..3], [1.0, 2.0, 3.0]);
    }

    #[test]
    fn fredoka_one_is_exact_at_the_sizes_the_game_measures() {
        for &(text, size, width, height) in FREDOKA {
            let (x, y) = measure("FredokaOne", text, size, 1e5);
            assert_eq!((x, y), (width, height), "FredokaOne {text:?} at {size}: {x} x {y}, Studio {width} x {height}");
        }
        // a size the table does not hold (a fraction) falls back to the per-glyph fit
        let (x, _) = measure("FredokaOne", "Hello", 14.5, 1e5);
        assert!((x - 33.0).abs() <= 3.0, "{x}");
    }
}
