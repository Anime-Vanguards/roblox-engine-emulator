//! `Context.TraceWrites(Properties?, Depth?)`: records where in the harness's Luau each hierarchy change
//! (Parent, Destroy, Clone's parenting, ...) and each write of the listed properties happened. Signals
//! are Deferred and Instance metatables are locked, so a script cannot see its writer; the hooks here run
//! inside the write, on the writing thread, and read its stack.
//!
//! Off by default: nothing runs and `meta_newindex` / `meta_namecall` pay one relaxed load. The first
//! `Context.TraceWrites` call turns it on for the process.
//!
//! A listed property `"*"` watches every property, and also SetAttribute (Kind "attribute", Property = the
//! attribute's name). Only writes that change a value are reported. `Drain().Dropped` (read through a metatable, so
//! iterating the drain still sees only the writes) counts the writes whose instance the collector had freed by then
//! (stale), which have no value to return.
//!
//! The call returns `Drain()`, which returns (and clears) the writes since the last call as an array of
//! `{Kind = "parent" | "property" | "attribute", Instance, Property?, Old?, New?, Add?, Remove?, Stack = {"Chunk:line", ...}}`,
//! in order. `Old` and `New` are the parents of a `parent` write; `Add` / `Remove` say the new / old parent is
//! `Root` (the third argument, default Workspace) or inside it, which is when DescendantAdded / Removing fire
//! on it (a move inside Root is both). `Stack` is the innermost Luau frames of the writing thread, innermost
//! first, `Depth` of them (default 3), as `Chunk:line` (the chunk name is the script's path in the DataModel).
//! Writes made by the engine outside any running Luau frame (Debris, the scheduler) have an empty Stack.

use std::{
    cell::{Cell, RefCell},
    ffi::CStr,
    os::raw::c_int,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use mlua::prelude::*;
use rustc_hash::FxHashSet;

use super::{Engine, InstanceId, NameId, lua::instance_value};

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The Lua thread of the metamethod now running (set by `enter`, only while tracing)
    static CURRENT: Cell<*mut mlua::ffi::lua_State> = const { Cell::new(std::ptr::null_mut()) };
}

struct Write {
    parent: bool,
    id: InstanceId,
    property: NameId,
    /// the attribute's name, for a SetAttribute write (Kind "attribute")
    attribute: Option<String>,
    old: Option<InstanceId>,
    new: Option<InstanceId>,
    add: bool,
    remove: bool,
    stack: Vec<String>,
}

/// Called by the Instance metamethods on entry. One relaxed load when tracing is off.
#[inline]
pub fn enter(state: *mut mlua::ffi::lua_State) {
    if ENABLED.load(Ordering::Relaxed) {
        CURRENT.with(|current| current.set(state));
    }
}

/// The innermost `depth` Luau frames of the thread last entered through a metamethod.
fn stack(depth: usize) -> Vec<String> {
    let state = CURRENT.with(Cell::get);
    let mut frames = Vec::new();
    // (a thread that is not running is a stale one: the write came from the engine, not from Luau)
    if state.is_null() || unsafe { mlua::ffi::lua_status(state) } != mlua::ffi::LUA_OK {
        return frames;
    }
    unsafe {
        let mut debug: mlua::ffi::lua_Debug = std::mem::zeroed();
        let mut level: c_int = 0;
        while level < 60 && frames.len() < depth && mlua::ffi::lua_getinfo(state, level, c"sl".as_ptr(), &mut debug) != 0 {
            level += 1;
            if debug.short_src.is_null() || debug.currentline < 0 {
                continue;
            }
            let source = CStr::from_ptr(debug.short_src).to_string_lossy();
            // (C functions and the engine's own Luau, the prelude, are not the writer)
            if source == "[C]" || source.starts_with("ree.") || source.starts_with("rur.") || source.contains("prelude") {
                continue;
            }
            // (a chunk loaded from a string is named `[string "Path"]`)
            let source = source.strip_prefix("[string \"").and_then(|inner| inner.strip_suffix("\"]")).unwrap_or(&source);
            frames.push(format!("{source}:{}", debug.currentline));
        }
    }
    frames
}

