//! PhysicsService's collision groups: a registry of up to 32 named groups per DataModel and which pairs of them collide,
//! and BasePart.CollisionGroup / CollisionGroupId as they follow it. ree simulates no collisions (it has no rigid-body
//! contacts): the registry is the state scripts read and write, nothing more.
//!
//! Measured in roblox-sim's 0.747.0.7471481 client and server, script identity 2 (tools/parity/probes/collision_groups.luau,
//! collision_group_args.luau, loopback/collision_groups.luau):
//!
//! - A group has an id, 0 to 31: Default is 0 and always there; a new group takes the lowest free id. GetMaxCollisionGroups
//!   is 32. GetRegisteredCollisionGroups is a new array each call of `{ name, mask }` in id order (no id field); the
//!   deprecated GetCollisionGroups adds `id`. `mask` is a signed 32-bit integer whose bit i says whether the group
//!   collides with group i: -1 for a group that collides with all. CollisionGroupSetCollidable sets both groups' bits
//!   (Default too, and a group with itself); CollisionGroupsAreCollidable is true when either name is not registered.
//!   Unregistering a group frees its id and sets its bit in every other mask again; a group that takes the id later starts
//!   at -1. A rename keeps the id and the mask. CollisionGroupCollidableChanged never fires for a script.
//! - Names are compared as bytes, case and all ("default" is a group of its own, "A\0B" is not "A"). Register, Unregister,
//!   Rename, Create and Remove refuse an empty name ("Collision group requires a valid name."), Default ("Can not use/change
//!   the Default collision group.") and more than 100 bytes ("Collision group name length cannot exceed 100 chars"); a
//!   rename checks its first name, then its second. A 33rd group: "Cannot register more than 32 collision groups in this
//!   world." (CreateCollisionGroup: "... collision groups."), checked before the name is found already registered.
//!   Registering a registered name, unregistering or renaming one that is not, and renaming to a registered name do
//!   nothing. CreateCollisionGroup returns the id and refuses a registered name ("Could not register collision group, the
//!   name already exists."). CollisionGroupSetCollidable with a name that is not registered (empty and long ones too):
//!   "Both collision groups must be registered." GetCollisionGroupId: "Collision group not found."; GetCollisionGroupName:
//!   "" for a free id, "The Collision group Id must be between 0 and 32" outside 0..=31.
//! - A playing client may read the registry but not change it: Register, Unregister, Rename, SetCollidable, Create and
//!   Remove raise "This API can only be used on the server!" (Create and Remove after their name checks, the others before).
//!   A client sees the groups its server registers, from its join on. A loopback client gets its server's registry at the
//!   join and, after a server frame that changed it, once at that frame's flush, as the frame left it (a property's changes
//!   in a frame go out once the same way): it works out its parts' ids again when a group's name changed (or the registry
//!   is new to it), and a mask alone leaves them, as SetCollidable does on the server. A server and Studio's edit DataModel
//!   may change it (the edit one by Roblox's documentation of the Collision Groups editor, not measured).
//! - Arguments are cast from the last to the first, before any other check: a missing one "Argument n missing or nil". A
//!   name casts any value to text (lua::convert::string_param: 5 is "5", true "true" after the warning a client gives for a
//!   value that is neither a string nor a number, a table "{}"). The collidable flag
//!   takes a boolean, a number (0 is false, any other true) or "true" / "false" in any case; another string "Unable to
//!   cast string to bool", another value "Unable to cast <type> to bool" (Array, Function, buffer, Instance, Vector3, an
//!   EnumItem's enum name). GetCollisionGroupName's id is a number rounded half away from zero, a boolean, or a numeric
//!   string ("Unable to cast string to int"). A part argument is nil or an Instance ("Unable to cast value to Object"),
//!   then a BasePart ("Parameter 2 must be BasePart in CollisionGroupContainsPart.", "Parameter 1 must be BasePart in
//!   SetPartCollisionGroup.").
//! - BasePart.CollisionGroup takes any name, registered or not; "" is ignored and a longer name is cut to its first 100
//!   bytes (inside a UTF-8 sequence too), before it is compared with the name the part holds. SetPartCollisionGroup writes
//!   it the same way, on a client too; CollisionGroupContainsPart compares the name. CollisionGroupId is the group's id (-1
//!   for a name that is not registered) for a part in the Workspace: worked out when the part comes into the Workspace
//!   (inside a model too), when its CollisionGroup changes there, and for every part there when a group is registered,
//!   created, renamed, unregistered or removed, over an id a script wrote; CollisionGroupSetCollidable leaves it. Out of
//!   the Workspace it keeps what it had, and a name written there leaves it. Each change fires Changed as a write does.
//!   ree works it out once the run has touched collision groups (a group registered or created, or a script's write to
//!   either property): until then a part keeps the id it was loaded with.
//! - What a group change costs: a server spends 1.7 ns on each part in the Workspace and 0.38 us more on each whose id
//!   changes (cost.rs COLLISION_GROUP_PART, COLLISION_GROUP_CHANGED), charged as the parts are worked out. A tree coming into
//!   the Workspace pays the same for the parts it brings, as they take their ids.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use mlua::prelude::*;

