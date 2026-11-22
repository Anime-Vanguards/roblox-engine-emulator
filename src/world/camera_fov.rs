//! A Camera's DiagonalFieldOfView and MaxAxisFieldOfView: views of its FieldOfView through the camera's viewport, as a
//! playing client answers them (Studio 0.748; roblox-sim's headless client, whose cameras are 1 x 1, and its rendered
//! 1920 x 1080 one: tools/parity/probes/camera_fov.luau and camera_fov_wide.luau, every value to the bit).
//!
//! - A client keeps the field of view in radians, `FieldOfView * pi / 180` in float32, and reads FieldOfView back as
//!   `radians * 180 / pi` (119 reads 119.00000762939453). ree stores FieldOfView in degrees; the radians come from it,
//!   which is what a client holds after a FieldOfView write, and a view's write keeps the radians it computed
//!   (`Cameras::radians`) so the views read back as a client's do.
//! - With `t = tan(radians / 2)` and `a` the viewport's width over its height (the viewport in whole pixels), all float32:
//!   DiagonalFieldOfView is `2 * atan(t * t * (1 + a * a))` in degrees, with no square root (88.88 for 70 degrees in a 1 x 1
//!   viewport, below FieldOfView from 60 down), and MaxAxisFieldOfView the larger of FieldOfView and the horizontal
//!   field of view `2 * atan(t * a)`.
//! - A write sets the radians through the inverse: DiagonalFieldOfView `2 * atan(sqrt(tan(d / 2) / (1 + a * a)))`,
//!   MaxAxisFieldOfView `2 * atan(tan(m / 2) * (h / w))` on a wide viewport and `2 * atan(tan(m / 2))` otherwise, held
//!   within 1 and 120 degrees (180 reads 1 through MaxAxisFieldOfView: tan of a float32 right angle is negative; 180
//!   through DiagonalFieldOfView and infinity through either read not-a-number). A NaN is ignored; a numeric string is its
//!   number and anything else (a boolean, a table, a Vector3) is 0, with no error. 98 of 100 writes are to the bit; two
//!   DiagonalFieldOfView writes on the wide viewport (90 and 120) read FieldOfView 2 and 1 float32 steps off.
//! - Changed fires for the member written, then FieldOfView, then the third (a FieldOfView write: FieldOfView,
//!   DiagonalFieldOfView, MaxAxisFieldOfView), each Changed before its property signal; a write that leaves the radians as
//!   they were fires nothing. FieldOfViewMode changes none of this in a client.

use std::f32::consts::PI;
use std::rc::Rc;

use mlua::prelude::*;

use crate::{
    ClassId, Engine, InstanceId, NameId, State,
    instance as core,
    lua::Flow,
    services::Behaviors,
    value::Value,
};

/// The camera's viewport in whole pixels (width, height), as the projections use it.
pub(super) fn viewport(engine: &Engine, camera: InstanceId) -> (f32, f32) {
    let (width, height) = super::camera::viewport_size(engine, camera);
    (width.round().max(1.0), height.round().max(1.0))
}

/// The camera's field of view in radians, as a client holds it.
pub(super) fn radians(state: &State, fov: NameId, camera: InstanceId) -> f32 {
    let degrees = state.dom.number(camera, fov).unwrap_or(70.0) as f32;
    match state.cameras.radians.get(&camera) {
        Some(&(bits, radians)) if bits == degrees.to_bits() => radians,
        _ => degrees * PI / 180.0,
    }
}

fn diagonal(radians: f32, (width, height): (f32, f32)) -> f32 {
    let a = width / height;
    let t = (radians * 0.5).tan();
    (t * t * (1.0 + a * a)).atan() * 2.0 * 180.0 / PI
}

fn max_axis(radians: f32, (width, height): (f32, f32)) -> f32 {
    let t = (radians * 0.5).tan();
    let horizontal = (t * (width / height)).atan() * 2.0 * 180.0 / PI;
    horizontal.max(radians * 180.0 / PI)
}

