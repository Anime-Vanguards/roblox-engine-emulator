//! A player leaving: what becomes of its character and of the Player when it leaves Players, whatever made it leave (its
//! client left, Kick, a script setting its Parent or destroying it, a loopback run's Context.Leave); a player's Character
//! changing; and Kick's delay.
//!
//! Measured in roblox-sim's 0.747.0.7471481 play sessions of one server and two to four clients (Workspace.SignalBehavior
//! Deferred, script identity 2): a server script and one client watched the other players leave in each session, by Kick,
//! Destroy, a Parent write and their own client's Kick, their characters the engine's own (a Model with a Humanoid) in the
//! Workspace, in a folder, in ReplicatedStorage, parented to nil, destroyed, none, or a script's Model; the leavers' own
//! clients reported what they saw. The paired probes tools/parity/probes/loopback/player_leaving.luau, own_leave.luau and
//! character_write.luau keep it checked:
//!
//! - On a server the leave is one step, in this order of events: Players.DescendantRemoving of the player; then, when its
//!   Character is an instance, the player's CharacterRemoving (the character as its argument), the character leaving its
//!   parent (its DescendantRemoving, ChildRemoved and AncestryChanged events; nothing when it has no parent) and Character
//!   becoming nil (Changed); then the player's PlayerGui destroyed (its Destroying and removal events); then the rest of the
//!   player's DescendantRemoving events (the DataModel's, then its other children's); then PlayerRemoving; then the
//!   player's ChildRemoved, AncestryChanged and Changed("Parent"). Handlers run deferred, after all of it: they see the
//!   character unparented, Character nil and the player out of Players. A character anywhere (in a folder, in
//!   ReplicatedStorage, parented to nil, already destroyed) is handled the same way; a player whose Character is nil leaves
//!   with no CharacterRemoving. The Backpack and StarterGear stay in the player until it is destroyed.
//! - The player's Parent is locked from then on ("The Parent property of <name> is locked, current parent: NULL, new
//!   parent Players", in PlayerRemoving's handlers already).
//! - With Workspace.PlayerCharacterDestroyBehavior Default or Enabled the character is destroyed one second after the leave
//!   (59 to 61 frames at 60 Hz) wherever it is then, even when a script put it back in the Workspace, and the Player five
//!   seconds after (299 to 301 frames). Both are the engine's own step: their Destroying handlers run after the frame's
//!   PostSimulation handlers, as a task.delay's do, and no script pays for them. With Disabled neither is destroyed: the
//!   character stays unparented, as does the player.
//! - Kick leaves the player where it is for about a second: Parent, Character and GetPlayers stay as they were, nothing
//!   fires at the call, a second Kick returns and changes nothing, FireClient still reaches its client. The server's own
//!   timer closes the player's connection at the first frame that starts more than a second after the call (what the server
//!   sends its client from that frame on is lost), and the player leaves at the next frame's network step: 62 frames,
//!   1.03 s, at 60 Hz (61 to 62 measured: a frame lands on the second's edge), 32 at 30 Hz, 26 with 40 ms frames. A client
//!   whose main thread was busy four seconds, or that read its replication a second late, did not move it; a client in the
//!   server's process (Studio's Play) left a frame sooner (61). The leave is then as any other, no script paying for it.
//! - A script's write of Character on a server (to another model, a model anywhere or parentless, or nil) fires
//!   CharacterRemoving with the old character when there was one, takes the old one out of its parent, then fires
//!   CharacterAdded with the new one when there is one, then Changed and the property's signal; the old character is
//!   destroyed a second on as a departed player's is (Disabled keeps it), even when it was parentless or put back; the same
//!   value fires nothing. A client (the player's own and the others) sees the old character's removal as its own change,
//!   then CharacterRemoving, CharacterAdded and Changed from the property's change; a client script's own write fires the
//!   same three and moves nothing.
//! - What a client sees of another player leaving: the character's removal first (its DescendantRemoving, ChildRemoved and
//!   AncestryChanged), then CharacterRemoving and Character's change, then (Destroy) the player's Destroying, then its
//!   DescendantRemoving, PlayerRemoving, ChildRemoved and AncestryChanged, all in one frame; it never sees either destroy.
//!   A client run's local player never leaves this way; a loopback client's does when its server takes it out (it sees its
//!   own leave so, then hears nothing more: loopback/mod.rs).
//!
//! A script's own Parent write or Destroy pays for the character's removal what that removal costs (a reparent's prices and
//! its events', instance.rs), and nothing for the rest (the PlayerGui's destroy included); a script's Character write pays
//! the old character's removal too; a leave the engine makes (Kick's, a client leaving) and the destroys after it are not a
//! script's: nothing is charged for them. A client pays 0.3 to 0.5 us for a Character write of another player's (and a
//! queued handler's price for each CharacterAdded and CharacterRemoving handler), which a write's own price covers.

