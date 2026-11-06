//! Model plans: a decoded model converted once (ConvertLune's structure and property values) into an
//! immutable form that engines make instances from, all at once ([`ModelPlan::instantiate`]) or as
//! scripts look at them ([`ModelPlan::instantiate_lazy`]).
//!
//! A plan is a few flat arrays (its nodes, their properties, the distinct values, the text of names
//! and strings, attributes, tags and unmodeled values) and holds no shared pointers: a process that
//! keeps plans between runs (a `ree serve` worker) keeps them in a few large blocks the allocator can
//! hand back whole (the many small blocks a converted model used to be pinned partly free pages long
//! after the decoded file was freed), and a plan converted on a decoding thread moves to the thread
//! that makes it (`Send + Sync`). Instances made of a plan get their own values; strings and names
//! are made once per instantiation.

use std::{rc::Rc, sync::Arc};

use rbx_dom_weak::{
    WeakDom,
    types::{self as dom, Ref, Variant},
};
use rustc_hash::FxHashMap;

mod codec;
mod store;

pub(crate) use store::Mapping;
use store::{Array, Pod};

use super::dom_enum_value;
use crate::{
    ClassId, Engine, InstanceId, K, NameId, Reflection, State,
    dom::{Props, flags},
    instance as core,
    reflection::EnumItemId,
    services,
    value::{self, Str, Value},
};

/// A range of one of a plan's arrays.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Span {
    start: u32,
    len: u32,
}

unsafe impl Pod for Span {}

impl Span {
    fn range(self) -> std::ops::Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }
}

/// How a model property converts: skipped (structure and the members read on their own), kept
/// as read (not modeled for the class) or stored as a property.
#[derive(Clone, Copy)]
enum PropertyPlan {
    Skip,
    Unmodeled,
    Modeled(NameId),
}

/// One instance of a [`ModelPlan`] (preorder).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct PlanNode {
    class: ClassId,
    /// The parent's index ([`NO_PARENT`]: a root, made a child of the instantiation's parent)
    parent: u32,
    /// The index after its subtree (its descendants are the nodes between it and this)
    end: u32,
    /// Its name in the plan's text
    name: Span,
    /// Its properties in the plan's `props`, sorted by name
    props: Span,
    /// Its unmodeled values in the plan's `unmodeled`
    unmodeled: Span,
    /// Its attributes, tags and source ([`NO_EXTRA`]: none)
    extra: u32,
    /// 1 when a property references an instance
    refs: u32,
}

unsafe impl Pod for PlanNode {}

/// One of a node's properties: its name and the index of its value in the plan's `values`, or
/// [`NODE_REF`] and the node it references.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Prop {
    name: NameId,
    value: u32,
}

unsafe impl Pod for Prop {}

const NO_PARENT: u32 = u32::MAX;
const NO_EXTRA: u32 = u32::MAX;
/// A property value that references another node of the plan (its index in the low bits)
const NODE_REF: u32 = 1 << 31;

impl PlanNode {
    fn parent(&self) -> Option<u32> {
        (self.parent != NO_PARENT).then_some(self.parent)
    }
}

/// A property value as a plan keeps it.
#[derive(Clone, Debug)]
enum PlanVal {
    Nil,
    Bool(bool),
    Number(f64),
    EnumItem(EnumItemId),
    UDim(dom::UDim),
    UDim2(dom::UDim2),
    Vector2(dom::Vector2),
    Vector3(dom::Vector3),
    Color3(dom::Color3),
    Rect(dom::Rect),
    NumberRange(dom::NumberRange),
    /// A string: bytes of the plan's text
    Text(Span),
    CFrame(dom::CFrame),
    /// Any other value (sequences, fonts, content, other datatypes): one of the plan's `others`,
    /// converted as it is made
    Other(u32),
}

/// A [`PlanVal`] as a plan's array holds it: its kind (`PlanVal`'s order) and up to 12 words (a float
/// by its bits, an f64 in two), the rest 0.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct RawVal {
    kind: u32,
    words: [u32; 12],
}

unsafe impl Pod for RawVal {}

impl RawVal {
    fn new(value: &PlanVal) -> RawVal {
        let mut words = [0u32; 12];
        let mut put = |values: &[u32]| words[..values.len()].copy_from_slice(values);
        let kind = match value {
            PlanVal::Nil => 0,
            PlanVal::Bool(b) => {
                put(&[*b as u32]);
                1
            }
            PlanVal::Number(n) => {
                let bits = n.to_bits();
                put(&[bits as u32, (bits >> 32) as u32]);
                2
            }
            PlanVal::EnumItem(item) => {
                put(&[*item]);
                3
            }
            PlanVal::UDim(u) => {
                put(&[u.scale.to_bits(), u.offset as u32]);
                4
            }
            PlanVal::UDim2(u) => {
                put(&[u.x.scale.to_bits(), u.x.offset as u32, u.y.scale.to_bits(), u.y.offset as u32]);
                5
            }
            PlanVal::Vector2(v) => {
                put(&[v.x.to_bits(), v.y.to_bits()]);
                6
            }
            PlanVal::Vector3(v) => {
                put(&[v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
                7
            }
            PlanVal::Color3(c) => {
                put(&[c.r.to_bits(), c.g.to_bits(), c.b.to_bits()]);
                8
            }
            PlanVal::Rect(r) => {
                put(&[r.min.x.to_bits(), r.min.y.to_bits(), r.max.x.to_bits(), r.max.y.to_bits()]);
                9
            }
            PlanVal::NumberRange(r) => {
                put(&[r.min.to_bits(), r.max.to_bits()]);
                10
            }
            PlanVal::Text(span) => {
                put(&[span.start, span.len]);
                11
            }
            PlanVal::CFrame(c) => {
                let (p, o) = (c.position, c.orientation);
                put(&[p.x, p.y, p.z, o.x.x, o.x.y, o.x.z, o.y.x, o.y.y, o.y.z, o.z.x, o.z.y, o.z.z].map(f32::to_bits));
                12
            }
            PlanVal::Other(index) => {
                put(&[*index]);
                13
            }
        };
        RawVal { kind, words }
    }

    /// The value; None for a kind there is none of (a file that does not hold a plan).
    fn get(&self) -> Option<PlanVal> {
        let w = &self.words;
        let f = |index: usize| f32::from_bits(w[index]);
        let vector3 = |at: usize| dom::Vector3::new(f(at), f(at + 1), f(at + 2));
        Some(match self.kind {
            0 => PlanVal::Nil,
            1 => PlanVal::Bool(w[0] != 0),
            2 => PlanVal::Number(f64::from_bits(w[0] as u64 | (w[1] as u64) << 32)),
            3 => PlanVal::EnumItem(w[0]),
            4 => PlanVal::UDim(dom::UDim::new(f(0), w[1] as i32)),
            5 => PlanVal::UDim2(dom::UDim2::new(dom::UDim::new(f(0), w[1] as i32), dom::UDim::new(f(2), w[3] as i32))),
            6 => PlanVal::Vector2(dom::Vector2::new(f(0), f(1))),
            7 => PlanVal::Vector3(vector3(0)),
            8 => PlanVal::Color3(dom::Color3::new(f(0), f(1), f(2))),
            9 => PlanVal::Rect(dom::Rect::new(dom::Vector2::new(f(0), f(1)), dom::Vector2::new(f(2), f(3)))),
            10 => PlanVal::NumberRange(dom::NumberRange::new(f(0), f(1))),
            11 => PlanVal::Text(Span { start: w[0], len: w[1] }),
            12 => PlanVal::CFrame(dom::CFrame::new(vector3(0), dom::Matrix3::new(vector3(3), vector3(6), vector3(9)))),
            13 => PlanVal::Other(w[0]),
            _ => return None,
        })
    }
}

/// An attribute value: a value, or a reference to another node of the plan.
#[derive(Clone, Debug)]
enum PlanAttribute {
    Value(PlanVal),
    Instance(u32),
}

/// An attribute as a plan's array holds it: its name, and kind 0 with a value or 1 with a node.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct RawAttribute {
    name: Span,
    kind: u32,
    target: u32,
    value: RawVal,
}

unsafe impl Pod for RawAttribute {}

impl RawAttribute {
    fn new(name: Span, attribute: &PlanAttribute) -> RawAttribute {
        match attribute {
            PlanAttribute::Value(value) => RawAttribute { name, kind: 0, target: 0, value: RawVal::new(value) },
            PlanAttribute::Instance(target) => RawAttribute { name, kind: 1, target: *target, value: RawVal::new(&PlanVal::Nil) },
        }
    }

    fn get(&self) -> Option<PlanAttribute> {
        match self.kind {
            0 => self.value.get().map(PlanAttribute::Value),
            1 => Some(PlanAttribute::Instance(self.target)),
            _ => None,
        }
    }
}

/// The attributes, tags and source of a node that has any.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct PlanExtra {
    attributes: Span,
    tags: Span,
    /// 1 when the node has source (`code`)
    has_code: u32,
    code: Span,
}

unsafe impl Pod for PlanExtra {}

impl PlanExtra {
    fn new(attributes: Span, tags: Span, code: Option<Span>) -> PlanExtra {
        PlanExtra { attributes, tags, has_code: code.is_some() as u32, code: code.unwrap_or_default() }
    }

    fn code(&self) -> Option<Span> {
        (self.has_code != 0).then_some(self.code)
    }
}

