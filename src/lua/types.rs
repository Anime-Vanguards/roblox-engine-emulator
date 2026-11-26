//! Engine datatypes as userdata (ports of `runtime/engine/enums.luau`, `signal.luau` and the
//! Font / Content / TweenInfo / Random / DateTime parts of `datatypes.luau`): interned
//! Enum / EnumItem, RBXScriptSignal / RBXScriptConnection and the values Lune does not provide.
//! Every type has Roblox's locked metatable and string errors.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use mlua::prelude::*;

use super::{
    Args, Flow, Ret, enum_item_value, enum_value, lock_metatable, none, raise, raise_at_caller, raise_plain,
    named, refs::{self, Hold}, signal_value, typeof_value, values, wrapped,
    cfunction::Kind::Library,
};
use crate::{
    Engine, InstanceId, ScriptError,
    reflection::{EnumId, EnumItemId},
    scheduler,
    signal::{ConnectionId, SignalId},
    value::{ContentValue, FontValue, Str},
};

pub struct LEnumItem(pub EnumItemId);
pub struct LEnum(pub EnumId);
pub struct LEnums;
/// A script's RBXScriptSignal: the signal's slot and the slot's generation when the value was made
/// (a freed instance's signal slots are reused: `signal_of`)
pub struct LSignal(pub SignalId, pub u32);
pub struct LConnection(pub ConnectionId);
pub struct LFont(pub RefCell<FontValue>);
/// A script's Content. One that holds an Object keeps it alive while the value lives, as Roblox's does (measured in a
/// 0.747 client: an EditableImage only a Content holds stays usable after the collector ran; the Luau wrapper of the
/// object is collected and `Object` answers a new one): the object is pinned until the value is collected.
pub struct LContent(pub ContentValue, #[allow(dead_code)] ObjectPin);

impl LContent {
    /// A Content value of `lua`'s engine (an Object's is pinned while the value lives).
    pub fn new(lua: &Lua, content: ContentValue) -> Self {
        let pin = match &content {
            ContentValue::Object(id) => ObjectPin::new(lua, *id),
            _ => ObjectPin(None),
        };
        LContent(content, pin)
    }
}

/// Keeps a Content's object pinned while the Content value lives. A pin or unpin that finds the engine's state in use
/// (the collector of the Luau heap drops a value wherever it runs) waits in [`PENDING_PINS`] until the next instance
/// collection, which applies it before it looks for garbage.
pub struct ObjectPin(Option<(Weak<Engine>, InstanceId)>);

thread_local! {
    static PENDING_PINS: RefCell<Vec<(Weak<Engine>, InstanceId, bool)>> = const { RefCell::new(Vec::new()) };
}

impl ObjectPin {
    fn new(lua: &Lua, id: InstanceId) -> Self {
        let Some(engine) = lua.app_data_ref::<Rc<Engine>>().map(|engine| Rc::downgrade(&engine)) else { return ObjectPin(None) };
        pin(&engine, id, true);
        ObjectPin(Some((engine, id)))
    }
}

impl Drop for ObjectPin {
    fn drop(&mut self) {
        if let Some((engine, id)) = self.0.take() {
            pin(&engine, id, false);
        }
    }
}

fn pin(engine: &Weak<Engine>, id: InstanceId, on: bool) {
    let Some(strong) = engine.upgrade() else { return };
    match strong.state.try_borrow_mut() {
        Ok(mut state) if on => state.dom.pin(id),
        Ok(mut state) => state.dom.unpin(id),
        Err(_) => PENDING_PINS.with(|pending| pending.borrow_mut().push((engine.clone(), id, on))),
    }
}

/// Applies the Content pins and unpins that waited for `engine`'s state (lua::raw::collect, before it looks for garbage).
pub fn apply_pending_pins(engine: &Engine) {
    let mine: Vec<(InstanceId, bool)> = PENDING_PINS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.is_empty() {
            return Vec::new();
        }
        let mut mine = Vec::new();
        pending.retain(|(owner, id, on)| {
            if std::ptr::eq(owner.as_ptr(), engine) {
                mine.push((*id, *on));
                false
            } else {
                owner.strong_count() > 0
            }
        });
        mine
    });
    if mine.is_empty() {
        return;
    }
    let mut state = engine.state.borrow_mut();
    for (id, on) in mine {
        if on {
            state.dom.pin(id);
        } else {
            state.dom.unpin(id);
        }
    }
}

/// A TweenInfo as a script value, its metatable locked as TweenInfo.new's are.
pub(crate) fn tween_info_value(lua: &Lua, info: LTweenInfo) -> LuaResult<LuaAnyUserData> {
    let userdata = lua.create_any_userdata(info)?;
    lock_metatable(lua, &userdata)?;
    Ok(userdata)
}

pub struct LTweenInfo {
    pub time: LuaValue,
    pub easing_style: EnumItemId,
    pub easing_direction: EnumItemId,
    pub repeat_count: LuaValue,
    pub reverses: bool,
    pub delay_time: LuaValue,
}

impl LTweenInfo {
    pub fn time(&self) -> f64 {
        number_of(&self.time)
    }

    pub fn delay(&self) -> f64 {
        number_of(&self.delay_time)
    }

    pub fn repeats(&self) -> f64 {
        number_of(&self.repeat_count)
    }
}

fn number_of(value: &LuaValue) -> f64 {
    match value {
        LuaValue::Number(n) => *n,
        LuaValue::Integer(n) => *n as f64,
        _ => 0.0,
    }
}

pub struct LDateTime(pub f64);

#[derive(Clone, Copy, Debug)]
pub struct RandomState {
    state: u64,
}

pub struct LRandom(pub Rc<Cell<RandomState>>);

/// Every Random a run made, in the order made, while it lives: Context.ReseedRandoms restarts each from
/// the run's seed and its place in that order (a Random made before a checkpoint is every run's).
#[derive(Default)]
pub struct RandomRegistry {
    made: u64,
    live: Vec<(u64, std::rc::Weak<Cell<RandomState>>)>,
}

impl RandomRegistry {
    fn add(&mut self, state: &Rc<Cell<RandomState>>) {
        if self.live.len() >= 64 && self.live.len().is_power_of_two() {
            self.live.retain(|(_, weak)| weak.strong_count() > 0);
        }
        self.live.push((self.made, Rc::downgrade(state)));
        self.made += 1;
    }

    /// Restarts every live Random from `seed` and the order it was made in (as Random.new(n) starts:
    /// `n` mixes both into 52 bits); how many it restarted
    pub fn reseed(&mut self, seed: f64) -> usize {
        self.live.retain(|(_, weak)| weak.strong_count() > 0);
        let base = seed.to_bits();
        for (made, weak) in &self.live {
            if let Some(state) = weak.upgrade() {
                state.set(seed_random(mix_seed(base, *made)));
            }
        }
        self.live.len()
    }
}

/// splitmix64 of the seed's bits and an ordinal, as a whole number below 2^52
fn mix_seed(base: u64, ordinal: u64) -> f64 {
    let mut z = base ^ ordinal.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 12) as f64
}

/// A Random that continues from `state`, registered in `engine` (a loopback run's remote argument: loopback/wire.rs)
pub(crate) fn random_with_state(engine: &Engine, state: RandomState) -> LRandom {
    new_random(engine, state)
}

fn new_random(engine: &Engine, state: RandomState) -> LRandom {
    let cell = Rc::new(Cell::new(state));
    engine.state.borrow_mut().sched.randoms.add(&cell);
    LRandom(cell)
}

fn key_text(key: &LuaValue) -> String {
    match key {
        LuaValue::String(s) => s.to_string_lossy(),
        LuaValue::Number(n) => n.to_string(),
        LuaValue::Integer(n) => n.to_string(),
        LuaValue::Boolean(b) => b.to_string(),
        LuaValue::Nil => "nil".into(),
        other => other.type_name().into(),
    }
}

/// A member a datatype of the engine's own lacks, as a playing client raises it (Studio 0.747,
/// tools/parity/probes/datatype_errors.luau): `Nope is not a valid member of DateTime` for a string key, without a
/// position, or, for TweenInfo and Font, `'Nope' is not a valid member of TweenInfo`, the key quoted and a number key too
/// (`'1'`); any other key (a number for the first kind) is `invalid argument #2 (string expected, got number)` from the
/// calling line. (The same rules as Lune's datatypes: lune-roblox datatypes::script_error::UnknownMembers.)
fn unknown_member<T>(engine: &Engine, key: &LuaValue, type_name: &str, quoted: bool) -> Result<T, Flow> {
    match key {
        LuaValue::String(text) if !quoted => raise(format!("{} is not a valid member of {type_name}", text.to_string_lossy())),
        LuaValue::String(text) => raise(format!("'{}' is not a valid member of {type_name}", text.to_string_lossy())),
        LuaValue::Number(n) if quoted => raise(format!("'{}' is not a valid member of {type_name}", lune_roblox::datatypes::types::format_g(*n, 14))),
        LuaValue::Integer(n) if quoted => raise(format!("'{n}' is not a valid member of {type_name}")),
        other => raise_at_caller(format!("invalid argument #2 (string expected, got {})", super::typeof_value(engine, other))),
    }
}

/// An assignment to a datatype of the engine's own, as a playing client raises it (Studio 0.747,
/// tools/parity/probes/datatype_errors.luau): `Time member of TweenInfo is read-only and cannot be assigned to` for one of
/// its `properties`, `Nope is not a valid member of TweenInfo` for any other key (a method, a number key), without a
/// position; a key of another type is `invalid argument #2 (string expected, got boolean)` from the calling line. (The
/// same rules as Lune's datatypes: lune-roblox datatypes::script_error::AssignMembers.)
fn assign_member<T>(engine: &Engine, key: &LuaValue, type_name: &str, properties: &[&str]) -> Result<T, Flow> {
    match key {
        LuaValue::String(text) => {
            let name = text.to_string_lossy();
            if properties.contains(&name.as_str()) {
                raise(format!("{name} member of {type_name} is read-only and cannot be assigned to"))
            } else {
                raise(format!("{name} is not a valid member of {type_name}"))
            }
        }
        LuaValue::Number(n) => raise(format!("{} is not a valid member of {type_name}", lune_roblox::datatypes::types::format_g(*n, 14))),
        LuaValue::Integer(n) => raise(format!("{n} is not a valid member of {type_name}")),
        other => raise_at_caller(format!("invalid argument #2 (string expected, got {})", super::typeof_value(engine, other))),
    }
}

/// The `__newindex` of a read-only datatype of the engine's own ([`assign_member`]).
fn read_only_members(lua: &Lua, engine: &Rc<Engine>, type_name: &'static str, properties: &'static [&'static str]) -> LuaResult<LuaFunction> {
    let e = engine.clone();
    wrapped(lua, engine, "__newindex", move |_, args| assign_member(&e, Args::new(args).get(1), type_name, properties))
}

fn read_only(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<LuaFunction> {
    wrapped(lua, engine, "__newindex", |_, args| {
        let args = Args::new(args);
        raise(format!("{} cannot be assigned to", key_text(args.get(1))))
    })
}

fn engine_of(lua: &Lua) -> Rc<Engine> {
    lua.app_data_ref::<Rc<Engine>>().expect("engine app data").clone()
}

fn string_of(lua: &Lua, value: &LuaValue) -> LuaResult<String> {
    Ok(match lua.coerce_string(value.clone())? {
        Some(s) => s.to_string_lossy(),
        None => match value {
            LuaValue::Nil => "nil".into(),
            LuaValue::Boolean(b) => b.to_string(),
            other => format!("{}", other.to_string().unwrap_or_default()),
        },
    })
}

/// Registers every engine datatype's metatable and returns the script libraries.
pub fn register(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    register_enums(lua, engine)?;
    register_signals(lua, engine)?;
    register_font(lua, engine)?;
    register_content(lua, engine)?;
    register_tween_info(lua, engine)?;
    register_random(lua, engine)?;
    register_date_time(lua, engine)?;
    register_instance_handle(lua, engine)?;
    super::curve_keys::register(lua, engine)?;
    super::shared_table::register(lua, engine)?;
    Ok(())
}

// InstanceHandle

/// An InstanceHandle: a reference to an instance, or to none, as a playing client has it (Studio 0.747 and 0.748, a client and
/// a server alike: tools/parity/probes/instance_handle.luau; 0.744: instance_attributes.luau).
///
/// - `InstanceHandle.new(instance)` makes one of any Instance (a parentless or destroyed one, a service, the DataModel), `new()`
///   and `new(nil)` one of none, extra arguments ignored; any other argument, a handle too, raises `InstanceHandle.new expects
///   an Instance argument`, without a position. The library is a frozen table holding `new` alone.
/// - Its one member is `Get`, which answers the instance (rawequal to it, a destroyed one too) or nil. Each read of `handle.Get`
///   gives a new function (`h.Get == h.Get` is false), a C function named Get whose wrong self raises Luau's own errors at the
///   calling line (`invalid argument #1 to 'Get' (InstanceHandle expected, got number)`, `missing argument #1 to 'Get'
///   (InstanceHandle expected)`); `:Get()` runs it through a `__namecall`. Any other string key, read, written or called as a
///   method, raises `Name is not a valid member of InstanceHandle` without a position, as does writing Get or a number key
///   (`1 is not a valid member ...`); a read with a key of another type raises `invalid argument #2 (string expected, got
///   number)` at the calling line.
/// - tostring is the instance's current Name (after a rename, an unparenting and a Destroy alike), `nil` for none. Two handles
///   are `==` when they hold the same instance or both none, never `==` an instance; each is a userdata of its own, never
///   rawequal to another (a table keyed by one does not find the next), and every engine value that hands one back (an
///   attribute's read, a Bindable's, a remote's, a SharedTable's) makes a new one.
///
/// A handle keeps its instance's userdata as its user value, so the instance lives while a script keeps the handle.
pub struct LInstanceHandle(pub Option<crate::InstanceId>);

/// The error of InstanceHandle.new given anything but an Instance or nil (measured, without a position)
const HANDLE_NEW_EXPECTS: &str = "InstanceHandle.new expects an Instance argument";

/// A new handle of `target`, or of none; a target the engine no longer has is none (what a receiver that lacks the instance
/// gets: loopback/wire.rs).
pub fn instance_handle(lua: &Lua, engine: &Engine, target: Option<crate::InstanceId>) -> LuaResult<LuaValue> {
    let target = target.filter(|id| engine.state.borrow().dom.get(*id).is_some());
    let handle = lua.create_any_userdata(LInstanceHandle(target))?;
    if let Some(id) = target {
        handle.set_user_value(super::instance_value(lua, engine, id)?)?;
    }
    Ok(LuaValue::UserData(handle))
}

/// The instance an InstanceHandle holds (`Some(None)` for a handle of none); None for any other value
pub fn instance_handle_of(value: &LuaValue) -> Option<Option<crate::InstanceId>> {
    match value {
        LuaValue::UserData(ud) => ud.borrow::<LInstanceHandle>().ok().map(|handle| handle.0),
        _ => None,
    }
}

/// Charges the work clock `seconds` for a handle's operation, and the profiler's account of the call (as `R.Charge` does)
fn charge_handle(engine: &Engine, seconds: f64) {
    let state = engine.state.borrow();
    state.sched.charge(seconds);
    if !state.sched.is_muted() {
        crate::boundary::add_cost(seconds);
    }
}

/// A new handle of what `held` (an InstanceHandle) holds, keeping its instance as `held` does: the copy a Bindable and a
/// SharedTable's read hand out
pub fn copy_instance_handle(lua: &Lua, held: &LuaAnyUserData) -> LuaResult<LuaValue> {
    let target = held.borrow::<LInstanceHandle>()?.0;
    let copy = lua.create_any_userdata(LInstanceHandle(target))?;
    if target.is_some() {
        copy.set_user_value(held.user_value::<LuaValue>()?)?;
    }
    Ok(LuaValue::UserData(copy))
}

#[inline(never)]
fn register_instance_handle(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    // Get, for `.` (a new C function each read: [`LInstanceHandle`]) and for `:` (the __namecall's, whose self always is a handle)
    let e = engine.clone();
    let get = super::create_boxed_fn(lua, Box::new(move |lua, args: LuaMultiValue| {
        let args = Args::new(args);
        super::encode(lua, match instance_handle_of(args.get(0)) {
            Some(Some(id)) if e.state.borrow().dom.get(id).is_some() => {
                charge_handle(&e, crate::cost::HANDLE_GET);
                super::instance_value(lua, &e, id).map(|value| values([value])).map_err(Flow::from)
            }
            Some(_) => Ok(values([LuaValue::Nil])),
            None if args.len() == 0 => raise_at_caller("missing argument #1 to 'Get' (InstanceHandle expected)"),
            None => raise_at_caller(format!("invalid argument #1 to 'Get' (InstanceHandle expected, got {})", typeof_value(&e, args.get(0)))),
        })
    }))?;
    let e = engine.clone();
    let dotted = get.clone();
    let index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        if let LuaValue::String(key) = args.get(1)
            && &*key.as_bytes() == b"Get"
        {
            return Ok(values([LuaValue::Function(super::cfunction::over_rust(lua, "Get", super::cfunction::Kind::Served, dotted.clone())?)]));
        }
        unknown_member(&e, args.get(1), "InstanceHandle", false)
    })?;
    let newindex = read_only_members(lua, engine, "InstanceHandle", &[])?;
    let e = engine.clone();
    let tostring = super::create_boxed_val(lua, Box::new(move |lua, this| {
        let state = e.state.borrow();
        let text = match instance_handle_of(&this).flatten().and_then(|id| state.dom.get(id)) {
            Some(instance) => instance.name.to_str_lossy().into_owned(),
            None => "nil".to_string(),
        };
        Ok(LuaValue::String(lua.create_string(text)?))
    }))?;
    let e = engine.clone();
    let eq = super::create_boxed_val2(lua, Box::new(move |_, a, b| {
        charge_handle(&e, crate::cost::HANDLE_EQ);
        Ok(LuaValue::Boolean(matches!((instance_handle_of(&a), instance_handle_of(&b)), (Some(a), Some(b)) if a == b)))
    }))?;
    lua.register_userdata_type::<LInstanceHandle>(|registry| {
        registry.add_meta_field("__type", "InstanceHandle");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", tostring);
        registry.add_meta_field("__eq", eq);
    })?;
    let sample = lua.create_any_userdata(LInstanceHandle(None))?;
    lock_metatable(lua, &sample)?;
    let calls = lua.create_table()?;
    calls.raw_set("Get", get)?;
    super::raw::set_table_namecall(lua, &sample, calls, "InstanceHandle", lua.create_table()?)?;

    let e = engine.clone();
    let library = lua.create_table()?;
    library.set(
        "new",
        wrapped(lua, engine, "new", move |lua, args| {
            let args = Args::new(args);
            let target = match args.get(0) {
                LuaValue::Nil => None,
                LuaValue::UserData(ud) => match super::raw::instance_of_ud(ud) {
                    Some(id) => Some(id),
                    None => return raise_plain(HANDLE_NEW_EXPECTS),
                },
                _ => return raise_plain(HANDLE_NEW_EXPECTS),
            };
            Ok(values([instance_handle(lua, &e, target)?]))
        })?,
    )?;
    library.set_readonly(true);
    engine.lua.borrow_mut().libraries.insert("InstanceHandle", library);
    Ok(())
}

