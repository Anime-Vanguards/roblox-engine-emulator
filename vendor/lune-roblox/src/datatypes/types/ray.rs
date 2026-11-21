use core::fmt;

use glam::Vec3;
use mlua::prelude::*;
use rbx_dom_weak::types::Ray as DomRay;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, Vector3};
use crate::datatypes::script_error::{luau_arg, luau_self, optional_ctor_arg};

/**
    An implementation of the [Ray](https://create.roblox.com/docs/reference/engine/datatypes/Ray)
    Roblox datatype, backed by [`glam::Vec3`].

    This implements all documented properties, methods & constructors of the Ray class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub(crate) origin: Vec3,
    pub(crate) direction: Vec3,
}

impl Ray {
    fn closest_point(&self, point: Vec3) -> Vec3 {
        let norm = self.direction.normalize();
        let lhs = point - self.origin;

        let dot_product = lhs.dot(norm).max(0.0);
        self.origin + norm * dot_product
    }
}

impl LuaExportsTable for Ray {
    const EXPORT_NAME: &'static str = "Ray";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        // (two Vector3s checked as a client's Luau checks them, naming the constructor; one not passed is the zero vector:
        // Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let ray_new = |_: &Lua, args: LuaMultiValue| {
            let origin = optional_ctor_arg::<Vector3>(&args, 1, "new", "Vector3")?.unwrap_or(Vector3(Vec3::ZERO));
            let direction = optional_ctor_arg::<Vector3>(&args, 2, "new", "Vector3")?.unwrap_or(Vector3(Vec3::ZERO));
            Ok(Ray {
                origin: origin.0,
                direction: direction.0,
            })
        };

        TableBuilder::new(lua)?
            .with_function("new", ray_new)?
            .build_readonly()
    }
}

impl LuaUserData for Ray {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::with_methods("Ray"));
        fields.add_field_method_get("Origin", |_, this| Ok(Vector3(this.origin)));
        fields.add_field_method_get("Direction", |_, this| Ok(Vector3(this.direction)));
        // (and Roblox's deprecated lowercase name, measured in a client: ree's tools/parity/probes/camelcase.luau)
        for name in ["Unit", "unit"] {
            fields.add_field_method_get(name, |_, this| {
                Ok(Ray {
                    origin: this.origin,
                    direction: this.direction.normalize(),
                })
            });
        }
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Methods
        // (self named as a `.` call names it, the point as a `:` call reports it: "missing argument #2 (Vector3 expected)";
        // Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        methods.add_function("ClosestPoint", |_, args: LuaMultiValue| {
            let this = luau_self::<Ray>(&args, "ClosestPoint", "Ray")?;
            let to = luau_arg::<Vector3>(&args, 2, "Vector3")?;
            Ok(Vector3(this.closest_point(to.0)))
        });
        methods.add_function("Distance", |_, args: LuaMultiValue| {
            let this = luau_self::<Ray>(&args, "Distance", "Ray")?;
            let to = luau_arg::<Vector3>(&args, 2, "Vector3")?;
            let closest = this.closest_point(to.0);
            Ok((closest - to.0).length())
        });
        // Metamethods
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for Ray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // "{origin}, {direction}", 9 significant digits each (Studio 0.744)
        let g = |value: f32| super::format_g(f64::from(value), 9);
        let (o, d) = (self.origin, self.direction);
        write!(f, "{{{}, {}, {}}}, {{{}, {}, {}}}", g(o.x), g(o.y), g(o.z), g(d.x), g(d.y), g(d.z))
    }
}

impl From<DomRay> for Ray {
    fn from(v: DomRay) -> Self {
        Ray {
            origin: Vector3::from(v.origin).0,
            direction: Vector3::from(v.direction).0,
        }
    }
}

impl From<Ray> for DomRay {
    fn from(v: Ray) -> Self {
        DomRay {
            origin: Vector3(v.origin).into(),
            direction: Vector3(v.direction).into(),
        }
    }
}
