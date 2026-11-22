//! Vector3's native methods (Abs, Ceil, Floor, Sign, Dot, Cross, Max, Min: lua/vector3.luau's VectorMethods) with a
//! playing client's argument errors. They are the VM's `vector` library functions, called straight from a C function of
//! the method's own name that checks the arguments first, as a client's Luau checks do (Studio 0.747,
//! tools/parity/probes/datatype_errors.luau): `missing argument #1 to 'Abs' (Vector3 expected)`, `invalid argument #1 to
//! 'Abs' (Vector3 expected, got number)` for self (only a `.` call can pass a wrong one, and names the method), and
//! `invalid argument #2 (Vector3 expected, got number)` for a later argument as a `:` call reports it, from the calling
//! line. The library's own checks say `vector expected` and name the library function (`to 'abs'`). A call is one C call
//! as before: the library function runs on the wrapper's own stack, without a call of its own.

use std::{ffi::{c_int, CStr}, sync::OnceLock};

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
    let entries: [(&std::ffi::CStr, ffi::lua_CFunction); 9] = [
        (c"Abs", abs),
        (c"Ceil", ceil),
        (c"Floor", floor),
        (c"Sign", sign),
        (c"Dot", dot),
        (c"Cross", cross),
        (c"Max", max),
        (c"Min", min),
        (c"Lerp", lerp),
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

unsafe extern "C-unwind" {
    fn lua_workhostnanos(state: *mut ffi::lua_State, nanos: c_int);
}

unsafe fn checked_lerp(state: *mut ffi::lua_State, dotted: bool) -> c_int {
    unsafe {
        let count = ffi::lua_gettop(state);
        if count < 1 {
            raise_script(state, "missing argument #1 to 'Lerp' (Vector3 expected)".into(), true);
        }
        if ffi::lua_type(state, 1) != ffi::LUA_TVECTOR {
            let message = format!("invalid argument #1 to 'Lerp' (Vector3 expected, got {})", type_name_at(state, 1));
            raise_script(state, message, true);
        }
        let to = if dotted { " to 'Lerp'" } else { "" };
        if count < 2 {
            raise_script(state, format!("missing argument #2{to} (Vector3 expected)"), true);
        }
        if ffi::lua_type(state, 2) != ffi::LUA_TVECTOR {
            let message = format!("invalid argument #2{to} (Vector3 expected, got {})", type_name_at(state, 2));
            raise_script(state, message, true);
        }
        if count < 3 {
            raise_script(state, format!("missing argument #3{to} (number expected)"), true);
        }
        let kind = ffi::lua_type(state, 3);
        let mut isnum: c_int = 0;
        let alpha = if kind == ffi::LUA_TNUMBER || kind == ffi::LUA_TSTRING {
            ffi::lua_tonumberx(state, 3, &mut isnum)
        } else {
            0.0
        };
        if isnum == 0 {
            let message = format!("invalid argument #3{to} (number expected, got {})", type_name_at(state, 3));
            raise_script(state, message, true);
        }
        // Priced to match the parity bench's 14.9 ns whole (11 ns call + 4 ns here)
        lua_workhostnanos(state, 4);
        let a = ffi::lua_tovector(state, 1);
        let b = ffi::lua_tovector(state, 2);
        let t = alpha as f32;
        let x = *a + (*b - *a) * t;
        let y = *a.add(1) + (*b.add(1) - *a.add(1)) * t;
        let z = *a.add(2) + (*b.add(2) - *a.add(2)) * t;
        ffi::lua_pushvector(state, x, y, z);
        1
    }
}

/// Checks the arguments of method `name` (self a Vector3, then `rest`), raising the client's error (naming the method
/// on `.` calls, omitting it after self on `:` calls), then runs library function `index` on this stack.
unsafe fn checked(state: *mut ffi::lua_State, name: &str, rest: Rest, index: usize, dotted: bool) -> c_int {
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
        let to = if dotted { format!(" to '{name}'") } else { String::new() };
        for argument in 2..=last {
            if argument > count {
                raise_script(state, format!("missing argument #{argument}{to} (Vector3 expected)"), true);
            }
            if ffi::lua_type(state, argument) != ffi::LUA_TVECTOR {
                let message = format!("invalid argument #{argument}{to} (Vector3 expected, got {})", type_name_at(state, argument));
                raise_script(state, message, true);
            }
        }
        let library = LIBRARY.get().expect("the vector library functions, read when the methods were made");
        (library[index])(state)
    }
}

