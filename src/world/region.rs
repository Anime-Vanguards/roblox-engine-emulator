//! The legacy Region3 queries (`WorldRoot:FindPartsInRegion3`, `FindPartsInRegion3WithIgnoreList`,
//! `FindPartsInRegion3WithWhiteList`, the deprecated `findPartsInRegion3`, `IsRegion3Empty`, `IsRegion3EmptyWithIgnoreList`)
//! and `WorldRoot:ArePartsTouchingOthers`, as a playing client answers them (Studio 0.748, measured with os.clock and
//! printed results; docs/engine.md "Region3 queries").
//!
//! - Arguments are checked from the last to the first. `maxParts` is an int (camera_moves.rs's cast), 20 when nil, and lists
//!   every part found when it is not past 0. An ignored instance is nil or an Instance ("Unable to cast value to Object").
//!   A list is a table read as world/mod.rs ordered_ignore_list reads an instance list (from 1 to its length, nil holes
//!   skipped, any other entry "Unable to cast value to Object"; each entry charged as it is read, also on a call that raises
//!   later). The region is a Region3 ("Argument 1 missing or nil", "Unable to cast <type> to Region3", named as camera.rs
//!   cast_name names it). A call that raises costs what any engine call's error costs a client (lua/cfunction.rs
//!   charge_raised). Not matched: a list whose first entry is no Instance costs a client more
//!   (FindPartsInRegion3WithIgnoreList(region, { 5 }) 6.2 to 9.5 us past an empty pcall, here RAISED and the call, 3.8).
//! - A Region3 query finds the BaseParts of the WorldRoot's own world (a WorldModel's parts are its own, the Workspace's
//!   are not a WorldModel's in it; Terrain never, voxels or not; a WorldModel outside the DataModel has none) that the spatial
//!   index holds: a part that came into the world since a spatial query of that world or the physics step last took parts in
//!   is not found (made in it, moved in from another world, or brought into the DataModel with its WorldModel:
//!   camera_moves.rs), and these queries take none in; a moved, resized or removed part counts at once. A part with CanQuery
//!   and CanCollide both off is not found; CanQuery off alone hides nothing (nor do Transparency, CanTouch and Anchored). A
//!   part is found when its world-aligned box, its collision box (a ball's cube of its smallest side, a cylinder's length by
//!   its diameter: query.rs `Solid`) turned into the world, meets the region's, touching included, the region's min and max
//!   as it holds them (an inverted one's too: a part reaching past both of its corners is found). A region with a bound or a
//!   centre that is not finite finds nothing and tests no part (a client's Region3 holds its centre, the bounds' mean in
//!   float32: from 2e38 to 2e38 it is infinite). The ignored instance (or each one an ignore list names) leaves out itself
//!   and its descendants; a whitelist keeps only what it names and their descendants: for each instance of the list in its
//!   order, until one is the part or an ancestor of it, a walk up from the part, each before that one to the top of the tree,
//!   and only until the result is full (a client's checks cost the same way and stop there: [`Keep::keeps`]). `maxParts`
//!   keeps the first parts found. The result is a plain table; IsRegion3Empty* return whether none was found.
//! - What a query looks at is what its world's grid walk gives it over its region (world/broadphase.rs, world/grid.rs: the
//!   Workspace's grid, which holds none of a WorldModel's parts in it, or the WorldModel's own), its world box clamped to its
//!   centre +- 7,500 studs as a box query's (world/query.rs CLAMP: a part kept must also meet the clamped box), a cell's
//!   unanchored parts first: so the parts come in a client's order (public #45 measured the Region3 queries' order the same
//!   as GetPartBoundsInBox's), `maxParts` keeps a client's first, and a long region finds only the parts around its centre
//!   (measured: a region from -1 out to 3e4 studs along x found only those, out to 1e4 every part in it), as parts past the
//!   grid's far bound are found by none (2^26 studs for small parts: a client's queries found no part 7e7 studs out, 5e7 out
//!   they did; its ArePartsTouchingOthers of two parts touching at 2e38 is false). A part waiting for the spatial index is in
//!   no cell (a client's index does not hold it yet, and it costs its query nothing: measured). A WorldModel's query pays ree's
//!   own price for what its index reads of the changes in its world (cost.rs FIND_NODE an instance its index looks at in its
//!   log or a walk of its world, MODEL_INDEX_PART a part such a walk files; a client spends about twice FIND_NODE there:
//!   ArePartsTouchingOthers after a Folder of 1,000 parts entered its WorldModel and left, 7.8 us on a client, 4.3 here);
//!   the index is kept while the WorldModel is in the DataModel. Each part the walk gives costs what a client spends on such a
//!   part (cost.rs REGION_FOUND and after): one found; one with CanQuery and CanCollide off in the region or within [`BAND`]
//!   of it (REGION_HIDDEN); one whose box is within [`BAND`] of the region and not in it (REGION_NEAR); one further out in a
//!   cell the walk visits (REGION_FAR, as a client pays for the parts of the cells its query looks at).
//! - Not matched: the last millionths at a face: a client finds a part whose box reaches past the region by up to 0.5e-6 to
//!   3e-6 depending on the sizes, here by up to [`EDGE`] in double precision. Nor a part's arrangement: a client spends more on
//!   parts stacked in one spot than spread out (measured, cost.rs REGION_NEAR), and less on parts packed in a lattice
//!   (measured: 1,000 half-stud cubes 0.6 apart, a region around them 6.6 us with maxParts 1 or IsRegion3Empty, here 20.4).
//!   Nor a WorldModel's own prices: a client's query of one spent about twice a Workspace query's on each part found (196
//!   parts 8.3 us, 4.55 in the Workspace), and as much on the others.
//! - ArePartsTouchingOthers(partList, overlapIgnored = 0.0002): the float first (camera.rs `float_argument`, nil the
//!   default), the list (objects_arg, its entries cast as above, nil holes kept, each entry charged as it is read:
//!   [`touching_entries`]), then each entry in turn: "All objects passed to ArePartsTouchingOthers must be BaseParts, got a
//!   <ClassName>." (a hole "got a Nil."), "All parts passed to ArePartsTouchingOthers must be in the WorldRoot" (its nearest
//!   WorldRoot, read from the dom's homes without walking up: world/mod.rs `world_root_of`; Terrain the Workspace's child). A
//!   call that raises costs what a client spends on it (cost.rs RAISED and TOUCHING_RAISED, measured). It then takes into
//!   the spatial index what came into its world, as other spatial queries do, and pays for it as they do (query.rs
//!   `take_in_of`, measured the same for it), and returns whether a listed part with CanCollide on overlaps a part of its
//!   world that is not listed, has CanCollide on and is not Terrain, by more than overlapIgnored (strictly): two balls by
//!   their spheres, a ball and anything else by the sphere against the other's box, two boxes by the least overlap along
//!   their separating axes (a negative overlapIgnored lets a gap that small count, up to [`REACH`]). Each listed part is
//!   tested once, its world's tree giving it the parts near its box one at a time until one touches it, and each part given
//!   is paid for: tested, or skipped for being listed or for having CanCollide off, within [`REACH`] of its box, or skipped
//!   further out (cost.rs TOUCHING_PAIR and after, a client's prices); a repeat in the list is charged as its first test was
//!   (a client's repeat costs about two thirds of it: measured, 30 copies of one part 3.3 us where 30 parts 4.9). Not
//!   matched: a client tests a cylinder's round side and a wedge's slope; here they are their boxes, as in the other overlap
//!   queries. Nor a long list of packed parts: 1,000 half-stud cubes 0.6 apart, all listed and none touching, cost a client
//!   11.4 to 11.6 ms, here 1.8 (measured).

use std::rc::Rc;

use glam::Vec3;
use lune_roblox::datatypes::types as lune;
use mlua::prelude::*;
use rbx_dom_weak::types as dom;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    IgnoreList,
    broadphase::{self, Entry},
    grid,
    camera, frame,
    query::{self, Shape, Solid},
};
use crate::{
    ClassId, Engine, InstanceId, NameId, State,
    lua::{Args, Flow, instance_id, raise, values},
    services::MethodFn,
};

/// ArePartsTouchingOthers' default overlapIgnored (the API reference's 0.000199999995: 0.0002 as a float)
const OVERLAP_IGNORED: f32 = 0.0002;

/// How far apart two parts may be and still be tested by ArePartsTouchingOthers (a negative overlapIgnored counts a gap only
/// this far: measured, two cubes 0.31 apart count at -1 and -10, 0.32 apart do not, on either axis; and its walk tests the parts
/// 0.31 from a listed part's box and not those 0.315 from it: cost.rs TOUCHING_FAR)
const REACH: f32 = 0.3125;

/// maxParts when none is given (the API reference; measured: 20 of 30 parts in the region)
const MAX_PARTS: usize = 20;

/// How far from a Region3 a part's box may lie, along each axis, and still cost a client's query a near part's price (cost.rs
/// REGION_NEAR; measured: half-stud cubes 0.30 past a face are near, 0.31 past are not, and a 0.3 past both faces of an edge is)
const BAND: f64 = 0.305;

/// The region argument: the Region3's min and max as it holds them.
fn region_argument(engine: &Engine, args: &Args, at: usize) -> Result<(Vec3, Vec3), Flow> {
    let value = args.get(at);
    if value.is_nil() {
        return raise(format!("Argument {at} missing or nil"));
    }
    if let LuaValue::UserData(ud) = value
        && let Ok(region) = ud.borrow::<lune::Region3>()
    {
        let region = dom::Region3::from(*region);
        return Ok((Vec3::new(region.min.x, region.min.y, region.min.z), Vec3::new(region.max.x, region.max.y, region.max.z)));
    }
    raise(format!("Unable to cast {} to Region3", camera::cast_name(engine, value)))
}

/// maxParts: an int as a client casts one, 20 when nil; one not past 0 keeps every part (measured: 0, -1, NaN, false, 2^31).
fn max_parts_argument(lua: &Lua, engine: &Engine, args: &Args, at: usize) -> Result<usize, Flow> {
    if args.get(at).is_nil() {
        return Ok(MAX_PARTS);
    }
    let count = super::camera_moves::int_argument(lua, engine, args, at)?;
    Ok(if count > 0 { count as usize } else { usize::MAX })
}

/// The ignored instance: nil, or an Instance.
fn ignore_argument(args: &Args, at: usize) -> Result<Option<InstanceId>, Flow> {
    match args.get(at) {
        LuaValue::Nil => Ok(None),
        value => match instance_id(value) {
            Some(id) => Ok(Some(id)),
            None => raise("Unable to cast value to Object"),
        },
    }
}

/// ArePartsTouchingOthers' list (world/mod.rs objects_arg): its entries in order up to its first nil hole, the hole kept as None
/// (where the entries' check raises); the entries after it are read and cast, not kept. Each entry read costs what a client
/// spends reading an instance list (cost.rs IGNORE_LISTED, IGNORE_HOLE, as world/mod.rs ignore_list charges; measured here, a
/// Folder after 1,000 parts raises for 30 to 37 us, the raise included), also on a call that raises later, and an entry that is
/// no Instance raises "Unable to cast value to Object" for LIST_CAST_FAILED each entry before it (0.26 us measured here).
fn touching_entries(engine: &Engine, args: &Args, at: usize) -> Result<Vec<Option<InstanceId>>, Flow> {
    let table = super::objects_arg(args, at)?;
    let (mut entries, mut listed, mut holes) = (Vec::new(), 0usize, 0usize);
    for index in 1..=table.raw_len() {
        match table.raw_get::<LuaValue>(index)? {
            LuaValue::Nil => {
                if holes == 0 {
                    entries.push(None);
                }
                holes += 1;
            }
            value => match instance_id(&value) {
                Some(id) => {
                    listed += 1;
                    if holes == 0 {
                        entries.push(Some(id));
                    }
                }
                None => {
                    query::charge_query(engine, (index - 1) as f64 * crate::cost::LIST_CAST_FAILED);
                    return raise("Unable to cast value to Object");
                }
            },
        }
    }
    query::charge_query(engine, listed as f64 * crate::cost::IGNORE_LISTED + holes as f64 * crate::cost::IGNORE_HOLE);
    Ok(entries)
}

/// How far past a Region3's face a part's box may reach and still be found, in double precision. Measured edges, a part's box
/// this far past (double): a 0.2 part past [500, 508] found at 0.998e-6 and not at 1.498e-6; a 2 stud part 2.03e-6 and not
/// 2.98e-6; a 0.2 part past [0, 64] 2.0e-6 and not 3.0e-6; past [0, 1] 0.5e-6 and not 1.0e-6; before [500, 508]'s min 1.0e-6
/// and not 2.0e-6; float32 boxes (c - h rounded) would find a 0.2 part at 508.1 (6.1e-6 past), which a client does not. No
/// one tolerance fits them all: this one fits 45 of the 52 measured edges (float32 boxes 34).
const EDGE: f64 = 1e-6;

/// Whether a part's world-aligned box (its centre and half size: query.rs `world_reach`) meets the region [lo, hi] grown by `by`
/// along each axis (touching counts), in double precision.
fn within((center, reach): (Vec3, Vec3), lo: Vec3, hi: Vec3, by: f64) -> bool {
    (0..3).all(|axis| {
        let (c, r) = (f64::from(center[axis]), f64::from(reach[axis]));
        c - r <= f64::from(hi[axis]) + by && c + r >= f64::from(lo[axis]) - by
    })
}

/// Whether a part's world-aligned box meets the region [lo, hi] (touching counts), in double precision within [`EDGE`].
fn meets(reach: (Vec3, Vec3), lo: Vec3, hi: Vec3) -> bool {
    within(reach, lo, hi, EDGE)
}

