//! Conversions between engine [`Value`]s and Luau values, and property coercion (port of
//! instance.luau Coerce). Lune's datatype userdata are the script representation of rbx-dom
//! datatypes, as in the Luau runtime.

use std::{any::TypeId, rc::Rc};

use lune_roblox::datatypes::{create as lune_create, types as lune};
use mlua::prelude::*;
use rbx_dom_weak::types::{self as dom, Variant};

use super::{
    Flow, enum_item_value, instance_value, raw::instance_of_ud, typeof_value,
    types::{LContent, LEnumItem, LFont},
};
use crate::{
    Engine, ScriptError,
    reflection::{EngineType, PropertyInfo, ScriptType},
    value::{ContentValue, Str, Value},
};

/// An engine value as scripts see it (instances and enum items keep their identity; Font values
/// are fresh copies, as CopyIfMutable returns).
pub fn to_lua(lua: &Lua, engine: &Engine, value: &Value) -> LuaResult<LuaValue> {
    Ok(match value {
        Value::Nil => LuaValue::Nil,
        Value::Bool(b) => LuaValue::Boolean(*b),
        Value::Number(n) => LuaValue::Number(*n),
        Value::String(s) => LuaValue::String(lua.create_string(s.as_bytes())?),
        Value::EnumItem(item) => enum_item_value(lua, engine, *item)?,
        Value::Instance(id) => {
            if engine.state.borrow().dom.get(*id).is_some() {
                instance_value(lua, engine, *id)?
            } else {
                LuaValue::Nil
            }
        }
        // (only an attribute holds one: a handle of none)
        Value::NoInstance => super::types::instance_handle(lua, engine, None)?,
        Value::UDim(v) => LuaValue::UserData(lune_create::udim(lua, lune::UDim::from(*v))?),
        Value::UDim2(v) => LuaValue::UserData(lune_create::udim2(lua, lune::UDim2::from(*v))?),
        Value::Vector2(v) => LuaValue::UserData(lune_create::vector2(lua, lune::Vector2::from(*v))?),
        // (the native vector type, as in Roblox: prelude.luau "Vector3")
        Value::Vector3(v) => LuaValue::Vector(mlua::Vector::new(v.x, v.y, v.z)),
        Value::Color3(v) => LuaValue::UserData(lune_create::color3(lua, lune::Color3::from(*v))?),
        Value::Rect(v) => LuaValue::UserData(lune_create::rect(lua, lune::Rect::from(*v))?),
        Value::NumberRange(v) => LuaValue::UserData(lune_create::number_range(lua, lune::NumberRange::from(*v))?),
        Value::NumberSequence(v) => {
            LuaValue::UserData(lune_create::number_sequence(lua, lune::NumberSequence::from((**v).clone()))?)
        }
        Value::ColorSequence(v) => {
            LuaValue::UserData(lune_create::color_sequence(lua, lune::ColorSequence::from((**v).clone()))?)
        }
        Value::Font(font) => LuaValue::UserData(lua.create_any_userdata(LFont(std::cell::RefCell::new((**font).clone())))?),
        Value::Content(content) => LuaValue::UserData(lua.create_any_userdata(LContent::new(lua, (**content).clone()))?),
        Value::Dom(variant) => dom_to_lua(lua, variant)?,
        Value::Lua(value) => {
            // (a table or buffer BindableEvent:Fire sent: every receiver gets a copy of its own, lua/bindable.rs)
            if let LuaValue::UserData(userdata) = &**value {
                if let Ok(copied) = userdata.borrow::<super::bindable::Copied>() {
                    return super::bindable::rebuild(lua, &copied.0);
                }
                // (a loopback run's received table or buffer: each handler rebuilds its own, loopback/copier.rs)
                if let Ok(delivered) = userdata.borrow::<crate::loopback::wire::Delivered>() {
                    return match &*delivered.0 {
                        crate::loopback::wire::Wire::Buffer(bytes) => lua.create_buffer(&bytes[..]).map(LuaValue::Buffer),
                        wire => crate::loopback::copier::rebuild_table(lua, engine, wire, &|id| Some(id)),
                    };
                }
            }
            (**value).clone()
        }
    })
}

