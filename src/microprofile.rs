//! Timeline recorder for the native app's MicroProfiler view (profiler.rs): named, nested scopes on
//! every thread (the app's main thread, the engine thread, the raster pool, the asset threads) on
//! one clock, with frame markers, like Roblox's MicroProfiler.
//!
//! [`scope`] returns a guard that records `[start, end)` when dropped. Scope names are static
//! strings; dynamic ones (Luau script labels) go through [`intern`] once. Each thread appends to
//! its own buffer (an uncontended lock per closed scope, taken by the viewer only when it copies a
//! window out), so a scope costs two clock reads and a push. With recording off (the view hidden
//! or paused) a scope is one relaxed load.
//!
//! Buffers are bounded (the oldest events drop first), as are frame markers.

use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Events kept per thread (about 20 bytes each)
const THREAD_CAPACITY: usize = 400_000;
/// Frame markers kept
const FRAME_CAPACITY: usize = 20_000;

/// One closed scope. Times are nanoseconds on [`now`]'s clock.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub start: u64,
    pub end: u64,
    pub name: &'static str,
    /// Nesting depth on its thread (0: outermost)
    pub depth: u16,
    /// A number shown with the scope (pixels rastered, nodes drawn, ...), 0 when unused
    pub detail: u64,
}

pub struct ThreadLog {
    pub name: String,
    /// Registration order (lanes are shown in it)
    pub order: u64,
    events: Mutex<VecDeque<Event>>,
}

impl ThreadLog {
    fn push(&self, event: Event) {
        let mut events = self.events.lock().unwrap_or_else(|poison| poison.into_inner());
        if events.len() >= THREAD_CAPACITY {
            events.pop_front();
        }
        events.push_back(event);
    }
}

struct Registry {
    threads: Mutex<Vec<Arc<ThreadLog>>>,
    frames: Mutex<VecDeque<Frame>>,
    next_order: AtomicU64,
}

/// A frame marker: the main thread's update started at `start` (ns) and ended at `end`
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub id: u64,
    pub start: u64,
    pub end: u64,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry {
        threads: Mutex::new(Vec::new()),
        frames: Mutex::new(VecDeque::new()),
        next_order: AtomicU64::new(0),
    })
}

struct Local {
    log: Arc<ThreadLog>,
    depth: u16,
}

