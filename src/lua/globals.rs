//! Engine.Start (port of `runtime/engine/init.luau`) and the shared global environment scripts
//! run in (port of `runtime/engine/globals.luau`).

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    actors, Args, Flow, LuaRefs, Prelude, enum_item_value, instance, instance_value, lock_metatable, modules, none, raise_plain,
    library_function, named, refs::{self, Hold}, types, values,
    cfunction::{self, Kind::Library},
};
use crate::{
    Engine, InstanceId, K, NameId, World,
    dom::flags,
    instance as core,
    scheduler::{self, TimerKind},
    services,
    value::{Str, Value},
};

const PRELUDE: &str = include_str!("prelude.luau");
const VECTOR3: &str = include_str!("vector3.luau");

/// Services Engine.Start creates, in its order.
const START_SERVICES: &[&str] = &[
    "ReplicatedStorage",
    "ReplicatedFirst",
    "StarterGui",
    "StarterPlayer",
    "Lighting",
    "CoreGui",
    "RunService",
    "TweenService",
    "UserInputService",
    "GuiService",
    "TextService",
    "HttpService",
    "SoundService",
    "CollectionService",
];

fn primitive<F>(lua: &Lua, engine: &Rc<Engine>, f: F) -> LuaResult<LuaFunction>
where
    F: Fn(&Lua, &Rc<Engine>, LuaMultiValue) -> LuaResult<LuaMultiValue> + 'static,
{
    let engine = engine.clone();
    lua.create_function(move |lua, args| f(lua, &engine, args))
}

/// The VM a prelude call names: nil for the main VM, else an Actor's key (modules::vm_key).
fn vm_of(value: &LuaValue) -> modules::Vm {
    match value {
        LuaValue::Integer(key) => Some(*key as u64),
        LuaValue::Number(key) => Some(*key as u64),
        _ => None,
    }
}

/// The thread task.spawn / defer / delay (`call`) runs: a thread as it is, a function in a new thread. Roblox's errors
/// (measured in a playing client, Studio 0.744, tools/parity/probes/task_order.luau): `invalid argument #1 to 'spawn'
/// (function or thread expected)` (#2 for delay), at the line that called it, as a C function's argument check places it (none
/// from `pcall(task.spawn, 5)`; measured, Studio 0.747, engine_functions.luau), and for a thread an earlier task.defer or
/// task.delay queued that has not resumed yet, `Cannot call task.spawn on a thread that is already 'deferred' in the task
/// library` (measured through pcall only: raised without a position).
fn to_thread(lua: &Lua, engine: &Engine, value: &LuaValue, index: usize, call: &str) -> Result<LuaThread, Flow> {
    match value {
        LuaValue::Thread(thread) => {
            if let Some(state) = engine.state.borrow().sched.task_state(thread) {
                return raise_plain(format!("Cannot call task.{call} on a thread that is already '{state}' in the task library"));
            }
            Ok(thread.clone())
        }
        LuaValue::Function(function) => Ok(lua.create_thread(function.clone())?),
        _ => super::raise_at_caller(format!("invalid argument #{index} to '{call}' (function or thread expected)")),
    }
}

/// The thread the legacy spawn / delay run (their errors were not measured).
fn legacy_thread(lua: &Lua, value: &LuaValue) -> Result<LuaThread, Flow> {
    match value {
        LuaValue::Thread(thread) => Ok(thread.clone()),
        LuaValue::Function(function) => Ok(lua.create_thread(function.clone())?),
        _ => raise_plain("invalid argument #1 (function or thread expected)"),
    }
}

/// task.delay's and task.wait's seconds: nil is none; a number, or a string Luau reads as one, goes; anything else is
/// `invalid argument #1 to 'delay' (number expected, got string)` (measured likewise), at the line that called it (task.wait's,
/// measured: engine_functions.luau, as the task library's other argument checks).
fn task_seconds(lua: &Lua, engine: &Engine, value: &LuaValue, call: &str) -> Result<Option<f64>, Flow> {
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::Number(number) => Ok(Some(*number)),
        LuaValue::Integer(number) => Ok(Some(*number as f64)),
        LuaValue::String(_) => match lua.coerce_number(value.clone())? {
            Some(number) => Ok(Some(number)),
            None => super::raise_at_caller(format!("invalid argument #1 to '{call}' (number expected, got string)")),
        },
        other => super::raise_at_caller(format!("invalid argument #1 to '{call}' (number expected, got {})", super::typeof_value(engine, other))),
    }
}

/// A function handed to task.spawn / defer / delay gets a thread of its own (cost::THREAD); a thread is used as it is.
fn charge_thread(lua: &Lua, engine: &Engine, value: &LuaValue, call: &str) {
    // (a thread handed over costs a client what a function's new thread does: cost.rs THREAD)
    if matches!(value, LuaValue::Function(_) | LuaValue::Thread(_)) {
        let state = engine.state.borrow();
        state.sched.charge(crate::cost::THREAD);
        if crate::boundary::active() && !state.sched.is_muted() {
            crate::boundary::enter(lua.current_thread().state());
            crate::boundary::record_named(crate::boundary::Kind::Method, "task", call, crate::cost::THREAD);
        }
    }
}

