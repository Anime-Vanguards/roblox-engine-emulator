//! Spatial queries of the Workspace's parts: `WorldRoot:Raycast`, `Blockcast`, `Spherecast`,
//! `Shapecast`, `GetPartBoundsInBox`, `GetPartBoundsInRadius`, `GetPartsInPart` and the legacy
//! `FindPartOnRay`, `FindPartOnRayWithIgnoreList`, `FindPartOnRayWithWhitelist` (the Region3 queries and
//! ArePartsTouchingOthers: region.rs).
//!
//! Fidelity: parts are their collision shapes where Roblox's are simple (Block a box, Ball a
//! sphere of the smallest axis, Cylinder along X, a wedge and a corner wedge their own: world/convex.rs; a TrussPart is its box,
//! measured), and their oriented bounding box otherwise (MeshParts, unions: no mesh geometry is loaded). Terrain holds
//! no voxels and is never hit. GetPartBoundsInBox, GetPartBoundsInRadius and the Region3 queries meet a wedge's box (measured). Casts follow Roblox's documented rules: nothing a ray starts inside
//! or a shape starts overlapping is hit, parts with CanQuery and CanCollide both off are skipped (CanQuery off alone hides
//! nothing: measured), RespectCanCollide skips parts with CanCollide off, and a part whose collision group does not collide with
//! the params' CollisionGroup is skipped ([`Filter::groups_pass`]). A
//! Blockcast against a ball or cylinder uses its bounding box; a Spherecast against a box meets the box grown by the radius
//! with rounded edges and corners ([`sweep_sphere_box`]); a cast touching a part only at its start or its end misses it; a
//! Blockcast's hit Position is where a client puts it ([`box_contact`]). Overlap queries
//! test each part's oriented box (a ball's sphere) against the query box or sphere.
//!
//! Filters ([`Filter`]): RaycastParams / OverlapParams carry FilterType with
//! FilterDescendantsInstances, and since April 2026 ExcludeInstances and IncludeInstances (Roblox's
//! announcement and reference); a part is seen when both admit it. Shapecast takes its documented
//! cframeOverride and sizeOverride.

use std::cell::RefCell;
use std::rc::Rc;

use glam::{Affine3A, Mat3, Vec3};
use lune_roblox::datatypes::types as lune;
use mlua::prelude::*;
use rbx_dom_weak::types as dom;

use super::broadphase::{self, Entry};
use super::convex;
use super::grid::{self, Passed, Walked};
use super::{cframe_arg, cframe_name, frame, vector_arg};
use crate::{
    Engine, InstanceId, K, NameId, State,
    lua::{Args, Flow, enum_item_value, instance_id, instance_value, raise, values},
    reflection::EnumItemId,
    services::MethodFn,
    value::Value,
};

/// A part's query geometry: its frame (unit axes, centre) and shape.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// Half extents along the part's axes
    Box(Vec3),
    Ball(f32),
    /// Along the part's X axis: half length, radius
    Cylinder(f32, f32),
    /// A wedge's or a corner wedge's own shape (world/convex.rs), its box's half extents
    Wedge(Vec3),
    CornerWedge(Vec3),
    /// A TrussPart: its box, but a ray entering it through an edge or touching it ([`ray_truss`])
    Truss(Vec3),
}

#[derive(Clone, Copy, Debug)]
pub struct Solid {
    pub part: InstanceId,
    pub frame: Affine3A,
    pub shape: Shape,
}

impl Solid {
    /// The oriented box that bounds it (its half extents).
    pub fn half_extents(&self) -> Vec3 {
        match self.shape {
            Shape::Box(half) | Shape::Truss(half) => half,
            Shape::Ball(radius) => Vec3::splat(radius),
            Shape::Cylinder(half_length, radius) => Vec3::new(half_length, radius, radius),
            Shape::Wedge(half) | Shape::CornerWedge(half) => half,
        }
    }

    /// Its hull, for a wedge or a corner wedge
    pub(crate) fn hull(&self) -> Option<convex::Hull> {
        match self.shape {
            Shape::Wedge(half) => Some(convex::Hull::new(convex::Kind::Wedge, half)),
            Shape::CornerWedge(half) => Some(convex::Hull::new(convex::Kind::CornerWedge, half)),
            _ => None,
        }
    }

