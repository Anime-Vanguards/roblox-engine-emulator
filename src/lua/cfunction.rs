//! Engine functions as scripts see them: one named C function each.
//!
//! On a playing client every engine function is one C function: `debug.info(f, "sln")` gives `[C]`, `-1` and the function's own
//! name (`print`, `wait` for task.wait, `new` for Vector3.new, `GetService` for game.GetService read as a value), `debug.info(f,
//! "a")` gives 0 and true, and an error raised inside one has no frame of its own: an `xpcall` handler's traceback lists the
//! handler, the script line that called the engine function, and the frames below it, and an error at level 1 is positioned at
//! that script line (measured, Studio 0.747, a playing client and a server: tools/parity/probes/engine_functions.luau). ree does
//! an engine function's work in Rust (an mlua callback answering the call protocol of lua/mod.rs) or in its own Luau (the prelude,
//! the service stubs), so each engine function is a C closure over that work, named as a client names it:
//!
//! - [`over_rust`]: over a Rust function that answers the call protocol ([`super::encode`]): its values are returned, its error
//!   is raised from this C function (at the calling line for level 1, as `luaL_where(L, 1)` gives it), and its yield is
//!   `lua_yield` from here, the values the thread is resumed with being the call's results (a continuation returns them).
//! - [`over_luau`]: over the engine's own Luau (or any function), called in protected mode, yieldably (`lua_pcallyieldable`):
//!   its results are returned and its error is raised again from this C function as it was raised (its message already holds
//!   the position its level gave it), so the handler of a script's `xpcall` runs after the engine Luau's frames are gone. The
//!   engine Luau computes error levels as from a C function's frame: one level for this function (the service stubs' ScriptLevel).
//! - [`raw`]: a body in Rust over the C API, run in this C function's own frame (print and warn, which call a value's
//!   `__tostring` from it, unprotected, as a client's do; require, which runs a module's chunk from it, so the chunk's caller is
//!   `[C] require`), with a continuation of its own for its yields.
//!
//! The work clock: a script's call of one is one call, the Rust or Luau it runs for the call being part of it
//! (patches/luau/20-engine-functions.patch, lua_setenginecfunction). A global or a library's function ([`Kind::Library`]) costs a
//! C function's call and first call in a frame (cost.rs LUAU_CCALL, LUAU_COLD_CFUNC, as measured for task.spawn and the
//! libraries); what the engine serves ([`Kind::Served`]: a datatype's constructors, functions and metamethods, an object's methods
//! read as values or written in Luau) costs what the engine Luau that stood for it did, a host call and its first call in a frame
//! (LUAU_HOSTCALL, LUAU_COLD_HOSTFUNC: the measured prices of a datatype's operation), a metamethod included.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::sync::{Mutex, OnceLock};

use mlua::{ffi, prelude::*};

/// How a script's call of an engine function is priced (the module's docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A global or a library's function: a C function's call (LUAU_CCALL) and first call in a frame (LUAU_COLD_CFUNC)
    Library,
    /// An operation the engine serves (a datatype's, an object's member): the engine Luau's host call (LUAU_HOSTCALL) and first
    /// call in a frame (LUAU_COLD_HOSTFUNC) it was
    Served,
}

unsafe extern "C-unwind" {
    fn lua_setenginecfunction(f: ffi::lua_CFunction, served: c_int);
    fn lua_pcallyieldable(state: *mut ffi::lua_State, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int;
}

/// A C closure keeps the pointer to its debug name, so names live for the process: one copy of each.
fn interned(name: &str) -> *const c_char {
    static NAMES: OnceLock<Mutex<HashMap<String, &'static CStr>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(found) = names.get(name) {
        return found.as_ptr();
    }
    let text: &'static CStr = Box::leak(CString::new(name.replace('\0', "")).unwrap_or_default().into_boxed_c_str());
    names.insert(name.to_string(), text);
    text.as_ptr()
}

