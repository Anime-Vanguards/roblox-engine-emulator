//! The spatial queries' broadphase: each world's parts filed in a grid of its own, as a client files them (world/grid.rs, the
//! grid; docs/engine.md "The spatial grid"). A world is the Workspace's (its BaseParts but Terrain, not those of a WorldModel
//! in it) or a WorldModel's in the DataModel (the BaseParts whose nearest WorldRoot it is): a client keeps one grid each, and
//! a WorldRoot's queries walk its own (measured: a WorldModel's parts are never found by the Workspace's queries, and a
//! WorldModel's grid follows the same rules). The indexes live in the engine's state ([`Spatial`], `State::spatial`) so that
//! every write can file a part at once.
//!
//! - **Coming in**: a part that enters a world waits (camera.rs `Cameras::unindexed`, `unindexed_models`: [`Waiting`], in the
//!   order the parts came, a moved subtree's parts as a client takes the subtree up: the moved part first, then the rest each
//!   after its own descendants, world/joints.rs taken_up) until a query of that world or the physics step takes the waiting
//!   parts in ([`take_in`], [`index_parts`]); each is then filed at the end of its cells' lists.
//! - **Going out** (destroyed, parented out of the world, into another): it leaves its cells at once ([`left`], from the
//!   hierarchy hook in camera_moves.rs, a moved subtree's parts in the order they come in), the list's last taking its place.
//! - **Every write files at once** ([`stored`], from instance.rs store): a write of CFrame, Size, Anchored, CanCollide or
//!   CanTouch of a filed part works out its filing again (float32 box arithmetic) and, when it changed, files it: at the end of
//!   every cell when its level, its list or CanCollide-and-CanTouch-both-off changed, else out of the cells it left and at
//!   the end of those it entered, keeping its place in the others. So a part moved out of its cell and back before a query is
//!   last there, as on a client (measured: tools/parity/probes/grid_order.luau).
//! - What a query reads of a part beyond its filing (its solid, CanQuery, CanCollide: [`Entry`]) is read again from the dom's
//!   change logs at the next query of its world (dom.rs `Watch` for the Workspace, `WorldLog` for a WorldModel), which also catch
//!   what came or went other than through a Parent write (a project mount, a replaced child, a class change): each logged
//!   instance is looked at once however often it changed, so a query after a change does work for what changed. A world's
//!   index is made from a walk of it at its first query or physics step (parts that do not wait filed in the walk's order, each
//!   after its descendants) and walked again only after its log overflowed (twice the parts it holds, at least 4,096 entries),
//!   which never moves a part already filed.
//! - A part whose box is not finite (a NaN or infinite position or size) is in no cell: every walk of its world gives it last
//!   (a list of its own, which it leaves in a step).
//!
//! A collision-group change lists the Workspace's parts from here ([`part_ids`], services/physics.rs), and puts the changed
//! ones in the Workspace's order ([`sort_by_order`]).

use glam::{Affine3A, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    frame,
    grid::{self, Filing, Grid, Passed, Walked},
    query::Solid,
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
    /// CanQuery is not false
    pub can_query: bool,
    /// CanCollide is not false
    pub can_collide: bool,
}

/// Where an entry is: waiting to be taken in, filed in the grid, or in the loose list (its box not finite)
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    Waits,
    Filed(Filing),
    Loose,
}

/// The parts waiting for a world's spatial index, in the order they came: a set, each part once (a part that leaves drops out,
/// and one that comes back is at the end), so it holds at most the world's parts and a lookup costs the same whatever it holds
#[derive(Default, Debug)]
pub struct Waiting {
    order: Vec<InstanceId>,
    at: FxHashMap<InstanceId, u32>,
}

impl Waiting {
    pub fn insert(&mut self, part: InstanceId) {
        if self.at.contains_key(&part) {
            return;
        }
        if self.order.len() >= 64 && self.order.len() > 2 * self.at.len() {
            self.compact();
        }
        self.at.insert(part, self.order.len() as u32);
        self.order.push(part);
    }

    pub fn remove(&mut self, part: &InstanceId) {
        self.at.remove(part);
    }

    pub fn contains(&self, part: &InstanceId) -> bool {
        !self.at.is_empty() && self.at.contains_key(part)
    }

    pub fn len(&self) -> usize {
        self.at.len()
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// How many parts its table has room for (tests: a burst's table is not kept)
    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.at.capacity()
    }

