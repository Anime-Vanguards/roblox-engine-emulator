use core::fmt;
use std::ops;

use glam::Vec3;
use mlua::prelude::*;
use rbx_dom_weak::types::Vector3 as DomVector3;

use lune_utils::TableBuilder;

use crate::{datatypes::util::round_float_decimal, exports::LuaExportsTable};

use super::{super::*, EnumItem};

/**
    An implementation of the [Vector3](https://create.roblox.com/docs/reference/engine/datatypes/Vector3)
    Roblox datatype, backed by [`glam::Vec3`].

    This implements all documented properties, methods &
    constructors of the Vector3 class as of October 2025.

    Note that this does not use native Luau vectors to simplify implementation
    and instead allow us to implement all abovementioned APIs accurately.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector3(pub Vec3);

/**
    A Vector3 argument, as Roblox scripts hold one: Luau's native vector, or a [`Vector3`] userdata.
*/
#[derive(Debug, Clone, Copy)]
pub struct VectorArg(pub Vec3);

impl FromLua for VectorArg {
    fn from_lua(value: LuaValue, _: &Lua) -> LuaResult<Self> {
        match &value {
            LuaValue::Vector(v) => Ok(Self(Vec3::new(v.x(), v.y(), v.z()))),
            LuaValue::UserData(ud) => ud
                .borrow::<Vector3>()
                .map(|v| Self(v.0))
                .map_err(|_| Self::mismatch(&value)),
            _ => Err(Self::mismatch(&value)),
        }
    }
}

impl VectorArg {
    fn mismatch(value: &LuaValue) -> LuaError {
        LuaError::FromLuaConversionError {
            // (Luau has one number type: an integral value is a number to a script)
            from: if matches!(value, LuaValue::Integer(_)) {
                "number"
            } else {
                value.type_name()
            },
            to: "userdata".to_string(),
            message: Some("expected userdata of type 'Vector3'".to_string()),
        }
    }
}

/**
    A vector as Luau's native vector, the value scripts see as a Vector3.
*/
pub fn native_vector(v: Vec3) -> LuaValue {
    LuaValue::Vector(mlua::Vector::new(v.x, v.y, v.z))
}

unsafe extern "C-unwind" {
    fn lua_workhostcalls(state: *mut mlua::ffi::lua_State, calls: std::ffi::c_int);
}

/**
    Counts `calls` host calls for the script that called the running native function: the work clock
    charges a script for each call into the host's own Luau (the patched VM's `lua_workhostcalls`), and a
    datatype operation served natively costs it what the Luau that served it did.
*/
pub fn charge_host_calls(lua: &Lua, calls: i32) {
    unsafe { lua_workhostcalls(lua.current_thread().state(), calls) }
}

unsafe extern "C-unwind" {
    fn lua_workhostnanos(state: *mut mlua::ffi::lua_State, nanos: std::ffi::c_int);
}

/**
    Charges `nanos` nanoseconds to the script that called the running native function: what Studio's
    operation costs beyond the instructions that reached it (the call, the loads, the loop around it),
    which the work clock counts on their own. Negative when Studio spends less on the whole operation
    than those instructions count. Priced per operation in `price` below.
*/
pub fn charge_host_nanos(lua: &Lua, nanos: i32) {
    if nanos != 0 {
        unsafe { lua_workhostnanos(lua.current_thread().state(), nanos) }
    }
}

/// What Studio spends on the other datatypes' operations that cost more than a call (CFrame's are in `cframe::price`):
/// the lobby playtest client's median of four runs of an op_cost probe less what ree counts for the
/// loop without a price. The other datatype rows are within 35 % of the client's number without one (Vector2 / UDim /
/// NumberRange / Ray constructors, Color3.new / fromRGB / fromHSV, UDim2.new, ToHSV), +-15 ns of noise.
pub mod datatype_price {
    pub const COLOR3_LERP: i32 = 21;
    pub const UDIM2_FROM_OFFSET: i32 = 20;
    pub const UDIM2_LERP: i32 = 17;
    pub const COLOR_SEQUENCE_NEW: i32 = 50;
    pub const NUMBER_SEQUENCE_NEW: i32 = 59;
}

impl LuaExportsTable for Vector3 {
    const EXPORT_NAME: &'static str = "Vector3";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        let vector3_from_axis = |_: &Lua, normal_id: LuaUserDataRef<EnumItem>| {
            if normal_id.parent.desc.name == "Axis" {
                Ok(match normal_id.name.as_str() {
                    "X" => Vector3(Vec3::X),
                    "Y" => Vector3(Vec3::Y),
                    "Z" => Vector3(Vec3::Z),
                    name => {
                        return Err(LuaError::RuntimeError(format!(
                            "Axis '{name}' is not known",
                        )));
                    }
                })
            } else {
                Err(LuaError::RuntimeError(format!(
                    "EnumItem must be a Axis, got {}",
                    normal_id.parent.desc.name
                )))
            }
        };

        let vector3_from_normal_id = |_: &Lua, normal_id: LuaUserDataRef<EnumItem>| {
            if normal_id.parent.desc.name == "NormalId" {
                Ok(match normal_id.name.as_str() {
                    "Left" => Vector3(Vec3::X),
                    "Top" => Vector3(Vec3::Y),
                    "Front" => Vector3(-Vec3::Z),
                    "Right" => Vector3(-Vec3::X),
                    "Bottom" => Vector3(-Vec3::Y),
                    "Back" => Vector3(Vec3::Z),
                    name => {
                        return Err(LuaError::RuntimeError(format!(
                            "NormalId '{name}' is not known",
                        )));
                    }
                })
            } else {
                Err(LuaError::RuntimeError(format!(
                    "EnumItem must be a NormalId, got {}",
                    normal_id.parent.desc.name
                )))
            }
        };

        let vector3_new = |_: &Lua, (x, y, z): (Option<f32>, Option<f32>, Option<f32>)| {
            Ok(Vector3(Vec3 {
                x: x.unwrap_or_default(),
                y: y.unwrap_or_default(),
                z: z.unwrap_or_default(),
            }))
        };

        TableBuilder::new(lua)?
            .with_value("xAxis", Vector3(Vec3::X))?
            .with_value("yAxis", Vector3(Vec3::Y))?
            .with_value("zAxis", Vector3(Vec3::Z))?
            .with_value("zero", Vector3(Vec3::ZERO))?
            .with_value("one", Vector3(Vec3::ONE))?
            .with_function("fromAxis", vector3_from_axis)?
            .with_function("fromNormalId", vector3_from_normal_id)?
            .with_function("new", vector3_new)?
            .build_readonly()
    }
}

