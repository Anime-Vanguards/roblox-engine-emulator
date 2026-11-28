//! Scripts a running game starts and stops, as a 0.747 playing client and server do (measured in roblox-sim sessions, each
//! capture twice and the client and the server alike: tools/parity/probes/script_starts.luau and script_stops.luau, and one-off
//! rounds in docs/engine.md "Script starts and stops").
//!
//! - **Where a script runs.** On a client a LocalScript runs under ReplicatedFirst, the local player's PlayerGui, Backpack and
//!   PlayerScripts, and the local player's character; a Script whose RunContext is Client anywhere in the DataModel (StarterGui
//!   and StarterPack too). On a server a Script whose RunContext is Legacy runs under the Workspace, ServerScriptService and a
//!   player's PlayerGui and Backpack; one whose RunContext is Server anywhere in the DataModel (ServerStorage too). Nothing else
//!   runs: a ModuleScript, a LocalScript or Client Script on a server, a Legacy or Server Script on a client.
//! - **A start.** A script starts when it comes to run where it is while it is enabled: a Parent write that takes it (or a tree
//!   holding it) to where it runs, or an Enabled write there. The write loads it (its Source as it is then: a later write of
//!   the Source does not change what that start runs) and queues its start in the deferred queue, right after the script's own
//!   AncestryChanged among the write's events (a tree's scripts in its post-order, its children before itself); the start runs
//!   the script on a thread of its own at its turn (ree: [`run_start`]). Moving it between two places it runs changes nothing;
//!   a move out and back before its turn starts it once; a move out, an Enabled false or a Destroy before its turn, none.
//! - **A stop.** A script that runs stops when it leaves where it runs (Parent nil, a move, Destroy) or is disabled. What it had
//!   waiting at that moment never resumes (its waits, threads it spawned, deferred or delayed, a WaitForChild or an event's
//!   Wait, a handler that yielded, threads a module's function made for it): they stay suspended. Its connections' handlers do
//!   not run from then on; an Enabled false disconnects them at the next deferred point, and the other stops each when its
//!   signal next reaches it. The thread that stopped its own script goes on, and what it makes afterwards runs. A restart runs
//!   the script again from the top in a new environment.
//!
//! Which run a thread belongs to is the VM's thread owner tag (patches/luau/30-thread-script-globals.patch), which every thread
//! made from a script's thread inherits; a connection keeps the tag of the thread that made it. ree's boot state (the mount, a
//! loopback join's copy, a playing client's PlayerGui copies of StarterGui) is the harness's to start: the starts begin with
//! the run ([`open`]).

use std::os::raw::{c_int, c_uint, c_void};
use std::rc::Rc;

use mlua::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{ClassId, Engine, InstanceId, NameId, State, reflection::EnumItemId, reflection::Reflection, signal::ConnectionId, value::Value};

unsafe extern "C" {
    fn lua_setthreadglobals(state: *mut mlua::ffi::lua_State, thread: *mut mlua::ffi::lua_State);
    fn lua_getthreadglobals(state: *mut mlua::ffi::lua_State, thread: *mut mlua::ffi::lua_State) -> c_int;
    fn lua_threadglobalspointer(thread: *mut mlua::ffi::lua_State) -> *const c_void;
    fn lua_setthreadowner(thread: *mut mlua::ffi::lua_State, owner: c_uint);
    fn lua_getthreadowner(thread: *mut mlua::ffi::lua_State) -> c_uint;
}

/// The side whose scripts run here
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Client,
    Server,
}

#[derive(Clone, Copy)]
struct Names {
    script: ClassId,
    local_script: ClassId,
    player: ClassId,
    player_gui: ClassId,
    backpack: ClassId,
    player_scripts: ClassId,
    replicated_first: Option<ClassId>,
    server_script_service: Option<ClassId>,
    enabled: NameId,
    disabled: Option<NameId>,
    run_context: NameId,
    character: NameId,
    client: Option<EnumItemId>,
    server: Option<EnumItemId>,
    plugin: Option<EnumItemId>,
}

/// A script's start or run
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Run {
    /// Its start is queued (the key of its job)
    Pending(u32),
    /// It runs: the owner tag of its threads and connections
    Running(u32),
}

struct Pending {
    script: InstanceId,
    /// The Source it had at the write, when it has one in memory (a mounted file's is read at the start)
    source: Option<Rc<[u8]>>,
}

