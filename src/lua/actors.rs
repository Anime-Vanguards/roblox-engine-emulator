//! Actors: the scripts under an Actor run in a VM of their own, as in Studio, and the Actor's messages reach them.
//!
//! An Actor's scripts have their own `_G` and their own copy of every module they require (modules.rs
//! `Vm`, modules.rs `environment_in`). A script under an Actor starts as any script does, at its turn in the deferred queue
//! after the write that put it there or enabled it (scripts.rs; measured in a 0.747 playing client and server,
//! SignalBehavior Deferred: a script parented into an Actor in the Workspace, and an Actor holding one parented to the
//! Workspace, start after the write's ChildAdded handlers, tools/parity/probes/actor_messages.luau and script_starts.luau; a
//! Studio session in Immediate mode started it inside the assignment), in its Actor's VM, and stops as any script does.
//!
//! Parallel work runs at a frame event's parallel point ([`parallel_point`]): after the event's serial handlers and the flush of
//! the deferred work they left, and before the flush that follows (measured in a 0.747 playing client and server,
//! tools/parity/probes/actor_messages.luau): after PreRender (its handlers, the tween step and that resumption point), PreAnimation,
//! PreSimulation, PostSimulation and Heartbeat, whether or not any ConnectParallel handler is connected there. The work is the
//! `ConnectParallel` handlers of signals that fired since the last point, the messages of `BindToMessageParallel` bindings, and the
//! threads of an Actor's scripts that called `task.desynchronize()` in serial code (a script's first lines, a serial handler: they
//! yield and resume there). Studio runs the Actors' threads at the same time; ree runs them one after the other on the one Luau state,
//! in Studio's order: every parallel thread runs up to its first `task.synchronize()`, then the synchronized threads run serially in
//! that order up to their next `task.desynchronize()`, then the desynchronized ones, and so on until all have finished (the order a
//! Studio client printed for two Actors: both "parallel", both "synchronized", both "desynchronized"). Parallel messages sent during
//! that (from any part, a synchronized one too) run in another round of the same point, as often as they are sent (a chain of 100
//! ran in one point); the flush after the rounds runs the serial work they left, and what that flush sends in parallel waits for the
//! next point. `task.synchronize` in serial code does nothing; outside an Actor both raise (`task.synchronize() may only be called
//! from a script that is a descendant of an Actor`, also where the thread cannot yield: measured in a 0.747 playing client and a
//! server, tools/parity/probes/yield_refused.luau). Results are the same as Studio's as long as no Actor reads what another writes in
//! the same step (Studio does not order Actors against each other: its order among Actors varies from frame to frame).
//!
//! Messages (`Actor:SendMessage(topic, ...)`, `BindToMessage`, `BindToMessageParallel`; measured in a 0.747 playing client and server,
//! tools/parity/probes/actor_messages.luau, actor_message_args.luau):
//! - Each topic of an Actor is a signal of its own (diagnostics `Actor.ActorMessageEvent`); a binding is a connection of it, newest
//!   first, and the bindings are taken when the message is sent: a topic bound afterwards gets nothing, and a message to a topic
//!   no script bound (or to an Actor with no script) goes nowhere, silently. A binding disconnected before its turn is skipped.
//! - A serial binding's handler runs as a deferred event's does (a handler thread reused while its runs return, the event
//!   re-entrancy cut at 80 shared with task.defer: a handler that sends to itself runs 80 times in a frame, then `Maximum event
//!   re-entrancy depth exceeded for Actor.ActorMessageEvent when calling ...` is logged), one deferred step later than an event
//!   fired at the same place: the send queues a deferred job that queues the handler ([`JobTarget::Post`]), so a message sent at a
//!   resumption point (task.wait's step, which runs one pass of the queue) is handled at the next full flush, ahead of what that
//!   pass left queued, and one sent in a handler runs after the deferred work queued beside it. Sent from a parallel thread, the
//!   handler is queued at once.
//! - A parallel binding's handler runs at the parallel point, on a new thread.
//! - The arguments are copied as BindableEvent:Fire copies them (lua/bindable.rs: tables copied at the send and again for each
//!   binding, cycles and boolean keys refused at the send). The topic is a string parameter (a number as its text, any other value
//!   after a warning). `SendMessage` raises `Actor:SendMessage may only be called on Actors attached to the DataModel.` for an Actor
//!   outside the DataModel; `BindToMessage` / `BindToMessageParallel` raise `Calling BindToMessage on an Actor is only allowed in
//!   scripts that are a descendent of that Actor` unless the calling code runs in that Actor's VM (its scripts and the modules they
//!   require, wherever those are; not a nested Actor's scripts).

