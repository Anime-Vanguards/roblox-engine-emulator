//! Lean non-generic JSON writer for the profiler's `profile.json`, `profile.speedscope.json`,
//! `diff.json`, and `compare.json` outputs.
//!
//! Writes directly into a `Vec<u8>` without building intermediate `serde_json::Value` DOM trees
//! or monomorphizing `serde::Serialize` / `serde_json::Serializer` across dozens of schema
//! structs. String escaping and `f64` formatting delegate to single non-generic `serde_json`
//! helpers so output stays byte-for-byte identical to `serde_json::to_string` (compact) and
//! `serde_json::to_string_pretty` (2-space indent).

use serde_json::Value as Json;

/// Infallible byte-buffer JSON writer supporting both compact and 2-space pretty output.
pub(super) struct JsonWriter {
    buf: Vec<u8>,
    pretty: bool,
    depth: usize,
}

impl JsonWriter {
    pub(super) fn compact() -> Self {
        Self { buf: Vec::new(), pretty: false, depth: 0 }
    }

    pub(super) fn pretty() -> Self {
        Self { buf: Vec::new(), pretty: true, depth: 0 }
    }

    pub(super) fn to_string<T: WriteJson + ?Sized>(value: &T) -> String {
        let mut w = Self::compact();
        value.write_json(&mut w);
        w.finish()
    }

    pub(super) fn to_string_pretty<T: WriteJson + ?Sized>(value: &T) -> String {
        let mut w = Self::pretty();
        value.write_json(&mut w);
        w.finish()
    }

    pub(super) fn finish(self) -> String {
        // SAFETY: Every write appends valid UTF-8 (ASCII tokens, JSON-escaped UTF-8 strings,
        // or UTF-8 output from `serde_json::to_writer`).
        unsafe { String::from_utf8_unchecked(self.buf) }
    }

    fn write_indent(&mut self) {
        self.buf.push(b'\n');
        let spaces = self.depth * 2;
        self.buf.resize(self.buf.len() + spaces, b' ');
    }

    #[inline(never)]
    pub(super) fn null(&mut self) {
        self.buf.extend_from_slice(b"null");
    }

    #[inline(never)]
    pub(super) fn bool(&mut self, v: bool) {
        self.buf.extend_from_slice(if v { b"true" } else { b"false" });
    }

    #[inline(never)]
    pub(super) fn u64(&mut self, mut v: u64) {
        if v == 0 {
            self.buf.push(b'0');
            return;
        }
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        while v > 0 {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        self.buf.extend_from_slice(&tmp[i..]);
    }

    #[inline(never)]
    pub(super) fn i64(&mut self, v: i64) {
        if v < 0 {
            self.buf.push(b'-');
            self.u64(v.unsigned_abs());
        } else {
            self.u64(v as u64);
        }
    }

    #[inline(never)]
    pub(super) fn f64(&mut self, v: f64) {
        let _ = serde_json::to_writer(&mut self.buf, &v);
    }

    #[inline(never)]
    pub(super) fn str(&mut self, s: &str) {
        let _ = serde_json::to_writer(&mut self.buf, s);
    }

    #[inline(never)]
    pub(super) fn json_value(&mut self, v: &Json) {
        match v {
            Json::Null => self.null(),
            Json::Bool(b) => self.bool(*b),
            Json::Number(n) => {
                if let Some(u) = n.as_u64() {
                    self.u64(u);
                } else if let Some(i) = n.as_i64() {
                    self.i64(i);
                } else if let Some(f) = n.as_f64() {
                    self.f64(f);
                } else {
                    self.null();
                }
            }
            Json::String(s) => self.str(s),
            Json::Array(items) => {
                let mut a = self.array();
                for item in items {
                    a.begin_elem();
                    a.w.json_value(item);
                }
                a.end();
            }
            Json::Object(map) => self.json_map(map),
        }
    }

    #[inline(never)]
    pub(super) fn json_map(&mut self, map: &serde_json::Map<String, Json>) {
        let mut o = self.object();
        for (k, v) in map {
            o.begin_field_escaped(k);
            o.w.json_value(v);
        }
        o.end();
    }

    #[inline(never)]
    pub(super) fn object(&mut self) -> ObjectWriter<'_> {
        self.buf.push(b'{');
        self.depth += 1;
        ObjectWriter {
            w: self,
            first: true,
            #[cfg(any(test, debug_assertions))]
            last: String::new(),
        }
    }

