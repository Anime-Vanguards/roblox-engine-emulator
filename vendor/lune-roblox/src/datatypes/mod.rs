pub(crate) use rbx_dom_weak::types::{Variant as DomValue, VariantType as DomType};

pub mod extension;

#[cfg(feature = "mlua")]
pub mod attributes;
#[cfg(feature = "mlua")]
pub mod conversion;
#[cfg(feature = "mlua")]
pub mod result;
#[cfg(feature = "mlua")]
pub mod script_error;
#[cfg(feature = "mlua")]
pub mod types;

#[cfg(feature = "mlua")]
mod util;

#[cfg(feature = "mlua")]
use result::*;

#[cfg(feature = "mlua")]
pub use crate::shared::userdata::*;

/// Non-generic userdata constructors compiled inside `lune-roblox` so downstream crates create
/// datatype userdata without re-monomorphizing `mlua::UserDataRegistry<T>`.
#[cfg(feature = "mlua")]
pub mod create {
    use mlua::prelude::*;

    use super::types::*;

    macro_rules! constructors {
        ($($fn_name:ident => $ty:ty),* $(,)?) => {
            $(
                #[inline(never)]
                pub fn $fn_name(lua: &Lua, value: $ty) -> LuaResult<LuaAnyUserData> {
                    lua.create_userdata(value)
                }
            )*
        };
    }

    constructors!(
        axes => Axes,
        brick_color => BrickColor,
        cframe => CFrame,
        color3 => Color3,
        color_sequence => ColorSequence,
        color_sequence_keypoint => ColorSequenceKeypoint,
        content => Content,
        faces => Faces,
        font => Font,
        number_range => NumberRange,
        number_sequence => NumberSequence,
        number_sequence_keypoint => NumberSequenceKeypoint,
        physical_properties => PhysicalProperties,
        ray => Ray,
        rect => Rect,
        region3 => Region3,
        region3int16 => Region3int16,
        udim => UDim,
        udim2 => UDim2,
        unique_id => UniqueId,
        vector2 => Vector2,
        vector2int16 => Vector2int16,
        vector3int16 => Vector3int16,
    );
}