    /// The convex shape a box query, a Blockcast or a Shapecast meets: a wedge's hull, else its box (a ball's and a cylinder's
    /// too, as a Blockcast meets them)
    pub(crate) fn polytope(&self) -> convex::Polytope {
        match self.hull() {
            Some(hull) => convex::Polytope::of_hull(&self.frame, &hull),
            None => convex::Polytope::of_box(&self.frame, self.half_extents()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub part: InstanceId,
    pub position: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// Which parts a query sees (RaycastParams / OverlapParams): a part passes both of the params'
/// filters, FilterType with FilterDescendantsInstances and ExcludeInstances with IncludeInstances.
/// A part must pass both, as on a playing client (measured, Studio 0.747, client and server: every combination of FilterType,
/// FilterDescendantsInstances and the two lists through GetPartBoundsInBox and Raycast, tools/parity/probes/query_filter_combine.luau).
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// FilterType Include (or the legacy Whitelist): FilterDescendantsInstances lists what is seen
    pub include: bool,
    /// FilterDescendantsInstances, sorted, without repeats
    pub instances: Vec<InstanceId>,
    /// Each of `instances`' first place among the list's instances, the order a client checks them in
    positions: Vec<u32>,
    /// How many instances FilterDescendantsInstances holds, repeats counted
    scanned: usize,
    /// The list read with each instance's place (storage kept between queries)
    pairs: Vec<(InstanceId, u32)>,
    /// ExcludeInstances
    pub exclude_instances: Listed,
    /// IncludeInstances
    pub include_instances: Listed,
    pub respect_can_collide: bool,
    pub max_parts: usize,
    /// Parts with CanQuery and CanCollide both off are seen too (the PlayerMouse's view of the Workspace: world/mouse.rs); never set
    /// from a query's params
    pub unqueryable: bool,
    /// The groups the params' CollisionGroup collides with, as bits by id, and the parts' CollisionGroupId (physics.rs
    /// query_mask); None: every group
    pub group_mask: Option<(u32, NameId)>,
}

/// An instance list that may be nil (`{Instance}?`): `given` false for nil; the instances sorted,
/// without repeats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listed {
    pub given: bool,
    pub ids: Vec<InstanceId>,
}

impl Listed {
    /// `value` read into this list (its storage kept): a table's array part, or a lone instance,
    /// as the legacy list reads; anything else is nil. How many entries it read.
    fn read(&mut self, value: &LuaValue) -> usize {
        self.given = matches!(value, LuaValue::Table(_)) || instance_id(value).is_some();
        read_instances(value, &mut self.ids)
    }
}

/// What a query pays for reading its params' lists (FilterDescendantsInstances, ExcludeInstances, IncludeInstances), as a client
/// reads them at every query: cost.rs PARAMS_LIST for each list that holds anything and PARAMS_LISTED for each entry it holds.
fn list_reads(lists: [usize; 3]) -> f64 {
    lists.into_iter().filter(|&entries| entries > 0).map(|entries| crate::cost::PARAMS_LIST + entries as f64 * crate::cost::PARAMS_LISTED).sum()
}

impl Filter {
    /// Whether the part `id` passes. FilterDescendantsInstances must list the part or an ancestor
    /// exactly when FilterType is Include. Of the part and then each ancestor in turn, the first that
    /// ExcludeInstances or IncludeInstances lists decides, ExcludeInstances first when both list it;
    /// with none listed the part passes unless IncludeInstances is given (nil lets everything through,
    /// `{}` nothing). That is the announcement's own sample (devforum.roblox.com/t/4563575), except
    /// its last line, `return include ~= nil`, which inverts what its text says of nil and `{}`.
    fn admits(&self, state: &State, id: InstanceId) -> bool {
        self.check(state, id, false).0
    }

    /// [`Filter::admits`]'s answer, and with `costed` what a client spends on deciding it (else 0): for FilterDescendantsInstances,
    /// a walk up from the part for each instance of the list in its order until one is the part or an ancestor of it, every walk
    /// before it to the top of the tree, each step at cost.rs filter_step for the part's depth; for ExcludeInstances and
    /// IncludeInstances, one walk up until either list names an instance, each instance looked at a cost.rs listed_step for each
    /// list that holds any.
    pub(crate) fn check(&self, state: &State, id: InstanceId, costed: bool) -> (bool, f64) {
        let (exclude, include) = (&self.exclude_instances.ids, &self.include_instances.ids);
        let lists = !exclude.is_empty() || !include.is_empty();
        let scanning = !self.instances.is_empty();
        // (the first instance of FilterDescendantsInstances, in the list's order, that is the part or an ancestor: its place and
        // how far up it is)
        let mut first: Option<(u32, u32)> = None;
        let (mut decided, mut looked) = (None, 0u32);
        let (mut current, mut up) = (Some(id), 0u32);
        while let Some(node) = current {
            if scanning
                && let Ok(at) = self.instances.binary_search(&node)
                && first.is_none_or(|(place, _)| self.positions[at] < place)
            {
                first = Some((self.positions[at], up));
            }
            if lists && decided.is_none() {
                looked += 1;
                if exclude.binary_search(&node).is_ok() {
                    decided = Some(false);
                } else if include.binary_search(&node).is_ok() {
                    decided = Some(true);
                }
            }
            // (nothing further up can change either answer, nor, once the list's first instance is found, what it costs; else
            // the walk goes on to the top, as a client's check of each instance before the first it finds does)
            let scanned = !scanning || if costed { first.is_some_and(|(place, _)| place == 0) } else { first.is_some() };
            if scanned && (decided.is_some() || !lists) {
                break;
            }
            current = state.dom.at(node).parent;
            up += u32::from(current.is_some());
        }
        let admitted = first.is_some() == self.include && decided.unwrap_or(!self.include_instances.given);
        if !costed {
            return (admitted, 0.0);
        }
        let mut seconds = 0.0;
        if scanning {
            // (past the top, `up` is how many ancestors the part has: what a walk that finds nothing takes)
            let steps = match first {
                Some((place, at)) => f64::from(place) * f64::from(up) + f64::from(at),
                None => self.scanned as f64 * f64::from(up),
            };
            seconds += steps * crate::cost::filter_step(up);
        }
        if lists {
            let step: f64 = [exclude, include].iter().filter(|ids| !ids.is_empty()).map(|ids| crate::cost::listed_step(ids.len())).sum();
            seconds += f64::from(looked) * step;
        }
        (admitted, seconds)
    }

    /// Whether a query with this filter sees the indexed part `part` (world/broadphase.rs): CanQuery and CanCollide off left out (unless the
    /// view takes such parts), CanCollide off left out with RespectCanCollide, and what [`Filter::admits`] says.
    pub(crate) fn sees(&self, state: &State, part: InstanceId, entry: &Entry) -> bool {
        self.flags_pass(entry) && self.groups_pass(state, part) && self.admits(state, part)
    }

    /// Whether the indexed part's flags let a query with this filter see it, before its lists are asked: CanQuery or CanCollide (unless the view
    /// takes parts with it off) and CanCollide with RespectCanCollide.
    fn flags_pass(&self, entry: &Entry) -> bool {
        (self.unqueryable || entry.can_query) && (!self.respect_can_collide || entry.can_collide)
    }

    /// Whether the part's collision group (its CollisionGroupId, read when the params' group does not collide with every group)
    /// collides with the params' (a part in a group that is not registered collides with all)
    fn groups_pass(&self, state: &State, part: InstanceId) -> bool {
        let Some((mask, name)) = self.group_mask else { return true };
        match state.dom.raw_get(part, name) {
            Some(Value::Number(id)) if (0.0..32.0).contains(id) => mask & (1u32 << (*id as u32)) != 0,
            _ => true,
        }
    }

    /// RaycastParams / OverlapParams (a prelude proxy of the type `kind`; nil for the default params) read into this filter, its
    /// lists' storage kept: a query reading its params allocates nothing.
    fn read(&mut self, engine: &Engine, world: InstanceId, kind: Params, params: &LuaValue) {
        let data = params_data(engine, kind, params);
        // (a proxy's stored fields are a plain table: raw reads, without mlua's protected call)
        let get = |key: &str| match &data {
            Some(data) => data.raw_get::<LuaValue>(key).unwrap_or(LuaValue::Nil),
            None => LuaValue::Nil,
        };
        self.unqueryable = false;
        // FilterType Exclude by default, Include also for the legacy Whitelist
        self.include = match get("FilterType") {
            LuaValue::UserData(ud) => ud
                .borrow::<crate::lua::types::LEnumItem>()
                .map(|item| matches!(&*engine.reflection.enum_item_info(item.0).name, "Include" | "Whitelist"))
                .unwrap_or(false),
            _ => false,
        };
        self.max_parts = match get("MaxParts") {
            LuaValue::Number(n) if n > 0.0 => n as usize,
            LuaValue::Integer(n) if n > 0 => n as usize,
            _ => 0,
        };
        let descendants = self.read_scanned(&get("FilterDescendantsInstances"));
        let excluded = self.exclude_instances.read(&get("ExcludeInstances"));
        let included = self.include_instances.read(&get("IncludeInstances"));
        self.respect_can_collide = matches!(get("RespectCanCollide"), LuaValue::Boolean(true));
        // (the group the params name, Default when none are given, in the queried world's groups)
        let (group_mask, named) = match get("CollisionGroup") {
            LuaValue::String(name) => super::super::services::physics::query_mask(engine, world, &name.as_bytes()),
            _ => super::super::services::physics::query_mask(engine, world, b"Default"),
        };
        self.group_mask = group_mask;
        charge_query(engine, list_reads([descendants, excluded, included]) + if named { crate::cost::QUERY_PARAMS_GROUP } else { 0.0 });
    }

    /// FilterDescendantsInstances (or a legacy list) into `instances` and `positions`; how many entries it read.
    fn read_scanned(&mut self, value: &LuaValue) -> usize {
        let pairs = &mut self.pairs;
        pairs.clear();
        let mut read = 0;
        match value {
            // (from 1 to its length, nil holes skipped: a legacy list as a client reads it, measured; a params list holds no holes)
            LuaValue::Table(table) => {
                for index in 1..=table.raw_len() {
                    let value = table.raw_get::<LuaValue>(index).unwrap_or(LuaValue::Nil);
                    if value.is_nil() {
                        continue;
                    }
                    read += 1;
                    if let Some(id) = instance_id(&value) {
                        pairs.push((id, pairs.len() as u32));
                    }
                }
            }
            other => {
                pairs.extend(instance_id(other).map(|id| (id, 0)));
                read = pairs.len();
            }
        }
        self.scanned = pairs.len();
        // (each instance once, at its first place)
        pairs.sort_unstable();
        pairs.dedup_by_key(|(id, _)| *id);
        self.instances.clear();
        self.instances.extend(pairs.iter().map(|(id, _)| *id));
        self.positions.clear();
        self.positions.extend(pairs.iter().map(|(_, place)| *place));
        read
    }

    /// A legacy FindPartOnRay filter: `list` (an ignore list, or with `include` a whitelist) alone.
    fn read_list(&mut self, engine: &Engine, world: InstanceId, list: &LuaValue, include: bool) {
        // (the Default group's: measured, FindPartOnRay misses a part in a group Default does not collide with)
        self.group_mask = super::super::services::physics::query_mask(engine, world, b"Default").0;
        self.include = include;
        self.read_scanned(list);
        self.exclude_instances.read(&LuaValue::Nil);
        self.include_instances.read(&LuaValue::Nil);
        self.respect_can_collide = false;
        self.max_parts = 0;
    }
}

/// The shape `part` collides as (unit-scale frame; the size is in the shape).
pub fn solid(state: &State, engine: &Engine, part: InstanceId) -> Solid {
    let transform = frame::affine_of(state.dom.raw_get(part, cframe_name(engine)));
    let frame = unit_frame(&transform);
    let size = match state.dom.raw_get(part, K::Size as NameId) {
        Some(Value::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
        _ => Vec3::ONE,
    };
    Solid { part, frame, shape: shape_of(state, engine, part, size) }
}

/// The shape `part` collides as at `size`: a Block part a box, a Ball a sphere of the smallest axis,
/// a Cylinder a cylinder along X, a WedgePart and a Part whose Shape is Wedge a wedge, a CornerWedgePart and a Part whose Shape
/// is CornerWedge a corner wedge, anything else its bounding box.
fn shape_of(state: &State, engine: &Engine, part: InstanceId, size: Vec3) -> Shape {
    let reflection = &engine.reflection;
    let half = (size * 0.5).max(Vec3::ZERO);
    let class = state.dom.at(part).class;
    let shape = reflection.name_id("Shape").filter(|_| reflection.is_a(class, "Part")).and_then(|n| state.dom.raw_get(part, n)).and_then(Value::as_enum).map(|item| &*reflection.enum_item_info(item).name);
    match shape {
        Some("Ball") => Shape::Ball(half.min_element()),
        Some("Cylinder") => Shape::Cylinder(half.x, half.y.min(half.z)),
        Some("Wedge") => Shape::Wedge(half),
        Some("CornerWedge") => Shape::CornerWedge(half),
        Some(_) => Shape::Box(half),
        None if reflection.is_a(class, "WedgePart") => Shape::Wedge(half),
        None if reflection.is_a(class, "CornerWedgePart") => Shape::CornerWedge(half),
        None if reflection.is_a(class, "TrussPart") => Shape::Truss(half),
        None => Shape::Box(half),
    }
}

/// What a spatial query reuses on an engine: the last query's filter, and the storage of the parts the last walk gave
/// (world/broadphase.rs holds each world's grid in the engine's state)
#[derive(Default)]
struct QueryCache {
    /// The last query's filter, whose lists the next query reads its params into ([`take_filter`])
    spare: RefCell<Filter>,
    /// The parts the last box walk gave, whose storage the next one fills ([`walk_box`])
    near: RefCell<Vec<(InstanceId, Entry)>>,
    /// The parts and cells the last ray walk gave ([`walk_ray`])
    rays: RefCell<RayParts>,
}

/// A ray walk's parts, each with its entry and the index of the first cell holding it, and the cells it passed
type RayParts = (Vec<(InstanceId, Entry, u32)>, Vec<Passed>);

impl crate::Extension for QueryCache {}

fn query_cache(engine: &Engine) -> &QueryCache {
    engine.extension_or_insert_with(QueryCache::default)
}

/// A query's filter, read from its params into the spare filter's storage; [`keep_filter`] hands it
/// back for the next query.
fn take_filter(engine: &Engine, world: InstanceId, kind: Params, params: &LuaValue) -> Filter {
    let mut filter = query_cache(engine).spare.take();
    filter.read(engine, world, kind, params);
    filter
}

fn keep_filter(engine: &Engine, filter: Filter) {
    *query_cache(engine).spare.borrow_mut() = filter;
}

/// What a spatial query of the Workspace does first ([`take_in_of`]); its world's index.
pub(crate) fn take_in(engine: &Engine) -> Option<u32> {
    take_in_of(engine, engine.world().workspace)
}

/// What a spatial query of the WorldRoot `world` does first: the index of its world (the Workspace's, or a WorldModel's own)
/// takes in what came into it since a query of it or the physics step last took that world's in, in the order it came
/// (world/broadphase.rs take_in: GetLargestCutoffDistance and the PlayerMouse, which take nothing in, see it from then), which
/// the query pays for as a client's does (cost.rs QUERY_TAKE_IN and QUERY_TAKEN_PART for each part; moved and removed parts cost
/// it nothing). A client keeps an index for each world, and a WorldModel's query takes in its own world's new parts and leaves
/// the Workspace's waiting (and the other way round: measured, world/region.rs), at the same price. Its world's index, none for a
/// WorldModel outside the DataModel.
pub(crate) fn take_in_of(engine: &Engine, world: InstanceId) -> Option<u32> {
    let (index, waiting, opened) = broadphase::take_in(engine, world);
    if waiting > 0 {
        charge_query(engine, crate::cost::QUERY_TAKE_IN + waiting as f64 * crate::cost::QUERY_TAKEN_PART);
    }
    charge_query(engine, opened);
    index
}

/// The Workspace's index brought up to the dom's changes (world/broadphase.rs open), the spatial index's wait left as it is: the
/// PlayerMouse's reads and GetLargestCutoffDistance take nothing in (world/mouse.rs, world/camera_moves.rs).
pub(crate) fn refresh(engine: &Engine) -> Option<u32> {
    broadphase::open(engine, engine.world().workspace).0
}

/// The Workspace's parts but Terrain, a WorldModel's in it included (world/broadphase.rs placed_parts), into `out` in no particular
/// order: what a collision-group change works out the ids of (services/physics.rs).
pub(crate) fn placed_parts(engine: &Engine, out: &mut Vec<InstanceId>) {
    broadphase::placed_parts(engine, out);
}

/// The parts of `world`'s box walk over `lo`..`hi` (the query's world box, clamped: [`clamped`]) in its order, a cell's `first`
/// list first, to `then` with what the walk did.
pub(crate) fn walk_box<R>(engine: &Engine, world: u32, lo: Vec3, hi: Vec3, first: u8, then: impl FnOnce(&[(InstanceId, Entry)], Walked) -> R) -> R {
    let cache = query_cache(engine);
    let mut near = cache.near.take();
    near.clear();
    let walked = broadphase::walk_box(&mut engine.state.borrow_mut(), world, lo, hi, first, &mut near);
    let result = then(&near, walked);
    near.clear();
    *cache.near.borrow_mut() = near;
    result
}

/// Every part of `world`'s index that is filed or loose, in no particular order, to `then` (a query whose geometry has no box)
fn every_part<R>(engine: &Engine, world: u32, then: impl FnOnce(&[(InstanceId, Entry)]) -> R) -> R {
    let cache = query_cache(engine);
    let mut near = cache.near.take();
    near.clear();
    broadphase::all_parts(&engine.state.borrow(), world, &mut near);
    let result = then(&near);
    near.clear();
    *cache.near.borrow_mut() = near;
    result
}

/// The parts of `world`'s ray walk along the segment from `origin` along `direction` in its order (world/grid.rs walk_ray: level 0
/// first, each level's cells in the order the segment enters them, a cell's anchored parts first), each with the index of the
/// first cell of the walk's cells that holds it, to `then` with those cells.
pub(crate) fn walk_ray<R>(engine: &Engine, world: u32, origin: Vec3, direction: Vec3, then: impl FnOnce(&[(InstanceId, Entry, u32)], &[Passed]) -> R) -> R {
    let cache = query_cache(engine);
    let (mut near, mut passed) = cache.rays.take();
    near.clear();
    passed.clear();
    broadphase::walk_ray(&mut engine.state.borrow_mut(), world, origin, direction, &mut near, &mut passed);
    let result = then(&near, &passed);
    near.clear();
    *cache.rays.borrow_mut() = (near, passed);
    result
}

/// How far about its centre a box query reaches on each axis: a client clamps the query's world box to its centre +- 7,500
/// studs, for its walk and its overlap test alike (measured, Studio 0.747, a playing client and a server: parts 7,500 studs out
/// along a 20,000-stud box found, 7,500.5 not; a 40,000-stud box turned 45 degrees about y finds parts out to 10,606 along its
/// own axis; a radius of 10,000 alike; tools/parity/probes/grid_limits.luau)
pub(crate) const CLAMP: f32 = 7500.0;

/// A Raycast's and FindPartOnRay's reach: a client cuts every ray 15,000 studs from its origin, whatever its length and
/// direction and wherever it starts (measured, Studio 0.747: a face 14,999.9 along a 1,000,000-stud ray hit, one at 15,000 not;
/// the same from 50,000 studs back; tools/parity/probes/grid_limits.luau)
pub(crate) const RAY_CUT: f32 = 15000.0;

/// A query's world box `center` +- `extent`, clamped to its centre +- [`CLAMP`] on each axis, and whether the clamp cut it
pub(crate) fn clamped(center: Vec3, extent: Vec3) -> (Vec3, Vec3, bool) {
    let reach = extent.min(Vec3::splat(CLAMP));
    (center - reach, center + reach, reach != extent)
}

/// A ray's direction cut to [`RAY_CUT`] (a hair short of it, so a face at the cut is not hit: measured)
fn cut(direction: Vec3) -> Vec3 {
    let length = direction.length();
    if length > RAY_CUT { direction * ((RAY_CUT - 0.002) / length) } else { direction }
}

/// Whether a solid's world box meets the box `lo`..`hi` (touching counts): a clamped query's second test
fn meets(solid: &Solid, lo: Vec3, hi: Vec3) -> bool {
    let (center, reach) = world_reach(solid);
    (center - reach).cmple(hi).all() && (center + reach).cmpge(lo).all()
}

/// Whether the world's index holds what a walk of its world finds (tests; world/broadphase.rs check_world)
#[cfg(test)]
pub(crate) fn index_matches_a_walk(engine: &Engine) -> Result<(), String> {
    broadphase::check_world(engine, engine.world().workspace)
}

/// [`index_matches_a_walk`] for the WorldModel `model`'s own index (tests)
#[cfg(test)]
pub(crate) fn model_index_matches_a_walk(engine: &Engine, model: InstanceId) -> Result<(), String> {
    broadphase::check_world(engine, model)
}

/// How many parts the Workspace's index holds (tests)
#[cfg(test)]
pub(crate) fn index_len(engine: &Engine) -> usize {
    let workspace = engine.world().workspace;
    let world = engine.state.borrow().spatial.world(workspace);
    world.map_or(0, |world| broadphase::held(&engine.state.borrow(), world))
}

/// Whether a ray or a shape cast's path from `origin` along `direction` can meet anything: both finite, and the direction's length
/// too. A playing client's Raycast and FindPartOnRay with a NaN or infinite origin or direction, or a direction whose length
/// overflows a float (1e20 studs), find nothing and test no part (Studio 0.747, client and server: 0.17 us beside 0 or 5,000
/// parts, cheaper than a miss; tools/parity/probes/query_finite.luau); a shape cast with a NaN direction alike.
fn finite_path(origin: Vec3, direction: Vec3) -> bool {
    origin.is_finite() && direction.is_finite() && direction.length().is_finite()
}

/// Whether every number of a frame is finite (its rotation and its position).
fn finite_frame(frame: &Affine3A) -> bool {
    frame.matrix3.x_axis.is_finite() && frame.matrix3.y_axis.is_finite() && frame.matrix3.z_axis.is_finite() && frame.translation.is_finite()
}

/// The largest radius a Spherecast takes, the largest side a Blockcast's or Shapecast's box takes and the longest path a shape cast
/// takes (measured, Studio 0.747, a playing client and a server: 256 and 512 pass, 256.001 and 512.001 raise; 1,024 studs pass,
/// 1,025 raise; a Raycast has no such limit, 1,000,000 studs hit)
const SPHERECAST_RADIUS: f32 = 256.0;
const SHAPECAST_SIDE: f32 = 512.0;
const SHAPECAST_DISTANCE: f32 = 1024.0;

/// A shape cast's direction as a client checks it: an infinite component raises (a NaN passes, and the cast then finds nothing),
/// and so does a path longer than [`SHAPECAST_DISTANCE`] (its length as a float32 written `%f`).
fn shapecast_direction(direction: Vec3) -> Result<(), Flow> {
    if direction.x.is_infinite() || direction.y.is_infinite() || direction.z.is_infinite() {
        return raise("Invalid direction. Must contain finite values.");
    }
    let distance = direction.length();
    if distance > SHAPECAST_DISTANCE {
        return raise(format!("Attempt to shapecast with distance {:.6}. The maximum distance is 1024.", f64::from(distance)));
    }
    Ok(())
}

/// A Blockcast's or Shapecast's box size as a client checks it: every component finite, none past [`SHAPECAST_SIDE`] (the first
/// such, x then y then z, written `%f`); a negative side passes.
fn shapecast_size(size: Vec3) -> Result<(), Flow> {
    if !size.is_finite() {
        return raise("Invalid size. Must contain finite values.");
    }
    match size.to_array().into_iter().find(|side| *side > SHAPECAST_SIDE) {
        Some(side) => raise(format!("Attempt to shapecast with side length {:.6}. The maximum side length is 512.000000.", f64::from(side))),
        None => Ok(()),
    }
}

/// A shape cast's frame as a client checks it, after its size and direction: every number finite.
fn shapecast_frame(frame: &Affine3A) -> Result<(), Flow> {
    if finite_frame(frame) { Ok(()) } else { raise("Invalid CFrame. Must contain finite values.") }
}

/// A GetPartBoundsInBox size as a client reads it: a NaN or negative side is 0 (measured: a box of size (NaN, 1, 1) or (-inf, 1, 1)
/// at a part's centre finds it and one beside it nothing, as a box of no width does; (-4, 1, 1) beside a face finds nothing where
/// (4, 1, 1) finds the part), and an infinite side reaches without end, held as [`UNBOUNDED`].
fn box_size(size: Vec3) -> Vec3 {
    Vec3::from_array(size.to_array().map(|side| if side > 0.0 { side.min(UNBOUNDED) } else { 0.0 }))
}

/// How far an infinite side of a GetPartBoundsInBox reaches: finite, so that a box's tests multiply it by an axis's zero
/// component to 0 (infinity's product is NaN)
const UNBOUNDED: f32 = 1e30;

/// The world box around an oriented box (its frame's axes as given, made unit, `size` its size): `|R| * size / 2`, which also
/// bounds a skewed frame's box; None when it is not finite.
fn box_extent(frame: &Affine3A, size: Vec3) -> Option<Vec3> {
    let unit = unit_frame(frame);
    let half = (size * 0.5).abs();
    let m = unit.matrix3;
    let extent = Vec3::from(m.x_axis).abs() * half.x + Vec3::from(m.y_axis).abs() * half.y + Vec3::from(m.z_axis).abs() * half.z;
    extent.is_finite().then_some(extent)
}

/// A solid's centre and the half size of its world-aligned box: its collision box turned into the world, `|R| * half` (float32).
pub(crate) fn world_reach(solid: &Solid) -> (Vec3, Vec3) {
    let m = solid.frame.matrix3;
    let half = solid.half_extents();
    let reach = Vec3::new(
        m.x_axis.x.abs() * half.x + m.y_axis.x.abs() * half.y + m.z_axis.x.abs() * half.z,
        m.x_axis.y.abs() * half.x + m.y_axis.y.abs() * half.y + m.z_axis.y.abs() * half.z,
        m.x_axis.z.abs() * half.x + m.y_axis.z.abs() * half.y + m.z_axis.z.abs() * half.z,
    );
    (frame::translation(&solid.frame), reach)
}

/// Where a ray from `origin` along `direction` (its length the reach) first enters `solid`, or a sphere of radius `grow` swept along
/// it first touches it: (fraction of the direction, local normal), or None (a ray starting inside or on its surface does not hit,
/// nor one only reaching it at its end). A swept sphere meets a box as the box grown by its radius with rounded edges and corners
/// ([`sweep_sphere_box`]), a ball or cylinder grown by it.
pub(crate) fn ray_solid(solid: &Solid, origin: Vec3, direction: Vec3, grow: f32) -> Option<(f32, Vec3)> {
    // a ray passing beyond the solid's bounding sphere misses it, found before the frame's inverse (most of a map's solids)
    let center = frame::translation(&solid.frame);
    let reach = solid.half_extents().length() + grow;
    let length_squared = direction.length_squared();
    let along = if length_squared > 0.0 { ((center - origin).dot(direction) / length_squared).clamp(0.0, 1.0) } else { 0.0 };
    if (origin + direction * along - center).length_squared() > reach * reach {
        return None;
    }
    let inverse = solid.frame.inverse();
    let o = inverse.transform_point3(origin);
    let d = inverse.transform_vector3(direction);
    match solid.shape {
        Shape::Box(half) | Shape::Truss(half) if grow > 0.0 => sweep_sphere_box(o, d, half, grow),
        Shape::Box(half) => ray_box(o, d, half + Vec3::splat(grow)),
        Shape::Truss(half) => ray_truss(o, d, half),
        Shape::Ball(radius) => ray_sphere(o, d, radius + grow),
        Shape::Cylinder(half_length, radius) => ray_cylinder(o, d, half_length + grow, radius + grow),
        Shape::Wedge(_) | Shape::CornerWedge(_) => {
            let hull = solid.hull()?;
            if grow > 0.0 { hull.sweep_sphere(o, d, grow) } else { hull.ray(o, d) }
        }
    }
}

/// Slab test against a box centred at the origin; the entry must be past the start and short of the end: a ray starting on a face
/// does not hit it, nor does one whose end is on it (measured, Studio 0.747, client and server: tools/parity/probes/query_edges.luau).
/// A ray along a face's plane or an edge hits the face it reaches.
pub fn ray_box(o: Vec3, d: Vec3, half: Vec3) -> Option<(f32, Vec3)> {
    let (mut enter, mut exit) = (f32::NEG_INFINITY, f32::INFINITY);
    let mut normal = Vec3::ZERO;
    for axis in 0..3 {
        let (origin, direction, extent) = (o[axis], d[axis], half[axis]);
        if direction.abs() < 1e-12 {
            if origin.abs() > extent {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((-extent - origin) / direction, (extent - origin) / direction);
        let mut sign = -1.0;
        if near > far {
            std::mem::swap(&mut near, &mut far);
            sign = 1.0;
        }
        if near > enter {
            enter = near;
            normal = Vec3::ZERO;
            normal[axis] = sign;
        }
        exit = exit.min(far);
        if enter > exit {
            return None;
        }
    }
    (enter > 0.0 && enter < 1.0 && enter <= exit).then_some((enter, normal))
}

/// A ray against a TrussPart (its box centred at the origin, half extents `half`): a client's ray meets a truss as its box, but one
/// entering it exactly through an edge or a corner is given the ray's own direction reversed as its normal, and one only touching
/// an edge misses (measured, Studio 0.747, client and server, tools/parity/probes/query_wedges.luau: a ray down and along +Z onto a
/// truss's top front edge reads Normal (0, 0.70711, -0.70711), one along (-25, -20, 25) onto a side edge (0.615, 0.492, -0.615),
/// rays grazing its bottom front and top back edges nil).
pub fn ray_truss(o: Vec3, d: Vec3, half: Vec3) -> Option<(f32, Vec3)> {
    let (mut enter, mut exit) = (f32::NEG_INFINITY, f32::INFINITY);
    let (mut normal, mut tied) = (Vec3::ZERO, false);
    for axis in 0..3 {
        let (origin, direction, extent) = (o[axis], d[axis], half[axis]);
        if direction.abs() < 1e-12 {
            if origin.abs() > extent {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((-extent - origin) / direction, (extent - origin) / direction);
        let mut sign = -1.0;
        if near > far {
            std::mem::swap(&mut near, &mut far);
            sign = 1.0;
        }
        if near > enter {
            tied = enter.is_finite() && near - enter <= 1e-6 * near.abs().max(1.0);
            enter = near;
            normal = Vec3::ZERO;
            normal[axis] = sign;
        } else if enter - near <= 1e-6 * near.abs().max(1.0) {
            tied = true;
        }
        exit = exit.min(far);
    }
    if !(enter > 0.0 && enter < 1.0 && enter < exit) {
        return None;
    }
    Some((enter, if tied { -d.normalize_or(Vec3::Y) } else { normal }))
}

/// Whether a cast's entry fraction `t` hits: past the start and short of the end (a cast touching a part only at its start or only
/// at its end does not hit it: measured)
fn within<T: PartialOrd + From<u8>>(t: T) -> bool {
    t > T::from(0) && t < T::from(1)
}

/// A sphere of `radius` swept from `o` along `d` against a box centred at the origin with half extents `half`: the box grown by the
/// radius with its edges and corners rounded (its sum with the sphere), as a playing client's Spherecast meets a box (measured,
/// Studio 0.747, client and server, query_edges.luau: a sphere passing a top edge hits where it touches the edge, its Normal from
/// the edge to its centre, its Position the edge's point). The first contact's fraction and normal; none when the sphere starts
/// touching or overlapping the box.
pub fn sweep_sphere_box(o: Vec3, d: Vec3, half: Vec3, radius: f32) -> Option<(f32, Vec3)> {
    // (in doubles: a contact found by a quadratic loses digits in floats that a client's does not)
    let (o, d, half, radius) = (o.as_dvec3(), d.as_dvec3(), half.as_dvec3(), f64::from(radius));
    if (o - o.clamp(-half, half)).length_squared() <= radius * radius {
        return None;
    }
    let mut best: Option<f64> = None;
    let mut keep = |t: f64| {
        if within(t) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    };
    // the faces, grown by the radius
    for axis in 0..3 {
        if d[axis] == 0.0 {
            continue;
        }
        let sign = if d[axis] < 0.0 { 1.0 } else { -1.0 };
        let t = (sign * (half[axis] + radius) - o[axis]) / d[axis];
        let p = o + d * t;
        let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
        if p[i].abs() <= half[i] && p[j].abs() <= half[j] {
            keep(t);
        }
    }
    // the edges (a cylinder of the radius about each)
    for axis in 0..3 {
        let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
        let a = d[i] * d[i] + d[j] * d[j];
        if a <= 0.0 {
            continue;
        }
        for (si, sj) in [(-1.0f64, -1.0f64), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            let (ci, cj) = (o[i] - si * half[i], o[j] - sj * half[j]);
            let b = ci * d[i] + cj * d[j];
            let c = ci * ci + cj * cj - radius * radius;
            let discriminant = b * b - a * c;
            if discriminant < 0.0 {
                continue;
            }
            let t = (-b - discriminant.sqrt()) / a;
            if (o[axis] + d[axis] * t).abs() <= half[axis] {
                keep(t);
            }
        }
    }
    // the corners (a sphere of the radius about each)
    let a = d.length_squared();
    for corner in 0..8 {
        let at = glam::DVec3::new(if corner & 1 == 0 { -half.x } else { half.x }, if corner & 2 == 0 { -half.y } else { half.y }, if corner & 4 == 0 { -half.z } else { half.z });
        let from = o - at;
        let b = from.dot(d);
        let discriminant = b * b - a * (from.length_squared() - radius * radius);
        if a > 0.0 && discriminant >= 0.0 {
            keep((-b - discriminant.sqrt()) / a);
        }
    }
    let t = best?;
    let center = o + d * t;
    Some((t as f32, (center - center.clamp(-half, half)).normalize_or(glam::DVec3::Y).as_vec3()))
}

pub fn ray_sphere(o: Vec3, d: Vec3, radius: f32) -> Option<(f32, Vec3)> {
    let a = d.length_squared();
    if a < 1e-24 {
        return None;
    }
    let b = o.dot(d);
    let c = o.length_squared() - radius * radius;
    if c < 0.0 {
        return None;
    }
    let discriminant = b * b - a * c;
    if discriminant < 0.0 {
        return None;
    }
    let t = (-b - discriminant.sqrt()) / a;
    within(t).then(|| (t, (o + d * t).normalize_or(Vec3::Y)))
}

/// A cylinder along X centred at the origin: its side or a cap.
pub fn ray_cylinder(o: Vec3, d: Vec3, half_length: f32, radius: f32) -> Option<(f32, Vec3)> {
    let inside = o.x.abs() <= half_length && o.y * o.y + o.z * o.z <= radius * radius;
    if inside {
        return None;
    }
    let mut best: Option<(f32, Vec3)> = None;
    let mut consider = |t: f32, normal: Vec3| {
        if within(t) && best.is_none_or(|(b, _)| t < b) {
            best = Some((t, normal));
        }
    };
    // side
    let a = d.y * d.y + d.z * d.z;
    if a > 1e-24 {
        let b = o.y * d.y + o.z * d.z;
        let c = o.y * o.y + o.z * o.z - radius * radius;
        let discriminant = b * b - a * c;
        if discriminant >= 0.0 {
            let t = (-b - discriminant.sqrt()) / a;
            let p = o + d * t;
            if p.x.abs() <= half_length {
                consider(t, Vec3::new(0.0, p.y, p.z).normalize_or(Vec3::Y));
            }
        }
    }
    // caps
    if d.x.abs() > 1e-12 {
        for sign in [-1.0f32, 1.0] {
            let t = (sign * half_length - o.x) / d.x;
            let p = o + d * t;
            if p.y * p.y + p.z * p.z <= radius * radius && o.x * sign > half_length {
                consider(t, Vec3::new(sign, 0.0, 0.0));
            }
        }
    }
    best
}

/// The closest hit of a ray (or a sphere of radius `grow` swept along it) among the parts of `near` that `sees` lets through, but
/// `except`'s: the parts it enters in the order its walk of the grid gave them, the nearest kept, the first of equals (a client's
/// cast takes the first of exact ties in its walk: measured, tools/parity/probes/grid_order.luau "ties").
pub fn cast_ray<'a>(near: impl IntoIterator<Item = (InstanceId, &'a Entry)>, sees: impl Fn(InstanceId, &Entry) -> bool, origin: Vec3, direction: Vec3, grow: f32, except: Option<InstanceId>) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for (part, entry) in near {
        if except == Some(part) {
            continue;
        }
        let Some((t, local_normal)) = ray_solid(&entry.solid, origin, direction, grow) else { continue };
        let hull = entry.solid.hull().is_some();
        // (a client reads a wedge's Distance off the hit's float position: 10.79999 for a hit at z 296.79999 from 286)
        let distance = if hull && grow == 0.0 { (origin + direction * t - origin).length() } else { direction.length() * t };
        if best.is_some_and(|hit| hit.distance <= distance) || !sees(part, entry) {
            continue;
        }
        let normal = entry.solid.frame.transform_vector3(local_normal).normalize_or(Vec3::Y);
        // a swept sphere touches the surface one radius short of its centre (worked in doubles against a wedge's shape, whose
        // contact a client puts at the float nearest its own)
        let position = if hull && grow > 0.0 {
            (origin.as_dvec3() + direction.as_dvec3() * f64::from(t) - normal.as_dvec3() * f64::from(grow)).as_vec3()
        } else {
            origin + direction * t - normal * grow
        };
        best = Some(Hit { part, position, normal, distance });
    }
    best
}

/// The axes and half extents of an oriented box.
pub(super) struct Oriented {
    pub(super) center: Vec3,
    axes: [Vec3; 3],
    half: Vec3,
}

impl Oriented {
    pub(super) fn of(frame: &Affine3A, half: Vec3) -> Self {
        Oriented {
            center: frame::translation(frame),
            axes: [Vec3::from(frame.matrix3.x_axis), Vec3::from(frame.matrix3.y_axis), Vec3::from(frame.matrix3.z_axis)],
            half,
        }
    }

    pub(super) fn radius(&self, axis: Vec3) -> f32 {
        (0..3).map(|i| self.axes[i].dot(axis).abs() * self.half[i]).sum()
    }
}

/// The separating axes of two boxes (their own and the cross products).
pub(super) fn sat_axes(a: &Oriented, b: &Oriented) -> Vec<Vec3> {
    let mut axes = Vec::with_capacity(15);
    axes.extend_from_slice(&a.axes);
    axes.extend_from_slice(&b.axes);
    for x in &a.axes {
        for y in &b.axes {
            let cross = x.cross(*y);
            if cross.length_squared() > 1e-10 {
                axes.push(cross.normalize());
            }
        }
    }
    axes
}

/// Whether two oriented boxes overlap (touching counts).
fn boxes_overlap(a: &Oriented, b: &Oriented) -> bool {
    let offset = b.center - a.center;
    sat_axes(a, b).into_iter().all(|axis| offset.dot(axis).abs() <= a.radius(axis) + b.radius(axis))
}

/// Box `a` moving along `direction` against box `b`: the fraction of the direction at first
/// contact and the contact normal (from `b` towards `a`); None when it never touches or already
/// overlaps at the start.
fn sweep_boxes(a: &Oriented, direction: Vec3, b: &Oriented) -> Option<(f32, Vec3)> {
    let (mut enter, mut exit) = (f32::NEG_INFINITY, f32::INFINITY);
    let mut normal = Vec3::ZERO;
    for axis in sat_axes(a, b) {
        let reach = a.radius(axis) + b.radius(axis);
        let start = (a.center - b.center).dot(axis);
        let speed = direction.dot(axis);
        if speed.abs() < 1e-12 {
            if start.abs() > reach {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((-reach - start) / speed, (reach - start) / speed);
        let (near, far) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
        if near > enter {
            enter = near;
            // (+ 0: no negative zero in a component, which a client's normal does not hold)
            normal = if speed > 0.0 { -axis } else { axis } + Vec3::ZERO;
        }
        exit = exit.min(far);
        if enter > exit {
            return None;
        }
    }
    within(enter).then_some((enter, normal))
}

/// Where a Blockcast's box (`moving` at its contact, `signed` its half extents as given: a side may be negative) touches `other`
/// across `normal`, as a playing client reports it (measured, Studio 0.747, client and server: query_edges.luau, 40 boxes on and
/// beside a part's faces): on the face of `other` that the normal leaves, and along each of that face's axes at the point that
/// divides the two centres as the boxes' half extents do (`a + ha (b - a) / (ha + hb)`: a 1-stud box 4.8 studs off the centre of
/// a 10-stud face touches 4.3636 off it), at the face's edge when both boxes' far edges coincide there, and at the box's centre
/// plus its negative half where a side is negative. None when the normal is no face axis of `other` (an edge or the box's own
/// face: its centre less its reach is reported then). A box turned against the face it hits is reported as its projected half
/// extents give it, which a client's differs from by up to a few tenths.
fn box_contact(moving: &Oriented, signed: Vec3, normal: Vec3, other: &Oriented) -> Option<Vec3> {
    let face = (0..3).find(|&k| other.axes[k].dot(normal).abs() > 1.0 - 1e-5)?;
    let offset = moving.center - other.center;
    let mut position = other.center;
    for (j, axis) in other.axes.iter().enumerate() {
        let a = offset.dot(*axis);
        let hb = other.half[j];
        let along = if j == face {
            hb.copysign(axis.dot(normal))
        } else {
            let (mut ha, mut negative, mut inverted) = (0.0f32, 0.0f32, false);
            for (i, own) in moving.axes.iter().enumerate() {
                // (a negative side of an axis lying in the face puts the point at the box's centre plus that side, along the axis:
                // measured, turned boxes too; one across the face, tilted onto an edge, counts as its size)
                if signed[i] < 0.0 && own.dot(normal).abs() < 1e-5 && own.dot(*axis).abs() > 1e-5 {
                    negative += own.dot(*axis) * signed[i];
                    inverted = true;
                } else {
                    ha += own.dot(*axis).abs() * signed[i].abs();
                }
            }
            if inverted {
                a + negative
            } else if a + ha == hb {
                hb
            } else if ha + hb > 0.0 {
                a - ha * a / (ha + hb)
            } else {
                a
            }
        };
        position += *axis * along;
    }
    Some(position)
}

/// The closest part of `near` that `sees` lets through a box (`frame`, `size`) swept along `direction` touches, `except` aside:
/// the parts it touches in the order its walk of the grid gave them, the nearest kept, the first of equals.
pub fn cast_box(near: &[(InstanceId, Entry)], sees: impl Fn(InstanceId, &Entry) -> bool, frame: &Affine3A, size: Vec3, direction: Vec3, except: Option<InstanceId>) -> Option<Hit> {
    // (a negative side sweeps as its size: measured, the distance of a cast down with size (1, -2, 1) is a 2-stud box's)
    let moving = Oriented::of(&unit_frame(frame), (size * 0.5).abs());
    // (a wedge's or corner wedge's own shape is met by separating axes: world/convex.rs)
    let shape = convex::Polytope::of_box(&unit_frame(frame), size * 0.5);
    let mut best: Option<Hit> = None;
    for (part, entry) in near {
        if except == Some(*part) {
            continue;
        }
        if entry.solid.hull().is_some() {
            let other = entry.solid.polytope();
            let Some((t, normal)) = convex::sweep(&shape, direction, &other) else { continue };
            let distance = (direction.as_dvec3().length() * t) as f32;
            if best.is_some_and(|hit| hit.distance <= distance) || !sees(*part, entry) {
                continue;
            }
            let position = convex::contact(&shape.moved(direction.as_dvec3() * t), normal, &other);
            best = Some(Hit { part: *part, position, normal: normal.as_vec3(), distance });
            continue;
        }
        let other = Oriented::of(&entry.solid.frame, entry.solid.half_extents());
        let Some((t, normal)) = sweep_boxes(&moving, direction, &other) else { continue };
        let distance = direction.length() * t;
        if best.is_some_and(|hit| hit.distance <= distance) || !sees(*part, entry) {
            continue;
        }
        let center = moving.center + direction * t;
        let at = Oriented { center, ..moving };
        let position = box_contact(&at, size * 0.5, normal, &other).unwrap_or(center - normal * moving.radius(normal));
        best = Some(Hit { part: *part, position, normal, distance });
    }
    best
}

/// The closest part of `near` that `sees` lets through a wedge's or corner wedge's shape `moving` swept along `direction` touches,
/// `except` aside (a Shapecast of such a part): each part's convex shape ([`Solid::polytope`]: a box for anything but a wedge) met
/// by separating axes, the nearest kept, the first of equals.
pub(crate) fn cast_polytope(near: &[(InstanceId, Entry)], sees: impl Fn(InstanceId, &Entry) -> bool, moving: &convex::Polytope, direction: Vec3, except: Option<InstanceId>) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for (part, entry) in near {
        if except == Some(*part) {
            continue;
        }
        let other = entry.solid.polytope();
        let Some((t, normal)) = convex::sweep(moving, direction, &other) else { continue };
        let distance = (direction.as_dvec3().length() * t) as f32;
        if best.is_some_and(|hit| hit.distance <= distance) || !sees(*part, entry) {
            continue;
        }
        let position = convex::contact(&moving.moved(direction.as_dvec3() * t), normal, &other);
        best = Some(Hit { part: *part, position, normal: normal.as_vec3(), distance });
    }
    best
}

fn unit_frame(frame: &Affine3A) -> Affine3A {
    let m: Mat3 = frame.matrix3.into();
    Affine3A::from_mat3_translation(Mat3::from_cols(m.x_axis.normalize_or(Vec3::X), m.y_axis.normalize_or(Vec3::Y), m.z_axis.normalize_or(Vec3::Z)), frame::translation(frame))
}

/// Whether `solid` overlaps the box (`frame`, `size`).
pub fn overlaps_box(solid: &Solid, frame: &Affine3A, size: Vec3) -> bool {
    let query = Oriented::of(&unit_frame(frame), size * 0.5);
    match solid.shape {
        Shape::Ball(radius) => {
            let local = unit_frame(frame).inverse().transform_point3(frame::translation(&solid.frame));
            let closest = local.clamp(-query.half, query.half);
            (local - closest).length() <= radius
        }
        _ => boxes_overlap(&Oriented::of(&solid.frame, solid.half_extents()), &query),
    }
}

/// Whether `solid` overlaps the sphere (`center`, `radius`).
pub fn overlaps_sphere(solid: &Solid, center: Vec3, radius: f32) -> bool {
    match solid.shape {
        Shape::Ball(own) => (frame::translation(&solid.frame) - center).length() <= own + radius,
        _ => {
            let local = solid.frame.inverse().transform_point3(center);
            let half = solid.half_extents();
            (local - local.clamp(-half, half)).length() <= radius
        }
    }
}

/// Whether `solid` overlaps the part `query`, each as its own shape (GetPartsInPart: a wedge's and a corner wedge's own, measured;
/// a ball's sphere; anything else its box)
pub fn overlaps_part(solid: &Solid, query: &Solid) -> bool {
    match (solid.hull(), query.hull()) {
        (None, None) => match query.shape {
            Shape::Ball(radius) => overlaps_sphere(solid, frame::translation(&query.frame), radius),
            _ => overlaps_box(solid, &query.frame, query.half_extents() * 2.0),
        },
        (Some(hull), _) if matches!(query.shape, Shape::Ball(_)) => ball_meets_hull(&hull, &solid.frame, query),
        (_, Some(hull)) if matches!(solid.shape, Shape::Ball(_)) => ball_meets_hull(&hull, &query.frame, solid),
        _ => convex::overlap(&solid.polytope(), &query.polytope()),
    }
}

/// Whether the ball `ball` meets `hull` in `frame`
fn ball_meets_hull(hull: &convex::Hull, frame: &Affine3A, ball: &Solid) -> bool {
    let Shape::Ball(radius) = ball.shape else { return false };
    hull.signed_distance(frame.inverse().transform_point3(frame::translation(&ball.frame))) <= radius
}

// Script bindings

// (a query that raises costs what a client spends on the error where ree raises it: lua/cfunction.rs charge_raised)
fn method(f: impl Fn(&Lua, &Rc<Engine>, InstanceId, &Args) -> Result<LuaMultiValue, Flow> + 'static) -> MethodFn {
    Rc::new(f)
}

/// The fields of a RaycastParams / OverlapParams: what a prelude proxy of the type `kind` stores (the prelude's `ParamsData`,
/// one table for each type, read without the proxy's copies of its lists), None for any other value.
fn params_data(engine: &Engine, kind: Params, params: &LuaValue) -> Option<LuaTable> {
    let LuaValue::UserData(ud) = params else { return None };
    let stored: LuaTable = engine.prelude().exports.raw_get("ParamsData").ok()?;
    let stored: LuaTable = stored.raw_get(kind.name()).ok()?;
    stored.raw_get::<Option<LuaTable>>(ud.clone()).ok().flatten()
}

/// The params type a query takes
#[derive(Clone, Copy, PartialEq)]
enum Params {
    Raycast,
    Overlap,
}

impl Params {
    fn name(self) -> &'static str {
        match self {
            Params::Raycast => "RaycastParams",
            Params::Overlap => "OverlapParams",
        }
    }
}

// A query's arguments as a playing client casts them (measured, Studio 0.747, a client and a server alike:
// tools/parity/probes/query_args.luau): each value cast to its parameter's type, the casts' order the method's own: from the
// last to the first for the casts (Raycast, Blockcast, Spherecast, Shapecast) and the FindPartOnRay family, as the reflection's
// methods take them, from the first to the last for the overlap queries (GetPartBoundsInBox, GetPartBoundsInRadius,
// GetPartsInPart, whose part is checked before its params). A Vector3, CFrame, float, Ray or Instances
// parameter left out or nil raises "Argument <n> missing or nil"; a params parameter or a bool takes nil as its default; an
// Instance parameter takes nil as no instance (raising "Argument <n> missing or nil" only when the call leaves it out). A value of
// another type raises "Unable to cast <type> to <Type>", the type as camera.rs cast_name names it ("double", "Array",
// "CoordinateFrame", "OverlapParams"), "Unable to cast value to Object" for an Instance parameter. The method's own checks of the
// values (a part that is no BasePart, a shape cast's limits) come after every cast. A call that raises costs a client what an
// engine error costs (lua/cfunction.rs charge_raised).

/// A Vector3 argument.
fn vector3(engine: &Engine, args: &Args, at: usize) -> Result<Vec3, Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    vector_arg(value).map_or_else(|| raise(format!("Unable to cast {} to Vector3", super::camera::cast_name(engine, value))), Ok)
}

/// A CFrame argument.
fn frame_argument(engine: &Engine, args: &Args, at: usize) -> Result<Affine3A, Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    cframe_arg(value).map_or_else(|| raise(format!("Unable to cast {} to CoordinateFrame", super::camera::cast_name(engine, value))), Ok)
}

/// An optional argument of a type `cast` reads: nil is None (Shapecast's cframeOverride and sizeOverride).
fn optional<T>(engine: &Engine, args: &Args, at: usize, kind: &str, cast: impl Fn(&LuaValue) -> Option<T>) -> Result<Option<T>, Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return Ok(None);
    }
    cast(value).map_or_else(|| raise(format!("Unable to cast {} to {kind}", super::camera::cast_name(engine, value))), |value| Ok(Some(value)))
}

