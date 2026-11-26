//! The part, model, attachment and motor methods that read or change assemblies and joints, as a Studio 0.747 playing client and its
//! server answer them (tools/parity/probes/part_methods.luau, part_joints.luau, part_walks.luau; client and server alike unless said):
//!
//! - BasePart:GetRootPart is AssemblyRootPart (nil outside every WorldRoot).
//! - A part's joints are one list a client keeps ([`joints::part_joints`]: the active rigid JointInstances and WeldConstraints, each
//!   going to the end of its parts' lists when it becomes active and leaving by the last one taking its place). GetJoints lists them
//!   ([`joints_of`]); GetConnectedParts() is the part and the part each of its joints joins it to, in that order, one for each joint
//!   (two welds to one part list it twice), and the part alone outside every world.
//! - GetConnectedParts(true) is every part a walk over those joints reaches (anchored ones too, past them), an empty list outside
//!   every world: in a component that holds an anchored part, the order a stack walk from the part finds them in ([`stack_walk`]); in
//!   one that holds none, its assembly's tree from the root, each part before its children, the child its newest joint holds first
//!   ([`assembly_tree`], [`tree_walk`]).
//! - JointInstance.Active: whether the joint is an edge of its assembly's tree ([`assembly_tree`]): a joint between two anchored
//!   parts, between two assemblies, or one a tree leaves out (a cycle's, a parallel joint's) is not.
//! - BreakJoints destroys a part's rigid joints in its list (the JointInstances, then the WeldConstraints), Model:BreakJoints each
//!   descendant part's; MakeJoints makes none (surfaces no longer join parts).
//!
//! The work clock charges what a client spends on each call (cost.rs method, CONNECTED_PART, JOINT_LISTED).

use std::{cell::RefCell, rc::Rc};

use glam::{Affine3A, Vec3};
use mlua::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{camera, frame, joints};
use crate::{
    Engine, InstanceId, NameId, Reflection, State,
    lua::{Args, Flow, Ret, instance_value, none, raise, values},
    services::{getter, method},
    value::Value,
};

#[cfg(test)]
#[path = "parts_tests.rs"]
mod tests;

fn charge(engine: &Engine, seconds: f64) {
    engine.state.borrow().sched.charge(seconds);
    crate::boundary::add_cost(seconds);
}

/// A new table of instances (each call its own: a client's lists are fresh, unfrozen tables).
fn list(lua: &Lua, engine: &Engine, ids: &[InstanceId]) -> Ret {
    let table = lua.create_table_with_capacity(ids.len(), 0)?;
    for &id in ids {
        table.raw_push(instance_value(lua, engine, id)?)?;
    }
    Ok(values([LuaValue::Table(table)]))
}

fn vector_value(lua: &Lua, engine: &Engine, vector: Vec3) -> Ret {
    Ok(values([crate::lua::convert::to_lua(lua, engine, &frame::vector_value(vector))?]))
}

/// A Vector3 argument (nil raises "Argument n missing or nil", another value "Unable to cast <value> to Vector3").
pub(super) fn vector_argument(engine: &Engine, args: &Args, at: usize) -> Result<Vec3, Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    match super::vector_arg(value) {
        Some(vector) => Ok(vector),
        None => raise(format!("Unable to cast {} to Vector3", camera::cast_name(engine, value))),
    }
}

/// GetConnectedParts' recursive flag: nil is false, a number is true unless 0, a string or any other value raises "Unable to cast
/// <value> to bool" (measured: "x" and "" alike, a table "Array", an EnumItem its enum's name).
fn bool_argument(engine: &Engine, args: &Args, at: usize) -> Result<bool, Flow> {
    match args.get(at) {
        LuaValue::Nil => Ok(false),
        LuaValue::Boolean(on) => Ok(*on),
        LuaValue::Number(number) => Ok(*number != 0.0),
        LuaValue::Integer(number) => Ok(*number != 0),
        LuaValue::String(_) => raise("Unable to cast string to bool"),
        other => raise(format!("Unable to cast {} to bool", camera::cast_name(engine, other))),
    }
}

/// An Instance argument: nil is None; a value that is no instance raises "Unable to cast value to Object".
fn object_argument(args: &Args, at: usize) -> Result<Option<InstanceId>, Flow> {
    match args.get(at) {
        LuaValue::Nil => Ok(None),
        _ => match args.instance(at) {
            Some(id) => Ok(Some(id)),
            None => raise("Unable to cast value to Object"),
        },
    }
}

/// GetConnectedParts(recursive) of `part`: the part and the parts its joints join it to, each joint's (none outside every world), or
/// with `recursive` every part a walk over the joints reaches ([`stack_walk`], [`tree_walk`]; an empty list outside every world).
fn connected(state: &mut State, reflection: &Reflection, part: InstanceId, recursive: bool) -> Vec<InstanceId> {
    if joints::world_root(state, reflection, part).is_none() {
        return if recursive { Vec::new() } else { vec![part] };
    }
    if !recursive {
        let mut found = vec![part];
        found.extend(joints::part_joints(state, reflection, part).into_iter().map(|entry| entry.other));
        return found;
    }
    // (the component: each part's list, as a walk from the part reaches them)
    let mut lists: FxHashMap<InstanceId, Vec<joints::JointEntry>> = FxHashMap::default();
    let mut order = vec![part];
    let mut index = 0;
    while index < order.len() {
        let at = order[index];
        let entries = joints::part_joints(state, reflection, at);
        for entry in &entries {
            if !lists.contains_key(&entry.other) && !order.contains(&entry.other) {
                order.push(entry.other);
            }
        }
        lists.insert(at, entries);
        index += 1;
    }
    if order.iter().any(|&each| joints::is_anchored(state, each)) {
        return stack_walk(&lists, part);
    }
    let root = joints::root_part(state, reflection, part).unwrap_or(part);
    let tree = assembly_tree(state, reflection, &lists, root);
    tree_walk(&tree, root)
}

