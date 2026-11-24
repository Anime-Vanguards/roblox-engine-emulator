//! The Camera's methods that move it or look through it, as a playing client answers them (Studio 0.748, roblox-sim's
//! headless and rendered clients and its server: tools/parity/probes/camera_units.luau, camera_cutoff.luau,
//! camera_interpolate.luau).
//!
//! - PanUnits(n) / TiltUnits(n), on any camera of any CameraType: the yaw of the LookVector snapped to the nearest 45 degrees
//!   and turned by n steps of 45 (n = 1 turns right), or its pitch snapped to the nearest 10 degrees and raised by n steps of
//!   10, held within 81 degrees; the camera then looks along that from Focus less the look times its distance from Focus (1
//!   when it sits on Focus), with no roll, G3D's aim (up is X once the look is within 0.99 of Y). Focus does not move.
//!   PanUnits returns nothing, TiltUnits whether the pitch changed (the camera untouched when it did not). The argument is an int: a number rounds half away from zero, a
//!   number past the int's range (or NaN, an infinity) is the int's least, a numeric string or a boolean is taken, another
//!   value raises "Unable to cast <type> to int" (an Instance "Instance", an EnumItem its enum's name), none "Argument 1
//!   missing or nil". The angles are a client's to about 1e-6: its float steps were not worked out (positions within a few
//!   float32 steps, rotation entries within 1e-6).
//! - ZoomToExtents(cframe, size), size checked first: the rotation kept, the camera backed off from the box's centre along
//!   its look until the sphere around the box's world-aligned bounds (`|R| * size / 2`, the size's sign kept) fits the wider
//!   half angle of the view, `radius / sin(half angle)` (the horizontal one on a wide viewport); a radius under 0.5 counts
//!   as 0.5. Focus becomes the centre, unrotated. 37 of 54 measured positions to the bit, the rest one or two float32 steps
//!   off.
//! - GetLargestCutoffDistance(ignore): five parallel rays, from Focus and from Focus moved by each near-plane corner's offset,
//!   to the near plane's centre and corners; against parts that can collide and are at most 0.95 transparent, the ignore
//!   list (nil holes in it skipped) and its descendants left out. The largest `length - distance to the first hit`, or 0. A
//!   part entering the Workspace is not seen until the spatial index takes it in: at the next physics step or another spatial
//!   query of the Workspace (Raycast, GetPartBoundsInBox, ...; a WorldModel's leaves it waiting; `Cameras::unindexed`, a set of
//!   the waiting parts still in the Workspace), with the parts moved in with it; a moved, resized or removed part counts at
//!   once. A WorldModel's parts in the Workspace are never met (the Workspace's grid holds none of them: measured).
//! - Interpolate(endCFrame, endFocus, duration): the casts from the last argument to the first, then "Interpolation time
//!   must be positive." (NaN passes; also on a camera that is not Scriptable), then "Attempted to use interpolation with a
//!   camera mode other than scriptable.". A Focus write ends the tween (any camera's) at the next render step and InterpolationFinished
//!   fires after its PreRender handlers' deferred work, CFrame and Focus as written; CameraType leaving Scriptable cancels
//!   it, unfired; a new call replaces it, and the replaced one never fires (all measured). How it moves was reported, not
//!   measured (neither roblox-sim's clients nor a Studio left alone run its step): only the CurrentCamera's tween steps,
//!   at the start of each render step before the BindToRenderStep callbacks, by that step's delta; the position, the focus
//!   point and the up vector go linearly, the camera aims from the position at the focus point (G3D's aim with that up),
//!   Focus is the position plus 20 studs along the look, and InterpolationFinished fires once after the PreRender handlers
//!   of the step that reaches the end. A CFrame write mid-tween becomes its end.

use std::f32::consts::PI;
use std::rc::Rc;

use glam::{Affine3A, Mat3, Vec3};
use lune_roblox::datatypes::types::float32 as f32m;
use mlua::prelude::*;

use super::{camera, camera_fov, cframe_arg, cframe_name, frame, query, vector_arg};
use crate::{
    ClassId, Engine, InstanceId, K, NameId, State,
    instance as core,
    lua::{Args, Flow, raise, values},
    services::Behaviors,
    value::Value,
};

