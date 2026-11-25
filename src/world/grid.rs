//! The spatial queries' grid: a client's broadphase as measured (Studio 0.747, roblox-sim's headless client and server alike;
//! docs/engine.md "The spatial grid", tools/parity/probes/grid_*.luau). world/broadphase.rs keeps one for each world (the
//! Workspace's, each WorldModel's) and files its parts here; this module is the grid alone: levels, cells, lists, the top level
//! and the walks.
//!
//! - **Levels** 0 to 7, level L's cells cubes of `8 * 2^L` studs aligned to the origin, cell `floor(x / s)` on each axis.
//! - **A part's filed box** is its box grown by 0.3 studs on every side in its own axes, then the world box of that
//!   ([`Filing::of`], float32 at the part's position), and its **level** the smallest L with `prod(G + 16) <= 51200 * 8^L` (G
//!   the filed box's size: the box grown by 8 more studs a side fits in 100 cells of the level), at most 7 (this float32 model
//!   gives the 16 level boundaries bisected on a client to the bit). It is filed in every closed cell of its level its filed
//!   box touches, and only in those within 2^23 cells of the origin on each axis (the far bound, 2^(26 + L) studs).
//! - **Each cell** holds two lists, the parts whose assembly is unanchored and the anchored ones, each in the order they came
//!   into the cell; a part leaving gives its place to the list's last ([`Grid::unfile`], [`Grid::move_cells`]).
//! - **The top level** is `max(5, the highest level holding a part)`: it rises in the frame a part of level 6 or 7 is filed and
//!   comes down in the frame after the last such part left ([`Grid::top`]: the highest level as the frame began, or filed since).
//! - **A box walk** ([`Grid::walk_box`]): the query's (clamped) world box Q, its level Lq (the smallest whose cells are as big as
//!   Q's largest side), its root level `Lr = min(Lq, top)`; the root level's cells in Q's range (`floor(lo / s)`, one less when
//!   `lo / s` is whole, to `floor(hi / s)`) in raster order, x outermost, each descended depth first: the cell's lists, then its
//!   eight children in Q's range on the level below in Morton order, down to level 0; then the levels Lq + 1 to top, each one's
//!   cells in Q's range in raster order. A part is given once, at the first cell that holds it; the box queries take a cell's
//!   unanchored list first, the casts its anchored one.
//! - **A ray walk** ([`Grid::walk_ray`], Raycast and the rays like it): level 0 first and up to the top, on each level the cells
//!   the segment passes in the order it enters them (inferred: a client's order across cells is not measured), each cell's
//!   anchored list before its unanchored one.
//!
//! Host work: only cells that hold something exist (a map a level, each node with its lists and a mask of which of its eight
//! children exist, kept up to level 7), so a walk descends only into what is there, and a range of many root cells is read from
//! the level's nodes rather than cell by cell; what a client spends walking every cell in range is counted ([`Walked`]), not
//! walked. A part's place in a list is found by a search of the list (lists are short).

use glam::{Affine3A, Vec3};
use rustc_hash::FxHashMap;

/// How many levels the grid has
pub const LEVELS: usize = 8;
/// The lowest the top level comes down to
pub const TOP_FLOOR: u8 = 5;
/// The far bound: cells are kept within this many of the origin on each axis, either side (`-FAR..FAR`: measured, a 2-stud part
/// at 2^26 on any axis and side is found, one at 2^26 + 8 not: their float32 boxes shrink to the point, which on a cell's edge
/// is also in the cell below, so in cell 2^23 - 1 and 2^23, or 2^23 and 2^23 + 1, of level 0)
pub const FAR: i64 = 1 << 23;
/// How far a part's box is grown on every side, in its own axes, before it is filed (measured: 0.3 on every level)
pub const MARGIN: f32 = 0.3;
/// A part's level is the first whose 100 cells hold its filed box grown by 8 a side: `prod(G + 16) <= LEVEL_BOUND * 8^L`
pub const LEVEL_BOUND: f64 = 51200.0;

/// The two lists of a cell
pub const UNANCHORED: u8 = 0;
pub const ANCHORED: u8 = 1;

pub type Key = [i32; 3];

/// The side of a cell of `level`
pub fn cell_size(level: u32) -> f64 {
    8.0 * f64::from(1u32 << level.min(30))
}

/// Where a part is filed: its level, the cells it meets there (inclusive, within the far bound: empty when none is), its list
/// and whether CanCollide and CanTouch are both off (a change of which files it again, as a change of level or list does)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filing {
    pub level: u8,
    pub lo: Key,
    pub hi: Key,
    pub list: u8,
    pub off: bool,
}

impl Filing {
    /// The filing of a part whose frame is `frame` (unit axes) and size `size`, in `list`: None when its box is not finite.
    pub fn of(frame: &Affine3A, size: Vec3, list: u8, off: bool) -> Option<Filing> {
        let (lo, hi) = filed_box(frame, size)?;
        let level = part_level(hi - lo);
        let (lo, hi) = cells(lo, hi, level, true);
        Some(Filing { level, lo, hi, list, off })
    }

    /// Whether it holds no cell (all beyond the far bound)
    pub fn is_empty(&self) -> bool {
        (0..3).any(|axis| self.lo[axis] > self.hi[axis])
    }

