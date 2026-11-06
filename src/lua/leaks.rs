//! What keeps removed UI alive: the trees outside the DataModel that scripts still hold, and the
//! connected handlers in them that capture their own tree.
//!
//! Roblox never frees an instance whose connected handler references it (measured in Studio
//! 0.740: a Frame whose GetPropertyChangedSignal handler captures it, parented to nil with every
//! other reference dropped, survives any amount of garbage collection, and so does a tree one of
//! whose descendants' handlers captures the root; `Destroy` or `Disconnect` releases it). The
//! engine follows that, so UI code that removes instances without destroying them (Vide's `show` /
//! `switch` unparent what they replace) leaks every instance a still-connected handler reaches,
//! here as in the game. A session's `{"cmd":"leaks"}` lists them: per handler (script and line,
//! signal, class of the instance it is connected to) the number of trees it holds and what it
//! captures.

use std::ffi::CStr;
use std::rc::Rc;

use mlua::{ffi, prelude::*};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Value as Json, json};

use super::raw;
use crate::{Engine, InstanceId, dom::flags};

/// How far a handler's captures are followed (upvalues, then table fields and closures' upvalues)
const DEPTH: usize = 6;
/// Values looked at per handler
const BUDGET: usize = 4000;

struct Walk<'a> {
    state: *mut ffi::lua_State,
    seen: FxHashSet<usize>,
    budget: usize,
    /// The instance that ends the search: one outside the DataModel
    outside: &'a dyn Fn(InstanceId) -> bool,
}

impl Walk<'_> {
    /// Looks for an instance outside the DataModel in the value on top of the stack (left there);
    /// returns it with the way to it ("upvalue Frame", "upvalue Props > field Instance").
    unsafe fn find(&mut self, depth: usize) -> Option<(InstanceId, String)> {
        unsafe {
            let state = self.state;
            if let Some(id) = raw::stack_instance(state, -1) {
                return (self.outside)(id).then(|| (id, String::new()));
            }
            let kind = ffi::lua_type(state, -1);
            if depth == 0 || self.budget == 0 || (kind != ffi::LUA_TFUNCTION && kind != ffi::LUA_TTABLE) {
                return None;
            }
            self.budget -= 1;
            if !self.seen.insert(ffi::lua_topointer(state, -1) as usize) {
                return None;
            }
            ffi::lua_checkstack(state, 8);
            if kind == ffi::LUA_TFUNCTION {
                if ffi::lua_iscfunction(state, -1) != 0 {
                    return None;
                }
                let mut index = 1;
                loop {
                    let name = ffi::lua_getupvalue(state, -1, index);
                    if name.is_null() {
                        return None;
                    }
                    let name = CStr::from_ptr(name).to_string_lossy().into_owned();
                    let found = self.find(depth - 1);
                    ffi::lua_pop(state, 1);
                    if let Some((id, path)) = found {
                        return Some((id, join(format!("upvalue {name}"), path)));
                    }
                    index += 1;
                }
            }
            // A table: its values (keys that are instances count too)
            ffi::lua_pushnil(state);
            while ffi::lua_next(state, -2) != 0 {
                let key = match ffi::lua_type(state, -2) {
                    ffi::LUA_TSTRING => {
                        let mut length = 0;
                        let text = ffi::lua_tolstring(state, -2, &mut length);
                        String::from_utf8_lossy(std::slice::from_raw_parts(text as *const u8, length.min(40))).into_owned()
                    }
                    ffi::LUA_TNUMBER => format!("[{}]", ffi::lua_tonumber(state, -2)),
                    _ => "[key]".into(),
                };
                let mut found = self.find(depth - 1);
                ffi::lua_pop(state, 1);
                if found.is_none() {
                    if let Some(id) = raw::stack_instance(state, -1).filter(|&id| (self.outside)(id)) {
                        found = Some((id, String::new()));
                    }
                }
                if let Some((id, path)) = found {
                    ffi::lua_pop(state, 1);
                    return Some((id, join(format!("field {key}"), path)));
                }
            }
            None
        }
    }
}

fn join(step: String, rest: String) -> String {
    if rest.is_empty() { step } else { format!("{step} > {rest}") }
}