use std::collections::VecDeque;
use std::rc::Rc;

use mlua::prelude::*;
use rustc_hash::FxHashMap;

use super::{Args, Flow, Ret, modules::vm_key, none, raise, values};
use crate::signal::{ConnectionId, SignalId};
use crate::value::{Str, Value};
use crate::{Engine, InstanceId};

/// The Actor `id` is or is under, the nearest.
pub fn actor_of(engine: &Engine, id: InstanceId) -> Option<InstanceId> {
    let actor = engine.class_id("Actor")?;
    let state = engine.state.borrow();
    let mut at = Some(id);
    while let Some(current) = at {
        if state.dom.class_of(current) == actor {
            return Some(current);
        }
        at = state.dom.parent_of(current);
    }
    None
}

/// Parallel work waiting for the next parallel point ([`parallel_point`]).
enum Work {
    /// A ConnectParallel handler's thread, made when its signal fired, with the fire's arguments
    Handler(LuaThread, LuaMultiValue),
    /// A BindToMessageParallel binding's message: the binding and the message's arguments (copied when it was sent)
    Message(ConnectionId, Vec<Value>),
    /// A thread of an Actor's script that called task.desynchronize in serial code
    Resume(LuaThread),
}

/// Where the threads of a parallel phase stand.
#[derive(Default)]
pub struct Parallel {
    /// Threads of running parallel phases: true while the thread is in the parallel part of its callback
    modes: FxHashMap<usize, bool>,
    /// The phases running (a parallel handler can fire a signal with parallel handlers): the innermost last
    phases: Vec<Phase>,
    /// What the next parallel point runs, in the order it came
    pending: Vec<Work>,
    /// What a function sent to another VM arrives as ([`foreign_function`])
    foreign: Option<LuaFunction>,
}

#[derive(Default)]
struct Phase {
    /// Threads that called task.synchronize, in the order they did
    synchronized: VecDeque<LuaThread>,
    /// Threads that called task.desynchronize, in the order they did
    desynchronized: VecDeque<LuaThread>,
}

fn key(thread: &LuaThread) -> usize {
    thread.to_pointer() as usize
}

/// task.synchronize (`to_serial`) or task.desynchronize of the running thread: true when it must yield now, because it is a
/// parallel handler's thread of a running phase that is leaving its part for the other, or a thread of an Actor's script that
/// desynchronizes in serial code (it resumes at the next parallel point). Where the thread cannot yield it raises `thread is not
/// yieldable` and stays in its part (by the rule measured for the engine's other yielding calls, lua/mod.rs NOT_YIELDABLE; not
/// measured in an Actor).
pub fn park(lua: &Lua, engine: &Engine, to_serial: bool) -> Result<bool, Flow> {
    let thread = lua.current_thread();
    let mut modules = engine.modules.borrow_mut();
    let parallel = &mut modules.parallel;
    let Some(&in_parallel) = parallel.modes.get(&key(&thread)) else {
        drop(modules);
        if !caller_in_actor(lua) {
            let name = if to_serial { "synchronize" } else { "desynchronize" };
            return super::raise_plain(format!("task.{name}() may only be called from a script that is a descendant of an Actor"));
        }
        if to_serial {
            return Ok(false);
        }
        // (serial code of an Actor's script going parallel: measured, a script's first lines and serial message and event handlers
        // resume at the next parallel point)
        if !super::yieldable(&thread) {
            return super::raise_plain(super::NOT_YIELDABLE);
        }
        engine.modules.borrow_mut().parallel.pending.push(Work::Resume(thread));
        return Ok(true);
    };
    let Some(phase) = parallel.phases.last_mut() else { return Ok(false) };
    if in_parallel != to_serial {
        return Ok(false);
    }
    if !super::yieldable(&thread) {
        drop(modules);
        return super::raise_plain(super::NOT_YIELDABLE);
    }
    parallel.modes.insert(key(&thread), !to_serial);
    if to_serial {
        phase.synchronized.push_back(thread);
    } else {
        phase.desynchronized.push_back(thread);
    }
    Ok(true)
}

