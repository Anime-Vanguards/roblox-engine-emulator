//! The Camera's projections: WorldToViewportPoint / WorldToScreenPoint and ViewportPointToRay /
//! ScreenPointToRay, from the camera's CFrame, its vertical FieldOfView and its viewport. On the camera with the
//! screen's viewport the screen forms are measured from below GuiService's top inset, where GUI positions are
//! (AbsolutePosition, InputObject.Position): WorldToScreenPoint's Y is WorldToViewportPoint's less the inset, and
//! ScreenPointToRay(x, y) is ViewportPointToRay(x, y + inset). A camera in a 1 x 1 viewport takes no inset: its screen
//! forms are its viewport forms (measured on a rendered desktop client under a 58-pixel inset: a new camera, one outside
//! the Workspace and one just made current; the start camera keeps the inset when another is made current).
//!
//! Measured in Studio 0.748, a playing client (and its server, for the viewport and near plane):
//! - Only the camera a run starts with has a viewport (the window) and a near plane 0.1 studs out (NearPlaneZ -0.1). Any other
//!   camera reads ViewportSize 1, 1 and NearPlaneZ -0.5 and projects into that 1 x 1 viewport (the start camera keeps its
//!   values when it is no longer current). A camera made the CurrentCamera reads 1 x 1 until the render step after the
//!   PreRender handlers, which gives it the window and a near plane 0.1 out (both Changed fire) for good. Not emulated: here a
//!   script's write of Workspace.CurrentCamera does not take, so the start camera stays the current one.
//! - The projections use the viewport rounded to whole pixels (908.8 x 895.2 projects as 909 x 895: the centre is 454.5, 447.5).
//! - A ray starts on the near plane (its point under the pixel, `-NearPlaneZ` along the look vector), `depth` studs further along
//!   it; the point at the camera itself projects to NaN, NaN at depth 0.
//! - FieldOfView stays within 1 to 120 degrees; a NaN written is ignored.
//!
//! Also the Camera's simpler methods, as a playing client answers them (Studio 0.748: results, result counts, argument errors
//! and values, on new cameras and the CurrentCamera): GetRoll / SetRoll and GetRenderCFrame, GetPanSpeed / GetTiltSpeed,
//! and SetCameraPanMode. (Zoom and SetImageServerView, which a game script cannot call, are reflection.rs CAPABILITY_CALLS.)
//!
//! And GetPartsObscuringTarget, as a client answers it (measured on new cameras and the CurrentCamera: which parts, in what
//! order, the ignore list and the argument errors).

use std::rc::Rc;

use glam::{Affine3A, Mat3, Vec3};
use lune_roblox::datatypes::types as lune;
use lune_roblox::datatypes::types::float32 as f32m;
use mlua::prelude::*;
use rbx_dom_weak::types as dom;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{cframe_name, frame, method, query, vector_arg};
use crate::{
    Engine, InstanceId, NameId,
    lua::{Args, Flow, convert, instance_id, instance_value, raise, typeof_value, types::LEnumItem, values},
    value::Value,
};

/// What cameras hold beyond their properties (`State::cameras`): the roll SetRoll gave a camera, float32. A CFrame write keeps
/// it; CameraType leaving Scriptable drops it (measured: Fixed and back to Scriptable reads 0). Also the field of view a
/// DiagonalFieldOfView or MaxAxisFieldOfView write left in radians, Interpolate's tweens and the parts the spatial index has
/// not taken in yet (camera_fov.rs, camera_moves.rs).
#[derive(Default)]
pub struct Cameras {
    rolls: FxHashMap<InstanceId, f32>,
    /// A client keeps the field of view in radians: what a DiagonalFieldOfView / MaxAxisFieldOfView write stored, with the
    /// FieldOfView it reads as (float32 bits), while FieldOfView still holds that
    pub(super) radians: FxHashMap<InstanceId, (u32, f32)>,
    /// The field-of-view member a write is going through (its own Changed fired first)
    pub(super) writing_view: Option<NameId>,
    /// Interpolate's tweens, by camera
    pub(super) tweens: FxHashMap<InstanceId, super::camera_moves::Tween>,
    /// Cameras whose tween ended this render step: InterpolationFinished fires after the PreRender handlers
    pub(super) finished: Vec<InstanceId>,
    /// An Interpolate step is writing CFrame and Focus (not a script's write)
    pub(super) stepping: bool,
    /// The parts that entered the Workspace since the spatial index last took parts in (the physics step, another spatial
    /// query) and are still in it: GetLargestCutoffDistance and the PlayerMouse's rays do not see them yet (camera_moves.rs,
    /// mouse.rs). A set, each part once however often it came back in, and a part that leaves drops out, so it holds at
    /// most the Workspace's parts and a ray's lookup costs the same whatever it holds
    pub(super) unindexed: FxHashSet<InstanceId>,
}

