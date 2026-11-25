//! What BindableEvent:Fire and BindableFunction:Invoke pass on. Measured in a playing client (Studio 0.744,
//! tools/parity/probes/signals.luau, each capture taken twice):
//!
//! - Every table is copied: once when it is sent (so a change the sender makes afterwards does not arrive), and
//!   again for each receiver, so every handler, waiting thread and Invoke caller gets a table of its own. Shared
//!   subtables and a table passed twice become separate copies. Metatables and freezing are dropped.
//! - A table whose border (`#t`) is not 0 is copied as an array: indices 1 to `#t`, holes kept, every other key
//!   dropped (`{1, 2, Key = "v"}` arrives as `{1, 2}`, `{[1] = "a", [3] = "c"}` as `{"a"}`).
//! - Any other table is copied as a dictionary whose keys all become strings: a number truncated to a 64-bit
//!   integer (`1.5` -> `"1"`, `-9223372036854775808` when out of range), any other key `<Type> (text)`, its text
//!   tostring's: an Instance `<Instance> (Name)`, an InstanceHandle `<InstanceRef> (Name)` (`<InstanceRef> (nil)` for a
//!   handle of none; Studio 0.748), a function `<Function> (function: 0x...)`, a table `<Table> (table: 0x...)` (or its
//!   `__tostring`'s text), a vector `<Vector3> (1, 2, 3)`, an EnumItem `<token> (Enum.Material.Plastic)`, a buffer
//!   `<Buffer> (buffer: 0x...)`, the other datatypes by their typeof but for the few [`key_type`] names otherwise (a CFrame
//!   `<CoordinateFrame> (...)`). Keys that end up equal keep the later one in iteration order. A boolean or thread key is
//!   an error, `Invalid table key type used`. A key's `__tostring` runs on the sending thread, once for each key, and an
//!   error it raises ends the copy: the sending call raises it as it was raised (a table as that table; a yield is `thread
//!   is not yieldable`) and sends nothing, the first such key's alone. So does the table's `next` from that key when its
//!   `__tostring` took the key out of the table (`invalid key to 'next'` after a `table.clear`): [`key_name`].
//! - A table that contains itself is an error, `tables cannot be cyclic`.
//! - The copy walks a table on the Lua stack above the call's own values: the outermost table sits at the height of the
//!   call's frame (`self` and the arguments of Fire and Invoke, the results of an Invoke's callback), a table inside an
//!   array one slot above its parent, one inside a dictionary two (its key below it). A table that would sit above
//!   [`STACK_LIMIT`] is an error, `Max lua stack size reached`, raised before a cycle through it is found; the outermost
//!   table is never refused, whatever the arguments around it. So Fire takes an array nested 299 deep alone and 298
//!   after another argument, a dictionary 150 deep, and a callback may return an array 300 deep (measured in a 0.747
//!   playing client and server, tools/parity/probes/table_depth.luau). Remotes, data stores and MessagingService walk
//!   the same way from heights of their own (loopback/wire.rs, server.luau `Data`).
//! - A thread arrives as nil (in an array it leaves a hole, in a dictionary its entry is dropped); a buffer as a
//!   new buffer with the same bytes; a SharedTable as a new handle of the same table (Studio 0.748, [`rebuild`]), an
//!   InstanceHandle as a new handle of the same instance, or of none (Studio 0.748: `==` to the one sent, not rawequal, at
//!   top level and in a table, Fire and Invoke alike); everything else (functions, Instances, datatypes, strings) as it is.
//! - The copy takes time in proportion to what it makes, a shared table counted each time it appears, and has no limit
//!   of its own (a 0.747 playing client, one table of 1,000 numbers held 3,000 times: Fire 38 ms and each handler's copy
//!   33 ms more; held 10,000 times, Fire 0.34 s). Since one small table may stand for a copy of any size (one holding the
//!   same 100,000-item table 100,000 times is 10^10 values), ree stops a copy past [`COPY_VALUES`] values or
//!   [`COPY_BYTES`] bytes with [`too_large`]'s error, a guard of its own ([`Tally`]).
//!
//! Keys measured in a 0.747 playing client and server, Fire, Invoke, JSONEncode and the remotes alike
//! (tools/parity/probes/table_keys.luau, loopback/remote_keys.luau): every datatype a script can make, and keys whose
//! `__tostring` returns a number, a string, nil, a boolean or a table, raises a string, a table, nil or a number, yields,
//! fails in its own code, or clears the table or takes its own entry out of it.