/// The order GetConnectedParts(true) lists a component holding an anchored part in (measured on 29 walks over chains, trees, cycles
/// and bridges between anchored parts, from every part): a stack walk from the part, each part listed when first found, its joints'
/// parts found in its list's order and the last found looked at first.
fn stack_walk(lists: &FxHashMap<InstanceId, Vec<joints::JointEntry>>, start: InstanceId) -> Vec<InstanceId> {
    let mut found = vec![start];
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    seen.insert(start);
    let mut stack = vec![start];
    while let Some(at) = stack.pop() {
        for entry in lists.get(&at).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(entry.other) {
                found.push(entry.other);
                stack.push(entry.other);
            }
        }
    }
    found
}

/// An assembly's tree: each part's children, with the joint and stamp that hold each, and the joints it is made of.
struct Tree {
    children: FxHashMap<InstanceId, Vec<(u64, InstanceId)>>,
    edges: FxHashSet<InstanceId>,
}

/// The tree a client keeps for an assembly (`lists`: its parts' joint lists; `root` its root): its joints taken in turn, each joining
/// its two parts unless they are joined already (a union over the parts), in the order (measured on 50 free assemblies of three to
/// eight parts, cycles welded in every order, the root picked by size or RootPriority, other parts made bigger, and through
/// JointInstance.Active): the Weld kinds before WeldConstraint, Motor and Motor6D (as several anchored parts' union takes them:
/// joints.rs claims), then the joints touching the better ranked part first (the root's own ranking, joints.rs part_rank), then the
/// newer first. Each part then hangs from the part its tree joint holds it to.
fn assembly_tree(state: &State, reflection: &Reflection, lists: &FxHashMap<InstanceId, Vec<joints::JointEntry>>, root: InstanceId) -> Tree {
    let parts: Vec<InstanceId> = lists.keys().copied().collect();
    let place: FxHashMap<InstanceId, usize> = parts.iter().enumerate().map(|(index, &part)| (part, index)).collect();
    let ranks: Vec<(bool, bool, i64, f32)> = parts.iter().map(|&part| joints::part_rank(state, reflection, part)).collect();
    let better = |a: &(bool, bool, i64, f32), b: &(bool, bool, i64, f32)| {
        (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)).then(a.3.total_cmp(&b.3))
    };
    // (each joint once: from the part it lists first)
    let mut edges: Vec<(&joints::JointEntry, usize, usize, usize)> = Vec::new();
    let mut taken: FxHashSet<InstanceId> = FxHashSet::default();
    for (&part, entries) in lists {
        for entry in entries {
            let Some(&other) = place.get(&entry.other) else { continue };
            if taken.insert(entry.joint) {
                let here = place[&part];
                let best = if better(&ranks[here], &ranks[other]).is_ge() { here } else { other };
                edges.push((entry, here, other, best));
            }
        }
    }
    edges.sort_by(|a, b| a.0.rank.cmp(&b.0.rank).then_with(|| better(&ranks[b.3], &ranks[a.3])).then(b.0.stamp.cmp(&a.0.stamp)));
    let mut parent: Vec<usize> = (0..parts.len()).collect();
    fn find(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut adjacency: FxHashMap<InstanceId, Vec<(u64, InstanceId)>> = FxHashMap::default();
    let mut tree_edges: FxHashSet<InstanceId> = FxHashSet::default();
    for (entry, a, b, _) in edges {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra == rb {
            continue;
        }
        parent[ra] = rb;
        tree_edges.insert(entry.joint);
        adjacency.entry(parts[a]).or_default().push((entry.stamp, parts[b]));
        adjacency.entry(parts[b]).or_default().push((entry.stamp, parts[a]));
    }
    // (rooted: each part's children, the newest joint's first)
    let mut children: FxHashMap<InstanceId, Vec<(u64, InstanceId)>> = FxHashMap::default();
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    seen.insert(root);
    let mut queue = vec![root];
    while let Some(at) = queue.pop() {
        for &(stamp, next) in adjacency.get(&at).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(next) {
                children.entry(at).or_default().push((stamp, next));
                queue.push(next);
            }
        }
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| b.0.cmp(&a.0));
    }
    Tree { children, edges: tree_edges }
}

/// A tree's parts from its root, each before its children (the newest joint's child first).
fn tree_walk(tree: &Tree, root: InstanceId) -> Vec<InstanceId> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(at) = stack.pop() {
        found.push(at);
        for &(_, child) in tree.children.get(&at).map(Vec::as_slice).unwrap_or_default().iter().rev() {
            stack.push(child);
        }
    }
    found
}