// Enums

fn enum_item_of(value: &LuaValue) -> Option<EnumItemId> {
    match value {
        LuaValue::UserData(ud) => ud.borrow::<LEnumItem>().ok().map(|item| item.0),
        _ => None,
    }
}

#[inline(never)]
fn register_enums(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    // EnumItem
    let is_a = named(lua, "IsA", Library, super::create_boxed_val2(lua, Box::new(|lua, item, name| {
        let engine = engine_of(lua);
        let Some(item) = enum_item_of(&item) else { return Ok(LuaValue::Boolean(false)) };
        Ok(LuaValue::Boolean(match name {
            LuaValue::String(name) => engine.reflection.enum_item_type_name(item).as_bytes() == &*name.as_bytes(),
            _ => false,
        }))
    }))?)?;
    engine.lua.borrow_mut().enum_item_is_a = Some(is_a.clone());
    let e = engine.clone();
    let item_index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let Some(item) = enum_item_of(args.get(0)) else { return raise("invalid EnumItem") };
        let info = e.reflection.enum_item_info(item);
        let key = args.get(1);
        if let LuaValue::String(key) = key {
            match &*key.as_bytes() {
                b"Name" => return Ok(values([LuaValue::String(lua.create_string(&*info.name)?)])),
                b"Value" => return Ok(values([LuaValue::Number(info.value)])),
                b"EnumType" => return Ok(values([enum_value(lua, &e, info.enum_type)?])),
                b"IsA" => return Ok(values([LuaValue::Function(super::cfunction::copy(lua, &is_a)?)])),
                _ => {}
            }
        }
        raise(format!(
            "{} is not a valid member of \"{}\"",
            key_text(key),
            e.reflection.enum_item_path(item)
        ))
    })?;
    let item_tostring = super::lua_fn(lua, |lua, item: LuaAnyUserData| {
        let engine = engine_of(lua);
        Ok(engine.reflection.enum_item_path(item.borrow::<LEnumItem>()?.0))
    })?;
    let newindex = read_only(lua, engine)?;
    let item_newindex = wrapped(lua, engine, "__newindex", |_, args| {
        let args = Args::new(args);
        raise(format!("{} is not a valid member of EnumItem", key_text(args.get(1))))
    })?;
    lua.register_userdata_type::<LEnumItem>(|registry| {
        registry.add_meta_field("__type", "EnumItem");
        registry.add_meta_field("__index", item_index);
        registry.add_meta_field("__newindex", item_newindex);
        registry.add_meta_field("__tostring", item_tostring);
    })?;

    // Enum
    let members = lua.create_table()?;
    members.set(
        "GetEnumItems",
        named(lua, "GetEnumItems", Library, super::lua_fn(lua, |lua, this: LuaValue| {
            let engine = engine_of(lua);
            let LuaValue::UserData(ud) = this else { return Err(LuaError::runtime("Expected ':' not '.' calling member function GetEnumItems")) };
            let id = ud.borrow::<LEnum>()?.0;
            let items = engine.reflection.enums[id as usize].items.clone();
            let table = lua.create_table_with_capacity(items.len(), 0)?;
            for item in items {
                table.raw_push(enum_item_value(lua, &engine, item)?)?;
            }
            Ok(table)
        })?)?,
    )?;
    members.set(
        "FromName",
        named(lua, "FromName", Library, super::lua_fn(lua, |lua, (this, name): (LuaAnyUserData, LuaValue)| {
            let engine = engine_of(lua);
            let id = this.borrow::<LEnum>()?.0;
            let LuaValue::String(name) = name else { return Ok(LuaValue::Nil) };
            match engine.reflection.enums[id as usize].by_name.get(&*name.to_string_lossy()) {
                Some(&item) => enum_item_value(lua, &engine, item),
                None => Ok(LuaValue::Nil),
            }
        })?)?,
    )?;
    members.set(
        "FromValue",
        named(lua, "FromValue", Library, super::lua_fn(lua, |lua, (this, value): (LuaAnyUserData, LuaValue)| {
            let engine = engine_of(lua);
            let id = this.borrow::<LEnum>()?.0;
            let number = number_of(&value);
            if number.fract() != 0.0 {
                return Ok(LuaValue::Nil);
            }
            match engine.reflection.enums[id as usize].by_value.get(&(number as i64)) {
                Some(&item) => enum_item_value(lua, &engine, item),
                None => Ok(LuaValue::Nil),
            }
        })?)?,
    )?;
    engine.lua.borrow_mut().enum_members = Some(members.clone());
    let e = engine.clone();
    let enum_index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let LuaValue::UserData(ud) = args.get(0) else { return raise("invalid Enum") };
        let id = ud.borrow::<LEnum>()?.0;
        let info = &e.reflection.enums[id as usize];
        if let LuaValue::String(key) = args.get(1) {
            if let Some(&item) = info.by_name.get(&*key.to_string_lossy()) {
                return Ok(values([enum_item_value(lua, &e, item)?]));
            }
            let member: LuaValue = members.raw_get(key.clone())?;
            if !member.is_nil() {
                return Ok(values([member]));
            }
        }
        raise(format!("{} is not a valid member of \"Enum.{}\"", key_text(args.get(1)), info.name))
    })?;
    let enum_tostring = super::lua_fn(lua, |lua, this: LuaAnyUserData| {
        let engine = engine_of(lua);
        Ok(engine.reflection.enums[this.borrow::<LEnum>()?.0 as usize].name.to_string())
    })?;
    lua.register_userdata_type::<LEnum>(|registry| {
        registry.add_meta_field("__type", "Enum");
        registry.add_meta_field("__index", enum_index);
        registry.add_meta_field("__newindex", newindex.clone());
        registry.add_meta_field("__tostring", enum_tostring);
    })?;

    // The Enum global
    let get_enums = named(lua, "GetEnums", Library, super::lua_fn(lua, |lua, ()| {
        let engine = engine_of(lua);
        let table = lua.create_table()?;
        for id in 0..engine.reflection.enums.len() {
            table.raw_push(enum_value(lua, &engine, id as EnumId)?)?;
        }
        Ok(table)
    })?)?;
    let e = engine.clone();
    let global_index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        if let LuaValue::String(key) = args.get(1) {
            if &*key.as_bytes() == b"GetEnums" {
                return Ok(values([LuaValue::Function(get_enums.clone())]));
            }
            if let Some(id) = e.reflection.enum_id(&key.to_string_lossy()) {
                return Ok(values([enum_value(lua, &e, id)?]));
            }
        }
        raise(format!("{} is not a valid member of \"Enum\"", key_text(args.get(1))))
    })?;
    let global_tostring = super::create_boxed_unit(lua, Box::new(|lua| Ok(LuaValue::String(lua.create_string("Enums")?))))?;
    lua.register_userdata_type::<LEnums>(|registry| {
        registry.add_meta_field("__type", "Enums");
        registry.add_meta_field("__index", global_index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", global_tostring);
    })?;

    let global = lua.create_any_userdata(LEnums)?;
    lock_metatable(lua, &global)?;
    engine.lua.borrow_mut().enums_global = Some(global);
    if let Some(&first) = engine.reflection.enum_items.first().map(|_| &0) {
        if let LuaValue::UserData(item) = enum_item_value(lua, engine, first)? {
            lock_metatable(lua, &item)?;
        }
        if let LuaValue::UserData(item) = enum_value(lua, engine, 0)? {
            lock_metatable(lua, &item)?;
        }
    }
    Ok(())
}

// Signals

/// The signal of a script's RBXScriptSignal value (None: not one); `Some(None)` when the signal's
/// instance was freed since (nothing fires it any more).
fn signal_of(engine: &Engine, value: &LuaValue) -> Option<Option<SignalId>> {
    match value {
        LuaValue::UserData(ud) => {
            let signal = ud.borrow::<LSignal>().ok()?;
            Some(engine.state.borrow().signals.is_current(signal.0, signal.1).then_some(signal.0))
        }
        _ => None,
    }
}

/// `Signal.Wait()`: "missing argument #1 to 'Wait' (RBXScriptSignal expected)" at the caller (measured,
/// tools/parity/probes/errors.luau); with a value that is no signal, the invalid-argument text Connect gives (measured,
/// before the yieldability check: tools/parity/probes/yield_refused.luau)
fn wait_misuse<T>(engine: &Engine, value: &LuaValue) -> Result<T, Flow> {
    if value.is_nil() {
        return super::raise_at_caller("missing argument #1 to 'Wait' (RBXScriptSignal expected)");
    }
    super::raise_at_caller(format!("invalid argument #1 to 'Wait' (RBXScriptSignal expected, got {})", typeof_value(engine, value)))
}

/// RunService.RenderStepped:Wait on a server: "RenderStepped event can only be used from local scripts", as Connect raises
/// there (measured, a 0.747 server's Script: tools/parity/probes/arg_checks.luau)
fn render_stepped_on_server(engine: &Engine, signal: Option<SignalId>) -> Result<(), Flow> {
    if let Some(id) = signal
        && engine.options.server()
        && engine.state.borrow().signals.get(id).label.as_bytes() == b"RunService.RenderStepped"
    {
        return raise("RenderStepped event can only be used from local scripts");
    }
    Ok(())
}

fn connect(lua: &Lua, engine: &Rc<Engine>, args: LuaMultiValue, once: bool) -> Ret {
    connect_as(lua, engine, args, once, false)
}

/// Connect, Once and ConnectParallel (`parallel`: a handler of the signal's parallel phase, which only an Actor's script may add).
fn connect_as(lua: &Lua, engine: &Rc<Engine>, args: LuaMultiValue, once: bool, parallel: bool) -> Ret {
    let args = Args::new(args);
    let Some(signal) = signal_of(engine, args.get(0)) else {
        // `Signal.Connect(f)`: "invalid argument #1 to 'Connect' (RBXScriptSignal expected, got function)" at the caller
        // (measured, tools/parity/probes/errors.luau)
        let verb = if parallel { "ConnectParallel" } else if once { "Once" } else { "Connect" };
        return super::raise_at_caller(format!(
            "invalid argument #1 to '{verb}' (RBXScriptSignal expected, got {})",
            typeof_value(engine, args.get(0))
        ));
    };
    // ConnectParallel outside an Actor, before the signal is looked at (measured, a 0.747 server's Script and a playing client,
    // tools/parity/probes/arg_checks.luau: RenderStepped's and Heartbeat's alike): "Event.ConnectParallel: The calling script must
    // be rooted under an Actor.", or where no script calls (a client's job) "Function Event.ConnectParallel can only be used
    // inside scripts"
    if parallel && !super::actors::caller_in_actor(lua) {
        // SAFETY: the running thread's state, valid while the callback runs
        return raise(if unsafe { super::script_of_thread(lua.state()) }.is_some() {
            "Event.ConnectParallel: The calling script must be rooted under an Actor."
        } else {
            "Function Event.ConnectParallel can only be used inside scripts"
        });
    }
    // A server's scripts may read RunService.RenderStepped but not connect to it (Connect and Once measured, a Studio 0.744
    // server's game script: tools/parity/probes/run_service_sides.luau; Wait raises the same, a 0.747 server's Script:
    // arg_checks.luau). PreRender and BindToRenderStep are taken there and never run, as here.
    if let Some(id) = signal {
        if engine.options.server() && engine.state.borrow().signals.get(id).label.as_bytes() == b"RunService.RenderStepped" {
            return raise("RenderStepped event can only be used from local scripts");
        }
        // (the events only Roblox's own scripts connect to: reflection.rs CAPABILITY_EVENTS)
        {
            let state = engine.state.borrow();
            let label = state.signals.get(id).label.as_bytes();
            if let Some(&(_, event, capability)) = crate::reflection::CAPABILITY_EVENTS
                .iter()
                .find(|&&(class, event, _)| label.len() == class.len() + 1 + event.len() && label.starts_with(class.as_bytes()) && label.ends_with(event.as_bytes()))
            {
                return raise(format!("The current thread cannot connect '{event}' (lacking capability {capability})"));
            }
        }
        // (a loopback run refuses the other side's remote event)
        crate::loopback::remotes::connecting(engine, id)?;
    }
    let LuaValue::Function(callback) = args.get(1).clone() else {
        // Roblox raises nothing: it logs the message and returns a connection that is not connected, which firing the
        // signal skips (measured in a playing client, Studio 0.744: tools/parity/probes/signals.luau)
        engine.output("error", "Attempt to connect failed: Passed value is not a function", None);
        super::cfunction::charge_error_work(engine, crate::cost::CONNECT_FAILED_LOGGED);
        let userdata = lua.create_any_userdata(LConnection(ConnectionId::DISCONNECTED))?;
        return Ok(values([LuaValue::UserData(userdata)]));
    };
    let noted = crate::census::active(engine).then(|| callback.clone());
    engine.state.borrow().sched.charge(crate::cost::CONNECT);
    if crate::boundary::active() && !engine.state.borrow().sched.is_muted() {
        crate::boundary::enter(lua.state());
        let name = signal.map_or_else(|| "signal".to_string(), |signal| engine.state.borrow().signals.get(signal).name.to_str_lossy().into_owned());
        let verb = if parallel { "ConnectParallel" } else if once { "Once" } else { "Connect" };
        crate::boundary::record_named(crate::boundary::Kind::Event, "RBXScriptSignal", &format!("{verb}({name})"), crate::cost::CONNECT);
    }
    let connection = match signal {
        Some(signal) => engine.state.borrow_mut().signals.connect_with(signal, callback, once, parallel),
        None => ConnectionId::DISCONNECTED,
    };
    if let Some(callback) = noted {
        crate::census::connected(lua, connection, &callback);
    }
    // (a loopback run's remote hands the events that waited for a first connection to this one)
    if let Some(signal) = signal
        && connection != ConnectionId::DISCONNECTED
    {
        crate::loopback::remotes::connected(engine, signal, connection);
    }
    // (a profile keeps where it was connected: its handlers' dispatch and runs are reported by that site)
    if crate::profiler::attribution::on()
        && connection != ConnectionId::DISCONNECTED
        && let Some(signal) = signal
    {
        let label = engine.state.borrow().signals.get(signal).label.to_str_lossy().into_owned();
        crate::profiler::attribution::connected(connection.slot(), &label);
    }
    let userdata = lua.create_any_userdata(LConnection(connection))?;
    Ok(values([LuaValue::UserData(userdata)]))
}

