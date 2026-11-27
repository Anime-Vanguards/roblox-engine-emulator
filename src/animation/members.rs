//! The animation classes' smaller members, as a 0.747 playing client and server answer them (measured in roblox-sim's client
//! and its server, each probe twice: tools/parity/probes/animation_lists.luau, animation_members.luau; docs/engine.md
//! "Animation members"):
//!
//! - KeyframeSequence AddKeyframe / GetKeyframes / RemoveKeyframe, Keyframe AddPose / GetPoses / RemovePose / AddMarker /
//!   GetMarkers / RemoveMarker and Pose AddSubPose / GetSubPoses / RemoveSubPose are Parent writes and child lists: Add sets
//!   the instance's Parent to self whatever its class (a Part too), with a Parent write's errors and events; Remove sets it to
//!   nil when self is its parent (any child, of any class) and does nothing otherwise; Get lists the children of the one class
//!   (Keyframe, Pose, KeyframeMarker; a NumberPose is no Pose) in child order, a new table each call, Keyframe Time not
//!   looked at. The argument is an Object: nil does nothing, no argument raises "Argument 1 missing or nil", any other value
//!   "Unable to cast value to Object".
//! - AnimationNodeDefinition's input pins: a list of names. AddInputPin appends (a name twice too; the name is a string
//!   parameter), RemoveInputPin takes out the first of the name, SetOrderedInputPinNames replaces the list with an array of
//!   strings and does nothing when an item is no string; each call that gets that far fires InputPinsChanged, a removal of a
//!   name not in the list and a list set to itself too.
//! - AnimationTrack's parameters: SetParameter(key, value) keeps a number (NaN as 0), string, boolean, Vector3, Vector2, CFrame,
//!   Color3, UDim2, NumberRange, BrickColor or EnumItem under the key's text (a value of another kind changes nothing),
//!   GetParameter reads it back (nil when none), GetParameterDefaults is an empty table;
//!   GetTargetNames / GetTargetInstance / SetTargetInstance raise "AnimationTrack:<name> is not available yet!".
//! - HumanoidRigDescription's joint names (Roblox's lists) and GetJointFromName (the joint property of that name, any case);
//!   DigitsRigDescription's finger controls, five Vector3s kept clamped (X to 0..1, Y and Z to -1..1), and finger tips, which
//!   read zero whatever was set.

use std::{cell::RefCell, rc::Rc};

use mlua::prelude::*;
use rustc_hash::FxHashMap;

use crate::{
    ClassId, Engine, InstanceId, NameId,
    instance as core,
    lua::{Args, Flow, instance_value, none, raise, values},
    services::{Behaviors, FIRST, StringParam, method, strings},
    value::Value,
};

/// An Object argument as a client casts it (lua/instance.rs object_arg): an instance, none for nil, no argument at all
/// "Argument 1 missing or nil", any other value "Unable to cast value to Object".
fn object(engine: &Engine, args: &Args) -> Result<Option<InstanceId>, Flow> {
    crate::lua::instance::object_arg(engine, args, 1)
}

