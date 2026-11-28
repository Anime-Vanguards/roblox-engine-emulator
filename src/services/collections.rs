//! CollectionService:CreateCollection's collections: the instances below a root that a QueryDescendants query matches, as a
//! playing client and a server keep them (measured, roblox-sim 0.747, two sessions each side, a client and a server alike:
//! tools/parity/probes/collection_handles.luau and collection_callbacks.luau). The handle a script holds is services.luau's (its
//! end block: typeof CollectionHandle, tostring Collection, Query, Root, GetCount, Destroy and the On* callbacks); this keeps each
//! collection's members and runs its updates.
//!
//! - A collection holds what the query matched when it was made, in QueryDescendants' order, and from then on an instance that
//!   comes to match (it or an ancestor moves below the root, it gains the tag, a compared property or attribute changes) joins at
//!   the next deferred point: a job queued at the change, after the change's own signals for a tag, a property or an attribute,
//!   before them for a move (the move's ChildAdded and CollectionService's added signal see it counted). One that stops matching
//!   leaves the same way.
//! - GetCount counts the members that match now: one that stopped matching is not counted from the change on, but one that comes
//!   back before it left (a tag off and on again) is counted all along, and one that came and went between two deferred points
//!   never joined. GetCollections lists the collections an instance is such a member of, in the order they were made.
//! - OnAdded and OnRemoved run when an instance joins or leaves (the collection's first members' OnAdded at the first deferred
//!   point after the collection was made; Destroy's OnRemoved for each member, in the order they joined, at the next one).
//!   OnChanged(instance, property) runs after each Changed of a member that still is one when it runs, a move's before the move's
//!   own signals. A client delivers it through a connection it makes to the member's Changed as it comes below the root, so a
//!   handler connected before that runs after it (Roblox's newest-first order); ree's follows the change's own handlers.
//! - A collection whose root is destroyed is destroyed at the next deferred point; until then it counts what still matches.

use std::rc::Rc;

use mlua::prelude::*;
use rustc_hash::FxHashSet;

use super::query::{self, Selector};
use crate::{
    Engine, InstanceId, State,
    lua::{Flow, instance_value, raise_plain},
    reflection::{K, NameId},
    scheduler::JobTarget,
    value::Value,
};

/// What a queued collection job does (its first argument)
const CHECK: f64 = 0.0;
const CHECK_SUBTREE: f64 = 1.0;
const FIRST_ADDED: f64 = 2.0;
const DESTROYED: f64 = 3.0;
const CHANGED: f64 = 4.0;
const ROOT_DESTROYED: f64 = 5.0;

pub(crate) struct Collection {
    query: Vec<u8>,
    selectors: Rc<Vec<Selector>>,
    root: InstanceId,
    /// The members, in the order they joined
    members: Vec<InstanceId>,
    held: FxHashSet<InstanceId>,
    destroyed: bool,
    /// Destroy's members, for its OnRemoved
    gone: Vec<InstanceId>,
    added: Option<LuaFunction>,
    removed: Option<LuaFunction>,
    changed: Option<LuaFunction>,
    /// The properties (Name for a #name filter) and attributes the query compares
    properties: Vec<NameId>,
    attributes: Vec<Vec<u8>>,
}

/// Every collection a run made (an id is its index + 1; destroyed ones stay, for their handles' errors)
#[derive(Default)]
pub(crate) struct Collections {
    list: Vec<Collection>,
    live: usize,
}

fn defer(state: &mut State, kind: f64, id: usize, instance: InstanceId, property: Option<&str>) {
    let mut args = vec![Value::Number(kind), Value::Number(id as f64)];
    if let Some(property) = property {
        args.push(Value::String(crate::value::Str::from(property)));
    }
    state.sched.defer(JobTarget::Collection(instance), args);
}

fn below(state: &State, id: InstanceId, root: InstanceId) -> bool {
    id != root && state.dom.is_descendant_of(id, root)
}

