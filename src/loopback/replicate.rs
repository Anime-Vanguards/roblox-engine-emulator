//! Replication, server to client: the copy of the replicated containers at the join, then the server's changes there, one
//! frame's at a time, in the stream with its remote traffic. Measured in a play session's two sides (Studio 0.747,
//! tools/parity/probes/loopback; docs/engine.md, Log 2026-11-19):
//!
//! - What reaches the client: the children of [`CONTAINERS`] (ReplicatedFirst's only at the join), not ServerStorage's
//!   or ServerScriptService's; the server's own Camera and the harness's RURFixtures stay the server's.
//! - Of an instance: its class, Name, every property but those rbx-dom marks NotReplicated (LocalTransparencyModifier among
//!   them, measured), its attributes and tags; a property holding an instance the client lacks arrives as nil, an attribute
//!   holding one as an InstanceHandle of none (Studio 0.748: [`attribute_ref`]); a server
//!   Script without its source (one whose RunContext is Client with it), a LocalScript or ModuleScript with it.
//! - When: a frame's changes go out when the frame ends, each as its value then, in the order first made: an instance made
//!   and written in the frame arrives whole, three writes to one property give one Changed with the last value, a write
//!   and its revert give none, two properties' Changed come in the order first written, and remote events keep their
//!   place among them (an instance made, parented and passed to FireClient arrives before the event).
//! - Hierarchy: an instance the server unparents or moves out of the containers keeps its copy on the client with Parent
//!   nil (it comes back as the same instance); a destroyed one is destroyed there.
//! - Order: each side keeps its lists as a client does (dom.rs Children: out of a list of more than 20 the last child takes a
//!   taken one's place), so the server's changes leave the client's lists in the server's order; but the join's copy lists
//!   each list in the order its children came into the containers ([`Ranks`]: tools/parity/probes/loopback/child_order.luau).
//!
//! The client applies them at its receive point through the engine's setters, so its events fire, deferred, as for a
//! script's change; a physics move arrives without Changed. Nothing the client does reaches the server.

use std::rc::Rc;

use mlua::prelude::*;
use rustc_hash::FxHashSet;

use super::{Endpoint, Side, bridge::{Bridge, Msg, Node}};
use crate::{ClassId, Engine, InstanceId, K, NameId, Reflection, State, dom::flags, instance as core, value::{Str, Value}};

/// The services whose children reach the client (measured: a marker Folder in each service the server can parent to,
/// before and after the join, tools/parity/probes/loopback/replicated_services.luau). ReplicatedFirst's only at the join.
pub const CONTAINERS: [&str; 19] = [
    "Workspace",
    "ReplicatedStorage",
    "ReplicatedFirst",
    "Lighting",
    "Players",
    "StarterGui",
    "StarterPack",
    "StarterPlayer",
    "Teams",
    "SoundService",
    "Chat",
    "TextChatService",
    "LocalizationService",
    "MaterialService",
    "TestService",
    "InsertService",
    "MarketplaceService",
    "ProximityPromptService",
    "PolicyService",
];

/// The properties whose values stay on the server, by (class, name; None: any class). Measured (Studio 0.747,
/// tools/parity/probes/loopback/replicated_properties.luau: 70 properties of 25 classes written on the server, before and
/// after the join): every one arrives but LocalTransparencyModifier. rbx-dom's NotReplicated tag is not used: it marks a
/// property not sent as itself (a script's Enabled, sent as Disabled; a part's Color, Size and Position), whose value arrives
/// all the same. A Workspace's CurrentCamera is each side's own; Parent is the hierarchy's. A player's Team each side works
/// out from its TeamColor, Neutral and that side's teams (services/teams.rs; loopback/team_links.luau: the client's events
/// are those of the TeamColor and Neutral it receives).
const NOT_REPLICATED: [(Option<&str>, &str); 3] = [(None, "LocalTransparencyModifier"), (Some("Workspace"), "CurrentCamera"), (Some("Player"), "Team")];

/// When each server instance came into the replicated containers, from the link to the join's copy: the order a joining client
/// is sent each list in (Studio 0.747, client and server: tools/parity/probes/loopback/child_order.luau). The server keeps its
/// lists as a client does (dom.rs Children), but the copy lists a list's children in the order they came into the containers:
/// one taken out closes its gap there, one that leaves the containers and comes back goes last, one moved to another list in
/// them and back keeps its place, and a tree that comes in is ranked in its walk's order then (a list it brings keeps the order
/// it had). The instances there at the link are ranked in the server's order then; each that comes in later is ranked after
/// every other ([`install_ranks`]); the copy lists each list by rank, a child with none (made there without a Parent change:
/// a lazily made one) right after the child before it ([`push_joined`]).
#[derive(Default)]
pub struct Ranks {
    /// By dom slot, the instance's rank (0: none)
    by_slot: Vec<u64>,
    next: u64,
    /// Ranking (from the link to the join's copy)
    on: bool,
}

impl Ranks {
    #[inline]
    fn of(&self, id: InstanceId) -> u64 {
        self.by_slot.get(id.index as usize).copied().unwrap_or(0)
    }

    /// Ranks `root` and what is below it after every instance ranked so far, in GetDescendants' order
    fn rank_tree(&mut self, state: &State, root: InstanceId) {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            self.next += 1;
            let slot = node.index as usize;
            if self.by_slot.len() <= slot {
                self.by_slot.resize(slot + 1, 0);
            }
            self.by_slot[slot] = self.next;
            stack.extend(state.dom.at(node).children.iter().rev());
        }
    }
}

/// Whether `at` is one of the replicated containers or inside one (`classes`: theirs)
fn in_containers(state: &State, game: InstanceId, classes: &[ClassId], mut at: Option<InstanceId>) -> bool {
    while let Some(node) = at {
        let Some(instance) = state.dom.get(node) else { return false };
        if instance.parent == Some(game) {
            return classes.contains(&instance.class);
        }
        at = instance.parent;
    }
    false
}

