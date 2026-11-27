//! Per-class behaviours (port of `runtime/engine/services.luau`): methods, getters and setters
//! that override or implement reflected members, looked up through the superclass chain as
//! instance.luau's FindBehaviorMember does.
//!
//! Rust: DataModel (GetService & co.), Workspace, Camera, Players, RunService (frame events,
//! BindToRenderStep), CollectionService, GuiService, UserInputService (the static answers; runtime/
//! input's live state is phase 5), StarterGui, ContextActionService, Debris, Sound, Bindable* /
//! Remote*, HttpService (GUIDs, URL / JSON encoding, disabled requests); TweenService (tween.rs),
//! QueryDescendants (query.rs), PhysicsService's collision groups (physics.rs), LocalizationTable and Translator (localization.rs); derived property views are engine::derived. GUI behaviours that
//! need layout or text measurement (Absolute*, TextBounds, TextService:GetTextSize, ContentText)
//! and live input are a host layer's (crate::Layer; roblox-ui-renderer installs them). The script-data stubs (Player methods, 3D model helpers,
//! humanoid stand-ins (AnimationTracks: crate::animation), EditableImage, ContentProvider, marketplace / policy /
//! social mocks) are Luau (lua/services.luau). The UI Labs package is the Luau
//! runtime's own file (runtime/engine/packages/ui-labs.luau, mounted by project.rs).

pub mod encoding;
pub mod input_actions;
pub mod json;
pub mod localization;
pub mod physics;
pub(crate) mod players;
pub mod query;
pub mod reflection_service;
pub mod settings;
pub mod teams;
pub mod tween;

use std::{cell::RefCell, rc::Rc};

use mlua::prelude::*;
use rbx_dom_weak::types as dom;
use rustc_hash::FxHashMap;

use super::{
    ClassId, Engine, InstanceId, K, NameId, Reflection, State,
    dom::flags,
    instance as core,
    lua::{
        Args, Flow, Ret, convert, enum_item_value, instance::fire_immediate_costing, instance::set_property,
        instance_value, none, raise, signal_value, values,
    },
    scheduler::{Phase, TimerKind},
    signal::SignalId,
    value::{Str, Value},
};

/// What Roblox's server puts in a Player when it joins, in its order (a 0.747 server's player before its first spawn:
/// StarterGear, Backpack, PlayerGui; a client's own player also has PlayerScripts)
const JOINED_PLAYER_CONTAINERS: [&str; 3] = ["StarterGear", "Backpack", "PlayerGui"];

pub type MethodFn = Rc<dyn Fn(&Lua, &Rc<Engine>, InstanceId, &Args) -> Ret>;
pub type GetterFn = Rc<dyn Fn(&Lua, &Rc<Engine>, InstanceId) -> Result<LuaValue, Flow>>;
pub type SetterFn = Rc<dyn Fn(&Lua, &Rc<Engine>, InstanceId, LuaValue) -> Result<(), Flow>>;

/// A behaviour method: Rust (through the call protocol) or a Luau function returned as is (the
/// Luau service stubs: their errors and yields are the script's own).
#[derive(Clone)]
pub enum Method {
    Rust(MethodFn),
    Lua(LuaFunction),
}

#[derive(Default)]
pub struct Behaviors {
    methods: FxHashMap<(ClassId, NameId), Method>,
    getters: FxHashMap<(ClassId, NameId), GetterFn>,
    setters: FxHashMap<(ClassId, NameId), SetterFn>,
}

fn find<'a, T>(
    map: &'a FxHashMap<(ClassId, NameId), T>,
    reflection: &Reflection,
    class: ClassId,
    name: NameId,
) -> Option<(ClassId, &'a T)> {
    let mut current = Some(class);
    while let Some(id) = current {
        if let Some(found) = map.get(&(id, name)) {
            return Some((id, found));
        }
        current = reflection.class(id).superclass;
    }
    None
}

impl Behaviors {
    #[inline(never)]
    pub fn add_method(&mut self, class: ClassId, name: NameId, method: MethodFn) {
        self.methods.insert((class, name), Method::Rust(method));
    }

    #[inline(never)]
    pub fn add_lua_method(&mut self, class: ClassId, name: NameId, function: LuaFunction) {
        self.methods.insert((class, name), Method::Lua(function));
    }

    #[inline(never)]
    pub fn add_getter(&mut self, class: ClassId, name: NameId, getter: GetterFn) {
        self.getters.insert((class, name), getter);
    }

    #[inline(never)]
    pub fn add_setter(&mut self, class: ClassId, name: NameId, setter: SetterFn) {
        self.setters.insert((class, name), setter);
    }

    pub fn find_method(&self, reflection: &Reflection, class: ClassId, name: NameId) -> Option<(ClassId, &Method)> {
        find(&self.methods, reflection, class, name)
    }

    /// [`Self::find_method`], and for a deprecated lower-case method with no behaviour of its own the behaviour of the method
    /// it stands for (Reflection::method_alias: `getService` does GetService's work, through a function of its own, and the
    /// Workspace's `breakJoints` Model's BreakJoints work, not the Workspace's own BreakJoints').
    pub fn find_method_or_alias(&self, reflection: &Reflection, class: ClassId, name: NameId) -> Option<(ClassId, &Method)> {
        find(&self.methods, reflection, class, name)
            .or_else(|| reflection.method_alias(class, name).and_then(|(twin, from)| find(&self.methods, reflection, from, twin)))
    }

    pub fn find_getter(&self, reflection: &Reflection, class: ClassId, name: NameId) -> Option<&GetterFn> {
        find(&self.getters, reflection, class, name).map(|(_, f)| f)
    }

    pub fn find_setter(&self, reflection: &Reflection, class: ClassId, name: NameId) -> Option<&SetterFn> {
        find(&self.setters, reflection, class, name).map(|(_, f)| f)
    }

    /// Every (class, name) a method behaviour is registered for (tests: the reflection's list of behaviour-only members).
    #[cfg(test)]
    pub(crate) fn method_keys(&self) -> Vec<(ClassId, NameId)> {
        self.methods.keys().copied().collect()
    }

    /// Every (class, name) a getter behaviour is registered for.
    #[cfg(test)]
    pub(crate) fn getter_keys(&self) -> Vec<(ClassId, NameId)> {
        self.getters.keys().copied().collect()
    }
}

/// CollectionService's per-tag signals.
///
/// As in Studio (0.740, measured with a tagged tree moved in, around and out of the DataModel): a
/// tag counts only while its instance is in the DataModel. GetInstanceAddedSignal fires when a tag
/// is added to an instance there and when a tagged instance enters it (for every tagged instance of
/// the entering subtree, in GetDescendants' order, an instance's tags in the order they were
/// added); GetInstanceRemovedSignal when the tag is removed there or the instance leaves (Parent =
/// nil, Destroy, ClearAllChildren). Moves inside the DataModel and changes outside it fire nothing,
/// and Destroy keeps the instance's tags.
#[derive(Default)]
pub struct CollectionState {
    pub game: Option<InstanceId>,
    tags: FxHashMap<Str, (SignalId, SignalId)>,
    /// Whether any instance was ever given a tag (parent changes skip the tag walk until then)
    pub tagged: bool,
    /// Every instance ever given each tag ([`note_tag`]; freed and untagged ones are skipped when
    /// read): GetTagged looks at these instead of the whole DataModel
    holders: FxHashMap<Str, Vec<InstanceId>>,
    /// Players leaving: what the leave reads, and the departed characters and Players waiting to be destroyed (players.rs)
    pub(crate) departures: players::Departures,
    /// CollectionService's class and its TagAdded and TagRemoved events ([`tag_event`])
    service_events: Option<(ClassId, NameId, NameId)>,
}

impl CollectionState {
    /// Whether a script asked for any tag's signals (GetInstanceAddedSignal, GetInstanceRemovedSignal)
    #[inline]
    pub fn tagged_signals(&self) -> bool {
        !self.tags.is_empty()
    }
}

/// Records that `id` was given `tag` (whatever gave it: AddTag, Clone, a model import).
pub fn note_tag(state: &mut State, id: InstanceId, tag: &Str) {
    state.collection.tagged = true;
    match state.collection.holders.get_mut(tag) {
        Some(holders) => holders.push(id),
        None => {
            state.collection.holders.insert(tag.clone(), vec![id]);
        }
    }
}

/// The descendants of `root` with `tag`, in GetDescendants' order: those of the tag's holders
/// that are still alive, tagged and below `root`, ordered by their place in the tree.
pub fn tagged_below(state: &mut State, root: InstanceId, tag: &Str) -> Vec<InstanceId> {
    let Some(holders) = state.collection.holders.get_mut(tag) else { return Vec::new() };
    let dom = &state.dom;
    holders.retain(|&id| dom.get(id).is_some_and(|instance| instance.tags.contains(tag)));
    let mut seen = rustc_hash::FxHashSet::default();
    let mut found: Vec<(Vec<u32>, InstanceId)> = holders
        .iter()
        .filter(|&&id| seen.insert(id))
        .filter_map(|&id| {
            // (the child's place at each level from `root` down, which grows along its parent's children: preorder is their order)
            let mut path = Vec::new();
            let mut node = id;
            while node != root {
                let parent = dom.at(node).parent?;
                path.push(dom.sibling_place(node));
                node = parent;
            }
            path.reverse();
            (!path.is_empty()).then_some((path, id))
        })
        .collect();
    crate::sort::ord(&mut found);
    found.into_iter().map(|(_, id)| id).collect()
}

/// The tags whose CollectionService signal for instances entering the DataModel (`entering`: GetInstanceAddedSignal) or leaving
/// it (GetInstanceRemovedSignal) has a connection
pub fn listened_tags(state: &State, entering: bool) -> Vec<Str> {
    state
        .collection
        .tags
        .iter()
        .filter(|&(_, &(added, removed))| state.signals.has_handlers(if entering { added } else { removed }))
        .map(|(tag, _)| tag.clone())
        .collect()
}

/// Whether `id` is the DataModel or inside it.
pub fn in_data_model(state: &State, id: InstanceId) -> bool {
    state.collection.game.is_some_and(|game| id == game || state.dom.is_descendant_of(id, game))
}

/// A subtree entered (`entered`) or is about to leave the DataModel: its tagged instances fire
/// their tags' added / removed signals, and CollectionService's TagAdded (TagRemoved) each tag no instance outside it in the
/// DataModel holds ([`tag_event`]), the first before the instances' signals, the second after.
pub fn subtree_moved(state: &mut State, root: InstanceId, entered: bool) {
    let event = tag_event(state, entered);
    if state.collection.tags.is_empty() && event.is_none() {
        return;
    }
    let mut nodes = vec![root];
    nodes.extend(state.dom.descendants(root));
    let mut first: Vec<Str> = Vec::new();
    if let Some(signal) = event {
        for &node in &nodes {
            for tag in &state.dom.at(node).tags {
                if !first.contains(tag) && !held_elsewhere(state, tag, |holder| holder != root && !state.dom.is_descendant_of(holder, root)) {
                    first.push(tag.clone());
                }
            }
        }
        if entered {
            for tag in &first {
                core::fire(state, signal, vec![Value::String(tag.clone())]);
            }
        }
    }
    for node in nodes {
        if state.dom.at(node).tags.is_empty() {
            continue;
        }
        for tag in state.dom.at(node).tags.clone() {
            if let Some(&(added, removed)) = state.collection.tags.get(&tag) {
                core::fire(state, if entered { added } else { removed }, vec![Value::Instance(node)]);
            }
        }
    }
    if let (Some(signal), false) = (event, entered) {
        for tag in first {
            core::fire(state, signal, vec![Value::String(tag)]);
        }
    }
}

/// CollectionService's TagAdded (`added`) or TagRemoved signal, when it has handlers. A client and a server fire TagAdded when
/// the first instance in the DataModel takes a tag (an instance given it there, or a tagged one entering it; not a loose one)
/// and TagRemoved when the last one loses it (removed, or the instance leaving or destroyed), with the tag (measured, 0.747:
/// tools/parity/probes/events_fired.luau).
fn tag_event(state: &State, added: bool) -> Option<SignalId> {
    let (class, tag_added, tag_removed) = state.collection.service_events?;
    let service = state.dom.find_child_of_class(state.collection.game?, class)?;
    let signal = state.dom.at(service).signals.as_ref()?.event(if added { tag_added } else { tag_removed })?;
    state.signals.has_handlers(signal).then_some(signal)
}

/// Whether an instance in the DataModel that `counts` holds `tag`
fn held_elsewhere(state: &State, tag: &Str, counts: impl Fn(InstanceId) -> bool) -> bool {
    state.collection.holders.get(tag).is_some_and(|holders| {
        holders.iter().any(|&holder| {
            counts(holder) && state.dom.get(holder).is_some_and(|instance| instance.tags.contains(tag)) && in_data_model(state, holder)
        })
    })
}

fn tag_signals(state: &mut State, tag: &Str) -> (SignalId, SignalId) {
    if let Some(&signals) = state.collection.tags.get(tag) {
        return signals;
    }
    // (named after the tag: tostring gives "Signal xInstanceAdded" for the tag "x", measured, 0.747, a playing client and a server,
    // tools/parity/probes/service_answers.luau)
    let added = state.signals.create(Str::new(&[tag.as_bytes(), b"InstanceAdded"].concat()));
    let removed = state.signals.create(Str::new(&[tag.as_bytes(), b"InstanceRemoved"].concat()));
    state.collection.tags.insert(tag.clone(), (added, removed));
    (added, removed)
}

/// Hooks.TagAdded: GetInstanceAddedSignal fires for instances in the DataModel (after TagAdded, [`tag_event`]).
pub fn tag_added(state: &mut State, id: InstanceId, tag: &Str) {
    note_tag(state, id, tag);
    if !in_data_model(state, id) {
        return;
    }
    if let Some(signal) = tag_event(state, true)
        && !held_elsewhere(state, tag, |holder| holder != id)
    {
        core::fire(state, signal, vec![Value::String(tag.clone())]);
    }
    if let Some(&(added, _)) = state.collection.tags.get(tag) {
        core::fire(state, added, vec![Value::Instance(id)]);
    }
}