/// JointInstance.Active: the joint is in its parts' lists and an edge of their assembly's tree ([`assembly_tree`]): false for a
/// joint between two anchored parts or two assemblies (several anchored parts' union gave its parts to different roots), and for one
/// the tree leaves out (Studio 0.747: of a ring of welds the oldest not touching the root, of parallel joints all but the one taken).
fn active(state: &mut State, reflection: &Reflection, joint: InstanceId) -> bool {
    let Some([first, second]) = joints::listed_pair(state, reflection, joint) else { return false };
    if joints::is_anchored(state, first) && joints::is_anchored(state, second) {
        return false;
    }
    let Some(root) = joints::root_part(state, reflection, first) else { return false };
    if joints::root_part(state, reflection, second) != Some(root) {
        return false;
    }
    let members = joints::assembly_parts(state, reflection, first);
    let inside: FxHashSet<InstanceId> = members.iter().copied().collect();
    let mut lists: FxHashMap<InstanceId, Vec<joints::JointEntry>> = FxHashMap::default();
    for &part in members.iter() {
        let entries = joints::part_joints(state, reflection, part).into_iter().filter(|entry| inside.contains(&entry.other)).collect();
        lists.insert(part, entries);
    }
    assembly_tree(state, reflection, &lists, root).edges.contains(&joint)
}

/// The constraints, Rotate, RotateP and RotateV joints active at `part` (GetJoints lists them beside its joints): (one-attachment
/// constraints, the others), each in the order they were made. A constraint is active while it is Enabled, in the part's WorldRoot,
/// with its attachments under parts of that world (two different ones), or its one attachment under the part for the classes that
/// take one (VectorForce, Torque, LinearVelocity, AngularVelocity, LineForce's Attachment0 alone counts too? not measured: two).
fn loose_joints(state: &mut State, reflection: &Reflection, part: InstanceId, world: InstanceId) -> (Vec<InstanceId>, Vec<InstanceId>) {
    let (mut single, mut double) = (Vec::new(), Vec::new());
    let name = |text: &str| reflection.name_id(text);
    let (Some(enabled), Some(attachment0), Some(attachment1), Some(part0), Some(part1)) = (name("Enabled"), name("Attachment0"), name("Attachment1"), name("Part0"), name("Part1")) else {
        return (single, double);
    };
    let Some(base_part) = reflection.class_id("BasePart") else { return (single, double) };
    let part_above = |state: &State, attachment: InstanceId| {
        let mut at = attachment;
        loop {
            at = state.dom.parent_of(at)?;
            if reflection.class(state.dom.at(at).class).is_a(base_part) {
                break Some(at);
            }
        }
    };
    let reference = |state: &State, id: InstanceId, name: NameId| match state.dom.raw_get(id, name) {
        Some(Value::Instance(target)) if state.dom.get(*target).is_some() => Some(*target),
        _ => None,
    };
    let on = |state: &State, id: InstanceId| state.dom.boolean(id, enabled) != Some(false);
    for text in ["Rotate", "RotateP", "RotateV"] {
        let Some(class) = reflection.class_id(text) else { continue };
        for joint in state.dom.instances_of_class(class) {
            if !on(state, joint) {
                continue;
            }
            let ends = [reference(state, joint, part0), reference(state, joint, part1)];
            let [Some(a), Some(b)] = ends else { continue };
            if a == b || (a != part && b != part) {
                continue;
            }
            let home = state.dom.home(joint);
            let homed = home == Some(world) || home.is_some_and(|home| reflection.class(state.dom.at(home).class).name.as_ref() == "JointsService");
            if homed && joints::world_root(state, reflection, a) == Some(world) && joints::world_root(state, reflection, b) == Some(world) {
                double.push(joint);
            }
        }
    }
    let Some(constraint) = reflection.class_id("Constraint") else { return (single, double) };
    let no_collision = reflection.class_id("NoCollisionConstraint");
    let one_attachment: Vec<_> = ["VectorForce", "Torque", "LinearVelocity", "AngularVelocity"].iter().filter_map(|text| reflection.class_id(text)).collect();
    let classes: Vec<_> = (0..reflection.classes.len() as crate::ClassId)
        .filter(|&class| reflection.class(class).is_a(constraint) && Some(class) != no_collision)
        .collect();
    for class in classes {
        let single_class = one_attachment.iter().any(|&one| reflection.class(class).is_a(one));
        for held in state.dom.instances_of_class(class) {
            if !on(state, held) || joints::world_root(state, reflection, held) != Some(world) {
                continue;
            }
            let ends = [reference(state, held, attachment0).and_then(|a| part_above(state, a)), reference(state, held, attachment1).and_then(|a| part_above(state, a))];
            if single_class {
                if ends[0] == Some(part) {
                    single.push(held);
                }
                continue;
            }
            let [Some(a), Some(b)] = ends else { continue };
            if a != b && (a == part || b == part) && joints::world_root(state, reflection, a) == Some(world) && joints::world_root(state, reflection, b) == Some(world) {
                double.push(held);
            }
        }
    }
    let serial = |state: &State, id: InstanceId| state.dom.at(id).serial;
    single.sort_by_key(|&id| serial(state, id));
    double.sort_by_key(|&id| serial(state, id));
    (single, double)
}

/// BasePart:GetJoints: the part's one-attachment constraints, its joints in their list ([`joints::part_joints`]), then its other
/// constraints and Rotate joints. A client keeps all of them in the one list (a one-attachment constraint takes the list's first
/// place after the others of its kind, the joint there going to the end); ree keeps the rigid joints' list and puts the constraints
/// and Rotate joints, which it does not index, around it in the order they were made (docs/engine.md "Not followed").
fn joints_of(state: &mut State, reflection: &Reflection, part: InstanceId) -> Vec<InstanceId> {
    let Some(world) = joints::world_root(state, reflection, part) else { return Vec::new() };
    let (single, double) = loose_joints(state, reflection, part, world);
    let mut found = single;
    found.extend(joints::part_joints(state, reflection, part).into_iter().map(|entry| entry.joint));
    found.extend(double);
    found
}