use super::{Behaviors, MethodFn, method, values};
use crate::{
    ClassId, Engine, Hooks, InstanceId, NameId, Reflection, RunContext, State,
    instance as core,
    lua::{Args, Flow, convert, none, raise, typeof_value},
    value::{Str, Value},
};

/// The groups a DataModel holds at most (GetMaxCollisionGroups)
const MAX_GROUPS: usize = 32;
/// The longest group name, in bytes
const MAX_NAME: usize = 100;
const DEFAULT: &[u8] = b"Default";

/// One DataModel's groups.
struct Registry {
    /// The group with each id, 0 (Default) to 31
    names: [Option<Box<[u8]>>; MAX_GROUPS],
    /// Each id's mask: bit i set when the group collides with group i (a free id's is all ones)
    masks: [u32; MAX_GROUPS],
    /// The run has touched collision groups: parts' CollisionGroupId follows the registry from now on (the module's notes)
    active: bool,
}

impl Registry {
    fn new() -> Self {
        let mut names: [Option<Box<[u8]>>; MAX_GROUPS] = std::array::from_fn(|_| None);
        names[0] = Some(DEFAULT.into());
        Self { names, masks: [u32::MAX; MAX_GROUPS], active: false }
    }

    fn id(&self, name: &[u8]) -> Option<usize> {
        self.names.iter().position(|held| held.as_deref() == Some(name))
    }

    fn full(&self) -> bool {
        self.names.iter().all(Option::is_some)
    }

    /// A new group at the lowest free id (the caller checked there is one), colliding with every group.
    fn add(&mut self, name: &[u8]) -> usize {
        let id = self.names.iter().position(Option::is_none).expect("a free id");
        self.names[id] = Some(name.into());
        self.masks[id] = u32::MAX;
        self.active = true;
        id
    }

    /// The group's id freed: every group collides with whatever takes it next.
    fn remove(&mut self, id: usize) {
        self.names[id] = None;
        for mask in &mut self.masks {
            *mask |= 1 << id;
        }
    }

    fn set_collidable(&mut self, a: usize, b: usize, collidable: bool) {
        if collidable {
            self.masks[a] |= 1 << b;
            self.masks[b] |= 1 << a;
        } else {
            self.masks[a] &= !(1 << b);
            self.masks[b] &= !(1 << a);
        }
    }

    /// The groups in id order as GetRegisteredCollisionGroups (and, `with_id`, GetCollisionGroups) lists them: a new table
    /// of new entries each call.
    fn list(&self, lua: &Lua, with_id: bool) -> LuaResult<LuaTable> {
        let list = lua.create_table()?;
        for (id, name) in self.names.iter().enumerate() {
            let Some(name) = name else { continue };
            let entry = lua.create_table_with_capacity(0, 3)?;
            if with_id {
                entry.raw_set("id", id as f64)?;
            }
            entry.raw_set("mask", self.masks[id] as i32 as f64)?;
            entry.raw_set("name", lua.create_string(name)?)?;
            list.raw_push(entry)?;
        }
        Ok(list)
    }
}