/// Starts the server's [`Ranks`] (a loopback pair's link): what the containers hold now is ranked in the server's order, and a
/// hook ranks each instance that comes into them until the join's copy.
pub fn install_ranks(server: &Rc<Engine>, bridge: &Rc<Bridge>) {
    let game = server.world().game;
    let classes: Vec<ClassId> = CONTAINERS.iter().filter_map(|&name| server.reflection.class_id(name)).collect();
    let excluded = excluded(server);
    {
        let state = server.state.borrow();
        let mut ranks = bridge.ranks.borrow_mut();
        ranks.on = true;
        for (_, service) in containers(server) {
            for &child in state.dom.at(service).children.iter().filter(|child| !excluded.contains(child)) {
                ranks.rank_tree(&state, child);
            }
        }
    }
    let mut hooks = server.hooks.borrow_mut();
    let ranks = bridge.clone();
    hooks.hierarchy_changed.push(Box::new(move |state, id, old, new| {
        let mut ranks = ranks.ranks.borrow_mut();
        if ranks.on && new.is_some() && in_containers(state, game, &classes, new) && !in_containers(state, game, &classes, old) {
            ranks.rank_tree(state, id);
        }
    }));
    let ranks = bridge.clone();
    hooks.instance_freed.push(Box::new(move |_, id| {
        if let Some(rank) = ranks.ranks.borrow_mut().by_slot.get_mut(id.index as usize) {
            *rank = 0;
        }
    }));
}

/// Pushes `node`'s children onto the join's copy `stack` (each with `copy`, its client copy), so they pop in the order the copy
/// lists them in: by rank ([`Ranks`]), a child with none right after the child before it in the server's list. `keyed` is
/// scratch.
fn push_joined(state: &State, ranks: &Ranks, node: InstanceId, copy: InstanceId, keyed: &mut Vec<(u64, InstanceId)>, stack: &mut Vec<(InstanceId, InstanceId)>) {
    keyed.clear();
    let (mut key, mut sorted) = (0, true);
    for &child in &state.dom.at(node).children {
        let rank = ranks.of(child);
        if rank != 0 {
            sorted &= rank > key;
            key = rank;
        }
        keyed.push((key, child));
    }
    if !sorted {
        crate::sort::by_key(keyed, |&(key, _)| key);
    }
    stack.extend(keyed.iter().rev().map(|&(_, child)| (child, copy)));
}

/// What the server did this frame that the client may have to hear of, in the order first done (sealed at the frame's
/// end by [`flush`]: values are read then).
pub enum Change {
    Moved(InstanceId),
    Prop(InstanceId, NameId, bool),
    Attr(InstanceId, Str),
    Tag(InstanceId, Str),
    /// A remote message, in its place among the changes
    Message(Msg),
    /// The collision-group registry changed (services/physics.rs): logged at the first change since the last flush, which
    /// sends the registry as it then is in this place, once however often it changed
    CollisionGroups,
    /// A WorldModel's own collision groups changed (services/physics.rs): logged at their first change since the last flush,
    /// which sends them as they then are in this place, once however often they changed
    WorldGroups(InstanceId),
}

/// Which properties replicate ([`NOT_REPLICATED`])
#[derive(Default)]
pub struct Filter {
    names: Vec<NameId>,
    pairs: Vec<(ClassId, NameId)>,
}

impl Filter {
    pub fn new(reflection: &Reflection) -> Filter {
        let mut filter = Filter::default();
        for (class, name) in NOT_REPLICATED {
            let Some(name) = reflection.name_id(name) else { continue };
            match class {
                None => filter.names.push(name),
                Some(class) => {
                    if let Some(class) = reflection.class_id(class) {
                        filter.pairs.push((class, name));
                    }
                }
            }
        }
        filter
    }

    /// Whether property `name` of an instance of `class` replicates
    #[inline]
    pub fn replicates(&self, reflection: &Reflection, class: ClassId, name: NameId) -> bool {
        if name == K::Parent as NameId || self.names.contains(&name) {
            return false;
        }
        !self.pairs.iter().any(|&(owner, denied)| denied == name && reflection.class(class).is_a(owner))
    }
}

/// The services of `engine` that replicate, by name, as they are now (found, not made)
fn containers(engine: &Engine) -> Vec<(&'static str, InstanceId)> {
    let game = engine.world().game;
    CONTAINERS.iter().filter_map(|&name| crate::services::find_service(engine, game, name).map(|id| (name, id))).collect()
}

/// Makes what waits below `root` that only the server can make and the join is to copy: a lazily imported plan's levels
/// not made yet (a level made later is made without the engine's hooks, so nothing would send it). A manifest bundle that
/// waits to import stays waiting (project.rs `bundle_waits`: the join copies its placeholder, and the server imports it when
/// either side looks: lazy.rs), but in a world (the Workspace, a WorldModel), whose walks and recorder read it whole with
/// no script looking, it is imported now with all below it, as before; the Studio snapshot's placeholders stay lazy (each
/// side makes its own) outside those.
fn make_lazy_regions(server: &Engine, root: InstanceId, excluded: &[InstanceId]) {
    let mut in_worlds = Vec::new();
    {
        let mut state = server.state.borrow_mut();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if excluded.contains(&node) {
                continue;
            }
            if crate::project::bundle_waits(&state, node) && state.dom.home(node).is_some() {
                in_worlds.push(node);
                continue;
            }
            if state.dom.at(node).has(flags::LAZY_PENDING) {
                crate::project::materialize_lazy(&mut state, &server.reflection, node);
            }
            state.dom.at(node).children.copy_to(&mut stack);
        }
    }
    for node in in_worlds {
        if server.state.borrow().dom.get(node).is_some() {
            crate::project::materialize_deep(server, node);
        }
    }
}

/// The copy flags an instance's copy takes of it: whether it is a Studio-only placeholder, whether its Studio children were
/// made (lazy.rs), whether it is a bundle's placeholder (project::model_assets: the client's copy waits for the server to
/// import it), and whether bundles wait below it (a move of the copy into the client's world imports them first)
const COPIED_FLAGS: u16 = flags::STUDIO_ONLY | flags::MATERIALIZED | flags::BUNDLE_ROOT | flags::LAZY_ABOVE;

/// What a copy keeps of its original's place in the Studio snapshot and its lazy state: the node (none for a script's
/// instance) and [`COPIED_FLAGS`]
#[inline]
fn studio_of(instance: &crate::dom::Instance) -> (crate::dom::StudioNode, u16) {
    let node = if instance.flags & flags::SCRIPT_CREATED != 0 { crate::dom::StudioNode::None } else { instance.studio_node };
    (node, instance.flags & COPIED_FLAGS)
}

