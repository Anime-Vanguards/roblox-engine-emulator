#![allow(clippy::items_after_statements)]

use core::fmt;
use std::ops;

use mlua::prelude::*;
use rbx_dom_weak::types::UDim2 as DomUDim2;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, UDim};
use crate::datatypes::script_error::{reflected_arg, reflected_float_arg, same_operands};

/**
    An implementation of the [UDim2](https://create.roblox.com/docs/reference/engine/datatypes/UDim2) Roblox datatype.

    This implements all documented properties, methods & constructors of the `UDim2` class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UDim2 {
    pub(crate) x: UDim,
    pub(crate) y: UDim,
}

impl LuaExportsTable for UDim2 {
    const EXPORT_NAME: &'static str = "UDim2";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        let udim2_from_offset = |lua: &Lua, (x, y): (Option<i32>, Option<i32>)| {
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_NEW);
            Ok(UDim2 {
                x: UDim::new(0f32, x.unwrap_or_default()),
                y: UDim::new(0f32, y.unwrap_or_default()),
            })
        };

        let udim2_from_scale = |lua: &Lua, (x, y): (Option<f32>, Option<f32>)| {
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_NEW);
            Ok(UDim2 {
                x: UDim::new(x.unwrap_or_default(), 0),
                y: UDim::new(y.unwrap_or_default(), 0),
            })
        };

        type ArgsUDims = (Option<LuaUserDataRef<UDim>>, Option<LuaUserDataRef<UDim>>);
        type ArgsNums = (Option<f32>, Option<i32>, Option<f32>, Option<i32>);
        let udim2_new = |lua: &Lua, args: LuaMultiValue| {
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_NEW);
            if let Ok((x, y)) = ArgsUDims::from_lua_multi(args.clone(), lua) {
                Ok(UDim2 {
                    x: x.map(|x| *x).unwrap_or_default(),
                    y: y.map(|y| *y).unwrap_or_default(),
                })
            } else if let Ok((sx, ox, sy, oy)) = ArgsNums::from_lua_multi(args, lua) {
                Ok(UDim2 {
                    x: UDim::new(sx.unwrap_or_default(), ox.unwrap_or_default()),
                    y: UDim::new(sy.unwrap_or_default(), oy.unwrap_or_default()),
                })
            } else {
                // FUTURE: Better error message here using given arg types
                Err(LuaError::RuntimeError(
                    "Invalid arguments to constructor".to_string(),
                ))
            }
        };

        TableBuilder::new(lua)?
            .with_function("fromOffset", udim2_from_offset)?
            .with_function("fromScale", udim2_from_scale)?
            .with_function("new", udim2_new)?
            .build_readonly()
    }
}

impl LuaUserData for UDim2 {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::with_methods("UDim2"));
        // (each makes a new UDim: datatype_price::MEMBER_NEW)
        fields.add_field_method_get("X", |lua, this| {
            super::charge_host_nanos(lua, super::datatype_price::MEMBER_NEW);
            Ok(this.x)
        });
        fields.add_field_method_get("Y", |lua, this| {
            super::charge_host_nanos(lua, super::datatype_price::MEMBER_NEW);
            Ok(this.y)
        });
        fields.add_field_method_get("Width", |lua, this| {
            super::charge_host_nanos(lua, super::datatype_price::MEMBER_NEW);
            Ok(this.x)
        });
        fields.add_field_method_get("Height", |lua, this| {
            super::charge_host_nanos(lua, super::datatype_price::MEMBER_NEW);
            Ok(this.y)
        });
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Methods
        // (and Roblox's deprecated lowercase name, measured in a client: ree's tools/parity/probes/camelcase.luau)
        // Each scale is a + (b - a) * t in floats, each offset the same rounded half away from zero (Studio 0.744,
        // tools/parity/probes/datatypes.luau)
        // (a playing client's reflection checks self, then the alpha, then the goal: "Argument 3 missing or nil", "Unable
        // to cast double to UDim2"; Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let lerp = |lua: &Lua, args: LuaMultiValue| {
            let this = reflected_arg::<UDim2>(&args, 1, "UDim2")?;
            let alpha = reflected_float_arg(lua, &args, 3)? as f32;
            let goal = reflected_arg::<UDim2>(&args, 2, "UDim2")?;
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_LERP);
            let lerp = super::float32::lerp;
            let offset = |a: i32, b: i32| lerp(a as f32, b as f32, alpha).round().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
            Ok(UDim2 {
                x: UDim { scale: lerp(this.x.scale, goal.x.scale, alpha), offset: offset(this.x.offset, goal.x.offset) },
                y: UDim { scale: lerp(this.y.scale, goal.y.scale, alpha), offset: offset(this.y.offset, goal.y.offset) },
            })
        };
        methods.add_function("Lerp", lerp);
        methods.add_function("lerp", lerp);
        // Metamethods
        methods.add_meta_method(LuaMetaMethod::Eq, |lua, this, value: LuaValue| {
            super::charge_host_nanos(lua, super::datatype_price::EQ);
            userdata_impl_eq(lua, this, value)
        });
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
        methods.add_meta_method(LuaMetaMethod::Unm, |lua, this, ()| {
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_ARITHMETIC);
            userdata_impl_unm(lua, this, ())
        });
        // (two UDim2s, whichever side, as UDim's)
        methods.add_meta_function(LuaMetaMethod::Add, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = same_operands::<UDim2>(&lhs, &rhs, "UDim2")?;
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_ARITHMETIC);
            Ok(a + b)
        });
        methods.add_meta_function(LuaMetaMethod::Sub, |lua, (lhs, rhs): (LuaValue, LuaValue)| {
            let (a, b) = same_operands::<UDim2>(&lhs, &rhs, "UDim2")?;
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_ARITHMETIC);
            Ok(a - b)
        });
    }
}

impl fmt::Display for UDim2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // "{0.333333343, 7}, {-2.5, -9}" (Studio 0.744)
        write!(f, "{{{}}}, {{{}}}", self.x, self.y)
    }
}

impl ops::Neg for UDim2 {
    type Output = Self;
    fn neg(self) -> Self::Output {
        UDim2 {
            x: -self.x,
            y: -self.y,
        }
    }
}

impl ops::Add for UDim2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        UDim2 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl ops::Sub for UDim2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        UDim2 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl From<DomUDim2> for UDim2 {
    fn from(v: DomUDim2) -> Self {
        UDim2 {
            x: v.x.into(),
            y: v.y.into(),
        }
    }
}

impl From<UDim2> for DomUDim2 {
    fn from(v: UDim2) -> Self {
        DomUDim2 {
            x: v.x.into(),
            y: v.y.into(),
        }
    }
}