/// One DataModel's collision-group registry as replicated from server to client (loopback/bridge.rs).
#[derive(Clone)]
pub struct Snapshot {
    pub names: [Option<Box<[u8]>>; MAX_GROUPS],
    pub masks: [u32; MAX_GROUPS],
    /// A frame's snapshot: one of the changes since the last flush left the registry with more than Default (or Default not
    /// colliding with all), so the client follows it from now on, as that change's own snapshot would have made it (false at
    /// the join, whose snapshot decides that alone)
    pub touched: bool,
}

impl Registry {
    /// More than Default, or Default not colliding with all: a client that receives it follows the registry
    fn non_default(names: &[Option<Box<[u8]>>; MAX_GROUPS], masks: &[u32; MAX_GROUPS]) -> bool {
        names[1..].iter().any(Option::is_some) || masks[0] != u32::MAX
    }
}

/// What the methods, the part setters and the hierarchy hook share: one engine's registry and the names they use.
pub(crate) struct Groups {
    registry: RefCell<Registry>,
    base_part: ClassId,
    /// BasePart.CollisionGroup and CollisionGroupId
    group: NameId,
    group_id: NameId,
    /// A loopback server's registry changed since the frame's last flush: its log holds the one marker that sends it
    /// ([`Groups::replicate`]), and whether a change left it other than Default alone ([`Snapshot::touched`])
    queued: Cell<bool>,
    touched: Cell<bool>,
}

impl Groups {
    fn snapshot(&self) -> Snapshot {
        let registry = self.registry.borrow();
        Snapshot { names: registry.names.clone(), masks: registry.masks, touched: false }
    }

    /// The server's registry, at the join or in a frame's changes. Every part in the Workspace takes its group's id again,
    /// unless the registry was already followed and no group's name changed: a mask alone leaves the ids, as
    /// CollisionGroupSetCollidable leaves them on the server.
    fn apply_snapshot(&self, engine: &Engine, snapshot: Snapshot) {
        let resolve = {
            let mut registry = self.registry.borrow_mut();
            let (followed, renamed) = (registry.active, registry.names != snapshot.names);
            if Registry::non_default(&snapshot.names, &snapshot.masks) || snapshot.touched {
                registry.active = true;
            }
            registry.names = snapshot.names;
            registry.masks = snapshot.masks;
            !followed || renamed
        };
        if resolve {
            self.resolve_placed(engine);
        }
    }

    /// A change a loopback client hears of: the first since the frame's last flush logs a marker, in its place among the
    /// frame's changes, and the flush sends the registry as it is then (loopback/replicate.rs flush, [`flushed`]), as a
    /// property's changes in a frame go out once with its last value. A frame that changes the groups many times sends one
    /// snapshot, and its client takes the parts' ids in once.
    fn replicate(&self, engine: &Engine) {
        let mut state = engine.state.borrow_mut();
        let Some(log) = state.loopback_log.as_mut() else { return };
        {
            let registry = self.registry.borrow();
            if Registry::non_default(&registry.names, &registry.masks) {
                self.touched.set(true);
            }
        }
        if !self.queued.replace(true) {
            log.push(crate::loopback::replicate::Change::CollisionGroups);
        }
    }

    fn is_part(&self, reflection: &Reflection, state: &State, id: InstanceId) -> bool {
        reflection.class(state.dom.at(id).class).is_a(self.base_part)
    }

    /// The part takes the id of the group its CollisionGroup names, -1 for a name that is not registered (Changed fires
    /// when that differs, as for a write). Returns whether the id changed.
    fn resolve(&self, state: &mut State, reflection: &Reflection, hooks: &Hooks, registry: &Registry, part: InstanceId) -> bool {
        let id = state.dom.raw_get(part, self.group).and_then(Value::as_str).and_then(|name| registry.id(name.as_bytes()));
        core::raw_set(state, reflection, hooks, part, self.group_id, Value::Number(id.map_or(-1.0, |id| id as f64)))
    }

