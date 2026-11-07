#![allow(clippy::items_after_statements)]

use core::fmt;
use std::{ops, sync::Arc};

use glam::{EulerRot, Mat3, Mat4, Quat, Vec3};
use mlua::{Variadic, prelude::*};
use rbx_dom_weak::types::{CFrame as DomCFrame, Matrix3 as DomMatrix3, Vector3 as DomVector3};

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, Vector3, VectorArg, charge_host_nanos, native_vector};

/**
    An implementation of the [CFrame](https://create.roblox.com/docs/reference/engine/datatypes/CFrame)
    Roblox datatype, backed by [`glam::Mat4`].

    This implements most documented properties, methods & constructors of the `CFrame` class as of October 2025,
    notably missing `CFrame.fromRotationBetweenVectors`, `CFrame:FuzzyEq()`, and `CFrame:AngleBetween()`.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CFrame(pub Mat4);

/// What Studio spends on each CFrame operation beyond the instructions that reach it (the loads, the
/// call, the loop around it), in nanoseconds, so that the work clock's cost for a CFrame operation in a
/// loop is Studio's (an op_cost probe, one body run in a lobby's playtest client and
/// in ree). Negative where the call alone already counts more than Studio spent on all of it.
pub mod price {
    //! Nanoseconds beyond what the instructions that reach the operation count (the loads, the call, the loop
    //! around it). Each is the lobby playtest client's median of four runs (the op_cost probe's
    //! `datatype` rows, in its client DataModel) less what ree counts for the same
    //! loop with no price (the VM's own charge for the call: 1.6 for a member, 15.1 for a method, 28-31 for a
    //! constructor the prelude wraps). A CFrame the operation returns is a userdata the garbage collector
    //! sweeps, which a client with a big heap pays for: the same operations in an empty test place cost
    //! 40-50 ns less (`*` 20 against 62, Inverse 20 against 69), and the allocating rows vary 2-3x between
    //! runs (Lerp 85-140, `*` 30-78, new(Vector3) 15-78), so these are good to about +-30 %.
    //!
    //! `*`, Inverse and `==` are the exception: they are priced as a handler meets them, a few times a
    //! frame on a CFrame taken from an instance (a bridge_ops probe, 28 rigs, five motors each, the median
    //! of 210 frames; an empty test place's client and a match place's client, three runs, six readings each): Inverse
    //! 18-42 ns (median 24.5), `* CFrame.identity` 20-39 (24.6), `== CFrame.identity` 15-26 (22.2); the same
    //! operations in the op_cost loop of the lobby client read 69, 62 and (not probed). A cut that removes them
    //! from per-frame code (a rig's per-frame cuts) is priced from the first set; a loop of thousands of them reads
    //! about 40 ns low per operation in ree, as the test place's loop does.
    //!
    //! `:ToObjectSpace`, `:Lerp`, `.Rotation`, `CFrame.Angles` and `CFrame.fromEulerAngles*` follow: the same
    //! rig-frame bench (a match place's client, three runs, six readings each, medians in ns): ToObjectSpace 28.9 (24-64),
    //! Lerp 66.8 (44-88), Rotation 30.3 (9-66), Angles 28.6 (18-36), fromEulerAnglesXYZ 27.0 (16-34). The op_cost loop of
    //! the lobby client reads 71, 110, 63, 87 for the first four, 1.5-3x higher.
    //!
    //! `:ToWorldSpace`, `:PointToWorldSpace` and `:ToAxisAngle` follow (the same bench, a match place's client, six
    //! runs of six readings, medians in ns): ToWorldSpace 26.6 (16-121; the twin of ToObjectSpace, which reads 28.5),
    //! PointToWorldSpace 24.4 with the vector hoisted out of the loop, ToAxisAngle 22.2 here and 26.3 in the earlier three runs.
    //!
    //! Refitted (2026-11-07) where the parity bench (tools/cost-parity: a rendered client of a game's match place, nine sessions of
    //! the whole corpus, the median of 9 rounds of a loop each) has a row outside its held-out third, ns whole: GetComponents 22.0 (all
    //! twelve values taken 26.9), `cf + v` 22.1, Rotation 18.1, Angles 41.3, the twelve-number constructor 35.5, Inverse 36.3, Lerp
    //! 75.7, PointToObjectSpace 17.9, ToEulerAnglesXYZ 22.1, ToObjectSpace 20.2, `cf * v` 12.3, X / LookVector / RightVector 3.3-3.7.
    //! Those rows read 0.41-5.9x before (GetComponents 131 ns). `*`, `==`, lookAt and the three-number constructor keep their
    //! prices: their rows are held out (`*` 41.8, lookAt 43.3, new 18.8).
    pub const MEMBER: i32 = 1;
    pub const ROTATION: i32 = 17;
    pub const MUL_CFRAME: i32 = 21;
    /// `CFrame == CFrame` is a metamethod call in Studio (22.2 ns median in the rig frame), where the VM counts only the
    /// instruction
    pub const EQ: i32 = 17;
    pub const MUL_VECTOR: i32 = 11;
    pub const ADD_VECTOR: i32 = 21;
    pub const INVERSE: i32 = 26;
    pub const LERP: i32 = 66;
    pub const TO_WORLD: i32 = 10;
    pub const TO_OBJECT: i32 = 10;
    pub const POINT_TO_WORLD: i32 = 8;
    pub const POINT_TO_OBJECT: i32 = 7;
    pub const VECTOR_TO_WORLD: i32 = -4;
    pub const VECTOR_TO_OBJECT: i32 = 0;
    pub const COMPONENTS: i32 = 14;
    pub const EULER: i32 = 12;
    pub const AXIS_ANGLE: i32 = 10;
    /// (not measured)
    pub const ORTHONORMALIZE: i32 = 0;
    pub const NEW_VECTOR: i32 = -3;
    pub const NEW_XYZ: i32 = -2;
    pub const NEW_LOOK: i32 = 55;
    /// (not measured)
    pub const NEW_QUAT: i32 = 0;
    /// (the twelve-number constructor)
    pub const NEW_MATRIX: i32 = 8;
    /// (a Studio playtest client read Angles at 27-29 ns, below what the call counts here; a client of the match place 49.6)
    pub const ANGLES: i32 = 18;
    pub const FROM_AXIS_ANGLE: i32 = 48;
    pub const LOOK_AT: i32 = 55;
}

/// A method under each of its names, charged `price` nanoseconds
macro_rules! aliased {
    ($methods:ident, $price:expr, [$($name:literal),+], $function:expr) => {
        $($methods.add_method($name, charged_method($price, $function));)+
    };
}

fn charged_method<A, R>(
    price: i32,
    function: impl Fn(&Lua, &CFrame, A) -> LuaResult<R> + 'static,
) -> impl Fn(&Lua, &CFrame, A) -> LuaResult<R> + 'static {
    move |lua, this, args| {
        charge_host_nanos(lua, price);
        function(lua, this, args)
    }
}

/// An Enum.RotationOrder argument (the host's Enum items print as "Enum.RotationOrder.XYZ"; nil is XYZ):
/// the order's name and glam's rotation for it. Angles go by the letters of the name, x y z.
fn rotation_order(order: Option<&LuaValue>, function: &str) -> LuaResult<(&'static str, EulerRot)> {
    let text = match order {
        None | Some(LuaValue::Nil) => String::from("Enum.RotationOrder.XYZ"),
        Some(item) => item.to_string()?,
    };
    Ok(match text.strip_prefix("Enum.RotationOrder.") {
        Some("XYZ") => ("XYZ", EulerRot::XYZ),
        Some("XZY") => ("XZY", EulerRot::XZY),
        Some("YXZ") => ("YXZ", EulerRot::YXZ),
        Some("YZX") => ("YZX", EulerRot::YZX),
        Some("ZXY") => ("ZXY", EulerRot::ZXY),
        Some("ZYX") => ("ZYX", EulerRot::ZYX),
        // (Studio's message for a string: "Invalid value for enum 'RotationOrder'")
        _ => {
            let _ = function;
            return Err(LuaError::RuntimeError("Invalid value for enum 'RotationOrder'".to_string()));
        }
    })
}

impl CFrame {
    pub const IDENTITY: Self = Self(Mat4::IDENTITY);

    fn position(&self) -> Vec3 {
        self.0.w_axis.truncate()
    }

    fn orientation(&self) -> Mat3 {
        Mat3::from_cols(
            self.0.x_axis.truncate(),
            self.0.y_axis.truncate(),
            self.0.z_axis.truncate(),
        )
    }

    fn inverse(&self) -> Self {
        Self(self.0.inverse())
    }
}

impl LuaExportsTable for CFrame {
    const EXPORT_NAME: &'static str = "CFrame";

    #[allow(clippy::too_many_lines)]
    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        let cframe_angles = |lua: &Lua, (rx, ry, rz): (f32, f32, f32)| {
            charge_host_nanos(lua, price::ANGLES);
            Ok(CFrame(Mat4::from_euler(EulerRot::XYZ, rx, ry, rz)))
        };

        let cframe_from_axis_angle = |lua: &Lua, (v, r): (VectorArg, f32)| {
            charge_host_nanos(lua, price::FROM_AXIS_ANGLE);
            Ok(CFrame(Mat4::from_axis_angle(v.0, r)))
        };

        let cframe_from_euler_angles_xyz = |lua: &Lua, (rx, ry, rz): (f32, f32, f32)| {
            charge_host_nanos(lua, price::ANGLES);
            Ok(CFrame(Mat4::from_euler(EulerRot::XYZ, rx, ry, rz)))
        };

        let cframe_from_euler_angles = |lua: &Lua, (rx, ry, rz, order): (f32, f32, f32, Option<LuaValue>)| {
            charge_host_nanos(lua, price::ANGLES);
            let (name, rotation) = rotation_order(order.as_ref(), "fromEulerAngles")?;
            let by_axis = [rx, ry, rz];
            let [a, b, c] = name.chars().map(|axis| by_axis[(axis as usize) - ('X' as usize)]).collect::<Vec<_>>()[..] else {
                unreachable!()
            };
            Ok(CFrame(Mat4::from_euler(rotation, a, b, c)))
        };

        let cframe_from_euler_angles_yxz = |lua: &Lua, (rx, ry, rz): (f32, f32, f32)| {
            charge_host_nanos(lua, price::ANGLES);
            Ok(CFrame(Mat4::from_euler(EulerRot::YXZ, ry, rx, rz)))
        };

        let cframe_from_matrix = |lua: &Lua,
                                  (pos, rx, ry, rz): (
            VectorArg,
            VectorArg,
            VectorArg,
            Option<VectorArg>,
        )| {
            charge_host_nanos(lua, price::NEW_MATRIX);
            Ok(CFrame(Mat4::from_cols(
                rx.0.extend(0.0),
                ry.0.extend(0.0),
                rz.map_or_else(|| rx.0.cross(ry.0).normalize(), |r| r.0)
                    .extend(0.0),
                pos.0.extend(1.0),
            )))
        };

        let cframe_from_orientation = |lua: &Lua, (rx, ry, rz): (f32, f32, f32)| {
            charge_host_nanos(lua, price::ANGLES);
            Ok(CFrame(Mat4::from_euler(EulerRot::YXZ, ry, rx, rz)))
        };

        let cframe_look_at = |lua: &Lua, (from, to, up): (VectorArg, VectorArg, Option<VectorArg>)| {
            charge_host_nanos(lua, price::LOOK_AT);
            Ok(CFrame(look_at(
                from.0,
                to.0,
                up.map_or(Vec3::Y, |up| up.0),
            )))
        };

        // Dynamic args constructor
        type ArgsPos = VectorArg;
        type ArgsLook = (VectorArg, VectorArg, Option<VectorArg>);

        type ArgsPosXYZ = (f32, f32, f32);
        type ArgsPosXYZQuat = (f32, f32, f32, f32, f32, f32, f32);
        type ArgsMatrix = (f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32);

        let cframe_new = |lua: &Lua, args: LuaMultiValue| {
            charge_host_nanos(lua, match (args.len(), args.front()) {
                (1, _) => price::NEW_VECTOR,
                (2, _) => price::NEW_LOOK,
                (3, Some(LuaValue::Number(_) | LuaValue::Integer(_))) => price::NEW_XYZ,
                (3, _) => price::NEW_LOOK,
                (7, _) => price::NEW_QUAT,
                (12, _) => price::NEW_MATRIX,
                _ => 0,
            });
            match args.len() {
            0 => Ok(CFrame(Mat4::IDENTITY)),

            1 => match ArgsPos::from_lua_multi(args, lua) {
                Ok(pos) => Ok(CFrame(Mat4::from_translation(pos.0))),
                Err(err) => Err(err),
            },

            // Roblox's CFrame.new(from, to): looking at `to`
            2 => {
                let vector = |index: usize| {
                    ArgsPos::from_lua(args[index].clone(), lua).map_err(|cause| {
                        LuaError::BadArgument {
                            to: Some("new".to_string()),
                            pos: index + 1,
                            name: None,
                            cause: Arc::new(cause),
                        }
                    })
                };
                Ok(CFrame(look_at(vector(0)?.0, vector(1)?.0, Vec3::Y)))
            }

            3 => {
                // the position as plain numbers, without the argument conversions below
                let number = |index: usize| match args[index] {
                    LuaValue::Number(n) => Some(n as f32),
                    LuaValue::Integer(n) => Some(n as f32),
                    _ => None,
                };
                if let (Some(x), Some(y), Some(z)) = (number(0), number(1), number(2)) {
                    Ok(CFrame(Mat4::from_translation(Vec3::new(x, y, z))))
                } else if let Ok((from, to, up)) = ArgsLook::from_lua_multi(args.clone(), lua) {
                    Ok(CFrame(look_at(
                        from.0,
                        to.0,
                        up.map_or(Vec3::Y, |up| up.0),
                    )))
                } else if let Ok((x, y, z)) = ArgsPosXYZ::from_lua_multi(args, lua) {
                    Ok(CFrame(Mat4::from_translation(Vec3::new(x, y, z))))
                } else {
                    // TODO: Make this error message better
                    Err(LuaError::RuntimeError(
                        "Invalid arguments to constructor".to_string(),
                    ))
                }
            }

            7 => match ArgsPosXYZQuat::from_lua_multi(args, lua) {
                Ok((x, y, z, qx, qy, qz, qw)) => Ok(CFrame(Mat4::from_rotation_translation(
                    Quat::from_array([qx, qy, qz, qw]),
                    Vec3::new(x, y, z),
                ))),
                Err(err) => Err(err),
            },

            12 => match ArgsMatrix::from_lua_multi(args, lua) {
                Ok((x, y, z, r00, r01, r02, r10, r11, r12, r20, r21, r22)) => {
                    Ok(CFrame(Mat4::from_cols_array_2d(&[
                        [r00, r10, r20, 0.0],
                        [r01, r11, r21, 0.0],
                        [r02, r12, r22, 0.0],
                        [x, y, z, 1.0],
                    ])))
                }
                Err(err) => Err(err),
            },

            _ => Err(LuaError::RuntimeError(format!(
                "Invalid number of arguments: expected 0, 1, 3, 7, or 12, got {}",
                args.len()
            ))),
            }
        };

        TableBuilder::new(lua)?
            .with_function("Angles", cframe_angles)?
            .with_value("identity", CFrame(Mat4::IDENTITY))?
            .with_function("fromAxisAngle", cframe_from_axis_angle)?
            .with_function("fromEulerAngles", cframe_from_euler_angles)?
            .with_function("fromEulerAnglesXYZ", cframe_from_euler_angles_xyz)?
            .with_function("fromEulerAnglesYXZ", cframe_from_euler_angles_yxz)?
            .with_function("fromMatrix", cframe_from_matrix)?
            .with_function("fromOrientation", cframe_from_orientation)?
            .with_function("lookAt", cframe_look_at)?
            .with_function("new", cframe_new)?
            .build_readonly()
    }
}

impl LuaUserData for CFrame {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        // Vectors come back as Luau's native vector, which is what a script's Vector3 is; each read is
        // charged to the script as the one Luau call the engine's prelude spent on it before it was native
        fields.add_field_method_get("Position", |lua, this| {
            charge_host_nanos(lua, price::MEMBER);
            Ok(native_vector(this.position()))
        });
        // (Roblox still answers its deprecated lowercase members)
        fields.add_field_method_get("p", |lua, this| {
            charge_host_nanos(lua, price::MEMBER);
            Ok(native_vector(this.position()))
        });
        fields.add_field_method_get("Rotation", |lua, this| {
            charge_host_nanos(lua, price::ROTATION);
            Ok(CFrame(Mat4::from_cols(
                this.0.x_axis,
                this.0.y_axis,
                this.0.z_axis,
                Vec3::ZERO.extend(1.0),
            )))
        });
        fields.add_field_method_get("X", |lua, this| {
            charge_host_nanos(lua, price::MEMBER);
            Ok(this.position().x)
        });
        fields.add_field_method_get("Y", |lua, this| {
            charge_host_nanos(lua, price::MEMBER);
            Ok(this.position().y)
        });
        fields.add_field_method_get("Z", |lua, this| {
            charge_host_nanos(lua, price::MEMBER);
            Ok(this.position().z)
        });
        for name in ["XVector", "RightVector", "rightVector"] {
            fields.add_field_method_get(name, |lua, this| {
                charge_host_nanos(lua, price::MEMBER);
                Ok(native_vector(this.orientation().x_axis))
            });
        }
        for name in ["YVector", "UpVector", "upVector"] {
            fields.add_field_method_get(name, |lua, this| {
                charge_host_nanos(lua, price::MEMBER);
                Ok(native_vector(this.orientation().y_axis))
            });
        }
        fields.add_field_method_get("ZVector", |lua, this| {
            charge_host_nanos(lua, price::MEMBER);
            Ok(native_vector(this.orientation().z_axis))
        });
        for name in ["LookVector", "lookVector"] {
            fields.add_field_method_get(name, |lua, this| {
                charge_host_nanos(lua, price::MEMBER);
                Ok(native_vector(-this.orientation().z_axis))
            });
        }
    }

    #[allow(clippy::too_many_lines)]
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Methods, and the deprecated lowercase names Roblox still answers
        aliased!(methods, price::INVERSE, ["Inverse", "inverse"], |_, this, ()| Ok(this.inverse()));
        aliased!(
            methods,
            price::LERP,
            ["Lerp", "lerp"],
            |_, this, (goal, alpha): (LuaUserDataRef<CFrame>, f32)| {
                let quat_this = Quat::from_mat4(&this.0);
                let quat_goal = Quat::from_mat4(&goal.0);
                let translation = this
                    .0
                    .w_axis
                    .truncate()
                    .lerp(goal.0.w_axis.truncate(), alpha);
                let rotation = quat_this.slerp(quat_goal, alpha);
                Ok(CFrame(Mat4::from_rotation_translation(
                    rotation,
                    translation,
                )))
            }
        );
        methods.add_method("Orthonormalize", charged_method(price::ORTHONORMALIZE, |_, this, ()| {
            let rotation = Quat::from_mat4(&this.0);
            let translation = this.0.w_axis.truncate();
            Ok(CFrame(Mat4::from_rotation_translation(
                rotation.normalize(),
                translation,
            )))
        }));
        aliased!(
            methods,
            price::TO_WORLD,
            ["ToWorldSpace", "toWorldSpace"],
            |_, this, rhs: Variadic<LuaUserDataRef<CFrame>>| {
                Ok(rhs
                    .into_iter()
                    .map(|cf| *this * *cf)
                    .collect::<Variadic<_>>())
            }
        );
        aliased!(
            methods,
            price::TO_OBJECT,
            ["ToObjectSpace", "toObjectSpace"],
            |_, this, rhs: Variadic<LuaUserDataRef<CFrame>>| {
                let inverse = this.inverse();
                Ok(rhs
                    .into_iter()
                    .map(|cf| inverse * *cf)
                    .collect::<Variadic<_>>())
            }
        );
        aliased!(
            methods,
            price::POINT_TO_WORLD,
            ["PointToWorldSpace", "pointToWorldSpace"],
            |_, this, rhs: Variadic<VectorArg>| {
                Ok(rhs
                    .into_iter()
                    .map(|v3| native_vector((*this * Vector3(v3.0)).0))
                    .collect::<Variadic<_>>())
            }
        );
        aliased!(
            methods,
            price::POINT_TO_OBJECT,
            ["PointToObjectSpace", "pointToObjectSpace"],
            |_, this, rhs: Variadic<VectorArg>| {
                let inverse = this.inverse();
                Ok(rhs
                    .into_iter()
                    .map(|v3| native_vector((inverse * Vector3(v3.0)).0))
                    .collect::<Variadic<_>>())
            }
        );
        aliased!(
            methods,
            price::VECTOR_TO_WORLD,
            ["VectorToWorldSpace", "vectorToWorldSpace"],
            |_, this, rhs: Variadic<VectorArg>| {
                let result = *this - Vector3(this.position());
                Ok(rhs
                    .into_iter()
                    .map(|v3| native_vector((result * Vector3(v3.0)).0))
                    .collect::<Variadic<_>>())
            }
        );
        aliased!(
            methods,
            price::VECTOR_TO_OBJECT,
            ["VectorToObjectSpace", "vectorToObjectSpace"],
            |_, this, rhs: Variadic<VectorArg>| {
                let inverse = this.inverse();
                let result = inverse - Vector3(inverse.position());
                Ok(rhs
                    .into_iter()
                    .map(|v3| native_vector((result * Vector3(v3.0)).0))
                    .collect::<Variadic<_>>())
            }
        );
        #[rustfmt::skip]
        aliased!(methods, price::COMPONENTS, ["GetComponents", "components"], |_, this, ()| {
            let pos = this.position();
            let transposed = this.orientation().transpose();
            Ok((
                pos.x,               pos.y,                 pos.z,
                transposed.x_axis.x, transposed.x_axis.y,   transposed.x_axis.z,
                transposed.y_axis.x, transposed.y_axis.y,   transposed.y_axis.z,
                transposed.z_axis.x, transposed.z_axis.y,   transposed.z_axis.z,
            ))
        });
        aliased!(methods, price::EULER, ["ToEulerAnglesXYZ", "toEulerAnglesXYZ"], |_, this, ()| {
            Ok(Quat::from_mat4(&this.0).to_euler(EulerRot::XYZ))
        });
        methods.add_method("ToEulerAnglesYXZ", charged_method(price::EULER, |_, this, ()| {
            let (ry, rx, rz) = Quat::from_mat4(&this.0).to_euler(EulerRot::YXZ);
            Ok((rx, ry, rz))
        }));
        methods.add_method("ToOrientation", charged_method(price::EULER, |_, this, ()| {
            let (ry, rx, rz) = Quat::from_mat4(&this.0).to_euler(EulerRot::YXZ);
            Ok((rx, ry, rz))
        }));
        // ToEulerAngles(order = Enum.RotationOrder.XYZ): the three angles in x, y, z order for the rotation order named
        aliased!(methods, price::EULER, ["ToEulerAngles", "toEulerAngles"], |_, this, order: Option<LuaValue>| {
            let (name, rotation) = rotation_order(order.as_ref(), "ToEulerAngles")?;
            let (a, b, c) = Quat::from_mat4(&this.0).to_euler(rotation);
            let mut angles = [0.0f32; 3];
            for (angle, axis) in [a, b, c].into_iter().zip(name.chars()) {
                angles[(axis as usize) - ('X' as usize)] = angle;
            }
            Ok((angles[0], angles[1], angles[2]))
        });
        aliased!(methods, price::AXIS_ANGLE, ["ToAxisAngle", "toAxisAngle"], |_, this, ()| {
            let (axis, angle) = Quat::from_mat4(&this.0).to_axis_angle();
            Ok((native_vector(axis), angle))
        });
        // Metamethods
        methods.add_meta_method(LuaMetaMethod::Eq, |lua, this, other: LuaValue| {
            charge_host_nanos(lua, price::EQ);
            userdata_impl_eq(lua, this, other)
        });
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
        methods.add_meta_method(LuaMetaMethod::Mul, |lua, this, rhs: LuaValue| {
            match &rhs {
                LuaValue::Vector(v) => {
                    charge_host_nanos(lua, price::MUL_VECTOR);
                    return Ok(native_vector(
                        (*this * Vector3(Vec3::new(v.x(), v.y(), v.z()))).0,
                    ));
                }
                LuaValue::UserData(ud) => {
                    if let Ok(cf) = ud.borrow::<CFrame>() {
                        charge_host_nanos(lua, price::MUL_CFRAME);
                        return Ok(LuaValue::UserData(lua.create_userdata(*this * *cf)?));
                    } else if let Ok(vec) = ud.borrow::<Vector3>() {
                        charge_host_nanos(lua, price::MUL_VECTOR);
                        return Ok(native_vector((*this * *vec).0));
                    }
                }
                _ => {}
            }
            Err(LuaError::FromLuaConversionError {
                from: rhs.type_name(),
                to: "userdata".to_string(),
                message: Some(format!(
                    "Expected CFrame or Vector3, got {}",
                    rhs.type_name()
                )),
            })
        });
        methods.add_meta_method(
            LuaMetaMethod::Add,
            |lua, this, vec: VectorArg| {
                charge_host_nanos(lua, price::ADD_VECTOR);
                Ok(*this + Vector3(vec.0))
            },
        );
        methods.add_meta_method(
            LuaMetaMethod::Sub,
            |lua, this, vec: VectorArg| {
                charge_host_nanos(lua, price::ADD_VECTOR);
                Ok(*this - Vector3(vec.0))
            },
        );
    }
}

impl fmt::Display for CFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pos = self.position();
        let transposed = self.orientation().transpose();
        write!(
            f,
            "{}, {}, {}, {}",
            Vector3(pos),
            Vector3(transposed.x_axis),
            Vector3(transposed.y_axis),
            Vector3(transposed.z_axis)
        )
    }
}

impl ops::Mul for CFrame {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        CFrame(self.0 * rhs.0)
    }
}

impl ops::Mul<Vector3> for CFrame {
    type Output = Vector3;
    fn mul(self, rhs: Vector3) -> Self::Output {
        Vector3(self.0.project_point3(rhs.0))
    }
}

impl ops::Add<Vector3> for CFrame {
    type Output = Self;
    fn add(self, rhs: Vector3) -> Self::Output {
        CFrame(Mat4::from_cols(
            self.0.x_axis,
            self.0.y_axis,
            self.0.z_axis,
            self.0.w_axis + rhs.0.extend(0.0),
        ))
    }
}

impl ops::Sub<Vector3> for CFrame {
    type Output = Self;
    fn sub(self, rhs: Vector3) -> Self::Output {
        CFrame(Mat4::from_cols(
            self.0.x_axis,
            self.0.y_axis,
            self.0.z_axis,
            self.0.w_axis - rhs.0.extend(0.0),
        ))
    }
}

impl From<DomCFrame> for CFrame {
    fn from(v: DomCFrame) -> Self {
        let transposed = v.orientation.transpose();
        CFrame(Mat4::from_cols(
            Vector3::from(transposed.x).0.extend(0.0),
            Vector3::from(transposed.y).0.extend(0.0),
            Vector3::from(transposed.z).0.extend(0.0),
            Vector3::from(v.position).0.extend(1.0),
        ))
    }
}

impl From<CFrame> for DomCFrame {
    fn from(v: CFrame) -> Self {
        let transposed = v.orientation().transpose();
        DomCFrame {
            position: DomVector3::from(Vector3(v.position())),
            orientation: DomMatrix3::new(
                DomVector3::from(Vector3(transposed.x_axis)),
                DomVector3::from(Vector3(transposed.y_axis)),
                DomVector3::from(Vector3(transposed.z_axis)),
            ),
        }
    }
}

/**
    Creates a matrix at the position `from`, looking towards `to`.

    [`glam`] does provide functions such as [`look_at_lh`], [`look_at_rh`] and more but
    they all create view matrices for camera transforms which is not what we want here.
*/
fn look_at(from: Vec3, to: Vec3, up: Vec3) -> Mat4 {
    let back = (from - to).normalize_or(Vec3::Z);
    let xaxis = up.cross(back).normalize_or(Vec3::X);
    let yaxis = back.cross(xaxis).normalize();

    Mat4::from_cols(
        xaxis.extend(0.0),
        yaxis.extend(0.0),
        back.extend(0.0),
        from.extend(1.0),
    )
}

