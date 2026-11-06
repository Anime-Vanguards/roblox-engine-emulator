//! A [`ModelPlan`] as bytes, for the converted-bundle files beside the bundle files
//! (project::model_assets): a run that finds one reads its plan instead of decoding and converting
//! the model file again. The bytes hold the plan's arrays as they are: the plain ones (nodes,
//! properties, values, text, attributes, tags, extras, unmodeled entries, floats) as their records'
//! little-endian bytes, each 4-aligned after its count, so a mapped file is read in place
//! (store.rs); the other datatypes' values after them. A file is only valid for the conversion that
//! wrote it (its key says which: build.rs digests the conversion's code and data); a plan holding a
//! value the format leaves out (rare datatypes) is not written.

use std::sync::Arc;

use rbx_dom_weak::types::{self as dom, Variant};

use super::*;

const MAGIC: &[u8; 8] = b"REEPLAN4";

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn i16(&mut self, value: i16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn i32(&mut self, value: i32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }
    fn f64(&mut self, value: f64) {
        self.0.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    fn len(&mut self, value: usize) {
        self.u32(value as u32);
    }
    fn span(&mut self, span: Span) {
        self.u32(span.start);
        self.u32(span.len);
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.0.extend_from_slice(bytes);
    }
    fn vector3(&mut self, v: dom::Vector3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    fn cframe(&mut self, c: &dom::CFrame) {
        self.vector3(c.position);
        self.vector3(c.orientation.x);
        self.vector3(c.orientation.y);
        self.vector3(c.orientation.z);
    }
    /// Zeros up to the next multiple of 4.
    fn align(&mut self) {
        while self.0.len() % 4 != 0 {
            self.0.push(0);
        }
    }
    /// A plain array: its count, its records' bytes, then [`Writer::align`].
    fn plain<T: Pod>(&mut self, items: &Array<T>) {
        self.len(items.len());
        self.0.extend_from_slice(items.bytes());
        self.align();
    }
}

struct Reader<'a> {
    /// Everything read (offsets of plain arrays count from its start)
    all: &'a [u8],
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.bytes.len() < count {
            return None;
        }
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Some(taken)
    }
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.array()?))
    }
    fn i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.array()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.array()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_bits(u64::from_le_bytes(self.array()?)))
    }
    /// A count of items of at least `size` bytes each (bounded by what is left)
    fn len(&mut self, size: usize) -> Option<usize> {
        let count = self.u32()? as usize;
        (count.saturating_mul(size.max(1)) <= self.bytes.len()).then_some(count)
    }
    fn span(&mut self) -> Option<Span> {
        Some(Span { start: self.u32()?, len: self.u32()? })
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let count = self.len(1)?;
        self.take(count)
    }
    fn vector3(&mut self) -> Option<dom::Vector3> {
        Some(dom::Vector3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn cframe(&mut self) -> Option<dom::CFrame> {
        let position = self.vector3()?;
        Some(dom::CFrame::new(position, dom::Matrix3::new(self.vector3()?, self.vector3()?, self.vector3()?)))
    }
    /// Skips up to the next multiple of 4.
    fn align(&mut self) -> Option<()> {
        let offset = self.all.len() - self.bytes.len();
        self.take((4 - offset % 4) % 4).map(|_| ())
    }
    /// A plain array [`Writer::plain`] wrote: in place when `file` holds the bytes read.
    fn plain<T: Pod>(&mut self, file: Option<&Arc<Mapping>>) -> Option<Array<T>> {
        let count = self.u32()? as usize;
        let size = count.checked_mul(std::mem::size_of::<T>())?;
        let offset = self.all.len() - self.bytes.len();
        self.take(size)?;
        self.align()?;
        Some(Array::read(self.all, file, offset, count))
    }
    fn list<T>(&mut self, size: usize, mut item: impl FnMut(&mut Self) -> Option<T>) -> Option<Box<[T]>> {
        let count = self.len(size)?;
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(item(self)?);
        }
        Some(items.into_boxed_slice())
    }
}