/// Hooks.TagRemoved: GetInstanceRemovedSignal fires for instances in the DataModel (then TagRemoved, [`tag_event`]).
pub fn tag_removed(state: &mut State, id: InstanceId, tag: &Str) {
    if !in_data_model(state, id) {
        return;
    }
    if let Some(&(_, removed)) = state.collection.tags.get(tag) {
        core::fire(state, removed, vec![Value::Instance(id)]);
    }
    if let Some(signal) = tag_event(state, false)
        && !held_elsewhere(state, tag, |_| true)
    {
        core::fire(state, signal, vec![Value::String(tag.clone())]);
    }
}

fn class(reflection: &Reflection, name: &str) -> Option<ClassId> {
    reflection.class_id(name)
}

fn name(reflection: &Reflection, name: &str) -> Option<NameId> {
    reflection.name_id(name)
}

/// Players:GetPlayers: the local player, then every other Player a harness parented to Players (ree has one
/// joined player of its own; a lobby harness adds the others), in child order.
fn joined_players(engine: &Rc<Engine>, players: InstanceId) -> Vec<InstanceId> {
    let local = engine.world().local_player;
    let state = engine.state.borrow();
    // (a loopback client's own player its server took out is no longer one: measured, a client counts the others only)
    let listed = local.filter(|&local| state.dom.get(local).is_some_and(|player| player.parent == Some(players)));
    let others = state
        .dom
        .children_of(players)
        .iter()
        .copied()
        .filter(|&child| Some(child) != local && engine.reflection.is_a(state.dom.at(child).class, "Player"));
    listed.into_iter().chain(others).collect()
}

/// A behaviour method from a closure (a layer's behaviours: [`Behaviors::add_method`]).
pub fn method(f: impl Fn(&Lua, &Rc<Engine>, InstanceId, &Args) -> Ret + 'static) -> MethodFn {
    Rc::new(f)
}

/// A method's string parameter ([`strings`]): its place among the call's arguments (`index`, 1 for the first after self), the
/// number its warning names (`argument`), the number "Argument n missing or nil" names when it is nil (`missing`; None leaves nil
/// to the method), and whether a client warns before it casts a value (a method that yields casts without the warning).
#[derive(Clone, Copy, Debug)]
pub struct StringParam {
    pub index: usize,
    pub argument: usize,
    pub missing: Option<usize>,
    pub warn: bool,
}

impl StringParam {
    /// The parameter at `index`, named by its place
    pub const fn at(index: usize) -> Self {
        Self { index, argument: index, missing: Some(index), warn: true }
    }

    /// [`Self::at`] of a method that yields: cast without the warning
    pub const fn quiet(index: usize) -> Self {
        Self { warn: false, ..Self::at(index) }
    }
}

/// The call's arguments with the string parameters `params` read as a client reads them (lua/convert.rs string_param), the last
/// first, as a client casts a call's arguments (measured: CollisionGroupsAreCollidable(true, {}) warns for the second, then the
/// first): None when every one is a string already, else the arguments with each other value replaced by its text; nil raises
/// "Argument n missing or nil".
pub fn cast_strings(lua: &Lua, engine: &Engine, args: &Args, params: &[StringParam]) -> Result<Option<Args>, Flow> {
    let mut cast: Option<Args> = None;
    for param in params.iter().rev() {
        let value = args.get(param.index);
        match value {
            LuaValue::String(_) => continue,
            LuaValue::Nil => match param.missing {
                Some(number) => return raise(format!("Argument {number} missing or nil")),
                None => continue,
            },
            _ => {
                let text = lua.create_string(crate::lua::convert::string_param(lua, engine, value, param.argument, param.warn)?)?;
                let cast = cast.get_or_insert_with(|| Args(args.0.clone()));
                if let Some(slot) = cast.0.get_mut(param.index) {
                    *slot = LuaValue::String(text);
                }
            }
        }
    }
    Ok(cast)
}

/// `method` given its string parameters `params` as a client reads them ([`cast_strings`]): a string the method gets as it is,
/// so its answers for strings and the work clock's prices stay those of the method alone.
pub fn strings(params: &'static [StringParam], method: MethodFn) -> MethodFn {
    if params.is_empty() {
        return method;
    }
    Rc::new(move |lua, engine, id, args| match cast_strings(lua, engine, args, params)? {
        Some(cast) => method(lua, engine, id, &cast),
        None => method(lua, engine, id, args),
    })
}

/// No string parameter ([`strings`] passes the call on as it is)
const NONE: &[StringParam] = &[];
/// The first argument a string parameter
pub(crate) const FIRST: &[StringParam] = &[StringParam::at(1)];
/// The first two arguments string parameters
const FIRST_TWO: &[StringParam] = &[StringParam::at(1), StringParam::at(2)];
/// [`FIRST`] of a method that yields: cast without the warning
const QUIET_FIRST: &[StringParam] = &[StringParam::quiet(1)];
/// CollectionService:AddTag, RemoveTag and HasTag's tag: the second argument, named so in the warning, and the third in "Argument 3
/// missing or nil" (measured, Studio 0.747)
const TAG: &[StringParam] = &[StringParam { index: 2, argument: 2, missing: Some(3), warn: true }];

/// A behaviour getter from a closure ([`Behaviors::add_getter`]).
pub fn getter(f: impl Fn(&Lua, &Rc<Engine>, InstanceId) -> Result<LuaValue, Flow> + 'static) -> GetterFn {
    Rc::new(f)
}

fn constant(value: impl Fn(&Lua) -> LuaResult<LuaValue> + 'static) -> MethodFn {
    Rc::new(move |lua, _, _, _| Ok(values([value(lua)?])))
}

/// A method that does nothing here where a client does something a script can see: each call reports `class`.`member` as not
/// emulated (Output `missing`, Engine::missing) and returns nothing, so the script runs on.
fn unemulated(class: &'static str, member: &'static str) -> MethodFn {
    Rc::new(move |lua, engine, _, _| {
        engine.missing(Some(lua), class, member);
        Ok(none())
    })
}

fn string_value(lua: &Lua, text: &str) -> LuaResult<LuaValue> {
    Ok(LuaValue::String(lua.create_string(text)?))
}

fn enum_item(lua: &Lua, engine: &Engine, enum_name: &str, item: &str) -> LuaResult<LuaValue> {
    match engine.reflection.enum_item(enum_name, item) {
        Some(item) => enum_item_value(lua, engine, item),
        None => Ok(LuaValue::Nil),
    }
}

fn vector2(lua: &Lua, x: f64, y: f64) -> LuaResult<LuaValue> {
    let vector = lune_roblox::datatypes::types::Vector2::from(dom::Vector2::new(x as f32, y as f32));
    Ok(LuaValue::UserData(lune_roblox::datatypes::create::vector2(lua, vector)?))
}

/// Services Roblox keeps for its own scripts: a script's `game:GetService` answers nil for them, with no error (Studio 0.747,
/// a play session's client and server at script identity 2 and a client at identity 4, the command bar's:
/// tools/parity/probes/core_services.luau). At identity 2 CoreGui, StudioService and ScriptEditorService are nil as well, and a
/// server refuses NetworkClient; ree answers those as identity 4 does, since edit-mode code that Studio runs as a plugin
/// parents its previews to CoreGui.
pub const ROBLOX_ONLY_SERVICES: &[&str] = &[
    "AppStorageService",
    "CSGDictionaryService",
    "CorePackages",
    "CoreScriptSyncService",
    "PluginGuiService",
    "RobloxPluginGuiService",
    "RobloxReplicatedStorage",
];

/// GetService: the existing service child, or a new one (named, parented without events and
/// parent-locked).
pub fn get_service(engine: &Engine, game: InstanceId, service_name: &str) -> Result<InstanceId, Flow> {
    let reflection = &engine.reflection;
    let mut state = engine.state.borrow_mut();
    let found = state
        .dom
        .at(game)
        .children
        .iter()
        .copied()
        .find(|&child| &*reflection.class(state.dom.at(child).class).name == service_name);
    if let Some(found) = found {
        return Ok(found);
    }
    let Some(class) = reflection.class_id(service_name).filter(|&c| reflection.class(c).service) else {
        return raise(format!("'{service_name}' is not a valid Service name"));
    };
    // Roblox's names that are not the class's: scripts index `game["Run Service"]` and
    // `UserSettings().GameSettings` (Studio 0.748: Run Service, Script Context and Teleport Service are the DataModel's
    // services there from the start named otherwise; a playing client and a server name the PathfindingService GetService makes
    // "Instance", measured, 0.747, tools/parity/probes/events_fired.luau)
    let instance_name = match service_name {
        "RunService" => "Run Service",
        "ScriptContext" => "Script Context",
        "TeleportService" => "Teleport Service",
        "UserGameSettings" => "GameSettings",
        "PathfindingService" => "Instance",
        other => other,
    };
    let service = core::create_named(&mut state, reflection, class, instance_name);
    for &(class_name, property, value) in SERVICE_VALUES {
        if class_name == service_name {
            if let Some(name) = reflection.name_id(property) {
                core::raw_set_silent(&mut state, reflection, service, name, Value::Number(value));
            }
        }
    }
    state.dom.attach(service, game);
    state.dom.at_mut(service).set_flag(flags::PARENT_LOCKED, true);
    // (the DataModel's ServiceAdded for a service a client or a server makes when a script first gets it, not for those it holds
    // from the start: STARTING_SERVICES)
    if Some(state.dom.at(game).class) == reflection.class_id("DataModel")
        && !STARTING_SERVICES.contains(&service_name)
        && !(engine.options.server() && SERVER_STARTING_SERVICES.contains(&service_name))
        && let Some(added) = reflection.name_id("ServiceAdded")
    {
        core::fire_event(&mut state, game, added, vec![Value::Instance(service)]);
    }
    Ok(service)
}

/// The services a playing client holds once a game's scripts run (FindService finds them without a GetService first): GetService
/// of any other makes it and fires the DataModel's ServiceAdded (measured, 0.747, a playing client and a server: every service
/// class looked up with FindService, tools/parity/probes/service_names.luau's sweep; ServiceAdded for PathfindingService and
/// HapticService, none for ProximityPromptService, Debris, TweenService, PhysicsService and TextService, events_fired.luau).
const STARTING_SERVICES: &[&str] = &[
    "AdService", "AnalyticsService", "AnimationClipProvider", "AssetService", "AvatarCreationService", "AvatarEditorService", "BadgeService",
    "BrowserService", "CaptureService", "Chat", "CollectionService", "CommerceService", "ContentProvider", "ContextActionService",
    "CookiesService", "DataStoreService", "Debris", "FeatureRestrictionManager", "FriendService", "GamePassService", "GenerationService",
    "GuiService", "GuidRegistryService", "HeapProfilerService", "HttpRbxApiService", "HttpService", "InsertService", "JointsService",
    "KeyboardService", "KeyframeSequenceProvider", "Lighting", "LocalizationService", "LogService", "MarketplaceService", "MaterialService",
    "MemoryStoreService", "MessagingService", "MicroProfilerService", "ModerationService", "MouseService", "NotificationService", "Packages",
    "PermissionsService", "PhysicsService", "Players", "PointsService", "PolicyService", "ProximityPromptService", "RecommendationService",
    "ReplicatedFirst", "ReplicatedStorage", "RunService", "ScriptContext", "ScriptProfilerService", "ScriptService", "Selection",
    "ServerScriptService", "SharedTableRegistry", "SlimContentProvider", "SocialService", "SoundService", "StarterGui", "StarterPack",
    "StarterPlayer", "Stats", "Teams", "TeleportService", "TestService", "TextChatService", "TextService", "TimerService", "TouchInputService",
    "TweenService", "UserInputService", "VRService", "VideoCaptureService", "VideoService", "VoiceChatService", "Workspace",
];
/// [`STARTING_SERVICES`]' server has these too
const SERVER_STARTING_SERVICES: &[&str] = &["ClusterPacketCache", "ServerStorage", "SpawnerService"];

/// What a playing client's services hold from the start where neither the dump nor rbx-dom gives a default (a service
/// has none, and rbx-dom leaves UserGameSettings' numbers out, so they read 0): the mouse and gamepad sensitivities read 1
/// (measured in a 0.747 client: tools/parity/probes/settings.luau; the settings file it read holds 1 for both sensitivities).
const SERVICE_VALUES: &[(&str, &str, f64)] = &[
    ("UserGameSettings", "MouseSensitivity", 1.0),
    ("UserGameSettings", "GamepadCameraSensitivity", 1.0),
    ("UserInputService", "MouseDeltaSensitivity", 1.0),
];

/// A UserSettings object (outside the DataModel, as in Roblox): `UserSettings()` makes it on its first
/// call and returns it from then on. None when the reflection has no UserSettings class.
pub fn user_settings(engine: &Engine) -> Option<InstanceId> {
    let class = engine.reflection.class_id("UserSettings")?;
    let id = {
        let mut state = engine.state.borrow_mut();
        // named as in Roblox (Studio 0.748: UserSettings().Name, tostring and GetFullName are "GlobalBasicSettings")
        let id = core::create_named(&mut state, &engine.reflection, class, "GlobalBasicSettings");
        state.dom.at_mut(id).set_flag(flags::PARENT_LOCKED, true);
        state.dom.pin(id);
        id
    };
    // GameSettings is there from the start: scripts read `UserSettings().GameSettings` (Roblox's own
    // PlayerModule among them) before any GetService
    let _ = get_service(engine, id, "UserGameSettings");
    Some(id)
}

