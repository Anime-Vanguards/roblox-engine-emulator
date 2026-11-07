//! `Context.Census(Depth?)`: what a long run accumulates, counted where Studio cannot cheaply: live Luau threads
//! by the line that made them, connected handlers by the line that connected them, handler runs, pending
//! task.wait / task.delay / deferred resumptions and live instances by class and top-level parent.
//!
//! The call installs the hooks and returns `Take(Collect?)`, which returns one census of the moment as a JSON
//! string; a harness prints it every N virtual seconds and a reader looks for what grows with time and never
//! falls. Off by default: until the first call the Connect path and handler runs pay one thread-local load and
//! the VM has no thread callback. A site is the innermost `Depth` (default 2) Luau frames of the call, innermost
//! first, as `Chunk:line < Chunk:line` (C functions and the engine's own Luau skipped); "(engine)" when no
//! script frame made it.
//!
//! - `threads`: every thread the VM makes from the call on (coroutine.create / wrap, task.spawn / defer / delay
//!   of a function, each handler run's thread, filed under its connection's site) is followed through Luau's
//!   `userthread` callback, which also drops it when the collector frees it. A census lists the live ones as
//!   `[site, fn, at, status, held, count]`: `fn` is where the thread's function is defined, `at` the line it is
//!   suspended on, `status` coroutine.status's word, `held` what in the engine will resume it ("timer",
//!   "legacy" (wait / spawn / delay), "delay", "deferred", "wait" (RBXScriptSignal:Wait), "" for nothing: a
//!   script holds it, or a closure that would resume it, such as a connection's). Threads made before the call
//!   are not followed.
//! - `connections`: live connections as `[site, fn, signal, class of the signal's instance, count]`; one made
//!   before the call has site "(before census)". `runs`: handler runs since the previous Take as `[site, fn,
//!   runs]`, `made`: threads made since then as `[site, count]`, with `frames` and `seconds` between.
//! - `timers`: pending resumptions as `[kind, at or fn, count, latest due in seconds]`.
//! - `instances`: live instances as `[class, root, count]`, root being the top-level parent ("Workspace/Map";
//!   under Players "PlayerGui/HUD"; "(nil) Model Name" for a tree outside the DataModel that something holds,
//!   "(nil) Model Name (pinned)" when the engine holds it: a tween's target, a playing animation).
//!
//! With Collect (the default) a full Luau collection and the engine's instance collection run first, so what is
//! counted is reachable; weak tables lose their dead entries then rather than later, so a run with a census is
//! not byte-identical to one without.

use std::{
    cell::{Cell, RefCell},
    ffi::{CStr, c_int},
    rc::Rc,
};

use mlua::{ffi, prelude::*};
use rustc_hash::FxHashMap;
use serde_json::{Value as Json, json};

use crate::{Engine, signal::ConnectionId};

unsafe extern "C-unwind" {
    fn lua_costatus(state: *mut ffi::lua_State, co: *mut ffi::lua_State) -> c_int;
}

const ENGINE: u32 = 0;
const BEFORE: u32 = 1;

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static LOG: RefCell<Option<Log>> = const { RefCell::new(None) };
}

struct Log {
    /// The engine and VM followed (a service worker runs one engine after another on its thread)
    engine: *const Engine,
    main: *mut ffi::lua_State,
    depth: usize,
    /// Interned sites and function lines: 0 "(engine)", 1 "(before census)"
    names: Vec<Box<str>>,
    ids: FxHashMap<Box<str>, u32>,
    /// Live followed threads -> the site that made them
    threads: FxHashMap<usize, u32>,
    /// Connection slot -> (connect site, handler fn), while connected
    connections: FxHashMap<u32, (u32, u32)>,
    /// Since the last Take: threads made by site, handler runs by (site, fn)
    made: FxHashMap<u32, u64>,
    runs: FxHashMap<(u32, u32), u64>,
    since: (f64, u64),
}

impl Log {
    fn intern(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(text.into());
        self.ids.insert(text.into(), id);
        id
    }

    fn name(&self, id: u32) -> &str {
        &self.names[id as usize]
    }
}

/// The innermost `depth` script frames of `state` from `level` on, innermost first (no allocation in the VM).
unsafe fn frames(state: *mut ffi::lua_State, depth: usize) -> String {
    let mut out = String::new();
    let mut taken = 0;
    unsafe {
        let mut debug: ffi::lua_Debug = std::mem::zeroed();
        let mut level: c_int = 0;
        while level < 80 && taken < depth && ffi::lua_getinfo(state, level, c"sl".as_ptr(), &mut debug) != 0 {
            level += 1;
            if let Some(source) = script_source(&debug) {
                if debug.currentline < 0 {
                    continue;
                }
                if taken > 0 {
                    out.push_str(" < ");
                }
                out.push_str(&format!("{source}:{}", debug.currentline));
                taken += 1;
            }
        }
    }
    out
}