/// A heap-free rbx-dom value; false for one the format leaves out.
fn write_variant(out: &mut Writer, variant: &Variant) -> bool {
    match variant {
        Variant::Axes(v) => {
            out.u8(0);
            out.u8(v.bits());
        }
        Variant::Bool(b) => {
            out.u8(1);
            out.u8(*b as u8);
        }
        Variant::BrickColor(c) => {
            out.u8(2);
            out.u16(*c as u16);
        }
        Variant::CFrame(c) => {
            out.u8(3);
            out.cframe(c);
        }
        Variant::Color3(c) => {
            out.u8(4);
            out.f32(c.r);
            out.f32(c.g);
            out.f32(c.b);
        }
        Variant::Color3uint8(c) => {
            out.u8(5);
            out.u8(c.r);
            out.u8(c.g);
            out.u8(c.b);
        }
        Variant::Enum(e) => {
            out.u8(6);
            out.u32(e.to_u32());
        }
        Variant::Faces(f) => {
            out.u8(7);
            out.u8(f.bits());
        }
        Variant::Float32(n) => {
            out.u8(8);
            out.f32(*n);
        }
        Variant::Float64(n) => {
            out.u8(9);
            out.f64(*n);
        }
        Variant::Int32(n) => {
            out.u8(10);
            out.i32(*n);
        }
        Variant::Int64(n) => {
            out.u8(11);
            out.i64(*n);
        }
        Variant::NumberRange(r) => {
            out.u8(12);
            out.f32(r.min);
            out.f32(r.max);
        }
        Variant::OptionalCFrame(None) => out.u8(13),
        Variant::OptionalCFrame(Some(c)) => {
            out.u8(14);
            out.cframe(c);
        }
        Variant::PhysicalProperties(dom::PhysicalProperties::Default) => out.u8(15),
        Variant::PhysicalProperties(dom::PhysicalProperties::Custom(p)) => {
            out.u8(16);
            for value in [p.density(), p.friction(), p.elasticity(), p.friction_weight(), p.elasticity_weight(), p.acoustic_absorption()] {
                out.f32(value);
            }
        }
        Variant::Ray(r) => {
            out.u8(17);
            out.vector3(r.origin);
            out.vector3(r.direction);
        }
        Variant::Rect(r) => {
            out.u8(18);
            out.f32(r.min.x);
            out.f32(r.min.y);
            out.f32(r.max.x);
            out.f32(r.max.y);
        }
        Variant::Region3(r) => {
            out.u8(19);
            out.vector3(r.min);
            out.vector3(r.max);
        }
        Variant::Region3int16(r) => {
            out.u8(20);
            for value in [r.min.x, r.min.y, r.min.z, r.max.x, r.max.y, r.max.z] {
                out.i16(value);
            }
        }
        Variant::UDim(u) => {
            out.u8(21);
            out.f32(u.scale);
            out.i32(u.offset);
        }
        Variant::UDim2(u) => {
            out.u8(22);
            out.f32(u.x.scale);
            out.i32(u.x.offset);
            out.f32(u.y.scale);
            out.i32(u.y.offset);
        }
        Variant::UniqueId(id) => {
            out.u8(23);
            out.u32(id.index());
            out.u32(id.time());
            out.i64(id.random());
        }
        Variant::Vector2(v) => {
            out.u8(24);
            out.f32(v.x);
            out.f32(v.y);
        }
        Variant::Vector2int16(v) => {
            out.u8(25);
            out.i16(v.x);
            out.i16(v.y);
        }
        Variant::Vector3(v) => {
            out.u8(26);
            out.vector3(*v);
        }
        Variant::Vector3int16(v) => {
            out.u8(27);
            out.i16(v.x);
            out.i16(v.y);
            out.i16(v.z);
        }
        _ => return false,
    }
    true
}

fn read_variant(input: &mut Reader) -> Option<Variant> {
    Some(match input.u8()? {
        0 => Variant::Axes(dom::Axes::from_bits(input.u8()?)?),
        1 => Variant::Bool(input.u8()? != 0),
        2 => Variant::BrickColor(dom::BrickColor::from_number(input.u16()?)?),
        3 => Variant::CFrame(input.cframe()?),
        4 => Variant::Color3(dom::Color3::new(input.f32()?, input.f32()?, input.f32()?)),
        5 => Variant::Color3uint8(dom::Color3uint8::new(input.u8()?, input.u8()?, input.u8()?)),
        6 => Variant::Enum(dom::Enum::from_u32(input.u32()?)),
        7 => Variant::Faces(dom::Faces::from_bits(input.u8()?)?),
        8 => Variant::Float32(input.f32()?),
        9 => Variant::Float64(input.f64()?),
        10 => Variant::Int32(input.i32()?),
        11 => Variant::Int64(input.i64()?),
        12 => Variant::NumberRange(dom::NumberRange::new(input.f32()?, input.f32()?)),
        13 => Variant::OptionalCFrame(None),
        14 => Variant::OptionalCFrame(Some(input.cframe()?)),
        15 => Variant::PhysicalProperties(dom::PhysicalProperties::Default),
        16 => Variant::PhysicalProperties(dom::PhysicalProperties::Custom(dom::CustomPhysicalProperties::new(
            input.f32()?,
            input.f32()?,
            input.f32()?,
            input.f32()?,
            input.f32()?,
            input.f32()?,
        ))),
        17 => Variant::Ray(dom::Ray::new(input.vector3()?, input.vector3()?)),
        18 => Variant::Rect(dom::Rect::new(dom::Vector2::new(input.f32()?, input.f32()?), dom::Vector2::new(input.f32()?, input.f32()?))),
        19 => Variant::Region3(dom::Region3::new(input.vector3()?, input.vector3()?)),
        20 => {
            let min = dom::Vector3int16::new(input.i16()?, input.i16()?, input.i16()?);
            Variant::Region3int16(dom::Region3int16::new(min, dom::Vector3int16::new(input.i16()?, input.i16()?, input.i16()?)))
        }
        21 => Variant::UDim(dom::UDim::new(input.f32()?, input.i32()?)),
        22 => Variant::UDim2(dom::UDim2::new(dom::UDim::new(input.f32()?, input.i32()?), dom::UDim::new(input.f32()?, input.i32()?))),
        23 => Variant::UniqueId(dom::UniqueId::new(input.u32()?, input.u32()?, input.i64()?)),
        24 => Variant::Vector2(dom::Vector2::new(input.f32()?, input.f32()?)),
        25 => Variant::Vector2int16(dom::Vector2int16::new(input.i16()?, input.i16()?)),
        26 => Variant::Vector3(input.vector3()?),
        27 => Variant::Vector3int16(dom::Vector3int16::new(input.i16()?, input.i16()?, input.i16()?)),
        _ => return None,
    })
}

