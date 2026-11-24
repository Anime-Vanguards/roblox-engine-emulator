//! ree's own caches, counted while a profile asks for them (`--profile caches`): hits, misses and evictions of each
//! cache the engine keeps between script calls. Off, a count site pays one relaxed load. The counts are the process's
//! (a profile runs in its own process): work the engine does on other threads (bundles decoded ahead, modules compiled
//! ahead) counts too. A host counts its own caches (rur's layout) through [`named`].
//!
//! The caches measure how fast ree runs, not what the game costs (the summary lists them under "emulator"); three of them
//! say something a client does too, and [`facts`] gives those as the game's: the instances its scripts touched (a userdata
//! made for each, as a client makes one), the modules that ran, and the datatype property reads (a client makes a value for
//! each read; ree hands back the one it made while the property is unchanged).

use std::{
    cell::RefCell,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

/// Active `--profile caches` sessions across all threads: off, a count site pays one relaxed load.
static ACTIVE_SESSIONS: AtomicUsize = AtomicUsize::new(0);
/// Set while more than one caches session overlaps in the process: each session then keeps only its own thread's counts
/// so parallel sessions do not mix background-thread counts into one another.
static CONCURRENT: AtomicBool = AtomicBool::new(false);

/// Instances touched on background threads (outside any session thread) while a single caches session is active.
static BG_TOUCHED: Mutex<Option<rustc_hash::FxHashSet<u64>>> = Mutex::new(None);

struct LocalCaches {
    active: bool,
    counts: [[u64; 3]; SLOTS],
    touched: rustc_hash::FxHashSet<u64>,
}

impl LocalCaches {
    fn new() -> Self {
        Self { active: false, counts: [[0; 3]; SLOTS], touched: rustc_hash::FxHashSet::default() }
    }
}

thread_local! {
    static LOCAL: RefCell<LocalCaches> = RefCell::new(LocalCaches::new());
}

/// A cache of the engine's
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cache {
    /// A string's member id, kept on the Luau string (its atom): found on the string (hit) or looked up by its bytes (miss)
    Atoms,
    /// The Instance API's dispatch per (class, member): found (hit) or worked out (miss); evictions: the table cleared
    Members,
    /// The engine's member resolution per (class, name)
    Resolve,
    /// A datatype property's converted value per (instance, property) while the instance is unchanged
    Values,
    /// An instance's userdata: found (hit) or made (miss); evictions: collected by the VM
    Wrappers,
    /// require: the module's result kept (hit) or the module run (miss)
    Requires,
    /// A module file's source and bytecode kept in the process (a service's, or read ahead): kept (hit) or read and
    /// compiled (miss); evictions: dropped as stale
    Bytecode,
    /// Spatial queries: the Workspace's parts in the broadphase, current (hit) or brought up to the dom's changes or made from
    /// the Workspace (miss); evictions: parts taken out
    Queries,
    /// A manifest bundle's converted plan: read from its plan file (hit) or converted from the bundle (miss)
    BundlePlans,
}

impl Cache {
    pub const ALL: [Cache; 9] = [Cache::Atoms, Cache::Members, Cache::Resolve, Cache::Values, Cache::Wrappers, Cache::Requires, Cache::Bytecode, Cache::Queries, Cache::BundlePlans];

    pub fn name(self) -> &'static str {
        match self {
            Cache::Atoms => "member atoms",
            Cache::Members => "member dispatch",
            Cache::Resolve => "member resolution",
            Cache::Values => "datatype values",
            Cache::Wrappers => "instance userdata",
            Cache::Requires => "require results",
            Cache::Bytecode => "module bytecode",
            Cache::Queries => "query broadphase",
            Cache::BundlePlans => "bundle plans",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Cache::Atoms => "a property or method name's id kept on the Luau string (hit: read off the string; miss: looked up by its bytes, once for each new string the VM makes)",
            Cache::Members => "the Instance API's fast path per class and member (miss: worked out from the reflection)",
            Cache::Resolve => "the engine's member resolution per class and name",
            Cache::Values => "a UDim2, Color3, ... property's converted value, kept while the instance is unchanged",
            Cache::Wrappers => "an instance's Luau userdata (miss: a new one made; eviction: one collected)",
            Cache::Requires => "a ModuleScript's result (miss: the module ran)",
            Cache::Bytecode => "a module file's source and bytecode kept by the process (miss: read and compiled)",
            Cache::Queries => "the Workspace's parts in the spatial queries' tree (miss: the dom's changes taken in, or the Workspace walked)",
            Cache::BundlePlans => "a manifest bundle's converted plan (miss: converted from the bundle file)",
        }
    }
}

