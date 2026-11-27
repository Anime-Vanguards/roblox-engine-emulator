//! Loads the Luau service stubs (`services.luau`) with the engine primitives they use.

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Flow, convert, enum_item_value,
    instance::{set_parent, set_property},
    instance_value, modules, raw, signal_value, types,
};
use crate::{Engine, instance as core};

pub(crate) static SERVICES: &str = include_str!("services.luau");
pub(crate) static SERVER: &str = include_str!("server.luau");
pub(crate) static PROFILE: &str = include_str!("profile.luau");

/// The named registry value holding server.luau's R.SetJoinData (a server run's only): what
/// Context.SetJoinData calls to give a player the join data Player:GetJoinData returns
pub const SET_JOIN_DATA: &str = "ree.setjoindata";

/// The named registry value holding the stubs' primitives (R): [`set_profile`] gives them a new client profile
const STUBS: &str = "ree.stubs";

/// R.Profile: the client profile's set fields as Luau tables, with its player's `UserId`, when it sets a field the service
/// stubs read ([`crate::client_profile::ClientProfile::read_by_luau`]); else nil, and the stubs answer as without a profile.
fn profile_table(lua: &Lua, engine: &Engine, profile: &crate::client_profile::ClientProfile) -> LuaResult<LuaValue> {
    if !profile.read_by_luau(engine.options.server()) {
        return Ok(LuaValue::Nil);
    }
    let options = LuaSerializeOptions::new().set_array_metatable(false).serialize_none_to_null(false).serialize_unit_to_null(false);
    let value = lua.to_value_with(&profile.to_json(), options)?;
    if let LuaValue::Table(table) = &value {
        // (its player's id as the engine gives it, set or not; its name too where the profile sets its identity, which the
        // name lookups then answer for it)
        let identity = &profile.identity;
        table.raw_set("UserId", identity.user_id())?;
        if identity != &crate::client_profile::Identity::default() {
            table.raw_set("Name", identity.name())?;
            table.raw_set("DisplayName", identity.display_name())?;
            table.raw_set("HasVerifiedBadge", identity.has_verified_badge == Some(true))?;
        }
    }
    Ok(value)
}

/// The client profile's answers in the stubs (profile.luau), loaded once into `r` (R): each member it replaces is wrapped,
/// found through `Original(Class, Name)`
fn load_profile(lua: &Lua, engine: &Rc<Engine>, r: &LuaTable) -> LuaResult<()> {
    let shared = engine.lua.borrow().shared.clone().ok_or_else(|| LuaError::runtime("shared globals"))?;
    let chunk = modules::load_engine_luau(lua, PROFILE, "=rur.profile", Some(shared))?;
    let e = engine.clone();
    let original = super::lua_fn(lua, move |_, (class_name, name): (String, String)| {
        let (Some(class), Some(name)) = (e.reflection.class_id(&class_name), e.reflection.name_id(&name)) else { return Ok(LuaValue::Nil) };
        let behaviors = e.behaviors.borrow();
        Ok(match behaviors.find_method(&e.reflection, class, name) {
            Some((_, crate::services::Method::Lua(function))) => LuaValue::Function(function.clone()),
            _ => LuaValue::Nil,
        })
    })?;
    crate::cost::engine_luau(lua, chunk)?.call::<()>((r.clone(), original))?;
    engine.clear_member_cache();
    Ok(())
}

/// The engine's client profile from now on ([`Engine::set_profile`]), and the service stubs' (R.Profile): the profile's
/// answers loaded the first time it sets what they read (profile.luau), and the members it lets answer added
/// (R.ProfileMethods)
pub fn set_profile(lua: &Lua, engine: &Rc<Engine>, profile: crate::client_profile::ClientProfile) -> LuaResult<()> {
    let table = profile_table(lua, engine, &profile)?;
    engine.set_profile(profile);
    if let Ok(r) = lua.named_registry_value::<LuaTable>(STUBS) {
        let read = !table.is_nil();
        r.raw_set("Profile", table)?;
        match r.raw_get::<LuaValue>("ProfileMethods")? {
            LuaValue::Function(methods) => methods.call::<()>(())?,
            _ if read => load_profile(lua, engine, &r)?,
            _ => {}
        }
    }
    engine.clear_member_cache();
    Ok(())
}