/// An unmodeled value: an instance's own id (every instance has two) or a distinct value.
#[derive(Clone, Debug)]
enum UnmodeledValue {
    Id(dom::UniqueId),
    Shared(u32),
}

/// An unmodeled value as a plan's array holds it: its name, and kind 0 with an id (index, time and
/// the random part's two halves) or 1 with a distinct value's index.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct UnmodeledEntry {
    /// In the table's names
    name: Span,
    kind: u32,
    words: [u32; 4],
}

unsafe impl Pod for UnmodeledEntry {}

impl UnmodeledEntry {
    fn new(name: Span, value: &UnmodeledValue) -> UnmodeledEntry {
        match value {
            UnmodeledValue::Id(id) => {
                let random = id.random() as u64;
                UnmodeledEntry { name, kind: 0, words: [id.index(), id.time(), random as u32, (random >> 32) as u32] }
            }
            UnmodeledValue::Shared(index) => UnmodeledEntry { name, kind: 1, words: [*index, 0, 0, 0] },
        }
    }

    fn value(&self) -> Option<UnmodeledValue> {
        let w = &self.words;
        match self.kind {
            0 => Some(UnmodeledValue::Id(dom::UniqueId::new(w[0], w[1], (w[2] as u64 | (w[3] as u64) << 32) as i64))),
            1 => Some(UnmodeledValue::Shared(w[0])),
            _ => None,
        }
    }
}

/// A plan's unmodeled values (instances made of the plan share it).
#[derive(Debug, Default)]
struct UnmodeledTable {
    entries: Array<UnmodeledEntry>,
    values: Box<[FlatVariant]>,
    /// Names and string values
    text: Array<u8>,
    floats: Array<f32>,
}

/// An rbx-dom value kept without data of its own on the heap: strings and sequences are ranges of
/// a plan's text and floats (their own small blocks, live among a decoded file's freed ones, kept
/// whole pages of the decoding thread's heap from being returned).
#[derive(Clone, Debug)]
enum FlatVariant {
    /// A value with no heap data (numbers, vectors, CFrames, enums, BrickColor, ...)
    Plain(Variant),
    String(Span),
    BinaryString(Span),
    ContentId(Span),
    SharedString(Span),
    /// A Content: its URI, or none
    Content(Option<Span>),
    /// Keypoints: time, value and envelope each
    NumberSequence(Span),
    /// Keypoints: time and r, g, b each
    ColorSequence(Span),
    /// Anything else, as read (fonts, attributes, ...: rare)
    Other(Box<Variant>),
}

impl FlatVariant {
    fn new(variant: &Variant, text: &mut Text, floats: &mut Vec<f32>) -> FlatVariant {
        let mut floats_of = |values: &mut dyn Iterator<Item = f32>| {
            let start = floats.len() as u32;
            floats.extend(values);
            Span { start, len: floats.len() as u32 - start }
        };
        match variant {
            Variant::String(s) => FlatVariant::String(text.add(s.as_bytes())),
            Variant::BinaryString(s) => FlatVariant::BinaryString(text.add(s.as_ref())),
            Variant::ContentId(s) => FlatVariant::ContentId(text.add(s.as_str().as_bytes())),
            Variant::SharedString(s) => FlatVariant::SharedString(text.add(s.data())),
            Variant::Content(content) => match content.value() {
                dom::ContentType::None => FlatVariant::Content(None),
                dom::ContentType::Uri(uri) => FlatVariant::Content(Some(text.add(uri.as_bytes()))),
                _ => FlatVariant::Other(Box::new(variant.clone())),
            },
            Variant::NumberSequence(sequence) => FlatVariant::NumberSequence(floats_of(&mut sequence.keypoints.iter().flat_map(|point| [point.time, point.value, point.envelope]))),
            Variant::ColorSequence(sequence) => FlatVariant::ColorSequence(floats_of(&mut sequence.keypoints.iter().flat_map(|point| [point.time, point.color.r, point.color.g, point.color.b]))),
            Variant::Axes(_)
            | Variant::Bool(_)
            | Variant::BrickColor(_)
            | Variant::CFrame(_)
            | Variant::Color3(_)
            | Variant::Color3uint8(_)
            | Variant::Enum(_)
            | Variant::Faces(_)
            | Variant::Float32(_)
            | Variant::Float64(_)
            | Variant::Int32(_)
            | Variant::Int64(_)
            | Variant::NumberRange(_)
            | Variant::OptionalCFrame(_)
            | Variant::PhysicalProperties(_)
            | Variant::Ray(_)
            | Variant::Rect(_)
            | Variant::Region3(_)
            | Variant::Region3int16(_)
            | Variant::UDim(_)
            | Variant::UDim2(_)
            | Variant::UniqueId(_)
            | Variant::Vector2(_)
            | Variant::Vector2int16(_)
            | Variant::Vector3(_)
            | Variant::Vector3int16(_) => FlatVariant::Plain(variant.clone()),
            _ => FlatVariant::Other(Box::new(variant.clone())),
        }
    }

    /// The value as rbx-dom's (`text` and `floats` the arrays its ranges are in).
    fn variant(&self, text: &[u8], floats: &[f32]) -> Variant {
        let string = |span: &Span| String::from_utf8_lossy(&text[span.range()]).into_owned();
        match self {
            FlatVariant::Plain(variant) => variant.clone(),
            FlatVariant::String(span) => Variant::String(string(span)),
            FlatVariant::BinaryString(span) => Variant::BinaryString(text[span.range()].to_vec().into()),
            FlatVariant::ContentId(span) => Variant::ContentId(string(span).into()),
            FlatVariant::SharedString(span) => Variant::SharedString(dom::SharedString::new(text[span.range()].to_vec())),
            FlatVariant::Content(None) => Variant::Content(dom::Content::none()),
            FlatVariant::Content(Some(span)) => Variant::Content(dom::Content::from_uri(string(span))),
            FlatVariant::NumberSequence(span) => Variant::NumberSequence(dom::NumberSequence {
                keypoints: floats[span.range()].chunks_exact(3).map(|point| dom::NumberSequenceKeypoint::new(point[0], point[1], point[2])).collect(),
            }),
            FlatVariant::ColorSequence(span) => Variant::ColorSequence(dom::ColorSequence {
                keypoints: floats[span.range()].chunks_exact(4).map(|point| dom::ColorSequenceKeypoint::new(point[0], dom::Color3::new(point[1], point[2], point[3]))).collect(),
            }),
            FlatVariant::Other(variant) => (**variant).clone(),
        }
    }
}

/// A decoded model converted once (ConvertLune's structure and property values), to be made as
/// engine instances any number of times on any thread ([`ModelPlan::instantiate`]): a process that
/// runs many engines converts a model file once.
#[derive(Debug)]
pub struct ModelPlan {
    nodes: Array<PlanNode>,
    /// (property, value) of every node: an index into `values`, or [`NODE_REF`] and a node
    props: Array<Prop>,
    /// The distinct property values
    values: Array<RawVal>,
    /// The distinct values of other datatypes ([`PlanVal::Other`])
    others: Box<[FlatVariant]>,
    /// Sequences' keypoints
    floats: Array<f32>,
    /// Names, strings, sources
    text: Array<u8>,
    attributes: Array<RawAttribute>,
    tags: Array<Span>,
    extras: Array<PlanExtra>,
    /// What the model file held that the engine does not model, kept to write instances back out
    unmodeled: Arc<UnmodeledTable>,
    /// Unknown classes imported as Folder, reported by each instantiation
    warnings: Vec<String>,
    /// The nodes with tags (a lazy import makes them at once: CollectionService finds them)
    tagged: Array<u32>,
    /// Whether a node references one under another root (a lazy import then makes everything at
    /// once: a reference that keeps its target alive must exist before scripts can drop the target)
    cross_root: bool,
    /// `cross_root` over the children of the first root, each with its subtree, or the root itself:
    /// what an import that makes those children at once (a folder bundle's, a loose bundle's
    /// members) goes by
    cross_member: bool,
}

/// What one conversion learns once per class or property (the same answers as asking for each
/// instance, without its string lookups): plans by (class, property), enum items by (rbx-dom
/// class, property, value), each class's string asset ids with their Content siblings (both
/// names), and whether it holds source.
#[derive(Default)]
struct ConvertMemo {
    classes: FxHashMap<rbx_dom_weak::Ustr, Option<ClassId>>,
    plans: FxHashMap<(ClassId, rbx_dom_weak::Ustr), PropertyPlan>,
    enums: FxHashMap<(rbx_dom_weak::Ustr, rbx_dom_weak::Ustr, u32), Option<EnumItemId>>,
    content: FxHashMap<ClassId, Rc<[(NameId, rbx_dom_weak::Ustr, rbx_dom_weak::Ustr)]>>,
    scripts: FxHashMap<ClassId, bool>,
}

impl ConvertMemo {
    fn plan(&mut self, reflection: &Reflection, class: ClassId, name: rbx_dom_weak::Ustr) -> PropertyPlan {
        *self.plans.entry((class, name)).or_insert_with(|| {
            let text = name.as_str();
            if matches!(text, "Parent" | "ClassName" | "Name" | "Attributes" | "Tags" | "Source") {
                return PropertyPlan::Skip;
            }
            match reflection.name_id(text).filter(|&name_id| reflection.class_property(class, name_id).is_some()) {
                Some(name_id) => PropertyPlan::Modeled(name_id),
                None => PropertyPlan::Unmodeled,
            }
        })
    }