fn load_prelude(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let r = lua.create_table()?;
    r.set("SIGNAL", super::signal_marker())?;
    // (the prelude's thread-making functions charge a script as task.spawn does)
    r.set("ThreadCost", crate::cost::THREAD)?;
    r.set("VectorLerpCost", crate::cost::VECTOR_LERP)?;
    r.set("VectorFuzzyEqCost", crate::cost::VECTOR_FUZZY_EQ)?;
    // (what copying a RaycastParams' or OverlapParams' instance list costs a script: prelude.luau ListCopy, AddToFilter)
    r.set("ParamsListWriteCost", crate::cost::PARAMS_LIST_WRITE)?;
    r.set("ParamsWrittenCost", crate::cost::PARAMS_WRITTEN)?;
    r.set("ParamsFilterWrittenCost", crate::cost::PARAMS_FILTER_WRITTEN)?;
    r.set("ParamsListReadCost", crate::cost::PARAMS_LIST_READ)?;
    r.set("ParamsReadCost", crate::cost::PARAMS_READ)?;
    // Vector3:Angle's atan2 in floats (the C runtime's atan2f), as a playing client takes it (Studio 0.744,
    // tools/parity/probes/cframe_order.luau and datatypes.luau: math.atan2's doubles differ in the last digits)
    r.set("Atan2f", lua.create_function(|_, (y, x): (f64, f64)| Ok(f64::from((y as f32).atan2(x as f32))))?)?;
    let e = engine.clone();
    r.set(
        "Charge",
        lua.create_function(move |_, seconds: f64| {
            let state = e.state.borrow();
            state.sched.charge(seconds);
            if !state.sched.is_muted() {
                crate::boundary::add_cost(seconds);
            }
            Ok(())
        })?,
    )?;
    r.set(
        "EnumItemName",
        lua.create_function(|lua, value: LuaValue| {
            let engine = lua.app_data_ref::<Rc<Engine>>().expect("engine").clone();
            Ok(match &value {
                LuaValue::UserData(ud) => match ud.borrow::<types::LEnumItem>() {
                    Ok(item) => Some(engine.reflection.enum_item_info(item.0).name.to_string()),
                    Err(_) => None,
                },
                _ => None,
            })
        })?,
    )?;
    r.set(
        "EnumItemType",
        lua.create_function(|lua, value: LuaValue| {
            let engine = lua.app_data_ref::<Rc<Engine>>().expect("engine").clone();
            Ok(match &value {
                LuaValue::UserData(ud) => match ud.borrow::<types::LEnumItem>() {
                    Ok(item) => Some(engine.reflection.enum_item_type_name(item.0).to_string()),
                    Err(_) => None,
                },
                _ => None,
            })
        })?,
    )?;
    r.set(
        "GetEnumItem",
        lua.create_function(|lua, (enum_name, item_name): (String, String)| {
            let engine = lua.app_data_ref::<Rc<Engine>>().expect("engine").clone();
            match engine.reflection.enum_item(&enum_name, &item_name) {
                Some(item) => enum_item_value(lua, &engine, item),
                None => Ok(LuaValue::Nil),
            }
        })?,
    )?;
    r.set(
        "Output",
        primitive(lua, engine, |_, engine, args| {
            let args = Args::new(args);
            let kind = match args.string(0).map(|s| s.to_string_lossy()).as_deref() {
                Some("warn") => "warn",
                Some("error") => "error",
                _ => "print",
            };
            let message = args.string(1).map(|s| s.to_string_lossy()).unwrap_or_default();
            engine.output(kind, message, None);
            Ok(none())
        })?,
    )?;
    // print and warn's mark on a SharedTable they write (lua/shared_table.rs `mark_value`)
    r.set(
        "MarkSharedTable",
        lua.create_function(|_, value: LuaValue| {
            super::shared_table::mark_value(&value);
            Ok(())
        })?,
    )?;
    // ValueCurveKey's table values, copied as an event's argument is (lua/bindable.rs): `R.Copied(height, value)`, the height
    // of the frame the copy starts above; and SecurityCapabilities.fromCurrent's report in the edit run context, where a
    // plugin's set was not measured (prelude.luau)
    r.set("Copied", primitive(lua, engine, |lua, engine, mut args| {
        let height = match args.pop_front() {
            Some(LuaValue::Integer(height)) => usize::try_from(i64::from(height)).unwrap_or(0),
            Some(LuaValue::Number(height)) if height >= 0.0 => height as usize,
            _ => 0,
        };
        super::encode(lua, super::bindable::copied(lua, engine, args, height))
    })?)?;
    let e = engine.clone();
    r.set(
        "Missing",
        lua.create_function(move |lua, (class_name, member): (String, String)| {
            e.missing(Some(lua), &class_name, &member);
            Ok(())
        })?,
    )?;
    r.set("EditMode", engine.options.edit_mode())?;
    // The prelude's engine functions as scripts see them (lua/cfunction.rs): `EngineFunction(name, served, luau, function)`, one
    // C function named `name` over `function`, a Rust function answering the call protocol or (`luau`) the prelude's own Luau
    r.set(
        "EngineFunction",
        lua.create_function(|lua, (name, served, luau, function): (LuaString, bool, bool, LuaFunction)| {
            let kind = if served { cfunction::Kind::Served } else { cfunction::Kind::Library };
            let name = name.to_string_lossy();
            if luau { cfunction::over_luau(lua, &name, kind, function) } else { cfunction::over_rust(lua, &name, kind, function) }
        })?,
    )?;
    r.set("Nothing", lua.create_function(|_, _: LuaMultiValue| Ok(()))?)?;
    // utf8.nfcnormalize and utf8.nfdnormalize (lua/normalize.rs)
    r.set("NfcNormalize", primitive(lua, engine, |lua, engine, args| super::encode(lua, super::normalize::normalize(lua, engine, args, true)))?)?;
    r.set("NfdNormalize", primitive(lua, engine, |lua, engine, args| super::encode(lua, super::normalize::normalize(lua, engine, args, false)))?)?;
    // task library primitives (scheduler.luau Task)
    r.set(
        "TaskSpawn",
        primitive(lua, engine, |lua, engine, args| {
            super::encode(lua, (|| {
                let args = Args::new(args);
                let thread = to_thread(lua, engine, args.get(0), 1, "spawn")?;
                refs::admit(engine, Hold::Work, "task.spawn")?;
                charge_thread(lua, engine, args.get(0), "spawn");
                if let Some(script) = unsafe { super::script_of_thread(lua.current_thread().state()) } {
                    engine.state.borrow_mut().sched.set_thread_script(&thread, script);
                }
                scheduler::resume(lua, engine, &thread, args.rest(1));
                Ok(values([LuaValue::Thread(thread)]))
            })())
        })?,
    )?;
    r.set(
        "TaskDefer",
        primitive(lua, engine, |lua, engine, args| {
            super::encode(lua, (|| {
                let args = Args::new(args);
                let thread = to_thread(lua, engine, args.get(0), 1, "defer")?;
                refs::admit(engine, Hold::Work, "task.defer")?;
                charge_thread(lua, engine, args.get(0), "defer");
                let caller_script = unsafe { super::script_of_thread(lua.current_thread().state()) };
                let rest: Vec<Value> = args.0.iter().skip(1).map(super::convert::from_lua).collect();
                if engine.state.borrow().sched.can_defer_event() {
                    let mut state = engine.state.borrow_mut();
                    if let Some(script) = caller_script {
                        state.sched.set_thread_script(&thread, script);
                    }
                    state.sched.defer_task(scheduler::JobTarget::Thread(thread.clone()), rest);
                    state.sched.set_task_state(&thread, "deferred");
                } else {
                    // Past the cut of a chain the thread is not queued; the call returns and Roblox logs an error
                    // (measured in a playing client, Studio 0.744: pcall(task.defer, f) is ok, the chain stops)
                    let message = format!("Maximum re-entrancy depth ({}) exceeded calling task.defer", scheduler::MAX_REENTRANCY);
                    engine.output("error", message, None);
                }
                Ok(values([LuaValue::Thread(thread)]))
            })())
        })?,
    )?;
    r.set(
        "TaskDelay",
        primitive(lua, engine, |lua, engine, args| {
            super::encode(lua, (|| {
                let args = Args::new(args);
                let delay = task_seconds(lua, engine, args.get(0), "delay")?.unwrap_or(0.0);
                let thread = to_thread(lua, engine, args.get(1), 2, "delay")?;
                refs::admit(engine, Hold::Work, "task.delay")?;
                charge_thread(lua, engine, args.get(1), "delay");
                // (its timer, a thread or a function's: cost.rs DELAYED)
                engine.state.borrow().sched.charge(crate::cost::DELAYED);
                crate::boundary::add_cost(crate::cost::DELAYED);
                let caller_script = unsafe { super::script_of_thread(lua.current_thread().state()) };
                let rest: Vec<Value> = args.0.iter().skip(2).map(super::convert::from_lua).collect();
                let mut state = engine.state.borrow_mut();
                if let Some(script) = caller_script {
                    state.sched.set_thread_script(&thread, script);
                }
                state.sched.schedule_call(TimerKind::Delay, delay, thread.clone(), rest);
                state.sched.set_task_state(&thread, "delayed");
                Ok(values([LuaValue::Thread(thread)]))
            })())
        })?,
    )?;
    // (task.wait and wait: the thread registered, a yield from the C function, lua/cfunction.rs. Where the thread cannot yield
    // both raise `thread is not yieldable` first, before task.wait reads its argument, and register nothing: lua/mod.rs
    // NOT_YIELDABLE. task.wait's argument error names the line that called it, none from pcall(task.wait, "x"): measured in a
    // 0.747 playing client and server, tools/parity/probes/yield_refused.luau and task_order.luau)
    r.set(
        "ScheduleWait",
        primitive(lua, engine, |lua, engine, args| {
            super::encode(lua, (|| {
                let args = Args::new(args);
                let thread = lua.current_thread();
                super::refuse_unyieldable(&thread, super::NOT_YIELDABLE)?;
                let delay = match task_seconds(lua, engine, args.get(0), "wait") {
                    Err(Flow::Error(error)) => return super::raise_at_caller(error.message),
                    seconds => seconds?.unwrap_or(0.0),
                };
                refs::admit(engine, Hold::Wait, "task.wait")?;
                engine.state.borrow_mut().sched.schedule_call(TimerKind::Wait, delay, thread, Vec::new());
                Err(Flow::Yield)
            })())
        })?,
    )?;
    r.set(
        "ScheduleLegacyWait",
        primitive(lua, engine, |lua, engine, args| {
            super::encode(lua, (|| {
                let args = Args::new(args);
                let thread = lua.current_thread();
                super::refuse_unyieldable(&thread, super::NOT_YIELDABLE)?;
                let duration = args.number(0);
                refs::admit(engine, Hold::Wait, "wait")?;
                engine.state.borrow_mut().sched.schedule_legacy(TimerKind::LegacyWait, duration, thread);
                Err(Flow::Yield)
            })())
        })?,
    )?;
    r.set(
        "TaskCancel",
        primitive(lua, engine, |lua, engine, args| {
            super::encode(lua, (|| {
                let args = Args::new(args);
                // (the running thread and one that resumed it raise `cannot cancel thread`, without a position, before anything
                // else; measured in a playing client, Studio 0.744, tools/parity/probes/task_order.luau; the argument's error is
                // at the line that called it, none from pcall: Studio 0.747, engine_functions.luau)
                let LuaValue::Thread(thread) = args.get(0) else {
                    let got = super::typeof_value(engine, args.get(0));
                    return super::raise_at_caller(format!("invalid argument #1 to 'cancel' (thread expected, got {got})"));
                };
                if matches!(thread.status(), LuaThreadStatus::Running | LuaThreadStatus::Normal) {
                    return raise_plain("cannot cancel thread");
                }
                if thread.status() == LuaThreadStatus::Resumable {
                    let close = engine.lua.borrow().coroutine_close.clone();
                    if let Some(close) = close {
                        let _ = close.call::<LuaMultiValue>(thread.clone());
                    }
                }
                // (after the close: only a thread that is still alive is listed)
                let mut state = engine.state.borrow_mut();
                state.sched.cancel(thread);
                state.sched.clear_task_state(thread);
                Ok(none())
            })())
        })?,
    )?;
    let require_begin = primitive(lua, engine, |lua, engine, args| {
        let args = Args::new(args);
        modules::require_begin(lua, engine, (args.get(0).clone(), args.get(1).clone(), vm_of(args.get(2))))
    })?;
    r.set("RequireBegin", require_begin.clone())?;
    // task.synchronize / task.desynchronize of a parallel handler's thread: a yield from their C function when its thread parks
    // (actors.rs; a thread that cannot yield raises), nothing when it is in the phase already
    for (name, to_serial) in [("Synchronize", true), ("Desynchronize", false)] {
        r.set(
            name,
            primitive(lua, engine, move |lua, engine, _| {
                super::encode(lua, actors::park(lua, engine, to_serial).and_then(|park| if park { Err(Flow::Yield) } else { Ok(none()) }))
            })?,
        )?;
    }
    let require_finish = primitive(lua, engine, |lua, engine, args| {
        let args = Args::new(args);
        let LuaValue::Table(results) = args.get(1).clone() else {
            return Err(LuaError::runtime("RequireFinish: results expected"));
        };
        modules::require_finish(lua, engine, (args.get(0).clone(), results, vm_of(args.get(2))))
    })?;
    r.set("RequireFinish", require_finish.clone())?;
    {
        let mut refs = engine.lua.borrow_mut();
        refs.require_begin = Some(require_begin.clone());
        refs.require_finish = Some(require_finish.clone());
    }
    // a script's require (prelude.luau Environment): `MakeRequire(Script, Vm)`, a C function of its own (lua/builtins.rs)
    r.set(
        "MakeRequire",
        lua.create_function(move |lua, (script, vm): (LuaValue, LuaValue)| super::builtins::require_function(lua, script, vm, require_begin.clone(), require_finish.clone()))?,
    )?;

    // Vector3 is the native vector type, as in Roblox (vector3.luau, loaded by the prelude): vectors
    // get their members through the type's metatable, and Lune's CFrame / Ray / Region3 convert,
    // through their own metatables reached here (they are locked for scripts)
    // (Vector3's native methods with a client's argument errors: lua/vector_methods.rs)
    r.set("VectorMethods", super::vector_methods::methods(lua)?)?;
    r.set(
        "MakeVectorNamecall",
        lua.create_function(|lua, extras: LuaTable| super::vector_methods::namecall(lua, extras))?,
    )?;
    r.set(
        "Vector3",
        crate::cost::engine_luau(
            lua,
            modules::load_engine_luau(lua, VECTOR3, "=rur.vector3", None)?,
        )?,
    )?;
    r.set(
        "RawMetatable",
        lua.create_function(|lua, value: LuaAnyUserData| unsafe {
            lua.exec_raw::<LuaValue>(value, |state| {
                if mlua::ffi::lua_getmetatable(state, -1) == 0 {
                    mlua::ffi::lua_pushnil(state);
                }
                mlua::ffi::lua_remove(state, -2);
            })
        })?,
    )?;
    r.set(
        "SetVectorMetatable",
        lua.create_function(|lua, metatable: LuaTable| {
            lua.set_type_metatable::<mlua::Vector>(Some(metatable));
            Ok(())
        })?,
    )?;

    let roblox = lune_roblox::module(lua.clone())?;
    let prelude = modules::load_engine_luau(lua, PRELUDE, "=rur.prelude", None)?;
    let exports: LuaTable = crate::cost::engine_luau(lua, prelude)?.call((r, roblox))?;
    name_datatype_methods(lua, &exports)?;
    let prelude = Prelude {
        wrap: exports.get("Wrap")?,
        // typeof as a C function over the same registry (lua/raw.rs)
        typeof_fn: super::raw::typeof_function(lua, exports.get("TypeRegistry")?)?,
        register: exports.get("Register")?,
        traceback: exports.get("Traceback")?,
        exports,
    };
    let globals = lua.globals();
    let coroutine: LuaTable = globals.get("coroutine")?;
    let math: LuaTable = globals.get("math")?;
    let mut refs = engine.lua.borrow_mut();
    refs.coroutine_close = Some(coroutine.get("close")?);
    refs.randomseed = Some(math.get("randomseed")?);
    refs.prelude = Some(Rc::new(prelude));
    drop(refs);
    super::strip_callback_tracebacks(lua)
}

