//! Rojo's JSON models (`*.model.json`, https://rojo.space/docs/v7/sync-details/#json-models): an
//! instance tree written by hand, which Rojo puts in the place as one instance named after the file
//! (a project RemoteEvent is often just `{"className": "RemoteEvent"}`).
//!
//! The format is Rojo 7's (its src/snapshot_middleware/json_model.rs and src/resolution.rs):
//! `className` (or `ClassName`); `name` / `Name`, a child's name, its class name when absent (the top
//! instance takes the file's name and its own is ignored, as since Rojo 6); `properties` /
//! `Properties`; `attributes`; `children` / `Children`. A property value is explicit, rbx-dom's form
//! of a value (`{"Vector3": [1, 2, 3]}`), or implicit, read by the property's type in rbx-dom's
//! reflection database: a string for a string, Content or enum item name, a number, a bool, `[x, y]` /
//! `[x, y, z]` for vectors and colours, twelve numbers for a CFrame, an array of strings for Tags, a
//! Font object. A property rbx-dom's database does not have (it is older than the engine's Studio
//! dump: InputBinding.Type, SurfaceAppearance.ResampleMode) is read by the engine's reflection, and
//! an enum item name by the engine's enum (KeyCode.MouseWheel), as the Rojo release that knows them
//! reads them. An attribute is explicit, a string, a number or a bool. A file holding only white
//! space makes nothing; one Rojo would reject (not a model, an unknown property given implicitly, a
//! value of the wrong type) makes nothing either and is reported.
//!
//! The tree then goes through rbx_binary and back, as `rojo build` writes it into a place: a property
//! that does not serialize is dropped (a part's Position: its CFrame holds it), a part's Color is kept
//! as Color3uint8, and the result converts as any model file does ([`super::ModelPlan`]).

use rustc_hash::FxHashMap;

use rbx_dom_weak::{
    InstanceBuilder, WeakDom,
    types::{Attributes, CFrame, Color3, Content, ContentId, Enum, Font, MaterialColors, Matrix3, Tags, Variant, VariantType, Vector2, Vector3},
};
use rbx_reflection::DataType;
use serde::Deserialize;

use crate::Reflection;

// (fields in Rojo's order and of its types: one of another type is refused alike, the first in order first)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonModel {
    #[serde(rename = "$schema")]
    #[allow(dead_code)]
    schema: Option<String>,
    #[serde(alias = "Name")]
    name: Option<String>,
    #[serde(alias = "ClassName")]
    class_name: String,
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(alias = "Children", default)]
    children: Vec<JsonModel>,
    #[serde(alias = "Properties", default)]
    properties: FxHashMap<String, UnresolvedValue>,
    #[serde(default)]
    attributes: FxHashMap<String, UnresolvedValue>,
}

/// A value as written: explicit (rbx-dom's tagged form) or one whose type the property decides. (Named as Rojo's
/// resolution.rs names it, which serde's "data did not match any variant of untagged enum UnresolvedValue" says.)
#[derive(Deserialize)]
#[serde(untagged)]
enum UnresolvedValue {
    Explicit(Variant),
    Implicit(Implicit),
}

/// The shapes an implicit value can take, tried in Rojo's order (its AmbiguousValue, whose variant names its Debug form
/// prints in "Cannot unambiguously resolve the value Array2([1.0, 2.0])").
#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum Implicit {
    Bool(bool),
    String(String),
    StringArray(Vec<String>),
    Number(f64),
    Array2([f64; 2]),
    Array3([f64; 3]),
    // (no property takes four numbers: read so that one given is reported as such, as Rojo does)
    #[allow(dead_code)]
    Array4([f64; 4]),
    Array12([f64; 12]),
    Attributes(Attributes),
    Font(Font),
    MaterialColors(MaterialColors),
}