/// A frame's chunk, unless it is a C function or the engine's own Luau (as trace.rs skips them).
unsafe fn script_source(debug: &ffi::lua_Debug) -> Option<String> {
    if debug.short_src.is_null() {
        return None;
    }
    let source = unsafe { CStr::from_ptr(debug.short_src) }.to_string_lossy();
    if source == "[C]" || source.starts_with("ree.") || source.starts_with("rur.") || source.contains("prelude") {
        return None;
    }
    Some(source.strip_prefix("[string \"").and_then(|inner| inner.strip_suffix("\"]")).unwrap_or(&source).to_string())
}

/// Where the function a thread runs is defined: its outermost script frame, else (not started) the function on
/// its stack.
unsafe fn thread_function(co: *mut ffi::lua_State) -> String {
    unsafe {
        let mut debug: ffi::lua_Debug = std::mem::zeroed();
        let mut found = None;
        let mut level: c_int = 0;
        while level < 200 && ffi::lua_getinfo(co, level, c"s".as_ptr(), &mut debug) != 0 {
            level += 1;
            if let Some(source) = script_source(&debug) {
                found = Some(format!("{source}:{}", debug.linedefined));
            }
        }
        if found.is_none() && level == 0 {
            let top = ffi::lua_gettop(co);
            if top > 0 && ffi::lua_getinfo(co, -top, c"s".as_ptr(), &mut debug) != 0 {
                found = script_source(&debug).map(|source| format!("{source}:{}", debug.linedefined));
            }
        }
        found.unwrap_or_default()
    }
}

/// Luau's `userthread` callback: a thread made (`parent` set) or about to be freed. A profile's timeline counts the live
/// threads through the same callback (profiler.rs).
pub(crate) unsafe extern "C-unwind" fn userthread(parent: *mut ffi::lua_State, child: *mut ffi::lua_State) {
    crate::profiler::thread_event(!parent.is_null());
    LOG.with(|log| {
        let Ok(mut log) = log.try_borrow_mut() else { return };
        let Some(log) = log.as_mut() else { return };
        if parent.is_null() {
            log.threads.remove(&(child as usize));
            return;
        }
        if unsafe { ffi::lua_mainthread(child) } != log.main {
            return;
        }
        let text = unsafe { frames(parent, log.depth) };
        let site = if text.is_empty() { ENGINE } else { log.intern(&text) };
        log.threads.insert(child as usize, site);
        *log.made.entry(site).or_default() += 1;
    });
}

/// Whether a census was installed on this thread (its callback stays when a profile ends)
pub(crate) fn installed() -> bool {
    ACTIVE.with(Cell::get)
}

/// Whether this engine's census is on (callers skip the work of [`connected`] when it is not).
#[inline]
pub fn active(engine: &Engine) -> bool {
    ACTIVE.with(Cell::get) && LOG.with(|log| log.borrow().as_ref().is_some_and(|log| std::ptr::eq(log.engine, engine)))
}

/// A script's Connect / Once / WaitForChild made `connection` with `callback`, while [`active`] (types.rs, instance.rs).
pub fn connected(lua: &Lua, connection: ConnectionId, callback: &LuaFunction) {
    if connection == ConnectionId::DISCONNECTED {
        return;
    }
    let state = lua.current_thread().state();
    let info = callback.info();
    let source = info.short_src.unwrap_or_default();
    let source = source.strip_prefix("[string \"").and_then(|inner| inner.strip_suffix("\"]")).unwrap_or(&source);
    let function = format!("{source}:{}", info.line_defined.unwrap_or(0));
    LOG.with(|log| {
        let mut log = log.borrow_mut();
        let Some(log) = log.as_mut() else { return };
        let text = unsafe { frames(state, log.depth) };
        let site = if text.is_empty() { ENGINE } else { log.intern(&text) };
        let function = log.intern(&function);
        log.connections.insert(connection.slot(), (site, function));
    });
}

/// A connection slot was let go (signal.rs): a later connection there is a new one.
#[inline]
pub fn released(slot: u32) {
    if ACTIVE.with(Cell::get) {
        LOG.with(|log| {
            if let Ok(mut log) = log.try_borrow_mut() {
                if let Some(log) = log.as_mut() {
                    log.connections.remove(&slot);
                }
            }
        });
    }
}

/// The site a handler run of `connection` is filed under, taken before the run (a :Once() connection is let go
/// as it starts).
#[inline]
pub fn handler_site(engine: &Engine, connection: ConnectionId) -> Option<(u32, u32)> {
    if !active(engine) {
        return None;
    }
    LOG.with(|log| log.borrow().as_ref().map(|log| log.connections.get(&connection.slot()).copied().unwrap_or((BEFORE, BEFORE))))
}