/// Which of the parts found a Region3 query keeps.
enum Keep<'a> {
    All,
    /// Not this instance nor its descendants
    Ignore(InstanceId),
    /// Not these nor their descendants (an ignore list, as world/mod.rs ordered_ignore_list reads it)
    IgnoreList(&'a IgnoreList),
    /// Only these and their descendants
    WhiteList(&'a IgnoreList),
}

impl Keep<'_> {
    /// Whether it keeps `part`, and what a client spends deciding it. A client checks each instance of the list in its order
    /// until one is the part or an ancestor of it, each with a walk up from the part, so each before that one walks to the top of
    /// the tree, at cost.rs filter_step for the part's depth a step, as a client checks a query's FilterDescendantsInstances
    /// (world/query.rs Filter::check). Measured (Studio 0.748, a playing client; 10 parts found under 1 to 300 Folders, ignore
    /// lists and whitelists of 1 to 1,000 parts elsewhere): 1,000 entries over parts 300 deep 2.7 to 2.9 ms (0.89 ns a step), 100
    /// over 30 deep 13 to 29 us; the parts' folder first in the list, the same at any length.
    fn keeps(&self, state: &State, part: InstanceId) -> (bool, f64) {
        let (ids, places, count, white): (&[InstanceId], &[u32], usize, bool) = match self {
            Keep::All => return (true, 0.0),
            Keep::Ignore(ignored) => (std::slice::from_ref(ignored), &[0], 1, false),
            Keep::IgnoreList(list) => (&list.ids, &list.places, list.count, false),
            Keep::WhiteList(list) => (&list.ids, &list.places, list.count, true),
        };
        if ids.is_empty() {
            return (!white, 0.0);
        }
        // (the first instance of the list, in its order, that is the part or an ancestor: its place and how far up it is)
        let mut first: Option<(u32, u32)> = None;
        let (mut current, mut up) = (Some(part), 0u32);
        while let Some(node) = current {
            if let Ok(at) = ids.binary_search(&node)
                && first.is_none_or(|(place, _)| places[at] < place)
            {
                first = Some((places[at], up));
            }
            // (nothing further up changes the answer, nor, once the list's first instance is found, what it costs; else the walk
            // goes on to the top, as a client's check of each instance before the first it finds does)
            if first.is_some_and(|(place, _)| place == 0) {
                break;
            }
            current = state.dom.parent_of(node);
            up += u32::from(current.is_some());
        }
        let steps = match first {
            Some((place, at)) => f64::from(place) * f64::from(up) + f64::from(at),
            None => count as f64 * f64::from(up),
        };
        (first.is_some() == white, steps * crate::cost::filter_step(up))
    }
}

/// The classes and the property the queries here read, resolved once (at install)
#[derive(Clone, Copy)]
struct Names {
    base_part: ClassId,
    terrain: ClassId,
    world_root: ClassId,
    can_collide: NameId,
}

impl Names {
    fn of(engine: &Engine) -> Option<Names> {
        let reflection = &engine.reflection;
        Some(Names {
            base_part: reflection.class_id("BasePart")?,
            terrain: reflection.class_id("Terrain")?,
            world_root: reflection.class_id("WorldRoot")?,
            can_collide: reflection.name_id("CanCollide")?,
        })
    }
}

/// A WorldRoot's world as a call here walks it: its index (world/broadphase.rs: the Workspace's, or a WorldModel's own), or none
/// for a WorldModel outside the DataModel, which has no world.
struct World(Option<u32>);

/// A part a world's grid gives a walk: its solid, its world-aligned box (centre, half size) and CanQuery and CanCollide (not
/// false)
struct Candidate<'a> {
    part: InstanceId,
    solid: &'a Solid,
    reach: (Vec3, Vec3),
    can_query: bool,
    can_collide: bool,
}

impl Candidate<'_> {
    fn of(part: InstanceId, entry: &Entry) -> Candidate<'_> {
        Candidate { part, solid: &entry.solid, reach: query::world_reach(&entry.solid), can_query: entry.can_query, can_collide: entry.can_collide }
    }
}

impl World {
    /// The world of the WorldRoot `root` for one call, its index brought up to the dom's changes (world/broadphase.rs open: for a
    /// WorldModel's, what taking its change log or a walk of its world costs, ree's own price, cost.rs FIND_NODE and
    /// MODEL_INDEX_PART).
    fn open(engine: &Engine, root: InstanceId) -> World {
        let (world, opened) = broadphase::open(engine, root);
        query::charge_query(engine, opened);
        World(world)
    }

    /// Whether its WorldRoot is in the DataModel (a WorldModel outside it has no world: measured)
    fn inside(&self) -> bool {
        self.0.is_some()
    }

    /// Gives `visit` each part of the world's grid walk over the box `lo`..`hi` (the box clamped to its centre +- 7,500 studs, as
    /// a box query's: world/query.rs CLAMP), a cell's unanchored parts first, in the walk's order (a client's: world/grid.rs), until
    /// it returns false; the box as clamped. A part waiting for the spatial index is in no cell: a client's index does not hold it
    /// yet, and it costs its queries nothing (1,000 of them 0.42 us).
    fn visit(&mut self, engine: &Engine, lo: Vec3, hi: Vec3, mut visit: impl FnMut(Candidate) -> bool) -> (Vec3, Vec3) {
        // (from the bounds' halves: their sum or difference can overflow where the halves do not)
        let (center, extent) = (lo * 0.5 + hi * 0.5, (hi * 0.5 - lo * 0.5).abs());
        let (lo, hi, _) = query::clamped(center, extent);
        let Some(world) = self.0 else { return (lo, hi) };
        if !(lo.is_finite() && hi.is_finite()) {
            return (lo, hi);
        }
        broadphase::visit_box(&mut engine.state.borrow_mut(), world, lo, hi, grid::UNANCHORED, |part, entry| visit(Candidate::of(part, entry)));
        (lo, hi)
    }
}

/// What a Region3 query found: the parts it keeps (at most its limit), how many it found, how many of those what it ignores or
/// keeps left out, what its checks of them and the walk that put them in order cost, and the other parts its world's tree gave
/// it: those with CanQuery and CanCollide off, those near the region and those further out (its charge: [`charge_region`]).
#[derive(Default)]
struct Found {
    parts: Vec<InstanceId>,
    found: usize,
    excluded: usize,
    checks: f64,
    hidden: usize,
    near: usize,
    far: usize,
}

/// A Region3 query of the WorldRoot `root`: the parts of its world whose box meets [lo, hi], kept by `keep`, the first `limit`
/// of them. Each part the tree gives it is told apart as it comes, and only the parts found are put in the world's order, when
/// `ordered` (IsRegion3Empty* only asks whether one is kept: in the tree's order). What it keeps is asked only until it has them
/// (measured: a client's ancestor checks stop once its result is full, and IsRegion3Empty's at the first part it keeps).
fn find(engine: &Engine, root: InstanceId, (lo, hi): (Vec3, Vec3), keep: Keep, limit: usize, ordered: bool) -> Found {
    let mut found = Found::default();
    // A region with a bound or a centre that is not finite finds nothing and tests no part: a client's Region3 holds its centre
    // (the bounds' mean, in float32) and size, which a NaN or infinite bound makes NaN or infinite, as finite bounds whose sum
    // overflows make its centre (measured, Studio 0.748, a playing client: -inf..inf, a NaN bound, one infinite side, and 2e38
    // to 2e38 or 1.9e38 to 2.1e38 along x, around 1,000 parts, find none, 0.26 to 0.27 us beside 1,000 parts as beside none,
    // where a finite empty region costs 0.41 to 0.52; -3e38 to 3e38, whose size alone overflows, still finds parts).
    if !(lo.is_finite() && hi.is_finite() && ((lo + hi) * 0.5).is_finite()) {
        return found;
    }
    let mut kept: Vec<InstanceId> = Vec::new();
    let mut world = World::open(engine, root);
    // (the walk of the region's cells, its box clamped about its centre as a box query's: a part kept must meet the clamped box
    // too)
    let (walk_lo, walk_hi) = (lo.min(hi), lo.max(hi));
    let (clamp_lo, clamp_hi, clamped) = query::clamped(walk_lo * 0.5 + walk_hi * 0.5, (walk_hi * 0.5 - walk_lo * 0.5).abs());
    world.visit(engine, walk_lo, walk_hi, |part| {
        if !within(part.reach, lo, hi, BAND) || (clamped && !within(part.reach, clamp_lo, clamp_hi, 0.0)) {
            found.far += 1;
        } else if !part.can_query && !part.can_collide {
            found.hidden += 1;
        } else if !meets(part.reach, lo, hi) {
            found.near += 1;
        } else {
            kept.push(part.part);
        }
        true
    });
    found.found = kept.len();
    let _ = ordered;
    let state = engine.state.borrow();
    for part in kept {
        if found.parts.len() >= limit {
            break;
        }
        let (kept, seconds) = keep.keeps(&state, part);
        found.checks += seconds;
        if kept {
            found.parts.push(part);
        } else {
            found.excluded += 1;
        }
    }
    found
}

/// What a Region3 query costs beyond its call and its list's read (cost.rs REGION_FOUND and after; world/mod.rs ignore_list):
/// each part found, each one what it ignores or keeps leaves out, its checks of them ([`Keep::keeps`]) and the walk that ordered
/// them, each other part its world's tree gave it (CanQuery and CanCollide off, near the region, further out), and each part it
/// lists (IsRegion3Empty*: once, when it finds one it keeps).
fn charge_region(engine: &Engine, found: &Found, emptiness: bool) {
    let listed = match (emptiness, found.parts.is_empty()) {
        (false, _) => found.parts.len() as f64 * crate::cost::REGION_LISTED,
        (true, false) => crate::cost::REGION_ANY,
        (true, true) => 0.0,
    };
    let seconds = found.found as f64 * crate::cost::REGION_FOUND
        + found.excluded as f64 * crate::cost::REGION_EXCLUDED
        + found.checks
        + found.hidden as f64 * crate::cost::REGION_HIDDEN
        + found.near as f64 * crate::cost::REGION_NEAR
        + found.far as f64 * crate::cost::REGION_FAR
        + listed;
    query::charge_query(engine, seconds);
}

/// FindPartsInRegion3(region, ignoreDescendantsInstance, maxParts) and its lowercase alias.
fn find_parts(lua: &Lua, engine: &Rc<Engine>, root: InstanceId, args: &Args) -> Result<LuaMultiValue, Flow> {
    let limit = max_parts_argument(lua, engine, args, 3)?;
    let ignored = ignore_argument(args, 2)?;
    let region = region_argument(engine, args, 1)?;
    let found = find(engine, root, region, ignored.map_or(Keep::All, Keep::Ignore), limit, true);
    charge_region(engine, &found, false);
    query::parts_table(lua, engine, found.parts)
}

/// FindPartsInRegion3WithIgnoreList / WithWhiteList(region, list, maxParts): the list read as world/mod.rs ordered_ignore_list
/// reads an instance list, each entry charged as it is read, also on a call that raises after it.
fn find_parts_listed(lua: &Lua, engine: &Rc<Engine>, root: InstanceId, args: &Args, white: bool) -> Result<LuaMultiValue, Flow> {
    let limit = max_parts_argument(lua, engine, args, 3)?;
    let listed = super::ordered_ignore_list(engine, args, 2)?;
    let region = region_argument(engine, args, 1)?;
    let keep = if white { Keep::WhiteList(&listed) } else { Keep::IgnoreList(&listed) };
    let found = find(engine, root, region, keep, limit, true);
    charge_region(engine, &found, false);
    query::parts_table(lua, engine, found.parts)
}

/// A listed part's collision test against another (`ball` a sphere, anything else its box): how far they overlap, negative
/// when apart.
fn depth(a: &Solid, b: &Solid) -> f32 {
    match (a.shape, b.shape) {
        (Shape::Ball(ra), Shape::Ball(rb)) => ra + rb - frame::translation(&a.frame).distance(frame::translation(&b.frame)),
        (Shape::Ball(radius), _) => ball_box(frame::translation(&a.frame), radius, b),
        (_, Shape::Ball(radius)) => ball_box(frame::translation(&b.frame), radius, a),
        _ => {
            let (a, b) = (query::Oriented::of(&a.frame, a.half_extents()), query::Oriented::of(&b.frame, b.half_extents()));
            let offset = b.center - a.center;
            query::sat_axes(&a, &b).into_iter().map(|axis| a.radius(axis) + b.radius(axis) - offset.dot(axis).abs()).fold(f32::INFINITY, f32::min)
        }
    }
}

/// What ArePartsTouchingOthers' walk was given for one listed part (its charge: cost.rs TOUCHING_PAIR and after): the parts it
/// tested, those it skipped for being listed or for having CanCollide off, and those further than [`REACH`] from its box, until
/// one touched.
#[derive(Clone, Copy, Default)]
struct Walked {
    tested: usize,
    listed: usize,
    off: usize,
    far: usize,
    touched: bool,
}

/// A sphere against a solid's box: its radius less the distance to the box, or plus how deep its centre is inside.
fn ball_box(center: Vec3, radius: f32, solid: &Solid) -> f32 {
    let local = solid.frame.inverse().transform_point3(center);
    let half = solid.half_extents();
    let closest = local.clamp(-half, half);
    if closest == local { radius + (half - local.abs()).min_element() } else { radius - (local - closest).length() }
}

/// ArePartsTouchingOthers(partList, overlapIgnored): its world opened once for the call ([`World::open`]) and given back.
fn touching(lua: &Lua, engine: &Rc<Engine>, names: Names, root: InstanceId, args: &Args) -> Result<LuaMultiValue, Flow> {
    let overlap = if args.get(2).is_nil() { OVERLAP_IGNORED } else { camera::float_argument(lua, engine, args, 2)? };
    let entries = touching_entries(engine, args, 1)?;
    let mut world = World::open(engine, root);
    touching_in(engine, names, root, &mut world, &entries, overlap)
}

