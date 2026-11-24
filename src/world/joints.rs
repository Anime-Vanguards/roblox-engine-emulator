//! Rigid assemblies: parts joined by rigid joints move as one, as Roblox's kinematics move them.
//! The rigid joints are the JointInstances Weld, Snap, ManualWeld, Motor and Motor6D (Part1 =
//! Part0 * C0 * Transform * C1:Inverse(), Transform being a Motor6D's) and WeldConstraint (the
//! parts' relative CFrame when it was enabled, kept in its CFrame0 with State's 2 bit set, as the
//! place files Studio writes hold it).
//!
//! Rules (Roblox's documentation, BasePart / JointInstance / WeldConstraint / Motor6D and the
//! Assemblies guide; the root measured in a Studio 0.746 playing client, the rest not measured yet):
//! - A joint is active while it is Enabled, names two different parts that are in the same
//!   WorldRoot (the Workspace or a WorldModel) and is itself in that WorldRoot or in JointsService.
//!   Active joints link parts into assemblies; each assembly is a tree hanging from its root part.
//! - The root (BasePart.AssemblyRootPart, nil outside a WorldRoot): an anchored part (each anchored
//!   part roots its own assembly: a joint between two anchored trees, or one closing a cycle, does
//!   nothing), else a part that is not Massless, then the higher RootPriority, then the larger
//!   largest face area (the product of its two largest Size components, not rounded: 16.0004 beats
//!   16) times 20 for a Seat or VehicleSeat, 10 for a part named exactly HumanoidRootPart and 5 for
//!   one named exactly Torso (the RootPriority announcement's weights), then the later made part.
//! - Setting a part's CFrame moves its whole assembly with it, now (BasePart.CFrame): the root moves
//!   as the part moved, rigidly (its rotation stored as a script's write to it would be:
//!   frame::stored_rotation), and the rest follows its joints from the root. The joints never
//!   place the root, so a Motor6D.Transform set since the parts were last placed bends the limbs it
//!   drives and leaves the root (an anchored one above all) where the move puts it. The other
//!   parts' CFrames change without Changed events, as physics moves parts; engine hooks (the
//!   recorder) see them.
//! - Setting a part's Position, Orientation or Rotation moves that part alone: the C0 or C1 of each
//!   Weld or Motor6D joined to it is rewritten to keep it there, and each WeldConstraint joined to
//!   it takes the new relative CFrame (BasePart.Position).
//! - When a joint becomes active, or its C0 / C1 / Part0 / Part1 / Enabled change, or a part of it
//!   is anchored or unanchored, the root stays and the rest of its assembly moves to where the
//!   joints put it (JointInstance.C0). A WeldConstraint takes its parts' relative CFrame when
//!   Part0, Part1 or Enabled is set (it links them where they are) and, when it never had one, when
//!   it becomes active.
//! - Motor6D.Transform is applied in a batch after RunService.PreSimulation (Motor6D.Transform);
//!   in edit mode, where the simulation events never fire, at the start of each Heartbeat, before
//!   its resumptions (where play mode's simulation step runs; not documented).
//! - Model:PivotTo and Model:ScaleTo move many parts at once ([`begin_batch`] / [`end_batch`]):
//!   each assembly they touched settles at the end from its root, if they wrote it (a write of the
//!   value it already had counts), else from its root moved as the first part of it they wrote moved.
//! - A part a resolution finds where its joints already put it (within float noise) keeps its
//!   stored CFrame, so loading a consistent model changes nothing.
//!
//! Which world a part or a joint is in is read from the homes the dom keeps (dom.rs `Dom::home`: each
//! instance's nearest WorldRoot or JointsService ancestor, kept as the tree changes), never by walking up
//! the tree: a lookup costs a few lookups for each joint it meets, however deep its parts and joints are.
//! A playing client's assembly reads cost the same at any depth and joint count (Studio 0.747, a
//! headless client: an AssemblyRootPart read 0.04 us, AssemblyMass 0.03 and AssemblyCenterOfMass 0.03,
//! with 1 to 1,000 joints to parts 0 to 1,000 folders deep), and the work clock charges them its flat
//! read price.
//!
//! The index lists each part's joints in the order they were indexed under it, and each joint keeps its
//! place in its parts' lists ([`PartJoints`]): making, re-pointing or taking out a joint costs the same
//! however many joints name its parts (each used to search the part's whole list: 5 to 8.5 us a write
//! at 40,000 joints), and the joints a walk takes up come in the same order as before. Each list marks
//! which of its joints are in a world or JointsService, as the dom logs their homes (dom.rs
//! Dom::log_homes, taken before each walk: [`take_homes`]), and the walks look at those alone, as a
//! playing client does (Studio 0.747, headless: a part named by 10,000 welds outside every world
//! enters a world as fast as one named by none; each weld in a world that cannot link costs it 0.04
//! to 0.09 us). A Parent write that takes parts or joints into a world pays for its joints (cost.rs
//! joints_entering): each joint its walks look at, each link it makes and each joint it brings in.
//!
//! Each list also marks which of its joints link the part now ([`relink`]: worked out again when a joint
//! is made, re-pointed, enabled or disabled, when its home or a part's world or class changes and when a
//! part is freed), and the walks take up those alone: a part's joints that cannot link (Part1 nil,
//! disabled, the other part in another world) cost its writes and reads nothing, as on a client. What a
//! walk finds is kept ([`Component`], for each of its parts) until something it depends on changes: a
//! link among its parts, or a part's Anchored, Massless, RootPriority, Size or Name ([`stored`], from
//! every write); so AssemblyRootPart, AssemblyMass and AssemblyCenterOfMass reads, and moves, walk no
//! joint while nothing changed, as a client keeps its assemblies resolved (Studio 0.747, headless: those
//! reads 0.03 to 0.08 us at 100 to 10,000 welds, linking or not). What a client does pay for is charged
//! (cost.rs, 2026-11-24): a CFrame write, PivotTo or BulkMoveTo each part it carries and each joint
//! between the moved assembly and another (BulkMoveTo not), a Position / Orientation / Rotation write
//! each joint it rewrites and, for each inside the part's assembly, each part of it, an Anchored write
//! each other part whose root or the root's anchoring it changes and each joint linking the part, a
//! velocity write each other part of the assembly. A change of the links (a weld enabled, a part
//! anchored) in a large assembly still walks it again (a client's costs it 1.7 us at any size).
//!
//! Not modelled: Motor.CurrentAngle / DesiredAngle (the legacy motors' rotation), AnimationConstraint,
//! RigidConstraint and the other constraints (solver-driven, nothing is simulated), and the members
//! that report assemblies other than AssemblyRootPart (GetRootPart, GetConnectedParts,
//! JointInstance.Active).

use std::{collections::VecDeque, rc::Rc};

use glam::Affine3A;
use rustc_hash::{FxHashMap, FxHashSet};

use super::frame;
use crate::{ClassId, Engine, Hooks, InstanceId, NameId, Reflection, State, instance as core, value::Value};

/// What a class is to the assembly code.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Other,
    /// A rigid JointInstance: C0, C1 (and a Motor6D's Transform)
    Joint,
    WeldConstraint,
}

/// Names and classes, resolved once at install.
struct Setup {
    cframe: NameId,
    c0: NameId,
    c1: NameId,
    part0: NameId,
    part1: NameId,
    enabled: NameId,
    transform: NameId,
    anchored: NameId,
    massless: NameId,
    root_priority: NameId,
    size: NameId,
    cframe0: NameId,
    weld_state: NameId,
    base_part: ClassId,
    motor6d: Option<ClassId>,
    world_root: ClassId,
    joints_service: Option<ClassId>,
    seats: Vec<ClassId>,
    /// Kind by class id
    kinds: Vec<Kind>,
}

impl Setup {
    fn kind(&self, class: ClassId) -> Kind {
        self.kinds.get(class as usize).copied().unwrap_or(Kind::Other)
    }
}

/// The assembly state of one engine (in [`State`]).
#[derive(Default)]
pub struct Joints {
    setup: Option<Rc<Setup>>,
    /// Rigid joints made in this engine: none, and every entry point returns at once
    created: u32,
    /// Whether the joints made before the index existed have been indexed
    scanned: bool,
    /// Rigid joints by the parts they named when last indexed (candidates: a joint's current Part0
    /// and Part1 are checked when it is used), each part's in the order they were indexed under it
    by_part: FxHashMap<InstanceId, PartJoints>,
    /// Each indexed joint's parts and its place in each one's list
    indexed: FxHashMap<InstanceId, Indexed>,
    /// How many times the dom's log of homes was taken ([`take_homes`])
    takes: u64,
    /// Indexed joints whose links must be worked out again before the next walk ([`relink`]): their homes or their parts' changed,
    /// or a part they name was freed
    relinks: Vec<InstanceId>,
    /// The components found, by each of their parts ([`component`]), kept until something they depend on changes ([`stored`],
    /// [`drop_component`])
    components: FxHashMap<InstanceId, Rc<Component>>,
    /// What a property written to a part means to its kept component ([`stored`]), by name id: [`Watch`]
    watch: Vec<u8>,
    /// The component an Anchored write took out of the kept ones, with the part ([`stored`], [`reanchored`])
    before_anchor: Option<(InstanceId, Option<Rc<Assemblies>>)>,
    /// While positive, the parts' CFrame writes are BulkMoveTo's, which pay nothing for the joints between assemblies ([`bulk_moves`])
    bulk: u32,
    /// Motor6Ds whose Transform changed since the last simulation step
    pending: Vec<InstanceId>,
    /// While positive, CFrame and joint writes are the assembly code's own (already consistent)
    quiet: u32,
    /// Model:PivotTo / ScaleTo: nesting depth, the parts they wrote (in order, with the CFrame each
    /// had before) and the parts of the joints they changed
    batch: u32,
    batch_parts: Vec<(InstanceId, Affine3A)>,
    batch_seeds: Vec<InstanceId>,
}