    /// Whether `key` is one of its cells
    pub fn holds(&self, key: Key) -> bool {
        (0..3).all(|axis| self.lo[axis] <= key[axis] && key[axis] <= self.hi[axis])
    }

    /// Its cells, x outermost
    pub fn cells(&self) -> impl Iterator<Item = Key> + '_ {
        let (lo, hi) = (self.lo, self.hi);
        (lo[0]..=hi[0]).flat_map(move |x| (lo[1]..=hi[1]).flat_map(move |y| (lo[2]..=hi[2]).map(move |z| [x, y, z])))
    }
}

/// A part's filed box (float32, as a client holds it): its box grown by [`MARGIN`] on every side in its own axes, then the
/// world box of that; None when it is not finite.
pub fn filed_box(frame: &Affine3A, size: Vec3) -> Option<(Vec3, Vec3)> {
    let own = size.abs() * 0.5 + Vec3::splat(MARGIN);
    let m = frame.matrix3;
    let half = Vec3::from(m.x_axis).abs() * own.x + Vec3::from(m.y_axis).abs() * own.y + Vec3::from(m.z_axis).abs() * own.z;
    let center = Vec3::from(frame.translation);
    let (lo, hi) = (center - half, center + half);
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

/// The level of a part whose filed box is `size` across: the first whose 100 cells hold it grown by 8 a side, at most 7
pub fn part_level(size: Vec3) -> u8 {
    let volume = (f64::from(size.x) + 16.0) * (f64::from(size.y) + 16.0) * (f64::from(size.z) + 16.0);
    let mut bound = LEVEL_BOUND;
    for level in 0..LEVELS as u8 - 1 {
        if volume <= bound {
            return level;
        }
        bound *= 8.0;
    }
    LEVELS as u8 - 1
}

/// The level a box query of the world box `lo`..`hi` starts at: the first whose cells are as big as its largest side (more
/// than 7 for a box past 1,024 studs, where no part is)
pub fn query_level(lo: Vec3, hi: Vec3) -> u32 {
    let side = f64::from((hi - lo).max_element());
    let mut level = 0u32;
    while level < 30 && side > cell_size(level) {
        level += 1;
    }
    level
}

/// The cells of `level` the box `lo`..`hi` meets (inclusive), clipped to the far bound: from `floor(lo / s)` to `floor(hi / s)`,
/// and with `low_edge` one more below where `lo / s` is whole: every closed cell the closed box touches. A query's range and a
/// part's cells both take it (measured: a query's low edge on a cell edge also walks the cell below, grid_walk "query from
/// 2048"; a part's low edge switches cells one float32 step above the cell's edge, grid_levels "margin L")
pub fn cells(lo: Vec3, hi: Vec3, level: u8, low_edge: bool) -> (Key, Key) {
    let s = cell_size(u32::from(level));
    let mut from = [0i32; 3];
    let mut to = [0i32; 3];
    for axis in 0..3 {
        let (l, h) = (f64::from(lo[axis]) / s, f64::from(hi[axis]) / s);
        let mut first = l.floor();
        if low_edge && first == l {
            first -= 1.0;
        }
        let clip = |c: f64| c.clamp(-(FAR as f64) - 1.0, FAR as f64 + 1.0) as i64;
        let (first, last) = (clip(first).max(-FAR), clip(h.floor()).min(FAR - 1));
        // (a side wholly beyond the bound holds no cell: an empty range)
        if first > last {
            from[axis] = 1;
            to[axis] = 0;
        } else {
            from[axis] = first as i32;
            to[axis] = last as i32;
        }
    }
    (from, to)
}

/// How many cells the notes' walk of the box `lo`..`hi` visits at most with the top level `top` (its root cells and every cell
/// below them): the tests' scan mode walks it so when that is small enough
pub fn count_walk(lo: Vec3, hi: Vec3, top: u8) -> u64 {
    let lr = query_level(lo, hi).min(u32::from(top));
    count(cells(lo, hi, lr as u8, true)).saturating_mul((8u64.pow(lr + 1) - 1) / 7)
}

/// How many cells the brute-force ray walk tests for the segment from `origin` along `direction` (every level's cells in its box)
pub fn count_ray(origin: Vec3, direction: Vec3, top: u8) -> u64 {
    let (o, d) = (origin.as_dvec3(), direction.as_dvec3());
    let (lo, hi) = (o.min(o + d).as_vec3(), o.max(o + d).as_vec3());
    (0..=top).map(|level| count(cells(lo - Vec3::splat(1e-3), hi + Vec3::splat(1e-3), level, false))).fold(0u64, u64::saturating_add)
}

#[derive(Default, Debug)]
struct Node {
    /// Its two lists (by [`UNANCHORED`], [`ANCHORED`])
    lists: [Vec<u32>; 2],
    /// Which of its eight children exist (bit m: child m in Morton order)
    children: u8,
}

impl Node {
    fn is_empty(&self) -> bool {
        self.children == 0 && self.lists[0].is_empty() && self.lists[1].is_empty()
    }

    fn holds_parts(&self) -> bool {
        !(self.lists[0].is_empty() && self.lists[1].is_empty())
    }
}

fn parent(key: Key) -> (Key, u8) {
    ([key[0] >> 1, key[1] >> 1, key[2] >> 1], ((key[0] & 1) | ((key[1] & 1) << 1) | ((key[2] & 1) << 2)) as u8)
}

fn child(key: Key, m: u8) -> Key {
    [2 * key[0] + i32::from(m & 1), 2 * key[1] + i32::from((m >> 1) & 1), 2 * key[2] + i32::from((m >> 2) & 1)]
}

fn within(key: Key, (lo, hi): (Key, Key)) -> bool {
    (0..3).all(|axis| lo[axis] <= key[axis] && key[axis] <= hi[axis])
}

fn count((lo, hi): (Key, Key)) -> u64 {
    (0..3).map(|axis| (i64::from(hi[axis]) - i64::from(lo[axis]) + 1).max(0) as u64).product()
}

/// What a walk did, as a client's walk of every cell in range costs it: the root cells in its range (empty or not, counted, not
/// walked), the cells it visited that gave it a part it had not met (a client's price for a cell is measured on cells of parts of
/// their own: one whose parts were all met in an earlier cell is not counted), and the parts it was given (each once)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Walked {
    pub root_cells: u64,
    pub cells: u64,
    pub given: u64,
}

