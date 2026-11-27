//! Scripts a running game starts and stops, and the thread globals of a script's threads, through the script API (scripts.rs,
//! patches/luau/30-thread-script-globals.patch). The lines are what a 0.747 playing client and server printed for the same scripts
//! (tools/parity/probes/script_starts.luau, script_stops.luau and thread_globals.luau; the client's and the server's alike).

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Engine, Options, RunContext,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

/// Runs `source` as a harness of a fresh engine with `options`, then its clock for a second: its prints and errors.
#[inline(never)]
fn run(options: Options, source: &str) -> (Vec<String>, Vec<String>) {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    run_on(&lua, &engine, source)
}

#[inline(never)]
fn run_on(lua: &Lua, engine: &Rc<Engine>, source: &str) -> (Vec<String>, Vec<String>) {
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        super::instance::create_named(&mut state, &engine.reflection, class, "Harness")
    };
    let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
    let function = lua.load(source).set_name("=Harness").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(lua, engine, function, LuaMultiValue::new()).expect("call");
    if let CallOutcome::Failed(message, _) = call.outcome().unwrap() {
        panic!("test harness failed: {message}");
    }
    let now = engine.state.borrow().sched.now;
    scheduler::run_until(lua, engine, now + 1.0).unwrap();
    if let Err(message) = call.finish(lua, engine) {
        panic!("test harness failed: {message}");
    }
    let entries = &engine.state.borrow().output.entries;
    let of = |kind: &str| entries.iter().filter(|e| e.kind == kind).map(|e| e.message.to_string()).collect();
    (of("print"), of("error"))
}

#[inline(never)]
fn check(lines: &[String], wanted: &[&str]) {
    assert_eq!(lines.iter().map(String::as_str).collect::<Vec<_>>(), wanted, "{lines:#?}");
}

fn client() -> Options {
    Options { mode: "play".into(), ..Options::default() }
}

fn server() -> Options {
    Options { mode: "play".into(), run_context: RunContext::Server, local_player: false, load_string: true, ..Options::default() }
}

/// The scripts a test makes log through `L`, shared by the harness and the scripts (one VM here)
const HELPERS: &str = r#"
    local RS = game:GetService("ReplicatedStorage")
    local Log = {}
    _G.L = function(s) table.insert(Log, s) end
    local L = _G.L
    local function Make(Class, Name, Source, Parent)
        local Script = Instance.new(Class)
        Script.Name = Name
        Script.Source = Source
        if Parent then
            Script.Parent = Parent
        end
        return Script
    end
    local function Dump()
        for _, Line in Log do
            print(Line)
        end
    end
"#;

fn with_helpers(body: &str) -> String {
    format!("{HELPERS}\n{body}\nDump()")
}