/// Other rbx-dom datatypes as Lune userdata.
pub fn dom_to_lua(lua: &Lua, variant: &Variant) -> LuaResult<LuaValue> {
    Ok(match variant {
        Variant::Axes(v) => LuaValue::UserData(lune_create::axes(lua, lune::Axes::from(*v))?),
        Variant::BrickColor(v) => LuaValue::UserData(lune_create::brick_color(lua, lune::BrickColor::from(*v))?),
        Variant::CFrame(v) => LuaValue::UserData(lune_create::cframe(lua, lune::CFrame::from(*v))?),
        Variant::Faces(v) => LuaValue::UserData(lune_create::faces(lua, lune::Faces::from(*v))?),
        Variant::Ray(v) => LuaValue::UserData(lune_create::ray(lua, lune::Ray::from(*v))?),
        Variant::Region3(v) => LuaValue::UserData(lune_create::region3(lua, lune::Region3::from(*v))?),
        Variant::Region3int16(v) => LuaValue::UserData(lune_create::region3int16(lua, lune::Region3int16::from(*v))?),
        Variant::UniqueId(v) => LuaValue::UserData(lune_create::unique_id(lua, lune::UniqueId::from(*v))?),
        Variant::Vector2int16(v) => LuaValue::UserData(lune_create::vector2int16(lua, lune::Vector2int16::from(*v))?),
        Variant::Vector3int16(v) => LuaValue::UserData(lune_create::vector3int16(lua, lune::Vector3int16::from(*v))?),
        Variant::PhysicalProperties(dom::PhysicalProperties::Custom(v)) => {
            LuaValue::UserData(lune_create::physical_properties(lua, lune::PhysicalProperties::from(*v))?)
        }
        _ => LuaValue::Nil,
    })
}

/// A script value stored as given (attributes, `any`-typed properties, event arguments): engine
/// objects and Lune datatypes become typed values, anything else stays a Luau reference.
pub fn from_lua(value: &LuaValue) -> Value {
    match value {
        LuaValue::Nil => Value::Nil,
        LuaValue::Boolean(b) => Value::Bool(*b),
        LuaValue::Integer(n) => Value::Number(*n as f64),
        LuaValue::Number(n) => Value::Number(*n),
        LuaValue::String(s) => Value::String(Str::new(&s.as_bytes())),
        LuaValue::Vector(v) => Value::Vector3(dom::Vector3::new(v.x(), v.y(), v.z())),
        LuaValue::UserData(ud) => userdata_value(ud).unwrap_or_else(|| Value::Lua(Box::new(value.clone()))),
        other => Value::Lua(Box::new(other.clone())),
    }
}

/// A script's Vector3 as a property stores it: its float32 components as they are. (lune's conversion rounds each
/// fractional part to 1/65536, which is for the files it writes: a playing client keeps the float, measured with
/// notes/macro/bodyprobe.luau.)
fn vector3_exact(v: &lune::Vector3) -> dom::Vector3 {
    dom::Vector3::new(v.0.x, v.0.y, v.0.z)
}

/// A script's CFrame as a property stores it: position and rotation rows as the float32s the script made (lune's
/// conversion rounds each to 1/65536: CFrame.Angles(0.001, 0, 0) read back sin 0.001007; a client reads 0.001000).
fn cframe_exact(v: &lune::CFrame) -> dom::CFrame {
    let (x, y, z, w) = (v.0.x_axis, v.0.y_axis, v.0.z_axis, v.0.w_axis);
    dom::CFrame::new(
        dom::Vector3::new(w.x, w.y, w.z),
        dom::Matrix3::new(dom::Vector3::new(x.x, y.x, z.x), dom::Vector3::new(x.y, y.y, z.y), dom::Vector3::new(x.z, y.z, z.z)),
    )
}

fn userdata_value(ud: &LuaAnyUserData) -> Option<Value> {
    if let Some(instance) = instance_of_ud(ud) {
        return Some(Value::Instance(instance));
    }
    let tid = ud.type_id()?;
    if tid == TypeId::of::<LEnumItem>() {
        return ud.borrow::<LEnumItem>().ok().map(|item| Value::EnumItem(item.0));
    }
    if tid == TypeId::of::<lune::CFrame>() {
        return ud.borrow::<lune::CFrame>().ok().map(|v| Value::Dom(Rc::new(Variant::CFrame(cframe_exact(&v)))));
    }
    if tid == TypeId::of::<lune::Color3>() {
        return ud.borrow::<lune::Color3>().ok().map(|v| Value::Color3(dom::Color3::from(*v)));
    }
    if tid == TypeId::of::<lune::UDim2>() {
        return ud.borrow::<lune::UDim2>().ok().map(|v| Value::UDim2(dom::UDim2::from(*v)));
    }
    if tid == TypeId::of::<lune::Vector2>() {
        return ud.borrow::<lune::Vector2>().ok().map(|v| Value::Vector2(dom::Vector2::from(*v)));
    }
    if tid == TypeId::of::<lune::UDim>() {
        return ud.borrow::<lune::UDim>().ok().map(|v| Value::UDim(dom::UDim::from(*v)));
    }
    if tid == TypeId::of::<lune::Vector3>() {
        return ud.borrow::<lune::Vector3>().ok().map(|v| Value::Vector3(vector3_exact(&v)));
    }
    if tid == TypeId::of::<LFont>() {
        return ud.borrow::<LFont>().ok().map(|font| Value::Font(Rc::new(font.0.borrow().clone())));
    }
    if tid == TypeId::of::<LContent>() {
        return ud.borrow::<LContent>().ok().map(|content| Value::Content(Rc::new(content.0.clone())));
    }
    if tid == TypeId::of::<lune::Rect>() {
        return ud.borrow::<lune::Rect>().ok().map(|v| Value::Rect(dom::Rect::from(*v)));
    }
    if tid == TypeId::of::<lune::NumberRange>() {
        return ud.borrow::<lune::NumberRange>().ok().map(|v| Value::NumberRange(dom::NumberRange::from(*v)));
    }
    if tid == TypeId::of::<lune::NumberSequence>() {
        return ud.borrow::<lune::NumberSequence>().ok().map(|v| Value::NumberSequence(Rc::new(dom::NumberSequence::from(v.clone()))));
    }
    if tid == TypeId::of::<lune::ColorSequence>() {
        return ud.borrow::<lune::ColorSequence>().ok().map(|v| Value::ColorSequence(Rc::new(dom::ColorSequence::from(v.clone()))));
    }
    if tid == TypeId::of::<lune::BrickColor>() {
        return ud.borrow::<lune::BrickColor>().ok().map(|v| Value::Dom(Rc::new(Variant::BrickColor(dom::BrickColor::from(*v)))));
    }
    None
}