/// A property of `id` changed (instance.rs notify_changed_hooks: after its Changed for a property, at the move for Parent)
pub(crate) fn property_changed(state: &mut State, reflection: &crate::reflection::Reflection, id: InstanceId, name: NameId) {
    if state.collection.handles.live == 0 {
        return;
    }
    for index in 0..state.collection.handles.list.len() {
        let collection = &state.collection.handles.list[index];
        if collection.destroyed {
            continue;
        }
        let root = collection.root;
        let inside = below(state, id, root);
        let changed = collection.changed.is_some();
        if name == K::Parent as NameId {
            let held = !collection.members.is_empty();
            if inside || held {
                // (the moved instance joins only if it matched as it moved: one given its tag later joins after that change's
                // signals; what is below it is looked at when the job runs)
                let joinable = inside && query::may_match(state, reflection, id, &collection.selectors);
                let mut args = vec![Value::Number(CHECK_SUBTREE), Value::Number((index + 1) as f64), Value::Bool(joinable)];
                args.shrink_to_fit();
                state.sched.defer(JobTarget::Collection(id), args);
            }
        } else if inside && collection.properties.contains(&name) {
            defer(state, CHECK, index + 1, id, None);
        }
        if inside && changed {
            defer(state, CHANGED, index + 1, id, Some(reflection.name(name)));
        }
    }
}

/// `id` gained or lost a tag (after CollectionService's signals for it)
pub(crate) fn tag_changed(state: &mut State, id: InstanceId) {
    if state.collection.handles.live == 0 {
        return;
    }
    for index in 0..state.collection.handles.list.len() {
        let collection = &state.collection.handles.list[index];
        if !collection.destroyed && below(state, id, collection.root) {
            defer(state, CHECK, index + 1, id, None);
        }
    }
}

/// An attribute of `id` changed
pub(crate) fn attribute_changed(state: &mut State, id: InstanceId, name: &[u8]) {
    if state.collection.handles.live == 0 {
        return;
    }
    for index in 0..state.collection.handles.list.len() {
        let collection = &state.collection.handles.list[index];
        if !collection.destroyed && collection.attributes.iter().any(|attribute| attribute == name) && below(state, id, collection.root) {
            defer(state, CHECK, index + 1, id, None);
        }
    }
}

/// `id` was destroyed: the collections it is the root of go at the next deferred point
pub(crate) fn destroyed(state: &mut State, id: InstanceId) {
    if state.collection.handles.live == 0 {
        return;
    }
    for index in 0..state.collection.handles.list.len() {
        let collection = &state.collection.handles.list[index];
        if !collection.destroyed && collection.root == id {
            defer(state, ROOT_DESTROYED, index + 1, id, None);
        }
    }
}

/// The instances a collection's queued jobs name (lua::raw::collect keeps them alive)
pub(crate) fn visit_instance_refs(state: &State, visit: &mut dyn FnMut(InstanceId)) {
    for collection in &state.collection.handles.list {
        if !collection.destroyed {
            visit(collection.root);
            collection.members.iter().for_each(|&id| visit(id));
        }
        collection.gone.iter().for_each(|&id| visit(id));
    }
}

fn matches_now(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, selectors: &[Selector], root: InstanceId) -> LuaResult<bool> {
    if !below(&engine.state.borrow(), id, root) {
        return Ok(false);
    }
    query::matches(lua, engine, id, selectors, root)
}

fn call(lua: &Lua, engine: &Rc<Engine>, callback: &LuaFunction, args: LuaMultiValue) -> LuaResult<()> {
    let thread = lua.create_thread(callback.clone())?;
    crate::scheduler::resume(lua, engine, &thread, args);
    Ok(())
}

fn one(lua: &Lua, engine: &Engine, id: InstanceId) -> LuaResult<LuaMultiValue> {
    Ok(LuaMultiValue::from_vec(vec![instance_value(lua, engine, id)?]))
}

