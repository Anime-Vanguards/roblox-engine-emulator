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
//! - `#`, `next`, `pairs`, `rawget`, `setmetatable` and the `table` library raise as on any userdata; every SharedTable shares
//!   one metatable, which is locked; `S:size()` raises "attempt to call missing method 'size' of SharedTable", and tostring is
//!   "SharedTable: 0x" and 16 hex digits.
//! - The library (a read-only table): `new([table])`, `clone(S [, deep])` and `cloneAndFreeze(S [, deep])` (a shallow copy
//!   shares the SharedTables it holds, a deep one copies them, frozen too for cloneAndFreeze; a clone of a frozen table is not
//!   frozen), `isFrozen`, `size`, `clear`, `increment(S, key, delta)` (returns the old value) and `update(S, key, callback)`
//!   (the callback gets the old value and returns the new one; it runs where it cannot yield, and an error it raises reaches
//!   the caller as raised). Their messages are below, as measured.
//!
//! Where the contents live: in the engine, as Roblox keeps them in its own (numbers, booleans, vectors, strings and buffers as
//! data, the value types the engine has values for as those, a SharedTable it holds as a shared reference), and the userdata
//! is a handle, one per SharedTable while a script holds it (so `S.t == S.t`). A SharedTable holds none of mlua's reference
//! slots (lua/refs.rs). The value types the engine has no value for (Ray, Axes, TweenInfo, the keypoints, ...) wait in one
//! engine table, a slot each, freed once no SharedTable holds them. A SharedTable that holds itself is never freed, while its
//! handle is: measured, a client collected the handles of 300 SharedTables that held themselves and their 300 MB of buffers
//! stayed (plain ones gave theirs back).
//!
//! What their calls cost the work clock: cost.rs [`crate::cost::SHARED_NEW_STRING_ENTRY`] and the prices after it.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::rc::Rc;

use mlua::prelude::*;
use rustc_hash::FxHashMap;

use super::{Args, Flow, convert, lock_metatable, none, raise, raise_at_caller, raw, typeof_value, values, wrapped};
use crate::{Engine, cost, value::Value};

/// A SharedTable as a script holds it: a handle to its contents.
pub struct LSharedTable(Rc<Shared>);

/// A SharedTable's contents; its identity is this allocation.
struct Shared(RefCell<Contents>);

#[derive(Clone, Default)]
struct Contents {
    frozen: bool,
    numbers: BTreeMap<u32, Stored>,
    strings: FxHashMap<Box<[u8]>, Stored>,
}

/// A value as a SharedTable holds it.
#[derive(Clone)]
enum Stored {
    Boolean(bool),
    Number(f64),
    Vector(f32, f32, f32),
    String(Rc<[u8]>),
    /// A buffer's bytes: a copy goes in, and every read makes a new buffer of them
    Buffer(Rc<[u8]>),
    /// A value type the engine has a value for (CFrame, UDim2, Color3, EnumItem, Font, ...)
    Value(Value),
    /// Another value type, kept in the [`Store`]'s pool
    Pooled(Rc<PoolSlot>),
    Shared(Rc<Shared>),
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

/// A pool slot of the [`Store`]: its value is dropped from the pool once the last SharedTable holding it lets go (at the
/// next value pooled; a drop may come from the collector, where no Lua may run).
struct PoolSlot {
    id: u32,
    released: Rc<RefCell<Vec<u32>>>,
}

impl Drop for PoolSlot {
    fn drop(&mut self) {
        if let Ok(mut released) = self.released.try_borrow_mut() {
            released.push(self.id);
        }
    }
}

/// The engine's part of every SharedTable: the handles (a weak-valued table, one per SharedTable while a script holds it) and
/// the pool of the values the engine has no value of its own for.
struct Store {
    handles: LuaTable,
    pool: LuaTable,
    released: Rc<RefCell<Vec<u32>>>,
    free: RefCell<Vec<u32>>,
    next: Cell<u32>,
}

impl Store {
    fn put(&self, value: LuaValue) -> LuaResult<Rc<PoolSlot>> {
        // (what was let go: taken out before any Lua runs, which may collect and let go of more)
        let released = std::mem::take(&mut *self.released.borrow_mut());
        for id in released {
            self.pool.raw_set(id, LuaValue::Nil)?;
            self.free.borrow_mut().push(id);
        }
        let reused = self.free.borrow_mut().pop();
        let id = reused.unwrap_or_else(|| {
            self.next.set(self.next.get() + 1);
            self.next.get()
        });
        self.pool.raw_set(id, value)?;
        Ok(Rc::new(PoolSlot { id, released: self.released.clone() }))
    }

