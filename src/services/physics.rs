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
//!   is new to it), and a mask alone leaves them, as SetCollidable does on the server. The parts it works out cost what a
//!   client spends on them (cost.rs CLIENT_GROUP_PART, CLIENT_GROUP_CHANGED: 30 ns a part looked at and 1.86 us more a part
//!   whose id changes, a 0.747 loopback client timed frame by frame); a snapshot of the groups it holds looks at no part (the
//!   join's Default alone from a server that never touched its groups). Not matched: a mask alone costs a client about 45 ns
//!   a part, not charged. A server and Studio's edit DataModel may change it (the edit one by Roblox's documentation of the
//!   Collision Groups editor, not measured).
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
//!   the Workspace pays the same for the parts it brings, as they take their ids. The Workspace's other instances cost it next
//!   to nothing (0.747 server, a register and an unregister timed together, the median of 11: 1.8-3.4 us beside nothing,
//!   4.0-6.8 beside 100,000 Folders flat or in chains of 100, 5.5 beside 1,000 parts, 8.4-9.5 beside both, 94 beside 20,000
//!   parts), so ree takes the parts from the spatial queries' broadphase, which holds them (world/query.rs `placed_parts`), and
//!   the Terrain, which it leaves out, and looks at no other instance. The parts whose ids change take them in the order the
//!   Workspace's descendants are listed (broadphase.rs `sort_by_order`), and the walk up from them that orders them costs what
//!   a spatial query's does past its allowance (cost.rs QUERY_ORDER_STEP: parts whose paths meet only far up). The broadphase
//!   holds the parts of the WorldModels in the Workspace too, which follow their own world: the Workspace's change looks at
//!   each, at a part's price, and leaves it out.
//! - ree charges the same for a WorldModel's own group change on the parts it works out (those of the WorldModel, not those of
//!   a WorldModel inside it), for a change of UseWorkspaceCollisionGroups on the parts that works out again, and for a tree
//!   that comes into a WorldModel's world, or moves between worlds, on the parts that take their ids (both passes for the parts
//!   of a WorldModel that uses the Workspace's groups); none of these was measured apart. A group change reads no index of a
//!   WorldModel's parts (world/broadphase.rs `WorldIndex`, made at its world's first spatial query or physics step): ree finds
//!   them by a walk of its descendants, which pays for each other instance it looks at what a recursive find pays for a
//!   node (cost.rs FIND_NODE, a price borrowed: where a server's change of the Workspace's groups costs next to nothing
//!   beside its other instances, a WorldModel's was not measured), a loopback client's copy included. Whether a WorldModel
//!   outside the Workspace is in the DataModel is looked for up its ancestors, which a method call, a part's CollisionGroup
//!   write and a change of UseWorkspaceCollisionGroups pay for as IsDescendantOf does (cost.rs ANCESTOR_STEP), once a call; a
//!   Parent write pays for the chains it walks already. A part's CollisionGroup write below WorldModels pays for them what any
//!   part write pays (lua/instance.rs `assign`, cost.rs world_levels), for three changes when it changed the part's
//!   CollisionGroupId and two when it did not, as a playing client counts them (Studio 0.748, in the Workspace and below a
//!   WorldModel in ReplicatedStorage alike: cost.rs part_changes). A write that changes nothing (the name the part has, or
//!   "") pays nothing for them, as any write that changes nothing (Studio 0.747), and "" costs 1 us more than such a write,
//!   whatever the levels (cost.rs COLLISION_GROUP_IGNORED).
//!
//! WorldRoot's eight collision-group methods are these on the world they are called on (Studio 0.748.0.7481526, a playing
//! client and its server, script identity 2: tools/parity/probes/world_root_groups.luau, world_root_group_args.luau,
//! world_model_groups.luau, world_model_group_places.luau, world_model_group_corners.luau, collision_group_warnings.luau,
//! loopback/world_model_groups.luau):
//!
//! - The Workspace's are PhysicsService's: one registry, the same rules, the same client refusal.
//! - A WorldModel has groups of its own, Default alone when it is made, with the same names, casts, limit and errors. A
//!   Clone gets a copy of them; Destroy keeps them. A client may change them only while the WorldModel is outside the
//!   DataModel (in nil, or under a parentless instance): anywhere in it (the Workspace, ReplicatedStorage, a ViewportFrame,
//!   the Camera) the four changes raise "This API can only be used on the server!", in the same place as the Workspace's
//!   (after the casts, before the names are checked). A server's WorldModel's groups reach a client's copy of it: in
//!   loopback, with the copy (which takes them before its tree enters, and again when it comes back into the replicated
//!   containers), and then once for each server frame that changed them, in the place of the frame's first change, as the
//!   DataModel's do, under the shared registry's rule for its parts' ids (again only when a group's name changed, or the
//!   groups are new to the client).
//! - With UseWorkspaceCollisionGroups true, the methods still work on the WorldModel's own groups, and the first call that
//!   gets past its casts warns once for that WorldModel (no stack): "<its full name>: UseWorkspaceCollisionGroups is true,
//!   so this WorldModel's own collision groups are not the ones used for its simulation. ..." A Clone warns again.
//! - A part's CollisionGroupId follows the world it is in: its nearest WorldModel, else the Workspace, as long as that
//!   world is in the DataModel. A WorldModel's world uses its own groups, or the Workspace's while UseWorkspaceCollisionGroups
//!   is true. A part takes its id when it comes into a world (its WorldModel entering the DataModel too) and when its name
//!   changes there; a world's parts take theirs again when its own registry changes (the Workspace's reaches no WorldModel's
//!   part, one with UseWorkspaceCollisionGroups true included), each firing Changed as a write does. A change of
//!   UseWorkspaceCollisionGroups works the WorldModel's parts out again with no Changed, over an id a script wrote. A part
//!   moved within its world keeps its id.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use mlua::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

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
/// PhysicsService's methods WorldRoot has too
const WORLD_ROOT_METHODS: &[&str] = &["CollisionGroupSetCollidable", "CollisionGroupsAreCollidable", "GetMaxCollisionGroups", "GetRegisteredCollisionGroups", "IsCollisionGroupRegistered", "RegisterCollisionGroup", "RenameCollisionGroup", "UnregisterCollisionGroup"];
/// What a WorldModel with UseWorkspaceCollisionGroups true warns once, after its full name and ": " (Studio 0.748)
const OWN_GROUPS_UNUSED: &str = "UseWorkspaceCollisionGroups is true, so this WorldModel's own collision groups are not the ones used for its simulation. If you meant to access or change the simulated groups, set UseWorkspaceCollisionGroups to false or edit them on the Workspace.";