/// A queued collection job (scheduler.rs run_job)
pub(crate) fn run_job(lua: &Lua, engine: &Rc<Engine>, instance: InstanceId, args: &[Value]) -> LuaResult<()> {
    let (Some(Value::Number(kind)), Some(Value::Number(id))) = (args.first(), args.get(1)) else { return Ok(()) };
    let (kind, index) = (*kind, *id as usize - 1);
    let (selectors, root, destroyed) = {
        let state = engine.state.borrow();
        let Some(collection) = state.collection.handles.list.get(index) else { return Ok(()) };
        (collection.selectors.clone(), collection.root, collection.destroyed)
    };
    if kind == DESTROYED {
        let (gone, removed) = {
            let mut state = engine.state.borrow_mut();
            let collection = &mut state.collection.handles.list[index];
            (std::mem::take(&mut collection.gone), collection.removed.clone())
        };
        if let Some(removed) = removed {
            for id in gone {
                call(lua, engine, &removed, one(lua, engine, id)?)?;
            }
        }
        return Ok(());
    }
    if destroyed {
        return Ok(());
    }
    match kind {
        ROOT_DESTROYED => {
            let mut state = engine.state.borrow_mut();
            let handles = &mut state.collection.handles;
            let collection = &mut handles.list[index];
            collection.destroyed = true;
            collection.members.clear();
            collection.held.clear();
            handles.live -= 1;
        }
        FIRST_ADDED => {
            let (members, added) = {
                let state = engine.state.borrow();
                let collection = &state.collection.handles.list[index];
                (collection.members.clone(), collection.added.clone())
            };
            if let Some(added) = added {
                for id in members {
                    if engine.state.borrow().collection.handles.list[index].held.contains(&id) && matches_now(lua, engine, id, &selectors, root)? {
                        call(lua, engine, &added, one(lua, engine, id)?)?;
                    }
                }
            }
        }
        CHANGED => {
            let Some(Value::String(property)) = args.get(2) else { return Ok(()) };
            let (held, changed) = {
                let state = engine.state.borrow();
                let collection = &state.collection.handles.list[index];
                (collection.held.contains(&instance), collection.changed.clone())
            };
            if let Some(changed) = changed
                && held
                && matches_now(lua, engine, instance, &selectors, root)?
            {
                let args = LuaMultiValue::from_vec(vec![instance_value(lua, engine, instance)?, LuaValue::String(lua.create_string(property.as_bytes())?)]);
                call(lua, engine, &changed, args)?;
            }
        }
        _ => {
            // (CHECK, CHECK_SUBTREE: the instance, and below it for a move, joins or leaves)
            let nodes = {
                let state = engine.state.borrow();
                let mut nodes = vec![instance];
                if kind == CHECK_SUBTREE {
                    nodes.extend(state.dom.descendants(instance));
                }
                nodes
            };
            let joinable = !matches!(args.get(2), Some(Value::Bool(false)));
            let mut joined = Vec::new();
            let mut left = Vec::new();
            for node in nodes {
                let now = matches_now(lua, engine, node, &selectors, root)? && (joinable || node != instance || engine.state.borrow().collection.handles.list[index].held.contains(&node));
                let mut state = engine.state.borrow_mut();
                let collection = &mut state.collection.handles.list[index];
                let held = collection.held.contains(&node);
                if held && !now {
                    collection.held.remove(&node);
                    collection.members.retain(|&member| member != node);
                    left.push(node);
                } else if !held && now {
                    collection.held.insert(node);
                    collection.members.push(node);
                    joined.push(node);
                }
            }
            let (added, removed) = {
                let state = engine.state.borrow();
                let collection = &state.collection.handles.list[index];
                (collection.added.clone(), collection.removed.clone())
            };
            if let Some(removed) = removed {
                for id in left {
                    call(lua, engine, &removed, one(lua, engine, id)?)?;
                }
            }
            if let Some(added) = added {
                for id in joined {
                    call(lua, engine, &added, one(lua, engine, id)?)?;
                }
            }
        }
    }
    Ok(())
}