    /// Every part in the Workspace takes its group's id again: a group came, went or changed its name. Returns how many
    /// parts it looked at and how many of their ids changed.
    fn resolve_placed(&self, engine: &Engine) -> Resolved {
        let Some(world) = engine.world.get() else { return Resolved::default() };
        let reflection = &engine.reflection;
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        let parts = state.dom.descendants_where(world.workspace, |instance| reflection.class(instance.class).is_a(self.base_part));
        let registry = self.registry.borrow();
        let mut resolved = Resolved { parts: parts.len(), changed: 0 };
        for part in parts {
            resolved.changed += usize::from(self.resolve(&mut state, reflection, &hooks, &registry, part));
        }
        resolved
    }

    /// A script's group change (a server's, or in edit): every part in the Workspace takes its id again, at what a server
    /// spends on that ([`Resolved::charge`]), and a loopback client hears of it.
    fn changed(&self, engine: &Engine) {
        self.resolve_placed(engine).charge(engine);
        self.replicate(engine);
    }

    /// A part's CollisionGroup written, by the property or SetPartCollisionGroup: "" is ignored and a longer name cut to
    /// 100 bytes; a change in the Workspace works out its CollisionGroupId.
    fn write_name(&self, engine: &Engine, part: InstanceId, name: &[u8]) {
        self.registry.borrow_mut().active = true;
        if name.is_empty() {
            return;
        }
        let name = &name[..name.len().min(MAX_NAME)];
        let reflection = &engine.reflection;
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        if !core::raw_set(&mut state, reflection, &hooks, part, self.group, Value::String(Str::new(name))) {
            return;
        }
        if engine.world.get().is_some_and(|world| state.dom.is_descendant_of_home(part, world.workspace)) {
            self.resolve(&mut state, reflection, &hooks, &self.registry.borrow(), part);
        }
    }
}

/// The parts a group change looked at and those whose CollisionGroupId it changed.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct Resolved {
    parts: usize,
    changed: usize,
}

impl Resolved {
    /// What a server spends on them beyond the call (cost.rs COLLISION_GROUP_PART, COLLISION_GROUP_CHANGED).
    fn charge(self, engine: &Engine) {
        self.charge_in(&engine.state.borrow());
    }

    /// [`Resolved::charge`] where the state is borrowed already.
    fn charge_in(self, state: &State) {
        let seconds = self.parts as f64 * crate::cost::COLLISION_GROUP_PART + self.changed as f64 * crate::cost::COLLISION_GROUP_CHANGED;
        if seconds > 0.0 {
            state.sched.charge(seconds);
            crate::boundary::add_cost(seconds);
        }
    }
}

/// A value's name in a failed cast, as these methods name it (measured: Array, Function, buffer, Instance, Vector3, and an
/// EnumItem's enum: "Material").
fn cast_name(engine: &Engine, value: &LuaValue) -> String {
    match value {
        LuaValue::Table(table) => if table.raw_len() > 0 || table.is_empty() { "Array" } else { "Dictionary" }.into(),
        LuaValue::Function(_) => "Function".into(),
        LuaValue::UserData(ud) => match ud.borrow::<crate::lua::types::LEnumItem>() {
            Ok(item) => engine.reflection.enum_item_type_name(item.0).to_string(),
            Err(_) => typeof_value(engine, value),
        },
        other => typeof_value(engine, other),
    }
}

/// A group name argument (1 is the first after self): any value but nil, read as a client reads a string parameter (warned about
/// and cast to text: convert::string_param).
fn name_arg(lua: &Lua, engine: &Engine, args: &Args, index: usize) -> Result<Vec<u8>, Flow> {
    match args.get(index) {
        LuaValue::Nil => raise(format!("Argument {index} missing or nil")),
        value => Ok(convert::string_param(lua, engine, value, index, true)?),
    }
}

/// CollisionGroupSetCollidable's flag: a boolean, a number (0 is false) or "true" / "false" in any case.
fn bool_arg(engine: &Engine, args: &Args, index: usize) -> Result<bool, Flow> {
    match args.get(index) {
        LuaValue::Nil => raise(format!("Argument {index} missing or nil")),
        LuaValue::Boolean(flag) => Ok(*flag),
        LuaValue::Number(number) => Ok(*number != 0.0),
        LuaValue::Integer(number) => Ok(*number != 0),
        LuaValue::String(text) if text.as_bytes().eq_ignore_ascii_case(b"true") => Ok(true),
        LuaValue::String(text) if text.as_bytes().eq_ignore_ascii_case(b"false") => Ok(false),
        other => raise(format!("Unable to cast {} to bool", cast_name(engine, other))),
    }
}

