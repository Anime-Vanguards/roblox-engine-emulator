//! Vector3's native methods (Abs, Ceil, Floor, Sign, Dot, Cross, Max, Min: lua/vector3.luau's VectorMethods) with a
//! playing client's argument errors. They are the VM's `vector` library functions, called straight from a C function of
//! the method's own name that checks the arguments first, as a client's Luau checks do (Studio 0.747,
//! tools/parity/probes/datatype_errors.luau): `missing argument #1 to 'Abs' (Vector3 expected)`, `invalid argument #1 to
//! 'Abs' (Vector3 expected, got number)` for self (only a `.` call can pass a wrong one, and names the method), and
//! `invalid argument #2 (Vector3 expected, got number)` for a later argument as a `:` call reports it, from the calling
//! line. The library's own checks say `vector expected` and name the library function (`to 'abs'`). A call is one C call
//! as before: the library function runs on the wrapper's own stack, without a call of its own.

use std::{ffi::c_int, sync::OnceLock};

use lune_roblox::datatypes::script_error::{raise_script, type_name_at};
use mlua::{ffi, prelude::*};

/// The library functions, by method: abs, ceil, floor, sign, dot, cross, max, min
static LIBRARY: OnceLock<[ffi::lua_CFunction; 8]> = OnceLock::new();

const ABS: usize = 0;
const CEIL: usize = 1;
const FLOOR: usize = 2;
const SIGN: usize = 3;
const DOT: usize = 4;
const CROSS: usize = 5;
const MAX: usize = 6;
const MIN: usize = 7;

/// How a method checks its arguments after self
#[derive(Clone, Copy)]
enum Rest {
    /// none (more are ignored)
    None,
    /// exactly one Vector3 (more are ignored)
    One,
    /// any number of Vector3s
    Any,
}

/// The methods as C functions by name, over the library functions of `vector` (the global table's).
pub fn methods(lua: &Lua) -> LuaResult<LuaTable> {
    let vector: LuaTable = lua.globals().get("vector")?;
    let names = ["abs", "ceil", "floor", "sign", "dot", "cross", "max", "min"];
    let mut library: [Option<ffi::lua_CFunction>; 8] = [None; 8];
    for (slot, name) in library.iter_mut().zip(names) {
        let function: LuaFunction = vector.get(name)?;
        let pointer = unsafe {
            lua.exec_raw::<LuaLightUserData>(&function, |state| {
                let function = ffi::lua_tocfunction(state, -1);
                ffi::lua_pop(state, 1);
                ffi::lua_pushlightuserdata(state, function.map_or(std::ptr::null_mut(), |f| f as *mut std::ffi::c_void));
            })?
        };
        if pointer.0.is_null() {
            return Err(LuaError::runtime(format!("vector.{name} is not a C function")));
        }
        *slot = Some(unsafe { std::mem::transmute::<*mut std::ffi::c_void, ffi::lua_CFunction>(pointer.0) });
    }
    let library = library.map(|function| function.expect("every library function"));
    let _ = LIBRARY.set(library);

    let table = lua.create_table()?;
    let entries: [(&std::ffi::CStr, ffi::lua_CFunction); 8] = [
        (c"Abs", abs),
        (c"Ceil", ceil),
        (c"Floor", floor),
        (c"Sign", sign),
        (c"Dot", dot),
        (c"Cross", cross),
        (c"Max", max),
        (c"Min", min),
    ];
    for (name, function) in entries {
        let method = unsafe {
            lua.exec_raw::<LuaFunction>((), |state| {
                ffi::lua_pushcclosurek(state, function, name.as_ptr(), 0, None);
            })?
        };
        table.raw_set(name.to_str().expect("an ASCII name"), method)?;
    }
    Ok(table)
}

/// Checks the arguments of method `name` (self a Vector3, then `rest`), raising the client's error, then runs library
/// function `index` on this stack.
unsafe fn checked(state: *mut ffi::lua_State, name: &str, rest: Rest, index: usize) -> c_int {
    unsafe {
        let count = ffi::lua_gettop(state);
        if count < 1 {
            raise_script(state, format!("missing argument #1 to '{name}' (Vector3 expected)"), true);
        }
        if ffi::lua_type(state, 1) != ffi::LUA_TVECTOR {
            let message = format!("invalid argument #1 to '{name}' (Vector3 expected, got {})", type_name_at(state, 1));
            raise_script(state, message, true);
        }
        let last = match rest {
            Rest::None => 1,
            Rest::One => 2,
            Rest::Any => count.max(1),
        };
        for argument in 2..=last {
            if argument > count {
                raise_script(state, format!("missing argument #{argument} (Vector3 expected)"), true);
            }
            if ffi::lua_type(state, argument) != ffi::LUA_TVECTOR {
                let message = format!("invalid argument #{argument} (Vector3 expected, got {})", type_name_at(state, argument));
                raise_script(state, message, true);
            }
        }
        let library = LIBRARY.get().expect("the vector library functions, read when the methods were made");
        (library[index])(state)
    }
}

unsafe extern "C-unwind" fn abs(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Abs", Rest::None, ABS) }
}

unsafe extern "C-unwind" fn ceil(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Ceil", Rest::None, CEIL) }
}

unsafe extern "C-unwind" fn floor(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Floor", Rest::None, FLOOR) }
}

unsafe extern "C-unwind" fn sign(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Sign", Rest::None, SIGN) }
}

unsafe extern "C-unwind" fn dot(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Dot", Rest::One, DOT) }
}

unsafe extern "C-unwind" fn cross(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Cross", Rest::One, CROSS) }
}

unsafe extern "C-unwind" fn max(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Max", Rest::Any, MAX) }
}

unsafe extern "C-unwind" fn min(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Min", Rest::Any, MIN) }
}