impl Implicit {
    fn describe(&self) -> &'static str {
        match self {
            Implicit::Bool(_) => "a bool",
            Implicit::String(_) => "a string",
            Implicit::StringArray(_) => "an array of strings",
            Implicit::Number(_) => "a number",
            Implicit::Array2(_) => "an array of two numbers",
            Implicit::Array3(_) => "an array of three numbers",
            Implicit::Array4(_) => "an array of four numbers",
            Implicit::Array12(_) => "an array of twelve numbers",
            Implicit::Attributes(_) => "an object containing attributes",
            Implicit::Font(_) => "an object describing a Font",
            Implicit::MaterialColors(_) => "an object describing MaterialColors",
        }
    }

    /// The value of `class_name.property` this stands for, by the property's type (its errors in Rojo's words).
    fn resolve(self, class_name: &str, property: &str) -> Result<Variant, String> {
        match property_type(class_name, property).ok_or_else(|| format!("Unknown property {class_name}.{property}"))? {
            PropertyType::Enum(enum_name) => {
                let wrong = |what: &str| format!("Invalid value for property {class_name}.{property}. Got {what} but expected a member of the {enum_name} enum such as {}", enum_examples(&enum_name));
                match self {
                    Implicit::String(item) => enum_item_value(&enum_name, &item).map(|value| Enum::from_u32(value).into()).ok_or_else(|| wrong(&item)),
                    other => Err(wrong(other.describe())),
                }
            }
            PropertyType::Unknown => Err(format!("Unknown data type for property {class_name}.{property}")),
            PropertyType::Value(kind) => match (kind, self) {
                (VariantType::Bool, Implicit::Bool(value)) => Ok(value.into()),
                (VariantType::Float32, Implicit::Number(value)) => Ok((value as f32).into()),
                (VariantType::Float64, Implicit::Number(value)) => Ok(value.into()),
                (VariantType::Int32, Implicit::Number(value)) => Ok((value as i32).into()),
                (VariantType::Int64, Implicit::Number(value)) => Ok((value as i64).into()),
                (VariantType::String, Implicit::String(value)) => Ok(value.into()),
                (VariantType::Tags, Implicit::StringArray(value)) => Ok(Tags::from(value).into()),
                (VariantType::Content, Implicit::String(value)) => Ok(Content::from(value).into()),
                (VariantType::ContentId, Implicit::String(value)) => Ok(ContentId::from(value).into()),
                (VariantType::Vector2, Implicit::Array2(v)) => Ok(Vector2::new(v[0] as f32, v[1] as f32).into()),
                (VariantType::Vector3, Implicit::Array3(v)) => Ok(Vector3::new(v[0] as f32, v[1] as f32, v[2] as f32).into()),
                (VariantType::Color3, Implicit::Array3(v)) => Ok(Color3::new(v[0] as f32, v[1] as f32, v[2] as f32).into()),
                (VariantType::CFrame, Implicit::Array12(v)) => {
                    let v = v.map(|n| n as f32);
                    let axes = Matrix3::new(Vector3::new(v[3], v[4], v[5]), Vector3::new(v[6], v[7], v[8]), Vector3::new(v[9], v[10], v[11]));
                    Ok(CFrame::new(Vector3::new(v[0], v[1], v[2]), axes).into())
                }
                (VariantType::Attributes, Implicit::Attributes(value)) => Ok(value.into()),
                (VariantType::Font, Implicit::Font(value)) => Ok(value.into()),
                (VariantType::MaterialColors, Implicit::MaterialColors(value)) => Ok(value.into()),
                (VariantType::Ref, Implicit::String(_)) => Err(format!("Cannot resolve Ref properties as a String.Use an attribute named `Rojo_Target_{property}")),
                (kind, other) => Err(format!("Wrong type of value for property {class_name}.{property}. Expected {kind:?}, got {}", other.describe())),
            },
        }
    }
}

/// What an implicit value of a property is read as.
enum PropertyType {
    Enum(String),
    Value(VariantType),
    Unknown,
}

/// `class_name.property`'s type: rbx-dom's descriptor, as Rojo resolves it, else the engine's reflection for a property
/// rbx-dom's (older) database does not have.
fn property_type(class_name: &str, property: &str) -> Option<PropertyType> {
    if let Some(descriptor) = crate::record::encode::dom_property(class_name, property) {
        return Some(match &descriptor.data_type {
            DataType::Enum(enum_name) => PropertyType::Enum(enum_name.to_string()),
            DataType::Value(kind) => PropertyType::Value(*kind),
            _ => PropertyType::Unknown,
        });
    }
    let reflection = Reflection::shared().ok()?;
    let info = reflection.class_property(reflection.class_id(class_name)?, reflection.name_id(property)?)?;
    if let Some(enum_name) = &info.enum_name {
        return Some(PropertyType::Enum(enum_name.to_string()));
    }
    Some(match &*info.engine_type_name {
        "bool" => PropertyType::Value(VariantType::Bool),
        "float" => PropertyType::Value(VariantType::Float32),
        "double" => PropertyType::Value(VariantType::Float64),
        "int" => PropertyType::Value(VariantType::Int32),
        "int64" => PropertyType::Value(VariantType::Int64),
        "string" => PropertyType::Value(VariantType::String),
        "Content" => PropertyType::Value(VariantType::Content),
        "ContentId" => PropertyType::Value(VariantType::ContentId),
        "Vector2" => PropertyType::Value(VariantType::Vector2),
        "Vector3" => PropertyType::Value(VariantType::Vector3),
        "Color3" => PropertyType::Value(VariantType::Color3),
        "CoordinateFrame" => PropertyType::Value(VariantType::CFrame),
        "Font" => PropertyType::Value(VariantType::Font),
        _ => PropertyType::Unknown,
    })
}