/// [`touching`] in its world, the arguments read.
fn touching_in(engine: &Rc<Engine>, names: Names, root: InstanceId, world: &mut World, entries: &[Option<InstanceId>], overlap: f32) -> Result<LuaMultiValue, Flow> {
    let reflection = &engine.reflection;
    {
        let state = engine.state.borrow();
        for entry in entries {
            let Some(id) = *entry else { return raise("All objects passed to ArePartsTouchingOthers must be BaseParts, got a Nil.") };
            let class = reflection.class(state.dom.at(id).class);
            if !class.is_a(names.base_part) {
                return raise(format!("All objects passed to ArePartsTouchingOthers must be BaseParts, got a {}.", class.name));
            }
            // a part of its world: the WorldRoot its nearest (dom.rs Dom::home, no walk up), and in the DataModel (a WorldModel in
            // nil has no world, measured); Terrain is the Workspace's child, and not among its world's parts
            let member = if class.is_a(names.terrain) {
                state.dom.parent_of(id) == Some(root)
            } else {
                super::world_root_of(&state, reflection, names.world_root, id) == Some(root) && world.inside()
            };
            if !member {
                return raise("All parts passed to ArePartsTouchingOthers must be in the WorldRoot");
            }
        }
    }
    // It takes into the spatial index what came into its world, as other spatial queries do, and pays for it as they do
    // (measured: FindPartsInRegion3 finds a part made just before it after it; 10, 100 and 1,000 parts taken in cost it 12.7,
    // 120 and 1,233 us, in a WorldModel 12.2, 110 and 1,080). A WorldModel's index takes the dom's changes in its world when a
    // listed part is first walked (none for an empty list, or one of Terrain and parts with CanCollide off).
    query::take_in_of(engine, root);
    let listed: FxHashSet<InstanceId> = entries.iter().flatten().copied().collect();
    // Each listed part is tested once however often it is listed; a repeat costs what its first test did, as the entry walk was
    // measured.
    let mut walks: FxHashMap<InstanceId, Walked> = FxHashMap::default();
    let (mut checked, mut total, mut tested_parts, mut touches) = (0usize, Walked::default(), 0usize, false);
    for &part in entries.iter().flatten() {
        // (Terrain and a part with CanCollide off are not tested)
        let own = {
            let state = engine.state.borrow();
            let skipped = reflection.class(state.dom.at(part).class).is_a(names.terrain) || state.dom.boolean(part, names.can_collide) == Some(false);
            (!skipped).then(|| query::solid(&state, engine, part))
        };
        let Some(own) = own else { continue };
        checked += 1;
        let walked = *walks.entry(part).or_insert_with(|| walk(engine, world, &listed, &own, overlap));
        (total.tested, total.listed, total.off, total.far) = (total.tested + walked.tested, total.listed + walked.listed, total.off + walked.off, total.far + walked.far);
        tested_parts += usize::from(walked.tested > 0);
        if walked.touched {
            touches = true;
            break;
        }
    }
    if !entries.is_empty() {
        let seconds = crate::cost::TOUCHING_LIST
            + entries.len() as f64 * crate::cost::TOUCHING_ENTRY
            + checked as f64 * crate::cost::TOUCHING_CHECKED
            + total.tested as f64 * crate::cost::TOUCHING_PAIR
            + tested_parts as f64 * crate::cost::TOUCHING_TESTED
            + total.listed as f64 * crate::cost::TOUCHING_LISTED_NEAR
            + total.off as f64 * crate::cost::TOUCHING_OFF_NEAR
            + total.far as f64 * crate::cost::TOUCHING_FAR;
        query::charge_query(engine, seconds);
    }
    Ok(values([LuaValue::Boolean(touches)]))
}

/// A listed part (`own`) against the parts of its world its tree gives near its box (within [`REACH`] of it: a superset), one
/// at a time and in no particular order, until one touches it: each part given is counted, tested or skipped, so what the walk
/// is given is what it is charged for (a client's walk costs the same way and stops there too: cost.rs TOUCHING_PAIR).
fn walk(engine: &Engine, world: &mut World, listed: &FxHashSet<InstanceId>, own: &Solid, overlap: f32) -> Walked {
    let (center, reach) = query::world_reach(own);
    let (lo, hi) = ((center - reach) - Vec3::splat(REACH), (center + reach) + Vec3::splat(REACH));
    let mut walked = Walked::default();
    // (a box that is not finite meets nothing)
    if !(lo.is_finite() && hi.is_finite()) {
        return walked;
    }
    world.visit(engine, lo, hi, |near| {
        if near.part == own.part {
            return true;
        }
        let (c, r) = near.reach;
        if !((c - r).cmple(hi).all() && (c + r).cmpge(lo).all()) {
            walked.far += 1;
        } else if listed.contains(&near.part) {
            walked.listed += 1;
        } else if !near.can_collide {
            walked.off += 1;
        } else {
            walked.tested += 1;
            if depth(own, near.solid) > overlap {
                walked.touched = true;
                return false;
            }
        }
        true
    });
    walked
}

