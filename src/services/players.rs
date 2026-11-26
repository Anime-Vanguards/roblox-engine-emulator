//! A player leaving: what becomes of its character and of the Player when it leaves Players, whatever made it leave (its
//! client left, Kick, a script setting its Parent or destroying it, a loopback run's Context.Leave).
//!
//! Measured in roblox-sim's 0.747.0.7471481 play sessions of one server and four clients (Workspace.SignalBehavior Deferred,
//! script identity 2): a server script and one client watched the three other players leave in each session, by Kick,
//! Destroy, a Parent write and their own client's Kick, their characters the engine's own (a Model with a Humanoid) in the
//! Workspace, in a folder, in ReplicatedStorage, parented to nil, destroyed, none, or a script's Model. The paired probe
//! tools/parity/probes/loopback/player_leaving.luau keeps what a server sees checked (three of four clients leave):
//!
//! - On a server the leave is one step, in this order of events: Players.DescendantRemoving of the player; then, when its
//!   Character is an instance, the player's CharacterRemoving (the character as its argument), the character leaving its
//!   parent (its DescendantRemoving, ChildRemoved and AncestryChanged events; nothing when it has no parent) and Character
//!   becoming nil (Changed); then the rest of the player's DescendantRemoving events (the DataModel's, then its children's);
//!   then PlayerRemoving; then the player's ChildRemoved, AncestryChanged and Changed("Parent"). Handlers run deferred, after
//!   all of it: they see the character unparented, Character nil and the player out of Players. A character anywhere
//!   (in a folder, in ReplicatedStorage, parented to nil, already destroyed) is handled the same way; a player whose
//!   Character is nil leaves with no CharacterRemoving.
//! - The player's Parent is locked from then on ("The Parent property of <name> is locked, current parent: NULL, new
//!   parent Players", in PlayerRemoving's handlers already).
//! - With Workspace.PlayerCharacterDestroyBehavior Default or Enabled the character is destroyed one second after the leave
//!   (59 to 61 frames at 60 Hz) wherever it is then, even when a script put it back in the Workspace, and the Player five
//!   seconds after (299 to 301 frames). Both are the engine's own step: their Destroying handlers run after the frame's
//!   PostSimulation handlers, as a task.delay's do, and no script pays for them. With Disabled neither is destroyed: the
//!   character stays unparented, as does the player.
//! - What a client sees of another player leaving: the character's removal first (its DescendantRemoving, ChildRemoved and
//!   AncestryChanged), then CharacterRemoving and Character's change, then (Destroy) the player's Destroying, then its
//!   DescendantRemoving, PlayerRemoving, ChildRemoved and AncestryChanged, all in one frame; it never sees either destroy.
//!   A client run's local player never leaves this way.
//!
//! A script's own Parent write or Destroy pays for the character's removal what that removal costs (a reparent's prices and
//! its events', instance.rs), and nothing for the rest; a leave the engine makes (Kick's, a client leaving) and the destroys
//! after it are not a script's: nothing is charged for them.

use std::rc::Rc;

use mlua::prelude::*;

use crate::{
    ClassId, Engine, Hooks, InstanceId, NameId, Reflection, State,
    dom::flags,
    instance as core,
    reflection::EnumItemId,
    scheduler::Muted,
    value::Value,
};

/// How long after the leave the character is destroyed, then the Player (PlayerCharacterDestroyBehavior Default or Enabled)
const CHARACTER_LIFETIME: f64 = 1.0;
const PLAYER_LIFETIME: f64 = 5.0;

/// What the leave reads, interned once ([`install`])
#[derive(Clone, Copy)]
struct Names {
    players: ClassId,
    player: ClassId,
    character: NameId,
    character_removing: NameId,
    player_removing: NameId,
    destroy_behavior: Option<NameId>,
    disabled: Option<EnumItemId>,
}

/// The engine's part of players leaving (State.collection, services' state)
#[derive(Default)]
pub(crate) struct Departures {
    names: Option<Names>,
    server: bool,
    workspace: Option<InstanceId>,
    /// A client run's own player, which never leaves this way
    local: Option<InstanceId>,
    /// What is to be destroyed and when (virtual time), pinned until then ([`destroy_due`])
    pending: Vec<(f64, InstanceId)>,
}

/// Interns what the leave reads (services.rs install_players).
pub(crate) fn install(state: &mut State, reflection: &Reflection, server: bool) {
    let (Some(players), Some(player)) = (reflection.class_id("Players"), reflection.class_id("Player")) else { return };
    let (Some(character), Some(character_removing), Some(player_removing)) =
        (reflection.name_id("Character"), reflection.name_id("CharacterRemoving"), reflection.name_id("PlayerRemoving"))
    else {
        return;
    };
    let departures = &mut state.collection.departures;
    departures.names = Some(Names {
        players,
        player,
        character,
        character_removing,
        player_removing,
        destroy_behavior: reflection.name_id("PlayerCharacterDestroyBehavior"),
        disabled: reflection.enum_item("PlayerCharacterDestroyBehavior", "Disabled"),
    });
    departures.server = server;
}

