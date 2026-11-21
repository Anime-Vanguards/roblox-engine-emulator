//! Datatype errors as a script sees them in a Roblox client (measured in a playing client, Studio 0.747: ree's
//! tools/parity/probes/datatype_errors.luau): a plain string, after the calling line's position where the client's Luau
//! checks raise it (`invalid argument #2 (Vector2 expected, got number)`), alone where its reflection raises it
//! (`Unable to cast double to Vector2`, `Argument 3 missing or nil`) or a type's own code does, and never with a
//! traceback. mlua raises a Rust callback's error as a userdata holding the Rust error, whose text carries one, so a
//! callback returns [`script_error`]'s error instead, and the hook mlua-sys runs before `lua_error` ([`install`]) raises
//! the string in its place.

use std::{cell::RefCell, ffi::CStr};

use mlua::{ffi, prelude::*};

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

struct Pending {
    message: String,
    positioned: bool,
}

#[derive(Debug)]
struct ScriptError(String);

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScriptError {}

/// The error a datatype's callback returns to raise `message` as the script's own error: a plain string, after the
/// calling line's position when `positioned` (where a client's Luau checks raise it), else alone.
pub fn script_error(message: impl Into<String>, positioned: bool) -> LuaError {
    let message = message.into();
    PENDING.with(|pending| *pending.borrow_mut() = Some(Pending { message: message.clone(), positioned }));
    LuaError::external(ScriptError(message))
}

/// Installs the hook that raises [`script_error`]'s strings (once a process).
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| ffi::lua_seterrorhook(Some(error_hook)));
}

/// Run before every `lua_error` a Rust caller raises: when the value mlua raises holds the error [`script_error`] made
/// last (its text starts with the message), the message takes its place. A message an error left behind that never
/// reached Lua (Rust code handled it) matches no later error and is dropped.
unsafe extern "C-unwind" fn error_hook(state: *mut ffi::lua_State) {
    let Some(pending) = PENDING.with(|pending| pending.borrow_mut().take()) else { return };
    unsafe {
        if ffi::lua_type(state, -1) != ffi::LUA_TUSERDATA {
            return;
        }
        let mut length = 0;
        let text = ffi::luaL_tolstring(state, -1, &mut length);
        let text = std::slice::from_raw_parts(text.cast::<u8>(), length);
        let message = pending.message.as_bytes();
        let matches = text.starts_with(message) && (text.len() == message.len() || text[message.len()] == b'\n');
        ffi::lua_pop(state, 1);
        if !matches {
            return;
        }
        let position = if pending.positioned { caller_position(state) } else { None };
        let full = match position {
            Some(position) => format!("{position}{}", pending.message),
            None => pending.message,
        };
        ffi::lua_pushlstring(state, full.as_ptr().cast(), full.len());
    }
}

/// Whether a chunk is code a client has none of: mlua's own Luau (`__mlua_index`) and the host's (ree names its chunks
/// `rur.*` and `ree.*`), which stand in for what a client does in C++.
fn internal_chunk(source: &[u8]) -> bool {
    let name = source.strip_prefix(b"=").or_else(|| source.strip_prefix(b"@")).unwrap_or(source);
    name.starts_with(b"__mlua") || name.starts_with(b"rur.") || name.starts_with(b"ree.")
}

/// Where a client's Luau check says an error is (`luaL_where(L, 1)`): the calling function's `<chunk>:<line>: `, or
/// nothing when the caller is a C function (`pcall(Vector2.Abs, 5)`). Frames of [`internal_chunk`]s are looked past,
/// and so is a C function one of them called (the pcall of the host's library wrapper), as a client has neither.
///
/// # Safety
/// `state` must be the running thread, inside a C function (level 0).
pub unsafe fn caller_position(state: *mut ffi::lua_State) -> Option<String> {
    unsafe {
        let mut level = 1;
        while level < 64 {
            let mut frame: ffi::lua_Debug = std::mem::zeroed();
            if ffi::lua_getinfo(state, level, c"sl".as_ptr(), &mut frame) == 0 {
                return None;
            }
            let source = if frame.source.is_null() { &b""[..] } else { CStr::from_ptr(frame.source).to_bytes() };
            if internal_chunk(source) {
                level += 1;
                continue;
            }
            if frame.currentline < 0 {
                // a C function: one the host's own Luau called is part of that code
                let mut next: ffi::lua_Debug = std::mem::zeroed();
                if ffi::lua_getinfo(state, level + 1, c"s".as_ptr(), &mut next) != 0
                    && !next.source.is_null()
                    && internal_chunk(CStr::from_ptr(next.source).to_bytes())
                {
                    level += 2;
                    continue;
                }
                return None;
            }
            if frame.currentline == 0 {
                return None;
            }
            if frame.short_src.is_null() {
                return None;
            }
            let chunk = CStr::from_ptr(frame.short_src).to_string_lossy();
            return Some(format!("{chunk}:{}: ", frame.currentline));
        }
        None
    }
}