struct View {
    transform: Affine3A,
    tan_y: f32,
    width: f32,
    height: f32,
    near: f32,
}

/// Whether the camera has the run's screen for its viewport: only the camera the run starts with does.
pub fn has_screen(engine: &Engine, camera: InstanceId) -> bool {
    camera == engine.world().camera
}

/// The camera's ViewportSize: the run's screen (`--camera` / the viewport) for the camera the run starts with, 1 x 1 for any
/// other camera.
pub fn viewport_size(engine: &Engine, camera: InstanceId) -> (f32, f32) {
    if has_screen(engine, camera) {
        let width = engine.options.camera_width.unwrap_or(engine.options.viewport_width).max(1.0);
        let height = engine.options.camera_height.unwrap_or(engine.options.viewport_height).max(1.0);
        (width as f32, height as f32)
    } else {
        (1.0, 1.0)
    }
}

/// How far ahead of the camera its near plane is (`-NearPlaneZ`): 0.1 for the camera the run starts with, 0.5 for others.
pub fn near_plane(engine: &Engine, camera: InstanceId) -> f32 {
    if has_screen(engine, camera) { 0.1 } else { 0.5 }
}

/// The top inset the camera's screen forms measure from: GuiService's (`Options::topbar_inset`) for the camera with the screen's
/// viewport, none for a camera in its 1 x 1 viewport, whose screen forms are its viewport forms.
fn screen_inset(engine: &Engine, camera: InstanceId) -> Option<f32> {
    has_screen(engine, camera).then_some(engine.options.topbar_inset as f32)
}

/// `tan(degrees / 2)` with the client's roundings: `degrees * pi / 180 / 2` in float32.
fn half_tan(degrees: f32) -> f32 {
    (degrees * std::f32::consts::PI / 180.0 / 2.0).tan()
}

fn view(engine: &Engine, camera: InstanceId) -> View {
    let state = engine.state.borrow();
    let transform = frame::affine_of(state.dom.raw_get(camera, cframe_name(engine)));
    let fov = engine.reflection.name_id("FieldOfView").and_then(|n| state.dom.number(camera, n)).unwrap_or(70.0);
    let (width, height) = viewport_size(engine, camera);
    View { transform, tan_y: half_tan(fov as f32), width: width.round().max(1.0), height: height.round().max(1.0), near: near_plane(engine, camera) }
}

impl View {
    /// The point in viewport pixels (x right, y down) and its depth in studs in front of the camera,
    /// and whether it is in front of the camera and inside the viewport.
    ///
    /// As a client computes it (18 of 18 measured values to the bit): the projection and view matrices multiplied into one in
    /// float32 (`f = 1 / tan(v / 2)`, the x row scaled by `f / aspect`, the view's translation `-(row . camera)` folded in, so a
    /// camera far from the origin loses bits to cancellation as a client does), the point through it, `1 / w` taken once.
    fn project(&self, point: Vec3) -> (Vec3, bool) {
        let m = self.transform.matrix3;
        let c = Vec3::from(self.transform.translation);
        let rows = [Vec3::from(m.x_axis), Vec3::from(m.y_axis), Vec3::from(m.z_axis)];
        let shift = |row: Vec3| -((row.x * c.x + row.y * c.y) + row.z * c.z);
        let f = 1.0 / self.tan_y;
        let sx = f / (self.width / self.height);
        let m0 = [sx * rows[0].x, sx * rows[0].y, sx * rows[0].z, sx * shift(rows[0])];
        let m1 = [f * rows[1].x, f * rows[1].y, f * rows[1].z, f * shift(rows[1])];
        let m3 = [-rows[2].x, -rows[2].y, -rows[2].z, -shift(rows[2])];
        let dot = |r: [f32; 4]| ((r[0] * point.x + r[1] * point.y) + r[2] * point.z) + r[3];
        let (cx, cy, cw) = (dot(m0), dot(m1), dot(m3));
        let iw = 1.0 / cw;
        let x = (cx * iw * 0.5 + 0.5) * self.width;
        let y = (0.5 - cy * iw * 0.5) * self.height;
        let visible = cw > 0.0 && (0.0..=self.width).contains(&x) && (0.0..=self.height).contains(&y);
        (Vec3::new(x, y, cw), visible)
    }

