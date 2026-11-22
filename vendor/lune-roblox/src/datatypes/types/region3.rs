use core::fmt;

use glam::{Mat4, Vec3};
use mlua::prelude::*;
use rbx_dom_weak::types::Region3 as DomRegion3;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, CFrame, Vector3, VectorArg, native_vector};
use crate::datatypes::script_error::{invalid_argument, reflected_arg, reflected_float_arg, MethodMembers};

/**
    An implementation of the [Region3](https://create.roblox.com/docs/reference/engine/datatypes/Region3)
    Roblox datatype, backed by [`glam::Vec3`].

    This implements all documented properties, methods & constructors of the Region3 class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region3 {
    pub(crate) min: Vec3,
    pub(crate) max: Vec3,
}

impl LuaExportsTable for Region3 {
    const EXPORT_NAME: &'static str = "Region3";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        // (two Vector3s checked as a client's Luau checks them, naming the constructor; one not passed is the zero vector:
        // Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let region3_new = |lua: &Lua, args: LuaMultiValue| {
            let vector_at = |index: usize| -> LuaResult<Vec3> {
                match args.get(index - 1) {
                    None => Ok(Vec3::ZERO),
                    Some(value) => VectorArg::from_lua(value.clone(), lua)
                        .map(|v| v.0)
                        .map_err(|_| invalid_argument(index, Some("new"), "Vector3", value)),
                }
            };
            let min = vector_at(1)?;
            let max = vector_at(2)?;
            Ok(Region3 { min, max })
        };

        TableBuilder::new(lua)?
            .with_function("new", region3_new)?
            .build_readonly()
    }
}

impl LuaUserData for Region3 {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (an assignment raises as a client's does: datatypes::script_error::AssignMembers)
        fields.add_meta_field("__newindex", crate::datatypes::script_error::AssignMembers("Region3"));
        // (a member it lacks raises as a client's does, and `:` resolves methods only: datatypes::script_error::MethodMembers)
        let members = MethodMembers::new("Region3");
        fields.add_meta_field("__index", members.index());
        fields.add_meta_field("__namecall", members.namecall());
        fields.add_field_method_get("CFrame", |_, this| {
            Ok(CFrame(Mat4::from_translation(this.min.lerp(this.max, 0.5))))
        });
        fields.add_field_method_get("Size", |_, this| Ok(native_vector(this.max - this.min)));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Methods
        // (a playing client's reflection checks the resolution, then self: "Argument 2 missing or nil", "Unable to cast
        // string to float", "Unable to cast double to Region3"; Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        methods.add_function("ExpandToGrid", |lua, args: LuaMultiValue| {
            let resolution = reflected_float_arg(lua, &args, 2)? as f32;
            let this = reflected_arg::<Region3>(&args, 1, "Region3")?;
            Ok(Region3 {
                min: (this.min / resolution).floor() * resolution,
                max: (this.max / resolution).ceil() * resolution,
            })
        });
        // Metamethods
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for Region3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // its CFrame (the centre, (min + max) * 0.5 in floats, no rotation) and its Size (max - min), 9 significant
        // digits each: "0.333333313, -0.125, 0.0500000007, 1, 0, 0, 0, 1, 0, 0, 0, 1; 1.33333337, 4.75, 0.100000001" for
        // Region3.new((-1/3, -2.5, 0), (1, 2.25, 0.1)) (Studio 0.744)
        let g = |value: f32| super::format_g(f64::from(value), 9);
        let (center, size) = ((self.min + self.max) * 0.5, self.max - self.min);
        write!(f, "{}, {}, {}, 1, 0, 0, 0, 1, 0, 0, 0, 1; {}, {}, {}", g(center.x), g(center.y), g(center.z), g(size.x), g(size.y), g(size.z))
    }
}

impl From<DomRegion3> for Region3 {
    fn from(v: DomRegion3) -> Self {
        Region3 {
            min: Vector3::from(v.min).0,
            max: Vector3::from(v.max).0,
        }
    }
}

impl From<Region3> for DomRegion3 {
    fn from(v: Region3) -> Self {
        DomRegion3 {
            min: Vector3(v.min).into(),
            max: Vector3(v.max).into(),
        }
    }
}
