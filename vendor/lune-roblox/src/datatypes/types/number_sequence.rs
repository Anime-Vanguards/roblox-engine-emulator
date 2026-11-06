use core::fmt;

use mlua::prelude::*;
use rbx_dom_weak::types::{
    NumberSequence as DomNumberSequence, NumberSequenceKeypoint as DomNumberSequenceKeypoint,
};

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, NumberSequenceKeypoint};

/**
    An implementation of the [NumberSequence](https://create.roblox.com/docs/reference/engine/datatypes/NumberSequence) Roblox datatype.

    This implements all documented properties, methods & constructors of the `NumberSequence` class as of October 2025.
*/
#[derive(Debug, Clone, PartialEq)]
pub struct NumberSequence {
    pub(crate) keypoints: Vec<NumberSequenceKeypoint>,
}

/// Roblox's keypoint limit for NumberSequence and ColorSequence ("a maximum of 20 keypoints").
pub(crate) const MAX_SEQUENCE_KEYPOINTS: usize = 20;

/// How far the first and last times may be from 0 and 1 (not measured in Studio; rbxmk's tolerance).
const SEQUENCE_END_TOLERANCE: f32 = 1e-4;

/// The keypoints of a sequence constructor's table, read as Roblox reads them: at most 20, each a
/// `T` (the messages are Studio's: "ColorSequence.new(): table is too long.",
/// "NumberSequence.new(): table of NumberSequenceKeypoints expected.").
pub(crate) fn sequence_keypoints<T: LuaUserData + Copy + 'static>(
    lua: &Lua,
    table: &LuaTable,
    kind: &str,
) -> LuaResult<Vec<T>> {
    let count = table.raw_len();
    if count > MAX_SEQUENCE_KEYPOINTS {
        return Err(LuaError::RuntimeError(format!(
            "{kind}.new(): table is too long."
        )));
    }
    let mut keypoints = Vec::with_capacity(count);
    for index in 1..=count {
        let value: LuaValue = table.raw_get(index)?;
        match LuaUserDataRef::<T>::from_lua(value, lua) {
            Ok(keypoint) => keypoints.push(*keypoint),
            Err(_) => {
                return Err(LuaError::RuntimeError(format!(
                    "{kind}.new(): table of {kind}Keypoints expected."
                )));
            }
        }
    }
    Ok(keypoints)
}

/// The rules a sequence's keypoint times follow: at least two, in non-descending order, the last at
/// time 1 and the first at time 0. Studio's messages ("NumberSequence: requires at least 2
/// keypoints", "...: all keypoints must be ordered by time", "ColorSequence must end at time=1.0");
/// the start message and the order of the two end checks are not measured.
pub(crate) fn check_sequence_times(kind: &str, times: &[f32]) -> LuaResult<()> {
    let fail = |message: String| Err(LuaError::RuntimeError(message));
    if times.len() < 2 {
        return fail(format!("{kind}: requires at least 2 keypoints"));
    }
    if times.windows(2).any(|pair| pair[1] < pair[0]) {
        return fail(format!("{kind}: all keypoints must be ordered by time"));
    }
    if (times[times.len() - 1] - 1.0).abs() > SEQUENCE_END_TOLERANCE {
        return fail(format!("{kind} must end at time=1.0"));
    }
    if times[0].abs() > SEQUENCE_END_TOLERANCE {
        return fail(format!("{kind} must start at time=0.0"));
    }
    Ok(())
}

impl LuaExportsTable for NumberSequence {
    const EXPORT_NAME: &'static str = "NumberSequence";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        type ArgsColor = f32;
        type ArgsColors = (f32, f32);

        let number_sequence_new = |lua: &Lua, args: LuaMultiValue| {
            super::charge_host_nanos(lua, super::datatype_price::NUMBER_SEQUENCE_NEW);
            if let Ok(value) = ArgsColor::from_lua_multi(args.clone(), lua) {
                Ok(NumberSequence {
                    keypoints: vec![
                        NumberSequenceKeypoint {
                            time: 0.0,
                            value,
                            envelope: 0.0,
                        },
                        NumberSequenceKeypoint {
                            time: 1.0,
                            value,
                            envelope: 0.0,
                        },
                    ],
                })
            } else if let Ok((v0, v1)) = ArgsColors::from_lua_multi(args.clone(), lua) {
                Ok(NumberSequence {
                    keypoints: vec![
                        NumberSequenceKeypoint {
                            time: 0.0,
                            value: v0,
                            envelope: 0.0,
                        },
                        NumberSequenceKeypoint {
                            time: 1.0,
                            value: v1,
                            envelope: 0.0,
                        },
                    ],
                })
            } else if let Some(LuaValue::Table(table)) = args.front() {
                let mut keypoints: Vec<NumberSequenceKeypoint> =
                    sequence_keypoints(lua, table, "NumberSequence")?;
                let times: Vec<f32> = keypoints.iter().map(|k| k.time).collect();
                check_sequence_times("NumberSequence", &times)?;
                let last = keypoints.len() - 1;
                keypoints[0].time = 0.0;
                keypoints[last].time = 1.0;
                Ok(NumberSequence { keypoints })
            } else {
                Err(LuaError::RuntimeError(
                    "NumberSequence.new(): table of NumberSequenceKeypoints expected.".to_string(),
                ))
            }
        };

        TableBuilder::new(lua)?
            .with_function("new", number_sequence_new)?
            .build_readonly()
    }
}

impl LuaUserData for NumberSequence {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("Keypoints", |_, this| Ok(this.keypoints.clone()));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for NumberSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, keypoint) in self.keypoints.iter().enumerate() {
            if index < self.keypoints.len() - 1 {
                write!(f, "{keypoint}, ")?;
            } else {
                write!(f, "{keypoint}")?;
            }
        }
        Ok(())
    }
}

impl From<DomNumberSequence> for NumberSequence {
    fn from(v: DomNumberSequence) -> Self {
        Self {
            keypoints: v
                .keypoints
                .iter()
                .copied()
                .map(NumberSequenceKeypoint::from)
                .collect(),
        }
    }
}

impl From<NumberSequence> for DomNumberSequence {
    fn from(v: NumberSequence) -> Self {
        Self {
            keypoints: v
                .keypoints
                .iter()
                .copied()
                .map(DomNumberSequenceKeypoint::from)
                .collect(),
        }
    }
}
