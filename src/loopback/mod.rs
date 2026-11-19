//! The loopback run context (`--run-context loopback`): a server and its client in one process, as one play session's two
//! sides. Two engines, two DataModels, two Luau VMs, one thread: each frame the server steps, then the client, with the
//! same `dt`. The client's DataModel is built by replication from the server's (replicate.rs): at the join a copy of the
//! replicated containers, then every change the server makes there, in one ordered stream with the remote traffic
//! (bridge.rs, remotes.rs, wire.rs). Nothing the client does to instances reaches the server.
//!
//! The link lives in the client engine ([`Engine::peer`]): a host that steps the client steps the pair, with no change
//! (`ree run` / `record`, rur's renders and sessions). Each side keeps its own rules: the server's are a server run's
//! (`RunContext::Server`, server.luau), the client's a client run's.
//!
//! Measured in a play session's two sides (Studio 0.747, a headless session of two processes, tools/parity/probes/loopback; docs/engine.md,
//! Log 2026-11-19): what one side sends during a frame reaches the other at the start of its next frame (an event echoed by
//! the other side comes back at the start of the sender's next frame), at the receive point, after the legacy wait()
//! resumptions and before PreAnimation ([`receive`], called by the scheduler).

pub mod bridge;
pub mod remotes;
pub mod replicate;
pub mod wire;

use std::rc::Rc;

use mlua::prelude::*;

use crate::{Emulator, Engine, InstanceId, K, NameId, Options, RunContext, instance as core, value::Value};

pub use bridge::Bridge;

/// A side of the pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Server,
    Client,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Server => "server",
            Side::Client => "client",
        }
    }

    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub fn other(self) -> Side {
        match self {
            Side::Server => Side::Client,
            Side::Client => Side::Server,
        }
    }
}

/// What a linked engine holds ([`Engine::loopback`]): its side, the bridge, and the calls its scripts wait on.
pub struct Endpoint {
    pub side: Side,
    pub bridge: Rc<Bridge>,
    /// This side's remote state: its callers waiting for an answer, events waiting for a handler (remotes.rs)
    pub remotes: remotes::Remotes,
}

/// What the client engine holds ([`Engine::peer`]): the server it is linked to.
pub struct Link {
    pub server: Emulator,
    pub bridge: Rc<Bridge>,
}

/// How a loopback run is set up.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// Frames each way ([`Bridge::latency`]); 1 by default, the measured minimum
    pub latency: u32,
    /// A recording samples the server's world (`--record-side server`), not the client's
    pub record_server: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Spec { latency: 1, record_server: false }
    }
}

/// The options of one side of a loopback run made from `options` (whose run context is [`RunContext::Loopback`]): the
/// server's without a local player, the client's with the player `--player` named.
pub fn side_options(options: &Options, side: Side) -> Options {
    let mut options = options.clone();
    match side {
        Side::Server => {
            options.run_context = RunContext::Server;
            options.local_player_name = None;
        }
        Side::Client => {
            options.run_context = RunContext::Client;
            if options.local_player_name.is_none() {
                options.local_player_name = Some("Player".into());
            }
        }
    }
    options
}

impl Engine {
    /// The endpoint of a linked engine (either side of a loopback run)
    #[inline]
    pub fn loopback(&self) -> Option<Rc<Endpoint>> {
        self.loopback.borrow().clone()
    }

    /// A loopback run's server as the client engine holds it: its VM and engine (handles: nothing stays borrowed)
    pub fn peer_server(&self) -> Option<(Lua, Rc<Engine>)> {
        self.peer.borrow().as_ref().map(|link| ((*link.server.lua).clone(), link.server.engine.clone()))
    }
}