/// GetService's and FindService's name as a client reads it (convert::string_param): a string as it is, any other value but nil
/// its text after the warning. A client then answers nil for a name it knows that is no service's and raises `'<name>' is not a
/// valid Service name` for any other ([`names_no_service`]), whatever value the name came from (measured, Studio 0.747, a playing
/// client and a server: GetService(workspace), GetService(Enum.Material.Plastic), GetService(print) are nil,
/// GetService(Enum.CoreGuiType.Chat) the Chat service, and GetService(true), GetService({}), GetService(Vector3.new()) and
/// GetService(5) raise).
fn service_param(lua: &Lua, engine: &Engine, args: &Args) -> Result<String, Flow> {
    let value = args.get(1);
    match value {
        LuaValue::String(name) => Ok(name.to_string_lossy()),
        LuaValue::Nil => raise("Argument 1 missing or nil"),
        _ => Ok(String::from_utf8_lossy(&convert::string_param(lua, engine, value, 1, true)?).into_owned()),
    }
}

/// Whether `name` is a service's class name.
fn names_service(engine: &Engine, name: &str) -> bool {
    engine.reflection.class_by_name(name).is_some_and(|class| class.service)
}

/// Whether GetService and FindService answer nil for `name`, a name that is no service's: one a client knows as a class's, a
/// member's, an enum's, an enum item's or a type's (Reflection::knows_name: "Part", "Anchored", "Plastic", "Instance", "nil",
/// "function", "EventInstance", "bool"), where any other ("Nope", "true", "5", "Lighting ", "part" in another case, a script's
/// own instance name or tag) raises "'<name>' is not a valid Service name" (measured, Studio 0.747, every class, member, enum
/// and item name of the dump and of rbx-dom's database and 230 others, a playing client and a server alike:
/// tools/parity/probes/service_names.luau).
fn names_no_service(engine: &Engine, name: &str) -> bool {
    !names_service(engine, name) && engine.reflection.knows_name(name)
}

/// GetService of a name that is no service's but a class's whose making a game script is refused: a client tries to make one
/// and raises its refusal (measured, 0.747, a playing client and a server: GetService("Player"), GetService("AuroraScript");
/// every other class of the dump and of rbx-dom's database answers nil).
fn refused_class(name: &str) -> Option<&'static str> {
    match name {
        "Player" => Some("The current thread cannot create 'Player' (lacking capability WritePlayer)"),
        "AuroraScript" => Some("This type can not be instantiated"),
        _ => None,
    }
}

pub fn find_service(engine: &Engine, game: InstanceId, service_name: &str) -> Option<InstanceId> {
    let state = engine.state.borrow();
    state
        .dom
        .at(game)
        .children
        .iter()
        .copied()
        .find(|&child| &*engine.reflection.class(state.dom.at(child).class).name == service_name)
}

struct RenderStep {
    name: String,
    priority: f64,
    callback: LuaFunction,
    sequence: u64,
}

/// The methods only Roblox's own scripts may call (reflection.rs CAPABILITY_CALLS): each is a function that raises the
/// client's error whatever it is called with, in every run context (but for the Plugin ones in edit mode, below). Plain
/// functions, not instance methods, because the error comes before the instance is checked (`game.DefineFastInt("x", 1)`
/// raises it too, not "Expected ':' not '.'").
/// Installed with the core's behaviours, before a host's layers, which may replace them. A plugin has the Plugin capability, so
/// in edit mode, where a plugin's code runs, the methods that lack only that one are left to report themselves not emulated.
pub fn install_capability_calls(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let plugin = engine.options.edit_mode();
    for &(class_name, member, capability) in crate::reflection::CAPABILITY_CALLS {
        let (Some(owner), Some(name)) = (class(&engine.reflection, class_name), engine.reflection.name_id(member)) else { continue };
        // (a plugin has PluginOrOpenCloud as well: Animator:StepAnimations runs in edit mode)
        if plugin && (capability == "Plugin" || capability == "PluginOrOpenCloud") {
            // A subclass method shadowing an emulated superclass method (Selection.Remove over Instance.Remove) would
            // fall through to the superclass's unrelated behaviour if left unregistered; bind its own unemulated
            // stand-in instead (lowercase Selection.remove still reaches Instance.Remove via alias_lookup_class).
            let shadowed = engine.reflection.class(owner).superclass.is_some_and(|superclass| {
                engine.behaviors.borrow().find_method(&engine.reflection, superclass, name).is_some()
            });
            if shadowed && let LuaValue::Function(function) = super::lua::instance::unimplemented_function(lua, engine, owner, name)? {
                engine.behaviors.borrow_mut().add_lua_method(owner, name, function);
            }
            continue;
        }
        let message = format!("The current thread cannot call '{member}' (lacking capability {capability})");
        let function = super::lua::wrapped(lua, engine, member, move |_, _| raise(message.clone()))?;
        engine.behaviors.borrow_mut().add_lua_method(owner, name, function);
    }
    Ok(())
}

/// Installs the service behaviours (Services.Install) and the RunService frame hooks.
#[inline(never)]
pub fn install(engine: &Rc<Engine>) {
    let reflection = engine.reflection.clone();
    let mut behaviors = engine.behaviors.borrow_mut();
    let options = engine.options.clone();

    install_data_model(&mut behaviors, &reflection, &options);
    install_workspace_and_camera(&mut behaviors, &reflection, &options);
    install_players(engine, &mut behaviors, &reflection, &options);
    install_run_service(engine, &mut behaviors, &reflection, &options);

    // TweenService, Instance:QueryDescendants, AnimationTracks
    tween::install(engine, &mut behaviors);
    reflection_service::install(engine, &mut behaviors);
    physics::install(engine, &mut behaviors);
    // Teams: a player's team follows its TeamColor and Neutral; Team:GetPlayers, Teams:GetTeams
    teams::install(engine, &mut behaviors);
    // LocalizationTable, Translator and LocalizationService's translators
    localization::install(engine, &mut behaviors);
    // settings() and UserSettings(): GetService / FindService and the settings services' members
    settings::install(engine, &mut behaviors);
    crate::animation::install(engine, &mut behaviors);
    crate::meshes::install(engine, &mut behaviors);
    input_actions::install(engine, &mut behaviors);
    crate::path2d::install(engine, &mut behaviors);
    query::install(engine, &mut behaviors);
    // (on every side: a playing client answers its members as a server does, measured)
    encoding::install(engine, &mut behaviors);

    install_collection_service(&mut behaviors, &reflection);
    if let (Some(collection), Some(added), Some(removed)) = (class(&reflection, "CollectionService"), name(&reflection, "TagAdded"), name(&reflection, "TagRemoved")) {
        engine.state.borrow_mut().collection.service_events = Some((collection, added, removed));
    }
    install_gui_and_input(&mut behaviors, &reflection, &options);
    install_starter_actions_debris_sound(&mut behaviors, &reflection, &options);
    install_remotes_and_http(&mut behaviors, &reflection);
    install_read_only_answers(&mut behaviors, &reflection, &options);
    drop(behaviors);
    engine.clear_member_cache();
}

#[inline(never)]
fn install_data_model(behaviors: &mut Behaviors, reflection: &Reflection, options: &crate::Options) {
    // DataModel
    if let Some(data_model) = class(reflection, "DataModel") {
        let get = method(|lua, engine, id, args| {
            let service_name = service_param(lua, engine, args)?;
            if names_no_service(engine, &service_name) {
                if let Some(refused) = refused_class(&service_name) {
                    return raise(refused);
                }
                return Ok(values([LuaValue::Nil]));
            }
            if ROBLOX_ONLY_SERVICES.contains(&&*service_name) {
                return Ok(values([LuaValue::Nil]));
            }
            // (the settings services belong to settings() and UserSettings(): settings.rs)
            if let Some(answer) = settings::data_model_service(engine, id, &service_name) {
                answer?;
                return Ok(values([LuaValue::Nil]));
            }
            let service = get_service(engine, id, &service_name)?;
            Ok(values([instance_value(lua, engine, service)?]))
        });
        behaviors.add_method(data_model, K::GetService as NameId, get.clone());
        behaviors.add_method(data_model, K::service as NameId, get);
        behaviors.add_method(data_model, K::FindService as NameId, method(|lua, engine, id, args| {
            let service_name = service_param(lua, engine, args)?;
            if names_no_service(engine, &service_name) {
                return Ok(values([LuaValue::Nil]));
            }
            // (a name no class, member, enum or type has raises as GetService does: tools/parity/probes/service_names.luau)
            if !names_service(engine, &service_name) && !settings::found_nowhere(&service_name) {
                return raise(format!("'{service_name}' is not a valid Service name"));
            }
            Ok(values([match find_service(engine, id, &service_name).filter(|_| !ROBLOX_ONLY_SERVICES.contains(&&*service_name)) {
                Some(service) => instance_value(lua, engine, service)?,
                None => LuaValue::Nil,
            }]))
        }));
        behaviors.add_method(data_model, K::IsLoaded as NameId, constant(|_| Ok(LuaValue::Boolean(true))));
        behaviors.add_getter(data_model, K::Workspace as NameId, getter(|lua, engine, id| {
            let service = get_service(engine, id, "Workspace")?;
            Ok(instance_value(lua, engine, service)?)
        }));
        // (the dump's deprecated `game.lighting` is the Lighting service: measured, tools/parity/probes/aliases.luau;
        // `game.workspace` reads Workspace through Reflection::property_alias)
        if let Some(lighting) = reflection.name_id("lighting") {
            behaviors.add_getter(data_model, lighting, getter(|lua, engine, id| {
                let service = get_service(engine, id, "Lighting")?;
                Ok(instance_value(lua, engine, service)?)
            }));
        }
        // (the dump's `game.RunService` is the RunService, though the service is named "Run Service": measured in Studio
        // 0.748, where game:FindFirstChild("RunService") is nil)
        if let Some(run_service) = reflection.name_id("RunService") {
            behaviors.add_getter(data_model, run_service, getter(|lua, engine, id| {
                let service = get_service(engine, id, "RunService")?;
                Ok(instance_value(lua, engine, service)?)
            }));
        }
        // (its function runs when the server shuts down, which no run reaches: Roblox's DataModel documentation)
        behaviors.add_method(data_model, K::BindToClose as NameId, method(|_, _, _, _| Ok(none())));
        behaviors.add_method(data_model, K::GetJobsInfo as NameId, constant(|lua| Ok(LuaValue::Table(lua.create_table()?))));
        // (the game the client joined: the client profile's, else what a Studio session of an unpublished place reads)
        let game = options.profile.game.clone();
        let place_id = game.place_id();
        behaviors.add_getter(data_model, K::PlaceId as NameId, getter(move |_, _, _| Ok(LuaValue::Number(place_id))));
        let game_id = game.game_id();
        behaviors.add_getter(data_model, K::GameId as NameId, getter(move |_, _, _| Ok(LuaValue::Number(game_id))));
        let job_id: Rc<str> = game.job_id.as_deref().unwrap_or("").into();
        behaviors.add_getter(data_model, K::JobId as NameId, getter(move |lua, _, _| Ok(string_value(lua, &job_id)?)));
        let creator_id = game.creator_id.unwrap_or(0.0);
        behaviors.add_getter(data_model, K::CreatorId as NameId, getter(move |_, _, _| Ok(LuaValue::Number(creator_id))));
        let creator_type: Rc<str> = game.creator_type.as_deref().unwrap_or("Group").into();
        behaviors.add_getter(data_model, K::CreatorType as NameId, getter(move |lua, engine, _| {
            Ok(enum_item(lua, engine, "CreatorType", &creator_type)?)
        }));
        let place_version = game.place_version.unwrap_or(0.0);
        behaviors.add_getter(data_model, K::PlaceVersion as NameId, getter(move |_, _, _| Ok(LuaValue::Number(place_version))));
        let private_server_id: Rc<str> = game.private_server_id.as_deref().unwrap_or("").into();
        behaviors.add_getter(data_model, K::PrivateServerId as NameId, getter(move |lua, _, _| Ok(string_value(lua, &private_server_id)?)));
        let private_server_owner_id = game.private_server_owner_id.unwrap_or(0.0);
        behaviors.add_getter(data_model, K::PrivateServerOwnerId as NameId, getter(move |_, _, _| Ok(LuaValue::Number(private_server_owner_id))));
    }
}