/// Marks the trampolines as the VM's engine functions (lua_setenginecfunction), once a process, before any script runs.
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let trampolines = [rust_library as ffi::lua_CFunction, rust_served, luau_library, luau_served, raw_library, raw_served];
        assert!(
            trampolines.iter().enumerate().all(|(i, a)| trampolines[..i].iter().all(|b| !std::ptr::fn_addr_eq(*a, *b))),
            "engine function trampolines folded together"
        );
        unsafe {
            lua_setenginecfunction(rust_library, 0);
            lua_setenginecfunction(rust_served, 1);
            lua_setenginecfunction(luau_library, 0);
            lua_setenginecfunction(luau_served, 1);
            lua_setenginecfunction(raw_library, 0);
            lua_setenginecfunction(raw_served, 1);
        }
        // (a datatype's error positioned at its caller looks through the engine function's frame, as a client's has none)
        lune_roblox::datatypes::script_error::set_engine_frame(is_engine_frame);
    });
}

/// Whether the function at `level` of `state`'s stack is an engine function's C closure (one of this module's): what an error
/// position looks through, as the client's own C function it stands for (lune-roblox's caller_position).
///
/// # Safety
/// `state` must be the running thread.
pub unsafe fn is_engine_frame(state: *mut ffi::lua_State, level: c_int) -> bool {
    unsafe {
        let mut frame: ffi::lua_Debug = std::mem::zeroed();
        if ffi::lua_getinfo(state, level, c"f".as_ptr(), &mut frame) == 0 {
            return false;
        }
        let function = ffi::lua_tocfunction(state, -1);
        ffi::lua_pop(state, 1);
        is_trampoline(function)
    }
}

/// Whether the function at stack index `index` is an engine function's C closure.
///
/// # Safety
/// `index` must be a valid index of `state`'s stack.
pub unsafe fn is_engine_function(state: *mut ffi::lua_State, index: c_int) -> bool {
    unsafe { is_trampoline(ffi::lua_tocfunction(state, index)) }
}

fn is_trampoline(function: Option<ffi::lua_CFunction>) -> bool {
    let Some(function) = function else { return false };
    [rust_library as ffi::lua_CFunction, rust_served, luau_library, luau_served, raw_library, raw_served].iter().any(|&candidate| std::ptr::fn_addr_eq(candidate, function))
}

/// A new closure of the C function `function`: the same C function, name, upvalues and continuation (a Luau function is
/// answered as it is). A client's method of a signal, a connection, a vector, an EnumItem or a Random read as a value is a new
/// function at every read (not rawequal to another read; measured in a 0.747 playing client and a server:
/// tools/parity/probes/method_values.luau); a call through it is the same call.
pub fn copy(lua: &Lua, function: &LuaFunction) -> LuaResult<LuaFunction> {
    // SAFETY: the function is pushed as the argument; its upvalues are pushed above it and taken by the new closure, which
    // replaces it as the one result
    unsafe {
        lua.exec_raw::<LuaFunction>(function, |state| {
            let Some(body) = ffi::lua_tocfunction(state, -1) else { return };
            let mut debug: ffi::lua_Debug = std::mem::zeroed();
            if ffi::lua_getinfo(state, -1, c"nu".as_ptr(), &mut debug) == 0 {
                return;
            }
            let at = ffi::lua_gettop(state);
            let count = debug.nupvals as c_int;
            ffi::lua_rawcheckstack(state, count + 1);
            for index in 1..=count {
                ffi::lua_getupvalue(state, at, index);
            }
            ffi::lua_pushcclosurek(state, body, debug.name, count, continuation_of(body));
            ffi::lua_remove(state, at);
        })
    }
}

/// The continuation of a C function this module makes (none for any other)
fn continuation_of(function: ffi::lua_CFunction) -> Option<ffi::lua_Continuation> {
    let is = |candidate: ffi::lua_CFunction| std::ptr::fn_addr_eq(candidate, function);
    if is(rust_library) || is(rust_served) {
        Some(rust_continue)
    } else if is(luau_library) || is(luau_served) {
        Some(luau_continue)
    } else if is(raw_library) || is(raw_served) {
        Some(raw_continue)
    } else {
        None
    }
}

/// An engine function named `name` doing its work in `body`, an mlua function answering the call protocol (lua/mod.rs `encode`).
pub fn over_rust(lua: &Lua, name: &str, kind: Kind, body: LuaFunction) -> LuaResult<LuaFunction> {
    let trampoline = match kind {
        Kind::Library => rust_library as ffi::lua_CFunction,
        Kind::Served => rust_served,
    };
    closure(lua, name, trampoline, rust_continue, body)
}

