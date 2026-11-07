use core::fmt;

use mlua::prelude::*;
use rbx_dom_weak::types::{
    ColorSequence as DomColorSequence, ColorSequenceKeypoint as DomColorSequenceKeypoint,
};

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{
    super::*,
    Color3, ColorSequenceKeypoint,
    number_sequence::{check_sequence_times, sequence_keypoints},
};

/**
    An implementation of the [ColorSequence](https://create.roblox.com/docs/reference/engine/datatypes/ColorSequence) Roblox datatype.

    This implements all documented properties, methods & constructors of the `ColorSequence` class as of October 2025.
*/
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSequence {
    pub(crate) keypoints: Vec<ColorSequenceKeypoint>,
}

impl LuaExportsTable for ColorSequence {
    const EXPORT_NAME: &'static str = "ColorSequence";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        type ArgsColor = LuaUserDataRef<Color3>;
        type ArgsColors = (LuaUserDataRef<Color3>, LuaUserDataRef<Color3>);

        let color_sequence_new = |lua: &Lua, args: LuaMultiValue| {
            if let Ok(color) = ArgsColor::from_lua_multi(args.clone(), lua) {
                super::charge_host_nanos(lua, super::datatype_price::COLOR_SEQUENCE_NEW);
                Ok(ColorSequence {
                    keypoints: vec![
                        ColorSequenceKeypoint {
                            time: 0.0,
                            color: *color,
                        },
                        ColorSequenceKeypoint {
                            time: 1.0,
                            color: *color,
                        },
                    ],
                })
            } else if let Ok((c0, c1)) = ArgsColors::from_lua_multi(args.clone(), lua) {
                super::charge_host_nanos(lua, super::datatype_price::COLOR_SEQUENCE_NEW);
                Ok(ColorSequence {
                    keypoints: vec![
                        ColorSequenceKeypoint {
                            time: 0.0,
                            color: *c0,
                        },
                        ColorSequenceKeypoint {
                            time: 1.0,
                            color: *c1,
                        },
                    ],
                })
            } else if let Some(LuaValue::Table(table)) = args.front() {
                let mut keypoints: Vec<ColorSequenceKeypoint> =
                    sequence_keypoints(lua, table, "ColorSequence")?;
                let count = i32::try_from(keypoints.len()).unwrap_or(i32::MAX / 64);
                super::charge_host_nanos(
                    lua,
                    super::datatype_price::COLOR_SEQUENCE_TABLE + super::datatype_price::COLOR_SEQUENCE_TABLE_KEYPOINT * count,
                );
                let times: Vec<f32> = keypoints.iter().map(|k| k.time).collect();
                check_sequence_times("ColorSequence", &times)?;
                let last = keypoints.len() - 1;
                keypoints[0].time = 0.0;
                keypoints[last].time = 1.0;
                Ok(ColorSequence { keypoints })
            } else {
                // Studio's message for a missing or non-table argument
                Err(LuaError::RuntimeError(
                    "ColorSequence.new(): table expected".to_string(),
                ))
            }
        };

        TableBuilder::new(lua)?
            .with_function("new", color_sequence_new)?
            .build_readonly()
    }
}

impl LuaUserData for ColorSequence {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (a new table of new keypoints on every read)
        fields.add_field_method_get("Keypoints", |lua, this| {
            super::charge_host_nanos(lua, super::datatype_price::keypoints_read(super::datatype_price::COLOR_KEYPOINTS, this.keypoints.len()));
            Ok(this.keypoints.clone())
        });
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for ColorSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // each keypoint as it prints alone, one after another (Studio 0.744)
        for keypoint in &self.keypoints {
            write!(f, "{keypoint}")?;
        }
        Ok(())
    }
}

impl From<DomColorSequence> for ColorSequence {
    fn from(v: DomColorSequence) -> Self {
        Self {
            keypoints: v
                .keypoints
                .iter()
                .copied()
                .map(ColorSequenceKeypoint::from)
                .collect(),
        }
    }
}

impl From<ColorSequence> for DomColorSequence {
    fn from(v: ColorSequence) -> Self {
        Self {
            keypoints: v
                .keypoints
                .iter()
                .copied()
                .map(DomColorSequenceKeypoint::from)
                .collect(),
        }
    }
}
