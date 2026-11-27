//! Globals whose bodies run in their own C frame (lua/cfunction.rs [`raw`](super::cfunction::raw)): print and warn, require, and
//! coroutine.create and wrap. A client's are C functions that call script code from their frame: print calls a value's
//! `__tostring` unprotected (an error it raises reaches the script's `xpcall` handler at the raise, the `__tostring` frame first,
//! and `debug.info(3)` in it is `[C] print`; measured, Studio 0.747, a playing client and a server:
//! tools/parity/probes/engine_functions.luau), and require runs a module's chunk on a thread of its own (lua/modules.rs require_run).

use std::ffi::c_int;

use mlua::{ffi, prelude::*};

use super::cfunction::{self, Kind, extra};

/// print or warn (`kind`): each argument written as `tostring` writes it (its `__tostring` called from here), a SharedTable
/// marked first as a client marks one it writes (lua/shared_table.rs `mark_value`, through `mark`), joined by spaces and handed to
/// `output` (prelude.luau's former Stringify and Print, as they did).
pub fn output_function(lua: &Lua, kind: &'static str, output: LuaFunction, mark: LuaFunction) -> LuaResult<LuaFunction> {
    let extras = vec![LuaValue::String(lua.create_string(kind)?), LuaValue::Function(output), LuaValue::Function(mark)];
    cfunction::raw(lua, kind, Kind::Library, output_body, None, extras)
}

unsafe fn output_body(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let count = ffi::lua_gettop(state);
        ffi::luaL_checkstack(state, 2 * count + 4, c"print".as_ptr());
        for index in 1..=count {
            if index > 1 {
                ffi::lua_pushlstring_(state, c" ".as_ptr(), 1);
            }
            if ffi::lua_type(state, index) == ffi::LUA_TUSERDATA {
                ffi::lua_pushvalue(state, extra(3));
                ffi::lua_pushvalue(state, index);
                ffi::lua_call(state, 1, 0);
            }
            // (the VM's own, as tostring is: mlua-sys's luaL_tolstring is a compatibility copy of its own)
            ffi::luaL_tolstring_(state, index, std::ptr::null_mut());
        }
        if count == 0 {
            ffi::lua_pushlstring_(state, c"".as_ptr(), 0);
        } else if count > 1 {
            ffi::lua_concat(state, 2 * count - 1);
        }
        ffi::lua_pushvalue(state, extra(2));
        ffi::lua_insert(state, -2);
        ffi::lua_pushvalue(state, extra(1));
        ffi::lua_insert(state, -2);
        ffi::lua_call(state, 2, 0);
        0
    }
}

/// The scripts' require, one global function as a client's is (every script's environment reads it through its metatable,
/// lua/modules.rs environment_in): the module's value, through `begin` and `run` (lua/modules.rs require_begin and require_run,
/// as prelude.luau Require answers them). It finds the requiring script as a client does, at each call: the nearest Luau function
/// up the stack (C functions such as pcall passed over, and the engine's own Luau, which runs in the VM's globals), whose
/// environment's own `script` (read raw, as it is at the call: a `setfenv` or a write of `script` changes it) is the script a
/// path is resolved from, and whose environment says which VM's records to read (its metatable's [`VM_FIELD`]: an Actor's VM). With
/// no Luau function on the stack (`coroutine.create(require)`) a path raises; an Instance is required as from anywhere. Measured, a
/// 0.747 playing client and a server: tools/parity/probes/script_env.luau. A module that must load has its chunk run on a thread of
/// its own (require_run, as a client does), and one whose chunk yielded, or whose loading another thread began, is waited for (a
/// yield, then the asking again).
pub fn require_function(lua: &Lua, begin: LuaFunction, run: LuaFunction) -> LuaResult<LuaFunction> {
    let extras = vec![LuaValue::Function(begin), LuaValue::Function(run)];
    cfunction::raw(lua, "require", Kind::Library, require_body, Some(require_resume), extras)
}

/// The field of a script environment's metatable that holds its VM's key (an Actor's: lua/modules.rs vm_key), absent for the main
/// VM's. Scripts never see it: the metatable is locked.
pub const VM_FIELD: &std::ffi::CStr = c"ree.vm";

/// Pushes the requiring script and its VM's key (see [`require_function`]): the environment's own `script` of the nearest Luau
/// function up the stack that is no engine Luau, and the key its environment's metatable holds (nil for the main VM); `false` and
/// nil when no such function is on the stack.
unsafe fn push_caller(state: *mut ffi::lua_State) {
    unsafe {
        ffi::luaL_checkstack(state, 5, c"require".as_ptr());
        let mut debug: ffi::lua_Debug = std::mem::zeroed();
        let mut level = 1;
        while ffi::lua_getinfo(state, level, c"f".as_ptr(), &mut debug) != 0 {
            level += 1;
            if ffi::lua_iscfunction(state, -1) != 0 {
                ffi::lua_pop(state, 1);
                continue;
            }
            ffi::lua_getfenv(state, -1);
            if ffi::lua_type(state, -1) != ffi::LUA_TTABLE || ffi::lua_rawequal(state, -1, ffi::LUA_GLOBALSINDEX) != 0 {
                ffi::lua_pop(state, 2);
                continue;
            }
            // (function, environment)
            ffi::lua_rawgetfield(state, -1, c"script".as_ptr());
            if ffi::lua_getmetatable(state, -2) != 0 {
                ffi::lua_rawgetfield(state, -1, VM_FIELD.as_ptr());
                ffi::lua_remove(state, -2);
            } else {
                ffi::lua_pushnil(state);
            }
            // (function, environment, script, vm)
            ffi::lua_remove(state, -4);
            ffi::lua_remove(state, -3);
            return;
        }
        ffi::lua_pushboolean(state, 0);
        ffi::lua_pushnil(state);
    }
}