    fn enum_value(&mut self, reflection: &Reflection, class_name: rbx_dom_weak::Ustr, property: rbx_dom_weak::Ustr, item: u32) -> Option<EnumItemId> {
        *self.enums.entry((class_name, property, item)).or_insert_with(|| match dom_enum_value(reflection, class_name.as_str(), property.as_str(), item) {
            Some(Value::EnumItem(item)) => Some(item),
            _ => None,
        })
    }

    fn content(&mut self, reflection: &Reflection, class: ClassId) -> Rc<[(NameId, rbx_dom_weak::Ustr, rbx_dom_weak::Ustr)]> {
        self.content
            .entry(class)
            .or_insert_with(|| {
                reflection
                    .content_siblings(class)
                    .into_iter()
                    .map(|(string, content)| (string, rbx_dom_weak::ustr(reflection.name(string)), rbx_dom_weak::ustr(reflection.name(content))))
                    .collect()
            })
            .clone()
    }

    fn script(&mut self, reflection: &Reflection, class: ClassId) -> bool {
        *self.scripts.entry(class).or_insert_with(|| reflection.is_a(class, "LuaSourceContainer"))
    }
}

/// Distinct entries by their bits (-0 and 0, or two NaNs, stay apart): most properties of a model's
/// instances repeat a few values, names and strings.
struct Distinct<T> {
    items: Vec<T>,
    index: FxHashMap<Box<[u8]>, u32>,
}

impl<T> Default for Distinct<T> {
    fn default() -> Self {
        Self { items: Vec::new(), index: FxHashMap::default() }
    }
}

impl<T> Distinct<T> {
    /// `item`'s index: an equal one's when `key` (its bits; None: not shared) was added before.
    fn add(&mut self, key: Option<&[u8]>, item: impl FnOnce() -> T) -> u32 {
        if let Some(key) = key {
            if let Some(&found) = self.index.get(key) {
                return found;
            }
            self.index.insert(key.into(), self.items.len() as u32);
        }
        self.items.push(item());
        self.items.len() as u32 - 1
    }
}

/// The text of a plan being converted: distinct byte strings, one after another.
#[derive(Default)]
struct Text {
    bytes: Vec<u8>,
    index: FxHashMap<Box<[u8]>, Span>,
}

impl Text {
    fn add(&mut self, bytes: &[u8]) -> Span {
        if let Some(&span) = self.index.get(bytes) {
            return span;
        }
        let span = Span { start: self.bytes.len() as u32, len: bytes.len() as u32 };
        self.bytes.extend_from_slice(bytes);
        self.index.insert(bytes.into(), span);
        span
    }
}