fn gone() -> Flow {
    Flow::Error(crate::ScriptError::plain("Collection does not exist"))
}

/// The live collection `id` (a handle's), else "Collection does not exist"
fn live(state: &State, id: usize) -> Result<&Collection, Flow> {
    state.collection.handles.list.get(id.wrapping_sub(1)).filter(|collection| !collection.destroyed).ok_or_else(gone)
}

/// CreateCollection(query, root) past its casts: the collection's id, its first members found as QueryDescendants finds them
fn create(lua: &Lua, engine: &Rc<Engine>, query_text: &[u8], root: InstanceId) -> Result<usize, Flow> {
    let selectors = match query::parse_collection(query_text) {
        Ok(selectors) => Rc::new(selectors),
        Err(message) => return raise_plain(message),
    };
    let members = query::collect_once(lua, engine, root, &selectors)?;
    let (names, properties, attributes) = query::compared(&selectors);
    let mut watched: Vec<NameId> = properties.iter().filter_map(|name| engine.reflection.name_id(name)).collect();
    if names {
        watched.push(K::Name as NameId);
    }
    let mut state = engine.state.borrow_mut();
    let handles = &mut state.collection.handles;
    handles.list.push(Collection {
        query: query_text.to_vec(),
        selectors,
        root,
        held: members.iter().copied().collect(),
        members,
        destroyed: false,
        gone: Vec::new(),
        added: None,
        removed: None,
        changed: None,
        properties: watched,
        attributes: attributes.into_iter().map(String::into_bytes).collect(),
    });
    handles.live += 1;
    let id = handles.list.len();
    defer(&mut state, FIRST_ADDED, id, root, None);
    Ok(id)
}

fn count(lua: &Lua, engine: &Rc<Engine>, id: usize) -> Result<usize, Flow> {
    let (members, selectors, root) = {
        let state = engine.state.borrow();
        let collection = live(&state, id)?;
        (collection.members.clone(), collection.selectors.clone(), collection.root)
    };
    let mut count = 0;
    for member in members {
        if matches_now(lua, engine, member, &selectors, root)? {
            count += 1;
        }
    }
    Ok(count)
}

fn destroy(engine: &Engine, id: usize) -> Result<(), Flow> {
    let mut state = engine.state.borrow_mut();
    live(&state, id)?;
    let handles = &mut state.collection.handles;
    let collection = &mut handles.list[id - 1];
    collection.destroyed = true;
    collection.gone = std::mem::take(&mut collection.members);
    collection.held.clear();
    handles.live -= 1;
    let root = collection.root;
    defer(&mut state, DESTROYED, id, root, None);
    Ok(())
}

/// The collections `instance` is a member of now, in the order they were made
fn collections_of(lua: &Lua, engine: &Rc<Engine>, instance: InstanceId) -> Result<Vec<usize>, Flow> {
    let candidates: Vec<(usize, Rc<Vec<Selector>>, InstanceId)> = {
        let state = engine.state.borrow();
        state
            .collection
            .handles
            .list
            .iter()
            .enumerate()
            .filter(|(_, collection)| !collection.destroyed && collection.held.contains(&instance))
            .map(|(index, collection)| (index + 1, collection.selectors.clone(), collection.root))
            .collect()
    };
    let mut found = Vec::new();
    for (id, selectors, root) in candidates {
        if matches_now(lua, engine, instance, &selectors, root)? {
            found.push(id);
        }
    }
    Ok(found)
}