/// What a view's write takes: a number, a numeric string's number, 0 for anything else.
fn written(lua: &Lua, value: &LuaValue) -> LuaResult<f32> {
    Ok(match value {
        LuaValue::Number(number) => *number as f32,
        LuaValue::Integer(number) => *number as f32,
        LuaValue::String(_) => lua.coerce_number(value.clone())?.unwrap_or(0.0) as f32,
        _ => 0.0,
    })
}

struct Names {
    fov: NameId,
    diagonal: NameId,
    max_axis: NameId,
}

/// Stores the radians a view's write computed: its own Changed, then FieldOfView's (and the third view's, from the hook).
fn store(engine: &Engine, names: &Names, camera: InstanceId, view: NameId, radians: f32) {
    let radians = if radians.is_nan() { radians } else { radians.clamp(PI / 180.0, 120.0 * PI / 180.0) };
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    if self::radians(&state, names.fov, camera).to_bits() == radians.to_bits() {
        return;
    }
    let degrees = radians * 180.0 / PI;
    core::fire_computed_changed(&mut state, &engine.reflection, camera, view);
    state.cameras.radians.insert(camera, (degrees.to_bits(), radians));
    state.cameras.writing_view = Some(view);
    core::raw_set(&mut state, &engine.reflection, &hooks, camera, names.fov, Value::Number(f64::from(degrees)));
    state.cameras.writing_view = None;
}

pub(super) fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors, camera: ClassId) {
    let reflection = &engine.reflection;
    let (Some(fov), Some(diagonal_name), Some(max_axis_name)) = (reflection.name_id("FieldOfView"), reflection.name_id("DiagonalFieldOfView"), reflection.name_id("MaxAxisFieldOfView")) else { return };
    let names = Rc::new(Names { fov, diagonal: diagonal_name, max_axis: max_axis_name });
    behaviors.add_getter(camera, diagonal_name, Rc::new(move |_, engine, id| {
        let radians = radians(&engine.state.borrow(), fov, id);
        Ok(LuaValue::Number(f64::from(diagonal(radians, viewport(engine, id)))))
    }));
    behaviors.add_getter(camera, max_axis_name, Rc::new(move |_, engine, id| {
        let radians = radians(&engine.state.borrow(), fov, id);
        Ok(LuaValue::Number(f64::from(max_axis(radians, viewport(engine, id)))))
    }));
    let for_diagonal = names.clone();
    behaviors.add_setter(camera, diagonal_name, Rc::new(move |lua, engine, id, value| {
        let degrees = written(lua, &value)?;
        if degrees.is_nan() {
            return Ok(());
        }
        let (width, height) = viewport(engine, id);
        let a = width / height;
        let x = (degrees * PI / 180.0 * 0.5).tan();
        store(engine, &for_diagonal, id, for_diagonal.diagonal, (x / (1.0 + a * a)).sqrt().atan() * 2.0);
        Ok::<(), Flow>(())
    }));
    let for_max_axis = names.clone();
    behaviors.add_setter(camera, max_axis_name, Rc::new(move |lua, engine, id, value| {
        let degrees = written(lua, &value)?;
        if degrees.is_nan() {
            return Ok(());
        }
        let (width, height) = viewport(engine, id);
        let x = (degrees * PI / 180.0 * 0.5).tan();
        let t = if width > height { x * (height / width) } else { x };
        store(engine, &for_max_axis, id, for_max_axis.max_axis, t.atan() * 2.0);
        Ok(())
    }));
    // A FieldOfView change, by any write: the views' Changed after FieldOfView's (but the one a view write fired already)
    let reflection = engine.reflection.clone();
    engine.hooks.borrow_mut().property_changed.push(Box::new(move |state, id, name| {
        if name != names.fov || state.quiet_write || state.dom.at(id).class != camera {
            return;
        }
        let writing = state.cameras.writing_view;
        for view in [names.diagonal, names.max_axis] {
            if writing != Some(view) {
                core::fire_computed_changed(state, &reflection, id, view);
            }
        }
    }));
}
