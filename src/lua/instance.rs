//! Instance userdata: `__index` / `__newindex` (port of instance.luau InstanceMeta), the base
//! Instance behaviours (Methods, Getters, Setters of class Instance) and the operations that run
//! scripts (Destroy, WaitForChild).

use std::{cell::Cell, rc::Rc};

use mlua::prelude::*;

use super::{
    Args, Flow, Ret, convert, encode, instance_id, instance_value, none, raise, raise_at_caller, raw, signal_value, typeof_value,
    values, wrapped,
};
use crate::{
    ClassId, Engine, InstanceId, K, NameId, State,
    dom::flags,
    instance::{self as core, Member},
    project,
    scheduler::{self, JobTarget, TimerKind},
    services::{Behaviors, GetterFn, MethodFn, SetterFn, StringParam, strings},
    signal::{Handler, SignalId},
    value::{Str, Value},
};

fn key_text(key: &LuaValue) -> String {
    match key {
        LuaValue::String(s) => s.to_string_lossy(),
        LuaValue::Number(n) => number_key_text(*n),
        LuaValue::Integer(n) => n.to_string(),
        LuaValue::Boolean(b) => b.to_string(),
        LuaValue::Nil => "nil".into(),
        other => other.type_name().into(),
    }
}

pub fn class_name(engine: &Engine, id: InstanceId) -> String {
    let class = engine.state.borrow().dom.at(id).class;
    engine.reflection.class(class).name.to_string()
}

/// The instance's full name, at what a client spends building it (cost.rs full_name): GetFullName's, and an error's that names it.
pub fn full_name(engine: &Engine, id: InstanceId) -> String {
    let (name, names) = engine.state.borrow().dom.full_name_counted(&engine.reflection, id);
    charge_work(engine, crate::cost::full_name(names, name.len()));
    name
}

/// IsDescendantOf's answer (IsAncestorOf's the other way round): a script's call walks the parents and pays for each, as a client's
/// does (cost.rs ANCESTOR_STEP); the engine's own Luau, in a call it prices whole (the clock muted: services.luau's PivotTo asks
/// whether a part is in the Workspace), reads the homes the dom keeps: one lookup at any depth and below any number of
/// WorldModels (dom.rs Dom::is_descendant_of_home).
pub fn descendant_of(engine: &Engine, id: InstanceId, ancestor: InstanceId) -> bool {
    let state = engine.state.borrow();
    if state.sched.muted.get() > 0 {
        return state.dom.is_descendant_of_home(id, ancestor);
    }
    let (result, steps) = state.dom.is_descendant_of_steps(id, ancestor);
    drop(state);
    charge_ancestors(engine, steps);
    result
}

/// What a call's walk of `steps` ancestors costs beyond its price (cost.rs ANCESTOR_STEP): IsDescendantOf, IsAncestorOf and the
/// FindFirstAncestor finds.
pub fn charge_ancestors(engine: &Engine, steps: usize) {
    charge_work(engine, steps as f64 * crate::cost::ANCESTOR_STEP);
}

fn charge_work(engine: &Engine, seconds: f64) {
    if seconds > 0.0 {
        engine.state.borrow().sched.charge(seconds);
        crate::boundary::add_cost(seconds);
    }
}

fn not_a_member(engine: &Engine, id: InstanceId, key: &LuaValue) -> Flow {
    Flow::Error(crate::ScriptError::plain(format!(
        "{} is not a valid member of {} \"{}\"",
        key_text(key),
        class_name(engine, id),
        full_name(engine, id)
    )))
}

/// The (cached) script function of a behaviour method (shared by every class inheriting it).
fn method_function(lua: &Lua, engine: &Rc<Engine>, class: ClassId, name: NameId) -> LuaResult<LuaValue> {
    let function = method_function_uncached(lua, engine, class, name)?;
    if let LuaValue::Function(function) = &function {
        raw::remember_index_function(engine, class, name, function)?;
    }
    Ok(function)
}

/// The function a method call (`instance:Name(...)`) runs: a Rust behaviour's protocol function
/// (the raw namecall checks its results), a Luau behaviour, or the unimplemented stand-in.
/// `None`: not a method.
pub fn call_function(lua: &Lua, engine: &Rc<Engine>, class: ClassId, name: NameId) -> LuaResult<Option<LuaFunction>> {
    if !matches!(engine.resolve_member(class, name), Member::Method { .. }) {
        return Ok(None);
    }
    let found = engine.behaviors.borrow().find_method_or_alias(&engine.reflection, class, name).map(|(owner, f)| (owner, f.clone()));
    let Some((owner, method)) = found else {
        return match unimplemented_function(lua, engine, class, name)? {
            LuaValue::Function(function) => Ok(Some(function)),
            _ => Ok(None),
        };
    };
    let method = match method {
        crate::services::Method::Lua(function) => return lua_method(lua, engine, owner, name, function).map(Some),
        crate::services::Method::Rust(method) => method,
    };
    if let Some(existing) = engine.lua.borrow().call_methods.get(&(owner, name)) {
        return Ok(Some(existing.clone()));
    }
    let e = engine.clone();
    let method_name = engine.reflection.name(name).to_string();
    let function = super::create_boxed_fn(lua, Box::new(move |lua, args: LuaMultiValue| {
        let args = Args::new(args);
        let outcome = match args.instance(0) {
            Some(id) => {
                project::before_method(&mut e.state.borrow_mut(), &e.reflection, id, name);
                reveal_arguments(&e, &args);
                method(lua, &e, id, &args)
            }
            None => raise(format!("Expected ':' not '.' calling member function {method_name}")),
        };
        encode(lua, outcome)
    }))?;
    engine.lua.borrow_mut().call_methods.insert((owner, name), function.clone());
    Ok(Some(function))
}

fn method_function_uncached(lua: &Lua, engine: &Rc<Engine>, class: ClassId, name: NameId) -> LuaResult<LuaValue> {
    let found = engine.behaviors.borrow().find_method_or_alias(&engine.reflection, class, name).map(|(owner, f)| (owner, f.clone()));
    let Some((owner, method)) = found else {
        return unimplemented_function(lua, engine, class, name);
    };
    let method = match method {
        crate::services::Method::Lua(function) => return lua_method(lua, engine, owner, name, function).map(LuaValue::Function),
        crate::services::Method::Rust(method) => method,
    };
    if let Some(existing) = engine.lua.borrow().methods.get(&(owner, name)) {
        return Ok(LuaValue::Function(existing.clone()));
    }
    let e = engine.clone();
    let method_name = engine.reflection.name(name).to_string();
    // (self must be an instance of the class that declares the method, a subclass's too: measured in a playing client, Studio
    // 0.748, Camera.GetRoll(workspace), Part.GetMass(workspace), game.GetService(workspace, "Players") and workspace.Raycast(game,
    // ...) raise as a missing self does, before the arguments are looked at, while Part.GetMass(aMeshPart),
    // Model.GetPivot(aPart), workspace.Raycast(aWorldModel, ...) and game.FindFirstChild(workspace, "x") run)
    let declaring = engine.reflection.declaring_class(owner, name);
    let label = method_name.clone();
    let function = wrapped(lua, engine, &label, move |lua, args| {
        let args = Args::new(args);
        let Some(id) = args.instance(0) else {
            return raise(format!("Expected ':' not '.' calling member function {method_name}"));
        };
        let class = e.state.borrow().dom.get(id).map(|instance| instance.class);
        if class.is_some_and(|class| !e.reflection.class(class).is_a(declaring)) {
            return raise(format!("Expected ':' not '.' calling member function {method_name}"));
        }
        project::before_method(&mut e.state.borrow_mut(), &e.reflection, id, name);
        reveal_arguments(&e, &args);
        method(lua, &e, id, &args)
    })?;
    engine.lua.borrow_mut().methods.insert((owner, name), function.clone());
    Ok(LuaValue::Function(function))
}

/// A behaviour method in the engine's Luau (services.luau, server.luau, a layer's) as scripts call it, read as a value or with
/// `:` alike: one C function named as the method over it (lua/cfunction.rs over_luau), as a client's method is one C function
/// (measured, Studio 0.747: `debug.info` gives `[C]` and the method's name, and an error raised inside has no frame of its own:
/// tools/parity/probes/engine_functions.luau). The Luau computes an error's level past that C function (services.luau
/// ScriptLevel). A function that is one of the engine's C functions already is used as it is.
fn lua_method(lua: &Lua, engine: &Rc<Engine>, owner: ClassId, name: NameId, function: LuaFunction) -> LuaResult<LuaFunction> {
    let body = function.to_pointer() as usize;
    if let Some((existing, wrapped)) = engine.lua.borrow().lua_methods.get(&(owner, name))
        && *existing == body
    {
        return Ok(wrapped.clone());
    }
    // SAFETY: the function is pushed as the argument and replaced by the one result
    let is_engine = unsafe {
        lua.exec_raw::<bool>(&function, |state| {
            let engine = super::cfunction::is_engine_function(state, -1);
            mlua::ffi::lua_settop(state, 0);
            mlua::ffi::lua_pushboolean(state, engine as std::ffi::c_int);
        })?
    };
    let wrapped = if is_engine {
        function
    } else {
        // (checking its self first, as the Rust methods do: cfunction::over_luau_method)
        super::cfunction::over_luau_method(lua, engine.reflection.name(name), engine.reflection.declaring_class(owner, name), function)?
    };
    engine.lua.borrow_mut().lua_methods.insert((owner, name), (body, wrapped.clone()));
    Ok(wrapped)
}

/// A reflected method no behaviour implements: records the call and returns nil.
pub(crate) fn unimplemented_function(lua: &Lua, engine: &Rc<Engine>, class: ClassId, name: NameId) -> LuaResult<LuaValue> {
    if let Some(existing) = engine.lua.borrow().unimplemented.get(&(class, name)) {
        return Ok(LuaValue::Function(existing.clone()));
    }
    let e = engine.clone();
    let class_label = engine.reflection.class(class).name.to_string();
    let member = engine.reflection.name(name).to_string();
    let label = member.clone();
    let function = super::named(lua, &label, super::cfunction::Kind::Library, super::create_boxed_void(lua, Box::new(move |lua| {
        e.missing(Some(lua), &class_label, &member);
        Ok(())
    }))?)?;
    engine.lua.borrow_mut().unimplemented.insert((class, name), function.clone());
    Ok(LuaValue::Function(function))
}

/// Hooks.Materialize (Studio-only children, once per instance) before children are looked up.
pub fn materialize(engine: &Engine, id: InstanceId) {
    // (a loopback pair's two sides make the same Studio-only instances and pair them: loopback/lazy.rs)
    let before = if engine.loopback.borrow().is_some() { engine.state.borrow().dom.get(id).map(|instance| instance.children.len()) } else { None };
    #[cfg(feature = "bundles")]
    project::model_assets::materialize(engine, id);
    {
        let mut state = engine.state.borrow_mut();
        project::materialize(&mut state, &engine.reflection, id);
    }
    if let Some(before) = before {
        crate::loopback::lazy::materialized(engine, id, before);
    }
}

/// A manifest bundle's placeholder ([`flags::BUNDLE_ROOT`]) a script is about to find, test or touch
/// becomes the bundle's instance first (project::model_assets::reveal). Call it with the state
/// unborrowed, for an instance whose flag is set.
pub fn reveal_bundle(engine: &Engine, id: InstanceId) {
    #[cfg(feature = "bundles")]
    project::model_assets::reveal(engine, id);
    #[cfg(not(feature = "bundles"))]
    let _ = (engine, id);
}

/// The bundle placeholders ([`flags::BUNDLE_ROOT`]) among an engine method's arguments become their bundles' instances first, as
/// a script's touch makes them: the method reads or writes the instance the game has there (CollectionService:AddTag,
/// TweenService:Create, ...)
fn reveal_arguments(engine: &Engine, args: &Args) {
    for value in args.0.iter().skip(1) {
        if let Some(id) = raw::instance_id(value)
            && engine.state.borrow().dom.get(id).is_some_and(|instance| instance.has(flags::BUNDLE_ROOT))
        {
            reveal_bundle(engine, id);
        }
    }
}

/// A child a script looked up by name and did not find that a bundle mount makes when asked for: a manifest alias's copy
/// (project::model_assets::requested).
fn made_on_request(engine: &Engine, id: InstanceId, name: &[u8]) -> Option<InstanceId> {
    #[cfg(feature = "bundles")]
    return project::model_assets::requested(engine, id, name);
    #[cfg(not(feature = "bundles"))]
    {
        let _ = (engine, id, name);
        None
    }
}

