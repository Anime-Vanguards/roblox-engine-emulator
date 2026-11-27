//! Datatype errors as a script sees them in a Roblox client (measured in a playing client, Studio 0.747: ree's
//! tools/parity/probes/datatype_errors.luau): a plain string, after the calling line's position where the client's Luau
//! checks raise it (`invalid argument #2 (Vector2 expected, got number)`), alone where its reflection raises it
//! (`Unable to cast double to Vector2`, `Argument 3 missing or nil`) or a type's own code does, and never with a
//! traceback. mlua raises a Rust callback's error as a userdata holding the Rust error, whose text carries one, so a
//! callback returns [`script_error`]'s error instead, and the hook mlua-sys runs before `lua_error` ([`install`]) raises
//! the string in its place.

use std::{cell::{Cell, RefCell}, ffi::CStr, rc::Rc};

use mlua::{ffi, prelude::*};

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static IN_NAMECALL: Cell<u32> = const { Cell::new(0) };
}

/// Whether the running datatype method was invoked via `__namecall` (`:`) rather than `__index` (`.`).
pub fn is_namecall() -> bool {
    IN_NAMECALL.with(|depth| depth.get() > 0)
}

/// Whether the function at a level of the running thread's stack is a host's engine function: a C function standing for the
/// client's own C function a script called, which the host does its work under ([`set_engine_frame`]).
pub type EngineFrame = unsafe fn(*mut ffi::lua_State, std::ffi::c_int) -> bool;

static ENGINE_FRAME: std::sync::OnceLock<EngineFrame> = std::sync::OnceLock::new();