type V = [f32; 3];

fn rows_of(transform: &Affine3A) -> f32m::Rot {
    let m = transform.matrix3;
    [[m.x_axis.x, m.y_axis.x, m.z_axis.x], [m.x_axis.y, m.y_axis.y, m.z_axis.y], [m.x_axis.z, m.y_axis.z, m.z_axis.z]]
}

fn affine(rows: &f32m::Rot, position: V) -> Affine3A {
    let column = |j: usize| Vec3::new(rows[0][j], rows[1][j], rows[2][j]);
    Affine3A::from_mat3_translation(Mat3::from_cols(column(0), column(1), column(2)), Vec3::from_array(position))
}

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn lerp(a: V, b: V, t: f32) -> V {
    [f32m::lerp(a[0], b[0], t), f32m::lerp(a[1], b[1], t), f32m::lerp(a[2], b[2], t)]
}

struct Names {
    cframe: NameId,
    focus: NameId,
    transparency: Option<NameId>,
    finished: Option<NameId>,
}

fn names(engine: &Engine) -> Names {
    let reflection = &engine.reflection;
    Names {
        cframe: cframe_name(engine),
        focus: reflection.name_id("Focus").expect("the reflection names Focus"),
        transparency: reflection.name_id("Transparency"),
        finished: reflection.name_id("InterpolationFinished"),
    }
}

/// The camera's CFrame and Focus.
fn frames(engine: &Engine, names: &Names, camera: InstanceId) -> (Affine3A, Affine3A) {
    let state = engine.state.borrow();
    (frame::affine_of(state.dom.raw_get(camera, names.cframe)), frame::affine_of(state.dom.raw_get(camera, names.focus)))
}

fn write(engine: &Engine, camera: InstanceId, name: NameId, transform: &Affine3A) {
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    core::raw_set(&mut state, &engine.reflection, &hooks, camera, name, frame::cframe_value(transform));
}

// PanUnits / TiltUnits

/// An int argument as a client casts it: rounded half away from zero, the int's least past its range.
pub(super) fn int_argument(lua: &Lua, engine: &Engine, args: &Args, at: usize) -> Result<i32, Flow> {
    let value = args.get(at);
    let number = match value {
        LuaValue::Nil => return raise(format!("Argument {at} missing or nil")),
        LuaValue::Number(number) => *number,
        LuaValue::Integer(number) => *number as f64,
        LuaValue::Boolean(on) => f64::from(u8::from(*on)),
        LuaValue::String(_) => match lua.coerce_number(value.clone())? {
            Some(number) => number,
            None => return raise("Unable to cast string to int"),
        },
        other => return raise(format!("Unable to cast {} to int", camera::cast_name(engine, other))),
    };
    let rounded = number.round();
    Ok(if (-2147483648.0..=2147483647.0).contains(&rounded) { rounded as i32 } else { i32::MIN })
}

/// Turns the camera about Focus: `turn` takes the look's (yaw, pitch) and gives the new ones.
fn orbit(engine: &Engine, names: &Names, camera: InstanceId, tilting: bool, turn: impl Fn(f32, f32) -> (f32, f32)) -> bool {
    let (transform, focus) = frames(engine, names, camera);
    let rows = rows_of(&transform);
    let position = frame::translation(&transform).to_array();
    let focus = frame::translation(&focus).to_array();
    // (the look as R * (0, 0, -1): its sums turn -0 into +0, so a look straight down reads yaw -180 as a client's does)
    let look = [0.0 - rows[0][2], 0.0 - rows[1][2], 0.0 - rows[2][2]];
    let offset = sub(position, focus);
    let mut distance = f32m::dot3(offset, offset).sqrt();
    if distance == 0.0 {
        distance = 1.0;
    }
    let pitch_before = look[1].clamp(-1.0, 1.0).asin();
    let (yaw, pitch) = turn((-look[0]).atan2(-look[2]), pitch_before);
    if tilting && pitch == pitch_before {
        return false;
    }
    let aimed = f32m::look_g3d([-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()]);
    let position = add(focus, scale(f32m::column(&aimed, 2), distance));
    write(engine, camera, names.cframe, &affine(&aimed, position));
    pitch != pitch_before
}

