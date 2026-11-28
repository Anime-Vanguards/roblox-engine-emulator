//! Script-visible instance semantics that never run Luau code (port of the state side of
//! `runtime/engine/instance.luau`): creation, parenting with its event order, raw property access
//! with change notification, clone, attributes, tags and signal firing. Operations that run
//! scripts (Destroy, WaitForChild) are in `engine::lua`.

use crate::cost::SignalKind;
use super::{
    Engine, Hooks, InstanceId, ScriptError, ScriptResult, State,
    dom::{InstanceSignals, flags},
    reflection::{ClassId, K, NameId, ParentLock, Reflection},
    scheduler::{DispatchKind, JobTarget},
    signal::{Handler, SignalId},
    value::{Str, Value},
};

/// What a key means on a class (instance.luau ResolveMember), cached per (class, key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    Property { property: Option<u32>, getter: bool, setter: bool, write: WriteGate },
    Method { implemented: bool },
    Event,
    Callback,
    /// A property whose Read permit needs a capability the running code lacks (reflection.rs CAPABILITY_PROPERTIES): a read and a
    /// write raise naming it
    Refused { capability: &'static str },
    None,
}

/// Whether a property's Write permit lets the running code write it (reflection.rs CAPABILITY_PROPERTIES)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteGate {
    /// As the property is: writable unless read only
    Open,
    /// The permit needs this capability, which the code lacks: a write raises naming it, before the value is cast
    Refused(&'static str),
    /// The permit's capabilities are the code's (a plugin's in the edit run context): writable though a game script's view reads it
    /// as read only (Lighting.LightingStyle)
    Granted,
}

/// Properties reflection.rs CAPABILITY_PROPERTIES gates that ree leaves as they were (measured, roblox-sim 0.747, a client and a server):
/// - a script's Source needs PluginOrOpenCloud to be read or written (a game script's read raises "The current thread cannot read
///   'Source' (lacking capability PluginOrOpenCloud)"), but ree runs all of a run's code at one identity, and the code that writes
///   Source in a play run is the command bar's (identity 4: the parity probes captured so) and ree's own tests, which make scripts at
///   run time;
/// - GuiService.CoreGuiNavigationEnabled reads and writes for a game script though its Read permit names RobloxScript, and
///   VoiceChatInternal.VoiceChatState refuses a read with "VoiceChatInternal API security check failed" and a write as read only: their
///   own behaviours say so (reflection.rs PROPERTY_SUPPLEMENT, services/mod.rs install_gui_and_input).
const OPEN_PROPERTIES: &[(&str, &str)] =
    &[("Script", "Source"), ("ModuleScript", "Source"), ("GuiService", "CoreGuiNavigationEnabled"), ("VoiceChatInternal", "VoiceChatState")];

/// Properties a client refuses a game script's write of with a capability its permits do not name (measured, roblox-sim 0.747, a
/// client and a server: Player.GameplayPaused, which a game script reads, "The current thread cannot write 'GameplayPaused'
/// (lacking capability NotAccessible)")
const NOT_ACCESSIBLE_WRITES: &[(&str, &str)] = &[("Player", "GameplayPaused")];

impl Engine {
    /// ResolveMember: property (reflection), behaviour method, reflected method, event, callback,
    /// or a getter-only behaviour property.
    pub fn resolve_member(&self, class: ClassId, name: NameId) -> Member {
        if let Some(member) = self.member_cache.borrow().get(&(class, name)) {
            crate::profiler::caches::hit(crate::profiler::caches::Cache::Resolve);
            return *member;
        }
        crate::profiler::caches::miss(crate::profiler::caches::Cache::Resolve);
        let reflection = &self.reflection;
        // (a deprecated lower-case property is the property it stands for: `brickColor` is BrickColor)
        if let Some(twin) = reflection.property_alias(class, name) {
            let member = self.resolve_member(class, twin);
            self.member_cache.borrow_mut().insert((class, name), member);
            return member;
        }
        let behaviors = self.behaviors.borrow();
        let info = reflection.class(class);
        let getter = behaviors.find_getter(reflection, class, name).is_some();
        let method = behaviors.find_method_or_alias(reflection, class, name).is_some();
        // a property scripts cannot read (NotScriptable: the dump permits nothing) is no member, as in
        // Roblox, unless a behaviour gives it a getter: `part.Origin` finds the part's child named
        // Origin, not PVInstance.Origin (an effect that emits from `Glow.Inner.Origin`, an Attachment)
        let scriptable = |property: u32| reflection.property(property).scriptable || getter;
        let member = if let Some(&property) = info.properties.get(&name).filter(|&&property| scriptable(property)) {
            Member::Property {
                property: Some(property),
                getter,
                setter: behaviors.find_setter(reflection, class, name).is_some(),
                write: WriteGate::Open,
            }
        } else if method {
            Member::Method { implemented: true }
        } else if info.methods.contains(&name) {
            Member::Method { implemented: false }
        } else if info.events.contains(&name) {
            Member::Event
        } else if info.callbacks.contains(&name) {
            Member::Callback
        } else if getter {
            Member::Property {
                property: None,
                getter: true,
                setter: behaviors.find_setter(reflection, class, name).is_some(),
                write: WriteGate::Open,
            }
        } else {
            Member::None
        };
        drop(behaviors);
        let member = self.gated(class, name, member);
        self.member_cache.borrow_mut().insert((class, name), member);
        member
    }

    /// The capabilities the running code holds: a game script's in a play run (a client's and a server's alike), a plugin's in the
    /// edit run context (services/reflection_data.rs)
    pub fn script_capabilities(&self) -> u64 {
        if self.options.edit_mode() { crate::services::reflection_data::PLUGIN_CAPS } else { crate::services::reflection_data::SCRIPT_CAPS }
    }

    /// `member` as the capabilities of the running code leave it (reflection.rs CAPABILITY_PROPERTIES): refused when the property's
    /// Read permit needs one the code lacks, its writes refused (or granted) by its Write permit.
    fn gated(&self, class: ClassId, name: NameId, member: Member) -> Member {
        let reflection = &self.reflection;
        let declared = |list: &[(&str, &str)]| {
            list.iter().any(|&(owner, property)| reflection.name(name) == property && reflection.is_a(class, owner))
        };
        if let Member::Property { property, getter, setter, .. } = member
            && declared(NOT_ACCESSIBLE_WRITES)
        {
            return Member::Property { property, getter, setter, write: WriteGate::Refused("NotAccessible") };
        }
        let Some(restriction) = reflection.restriction(class, name) else { return member };
        if !matches!(member, Member::Property { .. } | Member::None) || declared(OPEN_PROPERTIES) {
            return member;
        }
        // (a property with no permit is no member scripts can use)
        if restriction.read.is_none() && restriction.write.is_none() {
            return if matches!(member, Member::Property { getter: true, .. }) { member } else { Member::None };
        }
        let held = self.script_capabilities();
        let lacking = |need: u64| crate::services::reflection_data::first_lacking(held, need);
        if let Some(capability) = restriction.read.and_then(lacking) {
            return Member::Refused { capability };
        }
        let Member::Property { property, getter, setter, .. } = member else { return member };
        let write = match restriction.write {
            Some(need) => match lacking(need) {
                Some(capability) => WriteGate::Refused(capability),
                None => WriteGate::Granted,
            },
            None => WriteGate::Open,
        };
        Member::Property { property, getter, setter, write }
    }

    /// The name a member of `class` keeps its value under: a deprecated lower-case property's is the property it stands for
    /// (`brickColor` -> BrickColor: one value, and Changed names BrickColor), any other member's its own.
    #[inline]
    pub fn member_name(&self, class: ClassId, name: NameId) -> NameId {
        self.reflection.property_alias(class, name).unwrap_or(name)
    }

    /// Drops cached member resolutions (after behaviours change).
    pub fn clear_member_cache(&self) {
        self.member_cache.borrow_mut().clear();
        // and the raw Instance API's dispatch entries built on it (lua/raw.rs)
        if let Some(api) = self.lua.try_borrow().ok().and_then(|refs| refs.raw.clone()) {
            api.clear_members();
        }
    }
}

/// InstanceModule.Create: a fresh unparented instance.
pub fn create(state: &mut State, reflection: &Reflection, class: ClassId) -> InstanceId {
    // Scheduler.NoteBuildWork (InstanceModule.Hooks.Created): a recorded clock's build placement
    state.sched.note_build_work();
    state.joints.note_created(class);
    state.dom.create(reflection, class)
}

/// Creates an instance with a stored Name (RawSet silent), as the engine builds services.
pub fn create_named(state: &mut State, reflection: &Reflection, class: ClassId, name: &str) -> InstanceId {
    state.sched.note_build_work();
    state.joints.note_created(class);
    let id = state.dom.create(reflection, class);
    raw_set_silent(state, reflection, id, K::Name as NameId, Value::String(Str::from(name)));
    id
}