/// One world's groups: the DataModel's, or a WorldModel's own.
#[derive(Clone)]
struct Registry {
    /// The group with each id, 0 (Default) to 31
    names: [Option<Box<[u8]>>; MAX_GROUPS],
    /// Each id's mask: bit i set when the group collides with group i (a free id's is all ones)
    masks: [u32; MAX_GROUPS],
}

impl Registry {
    fn new() -> Self {
        let mut names: [Option<Box<[u8]>>; MAX_GROUPS] = std::array::from_fn(|_| None);
        names[0] = Some(DEFAULT.into());
        Self { names, masks: [u32::MAX; MAX_GROUPS] }
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

    /// Default alone, colliding with itself: what a WorldModel's groups are until its first change (a free id's mask is never
    /// read, and with no other group Default's has every other bit set).
    fn is_default(&self) -> bool {
        self.names[1..].iter().all(Option::is_none) && self.masks[0] == u32::MAX
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { names: self.names.clone(), masks: self.masks, touched: false }
    }

    fn from_snapshot(snapshot: Snapshot) -> Self {
        Self { names: snapshot.names, masks: snapshot.masks }
    }
}

/// A world's collision-group registry as replicated from server to client (loopback/bridge.rs): the DataModel's, or a
/// WorldModel's.
#[derive(Clone)]
pub struct Snapshot {
    pub names: [Option<Box<[u8]>>; MAX_GROUPS],
    pub masks: [u32; MAX_GROUPS],
    /// A frame's snapshot: one of the changes since the last flush left the registry with more than Default (or Default not
    /// colliding with all), so the client follows it from now on, as that change's own snapshot would have made it (false at
    /// the join, and for a WorldModel sent whole that did not change since the last flush, whose snapshot decides that alone)
    pub touched: bool,
}

/// A world with groups: the DataModel's (PhysicsService's and the Workspace's, its parts those of the Workspace that are in
/// no WorldModel), or a WorldModel's own.
#[derive(Clone, Copy, PartialEq)]
enum Owner {
    Shared,
    Model(InstanceId),
}

/// What the methods, the part setters and the hooks share: one engine's registries and the names they use.
pub(crate) struct Groups {
    /// The DataModel's groups
    registry: RefCell<Registry>,
    /// Each WorldModel's own groups, by instance, from its first change (one never changed reads as Default alone);
    /// dropped when the instance is freed
    models: RefCell<FxHashMap<InstanceId, Registry>>,
    /// Default alone, what a WorldModel with no groups of its own reads: one for every read, not one made for each
    default: Registry,
    /// The WorldModels that warned that their own groups are not the ones used (once each)
    warned: RefCell<FxHashSet<InstanceId>>,
    /// A loopback server's WorldModels whose groups changed since its last flush, each with whether one of those changes left
    /// them other than Default alone ([`Snapshot::touched`]): its log holds a marker for each ([`Groups::queue_model`])
    unsent: RefCell<FxHashMap<InstanceId, bool>>,
    /// The markers logged since the last flush: as many WorldModels as `unsent` has held since (the flush takes out those it
    /// sends), the size [`flush_done`] keeps its table at
    logged: Cell<usize>,
    /// A loopback server's registry changed since the frame's last flush: its log holds the one marker that sends it
    /// ([`Groups::replicate`]), and whether a change left it other than Default alone ([`Snapshot::touched`])
    queued: Cell<bool>,
    touched: Cell<bool>,
    /// The run has touched collision groups: parts' CollisionGroupId follows the registries from now on (the module's notes)
    tracking: Cell<bool>,
    /// The last CollisionGroup write changed its part's CollisionGroupId ([`wrote_id`])
    wrote_id: Cell<bool>,
    world_root: ClassId,
    world_model: ClassId,
    base_part: ClassId,
    /// The Terrain, a part the broadphase leaves out ([`Groups::resolve_placed`])
    terrain: Option<ClassId>,
    /// BasePart.CollisionGroup and CollisionGroupId, WorldModel.UseWorkspaceCollisionGroups
    group: NameId,
    group_id: NameId,
    use_workspace: NameId,
}

impl Groups {
    /// The groups a method called on `id` works on: a WorldModel's own, else the DataModel's (PhysicsService, the Workspace).
    fn world(&self, engine: &Engine, id: InstanceId) -> Owner {
        let state = engine.state.borrow();
        let model = state.dom.get(id).is_some_and(|instance| engine.reflection.class(instance.class).is_a(self.world_model));
        if model { Owner::Model(id) } else { Owner::Shared }
    }

    fn read<R>(&self, world: Owner, read: impl FnOnce(&Registry) -> R) -> R {
        match world {
            Owner::Shared => read(&self.registry.borrow()),
            Owner::Model(id) => match self.models.borrow().get(&id) {
                Some(registry) => read(registry),
                None => read(&self.default),
            },
        }
    }

    /// A WorldModel's registry is made by its first change: a call that leaves Default alone (an unknown name unregistered or
    /// renamed, a refused CollisionGroupSetCollidable) makes none, so none is sent to a loopback client or copied by Clone.
    /// One it has stays when it is back to Default alone, so a loopback client's copy is sent that too.
    fn write<R>(&self, world: Owner, write: impl FnOnce(&mut Registry) -> R) -> R {
        match world {
            Owner::Shared => write(&mut self.registry.borrow_mut()),
            Owner::Model(id) => {
                let mut models = self.models.borrow_mut();
                if let Some(registry) = models.get_mut(&id) {
                    return write(registry);
                }
                let mut registry = Registry::new();
                let result = write(&mut registry);
                if !registry.is_default() {
                    models.insert(id, registry);
                }
                result
            }
        }
    }