/// A cell a ray walk passes: its level, where the segment enters it (a fraction of the segment) and its key
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Passed {
    pub level: u8,
    pub enter: f64,
    pub key: Key,
}

/// One world's grid
#[derive(Default)]
pub struct Grid {
    levels: [FxHashMap<Key, Node>; LEVELS],
    /// The parts filed on each level (with any cell)
    counts: [u32; LEVELS],
    /// The frame `top` is for, and the top level then
    frame: u64,
    top: u8,
    /// Which walk last gave each entry (the walks' marks: a part is given once a walk)
    seen: Vec<u32>,
    walk: u32,
    stack: Vec<(u8, Key)>,
    roots: Vec<Key>,
    passed: Vec<Passed>,
}

impl Grid {
    /// The top level in `frame`: the highest level holding a part as the frame began, at least 5, raised by any filed since
    pub fn top(&mut self, frame: u64) -> u8 {
        self.roll(frame);
        self.top
    }

    fn roll(&mut self, frame: u64) {
        if frame != self.frame || self.top == 0 {
            self.frame = frame;
            self.top = (0..LEVELS).rev().find(|&level| self.counts[level] > 0).map_or(0, |level| level as u8).max(TOP_FLOOR);
        }
    }

    /// How many parts are filed on each level
    pub fn counts(&self) -> [u32; LEVELS] {
        self.counts
    }

    /// Files `entry` at the end of its list in each of `filing`'s cells.
    pub fn file(&mut self, entry: u32, filing: &Filing, frame: u64) {
        if filing.is_empty() {
            return;
        }
        self.roll(frame);
        let level = filing.level;
        for key in filing.cells() {
            self.append(level, key, filing.list, entry);
        }
        self.counts[level as usize] += 1;
        self.top = self.top.max(level);
    }

    /// Takes `entry` out of each of `filing`'s cells: in each, the list's last takes its place.
    pub fn unfile(&mut self, entry: u32, filing: &Filing, frame: u64) {
        if filing.is_empty() {
            return;
        }
        self.roll(frame);
        let level = filing.level;
        for key in filing.cells() {
            self.take(level, key, filing.list, entry);
        }
        self.counts[level as usize] -= 1;
    }

    /// Moves `entry` from the cells of `old` to those of `new`, of the same level and list: out of the cells it no longer meets,
    /// at the end of those it now meets, in its place in those it stays in.
    pub fn move_cells(&mut self, entry: u32, old: &Filing, new: &Filing, frame: u64) {
        debug_assert!(old.level == new.level && old.list == new.list);
        match (old.is_empty(), new.is_empty()) {
            (true, true) => return,
            (true, false) => return self.file(entry, new, frame),
            (false, true) => return self.unfile(entry, old, frame),
            (false, false) => {}
        }
        self.roll(frame);
        for key in old.cells() {
            if !new.holds(key) {
                self.take(old.level, key, old.list, entry);
            }
        }
        for key in new.cells() {
            if !old.holds(key) {
                self.append(new.level, key, new.list, entry);
            }
        }
    }

    fn append(&mut self, level: u8, key: Key, list: u8, entry: u32) {
        let map = &mut self.levels[level as usize];
        let made = !map.contains_key(&key);
        map.entry(key).or_default().lists[list as usize].push(entry);
        if made {
            self.link(level, key);
        }
    }

    /// Marks a new node of `level` in its parent, making the parents it needs up to level 7.
    fn link(&mut self, mut level: u8, mut key: Key) {
        while (level as usize) < LEVELS - 1 {
            let (up, bit) = parent(key);
            let map = &mut self.levels[level as usize + 1];
            let made = !map.contains_key(&up);
            map.entry(up).or_default().children |= 1 << bit;
            if !made {
                return;
            }
            level += 1;
            key = up;
        }
    }