fn put_f32s(key: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        key.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

/// A plan value's bits in `key` (cleared first); false for a value that is not shared.
fn value_key(value: &PlanVal, key: &mut Vec<u8>) -> bool {
    key.clear();
    match value {
        PlanVal::Nil => key.push(0),
        PlanVal::Bool(b) => key.extend_from_slice(&[1, *b as u8]),
        PlanVal::Number(n) => {
            key.push(2);
            key.extend_from_slice(&n.to_bits().to_le_bytes());
        }
        PlanVal::EnumItem(item) => {
            key.push(3);
            key.extend_from_slice(&item.to_le_bytes());
        }
        PlanVal::UDim(u) => {
            key.push(4);
            put_f32s(key, &[u.scale]);
            key.extend_from_slice(&u.offset.to_le_bytes());
        }
        PlanVal::UDim2(u) => {
            key.push(5);
            put_f32s(key, &[u.x.scale, u.y.scale]);
            key.extend_from_slice(&u.x.offset.to_le_bytes());
            key.extend_from_slice(&u.y.offset.to_le_bytes());
        }
        PlanVal::Vector2(v) => {
            key.push(6);
            put_f32s(key, &[v.x, v.y]);
        }
        PlanVal::Vector3(v) => {
            key.push(7);
            put_f32s(key, &[v.x, v.y, v.z]);
        }
        PlanVal::Color3(c) => {
            key.push(8);
            put_f32s(key, &[c.r, c.g, c.b]);
        }
        PlanVal::Rect(r) => {
            key.push(9);
            put_f32s(key, &[r.min.x, r.min.y, r.max.x, r.max.y]);
        }
        PlanVal::NumberRange(r) => {
            key.push(10);
            put_f32s(key, &[r.min, r.max]);
        }
        PlanVal::Text(span) => {
            key.push(11);
            key.extend_from_slice(&span.start.to_le_bytes());
            key.extend_from_slice(&span.len.to_le_bytes());
        }
        PlanVal::CFrame(c) => {
            key.push(12);
            let (p, o) = (c.position, c.orientation);
            put_f32s(key, &[p.x, p.y, p.z, o.x.x, o.x.y, o.x.z, o.y.x, o.y.y, o.y.z, o.z.x, o.z.y, o.z.z]);
        }
        PlanVal::Other(index) => {
            key.push(13);
            key.extend_from_slice(&index.to_le_bytes());
        }
    }
    true
}

/// An unmodeled value's bits in `key` (cleared first); false for one that is not shared.
fn variant_key(variant: &Variant, key: &mut Vec<u8>) -> bool {
    key.clear();
    match variant {
        Variant::String(s) => {
            key.push(0);
            key.extend_from_slice(s.as_bytes());
        }
        Variant::BinaryString(s) => {
            key.push(1);
            key.extend_from_slice(s.as_ref());
        }
        Variant::ContentId(s) => {
            key.push(2);
            key.extend_from_slice(s.as_str().as_bytes());
        }
        Variant::SharedString(s) => {
            key.push(3);
            key.extend_from_slice(s.hash().as_bytes());
        }
        Variant::Bool(b) => key.extend_from_slice(&[4, *b as u8]),
        Variant::Int32(n) => {
            key.push(5);
            key.extend_from_slice(&n.to_le_bytes());
        }
        Variant::Int64(n) => {
            key.push(6);
            key.extend_from_slice(&n.to_le_bytes());
        }
        Variant::Float32(n) => {
            key.push(7);
            put_f32s(key, &[*n]);
        }
        Variant::Float64(n) => {
            key.push(8);
            key.extend_from_slice(&n.to_bits().to_le_bytes());
        }
        Variant::Enum(item) => {
            key.push(9);
            key.extend_from_slice(&item.to_u32().to_le_bytes());
        }
        Variant::Vector3(v) => {
            key.push(10);
            put_f32s(key, &[v.x, v.y, v.z]);
        }
        Variant::Content(content) => {
            key.push(11);
            match content.value() {
                dom::ContentType::None => key.push(0),
                dom::ContentType::Uri(uri) => {
                    key.push(1);
                    key.extend_from_slice(uri.as_bytes());
                }
                _ => return false,
            }
        }
        Variant::NumberSequence(sequence) => {
            key.push(12);
            for point in &sequence.keypoints {
                put_f32s(key, &[point.time, point.value, point.envelope]);
            }
        }
        Variant::ColorSequence(sequence) => {
            key.push(13);
            for point in &sequence.keypoints {
                put_f32s(key, &[point.time, point.color.r, point.color.g, point.color.b]);
            }
        }
        Variant::BrickColor(color) => {
            key.push(14);
            key.extend_from_slice(&(*color as u16).to_le_bytes());
        }
        _ => return false,
    }
    true
}

/// A converted value of a model property: a plan value, a reference to another node, or nothing.
enum Converted {
    Value(PlanVal),
    Instance(u32),
}

/// `variant` as a plan value (what [`value::from_dom`] makes of it, kept without shared pointers).
fn plan_val(reflection: &Reflection, text: &mut Text, floats: &mut Vec<f32>, others: &mut Distinct<FlatVariant>, key: &mut Vec<u8>, variant: &Variant) -> Option<PlanVal> {
    Some(match variant {
        Variant::String(s) => PlanVal::Text(text.add(s.as_bytes())),
        Variant::BinaryString(s) => PlanVal::Text(text.add(s.as_ref())),
        Variant::ContentId(s) => PlanVal::Text(text.add(s.as_str().as_bytes())),
        Variant::CFrame(c) | Variant::OptionalCFrame(Some(c)) => PlanVal::CFrame(*c),
        other => match value::from_dom(reflection, other)? {
            Value::Nil => PlanVal::Nil,
            Value::Bool(b) => PlanVal::Bool(b),
            Value::Number(n) => PlanVal::Number(n),
            Value::EnumItem(item) => PlanVal::EnumItem(item),
            Value::UDim(u) => PlanVal::UDim(u),
            Value::UDim2(u) => PlanVal::UDim2(u),
            Value::Vector2(v) => PlanVal::Vector2(v),
            Value::Vector3(v) => PlanVal::Vector3(v),
            Value::Color3(c) => PlanVal::Color3(c),
            Value::Rect(r) => PlanVal::Rect(r),
            Value::NumberRange(r) => PlanVal::NumberRange(r),
            _ => {
                let shareable = variant_key(other, key);
                PlanVal::Other(others.add(shareable.then_some(key.as_slice()), || FlatVariant::new(other, text, floats)))
            }
        },
    })
}

impl ModelPlan {
    /// ConvertLune's conversion of `roots` of a decoded model (and their descendants).
    pub fn new(reflection: &Reflection, model: &WeakDom, roots: &[Ref]) -> ModelPlan {
        let folder = reflection.class_id("Folder").expect("Folder");
        let mut memo = ConvertMemo::default();
        let mut nodes: Vec<PlanNode> = Vec::new();
        let mut order: Vec<Ref> = Vec::new();
        let mut indices: FxHashMap<Ref, u32> = FxHashMap::default();
        let mut warnings = Vec::new();
        let mut text = Text::default();
        let mut floats: Vec<f32> = Vec::new();
        let mut unmodeled_text = Text::default();
        let mut unmodeled_floats: Vec<f32> = Vec::new();
        // Structure first (preorder, as ConvertLune recurses), then properties (refs resolved)
        let mut stack: Vec<(Ref, u32)> = roots.iter().rev().map(|&root| (root, NO_PARENT)).collect();
        while let Some((referent, parent)) = stack.pop() {
            let source = model.get_by_ref(referent).expect("model instance");
            let known = *memo.classes.entry(source.class).or_insert_with(|| reflection.class_id(source.class.as_str()));
            let class = known.unwrap_or_else(|| {
                let full = full_dom_name(model, referent);
                warnings.push(format!("snapshot: unknown class {} ({full}); imported as Folder", source.class));
                folder
            });
            let index = nodes.len() as u32;
            let name = text.add(source.name.as_bytes());
            nodes.push(PlanNode { class, parent, end: index + 1, name, props: Span::default(), unmodeled: Span::default(), extra: NO_EXTRA, refs: 0 });
            indices.insert(referent, index);
            order.push(referent);
            stack.extend(source.children().iter().rev().map(|&child| (child, index)));
        }
        let mut values: Distinct<PlanVal> = Distinct::default();
        let mut shared: Distinct<FlatVariant> = Distinct::default();
        let mut others: Distinct<FlatVariant> = Distinct::default();
        let mut props: Vec<Prop> = Vec::new();
        let mut attributes: Vec<(Span, PlanAttribute)> = Vec::new();
        let mut tags: Vec<Span> = Vec::new();
        let mut extras: Vec<PlanExtra> = Vec::new();
        let mut unmodeled: Vec<UnmodeledEntry> = Vec::new();
        let mut key = Vec::new();
        let mut own: Vec<(NameId, Converted)> = Vec::new();
        let (attributes_key, tags_key, source_key) = (rbx_dom_weak::ustr("Attributes"), rbx_dom_weak::ustr("Tags"), rbx_dom_weak::ustr("Source"));
        for (index, referent) in order.into_iter().enumerate() {
            let source = model.get_by_ref(referent).expect("model instance");
            let class = nodes[index].class;
            own.clear();
            let unmodeled_start = unmodeled.len() as u32;
            for (&name, variant) in &source.properties {
                let name_id = match memo.plan(reflection, class, name) {
                    PropertyPlan::Skip => continue,
                    PropertyPlan::Unmodeled => {
                        // kept for writing the instance back out (references are the file's own)
                        let value = match variant {
                            Variant::Ref(_) => continue,
                            Variant::UniqueId(id) => UnmodeledValue::Id(*id),
                            other => {
                                let shareable = variant_key(other, &mut key);
                                UnmodeledValue::Shared(shared.add(shareable.then_some(key.as_slice()), || FlatVariant::new(other, &mut unmodeled_text, &mut unmodeled_floats)))
                            }
                        };
                        unmodeled.push(UnmodeledEntry::new(unmodeled_text.add(name.as_str().as_bytes()), &value));
                        continue;
                    }
                    PropertyPlan::Modeled(name_id) => name_id,
                };
                let converted = match variant {
                    Variant::Ref(target) => indices.get(target).map(|&target| Converted::Instance(target)),
                    Variant::Enum(item) => memo.enum_value(reflection, source.class, name, item.to_u32()).map(|item| Converted::Value(PlanVal::EnumItem(item))),
                    other => plan_val(reflection, &mut text, &mut floats, &mut others, &mut key, other).map(Converted::Value),
                };
                if let Some(converted) = converted {
                    own.push((name_id, converted));
                }
            }
            // string asset ids read from their Content sibling (Reflection::content_sibling) when the
            // file stores none or an empty one: newer files keep only AnimationContent, MeshContent, ...
            for &(name_id, string, sibling) in memo.content(reflection, class).iter() {
                let stored = match source.properties.get(&string) {
                    Some(Variant::ContentId(id)) => !id.as_str().is_empty(),
                    Some(Variant::String(id)) => !id.is_empty(),
                    Some(_) => true,
                    None => false,
                };
                if stored {
                    continue;
                }
                if let Some(Variant::Content(content)) = source.properties.get(&sibling) {
                    if let Some(uri) = content.as_uri() {
                        let value = Converted::Value(PlanVal::Text(text.add(uri.as_bytes())));
                        match own.iter_mut().find(|(n, _)| *n == name_id) {
                            Some(entry) => entry.1 = value,
                            None => own.push((name_id, value)),
                        }
                    }
                }
            }
            // (sorted by name, as storing them one by one leaves them)
            own.sort_by_key(|(name, _)| *name);
            let props_start = props.len() as u32;
            let mut refs = false;
            for (name, converted) in own.drain(..) {
                let slot = match converted {
                    Converted::Instance(target) => {
                        refs = true;
                        NODE_REF | target
                    }
                    Converted::Value(value) => {
                        let shareable = value_key(&value, &mut key);
                        values.add(shareable.then_some(key.as_slice()), || value)
                    }
                };
                props.push(Prop { name, value: slot });
            }
            let attributes_start = attributes.len() as u32;
            if let Some(Variant::Attributes(list)) = source.properties.get(&attributes_key) {
                for (name, variant) in list.iter() {
                    let converted = match variant {
                        Variant::Ref(target) => indices.get(target).map(|&target| PlanAttribute::Instance(target)),
                        Variant::Enum(item) => match dom_enum_value(reflection, source.class.as_str(), name, item.to_u32()) {
                            Some(Value::EnumItem(item)) => Some(PlanAttribute::Value(PlanVal::EnumItem(item))),
                            _ => None,
                        },
                        other => plan_val(reflection, &mut text, &mut floats, &mut others, &mut key, other).map(PlanAttribute::Value),
                    };
                    if let Some(value) = converted {
                        attributes.push((text.add(name.as_bytes()), value));
                    }
                }
            }
            let tags_start = tags.len() as u32;
            match source.properties.get(&tags_key) {
                Some(Variant::Tags(list)) => tags.extend(list.iter().map(|tag| text.add(tag.as_bytes()))),
                // (a newer Studio writes Tags as a SharedString, which project::decode_model then reads
                // as one: the same NUL-separated names; uploaded assets can hold them so)
                Some(Variant::SharedString(names)) => tags.extend(names.data().split(|&byte| byte == 0).filter(|name| !name.is_empty()).map(|name| text.add(name))),
                _ => {}
            }
            let code = match source.properties.get(&source_key) {
                Some(Variant::String(code)) if !code.is_empty() && memo.script(reflection, class) => Some(text.add(code.as_bytes())),
                _ => None,
            };
            let node = &mut nodes[index];
            node.props = Span { start: props_start, len: props.len() as u32 - props_start };
            node.unmodeled = Span { start: unmodeled_start, len: unmodeled.len() as u32 - unmodeled_start };
            node.refs = refs as u32;
            let extra = PlanExtra::new(
                Span { start: attributes_start, len: attributes.len() as u32 - attributes_start },
                Span { start: tags_start, len: tags.len() as u32 - tags_start },
                code,
            );
            if extra.attributes.len > 0 || extra.tags.len > 0 || extra.code().is_some() {
                node.extra = extras.len() as u32;
                extras.push(extra);
            }
        }
        // (preorder: a subtree ends where its last descendant's does)
        for index in (0..nodes.len()).rev() {
            if let Some(parent) = nodes[index].parent() {
                let end = nodes[index].end;
                let parent = &mut nodes[parent as usize];
                parent.end = parent.end.max(end);
            }
        }
        let tagged: Vec<u32> = nodes.iter().enumerate().filter(|(_, node)| node.extra != NO_EXTRA && extras[node.extra as usize].tags.len > 0).map(|(index, _)| index as u32).collect();
        let mut root_of: Vec<u32> = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            let root = node.parent().map_or(index as u32, |parent| root_of[parent as usize]);
            root_of.push(root);
        }
        let references = |index: usize| {
            let node = &nodes[index];
            let props = props[node.props.range()].iter().filter(|prop| prop.value & NODE_REF != 0).map(|prop| prop.value & !NODE_REF);
            let own = if node.extra != NO_EXTRA { &attributes[extras[node.extra as usize].attributes.range()] } else { &[][..] };
            props.chain(own.iter().filter_map(|(_, value)| match value {
                PlanAttribute::Instance(target) => Some(*target),
                PlanAttribute::Value(_) => None,
            }))
        };
        let cross_root = (0..nodes.len()).any(|index| references(index).any(|target| root_of[target as usize] != root_of[index]));
        // (the same over the children of the first root, each with its subtree)
        let mut member_of: Vec<u32> = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            let member = match node.parent() {
                None | Some(0) => index as u32,
                Some(parent) => member_of[parent as usize],
            };
            member_of.push(member);
        }
        let first_end = nodes.first().map_or(0, |node| node.end as usize);
        let cross_member = (1..first_end).any(|index| references(index).any(|target| member_of[target as usize] != member_of[index]));
        ModelPlan {
            nodes: nodes.into(),
            props: props.into(),
            values: values.items.iter().map(RawVal::new).collect::<Vec<_>>().into(),
            others: others.items.into(),
            floats: floats.into(),
            text: text.bytes.into(),
            attributes: attributes.iter().map(|(name, attribute)| RawAttribute::new(*name, attribute)).collect::<Vec<_>>().into(),
            tags: tags.into(),
            extras: extras.into(),
            unmodeled: Arc::new(UnmodeledTable { entries: unmodeled.into(), values: shared.items.into(), text: unmodeled_text.bytes.into(), floats: unmodeled_floats.into() }),
            warnings,
            tagged: tagged.into(),
            cross_root,
            cross_member,
        }
    }

    /// The plan copied into blocks of their exact sizes, made now: a plan converted while its
    /// decoded file was alive sits among that file's freed blocks (and in blocks grown by doubling),
    /// which keeps whole pages of the converting thread's heap committed after the file is freed.
    /// Call it after freeing the decoded file.
    pub fn compacted(self) -> ModelPlan {
        fn copy<T: Clone>(items: &[T]) -> Box<[T]> {
            let mut fresh = Vec::with_capacity(items.len());
            fresh.extend_from_slice(items);
            fresh.into_boxed_slice()
        }
        let unmodeled = &*self.unmodeled;
        ModelPlan {
            nodes: self.nodes.compacted(),
            props: self.props.compacted(),
            values: self.values.compacted(),
            others: copy(&self.others),
            floats: self.floats.compacted(),
            text: self.text.compacted(),
            attributes: self.attributes.compacted(),
            tags: self.tags.compacted(),
            extras: self.extras.compacted(),
            unmodeled: Arc::new(UnmodeledTable { entries: unmodeled.entries.compacted(), values: copy(&unmodeled.values), text: unmodeled.text.compacted(), floats: unmodeled.floats.compacted() }),
            warnings: self.warnings.clone(),
            tagged: self.tagged.compacted(),
            cross_root: self.cross_root,
            cross_member: self.cross_member,
        }
    }

    /// [`ModelPlan::compacted`] of a plan that others hold too.
    pub fn clone_compacted(plan: &ModelPlan) -> ModelPlan {
        ModelPlan {
            nodes: plan.nodes.clone(),
            props: plan.props.clone(),
            values: plan.values.clone(),
            others: plan.others.clone(),
            floats: plan.floats.clone(),
            text: plan.text.clone(),
            attributes: plan.attributes.clone(),
            tags: plan.tags.clone(),
            extras: plan.extras.clone(),
            unmodeled: Arc::new(UnmodeledTable { entries: plan.unmodeled.entries.clone(), values: plan.unmodeled.values.clone(), text: plan.unmodeled.text.clone(), floats: plan.unmodeled.floats.clone() }),
            warnings: plan.warnings.clone(),
            tagged: plan.tagged.clone(),
            cross_root: plan.cross_root,
            cross_member: plan.cross_member,
        }
    }

    /// Whether every range, index and reference of the plan is within its arrays (a plan read from
    /// a file is checked before use).
    fn consistent(&self) -> bool {
        let within = |span: Span, len: usize| span.start as usize + span.len as usize <= len;
        let count = self.nodes.len() as u32;
        let flat = |value: &FlatVariant, text: usize, floats: usize| match value {
            FlatVariant::String(span) | FlatVariant::BinaryString(span) | FlatVariant::ContentId(span) | FlatVariant::SharedString(span) | FlatVariant::Content(Some(span)) => within(*span, text),
            FlatVariant::NumberSequence(span) => within(*span, floats) && span.len % 3 == 0,
            FlatVariant::ColorSequence(span) => within(*span, floats) && span.len % 4 == 0,
            FlatVariant::Plain(_) | FlatVariant::Content(None) | FlatVariant::Other(_) => true,
        };
        let value = |raw: &RawVal| match raw.get() {
            Some(PlanVal::Text(span)) => within(span, self.text.len()),
            Some(PlanVal::Other(index)) => (index as usize) < self.others.len(),
            Some(_) => true,
            None => false,
        };
        let unmodeled = &*self.unmodeled;
        self.nodes.iter().enumerate().all(|(index, node)| {
            let index = index as u32;
            let parent_ok = match node.parent() {
                Some(parent) => parent < index && node.end <= self.nodes[parent as usize].end,
                None => true,
            };
            parent_ok
                && node.end > index
                && node.end <= count
                && within(node.name, self.text.len())
                && within(node.props, self.props.len())
                && within(node.unmodeled, unmodeled.entries.len())
                && (node.extra == NO_EXTRA || (node.extra as usize) < self.extras.len())
                && node.refs <= 1
        }) && self.props.iter().all(|prop| if prop.value & NODE_REF != 0 { prop.value & !NODE_REF < count } else { (prop.value as usize) < self.values.len() })
            && self.values.iter().all(value)
            && self.others.iter().all(|other| flat(other, self.text.len(), self.floats.len()))
            && self.attributes.iter().all(|attribute| {
                within(attribute.name, self.text.len())
                    && match attribute.kind {
                        0 => value(&attribute.value),
                        1 => attribute.target < count,
                        _ => false,
                    }
            })
            && self.tags.iter().all(|&tag| within(tag, self.text.len()))
            && self.extras.iter().all(|extra| within(extra.attributes, self.attributes.len()) && within(extra.tags, self.tags.len()) && extra.code().is_none_or(|code| within(code, self.text.len())))
            && unmodeled.entries.iter().all(|entry| {
                within(entry.name, unmodeled.text.len())
                    && match entry.value() {
                        Some(UnmodeledValue::Shared(index)) => (index as usize) < unmodeled.values.len(),
                        Some(UnmodeledValue::Id(_)) => true,
                        None => false,
                    }
            })
            && unmodeled.values.iter().all(|v| flat(v, unmodeled.text.len(), unmodeled.floats.len()))
            && self.tagged.iter().all(|&node| node < count)
    }

    /// The names of the plan's roots.
    pub fn root_names(&self) -> impl Iterator<Item = &[u8]> {
        self.roots().map(|index| self.bytes(self.nodes[index as usize].name))
    }

    /// The instances the plan makes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Unknown classes imported as Folder (each instantiation reports them).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn bytes(&self, span: Span) -> &[u8] {
        &self.text[span.range()]
    }

    fn extra(&self, node: &PlanNode) -> Option<&PlanExtra> {
        (node.extra != NO_EXTRA).then(|| &self.extras[node.extra as usize])
    }

    /// The root nodes, in order.
    fn roots(&self) -> impl Iterator<Item = u32> + '_ {
        self.siblings_from(0, self.nodes.len() as u32)
    }

    /// The children of node `index`, in order.
    fn children(&self, index: u32) -> impl Iterator<Item = u32> + '_ {
        self.siblings_from(index + 1, self.nodes[index as usize].end)
    }

    /// The nodes from `first` on, each after the previous one's subtree, before `end`.
    fn siblings_from(&self, first: u32, end: u32) -> impl Iterator<Item = u32> + '_ {
        let mut next = first;
        std::iter::from_fn(move || {
            if next >= end {
                return None;
            }
            let current = next;
            next = self.nodes[current as usize].end;
            Some(current)
        })
    }

    /// The nodes node `index` references (by property or attribute).
    fn references(&self, index: u32) -> impl Iterator<Item = u32> + '_ {
        let node = &self.nodes[index as usize];
        let props = self.props[node.props.range()].iter().filter(|prop| prop.value & NODE_REF != 0).map(|prop| prop.value & !NODE_REF);
        let attributes = self.extra(node).map_or(&[][..], |extra| &self.attributes[extra.attributes.range()]).iter().filter_map(|attribute| match attribute.get() {
            Some(PlanAttribute::Instance(target)) => Some(target),
            _ => None,
        });
        props.chain(attributes)
    }

    /// Makes the plan's instances in `engine` (the roots children of `parent` when given), as
    /// ConvertLune makes them; returns the roots.
    pub fn instantiate(&self, engine: &Engine, parent: Option<InstanceId>) -> Vec<InstanceId> {
        let reflection = &engine.reflection;
        let mut texts = Texts::default();
        let mut ids: Vec<InstanceId> = Vec::with_capacity(self.nodes.len());
        let mut result = Vec::new();
        {
            let mut state = engine.state.borrow_mut();
            for node in self.nodes.iter() {
                let name = texts.get(self, node.name);
                let id = core::create_named_str(&mut state, reflection, node.class, name);
                match node.parent() {
                    Some(index) => state.dom.attach(id, ids[index as usize]),
                    None => {
                        if let Some(parent) = parent {
                            state.dom.attach(id, parent);
                        }
                        result.push(id);
                    }
                }
                ids.push(id);
            }
        }
        for warning in &self.warnings {
            engine.warn(warning.clone());
        }
        let mut state = engine.state.borrow_mut();
        for (index, &id) in ids.iter().enumerate() {
            fill(&mut state, reflection, self, index as u32, id, &mut texts, None, |target| ids[target as usize]);
        }
        result
    }

    /// How many roots the plan has (a manifest bundle as uploaded has one: the instance the game
    /// installs at its path).
    pub fn root_count(&self) -> usize {
        self.roots().count()
    }

    /// The names of the children of the plan's first root (what [`ModelPlan::instantiate_lazy_as`]
    /// and [`ModelPlan::instantiate_lazy_members`] make under the instance they are given).
    pub fn first_root_child_names(&self) -> impl Iterator<Item = &[u8]> {
        let children = if self.nodes.is_empty() { 1..1 } else { 1..self.nodes[0].end };
        self.siblings_from(children.start, children.end).map(|index| self.bytes(self.nodes[index as usize].name))
    }

    /// Makes the plan's roots as children of `parent` and the rest of its instances as they are
    /// looked at ([`materialize_lazy`]: before a script lists or finds the children of one, moves it
    /// or calls one of its methods), with the serials, build work and joints that making them all
    /// now would note. The instances a made instance references are made with it, and the tagged
    /// ones at once. Returns the roots.
    pub fn instantiate_lazy(self: &Arc<Self>, state: &mut State, reflection: &Reflection, parent: InstanceId) -> Vec<InstanceId> {
        self.import_lazy(state, reflection, Placing::Roots(parent))
    }

    /// [`ModelPlan::instantiate_lazy`] of a plan of one root that `target` stands for (a manifest
    /// bundle's placeholder at the bundle's path, which a script may hold already): `target` becomes
    /// the root, taking its class, properties, attributes and tags and keeping its own name, children
    /// and identity, and the root's children are made after its own. The plan must have one root.
    pub fn instantiate_lazy_as(self: &Arc<Self>, state: &mut State, reflection: &Reflection, target: InstanceId) {
        debug_assert_eq!(self.root_count(), 1);
        self.import_lazy(state, reflection, Placing::As(target));
    }

    /// [`ModelPlan::instantiate_lazy`] of the children of a plan's one root as children of `parent`
    /// (a loose manifest bundle: the game moves its instance's members into the parent folder and
    /// drops the instance, so the root itself is never made and references to it read nil). Returns
    /// the members.
    pub fn instantiate_lazy_members(self: &Arc<Self>, state: &mut State, reflection: &Reflection, parent: InstanceId) -> Vec<InstanceId> {
        debug_assert_eq!(self.root_count(), 1);
        self.import_lazy(state, reflection, Placing::Members(parent))
    }

    fn import_lazy(self: &Arc<Self>, state: &mut State, reflection: &Reflection, placing: Placing) -> Vec<InstanceId> {
        let count = self.nodes.len() as u32;
        let serial_base = state.dom.reserve_serials(count);
        state.sched.note_build_work_many(count);
        for node in self.nodes.iter() {
            state.joints.note_created(node.class);
        }
        let import = state.project.imports.len() as u32;
        state.project.imports.push(LazyImport { plan: self.clone(), ids: vec![UNMADE; self.nodes.len()], serial_base, texts: Texts::default(), props: Rc::new(PlanProps::new(self.clone())) });
        // (and whether references across what it makes at once make everything now)
        let (made, above, eager): (Vec<u32>, InstanceId, bool) = match placing {
            Placing::Roots(parent) => {
                let roots: Vec<u32> = self.roots().collect();
                make_level(state, reflection, import, parent, &roots);
                (roots, parent, self.cross_root)
            }
            Placing::As(target) => {
                // (a folder's children are made at once, each its own region as a folder bundle's
                // roots were; any other root is one region, made whole before it or anything in it moves)
                let region = make_as(state, reflection, import, 0, target);
                (vec![0], target, !region && self.cross_member)
            }
            Placing::Members(parent) => {
                let members: Vec<u32> = self.roots().next().map(|root| self.children(root).collect()).unwrap_or_default();
                make_level(state, reflection, import, parent, &members);
                (members, parent, self.cross_member)
            }
        };
        for &node in self.tagged.iter() {
            ensure_made(state, reflection, import, node);
        }
        if eager {
            let ids = state.project.imports[import as usize].ids.clone();
            for &node in &made {
                materialize_lazy_subtree(state, reflection, ids[node as usize]);
            }
        }
        // the folders above hold lazily made instances: moving one makes them first
        let mut current = Some(above);
        while let Some(node) = current {
            let instance = state.dom.at_mut(node);
            if instance.has(flags::LAZY_ABOVE) {
                break;
            }
            instance.set_flag(flags::LAZY_ABOVE, true);
            current = instance.parent;
        }
        let ids = &state.project.imports[import as usize].ids;
        made.iter().map(|&node| ids[node as usize]).collect()
    }

    /// Value `index` of the plan (a checked plan holds only values there are).
    fn val(&self, index: u32) -> PlanVal {
        self.values[index as usize].get().unwrap_or(PlanVal::Nil)
    }

    /// Value `index` of the plan as an engine value: made once per instantiation, then shared by the
    /// instances that hold it (values are immutable: shared datatype values read as their own copies).
    fn value(&self, reflection: &Reflection, texts: &mut Texts, index: u32) -> Value {
        if let Some(Some(made)) = texts.values.get(index as usize) {
            return made.clone();
        }
        let made = plan_value(self, reflection, texts, &self.val(index));
        if texts.values.len() < self.values.len() {
            texts.values.resize(self.values.len(), None);
        }
        texts.values[index as usize] = Some(made.clone());
        made
    }
}