/// The methods of Lune's datatypes with methods (CFrame, Color3, UDim2, Vector2, Ray, Region3), read as values (`cf.Lerp`) or
/// called with `:` alike: each a C function of its name (lua/cfunction.rs), as a client's are (measured, Studio 0.747:
/// `debug.info(cf.Lerp, "sln")` gives `[C]`, -1 and `Lerp`, engine_functions.luau). The type's methods are the table its
/// `__namecall` dispatches over (lune-roblox's MethodMembers, its first upvalue), which its `__index` reads too. They were C
/// functions already, and stay priced as one ([`cfunction::Kind::Library`]).
fn name_datatype_methods(lua: &Lua, exports: &LuaTable) -> LuaResult<()> {
    let datatypes: LuaTable = exports.get("Datatypes")?;
    let zero = || LuaValue::Vector(mlua::Vector::new(0.0, 0.0, 0.0));
    for (name, args) in [("CFrame", vec![]), ("Color3", vec![]), ("UDim2", vec![]), ("Vector2", vec![]), ("Ray", vec![zero(), zero()]), ("Region3", vec![zero(), zero()])] {
        let library: LuaTable = datatypes.get(name)?;
        let new: LuaFunction = library.get("new")?;
        let Ok(sample) = new.call::<LuaValue>(LuaMultiValue::from_vec(args)) else { continue };
        // SAFETY: the sample is pushed as the argument; its metatable's __namecall's first upvalue (or nil) is left as the one result
        let methods: LuaValue = unsafe {
            lua.exec_raw(sample, |state| {
                use mlua::ffi;
                let found = ffi::lua_getmetatable(state, 1) != 0 && {
                    ffi::lua_pushstring(state, c"__namecall".as_ptr());
                    ffi::lua_rawget(state, -2);
                    ffi::lua_iscfunction(state, -1) != 0 && !ffi::lua_getupvalue(state, -1, 1).is_null()
                };
                if found {
                    ffi::lua_replace(state, 1);
                } else {
                    ffi::lua_pushnil(state);
                    ffi::lua_replace(state, 1);
                }
                ffi::lua_settop(state, 1);
            })?
        };
        let LuaValue::Table(methods) = methods else { continue };
        let functions = methods.pairs::<LuaString, LuaValue>().filter_map(|pair| match pair {
            Ok((key, LuaValue::Function(function))) => Some((key, function)),
            _ => None,
        });
        for (key, function) in functions.collect::<Vec<_>>() {
            let key_name = key.to_string_lossy();
            methods.raw_set(key, cfunction::over_rust(lua, &key_name, Library, function)?)?;
        }
    }
    Ok(())
}