/// The rigid joints indexed under one part, in the order they were indexed under it. A joint taken out leaves a hole at its
/// place (kept with the joint: [`Indexed`]), so the others keep their order and indexing or taking out a joint costs the same
/// however many joints name the part; the holes close up once they outnumber the joints. A bit for each place says whether
/// its joint is in a world or JointsService (has a home: dom.rs Dom::home), and the walks look at those alone, in order, as a
/// client looks only at a part's joints that are in one: 4,096 joints outside every world cost a walk one look.
#[derive(Default)]
struct PartJoints {
    slots: Vec<Option<InstanceId>>,
    /// The places whose joint has a home, in blocks of 4,096: a word saying which of the block's words hold any, then its words,
    /// 64 places a word ([`BLOCK`])
    bits: Vec<u64>,
    /// The places whose joint links the part to another now (the checks [`link_in`] makes, worked out again whenever the joint,
    /// its parts or their worlds change: [`relink`]), in the same blocks: the walks take up these alone, so a part's joints that
    /// cannot link (disabled, Part1 nil, its other part in another world) cost a walk nothing
    links: Vec<u64>,
    /// How many places are marked as in a world, and as linking
    homed: usize,
    linking: usize,
    live: usize,
}

/// The words of a block of [`PartJoints::bits`]: its summary and its 64 words
const BLOCK: usize = 65;

/// The word of [`PartJoints::bits`] that holds `place`'s bit, and its block's summary word.
fn word_of(place: usize) -> (usize, usize) {
    let block = place / 4096 * BLOCK;
    (block + 1 + place % 4096 / 64, block)
}

/// The places of the bits set in `bits`, from `base`, lowest first.
fn ones(bits: u64, base: usize) -> impl Iterator<Item = usize> {
    let mut bits = bits;
    std::iter::from_fn(move || {
        if bits == 0 {
            return None;
        }
        let bit = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        Some(base + bit)
    })
}

/// Sets or clears `place`'s bit in `bits` (blocks of [`BLOCK`] words), keeping its block's summary word; whether it changed.
fn put_bit(bits: &mut [u64], place: usize, on: bool) -> bool {
    let ((word, summary), bit) = (word_of(place), 1u64 << (place % 64));
    let was = bits[word] & bit != 0;
    if was == on {
        return false;
    }
    if on {
        bits[word] |= bit;
    } else {
        bits[word] &= !bit;
    }
    let bit = 1u64 << (place % 4096 / 64);
    if bits[word] != 0 {
        bits[summary] |= bit;
    } else {
        bits[summary] &= !bit;
    }
    true
}

/// The places whose bit is set in `bits` (blocks of [`BLOCK`] words), lowest first.
fn places_of(bits: &[u64]) -> impl Iterator<Item = usize> + '_ {
    (0..bits.len().div_ceil(BLOCK))
        .flat_map(move |block| ones(bits[block * BLOCK], 0).map(move |word| (block, word)))
        .flat_map(move |(block, word)| ones(bits[block * BLOCK + 1 + word], block * 4096 + word * 64))
}

impl PartJoints {
    /// Puts `joint` after the others, not linking yet; its place.
    fn push(&mut self, joint: InstanceId, homed: bool) -> u32 {
        let place = self.slots.len();
        self.slots.push(Some(joint));
        if place.is_multiple_of(4096) {
            self.bits.push(0);
            self.links.push(0);
        }
        if place.is_multiple_of(64) {
            self.bits.push(0);
            self.links.push(0);
        }
        self.live += 1;
        self.mark(place, homed);
        place as u32
    }

    /// Marks whether the joint at `place` has a home.
    fn mark(&mut self, place: usize, homed: bool) {
        if put_bit(&mut self.bits, place, homed) {
            if homed {
                self.homed += 1;
            } else {
                self.homed -= 1;
            }
        }
    }

    /// Marks whether the joint at `place` links the part now; whether that changed.
    fn link(&mut self, place: usize, on: bool) -> bool {
        let changed = put_bit(&mut self.links, place, on);
        if changed {
            if on {
                self.linking += 1;
            } else {
                self.linking -= 1;
            }
        }
        changed
    }

    /// Whether the joint at `place` has a home.
    fn is_homed(&self, place: usize) -> bool {
        self.bits[word_of(place).0] >> (place % 64) & 1 != 0
    }

    /// Whether the joint at `place` links the part.
    fn is_linking(&self, place: usize) -> bool {
        self.links[word_of(place).0] >> (place % 64) & 1 != 0
    }

    /// Takes the joint at `place` out, leaving a hole; whether it linked the part.
    fn remove_at(&mut self, place: usize) -> bool {
        self.slots[place] = None;
        self.mark(place, false);
        self.live -= 1;
        self.link(place, false)
    }

    /// The joints, in the order they were indexed
    #[cfg(test)]
    fn iter(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.slots.iter().flatten().copied()
    }

    /// The joints that have a home, in the order they were indexed
    fn homed(&self) -> impl Iterator<Item = InstanceId> + '_ {
        places_of(&self.bits).filter_map(|place| self.slots[place])
    }

    /// The joints that link the part now, in the order they were indexed
    fn linking(&self) -> impl Iterator<Item = InstanceId> + '_ {
        places_of(&self.links).filter_map(|place| self.slots[place])
    }

    /// Closes the holes, keeping the joints' order, and gives each joint its new place under `part`.
    fn close_holes(&mut self, part: InstanceId, indexed: &mut FxHashMap<InstanceId, Indexed>) {
        let old = std::mem::take(self);
        for (place, slot) in old.slots.iter().enumerate() {
            let Some(joint) = *slot else { continue };
            let moved = self.push(joint, old.is_homed(place));
            self.link(moved as usize, old.is_linking(place));
            if let Some(entry) = indexed.get_mut(&joint) {
                for side in 0..2 {
                    if entry.parts[side] == Some(part) {
                        entry.places[side] = moved;
                    }
                }
            }
        }
    }
}

/// Where a joint is indexed: the parts it named when last indexed and its place in each one's list (a joint naming one part
/// as both is listed once, at the first's place).
#[derive(Clone, Copy)]
struct Indexed {
    parts: [Option<InstanceId>; 2],
    places: [u32; 2],
    /// The last take of the dom's log of homes that marked it ([`take_homes`]: a joint the log names twice is marked once)
    taken: u64,
}

/// One assembly: its parts as a tree, the root first, each after its parent.
struct Assembly {
    nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
struct Node {
    part: InstanceId,
    /// Index of the parent node and the joint to it (none for the root)
    parent: Option<(usize, InstanceId)>,
}

/// The parts linked to each other now, as a walk from one of them found them ([`component`]), kept for each of its parts until
/// something it depends on changes: a link among them (a joint's parts, enabling or home, a part's world or class, one freed:
/// [`relink`]), a part's Anchored, Massless, RootPriority, Size or Name ([`stored`]). So reading an assembly, or moving it, walks
/// no joint while nothing changed, as a playing client keeps its assemblies (a read costs it the same at any joint count).
struct Component {
    /// Every part, in the order the walk found them
    parts: Vec<InstanceId>,
    /// Each part's joints that link it to another part, with that part, in the order its list holds them ([`edges`]' answer)
    links: FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>>,
    /// Its assemblies, once worked out ([`assemblies_of`])
    tree: std::cell::RefCell<Option<Rc<Assemblies>>>,
    /// What was worked out from its parts ([`Memo`])
    memo: std::cell::RefCell<Memo>,
}

/// A component's assemblies (several when more than one part is anchored), the same from any of its parts.
struct Assemblies {
    list: Vec<Assembly>,
    /// Each part's assembly and node in it
    place: FxHashMap<InstanceId, (u32, u32)>,
    /// Each assembly's joints to another assembly ([`crate::cost::JOINT_ACROSS`])
    across: Vec<u32>,
}

impl Assemblies {
    /// The assembly `part` is in and its node there
    fn of(&self, part: InstanceId) -> Option<(&Assembly, usize, usize)> {
        let &(assembly, node) = self.place.get(&part)?;
        Some((&self.list[assembly as usize], assembly as usize, node as usize))
    }