#[inline(never)]
fn register_signals(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let members = lua.create_table()?;
    let e = engine.clone();
    let connect_fn = wrapped(lua, engine, "Connect", move |lua, args| connect(lua, &e, args, false))?;
    let e = engine.clone();
    let once_fn = wrapped(lua, engine, "Once", move |lua, args| connect(lua, &e, args, true))?;
    let e = engine.clone();
    let wait_fn = wrapped(lua, engine, "Wait", move |lua, args| {
        let args = Args::new(args);
        let Some(signal) = signal_of(&e, args.get(0)) else {
            return wait_misuse(&e, args.get(0));
        };
        render_stepped_on_server(&e, signal)?;
        // (where the thread cannot yield: raised before the thread waits on the signal, lua/mod.rs NOT_YIELDABLE)
        let thread = lua.current_thread();
        super::refuse_unyieldable(&thread, super::NOT_YIELDABLE_WAIT)?;
        if let Some(signal) = signal {
            refs::admit(&e, Hold::Wait, "Wait")?;
            e.state.borrow_mut().signals.wait(signal, thread);
        }
        Err(Flow::Yield)
    })?;
    members.set("Connect", connect_fn.clone())?;
    let e = engine.clone();
    let connect_parallel_fn = wrapped(lua, engine, "ConnectParallel", move |lua, args| connect_as(lua, &e, args, false, true))?;
    members.set("ConnectParallel", connect_parallel_fn.clone())?;
    members.set("Once", once_fn)?;
    members.set("Wait", wait_fn.clone())?;
    // The lowercase names a client answers (measured: connect, wait and connectParallel, not once;
    // tools/parity/probes/camelcase.luau)
    members.set("connect", connect_fn)?;
    members.set("wait", wait_fn)?;
    members.set("connectParallel", connect_parallel_fn)?;
    engine.lua.borrow_mut().signal_members = Some(members.clone());
    // (a method read as a value is a new function at every read, as a client's: cfunction::copy)
    let index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let member: LuaValue = match args.get(1) {
            LuaValue::String(key) => members.raw_get(key.clone())?,
            _ => LuaValue::Nil,
        };
        match member {
            LuaValue::Function(method) => Ok(values([LuaValue::Function(super::cfunction::copy(lua, &method)?)])),
            LuaValue::Nil => raise(format!("{} is not a valid member of RBXScriptSignal", key_text(args.get(1)))),
            member => Ok(values([member])),
        }
    })?;
    let tostring = super::lua_fn(lua, |lua, this: LuaAnyUserData| {
        let engine = engine_of(lua);
        let signal = this.borrow::<LSignal>()?;
        let state = engine.state.borrow();
        Ok(match state.signals.is_current(signal.0, signal.1) {
            true => format!("Signal {}", state.signals.get(signal.0).name),
            false => "Signal".to_string(),
        })
    })?;
    let newindex = read_only(lua, engine)?;
    lua.register_userdata_type::<LSignal>(|registry| {
        registry.add_meta_field("__type", "RBXScriptSignal");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex.clone());
        registry.add_meta_field("__tostring", tostring);
    })?;

    let connection_members = lua.create_table()?;
    let disconnect = named(lua, "Disconnect", Library, super::create_boxed_void_val(lua, Box::new(|lua, this| {
        let engine = engine_of(lua);
        if let LuaValue::UserData(ud) = this {
            if let Ok(connection) = ud.borrow::<LConnection>() {
                let mut state = engine.state.borrow_mut();
                state.sched.charge(crate::cost::DISCONNECT);
                if crate::boundary::active() && !state.sched.is_muted() {
                    crate::boundary::enter(lua.state());
                    crate::boundary::record_named(crate::boundary::Kind::Event, "RBXScriptConnection", "Disconnect", crate::cost::DISCONNECT);
                }
                state.signals.disconnect(connection.0);
            }
        }
        Ok(())
    }))?)?;
    connection_members.set("Disconnect", disconnect.clone())?;
    connection_members.set("disconnect", disconnect)?;
    engine.lua.borrow_mut().connection_members = Some(connection_members.clone());
    let e = engine.clone();
    let connection_index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let LuaValue::UserData(ud) = args.get(0) else { return raise("invalid connection") };
        let connection = ud.borrow::<LConnection>()?.0;
        if let LuaValue::String(key) = args.get(1) {
            // (and the lowercase `connected` a client answers: tools/parity/probes/camelcase.luau)
            if matches!(&*key.as_bytes(), b"Connected" | b"connected") {
                let connected = e.state.borrow().signals.is_connected(connection);
                return Ok(values([LuaValue::Boolean(connected)]));
            }
            if let LuaValue::Function(method) = connection_members.raw_get::<LuaValue>(key.clone())? {
                return Ok(values([LuaValue::Function(super::cfunction::copy(lua, &method)?)]));
            }
        }
        raise(format!("{} is not a valid member of RBXScriptConnection", key_text(args.get(1))))
    })?;
    let connection_tostring = super::create_boxed_unit(lua, Box::new(|lua| Ok(LuaValue::String(lua.create_string("Connection")?))))?;
    lua.register_userdata_type::<LConnection>(|registry| {
        registry.add_meta_field("__type", "RBXScriptConnection");
        registry.add_meta_field("__index", connection_index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", connection_tostring);
    })?;
    // Method calls (`signal:Connect(f)`, `connection:Disconnect()`) dispatch in C to the unwrapped
    // functions (lua/raw.rs table_namecall); indexing keeps the wrapped ones.
    let signal_calls = lua.create_table()?;
    let e = engine.clone();
    let raw_connect = super::create_boxed_fn(lua, Box::new(move |lua, args| super::encode(lua, connect(lua, &e, args, false))))?;
    let e = engine.clone();
    let raw_once = super::create_boxed_fn(lua, Box::new(move |lua, args| super::encode(lua, connect(lua, &e, args, true))))?;
    let e = engine.clone();
    let raw_wait = super::create_boxed_fn(lua, Box::new(move |lua, args: LuaMultiValue| {
        let args = Args::new(args);
        let outcome = match signal_of(&e, args.get(0)) {
            Some(signal) => {
                let thread = lua.current_thread();
                render_stepped_on_server(&e, signal).and_then(|()| super::refuse_unyieldable(&thread, super::NOT_YIELDABLE_WAIT)).and_then(|()| match signal {
                    Some(signal) => refs::admit(&e, Hold::Wait, "Wait").and_then(|()| {
                        e.state.borrow_mut().signals.wait(signal, thread);
                        Err(Flow::Yield)
                    }),
                    None => Err(Flow::Yield),
                })
            }
            None => wait_misuse(&e, args.get(0)),
        };
        super::encode(lua, outcome)
    }))?;
    let e = engine.clone();
    let raw_connect_parallel = super::create_boxed_fn(lua, Box::new(move |lua, args| super::encode(lua, connect_as(lua, &e, args, false, true))))?;
    for (name, function) in [
        ("Connect", &raw_connect),
        ("ConnectParallel", &raw_connect_parallel),
        ("connectParallel", &raw_connect_parallel),
        ("connect", &raw_connect),
        ("Once", &raw_once),
        ("Wait", &raw_wait),
        ("wait", &raw_wait),
    ] {
        signal_calls.raw_set(name, function.clone())?;
    }
    let connection_calls = lua.create_table()?;
    let members = engine.lua.borrow().connection_members.clone().expect("connection members");
    connection_calls.raw_set("Disconnect", members.raw_get::<LuaValue>("Disconnect")?)?;
    connection_calls.raw_set("disconnect", members.raw_get::<LuaValue>("disconnect")?)?;
    let connection_properties = lua.create_table()?;
    connection_properties.raw_set("Connected", "boolean")?;
    connection_properties.raw_set("connected", "boolean")?;

    // Lock both metatables
    let signal = engine.state.borrow_mut().signals.create(Str::from("Probe"));
    if let LuaValue::UserData(sample) = signal_value(lua, engine, signal)? {
        lock_metatable(lua, &sample)?;
        super::raw::set_table_namecall(lua, &sample, signal_calls, "RBXScriptSignal", lua.create_table()?)?;
    }
    let probe = super::create_boxed_void(lua, Box::new(|_| Ok(())))?;
    let connection = engine.state.borrow_mut().signals.connect(signal, probe, false);
    let sample = lua.create_any_userdata(LConnection(connection))?;
    lock_metatable(lua, &sample)?;
    super::raw::set_table_namecall(lua, &sample, connection_calls, "RBXScriptConnection", connection_properties)?;
    engine.state.borrow_mut().signals.disconnect(connection);
    Ok(())
}

/// A standalone signal (Mouse events, BindableEvent.Event before an instance owns it).
pub fn new_signal(engine: &Engine, name: &str) -> SignalId {
    engine.state.borrow_mut().signals.create(Str::from(name))
}

// Font

fn font_of(value: &LuaValue) -> Option<FontValue> {
    match value {
        LuaValue::UserData(ud) => ud.borrow::<LFont>().ok().map(|f| f.0.borrow().clone()),
        _ => None,
    }
}

fn coerce_named_enum(lua: &Lua, engine: &Engine, enum_name: &str, value: &LuaValue) -> Option<EnumItemId> {
    let id = engine.reflection.enum_id(enum_name)?;
    super::convert::coerce_enum(lua, engine, id, value)
}

fn make_font(lua: &Lua, font: FontValue) -> LuaResult<LuaValue> {
    Ok(LuaValue::UserData(lua.create_any_userdata(LFont(RefCell::new(font)))?))
}

#[inline(never)]
fn register_font(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let e = engine.clone();
    let index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let Some(font) = font_of(args.get(0)) else { return raise("invalid Font") };
        if let LuaValue::String(key) = args.get(1) {
            match &*key.as_bytes() {
                b"Family" => return Ok(values([LuaValue::String(lua.create_string(font.family.as_bytes())?)])),
                b"Weight" => return Ok(values([enum_item_value(lua, &e, font.weight)?])),
                b"Style" => return Ok(values([enum_item_value(lua, &e, font.style)?])),
                b"Bold" => {
                    return Ok(values([LuaValue::Boolean(e.reflection.enum_item_info(font.weight).value >= 600.0)]));
                }
                _ => {}
            }
        }
        unknown_member(&e, args.get(1), "Font", true)
    })?;
    let e = engine.clone();
    let newindex = wrapped(lua, engine, "__newindex", move |lua, args| {
        let args = Args::new(args);
        let LuaValue::UserData(ud) = args.get(0) else { return raise("invalid Font") };
        let font = ud.borrow::<LFont>()?;
        let value = args.get(2);
        let key = match args.get(1) {
            LuaValue::String(key) => key.to_string_lossy(),
            other @ (LuaValue::Number(_) | LuaValue::Integer(_)) => key_text(other),
            // (Studio 0.747, tools/parity/probes/datatype_errors.luau: the key's Luau check from the calling line)
            other => return raise_at_caller(format!("invalid argument #2 (string expected, got {})", typeof_value(&e, other))),
        };
        let mut data = font.0.borrow_mut();
        match (key.as_str(), value) {
            ("Family", LuaValue::String(family)) => data.family = Str::new(&family.as_bytes()),
            // (a number is its text, as a client takes `font.Family = 1`: Studio 0.747, datatype_errors.luau)
            ("Family", number @ (LuaValue::Number(_) | LuaValue::Integer(_))) => data.family = Str::new(string_of(lua, number)?.as_bytes()),
            ("Weight", _) => {
                if let Some(item) = coerce_named_enum(lua, &e, "FontWeight", value) {
                    data.weight = item;
                }
            }
            ("Style", _) => {
                if let Some(item) = coerce_named_enum(lua, &e, "FontStyle", value) {
                    data.style = item;
                }
            }
            ("Bold", _) => {
                let name = if matches!(value, LuaValue::Nil | LuaValue::Boolean(false)) { "Regular" } else { "Bold" };
                if let Some(item) = e.reflection.enum_item("FontWeight", name) {
                    data.weight = item;
                }
            }
            _ => return raise(format!("{key} is not a valid member of Font")),
        }
        Ok(none())
    })?;
    let tostring = super::lua_fn(lua, |lua, this: LuaAnyUserData| {
        let engine = engine_of(lua);
        let font = this.borrow::<LFont>()?;
        let font = font.0.borrow();
        Ok(format!(
            "Font {{ Family = {}, Weight = {}, Style = {} }}",
            font.family,
            engine.reflection.enum_item_info(font.weight).name,
            engine.reflection.enum_item_info(font.style).name
        ))
    })?;
    let eq = super::create_boxed_val2(lua, Box::new(|_, a, b| Ok(LuaValue::Boolean(matches!((font_of(&a), font_of(&b)), (Some(a), Some(b)) if a == b)))))?;
    lua.register_userdata_type::<LFont>(|registry| {
        registry.add_meta_field("__type", "Font");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", tostring);
        registry.add_meta_field("__eq", eq);
    })?;

    let library = lua.create_table()?;
    // The constructors check their arguments as a playing client does (Studio 0.747, tools/parity/probes/datatype_errors.luau):
    // the family or id as its Luau checks do, from the calling line (`missing argument #1 to 'new' (string expected)`, a
    // number a string's text, `invalid argument #1 to 'fromId' (number expected, got string)`), the weight and style as
    // its enum casts do, without a position (`Invalid value for enum 'FontWeight'`, `Invalid type 'KeyCode' for enum
    // 'FontWeight'` for another enum's item); nil is the default
    let enum_arg = |lua: &Lua, engine: &Engine, value: &LuaValue, enum_name: &str, default: &str| -> Result<EnumItemId, Flow> {
        if value.is_nil() {
            return Ok(engine.reflection.enum_item(enum_name, default).unwrap_or(0));
        }
        coerce_named_enum(lua, engine, enum_name, value).ok_or_else(|| match enum_item_of(value) {
            Some(item) => Flow::Error(ScriptError::plain(format!(
                "Invalid type '{}' for enum '{enum_name}'",
                engine.reflection.enum_item_type_name(item)
            ))),
            None => Flow::Error(ScriptError::plain(format!("Invalid value for enum '{enum_name}'"))),
        })
    };
    let weight = move |lua: &Lua, engine: &Engine, value: &LuaValue, _: &str| enum_arg(lua, engine, value, "FontWeight", "Regular");
    let style = move |lua: &Lua, engine: &Engine, value: &LuaValue, _: &str| enum_arg(lua, engine, value, "FontStyle", "Normal");
    let family_arg = |lua: &Lua, engine: &Engine, args: &Args, function: &str| -> Result<String, Flow> {
        match args.get(0) {
            _ if args.is_empty() => raise_at_caller(format!("missing argument #1 to '{function}' (string expected)")),
            value @ (LuaValue::String(_) | LuaValue::Number(_) | LuaValue::Integer(_)) => Ok(string_of(lua, value)?),
            value => raise_at_caller(format!("invalid argument #1 to '{function}' (string expected, got {})", typeof_value(engine, value))),
        }
    };
    let e = engine.clone();
    library.set(
        "new",
        wrapped(lua, engine, "new", move |lua, args| {
            let args = Args::new(args);
            let family = family_arg(lua, &e, &args, "new")?;
            let font = FontValue {
                family: Str::new(family.as_bytes()),
                weight: weight(lua, &e, args.get(1), "new")?,
                style: style(lua, &e, args.get(2), "new")?,
            };
            Ok(values([make_font(lua, font)?]))
        })?,
    )?;
    let e = engine.clone();
    library.set(
        "fromEnum",
        wrapped(lua, engine, "fromEnum", move |lua, args| {
            let args = Args::new(args);
            // Only an item of Enum.Font: another enum's item raises `Invalid type 'Material' for enum 'Font'`, anything
            // else (a number, a name, nil, nothing) `Invalid value for enum 'Font'`, and Enum.Font.Unknown `Enum.Font.Unknown
            // can't be passed to Font.fromEnum` from the calling line (Studio 0.747, tools/parity/probes/datatype_edges.luau
            // and datatype_edges_more.luau `font-`)
            let item = match args.get(0) {
                LuaValue::UserData(ud) => ud.borrow::<LEnumItem>().ok().map(|item| item.0),
                _ => None,
            };
            let Some(item) = item else {
                return raise_plain("Invalid value for enum 'Font'");
            };
            let info = e.reflection.enum_item_info(item);
            let enum_name = &e.reflection.enums[info.enum_type as usize].name;
            if &**enum_name != "Font" {
                return raise_plain(format!("Invalid type '{enum_name}' for enum 'Font'"));
            }
            if &*info.name == "Unknown" {
                return raise_at_caller("Enum.Font.Unknown can't be passed to Font.fromEnum");
            }
            Ok(values([make_font(lua, font_from_enum(&e, item))?]))
        })?,
    )?;
    let e = engine.clone();
    library.set(
        "fromName",
        wrapped(lua, engine, "fromName", move |lua, args| {
            let args = Args::new(args);
            let name = family_arg(lua, &e, &args, "fromName")?;
            let font = FontValue {
                family: Str::from(format!("rbxasset://fonts/families/{name}.json")),
                weight: weight(lua, &e, args.get(1), "fromName")?,
                style: style(lua, &e, args.get(2), "fromName")?,
            };
            Ok(values([make_font(lua, font)?]))
        })?,
    )?;
    let e = engine.clone();
    library.set(
        "fromId",
        wrapped(lua, engine, "fromId", move |lua, args| {
            let args = Args::new(args);
            let id = match args.get(0) {
                _ if args.is_empty() => return raise_at_caller("missing argument #1 to 'fromId' (number expected)"),
                value @ (LuaValue::Number(_) | LuaValue::Integer(_)) => string_of(lua, value)?,
                value @ LuaValue::String(_) if lua.coerce_number(value.clone()).ok().flatten().is_some() => string_of(lua, value)?,
                value => return raise_at_caller(format!("invalid argument #1 to 'fromId' (number expected, got {})", typeof_value(&e, value))),
            };
            let font = FontValue {
                family: Str::from(format!("rbxassetid://{id}")),
                weight: weight(lua, &e, args.get(1), "fromId")?,
                style: style(lua, &e, args.get(2), "fromId")?,
            };
            Ok(values([make_font(lua, font)?]))
        })?,
    )?;
    engine.lua.borrow_mut().libraries.insert("Font", library);
    if let LuaValue::UserData(sample) = make_font(lua, FontValue { family: Str::default(), weight: 0, style: 0 })? {
        lock_metatable(lua, &sample)?;
    }
    Ok(())
}