/// An engine function named `name` doing its work in `body` (the engine's own Luau), whose error it raises again as raised.
pub fn over_luau(lua: &Lua, name: &str, kind: Kind, body: LuaFunction) -> LuaResult<LuaFunction> {
    let trampoline = match kind {
        Kind::Library => luau_library as ffi::lua_CFunction,
        Kind::Served => luau_served,
    };
    closure(lua, name, trampoline, luau_continue, body)
}

/// A raw engine function's body: run in the engine function's own C frame, with the stack of a C function (its arguments), its
/// extra values at [`extra`]`(1)` and up. It returns its count of results, or `lua_yield`'s value, after which `resume` (the
/// continuation given with it) runs when the thread is resumed, with the slots below the yielded values kept.
pub type RawBody = unsafe fn(*mut ffi::lua_State) -> c_int;
/// A raw engine function's continuation (see [`RawBody`]).
pub type RawContinue = unsafe fn(*mut ffi::lua_State, c_int) -> c_int;

/// The stack index of a raw engine function's extra value `n` (1-based).
pub fn extra(n: c_int) -> c_int {
    ffi::lua_upvalueindex(2 + n)
}

/// An engine function named `name` whose body runs in its own C frame ([`RawBody`]), over `extras`.
pub fn raw(lua: &Lua, name: &str, kind: Kind, body: RawBody, resume: Option<RawContinue>, extras: Vec<LuaValue>) -> LuaResult<LuaFunction> {
    install();
    let trampoline = match kind {
        Kind::Library => raw_library as ffi::lua_CFunction,
        Kind::Served => raw_served,
    };
    let name = interned(name);
    let count = extras.len() as c_int;
    // SAFETY: the extras are pushed as the arguments; the two function pointers go below them as light userdata, and the closure
    // takes all of them as its upvalues and is left as the one result
    unsafe {
        lua.exec_raw(LuaMultiValue::from_vec(extras), move |state| {
            ffi::lua_rawcheckstack(state, 3);
            ffi::lua_pushlightuserdata(state, body as *mut std::ffi::c_void);
            ffi::lua_insert(state, -(count + 1));
            match resume {
                Some(resume) => ffi::lua_pushlightuserdata(state, resume as *mut std::ffi::c_void),
                None => ffi::lua_pushnil(state),
            }
            ffi::lua_insert(state, -(count + 1));
            ffi::lua_pushcclosurek(state, trampoline, name, count + 2, Some(raw_continue));
        })
    }
}

fn closure(lua: &Lua, name: &str, trampoline: ffi::lua_CFunction, continuation: ffi::lua_Continuation, body: LuaFunction) -> LuaResult<LuaFunction> {
    install();
    let name = interned(name);
    // SAFETY: the body is pushed as the argument and taken by the closure as its upvalue; the closure is left as the one result
    unsafe {
        lua.exec_raw(body, move |state| {
            ffi::lua_pushcclosurek(state, trampoline, name, 1, Some(continuation));
        })
    }
}

// Over Rust

// (each kind's trampoline is a function of its own, which the VM tells apart by its address: the kind is kept in its code, as an
// operand the optimizer cannot drop, so identical-code folding cannot merge a library trampoline with a served one)
unsafe extern "C-unwind" fn rust_library(state: *mut ffi::lua_State) -> c_int {
    unsafe { rust_call(state, Kind::Library) }
}

unsafe extern "C-unwind" fn rust_served(state: *mut ffi::lua_State) -> c_int {
    unsafe { rust_call(state, Kind::Served) }
}

#[inline(always)]
unsafe fn rust_call(state: *mut ffi::lua_State, kind: Kind) -> c_int {
    std::hint::black_box(kind);
    unsafe {
        let count = ffi::lua_gettop(state);
        ffi::lua_rawcheckstack(state, 1);
        ffi::lua_pushvalue(state, ffi::lua_upvalueindex(1));
        ffi::lua_insert(state, 1);
        // (yieldable: this function has a continuation, so the Rust may check that the thread can yield as its caller can)
        ffi::lua_call(state, count, ffi::LUA_MULTRET);
        finish(state)
    }
}