    /// The root `part` hangs from and whether that root is anchored now (`flipped`: the part whose Anchored reads the other way)
    fn root_of(&self, state: &State, setup: &Setup, part: InstanceId, flipped: Option<InstanceId>) -> Option<(InstanceId, bool)> {
        let (assembly, ..) = self.of(part)?;
        let root = assembly.nodes[0].part;
        Some((root, anchored(state, setup, root) != (flipped == Some(root))))
    }
}

/// What the gravity step and the assembly reads work out from a component's parts, from each part asked: linked_parts' answer, and
/// AssemblyMass and AssemblyCenterOfMass (world/gravity.rs body_of over it). The masses are forgotten when a part's Size, Massless,
/// Material, CustomPhysicalProperties or Shape changes, the centres also when a part moves ([`stored`]).
#[derive(Default)]
struct Memo {
    linked: FxHashMap<InstanceId, Rc<[InstanceId]>>,
    mass: FxHashMap<InstanceId, f32>,
    centre: FxHashMap<InstanceId, glam::Vec3>,
}

/// What a property written to a part means to its kept component ([`Joints::watch`])
mod watch {
    /// Its parts moved: the centres of mass go
    pub const PLACE: u8 = 1;
    /// Their mass changed: the masses and centres go
    pub const MASS: u8 = 2;
    /// Its assemblies' roots may change: the component goes
    pub const ROOTS: u8 = 3;
    /// Anchored: the component goes, kept aside for the write's price ([`super::reanchored`])
    pub const ANCHORED: u8 = 4;
}

/// Which part keeps its CFrame when an assembly settles.
#[derive(Clone, Copy)]
enum Anchor {
    Root,
    /// The node at this index was moved from this CFrame: it stays where it is and the root moves
    /// as it moved
    Moved(usize, Affine3A),
}

/// How far a resolution lets a part be from where its joints put it before rewriting it: float
/// noise of composing joint CFrames (single precision: a few ulps of the part's distance from the
/// origin, 1e-4 studs at least).
fn settled(current: &Affine3A, target: &Affine3A) -> bool {
    let reach = current.translation.abs().max_element();
    let position = (current.translation - target.translation).abs().max_element();
    let rotation = [
        current.matrix3.x_axis - target.matrix3.x_axis,
        current.matrix3.y_axis - target.matrix3.y_axis,
        current.matrix3.z_axis - target.matrix3.z_axis,
    ]
    .iter()
    .map(|axis| axis.abs().max_element())
    .fold(0.0f32, f32::max);
    position <= (2e-6 * reach).max(1e-4) && rotation <= 1e-5
}

/// The inverse of a rigid transform (rotation transposed, as Roblox inverts CFrames).
fn inverse(transform: &Affine3A) -> Affine3A {
    let rotation = transform.matrix3.transpose();
    Affine3A { matrix3: rotation, translation: -(rotation * transform.translation) }
}

/// Resolves the names and classes and registers the hooks that drop freed instances.
pub fn install(engine: &Rc<Engine>) {
    let reflection = &engine.reflection;
    let name = |text: &str| reflection.name_id(text);
    let class = |text: &str| reflection.class_id(text);
    let (
        Some(cframe), Some(c0), Some(c1), Some(part0), Some(part1), Some(enabled), Some(transform), Some(anchored),
        Some(massless), Some(root_priority), Some(size), Some(cframe0), Some(weld_state),
        Some(base_part), Some(world_root), Some(weld_constraint),
    ) = (
        name("CFrame"), name("C0"), name("C1"), name("Part0"), name("Part1"), name("Enabled"), name("Transform"), name("Anchored"),
        name("Massless"), name("RootPriority"), name("Size"), name("CFrame0"), name("State"),
        class("BasePart"), class("WorldRoot"), class("WeldConstraint"),
    )
    else {
        return;
    };
    // The rigid JointInstances (BasePart:GetConnectedParts' list); Glue, Rotate and the like are not
    let rigid: Vec<ClassId> = ["Weld", "Snap", "ManualWeld", "Motor"].iter().filter_map(|text| class(text)).collect();
    let kinds = reflection
        .classes
        .iter()
        .map(|info| {
            if info.is_a(weld_constraint) {
                Kind::WeldConstraint
            } else if rigid.iter().any(|&joint| info.is_a(joint)) {
                Kind::Joint
            } else {
                Kind::Other
            }
        })
        .collect();
    let setup = Setup {
        cframe,
        c0,
        c1,
        part0,
        part1,
        enabled,
        transform,
        anchored,
        massless,
        root_priority,
        size,
        cframe0,
        weld_state,
        base_part,
        motor6d: class("Motor6D"),
        world_root,
        joints_service: class("JointsService"),
        seats: ["Seat", "VehicleSeat"].iter().filter_map(|text| class(text)).collect(),
        kinds,
    };
    // (what a part's written property means to its kept component: stored)
    let mut watched = Vec::new();
    let mut put = |id: Option<NameId>, what: u8| {
        if let Some(id) = id {
            if watched.len() <= id as usize {
                watched.resize(id as usize + 1, 0);
            }
            watched[id as usize] = what;
        }
    };
    put(Some(cframe), watch::PLACE);
    for text in ["Material", "CustomPhysicalProperties", "Shape"] {
        put(name(text), watch::MASS);
    }
    for id in [massless, root_priority, size, crate::K::Name as NameId] {
        put(Some(id), watch::ROOTS);
    }
    put(Some(anchored), watch::ANCHORED);
    let mut state = engine.state.borrow_mut();
    // (which rigid joints are in a world, and which parts, is read from the dom's log of their homes: take_homes)
    let logged = (0..reflection.classes.len()).map(|class| setup.kinds[class] != Kind::Other || reflection.classes[class].is_a(base_part)).collect();
    state.dom.log_homes(logged);
    state.joints.setup = Some(Rc::new(setup));
    state.joints.watch = watched;
    drop(state);
    engine.hooks.borrow_mut().instance_freed.push(Box::new(forget));
    if let Some(root_name) = name("AssemblyRootPart") {
        engine.behaviors.borrow_mut().add_getter(base_part, root_name, crate::services::getter(|lua, engine, id| {
            let root = root_part(&mut engine.state.borrow_mut(), &engine.reflection, id);
            match root {
                Some(root) => crate::lua::instance_value(lua, engine, root).map_err(crate::lua::Flow::from),
                None => Ok(mlua::Value::Nil),
            }
        }));
    }
}

impl Joints {
    /// An instance of `class` was made (instance::create).
    #[inline]
    pub fn note_created(&mut self, class: ClassId) {
        if let Some(setup) = &self.setup {
            if setup.kind(class) != Kind::Other {
                self.created += 1;
            }
        }
    }

    /// Takes `joint` out of the lists of the parts it was indexed under, from its places in them (the components of the parts it
    /// linked go).
    fn unindex(&mut self, joint: InstanceId, old: &Indexed) {
        for side in 0..2 {
            let Some(part) = old.parts[side] else { continue };
            if side == 1 && old.parts[0] == Some(part) {
                continue;
            }
            // (a freed part's list went with it)
            let Some(list) = self.by_part.get_mut(&part) else { continue };
            let place = old.places[side] as usize;
            let there = list.slots.get(place) == Some(&Some(joint));
            debug_assert!(there, "a joint is at its place in its part's list");
            if !there {
                continue;
            }
            let linked = list.remove_at(place);
            if list.live == 0 {
                self.by_part.remove(&part);
            } else if list.slots.len() > 2 * list.live {
                list.close_holes(part, &mut self.indexed);
            }
            if linked {
                self.drop_component(part);
            }
        }
    }