use mlua::{ffi, prelude::*};
use std::ffi::{c_int, c_void};
use std::rc::Rc;

use super::{Flow, Ret, raise};
use crate::{Engine, value::Value};

/// Why a copy stopped: the engine's own error, or one a script's code raised while the copy ran (a dictionary key's
/// `__tostring`), which the call raises as it was raised (a table as that table, nil as nil).
#[derive(Debug)]
pub enum Refused {
    Message(String),
    Raised(LuaValue),
}

impl From<String> for Refused {
    fn from(message: String) -> Self {
        Refused::Message(message)
    }
}

impl Refused {
    /// The error the engine call raises
    pub fn raise<T>(self) -> Result<T, Flow> {
        match self {
            Refused::Message(message) => raise(message),
            Refused::Raised(value) => Err(Flow::Raised(value)),
        }
    }

    /// The error as text, where only text goes on (a RemoteFunction's answer to its caller): a raised string's text, any
    /// other raised value's empty text (as a RemoteFunction answers a callback's table error)
    pub fn text(self) -> String {
        match self {
            Refused::Message(message) => message,
            Refused::Raised(LuaValue::String(text)) => text.to_string_lossy(),
            Refused::Raised(_) => String::new(),
        }
    }
}

/// A sent value as its receivers rebuild it.
pub enum Sent {
    /// nil, a boolean, number, string, function, Instance, vector or other userdata: passed as it is
    Value(LuaValue),
    /// A buffer: a new one with these bytes for each receiver
    Buffer(Rc<[u8]>),
    /// A table: its entries, keys already converted, in the order they are set
    Table(Vec<(Key, Sent)>),
}

pub enum Key {
    Index(i64),
    Name(Vec<u8>),
}

/// The engine value of a sent table or buffer: each conversion to Luau (convert::to_lua) rebuilds a new copy.
pub struct Copied(pub Rc<Sent>);

impl LuaUserData for Copied {}

/// The highest slot of the Lua stack a copied table may sit at ([`nested_height`]); one higher is [`STACK_FULL`].
pub const STACK_LIMIT: usize = 300;

/// The error of a table nested too deep for the stack.
pub const STACK_FULL: &str = "Max lua stack size reached";

/// ree's guard on one copy: the values an engine call's copy may make (each argument, each array slot walked, holes
/// included, and each dictionary entry, a table counted each time it appears). Roblox has no limit but memory (10,000,000
/// values crossed Fire, Invoke, a ValueCurveKey and JSONEncode in a 0.747 client: the module's notes); this is hundreds of
/// times what a game sends, and ree's copy of this many holds about half a gigabyte.
pub const COPY_VALUES: u64 = 1 << 22;

/// ree's guard on the bytes one copy makes: the strings and dictionary keys it writes out and buffers' bytes (a string a
/// bindable passes on is shared, not counted); as much as one Luau string may hold.
pub const COPY_BYTES: u64 = 1 << 30;

/// What one engine call's copy has made so far, against ree's guard ([`COPY_VALUES`], [`COPY_BYTES`]). Every copy of script
/// tables counts as it goes (this module's, loopback/copier.rs and wire.rs, services/json.rs; server.luau `Data` keeps a
/// count of its own), so a table copied once for each place it appears stops at the guard; a SharedTable conversion checks a
/// table's slots against it before it walks them, and counts the empty ones it walks and the trie slots a deep copy copies
/// (lua/shared_table.rs).
#[derive(Default)]
pub struct Tally {
    pub values: u64,
    pub bytes: u64,
}

impl Tally {
    /// One more value copied, making `bytes` bytes: [`too_large`]'s error past the guard
    #[inline]
    pub fn add(&mut self, bytes: usize) -> Result<(), String> {
        self.values += 1;
        self.bytes += bytes as u64;
        if self.values > COPY_VALUES || self.bytes > COPY_BYTES { Err(self.too_large()) } else { Ok(()) }
    }

    /// `count` more values made at once (the empty slots a SharedTable conversion walks, the slots of a node its deep copy
    /// copies): [`too_large`]'s error past the guard
    #[inline]
    pub fn add_values(&mut self, count: u64) -> Result<(), String> {
        self.values += count;
        if self.values > COPY_VALUES { Err(self.too_large()) } else { Ok(()) }
    }