/// A RaycastParams or OverlapParams argument: nil (the default params), or a params object of that type.
fn params_argument(engine: &Engine, args: &Args, at: usize, kind: Params) -> Result<LuaValue, Flow> {
    let value = args.get(at);
    if value.is_nil() || params_data(engine, kind, value).is_some() {
        return Ok(value.clone());
    }
    raise(format!("Unable to cast {} to {}", super::camera::cast_name(engine, value), kind.name()))
}

/// An Instance argument: nil or an Instance; one the call leaves out raises.
fn object_argument(args: &Args, at: usize) -> Result<Option<InstanceId>, Flow> {
    if args.len() <= at {
        return raise(format!("Argument {at} missing or nil"));
    }
    optional_object(args, at)
}

/// An Instance argument that may be left out (FindPartOnRay's ignored instance)
fn optional_object(args: &Args, at: usize) -> Result<Option<InstanceId>, Flow> {
    match args.get(at) {
        LuaValue::Nil => Ok(None),
        value => instance_id(value).map_or_else(|| raise("Unable to cast value to Object"), |id| Ok(Some(id))),
    }
}

/// A bool argument (FindPartOnRay's terrainCellsAreCubes and ignoreWater, which ree's world has no use for): nil, a boolean, a
/// number, or the string "true" or "false" (measured: "1" raises "Unable to cast string to bool").
fn bool_argument(engine: &Engine, args: &Args, at: usize) -> Result<(), Flow> {
    match args.get(at) {
        LuaValue::Nil | LuaValue::Boolean(_) | LuaValue::Number(_) | LuaValue::Integer(_) => Ok(()),
        LuaValue::String(text) if matches!(text.as_bytes().as_ref(), b"true" | b"false") => Ok(()),
        value => raise(format!("Unable to cast {} to bool", super::camera::cast_name(engine, value))),
    }
}

