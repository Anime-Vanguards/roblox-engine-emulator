//! Teams: a player's Team, TeamColor and Neutral, the Teams service's teams, and the team a joining player gets.
//!
//! Measured in roblox-sim's 0.747.0.7471481 play sessions, script identity 2: tools/parity/probes/teams.luau (a client's own
//! player and teams), loopback/team_join.luau, team_pick.luau, team_none.luau, team_absent.luau and team_links.luau (each
//! captured three times, the same lines), and sessions of four clients joining one server ten seconds apart (docs/engine.md,
//! Log 2026-11-20):
//!
//! - A player's team follows its TeamColor and Neutral: Team is nil while Neutral is true, else the first team in Teams
//!   (child order, Teams' own children only) whose TeamColor is the player's, else nil. Each side works it out for itself
//!   whenever one of those changes: the player's TeamColor or Neutral, a team's TeamColor, a team coming into or leaving
//!   Teams (a destroyed one leaves). A player whose team leaves keeps its TeamColor and Neutral (false), its Team nil; the
//!   team coming back gives it the team again. A change fires the old team's PlayerRemoved, the new one's PlayerAdded, then
//!   the player's Team change signals, after those of the TeamColor or Neutral write that caused it.
//! - Writing Team writes TeamColor (the team's) and then Neutral (false), each with its own events, and the team follows
//!   from them: so writing a team whose colour an earlier team in Teams has gives that earlier team (no event when it was the
//!   team already). Writing nil writes Neutral true. A team outside Teams (destroyed or never parented too) raises "Setting
//!   Player.Team failed because team must be a child of the Teams service" and changes nothing; another instance "Expected
//!   Team got <ClassName> for Player.Team."; another value "Unable to assign property Team. Team expected, got <type>".
//!   TeamColor and Neutral are plain writes (a BrickColor; any value, by truth).
//! - A player starts neutral: Neutral true, TeamColor White, Team nil. On a server, a player who joins gets, before the
//!   team's PlayerAdded, Players.PlayerAdded and Players.ChildAdded fire (in that order; ree fires ChildAdded first,
//!   known-loopback.json), the auto-assignable team in Teams with the fewest players, the first in child order among the
//!   fewest, and with it that team's TeamColor and Neutral false (with no auto-assignable team in Teams, it stays
//!   neutral). A player is counted on every team of its colour. Players who are in already keep their teams when teams
//!   are added or changed later.
//! - Team:GetPlayers() is every player in Players who is not neutral and has the team's colour (two teams of one colour
//!   both list it), in Players' child order; a team outside Teams raises "Team:GetPlayers() failed because team must be a
//!   child of the Teams service". Teams:GetTeams() is Teams' children that are teams, in child order.
//! - What a client sees: its player's TeamColor and Neutral as the server writes them, its Team worked out on its side
//!   from its own copies of the teams (the same events, in the same order as the server's), so Team itself is not sent
//!   (loopback/replicate.rs); at its first line its player has its team already (ree has no staged ReplicatedFirst load,
//!   where the client's first LocalScript sees the colour before the teams arrive). A client-only run's local player gets the
//!   team its server would give the first player, with no events, as a client finds it at its first line.

use std::rc::{Rc, Weak};

use mlua::prelude::*;

use super::{Behaviors, method, values};
use crate::{
    ClassId, Engine, Hooks, InstanceId, NameId, Reflection, State,
    instance as core,
    lua::{instance_id, instance_value, raise, typeof_value},
    value::Value,
};

/// The classes and members the rules read, interned once
#[derive(Clone, Copy)]
pub struct Names {
    player: ClassId,
    team: ClassId,
    teams: ClassId,
    players: ClassId,
    team_prop: NameId,
    team_color: NameId,
    neutral: NameId,
    auto_assignable: NameId,
    player_added: NameId,
    player_removed: NameId,
}