/// Why a run stops: an Enabled (or Disabled) write disconnects its connections at the next deferred point; the others leave
/// each to its signal's next fire
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Disabled,
    Left,
}

/// The scripts of one engine that a running game starts and stops (State::scripts)
#[derive(Default)]
pub struct Scripts {
    side: Option<Side>,
    names: Option<Names>,
    game: Option<InstanceId>,
    workspace: Option<InstanceId>,
    local_player: Option<InstanceId>,
    /// Starts happen from the run's start on ([`open`])
    open: bool,
    /// While above 0, scripts that come to run are left as the boot's ([`Quiet`])
    quiet: u32,
    runs: FxHashMap<InstanceId, Run>,
    /// The owner tags of the runs going now
    live: FxHashSet<u32>,
    pending: FxHashMap<u32, Pending>,
    /// The owner tags of runs that stopped: their threads never resume, their connections' handlers never run
    retired: FxHashSet<u32>,
    /// The connections each run made (a stop by Enabled false disconnects them)
    connections: FxHashMap<u32, Vec<ConnectionId>>,
    next: u32,
}

impl Scripts {
    fn tag(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    /// Whether a thread of `owner` is one of a stopped run's
    #[inline]
    pub fn retired(&self, owner: u32) -> bool {
        owner != 0 && !self.retired.is_empty() && self.retired.contains(&owner)
    }

    /// Whether any script has started here (a connection then keeps its maker's owner tag)
    #[inline]
    pub fn any_run(&self) -> bool {
        self.next > 0
    }

    /// Whether a change of the tree or of a script's Enabled can start or stop one now
    #[inline]
    fn watching(&self) -> bool {
        self.open && self.side.is_some() && self.names.is_some()
    }

    /// The owner tag of `script`'s run, if it runs
    pub fn owner_of(&self, script: InstanceId) -> Option<u32> {
        match self.runs.get(&script) {
            Some(Run::Running(owner)) => Some(*owner),
            _ => None,
        }
    }
}

/// A stretch of the engine's own boot work (a playing client's PlayerGui copies of StarterGui, a loopback join's copy): the
/// scripts it brings to where they run are the harness's to start, as the mount's are.
pub struct Quiet;

impl Quiet {
    pub fn enter(state: &mut State) -> Quiet {
        state.scripts.quiet += 1;
        Quiet
    }

    pub fn leave(self, state: &mut State) {
        state.scripts.quiet -= 1;
    }
}

/// The run context's side and the names the rules read (services::install)
pub fn install(engine: &Rc<Engine>) {
    let reflection = &engine.reflection;
    let options = &engine.options;
    let side = if options.edit_mode() {
        None
    } else if options.server() {
        Some(Side::Server)
    } else {
        Some(Side::Client)
    };
    let names = names(reflection);
    {
        let mut state = engine.state.borrow_mut();
        state.scripts.side = side;
        state.scripts.names = names;
    }
    let Some(names) = names else { return };
    // An Enabled or Disabled write of a script: a start where it runs, a stop where it ran
    engine.hooks.borrow_mut().property_changed.push(Box::new(move |state, id, name| {
        if (name == names.enabled || Some(name) == names.disabled) && state.scripts.watching() {
            enabled_changed(state, id, name, names);
        }
    }));
}

fn names(reflection: &Reflection) -> Option<Names> {
    Some(Names {
        script: reflection.class_id("Script")?,
        local_script: reflection.class_id("LocalScript")?,
        player: reflection.class_id("Player")?,
        player_gui: reflection.class_id("PlayerGui")?,
        backpack: reflection.class_id("Backpack")?,
        player_scripts: reflection.class_id("PlayerScripts")?,
        replicated_first: reflection.class_id("ReplicatedFirst"),
        server_script_service: reflection.class_id("ServerScriptService"),
        enabled: reflection.name_id("Enabled")?,
        disabled: reflection.name_id("Disabled"),
        run_context: reflection.name_id("RunContext")?,
        character: reflection.name_id("Character")?,
        client: reflection.enum_item("RunContext", "Client"),
        server: reflection.enum_item("RunContext", "Server"),
        plugin: reflection.enum_item("RunContext", "Plugin"),
    })
}

/// The DataModel, the Workspace and the local player, once the DataModel has them (Engine::set_world)
pub fn set_world(state: &mut State, game: InstanceId, workspace: InstanceId, local_player: Option<InstanceId>) {
    let scripts = &mut state.scripts;
    scripts.game = Some(game);
    scripts.workspace = Some(workspace);
    scripts.local_player = local_player;
}

/// The run starts (a harness's call, a loopback side's boot): scripts that come to run from now on start, and those that leave
/// stop. What the mount put where scripts run is the harness's to start.
pub fn open(state: &mut State) {
    state.scripts.open = true;
}

/// What a script needs above it to run on this side
#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    /// A LocalScript: ReplicatedFirst, the local player's containers or character
    Local,
    /// A Legacy Script on a server: the Workspace, ServerScriptService, a player's PlayerGui or Backpack
    Legacy,
    /// A Client Script on a client, a Server Script on a server: the DataModel
    Anywhere,
}

