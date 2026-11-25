//! The Luau-facing engine API: Instance userdata, engine datatypes, globals, per-script
//! environments and require (ports of `runtime/engine/globals.luau`, `modules.luau`, the
//! metatable half of `instance.luau` and `init.luau`'s Engine.Start).
//!
//! Engine functions answer a call protocol: a Rust function returns its values, or a marker that makes the C function a script
//! called (lua/cfunction.rs, one named C function per engine function, as a client's are) raise a string error at the right
//! level or yield the calling thread. Scripts therefore see Roblox's string errors (never mlua's userdata errors), and yields
//! never cross a Rust frame.

pub mod actors;
pub mod bindable;
pub mod builtins;
pub mod cfunction;
pub mod convert;
pub mod curve_keys;
pub mod float_mode;
pub mod globals;
pub mod holders;
pub mod instance;
pub mod leaks;
pub mod loading;
pub mod modules;
pub mod normalize;
pub mod raw;
pub mod refs;
pub mod shared_table;
pub mod stubs;
pub mod types;
pub mod vector_methods;

#[cfg(test)]
mod datatype_tests;

use std::rc::Rc;

use mlua::prelude::*;
use rustc_hash::FxHashMap;

use super::{
    ClassId, Engine, InstanceId, NameId, ScriptError,
    reflection::{EnumId, EnumItemId},
    signal::SignalId,
};

static SIGNAL_MARKER: u8 = 0;

/// The protocol marker (a light userdata no script can forge).
pub fn signal_marker() -> LuaValue {
    LuaValue::LightUserData(LuaLightUserData(signal_marker_ptr()))
}

#[inline]
pub fn signal_marker_ptr() -> *mut std::ffi::c_void {
    &SIGNAL_MARKER as *const u8 as *mut std::ffi::c_void
}

/// How an engine function ended besides returning values.
#[derive(Debug)]
pub enum Flow {
    /// Raise a string error in the calling script
    Error(ScriptError),
    /// Yield the calling coroutine (its resumption is already registered)
    Yield,
    /// An internal failure (surfaces as an mlua error)
    Fail(LuaError),
    /// Raise a value a script's own code raised while the engine call ran (a dictionary key's `__tostring` during a copy: lua/
    /// bindable.rs), as it was raised: a table as that table, a string with the position it already has
    Raised(LuaValue),
}

impl Flow {
    /// A raised value's text, where an error can only be text (an mlua error, a warning): a string's, else tostring's form
    pub fn raised_text(value: &LuaValue) -> String {
        match value {
            LuaValue::String(text) => text.to_string_lossy(),
            other => other.to_string().unwrap_or_default(),
        }
    }
}

impl From<LuaError> for Flow {
    fn from(error: LuaError) -> Self {
        Flow::Fail(error)
    }
}

impl From<ScriptError> for Flow {
    fn from(error: ScriptError) -> Self {
        Flow::Error(error)
    }
}

pub type Ret = Result<LuaMultiValue, Flow>;

/// Encodes an engine function's outcome for the prelude's `Pass`.
pub fn encode(lua: &Lua, outcome: Ret) -> LuaResult<LuaMultiValue> {
    match outcome {
        Ok(values) => Ok(values),
        Err(Flow::Error(error)) => Ok(LuaMultiValue::from_iter([
            signal_marker(),
            LuaValue::Integer(1),
            LuaValue::String(lua.create_string(&error.message)?),
            LuaValue::Integer(error.level as _),
        ])),
        Err(Flow::Yield) => Ok(LuaMultiValue::from_iter([signal_marker(), LuaValue::Integer(2)])),
        Err(Flow::Fail(error)) => Err(error),
        // (level 0: raised as it is, no position added)
        Err(Flow::Raised(value)) => Ok(LuaMultiValue::from_iter([signal_marker(), LuaValue::Integer(1), value, LuaValue::Integer(0)])),
    }
}

/// An error of the engine's own members (properties, methods, events, services): Roblox raises them without the
/// script's position (`NotAMember is not a valid member of Part "Part"`, `Unable to assign property ClassName.
/// Property is read only`, `'X' is not a valid Service name`; measured in a playing client, Studio 0.744:
/// tools/parity/probes/errors.luau).
pub fn raise<T>(message: impl Into<String>) -> Result<T, Flow> {
    Err(Flow::Error(ScriptError::plain(message)))
}