/// Hits, misses and evictions
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// The engine's caches and up to this many of a host's
const SLOTS: usize = 32;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
#[allow(clippy::declare_interior_mutable_const)]
const ROW: [AtomicU64; 3] = [ZERO; 3];
/// Counts recorded on worker threads that do not own a caches session (prefetch, bundle decode).
static BG_COUNTS: [[AtomicU64; 3]; SLOTS] = [ROW; SLOTS];
/// A host's own caches after the engine's: (name, what it is)
static NAMED: Mutex<Vec<(&'static str, &'static str)>> = Mutex::new(Vec::new());

/// Whether any caches profile is active in this process
#[inline]
pub fn on() -> bool {
    ACTIVE_SESSIONS.load(Ordering::Relaxed) != 0
}

fn add(index: usize, field: usize, count: u64) {
    if index >= SLOTS {
        return;
    }
    let recorded_locally = LOCAL
        .try_with(|local| {
            let mut local = local.borrow_mut();
            if local.active {
                local.counts[index][field] += count;
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    if !recorded_locally {
        BG_COUNTS[index][field].fetch_add(count, Ordering::Relaxed);
    }
}

#[inline]
pub fn hit(cache: Cache) {
    if on() {
        add(cache as usize, 0, 1);
    }
}

#[inline]
pub fn miss(cache: Cache) {
    if on() {
        add(cache as usize, 1, 1);
    }
}

#[inline]
pub fn evicted(cache: Cache, count: u64) {
    if on() && count > 0 {
        add(cache as usize, 2, count);
    }
}

/// A host's own cache (a layer's: rur's layout), its id for [`named_hit`] and the others; made once per name. Past the
/// slots there are, a cache shares the last one.
pub fn named(name: &'static str, about: &'static str) -> usize {
    let mut named = NAMED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let index = match named.iter().position(|(known, _)| *known == name) {
        Some(index) => index,
        None => {
            named.push((name, about));
            named.len() - 1
        }
    };
    (Cache::ALL.len() + index).min(SLOTS - 1)
}

#[inline]
pub fn named_hit(id: usize) {
    if on() {
        add(id, 0, 1);
    }
}

#[inline]
pub fn named_miss(id: usize) {
    if on() {
        add(id, 1, 1);
    }
}

#[inline]
pub fn named_evicted(id: usize, count: u64) {
    if on() && count > 0 {
        add(id, 2, count);
    }
}

fn bg_counts(index: usize) -> Counts {
    BG_COUNTS.get(index).map_or_else(Counts::default, |slot| Counts { hits: slot[0].load(Ordering::Relaxed), misses: slot[1].load(Ordering::Relaxed), evictions: slot[2].load(Ordering::Relaxed) })
}

fn build_snapshot(slots: &[[u64; 3]; SLOTS]) -> Vec<(&'static str, &'static str, Counts)> {
    let at = |index: usize| Counts { hits: slots[index][0], misses: slots[index][1], evictions: slots[index][2] };
    let mut rows: Vec<(&'static str, &'static str, Counts)> = Cache::ALL.iter().map(|cache| (cache.name(), cache.about(), at(*cache as usize))).collect();
    let named = NAMED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    for (index, (name, about)) in named.into_iter().enumerate() {
        rows.push((name, about, at((Cache::ALL.len() + index).min(SLOTS - 1))));
    }
    rows
}

fn build_facts(slots: &[[u64; 3]; SLOTS], touched: u64) -> Facts {
    let at = |cache: Cache| {
        let row = slots[cache as usize];
        Counts { hits: row[0], misses: row[1], evictions: row[2] }
    };
    let (wrappers, requires, values) = (at(Cache::Wrappers), at(Cache::Requires), at(Cache::Values));
    Facts {
        instances_touched: touched,
        userdata_made: wrappers.misses,
        userdata_collected: wrappers.evictions,
        modules_run: requires.misses,
        requires_kept: requires.hits,
        datatype_reads: values.hits + values.misses,
        datatype_reads_reused: values.hits,
    }
}

fn collect_current() -> ([[u64; 3]; SLOTS], u64) {
    let include_bg = !CONCURRENT.load(Ordering::Acquire);
    LOCAL.with(|local| {
        let local = local.borrow();
        let mut slots = local.counts;
        let mut touched = local.touched.len() as u64;
        if include_bg {
            for (index, slot) in slots.iter_mut().enumerate() {
                let bg = bg_counts(index);
                slot[0] += bg.hits;
                slot[1] += bg.misses;
                slot[2] += bg.evictions;
            }
            let bg_touched = BG_TOUCHED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(bg) = bg_touched.as_ref() {
                touched += bg.iter().filter(|id| !local.touched.contains(id)).count() as u64;
            }
        }
        (slots, touched)
    })
}

/// Every cache's counts (name, what it is, counts), the engine's then the host's.
pub fn snapshot() -> Vec<(&'static str, &'static str, Counts)> {
    let (slots, _) = collect_current();
    build_snapshot(&slots)
}

/// A userdata was made for the instance (index, generation): one the scripts touched
#[inline]
pub fn instance_touched(index: u32, generation: u32) {
    if on() {
        let id = ((generation as u64) << 32) | index as u64;
        let recorded_locally = LOCAL
            .try_with(|local| {
                let mut local = local.borrow_mut();
                if local.active {
                    local.touched.insert(id);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if !recorded_locally {
            let mut touched = BG_TOUCHED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            touched.get_or_insert_with(Default::default).insert(id);
        }
    }
}

/// What the engine's caches say of the game itself: what a client does too
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Instances the scripts touched: a userdata was made for each (a client makes one for an instance a script first reaches)
    pub instances_touched: u64,
    /// The userdata made (an instance's again once the VM collected its first) and collected
    pub userdata_made: u64,
    pub userdata_collected: u64,
    /// ModuleScripts that ran (a require that ran the module), and requires served by the result kept
    pub modules_run: u64,
    pub requires_kept: u64,
    /// Datatype property reads (UDim2, Color3, ...): a client makes a new value for each; ree handed back the value it made
    /// while the property was unchanged for `datatype_reads_reused` of them
    pub datatype_reads: u64,
    pub datatype_reads_reused: u64,
}

/// The game's facts the caches counted (the counts since the profile began)
pub fn facts() -> Facts {
    let (slots, touched) = collect_current();
    build_facts(&slots, touched)
}

fn reset_bg() {
    for slot in &BG_COUNTS {
        for field in slot {
            field.store(0, Ordering::Relaxed);
        }
    }
    *BG_TOUCHED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// Starts counting caches on this thread (a `--profile caches` session's start).
pub(crate) fn begin() {
    let was_active = LOCAL.with(|local| {
        let mut local = local.borrow_mut();
        let prev = local.active;
        local.active = true;
        local.counts = [[0; 3]; SLOTS];
        local.touched.clear();
        prev
    });
    if !was_active {
        let prev = ACTIVE_SESSIONS.fetch_add(1, Ordering::AcqRel);
        if prev == 0 {
            reset_bg();
            CONCURRENT.store(false, Ordering::Release);
        } else {
            CONCURRENT.store(true, Ordering::Release);
        }
    }
}

/// Stops counting caches on this thread and returns this session's snapshot and facts.
pub(crate) fn finish() -> (Vec<(&'static str, &'static str, Counts)>, Facts) {
    let include_bg = !CONCURRENT.load(Ordering::Acquire);
    let (was_active, mut slots, local_touched) = LOCAL.with(|local| {
        let mut local = local.borrow_mut();
        let prev = local.active;
        local.active = false;
        let counts = local.counts;
        local.counts = [[0; 3]; SLOTS];
        let touched = std::mem::take(&mut local.touched);
        (prev, counts, touched)
    });
    if !was_active {
        return (Vec::new(), Facts::default());
    }
    let mut touched = local_touched.len() as u64;
    if include_bg {
        for (index, slot) in slots.iter_mut().enumerate() {
            let bg = bg_counts(index);
            slot[0] += bg.hits;
            slot[1] += bg.misses;
            slot[2] += bg.evictions;
        }
        let bg_touched = BG_TOUCHED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(bg) = bg_touched.as_ref() {
            touched += bg.iter().filter(|id| !local_touched.contains(id)).count() as u64;
        }
    }
    ACTIVE_SESSIONS.fetch_sub(1, Ordering::AcqRel);
    (build_snapshot(&slots), build_facts(&slots, touched))
}

/// Stops counting caches on this thread without building a report (session dropped early).
pub(crate) fn cancel() {
    let was_active = LOCAL
        .try_with(|local| {
            let mut local = local.borrow_mut();
            let prev = local.active;
            local.active = false;
            local.counts = [[0; 3]; SLOTS];
            local.touched.clear();
            prev
        })
        .unwrap_or(false);
    if was_active {
        ACTIVE_SESSIONS.fetch_sub(1, Ordering::AcqRel);
    }
}