/// GetCollisionGroupName's id: a number rounded half away from zero (0.5 is 1, 31.5 is 32), a boolean or a numeric
/// string; None outside 0..=31 (NaN and the infinities too).
fn id_arg(lua: &Lua, engine: &Engine, args: &Args, index: usize) -> Result<Option<usize>, Flow> {
    let value = args.get(index);
    let number = match value {
        LuaValue::Nil => return raise(format!("Argument {index} missing or nil")),
        LuaValue::Number(number) => *number,
        LuaValue::Integer(number) => *number as f64,
        LuaValue::Boolean(flag) => *flag as u8 as f64,
        LuaValue::String(_) => match lua.coerce_number(value.clone()).ok().flatten() {
            Some(number) => number,
            None => return raise("Unable to cast string to int"),
        },
        other => return raise(format!("Unable to cast {} to int", cast_name(engine, other))),
    };
    let rounded = number.round();
    Ok((0.0..MAX_GROUPS as f64).contains(&rounded).then_some(rounded as usize))
}

/// A part argument: nil or an instance (the method checks it is a BasePart).
fn instance_arg(args: &Args, index: usize) -> Result<Option<InstanceId>, Flow> {
    match args.get(index) {
        LuaValue::Nil => Ok(None),
        _ => match args.instance(index) {
            Some(id) => Ok(Some(id)),
            None => raise("Unable to cast value to Object"),
        },
    }
}

/// A playing client reads the registry but may not change it.
fn server_only(engine: &Engine) -> Result<(), Flow> {
    if engine.options.run_context == RunContext::Client && !engine.options.edit_mode() {
        return raise("This API can only be used on the server!");
    }
    Ok(())
}

/// A name Register, Unregister, Rename, Create and Remove take.
fn check_name(name: &[u8]) -> Result<(), Flow> {
    if name.is_empty() {
        raise("Collision group requires a valid name.")
    } else if name == DEFAULT {
        raise("Can not use/change the Default collision group.")
    } else if name.len() > MAX_NAME {
        raise("Collision group name length cannot exceed 100 chars")
    } else {
        Ok(())
    }
}

/// UnregisterCollisionGroup and the deprecated RemoveCollisionGroup: the group's id freed, if the name has one.
/// Unregister checks the side before the name, Remove after it (measured on a client).
fn unregister(groups: Rc<Groups>, side_first: bool) -> MethodFn {
    method(move |lua, engine, _, args| {
        let name = name_arg(lua, engine, args, 1)?;
        if side_first {
            server_only(engine)?;
            check_name(&name)?;
        } else {
            check_name(&name)?;
            server_only(engine)?;
        }
        let mut registry = groups.registry.borrow_mut();
        if let Some(id) = registry.id(&name) {
            registry.remove(id);
            drop(registry);
            groups.changed(engine);
        }
        Ok(none())
    })
}

/// The engine's collision-group registry snapshot for loopback join replication.
pub(crate) fn snapshot(engine: &Engine) -> Option<Snapshot> {
    engine.physics.get().map(|groups| groups.snapshot())
}

/// A loopback server's frame is being flushed (its log taken): the registry as it is now if it changed since the last flush,
/// for the place of the log's marker; the next change logs a marker again.
pub(crate) fn flushed(engine: &Engine) -> Option<Snapshot> {
    let groups = engine.physics.get()?;
    if !groups.queued.replace(false) {
        return None;
    }
    Some(Snapshot { touched: groups.touched.replace(false), ..groups.snapshot() })
}