/// Gives [`caller_position`] the host's test for its engine functions' frames, which it looks through (once a process).
pub fn set_engine_frame(test: EngineFrame) {
    let _ = ENGINE_FRAME.set(test);
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
/// and so is a C function one of them called (the pcall of the host's library wrapper), as a client has neither, and a host's
/// engine function ([`set_engine_frame`]): the client's C function itself, whose caller's position the error is.
///
/// # Safety
/// `state` must be the running thread, inside a C function (level 0).
pub unsafe fn caller_position(state: *mut ffi::lua_State) -> Option<String> {
    unsafe {
        let mut level = 1;
        let mut skip_namecall = is_namecall();
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
                if ENGINE_FRAME.get().is_some_and(|test| test(state, level)) {
                    level += 1;
                    continue;
                }
                if skip_namecall {
                    skip_namecall = false;
                    level += 1;
                    continue;
                }
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

/// The name of a userdata mlua did not make, as typeof gives it: its `__type`, else the name the host registered for it
/// (ree's prelude proxies, Axes or RaycastParams: [`host_type_name`]).
fn foreign_type_name(ud: &LuaAnyUserData) -> Option<String> {
    let lua = ud.weak_lua().try_upgrade()?;
    unsafe {
        lua.exec_raw::<Option<String>>(ud, |state| {
            let name = type_name_at(state, -1);
            ffi::lua_settop(state, 0);
            ffi::lua_pushlstring(state, name.as_ptr().cast(), name.len());
        })
        .ok()
        .flatten()
    }
}

/// The name the builtin typeof gives a table or a userdata without `__type`: the host's own name for it when it registered
/// one (lua_settypeofnames: ree's prelude proxies, Axes, RaycastParams, a SharedTable), else the basic type's.
///
/// # Safety
/// `state` must be a live thread and `index` a valid stack index.
unsafe fn host_type_name(state: *mut ffi::lua_State, index: std::ffi::c_int) -> Option<String> {
    unsafe {
        let index = ffi::lua_absindex(state, index);
        ffi::lua_getglobal(state, c"typeof".as_ptr());
        ffi::lua_pushvalue(state, index);
        let name = if ffi::lua_pcall(state, 1, 1, 0) == ffi::LUA_OK && ffi::lua_type(state, -1) == ffi::LUA_TSTRING {
            let mut length = 0;
            let text = ffi::lua_tolstring(state, -1, &mut length);
            Some(String::from_utf8_lossy(std::slice::from_raw_parts(text.cast::<u8>(), length)).into_owned())
        } else {
            None
        };
        ffi::lua_pop(state, 1);
        name
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

/// Argument `index` (1-based, self being 1) of a method a client checks in Luau: names `method` on a `.` call and
/// omits it on a `:` call.
pub fn luau_arg_named<T: LuaUserData + 'static>(
    args: &LuaMultiValue,
    index: usize,
    method: &str,
    type_name: &str,
) -> LuaResult<LuaUserDataRef<T>> {
    let function = (!is_namecall()).then_some(method);
    match args.get(index - 1) {
        None => Err(missing_argument(index, function, type_name)),
        Some(value) => userdata_of::<T>(Some(value)).ok_or_else(|| invalid_argument(index, function, type_name, value)),
    }
}

/// Invalid argument `index` (1-based) of a method: names `method` on a `.` call and omits it on a `:` call.
pub fn invalid_method_arg(index: usize, method: &str, expected: &str, value: &LuaValue) -> LuaError {
    invalid_argument(index, (!is_namecall()).then_some(method), expected, value)
}

/// Missing argument `index` (1-based) of a method: names `method` on a `.` call and omits it on a `:` call.
pub fn missing_method_arg(index: usize, method: &str, expected: &str) -> LuaError {
    missing_argument(index, (!is_namecall()).then_some(method), expected)
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

/// What a datatype does with a member it lacks, as a playing client does (Studio 0.747, ree's
/// tools/parity/probes/datatype_errors.luau): raises `Nope is not a valid member of Vector2` for a string key, without a
/// position, in a type with methods (Vector2, CFrame, UDim2, Color3, Region3, Ray), and `'Nope' is not a valid member of
/// UDim`, the key quoted and a number key too (`'1'`), in a type without (UDim, Rect, Vector3int16, NumberRange, ...);
/// any other key (a number in the first kind) is `invalid argument #2 (string expected, got number)` from the calling
/// line. It is the type's `__index` meta field, which mlua's own `__index` reaches only once the type's fields and
/// methods have no such key: for a type with methods a table (mlua puts the methods in it) whose own `__index` raises,
/// for one without a function, so a member that exists is read through the same Luau as before.
pub struct UnknownMembers {
    type_name: &'static str,
    methods: bool,
}

impl UnknownMembers {
    /// A type with methods (or fields of a fixed value)
    pub fn with_methods(type_name: &'static str) -> Self {
        Self { type_name, methods: true }
    }

    /// A type with field getters only
    pub fn without_methods(type_name: &'static str) -> Self {
        Self { type_name, methods: false }
    }
}

impl IntoLua for UnknownMembers {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let function = if self.methods { unknown_member as ffi::lua_CFunction } else { unknown_member_quoted };
        let raise = unsafe {
            lua.exec_raw::<LuaFunction>(self.type_name, |state| {
                ffi::lua_pushcclosurek(state, function, c"__index".as_ptr(), 1, None);
            })?
        };
        if !self.methods {
            return Ok(LuaValue::Function(raise));
        }
        let methods = lua.create_table()?;
        let meta = lua.create_table()?;
        meta.raw_set("__index", raise)?;
        methods.set_metatable(Some(meta))?;
        Ok(LuaValue::Table(methods))
    }
}

/// Pairs a datatype's `__index` method table (which mlua populates in-place with every registered method, while keeping
/// property getters separate) with a `__namecall` dispatcher over that same table. A `:` call only resolves methods (so
/// `V2:X()` raises `X is not a valid member of Vector2`) and marks [`is_namecall`] for the call's duration so Luau-checked
/// argument errors after self omit ` to '<method>'` on `:` calls and include it on `.` calls.
#[derive(Clone)]
pub struct MethodMembers {
    type_name: &'static str,
    table: Rc<RefCell<Option<LuaTable>>>,
}

impl MethodMembers {
    pub fn new(type_name: &'static str) -> Self {
        Self {
            type_name,
            table: Rc::new(RefCell::new(None)),
        }
    }

    pub fn index(&self) -> MethodIndex {
        MethodIndex(self.clone())
    }

    pub fn namecall(&self) -> MethodNamecall {
        MethodNamecall(self.clone())
    }

    fn methods_table(&self, lua: &Lua) -> LuaResult<LuaTable> {
        if let Some(existing) = self.table.borrow().as_ref() {
            return Ok(existing.clone());
        }
        let raise = unsafe {
            lua.exec_raw::<LuaFunction>(self.type_name, |state| {
                ffi::lua_pushcclosurek(state, unknown_member, c"__index".as_ptr(), 1, None);
            })?
        };
        let methods = lua.create_table()?;
        let meta = lua.create_table()?;
        meta.raw_set("__index", raise)?;
        methods.set_metatable(Some(meta))?;
        *self.table.borrow_mut() = Some(methods.clone());
        Ok(methods)
    }
}

pub struct MethodIndex(MethodMembers);

impl IntoLua for MethodIndex {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        Ok(LuaValue::Table(self.0.methods_table(lua)?))
    }
}

pub struct MethodNamecall(MethodMembers);

impl IntoLua for MethodNamecall {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let methods = self.0.methods_table(lua)?;
        let function = unsafe {
            lua.exec_raw::<LuaFunction>((methods, self.0.type_name), |state| {
                ffi::lua_pushcclosurek(state, datatype_namecall, c"__namecall".as_ptr(), 2, None);
            })?
        };
        Ok(LuaValue::Function(function))
    }
}

pub unsafe extern "C-unwind" fn datatype_namecall(state: *mut ffi::lua_State) -> std::ffi::c_int {
    unsafe {
        let key = ffi::lua_namecallatom(state, std::ptr::null_mut());
        if key.is_null() {
            raise_script(state, "invalid namecall".into(), false);
        }
        let bytes = CStr::from_ptr(key).to_bytes();
        ffi::lua_pushlstring(state, bytes.as_ptr().cast(), bytes.len());
        if ffi::lua_rawget(state, ffi::lua_upvalueindex(1)) == ffi::LUA_TFUNCTION {
            ffi::lua_insert(state, 1);
            let nargs = ffi::lua_gettop(state) - 1;
            IN_NAMECALL.with(|depth| depth.set(depth.get() + 1));
            let status = ffi::lua_pcall(state, nargs, ffi::LUA_MULTRET, 0);
            IN_NAMECALL.with(|depth| depth.set(depth.get() - 1));
            if status == ffi::LUA_OK {
                return ffi::lua_gettop(state);
            }
            ffi::lua_error(state);
        }
        ffi::lua_pop(state, 1);
        let key_name = String::from_utf8_lossy(bytes).into_owned();
        let mut length = 0;
        let type_ptr = ffi::lua_tolstring(state, ffi::lua_upvalueindex(2), &mut length);
        let type_name = String::from_utf8_lossy(std::slice::from_raw_parts(type_ptr.cast::<u8>(), length)).into_owned();
        let message = format!("{key_name} is not a valid member of {type_name}");
        charge_missed_member(state);
        drop(key_name);
        drop(type_name);
        raise_script(state, message, false)
    }
}

/// What an assignment to a datatype raises, as a playing client raises it (Studio 0.747, ree's
/// tools/parity/probes/datatype_errors.luau): `X member of Vector2 is read-only and cannot be assigned to` for a property
/// (a member whose value is no function), `Nope is not a valid member of UDim` for anything else (a method, a key the type
/// lacks, a number key: `1 is not a valid member of Vector2`), unquoted on every type, without a position; a key of
/// another type is `invalid argument #2 (string expected, got boolean)` from the calling line. It is the type's
/// `__newindex` meta field (Lune's datatypes have no field setters, so mlua makes no `__newindex` of its own).
pub struct AssignMembers(pub &'static str);

impl IntoLua for AssignMembers {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let function = unsafe {
            lua.exec_raw::<LuaFunction>(self.0, |state| {
                ffi::lua_pushcclosurek(state, assign_member, c"__newindex".as_ptr(), 1, None);
            })?
        };
        Ok(LuaValue::Function(function))
    }
}

/// Reads `value[key]` (the value and the key on the stack), for a protected call.
unsafe extern "C-unwind" fn read_member(state: *mut ffi::lua_State) -> std::ffi::c_int {
    unsafe {
        ffi::lua_gettable(state, 1);
        1
    }
}

/// `__newindex` of a datatype (the value, the key, the new value)
unsafe extern "C-unwind" fn assign_member(state: *mut ffi::lua_State) -> std::ffi::c_int {
    unsafe {
        let Some(key) = member_key(state, true) else {
            let message = format!("invalid argument #2 (string expected, got {})", type_name_at(state, 2));
            raise_script(state, message, true)
        };
        // (a property reads as a value that is no function; reading a method gives its function, a key the type lacks raises)
        let property = ffi::lua_type(state, 2) == ffi::LUA_TSTRING && {
            ffi::lua_pushcfunction(state, read_member);
            ffi::lua_pushvalue(state, 1);
            ffi::lua_pushvalue(state, 2);
            let read = ffi::lua_pcall(state, 2, 1, 0) == ffi::LUA_OK && ffi::lua_type(state, -1) != ffi::LUA_TFUNCTION;
            ffi::lua_pop(state, 1);
            read
        };
        let name = upvalue_name(state);
        charge_missed_member(state);
        let message = if property {
            format!("{key} member of {name} is read-only and cannot be assigned to")
        } else {
            format!("{key} is not a valid member of {name}")
        };
        drop(name);
        drop(key);
        raise_script(state, message, false)
    }
}

/// The key at 2 as an unknown member's message names it (a string, or a number in its Luau form when `numbers`), or
/// None for a key of another type.
unsafe fn member_key(state: *mut ffi::lua_State, numbers: bool) -> Option<String> {
    unsafe {
        let kind = ffi::lua_type(state, 2);
        if kind != ffi::LUA_TSTRING && !(numbers && kind == ffi::LUA_TNUMBER) {
            return None;
        }
        ffi::lua_pushvalue(state, 2);
        let mut length = 0;
        let text = ffi::lua_tolstring(state, -1, &mut length);
        let key = String::from_utf8_lossy(std::slice::from_raw_parts(text.cast::<u8>(), length)).into_owned();
        ffi::lua_pop(state, 1);
        Some(key)
    }
}

/// The type name upvalue of an [`UnknownMembers`] function.
unsafe fn upvalue_name(state: *mut ffi::lua_State) -> String {
    unsafe {
        let mut length = 0;
        let text = ffi::lua_tolstring(state, ffi::lua_upvalueindex(1), &mut length);
        String::from_utf8_lossy(std::slice::from_raw_parts(text.cast::<u8>(), length)).into_owned()
    }
}

unsafe extern "C-unwind" {
    fn lua_workhostnanos(state: *mut ffi::lua_State, nanos: std::ffi::c_int);
}

/// What a playing client spends on a datatype's missing member beyond the error the VM counts for its throw (ree's cost.rs
/// MEMBER_MISSED, 2.75 us, less SCRIPT_ERROR, 2.1), in nanoseconds: a read, a `:` call or a write of a member the type lacks
/// (Studio 0.747, a headless client and a server, two sessions each, beside FindFirstChild(nil) in the same sample: CFrame.Nope
/// 2.78-2.84 us, UDim2 2.80-2.91, Color3 2.69-2.86, `CFrame.X = 1` 2.84-2.94, where ree's 2.14 was the throw alone), charged to the
/// script whose line reads it.
const MEMBER_MISSED_NANOS: std::ffi::c_int = 650;

/// Charges [`MEMBER_MISSED_NANOS`] before a missing member's error is raised.
unsafe fn charge_missed_member(state: *mut ffi::lua_State) {
    unsafe { lua_workhostnanos(state, MEMBER_MISSED_NANOS) }
}

/// `__index` of a type with methods' method table (the table, the key)
unsafe extern "C-unwind" fn unknown_member(state: *mut ffi::lua_State) -> std::ffi::c_int {
    unsafe {
        let message = match member_key(state, false) {
            Some(key) => {
                charge_missed_member(state);
                format!("{key} is not a valid member of {}", upvalue_name(state))
            }
            None => {
                let message = format!("invalid argument #2 (string expected, got {})", type_name_at(state, 2));
                raise_script(state, message, true)
            }
        };
        raise_script(state, message, false)
    }
}

/// `__index` fallback of a type without methods (the value, the key)
unsafe extern "C-unwind" fn unknown_member_quoted(state: *mut ffi::lua_State) -> std::ffi::c_int {
    unsafe {
        let message = match member_key(state, true) {
            Some(key) => {
                charge_missed_member(state);
                format!("'{key}' is not a valid member of {}", upvalue_name(state))
            }
            None => {
                let message = format!("invalid argument #2 (string expected, got {})", type_name_at(state, 2));
                raise_script(state, message, true)
            }
        };
        raise_script(state, message, false)
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
        if kind == ffi::LUA_TUSERDATA || kind == ffi::LUA_TTABLE {
            if let Some(name) = host_type_name(state, index) {
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