    pub fn clear(&mut self) {
        self.order.clear();
        // (a table that a burst grew is made again for what it held, as dom.rs clear_set does)
        if self.at.capacity() > 4 * self.at.len().max(1024) {
            self.at = FxHashMap::default();
        } else {
            self.at.clear();
        }
    }

    /// The parts in the order they came
    pub fn iter(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.order.iter().enumerate().filter(|(place, part)| self.at.get(part) == Some(&(*place as u32))).map(|(_, &part)| part)
    }

    fn compact(&mut self) {
        let kept: Vec<InstanceId> = self.iter().collect();
        self.order = kept;
        for (place, part) in self.order.iter().enumerate() {
            self.at.insert(*part, place as u32);
        }
    }
}

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

/// One world's index: its grid and its parts' entries
pub struct WorldIndex {
    /// Its WorldRoot (the Workspace, or a WorldModel)
    root: InstanceId,
    grid: Grid,
    entries: Vec<Entry>,
    places: Vec<Place>,
    /// Each entry's part (stale for a free entry)
    parts: Vec<InstanceId>,
    free: Vec<u32>,
    /// The entries in use, in no particular order, and each one's place in that list ([`part_ids`] reads the parts held without
    /// the free entries)
    live: Vec<u32>,
    live_at: Vec<u32>,
    /// The entries whose box is not finite, and each one's place in that list (by entry), so one comes out in a step: the last
    /// takes its place. A walk gives them last, in this order
    loose: Vec<u32>,
    loose_at: Vec<u32>,
    /// How many instances the last walk of its world visited (its log's limit)
    walked: usize,
}

impl WorldIndex {
    fn new(root: InstanceId) -> Self {
        WorldIndex {
            root,
            grid: Grid::default(),
            entries: Vec::new(),
            places: Vec::new(),
            parts: Vec::new(),
            free: Vec::new(),
            live: Vec::new(),
            live_at: Vec::new(),
            loose: Vec::new(),
            loose_at: Vec::new(),
            walked: 0,
        }
    }

    /// How many parts it holds
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
}

/// The spatial indexes of an engine (`State::spatial`): the Workspace's and each WorldModel's, and which holds each part
#[derive(Default)]
pub struct Spatial {
    names: Option<Names>,
    /// Which properties file a part again when written (by name id: CFrame, Size, Anchored, CanCollide, CanTouch)
    watched: Vec<bool>,
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
    /// The links that started since [`links_started`] last took them (world/joints.rs relink), each with its two parts
    links: Vec<(InstanceId, InstanceId)>,
    /// The parts the Parent write going on put in a wait (camera_moves.rs's hierarchy hook): the links it starts leave them where
    /// they came in
    fresh: FxHashSet<InstanceId>,
    /// When each part came into the Workspace's world through a Parent write, by a number that grows ([`entered`]); a part that
    /// leaves it is forgotten, and the parts there before any write (a mounted place's) have none
    entered: FxHashMap<InstanceId, u64>,
    entries: u64,
}

impl Spatial {
    /// A link started between `first` and `second` (world/joints.rs relink): they are filed again at the write's end
    /// ([`links_started`]).
    pub(crate) fn link_started(&mut self, first: InstanceId, second: InstanceId) {
        if self.names.is_some() {
            self.links.push((first, second));
        }
    }
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
    for name in [Some(cframe), Some(K::Size as NameId), names.anchored, names.can_collide, names.can_touch].into_iter().flatten() {
        if spatial.watched.len() <= name as usize {
            spatial.watched.resize(name as usize + 1, false);
        }
        spatial.watched[name as usize] = true;
    }
    spatial.names = Some(names);
}

fn is_part(engine: &Engine, names: Names, class: ClassId) -> bool {
    let info = engine.reflection.class(class);
    info.is_a(names.base_part) && !info.is_a(names.terrain)
}