/// An answer for the stubs: (value, nil), or (nil, the error's text) for Luau to raise as the engine raises (no position)
fn answer<T: IntoLua>(lua: &Lua, result: Result<T, Flow>) -> LuaResult<(LuaValue, LuaValue)> {
    match result {
        Ok(value) => Ok((value.into_lua(lua)?, LuaValue::Nil)),
        Err(Flow::Error(error)) => Ok((LuaValue::Nil, LuaValue::String(lua.create_string(&error.message)?))),
        Err(Flow::Raised(value)) => Ok((LuaValue::Nil, LuaValue::String(lua.create_string(Flow::raised_text(&value))?))),
        Err(Flow::Fail(error)) => Err(error),
        Err(Flow::Yield) => Err(LuaError::runtime("attempt to yield across metamethod/C-call boundary")),
    }
}

/// The stubs' collection functions (lua/stubs.rs load), each answering (value, nil) or (nil, error) for services.luau to raise:
/// R.CollectionCreate(query, root) -> id, R.CollectionCount(id), R.CollectionQuery(id), R.CollectionRoot(id), R.CollectionDestroy(id),
/// R.CollectionCallback(id, name, function), and R.CollectionsOf(instance) -> { id }. A destroyed collection's error is "Collection
/// does not exist".
pub(crate) fn install(lua: &Lua, engine: &Rc<Engine>, r: &LuaTable) -> LuaResult<()> {
    let e = engine.clone();
    r.set(
        "CollectionCreate",
        crate::lua::lua_fn(lua, move |lua, (text, root): (LuaString, LuaValue)| {
            let Some(root) = crate::lua::instance_id(&root) else { return answer(lua, raise_plain::<usize>("Unable to cast value to Object")) };
            answer(lua, create(lua, &e, &text.as_bytes(), root))
        })?,
    )?;
    let e = engine.clone();
    r.set("CollectionCount", crate::lua::lua_fn(lua, move |lua, id: usize| answer(lua, count(lua, &e, id)))?)?;
    let e = engine.clone();
    r.set(
        "CollectionQuery",
        crate::lua::lua_fn(lua, move |lua, id: usize| {
            let text = live(&e.state.borrow(), id).map(|collection| collection.query.clone());
            answer(lua, text.map(|text| lua.create_string(text)).and_then(|made| made.map_err(Flow::Fail)))
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "CollectionRoot",
        crate::lua::lua_fn(lua, move |lua, id: usize| {
            let root = live(&e.state.borrow(), id).map(|collection| collection.root);
            answer(lua, root.and_then(|root| instance_value(lua, &e, root).map_err(Flow::Fail)))
        })?,
    )?;
    let e = engine.clone();
    r.set("CollectionDestroy", crate::lua::lua_fn(lua, move |lua, id: usize| answer(lua, destroy(&e, id).map(|()| LuaValue::Nil)))?)?;
    let e = engine.clone();
    r.set(
        "CollectionCallback",
        crate::lua::lua_fn(lua, move |lua, (id, name, function): (usize, String, LuaFunction)| {
            let mut state = e.state.borrow_mut();
            if let Err(error) = live(&state, id) {
                return answer(lua, Err::<LuaValue, _>(error));
            }
            let collection = &mut state.collection.handles.list[id - 1];
            match name.as_str() {
                "OnAdded" => collection.added = Some(function),
                "OnRemoved" => collection.removed = Some(function),
                "OnChanged" => collection.changed = Some(function),
                // (OnItemAdded and OnItemRemoved take a function and were not seen to run)
                _ => {}
            }
            answer(lua, Ok(LuaValue::Nil))
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "CollectionsOf",
        crate::lua::lua_fn(lua, move |lua, instance: LuaValue| {
            let Some(instance) = crate::lua::instance_id(&instance) else { return answer(lua, Ok(lua.create_table()?)) };
            let ids = collections_of(lua, &e, instance);
            answer(lua, ids.and_then(|ids| lua.create_sequence_from(ids).map_err(Flow::Fail)))
        })?,
    )?;
    Ok(())
}
