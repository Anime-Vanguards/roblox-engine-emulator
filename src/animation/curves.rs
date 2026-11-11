//! The curve instances' members: FloatCurve, RotationCurve, MarkerCurve, Vector3Curve,
//! EulerRotationCurve and CurveAnimation.Length. A curve keeps its keys in its `ValuesAndTimes`
//! property in the form files hold them ([`super::curve`]), so a clone and a model file carry them;
//! scripts cannot read the property ("ValuesAndTimes is not a valid member of FloatCurve"). Measured
//! in Studio 0.747 (2026-11-11; docs/engine.md "Animation curves"):
//!
//! - `Length` is the number of keys, read only. No member fires Changed.
//! - GetKeyAtIndex and GetMarkerAtIndex count from 1 and raise "FloatCurve index is out of range";
//!   an index rounds to the nearest whole number (1.5 is 2, 0.4 is 0). The keys they and GetKeys
//!   return are copies. RotationCurve:GetKeyAtIndex of one past the last key gives the last key
//!   (Studio 0.747 does). GetValueAtTime of a curve without keys is nil.
//! - InsertKey and InsertMarkerAtTime return `{true, index}` (from 0) for a new key and
//!   `{false, index}` when one of that time was replaced. SetKeys sorts the keys by time, drops each
//!   that comes less than 1/2400 s (in float32) after the one kept before it and returns how many it
//!   kept: 201 keys a tick apart from 0 s leave 113. RotationCurve:SetKeys raises "Variant cast
//!   failed" for any key (Studio 0.747 does) and clears the curve for an empty table.
//!   RemoveKeyAtIndex(start, count = 1) returns how many it removed, 0 for a start past the end;
//!   a start below 1 or a negative count raises "Invalid argument provided: startingIndex." /
//!   "count.".
//! - A time may be a number, a string of one or a boolean; anything else raises "Unable to cast
//!   Array to float" (an index: "to int"), a missing one "Argument 1 missing or nil".
//! - Vector3Curve and EulerRotationCurve: `X()`, `Y()`, `Z()` return the FloatCurve child of that
//!   name and make one when there is none. GetValueAtTime / GetAnglesAtTime return `{x, y, z}` with
//!   nil for a channel that is missing or has no keys (and make no child);
//!   GetRotationAtTime is `CFrame.fromEulerAngles` of the three, 0 for a missing one.
//!
//! Not emulated: Studio remembers a channel by the name the child had when it was parented (a
//! FloatCurve renamed afterwards stays the channel, one named after it was parented is not found);
//! here a channel is the first FloatCurve child that has the name now. What a marker's value that is
//! no string or number becomes is not measured (empty here).

use std::rc::Rc;

use glam::Affine3A;
use mlua::prelude::*;

use super::clip::EngineTree;
use super::curve::{self, FloatKey, MarkerKey, RotationKey, Timed};
use crate::{
    Engine, InstanceId, NameId,
    instance as core,
    lua::{
        Args, Flow, convert,
        curve_keys::{self, LFloatCurveKey, LRotationCurveKey},
        instance_value, raise, typeof_value, values,
    },
    services::{Behaviors, getter, method},
    value::{Str, Value},
};

/// A script value's name in "Unable to cast X to Y".
fn cast_name(engine: &Engine, value: &LuaValue) -> String {
    match value {
        LuaValue::Table(table) => if table.raw_len() > 0 || table.is_empty() { "Array" } else { "Dictionary" }.into(),
        other => match typeof_value(engine, other).as_str() {
            "number" => "double".into(),
            "boolean" => "bool".into(),
            other => other.into(),
        },
    }
}

/// Argument `index` (1 is the first after self) as a number of kind `kind` ("float", "int").
fn number_arg(lua: &Lua, engine: &Engine, args: &Args, index: usize, kind: &str) -> Result<f64, Flow> {
    let value = args.get(index);
    match value {
        LuaValue::Nil => raise(format!("Argument {index} missing or nil")),
        LuaValue::Number(n) => Ok(*n),
        LuaValue::Integer(n) => Ok(*n as f64),
        LuaValue::Boolean(flag) => Ok(*flag as u8 as f64),
        LuaValue::String(_) => match lua.coerce_number(value.clone()).ok().flatten() {
            Some(number) => Ok(number),
            None => raise(format!("Unable to cast string to {kind}")),
        },
        other => raise(format!("Unable to cast {} to {kind}", cast_name(engine, other))),
    }
}