/// Applies a replicated collision-group snapshot on a loopback client.
pub(crate) fn apply_snapshot(engine: &Engine, snapshot: Snapshot) {
    if let Some(groups) = engine.physics.get() {
        groups.apply_snapshot(engine, snapshot);
    }
}

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let names = (reflection.class_id("PhysicsService"), reflection.class_id("BasePart"), reflection.name_id("CollisionGroup"), reflection.name_id("CollisionGroupId"));
    let (Some(service), Some(base_part), Some(group), Some(group_id)) = names else { return };
    let groups = Rc::new(Groups { registry: RefCell::new(Registry::new()), base_part, group, group_id, queued: Cell::new(false), touched: Cell::new(false) });
    let _ = engine.physics.set(groups.clone());
    let mut add = |name: &str, function: MethodFn| {
        if let Some(name) = reflection.name_id(name) {
            behaviors.add_method(service, name, function);
        }
    };

    add("GetMaxCollisionGroups", method(|_, _, _, _| Ok(values([LuaValue::Number(MAX_GROUPS as f64)]))));
    let shared = groups.clone();
    add("GetRegisteredCollisionGroups", method(move |lua, _, _, _| Ok(values([LuaValue::Table(shared.registry.borrow().list(lua, false)?)]))));
    let shared = groups.clone();
    add("GetCollisionGroups", method(move |lua, _, _, _| Ok(values([LuaValue::Table(shared.registry.borrow().list(lua, true)?)]))));
    let shared = groups.clone();
    add("IsCollisionGroupRegistered", method(move |lua, engine, _, args| {
        let name = name_arg(lua, engine, args, 1)?;
        Ok(values([LuaValue::Boolean(shared.registry.borrow().id(&name).is_some())]))
    }));
    let shared = groups.clone();
    add("CollisionGroupsAreCollidable", method(move |lua, engine, _, args| {
        let second = name_arg(lua, engine, args, 2)?;
        let first = name_arg(lua, engine, args, 1)?;
        let registry = shared.registry.borrow();
        let collidable = match (registry.id(&first), registry.id(&second)) {
            (Some(a), Some(b)) => registry.masks[a] & (1 << b) != 0,
            _ => true,
        };
        Ok(values([LuaValue::Boolean(collidable)]))
    }));
    let shared = groups.clone();
    add("GetCollisionGroupId", method(move |lua, engine, _, args| {
        let name = name_arg(lua, engine, args, 1)?;
        match shared.registry.borrow().id(&name) {
            Some(id) => Ok(values([LuaValue::Number(id as f64)])),
            None => raise("Collision group not found."),
        }
    }));
    let shared = groups.clone();
    add("GetCollisionGroupName", method(move |lua, engine, _, args| {
        let Some(id) = id_arg(lua, engine, args, 1)? else { return raise("The Collision group Id must be between 0 and 32") };
        let registry = shared.registry.borrow();
        Ok(values([LuaValue::String(lua.create_string(registry.names[id].as_deref().unwrap_or_default())?)]))
    }));

    let shared = groups.clone();
    add("RegisterCollisionGroup", method(move |lua, engine, _, args| {
        let name = name_arg(lua, engine, args, 1)?;
        server_only(engine)?;
        check_name(&name)?;
        let mut registry = shared.registry.borrow_mut();
        if registry.full() {
            return raise("Cannot register more than 32 collision groups in this world.");
        }
        if registry.id(&name).is_none() {
            registry.add(&name);
            drop(registry);
            shared.changed(engine);
        }
        Ok(none())
    }));
    let shared = groups.clone();
    add("CreateCollisionGroup", method(move |lua, engine, _, args| {
        let name = name_arg(lua, engine, args, 1)?;
        check_name(&name)?;
        server_only(engine)?;
        let mut registry = shared.registry.borrow_mut();
        if registry.full() {
            return raise("Cannot register more than 32 collision groups.");
        }
        if registry.id(&name).is_some() {
            return raise("Could not register collision group, the name already exists.");
        }
        let id = registry.add(&name);
        drop(registry);
        shared.changed(engine);
        Ok(values([LuaValue::Number(id as f64)]))
    }));
    add("UnregisterCollisionGroup", unregister(groups.clone(), true));
    add("RemoveCollisionGroup", unregister(groups.clone(), false));
    let shared = groups.clone();
    add("RenameCollisionGroup", method(move |lua, engine, _, args| {
        let to = name_arg(lua, engine, args, 2)?;
        let from = name_arg(lua, engine, args, 1)?;
        server_only(engine)?;
        check_name(&from)?;
        check_name(&to)?;
        let mut registry = shared.registry.borrow_mut();
        if let (Some(id), None) = (registry.id(&from), registry.id(&to)) {
            registry.names[id] = Some(to.into());
            drop(registry);
            shared.changed(engine);
        }
        Ok(none())
    }));
    let shared = groups.clone();
    add("CollisionGroupSetCollidable", method(move |lua, engine, _, args| {
        let collidable = bool_arg(engine, args, 3)?;
        let second = name_arg(lua, engine, args, 2)?;
        let first = name_arg(lua, engine, args, 1)?;
        server_only(engine)?;
        let mut registry = shared.registry.borrow_mut();
        let (Some(a), Some(b)) = (registry.id(&first), registry.id(&second)) else { return raise("Both collision groups must be registered.") };
        registry.set_collidable(a, b, collidable);
        drop(registry);
        shared.replicate(engine);
        Ok(none())
    }));

    let shared = groups.clone();
    add("CollisionGroupContainsPart", method(move |lua, engine, _, args| {
        let part = instance_arg(args, 2)?;
        let name = name_arg(lua, engine, args, 1)?;
        let state = engine.state.borrow();
        let Some(part) = part.filter(|&part| shared.is_part(&engine.reflection, &state, part)) else {
            return raise("Parameter 2 must be BasePart in CollisionGroupContainsPart.");
        };
        let held = state.dom.raw_get(part, shared.group).and_then(Value::as_str).is_some_and(|held| held.as_bytes() == name.as_slice());
        Ok(values([LuaValue::Boolean(held)]))
    }));
    let shared = groups.clone();
    add("SetPartCollisionGroup", method(move |lua, engine, _, args| {
        let name = name_arg(lua, engine, args, 2)?;
        let part = instance_arg(args, 1)?;
        let part = part.filter(|&part| shared.is_part(&engine.reflection, &engine.state.borrow(), part));
        let Some(part) = part else { return raise("Parameter 1 must be BasePart in SetPartCollisionGroup.") };
        shared.write_name(engine, part, &name);
        Ok(none())
    }));

    // BasePart.CollisionGroup: a string as a property write coerces one, then written as SetPartCollisionGroup writes it;
    // CollisionGroupId: a plain write, which starts the parts' tracking
    if let Some(info) = reflection.class_property(base_part, group).cloned() {
        let shared = groups.clone();
        behaviors.add_setter(base_part, group, Rc::new(move |lua, engine, id, value| {
            if let Value::String(name) = convert::coerce(lua, engine, &info, &value)? {
                shared.write_name(engine, id, name.as_bytes());
            }
            Ok(())
        }));
    }
    if let Some(info) = reflection.class_property(base_part, group_id).cloned() {
        let shared = groups.clone();
        behaviors.add_setter(base_part, group_id, Rc::new(move |lua, engine, id, value| {
            let value = convert::coerce(lua, engine, &info, &value)?;
            shared.registry.borrow_mut().active = true;
            crate::lua::instance::set_property(engine, id, group_id, value);
            Ok(())
        }));
    }

    // A part that comes into the Workspace (with the model it is in) takes its group's id, at what a group change spends on the
    // parts it works out ([`Resolved::charge`])
    let weak: Weak<Engine> = Rc::downgrade(engine);
    engine.hooks.borrow_mut().hierarchy_changed.push(Box::new(move |state, id, old, new| {
        if !groups.registry.borrow().active {
            return;
        }
        let Some(engine) = weak.upgrade() else { return };
        let Some(world) = engine.world.get() else { return };
        let inside = |parent: Option<InstanceId>| parent.is_some_and(|parent| parent == world.workspace || state.dom.is_descendant_of_home(parent, world.workspace));
        if !inside(new) || inside(old) {
            return;
        }
        let reflection = &engine.reflection;
        let mut parts = if groups.is_part(reflection, state, id) { vec![id] } else { Vec::new() };
        parts.extend(state.dom.descendants_where(id, |instance| reflection.class(instance.class).is_a(groups.base_part)));
        let hooks = engine.hooks.borrow();
        let registry = groups.registry.borrow();
        let mut resolved = Resolved { parts: parts.len(), changed: 0 };
        for part in parts {
            resolved.changed += usize::from(groups.resolve(state, reflection, &hooks, &registry, part));
        }
        resolved.charge_in(state);
    }));
}
