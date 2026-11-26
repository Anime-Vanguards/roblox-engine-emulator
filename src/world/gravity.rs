//! Free flight: an assembly in the Workspace with no anchored part and nothing that can collide flies from the
//! simulation step after it is there, as Roblox's physics moves it (play mode only: edit mode simulates nothing): under
//! Workspace.Gravity, at the velocity and with the spin a script gives any of its parts (AssemblyLinearVelocity or the legacy
//! Velocity, AssemblyAngularVelocity or RotVelocity; ApplyImpulse, ApplyImpulseAtPosition and ApplyAngularImpulse take effect
//! at its next step).
//!
//! Measured in a playing client (roblox-sim, Studio 0.744; notes/macro flyprobe.luau, bodyprobe.luau): each step updates the
//! velocity, then moves the assembly by it (semi-implicit Euler), four steps of 1/240 s in an assembly's first simulated
//! frame and one of 1/60 s a frame after: a dropped part, a thrown spinning cube, a thrown spinning brick and a thrown welded
//! pair follow it within 0.0001 studs over their first ten frames, where the exact parabola ree flew was 0.25 studs off. A
//! velocity set on either part of a welded pair flies the pair; one set before the part is in the Workspace holds. Spin
//! turns an assembly about its centre of mass as a free rigid body: its angular momentum is kept while its inertia turns
//! with it, and decays 5.2% a second (a 2-stud cube spun at 6 rad/s read 5.995, 5.990, 5.903 and 5.800 at its 2nd, 3rd, 20th
//! and 40th frames). Mass is volume times density, CustomPhysicalProperties' or else the Material's (Plastic 0.7, Slate
//! 2.691, Metal 7.85, a 0.1-stud cube 0.0007, no minimum), a Ball's volume a sphere's, a mesh-less MeshPart's its box; a
//! Massless part's Mass reads 0 and adds nothing to its assembly's, but as the assembly's root (alone, anchored, the best of an
//! all-Massless assembly) or outside every world, where it weighs what it would (Studio 0.747, tools/parity/probes/
//! assembly_mass.luau, as an anchored assembly's AssemblyMass reading inf). An impulse is divided by the assembly's mass, an angular
//! one by its inertia (a cube's spin from ApplyAngularImpulse(m * (0, 4, 0)) read 6 rad/s less one frame's decay), and both
//! add to a velocity a script set.
//!
//! Measured in a capture Studio (presence, 2026-10-17): a 4 x 1 x 4 unanchored CanCollide-false Part
//! parented to the Workspace by a playtest's client, and an effect scene cloned there by the
//! client (its unanchored scene mesh), fall from their first frame at 196.2 studs/s² (37.5 studs
//! at 0.62 s, 99.8 at 0.99), near the character, 600 and 2,000 studs off, 2,048 up, the camera on
//! them or not; Run mode (the server) alike.
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
//! Not modelled: collisions (an assembly with a part that can collide stays where it is, as before), Humanoids (a part
//! beside one), constraints and body movers under its parts, FallenPartsDestroyHeight; a mesh's own volume and inertia (a
//! MeshPart weighs and turns as its box); the steps a client adds when a frame runs long (five 1/240 s steps in a slow
//! first frame) or the 1/120 s ones its adaptive stepping gave a fast thrown part for a while in one session; and a thrown
//! part without spin that took half a frame more of gravity on its third frame in one session (0.12 studs by its 11th).

use std::rc::Rc;

use mlua::prelude::*;