impl Names {
    pub fn new(reflection: &Reflection) -> Option<Names> {
        Some(Names {
            player: reflection.class_id("Player")?,
            team: reflection.class_id("Team")?,
            teams: reflection.class_id("Teams")?,
            players: reflection.class_id("Players")?,
            team_prop: reflection.name_id("Team")?,
            team_color: reflection.name_id("TeamColor")?,
            neutral: reflection.name_id("Neutral")?,
            auto_assignable: reflection.name_id("AutoAssignable")?,
            player_added: reflection.name_id("PlayerAdded")?,
            player_removed: reflection.name_id("PlayerRemoved")?,
        })
    }
}

/// A BrickColor value's number (TeamColor is a BrickColor on players and teams)
fn brick(value: Option<&Value>) -> Option<u16> {
    match value? {
        Value::Dom(variant) => match &**variant {
            rbx_dom_weak::types::Variant::BrickColor(color) => Some(*color as u16),
            _ => None,
        },
        _ => None,
    }
}

/// The DataModel's Teams service, if it has one (found, never made)
fn teams_service(state: &State, names: &Names, game: InstanceId) -> Option<InstanceId> {
    state.dom.get(game)?;
    state.dom.children_of(game).iter().copied().find(|&child| state.dom.at(child).class == names.teams)
}

/// The DataModel's Players service
fn players_service(state: &State, names: &Names, game: InstanceId) -> Option<InstanceId> {
    state.dom.get(game)?;
    state.dom.children_of(game).iter().copied().find(|&child| state.dom.at(child).class == names.players)
}

/// Teams' children that are teams, in child order
fn teams_of(state: &State, names: &Names, teams: Option<InstanceId>) -> Vec<InstanceId> {
    let Some(teams) = teams else { return Vec::new() };
    state.dom.children_of(teams).iter().copied().filter(|&child| state.dom.at(child).class == names.team).collect()
}

/// The players in Players, in child order
fn players_of(state: &State, names: &Names, game: InstanceId) -> Vec<InstanceId> {
    let Some(players) = players_service(state, names, game) else { return Vec::new() };
    state.dom.children_of(players).iter().copied().filter(|&child| state.dom.at(child).class == names.player).collect()
}

fn neutral(state: &State, names: &Names, player: InstanceId) -> bool {
    state.dom.boolean(player, names.neutral) != Some(false)
}

/// Whether a player is on a team's colour (not neutral, the same TeamColor): what Team:GetPlayers counts
fn on_colour(state: &State, names: &Names, player: InstanceId, colour: Option<u16>) -> bool {
    !neutral(state, names, player) && colour.is_some() && brick(state.dom.raw_get(player, names.team_color)) == colour
}

/// The team a player's TeamColor and Neutral give it: none when neutral, else the first team in Teams of its colour
fn derived(state: &State, names: &Names, teams: Option<InstanceId>, player: InstanceId) -> Option<InstanceId> {
    if neutral(state, names, player) {
        return None;
    }
    let colour = brick(state.dom.raw_get(player, names.team_color))?;
    teams_of(state, names, teams).into_iter().find(|&team| brick(state.dom.raw_get(team, names.team_color)) == Some(colour))
}

fn current(state: &State, names: &Names, player: InstanceId) -> Option<InstanceId> {
    match state.dom.at(player).stored(names.team_prop) {
        Some(Value::Instance(team)) => Some(*team),
        _ => None,
    }
}

/// Brings a player's Team to what its TeamColor and Neutral give: the old team's PlayerRemoved, the new one's PlayerAdded,
/// then the Team write's own events
fn refresh(state: &mut State, reflection: &Reflection, hooks: &Hooks, names: &Names, teams: Option<InstanceId>, player: InstanceId) {
    let (was, now) = (current(state, names, player), derived(state, names, teams, player));
    if was == now {
        return;
    }
    if let Some(old) = was.filter(|&old| state.dom.get(old).is_some()) {
        core::fire_event(state, old, names.player_removed, vec![Value::Instance(player)]);
    }
    if let Some(new) = now {
        core::fire_event(state, new, names.player_added, vec![Value::Instance(player)]);
    }
    core::raw_set(state, reflection, hooks, player, names.team_prop, now.map_or(Value::Nil, Value::Instance));
}