    #[inline(never)]
    pub(super) fn array(&mut self) -> ArrayWriter<'_> {
        self.buf.push(b'[');
        self.depth += 1;
        ArrayWriter { w: self, first: true }
    }
}

pub(super) struct ObjectWriter<'a> {
    pub(super) w: &'a mut JsonWriter,
    first: bool,
    /// The key written last. serde_json's maps wrote every object's keys sorted, so test and debug builds check that each
    /// key sorts after the one before it: a field declared out of order, or two of the same name, would change the output.
    #[cfg(any(test, debug_assertions))]
    last: String,
}

impl<'a> ObjectWriter<'a> {
    #[cfg(any(test, debug_assertions))]
    fn sorted_after_last(&mut self, key: &str) {
        assert!(self.first || key > self.last.as_str(), "JSON key {key:?} comes after {:?}: an object's keys go in sorted order", self.last);
        self.last.clear();
        self.last.push_str(key);
    }

    /// Writes a field key known to be plain printable ASCII without quotes or backslashes.
    #[inline(never)]
    pub(super) fn begin_field(&mut self, key: &str) {
        debug_assert!(key.bytes().all(|b| b >= 0x20 && b != b'"' && b != b'\\'));
        #[cfg(any(test, debug_assertions))]
        self.sorted_after_last(key);
        if !self.first {
            self.w.buf.push(b',');
        }
        self.first = false;
        if self.w.pretty {
            self.w.write_indent();
        }
        self.w.buf.push(b'"');
        self.w.buf.extend_from_slice(key.as_bytes());
        self.w.buf.extend_from_slice(if self.w.pretty { b"\": " } else { b"\":" });
    }

    /// Writes a dynamic field key with full JSON string escaping.
    #[inline(never)]
    pub(super) fn begin_field_escaped(&mut self, key: &str) {
        #[cfg(any(test, debug_assertions))]
        self.sorted_after_last(key);
        if !self.first {
            self.w.buf.push(b',');
        }
        self.first = false;
        if self.w.pretty {
            self.w.write_indent();
        }
        self.w.str(key);
        self.w.buf.extend_from_slice(if self.w.pretty { b": " } else { b":" });
    }

    #[inline(always)]
    pub(super) fn field<T: WriteJson + ?Sized>(&mut self, key: &str, value: &T) {
        self.begin_field(key);
        value.write_json(self.w);
    }

    #[inline(never)]
    pub(super) fn end(self) {
        self.w.depth -= 1;
        if self.w.pretty && !self.first {
            self.w.write_indent();
        }
        self.w.buf.push(b'}');
    }
}

pub(super) struct ArrayWriter<'a> {
    pub(super) w: &'a mut JsonWriter,
    first: bool,
}

impl<'a> ArrayWriter<'a> {
    #[inline(never)]
    pub(super) fn begin_elem(&mut self) {
        if !self.first {
            self.w.buf.push(b',');
        }
        self.first = false;
        if self.w.pretty {
            self.w.write_indent();
        }
    }

    #[inline(always)]
    pub(super) fn elem<T: WriteJson + ?Sized>(&mut self, value: &T) {
        self.begin_elem();
        value.write_json(self.w);
    }

    #[inline(never)]
    pub(super) fn end(self) {
        self.w.depth -= 1;
        if self.w.pretty && !self.first {
            self.w.write_indent();
        }
        self.w.buf.push(b']');
    }
}

pub(super) trait WriteJson {
    fn write_json(&self, w: &mut JsonWriter);
}

impl<T: WriteJson + ?Sized> WriteJson for &T {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        (**self).write_json(w);
    }
}

impl WriteJson for bool {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.bool(*self);
    }
}

impl WriteJson for u8 {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.u64(u64::from(*self));
    }
}