/// BasePart:GetNoCollisionConstraints: the Enabled NoCollisionConstraints in the part's world that name it, in the order they were
/// made (none outside every world).
fn no_collision_constraints(state: &mut State, reflection: &Reflection, part: InstanceId) -> Vec<InstanceId> {
    let Some(world) = joints::world_root(state, reflection, part) else { return Vec::new() };
    let (Some(class), Some(enabled), Some(part0), Some(part1)) = (reflection.class_id("NoCollisionConstraint"), reflection.name_id("Enabled"), reflection.name_id("Part0"), reflection.name_id("Part1")) else {
        return Vec::new();
    };
    let mut found: Vec<InstanceId> = state
        .dom
        .instances_of_class(class)
        .into_iter()
        .filter(|&held| {
            let names = |name| matches!(state.dom.raw_get(held, name), Some(Value::Instance(target)) if *target == part);
            state.dom.boolean(held, enabled) != Some(false) && (names(part0) || names(part1)) && joints::world_root(state, reflection, held) == Some(world)
        })
        .collect();
    found.sort_by_key(|&id| state.dom.at(id).serial);
    found
}

/// Whether an active NoCollisionConstraint holds `a` and `b` apart.
fn held_apart(state: &mut State, reflection: &Reflection, a: InstanceId, b: InstanceId) -> bool {
    let (Some(part0), Some(part1)) = (reflection.name_id("Part0"), reflection.name_id("Part1")) else { return false };
    no_collision_constraints(state, reflection, a).into_iter().any(|held| {
        let names = |name| matches!(state.dom.raw_get(held, name), Some(Value::Instance(target)) if *target == b);
        names(part0) || names(part1)
    })
}

/// The network owner a server keeps for each assembly, by its root: the player, and whether ownership is automatic (each root's
/// entry made by its first SetNetworkOwner or SetNetworkOwnershipAuto; until then the server owns it, automatically).
#[derive(Default)]
struct Owners {
    by_root: RefCell<FxHashMap<InstanceId, (Option<InstanceId>, bool)>>,
}

/// The checks every Network Ownership call makes, in a client's order: a part outside the Workspace, a client, an anchored assembly.
fn ownership_refused(engine: &Engine, part: InstanceId) -> Option<&'static str> {
    let workspace = engine.world().workspace;
    let mut state = engine.state.borrow_mut();
    if !state.dom.is_descendant_of(part, workspace) {
        return Some("Can only call Network Ownership API on a part that is descendent of Workspace");
    }
    if !engine.options.server() {
        return Some("Network Ownership API can only be called from the Server.");
    }
    let root = joints::root_part(&mut state, &engine.reflection, part);
    if root.is_none_or(|root| joints::is_anchored(&state, root)) {
        return Some("Network Ownership API cannot be called on Anchored parts or parts welded to Anchored parts.");
    }
    None
}

fn owner_root(engine: &Engine, part: InstanceId) -> InstanceId {
    joints::root_part(&mut engine.state.borrow_mut(), &engine.reflection, part).unwrap_or(part)
}

/// What BasePart:Resize does to a part's size and place: `delta` studs on the face `normal`, the face moving out along its normal by
/// that much and the opposite face staying (a size below 0.001 is 0.001, the place moved by the whole delta); a Ball grows evenly.
fn resized(size: Vec3, cframe: &Affine3A, normal: &str, delta: f32, ball: bool) -> (Vec3, Affine3A) {
    let (axis, sign) = match normal {
        "Right" => (0, 1.0),
        "Left" => (0, -1.0),
        "Top" => (1, 1.0),
        "Bottom" => (1, -1.0),
        "Back" => (2, 1.0),
        _ => (2, -1.0),
    };
    let mut new_size = size;
    new_size[axis] = (size[axis] + delta).max(0.001);
    if ball {
        new_size = Vec3::splat(new_size[axis]);
    }
    let mut local = Vec3::ZERO;
    local[axis] = sign * delta * 0.5;
    let mut moved = *cframe;
    moved.translation += cframe.matrix3 * glam::Vec3A::from(local);
    (new_size, moved)
}

/// The shapes GetClosestPointOnSurface knows (measured: Block, Ball, Cylinder along X, Wedge and CornerWedge as parts and as Shapes;
/// a mesh-less MeshPart is its box).
enum Shape {
    Block,
    Ball,
    Cylinder,
    Wedge,
    CornerWedge,
}

/// The point of `shape` (half size `half`, in its own frame) nearest `p`, or `p` itself inside it.
fn closest_local(shape: &Shape, half: Vec3, p: Vec3) -> Vec3 {
    match shape {
        Shape::Block => p.clamp(-half, half),
        Shape::Ball => {
            let radius = half.min_element();
            if p.length() <= radius { p } else { p.normalize_or_zero() * radius }
        }
        Shape::Cylinder => {
            let radius = half.y.min(half.z);
            let x = p.x.clamp(-half.x, half.x);
            let across = glam::Vec2::new(p.y, p.z);
            let across = if across.length() <= radius { across } else { across.normalize_or_zero() * radius };
            Vec3::new(x, across.x, across.y)
        }
        Shape::Wedge | Shape::CornerWedge => {
            let (x, y, z) = (half.x, half.y, half.z);
            // (a wedge: the box without its top front edge; a corner wedge: its bottom face and the top corner at +X, -Z)
            let vertices: Vec<Vec3> = match shape {
                Shape::Wedge => vec![
                    Vec3::new(-x, -y, -z), Vec3::new(x, -y, -z), Vec3::new(x, -y, z), Vec3::new(-x, -y, z), Vec3::new(-x, y, z), Vec3::new(x, y, z),
                ],
                _ => vec![Vec3::new(-x, -y, -z), Vec3::new(x, -y, -z), Vec3::new(x, -y, z), Vec3::new(-x, -y, z), Vec3::new(x, y, -z)],
            };
            closest_on_hull(&vertices, p)
        }
    }
}