/// [`create_named`] with a name the caller holds already.
pub fn create_named_str(state: &mut State, reflection: &Reflection, class: ClassId, name: Str) -> InstanceId {
    state.sched.note_build_work();
    state.joints.note_created(class);
    let id = state.dom.create(reflection, class);
    raw_set_silent(state, reflection, id, K::Name as NameId, Value::String(name));
    id
}

/// Services a script's `Instance.new` makes all the same: a new instance of the class outside the DataModel, as a playing client and a
/// server make one (measured, roblox-sim 0.747, every class data/reflection.json, data/reflection-security.json and rbx-dom's database
/// name: tools/parity/probes/instance_new_refusals.luau); every other service is "Unable to create an Instance of type".
const MADE_SERVICES: &[&str] = &["HeightmapImporterService", "MemoryStoreService", "ProximityPromptService", "TestService", "VirtualInputManager"];

/// Classes whose making needs capabilities a script may lack, with them: the class's New permit (data/reflection-security.json), for
/// the three services its GetService permit. A client's `Instance.new` raises "The current thread cannot create 'Breakpoint' (lacking
/// capability RobloxScript)" for code that lacks one, naming the first it lacks (measured likewise, a game script's set; a plugin's
/// code, ree's edit run context, holds Plugin). Player's (WritePlayer) is refused at identity 4 too, before the parent argument is
/// looked at (measured likewise, the command bar's identity: tools/parity/probes/player_new.luau): a play run's harness lets players
/// in with Context.Join (record/run.rs). The edit run context still makes one (Studio's edit mode is not measured).
const NEW_PERMITS: &[(&str, &str)] = &[
    ("AnimationPlayer", "RobloxScript"),
    ("AvatarAbilityRules", "RobloxScript"),
    ("Breakpoint", "RobloxScript"),
    ("CSGDictionaryService", "RobloxEngine"),
    ("CustomLog", "InternalTest"),
    ("DebuggerWatch", "LocalUser"),
    ("ExplorerFilter", "RobloxScript"),
    ("FlyweightService", "RobloxEngine"),
    ("FollowRotationBoneBehavior", "InternalTest"),
    ("FollowTwistBoneBehavior", "InternalTest"),
    ("Noise", "RobloxScript"),
    ("NonReplicatedCSGDictionaryService", "RobloxEngine"),
    ("Player", "WritePlayer"),
    ("PluginAction", "Plugin"),
    ("PluginCapabilities", "Plugin"),
    ("RTAnimationTracker", "RobloxScript"),
    ("RealtimeMedia", "InternalTest"),
    ("RelativeGui", "RobloxScript | UI"),
    ("StandalonePluginScripts", "Plugin"),
    ("StateMachineDefinition", "RobloxScript"),
    ("StudioAttachment", "RobloxScript"),
    ("StudioCallout", "RobloxScript"),
    ("TrackerStreamAnimation", "RobloxScript"),
    ("VideoDeviceInput", "RobloxScript"),
];

/// Classes a client's `Instance.new` refuses with words of their own (measured likewise)
const NEW_REFUSALS: &[(&str, &str)] = &[
    ("AuroraScript", "This type can not be instantiated"),
    ("InternalSyncItem", "Internal Permission is required for this feature."),
    ("ParabolaAdornment", "ParabolaAdornment may only be instantiated by CoreScripts at this time"),
];

/// What `Instance.new(class_name)` (and `Instance.fromExisting` of one) raises for code holding the capabilities `held`
/// (services/reflection_data.rs) before it makes anything, in a play run (`play`) or the edit run context: None when the class's
/// making is no capability's or the code holds it.
pub fn new_refusal(class_name: &str, held: u64, play: bool) -> Option<String> {
    if let Some(&(_, message)) = NEW_REFUSALS.iter().find(|&&(class, _)| class == class_name) {
        return Some(message.to_string());
    }
    let &(_, need) = NEW_PERMITS.iter().find(|&&(class, _)| class == class_name && (play || class != "Player"))?;
    let capability = crate::services::reflection_data::first_lacking(held, crate::services::reflection_data::caps_of(need))?;
    Some(format!("The current thread cannot create '{class_name}' (lacking capability {capability})"))
}

/// InstanceModule.New: `Instance.new(ClassName, Parent)`.
pub fn new_instance(
    state: &mut State,
    reflection: &Reflection,
    hooks: &Hooks,
    class_name: &str,
    parent: Option<InstanceId>,
) -> ScriptResult<InstanceId> {
    let class = reflection
        .class_id(class_name)
        .filter(|&class| reflection.class(class).creatable || MADE_SERVICES.contains(&class_name))
        .ok_or_else(|| ScriptError::plain(format!("Unable to create an Instance of type \"{class_name}\"")))?;
    let id = create(state, reflection, class);
    state.dom.at_mut(id).set_flag(flags::SCRIPT_CREATED, true);
    // a GetTextBoundsParams' Parent is locked as an instance's from the start: "The Parent property of Instance is locked, current
    // parent: NULL, new parent Workspace", Instance.new's parent argument too (measured, Studio 0.747, a client and a server:
    // tools/parity/probes/instance_api_texts.luau)
    if &*reflection.class(class).name == "GetTextBoundsParams" {
        state.dom.at_mut(id).set_flag(flags::PARENT_LOCKED, true);
    }
    if let Some(parent) = parent {
        set_parent(state, reflection, hooks, id, Some(parent))?;
    }
    Ok(id)
}

// Signals

/// The event signal `name` of an instance, created on first use (GetSignal).
pub fn event_signal(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId) -> SignalId {
    if let Some(signal) = state.dom.at(id).signals.as_ref().and_then(|s| s.event(name)) {
        return signal;
    }
    let declaring = reflection.declaring_class(state.dom.at(id).class, name);
    let label = format!("{}.{}", reflection.class(declaring).name, reflection.name(name));
    let signal = state.signals.create_labelled(Str::from(reflection.name(name)), Str::from(label.as_str()));
    state
        .dom
        .at_mut(id)
        .signals
        .get_or_insert_with(Box::default)
        .events
        .push((name, signal));
    if name == K::AncestryChanged as NameId {
        state.ancestry_signals = state.ancestry_signals.saturating_add(1);
    }
    signal
}

/// GetPropertyChangedSignal's signal for `name` (created on first use).
pub fn property_signal(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId) -> SignalId {
    if let Some(signal) = state.dom.at(id).signals.as_ref().and_then(|s| s.property(name)) {
        return signal;
    }
    // tostring reads "Signal SizeChanged" and diagnostics "Object.SizeChanged" (measured in a playing client, Studio 0.744)
    let changed = format!("{}Changed", reflection.name(name));
    let signal = state.signals.create_labelled(Str::from(changed.as_str()), Str::from(format!("Object.{changed}").as_str()));
    state
        .dom
        .at_mut(id)
        .signals
        .get_or_insert_with(Box::default)
        .properties
        .push((name, signal));
    signal
}

pub fn attribute_signal(state: &mut State, id: InstanceId, name: &Str) -> SignalId {
    if let Some(signal) = state.dom.at(id).signals.as_ref().and_then(|s| s.attribute(name.as_bytes())) {
        return signal;
    }
    // tostring reads "Signal nAttributeChanged" and diagnostics "Instance.nAttributeChanged" (measured in a playing client and a server,
    // Studio 0.747: tools/parity/probes/instance_api_texts.luau)
    let changed = [name.as_bytes(), b"AttributeChanged".as_slice()].concat();
    let label = [b"Instance.".as_slice(), &changed].concat();
    let signal = state.signals.create_labelled(Str::new(&changed), Str::new(&label));
    state
        .dom
        .at_mut(id)
        .signals
        .get_or_insert_with(Box::default)
        .attributes
        .push((name.clone(), signal));
    signal
}

#[inline]
fn existing_event(state: &State, id: InstanceId, name: NameId) -> Option<SignalId> {
    state.dom.at(id).signals.as_ref().and_then(|s| s.event(name))
}

/// Signal.Fire: queues every handler (deferred) and wakes threads blocked in :Wait(): an event of a kind no bench measured, priced as
/// a heard BindableEvent's (cost.rs signal_costs: a borrowed price, counted apart)
pub fn fire(state: &mut State, signal: SignalId, args: Vec<Value>) {
    fire_measured(state, signal, args, SignalKind::Unmeasured);
}