/// Font.fromEnum: the face data/font-enums.json records for an Enum.Font item (LegacyArial
/// otherwise).
pub fn font_from_enum(engine: &Engine, item: EnumItemId) -> FontValue {
    let reflection = &engine.reflection;
    let name = &reflection.enum_item_info(item).name;
    match reflection.font_enum_faces.get(&**name) {
        Some((family, weight, style)) => FontValue {
            family: Str::from(&**family),
            weight: reflection.enum_item("FontWeight", weight).unwrap_or(0),
            style: reflection.enum_item("FontStyle", style).unwrap_or(0),
        },
        None => FontValue {
            family: Str::from("rbxasset://fonts/families/LegacyArial.json"),
            weight: reflection.enum_item("FontWeight", "Regular").unwrap_or(0),
            style: reflection.enum_item("FontStyle", "Normal").unwrap_or(0),
        },
    }
}

/// FontInternal.ToEnum: the Enum.Font whose face equals `font`, or Enum.Font.Unknown.
pub fn font_to_enum(engine: &Engine, font: &FontValue) -> Option<EnumItemId> {
    let reflection = &engine.reflection;
    let weight = &*reflection.enum_item_info(font.weight).name;
    let style = &*reflection.enum_item_info(font.style).name;
    let mut matches: Vec<&str> = reflection
        .font_enum_faces
        .iter()
        .filter(|(_, (family, w, s))| family.as_bytes() == font.family.as_bytes() && &**w == weight && &**s == style)
        .map(|(name, _)| &**name)
        .collect();
    // The Luau runtime takes the first in table order; prefer a stable choice
    crate::sort::ord(&mut matches);
    match matches.first() {
        Some(name) => reflection.enum_item("Font", name),
        None => reflection.enum_item("Font", "Unknown"),
    }
}

// Content

fn content_of(value: &LuaValue) -> Option<ContentValue> {
    match value {
        LuaValue::UserData(ud) => ud.borrow::<LContent>().ok().map(|c| c.0.clone()),
        _ => None,
    }
}

#[inline(never)]
fn register_content(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let e = engine.clone();
    let index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let Some(content) = content_of(args.get(0)) else { return raise("invalid Content") };
        if let LuaValue::String(key) = args.get(1) {
            match &*key.as_bytes() {
                b"Uri" => {
                    return Ok(values([match &content {
                        ContentValue::Uri(uri) => LuaValue::String(lua.create_string(uri.as_bytes())?),
                        _ => LuaValue::Nil,
                    }]));
                }
                b"Object" => {
                    // (a Content keeps its object; one that is gone anyway answers nil, never a dead instance)
                    let live = matches!(content, ContentValue::Object(id) if e.state.borrow().dom.get(id).is_some());
                    return Ok(values([match content {
                        ContentValue::Object(id) if live => super::instance_value(lua, &e, id)?,
                        _ => LuaValue::Nil,
                    }]));
                }
                b"SourceType" => {
                    let name = match content {
                        ContentValue::Uri(_) => "Uri",
                        ContentValue::Object(_) => "Object",
                        ContentValue::Opaque(..) => "Opaque",
                        ContentValue::None => "None",
                    };
                    return Ok(values([match e.reflection.enum_item("ContentSourceType", name) {
                        Some(item) => enum_item_value(lua, &e, item)?,
                        None => LuaValue::Nil,
                    }]));
                }
                _ => {}
            }
        }
        unknown_member(&e, args.get(1), "Content", false)
    })?;
    let tostring = super::lua_fn(lua, |lua, this: LuaAnyUserData| {
        let engine = engine_of(lua);
        Ok(match &this.borrow::<LContent>()?.0 {
            ContentValue::Uri(uri) => format!("Content{{SourceType=Uri, Uri={uri}}}"),
            ContentValue::Object(id) => {
                let name = engine.state.borrow().dom.get(*id).map(|i| i.name.to_string()).unwrap_or_default();
                format!("Content{{SourceType=Object, Object={name}}}")
            }
            ContentValue::Opaque(..) => "Content{SourceType=Opaque}".into(),
            ContentValue::None => "Content{SourceType=None}".into(),
        })
    })?;
    let eq = super::create_boxed_val2(lua, Box::new(|_, a, b| Ok(LuaValue::Boolean(matches!((content_of(&a), content_of(&b)), (Some(a), Some(b)) if a == b)))))?;
    let newindex = read_only_members(lua, engine, "Content", &["SourceType", "Uri", "Object"])?;
    lua.register_userdata_type::<LContent>(|registry| {
        registry.add_meta_field("__type", "Content");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", tostring);
        registry.add_meta_field("__eq", eq);
    })?;
    let library = lua.create_table()?;
    library.set(
        "fromUri",
        named(lua, "fromUri", Library, super::create_boxed_val(lua, Box::new(|lua, uri| {
            let text = string_of(lua, &uri)?;
            lua.create_any_userdata(LContent::new(lua, ContentValue::Uri(Str::from(text)))).map(LuaValue::UserData)
        }))?)?,
    )?;
    let none_value = lua.create_any_userdata(LContent::new(lua, ContentValue::None))?;
    lock_metatable(lua, &none_value)?;
    // fromAssetId takes a number (or a string Luau reads as one), rounds it half away from zero to a 64-bit id (0 is
    // rbxassetid://0; past +-2^63 the id is 0, and 2^63 itself overflows to -2^63) and refuses a non-finite one;
    // fromObject takes an Object that is no Instance (an EditableImage, an EditableMesh). The texts and rules as a
    // playing client has them (Studio 0.747, tools/parity/probes/datatype_edges.luau and datatype_edges_more.luau
    // `content-`): no argument at all is `Invalid number of arguments.`.
    let e = engine.clone();
    library.set(
        "fromAssetId",
        wrapped(lua, engine, "fromAssetId", move |lua, args| {
            let args = Args::new(args);
            if args.is_empty() {
                return raise_plain("Invalid number of arguments.");
            }
            let value = args.get(0);
            let number = match value {
                LuaValue::Number(number) => Some(*number),
                LuaValue::Integer(number) => Some(*number as f64),
                LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten(),
                _ => None,
            };
            let Some(number) = number else {
                return raise_at_caller(format!("invalid argument #1 to 'fromAssetId' (number expected, got {})", super::typeof_value(&e, value)));
            };
            if !number.is_finite() {
                return raise_plain("Invalid id. Expected a finite value.");
            }
            const LIMIT: f64 = 9_223_372_036_854_775_808.0;
            let rounded = number.round();
            let id = if rounded > LIMIT || rounded < -LIMIT {
                0
            } else if rounded == LIMIT {
                i64::MIN
            } else {
                rounded as i64
            };
            Ok(values([LuaValue::UserData(
                lua.create_any_userdata(LContent::new(lua, ContentValue::Uri(Str::from(format!("rbxassetid://{id}")))))?,
            )]))
        })?,
    )?;
    let e = engine.clone();
    library.set(
        "fromObject",
        wrapped(lua, engine, "fromObject", move |lua, args| {
            let args = Args::new(args);
            if args.is_empty() {
                return raise_plain("Invalid number of arguments.");
            }
            let value = args.get(0);
            let object = match value {
                LuaValue::Nil => return raise_plain("Expected Object, got nil."),
                LuaValue::UserData(ud) => super::raw::instance_of_ud(ud).filter(|&id| {
                    let state = e.state.borrow();
                    state.dom.get(id).is_some_and(|instance| !e.reflection.is_a(instance.class, "Instance"))
                }),
                _ => None,
            };
            let Some(object) = object else {
                return raise_at_caller(format!("invalid argument #1 to 'fromObject' (Object expected, got {})", super::typeof_value(&e, value)));
            };
            Ok(values([LuaValue::UserData(lua.create_any_userdata(LContent::new(lua, ContentValue::Object(object)))?)]))
        })?,
    )?;
    library.set("none", none_value)?;
    engine.lua.borrow_mut().libraries.insert("Content", library);
    Ok(())
}

// TweenInfo

#[inline(never)]
fn register_tween_info(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let e = engine.clone();
    let index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let LuaValue::UserData(ud) = args.get(0) else { return raise("invalid TweenInfo") };
        let info = ud.borrow::<LTweenInfo>()?;
        if let LuaValue::String(key) = args.get(1) {
            match &*key.as_bytes() {
                b"Time" => return Ok(values([info.time.clone()])),
                b"EasingStyle" => return Ok(values([enum_item_value(lua, &e, info.easing_style)?])),
                b"EasingDirection" => return Ok(values([enum_item_value(lua, &e, info.easing_direction)?])),
                b"RepeatCount" => return Ok(values([info.repeat_count.clone()])),
                b"Reverses" => return Ok(values([LuaValue::Boolean(info.reverses)])),
                b"DelayTime" => return Ok(values([info.delay_time.clone()])),
                _ => {}
            }
        }
        unknown_member(&e, args.get(1), "TweenInfo", true)
    })?;
    // "Time:0.1 DelayTime:0.333333 RepeatCount:-1 Reverses:True ..." (Studio 0.744): the times with 6 significant digits,
    // the flag capitalized, a time that is not a number as MSVC's printf writes it (0/0 "-nan(ind)", -(0/0) "nan": measured,
    // Studio 0.747, tools/parity/probes/tween_args.luau)
    let tostring = super::lua_fn(lua, |lua, this: LuaAnyUserData| {
        let engine = engine_of(lua);
        let info = this.borrow::<LTweenInfo>()?;
        let time = |value: &LuaValue| match value {
            LuaValue::Number(n) if n.is_nan() => Ok(msvc_nan(*n).to_string()),
            LuaValue::Number(n) => Ok(lune_roblox::datatypes::types::format_g(*n, 6)),
            LuaValue::Integer(n) => Ok(lune_roblox::datatypes::types::format_g(*n as f64, 6)),
            other => string_of(lua, other),
        };
        Ok(format!(
            "Time:{} DelayTime:{} RepeatCount:{} Reverses:{} EasingDirection:{} EasingStyle:{}",
            time(&info.time)?,
            time(&info.delay_time)?,
            string_of(lua, &info.repeat_count)?,
            if info.reverses { "True" } else { "False" },
            engine.reflection.enum_item_info(info.easing_direction).name,
            engine.reflection.enum_item_info(info.easing_style).name,
        ))
    })?;
    let eq = super::create_boxed_val2(lua, Box::new(|_, a, b| {
        let (LuaValue::UserData(a), LuaValue::UserData(b)) = (a, b) else { return Ok(LuaValue::Boolean(false)) };
        let (Ok(a), Ok(b)) = (a.borrow::<LTweenInfo>(), b.borrow::<LTweenInfo>()) else { return Ok(LuaValue::Boolean(false)) };
        Ok(LuaValue::Boolean(
            a.time == b.time
                && a.easing_style == b.easing_style
                && a.easing_direction == b.easing_direction
                && a.repeat_count == b.repeat_count
                && a.reverses == b.reverses
                && a.delay_time == b.delay_time,
        ))
    }))?;
    let newindex = read_only_members(lua, engine, "TweenInfo", &["Time", "EasingStyle", "EasingDirection", "RepeatCount", "Reverses", "DelayTime"])?;
    lua.register_userdata_type::<LTweenInfo>(|registry| {
        registry.add_meta_field("__type", "TweenInfo");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", tostring);
        registry.add_meta_field("__eq", eq);
    })?;
    let library = lua.create_table()?;
    let e = engine.clone();
    // TweenInfo.new(time, easingStyle, easingDirection, repeatCount, reverses, delayTime), measured in Studio 0.747: an
    // argument that is passed must be of its type, nil included (`TweenInfo.new(1, nil)` raises as `TweenInfo.new(1, "Quad")`
    // does; only the arguments left off take their defaults), checked in order. A number is a number value (no string, no
    // boolean); a style or a direction is an item of its enum or a number, cut to a whole one, that is an item's value (2.5
    // is Back, 13 and -1 are refused); RepeatCount is cut to a whole number (2.7 reads 2). Spring and Bezier are refused
    // once the style is known, before the direction is looked at. A seventh argument is ignored.
    library.set(
        "new",
        wrapped(lua, engine, "new", move |lua, args| {
            let args = Args::new(args);
            let count = args.len();
            let number = |index: usize| match args.get(index) {
                LuaValue::Number(n) => Some(*n),
                LuaValue::Integer(n) => Some(*n as f64),
                _ => None,
            };
            let item = |enum_name: &str, value: &LuaValue| -> Option<EnumItemId> {
                let id = e.reflection.enum_id(enum_name)?;
                let by_value = |n: f64| n.is_finite().then(|| e.reflection.enums[id as usize].by_value.get(&(n.trunc() as i64)).copied()).flatten();
                match value {
                    LuaValue::UserData(_) => super::convert::coerce_enum(lua, &e, id, value),
                    LuaValue::Number(n) => by_value(*n),
                    LuaValue::Integer(n) => by_value(*n as f64),
                    _ => None,
                }
            };
            let time = match number(0) {
                Some(time) => time,
                None if count == 0 => 1.0,
                None => return raise_plain("TweenInfo.new first argument expects a number for time."),
            };
            let easing_style = if count > 1 {
                let Some(style) = item("EasingStyle", args.get(1)) else {
                    return raise_plain("TweenInfo.new second argument expects Enum.EasingStyle input");
                };
                if matches!(&*e.reflection.enum_item_info(style).name, "Spring" | "Bezier") {
                    return raise_plain("TweenInfo.new does not support Enum.EasingStyle.Spring or Enum.EasingStyle.Bezier");
                }
                style
            } else {
                e.reflection.enum_item("EasingStyle", "Quad").unwrap_or(0)
            };
            let easing_direction = if count > 2 {
                let Some(direction) = item("EasingDirection", args.get(2)) else {
                    return raise_plain("TweenInfo.new third argument expects Enum.EasingDirection input");
                };
                direction
            } else {
                e.reflection.enum_item("EasingDirection", "Out").unwrap_or(0)
            };
            let repeat_count = match number(3) {
                Some(count) => repeat_count(count),
                None if count <= 3 => 0.0,
                None => return raise_plain("TweenInfo.new fourth arg should be a number for RepeatCount."),
            };
            let reverses = match args.get(4) {
                LuaValue::Boolean(reverses) => *reverses,
                _ if count <= 4 => false,
                _ => return raise_plain("TweenInfo.new fifth arg should be a boolean for Reverses."),
            };
            let delay_time = match number(5) {
                Some(delay) => delay,
                None if count <= 5 => 0.0,
                None => return raise_plain("TweenInfo.new sixth arg should be a number for DelayTime."),
            };
            let info = LTweenInfo {
                time: LuaValue::Number(time),
                easing_style,
                easing_direction,
                repeat_count: LuaValue::Number(repeat_count),
                reverses,
                delay_time: LuaValue::Number(delay_time),
            };
            let userdata = lua.create_any_userdata(info)?;
            lock_metatable(lua, &userdata)?;
            Ok(values([LuaValue::UserData(userdata)]))
        })?,
    )?;
    engine.lua.borrow_mut().libraries.insert("TweenInfo", library);
    Ok(())
}