/// A plan value as an engine value (strings made once per instantiation).
fn plan_value(plan: &ModelPlan, reflection: &Reflection, texts: &mut Texts, value: &PlanVal) -> Value {
    match value {
        PlanVal::Nil => Value::Nil,
        PlanVal::Bool(b) => Value::Bool(*b),
        PlanVal::Number(n) => Value::Number(*n),
        PlanVal::EnumItem(item) => Value::EnumItem(*item),
        PlanVal::UDim(u) => Value::UDim(*u),
        PlanVal::UDim2(u) => Value::UDim2(*u),
        PlanVal::Vector2(v) => Value::Vector2(*v),
        PlanVal::Vector3(v) => Value::Vector3(*v),
        PlanVal::Color3(c) => Value::Color3(*c),
        PlanVal::Rect(r) => Value::Rect(*r),
        PlanVal::NumberRange(r) => Value::NumberRange(*r),
        PlanVal::Text(span) => Value::String(texts.get(plan, *span)),
        PlanVal::CFrame(c) => Value::Dom(Rc::new(Variant::CFrame(*c))),
        // (from_dom made a value of it when the plan was converted)
        PlanVal::Other(index) => match &plan.others[*index as usize] {
            // (what value::from_dom makes of these, without the rbx-dom value)
            FlatVariant::Content(None) => Value::Content(Rc::new(value::ContentValue::None)),
            FlatVariant::Content(Some(span)) if span.len > 0 => Value::Content(Rc::new(value::ContentValue::Uri(texts.get(plan, *span)))),
            FlatVariant::Content(Some(_)) => Value::Content(Rc::new(value::ContentValue::None)),
            FlatVariant::Plain(variant) => value::from_dom(reflection, variant).unwrap_or(Value::Nil),
            other => value::from_dom(reflection, &other.variant(&plan.text, &plan.floats)).unwrap_or(Value::Nil),
        },
    }
}