/// A value's type as a client's errors name it (its `typeof`): a userdata's `__type` (Vector2, CFrame, Instance),
/// Vector3 for a vector, else the basic type's name.
pub fn type_name(value: &LuaValue) -> String {
    match value {
        LuaValue::Nil => "nil".into(),
        LuaValue::Boolean(_) => "boolean".into(),
        LuaValue::Integer(_) | LuaValue::Number(_) => "number".into(),
        LuaValue::Vector(_) => "Vector3".into(),
        LuaValue::String(_) => "string".into(),
        LuaValue::Table(_) => "table".into(),
        LuaValue::Function(_) => "function".into(),
        LuaValue::Thread(_) => "thread".into(),
        LuaValue::UserData(ud) => match ud.type_name() {
            Ok(name) => name.to_string_lossy(),
            // (a userdata mlua did not make, as the host's Instances: its __type read directly)
            Err(_) => foreign_type_name(ud).unwrap_or_else(|| "userdata".into()),
        },
        LuaValue::Buffer(_) => "buffer".into(),
        _ => "userdata".into(),
    }
}

/// The `__type` of a userdata mlua did not make (none: it has none, as a newproxy has none).
fn foreign_type_name(ud: &LuaAnyUserData) -> Option<String> {
    let lua = ud.weak_lua().try_upgrade()?;
    unsafe {
        lua.exec_raw::<Option<String>>(ud, |state| {
            if ffi::luaL_getmetafield(state, -1, c"__type".as_ptr()) == ffi::LUA_TSTRING {
                ffi::lua_remove(state, -2);
            } else {
                ffi::lua_settop(state, 0);
                ffi::lua_pushnil(state);
            }
        })
        .ok()
        .flatten()
    }
}

/// A value's type as a client's reflection names it in `Unable to cast <type> to <type>`: double, string, bool,
/// Array or Dictionary, Function, CoordinateFrame for a CFrame, an EnumItem's enum (Material), else its `typeof`.
pub fn cast_name(value: &LuaValue) -> String {
    match value {
        LuaValue::Integer(_) | LuaValue::Number(_) => "double".into(),
        LuaValue::Boolean(_) => "bool".into(),
        LuaValue::Table(table) => if table.raw_len() > 0 || table.is_empty() { "Array" } else { "Dictionary" }.into(),
        LuaValue::Function(_) => "Function".into(),
        LuaValue::UserData(ud) => match type_name(value).as_str() {
            "CFrame" => "CoordinateFrame".into(),
            "EnumItem" => ud
                .get::<LuaValue>("EnumType")
                .ok()
                .and_then(|kind| match kind {
                    LuaValue::UserData(kind) => kind.to_string().ok(),
                    _ => None,
                })
                .unwrap_or_else(|| "EnumItem".into()),
            name => name.into(),
        },
        other => type_name(other),
    }
}

/// `invalid argument #<index>[ to '<function>'] (<expected> expected, got <type>)`, from the calling line: a client
/// names the function in a `.` call and not in a `:` call (a method takes the `:` form for arguments after self, the
/// call a script makes; self itself is wrong only in a `.` call).
pub fn invalid_argument(index: usize, function: Option<&str>, expected: &str, value: &LuaValue) -> LuaError {
    let to = function.map(|name| format!(" to '{name}'")).unwrap_or_default();
    script_error(format!("invalid argument #{index}{to} ({expected} expected, got {})", type_name(value)), true)
}