pub fn raise_plain<T>(message: impl Into<String>) -> Result<T, Flow> {
    Err(Flow::Error(ScriptError::plain(message)))
}

/// An error Luau's own checks would raise, at the calling script line: `invalid argument #2 (string expected, got
/// number)`, `attempt to iterate over a Instance value` (Roblox's are positioned like the VM's).
pub fn raise_at_caller<T>(message: impl Into<String>) -> Result<T, Flow> {
    Err(Flow::Error(ScriptError::at_caller(message)))
}

/// Whether `thread`, the running one, may yield where it is: not inside a metamethod, a `table.sort` comparator, a
/// `string.gsub` callback or another function called from C (Luau's `lua_isyieldable`; a method call through a C
/// `__namecall` with a continuation keeps its caller's answer).
pub fn yieldable(thread: &LuaThread) -> bool {
    unsafe { mlua::ffi::lua_isyieldable(thread.state()) != 0 }
}

/// What a yielding engine call raises, without a position, where the running thread cannot yield: it is checked before
/// the call registers anything with the scheduler (a refused yield leaves no resumption behind, so the thread's later
/// waits take their own time and return their own results). `thread is not yieldable` for task.wait, wait, WaitForChild of
/// a missing child, require, a BindableFunction Invoke whose handler yields and the remote invokes; a signal's Wait raises
/// [`NOT_YIELDABLE_WAIT`]. coroutine.yield keeps the VM's own error. Measured in a 0.747 playing client and server, also
/// that task.wait checks it before its argument: tools/parity/probes/yield_refused.luau.
pub const NOT_YIELDABLE: &str = "thread is not yieldable";
/// RBXScriptSignal:Wait's [`NOT_YIELDABLE`]
pub const NOT_YIELDABLE_WAIT: &str = "thread cannot be yielded to Wait";

/// [`NOT_YIELDABLE`] (or `message`) unless `thread` may yield.
pub fn refuse_unyieldable(thread: &LuaThread, message: &str) -> Result<(), Flow> {
    if yieldable(thread) { Ok(()) } else { raise_plain(message) }
}

pub fn values<const N: usize>(items: [LuaValue; N]) -> LuaMultiValue {
    LuaMultiValue::from_iter(items)
}

pub fn none() -> LuaMultiValue {
    LuaMultiValue::new()
}

/// Functions the prelude chunk returns.
pub struct Prelude {
    pub wrap: LuaFunction,
    pub typeof_fn: LuaFunction,
    pub register: LuaFunction,
    pub traceback: LuaFunction,
    pub exports: LuaTable,
}

/// Luau-side caches: one userdata per enum item / signal (identity matters: they are table keys
/// and compared with ==), and one function per method implementation. Instance userdata live in
/// the raw API's weak / anchor tables (`raw.rs`).
#[derive(Default)]
pub struct LuaRefs {
    /// The raw Instance API (userdata, dispatch cache, lifetime)
    pub raw: Option<Rc<raw::Api>>,
    pub enum_items: Vec<Option<LuaAnyUserData>>,
    pub enums: Vec<Option<LuaAnyUserData>>,
    pub enums_global: Option<LuaAnyUserData>,
    pub signals: Vec<Option<LuaAnyUserData>>,
    /// Behaviour methods by the (class, name) that defines them
    pub methods: FxHashMap<(ClassId, NameId), LuaFunction>,
    /// Behaviour methods as namecall runs them (protocol results, no Luau wrapper)
    pub call_methods: FxHashMap<(ClassId, NameId), LuaFunction>,
    /// Stand-ins for reflected methods without a behaviour (report + return nil)
    pub unimplemented: FxHashMap<(ClassId, NameId), LuaFunction>,
    /// Behaviour methods in the engine's Luau as scripts call them, one C function each (instance.rs `lua_method`), by the
    /// (class, name) that defines them, with the Luau function each stands for
    pub lua_methods: FxHashMap<(ClassId, NameId), (usize, LuaFunction)>,
    pub prelude: Option<Rc<Prelude>>,
    /// The shared global table every script environment inherits
    pub shared: Option<LuaTable>,
    /// Rust halves of a script's `require` closure (stored for fast Rust-side environment creation)
    pub require_begin: Option<LuaFunction>,
    pub require_finish: Option<LuaFunction>,
    /// Signal / connection member functions
    pub signal_members: Option<LuaTable>,
    pub connection_members: Option<LuaTable>,
    pub enum_members: Option<LuaTable>,
    pub enum_item_is_a: Option<LuaFunction>,
    /// Lune's `coroutine.close` (task.cancel)
    pub coroutine_close: Option<LuaFunction>,
    /// `math.randomseed`
    pub randomseed: Option<LuaFunction>,
    /// Library tables of engine datatypes (Font, Content, ...)
    pub libraries: FxHashMap<&'static str, LuaTable>,
    /// The slots the VM holds for scripts (refs.rs)
    pub gauge: Option<refs::Gauge>,
}