/// Hooks.MissingChild (edit mode): the running client's PlayerGui a script waits for or indexes
/// is created on first use, and PlayerScripts children are copied from StarterPlayerScripts as a
/// starting client's would be. FindFirstChild never asks. Indexing (`__index`) cannot yield, so
/// the client gui mount (rur.json clientGui) waits for a lookup that may: see
/// [`missing_child_yieldable`].
pub fn missing_child(lua: &Lua, engine: &Rc<Engine>, parent: InstanceId, name: &[u8]) -> LuaResult<Option<InstanceId>> {
    missing_child_in(lua, engine, parent, name, false)
}

/// Hooks.MissingChild from a lookup that may yield (WaitForChild): a pending client gui mount
/// (init.luau MountClient) runs first. (Added for the services port: the one caller is
/// lua/instance.rs wait_for_child.)
pub fn missing_child_yieldable(lua: &Lua, engine: &Rc<Engine>, parent: InstanceId, name: &[u8]) -> LuaResult<Option<InstanceId>> {
    missing_child_in(lua, engine, parent, name, true)
}

/// init.luau MountClient: Engine.ClientGuiMount once, from a yieldable lookup. It runs as a
/// plain call here (a Luau yield inside it cannot cross the Rust frame: the mount then fails with
/// a warning where the Luau runtime would wait).
fn mount_client(lua: &Lua, engine: &Rc<Engine>, yieldable: bool) -> LuaResult<()> {
    if !yieldable {
        return Ok(());
    }
    let mount = {
        let mut state = engine.state.borrow_mut();
        if !state.project.client_mount_pending {
            return Ok(());
        }
        state.project.client_mount_pending = false;
        state.project.client_gui_mount.clone()
    };
    if let Some(mount) = mount {
        if let Err(error) = mount.call::<()>(()) {
            engine.warn(format!("rur.json clientGui: {}", scheduler::error_message(&error)));
        }
        let _ = lua;
    }
    Ok(())
}

/// StarterGui's child `name` copied into `gui` (FindFirstChild on StarterGui: Studio-only children
/// materialize first, so a gui that exists only in the place is copied too).
fn copy_starter_gui(engine: &Rc<Engine>, gui: InstanceId, name: &[u8]) -> Option<InstanceId> {
    let starter = {
        let state = engine.state.borrow();
        state.dom.find_child_of_class(engine.world().game, engine.reflection.class_id("StarterGui").unwrap_or(u32::MAX))
    };
    let template = starter.and_then(|s| instance::find_first_child(engine, s, Some(name), false))?;
    let copy = instance::clone(engine, template)?;
    instance::set_parent(engine, copy, Some(gui)).ok();
    Some(copy)
}

fn missing_child_in(lua: &Lua, engine: &Rc<Engine>, parent: InstanceId, name: &[u8], yieldable: bool) -> LuaResult<Option<InstanceId>> {
    if !engine.options.edit_mode() {
        // A playing client's PlayerGui holds StarterGui's copies (Roblox copies them when the local
        // player spawns): copied when first looked up, as the edit-mode PlayerGui below. What the
        // client's own boot builds there (rur.json clientGui) is not mounted: that is the whole client.
        let Some(player) = engine.world().local_player else { return Ok(None) };
        let is_player_gui = {
            let state = engine.state.borrow();
            let instance = state.dom.at(parent);
            instance.parent == Some(player) && Some(instance.class) == engine.reflection.class_id("PlayerGui")
        };
        return Ok(if is_player_gui { copy_starter_gui(engine, parent, name) } else { None });
    }
    let Some(player) = engine.world().local_player else { return Ok(None) };
    let reflection = &engine.reflection;
    if parent == player && name == b"PlayerGui" {
        let existing = engine.state.borrow().project.client_gui;
        if let Some(gui) = existing {
            mount_client(lua, engine, yieldable)?;
            return Ok(Some(gui));
        }
        let gui = {
            let hooks = engine.hooks.borrow();
            let mut state = engine.state.borrow_mut();
            let class = reflection.class_id("PlayerGui").expect("PlayerGui");
            let gui = core::create_named(&mut state, reflection, class, "PlayerGui");
            let _ = core::set_parent(&mut state, reflection, &hooks, gui, Some(player));
            state.project.client_gui = Some(gui);
            state.project.client_mount_pending = state.project.client_gui_mount.is_some();
            gui
        };
        engine.warn(
            "edit mode has no PlayerGui; created one as a play test would (in Studio this story previews only while playing)",
        );
        mount_client(lua, engine, yieldable)?;
        return Ok(Some(gui));
    }
    let parent_class = engine.state.borrow().dom.at(parent).class;
    let player_scripts = reflection.class_id("PlayerScripts");
    if Some(parent_class) == player_scripts && engine.state.borrow().dom.at(parent).parent == Some(player) {
        // FindFirstChild, as the reference does: materializes Studio-only children first
        let starter = {
            let state = engine.state.borrow();
            state.dom.find_child_of_class(engine.world().game, reflection.class_id("StarterPlayer").unwrap_or(u32::MAX))
        };
        let scripts = starter.and_then(|s| instance::find_first_child(engine, s, Some(b"StarterPlayerScripts"), false));
        let template = scripts.and_then(|s| instance::find_first_child(engine, s, Some(name), false));
        if let Some(template) = template {
            if let Some(copy) = instance::clone(engine, template) {
                instance::set_parent(engine, copy, Some(parent)).ok();
                return Ok(Some(copy));
            }
        }
        return Ok(None);
    }
    let client_gui = engine.state.borrow().project.client_gui;
    if client_gui == Some(parent) {
        mount_client(lua, engine, yieldable)?;
        let built = instance::find_first_child(engine, parent, Some(name), false);
        if let Some(built) = built {
            return Ok(Some(built));
        }
        // StarterGui:FindFirstChild(Name), as the reference does: it materializes StarterGui's
        // Studio-only children first, so a gui that exists only in the place (the game place's
        // StarterGui.Windows) is copied into PlayerGui. A raw child lookup missed it and
        // PlayerGui:WaitForChild("Windows") yielded forever.
        if let Some(copy) = copy_starter_gui(engine, parent, name) {
            return Ok(Some(copy));
        }
    }
    Ok(None)
}