fn type_error(info: &PropertyInfo, engine: &Engine, reflection_name: &str, expected: &str, value: &LuaValue) -> Flow {
    let _ = reflection_name;
    // (Roblox names a CFrame property's type CoordinateFrame here: measured in a playing client, Studio 0.744,
    // tools/parity/probes/coercion.luau)
    let expected = if expected == "CFrame" { "CoordinateFrame" } else { expected };
    Flow::Error(ScriptError::plain(format!(
        "Unable to assign property {}. {expected} expected, got {}",
        engine.reflection.name(info.name),
        typeof_value(engine, value)
    )))
}

/// Text objects keep TextSize as a whole number from 1 to 100 (measured: 10.61 and 10.99 read
/// back as 10, 0.5 and -3 as 1, 150 as 100).
fn whole_text_size_owner(engine: &Engine, info: &PropertyInfo) -> bool {
    let owner = &*engine.reflection.class(info.owner).name;
    matches!(owner, "TextLabel" | "TextButton" | "TextBox")
}

/// BasePart.Size (a part of any class).
fn part_size_owner(engine: &Engine, info: &PropertyInfo) -> bool {
    engine.reflection.class_id("BasePart").is_some_and(|base| engine.reflection.class(info.owner).is_a(base))
}

/// The Vector3 a Vector3 property stores (BasePart.Size holds each axis from 0.001 to 2048, a not-a-number axis left for
/// lua/instance.rs size_as_stored, which keeps the part's Size: measured, Studio 0.747, tools/parity/probes/datatype_edges.luau).
#[inline]
fn vector3_for(engine: &Engine, info: &PropertyInfo, x: f32, y: f32, z: f32) -> Value {
    if info.name == crate::K::Size as crate::NameId && part_size_owner(engine, info) {
        return Value::Vector3(dom::Vector3::new(x.clamp(0.001, 2048.0), y.clamp(0.001, 2048.0), z.clamp(0.001, 2048.0)));
    }
    Value::Vector3(dom::Vector3::new(x, y, z))
}

/// UIScale.Scale stays within [float32(1e-7), 1000] (measured, tools/studio/layout-probe-zeroscale2:
/// 0, 1e-9, -1 and -inf read back as 1.0000000116860974e-07, inf and 1e30 as 1000, NaN stays NaN).
const MIN_UI_SCALE: f64 = 1e-7_f32 as f64;
const MAX_UI_SCALE: f64 = 1000.0;

/// An `int` property's value for an assigned number, as a C cast to a 32-bit integer gives it: toward zero, and the
/// minimum for a number out of range or not finite (measured in a playing client, Studio 0.744,
/// tools/parity/probes/coercion.luau: ZIndex = 2.7, -2.7, 2^31, 1e20, NaN, inf read 2, -2, then -2147483648).
#[inline]
fn int32_for(number: f64) -> f64 {
    // (+ 0.0: an integer has no negative zero; -0.5 reads 0)
    let truncated = number.trunc() + 0.0;
    if truncated >= i32::MIN as f64 && truncated <= i32::MAX as f64 { truncated } else { i32::MIN as f64 }
}

/// An `int64` property's value for an assigned number: rounded half away from zero, zero for a number out of range or
/// not finite (measured likewise: IntValue.Value = 2.5, -2.5, 0.5, 1e20, NaN read 3, -3, 1, 0, 0).
#[inline]
fn int64_for(number: f64) -> f64 {
    let rounded = number.round() + 0.0;
    if rounded >= -9.223372036854775808e18 && rounded < 9.223372036854775808e18 { rounded } else { 0.0 }
}