    fn uses_workspace(&self, state: &State, model: InstanceId) -> bool {
        matches!(state.dom.raw_get(model, self.use_workspace), Some(Value::Bool(true)))
    }

    /// The registry a world's parts take their ids from: a WorldModel with UseWorkspaceCollisionGroups true uses the
    /// DataModel's.
    fn used(&self, state: &State, world: Owner) -> Owner {
        match world {
            Owner::Model(model) if self.uses_workspace(state, model) => Owner::Shared,
            world => world,
        }
    }

    /// A WorldModel's first call (past its casts) while UseWorkspaceCollisionGroups is true warns once, naming it by its full
    /// name at what a client spends building one (lua::instance::full_name).
    fn warn_unused(&self, engine: &Engine, world: Owner) {
        let Owner::Model(model) = world else { return };
        if !self.uses_workspace(&engine.state.borrow(), model) || self.warned.borrow().contains(&model) {
            return;
        }
        let name = crate::lua::instance::full_name(engine, model);
        self.warned.borrow_mut().insert(model);
        engine.warn(format!("{name}: {OWN_GROUPS_UNUSED}"));
    }

    /// A playing client reads every world's groups but may change only a WorldModel's that is outside the DataModel. Returns
    /// whether it found the WorldModel outside it (a client's change, which looked: its parts are in no world), so
    /// [`Groups::changed`] does not look again.
    fn server_only(&self, engine: &Engine, world: Owner) -> Result<bool, Flow> {
        if engine.options.run_context != RunContext::Client || engine.options.edit_mode() {
            return Ok(false);
        }
        let inside = match world {
            Owner::Shared => true,
            Owner::Model(model) => engine.world.get().is_some_and(|data_model| self.model_inside(&engine.state.borrow(), data_model, model)),
        };
        if inside { raise("This API can only be used on the server!") } else { Ok(true) }
    }

    /// A script's change of a world's groups (a server's, a client's of a WorldModel outside the DataModel, or in edit): when
    /// a group came, went or changed its name (`ids`), the parts that use them take their ids again, at what a server spends
    /// on that ([`Resolved::charge`]); a loopback client hears of it. `outside`: [`Groups::server_only`] found the WorldModel
    /// outside the DataModel, so there are no parts to work out and no walk up to look for it again.
    fn changed(&self, engine: &Engine, world: Owner, ids: bool, outside: bool) {
        if ids {
            self.tracking.set(true);
            if !outside {
                self.resolve_world(engine, world).charge(engine);
            }
        }
        match world {
            Owner::Shared => self.replicate(engine),
            Owner::Model(model) => self.queue_model(engine, model),
        }
    }

    fn snapshot(&self) -> Snapshot {
        self.registry.borrow().snapshot()
    }

    /// A loopback client takes the groups its server sent, the DataModel's or a WorldModel's copy's, at the join or once in a
    /// frame's changes. Groups other than Default alone, or a frame whose changes left them so ([`Snapshot::touched`]), start
    /// its parts' tracking. The parts that use them take their ids again, unless the parts were already followed and no
    /// group's name changed (a WorldModel's copy with no groups yet holds Default alone): a mask alone leaves the ids, as
    /// CollisionGroupSetCollidable leaves them on the server. One rule for both kinds of snapshot. A snapshot of the groups the
    /// client holds that leaves its tracking as it was (the join's Default alone from a server that never touched its groups)
    /// changes nothing and looks at no part. The parts it works out cost what a client spends on them (cost.rs CLIENT_GROUP_PART,
    /// CLIENT_GROUP_CHANGED), and a WorldModel's walk to them, ree's own work, what the server's change pays for it.
    fn apply(&self, engine: &Engine, world: Owner, snapshot: Snapshot) {
        let followed = self.tracking.get();
        let touched = snapshot.touched;
        let registry = Registry::from_snapshot(snapshot);
        let (held, renamed) = self.read(world, |held| (held.names == registry.names && held.masks == registry.masks, held.names != registry.names));
        let follows = followed || touched || !registry.is_default();
        if held && follows == followed {
            return;
        }
        self.tracking.set(follows);
        match world {
            Owner::Shared => *self.registry.borrow_mut() = registry,
            Owner::Model(model) => {
                self.models.borrow_mut().insert(model, registry);
            }
        }
        if !followed || renamed {
            self.resolve_world(engine, world).charge_client(engine);
        }
    }

    /// A change a loopback client hears of: the first since the frame's last flush logs a marker, in its place among the
    /// frame's changes, and the flush sends the registry as it is then (loopback/replicate.rs flush, [`flushed`]), as a
    /// property's changes in a frame go out once with its last value. A frame that changes the groups many times sends one
    /// snapshot, and its client takes the parts' ids in once.
    fn replicate(&self, engine: &Engine) {
        let mut state = engine.state.borrow_mut();
        let Some(log) = state.loopback_log.as_mut() else { return };
        if !self.registry.borrow().is_default() {
            self.touched.set(true);
        }
        if !self.queued.replace(true) {
            log.push(crate::loopback::replicate::Change::CollisionGroups);
        }
    }

    /// A loopback server's WorldModel whose groups changed: the first change since the frame's last flush logs a marker, in its
    /// place among the frame's changes, and the flush sends its groups as they are then ([`flushed_model`]), once however often
    /// they changed, noting whether a change left them other than Default alone, as [`Groups::replicate`] does for the
    /// DataModel's. A WorldModel with no groups of its own has nothing to send.
    fn queue_model(&self, engine: &Engine, model: InstanceId) {
        let mut state = engine.state.borrow_mut();
        let Some(log) = state.loopback_log.as_mut() else { return };
        let Some(touched) = self.models.borrow().get(&model).map(|registry| !registry.is_default()) else { return };
        let mut unsent = self.unsent.borrow_mut();
        if let Some(held) = unsent.get_mut(&model) {
            *held |= touched;
        } else {
            unsent.insert(model, touched);
            self.logged.set(self.logged.get() + 1);
            log.push(crate::loopback::replicate::Change::WorldGroups(model));
        }
    }