/// A Ray argument: its origin and direction
fn ray_argument(engine: &Engine, args: &Args, at: usize) -> Result<(Vec3, Vec3), Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    let converted = match value {
        LuaValue::UserData(ud) => ud.borrow::<lune::Ray>().ok().map(|ray| dom::Ray::from(*ray)),
        _ => None,
    };
    let Some(converted) = converted else { return raise(format!("Unable to cast {} to Ray", super::camera::cast_name(engine, value))) };
    let mut origin = Vec3::new(converted.origin.x, converted.origin.y, converted.origin.z);
    let mut direction = Vec3::new(converted.direction.x, converted.direction.y, converted.direction.z);
    // (the conversion rounds each component's fraction, which makes an infinity NaN: a ray that is not finite is read back
    // through its members, so its end keeps the infinity a client's does)
    if !(origin.is_finite() && direction.is_finite())
        && let LuaValue::UserData(ud) = value
        && let (Some(own_origin), Some(own_direction)) = (ud.get::<LuaValue>("Origin").ok().and_then(|v| vector_arg(&v)), ud.get::<LuaValue>("Direction").ok().and_then(|v| vector_arg(&v)))
    {
        (origin, direction) = (own_origin, own_direction);
    }
    Ok((origin, direction))
}

/// The part a Shapecast or GetPartsInPart is given, after the casts: a BasePart, else `message`
fn base_part(engine: &Engine, part: Option<InstanceId>, message: &str) -> Result<InstanceId, Flow> {
    let state = engine.state.borrow();
    match part.and_then(|id| state.dom.get(id).map(|instance| (id, instance.class))) {
        Some((id, class)) if engine.reflection.is_a(class, "BasePart") => Ok(id),
        _ => raise(message.to_string()),
    }
}