/// Whether the running thread is in the parallel part of a parallel phase (desynchronized).
fn in_parallel_part(lua: &Lua, engine: &Engine) -> bool {
    engine.modules.borrow().parallel.modes.get(&key(&lua.current_thread())).copied().unwrap_or(false)
}

/// A ConnectParallel handler of a fire: it runs at the next parallel point (instance.rs fire_immediate, scheduler.rs run_job).
pub fn defer_parallel(engine: &Engine, thread: LuaThread, values: LuaMultiValue) {
    engine.modules.borrow_mut().parallel.pending.push(Work::Handler(thread, values));
}

/// Whether parallel work waits for the next parallel point.
pub fn parallel_waiting(engine: &Engine) -> bool {
    !engine.modules.borrow().parallel.pending.is_empty()
}

/// How many rounds one parallel point runs before it leaves the rest to the next point (ree's guard against parallel work that
/// sends itself forever, which a client runs until it stops responding)
const MAX_ROUNDS: usize = 100_000;

/// A frame event's parallel point, after its serial handlers and the flush of their deferred work: the parallel work waiting
/// ([`Work`]) in rounds until none is left, then the flush of what it left. Nothing at all when nothing waits.
pub fn parallel_point(lua: &Lua, engine: &Rc<Engine>) -> LuaResult<()> {
    if !parallel_waiting(engine) {
        return Ok(());
    }
    let mut rounds = 0;
    loop {
        let work = std::mem::take(&mut engine.modules.borrow_mut().parallel.pending);
        if work.is_empty() {
            break;
        }
        rounds += 1;
        if rounds > MAX_ROUNDS {
            engine.output("error", format!("ree: parallel work went on for {MAX_ROUNDS} rounds of one parallel phase (an Actor's message sending itself?); the rest waits for the next phase"), None);
            engine.modules.borrow_mut().parallel.pending.splice(0..0, work);
            break;
        }
        let mut threads = Vec::with_capacity(work.len());
        for item in work {
            match item {
                Work::Handler(thread, values) => threads.push((thread, values)),
                Work::Resume(thread) => threads.push((thread, LuaMultiValue::new())),
                Work::Message(connection, args) => {
                    if !crate::scripts::handler_may_run(&mut engine.state.borrow_mut(), connection) {
                        continue;
                    }
                    let maker = crate::scripts::maker(engine, connection);
                    let begun = engine.state.borrow_mut().signals.begin_handler(connection);
                    if let Some((callback, _)) = begun {
                        let mut values = LuaMultiValue::with_capacity(args.len());
                        for value in &args {
                            values.push_back(super::convert::to_lua(lua, engine, value)?);
                        }
                        let thread = lua.create_thread(callback)?;
                        crate::scripts::adopt_if(lua, &thread, maker.as_ref());
                        threads.push((thread, values));
                    }
                }
            }
        }
        if !threads.is_empty() {
            parallel_phase(lua, engine, threads);
        }
    }
    crate::scheduler::flush_deferred(lua, engine)
}