    /// Whether `count` more values would still be within the guard, counting nothing: [`too_large`]'s error when they would not
    /// (a SharedTable conversion's check of a table's slots, read at once, before it walks them)
    #[inline]
    pub fn check_values(&self, count: u64) -> Result<(), String> {
        if self.values.saturating_add(count) > COPY_VALUES { Err(too_large(COPY_VALUES, "values")) } else { Ok(()) }
    }

    /// `bytes` more bytes made beside a value (a dictionary key JSONEncode writes): [`too_large`]'s error past the guard
    #[inline]
    pub fn add_bytes(&mut self, bytes: usize) -> Result<(), String> {
        self.bytes += bytes as u64;
        if self.bytes > COPY_BYTES { Err(self.too_large()) } else { Ok(()) }
    }

    #[cold]
    fn too_large(&self) -> String {
        if self.values > COPY_VALUES { too_large(COPY_VALUES, "values") } else { too_large(COPY_BYTES, "bytes") }
    }
}

/// The most entries a copy reserves ahead for an array: a table's border (`#t`) can stand for an array part far larger than
/// what it holds (`table.create(n)` with only its last slot set), so a longer array's copy grows as it fills
pub const PRESIZE_LIMIT: usize = 1 << 16;

/// The error of a copy past ree's guard ([`Tally`]): more than `limit` `what` (values, bytes)
pub fn too_large(limit: u64, what: &str) -> String {
    format!("ree: this copy would make more than {limit} {what} (a table is copied once for each place it appears)")
}

/// The stack height of a table found inside one at `height`: an array's item sits one slot above it, a dictionary's value
/// two (above its key); an error when that is above [`STACK_LIMIT`].
pub fn nested_height(height: usize, array: bool) -> Result<usize, String> {
    let nested = height + if array { 1 } else { 2 };
    if nested > STACK_LIMIT { Err(STACK_FULL.into()) } else { Ok(nested) }
}

/// A value as it is sent: an error when a table cannot be (cyclic, nested too deep, or a key of a kind that cannot be
/// sent). `height`: the call's frame, where the value's outermost table sits (the module's notes).
/// `tally`: what the call's copy has made so far, its other values' included ([`Tally`]).
pub fn capture(lua: &Lua, engine: &Engine, value: &LuaValue, height: usize, tally: &mut Tally) -> Result<Sent, Refused> {
    tally.add(buffer_bytes(value))?;
    Ok(capture_value(lua, engine, value, height, &mut Vec::new(), tally)?.unwrap_or(Sent::Value(LuaValue::Nil)))
}

/// The bytes a copy of `value` makes: a buffer's
#[inline]
fn buffer_bytes(value: &LuaValue) -> usize {
    if let LuaValue::Buffer(buffer) = value { buffer.len() } else { 0 }
}

/// `None`: a value that does not arrive (a thread).
fn capture_value(lua: &Lua, engine: &Engine, value: &LuaValue, height: usize, ancestors: &mut Vec<*const c_void>, tally: &mut Tally) -> Result<Option<Sent>, Refused> {
    Ok(Some(match value {
        LuaValue::Thread(_) => return Ok(None),
        LuaValue::Table(table) => capture_table(lua, engine, table, height, ancestors, tally)?,
        LuaValue::Buffer(buffer) => Sent::Buffer(buffer.to_vec().into()),
        other => {
            // (a SharedTable sent is marked, at top level and in a table, Invoke's results too: shared_table.rs `mark_value`,
            // measured; a ValueCurveKey's Value, copied alike, not measured)
            super::shared_table::mark_value(other);
            Sent::Value(other.clone())
        }
    }))
}

fn capture_table(lua: &Lua, engine: &Engine, table: &LuaTable, height: usize, ancestors: &mut Vec<*const c_void>, tally: &mut Tally) -> Result<Sent, Refused> {
    let pointer = table.to_pointer();
    if ancestors.contains(&pointer) {
        return Err(Refused::Message("tables cannot be cyclic".into()));
    }
    ancestors.push(pointer);
    let entries = capture_entries(lua, engine, table, height, ancestors, tally);
    ancestors.pop();
    Ok(Sent::Table(entries?))
}

/// The copy of a value held by a table at `height`: a table is checked against the stack ([`nested_height`]) before it is
/// walked, so one too deep is refused before a cycle through it is found
fn capture_item(lua: &Lua, engine: &Engine, item: &LuaValue, height: usize, array: bool, ancestors: &mut Vec<*const c_void>, tally: &mut Tally) -> Result<Option<Sent>, Refused> {
    let height = if matches!(item, LuaValue::Table(_)) { nested_height(height, array)? } else { height };
    capture_value(lua, engine, item, height, ancestors, tally)
}