/// A method's `int64` argument at `index` (1 is the first after self), as a playing client and a server cast it (Studio 0.747,
/// Player's group methods: tools/parity/probes/loopback/player_groups.luau). A number rounds half away from zero, as an
/// `int64` property's does, but a finite one out of range is the least int64 (2^63, 1e20 and -1e20 name one group) and NaN
/// and the infinities are 0; true is 1 and false 0; a string must be a whole decimal number in range, read exactly
/// ("9007199254740993" is not 2^53): an optional "-" and leading zeros, spaces around it ("+43", "47.0", "4.8e1", "0x3d"
/// and "9223372036854775808" raise; other whitespace than a space is C's isspace, not measured). Err: the message, without
/// a position ("Argument 1 missing or nil", "Unable to cast string to int64", "... Array to int64", "... KeyCode to int64"
/// for an enum item, "... Function to int64").
pub fn int64_argument(engine: &Engine, value: &LuaValue, index: usize) -> Result<i64, String> {
    let number = match value {
        LuaValue::Nil => return Err(format!("Argument {index} missing or nil")),
        LuaValue::Boolean(on) => return Ok(*on as i64),
        LuaValue::Integer(number) => *number as f64,
        LuaValue::Number(number) => *number,
        LuaValue::String(text) => {
            let bytes = text.as_bytes();
            let space = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
            let start = bytes.iter().position(|byte| !space(byte)).unwrap_or(bytes.len());
            let end = bytes.iter().rposition(|byte| !space(byte)).map_or(start, |last| last + 1);
            let trimmed = &bytes[start..end];
            let digits = trimmed.strip_prefix(b"-").unwrap_or(trimmed);
            let whole = !digits.is_empty() && digits.iter().all(u8::is_ascii_digit);
            return std::str::from_utf8(trimmed).ok().filter(|_| whole).and_then(|text| text.parse::<i64>().ok()).ok_or_else(|| "Unable to cast string to int64".to_string());
        }
        LuaValue::Table(table) => return Err(format!("Unable to cast {} to int64", if table.raw_len() > 0 || table.is_empty() { "Array" } else { "Dictionary" })),
        LuaValue::Function(_) => return Err("Unable to cast Function to int64".into()),
        LuaValue::UserData(ud) if ud.is::<LEnumItem>() => {
            let item = ud.borrow::<LEnumItem>().map_err(|error| error.to_string())?.0;
            return Err(format!("Unable to cast {} to int64", engine.reflection.enum_item_type_name(item)));
        }
        other => return Err(format!("Unable to cast {} to int64", typeof_value(engine, other))),
    };
    if !number.is_finite() {
        return Ok(0);
    }
    let rounded = number.round();
    // (-2^63 and 2^63 are doubles exactly)
    let least = i64::MIN as f64;
    Ok(if (least..-least).contains(&rounded) { rounded as i64 } else { i64::MIN })
}

/// The number a numeric property stores for an assigned number (`int` properties truncate, `int64` ones round,
/// float properties round to float32, TextSize of text objects and GetTextBoundsParams.Size are whole and clamped, BorderSizePixel clamped,
/// UIScale.Scale clamped).
#[inline]
pub fn number_for(engine: &Engine, info: &PropertyInfo, number: f64) -> f64 {
    if info.engine_type == EngineType::Integer {
        let whole = int32_for(number);
        // GuiObject.BorderSizePixel stays within [0, 100] (measured: -2.7, 2^31 and "1e3" read 0, 0 and 100)
        if info.name == crate::K::BorderSizePixel as crate::NameId { whole.clamp(0.0, 100.0) } else { whole }
    } else if info.engine_type == EngineType::Int64 {
        int64_for(number)
    } else if info.engine_type == EngineType::Float {
        if info.name == crate::K::TextSize as crate::NameId && whole_text_size_owner(engine, info) {
            // a whole number as an int property takes it, then 1 to 100 (2^31 and NaN read 1)
            int32_for(number).clamp(1.0, 100.0)
        } else if info.name == crate::K::Size as crate::NameId && &*engine.reflection.class(info.owner).name == "GetTextBoundsParams" {
            // 1 to 100 first, then a whole number as an int property takes it (measured, tools/parity/probes/text_bounds.luau: 14.99
            // reads 14, 0.2 and -inf 1, 2^31 and inf 100, and NaN -2^31)
            int32_for(number.clamp(1.0, 100.0))
        } else if info.name == crate::K::Scale as crate::NameId
            && !number.is_nan()
            && &*engine.reflection.class(info.owner).name == "UIScale"
        {
            (number as f32 as f64).clamp(MIN_UI_SCALE, MAX_UI_SCALE)
        } else {
            number as f32 as f64
        }
    } else {
        number
    }
}