/// One round of a parallel point: `threads` (with their arguments) run their parallel parts, then their synchronized parts, and so
/// on until all have finished.
fn parallel_phase(lua: &Lua, engine: &Rc<Engine>, threads: Vec<(LuaThread, LuaMultiValue)>) {
    {
        let mut modules = engine.modules.borrow_mut();
        modules.parallel.phases.push(Phase::default());
        for (thread, _) in &threads {
            modules.parallel.modes.insert(key(thread), true);
        }
    }
    for (thread, values) in &threads {
        crate::scheduler::resume(lua, engine, thread, values.clone());
    }
    loop {
        let (synchronized, desynchronized) = {
            let mut modules = engine.modules.borrow_mut();
            let phase = modules.parallel.phases.last_mut().expect("a parallel phase");
            (std::mem::take(&mut phase.synchronized), std::mem::take(&mut phase.desynchronized))
        };
        if synchronized.is_empty() && desynchronized.is_empty() {
            break;
        }
        for thread in synchronized.into_iter().chain(desynchronized) {
            crate::scheduler::resume(lua, engine, &thread, LuaMultiValue::new());
        }
    }
    let mut modules = engine.modules.borrow_mut();
    modules.parallel.phases.pop();
    for (thread, _) in &threads {
        modules.parallel.modes.remove(&key(thread));
    }
}

/// The VM the running thread's innermost script frame runs in (C functions and the engine's own Luau skipped): its environment's
/// metatable holds an Actor VM's key (modules.rs `environment_in`, builtins.rs `VM_FIELD`), Some(None) for the main VM's scripts,
/// None when no script frame runs.
fn caller_vm(lua: &Lua) -> Option<Option<u64>> {
    use mlua::ffi;
    // SAFETY: the running thread's state, valid while the callback runs; each frame's function is pushed and popped again
    unsafe {
        let state = lua.state();
        let mut debug: ffi::lua_Debug = std::mem::zeroed();
        let mut level = 0;
        while level < 128 && ffi::lua_getinfo(state, level, c"sf".as_ptr(), &mut debug) != 0 {
            level += 1;
            let script = !debug.source.is_null() && {
                let source = std::ffi::CStr::from_ptr(debug.source).to_string_lossy();
                let chunk = source.strip_prefix('=').or_else(|| source.strip_prefix('@')).unwrap_or(&source);
                chunk != "[C]" && !chunk.starts_with("rur.") && !chunk.starts_with("ree.")
            };
            if !script {
                ffi::lua_pop(state, 1);
                continue;
            }
            ffi::lua_getfenv(state, -1);
            let mut vm = None;
            if ffi::lua_type(state, -1) == ffi::LUA_TTABLE && ffi::lua_getmetatable(state, -1) != 0 {
                ffi::lua_rawgetfield(state, -1, super::builtins::VM_FIELD.as_ptr());
                if ffi::lua_type(state, -1) == ffi::LUA_TNUMBER {
                    vm = Some(ffi::lua_tonumberx(state, -1, std::ptr::null_mut()) as u64);
                }
                ffi::lua_pop(state, 2);
            }
            ffi::lua_pop(state, 2);
            return Some(vm);
        }
        None
    }
}

/// Whether the running thread's innermost script frame (C functions and the engine's own Luau skipped) runs in an Actor's VM, whose
/// environments' metatable holds its key (modules.rs `environment_in`): the calling script is one under an Actor, or a module such a
/// script required.
pub fn caller_in_actor(lua: &Lua) -> bool {
    matches!(caller_vm(lua), Some(Some(_)))
}

/// What a client raises when code calls a function another VM sent (measured, a 0.747 playing client: a function sent from a
/// game script to an Actor's script arrives as a function, and calling it raises this, without a position)
const FOREIGN: &str = "Attempt to load a function from a different Lua VM";

/// The function a function sent to another VM arrives as: one that raises [`FOREIGN`] when called.
fn foreign_function(lua: &Lua, engine: &Engine) -> LuaResult<LuaFunction> {
    if let Some(function) = engine.modules.borrow().parallel.foreign.clone() {
        return Ok(function);
    }
    let function = super::library_function(lua, "function", |_, _| super::raise_plain(FOREIGN))?;
    engine.modules.borrow_mut().parallel.foreign = Some(function.clone());
    Ok(function)
}