/// The instances of an instance list (a table's array part, or a lone instance) into `ids`, sorted,
/// without repeats (the filters search them); how many entries it read.
fn read_instances(value: &LuaValue, ids: &mut Vec<InstanceId>) -> usize {
    ids.clear();
    let mut read = 0;
    match value {
        LuaValue::Table(table) => ids.extend(table.sequence_values::<LuaValue>().filter_map(|v| v.ok()).inspect(|_| read += 1).filter_map(|v| instance_id(&v))),
        other => {
            ids.extend(instance_id(other));
            read = ids.len();
        }
    }
    ids.sort_unstable();
    ids.dedup();
    read
}

fn material_of(engine: &Engine, part: InstanceId) -> Option<EnumItemId> {
    let name = engine.reflection.name_id("Material")?;
    engine.state.borrow().dom.raw_get(part, name).and_then(Value::as_enum)
}

fn vector_lua(v: Vec3) -> LuaValue {
    LuaValue::Vector(mlua::Vector::new(v.x, v.y, v.z))
}

/// A RaycastResult (a read-only prelude proxy, typeof "RaycastResult": prelude.luau RaycastResult, its members, tostring and
/// equality).
fn result_value(lua: &Lua, engine: &Rc<Engine>, hit: &Hit) -> LuaResult<LuaValue> {
    let make: LuaFunction = engine.prelude().exports.raw_get("RaycastResult")?;
    let data = lua.create_table()?;
    data.set("Instance", instance_value(lua, engine, hit.part)?)?;
    data.set("Position", vector_lua(hit.position))?;
    data.set("Normal", vector_lua(hit.normal))?;
    data.set("Distance", hit.distance as f64)?;
    if let Some(material) = material_of(engine, hit.part) {
        data.set("Material", enum_item_value(lua, engine, material)?)?;
    }
    make.call(data)
}