fn time_arg(lua: &Lua, engine: &Engine, args: &Args, index: usize) -> Result<f32, Flow> {
    number_arg(lua, engine, args, index, "float").map(|time| time as f32)
}

fn index_arg(lua: &Lua, engine: &Engine, args: &Args, index: usize) -> Result<i64, Flow> {
    // (measured: 1.5 is index 2 and 0.4 index 0)
    number_arg(lua, engine, args, index, "int").map(|number| if number.is_finite() { number.round() as i64 } else { 0 })
}

/// RemoveKeyAtIndex's range: the start (from 1) and the count (1 when not given).
fn range_args(lua: &Lua, engine: &Engine, args: &Args) -> Result<(usize, usize), Flow> {
    let start = index_arg(lua, engine, args, 1)?;
    let count = if args.get(2).is_nil() { 1 } else { index_arg(lua, engine, args, 2)? };
    if start < 1 {
        return raise("Invalid argument provided: startingIndex.");
    }
    if count < 0 {
        return raise("Invalid argument provided: count.");
    }
    Ok((start as usize, count as usize))
}

fn stored_bytes(engine: &Engine, id: InstanceId, property: NameId) -> Vec<u8> {
    engine.state.borrow().dom.raw_get(id, property).and_then(Value::as_str).map_or_else(Vec::new, |bytes| bytes.as_bytes().to_vec())
}

fn store(engine: &Engine, id: InstanceId, property: NameId, bytes: Vec<u8>) {
    core::raw_set_silent(&mut engine.state.borrow_mut(), &engine.reflection, id, property, Value::String(Str::new(&bytes)));
}

/// How many keys a curve's stored form holds.
fn stored_count(bytes: &[u8]) -> f64 {
    bytes.get(4..8).map_or(0.0, |count| u32::from_le_bytes(count.try_into().expect("four bytes")) as f64)
}

fn pair(lua: &Lua, first: LuaValue, second: LuaValue) -> LuaResult<LuaMultiValue> {
    let table = lua.create_table()?;
    table.raw_push(first)?;
    table.raw_push(second)?;
    Ok(values([LuaValue::Table(table)]))
}

fn inserted(lua: &Lua, (new, index): (bool, usize)) -> LuaResult<LuaMultiValue> {
    pair(lua, LuaValue::Boolean(new), LuaValue::Number(index as f64))
}

fn indices<K: Timed>(lua: &Lua, keys: &[K], time: f32) -> LuaResult<LuaMultiValue> {
    let (before, after) = curve::indices_at(keys, time);
    pair(lua, LuaValue::Number(before as f64), LuaValue::Number(after as f64))
}

/// The key at script index `index`, else the class's out-of-range error.
fn key_at<'a, K>(keys: &'a [K], index: i64, class: &str) -> Result<&'a K, Flow> {
    match usize::try_from(index).ok().filter(|&index| index >= 1).and_then(|index| keys.get(index - 1)) {
        Some(key) => Ok(key),
        None => raise(format!("{class} index is out of range")),
    }
}

/// SetKeys' argument: an array.
fn key_table<'a>(engine: &Engine, args: &'a Args) -> Result<&'a LuaTable, Flow> {
    match args.get(1) {
        LuaValue::Nil => raise("Argument 1 missing or nil"),
        LuaValue::Table(table) if table.raw_len() == 0 && !table.is_empty() => raise("Unable to cast Dictionary to Array"),
        LuaValue::Table(table) => Ok(table),
        other => raise(format!("Unable to cast {} to Array", cast_name(engine, other))),
    }
}

/// The X, Y or Z FloatCurve of a Vector3Curve or EulerRotationCurve: its first FloatCurve child of
/// that name.
fn channel(engine: &Engine, id: InstanceId, axis: &str) -> Option<InstanceId> {
    let state = engine.state.borrow();
    let float_curve = engine.reflection.class_id("FloatCurve")?;
    state.dom.children_of(id).iter().copied().find(|&child| {
        let instance = state.dom.at(child);
        instance.class == float_curve && instance.name.as_bytes() == axis.as_bytes()
    })
}

