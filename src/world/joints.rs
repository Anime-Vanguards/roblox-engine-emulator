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
//! joints_entering, [`entering_price`]): each joint in a world it looks at, each link it makes and each joint it brings in, and a
//! link change's price when a link it makes reaches a part that was in the world already.
//! The joints made before the index was first needed are taken in by one look at the dom ([`ensure_scanned`]), the later ones
//! by their Part0 / Part1 and Parent writes, and one made where it stands with no write (a lazy import's) as it is made
//! ([`made_in_place`]), so a lazily made model's welds link as the same model's made with the place.
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
//! velocity write each other part of the assembly. A change of the links (a joint enabled, disabled, re-pointed, taken into or out
//! of its world, or freed) splits or joins the kept components instead of walking them again, and in a settled component
//! places only what it moves ([`link_changed`]); it pays what a client spends ([`charge_link`]: each
//! assembly's first change in a frame by its parts, as a client's). A move works the centres of mass read
//! since the last one out again, so a read after it walks nothing ([`refresh_centres`]).
//!
//! AssemblyMass and AssemblyCenterOfMass are the assembly's, not the component's ([`assembly_body`],
//! Studio 0.747, client and server): inf for an assembly holding an anchored part (from the Anchored
//! write's own frame), its centre still its parts' worked out about the root, so each anchored part
//! welded to another reads its own assembly's; a Massless part weighs nothing but as its assembly's root
//! or outside every world. A velocity write sets the assembly's motion as a client keeps it, the
//! velocity of the root's position and the spin (world/gravity.rs assign), through the centre's offset
//! from the root ([`assembly_frame`]).
//!
//! Beside the index, each part keeps a list of its active joints of the kinds Workspace:UnjoinFromOutsiders takes out
//! ([`Joints::attached`]: the rigid JointInstances), in the order a client keeps it (Studio 0.748, through UnjoinFromOutsiders'
//! order: [`refresh_attached`], [`taken_up`]). A Parent write pays what a client spends taking joints out of those lists
//! ([`left_every_world`]; on a move between worlds too: [`relist_moved`]).
//!
//! A legacy Motor turns by its CurrentAngle toward its DesiredAngle ([`legacy_angle`], [`step_motors`]).
//! Not modelled: AnimationConstraint,
//! RigidConstraint and the other constraints (solver-driven, nothing is simulated), and the members
//! that report assemblies other than AssemblyRootPart (GetRootPart, GetConnectedParts,
//! JointInstance.Active).

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};

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

impl Kind {
    /// Whether it links parts into an assembly.
    fn rigid(self) -> bool {
        matches!(self, Kind::Joint | Kind::WeldConstraint)
    }

    /// Whether it is a JointInstance Workspace:UnjoinFromOutsiders takes out, which a part keeps in its list of active joints
    /// ([`Joints::attached`]): every one but Rotate, RotateP, RotateV and VelocityMotor (Studio 0.748).
    fn unjoined(self) -> bool {
        self == Kind::Joint
    }
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
    /// The legacy Motor (and Motor6D under it) and its angles ([`legacy_angle`], [`step_motors`])
    motor: Option<ClassId>,
    current_angle: Option<NameId>,
    desired_angle: Option<NameId>,
    max_velocity: Option<NameId>,
    world_root: ClassId,
    workspace: Option<ClassId>,
    joints_service: Option<ClassId>,
    seats: Vec<ClassId>,
    /// Kind by class id
    kinds: Vec<Kind>,
    /// The order a client takes each class's links in when it joins parts to anchored ones, by class id ([`claims`])
    ranks: Vec<u8>,
}

impl Setup {
    fn kind(&self, class: ClassId) -> Kind {
        self.kinds.get(class as usize).copied().unwrap_or(Kind::Other)
    }

    /// Where a joint of `class` comes in [`claims`]' order: a Weld, ManualWeld, Snap, Glue or ManualGlue 0, a WeldConstraint 1, a
    /// Motor 2, a Motor6D 3.
    fn union_rank(&self, class: ClassId) -> u8 {
        self.ranks.get(class as usize).copied().unwrap_or(0)
    }
}

/// The assembly state of one engine (in [`State`]).
#[derive(Default)]
pub struct Joints {
    setup: Option<Rc<Setup>>,
    /// Joints the index keeps (rigid ones and WeldConstraints) made in this engine: none, and every entry
    /// point returns at once
    pub(crate) created: u32,
    /// Whether the joints made before the index existed have been indexed
    scanned: bool,
    /// Those joints by the parts they named when last indexed (candidates: a joint's current Part0
    /// and Part1 are checked when it is used), each part's in the order they were indexed under it
    by_part: FxHashMap<InstanceId, PartJoints>,
    /// Each indexed joint's parts and its place in each one's list
    indexed: FxHashMap<InstanceId, Indexed>,
    /// How many times the dom's log of homes was taken ([`take_homes`])
    takes: u64,
    /// The last number given to a joint's naming of a part ([`Indexed::named`])
    naming: u64,
    /// Each part's active joints of the kinds UnjoinFromOutsiders takes out ([`Kind::unjoined`]), in the order a client keeps
    /// them: a joint goes to the end of each of its parts' lists when it becomes active or its parts change while it is, and
    /// leaves by the list's last joint taking its place ([`refresh_attached`]; Studio 0.748, through UnjoinFromOutsiders' order)
    attached: FxHashMap<InstanceId, Vec<InstanceId>>,
    /// The parts whose lists each of those joints is in, with its place in each (so a joint leaves a list without a search)
    attached_parts: FxHashMap<InstanceId, [(InstanceId, u32); 2]>,
    /// Indexed joints whose links must be worked out again before the next walk ([`relink`]): their homes or their parts' changed,
    /// or a part they name was freed
    relinks: Vec<InstanceId>,
    /// The same joints, whose place in the lists of active joints must be worked out again ([`list_logged`]): a Parent write's walk
    /// puts its own in order first ([`reparented`]), and these catch the moves the engine makes in the dom without one
    unlisted: Vec<InstanceId>,
    /// How many Parent writes' walks changed the lists ([`relist_moved`]); the last marks the joints it took up ([`Indexed::listed`])
    walks: u64,
    /// While positive, a link change that ends a single link charges no flat link-change price ([`charge_link`]: LINK_CHANGED,
    /// LINK_SPLIT_FIRST, LINK_SPLIT_AGAIN, LINK_REJOINED), its frame's and assemblies' prices as ever:
    /// Workspace:UnjoinFromOutsiders' removals, whose flat prices its own price covers, as a client's measured cost of the call
    /// does (cost.rs UNJOIN_REMOVED; [`Unpriced`] holds it up). A removal that ends several links at once (a joint carrying others)
    /// pays every link price.
    unpriced: u32,
    /// The components found, by each of their parts ([`component`]), kept until something they depend on changes ([`stored`],
    /// [`drop_component`])
    components: FxHashMap<InstanceId, Rc<Component>>,
    /// What a property written to a part means to its kept component ([`stored`]), by name id: [`Watch`]
    watch: Vec<u8>,
    /// The assemblies an Anchored write took out of its part's kept component, with the part and whether the component was
    /// settled ([`stored`], [`reanchored`])
    before_anchor: Option<(InstanceId, Option<Rc<Assemblies>>, bool)>,
    /// While positive, the parts' CFrame writes are the assembly code's own placing ([`settle`]), which leaves a component settled
    placing: u32,
    /// The part whose CFrame write last unsettled its component, which was settled until then ([`stored`]; [`part_moved`] settles
    /// the moved assembly again)
    unsettled: Option<InstanceId>,
    /// The frame of the last priced link change ([`charge_link`]), and the parts of the assemblies whose first link change of that
    /// frame was charged, each with whether its assembly held an anchored part: each assembly's parts are marked together
    link_frame: Option<u64>,
    marked: FxHashMap<InstanceId, bool>,
    /// The joints that link each pair of parts now (the pair in id order), by the order they first named the pair ([`pair_order`]),
    /// kept as their marks change ([`Joints::sync_pair`]): a single link change finds the other joints of its pair here
    /// ([`parallel_links`]) at the same cost however many joints its parts hold
    pairs: FxHashMap<[InstanceId; 2], PairLinks>,
    /// That frame's priced link changes that ended links and that started links, and whether one that started a link after one
    /// that ended one was charged ([`charge_link`])
    splits: u32,
    joins: u32,
    rejoined: bool,
    /// The parts of that frame's link changes whose assemblies no read has paid for working their bodies out again since
    /// ([`pay_body`])
    owed: FxHashSet<InstanceId>,
    /// While positive, the parts' CFrame writes are BulkMoveTo's, which pay nothing for the joints between assemblies ([`bulk_moves`])
    bulk: u32,
    /// Motor6Ds whose Transform changed since the last simulation step
    pending: Vec<InstanceId>,
    /// Motor6Ds whose Transform was ever written: their CurrentAngle no longer turns them ([`legacy_angle`])
    transformed: FxHashSet<InstanceId>,
    /// Motors whose CurrentAngle may still turn toward their DesiredAngle ([`step_motors`])
    turning: FxHashSet<InstanceId>,
    /// While positive, CFrame and joint writes are the assembly code's own (already consistent)
    quiet: u32,
    /// Model:PivotTo / ScaleTo: nesting depth, the parts they wrote (in order, with the CFrame each
    /// had before) and the parts of the joints they changed
    batch: u32,
    batch_parts: Vec<(InstanceId, Affine3A)>,
    batch_seeds: Vec<InstanceId>,
    /// While some part moves (world/gravity.rs `moving`: a velocity, a spin or its body's motion not zero), the parts whose assemblies a
    /// change may have joined, split or rooted elsewhere since gravity last looked ([`Joints::regroup`]), which then take their roots'
    /// motion (gravity::assemblies_changed, after each write: [`property_changed`], [`reparented`])
    pub(crate) motion: bool,
    pub(crate) regrouped: Vec<InstanceId>,
}

/// The joints indexed under one part (the rigid ones and WeldConstraints: [`link_in`]), in the order
/// they were indexed under it. A joint taken out leaves a hole at its
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
    /// When each side last named its part ([`Joints::naming`]): a side named anew when that side's part changes, not when the
    /// other side's does, as a client orders a part's joints (Studio 0.748: x1 of a part's joints x1 and x3, its other part
    /// re-pointed and back, still counts as named before x3; its own side re-pointed and back, or the two sides swapped, after)
    named: [u64; 2],
    /// The last walk of a Parent write's subtree that took it up ([`Joints::walks`]): [`list_logged`] passes it by right after
    listed: u64,
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
/// something it depends on changes: a link among them (a joint's parts, home, a part's world or class, one freed: [`relink`]),
/// a part's Massless, RootPriority, Size or Name ([`stored`]). So reading an assembly, or moving it, walks no joint while nothing
/// changed, as a playing client keeps its assemblies (a read costs it the same at any joint count). A joint enabled, disabled or
/// freed, which links or unlinks two parts, splits or joins the kept components instead ([`link_changed`], [`Joints::unlink`],
/// [`Joints::join`]), and an Anchored write changes its assemblies alone.
struct Component {
    /// Each part (the keys: its parts, in no order that matters; a walk's order is worked out from the lists: [`order_from`]) and
    /// its joints that link it to another part, with that part, in the order its list holds them ([`edges`]' answer)
    links: RefCell<FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>>>,
    /// Its anchored parts
    anchored: RefCell<FxHashSet<InstanceId>>,
    /// Its root when none of its parts is anchored, once known: the best ranked part (its tree's root: [`assemblies_of`]; kept as
    /// links join or split it)
    best: Cell<Option<InstanceId>>,
    /// Every part is where its assemblies' joints put it from their roots (within float noise: [`settled`]), as the last settle
    /// of all of them left it, and nothing has moved one since: a link change that leaves a part's path from its root as it was has
    /// nothing to place there ([`link_changed`])
    settled: Cell<bool>,
    /// No WeldConstraint among its links waits to take its parts' relative CFrame (a walk that reads a root makes them take it:
    /// [`assemblies_of`]), so a root read from [`Component::single_root`] skips nothing a tree's would do
    derived: Cell<bool>,
    /// Its assemblies, once worked out ([`assemblies_of`])
    tree: RefCell<Option<Rc<Assemblies>>>,
    /// Each part's assembly, kept from its last tree through the link changes since that left the paths from their roots as they
    /// were ([`KeptRoots`]), while it has no tree
    roots: RefCell<Option<KeptRoots>>,
    /// What was worked out from its parts ([`Memo`])
    memo: RefCell<Memo>,
}

/// Each part's assembly in a component, read from a tree worked out before the link changes since, which each left the paths of the
/// component's parts from their roots as they were: a side that held no root split off ([`Joints::unlink`]: its parts were all in
/// the assembly of its end of the link, below it), or a side that held no anchored part hung from a part of it ([`attach`]). So
/// AssemblyRootPart and its size, and the path a join hangs its other side from, are read without a tree after such changes, as a
/// client's root read after a link change costs the same at any size (Studio 0.747: 2.1 us with 10,000 parts and two anchored).
struct KeptRoots {
    tree: Rc<Assemblies>,
    /// The parts that joined since, each with its assembly in `tree`
    joined: FxHashMap<InstanceId, u32>,
    /// Each assembly's parts now less its parts in `tree`
    sizes: Vec<isize>,
}

impl KeptRoots {
    fn of(tree: Rc<Assemblies>) -> Self {
        let sizes = vec![0; tree.list.len()];
        KeptRoots { tree, joined: FxHashMap::default(), sizes }
    }

    /// The assembly `part` (one of the component's parts) is in
    fn assembly(&self, part: InstanceId) -> Option<u32> {
        self.joined.get(&part).copied().or_else(|| self.tree.place.get(&part).map(|&(assembly, _)| assembly))
    }

    /// The root of the assembly `part` is in
    fn root(&self, part: InstanceId) -> Option<InstanceId> {
        self.assembly(part).map(|assembly| self.tree.list[assembly as usize].nodes[0].part)
    }

    /// How many parts the assembly `part` is in holds
    fn size(&self, part: InstanceId) -> Option<usize> {
        let assembly = self.assembly(part)? as usize;
        Some((self.tree.list[assembly].nodes.len() as isize + self.sizes[assembly]) as usize)
    }

    /// `count` parts left (`count` negative) or joined the assembly of `part`
    fn resize(&mut self, part: InstanceId, count: isize) -> Option<()> {
        let assembly = self.assembly(part)? as usize;
        self.sizes[assembly] += count;
        Some(())
    }
}

/// The joints from its root to `part` in `tree`, each with the part it hangs from, the root's first (a part of the tree that joined
/// nothing since: its path is the tree's).
fn tree_path(tree: &Assemblies, part: InstanceId) -> Option<Vec<(InstanceId, InstanceId)>> {
    let (assembly, _, node) = tree.of(part)?;
    let mut path = Vec::new();
    let mut at = node;
    while let Some((parent, link)) = assembly.nodes[at].parent {
        path.push((link, assembly.nodes[parent].part));
        at = parent;
    }
    path.reverse();
    Some(path)
}

impl Component {
    fn new(links: FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>>, anchored: FxHashSet<InstanceId>) -> Self {
        Component {
            links: RefCell::new(links),
            anchored: RefCell::new(anchored),
            best: Cell::new(None),
            settled: Cell::new(false),
            derived: Cell::new(false),
            tree: RefCell::default(),
            roots: RefCell::default(),
            memo: RefCell::default(),
        }
    }

    /// Its parts' assemblies as kept: from its tree, else as [`KeptRoots`] kept them (taken: a link change gives them to the side
    /// whose paths stay)
    fn take_roots(&self) -> Option<KeptRoots> {
        match self.tree.borrow_mut().take() {
            Some(tree) => Some(KeptRoots::of(tree)),
            None => self.roots.borrow_mut().take(),
        }
    }

    /// The root of the assembly `part` is in, when known without working a tree out: its tree's, else the kept roots'
    fn kept_root(&self, part: InstanceId) -> Option<InstanceId> {
        if let Some(tree) = self.tree.borrow().as_ref() {
            return tree.of(part).map(|(assembly, ..)| assembly.nodes[0].part);
        }
        self.roots.borrow().as_ref().and_then(|roots| roots.root(part))
    }

    /// The joints from its root to `part` ([`tree_path`]), when known without working a tree out
    fn kept_path(&self, part: InstanceId) -> Option<Vec<(InstanceId, InstanceId)>> {
        if let Some(tree) = self.tree.borrow().as_ref() {
            return tree_path(tree, part);
        }
        let roots = self.roots.borrow();
        let roots = roots.as_ref()?;
        if roots.joined.contains_key(&part) {
            return None;
        }
        tree_path(&roots.tree, part)
    }

    /// How many parts it holds
    fn size(&self) -> usize {
        self.links.borrow().len()
    }

    /// Its parts
    fn parts(&self) -> Vec<InstanceId> {
        self.links.borrow().keys().copied().collect()
    }

    /// The root of all its parts when it is one assembly whose root is known without its tree: its one anchored part, or the best
    /// ranked when none is anchored
    fn single_root(&self) -> Option<InstanceId> {
        let anchored = self.anchored.borrow();
        match anchored.len() {
            0 => self.best.get(),
            1 => anchored.iter().next().copied(),
            _ => None,
        }
    }

    /// Forgets what was worked out from its links: its assemblies and what its reads kept
    fn relinked(&self) {
        *self.tree.borrow_mut() = None;
        *self.roots.borrow_mut() = None;
        *self.memo.borrow_mut() = Memo::default();
    }
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

/// What the gravity step and the assembly reads work out from a component's parts, from each part asked: linked_parts' answer, the
/// parts of its assembly ([`members`]), and AssemblyMass and AssemblyCenterOfMass ([`mass_centre`] over those), kept for each part
/// of the assembly at once. The masses are forgotten when a part's Size, Massless, Material, CustomPhysicalProperties, Shape or
/// Anchored changes (an Anchored write the assemblies with them); a part placed makes the centres stale ([`stored`]), and a move
/// works out again those read since the last one ([`refresh_centres`]).
#[derive(Default)]
struct Memo {
    linked: FxHashMap<InstanceId, Rc<[InstanceId]>>,
    /// Each part's assembly: its parts, the root first, each after its parent ([`members`])
    members: FxHashMap<InstanceId, Rc<[InstanceId]>>,
    mass: FxHashMap<InstanceId, f32>,
    centre: FxHashMap<InstanceId, Centre>,
    /// Each part's mass as its assembly counts it (none for a Massless part but its root), as the centres were worked out from them
    masses: FxHashMap<InstanceId, f32>,
    /// How many times one of its parts was placed: a centre worked out at another count is stale
    placed: u64,
    /// The parts whose centre was read since the last move ([`Centre::read`]), which the next move works out again
    /// ([`refresh_centres`]), so a move looks at those alone however many parts' centres are kept
    reads: Vec<InstanceId>,
    /// The walks free assemblies' AssemblyMass reads summed the masses over, by the root walked from ([`Body`]), up to
    /// [`KEPT_BODIES`]: kept through the link changes whose walk is the same order with parts gone or after it ([`Joints::unlink`],
    /// [`Joints::join`])
    bodies: FxHashMap<InstanceId, Body>,
}

/// How many parts' walks a component keeps for its AssemblyMass reads ([`Memo::bodies`])
const KEPT_BODIES: usize = 8;

/// What a free assembly's AssemblyMass read summed ([`assembly_body`], kept by its root: an assembly with an anchored part reads inf and
/// sums nothing): the parts in the order a walk from the root takes them up ([`walk_order`]), each with the part that took it up (its
/// index in the order) and by which joint (none for the first), its mass as the assembly counts it once worked out, and the sums of
/// those masses in turn up to some part (the read's answer is the last, as [`mass_centre`] adds them). Kept through the link changes
/// after which a walk from the part takes the same parts up in the same order but for some gone and some after them: a
/// side split off is marked gone where it was ([`Body::cut`]: a walk of the parts left takes them up in the order they had); a side
/// joined by a joint the walk takes up after every part of the order follows it ([`Body::extend`]); a side split off comes back where
/// it was when the joint it left by links it again at the same place and nothing followed the order since ([`Body::restore`]); a side
/// of one part joined anywhere else goes where the walk takes it up, the later parts' places moving up by one ([`Body::insert_one`]).
/// The sums from the first part a change touched on are worked out again at the next read from the masses kept, a few ns a part, as a
/// client's mass read after a link change stays flat (Studio 0.747: 0.3 to 1.1 us at 1,000 to 10,000 parts, an anchored assembly's;
/// a free chain's too but for a join in its middle).
/// A body with more parts gone than kept is dropped, and worked out again at the next read.
#[derive(Default)]
struct Body {
    order: Vec<InstanceId>,
    by: Vec<(u32, Option<InstanceId>)>,
    own: Vec<Option<f32>>,
    gone: Vec<bool>,
    /// Each part's index in `order` (the one where it is, for a part that left and came back after it)
    at: FxHashMap<InstanceId, u32>,
    /// The sums of the masses of the parts not gone, in turn, each at its part's index, up to the first part changed since
    sums: Vec<f32>,
    live: usize,
    /// The sides split off that may come back where they were: the part at their end, the joint and its place under the part it
    /// linked them to, and the order's length then
    cuts: Vec<(InstanceId, InstanceId, u32, usize)>,
}

impl Body {
    /// A walk's parts ([`walk_order`]) and their own masses, summed in turn
    fn new(walk: Vec<(InstanceId, u32, Option<InstanceId>)>, own: &[f32]) -> Self {
        let mut sum: f32 = std::iter::empty::<f32>().sum();
        let sums = own
            .iter()
            .map(|&mass| {
                sum += mass;
                sum
            })
            .collect();
        let at = walk.iter().enumerate().map(|(index, &(part, ..))| (part, index as u32)).collect();
        let (order, by) = walk.into_iter().map(|(part, by, link)| (part, (by, link))).unzip();
        Body { order, by, own: own.iter().map(|&mass| Some(mass)).collect(), gone: vec![false; own.len()], at, sums, live: own.len(), cuts: Vec::new() }
    }

    /// The parts of `gone` split off, the side whose end `end` was linked by `joint` at `place` under the other end: each is marked
    /// gone where it is (false when one is not there, or when more are gone than kept: the body is no longer kept).
    fn cut<T>(&mut self, gone: &FxHashMap<InstanceId, T>, end: InstanceId, joint: InstanceId, place: u32) -> bool {
        let mut first = usize::MAX;
        for part in gone.keys() {
            let Some(&index) = self.at.get(part) else { return false };
            let index = index as usize;
            if self.gone[index] {
                return false;
            }
            self.gone[index] = true;
            first = first.min(index);
        }
        self.live -= gone.len();
        self.sums.truncate(first);
        self.cuts.push((end, joint, place, self.order.len()));
        2 * self.live >= self.order.len()
    }

    /// Where the walk takes up `joint` at `end` (one of its parts, the joint's place in its list `place`, `places` giving another
    /// joint's place under it): when after every part of the order not gone (`end` the last, or the part that took up the last, by a
    /// joint before this one), the index of `end`, so the other side's parts follow the order's in a walk of the joined parts.
    fn joins_after(&self, end: InstanceId, place: u32, places: impl Fn(InstanceId) -> u32) -> Option<u32> {
        let last = (0..self.order.len()).rev().find(|&index| !self.gone[index])?;
        if self.order[last] == end {
            return Some(last as u32);
        }
        let (by, link) = self.by[last];
        (self.order[by as usize] == end && link.is_some_and(|link| places(link) < place)).then_some(by)
    }

    /// The other side's parts, in the order a walk from its end of the joint takes them up (`side`, with the part that took up each
    /// one there and by which joint), follow the order's, the first taken up by `end` at index `at`.
    fn extend(&mut self, at: u32, joint: InstanceId, side: &[(InstanceId, u32, Option<InstanceId>)]) {
        let base = self.order.len() as u32;
        for (index, &(part, by, link)) in side.iter().enumerate() {
            self.at.insert(part, self.order.len() as u32);
            self.order.push(part);
            self.by.push(if index == 0 { (at, Some(joint)) } else { (base + by, link) });
            self.own.push(None);
            self.gone.push(false);
        }
        self.live += side.len();
    }

    /// The side `side` (as [`extend`](Body::extend) takes it) linked by `joint` at `place` under `end` comes back where it was: when it
    /// left by that joint at that place, nothing followed the order since, and each of its parts is gone and was taken up by the part
    /// and joint that take it up now. Whether it came back.
    fn restore(&mut self, end: InstanceId, joint: InstanceId, place: u32, side: &[(InstanceId, u32, Option<InstanceId>)]) -> bool {
        let length = self.order.len();
        let Some(cut) = self.cuts.iter().rposition(|&(at, link, under, then)| at == side[0].0 && link == joint && under == place && then == length) else {
            return false;
        };
        let mut indices = Vec::with_capacity(side.len());
        for (index, &(part, by, link)) in side.iter().enumerate() {
            let Some(&at) = self.at.get(&part) else { return false };
            let want = if index == 0 { self.at.get(&end).map(|&at| (at, Some(joint))) } else { Some((indices[by as usize], link)) };
            if !self.gone[at as usize] || Some(self.by[at as usize]) != want {
                return false;
            }
            indices.push(at);
        }
        for &at in &indices {
            self.gone[at as usize] = false;
        }
        self.live += indices.len();
        self.sums.truncate(indices.iter().min().map_or(0, |&at| at as usize));
        self.cuts.remove(cut);
        true
    }