/// How MSVC's printf writes a NaN: the sign, then `nan(ind)` for the quiet NaN with no payload (0/0's), `nan(snan)` for a
/// signaling one, `nan` for any other.
fn msvc_nan(value: f64) -> &'static str {
    let bits = value.to_bits();
    let negative = bits >> 63 == 1;
    let quiet = bits & (1 << 51) != 0;
    let payload = bits & ((1 << 51) - 1);
    match (negative, quiet, payload) {
        (true, true, 0) => "-nan(ind)",
        (true, true, _) => "-nan",
        (false, true, _) => "nan",
        (true, false, _) => "-nan(snan)",
        (false, false, _) => "nan(snan)",
    }
}

/// TweenInfo's RepeatCount: a 32-bit integer, the number cut toward zero; one out of its range, an infinity or not a number,
/// reads -2147483648, as an x86 conversion gives (measured, Studio 0.747: 2^40, math.huge and 0/0 read -2147483648, -3 reads
/// -3; tools/parity/probes/tween_args.luau). Below 0 a tween repeats without end.
fn repeat_count(count: f64) -> f64 {
    let whole = count.trunc();
    if whole.is_nan() || whole < i32::MIN as f64 || whole > i32::MAX as f64 { i32::MIN as f64 } else { whole }
}

// Random: Luau's math.random generator (PCG32), one state per object (measured: Random.new(7)
// NextNumber 0.87370859248613331, NextInteger(1, 100) 2, ...).

const MULTIPLIER: u64 = 6364136223846793005;
const INCREMENT: u64 = 105;
const TWO32: f64 = 4294967296.0;

impl RandomState {
    fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        xorshifted.rotate_right((old >> 59) as u32)
    }

    /// ldexp(lo | hi << 32, -64) from two draws (lo first): the double conversion rounds once.
    fn next_unit(&mut self) -> f64 {
        let low = self.next_u32() as f64;
        let high = self.next_u32() as f64;
        (high * TWO32 + low) / 18446744073709551616.0
    }

    /// NextNumber: the bounds in order, then `min * (1 - u) + max * u` (measured in a playing client, Studio 0.744,
    /// tools/parity/probes/random.luau: 30 of 30 draws, where `min + (max - min) * u` matched 22; NextNumber(5, 1)
    /// draws from [1, 5]).
    fn next_number(&mut self, min: f64, max: f64) -> f64 {
        let (low, high) = if min > max { (max, min) } else { (min, max) };
        let unit = self.next_unit();
        low * (1.0 - unit) + high * unit
    }

    /// NextInteger: each bound as a C cast makes it a 64-bit integer (toward zero, so -1.5 is -1; NaN, infinities and
    /// values out of range the least one), in order, and `count` = high - low + 1 values. Up to 2^32 - 2 values take one
    /// draw, `low + (count * draw >> 32)`, as math.random does; from 2^32 - 1 values on, two draws make one 64-bit number
    /// `x` (the first draw its low half, as for NextNumber), and the value is `low` plus the high half of `count * x`
    /// from 32-bit halves, leaving out the product of the two low halves: `ch*xh + (ch*xl >> 32) + (cl*xh >> 32)`,
    /// rounded to a double (measured in a playing client, Studio 0.747, tools/parity/probes/random.luau: the draw counts
    /// from 2^32 - 3 to 2^32 + 1 values, 31 wide ranges for 20 seeds, 300 random calls, and 15 fractional bounds,
    /// negative and reversed, for 2 seeds, where flooring them gave another value or draw count for 9; before that
    /// 2,048 two-draw values of 32 seeds and 64 ranges, where the float interpolation ree used matched 1,235 and the
    /// exact 128-bit product 1,211, and 1,000 random calls).
    fn next_integer(&mut self, min: f64, max: f64) -> f64 {
        let (mut low, mut high) = (int64_cast(min), int64_cast(max));
        if low > high {
            std::mem::swap(&mut low, &mut high);
        }
        // (at most 2^64 - 1023 values: the widest bounds are -2^63 and 2^63 - 1024)
        let count = (high as i128 - low as i128 + 1) as u64;
        if count < u32::MAX as u64 {
            let offset = (count * self.next_u32() as u64) >> 32;
            return low.wrapping_add(offset as i64) as f64;
        }
        let (x_low, x_high) = (self.next_u32() as u64, self.next_u32() as u64);
        let (count_low, count_high) = (count & 0xFFFF_FFFF, count >> 32);
        // (below 2^64: each product is under 2^64 - 2^33 + 2, each shifted one under 2^32)
        let offset = count_high * x_high + ((count_high * x_low) >> 32) + ((count_low * x_high) >> 32);
        low.wrapping_add(offset as i64) as f64
    }
}

/// A double as a C cast to a 64-bit integer makes it on x86-64: toward zero, the least integer when out of range.
fn int64_cast(value: f64) -> i64 {
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value) {
        value as i64
    } else {
        i64::MIN
    }
}

/// Luau's `%` on doubles.
fn luau_mod(a: f64, b: f64) -> f64 {
    a - (a / b).floor() * b
}

/// Luau's bit32 argument conversion (non-finite and huge values become 0).
fn luau_u32(value: f64) -> u32 {
    if !value.is_finite() || value.abs() >= 9.2e18 {
        0
    } else {
        (value as i64) as u32
    }
}

/// The 64-bit number Random.new(seed) seeds with: the seed toward zero as a two's complement integer; a seed of 2^53 or
/// more either way, infinities too, is 0, and NaN (any payload) 0x8000000000000000, as a C cast makes it (measured in a
/// playing client, Studio 0.747, tools/parity/probes/random.luau Random-seeds-*: 41 of 41 seeds in an exploratory
/// capture, where ree's wrapped bits gave other streams from 2^53 up, and -1's for NaN and -inf, and zeros for inf).
fn seed_bits(seed: f64) -> u64 {
    if seed.is_nan() {
        1 << 63
    } else if seed.abs() >= 9_007_199_254_740_992.0 {
        0
    } else {
        seed as i64 as u64
    }
}

/// Measured seeding (pcg32_seed): s = 0; step; s += the seed's bits; step.
pub fn seed_random(seed: f64) -> RandomState {
    let mut random = RandomState { state: 0 };
    random.next_u32();
    random.state = random.state.wrapping_add(seed_bits(seed));
    random.next_u32();
    random
}

/// A number argument of Random's as Luau's checks read it: a number, or a string Luau reads as one, hex and spaces
/// included (measured in a playing client, Studio 0.747, tools/parity/probes/random.luau Random-strings and
/// Random-string-seeds: NextInteger("0x10", "20") is 20, Random.new(" 7 ") is Random.new(7)).
fn random_number(lua: &Lua, value: &LuaValue) -> LuaResult<Option<f64>> {
    match value {
        LuaValue::Number(n) => Ok(Some(*n)),
        LuaValue::Integer(n) => Ok(Some(*n as f64)),
        LuaValue::String(_) => lua.coerce_number(value.clone()),
        _ => Ok(None),
    }
}

/// A call of a Random method: its name, and whether it came by `.` (the type's __index) or by `:` (its __namecall).
/// A client raises Luau's own argument errors at the calling line, counting self as #1, and names the method in a `.`
/// call only (measured in a playing client, Studio 0.747, tools/parity/probes/random.luau Random-arguments-*:
/// `invalid argument #2 to 'NextInteger' (number expected, got nil)` and `missing argument #3 to 'NextInteger' (number
/// expected)` by `.`, `invalid argument #2 (number expected, got nil)` by `:`).
#[derive(Clone, Copy)]
struct RandomCall {
    name: &'static str,
    dotted: bool,
}

impl RandomCall {
    /// Argument `index` (0 is self) is not a `expected`: `invalid argument #n ... got <type>`, or `missing argument #n`
    /// past the last argument.
    fn bad<T>(self, engine: &Engine, args: &Args, index: usize, expected: &str) -> Result<T, Flow> {
        let to = if self.dotted { format!(" to '{}'", self.name) } else { String::new() };
        if index >= args.len() {
            raise_at_caller(format!("missing argument #{}{to} ({expected} expected)", index + 1))
        } else {
            raise_at_caller(format!("invalid argument #{}{to} ({expected} expected, got {})", index + 1, typeof_value(engine, args.get(index))))
        }
    }

    /// Self (`invalid argument #1 to 'Clone' (Random expected, got number)`; a `:` call's self always is a Random).
    fn this(self, engine: &Engine, args: &Args) -> Result<Rc<Cell<RandomState>>, Flow> {
        if let LuaValue::UserData(ud) = args.get(0)
            && let Ok(random) = ud.borrow::<LRandom>()
        {
            return Ok(random.0.clone());
        }
        self.bad(engine, args, 0, "Random")
    }

    fn number(self, lua: &Lua, engine: &Engine, args: &Args, index: usize) -> Result<f64, Flow> {
        match random_number(lua, args.get(index))? {
            Some(n) => Ok(n),
            None => self.bad(engine, args, index, "number"),
        }
    }
}

/// Registers a Random method twice: wrapped in `dotted` for `.` (the type's __index), and raw in `called` for `:` (the
/// type's __namecall, which runs it without the Luau wrapper).
#[inline(never)]
fn random_method(
    lua: &Lua,
    engine: &Rc<Engine>,
    (dotted, called): (&LuaTable, &LuaTable),
    name: &'static str,
    method: fn(&Lua, &Engine, &Args, RandomCall) -> Ret,
) -> LuaResult<()> {
    let owner = engine.clone();
    dotted.raw_set(name, wrapped(lua, engine, name, move |lua, args| method(lua, &owner, &Args::new(args), RandomCall { name, dotted: true }))?)?;
    let owner = engine.clone();
    let raw = super::create_boxed_fn(lua, Box::new(move |lua, args: LuaMultiValue| super::encode(lua, method(lua, &owner, &Args::new(args), RandomCall { name, dotted: false }))))?;
    called.raw_set(name, raw)
}

