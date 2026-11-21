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

/// The parts the mouse sees: every BasePart in the Workspace but Terrain, TargetFilter and its descendants left out. CanQuery,
/// CanCollide, CanTouch and Transparency do not matter (measured: a wall with each turned off, or fully transparent, is Target).
fn targets(engine: &Engine, skip: Option<InstanceId>) -> Vec<query::Solid> {
    let state = engine.state.borrow();
    let reflection = &engine.reflection;
    let (Some(base_part), Some(terrain)) = (reflection.class_id("BasePart"), reflection.class_id("Terrain")) else { return Vec::new() };
    let mut result = Vec::new();
    for id in state.dom.descendants(engine.world().workspace) {
        let info = reflection.class(state.dom.at(id).class);
        if !info.is_a(base_part) || info.is_a(terrain) || skip.is_some_and(|skip| id == skip || state.dom.is_descendant_of(id, skip)) {
            continue;
        }
        result.push(query::solid(&state, engine, id));
    }
    result
}

/// Where the ray first meets a part the mouse sees: the point, and the part with its face's normal in the part's frame.
fn pointed(engine: &Engine, target_filter: Option<NameId>, mouse: InstanceId, origin: Vec3, direction: Vec3) -> (Vec3, Option<(InstanceId, Vec3)>) {
    let skip = match target_filter.and_then(|name| engine.state.borrow().dom.raw_get(mouse, name).cloned()) {
        Some(Value::Instance(skip)) => Some(skip),
        _ => None,
    };
    let solids = targets(engine, skip);
    match query::cast_ray(&solids, origin, direction * REACH, 0.0, None) {
        Some(hit) => {
            let local = solids.iter().find(|solid| solid.part == hit.part).map_or(hit.normal, |solid| solid.frame.inverse().transform_vector3(hit.normal));
            (origin + direction * hit.distance, Some((hit.part, local)))
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
        let (origin, direction) = unit_ray(engine);
        let ray = lune::Ray::from(dom::Ray::new(dom::Vector3::new(origin.x, origin.y, origin.z), dom::Vector3::new(direction.x, direction.y, direction.z)));
        Ok(LuaValue::UserData(lua.create_userdata(ray)?))
    })));
    members.push(("Origin", Rc::new(|lua, engine, _| {
        let (origin, direction) = unit_ray(engine);
        Ok(cframe_value(lua, engine, &aimed(origin, direction))?)
    })));
    let hit: GetterFn = Rc::new(move |lua, engine, id| {
        let (origin, direction) = unit_ray(engine);
        let mut frame = aimed(origin, direction);
        frame.translation = pointed(engine, filter, id, origin, direction).0.into();
        Ok(cframe_value(lua, engine, &frame)?)
    });
    members.push(("Hit", hit.clone()));
    members.push(("hit", hit));
    let target: GetterFn = Rc::new(move |lua, engine, id| {
        let (origin, direction) = unit_ray(engine);
        match pointed(engine, filter, id, origin, direction).1 {
            Some((part, _)) => Ok(instance_value(lua, engine, part)?),
            None => Ok(LuaValue::Nil),
        }
    });
    members.push(("Target", target.clone()));
    members.push(("target", target));
    members.push(("TargetSurface", Rc::new(move |lua, engine, id| {
        let (origin, direction) = unit_ray(engine);
        let item = match pointed(engine, filter, id, origin, direction).1 {
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