/// A sent value as another VM receives it: each function in it, at top level or in its tables, the [`foreign_function`].
fn across(lua: &Lua, engine: &Engine, value: Value) -> LuaResult<Value> {
    use super::bindable::{Copied, Sent};
    fn has_function(sent: &Sent) -> bool {
        match sent {
            Sent::Value(value) => matches!(value, LuaValue::Function(_)),
            Sent::Buffer(_) => false,
            Sent::Table(entries) => entries.iter().any(|(_, item)| has_function(item)),
        }
    }
    fn replaced(sent: &Sent, foreign: &LuaFunction) -> Sent {
        match sent {
            Sent::Value(LuaValue::Function(_)) => Sent::Value(LuaValue::Function(foreign.clone())),
            Sent::Value(value) => Sent::Value(value.clone()),
            Sent::Buffer(bytes) => Sent::Buffer(bytes.clone()),
            Sent::Table(entries) => Sent::Table(entries.iter().map(|(key, item)| (match key {
                super::bindable::Key::Index(index) => super::bindable::Key::Index(*index),
                super::bindable::Key::Name(name) => super::bindable::Key::Name(name.clone()),
            }, replaced(item, foreign))).collect()),
        }
    }
    let Value::Lua(held) = &value else { return Ok(value) };
    match &**held {
        LuaValue::Function(_) => Ok(Value::Lua(Box::new(LuaValue::Function(foreign_function(lua, engine)?)))),
        LuaValue::UserData(userdata) => {
            let sent = match userdata.borrow::<Copied>() {
                Ok(copied) if has_function(&copied.0) => copied.0.clone(),
                _ => return Ok(value),
            };
            let foreign = foreign_function(lua, engine)?;
            let userdata = lua.create_userdata(Copied(Rc::new(replaced(&sent, &foreign))))?;
            Ok(Value::Lua(Box::new(LuaValue::UserData(userdata))))
        }
        _ => Ok(value),
    }
}

/// The signal of an Actor's message topic, made on first use when `create`.
fn topic_signal(engine: &Engine, actor: InstanceId, topic: &[u8], create: bool) -> Option<SignalId> {
    let mut state = engine.state.borrow_mut();
    if let Some(signal) = state.dom.at(actor).signals.as_ref().and_then(|signals| signals.message(topic)) {
        return Some(signal);
    }
    if !create {
        return None;
    }
    // (the depth cut names it "Actor.ActorMessageEvent": measured, a 0.747 playing client)
    let signal = state.signals.create_labelled(Str::new(b"ActorMessageEvent"), Str::new(b"Actor.ActorMessageEvent"));
    state.dom.at_mut(actor).signals.get_or_insert_with(Box::default).messages.push((Str::new(topic), signal));
    Some(signal)
}

/// A message to `actor`'s bindings of `topic` (taken now), with its arguments copied already: a serial binding's handler through a
/// deferred job that queues it (at once from a parallel thread), a parallel binding's at the next parallel point.
fn post(lua: &Lua, engine: &Engine, actor: InstanceId, topic: &[u8], args: Vec<Value>) {
    let Some(signal) = topic_signal(engine, actor, topic, false) else { return };
    let direct = in_parallel_part(lua, engine);
    let mut state = engine.state.borrow_mut();
    if !state.signals.has_handlers(signal) {
        return;
    }
    let handlers = state.signals.take_handlers(signal);
    let mut parallel = Vec::new();
    for handler in handlers {
        let crate::signal::Handler::Connection(connection) = handler else { continue };
        if state.signals.is_parallel(connection) {
            parallel.push(Work::Message(connection, args.clone()));
        } else if direct {
            queue_handler(&mut state, connection, signal, args.clone());
        } else {
            state.sched.post(connection, signal, args.clone());
        }
    }
    drop(state);
    if !parallel.is_empty() {
        engine.modules.borrow_mut().parallel.pending.extend(parallel);
    }
}