/// The value of the item named `item` of `enum_name`: the engine's enum first (it has the items a newer Studio added),
/// then rbx-dom's.
fn enum_item_value(enum_name: &str, item: &str) -> Option<u32> {
    if let Ok(reflection) = Reflection::shared() {
        if let Some(id) = reflection.enum_item(enum_name, item) {
            return Some(reflection.enum_item_info(id).value as u32);
        }
    }
    let database = rbx_reflection_database::get().ok()?;
    database.enums.get(enum_name)?.items.get(item).copied()
}

/// Rojo's list of an enum's items in an error (nonexhaustive_list over rbx-dom's items in byte order): the first nine, then
/// "or <the last>" for up to eight items, "or <n> more" past that
fn enum_examples(enum_name: &str) -> String {
    let mut items: Vec<&str> = rbx_reflection_database::get().ok().and_then(|database| database.enums.get(enum_name)).map(|descriptor| descriptor.items.keys().map(|item| item.as_ref()).collect()).unwrap_or_default();
    items.sort();
    let Some(last) = items.last() else { return String::new() };
    let shown = (items.len() - 1).min(9);
    let mut text: String = items[..shown].iter().map(|item| format!("{item}, ")).collect();
    if items.len() > 8 {
        text.push_str(&format!("or {} more", items.len() - shown));
    } else {
        text.push_str(&format!("or {last}"));
    }
    text
}

impl UnresolvedValue {
    fn resolve(self, class_name: &str, property: &str) -> Result<Variant, String> {
        match self {
            UnresolvedValue::Explicit(value) => Ok(value),
            UnresolvedValue::Implicit(value) => value.resolve(class_name, property),
        }
    }

    /// An attribute's value: explicit, or a string, a number (Float64) or a bool (Rojo's resolve_unambiguous).
    fn resolve_attribute(self) -> Result<Variant, String> {
        match self {
            UnresolvedValue::Explicit(value) => Ok(value),
            UnresolvedValue::Implicit(Implicit::Bool(value)) => Ok(value.into()),
            UnresolvedValue::Implicit(Implicit::Number(value)) => Ok(value.into()),
            UnresolvedValue::Implicit(Implicit::String(value)) => Ok(value.into()),
            UnresolvedValue::Implicit(other) => Err(format!("Cannot unambiguously resolve the value {other:?}")),
        }
    }
}

/// The instance `model` describes, named `name`.
fn build(model: JsonModel, name: String) -> Result<InstanceBuilder, String> {
    let JsonModel { class_name, children, properties, attributes, .. } = model;
    let mut builder = InstanceBuilder::new(class_name.as_str()).with_name(name);
    let mut properties: Vec<(String, UnresolvedValue)> = properties.into_iter().collect();
    crate::sort::by(&mut properties, |(a, _), (b, _)| a.cmp(b));
    for (property, value) in properties {
        let value = value.resolve(&class_name, &property)?;
        builder.add_property(property.as_str(), value);
    }
    if !attributes.is_empty() {
        let mut attributes: Vec<(String, UnresolvedValue)> = attributes.into_iter().collect();
        crate::sort::by(&mut attributes, |(a, _), (b, _)| a.cmp(b));
        let mut list = Attributes::new();
        for (attribute, value) in attributes {
            let value = value.resolve_attribute()?;
            list.insert(attribute, value);
        }
        builder.add_property("Attributes", list);
    }
    for child in children {
        let child_name = child.name.clone().unwrap_or_else(|| child.class_name.clone());
        builder.add_child(build(child, child_name)?);
    }
    Ok(builder)
}

/// A `.model.json` file's instance tree (its top instance named `name`) as a decoded model whose root
/// holds it; None for a file of white space alone. Read as Rojo reads it (lenient JSON, data_files.rs: a byte order mark is
/// refused, as `rojo build` 7.6.1 refuses it), its errors in Rojo's words.
pub fn decode(bytes: &[u8], name: &str) -> Result<Option<WeakDom>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "File was not valid UTF-8".to_string())?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    let model: JsonModel = super::data_files::read_json_as(text.as_bytes()).map_err(|error| format!("File is not a valid JSON model ({error})"))?;
    let top = build(model, name.to_string()).map_err(|error| format!("Could not load JSON model ({error})"))?;
    written(top).map(Some)
}