use std::rc::Rc;

use mlua::prelude::*;

use crate::{
    ClassId, Engine, Hooks, InstanceId, K, NameId, Reflection, State,
    dom::flags,
    instance as core,
    reflection::EnumItemId,
    scheduler::Muted,
    value::Value,
};

/// How long after the leave the character is destroyed, then the Player (PlayerCharacterDestroyBehavior Default or Enabled)
const CHARACTER_LIFETIME: f64 = 1.0;
const PLAYER_LIFETIME: f64 = 5.0;
/// Kick: the server closes the player's connection at the first frame that starts more than this long after the call
const KICK_DELAY: f64 = 1.0;

/// What the leave reads, interned once ([`install`])
#[derive(Clone, Copy)]
struct Names {
    players: ClassId,
    player: ClassId,
    player_gui: Option<ClassId>,
    character: NameId,
    character_added: NameId,
    character_removing: NameId,
    player_removing: NameId,
    destroy_behavior: Option<NameId>,
    disabled: Option<EnumItemId>,
}

/// A kicked player waiting to leave ([`kick`], [`network_step`])
struct Kick {
    player: InstanceId,
    /// The call's time and the delay: the first frame that starts after it closes the connection
    due: f64,
    /// The connection closed (in an earlier frame): the player leaves at this frame's network step
    closed: bool,
}

/// The engine's part of players leaving (State.collection, services' state)
#[derive(Default)]
pub(crate) struct Departures {
    names: Option<Names>,
    server: bool,
    workspace: Option<InstanceId>,
    /// A client run's own player, which never leaves this way
    local: Option<InstanceId>,
    /// A loopback run's client: its own player leaves as any other when its server takes it out
    loopback_client: bool,
    /// What is to be destroyed and when (virtual time), pinned until then ([`destroy_due`])
    pending: Vec<(f64, InstanceId)>,
    /// Players kicked and not yet gone
    kicks: Vec<Kick>,
}

