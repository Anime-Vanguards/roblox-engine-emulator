use core::fmt;

use glam::Vec3;
use mlua::prelude::*;
use rbx_dom_weak::types::Ray as DomRay;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, Vector3, VectorArg, native_vector};
use crate::datatypes::script_error::{
    invalid_argument, invalid_method_arg, luau_self, missing_method_arg, MethodMembers,
};

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
    fn closest_point_f64(&self, point: Vec3) -> [f64; 3] {
        let o = [f64::from(self.origin.x), f64::from(self.origin.y), f64::from(self.origin.z)];
        let d = [f64::from(self.direction.x), f64::from(self.direction.y), f64::from(self.direction.z)];
        let p = [f64::from(point.x), f64::from(point.y), f64::from(point.z)];
        let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if dd == 0.0 {
            return o;
        }
        let t = (((p[0] - o[0]) * d[0] + (p[1] - o[1]) * d[1] + (p[2] - o[2]) * d[2]) / dd).max(0.0);
        [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t]
    }

    fn closest_point(&self, point: Vec3) -> Vec3 {
        let c = self.closest_point_f64(point);
        Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32)
    }

    fn distance(&self, point: Vec3) -> f64 {
        let c = self.closest_point_f64(point);
        let dx = f64::from(point.x) - c[0];
        let dy = f64::from(point.y) - c[1];
        let dz = f64::from(point.z) - c[2];
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

fn optional_vector_arg(lua: &Lua, args: &LuaMultiValue, index: usize, name: &str) -> LuaResult<Vec3> {
    match args.get(index - 1) {
        None => Ok(Vec3::ZERO),
        Some(value) => VectorArg::from_lua(value.clone(), lua)
            .map(|v| v.0)
            .map_err(|_| invalid_argument(index, Some(name), "Vector3", value)),
    }
}

fn method_vector_arg(lua: &Lua, args: &LuaMultiValue, index: usize, method: &str) -> LuaResult<Vec3> {
    match args.get(index - 1) {
        None => Err(missing_method_arg(index, method, "Vector3")),
        Some(value) => VectorArg::from_lua(value.clone(), lua)
            .map(|v| v.0)
            .map_err(|_| invalid_method_arg(index, method, "Vector3", value)),
    }
}

impl LuaExportsTable for Ray {
    const EXPORT_NAME: &'static str = "Ray";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        // (two Vector3s checked as a client's Luau checks them, naming the constructor; one not passed is the zero vector:
        // Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let ray_new = |lua: &Lua, args: LuaMultiValue| {
            let origin = optional_vector_arg(lua, &args, 1, "new")?;
            let direction = optional_vector_arg(lua, &args, 2, "new")?;
            Ok(Ray { origin, direction })
        };

        TableBuilder::new(lua)?
            .with_function("new", ray_new)?
            .build_readonly()
    }
}

impl LuaUserData for Ray {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (an assignment raises as a client's does: datatypes::script_error::AssignMembers)
        fields.add_meta_field("__newindex", crate::datatypes::script_error::AssignMembers("Ray"));
        // (a member it lacks raises as a client's does, and `:` resolves methods only: datatypes::script_error::MethodMembers)
        let members = MethodMembers::new("Ray");
        fields.add_meta_field("__index", members.index());
        fields.add_meta_field("__namecall", members.namecall());
        fields.add_field_method_get("Origin", |_, this| Ok(native_vector(this.origin)));
        fields.add_field_method_get("Direction", |_, this| Ok(native_vector(this.direction)));
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
        // (self named as a `.` call names it; point names the method on `.` and omits it on `:`: Studio 0.747)
        methods.add_function("ClosestPoint", |lua, args: LuaMultiValue| {
            let this = luau_self::<Ray>(&args, "ClosestPoint", "Ray")?;
            let to = method_vector_arg(lua, &args, 2, "ClosestPoint")?;
            Ok(native_vector(this.closest_point(to)))
        });
        methods.add_function("Distance", |lua, args: LuaMultiValue| {
            let this = luau_self::<Ray>(&args, "Distance", "Ray")?;
            let to = method_vector_arg(lua, &args, 2, "Distance")?;
            Ok(this.distance(to))
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