    fn take(&mut self, level: u8, key: Key, list: u8, entry: u32) {
        let map = &mut self.levels[level as usize];
        let Some(node) = map.get_mut(&key) else { return };
        let held = &mut node.lists[list as usize];
        if let Some(place) = held.iter().position(|&e| e == entry) {
            held.swap_remove(place);
        }
        if node.is_empty() {
            map.remove(&key);
            self.unlink(level, key);
        }
    }

    /// Clears an emptied node's mark in its parent, removing the parents left empty.
    fn unlink(&mut self, mut level: u8, mut key: Key) {
        while (level as usize) < LEVELS - 1 {
            let (up, bit) = parent(key);
            let map = &mut self.levels[level as usize + 1];
            let Some(node) = map.get_mut(&up) else { return };
            node.children &= !(1 << bit);
            if !node.is_empty() {
                return;
            }
            map.remove(&up);
            level += 1;
            key = up;
        }
    }

    /// The next walk's mark, the marks grown to hold `entries` entries
    fn begin(&mut self, entries: usize) -> u32 {
        if self.seen.len() < entries {
            self.seen.resize(entries, 0);
        }
        self.walk = self.walk.wrapping_add(1);
        if self.walk == 0 {
            self.seen.iter_mut().for_each(|mark| *mark = 0);
            self.walk = 1;
        }
        self.walk
    }

    /// Gives `visit` each part of the cell `key` of `level` not given before in this walk, `first` list first; false when
    /// `visit` asked to stop.
    #[allow(clippy::too_many_arguments)]
    fn visit_cell(levels: &[FxHashMap<Key, Node>; LEVELS], seen: &mut [u32], walk: u32, level: u8, key: Key, first: u8, walked: &mut Walked, visit: &mut impl FnMut(u32) -> bool) -> bool {
        let Some(node) = levels[level as usize].get(&key) else { return true };
        if !node.holds_parts() {
            return true;
        }
        let mut fresh = false;
        for list in [first, 1 - first] {
            for &entry in &node.lists[list as usize] {
                let mark = &mut seen[entry as usize];
                if *mark == walk {
                    continue;
                }
                *mark = walk;
                walked.given += 1;
                if !fresh {
                    fresh = true;
                    walked.cells += 1;
                }
                if !visit(entry) {
                    return false;
                }
            }
        }
        true
    }