/// [`fire`], charging the firer `queue` seconds for a signal with handlers and each handler's thread `resume` seconds when it runs,
/// keeping `kind` for delivery so a later-started profile still knows where its price came from
fn fire_costing(state: &mut State, signal: SignalId, args: Vec<Value>, queue: f64, resume: f64, kind: SignalKind) {
    let has_handlers = state.signals.has_handlers(signal);
    // Kind bits of the queue charge exist only while a work profile runs, which is the one reader of them
    let profiling = state.sched.kinds.is_some();
    if profiling && has_handlers && kind == SignalKind::Unmeasured {
        crate::profiler::attribution::unmeasured_fire();
    }
    let queue_kinds = if profiling { crate::cost::signal_kinds(kind).queue } else { 0 };
    if !has_handlers {
        return;
    }
    state.sched.charge_kind(queue, queue_kinds);
    let handlers = state.signals.take_handlers(signal);
    // (a measured kind: a handler beyond the first queues and resumes for less than a lone one, cost.rs EXTRA_HANDLER_*)
    if resume > 0.0 && handlers.len() > 1 {
        state.sched.charge_kind((handlers.len() - 1) as f64 * crate::cost::EXTRA_HANDLER_QUEUED, crate::cost::kind::STUDIO);
    }
    for (index, handler) in handlers.into_iter().enumerate() {
        let (target, thread_price) = match handler {
            // a thread parked in the signal's Wait resumes at a client's measured price, whatever the signal (cost.rs RESUMED_SIGNAL_WAIT)
            Handler::Thread(thread) => {
                // (past the re-entrancy cut it is not queued: the fire logs it, scheduler.rs refuse_waiter)
                if !state.sched.can_defer_event() {
                    crate::scheduler::refuse_waiter(state);
                    continue;
                }
                let price = if resume > 0.0 { crate::cost::RESUMED_SIGNAL_WAIT } else { 0.0 };
                (JobTarget::Thread(thread), price)
            }
            // the first handler's thread costs what the kind says
            Handler::Connection(connection) if index == 0 => {
                let target = JobTarget::Handler { connection, kind: DispatchKind::Signal(kind) };
                (target, resume)
            }
            // the later ones cost less
            Handler::Connection(connection) => {
                let target = JobTarget::Handler { connection, kind: DispatchKind::ExtraHandler };
                let price = resume.min(crate::cost::EXTRA_HANDLER_THREAD);
                (target, price)
            }
        };
        state.sched.defer_handler(target, args.clone(), thread_price, signal);
    }
}

/// FireSignal: fires an instance event if it was ever created.
pub fn fire_event(state: &mut State, id: InstanceId, name: NameId, args: Vec<Value>) {
    if let Some(signal) = existing_event(state, id, name) {
        fire(state, signal, args);
    }
}

/// [`fire_event`] for an event of the kinds Studio's bench measured (cost.rs `signal_costs`)
pub fn fire_event_measured(state: &mut State, id: InstanceId, name: NameId, args: Vec<Value>, kind: SignalKind) {
    if let Some(signal) = existing_event(state, id, name) {
        fire_measured(state, signal, args, kind);
    }
}

/// [`fire`] for an event of the kinds Studio's bench measured (cost.rs `signal_costs`, `signal_kinds`)
fn fire_measured(state: &mut State, signal: SignalId, args: Vec<Value>, kind: SignalKind) {
    let (queue, resume) = crate::cost::signal_costs(kind);
    fire_costing(state, signal, args, queue, resume, kind);
}

// Properties

/// RawSet(Silent = true): stores without notifications. Returns whether the value changed.
pub fn raw_set_silent(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId, value: Value) -> bool {
    store(state, reflection, id, name, value).0
}

/// RawSet's store (one property lookup): (whether the value changed, the previous value when it
/// did).
fn store(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId, value: Value) -> (bool, Value) {
    let (changed, old) = state.dom.exchange(id, name, value);
    if name == K::Name as NameId {
        state.dom.refresh_name(reflection, id);
    }
    if changed {
        state.dom.touch_rigs_for_property(id, name);
        // (what a kept assembly was worked out from: world/joints.rs stored)
        super::world::joints::stored(state, id, name);
        // (a part's place in its world's spatial grid, filed again at the write: world/broadphase.rs stored)
        super::world::broadphase::stored(state, reflection, id, name);
    }
    if changed {
        state.dom.bump_version_of(id, name);
    }
    (changed, old)
}

/// RawSet: stores a value and fires change notifications when it differs.
pub fn raw_set(
    state: &mut State,
    reflection: &Reflection,
    hooks: &Hooks,
    id: InstanceId,
    name: NameId,
    value: Value,
) -> bool {
    let (changed, old) = store(state, reflection, id, name, value);
    if changed {
        notify_changed(state, reflection, hooks, id, name, Some(old));
    } else {
        // Model:PivotTo writing a part where it is still moves its assembly (world/joints.rs)
        super::world::joints::written_unchanged(state, reflection, id, name);
        for hook in &hooks.property_unchanged {
            hook(state, id, name);
        }
    }
    changed
}

/// Stores a value the engine computed, as physics moves parts in Roblox: no Changed events (scripts
/// see none), but the PropertyChanged hooks run (a recorder sees it). Returns whether it changed.
pub fn raw_set_unsignalled(
    state: &mut State,
    reflection: &Reflection,
    hooks: &Hooks,
    id: InstanceId,
    name: NameId,
    value: Value,
) -> bool {
    let (changed, _) = store(state, reflection, id, name, value);
    if changed {
        state.quiet_write = true;
        for hook in &hooks.property_changed {
            hook(state, id, name);
        }
        state.quiet_write = false;
    }
    changed
}

/// NotifyChanged: Changed / GetPropertyChangedSignal events (measured order: other instances fire
/// Changed(name) before the property signal; ValueBase objects fire the property signal first and
/// Changed(value) only for Value), then the PropertyChanged hooks.
pub fn notify_changed(
    state: &mut State,
    reflection: &Reflection,
    hooks: &Hooks,
    id: InstanceId,
    name: NameId,
    old: Option<Value>,
) {
    notify_changed_events(state, reflection, hooks, id, name, old.as_ref());
    notify_changed_hooks(state, reflection, hooks, id, name, old);
}

/// NotifyChanged's events (Changed and the property signals, with the derived views').
fn notify_changed_events(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, name: NameId, old: Option<&Value>) {
    // An Attachment's CFrame: its own list of Changed events, the views' events left out (derived::AttachmentEvents)
    if let Some(events) = hooks.derived.attachment_events(reflection, state.dom.at(id).class, name) {
        let cframe_of = |value: Option<&Value>| match value {
            Some(Value::Dom(variant)) => match &**variant {
                rbx_dom_weak::types::Variant::CFrame(cframe) => Some(*cframe),
                _ => None,
            },
            _ => None,
        };
        // (Axis and WorldAxis fire only when the first column changed: a SecondaryAxis write that keeps the Axis fires neither,
        // while an Axis write that keeps the SecondaryAxis still fires its two; measured, tools/parity/probes/attachment_axes.luau)
        let column = |cframe: &rbx_dom_weak::types::CFrame, index: usize| {
            let o = &cframe.orientation;
            let rows = [o.x, o.y, o.z];
            rows.map(|row| [row.x, row.y, row.z][index])
        };
        let (rotated, moved, turned) = match (cframe_of(old), cframe_of(state.dom.raw_get(id, name))) {
            (Some(before), Some(after)) => {
                let differs = column(&before, 0).iter().zip(column(&after, 0)).any(|(a, b)| *a != b);
                (before.orientation != after.orientation, before.position != after.position, differs)
            }
            _ => (true, true, true),
        };
        let rotation = if rotated { events.rotation.as_slice() } else { &[] };
        let position = if moved { events.position.as_slice() } else { &[] };
        for &view in rotation.iter().chain(position) {
            if events.axis.contains(&view) && !turned {
                continue;
            }
            fire_changed_only(state, reflection, id, view);
        }
        fire_changed(state, reflection, id, name);
        fire_changed_only(state, reflection, id, events.world_cframe);
        return;
    }
    // Derived views (Font of FontFace, Transparency of BackgroundTransparency, ...): those someone
    // listens to fire when their value changed, before or after the source's events; a part's companion events
    // (derived::Companions: Position, Orientation and Rotation beside CFrame, AssemblyMass beside Size, ...) fire
    // whatever their values
    let companions = hooks.derived.companions_for(reflection, state.dom.at(id).class, name);
    let (before, after) = super::derived::changed_views(state, reflection, hooks, id, name, old, companions.as_deref());
    if let Some(companions) = &companions {
        for &companion in &companions.before {
            fire_changed(state, reflection, id, companion);
        }
    }
    for view in before {
        fire_changed(state, reflection, id, view);
    }
    fire_changed(state, reflection, id, name);
    if let Some(companions) = &companions {
        for &companion in &companions.after {
            fire_changed(state, reflection, id, companion);
        }
    }
    for view in after {
        fire_changed(state, reflection, id, view);
    }
}