/// `child.Parent = parent` as a script writes it: the Parent write's errors, events and price (the scripts it starts with them:
/// scripts.rs).
fn reparent(lua: &Lua, engine: &Rc<Engine>, child: InstanceId, parent: Option<InstanceId>) -> Result<(), Flow> {
    let _ = lua;
    crate::lua::instance::set_parent(engine, child, parent)?;
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

/// Per-instance state these members keep beside the instance (freed with it).
#[derive(Default)]
struct Kept {
    /// AnimationNodeDefinition input pins
    pins: FxHashMap<InstanceId, Vec<Vec<u8>>>,
    /// AnimationTrack parameters, by key, in the order first set
    parameters: FxHashMap<InstanceId, Vec<(Vec<u8>, LuaValue)>>,
    /// DigitsRigDescription finger controls (fingers 1 to 5)
    fingers: FxHashMap<InstanceId, [glam::Vec3; 5]>,
}

const ONE_STRING: &[StringParam] = &[StringParam::at(1)];

fn install_pins(behaviors: &mut Behaviors, engine: &Rc<Engine>, kept: &Rc<RefCell<Kept>>) {
    let reflection = engine.reflection.clone();
    let (Some(class), Some(changed)) = (reflection.class_id("AnimationNodeDefinition"), reflection.name_id("InputPinsChanged")) else { return };
    let fire = move |engine: &Engine, id: InstanceId| core::fire_event(&mut engine.state.borrow_mut(), id, changed, Vec::new());
    let text = |args: &Args| args.string(1).map(|text| text.as_bytes().to_vec()).unwrap_or_default();
    if let Some(name) = reflection.name_id("AddInputPin") {
        let kept = kept.clone();
        behaviors.add_method(class, name, strings(ONE_STRING, method(move |_, engine, id, args| {
            kept.borrow_mut().pins.entry(id).or_default().push(text(args));
            fire(engine, id);
            Ok(none())
        })));
    }
    if let Some(name) = reflection.name_id("RemoveInputPin") {
        let kept = kept.clone();
        behaviors.add_method(class, name, strings(ONE_STRING, method(move |_, engine, id, args| {
            let wanted = text(args);
            if let Some(pins) = kept.borrow_mut().pins.get_mut(&id)
                && let Some(at) = pins.iter().position(|pin| *pin == wanted)
            {
                pins.remove(at);
            }
            fire(engine, id);
            Ok(none())
        })));
    }
    if let Some(name) = reflection.name_id("GetOrderedInputPinNames") {
        let kept = kept.clone();
        behaviors.add_method(class, name, method(move |lua, _, id, _| {
            let table = lua.create_table()?;
            if let Some(pins) = kept.borrow().pins.get(&id) {
                for pin in pins {
                    table.raw_push(lua.create_string(pin)?)?;
                }
            }
            Ok(values([LuaValue::Table(table)]))
        }));
    }
    if let Some(name) = reflection.name_id("SetOrderedInputPinNames") {
        let kept = kept.clone();
        behaviors.add_method(class, name, method(move |_, engine, id, args| {
            let table = match args.get(1) {
                LuaValue::Nil => return raise("Argument 1 missing or nil"),
                LuaValue::Table(table) if table.raw_len() > 0 || table.is_empty() => table,
                other => return raise(format!("Unable to cast {} to Array", crate::services::tween::cast_name(engine, other))),
            };
            let mut pins = Vec::with_capacity(table.raw_len());
            for value in table.sequence_values::<LuaValue>() {
                match value? {
                    LuaValue::String(text) => pins.push(text.as_bytes().to_vec()),
                    // (measured: an item that is no string leaves the list as it was, with no event)
                    _ => return Ok(none()),
                }
            }
            kept.borrow_mut().pins.insert(id, pins);
            fire(engine, id);
            Ok(none())
        }));
    }
}

/// Whether a track keeps a parameter value of this kind (measured: numbers, strings, booleans, Vector3, Vector2, CFrame, Color3,
/// UDim2, NumberRange, BrickColor and EnumItems; a table, an instance, a function, a buffer or a Vector3int16 leaves the
/// parameter as it was)
fn kept_kind(engine: &Engine, value: &LuaValue) -> bool {
    matches!(value, LuaValue::Number(_) | LuaValue::Integer(_) | LuaValue::String(_) | LuaValue::Boolean(_) | LuaValue::Vector(_))
        || matches!(crate::lua::typeof_value(engine, value).as_str(), "Vector2" | "CFrame" | "Color3" | "UDim2" | "NumberRange" | "BrickColor" | "EnumItem")
}

fn install_track_members(behaviors: &mut Behaviors, engine: &Rc<Engine>, kept: &Rc<RefCell<Kept>>) {
    let reflection = engine.reflection.clone();
    let Some(track) = reflection.class_id("AnimationTrack") else { return };
    if let Some(name) = reflection.name_id("SetParameter") {
        let kept = kept.clone();
        behaviors.add_method(track, name, method(move |lua, engine, id, args| {
            // (the value is cast first, as a client casts a call's arguments from the last)
            if args.get(2).is_nil() {
                return raise("Argument 2 missing or nil");
            }
            let key = match args.get(1) {
                LuaValue::Nil => return raise("Argument 1 missing or nil"),
                other => crate::lua::convert::string_param(lua, engine, other, 1, true)?,
            };
            let value = args.get(2);
            if !kept_kind(engine, value) {
                return Ok(none());
            }
            // (measured: NaN is kept as 0)
            let value = match value {
                LuaValue::Number(number) if number.is_nan() => LuaValue::Number(0.0),
                other => other.clone(),
            };
            let mut kept = kept.borrow_mut();
            let parameters = kept.parameters.entry(id).or_default();
            match parameters.iter_mut().find(|(name, _)| *name == key) {
                Some(slot) => slot.1 = value,
                None => parameters.push((key, value)),
            }
            Ok(none())
        }));
    }
    if let Some(name) = reflection.name_id("GetParameter") {
        let kept = kept.clone();
        behaviors.add_method(track, name, strings(FIRST, method(move |_, _, id, args| {
            let key = args.string(1).map(|text| text.as_bytes().to_vec()).unwrap_or_default();
            let found = kept.borrow().parameters.get(&id).and_then(|parameters| parameters.iter().find(|(name, _)| *name == key).map(|(_, value)| value.clone()));
            Ok(values([found.unwrap_or(LuaValue::Nil)]))
        })));
    }
    if let Some(name) = reflection.name_id("GetParameterDefaults") {
        behaviors.add_method(track, name, method(|lua, _, _, _| Ok(values([LuaValue::Table(lua.create_table()?)]))));
    }
    for member in ["GetTargetNames", "GetTargetInstance", "SetTargetInstance"] {
        if let Some(name) = reflection.name_id(member) {
            let message = format!("AnimationTrack:{member} is not available yet!");
            behaviors.add_method(track, name, method(move |_, _, _, _| raise(message.clone())));
        }
    }
}

/// HumanoidRigDescription:GetJointNames (its joint properties, in this order), GetR15JointNames and GetR6JointNames: what a
/// 0.747 client answers.
const JOINT_NAMES: [&str; 22] = [
    "Root", "Waist", "Spine", "Chest", "Neck", "HeadBase", "LeftClavicle", "LeftShoulder", "LeftElbow", "LeftWrist", "RightClavicle",
    "RightShoulder", "RightElbow", "RightWrist", "LeftHip", "LeftKnee", "LeftAnkle", "LeftToeBase", "RightHip", "RightKnee", "RightAnkle",
    "RightToeBase",
];
const R15_JOINT_NAMES: [&str; 15] = [
    "Root", "Waist", "Neck", "LeftShoulder", "LeftElbow", "LeftWrist", "RightShoulder", "RightElbow", "RightWrist", "LeftHip", "LeftKnee",
    "LeftAnkle", "RightHip", "RightKnee", "RightAnkle",
];
const R6_JOINT_NAMES: [&str; 6] = ["Root", "Neck", "LeftShoulder", "RightShoulder", "LeftHip", "RightHip"];

/// An int argument as a client casts it (curves.rs index_arg: a number rounds to the nearest whole one, a string of one reads as
/// it, a boolean is 0 or 1; nil "Argument n missing or nil", other values "Unable to cast <value> to int").
fn int_arg(lua: &Lua, engine: &Engine, args: &Args, index: usize) -> Result<i64, Flow> {
    let number = match args.get(index) {
        LuaValue::Nil => return raise(format!("Argument {index} missing or nil")),
        LuaValue::Number(number) => *number,
        LuaValue::Integer(number) => *number as f64,
        LuaValue::Boolean(flag) => *flag as u8 as f64,
        value @ LuaValue::String(_) => match lua.coerce_number(value.clone()).ok().flatten() {
            Some(number) => number,
            None => return raise("Unable to cast string to int"),
        },
        other => return raise(format!("Unable to cast {} to int", crate::services::tween::cast_name(engine, other))),
    };
    Ok(if number.is_finite() { number.round() as i64 } else { 0 })
}

fn vector3_arg(engine: &Engine, args: &Args, index: usize) -> Result<glam::Vec3, Flow> {
    match args.get(index) {
        LuaValue::Nil => raise(format!("Argument {index} missing or nil")),
        LuaValue::Vector(vector) => Ok(glam::Vec3::new(vector.x(), vector.y(), vector.z())),
        other => raise(format!("Unable to cast {} to Vector3", crate::services::tween::cast_name(engine, other))),
    }
}

fn vector3_value(vector: glam::Vec3) -> LuaValue {
    LuaValue::Vector(mlua::Vector::new(vector.x, vector.y, vector.z))
}

fn install_rig_descriptions(behaviors: &mut Behaviors, engine: &Rc<Engine>, kept: &Rc<RefCell<Kept>>) {
    let reflection = engine.reflection.clone();
    if let Some(class) = reflection.class_id("HumanoidRigDescription") {
        let list = |names: &'static [&'static str]| method(move |lua, _, _, _| {
            let table = lua.create_table_with_capacity(names.len(), 0)?;
            for name in names {
                table.raw_push(*name)?;
            }
            Ok(values([LuaValue::Table(table)]))
        });
        for (member, names) in [("GetJointNames", &JOINT_NAMES[..]), ("GetR15JointNames", &R15_JOINT_NAMES[..]), ("GetR6JointNames", &R6_JOINT_NAMES[..])] {
            if let Some(name) = reflection.name_id(member) {
                behaviors.add_method(class, name, list(names));
            }
        }
        // (measured: the joint property of that name in any case, "root" too; a name no joint has, nil)
        let joints: Vec<(&'static str, Option<NameId>)> = JOINT_NAMES.iter().map(|&name| (name, reflection.name_id(name))).collect();
        if let Some(name) = reflection.name_id("GetJointFromName") {
            behaviors.add_method(class, name, strings(FIRST, method(move |lua, engine, id, args| {
                let wanted = args.string(1).map(|text| text.to_string_lossy()).unwrap_or_default();
                let Some(property) = joints.iter().find(|(name, _)| name.eq_ignore_ascii_case(&wanted)).and_then(|(_, property)| *property) else {
                    return Ok(values([LuaValue::Nil]));
                };
                let held = engine.state.borrow().dom.raw_get(id, property).cloned();
                match held {
                    Some(Value::Instance(joint)) if engine.state.borrow().dom.get(joint).is_some() => Ok(values([instance_value(lua, engine, joint)?])),
                    _ => Ok(values([LuaValue::Nil])),
                }
            })));
        }
    }
    if let Some(class) = reflection.class_id("DigitsRigDescription") {
        // (measured: fingers 1 to 5; another index reads zero and a write to it does nothing)
        let finger = |index: i64| usize::try_from(index - 1).ok().filter(|&index| index < 5);
        if let Some(name) = reflection.name_id("GetFingerControl") {
            let kept = kept.clone();
            behaviors.add_method(class, name, method(move |lua, engine, id, args| {
                let index = int_arg(lua, engine, args, 1)?;
                let found = finger(index).and_then(|at| kept.borrow().fingers.get(&id).map(|fingers| fingers[at])).unwrap_or(glam::Vec3::ZERO);
                Ok(values([vector3_value(found)]))
            }));
        }
        if let Some(name) = reflection.name_id("SetFingerControl") {
            let kept = kept.clone();
            behaviors.add_method(class, name, method(move |lua, engine, id, args| {
                // (cast from the last argument to the first)
                let control = vector3_arg(engine, args, 2)?;
                let index = int_arg(lua, engine, args, 1)?;
                if let Some(at) = finger(index) {
                    let clamped = glam::Vec3::new(control.x.clamp(0.0, 1.0), control.y.clamp(-1.0, 1.0), control.z.clamp(-1.0, 1.0));
                    kept.borrow_mut().fingers.entry(id).or_insert([glam::Vec3::ZERO; 5])[at] = clamped;
                }
                Ok(none())
            }));
        }
        if let Some(name) = reflection.name_id("GetFingerTip") {
            behaviors.add_method(class, name, method(|lua, engine, _, args| {
                int_arg(lua, engine, args, 1)?;
                Ok(values([vector3_value(glam::Vec3::ZERO)]))
            }));
        }
        if let Some(name) = reflection.name_id("SetFingerTip") {
            behaviors.add_method(class, name, method(|lua, engine, _, args| {
                vector3_arg(engine, args, 2)?;
                int_arg(lua, engine, args, 1)?;
                Ok(none())
            }));
        }
    }
}

/// IKControl's chain as a client reads it before its solver has moved anything (measured: chains of Motor6Ds and of Bones,
/// with turned parts, a weld and a joint's C1): the nodes from the chain's top to the EndEffector (an Attachment's part stands
/// for it). Over Motor6Ds the walk goes from the EndEffector up through the Motor6D whose Part1 each part is, to its Part0,
/// while that Part0 is itself some Motor6D's Part1, and stops at the ChainRoot (an unrelated ChainRoot leaves the walk to the
/// top); a node's world frame is its joint's frame on the Part0 side (Part0 * C0), the frame of a node without such a joint
/// its part's, each part where its joints place it from the rig's top. Over Bones the walk follows the parents, a part above
/// the bones counting only as the ChainRoot itself. A node's local frame is its world frame relative to the node above it,
/// in the chain or not (the topmost node's: its world frame). No ChainRoot or no EndEffector, no nodes. Welds and other joints
/// place parts but are no nodes.
struct Node {
    world: glam::Affine3A,
    local: glam::Affine3A,
}

struct IkNames {
    chain_root: NameId,
    end_effector: NameId,
    part0: NameId,
    part1: NameId,
    c0: NameId,
    c1: NameId,
    transform: NameId,
    cframe: NameId,
    motor: ClassId,
    joint: ClassId,
}

/// The joint whose Part1 a part is: its Part0, C0, C1 (with a Motor6D's Transform before it) and whether it is a Motor6D.
#[derive(Clone, Copy)]
struct Held {
    part0: InstanceId,
    c0: glam::Affine3A,
    c1: glam::Affine3A,
    motor: bool,
}

fn ik_chain(engine: &Engine, id: InstanceId, names: &IkNames) -> Vec<Node> {
    let state = engine.state.borrow();
    let reflection = &engine.reflection;
    let reference = |property: NameId| match state.dom.raw_get(id, property) {
        Some(Value::Instance(target)) if state.dom.get(*target).is_some() => Some(*target),
        _ => None,
    };
    let (Some(root), Some(mut effector)) = (reference(names.chain_root), reference(names.end_effector)) else { return Vec::new() };
    let class = |instance: InstanceId| state.dom.at(instance).class;
    let is_bone = |instance: InstanceId| reflection.is_a(class(instance), "Bone");
    if !is_bone(effector) && reflection.is_a(class(effector), "Attachment") {
        match state.dom.at(effector).parent {
            Some(part) if reflection.is_a(class(part), "BasePart") => effector = part,
            _ => return Vec::new(),
        }
    }
    let cframe = |instance: InstanceId, property: NameId| crate::world::frame::affine_of(state.dom.raw_get(instance, property));
    let mut nodes: Vec<Node> = Vec::new();
    if is_bone(effector) {
        let mut chain = vec![effector];
        let mut at = effector;
        while at != root {
            match state.dom.at(at).parent {
                Some(parent) if is_bone(parent) => {
                    chain.push(parent);
                    at = parent;
                }
                Some(parent) if parent == root => {
                    chain.push(parent);
                    break;
                }
                _ => break,
            }
        }
        chain.reverse();
        for (index, &node) in chain.iter().enumerate() {
            let world = crate::world::world_cframe(&state, engine, node);
            let local = match index {
                0 if is_bone(node) => cframe(node, names.cframe),
                0 => world,
                _ => nodes[index - 1].world.inverse() * world,
            };
            nodes.push(Node { world, local });
        }
        return nodes;
    }
    if !reflection.is_a(class(effector), "BasePart") {
        return Vec::new();
    }
    // (the joints of the rig the EndEffector is in, its nearest Model, by their Part1)
    let mut scope = state.dom.at(effector).parent;
    while let Some(at) = scope {
        if reflection.is_a(class(at), "Model") {
            break;
        }
        match state.dom.at(at).parent {
            Some(parent) => scope = Some(parent),
            None => break,
        }
    }
    let mut held: FxHashMap<InstanceId, Held> = FxHashMap::default();
    if let Some(scope) = scope {
        let mut stack: Vec<InstanceId> = state.dom.at(scope).children.iter().copied().collect();
        while let Some(at) = stack.pop() {
            stack.extend(state.dom.at(at).children.iter().copied());
            if !reflection.class(class(at)).is_a(names.joint) {
                continue;
            }
            let motor = class(at) == names.motor;
            if let (Some(Value::Instance(part0)), Some(Value::Instance(part1))) = (state.dom.raw_get(at, names.part0), state.dom.raw_get(at, names.part1)) {
                let transform = if motor { cframe(at, names.transform) } else { glam::Affine3A::IDENTITY };
                let joint = Held { part0: *part0, c0: cframe(at, names.c0), c1: cframe(at, names.c1) * transform.inverse(), motor };
                let entry = held.entry(*part1).or_insert(joint);
                if motor && !entry.motor {
                    *entry = joint;
                }
            }
        }
    }
    let motor_of = |part: InstanceId| held.get(&part).filter(|joint| joint.motor).copied();
    // (where a part's joints place it, from the rig's top down; a part held by none, its CFrame)
    let placed = |part: InstanceId| -> glam::Affine3A {
        let mut steps = Vec::new();
        let mut at = part;
        while let Some(joint) = held.get(&at) {
            if steps.len() > held.len() {
                break;
            }
            steps.push(*joint);
            at = joint.part0;
        }
        let mut world = crate::world::world_cframe(&state, engine, at);
        for joint in steps.iter().rev() {
            world = world * joint.c0 * joint.c1.inverse();
        }
        world
    };
    let node_world = |part: InstanceId| match motor_of(part) {
        Some(joint) => placed(joint.part0) * joint.c0,
        None => placed(part),
    };
    let mut chain = vec![effector];
    let mut at = effector;
    while at != root {
        let Some(joint) = motor_of(at) else { break };
        if motor_of(joint.part0).is_none() {
            break;
        }
        chain.push(joint.part0);
        at = joint.part0;
    }
    chain.reverse();
    for &node in &chain {
        let world = node_world(node);
        // (relative to the node above, which the chain may leave out)
        let above = motor_of(node).map(|joint| joint.part0).filter(|&part0| motor_of(part0).is_some());
        let local = match above {
            Some(above) => node_world(above).inverse() * world,
            None => world,
        };
        nodes.push(Node { world, local });
    }
    nodes
}

fn install_ik(behaviors: &mut Behaviors, engine: &Rc<Engine>) {
    let reflection = engine.reflection.clone();
    let name = |name: &str| reflection.name_id(name);
    let (Some(class), Some(chain_root), Some(end_effector), Some(part0), Some(part1), Some(c0), Some(c1), Some(transform), Some(cframe), Some(motor), Some(joint)) = (
        reflection.class_id("IKControl"), name("ChainRoot"), name("EndEffector"), name("Part0"), name("Part1"), name("C0"), name("C1"), name("Transform"),
        name("CFrame"), reflection.class_id("Motor6D"), reflection.class_id("JointInstance"),
    ) else {
        return;
    };
    let names = Rc::new(IkNames { chain_root, end_effector, part0, part1, c0, c1, transform, cframe, motor, joint });
    let frame = |lua: &Lua, engine: &Engine, transform: &glam::Affine3A| crate::lua::convert::to_lua(lua, engine, &crate::world::frame::cframe_value(transform));
    if let Some(member) = name("GetChainCount") {
        let names = names.clone();
        behaviors.add_method(class, member, method(move |_, engine, id, _| {
            Ok(values([LuaValue::Number(ik_chain(engine, id, &names).len().saturating_sub(1) as f64)]))
        }));
    }
    if let Some(member) = name("GetChainLength") {
        let names = names.clone();
        behaviors.add_method(class, member, method(move |_, engine, id, _| {
            let nodes = ik_chain(engine, id, &names);
            let length = nodes.windows(2).fold(0.0f32, |sum, pair| sum + (pair[1].world.translation - pair[0].world.translation).length());
            Ok(values([LuaValue::Number(length as f64)]))
        }));
    }
    for (member, local) in [("GetNodeLocalCFrame", true), ("GetNodeWorldCFrame", false)] {
        if let Some(member) = name(member) {
            let names = names.clone();
            behaviors.add_method(class, member, method(move |lua, engine, id, args| {
                // (measured: nodes count from 1; another index reads the identity)
                let index = int_arg(lua, engine, args, 1)?;
                let nodes = ik_chain(engine, id, &names);
                let found = usize::try_from(index - 1).ok().and_then(|at| nodes.get(at)).map_or(glam::Affine3A::IDENTITY, |node| if local { node.local } else { node.world });
                Ok(values([frame(lua, engine, &found)?]))
            }));
        }
    }
    // (the solver's targets: the identity before it has run; the solver is not emulated)
    for member in ["GetRawFinalTarget", "GetSmoothedFinalTarget"] {
        if let Some(member) = name(member) {
            behaviors.add_method(class, member, method(move |lua, engine, _, _| Ok(values([frame(lua, engine, &glam::Affine3A::IDENTITY)?]))));
        }
    }
}

/// Animator:ApplyJointVelocities(motors) and RegisterEvaluationParallelCallback(callback): their checks, measured on a 0.747
/// playing client. The motors must be an array ("motors must be an array." for any other value, a table with holes or keys
/// too) of Motor6Ds ("Value at index 2 was not an instance.", "Value at index 1 was not a Motor6D."); then nothing happens that a
/// script sees (it sets the joints' velocities for the physics, not emulated). The callback is cast to a function ("Unable to
/// cast string to Function"), then "RegisterEvaluationParallelCallback is not enabled!".
fn install_animator_members(behaviors: &mut Behaviors, engine: &Rc<Engine>) {
    let reflection = engine.reflection.clone();
    let (Some(animator), Some(motor)) = (reflection.class_id("Animator"), reflection.class_id("Motor6D")) else { return };
    if let Some(name) = reflection.name_id("ApplyJointVelocities") {
        behaviors.add_method(animator, name, method(move |_, engine, _, args| {
            let table = match args.get(1) {
                LuaValue::Nil => return raise("Argument 1 missing or nil"),
                LuaValue::Table(table) => table,
                _ => return raise("motors must be an array."),
            };
            let length = table.raw_len();
            if table.clone().pairs::<LuaValue, LuaValue>().count() != length {
                return raise("motors must be an array.");
            }
            for index in 1..=length {
                let value: LuaValue = table.raw_get(index)?;
                match crate::lua::instance_id(&value) {
                    None => return raise(format!("Value at index {index} was not an instance.")),
                    Some(id) if engine.state.borrow().dom.at(id).class != motor => return raise(format!("Value at index {index} was not a Motor6D.")),
                    Some(_) => {}
                }
            }
            Ok(none())
        }));
    }
    if let Some(name) = reflection.name_id("RegisterEvaluationParallelCallback") {
        behaviors.add_method(animator, name, method(|_, engine, _, args| match args.get(1) {
            LuaValue::Nil => raise("Argument 1 missing or nil"),
            LuaValue::Function(_) => raise("RegisterEvaluationParallelCallback is not enabled!"),
            other => raise(format!("Unable to cast {} to Function", crate::services::tween::cast_name(engine, other))),
        }));
    }
}

pub(super) fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    install_lists(engine, behaviors);
    let kept: Rc<RefCell<Kept>> = Rc::default();
    let freed = kept.clone();
    engine.hooks.borrow_mut().instance_freed.push(Box::new(move |_, id| {
        let mut kept = freed.borrow_mut();
        kept.pins.remove(&id);
        kept.parameters.remove(&id);
        kept.fingers.remove(&id);
    }));
    install_pins(behaviors, engine, &kept);
    install_track_members(behaviors, engine, &kept);
    install_rig_descriptions(behaviors, engine, &kept);
    install_ik(behaviors, engine);
    install_animator_members(behaviors, engine);
}
