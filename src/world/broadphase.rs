//! The spatial queries' broadphase (world/query.rs, and the Camera's and the PlayerMouse's rays: camera.rs, camera_moves.rs,
//! mouse.rs): the Workspace's parts, every BasePart in it but Terrain, as query solids in a dynamic bounding-volume tree, so a
//! query looks only at the parts whose bounds come near it, as a client's does (Studio 0.747, a headless client: a Raycast,
//! Spherecast, Blockcast, Shapecast, GetPartBoundsInBox, GetPartBoundsInRadius or GetPartsInPart beside 0, 10,000 or 100,000
//! parts away from it costs the same). A collision-group change lists the parts it holds ([`Index::part_ids`],
//! services/physics.rs), so it looks at none of the Workspace's other instances.
//!
//! The index follows the dom through its change log (dom.rs `Watch`): the instances attached below the Workspace, detached
//! from it, replaced or given another class there, the indexed parts freed, and the indexed parts written. A query first takes
//! what the log holds: an instance that moved has its subtree's parts put in or taken out, a written part its solid and flags
//! read again, each instance once however often it changed and however many of its ancestors moved too. So a query after a
//! change does work for what changed, never a walk of the Workspace; the walk happens when the index is first made and when the
//! log overflowed (dom.rs `Watch::limit`: at least twice the parts the index holds and the instances the last walk visited, each
//! instance logged once until taken), which takes as many distinct instances changed as the walk visits.
//!
//! A WorldModel inside the Workspace is a world of its own: the index keeps its parts in a second tree, which query.rs's queries
//! look in as in the first, and the walks of the Workspace's own world ([`Index::visit_box`]: world/region.rs's Region3 queries
//! and ArePartsTouchingOthers) never. Each WorldModel's world has an index of its own on the same tree ([`ModelIndex`]), which
//! follows the change log the dom keeps for that world (dom.rs `WorldLog`, read off the homes).
//!
//! What a query asks of the index is a superset: the parts whose bounds, a box around each part's bounding sphere (the sphere
//! query.rs `ray_solid` first tests against), meet the query's box or segment, both grown by more than float32 rounding can
//! move the narrow tests. The caller then tests each part as before and puts what it keeps in the order the Workspace's
//! descendants are listed ([`sort_by_order`]), so answers, the choice among ties and the counts a query is charged by are
//! those of a test of every part in that order. A part whose bounds are not finite (a NaN or infinite position or size) is
//! taken by every query (from a list of its own, which it leaves in a step when it is read again, moved out or destroyed),
//! and a query whose own geometry is not finite takes every part.
//!
//! A part that waits for the spatial index (camera.rs `Cameras::unindexed`: it entered the Workspace since a query or the
//! physics step last took parts in) is kept out of the tree, and so out of what the PlayerMouse's ray and
//! GetLargestCutoffDistance, which take nothing in and do not see it, are handed; it goes in when the index takes parts in
//! (`Cameras::taken_in` moved on), which every other query does first. A query whose geometry is not finite still takes
//! every part, waiting ones too.
//!
//! The tree is Box2D's dynamic AABB tree (surface-area insertion, rotations that keep it balanced), with leaves bounded
//! exactly (a moved part is put in again), and every walk of it iterative over a stack of its own.

use glam::Vec3;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{frame, query::Solid};
use crate::{
    ClassId, Engine, InstanceId, NameId, State,
    dom::{Dom, Watch, flags},
};

const NONE: u32 = u32::MAX;
/// An entry's leaf while its part waits for the spatial index ([`Index::waiting`])
const WAITS: u32 = u32::MAX - 1;

/// A part in the index: its solid and what the queries' filters read of it, as the dom last held them
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub solid: Solid,
    /// CanQuery is not false
    pub can_query: bool,
    /// CanCollide is not false
    pub can_collide: bool,
    /// Its leaf; NONE: its bounds are not finite, and it is among the parts every query takes; WAITS: its part waits for the
    /// spatial index, and it is in neither
    leaf: u32,
    /// In a WorldModel inside the Workspace, so in [`Index`]'s tree of those (`nested`)
    nested: bool,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    lo: Vec3,
    hi: Vec3,
    parent: u32,
    left: u32,
    /// NONE for a leaf
    right: u32,
    /// A leaf's entry
    entry: u32,
    /// 0 for a leaf
    height: i32,
}

/// Box2D's b2DynamicTree over boxes of float32 bounds
struct Tree {
    nodes: Vec<Node>,
    free: Vec<u32>,
    root: u32,
}

impl Default for Tree {
    fn default() -> Self {
        Tree { nodes: Vec::new(), free: Vec::new(), root: NONE }
    }
}

/// A box's surface area, in double (float32 bounds' squares do not overflow it)
fn area(lo: Vec3, hi: Vec3) -> f64 {
    let d = hi.as_dvec3() - lo.as_dvec3();
    2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
}

impl Tree {
    fn clear(&mut self) {
        self.nodes.clear();
        self.free.clear();
        self.root = NONE;
    }

    fn node(&self, index: u32) -> &Node {
        &self.nodes[index as usize]
    }

    fn node_mut(&mut self, index: u32) -> &mut Node {
        &mut self.nodes[index as usize]
    }

    fn alloc(&mut self, node: Node) -> u32 {
        match self.free.pop() {
            Some(index) => {
                self.nodes[index as usize] = node;
                index
            }
            None => {
                self.nodes.push(node);
                (self.nodes.len() - 1) as u32
            }
        }
    }

    fn union(&self, a: u32, b: u32) -> (Vec3, Vec3) {
        let (a, b) = (self.node(a), self.node(b));
        (a.lo.min(b.lo), a.hi.max(b.hi))
    }

    /// Recomputes `index`'s box and height from its children.
    fn refit(&mut self, index: u32) {
        let (left, right) = (self.node(index).left, self.node(index).right);
        let (lo, hi) = self.union(left, right);
        let height = 1 + self.node(left).height.max(self.node(right).height);
        let node = self.node_mut(index);
        node.lo = lo;
        node.hi = hi;
        node.height = height;
    }

    fn replace_child(&mut self, parent: u32, old: u32, new: u32) {
        if parent == NONE {
            self.root = new;
        } else if self.node(parent).left == old {
            self.node_mut(parent).left = new;
        } else {
            self.node_mut(parent).right = new;
        }
    }