/// Notes on the client which of the copies `ids` maps the server's instances `originals` to wait for bundles the server
/// imports when the client looks (project::model_assets `waits_remotely`)
#[cfg(feature = "bundles")]
fn mirror_waiting(server: &State, client: &mut State, ids: &super::bridge::Ids, originals: impl Iterator<Item = InstanceId>) {
    for original in originals {
        if let Some(copy) = ids.to_client.get(original)
            && client.dom.get(copy).is_some()
        {
            let waits = crate::project::bundle_waits(server, original);
            client.project.model_assets.set_waits_remotely(copy, waits);
        }
    }
}

/// The server instances that never replicate: its Camera, the harness's RURFixtures
fn excluded(server: &Engine) -> Vec<InstanceId> {
    let world = server.world();
    let mut out = vec![world.camera];
    let state = server.state.borrow();
    if let Some(storage) = crate::services::find_service(server, world.game, "ReplicatedStorage")
        && let Some(fixtures) = state.dom.find_child(storage, b"RURFixtures")
    {
        out.push(fixtures);
    }
    out
}

/// The join's copy of the server's replicated containers into the client, and the hooks that log the server's changes
/// from then on. The server's lazily made regions are made first (the copy is of instances). The loopback player pairs
/// with the client's local player, and the containers a server makes for it (Backpack, PlayerGui, StarterGear) with the
/// client's of their class, made where the client has none. No event fires for the copy: no client script runs yet.
pub fn join(server_lua: &Lua, server: &Rc<Engine>, client_lua: &Lua, client: &Rc<Engine>, bridge: &Rc<Bridge>, server_player: InstanceId, client_player: InstanceId) -> Result<(), String> {
    let _ = (server_lua, client_lua);
    let filter = Rc::new(Filter::new(&server.reflection));
    let excluded = excluded(server);
    // the client's services, made where it has none (with no state borrowed: services run code as they are made)
    let mut pairs = Vec::new();
    for (name, service) in containers(server) {
        // (what only the server can make, made now: a lazily imported plan's instances; a bundle that waits to import stays
        // waiting, and the Studio snapshot's placeholders stay lazy: lazy.rs)
        crate::timing::time("loopback: join: make the server's lazy regions", || make_lazy_regions(server, service, &excluded));
        let game = client.world().game;
        let copy = crate::services::get_service(client, game, name).map_err(|_| format!("the client cannot make {name}"))?;
        pairs.push((name, service, copy));
    }
    let copying = crate::timing::start("loopback: join: copy");
    let mut server_state = server.state.borrow_mut();
    let mut client_state = client.state.borrow_mut();
    crate::project::share_studio(&server_state, &mut client_state, client.world().game);
    let reflection = &client.reflection;
    let mut ids = bridge.ids.borrow_mut();
    let mut refs: Vec<(InstanceId, NameId, InstanceId)> = Vec::new();
    let mut attribute_refs: Vec<(InstanceId, Str, InstanceId)> = Vec::new();
    let mut created = 0u64;
    // (each list in the order its children came into the containers: Ranks, which stop here)
    let ranks = std::mem::take(&mut *bridge.ranks.borrow_mut());
    let mut keyed: Vec<(u64, InstanceId)> = Vec::new();
    let mut stack: Vec<(InstanceId, InstanceId)> = Vec::new();
    for &(name, service, copy) in &pairs {
        ids.pair(service, copy);
        copy_values(&server_state, &mut client_state, reflection, &filter, service, copy, false, (&mut refs, &mut attribute_refs), crate::services::tag_added);
        server_state.dom.at_mut(service).flags |= flags::SENT;
        push_joined(&server_state, &ranks, service, copy, &mut keyed, &mut stack);
        while let Some((node, parent)) = stack.pop() {
            if excluded.contains(&node) {
                continue;
            }
            let instance = server_state.dom.at(node);
            let (class, parent_of_node) = (instance.class, instance.parent);
            // the loopback player is the client's local one; its containers the client's of their class
            let paired = if node == server_player {
                Some(client_player)
            } else if parent_of_node == Some(server_player) {
                client_state.dom.at(client_player).children.iter().copied().find(|&child| client_state.dom.at(child).class == class && ids.to_server.get(child).is_none())
            } else {
                None
            };
            let made = match paired {
                Some(existing) => existing,
                None => {
                    created += 1;
                    let made = core::create(&mut client_state, reflection, class);
                    client_state.dom.attach(made, parent);
                    made
                }
            };
            ids.pair(node, made);
            copy_values(&server_state, &mut client_state, reflection, &filter, node, made, paired.is_none(), (&mut refs, &mut attribute_refs), crate::services::tag_added);
            server_state.dom.at_mut(node).flags |= flags::SENT;
            push_joined(&server_state, &ranks, node, made, &mut keyed, &mut stack);
        }
        let _ = name;
    }
    // instance values, once every copy is made: the client's copy, or nil for one it lacks
    for (copy, name, target) in refs {
        let value = ids.to_client.get(target).map_or(Value::Nil, Value::Instance);
        client_state.dom.at_mut(copy).store(name, value);
    }
    for (copy, name, target) in attribute_refs {
        set_attribute_ref(&mut client_state, copy, &name, ids.to_client.get(target));
    }
    // (the copies of the server's instances where bundles wait: the client's looks have the server import them)
    #[cfg(feature = "bundles")]
    {
        let waiting: Vec<InstanceId> = server_state.project.model_assets.waiting().collect();
        mirror_waiting(&server_state, &mut client_state, &ids, waiting.into_iter());
    }
    bridge.stats.borrow_mut().created += created;
    copying.stop();
    drop(ids);
    drop(client_state);
    drop(server_state);
    // the server's replicated instances from now on, and the hooks that log their changes (ReplicatedFirst replicates no
    // more after the join)
    {
        let mut state = server.state.borrow_mut();
        for &(name, service, _) in &pairs {
            if name == "ReplicatedFirst" {
                continue;
            }
            mark(&mut state, service, true, &excluded);
        }
    }
    // Copy the server's collision-group registry before client scripts run (tools/parity/probes/loopback/collision_groups.luau)
    if let Some(snapshot) = crate::services::physics::snapshot(server) {
        crate::services::physics::apply_snapshot(client, snapshot);
    }
    // and the own groups of the WorldModels the join copied
    for (id, snapshot) in crate::services::physics::model_snapshots(server) {
        let copy = bridge.ids.borrow().to_client.get(id);
        if let Some(copy) = copy {
            crate::services::physics::apply_model_snapshot(client, copy, snapshot);
        }
    }
    server.state.borrow_mut().loopback_log = Some(Vec::new());
    install(server, bridge, filter, excluded);
    install_client(client, bridge);
    Ok(())
}