/// A serial binding's handler queued (deferred, one level deeper than what queues it): a client's message handler costs what a
/// BindableEvent's does (the gap between two handlers of 40 messages 0.5 us on a 0.747 client, a BindableEvent's 0.5-0.6: cost.rs
/// signal_costs Bindable)
pub fn queue_handler(state: &mut crate::State, connection: ConnectionId, signal: SignalId, args: Vec<Value>) {
    let kind = crate::cost::SignalKind::Bindable;
    let (_, resume) = crate::cost::signal_costs(kind);
    let target = crate::scheduler::JobTarget::Handler { connection, kind: crate::scheduler::DispatchKind::Signal(kind) };
    state.sched.defer_handler(target, args, resume, signal);
}

/// Actor:BindToMessage / BindToMessageParallel / SendMessage.
pub fn install(engine: &Rc<Engine>, behaviors: &mut crate::services::Behaviors) {
    let reflection = engine.reflection.clone();
    let Some(actor) = reflection.class_id("Actor") else { return };
    for (member, parallel) in [("BindToMessage", false), ("BindToMessageParallel", true)] {
        let Some(name) = reflection.name_id(member) else { continue };
        behaviors.add_method(actor, name, crate::services::method(move |lua, engine, id, args| bind(lua, engine, id, args, member, parallel)));
    }
    if let Some(name) = reflection.name_id("SendMessage") {
        behaviors.add_method(actor, name, crate::services::method(send_message));
    }
}

/// BindToMessage(topic, callback): the arguments cast from the last (the callback: nil "Argument 2 missing or nil", another value
/// "Unable to cast double to Function"; the topic a string parameter), then the caller's VM checked.
fn bind(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, args: &Args, member: &str, parallel: bool) -> Ret {
    let callback = match args.get(2) {
        LuaValue::Function(callback) => callback.clone(),
        LuaValue::Nil => return raise("Argument 2 missing or nil"),
        other => return raise(format!("Unable to cast {} to Function", crate::services::cast_name(engine, other))),
    };
    let topic = match args.get(1) {
        LuaValue::Nil => return raise("Argument 1 missing or nil"),
        value => super::convert::string_param(lua, engine, value, 1, true)?,
    };
    if caller_vm(lua).flatten() != Some(vm_key(id)) {
        return raise(format!("Calling {member} on an Actor is only allowed in scripts that are a descendent of that Actor"));
    }
    let signal = topic_signal(engine, id, &topic, true).expect("a topic signal");
    engine.state.borrow().sched.charge(crate::cost::CONNECT);
    let connection = engine.state.borrow_mut().signals.connect_with(signal, callback, false, parallel);
    // (a new thread of the binding's takes the binding thread's memory category, as a connection's does: lua/globals.rs handler_thread;
    // and a script's thread's owner tag and globals: a stopped script's bindings run nothing, scripts.rs)
    let thread = lua.current_thread();
    let scripted = crate::scripts::scripted(&thread);
    if super::globals::memory_categories_set() || scripted {
        engine.state.borrow_mut().signals.set_connector(connection, thread);
    }
    if scripted {
        crate::scripts::connected(lua, &mut engine.state.borrow_mut(), connection);
    }
    let userdata = lua.create_any_userdata(super::types::LConnection(connection))?;
    Ok(values([LuaValue::UserData(userdata)]))
}