#[inline(never)]
fn install_workspace_and_camera(behaviors: &mut Behaviors, reflection: &Reflection, options: &crate::Options) {
    // Workspace / Camera
    if let Some(workspace) = class(reflection, "Workspace") {
        let edit = options.edit_mode();
        // DistributedGameTime: 0 in edit mode (the game never started running), like time(). In a running game, a playing client's
        // and a server's alike, the game time as of the frame's simulation step (Studio 0.747,
        // tools/parity/probes/distributed_game_time.luau): time() moves at the frame's start, DistributedGameTime after
        // PreSimulation (PreAnimation, Stepped and PreSimulation read the last frame's, PostSimulation, the task.wait resumptions
        // and Heartbeat the frame's, equal to time() to the bit). A client's is its own clock, not its server's
        // (probes/loopback/distributed_game_time.luau). A script may write it: the value holds until the next step sets the game
        // time again (scheduler.rs Scheduler::game_time; the step: install_run_service)
        behaviors.add_getter(workspace, K::DistributedGameTime as NameId, getter(move |_, engine, _| {
            let state = engine.state.borrow();
            Ok(LuaValue::Number(if edit { 0.0 } else { state.sched.game_time.unwrap_or_else(|| state.sched.time()) }))
        }));
        if !edit {
            // (a write takes any value a number property takes, "x" as 0, and fires Changed when it changes the value; edit mode
            // keeps the plain write it had, which the getter's 0 hides: not measured there)
            behaviors.add_setter(workspace, K::DistributedGameTime as NameId, Rc::new(|lua, engine, id, value| {
                let class = engine.state.borrow().dom.at(id).class;
                let Some(info) = engine.reflection.class_property(class, K::DistributedGameTime as NameId).cloned() else { return Ok(()) };
                let Value::Number(time) = convert::coerce(lua, engine, &info, &value)? else { return Ok(()) };
                set_game_time(engine, id, time, true);
                Ok(())
            }));
        }
        behaviors.add_method(workspace, K::GetServerTimeNow as NameId, method(|_, engine, _, _| {
            let time = engine.state.borrow_mut().sched.unix_time();
            Ok(values([LuaValue::Number(time)]))
        }));
        behaviors.add_method(workspace, K::GetRealPhysicsFPS as NameId, constant(|_| Ok(LuaValue::Number(60.0))));
        // (a playing client and its server read true and 100, the share of real time physics runs at, whatever the arguments
        // (Studio 0.748, tools/parity/probes/workspace_physics_state.luau), and so does plugin code in Studio's edit DataModel
        // (identity 5, the same calls run there by hand, not a capture). ree's world is never throttled, as its physics rate is
        // always 60)
        if let Some(n) = name(reflection, "PGSIsEnabled") {
            behaviors.add_method(workspace, n, constant(|_| Ok(LuaValue::Boolean(true))));
        }
        if let Some(n) = name(reflection, "GetPhysicsThrottling") {
            behaviors.add_method(workspace, n, constant(|_| Ok(LuaValue::Number(100.0))));
        }
        // (nil after a nil write until the simulation step gives it a camera: world/camera.rs, which also takes the writes)
        behaviors.add_getter(workspace, K::CurrentCamera as NameId, getter(|lua, engine, _| match crate::world::camera::current(engine) {
            Some(camera) => Ok(instance_value(lua, engine, camera)?),
            None => Ok(LuaValue::Nil),
        }));
    }
    if let Some(camera) = class(reflection, "Camera") {
        // (the run's screen for the camera it starts with, 1 x 1 for others: world/camera.rs)
        behaviors.add_getter(camera, K::ViewportSize as NameId, getter(|lua, engine, id| {
            let (width, height) = crate::world::camera::viewport_size(engine, id);
            Ok(vector2(lua, f64::from(width), f64::from(height))?)
        }));
    }
}

/// Workspace.DistributedGameTime from now on: a script's write, or the game time a simulation step gives it. Changed and the
/// property's own signal fire when the value changes and `signal` asks for them (a script's write, a server's step).
fn set_game_time(engine: &Engine, workspace: InstanceId, time: f64, signal: bool) {
    let mut state = engine.state.borrow_mut();
    let old = state.sched.game_time.unwrap_or_else(|| state.sched.time());
    state.sched.game_time = Some(time);
    if signal && old.to_bits() != time.to_bits() {
        core::fire_computed_changed(&mut state, &engine.reflection, workspace, K::DistributedGameTime as NameId);
    }
}

#[inline(never)]
fn install_players(engine: &Rc<Engine>, behaviors: &mut Behaviors, reflection: &Reflection, options: &crate::Options) {
    // Players / Player
    if let Some(players) = class(reflection, "Players") {
        let local_player = getter(|lua, engine, _| match engine.world().local_player {
            Some(player) => Ok(instance_value(lua, engine, player)?),
            None => Ok(LuaValue::Nil),
        });
        behaviors.add_getter(players, K::LocalPlayer as NameId, local_player.clone());
        behaviors.add_getter(players, K::localPlayer as NameId, local_player);
        // The local player, then the Player instances a harness parented to Players (other players of a lobby)
        behaviors.add_method(players, K::GetPlayers as NameId, method(|lua, engine, this, _| {
            let table = lua.create_table()?;
            for player in joined_players(engine, this) {
                table.raw_push(instance_value(lua, engine, player)?)?;
            }
            Ok(values([LuaValue::Table(table)]))
        }));
        // (the id is cast as an int64 argument is, the type named "User": "5" is 5, true 1, "x" raises "Unable to cast string to
        // User", a table "Unable to cast Array to User", an enum item "Unable to cast Material to User", nil "Argument 1 missing or
        // nil"; measured, 0.747, a playing client and a server: tools/parity/probes/service_answers.luau)
        behaviors.add_method(players, K::GetPlayerByUserId as NameId, method(|lua, engine, this, args| {
            let wanted = match convert::int64_argument(engine, args.get(1), 1) {
                Ok(id) => Some(id as f64),
                Err(message) => return raise(match message.strip_suffix(" to int64") {
                    Some(cast) => format!("{cast} to User"),
                    None => message,
                }),
            };
            for player in joined_players(engine, this) {
                let user_id = engine.state.borrow().dom.number(player, K::UserId as NameId);
                if user_id.is_some() && user_id == wanted {
                    return Ok(values([instance_value(lua, engine, player)?]));
                }
            }
            Ok(values([LuaValue::Nil]))
        }));
        if let Some(n) = name(reflection, "GetPlayerFromCharacter") {
            let character = name(reflection, "Character");
            behaviors.add_method(players, n, method(move |lua, engine, this, args| {
                let (Some(character), Some(model)) = (character, args.instance(1)) else { return Ok(values([LuaValue::Nil])) };
                for player in joined_players(engine, this) {
                    let held = engine.state.borrow().dom.at(player).stored(character).cloned();
                    if matches!(held, Some(Value::Instance(held)) if held == model) {
                        return Ok(values([instance_value(lua, engine, player)?]));
                    }
                }
                Ok(values([LuaValue::Nil]))
            }));
        }
        // (the client profile's player and friends by name; any other name reads 1, a stand-in: an account's id is the web's)
        if let Some(n) = name(reflection, "GetUserIdFromNameAsync") {
            behaviors.add_method(players, n, strings(QUIET_FIRST, method(|_, engine, _, args| {
                let profile = engine.profile();
                let wanted = args.string(1).map(|name| name.to_string_lossy());
                let id = wanted.and_then(|wanted| {
                    let identity = &profile.identity;
                    if identity.name() == wanted {
                        return Some(identity.user_id());
                    }
                    let friends = profile.social.as_ref()?.get("friends")?.as_array()?;
                    friends.iter().find(|friend| friend["name"].as_str() == Some(wanted.as_str()))?["userId"].as_f64()
                });
                Ok(values([LuaValue::Number(id.unwrap_or(1.0))]))
            })));
        }
        // Players a harness adds after the run started: PlayerAdded when a Player is parented to Players (the local player
        // joined before the run). On a server a joining player gets the containers Roblox's server gives it (StarterGear,
        // Backpack, PlayerGui, in that order) before PlayerAdded, unless the harness made them, and its team (teams.rs). A
        // player leaving (PlayerRemoving, its character's removal) is instance.rs reparent's, with players.rs; a Character
        // change (CharacterRemoving, CharacterAdded, a server's removal of the old character) is players.rs set_character's.
        if let (Some(_), Some(added)) = (class(reflection, "Player"), name(reflection, "PlayerAdded")) {
            let weak = Rc::downgrade(engine);
            let reflection_for_enter = engine.reflection.clone();
            let server = options.server();
            players::install(&mut engine.state.borrow_mut(), reflection, server);
            let mut hooks = engine.hooks.borrow_mut();
            // Joining player setup (containers, team auto-assignment + Team.PlayerAdded, Players.PlayerAdded) runs before
            // Players.ChildAdded (measured in Studio 0.747, tools/parity/probes/loopback/team_join.luau).
            hooks.before_child_added.push(Box::new(move |state, hooks, id, old, new| {
                let Some(engine) = weak.upgrade() else { return };
                if state.dom.at(new).class != players
                    || !reflection_for_enter.is_a(state.dom.at(id).class, "Player")
                    || engine.world().local_player == Some(id)
                {
                    return;
                }
                let was_in_players = old.is_some_and(|parent| state.dom.at(parent).class == players);
                if server && !was_in_players {
                    for name in JOINED_PLAYER_CONTAINERS {
                        if state.dom.find_child(id, name.as_bytes()).is_some() {
                            continue;
                        }
                        if let Some(class) = reflection_for_enter.class_id(name) {
                            let child = core::create_named(state, &reflection_for_enter, class, name);
                            state.dom.attach(child, id);
                        }
                    }
                    // (its team before PlayerAdded: the team's PlayerAdded fires first, teams.rs)
                    teams::assign_joining(state, &reflection_for_enter, hooks, engine.world().game, id);
                }
                core::fire_event(state, new, added, vec![Value::Instance(id)]);
            }));
        }
    }
    // Player methods and the other script-data stubs are Luau (lua/services.luau)
}

#[inline(never)]
fn install_run_service(engine: &Rc<Engine>, behaviors: &mut Behaviors, reflection: &Reflection, options: &crate::Options) {
    // RunService: frame events (measured in Studio). Edit mode: Heartbeat ~240 Hz;
    // per render step the BindToRenderStep callbacks, the tween step, then RenderStepped before
    // PreRender. PreAnimation / Stepped / PreSimulation / PostSimulation never fire in edit mode.
    // A server (--run-context server) answers as Roblox's RunService documentation's table gives a Play-mode
    // server: IsServer, IsStudio and IsRunning true, IsClient and IsRunMode false (IsEdit is refused to game scripts:
    // below). It renders nothing: BindToRenderStep callbacks and PreRender never run, and RenderStepped refuses a
    // connection (measured in a server's game script, Studio 0.744: tools/parity/probes/run_service_sides.luau).
    // `--studio false` answers IsStudio as a live game does (false), on either side.
    if let Some(run_service) = class(reflection, "RunService") {
        let mode_edit = options.edit_mode();
        let server = options.server();
        // (Studio's edit DataModel, `--run-context edit`, is both sides: the same documentation's table, not measured)
        let both = options.run_context == crate::RunContext::Edit;
        let studio = options.studio;
        behaviors.add_method(run_service, K::IsStudio as NameId, constant(move |_| Ok(LuaValue::Boolean(studio))));
        behaviors.add_method(run_service, K::IsClient as NameId, constant(move |_| Ok(LuaValue::Boolean(!server))));
        behaviors.add_method(run_service, K::IsServer as NameId, constant(move |_| Ok(LuaValue::Boolean(server || both))));
        behaviors.add_method(run_service, K::IsRunning as NameId, constant(move |_| Ok(LuaValue::Boolean(!mode_edit))));
        behaviors.add_method(run_service, K::IsRunMode as NameId, constant(|_| Ok(LuaValue::Boolean(false))));
        // (a game script may not ask: "The current thread cannot call 'IsEdit' (lacking capability PluginOrOpenCloud)" on a
        // playing client and on a server, measured, Studio 0.744: tools/parity/probes/run_service_sides.luau; edit mode, where
        // a plugin's story runs, answers)
        behaviors.add_method(run_service, K::IsEdit as NameId, method(move |_, _, _, _| {
            if !mode_edit {
                return raise("The current thread cannot call 'IsEdit' (lacking capability PluginOrOpenCloud)");
            }
            Ok(values([LuaValue::Boolean(true)]))
        }));

        let steps: Rc<RefCell<Vec<Rc<RenderStep>>>> = Rc::default();
        let sequence = Rc::new(std::cell::Cell::new(0u64));
        let bind_steps = steps.clone();
        behaviors.add_method(run_service, K::BindToRenderStep as NameId, method(move |lua, engine, _, args| {
            let LuaValue::Function(callback) = args.get(3).clone() else {
                return raise("Argument 3 missing or nil");
            };
            // (a priority that is no number raises "Unable to cast string to int": measured in a playing client, Studio 0.744,
            // tools/parity/probes/frame_phases.luau, with "high"; a numeric string is read as its number)
            let level = match args.get(2) {
                LuaValue::String(_) => match lua.coerce_number(args.get(2).clone())? {
                    Some(level) => level,
                    None => return raise("Unable to cast string to int"),
                },
                _ => args.number(2).unwrap_or(0.0),
            };
            sequence.set(sequence.get() + 1);
            let level = if level >= 0.0 { level.floor() } else { level.ceil() };
            // (the name last, as a client casts the arguments from the last: services::cast_strings)
            let name = match args.get(1) {
                LuaValue::Nil => return raise("Argument 1 missing or nil"),
                value => String::from_utf8_lossy(&convert::string_param(lua, engine, value, 1, true)?).into_owned(),
            };
            let mut list = bind_steps.borrow_mut();
            list.push(Rc::new(RenderStep { name, priority: level, callback, sequence: sequence.get() }));
            crate::sort::by(&mut list, |a, b| a.priority.partial_cmp(&b.priority).unwrap_or(std::cmp::Ordering::Equal).then(a.sequence.cmp(&b.sequence)));
            Ok(none())
        }));
        let unbind_steps = steps.clone();
        // (every binding of the name goes; when it took more than one, a client and a server warn "RunService:UnbindFromRenderStep
        // removed different functions with same reference name <name> <n> times.", without a stack: measured, 0.747,
        // tools/parity/probes/service_answers.luau)
        behaviors.add_method(run_service, K::UnbindFromRenderStep as NameId, strings(FIRST, method(move |_, engine, _, args| {
            let name = args.string(1).map(|s| s.to_string_lossy());
            let removed = {
                let mut list = unbind_steps.borrow_mut();
                let before = list.len();
                list.retain(|step| Some(&step.name) != name.as_ref());
                before - list.len()
            };
            if removed > 1 {
                let name = name.unwrap_or_default();
                engine.output("warn", format!("RunService:UnbindFromRenderStep removed different functions with same reference name {name} {removed} times."), None);
            }
            Ok(none())
        })));
        let mut state = engine.state.borrow_mut();
        let hook_steps = steps;
        state.sched.add_hook(Phase::RenderStep, Rc::new(move |lua, engine, delta| {
            if server {
                return Ok(());
            }
            // Camera:Interpolate's step: at the start of the render step, before the bindings (world/camera_moves.rs)
            crate::world::camera_moves::step(engine, delta);
            let snapshot: Vec<Rc<RenderStep>> = hook_steps.borrow().clone();
            for step in snapshot {
                // Unbound by an earlier callback of this render step: skipped
                if hook_steps.borrow().iter().any(|s| Rc::ptr_eq(s, &step)) {
                    // (a binding's call, booked to it: cost.rs RENDER_STEP_DISPATCH)
                    let thread = lua.create_thread(step.callback.clone())?;
                    super::scheduler::charge_dispatch_to(lua, engine, &thread, crate::cost::RENDER_STEP_DISPATCH, 0);
                    super::scheduler::resume(lua, engine, &thread, LuaMultiValue::from_iter([LuaValue::Number(delta)]));
                }
            }
            Ok(())
        }));
        state.sched.add_hook(Phase::RenderEvents, Rc::new(move |lua, engine, delta| {
            if server {
                return Ok(());
            }
            fire_frame_event(lua, engine, K::RenderStepped as NameId, &[Value::Number(delta)])?;
            fire_frame_event(lua, engine, K::PreRender as NameId, &[Value::Number(delta)])
        }));
        state.sched.add_hook(Phase::Simulation, Rc::new(|lua, engine, _| crate::lua::actors::start_scripts(lua, engine)));
        state.sched.add_hook(Phase::Simulation, Rc::new(|lua, engine, delta| {
            let now = engine.state.borrow().sched.now;
            fire_frame_event(lua, engine, K::PreAnimation as NameId, &[Value::Number(delta)])?;
            // The Animator step: AnimationTracks pose their rigs' joints (crate::animation)
            crate::luau_profile::engine_step("animation", || crate::animation::step(engine, delta));
            super::scheduler::flush_deferred(lua, engine)?;
            fire_frame_event(lua, engine, K::Stepped as NameId, &[Value::Number(now), Value::Number(delta)])?;
            fire_frame_event(lua, engine, K::PreSimulation as NameId, &[Value::Number(delta)])?;
            // The deferred handlers of what Stepped and PreSimulation handlers did (a Changed or ChildAdded handler of a write)
            // run here, before the engine's step and PostSimulation: Studio's client orders a PreSimulation write's Changed,
            // AttributeChanged and ChildAdded handlers between PreSimulation and PostSimulation, and a Motor6D.Transform one of
            // them writes is applied by the batch below the same frame (a deferred_phase probe)
            super::scheduler::flush_deferred(lua, engine)?;
            // Motor6D.Transform: applied in a batch after PreSimulation (Roblox's Motor6D docs)
            crate::luau_profile::engine_step("joints", || crate::world::joints::step_transforms(engine));
            // (the legacy motors turn toward their DesiredAngle: world/joints.rs step_motors)
            crate::luau_profile::engine_step("joints", || crate::world::joints::step_motors(engine));
            // The physics step: free assemblies fall (world::gravity); the spatial index takes in what entered the Workspace
            crate::luau_profile::engine_step("physics", || crate::world::gravity::step(engine, delta));
            crate::world::broadphase::index_parts(engine);
            // Workspace.DistributedGameTime takes the frame's game time after the step, before PostSimulation; a server fires its
            // Changed for it every frame, a playing client none (Studio 0.747, tools/parity/probes/distributed_game_time.luau)
            set_game_time(engine, engine.world().workspace, now, engine.options.server());
            fire_frame_event(lua, engine, K::PostSimulation as NameId, &[Value::Number(delta)])
        }));
        state.sched.add_hook(Phase::Heartbeat, Rc::new(|lua, engine, delta| {
            fire_frame_event(lua, engine, K::Heartbeat as NameId, &[Value::Number(delta)])
        }));
    }
}