/// An attribute's instance as the client holds it: its copy, or an InstanceHandle of none for one it lacks (measured, Studio
/// 0.748, a play session's server and client: an attribute holding an Instance or a handle of a server-only or parentless
/// instance reads as a handle of none on the client, set before the join and after;
/// tools/parity/probes/loopback/remote_instance_handles.luau)
fn attribute_ref(copy: Option<InstanceId>) -> Value {
    copy.map_or(Value::NoInstance, Value::Instance)
}

/// Gives a copy's attribute `name`, which holds the server's instance, the client's ([`attribute_ref`]), once every copy is made
fn set_attribute_ref(state: &mut State, copy: InstanceId, name: &Str, target: Option<InstanceId>) {
    if let Some(slot) = state.dom.at_mut(copy).attributes.own_mut().iter_mut().find(|(held, _)| held.as_bytes() == name.as_bytes()) {
        slot.1 = attribute_ref(target);
    }
}

/// Copies an instance's name, replicated properties, attributes, tags and script source into its client copy
/// (properties and attributes holding instances are noted in `refs`, set once every copy is made)
/// (a `fresh` copy, made for it, takes them as one vector in the server's name order, as Clone gives a copy its own; a
/// paired one of the client's keeps what it has, a service's camera say; `tag` notes each of its tags: the join's
/// [`crate::services::tag_added`], a bundle import's copy [`crate::services::note_tag`], as the import itself notes them)
#[allow(clippy::too_many_arguments)]
fn copy_values(
    server: &State, client: &mut State, reflection: &Reflection, filter: &Filter, from: InstanceId, to: InstanceId, fresh: bool,
    (refs, attribute_refs): (&mut Vec<(InstanceId, NameId, InstanceId)>, &mut Vec<(InstanceId, Str, InstanceId)>), tag: fn(&mut State, InstanceId, &Str),
) {
    let source = server.dom.at(from);
    let class = source.class;
    let script = script_source(reflection, source);
    let mut props: Vec<(NameId, Value)> = Vec::with_capacity(source.props.len());
    for (name, value) in source.props.iter() {
        if !filter.replicates(reflection, class, name) {
            continue;
        }
        if let Some(target) = value.as_instance() {
            refs.push((to, name, target));
            continue;
        }
        if let Some(value) = super::wire::portable(value) {
            props.push((name, value));
        }
    }
    let attributes: crate::dom::Attributes = source.attributes.iter().filter_map(|(name, value)| super::wire::portable(value).map(|value| (name.clone(), value))).collect::<Vec<_>>().into();
    for (name, value) in attributes.iter() {
        if let Value::Instance(target) = value {
            attribute_refs.push((to, name.clone(), *target));
        }
    }
    let tags = source.tags.clone();
    let json = source.flags & flags::JSON_MODULE;
    let (studio, studio_flags) = studio_of(source);
    let target = client.dom.at_mut(to);
    target.studio_node = studio;
    target.flags |= studio_flags;
    if fresh {
        target.props = crate::dom::Props::from(props);
    } else {
        for (name, value) in props {
            target.store(name, value);
        }
    }
    target.attributes = attributes;
    for tag in tags.iter() {
        if !target.tags.contains(tag) {
            target.tags.push(tag.clone());
        }
    }
    target.flags |= json;
    if let Some(script) = script {
        target.script = Some(Box::new(script));
    }
    // (a paired copy's values changed in place: the spatial queries' broadphase reads it again, world/broadphase.rs)
    if !fresh {
        client.dom.note_written(to);
    }
    client.dom.refresh_name(reflection, to);
    for name in tags.iter() {
        tag(client, to, name);
    }
}

/// What source a client copy of a script gets: a LocalScript's, a ModuleScript's and a Client-context Script's, not a server
/// Script's
fn script_source(reflection: &Reflection, instance: &crate::dom::Instance) -> Option<crate::dom::ScriptSource> {
    let script = instance.script.as_deref()?;
    let class = &*reflection.class(instance.class).name;
    let runs_on_client = match class {
        "LocalScript" | "ModuleScript" => true,
        "Script" => {
            let client = reflection.enum_item("RunContext", "Client");
            reflection.name_id("RunContext").and_then(|name| instance.props.get(name)).and_then(Value::as_enum).is_some_and(|item| Some(item) == client)
        }
        _ => false,
    };
    runs_on_client.then(|| script.clone())
}

/// Sets or clears [`flags::REPLICATED`] on `root` and what is below it (an excluded instance and its subtree stay out)
pub(super) fn mark(state: &mut State, root: InstanceId, on: bool, excluded: &[InstanceId]) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if on && excluded.contains(&node) {
            continue;
        }
        let instance = state.dom.at_mut(node);
        if on {
            instance.flags |= flags::REPLICATED;
        } else {
            if instance.flags & flags::SENT != 0 {
                instance.flags |= flags::STALE;
            }
            instance.flags &= !flags::REPLICATED;
        }
        state.dom.at(node).children.copy_to(&mut stack);
    }
}

