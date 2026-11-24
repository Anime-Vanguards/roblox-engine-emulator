//! The PlayerMouse `Player:GetMouse()` returns, and the host's record of the mouse.
//!
//! Measured in Studio 0.748, a playing client:
//! - It is an Instance of class PlayerMouse named "Instance", parentless, the same object on every call (also after Destroy).
//!   It is made on the first call ([`create`]): one made at start would take an instance serial, and every later instance's
//!   (GetDebugId, a host's node ids) would move by one.
//!   Its Parent is locked ("The Parent property of Instance is locked, current parent: NULL, new parent Workspace"), and Clone
//!   raises "Instance cannot be cloned" while it is Archivable (and gives nil once it is not).
//! - Being an Instance, it is no table: `next`, `rawget` and `#` raise on it, and members it lacks raise
//!   "NotAMember is not a valid member of PlayerMouse \"Instance\"".
//! - `UnitRay` is `CurrentCamera:ViewportPointToRay` at the mouse location, to the bit; `Origin` is `CFrame.new(o, o + d)` of that
//!   ray, to the bit; `Hit` is the same rotation at `o + d * distance` of the first part the ray meets within 10,000 studs, or at
//!   `o + d * 10000`, to the bit; `Target` (`target`) is that part or nil, `TargetSurface` the face it was hit on, `Right` when
//!   none was. Every part counts (CanQuery, CanCollide, CanTouch and Transparency do not matter), but TargetFilter and its
//!   descendants. They follow the camera within the frame.
//! - Reading Hit, Target or TargetSurface casts that ray: it costs the work clock what a client spends on it (cost.rs
//!   MOUSE_RAY; roblox-sim, Studio 0.747), sees only the parts the spatial index has taken in, and takes nothing into it
//!   ([`pointed`]).
//! - Reading Icon, UnitRay or Origin costs the work clock what a client spends on it beyond a plain read (cost.rs MOUSE_ICON: Icon
//!   4.49 us, UnitRay 0.06, Origin 0.09; roblox-sim, Studio 0.747).
//! - The computed members are read-only ("Unable to assign property Hit. Property is read only"); Icon and TargetFilter are not.
//!
//! The host's record (`shared.__rurMouse`, a plain table made at start: roblox-ui-renderer's input layer reads it) holds X, Y,
//! ViewSizeX and ViewSizeY, which the host writes as the mouse moves (X and Y are the mouse location less the GUI inset, as
//! measured), and the mouse's legacy events under their names (Button1Down to KeyUp), which become the PlayerMouse's own event
//! signals: a host fires them and the PlayerMouse's handlers run. Without a host the mouse stays at (-1000, -1000).

use std::rc::Rc;

use glam::{Affine3A, Mat3, Vec3};
use lune_roblox::datatypes::types as lune;
use lune_roblox::datatypes::types::float32 as f32m;
use mlua::prelude::*;
use rbx_dom_weak::types as dom;

use super::{camera, query};
use crate::{
    Engine, InstanceId, K, NameId,
    dom::flags,
    lua::{convert, enum_item_value, instance_value, signal_value, types::LSignal},
    services::GetterFn,
    value::{Str, Value},
};

/// How far `Hit` reaches when nothing is in the way (measured: `Hit.Position = UnitRay.Origin + UnitRay.Direction * 10000`)
const REACH: f32 = 10000.0;

/// The key of the host's record in the shared globals (roblox-ui-renderer sets and reads it: keep the name)
pub const RECORD: &str = "__rurMouse";

/// The mouse's legacy events, in the order the record always made their signals in.
const EVENTS: [&str; 10] = ["Button1Down", "Button1Up", "Button2Down", "Button2Up", "Move", "Idle", "WheelForward", "WheelBackward", "KeyDown", "KeyUp"];