/// The three channels' values at `time`: None for one that is missing or has no keys.
fn channel_values(engine: &Engine, id: InstanceId, property: NameId, time: f32) -> [Option<f32>; 3] {
    ["X", "Y", "Z"].map(|axis| {
        let keys = curve::decode_floats(&stored_bytes(engine, channel(engine, id, axis)?, property));
        curve::float_at(&keys, time)
    })
}

/// `{x, y, z}` with holes.
fn triple(lua: &Lua, found: [Option<f32>; 3]) -> LuaResult<LuaMultiValue> {
    let table = lua.create_table()?;
    for (index, value) in found.into_iter().enumerate() {
        if let Some(value) = value {
            table.raw_set(index + 1, value as f64)?;
        }
    }
    Ok(values([LuaValue::Table(table)]))
}

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let (Some(stored), Some(length)) = (reflection.name_id("ValuesAndTimes"), reflection.name_id("Length")) else { return };
    let add = |behaviors: &mut Behaviors, class: &str, name: &str, function: crate::services::MethodFn| {
        if let (Some(class), Some(name)) = (reflection.class_id(class), reflection.name_id(name)) {
            behaviors.add_method(class, name, function);
        }
    };

    for class in ["FloatCurve", "RotationCurve", "MarkerCurve"] {
        if let Some(class) = reflection.class_id(class) {
            behaviors.add_getter(class, length, getter(move |_, engine, id| Ok(LuaValue::Number(stored_count(&stored_bytes(engine, id, stored))))));
        }
    }
    if let Some(class) = reflection.class_id("CurveAnimation") {
        behaviors.add_getter(class, length, getter(|_, engine, id| {
            let state = engine.state.borrow();
            let tree = EngineTree { state: &state, reflection: &engine.reflection };
            Ok(LuaValue::Number(super::curve_clip::length(&tree, id) as f64))
        }));
    }

    // FloatCurve
    let floats = move |engine: &Engine, id: InstanceId| curve::decode_floats(&stored_bytes(engine, id, stored));
    add(behaviors, "FloatCurve", "GetKeyAtIndex", method(move |lua, engine, id, args| {
        let index = index_arg(lua, engine, args, 1)?;
        let keys = floats(engine, id);
        Ok(values([curve_keys::float_key_value(lua, key_at(&keys, index, "FloatCurve")?)?]))
    }));
    add(behaviors, "FloatCurve", "GetKeys", method(move |lua, engine, id, _| {
        let table = lua.create_table()?;
        for key in floats(engine, id) {
            table.raw_push(curve_keys::float_key_value(lua, &key)?)?;
        }
        Ok(values([LuaValue::Table(table)]))
    }));
    add(behaviors, "FloatCurve", "GetKeyIndicesAtTime", method(move |lua, engine, id, args| {
        let time = time_arg(lua, engine, args, 1)?;
        Ok(indices(lua, &floats(engine, id), time)?)
    }));
    add(behaviors, "FloatCurve", "GetValueAtTime", method(move |lua, engine, id, args| {
        let time = time_arg(lua, engine, args, 1)?;
        Ok(values([curve::float_at(&floats(engine, id), time).map_or(LuaValue::Nil, |value| LuaValue::Number(value as f64))]))
    }));
    let float_key = |engine: &Engine, value: &LuaValue| -> Result<(f32, FloatKey), Flow> {
        match value {
            LuaValue::UserData(ud) if ud.is::<LFloatCurveKey>() => Ok(ud.borrow::<LFloatCurveKey>()?.stored()),
            // (measured: a RotationCurveKey is "null" here, a FloatCurveKey keeps its name for a RotationCurve)
            LuaValue::UserData(ud) if ud.is::<LRotationCurveKey>() => raise("Unable to cast null to FloatCurveKey"),
            other => raise(format!("Unable to cast {} to FloatCurveKey", cast_name(engine, other))),
        }
    };
    add(behaviors, "FloatCurve", "InsertKey", method(move |lua, engine, id, args| {
        if args.get(1).is_nil() {
            return raise("Argument 1 missing or nil");
        }
        let (_, key) = float_key(engine, args.get(1))?;
        let mut keys = floats(engine, id);
        let outcome = curve::insert(&mut keys, key);
        store(engine, id, stored, curve::encode_floats(&keys));
        Ok(inserted(lua, outcome)?)
    }));
    add(behaviors, "FloatCurve", "RemoveKeyAtIndex", method(move |lua, engine, id, args| {
        let (start, count) = range_args(lua, engine, args)?;
        let mut keys = floats(engine, id);
        let removed = curve::remove(&mut keys, start, count);
        store(engine, id, stored, curve::encode_floats(&keys));
        Ok(values([LuaValue::Number(removed as f64)]))
    }));
    add(behaviors, "FloatCurve", "SetKeys", method(move |_, engine, id, args| {
        let table = key_table(engine, args)?;
        let mut given = Vec::with_capacity(table.raw_len());
        for value in table.sequence_values::<LuaValue>() {
            match value? {
                LuaValue::UserData(ud) if ud.is::<LFloatCurveKey>() => given.push(ud.borrow::<LFloatCurveKey>()?.stored()),
                _ => return raise("Variant cast failed"),
            }
        }
        let keys = curve::set(given);
        store(engine, id, stored, curve::encode_floats(&keys));
        Ok(values([LuaValue::Number(keys.len() as f64)]))
    }));

    // RotationCurve
    let rotations = move |engine: &Engine, id: InstanceId| curve::decode_rotations(&stored_bytes(engine, id, stored));
    add(behaviors, "RotationCurve", "GetKeyAtIndex", method(move |lua, engine, id, args| {
        let index = index_arg(lua, engine, args, 1)?;
        let keys = rotations(engine, id);
        // (measured: the index one past the last key gives the last key; of two keys, 3 does)
        let index = if index >= 2 && index as usize == keys.len() + 1 { index - 1 } else { index };
        Ok(values([curve_keys::rotation_key_value(lua, key_at(&keys, index, "RotationCurve")?)?]))
    }));
    add(behaviors, "RotationCurve", "GetKeys", method(move |lua, engine, id, _| {
        let table = lua.create_table()?;
        for key in rotations(engine, id) {
            table.raw_push(curve_keys::rotation_key_value(lua, &key)?)?;
        }
        Ok(values([LuaValue::Table(table)]))
    }));
    add(behaviors, "RotationCurve", "GetKeyIndicesAtTime", method(move |lua, engine, id, args| {
        let time = time_arg(lua, engine, args, 1)?;
        Ok(indices(lua, &rotations(engine, id), time)?)
    }));
    add(behaviors, "RotationCurve", "GetValueAtTime", method(move |lua, engine, id, args| {
        let time = time_arg(lua, engine, args, 1)?;
        match curve::rotation_at(&rotations(engine, id), time) {
            Some(rotation) => Ok(values([curve_keys::rotation_value(lua, engine, rotation)?])),
            None => Ok(values([LuaValue::Nil])),
        }
    }));
    add(behaviors, "RotationCurve", "InsertKey", method(move |lua, engine, id, args| {
        let (_, key): (f32, RotationKey) = match args.get(1) {
            LuaValue::Nil => return raise("Argument 1 missing or nil"),
            LuaValue::UserData(ud) if ud.is::<LRotationCurveKey>() => ud.borrow::<LRotationCurveKey>()?.stored(),
            other => return raise(format!("Unable to cast {} to RotationCurveKey", cast_name(engine, other))),
        };
        let mut keys = rotations(engine, id);
        let outcome = curve::insert(&mut keys, key);
        store(engine, id, stored, curve::encode_rotations(&keys));
        Ok(inserted(lua, outcome)?)
    }));
    add(behaviors, "RotationCurve", "RemoveKeyAtIndex", method(move |lua, engine, id, args| {
        let (start, count) = range_args(lua, engine, args)?;
        let mut keys = rotations(engine, id);
        let removed = curve::remove(&mut keys, start, count);
        store(engine, id, stored, curve::encode_rotations(&keys));
        Ok(values([LuaValue::Number(removed as f64)]))
    }));
    add(behaviors, "RotationCurve", "SetKeys", method(move |_, engine, id, args| {
        // Measured: any key in the table fails, RotationCurveKeys included; an empty table clears the curve
        if key_table(engine, args)?.raw_len() > 0 {
            return raise("Variant cast failed");
        }
        store(engine, id, stored, curve::encode_rotations(&[]));
        Ok(values([LuaValue::Number(0.0)]))
    }));

    // MarkerCurve
    let markers = move |engine: &Engine, id: InstanceId| curve::decode_markers(&stored_bytes(engine, id, stored));
    fn marker_value(lua: &Lua, marker: &MarkerKey) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        table.raw_set("Time", curve::time_of(marker.tick) as f64)?;
        table.raw_set("Value", lua.create_string(&marker.value)?)?;
        Ok(LuaValue::Table(table))
    }
    add(behaviors, "MarkerCurve", "GetMarkerAtIndex", method(move |lua, engine, id, args| {
        let index = index_arg(lua, engine, args, 1)?;
        let found = markers(engine, id);
        Ok(values([marker_value(lua, key_at(&found, index, "MarkerCurve")?)?]))
    }));
    add(behaviors, "MarkerCurve", "GetMarkers", method(move |lua, engine, id, _| {
        let table = lua.create_table()?;
        for marker in markers(engine, id) {
            table.raw_push(marker_value(lua, &marker)?)?;
        }
        Ok(values([LuaValue::Table(table)]))
    }));
    add(behaviors, "MarkerCurve", "InsertMarkerAtTime", method(move |lua, engine, id, args| {
        if args.get(2).is_nil() {
            return raise("Argument 2 missing or nil");
        }
        let time = time_arg(lua, engine, args, 1)?;
        let value = match args.get(2) {
            LuaValue::String(text) => text.as_bytes().to_vec(),
            number @ (LuaValue::Number(_) | LuaValue::Integer(_)) => lua.coerce_string(number.clone())?.map_or_else(Vec::new, |text| text.as_bytes().to_vec()),
            _ => Vec::new(),
        };
        let mut found = markers(engine, id);
        let outcome = curve::insert(&mut found, MarkerKey { tick: curve::tick_of(time), value });
        store(engine, id, stored, curve::encode_markers(&found));
        Ok(inserted(lua, outcome)?)
    }));
    add(behaviors, "MarkerCurve", "RemoveMarkerAtIndex", method(move |lua, engine, id, args| {
        let (start, count) = range_args(lua, engine, args)?;
        let mut found = markers(engine, id);
        let removed = curve::remove(&mut found, start, count);
        store(engine, id, stored, curve::encode_markers(&found));
        Ok(values([LuaValue::Number(removed as f64)]))
    }));

    // Vector3Curve and EulerRotationCurve
    for class in ["Vector3Curve", "EulerRotationCurve"] {
        for axis in ["X", "Y", "Z"] {
            add(behaviors, class, axis, method(move |lua, engine, id, _| {
                let found = match channel(engine, id, axis) {
                    Some(found) => found,
                    None => {
                        let hooks = engine.hooks.borrow();
                        let mut state = engine.state.borrow_mut();
                        let made = core::new_instance(&mut state, &engine.reflection, &hooks, "FloatCurve", None).map_err(Flow::Error)?;
                        core::raw_set_silent(&mut state, &engine.reflection, made, crate::K::Name as NameId, Value::String(Str::from(axis)));
                        core::set_parent(&mut state, &engine.reflection, &hooks, made, Some(id)).map_err(Flow::Error)?;
                        made
                    }
                };
                Ok(values([instance_value(lua, engine, found)?]))
            }));
        }
    }
    add(behaviors, "Vector3Curve", "GetValueAtTime", method(move |lua, engine, id, args| {
        let time = time_arg(lua, engine, args, 1)?;
        Ok(triple(lua, channel_values(engine, id, stored, time))?)
    }));
    add(behaviors, "EulerRotationCurve", "GetAnglesAtTime", method(move |lua, engine, id, args| {
        let time = time_arg(lua, engine, args, 1)?;
        Ok(triple(lua, channel_values(engine, id, stored, time))?)
    }));
    let rotation_order = reflection.name_id("RotationOrder");
    add(behaviors, "EulerRotationCurve", "GetRotationAtTime", method(move |lua, engine, id, args| {
        let time = time_arg(lua, engine, args, 1)?;
        let [x, y, z] = channel_values(engine, id, stored, time).map(|angle| angle.unwrap_or(0.0));
        let order = {
            let state = engine.state.borrow();
            let item = rotation_order.and_then(|name| state.dom.raw_get(id, name)).and_then(Value::as_enum);
            item.map_or(0, |item| engine.reflection.enum_item_info(item).value as u32)
        };
        let rotation = Affine3A::from_mat3(curve::euler_matrix(x, y, z, order));
        Ok(values([convert::to_lua(lua, engine, &crate::world::frame::cframe_value(&rotation))?]))
    }));
}
