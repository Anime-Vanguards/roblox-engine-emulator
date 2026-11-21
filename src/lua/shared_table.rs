//! SharedTable: Roblox's table for sharing data between Actors. It is a userdata here as in Roblox (`type` is "userdata",
//! `typeof` "SharedTable"), so the table functions refuse it and the VM names it in its errors. Measured in Studio 0.748, a
//! playing client:
//!
//! - Keys are strings, or integers from 0 to 2^32 - 1 (a float of an integer value is that integer; "1" and 1 are two keys).
//!   Any other key raises at the script's line, on a read as on a write: "SharedTable keys can only be strings or numbers."
//!   for a key of another type, "Numeric SharedTable keys must be nonnegative integers less than 2^32" for another number.
//! - Values: booleans, numbers, strings, vectors and Roblox's value types are kept as they are ([`KEPT`]), a buffer as a copy
//!   (and a new copy on every read), a SharedTable as itself. A table becomes a new SharedTable (each place it appears, its
//!   tables too, its metatable ignored; a cycle raises "Cannot convert a cyclic table to a SharedTable"). Instances,
//!   functions and the engine's other objects raise "<Name>s can not be stored in a SharedTable"; threads, proxies, Enum,
//!   Enums and RotationCurveKeys are dropped (the key reads nil). nil removes the key.
//! - A write checks the key, then whether the table is frozen ("SharedTable is frozen"), then the value. Only the key's errors
//!   and the library's argument errors carry the script's position.
//! - `for k, v in S` walks the entries as they were when the loop started: the integer keys ascending, then the string keys in
//!   the order of Roblox's hash trie ([`string_order`]).
//! - `#`, `next`, `pairs`, `rawget`, `setmetatable` and the `table` library raise as on any userdata, the metatable is
//!   locked, `S:size()` raises "attempt to call missing method 'size' of SharedTable", and tostring is "SharedTable: 0x" and
//!   16 hex digits.
//! - The library (a read-only table): `new([table])`, `clone(S [, deep])` and `cloneAndFreeze(S [, deep])` (a shallow copy
//!   shares the SharedTables it holds, a deep one copies them, frozen too for cloneAndFreeze; a clone of a frozen table is not
//!   frozen), `isFrozen`, `size`, `clear`, `increment(S, key, delta)` (returns the old value) and `update(S, key, callback)`
//!   (the callback gets the old value and returns the new one; it runs where it cannot yield, and an error it raises reaches
//!   the caller as raised). Their messages are below, as measured.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::rc::Rc;

use mlua::prelude::*;
use rustc_hash::FxHashMap;

use super::{Args, Flow, lock_metatable, none, raise, raise_at_caller, raw, typeof_value, values, wrapped};
use crate::Engine;

/// A SharedTable: its entries, and whether it is frozen. The userdata is its identity (a SharedTable stored in another is the
/// same object).
pub struct LSharedTable(RefCell<Contents>);

#[derive(Clone, Default)]
struct Contents {
    frozen: bool,
    numbers: BTreeMap<u32, Stored>,
    strings: FxHashMap<Box<[u8]>, Stored>,
}

/// A value as a SharedTable holds it.
#[derive(Clone)]
enum Stored {
    Value(LuaValue),
    /// A buffer's bytes: a copy goes in, and every read makes a new buffer of them
    Buffer(Rc<[u8]>),
}

#[derive(Clone)]
enum Key {
    Number(u32),
    String(Box<[u8]>),
}

impl Contents {
    fn get(&self, key: &Key) -> Option<&Stored> {
        match key {
            Key::Number(n) => self.numbers.get(n),
            Key::String(s) => self.strings.get(s),
        }
    }

    fn set(&mut self, key: Key, value: Option<Stored>) {
        match (key, value) {
            (Key::Number(n), Some(value)) => {
                self.numbers.insert(n, value);
            }
            (Key::String(s), Some(value)) => {
                self.strings.insert(s, value);
            }
            (Key::Number(n), None) => {
                self.numbers.remove(&n);
            }
            (Key::String(s), None) => {
                self.strings.remove(&s);
            }
        }
    }

    fn len(&self) -> usize {
        self.numbers.len() + self.strings.len()
    }

    /// The entries in the order a loop walks them.
    fn ordered(&self) -> Vec<(Key, Stored)> {
        let mut entries: Vec<(Key, Stored)> = self.numbers.iter().map(|(n, value)| (Key::Number(*n), value.clone())).collect();
        for key in string_order(self.strings.keys().map(|key| &**key).collect()) {
            entries.push((Key::String(key.into()), self.strings[key].clone()));
        }
        entries
    }
}