/// `__namecall` for native Vector3: runs the 8 native methods directly in `:` mode (no ` to '<method>'` after self),
/// dispatches the remaining Luau methods (`Lerp`, `lerp`, `FuzzyEq`, `Angle`) from upvalue 1 (`extras`), and raises
/// `<name> is not a valid member of Vector3` for properties or unknown members.
pub fn namecall(lua: &Lua, extras: LuaTable) -> LuaResult<LuaFunction> {
    unsafe {
        lua.exec_raw::<LuaFunction>(extras, |state| {
            ffi::lua_pushcclosurek(state, vector_namecall, c"__namecall".as_ptr(), 1, None);
        })
    }
}

pub unsafe extern "C-unwind" fn vector_namecall(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let key = ffi::lua_namecallatom(state, std::ptr::null_mut());
        if key.is_null() {
            raise_script(state, "invalid namecall".into(), false);
        }
        let bytes = CStr::from_ptr(key).to_bytes();
        match bytes {
            b"Abs" => return checked(state, "Abs", Rest::None, ABS, false),
            b"Ceil" => return checked(state, "Ceil", Rest::None, CEIL, false),
            b"Floor" => return checked(state, "Floor", Rest::None, FLOOR, false),
            b"Sign" => return checked(state, "Sign", Rest::None, SIGN, false),
            b"Dot" => return checked(state, "Dot", Rest::One, DOT, false),
            b"Cross" => return checked(state, "Cross", Rest::One, CROSS, false),
            b"Max" => return checked(state, "Max", Rest::Any, MAX, false),
            b"Min" => return checked(state, "Min", Rest::Any, MIN, false),
            b"Lerp" | b"lerp" => return checked_lerp(state, false),
            _ => {}
        }
        ffi::lua_pushlstring(state, bytes.as_ptr().cast(), bytes.len());
        if ffi::lua_rawget(state, ffi::lua_upvalueindex(1)) == ffi::LUA_TFUNCTION {
            ffi::lua_insert(state, 1);
            let nargs = ffi::lua_gettop(state) - 1;
            let overhead = (5.77 + bytes.len() as f64 * 0.169).round() as c_int;
            lua_workhostnanos(state, -overhead);
            ffi::lua_call(state, nargs, ffi::LUA_MULTRET);
            return ffi::lua_gettop(state);
        }
        ffi::lua_pop(state, 1);
        let name = String::from_utf8_lossy(bytes).into_owned();
        let message = format!("{name} is not a valid member of Vector3");
        drop(name);
        raise_script(state, message, false)
    }
}

unsafe extern "C-unwind" fn lerp(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked_lerp(state, true) }
}

unsafe extern "C-unwind" fn abs(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Abs", Rest::None, ABS, true) }
}

unsafe extern "C-unwind" fn ceil(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Ceil", Rest::None, CEIL, true) }
}

unsafe extern "C-unwind" fn floor(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Floor", Rest::None, FLOOR, true) }
}

unsafe extern "C-unwind" fn sign(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Sign", Rest::None, SIGN, true) }
}

unsafe extern "C-unwind" fn dot(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Dot", Rest::One, DOT, true) }
}

unsafe extern "C-unwind" fn cross(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Cross", Rest::One, CROSS, true) }
}

unsafe extern "C-unwind" fn max(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Max", Rest::Any, MAX, true) }
}

unsafe extern "C-unwind" fn min(state: *mut ffi::lua_State) -> c_int {
    unsafe { checked(state, "Min", Rest::Any, MIN, true) }
}
