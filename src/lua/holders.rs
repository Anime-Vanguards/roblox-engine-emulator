//! `Context.Holders(Filters, Examples?)`: what keeps orphan instances alive, named by the way to them.
//!
//! The census (census.rs) counts trees outside the DataModel that something holds; this says what. After a full
//! Luau collection and the instance collection, every live tree outside the DataModel whose root reads
//! "Class Name" starting with one of `Filters` ("Part HumanoidRootPart", "ImageLabel Health") is a target, and
//! each target root is explained by the first of:
//!
//! - `pinned`: the engine pins an instance of it (a playing tween's target, a playing animation);
//! - `property`: another live instance's property references an instance of it (a Weld's Part0, an Adornee, a
//!   Tween's Instance), as `lua::raw::collect` keeps such trees; the holder is named;
//! - `queued`: queued work (a deferred event's or a timer's arguments) references it;
//! - `lua`: a Luau value reaches its userdata. A breadth-first walk from labelled seeds (module results, connected
//!   handlers, threads waiting on signals, threads the scheduler holds, the running frames, the globals and the
//!   registry, which holds mlua's ref thread and so everything Rust keeps) through table keys and values (weak
//!   halves skipped), metatables, closure upvalues and environments, and whole thread stacks finds the shortest
//!   way to it; the way is printed as `seed > step > step`, and ways that differ only by numbers are counted
//!   together.
//!
//! Anything left is `unexplained` (a holder this walk cannot see). The walk stops the collector while it runs and
//! keeps what it visits in a table, so nothing it reads is freed under it.

use std::{
    ffi::{CStr, c_int},
    rc::Rc,
};

use mlua::{ffi, prelude::*};
use rustc_hash::FxHashMap;
use serde_json::{Value as Json, json};

use super::raw;
use crate::{Engine, InstanceId, dom::flags};

const REGISTRY: c_int = ffi::LUA_REGISTRYINDEX;

/// lua_State fields read to walk a suspended thread's whole stack (lstate.h: 7 header bytes, then `top`, `base`,
/// `global`, `ci`, `stack_last`, `stack`); checked against the running thread before use.
const TOP_OFFSET: usize = 8;
const BASE_OFFSET: usize = 16;
const STACK_LAST_OFFSET: usize = 40;
const STACK_OFFSET: usize = 48;
const TVALUE_SIZE: usize = 16;

/// Ways printed per group, and full examples per way
const WAYS: usize = 12;

struct Node {
    parent: u32,
    label: String,
}

struct Walk<'a> {
    state: *mut ffi::lua_State,
    /// The table at this absolute index keeps every visited value
    queue: c_int,
    nodes: Vec<Node>,
    seen: FxHashMap<usize, u32>,
    /// Instance -> its target root
    targets: &'a FxHashMap<InstanceId, InstanceId>,
    /// Target root -> (node holding it, the step from that node, the instance reached)
    hits: FxHashMap<InstanceId, (u32, String, InstanceId)>,
    /// Whether whole stacks of suspended threads can be read (the layout check passed)
    raw_stacks: bool,
    registry_names: FxHashMap<i64, &'static str>,
}

unsafe fn read_ptr(thread: *mut ffi::lua_State, offset: usize) -> usize {
    unsafe { std::ptr::read_unaligned((thread as *const u8).add(offset) as *const usize) }
}

/// Whether the lua_State layout above holds for `state` (its top and base agree with lua_gettop).
unsafe fn layout_holds(state: *mut ffi::lua_State) -> bool {
    unsafe {
        let top = read_ptr(state, TOP_OFFSET);
        let base = read_ptr(state, BASE_OFFSET);
        let stack = read_ptr(state, STACK_OFFSET);
        let last = read_ptr(state, STACK_LAST_OFFSET);
        top >= base
            && (top - base) / TVALUE_SIZE == ffi::lua_gettop(state) as usize
            && (top - base) % TVALUE_SIZE == 0
            && stack <= base
            && (base - stack) % TVALUE_SIZE == 0
            && last >= top
    }
}

/// Whether a value of this type can hold others or be a target
fn collectable(kind: c_int) -> bool {
    matches!(kind, ffi::LUA_TTABLE | ffi::LUA_TFUNCTION | ffi::LUA_TTHREAD | ffi::LUA_TUSERDATA)
}

fn source_of(debug: &ffi::lua_Debug) -> String {
    if debug.short_src.is_null() {
        return "?".into();
    }
    let source = unsafe { CStr::from_ptr(debug.short_src) }.to_string_lossy();
    source.strip_prefix("[string \"").and_then(|inner| inner.strip_suffix("\"]")).unwrap_or(&source).to_string()
}