const KEY_TYPE: &str = "SharedTable keys can only be strings or numbers.";
const KEY_NUMBER: &str = "Numeric SharedTable keys must be nonnegative integers less than 2^32";
const FROZEN: &str = "SharedTable is frozen";
const CYCLIC: &str = "Cannot convert a cyclic table to a SharedTable";

/// The value types a SharedTable keeps as they are (measured, each read back as itself). Native vectors, booleans, numbers and
/// strings are kept too.
const KEPT: &[&str] = &[
    "Axes", "BrickColor", "CFrame", "Color3", "ColorSequence", "ColorSequenceKeypoint", "EnumItem", "Faces", "Font", "NumberRange",
    "NumberSequence", "NumberSequenceKeypoint", "Path2DControlPoint", "PhysicalProperties", "Ray", "Rect", "Region3", "Region3int16",
    "TweenInfo", "UDim", "UDim2", "Vector2", "Vector2int16", "Vector3int16",
];

/// The value types a SharedTable drops without a word (measured: the key reads nil after the write).
const DROPPED: &[&str] = &["Enum", "Enums", "RotationCurveKey", "userdata"];

fn key_of(value: &LuaValue) -> Result<Key, Flow> {
    let number = match value {
        LuaValue::String(s) => return Ok(Key::String(s.as_bytes().to_vec().into_boxed_slice())),
        LuaValue::Integer(n) => *n as f64,
        LuaValue::Number(n) => *n,
        _ => return raise_at_caller(KEY_TYPE),
    };
    if (0.0..4294967296.0).contains(&number) && number.fract() == 0.0 { Ok(Key::Number(number as u32)) } else { raise_at_caller(KEY_NUMBER) }
}

fn key_value(lua: &Lua, key: &Key) -> LuaResult<LuaValue> {
    Ok(match key {
        Key::Number(n) => LuaValue::Number(f64::from(*n)),
        Key::String(s) => LuaValue::String(lua.create_string(s)?),
    })
}

/// A key as the increment messages quote it.
fn key_text(key: &Key) -> String {
    match key {
        Key::Number(n) => n.to_string(),
        Key::String(s) => String::from_utf8_lossy(s).into_owned(),
    }
}

fn refused<T>(name: &str) -> Result<T, Flow> {
    raise(format!("{name}s can not be stored in a SharedTable"))
}

/// What a SharedTable holds for `value` (None: no entry). `open` holds the tables being converted, to find a cycle.
fn stored(lua: &Lua, engine: &Engine, value: &LuaValue, open: &mut Vec<*const c_void>) -> Result<Option<Stored>, Flow> {
    Ok(match value {
        LuaValue::Nil | LuaValue::Thread(_) | LuaValue::LightUserData(_) | LuaValue::Error(_) | LuaValue::Other(_) => None,
        LuaValue::Boolean(_) | LuaValue::Integer(_) | LuaValue::Number(_) | LuaValue::String(_) | LuaValue::Vector(_) => Some(Stored::Value(value.clone())),
        LuaValue::Buffer(buffer) => Some(Stored::Buffer(buffer.to_vec().into())),
        LuaValue::Function(_) => return refused("Function"),
        LuaValue::Table(table) => Some(Stored::Value(LuaValue::UserData(converted(lua, engine, table, open)?))),
        LuaValue::UserData(ud) => {
            if ud.is::<LSharedTable>() {
                return Ok(Some(Stored::Value(value.clone())));
            }
            if raw::instance_of_ud(ud).is_some() {
                return refused("Instance");
            }
            let name = typeof_value(engine, value);
            match name.as_str() {
                name if KEPT.contains(&name) => Some(Stored::Value(value.clone())),
                name if DROPPED.contains(&name) => None,
                "RBXScriptSignal" => return refused("EventInstance"),
                "RBXScriptConnection" => return refused("Connection"),
                other => return refused(other),
            }
        }
    })
}

/// A table as a new SharedTable (its own tables converted too).
fn converted(lua: &Lua, engine: &Engine, table: &LuaTable, open: &mut Vec<*const c_void>) -> Result<LuaAnyUserData, Flow> {
    let pointer = table.to_pointer();
    if open.contains(&pointer) {
        return raise(CYCLIC);
    }
    open.push(pointer);
    let mut contents = Contents::default();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        let key = key_of(&key)?;
        let value = stored(lua, engine, &value, open)?;
        contents.set(key, value);
    }
    open.pop();
    Ok(make(lua, contents)?)
}