/// The filing a part's properties give it now: its frame and Size, its list (`list`, else its assembly's: [`list_of`]), CanCollide
/// and CanTouch both off. None when its box is not finite.
fn filing_of(state: &State, names: &Names, part: InstanceId, list: Option<u8>) -> Option<Filing> {
    let dom = &state.dom;
    let transform = frame::affine_of(dom.raw_get(part, names.cframe));
    let size = match dom.raw_get(part, K::Size as NameId) {
        Some(Value::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
        _ => Vec3::ONE,
    };
    let flag = |name: Option<NameId>| name.and_then(|n| dom.boolean(part, n));
    let list = list.unwrap_or_else(|| list_of(state, names, part));
    let off = flag(names.can_collide) == Some(false) && flag(names.can_touch) == Some(false);
    Filing::of(&transform, size, list, off)
}

/// The list a part is filed in: its assembly's (measured: an unanchored part welded to an anchored one is in the anchored list,
/// made so or welded later), read from its kept component (world/joints.rs grid_anchored), else its own Anchored.
fn list_of(state: &State, names: &Names, part: InstanceId) -> u8 {
    let own = names.anchored.and_then(|n| state.dom.boolean(part, n)) == Some(true);
    if own || super::joints::grid_anchored(state, part) == Some(true) { grid::ANCHORED } else { grid::UNANCHORED }
}

/// The list `entry` is filed in now, if it is filed
fn held_list(spatial: &Spatial, world: u32, entry: u32) -> Option<u8> {
    match spatial.index(world).places[entry as usize] {
        Place::Filed(filing) => Some(filing.list),
        _ => None,
    }
}

/// A part's entry, read from the dom now
fn entry_of(state: &State, engine: &Engine, names: &Names, part: InstanceId) -> Entry {
    let solid = super::query::solid(state, engine, part);
    let not_false = |name: Option<NameId>| name.and_then(|n| state.dom.boolean(part, n)) != Some(false);
    Entry { solid, can_query: not_false(names.can_query), can_collide: not_false(names.can_collide) }
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

    /// Drops the index of the WorldRoot `root` (a WorldModel freed or out of the DataModel), its parts' entries with it.
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

    /// A new entry in `world` for `part` (taken out of any other index first), with its place
    fn add(&mut self, world: u32, part: InstanceId, entry: Entry, place: Place, frame: u64) -> u32 {
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
                index.places[at as usize] = Place::Waits;
                at
            }
            None => {
                index.entries.push(entry);
                index.parts.push(part);
                index.places.push(Place::Waits);
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
        self.set_place(world, at, place, frame);
        at
    }

    /// Takes `entry` out of `world`'s index: out of its cells or the loose list, its entry freed.
    fn remove(&mut self, world: u32, entry: u32, frame: u64) {
        self.set_place(world, entry, Place::Waits, frame);
        let index = self.index_mut(world);
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

    /// Moves `entry` from its place to `place`: out of its cells (the lists' last taking its place) or the loose list, into the
    /// new cells at their ends; a filing of the same level, list and flags moves only the cells that changed.
    fn set_place(&mut self, world: u32, entry: u32, place: Place, frame: u64) {
        let index = self.index_mut(world);
        let old = index.places[entry as usize];
        if old == place {
            return;
        }
        match (old, place) {
            (Place::Filed(a), Place::Filed(b)) if a.level == b.level && a.list == b.list && a.off == b.off => index.grid.move_cells(entry, &a, &b, frame),
            _ => {
                match old {
                    Place::Filed(filing) => index.grid.unfile(entry, &filing, frame),
                    Place::Loose => take_loose(&mut index.loose, &mut index.loose_at, entry),
                    Place::Waits => {}
                }
                match place {
                    Place::Filed(filing) => index.grid.file(entry, &filing, frame),
                    Place::Loose => push_loose(&mut index.loose, &mut index.loose_at, entry),
                    Place::Waits => {}
                }
            }
        }
        index.places[entry as usize] = place;
    }
}

/// Where a part not waiting goes: its filing, or the loose list
fn placed(filing: Option<Filing>) -> Place {
    filing.map_or(Place::Loose, Place::Filed)
}

/// A property of `id` was stored (instance.rs store: every write, signalled or not). A write of CFrame, Size, Anchored,
/// CanCollide or CanTouch of a part an index holds files it at once.
#[inline]
pub fn stored(state: &mut State, reflection: &crate::Reflection, id: InstanceId, name: NameId) {
    if !state.spatial.watched.get(name as usize).copied().unwrap_or(false) {
        return;
    }
    links_started(state, reflection);
    let anchored = state.spatial.names.is_some_and(|names| names.anchored == Some(name));
    refile(state, id, anchored);
}

/// `part` filed again as its properties stand now, if an index holds it and it does not wait: in its list as filed, or with
/// `relist` (an Anchored write) its assembly's now; one whose Anchored write anchored or freed its whole kept component files the
/// component's other parts again too, in their new list (measured: an unanchored pair welded, one of them anchored, both go to
/// the end of the anchored list, that one first).
fn refile(state: &mut State, part: InstanceId, relist: bool) {
    let Some(names) = state.spatial.names else { return };
    let Some((world, entry)) = state.spatial.holder(part) else { return };
    if state.spatial.index(world).places[entry as usize] == Place::Waits {
        return;
    }
    let list = if relist { None } else { held_list(&state.spatial, world, entry) };
    let place = placed(filing_of(state, &names, part, list));
    let before = held_list(&state.spatial, world, entry);
    let frame = state.sched.frame;
    state.spatial.set_place(world, entry, place, frame);
    let after = held_list(&state.spatial, world, entry);
    if relist && before != after && super::joints::grid_anchored(state, part).is_some() {
        for other in super::joints::grid_component(state, part).unwrap_or_default() {
            if other == part {
                continue;
            }
            let Some((world, entry)) = state.spatial.holder(other) else { continue };
            if held_list(&state.spatial, world, entry).is_some_and(|list| list != list_of(state, &names, other)) {
                let place = placed(filing_of(state, &names, other, None));
                state.spatial.set_place(world, entry, place, frame);
            }
        }
    }
}

/// The Parent write going on starts putting parts in waits (camera_moves.rs's hierarchy hook): what an earlier write left fresh
/// is fresh no more.
pub(crate) fn fresh_write(state: &mut State) {
    if !state.spatial.fresh.is_empty() {
        crate::dom::clear_set(&mut state.spatial.fresh);
    }
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

/// `part` came into a wait in the Parent write going on (camera_moves.rs).
pub(crate) fn came_in(state: &mut State, part: InstanceId) {
    if state.joints.created > 0 {
        state.spatial.fresh.insert(part);
    }
}

/// The links that started (world/joints.rs relink, taken at the end of the joints' work on a write: instance.rs) file their two
/// parts again, the one their assembly's root would be first (world/joints.rs root_first), unless both are anchored: a part
/// waiting goes to the end of its wait, one filed to the end of its cells' lists, in its assembly's list (measured, Studio 0.747,
/// a playing client and a server alike: two unanchored parts welded before the take-in come in as their root and then the
/// other, whichever came in first, the larger first; welded after it, they leave and come at the end, root first; an
/// unanchored part welded to an anchored one is filed after it in the anchored list; a far unanchored part welded to an
/// anchored one files that one again at the end; two anchored parts welded keep their places). The parts the same Parent write
/// put in a wait keep their places there (unmeasured: a model coming in with its welds).
pub(crate) fn links_started(state: &mut State, reflection: &crate::Reflection) {
    if state.spatial.links.is_empty() {
        return;
    }
    let Some(names) = state.spatial.names else { return };
    let links = std::mem::take(&mut state.spatial.links);
    let anchored = |state: &State, part: InstanceId| names.anchored.and_then(|n| state.dom.boolean(part, n)) == Some(true);
    for &(a, b) in &links {
        if state.dom.get(a).is_none() || state.dom.get(b).is_none() || (anchored(state, a) && anchored(state, b)) {
            continue;
        }
        let pair = if super::joints::root_first(state, reflection, a, b) { [a, b] } else { [b, a] };
        let pair = pair.map(|part| (!state.spatial.fresh.contains(&part)).then_some(part));
        // (both out first, then both in, root first: measured, a pair welded after the take-in in the cell list u2 u4 u5 comes
        // to u5 u4 u2)
        let taken = pair.map(|part| part.and_then(|part| take_for_filing(state, part)));
        for (part, taken) in pair.into_iter().zip(taken) {
            if let (Some(part), Some(taken)) = (part, taken) {
                file_back(state, &names, part, taken);
            }
        }
    }
    let mut links = links;
    links.clear();
    state.spatial.links = links;
    fresh_write(state);
}

/// Where [`take_for_filing`] took a part from: its world's wait (by the WorldModel's id, None for the Workspace's) or its index
enum Taken {
    Wait(Option<InstanceId>),
    Cells(u32, u32),
}

/// `part` out of its wait if it waits, else out of its cells' lists, for [`file_back`]; None when it is in neither.
fn take_for_filing(state: &mut State, part: InstanceId) -> Option<Taken> {
    if state.cameras.unindexed.contains(&part) {
        state.cameras.unindexed.remove(&part);
        return Some(Taken::Wait(None));
    }
    let home = state.dom.home(part)?;
    if let Some(waiting) = state.cameras.unindexed_models.get_mut(&home).filter(|waiting| waiting.contains(&part)) {
        waiting.remove(&part);
        return Some(Taken::Wait(Some(home)));
    }
    let (world, entry) = state.spatial.holder(part)?;
    if !matches!(state.spatial.index(world).places[entry as usize], Place::Filed(_)) {
        return None;
    }
    let frame = state.sched.frame;
    state.spatial.set_place(world, entry, Place::Waits, frame);
    Some(Taken::Cells(world, entry))
}

/// `part` back where [`take_for_filing`] took it from, at the end: of its wait, or of its cells' lists in its assembly's list.
fn file_back(state: &mut State, names: &Names, part: InstanceId, taken: Taken) {
    match taken {
        Taken::Wait(None) => state.cameras.unindexed.insert(part),
        Taken::Wait(Some(home)) => state.cameras.unindexed_models.entry(home).or_default().insert(part),
        Taken::Cells(world, entry) => {
            let place = placed(filing_of(state, names, part, None));
            let frame = state.sched.frame;
            state.spatial.set_place(world, entry, place, frame);
        }
    }
}

/// `part` left the world of the WorldRoot `root` (camera_moves.rs's hierarchy hook): out of its index at once, if it holds it.
pub(crate) fn left(state: &mut State, root: InstanceId, part: InstanceId) {
    let State { spatial, dom, sched, .. } = state;
    let Some(world) = spatial.world(root) else { return };
    let Some((holder, entry)) = spatial.holder(part) else { return };
    if holder == world {
        spatial.remove(world, entry, sched.frame);
        if let Some(instance) = dom.get_mut(part) {
            instance.flags &= !(flags::INDEXED | flags::LOGGED);
        }
    }
}

/// Whether the world of `root` has an index holding any part (camera_moves.rs: a subtree leaving it is walked to take them out)
pub(crate) fn holds_any(state: &State, root: InstanceId) -> bool {
    state.spatial.world(root).is_some_and(|world| !state.spatial.index(world).is_empty())
}

/// The WorldModel `root` left the DataModel or was freed: its index goes, its parts' entries with it.
pub(crate) fn world_gone(state: &mut State, root: InstanceId) {
    let State { spatial, dom, .. } = state;
    spatial.drop_world(dom, root);
}

/// `id` is about to be freed: out of any index (its slot is reused).
pub(crate) fn freed(state: &mut State, id: InstanceId) {
    not_entered(state, id);
    let State { spatial, sched, .. } = state;
    if let Some((world, entry)) = spatial.holder(id) {
        spatial.remove(world, entry, sched.frame);
    }
}

/// Whether `part` waits for the world of `root`
fn waits(state: &State, workspace: InstanceId, root: InstanceId, part: InstanceId) -> bool {
    if root == workspace { state.cameras.unindexed.contains(&part) } else { state.cameras.unindexed_models.get(&root).is_some_and(|waiting| waiting.contains(&part)) }
}

/// Puts `part` in `world`'s index, or reads it again: its entry, and its filing if it does not wait (a new part that waits is
/// filed when it is taken in).
fn put(state: &mut State, engine: &Engine, names: Names, world: u32, part: InstanceId) {
    let workspace = engine.world().workspace;
    let entry = entry_of(state, engine, &names, part);
    let root = state.spatial.index(world).root;
    let waiting = waits(state, workspace, root, part);
    let frame = state.sched.frame;
    match state.spatial.holder(part).filter(|&(holder, _)| holder == world) {
        Some((_, at)) => {
            state.spatial.index_mut(world).entries[at as usize] = entry;
            let old = state.spatial.index(world).places[at as usize];
            // (a part that waits stays where it is until taken in; one that came back into the wait leaves its cells; one filed
            // keeps its list, which only an Anchored write or a link changes)
            let list = held_list(&state.spatial, world, at);
            let place = if waiting { Place::Waits } else { placed(filing_of(state, &names, part, list)) };
            let State { spatial, .. } = state;
            if !(waiting && old == Place::Waits) {
                spatial.set_place(world, at, place, frame);
            }
        }
        None => {
            let place = if waiting { Place::Waits } else { placed(filing_of(state, &names, part, None)) };
            let State { spatial, dom, .. } = state;
            spatial.add(world, part, entry, place, frame);
            if root == workspace {
                dom.at_mut(part).flags |= flags::INDEXED;
            }
        }
    }
}

/// Takes `part` out of `world`'s index, if it is in; whether it was.
fn take_out(state: &mut State, world: u32, part: InstanceId) -> bool {
    let State { spatial, dom, sched, .. } = state;
    let Some((holder, entry)) = spatial.holder(part) else { return false };
    if holder != world {
        return false;
    }
    spatial.remove(world, entry, sched.frame);
    if let Some(instance) = dom.get_mut(part) {
        instance.flags &= !(flags::INDEXED | flags::LOGGED);
    }
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
    if state.dom.get(root).is_none() {
        return (None, 0.0);
    }
    if root == workspace {
        return (Some(open_workspace(state, engine, names, workspace)), 0.0);
    }
    if !in_game(state, engine, root) {
        return (None, 0.0);
    }
    let (world, looked, filed) = open_model(state, engine, names, root);
    (Some(world), looked as f64 * crate::cost::FIND_NODE + filed as f64 * crate::cost::MODEL_INDEX_PART)
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
    let current = made.is_some() && state.dom.watch.as_ref().is_some_and(|watch| !watch.overflowed && watch.root == workspace);
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
            removed += u64::from(take_out(state, world, id));
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
                removed += u64::from(take_out(state, world, node));
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
    let current = made.is_some() && state.dom.world_log(model).is_some_and(|log| !log.overflowed);
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
            take_out(state, world, id);
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
                take_out(state, world, node);
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
/// already filed keeps its place. How many instances it visited and how many parts it filed.
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
    // (the WorldModels the walk meets inside the Workspace, for their indexes: open_workspace)
    let in_workspace = root == engine.world().workspace || state.dom.homes_above(root).top == Some(engine.world().workspace);
    for &node in &order {
        let class = state.dom.at(node).class;
        if in_workspace && node != root && engine.reflection.class(class).is_a(names.world_root) {
            state.spatial.nested.push(node);
        }
        if is_part(engine, names, class) && state.dom.home(node) == Some(root) {
            found.insert(node);
            let before = state.spatial.holder(node).map(|(holder, entry)| (holder, state.spatial.index(holder).places[entry as usize]));
            put(state, engine, names, world, node);
            let now = state.spatial.holder(node).map(|(_, entry)| state.spatial.index(world).places[entry as usize]);
            filed += usize::from(now.is_some_and(|place| place != Place::Waits) && !before.is_some_and(|(holder, place)| holder == world && place != Place::Waits));
        }
    }
    let stale: Vec<InstanceId> = {
        let index = state.spatial.index(world);
        index.live.iter().map(|&entry| index.parts[entry as usize]).filter(|part| !found.contains(part)).collect()
    };
    for part in stale {
        take_out(state, world, part);
    }
    state.spatial.index_mut(world).walked = visited;
    state.spatial.walk = walk;
    state.spatial.order = order;
    state.spatial.seen = seen;
    (visited, filed)
}

/// A query of the world of `root` takes in the parts that wait for it, in the order they came: each filed at the end of its
/// cells' lists. Its index (made first when it has none: none for a WorldModel outside the DataModel) and how many it took in.
pub(crate) fn take_in(engine: &Engine, root: InstanceId) -> (Option<u32>, usize, f64) {
    let (world, opened) = open(engine, root);
    let mut state = engine.state.borrow_mut();
    let taken = take_in_world(&mut state, engine, world, root);
    (world, taken, opened)
}

fn take_in_world(state: &mut State, engine: &Engine, world: Option<u32>, root: InstanceId) -> usize {
    links_started(state, &engine.reflection);
    let workspace = engine.world().workspace;
    let waiting = if root == workspace { std::mem::take(&mut state.cameras.unindexed) } else { state.cameras.unindexed_models.remove(&root).unwrap_or_default() };
    let count = waiting.len();
    if let (Some(world), Some(names)) = (world, state.spatial.names) {
        for part in waiting.iter() {
            if state.dom.get(part).is_none() {
                continue;
            }
            match state.spatial.holder(part) {
                Some((holder, entry)) if holder == world => {
                    let place = placed(filing_of(state, &names, part, None));
                    let frame = state.sched.frame;
                    let State { spatial, .. } = &mut *state;
                    if spatial.index(world).places[entry as usize] == Place::Waits {
                        spatial.set_place(world, entry, place, frame);
                    }
                }
                // (not in its index yet: read in now)
                _ if state.dom.home(part) == Some(root) => put(state, engine, names, world, part),
                _ => {}
            }
        }
    }
    let mut waiting = waiting;
    waiting.clear();
    if root == workspace {
        state.cameras.unindexed = waiting;
    }
    count
}

/// The physics step: every world's index takes in what waits for it, as a client's step does (the Workspace's index made at the
/// first step, a WorldModel's when its world has parts waiting).
pub fn index_parts(engine: &Engine) {
    let workspace = engine.world().workspace;
    let (world, _) = open(engine, workspace);
    let mut models: Vec<InstanceId> = engine.state.borrow().cameras.unindexed_models.keys().copied().collect();
    models.sort_unstable();
    {
        let mut state = engine.state.borrow_mut();
        take_in_world(&mut state, engine, world, workspace);
    }
    for model in models {
        let (world, _) = open(engine, model);
        let mut state = engine.state.borrow_mut();
        take_in_world(&mut state, engine, world, model);
    }
    engine.state.borrow_mut().cameras.unindexed_models.clear();
}

/// A walk's parts, in its order, into `out` (each with its entry), and what it did; `first` the list a cell gives first
/// ([`grid::UNANCHORED`] for the box queries, [`grid::ANCHORED`] for the casts). The world box `lo`..`hi` is the query's,
/// clamped. The world's parts whose box is not finite come last.
pub(crate) fn walk_box(state: &mut State, world: u32, lo: Vec3, hi: Vec3, first: u8, out: &mut Vec<(InstanceId, Entry)>) -> Walked {
    let State { spatial, sched, .. } = state;
    let index = spatial.index_mut(world);
    let before = out.len();
    let entries = index.entries.len();
    let WorldIndex { grid, entries: held, parts, loose, .. } = index;
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut walked = None;
    #[cfg(test)]
    if scan() && grid::count_walk(lo, hi, grid.top(sched.frame)) <= 2_000_000 {
        let (order, reference) = grid.walk_box_reference(lo, hi, sched.frame, entries, first);
        out.extend(order.into_iter().map(|entry| (parts[entry as usize], held[entry as usize])));
        walked = Some(reference);
    }
    let walked = walked.unwrap_or_else(|| {
        grid.walk_box(lo, hi, sched.frame, entries, first, |entry| {
            out.push((parts[entry as usize], held[entry as usize]));
            true
        })
    });
    out.extend(loose.iter().map(|&entry| (parts[entry as usize], held[entry as usize])));
    taken(out.len() - before);
    walked
}

/// [`walk_box`] a part at a time: `visit` is given each part of the walk with its entry until it returns false (world/region.rs:
/// ArePartsTouchingOthers' walk stops at the first part touching), the world's parts whose box is not finite last. What it did.
pub(crate) fn visit_box(state: &mut State, world: u32, lo: Vec3, hi: Vec3, first: u8, mut visit: impl FnMut(InstanceId, &Entry) -> bool) -> Walked {
    let State { spatial, sched, .. } = state;
    let index = spatial.index_mut(world);
    let entries = index.entries.len();
    let WorldIndex { grid, entries: held, parts, loose, .. } = index;
    let mut going = true;
    let walked = grid.walk_box(lo, hi, sched.frame, entries, first, |entry| {
        going = visit(parts[entry as usize], &held[entry as usize]);
        going
    });
    let mut given = walked.given as usize;
    if going {
        for &entry in loose.iter() {
            given += 1;
            if !visit(parts[entry as usize], &held[entry as usize]) {
                break;
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
    let entries = index.entries.len();
    let WorldIndex { grid, entries: held, parts, loose, .. } = index;
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut found = Vec::new();
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut done = false;
    #[cfg(test)]
    if scan() && grid::count_ray(origin, direction, grid.top(sched.frame)) <= 2_000_000 {
        let (cells, order) = grid.walk_ray_reference(origin, direction, sched.frame, entries);
        *passed = cells;
        found = order;
        done = true;
    }
    if !done {
        grid.walk_ray(origin, direction, sched.frame, entries, passed, &mut found);
    }
    out.extend(found.into_iter().map(|(entry, cell)| (parts[entry as usize], held[entry as usize], cell)));
    out.extend(loose.iter().map(|&entry| (parts[entry as usize], held[entry as usize], u32::MAX)));
    taken(out.len() - before);
}

/// Every part of the world's index that is filed or loose (a waiting part is not), into `out`, in no particular order
pub(crate) fn all_parts(state: &State, world: u32, out: &mut Vec<(InstanceId, Entry)>) {
    let index = state.spatial.index(world);
    let before = out.len();
    out.extend(index.live.iter().filter(|&&entry| index.places[entry as usize] != Place::Waits).map(|&entry| index.give(entry)));
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

/// Whether the world's index holds what a walk of its world finds: the same parts, each with the entry the dom gives it now and
/// filed as its properties file it now, or waiting exactly while it waits; and its grid sound (tests)
#[cfg(test)]
pub(crate) fn check_world(engine: &Engine, root: InstanceId) -> Result<(), String> {
    let world = open(engine, root).0.ok_or("no index")?;
    let state = engine.state.borrow();
    let names = state.spatial.names.expect("names");
    let index = state.spatial.index(world);
    let workspace = engine.world().workspace;
    let reflection = &engine.reflection;
    let mut walked: Vec<String> = state
        .dom
        .descendants(root)
        .into_iter()
        .filter(|&id| is_part(engine, names, state.dom.at(id).class) && state.dom.home(id) == Some(root))
        .map(|id| {
            let entry = entry_of(&state, engine, &names, id);
            // (the list as filed: an assembly's list is read from what the joints keep, which a check does not work out)
            let held = state.spatial.holder(id).and_then(|(world, entry)| held_list(&state.spatial, world, entry));
            let place = if waits(&state, workspace, root, id) { Place::Waits } else { placed(filing_of(&state, &names, id, held)) };
            format!("{id:?} {:?} {} {} {place:?}", entry.solid, entry.can_query, entry.can_collide)
        })
        .collect();
    let mut kept: Vec<String> = index
        .live
        .iter()
        .map(|&entry| {
            let (id, held) = index.give(entry);
            format!("{id:?} {:?} {} {} {:?}", held.solid, held.can_query, held.can_collide, index.places[entry as usize])
        })
        .collect();
    walked.sort();
    kept.sort();
    let _ = reflection;
    if walked != kept {
        let missing: Vec<&String> = walked.iter().filter(|line| !kept.contains(line)).take(3).collect();
        let extra: Vec<&String> = kept.iter().filter(|line| !walked.contains(line)).take(3).collect();
        return Err(format!("{} walked, {} held; missing {missing:?}; extra {extra:?}", walked.len(), kept.len()));
    }
    let filed: Vec<(u32, Filing)> = index.live.iter().filter_map(|&entry| match index.places[entry as usize] {
        Place::Filed(filing) => Some((entry, filing)),
        _ => None,
    }).collect();
    index.grid.check(&filed)?;
    let loose_ok = index.loose.iter().enumerate().all(|(place, &entry)| index.loose_at[entry as usize] as usize == place && index.places[entry as usize] == Place::Loose);
    if !loose_ok || index.live.iter().filter(|&&entry| index.places[entry as usize] == Place::Loose).count() != index.loose.len() {
        return Err("the loose list".to_string());
    }
    for &entry in &index.live {
        if state.spatial.holder(index.parts[entry as usize]) != Some((world, entry)) {
            return Err(format!("entry {entry}'s slot"));
        }
    }
    Ok(())
}

/// Puts `entry` at the end of a list of entries whose box is not finite, keeping its place there in `at` (by entry).
fn push_loose(loose: &mut Vec<u32>, at: &mut Vec<u32>, entry: u32) {
    if at.len() <= entry as usize {
        at.resize(entry as usize + 1, 0);
    }
    at[entry as usize] = loose.len() as u32;
    loose.push(entry);
}

/// Takes `entry` out of a list of entries whose box is not finite in a step, by the place `at` keeps for it: the last takes its
/// place.
fn take_loose(loose: &mut Vec<u32>, at: &mut [u32], entry: u32) {
    if let Some(place) = at.get(entry as usize).map(|&place| place as usize).filter(|&place| loose.get(place) == Some(&entry)) {
        loose.swap_remove(place);
        if let Some(&moved) = loose.get(place) {
            at[moved as usize] = place as u32;
        }
    }
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
    order.sort_by(|&a, &b| key(a).cmp(key(b)));
    let mut taken: Vec<Option<T>> = std::mem::take(items).into_iter().map(Some).collect();
    items.extend(order.into_iter().map(|at| taken[at].take().expect("each once")));
}