/// Reached: a script found `child` by name (a bundle's placeholder becomes the bundle's instance;
/// script access to Studio-only placeholders is noted).
pub(super) fn reached(engine: &Engine, child: InstanceId) -> InstanceId {
    if engine.state.borrow().dom.at(child).has(flags::BUNDLE_ROOT) {
        reveal_bundle(engine, child);
    }
    let mut state = engine.state.borrow_mut();
    if state.dom.at(child).has(flags::STUDIO_ONLY) {
        project::placeholder_used(&mut state, &engine.reflection, child);
    }
    child
}

/// The first child of `id` (made: [`materialize`] first) that `matches`, each bundle placeholder
/// among them made its bundle's instance before it is tested (a class or value test sees the
/// instance the game has there).
pub fn find_child_where(engine: &Engine, id: InstanceId, matches: impl Fn(&State, InstanceId) -> bool) -> Option<InstanceId> {
    let mut from = 0;
    loop {
        let (index, placeholder) = {
            let state = engine.state.borrow();
            let mut found = None;
            for (index, &child) in state.dom.at(id).children.iter().enumerate().skip(from) {
                if state.dom.at(child).has(flags::BUNDLE_ROOT) {
                    found = Some((index, child));
                    break;
                }
                if matches(&state, child) {
                    return Some(child);
                }
            }
            found?
        };
        reveal_bundle(engine, placeholder);
        if matches(&engine.state.borrow(), placeholder) {
            return Some(placeholder);
        }
        from = index + 1;
    }
}