fn hit_values(lua: &Lua, engine: &Rc<Engine>, hit: Option<Hit>) -> Result<LuaMultiValue, Flow> {
    Ok(match hit {
        Some(hit) => values([result_value(lua, engine, &hit)?]),
        None => values([LuaValue::Nil]),
    })
}

/// A cast whose path can meet nothing ([`finite_path`]): nil, no part tested. It takes in what came into `world`'s world, as
/// any query does ([`take_in_of`]).
fn missed(lua: &Lua, engine: &Rc<Engine>, world: InstanceId, filter: Filter) -> Result<LuaMultiValue, Flow> {
    take_in_of(engine, world);
    keep_filter(engine, filter);
    hit_values(lua, engine, None)
}

/// An overlap query whose geometry can find nothing (not finite): an empty table, no part tested, what came into `world`'s
/// world taken in.
fn found_nothing(lua: &Lua, engine: &Rc<Engine>, world: InstanceId, filter: Filter) -> Result<LuaMultiValue, Flow> {
    take_in_of(engine, world);
    keep_filter(engine, filter);
    parts_table(lua, engine, std::iter::empty())
}

pub(super) fn parts_table(lua: &Lua, engine: &Rc<Engine>, parts: impl IntoIterator<Item = InstanceId>) -> Result<LuaMultiValue, Flow> {
    let table = lua.create_table()?;
    for part in parts {
        table.raw_push(instance_value(lua, engine, part)?)?;
    }
    Ok(values([LuaValue::Table(table)]))
}

/// The legacy FindPartOnRay family: (part, position, normal, material); the ray's end, a zero
/// normal and Air when nothing is hit. The ray is cut at [`RAY_CUT`] as a Raycast's is (measured), and its end on a miss is the
/// ray's own. Its arguments, cast as a client casts them ([`vector3`] and the rest): FindPartOnRay(ray, ignoreDescendantsInstance?,
/// terrainCellsAreCubes?, ignoreWater?), FindPartOnRayWithIgnoreList(ray, ignoreDescendantsTable, terrainCellsAreCubes?,
/// ignoreWater?) and FindPartOnRayWithWhitelist(ray, whitelistDescendantsTable, ignoreWater?), a list read as world/mod.rs
/// objects_arg takes one (nil holes skipped, a table in it "Unable to cast value to Object": measured).
fn find_part_on_ray(lua: &Lua, engine: &Rc<Engine>, world: InstanceId, args: &Args, listed: bool, include: bool) -> Result<LuaMultiValue, Flow> {
    let bools = if include { 3 } else { 4 };
    for at in (3..=bools).rev() {
        bool_argument(engine, args, at)?;
    }
    let mut entries = None;
    let list = if listed {
        let table = super::objects_arg(args, 2)?;
        entries = Some(table.raw_len());
        for index in 1..=table.raw_len() {
            let entry: LuaValue = table.raw_get(index).map_err(Flow::from)?;
            if !entry.is_nil() && instance_id(&entry).is_none() {
                return raise("Unable to cast value to Object");
            }
        }
        LuaValue::Table(table)
    } else {
        optional_object(args, 2)?;
        args.get(2).clone()
    };
    let (origin, direction) = ray_argument(engine, args, 1)?;
    let mut filter = query_cache(engine).spare.take();
    filter.read_list(engine, world, &list, include);
    // (its call and its list cost a client more than a Raycast's; its walk, the parts it meets and its filter's checks as a
    // Raycast's: cost.rs FIND_PART_ON_RAY)
    charge_query(engine, crate::cost::FIND_PART_ON_RAY + entries.map_or(0.0, |entries| crate::cost::FIND_PART_ON_RAY_LIST + entries as f64 * crate::cost::FIND_PART_ON_RAY_LISTED));
    // (a ray that is not finite finds nothing and tests no part: its end is still origin + direction, an infinity kept)
    let hit = if finite_path(origin, direction) {
        raycast(engine, world, &filter, origin, direction)
    } else {
        take_in_of(engine, world);
        None
    };
    keep_filter(engine, filter);
    let air = engine.reflection.enum_item("Material", "Air");
    Ok(match hit {
        Some(hit) => {
            let material = match material_of(engine, hit.part) {
                Some(item) => enum_item_value(lua, engine, item)?,
                None => LuaValue::Nil,
            };
            values([instance_value(lua, engine, hit.part)?, vector_lua(hit.position), vector_lua(hit.normal), material])
        }
        None => values([
            LuaValue::Nil,
            vector_lua(origin + direction),
            vector_lua(Vec3::ZERO),
            match air {
                Some(item) => enum_item_value(lua, engine, item)?,
                None => LuaValue::Nil,
            },
        ]),
    })
}

/// The first `max_parts` of `parts` (OverlapParams.MaxParts; 0: all).
fn limited(max_parts: usize, parts: Vec<InstanceId>) -> Vec<InstanceId> {
    if max_parts > 0 { parts.into_iter().take(max_parts).collect() } else { parts }
}

/// What a cast pays for the parts on its path, as a playing client's does (cost.rs RAY_MET, RAY_CROSSED; roblox-sim, Studio 0.747,
/// a headless client): the parts it meets up to its hit (`hit`, the distance along `direction`), whatever its filter admits, since
/// the engine's broadphase meets them before the filter is asked, and nothing for the parts behind the hit, where a client's cast
/// stops (24 walls 2 studs apart behind the hit cost a Raycast 0.05 us over the hit alone, a Spherecast or Blockcast 0.07, filtered
/// out or not; 10 studs apart nothing). The first part met pays RAY_MET; each other part the filter leaves out pays RAY_CROSSED,
/// whatever the cast's shape (23 excluded walls before the hit 0.027 to 0.031 us each for a Raycast, a Spherecast, a Blockcast and
/// a Shapecast alike); each other part it admits, one tied with the hit (a stack), pays `each`, the cast's own test (a Spherecast
/// 0.09 us, a Blockcast 0.12, a Raycast 0.023 a coincident wall). A miss pays for every part its path crosses. `met` is
/// [`cast_counts`]'. The first part met pays `first`: RAY_MET for a Raycast, SPHERE_MET and BLOCK_MET for the shapes (a shape's hit
/// costs a client more over its miss than a ray's).
fn charge_cast(engine: &Engine, (met, admitted): (usize, usize), first: f64, each: f64, kinds: u8) {
    if met > 0 {
        let tied = admitted.saturating_sub(1);
        let seconds = first + tied as f64 * each + (met - 1 - tied) as f64 * crate::cost::RAY_CROSSED;
        charge_query(engine, seconds);
        engine.state.borrow().sched.touched.borrow().book_kinds(kinds, seconds);
    }
}

/// How far past a cast's hit, or beside its path, a client's cast still asks its filter about a part: the part's bounds grown by
/// this much each way (measured, Studio 0.747, a headless playing client, two sessions: walls 200 folders deep behind an Exclude list
/// of 50 parts elsewhere, so each check is a long walk; a wall entered 0.30 or 0.31 studs past the hit is asked about, one at 0.32
/// or more is not, whatever the walls' thickness, the hit's distance (5 to 500 studs) or the cast's length (60 to 2,000), for a
/// Raycast, a Spherecast and a Blockcast alike; a bar beside the path 0.1 or 0.2 studs from it is asked about, at 0.4 not).
pub(crate) const FILTER_MARGIN: f32 = 0.315;

/// How often a client's ray asks its filter about a part it passes close to: once for every [`FILTER_STEP`] studs of its path
/// inside the part's bounds grown by [`FILTER_MARGIN`], rounded, and once more (measured as [`FILTER_MARGIN`]: a 1, 2 or 4-stud bar
/// along the ray 0.1 studs beside it asked about twice, an 8-stud one about four times wherever it lay along the path, 16 six times,
/// 32 nine, 64 seventeen; a 9 x 9 x 30 box as a 1 x 1 x 30 bar). A wall the path crosses is asked about once.
pub(crate) const FILTER_STEP: f32 = 4.0;

/// How many times a Raycast's filter is asked about the part of `entry` ([`FILTER_STEP`]), its path from `origin` along `direction`
/// up to `until` (0 to 1).
fn ray_checks(entry: &Entry, origin: Vec3, direction: Vec3, until: f32) -> u32 {
    let inverse = entry.solid.frame.inverse();
    let (o, d) = (inverse.transform_point3(origin), inverse.transform_vector3(direction));
    let half = entry.solid.half_extents() + Vec3::splat(FILTER_MARGIN);
    let (mut enter, mut exit) = (0.0f32, until);
    for axis in 0..3 {
        if d[axis].abs() < 1e-12 {
            if o[axis].abs() > half[axis] {
                return 1;
            }
            continue;
        }
        let (a, b) = ((-half[axis] - o[axis]) / d[axis], (half[axis] - o[axis]) / d[axis]);
        enter = enter.max(a.min(b));
        exit = exit.min(a.max(b));
    }
    let inside = (exit - enter).max(0.0) * direction.length();
    1 + (inside / FILTER_STEP).round() as u32
}

/// The parts among `near` a query sees (CanQuery or CanCollide on, whatever else the cast's filter says) that a cast's path meets up to
/// its hit (`hit`, the distance along the path, `length` long), and how many of those its filter admits when it hit something: what
/// [`charge_cast`] charges for; and what its filter's checks cost. `enter(entry, grow)` is where along the path (0 to 1) the cast's
/// shape first meets the part grown by `grow` (exactly: a Blockcast's box sweep, not its bounding sphere). A client asks its filter
/// about each part the path meets up to the hit and about each part whose bounds grown by [`FILTER_MARGIN`] it enters before the hit
/// (a part close behind the hit, or close beside the path; anywhere on a miss): those are checked without being met. `checks`
/// says how often the filter is asked about a part, the path ending where its argument says ([`ray_checks`] for a ray, once for a
/// shape's: not measured).
fn cast_counts<'a>(state: &State, near: impl IntoIterator<Item = (InstanceId, &'a Entry)>, filter: &Filter, hit: Option<f32>, length: f32, enter: impl Fn(&Entry, f32) -> Option<f32>, checks: impl Fn(&Entry, f32) -> u32) -> (usize, usize, f64) {
    let (mut met, mut admitted, mut walks) = (0usize, 0usize, 0.0);
    let before = |t: f32| hit.is_none_or(|distance| length * t <= distance);
    for (part, entry) in near {
        // (the parts a query with no filter sees: CanQuery and CanCollide off left out)
        if !entry.can_query {
            continue;
        }
        // (the hit's own distance is reckoned the same way: the hit and the parts tied with it are met)
        let meets = enter(entry, 0.0).is_some_and(before);
        if !meets && !enter(entry, FILTER_MARGIN).is_some_and(before) {
            continue;
        }
        let collides = (!filter.respect_can_collide || entry.can_collide) && filter.groups_pass(state, part);
        // (a client's cast asks its filter of each part it meets or comes that close to, at what a client spends on it: Filter::check)
        let (admits, walk) = if collides { filter.check(state, part, true) } else { (false, 0.0) };
        // (a part along the path is asked about again every few studs: `checks`, given where the path stops)
        walks += if walk > 0.0 { walk * f64::from(checks(entry, hit.map_or(1.0, |distance| if length > 0.0 { distance / length } else { 0.0 }))) } else { 0.0 };
        if !meets {
            continue;
        }
        met += 1;
        if hit.is_some() && admits {
            admitted += 1;
        }
    }
    (met, admitted, walks)
}

