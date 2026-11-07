use core::fmt;
use std::ops;

use glam::{Vec2, Vec3};
use mlua::prelude::*;
use rbx_dom_weak::types::Vector2 as DomVector2;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, charge_host_nanos, datatype_price as price};

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
        // Methods
        methods.add_method("Angle", |_, this, rhs: LuaUserDataRef<Vector2>| {
            Ok(this.0.angle_to(rhs.0))
        });
        methods.add_method("Cross", |_, this, rhs: LuaUserDataRef<Vector2>| {
            let this_v3 = Vec3::new(this.0.x, this.0.y, 0f32);
            let rhs_v3 = Vec3::new(rhs.0.x, rhs.0.y, 0f32);
            Ok(this_v3.cross(rhs_v3).z)
        });
        methods.add_method("Dot", |_, this, rhs: LuaUserDataRef<Vector2>| {
            Ok(this.0.dot(rhs.0))
        });
        methods.add_method(
            "FuzzyEq",
            |_, this, (rhs, epsilon): (LuaUserDataRef<Vector2>, f32)| {
                let eq_x = (rhs.0.x - this.0.x).abs() <= epsilon;
                let eq_y = (rhs.0.y - this.0.y).abs() <= epsilon;
                Ok(eq_x && eq_y)
            },
        );
        methods.add_method(
            "Lerp",
            |lua, this, (rhs, alpha): (LuaUserDataRef<Vector2>, f32)| {
                charge_host_nanos(lua, price::VECTOR2_LERP);
                Ok(Vector2(this.0.lerp(rhs.0, alpha)))
            },
        );
        methods.add_method("Max", |_, this, rhs: LuaUserDataRef<Vector2>| {
            Ok(Vector2(this.0.max(rhs.0)))
        });
        methods.add_method("Min", |_, this, rhs: LuaUserDataRef<Vector2>| {
            Ok(Vector2(this.0.min(rhs.0)))
        });
        methods.add_method("Abs", |_, this, ()| Ok(Vector2(this.0.abs())));
        methods.add_method("Ceil", |_, this, ()| Ok(Vector2(this.0.ceil())));
        methods.add_method("Floor", |_, this, ()| Ok(Vector2(this.0.floor())));
        methods.add_method("Sign", |_, this, ()| Ok(Vector2(this.0.signum())));
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
        methods.add_meta_method(LuaMetaMethod::Add, |lua, this, rhs: LuaUserDataRef<Vector2>| {
            charge_host_nanos(lua, price::ARITHMETIC);
            userdata_impl_add(lua, this, rhs)
        });
        methods.add_meta_method(LuaMetaMethod::Sub, |lua, this, rhs: LuaUserDataRef<Vector2>| {
            charge_host_nanos(lua, price::ARITHMETIC);
            userdata_impl_sub(lua, this, rhs)
        });
        // Roblox also takes the number first (measured in Studio: 2 * v, 2 / v and 2 // v per
        // component)
        methods.add_meta_function(LuaMetaMethod::Mul, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            charge_host_nanos(lua, price::ARITHMETIC);
            match number_first(&lhs, &rhs)? {
                Some((n, v)) => Ok(Vector2(v.0 * n)),
                None => userdata_impl_mul_f32(lua, &*vector2_of(&lhs)?, rhs),
            }
        });
        methods.add_meta_function(LuaMetaMethod::Div, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            charge_host_nanos(lua, price::ARITHMETIC);
            match number_first(&lhs, &rhs)? {
                Some((n, v)) => Ok(Vector2(Vec2::splat(n) / v.0)),
                None => userdata_impl_div_f32(lua, &*vector2_of(&lhs)?, rhs),
            }
        });
        methods.add_meta_function(LuaMetaMethod::IDiv, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            charge_host_nanos(lua, price::ARITHMETIC);
            match number_first(&lhs, &rhs)? {
                Some((n, v)) => Ok(Vector2((Vec2::splat(n) / v.0).floor())),
                None => userdata_impl_idiv_f32(lua, &*vector2_of(&lhs)?, rhs),
            }
        });
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

fn vector2_of(value: &LuaValue) -> LuaResult<LuaUserDataRef<Vector2>> {
    match value {
        LuaValue::UserData(ud) => ud.borrow::<Vector2>(),
        _ => Err(LuaError::runtime(format!("Vector2 expected, got {}", value.type_name()))),
    }
}

/// `(number, vector)` when the number is the left operand and the Vector2 the right.
fn number_first(lhs: &LuaValue, rhs: &LuaValue) -> LuaResult<Option<(f32, Vector2)>> {
    let n = match lhs {
        LuaValue::Number(n) => *n as f32,
        LuaValue::Integer(i) => *i as f32,
        _ => return Ok(None),
    };
    Ok(Some((n, *vector2_of(rhs)?)))
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
