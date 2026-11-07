#![allow(clippy::items_after_statements)]

use core::fmt;
use std::ops;

use glam::Vec2;
use mlua::prelude::*;
use rbx_dom_weak::types::UDim2 as DomUDim2;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, UDim};

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
        methods.add_method(
            "Lerp",
            |lua, this, (goal, alpha): (LuaUserDataRef<UDim2>, f32)| {
                super::charge_host_nanos(lua, super::datatype_price::UDIM2_LERP);
                let this_x = Vec2::new(this.x.scale, this.x.offset as f32);
                let goal_x = Vec2::new(goal.x.scale, goal.x.offset as f32);

                let this_y = Vec2::new(this.y.scale, this.y.offset as f32);
                let goal_y = Vec2::new(goal.y.scale, goal.y.offset as f32);

                let x = this_x.lerp(goal_x, alpha);
                let y = this_y.lerp(goal_y, alpha);

                Ok(UDim2 {
                    x: UDim {
                        scale: x.x,
                        offset: x.y.clamp(i32::MIN as f32, i32::MAX as f32).round() as i32,
                    },
                    y: UDim {
                        scale: y.x,
                        offset: y.y.clamp(i32::MIN as f32, i32::MAX as f32).round() as i32,
                    },
                })
            },
        );
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
        methods.add_meta_method(LuaMetaMethod::Add, |lua, this, rhs: LuaUserDataRef<UDim2>| {
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_ARITHMETIC);
            userdata_impl_add(lua, this, rhs)
        });
        methods.add_meta_method(LuaMetaMethod::Sub, |lua, this, rhs: LuaUserDataRef<UDim2>| {
            super::charge_host_nanos(lua, super::datatype_price::UDIM2_ARITHMETIC);
            userdata_impl_sub(lua, this, rhs)
        });
    }
}

impl fmt::Display for UDim2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.x, self.y)
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