fn capture_entries(lua: &Lua, engine: &Engine, table: &LuaTable, height: usize, ancestors: &mut Vec<*const c_void>, tally: &mut Tally) -> Result<Vec<(Key, Sent)>, Refused> {
    let mut entries = Vec::new();
    let length = table.raw_len();
    if length > 0 {
        // Walk array slots directly on the C stack so nil/thread/primitive/buffer elements avoid
        // per-element mlua registry refs; pause onto `raw_get` + `capture_item` only for tables,
        // strings, functions, and userdata.
        let mut cursor = 1usize;
        while cursor <= length {
            let mut pause_at: Option<usize> = None;
            let mut refused: Option<Refused> = None;
            unsafe {
                lua.exec_raw::<()>(table, |state| {
                    while cursor <= length {
                        let kind = ffi::lua_rawgeti_(state, 1, cursor as c_int);
                        match kind {
                            ffi::LUA_TNIL | ffi::LUA_TTHREAD => {
                                ffi::lua_pop(state, 1);
                                // (every slot, a hole too: walking it is the copy's work)
                                if let Err(msg) = tally.add(0) {
                                    refused = Some(Refused::Message(msg));
                                    return;
                                }
                            }
                            ffi::LUA_TBOOLEAN => {
                                let b = ffi::lua_toboolean(state, -1) != 0;
                                ffi::lua_pop(state, 1);
                                if let Err(msg) = tally.add(0) {
                                    refused = Some(Refused::Message(msg));
                                    return;
                                }
                                entries.push((Key::Index(cursor as i64), Sent::Value(LuaValue::Boolean(b))));
                            }
                            ffi::LUA_TNUMBER => {
                                let n = ffi::lua_tonumber(state, -1);
                                ffi::lua_pop(state, 1);
                                if let Err(msg) = tally.add(0) {
                                    refused = Some(Refused::Message(msg));
                                    return;
                                }
                                entries.push((Key::Index(cursor as i64), Sent::Value(LuaValue::Number(n))));
                            }
                            ffi::LUA_TVECTOR => {
                                let v = ffi::lua_tovector(state, -1);
                                let vec = mlua::Vector::new(*v, *v.add(1), *v.add(2));
                                ffi::lua_pop(state, 1);
                                if let Err(msg) = tally.add(0) {
                                    refused = Some(Refused::Message(msg));
                                    return;
                                }
                                entries.push((Key::Index(cursor as i64), Sent::Value(LuaValue::Vector(vec))));
                            }
                            ffi::LUA_TBUFFER => {
                                let mut len = 0usize;
                                let ptr = ffi::lua_tobuffer(state, -1, &mut len) as *const u8;
                                if let Err(msg) = tally.add(len) {
                                    ffi::lua_pop(state, 1);
                                    refused = Some(Refused::Message(msg));
                                    return;
                                }
                                let bytes = if len == 0 { &[] } else { std::slice::from_raw_parts(ptr, len) };
                                let copied: Rc<[u8]> = Rc::from(bytes);
                                ffi::lua_pop(state, 1);
                                entries.push((Key::Index(cursor as i64), Sent::Buffer(copied)));
                            }
                            _ => {
                                ffi::lua_pop(state, 1);
                                pause_at = Some(cursor);
                                return;
                            }
                        }
                        cursor += 1;
                    }
                })
            }
            .map_err(|error| error.to_string())?;
            if let Some(err) = refused {
                return Err(err);
            }
            if let Some(index) = pause_at {
                let item: LuaValue = table.raw_get(index).map_err(|error| error.to_string())?;
                tally.add(buffer_bytes(&item))?;
                if !item.is_nil()
                    && let Some(copy) = capture_item(lua, engine, &item, height, true, ancestors, tally)?
                {
                    entries.push((Key::Index(index as i64), copy));
                }
                cursor = index + 1;
            }
        }
        return Ok(entries);
    }
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, item) = pair.map_err(|error| error.to_string())?;
        let key = key_name(lua, engine, table, &key)?;
        tally.add(key.len() + buffer_bytes(&item))?;
        if let Some(copy) = capture_item(lua, engine, &item, height, false, ancestors, tally)? {
            entries.push((Key::Name(key), copy));
        }
    }
    Ok(entries)
}