#[inline(never)]
fn register_random(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let methods = lua.create_table()?;
    let calls = lua.create_table()?;
    let tables = (&methods, &calls);
    random_method(lua, engine, tables, "NextNumber", |lua, engine, args, call| {
        let random = call.this(engine, args)?;
        // (no bounds is [0, 1); one argument, nil too, raises this, also at the calling line; a third is ignored)
        let (min, max) = match args.len() {
            1 => (0.0, 1.0),
            2 => return raise_at_caller("Random:NextNumber requires 0 or 2 arguments"),
            _ => (call.number(lua, engine, args, 1)?, call.number(lua, engine, args, 2)?),
        };
        let mut state = random.get();
        let value = state.next_number(min, max);
        random.set(state);
        Ok(values([LuaValue::Number(value)]))
    })?;
    random_method(lua, engine, tables, "NextInteger", |lua, engine, args, call| {
        let random = call.this(engine, args)?;
        let (min, max) = (call.number(lua, engine, args, 1)?, call.number(lua, engine, args, 2)?);
        let mut state = random.get();
        let value = state.next_integer(min, max);
        random.set(state);
        Ok(values([LuaValue::Number(value)]))
    })?;
    random_method(lua, engine, tables, "NextUnitVector", |_, engine, args, call| {
        let random = call.this(engine, args)?;
        // Two 32-bit draws in float arithmetic: the angle from the first, z from the second (measured in a playing
        // client, Studio 0.744, tools/parity/probes/random.luau: every component for 10 seeds; ree drew two
        // 64-bit numbers, which also moved every draw after it)
        let mut state = random.get();
        let (first, second) = (state.next_u32(), state.next_u32());
        random.set(state);
        let (u, v) = ((first as f64 / TWO32) as f32, (second as f64 / TWO32) as f32);
        let angle = std::f32::consts::TAU * u;
        let z = 2.0f32 * v - 1.0;
        let radius = (1.0f32 - z * z).max(0.0).sqrt();
        let (x, y) = (radius * ((angle as f64).cos() as f32), radius * ((angle as f64).sin() as f32));
        Ok(values([LuaValue::Vector(mlua::Vector::new(x, y, z))]))
    })?;
    random_method(lua, engine, tables, "Shuffle", |_, engine, args, call| {
        let random = call.this(engine, args)?;
        let LuaValue::Table(list) = args.get(1).clone() else {
            return call.bad(engine, args, 1, "table");
        };
        let length = list.len()?;
        // (measured likewise: Shuffle({1, nil, 3}) raises this, without a position, and draws nothing)
        for index in 1..=length {
            if list.get::<LuaValue>(index)?.is_nil() {
                return raise_plain("Attempted to shuffle table containing nil values. This is not allowed because the table length may change.");
            }
        }
        let mut index = length;
        while index >= 2 {
            let mut state = random.get();
            let other = state.next_integer(1.0, index as f64) as i64;
            random.set(state);
            let a: LuaValue = list.get(index)?;
            let b: LuaValue = list.get(other)?;
            list.set(index, b)?;
            list.set(other, a)?;
            index -= 1;
        }
        Ok(none())
    })?;
    random_method(lua, engine, tables, "Clone", |lua, engine, args, call| {
        let random = call.this(engine, args)?;
        let copy = lua.create_any_userdata(new_random(engine, random.get()))?;
        Ok(values([LuaValue::UserData(copy)]))
    })?;
    let e = engine.clone();
    // (a method read as a value is a new function at every read, as a client's: cfunction::copy)
    let index = wrapped(lua, engine, "__index", move |lua, args| {
        let args = Args::new(args);
        let member: LuaValue = match args.get(1) {
            LuaValue::String(key) => methods.raw_get(key.clone())?,
            _ => LuaValue::Nil,
        };
        match member {
            LuaValue::Function(method) => Ok(values([LuaValue::Function(super::cfunction::copy(lua, &method)?)])),
            LuaValue::Nil => unknown_member(&e, args.get(1), "Random", false),
            member => Ok(values([member])),
        }
    })?;
    let newindex = read_only_members(lua, engine, "Random", &[])?;
    let tostring = super::create_boxed_unit(lua, Box::new(|lua| Ok(LuaValue::String(lua.create_string("Random")?))))?;
    // (two Randoms are == when they would generate the same, as in a client: Random.new(1) == Random.new(1), not
    // Random.new(2); Studio 0.747, tools/parity/probes/datatype_errors.luau)
    let eq = super::create_boxed_val2(lua, Box::new(|_, a, b| {
        Ok(LuaValue::Boolean(match (a, b) {
            (LuaValue::UserData(a), LuaValue::UserData(b)) => match (a.borrow::<LRandom>(), b.borrow::<LRandom>()) {
                (Ok(a), Ok(b)) => a.0.get().state == b.0.get().state,
                _ => false,
            },
            _ => false,
        }))
    }))?;
    lua.register_userdata_type::<LRandom>(|registry| {
        registry.add_meta_field("__type", "Random");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", tostring);
        registry.add_meta_field("__eq", eq);
    })?;
    let sample = lua.create_any_userdata(LRandom(Rc::new(Cell::new(seed_random(0.0)))))?;
    lock_metatable(lua, &sample)?;
    super::raw::set_table_namecall(lua, &sample, calls, "Random", lua.create_table()?)?;

    // Random.new() without a seed: Studio seeds from entropy; offline runs stay deterministic, from
    // the run's seed (the default seed 1 keeps the sequence it always had); the sequence's state is the
    // scheduler's, so a harness's Context.SetRandomSeed restarts it
    let seeded = engine.clone();
    let library = lua.create_table()?;
    library.set(
        "new",
        wrapped(lua, engine, "new", move |lua, args| {
            let args = Args::new(args);
            let seed = match args.get(0) {
                LuaValue::Nil => {
                    let mut state = seeded.state.borrow_mut();
                    let run = state.sched.random_seed;
                    let start = state.sched.unseeded_random.unwrap_or_else(|| luau_u32(0x9E3779B9u32 as f64 + (run - 1.0) * 2654435761.0) as f64);
                    let next = start * 1103515245.0 + 12345.0;
                    let next = luau_u32(next) as f64;
                    state.sched.unseeded_random = Some(next);
                    next
                }
                // (measured likewise, Random-new-arguments: raised at the calling line, a vector named Vector3)
                other => match random_number(lua, other)? {
                    Some(n) => n,
                    None => return raise_at_caller(format!("invalid argument #1 to 'new' (number expected, got {})", typeof_value(&seeded, other))),
                },
            };
            let userdata = lua.create_any_userdata(new_random(&seeded, seed_random(seed)))?;
            Ok(values([LuaValue::UserData(userdata)]))
        })?,
    )?;
    engine.lua.borrow_mut().libraries.insert("Random", library);
    Ok(())
}

// DateTime (UTC calendar maths; the clock itself comes from the virtual scheduler)

fn days_from_civil(year: f64, month: f64, day: f64) -> f64 {
    let year = year - if month <= 2.0 { 1.0 } else { 0.0 };
    let era = (if year >= 0.0 { year } else { year - 399.0 } / 400.0).floor();
    let year_of_era = year - era * 400.0;
    let day_of_year = ((153.0 * (month + if month > 2.0 { -3.0 } else { 9.0 }) + 2.0) / 5.0).floor() + day - 1.0;
    let day_of_era = year_of_era * 365.0 + (year_of_era / 4.0).floor() - (year_of_era / 100.0).floor() + day_of_year;
    era * 146097.0 + day_of_era - 719468.0
}

fn civil_from_days(days: f64) -> (f64, f64, f64) {
    let days = days + 719468.0;
    let era = (if days >= 0.0 { days } else { days - 146096.0 } / 146097.0).floor();
    let day_of_era = days - era * 146097.0;
    let year_of_era = ((day_of_era - (day_of_era / 1460.0).floor() + (day_of_era / 36524.0).floor()
        - (day_of_era / 146096.0).floor())
        / 365.0)
        .floor();
    let year = year_of_era + era * 400.0;
    let day_of_year = day_of_era - (365.0 * year_of_era + (year_of_era / 4.0).floor() - (year_of_era / 100.0).floor());
    let month_prime = ((5.0 * day_of_year + 2.0) / 153.0).floor();
    let day = day_of_year - ((153.0 * month_prime + 2.0) / 5.0).floor() + 1.0;
    let month = month_prime + if month_prime < 10.0 { 3.0 } else { -9.0 };
    (year + if month <= 2.0 { 1.0 } else { 0.0 }, month, day)
}

struct Parts {
    year: f64,
    month: f64,
    day: f64,
    hour: f64,
    minute: f64,
    second: f64,
    millisecond: f64,
}

fn universal_parts(millis: f64) -> Parts {
    let days = (millis / 86400000.0).floor();
    let remainder = millis - days * 86400000.0;
    let (year, month, day) = civil_from_days(days);
    Parts {
        year,
        month,
        day,
        hour: (remainder / 3600000.0).floor(),
        minute: luau_mod((remainder / 60000.0).floor(), 60.0),
        second: luau_mod((remainder / 1000.0).floor(), 60.0),
        millisecond: luau_mod(remainder, 1000.0),
    }
}

/// Local time with the machine's zone and daylight saving (as Studio's ToLocalTime measures).
fn local_parts(millis: f64) -> Parts {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let seconds = (millis / 1000.0).floor() as i64;
    match Local.timestamp_opt(seconds, 0).single() {
        Some(time) => Parts {
            year: time.year() as f64,
            month: time.month() as f64,
            day: time.day() as f64,
            hour: time.hour() as f64,
            minute: time.minute() as f64,
            second: time.second() as f64,
            millisecond: luau_mod(millis, 1000.0),
        },
        None => universal_parts(millis),
    }
}