#[inline(never)]
fn install_collection_service(behaviors: &mut Behaviors, reflection: &Reflection) {
    // CollectionService
    if let Some(collection) = class(reflection, "CollectionService") {
        if let Some(n) = name(reflection, "GetTagged") {
            behaviors.add_method(collection, n, strings(FIRST, method(|lua, engine, _, args| {
                let tag = args.string(1).map(|s| Str::new(&s.as_bytes())).unwrap_or_default();
                // (the waiting bundles that hold the tag import first: the list is the game's, every bundle in place)
                #[cfg(feature = "bundles")]
                crate::project::model_assets::import_tagged(engine, Some(&*String::from_utf8_lossy(tag.as_bytes())));
                let game = engine.world().game;
                let tagged = tagged_below(&mut engine.state.borrow_mut(), game, &tag);
                // (an instance it lists: cost.rs TAGGED_LISTED)
                let listed = tagged.len() as f64 * crate::cost::TAGGED_LISTED;
                engine.state.borrow().sched.charge(listed);
                crate::boundary::add_cost(listed);
                let table = lua.create_table()?;
                for id in tagged {
                    table.raw_push(instance_value(lua, engine, id)?)?;
                }
                Ok(values([LuaValue::Table(table)]))
            })));
        }
        // GetAllTags: every tag in the DataModel, in first-seen order of a depth-first walk
        if let Some(n) = name(reflection, "GetAllTags") {
            behaviors.add_method(collection, n, method(|lua, engine, _, _| {
                // (the waiting bundles that hold any tag import first, so their tags come in the walk's order)
                #[cfg(feature = "bundles")]
                crate::project::model_assets::import_tagged(engine, None);
                let game = engine.world().game;
                let mut tags: Vec<Str> = Vec::new();
                {
                    let state = engine.state.borrow();
                    for id in state.dom.descendants_where(game, |instance| !instance.tags.is_empty()) {
                        for tag in &state.dom.at(id).tags {
                            if !tags.contains(tag) {
                                tags.push(tag.clone());
                            }
                        }
                    }
                }
                let table = lua.create_table()?;
                for tag in tags {
                    table.raw_push(lua.create_string(tag.as_bytes())?)?;
                }
                Ok(values([LuaValue::Table(table)]))
            }));
        }
        for (member, added) in [("GetInstanceAddedSignal", true), ("GetInstanceRemovedSignal", false)] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(collection, n, strings(FIRST, method(move |lua, engine, _, args| {
                    let tag = args.string(1).map(|s| Str::new(&s.as_bytes())).unwrap_or_default();
                    // (measured likewise)
                    if tag.as_bytes().is_empty() {
                        return raise(format!("Empty tag string provided to {member}"));
                    }
                    let (a, r) = tag_signals(&mut engine.state.borrow_mut(), &tag);
                    Ok(values([signal_value(lua, engine, if added { a } else { r })?]))
                })));
            }
        }
        for member in ["AddTag", "RemoveTag", "HasTag", "GetTags"] {
            if let Some(n) = name(reflection, member) {
                let forward = n;
                let tagged = member != "GetTags";
                // (what AddTag, RemoveTag and HasTag answer when they change nothing)
                let nothing = move || if member == "HasTag" { values([LuaValue::Boolean(false)]) } else { none() };
                behaviors.add_method(collection, n, method(move |lua, engine, this, args| {
                    // (GetTags of anything but an instance, nothing included, is an empty table, without a warning: measured, 0.747,
                    // a playing client and a server, tools/parity/probes/service_answers.luau)
                    if !tagged && args.instance(1).is_none() {
                        return Ok(values([LuaValue::Table(lua.create_table()?)]));
                    }
                    // (the tag first, as a client casts the arguments from the last: a nil one raises "Argument 3 missing or nil" and
                    // another value warns "argument #2", measured, Studio 0.747)
                    let cast = if tagged && args.len() > 2 { cast_strings(lua, engine, args, TAG)? } else { None };
                    let args = cast.as_ref().unwrap_or(args);
                    // (with one value and no tag, the call is Instance's on the service itself, the value its tag: AddTag(folder) tags
                    // the CollectionService "Instance", after "argument #1 expects a string, but Instance was passed", and HasTag of any
                    // instance answers true from then on; one nil raises "Argument 2 missing or nil", no argument at all "Argument 1
                    // missing or nil": measured, 0.747, a playing client and a server, tools/parity/probes/collection_tags.luau)
                    if tagged && args.len() <= 2 {
                        if args.len() < 2 {
                            return raise("Argument 1 missing or nil");
                        }
                        if args.get(1).is_nil() {
                            return raise("Argument 2 missing or nil");
                        }
                        let own = engine.reflection.class_id("Instance").and_then(|base| {
                            engine.behaviors.borrow().find_method(&engine.reflection, base, forward).map(|(_, f)| f.clone())
                        });
                        return match own {
                            Some(Method::Rust(f)) => f(lua, engine, this, args),
                            Some(Method::Lua(f)) => Ok(f.call::<LuaMultiValue>(args.0.clone())?),
                            None => Ok(nothing()),
                        };
                    }
                    // (a target that is no instance is the tag of a call on the service itself, the tag given after it unused:
                    // CollectionService:AddTag(5, "x") tags the CollectionService "5", AddTag({}, "x") "{}" after "argument #1 expects a
                    // string, but table was passed", HasTag(5, "x") answers whether it holds "5"; a nil target changes nothing (HasTag
                    // false). Measured, 0.747, a playing client and a server: tools/parity/probes/service_answers.luau)
                    let Some(target) = args.instance(1) else {
                        if args.get(1).is_nil() {
                            return Ok(nothing());
                        }
                        let own = engine.reflection.class_id("Instance").and_then(|base| {
                            engine.behaviors.borrow().find_method(&engine.reflection, base, forward).map(|(_, f)| f.clone())
                        });
                        let first = Args(args.0.iter().take(2).cloned().collect());
                        return match own {
                            Some(Method::Rust(f)) => f(lua, engine, this, &first),
                            Some(Method::Lua(f)) => Ok(f.call::<LuaMultiValue>(first.0)?),
                            None => Ok(nothing()),
                        };
                    };
                    let target_class = engine.state.borrow().dom.at(target).class;
                    let Some((_, f)) = engine.behaviors.borrow().find_method(&engine.reflection, target_class, forward).map(|(c, f)| (c, f.clone())) else {
                        return Ok(none());
                    };
                    // CollectionService:AddTag(Target, Tag) is Target:AddTag(Tag)
                    let forwarded = Args(args.0.iter().skip(1).cloned().collect());
                    match f {
                        Method::Rust(f) => f(lua, engine, target, &forwarded),
                        Method::Lua(f) => Ok(f.call::<LuaMultiValue>(forwarded.0)?),
                    }
                }));
            }
        }
    }
}

#[inline(never)]
fn install_gui_and_input(behaviors: &mut Behaviors, reflection: &Reflection, options: &crate::Options) {
    // GuiService / UserInputService
    // (the device and the player's settings are the client profile's: client_profile.rs)
    let device = options.profile.device.clone();
    let settings = options.profile.settings.clone();
    if let Some(gui) = class(reflection, "GuiService") {
        let inset = device.topbar_inset();
        if let Some(n) = name(reflection, "GetGuiInset") {
            behaviors.add_method(gui, n, method(move |lua, _, _, _| Ok(values([vector2(lua, 0.0, inset)?, vector2(lua, 0.0, 0.0)?]))));
        }
        // (a console's client is a ten-foot interface: Roblox's GuiService documentation, not measured)
        let ten_foot = device.ten_foot_interface.unwrap_or(false);
        if let Some(n) = name(reflection, "IsTenFootInterface") {
            behaviors.add_method(gui, n, constant(move |_| Ok(LuaValue::Boolean(ten_foot))));
        }
        for member in ["GetEmotesMenuOpen", "GetInspectMenuEnabled", "GetGameplayPausedNotificationEnabled"] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(gui, n, constant(|_| Ok(LuaValue::Boolean(false))));
            }
        }
        // the player's accessibility settings, where the profile sets them (else the properties' defaults)
        if let (Some(item), Some(n)) = (settings.preferred_text_size.clone(), name(reflection, "PreferredTextSize")) {
            behaviors.add_getter(gui, n, getter(move |lua, engine, _| Ok(enum_item(lua, engine, "PreferredTextSize", &item)?)));
        }
        // (1 unless the profile sets it: a 0.747 client and server read 1, measured 2026-11-26, where the dump's default is 0)
        if let Some(n) = name(reflection, "PreferredTransparency") {
            let value = settings.preferred_transparency.unwrap_or(1.0);
            behaviors.add_getter(gui, n, getter(move |_, _, _| Ok(LuaValue::Number(value))));
        }
        if let (Some(flag), Some(n)) = (settings.reduced_motion, name(reflection, "ReducedMotionEnabled")) {
            behaviors.add_getter(gui, n, getter(move |_, _, _| Ok(LuaValue::Boolean(flag))));
        }
        // (no inspect menu opens without a GUI layer, so there is none to close, and no selection group to remove but one
        // AddSelectionParent made, which reports itself)
        for (member, params) in [("CloseInspectMenu", NONE), ("RemoveSelectionGroup", FIRST)] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(gui, n, strings(params, method(|_, _, _, _| Ok(none()))));
            }
        }
        // (not kept: GetEmotesMenuOpen stays false, SelectedObject unset)
        for (member, params) in [("SetEmotesMenuOpen", NONE), ("Select", NONE), ("AddSelectionParent", FIRST)] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(gui, n, strings(params, unemulated("GuiService", member)));
            }
        }
    }
    if let Some(input) = class(reflection, "UserInputService") {
        // (a phone is a touch device: cost.rs device_scale gives it its prices too)
        for (member, value) in [
            ("TouchEnabled", device.touch_enabled()),
            ("KeyboardEnabled", device.keyboard_enabled()),
            ("MouseEnabled", device.mouse_enabled()),
            ("GamepadEnabled", device.gamepad_enabled()),
        ] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_getter(input, n, getter(move |_, _, _| Ok(LuaValue::Boolean(value))));
            }
        }
        // (inputs the profile sets that the device's kind does not decide: else the properties' defaults)
        for (member, value) in [("AccelerometerEnabled", device.inputs.accelerometer), ("GyroscopeEnabled", device.inputs.gyroscope), ("VREnabled", device.inputs.vr)] {
            if let (Some(value), Some(n)) = (value, name(reflection, member)) {
                behaviors.add_getter(input, n, getter(move |_, _, _| Ok(LuaValue::Boolean(value))));
            }
        }
        // (a touch device prefers Touch and a gamepad Gamepad: Roblox's PreferredInput documentation, and what roblox-sim's
        // device emulation writes; a desktop keeps the property's KeyboardAndMouse)
        if let (Some(item), Some(n)) = (device.preferred_input().map(str::to_string), name(reflection, "PreferredInput")) {
            behaviors.add_getter(input, n, getter(move |lua, engine, _| Ok(enum_item(lua, engine, "PreferredInput", &item)?)));
        }
        if let Some(n) = name(reflection, "GetMouseLocation") {
            behaviors.add_method(input, n, method(|lua, _, _, _| Ok(values([vector2(lua, -1000.0, -1000.0)?]))));
        }
        if let (Some(item), Some(settings_class), Some(n)) = (settings.saved_quality_level.clone(), class(reflection, "UserGameSettings"), name(reflection, "SavedQualityLevel")) {
            behaviors.add_getter(settings_class, n, getter(move |lua, engine, _| Ok(enum_item(lua, engine, "SavedQualitySetting", &item)?)));
        }
        let last_input: Rc<str> = device.last_input_type(options.edit_mode()).into();
        if let Some(n) = name(reflection, "GetLastInputType") {
            behaviors.add_method(input, n, method(move |lua, engine, _, _| Ok(values([enum_item(lua, engine, "UserInputType", &last_input)?]))));
        }
        for member in ["IsKeyDown", "IsMouseButtonPressed", "IsGamepadButtonDown", "GetGamepadConnected", "GamepadSupports"] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(input, n, constant(|_| Ok(LuaValue::Boolean(false))));
            }
        }
        for member in ["GetKeysPressed", "GetMouseButtonsPressed", "GetConnectedGamepads", "GetNavigationGamepads", "GetSupportedGamepadKeyCodes", "GetGamepadState"] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(input, n, constant(|lua| Ok(LuaValue::Table(lua.create_table()?))));
            }
        }
        if let Some(n) = name(reflection, "GetFocusedTextBox") {
            behaviors.add_method(input, n, constant(|_| Ok(LuaValue::Nil)));
        }
        if let Some(n) = name(reflection, "GetMouseDelta") {
            behaviors.add_method(input, n, constant(|lua| vector2(lua, 0.0, 0.0)));
        }
        if let Some(n) = name(reflection, "GetImageForKeyCode") {
            behaviors.add_method(input, n, constant(|lua| string_value(lua, "")));
        }
        if let Some(n) = name(reflection, "GetStringForKeyCode") {
            behaviors.add_method(input, n, method(|lua, engine, _, args| {
                let item = match args.get(1) {
                    LuaValue::UserData(ud) => ud.borrow::<super::lua::types::LEnumItem>().ok().map(|i| i.0),
                    _ => None,
                };
                let text = item.map(|i| engine.reflection.enum_item_info(i).name.to_string()).unwrap_or_default();
                Ok(values([string_value(lua, &text)?]))
            }));
        }
    }
}