/// A dictionary key of `table` as the receiver sees it (always a string: the module's notes). HttpService:JSONEncode and a
/// localization table's arguments and values name their keys the same way (services/json.rs, services/localization.rs). A
/// key's text is tostring's, called in protected mode: the error it raises, as it was raised, is the copy's
/// ([`Refused::Raised`]). Where that ran a script's code (a table's or a newproxy's `__tostring`), the table's `next` from the
/// key is checked too, as a client's walk takes it next: `invalid key to 'next'` once the code took the key out of the table
/// (`table.clear`; setting its value to nil keeps it).
pub(crate) fn key_name(lua: &Lua, engine: &Engine, table: &LuaTable, key: &LuaValue) -> Result<Vec<u8>, Refused> {
    let typeof_name;
    let (text, scripted) = match key {
        LuaValue::String(text) => return Ok(text.as_bytes().to_vec()),
        LuaValue::Integer(number) => return Ok(number.to_string().into_bytes()),
        LuaValue::Number(number) => return Ok(integer_of(*number).to_string().into_bytes()),
        LuaValue::Boolean(_) | LuaValue::Thread(_) => return Err(Refused::Message("Invalid table key type used".into())),
        LuaValue::Table(_) => ("Table", true),
        LuaValue::Function(_) => ("Function", false),
        LuaValue::Buffer(_) => ("Buffer", false),
        LuaValue::Vector(_) => ("Vector3", false),
        other => {
            typeof_name = super::typeof_value(engine, other);
            (key_type(&typeof_name), typeof_name == "userdata")
        }
    };
    let shown = match protected(engine, lua, "tostring", [key.clone()])? {
        LuaValue::String(shown) => shown.to_string_lossy(),
        _ => String::new(),
    };
    if scripted {
        protected(engine, lua, "next", [LuaValue::Table(table.clone()), key.clone()])?;
    }
    Ok(format!("<{text}> ({shown})").into_bytes())
}

/// The type a dictionary key's name gives a value whose typeof is `kind`: a client's own name for some types, the typeof for
/// the rest (measured in a 0.747 playing client and server: tools/parity/probes/table_keys.luau)
fn key_type(kind: &str) -> &str {
    match kind {
        "EnumItem" => "token",
        "InstanceHandle" => "InstanceRef",
        "CFrame" => "CoordinateFrame",
        "BrickColor" => "int",
        "Rect" => "Rect2D",
        "RBXScriptSignal" => "EventInstance",
        "RBXScriptConnection" => "Connection",
        // (an Enum, `Enum` itself, a RotationCurveKey and a newproxy)
        "Enum" | "Enums" | "RotationCurveKey" | "userdata" => "null",
        other => other,
    }
}

/// The library function `name` called with `args` in protected mode: its first result, or the error it raised, as it was
/// raised (charged what that raise costs a client beyond the call's own: cost.rs [`crate::cost::COPY_KEY_RAISED`])
fn protected<const N: usize>(engine: &Engine, lua: &Lua, name: &str, args: [LuaValue; N]) -> Result<LuaValue, Refused> {
    let failed = |error: LuaError| Refused::Message(error.to_string());
    let globals = lua.globals();
    let pcall: LuaFunction = globals.get("pcall").map_err(failed)?;
    let function: LuaFunction = globals.get(name).map_err(failed)?;
    let mut call = LuaMultiValue::with_capacity(N + 1);
    call.push_back(LuaValue::Function(function));
    call.extend(args);
    let mut results = pcall.call::<LuaMultiValue>(call).map_err(failed)?.into_iter();
    let ok = matches!(results.next(), Some(LuaValue::Boolean(true)));
    let first = results.next().unwrap_or(LuaValue::Nil);
    if ok {
        return Ok(first);
    }
    if let Ok(state) = engine.state.try_borrow() {
        state.sched.charge(crate::cost::COPY_KEY_RAISED);
    }
    Err(Refused::Raised(first))
}

/// A number key's integer, as a C cast on x86-64 makes it: toward zero, the least 64-bit integer when out of range.
fn integer_of(number: f64) -> i64 {
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&number) {
        number as i64
    } else {
        i64::MIN
    }
}