/// NotifyChanged's hooks (the PropertyChanged hooks, rigid assemblies).
fn notify_changed_hooks(state: &mut State, reflection: &Reflection, hooks: &Hooks, id: InstanceId, name: NameId, old: Option<Value>) {
    for hook in &hooks.property_changed {
        hook(state, id, name);
    }
    // (CreateCollection's collections: services/collections.rs)
    super::services::collections::property_changed(state, reflection, id, name);
    // Rigid assemblies: a part's CFrame moves its assembly, a joint's change settles it
    super::world::joints::property_changed(state, reflection, hooks, id, name, old.as_ref());
    // (what the write did to the spatial grid's units: world/broadphase.rs settle)
    super::world::broadphase::settle(state, reflection);
}

/// Changed and the property's own signal for a property the engine computes rather than stores (no hooks run): Camera's
/// DiagonalFieldOfView and MaxAxisFieldOfView beside FieldOfView (world/camera_fov.rs).
pub fn fire_computed_changed(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId) {
    fire_changed(state, reflection, id, name);
}

/// Changed alone for `name`, its property signal left out (an Attachment's views: derived::AttachmentEvents).
fn fire_changed_only(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId) {
    let Some(signal) = state.dom.at(id).signals.as_deref().and_then(|signals| signals.event(K::Changed as NameId)) else { return };
    fire_measured(state, signal, vec![Value::String(Str::from(reflection.name(name)))], SignalKind::Changed);
}

fn fire_changed(state: &mut State, reflection: &Reflection, id: InstanceId, name: NameId) {
    let Some(signals) = state.dom.at(id).signals.as_deref() else {
        return;
    };
    let changed = signals.event(K::Changed as NameId);
    let property = signals.property(name);
    let class = state.dom.at(id).class;
    let value_base = reflection.class_id("ValueBase").is_some_and(|base| reflection.class(class).is_a(base));
    if value_base {
        // (a value object's deprecated `changed` is a signal of its own that fires with Changed, the new value too: measured,
        // tools/parity/probes/aliases.luau)
        let changed_alias = reflection.changed_alias.and_then(|alias| signals.event(alias));
        if let Some(signal) = property {
            fire_measured(state, signal, Vec::new(), SignalKind::Property);
        }
        if name == K::Value as NameId {
            for signal in [changed, changed_alias].into_iter().flatten() {
                let value = state.dom.raw_get(id, K::Value as NameId).cloned().unwrap_or(Value::Nil);
                fire_measured(state, signal, vec![value], SignalKind::Changed);
            }
        }
        return;
    }
    if let Some(signal) = changed {
        fire_measured(state, signal, vec![Value::String(Str::from(reflection.name(name)))], SignalKind::Changed);
    }
    if let Some(signal) = property {
        fire_measured(state, signal, Vec::new(), SignalKind::Property);
    }
}

// Hierarchy

/// What a move inside the DataModel from `old_parent` to `new_parent`, keeping `kept` ancestors, costs for the instances it carries,
/// `id`'s subtree, in one walk of it (cost.rs): each instance by its class and depth ([`crate::cost::reparented`]); each part whose
/// world changes, by the worlds it leaves and joins ([`crate::cost::WORLD_WORKSPACE_PART`], [`crate::cost::WORLD_MODEL_PART`]: a
/// part below a WorldRoot of the subtree keeps its world); and each instance again when the move takes the subtree into or out of
/// the Workspace's (`placed`).
fn carried(
    state: &State,
    reflection: &Reflection,
    id: InstanceId,
    old_parent: Option<InstanceId>,
    new_parent: Option<InstanceId>,
    kept: usize,
    placed: bool,
) -> f64 {
    let touched = state.sched.touched.borrow();
    // (a part's world: the parent, or its nearest WorldRoot ancestor, read from the homes the dom keeps)
    let world = |parent: Option<InstanceId>| {
        let (parent, world_root) = (parent?, touched.world_root?);
        std::iter::successors(Some(parent), |&node| state.dom.home_step(node)).find(|&node| reflection.class(state.dom.at(node).class).is_a(world_root))
    };
    let price = |world: Option<InstanceId>| match world {
        None => 0.0,
        Some(world) if Some(world) == touched.workspace => crate::cost::WORLD_WORKSPACE_PART,
        Some(_) => crate::cost::WORLD_MODEL_PART,
    };
    let (before, after) = (world(old_parent), world(new_parent));
    let crossed = if before == after { 0.0 } else { price(before) + price(after) };
    // (each instance, past the 2 kept ancestors its class's price was measured at, pays for the others: the same for every one)
    let each = kept.saturating_sub(2) as f64 * crate::cost::REPARENTED_KEPT;
    let (mut work, mut nested) = (0.0, [0, 0]);
    // (each instance with its depth below the moved one, and whether a WorldRoot of the subtree holds it: the moved one never)
    let mut walk = vec![(id, 0usize, false)];
    while let Some((node, depth, held)) = walk.pop() {
        let instance = state.dom.at(node);
        let part = touched.kind(instance.class) == crate::cost::ColdKind::Part;
        work += touched.reparented(instance.class) + each;
        nested[usize::from(part)] += depth.saturating_sub(1);
        if part && !held {
            work += crossed;
        }
        if placed {
            work += if part { crate::cost::PLACED_PART } else { crate::cost::PLACED_OTHER };
        }
        let below = held || state.dom.is_home(node);
        walk.extend(instance.children.iter().map(|&child| (child, depth + 1, below)));
    }
    work + crate::cost::nested(crate::cost::REPARENTED_NESTED, nested)
}

/// How many instances `id`'s subtree holds, itself included, and the levels they sit below it past the first, summed for its other
/// instances and its parts (cost.rs nested, REPARENTED_LOOSE_NESTED, MOVED_NESTED: a flat subtree has none).
fn subtree_depths(state: &State, id: InstanceId) -> (usize, [usize; 2]) {
    let touched = state.sched.touched.borrow();
    let (mut count, mut nested) = (0, [0, 0]);
    let mut walk = vec![(id, 0usize)];
    while let Some((node, depth)) = walk.pop() {
        let instance = state.dom.at(node);
        count += 1;
        nested[usize::from(touched.kind(instance.class) == crate::cost::ColdKind::Part)] += depth.saturating_sub(1);
        let child_depth = depth + 1;
        for &child in instance.children.iter() {
            let child_inst = state.dom.at(child);
            if child_inst.children.is_empty() {
                count += 1;
                nested[usize::from(touched.kind(child_inst.class) == crate::cost::ColdKind::Part)] += depth;
            } else {
                walk.push((child, child_depth));
            }
        }
    }
    (count, nested)
}

/// `from` and its ancestors, nearest first.
fn ancestry(state: &State, from: Option<InstanceId>) -> Vec<InstanceId> {
    let mut chain = Vec::new();
    let mut current = from;
    while let Some(node) = current {
        chain.push(node);
        current = state.dom.at(node).parent;
    }
    chain
}

/// FireDescendantEvents (DescendantAdded, DescendantRemoving), in the order a Studio client and the edit DataModel
/// both fire them (an arrival probe, 2026-10-23): for each instance of `root`'s subtree in
/// GetDescendants' order (the root first, depth first), every ancestor in `ancestors` (nearest first) that has the event.
/// An ancestor the subtree stays under (a move inside one folder) fires nothing: the caller leaves it out.
fn fire_descendant_events(state: &mut State, ancestors: &[InstanceId], root: InstanceId, event: NameId) {
    let listeners: Vec<SignalId> = ancestors.iter().filter_map(|&node| existing_event(state, node, event)).collect();
    if listeners.is_empty() {
        return;
    }
    let mut nodes = vec![root];
    nodes.extend(state.dom.descendants(root));
    let kind = if event == K::DescendantAdded as NameId {
        SignalKind::DescendantAdded
    } else if event == K::DescendantRemoving as NameId {
        SignalKind::DescendantRemoving
    } else {
        SignalKind::Unmeasured
    };
    for node in nodes {
        for &signal in &listeners {
            fire_measured(state, signal, vec![Value::Instance(node)], kind);
        }
    }
}