/// A number property's value for a value that is no number nor a numeric string: 0, at what a client spends on the conversion it
/// fails (cost.rs CONVERSION_FAILED; a client raises nothing).
fn conversion_failed(engine: &Engine) -> f64 {
    super::cfunction::charge_error_work(engine, crate::cost::CONVERSION_FAILED);
    0.0
}

/// Coerce: converts an assigned value to what the property stores, or the assignment error.
pub fn coerce(lua: &Lua, engine: &Engine, info: &PropertyInfo, value: &LuaValue) -> Result<Value, Flow> {
    let name = engine.reflection.name(info.name);
    match info.script_type {
        ScriptType::Number => {
            // Any value goes: a string as Luau reads a number, anything else (and a string that is no number) as 0
            // (measured in a playing client, Studio 0.744, tools/parity/probes/coercion.luau: "0x10" reads 16,
            // "five", true, {}, a Vector3 and nil read 0)
            let number = match value {
                LuaValue::Number(n) => *n,
                LuaValue::Integer(n) => *n as f64,
                LuaValue::String(_) => lua.coerce_number(value.clone())?.unwrap_or_else(|| conversion_failed(engine)),
                _ => conversion_failed(engine),
            };
            Ok(Value::Number(number_for(engine, info, number)))
        }
        ScriptType::String => match value {
            LuaValue::String(s) => Ok(Value::String(Str::new(&s.as_bytes()))),
            LuaValue::Number(_) | LuaValue::Integer(_) => {
                let text = lua.coerce_string(value.clone())?.map(|s| Str::new(&s.as_bytes())).unwrap_or_default();
                Ok(Value::String(text))
            }
            LuaValue::UserData(ud) if info.engine_type == EngineType::ContentId && ud.is::<LContent>() => {
                let content = ud.borrow::<LContent>()?;
                Ok(Value::String(content.0.uri().cloned().unwrap_or_default()))
            }
            _ => Err(type_error(info, engine, name, "string", value)),
        },
        // Lua truth: only nil and false are false (measured likewise: 0, "" and {} read true)
        ScriptType::Boolean => Ok(Value::Bool(!matches!(value, LuaValue::Nil | LuaValue::Boolean(false)))),
        ScriptType::EnumItem => {
            let Some(enum_name) = &info.enum_name else {
                return Ok(from_lua(value));
            };
            // Beside an item of the enum, its name and its value: a value as a numeric string or with a fraction is
            // truncated ("512" and 512.5 are Wood's 512), and a string that is neither raises `Invalid value "wood" for enum
            // Material` (measured in a playing client, Studio 0.744: tools/parity/probes/values.luau, coercion.luau)
            let coerced = info.enum_type.and_then(|id| {
                coerce_enum(lua, engine, id, value).or_else(|| {
                    let number = match value {
                        LuaValue::Number(n) => Some(*n),
                        LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten(),
                        _ => None,
                    }?;
                    let info = &engine.reflection.enums[id as usize];
                    number.is_finite().then(|| info.by_value.get(&(number.trunc() as i64)).copied()).flatten()
                })
            });
            // What the rest raises, a playing client and a server alike (Studio 0.744, tools/parity/probes/enum_assignment.luau):
            // a number, or a numeric string, no item has `Invalid value 256 for enum Font` (the value truncated: 512.5 shows
            // 512); any other string `Invalid value "Wood" for enum Font`; an item of another enum `EnumItem of type Font
            // expected, got an EnumItem of type SortOrder`; anything else `EnumItem, number, or string expected, got nil`.
            if coerced.is_none() {
                let number = match value {
                    LuaValue::Number(n) => Some(*n),
                    LuaValue::Integer(n) => Some(*n as f64),
                    LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten(),
                    _ => None,
                };
                if let Some(number) = number {
                    let shown = if number.is_finite() {
                        format!("{}", number.trunc() as i64)
                    } else {
                        lua.coerce_string(LuaValue::Number(number)).ok().flatten().map(|s| s.to_string_lossy()).unwrap_or_default()
                    };
                    return Err(Flow::Error(ScriptError::plain(format!(
                        "Unable to assign property {name}. Invalid value {shown} for enum {enum_name}"
                    ))));
                }
                if let LuaValue::String(text) = value {
                    return Err(Flow::Error(ScriptError::plain(format!(
                        "Unable to assign property {name}. Invalid value \"{}\" for enum {enum_name}",
                        text.to_string_lossy()
                    ))));
                }
            }
            match coerced {
                Some(item) => Ok(Value::EnumItem(item)),
                None => Err(Flow::Error(ScriptError::plain(match value {
                    LuaValue::UserData(ud) if ud.is::<LEnumItem>() => {
                        let item = ud.borrow::<LEnumItem>()?.0;
                        let enum_type = engine.reflection.enum_item_info(item).enum_type;
                        format!(
                            "Unable to assign property {name}. EnumItem of type {enum_name} expected, got an EnumItem of type {}",
                            engine.reflection.enums[enum_type as usize].name
                        )
                    }
                    _ => format!("Unable to assign property {name}. EnumItem, number, or string expected, got {}", typeof_value(engine, value)),
                }))),
            }
        }
        ScriptType::Instance => match value {
            LuaValue::Nil => Ok(Value::Nil),
            LuaValue::UserData(ud) if instance_of_ud(ud).is_some() => Ok(Value::Instance(instance_of_ud(ud).unwrap())),
            _ => Err(type_error(info, engine, name, "Instance", value)),
        },
        ScriptType::Content => match value {
            LuaValue::String(s) => Ok(Value::Content(Rc::new(ContentValue::Uri(Str::new(&s.as_bytes()))))),
            LuaValue::Nil => Ok(Value::Content(Rc::new(ContentValue::None))),
            LuaValue::UserData(ud) if ud.is::<LContent>() => Ok(Value::Content(Rc::new(ud.borrow::<LContent>()?.0.clone()))),
            _ => Err(type_error(info, engine, name, "Content", value)),
        },
        ScriptType::Any => Ok(from_lua(value)),
        _ => {
            // Native vectors are Vector3 values in Roblox; properties store the datatype form
            if let (ScriptType::Vector3, LuaValue::Vector(v)) = (info.script_type, value) {
                return Ok(vector3_for(engine, info, v.x(), v.y(), v.z()));
            }
            // Fast path for userdata matching the property's expected datatype (1 borrow instead of scanning 14 types).
            if let LuaValue::UserData(ud) = value {
                match info.script_type {
                    ScriptType::CFrame => {
                        if let Ok(v) = ud.borrow::<lune::CFrame>() {
                            return Ok(Value::Dom(Rc::new(Variant::CFrame(cframe_exact(&v)))));
                        }
                    }
                    ScriptType::Color3 => {
                        if let Ok(v) = ud.borrow::<lune::Color3>() {
                            return Ok(Value::Color3(dom::Color3::from(*v)));
                        }
                    }
                    ScriptType::UDim2 => {
                        if let Ok(v) = ud.borrow::<lune::UDim2>() {
                            return Ok(Value::UDim2(dom::UDim2::from(*v)));
                        }
                    }
                    ScriptType::UDim => {
                        if let Ok(v) = ud.borrow::<lune::UDim>() {
                            return Ok(Value::UDim(dom::UDim::from(*v)));
                        }
                    }
                    ScriptType::Vector2 => {
                        if let Ok(v) = ud.borrow::<lune::Vector2>() {
                            return Ok(Value::Vector2(dom::Vector2::from(*v)));
                        }
                    }
                    ScriptType::Vector3 => {
                        if let Ok(v) = ud.borrow::<lune::Vector3>() {
                            let v = vector3_exact(&v);
                            return Ok(vector3_for(engine, info, v.x, v.y, v.z));
                        }
                    }
                    ScriptType::Rect => {
                        if let Ok(v) = ud.borrow::<lune::Rect>() {
                            return Ok(Value::Rect(dom::Rect::from(*v)));
                        }
                    }
                    ScriptType::NumberRange => {
                        if let Ok(v) = ud.borrow::<lune::NumberRange>() {
                            return Ok(Value::NumberRange(dom::NumberRange::from(*v)));
                        }
                    }
                    ScriptType::NumberSequence => {
                        if let Ok(v) = ud.borrow::<lune::NumberSequence>() {
                            return Ok(Value::NumberSequence(Rc::new(dom::NumberSequence::from(v.clone()))));
                        }
                    }
                    ScriptType::ColorSequence => {
                        if let Ok(v) = ud.borrow::<lune::ColorSequence>() {
                            return Ok(Value::ColorSequence(Rc::new(dom::ColorSequence::from(v.clone()))));
                        }
                    }
                    ScriptType::Font => {
                        if let Ok(font) = ud.borrow::<LFont>() {
                            return Ok(Value::Font(Rc::new(font.0.borrow().clone())));
                        }
                    }
                    ScriptType::BrickColor => {
                        if let Ok(v) = ud.borrow::<lune::BrickColor>() {
                            return Ok(Value::Dom(Rc::new(Variant::BrickColor(dom::BrickColor::from(*v)))));
                        }
                    }
                    _ => {}
                }
            }
            let converted = from_lua(value);
            let matches = match (info.script_type, &converted) {
                (ScriptType::UDim2, Value::UDim2(_))
                | (ScriptType::UDim, Value::UDim(_))
                | (ScriptType::Vector2, Value::Vector2(_))
                | (ScriptType::Vector3, Value::Vector3(_))
                | (ScriptType::Color3, Value::Color3(_))
                | (ScriptType::Rect, Value::Rect(_))
                | (ScriptType::NumberRange, Value::NumberRange(_))
                | (ScriptType::NumberSequence, Value::NumberSequence(_))
                | (ScriptType::ColorSequence, Value::ColorSequence(_))
                | (ScriptType::Font, Value::Font(_)) => true,
                (ScriptType::CFrame, Value::Dom(v)) => matches!(**v, Variant::CFrame(_)),
                (ScriptType::BrickColor, Value::Dom(v)) => matches!(**v, Variant::BrickColor(_)),
                _ => false,
            };
            if matches {
                if let Value::Vector3(v) = &converted {
                    if info.name == crate::K::Size as crate::NameId && part_size_owner(engine, info) {
                        return Ok(Value::Vector3(dom::Vector3::new(v.x.clamp(0.001, 2048.0), v.y.clamp(0.001, 2048.0), v.z.clamp(0.001, 2048.0))));
                    }
                }
                return Ok(converted);
            }
            // nil clears custom physical properties, another type raises (measured in Studio 2026-10-12:
            // BasePart.CustomPhysicalProperties = nil is accepted, also over a set value, and reads back nil; = 5 raises)
            let physical = &*info.engine_type_name == "PhysicalProperties";
            if value.is_nil() && physical {
                return Ok(Value::Nil);
            }
            let actual = typeof_value(engine, value);
            if actual != &*info.script_type_name {
                // (a Faces or an Axes property takes its own type only, an Axes no Faces and a Faces no Axes, read-only ones casting
                // first too: "Unable to assign property Faces. Faces expected, got Axes", measured, roblox-sim 0.747, a client and a
                // server, tools/parity/probes/parent_rules.luau)
                let faces = matches!(&*info.script_type_name, "Faces" | "Axes");
                if value.is_nil() || info.script_type.strict() || physical || faces {
                    return Err(type_error(info, engine, name, &info.script_type_name, value));
                }
            }
            Ok(converted)
        }
    }
}

