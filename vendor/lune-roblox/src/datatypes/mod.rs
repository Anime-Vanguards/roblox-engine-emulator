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

/// A playing client's datatypes answer getmetatable with "The metatable is locked" (Studio 0.747: every datatype, ree's
/// tools/parity/probes/datatype_edges.luau and datatype_edges_more.luau `metatable`), where mlua's registered metatables
/// hold `__metatable = false`. Each type's metatable is shared by all its values, so one sample of each is enough.
#[cfg(feature = "mlua")]
pub(crate) fn lock_metatables(lua: &mlua::Lua) -> mlua::Result<()> {
    use glam::{IVec2, IVec3, Mat4, Vec2, Vec3};
    use types::*;
    let udim = UDim { scale: 0.0, offset: 0 };
    let keypoint = NumberSequenceKeypoint { time: 0.0, value: 0.0, envelope: 0.0 };
    let color_keypoint = ColorSequenceKeypoint { time: 0.0, color: Color3::default() };
    let samples = [
        lua.create_userdata(BrickColor::default())?,
        lua.create_userdata(CFrame(Mat4::IDENTITY))?,
        lua.create_userdata(Color3::default())?,
        lua.create_userdata(ColorSequence { keypoints: vec![color_keypoint, color_keypoint] })?,
        lua.create_userdata(color_keypoint)?,
        lua.create_userdata(NumberRange { min: 0.0, max: 0.0 })?,
        lua.create_userdata(NumberSequence { keypoints: vec![keypoint, keypoint] })?,
        lua.create_userdata(keypoint)?,
        lua.create_userdata(PhysicalProperties::from(rbx_dom_weak::types::CustomPhysicalProperties::new(1.0, 1.0, 1.0, 1.0, 1.0, 1.0)))?,
        lua.create_userdata(Ray { origin: Vec3::ZERO, direction: Vec3::ZERO })?,
        lua.create_userdata(Rect { min: Vec2::ZERO, max: Vec2::ZERO })?,
        lua.create_userdata(Region3 { min: Vec3::ZERO, max: Vec3::ZERO })?,
        lua.create_userdata(Region3int16 { min: IVec3::ZERO, max: IVec3::ZERO })?,
        lua.create_userdata(udim)?,
        lua.create_userdata(UDim2 { x: udim, y: udim })?,
        lua.create_userdata(Vector2(Vec2::ZERO))?,
        lua.create_userdata(Vector2int16(IVec2::ZERO))?,
        lua.create_userdata(Vector3int16(IVec3::ZERO))?,
    ];
    for sample in &samples {
        // SAFETY: the sample is pushed as the argument; its metatable is read, given the field and popped
        unsafe {
            lua.exec_raw::<()>(sample, |state| {
                use mlua::ffi;
                if ffi::lua_getmetatable(state, -1) != 0 {
                    ffi::lua_pushstring(state, c"The metatable is locked".as_ptr());
                    ffi::lua_setfield(state, -2, c"__metatable".as_ptr());
                    ffi::lua_pop(state, 1);
                }
            })?;
        }
    }
    Ok(())
}

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