impl WriteJson for u32 {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.u64(u64::from(*self));
    }
}

impl WriteJson for u64 {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.u64(*self);
    }
}

impl WriteJson for usize {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.u64(*self as u64);
    }
}

impl WriteJson for i32 {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.i64(i64::from(*self));
    }
}

impl WriteJson for i64 {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.i64(*self);
    }
}

impl WriteJson for f64 {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.f64(*self);
    }
}

impl WriteJson for str {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.str(self);
    }
}

impl WriteJson for String {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.str(self.as_str());
    }
}

impl WriteJson for Json {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        w.json_value(self);
    }
}

impl<T: WriteJson> WriteJson for Option<T> {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        match self {
            Some(v) => v.write_json(w),
            None => w.null(),
        }
    }
}

impl<T: WriteJson> WriteJson for [T] {
    #[inline(never)]
    fn write_json(&self, w: &mut JsonWriter) {
        let mut a = w.array();
        for item in self {
            a.begin_elem();
            item.write_json(a.w);
        }
        a.end();
    }
}

impl<T: WriteJson> WriteJson for Vec<T> {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        self.as_slice().write_json(w);
    }
}

impl<T: WriteJson, const N: usize> WriteJson for [T; N] {
    #[inline(always)]
    fn write_json(&self, w: &mut JsonWriter) {
        self.as_slice().write_json(w);
    }
}

impl<A: WriteJson, B: WriteJson> WriteJson for (A, B) {
    #[inline(never)]
    fn write_json(&self, w: &mut JsonWriter) {
        let mut a = w.array();
        a.elem(&self.0);
        a.elem(&self.1);
        a.end();
    }
}

impl<A: WriteJson, B: WriteJson, C: WriteJson> WriteJson for (A, B, C) {
    #[inline(never)]
    fn write_json(&self, w: &mut JsonWriter) {
        let mut a = w.array();
        a.elem(&self.0);
        a.elem(&self.1);
        a.elem(&self.2);
        a.end();
    }
}

/// Sorted key-value list serialized as a JSON object with keys in ascending order.
/// Avoids monomorphizing `BTreeMap` across one-shot schema maps.
#[derive(Clone, Debug)]
pub(super) struct SortedMap<K, V>(pub(super) Vec<(K, V)>);

impl<K, V> Default for SortedMap<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: Ord, V> SortedMap<K, V> {
    pub(super) fn new() -> Self {
        Self(Vec::new())
    }