/// [`fire_descendant_events`] of a player leaving Players (services/players.rs): on a server the character's step, then the
/// PlayerGui's destroy, come after the nearest ancestor's (Players') DescendantRemoving of the player and before the rest, in
/// GetDescendants' order as ever; on a client the character's step comes before them all.
fn leaving_descendant_events(state: &mut State, reflection: &Reflection, hooks: &Hooks, ancestors: &[InstanceId], player: InstanceId) {
    let event = K::DescendantRemoving as NameId;
    if !super::services::players::on_server(state) || ancestors.is_empty() {
        super::services::players::remove_character(state, reflection, hooks, player);
        super::services::players::destroy_player_gui(state, reflection, hooks, player);
        fire_descendant_events(state, ancestors, player, event);
        return;
    }
    if let Some(signal) = existing_event(state, ancestors[0], event) {
        fire_measured(state, signal, vec![Value::Instance(player)], SignalKind::DescendantRemoving);
    }
    super::services::players::remove_character(state, reflection, hooks, player);
    super::services::players::destroy_player_gui(state, reflection, hooks, player);
    let listeners: Vec<(usize, SignalId)> =
        ancestors.iter().enumerate().filter_map(|(index, &node)| existing_event(state, node, event).map(|signal| (index, signal))).collect();
    if listeners.is_empty() {
        return;
    }
    let mut nodes = vec![player];
    nodes.extend(state.dom.descendants(player));
    for (position, node) in nodes.into_iter().enumerate() {
        for &(index, signal) in &listeners {
            if position == 0 && index == 0 {
                continue;
            }
            fire_measured(state, signal, vec![Value::Instance(node)], SignalKind::DescendantRemoving);
        }
    }
}

/// AncestryChanged of every instance of the moved subtree, children before the instance (post-order, measured in Studio
/// with the others above: A1, A2, A, B1, B, C, U for U(A(A1, A2), B(B1), C)).
/// The subtree's AncestryChanged events, in its post-order; a script among them that came to run starts or one that left stops
/// right after its own (a 0.747 client and server queue a start there: scripts.rs). `old_chain` is the old parent's chain.
fn fire_ancestry_changed(state: &mut State, root: InstanceId, new_parent: Option<InstanceId>, old_chain: &[InstanceId]) {
    let scripts = crate::scripts::watching_move(state, root);
    if state.ancestry_signals == 0 && !scripts {
        return;
    }
    let parent = new_parent.map_or(Value::Nil, Value::Instance);
    for node in state.dom.subtree_post_order(root) {
        if state.ancestry_signals > 0
            && let Some(signal) = existing_event(state, node, K::AncestryChanged as NameId)
        {
            fire_measured(state, signal, vec![Value::Instance(root), parent.clone()], SignalKind::AncestryChanged);
        }
        if scripts {
            crate::scripts::moved(state, node, root, old_chain);
        }
    }
}

/// How an instance's Parent is locked, where it is (set_parent; the class's part is [`ParentLock`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lock {
    /// "Cannot change Parent of type X", every write
    Class,
    /// "The Parent property of X is locked, current parent: P, new parent Y", once the parent it has passed as no change
    Instance,
    /// A settings root's capability error, once the parent it has passed and "its own parent" was looked at
    SettingsRoot,
}

/// An instance's full name for an error that names it, at what a client spends building it (cost.rs full_name).
fn named(state: &State, reflection: &Reflection, id: InstanceId) -> String {
    let (name, names) = state.dom.full_name_counted(reflection, id);
    let work = crate::cost::full_name(names, name.len());
    state.sched.charge(work);
    crate::boundary::add_cost(work);
    name
}

fn parent_lock(state: &State, reflection: &Reflection, id: InstanceId) -> Option<Lock> {
    let instance = state.dom.at(id);
    let class = reflection.class(instance.class);
    let locked = instance.has(flags::PARENT_LOCKED);
    // Terrain, a player's PlayerGui and PlayerScripts, StarterPlayer's script folders: locked once the engine has put one in
    // place, or Destroy locked it (a destroyed PlayerGui keeps its class's lock, measured). A Studio-only placeholder is not
    // in place: the place's own instance takes its place (project.rs)
    if class.parent_lock == ParentLock::Class && (locked || (instance.parent.is_some() && !instance.has(flags::STUDIO_ONLY))) {
        return Some(Lock::Class);
    }
    if !locked {
        return (instance.parent.is_none() && Some(instance.class) == reflection.class_id("DataModel")).then_some(Lock::Instance);
    }
    if is_service(state, reflection, id) {
        return Some(if class.parent_lock == ParentLock::Service { Lock::Class } else { Lock::Instance });
    }
    // a settings root (settings(), UserSettings(): services/settings.rs) refuses its own way, measured
    if instance.parent.is_none() && super::services::settings::is_root(reflection, instance.class) {
        return Some(Lock::SettingsRoot);
    }
    Some(Lock::Instance)
}

/// InstanceModule.SetParent.
pub fn set_parent(
    state: &mut State,
    reflection: &Reflection,
    hooks: &Hooks,
    id: InstanceId,
    new_parent: Option<InstanceId>,
) -> ScriptResult<()> {
    // A Parent write meets three checks in turn (measured in a playing client and a server, Studio 0.747, every service a game
    // script gets and every locked instance the engine makes: tools/parity/probes/parent_locks.luau; a 0.748 client reads the
    // same). What is locked by class refuses every write, the parent it has and itself too: "Cannot change Parent of type
    // Lighting" (112 services, the settings services, UserGameSettings, Terrain, a player's PlayerGui and PlayerScripts,
    // StarterPlayer's script folders: reflection.rs ParentLock). Then the parent it has is no change and raises nothing. Then
    // what is locked as an instance refuses, naming its parent: "The Parent property of Workspace is locked, current parent:
    // <the place's name>, new parent NULL" (the DataModel, 29 services, a destroyed instance, the PlayerMouse); a settings
    // root raises its capability error there, but made its own parent "Attempt to set GlobalBasicSettings as its own parent".
    // An unlocked instance (the Camera, the Player, its Backpack and character) made its own parent raises "Attempt to set X
    // as its own parent", and set below its descendant the circular-reference text.
    let lock = parent_lock(state, reflection, id);
    if lock == Some(Lock::Class) {
        return Err(ScriptError::plain(format!("Cannot change Parent of type {}", reflection.class(state.dom.at(id).class).name)));
    }
    if state.dom.at(id).parent == new_parent {
        return Ok(());
    }
    let name_of = |parent: Option<InstanceId>| parent.map_or_else(|| "NULL".to_string(), |parent| state.dom.at(parent).name.to_string());
    match lock {
        Some(Lock::SettingsRoot) if new_parent == Some(id) => {
            return Err(ScriptError::plain(format!("Attempt to set {} as its own parent", named(state, reflection, id))));
        }
        Some(Lock::SettingsRoot) => {
            let label = super::services::settings::parent_label(reflection, state.dom.at(id).class);
            return Err(ScriptError::plain(format!("The current thread cannot set '{label} parent' (lacking capability RobloxScript)")));
        }
        Some(Lock::Instance) => {
            return Err(ScriptError::plain(format!(
                "The Parent property of {} is locked, current parent: {}, new parent {}",
                state.dom.at(id).name,
                name_of(state.dom.at(id).parent),
                name_of(new_parent)
            )));
        }
        _ => {}
    }
    if let Some(parent) = new_parent
        && let Some(message) = parent_rule(reflection, state.dom.at(id).class, state.dom.at(parent).class)
    {
        return Err(ScriptError::plain(message));
    }
    // (a service Instance.new made goes under no instance: "Invalid parent for Service. TestService cannot be parented to Workspace .",
    // measured, roblox-sim 0.747, a client and a server, tools/parity/probes/instance_new_refusals.luau)
    if let Some(parent) = new_parent
        && reflection.class(state.dom.at(id).class).service
        && Some(state.dom.at(parent).class) != reflection.class_id("DataModel")
    {
        let class = &reflection.class(state.dom.at(id).class).name;
        return Err(ScriptError::plain(format!("Invalid parent for Service. {class} cannot be parented to {} .", state.dom.at(parent).name)));
    }
    reparent(state, reflection, hooks, id, new_parent)
}

/// The ancestors among `leaving` (those a move takes `id` away from) whose PrimaryPart is `id` or one of its descendants, with
/// PrimaryPart's name: none when no ancestor is left behind.
fn released_primaries(state: &State, reflection: &Reflection, leaving: &[InstanceId], id: InstanceId) -> Option<(NameId, Vec<InstanceId>)> {
    if leaving.is_empty() {
        return None;
    }
    let primary = reflection.name_id("PrimaryPart")?;
    let models: Vec<InstanceId> = leaving
        .iter()
        .copied()
        .filter(|&ancestor| match state.dom.raw_get(ancestor, primary) {
            Some(Value::Instance(part)) => *part == id || state.dom.is_descendant_of(*part, id),
            _ => false,
        })
        .collect();
    (!models.is_empty()).then_some((primary, models))
}