    /// The unit ray through a viewport pixel, from its point on the near plane, `depth` studs along it. As a client computes it
    /// (21 of 21 origins and directions measured to the bit): the pixel's offset from the viewport's centre over the focal length
    /// in pixels `(h / 2) / tan(v / 2)` (one for both axes), made a unit vector (lune's unit3), then the origin `near / -z` along it.
    fn ray(&self, x: f32, y: f32, depth: f32) -> (Vec3, Vec3) {
        let focal = (self.height * 0.5) / self.tan_y;
        let local = f32m::unit3([(x - self.width * 0.5) / focal, (self.height * 0.5 - y) / focal, -1.0]);
        let direction = self.transform.transform_vector3(Vec3::from_array(local));
        let along = self.near / (0.0 - local[2]);
        let camera = Vec3::from(self.transform.translation);
        (camera + direction * (along + depth), direction)
    }
}

/// ViewportPointToRay of `camera`: the ray through viewport point (x, y), from the near plane, `depth` studs along it (the
/// PlayerMouse's UnitRay: world/mouse.rs).
pub fn viewport_ray(engine: &Engine, camera: InstanceId, x: f32, y: f32, depth: f32) -> (Vec3, Vec3) {
    view(engine, camera).ray(x, y, depth)
}

fn point(at: Vec3, visible: bool) -> LuaResult<LuaMultiValue> {
    Ok(LuaMultiValue::from_vec(vec![LuaValue::Vector(mlua::Vector::new(at.x, at.y, at.z)), LuaValue::Boolean(visible)]))
}

fn ray(lua: &Lua, (origin, direction): (Vec3, Vec3)) -> LuaResult<LuaMultiValue> {
    let ray = lune::Ray::from(dom::Ray::new(dom::Vector3::new(origin.x, origin.y, origin.z), dom::Vector3::new(direction.x, direction.y, direction.z)));
    LuaValue::UserData(lua.create_userdata(ray)?).into_lua_multi(lua)
}

/// What a value is called in a cast error, as a client names it: a number "double", a boolean "bool", a table "Array" (empty or
/// with an array part) or "Dictionary", an Instance "Instance", an EnumItem its enum ("Unable to cast KeyCode to float"), a CFrame
/// "CoordinateFrame", a function "Function", an RBXScriptSignal "EventInstance", an Enum, Enums, thread or plain userdata "null",
/// any other value its typeof. Measured on a playing client and its server (Studio 0.748), the same in each of the Camera's casts
/// (to float, int, Vector3, CoordinateFrame, CameraPanMode and Array) and in BulkMoveTo's.
pub(super) fn cast_name(engine: &Engine, value: &LuaValue) -> String {
    match value {
        LuaValue::Number(_) | LuaValue::Integer(_) => "double".into(),
        LuaValue::Boolean(_) => "bool".into(),
        LuaValue::Table(table) => {
            let array = table.raw_len() > 0 || table.clone().pairs::<LuaValue, LuaValue>().next().is_none();
            (if array { "Array" } else { "Dictionary" }).into()
        }
        LuaValue::Function(_) => "Function".into(),
        LuaValue::Thread(_) => "null".into(),
        other if instance_id(other).is_some() => "Instance".into(),
        other => {
            if let LuaValue::UserData(ud) = other
                && let Ok(item) = ud.borrow::<LEnumItem>()
            {
                return engine.reflection.enum_item_type_name(item.0).to_string();
            }
            match typeof_value(engine, other).as_str() {
                "CFrame" => "CoordinateFrame".into(),
                "RBXScriptSignal" => "EventInstance".into(),
                "Enum" | "Enums" | "userdata" => "null".into(),
                name => name.into(),
            }
        }
    }
}

/// A float argument: a number, a boolean (true is 1), or a string that reads as a number; float32.
pub(super) fn float_argument(lua: &Lua, engine: &Engine, args: &Args, at: usize) -> Result<f32, Flow> {
    match args.get(at) {
        LuaValue::Nil => raise(format!("Argument {at} missing or nil")),
        LuaValue::Number(number) => Ok(*number as f32),
        LuaValue::Integer(number) => Ok(*number as f32),
        LuaValue::Boolean(on) => Ok(u8::from(*on) as f32),
        value @ LuaValue::String(_) => match lua.coerce_number(value.clone())? {
            Some(number) => Ok(number as f32),
            None => raise("Unable to cast string to float"),
        },
        other => raise(format!("Unable to cast {} to float", cast_name(engine, other))),
    }
}