fn need(state: &State, names: &Names, side: Side, script: InstanceId) -> Option<Need> {
    let class = state.dom.at(script).class;
    if class == names.local_script {
        return (side == Side::Client).then_some(Need::Local);
    }
    if class != names.script {
        return None;
    }
    let context = state.dom.raw_get(script, names.run_context).and_then(Value::as_enum);
    // (a Plugin Script runs in a plugin alone: a playing client and server start none, in any container)
    if context.is_some() && context == names.plugin {
        return None;
    }
    if context.is_some() && context == names.client {
        (side == Side::Client).then_some(Need::Anywhere)
    } else if context.is_some() && context == names.server {
        (side == Side::Server).then_some(Need::Anywhere)
    } else {
        (side == Side::Server).then_some(Need::Legacy)
    }
}

/// Whether `script` is where it runs on this side. With `above`, as it was before a move of `root` (one of its ancestors or
/// itself) from under the chain `above` (the old parent first, up to its root): the parents below `root` did not change.
fn located(state: &State, script: InstanceId, above: Option<(InstanceId, &[InstanceId])>) -> bool {
    let scripts = &state.scripts;
    let (Some(names), Some(side), Some(game)) = (scripts.names, scripts.side, scripts.game) else { return false };
    let Some(need) = need(state, &names, side, script) else { return false };
    let character = match (need, scripts.local_player) {
        (Need::Local, Some(player)) if state.dom.get(player).is_some() => state.dom.raw_get(player, names.character).and_then(Value::as_instance),
        _ => None,
    };
    // (whether `at`, whose parent is `parent`, is a container the script runs under)
    let runs_under = |at: InstanceId, parent: Option<InstanceId>| -> bool {
        let class = state.dom.at(at).class;
        match need {
            Need::Anywhere => false,
            Need::Local => {
                (Some(class) == names.replicated_first)
                    || Some(at) == character
                    || ((class == names.player_gui || class == names.backpack || class == names.player_scripts)
                        && scripts.local_player.is_some()
                        && parent == scripts.local_player)
            }
            Need::Legacy => {
                Some(at) == scripts.workspace
                    || Some(class) == names.server_script_service
                    || ((class == names.player_gui || class == names.backpack)
                        && parent.is_some_and(|parent| state.dom.at(parent).class == names.player))
            }
        }
    };
    // (the script's ancestors, nearest first, as they are or were: the tree's own up to its root, then the root's chain)
    let (root, chain) = match above {
        Some((root, chain)) => (Some(root), chain),
        None => (None, &[][..]),
    };
    let mut at = if root == Some(script) { chain.first().copied() } else { state.dom.at(script).parent };
    let mut in_chain = root == Some(script);
    let mut index = 0;
    while let Some(node) = at {
        if node == game {
            return need == Need::Anywhere;
        }
        let parent = if in_chain {
            chain.get(index + 1).copied()
        } else if root == Some(node) {
            chain.first().copied()
        } else {
            state.dom.at(node).parent
        };
        if runs_under(node, parent) {
            return true;
        }
        if in_chain {
            index += 1;
        } else if root == Some(node) {
            in_chain = true;
            index = 0;
        }
        at = parent;
    }
    false
}

fn enabled(state: &State, names: &Names, script: InstanceId) -> bool {
    state.dom.boolean(script, names.enabled) != Some(false) && !names.disabled.is_some_and(|name| state.dom.boolean(script, name) == Some(true))
}

fn is_script(state: &State, names: &Names, id: InstanceId) -> bool {
    let class = state.dom.at(id).class;
    class == names.script || class == names.local_script
}

