use core::fmt;
use std::ops;

use glam::Vec2;
use mlua::prelude::*;
use rbx_dom_weak::types::Rect as DomRect;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, Vector2};
use crate::datatypes::script_error::{invalid_argument, luau_number, script_error, userdata_of};

/**
    An implementation of the [Rect](https://create.roblox.com/docs/reference/engine/datatypes/Rect)
    Roblox datatype, backed by [`glam::Vec2`].

    This implements all documented properties, methods & constructors of the Rect class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub(crate) min: Vec2,
    pub(crate) max: Vec2,
}

impl Rect {
    fn new(lhs: Vec2, rhs: Vec2) -> Self {
        Self {
            min: lhs.min(rhs),
            max: lhs.max(rhs),
        }
    }
}

impl LuaExportsTable for Rect {
    const EXPORT_NAME: &'static str = "Rect";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        // Rect.new() (empty), Rect.new(min, max) with two Vector2s checked as a client's Luau checks them, or Rect.new(x0,
        // y0, x1, y1) with four numbers read as a client reads them (a string Luau reads as one, anything else 0), the corners
        // put in order; any other count is the client's count error, without a position (Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let rect_new = |lua: &Lua, args: LuaMultiValue| match args.len() {
            0 => Ok(Rect::new(Vec2::ZERO, Vec2::ZERO)),
            2 => {
                let corner = |index: usize| {
                    let value = &args[index - 1];
                    userdata_of::<Vector2>(Some(value)).map(|v| v.0).ok_or_else(|| invalid_argument(index, Some("new"), "Vector2", value))
                };
                Ok(Rect::new(corner(1)?, corner(2)?))
            }
            4 => {
                let n = args.iter().map(|value| luau_number(lua, value).unwrap_or(0.0) as f32).collect::<Vec<_>>();
                Ok(Rect::new(Vec2::new(n[0], n[1]), Vec2::new(n[2], n[3])))
            }
            count => Err(script_error(format!("Invalid number of arguments: {count}"), false)),
        };

        TableBuilder::new(lua)?
            .with_function("new", rect_new)?
            .build_readonly()
    }
}

impl LuaUserData for Rect {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (an assignment raises as a client's does: datatypes::script_error::AssignMembers)
        fields.add_meta_field("__newindex", crate::datatypes::script_error::AssignMembers("Rect"));
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::without_methods("Rect"));
        fields.add_field_method_get("Min", |_, this| Ok(Vector2(this.min)));
        fields.add_field_method_get("Max", |_, this| Ok(Vector2(this.max)));
        fields.add_field_method_get("Width", |_, this| Ok(this.max.x - this.min.x));
        fields.add_field_method_get("Height", |_, this| Ok(this.max.y - this.min.y));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
        methods.add_meta_method(LuaMetaMethod::Unm, userdata_impl_unm);
        methods.add_meta_method(LuaMetaMethod::Add, userdata_impl_add);
        methods.add_meta_method(LuaMetaMethod::Sub, userdata_impl_sub);
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // "min.X, min.Y, max.X, max.Y" with 9 significant digits each (Studio 0.744)
        let g = |value: f32| super::format_g(f64::from(value), 9);
        write!(f, "{}, {}, {}, {}", g(self.min.x), g(self.min.y), g(self.max.x), g(self.max.y))
    }
}

impl ops::Neg for Rect {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Rect::new(-self.min, -self.max)
    }
}

impl ops::Add for Rect {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Rect::new(self.min + rhs.min, self.max + rhs.max)
    }
}

impl ops::Sub for Rect {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Rect::new(self.min - rhs.min, self.max - rhs.max)
    }
}

impl From<DomRect> for Rect {
    fn from(v: DomRect) -> Self {
        Rect {
            min: Vec2::new(v.min.x, v.min.y),
            max: Vec2::new(v.max.x, v.max.y),
        }
    }
}

impl From<Rect> for DomRect {
    fn from(v: Rect) -> Self {
        DomRect {
            min: Vector2(v.min).into(),
            max: Vector2(v.max).into(),
        }
    }
}