/// The world point argument of WorldTo*Point.
fn vector3_argument(engine: &Engine, args: &Args, at: usize) -> Result<Vec3, Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    match vector_arg(value) {
        Some(vector) => Ok(vector),
        None => raise(format!("Unable to cast {} to Vector3", cast_name(engine, value))),
    }
}

/// A ViewportPointToRay / ScreenPointToRay call's (x, y, depth), each a float argument, checked from the last to the first: the
/// depth (nil takes 0), then y, then x (measured: no arguments raise "Argument 2 missing or nil", (Instance, 1, KeyCode) "Unable to
/// cast KeyCode to float").
fn ray_arguments(lua: &Lua, engine: &Engine, args: &Args) -> Result<(f32, f32, f32), Flow> {
    let depth = if args.get(3).is_nil() { 0.0 } else { float_argument(lua, engine, args, 3)? };
    let y = float_argument(lua, engine, args, 2)?;
    let x = float_argument(lua, engine, args, 1)?;
    Ok((x, y, depth))
}

fn roll(engine: &Engine, camera: InstanceId) -> f32 {
    engine.state.borrow().cameras.rolls.get(&camera).copied().unwrap_or(0.0)
}

/// Whether the camera's CameraType is Scriptable (SetRoll requires it).
pub(super) fn scriptable(engine: &Engine, camera: InstanceId) -> bool {
    let (Some(name), Some(item)) = (engine.reflection.name_id("CameraType"), engine.reflection.enum_item("CameraType", "Scriptable")) else { return false };
    engine.state.borrow().dom.raw_get(camera, name).and_then(Value::as_enum) == Some(item)
}

/// GetRenderCFrame: the CFrame turned by the roll about its look vector, `CFrame * CFrame.Angles(0, 0, -roll)` in float32 (lune's
/// helpers, by rows; measured to the bit: roll 0.5 turns the right vector toward -up). Computed at roll 0 too, so the default
/// camera's -0 terms read 0, as a client's do.
fn render_cframe(engine: &Engine, camera: InstanceId) -> Affine3A {
    let transform = frame::affine_of(engine.state.borrow().dom.raw_get(camera, cframe_name(engine)));
    let m = transform.matrix3;
    let rows = [[m.x_axis.x, m.y_axis.x, m.z_axis.x], [m.x_axis.y, m.y_axis.y, m.z_axis.y], [m.x_axis.z, m.y_axis.z, m.z_axis.z]];
    let rows = f32m::mat_mul(&rows, &f32m::from_euler("XYZ", 0.0, 0.0, -roll(engine, camera)));
    let column = |j: usize| Vec3::new(rows[0][j], rows[1][j], rows[2][j]);
    Affine3A::from_mat3_translation(Mat3::from_cols(column(0), column(1), column(2)), frame::translation(&transform))
}

fn number(number: f32) -> Result<LuaMultiValue, Flow> {
    Ok(values([LuaValue::Number(f64::from(number))]))
}