impl Engine {
    pub fn prelude(&self) -> Rc<Prelude> {
        self.lua.borrow().prelude.clone().expect("prelude not loaded")
    }
}

/// A Rust engine function (protocol results) as scripts call it: one C function named `name` (lua/cfunction.rs), for what the
/// engine serves (a datatype's constructors, functions and metamethods, an object's members: [`cfunction::Kind::Served`], priced
/// as the engine Luau that stood for it was).
pub fn wrapped<F>(lua: &Lua, engine: &Rc<Engine>, name: &str, function: F) -> LuaResult<LuaFunction>
where
    F: Fn(&Lua, LuaMultiValue) -> Ret + 'static,
{
    let _ = engine;
    let raw = lua.create_function(move |lua, args: LuaMultiValue| encode(lua, function(lua, args)))?;
    cfunction::over_rust(lua, name, cfunction::Kind::Served, raw)
}

/// [`wrapped`] for a global or a library's function ([`cfunction::Kind::Library`]).
pub fn library_function<F>(lua: &Lua, name: &str, function: F) -> LuaResult<LuaFunction>
where
    F: Fn(&Lua, LuaMultiValue) -> Ret + 'static,
{
    let raw = lua.create_function(move |lua, args: LuaMultiValue| encode(lua, function(lua, args)))?;
    cfunction::over_rust(lua, name, cfunction::Kind::Library, raw)
}

/// `function` (an mlua function, which may answer the call protocol) as one C function named `name` (lua/cfunction.rs).
pub fn named(lua: &Lua, name: &str, kind: cfunction::Kind, function: LuaFunction) -> LuaResult<LuaFunction> {
    cfunction::over_rust(lua, name, kind, function)
}

/// `debug.traceback(thread)` (diagnostics only).
pub fn traceback(lua: &Lua, thread: &LuaThread) -> Option<String> {
    let engine = lua.app_data_ref::<Rc<Engine>>()?.clone();
    let prelude = engine.lua.borrow().prelude.clone()?;
    prelude.traceback.call::<Option<String>>(thread.clone()).ok().flatten()
}

/// Where the running script is: the innermost frame of a script's Luau on `lua`'s running thread (the thread of the callback
/// now running), as `<chunk>:<line>`, the chunk being the script's path in the DataModel as error positions name it
/// (`ReplicatedStorage.Module:3`). C functions and the engine's own Luau (the `rur.*` and `ree.*` chunks: the prelude, the
/// service stubs, the harness wrapper) are skipped; none when no script's frame is on the stack.
pub fn script_position(lua: &Lua) -> Option<String> {
    // SAFETY: the running thread's state, valid while the callback runs
    unsafe { script_position_on(lua.current_thread().state()) }
}