    fn is_part(&self, reflection: &Reflection, state: &State, id: InstanceId) -> bool {
        reflection.class(state.dom.at(id).class).is_a(self.base_part)
    }

    /// Whether `node` is in the DataModel: up the tree to it.
    fn inside(state: &State, data_model: crate::World, node: Option<InstanceId>) -> bool {
        node.is_some_and(|node| node == data_model.game || state.dom.is_descendant_of(node, data_model.game))
    }

    /// Whether the WorldModel `model` is in the DataModel, and how many of its ancestors that looked at: one in the Workspace is
    /// found from the homes the dom keeps (dom.rs Dom::is_descendant_of_home) and looks at none; another is looked for up the
    /// tree, a parent at a time.
    fn model_inside_walked(&self, state: &State, data_model: crate::World, model: InstanceId) -> (bool, usize) {
        if state.dom.is_descendant_of_home(model, data_model.workspace) {
            return (true, 0);
        }
        state.dom.is_descendant_of_steps(model, data_model.game)
    }

    /// [`Groups::model_inside_walked`]'s answer for a method call, a part's CollisionGroup write or a change of
    /// UseWorkspaceCollisionGroups, which pay for the ancestors it looked at as IsDescendantOf does (cost.rs ANCESTOR_STEP:
    /// lua::instance::descendant_of), on the state the caller holds. A Parent write's hook does not: the move paid for the
    /// ancestor chains of the old parent and the new one (cost.rs reparent_chains), and a WorldModel above either is in its
    /// chain.
    fn model_inside(&self, state: &State, data_model: crate::World, model: InstanceId) -> bool {
        let (inside, steps) = self.model_inside_walked(state, data_model, model);
        charge_on(state, steps as f64 * crate::cost::ANCESTOR_STEP);
        inside
    }

    /// The world of what is under `node` (a part's parent): the nearest WorldRoot at or above it, found from the homes the dom
    /// keeps (dom.rs Dom::home_step: one step a WorldRoot above it, however deep it is). The Workspace, or a WorldModel while it
    /// is in the DataModel; None elsewhere. `paid`: whether the caller paid for the walk up a WorldModel's chain already (a
    /// Parent write's hook), else it is charged here ([`Groups::model_inside`]).
    fn world_at(&self, state: &State, reflection: &Reflection, data_model: crate::World, node: Option<InstanceId>, paid: bool) -> Option<Owner> {
        let mut at = node;
        while let Some(id) = at {
            if id == data_model.workspace {
                return Some(Owner::Shared);
            }
            let class = reflection.class(state.dom.at(id).class);
            if class.is_a(self.world_model) {
                let inside = if paid { self.model_inside_walked(state, data_model, id).0 } else { self.model_inside(state, data_model, id) };
                return inside.then_some(Owner::Model(id));
            }
            if class.is_a(self.world_root) {
                return None;
            }
            at = state.dom.home_step(id);
        }
        None
    }

    /// The parts whose nearest world is the WorldModel `root`'s: its descendants but those in a WorldModel below it, in
    /// GetDescendants' order, and how many other instances the walk looked at (no index is read for them: the module's
    /// notes).
    fn parts_of(&self, state: &State, reflection: &Reflection, root: InstanceId) -> (Vec<InstanceId>, usize) {
        let mut parts = Vec::new();
        let mut walked = 0;
        let mut stack: Vec<InstanceId> = state.dom.at(root).children.iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            let instance = state.dom.at(node);
            let class = reflection.class(instance.class);
            if class.is_a(self.base_part) {
                parts.push(node);
            } else {
                walked += 1;
                if class.is_a(self.world_model) {
                    continue;
                }
            }
            stack.extend(instance.children.iter().rev().copied());
        }
        (parts, walked)
    }

    /// The id of the group the part's CollisionGroup names in `registry`, -1 for a name that is not registered, as
    /// CollisionGroupId holds it.
    fn group_id_of(&self, state: &State, registry: &Registry, part: InstanceId) -> Value {
        let id = state.dom.raw_get(part, self.group).and_then(Value::as_str).and_then(|name| registry.id(name.as_bytes()));
        Value::Number(id.map_or(-1.0, |id| id as f64))
    }

    /// The part takes the id its CollisionGroup has in `world`'s registry ([`Groups::group_id_of`]): as a write does (Changed
    /// fires when it differs), or `quiet`ly. Returns whether the id changed.
    fn resolve(&self, state: &mut State, reflection: &Reflection, hooks: &Hooks, world: Owner, part: InstanceId, quiet: bool) -> bool {
        let value = self.read(world, |registry| self.group_id_of(state, registry, part));
        if quiet {
            core::raw_set_unsignalled(state, reflection, hooks, part, self.group_id, value)
        } else {
            core::raw_set(state, reflection, hooks, part, self.group_id, value)
        }
    }

    /// The parts of the world that uses `world`'s registry take their ids again: a group came, went or changed its name. The
    /// DataModel's reaches the Workspace's own parts ([`Groups::resolve_placed`]); a WorldModel's its parts while it is in the
    /// DataModel and uses its own groups ([`Groups::resolve_model`]).
    fn resolve_world(&self, engine: &Engine, world: Owner) -> Resolved {
        match world {
            Owner::Shared => self.resolve_placed(engine),
            Owner::Model(model) => self.resolve_model(engine, model),
        }
    }

