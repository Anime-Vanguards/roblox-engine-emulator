//! Spatial queries of the Workspace's parts: `WorldRoot:Raycast`, `Blockcast`, `Spherecast`,
//! `Shapecast`, `GetPartBoundsInBox`, `GetPartBoundsInRadius`, `GetPartsInPart` and the legacy
//! `FindPartOnRay`, `FindPartOnRayWithIgnoreList`, `FindPartOnRayWithWhitelist`.
//!
//! Fidelity: parts are their collision primitives where Roblox's are simple (Block a box, Ball a
//! sphere of the smallest axis, Cylinder along X), and their oriented bounding box otherwise
//! (wedges, corner wedges, MeshParts, unions, trusses: no mesh geometry is loaded). Terrain holds
//! no voxels and is never hit. Casts follow Roblox's documented rules: nothing a ray starts inside
//! or a shape starts overlapping is hit, parts with CanQuery off are skipped, RespectCanCollide
//! skips parts with CanCollide off, collision groups are not modelled (everything collides). A
//! Blockcast against a ball or cylinder uses its bounding box; a Spherecast against a box uses the
//! box grown by the radius (its corners square, so a glancing hit near a corner lands a little
//! early); a Blockcast's hit Position is the cast box's face centre at contact. Overlap queries
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

use super::broadphase::{self, Entry, Index};
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
            Shape::Box(half) => half,
            Shape::Ball(radius) => Vec3::splat(radius),
            Shape::Cylinder(half_length, radius) => Vec3::new(half_length, radius, radius),
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
/// How Roblox combines the two is not documented; here each applies (unmeasured), so params that set
/// only one filter (a new RaycastParams excludes nothing by the other) behave as documented.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// FilterType Include (or the legacy Whitelist): FilterDescendantsInstances lists what is seen
    pub include: bool,
    /// FilterDescendantsInstances, sorted, without repeats
    pub instances: Vec<InstanceId>,
    /// ExcludeInstances
    pub exclude_instances: Listed,
    /// IncludeInstances
    pub include_instances: Listed,
    pub respect_can_collide: bool,
    pub max_parts: usize,
    /// Parts with CanQuery off are seen too (GetPartsObscuringTarget's and the PlayerMouse's view of the Workspace: world/camera.rs,
    /// world/mouse.rs); never set from a query's params
    pub unqueryable: bool,
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
    /// as the legacy list reads; anything else is nil.
    fn read(&mut self, value: &LuaValue) {
        self.given = matches!(value, LuaValue::Table(_)) || instance_id(value).is_some();
        read_instances(value, &mut self.ids);
    }
}

impl Filter {
    /// Whether the part `id` passes. FilterDescendantsInstances must list the part or an ancestor
    /// exactly when FilterType is Include. Of the part and then each ancestor in turn, the first that
    /// ExcludeInstances or IncludeInstances lists decides, ExcludeInstances first when both list it;
    /// with none listed the part passes unless IncludeInstances is given (nil lets everything through,
    /// `{}` nothing). That is the announcement's own sample (devforum.roblox.com/t/4563575), except
    /// its last line, `return include ~= nil`, which inverts what its text says of nil and `{}`.
    fn admits(&self, state: &State, id: InstanceId) -> bool {
        let (exclude, include) = (&self.exclude_instances.ids, &self.include_instances.ids);
        let lists = !exclude.is_empty() || !include.is_empty();
        let (mut listed, mut decided) = (false, None);
        let mut current = Some(id);
        while let Some(node) = current {
            listed = listed || self.instances.binary_search(&node).is_ok();
            if lists && decided.is_none() {
                if exclude.binary_search(&node).is_ok() {
                    decided = Some(false);
                } else if include.binary_search(&node).is_ok() {
                    decided = Some(true);
                }
            }
            // (nothing further up can change either answer)
            if (listed || self.instances.is_empty()) && (decided.is_some() || !lists) {
                break;
            }
            current = state.dom.at(node).parent;
        }
        listed == self.include && decided.unwrap_or(!self.include_instances.given)
    }