    /// A side of one part (`side`, as [`extend`](Body::extend) takes it) linked by `joint` at `place` under `end` (`places` giving
    /// a joint's place under a part) joins where a walk of the joined parts takes it up: after the parts not gone that parts before
    /// `end` took up, and those `end` took up by joints before this one; before the rest, whose indices move up by one. The masses are
    /// added again from it on at the next read; the sides split off no longer come back where they were ([`restore`](Body::restore)).
    /// Whether it joined (`end` in the order, not gone).
    fn insert_one(&mut self, end: InstanceId, joint: InstanceId, place: u32, side: &[(InstanceId, u32, Option<InstanceId>)], places: impl Fn(InstanceId, InstanceId) -> u32) -> bool {
        let &[(part, ..)] = side else { return false };
        let Some(&from) = self.at.get(&end).filter(|&&from| !self.gone[from as usize]) else { return false };
        // (the walk's order is by the index of the part that took each up, then by the joint's place under it)
        let after = |index: usize| {
            let (by, link) = self.by[index];
            by > from || by == from && link.is_some_and(|link| places(link, end) > place)
        };
        let at = (from as usize + 1..self.order.len()).find(|&index| !self.gone[index] && after(index)).unwrap_or(self.order.len());
        let Body { order, by, own, gone, at: index_of, sums, live, cuts } = self;
        order.insert(at, part);
        by.insert(at, (from, Some(joint)));
        own.insert(at, None);
        gone.insert(at, false);
        for entry in &mut by[at + 1..] {
            if entry.0 >= at as u32 {
                entry.0 += 1;
            }
        }
        for (index, other) in order.iter().enumerate().skip(at + 1) {
            if let Some(kept) = index_of.get_mut(other)
                && *kept == index as u32 - 1
            {
                *kept = index as u32;
            }
        }
        index_of.insert(part, at as u32);
        *live += 1;
        sums.truncate(at);
        cuts.clear();
        true
    }

    /// The sum of the own masses of the parts not gone, in turn (each worked out by `own_of` once), from the last sum kept on.
    fn mass(&mut self, own_of: impl Fn(InstanceId) -> f32) -> f32 {
        let mut sum = self.sums.last().copied().unwrap_or_else(|| std::iter::empty::<f32>().sum());
        for index in self.sums.len()..self.order.len() {
            if !self.gone[index] {
                let part = self.order[index];
                sum += *self.own[index].get_or_insert_with(|| own_of(part));
            }
            self.sums.push(sum);
        }
        sum
    }

    /// The parts not gone, in order
    fn parts(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.order.iter().zip(&self.gone).filter(|(_, gone)| !**gone).map(|(&part, _)| part)
    }
}

/// A component's parts in the order a walk from `start` takes them up ([`order_from`]), each with the part that took it up (its index)
/// and by which joint (none for `start`), not going past `stop` (a part already taken up elsewhere: none for a whole component).
fn walk_order(links: &FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>>, start: InstanceId, stop: Option<InstanceId>) -> Vec<(InstanceId, u32, Option<InstanceId>)> {
    let mut order = vec![(start, 0, None)];
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    seen.insert(start);
    seen.extend(stop);
    let mut index = 0;
    while index < order.len() {
        for &(joint, other) in links.get(&order[index].0).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(other) {
                order.push((other, index as u32, Some(joint)));
            }
        }
        index += 1;
    }
    order
}

/// A part's AssemblyCenterOfMass as worked out at a count of placings ([`Memo::placed`]), its offset from the assembly's root (what
/// a velocity write converts through: world/gravity.rs assign), and whether it was read since.
#[derive(Clone, Copy)]
struct Centre {
    at: glam::Vec3,
    offset: glam::Vec3,
    placed: u64,
    read: bool,
}

/// How many stale centres of mass a move of an assembly works out again ([`refresh_centres`]): those read since the last move, up to
/// this many; the others are worked out when read.
const REFRESHED_CENTRES: usize = 8;

/// The mass of `points` (each part's position and mass as its assembly counts it, the root first, as [`members`] orders them) and
/// its centre of mass's offset from the first point: each part's offset from the root times its mass, summed, times the inverse of
/// the mass, as a client works it out (Studio 0.747, tools/parity/probes/assembly_mass.luau: a velocity written and an angular one
/// after it read back 1.99999988 for 2, which a quotient does not round to). About the root, so far from the origin the float's step
/// does not show (3e-5 studs at 400 up) and every part of an assembly reads the same centre; the plain average when the parts weigh
/// nothing. World/gravity.rs body_of works the same out, from the same points.
pub(crate) fn mass_centre<I: Iterator<Item = (glam::Vec3, f32)> + Clone>(points: I) -> (f32, glam::Vec3) {
    let mass: f32 = points.clone().map(|(_, mass)| mass).sum();
    let origin = points.clone().next().map(|(at, _)| at).unwrap_or(glam::Vec3::ZERO);
    let offset = if mass > 0.0 {
        points.map(|(at, part_mass)| (at - origin) * part_mass).sum::<glam::Vec3>() * (1.0 / mass)
    } else {
        let count = points.clone().count();
        points.map(|(at, _)| at - origin).sum::<glam::Vec3>() / count.max(1) as f32
    };
    (mass, offset)
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
    // The rigid JointInstances: Glue and ManualGlue link and place their parts as Welds do (Studio 0.747, a playing client and its
    // server: a part glued to an anchored part reads it as its AssemblyRootPart, takes the place C0 and C1 give it, moves with it and
    // adds to its AssemblyMass; tools/parity/probes/assembly_roots.luau); Rotate, RotateP, RotateV and VelocityMotor link nothing
    let rigid: Vec<ClassId> = ["Weld", "Snap", "ManualWeld", "Motor", "Glue", "ManualGlue"].iter().filter_map(|text| class(text)).collect();
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
    let (motor, motor6d) = (class("Motor"), class("Motor6D"));
    let ranks = reflection
        .classes
        .iter()
        .map(|info| {
            if info.is_a(weld_constraint) {
                1
            } else if motor6d.is_some_and(|motor6d| info.is_a(motor6d)) {
                3
            } else if motor.is_some_and(|motor| info.is_a(motor)) {
                2
            } else {
                0
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
        motor6d,
        motor,
        current_angle: name("CurrentAngle"),
        desired_angle: name("DesiredAngle"),
        max_velocity: name("MaxVelocity"),
        world_root,
        workspace: class("Workspace"),
        joints_service: class("JointsService"),
        seats: ["Seat", "VehicleSeat"].iter().filter_map(|text| class(text)).collect(),
        kinds,
        ranks,
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
    // (which joints the index keeps are in a world, and which parts, is read from the dom's log of their homes: take_homes)
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

/// A count of [`Joints`] raised for some work and lowered when this drops, so a panic in the work (lua/raw.rs guard turns an engine
/// panic into an error a script can catch) leaves it as it was: [`Joints::quiet`] left raised made every later CFrame and joint
/// write the assembly code's own (ignored) and stopped Parent writes linking, [`Joints::placing`] made every CFrame write count as
/// the assembly code's placing. It holds the state the caller holds, no borrow of its own.
struct Raised<'a> {
    state: &'a mut State,
    count: fn(&mut Joints) -> &mut u32,
}

impl<'a> Raised<'a> {
    fn new(state: &'a mut State, count: fn(&mut Joints) -> &mut u32) -> Self {
        *count(&mut state.joints) += 1;
        Raised { state, count }
    }
}

impl Drop for Raised<'_> {
    fn drop(&mut self) {
        let count = (self.count)(&mut self.state.joints);
        *count = count.saturating_sub(1);
    }
}

impl std::ops::Deref for Raised<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        self.state
    }
}

impl std::ops::DerefMut for Raised<'_> {
    fn deref_mut(&mut self) -> &mut State {
        self.state
    }
}

/// [`bulk_moves`] raised for a BulkMoveTo call (world/mod.rs) and lowered when this drops, a panic in the moves included: left raised,
/// no CFrame write would pay for the joints between assemblies ([`crate::cost::JOINT_ACROSS`]). It holds no borrow while the moves
/// run, and takes the state's when it drops (none is held then; when one still is, the count stays).
pub struct BulkMoving<'a>(&'a RefCell<State>);

/// Raises [`bulk_moves`] until the returned guard drops.
pub fn bulk_moving(state: &RefCell<State>) -> BulkMoving<'_> {
    bulk_moves(&mut state.borrow_mut(), true);
    BulkMoving(state)
}

impl Drop for BulkMoving<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            bulk_moves(&mut state, false);
        } else {
            // (a borrow still held would leave the count raised and no later CFrame write paying for the joints between assemblies:
            // never so today)
            debug_assert!(std::thread::panicking(), "BulkMoveTo's count stays raised: the state is still borrowed");
        }
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

    /// Takes `joint` out of the lists of the parts it was indexed under, from its places in them. The components of the parts it
    /// linked go; `keep_up`: a joint that linked its two parts leaves their kept component instead, which splits when nothing else
    /// links its two sides ([`Joints::unlink`]).
    fn unindex(&mut self, joint: InstanceId, old: &Indexed, keep_up: bool) {
        if let [Some(first), Some(second)] = old.parts {
            self.pair_remove([first, second], pair_order(old), joint);
        }
        let linking = |side: usize| {
            let part = old.parts[side]?;
            let place = old.places[side] as usize;
            self.by_part.get(&part).filter(|list| list.slots.get(place) == Some(&Some(joint)) && list.is_linking(place)).map(|_| part)
        };
        let kept_up = keep_up
            && match (linking(0), linking(1)) {
                (Some(a), Some(b)) if a != b => self.unlink(joint, [a, b], old.places).is_some(),
                _ => false,
            };
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
            if linked && !kept_up {
                self.drop_component(part);
            }
        }
    }

    /// Takes `joint` out of its parts' lists of active joints: the last joint of each list takes its place (whose stored place
    /// follows), at the same cost however long the lists are.
    fn detach(&mut self, joint: InstanceId) {
        let Some(places) = self.attached_parts.remove(&joint) else { return };
        for (part, place) in places {
            let Some(list) = self.attached.get_mut(&part) else { continue };
            let place = place as usize;
            debug_assert_eq!(list.get(place), Some(&joint), "a joint's stored place in its part's list");
            list.swap_remove(place);
            if let Some(&moved) = list.get(place)
                && let Some(entry) = self.attached_parts.get_mut(&moved)
            {
                for slot in entry.iter_mut().filter(|slot| slot.0 == part) {
                    slot.1 = place as u32;
                }
            }
            if list.is_empty() {
                self.attached.remove(&part);
            }
        }
    }

    /// Forgets the kept component of `part`, for each of its parts.
    /// Notes that `part`'s assembly may have changed, while a part moves ([`Joints::regrouped`]).
    fn regroup(&mut self, part: InstanceId) {
        if self.motion {
            self.regrouped.push(part);
        }
    }

    fn drop_component(&mut self, part: InstanceId) {
        let Some(found) = self.components.remove(&part) else { return };
        self.unsettled = None;
        for other in found.parts() {
            if self.components.get(&other).is_some_and(|kept| Rc::ptr_eq(kept, &found)) {
                self.components.remove(&other);
            }
        }
    }

    /// The place `joint` holds in `part`'s list (`known`: the place of the joint being taken out, which may no longer be indexed).
    fn place_under(&self, joint: InstanceId, part: InstanceId, known: (InstanceId, u32)) -> u32 {
        if joint == known.0 {
            return known.1;
        }
        self.indexed_place(joint, part)
    }

    /// The place an indexed `joint` holds in `part`'s list.
    fn indexed_place(&self, joint: InstanceId, part: InstanceId) -> u32 {
        let entry = &self.indexed[&joint];
        if entry.parts[0] == Some(part) { entry.places[0] } else { entry.places[1] }
    }

    /// Where `joint` (at `place` in `part`'s list) goes among `list`, a kept component's links of `part` in its list's order.
    fn link_at(&self, list: &[(InstanceId, InstanceId)], part: InstanceId, joint: InstanceId, place: u32) -> usize {
        list.partition_point(|&(other, _)| self.place_under(other, part, (joint, place)) < place)
    }

    /// The link of `joint` between `parts` (at `places` in their lists) stopped: their kept component (both are in the same one, or
    /// None: nothing is changed) drops it from both parts' links and, when nothing else links its two sides, the side found first
    /// to end leaves it as a component of its own (none for one part), each side then walked one link at a time in turn: it costs
    /// the smaller side, not the whole component, as a client's link change does not walk its assembly. Its assemblies and what
    /// its reads kept go; its anchored parts and best ranked part go with their sides.
    fn unlink(&mut self, joint: InstanceId, parts: [InstanceId; 2], places: [u32; 2]) -> Option<()> {
        let found = self.components.get(&parts[0])?.clone();
        if !self.components.get(&parts[1]).is_some_and(|other| Rc::ptr_eq(other, &found)) {
            return None;
        }
        let at = {
            let links = found.links.borrow();
            let mut at = [0; 2];
            for side in 0..2 {
                let list = links.get(&parts[side])?;
                at[side] = self.link_at(list, parts[side], joint, places[side]);
                if list.get(at[side]).map(|&(other, _)| other) != Some(joint) {
                    return None;
                }
            }
            at
        };
        {
            let mut links = found.links.borrow_mut();
            for side in 0..2 {
                links.get_mut(&parts[side]).expect("listed").remove(at[side]);
            }
        }
        self.unlinked(&found, joint, parts, places, &Pending::default());
        Some(())
    }

    /// The links of `ended` (each joint with its two parts, Part0's first, and its places in their lists: the links one write
    /// ended, in turn) stopped: as [`Joints::unlink`] for each in turn, with the same components, splits and kept assemblies, but
    /// each part's kept list loses all of its links that ended at once, so what a write's link changes cost grows with the links,
    /// however many of them one part has (each link taken out of a list moved the rest of the list: a part with 30,000 welds to one
    /// other part taken out of its world took 0.2 to 0.5 s of host time, 30 parts with 1,000 each 0.05). The links not unlinked yet
    /// still link their parts for each split ([`Pending`], [`sides`]). Whether each was unlinked: not when its parts are not both in
    /// one kept component, or it is not at its places in their lists; the components of the parts of those go first, each whole.
    fn unlink_all(&mut self, ended: &[(InstanceId, [InstanceId; 2], [u32; 2])]) -> Vec<bool> {
        if let &[(joint, parts, places)] = ended {
            return vec![self.unlink(joint, parts, places).is_some()];
        }
        // (where each is in its parts' lists, as they are before any leaves: the others leaving moves none of them out)
        let placed = |joints: &Joints, (joint, parts, places): (InstanceId, [InstanceId; 2], [u32; 2])| -> Option<(Rc<Component>, [usize; 2])> {
            let found = joints.components.get(&parts[0])?;
            if !joints.components.get(&parts[1]).is_some_and(|other| Rc::ptr_eq(other, found)) {
                return None;
            }
            let links = found.links.borrow();
            let mut at = [0; 2];
            for side in 0..2 {
                let list = links.get(&parts[side])?;
                at[side] = joints.link_at(list, parts[side], joint, places[side]);
                if list.get(at[side]).map(|&(other, _)| other) != Some(joint) {
                    return None;
                }
            }
            Some((found.clone(), at))
        };
        let mut located: Vec<Option<(Rc<Component>, [usize; 2])>> = ended.iter().map(|&link| placed(self, link)).collect();
        if located.iter().any(Option::is_none) {
            for (&(_, parts, _), at) in ended.iter().zip(&located) {
                if at.is_none() {
                    self.drop_component(parts[0]);
                    self.drop_component(parts[1]);
                }
            }
            for (&(_, parts, _), at) in ended.iter().zip(located.iter_mut()) {
                if at.as_ref().is_some_and(|(found, _)| !self.components.get(&parts[0]).is_some_and(|kept| Rc::ptr_eq(kept, found))) {
                    *at = None;
                }
            }
        }
        // (each part's links that ended, in turn, with their places in its list)
        let mut taken: Vec<(InstanceId, u32, u32)> = Vec::with_capacity(2 * ended.len());
        for (index, at) in located.iter().enumerate() {
            if let Some((_, at)) = at {
                let parts = ended[index].1;
                taken.extend([(parts[0], index as u32, at[0] as u32), (parts[1], index as u32, at[1] as u32)]);
            }
        }
        taken.sort_unstable();
        let mut pending = Pending { ranges: FxHashMap::with_capacity_and_hasher(taken.len(), Default::default()), links: Vec::with_capacity(taken.len()) };
        let mut places: Vec<usize> = Vec::new();
        let mut first = 0;
        while first < taken.len() {
            let part = taken[first].0;
            let last = first + taken[first..].iter().take_while(|&&(other, ..)| other == part).count();
            let found = &located[taken[first].1 as usize].as_ref().expect("located").0;
            let mut links = found.links.borrow_mut();
            let list = links.get_mut(&part).expect("listed");
            // (the list closes up once over its links that left, from the first)
            places.clear();
            places.extend(taken[first..last].iter().map(|&(.., at)| at as usize));
            places.sort_unstable();
            let (mut kept, mut next) = (places[0], 0);
            for place in places[0]..list.len() {
                if places.get(next) == Some(&place) {
                    next += 1;
                    continue;
                }
                list[kept] = list[place];
                kept += 1;
            }
            list.truncate(kept);
            pending.ranges.insert(part, (first as u32, last as u32));
            pending.links.extend(taken[first..last].iter().map(|&(_, index, _)| {
                let (joint, parts, _) = ended[index as usize];
                (joint, if parts[0] == part { parts[1] } else { parts[0] })
            }));
            first = last;
        }
        for (index, &(joint, parts, places)) in ended.iter().enumerate() {
            if located[index].is_none() {
                continue;
            }
            pending.pop(parts[0]);
            pending.pop(parts[1]);
            // (the component both parts are in now: a split before this link leaves them together, as it still linked them)
            let found = self.components.get(&parts[0]).cloned().expect("kept");
            debug_assert!(self.components.get(&parts[1]).is_some_and(|other| Rc::ptr_eq(other, &found)), "both parts of a link not yet unlinked are in one component");
            self.unlinked(&found, joint, parts, places, &pending);
        }
        located.iter().map(Option::is_some).collect()
    }

    /// The link of `joint` between `parts` (at `places` in their lists) left their lists in `found`, their kept component
    /// ([`Joints::unlink`]; `pending`: links that left them too but still link their parts, [`Joints::unlink_all`]): the component
    /// splits when nothing else links its two sides.
    fn unlinked(&mut self, found: &Rc<Component>, joint: InstanceId, parts: [InstanceId; 2], places: [u32; 2], pending: &Pending) {
        let (anchored_before, best_before) = (found.anchored.borrow().len(), found.best.get());
        let settled = found.settled.get();
        let roots = found.take_roots();
        let bodies = std::mem::take(&mut found.memo.borrow_mut().bodies);
        self.unsettled = None;
        let apart = sides(&found.links.borrow(), pending, parts);
        found.relinked();
        // (a side that keeps every root keeps its paths from them, and stays settled, its parts' assemblies kept; the others, and
        // both sides while something else links them, which may change the paths, are not known to be)
        found.settled.set(false);
        let Some(apart) = apart else {
            return;
        };
        // (the side that leaves: its links, anchored parts and best ranked part go with it)
        let mut moved: FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>> = FxHashMap::default();
        let mut moved_anchored: FxHashSet<InstanceId> = FxHashSet::default();
        {
            let mut links = found.links.borrow_mut();
            let mut anchored = found.anchored.borrow_mut();
            for part in &apart {
                if let Some(list) = links.remove(part) {
                    moved.insert(*part, list);
                }
                if anchored.remove(part) {
                    moved_anchored.insert(*part);
                }
            }
        }
        let best = found.best.get();
        if best.is_some_and(|best| moved.contains_key(&best)) {
            found.best.set(None);
        }
        // (the mass reads' walks of the staying side's parts, the side split off marked gone in them)
        let side = usize::from(moved.contains_key(&parts[1]));
        let bodies: FxHashMap<InstanceId, Body> = bodies
            .into_iter()
            .filter(|(part, _)| !moved.contains_key(part))
            .filter_map(|(part, mut body)| body.cut(&moved, parts[side], joint, places[1 - side]).then_some((part, body)))
            .collect();
        found.memo.borrow_mut().bodies = bodies;
        // (which side holds every root: all the anchored parts, else the best ranked; the other side's parts were all in the
        // assembly of the root side's end of the link, below it)
        let (roots_apart, roots_kept) = if anchored_before > 0 {
            (found.anchored.borrow().is_empty(), moved_anchored.is_empty())
        } else {
            (best_before.is_some_and(|best| moved.contains_key(&best)), best_before.is_some_and(|best| found.links.borrow().contains_key(&best)))
        };
        let (end_apart, end_kept) = if moved.contains_key(&parts[0]) { (parts[0], parts[1]) } else { (parts[1], parts[0]) };
        let kept_size = found.size() as isize;
        let mut roots = roots;
        if apart.len() > 1 {
            let piece = Rc::new(Component::new(moved, moved_anchored));
            piece.best.set(best.filter(|best| apart.contains(best)));
            piece.settled.set(settled && roots_apart);
            piece.derived.set(found.derived.get());
            if roots_apart && let Some(mut kept) = roots.take() && kept.resize(end_apart, -kept_size).is_some() {
                *piece.roots.borrow_mut() = Some(kept);
            }
            for part in &apart {
                self.components.insert(*part, piece.clone());
            }
        } else {
            self.components.remove(&apart[0]);
        }
        found.settled.set(settled && roots_kept);
        if roots_kept && let Some(mut kept) = roots && kept.resize(end_kept, -(apart.len() as isize)).is_some() {
            *found.roots.borrow_mut() = Some(kept);
        }
        // (a side of one part is no component)
        if found.size() == 1 {
            for part in found.parts() {
                self.components.remove(&part);
            }
        }
    }

    /// The link of `joint` between `parts` (at `places` in their lists; `anchored`: whether each is) started: their components join,
    /// the smaller taken into the larger (a part in none, whose only link is this one, is a component of one), and the joint goes
    /// into both parts' links at its place. None when a part's component is not kept (nothing is changed). The joined component's
    /// assemblies and what its reads kept go. `fresh`: links joined in turn ([`join_started`]), the parts in none whose links are all
    /// among them, each a component of one at its first. `later`: the joint goes into the lists when they are next needed instead
    /// ([`Joints::insert_later`]), unless the join keeps mass reads' walks, which read them now.
    fn join(&mut self, joint: InstanceId, parts: [InstanceId; 2], places: [u32; 2], anchored: [bool; 2], fresh: Option<&FxHashSet<InstanceId>>, later: Option<&mut Later>) -> Option<Rc<Component>> {
        let kept = parts.map(|part| self.components.get(&part).cloned());
        for side in 0..2 {
            let alone = match fresh {
                Some(fresh) => fresh.contains(&parts[side]),
                None => self.by_part.get(&parts[side]).map_or(0, |list| list.linking) == 1,
            };
            if kept[side].is_none() && !alone {
                return None;
            }
        }
        if !keeping() {
            return None;
        }
        self.unsettled = None;
        let mut joined = None;
        let found = match kept {
            [Some(first), Some(second)] if Rc::ptr_eq(&first, &second) => first,
            [first, second] => {
                let size = |kept: &Option<Rc<Component>>| kept.as_ref().map_or(1, |found| found.size());
                let (into, from, from_part) = if size(&first) >= size(&second) { (first, second, parts[1]) } else { (second, first, parts[0]) };
                let into_part = if from_part == parts[0] { parts[1] } else { parts[0] };
                let alone = |part: InstanceId| {
                    let mut links = FxHashMap::default();
                    links.insert(part, Vec::new());
                    let mut set = FxHashSet::default();
                    if anchored[usize::from(part == parts[1])] {
                        set.insert(part);
                    }
                    let alone = Rc::new(Component::new(links, set));
                    alone.derived.set(true);
                    alone
                };
                // (the mass reads' walks of both sides, each with its side's end of the joint and the other's)
                let mut bodies: Vec<(InstanceId, InstanceId, InstanceId, Body)> = Vec::new();
                for (side, end, other) in [(&into, into_part, from_part), (&from, from_part, into_part)] {
                    if let Some(side) = side {
                        bodies.extend(std::mem::take(&mut side.memo.borrow_mut().bodies).into_iter().map(|(part, body)| (part, end, other, body)));
                    }
                }
                joined = Some(bodies);
                let fresh = into.is_none();
                let into = into.unwrap_or_else(|| alone(into_part));
                let from = from.unwrap_or_else(|| alone(from_part));
                let taken = from.parts();
                {
                    let mut links = into.links.borrow_mut();
                    let mut anchored = into.anchored.borrow_mut();
                    for (part, list) in from.links.borrow_mut().drain() {
                        links.insert(part, list);
                    }
                    anchored.extend(from.anchored.borrow().iter().copied());
                }
                for part in taken.into_iter().chain(fresh.then_some(into_part)) {
                    self.components.insert(part, into.clone());
                }
                into.derived.set(into.derived.get() && from.derived.get());
                into.best.set(None);
                into.settled.set(false);
                into
            }
        };
        match later {
            Some(later) if joined.as_ref().is_none_or(Vec::is_empty) => {
                for side in 0..2 {
                    later.entry(parts[side]).or_default().push((places[side], (joint, parts[1 - side])));
                }
            }
            later => {
                if let Some(later) = later {
                    self.insert_later(std::mem::take(later));
                }
                let mut links = found.links.borrow_mut();
                for side in 0..2 {
                    let list = links.get(&parts[side]).expect("joined");
                    let at = self.link_at(list, parts[side], joint, places[side]);
                    links.get_mut(&parts[side]).expect("joined").insert(at, (joint, parts[1 - side]));
                }
            }
        }
        found.relinked();
        // (a walk that takes the joint up after every part of its order goes on through the other side: its parts follow)
        let mut sides: FxHashMap<InstanceId, Vec<(InstanceId, u32, Option<InstanceId>)>> = FxHashMap::default();
        for (part, end, other, mut body) in joined.into_iter().flatten() {
            let place = places[usize::from(end == parts[1])];
            let side = sides.entry(other).or_insert_with(|| walk_order(&found.links.borrow(), other, Some(end)));
            let kept = body.restore(end, joint, place, side)
                || match body.joins_after(end, place, |link| self.indexed_place(link, end)) {
                    Some(at) => {
                        body.extend(at, joint, side);
                        true
                    }
                    None => false,
                }
                || body.insert_one(end, joint, place, side, |link, under| self.indexed_place(link, under));
            if kept {
                found.memo.borrow_mut().bodies.insert(part, body);
            }
        }
        Some(found)
    }

    /// The joints [`Joints::join`] left for later go into their parts' kept lists, each at its place: each list takes its new ones
    /// in one pass from the first of them, so links joined in turn cost the links, however many of them one part has (each joint
    /// put into a list moved the rest of it).
    fn insert_later(&self, later: Later) {
        for (part, mut new) in later {
            let Some(found) = self.components.get(&part) else { continue };
            let mut links = found.links.borrow_mut();
            let Some(list) = links.get_mut(&part) else { continue };
            new.sort_unstable_by_key(|&(place, _)| place);
            let mut read = list.len();
            list.extend(new.iter().map(|&(_, link)| link));
            let mut write = list.len();
            for &(place, link) in new.iter().rev() {
                while read > 0 && self.indexed_place(list[read - 1].0, part) > place {
                    read -= 1;
                    write -= 1;
                    list[write] = list[read];
                }
                write -= 1;
                list[write] = link;
            }
        }
    }
}