/// The settings StarterGui's SetCore takes on a client: those Roblox's core scripts register (Roblox's StarterGui documentation,
/// its SetCore list and the GetCore names it marks settable). Any other raises "SetCore: <name> has not been registered by the
/// CoreScripts" (measured, 0.747; the sim's clients run no core scripts, so there every name raises: service_answers.luau).
const SET_CORE: &[&str] = &[
    "ChatActive", "PointsNotificationsActive", "BadgesNotificationsActive", "ResetButtonCallback", "ChatMakeSystemMessage", "SendNotification",
    "TopbarEnabled", "DevConsoleVisible", "PromptSendFriendRequest", "PromptUnfriend", "PromptBlockPlayer", "PromptUnblockPlayer",
    "AvatarContextMenuEnabled", "AvatarContextMenuTarget", "AddAvatarContextMenuOption", "RemoveAvatarContextMenuOption", "AvatarContextMenuTheme",
    "CoreGuiChatConnections", "ChatWindowSize", "ChatWindowPosition", "ChatBarDisabled",
];
/// The settings GetCore reads on a client (Roblox's StarterGui documentation; VRRotationIntensity only with VR on, which no
/// client profile here turns on). Any other raises "GetCore: <name> has not been registered by the CoreScripts" (measured likewise).
const GET_CORE: &[&str] = &[
    "PointsNotificationsActive", "BadgesNotificationsActive", "AvatarContextMenuEnabled", "ChatActive", "ChatWindowSize", "ChatWindowPosition",
    "ChatBarDisabled", "GetBlockedUserIds", "PlayerBlockedEvent", "PlayerUnblockedEvent", "PlayerMutedEvent", "PlayerUnmutedEvent",
    "PlayerFriendedEvent", "PlayerUnfriendedEvent", "DevConsoleVisible",
];

/// A script value's name in "Unable to cast X to Y" as a method's argument cast writes it: "double", "bool", "Array" or
/// "Dictionary" for a table, "Function", an enum item's enum (measured, 0.747: SetCoreGuiEnabled, GetPlayerByUserId,
/// GetAssetFetchStatus; tools/parity/probes/service_answers.luau).
fn cast_name(engine: &Engine, value: &LuaValue) -> String {
    match value {
        LuaValue::Number(_) | LuaValue::Integer(_) => "double".into(),
        LuaValue::Boolean(_) => "bool".into(),
        LuaValue::Table(table) => if table.raw_len() > 0 || table.is_empty() { "Array" } else { "Dictionary" }.into(),
        LuaValue::Function(_) => "Function".into(),
        LuaValue::UserData(ud) if ud.is::<super::lua::types::LEnumItem>() => match ud.borrow::<super::lua::types::LEnumItem>() {
            Ok(item) => engine.reflection.enum_item_type_name(item.0).to_string(),
            Err(_) => "EnumItem".into(),
        },
        other => super::lua::typeof_value(engine, other),
    }
}

/// A method's enum argument at `index` (1 is the first after self): an item of `enum_name`, its name or its number; nil raises
/// "Argument n missing or nil", anything else "Unable to cast <value> to <enum_name>".
fn enum_param(lua: &Lua, engine: &Engine, value: &LuaValue, index: usize, enum_name: &str) -> Result<crate::reflection::EnumItemId, Flow> {
    if value.is_nil() {
        return raise(format!("Argument {index} missing or nil"));
    }
    if let Some(item) = engine.reflection.enum_id(enum_name).and_then(|id| convert::coerce_enum_arg(lua, engine, id, value)) {
        return Ok(item);
    }
    raise(format!("Unable to cast {} to {enum_name}", cast_name(engine, value)))
}

/// A method's bool argument at `index`: a boolean, or a number (true unless 0); nil raises "Argument n missing or nil", anything
/// else "Unable to cast <value> to bool" (a string too).
fn bool_param(engine: &Engine, value: &LuaValue, index: usize) -> Result<bool, Flow> {
    match value {
        LuaValue::Nil => raise(format!("Argument {index} missing or nil")),
        LuaValue::Boolean(on) => Ok(*on),
        LuaValue::Number(number) => Ok(*number != 0.0),
        LuaValue::Integer(number) => Ok(*number != 0),
        LuaValue::String(_) => raise("Unable to cast string to bool"),
        other => raise(format!("Unable to cast {} to bool", cast_name(engine, other))),
    }
}

#[inline(never)]
fn install_starter_actions_debris_sound(behaviors: &mut Behaviors, reflection: &Reflection, options: &crate::Options) {
    // StarterGui / ContextActionService / Debris / Sound
    if let Some(starter) = class(reflection, "StarterGui") {
        let server = options.server();
        // (Roblox's own UI is not emulated: SetCore of a name the core scripts register reports itself; GetCore keeps its first
        // answers. A server's SetCore warns "StarterGui:SetCore must be called from a local script." and does nothing: measured,
        // 0.747, tools/parity/probes/service_answers.luau)
        if let Some(n) = name(reflection, "SetCore") {
            behaviors.add_method(starter, n, strings(FIRST, method(move |lua, engine, _, args| {
                let name = args.string(1).map(|s| s.to_string_lossy()).unwrap_or_default();
                if server {
                    engine.output("warn", "StarterGui:SetCore must be called from a local script.", None);
                    return Ok(none());
                }
                if !SET_CORE.contains(&name.as_str()) {
                    return raise(format!("SetCore: {name} has not been registered by the CoreScripts"));
                }
                engine.missing(Some(lua), "StarterGui", "SetCore");
                Ok(none())
            })));
        }
        // (GetCore yields on a server until the core scripts register the name: a client casts its name without the warning)
        if let Some(n) = name(reflection, "GetCore") {
            behaviors.add_method(starter, n, strings(QUIET_FIRST, method(move |lua, engine, _, args| {
                let name = args.string(1).map(|s| s.to_string_lossy()).unwrap_or_default();
                if !server && !GET_CORE.contains(&name.as_str()) {
                    return raise(format!("GetCore: {name} has not been registered by the CoreScripts"));
                }
                // (the user ids the player blocked: the client profile's social.blocked, where it gives them)
                if name == "GetBlockedUserIds" {
                    let profile = engine.profile();
                    if let Some(blocked) = profile.social.as_ref().and_then(|social| social.get("blocked")).and_then(serde_json::Value::as_array) {
                        let list = lua.create_table_with_capacity(blocked.len(), 0)?;
                        for id in blocked {
                            list.raw_push(id.as_f64().unwrap_or(0.0))?;
                        }
                        return Ok(values([LuaValue::Table(list)]));
                    }
                }
                let active = matches!(name.as_str(), "ChatActive" | "PointsNotificationsActive" | "BadgesNotificationsActive");
                Ok(values([if active { LuaValue::Boolean(true) } else { LuaValue::Nil }]))
            })));
        }
        // The core GUI switches (measured, 0.747, a playing client and a server: tools/parity/probes/service_answers.luau). Every
        // type starts enabled; SetCoreGuiEnabled(type, enabled) sets one, All sets every one, and GetCoreGuiEnabled(All) is true
        // when every one is. The enabled flag is cast first ("Argument 2 missing or nil", "Unable to cast string to bool", a
        // number is whether it is not 0), then the type ("Unable to cast double to CoreGuiType" for a number no item has, "...
        // Material to CoreGuiType" for another enum's item, an item's name or number taken). A server casts them, then warns
        // "StarterGui:SetCoreGuiEnabled must be called from a local script." and changes nothing, and GetCoreGuiEnabled warns the
        // same and answers false.
        let disabled: Rc<RefCell<rustc_hash::FxHashSet<crate::reflection::EnumItemId>>> = Rc::default();
        if let Some(n) = name(reflection, "SetCoreGuiEnabled") {
            let disabled = disabled.clone();
            behaviors.add_method(starter, n, method(move |lua, engine, _, args| {
                let enabled = bool_param(engine, args.get(2), 2)?;
                let item = enum_param(lua, engine, args.get(1), 1, "CoreGuiType")?;
                if server {
                    engine.output("warn", "StarterGui:SetCoreGuiEnabled must be called from a local script.", None);
                    return Ok(none());
                }
                let mut disabled = disabled.borrow_mut();
                if engine.reflection.enum_item_info(item).name.as_ref() == "All" {
                    disabled.clear();
                    if !enabled {
                        let info = &engine.reflection.enums[engine.reflection.enum_item_info(item).enum_type as usize];
                        disabled.extend(info.items.iter().copied().filter(|&other| other != item));
                    }
                } else if enabled {
                    disabled.remove(&item);
                } else {
                    disabled.insert(item);
                }
                Ok(none())
            }));
        }
        if let Some(n) = name(reflection, "GetCoreGuiEnabled") {
            behaviors.add_method(starter, n, method(move |lua, engine, _, args| {
                let item = enum_param(lua, engine, args.get(1), 1, "CoreGuiType")?;
                if server {
                    engine.output("warn", "StarterGui:GetCoreGuiEnabled must be called from a local script.", None);
                    return Ok(values([LuaValue::Boolean(false)]));
                }
                let disabled = disabled.borrow();
                let on = if engine.reflection.enum_item_info(item).name.as_ref() == "All" { disabled.is_empty() } else { !disabled.contains(&item) };
                Ok(values([LuaValue::Boolean(on)]))
            }));
        }
    }
    if let Some(actions) = class(reflection, "ContextActionService") {
        // Without an input layer no binding is kept (GetBoundActionInfo answers none), so a bind and a binding's touch button
        // settings report themselves, and an unbind has nothing to remove
        // (an action's name, and a binding's title, image and description, are read as a client reads a string parameter)
        for (member, params) in [
            ("BindAction", FIRST),
            ("BindActionAtPriority", FIRST),
            ("SetTitle", FIRST_TWO),
            ("SetImage", FIRST_TWO),
            ("SetDescription", FIRST_TWO),
            ("SetPosition", FIRST),
            ("BindActivate", NONE),
        ] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(actions, n, strings(params, unemulated("ContextActionService", member)));
            }
        }
        for (member, params) in [("UnbindAction", FIRST), ("UnbindAllActions", NONE), ("UnbindActivate", NONE)] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(actions, n, strings(params, method(|_, _, _, _| Ok(none()))));
            }
        }
        if let Some(n) = name(reflection, "GetButton") {
            behaviors.add_method(actions, n, strings(FIRST, constant(|_| Ok(LuaValue::Nil))));
        }
        for (member, params) in [("GetAllBoundActionInfo", NONE), ("GetBoundActionInfo", FIRST)] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(actions, n, strings(params, constant(|lua| Ok(LuaValue::Table(lua.create_table()?)))));
            }
        }
    }
    if let (Some(debris), Some(n)) = (class(reflection, "Debris"), name(reflection, "AddItem")) {
        behaviors.add_method(debris, n, method(|lua, engine, _, args| {
            // Measured in a playing client (Studio 0.744, tools/parity/probes/lifetime.luau): an item that is no Instance
            // raises "Unable to cast value to Object" (nil is ignored), a lifetime that is no number "Unable to cast string to
            // double", and a lifetime of 0 or less removes the item before the next Heartbeat (here at once; whether the
            // client does it in the call is not measured)
            let item_value = args.get(1);
            if !item_value.is_nil() && args.instance(1).is_none() {
                return raise("Unable to cast value to Object");
            }
            let lifetime = match args.get(2) {
                LuaValue::String(_) => match lua.coerce_number(args.get(2).clone())? {
                    Some(lifetime) => lifetime,
                    None => return raise("Unable to cast string to double"),
                },
                _ => args.number(2).unwrap_or(10.0),
            };
            if let (Some(item), true) = (args.instance(1), lifetime <= 0.0) {
                super::lua::instance::destroy(lua, engine, item)?;
                return Ok(none());
            }
            if let Some(item) = args.instance(1) {
                super::lua::refs::admit(engine, super::lua::refs::Hold::Work, "AddItem")?;
                // The item rides along as the timer's argument: queued work keeps it alive
                // (lua/raw.rs collection) until Debris destroys it
                let e = engine.clone();
                let body = super::lua::create_boxed_void_val(lua, Box::new(move |lua, item: LuaValue| {
                    if let Some(item) = super::lua::instance_id(&item) {
                        // (a client's Debris destroys the item in the engine's own step, outside any script: no script pays for it)
                        let muted = crate::scheduler::Muted::raise(&e.state.borrow().sched.muted);
                        let _ = super::lua::instance::destroy(lua, &e, item);
                        drop(muted);
                    }
                    Ok(())
                }))?;
                let thread = lua.create_thread(body)?;
                engine.state.borrow_mut().sched.schedule_task(TimerKind::Delay, lifetime, thread, vec![Value::Instance(item)]);
            }
            Ok(none())
        }));
    }
    if let Some(sound) = class(reflection, "Sound") {
        // (Play / Stop / Pause / Resume are engine::world's: they set Playing and report the play. A Sound with no SoundId is not
        // loaded and 0 long, measured, 0.747, tools/parity/probes/readonly_properties.luau; one with an id loads at once here, 1 long)
        let has_id = |engine: &Rc<Engine>, id: InstanceId| {
            let state = engine.state.borrow();
            engine.reflection.name_id("SoundId").and_then(|n| state.dom.raw_get(id, n)).is_some_and(|value| !matches!(value, Value::String(text) if text.as_bytes().is_empty()))
        };
        if let Some(n) = name(reflection, "IsLoaded") {
            behaviors.add_getter(sound, n, getter(move |_, engine, id| Ok(LuaValue::Boolean(has_id(engine, id)))));
        }
        if let Some(n) = name(reflection, "TimeLength") {
            behaviors.add_getter(sound, n, getter(move |_, engine, id| Ok(LuaValue::Number(if has_id(engine, id) { 1.0 } else { 0.0 }))));
        }
    }
}