fn pan(engine: &Engine, names: &Names, camera: InstanceId, units: i32) {
    let step = PI / 4.0;
    orbit(engine, names, camera, false, |yaw, pitch| ((yaw / step).round() * step - units as f32 * step, pitch));
}

/// TiltUnits: whether the pitch changed; when it did not, the camera is left as it is (measured: TiltUnits(0) on a camera
/// off the 10-degree steps returns true, on one looking level false and a rolled one keeps its roll, at the 81-degree limit
/// true while the float pitch still moves).

fn tilt(engine: &Engine, names: &Names, camera: InstanceId, units: i32) -> bool {
    let step = PI / 18.0;
    let limit = 81.0 * PI / 180.0;
    orbit(engine, names, camera, true, |yaw, pitch| (yaw, ((pitch / step).round() * step + units as f32 * step).clamp(-limit, limit)))
}

// ZoomToExtents

fn zoom_to_extents(engine: &Engine, names: &Names, camera: InstanceId, center: &Affine3A, size: Vec3) {
    let r = rows_of(center);
    let half = [size.x * 0.5, size.y * 0.5, size.z * 0.5];
    let reach = |row: [f32; 3]| (row[0].abs() * half[0] + row[1].abs() * half[1]) + row[2].abs() * half[2];
    let bounds = [reach(r[0]), reach(r[1]), reach(r[2])];
    let mut radius = f32m::dot3(bounds, bounds).sqrt();
    if radius < 0.5 {
        radius = 0.5;
    }
    let radians = camera_fov::radians(&engine.state.borrow(), engine.reflection.name_id("FieldOfView").expect("FieldOfView"), camera);
    let (width, height) = camera_fov::viewport(engine, camera);
    let vertical = radians * 0.5;
    let horizontal = ((radians * 0.5).tan() * (width / height)).atan();
    let distance = radius / horizontal.max(vertical).sin();
    let (transform, _) = frames(engine, names, camera);
    let rows = rows_of(&transform);
    let centre = frame::translation(center).to_array();
    write(engine, camera, names.cframe, &affine(&rows, add(centre, scale(f32m::column(&rows, 2), distance))));
    write(engine, camera, names.focus, &Affine3A::from_translation(Vec3::from_array(centre)));
}

// GetLargestCutoffDistance and the spatial index's lag

/// Which wait a world's parts are in: the Workspace's or a WorldModel's (`Cameras::unindexed`, `unindexed_models`), by its
/// WorldRoot
#[derive(Clone, Copy, PartialEq)]
enum Wait {
    Workspace(InstanceId),
    Model(InstanceId),
}

impl Wait {
    fn insert(self, state: &mut State, part: InstanceId) {
        match self {
            Wait::Workspace(_) => state.cameras.unindexed.insert(part),
            Wait::Model(model) => state.cameras.unindexed_models.entry(model).or_default().insert(part),
        };
        super::broadphase::came_in(state, part);
    }

    /// Out of the wait, and out of the world's index at once (world/broadphase.rs left)
    fn remove(self, state: &mut State, part: InstanceId) {
        match self {
            Wait::Workspace(root) => {
                state.cameras.unindexed.remove(&part);
                super::broadphase::left(state, root, part);
            }
            Wait::Model(model) => {
                if let Some(waiting) = state.cameras.unindexed_models.get_mut(&model) {
                    waiting.remove(&part);
                    if waiting.is_empty() {
                        state.cameras.unindexed_models.remove(&model);
                    }
                }
                super::broadphase::left(state, model, part);
            }
        }
    }

    /// Whether anything waits in it or its world's index holds a part
    fn holds_any(self, state: &State) -> bool {
        match self {
            Wait::Workspace(root) => !state.cameras.unindexed.is_empty() || super::broadphase::holds_any(state, root),
            Wait::Model(model) => state.cameras.unindexed_models.contains_key(&model) || super::broadphase::holds_any(state, model),
        }
    }
}

