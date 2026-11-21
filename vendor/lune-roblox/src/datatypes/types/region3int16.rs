use core::fmt;

use glam::IVec3;
use mlua::prelude::*;
use rbx_dom_weak::types::Region3int16 as DomRegion3int16;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, Vector3int16};
use crate::datatypes::script_error::optional_ctor_arg;

/**
    An implementation of the [Region3int16](https://create.roblox.com/docs/reference/engine/datatypes/Region3int16)
    Roblox datatype, backed by [`glam::IVec3`].

    This implements all documented properties, methods & constructors of the Region3int16 class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region3int16 {
    pub(crate) min: IVec3,
    pub(crate) max: IVec3,
}

impl LuaExportsTable for Region3int16 {
    const EXPORT_NAME: &'static str = "Region3int16";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        // (two Vector3int16s checked as a client's Luau checks them, naming the constructor; one not passed is zero:
        // Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let region3int16_new = |_: &Lua, args: LuaMultiValue| {
            let min = optional_ctor_arg::<Vector3int16>(&args, 1, "new", "Vector3int16")?.map_or(Default::default(), |v| v.0);
            let max = optional_ctor_arg::<Vector3int16>(&args, 2, "new", "Vector3int16")?.map_or(Default::default(), |v| v.0);
            Ok(Region3int16 { min, max })
        };

        TableBuilder::new(lua)?
            .with_function("new", region3int16_new)?
            .build_readonly()
    }
}

impl LuaUserData for Region3int16 {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::without_methods("Region3int16"));
        fields.add_field_method_get("Min", |_, this| Ok(Vector3int16(this.min)));
        fields.add_field_method_get("Max", |_, this| Ok(Vector3int16(this.max)));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for Region3int16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // "min; max" ("-1, 2, 3; 4, 5, 6"; Studio 0.744)
        write!(f, "{}; {}", Vector3int16(self.min), Vector3int16(self.max))
    }
}

impl From<DomRegion3int16> for Region3int16 {
    fn from(v: DomRegion3int16) -> Self {
        Region3int16 {
            min: Vector3int16::from(v.min).0,
            max: Vector3int16::from(v.max).0,
        }
    }
}

impl From<Region3int16> for DomRegion3int16 {
    fn from(v: Region3int16) -> Self {
        DomRegion3int16 {
            min: Vector3int16(v.min).into(),
            max: Vector3int16(v.max).into(),
        }
    }
}