/// LogService error tables turned into text (their `__tostring`, services.luau ErrorText): how many so far, and the last
/// one's (thread that asked, text). A thread that ends on such a table has mlua convert it into text on its resumer's
/// thread before the scheduler sees the error ([`uncaught_log_error`]).
#[derive(Default)]
struct LogErrorTexts {
    count: u64,
    thread: usize,
    text: String,
}

thread_local! {
    static LOG_ERROR_TEXTS: std::cell::RefCell<LogErrorTexts> = std::cell::RefCell::new(LogErrorTexts::default());
}

/// How many LogService error tables have been turned into text so far (taken before a thread is resumed).
pub fn log_error_texts() -> u64 {
    LOG_ERROR_TEXTS.with(|texts| texts.borrow().count)
}

/// Whether `thread`, which ended on the error `message`, ended on a LogService error table no script caught: one was turned
/// into text since `since`, last, on another thread than `thread` (mlua converts the ended thread's error value on the
/// thread that resumed it; a script's own `tostring` of the table runs on the script's thread), giving `message`.
pub fn uncaught_log_error(since: u64, thread: &LuaThread, message: &str) -> bool {
    LOG_ERROR_TEXTS.with(|texts| {
        let texts = texts.borrow();
        texts.count > since && texts.thread != thread.to_pointer() as usize && texts.text == message
    })
}

fn instance_of(value: &LuaValue) -> LuaResult<crate::InstanceId> {
    match value {
        LuaValue::UserData(ud) => raw::instance_of_ud(ud).ok_or_else(|| LuaError::runtime("Instance expected")),
        _ => Err(LuaError::runtime("Instance expected")),
    }
}