/// The body's results (the whole stack): a protocol error raised from here, a protocol yield made from here, else returned.
unsafe fn finish(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let count = ffi::lua_gettop(state);
        if count >= 1 && ffi::lua_type(state, 1) == ffi::LUA_TLIGHTUSERDATA && ffi::lua_tolightuserdata(state, 1) == super::signal_marker_ptr() {
            let kind = ffi::lua_tonumberx(state, 2, std::ptr::null_mut()) as c_int;
            if kind == 1 {
                let level = ffi::lua_tonumberx(state, 4, std::ptr::null_mut()) as c_int;
                ffi::lua_settop(state, 3);
                raise_at(state, level)
            }
            // (the scheduler has the thread's resumption: its values are the call's results, which the continuation returns)
            ffi::lua_settop(state, 0);
            return ffi::lua_yield(state, 0);
        }
        count
    }
}

/// Charges the work clock what a client spends on an error an engine function raises, from where ree raises one: an engine
/// function's error ([`raise_at`], and an engine Luau body's raised again by [`luau_continue`]), an Instance metamethod's
/// (lua/raw.rs raise, raise_top) and require's (lua/builtins.rs). The error is the cost, whatever raised it (a playing client and
/// a server, Studio 0.747): cost.rs RAISED for a plain error, the engine's own ("Argument 1 missing or nil", a member that is not,
/// a service name, a spatial query's), RAISED_AT_CALLER for one at the calling line, an argument check in Luau's words (`positioned`).
/// Nothing when the error goes back to the engine's own Luau (the frame it returns to is a chunk of the engine's, or a C function
/// one called: its pcall), which handles it or raises it again from its engine function, charged then; nothing while the engine's
/// Luau prices a whole call (the clock muted).
///
/// # Safety
/// `state` must be the running thread, inside the C function that raises the error.
pub(crate) unsafe fn charge_raised(state: *mut ffi::lua_State, positioned: bool) {
    unsafe {
        if raised_into_engine(state) {
            return;
        }
        let lua = Lua::get_or_init_from_ptr(state);
        let Ok(Some(engine)) = lua.try_app_data_ref::<std::rc::Rc<crate::Engine>>() else { return };
        let Ok(engine_state) = engine.state.try_borrow() else { return };
        let seconds = if positioned { crate::cost::RAISED_AT_CALLER } else { crate::cost::RAISED };
        engine_state.sched.charge(seconds);
        if !engine_state.sched.is_muted() {
            crate::boundary::add_cost(seconds);
        }
    }
}

/// Whether an error the running C function raises now goes back to the engine's own Luau: its caller is a chunk of the engine's
/// (`rur.*`, `ree.*`, mlua's own), or a C function such a chunk called.
unsafe fn raised_into_engine(state: *mut ffi::lua_State) -> bool {
    fn engine_chunk(frame: &ffi::lua_Debug) -> bool {
        if frame.source.is_null() {
            return false;
        }
        let source = unsafe { CStr::from_ptr(frame.source) }.to_bytes();
        let name = source.strip_prefix(b"=").or_else(|| source.strip_prefix(b"@")).unwrap_or(source);
        name.starts_with(b"rur.") || name.starts_with(b"ree.") || name.starts_with(b"__mlua")
    }
    unsafe {
        let mut frame: ffi::lua_Debug = std::mem::zeroed();
        if ffi::lua_getinfo(state, 1, c"s".as_ptr(), &mut frame) == 0 {
            return false;
        }
        if engine_chunk(&frame) {
            return true;
        }
        let c_function = !frame.what.is_null() && CStr::from_ptr(frame.what).to_bytes() == b"C";
        c_function && ffi::lua_getinfo(state, 2, c"s".as_ptr(), &mut frame) != 0 && engine_chunk(&frame)
    }
}