/// The host's record of the mouse (made once at start, globals.rs): the mouse at (-1000, -1000) and a signal for each event,
/// named as the PlayerMouse's own events are (`Mouse.Button1Down`).
pub fn record(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<LuaTable> {
    let record = lua.create_table()?;
    record.raw_set("X", -1000)?;
    record.raw_set("Y", -1000)?;
    record.raw_set("ViewSizeX", 0)?;
    record.raw_set("ViewSizeY", 0)?;
    for name in EVENTS {
        let signal = engine.state.borrow_mut().signals.create_labelled(Str::from(name), Str::from(format!("Mouse.{name}").as_str()));
        record.raw_set(name, signal_value(lua, engine, signal)?)?;
    }
    Ok(record)
}

/// Player:GetMouse()'s PlayerMouse, made on the first call (services.luau keeps it), with the record's signals as its events.
pub fn create(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<LuaValue> {
    let reflection = &engine.reflection;
    let Some(class) = reflection.class_id("PlayerMouse") else { return Ok(LuaValue::Nil) };
    let mut events = Vec::new();
    let shared = engine.lua.borrow().shared.clone();
    if let Some(LuaValue::Table(record)) = shared.map(|shared| shared.raw_get::<LuaValue>(RECORD)).transpose()? {
        for name in EVENTS {
            if let (Some(id), LuaValue::UserData(signal)) = (reflection.name_id(name), record.raw_get::<LuaValue>(name)?)
                && let Ok(signal) = signal.borrow::<LSignal>()
            {
                events.push((id, signal.0));
            }
        }
    }
    let mouse = {
        let mut state = engine.state.borrow_mut();
        let mouse = crate::instance::create_named(&mut state, reflection, class, "Instance");
        if let Some(icon) = reflection.name_id("Icon") {
            crate::instance::raw_set_silent(&mut state, reflection, mouse, icon, Value::String(Str::from("")));
        }
        let instance = state.dom.at_mut(mouse);
        instance.set_flag(flags::PARENT_LOCKED, true);
        instance.signals.get_or_insert_with(Box::default).events.extend(events);
        mouse
    };
    instance_value(lua, engine, mouse)
}

/// A number of the host's record.
fn recorded(engine: &Engine, key: &str, default: f64) -> f64 {
    let shared = engine.lua.borrow().shared.clone();
    shared
        .and_then(|shared| shared.raw_get::<LuaValue>(RECORD).ok())
        .and_then(|record| match record {
            LuaValue::Table(table) => table.raw_get::<Option<f64>>(key).ok().flatten(),
            _ => None,
        })
        .unwrap_or(default)
}

fn current_camera(engine: &Engine) -> InstanceId {
    let state = engine.state.borrow();
    match state.dom.at(engine.world().workspace).stored(K::CurrentCamera as NameId) {
        Some(Value::Instance(id)) => *id,
        _ => engine.world().camera,
    }
}

/// UnitRay: the CurrentCamera's ViewportPointToRay at the mouse location (the record's X, and its Y plus the inset back).
fn unit_ray(engine: &Engine) -> (Vec3, Vec3) {
    let x = recorded(engine, "X", -1000.0) as f32;
    let y = (recorded(engine, "Y", -1000.0) + engine.options.topbar_inset) as f32;
    camera::viewport_ray(engine, current_camera(engine), x, y, 0.0)
}

/// Origin's frame: `CFrame.new(o, o + d)`, G3D's aim of `(o + d) - o` (lune's look_g3d), its float steps included.
fn aimed(origin: Vec3, direction: Vec3) -> Affine3A {
    let rows = f32m::look_g3d(((origin + direction) - origin).to_array());
    let column = |j: usize| Vec3::new(rows[0][j], rows[1][j], rows[2][j]);
    Affine3A::from_mat3_translation(Mat3::from_cols(column(0), column(1), column(2)), origin)
}

/// Where the ray first meets a part the mouse sees: the point, and the part with its face's normal in the part's frame. The
/// mouse sees every BasePart of the Workspace's own world but Terrain, TargetFilter and its descendants left out: not a
/// WorldModel's parts in it (measured: a wall in a WorldModel in the Workspace across the ray leaves Target nil, where the same
/// wall in the Workspace is Target; query.rs near_segment_own); CanQuery, CanCollide, CanTouch and Transparency do not matter
/// (measured: a wall with each turned off, or fully transparent, is Target).
///
/// The parts come from the spatial queries' broadphase (world/broadphase.rs: the ray tests the parts near it), TargetFilter's
/// among them: the ray meets those too, and a client pays for them. The read is charged as the ray it is: [`crate::cost::MOUSE_RAY`] and the member's `price`, then the parts
/// it meets up to its hit (the hit and those under TargetFilter before it) and the hit's own. It sees the parts the spatial index
/// has: one that entered the Workspace since a spatial query of the Workspace or the physics step last took parts in
/// (`Cameras::unindexed`, world/camera_moves.rs) is neither met nor paid for, and the read takes nothing in (measured: a
/// client's Target misses such a part until a Raycast or the next frame, and GetLargestCutoffDistance still misses it after any
/// member is read; tools/parity/probes/mouse_index.luau).
fn pointed(engine: &Engine, target_filter: Option<NameId>, mouse: InstanceId, origin: Vec3, direction: Vec3, price: f64) -> (Vec3, Option<(InstanceId, Vec3)>) {
    let skip = match target_filter.and_then(|name| engine.state.borrow().dom.raw_get(mouse, name).cloned()) {
        Some(Value::Instance(skip)) => Some(skip),
        _ => None,
    };
    query::refresh(engine);
    let root = engine.world().workspace;
    let reach = direction * REACH;
    let length = reach.length();
    // (the parts the ray meets, in the Workspace's order)
    let mut entered: Vec<(query::Solid, f32, Vec3)> = Vec::new();
    // (a ray that is not finite, from a camera whose CFrame is not, meets nothing and tests no part, as a Raycast's)
    if origin.is_finite() && reach.is_finite() && length.is_finite() {
        query::near_segment_own(engine, origin, reach, 0.0, |parts| {
            let state = engine.state.borrow();
            for (part, entry) in parts {
                let Some((t, local)) = query::ray_solid(&entry.solid, origin, reach, 0.0) else { continue };
                // (looked up only for the parts the ray meets, in a set emptied at every physics step; a waiting part comes here
                // only with every part, in the broadphase's scan mode: it keeps them out of its tree)
                if state.cameras.unindexed.contains(part) {
                    continue;
                }
                entered.push((entry.solid, t, local));
            }
        });
    }
    // (query::cast_ray's choice among the parts TargetFilter leaves, the first of equals; and how far along the ray each part
    // under TargetFilter is)
    let mut best: Option<(f32, &query::Solid, Vec3)> = None;
    // (how far along the ray each part under TargetFilter is, and each part's TargetFilter check: its distance and the steps up)
    let (mut filtered, mut walks) = (Vec::new(), Vec::new());
    {
        let state = engine.state.borrow();
        let steps = super::broadphase::sort_by_order(&state.dom, root, &mut entered, |(solid, ..)| solid.part);
        query::charge_order(&state, steps, entered.len());
        for (solid, t, local) in &entered {
            let distance = length * t;
            let under = skip.map(|skip| query::steps_up_to(&state, solid.part, skip));
            if let Some((_, steps)) = under {
                walks.push((distance, steps));
            }
            if under.is_some_and(|(found, _)| found) {
                filtered.push(distance);
            } else if best.is_none_or(|(nearest, ..)| distance < nearest) {
                best = Some((distance, solid, *local));
            }
        }
    }
    let nearer = |distance: f32| best.is_none_or(|(nearest, ..)| distance < nearest);
    let met = usize::from(best.is_some()) + filtered.iter().filter(|&&distance| nearer(distance)).count();
    query::charge_query(engine, crate::cost::MOUSE_RAY + price + if best.is_some() { crate::cost::MOUSE_HIT } else { 0.0 });
    query::charge_crossed(engine, met, crate::cost::MOUSE_FILTERED, 0);
    // (a client checks TargetFilter for each part its ray meets up to its hit: up to the TargetFilter from a part under it, to the
    // top from one that is not, at cost.rs MOUSE_FILTER_STEP a step)
    let reach = best.map(|(nearest, ..)| nearest);
    let steps: u32 = walks.iter().filter(|(distance, _)| reach.is_none_or(|hit| *distance <= hit)).map(|(_, steps)| steps).sum();
    query::charge_query(engine, f64::from(steps) * crate::cost::MOUSE_FILTER_STEP);
    match best {
        Some((distance, solid, local)) => {
            // (the face's normal as query::cast_ray gives it, in the world, then back in the part's frame)
            let normal = solid.frame.transform_vector3(local).normalize_or(Vec3::Y);
            (origin + direction * distance, Some((solid.part, solid.frame.inverse().transform_vector3(normal))))
        }
        None => (origin + direction * REACH, None),
    }
}

/// The NormalId of a face from its normal in the part's frame (the largest component).
fn surface(local: Vec3) -> &'static str {
    let a = local.abs();
    if a.x >= a.y && a.x >= a.z {
        if local.x >= 0.0 { "Right" } else { "Left" }
    } else if a.y >= a.z {
        if local.y >= 0.0 { "Top" } else { "Bottom" }
    } else if local.z >= 0.0 {
        "Back"
    } else {
        "Front"
    }
}

fn cframe_value(lua: &Lua, engine: &Engine, transform: &Affine3A) -> LuaResult<LuaValue> {
    convert::to_lua(lua, engine, &super::frame::cframe_value(transform))
}

/// The members read from the record and the ones computed from the camera, as getters of Mouse.
pub fn install(engine: &Rc<Engine>) {
    let Some(mouse_class) = engine.reflection.class_id("Mouse") else { return };
    let filter = engine.reflection.name_id("TargetFilter");
    let (right, normal_id) = (engine.reflection.enum_item("NormalId", "Right"), engine.reflection.enum_id("NormalId"));
    let mut members: Vec<(&str, GetterFn)> = Vec::new();
    for (key, default) in [("X", -1000.0), ("Y", -1000.0), ("ViewSizeX", 0.0), ("ViewSizeY", 0.0)] {
        members.push((key, Rc::new(move |_, engine, _| Ok(LuaValue::Number(recorded(engine, key, default))))));
    }
    members.push(("UnitRay", Rc::new(|lua, engine, _| {
        query::charge_query(engine, crate::cost::MOUSE_UNIT_RAY);
        let (origin, direction) = unit_ray(engine);
        let ray = lune::Ray::from(dom::Ray::new(dom::Vector3::new(origin.x, origin.y, origin.z), dom::Vector3::new(direction.x, direction.y, direction.z)));
        Ok(LuaValue::UserData(lua.create_userdata(ray)?))
    })));
    members.push(("Origin", Rc::new(|lua, engine, _| {
        query::charge_query(engine, crate::cost::MOUSE_ORIGIN);
        let (origin, direction) = unit_ray(engine);
        Ok(cframe_value(lua, engine, &aimed(origin, direction))?)
    })));
    // Icon is a plain property, read at what a client spends on it
    let icon = engine.reflection.name_id("Icon");
    members.push(("Icon", Rc::new(move |lua, engine, id| {
        query::charge_query(engine, crate::cost::MOUSE_ICON);
        let value = icon.and_then(|name| engine.state.borrow().dom.raw_get(id, name).cloned());
        Ok(match value {
            Some(value) => convert::to_lua(lua, engine, &value)?,
            None => LuaValue::Nil,
        })
    })));
    let hit: GetterFn = Rc::new(move |lua, engine, id| {
        let (origin, direction) = unit_ray(engine);
        let mut frame = aimed(origin, direction);
        frame.translation = pointed(engine, filter, id, origin, direction, crate::cost::MOUSE_FRAME).0.into();
        Ok(cframe_value(lua, engine, &frame)?)
    });
    members.push(("Hit", hit.clone()));
    members.push(("hit", hit));
    let target: GetterFn = Rc::new(move |lua, engine, id| {
        let (origin, direction) = unit_ray(engine);
        match pointed(engine, filter, id, origin, direction, 0.0).1 {
            Some((part, _)) => Ok(instance_value(lua, engine, part)?),
            None => Ok(LuaValue::Nil),
        }
    });
    members.push(("Target", target.clone()));
    members.push(("target", target));
    members.push(("TargetSurface", Rc::new(move |lua, engine, id| {
        let (origin, direction) = unit_ray(engine);
        let item = match pointed(engine, filter, id, origin, direction, 0.0).1 {
            Some((_, local)) => normal_id.and_then(|_| engine.reflection.enum_item("NormalId", surface(local))),
            None => right,
        };
        match item {
            Some(item) => Ok(enum_item_value(lua, engine, item)?),
            None => Ok(LuaValue::Nil),
        }
    })));
    let mut behaviors = engine.behaviors.borrow_mut();
    for (member, getter) in members {
        if let Some(name) = engine.reflection.name_id(member) {
            behaviors.add_getter(mouse_class, name, getter);
        }
    }
}