/// The parent some classes require, as a playing client and a server refuse another (a Parent write and Instance.new's parent
/// alike; nil and a parent's own move are fine): an AdPortal goes under a Part (no other BasePart), an Animator under a Humanoid or
/// an AnimationController, a Bone under a BasePart or a Bone, a WrapLayer or WrapTarget under a MeshPart (measured, Studio 0.747:
/// tools/parity/probes/instance_api_texts.luau).
fn parent_rule(reflection: &Reflection, class: ClassId, parent: ClassId) -> Option<&'static str> {
    let is = |name: &str| reflection.is_a(parent, name);
    // (a TextChannelWindow takes scripts and three UI objects only, a client and a server alike: measured, roblox-sim 0.747,
    // tools/parity/probes/parent_rules.luau)
    if &*reflection.class(parent).name == "TextChannelWindow"
        && !["Script", "LocalScript", "ModuleScript", "UICorner", "UIAspectRatioConstraint", "UISizeConstraint"].iter().any(|name| reflection.is_a(class, name))
    {
        return Some("TextChannelWindow only accepts Script, LocalScript, ModuleScript, UICorner, UIAspectRatioConstraint, or UISizeConstraint as children");
    }
    match &*reflection.class(class).name {
        "AdPortal" if &*reflection.class(parent).name != "Part" => Some("AdPortal must be parented to a Part."),
        "Animator" if !is("Humanoid") && !is("AnimationController") => Some("Animator has to be placed under Humanoid or AnimationController!"),
        "Bone" if !is("BasePart") && !is("Bone") => Some("Bones must be parented to a BasePart or another Bone."),
        "WrapLayer" | "WrapTarget" if !is("MeshPart") => Some("BaseWrap must be parented to a MeshPart."),
        _ => None,
    }
}

/// set_parent past its Parent locks: the new parent's own checks, then the move and its events.
fn reparent(
    state: &mut State,
    reflection: &Reflection,
    hooks: &Hooks,
    id: InstanceId,
    new_parent: Option<InstanceId>,
) -> ScriptResult<()> {
    let old_parent = state.dom.at(id).parent;
    if old_parent == new_parent {
        return Ok(());
    }
    if let Some(parent) = new_parent {
        // nothing goes under a settings root or one of its services (services/settings.rs; measured: "Not allowed to add that
        // under settings", for settings() and UserSettings() alike)
        let top = match state.dom.at(parent).parent {
            None => Some(parent),
            Some(up) if state.dom.at(up).parent.is_none() => Some(up),
            Some(_) => None,
        };
        if top.is_some_and(|top| super::services::settings::is_root(reflection, state.dom.at(top).class)) {
            return Err(ScriptError::plain("Not allowed to add that under settings"));
        }
        if parent == id {
            return Err(ScriptError::plain(format!("Attempt to set {} as its own parent", named(state, reflection, id))));
        }
        if state.dom.is_descendant_of(parent, id) {
            return Err(ScriptError::plain(format!(
                "Attempt to set parent of {} to {} would result in circular reference",
                named(state, reflection, id),
                named(state, reflection, parent)
            )));
        }
    }
    // Lazily imported instances: a moved subtree is made whole first (its descendants fire their
    // events, and what references them within their model exists), and a new parent's own children
    // come before the new child
    if state.dom.at(id).flags & (flags::LAZY_REGION | flags::LAZY_ABOVE) != 0 {
        super::project::materialize_region(state, reflection, id);
    }
    if let Some(parent) = new_parent.filter(|&parent| state.dom.at(parent).has(flags::LAZY_PENDING)) {
        super::project::materialize_lazy(state, reflection, parent);
    }
    // The two ancestor chains, nearest first, each up to its root, and how many ancestors they share: chains of one tree meet and go
    // on together to the root, so what they share is a common tail (an ancestor the subtree keeps is told nothing:
    // Root.DescendantAdded / DescendantRemoving stay quiet when a model moves from Root.Units to Root.Other)
    let old_chain = ancestry(state, old_parent);
    let new_chain = ancestry(state, new_parent);
    let kept = old_chain.iter().rev().zip(new_chain.iter().rev()).take_while(|(old, new)| old == new).count();
    let in_data_model = |chain: &[InstanceId]| state.collection.game.is_some_and(|game| chain.last() == Some(&game));
    let (inside_before, inside_after) = (in_data_model(&old_chain), in_data_model(&new_chain));
    // CollectionService: tagged instances entering or leaving the DataModel (services/mod.rs
    // CollectionState; Studio fires the removed signals with DescendantRemoving, before ChildRemoved
    // and AncestryChanged, and the added ones with DescendantAdded)
    let (was_inside, now_inside) = if state.collection.tagged { (inside_before, inside_after) } else { (false, false) };
    // (entering the Workspace: the frame's first entry runs the engine's code for it cold, the first part's more (cost.rs Path::Enter,
    // EnterPart); each instance's entry is noted, so a destroy knows the engine took it into its frame work: destroy_live)
    let workspace = state.sched.touched.borrow().workspace;
    let placed = |state: &State, parent: Option<InstanceId>| {
        workspace.is_some_and(|workspace| parent.is_some_and(|parent| parent == workspace || state.dom.is_descendant_of_home(parent, workspace)))
    };
    let (was_placed, now_placed) = (placed(state, old_parent), placed(state, new_parent));
    // the work clock (crate::cost): the chains' walks, and Studio's cost of a subtree entering or leaving the DataModel, or of what a
    // move inside it or outside it carries
    let chains = crate::cost::reparent_chains(old_chain.len(), new_chain.len(), kept);
    let work = if inside_before != inside_after {
        crate::cost::MOVE_BASE
            + std::iter::once(id).chain(state.dom.descendants(id)).map(|node| crate::cost::moved(reflection, state.dom.at(node).class)).sum::<f64>()
            + crate::cost::nested(crate::cost::MOVED_NESTED, subtree_depths(state, id).1)
    } else if inside_after {
        crate::cost::REPARENTED + carried(state, reflection, id, old_parent, new_parent, kept, was_placed != now_placed)
    } else {
        let (count, nested) = subtree_depths(state, id);
        crate::cost::REPARENTED_LOOSE + count as f64 * crate::cost::REPARENTED_LOOSE_INSTANCE + crate::cost::nested(crate::cost::REPARENTED_LOOSE_NESTED, nested)
    } + chains
        // (parts taken into or out of the Workspace, for how deep they sit below the moved instance: cost.rs PLACED_NESTED_PART)
        + if was_placed != now_placed { subtree_depths(state, id).1[1] as f64 * crate::cost::PLACED_NESTED_PART[usize::from(was_placed)] } else { 0.0 };
    state.sched.charge(work);
    crate::boundary::add_cost(work);
    if was_placed != now_placed {
        let mut touched = state.sched.touched.borrow_mut();
        let mut first = 0.0;
        let mut part = false;
        for node in std::iter::once(id).chain(state.dom.descendants(id)) {
            touched.set_entered(node.index, now_placed);
            touched.count(|ops| ops.entered += u64::from(now_placed));
            part = part || touched.kind(state.dom.at(node).class) == crate::cost::ColdKind::Part;
        }
        if now_placed {
            first += touched.first_path(crate::cost::Path::Enter);
            if part {
                first += touched.first_path(crate::cost::Path::EnterPart);
            }
        }
        drop(touched);
        if first > 0.0 {
            state.sched.charge(first);
            crate::boundary::add_cost(first);
        }
    }
    if let Some(old) = old_parent {
        // (a player leaving Players: its character's step and PlayerRemoving come in among its removal events, where a server
        // and a client put them: services/players.rs)
        let leaving = super::services::players::leaving(state, id, old);
        if leaving {
            leaving_descendant_events(state, reflection, hooks, &old_chain[..old_chain.len() - kept], id);
        } else {
            fire_descendant_events(state, &old_chain[..old_chain.len() - kept], id, K::DescendantRemoving as NameId);
        }
        if was_inside && !now_inside {
            super::services::subtree_moved(state, id, false);
        }
        if leaving {
            super::services::players::player_removing(state, id, old);
        }
    }
    // A Model whose PrimaryPart leaves it lets go of it (measured in a 0.747 playing client and a server:
    // tools/parity/probes/engine_data_methods.luau): an ancestor the move leaves whose PrimaryPart is the moved instance or under it
    // reads nil from the move on (its Changed fires), destroyed or taken out alike; a part moved deeper inside, or the whole model
    // moved, keeps it, and a PrimaryPart set to a part outside the model stays until that part leaves it after joining
    let released = released_primaries(state, reflection, &old_chain[..old_chain.len() - kept], id);
    // (out of the old parent and under the new one, its home in the tree worked out once: dom.rs reparent; a client looks for the
    // child from the start of its old parent's list, in the order it keeps them in: cost.rs CHILD_SEARCHED)
    let searched = state.dom.reparent(id, new_parent);
    if searched > 0 {
        let work = searched as f64 * crate::cost::CHILD_SEARCHED;
        state.sched.charge(work);
        crate::boundary::add_cost(work);
    }
    notify_changed_hooks(state, reflection, hooks, id, K::Parent as NameId, None);
    if let Some(old) = old_parent {
        fire_event_measured(state, old, K::ChildRemoved as NameId, vec![Value::Instance(id)], SignalKind::ChildRemoved);
    }
    if let Some(parent) = new_parent {
        // (Studio: every DescendantAdded of the subtree, then any before-child-added setup such as Players.PlayerAdded, then ChildAdded)
        fire_descendant_events(state, &new_chain[..new_chain.len() - kept], id, K::DescendantAdded as NameId);
        // (CollectionService's added signal for a tagged subtree entering the DataModel comes with them, before ChildAdded: measured,
        // roblox-sim 0.747, tools/parity/probes/collection_updates.luau)
        if now_inside && !was_inside {
            super::services::subtree_moved(state, id, true);
        }
        for hook in &hooks.before_child_added {
            hook(state, hooks, id, old_parent, parent);
        }
        if let Some(signal) = existing_event(state, parent, K::ChildAdded as NameId) {
            fire_measured(state, signal, vec![Value::Instance(id)], SignalKind::ChildAdded);
        }
        // (the dump's deprecated childAdded is a signal of its own that fires with ChildAdded: measured,
        // tools/parity/probes/aliases.luau)
        if let Some(alias) = reflection.child_added_alias
            && let Some(signal) = existing_event(state, parent, alias)
        {
            fire_measured(state, signal, vec![Value::Instance(id)], SignalKind::ChildAdded);
        }
    }
    fire_ancestry_changed(state, id, new_parent, &old_chain);
    // (the moved instance's own Changed("Parent") and Parent property signal come last: measured with the others)
    notify_changed_events(state, reflection, hooks, id, K::Parent as NameId, None);
    if let Some((primary, models)) = released {
        for model in models {
            raw_set(state, reflection, hooks, model, primary, Value::Nil);
        }
    }
    for hook in &hooks.hierarchy_changed {
        hook(state, id, old_parent, new_parent);
    }
    // Joints and parts entering a WorldRoot link up, and those leaving every world leave their parts' lists of active joints
    // (world::joints)
    super::world::joints::reparented(state, reflection, hooks, id, old_parent, new_parent);
    super::world::broadphase::settle(state, reflection);
    Ok(())
}