struct DateLocale {
    months: [&'static str; 12],
    months_short: [&'static str; 12],
    /// Sunday (0) through Saturday (6)
    days: [&'static str; 7],
    days_short: [&'static str; 7],
    days_min: [&'static str; 7],
    am: (&'static str, &'static str),
    pm: (&'static str, &'static str),
    formats: [(&'static str, &'static str); 10],
}

const EN_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];
const EN_MONTHS_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const EN_DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const EN_DAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const EN_DAYS_MIN: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/// Measured in a playing client (Studio 0.747: tools/parity/probes/datetime.luau, buffer_vector_time.luau, and
/// per-month/day/token measurements for en-us, en-gb, fr-fr, de-de, ja-jp). Unknown or bare language tags ("fr", "")
/// fall back to en-us.
const LOCALE_EN_US: DateLocale = DateLocale {
    months: EN_MONTHS,
    months_short: EN_MONTHS_SHORT,
    days: EN_DAYS,
    days_short: EN_DAYS_SHORT,
    days_min: EN_DAYS_MIN,
    am: ("AM", "am"),
    pm: ("PM", "pm"),
    formats: [
        ("LLLL", "dddd, MMMM D, YYYY h:mm A"),
        ("llll", "ddd, MMM D, YYYY h:mm A"),
        ("LTS", "h:mm:ss A"),
        ("LLL", "MMMM D, YYYY h:mm A"),
        ("lll", "MMM D, YYYY h:mm A"),
        ("LL", "MMMM D, YYYY"),
        ("ll", "MMM D, YYYY"),
        ("LT", "h:mm A"),
        ("L", "MM/DD/YYYY"),
        ("l", "M/D/YYYY"),
    ],
};

const LOCALE_EN_GB: DateLocale = DateLocale {
    months: EN_MONTHS,
    months_short: EN_MONTHS_SHORT,
    days: EN_DAYS,
    days_short: EN_DAYS_SHORT,
    days_min: EN_DAYS_MIN,
    am: ("AM", "am"),
    pm: ("PM", "pm"),
    formats: [
        ("LLLL", "dddd, D MMMM YYYY HH:mm"),
        ("llll", "ddd, D MMM YYYY HH:mm"),
        ("LTS", "HH:mm:ss"),
        ("LLL", "D MMMM YYYY HH:mm"),
        ("lll", "D MMM YYYY HH:mm"),
        ("LL", "D MMMM YYYY"),
        ("ll", "D MMM YYYY"),
        ("LT", "HH:mm"),
        ("L", "DD/MM/YYYY"),
        ("l", "D/M/YYYY"),
    ],
};

const LOCALE_FR_FR: DateLocale = DateLocale {
    months: [
        "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre",
        "décembre",
    ],
    months_short: [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
    ],
    days: ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"],
    days_short: ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."],
    days_min: ["di", "lu", "ma", "me", "je", "ve", "sa"],
    am: ("AM", "am"),
    pm: ("PM", "pm"),
    formats: [
        ("LLLL", "dddd D MMMM YYYY HH:mm"),
        ("llll", "ddd D MMM YYYY HH:mm"),
        ("LTS", "HH:mm:ss"),
        ("LLL", "D MMMM YYYY HH:mm"),
        ("lll", "D MMM YYYY HH:mm"),
        ("LL", "D MMMM YYYY"),
        ("ll", "D MMM YYYY"),
        ("LT", "HH:mm"),
        ("L", "DD/MM/YYYY"),
        ("l", "D/M/YYYY"),
    ],
};

const LOCALE_DE_DE: DateLocale = DateLocale {
    months: [
        "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November",
        "Dezember",
    ],
    months_short: [
        "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.", "Dez.",
    ],
    days: ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"],
    days_short: ["So.", "Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa."],
    days_min: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
    am: ("AM", "am"),
    pm: ("PM", "pm"),
    formats: [
        ("LLLL", "dddd, D. MMMM YYYY HH:mm"),
        ("llll", "ddd, D. MMM YYYY HH:mm"),
        ("LTS", "HH:mm:ss"),
        ("LLL", "D. MMMM YYYY HH:mm"),
        ("lll", "D. MMM YYYY HH:mm"),
        ("LL", "D. MMMM YYYY"),
        ("ll", "D. MMM YYYY"),
        ("LT", "HH:mm"),
        ("L", "DD.MM.YYYY"),
        ("l", "D.M.YYYY"),
    ],
};

const LOCALE_JA_JP: DateLocale = DateLocale {
    months: ["一月", "二月", "三月", "四月", "五月", "六月", "七月", "八月", "九月", "十月", "十一月", "十二月"],
    months_short: ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"],
    days: ["日曜日", "月曜日", "火曜日", "水曜日", "木曜日", "金曜日", "土曜日"],
    days_short: ["日", "月", "火", "水", "木", "金", "土"],
    days_min: ["日", "月", "火", "水", "木", "金", "土"],
    am: ("午前", "午前"),
    pm: ("午後", "午後"),
    formats: [
        ("LLLL", "YYYY年M月D日 dddd HH:mm"),
        ("llll", "YYYY年M月D日(ddd) HH:mm"),
        ("LTS", "HH:mm:ss"),
        ("LLL", "YYYY年M月D日 HH:mm"),
        ("lll", "YYYY年M月D日 HH:mm"),
        ("LL", "YYYY年M月D日"),
        ("ll", "YYYY年M月D日"),
        ("LT", "HH:mm"),
        ("L", "YYYY/MM/DD"),
        ("l", "YYYY/MM/DD"),
    ],
};

fn date_locale(locale: &str) -> &'static DateLocale {
    if locale.eq_ignore_ascii_case("en-gb") {
        &LOCALE_EN_GB
    } else if locale.eq_ignore_ascii_case("fr-fr") {
        &LOCALE_FR_FR
    } else if locale.eq_ignore_ascii_case("de-de") {
        &LOCALE_DE_DE
    } else if locale.eq_ignore_ascii_case("ja-jp") {
        &LOCALE_JA_JP
    } else {
        &LOCALE_EN_US
    }
}

/// FormatUniversalTime / FormatLocalTime as a playing client formats (Studio 0.744 / 0.747,
/// tools/parity/probes/datetime.luau, buffer_vector_time.luau): the longest token at each place, `[...]` copied
/// without its brackets, any other character as it is. There are no ordinal tokens (`Do` is the day and a literal o,
/// `14o`), and `Y`, `Q`, `w`, `W`, `e`, `E`, `g`, `G`, `k`, `Z`, `X`, `x` are literals too. `DDD` / `DDDD` are the day
/// of the year, `d` the weekday (0 Sunday), `dd` its short 2-letter form, `S` / `SS` / `SSS` tenths, hundredths and
/// milliseconds (`SSSS` is `SSS` then `S`). Measured locales: en-us, en-gb, fr-fr, de-de, ja-jp (other or unknown
/// locales fall back to en-us).
fn format_parts(t: &Parts, format: &str, locale: &str) -> String {
    const TOKENS: [&str; 27] = [
        "YYYY", "MMMM", "DDDD", "dddd", "MMM", "DDD", "ddd", "SSS", "YY", "MM", "DD", "dd", "HH", "hh", "mm", "ss", "SS", "M",
        "D", "d", "H", "h", "m", "s", "S", "A", "a",
    ];
    let loc = date_locale(locale);
    let days = days_from_civil(t.year, t.month, t.day);
    let weekday_index = luau_mod(days + 4.0, 7.0) as usize; // 1970-01-01 was a Thursday; 0 is Sunday
    let day_of_year = days - days_from_civil(t.year, 1.0, 1.0) + 1.0;
    let hour12 = if luau_mod(t.hour, 12.0) == 0.0 { 12.0 } else { luau_mod(t.hour, 12.0) };
    let month_index = (t.month as usize).clamp(1, 12) - 1;
    let value = |token: &str| -> String {
        match token {
            "YYYY" => format!("{:04}", t.year as i64),
            "YY" => format!("{:02}", (t.year as i64).rem_euclid(100)),
            "MMMM" => loc.months[month_index].to_string(),
            "MMM" => loc.months_short[month_index].to_string(),
            "MM" => format!("{:02}", t.month as i64),
            "M" => format!("{}", t.month as i64),
            "DDDD" => format!("{:03}", day_of_year as i64),
            "DDD" => format!("{}", day_of_year as i64),
            "DD" => format!("{:02}", t.day as i64),
            "D" => format!("{}", t.day as i64),
            "dddd" => loc.days[weekday_index].to_string(),
            "ddd" => loc.days_short[weekday_index].to_string(),
            "dd" => loc.days_min[weekday_index].to_string(),
            "d" => format!("{}", weekday_index),
            "HH" => format!("{:02}", t.hour as i64),
            "H" => format!("{}", t.hour as i64),
            "hh" => format!("{:02}", hour12 as i64),
            "h" => format!("{}", hour12 as i64),
            "mm" => format!("{:02}", t.minute as i64),
            "m" => format!("{}", t.minute as i64),
            "ss" => format!("{:02}", t.second as i64),
            "s" => format!("{}", t.second as i64),
            "SSS" => format!("{:03}", t.millisecond as i64),
            "SS" => format!("{:02}", (t.millisecond as i64) / 10),
            "S" => format!("{}", (t.millisecond as i64) / 100),
            "A" => if t.hour < 12.0 { loc.am.0 } else { loc.pm.0 }.to_string(),
            "a" => if t.hour < 12.0 { loc.am.1 } else { loc.pm.1 }.to_string(),
            _ => String::new(),
        }
    };
    let simple = |format: &str, output: &mut String| {
        let mut rest = format;
        while !rest.is_empty() {
            if let Some(token) = TOKENS.iter().find(|token| rest.starts_with(**token)) {
                output.push_str(&value(token));
                rest = &rest[token.len()..];
            } else {
                let character = rest.chars().next().expect("a character");
                output.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
    };
    let mut output = String::new();
    let mut rest = format;
    while !rest.is_empty() {
        if let Some(escaped) = rest.strip_prefix('[') {
            if let Some(end) = escaped.find(']') {
                output.push_str(&escaped[..end]);
                rest = &escaped[end + 1..];
                continue;
            }
        }
        if let Some((token, expansion)) = loc.formats.iter().find(|(token, _)| rest.starts_with(*token)) {
            simple(expansion, &mut output);
            rest = &rest[token.len()..];
            continue;
        }
        if let Some(token) = TOKENS.iter().find(|token| rest.starts_with(**token)) {
            output.push_str(&value(token));
            rest = &rest[token.len()..];
            continue;
        }
        let character = rest.chars().next().expect("a character");
        output.push(character);
        rest = &rest[character.len_utf8()..];
    }
    output
}

fn parts_table(lua: &Lua, t: &Parts) -> LuaResult<LuaTable> {
    let table = lua.create_table()?;
    table.set("Year", t.year)?;
    table.set("Month", t.month)?;
    table.set("Day", t.day)?;
    table.set("Hour", t.hour)?;
    table.set("Minute", t.minute)?;
    table.set("Second", t.second)?;
    table.set("Millisecond", t.millisecond)?;
    Ok(table)
}

fn iso_date(millis: f64) -> String {
    let t = universal_parts(millis);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.year as i64, t.month as i64, t.day as i64, t.hour as i64, t.minute as i64, t.second as i64
    )
}

fn date_time_of(value: &LuaValue) -> Option<f64> {
    match value {
        LuaValue::UserData(ud) => ud.borrow::<LDateTime>().ok().map(|d| d.0),
        _ => None,
    }
}

/// The instants a DateTime can hold: Unix timestamps from -17,987,443,200 to 253,402,300,799 seconds, years 1400 to 9999
/// (the client's own message gives the bounds; Roblox's documentation the years).
const DATE_TIME_MIN_MILLIS: f64 = -17_987_443_200_000.0;
const DATE_TIME_MAX_MILLIS: f64 = 253_402_300_799_999.0;
/// The messages a playing client raises (Studio 0.744, tools/parity/probes/datetime.luau)
const TIMESTAMP_OUT_OF_RANGE: &str =
    "Error interpreting timestamp as a precise point in time. Fields out of range. UnixTimestamp should be between -17987443200 and 253402300799.";
const FIELDS_OUT_OF_RANGE: &str = "Error interpreting year, month, day, hour, minute, second as a precise point in time. Fields out of range.";
const OVER_THE_MAX: &str = "Over the max DateTime supported: 9999-12-31T23:59:59Z";
/// fromUnixTimestampMillis' own message, and fromUniversalTime's for units that roll below 1400 (Studio 0.747,
/// tools/parity/probes/datatype_edges.luau `millis-range`, `rolled`)
const TIMESTAMP_MILLIS_OUT_OF_RANGE: &str = "Error interpreting timestampMillis as a precise point in time. Fields out of range. UnixTimestampMillis should be between -17987443200000 and 253402300799999.";
const BELOW_THE_MIN: &str = "Below the min DateTime supported: 1400-01-01T00:00:00Z";

/// A DateTime constructor's numeric argument: one not passed is absent, a number or numeric string is read, anything
/// else (nil passed included) raises from the calling line with the value's typeof (Studio 0.747,
/// tools/parity/probes/datatype_errors.luau: `fromUnixTimestamp(nil, 1)` raises `invalid argument #1 to
/// 'fromUnixTimestamp' (number expected, got nil)`, `fromUnixTimestamp()` is the epoch).
fn date_time_number(lua: &Lua, engine: &Engine, args: &Args, index: usize, name: &str) -> Result<Option<f64>, Flow> {
    if index >= args.len() {
        return Ok(None);
    }
    match args.get(index) {
        LuaValue::Number(n) => Ok(Some(*n)),
        LuaValue::Integer(n) => Ok(Some(*n as f64)),
        other => match other {
            LuaValue::String(_) => lua.coerce_number(other.clone()).ok().flatten(),
            _ => None,
        }
        .map(|n| Ok(Some(n)))
        .unwrap_or_else(|| {
            raise_at_caller(format!(
                "invalid argument #{} to '{name}' (number expected, got {})",
                index + 1,
                super::typeof_value(engine, other)
            ))
        }),
    }
}

/// A DateTime method's string argument as Luau's checks read it: a string, or a number in its tostring form
/// (`FormatUniversalTime(5, "en-us")` is "5"); anything else raises from the calling line (Studio 0.744,
/// tools/parity/probes/datetime.luau: `invalid argument #3 to 'FormatUniversalTime' (string expected, got nil)`; 0.747,
/// datatype_errors.luau: `missing argument #2 to 'FormatUniversalTime' (string expected)` when none was passed, and the
/// value's typeof, `got DateTime`).
fn date_time_string(lua: &Lua, engine: &Engine, args: &Args, index: usize, name: Option<&str>) -> Result<String, Flow> {
    let to = name.map(|n| format!(" to '{n}'")).unwrap_or_default();
    if index >= args.len() {
        return raise_at_caller(format!("missing argument #{}{to} (string expected)", index + 1));
    }
    let value = args.get(index);
    let text = match value {
        LuaValue::String(text) => Some(text.clone()),
        LuaValue::Number(_) | LuaValue::Integer(_) => lua.coerce_string(value.clone()).ok().flatten(),
        _ => None,
    };
    match text {
        Some(text) => Ok(text.to_string_lossy()),
        None => raise_at_caller(format!(
            "invalid argument #{}{to} (string expected, got {})",
            index + 1,
            super::typeof_value(engine, value)
        )),
    }
}

/// Self of ToUniversalTime, ToLocalTime and ToIsoDate as a playing client's reflection casts it (Studio 0.747,
/// tools/parity/probes/datatype_errors.luau): `Argument 1 missing or nil`, `Unable to cast double to DateTime`; a string
/// is read as milliseconds from its leading number, as C's strtod reads it ("86400" is 86.4 s, "2020-01-01T00:00:00Z"
/// 2.02 s, "x" 0).
fn date_time_self_cast(value: &LuaValue) -> Result<f64, Flow> {
    if let Some(millis) = date_time_of(value) {
        return Ok(millis);
    }
    match value {
        LuaValue::Nil => raise_plain("Argument 1 missing or nil"),
        LuaValue::String(text) => Ok(leading_number(&text.as_bytes())),
        other => raise_plain(format!("Unable to cast {} to DateTime", lune_roblox::datatypes::script_error::cast_name(other))),
    }
}

/// The decimal number a string starts with (spaces, a sign, digits, a fraction, an exponent), as strtod reads it; 0 when
/// it starts with none.
fn leading_number(bytes: &[u8]) -> f64 {
    let mut end = 0;
    while end < bytes.len() && bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    let start = end;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let digits = |from: usize| bytes[from..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    let whole = digits(end);
    end += whole;
    let mut fraction = 0;
    if end < bytes.len() && bytes[end] == b'.' {
        fraction = digits(end + 1);
        if whole + fraction > 0 {
            end += 1 + fraction;
        }
    }
    if whole + fraction == 0 {
        return 0.0;
    }
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut exponent = end + 1;
        if exponent < bytes.len() && (bytes[exponent] == b'+' || bytes[exponent] == b'-') {
            exponent += 1;
        }
        let count = digits(exponent);
        if count > 0 {
            end = exponent + count;
        }
    }
    std::str::from_utf8(&bytes[start..end]).ok().and_then(|text| text.parse().ok()).unwrap_or(0.0)
}

/// A fromUniversalTime unit as a client's 32-bit field holds it: truncated toward zero; not-a-number and a value outside
/// the int32 range are i32::MIN, as x86's conversion makes them.
fn date_time_int32(value: f64) -> i32 {
    let whole = value.trunc();
    if (-2_147_483_648.0..=2_147_483_647.0).contains(&whole) { whole as i32 } else { i32::MIN }
}

/// fromUniversalTime's instant for (year, month, day, hour, minute, second, millisecond) as a playing client reads them
/// (Studio 0.744, tools/parity/probes/datetime.luau; 0.747, datatype_edges.luau `rolled` and datatype_edges_more.luau
/// `rolled-`): absent units are 1970, 1, 1, 0, 0, 0, 0, and each unit is a 32-bit integer truncated toward zero
/// (2021.5, 1.5, 1.5 is 2021-01-01; one past the int32 range or not-a-number is i32::MIN). A year after 9999 raises
/// OVER_THE_MAX; a year before 1400 (i32::MIN included) or a month or day that names no date FIELDS_OUT_OF_RANGE. The
/// time units roll over into the date (hour 24 and -1, minute 60, second 60, millisecond 1000), where i32::MIN in an
/// hour, minute or second counts as +2^31; the millisecond becomes 32-bit microseconds (its whole milliseconds times
/// 1000, i32::MIN past the range: -2147483.648 ms). An instant past 9999-12-31T23:59:59.999Z raises OVER_THE_MAX, one
/// before 1400 BELOW_THE_MIN.
fn date_time_millis(fields: [Option<f64>; 7]) -> Result<f64, &'static str> {
    let [year, month, day, hour, minute, second, millisecond] = fields;
    let year = year.map_or(1970, date_time_int32);
    let (month, day) = (month.map_or(1, date_time_int32), day.map_or(1, date_time_int32));
    if year > 9999 {
        return Err(OVER_THE_MAX);
    }
    if year < 1400 || !(1..=12).contains(&month) {
        return Err(FIELDS_OUT_OF_RANGE);
    }
    if !(1..=month_length(year, month)).contains(&day) {
        return Err(FIELDS_OUT_OF_RANGE);
    }
    let time_unit = |value: Option<f64>| match value.map_or(0, date_time_int32) {
        i32::MIN => 2_147_483_648.0,
        whole => f64::from(whole),
    };
    let micros = match millisecond.map(|millis| millis.trunc() * 1000.0) {
        None => 0.0,
        Some(micros) if (-2_147_483_648.0..=2_147_483_647.0).contains(&micros) => micros,
        Some(_) => -2_147_483_648.0,
    };
    let millis = days_from_civil(f64::from(year), f64::from(month), f64::from(day)) * 86400000.0
        + time_unit(hour) * 3600000.0
        + time_unit(minute) * 60000.0
        + time_unit(second) * 1000.0
        + micros / 1000.0;
    if millis > DATE_TIME_MAX_MILLIS {
        return Err(OVER_THE_MAX);
    }
    if millis < DATE_TIME_MIN_MILLIS {
        return Err(BELOW_THE_MIN);
    }
    Ok(millis)
}

/// The days of `month` in `year` (proleptic Gregorian)
fn month_length(year: i32, month: i32) -> i32 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// fromIsoDate as a playing client parses it: a model fitted with no miss to 232 texts (Studio 0.744,
/// tools/parity/probes/datetime.luau; 0.747, datatype_edges.luau and datatype_edges_more.luau `iso`, and two one-off
/// captures of the same kind), not the client's own code:
/// - leading white space (C's isspace) is skipped, then a 4-digit year;
/// - each later field follows one separator character of any kind (`2020/01/01`, `2020-02-29x12:34Z`), and the text
///   may end before any separator or right after one (`2020`, `2020-02-`, `2020-02-29T12:`: the missing fields are 1 or
///   0); the month is 1 or 2 digits after optional white space, the day, hour, minute and second exactly 2 digits;
/// - after the second, `.` and any digits (the first three are the milliseconds);
/// - then the zone: white space is skipped; letters (`Z`, `junk`) end it unless `+` or `-` follows them; `+`, `,`, `:`
///   and `-` start an offset (`-` negative), whose hours are 1 or 2 digits (`+` and `:` with none is nil, `-` with
///   none is no offset; `,` first skips letters and commas, and with no digits after them is no offset, or nil when it
///   skipped nothing), followed by fields of 0 to 2 digits each after `:` or `-` (minutes, seconds, then ignored); an
///   offset past +14:00 or -12:00 is nil; after an offset only white space may follow;
/// - hours, minutes and seconds past their range roll over; a date that does not exist, a year before 1400 and an
///   instant outside 1400-01-01 to 9999-12-31T23:59:59.999Z after the offset are nil.
fn parse_iso_date(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let space = |byte: u8| matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r');
    let alpha = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_alphabetic);
    let digits = |at: &mut usize, least: usize, most: usize| -> Option<u32> {
        let (mut count, mut value) = (0, 0u32);
        while count < most && bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            value = value * 10 + u32::from(bytes[*at] - b'0');
            *at += 1;
            count += 1;
        }
        (count >= least).then_some(value)
    };
    let skip_space = |at: &mut usize| {
        while bytes.get(*at).copied().is_some_and(space) {
            *at += 1;
        }
    };
    // one separator of any kind before a field; false when the text ends before or right after it
    let separator = |at: &mut usize| -> bool {
        if *at >= bytes.len() {
            return false;
        }
        *at += 1;
        *at < bytes.len()
    };
    let mut at = 0;
    skip_space(&mut at);
    let year = digits(&mut at, 4, 4)?;
    let (mut month, mut day, mut hour, mut minute, mut second, mut millis, mut offset) = (1, 1, 0, 0, 0, 0.0, 0.0);
    'fields: {
        if !separator(&mut at) {
            break 'fields;
        }
        skip_space(&mut at);
        month = digits(&mut at, 1, 2)?;
        if !separator(&mut at) {
            break 'fields;
        }
        day = digits(&mut at, 2, 2)?;
        if !separator(&mut at) {
            break 'fields;
        }
        hour = digits(&mut at, 2, 2)?;
        if !separator(&mut at) {
            break 'fields;
        }
        minute = digits(&mut at, 2, 2)?;
        if !separator(&mut at) {
            break 'fields;
        }
        second = digits(&mut at, 2, 2)?;
        if bytes.get(at) == Some(&b'.') {
            at += 1;
            let start = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            let fraction = &bytes[start..at.min(start + 3)];
            let value = fraction.iter().fold(0.0, |value, digit| value * 10.0 + f64::from(digit - b'0'));
            millis = value * 10f64.powi(3 - fraction.len() as i32);
        }
        // the zone
        skip_space(&mut at);
        if alpha(at) {
            while alpha(at) {
                at += 1;
            }
            if !matches!(bytes.get(at), Some(b'+' | b'-')) {
                break 'fields;
            }
        }
        let Some(&sign) = bytes.get(at) else { break 'fields };
        if !matches!(sign, b'+' | b',' | b'-' | b':') {
            return None;
        }
        at += 1;
        if sign == b',' {
            let from = at;
            while alpha(at) || bytes.get(at) == Some(&b',') {
                at += 1;
            }
            if !bytes.get(at).is_some_and(u8::is_ascii_digit) {
                if at > from {
                    break 'fields;
                }
                return None;
            }
        }
        let (mut hours, mut parts) = (0, [0u32; 2]);
        match digits(&mut at, 1, 2) {
            Some(value) => {
                hours = value;
                let mut index = 0;
                while matches!(bytes.get(at), Some(b':' | b'-')) {
                    at += 1;
                    let value = digits(&mut at, 0, 2).unwrap_or(0);
                    if let Some(part) = parts.get_mut(index) {
                        *part = value;
                    }
                    index += 1;
                }
            }
            None if sign == b'-' => {}
            None => return None,
        }
        let seconds = f64::from(hours * 3600 + parts[0] * 60 + parts[1]);
        if seconds > if sign == b'-' { 12.0 * 3600.0 } else { 14.0 * 3600.0 } {
            return None;
        }
        offset = seconds * 1000.0 * if sign == b'-' { -1.0 } else { 1.0 };
        if bytes.get(at).is_some_and(|byte| !space(*byte)) {
            return None;
        }
    }
    let (year, month, day) = (year as i32, month as i32, day as i32);
    if year < 1400 || !(1..=12).contains(&month) || !(1..=month_length(year, month)).contains(&day) {
        return None;
    }
    let instant = days_from_civil(f64::from(year), f64::from(month), f64::from(day)) * 86400000.0
        + f64::from(hour) * 3600000.0
        + f64::from(minute) * 60000.0
        + f64::from(second) * 1000.0
        + millis
        - offset;
    (DATE_TIME_MIN_MILLIS..=DATE_TIME_MAX_MILLIS).contains(&instant).then_some(instant)
}

/// A DateTime at whole milliseconds (-0 is 0: `fromUnixTimestampMillis(-0.5)` prints 0).
fn make_date_time(lua: &Lua, millis: f64) -> LuaResult<LuaValue> {
    Ok(LuaValue::UserData(lua.create_any_userdata(LDateTime(millis.floor() + 0.0))?))
}

#[inline(never)]
fn register_date_time(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let methods = lua.create_table()?;
    let calls = lua.create_table()?;
    let this_millis = |args: &Args| date_time_self_cast(args.get(0));
    let add_cast = |name: &'static str, f: fn(&Lua, f64) -> Ret| -> LuaResult<()> {
        methods.set(
            name,
            wrapped(lua, engine, name, move |lua, args| f(lua, this_millis(&Args::new(args))?))?,
        )?;
        calls.raw_set(
            name,
            super::create_boxed_fn(lua, Box::new(move |lua, args: LuaMultiValue| {
                super::encode(lua, this_millis(&Args::new(args)).and_then(|millis| f(lua, millis)))
            }))?,
        )?;
        Ok(())
    };
    add_cast("ToUniversalTime", |lua, millis| Ok(values([LuaValue::Table(parts_table(lua, &universal_parts(millis))?)])))?;
    add_cast("ToLocalTime", |lua, millis| Ok(values([LuaValue::Table(parts_table(lua, &local_parts(millis))?)])))?;
    add_cast("ToIsoDate", |lua, millis| Ok(values([LuaValue::String(lua.create_string(iso_date(millis))?)])))?;
    // A `.` call names the method (`to 'FormatUniversalTime'`), a `:` namecall omits it (Studio 0.747,
    // tools/parity/probes/datatype_errors.luau).
    let format = |name: Option<&'static str>, parts: fn(f64) -> Parts, engine: Rc<Engine>| {
        move |lua: &Lua, args: LuaMultiValue| -> Ret {
            let args = Args::new(args);
            let to = name.map(|n| format!(" to '{n}'")).unwrap_or_default();
            let millis = match date_time_of(args.get(0)) {
                Some(millis) => millis,
                None if args.is_empty() => return raise_at_caller(format!("missing argument #1{to} (DateTime expected)")),
                None => {
                    return raise_at_caller(format!(
                        "invalid argument #1{to} (DateTime expected, got {})",
                        super::typeof_value(&engine, args.get(0))
                    ));
                }
            };
            let format = date_time_string(lua, &engine, &args, 1, name)?;
            let locale = date_time_string(lua, &engine, &args, 2, name)?;
            Ok(values([LuaValue::String(lua.create_string(format_parts(&parts(millis), &format, &locale))?)]))
        }
    };
    for (name, parts) in [("FormatUniversalTime", universal_parts as fn(f64) -> Parts), ("FormatLocalTime", local_parts as fn(f64) -> Parts)] {
        methods.set(name, wrapped(lua, engine, name, format(Some(name), parts, engine.clone()))?)?;
        let call = format(None, parts, engine.clone());
        calls.raw_set(name, super::create_boxed_fn(lua, Box::new(move |lua, args: LuaMultiValue| super::encode(lua, call(lua, args))))?)?;
    }
    let e = engine.clone();
    let index = wrapped(lua, engine, "__index", move |_, args| {
        let args = Args::new(args);
        let millis = date_time_of(args.get(0)).unwrap_or(0.0);
        if let LuaValue::String(key) = args.get(1) {
            match &*key.as_bytes() {
                b"UnixTimestampMillis" => return Ok(values([LuaValue::Number(millis)])),
                // truncated toward zero: -1 ms is 0 (Studio 0.744, tools/parity/probes/datetime.luau)
                b"UnixTimestamp" => return Ok(values([LuaValue::Number((millis / 1000.0).trunc() + 0.0)])),
                _ => {
                    let member: LuaValue = methods.raw_get(key.clone())?;
                    if !member.is_nil() {
                        return Ok(values([member]));
                    }
                }
            }
        }
        unknown_member(&e, args.get(1), "DateTime", false)
    })?;
    let newindex = read_only_members(lua, engine, "DateTime", &["UnixTimestamp", "UnixTimestampMillis"])?;
    let eq = super::create_boxed_val2(lua, Box::new(|_, a, b| {
        Ok(LuaValue::Boolean(matches!((date_time_of(&a), date_time_of(&b)), (Some(a), Some(b)) if a == b)))
    }))?;
    // tostring is the instant's UnixTimestampMillis ("1700000000123", "-1500"; Studio 0.744)
    let tostring = super::create_boxed_val(lua, Box::new(|lua, this| {
        Ok(LuaValue::String(lua.create_string(format!("{}", date_time_of(&this).unwrap_or(0.0) as i64))?))
    }))?;
    lua.register_userdata_type::<LDateTime>(|registry| {
        registry.add_meta_field("__type", "DateTime");
        registry.add_meta_field("__index", index);
        registry.add_meta_field("__newindex", newindex);
        registry.add_meta_field("__tostring", tostring);
        // (== only: a client's DateTime has no < or <=, the VM's 'attempt to compare DateTime < DateTime'; Studio 0.747,
        // tools/parity/probes/datatype_errors.luau)
        registry.add_meta_field("__eq", eq);
    })?;
    if let LuaValue::UserData(sample) = make_date_time(lua, 0.0)? {
        lock_metatable(lua, &sample)?;
        super::raw::set_table_namecall(lua, &sample, calls, "DateTime", lua.create_table()?)?;
    }

    let standard_offset_millis = scheduler::standard_offset_min_seconds() * 1000.0;
    let library = lua.create_table()?;
    let e = engine.clone();
    library.set(
        "now",
        named(lua, "now", Library, super::create_boxed_unit(lua, Box::new(move |lua| {
            let millis = e.state.borrow_mut().sched.unix_millis();
            make_date_time(lua, millis)
        }))?)?,
    )?;
    // fromUnixTimestamp truncates its seconds toward zero before scaling (0.5 and -0.5 are 0, 1.999 is 1000 ms, -1.5
    // -1000 ms) and raises TIMESTAMP_OUT_OF_RANGE for a second outside the range, NaN and the infinities included;
    // fromUnixTimestampMillis truncates its milliseconds (1.5 is 1, -1.5 -1, -0.5 0) and raises
    // TIMESTAMP_MILLIS_OUT_OF_RANGE (Studio 0.744, tools/parity/probes/datetime.luau; 0.747, datatype_edges.luau
    // `millis-range`).
    let timestamp = |name: &'static str, scale: f64, out_of_range: &'static str, engine: Rc<Engine>| {
        move |lua: &Lua, args: LuaMultiValue| -> Ret {
            let args = Args::new(args);
            let value = date_time_number(lua, &engine, &args, 0, name)?.unwrap_or(0.0).trunc();
            let millis = value * scale;
            if !(DATE_TIME_MIN_MILLIS..=DATE_TIME_MAX_MILLIS).contains(&millis) {
                return raise_plain(out_of_range);
            }
            Ok(values([make_date_time(lua, millis)?]))
        }
    };
    library.set("fromUnixTimestamp", wrapped(lua, engine, "fromUnixTimestamp", timestamp("fromUnixTimestamp", 1000.0, TIMESTAMP_OUT_OF_RANGE, engine.clone()))?)?;
    library.set("fromUnixTimestampMillis", wrapped(lua, engine, "fromUnixTimestampMillis", timestamp("fromUnixTimestampMillis", 1.0, TIMESTAMP_MILLIS_OUT_OF_RANGE, engine.clone()))?)?;
    // (the units are checked from the last one passed to the first, as a client does: fromUniversalTime(Vector3.one, "x")
    // raises for argument #2; Studio 0.747, datatype_errors.luau)
    let from_units = |name: &'static str, offset_millis: f64, engine: Rc<Engine>| {
        move |lua: &Lua, args: LuaMultiValue| -> Ret {
            let args = Args::new(args);
            let mut fields = [None; 7];
            for (index, field) in fields.iter_mut().enumerate().rev() {
                *field = date_time_number(lua, &engine, &args, index, name)?;
            }
            match date_time_millis(fields) {
                Ok(millis) => Ok(values([make_date_time(lua, millis - offset_millis)?])),
                Err(message) => raise_plain(message),
            }
        }
    };
    library.set("fromUniversalTime", wrapped(lua, engine, "fromUniversalTime", from_units("fromUniversalTime", 0.0, engine.clone()))?)?;
    library.set("fromLocalTime", wrapped(lua, engine, "fromLocalTime", from_units("fromLocalTime", standard_offset_millis, engine.clone()))?)?;
    // (no argument at all answers nil, as text that is no date does; nil passed raises: Studio 0.747, datatype_errors.luau)
    let e = engine.clone();
    library.set(
        "fromIsoDate",
        wrapped(lua, engine, "fromIsoDate", move |lua, args| {
            let args = Args::new(args);
            if args.is_empty() {
                return Ok(values([LuaValue::Nil]));
            }
            let text = date_time_string(lua, &e, &args, 0, Some("fromIsoDate"))?;
            Ok(values([match parse_iso_date(&text) {
                Some(millis) => make_date_time(lua, millis)?,
                None => LuaValue::Nil,
            }]))
        })?,
    )?;
    engine.lua.borrow_mut().libraries.insert("DateTime", library);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_matches_the_luau_oracle() {
        // runtime/tests/random.luau vectors; the wide NextInteger as a playing client gives it (Studio 0.747,
        // tools/parity/expected/random.txt Random-test-vectors: the floats ree interpolated gave one less for seeds 7, -3
        // and 4294967303)
        let vectors: &[(f64, [f64; 3], f64, f64)] = &[
            (0.0, [0.5514556904749074, 0.01723102436113014, 0.883376920906403], 73.0, 72384586732.0),
            (7.0, [0.8737085924861333, 0.4321572161309693, 0.3089922198934871], 5.0, 482964445447.0),
            (-3.0, [0.23554798859579634, 0.32381286642224644, 0.14705557445875167], 28.0, -493285887951.0),
            (1.9, [0.9484344206192203, 0.061285879733742145, 0.8770159548057006], 9.0, -497511649161.0),
            (-1.9, [0.0037155688242635435, 0.5469862230236278, 0.31011551315193603], 67.0, 1062967531246.0),
            (4294967303.0, [0.9971430285879401, 0.5584106571948971, 0.05136057646030647], 58.0, -943718980274.0),
            (9007199254740991.0, [0.027684032073451197, 0.09103798432822455, 0.9869885996197573], 53.0, 502533245394.0),
            (-9007199254740991.0, [0.22788703655698744, 0.9425859688069043, 0.06775640206431996], 38.0, -518359285893.0),
        ];
        for (seed, doubles, small, large) in vectors {
            let mut random = seed_random(*seed);
            for expected in doubles {
                assert_eq!(random.next_number(0.0, 1.0), *expected, "seed {seed}");
            }
            assert_eq!(random.next_integer(1.0, 100.0), *small);
            assert_eq!(random.next_integer(-1099511627776.0, 1099511628776.0), *large);
        }
        // (tools/parity/expected/random.txt, Studio 0.747: Random-seeds-large and Random-seeds-nan-fraction)
        assert_eq!(seed_random(f64::INFINITY).next_unit(), 0.5514556904749074);
        assert_eq!(seed_random(f64::NEG_INFINITY).next_unit(), 0.5514556904749074);
        assert_eq!(seed_random(f64::NAN).next_unit(), 0.20012743358005303);
        assert_eq!(seed_random(-0.0).next_unit(), seed_random(0.0).next_unit());
        // Unseeded sequence
        let mut counter = 0x9E3779B9u32 as f64;
        for expected in [0.15903319831126075, 0.6648808275887605, 0.7537480905786614] {
            counter = luau_u32(counter * 1103515245.0 + 12345.0) as f64;
            assert_eq!(seed_random(counter).next_unit(), expected);
        }
    }

    #[test]
    fn next_integer_takes_two_draws_from_2_32_minus_1_values() {
        // tools/parity/expected/random.txt (Studio 0.747): Random-draws seed 7, the value and the NextNumber after it
        let draws = |count: f64| {
            let mut random = seed_random(7.0);
            let value = random.next_integer(0.0, count - 1.0);
            (value, random.next_number(0.0, 1.0))
        };
        assert_eq!(draws(4294967294.0), (4132333491.0, 0.01291674651990379));
        assert_eq!(draws(4294967295.0), (3752549829.0, 0.4321572161309693));
        assert_eq!(draws(4294967296.0), (3752549830.0, 0.4321572161309693));
        // Random-wide 0 8589934591, seed 42: the high half of count * x without the low halves' product (floats gave
        // 1896055670); and the widest range there is, rounded to a double (Random-wide -2^63 2^63 - 1024, seed 0)
        assert_eq!(seed_random(42.0).next_integer(0.0, 8589934591.0), 1896055671.0);
        assert_eq!(seed_random(0.0).next_integer(-9223372036854775808.0, 9223372036854774784.0), 9.491899533266309e17);
    }

    #[test]
    fn seeds_of_2_53_or_more_are_0_and_nan_the_integer_indefinite() {
        // tools/parity/expected/random.txt (Studio 0.747): Random-seeds-large and Random-seeds-nan-fraction
        let first = |seed: f64| seed_random(seed).next_number(0.0, 1.0);
        assert_eq!(first(9007199254740991.0), 0.027684032073451197);
        assert_eq!(first(-9007199254740991.0), 0.22788703655698744);
        for seed in [9007199254740992.0, -9007199254740992.0, 2f64.powi(55), 2f64.powi(63), -2f64.powi(64), 1e19, -1e19, 1e300, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(first(seed), 0.5514556904749074, "seed {seed}");
        }
        for bits in [0x7FF8_0000_0000_0000u64, 0xFFF8_0000_0000_0000, 0x7FF0_0000_0000_0001] {
            assert_eq!(first(f64::from_bits(bits)), 0.20012743358005303, "NaN {bits:#x}");
        }
        assert_eq!(first(4503599627370495.5), 0.911891985182802);
        assert_eq!(first(-0.5), 0.5514556904749074);
    }

    #[test]
    fn next_integer_truncates_fractional_bounds() {
        // tools/parity/expected/random.txt (Studio 0.747), Random-fraction, seed 7: toward zero, so -1.5 is -1 and
        // (-0.5, 2^32 - 3) holds 2^32 - 2 values (one draw)
        assert_eq!(seed_random(7.0).next_integer(-1.5, -1.5), -1.0);
        assert_eq!(seed_random(7.0).next_integer(-0.5, -0.25), 0.0);
        assert_eq!(seed_random(7.0).next_integer(-2.7, -2.2), -2.0);
        assert_eq!(seed_random(7.0).next_integer(10.9, -4.2), 10.0);
        let mut random = seed_random(7.0);
        assert_eq!(random.next_integer(-0.5, 4294967293.0), 4132333491.0);
        assert_eq!(random.next_number(0.0, 1.0), 0.01291674651990379);
    }

    #[test]
    fn calendar_round_trips() {
        let days = days_from_civil(2026.0, 9.0, 21.0);
        assert_eq!(civil_from_days(days), (2026.0, 9.0, 21.0));
        assert_eq!(iso_date(1767225600000.0), "2026-01-01T00:00:00Z");
        let parts = universal_parts(1767225600000.0 + 13.0 * 3600000.0 + 5.0 * 60000.0);
        assert_eq!(format_parts(&parts, "LLLL", "en-us"), "Thursday, January 1, 2026 1:05 PM");
    }
}