    /// Whether a query with this filter sees the indexed part `part` (world/broadphase.rs): CanQuery off left out (unless the
    /// view takes such parts), CanCollide off left out with RespectCanCollide, and what [`Filter::admits`] says.
    pub(crate) fn sees(&self, state: &State, part: InstanceId, entry: &Entry) -> bool {
        (self.unqueryable || entry.can_query) && (!self.respect_can_collide || entry.can_collide) && self.admits(state, part)
    }

    /// RaycastParams / OverlapParams (a prelude proxy, or a table passed in its place) read into this
    /// filter, its lists' storage kept: a query reading its params allocates nothing.
    fn read(&mut self, engine: &Engine, params: &LuaValue) {
        let data = params_data(engine, params);
        // (a proxy's stored fields are a plain table: raw reads, without mlua's protected call)
        let get = |key: &str| match &data {
            Some((data, true)) => data.raw_get::<LuaValue>(key).unwrap_or(LuaValue::Nil),
            Some((data, false)) => data.get::<LuaValue>(key).unwrap_or(LuaValue::Nil),
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
        read_instances(&get("FilterDescendantsInstances"), &mut self.instances);
        self.exclude_instances.read(&get("ExcludeInstances"));
        self.include_instances.read(&get("IncludeInstances"));
        self.respect_can_collide = matches!(get("RespectCanCollide"), LuaValue::Boolean(true));
    }

    /// A legacy FindPartOnRay filter: `list` (an ignore list, or with `include` a whitelist) alone.
    fn read_list(&mut self, list: &LuaValue, include: bool) {
        self.include = include;
        read_instances(list, &mut self.instances);
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
/// a Cylinder a cylinder along X, anything else its bounding box.
fn shape_of(state: &State, engine: &Engine, part: InstanceId, size: Vec3) -> Shape {
    let reflection = &engine.reflection;
    let half = (size * 0.5).max(Vec3::ZERO);
    let class = state.dom.at(part).class;
    let shape = reflection.name_id("Shape").filter(|_| reflection.is_a(class, "Part")).and_then(|n| state.dom.raw_get(part, n)).and_then(Value::as_enum).map(|item| &*reflection.enum_item_info(item).name);
    match shape {
        Some("Ball") => Shape::Ball(half.min_element()),
        Some("Cylinder") => Shape::Cylinder(half.x, half.y.min(half.z)),
        _ => Shape::Box(half),
    }
}

/// The spatial queries' state on an engine: the broadphase over the Workspace's parts (world/broadphase.rs) and what a query
/// reuses. Each query walked and measured the whole Workspace whenever anything in the dom had changed (3.7 ms a ray on a
/// mounted map of 28k instances), and then tested every part (2.8 ms of host time a Raycast beside 100,000 parts away from it,
/// where a client spends 0.6 us); now it takes the dom's changes and tests the parts near it.
#[derive(Default)]
struct QueryCache {
    index: RefCell<Index>,
    /// The last query's filter, whose lists the next query reads its params into ([`take_filter`])
    spare: RefCell<Filter>,
    /// The parts near the last query, whose storage the next one fills ([`with_near`])
    near: RefCell<Vec<(InstanceId, Entry)>>,
}

impl crate::Extension for QueryCache {}

fn query_cache(engine: &Engine) -> &QueryCache {
    engine.extension_or_insert_with(QueryCache::default)
}

/// A query's filter, read from its params into the spare filter's storage; [`keep_filter`] hands it
/// back for the next query.
fn take_filter(engine: &Engine, params: &LuaValue) -> Filter {
    let mut filter = query_cache(engine).spare.take();
    filter.read(engine, params);
    filter
}

fn keep_filter(engine: &Engine, filter: Filter) {
    *query_cache(engine).spare.borrow_mut() = filter;
}

/// What a spatial query does first: the spatial index takes in what entered the Workspace since the last query or the physics
/// step (world/camera_moves.rs `index_parts`: GetLargestCutoffDistance and the PlayerMouse, which take nothing in, see it from
/// then), which the query pays for as a client's does (cost.rs QUERY_TAKE_IN and QUERY_TAKEN_PART for each part; moved and
/// removed parts cost it nothing), and the broadphase takes the dom's changes ([`refresh`]).
pub(crate) fn take_in(engine: &Engine) {
    let waiting = {
        let mut state = engine.state.borrow_mut();
        let waiting = state.cameras.unindexed.len();
        super::camera_moves::index_parts(&mut state);
        waiting
    };
    if waiting > 0 {
        charge_query(engine, crate::cost::QUERY_TAKE_IN + waiting as f64 * crate::cost::QUERY_TAKEN_PART);
    }
    refresh(engine);
}

/// The broadphase brought up to the dom's changes (world/broadphase.rs), the spatial index's wait left as it is: the PlayerMouse's
/// reads and GetLargestCutoffDistance take nothing in (world/mouse.rs, world/camera_moves.rs).
pub(crate) fn refresh(engine: &Engine) {
    query_cache(engine).index.borrow_mut().sync(engine);
}

/// `then` on the parts `gather` takes from the broadphase for a query (the Workspace's parts near it, or all of them, in no
/// particular order: the caller tests each and orders what it keeps).
pub(crate) fn with_near<R>(engine: &Engine, gather: impl FnOnce(&mut Index, &mut Vec<(InstanceId, Entry)>), then: impl FnOnce(&[(InstanceId, Entry)]) -> R) -> R {
    let cache = query_cache(engine);
    let mut near = cache.near.take();
    near.clear();
    gather(&mut cache.index.borrow_mut(), &mut near);
    let result = then(&near);
    near.clear();
    *cache.near.borrow_mut() = near;
    result
}

/// The index's tree height, whether its links and boxes hold, and how many parts it has (tests)
#[cfg(test)]
pub(crate) fn index_state(engine: &Engine) -> (i32, bool, usize) {
    let index = query_cache(engine).index.borrow();
    let (height, ok) = index.check_tree();
    (height, ok, index.len())
}

/// Whether the index holds what a walk of the Workspace finds: the same parts (every BasePart but Terrain), each with the solid
/// and flags the dom gives it now, to the bit (tests; its log taken first)
#[cfg(test)]
pub(crate) fn index_matches_a_walk(engine: &Engine) -> Result<(), String> {
    refresh(engine);
    let mut held = Vec::new();
    query_cache(engine).index.borrow().all_parts(&mut held);
    let state = engine.state.borrow();
    let reflection = &engine.reflection;
    let (base_part, terrain) = (reflection.class_id("BasePart").expect("BasePart"), reflection.class_id("Terrain").expect("Terrain"));
    let not_false = |part: InstanceId, name: &str| reflection.name_id(name).and_then(|n| state.dom.boolean(part, n)) != Some(false);
    let mut walked: Vec<String> = state
        .dom
        .descendants(engine.world().workspace)
        .into_iter()
        .filter(|&id| reflection.class(state.dom.at(id).class).is_a(base_part) && !reflection.class(state.dom.at(id).class).is_a(terrain))
        .map(|id| format!("{id:?} {:?} {} {}", solid(&state, engine, id), not_false(id, "CanQuery"), not_false(id, "CanCollide")))
        .collect();
    let mut kept: Vec<String> = held.iter().map(|(id, entry)| format!("{id:?} {:?} {} {}", entry.solid, entry.can_query, entry.can_collide)).collect();
    walked.sort();
    kept.sort();
    if walked == kept {
        return Ok(());
    }
    let missing: Vec<&String> = walked.iter().filter(|line| !kept.contains(line)).take(3).collect();
    let extra: Vec<&String> = kept.iter().filter(|line| !walked.contains(line)).take(3).collect();
    Err(format!("{} walked, {} held; missing {missing:?}; extra {extra:?}", walked.len(), kept.len()))
}

/// [`with_near`] for the parts a segment from `origin` along `direction`, with a sphere of radius `grow` swept along it, may meet.
pub(crate) fn near_segment<R>(engine: &Engine, origin: Vec3, direction: Vec3, grow: f32, then: impl FnOnce(&[(InstanceId, Entry)]) -> R) -> R {
    with_near(engine, |index, out| index.near_segment(origin, direction, grow, out), then)
}

/// The world box around an oriented box (its frame's axes as given, `size` its size), or None when the frame is skewed and its
/// box is no box (world/broadphase.rs skewed).
fn box_extent(frame: &Affine3A, size: Vec3) -> Option<Vec3> {
    let unit = unit_frame(frame);
    if broadphase::skewed(&unit) {
        return None;
    }
    let half = (size * 0.5).abs();
    let m = unit.matrix3;
    Some(Vec3::from(m.x_axis).abs() * half.x + Vec3::from(m.y_axis).abs() * half.y + Vec3::from(m.z_axis).abs() * half.z)
}

/// Where a ray from `origin` along `direction` (its length the reach) first enters `solid`:
/// (fraction of the direction, local normal), or None (a ray starting inside does not hit).
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
        Shape::Box(half) => ray_box(o, d, half + Vec3::splat(grow)),
        Shape::Ball(radius) => ray_sphere(o, d, radius + grow),
        Shape::Cylinder(half_length, radius) => ray_cylinder(o, d, half_length + grow, radius + grow),
    }
}

/// Slab test against a box centred at the origin; the entry must be at or after the start.
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
    (enter >= 0.0 && enter <= 1.0 && enter <= exit).then_some((enter, normal))
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
    (0.0..=1.0).contains(&t).then(|| (t, (o + d * t).normalize_or(Vec3::Y)))
}