/// The strings an instantiation made of a plan's text, by span, and the values it made of the plan's
/// distinct values, by index (each made once).
#[derive(Default)]
pub struct Texts {
    made: FxHashMap<u64, Str>,
    values: Vec<Option<Value>>,
}

impl Texts {
    fn get(&mut self, plan: &ModelPlan, span: Span) -> Str {
        // (an empty string starts where the next one does)
        let key = (span.start as u64) << 32 | span.len as u64;
        self.made.entry(key).or_insert_with(|| Str::new(plan.bytes(span))).clone()
    }
}

/// The properties a model file held that the engine does not model (a MeshPart's InitialSize and
/// physics data, ...), kept as read so an instance made of it can be written back out.
#[derive(Clone, Debug)]
pub struct Unmodeled {
    table: Arc<UnmodeledTable>,
    span: Span,
}

impl Unmodeled {
    /// (name, value) of each, in the file's order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Variant)> + '_ {
        self.table.entries[self.span.range()].iter().map(|entry| {
            let name = std::str::from_utf8(&self.table.text[entry.name.range()]).unwrap_or_default();
            let value = match entry.value() {
                Some(UnmodeledValue::Id(id)) => Variant::UniqueId(id),
                Some(UnmodeledValue::Shared(index)) => self.table.values[index as usize].variant(&self.table.text, &self.table.floats),
                None => Variant::String(String::new()),
            };
            (name, value)
        })
    }
}

/// The property values of a lazily imported plan's made nodes as engine values, made once per import,
/// and the Name each made node stores: the properties of an instance made of a node without instance
/// references, and of its unchanged copies, are read through it ([`Props::Plan`]) until written. Its
/// nodes' attributes too ([`crate::dom::Attributes::Plan`]), made when first read.
pub struct PlanProps {
    plan: Arc<ModelPlan>,
    /// The plan's distinct values, each made when a node holding it is made
    values: Box<[std::cell::OnceCell<Value>]>,
    /// The Name stored on each made node
    names: Box<[std::cell::OnceCell<Value>]>,
    /// The attributes of each node with extras (by its extra), made when first read
    attributes: Box<[std::cell::OnceCell<Box<[(Str, Value)]>>]>,
    /// What makes them: the reflection and the strings made so far
    reflection: Option<Arc<Reflection>>,
    texts: std::cell::RefCell<Texts>,
}

