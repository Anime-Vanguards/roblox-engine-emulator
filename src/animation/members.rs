//! The animation classes' smaller members, as a 0.747 playing client and server answer them (measured in roblox-sim's client
//! and its server, each probe twice: tools/parity/probes/animation_lists.luau; docs/engine.md "Animation members"):
//!
//! - KeyframeSequence AddKeyframe / GetKeyframes / RemoveKeyframe, Keyframe AddPose / GetPoses / RemovePose / AddMarker /
//!   GetMarkers / RemoveMarker and Pose AddSubPose / GetSubPoses / RemoveSubPose are Parent writes and child lists: Add sets
//!   the instance's Parent to self whatever its class (a Part too), with a Parent write's errors and events; Remove sets it to
//!   nil when self is its parent (any child, of any class) and does nothing otherwise; Get lists the children of the one class
//!   (Keyframe, Pose, KeyframeMarker; a NumberPose is no Pose) in child order, a new table each call, Keyframe Time not
//!   looked at. The argument is an Object: nil does nothing, no argument raises "Argument 1 missing or nil", any other value
//!   "Unable to cast value to Object".

use std::rc::Rc;

use mlua::prelude::*;

use crate::{
    ClassId, Engine, InstanceId,
    lua::{Args, Flow, instance_value, none, values},
    services::{Behaviors, method},
};

/// An Object argument as a client casts it (lua/instance.rs object_arg): an instance, none for nil, no argument at all
/// "Argument 1 missing or nil", any other value "Unable to cast value to Object".
fn object(engine: &Engine, args: &Args) -> Result<Option<InstanceId>, Flow> {
    crate::lua::instance::object_arg(engine, args, 1)
}

/// `child.Parent = parent` as a script writes it: the Parent write's errors, events and price, and the scripts under Actors in
/// it starting.
fn reparent(lua: &Lua, engine: &Rc<Engine>, child: InstanceId, parent: Option<InstanceId>) -> Result<(), Flow> {
    crate::lua::instance::set_parent(engine, child, parent)?;
    if parent.is_some() {
        crate::lua::actors::start_under(lua, engine, child)?;
    }
    Ok(())
}

fn children_of(lua: &Lua, engine: &Engine, id: InstanceId, class: ClassId) -> LuaResult<LuaMultiValue> {
    let found: Vec<InstanceId> = {
        let state = engine.state.borrow();
        state.dom.at(id).children.iter().copied().filter(|&child| state.dom.at(child).class == class).collect()
    };
    let table = lua.create_table_with_capacity(found.len(), 0)?;
    for child in found {
        table.raw_push(instance_value(lua, engine, child)?)?;
    }
    Ok(values([LuaValue::Table(table)]))
}

/// The child-list methods of KeyframeSequence, Keyframe and Pose.
fn install_lists(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let lists: [(&str, &str, &str, &str, &str); 4] = [
        ("KeyframeSequence", "Keyframe", "AddKeyframe", "GetKeyframes", "RemoveKeyframe"),
        ("Keyframe", "Pose", "AddPose", "GetPoses", "RemovePose"),
        ("Keyframe", "KeyframeMarker", "AddMarker", "GetMarkers", "RemoveMarker"),
        ("Pose", "Pose", "AddSubPose", "GetSubPoses", "RemoveSubPose"),
    ];
    for (owner, listed, add, get, remove) in lists {
        let (Some(owner), Some(listed)) = (reflection.class_id(owner), reflection.class_id(listed)) else { continue };
        let name = |name: &str| reflection.name_id(name);
        if let Some(add) = name(add) {
            behaviors.add_method(owner, add, method(|lua, engine, id, args| {
                if let Some(child) = object(engine, args)? {
                    reparent(lua, engine, child, Some(id))?;
                }
                Ok(none())
            }));
        }
        if let Some(get) = name(get) {
            behaviors.add_method(owner, get, method(move |lua, engine, id, _| Ok(children_of(lua, engine, id, listed)?)));
        }
        if let Some(remove) = name(remove) {
            behaviors.add_method(owner, remove, method(|lua, engine, id, args| {
                if let Some(child) = object(engine, args)? {
                    let parented = engine.state.borrow().dom.get(child).is_some_and(|instance| instance.parent == Some(id));
                    if parented {
                        reparent(lua, engine, child, None)?;
                    }
                }
                Ok(none())
            }));
        }
    }
}

pub(super) fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    install_lists(engine, behaviors);
}