/// The server's hooks: a moved, written, attributed or tagged instance that replicates is logged for the frame's flush
/// ([`flush`]); each tests the instance's flag first, so a change elsewhere costs one test.
fn install(server: &Rc<Engine>, bridge: &Rc<Bridge>, filter: Rc<Filter>, excluded: Vec<InstanceId>) {
    *bridge.filter.borrow_mut() = Some(filter);
    let mut hooks = server.hooks.borrow_mut();
    hooks.hierarchy_changed.push(Box::new(move |state, id, _old, new| {
        let was = state.dom.at(id).flags & flags::REPLICATED != 0;
        let now = new.is_some_and(|parent| state.dom.at(parent).flags & flags::REPLICATED != 0) && !excluded.contains(&id);
        if was != now {
            mark(state, id, now, &excluded);
        }
        if (was || now || state.dom.at(id).flags & flags::SENT != 0)
            && let Some(log) = state.loopback_log.as_mut()
        {
            log.push(Change::Moved(id));
        }
    }));
    hooks.property_changed.push(Box::new(move |state, id, name| {
        if state.dom.at(id).flags & flags::REPLICATED == 0 || name == K::Parent as NameId {
            return;
        }
        let quiet = state.quiet_write;
        if let Some(log) = state.loopback_log.as_mut() {
            log.push(Change::Prop(id, name, quiet));
        }
    }));
    hooks.attribute_changed.push(Box::new(move |state, id, name| {
        if state.dom.at(id).flags & flags::REPLICATED == 0 {
            return;
        }
        if let Some(log) = state.loopback_log.as_mut() {
            log.push(Change::Attr(id, Str::from(name)));
        }
    }));
    let log = bridge.clone();
    hooks.instance_freed.push(Box::new(move |_, id| {
        log.ids.borrow_mut().to_client.remove(id);
        let mut deferred = log.deferred.borrow_mut();
        if !deferred.is_empty() {
            deferred.remove(&id);
        }
    }));
}

/// The client's hook: a copy it frees forgets its server instance
pub fn install_client(client: &Rc<Engine>, bridge: &Rc<Bridge>) {
    let ids = bridge.clone();
    client.hooks.borrow_mut().instance_freed.push(Box::new(move |_, id| {
        ids.ids.borrow_mut().to_server.remove(id);
    }));
}

/// The server's frame ended: its logged changes become messages, each read now, in the order first made, with its remote
/// messages in their places ([`Change`]); then the batch is sealed (bridge.rs).
pub fn flush(server: &Engine, bridge: &Bridge) {
    let mut state = server.state.borrow_mut();
    let Some(log) = state.loopback_log.as_mut().filter(|log| !log.is_empty()).map(std::mem::take) else { return };
    // (the collision groups as they are now, if they changed since the last flush: sent in the place of the first change)
    let mut groups = crate::services::physics::flushed(server);
    let Some(filter) = bridge.filter.borrow().clone() else {
        crate::services::physics::flush_done(server);
        return;
    };
    let reflection = &server.reflection;
    let mut out: Vec<Msg> = Vec::with_capacity(log.len());
    let mut moved: FxHashSet<InstanceId> = FxHashSet::default();
    let mut written: FxHashSet<(InstanceId, NameId)> = FxHashSet::default();
    let mut attributed: FxHashSet<(InstanceId, Str)> = FxHashSet::default();
    let mut tagged: FxHashSet<(InstanceId, Str)> = FxHashSet::default();
    // instances whose whole state went out this flush (made, or brought back): their later changes are in it
    let mut whole: FxHashSet<InstanceId> = FxHashSet::default();
    let mut writes = 0u64;
    let mut created = 0u64;
    for change in log {
        match change {
            Change::Message(message) => out.push(message),
            Change::CollisionGroups => {
                if let Some(snapshot) = groups.take() {
                    out.push(Msg::CollisionGroups(snapshot));
                }
            }
            // (a WorldModel's own groups as they are now, in the place of their first change, to the copy the client has; one
            // sent whole in this flush took them with its tree)
            Change::WorldGroups(id) => {
                if sent_and_replicated(&state, id)
                    && let Some(snapshot) = crate::services::physics::flushed_model(server, id)
                {
                    out.push(Msg::WorldGroups { id, snapshot });
                }
            }
            Change::Moved(id) => {
                if moved.insert(id) {
                    created += flush_move(server, &mut state, reflection, &filter, id, &mut out, &mut whole);
                }
            }
            Change::Prop(id, name, quiet) => {
                if whole.contains(&id) || !written.insert((id, name)) || !sent_and_replicated(&state, id) {
                    continue;
                }
                let class = state.dom.at(id).class;
                if !filter.replicates(reflection, class, name) {
                    continue;
                }
                let value = state.dom.raw_get(id, name).and_then(super::wire::portable).unwrap_or(Value::Nil);
                writes += 1;
                out.push(Msg::Prop { id, name, value, quiet });
            }
            Change::Attr(id, name) => {
                if whole.contains(&id) || !sent_and_replicated(&state, id) || !attributed.insert((id, name.clone())) {
                    continue;
                }
                let value = state.dom.at(id).attribute(name.as_bytes()).and_then(super::wire::portable).unwrap_or(Value::Nil);
                out.push(Msg::Attr { id, name, value });
            }
            Change::Tag(id, tag) => {
                if whole.contains(&id) || !sent_and_replicated(&state, id) || !tagged.insert((id, tag.clone())) {
                    continue;
                }
                let added = state.dom.at(id).tags.contains(&tag);
                out.push(Msg::Tag { id, tag, added });
            }
        }
    }
    {
        let mut stats = bridge.stats.borrow_mut();
        stats.writes += writes;
        stats.created += created;
    }
    // (the WorldModels whose groups changed but which the client has no copy of: they log a marker again at their next change)
    crate::services::physics::flush_done(server);
    drop(state);
    bridge.outgoing(Side::Server).borrow_mut().open.extend(out);
}

/// An instance the client has a copy of (or will, from a message already sealed) that replicates now
#[inline]
fn sent_and_replicated(state: &State, id: InstanceId) -> bool {
    state.dom.get(id).is_some_and(|instance| instance.flags & (flags::SENT | flags::REPLICATED) == flags::SENT | flags::REPLICATED)
}