    /// Forgets the kept component of `part`, for each of its parts.
    fn drop_component(&mut self, part: InstanceId) {
        let Some(found) = self.components.remove(&part) else { return };
        for other in &found.parts {
            if self.components.get(other).is_some_and(|kept| Rc::ptr_eq(kept, &found)) {
                self.components.remove(other);
            }
        }
    }
}

/// An instance was freed: a joint leaves its parts' lists, and a part's list goes, its joints' other parts to be worked out again
/// ([`relink`]).
fn forget(state: &mut State, id: InstanceId) {
    let joints = &mut state.joints;
    if let Some(old) = joints.indexed.remove(&id) {
        joints.unindex(id, &old);
    }
    if let Some(list) = joints.by_part.remove(&id) {
        joints.relinks.extend(list.slots.iter().flatten());
        joints.drop_component(id);
    }
}

fn setup_of(state: &State) -> Option<Rc<Setup>> {
    if state.joints.created == 0 {
        return None;
    }
    state.joints.setup.clone()
}

fn alive(state: &State, id: InstanceId) -> bool {
    state.dom.get(id).is_some()
}

fn reference(state: &State, id: InstanceId, name: NameId) -> Option<InstanceId> {
    state.dom.raw_get(id, name).and_then(Value::as_instance).filter(|&target| alive(state, target))
}

fn parts_of(state: &State, setup: &Setup, joint: InstanceId) -> [Option<InstanceId>; 2] {
    [reference(state, joint, setup.part0), reference(state, joint, setup.part1)]
}

fn cframe_of(state: &State, setup: &Setup, id: InstanceId) -> Affine3A {
    frame::affine_of(state.dom.raw_get(id, setup.cframe))
}

fn is_part(state: &State, reflection: &Reflection, setup: &Setup, id: InstanceId) -> bool {
    state.dom.get(id).is_some_and(|instance| reflection.class(instance.class).is_a(setup.base_part))
}

/// (Re)indexes a joint under the parts it names now: out of its old parts' lists and onto the end of its new ones', each at
/// the cost of one, however many joints name the part, marked there as linking or not.
fn index_joint(state: &mut State, reflection: &Reflection, setup: &Setup, joint: InstanceId) {
    let parts = parts_of(state, setup, joint);
    let homed = joint_home(state, joint).is_some();
    let joints = &mut state.joints;
    match joints.indexed.get(&joint) {
        Some(old) if old.parts == parts => return,
        Some(old) => {
            let old = *old;
            joints.unindex(joint, &old);
        }
        None => {}
    }
    let mut places = [0; 2];
    for side in 0..2 {
        let Some(part) = parts[side] else { continue };
        if side == 1 && parts[0] == Some(part) {
            places[1] = places[0];
            continue;
        }
        places[side] = joints.by_part.entry(part).or_default().push(joint, homed);
    }
    joints.indexed.insert(joint, Indexed { parts, places, taken: 0 });
    relink(state, reflection, setup, joint);
}

/// Works out again whether an indexed joint links each part it is listed under ([`link_in`]), marking its places; a part whose
/// mark changes loses its kept component.
fn relink(state: &mut State, reflection: &Reflection, setup: &Setup, joint: InstanceId) {
    let Some(entry) = state.joints.indexed.get(&joint).copied() else { return };
    for side in 0..2 {
        let Some(part) = entry.parts[side] else { continue };
        if side == 1 && entry.parts[0] == Some(part) {
            continue;
        }
        let place = entry.places[side] as usize;
        if !state.joints.by_part.get(&part).is_some_and(|list| list.slots.get(place) == Some(&Some(joint))) {
            continue;
        }
        let on = alive(state, part) && world_of(state, reflection, setup, part).is_some_and(|world| link_in(state, reflection, setup, joint, part, world).is_some());
        let joints = &mut state.joints;
        if joints.by_part.get_mut(&part).is_some_and(|list| list.link(place, on)) {
            joints.drop_component(part);
        }
    }
}

/// Takes the dom's log of the rigid joints and parts whose home changed, by whatever moved them (dom.rs Dom::log_homes), marks each
/// indexed joint in its parts' lists as in a world or not, and works out again whether each such joint, and each joint of each such
/// part in a world, links its parts ([`relink`]): every indexed joint when the log overflowed. Then the joints [`forget`] left.
fn take_homes(state: &mut State, reflection: &Reflection, setup: &Setup) {
    let log = state.dom.take_home_log();
    let State { dom, joints, .. } = state;
    let mark = |by_part: &mut FxHashMap<InstanceId, PartJoints>, joint: InstanceId, entry: &Indexed| {
        let homed = dom.home(joint).is_some();
        for side in 0..2 {
            let Some(part) = entry.parts[side] else { continue };
            if side == 1 && entry.parts[0] == Some(part) {
                continue;
            }
            let place = entry.places[side] as usize;
            if let Some(list) = by_part.get_mut(&part).filter(|list| list.slots.get(place) == Some(&Some(joint))) {
                list.mark(place, homed);
            }
        }
    };
    match log {
        Some(log) if !log.is_empty() => {
            joints.takes += 1;
            let mut parts: FxHashSet<InstanceId> = FxHashSet::default();
            for id in log {
                if let Some(entry) = joints.indexed.get_mut(&id).filter(|entry| entry.taken != joints.takes) {
                    entry.taken = joints.takes;
                    mark(&mut joints.by_part, id, entry);
                    joints.relinks.push(id);
                } else if joints.by_part.contains_key(&id) && parts.insert(id) {
                    // (a part's world changed: its joints in a world may link or stop; the others' homes are logged themselves)
                    let list = &joints.by_part[&id];
                    joints.relinks.extend(list.homed());
                    joints.drop_component(id);
                }
            }
        }
        Some(_) => {}
        None => {
            for (&joint, entry) in &joints.indexed {
                mark(&mut joints.by_part, joint, entry);
            }
            joints.relinks.extend(joints.indexed.keys().copied());
            joints.components.clear();
        }
    }
    let relinks = std::mem::take(&mut state.joints.relinks);
    for joint in relinks {
        relink(state, reflection, setup, joint);
    }
}

/// Whether the joint index holds together (tests), once it took the homes the dom logged: each indexed joint at its place in its
/// live parts' lists (one place when it names one part as both), marked there as in a world exactly when it has a home, each
/// list holding each of its joints once, as many as it counts, and none empty.
/// And each place marked as linking exactly when its joint links the part now ([`link_in`]), the walks taking up those, in order;
/// each kept component holding the parts linked to each of its parts.
#[cfg(test)]
pub(crate) fn check_index(state: &mut State, reflection: &Reflection) -> Result<(), String> {
    let Some(setup) = state.joints.setup.clone() else { return Ok(()) };
    take_homes(state, reflection, &setup);
    for (&part, list) in &state.joints.by_part {
        let world = if alive(state, part) { world_of(state, reflection, &setup, part) } else { None };
        for (place, slot) in list.slots.iter().enumerate() {
            let linking = slot.is_some_and(|joint| world.is_some_and(|world| link_in(state, reflection, &setup, joint, part, world).is_some()));
            if list.is_linking(place) != linking {
                return Err(format!("{part:?} marks {slot:?} at {place} as linking: {}, where it does: {linking}", list.is_linking(place)));
            }
        }
        let walked: Vec<InstanceId> = list.linking().collect();
        let want: Vec<InstanceId> = list.slots.iter().enumerate().filter(|&(place, _)| list.is_linking(place)).filter_map(|(_, slot)| *slot).collect();
        if walked != want || list.linking != want.len() || list.homed != list.homed().count() {
            return Err(format!("{part:?}'s walk takes up {walked:?} where its linking joints are {want:?} (counted {}, {} in a world)", list.linking, list.homed));
        }
    }
    let mut checked: Vec<*const Component> = Vec::new();
    for (&part, found) in &state.joints.components {
        if checked.contains(&Rc::as_ptr(found)) {
            continue;
        }
        checked.push(Rc::as_ptr(found));
        let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
        let mut queue = vec![part];
        seen.insert(part);
        while let Some(at) = queue.pop() {
            for (_, other) in edges(state, reflection, &setup, at, &mut Walk::default()) {
                if seen.insert(other) {
                    queue.push(other);
                }
            }
        }
        let kept: FxHashSet<InstanceId> = found.parts.iter().copied().collect();
        if seen != kept {
            return Err(format!("{part:?}'s kept component holds {} parts where {} are linked to it", kept.len(), seen.len()));
        }
    }
    let joints = &state.joints;
    for (&joint, entry) in &joints.indexed {
        for side in 0..2 {
            let Some(part) = entry.parts[side] else { continue };
            match joints.by_part.get(&part) {
                Some(list) if list.slots.get(entry.places[side] as usize).copied().flatten() == Some(joint) => {}
                None if !alive(state, part) => {}
                _ => return Err(format!("{joint:?} is not at its place {} under {part:?}", entry.places[side])),
            }
        }
    }
    for (&part, list) in &joints.by_part {
        let listed: Vec<InstanceId> = list.iter().collect();
        let distinct: FxHashSet<InstanceId> = listed.iter().copied().collect();
        if listed.is_empty() || listed.len() != list.live || distinct.len() != listed.len() {
            return Err(format!("{part:?} lists {} joints ({} distinct) where it counts {}", listed.len(), distinct.len(), list.live));
        }
        if let Some(joint) = listed.iter().find(|joint| !joints.indexed.get(joint).is_some_and(|entry| entry.parts.contains(&Some(part)))) {
            return Err(format!("{part:?} lists {joint:?}, which is not indexed under it"));
        }
        for (place, slot) in list.slots.iter().enumerate() {
            let homed = slot.is_some_and(|joint| joint_home(state, joint).is_some());
            if list.is_homed(place) != homed {
                return Err(format!("{part:?} marks {slot:?} at {place} as in a world: {}, where it is: {homed}", list.is_homed(place)));
            }
        }
        let walked: Vec<InstanceId> = list.homed().collect();
        let homed: Vec<InstanceId> = listed.iter().copied().filter(|&joint| joint_home(state, joint).is_some()).collect();
        if walked != homed {
            return Err(format!("{part:?}'s walk takes up {walked:?} where its joints in a world are {homed:?}"));
        }
    }
    Ok(())
}

/// Readies the index for a walk: every rigid joint the engine holds indexed, once (those made before anything needed the
/// index), each marked as in a world or not as the dom last logged its home, and as linking or not ([`take_homes`]).
fn ensure_scanned(state: &mut State, reflection: &Reflection, setup: &Setup) {
    if !state.joints.scanned {
        state.joints.scanned = true;
        let joints: Vec<InstanceId> = (0..state.dom.slot_count() as u32)
            .filter_map(|slot| state.dom.id_at_slot(slot))
            .filter(|&id| setup.kind(state.dom.at(id).class) != Kind::Other)
            .collect();
        for joint in joints {
            index_joint(state, reflection, setup, joint);
        }
    }
    take_homes(state, reflection, setup);
}

/// The WorldRoot an instance is in (its nearest WorldRoot ancestor), from the homes the dom keeps: no walk up the tree, however
/// deep the instance is (world/mod.rs world_root_of).
fn world_of(state: &State, reflection: &Reflection, setup: &Setup, id: InstanceId) -> Option<InstanceId> {
    super::world_root_of(state, reflection, setup.world_root, id)
}

/// Where a joint works from: its WorldRoot, or JointsService (its nearest ancestor of either: the home the dom keeps, dom.rs
/// Dom::home).
fn joint_home(state: &State, id: InstanceId) -> Option<InstanceId> {
    state.dom.home(id)
}

/// What the walks of one change looked at, for its price ([`reparented`]): the joints looked at in their parts' lists, and the
/// links found that the change made (their joint or one of their parts is among `moved`), each found from both its parts.
#[derive(Default)]
struct Walk<'a> {
    moved: Option<&'a FxHashSet<InstanceId>>,
    looked: usize,
    made: usize,
}

