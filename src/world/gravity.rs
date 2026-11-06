//! Free fall: an assembly in the Workspace with no anchored part and nothing that can collide falls
//! under Workspace.Gravity from the simulation step after it is there, as Roblox's physics moves it
//! (play mode only: edit mode simulates nothing).
//!
//! Measured in a capture Studio (presence, 2026-10-17): a 4 x 1 x 4 unanchored CanCollide-false Part
//! parented to the Workspace by a playtest's client, and an effect scene cloned there by the
//! client (its unanchored scene mesh), fall from their first frame at 196.2 studs/s² (drop ½ g t²
//! within a frame's time: 37.5 studs at 0.62 s, 99.8 at 0.99), near the character, 600 and 2,000
//! studs off, 2,048 up, the camera on them or not; Run mode (the server) alike.
//!
//! A place that streams (Workspace.StreamingEnabled) simulates only near its replication foci: with
//! it on, the same client parts stand still 2,900 studs off and 1,104 off (StreamingTargetRadius
//! 1,024) and fall 905 off, the camera and its Focus moved onto them or not. The foci here: the
//! local character's PrimaryPart (else HumanoidRootPart), Player.ReplicationFocus and the parts
//! Player:AddReplicationFocus adds (a server sets them; a harness stands in); with none, nothing is
//! held (not measured). How far (`reach`, measured 2026-10-17 in a playtest client of a place with
//! StreamingTargetRadius 400, PauseOutsideLoadedArea, Opportunistic): an assembly whose bounding
//! sphere starts about a focus simulates while its centre's distance plus its radius stays within
//! the target radius cut to a whole 64 studs (384) for the character and a ReplicationFocus part, 32
//! less (352) for an added focus, and then stands still keeping its velocity (cubes 100-400 studs
//! froze at 384.0-384.15 and 351.6-352.3; an effect's 400-stud sphere cage about its focus anchor
//! falls 5.9 studs in 0.21 s in Studio, another's, 170, about 205); one that
//! starts clear of every focus falls on wherever it goes (4-stud cubes 300-380 out and a 200-stud one
//! 200 out past 530), and starts within that reach about the character (380 out does, 390 does not),
//! within the target radius about an added focus (400 out does).
//!
//! LockedToPart particles do not follow what physics moves (measured: four locked particles, their
//! emitters enabled or not, TimeScale 1 or 0.125, stayed where they were while their parts fell 27
//! studs; a CFrame write carries them): the recorder marks a part's motion from the frame physics
//! first moved it (`physicsFrom`), and a renderer leaves its locked particles behind from there.
//!
//! Not modelled (such an assembly stays where it is, as before): collisions (an assembly with a part
//! that can collide), Humanoids (a part beside one), constraints and body movers under its parts,
//! angular velocity, FallenPartsDestroyHeight.

use std::rc::Rc;