    /// The box walk over the world box `lo`..`hi` (a query's, clamped by the caller) in `frame`, `entries` the most entries the
    /// world holds: each part of the cells it walks, once, to `visit` until it returns false (a cell's `first` list first:
    /// [`UNANCHORED`] for the box queries, [`ANCHORED`] for the casts). What it did, as a client's walk costs it.
    pub fn walk_box(&mut self, lo: Vec3, hi: Vec3, frame: u64, entries: usize, first: u8, mut visit: impl FnMut(u32) -> bool) -> Walked {
        let top = self.top(frame);
        let walk = self.begin(entries);
        let lq = query_level(lo, hi);
        let lr = lq.min(u32::from(top)) as u8;
        let ranges: Vec<(Key, Key)> = (0..=top).map(|level| cells(lo, hi, level, true)).collect();
        let mut walked = Walked { root_cells: count(ranges[lr as usize]), ..Walked::default() };
        // the root level's cells in range, raster order: cell by cell when few, else the level's nodes in range, sorted
        let mut roots = std::mem::take(&mut self.roots);
        roots.clear();
        let range = ranges[lr as usize];
        let map = &self.levels[lr as usize];
        if walked.root_cells <= (map.len() as u64).max(64) {
            let (a, b) = range;
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        if map.contains_key(&[x, y, z]) {
                            roots.push([x, y, z]);
                        }
                    }
                }
            }
        } else {
            roots.extend(map.keys().copied().filter(|&key| within(key, range)));
            roots.sort_unstable();
        }
        let Grid { levels, seen, stack, .. } = self;
        let mut going = true;
        'roots: for &root in &roots {
            stack.clear();
            stack.push((lr, root));
            while let Some((level, key)) = stack.pop() {
                if !Self::visit_cell(levels, seen, walk, level, key, first, &mut walked, &mut visit) {
                    going = false;
                    break 'roots;
                }
                if level == 0 {
                    continue;
                }
                let Some(node) = levels[level as usize].get(&key) else { continue };
                let below = ranges[level as usize - 1];
                // (children pushed last first, so they come off the stack in Morton order)
                for m in (0..8u8).rev() {
                    if node.children & (1 << m) != 0 {
                        let next = child(key, m);
                        if within(next, below) {
                            stack.push((level - 1, next));
                        }
                    }
                }
            }
        }
        self.roots = roots;
        if going {
            // the levels above the query's, each in raster order
            let Grid { levels, seen, .. } = self;
            'above: for level in (lq + 1)..=u32::from(top) {
                let (a, b) = ranges[level as usize];
                if levels[level as usize].is_empty() {
                    continue;
                }
                for x in a[0]..=b[0] {
                    for y in a[1]..=b[1] {
                        for z in a[2]..=b[2] {
                            if !Self::visit_cell(levels, seen, walk, level as u8, [x, y, z], first, &mut walked, &mut visit) {
                                break 'above;
                            }
                        }
                    }
                }
            }
        }
        walked
    }

    /// The ray walk along the segment from `origin` along `direction` in `frame`: the cells holding parts it passes, level 0
    /// first and up to the top, on each level in the order it enters them, into `passed`; and each part of them once, a cell's
    /// anchored list first, into `out` with the index of the first cell of `passed` that holds it.
    pub fn walk_ray(&mut self, origin: Vec3, direction: Vec3, frame: u64, entries: usize, passed: &mut Vec<Passed>, out: &mut Vec<(u32, u32)>) {
        passed.clear();
        let top = self.top(frame);
        let walk = self.begin(entries);
        let (o, d) = (origin.as_dvec3(), direction.as_dvec3());
        let end = o + d;
        let (lo, hi) = (o.min(end).as_vec3(), o.max(end).as_vec3());
        // the level-7 nodes the segment meets: cell by cell over its box when few, else every node of the level
        let level = (LEVELS - 1) as u8;
        let range = cells(lo - Vec3::splat(1e-3), hi + Vec3::splat(1e-3), level, false);
        let mut found = std::mem::take(&mut self.passed);
        found.clear();
        let mut stack = std::mem::take(&mut self.stack);
        stack.clear();
        let map = &self.levels[level as usize];
        if count(range) <= (map.len() as u64).max(64) {
            let (a, b) = range;
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        if map.contains_key(&[x, y, z]) {
                            stack.push((level, [x, y, z]));
                        }
                    }
                }
            }
        } else {
            stack.extend(map.keys().copied().map(|key| (level, key)));
        }
        while let Some((level, key)) = stack.pop() {
            let Some(enter) = segment_enters(o, d, level, key) else { continue };
            let Some(node) = self.levels[level as usize].get(&key) else { continue };
            if node.holds_parts() && level <= top {
                found.push(Passed { level, enter, key });
            }
            if level > 0 {
                for m in 0..8u8 {
                    if node.children & (1 << m) != 0 {
                        stack.push((level - 1, child(key, m)));
                    }
                }
            }
        }
        self.stack = stack;
        found.sort_by(compare_passed);
        for (index, cell) in found.iter().enumerate() {
            let node = &self.levels[cell.level as usize][&cell.key];
            for list in [ANCHORED, UNANCHORED] {
                for &entry in &node.lists[list as usize] {
                    let mark = &mut self.seen[entry as usize];
                    if *mark != walk {
                        *mark = walk;
                        out.push((entry, index as u32));
                    }
                }
            }
        }
        passed.extend_from_slice(&found);
        self.passed = found;
    }

    /// Whether every node is linked from its parent, every parent's mask names exactly its children, no node is empty, and the
    /// counts are the parts filed on each level (tests: `filings` the filing of each entry filed)
    #[cfg(test)]
    pub(crate) fn check(&self, filings: &[(u32, Filing)]) -> Result<(), String> {
        for (level, map) in self.levels.iter().enumerate() {
            for (key, node) in map {
                if node.is_empty() {
                    return Err(format!("an empty node at level {level} {key:?}"));
                }
                for m in 0..8u8 {
                    let exists = level > 0 && self.levels[level - 1].contains_key(&child(*key, m));
                    if exists != (node.children & (1 << m) != 0) {
                        return Err(format!("level {level} {key:?}: child {m} {exists}"));
                    }
                }
                if level + 1 < LEVELS {
                    let (up, bit) = parent(*key);
                    if !self.levels[level + 1].get(&up).is_some_and(|node| node.children & (1 << bit) != 0) {
                        return Err(format!("level {level} {key:?}: not linked"));
                    }
                }
            }
        }
        let mut counts = [0u32; LEVELS];
        let mut cells = 0usize;
        for (entry, filing) in filings {
            if filing.is_empty() {
                continue;
            }
            counts[filing.level as usize] += 1;
            for key in filing.cells() {
                cells += 1;
                let held = self.levels[filing.level as usize].get(&key).map_or(0, |node| node.lists[filing.list as usize].iter().filter(|&&e| e == *entry).count());
                if held != 1 {
                    return Err(format!("entry {entry} held {held} times in {key:?}"));
                }
            }
        }
        if counts != self.counts {
            return Err(format!("counts {:?}, filed {counts:?}", self.counts));
        }
        let held: usize = self.levels.iter().flat_map(|map| map.values()).map(|node| node.lists[0].len() + node.lists[1].len()).sum();
        if held != cells {
            return Err(format!("{held} held, {cells} filed"));
        }
        Ok(())
    }

    /// [`Grid::walk_box`] as the notes write it: every cell in range on every level visited, none skipped for being empty (tests:
    /// the order and counts the fast walk must give)
    #[cfg(test)]
    pub(crate) fn walk_box_reference(&mut self, lo: Vec3, hi: Vec3, frame: u64, entries: usize, first: u8) -> (Vec<u32>, Walked) {
        let top = self.top(frame);
        let walk = self.begin(entries);
        let lq = query_level(lo, hi);
        let lr = lq.min(u32::from(top)) as u8;
        let ranges: Vec<(Key, Key)> = (0..LEVELS as u8).map(|level| cells(lo, hi, level, true)).collect();
        let mut walked = Walked { root_cells: count(ranges[lr as usize]), ..Walked::default() };
        let mut out = Vec::new();
        let Grid { levels, seen, .. } = self;
        let mut visit = |entry: u32| {
            out.push(entry);
            true
        };
        #[allow(clippy::too_many_arguments)]
        fn descend(levels: &[FxHashMap<Key, Node>; LEVELS], seen: &mut [u32], walk: u32, ranges: &[(Key, Key)], level: u8, key: Key, first: u8, walked: &mut Walked, visit: &mut impl FnMut(u32) -> bool) {
            Grid::visit_cell(levels, seen, walk, level, key, first, walked, visit);
            if level > 0 {
                for m in 0..8u8 {
                    let next = child(key, m);
                    if within(next, ranges[level as usize - 1]) {
                        descend(levels, seen, walk, ranges, level - 1, next, first, walked, visit);
                    }
                }
            }
        }
        let (a, b) = ranges[lr as usize];
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    // (the levels from the query's down to the root's above this root cell: empty, as they are above the top)
                    for level in (u32::from(lr) + 1..=lq.min(LEVELS as u32 - 1)).rev() {
                        let shift = level - u32::from(lr);
                        Self::visit_cell(levels, seen, walk, level as u8, [x >> shift, y >> shift, z >> shift], first, &mut walked, &mut visit);
                    }
                    descend(levels, seen, walk, &ranges, lr, [x, y, z], first, &mut walked, &mut visit);
                }
            }
        }
        for level in (lq + 1)..=u32::from(top) {
            let (a, b) = ranges[level as usize];
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        Self::visit_cell(levels, seen, walk, level as u8, [x, y, z], first, &mut walked, &mut visit);
                    }
                }
            }
        }
        (out, walked)
    }

    /// [`Grid::walk_ray`] by brute force: every cell of every level in the segment's box tested (tests)
    #[cfg(test)]
    pub(crate) fn walk_ray_reference(&mut self, origin: Vec3, direction: Vec3, frame: u64, entries: usize) -> (Vec<Passed>, Vec<(u32, u32)>) {
        let top = self.top(frame);
        let walk = self.begin(entries);
        let (o, d) = (origin.as_dvec3(), direction.as_dvec3());
        let end = o + d;
        let (lo, hi) = (o.min(end).as_vec3(), o.max(end).as_vec3());
        let mut found = Vec::new();
        for level in 0..=top {
            let (a, b) = cells(lo - Vec3::splat(1e-3), hi + Vec3::splat(1e-3), level, false);
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        if self.levels[level as usize].get(&[x, y, z]).is_some_and(Node::holds_parts)
                            && let Some(enter) = segment_enters(o, d, level, [x, y, z])
                        {
                            found.push(Passed { level, enter, key: [x, y, z] });
                        }
                    }
                }
            }
        }
        found.sort_by(compare_passed);
        let mut out = Vec::new();
        for (index, cell) in found.iter().enumerate() {
            let node = &self.levels[cell.level as usize][&cell.key];
            for list in [ANCHORED, UNANCHORED] {
                for &entry in &node.lists[list as usize] {
                    if self.seen[entry as usize] != walk {
                        self.seen[entry as usize] = walk;
                        out.push((entry, index as u32));
                    }
                }
            }
        }
        (found, out)
    }
}