impl PlanProps {
    fn new(plan: Arc<ModelPlan>) -> PlanProps {
        let values = (0..plan.values.len()).map(|_| std::cell::OnceCell::new()).collect();
        let names = (0..plan.nodes.len()).map(|_| std::cell::OnceCell::new()).collect();
        let attributes = (0..plan.extras.len()).map(|_| std::cell::OnceCell::new()).collect();
        PlanProps { plan, values, names, attributes, reflection: Reflection::shared().ok(), texts: Default::default() }
    }

    /// Node `node`'s attributes, made now when they were not (a node read through it has some and
    /// none references an instance: [`fill`]).
    pub(crate) fn attributes(&self, node: u32) -> &[(Str, Value)] {
        let extra = self.plan.nodes[node as usize].extra;
        if extra == NO_EXTRA {
            return &[];
        }
        self.attributes[extra as usize].get_or_init(|| {
            let (plan, Some(reflection)) = (&self.plan, self.reflection.as_deref()) else { return Box::new([]) };
            let mut texts = self.texts.borrow_mut();
            plan.attributes[plan.extras[extra as usize].attributes.range()]
                .iter()
                .map(|attribute| {
                    let value = match attribute.get() {
                        Some(PlanAttribute::Value(value)) => plan_value(plan, reflection, &mut texts, &value),
                        _ => Value::Nil,
                    };
                    (texts.get(plan, attribute.name), value)
                })
                .collect()
        })
    }

    /// How many attributes node `node` has.
    pub(crate) fn attribute_count(&self, node: u32) -> usize {
        let extra = self.plan.nodes[node as usize].extra;
        if extra == NO_EXTRA { 0 } else { self.plan.extras[extra as usize].attributes.len as usize }
    }

    /// Node `node`'s stored value of property `name`.
    #[inline]
    pub(crate) fn get(&self, node: u32, name: NameId) -> Option<&Value> {
        if name == K::Name as NameId {
            return self.names[node as usize].get();
        }
        let pairs = &self.plan.props[self.plan.nodes[node as usize].props.range()];
        let index = pairs.binary_search_by_key(&name, |prop| prop.name).ok()?;
        self.values[pairs[index].value as usize].get()
    }

    /// Node `node`'s stored (name, value) pairs, sorted by name.
    pub(crate) fn iter(&self, node: u32) -> PlanPropsIter<'_> {
        PlanPropsIter { table: self, pairs: &self.plan.props[self.plan.nodes[node as usize].props.range()], name: self.names[node as usize].get() }
    }

    /// Value `index` of the plan as an engine value, made once.
    fn value(&self, reflection: &Reflection, texts: &mut Texts, index: u32) -> &Value {
        self.values[index as usize].get_or_init(|| plan_value(&self.plan, reflection, texts, &self.plan.val(index)))
    }
}

/// [`PlanProps::iter`]: the plan node's pairs with the stored Name merged in.
pub struct PlanPropsIter<'a> {
    table: &'a PlanProps,
    pairs: &'a [Prop],
    name: Option<&'a Value>,
}

impl<'a> Iterator for PlanPropsIter<'a> {
    type Item = (NameId, &'a Value);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        const NAME: NameId = K::Name as NameId;
        if let Some(name) = self.name {
            if self.pairs.first().is_none_or(|prop| prop.name > NAME) {
                self.name = None;
                return Some((NAME, name));
            }
        }
        let (prop, rest) = self.pairs.split_first()?;
        self.pairs = rest;
        Some((prop.name, self.table.values[prop.value as usize].get().expect("a node's values are made with it")))
    }
}

/// A reference to a node's instance: nil for a node an import never makes (a loose bundle's root).
fn made_or_nil(id: InstanceId) -> Value {
    if id == UNMADE { Value::Nil } else { Value::Instance(id) }
}

/// Stores a made plan node's properties, attributes, tags, source and unmodeled values on `id`
/// (`resolve` gives the instance made for a node it references). With `table` (a lazy import's), a node
/// without instance references whose instance stores only its Name reads its properties through it.
#[allow(clippy::too_many_arguments)]
fn fill(state: &mut State, reflection: &Reflection, plan: &ModelPlan, index: u32, id: InstanceId, texts: &mut Texts, table: Option<&Rc<PlanProps>>, resolve: impl Fn(u32) -> InstanceId) {
    let node = &plan.nodes[index as usize];
    let extra = plan.extra(node).copied().unwrap_or_default();
    let tags: Vec<Str> = plan.tags[extra.tags.range()].iter().map(|&tag| texts.get(plan, tag)).collect();
    for tag in &tags {
        services::note_tag(state, id, tag);
    }
    let own = &plan.props[node.props.range()];
    let instance = state.dom.at_mut(id);
    let only_name = matches!(&instance.props, Props::Own(stored) if stored.len() == 1 && stored[0].0 == K::Name as NameId);
    match table {
        Some(table) if node.refs == 0 && only_name && table.names[index as usize].get().is_none() && own.iter().all(|prop| prop.name != K::Name as NameId) => {
            if let Props::Own(stored) = std::mem::take(&mut instance.props) {
                let _ = table.names[index as usize].set(stored.into_iter().next().map(|(_, name)| name).unwrap_or_default());
            }
            for prop in own {
                table.value(reflection, texts, prop.value);
            }
            instance.props = Props::Plan(table.clone(), index);
        }
        _ => {
            // (what storing them one by one makes: the stored Name merged in)
            let stored = std::mem::take(instance.props.own_mut());
            let mut props = Vec::with_capacity(stored.len() + own.len());
            let mut stored = stored.into_iter().peekable();
            for &Prop { name, value } in own {
                while let Some(earlier) = stored.next_if(|(other, _)| *other < name) {
                    props.push(earlier);
                }
                stored.next_if(|(other, _)| *other == name);
                let value = if value & NODE_REF != 0 {
                    made_or_nil(resolve(value & !NODE_REF))
                } else {
                    match table {
                        Some(table) => table.value(reflection, texts, value).clone(),
                        None => plan.value(reflection, texts, value),
                    }
                };
                props.push((name, value));
            }
            props.extend(stored);
            instance.props = Props::Own(props);
        }
    }
    if node.refs != 0 {
        instance.flags |= flags::HOLDS_REFS;
    }
    let attributes = &plan.attributes[extra.attributes.range()];
    match table {
        // (read through the import, made when first read: the values a node holds are made of its plan alone)
        Some(table) if !attributes.is_empty() && table.reflection.is_some() && attributes.iter().all(|attribute| attribute.kind == 0) => {
            instance.attributes = crate::dom::Attributes::Plan(table.clone(), index);
        }
        _ => {
            instance.attributes = attributes
                .iter()
                .map(|attribute| {
                    let value = match attribute.get() {
                        Some(PlanAttribute::Value(value)) => plan_value(plan, reflection, texts, &value),
                        Some(PlanAttribute::Instance(target)) => made_or_nil(resolve(target)),
                        None => Value::Nil,
                    };
                    (texts.get(plan, attribute.name), value)
                })
                .collect::<Vec<_>>()
                .into();
        }
    }
    instance.tags = tags.into();
    if let Some(code) = extra.code() {
        instance.set_source(Some(Rc::from(plan.bytes(code))));
    }
    if node.unmodeled.len > 0 {
        instance.unmodeled = Some(Unmodeled { table: plan.unmodeled.clone(), span: node.unmodeled });
    }
}

/// A plan imported lazily ([`ModelPlan::instantiate_lazy`]) and the instances made of it so far.
pub struct LazyImport {
    plan: Arc<ModelPlan>,
    /// The instance made for each node ([`UNMADE`] while it is not)
    ids: Vec<InstanceId>,
    /// Node 0's serial (each node has the serial making them all at once would have given it)
    serial_base: u32,
    texts: Texts,
    /// The values its made instances read their properties through
    props: Rc<PlanProps>,
}