/// The part `joint` links `part` (in `world`) to now, if it does: the joint is a rigid one and not disabled, names `part` and
/// another BasePart in the same world, and is itself in that world or JointsService. A few lookups, whatever the depth of the
/// parts and the joint in the tree.
fn link_in(state: &State, reflection: &Reflection, setup: &Setup, joint: InstanceId, part: InstanceId, world: InstanceId) -> Option<InstanceId> {
    let instance = state.dom.get(joint)?;
    if setup.kind(instance.class) == Kind::Other || state.dom.boolean(joint, setup.enabled) == Some(false) {
        return None;
    }
    let [Some(part0), Some(part1)] = parts_of(state, setup, joint) else { return None };
    let other = if part0 == part {
        part1
    } else if part1 == part {
        part0
    } else {
        return None;
    };
    if other == part || !reflection.class(state.dom.at(other).class).is_a(setup.base_part) {
        return None;
    }
    if world_of(state, reflection, setup, other) != Some(world) {
        return None;
    }
    match joint_home(state, joint) {
        Some(home) if home == world || Some(state.dom.at(home).class) == setup.joints_service => Some(other),
        _ => None,
    }
}

/// The joints that link `part` to another part now, with that part, in the order its list holds them. The walk takes up the
/// places marked as linking alone ([`relink`]), each checked again ([`link_in`]), so a part's joints that cannot link cost it
/// nothing; `walk` counts its joints in a world as looked at (what a move into a world pays for: cost.rs joints_entering).
fn edges(state: &State, reflection: &Reflection, setup: &Setup, part: InstanceId, walk: &mut Walk) -> Vec<(InstanceId, InstanceId)> {
    let Some(candidates) = state.joints.by_part.get(&part) else {
        return Vec::new();
    };
    // (a part outside every WorldRoot links to nothing)
    let Some(world) = world_of(state, reflection, setup, part) else {
        return Vec::new();
    };
    walk.looked += candidates.homed;
    let mut found = Vec::new();
    for joint in candidates.linking() {
        // (marked as linking, it links while the marks are kept: check_index)
        let Some(other) = link_in(state, reflection, setup, joint, part, world) else { continue };
        if walk.moved.is_some_and(|moved| moved.contains(&joint) || moved.contains(&part) || moved.contains(&other)) {
            walk.made += 1;
        }
        found.push((joint, other));
    }
    found
}

fn anchored(state: &State, setup: &Setup, part: InstanceId) -> bool {
    state.dom.boolean(part, setup.anchored) == Some(true)
}