fn compare_passed(a: &Passed, b: &Passed) -> std::cmp::Ordering {
    a.level.cmp(&b.level).then(a.enter.total_cmp(&b.enter)).then(a.key.cmp(&b.key))
}

/// Where the segment from `o` along `d` enters the cell `key` of `level` (a fraction of the segment, 0 when it starts inside),
/// or None when it misses it; the cell grown by a thousandth of a stud, so a segment along a cell's face is in it.
fn segment_enters(o: glam::DVec3, d: glam::DVec3, level: u8, key: Key) -> Option<f64> {
    let s = cell_size(u32::from(level));
    let (mut enter, mut exit) = (0.0f64, 1.0f64);
    for axis in 0..3 {
        let lo = f64::from(key[axis]) * s - 1e-3;
        let hi = lo + s + 2e-3;
        let (start, step) = (o[axis], d[axis]);
        if step == 0.0 {
            if start < lo || start > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - start) / step, (hi - start) / step);
        let (near, far) = if a < b { (a, b) } else { (b, a) };
        enter = enter.max(near);
        exit = exit.min(far);
        if enter > exit {
            return None;
        }
    }
    Some(enter)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next()
        }

        fn log(&mut self, lo: f64, hi: f64) -> f64 {
            self.range(lo.ln(), hi.ln()).exp()
        }
    }

    fn turned(rng: &mut Rng, at: Vec3) -> Affine3A {
        let rotation = if rng.next() < 0.35 { glam::Quat::from_euler(glam::EulerRot::XYZ, rng.range(0.0, std::f64::consts::TAU) as f32, rng.range(0.0, std::f64::consts::TAU) as f32, rng.range(0.0, std::f64::consts::TAU) as f32) } else { glam::Quat::IDENTITY };
        Affine3A::from_rotation_translation(rotation, at)
    }

    /// Levels and margins as measured (notes: grid_levels): cubes up to 20.53 on level 0, rods (S x 1 x 1) to 148.69, a turned
    /// 2048 cube on level 7, a 0.2 part at 4 studs into a cell in that cell alone
    #[test]
    fn parts_go_on_the_levels_a_client_puts_them_on() {
        let at = |size: Vec3| Filing::of(&Affine3A::from_translation(Vec3::new(4100.0, 1028.0, 4100.0)), size, ANCHORED, false).expect("finite").level;
        assert_eq!(at(Vec3::splat(20.5)), 0);
        assert_eq!(at(Vec3::splat(20.6)), 1);
        assert_eq!(at(Vec3::new(148.0, 1.0, 1.0)), 0);
        assert_eq!(at(Vec3::new(149.5, 1.0, 1.0)), 1);
        assert_eq!(at(Vec3::splat(1171.0)), 5);
        assert_eq!(at(Vec3::splat(1200.0)), 6);
        assert_eq!(at(Vec3::splat(2048.0)), 6);
        let turned = Affine3A::from_rotation_translation(glam::Quat::from_euler(glam::EulerRot::XYZ, std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4, 0.0), Vec3::new(4100.0, 1028.0, 4100.0));
        assert_eq!(Filing::of(&turned, Vec3::splat(2048.0), ANCHORED, false).expect("finite").level, 7);
        let small = Filing::of(&Affine3A::from_translation(Vec3::new(4.0, 4.0, 4.0)), Vec3::splat(0.2), ANCHORED, false).expect("finite");
        assert_eq!((small.lo, small.hi), ([0, 0, 0], [0, 0, 0]));
        // the far bound: 2^26 keeps its cell below the bound, 2^26 + 8 has none
        let far = |x: f32| Filing::of(&Affine3A::from_translation(Vec3::new(x, 0.0, 0.0)), Vec3::splat(2.0), ANCHORED, false).expect("finite");
        assert!(!far(67_108_864.0).is_empty() && far(67_108_872.0).is_empty() && !far(-67_108_864.0).is_empty() && far(-67_108_872.0).is_empty());
        assert_eq!(query_level(Vec3::ZERO, Vec3::splat(8.0)), 0);
        assert_eq!(query_level(Vec3::ZERO, Vec3::splat(8.001)), 1);
        assert_eq!(query_level(Vec3::ZERO, Vec3::new(20.0, 1.0, 1.0)), 2);
        assert_eq!(cells(Vec3::new(2048.0, 1.0, 1.0), Vec3::new(2060.0, 2.0, 2.0), 0, true).0[0], 255);
    }

    /// A cell's lists keep the order parts came in; a part leaving gives its place to the last (notes: grid_order)
    #[test]
    fn a_cell_keeps_the_order_parts_came_in() {
        let mut grid = Grid::default();
        let at = |x: f32, list: u8| Filing::of(&Affine3A::from_translation(Vec3::new(x, 4.0, 4.0)), Vec3::splat(0.2), list, false).expect("finite");
        let filings: Vec<Filing> = (0..5).map(|i| at(1.0 + i as f32, ANCHORED)).collect();
        for (entry, filing) in filings.iter().enumerate() {
            grid.file(entry as u32, filing, 1);
        }
        let order = |grid: &mut Grid| {
            let mut out = Vec::new();
            grid.walk_box(Vec3::new(0.5, 3.0, 3.0), Vec3::new(6.0, 5.0, 5.0), 1, 8, UNANCHORED, |entry| {
                out.push(entry);
                true
            });
            out
        };
        assert_eq!(order(&mut grid), [0, 1, 2, 3, 4]);
        grid.unfile(1, &filings[1], 1);
        assert_eq!(order(&mut grid), [0, 4, 2, 3]);
        let free = at(2.0, UNANCHORED);
        grid.file(1, &free, 1);
        assert_eq!(order(&mut grid), [1, 0, 4, 2, 3]);
        grid.unfile(1, &free, 1);
        grid.file(1, &filings[1], 1);
        assert_eq!(order(&mut grid), [0, 4, 2, 3, 1]);
        // grown into the next cell too: its place kept in the first
        let grown = Filing { hi: [1, 0, 0], ..filings[0] };
        grid.move_cells(0, &filings[0], &grown, 1);
        assert_eq!(order(&mut grid), [0, 4, 2, 3, 1]);
        let all: Vec<(u32, Filing)> = vec![(0, grown), (1, filings[1]), (2, filings[2]), (3, filings[3]), (4, filings[4])];
        grid.check(&all).expect("a sound grid");
    }

    /// The top level stays up through the frame its last level-6 part left, and comes down the frame after (notes: grid_walk)
    #[test]
    fn the_top_level_comes_down_a_frame_late() {
        let mut grid = Grid::default();
        assert_eq!(grid.top(1), 5);
        let six = Filing::of(&Affine3A::from_translation(Vec3::new(-20000.0, 1000.0, -20000.0)), Vec3::splat(2048.0), ANCHORED, false).expect("finite");
        assert_eq!(six.level, 6);
        grid.file(0, &six, 1);
        assert_eq!(grid.top(1), 6);
        assert_eq!(grid.top(2), 6);
        grid.unfile(0, &six, 2);
        assert_eq!(grid.top(2), 6, "the frame it left");
        assert_eq!(grid.top(3), 5, "a frame after");
        grid.file(0, &six, 4);
        grid.unfile(0, &six, 4);
        assert_eq!(grid.top(4), 6, "made and gone in one frame");
        assert_eq!(grid.top(5), 5);
    }

    /// The fast walks give what the notes' walks give, part for part and count for count, over random scenes of every level, as
    /// parts are filed, moved and taken out: box walks of every size (beside the clamp too) and ray walks
    #[test]
    fn the_walks_give_what_a_walk_of_every_cell_gives() {
        for seed in [1u64, 7, 42, 2024, 31337] {
            let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
            let mut grid = Grid::default();
            let center = Vec3::new(rng.range(-3000.0, 3000.0) as f32, rng.range(-500.0, 500.0) as f32, rng.range(-3000.0, 3000.0) as f32);
            let spread = rng.range(20.0, 300.0);
            let mut filings: Vec<Option<Filing>> = Vec::new();
            let make = |rng: &mut Rng| {
                let size = if rng.next() < 0.04 { Vec3::splat(rng.log(600.0, 2048.0) as f32) } else { Vec3::new(rng.log(0.2, 80.0) as f32, rng.log(0.2, 80.0) as f32, rng.log(0.2, 80.0) as f32) };
                let at = center + Vec3::new(rng.range(-spread, spread) as f32, rng.range(-spread, spread) as f32, rng.range(-spread, spread) as f32);
                let list = if rng.next() < 0.3 { UNANCHORED } else { ANCHORED };
                Filing::of(&turned(rng, at), size, list, false).expect("finite")
            };
            for round in 0..6u64 {
                let frame = round + 1;
                for _ in 0..120 {
                    let roll = rng.next();
                    let held: Vec<usize> = (0..filings.len()).filter(|&i| filings[i].is_some()).collect();
                    if roll < 0.6 || held.is_empty() {
                        let filing = make(&mut rng);
                        grid.file(filings.len() as u32, &filing, frame);
                        filings.push(Some(filing));
                    } else {
                        let entry = held[(rng.next() * held.len() as f64) as usize];
                        let old = filings[entry].expect("held");
                        if roll < 0.75 {
                            grid.unfile(entry as u32, &old, frame);
                            filings[entry] = None;
                        } else {
                            // (moved a little: the same level and list, other cells)
                            let shift = Vec3::new(rng.range(-12.0, 12.0) as f32, 0.0, rng.range(-12.0, 12.0) as f32);
                            let s = cell_size(u32::from(old.level)) as f32;
                            let new = Filing { lo: [old.lo[0] + (shift.x / s) as i32, old.lo[1], old.lo[2] + (shift.z / s) as i32], hi: [old.hi[0] + (shift.x / s) as i32, old.hi[1], old.hi[2] + (shift.z / s) as i32], ..old };
                            grid.move_cells(entry as u32, &old, &new, frame);
                            filings[entry] = Some(new);
                        }
                    }
                }
                let held: Vec<(u32, Filing)> = filings.iter().enumerate().filter_map(|(i, f)| f.map(|f| (i as u32, f))).collect();
                grid.check(&held).unwrap_or_else(|error| panic!("seed {seed} round {round}: {error}"));
                for query in 0..40 {
                    let size = if query % 5 == 0 { Vec3::splat(rng.log(300.0, 15000.0) as f32) } else { Vec3::new(rng.log(0.5, 300.0) as f32, rng.log(0.5, 300.0) as f32, rng.log(0.5, 300.0) as f32) };
                    let at = center + Vec3::new(rng.range(-spread, spread) as f32, rng.range(-spread, spread) as f32, rng.range(-spread, spread) as f32);
                    let (lo, hi) = (at - size * 0.5, at + size * 0.5);
                    // (the reference walks every cell below its root cells: keep it to queries it can finish quickly)
                    let top = grid.top(frame);
                    if count_walk(lo, hi, top) <= 100_000 {
                        let first = if query % 2 == 0 { UNANCHORED } else { ANCHORED };
                        let mut fast = Vec::new();
                        let walked = grid.walk_box(lo, hi, frame, filings.len(), first, |entry| {
                            fast.push(entry);
                            true
                        });
                        let reference = grid.walk_box_reference(lo, hi, frame, filings.len(), first);
                        assert_eq!((fast, walked), reference, "seed {seed} round {round} query {query}: {lo} {hi}");
                    }
                    // (a ray along an axis now and then, and long ones whose reference is quick enough)
                    let mut direction = Vec3::new(rng.range(-1.0, 1.0) as f32, rng.range(-1.0, 1.0) as f32, rng.range(-1.0, 1.0) as f32) * rng.log(5.0, 2000.0) as f32;
                    if query % 7 == 3 {
                        direction = Vec3::new(direction.x, 0.0, 0.0);
                    }
                    if count_ray(at, direction, grid.top(frame)) <= 200_000 {
                        let (mut passed, mut out) = (Vec::new(), Vec::new());
                        grid.walk_ray(at, direction, frame, filings.len(), &mut passed, &mut out);
                        assert_eq!((passed, out), grid.walk_ray_reference(at, direction, frame, filings.len()), "seed {seed} round {round} ray {query}");
                    }
                }
            }
        }
    }
}