thread_local! {
    static LOCAL: RefCell<Option<Local>> = const { RefCell::new(None) };
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Nanoseconds since the recorder's epoch
pub fn now() -> u64 {
    epoch().elapsed().as_nanos() as u64
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    epoch();
    ENABLED.store(on, Ordering::Relaxed);
}

/// Names this thread's lane (threads that record without it get their OS thread name)
pub fn register_thread(name: &str) {
    LOCAL.with(|local| {
        let mut local = local.borrow_mut();
        if let Some(existing) = local.as_ref() {
            if existing.log.name == name {
                return;
            }
        }
        *local = Some(Local { log: new_log(name.to_string()), depth: 0 });
    });
}

fn new_log(name: String) -> Arc<ThreadLog> {
    let registry = registry();
    let log = Arc::new(ThreadLog {
        name,
        order: registry.next_order.fetch_add(1, Ordering::Relaxed),
        events: Mutex::new(VecDeque::new()),
    });
    registry.threads.lock().unwrap_or_else(|p| p.into_inner()).push(log.clone());
    log
}

fn with_local<R>(f: impl FnOnce(&mut Local) -> R) -> R {
    LOCAL.with(|local| {
        let mut local = local.borrow_mut();
        let local = local.get_or_insert_with(|| {
            let name = std::thread::current().name().map(str::to_owned).unwrap_or_else(|| {
                format!("Thread {:?}", std::thread::current().id())
            });
            Local { log: new_log(name), depth: 0 }
        });
        f(local)
    })
}

/// Records `name` from now until the guard drops
#[must_use = "the scope ends when the guard drops"]
pub fn scope(name: &'static str) -> Scope {
    scope_detail(name, 0)
}

#[must_use = "the scope ends when the guard drops"]
pub fn scope_detail(name: &'static str, detail: u64) -> Scope {
    if !enabled() {
        return Scope { open: None };
    }
    let depth = with_local(|local| {
        local.depth += 1;
        local.depth - 1
    });
    Scope { open: Some(Open { name, start: now(), depth, detail }) }
}

struct Open {
    name: &'static str,
    start: u64,
    depth: u16,
    detail: u64,
}

pub struct Scope {
    open: Option<Open>,
}

impl Scope {
    /// Sets the number shown with the scope once its work is known
    pub fn set_detail(&mut self, detail: u64) {
        if let Some(open) = &mut self.open {
            open.detail = detail;
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let Some(open) = self.open.take() else {
            return;
        };
        let end = now();
        with_local(|local| {
            local.depth = local.depth.saturating_sub(1);
            local.log.push(Event { start: open.start, end, name: open.name, depth: open.depth, detail: open.detail });
        });
    }
}

/// A scope measured elsewhere (its start and end on [`now`]'s clock), at this thread's depth
pub fn record(name: &'static str, start: u64, end: u64, detail: u64) {
    if !enabled() {
        return;
    }
    with_local(|local| local.log.push(Event { start, end, name, depth: local.depth, detail }));
}

/// Enter / leave pairs for code that cannot hold a guard (the engine's profile buckets)
pub fn enter(name: &'static str) -> Option<(u64, u16)> {
    if !enabled() {
        return None;
    }
    let depth = with_local(|local| {
        local.depth += 1;
        local.depth - 1
    });
    let _ = name;
    Some((now(), depth))
}

pub fn leave(name: &'static str, opened: Option<(u64, u16)>) {
    let Some((start, depth)) = opened else {
        return;
    };
    let end = now();
    with_local(|local| {
        local.depth = local.depth.saturating_sub(1);
        local.log.push(Event { start, end, name, depth, detail: 0 });
    });
}

/// A static name for a dynamic string (script labels); the same text returns the same name.
/// Bounded: past 16384 names, new text shares one overflow name.
pub fn intern(text: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    if let Some(name) = names.get(text) {
        return name;
    }
    if names.len() >= 16_384 {
        return "(more names)";
    }
    let name: &'static str = Box::leak(text.to_owned().into_boxed_str());
    names.insert(name);
    name
}

/// Marks one frame of the app's main loop
pub fn frame(id: u64, start: u64, end: u64) {
    if !enabled() {
        return;
    }
    let mut frames = registry().frames.lock().unwrap_or_else(|p| p.into_inner());
    if frames.len() >= FRAME_CAPACITY {
        frames.pop_front();
    }
    frames.push_back(Frame { id, start, end });
}

/// The frame markers kept
pub fn frames() -> Vec<Frame> {
    registry().frames.lock().unwrap_or_else(|p| p.into_inner()).iter().copied().collect()
}

/// One lane of a copied window
pub struct Lane {
    pub name: String,
    pub order: u64,
    pub events: Vec<Event>,
}

/// Every thread's events overlapping `[from, to)`, lanes in registration order
pub fn window(from: u64, to: u64) -> Vec<Lane> {
    let threads = registry().threads.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let mut lanes: Vec<Lane> = threads
        .iter()
        .map(|log| {
            let events = log.events.lock().unwrap_or_else(|p| p.into_inner());
            // Events are pushed when they close: ordered by end time on their thread
            let first = events.partition_point(|e| e.end < from);
            Lane {
                name: log.name.clone(),
                order: log.order,
                events: events.range(first..).filter(|e| e.start < to).copied().collect(),
            }
        })
        .collect();
    lanes.sort_by_key(|lane| lane.order);
    lanes
}

/// Drops every recorded event and frame marker
pub fn clear() {
    let registry = registry();
    for log in registry.threads.lock().unwrap_or_else(|p| p.into_inner()).iter() {
        log.events.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
    registry.frames.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_scopes_record_depth_and_order() {
        set_enabled(true);
        register_thread("test-lane");
        let start = now();
        {
            let _outer = scope("outer");
            let _inner = scope_detail("inner", 7);
        }
        let lane = window(start, now() + 1).into_iter().find(|lane| lane.name == "test-lane").unwrap();
        let names: Vec<_> = lane.events.iter().map(|e| (e.name, e.depth, e.detail)).collect();
        assert_eq!(names, [("inner", 1, 7), ("outer", 0, 0)]);
        assert!(lane.events[1].start <= lane.events[0].start && lane.events[0].end <= lane.events[1].end);
        assert_eq!(intern("Script x"), intern(&String::from("Script x")));
    }
}