use glam::{Affine3A, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{frame, joints};
use rbx_dom_weak::types::Variant;

use crate::{ClassId, Engine, InstanceId, NameId, Reflection, State, instance as core, value::Value};

/// Names and classes, resolved once at install.
struct Setup {
    anchored: NameId,
    can_collide: NameId,
    cframe: NameId,
    velocity: NameId,
    gravity: NameId,
    streaming: Option<NameId>,
    radius: Option<NameId>,
    character: Option<NameId>,
    primary_part: Option<NameId>,
    focus: Option<NameId>,
    mesh_id: Option<NameId>,
    size: Option<NameId>,
    initial_size: Option<NameId>,
    mesh_part: Option<ClassId>,
    base_part: ClassId,
    world_root: ClassId,
    workspace: ClassId,
    humanoid: Option<ClassId>,
    attachment: Option<ClassId>,
    /// Constraint, BodyMover: what pushes an assembly besides gravity
    movers: Vec<ClassId>,
}

/// The free-fall state of one engine (in [`State`]).
#[derive(Default)]
pub struct Gravity {
    setup: Option<Rc<Setup>>,
    /// Whether the parts already in the Workspace at the first step have been listed
    scanned: bool,
    /// Unanchored parts that entered the Workspace, in order (checked each step)
    loose: Vec<InstanceId>,
    listed: FxHashSet<InstanceId>,
    /// Each falling part's velocity (the parts physics has moved)
    velocity: FxHashMap<InstanceId, Vec3>,
    /// Falling parts something else moved too (a script's CFrame write, a joint carrying them)
    steered: FxHashSet<InstanceId>,
    /// While the step writes its own moves
    stepping: bool,
    /// The parts Player:AddReplicationFocus added, in order
    added: Vec<InstanceId>,
    /// Falling parts whose assembly started about a focus (held to its reach)
    bounded: FxHashSet<InstanceId>,
}

impl Gravity {
    /// Whether physics has moved a part.
    pub fn moved(&self, id: InstanceId) -> bool {
        self.velocity.contains_key(&id)
    }

    /// Whether a part physics moves was also moved by something else since.
    pub fn steered(&self, id: InstanceId) -> bool {
        self.steered.contains(&id)
    }
}

pub fn install(engine: &Rc<Engine>) {
    let reflection = &engine.reflection;
    let name = |text: &str| reflection.name_id(text);
    let class = |text: &str| reflection.class_id(text);
    let (Some(anchored), Some(can_collide), Some(cframe), Some(velocity), Some(gravity), Some(base_part), Some(world_root), Some(workspace)) = (
        name("Anchored"), name("CanCollide"), name("CFrame"), name("Velocity"), name("Gravity"),
        class("BasePart"), class("WorldRoot"), class("Workspace"),
    ) else {
        return;
    };
    let setup = Rc::new(Setup {
        anchored,
        can_collide,
        cframe,
        velocity,
        gravity,
        streaming: name("StreamingEnabled"),
        radius: name("StreamingTargetRadius"),
        character: name("Character"),
        primary_part: name("PrimaryPart"),
        focus: name("ReplicationFocus"),
        mesh_id: name("MeshId"),
        size: name("Size"),
        initial_size: name("InitialSize"),
        mesh_part: class("MeshPart"),
        base_part,
        world_root,
        workspace,
        humanoid: class("Humanoid"),
        attachment: class("Attachment"),
        movers: ["Constraint", "BodyMover"].iter().filter_map(|text| class(text)).collect(),
    });
    engine.state.borrow_mut().gravity.setup = Some(setup.clone());
    let mut hooks = engine.hooks.borrow_mut();
    let entered = setup.clone();
    let reflection_for_parent = engine.reflection.clone();
    hooks.hierarchy_changed.push(Box::new(move |state, id, _, new| {
        if new.is_some() && in_workspace(state, &reflection_for_parent, &entered, id) {
            let mut nodes = vec![id];
            nodes.extend(state.dom.descendants(id));
            for node in nodes {
                note(state, &reflection_for_parent, &entered, node);
            }
        }
    }));
    let reflection_for_property = engine.reflection.clone();
    hooks.property_changed.push(Box::new(move |state, id, name| {
        if name == setup.anchored && in_workspace(state, &reflection_for_property, &setup, id) {
            note(state, &reflection_for_property, &setup, id);
        } else if name == setup.cframe && !state.gravity.stepping && state.gravity.velocity.contains_key(&id) {
            state.gravity.steered.insert(id);
        }
    }));
    hooks.instance_freed.push(Box::new(|state, id| {
        let gravity = &mut state.gravity;
        if gravity.listed.remove(&id) {
            gravity.loose.retain(|&part| part != id);
        }
        gravity.velocity.remove(&id);
        gravity.steered.remove(&id);
        gravity.bounded.remove(&id);
        gravity.added.retain(|&focus| focus != id);
    }));
    drop(hooks);

    // Player:AddReplicationFocus / RemoveReplicationFocus: the server's extra foci, which a harness calls in the client
    let Some(player_class) = reflection.class_id("Player") else { return };
    let mut behaviors = engine.behaviors.borrow_mut();
    if let Some(add) = name("AddReplicationFocus") {
        behaviors.add_method(player_class, add, crate::services::method(|_, engine, _, args| {
            if let Some(part) = args.instance(1) {
                let added = &mut engine.state.borrow_mut().gravity.added;
                if !added.contains(&part) {
                    added.push(part);
                }
            }
            Ok(crate::lua::none())
        }));
    }
    if let Some(remove) = name("RemoveReplicationFocus") {
        behaviors.add_method(player_class, remove, crate::services::method(|_, engine, _, args| {
            if let Some(part) = args.instance(1) {
                engine.state.borrow_mut().gravity.added.retain(|&focus| focus != part);
            }
            Ok(crate::lua::none())
        }));
    }
}

/// Whether an instance is in the Workspace's own world (not a WorldModel's).
fn in_workspace(state: &State, reflection: &Reflection, setup: &Setup, id: InstanceId) -> bool {
    let mut current = state.dom.parent_of(id);
    while let Some(node) = current {
        let class = reflection.class(state.dom.at(node).class);
        if class.is_a(setup.world_root) {
            return class.is_a(setup.workspace);
        }
        current = state.dom.parent_of(node);
    }
    false
}

/// Lists an unanchored part for the step.
fn note(state: &mut State, reflection: &Reflection, setup: &Setup, id: InstanceId) {
    if !reflection.class(state.dom.at(id).class).is_a(setup.base_part) || state.dom.boolean(id, setup.anchored) != Some(false) {
        return;
    }
    if state.gravity.listed.insert(id) {
        state.gravity.loose.push(id);
    }
}

/// Whether anything but gravity acts on a part: it can collide, sits beside a Humanoid, or holds a
/// constraint or body mover (on itself or an attachment of it).
fn held(state: &State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> bool {
    if state.dom.boolean(part, setup.can_collide) != Some(false) {
        return true;
    }
    if let (Some(humanoid), Some(parent)) = (setup.humanoid, state.dom.parent_of(part)) {
        if state.dom.find_child_of_class(parent, humanoid).is_some() {
            return true;
        }
    }
    let mover = |id: InstanceId| {
        let class = reflection.class(state.dom.at(id).class);
        setup.movers.iter().any(|&kind| class.is_a(kind))
    };
    state.dom.children_of(part).iter().any(|&child| {
        mover(child)
            || setup.attachment.is_some_and(|attachment| reflection.class(state.dom.at(child).class).is_a(attachment))
                && state.dom.children_of(child).iter().any(|&inner| mover(inner))
    })
}

/// A MeshPart without a mesh grown to 5 times its InitialSize or more on any axis, which a streaming
/// place never simulates (measured: an effect's FX.Emit part, saved mesh-less at InitialSize 1 and
/// Size 70, cloned in a streaming playtest's client, stands still at 70, 48, 5 and 70 x 1 x 1 and falls
/// at 4.5, 4 and 2, beside the character or 2,000 studs off once a focus reaches it, where a new
/// MeshPart of 70 and Parts of 70 fall; without streaming it falls).
fn unsimulated_in_stream(state: &State, setup: &Setup, part: InstanceId) -> bool {
    let (Some(mesh_part), Some(mesh_id), Some(size), Some(initial)) = (setup.mesh_part, setup.mesh_id, setup.size, setup.initial_size) else {
        return false;
    };
    if state.dom.at(part).class != mesh_part {
        return false;
    }
    let meshless = match state.dom.raw_get(part, mesh_id) {
        None => true,
        Some(value) => value.as_str().is_some_and(|text| text.is_empty()),
    };
    if !meshless {
        return false;
    }
    let vector = |value: Option<&Value>| match value {
        Some(Value::Vector3(v)) => Some(Vec3::new(v.x, v.y, v.z)),
        _ => None,
    };
    let stored = vector(state.dom.raw_get(part, initial)).filter(|v| v.max_element() > 0.0);
    let from_file = || {
        state.dom.at(part).unmodeled.as_ref().and_then(|unmodeled| {
            unmodeled.iter().find(|(name, _)| *name == "InitialSize").and_then(|(_, value)| match value {
                Variant::Vector3(v) => Some(Vec3::new(v.x, v.y, v.z)),
                _ => None,
            })
        })
    };
    let (Some(initial), Some(size)) = (stored.or_else(from_file), vector(state.dom.raw_get(part, size))) else {
        return false;
    };
    initial.max_element() > 0.0 && (size / initial.max(Vec3::splat(1e-6))).max_element() >= 5.0
}

/// A replication focus and how far its streamed area holds an assembly's bounding sphere.
struct Focus {
    at: Vec3,
    /// An assembly starting with its bounding sphere about this focus simulates while its centre's
    /// distance plus its radius stays within this
    reach: f32,
    /// Player:AddReplicationFocus's: an assembly clear of it starts while its centre is within the
    /// target radius (the character's and Player.ReplicationFocus's start within `reach`)
    added: bool,
}

/// Where a streaming place simulates: its foci and target radius (None: everywhere). The character's
/// and Player.ReplicationFocus's reach is StreamingTargetRadius cut down to a whole 64 studs, an added
/// focus's 32 less (measured in a playtest client of a streaming place: target 400 holds
/// bounding spheres to 384 about the character and a ReplicationFocus part, 352 about one
/// AddReplicationFocus adds; target 1,024 to 1,024).
fn reach(state: &State, reflection: &Reflection, setup: &Setup, engine: &Engine) -> Option<(Vec<Focus>, f32)> {
    let world = engine.world();
    let streams = setup.streaming.is_some_and(|name| state.dom.boolean(world.workspace, name) == Some(true));
    if !streams {
        return None;
    }
    let radius = setup.radius.and_then(|name| state.dom.number(world.workspace, name)).unwrap_or(1024.0) as f32;
    let whole = (radius / 64.0).floor() * 64.0;
    let position = |id: InstanceId| {
        let alive = state.dom.get(id).is_some_and(|instance| reflection.class(instance.class).is_a(setup.base_part));
        alive.then(|| Vec3::from(frame::affine_of(state.dom.raw_get(id, setup.cframe)).translation))
    };
    let reference = |id: InstanceId, name: Option<NameId>| name.and_then(|name| state.dom.raw_get(id, name)).and_then(|value| value.as_instance());
    let mut foci: Vec<Focus> = Vec::new();
    if let Some(player) = world.local_player {
        if let Some(character) = reference(player, setup.character).filter(|&id| state.dom.get(id).is_some()) {
            let root = reference(character, setup.primary_part).or_else(|| state.dom.find_child(character, b"HumanoidRootPart"));
            foci.extend(root.and_then(position).map(|at| Focus { at, reach: whole, added: false }));
        }
        foci.extend(reference(player, setup.focus).and_then(position).map(|at| Focus { at, reach: whole, added: false }));
    }
    foci.extend(state.gravity.added.iter().filter_map(|&id| position(id)).map(|at| Focus { at, reach: whole - 32.0, added: true }));
    (!foci.is_empty()).then_some((foci, radius))
}

/// An assembly's bounding sphere: each part's is its centre and half its Size's diagonal, whatever its
/// rotation (a turned sphere cage starts as its unturned size does); several are wrapped about
/// their centres' middle.
fn bounds(state: &State, setup: &Setup, component: &[InstanceId]) -> (Vec3, f32) {
    let spheres: Vec<(Vec3, f32)> = component
        .iter()
        .map(|&part| {
            let centre = Vec3::from(frame::affine_of(state.dom.raw_get(part, setup.cframe)).translation);
            let radius = match setup.size.and_then(|name| state.dom.raw_get(part, name)) {
                Some(Value::Vector3(v)) => Vec3::new(v.x, v.y, v.z).length() * 0.5,
                _ => 0.0,
            };
            (centre, radius)
        })
        .collect();
    let (low, high) = spheres.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(low, high), (centre, _)| (low.min(*centre), high.max(*centre)));
    let middle = (low + high) * 0.5;
    (middle, spheres.iter().map(|(centre, radius)| centre.distance(middle) + radius).fold(0.0, f32::max))
}

/// The simulation step: every free assembly falls for `delta` seconds.
pub fn step(engine: &Engine, delta: f64) {
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    let Some(setup) = state.gravity.setup.clone() else { return };
    let reflection = &engine.reflection;
    let workspace = engine.world().workspace;
    if !state.gravity.scanned {
        state.gravity.scanned = true;
        for node in state.dom.descendants(workspace) {
            if in_workspace(&state, reflection, &setup, node) {
                note(&mut state, reflection, &setup, node);
            }
        }
    }
    if state.gravity.loose.is_empty() || delta <= 0.0 {
        return;
    }
    let g = Vec3::new(0.0, -(state.dom.number(workspace, setup.gravity).unwrap_or(196.2) as f32), 0.0);
    let reach = reach(&state, reflection, &setup, engine);
    let dt = delta as f32;
    let candidates = state.gravity.loose.clone();
    let mut visited: FxHashSet<InstanceId> = FxHashSet::default();
    let mut gone = Vec::new();
    for part in candidates {
        if visited.contains(&part) {
            continue;
        }
        if state.dom.get(part).is_none() || state.dom.boolean(part, setup.anchored) != Some(false) || !in_workspace(&state, reflection, &setup, part) {
            gone.push(part);
            continue;
        }
        let component = joints::linked_parts(&mut state, reflection, part);
        visited.extend(component.iter().copied());
        if component.iter().any(|&other| state.dom.boolean(other, setup.anchored) != Some(false) || held(&state, reflection, &setup, other)) {
            continue;
        }
        if let Some((foci, radius)) = &reach {
            if component.iter().any(|&other| unsimulated_in_stream(&state, &setup, other)) {
                continue;
            }
            // one that started about a focus is held to that focus's reach, one that started clear of
            // every focus falls on wherever it goes (measured: cubes about the character froze with
            // their velocity as centre distance + half diagonal passed 384, 4-stud ones and a 200-stud
            // one 200 off fell on past 530)
            let (centre, radius_of) = bounds(&state, &setup, &component);
            let within = |focus: &Focus| focus.at.distance(centre) + radius_of <= focus.reach;
            let simulates = if state.gravity.velocity.contains_key(&part) {
                !state.gravity.bounded.contains(&part) || foci.iter().any(within)
            } else {
                let starts = foci.iter().any(|focus| within(focus) || focus.added && focus.at.distance(centre) >= radius_of && focus.at.distance(centre) <= *radius);
                if starts && foci.iter().any(|focus| focus.at.distance(centre) < radius_of) {
                    state.gravity.bounded.extend(component.iter().copied());
                }
                starts
            };
            if !simulates {
                continue;
            }
        }
        let velocity = match state.gravity.velocity.get(&part) {
            Some(&velocity) => velocity,
            None => state.dom.raw_get(part, setup.velocity).and_then(frame::vector_of).unwrap_or(Vec3::ZERO),
        };
        let moved = velocity * dt + 0.5 * g * dt * dt;
        let velocity = velocity + g * dt;
        state.gravity.stepping = true;
        for &other in &component {
            let cframe = frame::affine_of(state.dom.raw_get(other, setup.cframe));
            let target = Affine3A::from_translation(moved) * cframe;
            core::raw_set_unsignalled(&mut state, reflection, &hooks, other, setup.cframe, frame::cframe_value(&target));
            core::raw_set_unsignalled(&mut state, reflection, &hooks, other, setup.velocity, frame::vector_value(velocity));
            state.gravity.velocity.insert(other, velocity);
        }
        state.gravity.stepping = false;
    }
    if !gone.is_empty() {
        let gravity = &mut state.gravity;
        for part in gone {
            gravity.listed.remove(&part);
            gravity.velocity.remove(&part);
            gravity.bounded.remove(&part);
        }
        let listed = &gravity.listed;
        gravity.loose.retain(|part| listed.contains(part));
    }
}