    /// The Workspace's own parts take the DataModel's ids again. The parts are those the spatial queries' broadphase holds
    /// (world/query.rs `placed_parts`, brought up to the dom's changes), and the Terrain in the Workspace, which it leaves out:
    /// the Workspace's other instances are not looked at, as a server's change costs next to nothing more beside 100,000
    /// Folders (the module's notes). The broadphase holds the parts of the WorldModels in the Workspace too, whose world is
    /// their WorldModel's: each is looked at and left out (its home is not the Workspace). A part whose id stays stores it,
    /// firing nothing, in the broadphase's order; the parts whose ids change take theirs, firing Changed, in the order the
    /// Workspace's descendants are listed (broadphase.rs `sort_by_order`), the order a walk of the Workspace gave them. Returns
    /// how many parts it looked at, how many of their ids changed and what putting those in order costs.
    fn resolve_placed(&self, engine: &Engine) -> Resolved {
        let Some(data_model) = engine.world.get() else { return Resolved::default() };
        let mut parts = Vec::new();
        crate::world::query::placed_parts(engine, &mut parts);
        let reflection = &engine.reflection;
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        looked_at(parts.len());
        if let Some(terrain) = self.terrain {
            let terrains = state.dom.instances_of_class(terrain);
            looked_at(terrains.len());
            parts.extend(terrains.into_iter().filter(|&id| state.dom.is_descendant_of_home(id, data_model.workspace)));
        }
        let registry = self.registry.borrow();
        let mut changes = Vec::new();
        for &part in &parts {
            if state.dom.home(part) != Some(data_model.workspace) {
                continue;
            }
            let id = self.group_id_of(&state, &registry, part);
            if state.dom.raw_get(part, self.group_id).is_some_and(|held| held.same(&id)) {
                core::raw_set(&mut state, reflection, &hooks, part, self.group_id, id);
            } else {
                changes.push((part, id));
            }
        }
        let steps = crate::world::broadphase::sort_by_order(&state.dom, data_model.workspace, &mut changes, |(part, _)| *part);
        let mut resolved = Resolved { parts: parts.len(), ordered: crate::world::broadphase::order_cost(steps, changes.len()), ..Resolved::default() };
        for (part, id) in changes {
            resolved.changed += usize::from(core::raw_set(&mut state, reflection, &hooks, part, self.group_id, id));
        }
        resolved
    }

    /// The WorldModel `model`'s own parts take its ids again, while it is in the DataModel and uses its own groups: those the
    /// walk of its descendants finds ([`Groups::parts_of`]), in that order. Returns how many parts it looked at, how many of
    /// their ids changed and how many other instances the walk looked at.
    fn resolve_model(&self, engine: &Engine, model: InstanceId) -> Resolved {
        let Some(data_model) = engine.world.get() else { return Resolved::default() };
        let reflection = &engine.reflection;
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        if self.uses_workspace(&state, model) || !self.model_inside(&state, data_model, model) {
            return Resolved::default();
        }
        let (parts, walked) = self.parts_of(&state, reflection, model);
        looked_at(parts.len() + walked);
        let mut resolved = Resolved { parts: parts.len(), walked, ..Resolved::default() };
        for part in parts {
            resolved.changed += usize::from(self.resolve(&mut state, reflection, &hooks, Owner::Model(model), part, false));
        }
        resolved
    }

    /// A part's CollisionGroup written, by the property or SetPartCollisionGroup: "" is ignored and a longer name cut to
    /// 100 bytes; a change in a world works out its CollisionGroupId.
    fn write_name(&self, engine: &Engine, part: InstanceId, name: &[u8]) {
        self.tracking.set(true);
        self.wrote_id.set(false);
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
        let Some(data_model) = engine.world.get() else { return };
        let parent = state.dom.at(part).parent;
        if let Some(world) = self.world_at(&state, reflection, data_model, parent, false) {
            let world = self.used(&state, world);
            self.wrote_id.set(self.resolve(&mut state, reflection, &hooks, world, part, false));
        }
    }
}

/// Whether the behaviour setter of the property `name` that just ran was CollisionGroup's and changed its part's
/// CollisionGroupId: a client counts that write one more change of the part, which lua/instance.rs `assign` charges for the
/// WorldModels above it (cost.rs part_changes: 3, else 2). A write leaves the id where it works no id out (out of every world
/// in the DataModel) and where the new name has the old one's id.
pub(crate) fn wrote_id(engine: &Engine, name: NameId) -> bool {
    engine.physics.get().is_some_and(|groups| name == groups.group && groups.wrote_id.get())
}

/// Charges the work clock `seconds` on the state the caller holds, as [`Resolved::charge_in`] does.
fn charge_on(state: &State, seconds: f64) {
    if seconds > 0.0 {
        state.sched.charge(seconds);
        crate::boundary::add_cost(seconds);
    }
}

#[cfg(test)]
thread_local! {
    /// How many instances the group changes have looked at on this thread, each Terrain and each part they work out, and each
    /// other instance a WorldModel's walk looks at (tests)
    pub(crate) static LOOKED_AT: Cell<u64> = const { Cell::new(0) };
}

/// Counts instances a group change looked at ([`LOOKED_AT`], tests)
fn looked_at(_count: usize) {
    #[cfg(test)]
    LOOKED_AT.with(|looked| looked.set(looked.get() + _count as u64));
}

/// The parts a group change looked at, those whose CollisionGroupId it changed, and ree's own walks for them.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct Resolved {
    parts: usize,
    changed: usize,
    /// What the walks that put the changed parts in the Workspace's order cost past their allowance (broadphase.rs
    /// `order_cost`: ree's own work, for parts whose paths meet only far up), in seconds
    ordered: f64,
    /// The other instances a walk of a WorldModel's descendants looked at for its parts ([`Groups::parts_of`]: ree's own work,
    /// at a recursive find's price for a node, cost.rs FIND_NODE)
    walked: usize,
}

impl Resolved {
    /// What a server spends on them beyond the call (cost.rs COLLISION_GROUP_PART, COLLISION_GROUP_CHANGED), and ree's walks
    /// that order them and that find a WorldModel's parts.
    fn charge(self, engine: &Engine) {
        self.charge_in(&engine.state.borrow());
    }

    /// [`Resolved::charge`] where the state is borrowed already.
    fn charge_in(self, state: &State) {
        self.charge_at(state, crate::cost::COLLISION_GROUP_PART, crate::cost::COLLISION_GROUP_CHANGED);
    }

    /// What a client spends on them taking in a snapshot of its server's groups (cost.rs CLIENT_GROUP_PART, CLIENT_GROUP_CHANGED),
    /// and ree's walks.
    fn charge_client(self, engine: &Engine) {
        self.charge_at(&engine.state.borrow(), crate::cost::CLIENT_GROUP_PART, crate::cost::CLIENT_GROUP_CHANGED);
    }