/// One instance tree as `rojo build` writes it into a place: through rbx_binary and back, as a decoded model whose root
/// holds it
fn written(top: InstanceBuilder) -> Result<WeakDom, String> {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let top = dom.insert(dom.root_ref(), top);
    let mut file = Vec::new();
    rbx_binary::to_writer(&mut file, &dom, &[top]).map_err(|error| error.to_string())?;
    // (bytes written just above: never an archive, so nothing to note)
    super::decode_model(&file).map(|(dom, _)| dom)
}

/// Rojo's meta file beside a file (`x.meta.json`, its AdjacentMetadata): `className` is no field of it and, like any other
/// unknown field, is ignored. `$schema`, `id` and `ignoreUnknownInstances` change nothing a build makes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AdjacentMetadata {
    #[serde(rename = "$schema")]
    schema: Option<String>,
    id: Option<String>,
    ignore_unknown_instances: Option<bool>,
    #[serde(default)]
    properties: FxHashMap<String, UnresolvedValue>,
    #[serde(default)]
    attributes: FxHashMap<String, UnresolvedValue>,
}

/// A directory's `init.meta.json` (Rojo's DirectoryMetadata): the same and `className`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct DirectoryMetadata {
    #[serde(rename = "$schema")]
    schema: Option<String>,
    id: Option<String>,
    ignore_unknown_instances: Option<bool>,
    #[serde(default)]
    properties: FxHashMap<String, UnresolvedValue>,
    #[serde(default)]
    attributes: FxHashMap<String, UnresolvedValue>,
    class_name: Option<String>,
}

/// A meta file's values, read as Rojo reads it (lenient JSON, data_files.rs)
pub(crate) struct Meta {
    pub class_name: Option<String>,
    properties: Vec<(String, UnresolvedValue)>,
    attributes: Vec<(String, UnresolvedValue)>,
}

/// Why a meta file's values were not applied: Rojo refuses the build in each case
pub(crate) enum MetaError {
    /// A property that does not resolve (Rojo: "error applying meta file <path>", then this)
    Property(String),
    /// An attribute that does not (Rojo says this alone)
    Attribute(String),
    /// The instance does not write (rbx_binary's error)
    Write(String),
}

impl Meta {
    /// A meta file's bytes; `directory` for an init.meta.json. Err: why Rojo refuses it ("JSONC parse error (...)", "File
    /// contains no JSON value", "Invalid JSON structure (...)"), which Rojo prefixes with "File contained malformed
    /// .meta.json data" (or "init.meta.json") and the file.
    pub(crate) fn read(bytes: &[u8], directory: bool) -> Result<Meta, String> {
        let (class_name, properties, attributes) = if directory {
            let meta: DirectoryMetadata = super::data_files::read_json_as(bytes)?;
            (meta.class_name, meta.properties, meta.attributes)
        } else {
            let meta: AdjacentMetadata = super::data_files::read_json_as(bytes)?;
            (None, meta.properties, meta.attributes)
        };
        // (Rojo drains hash maps, in no order: in name order here, so the first error is always the same one)
        let mut properties: Vec<(String, UnresolvedValue)> = properties.into_iter().collect();
        crate::sort::by(&mut properties, |(a, _), (b, _)| a.cmp(b));
        let mut attributes: Vec<(String, UnresolvedValue)> = attributes.into_iter().collect();
        crate::sort::by(&mut attributes, |(a, _), (b, _)| a.cmp(b));
        Ok(Meta { class_name, properties, attributes })
    }

    /// Whether it sets no value
    pub(crate) fn is_empty(&self) -> bool {
        self.properties.is_empty() && self.attributes.is_empty()
    }

    /// Its values on an instance of class `class_name` named `name`, as `rojo build` writes them (resolved by the class's
    /// property types, then through rbx_binary): a decoded model whose root holds that one instance. Its attributes replace
    /// a property `Attributes` it also gives, as Rojo inserts them after the properties.
    pub(crate) fn model(self, class_name: &str, name: &str) -> Result<WeakDom, MetaError> {
        let mut builder = InstanceBuilder::new(class_name).with_name(name);
        for (property, value) in self.properties {
            let value = value.resolve(class_name, &property).map_err(MetaError::Property)?;
            builder.add_property(property.as_str(), value);
        }
        if !self.attributes.is_empty() {
            let mut list = Attributes::new();
            for (attribute, value) in self.attributes {
                list.insert(attribute, value.resolve_attribute().map_err(MetaError::Attribute)?);
            }
            builder.add_property("Attributes", list);
        }
        written(builder).map_err(MetaError::Write)
    }
}