    pub(super) fn from_unsorted(mut items: Vec<(K, V)>) -> Self {
        if items.len() > 1 {
            crate::sort::by(&mut items, |a, b| a.0.cmp(&b.0));
            let mut write = 0usize;
            for read in 0..items.len() {
                if write > 0 && items[write - 1].0 == items[read].0 {
                    items.swap(write - 1, read);
                } else {
                    if write != read {
                        items.swap(write, read);
                    }
                    write += 1;
                }
            }
            items.truncate(write);
        }
        Self(items)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn insert(&mut self, key: K, value: V) {
        match self.0.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(idx) => self.0[idx].1 = value,
            Err(idx) => self.0.insert(idx, (key, value)),
        }
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, (K, V)> {
        self.0.iter()
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for SortedMap<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self::from_unsorted(iter.into_iter().collect())
    }
}

impl<K: Ord, V, const N: usize> From<[(K, V); N]> for SortedMap<K, V> {
    fn from(items: [(K, V); N]) -> Self {
        Self::from_unsorted(items.into_iter().collect())
    }
}

impl<K: AsRef<str>, V: WriteJson> WriteJson for SortedMap<K, V> {
    #[inline(never)]
    fn write_json(&self, w: &mut JsonWriter) {
        let mut o = w.object();
        for (k, v) in &self.0 {
            o.begin_field_escaped(k.as_ref());
            v.write_json(o.w);
        }
        o.end();
    }
}

/// Defines a JSON-serializable struct and implements [`WriteJson`] with `#[inline(never)]`.
/// Fields are serialized in declaration order (must be strict ASCII ascending to match `BTreeMap`).
/// Supports `#[skip_none]` (omits `Option` fields when `None`) and `#[rename = "..."]` per field.
macro_rules! json_struct_acc {
    (@def [$($smeta:tt)*] [$vis:vis] [$name:ident] [$($lt:lifetime)?] [$($fields:tt)*]) => {
        $($smeta)*
        $vis struct $name $(<$lt>)? {
            $($fields)*
        }
    };
    (@def [$($smeta:tt)*] [$vis:vis] [$name:ident] [$($lt:lifetime)?] [$($fields:tt)*]
        #[skip_none] #[rename = $key:literal] $field:ident : $ty:ty $(, $($rest:tt)*)?
    ) => {
        $crate::profiler::json_writer::json_struct_acc!(
            @def [$($smeta)*] [$vis] [$name] [$($lt)?] [$($fields)* $field : $ty,] $($($rest)*)?
        );
    };
    (@def [$($smeta:tt)*] [$vis:vis] [$name:ident] [$($lt:lifetime)?] [$($fields:tt)*]
        #[rename = $key:literal] #[skip_none] $field:ident : $ty:ty $(, $($rest:tt)*)?
    ) => {
        $crate::profiler::json_writer::json_struct_acc!(
            @def [$($smeta)*] [$vis] [$name] [$($lt)?] [$($fields)* $field : $ty,] $($($rest)*)?
        );
    };
    (@def [$($smeta:tt)*] [$vis:vis] [$name:ident] [$($lt:lifetime)?] [$($fields:tt)*]
        #[skip_none] $field:ident : $ty:ty $(, $($rest:tt)*)?
    ) => {
        $crate::profiler::json_writer::json_struct_acc!(
            @def [$($smeta)*] [$vis] [$name] [$($lt)?] [$($fields)* $field : $ty,] $($($rest)*)?
        );
    };
    (@def [$($smeta:tt)*] [$vis:vis] [$name:ident] [$($lt:lifetime)?] [$($fields:tt)*]
        #[rename = $key:literal] $field:ident : $ty:ty $(, $($rest:tt)*)?
    ) => {
        $crate::profiler::json_writer::json_struct_acc!(
            @def [$($smeta)*] [$vis] [$name] [$($lt)?] [$($fields)* $field : $ty,] $($($rest)*)?
        );
    };
    (@def [$($smeta:tt)*] [$vis:vis] [$name:ident] [$($lt:lifetime)?] [$($fields:tt)*]
        $field:ident : $ty:ty $(, $($rest:tt)*)?
    ) => {
        $crate::profiler::json_writer::json_struct_acc!(
            @def [$($smeta)*] [$vis] [$name] [$($lt)?] [$($fields)* $field : $ty,] $($($rest)*)?
        );
    };

    (@write $self:ident, $o:ident,) => {};
    (@write $self:ident, $o:ident, #[skip_none] #[rename = $key:literal] $field:ident : $ty:ty $(, $($rest:tt)*)?) => {
        if let Some(v) = &$self.$field {
            $o.field($key, v);
        }
        $crate::profiler::json_writer::json_struct_acc!(@write $self, $o, $($($rest)*)?);
    };
    (@write $self:ident, $o:ident, #[rename = $key:literal] #[skip_none] $field:ident : $ty:ty $(, $($rest:tt)*)?) => {
        if let Some(v) = &$self.$field {
            $o.field($key, v);
        }
        $crate::profiler::json_writer::json_struct_acc!(@write $self, $o, $($($rest)*)?);
    };
    (@write $self:ident, $o:ident, #[skip_none] $field:ident : $ty:ty $(, $($rest:tt)*)?) => {
        if let Some(v) = &$self.$field {
            $o.field(stringify!($field), v);
        }
        $crate::profiler::json_writer::json_struct_acc!(@write $self, $o, $($($rest)*)?);
    };
    (@write $self:ident, $o:ident, #[rename = $key:literal] $field:ident : $ty:ty $(, $($rest:tt)*)?) => {
        $o.field($key, &$self.$field);
        $crate::profiler::json_writer::json_struct_acc!(@write $self, $o, $($($rest)*)?);
    };
    (@write $self:ident, $o:ident, $field:ident : $ty:ty $(, $($rest:tt)*)?) => {
        $o.field(stringify!($field), &$self.$field);
        $crate::profiler::json_writer::json_struct_acc!(@write $self, $o, $($($rest)*)?);
    };
}

macro_rules! json_struct {
    ($(#[$smeta:meta])* $vis:vis struct $name:ident $(<$lt:lifetime>)? { $($body:tt)* }) => {
        $crate::profiler::json_writer::json_struct_acc!(
            @def [$(#[$smeta])*] [$vis] [$name] [$($lt)?] [] $($body)*
        );
        impl $(<$lt>)? $crate::profiler::json_writer::WriteJson for $name $(<$lt>)? {
            #[inline(never)]
            fn write_json(&self, w: &mut $crate::profiler::json_writer::JsonWriter) {
                #[allow(unused_mut)]
                let mut o = w.object();
                $crate::profiler::json_writer::json_struct_acc!(@write self, o, $($body)*);
                o.end();
            }
        }
    };
}

pub(super) use json_struct;
pub(super) use json_struct_acc;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    json_struct! {
        struct Sample<'a> {
            a_bool: bool,
            b_empty_arr: Vec<u64>,
            c_empty_map: SortedMap<String, u64>,
            d_floats: Vec<f64>,
            e_ints: Vec<i64>,
            f_map: SortedMap<String, u64>,
            #[skip_none]
            g_absent: Option<u64>,
            #[skip_none]
            h_null_inner: Option<Option<u64>>,
            i_str: &'a str,
            #[rename = "type"]
            ty: &'a str,
        }
    }

    #[test]
    fn compact_and_pretty_match_serde_json_byte_for_byte() {
        let sample = Sample {
            a_bool: true,
            b_empty_arr: Vec::new(),
            c_empty_map: SortedMap::new(),
            d_floats: vec![0.0, -0.0, 1.5, f64::NAN, f64::INFINITY],
            e_ints: vec![0, 1, -1, i64::MIN, i64::MAX],
            f_map: SortedMap::from([("a\"b\\c\n".to_string(), 42), ("z".to_string(), 0)]),
            g_absent: None,
            h_null_inner: Some(None),
            i_str: "hello \"world\"\n\t\x08\u{0001}😀",
            ty: "table",
        };
        let expected = json!({
            "a_bool": true,
            "b_empty_arr": [],
            "c_empty_map": {},
            "d_floats": [0.0, -0.0, 1.5, null, null],
            "e_ints": [0, 1, -1, i64::MIN, i64::MAX],
            "f_map": {"a\"b\\c\n": 42, "z": 0},
            "h_null_inner": null,
            "i_str": "hello \"world\"\n\t\x08\u{0001}😀",
            "type": "table",
        });
        assert_eq!(JsonWriter::to_string(&sample), serde_json::to_string(&expected).unwrap());
        assert_eq!(JsonWriter::to_string_pretty(&sample), serde_json::to_string_pretty(&expected).unwrap());
    }

    json_struct! {
        struct OutOfOrder {
            b: u64,
            a: u64,
        }
    }

    json_struct! {
        struct RenamedOntoAnother {
            kind: u64,
            #[rename = "kind"]
            ty: u64,
        }
    }

    /// A field declared out of order, or renamed onto another's name, writes keys a sorted map never would: the
    /// writer refuses it in tests and debug builds.
    #[test]
    fn keys_out_of_order_are_refused() {
        for written in [
            std::panic::catch_unwind(|| JsonWriter::to_string(&OutOfOrder { b: 1, a: 2 })),
            std::panic::catch_unwind(|| JsonWriter::to_string(&RenamedOntoAnother { kind: 1, ty: 2 })),
        ] {
            let problem = written.expect_err("written unsorted");
            let text = problem.downcast_ref::<String>().map(String::as_str).unwrap_or_default();
            assert!(text.contains("an object's keys go in sorted order"), "{text}");
        }
    }
}
