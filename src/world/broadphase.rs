//! The spatial queries' broadphase: each world's parts filed in a grid of its own, as a client files them (world/grid.rs, the
//! grid; docs/engine.md "The spatial grid"). A world is the Workspace's (its BaseParts but Terrain, not those of a WorldModel
//! in it) or a WorldModel's in the DataModel (the BaseParts whose nearest WorldRoot it is): a client keeps one grid each, and
//! a WorldRoot's queries walk its own (measured: a WorldModel's parts are never found by the Workspace's queries, and a
//! WorldModel's grid follows the same rules). The indexes live in the engine's state ([`Spatial`], `State::spatial`) so that
//! every write can file a part at once.
//!
//! - **Units**: the grid files units ([`Unit`]), each holding its parts in the order a walk lists them; a walk that meets a
//!   unit gives each of its parts.
//! - **Coming in**: a part that enters a world waits (its unit at the end of its world's wait, [`wait_in`] from the hierarchy hook
//!   in camera_moves.rs, a moved subtree's parts as a client takes the subtree up: the moved part first, then the rest each after
//!   its own descendants, world/joints.rs taken_up) until a query of that world or the physics step takes the waiting units in
//!   ([`take_in`], [`index_parts`]); each is then filed at the end of its cells' lists. A unit leaving the wait gives its place to
//!   the last, as a cell's list does (measured, Studio 0.747, a client and a server: five parts waiting, the second taken out
//!   before a query, come in as 1, 5, 3, 4; tools/parity/probes/grid_welds.luau).
//! - **Going out** (destroyed, parented out of the world, into another): it leaves its cells or the wait at once ([`wait_out`],
//!   from the hierarchy hook, a moved subtree's parts in the order they come in), the list's last taking its place.
//! - **Every write files at once** ([`stored`], from instance.rs store): a write of CFrame, Size, Anchored, CanCollide or
//!   CanTouch of a filed part works out its filing again (float32 box arithmetic) and, when it changed, files it: at the end of
//!   every cell when its level, its list or CanCollide-and-CanTouch-both-off changed, else out of the cells it left and at
//!   the end of those it entered, keeping its place in the others. So a part moved out of its cell and back before a query is
//!   last there, as on a client (measured: tools/parity/probes/grid_order.luau).
//! - What a query reads of a part beyond its filing (its solid, CanQuery, CanCollide: [`Entry`]) is read again from the dom's
//!   change logs at the next query of its world (dom.rs `Watch` for the Workspace, `WorldLog` for a WorldModel), which also catch
//!   what came or went other than through a Parent write (a project mount, a replaced child, a class change): each logged
//!   instance is looked at once however often it changed, so a query after a change does work for what changed. A world's
//!   index is made when a part first waits for it, or from a walk of its world at its first query or physics step (parts that do
//!   not wait filed in the walk's order, each after its descendants); it is walked again only after its log overflowed (twice the
//!   parts it holds, at least 4,096 entries), which never moves a part already filed.
//! - A unit whose box is not finite (a NaN or infinite position or size) is in no cell: every walk of its world gives it last (a
//!   list of its own, which it leaves in a step).
//!
//! A collision-group change lists the Workspace's parts from here ([`part_ids`], services/physics.rs), and puts the changed
//! ones in the Workspace's order ([`sort_by_order`]).

use glam::{Affine3A, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    frame,
    grid::{self, Filing, Grid, Passed, Walked},
    query::{Shape, Solid},
};
use crate::{
    ClassId, Engine, InstanceId, K, NameId, State,
    dom::{Dom, Watch, flags},
    value::Value,
};

/// A part in an index: its solid and what the queries' filters read of it, as the dom last held them
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub solid: Solid,
    /// A query sees it: CanQuery is not false, or CanCollide is not (a playing client queries a part with CanQuery off while
    /// CanCollide is on, whichever was written first: measured, Studio 0.747, client and server, tools/parity/probes/query_canquery.luau)
    pub can_query: bool,
    /// CanCollide is not false
    pub can_collide: bool,
}

impl Entry {
    /// What a part that came into a wait holds until its world's index reads it (the next query reads every part that came in)
    fn unread(part: InstanceId) -> Entry {
        Entry { solid: Solid { part, frame: Affine3A::IDENTITY, shape: Shape::Box(Vec3::ZERO) }, can_query: false, can_collide: false }
    }
}

/// Where a unit is: waiting to be taken in, filed in the grid, or in the loose list (its box not finite)
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    Waits,
    Filed(Filing),
    Loose,
}

/// What a world's grid files: a unit, one rigid assembly (a part linked to no other alone), its parts in the order a walk lists
/// them (its first, then the rest: two parts its root first, three or more in the order they were made: measured, Studio 0.747, a
/// playing client and a server alike, tools/parity/probes/grid_welds.luau), and where it is
#[derive(Debug)]
struct Unit {
    first: u32,
    rest: Vec<u32>,
    place: Place,
    /// Its place in its world's wait or loose list, while it is in one
    at: u32,
    /// Its root's entry ([`refresh_root`]), stale after a write that may have changed it
    root: u32,
    root_stale: bool,
    /// Its parts are to be put in their walk order again ([`ready_unit`])
    unsorted: bool,
    /// In the settle's list of units to file again ([`Spatial::mark_dirty`])
    dirty: bool,
    /// How many times its slot was freed: a unit freed since a settle marked it is not the one marked
    generation: u32,
    /// How many of its parts are anchored (one at most once its assembly settled: each anchored part roots an assembly of its own)
    anchored: u32,
}

impl Unit {
    /// Its parts' entries, in the order a walk lists them
    fn parts(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::once(self.first).chain(self.rest.iter().copied())
    }

    fn len(&self) -> usize {
        1 + self.rest.len()
    }
}

/// No unit (an entry's unit while it has none, a free unit's first part)
const NONE: u32 = u32::MAX;