pub fn install(engine: &Rc<Engine>) {
    let Some(names) = Names::of(engine) else { return };
    // (a call that raises costs what any engine call's error costs a client, charged where ree raises it: lua/cfunction.rs
    // charge_raised, cost.rs RAISED; measured, Studio 0.748, a client's Region3 query that raises costs past an empty pcall,
    // its call included, 3.7 to 4.5 us for FindPartsInRegion3's argument errors, 3.8 to 6.0 for IsRegion3Empty's and 6.2 to 9.5
    // for FindPartsInRegion3WithIgnoreList given a list whose first entry is no Instance ({ 5 }), which ree leaves at RAISED
    // and the call, 3.8 (not matched); ArePartsTouchingOthers 4.6 to 9.2, so it pays cost.rs TOUCHING_RAISED past RAISED)
    let members: [(&str, MethodFn); 7] = [
        ("FindPartsInRegion3", Rc::new(find_parts)),
        ("findPartsInRegion3", Rc::new(find_parts)),
        ("FindPartsInRegion3WithIgnoreList", Rc::new(|lua, engine, root, args| find_parts_listed(lua, engine, root, args, false))),
        ("FindPartsInRegion3WithWhiteList", Rc::new(|lua, engine, root, args| find_parts_listed(lua, engine, root, args, true))),
        ("IsRegion3Empty", Rc::new(|_, engine, root, args| {
            let ignored = ignore_argument(args, 2)?;
            let region = region_argument(engine, args, 1)?;
            let found = find(engine, root, region, ignored.map_or(Keep::All, Keep::Ignore), 1, false);
            charge_region(engine, &found, true);
            Ok(values([LuaValue::Boolean(found.parts.is_empty())]))
        })),
        ("IsRegion3EmptyWithIgnoreList", Rc::new(|_, engine, root, args| {
            let listed = super::ordered_ignore_list(engine, args, 2)?;
            let region = region_argument(engine, args, 1)?;
            let found = find(engine, root, region, Keep::IgnoreList(&listed), 1, false);
            charge_region(engine, &found, true);
            Ok(values([LuaValue::Boolean(found.parts.is_empty())]))
        })),
        ("ArePartsTouchingOthers", Rc::new(move |lua, engine, root, args| {
            let result = touching(lua, engine, names, root, args);
            if matches!(result, Err(Flow::Error(_))) {
                query::charge_query(engine, crate::cost::TOUCHING_RAISED);
            }
            result
        })),
    ];
    let mut behaviors = engine.behaviors.borrow_mut();
    for (member, function) in members {
        if let Some(name) = engine.reflection.name_id(member) {
            behaviors.add_method(names.world_root, name, function);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{run, start, start_play};
    use super::{broadphase, query};

    /// The Region3 queries as a playing client answers them (Studio 0.748; world/region.rs's notes): the parts of the WorldRoot's
    /// own world the spatial index holds, their world-aligned collision boxes against the region's, touching included; CanQuery
    /// off alone hides nothing; the ignored instance, ignore list and whitelist with their descendants; maxParts as an int, 20 when
    /// nil, none past 0 for all; the argument errors from the last argument to the first.
    #[test]
    fn region3_queries_find_what_a_client_finds() {
        let (lua, engine) = start();
        run(
            &lua,
            &engine,
            r##"
            local function Fails(Message, F, ...)
                local Ok, E = pcall(F, ...)
                if Ok or tostring(E) ~= Message then
                    error(`{Message}: {E}`, 2)
                end
            end
            local function Names(List)
                local Out = {}
                for I, P in List do
                    Out[I] = P.Name
                end
                table.sort(Out)
                return table.concat(Out, " ")
            end
            local function Index(Root)
                (Root or workspace):GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            end
            local Folder = Instance.new("Folder")
            Folder.Parent = workspace
            local function Part(Name, At, Size, Props)
                local P = Instance.new("Part")
                P.Name = Name
                P.Anchored = true
                P.Size = Size or Vector3.new(0.2, 0.2, 0.2)
                P.CFrame = if typeof(At) == "CFrame" then At else CFrame.new(At)
                for K, V in Props or {} do
                    (P :: any)[K] = V
                end
                P.Parent = Folder
                return P
            end
            local Region = Region3.new(Vector3.new(500, 500, 500), Vector3.new(508, 508, 508))
            local Find = workspace.FindPartsInRegion3
            -- a part that just entered the Workspace is not in the spatial index, and these queries take none in
            local Fresh = Part("Fresh", Vector3.new(504, 504, 504))
            assert(#workspace:FindPartsInRegion3(Region) == 0 and workspace:IsRegion3Empty(Region))
            assert(#workspace:FindPartsInRegion3WithIgnoreList(Region, {}) == 0 and #workspace:FindPartsInRegion3(Region) == 0)
            Index()
            assert(Names(workspace:FindPartsInRegion3(Region)) == "Fresh" and not workspace:IsRegion3Empty(Region))
            Fresh.CFrame = CFrame.new(504, 520, 504)
            assert(workspace:IsRegion3Empty(Region), "a moved part counts at once")
            Fresh:Destroy()

            Part("Inside", Vector3.new(504, 504, 504))
            Part("TouchHigh", Vector3.new(509, 504, 504), Vector3.new(2, 2, 2))
            Part("TouchLow", Vector3.new(499, 504, 504), Vector3.new(2, 2, 2))
            -- its box's min is 508.1 - 0.1 in float32, 6e-6 past 508 in double: a client does not find it
            Part("Past", Vector3.new(508.1, 504, 504))
            -- turned, its box reaches 1.414 from its centre: 507.79 and 508.09
            Part("Turned", CFrame.new(509.2, 504, 504) * CFrame.Angles(0, 0, math.rad(45)), Vector3.new(2, 2, 2))
            Part("TurnedAway", CFrame.new(509.5, 504, 504) * CFrame.Angles(0, 0, math.rad(45)), Vector3.new(2, 2, 2))
            -- a ball by its cube (whose corner is in), a cylinder by its length and diameter (0.5 from its axis, not 1.5)
            Part("Ball", Vector3.new(508.8, 508.8, 508.8), Vector3.new(2, 2, 2), { Shape = Enum.PartType.Ball })
            Part("Cylinder", CFrame.new(509.2, 504, 504) * CFrame.Angles(0, 0, math.rad(90)), Vector3.new(4, 3, 1), { Shape = Enum.PartType.Cylinder })
            Part("NoQuery", Vector3.new(503, 503, 503), nil, { CanQuery = false })
            Part("NoCollide", Vector3.new(503, 505, 503), nil, { CanCollide = false })
            Part("Neither", Vector3.new(503, 506, 503), nil, { CanCollide = false, CanQuery = false })
            Part("Clear", Vector3.new(503, 507, 503), nil, { Transparency = 1, CanTouch = false })
            local Model = Instance.new("Model")
            Model.Parent = Folder
            local InModel = Part("InModel", Vector3.new(505, 505, 505))
            InModel.Parent = Model
            local Outside = Part("Outside", Vector3.new(504, 504, 504))
            Outside.Parent = game:GetService("ReplicatedStorage")
            local World = Instance.new("WorldModel")
            World.Parent = workspace
            Part("InWorldModel", Vector3.new(504, 505, 504)).Parent = World
            Index()
            Index(World)
            local All = "Ball Clear InModel Inside NoCollide NoQuery TouchHigh TouchLow Turned"
            assert(Names(workspace:FindPartsInRegion3(Region)) == All, Names(workspace:FindPartsInRegion3(Region)))
            assert(Names(workspace:findPartsInRegion3(Region)) == All)
            assert(Names(World:FindPartsInRegion3(Region)) == "InWorldModel")
            assert(not table.find(workspace:FindPartsInRegion3(Region3.new(-Vector3.one * 1e4, Vector3.one * 1e4), nil, 0), workspace.Terrain))
            -- what each ignores or keeps, with descendants
            assert(Names(workspace:FindPartsInRegion3(Region, Model)) == "Ball Clear Inside NoCollide NoQuery TouchHigh TouchLow Turned")
            assert(Names(workspace:FindPartsInRegion3(Region, Folder)) == "" and workspace:IsRegion3Empty(Region, Folder))
            assert(Names(workspace:FindPartsInRegion3(Region, Outside)) == All and not workspace:IsRegion3Empty(Region, Outside))
            local Ball, Inside = Folder.Ball, Folder.Inside
            assert(Names(workspace:FindPartsInRegion3WithIgnoreList(Region, { Model, nil, Ball })) == "Clear Inside NoCollide NoQuery TouchHigh TouchLow Turned")
            assert(Names(workspace:FindPartsInRegion3WithIgnoreList(Region, { a = Ball } :: any)) == All)
            assert(Names(workspace:FindPartsInRegion3WithWhiteList(Region, { Model, nil, Ball })) == "Ball InModel")
            assert(Names(workspace:FindPartsInRegion3WithWhiteList(Region, {})) == "" and Names(workspace:FindPartsInRegion3WithWhiteList(Region, { workspace })) == All)
            assert(workspace:IsRegion3EmptyWithIgnoreList(Region, { Folder }) and not workspace:IsRegion3EmptyWithIgnoreList(Region, { Model, Ball }))
            -- maxParts, an int: 20 when nil, every part when not past 0
            for I = 1, 25 do
                Part("Extra" .. I, Vector3.new(501 + I * 0.2, 501, 501))
            end
            Index()
            local Count = function(...) return #workspace:FindPartsInRegion3(Region, nil, ...) end
            assert(Count() == 20 and Count(nil) == 20 and Count(0) == 34 and Count(-1) == 34 and Count(1e10) == 34 and Count(0 / 0) == 34)
            assert(Count(1) == 1 and Count(2.5) == 3 and Count(2.4) == 2 and Count(-2.5) == 34 and Count("3") == 3 and Count(" 3.7 ") == 4)
            assert(Count(true) == 1 and Count(false) == 34)
            assert(#workspace:FindPartsInRegion3WithIgnoreList(Region, {}) == 20 and #workspace:FindPartsInRegion3WithWhiteList(Region, { Folder }, 2.5) == 3)
            -- an inverted region keeps its corners: a part reaching past both is found
            local Inverted = Region3.new(Vector3.new(901, 901, 901), Vector3.new(899, 899, 899))
            Part("Big", Vector3.new(900, 900, 900), Vector3.new(20, 20, 20))
            Part("Small", Vector3.new(903, 900, 900), Vector3.new(0.5, 0.5, 0.5))
            Index()
            assert(Names(workspace:FindPartsInRegion3(Inverted)) == "Big")
            -- results
            local Result = workspace:FindPartsInRegion3(Region)
            assert(typeof(Result) == "table" and getmetatable(Result) == nil and typeof(workspace:IsRegion3Empty(Region)) == "boolean")
            -- errors, the last argument checked first
            Fails("Argument 1 missing or nil", Find, workspace)
            Fails("Argument 1 missing or nil", Find, workspace, nil)
            Fails("Unable to cast double to Region3", Find, workspace, 5)
            Fails("Unable to cast Array to Region3", workspace.FindPartsInRegion3WithIgnoreList, workspace, {}, {})
            Fails("Unable to cast CoordinateFrame to Region3", workspace.IsRegion3Empty, workspace, CFrame.new())
            Fails("Unable to cast Region3int16 to Region3", Find, workspace, Region3int16.new())
            Fails("Unable to cast KeyCode to Region3", workspace.IsRegion3EmptyWithIgnoreList, workspace, Enum.KeyCode.A, {})
            Fails("Unable to cast value to Object", Find, workspace, Region, 5)
            Fails("Unable to cast value to Object", workspace.IsRegion3Empty, workspace, 5, 5)
            Fails("Unable to cast string to int", Find, workspace, 5, 5, "x")
            Fails("Unable to cast Instance to int", Find, workspace, Region, nil, workspace)
            Fails("Unable to cast KeyCode to int", workspace.FindPartsInRegion3WithWhiteList, workspace, Region, {}, Enum.KeyCode.A)
            Fails("Argument 2 missing or nil", workspace.FindPartsInRegion3WithIgnoreList, workspace)
            Fails("Argument 2 missing or nil", workspace.IsRegion3EmptyWithIgnoreList, workspace, Region)
            Fails("Argument 1 missing or nil", workspace.FindPartsInRegion3WithIgnoreList, workspace, nil, {})
            Fails("Expected table argument, got nil", workspace.FindPartsInRegion3WithWhiteList, workspace, Region, nil)
            Fails("Expected table argument, got userdata", workspace.FindPartsInRegion3WithWhiteList, workspace, Region, workspace)
            Fails("Expected table argument, got vector", workspace.IsRegion3EmptyWithIgnoreList, workspace, Region, Vector3.one)
            Fails("Unable to cast value to Object", workspace.FindPartsInRegion3WithIgnoreList, workspace, 5, { 5 }, 3)
            Fails("Unable to cast value to Object", workspace.FindPartsInRegion3WithIgnoreList, workspace, Region, { Inside, "Inside" })
            Fails("Expected ':' not '.' calling member function FindPartsInRegion3", Find, nil, Region)
            Fails("Expected ':' not '.' calling member function IsRegion3Empty", workspace.IsRegion3Empty, Inside, Region)
            Folder:Destroy()
            World:Destroy()
            "##,
        );
        assert!(!engine.state.borrow().log_history.iter().any(|entry| entry.text.contains("is not emulated")));
    }

    /// A WorldModel's own world, as a playing client keeps it (Studio 0.748): outside the Workspace, its parts wait for a spatial
    /// query of it (an ArePartsTouchingOthers too) or a physics step, and the Workspace's queries never see them; a part that
    /// moves between worlds comes into its new one anew; a WorldModel moved within the DataModel keeps its parts in, one in nil
    /// has no world (it finds nothing, and its parts are not in it) and brings them in anew when it comes back.
    #[test]
    fn region3_queries_see_a_worldmodels_own_world() {
        let (lua, engine) = start_play();
        run(
            &lua,
            &engine,
            r##"
            local function Names(List)
                local Out = {}
                for I, P in List do
                    Out[I] = P.Name
                end
                table.sort(Out)
                return table.concat(Out, " ")
            end
            local function Part(Name, At, Parent, Size)
                local P = Instance.new("Part")
                P.Name = Name
                P.Anchored = true
                P.Size = Size or Vector3.new(0.2, 0.2, 0.2)
                P.CFrame = CFrame.new(At)
                P.Parent = Parent
                return P
            end
            local function Index(Root)
                (Root or workspace):GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            end
            local Region = Region3.new(Vector3.new(500, 500, 500), Vector3.new(508, 508, 508))
            local Holder = Instance.new("Folder")
            Holder.Name = "Holder"
            Holder.Parent = game:GetService("ReplicatedStorage")
            local World = Instance.new("WorldModel")
            World.Parent = Holder
            -- a part made in it waits for its spatial index; its own query takes it in, the Workspace's never see it nor take it in
            local P1 = Part("P1", Vector3.new(504, 504, 504), World)
            assert(#World:FindPartsInRegion3(Region) == 0 and World:IsRegion3Empty(Region) and #workspace:FindPartsInRegion3(Region) == 0)
            Index()
            assert(#World:FindPartsInRegion3(Region) == 0)
            -- and the other way round: a part made in the Workspace waits for the Workspace's index alone
            local W1 = Part("W1", Vector3.new(504, 504, 504), workspace)
            Index(World)
            assert(#workspace:FindPartsInRegion3(Region) == 0)
            Index()
            assert(Names(workspace:FindPartsInRegion3(Region)) == "W1")
            W1:Destroy()
            assert(Names(World:FindPartsInRegion3(Region)) == "P1" and not World:IsRegion3Empty(Region) and #workspace:FindPartsInRegion3(Region) == 0)
            -- moved out of the region and back: at once
            P1.CFrame = CFrame.new(504, 600, 504)
            assert(#World:FindPartsInRegion3(Region) == 0 and World:IsRegion3Empty(Region))
            P1.CFrame = CFrame.new(504, 504, 504)
            assert(Names(World:FindPartsInRegion3(Region)) == "P1")
            -- its ArePartsTouchingOthers takes new parts in too
            Part("P2", Vector3.new(503, 503, 503), World)
            assert(Names(World:FindPartsInRegion3(Region)) == "P1")
            assert(not World:ArePartsTouchingOthers({ P1 }) and Names(World:FindPartsInRegion3(Region)) == "P1 P2")
            -- a part moved in from the Workspace comes into its world anew, and leaves the Workspace's
            local P3 = Part("P3", Vector3.new(505, 505, 505), workspace)
            Index()
            assert(Names(workspace:FindPartsInRegion3(Region)) == "P3")
            P3.Parent = World
            assert(Names(World:FindPartsInRegion3(Region)) == "P1 P2" and #workspace:FindPartsInRegion3(Region) == 0)
            Index(World)
            assert(Names(World:FindPartsInRegion3(Region)) == "P1 P2 P3")
            -- two of its parts touching, which the Workspace does not hold
            local A = Part("A", Vector3.new(700, 700, 700), World, Vector3.new(2, 2, 2))
            local B = Part("B", Vector3.new(700, 701, 700), World, Vector3.new(2, 2, 2))
            assert(World:ArePartsTouchingOthers({ A }) and World:ArePartsTouchingOthers({ A }, 0.999) and not World:ArePartsTouchingOthers({ A }, 1) and not World:ArePartsTouchingOthers({ A, B }))
            local Ok, E = pcall(workspace.ArePartsTouchingOthers, workspace, { A })
            assert(not Ok and E == "All parts passed to ArePartsTouchingOthers must be in the WorldRoot", E)
            -- in nil it has no world; back in the DataModel, its parts come in anew
            World.Parent = nil
            assert(#World:FindPartsInRegion3(Region) == 0 and World:IsRegion3Empty(Region) and not World:ArePartsTouchingOthers({}))
            Ok, E = pcall(World.ArePartsTouchingOthers, World, { A })
            assert(not Ok and E == "All parts passed to ArePartsTouchingOthers must be in the WorldRoot", E)
            World.Parent = Holder
            assert(#World:FindPartsInRegion3(Region) == 0)
            Index(World)
            assert(Names(World:FindPartsInRegion3(Region)) == "P1 P2 P3")
            -- moved into the Workspace, it keeps them, and the Workspace's queries still do not see them
            World.Parent = workspace
            assert(Names(World:FindPartsInRegion3(Region)) == "P1 P2 P3" and #workspace:FindPartsInRegion3(Region) == 0)
            -- a part moved out of it into the Workspace comes into the Workspace's world anew
            P3.Parent = workspace
            assert(#workspace:FindPartsInRegion3(Region) == 0 and Names(World:FindPartsInRegion3(Region)) == "P1 P2")
            Index()
            assert(Names(workspace:FindPartsInRegion3(Region)) == "P3")
            P3:Destroy()
            World:Destroy()
            -- (the physics step takes new parts in: below)
            local Later = Instance.new("WorldModel")
            Later.Name = "Later"
            Later.Parent = Holder
            Part("Q", Vector3.new(504, 504, 504), Later)
            assert(#Later:FindPartsInRegion3(Region) == 0)
            "##,
        );
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(
            &lua,
            &engine,
            r##"
            local Holder = game:GetService("ReplicatedStorage").Holder
            assert(#Holder.Later:FindPartsInRegion3(Region3.new(Vector3.new(500, 500, 500), Vector3.new(508, 508, 508))) == 1)
            Holder:Destroy()
            "##,
        );
    }

    /// The trees the queries here walk do work for what changes in their own world (a client keeps an index for each world): a
    /// WorldModel's index walks its world when it is first made and not again for writes, attributes or parts made elsewhere
    /// (a 2,000-part WorldModel queried 50 times with another part's attribute and position written and a part made between),
    /// for a write or a move inside it, or for other WorldModels queried in turn; a WorldModel outside the DataModel makes none;
    /// the Workspace's is the broadphase's (no walk of the Workspace), whose walks are given none of a WorldModel's parts in it;
    /// ArePartsTouchingOthers' walk stops at the first part touching (a part with 300 stacked on it is given at most 2), and an
    /// empty list makes no index.
    #[test]
    fn region3_queries_follow_their_worlds_and_touching_stops_at_a_touch() {
        let (lua, engine) = start();
        run(
            &lua,
            &engine,
            r##"
            local Store = game:GetService("ReplicatedStorage")
            local function Part(At, Parent, Size)
                local P = Instance.new("Part")
                P.Anchored = true
                P.Size = Size or Vector3.new(2, 2, 2)
                P.CFrame = CFrame.new(At)
                P.Parent = Parent
                return P
            end
            shared.World = Instance.new("WorldModel")
            shared.World.Parent = Store
            for I = 1, 2000 do
                Part(Vector3.new(I % 50 * 3, I // 50 * 3, 0), shared.World)
            end
            shared.Inside = Part(Vector3.new(500, 0, 0), shared.World)
            shared.Worlds = {}
            for I = 1, 10 do
                local W = Instance.new("WorldModel")
                W.Parent = Store
                for J = 1, 10 do
                    Part(Vector3.new(J * 3, 0, 0), W)
                end
                table.insert(shared.Worlds, W)
            end
            shared.Lone = Part(Vector3.new(0, -500, 0), workspace)
            shared.Stacked = Part(Vector3.new(0, 500, 0), workspace)
            for _ = 1, 300 do
                Part(Vector3.new(0, 500.5, 0), workspace)
            end
            local Nested = Instance.new("WorldModel")
            Nested.Parent = workspace
            for I = 1, 500 do
                Part(Vector3.new(I % 20, 800, I // 20), Nested, Vector3.new(0.5, 0.5, 0.5))
            end
            shared.R = Region3.new(Vector3.new(490, -10, -10), Vector3.new(510, 10, 10))
            shared.NestedRegion = Region3.new(Vector3.new(-5, 795, -5), Vector3.new(30, 805, 30))
            -- (everything taken into the spatial index)
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            for _, W in shared.Worlds do
                W:ArePartsTouchingOthers({})
            end
            shared.World:ArePartsTouchingOthers({})
            Nested:ArePartsTouchingOthers({})
            "##,
        );
        assert_eq!(broadphase::MODEL_WALKS.with(|walks| walks.get()), 12, "each WorldModel's world walked once, when its first take-in made its index");
        broadphase::MODEL_WALKS.with(|walks| walks.set(0));
        run(&lua, &engine, r##"for _, W in shared.Worlds do assert(W:IsRegion3Empty(shared.R)) end assert(not shared.World:IsRegion3Empty(shared.R))"##);
        assert_eq!(broadphase::MODEL_WALKS.with(|walks| walks.get()), 0, "and not again");
        broadphase::MODEL_WALKS.with(|walks| walks.set(0));
        run(
            &lua,
            &engine,
            r##"
            for I = 1, 50 do
                shared.Lone:SetAttribute("a", I)
                shared.Lone.Position += Vector3.new(0, 0, 0.1)
                Instance.new("Part").Parent = workspace
                assert(not shared.World:IsRegion3Empty(shared.R))
            end
            -- a write and moves inside it are followed
            shared.Inside.Position = Vector3.new(0, 900, 0)
            assert(shared.World:IsRegion3Empty(shared.R))
            shared.Inside.Position = Vector3.new(500, 0, 0)
            assert(#shared.World:FindPartsInRegion3(shared.R) == 1)
            shared.Inside.Parent = workspace
            assert(shared.World:IsRegion3Empty(shared.R))
            shared.Inside.Parent = shared.World
            shared.World:ArePartsTouchingOthers({})
            assert(#shared.World:FindPartsInRegion3(shared.R) == 1)
            for _ = 1, 3 do
                for _, W in shared.Worlds do
                    assert(W:IsRegion3Empty(shared.R))
                end
            end
            "##,
        );
        assert_eq!(broadphase::MODEL_WALKS.with(|walks| walks.get()), 0, "no WorldModel walked again");
        broadphase::WORKSPACE_WALKS.with(|walks| walks.set(0));
        broadphase::TAKEN.with(|taken| taken.set(0));
        run(&lua, &engine, r##"assert(workspace:IsRegion3Empty(shared.NestedRegion))"##);
        assert_eq!(broadphase::TAKEN.with(|taken| taken.get()), 0, "a WorldModel's parts in the Workspace are not given to its walk");
        broadphase::TAKEN.with(|taken| taken.set(0));
        run(&lua, &engine, r##"assert(workspace:ArePartsTouchingOthers({ shared.Stacked }))"##);
        let taken = broadphase::TAKEN.with(|taken| taken.get());
        assert!(taken <= 2, "the walk stops at the first part touching: given {taken} of 301");
        assert_eq!(broadphase::WORKSPACE_WALKS.with(|walks| walks.get()), 0, "no walk of the Workspace");
        broadphase::MODEL_WALKS.with(|walks| walks.set(0));
        run(&lua, &engine, r##"local W = Instance.new("WorldModel") W.Parent = workspace assert(not W:ArePartsTouchingOthers({}))"##);
        assert_eq!(broadphase::MODEL_WALKS.with(|walks| walks.get()), 1, "an empty list still takes its world in, which makes its index");
        broadphase::MODEL_WALKS.with(|walks| walks.set(0));
        let held = broadphase::worlds(&engine.state.borrow());
        run(
            &lua,
            &engine,
            r##"
            local W = Instance.new("WorldModel")
            local Listed
            for I = 1, 100 do
                local P = Instance.new("Part")
                P.CFrame = CFrame.new(500, I, 0)
                P.Parent = W
                Listed = P
            end
            assert(W:IsRegion3Empty(shared.R) and #W:FindPartsInRegion3(shared.R) == 0)
            local Ok = pcall(W.ArePartsTouchingOthers, W, { Listed })
            assert(not Ok)
            "##,
        );
        assert_eq!(broadphase::MODEL_WALKS.with(|walks| walks.get()), 0, "a WorldModel in nil makes no index");
        assert_eq!(broadphase::worlds(&engine.state.borrow()), held, "nor keeps one");
    }

    /// A WorldModel's index holds what a walk of its world finds after each kind of change in it (dom.rs WorldLog): parts written
    /// (CFrame, Size, Shape, CanCollide, CanQuery), moved in and out (alone, in a Model, from and to the Workspace and a WorldModel
    /// inside it), destroyed, cloned in, the inner WorldModel moved out and back, the WorldModel itself in nil and back, and a log
    /// that overflowed (5,000 writes), which walks it again once; a child put in place of another and a Folder turned into a
    /// WorldModel and back, a part added below it while it is a Folder (from Rust: dom.rs replace_child, set_class), a child put in
    /// place of another that keeps its parts, a new Part in place of one of the world's, and a WorldModel holding a child's
    /// parts in its place. Its parts wait out of the tree until a query of its world takes them in, also after it comes back
    /// into the DataModel. The Workspace's index keeps a WorldModel's parts in it apart and still matches a walk of the
    /// Workspace; with every part taken (broadphase.rs SCAN) and with a box that is not finite, the walks are given every part
    /// of their own world (beside a WorldModel in the Workspace with a part in the region, one of them with a NaN position).
    #[test]
    fn a_worldmodels_index_matches_a_walk_of_its_world() {
        let (lua, engine) = start();
        let steps: [&str; 7] = [
            r##"
            local Store = game:GetService("ReplicatedStorage")
            local function Part(Name, At, Parent)
                local P = Instance.new("Part")
                P.Name = Name
                P.Anchored = true
                P.CFrame = CFrame.new(At)
                P.Parent = Parent
                return P
            end
            local World = Instance.new("WorldModel")
            World.Name = "World"
            World.Parent = Store
            local F = Instance.new("Folder")
            F.Name = "F"
            F.Parent = World
            for I = 1, 20 do
                Part("F" .. I, Vector3.new(I * 3, 0, 0), F)
            end
            local M = Instance.new("Model")
            M.Name = "M"
            M.Parent = World
            for I = 1, 10 do
                Part("M" .. I, Vector3.new(I * 3, 10, 0), M)
            end
            local Inner = Instance.new("WorldModel")
            Inner.Name = "Inner"
            Inner.Parent = World
            for I = 1, 5 do
                Part("I" .. I, Vector3.new(I * 3, 20, 0), Inner)
            end
            Part("P0", Vector3.new(0, 30, 0), workspace)
            shared.World, shared.Inner, shared.M = World, Inner, M
            "##,
            r##"
            local F = shared.World.F
            F.F1.CFrame = CFrame.new(1, 2, 3) * CFrame.Angles(0.3, 0.2, 0.1)
            F.F2.Size = Vector3.new(4, 1, 9)
            F.F3.Shape = Enum.PartType.Ball
            F.F4.CanCollide = false
            F.F5.CanQuery = false
            F.F6.Position = Vector3.new(0, 0 / 0, 0)
            "##,
            r##"
            local World, F = shared.World, shared.World.F
            F.F7.Parent = workspace
            workspace.P0.Parent = F
            shared.M.Parent = workspace
            shared.M.Parent = World
            F.F8.Parent = shared.Inner
            shared.Inner.I1.Parent = F
            "##,
            r##"
            local Store = game:GetService("ReplicatedStorage")
            shared.Inner.Parent = Store
            shared.World.F.F9:Destroy()
            shared.M:Clone().Parent = shared.World
            shared.Inner.Parent = shared.World.F
            "##,
            r##"
            shared.World.Parent = nil
            shared.World.F.F10.CFrame += Vector3.new(0, 1, 0)
            shared.World.Parent = game:GetService("ReplicatedStorage")
            shared.Inner.Parent = workspace
            "##,
            r##"
            -- a part written 5,000 times is logged once; 5,000 parts made in it overflow its log
            local P = shared.World.F.F11
            for I = 1, 5000 do
                P.CFrame = CFrame.new(I, 0, 0)
            end
            local Many = Instance.new("Folder")
            Many.Parent = shared.World
            for I = 1, 5000 do
                local Q = Instance.new("Part")
                Q.CFrame = CFrame.new(I, 60, 0)
                Q.Parent = shared.World
                Q.Parent = Many
            end
            Many:Destroy()
            "##,
            r##"
            shared.Inner.Parent = shared.World
            "##,
        ];
        let (mut world, mut inner) = (None, None);
        for (at, step) in steps.iter().enumerate() {
            broadphase::MODEL_WALKS.with(|walks| walks.set(0));
            run(&lua, &engine, step);
            if world.is_none() {
                let state = engine.state.borrow();
                let store = state.dom.find_child(engine.world().game, b"ReplicatedStorage").expect("ReplicatedStorage");
                world = state.dom.find_child(store, b"World");
                inner = world.and_then(|world| state.dom.find_child(world, b"Inner"));
            }
            let (world, inner) = (world.expect("World"), inner.expect("Inner"));
            for model in [world, inner] {
                query::model_index_matches_a_walk(&engine, model).unwrap_or_else(|error| panic!("step {at}: {error}"));
            }
            let walks = broadphase::MODEL_WALKS.with(|walks| walks.get());
            // (made at the first step; walked again after the overflowed log)
            let expected = match at {
                0 => 2,
                // (World left the DataModel and came back, Inner inside it: their indexes went with them, made again)
                4 => 2,
                5 => 1,
                _ => 0,
            };
            assert_eq!(walks, expected, "step {at}: walks of the WorldModels' worlds");
            query::index_matches_a_walk(&engine).unwrap_or_else(|error| panic!("step {at}, the Workspace: {error}"));
        }
        let (world, inner) = (world.expect("World"), inner.expect("Inner"));
        // a child put in place of another, holding its children (dom.rs replace_child, as a project mount does), and a Folder
        // turned into a WorldModel and back (dom.rs set_class), its parts leaving the world and coming back
        run(&lua, &engine, r##"local G = Instance.new("Folder") G.Name = "G" G.Parent = shared.World for I = 1, 4 do local P = Instance.new("Part") P.CFrame = CFrame.new(I, 40, 0) P.Parent = G end"##);
        query::model_index_matches_a_walk(&engine, world).expect("before");
        let (folder, world_model) = (engine.reflection.class_id("Folder").expect("Folder"), engine.reflection.class_id("WorldModel").expect("WorldModel"));
        let (f, g) = {
            let state = engine.state.borrow();
            (state.dom.find_child(world, b"F").expect("F"), state.dom.find_child(world, b"G").expect("G"))
        };
        {
            let mut state = engine.state.borrow_mut();
            let new = state.dom.create(&engine.reflection, folder);
            let children = std::mem::take(&mut state.dom.at_mut(f).children);
            for &child in &children {
                state.dom.at_mut(child).parent = Some(new);
            }
            state.dom.at_mut(new).children = children;
            state.dom.replace_child(world, f, new);
        }
        query::model_index_matches_a_walk(&engine, world).expect("a child replaced");
        query::model_index_matches_a_walk(&engine, inner).expect("a child replaced, the inner WorldModel");
        engine.state.borrow_mut().dom.set_class(&engine.reflection, g, world_model);
        query::model_index_matches_a_walk(&engine, world).expect("a Folder made a WorldModel");
        query::model_index_matches_a_walk(&engine, g).expect("the Folder made a WorldModel, its own world");
        engine.state.borrow_mut().dom.set_class(&engine.reflection, g, folder);
        query::model_index_matches_a_walk(&engine, world).expect("the WorldModel made a Folder again");
        // while it is a Folder a part goes below it; made a WorldModel again, its index starts over (its log was dropped)
        run(&lua, &engine, r##"local P = Instance.new("Part") P.CFrame = CFrame.new(9, 40, 0) P.Parent = shared.World.G"##);
        engine.state.borrow_mut().dom.set_class(&engine.reflection, g, world_model);
        query::model_index_matches_a_walk(&engine, g).expect("a Folder made a WorldModel again, a part added below it");
        engine.state.borrow_mut().dom.set_class(&engine.reflection, g, folder);
        query::model_index_matches_a_walk(&engine, world).expect("and a Folder again");
        // a child put in place of another that keeps its parts (they leave the world), and a new Part put in place of a part of
        // the world (one of M's: the index takes the new one in, from the log, and drops the old)
        run(&lua, &engine, r##"local H = Instance.new("Folder") H.Name = "H" H.Parent = shared.World for I = 1, 3 do local P = Instance.new("Part") P.Name = "Q" .. I P.CFrame = CFrame.new(I, 50, 0) P.Parent = H end"##);
        query::model_index_matches_a_walk(&engine, world).expect("H made");
        let (h, m, m2) = {
            let state = engine.state.borrow();
            let m = state.dom.find_child(world, b"M").expect("M");
            (state.dom.find_child(world, b"H").expect("H"), m, state.dom.find_child(m, b"M2").expect("M2"))
        };
        {
            let mut state = engine.state.borrow_mut();
            let new = state.dom.create(&engine.reflection, folder);
            state.dom.replace_child(world, h, new);
        }
        query::model_index_matches_a_walk(&engine, world).expect("a child that keeps its parts replaced");
        {
            let part = engine.reflection.class_id("Part").expect("Part");
            let mut state = engine.state.borrow_mut();
            let other = state.dom.create(&engine.reflection, part);
            state.dom.replace_child(m, m2, other);
        }
        query::model_index_matches_a_walk(&engine, world).expect("a part of the world replaced by a new Part");
        // a WorldModel put in place of a child, holding the children handed over to it: their parts leave this world for its
        // own
        run(&lua, &engine, r##"local J = Instance.new("Folder") J.Name = "J" J.Parent = shared.World for I = 1, 3 do local P = Instance.new("Part") P.CFrame = CFrame.new(I, 70, 0) P.Parent = J end"##);
        query::model_index_matches_a_walk(&engine, world).expect("J made");
        {
            let mut state = engine.state.borrow_mut();
            let j = state.dom.find_child(world, b"J").expect("J");
            let new = state.dom.create(&engine.reflection, world_model);
            let children = std::mem::take(&mut state.dom.at_mut(j).children);
            for &child in &children {
                state.dom.at_mut(child).parent = Some(new);
            }
            state.dom.at_mut(new).children = children;
            state.dom.replace_child(world, j, new);
        }
        query::model_index_matches_a_walk(&engine, world).expect("a WorldModel put in place of a child, holding its parts");
        query::index_matches_a_walk(&engine).expect("the Workspace");
        // the notes' walk of every cell (SCAN), beside a WorldModel in the Workspace holding a part in the region, which its walks
        // never meet
        run(
            &lua,
            &engine,
            r##"
            local function Part(At)
                local P = Instance.new("Part")
                P.Anchored = true
                P.CFrame = CFrame.new(At)
                P.Parent = workspace
                return P
            end
            for I = 1, 5 do
                Part(Vector3.new(2000 + I * 3, 0, 0))
            end
            Part(Vector3.new(2000, 50, 0))
            local Beside = Instance.new("WorldModel")
            Beside.Parent = workspace
            Part(Vector3.new(2009, 0, 0)).Parent = Beside
            Part(Vector3.new(0 / 0, 0, 0)).Parent = Beside
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            shared.Scan = Region3.new(Vector3.new(1995, -5, -5), Vector3.new(2020, 5, 5))
            function shared.Seen()
                local Found = workspace:FindPartsInRegion3(shared.Scan, nil, 0)
                return `{#Found} {workspace:IsRegion3Empty(shared.Scan)} {workspace:ArePartsTouchingOthers({ Found[1] })} {workspace:ArePartsTouchingOthers({ workspace:FindPartsInRegion3(Region3.new(Vector3.new(1995, 45, -5), Vector3.new(2005, 55, 5)))[1] })}`
            end
            shared.Before = shared.Seen()
            assert(shared.Before == "5 false true false", shared.Before)
            "##,
        );
        broadphase::SCAN.with(|scan| scan.set(true));
        run(&lua, &engine, r##"assert(shared.Seen() == shared.Before, shared.Seen())"##);
        broadphase::SCAN.with(|scan| scan.set(false));
    }

    /// A WorldModel's index takes each changed instance once (dom.rs WorldLog): a chain of 2,000 parts (each in the one before)
    /// moved within its world, parented out and built inside costs the next query's sync about the chain's length each time, where
    /// each part's walk went down the rest of it; and the index holds what a walk of its world finds after each.
    #[test]
    fn a_chain_of_parts_that_moves_costs_a_worldmodels_index_its_length() {
        let (lua, engine) = start();
        let visits = || broadphase::MODEL_SYNC_VISITS.with(|visits| visits.replace(0));
        run(
            &lua,
            &engine,
            r##"
            local World = Instance.new("WorldModel")
            World.Name = "World"
            World.Parent = game:GetService("ReplicatedStorage")
            local Holder, Other = Instance.new("Folder"), Instance.new("Folder")
            Holder.Parent, Other.Parent = World, World
            function shared.Chain(Count)
                local First, At = nil, Holder
                for I = 1, Count do
                    local Piece = Instance.new("Part")
                    Piece.Anchored = true
                    Piece.Size = Vector3.new(1, 1, 1)
                    Piece.CFrame = CFrame.new(I * 2, 0, 0)
                    Piece.Parent = At
                    First = First or Piece
                    At = Piece
                end
                return First
            end
            function shared.Count()
                World:ArePartsTouchingOthers({})
                return #World:FindPartsInRegion3(Region3.new(Vector3.new(0, -1, -1), Vector3.new(4002, 1, 1)), nil, 0)
            end
            shared.World, shared.First, shared.Other = World, shared.Chain(2000), Other
            assert(shared.Count() == 2000)
            "##,
        );
        let world = {
            let state = engine.state.borrow();
            let store = state.dom.find_child(engine.world().game, b"ReplicatedStorage").expect("ReplicatedStorage");
            state.dom.find_child(store, b"World").expect("World")
        };
        visits();
        run(&lua, &engine, r##"shared.First.Parent = shared.Other assert(shared.Count() == 2000, "moved within the world")"##);
        let moved = visits();
        query::model_index_matches_a_walk(&engine, world).unwrap();
        run(&lua, &engine, r##"shared.First.Parent = nil assert(shared.Count() == 0, "parented out")"##);
        let out = visits();
        query::model_index_matches_a_walk(&engine, world).unwrap();
        run(&lua, &engine, r##"shared.Chain(2000) assert(shared.Count() == 2000, "built inside")"##);
        let built = visits();
        query::model_index_matches_a_walk(&engine, world).unwrap();
        for (what, visits) in [("moved within the world", moved), ("parented out", out), ("built inside", built)] {
            assert!(visits <= 2 * 2000 + 10, "a chain of 2,000 parts {what}: the next sync looked at {visits} instances");
        }
    }

    /// An instance that left a WorldModel's world is taken out of its index without a walk of what is below it now (dom.rs
    /// logged the parts that left with it one by one), and a sync that walks pays FIND_NODE for each instance it looks at: a
    /// Folder moved through 200 WorldModels, each queried once before, sits in all their logs, and given 2,000 Folders outside
    /// them all it costs each one's next sync a look, where each sync walked the 2,000 (400,400 instances in all); moved into
    /// one of them, the 2,002 it brings cost that one's next query 2,002 FIND_NODE.
    #[test]
    fn a_folder_that_passed_through_many_worldmodels_costs_them_no_walk_of_what_it_holds() {
        let (lua, engine) = start();
        crate::lua::modules::interpret_all(&lua, &engine);
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let visits = || broadphase::MODEL_SYNC_VISITS.with(|visits| visits.replace(0));
        run(
            &lua,
            &engine,
            r##"
            local Store = game:GetService("ReplicatedStorage")
            local Region = Region3.new(Vector3.new(-1, -1, -1), Vector3.new(1, 1, 1))
            local Worlds = {}
            for I = 1, 200 do
                local World = Instance.new("WorldModel")
                World.Parent = Store
                assert(#World:FindPartsInRegion3(Region, nil, 0) == 0)
                Worlds[I] = World
            end
            local Passing = Instance.new("Folder")
            for _, World in Worlds do
                Passing.Parent = World
            end
            Passing.Parent = Store
            local Held = Instance.new("Folder")
            for _ = 1, 2000 do
                Instance.new("Folder").Parent = Held
            end
            Held.Parent = Passing
            shared.Worlds, shared.Passing, shared.Region = Worlds, Passing, Region
            "##,
        );
        visits();
        run(&lua, &engine, r##"for _, World in shared.Worlds do assert(#World:FindPartsInRegion3(shared.Region, nil, 0) == 0) end"##);
        let passed = visits();
        assert!(passed <= 400, "200 WorldModels' syncs after a Folder holding 2,000 passed through them looked at {passed} instances");
        run(
            &lua,
            &engine,
            r##"
            local World = shared.Worlds[1]
            local function Once()
                local Start = os.clock()
                assert(#World:FindPartsInRegion3(shared.Region, nil, 0) == 0)
                return (os.clock() - Start) * 1e6
            end
            shared.Passing.Parent = World
            local Moved = Once()
            shared.Paid = Moved - Once()
            "##,
        );
        assert_eq!(visits(), 2002, "the Folder, what it holds and its 2,000 moved into a WorldModel");
        let expected = 2002.0 * crate::cost::FIND_NODE * 1e6;
        run(&lua, &engine, &format!(r##"assert(math.abs(shared.Paid - {expected}) < 0.3, `the walk of 2,002 instances cost {{shared.Paid}} us`)"##));
        for world in 0..2 {
            let model = {
                let state = engine.state.borrow();
                let store = state.dom.find_child(engine.world().game, b"ReplicatedStorage").expect("ReplicatedStorage");
                state.dom.children_of(store).iter().copied().filter(|&child| state.dom.at(child).class == engine.reflection.class_id("WorldModel").expect("WorldModel")).nth(world).expect("a WorldModel")
            };
            query::model_index_matches_a_walk(&engine, model).unwrap();
        }
    }

    /// A WorldModel's index looks at its world's wait again only when that wait changed other than as its log shows: a query of
    /// that world took its parts in, the WorldModel came back into the DataModel, or the physics step ran. Another world's intake
    /// leaves it alone, so 1,000 parts waiting in one WorldModel cost nothing to a query of it while another WorldModel's queries
    /// take their own parts in.
    #[test]
    fn another_worlds_intake_leaves_a_worldmodels_wait_alone() {
        let (lua, engine) = start();
        let looks = || broadphase::MODEL_SYNC_VISITS.with(|looks| looks.replace(0));
        run(
            &lua,
            &engine,
            r##"
            local Store = game:GetService("ReplicatedStorage")
            local A, B = Instance.new("WorldModel"), Instance.new("WorldModel")
            A.Name = "A"
            A.Parent, B.Parent = Store, Store
            local Region = Region3.new(Vector3.new(-1, -1, -1), Vector3.new(1, 1, 1))
            assert(#A:FindPartsInRegion3(Region, nil, 0) == 0)
            for _ = 1, 1000 do
                local P = Instance.new("Part")
                P.Anchored = true
                P.Size = Vector3.new(1, 1, 1)
                P.Parent = A
            end
            assert(#A:FindPartsInRegion3(Region, nil, 0) == 0, "waiting")
            function shared.Round()
                local P = Instance.new("Part")
                P.Anchored = true
                P.Parent = B
                B:GetPartBoundsInBox(CFrame.new(), Vector3.one)
                return #A:FindPartsInRegion3(Region, nil, 0)
            end
            shared.A, shared.Store, shared.Region = A, Store, Region
            "##,
        );
        looks();
        run(&lua, &engine, r##"for _ = 1, 20 do assert(shared.Round() == 0, "still waiting") end"##);
        assert!(looks() <= 20, "B's intakes look at B's one new part a round, never at A's wait");
        run(&lua, &engine, r##"shared.A:GetPartBoundsInBox(CFrame.new(), Vector3.one) assert(#shared.A:FindPartsInRegion3(shared.Region, nil, 0) == 1000)"##);
        run(&lua, &engine, r##"shared.A.Parent = nil shared.A.Parent = shared.Store assert(#shared.A:FindPartsInRegion3(shared.Region, nil, 0) == 0, "back from nil")"##);
        looks();
        run(&lua, &engine, r##"for _ = 1, 20 do assert(shared.Round() == 0, "waiting again") end"##);
        assert!(looks() <= 20, "B's intakes leave A's wait alone again");
        let world = {
            let state = engine.state.borrow();
            let store = state.dom.find_child(engine.world().game, b"ReplicatedStorage").expect("ReplicatedStorage");
            state.dom.find_child(store, b"A").expect("A")
        };
        query::model_index_matches_a_walk(&engine, world).unwrap();
    }

    /// A region whose bounds are finite but whose centre overflows float32 (2e38 to 2e38 along x) finds nothing and walks no
    /// part, as a client's finds nothing (its Region3's centre is infinite: measured); one whose size alone overflows (-3e38 to
    /// 3e38) still finds what it holds, as a client's does; and ArePartsTouchingOthers for a part at 2e38 walks the parts near
    /// it alone (World::visit works a box out from the halves of its bounds).
    #[test]
    fn a_region_or_a_part_whose_centre_overflows_walks_no_world() {
        let (lua, engine) = start();
        run(
            &lua,
            &engine,
            r##"
            for I = 1, 1000 do
                local P = Instance.new("Part")
                P.Anchored = true
                P.Size = Vector3.one
                P.CFrame = CFrame.new(I % 10 * 2, 3 + I // 10 % 10 * 2, I // 100 * 2)
                P.Parent = workspace
            end
            local O = Instance.new("Part")
            O.Anchored = true
            O.Size = Vector3.one
            O.CFrame = CFrame.new(0.5, 0.5, 0.5)
            O.Parent = workspace
            local Far = Instance.new("Part")
            Far.Anchored = true
            Far.Size = Vector3.new(4, 1, 2)
            Far.CFrame = CFrame.new(2e38, 100, 0)
            Far.Parent = workspace
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            shared.Far = Far
            "##,
        );
        let taken = || broadphase::TAKEN.with(|taken| taken.replace(0));
        taken();
        run(&lua, &engine, r##"local R = Region3.new(Vector3.new(2e38, 0, 0), Vector3.new(2e38, 1, 1)) assert(workspace:IsRegion3Empty(R) and #workspace:FindPartsInRegion3(R, nil, 0) == 0)"##);
        assert_eq!(taken(), 0, "a region whose centre overflows walks no part");
        run(&lua, &engine, r##"assert(not workspace:ArePartsTouchingOthers({ shared.Far }))"##);
        let near = taken();
        assert!(near <= 1, "a part at 2e38 walks the parts near it alone: given {near}");
        run(&lua, &engine, r##"local Found = workspace:FindPartsInRegion3(Region3.new(Vector3.new(-3e38, 0, 0), Vector3.new(3e38, 1, 1)), nil, 0) assert(#Found == 1, #Found)"##);
    }

    /// A WorldModel freed takes its index, its wait and its change log with it
    /// (world/camera_moves.rs's instance_freed hook, dom.rs Dom::free): one made, queried, a part taken in by a query of it,
    /// destroyed and collected leaves none of them (its Destroy drops the wait alone).
    #[test]
    fn a_freed_worldmodel_takes_its_index_and_log_with_it() {
        let (lua, engine) = start();
        run(
            &lua,
            &engine,
            r##"
            local World = Instance.new("WorldModel")
            World.Name = "Gone"
            World.Parent = game:GetService("ReplicatedStorage")
            local P = Instance.new("Part")
            P.Parent = World
            World:IsRegion3Empty(Region3.new(Vector3.new(-1, -1, -1), Vector3.new(1, 1, 1)))
            Instance.new("Part").Parent = World
            World:GetPartBoundsInBox(CFrame.new(), Vector3.one)
            Instance.new("Part").Parent = World
            "##,
        );
        let world = {
            let state = engine.state.borrow();
            let store = state.dom.find_child(engine.world().game, b"ReplicatedStorage").expect("ReplicatedStorage");
            state.dom.find_child(store, b"Gone").expect("Gone")
        };
        {
            let mut state = engine.state.borrow_mut();
            assert!(state.spatial.world(world).is_some(), "an index made");
            assert!(state.cameras.unindexed_models.contains_key(&world), "a part waiting");
            assert!(state.dom.world_log(world).is_some(), "a log kept");
        }
        run(&lua, &engine, r##"game:GetService("ReplicatedStorage").Gone:Destroy()"##);
        lua.gc_collect().unwrap();
        lua.gc_collect().unwrap();
        crate::lua::raw::collect(&lua, &engine).unwrap();
        let mut state = engine.state.borrow_mut();
        assert!(state.dom.get(world).is_none(), "the WorldModel freed");
        assert!(state.spatial.world(world).is_none(), "its index dropped");
        assert!(!state.cameras.unindexed_models.contains_key(&world), "its wait dropped");
        assert!(state.dom.world_log(world).is_none(), "its log dropped");
    }

    /// A WorldModel's change log keeps the writes of its parts alone (dom.rs WorldLog, the classes Dom::keep_logged_classes
    /// names), as the Workspace's keeps those of the parts it indexes: its index reads nothing else, so 100 Values and a Folder
    /// written in its world leave the log empty, and a part written is in it.
    #[test]
    fn a_worldmodels_log_keeps_its_parts_writes_alone() {
        let (lua, engine) = start();
        run(
            &lua,
            &engine,
            r##"
            local World = Instance.new("WorldModel")
            World.Name = "World"
            World.Parent = game:GetService("ReplicatedStorage")
            local P = Instance.new("Part")
            P.Parent = World
            local Values = {}
            for I = 1, 100 do
                local V = Instance.new("StringValue")
                V.Parent = World
                Values[I] = V
            end
            local F = Instance.new("Folder")
            F.Parent = World
            World:IsRegion3Empty(Region3.new(Vector3.new(-1, -1, -1), Vector3.new(1, 1, 1)))
            shared.Values, shared.P, shared.F = Values, P, F
            "##,
        );
        let world = {
            let state = engine.state.borrow();
            let store = state.dom.find_child(engine.world().game, b"ReplicatedStorage").expect("ReplicatedStorage");
            state.dom.find_child(store, b"World").expect("World")
        };
        let logged = || {
            let mut state = engine.state.borrow_mut();
            let log = state.dom.world_log(world).expect("followed");
            (log.moved.len(), log.written.len())
        };
        assert_eq!(logged(), (0, 0), "taken by the query");
        run(&lua, &engine, r##"for I, V in shared.Values do V.Value = tostring(I) end shared.F.Name = "G""##);
        assert_eq!(logged(), (0, 0), "Values and a Folder written");
        run(&lua, &engine, r##"shared.P.CFrame = CFrame.new(5, 0, 0)"##);
        assert_eq!(logged(), (0, 1), "a part written");
        query::model_index_matches_a_walk(&engine, world).unwrap();
    }

    /// The PlayerMouse's ray and GetLargestCutoffDistance never meet a WorldModel's parts in the Workspace, settled or just made,
    /// as a client's do not (measured, Studio 0.748: a wall in a WorldModel in the Workspace between a camera and its focus leaves
    /// its cutoff 0 and the mouse's Target nil, where the same wall in the Workspace gives 18 and the wall).
    #[test]
    fn the_mouse_and_cutoff_rays_never_meet_a_worldmodels_parts() {
        let (lua, engine) = start_play();
        run(
            &lua,
            &engine,
            r##"
            local O = Vector3.new(200, 100, 200)
            local Cam = Instance.new("Camera")
            Cam.Parent = workspace
            Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 5, 30), O + Vector3.new(0, 5, 0))
            Cam.Focus = CFrame.new(O + Vector3.new(0, 5, 0))
            local WM = Instance.new("WorldModel")
            WM.Parent = workspace
            local function Wall(Parent, At)
                local P = Instance.new("Part")
                P.Anchored = true
                P.Size = Vector3.new(60, 60, 1)
                P.CFrame = At
                P.Parent = Parent
                return P
            end
            local function Cut()
                return string.format("%.3f", Cam:GetLargestCutoffDistance({}))
            end
            -- the control: a Workspace wall
            local W = Wall(workspace, CFrame.new(O + Vector3.new(0, 5, 12)))
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            assert(Cut() == "18.000", Cut())
            W:Destroy()
            -- a WorldModel's wall, just made and then taken in by its own query and the Workspace's
            W = Wall(WM, CFrame.new(O + Vector3.new(0, 5, 12)))
            assert(Cut() == "0.000", Cut())
            WM:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            assert(Cut() == "0.000", Cut())
            W:Destroy()
            -- the mouse: a wall along its ray, in the Workspace and in the WorldModel
            local M = game:GetService("Players").LocalPlayer:GetMouse()
            local Ray = M.UnitRay
            local At = CFrame.lookAt(Ray.Origin + Ray.Direction * 40, Ray.Origin + Ray.Direction * 41)
            W = Wall(workspace, At)
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            assert(M.Target == W, tostring(M.Target))
            W:Destroy()
            W = Wall(WM, At)
            assert(M.Target == nil, tostring(M.Target))
            WM:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            assert(M.Target == nil, tostring(M.Target))
            "##,
        );
    }

    /// ArePartsTouchingOthers as a playing client answers it (Studio 0.748): whether a listed part with CanCollide on overlaps a part
    /// not listed with CanCollide on by more than overlapIgnored (0.0002 when nil), boxes by their least overlap along a separating
    /// axis and balls by their spheres; a gap counts for a negative overlapIgnored only up to 0.3125; parts that just entered count;
    /// the argument errors from the last argument to the first, then each entry in turn.
    #[test]
    fn are_parts_touching_others_follows_a_client() {
        let (lua, engine) = start();
        run(
            &lua,
            &engine,
            r##"
            local function Fails(Message, F, ...)
                local Ok, E = pcall(F, ...)
                if Ok or tostring(E) ~= Message then
                    error(`{Message}: {E}`, 2)
                end
            end
            local Folder = Instance.new("Folder")
            Folder.Parent = workspace
            local Next = 0
            local function Pair(Offset, AProps, BProps, ASize, BSize, Turn)
                Next += 1
                local Base = Vector3.new(800 + 50 * Next, 800, 800)
                local function Make(Name, At, Props, Size)
                    local P = Instance.new("Part")
                    P.Name = Name
                    P.Anchored = true
                    P.Size = Size or Vector3.new(2, 2, 2)
                    P.CFrame = CFrame.new(At)
                    for K, V in Props or {} do
                        (P :: any)[K] = V
                    end
                    P.Parent = Folder
                    return P
                end
                local A = Make("A", Base, AProps, ASize)
                local B = Make("B", Base + Offset, BProps, BSize)
                if Turn then
                    B.CFrame = CFrame.new(Base + Offset) * Turn
                end
                return A, B
            end
            local T = function(List, ...) return workspace:ArePartsTouchingOthers(List, ...) end
            -- the reference's sample: two 2-stud cubes overlapping by 1 (and both just made: they count at once)
            local A, B = Pair(Vector3.new(0, 1, 0))
            assert(T({ A }) and T({ A }, 0) and T({ A }, 0.999) and not T({ A }, 1) and T({ A }, "0.5") and not T({ A }, true))
            assert(#workspace:FindPartsInRegion3(Region3.new(A.Position - Vector3.one, A.Position + Vector3.one)) == 2, "it took them into the index")
            assert(not T({ A, B }) and T({ A, A }) and not T({}) and not T({ a = A } :: any))
            assert(T(table.create(500, A)) and not T(table.create(500, B), 1))
            -- faces touching: not by default, yes once a gap may count; a gap up to 0.3125
            A = Pair(Vector3.new(0, 2, 0))
            assert(not T({ A }) and not T({ A }, 0) and T({ A }, -0.0001))
            A = Pair(Vector3.new(0, 2.3, 0))
            assert(T({ A }, -1) and not T({ A }, -0.29))
            A = Pair(Vector3.new(2.33, 0, 0))
            assert(not T({ A }, -1))
            -- the least overlap along an axis: 1 in y, 0.3 in x
            A = Pair(Vector3.new(1.7, 1, 0))
            assert(T({ A }, 0.29) and not T({ A }, 0.31))
            -- a cube turned 45 degrees by a's edge: their world boxes overlap, they do not
            A = Pair(Vector3.new(2.214, 0, 2.214), nil, nil, nil, nil, CFrame.Angles(0, math.rad(45), 0))
            assert(not T({ A }, 0))
            -- its corner 0.3 into a's face
            A = Pair(Vector3.new(2.114, 0, 0), nil, nil, nil, nil, CFrame.Angles(0, math.rad(45), 0))
            assert(T({ A }, 0.29) and not T({ A }, 0.31))
            -- balls by their spheres
            A = Pair(Vector3.new(1.8, 1.8, 0), { Shape = Enum.PartType.Ball }, { Shape = Enum.PartType.Ball })
            assert(not T({ A }, 0))
            A = Pair(Vector3.new(1.9, 0, 0), { Shape = Enum.PartType.Ball }, { Shape = Enum.PartType.Ball })
            assert(T({ A }, 0.09) and not T({ A }, 0.11))
            -- a ball and a block by the sphere against the box, whichever is listed and whichever was made first: sunk 0.3 into
            -- its face; by its corner, where their boxes overlap and the sphere is 0.131 from it; its centre inside, 1.5 deep by
            -- the nearest face; a turned block's edge 0.3 into the sphere
            local function Both(Ball, Block, Low, High)
                return T({ Ball }, Low) and not T({ Ball }, High) and T({ Block }, Low) and not T({ Block }, High)
            end
            local Ball = { Shape = Enum.PartType.Ball }
            local Sphere, Block = Pair(Vector3.new(1.7, 0, 0), Ball)
            assert(Both(Sphere, Block, 0.29, 0.31))
            Block, Sphere = Pair(Vector3.new(1.7, 0, 0), nil, Ball)
            assert(Both(Sphere, Block, 0.29, 0.31))
            Sphere, Block = Pair(Vector3.new(1.8, 1.8, 0), Ball)
            assert(Both(Sphere, Block, -0.14, -0.12) and not T({ Sphere }, 0) and not T({ Block }, 0))
            Sphere, Block = Pair(Vector3.new(0.5, 0, 0), Ball)
            assert(Both(Sphere, Block, 1.49, 1.51))
            Sphere, Block = Pair(Vector3.new(2.114, 0, 0), Ball, nil, nil, nil, CFrame.Angles(0, math.rad(45), 0))
            assert(Both(Sphere, Block, 0.29, 0.31))
            -- CanCollide off on either side: never; CanQuery, CanTouch and Transparency do not matter
            assert(not T({ (Pair(Vector3.new(0, 1.5, 0), nil, { CanCollide = false })) }))
            assert(not T({ (Pair(Vector3.new(0, 1.5, 0), { CanCollide = false })) }))
            assert(T({ (Pair(Vector3.new(0, 1.5, 0), nil, { CanQuery = false, CanTouch = false, Transparency = 1 })) }))
            -- Terrain may be listed, and never counts
            assert(not T({ workspace.Terrain }))
            -- errors: the float, the list, then each entry
            local Inside = (Pair(Vector3.new(0, 10, 0)))
            local Outside = Instance.new("Part")
            local World = Instance.new("WorldModel")
            World.Parent = workspace
            local InWorld = Instance.new("Part")
            InWorld.Parent = World
            Fails("Argument 1 missing or nil", workspace.ArePartsTouchingOthers, workspace)
            Fails("Expected table argument, got nil", workspace.ArePartsTouchingOthers, workspace, nil)
            Fails("Expected table argument, got userdata", workspace.ArePartsTouchingOthers, workspace, Inside)
            Fails("Unable to cast string to float", workspace.ArePartsTouchingOthers, workspace, 5, "x")
            Fails("Unable to cast Array to float", workspace.ArePartsTouchingOthers, workspace, { Inside }, {})
            Fails("Unable to cast Instance to float", workspace.ArePartsTouchingOthers, workspace, { Inside }, workspace)
            Fails("Unable to cast value to Object", workspace.ArePartsTouchingOthers, workspace, { Folder, 5 })
            Fails("All objects passed to ArePartsTouchingOthers must be BaseParts, got a Folder.", workspace.ArePartsTouchingOthers, workspace, { Folder, Outside })
            Fails("All parts passed to ArePartsTouchingOthers must be in the WorldRoot", workspace.ArePartsTouchingOthers, workspace, { Outside, Folder })
            Fails("All objects passed to ArePartsTouchingOthers must be BaseParts, got a Nil.", workspace.ArePartsTouchingOthers, workspace, { nil, Inside })
            Fails("All parts passed to ArePartsTouchingOthers must be in the WorldRoot", workspace.ArePartsTouchingOthers, workspace, { InWorld })
            Fails("All parts passed to ArePartsTouchingOthers must be in the WorldRoot", World.ArePartsTouchingOthers, World, { Inside })
            assert(not World:ArePartsTouchingOthers({ InWorld }))
            Folder:Destroy()
            World:Destroy()
            "##,
        );
    }

    /// The Region3 queries and ArePartsTouchingOthers cost the work clock what a playing client spent on them (cost.rs REGION_FOUND,
    /// TOUCHING_LIST; Studio 0.748, os.clock less an empty loop): FindPartsInRegion3 over 50 parts 7.2 us, over 200 with maxParts 5
    /// 5.2, with the 50 ignored 3.4; an ignore list of another folder over 50 parts under 256 Folders 21.5, with maxParts 5 3.7 (its
    /// ancestor checks stop once the result is full); over 100 parts with CanQuery and CanCollide off 7.37, over 1,000 waiting for
    /// the spatial index 0.42 (nothing for them); beside 196 parts 0.1 past its face 16.8 to 18.6, 0.5 past it 2.4; IsRegion3Empty
    /// over 200 parts 4.57; ArePartsTouchingOthers of 30 parts with nothing near 4.67, of 30 whose first touches 2.53, of 30 in one
    /// spot 65.2 (each meets the others), of one among 100 with CanCollide off 7.71, of one beside 100 17.4, of a cube beside 196
    /// parts 0.5 from it 2.65 to 3.0, taking in 100 parts 120 more than alone, and nothing more for 100 parts a WorldModel's world
    /// is waiting for; and the first query of a WorldModel of 1,000 parts pays for its index (ree's own cost: cost.rs
    /// MODEL_INDEX_PART), but not for parts a query of it took in and paid for. An ignore list of 100 over 10 parts 100 deep 58 to
    /// 79 (each part walked to the top for each entry); a region from -inf to inf 0.27, finding nothing; a raise past an empty pcall
    /// 3.7 to 4.5 (FindPartsInRegion3's argument errors) and 7.0 to 8.4 (ArePartsTouchingOthers' overlap), a bad entry after
    /// 1,000 233 to 241 and a Folder after 1,000 in ArePartsTouchingOthers' list 30 to 37.
    #[test]
    fn region3_queries_cost_what_a_client_spent() {
        let (lua, engine) = start();
        crate::lua::modules::interpret_all(&lua, &engine);
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(
            &lua,
            &engine,
            r##"
            local function Per(Body)
                local N = 5
                local Start = os.clock()
                Body(N)
                local Middle = os.clock()
                for _ = 1, N do end
                return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
            end
            local function Within(Label, Value, Low, High)
                assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
            end
            local function Line(At, K)
                local F = Instance.new("Folder")
                F.Parent = workspace
                for I = 1, K do
                    local P = Instance.new("Part")
                    P.Anchored = true
                    P.Size = Vector3.new(0.5, 0.5, 0.5)
                    P.CFrame = CFrame.new(At + Vector3.new(I, 0, 0))
                    P.Parent = F
                end
                return F, Region3.new(At - Vector3.new(0, 2, 2), At + Vector3.new(K + 2, 2, 2))
            end
            local Fifty, R50 = Line(Vector3.new(3000, 900, 900), 50)
            local _, R200 = Line(Vector3.new(4000, 900, 900), 200)
            local Listed, Apart = {}, {}
            for I = 1, 30 do
                local Base = Vector3.new(-3000 + 10 * I, 900, -3000)
                for _, Spec in { { Listed, Base }, { false, Base + Vector3.new(0, 1.5, 0) }, { Apart, Base + Vector3.new(0, 20, 0) } } do
                    local P = Instance.new("Part")
                    P.Anchored = true
                    P.Size = Vector3.new(2, 2, 2)
                    P.CFrame = CFrame.new(Spec[2])
                    P.Parent = workspace
                    if Spec[1] then
                        table.insert(Spec[1], P)
                    end
                end
            end
            -- 50 parts under a chain of 256 Folders, checked against an ignore list of another folder: each of their ancestors
            local Deepest = workspace
            for _ = 1, 256 do
                local F = Instance.new("Folder")
                F.Parent = Deepest
                Deepest = F
            end
            local Deep, RDeep = Line(Vector3.new(5000, 900, 900), 50)
            Deep.Parent = Deepest
            local Unrelated = Instance.new("Folder")
            Unrelated.Parent = workspace
            -- the parts ArePartsTouchingOthers meets near a listed one: 30 in one spot, all listed; one among 100 with CanCollide
            -- off; one beside 100 others 0.1 away
            local function Make(At, Size, Props)
                local P = Instance.new("Part")
                P.Anchored = true
                P.Size = Size
                P.CFrame = CFrame.new(At)
                for K, V in Props or {} do
                    (P :: any)[K] = V
                end
                P.Parent = workspace
                return P
            end
            local Stacked = {}
            for _ = 1, 30 do
                table.insert(Stacked, Make(Vector3.new(-5000, 900, 0), Vector3.new(2, 2, 2)))
            end
            local Among, Beside = { Make(Vector3.new(-5000, 1000, 0), Vector3.new(2, 2, 2)) }, { Make(Vector3.new(-5000, 1100, 0), Vector3.new(2, 2, 2)) }
            for _ = 1, 100 do
                Make(Vector3.new(-5000, 1000, 0), Vector3.new(2, 2, 2), { CanCollide = false })
                Make(Vector3.new(-4997.9, 1100, 0), Vector3.new(2, 2, 2))
            end
            -- a region holding 100 parts with CanQuery and CanCollide off, and one holding 1,000 waiting for the spatial index
            local function Grid(At, K, Props)
                for I = 0, K - 1 do
                    Make(At + Vector3.new(I % 10, I // 10 % 10, I // 100) * 0.6, Vector3.new(0.5, 0.5, 0.5), Props)
                end
                return Region3.new(At - Vector3.one, At + Vector3.new(7, 7, 7))
            end
            local RHidden = Grid(Vector3.new(6000, 900, 900), 100, { CanQuery = false, CanCollide = false })
            -- 196 half-stud cubes spread over a region's +X face, 0.1 and 0.5 past it, and over a listed 8-stud cube's face 0.5 from
            -- it (at 4 mod 8, as measured)
            local function Plane(Base, X)
                for I = 0, 13 do
                    for J = 0, 13 do
                        Make(Base + Vector3.new(X, 0.35 + I * 0.56, 0.35 + J * 0.56), Vector3.new(0.5, 0.5, 0.5))
                    end
                end
            end
            local NearBase, FarBase, CubeBase = Vector3.new(10004, 1004, 4), Vector3.new(10100, 1004, 4), Vector3.new(12004, 1004, 4)
            Plane(NearBase, 8.1 + 0.25)
            Plane(FarBase, 8.5 + 0.25)
            Plane(CubeBase, 8.5 + 0.25)
            local RNear, RFar = Region3.new(NearBase, NearBase + Vector3.new(8, 8, 8)), Region3.new(FarBase, FarBase + Vector3.new(8, 8, 8))
            local Cube = { Make(CubeBase + Vector3.new(4, 4, 4), Vector3.new(8, 8, 8)) }
            local Lone = { Make(Vector3.new(-9000, 900, 0), Vector3.new(2, 2, 2)) }
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            local RWaiting = Grid(Vector3.new(7000, 900, 900), 1000)
            Within("FindPartsInRegion3 over 1,000 parts waiting for the index", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(RWaiting, nil, 0) end end), 0.2, 0.6)
            Within("over 100 parts with CanQuery and CanCollide off", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(RHidden, nil, 0) end end), 5.2, 8.5)
            Within("beside 196 parts 0.1 past its face", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(RNear, nil, 1) end end), 15.5, 19.5)
            Within("IsRegion3Empty beside them", Per(function(N) for _ = 1, N do workspace:IsRegion3Empty(RNear) end end), 15.5, 19.5)
            Within("beside 196 parts 0.5 past its face", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(RFar, nil, 1) end end), 1.8, 3.2)
            -- what ArePartsTouchingOthers takes into the index costs it as other queries pay; what a WorldModel's world waits for,
            -- nothing
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            local Alone = Per(function(N) for _ = 1, N do workspace:ArePartsTouchingOthers(Lone) end end)
            local function Took(Holder, List)
                local Start = os.clock()
                Holder:ArePartsTouchingOthers(List)
                return (os.clock() - Start) * 1e6 - Alone
            end
            local World = Instance.new("WorldModel")
            World.Parent = game:GetService("ReplicatedStorage")
            local InWorld = { Make(Vector3.new(0, 0, 0), Vector3.new(2, 2, 2)) }
            InWorld[1].Parent = World
            for I = 1, 100 do
                Make(Vector3.new(I * 3, 50, 0), Vector3.one).Parent = World
            end
            Within("ArePartsTouchingOthers of the Workspace with 100 of a WorldModel's parts waiting", Took(workspace, Lone), -0.3, 0.3)
            Within("of the WorldModel, taking them in", Took(World, InWorld), 100, 135)
            Within("of the WorldModel, after", Took(World, InWorld), -0.3, 0.3)
            -- the first query of a WorldModel of 1,000 parts makes its index (ree's own work: cost.rs FIND_NODE an instance walked;
            -- the parts wait, and a query that takes them in pays for that, so its walk files none, MODEL_INDEX_PART); one after an
            -- ArePartsTouchingOthers of nothing, which took them in and made the index, pays nothing more
            local function Once(F)
                local Start = os.clock()
                F()
                return (os.clock() - Start) * 1e6
            end
            local Region = Region3.new(Vector3.new(-1, -1, -1), Vector3.new(1, 1, 1))
            for _, Paid in { false, true } do
                local Big = Instance.new("WorldModel")
                Big.Parent = game:GetService("ReplicatedStorage")
                for I = 1, 1000 do
                    Make(Vector3.new(I * 3, 80, 0), Vector3.one).Parent = Big
                end
                if Paid then
                    Big:ArePartsTouchingOthers({})
                end
                local Made = Once(function() Big:IsRegion3Empty(Region) end)
                local After = Once(function() Big:IsRegion3Empty(Region) end)
                if Paid then
                    Within("the first IsRegion3Empty of a WorldModel of 1,000 parts it took in, its index made", Made - After, -0.3, 0.3)
                else
                    Within("the first IsRegion3Empty of a WorldModel of 1,000 parts, its index made", Made - After, 3, 5)
                end
                Big:Destroy()
            end
            for I = 1, 100 do
                Make(Vector3.new(I * 3, -9000, 0), Vector3.one)
            end
            Within("of the Workspace, taking in 100 parts", Took(workspace, Lone), 100, 135)
            World:Destroy()
            Within("of a cube beside 196 parts 0.5 from it", Per(function(N) for _ = 1, N do workspace:ArePartsTouchingOthers(Cube) end end), 2.2, 3.4)
            -- a list checked against parts deep down: 100 entries elsewhere over 10 parts 100 deep, each walked to the top for each
            -- entry (a client 58 to 79 us)
            local Chain = workspace
            for _ = 1, 100 do
                local F = Instance.new("Folder")
                F.Parent = Chain
                Chain = F
            end
            local Ten, RTen = Line(Vector3.new(8000, 900, 900), 10)
            Ten.Parent = Chain
            local Hundred = {}
            for I = 1, 100 do
                Hundred[I] = Make(Vector3.new(-8000, I * 3, 0), Vector3.one)
            end
            workspace:GetPartBoundsInBox(CFrame.new(0, -1000, 0), Vector3.one)
            Within("an ignore list of 100 over 10 parts 100 deep", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3WithIgnoreList(RTen, Hundred, 0) end end), 50, 85)
            -- regions that are not finite find nothing, for less than a finite empty one (a client 0.27 us)
            local Inf = math.huge
            Within("a region from -inf to inf", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(Region3.new(-Vector3.one * Inf, Vector3.one * Inf), nil, 1) end end), 0.2, 0.5)
            assert(#workspace:FindPartsInRegion3(Region3.new(Vector3.new(2990, 890, 890), Vector3.new(Inf, 910, 910)), nil, 0) == 0)
            -- a call that raises (a client's Region3 query that raises costs past an empty pcall, its call included, 3.7 to 4.5
            -- us for FindPartsInRegion3's argument errors, 3.8 to 6.0 for IsRegion3Empty's and 6.2 to 9.5 for
            -- FindPartsInRegion3WithIgnoreList given a list whose first entry is no Instance, not matched;
            -- ArePartsTouchingOthers 4.6 to 9.2); a bad entry after 1,000 parts costs each entry before it (0.23 to 0.26 us a
            -- client)
            local function Empty() end
            local Pcall = Per(function(N) for _ = 1, N do pcall(Empty) end end)
            Within("FindPartsInRegion3 with no region", Per(function(N) for _ = 1, N do pcall(workspace.FindPartsInRegion3, workspace) end end) - Pcall, 3.4, 4.5)
            Within("ArePartsTouchingOthers with an overlap that is no number", Per(function(N) for _ = 1, N do pcall(workspace.ArePartsTouchingOthers, workspace, {}, "x") end end) - Pcall, 6.5, 8)
            local Bad = table.clone(Hundred)
            for I = 101, 1000 do
                Bad[I] = Hundred[1]
            end
            Bad[1001] = 5
            Within("an ignore list with a bad entry after 1,000", Per(function(N) for _ = 1, N do pcall(workspace.FindPartsInRegion3WithIgnoreList, workspace, RTen, Bad) end end) - Pcall, 190, 245)
            Bad[1001] = Instance.new("Folder")
            Within("ArePartsTouchingOthers with a Folder after 1,000", Per(function(N) for _ = 1, N do pcall(workspace.ArePartsTouchingOthers, workspace, Bad) end end) - Pcall, 26, 37)
            Within("ArePartsTouchingOthers of 30 parts in one spot", Per(function(N) for _ = 1, N do workspace:ArePartsTouchingOthers(Stacked) end end), 55, 80)
            Within("of one among 100 with CanCollide off", Per(function(N) for _ = 1, N do workspace:ArePartsTouchingOthers(Among) end end), 5.5, 9)
            Within("of one beside 100", Per(function(N) for _ = 1, N do workspace:ArePartsTouchingOthers(Beside) end end), 15.5, 19.5)
            Within("FindPartsInRegion3 over 50 parts", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(R50, nil, 0) end end), 6.5, 8)
            Within("an ignore list over 50 parts under 256 Folders", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3WithIgnoreList(RDeep, { Unrelated }, 0) end end), 17, 23.5)
            Within("with maxParts 5", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3WithIgnoreList(RDeep, { Unrelated }, 5) end end), 2.8, 4.2)
            Within("over 200 parts, maxParts 5", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(R200, nil, 5) end end), 4.5, 5.8)
            Within("over 50 parts ignored", Per(function(N) for _ = 1, N do workspace:FindPartsInRegion3(R50, Fifty, 0) end end), 3, 3.8)
            Within("IsRegion3Empty over 200 parts", Per(function(N) for _ = 1, N do workspace:IsRegion3Empty(R200) end end), 4, 5.2)
            Within("ArePartsTouchingOthers of 30 apart", Per(function(N) for _ = 1, N do workspace:ArePartsTouchingOthers(Apart) end end), 4.2, 5.2)
            Within("of 30, the first touching", Per(function(N) for _ = 1, N do workspace:ArePartsTouchingOthers(Listed) end end), 2.2, 2.9)
            "##,
        );
    }
}