#[cfg(test)]
mod cframe_test {
    use glam::{Mat4, Vec3};
    use rbx_dom_weak::types::{CFrame as DomCFrame, Matrix3 as DomMatrix3, Vector3 as DomVector3};

    use super::CFrame;

    #[test]
    fn dom_cframe_from_cframe() {
        let dom_cframe = DomCFrame::new(
            DomVector3::new(1.0, 2.0, 3.0),
            DomMatrix3::new(
                DomVector3::new(1.0, 2.0, 3.0),
                DomVector3::new(1.0, 2.0, 3.0),
                DomVector3::new(1.0, 2.0, 3.0),
            ),
        );

        let cframe = CFrame(Mat4::from_cols(
            Vec3::new(1.0, 1.0, 1.0).extend(0.0),
            Vec3::new(2.0, 2.0, 2.0).extend(0.0),
            Vec3::new(3.0, 3.0, 3.0).extend(0.0),
            Vec3::new(1.0, 2.0, 3.0).extend(1.0),
        ));

        assert_eq!(CFrame::from(dom_cframe), cframe);
    }

    #[test]
    fn cframe_from_dom_cframe() {
        let cframe = CFrame(Mat4::from_cols(
            Vec3::new(1.0, 2.0, 3.0).extend(0.0),
            Vec3::new(1.0, 2.0, 3.0).extend(0.0),
            Vec3::new(1.0, 2.0, 3.0).extend(0.0),
            Vec3::new(1.0, 2.0, 3.0).extend(1.0),
        ));

        let dom_cframe = DomCFrame::new(
            DomVector3::new(1.0, 2.0, 3.0),
            DomMatrix3::new(
                DomVector3::new(1.0, 1.0, 1.0),
                DomVector3::new(2.0, 2.0, 2.0),
                DomVector3::new(3.0, 3.0, 3.0),
            ),
        );

        assert_eq!(DomCFrame::from(cframe), dom_cframe);
    }
}