fn write_flat(out: &mut Writer, value: &FlatVariant) -> bool {
    match value {
        FlatVariant::Plain(variant) => {
            out.u8(0);
            return write_variant(out, variant);
        }
        FlatVariant::String(span) => {
            out.u8(1);
            out.span(*span);
        }
        FlatVariant::BinaryString(span) => {
            out.u8(2);
            out.span(*span);
        }
        FlatVariant::ContentId(span) => {
            out.u8(3);
            out.span(*span);
        }
        FlatVariant::SharedString(span) => {
            out.u8(4);
            out.span(*span);
        }
        FlatVariant::Content(None) => out.u8(5),
        FlatVariant::Content(Some(span)) => {
            out.u8(6);
            out.span(*span);
        }
        FlatVariant::NumberSequence(span) => {
            out.u8(7);
            out.span(*span);
        }
        FlatVariant::ColorSequence(span) => {
            out.u8(8);
            out.span(*span);
        }
        FlatVariant::Other(_) => return false,
    }
    true
}

fn read_flat(input: &mut Reader) -> Option<FlatVariant> {
    Some(match input.u8()? {
        0 => FlatVariant::Plain(read_variant(input)?),
        1 => FlatVariant::String(input.span()?),
        2 => FlatVariant::BinaryString(input.span()?),
        3 => FlatVariant::ContentId(input.span()?),
        4 => FlatVariant::SharedString(input.span()?),
        5 => FlatVariant::Content(None),
        6 => FlatVariant::Content(Some(input.span()?)),
        7 => FlatVariant::NumberSequence(input.span()?),
        8 => FlatVariant::ColorSequence(input.span()?),
        _ => return None,
    })
}

impl ModelPlan {
    /// The plan as bytes ([`ModelPlan::decode`] reads them back), with `key` (what makes them valid:
    /// the build and the model file) at their start; None when it holds a value the format leaves out.
    pub fn encode(&self, key: &[u8]) -> Option<Vec<u8>> {
        if cfg!(target_endian = "big") {
            return None;
        }
        let unmodeled = &*self.unmodeled;
        let mut out = Writer::default();
        out.0.extend_from_slice(MAGIC);
        out.bytes(key);
        out.align();
        out.plain(&self.nodes);
        out.plain(&self.props);
        out.plain(&self.values);
        out.plain(&self.floats);
        out.plain(&self.text);
        out.plain(&self.attributes);
        out.plain(&self.tags);
        out.plain(&self.extras);
        out.plain(&unmodeled.entries);
        out.plain(&unmodeled.text);
        out.plain(&unmodeled.floats);
        out.plain(&self.tagged);
        for values in [&self.others, &unmodeled.values] {
            out.len(values.len());
            for value in values.iter() {
                if !write_flat(&mut out, value) {
                    return None;
                }
            }
        }
        out.len(self.warnings.len());
        for warning in &self.warnings {
            out.bytes(warning.as_bytes());
        }
        out.u8(self.cross_root as u8);
        out.u8(self.cross_member as u8);
        Some(out.0)
    }

    /// A plan from [`ModelPlan::encode`]'s bytes when they start with `key`; None for others or
    /// for bytes that do not hold a whole, consistent plan.
    pub fn decode(bytes: &[u8], key: &[u8]) -> Option<ModelPlan> {
        ModelPlan::decode_from(bytes, None, key)
    }