/// GetPartsObscuringTarget's ignore list, its second argument, checked before the points: a table whose array part holds instances
/// (nil holes are skipped), sorted, without repeats. Measured: none given "Argument 2 missing or nil", nil "Expected table argument,
/// got nil", an Instance "... got userdata" (world/mod.rs objects_arg), a number in it "Unable to cast value to Object".
fn ignore_list(args: &Args) -> Result<Vec<InstanceId>, Flow> {
    let table = super::objects_arg(args, 2)?;
    let mut ids = Vec::new();
    for index in 1..=table.raw_len() {
        let item: LuaValue = table.raw_get(index)?;
        if item.is_nil() {
            continue;
        }
        match instance_id(&item) {
            Some(id) => ids.push(id),
            None => return raise("Unable to cast value to Object"),
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Whether `part` or an ancestor of it is in `ignored` (sorted).
fn is_ignored(engine: &Engine, ignored: &[InstanceId], part: InstanceId) -> bool {
    let state = engine.state.borrow();
    let mut node = Some(part);
    while let Some(at) = node {
        if ignored.binary_search(&at).is_ok() {
            return true;
        }
        node = state.dom.at(at).parent;
    }
    false
}

/// GetPartsObscuringTarget: for each cast point in turn (entries that are no Vector3 are skipped), the parts the segment from the
/// camera's position to it enters before reaching it, nearest first, each part once over all the points. A part the segment only
/// touches at the point is not counted, a point inside a part counts it, a part holding the camera is not counted (a ray starting
/// inside does not hit). The camera's orientation does not matter.
///
/// The parts are the Workspace's every BasePart but Terrain, whatever its Transparency, CanCollide or CanQuery (measured), from
/// the spatial queries' cache (world/query.rs); an ignored instance or descendant is left out as it is met. What a client spends
/// on it is charged as the work goes (cost.rs OBSCURING_POINT): each point, its segment as a ray for the parts it crosses
/// (ignored ones too: the broadphase meets them all), and each part returned.
fn parts_obscuring(lua: &Lua, engine: &Rc<Engine>, camera: InstanceId, args: &Args) -> Result<LuaMultiValue, Flow> {
    // (the ignore list is checked first: measured, ("x", "y") raises "Expected table argument, got string")
    let ignored = ignore_list(args)?;
    let points = super::array_arg(engine, args.get(1), 1)?;
    let origin = frame::translation(&frame::affine_of(engine.state.borrow().dom.raw_get(camera, cframe_name(engine))));
    let solids = query::cached_solids(engine, &query::Filter { unqueryable: true, ..query::Filter::default() });
    // (the parts listed so far, in order, and every part already decided: listed or ignored)
    let (mut found, mut decided): (Vec<InstanceId>, FxHashSet<InstanceId>) = (Vec::new(), FxHashSet::default());
    let mut targets = Vec::new();
    for index in 1..=points.raw_len() {
        let value: LuaValue = points.raw_get(index)?;
        if let Some(point) = vector_arg(&value) {
            targets.push(point);
        }
    }
    // (a segment lies in the box around the camera and every point, so only the parts whose bounding sphere meets that box can be
    // crossed: found once a call, so each point is tested against those alone, as a broadphase would)
    let (low, high) = targets.iter().fold((origin, origin), |(low, high), point| (low.min(*point), high.max(*point)));
    let near: Vec<&query::Solid> = solids
        .iter()
        .filter(|solid| {
            let center = frame::translation(&solid.frame);
            let radius = solid.half_extents().length();
            center.clamp(low, high).distance(center) <= radius * 1.0001 + 1e-4
        })
        .collect();
    let mut hits: Vec<(f32, InstanceId)> = Vec::new();
    for point in targets {
        let direction = point - origin;
        hits.clear();
        hits.extend(near.iter().filter_map(|solid| query::ray_solid(solid, origin, direction, 0.0).filter(|(t, _)| *t < 1.0).map(|(t, _)| (t, solid.part))));
        query::charge_query(engine, crate::cost::OBSCURING_POINT);
        query::charge_crossed(engine, hits.len(), crate::cost::RAY_CROSSED, 0);
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        for &(_, part) in &hits {
            if decided.insert(part) && (ignored.is_empty() || !is_ignored(engine, &ignored, part)) {
                found.push(part);
            }
        }
    }
    query::charge_query(engine, crate::cost::OBSCURING_CALL + found.len() as f64 * crate::cost::OBSCURING_LISTED);
    let table = lua.create_table()?;
    for part in found {
        table.raw_push(instance_value(lua, engine, part)?)?;
    }
    Ok(values([LuaValue::Table(table)]))
}

/// The methods in this module's notes after the projections, and the hooks that keep the rolls: CameraType leaving Scriptable drops a
/// camera's, a freed camera's goes.
fn install_methods(engine: &Rc<Engine>, add: &mut impl FnMut(&str, super::MethodFn)) {
    add("GetRoll", method(|_, engine, id, _| number(roll(engine, id))));
    // (outside Scriptable it warns and changes nothing: measured on a Fixed camera and the CurrentCamera, Custom)
    add("SetRoll", method(|lua, engine, id, args| {
        let roll = float_argument(lua, engine, args, 1)?;
        if scriptable(engine, id) {
            engine.state.borrow_mut().cameras.rolls.insert(id, roll);
        } else {
            engine.warn("SetRoll can only be used on Camera objects with a CameraType of Scriptable");
        }
        Ok(LuaMultiValue::new())
    }));
    add("GetRenderCFrame", method(|lua, engine, id, _| Ok(values([convert::to_lua(lua, engine, &frame::cframe_value(&render_cframe(engine, id)))?]))));
    // (documented as broken: 0 on every camera measured)
    add("GetPanSpeed", method(|_, _, _, _| number(0.0)));
    add("GetTiltSpeed", method(|_, _, _, _| number(0.0)));
    // (no effect on a desktop client; the argument is checked as a CameraPanMode, nil taking the default, an item of another
    // enum named by its enum: "Unable to cast CameraType to CameraPanMode")
    add("SetCameraPanMode", method(|lua, engine, _, args| {
        let value = args.get(1);
        if value.is_nil() || engine.reflection.enum_id("CameraPanMode").and_then(|id| convert::coerce_enum(lua, engine, id, value)).is_some() {
            return Ok(LuaMultiValue::new());
        }
        raise(format!("Unable to cast {} to CameraPanMode", cast_name(engine, value)))
    }));
    add("GetPartsObscuringTarget", method(parts_obscuring));
    let mut hooks = engine.hooks.borrow_mut();
    if let (Some(camera_type), Some(scriptable)) = (engine.reflection.name_id("CameraType"), engine.reflection.enum_item("CameraType", "Scriptable")) {
        hooks.property_changed.push(Box::new(move |state, id, name| {
            if name == camera_type && state.dom.raw_get(id, camera_type).and_then(Value::as_enum) != Some(scriptable) {
                state.cameras.rolls.remove(&id);
                // (an Interpolate tween is cancelled, InterpolationFinished never fires: measured, a Fixed write mid-tween)
                state.cameras.tweens.remove(&id);
            }
        }));
    }
    hooks.instance_freed.push(Box::new(|state, id| {
        state.cameras.rolls.remove(&id);
        state.cameras.radians.remove(&id);
        state.cameras.tweens.remove(&id);
    }));
}

pub fn install(engine: &Rc<Engine>) {
    let Some(camera) = engine.reflection.class_id("Camera") else { return };
    let mut behaviors = engine.behaviors.borrow_mut();
    let mut add = |member: &str, f: super::MethodFn| {
        if let Some(name) = engine.reflection.name_id(member) {
            behaviors.add_method(camera, name, f);
        }
    };
    add("WorldToViewportPoint", method(|_, engine, id, args| {
        let (at, visible) = view(engine, id).project(vector3_argument(engine, args, 1)?);
        Ok(point(at, visible)?)
    }));
    add("WorldToScreenPoint", method(|_, engine, id, args| {
        let (at, visible) = view(engine, id).project(vector3_argument(engine, args, 1)?);
        let at = match screen_inset(engine, id) {
            Some(inset) => at - Vec3::new(0.0, inset, 0.0),
            None => at,
        };
        Ok(point(at, visible)?)
    }));
    add("ViewportPointToRay", method(|lua, engine, id, args| {
        let (x, y, depth) = ray_arguments(lua, engine, args)?;
        Ok(ray(lua, view(engine, id).ray(x, y, depth))?)
    }));
    add("ScreenPointToRay", method(|lua, engine, id, args| {
        let (x, y, depth) = ray_arguments(lua, engine, args)?;
        let y = screen_inset(engine, id).map_or(y, |inset| y + inset);
        Ok(ray(lua, view(engine, id).ray(x, y, depth))?)
    }));
    install_methods(engine, &mut add);
    super::camera_fov::install(engine, &mut behaviors, camera);
    super::camera_moves::install(engine, &mut behaviors, camera);
    if let Some(name) = engine.reflection.name_id("NearPlaneZ") {
        behaviors.add_getter(camera, name, Rc::new(|_, engine, id| Ok(LuaValue::Number(-f64::from(near_plane(engine, id))))));
    }
    // FieldOfView stays within 1 to 120 degrees, and a NaN written leaves it as it was (measured on a new camera and the
    // CurrentCamera: 0.5, -0, 1e-40 and -inf read 1, 120.5 and inf 120, a string that is no number 1 as it reads 0; a tween
    // past 120 ends at 120, and writing 200 at 120 fires no Changed)
    if let Some(name) = engine.reflection.name_id("FieldOfView")
        && let Some(info) = engine.reflection.class_property(camera, name).cloned()
    {
        behaviors.add_setter(
            camera,
            name,
            Rc::new(move |lua, engine, id, value| {
                if let Value::Number(degrees) = convert::coerce(lua, engine, &info, &value)?
                    && !degrees.is_nan()
                {
                    crate::lua::instance::set_property(engine, id, name, Value::Number(degrees.clamp(1.0, 120.0)));
                }
                Ok(())
            }),
        );
    }
    drop(behaviors);
    super::camera_moves::install_aliases(engine);
}
