//! Conversions between engine [`Value`]s and Luau values, and property coercion (port of
//! instance.luau Coerce). Lune's datatype userdata are the script representation of rbx-dom
//! datatypes, as in the Luau runtime.

use std::rc::Rc;

use lune_roblox::datatypes::types as lune;
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
        Value::UDim(v) => LuaValue::UserData(lua.create_userdata(lune::UDim::from(*v))?),
        Value::UDim2(v) => LuaValue::UserData(lua.create_userdata(lune::UDim2::from(*v))?),
        Value::Vector2(v) => LuaValue::UserData(lua.create_userdata(lune::Vector2::from(*v))?),
        // (the native vector type, as in Roblox: prelude.luau "Vector3")
        Value::Vector3(v) => LuaValue::Vector(mlua::Vector::new(v.x, v.y, v.z)),
        Value::Color3(v) => LuaValue::UserData(lua.create_userdata(lune::Color3::from(*v))?),
        Value::Rect(v) => LuaValue::UserData(lua.create_userdata(lune::Rect::from(*v))?),
        Value::NumberRange(v) => LuaValue::UserData(lua.create_userdata(lune::NumberRange::from(*v))?),
        Value::NumberSequence(v) => {
            LuaValue::UserData(lua.create_userdata(lune::NumberSequence::from((**v).clone()))?)
        }
        Value::ColorSequence(v) => {
            LuaValue::UserData(lua.create_userdata(lune::ColorSequence::from((**v).clone()))?)
        }
        Value::Font(font) => LuaValue::UserData(lua.create_any_userdata(LFont(std::cell::RefCell::new((**font).clone())))?),
        Value::Content(content) => LuaValue::UserData(lua.create_any_userdata(LContent((**content).clone()))?),
        Value::Dom(variant) => dom_to_lua(lua, variant)?,
        Value::Lua(value) => (**value).clone(),
    })
}

