//! What BindableEvent:Fire and BindableFunction:Invoke pass on. Measured in a playing client (Studio 0.744,
//! tools/parity/probes/signals.luau, each capture taken twice):
//!
//! - Every table is copied: once when it is sent (so a change the sender makes afterwards does not arrive), and
//!   again for each receiver, so every handler, waiting thread and Invoke caller gets a table of its own. Shared
//!   subtables and a table passed twice become separate copies. Metatables and freezing are dropped.
//! - A table whose border (`#t`) is not 0 is copied as an array: indices 1 to `#t`, holes kept, every other key
//!   dropped (`{1, 2, Key = "v"}` arrives as `{1, 2}`, `{[1] = "a", [3] = "c"}` as `{"a"}`).
//! - Any other table is copied as a dictionary whose keys all become strings: a number truncated to a 64-bit
//!   integer (`1.5` -> `"1"`, `-9223372036854775808` when out of range), an Instance `<Instance> (Name)`, a
//!   function `<Function> (function: 0x...)`, a table `<Table> (table: 0x...)`, a vector `<Vector3> (1, 2, 3)`, an
//!   EnumItem `<token> (Enum.Material.Plastic)`, a buffer `<Buffer> (buffer: 0x...)`. Keys that end up equal keep the
//!   later one in iteration order. A boolean or thread key is an error, `Invalid table key type used`.
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
//!   new buffer with the same bytes; everything else (functions, Instances, datatypes, strings) as it is.
//!
//! Not measured: other userdata keys (named here by their typeof, e.g. `<Color3> (1, 0, 0)`), and whether a table
//! key's text calls its `__tostring` (it does here).

use mlua::prelude::*;
use std::ffi::c_void;
use std::rc::Rc;

use super::{Ret, raise};
use crate::{Engine, value::Value};

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

/// The stack height of a table found inside one at `height`: an array's item sits one slot above it, a dictionary's value
/// two (above its key); an error when that is above [`STACK_LIMIT`].
pub fn nested_height(height: usize, array: bool) -> Result<usize, String> {
    let nested = height + if array { 1 } else { 2 };
    if nested > STACK_LIMIT { Err(STACK_FULL.into()) } else { Ok(nested) }
}

/// A value as it is sent: an error when a table cannot be (cyclic, nested too deep, or a key of a kind that cannot be
/// sent). `height`: the call's frame, where the value's outermost table sits (the module's notes).
pub fn capture(lua: &Lua, engine: &Engine, value: &LuaValue, height: usize) -> Result<Sent, String> {
    Ok(capture_value(lua, engine, value, height, &mut Vec::new())?.unwrap_or(Sent::Value(LuaValue::Nil)))
}

/// `None`: a value that does not arrive (a thread).
fn capture_value(lua: &Lua, engine: &Engine, value: &LuaValue, height: usize, ancestors: &mut Vec<*const c_void>) -> Result<Option<Sent>, String> {
    Ok(Some(match value {
        LuaValue::Thread(_) => return Ok(None),
        LuaValue::Table(table) => capture_table(lua, engine, table, height, ancestors)?,
        LuaValue::Buffer(buffer) => Sent::Buffer(buffer.to_vec().into()),
        other => Sent::Value(other.clone()),
    }))
}

fn capture_table(lua: &Lua, engine: &Engine, table: &LuaTable, height: usize, ancestors: &mut Vec<*const c_void>) -> Result<Sent, String> {
    let pointer = table.to_pointer();
    if ancestors.contains(&pointer) {
        return Err("tables cannot be cyclic".into());
    }
    ancestors.push(pointer);
    let entries = capture_entries(lua, engine, table, height, ancestors);
    ancestors.pop();
    Ok(Sent::Table(entries?))
}

/// The copy of a value held by a table at `height`: a table is checked against the stack ([`nested_height`]) before it is
/// walked, so one too deep is refused before a cycle through it is found
fn capture_item(lua: &Lua, engine: &Engine, item: &LuaValue, height: usize, array: bool, ancestors: &mut Vec<*const c_void>) -> Result<Option<Sent>, String> {
    let height = if matches!(item, LuaValue::Table(_)) { nested_height(height, array)? } else { height };
    capture_value(lua, engine, item, height, ancestors)
}

fn capture_entries(lua: &Lua, engine: &Engine, table: &LuaTable, height: usize, ancestors: &mut Vec<*const c_void>) -> Result<Vec<(Key, Sent)>, String> {
    let mut entries = Vec::new();
    let length = table.raw_len();
    if length > 0 {
        for index in 1..=length {
            let item: LuaValue = table.raw_get(index).map_err(|error| error.to_string())?;
            if item.is_nil() {
                continue;
            }
            if let Some(copy) = capture_item(lua, engine, &item, height, true, ancestors)? {
                entries.push((Key::Index(index as i64), copy));
            }
        }
        return Ok(entries);
    }
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, item) = pair.map_err(|error| error.to_string())?;
        let key = key_name(lua, engine, &key)?;
        if let Some(copy) = capture_item(lua, engine, &item, height, false, ancestors)? {
            entries.push((Key::Name(key), copy));
        }
    }
    Ok(entries)
}

/// A dictionary key as the receiver sees it (always a string). HttpService:JSONEncode names its object keys the same
/// way (services/json.rs).
pub(crate) fn key_name(lua: &Lua, engine: &Engine, key: &LuaValue) -> Result<Vec<u8>, String> {
    let text = match key {
        LuaValue::String(text) => return Ok(text.as_bytes().to_vec()),
        LuaValue::Integer(number) => return Ok(number.to_string().into_bytes()),
        LuaValue::Number(number) => return Ok(integer_of(*number).to_string().into_bytes()),
        LuaValue::Boolean(_) | LuaValue::Thread(_) => return Err("Invalid table key type used".into()),
        LuaValue::Table(_) => "Table".to_string(),
        LuaValue::Function(_) => "Function".to_string(),
        LuaValue::Buffer(_) => "Buffer".to_string(),
        LuaValue::Vector(_) => "Vector3".to_string(),
        other => match super::typeof_value(engine, other).as_str() {
            "EnumItem" => "token".to_string(),
            name => name.to_string(),
        },
    };
    let shown = lua.globals().get::<LuaFunction>("tostring").and_then(|tostring| tostring.call::<LuaString>(key.clone()));
    let shown = shown.map(|shown| shown.to_string_lossy()).unwrap_or_default();
    Ok(format!("<{text}> ({shown})").into_bytes())
}

/// A number key's integer, as a C cast on x86-64 makes it: toward zero, the least 64-bit integer when out of range.
fn integer_of(number: f64) -> i64 {
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&number) {
        number as i64
    } else {
        i64::MIN
    }
}

/// A receiver's copy of a sent value.
pub fn rebuild(lua: &Lua, sent: &Sent) -> LuaResult<LuaValue> {
    Ok(match sent {
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
/// receiver rebuilds, a thread as nil, anything else converted as usual. `height`: the call's frame (`self` and the
/// arguments).
pub fn sent_value(lua: &Lua, engine: &Engine, value: &LuaValue, height: usize) -> Result<Value, String> {
    match value {
        LuaValue::Table(_) | LuaValue::Buffer(_) => {
            let sent = capture(lua, engine, value, height)?;
            let userdata = lua.create_userdata(Copied(Rc::new(sent))).map_err(|error| error.to_string())?;
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
    for value in values.iter() {
        let sent = match capture(lua, engine, value, height) {
            Ok(sent) => sent,
            Err(message) => return raise(message),
        };
        out.push_back(rebuild(lua, &sent)?);
    }
    Ok(out)
}