fn make(lua: &Lua, contents: Contents) -> LuaResult<LuaAnyUserData> {
    lua.create_any_userdata(LSharedTable(RefCell::new(contents)))
}

fn load(lua: &Lua, stored: &Stored) -> LuaResult<LuaValue> {
    Ok(match stored {
        Stored::Value(value) => value.clone(),
        Stored::Buffer(bytes) => LuaValue::Buffer(lua.create_buffer(&**bytes)?),
    })
}

/// A copy of `table`, frozen or not; a deep one copies the SharedTables it holds as well (`copies`: those copied so far, so a
/// table held twice, or holding itself, is copied once).
fn copy(lua: &Lua, table: &LuaAnyUserData, deep: bool, frozen: bool, copies: &mut FxHashMap<*const c_void, LuaAnyUserData>) -> LuaResult<LuaAnyUserData> {
    if let Some(done) = copies.get(&table.to_pointer()) {
        return Ok(done.clone());
    }
    let mut contents = table.borrow::<LSharedTable>()?.0.borrow().clone();
    contents.frozen = frozen;
    let result = make(lua, Contents::default())?;
    copies.insert(table.to_pointer(), result.clone());
    if deep {
        let held = |stored: &Stored| match stored {
            Stored::Value(LuaValue::UserData(ud)) if ud.is::<LSharedTable>() => Some(ud.clone()),
            _ => None,
        };
        for value in contents.numbers.values_mut().chain(contents.strings.values_mut()) {
            if let Some(inner) = held(value) {
                *value = Stored::Value(LuaValue::UserData(copy(lua, &inner, true, frozen, copies)?));
            }
        }
    }
    *result.borrow::<LSharedTable>()?.0.borrow_mut() = contents;
    Ok(result)
}

/// The order Roblox's SharedTable walks its string keys: a CHAMP trie keyed by the 64-bit FNV-1a hash of the key's bytes, five
/// bits a level from the lowest, a node's own entries before its subnodes (both by those five bits). Measured: 299 random keys,
/// 40 keys sharing a prefix, and keys after deletes and re-adds all came out in this order, whatever order they were added in.
fn string_order(keys: Vec<&[u8]>) -> Vec<&[u8]> {
    let mut hashed: Vec<(u64, &[u8])> = keys.into_iter().map(|key| (fnv1a(key), key)).collect();
    let mut order = Vec::with_capacity(hashed.len());
    walk(&mut hashed, 0, &mut order);
    order
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn walk<'a>(group: &mut [(u64, &'a [u8])], level: u32, order: &mut Vec<&'a [u8]>) {
    if group.len() <= 1 || level * 5 >= 64 {
        // (one key, or keys whose 64-bit hashes are equal: not measured, by their bytes)
        group.sort_by(|a, b| a.1.cmp(b.1));
        order.extend(group.iter().map(|(_, key)| *key));
        return;
    }
    let slot = |hash: u64| (hash >> (5 * level)) & 31;
    group.sort_by_key(|(hash, _)| slot(*hash));
    let mut branches = Vec::new();
    let mut start = 0;
    while start < group.len() {
        let at = slot(group[start].0);
        let end = start + group[start..].iter().take_while(|(hash, _)| slot(*hash) == at).count();
        if end - start == 1 {
            order.push(group[start].1);
        } else {
            branches.push(start..end);
        }
        start = end;
    }
    for range in branches {
        walk(&mut group[range], level + 1, order);
    }
}

/// The script-facing name of argument `index` (0-based) for an argument error: "no value" when it was not passed.
fn arg_type(engine: &Engine, args: &Args, index: usize) -> String {
    if index >= args.len() { "no value".into() } else { typeof_value(engine, args.get(index)) }
}

/// Argument `index` (0-based) of library function `function`, which must be a SharedTable.
fn shared_arg(engine: &Engine, args: &Args, index: usize, function: &str) -> Result<LuaAnyUserData, Flow> {
    match args.get(index) {
        LuaValue::UserData(ud) if ud.is::<LSharedTable>() => Ok(ud.clone()),
        _ => raise_at_caller(format!("invalid argument #{} to '{function}' (SharedTable expected, got {})", index + 1, arg_type(engine, args, index))),
    }
}

/// The optional `deep` argument of clone and cloneAndFreeze: false when not passed, else a boolean.
fn deep_arg(engine: &Engine, args: &Args, function: &str) -> Result<bool, Flow> {
    match args.get(1) {
        _ if args.len() < 2 => Ok(false),
        LuaValue::Boolean(deep) => Ok(*deep),
        _ => raise_at_caller(format!("invalid argument #2 to '{function}' (boolean expected, got {})", arg_type(engine, args, 1))),
    }
}