#[cfg(test)]
thread_local! {
    /// How many times an index has walked the Workspace on this thread (tests)
    pub(crate) static WORKSPACE_WALKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Every walk is the notes' walk of every cell in range where that is small enough to finish (tests: the grid's answers
    /// against it)
    pub(crate) static SCAN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How many parts the queries have taken from an index on this thread (tests)
    pub(crate) static TAKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// How many times [`sort_by_order`] has ordered by the trie with one node for each list ([`Trie::keyed`]) on this thread (tests)
    pub(crate) static KEYED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// How many instances the walks of what moved have looked at, taking the Workspace's log, on this thread (tests)
    pub(crate) static SYNC_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// How many times a WorldModel's index has walked its world on this thread (tests)
    pub(crate) static MODEL_WALKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// How many instances the WorldModel indexes' walks of what moved have looked at on this thread (tests)
    pub(crate) static MODEL_SYNC_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Counts what a query took ([`TAKEN`], tests)
fn taken(_count: usize) {
    #[cfg(test)]
    TAKEN.with(|taken| taken.set(taken.get() + _count as u64));
}

#[cfg(test)]
fn scan() -> bool {
    SCAN.with(|scan| scan.get())
}

/// Whether a frame's axes are further from square than a part's stored rotation can be (a script's CFrame write is
/// orthonormalized past a client's tolerance: lua/instance.rs cframe_as_stored), or not finite (world/query.rs: such a query
/// box's overlap test is no box test)
pub fn skewed(frame: &Affine3A) -> bool {
    let (x, y, z) = (Vec3::from(frame.matrix3.x_axis), Vec3::from(frame.matrix3.y_axis), Vec3::from(frame.matrix3.z_axis));
    let worst = x.dot(y).abs().max(y.dot(z).abs()).max(z.dot(x).abs());
    !(x.is_finite() && y.is_finite() && z.is_finite()) || worst.is_nan() || worst > 1e-4
}

/// The classes and properties the indexes read, resolved once
#[derive(Clone, Copy)]
struct Names {
    base_part: ClassId,
    terrain: ClassId,
    world_root: ClassId,
    cframe: NameId,
    can_query: Option<NameId>,
    can_collide: Option<NameId>,
    can_touch: Option<NameId>,
    anchored: Option<NameId>,
}

/// One world's index: its grid of units, its wait, and its parts' entries
pub struct WorldIndex {
    /// Its WorldRoot (the Workspace, or a WorldModel)
    root: InstanceId,
    grid: Grid,
    entries: Vec<Entry>,
    /// Each entry's part (stale for a free entry), when it was made (instance.rs serial: a unit of three or more lists its parts by
    /// it) and its unit
    parts: Vec<InstanceId>,
    serials: Vec<u32>,
    unit_of: Vec<u32>,
    free: Vec<u32>,
    /// The entries in use, in no particular order, and each one's place in that list ([`part_ids`] reads the parts held without
    /// the free entries)
    live: Vec<u32>,
    live_at: Vec<u32>,
    units: Vec<Unit>,
    free_units: Vec<u32>,
    /// The units waiting to be taken in, in the order they came
    wait: Vec<u32>,
    /// How many parts the waiting units hold
    waiting: usize,
    /// The units whose box is not finite: a walk gives them last, in this order
    loose: Vec<u32>,
    /// How many instances the last walk of its world visited (its log's limit)
    walked: usize,
    /// A walk of its world read it (an index a wait made reads its world whole at its first query, whatever log its WorldRoot kept)
    read: bool,
}

impl WorldIndex {
    fn new(root: InstanceId) -> Self {
        WorldIndex {
            root,
            grid: Grid::default(),
            entries: Vec::new(),
            parts: Vec::new(),
            serials: Vec::new(),
            unit_of: Vec::new(),
            free: Vec::new(),
            live: Vec::new(),
            live_at: Vec::new(),
            units: Vec::new(),
            free_units: Vec::new(),
            wait: Vec::new(),
            waiting: 0,
            loose: Vec::new(),
            walked: 0,
            read: false,
        }
    }

    /// How many parts it holds, waiting ones too
    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// The log's limit: twice the parts held and the instances the last walk of its world visited, so the log overflows only
    /// after as many distinct instances changed (each is logged once until taken) as the walk it forces visits
    fn limit(&self) -> usize {
        (self.len() * 2).max(self.walked).max(4096)
    }

    fn give(&self, entry: u32) -> (InstanceId, Entry) {
        (self.parts[entry as usize], self.entries[entry as usize])
    }

    /// The place of `entry`'s unit
    fn place_of(&self, entry: u32) -> Place {
        self.units[self.unit_of[entry as usize] as usize].place
    }

    /// A new unit of the parts `entries`, nowhere yet (taken as waiting, put in no list)
    fn new_unit(&mut self, entries: &[u32]) -> u32 {
        let several = entries.len() > 1;
        let mut unit = Unit { first: entries[0], rest: entries[1..].to_vec(), place: Place::Waits, at: NONE, root: entries[0], root_stale: several, unsorted: several, dirty: false, generation: 0, anchored: 0 };
        let id = match self.free_units.pop() {
            Some(id) => {
                unit.generation = self.units[id as usize].generation;
                self.units[id as usize] = unit;
                id
            }
            None => {
                self.units.push(unit);
                (self.units.len() - 1) as u32
            }
        };
        for &entry in entries {
            self.unit_of[entry as usize] = id;
        }
        id
    }

    /// Frees a unit that is in no list and no cell.
    fn free_unit(&mut self, unit: u32) {
        let held = &mut self.units[unit as usize];
        held.first = NONE;
        held.rest = Vec::new();
        held.at = NONE;
        held.dirty = false;
        held.generation = held.generation.wrapping_add(1);
        self.free_units.push(unit);
    }

    /// Takes `unit` out of its place (its cells, the wait or the loose list, the list's last taking its place), in none until it is
    /// given one ([`WorldIndex::set_place`] with `fresh`) or freed.
    fn take_unit(&mut self, unit: u32, frame: u64) {
        match self.units[unit as usize].place {
            Place::Filed(filing) => self.grid.unfile(unit, &filing, frame),
            Place::Loose => self.take_from(unit, false),
            Place::Waits => self.take_from(unit, true),
        }
    }

    /// A filed or loose `unit` out of its place and at the end of the wait; one that waits stays where it is.
    fn requeue(&mut self, unit: u32, frame: u64) {
        if self.units[unit as usize].place == Place::Waits {
            return;
        }
        self.take_unit(unit, frame);
        self.units[unit as usize].dirty = false;
        self.set_place(unit, Place::Waits, true, frame);
    }

    /// Moves `unit` from its place to `place`: out of its cells (the lists' last taking its place), the wait or the loose list (the
    /// list's last taking its place there too), into the new cells at their ends or at the end of the wait or the loose list; a
    /// filing of the same level, list and flags moves only the cells that changed. `fresh`: the unit is in no list or cell yet.
    fn set_place(&mut self, unit: u32, place: Place, fresh: bool, frame: u64) {
        let old = self.units[unit as usize].place;
        if !fresh && old == place {
            return;
        }
        match (old, place) {
            (Place::Filed(a), Place::Filed(b)) if !fresh && a.level == b.level && a.list == b.list && a.off == b.off => self.grid.move_cells(unit, &a, &b, frame),
            _ => {
                if !fresh {
                    match old {
                        Place::Filed(filing) => self.grid.unfile(unit, &filing, frame),
                        Place::Loose => self.take_from(unit, false),
                        Place::Waits => self.take_from(unit, true),
                    }
                }
                match place {
                    Place::Filed(filing) => self.grid.file(unit, &filing, frame),
                    Place::Loose => self.push_to(unit, false),
                    Place::Waits => self.push_to(unit, true),
                }
            }
        }
        self.units[unit as usize].place = place;
    }

    /// Takes `unit` out of the wait (or the loose list) in a step: the last takes its place.
    fn take_from(&mut self, unit: u32, wait: bool) {
        let at = self.units[unit as usize].at as usize;
        let count = self.units[unit as usize].len();
        let list = if wait { &mut self.wait } else { &mut self.loose };
        if list.get(at) != Some(&unit) {
            return;
        }
        list.swap_remove(at);
        if let Some(&moved) = list.get(at) {
            self.units[moved as usize].at = at as u32;
        }
        self.units[unit as usize].at = NONE;
        if wait {
            self.waiting -= count;
        }
    }

    /// Puts `unit` at the end of the wait (or the loose list).
    fn push_to(&mut self, unit: u32, wait: bool) {
        let list = if wait { &mut self.wait } else { &mut self.loose };
        self.units[unit as usize].at = list.len() as u32;
        list.push(unit);
        if wait {
            self.waiting += self.units[unit as usize].len();
        }
    }
}

/// The spatial indexes of an engine (`State::spatial`): the Workspace's and each WorldModel's, and which holds each part
#[derive(Default)]
pub struct Spatial {
    names: Option<Names>,
    /// What a write of each property does to its part's unit, by name id ([`stored`]: WATCH_*; 0 nothing)
    watched: Vec<u8>,
    worlds: Vec<Option<Box<WorldIndex>>>,
    free_worlds: Vec<u32>,
    /// Each WorldRoot's index
    by_root: FxHashMap<InstanceId, u32>,
    /// Each dom slot's index and entry, plus one (0: none)
    slots: Vec<(u32, u32)>,
    /// Scratch kept between syncs
    seen: FxHashSet<InstanceId>,
    walk: Vec<InstanceId>,
    order: Vec<InstanceId>,
    /// The WorldModels inside the Workspace its index met, whose indexes it makes (their parts are in the Workspace, and its
    /// collision groups' ids are theirs too: [`placed_parts`])
    nested: Vec<InstanceId>,
    /// What the writes since the last settle left for it ([`settle`])
    pending: Pending,
    /// The settle's pieces whose root had no unit yet (a part coming in later in the write), each with that root: joined at its end
    orphans: Vec<(InstanceId, InstanceId)>,
    /// While the settle splits units, the joins the splits ask for wait (a join could take in a unit not split yet): the parts to put in
    /// units by their roots, and the parts whose units join the units their links reach
    deferring: bool,
    later_regroups: Vec<(u32, Vec<InstanceId>)>,
    later_joins: Vec<(u32, InstanceId)>,
    /// When each part came into the Workspace's world through a Parent write, by a number that grows ([`entered`]); a part that
    /// leaves it is forgotten, and the parts there before any write (a mounted place's) have none
    entered: FxHashMap<InstanceId, u64>,
    entries: u64,
}

/// Resolves the names the indexes read (world/mod.rs install).
pub fn install(engine: &Engine) {
    let reflection = &engine.reflection;
    let (Some(base_part), Some(terrain), Some(world_root), Some(cframe)) = (reflection.class_id("BasePart"), reflection.class_id("Terrain"), reflection.class_id("WorldRoot"), reflection.name_id("CFrame")) else { return };
    let names = Names {
        base_part,
        terrain,
        world_root,
        cframe,
        can_query: reflection.name_id("CanQuery"),
        can_collide: reflection.name_id("CanCollide"),
        can_touch: reflection.name_id("CanTouch"),
        anchored: reflection.name_id("Anchored"),
    };
    let mut state = engine.state.borrow_mut();
    let spatial = &mut state.spatial;
    let watch = [
        (Some(cframe), WATCH_GEOMETRY),
        (names.can_collide, WATCH_GEOMETRY),
        (names.can_touch, WATCH_GEOMETRY),
        (Some(K::Size as NameId), WATCH_SIZE),
        (reflection.name_id("Massless"), WATCH_RANK),
        (reflection.name_id("RootPriority"), WATCH_RANK),
        (Some(K::Name as NameId), WATCH_RANK),
        (names.anchored, WATCH_ANCHORED),
    ];
    for (name, kind) in watch {
        let Some(name) = name else { continue };
        if spatial.watched.len() <= name as usize {
            spatial.watched.resize(name as usize + 1, 0);
        }
        spatial.watched[name as usize] = kind;
    }
    spatial.names = Some(names);
}

/// What a property's write does ([`Spatial::watched`]): files the unit again (CFrame, CanCollide, CanTouch), that and, in a unit of
/// several, its root (Size), its root (Massless, RootPriority, Name), its list and its assemblies (Anchored)
const WATCH_GEOMETRY: u8 = 1;
const WATCH_SIZE: u8 = 2;
const WATCH_RANK: u8 = 3;
const WATCH_ANCHORED: u8 = 4;

fn is_part(engine: &Engine, names: Names, class: ClassId) -> bool {
    let info = engine.reflection.class(class);
    info.is_a(names.base_part) && !info.is_a(names.terrain)
}

/// The place a unit's parts give it now: the union of their filed boxes (each part's box grown by the margin in its own axes, then the
/// world box of that, float32 at its position: grid.rs filed_box), on that union's level (measured: two 1-stud parts welded 147
/// studs apart are on level 0, 149 apart on level 1), in the anchored list when one of them is anchored (an unanchored part welded to
/// an anchored one is in the anchored list), off when every part has CanCollide and CanTouch both off (a pair one of whose parts has
/// both off keeps its place: measured); the loose list when a part's box is not finite.
fn unit_place(state: &State, names: &Names, world: u32, unit: u32) -> Place {
    let index = state.spatial.index(world);
    let dom = &state.dom;
    let flag = |part: InstanceId, name: Option<NameId>| name.and_then(|n| dom.boolean(part, n));
    let (mut bounds, mut anchored, mut off) = (None::<(Vec3, Vec3)>, false, true);
    for entry in index.units[unit as usize].parts() {
        let part = index.parts[entry as usize];
        let transform = frame::affine_of(dom.raw_get(part, names.cframe));
        let size = match dom.raw_get(part, K::Size as NameId) {
            Some(Value::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
            _ => Vec3::ONE,
        };
        let Some((lo, hi)) = grid::filed_box(&transform, size) else { return Place::Loose };
        bounds = Some(match bounds {
            Some((a, b)) => (a.min(lo), b.max(hi)),
            None => (lo, hi),
        });
        anchored |= flag(part, names.anchored) == Some(true);
        off &= flag(part, names.can_collide) == Some(false) && flag(part, names.can_touch) == Some(false);
    }
    let Some((lo, hi)) = bounds else { return Place::Loose };
    Place::Filed(Filing::of_box(lo, hi, if anchored { grid::ANCHORED } else { grid::UNANCHORED }, off))
}

/// How many of `unit`'s parts are anchored, counted from the dom (a unit just made)
fn count_anchored(state: &mut State, names: &Names, world: u32, unit: u32) {
    let index = state.spatial.index(world);
    let count = index.units[unit as usize].parts().filter(|&entry| names.anchored.and_then(|n| state.dom.boolean(index.parts[entry as usize], n)) == Some(true)).count() as u32;
    state.spatial.index_mut(world).units[unit as usize].anchored = count;
}

/// `unit` readied for a walk: its root worked out again if stale, its parts in the order a walk lists them (two: the root first;
/// three or more: the order they were made)
fn ready_unit(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32) {
    let held = &state.spatial.index(world).units[unit as usize];
    if held.root_stale {
        refresh_root(state, reflection, world, unit);
    }
    let index = state.spatial.index_mut(world);
    let held = &index.units[unit as usize];
    if !held.unsorted {
        return;
    }
    let mut parts: Vec<u32> = held.parts().collect();
    if parts.len() == 2 {
        if parts[1] == held.root {
            parts.swap(0, 1);
        }
    } else {
        let serials = &index.serials;
        parts.sort_by_key(|&entry| serials[entry as usize]);
    }
    let held = &mut index.units[unit as usize];
    held.first = parts[0];
    held.rest = parts[1..].to_vec();
    held.unsorted = false;
}

/// A part's entry, read from the dom now
fn entry_of(state: &State, engine: &Engine, names: &Names, part: InstanceId) -> Entry {
    let solid = super::query::solid(state, engine, part);
    let not_false = |name: Option<NameId>| name.and_then(|n| state.dom.boolean(part, n)) != Some(false);
    let can_collide = not_false(names.can_collide);
    Entry { solid, can_query: can_collide || not_false(names.can_query), can_collide }
}

impl Spatial {
    fn index(&self, world: u32) -> &WorldIndex {
        self.worlds[world as usize].as_deref().expect("a live index")
    }

    fn index_mut(&mut self, world: u32) -> &mut WorldIndex {
        self.worlds[world as usize].as_deref_mut().expect("a live index")
    }

    /// The index of the world of the WorldRoot `root`, if made
    pub fn world(&self, root: InstanceId) -> Option<u32> {
        self.by_root.get(&root).copied()
    }

    /// `part`'s index and entry, if an index holds it
    fn holder(&self, part: InstanceId) -> Option<(u32, u32)> {
        let &(world, entry) = self.slots.get(part.index as usize)?;
        if world == 0 {
            return None;
        }
        let index = self.worlds.get(world as usize - 1)?.as_deref()?;
        (index.parts.get(entry as usize - 1) == Some(&part)).then_some((world - 1, entry - 1))
    }

    fn make(&mut self, root: InstanceId) -> u32 {
        let index = Box::new(WorldIndex::new(root));
        let world = match self.free_worlds.pop() {
            Some(world) => {
                self.worlds[world as usize] = Some(index);
                world
            }
            None => {
                self.worlds.push(Some(index));
                (self.worlds.len() - 1) as u32
            }
        };
        self.by_root.insert(root, world);
        world
    }

    /// Drops the index of the WorldRoot `root` (a WorldModel freed or out of the DataModel), its parts' entries and its wait with it.
    fn drop_world(&mut self, dom: &mut Dom, root: InstanceId) {
        let Some(world) = self.by_root.remove(&root) else { return };
        let index = self.worlds[world as usize].take().expect("a live index");
        for &entry in &index.live {
            let part = index.parts[entry as usize];
            if self.slots.get(part.index as usize) == Some(&(world + 1, entry + 1)) {
                self.slots[part.index as usize] = (0, 0);
            }
            if let Some(instance) = dom.get_mut(part) {
                instance.flags &= !(flags::INDEXED | flags::LOGGED);
            }
        }
        self.free_worlds.push(world);
    }

    /// A new entry in `world` for `part` (taken out of any other index first; `serial` when it was made), in no unit yet
    fn add(&mut self, world: u32, part: InstanceId, entry: Entry, serial: u32, frame: u64) -> u32 {
        if let Some((other, old)) = self.holder(part) {
            self.remove(other, old, frame);
        } else if let Some(&(other, old)) = self.slots.get(part.index as usize).filter(|&&(other, _)| other != 0) {
            // (a slot's earlier instance, freed and not yet taken out)
            if let Some(index) = self.worlds.get(other as usize - 1).and_then(|index| index.as_deref())
                && index.parts.get(old as usize - 1).is_some()
                && index.live_at.get(old as usize - 1).is_some_and(|&at| index.live.get(at as usize) == Some(&(old - 1)))
            {
                self.remove(other - 1, old - 1, frame);
            }
        }
        let index = self.index_mut(world);
        let at = match index.free.pop() {
            Some(at) => {
                index.entries[at as usize] = entry;
                index.parts[at as usize] = part;
                index.serials[at as usize] = serial;
                index.unit_of[at as usize] = NONE;
                at
            }
            None => {
                index.entries.push(entry);
                index.parts.push(part);
                index.serials.push(serial);
                index.unit_of.push(NONE);
                (index.entries.len() - 1) as u32
            }
        };
        if index.live_at.len() <= at as usize {
            index.live_at.resize(at as usize + 1, 0);
        }
        index.live_at[at as usize] = index.live.len() as u32;
        index.live.push(at);
        let slot = part.index as usize;
        if self.slots.len() <= slot {
            self.slots.resize(slot + 1, (0, 0));
        }
        self.slots[slot] = (world + 1, at + 1);
        at
    }

    /// A new entry in `world` for `part` in a unit of its own at `place`
    fn add_alone(&mut self, world: u32, part: InstanceId, entry: Entry, serial: u32, place: Place, frame: u64) -> u32 {
        let at = self.add(world, part, entry, serial, frame);
        let index = self.index_mut(world);
        let unit = index.new_unit(&[at]);
        index.set_place(unit, place, true, frame);
        at
    }

    /// Takes `entry` out of `world`'s index: out of its unit (a unit left with no part out of its cells, the wait or the loose
    /// list, and freed), its entry freed.
    fn remove(&mut self, world: u32, entry: u32, frame: u64) {
        let index = self.index_mut(world);
        let unit = index.unit_of[entry as usize];
        if unit != NONE {
            let held = &mut index.units[unit as usize];
            if held.rest.is_empty() {
                let place = held.place;
                match place {
                    Place::Filed(filing) => index.grid.unfile(unit, &filing, frame),
                    Place::Loose => index.take_from(unit, false),
                    Place::Waits => index.take_from(unit, true),
                }
                index.free_unit(unit);
            } else {
                // (a part leaving a unit of several: the others stay in it, its root to be worked out again)
                if held.first == entry {
                    held.first = held.rest.pop().expect("a unit of several");
                } else if let Some(at) = held.rest.iter().position(|&e| e == entry) {
                    held.rest.swap_remove(at);
                }
                if held.root == entry {
                    held.root = held.first;
                    held.root_stale = true;
                }
                held.unsorted = true;
                if held.place == Place::Waits {
                    index.waiting -= 1;
                }
            }
            index.unit_of[entry as usize] = NONE;
        }
        let part = index.parts[entry as usize];
        index.free.push(entry);
        let at = index.live_at[entry as usize] as usize;
        index.live.swap_remove(at);
        if let Some(&moved) = index.live.get(at) {
            index.live_at[moved as usize] = at as u32;
        }
        if self.slots.get(part.index as usize) == Some(&(world + 1, entry + 1)) {
            self.slots[part.index as usize] = (0, 0);
        }
    }
}

/// What a write does to a part's unit, by the property written ([`stored`])
#[derive(Clone, Copy, PartialEq)]
enum Change {
    /// CFrame, CanCollide, CanTouch: the unit's box or flags
    Geometry,
    /// Size: its box and, in a unit of several parts, its root
    Size,
    /// Massless, RootPriority, Name: the root of a unit of several parts
    Rank,
    Anchored,
}

/// A property of `id` was stored (instance.rs store: every write, signalled or not). A write of CFrame, Size, CanCollide or
/// CanTouch of a part an index holds files its unit again at the next settle ([`settle`]: a moved assembly once however many of its
/// parts moved), an Anchored write moves its unit to its new list at once, and a write of Size, Massless, RootPriority or Name of a
/// part of a unit of several files that unit again at the end of its world's wait (measured, Studio 0.747, a playing client and a
/// server alike, tools/parity/probes/grid_assemblies.luau: a pair filed in a cell, one part resized or made Massless, its
/// RootPriority or Name written, whether its root changes or not, is listed after the parts filed after it; a Material,
/// CustomPhysicalProperties, Shape, Transparency, Color, CollisionGroup, CanCollide or CanTouch write keeps its place, and a waiting
/// unit keeps its place in the wait).
#[inline]
pub fn stored(state: &mut State, reflection: &crate::Reflection, id: InstanceId, name: NameId) {
    let Some(&kind) = state.spatial.watched.get(name as usize) else { return };
    if kind == 0 {
        return;
    }
    let Some((world, entry)) = state.spatial.holder(id) else { return };
    let change = match kind {
        WATCH_GEOMETRY => Change::Geometry,
        WATCH_SIZE => Change::Size,
        WATCH_RANK => Change::Rank,
        _ => Change::Anchored,
    };
    let index = state.spatial.index(world);
    let unit = index.unit_of[entry as usize];
    if unit == NONE {
        return;
    }
    let several = index.units[unit as usize].len() > 1;
    let frame = state.sched.frame;
    match change {
        Change::Anchored => anchored_write(state, reflection, world, unit, entry, id),
        Change::Rank | Change::Size if several => {
            let index = state.spatial.index_mut(world);
            index.units[unit as usize].root_stale = true;
            index.units[unit as usize].unsorted = true;
            index.requeue(unit, frame);
        }
        Change::Rank => {}
        Change::Geometry | Change::Size => state.spatial.mark_dirty(world, unit),
    }
}

/// An Anchored write of `part` (its `entry`), in `unit` of `world`: the unit's anchored count and root kept up (an anchored part roots
/// its assembly; the root freed, the root is worked out again when next needed) and, filed, the unit at the end of the other list when
/// its list changed (measured: a pair or a chain welded, one part anchored, goes to the end of the anchored list as one, its root
/// first; freed again, to the end of the free list). The assemblies it splits or joins are worked out at the settle.
fn anchored_write(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32, entry: u32, part: InstanceId) {
    let Some(names) = state.spatial.names else { return };
    let now = names.anchored.and_then(|n| state.dom.boolean(part, n)) == Some(true);
    let index = state.spatial.index_mut(world);
    let before = index.parts[index.units[unit as usize].root as usize];
    let held = &mut index.units[unit as usize];
    let was = held.anchored;
    if now {
        held.anchored += 1;
    } else {
        held.anchored = held.anchored.saturating_sub(1);
    }
    if now && was == 0 {
        held.root = entry;
        held.root_stale = false;
        held.unsorted = true;
    } else if !now && held.root == entry {
        held.root_stale = true;
        held.unsorted = true;
    }
    let list = if held.anchored > 0 { grid::ANCHORED } else { grid::UNANCHORED };
    if let Place::Filed(filing) = held.place
        && list != filing.list
    {
        ready_unit(state, reflection, world, unit);
        let place = unit_place(state, &names, world, unit);
        let frame = state.sched.frame;
        state.spatial.index_mut(world).set_place(unit, place, false, frame);
    }
    state.spatial.pending.anchored.push((part, before));
}

/// The Parent write of `id` going on starts putting parts in waits (camera_moves.rs's hierarchy hook): they come in as `id`'s subtree
/// is taken up ([`settle`]).
pub(crate) fn parent_write(state: &mut State, id: InstanceId) {
    state.spatial.pending.moved = Some(id);
}

/// `part` came into the Workspace's world (camera_moves.rs's hierarchy hook numbers a moved subtree's parts in the order a client
/// takes the subtree up: the moved instance first, then the rest each after its own descendants, world/joints.rs taken_up). A
/// client keeps its parts in the order they came into the world (a part moved, written or put in another Folder of it keeps its
/// place; one taken out and back is last), which Workspace:JoinToOutsiders welds a part's outsiders in (world/joints.rs).
pub(crate) fn entered(state: &mut State, part: InstanceId) {
    let spatial = &mut state.spatial;
    spatial.entries += 1;
    spatial.entered.insert(part, spatial.entries);
}

/// `part` left the Workspace's world, or is freed: [`entered`] forgets it.
pub(crate) fn not_entered(state: &mut State, part: InstanceId) {
    if !state.spatial.entered.is_empty() {
        state.spatial.entered.remove(&part);
    }
}

/// When `part` came into the Workspace's world ([`entered`]): 0 for a part there before any Parent write.
pub(crate) fn entered_at(state: &State, part: InstanceId) -> u64 {
    state.spatial.entered.get(&part).copied().unwrap_or(0)
}

/// `part` came into the world of the WorldRoot `root` (camera_moves.rs's hierarchy hook, in the order a client takes the moved
/// subtree up): its world's index (made first when it has none) holds it from now, and the settle at the write's end puts it at the
/// end of the world's wait in a unit of its own, joined to the units its links reach as they start ([`settle`]), until a query of
/// that world or the physics step takes the wait in. A part already in that world's index stays where it is.
pub(crate) fn wait_in(state: &mut State, root: InstanceId, part: InstanceId) {
    if state.spatial.names.is_none() {
        return;
    }
    let world = match state.spatial.world(root) {
        Some(world) => world,
        None => state.spatial.make(root),
    };
    if state.spatial.holder(part).is_some_and(|(holder, _)| holder == world) {
        return;
    }
    let frame = state.sched.frame;
    let serial = state.dom.at(part).serial;
    state.spatial.add(world, part, Entry::unread(part), serial, frame);
    state.spatial.pending.entering.push((root, part));
}

/// `part` left the world of the WorldRoot `root` (camera_moves.rs's hierarchy hook, a moved subtree's parts in the order they would
/// come in): out of its index at once. A unit whose parts all leave goes at once, from its cells, its wait or the loose list (the
/// list's last taking its place: measured, a subtree's parts go out one at a time, tools/parity/probes/grid_subtree.luau); one that
/// keeps some loses this one and splits at the settle by the links that are left, as a client's assembly splits ([`settle`]).
pub(crate) fn wait_out(state: &mut State, reflection: &crate::Reflection, root: InstanceId, part: InstanceId) {
    let Some(world) = state.spatial.world(root) else { return };
    let Some((holder, entry)) = state.spatial.holder(part) else { return };
    if holder != world {
        return;
    }
    let frame = state.sched.frame;
    let unit = state.spatial.index(world).unit_of[entry as usize];
    if unit != NONE && state.spatial.index(world).units[unit as usize].len() > 1 {
        // (the parts of its unit still in its world: the hook runs after the move, so a part leaving with it is in another world now)
        let staying = {
            let index = state.spatial.index(world);
            let held = &index.units[unit as usize];
            let stays = |other: u32| other != entry && state.dom.home(index.parts[other as usize]) == Some(root);
            stays(held.root) || held.parts().any(stays)
        };
        if !staying {
            // (the unit out of its place at once, then its parts' entries)
            let index = state.spatial.index_mut(world);
            let gone: Vec<u32> = index.units[unit as usize].parts().collect();
            index.take_unit(unit, frame);
            index.free_unit(unit);
            for &other in &gone {
                index.unit_of[other as usize] = NONE;
            }
            for other in gone {
                let id = state.spatial.index(world).parts[other as usize];
                state.spatial.remove(world, other, frame);
                if let Some(instance) = state.dom.get_mut(id) {
                    instance.flags &= !(flags::INDEXED | flags::LOGGED);
                }
            }
            return;
        }
        let generation = state.spatial.index(world).units[unit as usize].generation;
        // (its joints in the order they named it, each with its other part: the piece through the first leaving part's oldest keeps the
        // unit when that was the root; every leaving part's other parts start the race that splits it)
        let mut links = super::joints::part_links(state, part);
        links.sort_by_key(|&(_, _, named)| named);
        let others = links.into_iter().map(|(_, other, _)| other);
        match state.spatial.pending.splits.iter_mut().find(|split| split.world == world && split.unit == unit && split.generation == generation) {
            Some(split) => split.links.extend(others),
            None => {
                let links = others.collect();
                let old_root = root_part(state, reflection, world, unit);
                state.spatial.pending.splits.push(Split { world, unit, generation, old_root, links });
            }
        }
    }
    if unit != NONE && state.spatial.names.and_then(|names| names.anchored).and_then(|n| state.dom.boolean(part, n)) == Some(true) {
        let held = &mut state.spatial.index_mut(world).units[unit as usize];
        held.anchored = held.anchored.saturating_sub(1);
    }
    state.spatial.remove(world, entry, frame);
    if let Some(instance) = state.dom.get_mut(part) {
        instance.flags &= !(flags::INDEXED | flags::LOGGED);
    }
}

/// What a write left for the settle at its end ([`settle`]): the subtree a Parent write moved and the parts it brought into waits
/// (in the order they came), the units that lost parts that left, the parts whose Anchored changed, and the units whose box changed
#[derive(Default)]
pub(crate) struct Pending {
    moved: Option<InstanceId>,
    entering: Vec<(InstanceId, InstanceId)>,
    splits: Vec<Split>,
    /// Parts whose Anchored was written, each with its unit's root before the write
    anchored: Vec<(InstanceId, InstanceId)>,
    dirty: Vec<(u32, u32, u32)>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.entering.is_empty() && self.splits.is_empty() && self.anchored.is_empty() && self.dirty.is_empty()
    }
}

/// A unit that lost a part that left its world, to split by the links left: its world, the unit and its generation (a unit freed
/// since is no longer this one), its root before, and the parts the leaving part's joints linked it to, in the order they named it
#[derive(Debug)]
struct Split {
    world: u32,
    unit: u32,
    generation: u32,
    old_root: InstanceId,
    links: Vec<InstanceId>,
}

/// Whether world/joints.rs kept the roots of the component of `unit`'s parts through the write ([`super::joints::roots_kept`]), read
/// before the settle's own reads make it a tree: a component of one anchored part or none needs nothing kept
fn claims_kept(state: &State, world: u32, unit: u32) -> bool {
    let index = state.spatial.index(world);
    let held = &index.units[unit as usize];
    let any = index.parts[held.first as usize];
    match super::joints::component_anchored(state, any) {
        Some(count) if count <= 1 => true,
        Some(_) => super::joints::roots_kept(state, any),
        None => held.anchored == 0,
    }
}

impl Spatial {
    /// Marks `unit` of `world` to be filed again at the next settle, once
    fn mark_dirty(&mut self, world: u32, unit: u32) {
        let held = &mut self.index_mut(world).units[unit as usize];
        if held.dirty || held.place == Place::Waits {
            return;
        }
        held.dirty = true;
        let generation = held.generation;
        self.pending.dirty.push((world, unit, generation));
    }

    /// The world and unit of `part`, if an index holds it in a unit
    fn unit_of(&self, part: InstanceId) -> Option<(u32, u32)> {
        let (world, entry) = self.holder(part)?;
        let unit = self.index(world).unit_of[entry as usize];
        (unit != NONE).then_some((world, unit))
    }
}

/// A write's settle (instance.rs, after the joints' work on every write; and before every query): what the write did to the worlds'
/// units, as a playing client files its rigid assemblies (measured, Studio 0.747, a playing client and a server alike,
/// tools/parity/probes/grid_welds.luau and grid_assemblies.luau). Each assembly is one unit of its world's grid ([`Unit`]):
/// - **Joins.** A link that joins two units takes the one not holding the joined assembly's root out of its place (the list's last
///   taking it), its parts into the root's unit; the root's unit keeps its place if it waits, and goes to the end of the wait if it
///   was filed (a b c d waiting, a-b welded: d b a c; filed: d c b a).
/// - **Parts coming in.** A Parent write's parts come in as the moved subtree is taken up (world/joints.rs taken_up), each at the end
///   of its world's wait in a unit of its own; a link starts when the last of its joint and its two parts comes in, a part's links in
///   the order its joints last named it, newest first (welds c-a and b-c made in that order, coming in with a x b y c, join b-c first:
///   y x a b c).
/// - **Splits.** A link that ends, or a part that leaves, splits its unit by the links left: the piece holding the root keeps the unit
///   (the root gone, the piece through the leaving part's oldest joint), at its place if it waits, else at the end of the wait; the
///   other pieces go to the end of the wait after it, by the leaving part's joints newest first (a hub of three leaves made a b c,
///   leaving x: a x c b).
/// - **Anchored writes** that split or join assemblies, and parts another anchored part claims when links change (a part two
///   anchored parts reach), go the same ways.
/// - Then the units whose box changed are filed again, in the order they changed.
///
/// Host work: a split is found by searches from the ends of what left, through the links among the unit's parts, the one with the
/// fewest links left going on first, until one is left ([`race`]): only the pieces that split off are walked, so a leaf leaving a hub
/// of 5,000 parts walks the leaf. A join moves the parts of the unit without the root. Where two anchored parts' assemblies meet, the
/// parts of the units involved are put in units by their roots as world/joints.rs works them out ([`regroup`]).
pub(crate) fn settle(state: &mut State, reflection: &crate::Reflection) {
    if state.joints.grid_links.is_empty() && state.spatial.pending.is_empty() {
        return;
    }
    // (a write that only moved or resized parts: their units filed again, nothing else to do)
    if state.joints.grid_links.is_empty() && state.spatial.pending.entering.is_empty() && state.spatial.pending.splits.is_empty() && state.spatial.pending.anchored.is_empty() {
        refile_marked(state);
        return;
    }
    let links = std::mem::take(&mut state.joints.grid_links);
    if state.spatial.names.is_none() {
        state.spatial.pending = Pending::default();
        return;
    }
    let mut pending = std::mem::take(&mut state.spatial.pending);
    let frame = state.sched.frame;
    // the links that start as the write's parts come in, each when the last of its joint and parts comes in
    let replayed = replay_order(state, &pending, &links);
    // units that lost parts or links: each split once, by a race from every part next to what it lost (the leaving parts' joints'
    // other parts, oldest named first, then the ends of the links that ended), so the rest of it is one piece
    let mut work: Vec<Split> = Vec::new();
    for split in pending.splits.drain(..) {
        let live = state.spatial.worlds.get(split.world as usize).and_then(|index| index.as_deref()).is_some_and(|index| index.units.get(split.unit as usize).is_some_and(|held| held.generation == split.generation && held.first != NONE));
        if live {
            work.push(split);
        }
    }
    let leaving = work.len();
    // (the units on the far side of what a unit lost: with several anchored parts in their component, the change may have moved parts
    // between their assemblies though no link of theirs changed)
    let mut beside: Vec<(u32, u32, u32)> = Vec::new();
    let mut near = |state: &State, part: InstanceId, from: Option<(u32, u32)>| {
        if let Some((world, unit)) = state.spatial.unit_of(part)
            && Some((world, unit)) != from
            && !beside.iter().any(|&(w, u, _)| w == world && u == unit)
        {
            beside.push((world, unit, state.spatial.index(world).units[unit as usize].generation));
        }
    };
    for split in &work {
        for &part in &split.links {
            near(state, part, Some((split.world, split.unit)));
        }
    }
    for &(_, first, second, started) in &links {
        if started {
            continue;
        }
        let (a, b) = (state.spatial.unit_of(first), state.spatial.unit_of(second));
        if a != b || a.is_none() {
            near(state, first, None);
            near(state, second, None);
        }
        let (Some((world, a)), Some((other, b))) = (a, b) else { continue };
        if world != other || a != b {
            continue;
        }
        if super::joints::pair_linked(state, first, second) {
            // (still linked by another joint: nothing splits, but which anchored part a part hangs from may have changed)
            near(state, first, None);
            continue;
        }
        match work.iter_mut().find(|split| split.world == world && split.unit == a) {
            Some(split) => split.links.extend([first, second]),
            None => {
                let generation = state.spatial.index(world).units[a as usize].generation;
                let old_root = root_part(state, reflection, world, a);
                work.push(Split { world, unit: a, generation, old_root, links: vec![first, second] });
            }
        }
    }
    let kept: Vec<bool> = work.iter().map(|split| claims_kept(state, split.world, split.unit)).collect();
    state.spatial.deferring = true;
    let beside: Vec<(u32, u32, u32)> = beside.into_iter().filter(|&(world, unit, _)| !claims_kept(state, world, unit)).collect();
    for (at, split) in work.into_iter().enumerate() {
        // (a unit an earlier split joined to another is gone; its parts went with it)
        let live = state.spatial.worlds.get(split.world as usize).and_then(|index| index.as_deref()).is_some_and(|index| index.units.get(split.unit as usize).is_some_and(|held| held.generation == split.generation && held.first != NONE));
        if live {
            split_leaving(state, reflection, split.world, split.unit, split.old_root, &split.links, at < leaving, kept[at], frame);
        }
    }
    state.spatial.deferring = false;
    flush_later(state, reflection, frame);
    for (world, unit, generation) in beside {
        let live = state.spatial.worlds.get(world as usize).and_then(|index| index.as_deref()).is_some_and(|index| index.units.get(unit as usize).is_some_and(|held| held.generation == generation && held.first != NONE));
        if live {
            let parts: Vec<InstanceId> = {
                let index = state.spatial.index(world);
                index.units[unit as usize].parts().map(|entry| index.parts[entry as usize]).collect()
            };
            regroup(state, reflection, world, &parts, frame);
        }
    }
    // links that started outside the parts coming in
    for &(joint, first, second, started) in &links {
        if started && !replayed.joints.contains(&joint) {
            join_parts(state, reflection, first, second, frame);
        }
    }
    // Anchored writes that split or join assemblies
    for (part, before) in pending.anchored.drain(..) {
        reanchored(state, reflection, part, before, frame);
    }
    // the parts coming in, each with the links it completes
    let mut next = 0;
    for (at, &(root, part)) in pending.entering.iter().enumerate() {
        while let Some(link) = replayed.order.get(next).filter(|link| link.when < replayed.part_events[at]) {
            join_parts(state, reflection, link.parts[0], link.parts[1], frame);
            next += 1;
        }
        if let Some(world) = state.spatial.world(root)
            && let Some((holder, entry)) = state.spatial.holder(part)
            && holder == world
            && state.spatial.index(world).unit_of[entry as usize] == NONE
        {
            let names = state.spatial.names.expect("names");
            let unit = state.spatial.index_mut(world).new_unit(&[entry]);
            count_anchored(state, &names, world, unit);
            state.spatial.index_mut(world).set_place(unit, Place::Waits, true, frame);
        }
        while let Some(link) = replayed.order.get(next).filter(|link| link.when == replayed.part_events[at]) {
            join_parts(state, reflection, link.parts[0], link.parts[1], frame);
            next += 1;
        }
    }
    for link in &replayed.order[next..] {
        join_parts(state, reflection, link.parts[0], link.parts[1], frame);
    }
    // the pieces whose root came in after them
    for (part, root) in std::mem::take(&mut state.spatial.orphans) {
        if let (Some((world, piece)), Some((other, home))) = (state.spatial.unit_of(part), state.spatial.unit_of(root))
            && world == other
            && piece != home
            && super::joints::grid_root(state, reflection, part) == root
        {
            join_units(state, reflection, world, piece, home, frame);
        }
    }
    // the units whose box changed, unless a bulk move is still placing parts (Model:PivotTo, ScaleTo: the next settle files them)
    if state.joints.bulk_placing() {
        pending.dirty.retain(|&(world, unit, generation)| state.spatial.worlds.get(world as usize).and_then(|index| index.as_deref()).is_some_and(|index| index.units.get(unit as usize).is_some_and(|held| held.generation == generation)));
        state.spatial.pending.dirty.append(&mut pending.dirty);
    } else {
        for (world, unit, generation) in pending.dirty.drain(..) {
            refile_dirty(state, world, unit, generation, frame);
        }
    }
    // (the link list's storage back, if no link came since)
    let mut links = links;
    links.clear();
    if state.joints.grid_links.is_empty() {
        state.joints.grid_links = links;
    }
}

/// A unit marked to be filed again ([`Spatial::mark_dirty`]), filed as its parts stand now if it is still the one marked and filed
fn refile_dirty(state: &mut State, world: u32, unit: u32, generation: u32, frame: u64) {
    let Some(names) = state.spatial.names else { return };
    let Some(index) = state.spatial.worlds.get(world as usize).and_then(|index| index.as_deref()) else { return };
    let Some(held) = index.units.get(unit as usize).filter(|held| held.generation == generation && held.first != NONE) else { return };
    if held.place == Place::Waits {
        return;
    }
    let place = unit_place(state, &names, world, unit);
    let index = state.spatial.index_mut(world);
    index.units[unit as usize].dirty = false;
    index.set_place(unit, place, false, frame);
}

/// The links a write's parts complete as they come in ([`settle`]), in the order they start: each when the last of its joint and its
/// two parts came in (by its place in the moved subtree's take-up, world/joints.rs taken_up), the links a part completes newest
/// named first ([`super::joints::part_links`]), and the joints they belong to (their own starts are not joins again)
#[derive(Default)]
struct Replayed {
    order: Vec<ReplayLink>,
    joints: FxHashSet<InstanceId>,
    /// The take-up place of each part coming in, by its place in the write's list of them
    part_events: Vec<u32>,
}

struct ReplayLink {
    when: u32,
    newest: std::cmp::Reverse<u64>,
    parts: [InstanceId; 2],
}

fn replay_order(state: &State, pending: &Pending, links: &[(InstanceId, InstanceId, InstanceId, bool)]) -> Replayed {
    let mut replayed = Replayed::default();
    if pending.entering.is_empty() {
        return replayed;
    }
    // (each link once, by its joint; when each part coming in named each of its joints)
    let mut candidates: FxHashMap<InstanceId, [InstanceId; 2]> = FxHashMap::default();
    let mut named: FxHashMap<(InstanceId, InstanceId), u64> = FxHashMap::default();
    if state.joints.created > 0 {
        for &(_, part) in &pending.entering {
            for (joint, other, stamp) in super::joints::part_links(state, part) {
                candidates.entry(joint).or_insert([part, other]);
                named.insert((joint, part), stamp);
            }
        }
        for &(joint, first, second, started) in links {
            if started {
                candidates.entry(joint).or_insert([first, second]);
            }
        }
    }
    let Some(moved) = pending.moved.filter(|_| !candidates.is_empty()) else {
        replayed.part_events = (1..=pending.entering.len() as u32).collect();
        return replayed;
    };
    // (the take-up's places, from 1: a joint or part that was there before the write is at 0)
    let order = super::joints::taken_up(state, moved, |_, _| false);
    let place: FxHashMap<InstanceId, u32> = order.iter().enumerate().map(|(at, &id)| (id, at as u32 + 1)).collect();
    let entering: FxHashMap<InstanceId, u32> = pending.entering.iter().map(|&(_, part)| (part, place.get(&part).copied().unwrap_or(0))).collect();
    replayed.part_events = pending.entering.iter().map(|&(_, part)| entering[&part]).collect();
    let came = |id: InstanceId| entering.get(&id).copied().unwrap_or(0);
    for (joint, [first, second]) in candidates {
        let when = place.get(&joint).copied().unwrap_or(0).max(came(first)).max(came(second));
        if when == 0 {
            continue;
        }
        // (the links a part completes, newest named first: the part that came in last of the three orders them)
        let by = if came(first) == when { first } else { second };
        let stamp = named.get(&(joint, by)).copied().unwrap_or(0);
        replayed.joints.insert(joint);
        replayed.order.push(ReplayLink { when, newest: std::cmp::Reverse(stamp), parts: [first, second] });
    }
    replayed.order.sort_by_key(|link| (link.when, link.newest));
    replayed
}

/// The root of `unit`'s assembly worked out again from its parts: the part a client roots it at (world/joints.rs root_first: an
/// anchored one, else the better ranked, of equal ones the later made)
fn refresh_root(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32) {
    let index = state.spatial.index(world);
    let held = &index.units[unit as usize];
    let mut best = held.first;
    for entry in held.rest.iter().copied() {
        if super::joints::root_first(state, reflection, index.parts[entry as usize], index.parts[best as usize]) {
            best = entry;
        }
    }
    let index = state.spatial.index_mut(world);
    let held = &mut index.units[unit as usize];
    if held.root != best {
        held.unsorted = true;
    }
    held.root = best;
    held.root_stale = false;
}

/// The root of `unit` as a part, worked out again first if stale
fn root_part(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32) -> InstanceId {
    if state.spatial.index(world).units[unit as usize].root_stale {
        refresh_root(state, reflection, world, unit);
    }
    let index = state.spatial.index(world);
    index.parts[index.units[unit as usize].root as usize]
}

/// A link that started between `first` and `second` joins their units when it joins their assemblies ([`join_units`]); two parts in
/// one unit already, in no index or in two worlds stay. Two units that each hold an anchored part are two assemblies, which the link
/// may only move parts between (a part both reach, claimed by the newer link's: world/joints.rs claims): their parts go in units by
/// their roots ([`regroup`]).
fn join_parts(state: &mut State, reflection: &crate::Reflection, first: InstanceId, second: InstanceId, frame: u64) {
    let (Some((world, a)), Some((other, b))) = (state.spatial.unit_of(first), state.spatial.unit_of(second)) else { return };
    if world != other || a == b {
        return;
    }
    let index = state.spatial.index(world);
    let (fixed_a, fixed_b) = (index.units[a as usize].anchored > 0, index.units[b as usize].anchored > 0);
    if fixed_a && fixed_b {
        // (a link between two anchored parts moves nothing: each roots its own assembly)
        let anchored = |part: InstanceId| state.spatial.names.and_then(|names| names.anchored).and_then(|n| state.dom.boolean(part, n)) == Some(true);
        if !(anchored(first) && anchored(second)) {
            regroup(state, reflection, world, &[first, second], frame);
        }
        return;
    }
    // (a free unit meeting an anchored one, in a component of several anchored parts: its parts hang from the anchored part the newer
    // links give them, which may be another's)
    if fixed_a != fixed_b && super::joints::component_anchored(state, first).is_none_or(|count| count > 1) {
        let (free, held) = if fixed_a { (b, a) } else { (a, b) };
        let root = root_part(state, reflection, world, held);
        let parts: Vec<InstanceId> = {
            let index = state.spatial.index(world);
            index.units[free as usize].parts().map(|entry| index.parts[entry as usize]).collect()
        };
        if !parts.iter().all(|&part| super::joints::grid_root(state, reflection, part) == root) {
            let mut all = parts;
            all.push(root);
            regroup(state, reflection, world, &all, frame);
            return;
        }
    }
    join_units(state, reflection, world, a, b, frame);
}

/// Joins two units of `world` ([`settle`]): the one not holding the better root out of its place (the list's last taking it), its
/// parts into the other; that one at the end of the wait if it was filed (or loose), else where it waits. Its survivor.
fn join_units(state: &mut State, reflection: &crate::Reflection, world: u32, a: u32, b: u32, frame: u64) -> u32 {
    let (root_a, root_b) = (root_part(state, reflection, world, a), root_part(state, reflection, world, b));
    let (survivor, absorbed) = if super::joints::root_first(state, reflection, root_b, root_a) { (b, a) } else { (a, b) };
    let index = state.spatial.index_mut(world);
    index.take_unit(absorbed, frame);
    let moved: Vec<u32> = index.units[absorbed as usize].parts().collect();
    let anchored = index.units[absorbed as usize].anchored;
    index.free_unit(absorbed);
    for &entry in &moved {
        index.unit_of[entry as usize] = survivor;
    }
    if index.units[survivor as usize].place == Place::Waits {
        index.waiting += moved.len();
    }
    let held = &mut index.units[survivor as usize];
    held.rest.extend(moved);
    held.anchored += anchored;
    held.unsorted = true;
    index.requeue(survivor, frame);
    survivor
}

/// What [`race`] found: the pieces its searches finished, each with the index of the start it came from (merged searches keep the
/// lower), the start whose search was still going when all others had finished (its piece is the rest of the unit), and whether a
/// search met a link to a part outside the unit
struct Raced {
    done: Vec<(usize, Vec<u32>)>,
    open: Option<usize>,
    outside: bool,
    /// The parts it went through that link to a part outside the unit
    borders: Vec<InstanceId>,
}

/// The pieces of `unit` of `world` from `starts` (parts in it), through the links among its parts but through no part of `skip`:
/// one search from each start, the one with the fewest links left to follow going on first, two that meet one search, until at most
/// one is still going. The others' parts are pieces of their own; the one going holds the rest of the unit, unwalked.
fn race(state: &State, world: u32, unit: u32, starts: &[InstanceId]) -> Raced {
    let index = state.spatial.index(world);
    let in_unit = |part: InstanceId| state.spatial.holder(part).filter(|&(holder, _)| holder == world).map(|(_, entry)| entry).filter(|&entry| index.unit_of[entry as usize] == unit);
    struct Search {
        frontier: Vec<u32>,
        found: Vec<u32>,
        work: usize,
        into: usize,
    }
    let mut searches: Vec<Search> = Vec::new();
    let mut owner: FxHashMap<u32, usize> = FxHashMap::default();
    let mut start_of: Vec<usize> = Vec::new();
    for (at, &start) in starts.iter().enumerate() {
        let Some(entry) = in_unit(start) else { continue };
        if owner.contains_key(&entry) {
            continue;
        }
        owner.insert(entry, searches.len());
        start_of.push(at);
        searches.push(Search { frontier: vec![entry], found: vec![entry], work: super::joints::link_count(state, start), into: usize::MAX });
    }
    fn find(searches: &[Search], mut at: usize) -> usize {
        while searches[at].into != usize::MAX {
            at = searches[at].into;
        }
        at
    }
    let mut outside = false;
    let mut borders: Vec<InstanceId> = Vec::new();
    loop {
        let going: Vec<usize> = (0..searches.len()).filter(|&at| searches[at].into == usize::MAX && !searches[at].frontier.is_empty()).collect();
        if going.len() <= 1 {
            break;
        }
        let at = *going.iter().min_by_key(|&&at| searches[at].work).expect("two going");
        let entry = searches[at].frontier.pop().expect("a frontier");
        let part = index.parts[entry as usize];
        searches[at].work = searches[at].work.saturating_sub(super::joints::link_count(state, part));
        for (_, other, _) in super::joints::part_links(state, part) {
            let Some(next) = in_unit(other) else {
                if !outside || borders.last() != Some(&part) {
                    borders.push(part);
                }
                outside = true;
                continue;
            };
            match owner.get(&next).copied() {
                Some(met) => {
                    let met = find(&searches, met);
                    if met != at {
                        // (two searches meet: one piece, searched on by the first)
                        let (keep, gone) = (at.min(met), at.max(met));
                        let taken = std::mem::take(&mut searches[gone].frontier);
                        let found = std::mem::take(&mut searches[gone].found);
                        let work = searches[gone].work;
                        searches[gone].into = keep;
                        searches[keep].frontier.extend(taken);
                        searches[keep].found.extend(found);
                        searches[keep].work += work;
                        // (one search now: the part's other links later, if the race goes on)
                        searches[keep].frontier.push(entry);
                        searches[keep].work += super::joints::link_count(state, part);
                        break;
                    }
                }
                None => {
                    owner.insert(next, at);
                    searches[at].frontier.push(next);
                    searches[at].found.push(next);
                    searches[at].work += super::joints::link_count(state, other);
                }
            }
        }
    }
    let mut done = Vec::new();
    let mut open = None;
    for (at, search) in searches.into_iter().enumerate() {
        if search.into != usize::MAX {
            continue;
        }
        if search.frontier.is_empty() {
            done.push((start_of[at], search.found));
        } else {
            open = Some(start_of[at]);
        }
    }
    // (a single search left with nothing to follow is the whole rest too)
    if open.is_none() && done.len() == 1 {
        open = Some(done.pop().expect("one").0);
    }
    Raced { done, open, outside, borders }
}

/// `unit` of `world` lost parts (`lost`: they left, [`wait_out`]) or links ([`settle`]): it splits by the links left, a race from the
/// parts next to what it lost (`links`: a leaving part's joints' other parts, oldest named first, then the ends of links that ended).
#[allow(clippy::too_many_arguments)]
fn split_leaving(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32, old_root: InstanceId, links: &[InstanceId], lost: bool, kept: bool, frame: u64) {
    if lost && !links.iter().any(|&part| state.spatial.unit_of(part) == Some((world, unit))) {
        // (none of its joints reached a part of the unit, as when it was freed: the unit's parts by their roots)
        state.spatial.index_mut(world).requeue(unit, frame);
        let parts: Vec<InstanceId> = {
            let index = state.spatial.index(world);
            index.units[unit as usize].parts().map(|entry| index.parts[entry as usize]).collect()
        };
        later_regroup(state, reflection, world, parts, frame);
        return;
    }
    let raced = race(state, world, unit, links);
    apply_split(state, reflection, world, unit, raced, old_root, links, lost, kept, frame);
}

/// The pieces [`race`] found, put in units ([`settle`]): the piece holding `old_root` keeps the unit (when it is gone, the piece of the
/// first of `starts` that is in one), at its place if it waits, else at the end of the wait; the other pieces at the end of the wait
/// after it, by `starts` from the last. `lost`: a part left the unit (its root worked out again; filed, the unit at the end). With
/// several anchored parts in the component, or a link met to a part outside the unit, the pieces go in units by their roots.
#[allow(clippy::too_many_arguments)]
fn apply_split(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32, raced: Raced, old_root: InstanceId, starts: &[InstanceId], lost: bool, kept: bool, frame: u64) {
    let Raced { mut done, open, outside, borders } = raced;
    let names = state.spatial.names.expect("names");
    // (several anchored parts in the component: a change may move parts between their assemblies while they stay linked, at the parts
    // linked to another unit: when the race went through one whose root is another unit's now, the units' parts go in units by their
    // roots; else the pieces that met a link out of the unit do)
    let claims = outside;
    let several = {
        let any = state.spatial.index(world).parts[state.spatial.index(world).units[unit as usize].first as usize];
        super::joints::component_anchored(state, any).is_none_or(|count| count > 1)
    };
    let mut borders = borders;
    if several {
        // (the race stops once one search is left: the free parts it started from that link out of their unit are borders too, but
        // for one of many links, a hub, which the race would have to walk through)
        let anchored = state.spatial.names.and_then(|names| names.anchored);
        for &part in starts {
            if borders.contains(&part) || anchored.and_then(|n| state.dom.boolean(part, n)) == Some(true) || super::joints::link_count(state, part) > 64 {
                continue;
            }
            let home = state.spatial.unit_of(part);
            if home.is_some() && super::joints::part_links(state, part).iter().any(|&(_, other, _)| state.spatial.unit_of(other) != home) {
                borders.push(part);
            }
        }
    }
    let unsure = several && (!kept || borders.iter().any(|&part| {
        let home = state.spatial.unit_of(part);
        let root = super::joints::grid_root(state, reflection, part);
        home.is_some_and(|(_, at)| state.spatial.unit_of(root).is_none_or(|(_, other)| other != at))
    }));
    if done.is_empty() {
        // (one piece: the unit keeps its parts; one that lost a part is filed again, and one that lost its root may hang from another
        // assembly now)
        if lost {
            state.spatial.index_mut(world).requeue(unit, frame);
            let root_gone = state.spatial.holder(old_root).is_none_or(|(holder, entry)| holder != world || state.spatial.index(world).unit_of[entry as usize] != unit);
            if root_gone {
                state.spatial.index_mut(world).units[unit as usize].root_stale = true;
                later_join(state, reflection, world, unit, frame);
                return;
            }
        }
        if unsure {
            let parts: Vec<InstanceId> = {
                let index = state.spatial.index(world);
                index.units[unit as usize].parts().map(|entry| index.parts[entry as usize]).collect()
            };
            later_regroup(state, reflection, world, parts, frame);
            later_join(state, reflection, world, unit, frame);
        }
        return;
    }
    // the piece that keeps the unit: the root's, else the first start's
    let of_entry = |state: &State, part: InstanceId| state.spatial.holder(part).filter(|&(holder, _)| holder == world).map(|(_, entry)| entry);
    let root_entry = of_entry(state, old_root).filter(|&entry| state.spatial.index(world).unit_of[entry as usize] == unit);
    let piece_of = |state: &State, done: &[(usize, Vec<u32>)], entry: u32| -> Option<usize> {
        let _ = state;
        done.iter().position(|(_, found)| found.contains(&entry))
    };
    let kept_done: Option<usize> = match root_entry {
        Some(entry) => piece_of(state, &done, entry),
        None => {
            // (the root left: the piece through its oldest joint)
            let first_start = starts.iter().enumerate().find_map(|(at, &part)| of_entry(state, part).filter(|&entry| state.spatial.index(world).unit_of[entry as usize] == unit).map(|entry| (at, entry)));
            match first_start {
                Some((at, entry)) if open != Some(at) => piece_of(state, &done, entry),
                _ => None,
            }
        }
    };
    // the rest of the unit (the open search's piece) as its own list when it does not keep the unit
    if let Some(kept) = kept_done {
        let taken: FxHashSet<u32> = done.iter().flat_map(|(_, found)| found.iter().copied()).collect();
        let rest: Vec<u32> = state.spatial.index(world).units[unit as usize].parts().filter(|entry| !taken.contains(entry)).collect();
        let kept_parts = std::mem::take(&mut done[kept].1);
        done.remove(kept);
        if !rest.is_empty() {
            done.push((open.unwrap_or(usize::MAX), rest));
        }
        set_parts(state, &names, world, unit, kept_parts);
    } else {
        // (the unit keeps the rest: the done pieces' parts leave it, the rest unwalked)
        let taken: Vec<u32> = done.iter().flat_map(|(_, found)| found.iter().copied()).collect();
        let gone_anchored = taken.iter().filter(|&&entry| names.anchored.and_then(|n| state.dom.boolean(state.spatial.index(world).parts[entry as usize], n)) == Some(true)).count() as u32;
        let index = state.spatial.index_mut(world);
        let held = &mut index.units[unit as usize];
        let small: FxHashSet<u32> = taken.iter().copied().collect();
        held.rest.retain(|entry| !small.contains(entry));
        if small.contains(&held.first) {
            held.first = held.rest.pop().expect("the rest holds a part");
        }
        held.anchored = held.anchored.saturating_sub(gone_anchored);
        held.unsorted = true;
        if held.place == Place::Waits {
            index.waiting -= taken.len();
        }
    }
    if root_entry.is_none() || lost {
        state.spatial.index_mut(world).units[unit as usize].root_stale = true;
    }
    state.spatial.index_mut(world).requeue(unit, frame);
    // the other pieces at the end of the wait: by the starts from the last (the leaving part's newest joint first)
    done.sort_by_key(|&(at, _)| std::cmp::Reverse(at));
    let mut pieces = vec![unit];
    for (_, found) in done {
        let index = state.spatial.index_mut(world);
        let piece = index.new_unit(&found);
        index.set_place(piece, Place::Waits, true, frame);
        count_anchored(state, &names, world, piece);
        refresh_root(state, reflection, world, piece);
        pieces.push(piece);
    }
    if claims {
        let parts: Vec<InstanceId> = {
            let index = state.spatial.index(world);
            let from = if unsure { 0 } else { 1 };
            pieces[from..].iter().flat_map(|&piece| index.units[piece as usize].parts().map(|entry| index.parts[entry as usize])).collect()
        };
        later_regroup(state, reflection, world, parts, frame);
    }
    // (each piece, and the unit when its root left, joins the units its parts' links reach: a piece that hung from the root may hang
    // from another anchored part now)
    let pieces: Vec<InstanceId> = {
        let index = state.spatial.index(world);
        pieces.iter().filter(|&&piece| index.units.get(piece as usize).is_some_and(|held| held.first != NONE)).map(|&piece| index.parts[index.units[piece as usize].first as usize]).collect()
    };
    for (at, first) in pieces.into_iter().enumerate() {
        if at == 0 && root_entry.is_some() {
            continue;
        }
        if let Some((holder, piece)) = state.spatial.unit_of(first)
            && holder == world
        {
            later_join(state, reflection, world, piece, frame);
        }
    }
}

/// [`regroup`] now, or once the settle's splits are done ([`Spatial::deferring`])
fn later_regroup(state: &mut State, reflection: &crate::Reflection, world: u32, parts: Vec<InstanceId>, frame: u64) {
    if state.spatial.deferring {
        state.spatial.later_regroups.push((world, parts));
    } else {
        regroup(state, reflection, world, &parts, frame);
    }
}

/// [`join_neighbours`] now, or once the settle's splits are done ([`Spatial::deferring`]), for the unit holding `unit`'s first part
fn later_join(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32, frame: u64) {
    if state.spatial.deferring {
        let first = state.spatial.index(world).parts[state.spatial.index(world).units[unit as usize].first as usize];
        state.spatial.later_joins.push((world, first));
    } else {
        join_neighbours(state, reflection, world, unit, frame);
    }
}

/// The joins the settle's splits asked for ([`later_regroup`], [`later_join`]), in turn
fn flush_later(state: &mut State, reflection: &crate::Reflection, frame: u64) {
    for (world, parts) in std::mem::take(&mut state.spatial.later_regroups) {
        let parts: Vec<InstanceId> = parts.into_iter().filter(|&part| state.spatial.unit_of(part).is_some_and(|(holder, _)| holder == world)).collect();
        if !parts.is_empty() {
            regroup(state, reflection, world, &parts, frame);
        }
    }
    for (world, part) in std::mem::take(&mut state.spatial.later_joins) {
        if let Some((holder, unit)) = state.spatial.unit_of(part)
            && holder == world
        {
            join_neighbours(state, reflection, world, unit, frame);
        }
    }
}

/// `unit` of `world` joins the units its parts' links reach ([`join_parts`]).
fn join_neighbours(state: &mut State, reflection: &crate::Reflection, world: u32, unit: u32, frame: u64) {
    let members: Vec<InstanceId> = {
        let index = state.spatial.index(world);
        index.units[unit as usize].parts().map(|entry| index.parts[entry as usize]).collect()
    };
    for member in members {
        for (_, other, _) in super::joints::part_links(state, member) {
            join_parts(state, reflection, member, other, frame);
        }
    }
}

/// `unit` of `world` holds `parts` (all already its, or moved to it), its anchored count recounted from them
fn set_parts(state: &mut State, names: &Names, world: u32, unit: u32, parts: Vec<u32>) {
    let index = state.spatial.index_mut(world);
    let before = index.units[unit as usize].len();
    for &entry in &parts {
        index.unit_of[entry as usize] = unit;
    }
    let held = &mut index.units[unit as usize];
    let count = parts.len();
    held.first = parts[0];
    held.rest = parts[1..].to_vec();
    held.unsorted = true;
    if held.place == Place::Waits {
        index.waiting = index.waiting + count - before;
    }
    count_anchored(state, names, world, unit);
}

/// An Anchored write of `part` settled ([`settle`]): a part anchored in a unit that holds another anchored part roots an assembly of
/// its own (a part linked to that unit alone, by one link to a part that keeps its root, splits off by itself; else the unit's parts go
/// in units by their roots); a part freed has its unit join the units its parts' links reach (two anchored ones: by their roots).
fn reanchored(state: &mut State, reflection: &crate::Reflection, part: InstanceId, before: InstanceId, frame: u64) {
    let Some(names) = state.spatial.names else { return };
    let Some((world, unit)) = state.spatial.unit_of(part) else { return };
    let now = names.anchored.and_then(|n| state.dom.boolean(part, n)) == Some(true);
    let held = &state.spatial.index(world).units[unit as usize];
    if now {
        if held.anchored <= 1 || held.len() == 1 {
            return;
        }
        // (a leaf: one link, to a part of the unit that keeps its root: a newer link may have its other part claimed by this one)
        let links = super::joints::part_links(state, part);
        let leaf = links.len() == 1 && state.spatial.unit_of(links[0].1) == Some((world, unit)) && super::joints::grid_root(state, reflection, links[0].1) == before;
        if leaf {
            let (_, entry) = state.spatial.holder(part).expect("held");
            let raced = Raced { done: vec![(1, vec![entry])], open: Some(0), outside: false, borders: Vec::new() };
            apply_split(state, reflection, world, unit, raced, before, &[links[0].1, part], false, true, frame);
            return;
        }
        let parts: Vec<InstanceId> = {
            let index = state.spatial.index(world);
            index.units[unit as usize].parts().map(|entry| index.parts[entry as usize]).collect()
        };
        regroup(state, reflection, world, &parts, frame);
        return;
    }
    // freed: its unit joins the units its parts' links reach (a piece an anchored part held joins the assembly it hangs from now)
    join_neighbours(state, reflection, world, unit, frame);
}

/// The units holding `parts` (of `world`) made again by their parts' roots, as world/joints.rs works them out now: each unit keeps the
/// parts its root's assembly holds (the root's unit, the root of none of them gone: the piece of its first part), the others go to the
/// end of the wait (the better root first), and each piece joins the unit of its root when that root is in another unit.
fn regroup(state: &mut State, reflection: &crate::Reflection, world: u32, parts: &[InstanceId], frame: u64) {
    let names = state.spatial.names.expect("names");
    // the units involved, each with its parts by root
    let mut units: Vec<u32> = Vec::new();
    for &part in parts {
        if let Some((holder, unit)) = state.spatial.unit_of(part)
            && holder == world
            && !units.contains(&unit)
        {
            units.push(unit);
        }
    }
    let mut pieces: Vec<(InstanceId, u32)> = Vec::new();
    for unit in units {
        let members: Vec<u32> = state.spatial.index(world).units[unit as usize].parts().collect();
        let mut groups: Vec<(InstanceId, Vec<u32>)> = Vec::new();
        for entry in members {
            let part = state.spatial.index(world).parts[entry as usize];
            let root = super::joints::grid_root(state, reflection, part);
            match groups.iter_mut().find(|(r, _)| *r == root) {
                Some((_, list)) => list.push(entry),
                None => groups.push((root, vec![entry])),
            }
        }
        if groups.len() == 1 {
            pieces.push((groups[0].0, unit));
            state.spatial.index_mut(world).units[unit as usize].root_stale = true;
            continue;
        }
        let old_root = root_part(state, reflection, world, unit);
        let kept = groups.iter().position(|(_, list)| list.iter().any(|&entry| state.spatial.index(world).parts[entry as usize] == old_root)).unwrap_or(0);
        let (kept_root, kept_parts) = groups.remove(kept);
        set_parts(state, &names, world, unit, kept_parts);
        state.spatial.index_mut(world).units[unit as usize].root_stale = true;
        state.spatial.index_mut(world).requeue(unit, frame);
        pieces.push((kept_root, unit));
        groups.sort_by(|x, y| if super::joints::root_first(state, reflection, x.0, y.0) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
        for (root, list) in groups {
            let index = state.spatial.index_mut(world);
            let piece = index.new_unit(&list);
            index.set_place(piece, Place::Waits, true, frame);
            count_anchored(state, &names, world, piece);
            state.spatial.index_mut(world).units[piece as usize].root_stale = true;
            pieces.push((root, piece));
        }
    }
    // each piece into the unit of its root, when that is another
    for (root, piece) in pieces {
        let Some(index) = state.spatial.worlds.get(world as usize).and_then(|index| index.as_deref()) else { return };
        if index.units.get(piece as usize).is_none_or(|held| held.first == NONE) {
            continue;
        }
        match state.spatial.unit_of(root) {
            Some((holder, home)) if holder == world && home != piece => {
                join_units(state, reflection, world, piece, home, frame);
            }
            None if root != state.spatial.index(world).parts[state.spatial.index(world).units[piece as usize].first as usize] => {
                let first = state.spatial.index(world).parts[state.spatial.index(world).units[piece as usize].first as usize];
                state.spatial.orphans.push((first, root));
            }
            _ => {}
        }
    }
}

/// Whether the world of `root` has an index holding any part, waiting ones too (camera_moves.rs: a subtree leaving it is walked
/// to take them out)
pub(crate) fn holds_any(state: &State, root: InstanceId) -> bool {
    state.spatial.world(root).is_some_and(|world| !state.spatial.index(world).is_empty())
}

/// The WorldModel `root` left the DataModel or was freed: its index goes, its parts' entries and its wait with it.
pub(crate) fn world_gone(state: &mut State, root: InstanceId) {
    let State { spatial, dom, .. } = state;
    spatial.drop_world(dom, root);
}

/// `id` is about to be freed: out of any index (its slot is reused), as a part leaving its world ([`wait_out`]).
pub(crate) fn freed(state: &mut State, reflection: &crate::Reflection, id: InstanceId) {
    not_entered(state, id);
    if let Some((world, _)) = state.spatial.holder(id) {
        let root = state.spatial.index(world).root;
        wait_out(state, reflection, root, id);
    }
}

/// Puts `part` in `world`'s index, or reads it again: its entry, and its unit filed again if it is filed (a new part is filed in a
/// unit of its own; one that waits is filed when its unit is taken in).
fn put(state: &mut State, engine: &Engine, names: Names, world: u32, part: InstanceId) {
    let workspace = engine.world().workspace;
    let entry = entry_of(state, engine, &names, part);
    let root = state.spatial.index(world).root;
    let frame = state.sched.frame;
    match state.spatial.holder(part).filter(|&(holder, _)| holder == world) {
        Some((_, at)) => {
            state.spatial.index_mut(world).entries[at as usize] = entry;
            // (a unit that waits stays where it is until taken in; one filed is filed again as its parts stand, once however many of
            // its parts the log names: open's end)
            let unit = state.spatial.index(world).unit_of[at as usize];
            if unit != NONE {
                state.spatial.mark_dirty(world, unit);
            }
        }
        None => {
            // (a part that came in without a Parent write: filed at once, joined to the units holding the rest of its assembly)
            let serial = state.dom.at(part).serial;
            let at = state.spatial.add_alone(world, part, entry, serial, Place::Waits, frame);
            let unit = state.spatial.index(world).unit_of[at as usize];
            count_anchored(state, &names, world, unit);
            let place = unit_place(state, &names, world, unit);
            state.spatial.index_mut(world).take_unit(unit, frame);
            state.spatial.index_mut(world).set_place(unit, place, true, frame);
            for (_, other, _) in super::joints::part_links(state, part) {
                join_parts(state, &engine.reflection, part, other, frame);
            }
        }
    }
    // (the Workspace's log keeps the writes of the parts its index holds from their first read: a part that came into the wait is
    // read here before it is taken in)
    if root == workspace {
        state.dom.at_mut(part).flags |= flags::INDEXED;
    }
}

/// Takes `part` out of `world`'s index, if it is in, as a part leaving its world ([`wait_out`]); whether it was.
fn take_out(state: &mut State, reflection: &crate::Reflection, world: u32, part: InstanceId) -> bool {
    let Some((holder, _)) = state.spatial.holder(part) else { return false };
    if holder != world {
        return false;
    }
    let root = state.spatial.index(world).root;
    wait_out(state, reflection, root, part);
    true
}

/// The index of the world of the WorldRoot `root` brought up to the dom's changes: made from a walk of its world the first time
/// (none for a WorldModel outside the DataModel), and its change log taken. Its id, if it has one, and what that cost: nothing
/// for the Workspace's; for a WorldModel's, cost.rs FIND_NODE for each instance its index looked at, in its log (what moved, and
/// the walks below what moved within the world) or in a walk of its world, and MODEL_INDEX_PART for each part such a walk filed
/// (ree's own work: a client files a world's parts as they come in, at the price of taking them in).
pub(crate) fn open(engine: &Engine, root: InstanceId) -> (Option<u32>, f64) {
    let workspace = engine.world().workspace;
    let Some(names) = engine.state.borrow().spatial.names else { return (None, 0.0) };
    let mut state = engine.state.borrow_mut();
    let state = &mut *state;
    settle(state, &engine.reflection);
    if state.dom.get(root).is_none() {
        return (None, 0.0);
    }
    if root == workspace {
        let world = open_workspace(state, engine, names, workspace);
        refile_marked(state);
        return (Some(world), 0.0);
    }
    if !in_game(state, engine, root) {
        return (None, 0.0);
    }
    let (world, looked, filed) = open_model(state, engine, names, root);
    refile_marked(state);
    (Some(world), looked as f64 * crate::cost::FIND_NODE + filed as f64 * crate::cost::MODEL_INDEX_PART)
}

/// The units marked to be filed again ([`Spatial::mark_dirty`]) filed now, in the order they were marked, unless a bulk move is
/// still placing parts
fn refile_marked(state: &mut State) {
    if state.spatial.pending.dirty.is_empty() || state.joints.bulk_placing() {
        return;
    }
    let frame = state.sched.frame;
    for (world, unit, generation) in std::mem::take(&mut state.spatial.pending.dirty) {
        refile_dirty(state, world, unit, generation, frame);
    }
}

/// Whether the WorldModel `model` is in the DataModel (a WorldModel outside it has no world): read off the outermost home above
/// it (dom.rs Dom::homes_above, one lookup however many WorldModels are nested above it: the Workspace is in the DataModel), then
/// up that home's parents, if it is not the Workspace.
fn in_game(state: &State, engine: &Engine, model: InstanceId) -> bool {
    let (workspace, game) = (engine.world().workspace, engine.world().game);
    let top = state.dom.homes_above(model).top;
    if top == Some(workspace) {
        return true;
    }
    let mut current = state.dom.parent_of(top.unwrap_or(model));
    while let Some(node) = current {
        if node == game {
            return true;
        }
        current = state.dom.parent_of(node);
    }
    false
}

/// [`open`] for the Workspace's world, and then the WorldModels inside it its walks met made indexes of their own.
fn open_workspace(state: &mut State, engine: &Engine, names: Names, workspace: InstanceId) -> u32 {
    let world = open_workspace_log(state, engine, names, workspace);
    // (each made index's walk may meet WorldModels inside its own)
    while let Some(model) = state.spatial.nested.pop() {
        if state.dom.get(model).is_some() && state.spatial.world(model).is_none() && in_game(state, engine, model) {
            open_model(state, engine, names, model);
        }
    }
    world
}

fn open_workspace_log(state: &mut State, engine: &Engine, names: Names, workspace: InstanceId) -> u32 {
    let made = state.spatial.world(workspace);
    let current = made.is_some_and(|world| state.spatial.index(world).read) && state.dom.watch.as_ref().is_some_and(|watch| !watch.overflowed && watch.root == workspace);
    let world = match made {
        Some(world) => world,
        None => state.spatial.make(workspace),
    };
    if !current {
        crate::profiler::caches::miss(crate::profiler::caches::Cache::Queries);
        walk_world(state, engine, names, world, workspace);
        state.dom.watch = Some(Watch::new(workspace, state.spatial.index(world).limit()));
        return world;
    }
    let watch = state.dom.watch.as_mut().expect("watched");
    if watch.moved.is_empty() && watch.written.is_empty() {
        crate::profiler::caches::hit(crate::profiler::caches::Cache::Queries);
        return world;
    }
    crate::profiler::caches::miss(crate::profiler::caches::Cache::Queries);
    let (mut moved, mut written) = watch.take();
    let mut removed = 0u64;
    // what moved first (a part written and then moved out is gone before its write is looked at). Each instance is looked at
    // once: a walk does not go into an instance an earlier walk reached, whose subtree that walk took whole with the same answer
    // (the tree stays as it is while the log is taken), so a chain of parts that moved costs the walk its length. (A log naming
    // the Workspace itself, given another class, walks each entry's subtree whole.)
    let once = !moved.contains(&workspace);
    let mut seen = std::mem::take(&mut state.spatial.seen);
    let mut walk = std::mem::take(&mut state.spatial.walk);
    let mut order = std::mem::take(&mut state.spatial.order);
    crate::dom::clear_set(&mut seen);
    for &id in &moved {
        if !seen.insert(id) {
            continue;
        }
        if state.dom.get(id).is_none() {
            removed += u64::from(take_out(state, &engine.reflection, world, id));
            continue;
        }
        let inside = state.dom.is_descendant_of_home(id, workspace);
        // (the subtree's parts each after their descendants, as a subtree comes in)
        post_order(&state.dom, id, once, &mut seen, &mut walk, &mut order, |_, _| true);
        for &node in &order {
            #[cfg(test)]
            SYNC_VISITS.with(|visits| visits.set(visits.get() + 1));
            let (class, indexed) = {
                let instance = state.dom.at(node);
                (instance.class, instance.flags & flags::INDEXED != 0)
            };
            if inside && node != workspace && engine.reflection.class(class).is_a(names.world_root) {
                state.spatial.nested.push(node);
            }
            if inside && is_part(engine, names, class) && state.dom.home(node) == Some(workspace) {
                put(state, engine, names, world, node);
            } else if indexed || state.spatial.holder(node).is_some_and(|(holder, _)| holder == world) {
                removed += u64::from(take_out(state, &engine.reflection, world, node));
            }
        }
    }
    for &id in &written {
        let Some(instance) = state.dom.get_mut(id) else { continue };
        instance.flags &= !flags::LOGGED;
        if state.spatial.holder(id).is_some_and(|(holder, _)| holder == world) {
            put(state, engine, names, world, id);
        }
    }
    state.spatial.seen = seen;
    state.spatial.walk = walk;
    state.spatial.order = order;
    crate::profiler::caches::evicted(crate::profiler::caches::Cache::Queries, removed);
    // the log's storage back to it
    moved.clear();
    written.clear();
    let limit = state.spatial.index(world).limit();
    let watch = state.dom.watch.as_mut().expect("watched");
    watch.limit = limit;
    if watch.moved.is_empty() {
        watch.moved = moved;
    }
    if watch.written.is_empty() {
        watch.written = written;
    }
    world
}

/// [`open`] for a WorldModel's world: its index, how many instances it looked at and how many parts a walk of its world filed.
fn open_model(state: &mut State, engine: &Engine, names: Names, model: InstanceId) -> (u32, usize, usize) {
    let made = state.spatial.world(model);
    let current = made.is_some_and(|world| state.spatial.index(world).read) && state.dom.world_log(model).is_some_and(|log| !log.overflowed);
    let world = match made {
        Some(world) => world,
        None => state.spatial.make(model),
    };
    if !current {
        let (visited, filed) = walk_world(state, engine, names, world, model);
        let limit = state.spatial.index(world).limit();
        state.dom.follow_world(model, limit);
        return (world, visited, filed);
    }
    let log = state.dom.world_log(model).expect("followed");
    if log.moved.is_empty() && log.written.is_empty() {
        return (world, 0, 0);
    }
    let mut looked = 0usize;
    let (mut moved, mut written) = log.take();
    // what moved first. An instance no longer in this world is taken out and not walked: the parts that left with it were logged
    // one by one as they left (dom.rs `log_world_leaving`), and it sits in the log of every world it passed through, so a walk
    // of what is below it now would be paid by every one of them.
    let mut seen = std::mem::take(&mut state.spatial.seen);
    let mut walk = std::mem::take(&mut state.spatial.walk);
    let mut order = std::mem::take(&mut state.spatial.order);
    crate::dom::clear_set(&mut seen);
    let world_root = names.world_root;
    for &id in &moved {
        if !seen.insert(id) {
            continue;
        }
        if state.dom.get(id).is_none() || state.dom.home(id) != Some(model) {
            looked += 1;
            take_out(state, &engine.reflection, world, id);
            continue;
        }
        // (a world inside it keeps its own parts)
        post_order(&state.dom, id, true, &mut seen, &mut walk, &mut order, |node, class| node == id || !engine.reflection.class(class).is_a(world_root));
        looked += order.len();
        for &node in &order {
            let class = state.dom.at(node).class;
            if is_part(engine, names, class) && state.dom.home(node) == Some(model) {
                put(state, engine, names, world, node);
            } else {
                take_out(state, &engine.reflection, world, node);
            }
        }
    }
    for &id in &written {
        if state.dom.get(id).is_some() && state.spatial.holder(id).is_some_and(|(holder, _)| holder == world) {
            put(state, engine, names, world, id);
        }
    }
    state.spatial.seen = seen;
    state.spatial.walk = walk;
    state.spatial.order = order;
    moved.clear();
    written.clear();
    let limit = state.spatial.index(world).limit();
    let log = state.dom.world_log(model).expect("followed");
    log.limit = limit;
    if log.moved.is_empty() {
        log.moved = moved;
    }
    if log.written.is_empty() {
        log.written = written;
    }
    #[cfg(test)]
    MODEL_SYNC_VISITS.with(|visits| visits.set(visits.get() + looked as u64));
    (world, looked, 0)
}

/// `id`'s subtree into `order`, each instance after its descendants (children in order), not going into an instance `seen`
/// already holds when `once` (marking those it reaches), nor below one `descend` refuses.
fn post_order(dom: &Dom, id: InstanceId, once: bool, seen: &mut FxHashSet<InstanceId>, walk: &mut Vec<InstanceId>, order: &mut Vec<InstanceId>, descend: impl Fn(InstanceId, ClassId) -> bool) {
    // (a stack walk taking children last first gives the reverse of each instance after its descendants)
    walk.clear();
    order.clear();
    walk.push(id);
    while let Some(node) = walk.pop() {
        order.push(node);
        let instance = dom.at(node);
        if !descend(node, instance.class) {
            continue;
        }
        if once {
            walk.extend(instance.children.iter().filter(|&&child| seen.insert(child)));
        } else {
            instance.children.copy_to(walk);
        }
    }
    order.reverse();
}

/// The index made (or made again, after its log overflowed) from a walk of its world: every part read and, unless it waits or is
/// filed, filed in the walk's order (each part after its descendants); entries whose parts are no longer in it taken out. A part
/// already filed or waiting keeps its place. How many instances it visited and how many parts it filed.
fn walk_world(state: &mut State, engine: &Engine, names: Names, world: u32, root: InstanceId) -> (usize, usize) {
    #[cfg(test)]
    if root == engine.world().workspace {
        WORKSPACE_WALKS.with(|walks| walks.set(walks.get() + 1));
    } else {
        MODEL_WALKS.with(|walks| walks.set(walks.get() + 1));
    }
    let mut walk = std::mem::take(&mut state.spatial.walk);
    let mut order = std::mem::take(&mut state.spatial.order);
    let mut seen = std::mem::take(&mut state.spatial.seen);
    crate::dom::clear_set(&mut seen);
    let world_root = names.world_root;
    post_order(&state.dom, root, false, &mut seen, &mut walk, &mut order, |node, class| node == root || !engine.reflection.class(class).is_a(world_root));
    // (the root itself, last, is no part of its world)
    order.pop();
    let visited = order.len();
    let mut found: FxHashSet<InstanceId> = FxHashSet::default();
    let mut filed = 0usize;
    let mut new_parts: Vec<InstanceId> = Vec::new();
    // (the WorldModels the walk meets inside the Workspace, for their indexes: open_workspace)
    let in_workspace = root == engine.world().workspace || state.dom.homes_above(root).top == Some(engine.world().workspace);
    for &node in &order {
        let class = state.dom.at(node).class;
        if in_workspace && node != root && engine.reflection.class(class).is_a(names.world_root) {
            state.spatial.nested.push(node);
        }
        if is_part(engine, names, class) && state.dom.home(node) == Some(root) {
            found.insert(node);
            if state.spatial.holder(node).is_some_and(|(holder, _)| holder == world) {
                put(state, engine, names, world, node);
            } else {
                new_parts.push(node);
            }
        }
    }
    // the parts not in the index yet, filed in the walk's order: each assembly as one unit where its first part comes
    if !new_parts.is_empty() {
        let joined = state.joints.created > 0;
        if joined {
            super::joints::ready(state, &engine.reflection);
        }
        let fresh: FxHashSet<InstanceId> = if joined { new_parts.iter().copied().collect() } else { FxHashSet::default() };
        let frame = state.sched.frame;
        for &part in &new_parts {
            if state.spatial.holder(part).is_some_and(|(holder, _)| holder == world) {
                continue;
            }
            let members: Vec<InstanceId> = if joined { super::joints::assembly_parts(state, &engine.reflection, part).iter().copied().filter(|member| fresh.contains(member) && !state.spatial.holder(*member).is_some_and(|(holder, _)| holder == world)).collect() } else { vec![part] };
            let mut entries = Vec::with_capacity(members.len());
            for &member in &members {
                let entry = entry_of(state, engine, &names, member);
                let serial = state.dom.at(member).serial;
                entries.push(state.spatial.add(world, member, entry, serial, frame));
                if root == engine.world().workspace {
                    state.dom.at_mut(member).flags |= flags::INDEXED;
                }
            }
            let unit = state.spatial.index_mut(world).new_unit(&entries);
            count_anchored(state, &names, world, unit);
            ready_unit(state, &engine.reflection, world, unit);
            let place = unit_place(state, &names, world, unit);
            state.spatial.index_mut(world).set_place(unit, place, true, frame);
            filed += entries.len();
        }
    }
    let stale: Vec<InstanceId> = {
        let index = state.spatial.index(world);
        index.live.iter().map(|&entry| index.parts[entry as usize]).filter(|part| !found.contains(part)).collect()
    };
    for part in stale {
        take_out(state, &engine.reflection, world, part);
    }
    state.spatial.index_mut(world).walked = visited;
    state.spatial.index_mut(world).read = true;
    state.spatial.walk = walk;
    state.spatial.order = order;
    state.spatial.seen = seen;
    (visited, filed)
}

/// A query of the world of `root` takes in the units that wait for it, in the order they came: each filed at the end of its
/// cells' lists. Its index (made first when it has none: none for a WorldModel outside the DataModel), how many parts it took in
/// and what opening the index cost.
pub(crate) fn take_in(engine: &Engine, root: InstanceId) -> (Option<u32>, usize, f64) {
    let (world, opened) = open(engine, root);
    let mut state = engine.state.borrow_mut();
    let taken = match world {
        Some(world) => take_in_world(&mut state, &engine.reflection, world),
        None => 0,
    };
    (world, taken, opened)
}

fn take_in_world(state: &mut State, reflection: &crate::Reflection, world: u32) -> usize {
    settle(state, reflection);
    let Some(names) = state.spatial.names else { return 0 };
    let frame = state.sched.frame;
    let mut wait = std::mem::take(&mut state.spatial.index_mut(world).wait);
    let count = state.spatial.index(world).waiting;
    for &unit in &wait {
        ready_unit(state, reflection, world, unit);
        let place = unit_place(state, &names, world, unit);
        let index = state.spatial.index_mut(world);
        index.units[unit as usize].at = NONE;
        index.set_place(unit, place, true, frame);
    }
    // (a table that a burst grew is made again for what it held, as dom.rs clear_set does)
    if wait.capacity() > 4 * wait.len().max(1024) {
        wait = Vec::new();
    } else {
        wait.clear();
    }
    let index = state.spatial.index_mut(world);
    index.wait = wait;
    index.waiting = 0;
    count
}

/// The physics step: every world's index takes in what waits for it, as a client's step does (the Workspace's index made at the
/// first step, a WorldModel's when its world has parts waiting).
pub fn index_parts(engine: &Engine) {
    let workspace = engine.world().workspace;
    let (world, _) = open(engine, workspace);
    if let Some(world) = world {
        take_in_world(&mut engine.state.borrow_mut(), &engine.reflection, world);
    }
    let mut models: Vec<InstanceId> = {
        let state = engine.state.borrow();
        let spatial = &state.spatial;
        spatial.by_root.iter().filter(|&(&root, &world)| root != workspace && !spatial.index(world).wait.is_empty()).map(|(&root, _)| root).collect()
    };
    models.sort_unstable();
    for model in models {
        if let (Some(world), _) = open(engine, model) {
            take_in_world(&mut engine.state.borrow_mut(), &engine.reflection, world);
        }
    }
}

/// A walk's parts, in its order, into `out` (each with its entry), and what it did; `first` the list a cell gives first
/// ([`grid::UNANCHORED`] for the box queries, [`grid::ANCHORED`] for the casts). The world box `lo`..`hi` is the query's,
/// clamped. The world's parts whose box is not finite come last.
pub(crate) fn walk_box(state: &mut State, world: u32, lo: Vec3, hi: Vec3, first: u8, out: &mut Vec<(InstanceId, Entry)>) -> Walked {
    let State { spatial, sched, .. } = state;
    let index = spatial.index_mut(world);
    let before = out.len();
    let slots = index.units.len();
    let WorldIndex { grid, entries: held, parts, units, loose, .. } = index;
    let give = |out: &mut Vec<(InstanceId, Entry)>, unit: u32| out.extend(units[unit as usize].parts().map(|entry| (parts[entry as usize], held[entry as usize])));
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut walked = None;
    #[cfg(test)]
    if scan() && grid::count_walk(lo, hi, grid.top(sched.frame)) <= 2_000_000 {
        let (order, reference) = grid.walk_box_reference(lo, hi, sched.frame, slots, first);
        for unit in order {
            give(out, unit);
        }
        walked = Some(reference);
    }
    let walked = walked.unwrap_or_else(|| {
        grid.walk_box(lo, hi, sched.frame, slots, first, |unit| {
            give(out, unit);
            true
        })
    });
    for &unit in loose.iter() {
        give(out, unit);
    }
    taken(out.len() - before);
    walked
}

/// [`walk_box`] a part at a time: `visit` is given each part of the walk with its entry until it returns false (world/region.rs:
/// ArePartsTouchingOthers' walk stops at the first part touching), the world's parts whose box is not finite last. What it did.
pub(crate) fn visit_box(state: &mut State, world: u32, lo: Vec3, hi: Vec3, first: u8, mut visit: impl FnMut(InstanceId, &Entry) -> bool) -> Walked {
    let State { spatial, sched, .. } = state;
    let index = spatial.index_mut(world);
    let slots = index.units.len();
    let WorldIndex { grid, entries: held, parts, units, loose, .. } = index;
    let mut going = true;
    let mut given = 0usize;
    let walked = grid.walk_box(lo, hi, sched.frame, slots, first, |unit| {
        for entry in units[unit as usize].parts() {
            given += 1;
            going = visit(parts[entry as usize], &held[entry as usize]);
            if !going {
                break;
            }
        }
        going
    });
    if going {
        'loose: for &unit in loose.iter() {
            for entry in units[unit as usize].parts() {
                given += 1;
                if !visit(parts[entry as usize], &held[entry as usize]) {
                    break 'loose;
                }
            }
        }
    }
    taken(given);
    walked
}

/// A ray walk's parts (Raycast, the PlayerMouse's ray, GetPartsObscuringTarget's and GetLargestCutoffDistance's), in its order,
/// into `out` with the index in `passed` of the first cell holding each (u32::MAX for a part whose box is not finite, last),
/// and the cells it passed into `passed`.
pub(crate) fn walk_ray(state: &mut State, world: u32, origin: Vec3, direction: Vec3, out: &mut Vec<(InstanceId, Entry, u32)>, passed: &mut Vec<Passed>) {
    let State { spatial, sched, .. } = state;
    let index = spatial.index_mut(world);
    let before = out.len();
    let slots = index.units.len();
    let WorldIndex { grid, entries: held, parts, units, loose, .. } = index;
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut found = Vec::new();
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut done = false;
    #[cfg(test)]
    if scan() && grid::count_ray(origin, direction, grid.top(sched.frame)) <= 2_000_000 {
        let (cells, order) = grid.walk_ray_reference(origin, direction, sched.frame, slots);
        *passed = cells;
        found = order;
        done = true;
    }
    if !done {
        grid.walk_ray(origin, direction, sched.frame, slots, passed, &mut found);
    }
    for (unit, cell) in found {
        out.extend(units[unit as usize].parts().map(|entry| (parts[entry as usize], held[entry as usize], cell)));
    }
    for &unit in loose.iter() {
        out.extend(units[unit as usize].parts().map(|entry| (parts[entry as usize], held[entry as usize], u32::MAX)));
    }
    taken(out.len() - before);
}

/// Every part of the world's index that is filed or loose (a waiting part is not), into `out`, in no particular order
pub(crate) fn all_parts(state: &State, world: u32, out: &mut Vec<(InstanceId, Entry)>) {
    let index = state.spatial.index(world);
    let before = out.len();
    out.extend(index.live.iter().filter(|&&entry| index.place_of(entry) != Place::Waits).map(|&entry| index.give(entry)));
    taken(out.len() - before);
}

/// Every part the Workspace's index holds, waiting ones too, into `out`, in no particular order: one step for each, however
/// many instances the Workspace holds beside them (a list of its own, not a query's take: [`TAKEN`] leaves them out)
pub(crate) fn part_ids(state: &State, world: u32, out: &mut Vec<InstanceId>) {
    let index = state.spatial.index(world);
    out.extend(index.live.iter().map(|&entry| index.parts[entry as usize]));
}

/// Every BasePart in the Workspace but Terrain, from the indexes of the Workspace's world and of the WorldModels inside it
/// (each brought up to the dom's changes), into `out` in no particular order: what a collision-group change works out the ids
/// of (services/physics.rs), without a walk of the Workspace's other instances.
pub(crate) fn placed_parts(engine: &Engine, out: &mut Vec<InstanceId>) {
    let workspace = engine.world().workspace;
    let Some(world) = open(engine, workspace).0 else { return };
    part_ids(&engine.state.borrow(), world, out);
    let mut models: Vec<InstanceId> = {
        let state = engine.state.borrow();
        state.spatial.by_root.keys().copied().filter(|&root| root != workspace && state.dom.get(root).is_some() && state.dom.homes_above(root).top == Some(workspace)).collect()
    };
    models.sort_unstable();
    for model in models {
        if let Some(world) = open(engine, model).0 {
            part_ids(&engine.state.borrow(), world, out);
        }
    }
}

/// How many indexes the engine holds (tests)
#[cfg(test)]
pub(crate) fn worlds(state: &State) -> usize {
    state.spatial.by_root.len()
}

/// How many parts the world's index holds (tests)
#[cfg(test)]
pub(crate) fn held(state: &State, world: u32) -> usize {
    state.spatial.index(world).len()
}

/// How many parts wait for the world of `root` (tests)
#[cfg(test)]
pub(crate) fn waiting(state: &State, root: InstanceId) -> usize {
    state.spatial.world(root).map_or(0, |world| state.spatial.index(world).waiting)
}

/// How many units the wait of the world of `root` has room for (tests: a burst's list is not kept)
#[cfg(test)]
pub(crate) fn wait_capacity(state: &State, root: InstanceId) -> usize {
    state.spatial.world(root).map_or(0, |world| state.spatial.index(world).wait.capacity())
}

/// Whether the world's index holds what a walk of its world finds: the same parts, each with the entry the dom gives it now, in a
/// unit filed as its parts file it now or waiting; its units, wait, loose list and grid sound (tests)
#[cfg(test)]
pub(crate) fn check_world(engine: &Engine, root: InstanceId) -> Result<(), String> {
    let world = open(engine, root).0.ok_or("no index")?;
    {
        // (each unit one rigid assembly: its parts those of their assembly in this world, as world/joints.rs works it out)
        let mut state = engine.state.borrow_mut();
        let state = &mut *state;
        if state.joints.created > 0 {
            let units: Vec<(u32, Vec<InstanceId>)> = {
                let index = state.spatial.index(world);
                index.units.iter().enumerate().filter(|(_, held)| held.first != NONE).map(|(unit, held)| (unit as u32, held.parts().map(|entry| index.parts[entry as usize]).collect())).collect()
            };
            for (unit, parts) in units {
                let members = super::joints::assembly_parts(state, &engine.reflection, parts[0]);
                let here = members.iter().filter(|&&member| state.spatial.holder(member).is_some_and(|(holder, _)| holder == world)).count();
                for &part in &parts {
                    if super::joints::assembly_parts(state, &engine.reflection, part)[0] != members[0] {
                        let names = state.spatial.names.expect("names");
                        let described: Vec<String> = parts
                            .iter()
                            .map(|&p| {
                                let anchored = names.anchored.and_then(|n| state.dom.boolean(p, n)) == Some(true);
                                let root = super::joints::assembly_parts(state, &engine.reflection, p)[0];
                                let name = |id: InstanceId| String::from_utf8_lossy(state.dom.at(id).name.as_bytes()).to_string();
                                let links: Vec<String> = super::joints::part_links(state, p).iter().map(|(j, o, _)| format!("{}->{}", name(*j), name(*o))).collect();
                                format!("{}{} root {} links [{}]", name(p), if anchored { "A" } else { "" }, name(root), links.join(" "))
                            })
                            .collect();
                        return Err(format!("unit {unit} holds parts of two assemblies: {}", described.join("; ")));
                    }
                }
                if here != parts.len() {
                    return Err(format!("unit {unit} holds {} of its assembly's {here} parts in its world: {parts:?}, assembly {members:?}", parts.len()));
                }
            }
        }
    }
    let state = engine.state.borrow();
    let names = state.spatial.names.expect("names");
    let index = state.spatial.index(world);
    let mut walked: Vec<String> = state
        .dom
        .descendants(root)
        .into_iter()
        .filter(|&id| is_part(engine, names, state.dom.at(id).class) && state.dom.home(id) == Some(root))
        .map(|id| {
            let entry = entry_of(&state, engine, &names, id);
            format!("{id:?} {:?} {} {}", entry.solid, entry.can_query, entry.can_collide)
        })
        .collect();
    let mut kept: Vec<String> = index
        .live
        .iter()
        .map(|&entry| {
            let (id, held) = index.give(entry);
            // (a waiting part's entry is read at the next query: the check opened the index, which read it)
            format!("{id:?} {:?} {} {}", held.solid, held.can_query, held.can_collide)
        })
        .collect();
    crate::sort::ord(&mut walked);
    crate::sort::ord(&mut kept);
    if walked != kept {
        let missing: Vec<&String> = walked.iter().filter(|line| !kept.contains(line)).take(3).collect();
        let extra: Vec<&String> = kept.iter().filter(|line| !walked.contains(line)).take(3).collect();
        return Err(format!("{} walked, {} held; missing {missing:?}; extra {extra:?}", walked.len(), kept.len()));
    }
    // each live entry in exactly one live unit, each live unit's parts live entries
    let mut units_seen: FxHashMap<u32, usize> = FxHashMap::default();
    for &entry in &index.live {
        let unit = index.unit_of[entry as usize];
        if unit == NONE || index.units.get(unit as usize).is_none_or(|held| held.first == NONE) {
            return Err(format!("entry {entry} in no unit"));
        }
        if !index.units[unit as usize].parts().any(|e| e == entry) {
            return Err(format!("entry {entry} not in its unit {unit}"));
        }
        *units_seen.entry(unit).or_default() += 1;
        if state.spatial.holder(index.parts[entry as usize]) != Some((world, entry)) {
            return Err(format!("entry {entry}'s slot"));
        }
    }
    let mut filed: Vec<(u32, Filing)> = Vec::new();
    let mut waiting = 0usize;
    for (&unit, &count) in &units_seen {
        let held = &index.units[unit as usize];
        if held.len() != count {
            return Err(format!("unit {unit} holds {} parts, {count} live", held.len()));
        }
        match held.place {
            Place::Filed(filing) => {
                filed.push((unit, filing));
                let now = unit_place(&state, &names, world, unit);
                if now != held.place {
                    return Err(format!("unit {unit} filed as {:?}, its parts file it as {now:?}", held.place));
                }
            }
            Place::Loose => {
                if index.loose.get(held.at as usize) != Some(&unit) {
                    return Err(format!("unit {unit} not at its place in the loose list"));
                }
            }
            Place::Waits => {
                waiting += count;
                if index.wait.get(held.at as usize) != Some(&unit) {
                    return Err(format!("unit {unit} not at its place in the wait"));
                }
            }
        }
    }
    if waiting != index.waiting || index.wait.iter().map(|&unit| index.units[unit as usize].len()).sum::<usize>() != waiting {
        return Err(format!("{} waiting counted, {waiting} in waiting units", index.waiting));
    }
    if index.loose.iter().any(|&unit| index.units[unit as usize].place != Place::Loose) || index.wait.iter().any(|&unit| index.units[unit as usize].place != Place::Waits) {
        return Err("a list holds a unit placed elsewhere".to_string());
    }
    index.grid.check(&filed)?;
    Ok(())
}

/// Puts `items` in the order the Workspace's descendants are listed in ([`Dom::descendants`] from `root`), each item's part
/// read by `part_of`, and says how many steps up the tree its walks took (from an instance to its parent, `root` aside: what
/// [`order_cost`] prices). Nothing to do for one item or none.
///
/// The order is that of each part's places among its siblings (dom.rs `sibling_place`) from below `root` down to the part, compared as
/// lists: an ancestor's is a prefix of its descendants', so it comes first, and items with equal lists keep their own order (the
/// order of a stable sort of those lists). The lists are not made: they are the paths of a tree of their own ([`Trie`]), which
/// walks up from every part at once build, a walk ending where it meets another's, at `root` or at the top, and all of them
/// once a single walk is left with every part below it (what lies above it is the same for every part), and which is then read
/// in preorder, each node's items first and its children by number. So the parts' paths are looked at only up to where they
/// meet: 10,000 parts in a folder at the bottom of a 10,000-deep chain take one step each, where comparing whole lists held a
/// key of 10,000 numbers for each part (2 GB) and compared them for 0.7 s. Parts whose paths meet only far up (each at the
/// bottom of a chain of its own, or one part in the Workspace beside one deep down) still take a step for each instance
/// between them and that meeting, which the caller charges ([`order_cost`]).
pub fn sort_by_order<T>(dom: &Dom, root: InstanceId, items: &mut Vec<T>, part_of: impl Fn(&T) -> InstanceId) -> usize {
    if items.len() < 2 {
        return 0;
    }
    let (order, steps) = MARKS.with(|marks| {
        let mut marks = marks.borrow_mut();
        let (call, marks) = &mut *marks;
        *call = call.wrapping_add(1);
        if *call == 0 {
            marks.clear();
            *call = 1;
        }
        let (trie, steps) = Trie::climbed(dom, root, items.iter().map(&part_of), marks, *call);
        match trie.order() {
            Some(order) => (order, steps),
            // (two instances at one place in the trie with the same number: the trie that keeps one node for both)
            None => {
                #[cfg(test)]
                KEYED.with(|keyed| keyed.set(keyed.get() + 1));
                let (trie, steps) = Trie::keyed(dom, root, items.iter().map(&part_of));
                (trie.order().expect("one node for each list"), steps)
            }
        }
    });
    let mut taken: Vec<Option<T>> = std::mem::take(items).into_iter().map(Some).collect();
    items.extend(order.into_iter().map(|at| taken[at].take().expect("each once")));
    steps
}

/// What the walk [`sort_by_order`] took `steps` for, ordering `items` items, costs the work clock: each step past
/// [`crate::cost::QUERY_ORDER_FREE`] for each item costs [`crate::cost::QUERY_ORDER_STEP`].
pub fn order_cost(steps: usize, items: usize) -> f64 {
    steps.saturating_sub(items.saturating_mul(crate::cost::QUERY_ORDER_FREE)) as f64 * crate::cost::QUERY_ORDER_STEP
}

/// What [`sort_by_order`] marks on the instances its walks reach, by dom slot: the call that marked the slot (each call takes the
/// next number, so nothing is cleared between calls), the instance, and its node in that call's [`Trie`]
#[derive(Clone, Copy)]
struct Mark {
    call: u32,
    id: InstanceId,
    node: u32,
}

thread_local! {
    /// [`sort_by_order`]'s marks and the last call's number (the engines of a thread share them: a call reads only its own; they
    /// grow to the highest slot a walk reached, 16 bytes a slot)
    static MARKS: std::cell::RefCell<(u32, Vec<Mark>)> = const { std::cell::RefCell::new((0, Vec::new())) };
}

/// The lists [`sort_by_order`] orders, as a trie: one node for each distinct list, below the node of the list one shorter, with the
/// list's last number; node 0 is the empty list (`root` itself, and above the top of a part that is not below `root`)
struct Trie {
    parent: Vec<u32>,
    number: Vec<u64>,
    /// Each item's node
    node_of: Vec<u32>,
}

impl Trie {
    fn new(items: usize) -> Trie {
        Trie { parent: vec![u32::MAX], number: vec![0], node_of: Vec::with_capacity(items) }
    }

    fn add(&mut self, parent: u32, number: u64) -> u32 {
        self.parent.push(parent);
        self.number.push(number);
        (self.parent.len() - 1) as u32
    }

    /// The trie of `parts`' lists, made from below, and how many steps up the tree the walks took. Each distinct part
    /// gets a node and a mark, then all of them walk up a step at a time: a step onto `root` or past the top ends a walk (its node
    /// a child of node 0), one onto a marked instance ends it there (its node that one's child), and one onto any other instance
    /// gives that instance a node and a mark and goes on from it. A single walk left before any has ended has every part below it,
    /// and what lies above it is the same for all of them, so it ends at once (its node a child of node 0). Two instances with one
    /// parent and the same number get a node each here ([`Trie::order`] then declines).
    fn climbed(dom: &Dom, root: InstanceId, parts: impl Iterator<Item = InstanceId>, marks: &mut Vec<Mark>, call: u32) -> (Trie, usize) {
        let mut trie = Trie::new(parts.size_hint().0);
        let marked = |marks: &Vec<Mark>, id: InstanceId| marks.get(id.index as usize).filter(|mark| mark.call == call && mark.id == id).map(|mark| mark.node);
        let mark = |marks: &mut Vec<Mark>, id: InstanceId, node: u32| {
            let slot = id.index as usize;
            if marks.len() <= slot {
                marks.resize(slot + 1, Mark { call: 0, id, node: 0 });
            }
            marks[slot] = Mark { call, id, node };
        };
        // (each walk: where it is, and that instance's node)
        let mut walks: Vec<(InstanceId, u32)> = Vec::new();
        for part in parts {
            if part == root {
                trie.node_of.push(0);
                continue;
            }
            let node = match marked(marks, part) {
                Some(node) => node,
                None => {
                    let node = trie.add(u32::MAX, u64::from(dom.sibling_place(part)));
                    mark(marks, part, node);
                    walks.push((part, node));
                    node
                }
            };
            trie.node_of.push(node);
        }
        let (mut ended, mut steps) = (false, 0usize);
        let mut next: Vec<(InstanceId, u32)> = Vec::new();
        while !walks.is_empty() {
            if walks.len() == 1 && !ended {
                trie.parent[walks[0].1 as usize] = 0;
                break;
            }
            next.clear();
            for &(at, node) in &walks {
                let Some(up) = dom.get(at).and_then(|instance| instance.parent).filter(|&up| up != root) else {
                    trie.parent[node as usize] = 0;
                    ended = true;
                    continue;
                };
                steps += 1;
                match marked(marks, up) {
                    Some(known) => trie.parent[node as usize] = known,
                    None => {
                        let above = trie.add(u32::MAX, u64::from(dom.sibling_place(up)));
                        mark(marks, up, above);
                        trie.parent[node as usize] = above;
                        next.push((up, above));
                    }
                }
            }
            std::mem::swap(&mut walks, &mut next);
        }
        (trie, steps)
    }

    /// The trie of `parts`' lists with one node for each distinct list however many instances share it (a hash of each node's
    /// children by number, and of the instances reached): a walk from each part up to `root`, the top, or an instance an earlier
    /// walk reached, then down again. What [`Trie::climbed`]'s trie declines to order; its steps counted the same way.
    fn keyed(dom: &Dom, root: InstanceId, parts: impl Iterator<Item = InstanceId>) -> (Trie, usize) {
        let mut trie = Trie::new(parts.size_hint().0);
        let mut by_instance: FxHashMap<InstanceId, u32> = FxHashMap::default();
        let mut by_key: FxHashMap<(u32, u64), u32> = FxHashMap::default();
        let mut chain: Vec<InstanceId> = Vec::new();
        let mut steps = 0usize;
        for part in parts {
            chain.clear();
            let mut at = 0u32;
            let mut node = Some(part);
            while let Some(id) = node {
                if id == root {
                    break;
                }
                if let Some(&known) = by_instance.get(&id) {
                    at = known;
                    break;
                }
                chain.push(id);
                node = dom.get(id).and_then(|instance| instance.parent);
                steps += usize::from(node.is_some_and(|up| up != root));
            }
            for &id in chain.iter().rev() {
                let key = (at, u64::from(dom.sibling_place(id)));
                at = match by_key.get(&key) {
                    Some(&node) => node,
                    None => {
                        let node = trie.add(key.0, key.1);
                        by_key.insert(key, node);
                        node
                    }
                };
                by_instance.insert(id, at);
            }
            trie.node_of.push(at);
        }
        (trie, steps)
    }

    /// The items' order, a preorder of the trie with each node's items first (in their own order) and then its children by
    /// number; none when two children of a node have the same number.
    fn order(&self) -> Option<Vec<usize>> {
        let nodes = self.parent.len();
        // each node's children (in the order they were made, then by number) and items, as ranges of two lists
        let mut first_child = vec![0u32; nodes + 1];
        for &parent in &self.parent[1..] {
            first_child[parent as usize + 1] += 1;
        }
        let mut first_item = vec![0u32; nodes + 1];
        for &node in &self.node_of {
            first_item[node as usize + 1] += 1;
        }
        for at in 0..nodes {
            first_child[at + 1] += first_child[at];
            first_item[at + 1] += first_item[at];
        }
        let mut children = vec![0u32; nodes - 1];
        let mut placed = first_child.clone();
        for (child, &parent) in self.parent.iter().enumerate().skip(1) {
            children[placed[parent as usize] as usize] = child as u32;
            placed[parent as usize] += 1;
        }
        for at in 0..nodes {
            let siblings = &mut children[first_child[at] as usize..first_child[at + 1] as usize];
            if siblings.len() > 1 {
                siblings.sort_unstable_by_key(|&child| self.number[child as usize]);
                if siblings.windows(2).any(|pair| self.number[pair[0] as usize] == self.number[pair[1] as usize]) {
                    return None;
                }
            }
        }
        let mut item_at = vec![0u32; self.node_of.len()];
        let mut placed = first_item.clone();
        for (index, &node) in self.node_of.iter().enumerate() {
            item_at[placed[node as usize] as usize] = index as u32;
            placed[node as usize] += 1;
        }
        // (over a stack of its own: the trie is as deep as the tree)
        let mut order: Vec<usize> = Vec::with_capacity(self.node_of.len());
        let mut stack: Vec<u32> = vec![0];
        while let Some(node) = stack.pop() {
            let node = node as usize;
            order.extend(item_at[first_item[node] as usize..first_item[node + 1] as usize].iter().map(|&index| index as usize));
            stack.extend(children[first_child[node] as usize..first_child[node + 1] as usize].iter().rev());
        }
        Some(order)
    }
}

/// [`sort_by_order`] as it was first written, each item's list of numbers made and the lists compared (tests: the order it
/// must keep)
#[cfg(test)]
pub(crate) fn sort_by_order_lists<T>(dom: &Dom, root: InstanceId, items: &mut Vec<T>, part_of: impl Fn(&T) -> InstanceId) {
    if items.len() < 2 {
        return;
    }
    let mut keys: Vec<u64> = Vec::with_capacity(items.len() * 4);
    let mut spans: Vec<(u32, u32)> = Vec::with_capacity(items.len());
    for item in items.iter() {
        let start = keys.len();
        let mut node = Some(part_of(item));
        while let Some(id) = node {
            if id == root {
                break;
            }
            keys.push(u64::from(dom.sibling_place(id)));
            node = dom.get(id).and_then(|instance| instance.parent);
        }
        keys[start..].reverse();
        spans.push((start as u32, (keys.len() - start) as u32));
    }
    let key = |at: usize| {
        let (start, len) = spans[at];
        &keys[start as usize..(start + len) as usize]
    };
    let mut order: Vec<usize> = (0..items.len()).collect();
    crate::sort::by(&mut order, |&a, &b| key(a).cmp(key(b)));
    let mut taken: Vec<Option<T>> = std::mem::take(items).into_iter().map(Some).collect();
    items.extend(order.into_iter().map(|at| taken[at].take().expect("each once")));
}