// Clone

fn archivable(state: &State, id: InstanceId) -> bool {
    !matches!(state.dom.raw_get(id, K::Archivable as NameId), Some(Value::Bool(false)))
}

/// InstanceModule.Clone (after the caller materialized the subtree): copies properties (except
/// Parent), attributes, tags, sources and markers; instance-valued properties pointing inside
/// the copied tree are remapped to the copies. `None` when the root is not Archivable.
pub fn clone_instance(state: &mut State, reflection: &Reflection, root: InstanceId) -> Option<InstanceId> {
    clone_instance_mapped(state, reflection, root).map(|(copy, _)| copy)
}

/// [`clone_instance`], with each instance copied paired with its copy (original, copy), the root's first.
pub fn clone_instance_mapped(state: &mut State, reflection: &Reflection, root: InstanceId) -> Option<(InstanceId, Vec<(InstanceId, InstanceId)>)> {
    if !archivable(state, root) {
        return None;
    }
    let mut map: Vec<(InstanceId, InstanceId)> = Vec::new();
    let result = copy_tree(state, reflection, root, &mut map)?;
    let copies: rustc_hash::FxHashMap<InstanceId, InstanceId> = map.iter().copied().collect();
    for &(_, copy) in &map {
        if !state.dom.at(copy).has(flags::HOLDS_REFS) {
            continue;
        }
        let remapped: Vec<(usize, InstanceId)> = state
            .dom
            .at(copy)
            .props
            .iter()
            .enumerate()
            .filter_map(|(index, (_, value))| copies.get(&value.as_instance()?).map(|c| (index, *c)))
            .collect();
        if remapped.is_empty() {
            continue;
        }
        let props = state.dom.at_mut(copy).props.own_mut();
        for (index, target) in remapped {
            props[index].1 = Value::Instance(target);
        }
    }
    // an attribute holding an instance of the copied tree holds its copy, one outside it keeps its target (measured, Studio 0.747,
    // a client and a server: tools/parity/probes/datatype_edges.luau `clone-attribute`); a lazily imported plan node's attributes
    // come from a file and hold no instance
    for &(_, copy) in &map {
        let crate::dom::Attributes::Own(attributes) = &state.dom.at(copy).attributes else { continue };
        let remapped: Vec<(usize, InstanceId)> =
            attributes.iter().enumerate().filter_map(|(index, (_, value))| copies.get(&value.as_instance()?).map(|c| (index, *c))).collect();
        if remapped.is_empty() {
            continue;
        }
        let attributes = state.dom.at_mut(copy).attributes.own_mut();
        for (index, target) in remapped {
            attributes[index].1 = Value::Instance(target);
        }
    }
    for &(_, copy) in &map {
        let tags = state.dom.at(copy).tags.clone();
        for tag in tags {
            super::services::tag_added(state, copy, &tag);
        }
    }
    Some((result, map))
}

/// Copies `original`'s subtree, its Archivable instances, each before its children, each copy attached to its parent's copy
/// once the copy's own subtree is made. A stack of steps in place of recursion: a script can build a chain deeper than any
/// thread's stack holds frames for.
fn copy_tree(
    state: &mut State,
    reflection: &Reflection,
    original: InstanceId,
    map: &mut Vec<(InstanceId, InstanceId)>,
) -> Option<InstanceId> {
    enum Step {
        /// Copy the instance, then its subtree under the copy of the second (none for the root)
        Copy(InstanceId, Option<InstanceId>),
        /// Attach the copy to its parent's copy, its subtree made
        Attach(InstanceId, InstanceId),
    }
    let mut root = None;
    let mut steps = vec![Step::Copy(original, None)];
    while let Some(step) = steps.pop() {
        match step {
            Step::Attach(copy, parent) => state.dom.attach(copy, parent),
            Step::Copy(original, parent) => {
                let Some((copy, children)) = copy_one(state, reflection, original, map) else { continue };
                match parent {
                    Some(parent) => steps.push(Step::Attach(copy, parent)),
                    None => root = Some(copy),
                }
                steps.extend(children.into_iter().rev().map(|child| Step::Copy(child, Some(copy))));
            }
        }
    }
    root
}

/// [`copy_tree`]'s copy of one instance, unparented, with the children of `original` to copy below it; None when it is not
/// Archivable.
fn copy_one(
    state: &mut State,
    reflection: &Reflection,
    original: InstanceId,
    map: &mut Vec<(InstanceId, InstanceId)>,
) -> Option<(InstanceId, Vec<InstanceId>)> {
    if !archivable(state, original) {
        return None;
    }
    let source = state.dom.at(original);
    let class = source.class;
    let props = source.props.copied_without(K::Parent as NameId);
    let attributes = source.attributes.copied();
    let tags = source.tags.clone();
    let script = source.script.clone();
    let copied_flags = source.flags
        & (flags::SCRIPT_CREATED | flags::STUDIO_ONLY | flags::MATERIALIZED | flags::JSON_MODULE | flags::HOLDS_REFS);
    let studio_node = source.studio_node;
    let callbacks = source.callbacks.clone();
    let unmodeled = source.unmodeled.clone();
    let children = source.children.to_vec();

    let copy = create(state, reflection, class);
    {
        let target = state.dom.at_mut(copy);
        target.props = props;
        target.attributes = attributes;
        target.tags = tags;
        target.script = script;
        target.flags |= copied_flags;
        target.studio_node = studio_node;
        target.callbacks = callbacks;
        target.unmodeled = unmodeled;
    }
    state.dom.refresh_name(reflection, copy);
    map.push((original, copy));
    Some((copy, children))
}

/// An instance tree kept as data: what [`clone_instance`] would copy of it now, without making an instance (a clip
/// as it was when registered: animation/source.rs). Descendants that are not Archivable are left out, as a clone
/// leaves them out; a property that refers to an instance keeps referring to it; scripts' sources and callbacks are
/// not kept.
pub struct KeptTree {
    nodes: Vec<KeptNode>,
}

pub struct KeptNode {
    pub class: ClassId,
    pub name: crate::value::Str,
    pub props: crate::dom::Props,
    /// The node's children, as indexes into the tree
    pub children: Vec<u32>,
    parent: Option<u32>,
    attributes: crate::dom::Attributes,
    tags: crate::dom::ThinVec<crate::value::Str>,
    flags: u16,
    studio_node: crate::dom::StudioNode,
    unmodeled: Option<crate::project::Unmodeled>,
}