fn is_frozen(table: &LuaAnyUserData) -> LuaResult<bool> {
    Ok(table.borrow::<LSharedTable>()?.0.borrow().frozen)
}

/// The value at `key`, for a script.
fn read(lua: &Lua, table: &LuaAnyUserData, key: &Key) -> LuaResult<LuaValue> {
    match table.borrow::<LSharedTable>()?.0.borrow().get(key) {
        Some(stored) => load(lua, stored),
        None => Ok(LuaValue::Nil),
    }
}

/// Writes `value` at `key` (the key already checked, the table not frozen).
fn write(lua: &Lua, engine: &Engine, table: &LuaAnyUserData, key: Key, value: &LuaValue) -> Result<(), Flow> {
    let value = stored(lua, engine, value, &mut Vec::new())?;
    table.borrow::<LSharedTable>()?.0.borrow_mut().set(key, value);
    Ok(())
}

/// SharedTable.update's checks and its callback's call, as (failed, problem, level): the wrapper in [`register`] raises
/// `problem` at the script's line (level 1) or as it is (level 0: the callback's own error, its value kept).
fn update(lua: &Lua, engine: &Engine, pcall: &LuaFunction, args: &Args) -> Result<LuaMultiValue, Flow> {
    let table = shared_arg(engine, args, 0, "update")?;
    let key = key_of(args.get(1))?;
    let LuaValue::Function(callback) = args.get(2) else { return raise("Third argument to SharedTable::Update should be a function.") };
    if is_frozen(&table)? {
        return raise(FROZEN);
    }
    let old = read(lua, &table, &key)?;
    // (through pcall: an error the callback raises comes back as its value, not mlua's text of it)
    let mut results = pcall.call::<LuaMultiValue>((callback.clone(), old))?.into_iter();
    let succeeded = matches!(results.next(), Some(LuaValue::Boolean(true)));
    let first = results.next().unwrap_or(LuaValue::Nil);
    if !succeeded {
        return Ok(values([LuaValue::Boolean(true), first, LuaValue::Integer(0)]));
    }
    write(lua, engine, &table, key, &first)?;
    Ok(values([LuaValue::Boolean(false)]))
}

/// SharedTable.increment: adds `delta` to the number at `key` and returns the old one.
fn increment(engine: &Engine, args: &Args) -> Result<LuaMultiValue, Flow> {
    let table = shared_arg(engine, args, 0, "increment")?;
    let key = key_of(args.get(1))?;
    if is_frozen(&table)? {
        return raise(FROZEN);
    }
    let delta = match args.get(2) {
        LuaValue::Integer(n) => *n as f64,
        LuaValue::Number(n) => *n,
        _ => return raise("Variant cast failed"),
    };
    let userdata = table.borrow::<LSharedTable>()?;
    let mut contents = userdata.0.borrow_mut();
    let old = match contents.get(&key) {
        None => return raise(format!("Unable to increment the value for key \"{}\" because it does not exist in the SharedTable.", key_text(&key))),
        Some(Stored::Value(LuaValue::Number(n))) => *n,
        Some(Stored::Value(LuaValue::Integer(n))) => *n as f64,
        Some(_) => return raise(format!("Unable to increment the value for key \"{}\" because it is not a number value.", key_text(&key))),
    };
    contents.set(key, Some(Stored::Value(LuaValue::Number(old + delta))));
    // (what Studio spent on it: cost.rs)
    let state = engine.state.borrow();
    state.sched.charge(crate::cost::SHARED_INCREMENT);
    if !state.sched.is_muted() {
        crate::boundary::add_cost(crate::cost::SHARED_INCREMENT);
    }
    Ok(values([LuaValue::Number(old)]))
}

const UPDATE_WRAPPER: &str = r#"
local Update = ...
return function(Value, Key, Callback)
	local Failed, Problem, Level = Update(Value, Key, Callback)
	if Failed then
		error(Problem, if Level == 1 then 2 else 0)
	end
end
"#;