use glam::{Affine3A, Mat3, Quat, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{frame, joints};
use rbx_dom_weak::types::Variant;

use crate::{ClassId, Engine, Hooks, InstanceId, NameId, Reflection, State, instance as core, reflection::EnumItemId, value::Value};

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
    assembly_linear: Option<NameId>,
    assembly_angular: Option<NameId>,
    rot_velocity: Option<NameId>,
    material: Option<NameId>,
    custom_physical: Option<NameId>,
    massless: Option<NameId>,
    shape: Option<NameId>,
    /// Each Material's density (PhysicalProperties.new(material), as measured: vendor lune-roblox physical_properties.rs)
    densities: FxHashMap<EnumItemId, f32>,
    ball: Option<EnumItemId>,
    cylinder: Option<EnumItemId>,
    wedge_shape: Option<EnumItemId>,
    corner_shape: Option<EnumItemId>,
    wedge_part: Option<ClassId>,
    corner_part: Option<ClassId>,
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
    /// Each flying part's spin (world, rad/s), its assembly's
    spin: FxHashMap<InstanceId, Vec3>,
    /// Impulses applied since the last step, by part: linear (ApplyImpulse) and angular
    impulses: FxHashMap<InstanceId, (Vec3, Vec3)>,
    /// Each part's own motion, as a client's physics body keeps it: the velocity of its position and the spin. A velocity write
    /// sets its assembly root's ([`assign`]), a simulation step each part's it moves ([`fly`]); none reads zero. An assembly reads its
    /// motion from its root's, so a part that leaves its root's assembly, or becomes its assembly's root, takes what its own body
    /// holds ([`assemblies_changed`])
    bodies: FxHashMap<InstanceId, (Vec3, Vec3)>,
    /// The parts whose velocities, spin or body are not all zero (world/joints.rs `motion` follows whether there is any)
    moving: FxHashSet<InstanceId>,
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
        assembly_linear: name("AssemblyLinearVelocity"),
        assembly_angular: name("AssemblyAngularVelocity"),
        rot_velocity: name("RotVelocity"),
        material: name("Material"),
        custom_physical: name("CustomPhysicalProperties"),
        massless: name("Massless"),
        shape: name("Shape"),
        densities: DENSITIES.iter().filter_map(|&(material, density)| reflection.enum_item("Material", material).map(|item| (item, density))).collect(),
        ball: reflection.enum_item("PartType", "Ball"),
        cylinder: reflection.enum_item("PartType", "Cylinder"),
        wedge_shape: reflection.enum_item("PartType", "Wedge"),
        corner_shape: reflection.enum_item("PartType", "CornerWedge"),
        wedge_part: class("WedgePart"),
        corner_part: class("CornerWedgePart"),
    });
    engine.state.borrow_mut().gravity.setup = Some(setup.clone());
    let body_setup = setup.clone();
    let mut hooks = engine.hooks.borrow_mut();
    let entered = setup.clone();
    let reflection_for_parent = engine.reflection.clone();
    hooks.hierarchy_changed.push(Box::new(move |state, id, old, new| {
        // (a tree that was in the Workspace's world already, once the first step listed what was there, holds nothing to list:
        // its loose parts were listed as they came, or as they were let go there, and a step drops only what left or was anchored)
        let stayed = state.gravity.scanned
            && old.is_some_and(|old| {
                let Some(instance) = state.dom.get(old) else { return false };
                let world = if reflection_for_parent.class(instance.class).is_a(entered.world_root) {
                    Some(old)
                } else {
                    super::world_root_of(state, &reflection_for_parent, entered.world_root, old)
                };
                world.is_some_and(|world| reflection_for_parent.class(state.dom.at(world).class).is_a(entered.workspace))
            });
        if new.is_some() && !stayed && in_workspace(state, &reflection_for_parent, &entered, id) {
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
        } else if !state.gravity.stepping && (name == setup.velocity || Some(name) == setup.assembly_linear) {
            assign(state, &reflection_for_property, &setup, id, name, false);
        } else if !state.gravity.stepping && (Some(name) == setup.assembly_angular || Some(name) == setup.rot_velocity) {
            assign(state, &reflection_for_property, &setup, id, name, true);
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
        gravity.spin.remove(&id);
        gravity.impulses.remove(&id);
        gravity.added.retain(|&focus| focus != id);
        gravity.bodies.remove(&id);
        if gravity.moving.remove(&id) {
            state.joints.motion = !state.gravity.moving.is_empty();
        }
    }));
    drop(hooks);
    install_body(engine, &body_setup);

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

/// Whether an instance is in the Workspace's own world (not a WorldModel's): its nearest WorldRoot, from the homes the dom
/// keeps (world/mod.rs world_root_of), is a Workspace. No walk up the tree, so each step asks it of every loose part.
fn in_workspace(state: &State, reflection: &Reflection, setup: &Setup, id: InstanceId) -> bool {
    super::world_root_of(state, reflection, setup.world_root, id).is_some_and(|world| reflection.class(state.dom.at(world).class).is_a(setup.workspace))
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
/// constraint or body mover (on itself or an attachment of it). `beside_humanoid` keeps, for one step, whether
/// each parent asked about holds a Humanoid, so a step looks through a parent's children once however many loose
/// parts it holds.
fn held(state: &State, reflection: &Reflection, setup: &Setup, part: InstanceId, beside_humanoid: &mut FxHashMap<InstanceId, bool>) -> bool {
    if state.dom.boolean(part, setup.can_collide) != Some(false) {
        return true;
    }
    if let (Some(humanoid), Some(parent)) = (setup.humanoid, state.dom.parent_of(part)) {
        if *beside_humanoid.entry(parent).or_insert_with(|| state.dom.find_child_of_class(parent, humanoid).is_some()) {
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
    let mut beside_humanoid: FxHashMap<InstanceId, bool> = FxHashMap::default();
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
        if component.iter().any(|&other| state.dom.boolean(other, setup.anchored) != Some(false) || held(&state, reflection, &setup, other, &mut beside_humanoid)) {
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
        fly(&mut state, reflection, &hooks, &setup, &component, part, g, dt);
    }
    if !gone.is_empty() {
        let gravity = &mut state.gravity;
        for part in gone {
            gravity.listed.remove(&part);
            gravity.velocity.remove(&part);
            gravity.bounded.remove(&part);
            gravity.spin.remove(&part);
        }
        let listed = &gravity.listed;
        gravity.loose.retain(|part| listed.contains(part));
    }
}

/// Each Material's density, as PhysicalProperties.new(material) reads it in a playing client (the table the datatype keeps:
/// vendor lune-roblox physical_properties.rs, Studio 0.744); a part's Mass is its volume times this (notes/macro bodyprobe.luau).
const DENSITIES: &[(&str, f32)] = &[
    ("Plastic", 0.7), ("SmoothPlastic", 0.7), ("Neon", 0.7), ("Wood", 0.35), ("WoodPlanks", 0.35), ("Marble", 2.563),
    ("Basalt", 2.691), ("Slate", 2.691), ("CrackedLava", 2.691), ("Concrete", 2.403), ("Limestone", 2.691), ("Granite", 2.691),
    ("Pavement", 2.691), ("Brick", 1.922), ("Pebble", 2.403), ("Cobblestone", 2.691), ("Rock", 2.691), ("Sandstone", 2.691),
    ("CorrodedMetal", 7.85), ("DiamondPlate", 7.85), ("Foil", 2.7), ("Metal", 7.85), ("Grass", 0.9), ("LeafyGrass", 0.9),
    ("Sand", 1.602), ("Fabric", 0.7), ("Snow", 0.9), ("Mud", 0.9), ("Ground", 0.9), ("Asphalt", 2.36), ("Salt", 2.165),
    ("Ice", 0.919), ("Glacier", 0.919), ("Glass", 2.403), ("ForceField", 2.403), ("Air", 0.01), ("Water", 1.0),
    ("Cardboard", 0.7), ("Carpet", 1.1), ("CeramicTiles", 2.4), ("ClayRoofTiles", 2.0), ("RoofShingles", 2.36),
    ("Leather", 0.86), ("Plaster", 0.75), ("Rubber", 1.3),
];

/// A free assembly's spin decays this much a second: 0.99913 a 60 Hz frame (notes/macro flyprobe.luau: a 2-stud cube spun at
/// 6 rad/s read 5.995, 5.990, 5.903 and 5.800 at its 2nd, 3rd, 20th and 40th frames).
const SPIN_DECAY: f32 = 0.05222;

/// Whether a part is Massless: it weighs nothing in its assembly unless it is the root (its own, alone), and outside every world it
/// weighs what it would ([`own_mass`]).
fn massless(state: &State, setup: &Setup, part: InstanceId) -> bool {
    setup.massless.is_some_and(|name| state.dom.boolean(part, name) == Some(true))
}

/// What the assembly reads take from a part (joints::Point): its position, its mass and whether it is Massless.
fn part_point(state: &State, setup: &Setup, part: InstanceId) -> joints::Point {
    (Vec3::from(frame::affine_of(state.dom.raw_get(part, setup.cframe)).translation), part_body(state, setup, part).0, massless(state, setup, part))
}

/// A part's Mass and GetMass(): its mass ([`part_body`]), or 0 when it is Massless in an assembly another part roots (a Massless
/// part alone, anchored, at the root of an all-Massless assembly or outside every world reads its mass: Studio 0.747, client and
/// server, tools/parity/probes/assembly_mass.luau).
fn own_mass(state: &mut State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> f32 {
    let mass = part_body(state, setup, part).0;
    if massless(state, setup, part) && joints::root_part(state, reflection, part).is_some_and(|root| root != part) { 0.0 } else { mass }
}

/// A part's mass, Massless or not ([`massless`] says when it counts), and its principal moments of inertia about its centre, in
/// its own frame: a box's from its Size, a sphere's for a Ball, a cylinder's (along X) for a Cylinder, half a box's volume for a
/// wedge and a third for a corner wedge (their moments a box's).
fn part_body(state: &State, setup: &Setup, part: InstanceId) -> (f32, Vec3) {
    let size = match setup.size.and_then(|name| state.dom.raw_get(part, name)) {
        Some(Value::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
        _ => Vec3::ONE,
    };
    let custom = setup.custom_physical.and_then(|name| match state.dom.raw_get(part, name) {
        Some(Value::Dom(variant)) => match &**variant {
            Variant::PhysicalProperties(rbx_dom_weak::types::PhysicalProperties::Custom(custom)) => Some(custom.density()),
            _ => None,
        },
        // (a script's PhysicalProperties.new(...) is kept as it was given)
        Some(Value::Lua(value)) => match &**value {
            LuaValue::UserData(data) => data
                .borrow::<lune_roblox::datatypes::types::PhysicalProperties>()
                .ok()
                .map(|properties| rbx_dom_weak::types::CustomPhysicalProperties::from(*properties).density()),
            _ => None,
        },
        _ => None,
    });
    let density = custom.unwrap_or_else(|| {
        let material = setup.material.and_then(|name| state.dom.raw_get(part, name)).and_then(|value| match value {
            Value::EnumItem(item) => Some(*item),
            _ => None,
        });
        material.and_then(|item| setup.densities.get(&item).copied()).unwrap_or(0.7)
    });
    let class = state.dom.at(part).class;
    let shape = setup.shape.and_then(|name| state.dom.raw_get(part, name)).and_then(|value| match value {
        Value::EnumItem(item) => Some(*item),
        _ => None,
    });
    let boxed = Vec3::new(size.y * size.y + size.z * size.z, size.x * size.x + size.z * size.z, size.x * size.x + size.y * size.y) / 12.0;
    let volume = size.x * size.y * size.z;
    let (volume, moments) = if shape.is_some() && shape == setup.ball {
        let r = size.min_element() * 0.5;
        (4.0 / 3.0 * std::f32::consts::PI * r * r * r, Vec3::splat(0.4 * r * r))
    } else if shape.is_some() && shape == setup.cylinder {
        let (r, h) = (size.y.min(size.z) * 0.5, size.x);
        let across = (3.0 * r * r + h * h) / 12.0;
        (std::f32::consts::PI * r * r * h, Vec3::new(0.5 * r * r, across, across))
    } else if (shape.is_some() && shape == setup.wedge_shape) || Some(class) == setup.wedge_part {
        (volume * 0.5, boxed)
    } else if (shape.is_some() && shape == setup.corner_shape) || Some(class) == setup.corner_part {
        (volume / 3.0, boxed)
    } else {
        (volume, boxed)
    };
    let mass = volume * density;
    (mass, moments * mass)
}

/// An assembly's mass, centre of mass and inertia about it (world axes).
struct Body {
    mass: f32,
    centre: Vec3,
    inertia: Mat3,
}

/// The body of an assembly's parts (`assembly`, its root first: joints::assembly_parts), each counted as the assembly reads count
/// it (a Massless part but the root weighs nothing), its centre worked out about the root as they work it out (joints::mass_centre).
fn body_of(state: &State, setup: &Setup, assembly: &[InstanceId]) -> Body {
    let parts: Vec<(Affine3A, f32, Vec3)> = assembly
        .iter()
        .enumerate()
        .map(|(index, &part)| {
            let (mass, moments) = if index > 0 && massless(state, setup, part) { (0.0, Vec3::ZERO) } else { part_body(state, setup, part) };
            (frame::affine_of(state.dom.raw_get(part, setup.cframe)), mass, moments)
        })
        .collect();
    let (mass, offset) = joints::mass_centre(parts.iter().map(|(at, part_mass, _)| (Vec3::from(at.translation), *part_mass)));
    let centre = parts.first().map_or(Vec3::ZERO, |(at, ..)| Vec3::from(at.translation)) + offset;
    let mut inertia = Mat3::ZERO;
    for (at, part_mass, moments) in &parts {
        let rotation = Mat3::from(at.matrix3);
        inertia += rotation * Mat3::from_diagonal(*moments) * rotation.transpose();
        let r = Vec3::from(at.translation) - centre;
        inertia += (Mat3::IDENTITY * r.length_squared() - Mat3::from_cols(r * r.x, r * r.y, r * r.z)) * *part_mass;
    }
    Body { mass, centre, inertia }
}

/// A script set a part's velocity or spin: its assembly (not the parts welded to it beyond an anchored part: joints::assembly_parts)
/// takes it, as a client keeps an assembly's motion, the velocity of its root's position and its spin (Studio 0.747, client and
/// server, tools/parity/probes/assembly_mass.luau): AssemblyLinearVelocity is the velocity of its centre of mass (the root's plus
/// the spin across the centre's offset from the root: joints::assembly_frame), the legacy Velocity each part's own position's, and
/// AssemblyAngularVelocity and RotVelocity the spin, on each of its parts, anchored or not. AssemblyLinearVelocity sets the
/// centre's velocity and keeps the spin; Velocity sets the root's, whichever part it is written to; AssemblyAngularVelocity sets
/// the spin and keeps the centre's velocity; RotVelocity sets the spin and keeps the root's. Each is worked out in floats as the
/// client works it (a velocity written and a spin after it read back 1.99999988 for 2), so the written name reads back what the
/// client's does, not always what was written. The motion is the root's body's ([`Gravity::bodies`]): the write sets the root's alone,
/// as a client's does (a part welded to the root and split off later reads its own body, not the write: [`assemblies_changed`]). A
/// flying assembly flies on with it.
fn assign(state: &mut State, reflection: &Reflection, setup: &Setup, id: InstanceId, name: NameId, angular: bool) {
    if state.dom.get(id).is_none() || !reflection.class(state.dom.at(id).class).is_a(setup.base_part) {
        return;
    }
    let Some(value) = state.dom.raw_get(id, name).and_then(frame::vector_of) else { return };
    let flying = state.gravity.velocity.contains_key(&id);
    // (a script's write sets the assembly: each other part of it costs a client 4.5 ns, cost.rs ASSEMBLY_VELOCITY_PART)
    if !state.quiet_write {
        let others = joints::assembly_size(state, reflection, id) - 1;
        if others > 0 {
            let work = others as f64 * crate::cost::ASSEMBLY_VELOCITY_PART;
            state.sched.charge(work);
            crate::boundary::add_cost(work);
        }
    }
    let (parts, offset) = joints::assembly_frame(state, reflection, id, |state, part| part_point(state, setup, part));
    let root = parts[0];
    let vector = |state: &State, part: InstanceId, name: Option<NameId>| name.and_then(|name| state.dom.raw_get(part, name)).and_then(frame::vector_of).unwrap_or(Vec3::ZERO);
    // (the motion before the write, from what it left: the root's Velocity is its position's, and of the two names of the spin the
    // one not written still holds it)
    let (at_root, spin) = if !angular {
        let spin = vector(state, id, setup.assembly_angular);
        (if name == setup.velocity { value } else { value - spin.cross(offset) }, spin)
    } else if Some(name) == setup.rot_velocity {
        (vector(state, root, Some(setup.velocity)), value)
    } else {
        // (the root's velocity moved by the change of spin across the centre's offset, which keeps the centre's: in that order, as a
        // client rounds it, 24 of 24 spin pairs where the other orders read a float step apart)
        let before = vector(state, id, setup.rot_velocity);
        (vector(state, root, Some(setup.velocity)) + (before - value).cross(offset), value)
    };
    set_body(state, root, at_root, spin);
    write_motion(state, setup, &parts, offset, flying);
}

/// Sets `part`'s body ([`Gravity::bodies`]).
fn set_body(state: &mut State, part: InstanceId, velocity: Vec3, spin: Vec3) {
    if velocity == Vec3::ZERO && spin == Vec3::ZERO {
        state.gravity.bodies.remove(&part);
    } else {
        state.gravity.bodies.insert(part, (velocity, spin));
    }
}

/// Each of `parts` (an assembly, its root first; `offset` its centre of mass's from the root) reads the motion of the root's body, as a
/// client reads an assembly's: AssemblyLinearVelocity the velocity of its centre of mass, the legacy Velocity each part's own
/// position's, AssemblyAngularVelocity and RotVelocity the spin, worked out in floats as a client works them. A flying assembly
/// (`flying`) flies on with it.
fn write_motion(state: &mut State, setup: &Setup, parts: &[InstanceId], offset: Vec3, flying: bool) {
    let root = parts[0];
    let (at_root, spin) = state.gravity.bodies.get(&root).copied().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let position = |state: &State, part: InstanceId| Vec3::from(frame::affine_of(state.dom.raw_get(part, setup.cframe)).translation);
    let linear = at_root + spin.cross(offset);
    let root_at = position(state, root);
    for &part in parts {
        let own = at_root + spin.cross(position(state, part) - root_at);
        for (target, written) in [(Some(setup.velocity), own), (setup.assembly_linear, linear), (setup.assembly_angular, spin), (setup.rot_velocity, spin)] {
            if let Some(target) = target {
                state.dom.exchange(part, target, frame::vector_value(written));
            }
        }
        state.dom.bump_version(part);
        if flying {
            state.gravity.velocity.insert(part, linear);
            if spin == Vec3::ZERO {
                state.gravity.spin.remove(&part);
            } else {
                state.gravity.spin.insert(part, spin);
            }
        }
        let still = own == Vec3::ZERO && linear == Vec3::ZERO && spin == Vec3::ZERO && !state.gravity.bodies.contains_key(&part);
        if still {
            state.gravity.moving.remove(&part);
        } else {
            state.gravity.moving.insert(part);
        }
    }
    state.joints.motion = !state.gravity.moving.is_empty();
}

/// A joint started a link: a client works out the motion of the joint's end on the side that keeps its root, and keeps it in the body
/// of each part from the root down to that end (`path`, the root first: world/joints.rs root_path), from the root's motion as it is now
/// (Studio 0.747, a playing client and its server: a chain welded on a part at a time to a moving part, then a part of it anchored or
/// split off, reads the motion the join left in it, and a part beyond that end none; a later write to the root leaves it as the join
/// left it; tools/parity/probes/assembly_motion.luau). A script's read of a part's Velocity or RotVelocity keeps its motion in its
/// body the same way on a client, which is not modelled (ree has no hook on a property's read).
pub(crate) fn hold_path(state: &mut State, path: &[InstanceId]) {
    let Some(setup) = state.gravity.setup.clone() else { return };
    let Some((&root, rest)) = path.split_first() else { return };
    let (at_root, spin) = state.gravity.bodies.get(&root).copied().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let position = |state: &State, part: InstanceId| Vec3::from(frame::affine_of(state.dom.raw_get(part, setup.cframe)).translation);
    let root_at = position(state, root);
    for &part in rest {
        let own = at_root + spin.cross(position(state, part) - root_at);
        set_body(state, part, own, spin);
        if state.gravity.bodies.contains_key(&part) {
            state.gravity.moving.insert(part);
        }
    }
    state.joints.motion = !state.gravity.moving.is_empty();
}

/// The assemblies a write joined, split or rooted elsewhere (world/joints.rs `regrouped`: the parts whose assembly it may have
/// changed) read their motion from their roots' bodies ([`write_motion`]), as a client's do (Studio 0.747, a playing client and its
/// server alike, tools/parity/probes/assembly_motion.luau): the parts a weld or an Anchored write joins to an assembly take its root's
/// velocity and spin, the joined centre's velocity with them, whichever assembly they came from; a piece split off takes what the body
/// of its new root holds (its point velocity and the spin when a simulation step or a velocity write as that piece's root set it,
/// else nothing: a piece split off in the frame a script set the velocity reads zero) and the piece that keeps the root keeps the
/// motion, its centre's velocity worked out again; a part that becomes its assembly's root (anchored, or ranked above the root) gives
/// the assembly its own body's motion. While no part moves, nothing is looked at.
pub(crate) fn assemblies_changed(state: &mut State, reflection: &Reflection) {
    if state.joints.regrouped.is_empty() {
        return;
    }
    let seeds = std::mem::take(&mut state.joints.regrouped);
    let Some(setup) = state.gravity.setup.clone() else { return };
    let mut done: FxHashSet<InstanceId> = FxHashSet::default();
    for &seed in &seeds {
        if state.gravity.moving.is_empty() {
            break;
        }
        if state.dom.get(seed).is_none() || !reflection.class(state.dom.at(seed).class).is_a(setup.base_part) || super::world_root_of(state, reflection, setup.world_root, seed).is_none() {
            continue;
        }
        let (parts, offset) = joints::assembly_offset(state, reflection, seed, |state, part| part_point(state, &setup, part));
        if !done.insert(parts[0]) {
            continue;
        }
        // (an assembly that does not move and holds no part that moved has nothing to change)
        let gravity = &state.gravity;
        let still = !gravity.bodies.contains_key(&parts[0])
            && if gravity.moving.len() < parts.len() {
                let set: FxHashSet<InstanceId> = parts.iter().copied().collect();
                !gravity.moving.iter().any(|part| set.contains(part))
            } else {
                !parts.iter().any(|part| gravity.moving.contains(part))
            };
        if still {
            continue;
        }
        let flying = parts.iter().any(|part| state.gravity.velocity.contains_key(part));
        write_motion(state, &setup, &parts, offset, flying);
    }
    let mut seeds = seeds;
    seeds.clear();
    if state.joints.regrouped.is_empty() {
        state.joints.regrouped = seeds;
    }
}

/// One simulation step of a free assembly (`part` one of `component`'s): the velocity and spin it has (a first step reads the
/// ones a script set), plus the impulses since the last step, then four steps of 1/240 s on its first frame and one of 1/60 s
/// after (each: velocity, then the move by it), the spin turning it about its centre of mass as a free rigid body.
#[allow(clippy::too_many_arguments)]
fn fly(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, component: &[InstanceId], part: InstanceId, g: Vec3, dt: f32) {
    let first = !state.gravity.velocity.contains_key(&part);
    let read = |state: &State, name: Option<NameId>| name.and_then(|name| state.dom.raw_get(part, name)).and_then(frame::vector_of);
    let (mut linear, spin) = if first {
        let linear = read(state, setup.assembly_linear).filter(|v| *v != Vec3::ZERO).or_else(|| read(state, Some(setup.velocity))).unwrap_or(Vec3::ZERO);
        let spin = read(state, setup.assembly_angular).filter(|v| *v != Vec3::ZERO).or_else(|| read(state, setup.rot_velocity)).unwrap_or(Vec3::ZERO);
        (linear, spin)
    } else {
        (state.gravity.velocity[&part], state.gravity.spin.get(&part).copied().unwrap_or(Vec3::ZERO))
    };
    let (mut push, mut twist) = (Vec3::ZERO, Vec3::ZERO);
    for other in component {
        if let Some((pushed, twisted)) = state.gravity.impulses.remove(other) {
            push += pushed;
            twist += twisted;
        }
    }
    let wants_spin = spin != Vec3::ZERO || twist != Vec3::ZERO;
    let body = (push != Vec3::ZERO || wants_spin).then(|| {
        let assembly = joints::assembly_parts(state, reflection, part);
        body_of(state, setup, &assembly)
    });
    if let Some(body) = &body {
        if push != Vec3::ZERO && body.mass > 0.0 {
            linear += push / body.mass;
        }
    }
    // (an assembly with no inertia to turn, all of it Massless, does not spin)
    let turning = body.as_ref().filter(|body| wants_spin && body.inertia.determinant().abs() > 1e-12);
    let inverse = turning.map(|body| body.inertia.inverse()).unwrap_or(Mat3::ZERO);
    let mut momentum = turning.map(|body| body.inertia * spin + twist).unwrap_or(Vec3::ZERO);
    let steps = if first { 4 } else { 1 };
    let h = dt / steps as f32;
    let mut moved = Vec3::ZERO;
    let mut turn = Quat::IDENTITY;
    let spin_at = |turn: Quat, momentum: Vec3| {
        let rotation = Mat3::from_quat(turn);
        rotation * (inverse * (rotation.transpose() * momentum))
    };
    for _ in 0..steps {
        linear += g * h;
        moved += linear * h;
        if turning.is_some() {
            momentum *= (-SPIN_DECAY * h).exp();
            let omega = spin_at(turn, momentum);
            let angle = omega.length() * h;
            if angle > 0.0 {
                turn = (Quat::from_axis_angle(omega / omega.length(), angle) * turn).normalize();
            }
        }
    }
    let omega = if turning.is_some() { spin_at(turn, momentum) } else { Vec3::ZERO };
    let centre = turning.map(|body| body.centre);
    state.gravity.stepping = true;
    for &other in component {
        let cframe = frame::affine_of(state.dom.raw_get(other, setup.cframe));
        let (target, velocity) = match centre {
            Some(centre) => {
                let target = Affine3A::from_translation(centre + moved) * Affine3A::from_quat(turn) * Affine3A::from_translation(-centre) * cframe;
                let r = Vec3::from(target.translation) - (centre + moved);
                (target, linear + omega.cross(r))
            }
            None => (Affine3A::from_translation(moved) * cframe, linear),
        };
        core::raw_set_unsignalled(state, reflection, hooks, other, setup.cframe, frame::cframe_value(&target));
        core::raw_set_unsignalled(state, reflection, hooks, other, setup.velocity, frame::vector_value(velocity));
        if let Some(name) = setup.assembly_linear {
            core::raw_set_unsignalled(state, reflection, hooks, other, name, frame::vector_value(linear));
        }
        for name in [setup.assembly_angular, setup.rot_velocity].into_iter().flatten() {
            core::raw_set_unsignalled(state, reflection, hooks, other, name, frame::vector_value(omega));
        }
        state.gravity.velocity.insert(other, linear);
        if omega != Vec3::ZERO {
            state.gravity.spin.insert(other, omega);
        } else {
            state.gravity.spin.remove(&other);
        }
        set_body(state, other, velocity, omega);
        state.gravity.moving.insert(other);
    }
    state.joints.motion = true;
    state.gravity.stepping = false;
    // (the centres of mass read since the last step, worked out again where the step left the parts)
    joints::moved(state, part);
}

/// BasePart's mass and impulses: Mass, AssemblyMass, AssemblyCenterOfMass and GetMass() read the masses above; ApplyImpulse,
/// ApplyImpulseAtPosition and ApplyAngularImpulse act at an unanchored part's next physics step (read back after it), as measured.
fn install_body(engine: &Rc<Engine>, setup: &Rc<Setup>) {
    let reflection = &engine.reflection;
    let Some(base_part) = reflection.class_id("BasePart") else { return };
    let name = |text: &str| reflection.name_id(text);
    let mut behaviors = engine.behaviors.borrow_mut();
    let number = |value: f32| Ok(LuaValue::Number(value as f64));
    if let Some(mass) = name("Mass") {
        let setup = setup.clone();
        behaviors.add_getter(base_part, mass, crate::services::getter(move |_, engine, id| number(own_mass(&mut engine.state.borrow_mut(), &engine.reflection, &setup, id))));
    }
    if let Some(get_mass) = name("GetMass") {
        let setup = setup.clone();
        behaviors.add_method(base_part, get_mass, crate::services::method(move |_, engine, id, _| {
            Ok(crate::lua::values([LuaValue::Number(own_mass(&mut engine.state.borrow_mut(), &engine.reflection, &setup, id) as f64)]))
        }));
    }
    // (the assembly's mass and centre, kept with its component while its parts stay where they are: joints::assembly_body)
    let assembly = |engine: &Rc<Engine>, setup: &Setup, id: InstanceId, centre: bool| {
        let mut state = engine.state.borrow_mut();
        let (mass, centre) = joints::assembly_body(&mut state, &engine.reflection, id, centre, |state, part| part_point(state, setup, part));
        Body { mass, centre, inertia: Mat3::ZERO }
    };
    if let Some(assembly_mass) = name("AssemblyMass") {
        let setup = setup.clone();
        behaviors.add_getter(base_part, assembly_mass, crate::services::getter(move |_, engine, id| number(assembly(engine, &setup, id, false).mass)));
    }
    if let Some(centre_of_mass) = name("AssemblyCenterOfMass") {
        let setup = setup.clone();
        behaviors.add_getter(base_part, centre_of_mass, crate::services::getter(move |lua, engine, id| {
            let centre = assembly(engine, &setup, id, true).centre;
            Ok(crate::lua::convert::to_lua(lua, engine, &frame::vector_value(centre))?)
        }));
    }
    // AngularAccelerationToTorque(acceleration, spin = 0) and TorqueToAngularAcceleration(torque, spin = 0): through the assembly's
    // inertia about its centre of mass, I a + w x (I w) and its inverse; an anchored assembly, or a part outside every world, answers
    // inf and 0 (Studio 0.747, a playing client and its server: a lone block, a welded pair turned, a Ball, a Massless part alone;
    // tools/parity/probes/part_methods.luau)
    for (method_name, to_torque) in [("AngularAccelerationToTorque", true), ("TorqueToAngularAcceleration", false)] {
        let Some(method_id) = name(method_name) else { continue };
        let setup = setup.clone();
        behaviors.add_method(base_part, method_id, crate::services::method(move |lua, engine, id, args| {
            let given = super::parts::vector_argument(engine, args, 1)?;
            let spin = if args.get(2).is_nil() { Vec3::ZERO } else { super::parts::vector_argument(engine, args, 2)? };
            let inertia = {
                let mut state = engine.state.borrow_mut();
                let root = joints::root_part(&mut state, &engine.reflection, id);
                match root {
                    Some(root) if state.dom.boolean(root, setup.anchored) != Some(true) => {
                        let parts = joints::assembly_parts(&mut state, &engine.reflection, id);
                        Some(body_of(&state, &setup, &parts).inertia)
                    }
                    _ => None,
                }
            };
            let answer = match inertia {
                Some(inertia) if to_torque => inertia * given + spin.cross(inertia * spin),
                Some(inertia) => inertia.inverse() * (given - spin.cross(inertia * spin)),
                None if to_torque => Vec3::splat(f32::INFINITY),
                None => Vec3::ZERO,
            };
            Ok(crate::lua::values([crate::lua::convert::to_lua(lua, engine, &frame::vector_value(answer))?]))
        }));
    }
    // GetVelocityAtPosition(point): the assembly's velocity at the point, its root's velocity (the root's Velocity) and its spin across
    // the point's offset from the root, as a client keeps an assembly's motion (measured as above)
    if let Some(method_id) = name("GetVelocityAtPosition") {
        let setup = setup.clone();
        behaviors.add_method(base_part, method_id, crate::services::method(move |lua, engine, id, args| {
            let point = super::parts::vector_argument(engine, args, 1)?;
            let read = |text: &str, id: InstanceId| -> Result<Vec3, crate::lua::Flow> {
                let Some(property) = engine.reflection.name_id(text) else { return Ok(Vec3::ZERO) };
                let class = engine.state.borrow().dom.at(id).class;
                let getter = engine.behaviors.borrow().find_getter(&engine.reflection, class, property).cloned();
                // (a getter's answer, else the stored value scripts read)
                Ok(match getter {
                    Some(getter) => super::vector_arg(&getter(lua, engine, id)?).unwrap_or(Vec3::ZERO),
                    None => engine.state.borrow().dom.raw_get(id, property).and_then(frame::vector_of).unwrap_or(Vec3::ZERO),
                })
            };
            let root = joints::root_part(&mut engine.state.borrow_mut(), &engine.reflection, id).unwrap_or(id);
            let (velocity, spin) = (read("Velocity", root)?, read("AssemblyAngularVelocity", root)?);
            let at = Vec3::from(frame::affine_of(engine.state.borrow().dom.raw_get(root, setup.cframe)).translation);
            let answer = velocity + spin.cross(point - at);
            Ok(crate::lua::values([crate::lua::convert::to_lua(lua, engine, &frame::vector_value(answer))?]))
        }));
    }
    let vector = |args: &crate::lua::Args, index: usize| frame::vector_of(&crate::lua::convert::from_lua(args.get(index)));
    for (method, kind) in [("ApplyImpulse", 0u8), ("ApplyAngularImpulse", 1), ("ApplyImpulseAtPosition", 2)] {
        let Some(method_name) = name(method) else { continue };
        let setup = setup.clone();
        behaviors.add_method(base_part, method_name, crate::services::method(move |_, engine, id, args| {
            let Some(impulse) = vector(args, 1) else { return crate::lua::raise("Argument 1 missing or nil") };
            let at = if kind == 2 {
                let Some(at) = vector(args, 2) else { return crate::lua::raise("Argument 2 missing or nil") };
                Some(at)
            } else {
                None
            };
            let centre = at.map(|_| assembly(engine, &setup, id, true).centre);
            let mut state = engine.state.borrow_mut();
            if state.dom.boolean(id, setup.anchored) == Some(false) {
                let entry = state.gravity.impulses.entry(id).or_default();
                match (kind, at, centre) {
                    (0, ..) => entry.0 += impulse,
                    (1, ..) => entry.1 += impulse,
                    (_, Some(at), Some(centre)) => {
                        entry.0 += impulse;
                        entry.1 += (at - centre).cross(impulse);
                    }
                    _ => {}
                }
            }
            Ok(crate::lua::none())
        }));
    }
}