impl KeptTree {
    /// `root` and what is below it, as it is now; None when it is not Archivable.
    pub fn of(state: &State, root: InstanceId) -> Option<KeptTree> {
        if !archivable(state, root) {
            return None;
        }
        let mut nodes: Vec<KeptNode> = Vec::new();
        let mut stack: Vec<(InstanceId, Option<u32>)> = vec![(root, None)];
        while let Some((id, parent)) = stack.pop() {
            let source = state.dom.at(id);
            let index = nodes.len() as u32;
            if let Some(parent) = parent {
                nodes[parent as usize].children.push(index);
            }
            nodes.push(KeptNode {
                class: source.class,
                name: source.name.clone(),
                props: source.props.copied_without(K::Parent as NameId),
                children: Vec::new(),
                parent,
                attributes: source.attributes.copied(),
                tags: source.tags.clone(),
                flags: source.flags & (flags::SCRIPT_CREATED | flags::STUDIO_ONLY | flags::MATERIALIZED | flags::JSON_MODULE | flags::HOLDS_REFS),
                studio_node: source.studio_node,
                unmodeled: source.unmodeled.clone(),
            });
            stack.extend(source.children.iter().rev().filter(|&&child| archivable(state, child)).map(|&child| (child, Some(index))));
        }
        Some(KeptTree { nodes })
    }

    pub fn nodes(&self) -> &[KeptNode] {
        &self.nodes
    }

    /// The tree as new instances, unparented: its root. `bare`: the root without its Name, attributes and tags.
    pub fn make(&self, state: &mut State, reflection: &Reflection, bare: bool) -> InstanceId {
        let mut made: Vec<InstanceId> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let copy = create(state, reflection, node.class);
            let bare = bare && made.is_empty();
            {
                let target = state.dom.at_mut(copy);
                target.props = node.props.copied_without(if bare { K::Name as NameId } else { K::Parent as NameId });
                if !bare {
                    target.attributes = node.attributes.copied();
                    target.tags = node.tags.clone();
                }
                target.flags |= node.flags;
                target.studio_node = node.studio_node;
                target.unmodeled = node.unmodeled.clone();
            }
            state.dom.refresh_name(reflection, copy);
            if let Some(parent) = node.parent {
                state.dom.attach(copy, made[parent as usize]);
            }
            made.push(copy);
        }
        for (node, &copy) in self.nodes.iter().zip(&made).skip(bare as usize) {
            for tag in node.tags.clone() {
                super::services::tag_added(state, copy, &tag);
            }
        }
        made[0]
    }
}

// Attributes and tags

/// SetAttribute after type validation: stores (or removes) and fires AttributeChanged plus the
/// attribute's own signal when the value changed.
pub fn set_attribute(state: &mut State, reflection: &Reflection, id: InstanceId, name: Str, value: Value) {
    let instance = state.dom.at_mut(id);
    // (an Instance attribute keeps its instance as an instance property does: lua::raw's collector walks both)
    if value.references_instance() {
        instance.flags |= flags::HOLDS_REFS;
    }
    let index = instance.attributes.iter().position(|(n, _)| *n == name);
    match (index, value.is_nil()) {
        (Some(index), _) if instance.attributes.as_slice()[index].1.same(&value) => return,
        (None, true) => return,
        (Some(index), true) => {
            instance.attributes.own_mut().remove(index);
        }
        (Some(index), false) => instance.attributes.own_mut()[index].1 = value,
        (None, false) => instance.attributes.own_mut().push((name.clone(), value)),
    }
    instance.version = instance.version.wrapping_add(1);
    let _ = reflection;
    if let Some(signal) = existing_event(state, id, K::AttributeChanged as NameId) {
        fire_measured(state, signal, vec![Value::String(name.clone())], SignalKind::Attribute);
    }
    if let Some(signal) = state.dom.at(id).signals.as_ref().and_then(|s| s.attribute(name.as_bytes())) {
        fire_measured(state, signal, Vec::new(), SignalKind::Attribute);
    }
}

pub fn add_tag(state: &mut State, id: InstanceId, tag: Str) {
    if state.dom.at(id).tags.contains(&tag) {
        return;
    }
    state.dom.at_mut(id).tags.push(tag.clone());
    state.dom.bump_version(id);
    super::services::tag_added(state, id, &tag);
    super::services::collections::tag_changed(state, id);
    log_tag(state, id, &tag);
}

/// A loopback run's server logs a replicated instance's tag change (loopback/replicate.rs)
fn log_tag(state: &mut State, id: InstanceId, tag: &Str) {
    if state.dom.at(id).flags & flags::REPLICATED != 0
        && let Some(log) = state.loopback_log.as_mut()
    {
        log.push(crate::loopback::replicate::Change::Tag(id, tag.clone()));
    }
}

pub fn remove_tag(state: &mut State, id: InstanceId, tag: &Str) {
    let tags = &mut state.dom.at_mut(id).tags;
    let Some(index) = tags.iter().position(|t| t == tag) else {
        return;
    };
    tags.remove(index);
    state.dom.bump_version(id);
    super::services::tag_removed(state, id, tag);
    super::services::collections::tag_changed(state, id);
    log_tag(state, id, tag);
}

/// A service: a parent-locked instance of a service class under a service provider (the DataModel, a settings root).
fn is_service(state: &State, reflection: &Reflection, id: InstanceId) -> bool {
    let instance = state.dom.at(id);
    instance.has(flags::PARENT_LOCKED)
        && reflection.class(instance.class).service
        && instance.parent.is_some_and(|parent| {
            let class = state.dom.at(parent).class;
            Some(class) == reflection.class_id("DataModel") || super::services::settings::is_root(reflection, class)
        })
}

/// The first half of Destroy (after its Destroying handlers were queued): marks the instance destroyed,
/// unparents it and locks its parent. Returns the children to destroy next.
pub fn begin_destroy(
    state: &mut State,
    reflection: &Reflection,
    hooks: &Hooks,
    id: InstanceId,
) -> ScriptResult<Vec<InstanceId>> {
    // a service stays as it was: "The Parent property of Lighting is locked" (measured, tools/parity/probes/hierarchy.luau); a
    // settings root too, naming its first service (settings(): "The Parent property of Game Options is locked", measured)
    if state.dom.at(id).has(flags::PARENT_LOCKED) {
        if is_service(state, reflection, id) {
            return Err(ScriptError::plain(format!("The Parent property of {} is locked", state.dom.at(id).name)));
        }
        let instance = state.dom.at(id);
        if instance.parent.is_none()
            && let Some(&first) = instance.children.first()
            && super::services::settings::is_root(reflection, instance.class)
        {
            return Err(ScriptError::plain(format!("The Parent property of {} is locked", state.dom.at(first).name)));
        }
    }
    // what is locked by class but no service (measured in a 0.747 client and server, tools/parity/probes/parent_locks.luau):
    // Terrain refuses ("Cannot Destroy() Terrain"), StarterPlayer's script folders refuse naming their full name ("The Parent
    // property of StarterPlayer.StarterPlayerScripts is locked"), and a player's PlayerGui and PlayerScripts are destroyed,
    // their class's lock kept
    if parent_lock(state, reflection, id) == Some(Lock::Class) {
        match &*reflection.class(state.dom.at(id).class).name {
            "Terrain" => return Err(ScriptError::plain("Cannot Destroy() Terrain")),
            "StarterPlayerScripts" | "StarterCharacterScripts" => {
                return Err(ScriptError::plain(format!("The Parent property of {} is locked", named(state, reflection, id))));
            }
            _ => {}
        }
    }
    state.dom.at_mut(id).set_flag(flags::DESTROYED, true);
    if state.dom.at(id).parent.is_some() {
        reparent(state, reflection, hooks, id, None)?;
    }
    // A destroyed Model lets go of its PrimaryPart (measured in Studio, 5 of 5; unparenting keeps it): the
    // part it named stays collectable once the Model is the last thing that referenced it
    if reflection.is_a(state.dom.at(id).class, "Model")
        && let Some(primary) = reflection.name_id("PrimaryPart")
        && matches!(state.dom.raw_get(id, primary), Some(Value::Instance(_)))
    {
        raw_set(state, reflection, hooks, id, primary, Value::Nil);
    }
    state.dom.at_mut(id).set_flag(flags::PARENT_LOCKED, true);
    Ok(state.dom.at(id).children.to_vec())
}

/// The rest of Destroy: signals disconnect at the next resumption point. (The instance keeps its
/// tags, as in Studio: CollectionService's removed signals fired when it left the DataModel.)
pub fn finish_destroy(state: &mut State, id: InstanceId) {
    state.sched.defer(JobTarget::DisconnectAll(id), Vec::new());
    super::services::collections::destroyed(state, id);
}

/// Disconnects every signal of an instance (Destroy's deferred cleanup).
pub fn disconnect_all(state: &mut State, id: InstanceId) {
    let Some(signals) = state.dom.at(id).signals.as_deref() else {
        return;
    };
    let ids: Vec<SignalId> = signals.all().collect();
    for signal in ids {
        state.signals.disconnect_all(signal);
    }
}

/// The signals created for an instance (for invalidation-aware ports).
pub fn instance_signals(state: &State, id: InstanceId) -> Option<&InstanceSignals> {
    state.dom.at(id).signals.as_deref()
}