/// The point of the convex hull of `vertices` nearest `p`, or `p` inside it.
fn closest_on_hull(vertices: &[Vec3], p: Vec3) -> Vec3 {
    let mut faces: Vec<[usize; 3]> = Vec::new();
    let mut inside = true;
    let count = vertices.len();
    for i in 0..count {
        for j in i + 1..count {
            for k in j + 1..count {
                let normal = (vertices[j] - vertices[i]).cross(vertices[k] - vertices[i]);
                if normal.length_squared() < 1e-12 {
                    continue;
                }
                let (mut above, mut below) = (false, false);
                for m in 0..count {
                    let side = normal.dot(vertices[m] - vertices[i]);
                    above |= side > 1e-6;
                    below |= side < -1e-6;
                }
                if above && below {
                    continue;
                }
                let outward = if above { -normal } else { normal };
                if outward.dot(p - vertices[i]) > 0.0 {
                    inside = false;
                }
                faces.push([i, j, k]);
            }
        }
    }
    if inside {
        return p;
    }
    let mut best = p;
    let mut nearest = f32::INFINITY;
    for [a, b, c] in faces {
        let point = closest_on_triangle(p, vertices[a], vertices[b], vertices[c]);
        let distance = (point - p).length_squared();
        if distance < nearest {
            nearest = distance;
            best = point;
        }
    }
    best
}

/// The point of the triangle abc nearest p (by its regions: Ericson, Real-Time Collision Detection 5.1.5).
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denominator = 1.0 / (va + vb + vc);
    a + ab * (vb * denominator) + ac * (vc * denominator)
}

