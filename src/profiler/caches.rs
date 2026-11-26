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
        Mutex, MutexGuard,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

/// The `--profile caches` sessions of the process. The count and the flag change together under this one lock, and a
/// session reads the flag and merges the background's counts under it too, so no begin or finish on another thread can
/// come between (two begins that each set the flag in a step of their own once left it unset with two sessions active).
struct Sessions {
    /// Active sessions across all threads
    active: usize,
    /// Set when a session began while another was active, until one begins with none: each session then keeps only its
    /// own thread's counts, so parallel sessions do not mix background-thread counts into one another
    concurrent: bool,
}

static SESSIONS: Mutex<Sessions> = Mutex::new(Sessions { active: 0, concurrent: false });
/// `SESSIONS.active` for the count sites, written only under its lock: off, a count site pays one relaxed load.
static ACTIVE_SESSIONS: AtomicUsize = AtomicUsize::new(0);

fn sessions() -> MutexGuard<'static, Sessions> {
    SESSIONS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Coordinates unit tests that assert exclusive single-session background merging with concurrent
/// `Session` tests in the same library binary (normal sessions hold a shared read lock).
#[cfg(test)]
pub(super) static TEST_SESSION_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

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
            Cache::Values => "a UDim2, Color3, ... property's value: each read makes a userdata of its own, as a client's does (every read a miss)",
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
    let sessions = sessions();
    let include_bg = !sessions.concurrent;
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
    /// Datatype property reads (UDim2, Color3, ...): each makes a new value, as a client's does (`datatype_reads_reused`, the
    /// reads that handed a kept value back, stays 0)
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
        let mut sessions = sessions();
        if sessions.active == 0 {
            reset_bg();
            sessions.concurrent = false;
        } else {
            sessions.concurrent = true;
        }
        sessions.active += 1;
        ACTIVE_SESSIONS.store(sessions.active, Ordering::Release);
    }
}