/// A receiver's copy of a sent value. A SharedTable arrives as a new handle of the same table, at top level and inside a table
/// (measured, Studio 0.748, a playing client: Fire, and Invoke's arguments and results, deliver one `==` to the one sent but not
/// rawequal, its tostring different; lua/shared_table.rs `handle`).
pub fn rebuild(lua: &Lua, sent: &Sent) -> LuaResult<LuaValue> {
    Ok(match sent {
        Sent::Value(LuaValue::UserData(held)) if held.is::<super::shared_table::LSharedTable>() => {
            LuaValue::UserData(super::shared_table::fresh_handle(lua, held)?)
        }
        Sent::Value(LuaValue::UserData(held)) if held.is::<super::types::LInstanceHandle>() => super::types::copy_instance_handle(lua, held)?,
        Sent::Value(value) => value.clone(),
        Sent::Buffer(bytes) => LuaValue::Buffer(lua.create_buffer(&bytes[..])?),
        Sent::Table(entries) => {
            let indexed = entries.iter().filter(|(key, _)| matches!(key, Key::Index(_))).count();
            let table = lua.create_table_with_capacity(indexed, entries.len() - indexed)?;
            for (key, item) in entries {
                let item = rebuild(lua, item)?;
                match key {
                    Key::Index(index) => table.raw_set(*index, item)?,
                    Key::Name(name) => table.raw_set(lua.create_string(name)?, item)?,
                }
            }
            LuaValue::Table(table)
        }
    })
}

/// The engine value BindableEvent:Fire queues for an argument: a table or buffer as a [`Copied`] that every
/// receiver rebuilds, a SharedTable as one that gives every receiver a new handle ([`rebuild`]), a thread as nil, anything
/// else converted as usual. `height`: the call's frame (`self` and the arguments); `tally`: what the call's copy has made so
/// far ([`Tally`]).
pub fn sent_value(lua: &Lua, engine: &Engine, value: &LuaValue, height: usize, tally: &mut Tally) -> Result<Value, Refused> {
    match value {
        LuaValue::Table(_) | LuaValue::Buffer(_) => {
            let sent = capture(lua, engine, value, height, tally)?;
            let userdata = lua.create_userdata(Copied(Rc::new(sent))).map_err(|error| error.to_string())?;
            Ok(Value::Lua(Box::new(LuaValue::UserData(userdata))))
        }
        LuaValue::UserData(held) if held.is::<super::shared_table::LSharedTable>() => {
            super::shared_table::mark_value(value);
            let userdata = lua.create_userdata(Copied(Rc::new(Sent::Value(value.clone())))).map_err(|error| error.to_string())?;
            Ok(Value::Lua(Box::new(LuaValue::UserData(userdata))))
        }
        LuaValue::UserData(held) if held.is::<super::types::LInstanceHandle>() => {
            let userdata = lua.create_userdata(Copied(Rc::new(Sent::Value(value.clone())))).map_err(|error| error.to_string())?;
            Ok(Value::Lua(Box::new(LuaValue::UserData(userdata))))
        }
        LuaValue::Thread(_) => Ok(Value::Nil),
        other => Ok(super::convert::from_lua(other)),
    }
}

/// BindableFunction:Invoke's copy of its arguments and of the callback's results, and a ValueCurveKey's Value's: each
/// value's tables sitting at `height` (the call's frame: [`capture`]).
pub fn copied(lua: &Lua, engine: &Engine, values: LuaMultiValue, height: usize) -> Ret {
    let mut out = LuaMultiValue::with_capacity(values.len());
    let mut tally = Tally::default();
    for value in values.iter() {
        let sent = match capture(lua, engine, value, height, &mut tally) {
            Ok(sent) => sent,
            Err(refused) => return refused.raise(),
        };
        out.push_back(rebuild(lua, &sent)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{COPY_BYTES, COPY_VALUES, Tally, too_large};

    /// A copy's tally passes the guard's last value and refuses the next, and refuses a byte past [`COPY_BYTES`] with or
    /// without a value; server.luau's walk keeps the same guard
    #[test]
    fn a_copy_stops_one_past_rees_guard() {
        let mut tally = Tally { values: COPY_VALUES - 1, bytes: 0 };
        assert_eq!(tally.add(0), Ok(()));
        assert_eq!(tally.add(0), Err(too_large(COPY_VALUES, "values")));
        let mut tally = Tally::default();
        assert_eq!(tally.add(COPY_BYTES as usize), Ok(()));
        assert_eq!(tally.add_bytes(1), Err(too_large(COPY_BYTES, "bytes")));
        let mut tally = Tally::default();
        assert_eq!(tally.add(COPY_BYTES as usize + 1), Err(too_large(COPY_BYTES, "bytes")));
        let line = include_str!("server.luau").lines().find(|line| line.starts_with("local COPY_VALUES = ")).expect("server.luau's guard");
        assert_eq!(line.trim_end(), format!("local COPY_VALUES = {COPY_VALUES}"));
    }
}