impl Walk<'_> {
    /// Short text for a key or a value at `index`, without changing it (lua_next needs its key as it is).
    unsafe fn describe(&self, index: c_int) -> String {
        unsafe {
            let state = self.state;
            match ffi::lua_type(state, index) {
                ffi::LUA_TSTRING => {
                    let mut length = 0;
                    let text = ffi::lua_tolstring(state, index, &mut length);
                    let bytes = std::slice::from_raw_parts(text as *const u8, length.min(48));
                    format!("\"{}\"", String::from_utf8_lossy(bytes))
                }
                ffi::LUA_TNUMBER => format!("{}", ffi::lua_tonumber(state, index)),
                ffi::LUA_TBOOLEAN => format!("{}", ffi::lua_toboolean(state, index) != 0),
                ffi::LUA_TUSERDATA => match raw::stack_instance(state, index) {
                    Some(_) => "<Instance>".into(),
                    None => "<userdata>".into(),
                },
                ffi::LUA_TFUNCTION => format!("<fn {}>", self.function_place(index)),
                ffi::LUA_TTABLE => "<table>".into(),
                ffi::LUA_TTHREAD => "<thread>".into(),
                ffi::LUA_TVECTOR => "<vector>".into(),
                _ => "<value>".into(),
            }
        }
    }

    unsafe fn function_place(&self, index: c_int) -> String {
        unsafe {
            let state = self.state;
            let index = if index < 0 { ffi::lua_gettop(state) + index + 1 } else { index };
            let mut debug: ffi::lua_Debug = std::mem::zeroed();
            ffi::lua_pushvalue(state, index);
            let ok = ffi::lua_getinfo(state, -1, c"s".as_ptr(), &mut debug) != 0;
            ffi::lua_pop(state, 1);
            if !ok {
                return "?".into();
            }
            if debug.what.is_null() == false && CStr::from_ptr(debug.what).to_bytes() == b"C" {
                return "C".into();
            }
            format!("{}:{}", source_of(&debug), debug.linedefined)
        }
    }

    /// Visits the value at `index` reached from node `parent` by `label`: a target instance is recorded, a new
    /// table, function, thread or userdata is queued.
    unsafe fn reach(&mut self, index: c_int, parent: u32, label: impl FnOnce() -> String) {
        unsafe {
            let state = self.state;
            let kind = ffi::lua_type(state, index);
            if kind == ffi::LUA_TUSERDATA {
                if let Some(id) = raw::stack_instance(state, index) {
                    if let Some(&root) = self.targets.get(&id) {
                        if !self.hits.contains_key(&root) {
                            self.hits.insert(root, (parent, label(), id));
                        }
                    }
                    // (an Instance's own references are its properties: the `property` section)
                    return;
                }
            }
            if !matches!(kind, ffi::LUA_TTABLE | ffi::LUA_TFUNCTION | ffi::LUA_TTHREAD | ffi::LUA_TUSERDATA) {
                return;
            }
            let pointer = ffi::lua_topointer(state, index) as usize;
            if pointer == 0 || self.seen.contains_key(&pointer) {
                return;
            }
            let id = self.nodes.len() as u32;
            self.seen.insert(pointer, id);
            self.nodes.push(Node { parent, label: label() });
            ffi::lua_pushvalue(state, index);
            ffi::lua_rawseti(state, self.queue, id as ffi::lua_Integer + 1);
        }
    }

    /// Queues what the value of node `id` (on top of the stack) references.
    unsafe fn expand(&mut self, id: u32) {
        unsafe {
            let state = self.state;
            ffi::lua_checkstack(state, 16);
            let top = ffi::lua_gettop(state);
            match ffi::lua_type(state, top) {
                ffi::LUA_TTABLE => {
                    let (mut weak_keys, mut weak_values) = (false, false);
                    if ffi::lua_getmetatable(state, top) != 0 {
                        let meta = ffi::lua_gettop(state);
                        ffi::lua_pushstring(state, c"__mode".as_ptr());
                        ffi::lua_rawget(state, meta);
                        if ffi::lua_type(state, -1) == ffi::LUA_TSTRING {
                            let mode = CStr::from_ptr(ffi::lua_tostring(state, -1)).to_bytes().to_vec();
                            weak_keys = mode.contains(&b'k');
                            weak_values = mode.contains(&b'v');
                        }
                        ffi::lua_pop(state, 1);
                        self.reach(meta, id, || "metatable".into());
                        ffi::lua_pop(state, 1);
                    }
                    let weak = if weak_keys { " (weak-keyed)" } else { "" };
                    ffi::lua_pushnil(state);
                    while ffi::lua_next(state, top) != 0 {
                        let key = top + 1;
                        let value = top + 2;
                        if !weak_values && collectable(ffi::lua_type(state, value)) {
                            let text = self.describe(key);
                            self.reach(value, id, || format!("[{text}]{weak}"));
                        }
                        if !weak_keys && collectable(ffi::lua_type(state, key)) {
                            let text = self.describe(value);
                            self.reach(key, id, || format!("key (value {text})"));
                        }
                        ffi::lua_settop(state, key);
                    }
                }
                ffi::LUA_TFUNCTION => {
                    let place = self.function_place(top);
                    let mut index = 1;
                    loop {
                        let name = ffi::lua_getupvalue(state, top, index);
                        if name.is_null() {
                            break;
                        }
                        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
                        let at = ffi::lua_gettop(state);
                        self.reach(at, id, || {
                            if name.is_empty() { format!("fn {place} upvalue #{index}") } else { format!("fn {place} upvalue {name}") }
                        });
                        ffi::lua_settop(state, top);
                        index += 1;
                    }
                    if ffi::lua_iscfunction(state, top) == 0 {
                        ffi::lua_getfenv(state, top);
                        let at = ffi::lua_gettop(state);
                        self.reach(at, id, || format!("fn {place} env"));
                        ffi::lua_settop(state, top);
                    }
                }
                ffi::LUA_TTHREAD => {
                    let co = ffi::lua_tothread(state, top);
                    if co == state {
                        return;
                    }
                    // The thread's globals
                    ffi::lua_getfenv(state, top);
                    let at = ffi::lua_gettop(state);
                    self.reach(at, id, || "thread env".into());
                    ffi::lua_settop(state, top);
                    // Its whole stack: every frame's function and registers, as the collector marks them
                    let slots = if self.raw_stacks {
                        let stack = read_ptr(co, STACK_OFFSET);
                        let co_top = read_ptr(co, TOP_OFFSET);
                        if co_top >= stack { (co_top - stack) / TVALUE_SIZE } else { 0 }
                    } else {
                        ffi::lua_gettop(co).max(0) as usize
                    };
                    let mut debug: ffi::lua_Debug = std::mem::zeroed();
                    let frame = if ffi::lua_getinfo(co, 0, c"sl".as_ptr(), &mut debug) != 0 {
                        format!("{}:{}", source_of(&debug), debug.currentline)
                    } else {
                        String::new()
                    };
                    // (a slot is copied as the VM's setobj does: an index below -10000 would be a pseudo-index,
                    // and mlua's ref thread holds more values than that)
                    let stack = if self.raw_stacks { read_ptr(co, STACK_OFFSET) } else { 0 };
                    for k in 0..slots.min(1_000_000) {
                        if self.raw_stacks {
                            ffi::lua_pushnil(state);
                            let destination = read_ptr(state, TOP_OFFSET) - TVALUE_SIZE;
                            std::ptr::copy_nonoverlapping((stack + k * TVALUE_SIZE) as *const u8, destination as *mut u8, TVALUE_SIZE);
                        } else {
                            ffi::lua_xpush(co, state, k as c_int + 1);
                        }
                        let at = ffi::lua_gettop(state);
                        self.reach(at, id, || format!("stack slot {k} (thread at {frame})"));
                        ffi::lua_settop(state, top);
                    }
                }
                ffi::LUA_TUSERDATA => {
                    if ffi::lua_getmetatable(state, top) != 0 {
                        let at = ffi::lua_gettop(state);
                        self.reach(at, id, || "metatable".into());
                        ffi::lua_settop(state, top);
                    }
                }
                _ => {}
            }
        }
    }

    fn path(&self, mut node: u32, last: &str) -> Vec<String> {
        let mut steps = vec![last.to_string()];
        while node != u32::MAX {
            let entry = &self.nodes[node as usize];
            steps.push(entry.label.clone());
            node = entry.parent;
        }
        steps.reverse();
        steps
    }
}