/// [`refresh`] for every player in Players (a team changed colour, or came into Teams or left it)
fn refresh_all(state: &mut State, reflection: &Reflection, hooks: &Hooks, names: &Names, game: InstanceId) {
    let teams = teams_service(state, names, game);
    for player in players_of(state, names, game) {
        refresh(state, reflection, hooks, names, teams, player);
    }
}

/// The team a joining player gets: the auto-assignable team in Teams with the fewest players on its colour, the first in
/// child order among them (`joining` itself not counted)
fn pick(state: &State, names: &Names, game: InstanceId, joining: InstanceId) -> Option<InstanceId> {
    let teams = teams_service(state, names, game);
    let players = players_of(state, names, game);
    let mut best: Option<(InstanceId, usize)> = None;
    for team in teams_of(state, names, teams) {
        if state.dom.boolean(team, names.auto_assignable) != Some(true) {
            continue;
        }
        let colour = brick(state.dom.raw_get(team, names.team_color));
        let count = players.iter().filter(|&&player| player != joining && on_colour(state, names, player, colour)).count();
        if best.is_none_or(|(_, fewest)| count < fewest) {
            best = Some((team, count));
        }
    }
    best.map(|(team, _)| team)
}

/// A server's joining player takes its team (services/mod.rs, before PlayerAdded): the TeamColor and Neutral writes, each
/// with its events, and the team's PlayerAdded with the Team write
pub fn assign_joining(state: &mut State, reflection: &Reflection, hooks: &Hooks, game: InstanceId, player: InstanceId) {
    let Some(names) = Names::new(reflection) else { return };
    let Some(team) = pick(state, &names, game, player) else { return };
    let colour = state.dom.raw_get(team, names.team_color).cloned().unwrap_or(Value::Nil);
    core::raw_set(state, reflection, hooks, player, names.team_color, colour);
    core::raw_set(state, reflection, hooks, player, names.neutral, Value::Bool(false));
}