/// Installs the hooks and returns `Drain`.
pub fn install(lua: &Lua, engine: &Rc<Engine>, properties: Vec<String>, depth: usize, root: Option<InstanceId>) -> LuaResult<LuaFunction> {
    ENABLED.store(true, Ordering::Relaxed);
    // ("*" lists every property: the census of what a script writes at all, for a hierarchy it names itself)
    let all = properties.iter().any(|name| name == "*");
    let watched: FxHashSet<NameId> = properties.iter().filter_map(|name| engine.reflection.name_id(name)).collect();
    let root = root.unwrap_or(engine.world().workspace);
    let writes: Rc<RefCell<Vec<Write>>> = Rc::default();
    {
        let mut hooks = engine.hooks.borrow_mut();
        let log = writes.clone();
        hooks.hierarchy_changed.push(Box::new(move |state, id, old, new| {
            let within = |parent: Option<InstanceId>| parent.is_some_and(|parent| parent == root || state.dom.is_descendant_of(parent, root));
            let (add, remove) = (within(new), within(old));
            log.borrow_mut().push(Write { parent: true, id, property: 0, attribute: None, old, new, add, remove, stack: stack(depth) });
        }));
        if all || !watched.is_empty() {
            let log = writes.clone();
            hooks.property_changed.push(Box::new(move |_state, id, name| {
                if all || watched.contains(&name) {
                    log.borrow_mut().push(Write { parent: false, id, property: name, attribute: None, old: None, new: None, add: false, remove: false, stack: stack(depth) });
                }
            }));
        }
        if all {
            let log = writes.clone();
            hooks.attribute_changed.push(Box::new(move |_state, id, name| {
                log.borrow_mut().push(Write { parent: false, id, property: 0, attribute: Some(name.to_owned()), old: None, new: None, add: false, remove: false, stack: stack(depth) });
            }));
        }
    }
    let engine = Rc::downgrade(engine);
    lua.create_function(move |lua, ()| {
        let Some(engine) = engine.upgrade() else { return Ok(lua.create_table()?) };
        let taken = std::mem::take(&mut *writes.borrow_mut());
        let out = lua.create_table_with_capacity(taken.len(), 0)?;
        let mut dropped = 0;
        for write in taken {
            // (an instance the collector freed since its write has no value to hand back: counted in Dropped, and a
            // freed Old or New is left out, so one dead instance does not lose the whole drain)
            let Ok(instance) = instance_value(lua, &engine, write.id) else {
                dropped += 1;
                continue;
            };
            let entry = lua.create_table()?;
            entry.set("Kind", if write.parent { "parent" } else if write.attribute.is_some() { "attribute" } else { "property" })?;
            entry.set("Instance", instance)?;
            if write.parent {
                entry.set("Add", write.add)?;
                entry.set("Remove", write.remove)?;
                if let Some(old) = write.old.and_then(|old| instance_value(lua, &engine, old).ok()) {
                    entry.set("Old", old)?;
                }
                if let Some(new) = write.new.and_then(|new| instance_value(lua, &engine, new).ok()) {
                    entry.set("New", new)?;
                }
            } else if let Some(attribute) = &write.attribute {
                entry.set("Property", attribute.as_str())?;
            } else {
                entry.set("Property", engine.reflection.name(write.property))?;
            }
            entry.set("Stack", lua.create_sequence_from(write.stack)?)?;
            out.push(entry)?;
        }
        // (`Dropped` through a metatable, not a key: the drained table is iterated as an array of writes)
        let (meta, index) = (lua.create_table()?, lua.create_table()?);
        index.set("Dropped", dropped)?;
        meta.set("__index", index)?;
        out.set_metatable(Some(meta))?;
        Ok(out)
    })
}