/// What a moved instance comes to: a tree the client makes (a new instance entering), a move of the client's copy (in or
/// out of the containers), a destroy, or nothing (never sent and not replicated). Returns how many instances it made.
fn flush_move(server: &Engine, state: &mut State, reflection: &Reflection, filter: &Filter, id: InstanceId, out: &mut Vec<Msg>, whole: &mut FxHashSet<InstanceId>) -> u64 {
    let Some(instance) = state.dom.get(id) else { return 0 };
    let sent = instance.flags & flags::SENT != 0;
    if instance.flags & flags::DESTROYED != 0 {
        if sent {
            out.push(Msg::Destroy { id });
        }
        return 0;
    }
    let now = instance.flags & flags::REPLICATED != 0;
    let parent = instance.parent;
    match (now, sent) {
        (true, false) => {
            let Some(parent) = parent else { return 0 };
            let nodes = tree(state, reflection, filter, id, whole);
            let count = nodes.len() as u64;
            let groups = crate::services::physics::sent_models(server, nodes.iter().map(|node| node.id));
            out.push(Msg::Tree { parent, nodes, groups });
            count
        }
        (true, true) => {
            // (the WorldModels coming back take their groups before they enter, as a new tree's do: those messages go in front
            // of the move)
            let at = out.len();
            let mut groups: Vec<Msg> = Vec::new();
            out.push(Msg::Parent { id, parent });
            // back in the containers after time out of them: what changed meanwhile, and children made meanwhile
            let mut made = 0;
            let mut stack = vec![id];
            while let Some(node) = stack.pop() {
                let flags_now = state.dom.at(node).flags;
                if flags_now & flags::REPLICATED == 0 {
                    continue;
                }
                if flags_now & flags::SENT == 0 {
                    let Some(parent) = state.dom.at(node).parent else { continue };
                    let nodes = tree(state, reflection, filter, node, whole);
                    made += nodes.len() as u64;
                    let tree_groups = crate::services::physics::sent_models(server, nodes.iter().map(|node| node.id));
                    out.push(Msg::Tree { parent, nodes, groups: tree_groups });
                    continue;
                }
                if flags_now & flags::STALE != 0 {
                    state.dom.at_mut(node).flags &= !flags::STALE;
                    whole.insert(node);
                    let class = state.dom.at(node).class;
                    for (name, value) in state.dom.at(node).props.iter() {
                        if filter.replicates(reflection, class, name)
                            && let Some(value) = super::wire::portable(value)
                        {
                            out.push(Msg::Prop { id: node, name, value, quiet: false });
                        }
                    }
                    for (name, value) in state.dom.at(node).attributes.iter() {
                        if let Some(value) = super::wire::portable(value) {
                            out.push(Msg::Attr { id: node, name: name.clone(), value });
                        }
                    }
                    groups.extend(crate::services::physics::sent_models(server, std::iter::once(node)).into_iter().map(|(_, snapshot)| Msg::WorldGroups { id: node, snapshot }));
                }
                stack.extend(state.dom.at(node).children.iter().rev());
            }
            out.splice(at..at, groups);
            made
        }
        (false, true) => {
            out.push(Msg::Parent { id, parent: None });
            0
        }
        (false, false) => 0,
    }
}

/// `root` and its replicated descendants as the client makes them (marked sent), in GetDescendants' order
fn tree(state: &mut State, reflection: &Reflection, filter: &Filter, root: InstanceId, whole: &mut FxHashSet<InstanceId>) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<(InstanceId, Option<u32>)> = vec![(root, None)];
    while let Some((id, parent)) = stack.pop() {
        let instance = state.dom.at(id);
        if instance.flags & flags::REPLICATED == 0 {
            continue;
        }
        let class = instance.class;
        // (in name order, as the instance stores them: the client's copy takes the vector as it is)
        let mut props: Vec<(NameId, Value)> = Vec::with_capacity(instance.props.len());
        for (name, value) in instance.props.iter() {
            if filter.replicates(reflection, class, name)
                && let Some(value) = super::wire::portable(value)
            {
                props.push((name, value));
            }
        }
        let (studio, studio_flags) = studio_of(instance);
        let node = Node {
            id,
            class,
            parent,
            props,
            attributes: instance.attributes.iter().filter_map(|(name, value)| super::wire::portable(value).map(|value| (name.clone(), value))).collect(),
            tags: instance.tags.iter().cloned().collect(),
            source: script_source(reflection, instance),
            json: instance.flags & flags::JSON_MODULE != 0,
            studio,
            studio_flags,
            waits: crate::project::bundle_waits(state, id),
        };
        let index = nodes.len() as u32;
        nodes.push(node);
        stack.extend(instance.children.iter().rev().map(|&child| (child, Some(index))));
        let target = state.dom.at_mut(id);
        target.flags |= flags::SENT;
        target.flags &= !flags::STALE;
        whole.insert(id);
    }
    nodes
}

/// Makes the levels a lazy import left for later in the subtrees `made` (what an import made): they would be made without the
/// engine's hooks, so nothing would send them; a client's copy and a tree take them whole
pub(super) fn make_whole(server: &Engine, made: &[InstanceId]) {
    let mut state = server.state.borrow_mut();
    let mut stack: Vec<InstanceId> = made.to_vec();
    while let Some(node) = stack.pop() {
        if state.dom.get(node).is_none() {
            continue;
        }
        if state.dom.at(node).has(flags::LAZY_PENDING) {
            crate::project::materialize_lazy(&mut state, &server.reflection, node);
        }
        state.dom.at(node).children.copy_to(&mut stack);
    }
}

/// Marks the subtrees `made` (what an import made below an instance the client has a copy of, or will have) [`flags::SENT`],
/// and [`flags::STALE`] too when that instance is out of the containers: its client copy holds them
pub(super) fn mark_sent(state: &mut State, original: InstanceId, made: &[InstanceId]) {
    let stale = state.dom.at(original).flags & flags::STALE;
    let mut stack: Vec<InstanceId> = made.to_vec();
    while let Some(node) = stack.pop() {
        let Some(instance) = state.dom.get_mut(node) else { continue };
        instance.flags |= flags::SENT | stale;
        state.dom.at(node).children.copy_to(&mut stack);
    }
}