/// The text a number key of an instance names (`part[5]`, `part[1e20]`): its shortest round-trip digits, written out from 1e-6 to
/// below 1e21 (`0.3333333333333333`, `100000000000000000000`) and with an exponent of at least two digits outside
/// (`1e-07`, `-1.5e-10`, `1e+21`, `1e+300`), and `inf`, `-inf`, `nan` and `-0` (measured in a playing client and a server, Studio
/// 0.747: `Folder[5]` finds a child named "5", `part[1e20]` raises `100000000000000000000 is not a valid member of Part "..."`;
/// tools/parity/probes/instance_api_texts.luau).
fn number_key_text(number: f64) -> String {
    if number.is_nan() {
        return "nan".into();
    }
    if number.is_infinite() || number == 0.0 {
        return format!("{number}");
    }
    let scientific = format!("{number:e}");
    let (digits, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if (-6..21).contains(&exponent) {
        return format!("{number}");
    }
    format!("{digits}e{}{:02}", if exponent < 0 { '-' } else { '+' }, exponent.abs())
}

/// `instance[key]` (InstanceMeta.__index): a member by its exact name, else a child by that name, else (a client's rule,
/// docs/engine.md "Member names") the member the key names with its first letter upper-cased. A number key names the member or
/// child of its text, and one that names nothing raises `5 is not a valid member of Part "..."` without a position; any other
/// key that is no string raises Luau's `invalid argument #2 (string expected, got boolean)` at the calling line (measured,
/// Studio 0.747, a client and a server: tools/parity/probes/instance_api_texts.luau).
pub fn get(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, key: &LuaValue) -> Result<LuaValue, Flow> {
    let number_text;
    let key_string;
    let key_bytes: &[u8] = match key {
        LuaValue::String(text) => {
            key_string = text.as_bytes();
            &key_string
        }
        LuaValue::Number(number) => {
            number_text = number_key_text(*number);
            number_text.as_bytes()
        }
        LuaValue::Integer(number) => {
            number_text = number.to_string();
            number_text.as_bytes()
        }
        _ => return raise_at_caller(format!("invalid argument #2 (string expected, got {})", typeof_value(engine, key))),
    };
    if engine.state.borrow().dom.at(id).has(flags::BUNDLE_ROOT) {
        reveal_bundle(engine, id);
    }
    let class = engine.state.borrow().dom.at(id).class;
    if let Some(name) = engine.reflection.name_id_bytes(&key_bytes)
        && let Some(value) = get_member(lua, engine, id, class, name)
    {
        return value;
    }
    materialize(engine, id);
    let child = engine.state.borrow().dom.find_child(id, &key_bytes);
    if let Some(child) = child {
        reached(engine, child);
        return Ok(instance_value(lua, engine, child)?);
    }
    if let Some(child) = made_on_request(engine, id, &key_bytes) {
        return Ok(instance_value(lua, engine, child)?);
    }
    if let Some(child) = super::globals::missing_child(lua, engine, id, &key_bytes)? {
        return Ok(instance_value(lua, engine, child)?);
    }
    if let Some(name) = engine.reflection.upper_first_id(&key_bytes)
        && let Some(value) = get_member(lua, engine, id, class, name)
    {
        return value;
    }
    Err(not_a_member(engine, id, key))
}

/// A read of member `name` of an instance of `class`; `None` when the class has no such member a script can read.
fn get_member(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, class: ClassId, name: NameId) -> Option<Result<LuaValue, Flow>> {
    let member = engine.resolve_member(class, name);
    // (a deprecated lower-case property reads the property it stands for)
    let name = engine.member_name(class, name);
    Some(match member {
        Member::Property { getter: true, .. } => {
            let getter = engine.behaviors.borrow().find_getter(&engine.reflection, class, name).cloned()?;
            getter(lua, engine, id)
        }
        Member::Property { .. } => read_property(lua, engine, id, name),
        Member::Method { .. } => method_function(lua, engine, class, name).map_err(Flow::from),
        Member::Event => {
            let signal = core::event_signal(&mut engine.state.borrow_mut(), &engine.reflection, id, name);
            signal_value(lua, engine, signal).map_err(Flow::from)
        }
        // (the member's own name, also when the key spelled it with a lower-case first letter: measured, `onInvoke`)
        Member::Callback => raise(format!(
            "{} is a callback member of {}; you can only set the callback value, get is not available",
            engine.reflection.name(name),
            class_name(engine, id)
        )),
        Member::None => return None,
    })
}

/// The error a client raises for a key that names nothing on an instance: no member, no child (and for a method call, no
/// method), with the key as the script wrote it.
pub fn not_a_member_message(engine: &Engine, id: InstanceId, key: &[u8]) -> String {
    format!("{} is not a valid member of {} \"{}\"", String::from_utf8_lossy(key), class_name(engine, id), full_name(engine, id))
}

/// A property read. Each read of a datatype value other than a Vector3 (a Color3, CFrame, UDim2, Vector2, UDim, Rect,
/// NumberRange, NumberSequence, ColorSequence, PhysicalProperties, Ray, ...) is a userdata of its own: not rawequal to another
/// read's nor to the value written, so a table keyed by one read does not find the next (measured in a playing client and a
/// server, Studio 0.747: tools/parity/probes/value_read_identity.luau). A Vector3 is a value.
pub(crate) fn read_property(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, name: NameId) -> Result<LuaValue, Flow> {
    let value = engine.state.borrow().dom.raw_get(id, name).cloned();
    let Some(value) = value else { return Ok(LuaValue::Nil) };
    if let Value::Lua(held) = &value
        && let Some(copy) = copied_datatype(lua, held)?
    {
        return Ok(copy);
    }
    Ok(convert::to_lua(lua, engine, &value)?)
}

/// A copy of a datatype userdata a script wrote and a property keeps as it was given (a PhysicalProperties, a Ray, Faces, Axes,
/// a Region3, the int16 vectors: kept as the script's userdata, since rbx-dom's conversion would round a Ray's vectors), so each
/// read hands out a new value as a client's does; None for any other value.
pub fn copied_datatype(lua: &Lua, value: &LuaValue) -> LuaResult<Option<LuaValue>> {
    use lune_roblox::datatypes::{create, types as lune};
    let LuaValue::UserData(ud) = value else { return Ok(None) };
    macro_rules! copied {
        ($(($kind:ty, $ctor:ident)),*) => {$(
            if let Ok(held) = ud.borrow::<$kind>() {
                let copy = *held;
                drop(held);
                return Ok(Some(LuaValue::UserData(create::$ctor(lua, copy)?)));
            }
        )*};
    }
    copied!(
        (lune::PhysicalProperties, physical_properties),
        (lune::Ray, ray),
        (lune::Faces, faces),
        (lune::Axes, axes),
        (lune::Region3, region3),
        (lune::Region3int16, region3int16),
        (lune::Vector2int16, vector2int16),
        (lune::Vector3int16, vector3int16)
    );
    Ok(None)
}

/// `instance[key] = value` (InstanceMeta.__newindex).
pub fn assign(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, key: &LuaValue, value: &LuaValue) -> Result<(), Flow> {
    if engine.state.borrow().dom.at(id).has(flags::BUNDLE_ROOT) {
        reveal_bundle(engine, id);
    }
    let class = engine.state.borrow().dom.at(id).class;
    let name = match key {
        LuaValue::String(s) => engine.reflection.name_id_bytes(&s.as_bytes()),
        // (a write's key that is no string raises Luau's argument error at the calling line, a number's too, where a read takes the
        // number's text: measured, Studio 0.747, a client and a server, tools/parity/probes/instance_api_texts.luau)
        _ => return raise_at_caller(format!("invalid argument #2 (string expected, got {})", typeof_value(engine, key))),
    };
    let member = name.map_or(Member::None, |name| engine.resolve_member(class, name));
    let name = match (member, name) {
        (Member::None, _) | (_, None) => return Err(not_a_member(engine, id, key)),
        // (a deprecated lower-case property writes the property it stands for)
        (_, Some(name)) => engine.member_name(class, name),
    };
    match member {
        Member::Callback => {
            // (a loopback run refuses the other side's callback, and runs the invokes that waited for this one)
            crate::loopback::remotes::setting_callback(engine, name)?;
            {
                let mut state = engine.state.borrow_mut();
                let callbacks = state.dom.at_mut(id).callbacks.vec_mut();
                match callbacks.iter_mut().find(|(n, _)| *n == name) {
                    Some(entry) => entry.1 = value.clone(),
                    None => callbacks.push((name, value.clone())),
                }
            }
            if engine.loopback.borrow().is_some() && matches!(value, LuaValue::Function(_)) {
                crate::loopback::remotes::callback_set(lua, engine, id)?;
            }
            // (its thread takes the setting thread's memory category at every run, once categories are in use, and a script's thread's
            // owner tag and globals: lua/bindable.rs, scripts.rs)
            if super::globals::memory_categories_set() || crate::scripts::scripted(&lua.current_thread()) {
                super::bindable::callback_set(lua, value)?;
            }
            return Ok(());
        }
        Member::Property { property, setter, .. } => {
            if setter {
                let setter = engine.behaviors.borrow().find_setter(&engine.reflection, class, name).cloned();
                if let Some(setter) = setter {
                    // (a part's version: a setter that stores nothing new on the part leaves it; for any other instance the dom's
                    // count of writes anywhere, which a setter that stores nothing new leaves; a Parent write moves or keeps its parent)
                    let part = engine.reflection.is_a(class, "BasePart");
                    let (version, written, kept) = {
                        let state = engine.state.borrow();
                        let target = if value.is_nil() { Some(None) } else { instance_id(value).map(Some) };
                        let kept = name == K::Parent as NameId && target.is_some_and(|target| state.dom.get(id).is_some_and(|instance| instance.parent == target));
                        (part.then(|| state.dom.at(id).version), state.dom.write_version, kept)
                    };
                    setter(lua, engine, id, value.clone())?;
                    // (a behaviour's setter stores through the engine: priced as a write that changed, a part's with the WorldModels
                    // above it, unless it left the part as it was (the value it holds, a CollisionGroup of "" or of the name it has):
                    // that write changes nothing and pays nothing more, as a playing client's (Studio 0.747, a part below 0, 100 and 300
                    // WorldModels in the Workspace and in ReplicatedStorage: CollisionGroup, CollisionGroupId, Color and Position
                    // written what they hold 0.05 to 1.2 us, where ree charged them 2, 1, 3 and 5 changes' worth, 76 to 380 us at 300).
                    // Another instance's setter that stored nothing new is a write that changed nothing too (Studio 0.747, a headless
                    // client and a server, two sessions: a Camera's FieldOfView written what it holds 0.050-0.052 us in the Workspace and
                    // out of it, a UICorner's CornerRadius 0.036-0.061, where ree charged a changed write, 0.13 to 0.22), and a Parent
                    // written the parent it has costs cost.rs PARENT_KEPT, as on the fast path (lua/raw.rs; its errors raised first)
                    if kept {
                        charge_parent_kept(engine);
                        return Ok(());
                    }
                    let state = engine.state.borrow();
                    let changed = name == K::Parent as NameId
                        || match version {
                            Some(version) => state.dom.get(id).is_none_or(|part| part.version != version),
                            None => state.dom.write_version != written,
                        };
                    let property = engine.reflection.name(name);
                    let (placed, levels) = if changed && part {
                        crate::world::placement(&state.dom, &engine.reflection, engine.world().workspace, id)
                    } else {
                        (changed && camera_placed(engine, &state, class, id), 0)
                    };
                    // (a Parent setter's move pays its levels itself: set_parent; a CollisionGroup setter that changed the part's
                    // CollisionGroupId made one more change: services/physics.rs wrote_id)
                    let id_changed = crate::services::physics::wrote_id(engine, name);
                    let levels = if name == K::Parent as NameId {
                        0.0
                    } else if changed {
                        changed_levels(&state, id, property, id_changed, levels, part_ancestry(engine, &state, class, id))
                    } else {
                        if state.sched.is_muted() {
                            state.sched.touched.borrow_mut().pivot_root.take_if(|root| *root == id);
                        }
                        // (a part's Position written what it holds walks what is above it: cost.rs POSITION_KEPT_LEVEL)
                        if part && property == "Position" && !state.sched.is_muted() {
                            let (above, _) = part_ancestry(engine, &state, class, id).unwrap_or_default();
                            let (_, worlds) = crate::world::placement(&state.dom, &engine.reflection, engine.world().workspace, id);
                            f64::from(above.levels().saturating_sub(1)) * crate::cost::POSITION_KEPT_LEVEL + f64::from(worlds) * crate::cost::POSITION_KEPT_WORLD_LEVEL
                        } else {
                            0.0
                        }
                    };
                    let price = crate::cost::write(&engine.reflection, class, property, changed, placed) + levels + write_touch(&state, id, name, property, changed);
                    state.sched.charge(price);
                    cross_set(engine, &state, id, name, price, changed);
                    return Ok(());
                }
            }
            let Some(property) = property else {
                return raise(format!("Unable to assign property {}. Property is read only", key_text(key)));
            };
            let info = engine.reflection.property(property).clone();
            if info.read_only {
                return raise(format!("Unable to assign property {}. Property is read only", key_text(key)));
            }
            let coerced = convert::coerce(lua, engine, &info, value)?;
            if script_set_property(engine, id, name, coerced) && unemulated_write(engine, class, name) {
                report_unemulated_write(engine, id, name, || super::script_position(lua));
            }
            Ok(())
        }
        _ => Err(not_a_member(engine, id, key)),
    }
}

/// Whether a write of `class`'s property `name` does more in a client than store the value, in a way the engine does not
/// emulate, so that a script's write that changes it reports itself ([`report_unemulated_write`]; lua/raw.rs Write::Watched):
/// a Humanoid's Health. Roblox's Humanoid documentation: a change of Health fires HealthChanged, the Humanoid dies (Died, the
/// Dead state, its joints broken) when Health reaches 0, and Health stays between 0 and MaxHealth; ree has no Humanoid
/// simulation, so none of that happens here and the value is stored as written.
pub fn unemulated_write(engine: &Engine, class: ClassId, name: NameId) -> bool {
    engine.reflection.name(name) == "Health" && engine.reflection.is_a(class, "Humanoid")
}

/// A script's write of an [`unemulated_write`] property changed it (stored as written): what a client does beside it is
/// reported (Output `missing`), at the script's position `at` gives. A Humanoid's Health: HealthChanged for any change, Died at
/// 0 or below, and a value outside 0..MaxHealth (which a client keeps within them) as such.
pub fn report_unemulated_write(engine: &Engine, id: InstanceId, name: NameId, at: impl Fn() -> Option<String>) {
    let (health, max) = {
        let state = engine.state.borrow();
        let number = |name: Option<NameId>| match name.and_then(|name| state.dom.raw_get(id, name)) {
            Some(Value::Number(number)) => *number,
            _ => f64::NAN,
        };
        (number(Some(name)), number(engine.reflection.name_id("MaxHealth")))
    };
    engine.missing_where("Humanoid", "HealthChanged", &at);
    if health <= 0.0 {
        engine.missing_where("Humanoid", "Died", &at);
    }
    if health < 0.0 || health > max {
        engine.missing_where("Humanoid", "Health outside 0..MaxHealth", &at);
    }
}

/// Whether the instance (of `class`) is a part in the Workspace, and how many WorldModels are above it when it is a part (world/mod.rs
/// placement: one lookup, whatever its depth); neither for any other instance.
fn part_placement(engine: &Engine, state: &State, class: ClassId, id: InstanceId) -> (bool, u32) {
    if engine.reflection.is_a(class, "BasePart") { crate::world::placement(&state.dom, &engine.reflection, engine.world().workspace, id) } else { (camera_placed(engine, state, class, id), 0) }
}

/// Whether `id` is a Camera in the Workspace, whose writes cost a client more than a loose camera's (cost.rs camera_write)
fn camera_placed(engine: &Engine, state: &State, class: ClassId, id: InstanceId) -> bool {
    engine.reflection.is_a(class, "Camera") && state.dom.is_descendant_of(id, engine.world().workspace)
}

/// What a change of `property` costs a part below `levels` WorldModels for them (cost.rs world_levels, part_changes;
/// `id_changed`: a CollisionGroup write changed the part's CollisionGroupId). Inside a call the engine's Luau prices whole (the
/// clock muted) the call's own price does not hold them, so they are charged on top of it: the part a PivotTo moves as a
/// script's change would be, each other part it changes one change's worth.
fn changed_levels(state: &State, id: InstanceId, property: &str, id_changed: bool, levels: u32, ancestry: Option<(crate::dom::Ancestry, bool)>) -> f64 {
    // (the WorldModels above, and the levels and Models a part's change walks: cost.rs world_levels, ancestor_levels)
    let above = |changes: u32| {
        (if levels == 0 { 0.0 } else { crate::cost::world_levels(levels, changes) })
            + ancestry.map_or(0.0, |(ancestry, placed)| crate::cost::ancestor_levels(&ancestry, placed, changes))
    };
    if !state.sched.is_muted() {
        return above(crate::cost::part_changes(property, id_changed));
    }
    let root = state.sched.touched.borrow_mut().pivot_root.take_if(|root| *root == id).is_some();
    let changes = if root { crate::cost::part_changes(property, id_changed) } else { 1 };
    let price = above(changes);
    if price > 0.0 {
        state.sched.charge_priced(price);
    }
    0.0
}

/// What is above a part and whether it is in the Workspace, for its change's price (dom.rs Dom::ancestry; cost.rs ancestor_levels);
/// None for any other instance
fn part_ancestry(engine: &Engine, state: &State, class: ClassId, id: InstanceId) -> Option<(crate::dom::Ancestry, bool)> {
    if !engine.reflection.is_a(class, "BasePart") {
        return None;
    }
    let model = engine.reflection.class_id("Model");
    let world_model = engine.reflection.class_id("WorldModel");
    let data_model = engine.reflection.class_id("DataModel");
    let above = state.dom.ancestry(id, |class| {
        let info = engine.reflection.class(class);
        (model.is_some_and(|model| info.is_a(model)) && !world_model.is_some_and(|world| info.is_a(world)), Some(class) == data_model)
    });
    Some((above, state.dom.is_descendant_of_home(id, engine.world().workspace)))
}

/// RawSet with notifications (signals queue; hooks run) on an engine handle.
pub fn set_property(engine: &Engine, id: InstanceId, name: NameId, value: Value) -> bool {
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    core::raw_set(&mut state, &engine.reflection, &hooks, id, name, value)
}

/// What a script's write adds to its price for the cache the engine's write path runs in (crate::cost): the first touch of the
/// instance in the frame (a read's miss), and for a changed write the property's share of a dirtied instance on top of it. A write
/// that changed nothing pays only the first touch. The property's first write in the frame runs the engine's code for it cold
/// (cost.rs FIRST_WRITE), changed or not.
fn write_touch(state: &State, id: InstanceId, name: NameId, property: &str, changed: bool) -> f64 {
    let (class, serial) = (state.dom.at(id).class, state.dom.at(id).serial);
    let mut touched = state.sched.touched.borrow_mut();
    let first = touched.first_use(crate::cost::MemberUse::Write, name);
    let touch = touched.touch(id.index, class, serial);
    if changed && touch > 0.0 {
        let extra = crate::cost::cold_write_extra(touched.kind(class), property);
        first + touch + touched.cold(extra, crate::cost::ColdKey::Instance(id.index, class))
    } else {
        first + touch
    }
}

/// A CFrame a script writes to a part's CFrame or a joint's C0 / C1, as a client stores it: its rotation orthonormalized once it
/// is off by more than the client lets pass (world::frame::stored_rotation). Other values, and CFrames written elsewhere (a
/// model's WorldPivot keeps a scaled rotation, measured), pass as they are.
fn cframe_as_stored(engine: &Engine, id: InstanceId, name: NameId, value: Value) -> Value {
    use rbx_dom_weak::types::Variant;
    let Value::Dom(variant) = &value else { return value };
    let Variant::CFrame(cframe) = &**variant else { return value };
    let class = engine.state.borrow().dom.at(id).class;
    // (a Camera refuses a CFrame that is not finite or has a rotation entry of 1.2 or more: the write keeps what it holds, world/camera.rs)
    if name == crate::world::cframe_name(engine) && engine.reflection.is_a(class, "Camera") && !crate::world::camera::takes_cframe(cframe) {
        return engine.state.borrow().dom.raw_get(id, name).cloned().unwrap_or(value);
    }
    match stored_cframe(&engine.reflection, class, name, cframe) {
        Some(stored) => Value::Dom(Rc::new(Variant::CFrame(stored))),
        None => value,
    }
}

/// A CFrame stored in property `name` of an instance of `class` as a client stores it when a script writes it or the server
/// sends it (loopback/replicate.rs): an Attachment's CFrame always through [`attachment_rotation`], a joint's C0 / C1 and a
/// part's CFrame orthonormalized past their tolerances (world::frame::stored_rotation); None where it is stored as it is
/// (other properties, a rotation within the tolerance)
pub(crate) fn stored_cframe(reflection: &crate::reflection::Reflection, class: crate::reflection::ClassId, name: NameId, cframe: &rbx_dom_weak::types::CFrame) -> Option<rbx_dom_weak::types::CFrame> {
    use rbx_dom_weak::types as dom;
    let property = reflection.name(name);
    let o = &cframe.orientation;
    let rows = [[o.x.x, o.x.y, o.x.z], [o.y.x, o.y.y, o.y.z], [o.z.x, o.z.y, o.z.z]];
    let row = |r: [f32; 3]| dom::Vector3::new(r[0], r[1], r[2]);
    let made = |stored: [[f32; 3]; 3]| dom::CFrame::new(cframe.position, dom::Matrix3::new(row(stored[0]), row(stored[1]), row(stored[2])));
    let is_cframe = property == "CFrame";
    if is_cframe && reflection.is_a(class, "Attachment") {
        return Some(made(attachment_rotation(rows)));
    }
    let joint = matches!(property, "C0" | "C1") && reflection.is_a(class, "JointInstance");
    if !joint && !(is_cframe && reflection.is_a(class, "BasePart")) {
        return None;
    }
    let stored = if joint { crate::world::frame::stored_rotation_within(rows, JOINT_TOLERANCE) } else { crate::world::frame::stored_rotation(rows) };
    (stored != rows).then(|| made(stored))
}

/// How far a joint's C0 / C1 may be off orthonormal (an entry of RᵀR - I) before a write orthonormalizes it, with the same float32
/// Gram-Schmidt a part's CFrame gets past 1e-5 (world::frame::stored_rotation). Measured on a 0.747 playing client and server
/// (tools/parity/probes/joint_c0_tolerance.luau, captured twice on each side): kept with entries up to 4.8e-7 (a scale of 2.5e-7,
/// a column leaning 5e-7), orthonormalized from 5.07e-7 (a column leaning 5.5e-7); cframe_writes.luau's 27 C0 writes, their
/// residues of 1e-13 included, then match to the bit.
const JOINT_TOLERANCE: f32 = 5e-7;

/// The rotation (rows) an Attachment's CFrame write stores, whatever its error: in float32, X is the first column times the inverse
/// of its length, Y the second column less its part along X, normalized the same way, and Z the cross product X x Y, so the
/// written third column is dropped (a client's R12 reads -0 for a rotation about Z, and a scaled one keeps the rounding of X x Y in
/// R22: 1.00000012). Measured, Studio 0.747, a client and a server, tools/parity/probes/cframe_writes.luau: this matches 25 of the
/// 27 writes (a column, a scale and a skew at errors from 0 to 0.1); the scale of 0.1 and the skew of 2e-5 differ in one ulp
/// (the client's reciprocal square root, unmodelled).
fn attachment_rotation(rows: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let column = |k: usize| [rows[0][k], rows[1][k], rows[2][k]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let normalized = |v: [f32; 3]| {
        let inverse = 1.0 / dot(v, v).sqrt();
        [v[0] * inverse, v[1] * inverse, v[2] * inverse]
    };
    let x = normalized(column(0));
    let y0 = column(1);
    let along = dot(x, y0);
    let y = normalized([y0[0] - along * x[0], y0[1] - along * x[1], y0[2] - along * x[2]]);
    let z = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]]
}