#[test]
fn a_local_script_parented_in_starts_at_its_turn_in_the_deferred_queue() {
    // (the client's lines: the start after the write's ChildAdded handler and the work deferred before it, before what was
    // deferred after the write; what it defers runs at the next resumption point)
    let (prints, errors) = run(client(), &with_helpers(r#"
        local PG = game:GetService("Players").LocalPlayer.PlayerGui
        task.wait()
        local s = Make("LocalScript", "A", [[_G.L("start") task.defer(function() _G.L("its defer") end)]])
        local c = PG.ChildAdded:Connect(function(child) L("childadded " .. child.Name) end)
        task.defer(function() L("defer-before") end)
        s.Parent = PG
        L("after")
        task.defer(function() L("defer-after") end)
        task.spawn(function() L("spawn") end)
        task.wait()
        c:Disconnect()
        L("resumed")
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &["after", "spawn", "defer-before", "childadded A", "start", "defer-after", "its defer", "resumed"]);
}

#[test]
fn where_a_script_runs_follows_its_class_its_run_context_and_the_side() {
    // (a client: LocalScripts in the player's containers and ReplicatedFirst, Client Scripts anywhere; a server: Legacy Scripts in
    // the Workspace, ServerScriptService and a player's PlayerGui and Backpack, Server Scripts anywhere)
    let (prints, errors) = run(client(), &with_helpers(r#"
        local Player = game:GetService("Players").LocalPlayer
        task.wait()
        Make("LocalScript", "LS_PlayerGui", '_G.L("start " .. script.Name)', Player.PlayerGui)
        Make("LocalScript", "LS_Backpack", '_G.L("start " .. script.Name)', Player.Backpack)
        Make("LocalScript", "LS_ReplicatedFirst", '_G.L("start " .. script.Name)', game:GetService("ReplicatedFirst"))
        Make("LocalScript", "LS_Workspace", '_G.L("start " .. script.Name)', workspace)
        Make("LocalScript", "LS_ReplicatedStorage", '_G.L("start " .. script.Name)', RS)
        Make("Script", "SL_Workspace", '_G.L("start " .. script.Name)', workspace)
        task.wait()
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &["start LS_PlayerGui", "start LS_Backpack", "start LS_ReplicatedFirst"]);
    let (prints, errors) = run(server(), &with_helpers(r#"
        task.wait()
        local Player = Instance.new("Player")
        Player.Parent = game:GetService("Players")
        task.wait()
        Make("Script", "SL_Workspace", '_G.L("start " .. script.Name)', workspace)
        Make("Script", "SL_ServerScriptService", '_G.L("start " .. script.Name)', game:GetService("ServerScriptService"))
        Make("Script", "SL_PlayerGui", '_G.L("start " .. script.Name)', Player:FindFirstChildOfClass("PlayerGui"))
        Make("Script", "SL_Backpack", '_G.L("start " .. script.Name)', Player:FindFirstChildOfClass("Backpack"))
        Make("Script", "SL_ServerStorage", '_G.L("start " .. script.Name)', game:GetService("ServerStorage"))
        Make("Script", "SL_ReplicatedStorage", '_G.L("start " .. script.Name)', RS)
        Make("LocalScript", "LS_ServerScriptService", '_G.L("start " .. script.Name)', game:GetService("ServerScriptService"))
        task.wait()
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &["start SL_Workspace", "start SL_ServerScriptService", "start SL_PlayerGui", "start SL_Backpack"]);
}

#[test]
fn a_client_script_runs_anywhere_in_the_data_model_and_a_server_script_on_a_server_only() {
    // RunContext as the place sets it (a game script may not write it in play mode): the engine writes it
    for (options, context, wanted) in [(client(), "Client", "start Ctx"), (server(), "Server", "start Ctx")] {
        let engine = Engine::new(options).expect("engine");
        let lua = Lua::new();
        engine_lua::start(&lua, &engine).expect("start");
        let script = {
            let reflection = &engine.reflection;
            let mut state = engine.state.borrow_mut();
            let script = super::instance::create_named(&mut state, reflection, reflection.class_id("Script").unwrap(), "Ctx");
            let item = reflection.enum_item("RunContext", context).unwrap();
            super::instance::raw_set_silent(&mut state, reflection, script, reflection.name_id("RunContext").unwrap(), crate::value::Value::EnumItem(item));
            state.dom.at_mut(script).set_source(Some(Rc::from(&b"_G.L('start ' .. script.Name)"[..])));
            script
        };
        // (an ObjectValue in ReplicatedStorage holds it, out of the DataModel, for the harness to move in)
        {
            let reflection = &engine.reflection;
            let mut state = engine.state.borrow_mut();
            let holder = super::instance::create_named(&mut state, reflection, reflection.class_id("ObjectValue").unwrap(), "Holder");
            super::instance::raw_set_silent(&mut state, reflection, holder, reflection.name_id("Value").unwrap(), crate::value::Value::Instance(script));
            let game = engine.world().game;
            let storage = state.dom.find_child_of_class(game, reflection.class_id("ReplicatedStorage").unwrap()).unwrap();
            super::instance::set_parent(&mut state, reflection, &engine.hooks.borrow(), holder, Some(storage)).unwrap();
        }
        let (prints, errors) = run_on(&lua, &engine, &with_helpers(r#"
            task.wait()
            RS.Holder.Value.Parent = RS
            task.wait()
        "#));
        assert!(errors.is_empty(), "{errors:?}");
        check(&prints, &[wanted]);
    }
}

#[test]
fn a_stop_takes_what_the_script_left_waiting_and_a_restart_runs_it_again() {
    let (prints, errors) = run(client(), &with_helpers(r#"
        local PG = game:GetService("Players").LocalPlayer.PlayerGui
        task.wait()
        local s = Make("LocalScript", "S", [[
            local run = (script:GetAttribute("runs") or 0) + 1
            script:SetAttribute("runs", run)
            _G.L("start " .. run .. " fresh " .. tostring(rawget(getfenv(1), "Last") == nil))
            Last = true
            task.spawn(function() while true do task.wait() _G.L("spawned " .. run) end end)
            game:GetService("RunService").Heartbeat:Connect(function() _G.L("conn " .. run) end)
            while true do task.wait() _G.L("loop " .. run) end
        ]], PG)
        task.wait()
        task.wait()
        L("off")
        s.Enabled = false
        task.wait()
        task.wait()
        L("on")
        s.Enabled = true
        task.wait()
        L("to ReplicatedStorage")
        s.Parent = RS
        task.wait()
        L("back")
        s.Parent = PG
        task.wait()
        L("to the Backpack")
        s.Parent = game:GetService("Players").LocalPlayer.Backpack
        task.wait()
        s:Destroy()
        task.wait()
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &[
        "start 1 fresh true",
        "spawned 1", "loop 1", "conn 1",
        "off",
        "on",
        "start 2 fresh true",
        "to ReplicatedStorage",
        "back",
        "start 3 fresh true",
        "to the Backpack",
        "spawned 3", "loop 3", "conn 3",
    ]);
}

#[test]
fn a_script_that_disables_itself_goes_on_and_what_it_makes_afterwards_runs() {
    let (prints, errors) = run(client(), &with_helpers(r#"
        local PG = game:GetService("Players").LocalPlayer.PlayerGui
        task.wait()
        Make("LocalScript", "E", [[
            task.defer(function() _G.L("deferred from before") end)
            task.spawn(function() task.wait() _G.L("spawned from before") end)
            script.Enabled = false
            _G.L("after disabling itself")
            task.spawn(function() task.wait() _G.L("spawned from after") end)
            task.defer(function() _G.L("deferred from after") end)
            task.wait()
            _G.L("resumed")
        ]], PG)
        task.wait()
        task.wait()
        task.wait()
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &["after disabling itself", "deferred from after", "spawned from after", "resumed"]);
}

#[test]
fn a_stopped_scripts_connections_end_at_the_next_deferred_point_or_at_their_signals_next_fire() {
    let (prints, errors) = run(client(), &with_helpers(r#"
        local PG = game:GetService("Players").LocalPlayer.PlayerGui
        for i = 1, 2 do
            local e = Instance.new("BindableEvent")
            e.Name = "Event" .. i
            e.Parent = RS
        end
        task.wait()
        for i = 1, 2 do
            Make("LocalScript", "B" .. i, [[
                local i = string.sub(script.Name, 2)
                _G["c" .. i] = game:GetService("ReplicatedStorage")["Event" .. i].Event:Connect(function() _G.L("handler " .. i) end)
                while true do task.wait() end
            ]], PG)
        end
        task.wait()
        PG.B1.Enabled = false
        PG.B2:Destroy()
        L("right after: " .. tostring(_G.c1.Connected) .. " " .. tostring(_G.c2.Connected))
        RS.Event1:Fire()
        task.defer(function() L("deferred: " .. tostring(_G.c1.Connected) .. " " .. tostring(_G.c2.Connected)) end)
        task.wait()
        L("next frame: " .. tostring(_G.c1.Connected) .. " " .. tostring(_G.c2.Connected))
        RS.Event2:Fire()
        task.wait()
        L("after a fire: " .. tostring(_G.c1.Connected) .. " " .. tostring(_G.c2.Connected))
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &["right after: true true", "deferred: false true", "next frame: false true", "after a fire: false false"]);
}

#[test]
fn a_thread_reads_its_scripts_globals_through_a_modules_functions_and_handlers() {
    let (prints, errors) = run(server(), &with_helpers(r#"
        local SSS = game:GetService("ServerScriptService")
        local M = Make("ModuleScript", "M", [[
            _G.envs = _G.envs or {}
            _G.envs.M = getfenv(1)
            local M = {}
            function M.who(e)
                e = e or getfenv(0)
                for k, v in _G.envs do
                    if v == e then return k end
                end
                return "other"
            end
            _G.L("chunk " .. M.who())
            function M.spawned(tag)
                task.spawn(function() _G.L(tag .. " spawn " .. M.who()) end)
                _G.L(tag .. " wrap " .. coroutine.wrap(function() return M.who() end)())
                local one, zero = loadstring("return getfenv(1), getfenv(0)")()
                _G.L(tag .. " loadstring " .. M.who(one) .. " " .. M.who(zero))
            end
            return M
        ]], RS)
        task.wait()
        Make("Script", "A", [[
            _G.envs = _G.envs or {}
            _G.envs.A = getfenv(1)
            local M = require(game:GetService("ReplicatedStorage").M)
            _G.L("A " .. M.who())
            M.spawned("A")
            game:GetService("ReplicatedStorage").ChildAdded:Connect(function() _G.L("A's handler " .. M.who()) end)
            task.spawn(function()
                local T = setmetatable({}, { __index = getfenv(0) })
                _G.envs.T = T
                setfenv(0, T)
                task.spawn(function() _G.L("child of setfenv(0) " .. M.who()) end)
            end)
        ]], SSS)
        task.wait()
        Make("Script", "B", [[
            _G.envs.B = getfenv(1)
            Instance.new("Folder").Parent = game:GetService("ReplicatedStorage")
        ]], SSS)
        task.wait()
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &["chunk M", "A A", "A spawn A", "A wrap A", "A loadstring A A", "child of setfenv(0) T", "A's handler A"]);
}

#[test]
fn a_write_that_starts_a_script_costs_what_a_client_spends_loading_it() {
    let (prints, errors) = run(client(), &with_helpers(r#"
        local PG = game:GetService("Players").LocalPlayer.PlayerGui
        task.wait()
        local function Time(Enabled)
            local s = Instance.new("LocalScript")
            s.Source = "local x = 1"
            s.Enabled = Enabled
            task.wait()
            local a = os.clock()
            s.Parent = PG
            local b = os.clock()
            return b - a
        end
        local started, idle = Time(true), Time(false)
        L(string.format("%.1f", (started - idle) * 1e6))
    "#));
    assert!(errors.is_empty(), "{errors:?}");
    check(&prints, &[&format!("{:.1}", crate::cost::SCRIPT_START_QUEUED[0] * 1e6)]);
}