pub fn install(engine: &Rc<Engine>) {
    let reflection = &engine.reflection;
    let name = |text: &str| reflection.name_id(text);
    let class = |text: &str| reflection.class_id(text);
    let (Some(base_part), Some(model), Some(attachment), Some(joint_instance)) = (class("BasePart"), class("Model"), class("Attachment"), class("JointInstance")) else {
        return;
    };
    let mut behaviors = engine.behaviors.borrow_mut();
    // (the Workspace's own BreakJoints and MakeJoints take a list of parts and lack Plugin: their capability errors are bound already,
    // but in edit mode, where a plugin's call gets past the check, they would reach Model's; they stay not emulated there)
    let workspace = class("Workspace");
    let unbound: Vec<&'static str> = ["BreakJoints", "MakeJoints"]
        .into_iter()
        .filter(|text| {
            let (Some(workspace), Some(id)) = (workspace, name(text)) else { return false };
            !behaviors.find_method(reflection, workspace, id).is_some_and(|(owner, _)| owner == workspace)
        })
        .collect();
    let mut add = |owner: crate::ClassId, text: &str, f: crate::services::MethodFn| {
        if let Some(id) = name(text) {
            behaviors.add_method(owner, id, f);
        }
    };

    add(base_part, "GetRootPart", method(|lua, engine, id, _| {
        let root = joints::root_part(&mut engine.state.borrow_mut(), &engine.reflection, id);
        Ok(values([match root {
            Some(root) => instance_value(lua, engine, root)?,
            None => LuaValue::Nil,
        }]))
    }));
    add(base_part, "GetConnectedParts", method(|lua, engine, id, args| {
        let recursive = bool_argument(engine, args, 1)?;
        let found = connected(&mut engine.state.borrow_mut(), &engine.reflection, id, recursive);
        charge(engine, found.len() as f64 * crate::cost::CONNECTED_PART);
        list(lua, engine, &found)
    }));
    add(base_part, "GetJoints", method(|lua, engine, id, _| {
        let found = joints_of(&mut engine.state.borrow_mut(), &engine.reflection, id);
        charge(engine, found.len() as f64 * crate::cost::JOINT_LISTED);
        list(lua, engine, &found)
    }));
    add(base_part, "GetNoCollisionConstraints", method(|lua, engine, id, _| {
        let found = no_collision_constraints(&mut engine.state.borrow_mut(), &engine.reflection, id);
        charge(engine, found.len() as f64 * crate::cost::JOINT_LISTED);
        list(lua, engine, &found)
    }));
    add(base_part, "IsGrounded", method(|_, engine, id, _| {
        let mut state = engine.state.borrow_mut();
        let root = joints::root_part(&mut state, &engine.reflection, id);
        Ok(values([LuaValue::Boolean(root.is_some_and(|root| joints::is_anchored(&state, root)))]))
    }));
    let cframe = name("CFrame");
    add(base_part, "GetRenderCFrame", method(move |lua, engine, id, _| {
        let value = cframe.and_then(|cframe| engine.state.borrow().dom.raw_get(id, cframe).cloned());
        Ok(values([crate::lua::convert::to_lua(lua, engine, &frame::cframe_value(&frame::affine_of(value.as_ref())))?]))
    }));
    // (MakeJoints: surfaces join nothing now; BreakJoints: the part's rigid joints, measured)
    for text in ["MakeJoints", "makeJoints"] {
        add(base_part, text, method(|_, _, _, _| Ok(none())));
        add(model, text, method(|_, _, _, _| Ok(none())));
    }
    // (its lower-case breakJoints and makeJoints are Model's, measured: docs/engine.md "CamelCase")
    if let Some(workspace) = workspace {
        for text in unbound {
            add(workspace, text, method(move |lua, engine, _, _| {
                engine.missing(Some(lua), "Workspace", text);
                Ok(none())
            }));
        }
    }
    for text in ["BreakJoints", "breakJoints"] {
        add(base_part, text, method(|lua, engine, id, _| {
            break_joints(lua, engine, &[id])?;
            Ok(none())
        }));
        add(model, text, method(move |lua, engine, id, _| {
            let parts: Vec<InstanceId> = {
                let state = engine.state.borrow();
                state.dom.descendants(id).into_iter().filter(|&each| engine.reflection.class(state.dom.at(each).class).is_a(base_part)).collect()
            };
            break_joints(lua, engine, &parts)?;
            Ok(none())
        }));
    }
    add(base_part, "CanCollideWith", method(move |_, engine, id, args| {
        let Some(other) = object_argument(args, 1)? else { return raise("Argument 1 missing or nil") };
        Ok(values([LuaValue::Boolean(can_collide_with(engine, id, other)?)]))
    }));
    add(base_part, "GetClosestPointOnSurface", method(|lua, engine, id, args| {
        let point = vector_argument(engine, args, 1)?;
        let closest = closest_point(engine, id, point);
        vector_value(lua, engine, closest)
    }));
    let resize = method(|lua, engine, id, args| {
        let normal = normal_argument(lua, engine, args, 1)?;
        let delta = int_argument(engine, args, 2)?;
        Ok(values([LuaValue::Boolean(resize_part(engine, id, &normal, delta))]))
    });
    add(base_part, "Resize", resize.clone());
    add(base_part, "resize", resize);

    let owners = Rc::new(Owners::default());
    let shared = owners.clone();
    add(base_part, "GetNetworkOwner", method(move |lua, engine, id, _| {
        if let Some(refused) = ownership_refused(engine, id) {
            return raise(refused);
        }
        let root = owner_root(engine, id);
        let owner = shared.by_root.borrow().get(&root).and_then(|(owner, _)| *owner);
        Ok(values([match owner {
            Some(owner) if engine.state.borrow().dom.get(owner).is_some() => instance_value(lua, engine, owner)?,
            _ => LuaValue::Nil,
        }]))
    }));
    let shared = owners.clone();
    add(base_part, "GetNetworkOwnershipAuto", method(move |_, engine, id, _| {
        if let Some(refused) = ownership_refused(engine, id) {
            return raise(refused);
        }
        let root = owner_root(engine, id);
        Ok(values([LuaValue::Boolean(shared.by_root.borrow().get(&root).is_none_or(|(_, auto)| *auto))]))
    }));
    let shared = owners.clone();
    let player = class("Player");
    add(base_part, "SetNetworkOwner", method(move |_, engine, id, args| {
        let owner = object_argument(args, 1)?;
        if let Some(refused) = ownership_refused(engine, id) {
            return raise(refused);
        }
        let root = owner_root(engine, id);
        // (an instance that is no Player sets no player: a server takes a Folder without an error)
        let owner = owner.filter(|&owner| player.is_some_and(|player| engine.reflection.class(engine.state.borrow().dom.at(owner).class).is_a(player)));
        shared.by_root.borrow_mut().insert(root, (owner, false));
        Ok(none())
    }));
    let shared = owners;
    add(base_part, "SetNetworkOwnershipAuto", method(move |_, engine, id, _| {
        if let Some(refused) = ownership_refused(engine, id) {
            return raise(refused);
        }
        let root = owner_root(engine, id);
        shared.by_root.borrow_mut().insert(root, (None, true));
        Ok(none())
    }));
    add(base_part, "CanSetNetworkOwnership", method(|lua, engine, id, _| match ownership_refused(engine, id) {
        Some(refused) => Ok(values([LuaValue::Boolean(false), LuaValue::String(lua.create_string(refused)?)])),
        None => Ok(values([LuaValue::Boolean(true)])),
    }));

    // Model's persistent players: a server streams to them, a client refuses (measured: GetPersistentPlayers lists none either way)
    add(model, "GetPersistentPlayers", method(|lua, engine, _, _| list(lua, engine, &[])));
    for text in ["AddPersistentPlayer", "RemovePersistentPlayer"] {
        add(model, text, method(|_, engine, _, args| {
            if object_argument(args, 1)?.is_none() {
                return raise("PersistentPerPlayer player not specified.");
            }
            if !engine.options.server() {
                return raise("PersistentPerPlayer API can only be called from the Server.");
            }
            Ok(none())
        }));
    }

    // Attachment: the old axis methods are the Axis and SecondaryAxis properties; GetConstraints the constraints that name it
    for (text, property) in [("GetAxis", "Axis"), ("GetSecondaryAxis", "SecondaryAxis")] {
        let key = property;
        add(attachment, text, method(move |lua, engine, id, _| {
            let key = LuaValue::String(lua.create_string(key)?);
            Ok(values([crate::lua::instance::get(lua, engine, id, &key)?]))
        }));
    }
    for (text, property) in [("SetAxis", "Axis"), ("SetSecondaryAxis", "SecondaryAxis")] {
        let key = property;
        add(attachment, text, method(move |lua, engine, id, args| {
            vector_argument(engine, args, 1)?;
            let key = LuaValue::String(lua.create_string(key)?);
            crate::lua::instance::assign(lua, engine, id, &key, args.get(1))?;
            Ok(none())
        }));
    }
    add(attachment, "GetConstraints", method(|lua, engine, id, _| {
        let found = attachment_constraints(engine, id);
        charge(engine, found.len() as f64 * crate::cost::JOINT_LISTED);
        list(lua, engine, &found)
    }));

    // Motor:SetDesiredAngle writes DesiredAngle (a float argument)
    if let Some(motor) = class("Motor") {
        add(motor, "SetDesiredAngle", method(|lua, engine, id, args| {
            let angle = camera::float_argument(lua, engine, args, 1)?;
            let key = LuaValue::String(lua.create_string("DesiredAngle")?);
            crate::lua::instance::assign(lua, engine, id, &key, &LuaValue::Number(f64::from(angle)))?;
            Ok(none())
        }));
    }
    drop(add);

    if let Some(active_name) = name("Active") {
        for owner in [Some(joint_instance), class("WeldConstraint")].into_iter().flatten() {
            behaviors.add_getter(owner, active_name, getter(|_, engine, id| {
                Ok(LuaValue::Boolean(active(&mut engine.state.borrow_mut(), &engine.reflection, id)))
            }));
        }
    }
}