/// [`script_position`] on the thread `state` (a C metamethod's own: lua/raw.rs).
///
/// # Safety
/// `state` must be a live thread of the running VM.
pub unsafe fn script_position_on(state: *mut mlua::ffi::lua_State) -> Option<String> {
    use mlua::ffi;
    unsafe {
        let mut debug: ffi::lua_Debug = std::mem::zeroed();
        let mut level = 0;
        while level < 128 && ffi::lua_getinfo(state, level, c"sl".as_ptr(), &mut debug) != 0 {
            level += 1;
            // (a C function has no line)
            if debug.currentline < 0 || debug.source.is_null() {
                continue;
            }
            let source = std::ffi::CStr::from_ptr(debug.source).to_string_lossy();
            let chunk = source.strip_prefix('=').or_else(|| source.strip_prefix('@')).unwrap_or(&source);
            if chunk == "[C]" || chunk.starts_with("rur.") || chunk.starts_with("ree.") {
                continue;
            }
            return Some(format!("{chunk}:{}", debug.currentline));
        }
    }
    None
}

/// The running thread's stack as `debug.traceback()` writes it in the script that called the engine (`<chunk>:<line>[ function
/// <name>]` a Luau frame, C frames left out), read in Rust so that it calls nothing in the VM: the stack an engine warning about
/// the script's call carries (convert::string_param; lib.rs client_error_text keeps the script's own frames).
pub fn script_traceback(lua: &Lua) -> String {
    // SAFETY: the running thread's state, valid while the callback runs
    unsafe { script_traceback_on(lua.current_thread().state()) }
}

/// [`script_traceback`] on the thread `state` (a C metamethod's own: lua/raw.rs).
///
/// # Safety
/// `state` must be a live thread of the running VM.
pub unsafe fn script_traceback_on(state: *mut mlua::ffi::lua_State) -> String {
    use mlua::ffi;
    let mut trace = String::new();
    unsafe {
        let mut debug: ffi::lua_Debug = std::mem::zeroed();
        let mut level = 0;
        while level < 128 && ffi::lua_getinfo(state, level, c"sln".as_ptr(), &mut debug) != 0 {
            level += 1;
            if debug.what.is_null() || std::ffi::CStr::from_ptr(debug.what).to_bytes() == b"C" {
                continue;
            }
            if !debug.source.is_null() && !debug.short_src.is_null() {
                trace.push_str(&std::ffi::CStr::from_ptr(debug.short_src).to_string_lossy());
            }
            if debug.currentline > 0 {
                trace.push_str(&format!(":{}", debug.currentline));
            }
            if !debug.name.is_null() {
                trace.push_str(" function ");
                trace.push_str(&std::ffi::CStr::from_ptr(debug.name).to_string_lossy());
            }
            trace.push('\n');
        }
    }
    trace
}

/// The innermost script `Instance` on `state`'s call stack (read from each non-engine Luau frame's `fenv.script`),
/// for `ScriptContext.Error`'s third argument.
///
/// # Safety
/// `state` must be a live thread of the running VM.
pub unsafe fn script_of_thread(state: *mut mlua::ffi::lua_State) -> Option<InstanceId> {
    use mlua::ffi;
    unsafe {
        let mut debug: ffi::lua_Debug = std::mem::zeroed();
        let mut level = 0;
        while level < 128 && ffi::lua_getinfo(state, level, c"slf".as_ptr(), &mut debug) != 0 {
            level += 1;
            let is_script = debug.currentline >= 0
                && !debug.source.is_null()
                && {
                    let source = std::ffi::CStr::from_ptr(debug.source).to_string_lossy();
                    let chunk = source.strip_prefix('=').or_else(|| source.strip_prefix('@')).unwrap_or(&source);
                    chunk != "[C]" && !chunk.starts_with("rur.") && !chunk.starts_with("ree.")
                };
            if is_script {
                ffi::lua_getfenv(state, -1);
                if ffi::lua_type(state, -1) == ffi::LUA_TTABLE {
                    ffi::lua_getfield(state, -1, c"script".as_ptr());
                    let found = raw::stack_instance(state, -1);
                    ffi::lua_pop(state, 3); // script, env, func
                    if found.is_some() {
                        return found;
                    }
                    continue;
                }
                ffi::lua_pop(state, 2); // env, func
                continue;
            }
            ffi::lua_pop(state, 1); // func
        }
    }
    None
}