/// Links a started server and a started client: each gets its endpoint and the remote methods of its side, the client
/// holds the server ([`Engine::peer`]), the server's output goes to the client's list with `side` (one list in program
/// order), and the server's player is made, waiting for the join ([`join`]).
pub fn link(server: Emulator, client: &Emulator, spec: Spec) -> Result<(), String> {
    let bridge = Rc::new(Bridge::new(spec.latency));
    bridge.record_server.set(spec.record_server);
    for (emulator, side) in [(&server, Side::Server), (client, Side::Client)] {
        let endpoint = Rc::new(Endpoint { side, bridge: bridge.clone(), remotes: Default::default() });
        *emulator.engine.loopback.borrow_mut() = Some(endpoint);
        emulator.engine.state.borrow_mut().output.side = Some(side.name());
        remotes::install(&emulator.lua, &emulator.engine, side).map_err(|error| error.to_string())?;
    }
    server.engine.state.borrow_mut().output.forward = Some(crate::output::Forward(Rc::downgrade(&client.engine)));
    // the server's player: made now (a harness's Context.SetJoinData may give it its join data), joined at the join
    let client_player = client.engine.world().local_player.ok_or("the client has no local player")?;
    let server_player = {
        let engine = &server.engine;
        let reflection = &engine.reflection;
        let (name, display, user_id) = {
            let state = client.engine.state.borrow();
            let player = state.dom.at(client_player);
            let display = state.dom.raw_get(client_player, K::DisplayName as NameId).cloned().unwrap_or(Value::Nil);
            let user_id = state.dom.raw_get(client_player, K::UserId as NameId).cloned().unwrap_or(Value::Number(1.0));
            (player.name.clone(), display, user_id)
        };
        let mut state = engine.state.borrow_mut();
        let class = reflection.class_id("Player").ok_or("no Player class")?;
        let player = core::create_named_str(&mut state, reflection, class, name);
        core::raw_set_silent(&mut state, reflection, player, K::DisplayName as NameId, display);
        core::raw_set_silent(&mut state, reflection, player, K::UserId as NameId, user_id);
        // (the loopback player is no harness's fake one: it does not leave with the run's state)
        state.dom.pin(player);
        player
    };
    bridge.player.set(Some((server_player, client_player)));
    bridge.ids.borrow_mut().pair(server_player, client_player);
    *client.engine.peer.borrow_mut() = Some(Box::new(Link { server, bridge }));
    Ok(())
}

/// The player joins (before the client's harness starts, before frame 1 unless the server's harness held it): the
/// server's Player enters Players (its containers made and PlayerAdded fired there, as for any joining player of a
/// server), the client gets a copy of the server's replicated containers (replicate.rs), its local player takes
/// StarterPlayer's settings from that copy, and the server's changes replicate from now on.
pub fn join(client_lua: &Lua, client: &Rc<Engine>) -> Result<(), String> {
    let Some((server_lua, server)) = client.peer_server() else { return Err("not a loopback client".into()) };
    let endpoint = client.loopback().ok_or("not a loopback client")?;
    let bridge = endpoint.bridge.clone();
    if bridge.joined.get() {
        return Ok(());
    }
    let (server_player, client_player) = bridge.player.get().ok_or("no loopback player")?;
    let players = crate::services::get_service(&server, server.world().game, "Players").map_err(|_| "no Players")?;
    crate::lua::instance::set_parent(&server, server_player, Some(players)).map_err(|_| "the player cannot join".to_string())?;
    // (the containers a server makes for a joining player pair with the client's: by class, made where it has none)
    replicate::join(&server_lua, &server, client_lua, client, &bridge, server_player, client_player)?;
    // (a client has an empty ServerScriptService: measured, tools/parity/probes/loopback/replicated_services.luau)
    let _ = crate::services::get_service(client, client.world().game, "ServerScriptService");
    // (instance streaming is not emulated: the client gets the whole Workspace, said once)
    let streaming = server.reflection.name_id("StreamingEnabled").is_some_and(|name| server.state.borrow().dom.boolean(server.world().workspace, name) == Some(true));
    if streaming {
        client.state.borrow_mut().output.missing("Workspace", "StreamingEnabled (a loopback client gets the whole Workspace)");
    }
    crate::lua::globals::join_local_player(client);
    bridge.joined.set(true);
    let frames = server.state.borrow().sched.frame;
    bridge.joined_at.set(frames);
    Ok(())
}

/// The engine a loopback recording samples: the client's, or the server's (`--record-side server`); any other engine itself
pub fn recorded(engine: &Rc<Engine>) -> Rc<Engine> {
    match (engine.peer_server(), engine.loopback()) {
        (Some((_, server)), Some(endpoint)) if endpoint.bridge.record_server.get() => server,
        _ => engine.clone(),
    }
}

/// A loopback run with no harness starts each side from the place's own scripts, as a play session does (Roblox's
/// documentation of Script.RunContext and of the containers scripts run in; a session's own order is not measured: here,
/// GetDescendants' order). The server runs every enabled Script whose RunContext is Legacy under Workspace or
/// ServerScriptService, or Server anywhere but ServerStorage. The client copies StarterPlayerScripts' children into its
/// PlayerScripts, then runs every enabled LocalScript under ReplicatedFirst, its PlayerScripts, PlayerGui and Backpack,
/// and every enabled Script whose RunContext is Client outside ServerStorage and the Starter containers. Each starts once.
pub fn boot(lua: &Lua, engine: &Rc<Engine>, side: Side) -> Result<(), String> {
    if side == Side::Client {
        copy_player_scripts(engine);
    }
    for script in eligible(engine, side) {
        start_script(lua, engine, script);
    }
    Ok(())
}

