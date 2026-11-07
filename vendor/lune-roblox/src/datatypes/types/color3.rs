#![allow(clippy::many_single_char_names)]

use core::fmt;
use std::ops;

use glam::Vec3;
use mlua::prelude::*;
use rbx_dom_weak::types::{Color3 as DomColor3, Color3uint8 as DomColor3uint8};

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::super::*;

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

        let color3_from_hsv = |lua: &Lua, (h, s, v): (f32, f32, f32)| {
            super::charge_host_nanos(lua, super::datatype_price::COLOR3_NEW);
            // https://axonflux.com/handy-rgb-to-hsl-and-rgb-to-hsv-color-model-c
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

        let color3_from_hex = |_: &Lua, hex: String| {
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
        let lerp = |lua: &Lua, this: &Color3, (rhs, alpha): (LuaUserDataRef<Color3>, f32)| {
            super::charge_host_nanos(lua, super::datatype_price::COLOR3_LERP);
            let v3_this = Vec3::new(this.r, this.g, this.b);
            let v3_rhs = Vec3::new(rhs.r, rhs.g, rhs.b);
            let v3 = v3_this.lerp(v3_rhs, alpha);
            Ok(Color3 {
                r: v3.x,
                g: v3.y,
                b: v3.z,
            })
        };
        methods.add_method("Lerp", lerp);
        methods.add_method("lerp", lerp);
        methods.add_method("ToHSV", |_, this, ()| {
            // https://axonflux.com/handy-rgb-to-hsl-and-rgb-to-hsv-color-model-c
            let (r, g, b) = (this.r, this.g, this.b);
            let min = r.min(g).min(b);
            let max = r.max(g).max(b);
            let diff = max - min;

            #[allow(clippy::float_cmp)]
            let hue = (match max {
                max if max == min => 0.0,
                max if max == r => (g - b) / diff + (if g < b { 6.0 } else { 0.0 }),
                max if max == g => (b - r) / diff + 2.0,
                max if max == b => (r - g) / diff + 4.0,
                _ => unreachable!(),
            }) / 6.0;

            let sat = if max == 0.0 {
                0.0
            } else {
                (diff / max).clamp(0.0, 1.0)
            };

            Ok((hue, sat, max))
        });
        // Lower-case hex of each channel times 255, rounded half away from zero and clamped to 0..255: 0.5 is 80,
        // 0.5 / 255 is 01, 2.5 / 255 is 03, -0.1 is 00 (Studio 0.744)
        methods.add_method("ToHex", |_, this, ()| {
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