pub fn load(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let r = lua.create_table()?;
    let e = engine.clone();
    r.set(
        "Method",
        super::lua_fn(lua, move |_, (class_name, name, function): (String, String, LuaFunction)| {
            let (Some(class), Some(name)) = (e.reflection.class_id(&class_name), e.reflection.name_id(&name)) else {
                return Ok(());
            };
            e.behaviors.borrow_mut().add_lua_method(class, name, function);
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "Getter",
        super::lua_fn(lua, move |_, (class_name, name, function): (String, String, LuaFunction)| {
            let (Some(class), Some(name)) = (e.reflection.class_id(&class_name), e.reflection.name_id(&name)) else {
                return Ok(());
            };
            e.behaviors.borrow_mut().add_getter(
                class,
                name,
                Rc::new(move |lua, engine, id| Ok(function.call::<LuaValue>(instance_value(lua, engine, id)?)?)),
            );
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set(
        // a setter that first passes the assigned value through `filter(instance, value)` (which may
        // swap it), then assigns what it returns as the property's own setter does (a derived view's,
        // engine::derived: a Content view of a string asset id), else as a plain assignment (coerced,
        // notified)
        "ContentSetter",
        super::lua_fn(lua, move |_, (class_name, name, filter): (String, String, LuaFunction)| {
            let (Some(class), Some(name)) = (e.reflection.class_id(&class_name), e.reflection.name_id(&name)) else {
                return Ok(());
            };
            let Some(info) = e.reflection.class_property(class, name).cloned() else { return Ok(()) };
            let previous = e.behaviors.borrow().find_setter(&e.reflection, class, name).cloned();
            e.behaviors.borrow_mut().add_setter(
                class,
                name,
                Rc::new(move |lua, engine, id, value| {
                    let value = filter.call::<LuaValue>((instance_value(lua, engine, id)?, value))?;
                    if let Some(previous) = &previous {
                        return previous(lua, engine, id, value);
                    }
                    let coerced = convert::coerce(lua, engine, &info, &value)?;
                    set_property(engine, id, name, coerced);
                    Ok(())
                }),
            );
            // (a member resolved before now cached it without the setter)
            e.clear_member_cache();
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "Create",
        super::lua_fn(lua, move |lua, class_name: String| {
            let class = e.reflection.class_id(&class_name).ok_or_else(|| LuaError::runtime(format!("Unable to create an Instance of type \"{class_name}\"")))?;
            let id = core::create(&mut e.state.borrow_mut(), &e.reflection, class);
            instance_value(lua, &e, id)
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "RawSet",
        super::lua_fn(lua, move |_, (target, name, value, silent): (LuaValue, String, LuaValue, bool)| {
            let id = instance_of(&target)?;
            let Some(name) = e.reflection.name_id(&name) else { return Ok(false) };
            let value = convert::from_lua(&value);
            let hooks = e.hooks.borrow();
            let mut state = e.state.borrow_mut();
            Ok(if silent {
                core::raw_set_silent(&mut state, &e.reflection, id, name, value)
            } else {
                core::raw_set(&mut state, &e.reflection, &hooks, id, name, value)
            })
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "RawGet",
        // (`stored_only`: nil unless the instance holds a value, never the class default)
        super::lua_fn(lua, move |lua, (target, name, stored_only): (LuaValue, String, Option<bool>)| {
            let id = instance_of(&target)?;
            let Some(name) = e.reflection.name_id(&name) else { return Ok(LuaValue::Nil) };
            let value = {
                let state = e.state.borrow();
                if stored_only == Some(true) {
                    state.dom.at(id).stored(name).filter(|value| !matches!(value, crate::value::Value::Nil)).cloned()
                } else {
                    state.dom.raw_get(id, name).cloned()
                }
            };
            match value {
                Some(value) => convert::to_lua(lua, &e, &value),
                None => Ok(LuaValue::Nil),
            }
        })?,
    )?;
    // an instance's event fired from the engine's Luau, its values as a property holds them (a table as it is): HumanoidDescription's
    // EmotesChanged and EquippedEmotesChanged (services.luau)
    let e = engine.clone();
    r.set(
        "FireEvent",
        super::lua_fn(lua, move |_, (target, name, values): (LuaValue, String, LuaMultiValue)| {
            let id = instance_of(&target)?;
            let Some(name) = e.reflection.name_id(&name) else { return Ok(()) };
            let values = values.iter().map(convert::from_lua).collect();
            core::fire_event(&mut e.state.borrow_mut(), id, name, values);
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "Children",
        super::lua_fn(lua, move |lua, target: LuaValue| {
            let id = instance_of(&target)?;
            // (a lazily imported bundle instance's children are made first, as GetChildren makes them)
            if e.state.borrow().dom.at(id).has(crate::dom::flags::LAZY_PENDING) {
                crate::project::materialize_lazy(&mut e.state.borrow_mut(), &e.reflection, id);
            }
            let children = e.state.borrow().dom.at(id).children.to_vec();
            let table = lua.create_table_with_capacity(children.len(), 0)?;
            for child in children {
                table.raw_push(instance_value(lua, &e, child)?)?;
            }
            Ok(table)
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "SetParent",
        super::lua_fn(lua, move |_, (child, parent): (LuaValue, LuaValue)| {
            let child = instance_of(&child)?;
            let parent = if parent.is_nil() { None } else { Some(instance_of(&parent)?) };
            set_parent(&e, child, parent).map_err(|_| LuaError::runtime("SetParent failed"))
        })?,
    )?;
    // Player:Kick on a server (server.luau): the player leaves about a second on (services/players.rs kick)
    let e = engine.clone();
    r.set(
        "Kick",
        super::lua_fn(lua, move |_, player: LuaValue| {
            let player = instance_of(&player)?;
            crate::services::players::kick(&mut e.state.borrow_mut(), player);
            Ok(())
        })?,
    )?;
    // The engine's Luau that prices its own methods (services.luau Priced): Mute(true) drops the Instance API's
    // charges until Mute(false), Charge adds the method's own cost
    let e = engine.clone();
    r.set(
        "Mute",
        super::lua_fn(lua, move |lua, on: bool| {
            crate::cost::mute_host_nanos(lua, on);
            let muted = e.state.borrow().sched.muted.clone();
            muted.set(if on { muted.get() + 1 } else { muted.get().saturating_sub(1) });
            Ok(())
        })?,
    )?;
    let costs = lua.create_table()?;
    for (name, seconds) in [
        ("ActorInvoke", crate::cost::ACTOR_INVOKE),
        ("SharedIncrement", crate::cost::SHARED_INCREMENT),
        ("ModelQuery", crate::cost::MODEL_QUERY),
        ("ModelExtentsQuery", crate::cost::MODEL_EXTENTS_QUERY),
        ("LogCall", crate::cost::LOG_CALL),
        ("Thread", crate::cost::THREAD),
        ("PivotSingle", crate::cost::PIVOT_SINGLE),
        ("PivotPart", crate::cost::PIVOT_PART),
        ("PivotBase", crate::cost::PIVOT_BASE),
        ("PivotLoose", crate::cost::PIVOT_LOOSE),
        ("PivotDescendant", crate::cost::PIVOT_DESCENDANT),
        ("ScaleScanned", crate::cost::SCALE_SCANNED),
        ("ScalePartPlaced", crate::cost::SCALE_PART_PLACED),
        ("ScalePartLoose", crate::cost::SCALE_PART_LOOSE),
        ("ScaleAttachment", crate::cost::SCALE_ATTACHMENT),
        ("ScaleJoint", crate::cost::SCALE_JOINT),
    ] {
        costs.set(name, seconds)?;
    }
    r.set("Cost", costs)?;
    let e = engine.clone();
    r.set(
        "Charge",
        super::lua_fn(lua, move |_, seconds: f64| {
            e.state.borrow().sched.charge_priced(seconds);
            Ok(())
        })?,
    )?;
    // SharedTableRegistry:GetSharedTable: a new handle of the SharedTable a name holds (shared_table.rs `fresh_handle`)
    r.set("SharedTableHandle", super::lua_fn(lua, |lua, held: LuaAnyUserData| super::shared_table::fresh_handle(lua, &held))?)?;
    // SharedTableRegistry's mark on a table it holds (shared_table.rs `mark_value`; anything else is left alone)
    r.set(
        "MarkSharedTable",
        super::lua_fn(lua, |_, value: LuaValue| {
            super::shared_table::mark_value(&value);
            Ok(())
        })?,
    )?;
    // SharedTableRegistry's name for a buffer: its contents, charged as a string the script built (shared_table.rs `buffer_name`)
    let e = engine.clone();
    r.set("BufferRegistryName", super::lua_fn(lua, move |lua, held: LuaValue| super::shared_table::buffer_name(lua, &e, &held))?)?;
    // A value given to a method's string parameter as a client reads it (convert.rs `string_param`: warned about with the script's
    // stack and cast to text), and the warning alone (`warn_string_param`): the string parameters of the service stubs
    let e = engine.clone();
    r.set(
        "StringParam",
        super::lua_fn(lua, move |lua, (value, argument): (LuaValue, usize)| lua.create_string(super::convert::string_param(lua, &e, &value, argument, true)?))?,
    )?;
    let e = engine.clone();
    r.set(
        "WarnStringParam",
        super::lua_fn(lua, move |lua, (value, argument): (LuaValue, usize)| {
            super::convert::warn_string_param(lua, &e, &value, argument);
            Ok(())
        })?,
    )?;
    // Model:PivotTo / ScaleTo: many part moves as one step (world::joints batches)
    let e = engine.clone();
    r.set(
        "BeginMoves",
        super::lua_fn(lua, move |_, ()| {
            crate::world::joints::begin_batch(&mut e.state.borrow_mut());
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "EndMoves",
        super::lua_fn(lua, move |_, ()| {
            let hooks = e.hooks.borrow();
            let mut state = e.state.borrow_mut();
            crate::world::joints::end_batch(&mut state, &e.reflection, &hooks);
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "Missing",
        super::lua_fn(lua, move |lua, (class_name, member): (String, String)| {
            e.missing(Some(lua), &class_name, &member);
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set("LogHistory", super::lua_fn(lua, move |lua, ()| {
        let state = e.state.borrow();
        let history = lua.create_table_with_capacity(state.log_history.len(), 0)?;
        for (index, line) in state.log_history.iter().enumerate() {
            let entry = lua.create_table_with_capacity(0, 3)?;
            entry.raw_set("message", line.text.as_str())?;
            let name = match line.kind {
                "warn" => "MessageWarning",
                "error" => "MessageError",
                "info" => "MessageInfo",
                _ => "MessageOutput",
            };
            let item = e.reflection.enum_item("MessageType", name).unwrap();
            entry.raw_set("messageType", enum_item_value(lua, &e, item)?)?;
            entry.raw_set("timestamp", line.timestamp)?;
            history.raw_set(index + 1, entry)?;
        }
        Ok(history)
    })?)?;
    // LogService's logging methods (services.luau): a line of a kind with the converted context MessageOut passes on
    let e = engine.clone();
    r.set(
        "LogLine",
        super::lua_fn(lua, move |_, (kind, message, context): (LuaString, LuaString, Option<LuaTable>)| {
            let kind = match &*kind.as_bytes() {
                b"info" => "info",
                b"warn" => "warn",
                b"error" => "error",
                _ => "print",
            };
            let context = context.map(|table| crate::value::Value::Lua(Box::new(LuaValue::Table(table))));
            e.output_with_context(kind, message.to_string_lossy(), None, context);
            Ok(())
        })?,
    )?;
    // a LogService error's `stack`: "Stack Begin", a line per frame of the script's own code, "Stack End"
    r.set(
        "LogStack",
        super::lua_fn(lua, |lua, trace: LuaString| {
            let mut lines = vec!["Stack Begin".to_string()];
            lines.extend(crate::stack_frames(&trace.to_string_lossy()));
            lines.push("Stack End".to_string());
            lua.create_string(lines.join("\n"))
        })?,
    )?;
    // a LogService error table turned into text ([`uncaught_log_error`])
    r.set(
        "LogErrorText",
        super::lua_fn(lua, |lua, text: LuaString| {
            let thread = lua.state() as usize;
            LOG_ERROR_TEXTS.with(|texts| {
                let mut texts = texts.borrow_mut();
                texts.count += 1;
                texts.thread = thread;
                texts.text = text.to_string_lossy();
            });
            Ok(())
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "LocalPlayer",
        super::lua_fn(lua, move |lua, ()| match e.world().local_player {
            Some(player) => instance_value(lua, &e, player),
            None => Ok(LuaValue::Nil),
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "GetEnumItem",
        super::lua_fn(lua, move |lua, (enum_name, item_name): (String, String)| match e.reflection.enum_item(&enum_name, &item_name) {
            Some(item) => enum_item_value(lua, &e, item),
            None => Ok(LuaValue::Nil),
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "NewSignal",
        super::lua_fn(lua, move |lua, name: String| {
            let signal = types::new_signal(&e, &name);
            signal_value(lua, &e, signal)
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "GetCallback",
        super::lua_fn(lua, move |_, (target, name): (LuaValue, String)| {
            let id = instance_of(&target)?;
            let Some(name) = e.reflection.name_id(&name) else { return Ok(LuaValue::Nil) };
            Ok(e.state.borrow().dom.at(id).callback(name).cloned().unwrap_or(LuaValue::Nil))
        })?,
    )?;
    // A thread the engine Luau parked (a BindableFunction's caller waiting for its callback that yielded) resumed at the next
    // deferred point, after deferred work the callback queued (measured, a 0.747 client and server: tools/parity/probes/
    // engine_threads.luau), without task.defer's price: the call that parked it is priced whole
    let e = engine.clone();
    r.set(
        "Defer",
        super::lua_fn(lua, move |_, thread: LuaThread| {
            super::instance::defer_thread(&e, thread, Vec::new());
            Ok(())
        })?,
    )?;
    // A BindableFunction's callback run on a thread of its own, its outcome handed to Finished (lua/bindable.rs run_own; no thread
    // price: the call that runs the callback is priced whole)
    let e = engine.clone();
    r.set(
        "RunOwn",
        super::lua_fn(lua, move |lua, (target, callback, finished, args): (LuaValue, LuaFunction, LuaFunction, LuaMultiValue)| {
            super::bindable::run_own(lua, &e, target, callback, finished, args)
        })?,
    )?;
    // BindableFunction:Invoke's copies of its callback's results and of its arguments (lua/bindable.rs), whose tables
    // start above the call's frame: the results alone, `self` below the arguments
    let e = engine.clone();
    r.set("Copied", super::wrapped(lua, engine, "Copied", move |lua, values| {
        let height = values.len();
        super::bindable::copied(lua, &e, values, height)
    })?)?;
    let e = engine.clone();
    r.set("CopiedArguments", super::wrapped(lua, engine, "CopiedArguments", move |lua, values| {
        let height = values.len() + 1;
        super::bindable::copied(lua, &e, values, height)
    })?)?;
    // KeyframeSequenceProvider:GetKeyframeSequenceAsync's copy of an animation, else nil and the error
    let e = engine.clone();
    r.set(
        "KeyframeSequence",
        super::lua_fn(lua, move |lua, uri: String| match crate::animation::source::sequence(&e, &uri) {
            Ok(sequence) => Ok((instance_value(lua, &e, sequence)?, None)),
            // (an `active://` id's error is Studio's own text, measured, and so are the one for what is no clip and those
            // for an id no client can read: content_id::animation)
            Err(error) if uri.trim().starts_with("active://") || error == crate::animation::source::NOT_VALID || error.starts_with(crate::content_id::INVALID) => {
                Ok((LuaValue::Nil, Some(error)))
            }
            Err(error) => Ok((LuaValue::Nil, Some(format!("Failed to load animation {uri}: {error}")))),
        })?,
    )?;
    // AnimationClipProvider:GetAnimationClipAsync's clip, else nil and the error, or false for an id that never answers
    let e = engine.clone();
    r.set(
        "AnimationClip",
        super::lua_fn(lua, move |lua, uri: String| match crate::animation::source::animation_clip(&e, &uri) {
            Ok(clip) => Ok((instance_value(lua, &e, clip)?, None)),
            Err(error) if error == crate::animation::source::NEVER => Ok((LuaValue::Boolean(false), None)),
            // (an `active://` id's error is Studio's own text, measured, and so are those for an id no client can read)
            Err(error) if uri.trim().starts_with("active://") || error.starts_with(crate::content_id::INVALID) => Ok((LuaValue::Nil, Some(error))),
            Err(error) => Ok((LuaValue::Nil, Some(format!("Failed to load animation {uri}: {error}")))),
        })?,
    )?;
    // ContentProvider:PreloadAsync's animations: the seconds until the ones it asks for arrive, and by the index of each
    // item that is or holds Animations, `{ { AnimationId, Loads }, ... }` for those that have an id
    let e = engine.clone();
    r.set(
        "PreloadAnimations",
        super::lua_fn(lua, move |lua, items: LuaTable| {
            let mut ids = Vec::new();
            for (index, item) in items.sequence_values::<LuaValue>().enumerate() {
                if let Ok(id) = instance_of(&item?) {
                    ids.push((index + 1, id));
                }
            }
            let (wait, reports) = crate::animation::preload(&e, &ids.iter().map(|&(_, id)| id).collect::<Vec<_>>());
            let found = lua.create_table()?;
            for ((index, _), report) in ids.iter().zip(reports) {
                let Some(report) = report else { continue };
                let list = lua.create_table()?;
                for (uri, loads) in report {
                    list.raw_push(lua.create_sequence_from([LuaValue::String(lua.create_string(&uri)?), LuaValue::Boolean(loads)])?)?;
                }
                found.raw_set(*index, list)?;
            }
            Ok((wait, found))
        })?,
    )?;
    // EditableImage pixels (crate::images): an asset's or file's as (width, height, buffer), else nil
    // and the error; a snapshot of a drawn image written as a PNG, its path
    let e = engine.clone();
    r.set(
        "LoadImage",
        super::lua_fn(lua, move |lua, uri: String| match crate::images::load(&e, &uri) {
            Ok(pixels) => Ok((LuaValue::Number(pixels.width as f64), LuaValue::Number(pixels.height as f64), LuaValue::Buffer(lua.create_buffer(pixels.rgba)?), None)),
            Err(error) => Ok((LuaValue::Nil, LuaValue::Nil, LuaValue::Nil, Some(error))),
        })?,
    )?;
    let e = engine.clone();
    r.set(
        "SaveImage",
        super::lua_fn(lua, move |_, (width, height, pixels): (u32, u32, mlua::Buffer)| match crate::images::save(&e, width, height, &pixels.to_vec()) {
            Ok(path) => Ok((Some(path.to_string_lossy().replace('\\', "/")), None)),
            Err(error) => Ok((None, Some(error))),
        })?,
    )?;
    // AssetService:CreateDataModelContentAsync (lua/services.luau): a copy of an EditableMesh as it is now (its file URI,
    // else nil and the error), and the Opaque Content that names a copy (each call's its own)
    let e = engine.clone();
    r.set(
        "MeshCopy",
        super::lua_fn(lua, move |_, mesh: LuaValue| match crate::meshes::data_model_copy(&e, instance_of(&mesh)?) {
            Ok(uri) => Ok((Some(uri), None)),
            Err(problem) => Ok((None, Some(problem))),
        })?,
    )?;
    r.set(
        "OpaqueContent",
        super::lua_fn(lua, |lua, uri: LuaString| {
            static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let content = crate::value::ContentValue::Opaque(serial, crate::value::Str::new(&uri.as_bytes()));
            lua.create_any_userdata(types::LContent::new(lua, content))
        })?,
    )?;
    // A method's int64 argument (convert::int64_argument) as its decimal text, which keeps every int64 apart (a number holds
    // integers exactly to 2^53 only); else nil and the cast error, which the stub raises without a position
    let e = engine.clone();
    r.set(
        "Int64Argument",
        super::lua_fn(lua, move |_, (value, index): (LuaValue, usize)| {
            Ok(match convert::int64_argument(&e, &value, index) {
                Ok(number) => (Some(number.to_string()), None),
                Err(message) => (None, Some(message)),
            })
        })?,
    )?;
    // HttpService JSON: Lune's serde (sorted keys, empty tables as `{}`, integral numbers as integers)
    r.set(
        "JsonEncode",
        super::lua_fn(lua, |lua, value: LuaValue| {
            let safe = crate::services::json_safe(lua, &value, &mut Vec::new(), &mut super::bindable::Tally::default()).map_err(|flow| match flow {
                Flow::Error(error) => LuaError::runtime(error.message),
                Flow::Fail(error) => error,
                Flow::Yield => LuaError::runtime("JsonEncode yielded"),
                Flow::Raised(value) => LuaError::runtime(Flow::raised_text(&value)),
            })?;
            let json: serde_json::Value = lua.from_value_with(
                safe,
                LuaDeserializeOptions::new().sort_keys(true).deny_recursive_tables(false).deny_unsupported_types(true),
            )?;
            serde_json::to_string(&json).map_err(LuaError::external)
        })?,
    )?;
    r.set(
        "JsonDecode",
        super::lua_fn(lua, |lua, text: LuaValue| {
            let LuaValue::String(text) = text else { return Ok((false, LuaValue::Nil)) };
            // (services port) Lune's serde.decode number semantics (engine::lune_json)
            let decoded = std::str::from_utf8(&text.as_bytes()).map_err(|e| e.to_string()).and_then(crate::lune_json::decode);
            match decoded {
                Ok(value) => Ok((
                    true,
                    lua.to_value_with(
                        &value,
                        LuaSerializeOptions::new().set_array_metatable(false).serialize_none_to_null(false).serialize_unit_to_null(false),
                    )?,
                )),
                Err(_) => Ok((false, LuaValue::Nil)),
            }
        })?,
    )?;
    let options = lua.create_table()?;
    let device = &engine.options.profile.device;
    options.set("TopbarInset", device.topbar_inset())?;
    options.set("ViewportWidth", device.viewport().0)?;
    options.set("ViewportHeight", device.viewport().1)?;
    r.set("Options", options)?;
    // R.Profile: the client profile's JSON where it sets a field the service stubs read (client_profile.rs), else nil
    let profile = profile_table(lua, engine, &engine.profile())?;
    if !profile.is_nil() {
        r.set("Profile", profile)?;
    }
    lua.set_named_registry_value(STUBS, r.clone())?;
    // Player:GetMouse()'s PlayerMouse, made on the first call (world/mouse.rs)
    let e = engine.clone();
    r.set("PlayerMouse", super::lua_fn(lua, move |lua, ()| crate::world::mouse::create(lua, &e))?)?;

    let shared = engine.lua.borrow().shared.clone().ok_or_else(|| LuaError::runtime("shared globals"))?;
    let services = modules::load_engine_luau(lua, SERVICES, "=rur.services", Some(shared.clone()))?;
    crate::cost::engine_luau(lua, services)?.call::<()>(r.clone())?;
    // A server's own services (DataStores, MemoryStores, MessagingService, the server side of remotes)
    if engine.options.server() {
        let server = modules::load_engine_luau(lua, SERVER, "=rur.server", Some(shared))?;
        crate::cost::engine_luau(lua, server)?.call::<()>(r.clone())?;
        if let LuaValue::Function(set_join_data) = r.get::<LuaValue>("SetJoinData")? {
            lua.set_named_registry_value(SET_JOIN_DATA, set_join_data)?;
        }
    }
    // The client profile's answers, where it sets what the stubs read (else the stubs stay as they are)
    if r.raw_get::<LuaValue>("Profile")?.is_table() {
        load_profile(lua, engine, &r)?;
    }
    engine.clear_member_cache();
    Ok(())
}