/// typeof as scripts see it (registered table types, Vector3 for native vectors).
pub fn typeof_value(engine: &Engine, value: &LuaValue) -> String {
    match value {
        LuaValue::Nil => "nil".into(),
        LuaValue::Boolean(_) => "boolean".into(),
        LuaValue::Integer(_) | LuaValue::Number(_) => "number".into(),
        LuaValue::String(_) => "string".into(),
        LuaValue::Function(_) => "function".into(),
        LuaValue::Thread(_) => "thread".into(),
        LuaValue::Vector(_) => "Vector3".into(),
        LuaValue::UserData(ud) if raw::instance_of_ud(ud).is_some() => "Instance".into(),
        _ => {
            let prelude = engine.lua.borrow().prelude.clone();
            match prelude {
                Some(prelude) => prelude.typeof_fn.call::<String>(value.clone()).unwrap_or_else(|_| "userdata".into()),
                None => value.type_name().into(),
            }
        }
    }
}

/// The (cached) userdata of an instance.
#[inline]
pub fn instance_value(lua: &Lua, engine: &Engine, id: InstanceId) -> LuaResult<LuaValue> {
    raw::instance_value(lua, engine, id)
}

/// The instance a script value is, if any (no Lua call).
#[inline]
pub fn instance_id(value: &LuaValue) -> Option<InstanceId> {
    raw::instance_id(value)
}

pub fn enum_item_value(lua: &Lua, engine: &Engine, item: EnumItemId) -> LuaResult<LuaValue> {
    let index = item as usize;
    {
        let refs = engine.lua.borrow();
        if let Some(Some(existing)) = refs.enum_items.get(index) {
            return Ok(LuaValue::UserData(existing.clone()));
        }
    }
    let userdata = lua.create_any_userdata(types::LEnumItem(item))?;
    {
        let mut refs = engine.lua.borrow_mut();
        if refs.enum_items.len() <= index {
            refs.enum_items.resize(engine.reflection.enum_items.len().max(index + 1), None);
        }
        refs.enum_items[index] = Some(userdata.clone());
    }
    raw::remember_enum_item(engine, item, &userdata)?;
    Ok(LuaValue::UserData(userdata))
}

pub fn enum_value(lua: &Lua, engine: &Engine, id: EnumId) -> LuaResult<LuaValue> {
    let index = id as usize;
    {
        let refs = engine.lua.borrow();
        if let Some(Some(existing)) = refs.enums.get(index) {
            return Ok(LuaValue::UserData(existing.clone()));
        }
    }
    let userdata = lua.create_any_userdata(types::LEnum(id))?;
    let mut refs = engine.lua.borrow_mut();
    if refs.enums.len() <= index {
        refs.enums.resize(engine.reflection.enums.len().max(index + 1), None);
    }
    refs.enums[index] = Some(userdata.clone());
    Ok(LuaValue::UserData(userdata))
}

pub fn signal_value(lua: &Lua, engine: &Engine, signal: SignalId) -> LuaResult<LuaValue> {
    // With the raw Instance API, the signal userdata are kept in its Luau table only (the C `__index`
    // reads them there): a Rust-side handle each would take a slot of the VM's reference stack per
    // signal a script ever touched
    if let Some(existing) = raw::signal_userdata(engine, signal)? {
        if !existing.is_nil() {
            return Ok(existing);
        }
        let userdata = lua.create_any_userdata(types::LSignal(signal, engine.state.borrow().signals.generation(signal)))?;
        raw::remember_signal(engine, signal, &userdata)?;
        return Ok(LuaValue::UserData(userdata));
    }
    let index = signal as usize;
    {
        let refs = engine.lua.borrow();
        if let Some(Some(existing)) = refs.signals.get(index) {
            return Ok(LuaValue::UserData(existing.clone()));
        }
    }
    let userdata = lua.create_any_userdata(types::LSignal(signal, engine.state.borrow().signals.generation(signal)))?;
    {
        let mut refs = engine.lua.borrow_mut();
        if refs.signals.len() <= index {
            refs.signals.resize(index + 1, None);
        }
        refs.signals[index] = Some(userdata.clone());
    }
    raw::remember_signal(engine, signal, &userdata)?;
    Ok(LuaValue::UserData(userdata))
}