/// Read-only properties of services and the Terrain a playing client and a server answer where the dump gives no default or
/// another one (measured, 0.747: tools/parity/probes/readonly_properties.luau, the `service` lines; a server's BaseUrl is "").
/// Not here: the Terrain's CurrentPhysicalProperties, MaxExtents and ResizeableFaces (datatypes), Players.PreferredPlayers (1 on a
/// client, 0 on its server: not explained).
#[inline(never)]
fn install_read_only_answers(behaviors: &mut Behaviors, reflection: &Reflection, options: &crate::Options) {
    let server = options.server();
    let flags: &[(&str, &str, bool)] = &[
        ("Workspace", "FilteringEnabled", true),
        ("Players", "ClassicChat", true),
        ("TextChatService", "ChatTranslationEnabled", true),
        ("VRService", "ThirdPersonFollowCamEnabled", true),
        ("TestService", "Enabled", true),
        ("Terrain", "IsSmooth", true),
    ];
    for &(class_name, member, value) in flags {
        if let (Some(owner), Some(n)) = (class(reflection, class_name), name(reflection, member)) {
            behaviors.add_getter(owner, n, getter(move |_, _, _| Ok(LuaValue::Boolean(value))));
        }
    }
    let numbers: &[(&str, &str, f64)] = &[
        ("Selection", "SelectionThickness", 0.02f32 as f64),
        ("Terrain", "ResizeIncrement", 1.0),
        ("Terrain", "SpecificGravity", 0.7f32 as f64),
    ];
    for &(class_name, member, value) in numbers {
        if let (Some(owner), Some(n)) = (class(reflection, class_name), name(reflection, member)) {
            behaviors.add_getter(owner, n, getter(move |_, _, _| Ok(LuaValue::Number(value))));
        }
    }
    if let (Some(terrain), Some(n)) = (class(reflection, "Terrain"), name(reflection, "ExtentsSize")) {
        behaviors.add_getter(terrain, n, getter(|lua, engine, _| Ok(convert::to_lua(lua, engine, &Value::Vector3(dom::Vector3::new(2044.0, 252.0, 2044.0)))?)));
    }
    if let (Some(content), Some(n)) = (class(reflection, "ContentProvider"), name(reflection, "BaseUrl")) {
        let url = if server { "" } else { "https://www.roblox.com/" };
        behaviors.add_getter(content, n, getter(move |lua, _, _| Ok(string_value(lua, url)?)));
    }
    // (the players in Players, as GetPlayers lists them)
    if let Some(players) = class(reflection, "Players") {
        for member in ["NumPlayers", "numPlayers"] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_getter(players, n, getter(|_, engine, id| Ok(LuaValue::Number(joined_players(engine, id).len() as f64))));
            }
        }
    }
}

#[inline(never)]
fn install_remotes_and_http(behaviors: &mut Behaviors, reflection: &Reflection) {
    // Scripts' Source reads their text, as Studio's command bar and plugins read it (the mounted
    // file of a project script, else the stored property)
    if let Some(container) = class(reflection, "LuaSourceContainer") {
        behaviors.add_getter(container, K::Source as NameId, getter(|lua, engine, id| {
            let text = super::lua::modules::source_of(engine, id).unwrap_or_default();
            Ok(LuaValue::String(lua.create_string(&text)?))
        }));
    }

    // Remotes never reach a server offline (a client run's: a server run's and a loopback run's are their own), so FireServer
    // reports itself; Bindables fire locally
    for remote in ["RemoteEvent", "UnreliableRemoteEvent"] {
        if let (Some(c), Some(n)) = (class(reflection, remote), name(reflection, "FireServer")) {
            behaviors.add_method(c, n, unemulated(remote, "FireServer"));
        }
    }
    if let Some(bindable) = class(reflection, "BindableEvent") {
        behaviors.add_method(bindable, K::Fire as NameId, method(|lua, engine, id, args| {
            // (raw.rs fires one without table, function or thread arguments, which hold nothing for later; tables and
            // buffers are copied as Roblox copies them, threads arrive as nil: lua/bindable.rs)
            super::lua::refs::admit(engine, super::lua::refs::Hold::Work, "Fire")?;
            let mut arguments: Vec<Value> = Vec::with_capacity(args.0.len().saturating_sub(1));
            // (a table's copy starts above the call's frame, `self` and the arguments: lua/bindable.rs)
            let height = args.0.len();
            let mut tally = super::lua::bindable::Tally::default();
            for argument in args.0.iter().skip(1) {
                match super::lua::bindable::sent_value(lua, engine, argument, height, &mut tally) {
                    Ok(value) => arguments.push(value),
                    Err(refused) => return refused.raise(),
                }
            }
            core::fire_event_measured(&mut engine.state.borrow_mut(), id, K::Event as NameId, arguments, crate::cost::SignalKind::Bindable);
            Ok(none())
        }));
    }
    // HttpService (GUIDs and URL encoding; JSON and requests are phase 4)
    if let Some(http) = class(reflection, "HttpService") {
        let counter = Rc::new(std::cell::Cell::new(0u32));
        if let Some(n) = name(reflection, "GenerateGUID") {
            behaviors.add_method(http, n, method(move |lua, _, _, args| {
                counter.set(counter.get() + 1);
                let hex = format!("{:08X}", counter.get());
                let guid = format!("{hex}-0000-4000-8000-00000000{}", &hex[4..8]);
                let text = if matches!(args.get(1), LuaValue::Boolean(false)) { guid } else { format!("{{{guid}}}") };
                Ok(values([string_value(lua, &text)?]))
            }));
        }
        // JSONEncode / JSONDecode as a playing client runs them (json.rs), and the disabled requests; their errors carry
        // no position, as Roblox raises them
        if let Some(n) = name(reflection, "JSONEncode") {
            behaviors.add_method(http, n, method(|lua, engine, _, args| match json::encode_counted(lua, engine, args.get(1)) {
                Ok(encoded) => {
                    // (priced by what it wrote: cost.rs json)
                    engine.state.borrow().sched.charge(json::priced(encoded.result.len(), encoded.values, false));
                    Ok(values([string_value(lua, &encoded.result)?]))
                }
                Err(refused) => refused.raise(),
            }));
        }
        if let Some(n) = name(reflection, "JSONDecode") {
            // (any other value is read as its text, as a client reads a string parameter: 12.5 and true parse, a table is "{}" and
            // decodes to a new empty table, an Instance is "Instance", which does not parse; measured, json_limits.luau and
            // string_parameter_methods.luau)
            behaviors.add_method(http, n, strings(FIRST, method(|lua, engine, _, args| {
                let text = args.string(1).map(|text| text.as_bytes().to_vec()).unwrap_or_default();
                match json::decode_counted(lua, &text) {
                    Ok(decoded) => {
                        // (priced by the text it parsed and the values it made: cost.rs json)
                        engine.state.borrow().sched.charge(json::priced(text.len(), decoded.values, true));
                        Ok(values([decoded.result]))
                    }
                    Err(message) => raise(message),
                }
            })));
        }
        for member in ["GetAsync", "PostAsync", "RequestAsync"] {
            if let Some(n) = name(reflection, member) {
                behaviors.add_method(http, n, method(|_, _, _, _| raise("Http requests are not enabled. Enable via game settings")));
            }
        }
        if let Some(n) = name(reflection, "UrlEncode") {
            behaviors.add_method(http, n, strings(FIRST, method(|lua, _, _, args| {
                let text = args.string(1).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
                let mut encoded = String::new();
                for byte in text {
                    // (every byte but a letter or a digit, "-", "_", "." and "~" too: "0.1-0" is "0%2E1%2D0"; measured, a 0.747 client
                    // and server, tools/parity/probes/service_answers.luau)
                    if byte.is_ascii_alphanumeric() {
                        encoded.push(byte as char);
                    } else {
                        encoded.push_str(&format!("%{byte:02X}"));
                    }
                }
                Ok(values([string_value(lua, &encoded)?]))
            })));
        }
    }
}

/// Tables JSONEncode nests at most. Each level is native stack in the encoder (this copy, mlua's serde, serde_json): a
/// 1 MiB stack, a library host's main thread, held about 700 before it overflowed and ended the process. Roblox's limit is
/// not measured (a Studio probe of nesting up to 10,000 and of a cycle, on 2026-10-30, closed Studio); 256 is ree's.
const JSON_DEPTH: usize = 256;

/// Services.JsonSafe: engine values become JSON-encodable data as HttpService makes them (non-JSON types are dropped).
/// `path` holds the tables being copied: a table inside itself (that copy never ended, and overflowed the stack) and
/// nesting past [`JSON_DEPTH`] are the script's errors, as is a copy past ree's guard (lua/bindable.rs `Tally`: a table is
/// copied for each place it appears).
pub(crate) fn json_safe(lua: &Lua, value: &LuaValue, path: &mut Vec<*const std::ffi::c_void>, tally: &mut super::lua::bindable::Tally) -> Result<LuaValue, Flow> {
    if let Err(message) = tally.add(if let LuaValue::String(text) = value { text.as_bytes().len() } else { 0 }) {
        return raise(message);
    }
    Ok(match value {
        LuaValue::Table(table) => {
            let pointer = table.to_pointer();
            if path.contains(&pointer) {
                // (Roblox's message as remembered, not measured: see JSON_DEPTH)
                return raise("tables cannot be cyclic");
            }
            if path.len() >= JSON_DEPTH {
                return raise(format!("tables nested more than {JSON_DEPTH} levels deep cannot be encoded"));
            }
            path.push(pointer);
            let result = lua.create_table()?;
            for pair in table.pairs::<LuaValue, LuaValue>() {
                let (key, item) = pair?;
                result.raw_set(key, json_safe(lua, &item, path, tally)?)?;
            }
            path.pop();
            LuaValue::Table(result)
        }
        LuaValue::Number(_) | LuaValue::Integer(_) | LuaValue::String(_) | LuaValue::Boolean(_) => value.clone(),
        _ => LuaValue::Nil,
    })
}

/// FireFrameEvent: RunService's event, if a script ever created it, with immediate handlers.
fn fire_frame_event(lua: &Lua, engine: &Rc<Engine>, event: NameId, args: &[Value]) -> LuaResult<()> {
    let game = engine.world().game;
    let Some(run_service) = find_service(engine, game, "RunService") else { return Ok(()) };
    let signal = engine.state.borrow().dom.at(run_service).signals.as_ref().and_then(|s| s.event(event));
    if let Some(signal) = signal {
        fire_immediate_costing(lua, engine, signal, args, crate::cost::FRAME_HANDLER)?;
    }
    Ok(())
}

