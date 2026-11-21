#![allow(clippy::many_single_char_names)]

use core::fmt;
use std::ops;

use mlua::prelude::*;
use rbx_dom_weak::types::{Color3 as DomColor3, Color3uint8 as DomColor3uint8};

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::super::*;
use crate::datatypes::script_error::{luau_number, luau_self, reflected_arg, reflected_float_arg, script_error};

/**
    An implementation of the [Color3](https://create.roblox.com/docs/reference/engine/datatypes/Color3) Roblox datatype.

    This implements all documented properties, methods & constructors of the Color3 class as of October 2025.

    It also implements math operations for addition, subtraction, multiplication, and division,
    all of which are suspiciously missing from the Roblox implementation of the Color3 datatype.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color3 {
    pub(crate) r: f32,
    pub(crate) g: f32,
    pub(crate) b: f32,
}

impl LuaExportsTable for Color3 {
    const EXPORT_NAME: &'static str = "Color3";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        let color3_from_rgb = |lua: &Lua, (r, g, b): (Option<u8>, Option<u8>, Option<u8>)| {
            super::charge_host_nanos(lua, super::datatype_price::COLOR3_NEW);
            Ok(Color3 {
                r: (r.unwrap_or_default() as f32) / 255f32,
                g: (g.unwrap_or_default() as f32) / 255f32,
                b: (b.unwrap_or_default() as f32) / 255f32,
            })
        };

        // (fewer than three arguments is the client's own message, without a position; the three read as a client reads
        // them, a string Luau reads as a number included and anything else 0: Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let color3_from_hsv = |lua: &Lua, args: LuaMultiValue| {
            if args.len() < 3 {
                return Err(script_error("Color3.fromHSV requires three arguments", false));
            }
            let component = |index: usize| luau_number(lua, &args[index]).unwrap_or(0.0) as f32;
            let (h, s, v) = (component(0), component(1), component(2));
            super::charge_host_nanos(lua, super::datatype_price::COLOR3_NEW);
            // https://axonflux.com/handy-rgb-to-hsl-and-rgb-to-hsv-color-model-c, in floats, the hue clamped to 0..1 first
            // (fromHSV(1.5, 1, 1) is fromHSV(1, 1, 1) and -0.25 is 0: Studio 0.744, tools/parity/probes/datatypes.luau)
            let h = h.clamp(0.0, 1.0);
            let i = (h * 6.0).floor();
            let f = h * 6.0 - i;
            let p = v * (1.0 - s);
            let q = v * (1.0 - f * s);
            let t = v * (1.0 - (1.0 - f) * s);

            let (r, g, b) = match (i % 6.0) as u8 {
                0 => (v, t, p),
                1 => (q, v, p),
                2 => (p, v, t),
                3 => (p, q, v),
                4 => (t, p, v),
                5 => (v, p, q),
                _ => unreachable!(),
            };

            Ok(Color3 { r, g, b })
        };

        // (no argument is the client's own message; a number is its text, as Luau turns one into a string; any other value is
        // malformed text: Studio 0.747, ree's tools/parity/probes/datatype_errors.luau)
        let color3_from_hex = |lua: &Lua, args: LuaMultiValue| {
            let hex = match args.front() {
                None => return Err(script_error("Color3.fromHex requires one argument", false)),
                Some(value @ (LuaValue::String(_) | LuaValue::Number(_) | LuaValue::Integer(_))) => {
                    lua.coerce_string(value.clone())?.map(|text| text.to_string_lossy()).unwrap_or_default()
                }
                Some(_) => return Err(script_error("Unable to convert characters to hex value", false)),
            };
            let trimmed = hex.trim_start_matches('#').to_ascii_uppercase();
            let chars = if !trimmed.is_ascii() {
                return Err(LuaError::RuntimeError("Unable to convert characters to hex value".to_string()));
            } else if trimmed.len() == 3 {
                (
                    u8::from_str_radix(&trimmed[..1].repeat(2), 16),
                    u8::from_str_radix(&trimmed[1..2].repeat(2), 16),
                    u8::from_str_radix(&trimmed[2..3].repeat(2), 16),
                )
            } else if trimmed.len() == 6 {
                (
                    u8::from_str_radix(&trimmed[..2], 16),
                    u8::from_str_radix(&trimmed[2..4], 16),
                    u8::from_str_radix(&trimmed[4..6], 16),
                )
            } else {
                // every malformed string, of any length, is this one message in Roblox (Studio 0.744)
                return Err(LuaError::RuntimeError("Unable to convert characters to hex value".to_string()));
            };
            match chars {
                (Ok(r), Ok(g), Ok(b)) => Ok(Color3 {
                    r: (r as f32) / 255f32,
                    g: (g as f32) / 255f32,
                    b: (b as f32) / 255f32,
                }),
                _ => Err(LuaError::RuntimeError("Unable to convert characters to hex value".to_string())),
            }
        };

        let color3_new = |lua: &Lua, (r, g, b): (Option<f32>, Option<f32>, Option<f32>)| {
            super::charge_host_nanos(lua, super::datatype_price::COLOR3_NEW);
            Ok(Color3 {
                r: r.unwrap_or_default(),
                g: g.unwrap_or_default(),
                b: b.unwrap_or_default(),
            })
        };

        TableBuilder::new(lua)?
            .with_function("fromRGB", color3_from_rgb)?
            .with_function("fromHSV", color3_from_hsv)?
            .with_function("fromHex", color3_from_hex)?
            .with_function("new", color3_new)?
            .build_readonly()
    }
}

impl FromLua for Color3 {
    fn from_lua(value: LuaValue, _: &Lua) -> LuaResult<Self> {
        if let LuaValue::UserData(ud) = value {
            Ok(*ud.borrow::<Color3>()?)
        } else {
            Err(LuaError::FromLuaConversionError {
                from: value.type_name(),
                to: "Color3".to_string(),
                message: None,
            })
        }
    }
}

impl LuaUserData for Color3 {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // (an assignment raises as a client's does: datatypes::script_error::AssignMembers)
        fields.add_meta_field("__newindex", crate::datatypes::script_error::AssignMembers("Color3"));
        // (a member it lacks raises as a client's does: datatypes::script_error::UnknownMembers)
        fields.add_meta_field("__index", crate::datatypes::script_error::UnknownMembers::with_methods("Color3"));
        fields.add_field_method_get("R", |_, this| Ok(this.r));
        fields.add_field_method_get("G", |_, this| Ok(this.g));
        fields.add_field_method_get("B", |_, this| Ok(this.b));
        // (Roblox's deprecated lowercase names, measured in a client: ree's tools/parity/probes/camelcase.luau)
        fields.add_field_method_get("r", |_, this| Ok(this.r));
        fields.add_field_method_get("g", |_, this| Ok(this.g));
        fields.add_field_method_get("b", |_, this| Ok(this.b));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Methods
        // a + (b - a) * t per channel in floats (Studio 0.744, tools/parity/probes/datatypes.luau)
        // (a playing client's reflection checks self, then the alpha, then the goal: "Argument 3 missing or nil", "Unable
        // to cast double to Color3"; ToHSV and ToHex check self in Luau, from the calling line; Studio 0.747, ree's
        // tools/parity/probes/datatype_errors.luau)
        let lerp = |lua: &Lua, args: LuaMultiValue| {
            let this = reflected_arg::<Color3>(&args, 1, "Color3")?;
            let alpha = reflected_float_arg(lua, &args, 3)? as f32;
            let rhs = reflected_arg::<Color3>(&args, 2, "Color3")?;
            super::charge_host_nanos(lua, super::datatype_price::COLOR3_LERP);
            let lerp = super::float32::lerp;
            Ok(Color3 {
                r: lerp(this.r, rhs.r, alpha),
                g: lerp(this.g, rhs.g, alpha),
                b: lerp(this.b, rhs.b, alpha),
            })
        };
        methods.add_function("Lerp", lerp);
        methods.add_function("lerp", lerp);
        // G3D's (Foley and van Dam) in floats: pure red's hue is 1 (Studio 0.744, tools/parity/probes/datatypes.luau,
        // cframe_order.luau)
        methods.add_function("ToHSV", |_, args: LuaMultiValue| {
            let this = luau_self::<Color3>(&args, "ToHSV", "Color3")?;
            Ok(super::float32::to_hsv(this.r, this.g, this.b))
        });
        // Lower-case hex of each channel times 255, rounded half away from zero and clamped to 0..255: 0.5 is 80,
        // 0.5 / 255 is 01, 2.5 / 255 is 03, -0.1 is 00 (Studio 0.744)
        methods.add_function("ToHex", |_, args: LuaMultiValue| {
            let this = luau_self::<Color3>(&args, "ToHex", "Color3")?;
            let byte = |value: f32| (value * 255.0).round().clamp(u8::MIN as f32, u8::MAX as f32) as u8;
            Ok(format!("{:02x}{:02x}{:02x}", byte(this.r), byte(this.g), byte(this.b)))
        });
        // Metamethods (Color3 has no arithmetic in Roblox: `a + b`, `a * 2` and `-a` raise "attempt to perform
        // arithmetic (add) on Color3", measured in Studio 0.744)
        methods.add_meta_method(LuaMetaMethod::Eq, |lua, this, value: LuaValue| {
            super::charge_host_nanos(lua, super::datatype_price::EQ);
            userdata_impl_eq(lua, this, value)
        });
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl Default for Color3 {
    fn default() -> Self {
        Self {
            r: 0f32,
            g: 0f32,
            b: 0f32,
        }
    }
}

impl fmt::Display for Color3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 6 significant digits ("0.333333, 0.1, 0.666667"; Studio 0.744)
        let g = |value: f32| super::format_g(f64::from(value), 6);
        write!(f, "{}, {}, {}", g(self.r), g(self.g), g(self.b))
    }
}

impl ops::Neg for Color3 {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Color3 {
            r: -self.r,
            g: -self.g,
            b: -self.b,
        }
    }
}

impl ops::Add for Color3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Color3 {
            r: self.r + rhs.r,
            g: self.g + rhs.g,
            b: self.b + rhs.b,
        }
    }
}

impl ops::Sub for Color3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Color3 {
            r: self.r - rhs.r,
            g: self.g - rhs.g,
            b: self.b - rhs.b,
        }
    }
}

impl ops::Mul for Color3 {
    type Output = Color3;
    fn mul(self, rhs: Self) -> Self::Output {
        Color3 {
            r: self.r * rhs.r,
            g: self.g * rhs.g,
            b: self.b * rhs.b,
        }
    }
}

impl ops::Mul<f32> for Color3 {
    type Output = Color3;
    fn mul(self, rhs: f32) -> Self::Output {
        Color3 {
            r: self.r * rhs,
            g: self.g * rhs,
            b: self.b * rhs,
        }
    }
}

impl ops::Div for Color3 {
    type Output = Color3;
    fn div(self, rhs: Self) -> Self::Output {
        Color3 {
            r: self.r / rhs.r,
            g: self.g / rhs.g,
            b: self.b / rhs.b,
        }
    }
}

impl ops::Div<f32> for Color3 {
    type Output = Color3;
    fn div(self, rhs: f32) -> Self::Output {
        Color3 {
            r: self.r / rhs,
            g: self.g / rhs,
            b: self.b / rhs,
        }
    }
}

impl From<DomColor3> for Color3 {
    fn from(v: DomColor3) -> Self {
        Self {
            r: v.r,
            g: v.g,
            b: v.b,
        }
    }
}

impl From<Color3> for DomColor3 {
    fn from(v: Color3) -> Self {
        Self {
            r: v.r,
            g: v.g,
            b: v.b,
        }
    }
}

impl From<DomColor3uint8> for Color3 {
    fn from(v: DomColor3uint8) -> Self {
        Color3::from(DomColor3::from(v))
    }
}

impl From<Color3> for DomColor3uint8 {
    fn from(v: Color3) -> Self {
        DomColor3uint8::from(DomColor3::from(v))
    }
}