/// A handler of `site` runs on `thread` (scheduler.rs run_job).
pub fn handler_ran(site: (u32, u32), thread: &LuaThread) {
    LOG.with(|log| {
        let mut log = log.borrow_mut();
        let Some(log) = log.as_mut() else { return };
        *log.runs.entry(site).or_default() += 1;
        // (the thread was made by the engine; it belongs to the connection)
        let key = thread.state() as usize;
        if let Some(made) = log.threads.get_mut(&key) {
            if *made == ENGINE && site.0 != ENGINE {
                *made = site.0;
                if let Some(count) = log.made.get_mut(&ENGINE) {
                    *count = count.saturating_sub(1);
                }
                *log.made.entry(site.0).or_default() += 1;
            }
        }
    });
}

/// Installs the census for this engine's VM and returns `Take`.
pub fn install(lua: &Lua, engine: &Rc<Engine>, depth: usize) -> LuaResult<LuaFunction> {
    let main = unsafe { ffi::lua_mainthread(lua.current_thread().state()) };
    let since = {
        let state = engine.state.borrow();
        (state.sched.now, state.sched.frame)
    };
    let mut log = Log {
        engine: Rc::as_ptr(engine),
        main,
        depth: depth.max(1),
        names: Vec::new(),
        ids: FxHashMap::default(),
        threads: FxHashMap::default(),
        connections: FxHashMap::default(),
        made: FxHashMap::default(),
        runs: FxHashMap::default(),
        since,
    };
    log.intern("(engine)");
    log.intern("(before census)");
    LOG.with(|cell| *cell.borrow_mut() = Some(log));
    ACTIVE.with(|active| active.set(true));
    unsafe { (*ffi::lua_callbacks(main)).userthread = Some(userthread) };
    let engine = Rc::downgrade(engine);
    lua.create_function(move |lua, collect: Option<bool>| {
        let Some(engine) = engine.upgrade() else { return Ok(String::new()) };
        if collect.unwrap_or(true) {
            lua.gc_collect()?;
            crate::lua::raw::collect(lua, &engine)?;
            lua.gc_collect()?;
        }
        let current = lua.current_thread().state();
        let census = take(&engine, current, lua.used_memory());
        Ok(serde_json::to_string(&census).unwrap_or_default())
    })
}

fn bump<K: std::hash::Hash + Eq>(map: &mut FxHashMap<K, u64>, key: K) {
    *map.entry(key).or_default() += 1;
}

