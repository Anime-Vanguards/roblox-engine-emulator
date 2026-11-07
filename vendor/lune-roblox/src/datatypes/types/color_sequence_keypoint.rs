use core::fmt;

use mlua::prelude::*;
use rbx_dom_weak::types::ColorSequenceKeypoint as DomColorSequenceKeypoint;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, Color3};

/**
    An implementation of the [ColorSequenceKeypoint](https://create.roblox.com/docs/reference/engine/datatypes/ColorSequenceKeypoint) Roblox datatype.

    This implements all documented properties, methods & constructors of the `ColorSequenceKeypoint` class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorSequenceKeypoint {
    pub(crate) time: f32,
    pub(crate) color: Color3,
}

impl LuaExportsTable for ColorSequenceKeypoint {
    const EXPORT_NAME: &'static str = "ColorSequenceKeypoint";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        let color_sequence_keypoint_new =
            |lua: &Lua, (time, color): (f32, LuaUserDataRef<Color3>)| {
                super::charge_host_nanos(lua, super::datatype_price::KEYPOINT_NEW);
                Ok(ColorSequenceKeypoint {
                    time,
                    color: *color,
                })
            };

        TableBuilder::new(lua)?
            .with_function("new", color_sequence_keypoint_new)?
            .build_readonly()
    }
}

impl LuaUserData for ColorSequenceKeypoint {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("Time", |_, this| Ok(this.time));
        // (a new Color3: datatype_price::MEMBER_NEW)
        fields.add_field_method_get("Value", |lua, this| {
            super::charge_host_nanos(lua, super::datatype_price::MEMBER_NEW);
            Ok(this.color)
        });
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for ColorSequenceKeypoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // "time r g b 0 " with 6 significant digits, an envelope of 0 and a trailing space (Studio 0.744)
        let g = |value: f32| super::format_g(f64::from(value), 6);
        write!(f, "{} {} {} {} 0 ", g(self.time), g(self.color.r), g(self.color.g), g(self.color.b))
    }
}

impl From<DomColorSequenceKeypoint> for ColorSequenceKeypoint {
    fn from(v: DomColorSequenceKeypoint) -> Self {
        Self {
            time: v.time,
            color: v.color.into(),
        }
    }
}

impl From<ColorSequenceKeypoint> for DomColorSequenceKeypoint {
    fn from(v: ColorSequenceKeypoint) -> Self {
        Self {
            time: v.time,
            color: v.color.into(),
        }
    }
}