/// Registers the SharedTable type and its library (types.rs).
pub fn register(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let index = wrapped(lua, engine, |lua, args| {
        let args = Args::new(args);
        let LuaValue::UserData(table) = args.get(0) else { return raise("invalid SharedTable") };
        let key = key_of(args.get(1))?;
        Ok(values([read(lua, table, &key)?]))
    })?;
    let e = engine.clone();
    let newindex = wrapped(lua, engine, move |lua, args| {
        let args = Args::new(args);
        let LuaValue::UserData(table) = args.get(0) else { return raise("invalid SharedTable") };
        let key = key_of(args.get(1))?;
        if is_frozen(table)? {
            return raise(FROZEN);
        }
        write(lua, &e, table, key, args.get(2))?;
        Ok(none())
    })?;
    let iter = lua.create_function(|lua, table: LuaAnyUserData| {
        let mut entries = table.borrow::<LSharedTable>()?.0.borrow().ordered().into_iter();
        lua.create_function_mut(move |lua, _: LuaMultiValue| match entries.next() {
            Some((key, value)) => Ok((key_value(lua, &key)?, load(lua, &value)?)),
            None => Ok((LuaValue::Nil, LuaValue::Nil)),
        })
    })?;
    let tostring = lua.create_function(|_, table: LuaAnyUserData| Ok(format!("SharedTable: 0x{:016x}", table.to_pointer() as usize)))?;
    lua.register_userdata_type::<LSharedTable>(|registry| {
        registry.add_meta_field("__type", "SharedTable");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__iter", iter);
        registry.add_meta_field("__tostring", tostring);
    })?;
    lock_metatable(lua, &make(lua, Contents::default())?)?;

    let library = lua.create_table()?;
    let e = engine.clone();
    library.set(
        "new",
        wrapped(lua, engine, move |lua, args| {
            let args = Args::new(args);
            match args.get(0) {
                _ if args.is_empty() => Ok(values([LuaValue::UserData(make(lua, Contents::default())?)])),
                LuaValue::Table(table) => Ok(values([LuaValue::UserData(converted(lua, &e, table, &mut Vec::new())?)])),
                _ => raise_at_caller(format!("invalid argument #1 to 'new' (table expected, got {})", arg_type(&e, &args, 0))),
            }
        })?,
    )?;
    for (name, frozen) in [("clone", false), ("cloneAndFreeze", true)] {
        let e = engine.clone();
        library.set(
            name,
            wrapped(lua, engine, move |lua, args| {
                let args = Args::new(args);
                let table = shared_arg(&e, &args, 0, name)?;
                let deep = deep_arg(&e, &args, name)?;
                Ok(values([LuaValue::UserData(copy(lua, &table, deep, frozen, &mut FxHashMap::default())?)]))
            })?,
        )?;
    }
    let e = engine.clone();
    library.set(
        "isFrozen",
        wrapped(lua, engine, move |_, args| Ok(values([LuaValue::Boolean(is_frozen(&shared_arg(&e, &Args::new(args), 0, "isFrozen")?)?)])))?,
    )?;
    let e = engine.clone();
    library.set(
        "size",
        wrapped(lua, engine, move |_, args| {
            let table = shared_arg(&e, &Args::new(args), 0, "size")?;
            let size = table.borrow::<LSharedTable>()?.0.borrow().len();
            Ok(values([LuaValue::Number(size as f64)]))
        })?,
    )?;
    let e = engine.clone();
    library.set(
        "clear",
        wrapped(lua, engine, move |_, args| {
            let table = shared_arg(&e, &Args::new(args), 0, "clear")?;
            if is_frozen(&table)? {
                return raise(FROZEN);
            }
            let userdata = table.borrow::<LSharedTable>()?;
            let mut contents = userdata.0.borrow_mut();
            contents.numbers.clear();
            contents.strings.clear();
            Ok(none())
        })?,
    )?;
    let e = engine.clone();
    library.set("increment", wrapped(lua, engine, move |_, args| increment(&e, &Args::new(args)))?)?;
    let e = engine.clone();
    let pcall: LuaFunction = lua.globals().get("pcall")?;
    let checked = lua.create_function(move |lua, args: LuaMultiValue| match update(lua, &e, &pcall, &Args::new(args)) {
        Ok(results) => Ok(results),
        Err(Flow::Error(error)) => {
            Ok(values([LuaValue::Boolean(true), LuaValue::String(lua.create_string(&error.message)?), LuaValue::Integer(error.level.into())]))
        }
        Err(Flow::Yield) => Err(LuaError::runtime("SharedTable.update cannot yield")),
        Err(Flow::Fail(error)) => Err(error),
    })?;
    let update_fn: LuaFunction = lua.load(UPDATE_WRAPPER).set_name("=ree.sharedtable").call(checked)?;
    library.set("update", update_fn)?;
    library.set_readonly(true);
    engine.lua.borrow_mut().libraries.insert("SharedTable", library);
    Ok(())
}