/// Whether a part GetLargestCutoffDistance's ray meets stops it: in the spatial index, CanCollide on (CanQuery does not matter:
/// measured, a part with CanQuery off and CanCollide on blocks), at most 0.95 transparent (float32), neither in the ignore list
/// (sorted) nor under what it lists; and how many steps up the ignore list's check walked (none with an empty list: a client's
/// call costs the same at any depth then), which a client pays cost.rs CUTOFF_IGNORE_STEP for.
fn blocks(state: &State, names: &Names, can_collide: Option<NameId>, ignore: &[InstanceId], part: InstanceId) -> (bool, u32) {
    if can_collide.and_then(|n| state.dom.boolean(part, n)) == Some(false) || names.transparency.and_then(|n| state.dom.number(part, n)).is_some_and(|t| t as f32 > 0.95) {
        return (false, 0);
    }
    if ignore.is_empty() {
        return (true, 0);
    }
    let (mut node, mut steps) = (Some(part), 0u32);
    while let Some(id) = node {
        if ignore.binary_search(&id).is_ok() {
            return (false, steps);
        }
        node = state.dom.parent_of(id);
        steps += u32::from(node.is_some());
    }
    (true, steps)
}

/// The five rays against the Workspace's parts (not Terrain) as the spatial queries keep them (world/broadphase.rs, the
/// PlayerMouse's and GetPartsObscuringTarget's view: CanQuery off included), so a call walks nothing: each ray tests the parts
/// near it whose bounding sphere meets the box around the rays, and what decides whether a part stops it ([`blocks`]) is looked
/// at only for a part it meets nearer than its hit so far, in the Workspace's order. A part waiting for the spatial index is not
/// met. What a client spends is charged as a client's rays pay (Studio 0.747, cost.rs CUTOFF_CALL): the call, and
/// each ray the parts it meets up to its hit (all of them on a miss), the first [`crate::cost::CUTOFF_MET`] and each other a
/// Raycast's [`crate::cost::RAY_CROSSED`]; the parts off the rays cost nothing (the broadphase).
fn largest_cutoff(engine: &Engine, names: &Names, camera: InstanceId, ignore: Vec<InstanceId>) -> f32 {
    let world = query::refresh(engine);
    let (transform, focus) = frames(engine, names, camera);
    let rows = rows_of(&transform);
    let (right, up) = (f32m::column(&rows, 0), f32m::column(&rows, 1));
    let look = scale(f32m::column(&rows, 2), -1.0);
    let near = camera::near_plane(engine, camera);
    let radians = camera_fov::radians(&engine.state.borrow(), engine.reflection.name_id("FieldOfView").expect("FieldOfView"), camera);
    let (width, height) = camera_fov::viewport(engine, camera);
    let half_y = near * (radians * 0.5).tan();
    let half_x = half_y * (width / height);
    let centre = add(frame::translation(&transform).to_array(), scale(look, near));
    let focus = frame::translation(&focus).to_array();
    let along = sub(centre, focus);
    let length = f32m::dot3(along, along).sqrt();
    let origins = [(0.0, 0.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].map(|(x, y)| Vec3::from_array(add(focus, add(scale(right, x * half_x), scale(up, y * half_y)))));
    let direction = Vec3::from_array(along);
    let reach = direction.length();
    // (each ray lies in the box around its two ends, so only the parts whose bounding sphere meets the box around all ten can be
    // met, as in camera.rs parts_obscuring)
    let (low, high) = origins.iter().fold((origins[0], origins[0]), |(low, high), &origin| (low.min(origin).min(origin + direction), high.max(origin).max(origin + direction)));
    let near = |solid: &query::Solid| {
        let center = frame::translation(&solid.frame);
        center.clamp(low, high).distance(center) <= solid.half_extents().length() * 1.0001 + 1e-4
    };
    let can_collide = engine.reflection.name_id("CanCollide");
    let mut largest = 0.0f32;
    let mut met: Vec<f32> = Vec::new();
    let mut entered: Vec<(InstanceId, f32)> = Vec::new();
    let mut walked: Vec<(f32, u32)> = Vec::new();
    let mut charge = crate::cost::CUTOFF_CALL;
    for origin in origins {
        met.clear();
        entered.clear();
        // (a ray that is not finite meets nothing and tests no part, as a Raycast's: a client's cutoff with a NaN or infinite
        // Focus reads 0, tools/parity/probes/query_finite.luau)
        if !(origin.is_finite() && direction.is_finite() && reach.is_finite()) {
            continue;
        }
        let Some(world) = world else { continue };
        // (in a ray walk's order: a part waiting for the spatial index is in no cell yet)
        query::walk_ray(engine, world, origin, direction, |parts, _| {
            for (part, entry, _) in parts {
                if !near(&entry.solid) {
                    continue;
                }
                let Some((t, _)) = query::ray_solid(&entry.solid, origin, direction, 0.0) else { continue };
                // (query::cast_ray's distance; the nearest part that stops the ray is its hit)
                entered.push((*part, reach * t));
            }
        });
        let state = engine.state.borrow();
        let mut nearest: Option<f32> = None;
        // (each part's ignore check is asked of every part met up to the hit, as a client's ray asks it: its steps kept)
        walked.clear();
        for &(part, distance) in &entered {
            met.push(distance);
            let (stops, steps) = blocks(&state, names, can_collide, &ignore, part);
            walked.push((distance, steps));
            if stops && nearest.is_none_or(|nearest| distance < nearest) {
                nearest = Some(distance);
            }
        }
        if let Some(hit) = nearest {
            largest = largest.max(length - hit);
        }
        let crossed = met.iter().filter(|&&distance| nearest.is_none_or(|hit| distance <= hit)).count();
        if crossed > 0 {
            charge += crate::cost::CUTOFF_MET + (crossed - 1) as f64 * crate::cost::RAY_CROSSED;
        }
        let steps: u32 = walked.iter().filter(|(distance, _)| nearest.is_none_or(|hit| *distance <= hit)).map(|(_, steps)| steps).sum();
        charge += f64::from(steps) * crate::cost::CUTOFF_IGNORE_STEP;
    }
    query::charge_query(engine, charge);
    largest
}

// Interpolate

/// An Interpolate tween: where it starts and ends (position, focus point, up vector), its length and how far it is.
pub struct Tween {
    from: [V; 3],
    to: [V; 3],
    duration: f32,
    elapsed: f32,
    /// A Focus write: the tween ends at the next render step
    ending: bool,
}

fn ends(transform: &Affine3A, focus: &Affine3A) -> [V; 3] {
    [frame::translation(transform).to_array(), frame::translation(focus).to_array(), f32m::column(&rows_of(transform), 1)]
}

/// G3D's aim at `target` with an up vector: `up` normalized, X once the look is within 0.99 of it, then as lune's look_g3d.
fn aim(from: V, target: V, up: V) -> f32m::Rot {
    let look = f32m::unit3(sub(target, from));
    let mut up = f32m::unit3(up);
    if f32m::dot3(look, up).abs() > 0.99 {
        up = [1.0, 0.0, 0.0];
        if f32m::dot3(look, up).abs() > 0.99 {
            up = [0.0, 1.0, 0.0];
        }
    }
    let d = f32m::dot3(look, up);
    let u = f32m::unit3(sub(up, scale(look, d)));
    let z = scale(look, -1.0);
    let x = f32m::unit3(scale(f32m::cross3(z, u), -1.0));
    let y = f32m::cross3(z, x);
    f32m::from_columns(x, y, z)
}

fn current_camera(engine: &Engine) -> Option<InstanceId> {
    match engine.state.borrow().dom.at(engine.world().workspace).stored(K::CurrentCamera as NameId) {
        Some(Value::Instance(id)) => Some(*id),
        _ => None,
    }
}

/// The CurrentCamera's tween steps by `delta`, at the start of a client's render step (before the BindToRenderStep
/// callbacks; services' render-step hook).
pub fn step(engine: &Engine, delta: f64) {
    if engine.state.borrow().cameras.tweens.is_empty() {
        return;
    }
    // (a Focus write ends a tween on any camera: measured, a camera that is not the CurrentCamera fires too)
    {
        let mut state = engine.state.borrow_mut();
        let cameras = &mut state.cameras;
        let mut ended: Vec<InstanceId> = cameras.tweens.iter().filter(|(_, tween)| tween.ending).map(|(&id, _)| id).collect();
        ended.sort_unstable();
        for id in ended {
            cameras.tweens.remove(&id);
            cameras.finished.push(id);
        }
    }
    let Some(camera) = current_camera(engine) else { return };
    let Some(mut tween) = engine.state.borrow_mut().cameras.tweens.remove(&camera) else { return };
    if !tween.ending {
        let names = names(engine);
        tween.elapsed += delta as f32;
        let alpha = (tween.elapsed / tween.duration).min(1.0);
        let position = lerp(tween.from[0], tween.to[0], alpha);
        let target = lerp(tween.from[1], tween.to[1], alpha);
        let rows = aim(position, target, lerp(tween.from[2], tween.to[2], alpha));
        let focus = add(position, scale(f32m::column(&rows, 2), -20.0));
        engine.state.borrow_mut().cameras.stepping = true;
        write(engine, camera, names.cframe, &affine(&rows, position));
        write(engine, camera, names.focus, &Affine3A::from_translation(Vec3::from_array(focus)));
        engine.state.borrow_mut().cameras.stepping = false;
        if alpha < 1.0 {
            engine.state.borrow_mut().cameras.tweens.insert(camera, tween);
            return;
        }
    }
    engine.state.borrow_mut().cameras.finished.push(camera);
}

fn camera_argument(engine: &Engine, args: &Args, at: usize) -> Result<Affine3A, Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    cframe_arg(value).map_or_else(|| raise(format!("Unable to cast {} to CoordinateFrame", camera::cast_name(engine, value))), Ok)
}