/// `missing argument #<index>[ to '<function>'] (<expected> expected)`, from the calling line: no value at all was
/// passed there (an explicit nil is an invalid argument).
pub fn missing_argument(index: usize, function: Option<&str>, expected: &str) -> LuaError {
    let to = function.map(|name| format!(" to '{name}'")).unwrap_or_default();
    script_error(format!("missing argument #{index}{to} ({expected} expected)"), true)
}

/// The reflection's `Argument <index> missing or nil`, without a position.
pub fn argument_missing(index: usize) -> LuaError {
    script_error(format!("Argument {index} missing or nil"), false)
}

/// The reflection's `Unable to cast <value's type> to <to>`, without a position.
pub fn unable_to_cast(value: &LuaValue, to: &str) -> LuaError {
    script_error(format!("Unable to cast {} to {to}", cast_name(value)), false)
}

/// A number as a client's Luau checks read one (`luaL_checknumber`): a number, or a string Luau reads as one.
pub fn luau_number(lua: &Lua, value: &LuaValue) -> Option<f64> {
    match value {
        LuaValue::Number(n) => Some(*n),
        LuaValue::Integer(n) => Some(*n as f64),
        LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten(),
        _ => None,
    }
}

/// A number as a client's reflection casts one to a float: a number, a string Luau reads as one (hex and spaces
/// included: `"0x1"`, `" 0.5 "`), or a boolean (1 or 0), measured with Lerp's alpha.
pub fn reflected_float(lua: &Lua, value: &LuaValue) -> Option<f64> {
    match value {
        LuaValue::Boolean(on) => Some(if *on { 1.0 } else { 0.0 }),
        other => luau_number(lua, other),
    }
}

/// The value at `index` (0-based) of a call's arguments, None past the last one passed.
pub fn arg(args: &LuaMultiValue, index: usize) -> Option<&LuaValue> {
    args.get(index)
}

/// The userdata of type `T` a value is, if it is one.
pub fn userdata_of<T: LuaUserData + 'static>(value: Option<&LuaValue>) -> Option<LuaUserDataRef<T>> {
    match value {
        Some(LuaValue::UserData(ud)) => ud.borrow::<T>().ok(),
        _ => None,
    }
}

/// Self of a method a client checks in Luau, named as a `.` call names it (only a `.` call can pass a wrong self):
/// `missing argument #1 to '<method>' (<T> expected)` with no value, `invalid argument #1 to '<method>' (<T> expected,
/// got <type>)` with another.
pub fn luau_self<T: LuaUserData + 'static>(args: &LuaMultiValue, method: &str, type_name: &str) -> LuaResult<LuaUserDataRef<T>> {
    match args.front() {
        None => Err(missing_argument(1, Some(method), type_name)),
        Some(value) => userdata_of::<T>(Some(value)).ok_or_else(|| invalid_argument(1, Some(method), type_name, value)),
    }
}

/// Argument `index` (1-based, self being 1) of a method a client checks in Luau, as a `:` call reports it (no name).
pub fn luau_arg<T: LuaUserData + 'static>(args: &LuaMultiValue, index: usize, type_name: &str) -> LuaResult<LuaUserDataRef<T>> {
    match args.get(index - 1) {
        None => Err(missing_argument(index, None, type_name)),
        Some(value) => userdata_of::<T>(Some(value)).ok_or_else(|| invalid_argument(index, None, type_name, value)),
    }
}

/// Argument `index` (1-based) a client's reflection casts to `to`: `Argument <index> missing or nil`, `Unable to cast
/// <type> to <to>`.
pub fn reflected_arg<T: LuaUserData + 'static>(args: &LuaMultiValue, index: usize, to: &str) -> LuaResult<LuaUserDataRef<T>> {
    match args.get(index - 1) {
        None | Some(LuaValue::Nil) => Err(argument_missing(index)),
        Some(value) => userdata_of::<T>(Some(value)).ok_or_else(|| unable_to_cast(value, to)),
    }
}