    /// A new leaf for `entry` with bounds `lo`, `hi`
    fn insert(&mut self, entry: u32, lo: Vec3, hi: Vec3) -> u32 {
        let leaf = self.alloc(Node { lo, hi, parent: NONE, left: NONE, right: NONE, entry, height: 0 });
        if self.root == NONE {
            self.root = leaf;
            return leaf;
        }
        // the sibling that grows the tree's surface the least
        let mut index = self.root;
        while self.node(index).right != NONE {
            let node = *self.node(index);
            let own = area(node.lo, node.hi);
            let combined = area(node.lo.min(lo), node.hi.max(hi));
            let cost = 2.0 * combined;
            let inheritance = 2.0 * (combined - own);
            let descend = |child: u32| {
                let child = self.node(child);
                let grown = area(child.lo.min(lo), child.hi.max(hi));
                if child.right == NONE { grown + inheritance } else { grown - area(child.lo, child.hi) + inheritance }
            };
            let (left, right) = (descend(node.left), descend(node.right));
            if cost < left && cost < right {
                break;
            }
            index = if left < right { node.left } else { node.right };
        }
        let sibling = index;
        let old_parent = self.node(sibling).parent;
        let sibling_node = *self.node(sibling);
        let parent = self.alloc(Node { lo: sibling_node.lo.min(lo), hi: sibling_node.hi.max(hi), parent: old_parent, left: sibling, right: leaf, entry: NONE, height: sibling_node.height + 1 });
        self.replace_child(old_parent, sibling, parent);
        self.node_mut(sibling).parent = parent;
        self.node_mut(leaf).parent = parent;
        self.fix_upwards(parent);
        leaf
    }

    fn remove(&mut self, leaf: u32) {
        if leaf == self.root {
            self.root = NONE;
            self.free.push(leaf);
            return;
        }
        let parent = self.node(leaf).parent;
        let grand = self.node(parent).parent;
        let sibling = if self.node(parent).left == leaf { self.node(parent).right } else { self.node(parent).left };
        self.replace_child(grand, parent, sibling);
        self.node_mut(sibling).parent = grand;
        self.free.push(parent);
        self.free.push(leaf);
        if grand != NONE {
            self.fix_upwards(grand);
        }
    }

    /// Balances and refits `index` and every node above it.
    fn fix_upwards(&mut self, mut index: u32) {
        while index != NONE {
            index = self.balance(index);
            self.refit(index);
            index = self.node(index).parent;
        }
    }

    /// Box2D's rotation: when one child of `a` is more than one level taller than the other, its taller child takes `a`'s
    /// place. Returns the node now where `a` was.
    fn balance(&mut self, a: u32) -> u32 {
        let node = *self.node(a);
        if node.right == NONE || node.height < 2 {
            return a;
        }
        let (b, c) = (node.left, node.right);
        let balance = self.node(c).height - self.node(b).height;
        if balance > 1 {
            self.rotate_up(a, c, false)
        } else if balance < -1 {
            self.rotate_up(a, b, true)
        } else {
            a
        }
    }

    /// `up` (a child of `a`) takes `a`'s place; `a` keeps its other child and the shorter child of `up`.
    /// `left`: `up` was `a`'s left child.
    fn rotate_up(&mut self, a: u32, up: u32, left: bool) -> u32 {
        let (f, g) = (self.node(up).left, self.node(up).right);
        let a_parent = self.node(a).parent;
        self.node_mut(up).left = a;
        self.node_mut(up).parent = a_parent;
        self.node_mut(a).parent = up;
        self.replace_child(a_parent, a, up);
        let (keep, give) = if self.node(f).height > self.node(g).height { (f, g) } else { (g, f) };
        self.node_mut(up).right = keep;
        if left {
            self.node_mut(a).left = give;
        } else {
            self.node_mut(a).right = give;
        }
        self.node_mut(give).parent = a;
        self.refit(a);
        self.refit(up);
        up
    }

    /// Every leaf whose box `meets`, its entry to `visit`; `stack` is the walk's own.
    fn query(&self, stack: &mut Vec<u32>, meets: impl Fn(Vec3, Vec3) -> bool, mut visit: impl FnMut(u32)) {
        self.walk(stack, meets, |entry| {
            visit(entry);
            true
        });
    }