/// A client's local player as it finds itself at its first line, with no events: in a client-only run the team its server
/// would have given it (the first to join), in a loopback client the team its own copies of the teams give the TeamColor
/// and Neutral the server sent (Team is not sent)
pub fn settle_local_player(engine: &Engine, player: InstanceId, joined_from_server: bool) {
    let Some(names) = Names::new(&engine.reflection) else { return };
    let game = engine.world().game;
    let reflection = &engine.reflection;
    let mut state = engine.state.borrow_mut();
    if !joined_from_server
        && let Some(team) = pick(&state, &names, game, player)
    {
        let colour = state.dom.raw_get(team, names.team_color).cloned().unwrap_or(Value::Nil);
        core::raw_set_silent(&mut state, reflection, player, names.team_color, colour);
        core::raw_set_silent(&mut state, reflection, player, names.neutral, Value::Bool(false));
    }
    let teams = teams_service(&state, &names, game);
    let team = derived(&state, &names, teams, player);
    core::raw_set_silent(&mut state, reflection, player, names.team_prop, team.map_or(Value::Nil, Value::Instance));
}

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let Some(names) = Names::new(&engine.reflection) else { return };

    // Team:GetPlayers: the players in Players on the team's colour (extra arguments are ignored)
    if let Some(get_players) = engine.reflection.name_id("GetPlayers") {
        behaviors.add_method(names.team, get_players, method(move |lua, engine, this, _| {
            let game = engine.world().game;
            let members = {
                let state = engine.state.borrow();
                let in_teams = state.dom.at(this).parent.is_some_and(|parent| state.dom.at(parent).class == names.teams);
                if !in_teams {
                    drop(state);
                    return raise("Team:GetPlayers() failed because team must be a child of the Teams service");
                }
                let colour = brick(state.dom.raw_get(this, names.team_color));
                players_of(&state, &names, game).into_iter().filter(|&player| on_colour(&state, &names, player, colour)).collect::<Vec<_>>()
            };
            let table = lua.create_table()?;
            for player in members {
                table.raw_push(instance_value(lua, engine, player)?)?;
            }
            Ok(values([LuaValue::Table(table)]))
        }));
    }
    // Teams:GetTeams: its children that are teams
    if let Some(get_teams) = engine.reflection.name_id("GetTeams") {
        behaviors.add_method(names.teams, get_teams, method(move |lua, engine, this, _| {
            let teams = teams_of(&engine.state.borrow(), &names, Some(this));
            let table = lua.create_table()?;
            for team in teams {
                table.raw_push(instance_value(lua, engine, team)?)?;
            }
            Ok(values([LuaValue::Table(table)]))
        }));
    }
    // Player.Team: a team in Teams writes its TeamColor, then Neutral false; nil writes Neutral true
    behaviors.add_setter(names.player, names.team_prop, Rc::new(move |_, engine, id, value| {
        let team = match &value {
            LuaValue::Nil => None,
            other => match instance_id(other) {
                Some(team) => Some(team),
                None => return raise(format!("Unable to assign property Team. Team expected, got {}", typeof_value(engine, other))),
            },
        };
        let Some(team) = team else {
            crate::lua::instance::set_property(engine, id, names.neutral, Value::Bool(true));
            return Ok(());
        };
        let colour = {
            let state = engine.state.borrow();
            let class = state.dom.at(team).class;
            if class != names.team {
                let class_name = engine.reflection.class(class).name.to_string();
                drop(state);
                return raise(format!("Expected Team got {class_name} for Player.Team."));
            }
            let in_teams = state.dom.at(team).parent.is_some_and(|parent| state.dom.at(parent).class == names.teams);
            if !in_teams {
                drop(state);
                return raise("Setting Player.Team failed because team must be a child of the Teams service");
            }
            state.dom.raw_get(team, names.team_color).cloned().unwrap_or(Value::Nil)
        };
        crate::lua::instance::set_property(engine, id, names.team_color, colour);
        crate::lua::instance::set_property(engine, id, names.neutral, Value::Bool(false));
        Ok(())
    }));

    // Each side works a player's team out again when something it follows changes: the player's TeamColor or Neutral, a
    // team's TeamColor (one player, or all), a team coming into Teams or leaving it (all)
    let weak: Weak<Engine> = Rc::downgrade(engine);
    let mut hooks = engine.hooks.borrow_mut();
    hooks.property_changed.push(Box::new(move |state, id, name| {
        if name != names.team_color && name != names.neutral {
            return;
        }
        let class = state.dom.at(id).class;
        if class != names.player && class != names.team {
            return;
        }
        let Some(engine) = weak.upgrade() else { return };
        let Some(world) = engine.world.get() else { return };
        let hooks = engine.hooks.borrow();
        if class == names.player {
            let teams = teams_service(state, &names, world.game);
            refresh(state, &engine.reflection, &hooks, &names, teams, id);
        } else if name == names.team_color && state.dom.at(id).parent.is_some_and(|parent| state.dom.at(parent).class == names.teams) {
            refresh_all(state, &engine.reflection, &hooks, &names, world.game);
        }
    }));
    let weak: Weak<Engine> = Rc::downgrade(engine);
    hooks.hierarchy_changed.push(Box::new(move |state, id, old, new| {
        if state.dom.at(id).class != names.team {
            return;
        }
        let is_teams = |parent: Option<InstanceId>| parent.is_some_and(|parent| state.dom.get(parent).is_some_and(|parent| parent.class == names.teams));
        if !is_teams(old) && !is_teams(new) {
            return;
        }
        let Some(engine) = weak.upgrade() else { return };
        let Some(world) = engine.world.get() else { return };
        let hooks = engine.hooks.borrow();
        refresh_all(state, &engine.reflection, &hooks, &names, world.game);
    }));
}
