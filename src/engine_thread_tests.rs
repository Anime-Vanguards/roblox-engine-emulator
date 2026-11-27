//! The threads a module's chunk, a BindableFunction's callback and a signal's handlers run on, as a 0.747 playing client and a
//! server run them (tools/parity/probes/engine_threads.luau): a module's chunk and a callback at the bottom of a thread of their
//! own, which require is recursive and which waits, what a module's error logs, which handler thread is reused, and the memory
//! category each thread starts with.

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Engine, Options, RunContext,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

#[inline(never)]
fn start(run_context: RunContext) -> (Lua, Rc<Engine>) {
    let options = match run_context {
        RunContext::Server => Options { mode: "play".into(), run_context, local_player: false, ..Options::default() },
        _ => Options { mode: "play".into(), ..Options::default() },
    };
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// Runs `source` as a script named Test of a fresh engine, then the clock `seconds` on and the script to its end: the output lines
/// ("kind: message"). A script error fails the test; logged errors are lines.
#[inline(never)]
fn run(run_context: RunContext, source: &str, seconds: f64) -> Vec<String> {
    let (lua, engine) = start(run_context);
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        super::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(&lua, &engine, script).expect("environment");
    let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(&lua, &engine, function, LuaMultiValue::new()).expect("call");
    if let CallOutcome::Failed(message, _) = call.outcome().unwrap() {
        panic!("test script failed: {message}");
    }
    let now = engine.state.borrow().sched.now;
    scheduler::run_until(&lua, &engine, now + seconds).unwrap();
    if let Err(message) = call.finish(&lua, &engine) {
        panic!("test script failed: {message}");
    }
    let lines = engine.state.borrow().output.entries.iter().map(|e| format!("{}: {}", e.kind, e.message)).collect();
    lines
}

fn done(lines: &[String]) {
    assert!(lines.iter().any(|line| line == "print: done"), "{lines:?}");
}

/// The modules a test makes, in a Folder in ReplicatedStorage: `Make(Name, Source)`
const MAKE: &str = r##"
local Folder = Instance.new("Folder")
Folder.Parent = game:GetService("ReplicatedStorage")
local function Make(Name, Source)
    local Module = Instance.new("ModuleScript")
    Module.Name = Name
    Module.Source = Source
    Module.Parent = Folder
    return Module
end
"##;

/// A module's chunk is the bottom of a thread of its own: nothing above it, a traceback of the module alone, the requiring thread
/// "normal" meanwhile, the module's Name as its memory category, and its value left on that thread (which reads "suspended"). The
/// thread that began loading a module that waits shows only its require line.
#[test]
fn a_modules_chunk_runs_at_the_bottom_of_a_thread_of_its_own() {
    for context in [RunContext::Client, RunContext::Server] {
        let lines = run(
            context,
            &format!(
                "{MAKE}{}",
                r##"
                local Probe = Make("Probe", "shared.T = coroutine.running()\nshared.status = coroutine.status(shared.R)\nshared.info = debug.info(2, 's')\nshared.cat = debug.getmemorycategory()\ndebug.setmemorycategory('mine')\nshared.trace = debug.traceback()\nreturn 1")
                shared.R = coroutine.running()
                debug.setmemorycategory("req")
                assert(require(Probe) == 1)
                assert(shared.T ~= shared.R and shared.status == "normal" and shared.info == nil, shared.status)
                assert(shared.cat == "Probe" and debug.getmemorycategory() == "req", shared.cat)
                assert(shared.trace == "ReplicatedStorage.Folder.Probe:6\n", shared.trace)
                assert(coroutine.status(shared.T) == "suspended")
                assert(coroutine.resume(shared.T) == false and coroutine.status(shared.T) == "dead")
                local Slow = Make("Slow", "shared.S = coroutine.running()\ntask.wait(0.05)\nreturn 'slow'")
                local Got
                local Requirer = task.spawn(function()
                    Got = require(Slow)
                end)
                assert(shared.S ~= Requirer and coroutine.status(shared.S) == "suspended")
                assert(debug.traceback(shared.S) == "ReplicatedStorage.Folder.Slow:2\n", debug.traceback(shared.S))
                local Trace = debug.traceback(Requirer)
                assert(string.match(Trace, "^Test:%d+\n$"), Trace)
                task.wait(0.1)
                assert(Got == "slow" and coroutine.status(Requirer) == "dead")
                print("done")
                "##
            ),
            0.2,
        );
        done(&lines);
    }
}

/// A require is recursive when its module's chunk is running in the chain of loads it is part of (the chunk itself, a cycle of
/// modules, a thread the chunk runs without a yield between); any other waits, so a cycle with a yield in it waits for ever. A
/// module's error is logged as its chunk's thread's uncaught error, ScriptContext.Error naming the module.
#[test]
fn which_require_is_recursive_and_which_waits() {
    let lines = run(
        RunContext::Client,
        &format!(
            "{MAKE}{}",
            r##"
            local Errors = {}
            game:GetService("ScriptContext").Error:Connect(function(Message, Trace, Script)
                table.insert(Errors, Message .. " | " .. Trace .. " | " .. Script.Name)
            end)
            local Self = Make("Self", "return require(script)")
            assert(select(2, pcall(require, Self)) == "Requested module experienced an error while loading")
            Make("CycB", "return require(script.Parent.CycA)")
            local CycA = Make("CycA", "return require(script.Parent.CycB)")
            assert(select(2, pcall(require, CycA)) == "Requested module experienced an error while loading")
            local Sp = Make("Sp", "task.spawn(function() shared.sp = select(2, pcall(require, script)) end) return 'sp'")
            assert(require(Sp) == "sp" and shared.sp == "Requested module was required recursively", shared.sp)
            Make("YB", "task.wait() return require(script.Parent.YA)")
            local YA = Make("YA", "task.wait() return require(script.Parent.YB)")
            local Cycle = task.spawn(require, YA)
            Make("ResB", "return require(script.Parent.Res)")
            local Res = Make("Res", "task.wait() local T = coroutine.create(function() return pcall(require, script.Parent.ResB) end) shared.res = { coroutine.resume(T) } return 'res'")
            task.spawn(require, Res)
            task.wait(0.2)
            assert(coroutine.status(Cycle) == "suspended")
            assert(shared.res[1] == true and shared.res[2] == nil)
            assert(table.concat(Errors, "\n") == table.concat({
                "Requested module was required recursively | ReplicatedStorage.Folder.Self, line 1\n | Self",
                "Requested module was required recursively | ReplicatedStorage.Folder.CycB, line 1\n | CycB",
                "Requested module experienced an error while loading | ReplicatedStorage.Folder.CycA, line 1\n | CycA",
            }, "\n"), table.concat(Errors, "\n"))
            print("done")
            "##
        ),
        0.3,
    );
    done(&lines);
}

/// Every require of a module whose chunk returned anything but one value raises `Module code did not return exactly one value`,
/// the ones that waited for it and later ones too; a chunk's thread cancelled, or one a script resumes after it coroutine.yielded,
/// leaves its requirers waiting for ever.
#[test]
fn a_modules_failure_reaches_every_requirer_and_a_lost_chunk_none() {
    let lines = run(
        RunContext::Server,
        &format!(
            "{MAKE}{}",
            r##"
            local Nothing = Make("Nothing", "task.wait()\nlocal x = 1")
            local A, B
            task.spawn(function() A = select(2, pcall(require, Nothing)) end)
            task.spawn(function() B = select(2, pcall(require, Nothing)) end)
            task.wait(0.1)
            local Text = "Module code did not return exactly one value"
            assert(A == Text and B == Text and select(2, pcall(require, Nothing)) == Text, tostring(A))
            local Cn = Make("Cn", "shared.cn = coroutine.running() task.wait(0.05) return 1")
            local Waiter = task.spawn(require, Cn)
            task.cancel(shared.cn)
            local Yields = Make("Yields", "shared.y = coroutine.running() coroutine.yield() return 1")
            local Second = task.spawn(require, Yields)
            assert(coroutine.resume(shared.y) == true and coroutine.status(shared.y) == "dead")
            task.wait(0.2)
            assert(coroutine.status(Waiter) == "suspended" and coroutine.status(Second) == "suspended")
            print("done")
            "##
        ),
        0.3,
    );
    done(&lines);
}

/// When a module's chunk that yielded ends, its requirers go on at once, the first requirer first, before work the chunk deferred;
/// a BindableFunction's invoker whose callback yielded goes on at the next deferred point, after work the callback deferred.
#[test]
fn a_finished_chunk_wakes_its_requirers_at_once_and_an_invoker_later() {
    let lines = run(
        RunContext::Client,
        &format!(
            "{MAKE}{}",
            r##"
            local Order = {}
            shared.Order = Order
            local Wake = Make("Wake", "local Order = shared.Order task.wait() table.insert(Order, 'ends') task.defer(function() table.insert(Order, 'deferred') end) return 1")
            task.spawn(function() require(Wake) table.insert(Order, 'first') end)
            task.spawn(function() require(Wake) table.insert(Order, 'second') end)
            task.wait(0.1)
            assert(table.concat(Order, " ") == "ends first second deferred", table.concat(Order, " "))
            table.clear(Order)
            local F = Instance.new("BindableFunction")
            F.OnInvoke = function()
                task.wait()
                table.insert(Order, "ends")
                task.defer(function() table.insert(Order, "deferred") end)
            end
            task.spawn(function() F:Invoke() table.insert(Order, "invoker") end)
            task.wait(0.1)
            assert(table.concat(Order, " ") == "ends deferred invoker", table.concat(Order, " "))
            print("done")
            "##
        ),
        0.3,
    );
    done(&lines);
}

/// A BindableFunction's callback is the bottom of a thread of its own (the invoker "normal" meanwhile), reused while its runs
/// return, one per BindableFunction and callback, a new one after a yield; it runs with the memory category of the thread that set
/// it, as that thread has it at each invoke.
#[test]
fn a_bindable_functions_callback_runs_at_the_bottom_of_a_reused_thread() {
    let lines = run(
        RunContext::Client,
        r##"
        debug.setmemorycategory("one")
        local F = Instance.new("BindableFunction")
        local Invoker = coroutine.running()
        local Seen = {}
        F.OnInvoke = function(Wait)
            table.insert(Seen, coroutine.running())
            assert(debug.info(2, "s") == nil and coroutine.status(Invoker) == "normal")
            if Wait then
                task.wait()
            end
            return debug.getmemorycategory()
        end
        assert(F:Invoke() == "one")
        debug.setmemorycategory("two")
        assert(F:Invoke() == "two")
        assert(F:Invoke(true) == "two")
        F:Invoke()
        assert(Seen[1] == Seen[2] and Seen[2] == Seen[3] and Seen[4] ~= Seen[3], "reuse")
        local G = Instance.new("BindableFunction")
        local H = Instance.new("BindableFunction")
        local Other = {}
        local function Both()
            table.insert(Other, coroutine.running())
        end
        G.OnInvoke = Both
        H.OnInvoke = Both
        G:Invoke()
        H:Invoke()
        G:Invoke()
        assert(Other[1] == Other[3] and Other[1] ~= Other[2])
        local Setter = coroutine.create(function()
            debug.setmemorycategory("setter")
            G.OnInvoke = function() return debug.getmemorycategory() end
        end)
        coroutine.resume(Setter)
        assert(G:Invoke() == "setter")
        print("done")
        "##,
        0.2,
    );
    done(&lines);
}

/// A connection runs its handler on the thread its last run returned on (its category kept), a new thread after a yield or an
/// error, which takes the category its connecting thread has then; coroutine.wrap's thread takes its maker's.
#[test]
fn a_connections_handler_thread_is_kept_and_takes_its_connectors_category() {
    let lines = run(
        RunContext::Server,
        r##"
        debug.setmemorycategory("root")
        local Event = Instance.new("BindableEvent")
        local Seen, Cats = {}, {}
        Event.Event:Connect(function(Mode)
            table.insert(Seen, coroutine.running())
            table.insert(Cats, debug.getmemorycategory())
            debug.setmemorycategory("handler")
            if Mode == "wait" then
                task.wait()
            elseif Mode == "error" then
                error("handler")
            end
        end)
        for _, Mode in { "plain", "plain", "wait", "plain", "error", "plain" } do
            Event:Fire(Mode)
            task.wait()
            task.wait()
        end
        local Same = {}
        for I = 2, #Seen do
            table.insert(Same, if Seen[I] == Seen[I - 1] then "1" else "0")
        end
        assert(table.concat(Same) == "11010", table.concat(Same))
        assert(table.concat(Cats, ",") == "root,handler,handler,root,handler,root", table.concat(Cats, ","))
        local Other = Instance.new("BindableEvent")
        local Got
        local Connector = coroutine.create(function()
            debug.setmemorycategory("connector")
            Other.Event:Connect(function() Got = debug.getmemorycategory() end)
            coroutine.yield()
        end)
        coroutine.resume(Connector)
        Other:Fire()
        task.wait()
        assert(Got == "connector", Got)
        debug.setmemorycategory("maker")
        local Wrapped = coroutine.wrap(function() return debug.getmemorycategory() end)
        debug.setmemorycategory("caller")
        assert(Wrapped() == "maker")
        print("done")
        "##,
        0.3,
    );
    done(&lines);
}
