use core::fmt;

use mlua::prelude::*;
use rbx_dom_weak::types::NumberSequenceKeypoint as DomNumberSequenceKeypoint;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::super::*;
use crate::datatypes::script_error::{ctor_number, luau_number};

/**
    An implementation of the [NumberSequenceKeypoint](https://create.roblox.com/docs/reference/engine/datatypes/NumberSequenceKeypoint) Roblox datatype.

    This implements all documented properties, methods & constructors of the `NumberSequenceKeypoint` class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberSequenceKeypoint {
    pub(crate) time: f32,
    pub(crate) value: f32,
    pub(crate) envelope: f32,
}

impl LuaExportsTable for NumberSequenceKeypoint {
    const EXPORT_NAME: &'static str = "NumberSequenceKeypoint";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        // (time and value checked as a client's Luau checks them, naming the constructor; an envelope that is no number is
        // 0: Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let number_sequence_keypoint_new =
            |lua: &Lua, args: LuaMultiValue| {
                let time = ctor_number(lua, &args, 1, "new")? as f32;
                let value = ctor_number(lua, &args, 2, "new")? as f32;
                let envelope = args.get(2).and_then(|value| luau_number(lua, value)).map(|envelope| envelope as f32);
                super::charge_host_nanos(lua, super::datatype_price::KEYPOINT_NEW);
                // Roblox's documentation: "The envelope parameter must be a non-negative number"
                // (the message is not measured in Studio)
                let envelope = envelope.unwrap_or_default();
                if envelope < 0.0 {
                    return Err(LuaError::RuntimeError(
                        "NumberSequenceKeypoint: envelope must be non-negative".to_string(),
                    ));
                }
                Ok(NumberSequenceKeypoint {
                    time,
                    value,
                    envelope,
                })
            };

        TableBuilder::new(lua)?
            .with_function("new", number_sequence_keypoint_new)?
            .build_readonly()
    }
}

impl LuaUserData for NumberSequenceKeypoint {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::without_methods("NumberSequenceKeypoint"));
        fields.add_field_method_get("Time", |_, this| Ok(this.time));
        fields.add_field_method_get("Value", |_, this| Ok(this.value));
        fields.add_field_method_get("Envelope", |_, this| Ok(this.envelope));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for NumberSequenceKeypoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // "time value envelope " with 6 significant digits and a trailing space (Studio 0.744)
        let g = |value: f32| super::format_g(f64::from(value), 6);
        write!(f, "{} {} {} ", g(self.time), g(self.value), g(self.envelope))
    }
}

impl From<DomNumberSequenceKeypoint> for NumberSequenceKeypoint {
    fn from(v: DomNumberSequenceKeypoint) -> Self {
        Self {
            time: v.time,
            value: v.value,
            envelope: v.envelope,
        }
    }
}

impl From<NumberSequenceKeypoint> for DomNumberSequenceKeypoint {
    fn from(v: NumberSequenceKeypoint) -> Self {
        Self {
            time: v.time,
            value: v.value,
            envelope: v.envelope,
        }
    }
}