    /// [`Tree::query`] until `visit` returns false; whether it was given every leaf that meets.
    fn walk(&self, stack: &mut Vec<u32>, meets: impl Fn(Vec3, Vec3) -> bool, mut visit: impl FnMut(u32) -> bool) -> bool {
        if self.root == NONE {
            return true;
        }
        stack.clear();
        stack.push(self.root);
        while let Some(index) = stack.pop() {
            let node = self.node(index);
            if !meets(node.lo, node.hi) {
                continue;
            }
            if node.right == NONE {
                if !visit(node.entry) {
                    return false;
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
        true
    }
}

/// The box a part's index bounds: its bounding sphere's (`ray_solid`'s reach without the cast's growth), grown by more than
/// float32 rounding moves the narrow tests; None when it is not finite.
fn bounds(solid: &Solid) -> Option<(Vec3, Vec3)> {
    let center = frame::translation(&solid.frame);
    let radius = solid.half_extents().length();
    if !center.is_finite() || !radius.is_finite() || skewed(&solid.frame) {
        return None;
    }
    let reach = radius * 1.001 + center.abs().max_element() * 1e-5 + 1e-3;
    let (lo, hi) = (center - reach, center + reach);
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

/// Whether a frame's axes are further from square than a part's stored rotation can be (a script's CFrame write is
/// orthonormalized past a client's tolerance: lua/instance.rs cframe_as_stored), or not finite: its box is then no box its
/// bounding sphere holds, and the queries take it whatever its bounds.
pub fn skewed(frame: &glam::Affine3A) -> bool {
    let (x, y, z) = (Vec3::from(frame.matrix3.x_axis), Vec3::from(frame.matrix3.y_axis), Vec3::from(frame.matrix3.z_axis));
    let worst = x.dot(y).abs().max(y.dot(z).abs()).max(z.dot(x).abs());
    !(x.is_finite() && y.is_finite() && z.is_finite()) || worst.is_nan() || worst > 1e-4
}

/// How far a query's own box or segment is grown past its geometry for float32 rounding, from the largest coordinate it reaches
fn slack(largest: f32) -> f32 {
    largest * 1e-5 + 1e-3
}

/// Whether the segment from `origin` along `direction` meets the box `lo`..`hi` grown by `grow` (a conservative slab test: an
/// axis whose division is not a number constrains nothing).
fn segment_meets(lo: Vec3, hi: Vec3, origin: Vec3, direction: Vec3, grow: f32) -> bool {
    let (lo, hi) = (lo - grow, hi + grow);
    let (mut enter, mut exit) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        let (o, d) = (origin[axis], direction[axis]);
        if d == 0.0 {
            if o < lo[axis] || o > hi[axis] {
                return false;
            }
            continue;
        }
        let (mut near, mut far) = ((lo[axis] - o) / d, (hi[axis] - o) / d);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        enter = enter.max(near);
        exit = exit.min(far);
        if enter > exit {
            return false;
        }
    }
    true
}

#[cfg(test)]
thread_local! {
    /// How many times an index has walked the Workspace on this thread (tests)
    pub(crate) static WORKSPACE_WALKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Every query takes every part (tests: the broadphase's answers against a test of every part)
    pub(crate) static SCAN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How many parts the queries have taken from an index on this thread (tests)
    pub(crate) static TAKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// How many times [`sort_by_order`] has ordered by the trie with one node for each list ([`Trie::keyed`]) on this thread (tests)
    pub(crate) static KEYED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// How many instances the walks of what moved have looked at, taking the log, on this thread (tests)
    pub(crate) static SYNC_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Counts what a query took ([`TAKEN`], tests)
fn taken(_count: usize) {
    #[cfg(test)]
    TAKEN.with(|taken| taken.set(taken.get() + _count as u64));
}

/// Counts an instance a walk of what moved looked at ([`SYNC_VISITS`], tests)
fn visited() {
    #[cfg(test)]
    SYNC_VISITS.with(|visits| visits.set(visits.get() + 1));
}

/// The Workspace's parts in a tree ([`Index::sync`] keeps it in step with the dom)
#[derive(Default)]
pub struct Index {
    built: bool,
    entries: Vec<Entry>,
    /// Each entry's part (stale for a free entry)
    parts: Vec<InstanceId>,
    free_entries: Vec<u32>,
    /// The entries in use, in no particular order, and each one's place in that list ([`Index::part_ids`] reads the parts held
    /// without the free entries, which the lists above keep however many parts left)
    live: Vec<u32>,
    live_at: Vec<u32>,
    /// A dom slot's entry, plus one (0: none)
    by_slot: Vec<u32>,
    /// The entries whose bounds are not finite, and each one's place in that list (by entry), so one comes out in a step: the
    /// last takes its place, as a search for it and a swap_remove left the list. A part read again that stays loose goes to
    /// the end (taken out, then pushed), the order the queries take these parts in
    loose: Vec<u32>,
    loose_at: Vec<u32>,
    /// The entries whose parts wait for the spatial index (camera.rs `Cameras::unindexed`), in no tree or list a query near
    /// something reads, until it takes parts in
    waiting: FxHashSet<u32>,
    /// How many times the spatial index had taken parts in (camera.rs `Cameras::taken_in`) when the waiting entries were last
    /// placed
    taken_in: u64,
    tree: Tree,
    /// The parts of the WorldModels inside the Workspace (dom.rs `Dom::home`), in a tree of their own: each is its own world,
    /// which a client's Workspace index does not hold. The queries of query.rs look in both trees; a walk of the Workspace's own
    /// world ([`Index::visit_box`]: world/region.rs) in `tree` alone, so it never meets them.
    nested: Tree,
    /// The entries of `nested` whose bounds are not finite, and each one's place in that list, as `loose` keeps them
    nested_loose: Vec<u32>,
    nested_loose_at: Vec<u32>,
    stack: Vec<u32>,
    seen: FxHashSet<InstanceId>,
    walk: Vec<InstanceId>,
    /// How many instances the last walk of the Workspace visited ([`Index::limit`])
    walked: usize,
    names: Option<Names>,
}

#[derive(Clone, Copy)]
struct Names {
    base_part: ClassId,
    terrain: ClassId,
    world_root: ClassId,
    can_query: Option<NameId>,
    can_collide: Option<NameId>,
    /// BasePart.CollisionGroup and CollisionGroupId, which no entry reads (the queries model no collision groups: query.rs)
    collision_group: Option<NameId>,
    collision_group_id: Option<NameId>,
}

impl Names {
    fn of(engine: &Engine) -> Option<Names> {
        let reflection = &engine.reflection;
        let (base_part, terrain, world_root) = (reflection.class_id("BasePart")?, reflection.class_id("Terrain")?, reflection.class_id("WorldRoot")?);
        Some(Names {
            base_part,
            terrain,
            world_root,
            can_query: reflection.name_id("CanQuery"),
            can_collide: reflection.name_id("CanCollide"),
            collision_group: reflection.name_id("CollisionGroup"),
            collision_group_id: reflection.name_id("CollisionGroupId"),
        })
    }
}

impl Index {
    fn names(&mut self, engine: &Engine) -> Option<Names> {
        if self.names.is_none() {
            self.names = Names::of(engine);
        }
        self.names
    }

    /// How many parts it holds
    pub fn len(&self) -> usize {
        self.entries.len() - self.free_entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Takes the dom's change log (dom.rs `Watch`), or makes the index from the Workspace when it has none yet or the log
    /// overflowed.
    pub fn sync(&mut self, engine: &Engine) {
        let Some(names) = self.names(engine) else { return };
        let workspace = engine.world().workspace;
        let mut state = engine.state.borrow_mut();
        let state = &mut *state;
        let current = self.built && state.dom.watch.as_ref().is_some_and(|watch| !watch.overflowed && watch.root == workspace);
        if !current {
            crate::profiler::caches::miss(crate::profiler::caches::Cache::Queries);
            self.rebuild(state, engine, names, workspace);
            return;
        }
        self.take_waiting(state);
        let watch = state.dom.watch.as_mut().expect("watched");
        if watch.moved.is_empty() && watch.written.is_empty() {
            crate::profiler::caches::hit(crate::profiler::caches::Cache::Queries);
            return;
        }
        crate::profiler::caches::miss(crate::profiler::caches::Cache::Queries);
        let (mut moved, mut written) = watch.take();
        let mut removed = 0u64;
        // what moved first (a part written and then moved out is gone before its write is looked at). Each instance is looked at
        // once: a walk does not go into an instance an earlier walk reached, whose subtree that walk took whole with the same
        // answer (the tree stays as it is while the log is taken, and an instance below the Workspace or out of it has its subtree
        // with it), so a chain of parts that moved costs the walk its length, where each part's walk went down the rest of it.
        // (A log naming the Workspace itself, given another class, walks each entry's subtree whole, as each did: that walk's
        // answer is not its descendants'.)
        let once = !moved.contains(&workspace);
        crate::dom::clear_set(&mut self.seen);
        for &id in &moved {
            if !self.seen.insert(id) {
                continue;
            }
            if state.dom.get(id).is_none() {
                removed += u64::from(self.remove(&mut state.dom, id));
                continue;
            }
            let inside = state.dom.is_descendant_of_home(id, workspace);
            self.walk.clear();
            self.walk.push(id);
            while let Some(node) = self.walk.pop() {
                visited();
                let (class, indexed) = {
                    let instance = state.dom.at(node);
                    if once {
                        let seen = &mut self.seen;
                        self.walk.extend(instance.children.iter().filter(|&&child| seen.insert(child)));
                    } else {
                        instance.children.copy_to(&mut self.walk);
                    }
                    (instance.class, instance.flags & flags::INDEXED != 0)
                };
                if inside && is_part(engine, names, class) {
                    self.put(state, engine, names, node);
                } else if indexed {
                    removed += u64::from(self.remove(&mut state.dom, node));
                }
            }
        }
        for &id in &written {
            let Some(instance) = state.dom.get_mut(id) else { continue };
            instance.flags &= !flags::LOGGED;
            if instance.flags & flags::INDEXED != 0 {
                self.put(state, engine, names, id);
            }
        }
        crate::profiler::caches::evicted(crate::profiler::caches::Cache::Queries, removed);
        // the log's storage back to it
        moved.clear();
        written.clear();
        let limit = self.limit();
        let watch = state.dom.watch.as_mut().expect("watched");
        watch.limit = limit;
        if watch.moved.is_empty() {
            watch.moved = moved;
        }
        if watch.written.is_empty() {
            watch.written = written;
        }
    }

    /// The log's limit: twice the parts held and the instances the last walk of the Workspace visited, so the log overflows only
    /// after as many distinct instances changed (each is logged once until taken) as the walk it forces visits
    fn limit(&self) -> usize {
        (self.len() * 2).max(self.walked).max(4096)
    }

    fn rebuild(&mut self, state: &mut State, engine: &Engine, names: Names, workspace: InstanceId) {
        #[cfg(test)]
        WORKSPACE_WALKS.with(|walks| walks.set(walks.get() + 1));
        // (what the old index marked, unmarked)
        for (entry, &part) in self.parts.iter().enumerate() {
            if self.by_slot.get(part.index as usize) == Some(&(entry as u32 + 1))
                && let Some(instance) = state.dom.get_mut(part)
            {
                instance.flags &= !(flags::INDEXED | flags::LOGGED);
            }
        }
        self.entries.clear();
        self.parts.clear();
        self.free_entries.clear();
        self.live.clear();
        self.live_at.clear();
        self.by_slot.clear();
        self.loose.clear();
        self.loose_at.clear();
        self.waiting.clear();
        self.taken_in = state.cameras.taken_in;
        self.tree.clear();
        self.nested.clear();
        self.nested_loose.clear();
        self.nested_loose_at.clear();
        self.walk.clear();
        self.walk.extend(state.dom.at(workspace).children.iter().rev());
        self.walked = 0;
        while let Some(node) = self.walk.pop() {
            self.walked += 1;
            let class = {
                let instance = state.dom.at(node);
                self.walk.extend(instance.children.iter().rev());
                instance.class
            };
            if is_part(engine, names, class) {
                self.put(state, engine, names, node);
            }
        }
        self.built = true;
        let mut watch = Watch::new(workspace, self.limit());
        watch.unread.extend([names.collision_group, names.collision_group_id].into_iter().flatten());
        state.dom.watch = Some(watch);
    }

    /// The waiting entries placed by their bounds, once the spatial index has taken parts in since they were put (their parts no
    /// longer wait: a part that came back in since is in the log, and waits again when it is read)
    fn take_waiting(&mut self, state: &State) {
        if self.taken_in == state.cameras.taken_in {
            return;
        }
        self.taken_in = state.cameras.taken_in;
        let mut waiting = std::mem::take(&mut self.waiting);
        for &entry in &waiting {
            self.entries[entry as usize].leaf = NONE;
            let bounds = bounds(&self.entries[entry as usize].solid);
            self.place(entry, bounds, false);
        }
        waiting.clear();
        self.waiting = waiting;
    }

    fn entry_of(&self, id: InstanceId) -> Option<u32> {
        let entry = self.by_slot.get(id.index as usize).copied().filter(|&e| e != 0)? - 1;
        (self.parts[entry as usize] == id).then_some(entry)
    }

    /// Puts `part` in, or reads it again: its solid, flags and bounds.
    fn put(&mut self, state: &mut State, engine: &Engine, names: Names, part: InstanceId) {
        let solid = super::query::solid(state, engine, part);
        let not_false = |name: Option<NameId>| name.and_then(|n| state.dom.boolean(part, n)) != Some(false);
        let (can_query, can_collide) = (not_false(names.can_query), not_false(names.can_collide));
        let bounds = bounds(&solid);
        let waits = !state.cameras.unindexed.is_empty() && state.cameras.unindexed.contains(&part);
        // (a part of a WorldModel inside the Workspace goes in the tree of those)
        let nested = state.dom.home(part).is_some_and(|home| home != engine.world().workspace);
        let slot = part.index as usize;
        if self.by_slot.len() <= slot {
            self.by_slot.resize(slot + 1, 0);
        }
        let entry = match self.entry_of(part) {
            Some(entry) => {
                let old = self.entries[entry as usize];
                let same = if waits {
                    old.leaf == WAITS
                } else {
                    old.leaf != NONE && old.leaf != WAITS && old.nested == nested && bounds.is_some_and(|(lo, hi)| {
                        let node = if nested { self.nested.node(old.leaf) } else { self.tree.node(old.leaf) };
                        node.lo == lo && node.hi == hi
                    })
                };
                if !same {
                    self.unplace(entry);
                }
                self.entries[entry as usize] = Entry { solid, can_query, can_collide, leaf: if same { old.leaf } else { NONE }, nested };
                if same {
                    return;
                }
                entry
            }
            None => {
                // (a slot's earlier instance, freed and not yet taken out, goes first)
                if self.by_slot[slot] != 0 {
                    let stale = self.parts[(self.by_slot[slot] - 1) as usize];
                    self.remove(&mut state.dom, stale);
                }
                let fresh = Entry { solid, can_query, can_collide, leaf: NONE, nested };
                let entry = match self.free_entries.pop() {
                    Some(entry) => {
                        self.entries[entry as usize] = fresh;
                        self.parts[entry as usize] = part;
                        entry
                    }
                    None => {
                        self.entries.push(fresh);
                        self.parts.push(part);
                        (self.entries.len() - 1) as u32
                    }
                };
                self.by_slot[slot] = entry + 1;
                if self.live_at.len() <= entry as usize {
                    self.live_at.resize(entry as usize + 1, 0);
                }
                self.live_at[entry as usize] = self.live.len() as u32;
                self.live.push(entry);
                state.dom.at_mut(part).flags |= flags::INDEXED;
                entry
            }
        };
        self.place(entry, bounds, waits);
    }

    /// Puts `entry` (in none of them) in the tree by `bounds`, in the loose list without, or among the waiting entries.
    fn place(&mut self, entry: u32, bounds: Option<(Vec3, Vec3)>, waits: bool) {
        if waits {
            self.entries[entry as usize].leaf = WAITS;
            self.waiting.insert(entry);
            return;
        }
        let nested = self.entries[entry as usize].nested;
        match bounds {
            Some((lo, hi)) => self.entries[entry as usize].leaf = if nested { self.nested.insert(entry, lo, hi) } else { self.tree.insert(entry, lo, hi) },
            None if nested => push_loose(&mut self.nested_loose, &mut self.nested_loose_at, entry),
            None => push_loose(&mut self.loose, &mut self.loose_at, entry),
        }
    }

    /// Takes `entry` out of its tree, its loose list (in a step, by its place there) or the waiting entries.
    fn unplace(&mut self, entry: u32) {
        let Entry { leaf, nested, .. } = self.entries[entry as usize];
        if leaf == WAITS {
            self.waiting.remove(&entry);
        } else if leaf != NONE {
            if nested { self.nested.remove(leaf) } else { self.tree.remove(leaf) }
        } else if nested {
            take_loose(&mut self.nested_loose, &mut self.nested_loose_at, entry);
        } else {
            take_loose(&mut self.loose, &mut self.loose_at, entry);
        }
        self.entries[entry as usize].leaf = NONE;
    }

    /// Takes `part` out, if it is in; whether it was.
    fn remove(&mut self, dom: &mut Dom, part: InstanceId) -> bool {
        let Some(entry) = self.entry_of(part) else { return false };
        self.unplace(entry);
        self.by_slot[part.index as usize] = 0;
        self.free_entries.push(entry);
        let at = self.live_at[entry as usize] as usize;
        self.live.swap_remove(at);
        if let Some(&moved) = self.live.get(at) {
            self.live_at[moved as usize] = at as u32;
        }
        if let Some(instance) = dom.get_mut(part) {
            instance.flags &= !(flags::INDEXED | flags::LOGGED);
        }
        true
    }

    fn scan(&self) -> bool {
        #[cfg(test)]
        if SCAN.with(|scan| scan.get()) {
            return true;
        }
        false
    }

    /// Every part held, into `out`, in no particular order: one step for each, however many instances the Workspace holds
    /// beside them and however many parts it held before (a list of its own, not a query's take: [`TAKEN`] leaves them out)
    pub fn part_ids(&self, out: &mut Vec<InstanceId>) {
        out.extend(self.live.iter().map(|&entry| self.parts[entry as usize]));
    }

    /// Every part, into `out`
    pub fn all_parts(&self, out: &mut Vec<(InstanceId, Entry)>) {
        self.all_parts_in(out, true);
    }

    /// [`Index::all_parts`], with the WorldModels' parts in the Workspace (`nested`) or without.
    fn all_parts_in(&self, out: &mut Vec<(InstanceId, Entry)>, nested: bool) {
        let before = out.len();
        for (entry, &part) in self.parts.iter().enumerate() {
            if self.by_slot.get(part.index as usize) == Some(&(entry as u32 + 1)) && (nested || !self.entries[entry].nested) {
                out.push((part, self.entries[entry]));
            }
        }
        taken(out.len() - before);
    }

    /// The parts whose bounds may meet the segment from `origin` along `direction` with a sphere of radius `grow` swept along
    /// it, but those that wait for the spatial index (every part when the segment is not finite), into `out`, in no particular
    /// order.
    pub fn near_segment(&mut self, origin: Vec3, direction: Vec3, grow: f32, out: &mut Vec<(InstanceId, Entry)>) {
        self.near_segment_in(origin, direction, grow, out, true);
    }

    /// [`Index::near_segment`] for the Workspace's own world: not a WorldModel's parts in it (`nested`). The PlayerMouse's ray and
    /// GetLargestCutoffDistance read this: a client's never meet a WorldModel's parts, settled or new (measured, Studio 0.748, a
    /// playing client: a wall in a WorldModel in the Workspace between the camera and its focus leaves the cutoff 0 and Target
    /// nil, where the same wall in the Workspace gives 18 and the wall; world/region.rs).
    pub fn near_segment_own(&mut self, origin: Vec3, direction: Vec3, grow: f32, out: &mut Vec<(InstanceId, Entry)>) {
        self.near_segment_in(origin, direction, grow, out, false);
    }

    fn near_segment_in(&mut self, origin: Vec3, direction: Vec3, grow: f32, out: &mut Vec<(InstanceId, Entry)>, nested: bool) {
        let end = origin + direction;
        let finite = origin.is_finite() && direction.is_finite() && end.is_finite() && grow.is_finite();
        if !finite || self.scan() {
            return self.all_parts_in(out, nested);
        }
        let grow = grow.max(0.0) * 1.001 + slack(origin.abs().max_element().max(end.abs().max_element()));
        let (entries, parts, before) = (&self.entries, &self.parts, out.len());
        let meets = |lo, hi| segment_meets(lo, hi, origin, direction, grow);
        let trees: &[&Tree] = if nested { &[&self.tree, &self.nested] } else { &[&self.tree] };
        for tree in trees {
            tree.query(&mut self.stack, meets, |entry| out.push((parts[entry as usize], entries[entry as usize])));
        }
        let loose = if nested { &self.nested_loose[..] } else { &[] };
        out.extend(self.loose.iter().chain(loose).map(|&entry| (parts[entry as usize], entries[entry as usize])));
        taken(out.len() - before);
    }

    /// The parts whose bounds may meet the box `center` +- `extent`, but those that wait for the spatial index (every part when
    /// there is none or it is not finite), into `out`, in no particular order.
    pub fn near_box(&mut self, center: Vec3, extent: Option<Vec3>, out: &mut Vec<(InstanceId, Entry)>) {
        let Some(extent) = extent.map(Vec3::abs) else { return self.all_parts(out) };
        let finite = center.is_finite() && extent.is_finite() && (center + extent).is_finite() && (center - extent).is_finite();
        if !finite || self.scan() {
            return self.all_parts(out);
        }
        let extent = extent * 1.001 + slack(center.abs().max_element() + extent.max_element());
        let (low, high) = (center - extent, center + extent);
        let (entries, parts, before) = (&self.entries, &self.parts, out.len());
        for tree in [&self.tree, &self.nested] {
            tree.query(&mut self.stack, |lo: Vec3, hi: Vec3| lo.cmple(high).all() && hi.cmpge(low).all(), |entry| out.push((parts[entry as usize], entries[entry as usize])));
        }
        out.extend(self.loose.iter().chain(&self.nested_loose).map(|&entry| (parts[entry as usize], entries[entry as usize])));
        taken(out.len() - before);
    }

    /// [`Index::near_box`] for the Workspace's own world (not a WorldModel's in it: `nested`), a part at a time: `visit` is given
    /// each of its parts whose bounds may meet the box `center` +- `extent` (every one when it is not finite), in no particular
    /// order, until it returns false (world/region.rs: ArePartsTouchingOthers' walk stops at the first part touching). Whether it
    /// was given them all.
    pub fn visit_box(&mut self, center: Vec3, extent: Vec3, mut visit: impl FnMut(InstanceId, &Entry) -> bool) -> bool {
        let extent = extent.abs();
        let finite = center.is_finite() && extent.is_finite() && (center + extent).is_finite() && (center - extent).is_finite();
        let every = !finite || self.scan();
        let (entries, parts) = (&self.entries, &self.parts);
        let mut given = 0usize;
        let mut give = |entry: u32| {
            given += 1;
            visit(parts[entry as usize], &entries[entry as usize])
        };
        // (every part: the whole tree)
        let (low, high) = if every {
            (Vec3::NAN, Vec3::NAN)
        } else {
            let extent = extent * 1.001 + slack(center.abs().max_element() + extent.max_element());
            (center - extent, center + extent)
        };
        let all = self.tree.walk(&mut self.stack, |lo, hi| every || (lo.cmple(high).all() && hi.cmpge(low).all()), &mut give)
            && self.loose.iter().all(|&entry| give(entry));
        taken(given);
        all
    }

    #[cfg(test)]
    /// The tree's height and whether every node's box holds its children's and every parent link is right, and the loose lists
    /// hold each entry outside the trees and the waiting entries once, at the place they keep for it (tests)
    pub(crate) fn check_tree(&self) -> (i32, bool) {
        let loose_ok = |loose: &[u32], at: &[u32], nested: bool| {
            loose.iter().enumerate().all(|(place, &entry)| {
                let entry_now = self.entries[entry as usize];
                at[entry as usize] as usize == place && entry_now.leaf == NONE && entry_now.nested == nested
            })
        };
        let mut ok = loose_ok(&self.loose, &self.loose_at, false) && loose_ok(&self.nested_loose, &self.nested_loose_at, true);
        ok &= self.live.iter().filter(|&&entry| self.entries[entry as usize].leaf == NONE).count() == self.loose.len() + self.nested_loose.len();
        for (tree, nested) in [(&self.tree, false), (&self.nested, true)] {
            if tree.root == NONE {
                continue;
            }
            ok &= tree.node(tree.root).parent == NONE;
            let mut stack = vec![tree.root];
            while let Some(index) = stack.pop() {
                let node = tree.node(index);
                if node.right == NONE {
                    let entry = self.entries[node.entry as usize];
                    ok &= entry.leaf == index && entry.nested == nested;
                    continue;
                }
                for child in [node.left, node.right] {
                    let c = tree.node(child);
                    ok &= c.parent == index && c.lo.cmpge(node.lo).all() && c.hi.cmple(node.hi).all();
                    stack.push(child);
                }
                ok &= node.height == 1 + tree.node(node.left).height.max(tree.node(node.right).height);
            }
        }
        (if self.tree.root == NONE { 0 } else { self.tree.node(self.tree.root).height }, ok)
    }
}

/// Puts `entry` at the end of a list of entries whose bounds are not finite, keeping its place there in `at` (by entry).
fn push_loose(loose: &mut Vec<u32>, at: &mut Vec<u32>, entry: u32) {
    if at.len() <= entry as usize {
        at.resize(entry as usize + 1, 0);
    }
    at[entry as usize] = loose.len() as u32;
    loose.push(entry);
}

/// Takes `entry` out of a list of entries whose bounds are not finite in a step, by the place `at` keeps for it: the last takes
/// its place, as a search for it and a swap_remove left the list.
fn take_loose(loose: &mut Vec<u32>, at: &mut [u32], entry: u32) {
    if let Some(place) = at.get(entry as usize).map(|&place| place as usize).filter(|&place| loose.get(place) == Some(&entry)) {
        loose.swap_remove(place);
        if let Some(&moved) = loose.get(place) {
            at[moved as usize] = place as u32;
        }
    }
}

fn is_part(engine: &Engine, names: Names, class: ClassId) -> bool {
    let info = engine.reflection.class(class);
    info.is_a(names.base_part) && !info.is_a(names.terrain)
}

#[cfg(test)]
thread_local! {
    /// How many times a WorldModel's index has walked its world on this thread (tests)
    pub(crate) static MODEL_WALKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A WorldModel's parts in a tree of their own (world/region.rs: a client keeps an index for each world), as [`Index`] keeps the
/// Workspace's: its world is the BaseParts whose nearest WorldRoot it is (dom.rs `Dom::home`; Terrain left out, and a WorldModel
/// inside it keeps its own), and the index follows the change log the dom keeps for that world (dom.rs `WorldLog`): a query
/// after a change in it does work for what changed, and a change anywhere else none. The walk of the world happens when the
/// index is first made and when the log overflowed (at least twice the parts it holds), which takes as many changes as the walk
/// visits distinct instances: each changed instance is logged and looked at once (dom.rs `WorldLog`), however often it changed and
/// however many of its ancestors moved too. A part that waits for the spatial index (camera.rs `Cameras::unindexed_models`) is
/// kept out of the tree until a query of its world or the physics step takes it in, as [`Index`] keeps the Workspace's. The
/// engine keeps the index while its WorldModel exists (camera.rs `Cameras::model_indexes`: world/camera_moves.rs drops it when
/// the WorldModel is freed), and a query pays for each instance it looks at, in a walk of its world or in what changed in it
/// (world/region.rs).
#[derive(Default)]
pub struct ModelIndex {
    /// The WorldModel whose world it holds, once made
    model: Option<InstanceId>,
    entries: Vec<Entry>,
    /// Each entry's part (stale for a free entry)
    parts: Vec<InstanceId>,
    free_entries: Vec<u32>,
    /// A part's entry
    by_part: FxHashMap<InstanceId, u32>,
    /// The entries whose bounds are not finite, and each one's place in that list, as [`Index`] keeps them
    loose: Vec<u32>,
    loose_at: Vec<u32>,
    /// The entries whose parts wait for the spatial index, in no tree or list a walk reads, until their world takes them in
    waiting: FxHashSet<u32>,
    /// camera.rs `Cameras::models_stepped` and its WorldModel's `Cameras::models_waits` when its entries were last brought in step
    /// with its world's wait ([`waits_seen`])
    waits_seen: (u64, u64),
    tree: Tree,
    stack: Vec<u32>,
    seen: FxHashSet<InstanceId>,
    walk: Vec<InstanceId>,
    /// How many instances the last walk of its world visited ([`ModelIndex::limit`])
    walked: usize,
    names: Option<Names>,
    /// Whether the WorldModel was in the DataModel, and the dom's structure version when that was asked
    in_game: Option<(u64, bool)>,
}

#[cfg(test)]
thread_local! {
    /// How many instances the WorldModel indexes' walks of what moved have looked at on this thread (tests)
    pub(crate) static MODEL_SYNC_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// How many times a WorldModel's index has looked at its world's wait again on this thread (tests)
    pub(crate) static MODEL_WAIT_LOOKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Where `model`'s wait stands: how many times the physics step took every world's parts in, and how many times `model`'s own
/// wait changed other than as its log shows (camera.rs `Cameras::models_stepped`, `Cameras::models_waits`).
fn waits_seen(state: &State, model: InstanceId) -> (u64, u64) {
    (state.cameras.models_stepped, state.cameras.models_waits.get(&model).copied().unwrap_or(0))
}

impl ModelIndex {
    /// How many parts it holds
    pub fn len(&self) -> usize {
        self.entries.len() - self.free_entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether it was made from a walk of its world (a WorldModel outside the DataModel makes none)
    pub fn is_made(&self) -> bool {
        self.model.is_some()
    }

    /// Whether `model` is in the DataModel (a WorldModel outside it has no world: world/region.rs), and how many ancestors that
    /// took walking up (its price: world/region.rs). Read off the outermost home above it (dom.rs Dom::homes_above, one lookup
    /// however many WorldModels are nested above it: the Workspace is in the DataModel), then up that home's parents, if it is
    /// not the Workspace; asked again only after a parent changed somewhere.
    pub fn in_game(&mut self, engine: &Engine, model: InstanceId) -> (bool, usize) {
        let state = engine.state.borrow();
        let version = state.dom.structure_version;
        if let Some((at, inside)) = self.in_game
            && at == version
        {
            return (inside, 0);
        }
        let (workspace, game) = (engine.world().workspace, engine.world().game);
        let mut walked = 0usize;
        let inside = 'up: {
            if state.dom.get(model).is_none() {
                break 'up false;
            }
            let top = state.dom.homes_above(model).top;
            if top == Some(workspace) {
                break 'up true;
            }
            let mut current = state.dom.parent_of(top.unwrap_or(model));
            while let Some(node) = current {
                walked += 1;
                if node == game {
                    break 'up true;
                }
                current = state.dom.parent_of(node);
            }
            false
        };
        self.in_game = Some((version, inside));
        (inside, walked)
    }

    /// Takes the dom's log of `model`'s world, or makes the index from a walk of it when it has none yet or the log overflowed:
    /// how many instances it looked at (those in the log that moved and the walks below them, or the walk of its world) and how
    /// many parts a walk of its world put in (their price: world/region.rs).
    pub fn sync(&mut self, engine: &Engine, model: InstanceId) -> (usize, usize) {
        if self.names.is_none() {
            self.names = Names::of(engine);
        }
        let Some(names) = self.names else { return (0, 0) };
        let mut state = engine.state.borrow_mut();
        let state = &mut *state;
        if state.dom.get(model).is_none() {
            return (0, 0);
        }
        let current = self.model == Some(model) && state.dom.world_log(model).is_some_and(|log| !log.overflowed);
        if !current {
            return self.rebuild(state, engine, names, model);
        }
        self.take_waiting(state, model);
        let log = state.dom.world_log(model).expect("followed");
        if log.moved.is_empty() && log.written.is_empty() {
            return (0, 0);
        }
        let (mut moved, mut written) = log.take();
        // what moved first (a part written and then moved out is gone before its write is looked at). Each instance is looked at
        // once: a walk does not go into an instance an earlier walk of this log reached, whose subtree that walk took whole with
        // the same answer (the tree stays as it is while the log is taken), so a chain of parts that moved costs the walk its
        // length, where each part's walk went down the rest of it. An instance no longer in this world is taken out and not
        // walked: the parts that left with it were logged one by one as they left (dom.rs `log_world_leaving`), and it sits in
        // the log of every world it passed through, so a walk of what is below it now would be paid by every one of them.
        crate::dom::clear_set(&mut self.seen);
        let mut visited = 0usize;
        for &id in &moved {
            if !self.seen.insert(id) {
                continue;
            }
            if state.dom.get(id).is_none() || state.dom.home(id) != Some(model) {
                visited += 1;
                self.remove(id);
                continue;
            }
            self.walk.clear();
            self.walk.push(id);
            while let Some(node) = self.walk.pop() {
                visited += 1;
                let class = {
                    let instance = state.dom.at(node);
                    let class = engine.reflection.class(instance.class);
                    // (a world inside it keeps its own parts)
                    if !class.is_a(names.world_root) {
                        let seen = &mut self.seen;
                        self.walk.extend(instance.children.iter().filter(|&&child| seen.insert(child)));
                    }
                    instance.class
                };
                if is_part(engine, names, class) && state.dom.home(node) == Some(model) {
                    self.put(state, engine, names, node);
                } else {
                    self.remove(node);
                }
            }
        }
        for &id in &written {
            if self.by_part.contains_key(&id) && state.dom.get(id).is_some() {
                self.put(state, engine, names, id);
            }
        }
        // the log's storage back to it
        moved.clear();
        written.clear();
        let limit = self.limit();
        let log = state.dom.world_log(model).expect("followed");
        log.limit = limit;
        if log.moved.is_empty() {
            log.moved = moved;
        }
        if log.written.is_empty() {
            log.written = written;
        }
        #[cfg(test)]
        MODEL_SYNC_VISITS.with(|visits| visits.set(visits.get() + visited as u64));
        (visited, 0)
    }

    /// The log's limit: twice the parts held and the instances the last walk of its world visited, so the log overflows only
    /// after as many distinct instances changed (each is logged once until taken) as the walk it forces visits
    fn limit(&self) -> usize {
        (self.len() * 2).max(self.walked).max(4096)
    }

    /// The index made from a walk of `model`'s world: how many instances the walk looked at and how many parts it put in.
    fn rebuild(&mut self, state: &mut State, engine: &Engine, names: Names, model: InstanceId) -> (usize, usize) {
        #[cfg(test)]
        MODEL_WALKS.with(|walks| walks.set(walks.get() + 1));
        self.model = Some(model);
        self.entries.clear();
        self.parts.clear();
        self.free_entries.clear();
        self.by_part.clear();
        self.loose.clear();
        self.loose_at.clear();
        self.waiting.clear();
        self.waits_seen = waits_seen(state, model);
        self.tree.clear();
        self.walk.clear();
        self.walk.extend(state.dom.at(model).children.iter().rev());
        let mut walked = 0;
        while let Some(node) = self.walk.pop() {
            walked += 1;
            let class = {
                let instance = state.dom.at(node);
                if !engine.reflection.class(instance.class).is_a(names.world_root) {
                    self.walk.extend(instance.children.iter().rev());
                }
                instance.class
            };
            if is_part(engine, names, class) {
                self.put(state, engine, names, node);
            }
        }
        self.walked = walked;
        state.dom.follow_world(model, self.limit());
        (walked, self.len())
    }

    /// Its entries brought in step with its world's wait (camera.rs `Cameras::unindexed_models`) once that may have changed other
    /// than as its log shows (the physics step, or its own wait's count in `Cameras::models_waits`, moved on: [`waits_seen`], so
    /// another world's intake does not make it look): an entry whose part waits now (its WorldModel came back into the
    /// DataModel) leaves the tree for the waiting entries, and a waiting entry whose part no longer waits (taken in) is placed by
    /// its bounds.
    fn take_waiting(&mut self, state: &State, model: InstanceId) {
        let seen = waits_seen(state, model);
        if self.waits_seen == seen {
            return;
        }
        self.waits_seen = seen;
        #[cfg(test)]
        MODEL_WAIT_LOOKS.with(|looks| looks.set(looks.get() + 1));
        let still = state.cameras.unindexed_models.get(&model);
        for &part in still.into_iter().flatten() {
            if let Some(&entry) = self.by_part.get(&part)
                && self.entries[entry as usize].leaf != WAITS
            {
                self.unplace(entry);
                self.place(entry, None, true);
            }
        }
        if self.waiting.is_empty() {
            return;
        }
        let mut waiting = std::mem::take(&mut self.waiting);
        waiting.retain(|&entry| {
            if still.is_some_and(|still| still.contains(&self.parts[entry as usize])) {
                return true;
            }
            self.entries[entry as usize].leaf = NONE;
            let bounds = bounds(&self.entries[entry as usize].solid);
            self.place(entry, bounds, false);
            false
        });
        self.waiting = waiting;
    }

    /// Puts `part` in, or reads it again: its solid, flags and bounds.
    fn put(&mut self, state: &State, engine: &Engine, names: Names, part: InstanceId) {
        let solid = super::query::solid(state, engine, part);
        let not_false = |name: Option<NameId>| name.and_then(|n| state.dom.boolean(part, n)) != Some(false);
        let (can_query, can_collide) = (not_false(names.can_query), not_false(names.can_collide));
        let bounds = bounds(&solid);
        let waits = self.model.and_then(|model| state.cameras.unindexed_models.get(&model)).is_some_and(|waiting| waiting.contains(&part));
        let entry = match self.by_part.get(&part).copied() {
            Some(entry) => {
                let old = self.entries[entry as usize];
                let same = if waits {
                    old.leaf == WAITS
                } else {
                    old.leaf != NONE && old.leaf != WAITS && bounds.is_some_and(|(lo, hi)| {
                        let node = self.tree.node(old.leaf);
                        node.lo == lo && node.hi == hi
                    })
                };
                if !same {
                    self.unplace(entry);
                }
                self.entries[entry as usize] = Entry { solid, can_query, can_collide, leaf: if same { old.leaf } else { NONE }, nested: false };
                if same {
                    return;
                }
                entry
            }
            None => {
                let fresh = Entry { solid, can_query, can_collide, leaf: NONE, nested: false };
                let entry = match self.free_entries.pop() {
                    Some(entry) => {
                        self.entries[entry as usize] = fresh;
                        self.parts[entry as usize] = part;
                        entry
                    }
                    None => {
                        self.entries.push(fresh);
                        self.parts.push(part);
                        (self.entries.len() - 1) as u32
                    }
                };
                self.by_part.insert(part, entry);
                entry
            }
        };
        self.place(entry, bounds, waits);
    }

    /// Puts `entry` (in none of them) in the tree by `bounds`, in the loose list without, or among the waiting entries.
    fn place(&mut self, entry: u32, bounds: Option<(Vec3, Vec3)>, waits: bool) {
        if waits {
            self.entries[entry as usize].leaf = WAITS;
            self.waiting.insert(entry);
            return;
        }
        match bounds {
            Some((lo, hi)) => self.entries[entry as usize].leaf = self.tree.insert(entry, lo, hi),
            None => push_loose(&mut self.loose, &mut self.loose_at, entry),
        }
    }

    /// Takes `entry` out of the tree, the loose list (in a step, by its place there) or the waiting entries.
    fn unplace(&mut self, entry: u32) {
        let leaf = self.entries[entry as usize].leaf;
        if leaf == WAITS {
            self.waiting.remove(&entry);
        } else if leaf != NONE {
            self.tree.remove(leaf);
        } else {
            take_loose(&mut self.loose, &mut self.loose_at, entry);
        }
        self.entries[entry as usize].leaf = NONE;
    }

    /// Takes `part` out, if it is in.
    fn remove(&mut self, part: InstanceId) {
        let Some(entry) = self.by_part.remove(&part) else { return };
        self.unplace(entry);
        self.free_entries.push(entry);
    }

    /// [`Index::visit_box`] for this world: `visit` is given each part whose bounds may meet the box `center` +- `extent` (every
    /// part when it is not finite), but those that wait for the spatial index, in no particular order, until it returns false.
    /// Whether it was given them all.
    pub fn visit_box(&mut self, center: Vec3, extent: Vec3, mut visit: impl FnMut(InstanceId, &Entry) -> bool) -> bool {
        let extent = extent.abs();
        let finite = center.is_finite() && extent.is_finite() && (center + extent).is_finite() && (center - extent).is_finite();
        let (entries, parts) = (&self.entries, &self.parts);
        let mut given = 0usize;
        let mut give = |entry: u32| {
            given += 1;
            visit(parts[entry as usize], &entries[entry as usize])
        };
        // (every part: the whole tree)
        let (low, high) = if finite {
            let extent = extent * 1.001 + slack(center.abs().max_element() + extent.max_element());
            (center - extent, center + extent)
        } else {
            (Vec3::NAN, Vec3::NAN)
        };
        let all = self.tree.walk(&mut self.stack, |lo, hi| !finite || (lo.cmple(high).all() && hi.cmpge(low).all()), &mut give)
            && self.loose.iter().all(|&entry| give(entry));
        taken(given);
        all
    }

    /// Every part it holds with its solid and flags, and whether it waits out of the tree (tests: against a walk of its world)
    #[cfg(test)]
    pub(crate) fn all_parts(&self) -> Vec<(InstanceId, Entry, bool)> {
        self.by_part.iter().map(|(&part, &entry)| (part, self.entries[entry as usize], self.entries[entry as usize].leaf == WAITS)).collect()
    }

    /// Whether its tree's links, boxes and heights hold, and its loose list holds each entry outside the tree and the waiting
    /// entries once, at the place it keeps for it (tests)
    #[cfg(test)]
    pub(crate) fn check_tree(&self) -> bool {
        let mut loose_ok = self.loose.iter().enumerate().all(|(place, &entry)| self.loose_at[entry as usize] as usize == place && self.entries[entry as usize].leaf == NONE);
        loose_ok &= self.by_part.values().filter(|&&entry| self.entries[entry as usize].leaf == NONE).count() == self.loose.len();
        let tree = &self.tree;
        if tree.root == NONE {
            return loose_ok;
        }
        let mut ok = loose_ok && tree.node(tree.root).parent == NONE;
        let mut stack = vec![tree.root];
        while let Some(index) = stack.pop() {
            let node = tree.node(index);
            if node.right == NONE {
                ok &= self.entries[node.entry as usize].leaf == index;
                continue;
            }
            for child in [node.left, node.right] {
                let c = tree.node(child);
                ok &= c.parent == index && c.lo.cmpge(node.lo).all() && c.hi.cmple(node.hi).all();
                stack.push(child);
            }
            ok &= node.height == 1 + tree.node(node.left).height.max(tree.node(node.right).height);
        }
        ok
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