/// The root a part would be among unanchored parts: (not Massless, RootPriority, size score). The score is the largest
/// face's area, not rounded, times the seats' and names' weights (measured in Studio 0.746 through AssemblyRootPart: a
/// 4 x 4.0001 face beats a 4 x 4 one, a 1 x 1 HumanoidRootPart ties a face of 10, a Torso one of 5, a Seat one of 20).
fn root_rank(state: &State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> (bool, i64, f32) {
    let massless = state.dom.boolean(part, setup.massless) == Some(true);
    let priority = state.dom.number(part, setup.root_priority).unwrap_or(0.0) as i64;
    let (x, y, z) = match state.dom.raw_get(part, setup.size) {
        Some(Value::Vector3(v)) => (v.x, v.y, v.z),
        _ => (0.0, 0.0, 0.0),
    };
    let face = (x * y).max(y * z).max(z * x);
    let instance = state.dom.at(part);
    let multiplier = if setup.seats.iter().any(|&seat| reflection.class(instance.class).is_a(seat)) {
        20.0
    } else if instance.name.as_bytes() == b"HumanoidRootPart" {
        10.0
    } else if instance.name.as_bytes() == b"Torso" {
        5.0
    } else {
        1.0
    };
    (!massless, priority, face * multiplier)
}

/// BasePart.AssemblyRootPart: nil for a part outside a WorldRoot (the Workspace or a WorldModel), an anchored part
/// itself, else the root of the assembly the part is in, as [`assemblies_of`] picks it (measured in a Studio 0.746 playing
/// client: the joints' kind and direction, the parts' names and the order they were parented in do not matter; of equal
/// parts the later made one is the root). Read from the part's kept component: no walk while nothing it depends on changed.
pub fn root_part(state: &mut State, reflection: &Reflection, part: InstanceId) -> Option<InstanceId> {
    let setup = state.joints.setup.clone()?;
    world_of(state, reflection, &setup, part)?;
    if state.joints.created == 0 || anchored(state, &setup, part) {
        return Some(part);
    }
    ensure_scanned(state, reflection, &setup);
    let Some(found) = component(state, reflection, &setup, part) else {
        return Some(part);
    };
    let tree = assemblies_of(state, reflection, &setup, &found, part, None, true);
    tree.of(part).map(|(assembly, ..)| assembly.nodes[0].part)
}

/// The parts linked to `start` now and their links, as a walk from it finds them, what it looked at counted in `walk`; None when
/// `start` links to no other part.
fn walk_component(state: &State, reflection: &Reflection, setup: &Setup, start: InstanceId, walk: &mut Walk) -> Option<Component> {
    let mut parts = vec![start];
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    seen.insert(start);
    let mut links: FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>> = FxHashMap::default();
    let mut index = 0;
    while index < parts.len() {
        let part = parts[index];
        let found = edges(state, reflection, setup, part, walk);
        for &(_, other) in &found {
            if seen.insert(other) {
                parts.push(other);
            }
        }
        links.insert(part, found);
        index += 1;
    }
    if parts.len() == 1 {
        return None;
    }
    Some(Component { parts, links, tree: Default::default(), memo: Default::default() })
}

/// The component of `part`, the index readied ([`ensure_scanned`]): the one kept for it, else found by a walk from it and kept
/// for each of its parts. None when it links to no other part.
fn component(state: &mut State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> Option<Rc<Component>> {
    if let Some(found) = state.joints.components.get(&part) {
        return Some(found.clone());
    }
    let found = Rc::new(walk_component(state, reflection, setup, part, &mut Walk::default())?);
    keep(state, &found);
    Some(found)
}

/// Keeps `found` for each of its parts.
fn keep(state: &mut State, found: &Rc<Component>) {
    if !keeping() {
        return;
    }
    for &part in &found.parts {
        state.joints.components.insert(part, found.clone());
    }
}

#[cfg(test)]
thread_local! {
    static KEEPING: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether the walks keep the components they find ([`keep`]); tests turn it off on their thread, so every walk is made afresh, to
/// check that what is kept answers as a fresh walk does.
#[cfg(test)]
fn keeping() -> bool {
    KEEPING.with(std::cell::Cell::get)
}

#[cfg(not(test))]
#[inline]
fn keeping() -> bool {
    true
}

/// Turns keeping components on or off for this thread's engines ([`keeping`]).
#[cfg(test)]
pub(crate) fn set_keeping(on: bool) {
    KEEPING.with(|keeping| keeping.set(on));
}

/// A component's parts in the order a walk from `start` (one of them) finds them: its links, in their order, from `start`.
fn order_from(found: &Component, start: InstanceId) -> Vec<InstanceId> {
    if found.parts[0] == start {
        return found.parts.clone();
    }
    let mut order = vec![start];
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    seen.insert(start);
    let mut index = 0;
    while index < order.len() {
        for &(_, other) in found.links.get(&order[index]).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(other) {
                order.push(other);
            }
        }
        index += 1;
    }
    order
}

/// The assemblies of a component (several when more than one part is anchored), as a walk from `start` finds them, kept with it.
/// `derive`: a WeldConstraint among its links that never took its parts' relative CFrame takes it first (as every walk that settles
/// parts or reads a root does; one that only counts parts leaves it, and keeps nothing while one waits). `flipped`: a part whose
/// Anchored is read the other way (an Anchored write's assemblies as they were), kept for no one; nor are those of a component
/// where a part's root score is NaN, whose root the order the parts are looked at in picks.
fn assemblies_of(state: &mut State, reflection: &Reflection, setup: &Setup, found: &Component, start: InstanceId, flipped: Option<InstanceId>, derive: bool) -> Rc<Assemblies> {
    if let (None, Some(tree)) = (flipped, found.tree.borrow().as_ref()) {
        return tree.clone();
    }
    let mut keep = flipped.is_none();

    // A WeldConstraint that never took its parts' relative CFrame takes it now
    let welds: Vec<InstanceId> = found
        .links
        .values()
        .flatten()
        .map(|&(joint, _)| joint)
        .filter(|&joint| setup.kind(state.dom.at(joint).class) == Kind::WeldConstraint && !weld_valid(state, setup, joint))
        .collect();
    if derive {
        for weld in welds {
            derive_weld(state, reflection, setup, weld);
        }
    } else if !welds.is_empty() {
        keep = false;
    }

    // Roots: the anchored parts, else the best ranked one
    let serial = |state: &State, part: InstanceId| state.dom.at(part).serial;
    let is_anchored = |state: &State, part: InstanceId| anchored(state, setup, part) != (flipped == Some(part));
    let mut roots: Vec<InstanceId> = found.parts.iter().copied().filter(|&part| is_anchored(state, part)).collect();
    roots.sort_by_key(|&part| serial(state, part));
    if roots.is_empty() {
        // (of ranks that compare, the best is the same in any order; a NaN score is looked at in the walk's order from `start`)
        let nan = found.parts.iter().any(|&part| root_rank(state, reflection, setup, part).2.is_nan());
        let order: Vec<InstanceId> = if nan {
            keep = false;
            order_from(found, start)
        } else {
            Vec::new()
        };
        let candidates: &[InstanceId] = if nan { &order } else { &found.parts };
        let mut best = candidates[0];
        let mut best_rank = root_rank(state, reflection, setup, best);
        for &part in &candidates[1..] {
            let rank = root_rank(state, reflection, setup, part);
            let better = rank.0 != best_rank.0 && rank.0
                || rank.0 == best_rank.0 && (rank.1 > best_rank.1 || rank.1 == best_rank.1 && (rank.2 > best_rank.2 || rank.2 == best_rank.2 && serial(state, part) > serial(state, best)));
            if better {
                best = part;
                best_rank = rank;
            }
        }
        roots.push(best);
    }

    // Each root's tree, nearest root first
    let mut list: Vec<Assembly> = Vec::with_capacity(roots.len());
    let mut place: FxHashMap<InstanceId, (u32, u32)> = FxHashMap::default();
    let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
    for root in roots {
        place.insert(root, (list.len() as u32, 0));
        queue.push_back((list.len(), 0));
        list.push(Assembly { nodes: vec![Node { part: root, parent: None }] });
    }
    while let Some((assembly, node)) = queue.pop_front() {
        let part = list[assembly].nodes[node].part;
        for &(joint, other) in found.links.get(&part).map(Vec::as_slice).unwrap_or_default() {
            if let std::collections::hash_map::Entry::Vacant(entry) = place.entry(other) {
                let index = list[assembly].nodes.len();
                entry.insert((assembly as u32, index as u32));
                queue.push_back((assembly, index));
                list[assembly].nodes.push(Node { part: other, parent: Some((node, joint)) });
            }
        }
    }
    // (each joint between two assemblies, counted on both sides)
    let mut across = vec![0u32; list.len()];
    for (part, links) in &found.links {
        let Some(&(assembly, _)) = place.get(part) else { continue };
        across[assembly as usize] += links.iter().filter(|(_, other)| place.get(other).map(|&(at, _)| at) != Some(assembly)).count() as u32;
    }
    let tree = Rc::new(Assemblies { list, place, across });
    if keep {
        *found.tree.borrow_mut() = Some(tree.clone());
    }
    tree
}

/// Every part rigidly linked to `part` now (the part itself first, then as a walk from it finds them), anchored ones included,
/// kept with its component.
pub fn linked_parts(state: &mut State, reflection: &Reflection, part: InstanceId) -> Rc<[InstanceId]> {
    let Some(setup) = setup_of(state) else {
        return Rc::from([part]);
    };
    ensure_scanned(state, reflection, &setup);
    let Some(found) = component(state, reflection, &setup, part) else {
        return Rc::from([part]);
    };
    if let Some(order) = found.memo.borrow().linked.get(&part).cloned() {
        return order;
    }
    let order: Rc<[InstanceId]> = order_from(&found, part).into();
    found.memo.borrow_mut().linked.insert(part, order.clone());
    order
}

/// AssemblyMass and AssemblyCenterOfMass of `part` (`body` over [`linked_parts`]' answer: world/gravity.rs body_of), kept with its
/// component: the mass until a part's mass changes, the centre also until one moves ([`stored`]). `centre`: whether the centre is
/// asked for (a mass alone is answered while only the parts moved).
pub fn assembly_body(state: &mut State, reflection: &Reflection, part: InstanceId, centre: bool, body: impl FnOnce(&State, &[InstanceId]) -> (f32, glam::Vec3)) -> (f32, glam::Vec3) {
    let found = match setup_of(state) {
        Some(setup) => {
            ensure_scanned(state, reflection, &setup);
            component(state, reflection, &setup, part)
        }
        None => None,
    };
    let Some(found) = found else {
        return body(state, &[part]);
    };
    {
        let memo = found.memo.borrow();
        if let Some(&mass) = memo.mass.get(&part) {
            if !centre {
                return (mass, glam::Vec3::ZERO);
            }
            if let Some(&at) = memo.centre.get(&part) {
                return (mass, at);
            }
        }
    }
    let order = linked_parts(state, reflection, part);
    let (mass, at) = body(state, &order);
    let mut memo = found.memo.borrow_mut();
    memo.mass.insert(part, mass);
    memo.centre.insert(part, at);
    (mass, at)
}

/// How many parts the assembly of `part` holds (1 for one that links to no other), from its kept component.
pub fn assembly_size(state: &mut State, reflection: &Reflection, part: InstanceId) -> usize {
    let Some(setup) = setup_of(state) else { return 1 };
    ensure_scanned(state, reflection, &setup);
    let Some(found) = component(state, reflection, &setup, part) else { return 1 };
    let tree = assemblies_of(state, reflection, &setup, &found, part, None, false);
    tree.of(part).map_or(1, |(assembly, ..)| assembly.nodes.len())
}

/// A property of `id` was stored (instance.rs store: every write, signalled or not): its kept component forgets what the value
/// fed ([`watch`]): the centres of mass when a part moves, the masses with them when its mass changes, the assemblies when its
/// Anchored changes (set aside for the write's price: [`reanchored`]), the whole component when its root rank does.
#[inline]
pub fn stored(state: &mut State, id: InstanceId, name: NameId) {
    let joints = &mut state.joints;
    if joints.components.is_empty() {
        return;
    }
    let what = joints.watch.get(name as usize).copied().unwrap_or(0);
    if what == 0 {
        return;
    }
    let Some(found) = joints.components.get(&id).cloned() else { return };
    match what {
        watch::PLACE => found.memo.borrow_mut().centre.clear(),
        watch::MASS => {
            let mut memo = found.memo.borrow_mut();
            memo.mass.clear();
            memo.centre.clear();
        }
        watch::ANCHORED => {
            let tree = found.tree.borrow_mut().take();
            joints.before_anchor = Some((id, tree));
        }
        _ => joints.drop_component(id),
    }
}

/// BulkMoveTo's moves start (`moving`) or end: meanwhile a moved assembly pays nothing for its joints to other assemblies.
pub fn bulk_moves(state: &mut State, moving: bool) {
    if moving {
        state.joints.bulk += 1;
    } else {
        state.joints.bulk = state.joints.bulk.saturating_sub(1);
    }
}

fn weld_valid(state: &State, setup: &Setup, weld: InstanceId) -> bool {
    state.dom.number(weld, setup.weld_state).is_some_and(|value| (value as i64) & 2 != 0)
}

/// A WeldConstraint takes its parts' relative CFrame (CFrame0 = Part0:ToObjectSpace(Part1.CFrame),
/// and State 3, or 2 while disabled: Studio's files hold State 3 with that CFrame0, and State 1
/// (enabled, no offset yet) with the identity).
fn derive_weld(state: &mut State, reflection: &Reflection, setup: &Setup, weld: InstanceId) {
    let [Some(part0), Some(part1)] = parts_of(state, setup, weld) else { return };
    let offset = inverse(&cframe_of(state, setup, part0)) * cframe_of(state, setup, part1);
    let enabled = state.dom.boolean(weld, setup.enabled) != Some(false);
    core::raw_set_silent(state, reflection, weld, setup.cframe0, frame::cframe_value(&offset));
    core::raw_set_silent(state, reflection, weld, setup.weld_state, Value::Number(if enabled { 3.0 } else { 2.0 }));
}

/// The CFrame of the part across `joint` from `from`, relative to `from`.
fn across(state: &State, reflection: &Reflection, setup: &Setup, joint: InstanceId, from: InstanceId) -> Affine3A {
    let class = state.dom.at(joint).class;
    let forward = match setup.kind(class) {
        Kind::WeldConstraint => frame::affine_of(state.dom.raw_get(joint, setup.cframe0)),
        _ => {
            let c0 = frame::affine_of(state.dom.raw_get(joint, setup.c0));
            let c1 = frame::affine_of(state.dom.raw_get(joint, setup.c1));
            let transform = match setup.motor6d {
                Some(motor) if reflection.class(class).is_a(motor) => frame::affine_of(state.dom.raw_get(joint, setup.transform)),
                _ => Affine3A::IDENTITY,
            };
            c0 * transform * inverse(&c1)
        }
    };
    if reference(state, joint, setup.part0) == Some(from) { forward } else { inverse(&forward) }
}

/// Moves a part the assembly code computed: stored, the engine's hooks told, no Changed events.
fn place(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, part: InstanceId, cframe: &Affine3A) {
    core::raw_set_unsignalled(state, reflection, hooks, part, setup.cframe, frame::cframe_value(cframe));
}

/// Settles an assembly: every part where its joints put it from the anchor. `tolerant`: a part
/// already there within float noise keeps its value. How many parts it placed.
fn settle(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, assembly: &Assembly, anchor: Anchor, tolerant: bool) -> usize {
    let mut relative: Vec<Affine3A> = Vec::with_capacity(assembly.nodes.len());
    for node in &assembly.nodes {
        relative.push(match node.parent {
            None => Affine3A::IDENTITY,
            Some((parent, joint)) => relative[parent] * across(state, reflection, setup, joint, assembly.nodes[parent].part),
        });
    }
    // The root is never placed through the joints: a Motor6D.Transform set since the parts were
    // last placed would swing it (an anchored root too) about the moved part. It is stored as a
    // client stores a part's CFrame: `before`'s transposed inverse squares any stretch the root
    // carries, so unstored it doubled its error with each part a BulkMoveTo wrote
    let (fixed, root) = match anchor {
        Anchor::Root => (0, cframe_of(state, setup, assembly.nodes[0].part)),
        Anchor::Moved(index, before) => {
            let moved = cframe_of(state, setup, assembly.nodes[index].part) * inverse(&before);
            (index, frame::stored_transform(&(moved * cframe_of(state, setup, assembly.nodes[0].part))))
        }
    };
    let mut placed = 0;
    for (index, node) in assembly.nodes.iter().enumerate() {
        if index == fixed {
            continue;
        }
        let target = root * relative[index];
        let current = cframe_of(state, setup, node.part);
        if tolerant && settled(&current, &target) {
            continue;
        }
        place(state, reflection, hooks, setup, node.part, &target);
        placed += 1;
    }
    placed
}

/// Settles the assemblies of `seeds` from their roots. `walk`: a move into a world, whose price is what its walks look at
/// (cost.rs joints_entering), so it walks each component and counts what it looked at in `walk` (keeping what it found); else the
/// kept components serve.
fn settle_from_roots(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, seeds: &[InstanceId], mut walk: Option<&mut Walk>) {
    let mut visited: FxHashSet<InstanceId> = FxHashSet::default();
    for &seed in seeds {
        if visited.contains(&seed) || !is_part(state, reflection, setup, seed) {
            continue;
        }
        let found = match walk.as_deref_mut() {
            Some(walk) => walk_component(state, reflection, setup, seed, walk).map(|found| {
                let found = Rc::new(found);
                keep(state, &found);
                found
            }),
            None => component(state, reflection, setup, seed),
        };
        let Some(found) = found else {
            visited.insert(seed);
            continue;
        };
        let tree = assemblies_of(state, reflection, setup, &found, seed, None, true);
        for assembly in &tree.list {
            visited.extend(assembly.nodes.iter().map(|node| node.part));
            if assembly.nodes.len() > 1 {
                settle(state, reflection, hooks, setup, assembly, Anchor::Root, true);
            }
        }
    }
}

/// A property changed through RawSet (instance::notify_changed; `old` its previous value, when
/// known): a part's CFrame moves its assembly; a rigid joint's C0 / C1 / Part0 / Part1 / Enabled
/// settle it; a part's Anchored splits or joins its assemblies; a Motor6D's Transform waits for the simulation step.
pub fn property_changed(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, name: NameId, old: Option<&Value>) {
    let Some(setup) = setup_of(state) else { return };
    if name == setup.cframe {
        if state.joints.quiet > 0 || !reflection.class(state.dom.at(id).class).is_a(setup.base_part) {
            return;
        }
        let before = match old {
            Some(value) => frame::affine_of(Some(value)),
            None => cframe_of(state, &setup, id),
        };
        if state.joints.batch > 0 {
            note_batch_write(state, id, before);
            return;
        }
        part_moved(state, reflection, hooks, &setup, id, before);
    } else if name == setup.c0 || name == setup.c1 || name == setup.part0 || name == setup.part1 || name == setup.enabled {
        let kind = setup.kind(state.dom.at(id).class);
        if kind == Kind::Other {
            return;
        }
        ensure_scanned(state, reflection, &setup);
        index_joint(state, reflection, &setup, id);
        let links = name == setup.part0 || name == setup.part1 || name == setup.enabled && state.dom.boolean(id, setup.enabled) != Some(false);
        if name != setup.c0 && name != setup.c1 {
            relink(state, reflection, &setup, id);
        }
        if kind == Kind::WeldConstraint && links {
            derive_weld(state, reflection, &setup, id);
        }
        if state.joints.quiet > 0 {
            return;
        }
        let seeds: Vec<InstanceId> = parts_of(state, &setup, id).into_iter().flatten().collect();
        if state.joints.batch > 0 {
            state.joints.batch_seeds.extend(seeds);
            return;
        }
        settle_from_roots(state, reflection, hooks, &setup, &seeds, None);
    } else if name == setup.anchored {
        // (the assemblies as they were, which the write set aside: stored)
        let before = state.joints.before_anchor.take().filter(|&(part, _)| part == id).and_then(|(_, tree)| tree);
        // Anchoring splits or joins assemblies: a joint that links up again moves what hangs below it
        if state.joints.quiet > 0 || !reflection.class(state.dom.at(id).class).is_a(setup.base_part) || !state.joints.by_part.contains_key(&id) {
            return;
        }
        if state.joints.batch > 0 {
            state.joints.batch_seeds.push(id);
            return;
        }
        ensure_scanned(state, reflection, &setup);
        reanchored(state, reflection, hooks, &setup, id, before);
    } else if name == setup.transform {
        let is_motor = setup.motor6d.is_some_and(|motor| reflection.class(state.dom.at(id).class).is_a(motor));
        if is_motor && !state.joints.pending.contains(&id) {
            state.joints.pending.push(id);
        }
    }
}

/// A part's Anchored changed: its assemblies settle from their roots, and the work clock charges each other part whose assembly
/// root, or that root's anchoring, the write changed, and each joint linking the part (cost.rs joints_reanchored), beyond the
/// write. `before`: the part's component's assemblies as they were, when they were kept; else they are worked out again with its
/// Anchored read the other way.
fn reanchored(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, part: InstanceId, before: Option<Rc<Assemblies>>) {
    let found = component(state, reflection, setup, part);
    let before = before.or_else(|| found.as_ref().map(|found| assemblies_of(state, reflection, setup, found, part, Some(part), false)));
    settle_from_roots(state, reflection, hooks, setup, &[part], None);
    let (Some(found), Some(before)) = (found, before) else { return };
    let after = assemblies_of(state, reflection, setup, &found, part, None, true);
    let mut looked: FxHashSet<InstanceId> = FxHashSet::default();
    let mut changed = 0usize;
    for tree in [&before, &after] {
        let Some((assembly, ..)) = tree.of(part) else { continue };
        for node in &assembly.nodes {
            if node.part != part && looked.insert(node.part) && before.root_of(state, setup, node.part, Some(part)) != after.root_of(state, setup, node.part, None) {
                changed += 1;
            }
        }
    }
    let links = state.joints.by_part.get(&part).map_or(0, |list| list.linking);
    let work = crate::cost::joints_reanchored(changed, links);
    if work > 0.0 {
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
}

fn note_batch_write(state: &mut State, id: InstanceId, before: Affine3A) {
    if !state.joints.batch_parts.iter().any(|&(part, _)| part == id) {
        state.joints.batch_parts.push((id, before));
    }
}

/// A CFrame write that stored the value the part already had (instance::raw_set). Inside a
/// Model:PivotTo / ScaleTo batch it still counts as written: PivotTo writing an assembly's root
/// where it is settles the assembly from that root.
#[inline]
pub fn written_unchanged(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId) {
    if state.joints.batch == 0 || state.joints.quiet > 0 {
        return;
    }
    let Some(setup) = setup_of(state) else { return };
    if name != setup.cframe || !reflection.class(state.dom.at(id).class).is_a(setup.base_part) {
        return;
    }
    let before = cframe_of(state, &setup, id);
    note_batch_write(state, id, before);
}

/// A part's CFrame was set (it was at `before`): its assembly's root moves as the part moved and
/// the rest follows from the root through the joints (as Roblox derives an assembly's parts from
/// its root: nothing drifts however often it moves). The work clock charges each part carried and,
/// but for BulkMoveTo's moves, each joint between the assembly and another (cost.rs assembly_moved).
fn part_moved(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, part: InstanceId, before: Affine3A) {
    ensure_scanned(state, reflection, setup);
    if !state.joints.by_part.contains_key(&part) {
        return;
    }
    let Some(found) = component(state, reflection, setup, part) else { return };
    let tree = assemblies_of(state, reflection, setup, &found, part, None, true);
    let Some((assembly, index, node)) = tree.of(part) else { return };
    let carried = match node {
        _ if assembly.nodes.len() < 2 => 0,
        0 => settle(state, reflection, hooks, setup, assembly, Anchor::Root, false),
        node => settle(state, reflection, hooks, setup, assembly, Anchor::Moved(node, before), false),
    };
    let across = if state.joints.bulk > 0 { 0 } else { tree.across[index] as usize };
    let work = crate::cost::assembly_moved(carried, across);
    if work > 0.0 {
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
}

/// An instance's parent changed (instance::set_parent): joints and parts entering a WorldRoot
/// link up, and their assemblies settle from their roots. The work clock charges each joint the
/// walks look at, each link the move makes and each joint it brings into the world (cost.rs
/// joints_entering), beyond what the move pays for the instances it carries.
pub fn reparented(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, old_parent: Option<InstanceId>, new_parent: Option<InstanceId>) {
    let Some(setup) = setup_of(state) else { return };
    let Some(new_parent) = new_parent else { return };
    let home = |state: &State, node: InstanceId| {
        let class = state.dom.at(node).class;
        if reflection.class(class).is_a(setup.world_root) || Some(class) == setup.joints_service {
            Some(node)
        } else {
            joint_home(state, node)
        }
    };
    let Some(world) = home(state, new_parent) else { return };
    if old_parent.and_then(|parent| home(state, parent)) == Some(world) {
        return;
    }
    ensure_scanned(state, reflection, &setup);
    let mut seeds = Vec::new();
    // (the joints it carries and its parts that joints name: each link it makes has one of them; and how many of those joints
    // it brings into this world)
    let mut moved: FxHashSet<InstanceId> = FxHashSet::default();
    let mut entered = 0usize;
    let mut nodes = vec![id];
    nodes.extend(state.dom.descendants(id));
    for node in nodes {
        let class = state.dom.at(node).class;
        if setup.kind(class) != Kind::Other {
            index_joint(state, reflection, &setup, node);
            seeds.extend(parts_of(state, &setup, node).into_iter().flatten());
            moved.insert(node);
            entered += usize::from(joint_home(state, node) == Some(world));
        } else if reflection.class(class).is_a(setup.base_part) && state.joints.by_part.contains_key(&node) {
            seeds.push(node);
            moved.insert(node);
        }
    }
    if state.joints.quiet > 0 {
        return;
    }
    let mut walk = Walk { moved: Some(&moved), ..Walk::default() };
    if !seeds.is_empty() {
        if state.joints.batch > 0 {
            state.joints.batch_seeds.extend(seeds);
        } else {
            settle_from_roots(state, reflection, hooks, &setup, &seeds, Some(&mut walk));
        }
    }
    let work = crate::cost::joints_entering(walk.looked, walk.made, entered);
    if work > 0.0 {
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
}

/// BasePart Position / Orientation / Rotation: the part moves alone. Each Weld or Motor6D joined to
/// it gets the C0 (as its Part0) or C1 (as its Part1) that keeps it there; each WeldConstraint
/// joined to it takes the new relative CFrame. The work clock charges each joint rewritten and, for
/// each inside the part's assembly, each part of it (cost.rs joints_rewritten).
pub fn move_alone(state: &mut State, reflection: &Reflection, hooks: &Hooks, part: InstanceId, name: NameId, cframe: &Affine3A) {
    let Some(setup) = setup_of(state) else {
        core::raw_set(state, reflection, hooks, part, name, frame::cframe_value(cframe));
        return;
    };
    state.joints.quiet += 1;
    core::raw_set(state, reflection, hooks, part, name, frame::cframe_value(cframe));
    ensure_scanned(state, reflection, &setup);
    let links = edges(state, reflection, &setup, part, &mut Walk::default());
    // (the part's assembly, which a rewritten joint inside it settles again on a client)
    let tree = if links.is_empty() { None } else { component(state, reflection, &setup, part).map(|found| assemblies_of(state, reflection, &setup, &found, part, None, false)) };
    let own = tree.as_ref().and_then(|tree| tree.of(part).map(|(assembly, index, _)| (index, assembly.nodes.len())));
    let (mut rewritten, mut inside) = (0usize, 0usize);
    for (joint, other) in links {
        match setup.kind(state.dom.at(joint).class) {
            Kind::WeldConstraint => derive_weld(state, reflection, &setup, joint),
            Kind::Joint => {
                let [Some(part0), Some(part1)] = parts_of(state, &setup, joint) else { continue };
                let c0 = frame::affine_of(state.dom.raw_get(joint, setup.c0));
                let c1 = frame::affine_of(state.dom.raw_get(joint, setup.c1));
                let class = state.dom.at(joint).class;
                let transform = match setup.motor6d {
                    Some(motor) if reflection.class(class).is_a(motor) => frame::affine_of(state.dom.raw_get(joint, setup.transform)),
                    _ => Affine3A::IDENTITY,
                };
                // Part0 * C0 * Transform = Part1 * C1
                let (name, value) = if part0 == part {
                    (setup.c0, inverse(cframe) * cframe_of(state, &setup, part1) * c1 * inverse(&transform))
                } else {
                    (setup.c1, inverse(cframe) * cframe_of(state, &setup, part0) * c0 * transform)
                };
                core::raw_set(state, reflection, hooks, joint, name, frame::cframe_value(&value));
            }
            Kind::Other => continue,
        }
        rewritten += 1;
        if let (Some(tree), Some((index, _))) = (&tree, own) {
            inside += usize::from(tree.place.get(&other).is_some_and(|&(at, _)| at as usize == index));
        }
    }
    state.joints.quiet -= 1;
    let work = crate::cost::joints_rewritten(rewritten, inside, own.map_or(1, |(_, parts)| parts));
    if work > 0.0 {
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
}

/// Model:PivotTo / ScaleTo start moving many parts: nothing settles until [`end_batch`].
pub fn begin_batch(state: &mut State) {
    state.joints.batch += 1;
}

/// The batch ends: each assembly it touched settles from its root, if the batch wrote it, else
/// from its root moved as the first of its parts the batch wrote moved. The call (whose engine Luau
/// prices itself) is charged each part carried and each joint between an assembly it wrote and
/// another (cost.rs assembly_moved).
pub fn end_batch(state: &mut State, reflection: &Reflection, hooks: &Hooks) {
    let joints = &mut state.joints;
    joints.batch = joints.batch.saturating_sub(1);
    if joints.batch > 0 {
        return;
    }
    let written = std::mem::take(&mut joints.batch_parts);
    let seeds = std::mem::take(&mut joints.batch_seeds);
    let Some(setup) = setup_of(state) else { return };
    ensure_scanned(state, reflection, &setup);
    let order: FxHashMap<InstanceId, usize> = written.iter().enumerate().map(|(index, &(part, _))| (part, index)).collect();
    let mut visited: FxHashSet<InstanceId> = FxHashSet::default();
    let (mut carried, mut between) = (0usize, 0usize);
    for seed in written.iter().map(|&(part, _)| part).chain(seeds.iter().copied()) {
        if visited.contains(&seed) || !state.joints.by_part.contains_key(&seed) || !is_part(state, reflection, &setup, seed) {
            continue;
        }
        let Some(found) = component(state, reflection, &setup, seed) else {
            visited.insert(seed);
            continue;
        };
        let tree = assemblies_of(state, reflection, &setup, &found, seed, None, true);
        for (index, assembly) in tree.list.iter().enumerate() {
            visited.extend(assembly.nodes.iter().map(|node| node.part));
            let first = assembly
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| order.get(&node.part).map(|&rank| (rank, index)))
                .min();
            if first.is_some() {
                between += tree.across[index] as usize;
            }
            if assembly.nodes.len() < 2 {
                continue;
            }
            let anchor = match first {
                Some(_) if order.contains_key(&assembly.nodes[0].part) => Anchor::Root,
                Some((rank, index)) => Anchor::Moved(index, written[rank].1),
                None => Anchor::Root,
            };
            carried += settle(state, reflection, hooks, &setup, assembly, anchor, true);
        }
    }
    let work = crate::cost::assembly_moved(carried, between);
    if work > 0.0 {
        state.sched.charge_priced(work);
    }
}

/// The deferred Motor6D.Transform batch: each changed motor's assembly settles from its root.
pub fn step_transforms(engine: &Engine) {
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    if state.joints.pending.is_empty() {
        return;
    }
    let Some(setup) = setup_of(&state) else {
        state.joints.pending.clear();
        return;
    };
    let motors = std::mem::take(&mut state.joints.pending);
    ensure_scanned(&mut state, &engine.reflection, &setup);
    let mut seeds = Vec::new();
    for motor in motors {
        if alive(&state, motor) {
            seeds.extend(parts_of(&state, &setup, motor).into_iter().flatten());
        }
    }
    settle_from_roots(&mut state, &engine.reflection, &hooks, &setup, &seeds, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A part's list walks the joints marked as in a world in the order they were put in, wherever they stand among 20,000 places
    /// (blocks of 4,096 places and the words in them), as joints are put in, marked and taken out at random and the holes close,
    /// each joint keeping its place.
    #[test]
    fn a_list_walks_its_joints_in_a_world_in_order() {
        let part = InstanceId { index: 0, generation: 0 };
        let mut list = PartJoints::default();
        let mut indexed: FxHashMap<InstanceId, Indexed> = FxHashMap::default();
        // (the joints in order, and whether each is in a world)
        let mut model: Vec<(InstanceId, bool)> = Vec::new();
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        for round in 0..60_000u32 {
            let roll = if round < 20_000 { 0 } else { next(4) };
            if roll <= 1 || model.is_empty() {
                let joint = InstanceId { index: round + 1, generation: 0 };
                let homed = next(3) == 0;
                let place = list.push(joint, homed);
                indexed.insert(joint, Indexed { parts: [Some(part), None], places: [place, 0], taken: 0 });
                model.push((joint, homed));
            } else {
                let index = next(model.len());
                let joint = model[index].0;
                let place = indexed[&joint].places[0] as usize;
                assert_eq!(list.slots[place], Some(joint), "round {round}: the joint's place");
                if roll == 2 {
                    let homed = next(2) == 0;
                    list.mark(place, homed);
                    model[index].1 = homed;
                } else {
                    list.remove_at(place);
                    indexed.remove(&joint);
                    model.remove(index);
                    if list.slots.len() > 2 * list.live {
                        list.close_holes(part, &mut indexed);
                    }
                }
            }
            if round % 997 == 0 || round == 59_999 {
                let walked: Vec<InstanceId> = list.homed().collect();
                let want: Vec<InstanceId> = model.iter().filter(|(_, homed)| *homed).map(|&(joint, _)| joint).collect();
                assert_eq!(walked, want, "round {round}: the joints walked");
                assert_eq!(list.live, model.len(), "round {round}: the joints counted");
            }
        }
    }
}