/// Camera's deprecated CoordinateFrame and focus read and write CFrame and Focus; a CFrame change fires CoordinateFrame's
/// Changed after CFrame's, a Focus change none for focus (measured: PanUnits fires "CFrame, CoordinateFrame",
/// ZoomToExtents "CFrame, CoordinateFrame, Focus"). Called once the camera's behaviours are in (derived::add_view takes them).
pub(super) fn install_aliases(engine: &Rc<Engine>) {
    for (alias, source, events) in [("CoordinateFrame", "CFrame", crate::derived::ViewEvents::After), ("focus", "Focus", crate::derived::ViewEvents::None)] {
        let Some(target) = engine.reflection.name_id(source) else { continue };
        let read: crate::derived::ReadFn = Rc::new(|value| value.cloned().unwrap_or_else(|| frame::cframe_value(&Affine3A::IDENTITY)));
        let write: crate::derived::WriteFn = Rc::new(move |state, reflection, hooks, id, value| {
            core::raw_set(state, reflection, hooks, id, target, value);
        });
        crate::derived::add_view(engine, "Camera", alias, source, events, read, write);
    }
}

pub(super) fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors, camera_class: ClassId) {
    let mut add_method = |member: &str, f: crate::services::MethodFn| {
        if let Some(name) = engine.reflection.name_id(member) {
            behaviors.add_method(camera_class, name, f);
        }
    };
    add_method("PanUnits", Rc::new(|lua, engine, id, args| {
        let units = int_argument(lua, engine, args, 1)?;
        pan(engine, &names(engine), id, units);
        Ok(LuaMultiValue::new())
    }));
    add_method("TiltUnits", Rc::new(|lua, engine, id, args| {
        let units = int_argument(lua, engine, args, 1)?;
        Ok(values([LuaValue::Boolean(tilt(engine, &names(engine), id, units))]))
    }));
    add_method("ZoomToExtents", Rc::new(|_, engine, id, args| {
        let size = match args.get(2) {
            LuaValue::Nil => return raise("Argument 2 missing or nil"),
            value => vector_arg(value).map_or_else(|| raise(format!("Unable to cast {} to Vector3", camera::cast_name(engine, value))), Ok)?,
        };
        let center = camera_argument(engine, args, 1)?;
        zoom_to_extents(engine, &names(engine), id, &center, size);
        Ok(LuaMultiValue::new())
    }));
    add_method("GetLargestCutoffDistance", Rc::new(|_, engine, id, args| {
        let ignore = super::ignore_list(engine, args, 1)?;
        Ok(values([LuaValue::Number(f64::from(largest_cutoff(engine, &names(engine), id, ignore)))]))
    }));
    add_method("Interpolate", Rc::new(|lua, engine, id, args| {
        let duration = camera::float_argument(lua, engine, args, 3)?;
        let end_focus = camera_argument(engine, args, 2)?;
        let end = camera_argument(engine, args, 1)?;
        if duration <= 0.0 {
            return raise("Interpolation time must be positive.");
        }
        if !camera::scriptable(engine, id) {
            return raise("Attempted to use interpolation with a camera mode other than scriptable.");
        }
        let names = names(engine);
        let (transform, focus) = frames(engine, &names, id);
        let tween = Tween { from: ends(&transform, &focus), to: ends(&end, &end_focus), duration, elapsed: 0.0, ending: false };
        engine.state.borrow_mut().cameras.tweens.insert(id, tween);
        Ok(LuaMultiValue::new())
    }));

    let names = names(engine);
    let (cframe, focus, finished) = (names.cframe, names.focus, names.finished);
    let mut hooks = engine.hooks.borrow_mut();
    // A script's write mid-tween: a CFrame becomes the end, a Focus ends it at the next render step
    hooks.property_changed.push(Box::new(move |state, id, name| {
        if state.cameras.stepping || state.cameras.tweens.is_empty() || (name != cframe && name != focus) {
            return;
        }
        let written = frame::affine_of(state.dom.raw_get(id, name));
        let Some(tween) = state.cameras.tweens.get_mut(&id) else { return };
        if name == focus {
            tween.ending = true;
        } else {
            tween.to[0] = frame::translation(&written).to_array();
            tween.to[2] = f32m::column(&rows_of(&written), 1);
        }
    }));
    // The parts that come into a world (with what they are moved in) wait for the spatial index, each world's apart, and those
    // that leave it before then drop out of its wait. A part's world is its nearest WorldRoot's (dom.rs Dom::home), the Workspace
    // or a WorldModel in the DataModel. Measured on a client (world/region.rs): a part moved between the Workspace and a WorldModel
    // in it, either way, waits; a WorldModel moved between ReplicatedStorage and the Workspace keeps its parts in; one that comes
    // into the DataModel brings them in anew, and one that leaves it takes them out of the wait. A move reads its worlds from the
    // homes above the old and new parents (walking up only from the outermost home above a WorldModel to the DataModel). It
    // walks the moved subtree when the subtree's own parts change worlds, or when, being a WorldModel or holding anything while
    // WorldModels exist, it came into the DataModel or left it (asked up the parents of a parent in no world), to find the
    // WorldModels in it: a Parent write pays for each instance it carries (cost.rs reparented), so that walk is paid for.
    if let (Some(world_root), Some(world_model), Some(base_part), Some(data_model)) =
        (engine.reflection.class_id("WorldRoot"), engine.reflection.class_id("WorldModel"), engine.reflection.class_id("BasePart"), engine.reflection.class_id("DataModel"))
    {
        let reflection = engine.reflection.clone();
        hooks.hierarchy_changed.push(Box::new(move |state, id, old, new| {
            let models = state.dom.count_of_class(world_model) > 0;
            let state = &mut *state;
            let is_a = |state: &State, node: InstanceId, class: ClassId| reflection.class(state.dom.at(node).class).is_a(class);
            // (whether an instance is in the DataModel: up the parents of the outermost home above it, dom.rs Dom::homes_above,
            // one lookup however many WorldModels are nested above it; from the Workspace, one step)
            let in_game = |state: &State, node: InstanceId| {
                let mut current = Some(state.dom.homes_above(node).top.unwrap_or(node));
                while let Some(node) = current {
                    if state.dom.at(node).class == data_model {
                        return true;
                    }
                    current = state.dom.parent_of(node);
                }
                false
            };
            // a parent's nearest WorldRoot (itself, or the nearest above it)
            let nearest = |state: &State, parent: Option<InstanceId>| {
                parent.and_then(|parent| if is_a(state, parent, world_root) { Some(parent) } else { super::world_root_of(state, &reflection, world_root, parent) })
            };
            let (before, after) = (nearest(state, old), nearest(state, new));
            // the wait of the moved subtree's own parts (those below no WorldRoot in it), before and after: a world's while its
            // WorldRoot is in the DataModel (the Workspace always is)
            let wait = |state: &State, root: Option<InstanceId>| match root {
                Some(root) if is_a(state, root, world_model) => in_game(state, root).then_some(Wait::Model(root)),
                Some(root) => Some(Wait::Workspace(root)),
                None => None,
            };
            let (from, to) = (wait(state, before), wait(state, after));
            super::broadphase::fresh_write(state);
            let moves = from != to && (to.is_some() || from.is_some_and(|from| from.holds_any(state)));
            // the WorldModels in the subtree (it too, if it is one) come into the DataModel with it, or leave it: asked only of a
            // subtree that can hold one
            let (mut enters, mut leaves) = (false, false);
            if models && (is_a(state, id, world_model) || !state.dom.children_of(id).is_empty()) {
                let in_data_model = |parent: Option<InstanceId>, root: Option<InstanceId>| match (parent, root) {
                    (None, _) => false,
                    (Some(_), Some(root)) => !is_a(state, root, world_model) || in_game(state, root),
                    (Some(parent), None) => in_game(state, parent),
                };
                let (was_in, now_in) = (in_data_model(old, before), in_data_model(new, after));
                (enters, leaves) = (now_in && !was_in, was_in && !now_in);
            }
            if !(moves || enters || leaves) {
                return;
            }
            // (each node with the nearest WorldModel above it within the subtree, if any: the subtree's own parts are below none; the
            // walk goes into a WorldModel only when WorldModels come in or leave)
            let mut stack = std::mem::take(&mut state.cameras.walk);
            let mut parts = std::mem::take(&mut state.cameras.walked);
            stack.clear();
            parts.clear();
            stack.push((id, None));
            while let Some((node, below)) = stack.pop() {
                let model = is_a(state, node, world_root);
                let below = if model { Some(node) } else { below };
                if below.is_none() || enters || leaves {
                    stack.extend(state.dom.children_of(node).iter().map(|&child| (child, below)));
                }
                // a WorldModel that leaves the DataModel takes its parts out of the wait and its index with it; one that comes in
                // has them come in anew (world/broadphase.rs makes its index when it takes them in)
                if model && leaves {
                    state.cameras.unindexed_models.remove(&node);
                    super::broadphase::world_gone(state, node);
                }
                if is_a(state, node, base_part) {
                    parts.push((node, below));
                }
            }
            // (the parts in, or out, each after its own descendants: the reverse of the walk, which takes children last first; a
            // client takes a subtree in so, measured: tools/parity/probes/grid_order.luau)
            for &(node, below) in parts.iter().rev() {
                match below {
                    None if moves => {
                        if let Some(from) = from {
                            from.remove(state, node);
                        }
                        if let Some(to) = to {
                            to.insert(state, node);
                        }
                    }
                    // its world's parts come in anew
                    Some(model) if enters => Wait::Model(model).insert(state, node),
                    _ => {}
                }
            }
            state.cameras.walk = stack;
            state.cameras.walked = parts;
        }));
    }
    // A WorldModel freed takes its index and its wait with it, and a part freed leaves any index (world/region.rs)
    hooks.instance_freed.push(Box::new(|state, id| {
        super::broadphase::world_gone(state, id);
        super::broadphase::freed(state, id);
        state.cameras.unindexed_models.remove(&id);
    }));
    drop(hooks);
    // InterpolationFinished, after the PreRender handlers (and what they deferred)
    if let Some(finished) = finished {
        engine.state.borrow_mut().sched.add_hook(crate::scheduler::Phase::PreRender, Rc::new(move |_, engine, _| {
            let mut state = engine.state.borrow_mut();
            for camera in std::mem::take(&mut state.cameras.finished) {
                core::fire_event(&mut state, camera, finished, Vec::new());
            }
            Ok(())
        }));
    }
}