/// A bundle import on the server at `original`, which the client holds `copy` of (lazy.rs `bundle_imported`; `made`: the
/// children the import made there, `removed`: those it took out): the copy takes what the import made at once, as the join
/// copies, with no event. The bundles were the place's from the join on, made when first looked at, as a plain run makes
/// them; so the client gets the instances as they were made, whichever side looked first and in whichever frame (a server's
/// write after it reaches the client as any other). Only what the import made is copied: a child a script parented there
/// meanwhile arrives as the server's changes do. What the import made is made whole on the server first ([`make_whole`]) and
/// replicates from now on as the join's copy does. A copy that stood for the bundle's root (its placeholder) becomes that root
/// in place, as the server's did: its class, values and flags. The children the import took out (the Studio-only placeholders
/// a bundle's members replace) leave the copy first, as a Parent message takes them out, so both lists keep one order (the
/// server's Parent message for them moves nothing then). Returns how many instances the client made.
pub(super) fn copy_import(server: &Engine, client: &Engine, bridge: &Bridge, original: InstanceId, copy: InstanceId, made: &[InstanceId], removed: &[InstanceId]) -> u64 {
    let Some(filter) = bridge.filter.borrow().clone() else { return 0 };
    // the client's copies of what the import took out, out of the copy
    let gone: Vec<InstanceId> = {
        let client_state = client.state.borrow();
        let ids = bridge.ids.borrow();
        removed.iter().filter_map(|&child| ids.to_client.get(child)).filter(|&child| client_state.dom.get(child).is_some_and(|instance| instance.parent == Some(copy))).collect()
    };
    // (the server's Parent message for each, at the client's next frame, pins the copy as it pins any the server took out)
    for child in gone {
        let _ = crate::lua::instance::set_parent(client, child, None);
    }
    make_whole(server, made);
    let reflection = &client.reflection;
    let mut copied: Vec<(InstanceId, InstanceId)> = Vec::new();
    {
        let mut server_state = server.state.borrow_mut();
        let mut client_state = client.state.borrow_mut();
        let mut ids = bridge.ids.borrow_mut();
        let mut refs: Vec<(InstanceId, NameId, InstanceId)> = Vec::new();
        let mut attribute_refs: Vec<(InstanceId, Str, InstanceId)> = Vec::new();
        // a placeholder becomes the bundle's root in place: the class, values and flags the server's took
        let placeholder = client_state.dom.at(copy).has(flags::BUNDLE_ROOT);
        if placeholder {
            client_state.dom.set_class(reflection, copy, server_state.dom.at(original).class);
            copy_values(&server_state, &mut client_state, reflection, &filter, original, copy, true, (&mut refs, &mut attribute_refs), crate::services::note_tag);
            let (_, kept) = studio_of(server_state.dom.at(original));
            client_state.dom.at_mut(copy).flags &= !((flags::STUDIO_ONLY | flags::BUNDLE_ROOT) & !kept);
            client_state.dom.note_written(copy);
        }
        // what the import made the client lacks, each with what is below it, after the copy's own children, in the server's
        // order (each list in its order: a node's children are pushed last first)
        let lacks = |ids: &super::bridge::Ids, client_state: &State, node: InstanceId| ids.to_client.get(node).is_none_or(|copy| client_state.dom.get(copy).is_none());
        let mut stack: Vec<(InstanceId, InstanceId)> = Vec::new();
        stack.extend(made.iter().rev().filter(|&&root| server_state.dom.get(root).is_some_and(|instance| instance.parent == Some(original)) && lacks(&ids, &client_state, root)).map(|&root| (root, copy)));
        while let Some((node, parent)) = stack.pop() {
            let class = server_state.dom.at(node).class;
            let made_copy = core::create(&mut client_state, reflection, class);
            client_state.dom.attach(made_copy, parent);
            ids.pair(node, made_copy);
            copy_values(&server_state, &mut client_state, reflection, &filter, node, made_copy, true, (&mut refs, &mut attribute_refs), crate::services::note_tag);
            copied.push((node, made_copy));
            stack.extend(server_state.dom.at(node).children.iter().rev().map(|&child| (child, made_copy)));
        }
        mark_sent(&mut server_state, original, made);
        // instance values, every copy made: the client's copy, or nil for one it lacks
        for (holder, name, target) in refs {
            let value = ids.to_client.get(target).map_or(Value::Nil, Value::Instance);
            client_state.dom.at_mut(holder).store(name, value);
        }
        for (holder, name, target) in attribute_refs {
            set_attribute_ref(&mut client_state, holder, &name, ids.to_client.get(target));
        }
        // (made in place, as the import made the server's: their joints are indexed as theirs are)
        for &(_, made_copy) in &copied {
            crate::world::joints::made_in_place(&mut client_state, reflection, made_copy);
        }
        if placeholder {
            crate::world::joints::made_in_place(&mut client_state, reflection, copy);
        }
        // (what still waits at it: loose bundles a look inside imports)
        #[cfg(feature = "bundles")]
        mirror_waiting(&server_state, &mut client_state, &ids, std::iter::once(original));
    }
    // the own collision groups of the WorldModels among them
    for (index, snapshot) in crate::services::physics::sent_models(server, copied.iter().map(|&(node, _)| node)) {
        if let Some(&(_, made_copy)) = copied.get(index as usize) {
            crate::services::physics::apply_model_snapshot(client, made_copy, snapshot);
        }
    }
    let count = copied.len() as u64;
    bridge.stats.borrow_mut().created += count;
    count
}