/// StarterPlayer.StarterPlayerScripts' children, copied into the local player's PlayerScripts as a joining client does
fn copy_player_scripts(engine: &Rc<Engine>) {
    let Some(player) = engine.world().local_player else { return };
    let game = engine.world().game;
    let Some(starter) = crate::services::find_service(engine, game, "StarterPlayer") else { return };
    let templates = crate::lua::instance::find_first_child(engine, starter, Some(b"StarterPlayerScripts"), false);
    let scripts = engine.reflection.class_id("PlayerScripts").and_then(|class| engine.state.borrow().dom.find_child_of_class(player, class));
    let (Some(templates), Some(scripts)) = (templates, scripts) else { return };
    crate::project::materialize_deep(engine, templates);
    let children = engine.state.borrow().dom.at(templates).children.clone();
    for child in children {
        if let Some(copy) = crate::lua::instance::clone(engine, child) {
            let _ = crate::lua::instance::set_parent(engine, copy, Some(scripts));
        }
    }
}

/// The scripts `side` starts, in GetDescendants' order
fn eligible(engine: &Rc<Engine>, side: Side) -> Vec<InstanceId> {
    let reflection = &engine.reflection;
    let world = engine.world();
    let class = |name: &str| reflection.class_id(name);
    let (Some(script), Some(local)) = (class("Script"), class("LocalScript")) else { return Vec::new() };
    let run_context = reflection.name_id("RunContext");
    let item = |name: &str| reflection.enum_item("RunContext", name);
    let (legacy, server_context, client_context) = (item("Legacy"), item("Server"), item("Client"));
    let enabled = reflection.name_id("Enabled");
    let disabled = reflection.name_id("Disabled");
    let service = |name: &str| crate::services::find_service(engine, world.game, name);
    let (server_scripts, server_storage, first) = (service("ServerScriptService"), service("ServerStorage"), service("ReplicatedFirst"));
    let templates: Vec<InstanceId> = ["StarterGui", "StarterPack", "StarterPlayer"].iter().filter_map(|name| service(name)).collect();
    let containers: Vec<InstanceId> = match world.local_player {
        Some(player) => engine.state.borrow().dom.at(player).children.clone(),
        None => Vec::new(),
    };
    let state = engine.state.borrow();
    let under = |id: InstanceId, ancestor: Option<InstanceId>| ancestor.is_some_and(|ancestor| state.dom.is_descendant_of(id, ancestor));
    let mut out = Vec::new();
    for id in state.dom.descendants(world.game) {
        let instance = state.dom.at(id);
        if instance.class != script && instance.class != local {
            continue;
        }
        let on = enabled.and_then(|name| state.dom.boolean(id, name)).unwrap_or(true) && !disabled.and_then(|name| state.dom.boolean(id, name)).unwrap_or(false);
        if !on {
            continue;
        }
        let context = run_context.and_then(|name| state.dom.raw_get(id, name)).and_then(crate::Value::as_enum);
        let template = templates.iter().any(|&starter| state.dom.is_descendant_of(id, starter));
        let runs = match side {
            Side::Server => {
                instance.class == script
                    && (((context.is_none() || context == legacy) && (under(id, Some(world.workspace)) || under(id, server_scripts)))
                        || (context.is_some() && context == server_context && !under(id, server_storage)))
            }
            Side::Client => {
                if instance.class == local {
                    under(id, first) || containers.iter().any(|&container| state.dom.is_descendant_of(id, container))
                } else {
                    context.is_some() && context == client_context && !template && !under(id, server_storage)
                }
            }
        };
        if runs {
            out.push(id);
        }
    }
    out
}

/// Runs a script's source on a scheduler thread of its own, once (a compile error is the script's output)
fn start_script(lua: &Lua, engine: &Rc<Engine>, script: InstanceId) {
    if engine.modules.borrow().started(script) {
        return;
    }
    engine.modules.borrow_mut().mark_started(script);
    let Some(source) = crate::lua::modules::source_of(engine, script) else { return };
    let name = engine.state.borrow().dom.full_name(&engine.reflection, script);
    match crate::lua::modules::compile(lua, engine, script, &source) {
        Ok(chunk) => {
            if let Ok(thread) = lua.create_thread(chunk) {
                crate::scheduler::resume(lua, engine, &thread, LuaMultiValue::new());
            }
        }
        Err(error) => engine.output("error", format!("{name}: {}", crate::scheduler::error_message(&error)), None),
    }
}