/// Stops counting caches on this thread and returns this session's snapshot and facts.
pub(crate) fn finish() -> (Vec<(&'static str, &'static str, Counts)>, Facts) {
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
    let mut sessions = sessions();
    if !sessions.concurrent {
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
    sessions.active = sessions.active.saturating_sub(1);
    ACTIVE_SESSIONS.store(sessions.active, Ordering::Release);
    drop(sessions);
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
        let mut sessions = sessions();
        sessions.active = sessions.active.saturating_sub(1);
        ACTIVE_SESSIONS.store(sessions.active, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts_of(rows: &[(&'static str, &'static str, Counts)], cache: Cache) -> Counts {
        rows.iter().find(|(name, _, _)| *name == cache.name()).map(|(_, _, counts)| *counts).expect("cache row")
    }

    #[test]
    fn background_thread_counts_merge_into_single_session() {
        let _exclusive = TEST_SESSION_LOCK.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        // High generation values no concurrent non-profiler test Dom will ever touch.
        let sentinel_gen = u32::MAX;
        let is_sentinel = |id: u64| (id >> 32) as u32 == sentinel_gen;
        let foreign_touched = || {
            BG_TOUCHED
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map_or(0, |bg| bg.iter().filter(|&&id| !is_sentinel(id)).count() as u64)
        };

        begin();
        hit(Cache::Bytecode);
        miss(Cache::Wrappers);
        instance_touched(1, sentinel_gen);
        instance_touched(2, sentinel_gen);

        // Background worker (prefetch / bundle decode) has no caches session of its own.
        std::thread::spawn(move || {
            for _ in 0..51 {
                hit(Cache::Bytecode);
            }
            miss(Cache::Bytecode);
            evicted(Cache::Bytecode, 3);
            miss(Cache::Wrappers);
            // Instance (2, sentinel_gen) duplicates one already touched on the session thread; (3, sentinel_gen) is new.
            instance_touched(2, sentinel_gen);
            instance_touched(3, sentinel_gen);
        })
        .join()
        .unwrap();

        // Confirm both background instances actually landed in BG_TOUCHED (not LOCAL).
        {
            let bg = BG_TOUCHED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let bg = bg.as_ref().expect("background touched set");
            let id = |index: u32| ((sentinel_gen as u64) << 32) | index as u64;
            assert!(bg.contains(&id(2)) && bg.contains(&id(3)) && !bg.contains(&id(1)));
        }

        // Monotonic sandwich bounds: sibling non-profiler tests running in the same --lib binary may add to
        // BG_COUNTS / BG_TOUCHED while ACTIVE_SESSIONS == 1, never subtract. Sandwiching snapshot()/facts()/finish()
        // verifies exact local + bg merging and exact deduplication of (2, sentinel_gen) without flaking.
        let bg_bc_before = bg_counts(Cache::Bytecode as usize);
        let foreign_before_mid = foreign_touched();
        let mid_rows = snapshot();
        let mid_facts = facts();
        let bg_bc_after = bg_counts(Cache::Bytecode as usize);
        let foreign_after_mid = foreign_touched();

        assert!(bg_bc_before.hits >= 51 && bg_bc_before.misses >= 1 && bg_bc_before.evictions >= 3);
        let mid = counts_of(&mid_rows, Cache::Bytecode);
        assert!((1 + bg_bc_before.hits..=1 + bg_bc_after.hits).contains(&mid.hits), "{mid:?} vs {bg_bc_before:?}..={bg_bc_after:?}");
        assert!((bg_bc_before.misses..=bg_bc_after.misses).contains(&mid.misses), "{mid:?} vs {bg_bc_before:?}..={bg_bc_after:?}");
        assert!((bg_bc_before.evictions..=bg_bc_after.evictions).contains(&mid.evictions), "{mid:?} vs {bg_bc_before:?}..={bg_bc_after:?}");
        assert!(
            (3 + foreign_before_mid..=3 + foreign_after_mid).contains(&mid_facts.instances_touched),
            "mid instances_touched={} outside {}..={}",
            mid_facts.instances_touched,
            3 + foreign_before_mid,
            3 + foreign_after_mid
        );

        let bg_bc_before = bg_counts(Cache::Bytecode as usize);
        let bg_wr_before = bg_counts(Cache::Wrappers as usize);
        let foreign_before_done = foreign_touched();
        let (rows, done_facts) = finish();
        let bg_bc_after = bg_counts(Cache::Bytecode as usize);
        let bg_wr_after = bg_counts(Cache::Wrappers as usize);
        let foreign_after_done = foreign_touched();

        let done_bc = counts_of(&rows, Cache::Bytecode);
        assert!((1 + bg_bc_before.hits..=1 + bg_bc_after.hits).contains(&done_bc.hits), "{done_bc:?} vs {bg_bc_before:?}..={bg_bc_after:?}");
        assert!((bg_bc_before.misses..=bg_bc_after.misses).contains(&done_bc.misses), "{done_bc:?} vs {bg_bc_before:?}..={bg_bc_after:?}");
        assert!((bg_bc_before.evictions..=bg_bc_after.evictions).contains(&done_bc.evictions), "{done_bc:?} vs {bg_bc_before:?}..={bg_bc_after:?}");
        let done_wr = counts_of(&rows, Cache::Wrappers);
        assert!(bg_wr_before.misses >= 1);
        assert!((1 + bg_wr_before.misses..=1 + bg_wr_after.misses).contains(&done_wr.misses), "{done_wr:?} vs {bg_wr_before:?}..={bg_wr_after:?}");
        assert_eq!(done_facts.userdata_made, done_wr.misses);
        assert!(
            (3 + foreign_before_done..=3 + foreign_after_done).contains(&done_facts.instances_touched),
            "done instances_touched={} outside {}..={}",
            done_facts.instances_touched,
            3 + foreign_before_done,
            3 + foreign_after_done
        );
    }

    #[test]
    fn overlapping_sessions_keep_their_own_counts() {
        let _exclusive = TEST_SESSION_LOCK.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let both_begun = std::sync::Arc::new(std::sync::Barrier::new(2));
        let bg_done = std::sync::Arc::new(std::sync::Barrier::new(2));

        let (b1, d1) = (both_begun.clone(), bg_done.clone());
        let first = std::thread::spawn(move || {
            begin();
            b1.wait();
            for _ in 0..7 {
                hit(Cache::Atoms);
            }
            miss(Cache::Requires);
            instance_touched(10, 1);
            // Record background-thread counts while both sessions are active; neither session may pull them in.
            std::thread::spawn(|| {
                for _ in 0..99 {
                    hit(Cache::Atoms);
                    hit(Cache::Bytecode);
                }
                instance_touched(99, 9);
            })
            .join()
            .unwrap();
            d1.wait();
            finish()
        });

        let (b2, d2) = (both_begun.clone(), bg_done.clone());
        let second = std::thread::spawn(move || {
            begin();
            b2.wait();
            for _ in 0..13 {
                hit(Cache::Bytecode);
            }
            evicted(Cache::Queries, 4);
            instance_touched(20, 2);
            instance_touched(21, 2);
            d2.wait();
            finish()
        });

        let (rows1, facts1) = first.join().unwrap();
        let (rows2, facts2) = second.join().unwrap();

        assert_eq!(counts_of(&rows1, Cache::Atoms), Counts { hits: 7, misses: 0, evictions: 0 });
        assert_eq!(counts_of(&rows1, Cache::Bytecode), Counts::default());
        assert_eq!(counts_of(&rows1, Cache::Requires), Counts { hits: 0, misses: 1, evictions: 0 });
        assert_eq!(facts1.instances_touched, 1);

        assert_eq!(counts_of(&rows2, Cache::Bytecode), Counts { hits: 13, misses: 0, evictions: 0 });
        assert_eq!(counts_of(&rows2, Cache::Atoms), Counts::default());
        assert_eq!(counts_of(&rows2, Cache::Queries), Counts { hits: 0, misses: 0, evictions: 4 });
        assert_eq!(facts2.instances_touched, 2);
    }

    /// Two sessions that begin at the same moment, many times over: each round both begin, a thread with no session counts
    /// hits while both are on, and each session must read only its own. Their begins once raced (the count went up in one
    /// step and the overlap flag was set in another, so the first session's `false` could land after the second's `true`)
    /// and both sessions then took the background's hits as their own.
    #[test]
    fn sessions_beginning_together_keep_their_own_counts() {
        use std::sync::{Arc, Barrier, atomic::AtomicUsize, mpsc};
        let _exclusive = TEST_SESSION_LOCK.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        const ROUNDS: usize = 2000;
        const BACKGROUND_HITS: u64 = 5;
        // A spinning start, not a Barrier: a Barrier's last arrival runs on while it wakes the others, microseconds later,
        // which is long enough for one begin to finish before the other starts.
        let start = Arc::new(AtomicUsize::new(0));
        let begun = Arc::new(Barrier::new(3));
        let counted = Arc::new(Barrier::new(3));
        let (sender, results) = mpsc::channel();
        let workers: Vec<_> = [7u64, 13]
            .into_iter()
            .map(|own| {
                let (start, begun, counted, sender) = (start.clone(), begun.clone(), counted.clone(), sender.clone());
                std::thread::spawn(move || {
                    for round in 1..=ROUNDS {
                        while start.load(Ordering::Acquire) < round {
                            std::hint::spin_loop();
                        }
                        begin();
                        for _ in 0..own {
                            hit(Cache::Atoms);
                        }
                        begun.wait();
                        counted.wait();
                        let (rows, _) = finish();
                        sender.send((round, own, counts_of(&rows, Cache::Atoms).hits)).unwrap();
                    }
                })
            })
            .collect();
        drop(sender);
        let mut wrong = Vec::new();
        for round in 1..=ROUNDS {
            start.store(round, Ordering::Release);
            begun.wait();
            for _ in 0..BACKGROUND_HITS {
                hit(Cache::Atoms);
            }
            counted.wait();
            for _ in 0..2 {
                let (round, own, read) = results.recv().unwrap();
                if read != own {
                    wrong.push((round, own, read));
                }
            }
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(wrong.is_empty(), "{} of {} sessions read hits not their own (round, own, read): {:?}", wrong.len(), 2 * ROUNDS, &wrong[..wrong.len().min(8)]);
    }
}