/// Engine.Start: reflection-backed types, behaviours, the DataModel with its services, the local
/// player, camera and mouse, and the shared script environment.
pub fn start(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    lua.set_app_data(engine.clone());
    // Scripts flush subnormal results to zero, as Roblox's engine runs them (lua/float_mode.rs)
    super::float_mode::start(lua);
    lua.set_compiler(modules::compiler());
    // The string metatable is locked: getmetatable("") is a string in a playing client (measured, Studio 0.744:
    // tools/parity/probes/values.luau; the text is the lock text Roblox's other metatables give, not measured here)
    lua.load(r#"getmetatable("").__metatable = "The metatable is locked""#).set_name("=ree.strings").exec()?;
    // Luau native code generation (feature `jit`; a WebAssembly build has none: an interpreting VM),
    // unless the VM interprets everything (modules::interpret_all)
    #[cfg(feature = "jit")]
    lua.enable_jit(!modules::interprets_all(engine));
    // Instances are raw tagged userdata with C metamethods (lua/raw.rs)
    super::raw::install(lua, engine)?;
    // Connection handlers live in one Luau table (signal.rs)
    engine.state.borrow_mut().signals.hold_in(lua.create_table()?);
    // What the VM holds for scripts has a ceiling: past it a script errors, as for an exhausted resource (refs.rs)
    engine.lua.borrow_mut().gauge = refs::Gauge::find(lua);
    refs::quiet_panics();
    load_prelude(lua, engine)?;
    types::register(lua, engine)?;
    instance::register(lua, engine)?;
    services::install(engine);
    services::install_capability_calls(lua, engine)?;
    crate::derived::install(engine);
    // The 3D world's views, queries and reported calls (engine::world; after the derived views)
    crate::world::install(engine);
    services::install_gui_behaviors(engine);
    // The host's layers (roblox-ui-renderer: text measurement, layout, input), in their order
    for layer in engine.layers() {
        layer.install(engine);
        engine.clear_member_cache();
    }

    let reflection = engine.reflection.clone();
    let options = engine.options.clone();
    let class = |name: &str| reflection.class_id(name).unwrap_or_else(|| panic!("class {name}"));

    let game = core::create_named(&mut engine.state.borrow_mut(), &reflection, class("DataModel"), &options.place_name);
    engine.state.borrow_mut().collection.game = Some(game);
    // A provisional world so services can be reached while the rest is built
    engine.set_world(World { game, workspace: game, camera: game, local_player: None });
    let workspace = services::get_service(engine, game, "Workspace").map_err(flow_error)?;
    let camera = {
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        let camera = core::create_named(&mut state, &reflection, class("Camera"), "Camera");
        core::set_parent(&mut state, &reflection, &hooks, camera, Some(workspace)).map_err(|e| LuaError::runtime(e.message))?;
        core::raw_set_silent(&mut state, &reflection, workspace, K::CurrentCamera as NameId, Value::Instance(camera));
        camera
    };
    engine.set_world(World { game, workspace, camera, local_player: None });
    // The place file's streaming (scripts cannot set it): Workspace.StreamingEnabled and its radius
    if let Some(radius) = options.streaming_target_radius {
        let mut state = engine.state.borrow_mut();
        for (name, value) in [("StreamingEnabled", Value::Bool(true)), ("StreamingTargetRadius", Value::Number(radius))] {
            if let Some(name) = reflection.name_id(name) {
                core::raw_set_silent(&mut state, &reflection, workspace, name, value);
            }
        }
    }

    let players = services::get_service(engine, game, "Players").map_err(flow_error)?;
    let mut local_player = None;
    if let Some(player_name) = &options.local_player_name {
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        let player = core::create_named(&mut state, &reflection, class("Player"), player_name);
        let display = options.local_player_display_name.clone().unwrap_or_else(|| player_name.clone());
        core::raw_set_silent(&mut state, &reflection, player, K::DisplayName as NameId, Value::String(Str::from(display)));
        core::raw_set_silent(&mut state, &reflection, player, K::UserId as NameId, Value::Number(options.local_player_user_id));
        // Measured: in edit mode the local player has PlayerScripts and Backpack but no PlayerGui
        let mut containers = vec!["PlayerScripts", "Backpack"];
        if !options.edit_mode() {
            containers.insert(0, "PlayerGui");
        }
        for name in containers {
            let child = core::create_named(&mut state, &reflection, class(name), name);
            core::set_parent(&mut state, &reflection, &hooks, child, Some(player)).map_err(|e| LuaError::runtime(e.message))?;
        }
        core::set_parent(&mut state, &reflection, &hooks, player, Some(players)).map_err(|e| LuaError::runtime(e.message))?;
        local_player = Some(player);
    }
    engine.set_world(World { game, workspace, camera, local_player });

    // The host's record of the mouse, which Player:GetMouse()'s PlayerMouse reads (world/mouse.rs)
    let mouse = crate::world::mouse::record(lua, engine)?;
    for name in START_SERVICES {
        services::get_service(engine, game, name).map_err(flow_error)?;
    }
    instance::lock(lua, engine, game)?;

    let shared = build_shared(lua, engine)?;
    shared.raw_set(crate::world::mouse::RECORD, mouse)?;
    engine.lua.borrow_mut().shared = Some(shared.clone());
    super::stubs::load(lua, engine)?;
    if engine.options.native {
        crate::cost::set_safe_global_base(lua, &shared)?;
        #[cfg(feature = "jit")]
        if unsafe { mlua::ffi::luau_codegen_supported() } != 0 {
            unsafe extern "C" {
                fn luau_codegen_set_tiered(state: *mut mlua::ffi::lua_State, threshold: u32);
            }
            unsafe {
                lua.exec_raw::<()>((), |state| {
                    luau_codegen_set_tiered(state, 16);
                })?;
            }
            lua.enable_jit(false);
            modules::use_tiered_jit(engine);
        }
        unsafe {
            lua.exec_raw::<()>((), |state| {
                mlua::ffi::lua_gc(state, mlua::ffi::LUA_GCSETGOAL, 400);
            })?;
        }
    }

    // Scheduler.Configure: math.random starts from the run's seed
    randomseed(engine)?;
    Ok(())
}

fn flow_error(flow: Flow) -> LuaError {
    match flow {
        Flow::Error(error) => LuaError::runtime(error.message),
        Flow::Fail(error) => error,
        Flow::Yield => LuaError::runtime("unexpected yield"),
    }
}

/// math.randomseed(Scheduler.RandomSeed)
pub fn randomseed(engine: &Engine) -> LuaResult<()> {
    let seed = engine.state.borrow().sched.random_seed;
    let function = engine.lua.borrow().randomseed.clone();
    if let Some(function) = function {
        function.call::<()>(seed)?;
    }
    Ok(())
}

/// Engine.JoinLocalPlayer: a joining player takes these from StarterPlayer (after the project,
/// which may set them, is mounted), and its team.
pub fn join_local_player(engine: &Rc<Engine>) {
    let Some(player) = engine.world().local_player else { return };
    let game = engine.world().game;
    let Ok(starter) = services::get_service(engine, game, "StarterPlayer") else { return };
    let reflection = &engine.reflection;
    let pairs = [
        ("CameraMaxZoomDistance", "CameraMaxZoomDistance"),
        ("CameraMinZoomDistance", "CameraMinZoomDistance"),
        ("CameraMode", "CameraMode"),
        ("DevCameraOcclusionMode", "DevCameraOcclusionMode"),
        ("DevComputerCameraMode", "DevComputerCameraMovementMode"),
        ("DevComputerMovementMode", "DevComputerMovementMode"),
        ("DevTouchCameraMode", "DevTouchCameraMovementMode"),
        ("DevTouchMovementMode", "DevTouchMovementMode"),
        ("DevEnableMouseLock", "EnableMouseLockOption"),
        ("HealthDisplayDistance", "HealthDisplayDistance"),
        ("NameDisplayDistance", "NameDisplayDistance"),
        ("AutoJumpEnabled", "AutoJumpEnabled"),
    ];
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    for (player_name, starter_name) in pairs {
        let (Some(player_prop), Some(starter_prop)) = (reflection.name_id(player_name), reflection.name_id(starter_name)) else {
            continue;
        };
        let value = state.dom.raw_get(starter, starter_prop).cloned().unwrap_or(Value::Nil);
        core::raw_set(&mut state, reflection, &hooks, player, player_prop, value);
    }
    drop(state);
    drop(hooks);
    // and its team as a client finds it at its first line (services/teams.rs): in a client-only run the one its server gives
    // the first player, in a loopback client the one the server's TeamColor and Neutral give (Studio's edit DataModel has
    // no server to give one)
    if !engine.options.edit_mode() {
        services::teams::settle_local_player(engine, player, engine.loopback().is_some());
    }
}

fn build_shared(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<LuaTable> {
    let prelude = engine.prelude();
    let exports = &prelude.exports;
    let globals = lua.globals();
    let shared = lua.create_table()?;
    for name in [
        "assert", "error", "getmetatable", "ipairs", "next", "pairs", "pcall", "rawequal", "rawget", "rawlen", "rawset",
        "select", "setmetatable", "tonumber", "tostring", "type", "xpcall", "gcinfo", "newproxy", "getfenv", "setfenv",
        "string", "table", "math", "bit32", "buffer", "vector",
    ] {
        shared.raw_set(name, globals.get::<LuaValue>(name)?)?;
    }
    let table: LuaTable = globals.get("table")?;
    shared.raw_set("unpack", table.get::<LuaValue>("unpack")?)?;
    shared.raw_set("ypcall", globals.get::<LuaValue>("pcall")?)?;
    let gcinfo: LuaFunction = globals.get("gcinfo")?;
    // Roblox accepts only collectgarbage("count") (Studio 0.747, tools/parity/probes/errors.luau), and its errors name the line
    // that called it (Studio 0.747, engine_functions.luau)
    shared.raw_set(
        "collectgarbage",
        library_function(lua, "collectgarbage", move |_, args| {
            let args = Args::new(args);
            match args.get(0) {
                LuaValue::Nil => super::raise_at_caller("missing argument #1 to 'collectgarbage' (string expected)"),
                LuaValue::String(option) if &*option.as_bytes() == b"count" => Ok(values([gcinfo.call::<LuaValue>(())?])),
                _ => super::raise_at_caller("collectgarbage must be called with 'count'; use gcinfo() instead"),
            }
        })?,
    )?;
    // coroutine.create and wrap: the VM's, after charging the script for the thread as task.spawn does (lua/builtins.rs)
    let coroutine: LuaTable = exports.get("Coroutine")?;
    let native_coroutine: LuaTable = globals.get("coroutine")?;
    let e = engine.clone();
    let charge = lua.create_function(move |_, seconds: f64| {
        let state = e.state.borrow();
        state.sched.charge(seconds);
        if !state.sched.is_muted() {
            crate::boundary::add_cost(seconds);
        }
        Ok(())
    })?;
    for name in ["create", "wrap"] {
        let native: LuaFunction = native_coroutine.get(name)?;
        coroutine.raw_set(name, super::builtins::coroutine_function(lua, name, native, charge.clone(), crate::cost::THREAD)?)?;
    }
    shared.raw_set("coroutine", coroutine)?;
    shared.raw_set("utf8", exports.get::<LuaValue>("Utf8")?)?;
    shared.raw_set("debug", exports.get::<LuaValue>("Debug")?)?;

    // Virtual clocks (scheduler.luau): os.clock() and elapsedTime() stand still within a frame
    // phase; time() and DistributedGameTime are 0 in edit mode; tick() is Unix time plus the
    // standard zone offset; the wall clock starts at the current whole hour.
    let os: LuaTable = exports.get("Os")?;
    let native_os: LuaTable = globals.get("os")?;
    let e = engine.clone();
    os.raw_set(
        "clock",
        named(lua, "clock", Library, lua.create_function(move |lua, ()| {
            let clock = e.state.borrow_mut().sched.clock();
            // (a profile counts the reads by line and the clock's advance between them: a loop that checks a budget reads it once an
            // iteration)
            if crate::profiler::attribution::on() {
                crate::profiler::attribution::clock_read(lua, clock);
            }
            Ok(clock)
        })?)?,
    )?;
    if std::env::var("RUR_REAL_CLOCK").as_deref() == Ok("1") {
        let start = std::time::Instant::now();
        os.raw_set("realclock", lua.create_function(move |_, ()| Ok(start.elapsed().as_secs_f64()))?)?;
    }
    if std::env::var("REE_WORK_COUNTERS").as_deref() == Ok("1") {
        // calibration (cost.rs, tools/cost-parity): the VM's work counters, { ops = {[opcode + 1] = n}, nativeops, allocated, ccalls, ... }
        let e = engine.clone();
        os.raw_set(
            "workcounters",
            lua.create_function(move |lua, ()| {
                let Some(pointer) = e.state.borrow().sched.vm_counters else { return Ok(LuaValue::Nil) };
                let counters = unsafe { &*pointer };
                let ops = lua.create_table()?;
                for (op, count) in counters.ops.iter().enumerate() {
                    if *count > 0 {
                        ops.raw_set(op + 1, *count as f64)?;
                    }
                }
                let nativeops = lua.create_table()?;
                for (op, count) in counters.nativeops.iter().enumerate() {
                    if *count > 0 {
                        nativeops.raw_set(op + 1, *count as f64)?;
                    }
                }
                let result = lua.create_table()?;
                result.raw_set("ops", ops)?;
                result.raw_set("nativeops", nativeops)?;
                result.raw_set("allocated", counters.allocated as f64)?;
                result.raw_set("ccalls", counters.ccalls as f64)?;
                result.raw_set("tmcalls", counters.tmcalls as f64)?;
                result.raw_set("hostcalls", counters.hostcalls as f64)?;
                result.raw_set("hostnanos", counters.hostnanos as f64)?;
                result.raw_set("objects", counters.objects as f64)?;
                result.raw_set("stored", counters.stored as f64)?;
                result.raw_set("rehashed", counters.rehashed as f64)?;
                result.raw_set("hostops", crate::cost::host_instructions(counters) as f64)?;
                for (key, value) in [
                    ("vectormagnitudes", counters.vectormagnitudes),
                    ("vectorunits", counters.vectorunits),
                    ("vectormethods", counters.vectormethods),
                    ("numtostr", counters.numtostr),
                    ("sortcompares", counters.sortcompares),
                    ("sortcalls", counters.sortcalls),
                    ("createitems", counters.createitems),
                    ("cloneitems", counters.cloneitems),
                    ("concatitems", counters.concatitems),
                    ("splitparts", counters.splitparts),
                    ("formats", counters.formats),
                    ("formatspecs", counters.formatspecs),
                    ("newstrings", counters.newstrings),
                    ("slowimports", counters.slowimports),
                    ("coldtables", counters.coldtables),
                    ("coldclosures", counters.coldclosures),
                    ("coldprotos", counters.coldprotos),
                    ("coldcode", counters.coldcode),
                    ("scanned", counters.hostops[crate::cost::SCANNED_SLOT]),
                    ("moved", counters.hostops[crate::cost::MOVED_SLOT]),
                    ("strbytes", counters.hostops[crate::cost::STRBYTES_SLOT]),
                    ("strscanned", counters.hostops[crate::cost::STRSCANNED_SLOT]),
                    ("matchsteps", counters.hostops[crate::cost::MATCHSTEPS_SLOT]),
                    ("bufbytes", counters.hostops[crate::cost::BUFBYTES_SLOT]),
                    ("allocbytes", counters.hostops[crate::cost::ALLOCBYTES_SLOT]),
                    ("libobjects", counters.hostops[crate::cost::LIBOBJECTS_SLOT]),
                    ("casebytes", counters.hostops[crate::cost::CASEBYTES_SLOT]),
                    ("utf8bytes", counters.hostops[crate::cost::UTF8BYTES_SLOT]),
                    ("utf8chars", counters.hostops[crate::cost::UTF8CHARS_SLOT]),
                    ("strchars", counters.hostops[crate::cost::STRCHARS_SLOT]),
                    ("bytevalues", counters.hostops[crate::cost::BYTEVALUES_SLOT]),
                    ("buffers", counters.hostops[crate::cost::BUFFERS_SLOT]),
                    ("matchclasses", counters.hostops[crate::cost::MATCHCLASSES_SLOT]),
                    ("hashrehashed", counters.hostops[crate::cost::HASHREHASHED_SLOT]),
                    ("largeblocks", counters.hostops[crate::cost::LARGEBLOCKS_SLOT]),
                    ("buffreshbytes", counters.hostops[crate::cost::BUFFRESH_SLOT]),
                    ("bufbigcopybytes", counters.hostops[crate::cost::BUFBIGCOPY_SLOT]),
                    ("coldcfuncs", counters.hostops[crate::cost::COLD_CFUNCS_SLOT]),
                    ("coldhostfuncs", counters.hostops[crate::cost::COLD_HOSTFUNCS_SLOT]),
                    ("rareprotos", counters.hostops[crate::cost::RARE_PROTOS_SLOT]),
                ] {
                    result.raw_set(key, value as f64)?;
                }
                // (the engine's own counts of what a script asked it for by size: JSON text and values)
                let json = crate::services::json::counts();
                result.raw_set("jsonbytes", json.bytes as f64)?;
                result.raw_set("jsonvalues", json.values as f64)?;
                Ok(LuaValue::Table(result))
            })?,
        )?;
    }
    // Luau's own os.time and os.date on the virtual clock; their errors are Luau's plain messages (`invalid argument #1
    // to 'time' (table expected, got number)`; Studio 0.744, tools/parity/probes/datetime.luau)
    let native_time: LuaFunction = native_os.get("time")?;
    let e = engine.clone();
    os.raw_set(
        "time",
        library_function(lua, "time", move |_, args| {
            let args = Args::new(args);
            if !args.get(0).is_nil() {
                // (its error at the line that called it, as Luau's os.time places it: Studio 0.747, engine_functions.luau)
                return match native_time.call::<LuaValue>(args.get(0).clone()) {
                    Ok(time) => Ok(values([time])),
                    Err(error) => super::raise_at_caller(scheduler::error_message(&error)),
                };
            }
            let millis = e.state.borrow_mut().sched.unix_millis();
            Ok(values([LuaValue::Number((millis / 1000.0).floor())]))
        })?,
    )?;
    let native_date: LuaFunction = native_os.get("date")?;
    let e = engine.clone();
    os.raw_set(
        "date",
        library_function(lua, "date", move |_, args| {
            let args = Args::new(args);
            let time = match args.get(1) {
                LuaValue::Nil => LuaValue::Number((e.state.borrow_mut().sched.unix_millis() / 1000.0).floor()),
                time => time.clone(),
            };
            match native_date.call::<LuaValue>((args.get(0).clone(), time)) {
                Ok(date) => Ok(values([date])),
                Err(error) => raise_plain(scheduler::error_message(&error)),
            }
        })?,
    )?;
    shared.raw_set("os", os)?;
    // (a string in a playing client's and a server's scripts, measured: tools/parity/probes/errors.luau; "Luau" is Luau's own
    // base library value, the text is not measured)
    shared.raw_set("_VERSION", "Luau")?;

    let game = engine.world().game;
    let workspace = engine.world().workspace;
    shared.raw_set("game", instance_value(lua, engine, game)?)?;
    shared.raw_set("Game", instance_value(lua, engine, game)?)?;
    shared.raw_set("workspace", instance_value(lua, engine, workspace)?)?;
    shared.raw_set("Workspace", instance_value(lua, engine, workspace)?)?;
    shared.raw_set("Enum", engine.lua.borrow().enums_global.clone().expect("Enum global"))?;

    let instance_global = lua.create_table()?;
    // Instance.new: a C function (lua/raw.rs; InstanceModule.New)
    instance_global.raw_set("new", super::raw::instance_new_function(lua, engine)?)?;
    let e = engine.clone();
    instance_global.raw_set(
        "fromExisting",
        library_function(lua, "fromExisting", move |lua, args| {
            let args = Args::new(args);
            let Some(existing) = args.instance(0) else { return raise_plain("Instance expected") };
            let Some(copy) = instance::clone(&e, existing) else { return Ok(values([LuaValue::Nil])) };
            let children = e.state.borrow().dom.at(copy).children.to_vec();
            for child in children {
                instance::destroy(lua, &e, child)?;
            }
            Ok(values([instance_value(lua, &e, copy)?]))
        })?,
    )?;
    shared.raw_set("Instance", instance_global)?;
    shared.raw_set("typeof", prelude.typeof_fn.clone())?;
    // print and warn: C functions that write their arguments as tostring does, from their own frame (lua/builtins.rs)
    for kind in ["print", "warn"] {
        let e = engine.clone();
        let output = lua.create_function(move |_, (kind, message): (LuaString, LuaString)| {
            let kind = if &*kind.as_bytes() == b"warn" { "warn" } else { "print" };
            e.output(kind, message.to_string_lossy(), None);
            Ok(())
        })?;
        let mark = lua.create_function(|_, value: LuaValue| {
            super::shared_table::mark_value(&value);
            Ok(())
        })?;
        shared.raw_set(kind, super::builtins::output_function(lua, kind, output, mark)?)?;
    }

    // (the C functions below are named as a client names them, each alias a function of its own: `Wait` is named Wait; measured,
    // Studio 0.747, engine_functions.luau)
    let e = engine.clone();
    shared.raw_set("tick", named(lua, "tick", Library, lua.create_function(move |_, ()| Ok(e.state.borrow_mut().sched.tick()))?)?)?;
    let e = engine.clone();
    let edit = engine.options.edit_mode();
    shared.raw_set(
        "time",
        named(lua, "time", Library, lua.create_function(move |_, ()| Ok(if edit { 0.0 } else { e.state.borrow().sched.now }))?)?,
    )?;
    let e = engine.clone();
    let elapsed = lua.create_function(move |_, ()| Ok(e.state.borrow_mut().sched.elapsed_time()))?;
    shared.raw_set("elapsedTime", named(lua, "elapsedTime", Library, elapsed.clone())?)?;
    shared.raw_set("ElapsedTime", named(lua, "ElapsedTime", Library, elapsed)?)?;
    // (wait: the thread registered, then a yield from the C function, lua/cfunction.rs)
    let legacy_wait: LuaFunction = exports.get("LegacyWait")?;
    shared.raw_set("wait", named(lua, "wait", Library, legacy_wait.clone())?)?;
    shared.raw_set("Wait", named(lua, "Wait", Library, legacy_wait)?)?;
    // (each alias is a C function of its own name, as a client's: `Spawn` is named Spawn; measured, engine_functions.luau)
    for name in ["spawn", "Spawn"] {
        let e = engine.clone();
        let legacy_spawn = library_function(lua, name, move |lua, args| {
            let args = Args::new(args);
            let thread = legacy_thread(lua, args.get(0))?;
            refs::admit(&e, Hold::Work, "spawn")?;
            e.state.borrow_mut().sched.schedule_legacy(TimerKind::Delay, Some(0.0), thread);
            Ok(none())
        })?;
        shared.raw_set(name, legacy_spawn)?;
    }
    for name in ["delay", "Delay"] {
        let e = engine.clone();
        let legacy_delay = library_function(lua, name, move |lua, args| {
            let args = Args::new(args);
            let duration = args.number(0);
            let thread = legacy_thread(lua, args.get(1))?;
            refs::admit(&e, Hold::Work, "delay")?;
            e.state.borrow_mut().sched.schedule_legacy(TimerKind::Delay, duration, thread);
            Ok(none())
        })?;
        shared.raw_set(name, legacy_delay)?;
    }
    shared.raw_set("task", exports.get::<LuaValue>("Task")?)?;

    let make_stub: LuaFunction = exports.get("MakeStubTree")?;
    // settings(): refused to a game script, which a play run's scripts are ("The current thread cannot call 'settings' (lacking
    // capability Plugin)", with no position: a playing client and a server, Studio 0.747, tools/parity/probes/global_settings.luau);
    // in edit mode, where a plugin's code runs, the GlobalSettings object with its settings services (services/settings.rs),
    // made on the first call as UserSettings() is, else an inert stub without the class
    let stub_global: LuaValue = make_stub.call("settings")?;
    let global_settings: std::cell::RefCell<Option<LuaValue>> = std::cell::RefCell::new(None);
    let e = engine.clone();
    let edit = engine.options.edit_mode();
    let settings = library_function(lua, "settings", move |lua, _| {
        if !edit {
            return raise_plain("The current thread cannot call 'settings' (lacking capability Plugin)");
        }
        if let Some(existing) = global_settings.borrow().as_ref() {
            return Ok(values([existing.clone()]));
        }
        let made = match services::settings::global_settings(&e) {
            Some(id) => instance_value(lua, &e, id)?,
            None => stub_global.clone(),
        };
        *global_settings.borrow_mut() = Some(made.clone());
        Ok(values([made]))
    })?;
    shared.raw_set("settings", settings)?;
    // UserSettings(): the UserSettings object (its GetService gives UserGameSettings & co. at their
    // defaults: SavedQualityLevel Automatic, ...), made on the first call (an engine that made it at
    // start would number every later instance differently), else an inert stub without the class
    let stub_settings: LuaValue = make_stub.call("UserSettings")?;
    let user_settings: std::cell::RefCell<Option<LuaValue>> = std::cell::RefCell::new(None);
    let e = engine.clone();
    shared.raw_set(
        "UserSettings",
        named(lua, "UserSettings", Library, lua.create_function(move |lua, ()| {
            if let Some(existing) = user_settings.borrow().as_ref() {
                return Ok(existing.clone());
            }
            let made = match services::user_settings(&e) {
                Some(id) => instance_value(lua, &e, id)?,
                None => stub_settings.clone(),
            };
            *user_settings.borrow_mut() = Some(made.clone());
            Ok(made)
        })?)?,
    )?;
    let version = engine.reflection.version.clone();
    let version = if version.is_empty() { "0.0.0.0".to_string() } else { version };
    let version_fn = lua.create_function(move |_, ()| Ok(version.clone()))?;
    shared.raw_set("version", named(lua, "version", Library, version_fn.clone())?)?;
    shared.raw_set("Version", named(lua, "Version", Library, version_fn)?)?;
    let e = engine.clone();
    let stats = lua.create_function(move |lua, ()| {
        let game = e.world().game;
        let stats = services::get_service(&e, game, "Stats").map_err(flow_error)?;
        instance_value(lua, &e, stats)
    })?;
    shared.raw_set("stats", named(lua, "stats", Library, stats.clone())?)?;
    shared.raw_set("Stats", named(lua, "Stats", Library, stats)?)?;
    shared.raw_set("shared", lua.create_table()?)?;
    shared.raw_set("PluginManager", named(lua, "PluginManager", Library, lua.create_function(|_, ()| Ok(LuaValue::Nil))?)?)?;
    let print: LuaFunction = exports.get("Print")?;
    shared.raw_set(
        "printidentity",
        named(lua, "printidentity", Library, lua.create_function(move |_, prefix: Option<String>| {
            print.call::<()>(format!("{} 2", prefix.unwrap_or_else(|| "Current identity is".into())))
        })?)?,
    )?;
    let e = engine.clone();
    shared.raw_set("loadstring", library_function(lua, "loadstring", move |lua, args| super::loading::loadstring(lua, &e, args))?)?;
    let e = engine.clone();
    let debug: LuaTable = exports.get("Debug")?;
    debug.raw_set("loadmodule", library_function(lua, "loadmodule", move |lua, args| super::loading::loadmodule(lua, &e, args))?)?;
    let inert: LuaFunction = exports.get("InertLibrary")?;
    for name in ["PluginDrag", "CellId"] {
        shared.raw_set(name, inert.call::<LuaValue>(name)?)?;
    }

    // Datatypes: engine types, prelude proxies, then Lune-backed script libraries
    for name in ["Content", "DateTime", "FloatCurveKey", "Font", "InstanceHandle", "Random", "RotationCurveKey", "SharedTable", "TweenInfo"] {
        let library = engine.lua.borrow().libraries.get(name).cloned().expect("library");
        shared.raw_set(name, library)?;
    }
    for name in [
        "OverlapParams", "RaycastParams", "CatalogSearchParams", "PathWaypoint", "Path2DControlPoint", "DockWidgetPluginGuiInfo", "SecurityCapabilities", "ValueCurveKey",
    ] {
        shared.raw_set(name, exports.get::<LuaValue>(name)?)?;
    }
    let script_library: LuaFunction = exports.get("ScriptLibrary")?;
    for name in [
        "Axes", "BrickColor", "CFrame", "Color3", "ColorSequence", "ColorSequenceKeypoint", "Faces", "NumberRange",
        "NumberSequence", "NumberSequenceKeypoint", "PhysicalProperties", "Ray", "Rect", "Region3", "Region3int16",
        "UDim", "UDim2", "Vector2", "Vector2int16", "Vector3", "Vector3int16",
    ] {
        shared.raw_set(name, script_library.call::<LuaValue>(name)?)?;
    }
    shared.raw_set("_G", lua.create_table()?)?;
    Ok(shared)
}

/// Engine.Call: runs `function` on a fresh scheduler thread (so it may yield) under xpcall and
/// flushes the deferred queue.
pub fn call(lua: &Lua, engine: &Rc<Engine>, function: LuaFunction, args: LuaMultiValue) -> LuaResult<Call> {
    call_then(lua, engine, function, args, || true)
}

/// [`call`], flushing the deferred queue after the first resumption only when `flush` says so (a
/// recording parked at its harness's checkpoint flushes when it goes on: record/run.rs).
pub fn call_then(lua: &Lua, engine: &Rc<Engine>, function: LuaFunction, args: LuaMultiValue, flush: impl FnOnce() -> bool) -> LuaResult<Call> {
    let body: LuaFunction = engine.prelude().exports.get("CallBody")?;
    let outcome = lua.create_table()?;
    let thread = lua.create_thread(body)?;
    let mut arguments = LuaMultiValue::from_iter([LuaValue::Table(outcome.clone()), LuaValue::Function(function)]);
    arguments.extend(args);
    scheduler::resume(lua, engine, &thread, arguments);
    if flush() {
        scheduler::flush_deferred(lua, engine)?;
    }
    Ok(Call { thread, outcome })
}

/// A pending or finished Engine.Call.
pub struct Call {
    pub thread: LuaThread,
    outcome: LuaTable,
}

pub enum CallOutcome {
    Finished(LuaMultiValue),
    Pending,
    Failed(String, Option<String>),
}

impl Call {
    /// How the call ended so far (Outcome()).
    pub fn outcome(&self) -> LuaResult<CallOutcome> {
        if let Some(results) = self.outcome.get::<Option<LuaTable>>("Results")? {
            let count: i64 = results.get("n")?;
            let mut values = LuaMultiValue::new();
            for index in 2..=count {
                values.push_back(results.get(index)?);
            }
            return Ok(CallOutcome::Finished(values));
        }
        if let Some(failure) = self.outcome.get::<Option<LuaTable>>("Failure")? {
            let message: String = self.outcome.get("Message")?;
            let trace: Option<String> = failure.get("Trace")?;
            return Ok(CallOutcome::Failed(message, trace));
        }
        Ok(CallOutcome::Pending)
    }

    /// (Tests) Runs the call to its end: while it waits (task.wait, an event's Wait) the engine steps frames, one at a time, until
    /// it finishes. Its error (with the trace) when it failed or was still waiting after [`FINISH_FRAMES`]: a test helper that
    /// returned at a script's first yield never ran what follows a wait, its asserts passing unchecked.
    #[cfg(test)]
    pub(crate) fn finish(&self, lua: &Lua, engine: &Rc<Engine>) -> Result<(), String> {
        let mut frames = 0;
        loop {
            match self.outcome().map_err(|error| error.to_string())? {
                CallOutcome::Finished(_) => return Ok(()),
                CallOutcome::Failed(message, trace) => return Err(format!("{message}\n{}", trace.unwrap_or_default())),
                CallOutcome::Pending if frames < FINISH_FRAMES => {
                    scheduler::step(lua, engine, None).map_err(|error| error.to_string())?;
                    frames += 1;
                }
                CallOutcome::Pending => return Err(format!("still waiting after {frames} frames")),
            }
        }
    }
}

/// (Tests) The frames [`Call::finish`] steps a waiting script through before it fails: a minute of 60 frames a second
#[cfg(test)]
pub(crate) const FINISH_FRAMES: u32 = 3600;

impl LuaRefs {
    /// The shared environment table (after `start`).
    pub fn shared_globals(&self) -> Option<LuaTable> {
        self.shared.clone()
    }
}

/// Unused marker to keep the lock helper referenced for types built lazily.
#[allow(dead_code)]
fn _lock(lua: &Lua, ud: &LuaAnyUserData) -> LuaResult<()> {
    lock_metatable(lua, ud)
}

pub fn studio_only(engine: &Engine, id: InstanceId) -> bool {
    engine.state.borrow().dom.at(id).has(flags::STUDIO_ONLY)
}