/// A part's Size as a script's write leaves it: each axis held from 0.001 to 2048 (convert.rs vector3_for), and a write with an axis
/// that is not a number changes nothing (measured, Studio 0.747, a client and a server: tools/parity/probes/datatype_edges.luau
/// `size`: (nan, inf, -inf) leaves 2048, 2048, 2047.9 as it was); a TrussPart's as [`truss_size`] keeps it.
fn size_as_stored(engine: &Engine, id: InstanceId, name: NameId, value: Value) -> Value {
    if name != K::Size as NameId {
        return value;
    }
    let Value::Vector3(size) = &value else { return value };
    let nan = size.x.is_nan() || size.y.is_nan() || size.z.is_nan();
    let state = engine.state.borrow();
    let class = state.dom.at(id).class;
    if nan && engine.reflection.is_a(class, "BasePart") {
        return state.dom.raw_get(id, name).cloned().unwrap_or(value);
    }
    if !nan && engine.reflection.is_a(class, "TrussPart") {
        let [x, y, z] = truss_size([size.x, size.y, size.z]);
        return Value::Vector3(rbx_dom_weak::types::Vector3::new(x, y, z));
    }
    value
}

/// A TrussPart's Size as a write leaves it: 2 studs on two axes, the other one kept up to 512 studs. The kept axis is the one
/// written other than 2 when only one is (1.9 and 0.001 are kept so), else the largest, the first of equals in X, Y, Z order, at
/// least 2 (measured, Studio 0.747, a client and a server: (4, 6, 8) reads (2, 2, 8), (4, 4, 2) (4, 2, 2), (1, 9, 9) (2, 9, 2),
/// (0.5, 0.5, 0.5) (2, 2, 2), (1.9, 2, 2) as written, (2, 2, 3000) (2, 2, 512); whatever it held before;
/// tools/parity/probes/query_wedges.luau).
fn truss_size(size: [f32; 3]) -> [f32; 3] {
    let mut kept = [2.0f32; 3];
    let other = (0..3).filter(|&axis| size[axis] != 2.0).count();
    if other == 1 {
        let axis = (0..3).find(|&axis| size[axis] != 2.0).unwrap_or(0);
        kept[axis] = size[axis].min(512.0);
    } else if other > 1 {
        let axis = (0..3).fold(0, |best, axis| if size[axis] > size[best] { axis } else { best });
        kept[axis] = size[axis].clamp(2.0, 512.0);
    }
    kept
}

/// A script's property write: [`set_property`], charged to the work clock by what Studio spends on it
/// (crate::cost::write: the property, whether it changed, whether the instance is in the workspace).
pub fn script_set_property(engine: &Engine, id: InstanceId, name: NameId, value: Value) -> bool {
    let value = size_as_stored(engine, id, name, cframe_as_stored(engine, id, name, value));
    // (a client looks a Text write up for translation: AutoLocalize on and a letter in it)
    let client = !engine.options.edit_mode() && !engine.options.server();
    let looked_up = match &value {
        Value::String(text) if crate::cost::text_localization_in(&engine.options.place_prices, engine.options.profile.game.place_id(), 1, client).is_some() && engine.reflection.name(name) == "Text" => {
            let text = String::from_utf8_lossy(text.as_bytes());
            text.chars().any(char::is_alphabetic).then(|| text.chars().count())
        }
        _ => None,
    };
    // (a Player's Character: its events and, on a server, the old character's removal, services/players.rs)
    let character = crate::services::players::is_character(&engine.state.borrow(), id, name);
    let changed = if character { crate::services::players::write_character(engine, id, value) } else { set_property(engine, id, name, value) };
    let state = engine.state.borrow();
    let class = state.dom.at(id).class;
    let (placed, levels) = if changed { part_placement(engine, &state, class, id) } else { (false, 0) };
    let price = crate::cost::write(&engine.reflection, class, engine.reflection.name(name), changed, placed)
        * crate::cost::place_write_scale(&engine.options.place_prices, engine.options.profile.game.place_id(), engine.reflection.name(name), placed);
    let mut paid = price + write_touch(&state, id, name, engine.reflection.name(name), changed);
    if changed {
        paid += changed_levels(&state, id, engine.reflection.name(name), false, levels, part_ancestry(engine, &state, class, id));
    } else if state.sched.is_muted() {
        state.sched.touched.borrow_mut().pivot_root.take_if(|root| *root == id);
    }
    {
        let mut touched = state.sched.touched.borrow_mut();
        // (where the write's price came from: cost.rs write_kinds)
        touched.book_kinds_with(price, || crate::cost::write_kinds(&engine.reflection, class, engine.reflection.name(name), changed, placed));
        touched.count(|ops| {
            ops.writes += 1;
            ops.changed += u64::from(changed);
        });
    }
    // (the frame's first geometry write on a part in the Workspace, and its first switch of an effect, run the engine's code for
    // them cold; an effect switched cold reads more: cost.rs Path::Geometry, Path::Toggle, TOGGLE_COLD)
    if changed {
        let property = engine.reflection.name(name);
        let mut touched = state.sched.touched.borrow_mut();
        let cold = touched.take_last_cold();
        if placed && matches!(property, "CFrame" | "Position" | "Orientation" | "Rotation" | "Size") {
            touched.count(|ops| ops.geometry += 1);
            paid += touched.first_path(crate::cost::Path::Geometry);
        } else if property == "Enabled" && ["ParticleEmitter", "Beam", "Trail"].iter().any(|kind| engine.reflection.is_a(class, kind)) {
            touched.count(|ops| ops.toggles += 1);
            paid += touched.first_path(crate::cost::Path::Toggle)
                + if cold { touched.cold(crate::cost::TOGGLE_COLD, crate::cost::ColdKey::Instance(id.index, class)) } else { 0.0 };
        }
    }
    state.sched.charge(paid);
    cross_set(engine, &state, id, name, paid, changed);
    crate::cost::tally_write(&engine.reflection.class(class).name, engine.reflection.name(name), changed, placed, price);
    if let (true, Some(characters)) = (changed, looked_up) {
        let auto_localize = engine.reflection.name_id("AutoLocalize").and_then(|auto| state.dom.boolean(id, auto)).unwrap_or(true);
        if let (true, Some(lookup)) = (auto_localize, crate::cost::text_localization_in(&engine.options.place_prices, engine.options.profile.game.place_id(), characters, client)) {
            state.sched.charge(lookup);
            crate::boundary::add_cost(lookup);
        }
    }
    changed
}

/// A script's property write, for the boundary profiler (src/boundary.rs): nothing unless profiling, and nothing inside the
/// engine's own Luau pricing a whole call.
fn cross_set(engine: &Engine, state: &State, id: InstanceId, name: NameId, cost: f64, changed: bool) {
    if crate::boundary::active() && !state.sched.is_muted() {
        let class = state.dom.at(id).class;
        let reflection = &engine.reflection;
        crate::boundary::record(crate::boundary::Kind::Set, class, name, id.index, None, cost, changed, || {
            (reflection.class(reflection.declaring_class(class, name)).name.to_string(), reflection.name(name).to_string())
        });
    }
}

/// SetAttribute on an engine handle (after type validation): stores the value, then runs the attribute hooks
/// when it changed.
pub fn set_attribute(engine: &Engine, id: InstanceId, name: Str, value: Value) {
    let mut state = engine.state.borrow_mut();
    let version = state.dom.at(id).version;
    core::set_attribute(&mut state, &engine.reflection, id, name.clone(), value);
    // (the version moves only when the attribute changed)
    if state.dom.at(id).version != version {
        // (the namecall that got here touched the instance first: a cold one pays the dirtied instance's extra, as a property write does)
        let cold = {
            let mut touched = state.sched.touched.borrow_mut();
            let class = state.dom.at(id).class;
            if touched.take_last_cold() {
                let extra = crate::cost::cold_write_extra(touched.kind(class), "SetAttribute");
                touched.cold(extra, crate::cost::ColdKey::Instance(id.index, class))
            } else {
                0.0
            }
        };
        state.sched.charge(crate::cost::ATTRIBUTE_CHANGED + cold);
        let hooks = engine.hooks.borrow();
        if !hooks.attribute_changed.is_empty() {
            let text = String::from_utf8_lossy(name.as_bytes()).into_owned();
            for hook in &hooks.attribute_changed {
                hook(&mut state, id, &text);
            }
        }
    } else {
        state.sched.charge(crate::cost::ATTRIBUTE_SAME);
    }
}

/// SetParent on an engine handle: a part's move pays the WorldModels above it where it went, as its other changes do
/// ([`changed_levels`]), beyond the reparenting's price.
/// A script's Parent write of the parent the instance has (cost.rs PARENT_KEPT): it changes nothing
pub fn charge_parent_kept(engine: &Engine) {
    engine.state.borrow().sched.charge(crate::cost::PARENT_KEPT);
    crate::boundary::add_cost(crate::cost::PARENT_KEPT);
}

pub fn set_parent(engine: &Engine, id: InstanceId, parent: Option<InstanceId>) -> Result<(), Flow> {
    // (a bundle's placeholder that is to hold `id`, bundles waiting in a subtree that enters a world, or ones whose instances carry
    // a tag a CollectionService signal listens for as the subtree enters or leaves the DataModel, import first)
    #[cfg(feature = "bundles")]
    match parent {
        Some(parent) => project::model_assets::before_parent(engine, id, parent),
        None => project::model_assets::before_leaving(engine, id),
    }
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    let moved = state.dom.get(id).is_some_and(|instance| instance.parent != parent);
    core::set_parent(&mut state, &engine.reflection, &hooks, id, parent).map_err(Flow::Error)?;
    if moved && state.dom.get(id).is_some() {
        let (_, levels) = part_placement(engine, &state, state.dom.at(id).class, id);
        let seconds = if levels > 0 { changed_levels(&state, id, "Parent", false, levels, None) } else { 0.0 };
        if seconds > 0.0 {
            state.sched.charge(seconds);
            crate::boundary::add_cost(seconds);
        }
    }
    Ok(())
}

/// Signal.FireImmediate: runs handlers now, each on a fresh thread (engine callbacks that do not
/// defer: RunService frame events).
pub fn fire_immediate(lua: &Lua, engine: &Rc<Engine>, signal: SignalId, args: &[Value]) -> LuaResult<()> {
    fire_immediate_costing(lua, engine, signal, args, 0.0)
}

