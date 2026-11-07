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

/// What a client spends on the other datatypes' operations (CFrame's are in `cframe::price`) beyond what the work clock counts
/// for the instructions that reach them (the loads, the call, the loop around them), in nanoseconds: the parity bench
/// (tools/cost-parity, a rendered client of a game's match place, nine sessions of the whole corpus, the median of 9 rounds each),
/// each price fitted on the rows that are not held out. A value the operation makes (a new UDim, Vector2 or keypoint) is the engine's
/// allocation, which a script's own instructions do not count here, and the rows that make one vary 2-3x between sessions with the
/// collector's phase (CFrame:Inverse 18-65 ns): the figures are the sessions' medians. Rows without a price read within 25 % without one.
pub mod datatype_price {
    /// `+ - * / //` and unary `-` on a Vector2 or UDim, which make a new value (Vector2 + Vector2 15.3 ns whole and unary - 14.9,
    /// against the 1 ns the instruction counts; held out: Vector2 - * / 13.5-17.4, UDim + 23.0), and on a UDim2 (- 28.0; held out: + 29.6)
    pub const ARITHMETIC: i32 = 15;
    pub const UDIM2_ARITHMETIC: i32 = 27;
    /// `==` / `~=` of two Vector2, UDim, UDim2 or Color3 values, a metamethod call where the VM counts only the instruction
    /// (Vector2 9.8 ns whole, Color3 10.3; held out: UDim2 10.0)
    pub const EQ: i32 = 9;
    /// A member that makes a new value: a ColorSequenceKeypoint's Value (a Color3), a Vector2's Unit, a UDim2's X / Y / Width / Height
    /// (a UDim): Value 18.5 ns whole, Unit 18.8, UDim2.Y.Scale 21.4 and X.Offset 32.3, where the read counts 2.9 and a UDim's own Scale
    /// or Offset is 3.3 like any number member (held out: UDim2.X 16.4, Width 17.3)
    pub const MEMBER_NEW: i32 = 16;
    /// Vector2:Lerp (20.3 ns whole, a new Vector2) and a Vector2's Magnitude (3.9 ns, against an X read's 3.4)
    pub const VECTOR2_LERP: i32 = 9;
    pub const VECTOR2_MAGNITUDE: i32 = 1;
    /// Color3:Lerp 29.4 ns whole; Color3.new 17.1 and fromHSV 28.0, near what the call counts here (23.2 and 28.6)
    pub const COLOR3_LERP: i32 = 18;
    pub const COLOR3_NEW: i32 = -3;
    /// UDim2.new, fromScale and fromOffset 30.4-32.1 ns whole, UDim2:Lerp 39.9, UDim.new 16.8, Vector2.new 16.9 (the call counts 22.7)
    pub const UDIM2_NEW: i32 = 8;
    pub const UDIM2_LERP: i32 = 29;
    pub const UDIM_NEW: i32 = -6;
    pub const VECTOR2_NEW: i32 = -6;
    /// ColorSequence.new of a table of keypoints, by keypoint (five made before the loop 107.9 ns whole; the prelude makes one or two
    /// colours a table of two: two colours 39.5 ns whole, held out: one colour 70.8), and of colours given to it directly
    pub const COLOR_SEQUENCE_NEW: i32 = 17;
    pub const COLOR_SEQUENCE_TABLE: i32 = 0;
    pub const COLOR_SEQUENCE_TABLE_KEYPOINT: i32 = 16;
    /// NumberSequence.new of a table of keypoints, which the prelude makes of one or two numbers too (one 63.4 ns whole and two 30.3:
    /// the collector's phase moves them apart, so the price is their middle), and of numbers given to it directly
    pub const NUMBER_SEQUENCE_NEW: i32 = 12;
    pub const NUMBER_SEQUENCE_TABLE: i32 = 28;
    /// ColorSequenceKeypoint.new and NumberSequenceKeypoint.new (18.1 ns whole each, below what the call counts here: 23.2)
    pub const KEYPOINT_NEW: i32 = -5;
    /// A sequence's Keypoints, a new table of new keypoints on every read, by keypoint (ColorSequence of five 145.1 ns whole, of two
    /// 66.0; NumberSequence of five 105.5, of two 60.5)
    pub const COLOR_KEYPOINTS: (i32, i32) = (10, 26);
    pub const NUMBER_KEYPOINTS: (i32, i32) = (28, 15);

    /// What a read of a sequence's Keypoints costs beyond the read, for `count` keypoints at `(base, per keypoint)`
    pub fn keypoints_read(price: (i32, i32), count: usize) -> i32 {
        price.0 + price.1 * i32::try_from(count).unwrap_or(i32::MAX / 64)
    }
    /// BrickColor.new of a name (48.5 ns whole)
    pub const BRICK_COLOR_NEW: i32 = 26;
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