/// BasePart:BreakJoints of each of `parts`: its rigid joints in its list, the JointInstances first, then the WeldConstraints (the
/// order a client's destroys fire AncestryChanged in), each destroyed as a script's Destroy is.
fn break_joints(lua: &Lua, engine: &Rc<Engine>, parts: &[InstanceId]) -> Result<(), Flow> {
    for &part in parts {
        let entries = joints::part_joints(&mut engine.state.borrow_mut(), &engine.reflection, part);
        let order: Vec<InstanceId> = entries.iter().filter(|entry| !entry.weld_constraint).chain(entries.iter().filter(|entry| entry.weld_constraint)).map(|entry| entry.joint).collect();
        for joint in order {
            if engine.state.borrow().dom.get(joint).is_some() {
                crate::lua::instance::destroy(lua, engine, joint)?;
            }
        }
    }
    Ok(())
}

/// BasePart:CanCollideWith (measured): an argument that is no part raises; two parts of different WorldRoots (or one outside every
/// world) raise; a part with itself, a part with CanCollide off, two parts of one assembly, two a NoCollisionConstraint holds apart
/// and two of collision groups that do not collide are false.
fn can_collide_with(engine: &Engine, id: InstanceId, other: InstanceId) -> Result<bool, Flow> {
    let reflection = &engine.reflection;
    let (Some(base_part), Some(can_collide), Some(group)) = (reflection.class_id("BasePart"), reflection.name_id("CanCollide"), reflection.name_id("CollisionGroup")) else {
        return Ok(true);
    };
    let mut state = engine.state.borrow_mut();
    if !state.dom.get(other).is_some_and(|instance| reflection.class(instance.class).is_a(base_part)) {
        return raise("CanCollideWith expects a BasePart argument.");
    }
    let world = joints::world_root(&state, reflection, id);
    if world.is_none() || world != joints::world_root(&state, reflection, other) {
        return raise("CanCollideWith expects both parts to be descendants of the same WorldRoot.");
    }
    if id == other || state.dom.boolean(id, can_collide) == Some(false) || state.dom.boolean(other, can_collide) == Some(false) {
        return Ok(false);
    }
    if joints::root_part(&mut state, reflection, id) == joints::root_part(&mut state, reflection, other) {
        return Ok(false);
    }
    if held_apart(&mut state, reflection, id, other) {
        return Ok(false);
    }
    let name = match state.dom.raw_get(id, group) {
        Some(Value::String(text)) => text.as_bytes().to_vec(),
        _ => b"Default".to_vec(),
    };
    let world = world.expect("checked");
    drop(state);
    let (mask, _) = crate::services::physics::query_mask(engine, world, &name);
    let Some((mask, id_name)) = mask else { return Ok(true) };
    let state = engine.state.borrow();
    Ok(match state.dom.raw_get(other, id_name) {
        Some(Value::Number(group_id)) if (0.0..32.0).contains(group_id) => mask & (1u32 << (*group_id as u32)) != 0,
        _ => true,
    })
}

/// BasePart:GetClosestPointOnSurface: the point of the part's shape nearest `point`, `point` itself inside it.
fn closest_point(engine: &Engine, id: InstanceId, point: Vec3) -> Vec3 {
    let reflection = &engine.reflection;
    let state = engine.state.borrow();
    let get = |text: &str| reflection.name_id(text).and_then(|name| state.dom.raw_get(id, name));
    let cframe = frame::affine_of(get("CFrame"));
    let size = get("Size").and_then(frame::vector_of).unwrap_or(Vec3::ONE);
    let class = reflection.class(state.dom.at(id).class).name.to_string();
    let shape = match get("Shape").and_then(Value::as_enum).map(|item| reflection.enum_item_info(item).name.to_string()) {
        Some(shape) if class == "Part" => match shape.as_str() {
            "Ball" => Shape::Ball,
            "Cylinder" => Shape::Cylinder,
            "Wedge" => Shape::Wedge,
            "CornerWedge" => Shape::CornerWedge,
            _ => Shape::Block,
        },
        _ => match class.as_str() {
            "WedgePart" => Shape::Wedge,
            "CornerWedgePart" => Shape::CornerWedge,
            _ => Shape::Block,
        },
    };
    // (into the part's frame by its rotation's transpose and back, each sum in order: a client's answers to the bit)
    let m = cframe.matrix3;
    let rows = [[m.x_axis.x, m.y_axis.x, m.z_axis.x], [m.x_axis.y, m.y_axis.y, m.z_axis.y], [m.x_axis.z, m.y_axis.z, m.z_axis.z]];
    let at = frame::translation(&cframe);
    let d = point - at;
    let local = Vec3::new(
        rows[0][0] * d.x + rows[1][0] * d.y + rows[2][0] * d.z,
        rows[0][1] * d.x + rows[1][1] * d.y + rows[2][1] * d.z,
        rows[0][2] * d.x + rows[1][2] * d.y + rows[2][2] * d.z,
    );
    let closest = closest_local(&shape, size * 0.5, local);
    if closest == local {
        return point;
    }
    let world = |row: [f32; 3]| row[0] * closest.x + row[1] * closest.y + row[2] * closest.z;
    Vec3::new(world(rows[0]) + at.x, world(rows[1]) + at.y, world(rows[2]) + at.z)
}