    /// [`ModelPlan::decode`] of a mapped plan file: its plain arrays are read in place.
    pub(crate) fn decode_mapped(file: Arc<Mapping>, key: &[u8]) -> Option<ModelPlan> {
        ModelPlan::decode_from(file.bytes(), Some(&file), key)
    }

    fn decode_from(bytes: &[u8], file: Option<&Arc<Mapping>>, key: &[u8]) -> Option<ModelPlan> {
        if cfg!(target_endian = "big") {
            return None;
        }
        let mut input = Reader { all: bytes, bytes };
        if input.take(MAGIC.len())? != MAGIC || input.bytes()? != key {
            return None;
        }
        input.align()?;
        let nodes = input.plain(file)?;
        let props = input.plain(file)?;
        let values = input.plain(file)?;
        let floats = input.plain(file)?;
        let text = input.plain(file)?;
        let attributes = input.plain(file)?;
        let tags = input.plain(file)?;
        let extras = input.plain(file)?;
        let entries = input.plain(file)?;
        let unmodeled_text = input.plain(file)?;
        let unmodeled_floats = input.plain(file)?;
        let tagged = input.plain(file)?;
        let others = input.list(1, read_flat)?;
        let unmodeled_values = input.list(1, read_flat)?;
        let warnings = input.list(4, |input| Some(String::from_utf8_lossy(input.bytes()?).into_owned()))?.into_vec();
        let cross_root = input.u8()? != 0;
        let cross_member = input.u8()? != 0;
        if !input.bytes.is_empty() {
            return None;
        }
        let unmodeled = Arc::new(UnmodeledTable { entries, values: unmodeled_values, text: unmodeled_text, floats: unmodeled_floats });
        let plan = ModelPlan { nodes, props, values, others, floats, text, attributes, tags, extras, unmodeled, warnings, tagged, cross_root, cross_member };
        plan.consistent().then_some(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};

    /// A plan read back from its bytes is the plan written, and other keys or cut bytes read nothing.
    #[test]
    fn plans_read_back_from_their_bytes() {
        let reflection = crate::Reflection::shared().unwrap();
        let mut model = WeakDom::new(InstanceBuilder::new("DataModel"));
        let root = model.root_ref();
        let rig = model.insert(root, InstanceBuilder::new("Model").with_name("Rig").with_property("Scale", Variant::Float32(0.5)));
        let torso = model.insert(
            rig,
            InstanceBuilder::new("Part")
                .with_name("Torso")
                .with_property("Size", dom::Vector3::new(2.0, 2.0, 1.0))
                .with_property("CFrame", dom::CFrame::new(dom::Vector3::new(0.0, -0.0, 3.0), dom::Matrix3::identity()))
                .with_property("Color", dom::Color3uint8::new(10, 20, 30))
                .with_property("Tags", dom::Tags::from(vec!["Hazard".to_string()])),
        );
        model.insert(torso, InstanceBuilder::new("Attachment").with_name("Grip").with_property("SomethingUnmodeled", Variant::Int64(7)));
        let emitter = model.insert(torso, InstanceBuilder::new("ParticleEmitter").with_name("Sparks"));
        model.get_by_ref_mut(emitter).unwrap().properties.insert("Size".into(), Variant::NumberSequence(dom::NumberSequence { keypoints: vec![dom::NumberSequenceKeypoint::new(0.0, 1.0, 0.0), dom::NumberSequenceKeypoint::new(1.0, 0.0, 0.5)] }));
        let plan = ModelPlan::new(&reflection, &model, model.root().children());
        let bytes = plan.encode(b"key").expect("encodable");
        let read = ModelPlan::decode(&bytes, b"key").expect("read back");
        assert_eq!(format!("{plan:?}"), format!("{read:?}"));
        assert!(ModelPlan::decode(&bytes, b"other").is_none(), "another key");
        assert!(ModelPlan::decode(&bytes[..bytes.len() - 1], b"key").is_none(), "cut short");
        // a mapped file is read in place, as the same plan
        let file = std::env::temp_dir().join(format!("ree-plan-test-{}.plan", std::process::id()));
        std::fs::write(&file, &bytes).unwrap();
        if let Some(mapping) = Mapping::open(&file) {
            let mapped = ModelPlan::decode_mapped(mapping, b"key").expect("mapped");
            assert_eq!(format!("{plan:?}"), format!("{mapped:?}"));
            assert!(matches!(mapped.nodes, Array::Mapped { .. }) && matches!(mapped.unmodeled.entries, Array::Mapped { .. }));
            let compacted = mapped.compacted();
            assert!(matches!(compacted.props, Array::Mapped { .. }), "kept in place");
            drop(compacted);
        }
        let _ = std::fs::remove_file(&file);
    }
}