/// The joints [`Joints::join`] leaves for [`Joints::insert_later`]: each part's, with its place in the part's list and its link.
type Later = FxHashMap<InstanceId, Vec<(u32, (InstanceId, InstanceId))>>;

/// The links of one write that ended and already left their parts' kept lists, but are not unlinked yet ([`Joints::unlink_all`]):
/// each part's, each joint with the part it linked it to, in the order they will be unlinked. Until then they still link their parts
/// for the splits ([`sides`]).
#[derive(Default)]
struct Pending {
    /// Each part's links in `links`: from the next to be unlinked to its last
    ranges: FxHashMap<InstanceId, (u32, u32)>,
    links: Vec<(InstanceId, InstanceId)>,
}

impl Pending {
    /// `part`'s links still to be unlinked
    fn of(&self, part: InstanceId) -> &[(InstanceId, InstanceId)] {
        self.ranges.get(&part).map_or(&[], |&(next, last)| &self.links[next as usize..last as usize])
    }

    /// `part`'s next link to be unlinked is unlinked now
    fn pop(&mut self, part: InstanceId) {
        if let Some((next, _)) = self.ranges.get_mut(&part) {
            *next += 1;
        }
    }
}

/// The two sides of a component after a link between `parts` ended: None while something else still links them, else the side
/// whose walk ended first, in its walk's order from its end of the link. Each side is walked one link at a time in turn, so the
/// answer costs about twice the smaller side, however large the other. `pending`: links still linking parts beyond their lists,
/// each part's taken up after its list's (which side ends first follows how many links each side holds, not their order).
fn sides(links: &FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>>, pending: &Pending, parts: [InstanceId; 2]) -> Option<Vec<InstanceId>> {
    struct Side {
        order: Vec<InstanceId>,
        seen: FxHashSet<InstanceId>,
        node: usize,
        link: usize,
    }
    let start = |part: InstanceId| {
        let mut seen = FxHashSet::default();
        seen.insert(part);
        Side { order: vec![part], seen, node: 0, link: 0 }
    };
    let mut sides = [start(parts[0]), start(parts[1])];
    loop {
        for turn in 0..2 {
            let (first, second) = sides.split_at_mut(1);
            let (side, other) = if turn == 0 { (&mut first[0], &second[0]) } else { (&mut second[0], &first[0]) };
            loop {
                let Some(&at) = side.order.get(side.node) else {
                    return Some(std::mem::take(&mut side.order));
                };
                let list = links.get(&at).map(Vec::as_slice).unwrap_or_default();
                let link = if side.link < list.len() { list.get(side.link) } else { pending.of(at).get(side.link - list.len()) };
                if let Some(&(_, next)) = link {
                    side.link += 1;
                    if other.seen.contains(&next) {
                        return None;
                    }
                    if side.seen.insert(next) {
                        side.order.push(next);
                    }
                    break;
                }
                side.node += 1;
                side.link = 0;
            }
        }
    }
}

/// An instance was freed: a joint leaves its parts' lists (and, when it linked them, their kept component, which splits when nothing
/// else links its sides: [`Joints::unlink`]), and a part's list goes, its joints' other parts to be worked out again ([`relink`]).
fn forget(state: &mut State, id: InstanceId) {
    let joints = &mut state.joints;
    if let Some(old) = joints.indexed.remove(&id) {
        joints.unindex(id, &old, true);
    }
    if let Some(list) = joints.by_part.remove(&id) {
        joints.relinks.extend(list.slots.iter().flatten());
        joints.drop_component(id);
    }
    joints.detach(id);
    for joint in joints.attached.remove(&id).unwrap_or_default() {
        joints.detach(joint);
    }
    joints.transformed.remove(&id);
    joints.turning.remove(&id);
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
    let flips = reindex(state, reflection, setup, joint, false);
    drop_flipped(&mut state.joints, &flips);
    note_turning(state, reflection, setup, joint);
}

/// [`index_joint`]'s work, the parts whose marks for the joint changed under its new parts returned ([`relink`]) for the caller
/// to drop or keep up. `keep_up`: a link the joint made between its old parts leaves their kept component, which splits when
/// nothing else links its sides ([`Joints::unlink`]), instead of dropping it.
fn reindex(state: &mut State, reflection: &Reflection, setup: &Setup, joint: InstanceId, keep_up: bool) -> [Flip; 2] {
    let parts = parts_of(state, setup, joint);
    let homed = joint_home(state, joint).is_some();
    let joints = &mut state.joints;
    let old = joints.indexed.get(&joint).copied();
    match old {
        Some(old) if old.parts == parts => return [None, None],
        Some(old) => joints.unindex(joint, &old, keep_up),
        None => {}
    }
    // (a side keeps its naming while its part stays; the other side's change does not name it anew)
    let named = [0, 1].map(|side| match old {
        Some(old) if old.parts[side] == parts[side] => old.named[side],
        _ => {
            joints.naming += 1;
            joints.naming
        }
    });
    let mut places = [0; 2];
    for side in 0..2 {
        let Some(part) = parts[side] else { continue };
        if side == 1 && parts[0] == Some(part) {
            places[1] = places[0];
            continue;
        }
        places[side] = joints.by_part.entry(part).or_default().push(joint, homed);
    }
    joints.indexed.insert(joint, Indexed { parts, places, taken: 0, named, listed: 0 });
    relink(state, reflection, setup, joint)
}

/// The two parts an indexed joint links now, from its marks in their lists (None when it links none, or one part to itself).
fn linked_pair(joints: &Joints, joint: InstanceId) -> Option<[InstanceId; 2]> {
    let entry = joints.indexed.get(&joint)?;
    let side = |side: usize| {
        let part = entry.parts[side]?;
        let place = entry.places[side] as usize;
        joints.by_part.get(&part).filter(|list| list.slots.get(place) == Some(&Some(joint)) && list.is_linking(place)).map(|_| part)
    };
    match (side(0), side(1)) {
        (Some(first), Some(second)) if first != second => Some([first, second]),
        _ => None,
    }
}

/// A part whose mark for a joint changed ([`relink`]), with whether the joint links it now.
type Flip = Option<(InstanceId, bool)>;

/// Works out again whether an indexed joint links each part it is listed under ([`link_in`]), marking its places; the parts whose
/// mark changed (Part0's side first), whose kept components the caller drops ([`drop_flipped`]) or keeps up ([`link_changed`]).
fn relink(state: &mut State, reflection: &Reflection, setup: &Setup, joint: InstanceId) -> [Flip; 2] {
    let mut flips = [None, None];
    let Some(entry) = state.joints.indexed.get(&joint).copied() else { return flips };
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
        if state.joints.by_part.get_mut(&part).is_some_and(|list| list.link(place, on)) {
            flips[side] = Some((part, on));
        }
    }
    // (a link that started files its two parts again in the spatial grid: world/broadphase.rs links_started)
    if let Some((first, second, true)) = clean_flip(&flips) {
        state.spatial.link_started(first, second);
    }
    state.joints.sync_pair(joint);
    flips
}

/// The order a joint first named its pair of parts in, among the joints of that pair ([`Joints::pairs`]): the earlier of its two
/// sides' namings (a side keeps its naming while its part stays: [`reindex`]). A client keeps the order its joints between two parts
/// were made in, a weld re-pointed away and back keeping its place (Studio 0.747, 2026-11-25: the first made of 1,001 welds between
/// a hub and a leaf, its Part1 set to nil and back, then disabled, costs what it cost before, 2.5 to 2.9 us).
fn pair_order(entry: &Indexed) -> u64 {
    entry.named[0].min(entry.named[1])
}

/// The key of a pair of parts in [`Joints::pairs`].
fn pair_key(parts: [InstanceId; 2]) -> [InstanceId; 2] {
    if parts[0] <= parts[1] { parts } else { [parts[1], parts[0]] }
}

/// The joints that link one pair of parts ([`Joints::pairs`]), by [`pair_order`]: one, or several in order.
enum PairLinks {
    One(u64, InstanceId),
    Many(BTreeMap<u64, InstanceId>),
}

impl Joints {
    /// Keeps [`Joints::pairs`] as `joint` links now: under its pair while its marks say it links its two parts, else not.
    fn sync_pair(&mut self, joint: InstanceId) {
        let Some(entry) = self.indexed.get(&joint).copied() else { return };
        let [Some(first), Some(second)] = entry.parts else { return };
        if linked_pair(self, joint).is_some() {
            let order = pair_order(&entry);
            match self.pairs.entry(pair_key([first, second])) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(PairLinks::One(order, joint));
                }
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    let links = slot.get_mut();
                    match links {
                        PairLinks::One(was, other) => {
                            if *other != joint {
                                let many = BTreeMap::from([(*was, *other), (order, joint)]);
                                *links = PairLinks::Many(many);
                            }
                        }
                        PairLinks::Many(many) => {
                            many.insert(order, joint);
                        }
                    }
                }
            }
        } else {
            self.pair_remove([first, second], pair_order(&entry), joint);
        }
    }

    /// Takes `joint` (of `order`) out of the joints of the pair `parts` link by, when it is there.
    fn pair_remove(&mut self, parts: [InstanceId; 2], order: u64, joint: InstanceId) {
        let key = pair_key(parts);
        let Some(links) = self.pairs.get_mut(&key) else { return };
        match links {
            PairLinks::One(_, other) => {
                if *other == joint {
                    self.pairs.remove(&key);
                }
            }
            PairLinks::Many(many) => {
                if many.get(&order) == Some(&joint) {
                    many.remove(&order);
                }
                if many.len() == 1 {
                    let (&was, &other) = many.iter().next().expect("one left");
                    *links = PairLinks::One(was, other);
                }
            }
        }
    }
}

/// The parts whose marks changed lose their kept components.
fn drop_flipped(joints: &mut Joints, flips: &[Flip; 2]) {
    for &(part, _) in flips.iter().flatten() {
        joints.drop_component(part);
    }
}

/// A joint's link between two parts that started or stopped (`flips` of both, alike): the parts, Part0's first, and whether it links
/// them now.
fn clean_flip(flips: &[Flip; 2]) -> Option<(InstanceId, InstanceId, bool)> {
    match *flips {
        [Some((first, on)), Some((second, also))] if on == also && first != second => Some((first, second, on)),
        _ => None,
    }
}

