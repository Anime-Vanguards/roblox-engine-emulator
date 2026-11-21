use core::fmt;
use std::ops;

use glam::IVec3;
use mlua::prelude::*;
use rbx_dom_weak::types::Vector3int16 as DomVector3int16;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::super::*;
use crate::datatypes::script_error::{luau_number, same_operands, script_error, userdata_of};

/**
    An implementation of the [Vector3int16](https://create.roblox.com/docs/reference/engine/datatypes/Vector3int16)
    Roblox datatype, backed by [`glam::IVec3`].

    This implements all documented properties, methods &
    constructors of the Vector3int16 class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector3int16(pub IVec3);

/// A constructor component: truncated toward zero (1.5, -1.5, 2.7 make 1, -1, 2), and a value outside
/// [-32768, 32768) overflows the 16-bit integer (32768 is -32768, 65537 is 1) instead of raising (measured in a playing
/// client, Studio 0.744). Non-finite values are 0 (not measured in Studio).
pub(crate) fn int16_component(value: Option<f64>) -> i32 {
    match value {
        Some(value) if value.is_finite() => i32::from(value.trunc() as i64 as i16),
        _ => 0,
    }
}

impl LuaExportsTable for Vector3int16 {
    const EXPORT_NAME: &'static str = "Vector3int16";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        // (each component read as a client reads it: a number or a string Luau reads as one, anything else 0;
        // Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let vector3int16_new = |lua: &Lua, args: LuaMultiValue| {
            let x = args.get(0).and_then(|value| luau_number(lua, value));
            let y = args.get(1).and_then(|value| luau_number(lua, value));
            let z = args.get(2).and_then(|value| luau_number(lua, value));
            Ok(Vector3int16(IVec3 {
                x: int16_component(x),
                y: int16_component(y),
                z: int16_component(z),
            }))
        };

        TableBuilder::new(lua)?
            .with_function("new", vector3int16_new)?
            .build_readonly()
    }
}

impl LuaUserData for Vector3int16 {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (an assignment raises as a client's does: datatypes::script_error::AssignMembers)
        fields.add_meta_field("__newindex", crate::datatypes::script_error::AssignMembers("Vector3int16"));
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::without_methods("Vector3int16"));
        fields.add_field_method_get("X", |_, this| Ok(this.0.x));
        fields.add_field_method_get("Y", |_, this| Ok(this.0.y));
        fields.add_field_method_get("Z", |_, this| Ok(this.0.z));
        // (Roblox's lowercase names, measured in a client: ree's tools/parity/probes/camelcase.luau)
        fields.add_field_method_get("x", |_, this| Ok(this.0.x));
        fields.add_field_method_get("y", |_, this| Ok(this.0.y));
        fields.add_field_method_get("z", |_, this| Ok(this.0.z));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
        methods.add_meta_method(LuaMetaMethod::Unm, userdata_impl_unm);
        // (two Vector3int16s, whichever side: a client raises "invalid argument #2 (Vector3int16 expected, got number)" from the
        // calling line, Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        methods.add_meta_function(LuaMetaMethod::Add, |_, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = same_operands::<Vector3int16>(&lhs, &rhs, "Vector3int16")?;
            Ok(a + b)
        });
        methods.add_meta_function(LuaMetaMethod::Sub, |_, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = same_operands::<Vector3int16>(&lhs, &rhs, "Vector3int16")?;
            Ok(a - b)
        });
        // * and / take a number on either side (truncated to an integer first, then each component multiplied in 32 bits
        // and wrapped to 16) or another Vector3int16 (component by component); an operand of any other kind reads as 0,
        // without an error. A Vector3int16 divided by a number divides each component in doubles (x86's conversion: by 0
        // it is 0); a division by a Vector3int16 is an integer one, which raises the client's 'Divide by zero exception'
        // for a zero component (Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        methods.add_meta_function(LuaMetaMethod::Mul, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let scale = |v: Vector3int16, n: f64| {
                let k = x86_i32(n);
                Vector3int16(v.0.map(|c| c.wrapping_mul(k) as i16 as i32))
            };
            Ok(match (userdata_of::<Vector3int16>(Some(&lhs)), userdata_of::<Vector3int16>(Some(&rhs))) {
                (Some(a), Some(b)) => *a * *b,
                (Some(a), None) => scale(*a, operand_number(lua, &rhs)),
                (None, Some(b)) => scale(*b, operand_number(lua, &lhs)),
                (None, None) => unreachable!("a Vector3int16's * has one"),
            })
        });
        methods.add_meta_function(LuaMetaMethod::Div, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let divided = |a: IVec3, b: IVec3| -> LuaResult<Vector3int16> {
                if b.x == 0 || b.y == 0 || b.z == 0 {
                    return Err(divide_by_zero());
                }
                Ok(Vector3int16(IVec3::new(a.x.wrapping_div(b.x), a.y.wrapping_div(b.y), a.z.wrapping_div(b.z)).map(|c| c as i16 as i32)))
            };
            match (userdata_of::<Vector3int16>(Some(&lhs)), userdata_of::<Vector3int16>(Some(&rhs))) {
                (Some(a), Some(b)) => divided(a.0, b.0),
                (Some(a), None) => {
                    let n = operand_number(lua, &rhs);
                    Ok(Vector3int16(a.0.map(|c| x86_i32(f64::from(c) / n) as i16 as i32)))
                }
                (None, Some(b)) => divided(IVec3::splat(x86_i32(operand_number(lua, &lhs))), b.0),
                (None, None) => unreachable!("a Vector3int16's / has one"),
            }
        });
    }
}

impl fmt::Display for Vector3int16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}, {}", self.0.x, self.0.y, self.0.z)
    }
}

/// x86's conversion of a double to a 32-bit integer (cvttsd2si), which a client's int16 vector arithmetic goes through:
/// toward zero, and 0x80000000 for a value out of range or not a number (whose 16 low bits are 0: Vector3int16(10, -10, 7)
/// * 1e10 is 0, 0, 0; Studio 0.747, ree's tools/parity/probes/datatype_errors.luau).
pub(crate) fn x86_i32(value: f64) -> i32 {
    if value.is_nan() || value >= 2_147_483_648.0 || value <= -2_147_483_649.0 { i32::MIN } else { value as i32 }
}

/// The number an operand of an int16 vector's * or / reads as: a number or a string Luau reads as one, anything else 0
/// (Vector3int16 * "x" is 0, 0, 0 in a client: Studio 0.747, ree's tools/parity/probes/datatype_errors.luau).
pub(crate) fn operand_number(lua: &Lua, value: &LuaValue) -> f64 {
    luau_number(lua, value).unwrap_or(0.0)
}

/// The client's message for an integer division by zero (without a position)
pub(crate) fn divide_by_zero() -> LuaError {
    script_error("Divide by zero exception", false)
}

/// Arithmetic results overflow the 16-bit integer as the constructor's components do: `Vector3int16.new(1, 2, 3) +
/// Vector3int16.new(32767, 0, 0)` is (-32768, 2, 3) in a playing client (Studio 0.744; addition measured, the other
/// operators taken to be the same 16-bit arithmetic).
fn wrap16(v: IVec3) -> IVec3 {
    IVec3 { x: v.x as i16 as i32, y: v.y as i16 as i32, z: v.z as i16 as i32 }
}

impl ops::Neg for Vector3int16 {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Vector3int16(wrap16(-self.0))
    }
}

impl ops::Add for Vector3int16 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Vector3int16(wrap16(self.0 + rhs.0))
    }
}

impl ops::Sub for Vector3int16 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Vector3int16(wrap16(self.0 - rhs.0))
    }
}

impl ops::Mul for Vector3int16 {
    type Output = Vector3int16;
    fn mul(self, rhs: Self) -> Self::Output {
        Self(wrap16(self.0 * rhs.0))
    }
}

impl ops::Mul<i32> for Vector3int16 {
    type Output = Vector3int16;
    fn mul(self, rhs: i32) -> Self::Output {
        Self(wrap16(self.0 * rhs))
    }
}

impl ops::Div for Vector3int16 {
    type Output = Vector3int16;
    fn div(self, rhs: Self) -> Self::Output {
        Self(wrap16(self.0 / rhs.0))
    }
}

impl ops::Div<i32> for Vector3int16 {
    type Output = Vector3int16;
    fn div(self, rhs: i32) -> Self::Output {
        Self(wrap16(self.0 / rhs))
    }
}

impl From<DomVector3int16> for Vector3int16 {
    fn from(v: DomVector3int16) -> Self {
        Vector3int16(IVec3 {
            x: v.x.clamp(i16::MIN, i16::MAX) as i32,
            y: v.y.clamp(i16::MIN, i16::MAX) as i32,
            z: v.z.clamp(i16::MIN, i16::MAX) as i32,
        })
    }
}

impl From<Vector3int16> for DomVector3int16 {
    fn from(v: Vector3int16) -> Self {
        DomVector3int16 {
            x: v.0.x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            y: v.0.y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            z: v.0.z.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        }
    }
}