/// Argument `index` (1-based) a client's reflection casts to a float ([`reflected_float`]).
pub fn reflected_float_arg(lua: &Lua, args: &LuaMultiValue, index: usize) -> LuaResult<f64> {
    match args.get(index - 1) {
        None | Some(LuaValue::Nil) => Err(argument_missing(index)),
        Some(value) => reflected_float(lua, value).ok_or_else(|| unable_to_cast(value, "float")),
    }
}

/// The two operands of a type's + or -, whichever side the value that runs it is on: a client's Luau checks raise the
/// first that is not of the type, from the calling line (`invalid argument #1 (UDim expected, got number)`).
pub fn same_operands<T: LuaUserData + Copy + 'static>(lhs: &LuaValue, rhs: &LuaValue, type_name: &str) -> LuaResult<(T, T)> {
    let a = *userdata_of::<T>(Some(lhs)).ok_or_else(|| invalid_argument(1, None, type_name, lhs))?;
    let b = *userdata_of::<T>(Some(rhs)).ok_or_else(|| invalid_argument(2, None, type_name, rhs))?;
    Ok((a, b))
}

/// Argument `index` (1-based) of a constructor a client checks in Luau, naming it: none when not passed (the caller's
/// default), else a value of type `T` (nil passed is none of it).
pub fn optional_ctor_arg<T: LuaUserData + Copy + 'static>(args: &LuaMultiValue, index: usize, name: &str, type_name: &str) -> LuaResult<Option<T>> {
    match args.get(index - 1) {
        None => Ok(None),
        Some(value) => userdata_of::<T>(Some(value)).map(|v| Some(*v)).ok_or_else(|| invalid_argument(index, Some(name), type_name, value)),
    }
}

/// A number argument of a constructor a client checks in Luau, naming it: `missing argument #<index> to '<name>' (number
/// expected)`, `invalid argument #<index> to '<name>' (number expected, got <type>)`.
pub fn ctor_number(lua: &Lua, args: &LuaMultiValue, index: usize, name: &str) -> LuaResult<f64> {
    match args.get(index - 1) {
        None => Err(missing_argument(index, Some(name), "number")),
        Some(value) => luau_number(lua, value).ok_or_else(|| invalid_argument(index, Some(name), "number", value)),
    }
}

/// `attempt to <verb> a <T> with an incompatible value type or nil`, without a position: what a client's `*`, `/` and
/// `//` of a Vector2 raise for an operand they take no number from.
pub fn incompatible(verb: &str, type_name: &str) -> LuaError {
    script_error(format!("attempt to {verb} a {type_name} with an incompatible value type or nil"), false)
}

/// [`type_name`] of the value at `index` of a C function's stack.
///
/// # Safety
/// `state` must be a live thread and `index` a valid stack index.
pub unsafe fn type_name_at(state: *mut ffi::lua_State, index: std::ffi::c_int) -> String {
    unsafe {
        let kind = ffi::lua_type(state, index);
        if kind == ffi::LUA_TVECTOR {
            return "Vector3".into();
        }
        if kind == ffi::LUA_TUSERDATA && ffi::luaL_getmetafield(state, index, c"__type".as_ptr()) != 0 {
            let name = if ffi::lua_type(state, -1) == ffi::LUA_TSTRING {
                let mut length = 0;
                let text = ffi::lua_tolstring(state, -1, &mut length);
                Some(String::from_utf8_lossy(std::slice::from_raw_parts(text.cast::<u8>(), length)).into_owned())
            } else {
                None
            };
            ffi::lua_pop(state, 1);
            if let Some(name) = name {
                return name;
            }
        }
        CStr::from_ptr(ffi::lua_typename(state, kind)).to_string_lossy().into_owned()
    }
}

/// Raises `message` from a C function as the script's plain string error, after [`caller_position`] when `positioned`.
///
/// # Safety
/// `state` must be the running thread, inside a C function. Nothing the caller owns is dropped: pass owned values only.
pub unsafe fn raise_script(state: *mut ffi::lua_State, message: String, positioned: bool) -> ! {
    unsafe {
        let full = match if positioned { caller_position(state) } else { None } {
            Some(position) => format!("{position}{message}"),
            None => message,
        };
        ffi::lua_pushlstring(state, full.as_ptr().cast(), full.len());
        // (the strings are dropped before the error unwinds past this frame)
        drop(full);
        ffi::lua_error(state)
    }
}