/// One census. Reads the VM only through calls that allocate nothing in it, so no collection can free a
/// followed thread meanwhile.
fn take(engine: &Engine, current: *mut ffi::lua_State, heap: usize) -> Json {
    let state = engine.state.borrow();
    let reflection = &engine.reflection;

    // What the engine will resume, by thread
    let mut held: FxHashMap<usize, &'static str> = FxHashMap::default();
    let mut timers: FxHashMap<(&'static str, usize), (u64, f64)> = FxHashMap::default();
    let mut pending: Vec<(&'static str, usize, f64)> = Vec::new();
    state.sched.visit_waiting(&mut |thread, kind, due| {
        let key = thread.state() as usize;
        held.insert(key, kind);
        pending.push((kind, key, due));
    });
    state.signals.visit_waiting(&mut |thread| {
        held.insert(thread.state() as usize, "wait");
    });

    // Each live signal's instance class
    let world = engine.world();
    let mut owner: FxHashMap<u32, u32> = FxHashMap::default();
    let mut instances: FxHashMap<(u32, Box<str>), u64> = FxHashMap::default();
    let dom = &state.dom;
    for index in 0..dom.slot_count() as u32 {
        let Some(id) = dom.id_at_slot(index) else { continue };
        let instance = dom.at(id);
        if let Some(signals) = &instance.signals {
            for signal in signals.all() {
                owner.insert(signal, instance.class);
            }
        }
        // (the top-level parent's key, cached by the instance just below the top)
        let mut path = vec![id];
        let mut at = id;
        while let Some(parent) = dom.at(at).parent {
            path.push(parent);
            at = parent;
        }
        let root = if at == world.game {
            let names: Vec<String> = path.iter().rev().skip(1).take(4).map(|&i| dom.at(i).name.to_str_lossy().into_owned()).collect();
            match names.first().map(String::as_str) {
                None => "DataModel".to_string(),
                Some("Players") if names.len() >= 3 => names[2..].join("/"),
                Some(_) => names[..names.len().min(2)].join("/"),
            }
        } else {
            let top = dom.at(at);
            // (pinned: the engine holds it, a tween's target say, rather than a script)
            let pinned = if dom.is_pinned(at) { " (pinned)" } else { "" };
            format!("(nil) {} {}{pinned}", reflection.class(top.class).name, top.name.to_str_lossy())
        };
        *instances.entry((instance.class, root.into_boxed_str())).or_default() += 1;
    }

    let now = state.sched.now;
    let frame = state.sched.frame;
    LOG.with(|cell| {
        let mut cell = cell.borrow_mut();
        let Some(log) = cell.as_mut() else { return json!({}) };

        // Threads
        let mut threads: FxHashMap<(u32, String, String, &'static str, &'static str), u64> = FxHashMap::default();
        let followed: Vec<(usize, u32)> = log.threads.iter().map(|(&k, &v)| (k, v)).collect();
        let mut fn_of: FxHashMap<usize, String> = FxHashMap::default();
        for (key, site) in followed {
            let co = key as *mut ffi::lua_State;
            let status = match unsafe { lua_costatus(current, co) } {
                0 => "running",
                1 => "suspended",
                2 => "normal",
                3 => "dead",
                _ => "error",
            };
            let function = unsafe { thread_function(co) };
            let at = if status == "suspended" { unsafe { frames(co, 1) } } else { String::new() };
            fn_of.insert(key, if at.is_empty() { function.clone() } else { at.clone() });
            bump(&mut threads, (site, function, at, status, held.get(&key).copied().unwrap_or("")));
        }
        for (kind, key, due) in &pending {
            let entry = timers.entry((kind, *key)).or_default();
            entry.0 += 1;
            entry.1 = entry.1.max(due - now);
        }
        let mut timer_rows: FxHashMap<(&'static str, String), (u64, f64)> = FxHashMap::default();
        for ((kind, key), (count, ahead)) in timers {
            let place = fn_of.get(&key).cloned().unwrap_or_else(|| {
                let co = key as *mut ffi::lua_State;
                let at = unsafe { frames(co, 1) };
                if at.is_empty() { unsafe { thread_function(co) } } else { at }
            });
            let row = timer_rows.entry((kind, place)).or_default();
            row.0 += count;
            row.1 = row.1.max(ahead);
        }

        // Connections
        let mut connections: FxHashMap<(u32, u32, String, String), u64> = FxHashMap::default();
        state.signals.visit_connections(&mut |signal, connection| {
            let (site, function) = log.connections.get(&connection.slot()).copied().unwrap_or((BEFORE, BEFORE));
            let name = state.signals.get(signal).name.to_str_lossy().into_owned();
            let class = owner.get(&signal).map(|&class| reflection.class(class).name.to_string()).unwrap_or_default();
            bump(&mut connections, (site, function, name, class));
        });

        let name = |id: u32| log.name(id).to_string();
        let mut thread_rows: Vec<Json> = threads
            .into_iter()
            .map(|((site, function, at, status, held), count)| json!([name(site), function, at, status, held, count]))
            .collect();
        let mut connection_rows: Vec<Json> = connections
            .into_iter()
            .map(|((site, function, signal, class), count)| json!([name(site), name(function), signal, class, count]))
            .collect();
        let mut run_rows: Vec<Json> =
            log.runs.iter().map(|(&(site, function), &runs)| json!([name(site), name(function), runs])).collect();
        let mut made_rows: Vec<Json> = log.made.iter().filter(|(_, n)| **n > 0).map(|(&site, &n)| json!([name(site), n])).collect();
        let mut timer_rows: Vec<Json> =
            timer_rows.into_iter().map(|((kind, place), (count, ahead))| json!([kind, place, count, ahead])).collect();
        let mut instance_rows: Vec<Json> = instances
            .into_iter()
            .map(|((class, root), count)| json!([reflection.class(class).name.to_string(), root.to_string(), count]))
            .collect();
        // (stable output: biggest first, then by text)
        for rows in [&mut thread_rows, &mut connection_rows, &mut run_rows, &mut made_rows, &mut timer_rows, &mut instance_rows] {
            rows.sort_by(|a, b| {
                let count = |row: &Json| row.as_array().and_then(|r| r.last()).and_then(Json::as_f64).unwrap_or(0.0);
                count(b).partial_cmp(&count(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.to_string().cmp(&b.to_string()))
            });
        }
        let (live_signals, live_connections) = state.signals.counts();
        let census = json!({
            "time": now,
            "frame": frame,
            "seconds": now - log.since.0,
            "frames": frame - log.since.1,
            "lua_heap_kb": heap / 1024,
            "instances_live": dom.live_count(),
            "signals_live": live_signals,
            "connections_live": live_connections,
            "threads_followed": log.threads.len(),
            "threads": thread_rows,
            "connections": connection_rows,
            "runs": run_rows,
            "made": made_rows,
            "timers": timer_rows,
            "deferred": state.sched.deferred_len(),
            "instances": instance_rows,
        });
        log.runs.clear();
        log.made.clear();
        log.since = (now, frame);
        census
    })
}