impl LuaUserData for Vector3 {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("Magnitude", |_, this| Ok(this.0.length()));
        fields.add_field_method_get("Unit", |_, this| Ok(Vector3(this.0.normalize())));
        fields.add_field_method_get("X", |_, this| Ok(this.0.x));
        fields.add_field_method_get("Y", |_, this| Ok(this.0.y));
        fields.add_field_method_get("Z", |_, this| Ok(this.0.z));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Methods
        methods.add_method("Angle", |_, this, rhs: LuaUserDataRef<Vector3>| {
            Ok(this.0.angle_between(rhs.0))
        });
        methods.add_method("Cross", |_, this, rhs: LuaUserDataRef<Vector3>| {
            Ok(Vector3(this.0.cross(rhs.0)))
        });
        methods.add_method("Dot", |_, this, rhs: LuaUserDataRef<Vector3>| {
            Ok(this.0.dot(rhs.0))
        });
        methods.add_method(
            "FuzzyEq",
            |_, this, (rhs, epsilon): (LuaUserDataRef<Vector3>, f32)| {
                let eq_x = (rhs.0.x - this.0.x).abs() <= epsilon;
                let eq_y = (rhs.0.y - this.0.y).abs() <= epsilon;
                let eq_z = (rhs.0.z - this.0.z).abs() <= epsilon;
                Ok(eq_x && eq_y && eq_z)
            },
        );
        methods.add_method(
            "Lerp",
            |_, this, (rhs, alpha): (LuaUserDataRef<Vector3>, f32)| {
                Ok(Vector3(this.0.lerp(rhs.0, alpha)))
            },
        );
        methods.add_method("Max", |_, this, rhs: LuaUserDataRef<Vector3>| {
            Ok(Vector3(this.0.max(rhs.0)))
        });
        methods.add_method("Min", |_, this, rhs: LuaUserDataRef<Vector3>| {
            Ok(Vector3(this.0.min(rhs.0)))
        });
        methods.add_method("Abs", |_, this, ()| Ok(Vector3(this.0.abs())));
        methods.add_method("Ceil", |_, this, ()| Ok(Vector3(this.0.ceil())));
        methods.add_method("Floor", |_, this, ()| Ok(Vector3(this.0.floor())));
        methods.add_method("Sign", |_, this, ()| Ok(Vector3(this.0.signum())));
        // Metamethods
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
        methods.add_meta_method(LuaMetaMethod::Unm, userdata_impl_unm);
        methods.add_meta_method(LuaMetaMethod::Add, userdata_impl_add);
        methods.add_meta_method(LuaMetaMethod::Sub, userdata_impl_sub);
        methods.add_meta_method(LuaMetaMethod::Mul, userdata_impl_mul_f32);
        methods.add_meta_method(LuaMetaMethod::Div, userdata_impl_div_f32);
        methods.add_meta_method(LuaMetaMethod::IDiv, userdata_impl_idiv_f32);
    }
}

impl fmt::Display for Vector3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}, {}", self.0.x, self.0.y, self.0.z)
    }
}

impl ops::Neg for Vector3 {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Vector3(-self.0)
    }
}

impl ops::Add for Vector3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Vector3(self.0 + rhs.0)
    }
}

impl ops::Sub for Vector3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Vector3(self.0 - rhs.0)
    }
}

impl ops::Mul for Vector3 {
    type Output = Vector3;
    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl ops::Mul<f32> for Vector3 {
    type Output = Vector3;
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl ops::Div for Vector3 {
    type Output = Vector3;
    fn div(self, rhs: Self) -> Self::Output {
        Self(self.0 / rhs.0)
    }
}

impl ops::Div<f32> for Vector3 {
    type Output = Vector3;
    fn div(self, rhs: f32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl IDiv for Vector3 {
    type Output = Vector3;
    fn idiv(self, rhs: Self) -> Self::Output {
        Self((self.0 / rhs.0).floor())
    }
}

impl IDiv<f32> for Vector3 {
    type Output = Vector3;
    fn idiv(self, rhs: f32) -> Self::Output {
        Self((self.0 / rhs).floor())
    }
}

impl From<DomVector3> for Vector3 {
    fn from(v: DomVector3) -> Self {
        Vector3(Vec3 {
            x: v.x,
            y: v.y,
            z: v.z,
        })
    }
}

impl From<Vector3> for DomVector3 {
    fn from(v: Vector3) -> Self {
        DomVector3 {
            x: round_float_decimal(v.0.x),
            y: round_float_decimal(v.0.y),
            z: round_float_decimal(v.0.z),
        }
    }
}