/// The player leaves (Context.Leave), at a frame boundary: it leaves the server's Players (PlayerRemoving there), the
/// client steps no more, and the calls waiting on the other side raise.
pub fn leave(client_engine: &Rc<Engine>) -> Result<(), String> {
    let Some((server_lua, server)) = client_engine.peer_server() else { return Ok(()) };
    let Some(endpoint) = client_engine.loopback() else { return Ok(()) };
    let bridge = &endpoint.bridge;
    if bridge.left.get() || !bridge.joined.get() {
        return Ok(());
    }
    bridge.left.set(true);
    if let Some((server_player, _)) = bridge.player.get() {
        let _ = crate::lua::instance::set_parent(&server, server_player, None);
    }
    if let Some(server_end) = server.loopback() {
        remotes::player_left(&server_lua, &server, &server_end).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// One frame of the pair ([`crate::scheduler::step`] of a linked client): the server's frame, then the client's (once the
/// player joined and while it has not left), each followed by sealing what it sent. What the client sent between frames
/// (its harness's start) goes out first.
pub fn step_pair(lua: &Lua, client: &Rc<Engine>, duration: Option<f64>) -> LuaResult<()> {
    let Some((server_lua, server)) = client.peer_server() else { return crate::scheduler::step_one(lua, client, duration) };
    let Some(endpoint) = client.loopback() else { return crate::scheduler::step_one(lua, client, duration) };
    let bridge = endpoint.bridge.clone();
    if bridge.leave_asked.get() && !bridge.left.get() {
        leave(client).map_err(LuaError::runtime)?;
    }
    bridge.seal(Side::Client);
    crate::scheduler::step_one(&server_lua, &server, duration)?;
    replicate::flush(&server, &bridge);
    bridge.seal(Side::Server);
    if bridge.joined.get() && !bridge.left.get() {
        crate::scheduler::step_one(lua, client, duration)?;
        bridge.seal(Side::Client);
    }
    Ok(())
}

/// The receive point of a linked engine's frame (scheduler.rs, a play frame's start: after the legacy wait() resumptions'
/// due times are taken and before them, ahead of PreAnimation): what the other side sealed for this frame applies, in
/// order. Changes and events queue deferred work, which runs at the legacy step's flush (after the resumed threads, as
/// measured); a RemoteFunction's callback and an answered call resume now. Nothing due: one compare.
pub(crate) fn receive(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    let Some(endpoint) = engine.loopback() else { return Ok(()) };
    if endpoint.side == Side::Client && !endpoint.bridge.joined.get() {
        return Ok(());
    }
    let Some(messages) = endpoint.bridge.begin_frame(endpoint.side) else { return Ok(()) };
    apply(lua, engine, &endpoint, messages)
}

fn apply(lua: &Lua, engine: &Rc<Engine>, endpoint: &Rc<Endpoint>, messages: Vec<bridge::Msg>) -> LuaResult<()> {
    use bridge::Msg;
    for message in messages {
        match message {
            Msg::Event { remote, args } => remotes::deliver_event(lua, engine, endpoint, remote, &args)?,
            Msg::Invoke { remote, call, args } => remotes::deliver_invoke(lua, engine, endpoint, remote, call, args)?,
            Msg::Return { call, result } => remotes::deliver_return(lua, engine, endpoint, call, result)?,
            change => replicate::apply(lua, engine, endpoint, change)?,
        }
    }
    Ok(())
}

/// Releases the Lua values a linked engine's endpoint holds (the threads waiting on a call), before its VM goes
/// ([`Emulator`]'s drop); the client's also releases the server's.
pub fn release(engine: &Engine) {
    if let Some(endpoint) = engine.loopback.borrow_mut().take() {
        endpoint.remotes.release();
    }
    if let Some(link) = engine.peer.borrow().as_ref() {
        release(&link.server.engine);
    }
}

/// The line a summary reports of a loopback run (`summary.loopback`): its latency, the frame the player joined at, the
/// messages each way and what replicated.
pub fn summary(engine: &Engine) -> Option<serde_json::Value> {
    let endpoint = engine.loopback()?;
    let bridge = &endpoint.bridge;
    let stats = *bridge.stats.borrow();
    Some(serde_json::json!({
        "latency": bridge.latency,
        "joinedAt": bridge.joined_at.get(),
        "toClient": { "messages": stats.messages[Side::Server.index()] },
        "toServer": { "messages": stats.messages[Side::Client.index()] },
        "replicated": { "created": stats.created, "writes": stats.writes },
    }))
}

/// A client's name for the server's instance `id`, or the server's for the client's (tests, the recorder's side choice)
pub fn counterpart(engine: &Engine, id: InstanceId) -> Option<InstanceId> {
    let endpoint = engine.loopback()?;
    let ids = endpoint.bridge.ids.borrow();
    ids.map(endpoint.side.other(), id)
}