/// A shape cast's hit and what it is charged for (Spherecast, Blockcast, Shapecast): the parts of its world's box walk over its
/// swept box `lo`..`hi` (every part of its world for none), a cell's anchored parts first (a client's shape casts take the first of
/// exact ties so: measured, grid_order "ties", where a Spherecast takes a level-1 part before a level-0 one with the same top, as
/// a box walk of its swept box starting on level 2 gives them; inferred, its walk not measured further), tested by `cast` for the
/// hit and by [`cast_counts`] for what its path (along `direction`, `enter` the shape's entry test) meets. It takes in
/// `world`'s parts ([`take_in_of`]); a WorldModel outside the DataModel has none.
#[allow(clippy::too_many_arguments)]
fn cast_and_count(engine: &Engine, world: InstanceId, filter: &Filter, direction: Vec3, enter: impl Fn(&Entry, f32) -> Option<f32>, swept: Option<(Vec3, Vec3)>, cast: impl FnOnce(&State, &[(InstanceId, Entry)]) -> Option<Hit>) -> (Option<Hit>, (usize, usize)) {
    let Some(index) = take_in_of(engine, world) else { return (None, (0, 0)) };
    let then = |near: &[(InstanceId, Entry)]| {
        let state = engine.state.borrow();
        let hit = cast(&state, near);
        let (met, admitted, walks) = cast_counts(&state, near.iter().map(|(part, entry)| (*part, entry)), filter, hit.map(|hit| hit.distance), direction.length(), &enter, |_, _| 1);
        drop(state);
        charge_query(engine, walks);
        (hit, (met, admitted))
    };
    match swept {
        Some((lo, hi)) => walk_box(engine, index, lo, hi, grid::ANCHORED, |near, _| then(near)),
        None => every_part(engine, index, then),
    }
}

/// A Raycast's hit and what it is charged for: its world's ray walk along its path cut at [`RAY_CUT`] ([`walk_ray`]), the first of
/// exact ties in the walk's order (measured: a level-0 part before a level-1 one with the same top, a cell's anchored parts before
/// its unanchored ones, each list in its order; the order across cells is inferred), and what its path meets up to its hit
/// ([`cast_counts`], [`charge_cast`]) plus what a client's walk spends on the way ([`charge_ray_walk`]). It takes in `world`'s
/// parts ([`take_in_of`]).
fn raycast(engine: &Engine, world: InstanceId, filter: &Filter, origin: Vec3, direction: Vec3) -> Option<Hit> {
    let index = take_in_of(engine, world)?;
    let reach = cut(direction);
    let (hit, counts, walk) = walk_ray(engine, index, origin, reach, |near, passed| {
        let state = engine.state.borrow();
        let parts = || near.iter().map(|(part, entry, _)| (*part, entry));
        let hit = cast_ray(parts(), |part, entry| filter.sees(&state, part, entry), origin, reach, 0.0, None);
        let (met, admitted, walks) = cast_counts(&state, parts(), filter, hit.map(|hit| hit.distance), reach.length(), |entry, grow| ray_solid(&entry.solid, origin, reach, grow).map(|(t, _)| t), |entry, until| ray_checks(entry, origin, reach, until));
        (hit, (met, admitted, walks), ray_walk(near, passed, hit, origin, reach))
    });
    charge_query(engine, counts.2);
    charge_cast(engine, (counts.0, counts.1), crate::cost::RAY_MET, crate::cost::RAY_CROSSED, 0);
    charge_ray_walk(engine, walk);
    hit
}

/// What a client's ray walk passed up to the hit (all of its path on a miss): its length there, the cells it passed that first gave
/// it a part it tested and did not meet and none it met, and those parts (a cell whose parts were all given by an earlier cell is
/// not counted, nor is one of a part it met, whose price is the part's: cost.rs RAY_MET)
#[derive(Clone, Copy, Default)]
pub(crate) struct RayWalk {
    pub length: f32,
    pub cells: usize,
    pub missed: usize,
}

/// [`RayWalk`] for a ray walk's parts and cells ([`walk_ray`]) and its hit: a part counts when the ray enters the cell that first
/// holds it before the hit
pub(crate) fn ray_walk(near: &[(InstanceId, Entry, u32)], passed: &[Passed], hit: Option<Hit>, origin: Vec3, direction: Vec3) -> RayWalk {
    let length = direction.length();
    let reach = hit.map_or(1.0, |hit| if length > 0.0 { f64::from(hit.distance / length) } else { 0.0 });
    let before = |cell: u32| passed.get(cell as usize).is_some_and(|cell| cell.enter <= reach);
    let (mut missed, mut met_cells, mut missed_cells) = (0usize, rustc_hash::FxHashSet::default(), rustc_hash::FxHashSet::default());
    for (_, entry, cell) in near {
        if !before(*cell) {
            continue;
        }
        if ray_solid(&entry.solid, origin, direction, 0.0).is_some_and(|(t, _)| f64::from(t) <= reach) {
            met_cells.insert(*cell);
        } else {
            missed += 1;
            missed_cells.insert(*cell);
        }
    }
    let cells = missed_cells.difference(&met_cells).count();
    RayWalk { length: hit.map_or(length, |hit| hit.distance), cells, missed }
}

/// What a client's ray walk costs on its way (cost.rs RAY_LENGTH, RAY_CELL, RAY_TESTED; Studio 0.747, a headless client): its
/// length up to its hit, the cells holding parts it passes with none it meets, and the parts there it tests and misses.
pub(crate) fn charge_ray_walk(engine: &Engine, walk: RayWalk) {
    charge_query(engine, f64::from(walk.length) / 1000.0 * crate::cost::RAY_LENGTH + walk.cells as f64 * crate::cost::RAY_CELL + walk.missed as f64 * crate::cost::RAY_TESTED);
}

/// What a path that crossed `crossed` parts pays: [`crate::cost::RAY_MET`] when it met any, and `each` for every part after the
/// first (GetPartsObscuringTarget's segments: world/camera.rs; the PlayerMouse's ray: world/mouse.rs).
pub(crate) fn charge_crossed(engine: &Engine, crossed: usize, each: f64, kinds: u8) {
    if crossed > 0 {
        let seconds = crate::cost::RAY_MET + (crossed - 1) as f64 * each;
        charge_query(engine, seconds);
        engine.state.borrow().sched.touched.borrow().book_kinds(kinds, seconds);
    }
}

/// Whether `target` is `part` or an ancestor of it, and how many steps up a client's walk from `part` takes to find it: to it, or
/// to the top of the tree when it is not there (the part's ancestors). The PlayerMouse's TargetFilter and the ignore lists of
/// GetLargestCutoffDistance and GetPartsObscuringTarget are checked so (world/mouse.rs, camera_moves.rs, camera.rs).
pub(crate) fn steps_up_to(state: &State, part: InstanceId, target: InstanceId) -> (bool, u32) {
    let (mut current, mut steps) = (Some(part), 0u32);
    while let Some(node) = current {
        if node == target {
            return (true, steps);
        }
        current = state.dom.at(node).parent;
        steps += u32::from(current.is_some());
    }
    (false, steps)
}

/// A spatial query's charge beyond its call's price (cost.rs QUERY_BOX_PART, RAY_CROSSED)
pub(crate) fn charge_query(engine: &Engine, seconds: f64) {
    if seconds > 0.0 {
        engine.state.borrow().sched.charge(seconds);
        crate::boundary::add_cost(seconds);
    }
}

/// What a box walk costs a client beyond the parts it lists (cost.rs QUERY_ROOT_CELL, QUERY_CELL, QUERY_MISSED_PART; Studio 0.747, a
/// headless client): each root cell in its range, each cell holding parts it visits and each part it tests and misses.
fn walk_cost(walked: Walked, missed: usize) -> f64 {
    walked.root_cells as f64 * crate::cost::QUERY_ROOT_CELL + walked.cells as f64 * crate::cost::QUERY_CELL + missed as f64 * crate::cost::QUERY_MISSED_PART
}

/// The parts `filter` sees that `keep` holds for, in the order the walk of `world`'s grid gives them (a client's order: world/grid.rs
/// walk_box), at most MaxParts of them: those of the walk over the box `center` +- `extent` (every part for none), the box clamped
/// to its centre +- [`CLAMP`] for the walk and the test alike (a part kept must also meet the clamped box). What the walk cost a
/// client is charged with `walked_price` ([`walk_cost`]: GetPartBoundsInBox, GetPartBoundsInRadius as a box of 2r), as is each part
/// the box meets but CanQuery and CanCollide leave out (cost.rs QUERY_UNQUERYABLE) and each the filter leaves out (QUERY_FILTERED_PART). It takes
/// in `world`'s parts ([`take_in_of`]).
fn overlapping(engine: &Engine, world: InstanceId, filter: Filter, center: Vec3, extent: Option<Vec3>, walked_price: bool, keep: impl Fn(&Solid) -> bool) -> Vec<InstanceId> {
    let Some(index) = take_in_of(engine, world) else {
        keep_filter(engine, filter);
        return Vec::new();
    };
    let test = |near: &[(InstanceId, Entry)], clamp: Option<(Vec3, Vec3)>| {
        let state = engine.state.borrow();
        let (mut seconds, mut missed) = (0.0, 0usize);
        let mut parts = Vec::new();
        for (part, entry) in near {
            if !keep(&entry.solid) || clamp.is_some_and(|(lo, hi)| !meets(&entry.solid, lo, hi)) {
                missed += 1;
                continue;
            }
            if !filter.flags_pass(entry) || !filter.groups_pass(&state, *part) {
                // (a part CanQuery and CanCollide leave out, or the params' group: each 0.45 to 0.53 of a listed part's cost on a
                // client and a server, measured)
                if !(filter.unqueryable || entry.can_query) || filter.flags_pass(entry) {
                    seconds += crate::cost::QUERY_UNQUERYABLE;
                }
                continue;
            }
            let (admitted, walk) = filter.check(&state, *part, true);
            seconds += walk + if admitted { 0.0 } else { crate::cost::QUERY_FILTERED_PART };
            if admitted {
                parts.push(*part);
            }
        }
        (parts, seconds, missed)
    };
    let (parts, seconds) = match extent.filter(|extent| (center + *extent).is_finite() && (center - *extent).is_finite()) {
        Some(extent) => {
            let (lo, hi, cut) = clamped(center, extent);
            walk_box(engine, index, lo, hi, grid::UNANCHORED, |near, walked| {
                let (parts, seconds, missed) = test(near, cut.then_some((lo, hi)));
                (parts, seconds + if walked_price { walk_cost(walked, missed) } else { 0.0 })
            })
        }
        None => {
            // (an infinite radius takes every part, and costs a client the walk of a box clamped on every side, as a box that reaches
            // everywhere does: measured, cost.rs QUERY_WIDE_PART)
            let walked = match extent {
                Some(extent) if walked_price && !extent.is_nan() => {
                    let (lo, hi, _) = clamped(center, Vec3::splat(f32::INFINITY));
                    walk_box(engine, index, lo, hi, grid::UNANCHORED, |_, walked| walk_cost(walked, 0))
                }
                _ => 0.0,
            };
            every_part(engine, index, |near| {
                let (parts, seconds, _) = test(near, None);
                (parts, seconds + walked)
            })
        }
    };
    charge_query(engine, seconds);
    let max_parts = filter.max_parts;
    keep_filter(engine, filter);
    limited(max_parts, parts)
}

/// A sphere's world box half size for [`overlapping`] (NaN for a NaN radius: every part)
fn sphere_extent(radius: f32) -> Option<Vec3> {
    Some(Vec3::splat(if radius.is_nan() { f32::NAN } else { radius.max(0.0) }))
}

/// How much GetPartsInPart grows its part's world box for its walk: 0.01 studs a side (measured, Studio 0.747, a playing client
/// and a server: bisected, a rod or cube part of side 7.97998 starts the walk on level 0 and one a float32 step longer on level 1,
/// so 7.98 + 2 x 0.01 is the level's 8 studs; grid_levels "query GetPartsInPart": 7.5 on level 0, 8 on level 1)
const PART_QUERY_GROWTH: f32 = 0.01;