/// [`fire_immediate`], charging the clock `per_handler` seconds for each handler's dispatch after the first that runs, booked to the
/// handler (cost.rs `FRAME_HANDLER`: what one more handler costs; the first one's dispatch is the frame's). A waiting thread that is
/// cancelled or dead, a handler disconnected before its turn and a parallel one (they run after the serial ones) are not that first.
pub fn fire_immediate_costing(lua: &Lua, engine: &Rc<Engine>, signal: SignalId, args: &[Value], per_handler: f64) -> LuaResult<()> {
    let handlers = {
        let mut state = engine.state.borrow_mut();
        if !state.signals.has_handlers(signal) {
            return Ok(());
        }
        state.signals.take_handlers(signal)
    };
    let mut parallel = Vec::new();
    let mut dispatched = false;
    for handler in handlers {
        // (the first handler that runs is not charged: its dispatch is the frame's)
        let dispatch_price = if dispatched { per_handler } else { 0.0 };
        let mut values = LuaMultiValue::with_capacity(args.len());
        for value in args {
            values.push_back(convert::to_lua(lua, engine, value)?);
        }
        match handler {
            Handler::Thread(thread) => {
                if !scheduler::can_resume(engine, &thread) {
                    continue;
                }
                scheduler::charge_dispatch_to(lua, engine, &thread, dispatch_price, 0);
                dispatched = true;
                scheduler::resume_ready(lua, engine, &thread, values);
            }
            Handler::Connection(connection) => {
                // (a stopped script's connection runs no handler: it disconnects as the signal reaches it, scripts.rs)
                if !crate::scripts::handler_may_run(&mut engine.state.borrow_mut(), connection) {
                    continue;
                }
                let census = crate::census::handler_site(engine, connection);
                let is_parallel = engine.state.borrow().signals.is_parallel(connection);
                let (kept, connector) = if is_parallel { (None, None) } else { engine.state.borrow_mut().signals.take_threads(connection) };
                let maker = crate::scripts::maker(engine, connection);
                let callback = engine.state.borrow_mut().signals.begin_handler(connection);
                if let Some((callback, signal)) = callback {
                    if is_parallel {
                        let thread = lua.create_thread(callback)?;
                        crate::scripts::adopt_if(lua, &thread, maker.as_ref());
                        parallel.push((thread, values));
                        continue;
                    }
                    let sample = scheduler::sample_handler(engine, signal, &callback);
                    let _scope = crate::microprofile::enabled()
                        .then(|| crate::microprofile::scope(scheduler::handler_label(engine, signal, &callback)));
                    let thread = super::globals::handler_thread(lua, kept, connector, maker, callback)?;
                    if let Some(site) = census {
                        crate::census::handler_ran(site, &thread);
                    }
                    // (each handler's dispatch, booked to it, with its run by the site that connected it)
                    dispatched = true;
                    let dispatch = scheduler::DispatchPrice { seconds: dispatch_price, kind: scheduler::DispatchKind::Client };
                    scheduler::run_handler(lua, engine, connection, signal, &thread, values, dispatch);
                    super::globals::handler_ran(engine, connection, &thread);
                    scheduler::end_handler_sample(sample, &thread);
                }
            }
        }
    }
    // (they run at the frame event's parallel point, after its flush: lua/actors.rs)
    for (thread, values) in parallel {
        super::actors::defer_parallel(engine, thread, values);
    }
    Ok(())
}

/// InstanceModule.Destroy.
pub fn destroy(_lua: &Lua, engine: &Rc<Engine>, id: InstanceId) -> Result<(), Flow> {
    // (the frame's first destroy, and of each class, runs the engine's code for it cold; an instance in the Workspace since an
    // earlier frame has render, physics and particle state to undo: cost.rs Path::Destroy, first_destroyed, destroy_live. Priced
    // for the whole subtree before it leaves.)
    {
        let state = engine.state.borrow();
        if state.dom.at(id).has(flags::DESTROYED) {
            return Ok(());
        }
        let mut touched = state.sched.touched.borrow_mut();
        let mut cold = touched.first_path(crate::cost::Path::Destroy);
        for node in std::iter::once(id).chain(state.dom.descendants(id)) {
            cold += touched.destroyed(node.index, state.dom.at(node).class);
            touched.count(|ops| ops.destroyed += 1);
        }
        drop(touched);
        state.sched.charge(cold);
        crate::boundary::add_cost(cold);
    }
    destroy_subtree(engine, id)
}

/// Destroys `id`, then each of its children's subtrees in turn, then finishes `id`: a stack of steps in place of recursion, since a
/// script can build a chain deeper than any thread's stack holds frames for. An error ends the whole destroy where it stands, the
/// instances above it unfinished.
fn destroy_subtree(engine: &Rc<Engine>, id: InstanceId) -> Result<(), Flow> {
    enum Step {
        Destroy(InstanceId, bool),
        /// Its subtree destroyed: the instance's own price (with its extra) and its deferred cleanup
        Finish(InstanceId, f64),
    }
    // (bundles waiting in the subtree whose instances carry a tag a GetInstanceRemovedSignal listens for import first, so it
    // fires for them as the subtree leaves: project/model_assets.rs before_leaving)
    #[cfg(feature = "bundles")]
    project::model_assets::before_leaving(engine, id);
    // Capture DataModel placement before begin_destroy detaches the root so descendants keep their placed status.
    let root_placed = {
        let state = engine.state.borrow();
        state.dom.at(id).parent.is_some_and(|parent| crate::services::in_data_model(&state, parent))
    };
    let mut steps = vec![Step::Destroy(id, root_placed)];
    while let Some(step) = steps.pop() {
        let (id, placed) = match step {
            Step::Destroy(id, placed) => (id, placed),
            Step::Finish(id, extra) => {
                let mut state = engine.state.borrow_mut();
                state.sched.charge(crate::cost::DESTROYED + extra);
                crate::boundary::add_cost(crate::cost::DESTROYED + extra);
                core::finish_destroy(&mut state, id);
                continue;
            }
        };
        let destroying = {
            let state = engine.state.borrow();
            let instance = state.dom.at(id);
            if instance.has(flags::DESTROYED) {
                continue;
            }
            instance.signals.as_ref().and_then(|s| s.event(K::Destroying as NameId))
        };
        // (a GUI object or a part in the DataModel pays more to destroy than its leaving: cost.rs DESTROYED_GUI, DESTROYED_PART)
        let extra = {
            let state = engine.state.borrow();
            let class = state.dom.at(id).class;
            match placed {
                true if engine.reflection.is_a(class, "GuiObject") => crate::cost::DESTROYED_GUI,
                true if engine.reflection.is_a(class, "BasePart") => crate::cost::DESTROYED_PART,
                _ => 0.0,
            }
        };
        // (a client sees another player's character leave before that player's Destroying: services/players.rs)
        {
            let mut state = engine.state.borrow_mut();
            if !crate::services::players::on_server(&state)
                && let Some(parent) = state.dom.at(id).parent
                && crate::services::players::leaving(&state, id, parent)
            {
                let hooks = engine.hooks.borrow();
                crate::services::players::remove_character(&mut state, &engine.reflection, &hooks, id);
            }
        }
        if let Some(signal) = destroying {
            // Deferred like every other event (measured in a playing client, Studio 0.744, SignalBehavior Deferred:
            // tools/parity/probes/hierarchy.luau): the handlers run after Destroy returned and see the instance
            // unparented; each instance's are queued just before its own removal events, a child's after its parent's.
            core::fire(&mut engine.state.borrow_mut(), signal, Vec::new());
        }
        let children = {
            let hooks = engine.hooks.borrow();
            let mut state = engine.state.borrow_mut();
            core::begin_destroy(&mut state, &engine.reflection, &hooks, id)?
        };
        // (the first child on top: each child's subtree is destroyed in turn, then the instance finishes)
        steps.push(Step::Finish(id, extra));
        steps.extend(children.into_iter().rev().map(|child| Step::Destroy(child, placed)));
    }
    Ok(())
}

/// What a PivotTo of `id` adds for the engine's code and the instances it carries being cold (cost.rs Path::Pivot, PIVOT_COLD_PART,
/// PIVOT_COLD_ATTACHED): nothing out of the Workspace. Called right after the call's own touch of `id`, which says whether it was cold.
pub fn pivot_cold(engine: &Engine, id: InstanceId) -> f64 {
    let state = engine.state.borrow();
    let mut touched = state.sched.touched.borrow_mut();
    // (the part it moves, whose write in the engine's Luau pays the WorldModels above it as a script's: changed_levels)
    touched.pivot_root = Some(id);
    let root_cold = touched.take_last_cold();
    if !touched.workspace.is_some_and(|workspace| state.dom.is_descendant_of_home(id, workspace)) {
        return 0.0;
    }
    touched.count(|ops| ops.pivots += 1);
    let mut seconds = touched.first_path(crate::cost::Path::Pivot);
    // (the call touched the root: cold then, and not moved the frame before either)
    if root_cold && !touched.last_cold_was_recent() {
        let class = state.dom.at(id).class;
        let extra = touched.moved_cold(class, crate::cost::PIVOT_COLD_PART, crate::cost::PIVOT_COLD_ATTACHED);
        seconds += touched.cold(extra, crate::cost::ColdKey::Instance(id.index, class));
    }
    for node in state.dom.descendants(id) {
        if touched.mark_moved(node.index) {
            let class = state.dom.at(node).class;
            let extra = touched.moved_cold(class, crate::cost::PIVOT_COLD_PART, crate::cost::PIVOT_COLD_ATTACHED);
            seconds += touched.cold(extra, crate::cost::ColdKey::Instance(node.index, class));
        }
    }
    seconds
}