    fn charge_at(self, state: &State, part: f64, changed: f64) {
        let parts = self.parts as f64 * part + self.changed as f64 * changed;
        charge_on(state, parts + self.ordered + self.walked as f64 * crate::cost::FIND_NODE);
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
    method(move |lua, engine, id, args| {
        let name = name_arg(lua, engine, args, 1)?;
        let world = groups.world(engine, id);
        groups.warn_unused(engine, world);
        let outside = if side_first {
            let outside = groups.server_only(engine, world)?;
            check_name(&name)?;
            outside
        } else {
            check_name(&name)?;
            groups.server_only(engine, world)?
        };
        let removed = groups.write(world, |registry| registry.id(&name).map(|id| registry.remove(id)).is_some());
        if removed {
            groups.changed(engine, world, true, outside);
        }
        Ok(none())
    })
}

/// What a spatial query of the WorldRoot `root` reads of the group `name` its params give (world/query.rs Filter): which groups
/// it collides with, as bits by id, in the registry the parts of that world take their ids from (a WorldModel's own while it uses
/// them, else the DataModel's); None when it collides with every group: a name that is not registered there (measured, Studio
/// 0.747, a server and a loopback client alike: tools/parity/probes/query_collision_groups.luau), or a run that never touched
/// collision groups (Default alone, colliding with itself).
/// Also whether the name is a registered group other than Default (a query with such params costs a client more: cost.rs
/// QUERY_PARAMS_GROUP), and the parts' CollisionGroupId (whose value is a part's group id).
pub(crate) fn query_mask(engine: &Engine, root: InstanceId, name: &[u8]) -> (Option<(u32, NameId)>, bool) {
    let Some(groups) = engine.physics.get() else { return (None, false) };
    if !groups.tracking.get() {
        return (None, false);
    }
    let world = {
        let state = engine.state.borrow();
        let model = state.dom.get(root).is_some_and(|instance| engine.reflection.class(instance.class).is_a(groups.world_model));
        groups.used(&state, if model { Owner::Model(root) } else { Owner::Shared })
    };
    let id = groups.read(world, |registry| registry.id(name).map(|id| (id, registry.masks[id])));
    (id.map(|(_, mask)| mask).filter(|&mask| mask != u32::MAX).map(|mask| (mask, groups.group_id)), id.is_some_and(|(id, _)| id != 0))
}

/// Clone gives each WorldModel's copy the original's groups.
pub(crate) fn cloned(engine: &Engine, pairs: &[(InstanceId, InstanceId)]) {
    let Some(groups) = engine.physics.get() else { return };
    let mut models = groups.models.borrow_mut();
    if models.is_empty() {
        return;
    }
    for &(original, copy) in pairs {
        if let Some(registry) = models.get(&original).cloned() {
            models.insert(copy, registry);
        }
    }
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
        groups.apply(engine, Owner::Shared, snapshot);
    }
}

/// The WorldModels with groups of their own, as they are, for a loopback client's join: each copy the join made takes them.
pub(crate) fn model_snapshots(engine: &Engine) -> Vec<(InstanceId, Snapshot)> {
    let Some(groups) = engine.physics.get() else { return Vec::new() };
    groups.models.borrow().iter().map(|(&id, registry)| (id, registry.snapshot())).collect()
}

/// A loopback server's flush met the marker of a WorldModel whose groups changed ([`Groups::queue_model`]) and the client has
/// its copy: its groups as they are now, once for the frame's changes ([`Snapshot::touched`] as they left them); None when
/// they went with the WorldModel's tree in this flush ([`sent_models`]).
pub(crate) fn flushed_model(engine: &Engine, model: InstanceId) -> Option<Snapshot> {
    let groups = engine.physics.get()?;
    let touched = groups.unsent.borrow_mut().remove(&model)?;
    groups.models.borrow().get(&model).map(|registry| Snapshot { touched, ..registry.snapshot() })
}

/// The WorldModels with groups of their own among `ids` (a tree a loopback server sends whole, or an instance it sends again
/// as it comes back into the replicated containers), each with its groups, by its place among `ids`: the client's copy takes
/// them before the tree enters, with the frame's changes to them ([`Snapshot::touched`]), so their marker sends nothing more.
/// Nothing is looked up while no WorldModel has groups.
pub(crate) fn sent_models(engine: &Engine, ids: impl Iterator<Item = InstanceId>) -> Vec<(u32, Snapshot)> {
    let Some(groups) = engine.physics.get() else { return Vec::new() };
    let models = groups.models.borrow();
    if models.is_empty() {
        return Vec::new();
    }
    let mut unsent = groups.unsent.borrow_mut();
    ids.enumerate()
        .filter_map(|(index, id)| {
            let registry = models.get(&id)?;
            let touched = unsent.remove(&id).unwrap_or(false);
            Some((index as u32, Snapshot { touched, ..registry.snapshot() }))
        })
        .collect()
}

/// A loopback server's flush is done: the WorldModels whose markers it met while the client had no copy of them to send to
/// log a marker again at their next change (a copy the client is sent later comes with its groups: [`sent_models`]). Their
/// table is emptied as dom.rs `clear_set` empties a set, at the size of what the frame logged ([`Groups::logged`]): after a
/// frame that changed the groups of a million WorldModels, a flush that empties one entry rewrites no table of millions.
pub(crate) fn flush_done(engine: &Engine) {
    if let Some(groups) = engine.physics.get() {
        crate::dom::clear_map(&mut groups.unsent.borrow_mut(), groups.logged.replace(0));
    }
}

/// A loopback client's copy of a server WorldModel takes its groups (its parts their ids, under the rule the DataModel's
/// follow: [`Groups::apply`]).
pub(crate) fn apply_model_snapshot(engine: &Engine, model: InstanceId, snapshot: Snapshot) {
    if let Some(groups) = engine.physics.get() {
        groups.apply(engine, Owner::Model(model), snapshot);
    }
}