/// Interns what the leave reads (services.rs install_players).
pub(crate) fn install(state: &mut State, reflection: &Reflection, server: bool) {
    let (Some(players), Some(player)) = (reflection.class_id("Players"), reflection.class_id("Player")) else { return };
    let (Some(character), Some(character_added), Some(character_removing), Some(player_removing)) = (
        reflection.name_id("Character"),
        reflection.name_id("CharacterAdded"),
        reflection.name_id("CharacterRemoving"),
        reflection.name_id("PlayerRemoving"),
    ) else {
        return;
    };
    let departures = &mut state.collection.departures;
    departures.names = Some(Names {
        players,
        player,
        player_gui: reflection.class_id("PlayerGui"),
        character,
        character_added,
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

/// A loopback run's client (loopback::link): its own player leaves as any player when the server's removal reaches it.
pub(crate) fn set_loopback_client(state: &mut State) {
    state.collection.departures.loopback_client = true;
}

/// Whether `id` leaving `old_parent` is a player leaving Players (instance.rs reparent; any other move costs a compare or two).
#[inline]
pub(crate) fn leaving(state: &State, id: InstanceId, old_parent: InstanceId) -> bool {
    let departures = &state.collection.departures;
    departures.names.is_some_and(|names| {
        state.dom.at(old_parent).class == names.players
            && state.dom.at(id).class == names.player
            && (departures.local != Some(id) || departures.loopback_client)
    })
}

/// Whether `player` is in Players (a loopback run's check that its server took the player out)
pub(crate) fn in_players(state: &State, player: InstanceId) -> bool {
    let Some(names) = state.collection.departures.names else { return true };
    state.dom.get(player).and_then(|instance| instance.parent).is_some_and(|parent| state.dom.at(parent).class == names.players)
}

/// Whether the leave runs a server's way (the character's step inside Players' DescendantRemoving of the player)
pub(crate) fn on_server(state: &State) -> bool {
    state.collection.departures.server
}

/// Whether property `name` of instance `id` is a Player's Character (a script's write and a loopback client's replicated change
/// of it go through [`set_character`]: one compare for any other write)
#[inline]
pub(crate) fn is_character(state: &State, id: InstanceId, name: NameId) -> bool {
    state.collection.departures.names.is_some_and(|names| names.character == name && state.dom.at(id).class == names.player)
}

/// A script's write of a Player's Character (lua/instance.rs script_set_property): [`set_character`], the old character taken
/// out on a server. Returns whether the value changed.
pub(crate) fn write_character(engine: &Engine, player: InstanceId, value: Value) -> bool {
    let hooks = engine.hooks.borrow();
    let mut state = engine.state.borrow_mut();
    let server = state.collection.departures.server;
    set_character(&mut state, &engine.reflection, &hooks, player, value, server)
}

/// A player's Character becoming `new`: CharacterRemoving with the old character when it held one; with `takes_out` (a
/// server's script write or leave) the old character leaving its parent; the value stored; CharacterAdded with the new one when
/// it is an instance; then Changed and the property's signal (Changed's hooks after). With `takes_out` the old character is also
/// destroyed a second on, unless the place's PlayerCharacterDestroyBehavior is Disabled or it is destroyed already. The value it
/// holds: nothing. Returns whether the value changed.
pub(crate) fn set_character(state: &mut State, reflection: &Reflection, hooks: &Hooks, player: InstanceId, new: Value, takes_out: bool) -> bool {
    let Some(names) = state.collection.departures.names else { return false };
    let held = match state.dom.at(player).stored(names.character) {
        Some(Value::Instance(old)) => Some(*old),
        _ => None,
    };
    let wanted = match &new {
        Value::Instance(id) => Some(*id),
        _ => None,
    };
    if held == wanted {
        return false;
    }
    let old = held.filter(|&old| state.dom.get(old).is_some());
    if let Some(old) = old {
        core::fire_event(state, player, names.character_removing, vec![Value::Instance(old)]);
        // (a character's Parent is locked only once it is destroyed, when it has none: the write is no change; the class locks of
        // what a script may have named as the character, Terrain or a service, keep it where it is, and the player itself or what
        // holds it is left alone)
        if takes_out && old != player && state.dom.at(old).parent.is_some() && !state.dom.is_descendant_of(player, old) {
            let _ = core::set_parent(state, reflection, hooks, old, None);
        }
    }
    let previous = state.dom.at(player).stored(names.character).cloned();
    core::raw_set_silent(state, reflection, player, names.character, new);
    if let Some(added) = wanted.filter(|&added| state.dom.get(added).is_some()) {
        core::fire_event(state, player, names.character_added, vec![Value::Instance(added)]);
    }
    core::notify_changed(state, reflection, hooks, player, names.character, previous);
    if takes_out
        && let Some(old) = old
        && !destroy_disabled(state, names)
        && !state.dom.at(old).has(flags::DESTROYED)
        && !state.collection.departures.pending.iter().any(|&(_, id)| id == old)
    {
        let at = state.sched.now + CHARACTER_LIFETIME;
        state.dom.pin(old);
        state.collection.departures.pending.push((at, old));
    }
    true
}

/// The character's step of a leave. On a server, inside Players' DescendantRemoving of the player: CharacterRemoving, the
/// character leaves its parent, Character becomes nil ([`set_character`]). On a client, before any of the player's own events:
/// the character leaves its parent, then CharacterRemoving and Character's change. Nothing when Character is nil (or no longer
/// an instance the DataModel holds).
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
    if !server && character != player && !state.dom.is_descendant_of(player, character) {
        let _ = core::set_parent(state, reflection, hooks, character, None);
    }
    set_character(state, reflection, hooks, player, Value::Nil, server);
}

/// A server's leave, after the character's step and before the rest of the player's DescendantRemoving events: the player's
/// PlayerGui is destroyed (its Destroying, then its removal events), as the engine's own step no script pays for. The Backpack
/// and StarterGear stay until the Player is destroyed.
pub(crate) fn destroy_player_gui(state: &mut State, reflection: &Reflection, hooks: &Hooks, player: InstanceId) {
    let Some(names) = state.collection.departures.names else { return };
    let Some(class) = names.player_gui else { return };
    if !state.collection.departures.server {
        return;
    }
    let Some(gui) = state.dom.find_child_of_class(player, class) else { return };
    let muted = Muted::raise(&state.sched.muted);
    // (Destroy's steps on the state alone, lua/instance.rs destroy_subtree's order: each instance's Destroying, then its
    // removal, then its children's in turn; a GUI holds no bundle to import first)
    let mut steps = vec![(gui, false)];
    while let Some((id, finish)) = steps.pop() {
        if finish {
            core::finish_destroy(state, id);
            continue;
        }
        if state.dom.get(id).is_none_or(|instance| instance.has(flags::DESTROYED)) {
            continue;
        }
        if let Some(signal) = state.dom.at(id).signals.as_ref().and_then(|signals| signals.event(K::Destroying as NameId)) {
            core::fire(state, signal, Vec::new());
        }
        let Ok(children) = core::begin_destroy(state, reflection, hooks, id) else { continue };
        steps.push((id, true));
        steps.extend(children.into_iter().rev().map(|child| (child, false)));
    }
    drop(muted);
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

/// Player:Kick on a server (server.luau): the player stays until the server's timer closes its connection, at the first frame
/// that starts more than a second after the call, and leaves at the next frame's network step ([`network_step`]). A second
/// Kick changes nothing; a player not in Players is not kicked.
pub(crate) fn kick(state: &mut State, player: InstanceId) {
    let Some(names) = state.collection.departures.names else { return };
    let in_players = state.dom.get(player).and_then(|instance| instance.parent).is_some_and(|parent| state.dom.at(parent).class == names.players);
    if !state.collection.departures.server || !in_players || state.collection.departures.kicks.iter().any(|kick| kick.player == player) {
        return;
    }
    let due = state.sched.now + KICK_DELAY;
    state.collection.departures.kicks.push(Kick { player, due, closed: false });
}

/// A server frame's network step (scheduler.rs heartbeat_timers, after a loopback run's receive point): a kicked player whose
/// connection closed in an earlier frame leaves now, as the engine's own step that no script pays for; a kick whose second has
/// passed closes its player's connection (a loopback run's client hears nothing the server sends from now on:
/// loopback::connection_closed). A kicked player that left meanwhile is forgotten. Nothing kicked: one compare.
pub(crate) fn network_step(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let (leaving, closing) = {
        let mut state = engine.state.borrow_mut();
        if state.collection.departures.kicks.is_empty() {
            return Ok(());
        }
        let Some(names) = state.collection.departures.names else { return Ok(()) };
        // (the frames' times are sums of their durations: a frame on the second's edge counts as not past it)
        let now = state.sched.now - 1e-6;
        let kicks = std::mem::take(&mut state.collection.departures.kicks);
        let (mut leaving, mut closing, mut kept) = (Vec::new(), Vec::new(), Vec::new());
        for kick in kicks {
            let present = state.dom.get(kick.player).and_then(|instance| instance.parent).is_some_and(|parent| state.dom.at(parent).class == names.players);
            if !present {
                continue;
            }
            if kick.closed {
                leaving.push(kick.player);
            } else if now > kick.due {
                closing.push(kick.player);
                kept.push(Kick { closed: true, ..kick });
            } else {
                kept.push(kick);
            }
        }
        state.collection.departures.kicks = kept;
        (leaving, closing)
    };
    for player in closing {
        crate::loopback::connection_closed(lua, engine, player)?;
    }
    if !leaving.is_empty() {
        let muted = Muted::raise(&engine.state.borrow().sched.muted);
        for player in leaving {
            let _ = crate::lua::instance::set_parent(engine, player, None);
        }
        drop(muted);
    }
    Ok(())
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