/// Whether a move of `root` can start or stop a script: one may be in its tree (instance.rs reparent, before it walks the tree)
#[inline]
pub fn watching_move(state: &State, root: InstanceId) -> bool {
    let scripts = &state.scripts;
    if !scripts.watching() {
        return false;
    }
    let Some(names) = scripts.names else { return false };
    let instance = state.dom.at(root);
    !instance.children.is_empty() || instance.class == names.script || instance.class == names.local_script
}

/// `node` of a tree whose root just moved from under `old_chain` (instance.rs reparent: after the node's AncestryChanged, in the
/// tree's post-order): a script that came to run starts, one that left stops.
pub fn moved(state: &mut State, node: InstanceId, root: InstanceId, old_chain: &[InstanceId]) {
    let Some(names) = state.scripts.names else { return };
    if !is_script(state, &names, node) {
        return;
    }
    let on = state.dom.get(node).is_some() && enabled(state, &names, node);
    let after = on && located(state, node, None);
    let before = on && located(state, node, Some((root, old_chain)));
    if after == before {
        return;
    }
    if after {
        start(state, node);
    } else {
        stop(state, node, Stop::Left);
    }
}

fn enabled_changed(state: &mut State, id: InstanceId, name: NameId, names: Names) {
    if state.dom.get(id).is_none() || !is_script(state, &names, id) {
        return;
    }
    let Some(value) = state.dom.boolean(id, name) else { return };
    // (the write changed the value: before it, the other one)
    let now = enabled(state, &names, id);
    let before = if name == names.enabled {
        !value && !names.disabled.is_some_and(|disabled| state.dom.boolean(id, disabled) == Some(true))
    } else {
        state.dom.boolean(id, names.enabled) != Some(false) && value
    };
    if now == before || !located(state, id, None) {
        return;
    }
    if now {
        start(state, id);
    } else {
        stop(state, id, Stop::Disabled);
    }
}

/// Queues `script`'s start (a write took it to where it runs): its Source as it is now, the write's price
fn start(state: &mut State, script: InstanceId) {
    if state.scripts.quiet > 0 {
        return;
    }
    if state.scripts.runs.contains_key(&script) {
        return;
    }
    let key = state.scripts.tag();
    let source = state.dom.at(script).source().cloned();
    state.scripts.pending.insert(key, Pending { script, source });
    state.scripts.runs.insert(script, Run::Pending(key));
    state.sched.defer(crate::scheduler::JobTarget::Start(key), Vec::new());
    // (the write loads the script: a client's price, beyond the write's own: cost.rs SCRIPT_START_QUEUED)
    let price = crate::cost::SCRIPT_START_QUEUED[usize::from(state.scripts.side == Some(Side::Server))];
    state.sched.charge(price);
    crate::boundary::add_cost(price);
}

/// `script` no longer runs where it is: its queued start is dropped, or its run stops
fn stop(state: &mut State, script: InstanceId, why: Stop) {
    let Some(run) = state.scripts.runs.remove(&script) else { return };
    match run {
        Run::Pending(key) => {
            state.scripts.pending.remove(&key);
        }
        Run::Running(owner) => retire(state, owner, why),
    }
}

/// A run stops: its owner tag retires (its threads waiting never resume, its handlers never run), the threads running now that
/// are its keep going under a tag of their own, and an Enabled false disconnects its connections at the next deferred point.
fn retire(state: &mut State, owner: u32, why: Stop) {
    state.scripts.live.remove(&owner);
    state.scripts.retired.insert(owner);
    let survivor = state.scripts.tag();
    state.sched.visit_resuming(&mut |thread| {
        // SAFETY: a thread the scheduler is resuming now (on its resume stack): alive, its fields readable and writable
        unsafe {
            if lua_getthreadowner(thread) == owner {
                lua_setthreadowner(thread, survivor);
            }
        }
    });
    let connections = state.scripts.connections.remove(&owner).unwrap_or_default();
    let live = connections.iter().filter(|&&connection| state.signals.owner(connection) == Some(owner)).count();
    let price = crate::cost::script_stopped(live, state.scripts.side == Some(Side::Server));
    state.sched.charge(price);
    crate::boundary::add_cost(price);
    if why == Stop::Disabled && live > 0 {
        let key = state.scripts.tag();
        state.scripts.connections.insert(key, connections);
        state.sched.defer(crate::scheduler::JobTarget::DisconnectRun(key), Vec::new());
    }
}