/// The report (see the module comment). Collects garbage first, so what is listed is held.
pub fn report(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<Json> {
    lua.gc_collect()?;
    lua.gc_collect()?;
    raw::collect(lua, engine)?;
    let (userdata, _, _) = raw::stats(engine);

    // Trees outside the DataModel, and each instance's root
    struct Connected {
        instance: InstanceId,
        signal: String,
        handlers: Vec<LuaFunction>,
    }
    let game = engine.world().game;
    let mut roots: Vec<(InstanceId, usize)> = Vec::new();
    let mut root_of: FxHashMap<InstanceId, InstanceId> = FxHashMap::default();
    let mut connected: Vec<Connected> = Vec::new();
    let (live, outside_total) = {
        let state = engine.state.borrow();
        for index in 0..state.dom.slot_count() as u32 {
            let Some(id) = state.dom.id_at_slot(index) else { continue };
            let instance = state.dom.at(id);
            if instance.parent.is_some() || id == game || instance.has(flags::STUDIO_ONLY) {
                continue;
            }
            let mut nodes = vec![id];
            nodes.extend(state.dom.descendants(id));
            for &node in &nodes {
                root_of.insert(node, id);
                let Some(signals) = state.dom.at(node).signals.as_deref() else { continue };
                let names = signals
                    .events
                    .iter()
                    .map(|(name, signal)| (engine.reflection.name(*name).to_string(), *signal))
                    .chain(signals.properties.iter().map(|(name, signal)| (format!("GetPropertyChangedSignal(\"{}\")", engine.reflection.name(*name)), *signal)))
                    .chain(signals.attributes.iter().map(|(name, signal)| (format!("GetAttributeChangedSignal(\"{}\")", name.to_str_lossy()), *signal)));
                for (signal, id) in names {
                    let handlers = state.signals.connected(id);
                    if !handlers.is_empty() {
                        connected.push(Connected { instance: node, signal, handlers });
                    }
                }
            }
            roots.push((id, nodes.len()));
        }
        (state.dom.live_count(), root_of.len())
    };

    // Per handler: where it is defined and the instance outside the DataModel it captures
    let outside = |id: InstanceId| root_of.contains_key(&id);
    let describe = |id: InstanceId| {
        let state = engine.state.borrow();
        match state.dom.get(id) {
            Some(instance) => format!("{} \"{}\"", engine.reflection.class(instance.class).name, instance.name.to_str_lossy()),
            None => "(freed)".to_string(),
        }
    };
    #[derive(Default)]
    struct Group {
        connections: usize,
        trees: FxHashSet<InstanceId>,
        own_tree: usize,
        captures: Option<String>,
        example: String,
    }
    let mut groups: FxHashMap<(String, String, String), Group> = FxHashMap::default();
    let mut holding: FxHashSet<InstanceId> = FxHashSet::default();
    for entry in &connected {
        let root = root_of[&entry.instance];
        let class = {
            let state = engine.state.borrow();
            engine.reflection.class(state.dom.at(entry.instance).class).name.to_string()
        };
        for handler in &entry.handlers {
            let info = handler.info();
            let source = info.short_src.or(info.source).unwrap_or_default();
            let place = format!("{}:{}", source.trim_start_matches("[string \"").trim_end_matches("\"]"), info.line_defined.unwrap_or(0));
            let found = unsafe {
                let mut result = None;
                lua.exec_raw::<()>(handler.clone(), |state| {
                    let mut walk = Walk { state, seen: FxHashSet::default(), budget: BUDGET, outside: &outside };
                    result = walk.find(DEPTH);

                })?;
                result
            };
            let group = groups.entry((place, entry.signal.clone(), class.clone())).or_default();
            group.connections += 1;
            if let Some((captured, path)) = found {
                let captured_root = root_of[&captured];
                group.trees.insert(captured_root);
                holding.insert(captured_root);
                if captured_root == root {
                    group.own_tree += 1;
                }
                if group.captures.is_none() {
                    group.captures = Some(format!("{path}: {}", describe(captured)));
                    let state = engine.state.borrow();
                    group.example = state.dom.full_name(&engine.reflection, entry.instance);
                }
            }
        }
    }
    let mut handlers: Vec<Json> = groups
        .into_iter()
        .filter(|(_, group)| group.captures.is_some())
        .map(|((place, signal, class), group)| {
            json!({
                "handler": place,
                "signal": signal,
                "class": class,
                "connections": group.connections,
                "trees": group.trees.len(),
                "ownTree": group.own_tree,
                "captures": group.captures,
                "example": group.example,
            })
        })
        .collect();
    handlers.sort_by(|a, b| {
        (b["trees"].as_u64(), b["connections"].as_u64(), a["handler"].as_str()).cmp(&(a["trees"].as_u64(), a["connections"].as_u64(), b["handler"].as_str()))
    });
    roots.sort_by_key(|&(id, size)| (std::cmp::Reverse(size), id.index));
    let held_by_handlers: usize = roots.iter().filter(|(id, _)| holding.contains(id)).map(|&(_, size)| size).sum();
    let largest: Vec<Json> = roots
        .iter()
        .take(12)
        .map(|&(id, size)| json!({ "root": describe(id), "instances": size, "heldByHandler": holding.contains(&id) }))
        .collect();
    Ok(json!({
        "type": "leaks",
        "instances": live,
        "userdata": userdata,
        "outsideDataModel": { "trees": roots.len(), "instances": outside_total, "heldByConnectedHandlers": held_by_handlers },
        "handlers": handlers,
        "largestTrees": largest,
    }))
}

