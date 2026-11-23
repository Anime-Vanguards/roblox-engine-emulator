//! Globals whose bodies run in their own C frame (lua/cfunction.rs [`raw`](super::cfunction::raw)): print and warn, require, and
//! coroutine.create and wrap. A client's are C functions that call script code from their frame: print calls a value's
//! `__tostring` unprotected (an error it raises reaches the script's `xpcall` handler at the raise, the `__tostring` frame first,
//! and `debug.info(3)` in it is `[C] print`), and require runs a module's chunk, whose caller is `[C] require` (measured, Studio
//! 0.747, a playing client and a server: tools/parity/probes/engine_functions.luau).

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

/// A script's require (prelude.luau Environment): the module's value, through `begin` and `finish` (lua/modules.rs
/// require_begin and require_finish, as prelude.luau Require answers them), the requiring script and its VM's key given to them.
/// A module whose loading another thread has begun is waited for (a yield, then the asking again), and one that must load runs
/// its chunk from here, in protected mode and yieldably, so a chunk that waits keeps the requiring thread waiting.
pub fn require_function(lua: &Lua, script: LuaValue, vm: LuaValue, begin: LuaFunction, finish: LuaFunction) -> LuaResult<LuaFunction> {
    let extras = vec![script, vm, LuaValue::Function(begin), LuaValue::Function(finish)];
    cfunction::raw(lua, "require", Kind::Library, require_body, Some(require_resume), extras)
}

// The slots of require's frame: its target, then what the continuation goes on with
const TARGET: c_int = 1;
const PHASE: c_int = 2;
const KEY: c_int = 3;
const CHUNK: c_int = 4;
// (phases are the VM's integer, which is i32 on wasm32: the browser build matches them against lua_tointegerx)
/// Waiting on another thread's load: ask again when resumed
const WAITING: ffi::lua_Integer = 1;
/// Running the module's chunk: report its outcome
const RUNNING: ffi::lua_Integer = 2;

unsafe fn require_body(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::lua_settop(state, TARGET);
        require_ask(state)
    }
}

/// RequireBegin, and what its answer says (prelude.luau Require's loop).
unsafe fn require_ask(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::lua_settop(state, TARGET);
        ffi::luaL_checkstack(state, 6, c"require".as_ptr());
        ffi::lua_pushvalue(state, extra(3));
        ffi::lua_pushvalue(state, TARGET);
        ffi::lua_pushvalue(state, extra(1));
        ffi::lua_pushvalue(state, extra(2));
        ffi::lua_call(state, 3, 3);
        // (status, A, B at 2, 3, 4)
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
            2 => {
                ffi::lua_settop(state, TARGET);
                ffi::lua_pushinteger(state, WAITING);
                ffi::lua_yield(state, 0)
            }
            _ => {
                // (3: run the chunk A, then report its outcome for the module B)
                ffi::lua_pushinteger(state, RUNNING);
                ffi::lua_replace(state, PHASE);
                // (stack: target, phase, A, B: the key goes to 3, the chunk to 4)
                ffi::lua_insert(state, KEY);
                cfunction_pcall(state)
            }
        }
    }
}

unsafe extern "C-unwind" {
    fn lua_pcallyieldable(state: *mut ffi::lua_State, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int;
}

/// The module's chunk (at [`CHUNK`]), called in protected mode, yieldably: [`require_resume`] gets its outcome.
unsafe fn cfunction_pcall(state: *mut ffi::lua_State) -> c_int {
    unsafe { lua_pcallyieldable(state, 0, ffi::LUA_MULTRET, 0) }
}

unsafe fn require_resume(state: *mut ffi::lua_State, status: c_int) -> c_int {
    unsafe {
        match ffi::lua_tointegerx(state, PHASE, std::ptr::null_mut()) {
            WAITING => require_ask(state),
            RUNNING => {
                // (the chunk's results from CHUNK up, or its error, on top of whatever its frames left when it ran as a re-entry of
                // the interpreter, as Luau's own pcall continuation reads it: packed as table.pack(pcall(chunk)) packs them)
                if status != ffi::LUA_OK {
                    ffi::lua_replace(state, CHUNK);
                    ffi::lua_settop(state, CHUNK);
                }
                let results = ffi::lua_gettop(state) - KEY;
                ffi::luaL_checkstack(state, 6, c"require".as_ptr());
                ffi::lua_createtable(state, results + 1, 1);
                ffi::lua_pushboolean(state, (status == ffi::LUA_OK) as c_int);
                ffi::lua_rawseti_(state, -2, 1);
                for index in 0..results {
                    ffi::lua_pushvalue(state, CHUNK + index);
                    ffi::lua_rawseti_(state, -2, index + 2);
                }
                ffi::lua_pushinteger(state, (results + 1) as _);
                ffi::lua_setfield(state, -2, c"n".as_ptr());
                let packed = ffi::lua_gettop(state);
                ffi::lua_pushvalue(state, extra(4));
                ffi::lua_pushvalue(state, KEY);
                ffi::lua_pushvalue(state, packed);
                ffi::lua_pushvalue(state, extra(2));
                ffi::lua_call(state, 3, 3);
                // (finished, message, level)
                if ffi::lua_toboolean(state, -3) != 0 {
                    ffi::lua_pop(state, 1);
                    return 1;
                }
                let level = ffi::lua_tointegerx(state, -1, std::ptr::null_mut());
                ffi::lua_pop(state, 1);
                raise(state, level != 0)
            }
            _ => ffi::lua_gettop(state),
        }
    }
}

/// Raises the message on top of the stack, after the requiring line's position when `positioned`.
unsafe fn raise(state: *mut ffi::lua_State, positioned: bool) -> ! {
    unsafe {
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
pub fn coroutine_function(lua: &Lua, name: &'static str, native: LuaFunction, charge: LuaFunction, seconds: f64) -> LuaResult<LuaFunction> {
    let extras = vec![LuaValue::Function(native), LuaValue::Function(charge), LuaValue::Number(seconds)];
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
        1
    }
}