// The slots of require's frame: its target, then what the continuation goes on with
const TARGET: c_int = 1;
const PHASE: c_int = 2;
// (phases are the VM's integer, which is i32 on wasm32: the browser build matches them against lua_tointegerx)
/// Waiting on a module's load: ask again when resumed
const WAITING: ffi::lua_Integer = 1;

unsafe fn require_body(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::lua_settop(state, TARGET);
        require_ask(state)
    }
}

/// RequireBegin (and RequireRun when the module must load), and what the answer says (prelude.luau Require's loop).
unsafe fn require_ask(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::lua_settop(state, TARGET);
        ffi::luaL_checkstack(state, 6, c"require".as_ptr());
        ffi::lua_pushvalue(state, extra(1));
        ffi::lua_pushvalue(state, TARGET);
        push_caller(state);
        // (the VM's key, for RequireRun)
        ffi::lua_pushvalue(state, -1);
        ffi::lua_insert(state, 2);
        ffi::lua_call(state, 3, 3);
        // (vm at 2; status, A, B at 3, 4, 5)
        if ffi::lua_tointegerx(state, 3, std::ptr::null_mut()) == 3 {
            // (run the chunk A for the module B: RequireRun answers as RequireBegin does, 2 when it yielded)
            ffi::lua_pushvalue(state, extra(2));
            ffi::lua_insert(state, 4);
            ffi::lua_pushvalue(state, 2);
            ffi::lua_call(state, 3, 3);
            ffi::lua_remove(state, 3);
        }
        ffi::lua_remove(state, 2);
        match ffi::lua_tointegerx(state, 2, std::ptr::null_mut()) {
            0 => {
                ffi::lua_settop(state, 3);
                1
            }
            1 => {
                // (an error: B is its level, 0 alone, else the requiring line's position)
                let level = ffi::lua_tointegerx(state, 4, std::ptr::null_mut());
                ffi::lua_settop(state, 3);
                raise(state, level != 0)
            }
            _ => {
                ffi::lua_settop(state, TARGET);
                ffi::lua_pushinteger(state, WAITING);
                ffi::lua_yield(state, 0)
            }
        }
    }
}

unsafe fn require_resume(state: *mut ffi::lua_State, _status: c_int) -> c_int {
    unsafe {
        match ffi::lua_tointegerx(state, PHASE, std::ptr::null_mut()) {
            WAITING => require_ask(state),
            _ => ffi::lua_gettop(state),
        }
    }
}

/// Raises the message on top of the stack, after the requiring line's position when `positioned`, charged what a client spends
/// on it (cfunction.rs charge_raised).
unsafe fn raise(state: *mut ffi::lua_State, positioned: bool) -> ! {
    unsafe {
        cfunction::charge_raised(state, positioned);
        if positioned {
            ffi::luaL_where(state, 1);
            ffi::lua_insert(state, -2);
            ffi::lua_concat(state, 2);
        }
        ffi::lua_error(state)
    }
}

/// coroutine.create or wrap (`name`): the VM's own (`native`), after charging the script for the thread it makes as task.spawn
/// does (cost.rs THREAD, through `charge`) and checking its function here, so an error names the line that called it.
pub fn coroutine_function(lua: &Lua, name: &'static str, native: LuaFunction, charge: LuaFunction, seconds: f64, made: Option<LuaFunction>) -> LuaResult<LuaFunction> {
    let made = made.map_or(LuaValue::Nil, LuaValue::Function);
    let extras = vec![LuaValue::Function(native), LuaValue::Function(charge), LuaValue::Number(seconds), made];
    cfunction::raw(lua, name, Kind::Library, coroutine_body, None, extras)
}

unsafe fn coroutine_body(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::luaL_checkstack(state, 3, c"coroutine".as_ptr());
        ffi::lua_pushvalue(state, extra(2));
        ffi::lua_pushvalue(state, extra(3));
        ffi::lua_call(state, 1, 0);
        // (none at all is `missing argument #1 to 'create' (function expected)`, nil `invalid argument #1 ... got nil`: measured)
        ffi::luaL_checktype(state, 1, ffi::LUA_TFUNCTION);
        ffi::lua_settop(state, 1);
        ffi::lua_pushvalue(state, extra(1));
        ffi::lua_insert(state, 1);
        ffi::lua_call(state, 1, 1);
        // (`made`, given the new thread, which takes its maker's memory category: create's, or the one wrap's function resumes, its
        // C closure's upvalue; lua/globals.rs)
        if ffi::lua_type(state, extra(4)) == ffi::LUA_TFUNCTION && super::globals::memory_categories_set() {
            ffi::lua_pushvalue(state, extra(4));
            if ffi::lua_type(state, -2) == ffi::LUA_TFUNCTION {
                if ffi::lua_getupvalue(state, -2, 1).is_null() {
                    ffi::lua_pushnil(state);
                }
            } else {
                ffi::lua_pushvalue(state, -2);
            }
            if ffi::lua_type(state, -1) == ffi::LUA_TTHREAD {
                ffi::lua_call(state, 1, 0);
            } else {
                ffi::lua_pop(state, 2);
            }
        }
        1
    }
}