/// Script-visible GUI behaviours that need no layout (runtime/layout/install.luau's property rules:
/// TextScaled / TextWrapped, UICorner radii, Tween* methods, the static GuiState and TextBox
/// answers). The geometry parts (Absolute* / TextBounds getters, CanvasPosition clamping,
/// ApplyLayout, UIPageLayout navigation, the layout pass and its invalidation hooks), ContentText,
/// TextService's measurements and live input are a host layer's (crate::Layer).
#[inline(never)]
pub fn install_gui_behaviors(engine: &Rc<Engine>) {
    let reflection = engine.reflection.clone();
    let mut behaviors = engine.behaviors.borrow_mut();
    let n = |name: &str| reflection.name_id(name);
    // (a host layer's TextService:GetTextSize, measuring with real fonts, replaces this one)
    crate::text_measure::install(engine, &mut behaviors);

    for class_name in ["TextLabel", "TextButton", "TextBox"] {
        let Some(class) = class(&reflection, class_name) else { continue };
        // (ContentText needs rich-text parsing: a host layer's)
        // (empty until the text object is in the DataModel: measured, 0.747, tools/parity/probes/readonly_properties.luau)
        if let Some(name) = n("LocalizedText") {
            behaviors.add_getter(class, name, getter(|lua, engine, id| {
                if !in_data_model(&engine.state.borrow(), id) {
                    return Ok(LuaValue::String(lua.create_string("")?));
                }
                let text = engine.state.borrow().dom.raw_get(id, K::Text as NameId).cloned().unwrap_or(Value::String(Str::default()));
                Ok(convert::to_lua(lua, engine, &text)?)
            }));
        }
        // Measured: assigning TextScaled = true also sets TextWrapped = true, and assigning
        // TextWrapped = false also clears TextScaled (TextScaled = false leaves TextWrapped alone).
        if let (Some(scaled), Some(wrapped)) = (n("TextScaled"), n("TextWrapped")) {
            behaviors.add_setter(class, scaled, Rc::new(move |lua, engine, id, value| {
                let class = engine.state.borrow().dom.at(id).class;
                let on = match engine.reflection.class_property(class, scaled).cloned() {
                    Some(info) => convert::coerce(lua, engine, &info, &value)?,
                    None => Value::Bool(matches!(value, LuaValue::Boolean(true))),
                };
                let flag = matches!(on, Value::Bool(true));
                set_property(engine, id, scaled, on);
                if flag {
                    set_property(engine, id, wrapped, Value::Bool(true));
                }
                Ok(())
            }));
            behaviors.add_setter(class, wrapped, Rc::new(move |lua, engine, id, value| {
                let class = engine.state.borrow().dom.at(id).class;
                let on = match engine.reflection.class_property(class, wrapped).cloned() {
                    Some(info) => convert::coerce(lua, engine, &info, &value)?,
                    None => Value::Bool(matches!(value, LuaValue::Boolean(true))),
                };
                let flag = matches!(on, Value::Bool(true));
                set_property(engine, id, wrapped, on);
                if !flag {
                    set_property(engine, id, scaled, Value::Bool(false));
                }
                Ok(())
            }));
        }
    }
    if let Some(text_box) = class(&reflection, "TextBox") {
        // (focus is the input layer's: without one, nothing takes it)
        for member in ["CaptureFocus", "ReleaseFocus"] {
            if let Some(name) = n(member) {
                behaviors.add_method(text_box, name, unemulated("TextBox", member));
            }
        }
        if let Some(name) = n("IsFocused") {
            behaviors.add_method(text_box, name, constant(|_| Ok(LuaValue::Boolean(false))));
        }
    }
    for class_name in ["ImageLabel", "ImageButton"] {
        if let (Some(class), Some(name)) = (class(&reflection, class_name), n("IsLoaded")) {
            behaviors.add_getter(class, name, getter(|_, _, _| Ok(LuaValue::Boolean(true))));
        }
    }
    if let Some(gui_object) = class(&reflection, "GuiObject") {
        if let Some(name) = n("GuiState") {
            behaviors.add_getter(gui_object, name, getter(|lua, engine, _| Ok(enum_item(lua, engine, "GuiState", "Idle")?)));
        }
        // Tween* methods apply the end values at once and report Completed at the next resumption.
        // Their arguments are cast first, from the last to the first (Studio 0.747, a playing client and a server:
        // tools/parity/probes/tween_args.luau): the callback (nil or a function, else "Unable to cast <value> to Function"),
        // override (nil, a boolean or a number, else "Unable to cast <value> to bool"), the time, the style, the direction and
        // the UDim2s, the last first (a string "Unable to cast string to UDim2", nil "Argument 1 missing or nil"). Spring and
        // Bezier are refused then ("GuiObject:TweenPosition does not support Enum.EasingStyle.Spring or Enum.EasingStyle.Bezier",
        // by item, name or value, nothing moved and no callback); and an object outside the DataModel is refused last ("Can
        // only tween objects in the workspace": a Frame with no parent, or under a ScreenGui or Folder that has none).
        let tweens: [(&str, &[&str]); 3] = [
            ("TweenPosition", &["Position"]),
            ("TweenSize", &["Size"]),
            ("TweenSizeAndPosition", &["Size", "Position"]),
        ];
        for (member, fields) in tweens {
            let Some(name) = n(member) else { continue };
            let fields: Vec<NameId> = fields.iter().filter_map(|f| n(f)).collect();
            behaviors.add_method(gui_object, name, method(move |lua, engine, id, args| {
                let after = 1 + fields.len();
                let callback = match args.get(after + 4) {
                    LuaValue::Nil => None,
                    LuaValue::Function(callback) => Some(callback.clone()),
                    other => return raise(format!("Unable to cast {} to Function", tween::cast_name(engine, other))),
                };
                match args.get(after + 3) {
                    LuaValue::Nil | LuaValue::Boolean(_) | LuaValue::Number(_) | LuaValue::Integer(_) => {}
                    other => return raise(format!("Unable to cast {} to bool", tween::cast_name(engine, other))),
                }
                if !args.get(after + 2).is_nil() {
                    tween::float_arg(lua, engine, args, after + 2)?;
                }
                let style = if args.get(after + 1).is_nil() { None } else { Some(tween::enum_arg(lua, engine, args, after + 1, "EasingStyle")?) };
                if !args.get(after).is_nil() {
                    tween::enum_arg(lua, engine, args, after, "EasingDirection")?;
                }
                let mut ends = Vec::with_capacity(fields.len());
                for index in (0..fields.len()).rev() {
                    let value = args.get(1 + index);
                    match convert::from_lua(value) {
                        end @ Value::UDim2(_) => ends.push(end),
                        Value::Nil => return raise(format!("Argument {} missing or nil", 1 + index)),
                        _ => return raise(format!("Unable to cast {} to UDim2", tween::cast_name(engine, value))),
                    }
                }
                ends.reverse();
                if let Some(message) = style.and_then(|style| tween::unsupported_style(engine, style, &format!("GuiObject:{member}"))) {
                    return raise(message);
                }
                if !in_data_model(&engine.state.borrow(), id) {
                    return raise("Can only tween objects in the workspace");
                }
                for (field, end) in fields.iter().zip(ends) {
                    set_property(engine, id, *field, end);
                }
                if let Some(callback) = callback {
                    let thread = lua.create_thread(callback)?;
                    let status = engine.reflection.enum_item("TweenStatus", "Completed").map_or(Value::Nil, Value::EnumItem);
                    super::lua::instance::defer_thread(engine, thread, vec![status]);
                }
                Ok(values([LuaValue::Boolean(true)]))
            }));
        }
    }
    // Settings only a plugin or the engine's own scripts use (reflection.rs CAPABILITY_ONLY): a game script's read and
    // write raise as a client's do; the engine reads the stored values itself
    for &(class_name, member, capability) in crate::reflection::CAPABILITY_ONLY {
        let (Some(owner), Some(name)) = (class(&reflection, class_name), n(member)) else { continue };
        let read = format!("The current thread cannot read '{member}' (lacking capability {capability})");
        let write = format!("The current thread cannot write '{member}' (lacking capability {capability})");
        behaviors.add_getter(owner, name, getter(move |_, _, _| raise(read.clone())));
        behaviors.add_setter(owner, name, Rc::new(move |_, _, _, _| raise(write.clone())));
    }
    // Properties a game script reads but may not write (reflection.rs CAPABILITY_WRITES): a write raises as a client's does; a
    // plugin's code (the edit run context) writes it through the setter it had
    if !engine.options.edit_mode() {
        for &(class_name, member, capability) in crate::reflection::CAPABILITY_WRITES {
            let (Some(owner), Some(name)) = (class(&reflection, class_name), n(member)) else { continue };
            let write = format!("The current thread cannot write '{member}' (lacking capability {capability})");
            behaviors.add_setter(owner, name, Rc::new(move |_, _, _, _| raise(write.clone())));
        }
    }
    // UIPageLayout.EasingStyle keeps its value when Spring or Bezier is assigned (Studio 0.747: by item, name or value;
    // no error and no Changed)
    if let (Some(layout), Some(name)) = (class(&reflection, "UIPageLayout"), n("EasingStyle")) {
        behaviors.add_setter(layout, name, Rc::new(move |lua, engine, id, value| {
            let class = engine.state.borrow().dom.at(id).class;
            let Some(info) = engine.reflection.class_property(class, name).cloned() else { return Ok(()) };
            let style = convert::coerce(lua, engine, &info, &value)?;
            if let Value::EnumItem(item) = style
                && tween::unsupported_style(engine, item, "").is_some()
            {
                return Ok(());
            }
            set_property(engine, id, name, style);
            Ok(())
        }));
    }
    // A Camera's projection is not in use yet: Studio 0.747 lists ProjectionType, OrthographicMode, OrthographicSize and
    // ViewStretch, reads their defaults (Perspective, Vertical, 35, 1) and drops every write without a Changed. The two
    // enums still refuse what is no item of theirs; the two numbers take any value silently (a string, a table, nil).
    if let Some(camera) = class(&reflection, "Camera") {
        for member in ["ProjectionType", "OrthographicMode"] {
            let Some(name) = n(member) else { continue };
            behaviors.add_setter(camera, name, Rc::new(move |lua, engine, id, value| {
                let class = engine.state.borrow().dom.at(id).class;
                if let Some(info) = engine.reflection.class_property(class, name).cloned() {
                    convert::coerce(lua, engine, &info, &value)?;
                }
                Ok(())
            }));
        }
        for member in ["OrthographicSize", "ViewStretch"] {
            if let Some(name) = n(member) {
                behaviors.add_setter(camera, name, Rc::new(|_, _, _, _| Ok(())));
            }
        }
    }
    // Number ranges, measured in Studio 0.747: a write outside is stored as the nearest bound, and changes nothing when
    // that is the value already (ParticleEmitter2D.TimeScale 0.5, 5, 9, 1 from 1: Changed twice). InputBinding.Deadzone
    // holds each end within 0 and 1 (2..3 reads 1 1).
    const RANGES: &[(&str, &str, f64, f64)] = &[
        ("ParticleEmitter", "TimeScale", 0.0, 1.0),
        ("ParticleEmitter2D", "TimeScale", 0.0, 1.0),
        ("ParticleEmitter2D", "Brightness", 0.0, 10000.0),
        ("ParticleEmitter2D", "Blend", 0.0, 1.0),
        ("ParticleEmitter2D", "FlipbookSizeX", 1.0, 64.0),
        ("ParticleEmitter2D", "FlipbookSizeY", 1.0, 64.0),
        ("UIBlur", "Transparency", 0.0, 1.0),
    ];
    for &(class_name, member, low, high) in RANGES {
        let (Some(owner), Some(name)) = (class(&reflection, class_name), n(member)) else { continue };
        behaviors.add_setter(owner, name, Rc::new(move |lua, engine, id, value| {
            let class = engine.state.borrow().dom.at(id).class;
            let Some(info) = engine.reflection.class_property(class, name).cloned() else { return Ok(()) };
            let value = match convert::coerce(lua, engine, &info, &value)? {
                Value::Number(number) => Value::Number(number.clamp(low, high)),
                other => other,
            };
            set_property(engine, id, name, value);
            Ok(())
        }));
    }
    if let (Some(binding), Some(name)) = (class(&reflection, "InputBinding"), n("Deadzone")) {
        behaviors.add_setter(binding, name, Rc::new(move |lua, engine, id, value| {
            let class = engine.state.borrow().dom.at(id).class;
            let Some(info) = engine.reflection.class_property(class, name).cloned() else { return Ok(()) };
            let value = match convert::coerce(lua, engine, &info, &value)? {
                Value::NumberRange(range) => Value::NumberRange(dom::NumberRange::new(range.min.clamp(0.0, 1.0), range.max.clamp(0.0, 1.0))),
                other => other,
            };
            set_property(engine, id, name, value);
            Ok(())
        }));
    }
    // (ApplyLayout, UIPageLayout navigation, the Absolute* / TextBounds getters and the
    // CanvasPosition setter are a layout layer's)
    // Measured: CornerRadius writes all four per-corner radii and reads back TopLeftRadius.
    if let (Some(corner), Some(radius)) = (class(&reflection, "UICorner"), n("CornerRadius")) {
        behaviors.add_getter(corner, radius, getter(|lua, engine, id| {
            let value = engine.state.borrow().dom.raw_get(id, K::TopLeftRadius as NameId).cloned().unwrap_or(Value::Nil);
            Ok(convert::to_lua(lua, engine, &value)?)
        }));
        behaviors.add_setter(corner, radius, Rc::new(move |_, engine, id, value| {
            let converted = convert::from_lua(&value);
            if !matches!(converted, Value::UDim(_)) {
                return raise(format!(
                    "Unable to assign property CornerRadius. UDim expected, got {}",
                    super::lua::typeof_value(engine, &value)
                ));
            }
            for name in [K::TopLeftRadius, K::TopRightRadius, K::BottomRightRadius, K::BottomLeftRadius] {
                set_property(engine, id, name as NameId, converted.clone());
            }
            let hooks = engine.hooks.borrow();
            let mut state = engine.state.borrow_mut();
            core::notify_changed(&mut state, &engine.reflection, &hooks, id, radius, None);
            Ok(())
        }));
    }
    drop(behaviors);
    engine.clear_member_cache();
}