/// JobTarget::DisconnectRun: the connections a run that an Enabled false stopped had made
pub fn disconnect_run(state: &mut State, key: u32) {
    let Some(connections) = state.scripts.connections.remove(&key) else { return };
    for connection in connections {
        if state.signals.owner(connection).is_some_and(|owner| state.scripts.retired(owner)) {
            state.signals.disconnect(connection);
        }
    }
}

/// A connection a script's thread made: it keeps the thread's owner tag (lua/types.rs connect)
pub fn connected(lua: &Lua, state: &mut State, connection: ConnectionId) {
    if !state.scripts.any_run() || connection == ConnectionId::DISCONNECTED {
        return;
    }
    let owner = owner(&lua.current_thread());
    if owner == 0 {
        return;
    }
    state.signals.set_owner(connection, owner);
    if state.scripts.live.contains(&owner) {
        let signals = &state.signals;
        let list = state.scripts.connections.entry(owner).or_default();
        // (a script that connects and disconnects for ever keeps a list of what still holds its tag)
        if list.len() >= 64 && list.len().is_power_of_two() {
            list.retain(|&kept| signals.owner(kept) == Some(owner));
        }
        list.push(connection);
    }
}

/// A handler of `connection` is to run: false when its connection is a stopped run's, which it disconnects (scheduler.rs
/// run_job, instance.rs fire_immediate)
pub fn handler_may_run(state: &mut State, connection: ConnectionId) -> bool {
    if state.scripts.retired.is_empty() {
        return true;
    }
    match state.signals.owner(connection) {
        Some(owner) if state.scripts.retired(owner) => {
            state.signals.disconnect(connection);
            false
        }
        _ => true,
    }
}

/// The owner tag of the harness's threads in a run that gives the harness's own code an identity of its own (`--identity 4`,
/// Options::identity; record/run.rs). No script's run has it, so they never stop; the threads they make and the handlers they
/// connect take it as a run's do (measured: what code at identity 4 spawns, defers, delays and wraps, and its handlers whoever
/// fires them, keep identity 4). A module's chunk and a script's run take theirs (a game script's identity, measured likewise:
/// tools/parity/probes/identity_four.luau).
pub const HARNESS: u32 = 1 << 31;

/// Whether the running thread `state` is the harness's own code at the run's harness identity ([`HARNESS`])
pub fn harness_identity(state: *mut mlua::ffi::lua_State) -> bool {
    // SAFETY: the running thread of the VM
    tagged() && unsafe { lua_getthreadowner(state) } == HARNESS
}