/// Replaces mlua's `__metatable = false` with Roblox's locked-metatable string for a registered
/// userdata type (the metatable is shared by every value of the type).
pub fn lock_metatable(lua: &Lua, sample: &LuaAnyUserData) -> LuaResult<()> {
    unsafe {
        lua.exec_raw::<()>(sample, |state| {
            use mlua::ffi;
            if ffi::lua_getmetatable(state, -1) != 0 {
                ffi::lua_pushstring(state, c"The metatable is locked".as_ptr());
                ffi::lua_setfield(state, -2, c"__metatable".as_ptr());
                ffi::lua_pop(state, 1);
            }
        })
    }
}

/// The text a script reads from an error a Rust callback raised through mlua (an mlua error object, caught by pcall) without
/// the traceback mlua writes after the message: a client's errors are one line (`tostring` of the object; the engine's
/// own report of an uncaught error leaves it out already: scheduler::error_message). The engine's datatypes raise plain
/// strings (lune-roblox datatypes::script_error), so this is for any other callback's error. A panic's text has none.
pub fn strip_callback_tracebacks(lua: &Lua) -> LuaResult<()> {
    unsafe extern "C-unwind" fn tostring(state: *mut mlua::ffi::lua_State) -> std::ffi::c_int {
        use mlua::ffi;
        unsafe {
            ffi::lua_settop(state, 1);
            ffi::lua_pushvalue(state, ffi::lua_upvalueindex(1));
            ffi::lua_insert(state, 1);
            ffi::lua_call(state, 1, 1);
            let mut length = 0;
            let text = ffi::lua_tolstring(state, -1, &mut length);
            if !text.is_null() {
                let bytes = std::slice::from_raw_parts(text.cast::<u8>(), length);
                let marker = b"\nstack traceback:";
                if let Some(at) = bytes.windows(marker.len()).position(|window| window == marker) {
                    ffi::lua_pushlstring(state, text, at);
                }
            }
            1
        }
    }
    let failing = lua.create_function(|_, ()| Err::<(), _>(LuaError::runtime("")))?;
    unsafe {
        lua.exec_raw::<()>(failing, |state| {
            use mlua::ffi;
            // (the error object pcall leaves is mlua's; its metatable is every such object's)
            if ffi::lua_pcall(state, 0, 0, 0) != ffi::LUA_OK && ffi::lua_type(state, -1) == ffi::LUA_TUSERDATA && ffi::lua_getmetatable(state, -1) != 0 {
                ffi::lua_pushstring(state, c"__tostring".as_ptr());
                ffi::lua_rawget(state, -2);
                if ffi::lua_type(state, -1) == ffi::LUA_TFUNCTION {
                    ffi::lua_pushcclosurek(state, tostring, c"__tostring".as_ptr(), 1, None);
                    ffi::lua_pushstring(state, c"__tostring".as_ptr());
                    ffi::lua_insert(state, -2);
                    ffi::lua_rawset(state, -3);
                } else {
                    ffi::lua_pop(state, 1);
                }
                ffi::lua_pop(state, 1);
            }
            ffi::lua_settop(state, 0);
        })
    }
}

/// Argument helpers (engine functions parse their own arguments: conversion failures must be
/// Roblox errors, not mlua's).
pub struct Args(pub LuaMultiValue);

impl Args {
    #[inline]
    pub fn new(args: LuaMultiValue) -> Self {
        Self(args)
    }

    #[inline]
    pub fn get(&self, index: usize) -> &LuaValue {
        self.0.get(index).unwrap_or(&LuaValue::Nil)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn number(&self, index: usize) -> Option<f64> {
        match self.get(index) {
            LuaValue::Number(n) => Some(*n),
            LuaValue::Integer(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn string(&self, index: usize) -> Option<LuaString> {
        match self.get(index) {
            LuaValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn boolean(&self, index: usize) -> bool {
        !matches!(self.get(index), LuaValue::Nil | LuaValue::Boolean(false))
    }

    #[inline]
    pub fn instance(&self, index: usize) -> Option<InstanceId> {
        raw::instance_id(self.get(index))
    }

    pub fn rest(&self, from: usize) -> LuaMultiValue {
        LuaMultiValue::from_iter(self.0.iter().skip(from).cloned())
    }
}

pub use globals::start;