    /// The handle of `shared`: the one a script holds, else a new one.
    fn handle(&self, lua: &Lua, shared: &Rc<Shared>) -> LuaResult<LuaAnyUserData> {
        let key = Rc::as_ptr(shared) as usize as f64;
        if let LuaValue::UserData(handle) = self.handles.raw_get::<LuaValue>(key)?
            && handle.borrow::<LSharedTable>().is_ok_and(|held| Rc::ptr_eq(&held.0, shared))
        {
            return Ok(handle);
        }
        let handle = lua.create_any_userdata(LSharedTable(shared.clone()))?;
        self.handles.raw_set(key, handle.clone())?;
        Ok(handle)
    }
}

/// Entries a call handled, for its price: by their key's kind, and the SharedTables a deep clone copied.
#[derive(Default)]
struct Tally {
    strings: u64,
    integers: u64,
    tables: u64,
}

impl Tally {
    fn count(&mut self, key: &Key) {
        match key {
            Key::Number(_) => self.integers += 1,
            Key::String(_) => self.strings += 1,
        }
    }

    fn count_contents(&mut self, contents: &Contents) {
        self.integers += contents.numbers.len() as u64;
        self.strings += contents.strings.len() as u64;
    }
}

fn charge(engine: &Engine, seconds: f64) {
    engine.state.borrow().sched.charge(seconds);
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
    "SecurityCapabilities", "TweenInfo", "UDim", "UDim2", "Vector2", "Vector2int16", "Vector3int16",
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

/// What a SharedTable holds for `value` (None: no entry). `open` holds the tables being converted, to find a cycle; `tally`
/// counts the entries converted.
fn stored(lua: &Lua, engine: &Engine, store: &Store, value: &LuaValue, open: &mut Vec<*const c_void>, tally: &mut Tally) -> Result<Option<Stored>, Flow> {
    Ok(Some(match value {
        LuaValue::Nil | LuaValue::Thread(_) | LuaValue::LightUserData(_) | LuaValue::Error(_) | LuaValue::Other(_) => return Ok(None),
        LuaValue::Boolean(b) => Stored::Boolean(*b),
        LuaValue::Integer(n) => Stored::Number(*n as f64),
        LuaValue::Number(n) => Stored::Number(*n),
        LuaValue::Vector(v) => Stored::Vector(v.x(), v.y(), v.z()),
        LuaValue::String(s) => Stored::String(s.as_bytes().to_vec().into()),
        LuaValue::Buffer(buffer) => Stored::Buffer(buffer.to_vec().into()),
        LuaValue::Function(_) => return refused("Function"),
        LuaValue::Table(table) => Stored::Shared(converted(lua, engine, store, table, open, tally)?),
        LuaValue::UserData(ud) => {
            if let Ok(handle) = ud.borrow::<LSharedTable>() {
                return Ok(Some(Stored::Shared(handle.0.clone())));
            }
            if raw::instance_of_ud(ud).is_some() {
                return refused("Instance");
            }
            let name = typeof_value(engine, value);
            match name.as_str() {
                name if KEPT.contains(&name) => match convert::from_lua(value) {
                    Value::Lua(_) => Stored::Pooled(store.put(value.clone())?),
                    known => Stored::Value(known),
                },
                name if DROPPED.contains(&name) => return Ok(None),
                "RBXScriptSignal" => return refused("EventInstance"),
                "RBXScriptConnection" => return refused("Connection"),
                other => return refused(other),
            }
        }
    }))
}

/// A table as a new SharedTable (its own tables converted too).
fn converted(lua: &Lua, engine: &Engine, store: &Store, table: &LuaTable, open: &mut Vec<*const c_void>, tally: &mut Tally) -> Result<Rc<Shared>, Flow> {
    let pointer = table.to_pointer();
    if open.contains(&pointer) {
        return raise(CYCLIC);
    }
    open.push(pointer);
    let mut contents = Contents::default();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        let key = key_of(&key)?;
        tally.count(&key);
        let value = stored(lua, engine, store, &value, open, tally)?;
        contents.set(key, value);
    }
    open.pop();
    Ok(Rc::new(Shared(RefCell::new(contents))))
}

fn load(lua: &Lua, engine: &Engine, store: &Store, stored: &Stored) -> LuaResult<LuaValue> {
    Ok(match stored {
        Stored::Boolean(b) => LuaValue::Boolean(*b),
        Stored::Number(n) => LuaValue::Number(*n),
        Stored::Vector(x, y, z) => LuaValue::Vector(mlua::Vector::new(*x, *y, *z)),
        Stored::String(bytes) => LuaValue::String(lua.create_string(&**bytes)?),
        Stored::Buffer(bytes) => LuaValue::Buffer(lua.create_buffer(&**bytes)?),
        Stored::Value(value) => convert::to_lua(lua, engine, value)?,
        Stored::Pooled(slot) => store.pool.raw_get(slot.id)?,
        Stored::Shared(shared) => LuaValue::UserData(store.handle(lua, shared)?),
    })
}

/// A copy of `shared`, frozen or not; a deep one copies the SharedTables it holds as well (`copies`: those copied so far, so a
/// table held twice, or holding itself, is copied once), counting their entries and themselves in `tally`.
fn copy(shared: &Rc<Shared>, deep: bool, frozen: bool, copies: &mut FxHashMap<*const Shared, Rc<Shared>>, tally: &mut Tally) -> Rc<Shared> {
    if let Some(done) = copies.get(&Rc::as_ptr(shared)) {
        return done.clone();
    }
    let result = Rc::new(Shared(RefCell::new(Contents::default())));
    copies.insert(Rc::as_ptr(shared), result.clone());
    let mut contents = shared.0.borrow().clone();
    contents.frozen = frozen;
    if deep {
        tally.count_contents(&contents);
        for value in contents.numbers.values_mut().chain(contents.strings.values_mut()) {
            if let Stored::Shared(inner) = value {
                tally.tables += 1;
                *value = Stored::Shared(copy(inner, true, frozen, copies, tally));
            }
        }
    }
    *result.0.borrow_mut() = contents;
    result
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

/// Argument `index` (0-based) of library function `function`, which must be a SharedTable: its contents.
fn shared_arg(engine: &Engine, args: &Args, index: usize, function: &str) -> Result<Rc<Shared>, Flow> {
    match args.get(index) {
        LuaValue::UserData(ud) if ud.is::<LSharedTable>() => Ok(ud.borrow::<LSharedTable>()?.0.clone()),
        _ => raise_at_caller(format!("invalid argument #{} to '{function}' (SharedTable expected, got {})", index + 1, arg_type(engine, args, index))),
    }
}

/// The SharedTable a metamethod was called on.
fn this(args: &Args) -> Result<Rc<Shared>, Flow> {
    match args.get(0) {
        LuaValue::UserData(ud) => Ok(ud.borrow::<LSharedTable>()?.0.clone()),
        _ => raise("invalid SharedTable"),
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

/// The value at `key`, for a script.
fn read(lua: &Lua, engine: &Engine, store: &Store, shared: &Shared, key: &Key) -> LuaResult<LuaValue> {
    match shared.0.borrow().get(key) {
        Some(stored) => load(lua, engine, store, stored),
        None => Ok(LuaValue::Nil),
    }
}

/// Writes `value` at `key` (the key already checked, the table not frozen).
fn write(lua: &Lua, engine: &Engine, store: &Store, shared: &Shared, key: Key, value: &LuaValue) -> Result<(), Flow> {
    let value = stored(lua, engine, store, value, &mut Vec::new(), &mut Tally::default())?;
    shared.0.borrow_mut().set(key, value);
    Ok(())
}

/// SharedTable.update's checks and its callback's call, as (failed, problem, level): the wrapper in [`register`] raises
/// `problem` at the script's line (level 1) or as it is (level 0: the callback's own error, its value kept).
fn update(lua: &Lua, engine: &Engine, store: &Store, pcall: &LuaFunction, args: &Args) -> Result<LuaMultiValue, Flow> {
    let shared = shared_arg(engine, args, 0, "update")?;
    let key = key_of(args.get(1))?;
    let LuaValue::Function(callback) = args.get(2) else { return raise("Third argument to SharedTable::Update should be a function.") };
    if shared.0.borrow().frozen {
        return raise(FROZEN);
    }
    let old = read(lua, engine, store, &shared, &key)?;
    // (through pcall: an error the callback raises comes back as its value, not mlua's text of it)
    let mut results = pcall.call::<LuaMultiValue>((callback.clone(), old))?.into_iter();
    let succeeded = matches!(results.next(), Some(LuaValue::Boolean(true)));
    let first = results.next().unwrap_or(LuaValue::Nil);
    if !succeeded {
        return Ok(values([LuaValue::Boolean(true), first, LuaValue::Integer(0)]));
    }
    write(lua, engine, store, &shared, key, &first)?;
    Ok(values([LuaValue::Boolean(false)]))
}

/// SharedTable.increment: adds `delta` to the number at `key` and returns the old one.
fn increment(engine: &Engine, args: &Args) -> Result<LuaMultiValue, Flow> {
    let shared = shared_arg(engine, args, 0, "increment")?;
    let key = key_of(args.get(1))?;
    if shared.0.borrow().frozen {
        return raise(FROZEN);
    }
    let delta = match args.get(2) {
        LuaValue::Integer(n) => *n as f64,
        LuaValue::Number(n) => *n,
        _ => return raise("Variant cast failed"),
    };
    let mut contents = shared.0.borrow_mut();
    let old = match contents.get(&key) {
        None => return raise(format!("Unable to increment the value for key \"{}\" because it does not exist in the SharedTable.", key_text(&key))),
        Some(Stored::Number(n)) => *n,
        Some(_) => return raise(format!("Unable to increment the value for key \"{}\" because it is not a number value.", key_text(&key))),
    };
    contents.set(key, Some(Stored::Number(old + delta)));
    drop(contents);
    // (what Studio spent on it: cost.rs)
    let state = engine.state.borrow();
    state.sched.charge(cost::SHARED_INCREMENT);
    if !state.sched.is_muted() {
        crate::boundary::add_cost(cost::SHARED_INCREMENT);
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
    let handles = lua.create_table()?;
    let weak = lua.create_table()?;
    weak.raw_set("__mode", "v")?;
    handles.set_metatable(Some(weak))?;
    let store = Rc::new(Store { handles, pool: lua.create_table()?, released: Rc::default(), free: RefCell::default(), next: Cell::new(0) });

    let (e, s) = (engine.clone(), store.clone());
    let index = wrapped(lua, engine, move |lua, args| {
        let args = Args::new(args);
        let shared = this(&args)?;
        let key = key_of(args.get(1))?;
        Ok(values([read(lua, &e, &s, &shared, &key)?]))
    })?;
    let (e, s) = (engine.clone(), store.clone());
    let newindex = wrapped(lua, engine, move |lua, args| {
        let args = Args::new(args);
        let shared = this(&args)?;
        let key = key_of(args.get(1))?;
        if shared.0.borrow().frozen {
            return raise(FROZEN);
        }
        write(lua, &e, &s, &shared, key, args.get(2))?;
        Ok(none())
    })?;
    let (e, s) = (engine.clone(), store.clone());
    let iter = lua.create_function(move |lua, handle: LuaAnyUserData| {
        let entries = handle.borrow::<LSharedTable>()?.0.0.borrow().ordered();
        let mut tally = Tally::default();
        for (key, _) in &entries {
            tally.count(key);
        }
        charge(&e, cost::SHARED_LOOP + tally.strings as f64 * cost::SHARED_LOOP_STRING_ENTRY + tally.integers as f64 * cost::SHARED_LOOP_INTEGER_ENTRY);
        let mut entries = entries.into_iter();
        let (e, s) = (e.clone(), s.clone());
        lua.create_function_mut(move |lua, _: LuaMultiValue| match entries.next() {
            Some((key, value)) => Ok((key_value(lua, &key)?, load(lua, &e, &s, &value)?)),
            None => Ok((LuaValue::Nil, LuaValue::Nil)),
        })
    })?;
    let tostring = lua.create_function(|_, handle: LuaAnyUserData| Ok(format!("SharedTable: 0x{:016x}", handle.to_pointer() as usize)))?;
    lua.register_userdata_type::<LSharedTable>(|registry| {
        registry.add_meta_field("__type", "SharedTable");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__iter", iter);
        registry.add_meta_field("__tostring", tostring);
    })?;
    lock_metatable(lua, &lua.create_any_userdata(LSharedTable(Rc::new(Shared(RefCell::default()))))?)?;

    let library = lua.create_table()?;
    let (e, s) = (engine.clone(), store.clone());
    library.set(
        "new",
        wrapped(lua, engine, move |lua, args| {
            let args = Args::new(args);
            let shared = match args.get(0) {
                _ if args.is_empty() => Rc::new(Shared(RefCell::default())),
                LuaValue::Table(table) => {
                    let mut tally = Tally::default();
                    let shared = converted(lua, &e, &s, table, &mut Vec::new(), &mut tally)?;
                    charge(&e, tally.strings as f64 * cost::SHARED_NEW_STRING_ENTRY + tally.integers as f64 * cost::SHARED_NEW_INTEGER_ENTRY);
                    shared
                }
                _ => return raise_at_caller(format!("invalid argument #1 to 'new' (table expected, got {})", arg_type(&e, &args, 0))),
            };
            Ok(values([LuaValue::UserData(s.handle(lua, &shared)?)]))
        })?,
    )?;
    for (name, frozen) in [("clone", false), ("cloneAndFreeze", true)] {
        let (e, s) = (engine.clone(), store.clone());
        library.set(
            name,
            wrapped(lua, engine, move |lua, args| {
                let args = Args::new(args);
                let shared = shared_arg(&e, &args, 0, name)?;
                let deep = deep_arg(&e, &args, name)?;
                let mut tally = Tally::default();
                let copied = copy(&shared, deep, frozen, &mut FxHashMap::default(), &mut tally);
                // (a shallow copy costs Studio the same at any size: only the call)
                charge(
                    &e,
                    tally.strings as f64 * cost::SHARED_DEEP_STRING_ENTRY + tally.integers as f64 * cost::SHARED_DEEP_INTEGER_ENTRY + tally.tables as f64 * cost::SHARED_DEEP_TABLE,
                );
                Ok(values([LuaValue::UserData(s.handle(lua, &copied)?)]))
            })?,
        )?;
    }
    let e = engine.clone();
    library.set("isFrozen", wrapped(lua, engine, move |_, args| Ok(values([LuaValue::Boolean(shared_arg(&e, &Args::new(args), 0, "isFrozen")?.0.borrow().frozen)])))?)?;
    let e = engine.clone();
    library.set(
        "size",
        wrapped(lua, engine, move |_, args| Ok(values([LuaValue::Number(shared_arg(&e, &Args::new(args), 0, "size")?.0.borrow().len() as f64)])))?,
    )?;
    let e = engine.clone();
    library.set(
        "clear",
        wrapped(lua, engine, move |_, args| {
            let shared = shared_arg(&e, &Args::new(args), 0, "clear")?;
            if shared.0.borrow().frozen {
                return raise(FROZEN);
            }
            // (taken out first: what it held may let go of a pool slot)
            let cleared = std::mem::take(&mut *shared.0.borrow_mut());
            drop(cleared);
            Ok(none())
        })?,
    )?;
    let e = engine.clone();
    library.set("increment", wrapped(lua, engine, move |_, args| increment(&e, &Args::new(args)))?)?;
    let (e, s) = (engine.clone(), store.clone());
    let pcall: LuaFunction = lua.globals().get("pcall")?;
    let checked = lua.create_function(move |lua, args: LuaMultiValue| match update(lua, &e, &s, &pcall, &Args::new(args)) {
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