/// Whether the engine gave any thread an owner tag or script globals yet (until then no thread has one to pass on)
static TAGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn tagged() -> bool {
    TAGGED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether `thread` is a script's (an owner tag or script globals): its connections keep it as their connector
pub fn scripted(thread: &LuaThread) -> bool {
    // SAFETY: a live thread of the VM
    tagged() && unsafe { lua_getthreadowner(thread.state()) != 0 || !lua_threadglobalspointer(thread.state()).is_null() }
}

/// The thread that made `connection`, when its handler threads are to take its tag and globals
pub fn maker(engine: &Engine, connection: ConnectionId) -> Option<LuaThread> {
    if !tagged() {
        return None;
    }
    engine.state.borrow().signals.connector(connection)
}

/// [`adopt`] from `from`, or, with none, takes away what `thread` inherited from the thread that happened to make it
pub fn adopt_if(lua: &Lua, thread: &LuaThread, from: Option<&LuaThread>) {
    if from.is_some() || scripted(thread) {
        adopt(lua, thread, from);
    }
}

/// A thread's owner tag (0: no script run's)
pub fn owner(thread: &LuaThread) -> u32 {
    // SAFETY: a live thread of the VM
    unsafe { lua_getthreadowner(thread.state()) }
}

/// Whether the scheduler may resume `thread`: not when it is a stopped run's (scheduler.rs resume_core)
#[inline]
pub fn may_resume(state: &State, thread: &LuaThread) -> bool {
    state.scripts.retired.is_empty() || !state.scripts.retired(owner(thread))
}

/// Gives `thread` the owner tag and script globals `from` has (a handler's thread its connecting thread's, a callback's thread
/// its setter's), or none
pub fn adopt(lua: &Lua, thread: &LuaThread, from: Option<&LuaThread>) {
    let target = thread.state();
    // SAFETY: both threads are live threads of the VM; the globals are pushed onto the running thread and popped by the set
    unsafe {
        match from {
            Some(from) => {
                lua_setthreadowner(target, lua_getthreadowner(from.state()));
                let source = from.state();
                let _ = lua.exec_raw::<()>((), |state| {
                    lua_getthreadglobals(state, source);
                    lua_setthreadglobals(state, target);
                });
            }
            None => {
                lua_setthreadowner(target, 0);
                let _ = lua.exec_raw::<()>((), |state| {
                    mlua::ffi::lua_pushnil(state);
                    lua_setthreadglobals(state, target);
                });
            }
        }
    }
}

/// Gives `thread` `globals` as its script globals (getfenv(0), loadstring's environment) and `owner` as its tag
pub fn set_thread(lua: &Lua, thread: &LuaThread, globals: Option<&LuaTable>, owner: u32) {
    if globals.is_none() && owner == 0 && !scripted(thread) {
        return;
    }
    TAGGED.store(true, std::sync::atomic::Ordering::Relaxed);
    let target = thread.state();
    // SAFETY: a live thread of the VM; the table is pushed onto the running thread and popped by the set
    unsafe {
        lua_setthreadowner(target, owner);
        let _ = match globals {
            Some(globals) => lua.exec_raw::<()>(globals, |state| lua_setthreadglobals(state, target)),
            None => lua.exec_raw::<()>((), |state| {
                mlua::ffi::lua_pushnil(state);
                lua_setthreadglobals(state, target);
            }),
        };
    }
}

/// The running thread's script globals, when the engine gave it (or the thread that made it) some
pub fn thread_globals(lua: &Lua) -> Option<LuaTable> {
    let thread = lua.current_thread();
    // SAFETY: the running thread's state; a null pointer means none
    if unsafe { lua_threadglobalspointer(thread.state()) }.is_null() {
        return None;
    }
    let source = thread.state();
    // SAFETY: pushes the table onto the running thread, which exec_raw takes as its result
    unsafe { lua.exec_raw::<LuaValue>((), |state| {
        lua_getthreadglobals(state, source);
    }) }
    .ok()
    .and_then(|value| match value {
        LuaValue::Table(table) => Some(table),
        _ => None,
    })
}

/// JobTarget::Start: the queued start of a script, at its turn in the deferred queue: its chunk (compiled in its Actor's VM when it
/// is under one) on a thread of its own, its owner tag and its environment as the thread's globals. A source that does not
/// compile logs the compiler's message once, with no stack (measured: a 0.747 client and server, as for a module).
pub fn run_start(lua: &Lua, engine: &Rc<Engine>, key: u32) -> LuaResult<()> {
    let pending = {
        let mut state = engine.state.borrow_mut();
        let Some(pending) = state.scripts.pending.remove(&key) else { return Ok(()) };
        if state.scripts.runs.get(&pending.script) != Some(&Run::Pending(key)) || state.dom.get(pending.script).is_none() {
            return Ok(());
        }
        let owner = state.scripts.tag();
        state.scripts.runs.insert(pending.script, Run::Running(owner));
        state.scripts.live.insert(owner);
        (pending, owner)
    };
    let (Pending { script, source }, owner) = pending;
    let source = match source {
        Some(source) => source.to_vec(),
        None => match crate::lua::modules::source_of(engine, script) {
            Some(source) => source,
            None => return Ok(()),
        },
    };
    let vm = crate::lua::actors::actor_of(engine, script).map(crate::lua::modules::vm_key);
    match crate::lua::modules::compile_in(lua, engine, script, &source, vm) {
        Ok(chunk) => {
            let globals = chunk.environment();
            let thread = lua.create_thread(chunk)?;
            set_thread(lua, &thread, globals.as_ref(), owner);
            crate::lua::globals::memory_category_from(lua, &thread, None);
            crate::scheduler::resume(lua, engine, &thread, LuaMultiValue::new());
        }
        Err(error) => engine.output_error_with_script(crate::scheduler::error_message(&error), None, None, false),
    }
    Ok(())
}

/// A script the engine starts at once, outside the queue (a loopback side's boot): the same run as [`run_start`]'s
pub fn start_now(lua: &Lua, engine: &Rc<Engine>, script: InstanceId) -> LuaResult<()> {
    let key = {
        let mut state = engine.state.borrow_mut();
        if state.scripts.runs.contains_key(&script) {
            return Ok(());
        }
        let key = state.scripts.tag();
        state.scripts.pending.insert(key, Pending { script, source: None });
        state.scripts.runs.insert(script, Run::Pending(key));
        key
    };
    run_start(lua, engine, key)
}