pub fn install(engine: &Rc<Engine>) {
    let Some(world_root) = engine.reflection.class_id("WorldRoot") else { return };
    let mut members: Vec<(&str, MethodFn)> = Vec::new();
    members.push(("Raycast", method(|lua, engine, world, args| {
        let params = params_argument(engine, args, 3, Params::Raycast)?;
        let direction = vector3(engine, args, 2)?;
        let origin = vector3(engine, args, 1)?;
        let filter = take_filter(engine, world, Params::Raycast, &params);
        if !finite_path(origin, direction) {
            return missed(lua, engine, world, filter);
        }
        let hit = raycast(engine, world, &filter, origin, direction);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("Spherecast", method(|lua, engine, world, args| {
        let params = params_argument(engine, args, 4, Params::Raycast)?;
        let direction = vector3(engine, args, 3)?;
        let radius = super::camera::float_argument(lua, engine, args, 2)?;
        let center = vector3(engine, args, 1)?;
        // (a client checks the radius, then the direction, then the position: measured, tools/parity/probes/query_finite.luau)
        if !radius.is_finite() {
            return raise("Invalid Radius. Must contain finite values.");
        } else if radius < 0.0 {
            return raise(format!("Invalid Radius. Must be greater than or equal to 0, got {:.6}.", f64::from(radius)));
        } else if radius > SPHERECAST_RADIUS {
            return raise(format!("Attempt to spherecast with radius {:.6}. The maximum radius is 256.000000.", f64::from(radius)));
        }
        shapecast_direction(direction)?;
        if !center.is_finite() {
            return raise("Invalid position. Must contain finite values.");
        }
        let filter = take_filter(engine, world, Params::Raycast, &params);
        if !finite_path(center, direction) {
            return missed(lua, engine, world, filter);
        }
        let grow = radius;
        let swept = (center.min(center + direction) - Vec3::splat(grow), center.max(center + direction) + Vec3::splat(grow));
        let enter = |entry: &Entry, extra: f32| ray_solid(&entry.solid, center, direction, grow + extra).map(|(t, _)| t);
        let (hit, counts) = cast_and_count(engine, world, &filter, direction, enter, Some(swept), |state, near| cast_ray(near.iter().map(|(part, entry)| (*part, entry)), |part, entry| filter.sees(state, part, entry), center, direction, grow, None));
        charge_cast(engine, counts, crate::cost::SPHERE_MET, crate::cost::SPHERE_CROSSED, 0);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("Blockcast", method(|lua, engine, world, args| {
        let params = params_argument(engine, args, 4, Params::Raycast)?;
        let direction = vector3(engine, args, 3)?;
        let size = vector3(engine, args, 2)?;
        let frame = frame_argument(engine, args, 1)?;
        // (the size, then the direction, then the frame: measured)
        shapecast_size(size)?;
        shapecast_direction(direction)?;
        shapecast_frame(&frame)?;
        let filter = take_filter(engine, world, Params::Raycast, &params);
        // (a negative side past the largest finds nothing, where a positive one raises: measured, size (-600, 1, 1) found nothing and
        // sides of -1 to -20 hit; the bound is taken to be the positive one's, inferred)
        if !finite_path(frame::translation(&frame), direction) || size.min_element() < -SHAPECAST_SIDE {
            return missed(lua, engine, world, filter);
        }
        // (what a box's path meets counted by its own sweep, as cast_box finds its hit; the walk over its swept box, every part when
        // that is not finite)
        let center = frame::translation(&frame);
        let swept = box_extent(&frame, size).map(|extent| (center.min(center + direction) - extent, center.max(center + direction) + extent));
        let moving = Oriented::of(&unit_frame(&frame), (size * 0.5).abs());
        let enter = |entry: &Entry, extra: f32| sweep_boxes(&moving, direction, &Oriented::of(&entry.solid.frame, entry.solid.half_extents() + Vec3::splat(extra))).map(|(t, _)| t);
        let (hit, counts) = cast_and_count(engine, world, &filter, direction, enter, swept, |state, near| cast_box(near, |part, entry| filter.sees(state, part, entry), &frame, size, direction, None));
        charge_cast(engine, counts, crate::cost::BLOCK_MET, crate::cost::BLOCK_CROSSED, 0);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("Shapecast", method(|lua, engine, world, args| {
        let size_override = optional(engine, args, 5, "Vector3", vector_arg)?;
        let frame_override = optional(engine, args, 4, "CoordinateFrame", cframe_arg)?;
        let params = params_argument(engine, args, 3, Params::Raycast)?;
        let direction = vector3(engine, args, 2)?;
        let part = object_argument(args, 1)?;
        let part = base_part(engine, part, "Shapecast argument #1 expects a BasePart")?;
        // the part's shape where it is, or where cframeOverride puts it and at sizeOverride's size
        // (WorldRoot:Shapecast's optional parameters; Roblox's reference), checked as a client checks them: a negative side of
        // sizeOverride, then the size as a Blockcast's, the direction and the frame (measured)
        let (frame, size) = {
            let state = engine.state.borrow();
            let frame = frame_override.unwrap_or_else(|| frame::affine_of(state.dom.raw_get(part, cframe_name(engine))));
            let size = match size_override {
                Some(size) => {
                    if size.x < 0.0 || size.y < 0.0 || size.z < 0.0 {
                        return raise("Invalid sizeOverride. Must contain values greater than or equal to 0.");
                    }
                    size
                }
                None => match state.dom.raw_get(part, K::Size as NameId) {
                    Some(Value::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
                    _ => Vec3::ONE,
                },
            };
            (frame, size)
        };
        shapecast_size(size)?;
        shapecast_direction(direction)?;
        shapecast_frame(&frame)?;
        let filter = take_filter(engine, world, Params::Raycast, &params);
        if !finite_path(frame::translation(&frame), direction) {
            return missed(lua, engine, world, filter);
        }
        let cast = {
            let state = engine.state.borrow();
            let mut cast = solid(&state, engine, part);
            if let Some(frame) = frame_override {
                cast.frame = unit_frame(&frame);
            }
            if let Some(size) = size_override {
                cast.shape = shape_of(&state, engine, part, size);
            }
            cast
        };
        // the part being cast is never hit (documented); what it holds can be
        let center = frame::translation(&cast.frame);
        // (it stops at its hit as the other casts do: measured; the parts tied with the hit at a Blockcast's price, borrowed; what its
        // path meets counted by its own sweep, a ball's or a box's, as its hit is found; the walk over its swept box)
        let (_, reach) = world_reach(&cast);
        let swept = (center + reach).is_finite().then(|| (center.min(center + direction) - reach, center.max(center + direction) + reach));
        let moving = Oriented::of(&cast.frame, cast.half_extents());
        let enter = |entry: &Entry, extra: f32| match cast.shape {
            Shape::Ball(radius) => ray_solid(&entry.solid, center, direction, radius + extra).map(|(t, _)| t),
            _ => sweep_boxes(&moving, direction, &Oriented::of(&entry.solid.frame, entry.solid.half_extents() + Vec3::splat(extra))).map(|(t, _)| t),
        };
        let (hit, counts) = cast_and_count(engine, world, &filter, direction, enter, swept, |state, near| match cast.shape {
            Shape::Ball(radius) => cast_ray(near.iter().map(|(part, entry)| (*part, entry)), |part, entry| filter.sees(state, part, entry), center, direction, radius, Some(part)),
            Shape::Wedge(_) | Shape::CornerWedge(_) => cast_polytope(near, |part, entry| filter.sees(state, part, entry), &cast.polytope(), direction, Some(part)),
            _ => cast_box(near, |part, entry| filter.sees(state, part, entry), &cast.frame, cast.half_extents() * 2.0, direction, Some(part)),
        });
        let first = if matches!(cast.shape, Shape::Ball(_)) { crate::cost::SPHERE_MET } else { crate::cost::BLOCK_MET };
        charge_query(engine, crate::cost::SHAPECAST_CALL);
        charge_cast(engine, counts, first, crate::cost::BLOCK_CROSSED, crate::cost::kind::DEFAULTED);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("GetPartBoundsInBox", method(|lua, engine, world, args| {
        // (the overlap queries cast their arguments from the first to the last: measured)
        let frame = frame_argument(engine, args, 1)?;
        let size = vector3(engine, args, 2)?;
        let params = params_argument(engine, args, 3, Params::Overlap)?;
        let filter = take_filter(engine, world, Params::Overlap, &params);
        // (a frame that is not finite finds nothing, rotation or position: measured; it and a side that is not finite cost a
        // client what it spends on them, cost.rs QUERY_NOT_FINITE, QUERY_SIDE_NOT_FINITE)
        if !finite_frame(&frame) {
            charge_query(engine, crate::cost::QUERY_NOT_FINITE);
            return found_nothing(lua, engine, world, filter);
        }
        if !size.is_finite() {
            charge_query(engine, crate::cost::QUERY_SIDE_NOT_FINITE);
        }
        let size = box_size(size);
        let extent = box_extent(&frame, size);
        // (a box reaching the clamp on an axis reaches everywhere: each part it lists costs a client more, cost.rs QUERY_WIDE_PART)
        let wide = extent.is_none_or(|extent| extent.cmpge(Vec3::splat(CLAMP)).any());
        let parts = overlapping(engine, world, filter, frame::translation(&frame), extent, true, |s| overlaps_box(s, &frame, size));
        charge_query(engine, parts.len() as f64 * (crate::cost::QUERY_BOX_PART + if wide { crate::cost::QUERY_WIDE_PART } else { 0.0 }));
        parts_table(lua, engine, parts)
    })));
    members.push(("GetPartBoundsInRadius", method(|lua, engine, world, args| {
        let center = vector3(engine, args, 1)?;
        let radius = f64::from(super::camera::float_argument(lua, engine, args, 2)?);
        let params = params_argument(engine, args, 3, Params::Overlap)?;
        let filter = take_filter(engine, world, Params::Overlap, &params);
        // (a NaN radius or a position that is not finite finds nothing: measured, at what a client spends on it; an infinite
        // radius every part)
        if radius.is_nan() || !center.is_finite() {
            charge_query(engine, if center.is_finite() { crate::cost::QUERY_SIDE_NOT_FINITE } else { crate::cost::QUERY_NOT_FINITE });
            return found_nothing(lua, engine, world, filter);
        }
        let parts = overlapping(engine, world, filter, center, sphere_extent(radius as f32), true, |s| overlaps_sphere(s, center, radius as f32));
        // (a radius reaching the clamp reaches everywhere, an infinite one too: cost.rs QUERY_WIDE_PART)
        let wide = !((radius as f32) < CLAMP);
        charge_query(engine, parts.len() as f64 * (crate::cost::QUERY_RADIUS_PART + if wide { crate::cost::QUERY_WIDE_PART } else { 0.0 }));
        parts_table(lua, engine, parts)
    })));
    members.push(("GetPartsInPart", method(|lua, engine, world, args| {
        // (its part checked before its params are cast: measured)
        let part = object_argument(args, 1)?;
        let part = base_part(engine, part, "Part parameter must be BasePart")?;
        let params = params_argument(engine, args, 2, Params::Overlap)?;
        let filter = take_filter(engine, world, Params::Overlap, &params);
        let query = solid(&engine.state.borrow(), engine, part);
        // (its walk over the part's world box grown a little: PART_QUERY_GROWTH)
        let (center, extent) = match query.shape {
            Shape::Ball(radius) => (frame::translation(&query.frame), sphere_extent(radius)),
            _ => (frame::translation(&query.frame), box_extent(&query.frame, query.half_extents() * 2.0)),
        };
        let extent = extent.map(|extent| extent + Vec3::splat(PART_QUERY_GROWTH));
        // (its own fitted prices cover its walk: QUERY_PART_EXACT, QUERY_PART_SIZE; not measured apart; each part met as its own
        // shape, a wedge's too: overlaps_part)
        let parts = overlapping(engine, world, filter, center, extent, false, |s| s.part != part && overlaps_part(s, &query));
        // (its own work grows with the query part's box: cost.rs QUERY_PART_SIZE)
        let box_size = query.half_extents() * 2.0;
        let area = 2.0 * (box_size.x * box_size.y + box_size.y * box_size.z + box_size.z * box_size.x);
        charge_query(engine, parts.len() as f64 * crate::cost::QUERY_PART_EXACT + f64::from(area.max(0.0)).sqrt() * crate::cost::QUERY_PART_SIZE);
        parts_table(lua, engine, parts)
    })));
    members.push(("FindPartOnRay", method(|lua, engine, world, args| find_part_on_ray(lua, engine, world, args, false, false))));
    members.push(("FindPartOnRayWithIgnoreList", method(|lua, engine, world, args| find_part_on_ray(lua, engine, world, args, true, false))));
    members.push(("FindPartOnRayWithWhitelist", method(|lua, engine, world, args| find_part_on_ray(lua, engine, world, args, true, true))));
    let mut behaviors = engine.behaviors.borrow_mut();
    for (member, function) in members {
        if let Some(name) = engine.reflection.name_id(member) {
            behaviors.add_method(world_root, name, function);
        }
    }
}