impl LazyImport {
    /// The instances the import holds, made or not.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

const UNMADE: InstanceId = InstanceId { index: u32::MAX, generation: u32::MAX };

/// Where a lazy import puts what it makes at once.
enum Placing {
    /// The plan's roots, as children of this instance
    Roots(InstanceId),
    /// The plan's one root is this instance (which takes the root's class and values)
    As(InstanceId),
    /// The children of the plan's one root, as children of this instance (the root is not made)
    Members(InstanceId),
}

/// Makes plan nodes `nodes` of import `import` (siblings, in order) as children of `parent`:
/// first the instances, then their values (making the instances they reference).
fn make_level(state: &mut State, reflection: &Reflection, import: u32, parent: InstanceId, nodes: &[u32]) {
    let (plan, serial_base) = {
        let import = &state.project.imports[import as usize];
        (import.plan.clone(), import.serial_base)
    };
    let mut texts = std::mem::take(&mut state.project.imports[import as usize].texts);
    for &index in nodes {
        let node = &plan.nodes[index as usize];
        let id = state.dom.create_with_serial(reflection, node.class, serial_base + index);
        core::raw_set_silent(state, reflection, id, K::Name as NameId, Value::String(texts.get(&plan, node.name)));
        let pending = node.end > index + 1;
        let instance = state.dom.at_mut(id);
        instance.flags |= flags::MATERIALIZED | flags::LAZY_REGION | if pending { flags::LAZY_PENDING } else { 0 };
        state.dom.attach(id, parent);
        state.project.imports[import as usize].ids[index as usize] = id;
        if pending {
            state.project.lazy.insert(id, (import, index));
        }
        state.project.lazy_made += 1;
    }
    state.project.imports[import as usize].texts = texts;
    for &index in nodes {
        let targets: Vec<u32> = plan.references(index).collect();
        for target in targets {
            ensure_made(state, reflection, import, target);
        }
        let id = state.project.imports[import as usize].ids[index as usize];
        let ids = std::mem::take(&mut state.project.imports[import as usize].ids);
        let mut texts = std::mem::take(&mut state.project.imports[import as usize].texts);
        let table = state.project.imports[import as usize].props.clone();
        fill(state, reflection, &plan, index, id, &mut texts, Some(&table), |target| ids[target as usize]);
        let import = &mut state.project.imports[import as usize];
        import.ids = ids;
        import.texts = texts;
    }
}

/// Makes `target`, an instance made before the import, plan root `index` of import `import`: it
/// takes the node's class, properties, attributes and tags (keeping its stored Name and its children,
/// before which none of the node's come) and stops being a Studio placeholder; then the node's
/// children are made after its own children. A root of a class other than Folder is a lazily
/// imported region of its own (its whole tree is made before it, or anything in it, moves or runs a
/// method that may look below it) and adds no Studio-only children later (the game replaces whatever
/// stood at the path; it merges only folders); a folder stays what a folder bundle's folder was, the
/// place of regions. Returns whether `target` is a region.
fn make_as(state: &mut State, reflection: &Reflection, import: u32, index: u32, target: InstanceId) -> bool {
    let plan = state.project.imports[import as usize].plan.clone();
    let node = &plan.nodes[index as usize];
    state.dom.set_class(reflection, target, node.class);
    let pending = node.end > index + 1;
    let region = reflection.class_id("Folder") != Some(node.class);
    let instance = state.dom.at_mut(target);
    instance.flags &= !(flags::STUDIO_ONLY | flags::BUNDLE_ROOT);
    if pending {
        instance.flags |= flags::LAZY_PENDING;
    }
    if region {
        instance.flags |= flags::LAZY_REGION | flags::MATERIALIZED;
        instance.studio_node = crate::dom::StudioNode::None;
    }
    state.project.imports[import as usize].ids[index as usize] = target;
    if pending {
        state.project.lazy.insert(target, (import, index));
    }
    // (the instances its values reference first, which may make its children)
    let targets: Vec<u32> = plan.references(index).collect();
    for referenced in targets {
        ensure_made(state, reflection, import, referenced);
    }
    let ids = std::mem::take(&mut state.project.imports[import as usize].ids);
    let mut texts = std::mem::take(&mut state.project.imports[import as usize].texts);
    let table = state.project.imports[import as usize].props.clone();
    fill(state, reflection, &plan, index, target, &mut texts, Some(&table), |referenced| ids[referenced as usize]);
    let import_state = &mut state.project.imports[import as usize];
    import_state.ids = ids;
    import_state.texts = texts;
    // its children now, as the import of a folder made its roots at once: a recursive search of it
    // then looks at its own children (a nested bundle's placeholder among them) and not only the plan
    materialize_lazy(state, reflection, target);
    region
}

/// Makes plan node `index` of import `import` (with the children of each of its ancestors on the
/// way) when it is not made yet.
fn ensure_made(state: &mut State, reflection: &Reflection, import: u32, index: u32) {
    let plan = state.project.imports[import as usize].plan.clone();
    let mut chain = Vec::new();
    let mut current = index;
    while state.project.imports[import as usize].ids[current as usize] == UNMADE {
        chain.push(current);
        match plan.nodes[current as usize].parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    for &node in chain.iter().rev() {
        let Some(parent) = plan.nodes[node as usize].parent() else { continue };
        let parent_id = state.project.imports[import as usize].ids[parent as usize];
        if parent_id != UNMADE {
            materialize_lazy(state, reflection, parent_id);
        }
    }
}

/// Makes the plan children of a lazily imported instance when they are not made yet.
pub fn materialize_lazy(state: &mut State, reflection: &Reflection, id: InstanceId) {
    let Some((import, index)) = state.project.lazy.remove(&id) else { return };
    state.dom.at_mut(id).set_flag(flags::LAZY_PENDING, false);
    let plan = state.project.imports[import as usize].plan.clone();
    let children: Vec<u32> = plan.children(index).collect();
    make_level(state, reflection, import, id, &children);
}

/// A recursive FindFirstChild / FindFirstChildWhichIsA of `id`, a lazily imported instance whose
/// children are not made yet: the search over its plan, in the order the made instances would be
/// searched (each instance's children, then each child's own search), returning the instance it finds,
/// made with the children of its ancestors. Nothing it passes over is made (nothing below an
/// unmade instance can have changed), so a search of a whole asset tree makes no instance it does not
/// find. None when `id`'s children are made (the caller searches them).
pub fn find_unmade(state: &mut State, reflection: &Reflection, id: InstanceId, matches: impl Fn(&[u8], ClassId) -> bool) -> Option<Option<InstanceId>> {
    let &(import, index) = state.project.lazy.get(&id)?;
    let plan = state.project.imports[import as usize].plan.clone();
    fn search(plan: &ModelPlan, node: u32, matches: &impl Fn(&[u8], ClassId) -> bool) -> Option<u32> {
        let found = plan.children(node).find(|&child| matches(plan.bytes(plan.nodes[child as usize].name), plan.nodes[child as usize].class));
        found.or_else(|| plan.children(node).find_map(|child| search(plan, child, matches)))
    }
    let Some(found) = search(&plan, index, &matches) else { return Some(None) };
    ensure_made(state, reflection, import, found);
    Some(Some(state.project.imports[import as usize].ids[found as usize]))
}

/// Makes every lazily imported instance below `root` (and `root`'s own children): what moving or
/// destroying a subtree fires events for and what a method may look at.
pub fn materialize_lazy_subtree(state: &mut State, reflection: &Reflection, root: InstanceId) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if state.dom.at(node).has(flags::LAZY_PENDING) {
            materialize_lazy(state, reflection, node);
        }
        let dom = &state.dom;
        stack.extend(dom.at(node).children.iter().copied().filter(|&child| dom.at(child).flags & (flags::LAZY_REGION | flags::LAZY_ABOVE) != 0));
    }
}

/// Before a method of `id` runs: a lazily imported instance's whole import root is made first
/// (the method may look at its descendants, joints or assembly), unless the method only reads the
/// instance itself or finds and lists children, which make what they look at.
pub fn before_method(state: &mut State, reflection: &Reflection, id: InstanceId, method: NameId) {
    if !state.dom.at(id).has(flags::LAZY_REGION) {
        return;
    }
    const READS_ITSELF: &[&str] = &[
        "FindFirstChild", "findFirstChild", "FindFirstChildOfClass", "FindFirstChildWhichIsA", "FindFirstAncestor",
        "FindFirstAncestorOfClass", "FindFirstAncestorWhichIsA", "GetChildren", "getChildren", "children", "GetDescendants",
        "IsA", "isA", "IsDescendantOf", "IsAncestorOf", "GetAttribute", "GetAttributes", "SetAttribute",
        "GetAttributeChangedSignal", "GetPropertyChangedSignal", "HasTag", "GetTags", "AddTag", "RemoveTag", "GetFullName",
        "WaitForChild", "Clone", "clone", "GetDebugId", "QueryDescendants", "FindFirstDescendant", "GetStyled",
    ];
    if READS_ITSELF.contains(&reflection.name(method)) {
        return;
    }
    materialize_region(state, reflection, id);
}

/// Makes every lazily imported instance below `id` and, for one that was imported lazily, the rest
/// of its import root (what references and joints within a model reach): before it moves, as its
/// events and the instances that reference it need.
pub fn materialize_region(state: &mut State, reflection: &Reflection, id: InstanceId) {
    let mut root = id;
    while let Some(parent) = state.dom.at(root).parent.filter(|&parent| state.dom.at(parent).has(flags::LAZY_REGION)) {
        root = parent;
    }
    materialize_lazy_subtree(state, reflection, root);
}

fn full_dom_name(model: &WeakDom, referent: Ref) -> String {
    let mut parts = Vec::new();
    let mut current = model.get_by_ref(referent);
    while let Some(instance) = current {
        if instance.referent() == model.root_ref() {
            break;
        }
        parts.push(instance.name.clone());
        current = model.get_by_ref(instance.parent());
    }
    parts.reverse();
    parts.join(".")
}

/// ConvertLune for `roots` of a decoded model.
pub fn convert_instances(engine: &Engine, model: &WeakDom, roots: &[Ref], parent: Option<InstanceId>) -> Vec<InstanceId> {
    Arc::new(ModelPlan::new(&engine.reflection, model, roots)).instantiate(engine, parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plan is a few arrays that cross threads; its values are distinct by their bits.
    #[test]
    fn plans_are_shared_and_values_distinct_by_bits() {
        fn shared<T: Send + Sync>() {}
        shared::<ModelPlan>();
        let mut key = Vec::new();
        assert!(value_key(&PlanVal::Number(0.0), &mut key));
        let zero = key.clone();
        assert!(value_key(&PlanVal::Number(-0.0), &mut key));
        assert_ne!(zero, key, "-0 is not 0");
        assert!(!variant_key(&Variant::Ray(dom::Ray::new(dom::Vector3::new(0.0, 0.0, 0.0), dom::Vector3::new(1.0, 0.0, 0.0))), &mut key), "floats a key leaves out");
    }
}