/// Other rbx-dom datatypes as Lune userdata.
pub fn dom_to_lua(lua: &Lua, variant: &Variant) -> LuaResult<LuaValue> {
    macro_rules! ud {
        ($value:expr) => {
            LuaValue::UserData(lua.create_userdata($value)?)
        };
    }
    Ok(match variant {
        Variant::Axes(v) => ud!(lune::Axes::from(*v)),
        Variant::BrickColor(v) => ud!(lune::BrickColor::from(*v)),
        Variant::CFrame(v) => ud!(lune::CFrame::from(*v)),
        Variant::Faces(v) => ud!(lune::Faces::from(*v)),
        Variant::Ray(v) => ud!(lune::Ray::from(*v)),
        Variant::Region3(v) => ud!(lune::Region3::from(*v)),
        Variant::Region3int16(v) => ud!(lune::Region3int16::from(*v)),
        Variant::UniqueId(v) => ud!(lune::UniqueId::from(*v)),
        Variant::Vector2int16(v) => ud!(lune::Vector2int16::from(*v)),
        Variant::Vector3int16(v) => ud!(lune::Vector3int16::from(*v)),
        Variant::PhysicalProperties(dom::PhysicalProperties::Custom(v)) => {
            ud!(lune::PhysicalProperties::from(*v))
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

fn userdata_value(ud: &LuaAnyUserData) -> Option<Value> {
    if let Some(instance) = instance_of_ud(ud) {
        return Some(Value::Instance(instance));
    }
    if let Ok(item) = ud.borrow::<LEnumItem>() {
        return Some(Value::EnumItem(item.0));
    }
    if let Ok(v) = ud.borrow::<lune::UDim2>() {
        return Some(Value::UDim2(dom::UDim2::from(*v)));
    }
    if let Ok(v) = ud.borrow::<lune::Color3>() {
        return Some(Value::Color3(dom::Color3::from(*v)));
    }
    if let Ok(v) = ud.borrow::<lune::Vector2>() {
        return Some(Value::Vector2(dom::Vector2::from(*v)));
    }
    if let Ok(v) = ud.borrow::<lune::UDim>() {
        return Some(Value::UDim(dom::UDim::from(*v)));
    }
    if let Ok(v) = ud.borrow::<lune::Vector3>() {
        return Some(Value::Vector3(dom::Vector3::from(*v)));
    }
    if let Ok(font) = ud.borrow::<LFont>() {
        return Some(Value::Font(Rc::new(font.0.borrow().clone())));
    }
    if let Ok(content) = ud.borrow::<LContent>() {
        return Some(Value::Content(Rc::new(content.0.clone())));
    }
    if let Ok(v) = ud.borrow::<lune::Rect>() {
        return Some(Value::Rect(dom::Rect::from(*v)));
    }
    if let Ok(v) = ud.borrow::<lune::NumberRange>() {
        return Some(Value::NumberRange(dom::NumberRange::from(*v)));
    }
    if let Ok(v) = ud.borrow::<lune::NumberSequence>() {
        return Some(Value::NumberSequence(Rc::new(dom::NumberSequence::from(v.clone()))));
    }
    if let Ok(v) = ud.borrow::<lune::ColorSequence>() {
        return Some(Value::ColorSequence(Rc::new(dom::ColorSequence::from(v.clone()))));
    }
    if let Ok(v) = ud.borrow::<lune::CFrame>() {
        return Some(Value::Dom(Rc::new(Variant::CFrame(dom::CFrame::from(*v)))));
    }
    if let Ok(v) = ud.borrow::<lune::BrickColor>() {
        return Some(Value::Dom(Rc::new(Variant::BrickColor(dom::BrickColor::from(*v)))));
    }
    None
}

fn type_error(info: &PropertyInfo, engine: &Engine, reflection_name: &str, expected: &str, value: &LuaValue) -> Flow {
    let _ = reflection_name;
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

/// UIScale.Scale stays within [float32(1e-7), 1000] (measured, tools/studio/layout-probe-zeroscale2:
/// 0, 1e-9, -1 and -inf read back as 1.0000000116860974e-07, inf and 1e30 as 1000, NaN stays NaN).
const MIN_UI_SCALE: f64 = 1e-7_f32 as f64;
const MAX_UI_SCALE: f64 = 1000.0;

/// The number a numeric property stores for an assigned number (integer properties truncate,
/// float properties round to float32, TextSize of text objects is whole and clamped, UIScale.Scale
/// clamped).
#[inline]
pub fn number_for(engine: &Engine, info: &PropertyInfo, number: f64) -> f64 {
    if info.engine_type == EngineType::Integer && number.is_finite() {
        number.trunc()
    } else if info.engine_type == EngineType::Float {
        if info.name == crate::K::TextSize as crate::NameId && whole_text_size_owner(engine, info) && !number.is_nan() {
            number.floor().clamp(1.0, 100.0)
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

/// Coerce: converts an assigned value to what the property stores, or the assignment error.
pub fn coerce(lua: &Lua, engine: &Engine, info: &PropertyInfo, value: &LuaValue) -> Result<Value, Flow> {
    let name = engine.reflection.name(info.name);
    match info.script_type {
        ScriptType::Number => {
            let number = match value {
                LuaValue::Number(n) => Some(*n),
                LuaValue::Integer(n) => Some(*n as f64),
                LuaValue::String(_) => lua.coerce_number(value.clone())?,
                _ => None,
            };
            let Some(number) = number else {
                return Err(type_error(info, engine, name, "number", value));
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
        ScriptType::Boolean => match value {
            LuaValue::Boolean(b) => Ok(Value::Bool(*b)),
            LuaValue::Nil => Ok(Value::Bool(false)),
            _ => Err(type_error(info, engine, name, "boolean", value)),
        },
        ScriptType::EnumItem => {
            let Some(enum_name) = &info.enum_name else {
                return Ok(from_lua(value));
            };
            match info.enum_type.and_then(|id| coerce_enum(lua, engine, id, value)) {
                Some(item) => Ok(Value::EnumItem(item)),
                None => {
                    let got = match value {
                        LuaValue::UserData(ud) if ud.is::<LEnumItem>() => {
                            engine.reflection.enum_item_path(ud.borrow::<LEnumItem>()?.0)
                        }
                        _ => typeof_value(engine, value),
                    };
                    Err(Flow::Error(ScriptError::plain(format!(
                        "Unable to assign property {name}. EnumItem of type {enum_name} expected, got {got}"
                    ))))
                }
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
                return Ok(Value::Vector3(dom::Vector3::new(v.x(), v.y(), v.z())));
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
                if value.is_nil() || info.script_type.strict() || physical {
                    return Err(type_error(info, engine, name, &info.script_type_name, value));
                }
            }
            Ok(converted)
        }
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