/// Whether the engine's parts' CollisionGroupId follows the registries yet (the module's notes)
#[cfg(test)]
pub(crate) fn tracking(engine: &Engine) -> bool {
    engine.physics.get().is_some_and(|groups| groups.tracking.get())
}

/// The room the table of a loopback server's WorldModels with changes to send has ([`Groups::unsent`]; tests)
#[cfg(test)]
pub(crate) fn unsent_room(engine: &Engine) -> usize {
    engine.physics.get().map_or(0, |groups| groups.unsent.borrow().capacity())
}

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let classes = (reflection.class_id("PhysicsService"), reflection.class_id("WorldRoot"), reflection.class_id("WorldModel"), reflection.class_id("BasePart"));
    let names = (reflection.name_id("CollisionGroup"), reflection.name_id("CollisionGroupId"), reflection.name_id("UseWorkspaceCollisionGroups"));
    let ((Some(service), Some(world_root), Some(world_model), Some(base_part)), (Some(group), Some(group_id), Some(use_workspace))) = (classes, names) else { return };
    let groups = Rc::new(Groups {
        registry: RefCell::new(Registry::new()),
        models: RefCell::default(),
        default: Registry::new(),
        warned: RefCell::default(),
        unsent: RefCell::default(),
        logged: Cell::new(0),
        queued: Cell::new(false),
        touched: Cell::new(false),
        tracking: Cell::new(false),
        wrote_id: Cell::new(false),
        world_root,
        world_model,
        base_part,
        terrain: reflection.class_id("Terrain"),
        group,
        group_id,
        use_workspace,
    });
    let _ = engine.physics.set(groups.clone());
    // (WorldRoot's eight are PhysicsService's on the world they are called on: the Workspace's are the DataModel's groups, a
    // WorldModel's its own)
    let mut add = |name: &str, function: MethodFn| {
        let Some(name_id) = reflection.name_id(name) else { return };
        if WORLD_ROOT_METHODS.contains(&name) {
            behaviors.add_method(world_root, name_id, function.clone());
        }
        behaviors.add_method(service, name_id, function);
    };

    let shared = groups.clone();
    add("GetMaxCollisionGroups", method(move |_, engine, id, _| {
        shared.warn_unused(engine, shared.world(engine, id));
        Ok(values([LuaValue::Number(MAX_GROUPS as f64)]))
    }));
    let shared = groups.clone();
    add("GetRegisteredCollisionGroups", method(move |lua, engine, id, _| {
        let world = shared.world(engine, id);
        shared.warn_unused(engine, world);
        Ok(values([LuaValue::Table(shared.read(world, |registry| registry.list(lua, false))?)]))
    }));
    let shared = groups.clone();
    add("GetCollisionGroups", method(move |lua, _, _, _| Ok(values([LuaValue::Table(shared.registry.borrow().list(lua, true)?)]))));
    let shared = groups.clone();
    add("IsCollisionGroupRegistered", method(move |lua, engine, id, args| {
        let name = name_arg(lua, engine, args, 1)?;
        let world = shared.world(engine, id);
        shared.warn_unused(engine, world);
        Ok(values([LuaValue::Boolean(shared.read(world, |registry| registry.id(&name).is_some()))]))
    }));
    let shared = groups.clone();
    add("CollisionGroupsAreCollidable", method(move |lua, engine, id, args| {
        let second = name_arg(lua, engine, args, 2)?;
        let first = name_arg(lua, engine, args, 1)?;
        let world = shared.world(engine, id);
        shared.warn_unused(engine, world);
        let collidable = shared.read(world, |registry| match (registry.id(&first), registry.id(&second)) {
            (Some(a), Some(b)) => registry.masks[a] & (1 << b) != 0,
            _ => true,
        });
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
    add("RegisterCollisionGroup", method(move |lua, engine, id, args| {
        let name = name_arg(lua, engine, args, 1)?;
        let world = shared.world(engine, id);
        shared.warn_unused(engine, world);
        let outside = shared.server_only(engine, world)?;
        check_name(&name)?;
        let added = shared.write(world, |registry| {
            if registry.full() {
                return raise("Cannot register more than 32 collision groups in this world.");
            }
            if registry.id(&name).is_some() {
                return Ok(false);
            }
            registry.add(&name);
            Ok(true)
        })?;
        if added {
            shared.changed(engine, world, true, outside);
        }
        Ok(none())
    }));
    let shared = groups.clone();
    add("CreateCollisionGroup", method(move |lua, engine, _, args| {
        let name = name_arg(lua, engine, args, 1)?;
        check_name(&name)?;
        shared.server_only(engine, Owner::Shared)?;
        let id = {
            let mut registry = shared.registry.borrow_mut();
            if registry.full() {
                return raise("Cannot register more than 32 collision groups.");
            }
            if registry.id(&name).is_some() {
                return raise("Could not register collision group, the name already exists.");
            }
            registry.add(&name)
        };
        shared.changed(engine, Owner::Shared, true, false);
        Ok(values([LuaValue::Number(id as f64)]))
    }));
    add("UnregisterCollisionGroup", unregister(groups.clone(), true));
    add("RemoveCollisionGroup", unregister(groups.clone(), false));
    let shared = groups.clone();
    add("RenameCollisionGroup", method(move |lua, engine, id, args| {
        let to = name_arg(lua, engine, args, 2)?;
        let from = name_arg(lua, engine, args, 1)?;
        let world = shared.world(engine, id);
        shared.warn_unused(engine, world);
        let outside = shared.server_only(engine, world)?;
        check_name(&from)?;
        check_name(&to)?;
        let renamed = shared.write(world, |registry| match (registry.id(&from), registry.id(&to)) {
            (Some(id), None) => {
                registry.names[id] = Some(to.into());
                true
            }
            _ => false,
        });
        if renamed {
            shared.changed(engine, world, true, outside);
        }
        Ok(none())
    }));
    let shared = groups.clone();
    add("CollisionGroupSetCollidable", method(move |lua, engine, id, args| {
        let collidable = bool_arg(engine, args, 3)?;
        let second = name_arg(lua, engine, args, 2)?;
        let first = name_arg(lua, engine, args, 1)?;
        let world = shared.world(engine, id);
        shared.warn_unused(engine, world);
        let outside = shared.server_only(engine, world)?;
        shared.write(world, |registry| {
            let (Some(a), Some(b)) = (registry.id(&first), registry.id(&second)) else { return raise("Both collision groups must be registered.") };
            registry.set_collidable(a, b, collidable);
            Ok(())
        })?;
        shared.changed(engine, world, false, outside);
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
                // (the "" it ignores costs a client more than another write that changes nothing: cost.rs COLLISION_GROUP_IGNORED)
                if name.is_empty() {
                    charge_on(&engine.state.borrow(), crate::cost::COLLISION_GROUP_IGNORED);
                }
                shared.write_name(engine, id, name.as_bytes());
            }
            Ok(())
        }));
    }
    if let Some(info) = reflection.class_property(base_part, group_id).cloned() {
        let shared = groups.clone();
        behaviors.add_setter(base_part, group_id, Rc::new(move |lua, engine, id, value| {
            let value = convert::coerce(lua, engine, &info, &value)?;
            shared.tracking.set(true);
            crate::lua::instance::set_property(engine, id, group_id, value);
            Ok(())
        }));
    }

    // A WorldModel's groups go with it
    let shared = groups.clone();
    engine.hooks.borrow_mut().instance_freed.push(Box::new(move |_, id| {
        shared.models.borrow_mut().remove(&id);
        shared.warned.borrow_mut().remove(&id);
        shared.unsent.borrow_mut().remove(&id);
    }));

    // UseWorkspaceCollisionGroups changed: a WorldModel in the DataModel's parts take their ids from the other registry,
    // with no Changed, at what a group change costs on them (charged on the state the hook is given: its caller holds it)
    let weak: Weak<Engine> = Rc::downgrade(engine);
    let shared = groups.clone();
    engine.hooks.borrow_mut().property_changed.push(Box::new(move |state, id, name| {
        if name != shared.use_workspace || !shared.tracking.get() {
            return;
        }
        let Some(engine) = weak.upgrade() else { return };
        let Some(data_model) = engine.world.get() else { return };
        let reflection = &engine.reflection;
        if !reflection.class(state.dom.at(id).class).is_a(shared.world_model) || !shared.model_inside(state, data_model, id) {
            return;
        }
        let world = shared.used(state, Owner::Model(id));
        let hooks = engine.hooks.borrow();
        let (parts, walked) = shared.parts_of(state, reflection, id);
        looked_at(parts.len() + walked);
        let mut resolved = Resolved { parts: parts.len(), walked, ..Resolved::default() };
        for part in parts {
            resolved.changed += usize::from(shared.resolve(state, reflection, &hooks, world, part, true));
        }
        resolved.charge_in(state);
    }));

    // A part that comes into a world (with the model it is in, or with its WorldModel entering the DataModel) takes its
    // group's id, at what a group change spends on the parts it works out ([`Resolved::charge`]); one moved within its world
    // keeps its id
    let weak: Weak<Engine> = Rc::downgrade(engine);
    engine.hooks.borrow_mut().hierarchy_changed.push(Box::new(move |state, id, old, new| {
        if !groups.tracking.get() {
            return;
        }
        let Some(engine) = weak.upgrade() else { return };
        let Some(data_model) = engine.world.get() else { return };
        let reflection = &engine.reflection;
        // (the worlds of the tree's new place and its old one, from the homes the dom keeps: a lookup a WorldRoot above each,
        // and for a WorldModel outside the Workspace a walk up its chain, which the Parent write paid for)
        let after = groups.world_at(state, reflection, data_model, new, true);
        let before = groups.world_at(state, reflection, data_model, old, true);
        if after.is_some() && before == after {
            return;
        }
        // (a WorldModel in the moved tree is its parts' world while it is in the DataModel: whether a place in no world is in
        // it is looked for up the tree, and only while there is a WorldModel at all, so that a tree moved outside the worlds
        // of a place with none costs the two lookups)
        let models = state.dom.count_of_class(groups.world_model) > 0;
        let inside_after = after.is_some() || (models && Groups::inside(state, data_model, new));
        if !inside_after {
            return;
        }
        let inside_before = before.is_some() || (models && Groups::inside(state, data_model, old));
        if after.is_none() && before.is_none() && inside_before {
            return;
        }
        // (each part with its world before and after the move: a WorldModel in the moved tree is its parts' world either way,
        // in the DataModel after it and before it if the tree was)
        let mut moved: Vec<(InstanceId, Owner)> = Vec::new();
        let mut stack: Vec<(InstanceId, Option<InstanceId>)> = vec![(id, None)];
        while let Some((node, inner)) = stack.pop() {
            let class = reflection.class(state.dom.at(node).class);
            if class.is_a(groups.base_part) {
                let (was, is) = match inner {
                    Some(model) => (inside_before.then_some(Owner::Model(model)), Some(Owner::Model(model))),
                    None => (before, after),
                };
                if let Some(is) = is
                    && was != Some(is)
                {
                    moved.push((node, is));
                }
            }
            let inner = if class.is_a(groups.world_model) { Some(node) } else { inner };
            stack.extend(state.dom.at(node).children.iter().rev().map(|&child| (child, inner)));
        }
        // (from its world's own groups, then, for a WorldModel with UseWorkspaceCollisionGroups true, from the Workspace's
        // with no Changed: measured, a part whose id the WorldModel's own groups leave as it was fires none; each pass is a
        // part worked out, as a group change of either registry would charge it)
        let hooks = engine.hooks.borrow();
        let mut resolved = Resolved { parts: moved.len(), ..Resolved::default() };
        for (part, world) in moved {
            resolved.changed += usize::from(groups.resolve(state, reflection, &hooks, world, part, false));
            if let Owner::Shared = groups.used(state, world)
                && world != Owner::Shared
            {
                resolved.parts += 1;
                resolved.changed += usize::from(groups.resolve(state, reflection, &hooks, Owner::Shared, part, true));
            }
        }
        resolved.charge_in(state);
    }));
}
