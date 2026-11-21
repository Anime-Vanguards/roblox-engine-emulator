use core::fmt;
use std::ops;

use glam::{Vec2, Vec3};
use mlua::prelude::*;
use rbx_dom_weak::types::Vector2 as DomVector2;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, charge_host_nanos, datatype_price as price};
use crate::datatypes::script_error::{
    incompatible, invalid_argument, luau_arg, luau_number, luau_self, reflected_arg, reflected_float_arg, same_operands,
    script_error, userdata_of,
};

/**
    An implementation of the [Vector2](https://create.roblox.com/docs/reference/engine/datatypes/Vector2)
    Roblox datatype, backed by [`glam::Vec2`].

    This implements all documented properties, methods &
    constructors of the Vector2 class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vector2(pub Vec2);

impl LuaExportsTable for Vector2 {
    const EXPORT_NAME: &'static str = "Vector2";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        let vector2_new = |lua: &Lua, (x, y): (Option<f32>, Option<f32>)| {
            charge_host_nanos(lua, price::VECTOR2_NEW);
            Ok(Vector2(Vec2 {
                x: x.unwrap_or_default(),
                y: y.unwrap_or_default(),
            }))
        };

        TableBuilder::new(lua)?
            .with_value("xAxis", Vector2(Vec2::X))?
            .with_value("yAxis", Vector2(Vec2::Y))?
            .with_value("zero", Vector2(Vec2::ZERO))?
            .with_value("one", Vector2(Vec2::ONE))?
            .with_function("new", vector2_new)?
            .build_readonly()
    }
}

impl LuaUserData for Vector2 {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (an assignment raises as a client's does: datatypes::script_error::AssignMembers)
        fields.add_meta_field("__newindex", crate::datatypes::script_error::AssignMembers("Vector2"));
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::with_methods("Vector2"));
        fields.add_field_method_get("Magnitude", magnitude);
        fields.add_field_method_get("Unit", unit);
        fields.add_field_method_get("X", |_, this| Ok(this.0.x));
        fields.add_field_method_get("Y", |_, this| Ok(this.0.y));
        // Roblox's deprecated lowercase names (Roblox's PlayerModule reads `ViewportSize.x`), as fast as the others in a client:
        // the parity bench's Vector2.x 3.4 ns, as Vector2.X
        fields.add_field_method_get("magnitude", magnitude);
        fields.add_field_method_get("unit", unit);
        fields.add_field_method_get("x", |_, this| Ok(this.0.x));
        fields.add_field_method_get("y", |_, this| Ok(this.0.y));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Methods, with a playing client's argument errors (Studio 0.747, ree's tools/parity/probes/datatype_errors.luau).
        // Abs, Ceil, Floor, Sign, Angle, FuzzyEq, Max and Min are checked as the client's Luau checks them, from the calling
        // line (self named as a `.` call names it, a later argument as a `:` call does); Cross, Dot and Lerp go through its
        // reflection ("Argument 2 missing or nil", "Unable to cast double to Vector2"), each in the order it checks.
        for (name, op) in [("Abs", Vec2::abs as fn(Vec2) -> Vec2), ("Ceil", Vec2::ceil), ("Floor", Vec2::floor), ("Sign", Vec2::signum)] {
            methods.add_function(name, move |_, args: LuaMultiValue| {
                let this = luau_self::<Vector2>(&args, name, "Vector2")?;
                Ok(Vector2(op(this.0)))
            });
        }
        // atan2f(a x b, a . b) in floats, signed only when asked (Studio 0.744, tools/parity/probes/cframe_order.luau
        // measures the signed form; the unsigned one is its absolute value, as Roblox documents); fewer than two arguments
        // is the client's count error, without a position
        methods.add_function("Angle", |_, args: LuaMultiValue| {
            if args.len() < 2 {
                return Err(script_error(format!("Invalid number of arguments: {}, Vector2.Angle expects 2 or 3.", args.len()), false));
            }
            let this = luau_self::<Vector2>(&args, "Angle", "Vector2")?;
            let rhs = luau_arg::<Vector2>(&args, 2, "Vector2")?;
            let signed = match args.get(2) {
                None => false,
                Some(LuaValue::Boolean(signed)) => *signed,
                Some(other) => return Err(invalid_argument(3, None, "boolean", other)),
            };
            let (a, b) = (this.0, rhs.0);
            let angle = (a.x * b.y - a.y * b.x).atan2(a.x * b.x + a.y * b.y);
            Ok(if signed { angle } else { angle.abs() })
        });
        methods.add_function("Cross", |_, args: LuaMultiValue| {
            let rhs = reflected_arg::<Vector2>(&args, 2, "Vector2")?;
            let this = reflected_arg::<Vector2>(&args, 1, "Vector2")?;
            let this_v3 = Vec3::new(this.0.x, this.0.y, 0f32);
            let rhs_v3 = Vec3::new(rhs.0.x, rhs.0.y, 0f32);
            Ok(this_v3.cross(rhs_v3).z)
        });
        methods.add_function("Dot", |_, args: LuaMultiValue| {
            let this = reflected_arg::<Vector2>(&args, 1, "Vector2")?;
            let rhs = reflected_arg::<Vector2>(&args, 2, "Vector2")?;
            Ok(this.0.dot(rhs.0))
        });
        // (an eps over 0.1 in size is the client's message, without a position; no eps is 1e-5)
        methods.add_function("FuzzyEq", |lua, args: LuaMultiValue| {
            if args.len() < 2 {
                return Err(script_error(format!("Invalid number of arguments: {}, Vector2.FuzzyEq expects 2 or 3.", args.len()), false));
            }
            let this = luau_self::<Vector2>(&args, "FuzzyEq", "Vector2")?;
            let rhs = luau_arg::<Vector2>(&args, 2, "Vector2")?;
            let epsilon = match args.get(2) {
                None => 1e-5f32,
                Some(value) => luau_number(lua, value).ok_or_else(|| invalid_argument(3, None, "number", value))? as f32,
            };
            if epsilon.abs() > 0.1 {
                return Err(script_error("The eps value provided to FuzzyEq should be a small positive value <= 0.1", false));
            }
            let eq_x = (rhs.0.x - this.0.x).abs() <= epsilon;
            let eq_y = (rhs.0.y - this.0.y).abs() <= epsilon;
            Ok(eq_x && eq_y)
        });
        // (and Roblox's deprecated lowercase name, measured in a client: ree's tools/parity/probes/camelcase.luau)
        // a + (b - a) * t in floats (Studio 0.744, tools/parity/probes/cframe_order.luau); the reflection checks self, then
        // the alpha, then the goal
        let lerp = |lua: &Lua, args: LuaMultiValue| {
            let this = reflected_arg::<Vector2>(&args, 1, "Vector2")?;
            let alpha = reflected_float_arg(lua, &args, 3)? as f32;
            let rhs = reflected_arg::<Vector2>(&args, 2, "Vector2")?;
            charge_host_nanos(lua, price::VECTOR2_LERP);
            let lerp = super::float32::lerp;
            Ok(Vector2(Vec2::new(lerp(this.0.x, rhs.0.x, alpha), lerp(this.0.y, rhs.0.y, alpha))))
        };
        methods.add_function("Lerp", lerp);
        methods.add_function("lerp", lerp);
        // Max and Min take any number of Vector2s after self (none: self)
        for (name, op) in [("Max", Vec2::max as fn(Vec2, Vec2) -> Vec2), ("Min", Vec2::min)] {
            methods.add_function(name, move |_, args: LuaMultiValue| {
                let mut result = luau_self::<Vector2>(&args, name, "Vector2")?.0;
                for index in 2..=args.len() {
                    result = op(result, luau_arg::<Vector2>(&args, index, "Vector2")?.0);
                }
                Ok(Vector2(result))
            });
        }
        // Metamethods
        methods.add_meta_method(LuaMetaMethod::Eq, |lua, this, value: LuaValue| {
            charge_host_nanos(lua, price::EQ);
            userdata_impl_eq(lua, this, value)
        });
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
        methods.add_meta_method(LuaMetaMethod::Unm, |lua, this, ()| {
            charge_host_nanos(lua, price::ARITHMETIC);
            userdata_impl_unm(lua, this, ())
        });
        // + and - take two Vector2s, whichever side the Vector2 that runs them is on: a client's Luau checks raise the
        // first that is not one, from the calling line ("invalid argument #2 (Vector2 expected, got number)")
        methods.add_meta_function(LuaMetaMethod::Add, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = same_operands::<Vector2>(&lhs, &rhs, "Vector2")?;
            charge_host_nanos(lua, price::ARITHMETIC);
            Ok(a + b)
        });
        methods.add_meta_function(LuaMetaMethod::Sub, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = same_operands::<Vector2>(&lhs, &rhs, "Vector2")?;
            charge_host_nanos(lua, price::ARITHMETIC);
            Ok(a - b)
        });
        // *, / and // take a Vector2 and a Vector2 or a number (a string Luau reads as one included), the number on either
        // side (measured in Studio: 2 * v, 2 / v and 2 // v per component); anything else is the client's own message
        methods.add_meta_function(LuaMetaMethod::Mul, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = scaled(lua, &lhs, &rhs, "multiply")?;
            charge_host_nanos(lua, price::ARITHMETIC);
            Ok(Vector2(a * b))
        });
        methods.add_meta_function(LuaMetaMethod::Div, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = scaled(lua, &lhs, &rhs, "divide")?;
            charge_host_nanos(lua, price::ARITHMETIC);
            Ok(Vector2(a / b))
        });
        methods.add_meta_function(LuaMetaMethod::IDiv, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = scaled(lua, &lhs, &rhs, "floor divide")?;
            charge_host_nanos(lua, price::ARITHMETIC);
            Ok(Vector2((a / b).floor()))
        });
    }
}

/// The operands of a *, / or // as vectors (a number splatted), in their order.
fn scaled(lua: &Lua, lhs: &LuaValue, rhs: &LuaValue, verb: &str) -> LuaResult<(Vec2, Vec2)> {
    let side = |value: &LuaValue| match userdata_of::<Vector2>(Some(value)) {
        Some(v) => Some((v.0, true)),
        None => luau_number(lua, value).map(|n| (Vec2::splat(n as f32), false)),
    };
    match (side(lhs), side(rhs)) {
        (Some((a, a_vector)), Some((b, b_vector))) if a_vector || b_vector => Ok((a, b)),
        _ => Err(incompatible(verb, "Vector2")),
    }
}

fn magnitude(lua: &Lua, this: &Vector2) -> LuaResult<f32> {
    charge_host_nanos(lua, price::VECTOR2_MAGNITUDE);
    Ok(this.0.length())
}

fn unit(lua: &Lua, this: &Vector2) -> LuaResult<Vector2> {
    charge_host_nanos(lua, price::MEMBER_NEW);
    Ok(Vector2(this.0.normalize()))
}



impl fmt::Display for Vector2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 9 significant digits ("0.333333343, -2.5"; Studio 0.744)
        write!(f, "{}, {}", super::format_g(f64::from(self.0.x), 9), super::format_g(f64::from(self.0.y), 9))
    }
}

impl ops::Neg for Vector2 {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Vector2(-self.0)
    }
}

impl ops::Add for Vector2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Vector2(self.0 + rhs.0)
    }
}

impl ops::Sub for Vector2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Vector2(self.0 - rhs.0)
    }
}

impl ops::Mul for Vector2 {
    type Output = Vector2;
    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl ops::Mul<f32> for Vector2 {
    type Output = Vector2;
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl ops::Div for Vector2 {
    type Output = Vector2;
    fn div(self, rhs: Self) -> Self::Output {
        Self(self.0 / rhs.0)
    }
}

impl ops::Div<f32> for Vector2 {
    type Output = Vector2;
    fn div(self, rhs: f32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl IDiv for Vector2 {
    type Output = Vector2;
    fn idiv(self, rhs: Self) -> Self::Output {
        Self((self.0 / rhs.0).floor())
    }
}

impl IDiv<f32> for Vector2 {
    type Output = Vector2;
    fn idiv(self, rhs: f32) -> Self::Output {
        Self((self.0 / rhs).floor())
    }
}

impl From<DomVector2> for Vector2 {
    fn from(v: DomVector2) -> Self {
        Vector2(Vec2 { x: v.x, y: v.y })
    }
}

impl From<Vector2> for DomVector2 {
    fn from(v: Vector2) -> Self {
        DomVector2 { x: v.0.x, y: v.0.y }
    }
}