/// What SendMessage costs a client past a method call's price (a 0.747 playing client, two sessions of 9-sample medians of 200 calls,
/// in the rounds where a plain method call read 46-49 ns, as ree charges 58; tools/parity actor message price rows): 65 ns when it
/// has arguments and 16 ns an argument (one number 81 over none, three 116, ten 224), 105 ns a table it copies and 45 more for the
/// call's first (a table of one number 248 over none, of ten 384, of a hundred 1,617; three tables of one in a table 635), 15 ns an
/// array value, 82 ns a dictionary entry (ten string keys 1,056), 0.073 ns a string's byte (1,000 bytes 154).
const MESSAGE_ARGUMENTS: f64 = 65e-9;
const MESSAGE_ARGUMENT: f64 = 16e-9;
pub(crate) const MESSAGE_TABLE: f64 = 105e-9;
pub(crate) const MESSAGE_FIRST_TABLE: f64 = 45e-9;
pub(crate) const MESSAGE_ITEM: f64 = 15e-9;
pub(crate) const MESSAGE_ENTRY: f64 = 82e-9;
const MESSAGE_BYTE: f64 = 0.073e-9;

/// The copy part of SendMessage's price ([`MESSAGE_ARGUMENTS`] and after) for its copied arguments
fn message_price(sent: &[Value]) -> f64 {
    use super::bindable::{Copied, Key, Sent};
    fn walk(sent: &Sent, tables: &mut u32) -> f64 {
        match sent {
            Sent::Value(LuaValue::String(text)) => text.as_bytes().len() as f64 * MESSAGE_BYTE,
            Sent::Value(_) => 0.0,
            Sent::Buffer(bytes) => bytes.len() as f64 * MESSAGE_BYTE,
            Sent::Table(entries) => {
                *tables += 1;
                let mut price = MESSAGE_TABLE;
                for (key, item) in entries {
                    price += if matches!(key, Key::Index(_)) { MESSAGE_ITEM } else { MESSAGE_ENTRY } + walk(item, tables);
                }
                price
            }
        }
    }
    if sent.is_empty() {
        return 0.0;
    }
    let mut tables = 0;
    let mut price = MESSAGE_ARGUMENTS + sent.len() as f64 * MESSAGE_ARGUMENT;
    for value in sent {
        price += match value {
            Value::String(text) => text.as_bytes().len() as f64 * MESSAGE_BYTE,
            Value::Lua(held) => match &**held {
                LuaValue::UserData(userdata) => userdata.borrow::<Copied>().map(|copied| walk(&copied.0, &mut tables)).unwrap_or(0.0),
                _ => 0.0,
            },
            _ => 0.0,
        };
    }
    if tables > 0 {
        price += MESSAGE_FIRST_TABLE;
    }
    price
}

/// SendMessage(topic, ...): the arguments copied as BindableEvent:Fire copies them, the topic a string parameter (nil "Argument 1
/// missing or nil"); an Actor outside the DataModel refuses.
fn send_message(lua: &Lua, engine: &Rc<Engine>, id: InstanceId, args: &Args) -> Ret {
    super::refs::admit(engine, super::refs::Hold::Work, "SendMessage")?;
    let height = args.0.len();
    let mut tally = super::bindable::Tally::default();
    let mut sent = Vec::with_capacity(args.0.len().saturating_sub(2));
    for argument in args.0.iter().skip(2) {
        match super::bindable::sent_value(lua, engine, argument, height, &mut tally) {
            Ok(value) => sent.push(value),
            Err(refused) => return refused.raise(),
        }
    }
    let topic = match args.get(1) {
        LuaValue::Nil => return raise("Argument 1 missing or nil"),
        value => super::convert::string_param(lua, engine, value, 1, true)?,
    };
    if !crate::services::in_data_model(&engine.state.borrow(), id) {
        return raise("Actor:SendMessage may only be called on Actors attached to the DataModel.");
    }
    let price = message_price(&sent);
    if price > 0.0 {
        engine.state.borrow().sched.charge(price);
        crate::boundary::add_cost(price);
    }
    // (sent from another VM, its functions arrive as ones that raise when called: [`across`])
    if caller_vm(lua).flatten() != Some(vm_key(id)) {
        sent = sent.into_iter().map(|value| across(lua, engine, value)).collect::<LuaResult<_>>()?;
    }
    post(lua, engine, id, &topic, sent);
    Ok(none())
}