/// A number as a method's string parameter reads it: up to 20 significant digits, as C's `%.20g` writes them (`0.1` is
/// `0.10000000000000000555`, `1e20` is `1e+20`), and `NAN`, `INF`, `-INF`.
pub fn cast_number_text(number: f64) -> String {
    if number.is_nan() {
        return "NAN".into();
    }
    if number.is_infinite() {
        return if number > 0.0 { "INF" } else { "-INF" }.into();
    }
    let scientific = format!("{number:.19e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("an exponent");
    let exponent: i32 = exponent.parse().expect("a whole exponent");
    let trimmed = |digits: &str| if digits.contains('.') { digits.trim_end_matches('0').trim_end_matches('.').to_string() } else { digits.to_string() };
    if (-4..20).contains(&exponent) {
        trimmed(&format!("{number:.*}", (19 - exponent) as usize))
    } else {
        format!("{}e{}{:02}", trimmed(mantissa), if exponent < 0 { '-' } else { '+' }, exponent.abs())
    }
}

/// A Vector3 as a method's string parameter reads it: each component with nine significant digits, NaN and the infinities
/// as `NAN`, `INF` and `-INF`, as a Vector2's tostring writes its own (measured, Studio 0.748 on a playing client:
/// `0.100000001, 0.333333343, 2` where tostring gives `0.10000000149011612, 0.3333333432674408, 2`, and `NAN, INF, -INF`).
pub fn cast_vector_text(vector: mlua::Vector) -> String {
    let digits = |component: f32| lune_roblox::datatypes::types::format_g(component as f64, 9);
    format!("{}, {}, {}", digits(vector.x()), digits(vector.y()), digits(vector.z()))
}

/// A value as a method's string parameter casts it (measured, Studio 0.747: MarkerCurve:InsertMarkerAtTime's value and
/// PhysicsService's group names, and every Instance and service method tools/parity/probes/string_parameter_methods.luau
/// tries): a string as it is, a number by [`cast_number_text`], a Vector3 by [`cast_vector_text`], a boolean's name, an
/// EnumItem's name, "Instance" for an instance, "{}" for a table, "function", "EventInstance" for a signal, "nil" for an
/// Enum, the Enums, a thread or a plain userdata, "buffer { Size = 2 }", and another datatype as `tostring` writes it. (nil
/// is the caller's: "Argument n missing or nil".) The number and the Vector3 are written in Rust, so a subnormal number keeps
/// its digits in the scripts' float mode (lua/float_mode.rs), as a client writes them (Studio 0.748: 1e-310 is
/// `9.9999999999999694493e-311`).
pub fn cast_text(lua: &Lua, engine: &Engine, value: &LuaValue) -> LuaResult<Vec<u8>> {
    Ok(match value {
        LuaValue::String(text) => text.as_bytes().to_vec(),
        LuaValue::Number(number) => cast_number_text(*number).into_bytes(),
        LuaValue::Integer(number) => cast_number_text(*number as f64).into_bytes(),
        LuaValue::Vector(vector) => cast_vector_text(*vector).into_bytes(),
        LuaValue::Boolean(flag) => flag.to_string().into_bytes(),
        LuaValue::Table(_) => b"{}".to_vec(),
        LuaValue::Function(_) => b"function".to_vec(),
        LuaValue::Thread(_) => b"nil".to_vec(),
        LuaValue::Buffer(buffer) => format!("buffer {{ Size = {} }}", buffer.len()).into_bytes(),
        other => {
            let written = || -> LuaResult<Vec<u8>> { Ok(lua.globals().get::<LuaFunction>("tostring")?.call::<LuaString>(other.clone())?.as_bytes().to_vec()) };
            match typeof_value(engine, other).as_str() {
                "Instance" => b"Instance".to_vec(),
                "Enum" | "Enums" | "userdata" => b"nil".to_vec(),
                "RBXScriptSignal" => b"EventInstance".to_vec(),
                // (`Enum.Material.Plastic` is "Plastic")
                "EnumItem" => written()?.rsplit(|&byte| byte == b'.').next().unwrap_or_default().to_vec(),
                _ => written()?,
            }
        }
    })
}

/// A method's string parameter (`argument`: its number, 1 for the first after self) given `value`, which is not nil, as a client
/// reads it (measured, Studio 0.747, a playing client and a server alike: tools/parity/probes/string_parameter_methods.luau): a
/// string as it is, a number as [`cast_text`] writes it, and anything else as [`cast_text`] writes it after the engine warns
/// `argument #<n> expects a string, but <typeof> was passed` with the script's stack, as an engine warning carries it. A method
/// that yields casts without the warning (`warn` false: WaitForChild, StarterGui:GetCore, Players:GetUserIdFromNameAsync).
pub fn string_param(lua: &Lua, engine: &Engine, value: &LuaValue, argument: usize, warn: bool) -> LuaResult<Vec<u8>> {
    match value {
        LuaValue::String(text) => Ok(text.as_bytes().to_vec()),
        LuaValue::Number(_) | LuaValue::Integer(_) => cast_text(lua, engine, value),
        _ => {
            if warn {
                warn_string_param(lua, engine, value, argument);
            }
            cast_text(lua, engine, value)
        }
    }
}

/// The warning a client gives before it casts a value that is neither a string nor a number to a string parameter ([`string_param`]).
pub fn warn_string_param(lua: &Lua, engine: &Engine, value: &LuaValue, argument: usize) {
    super::cfunction::charge_error_work(engine, crate::cost::CONVERSION_FAILED);
    let message = format!("argument #{argument} expects a string, but {} was passed", typeof_value(engine, value));
    engine.output("warn", message, Some(super::script_traceback(lua)));
}

/// A method argument's cast to an enum, as a client's methods cast one: an item of the enum, an item's name, or a number cut
/// toward zero to an item's value (0.5 and -0.99 are the item of 0, 1.99 the item of 1, 2.5 no item of CameraPanMode's;
/// measured in a 0.747 playing client and a server: tools/parity/probes/camera_units.luau SetCameraPanMode, bulk_move_args.luau
/// BulkMoveTo's mode). [`coerce_enum`] takes whole numbers only.
pub fn coerce_enum_arg(lua: &Lua, engine: &Engine, enum_id: u32, value: &LuaValue) -> Option<u32> {
    match value {
        LuaValue::Number(n) => n.is_finite().then(|| engine.reflection.enums[enum_id as usize].by_value.get(&(n.trunc() as i64)).copied()).flatten(),
        _ => coerce_enum(lua, engine, enum_id, value),
    }
}

/// Enums.Coerce: an EnumItem of the type, an item name or an item value.
pub fn coerce_enum(lua: &Lua, engine: &Engine, enum_id: u32, value: &LuaValue) -> Option<u32> {
    let _ = lua;
    let info = &engine.reflection.enums[enum_id as usize];
    match value {
        LuaValue::UserData(ud) => {
            let item = ud.borrow::<LEnumItem>().ok()?.0;
            (engine.reflection.enum_item_info(item).enum_type == enum_id).then_some(item)
        }
        LuaValue::String(s) => info.by_name.get(&*s.to_str().ok()?).copied(),
        LuaValue::Number(n) => (n.fract() == 0.0).then(|| info.by_value.get(&(*n as i64)).copied()).flatten(),
        LuaValue::Integer(n) => info.by_value.get(&(*n as i64)).copied(),
        _ => None,
    }
}