/// Resize's face argument: a NormalId item, its name or number (nil raises "Argument 1 missing or nil").
fn normal_argument(lua: &Lua, engine: &Engine, args: &Args, at: usize) -> Result<String, Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    let item = engine.reflection.enum_id("NormalId").and_then(|id| crate::lua::convert::coerce_enum(lua, engine, id, value));
    match item {
        Some(item) => Ok(engine.reflection.enum_item_info(item).name.to_string()),
        None => raise(format!("Unable to cast {} to NormalId", camera::cast_name(engine, value))),
    }
}

/// Resize's int argument: a number rounded to the nearest whole (1.7 grows by 2), nil raises "Argument n missing or nil".
fn int_argument(engine: &Engine, args: &Args, at: usize) -> Result<f32, Flow> {
    match args.get(at) {
        LuaValue::Nil => raise(format!("Argument {at} missing or nil")),
        LuaValue::Number(number) => Ok(number.round() as f32),
        LuaValue::Integer(number) => Ok(*number as f32),
        other => raise(format!("Unable to cast {} to int", camera::cast_name(engine, other))),
    }
}

/// BasePart:Resize: grows the part on a face ([`resized`]) and answers true, unless in a world the grown part would overlap another
/// part that can collide (the part unchanged, false).
fn resize_part(engine: &Engine, id: InstanceId, normal: &str, delta: f32) -> bool {
    let reflection = &engine.reflection;
    let (Some(size_name), Some(cframe_name)) = (reflection.name_id("Size"), reflection.name_id("CFrame")) else { return false };
    let (size, cframe, ball, world) = {
        let state = engine.state.borrow();
        let size = state.dom.raw_get(id, size_name).and_then(frame::vector_of).unwrap_or(Vec3::ONE);
        let cframe = frame::affine_of(state.dom.raw_get(id, cframe_name));
        let ball = reflection
            .name_id("Shape")
            .and_then(|shape| state.dom.raw_get(id, shape))
            .and_then(Value::as_enum)
            .is_some_and(|item| &*reflection.enum_item_info(item).name == "Ball");
        (size, cframe, ball, joints::world_root(&state, reflection, id))
    };
    let (new_size, moved) = resized(size, &cframe, normal, delta, ball);
    if let Some(world) = world
        && box_blocked(engine, world, id, &moved, new_size)
    {
        return false;
    }
    crate::lua::instance::script_set_property(engine, id, size_name, frame::vector_value(new_size));
    crate::lua::instance::script_set_property(engine, id, cframe_name, frame::cframe_value(&moved));
    true
}

/// Whether the box (`frame`, `size`) in `world` overlaps a part other than `id` that can collide (Resize's check), by the world's
/// index as a spatial query reads it (taking in what came in first: world/query.rs take_in_of).
fn box_blocked(engine: &Engine, world: InstanceId, id: InstanceId, frame: &Affine3A, size: Vec3) -> bool {
    let Some(index) = super::query::take_in_of(engine, world) else { return false };
    let m = glam::Mat3::from(frame.matrix3);
    let half = size * 0.5;
    let extent = m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
    let center = frame::translation(frame);
    super::query::walk_box(engine, index, center - extent, center + extent, 0, |near, _| {
        near.iter().any(|(other, entry)| *other != id && entry.can_collide && super::query::overlaps_box(&entry.solid, frame, size))
    })
}

/// Attachment:GetConstraints: the constraints in the DataModel that name the attachment (as Attachment0 or Attachment1), Enabled or
/// not, in the order they were made.
fn attachment_constraints(engine: &Engine, id: InstanceId) -> Vec<InstanceId> {
    let reflection = &engine.reflection;
    let (Some(constraint), Some(attachment0), Some(attachment1)) = (reflection.class_id("Constraint"), reflection.name_id("Attachment0"), reflection.name_id("Attachment1")) else {
        return Vec::new();
    };
    let game = engine.world().game;
    let mut state = engine.state.borrow_mut();
    let classes: Vec<_> = (0..reflection.classes.len() as crate::ClassId).filter(|&class| reflection.class(class).is_a(constraint)).collect();
    let mut found = Vec::new();
    for class in classes {
        for held in state.dom.instances_of_class(class) {
            let names = |name| matches!(state.dom.raw_get(held, name), Some(Value::Instance(target)) if *target == id);
            if (names(attachment0) || names(attachment1)) && state.dom.is_descendant_of(held, game) {
                found.push(held);
            }
        }
    }
    found.sort_by_key(|&held| state.dom.at(held).serial);
    found
}