/// A cylinder along X centred at the origin: its side or a cap.
pub fn ray_cylinder(o: Vec3, d: Vec3, half_length: f32, radius: f32) -> Option<(f32, Vec3)> {
    let inside = o.x.abs() <= half_length && o.y * o.y + o.z * o.z <= radius * radius;
    if inside {
        return None;
    }
    let mut best: Option<(f32, Vec3)> = None;
    let mut consider = |t: f32, normal: Vec3| {
        if (0.0..=1.0).contains(&t) && best.is_none_or(|(b, _)| t < b) {
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
/// `except`'s: the parts it enters, in the Workspace's order ([`broadphase::sort_by_order`] below `root`), the nearest kept,
/// the first of equals.
#[allow(clippy::too_many_arguments)]
pub fn cast_ray(state: &State, root: InstanceId, near: &[(InstanceId, Entry)], sees: impl Fn(InstanceId, &Entry) -> bool, origin: Vec3, direction: Vec3, grow: f32, except: Option<InstanceId>) -> Option<Hit> {
    let mut entered: Vec<(&Solid, f32, Vec3)> = near
        .iter()
        .filter(|(part, _)| except != Some(*part))
        .filter_map(|(part, entry)| ray_solid(&entry.solid, origin, direction, grow).filter(|_| sees(*part, entry)).map(|(t, normal)| (&entry.solid, t, normal)))
        .collect();
    let steps = broadphase::sort_by_order(&state.dom, root, &mut entered, |(solid, ..)| solid.part);
    charge_order(state, steps, entered.len());
    let mut best: Option<Hit> = None;
    for (solid, t, local_normal) in entered {
        let distance = direction.length() * t;
        if best.is_some_and(|hit| hit.distance <= distance) {
            continue;
        }
        let normal = solid.frame.transform_vector3(local_normal).normalize_or(Vec3::Y);
        // a swept sphere touches the surface one radius short of its centre
        let position = origin + direction * t - normal * grow;
        best = Some(Hit { part: solid.part, position, normal, distance });
    }
    best
}

/// The axes and half extents of an oriented box.
struct Oriented {
    center: Vec3,
    axes: [Vec3; 3],
    half: Vec3,
}

impl Oriented {
    fn of(frame: &Affine3A, half: Vec3) -> Self {
        Oriented {
            center: frame::translation(frame),
            axes: [Vec3::from(frame.matrix3.x_axis), Vec3::from(frame.matrix3.y_axis), Vec3::from(frame.matrix3.z_axis)],
            half,
        }
    }

    fn radius(&self, axis: Vec3) -> f32 {
        (0..3).map(|i| self.axes[i].dot(axis).abs() * self.half[i]).sum()
    }
}

/// The separating axes of two boxes (their own and the cross products).
fn sat_axes(a: &Oriented, b: &Oriented) -> Vec<Vec3> {
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
            normal = if speed > 0.0 { -axis } else { axis };
        }
        exit = exit.min(far);
        if enter > exit {
            return None;
        }
    }
    (enter > 0.0 && enter <= 1.0).then_some((enter, normal))
}

/// The closest part of `near` that `sees` lets through a box (`frame`, `size`) swept along `direction` touches, `except` aside:
/// the parts it touches, in the Workspace's order, the nearest kept, the first of equals.
#[allow(clippy::too_many_arguments)]
pub fn cast_box(state: &State, root: InstanceId, near: &[(InstanceId, Entry)], sees: impl Fn(InstanceId, &Entry) -> bool, frame: &Affine3A, size: Vec3, direction: Vec3, except: Option<InstanceId>) -> Option<Hit> {
    let moving = Oriented::of(&unit_frame(frame), size * 0.5);
    let mut touched: Vec<(&Solid, f32, Vec3)> = near
        .iter()
        .filter(|(part, _)| except != Some(*part))
        .filter_map(|(part, entry)| {
            let other = Oriented::of(&entry.solid.frame, entry.solid.half_extents());
            sweep_boxes(&moving, direction, &other).filter(|_| sees(*part, entry)).map(|(t, normal)| (&entry.solid, t, normal))
        })
        .collect();
    let steps = broadphase::sort_by_order(&state.dom, root, &mut touched, |(solid, ..)| solid.part);
    charge_order(state, steps, touched.len());
    let mut best: Option<Hit> = None;
    for (solid, t, normal) in touched {
        let distance = direction.length() * t;
        if best.is_some_and(|hit| hit.distance <= distance) {
            continue;
        }
        let center = moving.center + direction * t;
        let position = center - normal * moving.radius(normal);
        best = Some(Hit { part: solid.part, position, normal, distance });
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

// Script bindings

fn method(f: impl Fn(&Lua, &Rc<Engine>, InstanceId, &Args) -> Result<LuaMultiValue, Flow> + 'static) -> MethodFn {
    Rc::new(f)
}

/// The fields of a RaycastParams / OverlapParams: what a prelude proxy stores (the prelude's
/// `ParamsData`, read without the proxy's copies of its lists; true: a plain table read raw), or a
/// table passed in its place (read as a script reads it).
fn params_data(engine: &Engine, params: &LuaValue) -> Option<(LuaTable, bool)> {
    match params {
        LuaValue::UserData(ud) => {
            let stored: LuaTable = engine.prelude().exports.raw_get("ParamsData").ok()?;
            stored.raw_get::<Option<LuaTable>>(ud.clone()).ok().flatten().map(|data| (data, true))
        }
        LuaValue::Table(table) => Some((table.clone(), false)),
        _ => None,
    }
}

/// The instances of an instance list (a table's array part, or a lone instance) into `ids`, sorted,
/// without repeats (the filters search them).
fn read_instances(value: &LuaValue, ids: &mut Vec<InstanceId>) {
    ids.clear();
    match value {
        LuaValue::Table(table) => ids.extend(table.sequence_values::<LuaValue>().filter_map(|v| v.ok()).filter_map(|v| instance_id(&v))),
        other => ids.extend(instance_id(other)),
    }
    ids.sort_unstable();
    ids.dedup();
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

fn parts_table(lua: &Lua, engine: &Rc<Engine>, parts: impl IntoIterator<Item = InstanceId>) -> Result<LuaMultiValue, Flow> {
    let table = lua.create_table()?;
    for part in parts {
        table.raw_push(instance_value(lua, engine, part)?)?;
    }
    Ok(values([LuaValue::Table(table)]))
}

/// The legacy FindPartOnRay family: (part, position, normal, material); the ray's end, a zero
/// normal and Air when nothing is hit.
fn find_part_on_ray(lua: &Lua, engine: &Rc<Engine>, ray: &LuaValue, list: &LuaValue, include: bool) -> Result<LuaMultiValue, Flow> {
    let ray = match ray {
        LuaValue::UserData(ud) => ud.borrow::<lune::Ray>().ok().map(|ray| dom::Ray::from(*ray)),
        _ => None,
    };
    let Some(ray) = ray else { return raise("Unable to cast value to Ray") };
    let origin = Vec3::new(ray.origin.x, ray.origin.y, ray.origin.z);
    let direction = Vec3::new(ray.direction.x, ray.direction.y, ray.direction.z);
    let mut filter = query_cache(engine).spare.take();
    filter.read_list(list, include);
    take_in(engine);
    let root = engine.world().workspace;
    let hit = near_segment(engine, origin, direction, 0.0, |near| {
        let state = engine.state.borrow();
        cast_ray(&state, root, near, |part, entry| filter.sees(&state, part, entry), origin, direction, 0.0, None)
    });
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
/// [`cast_counts`]'.
fn charge_cast(engine: &Engine, (met, admitted): (usize, usize), each: f64, kinds: u8) {
    if met > 0 {
        let tied = admitted.saturating_sub(1);
        let seconds = crate::cost::RAY_MET + tied as f64 * each + (met - 1 - tied) as f64 * crate::cost::RAY_CROSSED;
        charge_query(engine, seconds);
        engine.state.borrow().sched.touched.borrow().book_kinds(kinds, seconds);
    }
}

/// The parts among `near` with CanQuery on (whatever else the cast's filter says) that a cast's path from `origin` along
/// `direction`, grown by `grow`, meets up to its hit (`hit`, the distance along `direction`), and how many of those its filter
/// admits when it hit something: what [`charge_cast`] charges for.
#[allow(clippy::too_many_arguments)]
fn cast_counts(state: &State, near: &[(InstanceId, Entry)], filter: &Filter, hit: Option<f32>, origin: Vec3, direction: Vec3, grow: f32) -> (usize, usize) {
    let length = direction.length();
    let (mut met, mut admitted) = (0usize, 0usize);
    for (part, entry) in near {
        // (the parts a query with no filter sees: CanQuery off left out)
        if !entry.can_query {
            continue;
        }
        let Some((t, _)) = ray_solid(&entry.solid, origin, direction, grow) else { continue };
        // (the hit's own distance is reckoned the same way: the hit and the parts tied with it are met)
        if hit.is_some_and(|distance| length * t > distance) {
            continue;
        }
        met += 1;
        let collides = !filter.respect_can_collide || entry.can_collide;
        if hit.is_some() && collides && filter.admits(state, *part) {
            admitted += 1;
        }
    }
    (met, admitted)
}

/// A cast's hit and what it is charged for: the parts near its path (a segment from `origin` along `direction`, grown by `grow`
/// for the charge and at least as much for the cast itself, or every part with `all`), tested by `cast` for the hit and by
/// [`cast_counts`] for what its path meets.
#[allow(clippy::too_many_arguments)]
fn cast_and_count(engine: &Engine, filter: &Filter, origin: Vec3, direction: Vec3, grow: f32, reach: f32, all: bool, cast: impl FnOnce(&State, InstanceId, &[(InstanceId, Entry)]) -> Option<Hit>) -> (Option<Hit>, (usize, usize)) {
    take_in(engine);
    let root = engine.world().workspace;
    with_near(
        engine,
        |index, out| if all { index.all_parts(out) } else { index.near_segment(origin, direction, grow.max(reach), out) },
        |near| {
            let state = engine.state.borrow();
            let hit = cast(&state, root, near);
            let counts = cast_counts(&state, near, filter, hit.map(|hit| hit.distance), origin, direction, grow);
            (hit, counts)
        },
    )
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

/// What the walk that put a query's `items` parts in the Workspace's order took `steps` for costs past what the parts' own prices
/// cover (world/broadphase.rs order_cost, cost.rs QUERY_ORDER_STEP), charged as [`charge_query`] charges
pub(crate) fn charge_order(state: &State, steps: usize, items: usize) {
    let seconds = broadphase::order_cost(steps, items);
    if seconds > 0.0 {
        state.sched.charge(seconds);
        crate::boundary::add_cost(seconds);
    }
}

/// A spatial query's charge beyond its call's price (cost.rs QUERY_BOX_PART, RAY_CROSSED)
pub(crate) fn charge_query(engine: &Engine, seconds: f64) {
    if seconds > 0.0 {
        engine.state.borrow().sched.charge(seconds);
        crate::boundary::add_cost(seconds);
    }
}

/// The parts `filter` sees that `keep` holds for, in the Workspace's order, at most MaxParts of them: those near the box
/// `center` +- `extent` (every part for none) tested.
fn overlapping(engine: &Engine, filter: Filter, center: Vec3, extent: Option<Vec3>, keep: impl Fn(&Solid) -> bool) -> Vec<InstanceId> {
    take_in(engine);
    let root = engine.world().workspace;
    let mut parts = with_near(
        engine,
        |index, out| index.near_box(center, extent, out),
        |near| {
            let state = engine.state.borrow();
            near.iter().filter(|(part, entry)| keep(&entry.solid) && filter.sees(&state, *part, entry)).map(|(part, _)| *part).collect::<Vec<_>>()
        },
    );
    {
        let state = engine.state.borrow();
        let steps = broadphase::sort_by_order(&state.dom, root, &mut parts, |part| *part);
        charge_order(&state, steps, parts.len());
    }
    let max_parts = filter.max_parts;
    keep_filter(engine, filter);
    limited(max_parts, parts)
}

/// A sphere's world box half size for [`overlapping`] (NaN for a NaN radius: every part)
fn sphere_extent(radius: f32) -> Option<Vec3> {
    Some(Vec3::splat(if radius.is_nan() { f32::NAN } else { radius.max(0.0) }))
}

pub fn install(engine: &Rc<Engine>) {
    let Some(world_root) = engine.reflection.class_id("WorldRoot") else { return };
    let mut members: Vec<(&str, MethodFn)> = Vec::new();
    members.push(("Raycast", method(|lua, engine, _, args| {
        let (Some(origin), Some(direction)) = (vector_arg(args.get(1)), vector_arg(args.get(2))) else {
            return raise("Raycast: origin and direction must be Vector3s");
        };
        let filter = take_filter(engine, args.get(3));
        let (hit, counts) = cast_and_count(engine, &filter, origin, direction, 0.0, 0.0, false, |state, root, near| cast_ray(state, root, near, |part, entry| filter.sees(state, part, entry), origin, direction, 0.0, None));
        charge_cast(engine, counts, crate::cost::RAY_CROSSED, 0);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("Spherecast", method(|lua, engine, _, args| {
        let (Some(center), Some(radius), Some(direction)) = (vector_arg(args.get(1)), args.number(2), vector_arg(args.get(3))) else {
            return raise("Spherecast: position, radius and direction expected");
        };
        let filter = take_filter(engine, args.get(4));
        let grow = radius.max(0.0) as f32;
        let (hit, counts) = cast_and_count(engine, &filter, center, direction, grow, grow, false, |state, root, near| cast_ray(state, root, near, |part, entry| filter.sees(state, part, entry), center, direction, grow, None));
        charge_cast(engine, counts, crate::cost::SPHERE_CROSSED, 0);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("Blockcast", method(|lua, engine, _, args| {
        let (Some(frame), Some(size), Some(direction)) = (cframe_arg(args.get(1)), vector_arg(args.get(2)), vector_arg(args.get(3))) else {
            return raise("Blockcast: cframe, size and direction expected");
        };
        let filter = take_filter(engine, args.get(4));
        // (a box's path counted as its centre's, grown by its half diagonal: its bounding sphere, so also what the cast can touch
        // unless its frame is skewed)
        let skewed = broadphase::skewed(&unit_frame(&frame));
        let (hit, counts) = cast_and_count(engine, &filter, frame::translation(&frame), direction, size.length() * 0.5, 0.0, skewed, |state, root, near| cast_box(state, root, near, |part, entry| filter.sees(state, part, entry), &frame, size, direction, None));
        charge_cast(engine, counts, crate::cost::BLOCK_CROSSED, 0);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("Shapecast", method(|lua, engine, _, args| {
        let (Some(part), Some(direction)) = (args.instance(1), vector_arg(args.get(2))) else {
            return raise("Shapecast: part and direction expected");
        };
        let filter = take_filter(engine, args.get(3));
        // the part's shape where it is, or where cframeOverride puts it and at sizeOverride's size
        // (WorldRoot:Shapecast's optional parameters; Roblox's reference)
        let cast = {
            let state = engine.state.borrow();
            let mut cast = solid(&state, engine, part);
            if let Some(frame) = cframe_arg(args.get(4)) {
                cast.frame = unit_frame(&frame);
            }
            if let Some(size) = vector_arg(args.get(5)) {
                cast.shape = shape_of(&state, engine, part, size);
            }
            cast
        };
        // the part being cast is never hit (documented); what it holds can be
        let center = frame::translation(&cast.frame);
        let skewed = !matches!(cast.shape, Shape::Ball(_)) && broadphase::skewed(&cast.frame);
        // (it stops at its hit as the other casts do: measured; the parts tied with the hit at a Blockcast's price, borrowed; its
        // path counted grown by its half diagonal, which holds what it can touch)
        let (hit, counts) = cast_and_count(engine, &filter, center, direction, cast.half_extents().length(), 0.0, skewed, |state, root, near| match cast.shape {
            Shape::Ball(radius) => cast_ray(state, root, near, |part, entry| filter.sees(state, part, entry), center, direction, radius, Some(part)),
            _ => cast_box(state, root, near, |part, entry| filter.sees(state, part, entry), &cast.frame, cast.half_extents() * 2.0, direction, Some(part)),
        });
        charge_cast(engine, counts, crate::cost::BLOCK_CROSSED, crate::cost::kind::DEFAULTED);
        keep_filter(engine, filter);
        hit_values(lua, engine, hit)
    })));
    members.push(("GetPartBoundsInBox", method(|lua, engine, _, args| {
        let (Some(frame), Some(size)) = (cframe_arg(args.get(1)), vector_arg(args.get(2))) else {
            return raise("GetPartBoundsInBox: cframe and size expected");
        };
        let filter = take_filter(engine, args.get(3));
        let parts = overlapping(engine, filter, frame::translation(&frame), box_extent(&frame, size), |s| overlaps_box(s, &frame, size));
        charge_query(engine, parts.len() as f64 * crate::cost::QUERY_BOX_PART);
        parts_table(lua, engine, parts)
    })));
    members.push(("GetPartBoundsInRadius", method(|lua, engine, _, args| {
        let (Some(center), Some(radius)) = (vector_arg(args.get(1)), args.number(2)) else {
            return raise("GetPartBoundsInRadius: position and radius expected");
        };
        let filter = take_filter(engine, args.get(3));
        let parts = overlapping(engine, filter, center, sphere_extent(radius as f32), |s| overlaps_sphere(s, center, radius as f32));
        charge_query(engine, parts.len() as f64 * crate::cost::QUERY_RADIUS_PART);
        parts_table(lua, engine, parts)
    })));
    members.push(("GetPartsInPart", method(|lua, engine, _, args| {
        let Some(part) = args.instance(1) else { return raise("GetPartsInPart: part expected") };
        let filter = take_filter(engine, args.get(2));
        let query = solid(&engine.state.borrow(), engine, part);
        let (center, extent) = match query.shape {
            Shape::Ball(radius) => (frame::translation(&query.frame), sphere_extent(radius)),
            _ => (frame::translation(&query.frame), box_extent(&query.frame, query.half_extents() * 2.0)),
        };
        let parts = overlapping(engine, filter, center, extent, |s| {
            s.part != part
                && match query.shape {
                    Shape::Ball(radius) => overlaps_sphere(s, frame::translation(&query.frame), radius),
                    _ => overlaps_box(s, &query.frame, query.half_extents() * 2.0),
                }
        });
        // (its own work grows with the query part's box: cost.rs QUERY_PART_SIZE)
        let box_size = query.half_extents() * 2.0;
        let area = 2.0 * (box_size.x * box_size.y + box_size.y * box_size.z + box_size.z * box_size.x);
        charge_query(engine, parts.len() as f64 * crate::cost::QUERY_PART_EXACT + f64::from(area.max(0.0)).sqrt() * crate::cost::QUERY_PART_SIZE);
        parts_table(lua, engine, parts)
    })));
    members.push(("FindPartOnRay", method(|lua, engine, _, args| find_part_on_ray(lua, engine, args.get(1), args.get(2), false))));
    members.push(("FindPartOnRayWithIgnoreList", method(|lua, engine, _, args| find_part_on_ray(lua, engine, args.get(1), args.get(2), false))));
    members.push(("FindPartOnRayWithWhitelist", method(|lua, engine, _, args| find_part_on_ray(lua, engine, args.get(1), args.get(2), true))));
    let mut behaviors = engine.behaviors.borrow_mut();
    for (member, function) in members {
        if let Some(name) = engine.reflection.name_id(member) {
            behaviors.add_method(world_root, name, function);
        }
    }
}