/// Numbers in a way replaced by '#', so ways that differ only by an id or an index count together.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_number = false;
    for c in text.chars() {
        if c.is_ascii_digit() || (in_number && c == '.') {
            if !in_number {
                out.push('#');
                in_number = true;
            }
        } else {
            in_number = false;
            out.push(c);
        }
    }
    out
}

/// The report (module comment). `examples`: full ways printed per group.
pub fn report(lua: &Lua, engine: &Rc<Engine>, filters: Vec<String>, examples: usize, requeue: bool) -> LuaResult<Json> {
    let live_before = engine.state.borrow().dom.live_count();
    let mut freed_by_requeue = 0;
    if requeue {
        lua.gc_collect()?;
        lua.gc_collect()?;
        raw::collect(lua, engine)?;
        let settled = engine.state.borrow().dom.live_count();
        raw::requeue_orphans(engine);
        raw::collect(lua, engine)?;
        freed_by_requeue = settled - engine.state.borrow().dom.live_count();
    }
    // (until nothing more is freed: a collection can free userdata whose trees the next instance collection frees)
    for _ in 0..8 {
        lua.gc_collect()?;
        lua.gc_collect()?;
        if raw::collect(lua, engine)? == 0 {
            break;
        }
    }

    let reflection = &engine.reflection;
    let game = engine.world().game;

    // Targets: live trees outside the DataModel whose root matches a filter
    struct Group {
        filter: String,
        roots: Vec<InstanceId>,
        instances: usize,
        explained: FxHashMap<InstanceId, &'static str>,
        holders: FxHashMap<String, (usize, String)>,
        with_userdata: usize,
        anchored: usize,
    }
    let mut groups: Vec<Group> = filters
        .iter()
        .map(|filter| Group {
            filter: filter.clone(),
            roots: Vec::new(),
            instances: 0,
            explained: FxHashMap::default(),
            holders: FxHashMap::default(),
            with_userdata: 0,
            anchored: 0,
        })
        .collect();
    let mut targets: FxHashMap<InstanceId, InstanceId> = FxHashMap::default();
    let mut group_of: FxHashMap<InstanceId, usize> = FxHashMap::default();
    let describe_instance = |state: &crate::State, id: InstanceId| -> String {
        let instance = state.dom.at(id);
        let mut top = id;
        while let Some(parent) = state.dom.at(top).parent {
            top = parent;
        }
        let name = format!("{} \"{}\"", reflection.class(instance.class).name, instance.name.to_str_lossy());
        if top == game {
            format!("{name} at {}", state.dom.full_name(reflection, id))
        } else if top == id {
            format!("{name} (outside the DataModel)")
        } else {
            let root = state.dom.at(top);
            format!("{name} in (nil) {} \"{}\"", reflection.class(root.class).name, root.name.to_str_lossy())
        }
    };
    {
        let state = engine.state.borrow();
        let dom = &state.dom;
        for index in 0..dom.slot_count() as u32 {
            let Some(id) = dom.id_at_slot(index) else { continue };
            let instance = dom.at(id);
            if instance.parent.is_some() || id == game || instance.has(flags::STUDIO_ONLY) {
                continue;
            }
            let label = format!("{} {}", reflection.class(instance.class).name, instance.name.to_str_lossy());
            let Some(group) = filters.iter().position(|filter| label.starts_with(filter.as_str())) else { continue };
            let mut nodes = vec![id];
            nodes.extend(dom.descendants(id));
            groups[group].roots.push(id);
            groups[group].instances += nodes.len();
            group_of.insert(id, group);
            if nodes.iter().any(|&node| dom.is_pinned(node)) {
                groups[group].explained.insert(id, "pinned");
            }
            let userdata: Vec<(bool, bool)> = nodes.iter().map(|&node| raw::userdata_state(engine, node)).collect();
            if userdata.iter().any(|&(live, _)| live) {
                groups[group].with_userdata += 1;
            }
            if userdata.iter().any(|&(_, anchored)| anchored) {
                groups[group].anchored += 1;
            }
            for node in nodes {
                targets.insert(node, id);
            }
        }

        // Instance properties that reference a target (from outside its own tree)
        for index in 0..dom.slot_count() as u32 {
            let Some(id) = dom.id_at_slot(index) else { continue };
            let instance = dom.at(id);
            if !instance.has(flags::HOLDS_REFS) {
                continue;
            }
            for (name, value) in instance.props.iter() {
                let mut referenced = Vec::new();
                value.visit_instances(&mut |target| referenced.push(target));
                for target in referenced {
                    let Some(&root) = targets.get(&target) else { continue };
                    if targets.get(&id) == Some(&root) {
                        continue;
                    }
                    let group = &mut groups[group_of[&root]];
                    group.explained.entry(root).or_insert("property");
                    let holder = format!("{}.{} = {}", describe_instance(&state, id), reflection.name(name), describe_instance(&state, target));
                    let entry = group.holders.entry(format!("property: {}", normalize(&holder))).or_insert((0, holder));
                    entry.0 += 1;
                }
            }
        }
        let mut queued = Vec::new();
        state.sched.visit_instance_refs(&mut |id| queued.push(id));
        for target in queued {
            if let Some(&root) = targets.get(&target) {
                let group = &mut groups[group_of[&root]];
                group.explained.entry(root).or_insert("queued");
                let entry = group.holders.entry("queued work (deferred event or timer arguments)".into()).or_insert((0, String::new()));
                entry.0 += 1;
            }
        }
    }

    // Seeds, labelled: module results, connected handlers, waiting threads
    let seeds = lua.create_table()?;
    let mut seed_labels: Vec<String> = Vec::new();
    {
        let state = engine.state.borrow();
        let mut push = |label: String, value: LuaValue| -> LuaResult<()> {
            seed_labels.push(label);
            seeds.raw_set(seed_labels.len(), value)
        };
        for (module, value) in engine.modules.borrow().results() {
            let name = state.dom.get(module).map(|_| state.dom.full_name(reflection, module)).unwrap_or_else(|| "(freed module)".into());
            push(format!("module {name}"), value)?;
        }
        let mut owners: Vec<(u32, InstanceId, String)> = Vec::new();
        for index in 0..state.dom.slot_count() as u32 {
            let Some(id) = state.dom.id_at_slot(index) else { continue };
            let Some(signals) = state.dom.at(id).signals.as_deref() else { continue };
            for (name, signal) in signals.events.iter() {
                owners.push((*signal, id, reflection.name(*name).to_string()));
            }
            for (name, signal) in signals.properties.iter() {
                owners.push((*signal, id, format!("GetPropertyChangedSignal(\"{}\")", reflection.name(*name))));
            }
            for (name, signal) in signals.attributes.iter() {
                owners.push((*signal, id, format!("GetAttributeChangedSignal(\"{}\")", name.to_str_lossy())));
            }
        }
        for (signal, id, name) in owners {
            for handler in state.signals.connected(signal) {
                let info = handler.info();
                let source = info.short_src.unwrap_or_default();
                let source = source.strip_prefix("[string \"").and_then(|inner| inner.strip_suffix("\"]")).unwrap_or(&source).to_string();
                let label = format!("handler {source}:{} on {}.{name}", info.line_defined.unwrap_or(0), state.dom.full_name(reflection, id));
                push(label, LuaValue::Function(handler))?;
            }
        }
        let mut waiting: Vec<(LuaThread, String)> = Vec::new();
        state.signals.visit_waiting(&mut |thread| waiting.push((thread.clone(), "thread waiting on a signal".into())));
        state.sched.visit_waiting(&mut |thread, kind, _| waiting.push((thread.clone(), format!("thread the scheduler resumes ({kind})"))));
        for (thread, label) in waiting {
            push(label, LuaValue::Thread(thread))?;
        }
    }
    let registry_names: FxHashMap<i64, &'static str> = raw::registry_names(engine).into_iter().map(|(slot, name)| (slot as i64, name)).collect();

    // The walk
    lua.gc_stop();
    let mut hits: FxHashMap<InstanceId, (Vec<String>, InstanceId)> = FxHashMap::default();
    let mut visited = 0usize;
    let mut raw_stacks = false;
    let walked = unsafe {
        lua.exec_raw::<()>(seeds, |state| {
            ffi::lua_checkstack(state, 32);
            let seeds_at = ffi::lua_gettop(state);
            ffi::lua_createtable(state, 1 << 16, 0);
            let queue = ffi::lua_gettop(state);
            let mut walk = Walk {
                state,
                queue,
                nodes: Vec::new(),
                seen: FxHashMap::default(),
                targets: &targets,
                hits: FxHashMap::default(),
                raw_stacks: layout_holds(state),
                registry_names,
            };
            // (the walker's own tables are not holders)
            walk.seen.insert(ffi::lua_topointer(state, queue) as usize, u32::MAX);
            walk.seen.insert(ffi::lua_topointer(state, seeds_at) as usize, u32::MAX);
            for (index, label) in seed_labels.iter().enumerate() {
                ffi::lua_rawgeti(state, seeds_at, index as ffi::lua_Integer + 1);
                let at = ffi::lua_gettop(state);
                walk.reach(at, u32::MAX, || label.clone());
                ffi::lua_settop(state, queue);
            }
            // The running frames' functions
            let mut level = 1;
            loop {
                let mut debug: ffi::lua_Debug = std::mem::zeroed();
                if level > 200 || ffi::lua_getinfo(state, level, c"fs".as_ptr(), &mut debug) == 0 {
                    break;
                }
                let at = ffi::lua_gettop(state);
                let label = format!("running frame {level} ({}:{})", source_of(&debug), debug.linedefined);
                walk.reach(at, u32::MAX, || label);
                ffi::lua_settop(state, queue);
                level += 1;
            }
            ffi::lua_pushvalue(state, ffi::LUA_GLOBALSINDEX);
            walk.reach(ffi::lua_gettop(state), u32::MAX, || "globals".into());
            ffi::lua_settop(state, queue);
            // The registry, its Instance API tables named
            ffi::lua_pushvalue(state, REGISTRY);
            let registry = ffi::lua_gettop(state);
            walk.seen.insert(ffi::lua_topointer(state, registry) as usize, u32::MAX);
            ffi::lua_pushnil(state);
            while ffi::lua_next(state, registry) != 0 {
                let key = registry + 1;
                let value = registry + 2;
                let text = if ffi::lua_type(state, key) == ffi::LUA_TNUMBER {
                    let slot = ffi::lua_tonumber(state, key) as i64;
                    match walk.registry_names.get(&slot) {
                        Some(name) => format!("registry[{slot}] ({name})"),
                        None => format!("registry[{slot}]"),
                    }
                } else {
                    format!("registry[{}]", walk.describe(key))
                };
                walk.reach(value, u32::MAX, || text);
                ffi::lua_settop(state, key);
            }
            ffi::lua_settop(state, queue);
            let mut next = 0u32;
            while (next as usize) < walk.nodes.len() {
                ffi::lua_rawgeti(state, queue, next as ffi::lua_Integer + 1);
                walk.expand(next);
                ffi::lua_settop(state, queue);
                next += 1;
            }
            visited = walk.nodes.len();
            raw_stacks = walk.raw_stacks;
            for (root, (node, step, instance)) in std::mem::take(&mut walk.hits) {
                hits.insert(root, (walk.path(node, &step), instance));
            }
            ffi::lua_settop(state, seeds_at);
        })
    };
    lua.gc_restart();
    walked?;

    // Per group: what explains each root, and the ways
    let state = engine.state.borrow();
    let mut out = Vec::new();
    for group in &mut groups {
        let mut ways: FxHashMap<String, (usize, Vec<String>)> = FxHashMap::default();
        let mut by_lua = 0;
        for &root in &group.roots {
            let Some((path, instance)) = hits.get(&root) else { continue };
            by_lua += 1;
            group.explained.entry(root).or_insert("lua");
            let reached = if *instance == root { String::new() } else { format!(" (reaches {})", describe_instance(&state, *instance)) };
            let text = format!("{}{reached}", path.join(" > "));
            let entry = ways.entry(normalize(&text)).or_insert((0, Vec::new()));
            entry.0 += 1;
            if entry.1.len() < examples {
                entry.1.push(text);
            }
        }
        let mut ways: Vec<(String, (usize, Vec<String>))> = ways.into_iter().collect();
        ways.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
        let mut holders: Vec<(String, (usize, String))> = group.holders.drain().collect();
        holders.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
        let mut explained: FxHashMap<&'static str, usize> = FxHashMap::default();
        for &root in &group.roots {
            *explained.entry(group.explained.get(&root).copied().unwrap_or("unexplained")).or_default() += 1;
        }
        out.push(json!({
            "filter": group.filter,
            "roots": group.roots.len(),
            "instances": group.instances,
            "explained": explained,
            "reachedFromLua": by_lua,
            "withUserdata": group.with_userdata,
            "anchored": group.anchored,
            "ways": ways.iter().take(WAYS).map(|(way, (count, examples))| json!({"way": way, "roots": count, "examples": examples})).collect::<Vec<_>>(),
            "otherHolders": holders.iter().take(WAYS).map(|(key, (count, example))| json!({"holder": key, "references": count, "example": example})).collect::<Vec<_>>(),
        }));
    }
    Ok(json!({
        "type": "holders",
        "time": state.sched.now,
        "valuesWalked": visited,
        "liveBefore": live_before,
        "freedByRequeue": freed_by_requeue,
        "wholeStacks": raw_stacks,
        "groups": out,
    }))
}