/// A replication message at the client's receive point: through the engine's setters, so its events queue as for a
/// script's change (deferred: they run at the legacy step's flush)
pub fn apply(lua: &Lua, client: &Rc<Engine>, endpoint: &Rc<Endpoint>, message: Msg) -> LuaResult<()> {
    let bridge = &endpoint.bridge;
    // (the server's Studio-only instances are paired as they are met: lazy.rs)
    let map = |id: InstanceId| super::lazy::to_client(client, endpoint, id);
    let live = |id: InstanceId| -> Option<InstanceId> {
        let copy = map(id)?;
        let state = client.state.borrow();
        state.dom.get(copy).filter(|instance| instance.flags & flags::DESTROYED == 0).map(|_| copy)
    };
    let mapped = |value: Value| -> Value {
        match value {
            Value::Instance(target) => map(target).map_or(Value::Nil, Value::Instance),
            other => other,
        }
    };
    match message {
        Msg::Tree { parent, mut nodes, groups } => {
            let Some(parent) = live(parent) else { return Ok(()) };
            let reflection = &client.reflection;
            let mut made: Vec<InstanceId> = Vec::with_capacity(nodes.len());
            let mut refs: Vec<(InstanceId, NameId, InstanceId)> = Vec::new();
            let mut attribute_refs: Vec<(InstanceId, Str, InstanceId)> = Vec::new();
            {
                let mut state = client.state.borrow_mut();
                let mut ids = bridge.ids.borrow_mut();
                for node in nodes.iter_mut() {
                    let copy = core::create(&mut state, reflection, node.class);
                    {
                        // (the server's values in name order, as Clone gives a copy its vector; instance values once
                        // every copy is made)
                        let mut props = std::mem::take(&mut node.props);
                        props.retain(|(name, value)| match value.as_instance() {
                            Some(instance) => {
                                refs.push((copy, *name, instance));
                                false
                            }
                            None => true,
                        });
                        let target = state.dom.at_mut(copy);
                        target.props = crate::dom::Props::from(props);
                        for (name, value) in node.attributes.iter() {
                            if let Value::Instance(instance) = value {
                                attribute_refs.push((copy, name.clone(), *instance));
                            }
                        }
                        target.attributes = std::mem::take(&mut node.attributes).into();
                        target.tags = node.tags.clone().into();
                        if node.json {
                            target.flags |= flags::JSON_MODULE;
                        }
                        if let Some(source) = &node.source {
                            target.script = Some(Box::new(source.clone()));
                        }
                        target.studio_node = node.studio;
                        target.flags |= node.studio_flags;
                    }
                    state.dom.refresh_name(reflection, copy);
                    if let Some(index) = node.parent {
                        state.dom.attach(copy, made[index as usize]);
                    }
                    ids.pair(node.id, copy);
                    made.push(copy);
                }
                for (node, &copy) in nodes.iter().zip(&made) {
                    for tag in &node.tags {
                        crate::services::tag_added(&mut state, copy, tag);
                    }
                    // (a copy of an instance where bundles wait: the client's look has the server import them)
                    #[cfg(feature = "bundles")]
                    if node.waits {
                        state.project.model_assets.set_waits_remotely(copy, true);
                    }
                }
            }
            // instance values, every copy made (one the client makes of the snapshot is paired now: nothing borrowed)
            for (copy, name, target) in refs {
                let value = map(target).map_or(Value::Nil, Value::Instance);
                client.state.borrow_mut().dom.at_mut(copy).store(name, value);
            }
            for (copy, name, target) in attribute_refs {
                let target = map(target);
                set_attribute_ref(&mut client.state.borrow_mut(), copy, &name, target);
            }
            // the own collision groups of the WorldModels in it, before it enters: its parts take their ids from them then
            for (index, snapshot) in groups {
                if let Some(&copy) = made.get(index as usize) {
                    crate::services::physics::apply_model_snapshot(client, copy, snapshot);
                }
            }
            // what the server imported at an instance of the tree after the tree was sealed: in the tree before it enters, as
            // the bundle was in place (lazy.rs bundle_imported)
            let deferred: Vec<(InstanceId, InstanceId, Vec<InstanceId>, Vec<InstanceId>)> = {
                let mut deferred = bridge.deferred.borrow_mut();
                if deferred.is_empty() {
                    Vec::new()
                } else {
                    nodes.iter().zip(&made).filter_map(|(node, &copy)| deferred.remove(&node.id).map(|(imported, removed)| (node.id, copy, imported, removed))).collect()
                }
            };
            if !deferred.is_empty()
                && let Some((_, server)) = client.peer_server()
            {
                for (original, copy, imported, removed) in deferred {
                    copy_import(&server, client, bridge, original, copy, &imported, &removed);
                }
            }
            // the tree enters at once: its DescendantAdded, ChildAdded, the tagged instances' added signals
            if let Some(&root) = made.first() {
                let _ = crate::lua::instance::set_parent(client, root, Some(parent));
            }
        }
        Msg::Parent { id, parent } => {
            let Some(copy) = live(id) else { return Ok(()) };
            let parent = parent.and_then(live);
            // (a copy the server took out of the containers stays the server's instance: kept while it is out)
            {
                let mut state = client.state.borrow_mut();
                if parent.is_none() {
                    state.dom.pin(copy);
                } else if state.dom.is_pinned(copy) {
                    state.dom.unpin(copy);
                }
            }
            let _ = crate::lua::instance::set_parent(client, copy, parent);
        }
        Msg::Prop { id, name, value, quiet } => {
            let Some(copy) = live(id) else { return Ok(()) };
            let value = mapped(value);
            let hooks = client.hooks.borrow();
            let mut state = client.state.borrow_mut();
            if quiet {
                core::raw_set_unsignalled(&mut state, &client.reflection, &hooks, copy, name, value);
            } else {
                core::raw_set(&mut state, &client.reflection, &hooks, copy, name, value);
            }
        }
        Msg::Attr { id, name, value } => {
            let Some(copy) = live(id) else { return Ok(()) };
            let value = match value {
                Value::Instance(target) => attribute_ref(map(target)),
                other => other,
            };
            let mut state = client.state.borrow_mut();
            core::set_attribute(&mut state, &client.reflection, copy, name, value);
        }
        Msg::Tag { id, tag, added } => {
            let Some(copy) = live(id) else { return Ok(()) };
            let mut state = client.state.borrow_mut();
            if added {
                core::add_tag(&mut state, copy, tag);
            } else {
                core::remove_tag(&mut state, copy, &tag);
            }
        }
        Msg::Destroy { id } => {
            let Some(copy) = live(id) else { return Ok(()) };
            if client.state.borrow().dom.is_pinned(copy) {
                client.state.borrow_mut().dom.unpin(copy);
            }
            let _ = crate::lua::instance::destroy(lua, client, copy);
        }
        Msg::CollisionGroups(snapshot) => {
            crate::services::physics::apply_snapshot(client, snapshot);
        }
        Msg::WorldGroups { id, snapshot } => {
            let Some(copy) = live(id) else { return Ok(()) };
            crate::services::physics::apply_model_snapshot(client, copy, snapshot);
        }
        Msg::Event { .. } | Msg::Invoke { .. } | Msg::Return { .. } => {}
    }
    Ok(())
}