/// Takes the dom's log of the indexed joints and parts whose home changed, by whatever moved them (dom.rs Dom::log_homes), marks each
/// indexed joint in its parts' lists as in a world or not, and works out again whether each such joint, and each joint of each such
/// part in a world, links its parts ([`relink`]; none of a part in no world that none links): every indexed joint when the log
/// overflowed. Then the joints [`forget`] left.
/// Each link that ended (a joint, or one of its parts, taken out of their world, destroyed or moved to nil) leaves its parts' kept
/// component, which splits when nothing else links its sides, one link after another, each list losing its links at once
/// ([`Joints::unlink_all`]); a part whose world
/// changed and that still links others, and the parts of links that started, lose their components; `started`: a Parent write
/// into a world takes the links that started instead, each joint with its two parts (Part0's first), and joins their components
/// ([`reparented`]). The parts of the links that ended, both of each, for the price of a Parent write ([`reparented`]).
fn take_homes(state: &mut State, reflection: &Reflection, setup: &Setup, mut started: Option<&mut Vec<(InstanceId, [InstanceId; 2])>>) -> Vec<(InstanceId, [InstanceId; 2])> {
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
    // (whether an instance is in a WorldRoot now, from the dom's homes: world_of's walk)
    let in_world = |dom: &crate::dom::Dom, id: InstanceId| {
        let mut home = dom.home(id);
        while let Some(node) = home {
            if reflection.class(dom.at(node).class).is_a(setup.world_root) {
                return true;
            }
            home = dom.home(node);
        }
        false
    };
    let mut parts: FxHashSet<InstanceId> = FxHashSet::default();
    let overflowed = log.is_none();
    match log {
        Some(log) if !log.is_empty() => {
            joints.takes += 1;
            for id in log {
                if let Some(entry) = joints.indexed.get_mut(&id).filter(|entry| entry.taken != joints.takes) {
                    entry.taken = joints.takes;
                    mark(&mut joints.by_part, id, entry);
                    joints.relinks.push(id);
                } else if joints.by_part.contains_key(&id) && parts.insert(id) {
                    // (a part's world changed: its joints in a world may link or stop; the others' homes are logged themselves. Its
                    // component goes once its links are worked out, unless they all ended. A part in no world that none of its
                    // joints links, and that holds none in its list of active joints, has none to start or stop: its home moved
                    // between places outside every world,
                    // JointsService and ReplicatedStorage say, which costs a client nothing a joint, so none is worked out again)
                    let list = &joints.by_part[&id];
                    if list.linking > 0 || in_world(dom, id) || joints.attached.contains_key(&id) {
                        joints.relinks.extend(list.homed());
                    }
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
    state.joints.unlisted.extend(relinks.iter().copied());
    let mut flipped: Vec<(InstanceId, [Flip; 2])> = Vec::new();
    for joint in relinks {
        let flips = relink(state, reflection, setup, joint);
        if flips.iter().any(Option::is_some) {
            flipped.push((joint, flips));
        }
    }
    // (each link that ended leaves its parts' kept component, which splits when nothing else links its sides: those in a row
    // together, [`Joints::unlink_all`]; the components of the parts of the other changes go)
    let mut ended = Vec::new();
    let mut ending: Vec<(InstanceId, [InstanceId; 2], [u32; 2])> = Vec::new();
    let mut ending_flips: Vec<usize> = Vec::new();
    let unlink_ending = |joints: &mut Joints, ending: &mut Vec<(InstanceId, [InstanceId; 2], [u32; 2])>, ending_flips: &mut Vec<usize>| {
        for (done, &index) in joints.unlink_all(ending).into_iter().zip(ending_flips.iter()) {
            if !done {
                drop_flipped(joints, &flipped[index].1);
            }
        }
        ending.clear();
        ending_flips.clear();
    };
    for (index, (joint, flips)) in flipped.iter().enumerate() {
        let kept = match clean_flip(flips) {
            Some((first, second, false)) => {
                ended.push((*joint, [first, second]));
                if !overflowed {
                    ending.push((*joint, [first, second], state.joints.indexed[joint].places));
                    ending_flips.push(index);
                    continue;
                }
                false
            }
            Some((first, second, true)) if !overflowed && started.is_some() => {
                started.as_deref_mut().expect("taken").push((*joint, [first, second]));
                true
            }
            _ => false,
        };
        if !kept {
            unlink_ending(&mut state.joints, &mut ending, &mut ending_flips);
            drop_flipped(&mut state.joints, flips);
        }
    }
    unlink_ending(&mut state.joints, &mut ending, &mut ending_flips);
    // (a part that changed world and still links others moved with them: their component goes)
    for part in parts {
        state.joints.drop_component(part);
    }
    ended
}

/// Puts the joints whose homes the dom logged ([`Joints::unlisted`]) in their parts' lists of active joints, or takes them out, as
/// they now are, in the log's order (when the log overflowed, every indexed joint, in the order of the index's hash map:
/// [`take_homes`]): a move the engine makes in the dom without a Parent write (dom.rs reparent: one a test makes to overflow the
/// log, say) is not one [`reparented`] sees, and its joints catch up here before anything reads the lists, so no active joint is
/// ever missing from them (check_index). A Parent write's own joints are already where its walk put them: `walked`, that walk
/// ([`relist_moved`], right before), whose joints are passed by.
fn list_logged(state: &mut State, reflection: &Reflection, setup: &Setup, walked: Option<u64>) {
    for joint in std::mem::take(&mut state.joints.unlisted) {
        if walked.is_some() && state.joints.indexed.get(&joint).map(|entry| entry.listed) == walked {
            continue;
        }
        refresh_attached(state, reflection, setup, joint, None);
    }
}

/// Whether the joint index holds together (tests), once it took the homes the dom logged: each indexed joint at its place in its
/// live parts' lists (one place when it names one part as both), marked there as in a world exactly when it has a home, each
/// list holding each of its joints once, as many as it counts, and none empty.
/// And each place marked as linking exactly when its joint links the part now ([`link_in`]), the walks taking up those, in order;
/// each kept component holding the parts linked to each of its parts; each joint in its parts' lists of active joints exactly
/// while it is active ([`refresh_attached`]).
#[cfg(test)]
pub(crate) fn check_index(state: &mut State, reflection: &Reflection) -> Result<(), String> {
    let Some(setup) = state.joints.setup.clone() else { return Ok(()) };
    let _ = take_homes(state, reflection, &setup, None);
    list_logged(state, reflection, &setup, None);
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
    // (each joint that links two parts is listed under their pair by its order, and the pairs hold nothing else)
    let mut linking = 0usize;
    for (&joint, entry) in &state.joints.indexed {
        let Some(pair) = linked_pair(&state.joints, joint) else { continue };
        linking += 1;
        let order = pair_order(entry);
        let there = match state.joints.pairs.get(&pair_key(pair)) {
            Some(PairLinks::One(was, other)) => *was == order && *other == joint,
            Some(PairLinks::Many(many)) => many.len() > 1 && many.get(&order) == Some(&joint),
            None => false,
        };
        if !there {
            return Err(format!("{joint:?} links {pair:?} but is not listed under the pair by its order {order}"));
        }
    }
    let held: usize = state.joints.pairs.values().map(|links| match links {
        PairLinks::One(..) => 1,
        PairLinks::Many(many) => many.len(),
    }).sum();
    if held != linking {
        return Err(format!("the pairs list {held} joints where {linking} link two parts"));
    }
    let mut checked: Vec<*const Component> = Vec::new();
    let components: Vec<(InstanceId, Rc<Component>)> = state.joints.components.iter().map(|(&part, found)| (part, found.clone())).collect();
    for (part, found) in components {
        if checked.contains(&Rc::as_ptr(&found)) {
            continue;
        }
        checked.push(Rc::as_ptr(&found));
        let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
        let mut queue = vec![part];
        seen.insert(part);
        while let Some(at) = queue.pop() {
            for (_, other) in edges(state, reflection, &setup, at) {
                if seen.insert(other) {
                    queue.push(other);
                }
            }
        }
        let kept: FxHashSet<InstanceId> = found.parts().into_iter().collect();
        if seen != kept || kept.len() < 2 {
            return Err(format!("{part:?}'s kept component holds {} parts where {} are linked to it", kept.len(), seen.len()));
        }
        // (the links each part keeps are the ones a walk takes up, in order; each part is kept with it; its anchored parts)
        for &at in &kept {
            let walked = edges(state, reflection, &setup, at);
            if found.links.borrow().get(&at) != Some(&walked) {
                return Err(format!("{at:?}'s kept links are {:?} where a walk takes up {walked:?}", found.links.borrow().get(&at)));
            }
            if !state.joints.components.get(&at).is_some_and(|other| Rc::ptr_eq(other, &found)) {
                return Err(format!("{at:?} is in {part:?}'s kept component, which is not kept for it"));
            }
        }
        let anchored_now: FxHashSet<InstanceId> = kept.iter().copied().filter(|&at| anchored(state, &setup, at)).collect();
        if *found.anchored.borrow() != anchored_now {
            return Err(format!("{part:?}'s kept component counts {} anchored parts where {} are", found.anchored.borrow().len(), anchored_now.len()));
        }
        // (its root, when kept without its tree, and whether every part is where its joints put it, against a fresh walk's tree)
        let fresh = walk_component(state, reflection, &setup, part).expect("linked");
        let tree = assemblies_of(state, reflection, &setup, &fresh, part, None, false);
        if let Some(root) = found.single_root()
            && (tree.list.len() != 1 || tree.list[0].nodes[0].part != root)
        {
            return Err(format!("{part:?}'s kept component has root {root:?} where a walk's tree has {:?}", tree.list.iter().map(|assembly| assembly.nodes[0].part).collect::<Vec<_>>()));
        }
        // (the assemblies kept through link changes: each part's root and its assembly's size, as the walk's tree has them)
        if let Some(roots) = found.roots.borrow().as_ref() {
            for &at in &kept {
                let want = tree.of(at).map(|(assembly, ..)| (assembly.nodes[0].part, assembly.nodes.len()));
                let have = roots.root(at).zip(roots.size(at));
                if have != want {
                    return Err(format!("{at:?}'s kept roots give its root and assembly size as {have:?} where a walk's tree has {want:?}"));
                }
            }
        }
        if found.settled.get() {
            for assembly in &tree.list {
                let relative = relatives(state, reflection, &setup, assembly);
                let root = cframe_of(state, &setup, assembly.nodes[0].part);
                for (index, node) in assembly.nodes.iter().enumerate().skip(1) {
                    if !settled(&cframe_of(state, &setup, node.part), &(root * relative[index])) {
                        let (current, target) = (cframe_of(state, &setup, node.part), root * relative[index]);
                        return Err(format!(
                            "{part:?}'s kept component is settled, but {:?} is not where its joints put it (node {index} of {}, root {:?} anchored {}, {} assemblies; at {:?} where {:?})",
                            node.part,
                            assembly.nodes.len(),
                            assembly.nodes[0].part,
                            anchored(state, &setup, assembly.nodes[0].part),
                            tree.list.len(),
                            current.translation,
                            target.translation
                        ));
                    }
                }
            }
        }
    }
    // (once the index looked at the place, each rigid joint in a world or JointsService that names a live part is indexed: none
    // made since without a Parent write is left out, made_in_place)
    if state.joints.scanned {
        for slot in 0..state.dom.slot_count() as u32 {
            let Some(joint) = state.dom.id_at_slot(slot) else { continue };
            if setup.kind(state.dom.at(joint).class) != Kind::Other && joint_home(state, joint).is_some() && parts_of(state, &setup, joint).iter().any(Option::is_some) && !state.joints.indexed.contains_key(&joint) {
                return Err(format!("{joint:?} is in a world and names a part, but is not indexed"));
            }
        }
    }
    // (each joint active exactly when it is in its parts' lists of active joints, at its places there: no active joint is missed)
    for &joint in state.joints.indexed.keys() {
        let active = active_parts(state, reflection, &setup, joint, None);
        let listed = state.joints.attached_parts.get(&joint).map(|places| places.map(|(part, _)| part));
        if active != listed {
            return Err(format!("{joint:?} is active between {active:?} but listed under {listed:?}"));
        }
        for &(part, place) in state.joints.attached_parts.get(&joint).into_iter().flatten() {
            if state.joints.attached.get(&part).and_then(|list| list.get(place as usize)) != Some(&joint) {
                return Err(format!("{joint:?} is not at its place {place} in {part:?}'s list of active joints"));
            }
        }
    }
    let listed: usize = state.joints.attached.values().map(Vec::len).sum();
    if listed != 2 * state.joints.attached_parts.len() {
        return Err(format!("the lists of active joints hold {listed} places for {} joints", state.joints.attached_parts.len()));
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

/// Readies the index and the lists of active joints for a walk or a read ([`scan_and_take`], [`list_logged`]).
fn ensure_scanned(state: &mut State, reflection: &Reflection, setup: &Setup) {
    scan_and_take(state, reflection, setup, None);
    list_logged(state, reflection, setup, None);
}

/// Readies the index for a walk: every joint the index keeps indexed, once (those made before anything needed the index), the
/// active ones put in their parts' lists in the order of their slots (the engine did not see them become active; when that first
/// look comes inside a Parent write that brings joints in, it lists them so, and the write's walk then finds them listed and neither
/// orders nor charges them: once in an engine's life), each marked as in a world or not as the dom last logged its home, and as
/// linking or not ([`take_homes`]); the parts of the links that ended since the last take (`started`: the links that started).
fn scan_and_take(state: &mut State, reflection: &Reflection, setup: &Setup, started: Option<&mut Vec<(InstanceId, [InstanceId; 2])>>) -> Vec<(InstanceId, [InstanceId; 2])> {
    if !state.joints.scanned {
        state.joints.scanned = true;
        let joints: Vec<InstanceId> = (0..state.dom.slot_count() as u32)
            .filter_map(|slot| state.dom.id_at_slot(slot))
            .filter(|&id| setup.kind(state.dom.at(id).class) != Kind::Other)
            .collect();
        for joint in joints {
            index_joint(state, reflection, setup, joint);
            refresh_attached(state, reflection, setup, joint, None);
        }
    }
    take_homes(state, reflection, setup, started)
}

/// `id` was made where it stands, its values filled, with no Parent write taking it there (a lazy import's instance: project/plan.rs
/// make_level, make_as; a manifest alias's copy: model_assets.rs install_alias): a rigid joint is indexed now when the index has
/// looked at the place already ([`ensure_scanned`]; before that, that look indexes it with the rest), so it links its parts as the
/// same joint made with the place does. Nothing is placed or charged: a client has it from the place's load.
pub fn made_in_place(state: &mut State, reflection: &Reflection, id: InstanceId) {
    if !state.joints.scanned {
        return;
    }
    let Some(setup) = setup_of(state) else { return };
    if setup.kind(state.dom.at(id).class) != Kind::Other {
        index_joint(state, reflection, &setup, id);
        refresh_attached(state, reflection, &setup, id, None);
    }
}

/// A subtree that a Parent write moved into a world (or from one world into another), as a client takes it up there
/// ([`reparented`]): an instance at a time, in [`taken_up`]'s order, so an instance of it that the walk has not reached yet still
/// counts as where it was (Studio 0.748: a joint below a part that comes back joins the lists when the walk reaches it, after the
/// joints the part brings back, tools/parity/probes/unjoin_from_outsiders.luau section 6).
struct Moving {
    /// Each instance of the subtree, with its place in the walk
    places: FxHashMap<InstanceId, usize>,
    /// The place of the instance being taken up
    at: usize,
    /// The subtree's home and WorldRoot before the move (those of an instance not reached yet, unless one inside the subtree
    /// holds it)
    old_home: Option<InstanceId>,
    old_world: Option<InstanceId>,
}

impl Moving {
    /// Where `id`, at `now` (its home or its world, as the dom keeps them), counts as being: where it was while the walk has not
    /// reached it, unless a home inside the subtree holds it (which moved along with it).
    fn place(&self, id: InstanceId, now: Option<InstanceId>, before: Option<InstanceId>) -> Option<InstanceId> {
        let waiting = self.places.get(&id).is_some_and(|&place| place > self.at);
        if waiting && !now.is_some_and(|home| self.places.contains_key(&home)) { before } else { now }
    }
}

/// The parts of `joint` while it is active ([`Kind::unjoined`], Enabled, naming two different parts in one WorldRoot, and itself
/// in that WorldRoot or in JointsService with the parts in the Workspace, as [`edges`] reads a link: [`homed_in`]), None while it
/// is not. While a subtree moves, its instances count as `moving` says.
fn active_parts(state: &State, reflection: &Reflection, setup: &Setup, joint: InstanceId, moving: Option<&Moving>) -> Option<[InstanceId; 2]> {
    let instance = state.dom.get(joint)?;
    let kind = setup.kind(instance.class);
    if !kind.unjoined() || state.dom.boolean(joint, setup.enabled) == Some(false) {
        return None;
    }
    let [Some(part0), Some(part1)] = parts_of(state, setup, joint) else { return None };
    if part0 == part1 || !is_part(state, reflection, setup, part0) || !is_part(state, reflection, setup, part1) {
        return None;
    }
    let counted_world = |part: InstanceId| {
        let world = world_of(state, reflection, setup, part);
        moving.map_or(world, |moving| moving.place(part, world, moving.old_world))
    };
    let world = counted_world(part0)?;
    if counted_world(part1) != Some(world) {
        return None;
    }
    let home = joint_home(state, joint);
    match moving.map_or(home, |moving| moving.place(joint, home, moving.old_home)) {
        Some(home) if homed_in(state, reflection, setup, kind, home, world) => Some([part0, part1]),
        _ => None,
    }
}

/// What a [`refresh_attached`] changed: the parts whose lists of active joints the joint left.
#[derive(Default)]
struct Refreshed {
    left: Option<[InstanceId; 2]>,
}

/// Puts `joint` in its parts' lists of active joints, or takes it out, as it now is: one that becomes active, or is active with
/// other parts than it was (another part, the two swapped), goes to the end of each of its parts' lists; one that stops being
/// active leaves them, the last joint of each list taking its place. What a playing client and its server show through
/// Workspace:UnjoinFromOutsiders' order (Studio 0.748, unjoin_from_outsiders.luau): the order joints were parented in, or got
/// their parts while parented, not the order they were made in; one taken out and parented again (the same frame or the next),
/// disabled and enabled, or given another part and then its own back, goes last and the last one takes its old place (of four
/// joints, the second out and back leaves 1, 4, 3, 2); one moved into JointsService, or whose C0 changed, keeps its place.
/// While a subtree moves into a world, its instances count as `moving` says.
fn refresh_attached(state: &mut State, reflection: &Reflection, setup: &Setup, joint: InstanceId, moving: Option<&Moving>) -> Refreshed {
    let now = active_parts(state, reflection, setup, joint, moving);
    let joints = &mut state.joints;
    let was = joints.attached_parts.get(&joint).map(|places| places.map(|(part, _)| part));
    if was == now {
        return Refreshed::default();
    }
    joints.detach(joint);
    if let Some(parts) = now {
        let places = parts.map(|part| {
            let list = joints.attached.entry(part).or_default();
            list.push(joint);
            (part, list.len() as u32 - 1)
        });
        joints.attached_parts.insert(joint, places);
    }
    Refreshed { left: was }
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

/// Whether a joint of `kind` at `home` (its WorldRoot or JointsService: [`joint_home`]) can link parts of `world`: one in that world,
/// or a JointInstance in JointsService when the world is the Workspace's own (Studio 0.747, a playing client and its server alike: a
/// Weld, Snap, Glue or Motor6D in JointsService links two parts of the Workspace however they came in, and no two of a WorldModel,
/// one in the Workspace too; a WeldConstraint in JointsService links nothing; tools/parity/probes/assembly_roots.luau).
fn homed_in(state: &State, reflection: &Reflection, setup: &Setup, kind: Kind, home: InstanceId, world: InstanceId) -> bool {
    home == world
        || kind == Kind::Joint
            && Some(state.dom.at(home).class) == setup.joints_service
            && setup.workspace.is_some_and(|workspace| reflection.class(state.dom.at(world).class).is_a(workspace))
}

/// The part `joint` links `part` (in `world`) to now, if it does: the joint is a rigid one and not disabled, names `part` and
/// another BasePart in the same world, and is itself in that world or, a JointInstance, in JointsService with the world the
/// Workspace ([`homed_in`]). A few lookups, whatever the depth of the parts and the joint in the tree.
fn link_in(state: &State, reflection: &Reflection, setup: &Setup, joint: InstanceId, part: InstanceId, world: InstanceId) -> Option<InstanceId> {
    let instance = state.dom.get(joint)?;
    let kind = setup.kind(instance.class);
    if !kind.rigid() || state.dom.boolean(joint, setup.enabled) == Some(false) {
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
        Some(home) if homed_in(state, reflection, setup, kind, home, world) => Some(other),
        _ => None,
    }
}

/// The joints that link `part` to another part now, with that part, in the order its list holds them. The walk takes up the
/// places marked as linking alone ([`relink`]), each checked again ([`link_in`]), so a part's joints that cannot link cost it
/// nothing.
fn edges(state: &State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> Vec<(InstanceId, InstanceId)> {
    let Some(candidates) = state.joints.by_part.get(&part) else {
        return Vec::new();
    };
    // (a part outside every WorldRoot links to nothing)
    let Some(world) = world_of(state, reflection, setup, part) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for joint in candidates.linking() {
        // (marked as linking, it links while the marks are kept: check_index)
        let Some(other) = link_in(state, reflection, setup, joint, part, world) else { continue };
        found.push((joint, other));
    }
    found
}

fn anchored(state: &State, setup: &Setup, part: InstanceId) -> bool {
    state.dom.boolean(part, setup.anchored) == Some(true)
}

/// Whether `part`'s assembly holds an anchored part, read from its kept component alone (world/broadphase.rs: the spatial grid files a
/// part in its assembly's list); None when it links no other part or its component is not kept now, which this never works out.
pub(crate) fn grid_anchored(state: &State, part: InstanceId) -> Option<bool> {
    let found = state.joints.components.get(&part)?;
    Some(!found.anchored.borrow().is_empty())
}

/// The parts of `part`'s kept component, in no particular order (world/broadphase.rs: an Anchored write that anchors or frees the
/// whole of it files them again); None when it is not kept.
pub(crate) fn grid_component(state: &State, part: InstanceId) -> Option<Vec<InstanceId>> {
    let found = state.joints.components.get(&part)?;
    let mut parts: Vec<InstanceId> = found.links.borrow().keys().copied().collect();
    parts.sort_unstable();
    Some(parts)
}

/// Of two parts a joint links, whether `a` comes before `b` as their assembly's root would: an anchored part first, else the
/// better ranked ([`root_rank`]), of equal parts the later made (world/broadphase.rs: a link that starts files its parts again in
/// that order, measured).
pub(crate) fn root_first(state: &State, reflection: &Reflection, a: InstanceId, b: InstanceId) -> bool {
    let Some(setup) = state.joints.setup.clone() else { return true };
    let (anchored_a, anchored_b) = (anchored(state, &setup, a), anchored(state, &setup, b));
    if anchored_a != anchored_b {
        return anchored_a;
    }
    match root_rank(state, reflection, &setup, a).partial_cmp(&root_rank(state, reflection, &setup, b)) {
        Some(std::cmp::Ordering::Greater) => true,
        Some(std::cmp::Ordering::Less) => false,
        _ => state.dom.at(a).serial >= state.dom.at(b).serial,
    }
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
    // (one assembly whose root is known, or the assemblies kept through the link changes since the last tree: no tree to work out)
    if let Some(root) = found.single_root().or_else(|| found.kept_root(part)).filter(|_| found.derived.get()) {
        return Some(root);
    }
    let tree = assemblies_of(state, reflection, &setup, &found, part, None, true);
    tree.of(part).map(|(assembly, ..)| assembly.nodes[0].part)
}

/// The parts linked to `start` now and their links, as a walk from it finds them, what it looked at counted in `walk`; None when
/// `start` links to no other part.
fn walk_component(state: &State, reflection: &Reflection, setup: &Setup, start: InstanceId) -> Option<Component> {
    let mut parts = vec![start];
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    seen.insert(start);
    let mut links: FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>> = FxHashMap::default();
    let mut index = 0;
    while index < parts.len() {
        let part = parts[index];
        let found = edges(state, reflection, setup, part);
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
    let anchored_parts = parts.iter().copied().filter(|&part| anchored(state, setup, part)).collect();
    Some(Component::new(links, anchored_parts))
}

/// The component of `part`, the index readied ([`ensure_scanned`]): the one kept for it, else found by a walk from it and kept
/// for each of its parts. None when it links to no other part.
fn component(state: &mut State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> Option<Rc<Component>> {
    if let Some(found) = state.joints.components.get(&part) {
        return Some(found.clone());
    }
    let found = Rc::new(walk_component(state, reflection, setup, part)?);
    keep(state, &found);
    Some(found)
}

/// Keeps `found` for each of its parts.
fn keep(state: &mut State, found: &Rc<Component>) {
    if !keeping() {
        return;
    }
    for part in found.parts() {
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
    let links = found.links.borrow();
    let mut order = vec![start];
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    seen.insert(start);
    let mut index = 0;
    while index < order.len() {
        for &(_, other) in links.get(&order[index]).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(other) {
                order.push(other);
            }
        }
        index += 1;
    }
    order
}

/// The assemblies of a component (several when more than one part is anchored), as a walk from `start` finds them, kept with it.
/// Each anchored part roots an assembly of its own, and of several, each unanchored part hangs from the one [`claims`] gives it; each
/// assembly's tree is a walk from its root through its own parts, each part hanging from the first part the walk takes it up from (a
/// client's tree: a cycle's joints place its parts from the root's side, whichever joint was made last; Studio 0.747, client and
/// server, tools/parity/probes/assembly_roots.luau).
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
        .borrow()
        .values()
        .flatten()
        .map(|&(joint, _)| joint)
        .filter(|&joint| setup.kind(state.dom.at(joint).class) == Kind::WeldConstraint && !weld_valid(state, setup, joint))
        .collect();
    if derive || welds.is_empty() {
        found.derived.set(true);
    }
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
    let parts = found.parts();
    let mut roots: Vec<InstanceId> = parts.iter().copied().filter(|&part| is_anchored(state, part)).collect();
    crate::sort::by_key(&mut roots, |&part| serial(state, part));
    let mut ranked = None;
    if roots.is_empty() {
        // (of ranks that compare, the best is the same in any order; a NaN score is looked at in the walk's order from `start`)
        let nan = parts.iter().any(|&part| root_rank(state, reflection, setup, part).2.is_nan());
        let order: Vec<InstanceId> = if nan {
            keep = false;
            order_from(found, start)
        } else {
            Vec::new()
        };
        let candidates: &[InstanceId] = if nan { &order } else { &parts };
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
        ranked = Some(best);
    }

    // Each root's tree, a walk from it through the parts that hang from it (of several anchored parts, those [`claims`] gives it)
    let links = found.links.borrow();
    let claimed = (roots.len() > 1).then(|| claims(state, setup, &links, &roots));
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
        for &(joint, other) in links.get(&part).map(Vec::as_slice).unwrap_or_default() {
            if claimed.as_ref().is_some_and(|claimed| claimed.get(&other).is_some_and(|&at| at as usize != assembly)) {
                continue;
            }
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
    for (part, links) in links.iter() {
        let Some(&(assembly, _)) = place.get(part) else { continue };
        across[assembly as usize] += links.iter().filter(|(_, other)| place.get(other).map(|&(at, _)| at) != Some(assembly)).count() as u32;
    }
    let tree = Rc::new(Assemblies { list, place, across });
    if keep {
        *found.tree.borrow_mut() = Some(tree.clone());
        *found.roots.borrow_mut() = None;
        if ranked.is_some() {
            found.best.set(ranked);
        }
    }
    tree
}

/// Which anchored part each part of a component with several hangs from, as a playing client joins them (Studio 0.747, client and
/// server alike, tools/parity/probes/assembly_roots.luau): its links are taken one at a time, a Weld, ManualWeld, Snap, Glue or
/// ManualGlue before any WeldConstraint, a WeldConstraint before any Motor, a Motor before any Motor6D, and of one kind the later
/// made joint first (the order they were made in, not linked in: a weld made first and parented last counts as made first; a clone's
/// in the order the clone made them); each joins the parts on its two sides unless both sides already hang from an anchored part.
/// So a part welded to several anchored parts hangs from the one its last made weld goes to (whichever was anchored first), a free
/// piece between two anchored parts hangs whole from one end, and two free parts bridging the same two anchored parts can hang one
/// from each. `roots`: the anchored parts (each the index of its assembly); the answer gives each part its root's index.
fn claims(state: &State, setup: &Setup, links: &FxHashMap<InstanceId, Vec<(InstanceId, InstanceId)>>, roots: &[InstanceId]) -> FxHashMap<InstanceId, u32> {
    let index: FxHashMap<InstanceId, usize> = links.keys().enumerate().map(|(at, &part)| (part, at)).collect();
    let mut parent: Vec<usize> = (0..index.len()).collect();
    // (each set's anchored part, as its root's index)
    let mut fixed: Vec<Option<u32>> = vec![None; index.len()];
    for (at, root) in roots.iter().enumerate() {
        if let Some(&slot) = index.get(root) {
            fixed[slot] = Some(at as u32);
        }
    }
    fn find(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    // (each link once, from its side of the lower id)
    let mut order: Vec<(u8, u32, InstanceId, InstanceId)> = links
        .iter()
        .flat_map(|(&part, list)| list.iter().filter(move |&&(_, other)| part < other).map(move |&(joint, other)| (joint, part, other)))
        .map(|(joint, part, other)| {
            let instance = state.dom.at(joint);
            (setup.union_rank(instance.class), u32::MAX - instance.serial, part, other)
        })
        .collect();
    order.sort_unstable_by_key(|&(rank, later, ..)| (rank, later));
    for (_, _, a, b) in order {
        let (a, b) = (find(&mut parent, index[&a]), find(&mut parent, index[&b]));
        if a == b || fixed[a].is_some() && fixed[b].is_some() {
            continue;
        }
        parent[b] = a;
        fixed[a] = fixed[a].or(fixed[b]);
    }
    let mut claimed = FxHashMap::default();
    for (&part, &slot) in &index {
        let set = find(&mut parent, slot);
        if let Some(root) = fixed[set] {
            claimed.insert(part, root);
        }
    }
    claimed
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

/// What the mass reads take from a part (world/gravity.rs part_point): where it is, its mass, and whether it is Massless (it then
/// weighs nothing in its assembly unless it is the root).
pub type Point = (glam::Vec3, f32, bool);

/// AssemblyMass and AssemblyCenterOfMass of `part`, as a client reads them (Studio 0.747, client and server,
/// tools/parity/probes/assembly_mass.luau): over the parts of its assembly ([`members`]; two anchored parts welded together are two
/// assemblies), each counted at its mass but a Massless one other than the root, the centre worked out about the root
/// ([`mass_centre`]), so every part of the assembly reads the same; the mass is inf when the assembly holds an anchored part, from the
/// Anchored write on, and the centre still its parts'. A part linked to no other is an assembly of its own in a world (inf when
/// anchored) and none outside every world, where it reads its own mass and position, anchored or not; it is its own root either
/// way, so its Massless counts for nothing. `point` gives each part's position and mass (world/gravity.rs part_point). Kept with
/// its component: the mass until a part's mass changes, the centre while no part of it is placed; a move works out again the
/// centres read since the last one ([`refresh_centres`]), as a client's centre read after a move costs the same at any size.
/// `centre`: whether the centre is asked for (a mass alone is answered while only the parts moved). The first such read after a
/// link change pays for working the body out again, as a client's does ([`pay_body`]).
pub fn assembly_body(state: &mut State, reflection: &Reflection, part: InstanceId, centre: bool, point: impl Fn(&State, InstanceId) -> Point) -> (f32, glam::Vec3) {
    let found = match setup_of(state) {
        Some(setup) => {
            ensure_scanned(state, reflection, &setup);
            component(state, reflection, &setup, part).map(|found| (setup, found))
        }
        None => None,
    };
    pay_body(state, part, found.as_ref().map(|(_, found)| &**found), centre);
    let Some((setup, found)) = found else {
        let (at, mass, _) = point(state, part);
        let fixed = state.joints.setup.as_ref().is_some_and(|setup| anchored(state, setup, part) && world_of(state, reflection, setup, part).is_some());
        return (if fixed { f32::INFINITY } else { mass }, at);
    };
    let (mass, at, _) = kept_body(state, reflection, &setup, &found, part, centre, centre, &point);
    (mass, at)
}

/// The parts of the assembly `part` is in, in `found`: its root first, each after its parent, as [`assemblies_of`]' tree holds them,
/// kept for each of them.
fn members(state: &mut State, reflection: &Reflection, setup: &Setup, found: &Component, part: InstanceId) -> Rc<[InstanceId]> {
    if let Some(list) = found.memo.borrow().members.get(&part) {
        return list.clone();
    }
    let tree = assemblies_of(state, reflection, setup, found, part, None, false);
    let list: Rc<[InstanceId]> = match tree.of(part) {
        Some((assembly, ..)) => assembly.nodes.iter().map(|node| node.part).collect(),
        None => Rc::from([part]),
    };
    let mut memo = found.memo.borrow_mut();
    for &other in list.iter() {
        memo.members.insert(other, list.clone());
    }
    list
}

/// The mass, centre of mass and the centre's offset from the root of the assembly `part` is in ([`assembly_body`]), from `found`'s
/// memo, or worked out and kept for each part of the assembly. `centre`: whether the centre is wanted (a kept mass answers alone
/// otherwise, the centre and offset zero); `read`: whether a script read it, so a move works it out again ([`refresh_centres`]).
#[allow(clippy::too_many_arguments)]
fn kept_body(
    state: &mut State, reflection: &Reflection, setup: &Setup, found: &Component, part: InstanceId, centre: bool, read: bool,
    point: &impl Fn(&State, InstanceId) -> Point,
) -> (f32, glam::Vec3, glam::Vec3) {
    let free = found.anchored.borrow().is_empty();
    // (an assembly with an anchored part is immovable: a client keeps its mass inf through every link change, and so walks nothing)
    if !centre && !free {
        return (f32::INFINITY, glam::Vec3::ZERO, glam::Vec3::ZERO);
    }
    {
        let mut memo = found.memo.borrow_mut();
        let Memo { mass, centre: centres, placed, bodies, reads, .. } = &mut *memo;
        if let Some(&mass) = mass.get(&part) {
            if !centre {
                return (mass, glam::Vec3::ZERO, glam::Vec3::ZERO);
            }
            if let Some(entry) = centres.get_mut(&part).filter(|entry| entry.placed == *placed) {
                if read && !std::mem::replace(&mut entry.read, true) {
                    reads.push(part);
                }
                return (mass, entry.at, entry.offset);
            }
        }
        // (the root's walk, kept through the link changes since: the masses are added again from the first part they changed, nothing
        // walked)
        if !centre
            && let Some(root) = found.single_root().or_else(|| found.kept_root(part))
            && let Some(body) = bodies.get_mut(&root)
        {
            let sum = body.mass(|other| counted(point(state, other), other == root));
            mass.insert(part, sum);
            return (sum, glam::Vec3::ZERO, glam::Vec3::ZERO);
        }
    }
    let order = members(state, reflection, setup, found, part);
    let root = order[0];
    let points: Vec<(glam::Vec3, f32)> = order
        .iter()
        .map(|&other| {
            let point = point(state, other);
            (point.0, counted(point, other == root))
        })
        .collect();
    let (total, offset) = mass_centre(points.iter().copied());
    let at = points[0].0 + offset;
    // (every assembly of a component that holds an anchored part hangs from one)
    let mass = if free { total } else { f32::INFINITY };
    let mut memo = found.memo.borrow_mut();
    let placed = memo.placed;
    let mut listed = false;
    for (&other, &(_, own)) in order.iter().zip(&points) {
        memo.mass.insert(other, mass);
        let old = memo.centre.insert(other, Centre { at, offset, placed, read: read && other == part });
        listed |= other == part && old.is_some_and(|old| old.read);
        memo.masses.insert(other, own);
    }
    if read && !listed {
        memo.reads.push(part);
    }
    // (a free assembly's mass reads keep the walk from its root, the order its parts are summed in, for the link changes to come)
    if free {
        let kept = memo.bodies.len();
        match memo.bodies.get_mut(&root) {
            Some(body) => {
                let sum = body.mass(|other| counted(point(state, other), other == root));
                debug_assert!(body.parts().eq(order.iter().copied()), "a kept walk takes the assembly up in its order");
                debug_assert_eq!(sum.to_bits(), total.to_bits(), "a kept walk sums the masses as the read does");
            }
            None if kept < KEPT_BODIES => {
                let walk = walk_order(&found.links.borrow(), root, None);
                debug_assert!(walk.iter().map(|&(other, ..)| other).eq(order.iter().copied()), "a walk from the root takes the assembly up in its order");
                let own: Vec<f32> = points.iter().map(|&(_, own)| own).collect();
                let body = Body::new(walk, &own);
                debug_assert_eq!(body.sums.last().map(|sum| sum.to_bits()), Some(total.to_bits()), "a kept walk sums the masses as the read does");
                memo.bodies.insert(root, body);
            }
            None => {}
        }
    }
    (mass, at, offset)
}

/// A part's mass as its assembly counts it: none for a Massless part but the root (`root`: whether it is).
fn counted((_, mass, massless): Point, root: bool) -> f32 {
    if massless && !root { 0.0 } else { mass }
}

/// The parts of the assembly `part` is in, its root first, each after its parent ([`members`]); the part alone when it links to no
/// other.
pub fn assembly_parts(state: &mut State, reflection: &Reflection, part: InstanceId) -> Rc<[InstanceId]> {
    let Some(setup) = setup_of(state) else { return Rc::from([part]) };
    ensure_scanned(state, reflection, &setup);
    match component(state, reflection, &setup, part) {
        Some(found) => members(state, reflection, &setup, &found, part),
        None => Rc::from([part]),
    }
}

/// The parts of the assembly `part` is in, its root first ([`assembly_parts`]), and its centre of mass's offset from the root
/// ([`assembly_body`]'s, from the memo): what a velocity write converts the assembly's velocity through (world/gravity.rs assign).
/// It pays for nothing, as the write's own price stands; a part linked to no other is its own, its offset zero.
pub fn assembly_frame(state: &mut State, reflection: &Reflection, part: InstanceId, point: impl Fn(&State, InstanceId) -> Point) -> (Rc<[InstanceId]>, glam::Vec3) {
    let Some(setup) = setup_of(state) else { return (Rc::from([part]), glam::Vec3::ZERO) };
    ensure_scanned(state, reflection, &setup);
    let Some(found) = component(state, reflection, &setup, part) else { return (Rc::from([part]), glam::Vec3::ZERO) };
    let (_, _, offset) = kept_body(state, reflection, &setup, &found, part, true, false, &point);
    (members(state, reflection, &setup, &found, part), offset)
}

/// The parts of the assembly `part` is in, its root first ([`assembly_parts`]), and its centre of mass's offset from the root, worked
/// out as [`assembly_body`] works it (the same sum, to the bit) without keeping anything in the component's memo: world/gravity.rs
/// assemblies_changed reads the assemblies a write changed without moving what the reads after it keep or pay.
pub fn assembly_offset(state: &mut State, reflection: &Reflection, part: InstanceId, point: impl Fn(&State, InstanceId) -> Point) -> (Rc<[InstanceId]>, glam::Vec3) {
    let Some(setup) = setup_of(state) else { return (Rc::from([part]), glam::Vec3::ZERO) };
    ensure_scanned(state, reflection, &setup);
    let Some(found) = component(state, reflection, &setup, part) else { return (Rc::from([part]), glam::Vec3::ZERO) };
    let order = members(state, reflection, &setup, &found, part);
    let root = order[0];
    let (_, offset) = mass_centre(order.iter().map(|&other| {
        let point = point(state, other);
        (point.0, counted(point, other == root))
    }));
    (order, offset)
}

/// Works out again the stale centres of mass of `found` read since its last move, up to [`REFRESHED_CENTRES`] of them, from the
/// masses kept and where the parts are now: as [`assembly_body`] would at the next read, to the bit, so that read costs a lookup.
/// The others are worked out when read.
fn refresh_centres(state: &State, setup: &Setup, found: &Component) {
    let mut memo = found.memo.borrow_mut();
    let Memo { members, mass, centre, masses, placed, reads, .. } = &mut *memo;
    let mut left = REFRESHED_CENTRES;
    for part in std::mem::take(reads) {
        let Some(entry) = centre.get_mut(&part) else { continue };
        if !std::mem::replace(&mut entry.read, false) || entry.placed == *placed || left == 0 {
            continue;
        }
        let (Some(order), Some(&total)) = (members.get(&part), mass.get(&part)) else { continue };
        let points: Option<Vec<(glam::Vec3, f32)>> =
            order.iter().map(|other| masses.get(other).map(|&own| (glam::Vec3::from(cframe_of(state, setup, *other).translation), own))).collect();
        let Some(points) = points else { continue };
        let (again, offset) = mass_centre(points.iter().copied());
        debug_assert!(total.is_infinite() || again.to_bits() == total.to_bits(), "a refreshed mass is the kept one");
        entry.at = points[0].0 + offset;
        entry.offset = offset;
        entry.placed = *placed;
        left -= 1;
    }
}

/// A move of `part` (world/gravity.rs: the physics step's) ended: its component's centres of mass read since the last move are
/// worked out again ([`refresh_centres`]).
pub fn moved(state: &mut State, part: InstanceId) {
    let (Some(found), Some(setup)) = (state.joints.components.get(&part).cloned(), state.joints.setup.clone()) else { return };
    refresh_centres(state, &setup, &found);
}

/// An assembly read of `part` (AssemblyMass with `centre` false, AssemblyCenterOfMass, ApplyImpulseAtPosition) after a link change
/// of its assembly in this frame pays for working its body out again, as a client's first such read does, once a change (cost.rs
/// BODY_REBUILT_PART, each part of `found`, its component, or the part alone): a client keeps an assembly's mass up as its links
/// change when a part of it is anchored, and works the centre out at the read; when none is, either read works out both.
fn pay_body(state: &mut State, part: InstanceId, found: Option<&Component>, centre: bool) {
    let joints = &mut state.joints;
    if joints.owed.is_empty() || joints.link_frame != Some(state.sched.frame) {
        return;
    }
    let anchored_here = match found {
        Some(found) => !found.anchored.borrow().is_empty(),
        None => joints.setup.as_ref().is_some_and(|setup| state.dom.boolean(part, setup.anchored) == Some(true)),
    };
    if !centre && anchored_here {
        return;
    }
    // (the parts owed that are the part read or in its component: looked for from the smaller of the two, so a read costs the same
    // however many parts the frame's changes owe)
    let owed_here: Vec<InstanceId> = match found {
        Some(found) if found.size() < joints.owed.len() => found.links.borrow().keys().copied().filter(|other| joints.owed.contains(other)).collect(),
        Some(found) => joints.owed.iter().copied().filter(|other| *other == part || found.links.borrow().contains_key(other)).collect(),
        None => joints.owed.contains(&part).then_some(part).into_iter().collect(),
    };
    if owed_here.is_empty() {
        return;
    }
    for other in owed_here {
        joints.owed.remove(&other);
    }
    let work = found.map_or(1, Component::size) as f64 * crate::cost::BODY_REBUILT_PART;
    state.sched.charge(work);
    crate::boundary::add_cost(work);
}

/// How many parts the assembly of `part` holds (1 for one that links to no other), from its kept component.
pub fn assembly_size(state: &mut State, reflection: &Reflection, part: InstanceId) -> usize {
    let Some(setup) = setup_of(state) else { return 1 };
    ensure_scanned(state, reflection, &setup);
    let Some(found) = component(state, reflection, &setup, part) else { return 1 };
    // (one assembly: all of it, with no tree to work out after a link change; of several, as the kept roots count them)
    if found.single_root().is_some() {
        return found.size();
    }
    if found.tree.borrow().is_none()
        && let Some(size) = found.roots.borrow().as_ref().and_then(|roots| roots.size(part))
    {
        return size;
    }
    let tree = assemblies_of(state, reflection, &setup, &found, part, None, false);
    tree.of(part).map_or(1, |(assembly, ..)| assembly.nodes.len())
}

/// A property of `id` was stored (instance.rs store: every write, signalled or not): its kept component forgets what the value
/// fed ([`watch`]): a move makes the centres of mass stale and, but for the assembly code's own placing, leaves the component not
/// known to be settled; a mass input forgets the masses with them; an Anchored write forgets the assemblies' parts and masses (a
/// mass reads inf with an anchored part), sets the assemblies aside for the write's price ([`reanchored`]) and counts the part as
/// anchored or not; a root rank's input forgets the whole component.
#[inline]
pub fn stored(state: &mut State, id: InstanceId, name: NameId) {
    let State { joints, dom, .. } = state;
    let what = joints.watch.get(name as usize).copied().unwrap_or(0);
    // (a part's mass or rank changes its assembly's centre or root: its motion is read again, gravity::assemblies_changed)
    if joints.motion && (what == watch::MASS || what == watch::ROOTS) {
        joints.regrouped.push(id);
    }
    if joints.components.is_empty() {
        return;
    }
    if what == 0 {
        return;
    }
    let Some(found) = joints.components.get(&id).cloned() else { return };
    match what {
        watch::PLACE => {
            let mut memo = found.memo.borrow_mut();
            memo.placed = memo.placed.wrapping_add(1);
            if joints.placing == 0 {
                joints.unsettled = found.settled.replace(false).then_some(id);
            }
        }
        watch::MASS => {
            let mut memo = found.memo.borrow_mut();
            memo.mass.clear();
            memo.centre.clear();
            memo.reads.clear();
            memo.masses.clear();
            memo.bodies.clear();
        }
        watch::ANCHORED => {
            // (the assemblies change, and with them the masses: inf with an anchored part, a Massless part's as a root's)
            {
                let mut memo = found.memo.borrow_mut();
                memo.members.clear();
                memo.mass.clear();
                memo.centre.clear();
                memo.reads.clear();
                memo.masses.clear();
            }
            let tree = found.tree.borrow_mut().take();
            *found.roots.borrow_mut() = None;
            let settled = found.settled.replace(false);
            if joints.setup.as_ref().is_some_and(|setup| dom.boolean(id, setup.anchored) == Some(true)) {
                found.anchored.borrow_mut().insert(id);
            } else {
                found.anchored.borrow_mut().remove(&id);
            }
            joints.before_anchor = Some((id, tree, settled));
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
            match legacy_angle(state, reflection, setup, joint) {
                // (from Part1's side the angle turns Part0 the same way: Part0 = Part1 * C1 * Rz(angle) * C0:Inverse(), measured)
                Some(angle) if reference(state, joint, setup.part0) != Some(from) => {
                    return inverse(&(c0 * turn_z(-angle) * transform * inverse(&c1)));
                }
                Some(angle) => c0 * turn_z(angle) * transform * inverse(&c1),
                None => c0 * transform * inverse(&c1),
            }
        }
    };
    if reference(state, joint, setup.part0) == Some(from) { forward } else { inverse(&forward) }
}

/// A legacy Motor's turn: its CurrentAngle about Z between C0 and C1, Part1 = Part0 * C0 * CFrame.Angles(0, 0, CurrentAngle) *
/// C1:Inverse() (Studio 0.747, a playing client and its server: a write places it at once, and each simulation step turns it toward
/// DesiredAngle, [`step_motors`]; tools/parity/probes/legacy_motors.luau). A Motor6D turns so until its Transform is first written,
/// and from then by its Transform alone. None for no turn (an angle of 0 leaves the arithmetic as it was).
fn legacy_angle(state: &State, reflection: &Reflection, setup: &Setup, joint: InstanceId) -> Option<f32> {
    let class = reflection.class(state.dom.at(joint).class);
    if !setup.motor.is_some_and(|motor| class.is_a(motor)) {
        return None;
    }
    if setup.motor6d.is_some_and(|motor6d| class.is_a(motor6d)) && state.joints.transformed.contains(&joint) {
        return None;
    }
    let angle = state.dom.number(joint, setup.current_angle?)? as f32;
    (angle != 0.0).then_some(angle)
}

/// A turn about Z by `angle`, its sine and cosine worked out in double and stored as floats (a client's: sin 0.1 reads 0.099833414).
fn turn_z(angle: f32) -> Affine3A {
    let (sin, cos) = f64::from(angle).sin_cos();
    let (sin, cos) = (sin as f32, cos as f32);
    Affine3A::from_mat3(glam::Mat3::from_cols(glam::Vec3::new(cos, sin, 0.0), glam::Vec3::new(-sin, cos, 0.0), glam::Vec3::Z))
}

/// Notes `motor` as turning when its CurrentAngle is not its DesiredAngle and it has a MaxVelocity ([`step_motors`]).
fn note_turning(state: &mut State, reflection: &Reflection, setup: &Setup, motor: InstanceId) {
    if !setup.motor.is_some_and(|class| reflection.class(state.dom.at(motor).class).is_a(class)) {
        return;
    }
    let read = |name: Option<NameId>| name.and_then(|name| state.dom.number(motor, name)).unwrap_or(0.0) as f32;
    let (current, desired, velocity) = (read(setup.current_angle), read(setup.desired_angle), read(setup.max_velocity));
    if velocity != 0.0 && current != desired {
        state.joints.turning.insert(motor);
    }
}

/// Moves a part the assembly code computed: stored, the engine's hooks told, no Changed events. Its component stays settled.
fn place(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, part: InstanceId, cframe: &Affine3A) {
    let mut placing = Raised::new(state, |joints| &mut joints.placing);
    core::raw_set_unsignalled(&mut placing, reflection, hooks, part, setup.cframe, frame::cframe_value(cframe));
}

/// Settles an assembly: every part where its joints put it from the anchor. `tolerant`: a part
/// already there within float noise keeps its value. How many parts it placed, and whether every part is now where the joints put
/// it from the root within float noise (a moved part kept where it was written may not be).
fn settle(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, assembly: &Assembly, anchor: Anchor, tolerant: bool) -> (usize, bool) {
    let relative = relatives(state, reflection, setup, assembly);
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
    let whole = fixed == 0 || settled(&cframe_of(state, setup, assembly.nodes[fixed].part), &(root * relative[fixed]));
    (placed, whole)
}

/// Each node's CFrame relative to its assembly's root, through the joints from the root.
fn relatives(state: &State, reflection: &Reflection, setup: &Setup, assembly: &Assembly) -> Vec<Affine3A> {
    let mut relative: Vec<Affine3A> = Vec::with_capacity(assembly.nodes.len());
    for node in &assembly.nodes {
        relative.push(match node.parent {
            None => Affine3A::IDENTITY,
            Some((parent, joint)) => relative[parent] * across(state, reflection, setup, joint, assembly.nodes[parent].part),
        });
    }
    relative
}

/// Settles the assemblies of `seeds` from their roots, from their kept components (walked when not kept).
fn settle_from_roots(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, seeds: &[InstanceId]) {
    let mut visited: FxHashSet<InstanceId> = FxHashSet::default();
    for &seed in seeds {
        if visited.contains(&seed) || !is_part(state, reflection, setup, seed) {
            continue;
        }
        let Some(found) = component(state, reflection, setup, seed) else {
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
        // (settled from the roots, every part is where its joints put it)
        found.settled.set(true);
    }
}

/// A property changed through RawSet (instance::notify_changed; `old` its previous value, when
/// known): a part's CFrame moves its assembly; a rigid joint's C0 / C1 / Part0 / Part1 / Enabled
/// settle it; a part's Anchored splits or joins its assemblies; a Motor6D's Transform waits for the simulation step. The assemblies
/// it joined, split or rooted elsewhere then take their roots' motion (world/gravity.rs assemblies_changed).
pub fn property_changed(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, name: NameId, old: Option<&Value>) {
    changed(state, reflection, hooks, id, name, old);
    super::gravity::assemblies_changed(state, reflection);
}

fn changed(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, name: NameId, old: Option<&Value>) {
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
    } else if name == setup.c0 || name == setup.c1 || name == setup.part0 || name == setup.part1 || name == setup.enabled || Some(name) == setup.current_angle {
        let kind = setup.kind(state.dom.at(id).class);
        if kind == Kind::Other {
            return;
        }
        if Some(name) == setup.current_angle {
            note_turning(state, reflection, &setup, id);
        }
        // (a C0, C1 or CurrentAngle write places parts and links nothing new)
        let placing = name == setup.c0 || name == setup.c1 || Some(name) == setup.current_angle;
        ensure_scanned(state, reflection, &setup);
        // (the assemblies of the parts it linked and links read their motion again once the write is done: a C0 or C1 moves the parts
        // it places within their assembly too)
        if state.joints.motion {
            let old = state.joints.indexed.get(&id).map(|entry| entry.parts).unwrap_or_default();
            for part in old.into_iter().chain(parts_of(state, &setup, id)).flatten() {
                state.joints.regroup(part);
            }
        }
        // (a joint re-pointed to other parts: the link it made ends and the one it makes starts, as two link changes would)
        if (name == setup.part0 || name == setup.part1)
            && state.joints.quiet == 0
            && state.joints.batch == 0
            && !state.joints.indexed.get(&id).is_some_and(|old| old.parts == parts_of(state, &setup, id))
        {
            repointed(state, reflection, hooks, &setup, id, kind);
            refresh_attached(state, reflection, &setup, id, None);
            return;
        }
        index_joint(state, reflection, &setup, id);
        let links = name == setup.part0 || name == setup.part1 || name == setup.enabled && state.dom.boolean(id, setup.enabled) != Some(false);
        let flips = if !placing { relink(state, reflection, &setup, id) } else { [None, None] };
        if !placing {
            refresh_attached(state, reflection, &setup, id, None);
        }
        if kind == Kind::WeldConstraint && links {
            derive_weld(state, reflection, &setup, id);
        }
        // (enabling or disabling a joint that links two parts: their kept components join or split)
        if let (true, Some((first, second, on))) = (name == setup.enabled && state.joints.quiet == 0 && state.joints.batch == 0, clean_flip(&flips)) {
            link_changed(state, reflection, hooks, &setup, id, [first, second], on);
            return;
        }
        drop_flipped(&mut state.joints, &flips);
        if state.joints.quiet > 0 {
            return;
        }
        if state.joints.batch > 0 {
            state.joints.batch_seeds.extend(parts_of(state, &setup, id).into_iter().flatten());
            return;
        }
        // (a write that neither made nor broke a link on a joint that links nothing cannot move any part)
        if flips.iter().all(Option::is_none) && linked_pair(&state.joints, id).is_none() {
            return;
        }
        let seeds: Vec<InstanceId> = parts_of(state, &setup, id).into_iter().flatten().collect();
        // (an Enabled write that links nothing new places nothing in settled components, whose parts are where their joints put them)
        if name == setup.enabled && !seeds.is_empty() && seeds.iter().all(|seed| state.joints.components.get(seed).is_some_and(|found| found.settled.get() && found.derived.get())) {
            return;
        }
        settle_from_roots(state, reflection, hooks, &setup, &seeds);
    } else if name == setup.anchored {
        // (the assemblies as they were, which the write set aside, and whether the component was settled: stored)
        let (before, settled) = match state.joints.before_anchor.take().filter(|&(part, ..)| part == id) {
            Some((_, tree, settled)) => (tree, settled),
            None => (None, false),
        };
        // Anchoring splits or joins assemblies: a joint that links up again moves what hangs below it
        if state.joints.quiet > 0 || !reflection.class(state.dom.at(id).class).is_a(setup.base_part) || !state.joints.by_part.contains_key(&id) {
            return;
        }
        if state.joints.batch > 0 {
            state.joints.batch_seeds.push(id);
            return;
        }
        ensure_scanned(state, reflection, &setup);
        reanchored(state, reflection, hooks, &setup, id, before, settled);
        // (each of its component's assemblies may change: the part's, and of several anchored parts each one's, whose parts claims
        // gives again)
        if state.joints.motion {
            state.joints.regroup(id);
            if let Some(found) = component(state, reflection, &setup, id) {
                state.joints.regrouped.extend(found.anchored.borrow().iter().copied());
            }
        }
    } else if Some(name) == setup.desired_angle || Some(name) == setup.max_velocity {
        // (a legacy motor given somewhere to turn to, or a speed: step_motors)
        note_turning(state, reflection, &setup, id);
    } else if name == setup.transform {
        let is_motor = setup.motor6d.is_some_and(|motor| reflection.class(state.dom.at(id).class).is_a(motor));
        if is_motor && !state.joints.pending.contains(&id) {
            state.joints.pending.push(id);
        }
        if is_motor {
            state.joints.transformed.insert(id);
        }
        // (its parts are no longer where it puts them until the simulation step)
        if is_motor {
            for part in parts_of(state, &setup, id).into_iter().flatten() {
                if let Some(found) = state.joints.components.get(&part) {
                    found.settled.set(false);
                }
            }
        }
    }
}

/// An Enabled write linked (`on`) or unlinked the two parts `parts` (Part0's first) of `joint`: their kept components join or split
/// ([`Joints::join`], [`Joints::unlink`]) instead of being walked again, and what the write moves is placed: the side a split leaves
/// without its root, or the side a join hangs from the other's root, each part where the joints put it, as settling the whole
/// assemblies from their roots would (the parts whose paths from their root did not change, in a settled component, have nothing to
/// place). Components that are not kept, or whose roots the change moves, are walked and settled again as before. The work clock
/// charges the link change ([`charge_link`]).
fn link_changed(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, joint: InstanceId, parts: [InstanceId; 2], on: bool) {
    let seeds: Vec<InstanceId> = parts_of(state, setup, joint).into_iter().flatten().collect();
    let places = state.joints.indexed[&joint].places;
    state.joints.unsettled = None;
    if on {
        link_started(state, reflection, hooks, setup, joint, parts, &seeds, true);
    } else {
        if state.joints.unlink(joint, parts, places).is_none() {
            drop_flipped(&mut state.joints, &[Some((parts[0], false)), Some((parts[1], false))]);
        }
        settle_unsettled(state, reflection, hooks, setup, &seeds);
    }
    let rigid = parts.iter().all(|&part| anchored(state, setup, part));
    let joint = ordered(&state.joints, joint);
    charge_link(state, reflection, setup, &parts, Change { ends: !on, starts: on, links: 1, joined: 0, rigid, joint, parallel: 0 });
}

/// The link of `joint` between `parts` (Part0's first) started: their kept components join ([`Joints::join`]) and the side that
/// hangs from the other's root is placed ([`attach`]), or, when the components are not kept or the join moves their roots, the
/// assemblies of `seeds` settle from their roots.
fn link_started(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, joint: InstanceId, parts: [InstanceId; 2], seeds: &[InstanceId], hold: bool) {
    let places = state.joints.indexed[&joint].places;
    let plan = attach_plan(state, reflection, setup, joint, parts);
    let anchored_parts = parts.map(|part| anchored(state, setup, part));
    // (the parts' assemblies the side that keeps its roots kept, for the joined component: taken before the join forgets them)
    let roots = plan.as_ref().and_then(|plan| plan.kept.as_ref()).and_then(|kept| kept.take_roots());
    match state.joints.join(joint, parts, places, anchored_parts, None, None) {
        Some(found) => match plan {
            Some(plan) => attach(state, reflection, hooks, setup, &found, plan, roots),
            None => settle_from_roots(state, reflection, hooks, setup, seeds),
        },
        None => {
            drop_flipped(&mut state.joints, &[Some((parts[0], true)), Some((parts[1], true))]);
            settle_from_roots(state, reflection, hooks, setup, seeds);
        }
    }
    // (a client works out the motion of the end the joined side hangs from, and keeps it in each part from the root to that end; a
    // write that starts several links, as a model of welded parts entering, leaves none)
    if hold && state.joints.motion {
        let [first, second] = parts.map(|part| root_path(state, reflection, part));
        if first.first().is_some() && first.first() == second.first() {
            super::gravity::hold_path(state, if first.len() <= second.len() { &first } else { &second });
        }
    }
}

/// The parts from the root of `part`'s assembly down its tree to `part` (the root first, `part` last; `part` alone when it links no
/// other part): the path a client works a part's motion out along (world/gravity.rs hold_path).
pub(crate) fn root_path(state: &mut State, reflection: &Reflection, part: InstanceId) -> Vec<InstanceId> {
    let Some(setup) = setup_of(state) else { return vec![part] };
    let Some(found) = component(state, reflection, &setup, part) else { return vec![part] };
    let path = match found.kept_path(part) {
        Some(path) => path,
        None => tree_path(&assemblies_of(state, reflection, &setup, &found, part, None, true), part).unwrap_or_default(),
    };
    let mut parts: Vec<InstanceId> = path.iter().map(|&(_, parent)| parent).collect();
    parts.push(part);
    parts
}

/// The links of `started` (each joint with its parts, from one Parent write into a world) but the one at `skip` (left for
/// [`link_started`]) join their parts' components in turn ([`Joints::join`]), a part in none whose links all started a component of
/// one at its first, so the components are kept, not walked again: what it costs follows the parts that joined (the smaller side of
/// each join), not the components they joined. When a part's component is not kept (or none are), every component the links touch
/// goes, for the next walk.
fn join_started(state: &mut State, setup: &Setup, started: &[(InstanceId, [InstanceId; 2])], skip: usize) {
    let mut count: FxHashMap<InstanceId, usize> = FxHashMap::default();
    for (_, parts) in started {
        for &part in parts {
            *count.entry(part).or_default() += 1;
        }
    }
    let joints = &state.joints;
    let fresh: FxHashSet<InstanceId> = count
        .iter()
        .filter(|&(part, &links)| !joints.components.contains_key(part) && joints.by_part.get(part).map_or(0, |list| list.linking) == links)
        .map(|(&part, _)| part)
        .collect();
    let mut later = Later::default();
    for (index, &(joint, parts)) in started.iter().enumerate() {
        if index == skip {
            continue;
        }
        let places = state.joints.indexed[&joint].places;
        let anchored_parts = parts.map(|part| anchored(state, setup, part));
        if state.joints.join(joint, parts, places, anchored_parts, Some(&fresh), Some(&mut later)).is_none() {
            for &(_, parts) in started {
                for part in parts {
                    state.joints.drop_component(part);
                }
            }
            return;
        }
    }
    state.joints.insert_later(later);
}

/// The assemblies of `seeds` settle from their roots, but in a settled component (a side that kept its root and its paths), which
/// places nothing.
fn settle_unsettled(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, seeds: &[InstanceId]) {
    let unsettled: Vec<InstanceId> = seeds.iter().copied().filter(|seed| !state.joints.components.get(seed).is_some_and(|found| found.settled.get())).collect();
    settle_from_roots(state, reflection, hooks, setup, &unsettled);
}

/// A Part0 or Part1 write named other parts for `joint` (of `kind`; not the assembly code's own, nor inside a batch): the link it
/// made between its old parts ends, which leaves their kept component, split when nothing else links its sides ([`Joints::unlink`]),
/// and the link it makes between its new parts starts ([`link_started`]), instead of the components being walked again; the new
/// parts' assemblies are settled where they are not known to be. A client spends on it what it spends on a joint disabled or
/// enabled ([`charge_link`], the old parts' and the new parts' components counted): Studio 0.747, a leaf's weld re-pointed in a
/// star of 10,000 parts 874 us at a frame's start, its Part1 set to nil 885, its Part0 re-pointed 926, a later one 10 to 17 us.
fn repointed(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, joint: InstanceId, kind: Kind) {
    let ended = linked_pair(&state.joints, joint);
    // (the order it named its old pair in, which the parallel joints left there are weighed against: charge_link)
    let before = ordered(&state.joints, joint);
    let flips = reindex(state, reflection, setup, joint, true);
    if kind == Kind::WeldConstraint {
        derive_weld(state, reflection, setup, joint);
    }
    state.joints.unsettled = None;
    let started = match clean_flip(&flips) {
        Some((first, second, true)) => Some([first, second]),
        _ => None,
    };
    match started {
        Some(parts) => {
            let seeds: Vec<InstanceId> = parts_of(state, setup, joint).into_iter().flatten().collect();
            link_started(state, reflection, hooks, setup, joint, parts, &seeds, true);
        }
        None => {
            drop_flipped(&mut state.joints, &flips);
            if ended.is_some() || flips.iter().any(Option::is_some) {
                let seeds: Vec<InstanceId> = parts_of(state, setup, joint).into_iter().flatten().collect();
                settle_unsettled(state, reflection, hooks, setup, &seeds);
            }
        }
    }
    if ended.is_some() || started.is_some() {
        let (ends, starts) = (ended.is_some(), started.is_some());
        let parts: Vec<InstanceId> = ended.into_iter().flatten().chain(started.into_iter().flatten()).collect();
        let rigid = parts.iter().all(|&part| anchored(state, setup, part));
        let joint = if starts { ordered(&state.joints, joint) } else { before };
        charge_link(state, reflection, setup, &parts, Change { ends, starts, links: 1, joined: 0, rigid, joint, parallel: 0 });
    }
}

/// How a join places the side that hangs from the other's root ([`attach`]).
struct Attach {
    /// The end of the joint on the side that keeps its root, and its CFrame relative to that root through the joints
    from: InstanceId,
    from_relative: Affine3A,
    /// The end on the side that hangs from it
    to: InstanceId,
    joint: InstanceId,
    root: InstanceId,
    /// The kept component on the side that keeps its root, whose parts' assemblies the joined component keeps ([`KeptRoots`])
    kept: Option<Rc<Component>>,
}

/// Before `joint` joins the components of `parts`: how the join places what it moves, when one side keeps its roots (it holds an
/// anchored part and the other none, or, none anchored on either, its best ranks better than the other's), it was settled, and the
/// other side hangs from the root of the assembly its end is in (that side's one anchored part or its best; of several anchored,
/// the root its kept tree or kept roots give): then no part of that side moves and the other hangs from its end. None otherwise
/// (the joined component is settled from its roots).
fn attach_plan(state: &mut State, reflection: &Reflection, setup: &Setup, joint: InstanceId, parts: [InstanceId; 2]) -> Option<Attach> {
    let kept = parts.map(|part| state.joints.components.get(&part).cloned());
    if let [Some(first), Some(second)] = &kept
        && Rc::ptr_eq(first, second)
    {
        return None;
    }
    let anchored_count = |side: usize| match &kept[side] {
        Some(found) => found.anchored.borrow().len(),
        None => usize::from(anchored(state, setup, parts[side])),
    };
    let root_side = match (anchored_count(0), anchored_count(1)) {
        (0, 0) => {
            let best = |side: usize| match &kept[side] {
                Some(found) => found.best.get(),
                None => Some(parts[side]),
            };
            let (first, second) = (best(0)?, best(1)?);
            let (rank_first, rank_second) = (root_rank(state, reflection, setup, first), root_rank(state, reflection, setup, second));
            if rank_first.2.is_nan() || rank_second.2.is_nan() {
                return None;
            }
            let serial = |part: InstanceId| state.dom.at(part).serial;
            let better = rank_second.0 != rank_first.0 && rank_second.0
                || rank_second.0 == rank_first.0
                    && (rank_second.1 > rank_first.1 || rank_second.1 == rank_first.1 && (rank_second.2 > rank_first.2 || rank_second.2 == rank_first.2 && serial(second) > serial(first)));
            usize::from(better)
        }
        (_, 0) => 0,
        (0, _) => 1,
        _ => return None,
    };
    let from = parts[root_side];
    let root = match &kept[root_side] {
        Some(found) => {
            if !found.settled.get() || !found.derived.get() {
                return None;
            }
            match found.single_root() {
                Some(root) => root,
                None => found.kept_root(from)?,
            }
        }
        None => parts[root_side],
    };
    // (the joined side's end's CFrame relative to the root: through its tree, from the root down, as the settle computes it)
    let from_relative = if from == root {
        Affine3A::IDENTITY
    } else {
        let found = kept[root_side].clone()?;
        let path = match found.kept_path(from) {
            Some(path) => path,
            None => tree_path(&assemblies_of(state, reflection, setup, &found, from, None, true), from)?,
        };
        path.iter().fold(Affine3A::IDENTITY, |relative, &(link, parent)| relative * across(state, reflection, setup, link, parent))
    };
    Some(Attach { from, from_relative, to: parts[1 - root_side], joint, root, kept: kept[root_side].clone() })
}

/// A join's other side hangs from `plan`'s root ([`attach_plan`]): a walk from its end of the joint, the order a walk from the root
/// meets its parts in, works out each part's CFrame through the joints (its WeldConstraints that never took their parts' relative
/// CFrame take it first) and places those not where they put it, as settling the joined assembly from its root would; the rest of
/// the assembly keeps its paths and has nothing to place. The joined component is settled, its root known.
fn attach(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, found: &Rc<Component>, plan: Attach, roots: Option<KeptRoots>) {
    // (the hanging side's parts, each with its parent's index and the joint to it)
    let mut order: Vec<(InstanceId, usize, InstanceId)> = vec![(plan.to, usize::MAX, plan.joint)];
    {
        let links = found.links.borrow();
        let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
        seen.insert(plan.from);
        seen.insert(plan.to);
        let mut index = 0;
        while index < order.len() {
            let part = order[index].0;
            for &(link, other) in links.get(&part).map(Vec::as_slice).unwrap_or_default() {
                if seen.insert(other) {
                    order.push((other, index, link));
                }
            }
            index += 1;
        }
    }
    let welds: Vec<InstanceId> = {
        let links = found.links.borrow();
        order
            .iter()
            .flat_map(|&(part, ..)| links.get(&part).map(Vec::as_slice).unwrap_or_default().iter().map(|&(link, _)| link))
            .filter(|&link| setup.kind(state.dom.at(link).class) == Kind::WeldConstraint && !weld_valid(state, setup, link))
            .collect()
    };
    for weld in welds {
        derive_weld(state, reflection, setup, weld);
    }
    let mut relative: Vec<Affine3A> = Vec::with_capacity(order.len());
    for &(_, parent, link) in &order {
        relative.push(if parent == usize::MAX {
            plan.from_relative * across(state, reflection, setup, link, plan.from)
        } else {
            relative[parent] * across(state, reflection, setup, link, order[parent].0)
        });
    }
    let root = cframe_of(state, setup, plan.root);
    for (index, &(part, ..)) in order.iter().enumerate() {
        let target = root * relative[index];
        if !settled(&cframe_of(state, setup, part), &target) {
            place(state, reflection, hooks, setup, part, &target);
        }
    }
    if found.anchored.borrow().is_empty() {
        found.best.set(Some(plan.root));
    }
    found.settled.set(true);
    found.derived.set(true);
    // (the side that kept its roots keeps its parts' assemblies; the hung side's parts join the assembly of its end)
    if let Some(mut kept) = roots
        && let Some(assembly) = kept.assembly(plan.from)
    {
        kept.joined.extend(order.iter().map(|&(part, ..)| (part, assembly)));
        kept.sizes[assembly as usize] += order.len() as isize;
        *found.roots.borrow_mut() = Some(kept);
    }
}

/// The work clock's price of a link change of `parts` (an Enabled write that linked or unlinked them, an Anchored write of a part
/// that links others), as a client spends it (cost.rs LINK_CHANGED, LINK_FRAME, link_assembly): each change, the frame's first,
/// and the first of each assembly in the frame, each part of it ([`mark_component`]: the assemblies a client keeps, as they are
/// after the change). Each assembly's parts are marked together when it is charged, so an assembly the frame's changes split or
/// joined is charged once, and parts joined to one charged earlier in the frame are charged alone. `ends` and `starts`: whether it
/// ended a link and whether it started one; the frame's first change that ends a link, when it is not the frame's first change,
/// its second that ends one before any starts one, and its first that starts one after one ended one (less when one started
/// before), cost more (cost.rs LINK_SPLIT_FIRST, LINK_SPLIT_AGAIN, LINK_REJOINED, LINK_REJOINED_AFTER_JOIN), whichever assemblies
/// they change. Its reads then owe the body's working out ([`pay_body`]). A write that ended or started several links at once pays
/// for each beyond its first, each further piece holding no anchored part that it split off or joined, and each link it started
/// between two parts another of its links joins (cost.rs links_more, [`Change`]). Links between two anchored parts, each an
/// assembly of its own, change no assembly: a write whose links all are pays the changes alone, nothing for the frame or the
/// assemblies, and marks nothing; so does a single link that another joint between the same two parts keeps ([`parallel_links`]),
/// but for the last of them to name the pair ending, which pays LINK_KEPT_PARALLEL for each it keeps besides (a client picks the
/// link again then). While [`Joints::unpriced`] is positive (UnjoinFromOutsiders' removals, whose UNJOIN_REMOVED holds each
/// change's flat prices: LINK_CHANGED, LINK_SPLIT_FIRST, LINK_SPLIT_AGAIN and LINK_REJOINED) a change that ends a single link
/// charges the rest: the frame's first change, each assembly's first change and each part it marks, and each further link and piece
/// (LINK_FRAME, link_assembly, links_more), the marks and the reads owed kept as ever, as a client's call pays each split's
/// assembly price as the same Welds' Parent = nil in a loop does (Studio 0.747, three sessions, the geometric mean of their
/// medians: 300 and 1,000 listed anchored parts, each welded to an unanchored part of its own, the call 2.0 and 6.4 ms, the loop
/// 1.6 and 4.8 ms; ree reads x0.86 to x1.03 and x0.82 to x1.23 at 100 to 3,000, where each fresh assembly's 5 us read x2.0 to
/// x3.1). A removal that ends several links at once (a joint carrying others, whose pieces each pay their assembly's price) pays
/// every link price, as the same Parent write does.
fn charge_link(state: &mut State, reflection: &Reflection, setup: &Setup, parts: &[InstanceId], change: Change) {
    let Change { ends, starts, links, joined, rigid, joint, parallel } = change;
    let flat = state.joints.unpriced == 0 || links > 1;
    let mut work = if flat { crate::cost::LINK_CHANGED } else { 0.0 };
    // (a single link between two parts that other joints link as well: a client re-picks the link only when the last made of them
    // ends, walking the others; any other ends or starts as a change between anchored parts does, no assembly changing)
    let kept = match (joint, parts) {
        (Some((joint, order)), &[first, second]) if links == 1 && !rigid => parallel_links(&state.joints, joint, order, [first, second]),
        _ => None,
    };
    if rigid || kept.is_some_and(|(_, later)| starts || later) {
        work += crate::cost::links_more(links.saturating_sub(1), 0, 0);
        state.sched.charge(work);
        crate::boundary::add_cost(work);
        return;
    }
    if let Some((others, _)) = kept {
        work += others as f64 * crate::cost::LINK_KEPT_PARALLEL;
    }
    let first = link_frame(&mut state.joints, state.sched.frame);
    if first {
        work += crate::cost::LINK_FRAME;
    }
    let joints = &mut state.joints;
    if ends {
        if flat && !first && joints.splits == 0 {
            work += crate::cost::LINK_SPLIT_FIRST;
        } else if flat && joints.splits == 1 && joints.joins == 0 {
            work += crate::cost::LINK_SPLIT_AGAIN;
        }
        joints.splits += 1;
    }
    if starts {
        if joints.splits > 0 && !joints.rejoined {
            if flat {
                work += if joints.joins == 0 { crate::cost::LINK_REJOINED } else { crate::cost::LINK_REJOINED_AFTER_JOIN };
            }
            joints.rejoined = true;
        }
        joints.joins += 1;
    }
    if links > 1 {
        let pieces = if ends { split_pieces(state, reflection, setup, parts) } else { joined.saturating_sub(1) };
        work += crate::cost::links_more(links - 1, pieces, parallel);
    }
    // (a part met again is marked: its assembly is not charged twice)
    for &part in parts {
        state.joints.owed.insert(part);
        if let Some(marked) = mark_component(state, reflection, setup, part) {
            work += crate::cost::link_assembly(marked.parts, marked.free, marked.anchored);
        }
    }
    state.sched.charge(work);
    crate::boundary::add_cost(work);
}

/// A link change [`charge_link`] prices: whether it ended links and whether it started them; how many links reaching a part in a
/// world it ended or started (one write's, at once) and, of links it started, how many unanchored parts in a world it joined that no
/// other link held (each a piece of its own until then); whether each of those links is between two anchored parts; the joint of a
/// change of one link, when known; and of the links one write started between parts that were in the world, how many link two
/// parts another of them links.
#[derive(Clone, Copy, Default)]
struct Change {
    ends: bool,
    starts: bool,
    links: usize,
    joined: usize,
    rigid: bool,
    joint: Option<(InstanceId, u64)>,
    parallel: usize,
}

/// The joints other than `joint` that link `parts` to each other now (parallel joints, [`Joints::pairs`]): how many, and whether one
/// of them named the pair after `joint` did (`order`, its [`pair_order`] when the change began). None when no other joint links them.
/// A few map lookups, however many joints the parts or the pair hold.
fn parallel_links(joints: &Joints, joint: InstanceId, order: u64, parts: [InstanceId; 2]) -> Option<(usize, bool)> {
    let (count, later) = match joints.pairs.get(&pair_key(parts))? {
        PairLinks::One(_, other) if *other == joint => return None,
        PairLinks::One(was, _) => (1, *was > order),
        PairLinks::Many(many) => (many.len() - usize::from(many.get(&order) == Some(&joint)), many.keys().next_back().is_some_and(|&last| last > order)),
    };
    (count > 0).then_some((count, later))
}

/// `joint` with its [`pair_order`] now, for a [`Change`] of its link; None when it is not indexed.
fn ordered(joints: &Joints, joint: InstanceId) -> Option<(InstanceId, u64)> {
    joints.indexed.get(&joint).map(|entry| (joint, pair_order(entry)))
}

/// The pieces a write that ended several links split off that hold no anchored part, which pay [`crate::cost::LINK_PIECE`] each
/// ([`charge_link`]): of the components `parts` are in after it (each part's kept one, else a walk's; one alone a piece of its own),
/// those holding no anchored part, but one; a piece the frame had not changed pays its assembly's price too
/// ([`crate::cost::link_assembly`]). The same pieces however the components are kept.
fn split_pieces(state: &State, reflection: &Reflection, setup: &Setup, parts: &[InstanceId]) -> usize {
    let mut kept: FxHashSet<*const Component> = FxHashSet::default();
    let mut walked: FxHashSet<InstanceId> = FxHashSet::default();
    let mut pieces = 0usize;
    for &part in parts {
        if walked.contains(&part) {
            continue;
        }
        let free = match state.joints.components.get(&part) {
            Some(found) => {
                if !kept.insert(Rc::as_ptr(found)) {
                    continue;
                }
                found.anchored.borrow().is_empty()
            }
            None => match walk_component(state, reflection, setup, part) {
                Some(found) => {
                    walked.extend(found.links.borrow().keys().copied());
                    found.anchored.borrow().is_empty()
                }
                None => {
                    walked.insert(part);
                    !anchored(state, setup, part)
                }
            },
        };
        pieces += usize::from(free);
    }
    pieces.saturating_sub(1)
}

/// The frame's link changes are `frame`'s: whether they were another's until now (the marks and owed reads go with them).
fn link_frame(joints: &mut Joints, frame: u64) -> bool {
    if joints.link_frame == Some(frame) {
        return false;
    }
    joints.link_frame = Some(frame);
    joints.marked.clear();
    joints.owed.clear();
    (joints.splits, joints.joins, joints.rejoined) = (0, 0, false);
    true
}

/// Marks the parts of the assembly of `part` as a client keeps it as changed in the frame, together; how many of them were not
/// marked yet, when `part` was not: all of them, or, when the change joined parts to an assembly changed earlier in the frame, those
/// that joined it (a client's later change in the frame does not rebuild the assembly again for them: Studio 0.747, a leaf of a star
/// of 10,000 whose weld was disabled a frame before enabled after another leaf's 15.6 us, at 1,000 6.2).
/// They are found by a walk from `part` through the parts not marked (its kept component's links, else the walks' edges): an
/// assembly's parts are marked together, so these
/// are the parts that joined it since, and the walk costs them, not the assembly (it collected every part of the component: an
/// assembly built a weld at a time in a frame cost host time with the square of its parts). Parts not marked that one write joined to
/// it at several places are charged as pieces of their own. The walk stops at a link between two anchored parts (each the root of an
/// assembly of its own) and, once it has met an anchored part, at each other anchored part it meets through unanchored ones (the root
/// of another assembly: Studio 0.747, 2026-11-25, two anchored hubs of 1,000 leaves each, one unanchored part welded to both: a leaf
/// of each changed in turn costs 88 and 69 us, where unbridged hubs' cost 73 and 60), counting those links. An assembly charged
/// earlier in the frame that the walk meets still says whether its parts are anchored.
fn mark_component(state: &mut State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> Option<Marked> {
    if state.joints.marked.contains_key(&part) {
        return None;
    }
    state.joints.marked.insert(part, false);
    let mut found = vec![part];
    let start_fixed = anchored(state, setup, part);
    // (whether none of its parts is anchored, whether it met an anchored part, the links it stopped at)
    let (mut free, mut rooted, mut across) = (!start_fixed, start_fixed, 0usize);
    let mut others: Vec<InstanceId> = Vec::new();
    let mut index = 0;
    while index < found.len() {
        let at = found[index];
        others.clear();
        match state.joints.components.get(&at) {
            Some(kept) => others.extend(kept.links.borrow().get(&at).map(Vec::as_slice).unwrap_or_default().iter().map(|&(_, other)| other)),
            None => others.extend(edges(state, reflection, setup, at).into_iter().map(|(_, other)| other)),
        }
        let fixed = anchored(state, setup, at);
        for &other in &others {
            let other_fixed = anchored(state, setup, other);
            match state.joints.marked.get(&other) {
                // (met already, in this walk or in an assembly charged earlier in the frame, which is this one's when the walk has
                // met no anchored part yet: its anchoring holds for this one)
                Some(&held) => {
                    if !rooted {
                        free &= !held && !other_fixed;
                        rooted = held || other_fixed;
                    }
                }
                None if other_fixed && (fixed || rooted) => across += 1,
                None => {
                    state.joints.marked.insert(other, false);
                    free &= !other_fixed;
                    rooted |= other_fixed;
                    found.push(other);
                }
            }
        }
        index += 1;
    }
    if !free {
        for &found in &found {
            state.joints.marked.insert(found, true);
        }
    }
    Some(Marked { parts: found.len(), free, anchored: across })
}

/// What [`mark_component`] marked: how many parts, whether none of them is anchored, and how many links its anchored parts have to
/// other anchored parts (each an assembly of its own, which the walk stops at).
#[derive(Clone, Copy)]
struct Marked {
    parts: usize,
    free: bool,
    anchored: usize,
}

/// The parts whose assemblies an Anchored write of `part` (of the kept component `found`) changes, for [`charge_link`]: the part,
/// and when it is anchored now, the first anchored part it links, whose assembly it was in (a walk from it stops short of that one
/// now, as it stops at each link between two anchored parts).
fn reanchored_parts(state: &State, setup: &Setup, found: &Component, part: InstanceId) -> Vec<InstanceId> {
    let mut parts = vec![part];
    if anchored(state, setup, part) {
        let links = found.links.borrow();
        parts.extend(links.get(&part).map(Vec::as_slice).unwrap_or_default().iter().map(|&(_, other)| other).find(|&other| anchored(state, setup, other)));
    }
    parts
}

/// A part's Anchored changed: its assemblies settle from their roots, and the work clock charges each other part whose assembly
/// root, or that root's anchoring, the write changed, and each joint linking the part (cost.rs joints_reanchored), beyond the
/// write, and the link change ([`charge_link`]). `before`: the part's component's assemblies as they were, when they were kept;
/// else they are worked out again with its Anchored read the other way. `settled`: whether its component was settled before the
/// write. A part whose links all go to one anchored part (a leaf of an anchored part, as a client's leaf, costs it the same at any
/// size) changes no other part's root: in a settled component it alone is placed, when unanchored, and no tree is worked out.
fn reanchored(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, part: InstanceId, before: Option<Rc<Assemblies>>, settled: bool) {
    let found = component(state, reflection, setup, part);
    if let Some(found) = &found
        && settled
        && leaf_reanchored(state, reflection, hooks, setup, found, part)
    {
        let links = state.joints.by_part.get(&part).map_or(0, |list| list.linking);
        let work = crate::cost::joints_reanchored(0, links);
        if work > 0.0 {
            state.sched.charge(work);
            crate::boundary::add_cost(work);
        }
        let parts = reanchored_parts(state, setup, found, part);
        charge_link(state, reflection, setup, &parts, Change::default());
        return;
    }
    let before = before.or_else(|| found.as_ref().map(|found| assemblies_of(state, reflection, setup, found, part, Some(part), false)));
    settle_from_roots(state, reflection, hooks, setup, &[part]);
    if let Some(found) = &found {
        let parts = reanchored_parts(state, setup, found, part);
        charge_link(state, reflection, setup, &parts, Change::default());
    }
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

/// An Anchored write of `part` in the settled component `found` whose links all go to one anchored part: the write changes no other
/// part's root (anchored, the part is an assembly of its own, nothing hanging from it; unanchored, it hangs from that part alone), so
/// the part, unanchored, is placed where its first joint in that part's list puts it from that part, as settling the component
/// would, and nothing else moves. False (nothing done) for any other part.
fn leaf_reanchored(state: &mut State, reflection: &Reflection, hooks: &Hooks, setup: &Setup, found: &Rc<Component>, part: InstanceId) -> bool {
    let (joint, neighbour) = {
        let links = found.links.borrow();
        let Some(list) = links.get(&part) else { return false };
        let Some(&(_, neighbour)) = list.first() else { return false };
        if neighbour == part || list.iter().any(|&(_, other)| other != neighbour) || !anchored(state, setup, neighbour) || !found.derived.get() {
            return false;
        }
        // (the joint a walk from that part takes to it: the first in its list)
        let first = list.iter().map(|&(joint, _)| joint).min_by_key(|&joint| state.joints.indexed_place(joint, neighbour));
        let Some(first) = first else { return false };
        (first, neighbour)
    };
    if !anchored(state, setup, part) {
        let target = cframe_of(state, setup, neighbour) * (Affine3A::IDENTITY * across(state, reflection, setup, joint, neighbour));
        if !settled(&cframe_of(state, setup, part), &target) {
            place(state, reflection, hooks, setup, part, &target);
        }
    }
    found.settled.set(true);
    true
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
    let unsettled = state.joints.unsettled.take();
    let Some(found) = component(state, reflection, setup, part) else { return };
    let tree = assemblies_of(state, reflection, setup, &found, part, None, true);
    let Some((assembly, index, node)) = tree.of(part) else { return };
    let (carried, whole) = match node {
        _ if assembly.nodes.len() < 2 => (0, true),
        0 => settle(state, reflection, hooks, setup, assembly, Anchor::Root, false),
        node => settle(state, reflection, hooks, setup, assembly, Anchor::Moved(node, before), false),
    };
    // (the moved assembly is where its joints put it now, when the part written is; the others are as they were before the write)
    if whole && (tree.list.len() == 1 || unsettled == Some(part)) {
        found.settled.set(true);
    }
    refresh_centres(state, setup, &found);
    let across = if state.joints.bulk > 0 { 0 } else { tree.across[index] as usize };
    let work = crate::cost::assembly_moved(carried, across);
    if work > 0.0 {
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
}

/// The order a client takes up a subtree a Parent write moved, `root` the moved instance (Studio 0.748, a playing client and its
/// server, through the lists of active joints: tools/parity/probes/unjoin_from_outsiders.luau section 6): `root` first, then
/// the rest from the bottom up, each instance after its own descendants, children in their order (of a Folder holding `[x2,
/// Sub [x3]]`: the Folder, x2, x3, Sub). This is neither Dom::descendants' order (each instance before its descendants) nor
/// Dom::spread_home's (which takes the last child first), and it matters: it is the order joints leave and join their parts'
/// lists. The walk does not go below an instance `stop` marks (it takes that instance itself up).
fn taken_up(state: &State, root: InstanceId, stop: impl Fn(&State, InstanceId) -> bool) -> Vec<InstanceId> {
    let mut order = vec![root];
    if stop(state, root) {
        return order;
    }
    // (each instance on the way down, with the next of its children to go into)
    let mut path = vec![(root, 0usize)];
    while let Some(&(node, next)) = path.last() {
        match state.dom.children_of(node).slice().get(next).copied() {
            Some(child) => {
                if let Some(top) = path.last_mut() {
                    top.1 += 1;
                }
                if stop(state, child) {
                    order.push(child);
                } else {
                    path.push((child, 0));
                }
            }
            None => {
                path.pop();
                if node != root {
                    order.push(node);
                }
            }
        }
    }
    order
}

/// An instance's parent changed (instance::set_parent), and with it the home of what it carries:
/// - Into a world, or from one world into another (or into JointsService): joints and parts entering a WorldRoot link up, and
///   their assemblies settle from their roots. The links that start join the kept components instead of their being walked again,
///   as a joint enabled does: one link through [`link_started`] (the side that hangs from the other's root placed alone), several
///   in turn ([`join_started`]) before the joined components settle; the other components the instances it carries touch settle
///   where they are not known to be settled, in the order the write meets them. The joints it carries are indexed in that order
///   (Dom::descendants'), which the walks follow; then the joints join and leave their parts' lists of active joints in the order a
///   client takes the subtree up ([`relist_moved`]). The work clock charges what a client spends ([`entering_price`],
///   [`relist_moved`]), beyond what the move pays for the instances it carries.
/// - Out of the world or JointsService it was in: the links that ended with it end ([`left_world`]); out of every world, the
///   joints it carries, and those of the parts it carries, leave their parts' lists of active joints, in the walk's order
///   ([`left_every_world`]).
/// - The assemblies it joined, split or rooted elsewhere then take their roots' motion (world/gravity.rs assemblies_changed).
pub fn reparented(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, old_parent: Option<InstanceId>, new_parent: Option<InstanceId>) {
    moved_home(state, reflection, hooks, id, old_parent, new_parent);
    if state.joints.motion
        && let Some(setup) = setup_of(state)
    {
        moved_regroup(state, reflection, &setup, id);
    }
    super::gravity::assemblies_changed(state, reflection);
}

/// The parts whose assemblies a Parent write of `id` may have joined or split, while some part moves ([`Joints::regrouped`]): each
/// part it carries and the parts its indexed joints name, and the parts of each joint it carries.
fn moved_regroup(state: &mut State, reflection: &Reflection, setup: &Setup, id: InstanceId) {
    let mut nodes = vec![id];
    nodes.extend(state.dom.descendants(id));
    let mut found = Vec::new();
    for node in nodes {
        let class = state.dom.at(node).class;
        if setup.kind(class) != Kind::Other {
            found.extend(parts_of(state, setup, node).into_iter().flatten());
        } else if reflection.class(class).is_a(setup.base_part) {
            found.push(node);
            if let Some(list) = state.joints.by_part.get(&node) {
                for &joint in list.slots.iter().flatten() {
                    found.extend(parts_of(state, setup, joint).into_iter().flatten());
                }
            }
        }
    }
    state.joints.regrouped.extend(found);
}

fn moved_home(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, old_parent: Option<InstanceId>, new_parent: Option<InstanceId>) {
    let Some(setup) = setup_of(state) else { return };
    let home = |state: &State, node: InstanceId| {
        let class = state.dom.at(node).class;
        if reflection.class(class).is_a(setup.world_root) || Some(class) == setup.joints_service {
            Some(node)
        } else {
            joint_home(state, node)
        }
    };
    let old_home = old_parent.and_then(|parent| home(state, parent));
    let new_home = new_parent.and_then(|parent| home(state, parent));
    let open = state.joints.quiet == 0 && state.joints.batch == 0;
    // (the links that start: taken with the homes the write logged, and from the joints it carries that are indexed now)
    let mut started: Vec<(InstanceId, [InstanceId; 2])> = Vec::new();
    let entering = new_home.is_some_and(|world| old_home != Some(world));
    if old_home.is_some() && old_home != new_home && open {
        left_world(state, reflection, &setup, entering.then_some(&mut started));
    }
    if old_home.is_some() && new_home.is_none() {
        left_every_world(state, id);
    }
    let Some(world) = new_home.filter(|_| entering) else { return };
    scan_and_take(state, reflection, &setup, open.then_some(&mut started));
    let mut seeds = Vec::new();
    // (the joints it carries and its parts that joints name: each link it makes has one of them; and how many of those joints it
    // brings into this world. A client spends on a Glue brought in what it spends on a Weld: Studio 0.748, a Folder of 10,000
    // disabled Glue or Welds parented back into the Workspace 1.69 to 1.77 us and 1.68 to 1.75 us a joint)
    let mut moved: Vec<InstanceId> = Vec::new();
    let mut entered = 0usize;
    let mut nodes = vec![id];
    nodes.extend(state.dom.descendants(id));
    for node in nodes {
        let class = state.dom.at(node).class;
        let kind = setup.kind(class);
        if kind != Kind::Other {
            let flips = reindex(state, reflection, &setup, node, false);
            match clean_flip(&flips) {
                Some((first, second, true)) if open => started.push((node, [first, second])),
                _ => drop_flipped(&mut state.joints, &flips),
            }
            if kind.rigid() {
                seeds.extend(parts_of(state, &setup, node).into_iter().flatten());
            }
            moved.push(node);
            entered += usize::from(joint_home(state, node) == Some(world));
        } else if reflection.class(class).is_a(setup.base_part) && state.joints.by_part.contains_key(&node) {
            seeds.push(node);
            moved.push(node);
        }
    }
    // (the lists of active joints, as a client takes the subtree up: none when it carries no joint and no part a joint names, the
    // joints the dom logged then catching up at the next read, ensure_scanned)
    let listed = if moved.is_empty() { 0.0 } else { relist_moved(state, reflection, &setup, id, old_parent, old_home) };
    if state.joints.quiet > 0 {
        return;
    }
    if state.joints.batch > 0 {
        state.joints.batch_seeds.extend(seeds);
        let work = crate::cost::joints_entering(0, 0, entered) + listed;
        if work > 0.0 {
            state.sched.charge(work);
            crate::boundary::add_cost(work);
        }
        return;
    }
    if listed > 0.0 {
        state.sched.charge(listed);
        crate::boundary::add_cost(listed);
    }
    // (one link that starts joins as an enabled joint's does, when the write first meets its component. Of several, when they join
    // kept components larger than they are many, the one to the largest is that link, the others joining in turn first: the parts they
    // join hang from it, placed in the order a settle of the whole would place them (the larger side is settled and keeps its paths),
    // or, when they touch that component too, the joined components settle; else their components are walked again, as a link joined
    // costs about what a part walked does, and a Model of welded parts brings its links in with it)
    let mut single = None;
    let hold = started.len() == 1;
    match started.as_slice() {
        [] => {}
        &[link] => single = Some(link),
        links => {
            let mut touched: FxHashSet<*const Component> = FxHashSet::default();
            let (mut kept, mut largest) = (0usize, (0usize, 0usize));
            for (index, (_, parts)) in links.iter().enumerate() {
                for part in parts {
                    if let Some(found) = state.joints.components.get(part)
                        && touched.insert(Rc::as_ptr(found))
                    {
                        kept += found.size();
                        largest = largest.max((found.size(), usize::MAX - index));
                    }
                }
            }
            if kept > links.len() {
                let bridge = usize::MAX - largest.1;
                join_started(state, &setup, links, bridge);
                single = Some(links[bridge]);
            } else {
                for &(_, parts) in links {
                    drop_flipped(&mut state.joints, &[Some((parts[0], true)), Some((parts[1], true))]);
                }
            }
        }
    }
    // (the components done, held so none is freed and its address taken by another)
    let mut done: (Vec<Rc<Component>>, FxHashSet<*const Component>) = Default::default();
    let finish = |state: &State, done: &mut (Vec<Rc<Component>>, FxHashSet<*const Component>), part: InstanceId| {
        if let Some(found) = state.joints.components.get(&part)
            && done.1.insert(Rc::as_ptr(found))
        {
            done.0.push(found.clone());
        }
    };
    for &seed in &seeds {
        if !is_part(state, reflection, &setup, seed) || state.joints.components.get(&seed).is_some_and(|found| done.1.contains(&Rc::as_ptr(found))) {
            continue;
        }
        if let Some((joint, parts)) = single {
            let touched = |part: InstanceId| part == seed || state.joints.components.get(&part).is_some_and(|found| state.joints.components.get(&seed).is_some_and(|other| Rc::ptr_eq(found, other)));
            if touched(parts[0]) || touched(parts[1]) {
                single = None;
                // (settled from its roots when it cannot hang one side from the other's, from the part the write met first)
                let ends: Vec<InstanceId> = std::iter::once(seed).chain(parts_of(state, &setup, joint).into_iter().flatten()).collect();
                link_started(state, reflection, hooks, &setup, joint, parts, &ends, hold);
                finish(state, &mut done, parts[0]);
                continue;
            }
        }
        // (a component the write left as it was, settled, has nothing to place)
        if !state.joints.components.get(&seed).is_some_and(|found| found.settled.get() && found.derived.get()) {
            settle_from_roots(state, reflection, hooks, &setup, &[seed]);
        }
        finish(state, &mut done, seed);
    }
    if let Some((joint, parts)) = single {
        let ends: Vec<InstanceId> = parts_of(state, &setup, joint).into_iter().flatten().collect();
        link_started(state, reflection, hooks, &setup, joint, parts, &ends, hold);
    }
    entering_price(state, reflection, &setup, &moved, entered);
}

/// What a Parent write into a world spends on joints, beyond the instances it carries (`moved`: the joints it carries and its parts
/// that joints name, in the order it meets them; `entered`: the joints it brings into the world): each joint in a world it looks at
/// (cost.rs JOINT_LOOKED_AT; the joints in each part's list it carries, and each joint it carries or that links a part it carries,
/// at its part that stays), each link it makes (JOINT_LINKED), each joint it brings in (JOINT_ENTERED), and, when it makes a link, what a
/// client spends on a link change ([`charge_link`]: the frame's first, the first of each assembly in it, each part of it), as a client's
/// Parent write that links does (Studio 0.747, a weld parented back under its leaf of a star of 10,000 parts 920 us at a frame's start
/// and 22 us later, the leaf parented back 933 and 17, where its weld enabled costs 954 and 16; at 1,000 115 and 8, 112 and 9). A write
/// that makes several links reaching parts that were in the world pays for each beyond the first and for each unanchored part it
/// joins that no other link held ([`Change`]).
fn entering_price(state: &mut State, reflection: &Reflection, setup: &Setup, moved: &[InstanceId], entered: usize) {
    let carried: FxHashSet<InstanceId> = moved.iter().copied().collect();
    let (mut looked, mut made, mut reaching, mut rigid) = (0usize, 0usize, 0usize, true);
    // (the parts of the links it made that were in the world already, and a part of each link among the parts it carries; how many
    // links reach a part that was there, and whether each of those is between two anchored parts)
    let (mut placed, mut new): (Vec<InstanceId>, Vec<InstanceId>) = (Vec::new(), Vec::new());
    // (the last joint whose link reaches a part that was there, and the links it made between two such parts, by pair)
    let mut last = None;
    let mut pairs: FxHashMap<[InstanceId; 2], usize> = FxHashMap::default();
    for &node in moved {
        let joints = &state.joints;
        if setup.kind(state.dom.at(node).class) != Kind::Other {
            if joint_home(state, node).is_some() {
                let [first, second] = parts_of(state, setup, node);
                for part in [first, second.filter(|&second| Some(second) != first)].into_iter().flatten() {
                    looked += usize::from(!carried.contains(&part) && world_of(state, reflection, setup, part).is_some());
                }
            }
            if let Some(pair) = linked_pair(joints, node) {
                made += 1;
                if pair.iter().any(|part| !carried.contains(part)) {
                    reaching += 1;
                    rigid &= pair.iter().all(|&part| anchored(state, setup, part));
                    last = Some(node);
                    if pair.iter().all(|part| !carried.contains(part)) {
                        *pairs.entry(if pair[0] < pair[1] { pair } else { [pair[1], pair[0]] }).or_default() += 1;
                    }
                }
                for part in pair {
                    if carried.contains(&part) {
                        new.push(part);
                    } else {
                        placed.push(part);
                    }
                }
            }
            continue;
        }
        let Some(list) = joints.by_part.get(&node) else { continue };
        if world_of(state, reflection, setup, node).is_none() {
            continue;
        }
        looked += list.homed;
        // (each link of it by a joint it does not carry: once, at its end it carries, or at Part0's end when it carries both)
        for joint in list.linking() {
            let Some(entry) = joints.indexed.get(&joint).filter(|_| !carried.contains(&joint)) else { continue };
            let other = if entry.parts[0] == Some(node) { entry.parts[1] } else { entry.parts[0] };
            let Some(other) = other else { continue };
            if !carried.contains(&other) {
                looked += 1;
                made += 1;
                reaching += 1;
                rigid &= anchored(state, setup, node) && anchored(state, setup, other);
                last = Some(joint);
                placed.push(other);
            } else if entry.parts[0] == Some(node) {
                made += 1;
                new.push(node);
            }
        }
    }
    let work = crate::cost::joints_entering(looked, made, entered);
    if work > 0.0 {
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
    // (a link to a part the write found in the world changes that part's assembly, as a link change does; links among the parts it
    // carries make new assemblies, priced by their joints alone, as a client's Model of welded pairs entering costs it 3.1 us a weld
    // beyond its parts, and mark them as changed in the frame, so a later link change of them pays a later change's price)
    if !placed.is_empty() {
        // (the unanchored parts it linked that no other link held: each was a piece of its own)
        let mut linked: FxHashMap<InstanceId, usize> = FxHashMap::default();
        for &part in &placed {
            *linked.entry(part).or_default() += 1;
        }
        let joints = &state.joints;
        let joined = linked.iter().filter(|&(part, &links)| joints.by_part.get(part).is_some_and(|list| list.linking == links) && !anchored(state, setup, *part)).count();
        // (links between two parts another of its links joins cost a client more each: Studio 0.747, N Welds between two parts in a
        // Folder parented back)
        let parallel = pairs.values().map(|count| count - 1).sum();
        let joint = if reaching == 1 { last.and_then(|joint| ordered(&state.joints, joint)) } else { None };
        charge_link(state, reflection, setup, &placed, Change { ends: false, starts: true, links: reaching, joined, rigid, joint, parallel });
    }
    if !new.is_empty() {
        link_frame(&mut state.joints, state.sched.frame);
        for part in new {
            mark_component(state, reflection, setup, part);
        }
    }
}

/// A Parent write (Destroy's too) took instances out of the world or JointsService they were in: the links that ended with it end
/// now, each leaving its parts' kept component, split when nothing else links its sides ([`take_homes`]), instead of the
/// components going and being walked again; and the write costs what a client spends on a link change ([`charge_link`], the
/// components counted of the parts still in a world). Studio 0.747, a leaf of a star of 10,000 parts: its weld's Parent set to nil
/// 864 us at a frame's start, the weld destroyed 883, the leaf's Parent set to nil 982 and the leaf destroyed 929, beyond what the
/// write or destroy costs alone; later ones in the frame 15 to 43 us. A write that ends several links pays for each beyond the first
/// that reaches a part still in a world and for each piece it splits off ([`Change`]). `started`: the links the write starts in the
/// world it enters ([`take_homes`]).
fn left_world(state: &mut State, reflection: &Reflection, setup: &Setup, started: Option<&mut Vec<(InstanceId, [InstanceId; 2])>>) {
    let ended = scan_and_take(state, reflection, setup, started);
    if ended.is_empty() {
        return;
    }
    // (the links that ended reaching a part still in a world, and whether each of those is between two anchored parts)
    let placed_at = |part: InstanceId| alive(state, part) && world_of(state, reflection, setup, part).is_some();
    let (mut links, mut rigid, mut last) = (0usize, true, None);
    for &(joint, pair) in &ended {
        if pair.iter().any(|&part| placed_at(part)) {
            links += 1;
            rigid &= pair.iter().all(|&part| alive(state, part) && anchored(state, setup, part));
            last = Some(joint);
        }
    }
    let placed: Vec<InstanceId> = ended.into_iter().flat_map(|(_, pair)| pair).filter(|&part| placed_at(part)).collect();
    let joint = if links == 1 { last.and_then(|joint| ordered(&state.joints, joint)) } else { None };
    charge_link(state, reflection, setup, &placed, Change { ends: true, starts: false, links, joined: 0, rigid: rigid && links > 0, joint, parallel: 0 });
}

/// The lists of active joints after a Parent write took the subtree of `id` into a world, or from one world into another or into
/// JointsService ([`reparented`]; `old_parent` and `old_home` as they were): an instance at a time, in [`taken_up`]'s order, an
/// instance the walk has not reached yet counting as where it was ([`Moving`]). Each joint it carries joins or leaves its parts'
/// lists as the walk takes it up ([`refresh_attached`]), and so do the joints that name each part it carries, the last named
/// first (Studio 0.748, a part out of the Workspace, or into a WorldModel, and back): only those in a world or JointsService can
/// be active, and of those the ones that become active or are listed now change the lists, the others passed by before the sort;
/// a part in no world (in JointsService), where none of its joints can be active, only takes its active ones out. Then the joints
/// the dom logged catch up ([`list_logged`]), but for those the walk took up, which are as they now are.
///
/// What a playing client spends on it (Studio 0.748; a part with 1,000 or 10,000 Welds or Glue to another moved into a WorldModel
/// of the Workspace and back, into JointsService and back, out of the DataModel and back and into ReplicatedStorage and back, and
/// the Folder holding the joints out of the DataModel and back: Glue as much as Welds): each joint taken out pays what leaving
/// every world pays ([`TakenOut`]); a joint's link is priced with the links ([`entering_price`]). Its price, beyond the write.
fn relist_moved(state: &mut State, reflection: &Reflection, setup: &Setup, id: InstanceId, old_parent: Option<InstanceId>, old_home: Option<InstanceId>) -> f64 {
    let old_world = old_parent.and_then(|parent| {
        if reflection.class(state.dom.at(parent).class).is_a(setup.world_root) { Some(parent) } else { world_of(state, reflection, setup, parent) }
    });
    let order = taken_up(state, id, |_, _| false);
    let mut moving = Moving { places: order.iter().enumerate().map(|(place, &node)| (node, place)).collect(), at: 0, old_home, old_world };
    state.joints.walks += 1;
    let walk = state.joints.walks;
    let stamp = |joints: &mut Joints, joint: InstanceId| {
        if let Some(entry) = joints.indexed.get_mut(&joint) {
            entry.listed = walk;
        }
    };
    // (the joints taken out and the parts whose lists they left)
    let mut out = TakenOut::default();
    for (place, &node) in order.iter().enumerate() {
        moving.at = place;
        let class = state.dom.at(node).class;
        let changing: Vec<InstanceId> = if setup.kind(class) != Kind::Other {
            stamp(&mut state.joints, node);
            vec![node]
        } else if reflection.class(class).is_a(setup.base_part) && state.joints.by_part.contains_key(&node) {
            // (in no world, as in JointsService, none of its joints can be active: only its active ones are looked at, so a part
            // moved between places outside every world costs nothing for the joints that name it)
            if world_of(state, reflection, setup, node).is_none() {
                out.part(&mut state.joints, node);
                continue;
            }
            let homed: Vec<InstanceId> = state.joints.by_part[&node].homed().collect();
            let mut changing = Vec::new();
            for joint in homed {
                if state.joints.attached_parts.contains_key(&joint) || active_parts(state, reflection, setup, joint, Some(&moving)).is_some() {
                    changing.push(joint);
                }
                stamp(&mut state.joints, joint);
            }
            last_named_first(&state.joints, node, changing.into_iter())
        } else {
            continue;
        };
        for joint in changing {
            let refreshed = refresh_attached(state, reflection, setup, joint, Some(&moving));
            if let Some(parts) = refreshed.left {
                out.joint(parts);
            }
        }
    }
    list_logged(state, reflection, setup, Some(walk));
    out.price()
}

/// `joints`, joints indexed under `part`, the last to name it first ([`Indexed::named`]).
fn last_named_first(joints: &Joints, part: InstanceId, of: impl Iterator<Item = InstanceId>) -> Vec<InstanceId> {
    let named = |joint: InstanceId| {
        joints.indexed.get(&joint).map_or(0, |entry| if entry.parts[0] == Some(part) { entry.named[0] } else { entry.named[1] })
    };
    let mut found: Vec<(u64, InstanceId)> = of.map(|joint| (named(joint), joint)).collect();
    crate::sort::by(&mut found, |a, b| b.cmp(a));
    found.into_iter().map(|(_, joint)| joint).collect()
}

/// The active joints a change took out of their parts' lists, and the parts whose lists they left, for its price (cost.rs
/// JOINT_TAKEN_OUT, JOINT_LIST_LEFT).
#[derive(Default)]
struct TakenOut {
    joints: usize,
    lists: FxHashSet<InstanceId>,
}

impl TakenOut {
    /// A joint left the lists of `parts`.
    fn joint(&mut self, parts: [InstanceId; 2]) {
        self.joints += 1;
        self.lists.extend(parts);
    }

    /// Takes the active joints of `part` out of their parts' lists, the last named first (it is in no world now).
    fn part(&mut self, joints: &mut Joints, part: InstanceId) {
        let Some(list) = joints.attached.get(&part) else { return };
        let order = last_named_first(joints, part, list.iter().copied());
        for joint in order {
            if let Some(places) = joints.attached_parts.get(&joint) {
                self.joint(places.map(|(part, _)| part));
                joints.detach(joint);
            }
        }
    }

    /// What a playing client spends on them, beyond the Parent write.
    fn price(&self) -> f64 {
        self.joints as f64 * crate::cost::JOINT_TAKEN_OUT + self.lists.len() as f64 * crate::cost::JOINT_LIST_LEFT
    }
}

/// The subtree of `id` left every world ([`reparented`]): nothing links up, and only the lists of active joints change. Down to
/// the WorldRoots and JointsServices it holds (below them every instance keeps its world and home, so no joint there changes),
/// in [`taken_up`]'s order, each active joint it carries leaves its parts' lists, and so do the active joints of each part it
/// carries, the last named first ([`Indexed::named`]; Studio 0.748: of a part's joints x1 and x3, named in that order, x3
/// leaves first, whatever the order they were parented in). A joint that is not active cannot become active by leaving, so the
/// joints that name a part and are not active are not looked at, and a subtree that leaves where no joint is active anywhere
/// costs no walk.
///
/// The work clock pays what a playing client spends on them (cost.rs JOINT_TAKEN_OUT for each joint taken out, JOINT_LIST_LEFT
/// for each part whose list it takes them out of), beyond the Parent write's own price (instance.rs).
fn left_every_world(state: &mut State, id: InstanceId) {
    // (a WorldRoot or JointsService that leaves takes its world or home along: everything below it keeps both)
    if !state.joints.scanned || state.joints.attached_parts.is_empty() || state.dom.is_home(id) {
        return;
    }
    let order = taken_up(state, id, |state, node| state.dom.is_home(node));
    let joints = &mut state.joints;
    let mut out = TakenOut::default();
    for node in order {
        if let Some(places) = joints.attached_parts.get(&node) {
            out.joint(places.map(|(part, _)| part));
            joints.detach(node);
        } else {
            out.part(joints, node);
        }
    }
    let seconds = out.price();
    if seconds > 0.0 {
        state.sched.charge(seconds);
        crate::boundary::add_cost(seconds);
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
    // (quiet while it writes the part and rewrites its joints: lowered when the guard drops, a panic's unwinding included)
    let (rewritten, inside, own) = {
        let mut quiet = Raised::new(state, |joints| &mut joints.quiet);
        let state = &mut *quiet;
        core::raw_set(state, reflection, hooks, part, name, frame::cframe_value(cframe));
        ensure_scanned(state, reflection, &setup);
        let links = edges(state, reflection, &setup, part);
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
                    // (a legacy motor's turn, the way it turns from the side nearer the root: legacy_angle, across; measured, a Position
                    // write of a turned Motor's Part1 keeps it turned)
                    let transform = match legacy_angle(state, reflection, &setup, joint) {
                        Some(angle) => {
                            let place = |at: InstanceId| tree.as_ref().and_then(|tree| tree.place.get(&at).copied());
                            let part1_above = matches!((place(part0), place(part1)), (Some((a0, i0)), Some((a1, i1))) if a0 == a1 && i1 < i0);
                            turn_z(if part1_above { -angle } else { angle }) * transform
                        }
                        None => transform,
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
        (rewritten, inside, own)
    };
    let work = crate::cost::joints_rewritten(rewritten, inside, own.map_or(1, |(_, parts)| parts));
    if work > 0.0 {
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
    // (moved within its assembly, the part reads its velocity at its new place, and the assembly its centre's)
    state.joints.regroup(part);
    super::gravity::assemblies_changed(state, reflection);
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
        let mut whole = true;
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
            let (placed, done) = settle(state, reflection, hooks, &setup, assembly, anchor, true);
            carried += placed;
            whole &= done;
        }
        found.settled.set(whole);
        refresh_centres(state, &setup, &found);
    }
    let work = crate::cost::assembly_moved(carried, between);
    if work > 0.0 {
        state.sched.charge_priced(work);
    }
}

/// Workspace:UnjoinFromOutsiders(objects): every active joint between a listed part and a part that is not listed leaves the
/// DataModel (Parent = nil, as a client does it: the joint is not destroyed, keeps its parts and can be parented again, which
/// links them up again), and the assemblies split as for any joint taken out. Measured in Studio 0.748, a playing client and its
/// server alike (tools/parity/probes/unjoin_from_outsiders.luau):
/// - The listed parts: each listed Instance that is a BasePart and every BasePart below a listed Instance (a Model's, a
///   Folder's, a part's child parts). `{game}` and `{workspace}` list every part there is, so nothing is outside. An entry
///   listed again, or below or above another entry, lists nothing new.
/// - A joint counts while it is active in the Workspace: a JointInstance but Rotate, RotateP, RotateV and VelocityMotor (so a
///   Weld, ManualWeld, Snap, Glue, ManualGlue, Motor or Motor6D), Enabled, naming two different parts whose WorldRoot is the
///   Workspace itself (a WorldModel's parts are not its), and itself in the Workspace outside any WorldModel, or in
///   JointsService. WeldConstraints and other constraints, disabled joints, joints elsewhere or out of the DataModel, joints to a
///   part out of the Workspace and joints between two listed parts stay.
/// - The order: each listed part's joints go in the order of the part's list of active joints ([`refresh_attached`]), the
///   parts in the order the list first reaches them (each listed Instance, then the parts below it in GetDescendants' order).
///   A client goes through its listed parts in an order of its own, which changed between runs (`{C, B}` took C's joints first
///   in one run and B's in the next; `{D, A}` went one way on the client and the other on its server).
///
/// The joints come from the index of the joints by part ([`Joints::by_part`]), so a call's work is its entries, the instances
/// below them (each looked at once a call: an entry already seen is skipped, and a walk below an entry does not go into an
/// entry walked before), and each listed part's candidate joints and list of active ones, whatever else the place holds (and
/// before any joint the index keeps was made, only the entries and the instances below them). The work clock pays what a
/// playing client spends on each (cost.rs UNJOIN_PART for each part the list reaches, UNJOIN_BELOW for each other instance
/// below an entry, UNJOIN_NAMED for each joint that names a part it reaches, UNJOIN_REMOVED for each joint it takes out, beyond
/// what that joint's Parent write costs and the link prices that grow with its assemblies, [`charge_link`]; the method charges the
/// list's entries).
pub fn unjoin_from_outsiders(engine: &Engine, workspace: InstanceId, listed: &[InstanceId]) -> Result<(), crate::lua::Flow> {
    let reflection = &engine.reflection;
    let Some(setup) = engine.state.borrow().joints.setup.clone() else { return Ok(()) };
    let Listed { parts, seen, entries } = listed_parts(engine, &setup, listed);
    let (removed, seconds) = {
        let mut state = engine.state.borrow_mut();
        // (before any joint the index keeps was made there is none to take out: the walk below the entries is all the work)
        if state.joints.created > 0 {
            ensure_scanned(&mut state, reflection, &setup);
        }
        let state = &*state;
        let (below, reached) = (seen.len() - entries, parts.len());
        let mut named = 0usize;
        // (a joint of `part` this call takes out: active in the Workspace, to a part that is not listed)
        let goes = |part: InstanceId, joint: InstanceId| {
            let Some(instance) = state.dom.get(joint) else { return false };
            if !setup.kind(instance.class).unjoined() || state.dom.boolean(joint, setup.enabled) == Some(false) {
                return false;
            }
            let [Some(part0), Some(part1)] = parts_of(state, &setup, joint) else { return false };
            let other = if part0 == part {
                part1
            } else if part1 == part {
                part0
            } else {
                return false;
            };
            if other == part || seen.contains(&other) || !is_part(state, reflection, &setup, other) {
                return false;
            }
            if world_of(state, reflection, &setup, other) != Some(workspace) {
                return false;
            }
            matches!(joint_home(state, joint), Some(home) if home == workspace || Some(state.dom.at(home).class) == setup.joints_service)
        };
        let mut removed = Vec::new();
        for part in parts {
            let Some(candidates) = state.joints.by_part.get(&part) else { continue };
            let attached = state.joints.attached.get(&part).map(Vec::as_slice).unwrap_or_default();
            named += candidates.live;
            if world_of(state, reflection, &setup, part) != Some(workspace) {
                continue;
            }
            // (its active joints in the part's order: every active joint is in its parts' lists, check_index)
            removed.extend(attached.iter().copied().filter(|&joint| goes(part, joint)));
        }
        let seconds = reached as f64 * crate::cost::UNJOIN_PART
            + below as f64 * crate::cost::UNJOIN_BELOW
            + named as f64 * crate::cost::UNJOIN_NAMED
            + removed.len() as f64 * crate::cost::UNJOIN_REMOVED;
        (removed, seconds)
    };
    super::query::charge_query(engine, seconds);
    // (each joint taken out pays UNJOIN_REMOVED for what a client spends beyond its Parent write and the lists, its link change's
    // flat prices among it: charge_link charges a removal that ends a single link only what grows with the assemblies meanwhile; one
    // that ends several pays every link price)
    let _unpriced = Unpriced::new(engine);
    for joint in removed {
        crate::lua::instance::set_parent(engine, joint, None)?;
    }
    Ok(())
}

/// What a list of UnjoinFromOutsiders or JoinToOutsiders lists ([`listed_parts`]): its parts in the order the list first reaches
/// them, every instance looked at (the entries and those below them), and how many entries there were (each once).
struct Listed {
    parts: Vec<InstanceId>,
    seen: FxHashSet<InstanceId>,
    entries: usize,
}

/// The parts `listed` lists: each listed BasePart and every BasePart below a listed instance, each entry and each instance below
/// one looked at once a call (an entry listed again, or below or above another, lists nothing new) and made whole as the walk
/// reaches it when it was imported lazily, before its children are read, as GetDescendants makes it, so the walk is a client's.
fn listed_parts(engine: &Engine, setup: &Setup, listed: &[InstanceId]) -> Listed {
    let reflection = &engine.reflection;
    let mut seen: FxHashSet<InstanceId> = FxHashSet::default();
    let (mut parts, mut entries) = (Vec::new(), 0usize);
    let mut stack = Vec::new();
    for &entry in listed {
        if engine.state.borrow().dom.get(entry).is_none() || !seen.insert(entry) {
            continue;
        }
        entries += 1;
        stack.push(entry);
        while let Some(node) = stack.pop() {
            crate::lua::instance::materialize(engine, node);
            let state = engine.state.borrow();
            if is_part(&state, reflection, setup, node) {
                parts.push(node);
            }
            stack.extend(state.dom.children_of(node).iter().rev().copied().filter(|&child| seen.insert(child)));
        }
    }
    Listed { parts, seen, entries }
}

/// Workspace:JoinToOutsiders(objects, mode) with a mode that joins (All or Surface, which do the same; world/mod.rs reads the
/// arguments): a Weld between each listed part and each part outside the list whose face its face touches (world/faces.rs, the
/// rules), as a playing client and its server make them (roblox-sim, Studio 0.747, both alike: tools/parity/probes/
/// join_to_outsiders.luau):
/// - The listed parts are [`listed_parts`]'. A listed part in the Workspace's world (not a WorldModel's, not out of the Workspace)
///   is welded to the parts of that world that are not listed, Terrain aside; two listed parts never to each other.
/// - Anchored, CanCollide, CanQuery, CanTouch, Massless, Locked, Transparency and the surfaces (SurfaceType, both modes) make no
///   difference: every weld is a plain `Weld`.
/// - A pair already joined is passed by: an active joint between them (a Weld, Snap, ManualWeld, Motor, Motor6D, Glue or
///   ManualGlue in the Workspace or JointsService, Enabled, either way round; a WeldConstraint that links them; a Rotate, RotateP or
///   RotateV), or a Constraint between their attachments (an Attachment's part is the nearest BasePart above it; Enabled, in the
///   Workspace); a weld this call or an earlier one made counts. Not a disabled one, one out of the Workspace, a VelocityMotor, a
///   NoCollisionConstraint, a path through a third part.
/// - The weld: a `Weld` named Weld, Part0 the listed part, Part1 the other, C0 and C1 world/faces.rs gives, parented to the listed
///   part. A listed part's welds are made in the order the other parts came into the Workspace (world/broadphase.rs entered: a
///   moved subtree's parts as a client takes it up, the moved part first; those there before any Parent write first, in the tree's
///   order); the listed parts are taken in the order the list first reaches them (a client goes through them in an order of its own,
///   which differs between a client and its server, so the probe prints each part's welds apart).
///
/// The parts it looks at are the listed parts' neighbours in the Workspace's spatial index (world/broadphase.rs, a box 0.02 larger
/// than the part's on each side), so a call's work is its entries, the instances below them and each listed part's neighbours,
/// however large the rest of the place is. The work clock pays what a playing client spends (cost.rs JOIN_CALL, JOIN_PART for each
/// part the list reaches, UNJOIN_BELOW for each other instance below an entry, JOIN_CANDIDATE for each neighbour, JOIN_OUTSIDER
/// for the first full look at each neighbour whose axes line up with a listed part's, and JOIN_WELDING once for a call that makes a
/// weld, each weld paying what its Parent write and link change cost; the method charges JOIN_CAST and the list's entries).
pub fn join_to_outsiders(engine: &Engine, workspace: InstanceId, listed: &[InstanceId]) -> Result<(), crate::lua::Flow> {
    use super::faces;
    let reflection = &engine.reflection;
    let Some(setup) = engine.state.borrow().joints.setup.clone() else { return Ok(()) };
    let Some(weld_class) = reflection.class_id("Weld") else { return Ok(()) };
    let Listed { parts, seen, entries } = listed_parts(engine, &setup, listed);
    let shapes = Shapes::new(reflection);
    let mut work = (seen.len() - entries) as f64 * crate::cost::UNJOIN_BELOW + parts.len() as f64 * crate::cost::JOIN_PART;
    if !parts.is_empty() {
        work += crate::cost::JOIN_CALL;
    }
    super::query::charge_query(engine, work);
    if parts.is_empty() {
        return Ok(());
    }
    // (the spatial index's waiting parts taken in, as a query takes them, unpriced: a client's call is priced whole)
    let Some(world) = super::broadphase::take_in(engine, workspace).0 else { return Ok(()) };
    if engine.state.borrow().joints.created > 0 {
        ensure_scanned(&mut engine.state.borrow_mut(), reflection, &setup);
    }
    // (the other kinds of joints and constraints between two parts that stop it, looked up once a call when first needed)
    let mut held: Option<FxHashSet<(InstanceId, InstanceId)>> = None;
    let mut looked: FxHashSet<InstanceId> = FxHashSet::default();
    let mut welding = false;
    for part in parts {
        let (body, near) = {
            let mut state = engine.state.borrow_mut();
            if state.dom.get(part).is_none() || world_of(&state, reflection, &setup, part) != Some(workspace) {
                continue;
            }
            let body = shapes.body(&state, reflection, &setup, part);
            let (low, high) = faces::world_box(&body, 0.02);
            let mut near = Vec::new();
            let mut met: FxHashSet<InstanceId> = FxHashSet::default();
            // (the grid's cells give the parts filed in them: those whose boxes meet the part's grown box are its neighbours)
            super::broadphase::visit_box(&mut state, world, glam::Vec3::from(low), glam::Vec3::from(high), super::grid::UNANCHORED, |other, entry| {
                if other != part && !seen.contains(&other) && met.insert(other) {
                    let (lo, hi) = faces::world_box(&faces::Body { rot: rows_of(&entry.solid.frame), pos: entry.solid.frame.translation.to_array(), half: entry.solid.half_extents().to_array(), shape: faces::Shape::Block }, 0.0);
                    if (0..3).all(|i| lo[i] <= high[i] && hi[i] >= low[i]) {
                        near.push(other);
                    }
                }
                true
            });
            // (in the order they came into the Workspace: those there before any Parent write first, in the tree's order)
            let mut old: Vec<InstanceId> = near.iter().copied().filter(|&other| super::broadphase::entered_at(&state, other) == 0).collect();
            if old.len() > 1 {
                super::broadphase::sort_by_order(&state.dom, workspace, &mut old, |&other| other);
            }
            let mut new: Vec<(u64, InstanceId)> = near.iter().map(|&other| (super::broadphase::entered_at(&state, other), other)).filter(|&(at, _)| at > 0).collect();
            new.sort_unstable();
            old.extend(new.into_iter().map(|(_, other)| other));
            (body, old)
        };
        super::query::charge_query(engine, near.len() as f64 * crate::cost::JOIN_CANDIDATE);
        for other in near {
            let (face, other_body) = {
                let mut state = engine.state.borrow_mut();
                if state.dom.get(other).is_none() || world_of(&state, reflection, &setup, other) != Some(workspace) {
                    continue;
                }
                if joined(&mut state, reflection, &setup, workspace, part, other, &mut held) {
                    continue;
                }
                let other_body = shapes.body(&state, reflection, &setup, other);
                if !faces::aligned(&body, &other_body) {
                    continue;
                }
                if looked.insert(other) {
                    state.sched.charge(crate::cost::JOIN_OUTSIDER);
                    crate::boundary::add_cost(crate::cost::JOIN_OUTSIDER);
                }
                let Some(face) = faces::touching(&body, &other_body) else { continue };
                (face, other_body)
            };
            let c0 = faces::weld_c0(&body, face);
            let c1 = faces::weld_c1(&body, &other_body, c0);
            if !welding {
                super::query::charge_query(engine, crate::cost::JOIN_WELDING);
                welding = true;
            }
            make_weld(engine, &setup, weld_class, part, other, c0, c1)?;
        }
    }
    Ok(())
}

/// A Weld JoinToOutsiders makes between `part` (Part0, its parent) and `other`, with `c0` and `c1` (position, rotation rows), as a
/// script's Instance.new and writes would make it: unparented while its properties are set, then parented, so its Parent write and
/// link change are priced as any.
fn make_weld(engine: &Engine, setup: &Setup, class: ClassId, part: InstanceId, other: InstanceId, c0: ([f32; 3], [[f32; 3]; 3]), c1: ([f32; 3], [[f32; 3]; 3])) -> Result<(), crate::lua::Flow> {
    use rbx_dom_weak::types::{self as dom, Variant};
    let value = |(position, rows): ([f32; 3], [[f32; 3]; 3])| {
        let vector = |v: [f32; 3]| dom::Vector3::new(v[0], v[1], v[2]);
        Value::Dom(Rc::new(Variant::CFrame(dom::CFrame::new(vector(position), dom::Matrix3::new(vector(rows[0]), vector(rows[1]), vector(rows[2]))))))
    };
    let weld = {
        let mut state = engine.state.borrow_mut();
        let weld = core::create(&mut state, &engine.reflection, class);
        state.dom.at_mut(weld).set_flag(crate::dom::flags::SCRIPT_CREATED, true);
        weld
    };
    crate::lua::instance::set_property(engine, weld, setup.part0, Value::Instance(part));
    crate::lua::instance::set_property(engine, weld, setup.part1, Value::Instance(other));
    crate::lua::instance::set_property(engine, weld, setup.c0, value(c0));
    crate::lua::instance::set_property(engine, weld, setup.c1, value(c1));
    crate::lua::instance::set_parent(engine, weld, Some(part))
}

/// Whether `part` and `other` are joined already for JoinToOutsiders: an active joint of the kinds the index keeps between them
/// (a rigid one or WeldConstraint that links them, one in their lists of active joints: Glue too), else, looked up once a call into
/// `held`, a Rotate, RotateP or RotateV between them, Enabled, in the Workspace or JointsService, or a Constraint (Enabled, in the
/// Workspace or JointsService) whose two attachments' parts are the two.
fn joined(state: &mut State, reflection: &Reflection, setup: &Setup, workspace: InstanceId, part: InstanceId, other: InstanceId, held: &mut Option<FxHashSet<(InstanceId, InstanceId)>>) -> bool {
    if edges(state, reflection, setup, part).iter().any(|&(_, linked)| linked == other) {
        return true;
    }
    if let Some(list) = state.joints.attached.get(&part)
        && list.iter().any(|&joint| parts_of(state, setup, joint).contains(&Some(other)))
    {
        return true;
    }
    let pairs = held.get_or_insert_with(|| held_pairs(state, reflection, setup, workspace));
    pairs.contains(&(part.min(other), part.max(other)))
}

/// The pairs of parts (the lesser id first) a Rotate, RotateP or RotateV or a Constraint holds together ([`joined`]).
fn held_pairs(state: &mut State, reflection: &Reflection, setup: &Setup, workspace: InstanceId) -> FxHashSet<(InstanceId, InstanceId)> {
    let mut pairs = FxHashSet::default();
    let (Some(constraint), Some(attachment0), Some(attachment1)) = (reflection.class_id("Constraint"), reflection.name_id("Attachment0"), reflection.name_id("Attachment1")) else {
        return pairs;
    };
    let active = |state: &State, id: InstanceId| {
        state.dom.boolean(id, setup.enabled) != Some(false)
            && matches!(joint_home(state, id), Some(home) if home == workspace || Some(state.dom.at(home).class) == setup.joints_service)
    };
    let part_above = |state: &State, mut id: InstanceId| loop {
        id = state.dom.parent_of(id)?;
        if is_part(state, reflection, setup, id) {
            break Some(id);
        }
    };
    let mut put = |a: Option<InstanceId>, b: Option<InstanceId>| {
        if let (Some(a), Some(b)) = (a, b)
            && a != b
        {
            pairs.insert((a.min(b), a.max(b)));
        }
    };
    for name in ["Rotate", "RotateP", "RotateV"] {
        let Some(class) = reflection.class_id(name) else { continue };
        for joint in state.dom.instances_of_class(class) {
            if active(state, joint) {
                let [a, b] = parts_of(state, setup, joint);
                put(a, b);
            }
        }
    }
    let classes: Vec<ClassId> = (0..reflection.classes.len() as ClassId).filter(|&class| reflection.class(class).is_a(constraint)).collect();
    for class in classes {
        for held in state.dom.instances_of_class(class) {
            if !active(state, held) {
                continue;
            }
            let ends = [attachment0, attachment1].map(|name| reference(state, held, name).and_then(|attachment| part_above(state, attachment)));
            put(ends[0], ends[1]);
        }
    }
    pairs
}

/// A frame's rotation as rows.
fn rows_of(frame: &Affine3A) -> [[f32; 3]; 3] {
    let m = frame.matrix3;
    [[m.x_axis.x, m.y_axis.x, m.z_axis.x], [m.x_axis.y, m.y_axis.y, m.z_axis.y], [m.x_axis.z, m.y_axis.z, m.z_axis.z]]
}

/// The classes and names a part's shape is read from ([`Shapes::body`]).
struct Shapes {
    terrain: Option<ClassId>,
    wedge: Option<ClassId>,
    corner_wedge: Option<ClassId>,
    part: Option<ClassId>,
    shape: Option<NameId>,
}

impl Shapes {
    fn new(reflection: &Reflection) -> Self {
        Shapes {
            terrain: reflection.class_id("Terrain"),
            wedge: reflection.class_id("WedgePart"),
            corner_wedge: reflection.class_id("CornerWedgePart"),
            part: reflection.class_id("Part"),
            shape: reflection.name_id("Shape"),
        }
    }

    /// A part's frame, half size and shape as JoinToOutsiders reads them (world/faces.rs Body).
    fn body(&self, state: &State, reflection: &Reflection, setup: &Setup, part: InstanceId) -> super::faces::Body {
        use super::faces::Shape;
        use rbx_dom_weak::types::Variant;
        let (pos, rot) = match state.dom.raw_get(part, setup.cframe) {
            Some(Value::Dom(variant)) => match &**variant {
                Variant::CFrame(cframe) => {
                    let (p, o) = (cframe.position, cframe.orientation);
                    ([p.x, p.y, p.z], [[o.x.x, o.x.y, o.x.z], [o.y.x, o.y.y, o.y.z], [o.z.x, o.z.y, o.z.z]])
                }
                _ => ([0.0; 3], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            },
            _ => ([0.0; 3], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        };
        let half = match state.dom.raw_get(part, setup.size) {
            Some(Value::Vector3(size)) => [size.x * 0.5, size.y * 0.5, size.z * 0.5],
            _ => [0.5; 3],
        };
        let class = state.dom.at(part).class;
        let is = |of: Option<ClassId>| of.is_some_and(|of| reflection.class(class).is_a(of));
        let shape = if is(self.terrain) {
            Shape::Faceless
        } else if is(self.wedge) {
            Shape::Wedge
        } else if is(self.corner_wedge) {
            Shape::CornerWedge
        } else if is(self.part) {
            match self.shape.and_then(|name| state.dom.raw_get(part, name)).and_then(Value::as_enum).map(|item| &*reflection.enum_item_info(item).name) {
                Some("Ball") => Shape::Ball,
                Some("Cylinder") => Shape::Cylinder,
                Some("Wedge") => Shape::Wedge,
                Some("CornerWedge") => Shape::CornerWedge,
                _ => Shape::Block,
            }
        } else {
            Shape::Block
        };
        super::faces::Body { rot, pos, half, shape }
    }
}

/// Holds [`Joints::unpriced`] up while it lives, so it comes down however UnjoinFromOutsiders' removals end: a Parent write's error,
/// or a panic raw.rs's guard turns into one a script can catch. It holds no borrow while the removals run, and takes the state's
/// when it drops (none is held then; when one still is, the count stays), as [`BulkMoving`] does.
struct Unpriced<'a>(&'a Engine);

impl<'a> Unpriced<'a> {
    fn new(engine: &'a Engine) -> Self {
        engine.state.borrow_mut().joints.unpriced += 1;
        Unpriced(engine)
    }
}

impl Drop for Unpriced<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            state.joints.unpriced = state.joints.unpriced.saturating_sub(1);
        } else {
            // (a borrow still held would leave the count raised and every later link change charged less: never so today)
            debug_assert!(std::thread::panicking(), "UnjoinFromOutsiders' unpriced count stays raised: the state is still borrowed");
        }
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
    settle_from_roots(&mut state, &engine.reflection, &hooks, &setup, &seeds);
}

/// The simulation step's legacy motors: each active Motor (and Motor6D) whose CurrentAngle is not its DesiredAngle turns toward it in
/// four quarter steps of its MaxVelocity's size (MaxVelocity a 60 Hz frame), a step taken while the angle left is not below it (a
/// NaN DesiredAngle turns it down), and its assembly settles from its root. Measured in a playing client and its server (Studio
/// 0.747, tools/parity/probes/legacy_motors.luau): a client steps in quarters of MaxVelocity by its frames' real time, so its angle
/// after a frame varies between runs; its end and its rules do not. The angle changes without Changed events, as physics moves
/// parts. A motor outside every world does not turn.
pub fn step_motors(engine: &Engine) {
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    if state.joints.turning.is_empty() {
        return;
    }
    let Some(setup) = setup_of(&state) else { return };
    let (Some(current_name), Some(desired_name), Some(velocity_name)) = (setup.current_angle, setup.desired_angle, setup.max_velocity) else { return };
    let mut motors: Vec<InstanceId> = state.joints.turning.iter().copied().collect();
    motors.sort_by_key(|&motor| state.dom.get(motor).map(|instance| instance.serial));
    ensure_scanned(&mut state, &engine.reflection, &setup);
    let mut seeds = Vec::new();
    for motor in motors {
        if !alive(&state, motor) {
            state.joints.turning.remove(&motor);
            continue;
        }
        if active_parts(&state, &engine.reflection, &setup, motor, None).is_none() {
            continue;
        }
        // (in double within a step, stored as a float: 0.7 turned by eight steps of 0.0125 reads 0.800000012, a client's)
        let read = |name: NameId| f64::from(state.dom.number(motor, name).unwrap_or(0.0) as f32);
        let (mut angle, desired, step) = (read(current_name), read(desired_name), read(velocity_name).abs() * 0.25);
        let mut moved = false;
        for _ in 0..4 {
            let left = desired - angle;
            if left.abs() < step || step == 0.0 {
                break;
            }
            angle += if left > 0.0 { step } else { -step };
            moved = true;
        }
        if !moved {
            state.joints.turning.remove(&motor);
            continue;
        }
        core::raw_set_unsignalled(&mut state, &engine.reflection, &hooks, motor, current_name, Value::Number(f64::from(angle as f32)));
        seeds.extend(parts_of(&state, &setup, motor).into_iter().flatten());
    }
    if !seeds.is_empty() {
        settle_from_roots(&mut state, &engine.reflection, &hooks, &setup, &seeds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic in work that raises a joint count (lua/raw.rs guard turns it into an error its script catches) leaves the count as it
    /// was: a hook that panics while a Position write rewrites the hub's weld (quiet), while a C0 write's settle places the welded
    /// part (placing) and while BulkMoveTo moves the hub (bulk), each write's error caught by its script, and every count is 0 after
    /// each; the hub's CFrame write then still carries the welded part. Each count was raised and lowered by hand around the work, so
    /// the panic left it raised (quiet: every later CFrame and joint write ignored, Parent writes linking nothing).
    #[test]
    fn a_panic_in_a_counted_write_leaves_no_count_raised() {
        let (lua, engine) = crate::world::tests::start_play();
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let armed: Rc<Cell<Option<NameId>>> = Rc::new(Cell::new(None));
        {
            let armed = armed.clone();
            engine.hooks.borrow_mut().property_changed.push(Box::new(move |_, _, name| {
                if armed.get() == Some(name) {
                    armed.set(None);
                    panic!("a hook's panic in a counted write");
                }
            }));
        }
        crate::world::tests::run(
            &lua,
            &engine,
            r#"
            workspace.Gravity = 0
            local Hub = Instance.new("Part")
            Hub.Anchored, Hub.CanCollide, Hub.CFrame, Hub.Parent = true, false, CFrame.new(0, 300, 0), workspace
            local Leaf = Instance.new("Part")
            Leaf.Anchored, Leaf.CanCollide, Leaf.CFrame, Leaf.Parent = false, false, CFrame.new(0, 304, 0), workspace
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0, W.Parent = Hub, Leaf, CFrame.new(0, 4, 0), Leaf
            shared.Hub, shared.Leaf, shared.W = Hub, Leaf, W
            "#,
        );
        let name = |text: &str| engine.reflection.name_id(text).expect("a property");
        let counts = || {
            let state = engine.state.borrow();
            (state.joints.quiet, state.joints.placing, state.joints.bulk)
        };
        for (what, armed_on, write) in [
            ("a Position write rewriting the weld", "C0", "shared.Hub.Position = Vector3.new(1, 300, 0)"),
            ("a C0 write's settle placing the part", "CFrame", "shared.W.C0 = CFrame.new(0, 5, 0)"),
            ("BulkMoveTo", "CFrame", "workspace:BulkMoveTo({ shared.Hub }, { CFrame.new(2, 300, 0) })"),
        ] {
            armed.set(Some(name(armed_on)));
            crate::world::tests::run(&lua, &engine, &format!("assert(not pcall(function() {write} end), 'the panic is an error the script catches')"));
            assert_eq!(armed.get(), None, "{what}: the hook panicked");
            assert_eq!(counts(), (0, 0, 0), "{what}: the counts (quiet, placing, bulk) after the panic");
        }
        crate::world::tests::run(
            &lua,
            &engine,
            "shared.Hub.CFrame = CFrame.new(10, 300, 0) local Gap = (shared.Leaf.Position - shared.Hub.Position).Magnitude assert(math.abs(Gap - 5) < 1e-3, `the welded part is {Gap} studs from the hub`)",
        );
    }

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
                indexed.insert(joint, Indexed { parts: [Some(part), None], places: [place, 0], taken: 0, named: [0; 2], listed: 0 });
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