/// InstanceModule.Clone (materializing the subtree first).
pub fn clone(engine: &Engine, id: InstanceId) -> Option<InstanceId> {
    if matches!(engine.state.borrow().dom.raw_get(id, K::Archivable as NameId), Some(Value::Bool(false))) {
        return None;
    }
    // (the template's root was touched by the call itself: whether that touch found it cold)
    let root_cold = engine.state.borrow().sched.touched.borrow_mut().take_last_cold();
    project::materialize_deep(engine, id);
    let mut state = engine.state.borrow_mut();
    let (copy, pairs) = core::clone_instance_mapped(&mut state, &engine.reflection, id).unzip();
    if let Some(copy) = copy {
        let mut work: f64 = 0.0;
        for node in std::iter::once(copy).chain(state.dom.descendants(copy)) {
            let class = state.dom.at(node).class;
            let price = crate::cost::cloned(&engine.reflection, class);
            work += price;
            // (where each instance's price came from: cost.rs cloned_kinds)
            state.sched.touched.borrow().book_kinds_with(price, || crate::cost::cloned_kinds(&engine.reflection, class));
        }
        // (the frame's first clone, and of each class it holds, runs the engine's code for it cold; a template no code touched this
        // frame is read cold: cost.rs Path::Clone, first_cloned, clone_cold. The copy is made now, so it is hot.)
        {
            let mut touched = state.sched.touched.borrow_mut();
            work += touched.first_path(crate::cost::Path::Clone);
            work += touched.cloned_source(id.index, state.dom.at(id).class, Some(root_cold));
            for node in state.dom.descendants(id) {
                work += touched.cloned_source(node.index, state.dom.at(node).class, None);
            }
            for node in std::iter::once(copy).chain(state.dom.descendants(copy)) {
                touched.mark(node.index);
                touched.count(|ops| ops.cloned += 1);
            }
        }
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
    drop(state);
    // (state an instance keeps outside its properties, copied with no borrow of the engine's state held: a WorldModel's
    // collision groups)
    if let Some(pairs) = pairs {
        crate::services::physics::cloned(engine, &pairs);
    }
    copy
}

fn name_equals(state: &State, id: InstanceId, name: Option<&[u8]>) -> bool {
    name == Some(state.dom.at(id).name.as_bytes())
}

/// A string argument (other types match no name).
fn name_arg(value: &LuaValue) -> Option<LuaString> {
    match value {
        LuaValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// FindFirstChild (recursive: each child's subtree in turn, after the direct children).
pub fn find_first_child(engine: &Engine, id: InstanceId, name: Option<&[u8]>, recursive: bool) -> Option<InstanceId> {
    find_first_child_counting(engine, id, name, recursive, &mut 0)
}

/// [`find_first_child`], adding to `looked` the instances whose names it compared (a script's find pays for them: [`charge_find`])
pub fn find_first_child_counting(engine: &Engine, id: InstanceId, name: Option<&[u8]>, recursive: bool, looked: &mut usize) -> Option<InstanceId> {
    // (the instances whose children are still to be searched, the next on top: a stack in place of recursion, since a script can
    // build a chain deeper than any thread's stack holds frames for)
    let mut stack = vec![id];
    while let Some(id) = stack.pop() {
        if recursive {
            // (a lazily imported instance whose children are not made: searched in its plan)
            let unmade = project::find_unmade(&mut engine.state.borrow_mut(), &engine.reflection, id, |child, _| Some(child) == name);
            if let Some(found) = unmade {
                match found {
                    Some(child) => return Some(reached(engine, child)),
                    None => continue,
                }
            }
        }
        materialize(engine, id);
        let found = {
            let state = engine.state.borrow();
            let mut found = None;
            for &child in &state.dom.at(id).children {
                *looked += 1;
                if name_equals(&state, child, name) {
                    found = Some(child);
                    break;
                }
            }
            found
        };
        if let Some(child) = found {
            return Some(reached(engine, child));
        }
        if !recursive && let Some(name) = name && let Some(made) = made_on_request(engine, id, name) {
            return Some(made);
        }
        if recursive {
            stack.extend(engine.state.borrow().dom.at(id).children.iter().rev());
        }
    }
    None
}

/// What a script's FindFirstChild pays for the instances it looked at beyond the call's price (cost.rs FIND_CHILD past the first
/// child, FIND_NODE for each node of a recursive find)
pub fn charge_find(engine: &Engine, looked: usize, recursive: bool) {
    let seconds = if recursive { looked as f64 * crate::cost::FIND_NODE } else { looked.saturating_sub(1) as f64 * crate::cost::FIND_CHILD };
    if seconds > 0.0 {
        engine.state.borrow().sched.charge(seconds);
        crate::boundary::add_cost(seconds);
    }
}

fn class_is_a(engine: &Engine, class: ClassId, name: Option<&[u8]>) -> bool {
    match name.and_then(|n| std::str::from_utf8(n).ok()) {
        Some(name) => engine.reflection.is_a(class, name),
        None => false,
    }
}

/// FindFirstChildWhichIsA (recursive: each child's subtree in turn, after the direct children, as [`find_first_child`] searches)
pub fn find_first_child_which_is_a(engine: &Engine, id: InstanceId, name: Option<&[u8]>, recursive: bool) -> Option<InstanceId> {
    let mut stack = vec![id];
    while let Some(id) = stack.pop() {
        if recursive {
            let unmade = project::find_unmade(&mut engine.state.borrow_mut(), &engine.reflection, id, |_, class| class_is_a(engine, class, name));
            if let Some(found) = unmade {
                if found.is_some() {
                    return found;
                }
                continue;
            }
        }
        materialize(engine, id);
        let found = find_child_where(engine, id, |state, child| class_is_a(engine, state.dom.at(child).class, name));
        if found.is_some() {
            return found;
        }
        if recursive {
            stack.extend(engine.state.borrow().dom.at(id).children.iter().rev());
        }
    }
    None
}

fn optional_instance(lua: &Lua, engine: &Engine, id: Option<InstanceId>) -> Ret {
    Ok(values([match id {
        Some(id) => instance_value(lua, engine, id)?,
        None => LuaValue::Nil,
    }]))
}

fn instance_list(lua: &Lua, engine: &Engine, ids: &[InstanceId]) -> Ret {
    let table = lua.create_table_with_capacity(ids.len(), 0)?;
    for &id in ids {
        table.raw_push(instance_value(lua, engine, id)?)?;
    }
    Ok(values([LuaValue::Table(table)]))
}

fn string_arg(args: &Args, index: usize) -> Option<Str> {
    args.string(index).map(|s| Str::new(&s.as_bytes()))
}

/// Why SetAttribute refuses an attribute name, as Roblox words it, raised without a position (measured in a playing client and a
/// server, Studio 0.744 and 0.747, tools/parity/probes/instances.luau and arg_checks.luau), the checks in this order: the `RBX`
/// prefix (case matters: `rbxname` goes; `RBX$` and a 104-byte `RBX aa...` get it), empty, over 100 bytes (a 101-byte name with
/// `$` gets it), then the first byte other than letters, digits and `_ - . /` (a space, `$`, the lone lead byte 0xC3 of `Ü`). The
/// message holds the name's bytes as they are, so the illegal byte alone can make it no UTF-8.
pub fn attribute_name_error(name: &[u8]) -> Option<Vec<u8>> {
    let message = |head: &str, tail: &str| [head.as_bytes(), name, tail.as_bytes()].concat();
    if name.starts_with(b"RBX") {
        return Some(message("Attempt to assign attribute ", ". CoreScript permission required"));
    }
    if name.is_empty() {
        return Some(b"Attribute name cannot be empty (\"\")".to_vec());
    }
    if name.len() > 100 {
        return Some(message("Attribute name exceeds 100 character limit (\"", "\")"));
    }
    let legal = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/');
    let illegal = *name.iter().find(|&&byte| !legal(byte))?;
    Some([b"Attribute name contains illegal character '".as_slice(), &[illegal], b"' (\"", name, b"\")"].concat())
}

/// A capability-only property `name` of `class` (reflection.rs CAPABILITY_ONLY): the capability a game script lacks for it.
fn capability_property(engine: &Engine, class: ClassId, name: &[u8]) -> Option<&'static str> {
    crate::reflection::CAPABILITY_ONLY
        .iter()
        .find(|(owner, property, _)| property.as_bytes() == name && engine.reflection.is_a(class, owner))
        .map(|&(_, _, capability)| capability)
}

/// The property IsPropertyModified, ResetPropertyToDefault and GetStyled name (their string parameter, cast as `strings` casts it):
/// one a game script may use. A name that is no property's (a NotScriptable one, a deprecated lower-case one, an event's, a
/// method's) raises "Invalid argument to IsPropertyModified(): Class Folder has no property named `Nope`" (GetStyled "GetStyled():
/// No such property named `Nope`"), and one a game script lacks the capability for "The current thread cannot call
/// IsPropertyModified() on Model.LevelOfDetail (lacking capability PluginOrOpenCloud)" (GetStyled "... GetStyled() on
/// 'LevelOfDetail' ..."); measured, Studio 0.747, a client and a server: tools/parity/probes/instance_api_texts.luau.
fn property_name_arg(engine: &Engine, id: InstanceId, args: &Args, method: &str) -> Result<NameId, Flow> {
    let class = engine.state.borrow().dom.at(id).class;
    let text = args.string(1).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
    let shown = String::from_utf8_lossy(&text);
    // (the instance's own class, not the declaring one: the Workspace's LevelOfDetail is `Workspace.LevelOfDetail`, measured)
    if let Some(capability) = capability_property(engine, class, &text) {
        return raise(if method == "GetStyled" {
            format!("The current thread cannot call GetStyled() on '{shown}' (lacking capability {capability})")
        } else {
            format!("The current thread cannot call {method}() on {}.{shown} (lacking capability {capability})", engine.reflection.class(class).name)
        });
    }
    let name = engine.reflection.name_id_bytes(&text).filter(|&name| {
        engine.reflection.class_property(class, name).is_some_and(|property| property.scriptable)
            && matches!(engine.resolve_member(class, name), Member::Property { .. })
    });
    match name {
        Some(name) => Ok(name),
        None if method == "GetStyled" => raise(format!("GetStyled(): No such property named `{shown}`")),
        None => raise(format!("Invalid argument to {method}(): Class {} has no property named `{shown}`", engine.reflection.class(class).name)),
    }
}

/// A method's Object parameter (IsDescendantOf's, IsAncestorOf's) as a client casts it: an Instance, or none for nil and for
/// the values a client casts to null (a thread, an Enum, Enums, a newproxy); no argument at all raises `Argument 1 missing or
/// nil`, any other value (a number, a table, a datatype, an EnumItem, a function, a buffer, a signal, an InstanceHandle)
/// `Unable to cast value to Object`, both without a position (measured, Studio 0.748, a client and a server alike:
/// tools/parity/probes/object_parameter_casts.luau)
pub(crate) fn object_arg(engine: &Engine, args: &Args, index: usize) -> Result<Option<InstanceId>, Flow> {
    if index >= args.len() {
        return raise(format!("Argument {index} missing or nil"));
    }
    if let Some(instance) = args.instance(index) {
        return Ok(Some(instance));
    }
    match args.get(index) {
        LuaValue::Nil | LuaValue::Thread(_) => Ok(None),
        other => match typeof_value(engine, other).as_str() {
            "Enum" | "Enums" | "userdata" => Ok(None),
            _ => raise("Unable to cast value to Object"),
        },
    }
}

/// The attribute datatypes SetAttribute accepts (by typeof).
const ATTRIBUTE_TYPES: &[&str] = &[
    "string", "boolean", "number", "UDim", "UDim2", "BrickColor", "Color3", "Vector2", "Vector3", "CFrame", "EnumItem",
    "NumberSequence", "ColorSequence", "NumberRange", "Rect", "Font", "Instance",
];

/// An attribute's value as a script reads it: an Instance as a new InstanceHandle (types.rs), the handle of none an attribute
/// may hold as a new handle of none, anything else as itself
fn attribute_to_lua(lua: &Lua, engine: &Engine, value: &Value) -> LuaResult<LuaValue> {
    match value {
        Value::Instance(target) => super::types::instance_handle(lua, engine, Some(*target)),
        Value::NoInstance => super::types::instance_handle(lua, engine, None),
        other => convert::to_lua(lua, engine, other),
    }
}

/// WaitForChild: the existing child, or yields until a child with the name is added (a
/// ChildAdded connection resumes the thread), the timeout passes, or forever (with the
/// "Infinite yield possible" warning after 5 s). A nil name raises "Argument 1 missing or nil"; a thread that cannot yield
/// gets an existing child and raises `thread is not yieldable` for a missing one, whatever its timeout (in this order, and
/// without a position: measured in a 0.747 playing client and server, tools/parity/probes/yield_refused.luau).
fn wait_for_child(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, args: &Args) -> Ret {
    let name = args.get(1).clone();
    if name.is_nil() {
        return raise("Argument 1 missing or nil");
    }
    let name_string = name_arg(&name);
    let name_bytes = name_string.as_ref().map(|s| s.as_bytes());
    if let Some(existing) = find_first_child(engine, id, name_bytes.as_deref(), false) {
        return optional_instance(lua, engine, Some(existing));
    }
    let thread = lua.current_thread();
    let yieldable = super::yieldable(&thread);
    if let LuaValue::String(s) = &name {
        // (services port: the yieldable lookup runs a pending rur.json clientGui mount first; one that cannot yield is an
        // index's, globals::missing_child)
        let found = if yieldable {
            super::globals::missing_child_yieldable(lua, engine, id, &s.as_bytes())?
        } else {
            super::globals::missing_child(lua, engine, id, &s.as_bytes())?
        };
        if let Some(child) = found {
            return optional_instance(lua, engine, Some(child));
        }
    }
    if !yieldable {
        return raise(super::NOT_YIELDABLE);
    }
    super::refs::admit(engine, super::refs::Hold::Wait, "WaitForChild")?;
    let done = Rc::new(Cell::new(false));
    let connection_slot = Rc::new(Cell::new(None));
    let callback = {
        let engine = engine.clone();
        let done = done.clone();
        let connection_slot = connection_slot.clone();
        let thread = thread.clone();
        let name = name.clone();
        super::create_boxed_void_val(lua, Box::new(move |lua, child: LuaValue| {
            let Some(child_id) = instance_id(&child) else {
                return Ok(());
            };
            let wanted = name_arg(&name);
            let wanted = wanted.as_ref().map(|s| s.as_bytes());
            if done.get() || !name_equals(&engine.state.borrow(), child_id, wanted.as_deref()) {
                return Ok(());
            }
            done.set(true);
            if let Some(connection) = connection_slot.get() {
                engine.state.borrow_mut().signals.disconnect(connection);
            }
            scheduler::resume(lua, &engine, &thread, LuaMultiValue::from_iter([child]));
            Ok(())
        }))?
    };
    let signal = core::event_signal(&mut engine.state.borrow_mut(), &engine.reflection, id, K::ChildAdded as NameId);
    let noted = crate::census::active(engine).then(|| callback.clone());
    let connection = engine.state.borrow_mut().signals.connect(signal, callback, false);
    if let Some(callback) = noted {
        crate::census::connected(lua, connection, &callback);
    }
    connection_slot.set(Some(connection));
    match args.number(2) {
        Some(timeout) => {
            let engine_ref = engine.clone();
            let done = done.clone();
            let thread = thread.clone();
            let body = super::create_boxed_void(lua, Box::new(move |lua| {
                if done.get() {
                    return Ok(());
                }
                done.set(true);
                engine_ref.state.borrow_mut().signals.disconnect(connection);
                scheduler::resume(lua, &engine_ref, &thread, LuaMultiValue::from_iter([LuaValue::Nil]));
                Ok(())
            }))?;
            let timer = lua.create_thread(body)?;
            engine.state.borrow_mut().sched.schedule_task(TimerKind::Delay, timeout, timer, Vec::new());
        }
        None => {
            let label = format!(
                "Infinite yield possible on '{}:WaitForChild(\"{}\")'",
                full_name(engine, id),
                key_text(&name)
            );
            let engine_ref = engine.clone();
            let body = super::create_boxed_void(lua, Box::new(move |_| {
                if !done.get() {
                    engine_ref.warn(label.clone());
                }
                Ok(())
            }))?;
            let timer = lua.create_thread(body)?;
            engine.state.borrow_mut().sched.schedule_task(TimerKind::Delay, 5.0, timer, Vec::new());
        }
    }
    Err(Flow::Yield)
}

/// Registers the Instance userdata type (raw tagged userdata with C metamethods, lua/raw.rs) and
/// the base Instance behaviours.
pub fn register(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let iter = wrapped(lua, engine, "__iter", |_, _| raise_at_caller("attempt to iterate over a Instance value"))?;
    let metatable = lua.create_table()?;
    metatable.raw_set("__type", "Instance")?;
    metatable.raw_set("__metatable", "The metatable is locked")?;
    metatable.raw_set("__iter", iter)?;
    raw::set_metatable(lua, metatable)?;
    install_base(&mut engine.behaviors.borrow_mut(), &engine.reflection);
    Ok(())
}

/// The Instance metatable is locked when it is installed (kept for Engine.Start's call order).
pub fn lock(_lua: &Lua, _engine: &Engine, _sample: InstanceId) -> LuaResult<()> {
    Ok(())
}

fn method(f: impl Fn(&Lua, &Rc<Engine>, InstanceId, &Args) -> Ret + 'static) -> MethodFn {
    Rc::new(f)
}

/// The base methods' one string parameter, the first, read as a client reads it (services::strings; measured in a playing client
/// and a server, Studio 0.747: tools/parity/probes/string_parameter_methods.luau): nil raises "Argument 1 missing or nil", a number
/// is its `%.20g` text, anything else its text after the warning, and the method then answers as for that string.
const NAME: &[StringParam] = &[StringParam::at(1)];
/// [`NAME`] of WaitForChild, which yields: a client casts its name without the warning.
const QUIET_NAME: &[StringParam] = &[StringParam::quiet(1)];

fn install_base(behaviors: &mut Behaviors, reflection: &crate::Reflection) {
    let base = reflection.class_id("Instance").expect("Instance class");

    // Getters / setters
    let getter = |f: GetterFn| f;
    let class_name = getter(Rc::new(|lua, engine, id| {
        let class = engine.state.borrow().dom.at(id).class;
        Ok(LuaValue::String(lua.create_string(&*engine.reflection.class(class).name)?))
    }));
    behaviors.add_getter(base, K::ClassName as NameId, class_name.clone());
    behaviors.add_getter(base, K::Parent as NameId, getter(Rc::new(|lua, engine, id| {
        let parent = engine.state.borrow().dom.at(id).parent;
        match parent {
            Some(parent) => instance_value(lua, engine, parent).map_err(Flow::from),
            None => Ok(LuaValue::Nil),
        }
    })));
    let parent_setter: SetterFn = Rc::new(|lua, engine, id, value| {
        let parent = match &value {
            LuaValue::Nil => None,
            LuaValue::UserData(_) if instance_id(&value).is_some() => instance_id(&value),
            _ => {
                // Measured in a playing client (Studio 0.744, tools/parity/probes/hierarchy.luau): Luau's own
                // argument error of the __newindex call, at the script line.
                return raise_at_caller(format!("invalid argument #3 (Instance expected, got {})", typeof_value(engine, &value)));
            }
        };
        let _ = lua;
        set_parent(engine, id, parent)?;
        Ok(())
    });
    behaviors.add_setter(base, K::Parent as NameId, parent_setter);
    // Script.Source: Studio lets a plugin write it (a game's scripts cannot), and ree has no plugin or game boundary, so a
    // script may. It replaces the source file the instance was mounted from: the next require or start reads the text.
    if let Some(container) = reflection.class_id("LuaSourceContainer") {
        behaviors.add_setter(container, K::Source as NameId, Rc::new(|_, engine, id, value| {
            let LuaValue::String(text) = &value else {
                return raise(format!("Unable to assign property Source. string expected, got {}", typeof_value(engine, &value)));
            };
            let bytes = text.as_bytes().to_vec();
            {
                let mut state = engine.state.borrow_mut();
                let instance = state.dom.at_mut(id);
                instance.set_source(Some(Rc::from(bytes.as_slice())));
                instance.set_source_path(None);
            }
            engine.modules.borrow_mut().forget_bytecode(id);
            set_property(engine, id, K::Source as NameId, Value::String(Str::new(&bytes)));
            Ok(())
        }));
    }
    // Script.RunContext is plugin-writable in Studio as Source is
    if let (Some(script), Some(run_context)) = (reflection.class_id("BaseScript"), reflection.name_id("RunContext")) {
        behaviors.add_setter(script, run_context, Rc::new(move |lua, engine, id, value| {
            let class = engine.state.borrow().dom.at(id).class;
            if let Member::Property { property: Some(property), .. } = engine.resolve_member(class, run_context) {
                let info = engine.reflection.property(property).clone();
                let coerced = convert::coerce(lua, engine, &info, &value)?;
                set_property(engine, id, run_context, coerced);
            }
            Ok(())
        }));
    }
    behaviors.add_getter(base, K::Name as NameId, getter(Rc::new(|lua, engine, id| {
        let state = engine.state.borrow();
        let instance = state.dom.at(id);
        let name = match instance.stored(K::Name as NameId) {
            Some(Value::String(name)) => name.as_bytes().to_vec(),
            Some(_) => Vec::new(),
            None => engine.reflection.class(instance.class).name.as_bytes().to_vec(),
        };
        drop(state);
        Ok(LuaValue::String(lua.create_string(name)?))
    })));
    behaviors.add_setter(base, K::Name as NameId, Rc::new(|lua, engine, id, value| {
        let text = match &value {
            LuaValue::String(s) => Str::new(&s.as_bytes()),
            LuaValue::Number(_) | LuaValue::Integer(_) => {
                Str::new(&lua.coerce_string(value.clone())?.map(|s| s.as_bytes().to_vec()).unwrap_or_default())
            }
            _ => {
                return raise(format!(
                    "Unable to assign property Name. string expected, got {}",
                    typeof_value(engine, &value)
                ));
            }
        };
        set_property(engine, id, K::Name as NameId, Value::String(text));
        Ok(())
    }));

    // Methods
    let get_children = method(|lua, engine, id, _| {
        materialize(engine, id);
        let children = engine.state.borrow().dom.at(id).children.to_vec();
        instance_list(lua, engine, &children)
    });
    behaviors.add_method(base, K::GetChildren as NameId, get_children.clone());
    behaviors.add_method(base, K::children as NameId, get_children.clone());
    behaviors.add_method(base, K::getChildren as NameId, get_children);
    behaviors.add_method(base, K::GetDescendants as NameId, method(|lua, engine, id, _| {
        project::materialize_deep(engine, id);
        let descendants = engine.state.borrow().dom.descendants(id);
        // (the frame's first GetDescendants runs its code cold: cost.rs Path::Descendants)
        {
            let state = engine.state.borrow();
            let first = {
                let mut touched = state.sched.touched.borrow_mut();
                touched.count(|ops| ops.queries += 1);
                touched.first_path(crate::cost::Path::Descendants)
            };
            if first > 0.0 {
                state.sched.charge(first);
                crate::boundary::add_cost(first);
            }
        }
        instance_list(lua, engine, &descendants)
    }));
    // A missing name raises (measured in a playing client, Studio 0.744, tools/parity/probes/instances.luau; no position):
    // FindFirstChild(nil) is a script's bug in Roblox, not a miss
    let find_first_child_method = strings(NAME, method(|lua, engine, id, args| {
        let name = name_arg(args.get(1));
        let (recursive, mut looked) = (args.boolean(2), 0);
        let found = find_first_child_counting(engine, id, name.as_ref().map(|s| s.as_bytes()).as_deref(), recursive, &mut looked);
        charge_find(engine, looked, recursive);
        optional_instance(lua, engine, found)
    }));
    behaviors.add_method(base, K::FindFirstChild as NameId, find_first_child_method.clone());
    behaviors.add_method(base, K::findFirstChild as NameId, find_first_child_method);
    behaviors.add_method(base, K::FindFirstChildOfClass as NameId, strings(NAME, method(|lua, engine, id, args| {
        materialize(engine, id);
        let wanted = args.string(1).map(|s| s.to_string_lossy());
        let found = find_child_where(engine, id, |state, child| wanted.as_deref() == Some(&*engine.reflection.class(state.dom.at(child).class).name));
        optional_instance(lua, engine, found)
    })));
    behaviors.add_method(base, K::FindFirstChildWhichIsA as NameId, strings(NAME, method(|lua, engine, id, args| {
        let name = name_arg(args.get(1));
        let found = find_first_child_which_is_a(engine, id, name.as_ref().map(|s| s.as_bytes()).as_deref(), args.boolean(2));
        optional_instance(lua, engine, found)
    })));
    // (not enabled in Roblox: measured likewise, `FindFirstDescendant is not enabled`, once its name is read)
    behaviors.add_method(base, K::FindFirstDescendant as NameId, strings(NAME, method(|_, _, _, _| raise("FindFirstDescendant is not enabled"))));
    behaviors.add_method(base, K::FindFirstAncestor as NameId, strings(NAME, method(|lua, engine, id, args| {
        let (found, steps) = {
            let state = engine.state.borrow();
            let mut current = state.dom.at(id).parent;
            let wanted = name_arg(args.get(1));
            let wanted = wanted.as_ref().map(|s| s.as_bytes());
            let mut steps = 0;
            while let Some(node) = current {
                steps += 1;
                if name_equals(&state, node, wanted.as_deref()) {
                    break;
                }
                current = state.dom.at(node).parent;
            }
            (current, steps)
        };
        charge_ancestors(engine, steps);
        optional_instance(lua, engine, found)
    })));
    behaviors.add_method(base, K::FindFirstAncestorOfClass as NameId, strings(NAME, method(|lua, engine, id, args| {
        let (found, steps) = {
            let state = engine.state.borrow();
            let wanted = args.string(1).map(|s| s.to_string_lossy());
            let mut current = state.dom.at(id).parent;
            let mut steps = 0;
            while let Some(node) = current {
                steps += 1;
                if wanted.as_deref() == Some(&*engine.reflection.class(state.dom.at(node).class).name) {
                    break;
                }
                current = state.dom.at(node).parent;
            }
            (current, steps)
        };
        charge_ancestors(engine, steps);
        optional_instance(lua, engine, found)
    })));
    behaviors.add_method(base, K::FindFirstAncestorWhichIsA as NameId, strings(NAME, method(|lua, engine, id, args| {
        let (found, steps) = {
            let state = engine.state.borrow();
            let mut current = state.dom.at(id).parent;
            let wanted = name_arg(args.get(1));
            let wanted = wanted.as_ref().map(|s| s.as_bytes());
            let mut steps = 0;
            while let Some(node) = current {
                steps += 1;
                if class_is_a(engine, state.dom.at(node).class, wanted.as_deref()) {
                    break;
                }
                current = state.dom.at(node).parent;
            }
            (current, steps)
        };
        charge_ancestors(engine, steps);
        optional_instance(lua, engine, found)
    })));
    let is_a = strings(NAME, method(|_, engine, id, args| {
        let class = engine.state.borrow().dom.at(id).class;
        let wanted = name_arg(args.get(1));
        let wanted = wanted.as_ref().map(|s| s.as_bytes());
        Ok(values([LuaValue::Boolean(class_is_a(engine, class, wanted.as_deref()))]))
    }));
    behaviors.add_method(base, K::IsA as NameId, is_a.clone());
    behaviors.add_method(base, K::isA as NameId, is_a.clone());
    // Roblox's Object, Instance's superclass and EditableImage's and EditableMesh's, has ClassName
    // and IsA itself: an EditableImage answers them as any instance does (Instance keeps its own, found
    // first for its classes)
    if let Some(object) = reflection.class_id("Object") {
        behaviors.add_getter(object, K::ClassName as NameId, class_name);
        behaviors.add_method(object, K::IsA as NameId, is_a.clone());
        behaviors.add_method(object, K::isA as NameId, is_a);
    }
    let is_descendant_of = method(|_, engine, id, args| {
        let result = match object_arg(engine, args, 1)? {
            Some(ancestor) => descendant_of(engine, id, ancestor),
            // every instance is a descendant of nil (measured likewise: IsDescendantOf(nil) is true)
            None => true,
        };
        Ok(values([LuaValue::Boolean(result)]))
    });
    behaviors.add_method(base, K::IsDescendantOf as NameId, is_descendant_of.clone());
    behaviors.add_method(base, K::isDescendantOf as NameId, is_descendant_of);
    behaviors.add_method(base, K::IsAncestorOf as NameId, method(|_, engine, id, args| {
        let result = match object_arg(engine, args, 1)? {
            Some(descendant) => descendant_of(engine, descendant, id),
            None => false,
        };
        Ok(values([LuaValue::Boolean(result)]))
    }));
    behaviors.add_method(base, K::GetFullName as NameId, method(|lua, engine, id, _| {
        Ok(values([LuaValue::String(lua.create_string(full_name(engine, id))?)]))
    }));
    let clone_method = method(|lua, engine, id, _| {
        // A service cannot be cloned (measured likewise: `Lighting cannot be cloned`, `Players cannot be cloned`), nor a settings
        // root (`Global Settings cannot be cloned`, `GlobalBasicSettings cannot be cloned`: services/settings.rs)
        let (class, name) = {
            let state = engine.state.borrow();
            let instance = state.dom.at(id);
            (instance.class, instance.name.to_str_lossy().into_owned())
        };
        if engine.reflection.class(class).service || crate::services::settings::is_root(&engine.reflection, class) {
            return raise(format!("{name} cannot be cloned"));
        }
        // nor the PlayerMouse while it is Archivable (measured, Studio 0.748: "Instance cannot be cloned"; nil once it is not)
        let archivable = engine.reflection.name_id("Archivable").and_then(|n| engine.state.borrow().dom.boolean(id, n)).unwrap_or(true);
        if archivable && engine.reflection.is_a(class, "Mouse") {
            return raise(format!("{name} cannot be cloned"));
        }
        // nor what the engine keeps of its own: Terrain raises "Cannot Clone() Terrain", StarterPlayer's script folders
        // "StarterPlayer.StarterPlayerScripts cannot be cloned" (the full name), and a player's PlayerGui and PlayerScripts and a
        // GetTextBoundsParams answer nil, Archivable as they are (measured, Studio 0.747, a client and a server:
        // tools/parity/probes/instance_api_texts.luau; the script folders on a place copy holding them: parent_locks.luau's log)
        match &*engine.reflection.class(class).name {
            "Terrain" => return raise("Cannot Clone() Terrain"),
            "StarterPlayerScripts" | "StarterCharacterScripts" => return raise(format!("{} cannot be cloned", full_name(engine, id))),
            "PlayerGui" | "PlayerScripts" | "GetTextBoundsParams" => return Ok(values([LuaValue::Nil])),
            _ => {}
        }
        optional_instance(lua, engine, clone(engine, id))
    });
    behaviors.add_method(base, K::Clone as NameId, clone_method.clone());
    behaviors.add_method(base, K::clone as NameId, clone_method);
    let destroy_method = method(|lua, engine, id, _| {
        destroy(lua, engine, id)?;
        Ok(none())
    });
    behaviors.add_method(base, K::Destroy as NameId, destroy_method.clone());
    behaviors.add_method(base, K::destroy as NameId, destroy_method);
    let remove = method(|_, engine, id, _| {
        set_parent(engine, id, None)?;
        Ok(none())
    });
    behaviors.add_method(base, K::Remove as NameId, remove.clone());
    behaviors.add_method(base, K::remove as NameId, remove);
    behaviors.add_method(base, K::ClearAllChildren as NameId, method(|lua, engine, id, _| {
        // Last child first (measured in a playing client, Studio 0.744: tools/parity/probes/hierarchy.luau)
        let children = engine.state.borrow().dom.at(id).children.to_vec();
        for child in children.into_iter().rev() {
            destroy(lua, engine, child)?;
        }
        Ok(none())
    }));
    behaviors.add_method(base, K::WaitForChild as NameId, strings(QUIET_NAME, method(wait_for_child)));
    behaviors.add_method(base, K::GetPropertyChangedSignal as NameId, strings(NAME, method(|lua, engine, id, args| {
        let class = engine.state.borrow().dom.at(id).class;
        let text = args.string(1).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
        let shown = String::from_utf8_lossy(&text);
        // (a property a game script lacks the capability for raises the read's error, one scripts cannot use at all (NotScriptable:
        // PVInstance.Origin, Model.WorldPivotData) "Origin is not a scriptable property.", and any other name, a deprecated lower-case
        // one with no member of its own too, "Nope is not a valid property name."; measured, Studio 0.747, a client and a server:
        // tools/parity/probes/instance_api_texts.luau)
        if let Some(capability) = capability_property(engine, class, &text) {
            return raise(format!("The current thread cannot read '{shown}' (lacking capability {capability})"));
        }
        let name = engine.reflection.name_id_bytes(&text);
        let member = name.map_or(Member::None, |name| engine.resolve_member(class, name));
        let (Member::Property { .. }, Some(name)) = (member, name) else {
            if name.is_some_and(|name| engine.reflection.class_property(class, name).is_some()) {
                return raise(format!("{shown} is not a scriptable property."));
            }
            return raise(format!("{} is not a valid property name.", key_text(args.get(1))));
        };
        let signal = core::property_signal(&mut engine.state.borrow_mut(), &engine.reflection, id, name);
        Ok(values([signal_value(lua, engine, signal)?]))
    })));
    // (a number names the attribute of its text, as SetAttribute takes it: measured likewise, GetAttribute(5) reads what
    // SetAttribute(5, 1) set)
    behaviors.add_method(base, K::GetAttribute as NameId, strings(NAME, method(|lua, engine, id, args| {
        let Some(name) = string_arg(args, 1) else { return Ok(values([LuaValue::Nil])) };
        let value = engine.state.borrow().dom.at(id).attribute(name.as_bytes()).cloned();
        Ok(values([match value {
            Some(value) => attribute_to_lua(lua, engine, &value)?,
            None => LuaValue::Nil,
        }]))
    })));
    behaviors.add_method(base, K::GetAttributes as NameId, method(|lua, engine, id, _| {
        let attributes = engine.state.borrow().dom.at(id).attributes.as_slice().to_vec();
        let listed = attributes.len() as f64 * crate::cost::ATTRIBUTE_LISTED;
        engine.state.borrow().sched.charge(listed);
        crate::boundary::add_cost(listed);
        let table = lua.create_table()?;
        for (name, value) in attributes {
            table.raw_set(lua.create_string(name.as_bytes())?, attribute_to_lua(lua, engine, &value)?)?;
        }
        Ok(values([LuaValue::Table(table)]))
    }));
    // (the name is read before the value is checked: a boolean's warning comes before "Function is not a supported attribute
    // type", as a client casts its arguments before the method runs)
    behaviors.add_method(base, K::SetAttribute as NameId, strings(NAME, method(|lua, engine, id, args| {
        let value = args.get(2);
        // (an Instance, or an InstanceHandle, is kept by reference: Studio 0.744 takes both; a handle of none is kept as such
        // and reads back as one, listed by GetAttributes and copied by Clone, measured in a 0.748 client and server:
        // tools/parity/probes/instance_handle.luau)
        let handle = super::types::instance_handle_of(value);
        if !value.is_nil() && handle.is_none() {
            let kind = typeof_value(engine, value);
            if !ATTRIBUTE_TYPES.contains(&kind.as_str()) {
                // (a SharedTable given is marked before it is refused, as a client marks it: lua/shared_table.rs `mark_value`)
                super::shared_table::mark_value(value);
                // (Roblox names a table an Array and a function a Function: measured, {} and print; other values by
                // typeof, buffer in lower case. Not measured: a table with string keys, here a Dictionary)
                let kind = match value {
                    LuaValue::Table(table) if table.raw_len() > 0 || table.clone().pairs::<LuaValue, LuaValue>().next().is_none() => "Array".to_string(),
                    LuaValue::Table(_) => "Dictionary".to_string(),
                    LuaValue::Function(_) => "Function".to_string(),
                    _ => kind,
                };
                return raise(format!("{kind} is not a supported attribute type"));
            }
        }
        let name = string_arg(args, 1).unwrap_or_default();
        if let Some(message) = attribute_name_error(name.as_bytes()) {
            // (bytes that are no UTF-8, the illegal byte of a name such as `Ünï`, go as a string of those bytes, raised as it is)
            return match String::from_utf8(message) {
                Ok(text) => raise(text),
                Err(bytes) => Err(Flow::Raised(LuaValue::String(lua.create_string(bytes.into_bytes())?))),
            };
        }
        let converted = match handle {
            Some(Some(target)) => Value::Instance(target),
            Some(None) => Value::NoInstance,
            None => convert::from_lua(value),
        };
        set_attribute(engine, id, name, converted);
        Ok(none())
    })));
    behaviors.add_method(base, K::GetAttributeChangedSignal as NameId, strings(NAME, method(|lua, engine, id, args| {
        let name = string_arg(args, 1).unwrap_or_default();
        let signal = core::attribute_signal(&mut engine.state.borrow_mut(), id, &name);
        Ok(values([signal_value(lua, engine, signal)?]))
    })));
    behaviors.add_method(base, K::AddTag as NameId, strings(NAME, method(|_, engine, id, args| {
        if let Some(tag) = string_arg(args, 1) {
            core::add_tag(&mut engine.state.borrow_mut(), id, tag);
        }
        Ok(none())
    })));
    behaviors.add_method(base, K::RemoveTag as NameId, strings(NAME, method(|_, engine, id, args| {
        if let Some(tag) = string_arg(args, 1) {
            core::remove_tag(&mut engine.state.borrow_mut(), id, &tag);
        }
        Ok(none())
    })));
    behaviors.add_method(base, K::HasTag as NameId, strings(NAME, method(|_, engine, id, args| {
        let has = string_arg(args, 1).is_some_and(|tag| engine.state.borrow().dom.at(id).tags.contains(&tag));
        Ok(values([LuaValue::Boolean(has)]))
    })));
    // (sorted by their bytes, whatever order they were added in: "B,C,a", "1,10,100,2,9"; measured, 0.747, a playing client and a
    // server, tools/parity/probes/collection_tags.luau. The instance keeps them in the order added, which the tag signals follow)
    behaviors.add_method(base, K::GetTags as NameId, method(|lua, engine, id, _| {
        let mut tags = engine.state.borrow().dom.at(id).tags.clone();
        crate::sort::by(&mut tags, |a, b| a.as_bytes().cmp(b.as_bytes()));
        let table = lua.create_table()?;
        for tag in tags {
            table.raw_push(lua.create_string(tag.as_bytes())?)?;
        }
        Ok(values([LuaValue::Table(table)]))
    }));
    behaviors.add_method(base, K::GetActor as NameId, method(|lua, engine, id, _| {
        let actor = super::actors::actor_of(engine, id);
        optional_instance(lua, engine, actor)
    }));
    behaviors.add_method(base, K::IsPropertyModified as NameId, strings(NAME, method(|_, engine, id, args| {
        let name = property_name_arg(engine, id, args, "IsPropertyModified")?;
        let modified = engine.state.borrow().dom.at(id).stored(name).is_some();
        Ok(values([LuaValue::Boolean(modified)]))
    })));
    behaviors.add_method(base, K::ResetPropertyToDefault as NameId, strings(NAME, method(|_, engine, id, args| {
        let name = property_name_arg(engine, id, args, "ResetPropertyToDefault")?;
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        if state.dom.at_mut(id).unstore(name).is_some() {
            if name == K::Name as NameId {
                state.dom.refresh_name(&engine.reflection, id);
            }
            state.dom.bump_version(id);
            core::notify_changed(&mut state, &engine.reflection, &hooks, id, name, None);
        }
        Ok(none())
    })));
    behaviors.add_method(base, K::GetDebugId as NameId, method(|lua, engine, id, _| {
        let serial = engine.state.borrow().dom.at(id).serial;
        Ok(values([LuaValue::String(lua.create_string(format!("RUR_{serial}"))?)]))
    }));
    behaviors.add_method(base, K::GetStyled as NameId, strings(NAME, method(|lua, engine, id, args| {
        let name = property_name_arg(engine, id, args, "GetStyled")?;
        let value = engine.state.borrow().dom.raw_get(id, name).cloned();
        Ok(values([match value {
            Some(value) => convert::to_lua(lua, engine, &value)?,
            None => LuaValue::Nil,
        }]))
    })));
}

/// A deferred resumption of `thread` with `args` (task.defer, module waiters).
pub fn defer_thread(engine: &Engine, thread: LuaThread, args: Vec<Value>) {
    engine.state.borrow_mut().sched.defer(JobTarget::Thread(thread), args);
}