/// The Workspace and the local player, once the DataModel has them (Engine::set_world).
pub(crate) fn set_world(state: &mut State, workspace: InstanceId, local: Option<InstanceId>) {
    state.collection.departures.workspace = Some(workspace);
    state.collection.departures.local = local;
}

/// Whether `id` leaving `old_parent` is a player leaving Players (instance.rs reparent; any other move costs a compare or two).
#[inline]
pub(crate) fn leaving(state: &State, id: InstanceId, old_parent: InstanceId) -> bool {
    let departures = &state.collection.departures;
    departures.names.is_some_and(|names| {
        state.dom.at(old_parent).class == names.players && state.dom.at(id).class == names.player && departures.local != Some(id)
    })
}

/// Whether the leave runs a server's way (the character's step inside Players' DescendantRemoving of the player)
pub(crate) fn on_server(state: &State) -> bool {
    state.collection.departures.server
}

/// The character's step of a leave. On a server, inside Players' DescendantRemoving of the player: CharacterRemoving, the
/// character leaves its parent, Character becomes nil. On a client, before any of the player's own events: the character
/// leaves its parent, then CharacterRemoving and Character's change. Nothing when Character is nil (or no longer an
/// instance the DataModel holds).
pub(crate) fn remove_character(state: &mut State, reflection: &Reflection, hooks: &Hooks, player: InstanceId) {
    let Some(names) = state.collection.departures.names else { return };
    let character = match state.dom.at(player).stored(names.character) {
        Some(Value::Instance(character)) => *character,
        _ => return,
    };
    if state.dom.get(character).is_none() {
        return;
    }
    let server = state.collection.departures.server;
    if server {
        core::fire_event(state, player, names.character_removing, vec![Value::Instance(character)]);
    }
    // (a character's Parent is locked only once it is destroyed, when it has none: the write is no change; the class locks of
    // what a script may have named as the character, Terrain or a service, keep it where it is, and the player itself or what
    // holds it is left alone: it is leaving already)
    if character != player && !state.dom.is_descendant_of(player, character) {
        let _ = core::set_parent(state, reflection, hooks, character, None);
    }
    if !server {
        core::fire_event(state, player, names.character_removing, vec![Value::Instance(character)]);
    }
    core::raw_set(state, reflection, hooks, player, names.character, Value::Nil);
    if server && !destroy_disabled(state, names) && !state.dom.at(character).has(flags::DESTROYED) {
        let at = state.sched.now + CHARACTER_LIFETIME;
        state.dom.pin(character);
        state.collection.departures.pending.push((at, character));
    }
}

/// The player's own step of a leave, once its DescendantRemoving events have fired and before it leaves Players' children:
/// PlayerRemoving; on a server its Parent locks, and the Player is to be destroyed five seconds on.
pub(crate) fn player_removing(state: &mut State, player: InstanceId, players: InstanceId) {
    let Some(names) = state.collection.departures.names else { return };
    core::fire_event(state, players, names.player_removing, vec![Value::Instance(player)]);
    if !state.collection.departures.server {
        return;
    }
    state.dom.at_mut(player).set_flag(flags::PARENT_LOCKED, true);
    if !destroy_disabled(state, names) && !state.dom.at(player).has(flags::DESTROYED) {
        let at = state.sched.now + PLAYER_LIFETIME;
        state.dom.pin(player);
        state.collection.departures.pending.push((at, player));
    }
}

/// Whether the place keeps departed players' characters and Players (Workspace.PlayerCharacterDestroyBehavior Disabled)
fn destroy_disabled(state: &State, names: Names) -> bool {
    let (Some(workspace), Some(behavior), Some(disabled)) = (state.collection.departures.workspace, names.destroy_behavior, names.disabled) else {
        return false;
    };
    matches!(state.dom.at(workspace).stored(behavior), Some(Value::EnumItem(item)) if *item == disabled)
}

/// A play frame's step where task.delay threads resume (scheduler.rs heartbeat_timers, after the simulation events): the departed
/// characters and Players whose time came (a second, or five, after the leave) are destroyed, as the engine's own step that no
/// script pays for (nor a resumed thread's price: there is none). They stay pinned until then. Nothing departed: one compare.
pub(crate) fn destroy_due(lua: &Lua, engine: &Rc<Engine>, now: f64) -> LuaResult<()> {
    // (the frames' times are sums of their durations: five seconds of them can fall a rounding short of the leave's time + 5)
    let now = now + 1e-6;
    let due: Vec<InstanceId> = {
        let mut state = engine.state.borrow_mut();
        let pending = &mut state.collection.departures.pending;
        if pending.is_empty() || !pending.iter().any(|&(at, _)| at <= now) {
            return Ok(());
        }
        let mut due = Vec::new();
        pending.retain(|&(at, id)| {
            if at <= now {
                due.push(id);
            }
            at > now
        });
        for &id in &due {
            state.dom.unpin(id);
        }
        due
    };
    let muted = Muted::raise(&engine.state.borrow().sched.muted);
    for id in due {
        if engine.state.borrow().dom.get(id).is_some() {
            // (what a script made the character, a service say, may refuse: it stays as it is)
            let _ = crate::lua::instance::destroy(lua, engine, id);
        }
    }
    drop(muted);
    Ok(())
}