/// Whether the error on top of the stack is a string positioned at the calling line (`luaL_where(L, 1)`'s text begins it).
unsafe fn at_caller(state: *mut ffi::lua_State) -> bool {
    unsafe {
        if ffi::lua_type(state, -1) != ffi::LUA_TSTRING {
            return false;
        }
        ffi::lua_rawcheckstack(state, 1);
        ffi::luaL_where(state, 1);
        let (mut where_length, mut length) = (0, 0);
        let position = ffi::lua_tolstring(state, -1, &mut where_length);
        let message = ffi::lua_tolstring(state, -2, &mut length);
        let positioned = where_length > 0
            && length >= where_length
            && std::slice::from_raw_parts(message.cast::<u8>(), where_length) == std::slice::from_raw_parts(position.cast::<u8>(), where_length);
        ffi::lua_pop(state, 1);
        positioned
    }
}

/// Raises the message on top of the stack, after the position of the function `level` frames up (1: this C function's caller,
/// the script line that called the engine function), or alone for level 0.
unsafe fn raise_at(state: *mut ffi::lua_State, level: c_int) -> ! {
    unsafe {
        charge_raised(state, level > 0);
        if level > 0 {
            ffi::luaL_where(state, level);
            ffi::lua_insert(state, -2);
            ffi::lua_concat(state, 2);
        }
        ffi::lua_error(state)
    }
}

/// After a protocol yield: the values the thread was resumed with are the stack (grown over them first, as raw.rs's namecall
/// continuation does), and they are the call's results.
unsafe extern "C-unwind" fn rust_continue(state: *mut ffi::lua_State, _status: c_int) -> c_int {
    unsafe {
        ffi::lua_rawcheckstack(state, 1);
        ffi::lua_gettop(state)
    }
}

// Over Luau

unsafe extern "C-unwind" fn luau_library(state: *mut ffi::lua_State) -> c_int {
    unsafe { luau_call(state, Kind::Library) }
}

unsafe extern "C-unwind" fn luau_served(state: *mut ffi::lua_State) -> c_int {
    unsafe { luau_call(state, Kind::Served) }
}

#[inline(always)]
unsafe fn luau_call(state: *mut ffi::lua_State, kind: Kind) -> c_int {
    std::hint::black_box(kind);
    unsafe {
        let count = ffi::lua_gettop(state);
        ffi::lua_rawcheckstack(state, 1);
        ffi::lua_pushvalue(state, ffi::lua_upvalueindex(1));
        ffi::lua_insert(state, 1);
        lua_pcallyieldable(state, count, ffi::LUA_MULTRET, 0)
    }
}

/// The body's outcome, when it returns at once or after yielding: its results, or its error raised again as it was raised.
unsafe extern "C-unwind" fn luau_continue(state: *mut ffi::lua_State, status: c_int) -> c_int {
    unsafe {
        ffi::lua_rawcheckstack(state, 1);
        if status != ffi::LUA_OK {
            charge_raised(state, at_caller(state));
            ffi::lua_error(state)
        }
        ffi::lua_gettop(state)
    }
}

// Raw

unsafe extern "C-unwind" fn raw_library(state: *mut ffi::lua_State) -> c_int {
    unsafe { raw_call(state, Kind::Library) }
}

unsafe extern "C-unwind" fn raw_served(state: *mut ffi::lua_State) -> c_int {
    unsafe { raw_call(state, Kind::Served) }
}

#[inline(always)]
unsafe fn raw_call(state: *mut ffi::lua_State, kind: Kind) -> c_int {
    std::hint::black_box(kind);
    unsafe {
        // SAFETY: the first upvalue is the body's pointer, which [`raw`] put there
        let body: RawBody = std::mem::transmute::<*mut std::ffi::c_void, RawBody>(ffi::lua_tolightuserdata(state, ffi::lua_upvalueindex(1)));
        body(state)
    }
}

unsafe extern "C-unwind" fn raw_continue(state: *mut ffi::lua_State, status: c_int) -> c_int {
    unsafe {
        ffi::lua_rawcheckstack(state, 1);
        let resume = ffi::lua_tolightuserdata(state, ffi::lua_upvalueindex(2));
        if resume.is_null() {
            return ffi::lua_gettop(state);
        }
        // SAFETY: the second upvalue is the continuation's pointer, which [`raw`] put there
        let resume: RawContinue = std::mem::transmute::<*mut std::ffi::c_void, RawContinue>(resume);
        resume(state, status)
    }
}
