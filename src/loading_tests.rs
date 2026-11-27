//! `loadstring`, `debug.loadmodule` and the services Roblox keeps to itself, through the script API (lua/loading.rs,
//! services::ROBLOX_ONLY_SERVICES); the lines are what Studio 0.747 printed for tools/parity/probes/loadstring.luau,
//! loadmodule.luau and core_services.luau.

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Engine, Options, RunContext,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

/// Runs `source` as a script of a fresh engine with `options`, then its clock for a second: the lines it printed.
#[inline(never)]
fn run(options: Options, source: &str) -> Vec<String> {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    script(&lua, &engine, source)
}

#[inline(never)]
fn script(lua: &Lua, engine: &Rc<Engine>, source: &str) -> Vec<String> {
    script_with(lua, engine, source, false)
}

/// [`run`] with the VM's globals at `Base` in the script's environment: a script cannot reach them (its environment's metatable
/// is locked, as a client's), and the native sandbox tests chain their sandboxes to them, the VM's safe global base
#[inline(never)]
fn run_with_base(options: Options, source: &str) -> Vec<String> {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    script_with(&lua, &engine, source, true)
}

#[inline(never)]
fn script_with(lua: &Lua, engine: &Rc<Engine>, source: &str, base: bool) -> Vec<String> {
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        super::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
    if base {
        let globals = engine.lua.borrow().shared.clone().expect("globals");
        env.raw_set("Base", globals).expect("Base");
    }
    let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(lua, engine, function, LuaMultiValue::new()).expect("call");
    if let CallOutcome::Failed(message, _) = call.outcome().unwrap() {
        panic!("test script failed: {message}");
    }
    let now = engine.state.borrow().sched.now;
    scheduler::run_until(lua, engine, now + 1.0).unwrap();
    // (then to the script's end: what follows a wait past the second is checked too)
    if let Err(message) = call.finish(lua, engine) {
        panic!("test script failed: {message}");
    }
    let entries = &engine.state.borrow().output.entries;
    let errors: Vec<String> = entries.iter().filter(|e| e.kind == "error").map(|e| e.message.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    entries.iter().filter(|e| e.kind == "print").map(|e| e.message.to_string()).collect()
}

#[inline(never)]
fn server(load_string: bool) -> Options {
    Options { mode: "play".into(), run_context: RunContext::Server, local_player: false, load_string, ..Options::default() }
}

#[inline(never)]
fn check(lines: &[String], wanted: &[&str]) {
    assert_eq!(lines.iter().map(String::as_str).collect::<Vec<_>>(), wanted, "{lines:#?}");
}

const MODULES: &str = r##"
    local function Make(Name, Source, Class)
        local Module = Instance.new(Class or "ModuleScript")
        Module.Name = Name
        Module.Source = Source
        Module.Parent = game:GetService("ReplicatedStorage")
        return Module
    end
"##;

#[test]
fn loadstring_is_refused_on_a_client_and_where_the_server_has_it_off() {
    let source = r##"
        print(type(loadstring), pcall(loadstring, "return 1"))
        print(pcall(loadstring))
    "##;
    let refused = ["function false loadstring() is not available", "false loadstring() is not available"];
    check(&run(Options::default(), source), &refused);
    check(&run(Options { mode: "play".into(), load_string: true, ..Options::default() }, source), &refused);
    check(&run(server(false), source), &refused);
    let enabled = run(server(true), source);
    assert!(enabled[0].starts_with("function true function: "), "{enabled:?}");
    assert_eq!(enabled[1], "false missing argument #1 to 'loadstring' (string expected)");
}

#[test]
fn loadstring_compiles_a_chunk_in_the_callers_environment_without_running_it() {
    let lines = run(
        server(true),
        r##"
        local Chunk = loadstring("Ran = (Ran or 0) + 1 return Ran, script, ...")
        print("loaded", type(Chunk), Ran)
        print("called", Chunk("a") == 1, select(2, Chunk()) == script, select(3, Chunk("b")), Ran, rawget(getfenv(1), "Ran"))
        print("env", getfenv(Chunk) == getfenv(1), pcall(loadstring, "return 1") and select(2, pcall(function() return getfenv(loadstring("return 1")) == getfenv(1) end)))
        print("twins", loadstring("return 1") == loadstring("return 1"))
        local Fenced = loadstring("return Marker, type(print), script")
        print("setfenv", setfenv(Fenced, { Marker = "replaced", type = type }) == Fenced, Fenced())
        print("caller", Marker, type(print))
        setfenv(Fenced, setmetatable({ Marker = "layered" }, { __index = getfenv(1) }))
        print("layered", Fenced())
        print("yields", loadstring("task.wait() return 'waited'")())
        "##,
    );
    check(
        &lines,
        &[
            "loaded function nil",
            "called true true b 3 3",
            "env true true",
            "twins false",
            "setfenv true replaced nil nil",
            "caller nil function",
            "layered layered function Test",
            "yields waited",
        ],
    );
}

#[test]
fn loadstring_names_its_chunk_and_answers_a_syntax_error_as_a_message() {
    let lines = run(
        server(true),
        r##"
        local function Fail(...) return select(2, pcall(loadstring(...))) end
        print(Fail("error('boom')"))
        print(Fail("error('boom')", "Name"))
        print(Fail("error('boom')", "=Name"))
        print(Fail("error('boom')", "@Name"))
        print(Fail("local A = 1\nerror('boom')"))
        print(debug.info(loadstring("return 1", "=Name"), "s"), debug.info(loadstring(""), "s"), debug.info(loadstring("\0\1\2"), "s"))
        print(loadstring("local = 1"))
        print(loadstring("local = 1", "=Name"))
        print(select("#", loadstring("local = 1")), select("#", loadstring("return 1")))
        print(loadstring(12))
        print(pcall(loadstring, nil))
        print(pcall(loadstring, {}))
        print(pcall(loadstring, "return 1", {}))
        "##,
    );
    check(
        &lines,
        &[
            "[string \"error('boom')\"]:1: boom",
            "[string \"Name\"]:1: boom",
            "Name:1: boom",
            "Name:1: boom",
            "[string \"local A = 1...\"]:2: boom",
            "Name [string \"\"] [string \"\"]",
            "nil [string \"local = 1\"]:1: Expected identifier when parsing variable name, got '='",
            "nil Name:1: Expected identifier when parsing variable name, got '='",
            "2 1",
            "nil [string \"12\"]:1: Expected identifier when parsing expression, got '12'",
            "false invalid argument #1 to 'loadstring' (string expected, got nil)",
            "false invalid argument #1 to 'loadstring' (string expected, got table)",
            "false invalid argument #2 to 'loadstring' (string expected, got table)",
        ],
    );
}

#[test]
fn loadmodule_is_not_enabled_unless_the_run_turns_it_on() {
    let lines = run(
        Options::default(),
        r##"
        print(type(debug.loadmodule), pcall(debug.loadmodule, Instance.new("ModuleScript")))
        print(pcall(debug.loadmodule))
        "##,
    );
    check(&lines, &["function false debug.loadmodule is not enabled.", "false debug.loadmodule is not enabled."]);
}

#[test]
fn loadmodule_compiles_a_modules_source_without_running_it() {
    let source = format!(
        "{MODULES}{}",
        r##"
        local Counter = Make("Counter", "_G.Loads = (_G.Loads or 0) + 1 return { Count = _G.Loads, Script = script, Arguments = select('#', ...) }")
        local Load, Problem, Cleanup = debug.loadmodule(Counter)
        print("loaded", type(Load), Problem, type(Cleanup), select("#", debug.loadmodule(Counter)), _G.Loads)
        print("info", debug.info(Load, "s"), debug.info(Load, "l"))
        local First, Second = Load(), Load("a", "b")
        print("calls", First.Count, Second.Count, First ~= Second, First.Script == Counter, Second.Arguments)
        print("require", require(Counter).Count, require(Counter).Count, Load().Count)
        print("cleanup", select("#", Cleanup()), Load().Count)
        Counter.Source = "return 'second'"
        Counter.Name = "Renamed"
        local Again = debug.loadmodule(Counter)
        print("source", Again(), debug.info(Again, "s"), debug.info(Load, "s"))
        local Loose = Instance.new("ModuleScript")
        print("fresh", debug.info(debug.loadmodule(Loose), "s"), select("#", debug.loadmodule(Loose)()))
        print("fails", pcall(debug.loadmodule(Make("Fail", "local A = 1\nerror('boom')"))))
        print("yields", debug.loadmodule(Make("Yields", "task.wait() return 'waited'"))())
        "##
    );
    let lines = run(Options { load_module: true, ..Options::default() }, &source);
    check(
        &lines,
        &[
            "loaded function nil function 3 nil",
            "info ReplicatedStorage.Counter 1",
            "calls 1 2 true true 2",
            "require 3 3 4",
            "cleanup 0 5",
            "source second ReplicatedStorage.Renamed ReplicatedStorage.Counter",
            "fresh ModuleScript 0",
            "fails false ReplicatedStorage.Fail:2: boom",
            "yields waited",
        ],
    );
}

#[test]
fn a_loaded_modules_environment_is_its_own_and_setfenv_replaces_it() {
    let source = format!(
        "{MODULES}{}",
        r##"
        local Reader = Make("Reader", "return { getfenv(1), script, _G, shared, game }")
        local Load = debug.loadmodule(Reader)
        local Env, Script, Globals, Shared, Game = table.unpack(Load())
        print("env", Env == getfenv(Load), Env ~= getfenv(1), Script == Reader, rawget(Env, "script") == Reader, Globals == _G, Shared == shared, Game == game)
        local Twin = debug.loadmodule(Reader)
        print("twin", Twin ~= Load, getfenv(Twin) ~= Env, require(Reader)[1] ~= Env)
        print("requires", Env.require(Make("Dependency", "return 'dependency ' .. script.Name")))
        local Fenced = debug.loadmodule(Make("Fenced", "return Marker, type(print), script"))
        local Default = getfenv(Fenced)
        print("setfenv", setfenv(Fenced, { Marker = "replaced", type = type }) == Fenced, Fenced())
        local Layered = setmetatable({ Marker = "layered", script = "mocked", print = false }, { __index = Default })
        setfenv(Fenced, Layered)
        print("layered", getfenv(Fenced) == Layered, Fenced())
        print("other", debug.loadmodule(Make("Fenced", "return Marker, type(print), script.Name"))())
        "##
    );
    let lines = run(Options { load_module: true, ..Options::default() }, &source);
    check(
        &lines,
        &[
            "env true true true true true true true",
            "twin true true true",
            "requires dependency Dependency",
            "setfenv true replaced nil nil",
            "layered true layered boolean mocked",
            "other nil function Fenced",
        ],
    );
}

#[test]
fn loadmodule_answers_a_syntax_error_as_a_message_and_refuses_what_is_no_module() {
    let source = format!(
        "{MODULES}{}",
        r##"
        print(debug.loadmodule(Make("Syntax", "local = 1")))
        print(select("#", debug.loadmodule(Make("Lines", "local A = 1\nlocal B = (\nreturn"))), (select(2, debug.loadmodule(Make("Lines", "local A = 1\nlocal B = (\nreturn")))))
        for _, Case in { { Make("AScript", "return 1", "Script") }, { Make("ALocal", "return 1", "LocalScript") }, { game }, {}, { "return 1" }, { 12 }, { Make("Extra", "return 1"), "more" } } do
            print(pcall(debug.loadmodule, table.unpack(Case)))
        end
        "##
    );
    let lines = run(Options { load_module: true, ..Options::default() }, &source);
    let refused = "false Attempted to call loadmodule with invalid argument(s).";
    check(
        &lines,
        &[
            "nil ReplicatedStorage.Syntax:1: Expected identifier when parsing variable name, got '='",
            "2 ReplicatedStorage.Lines:3: Expected identifier when parsing expression, got 'return'",
            refused,
            refused,
            refused,
            refused,
            refused,
            refused,
            refused,
        ],
    );
}

#[test]
fn services_roblox_keeps_to_itself_answer_nil() {
    let lines = run(
        Options::default(),
        r##"
        for _, Name in { "CoreScriptSyncService", "CorePackages", "RobloxReplicatedStorage", "PluginGuiService", "RobloxPluginGuiService", "AppStorageService", "CSGDictionaryService" } do
            assert(game:GetService(Name) == nil and game:FindService(Name) == nil, Name)
        end
        print(pcall(function() local Service = game:GetService("CoreScriptSyncService") local _ = Service.Name return Service end))
        print(game:GetService("CoreGui"), game:GetService("ScriptContext").Name, game:FindService("ServerStorage"))
        print(pcall(game.FindService, game, "NotAService"))
        "##,
    );
    check(
        &lines,
        &[
            "false Test:5: attempt to index nil with 'Name'",
            "CoreGui Script Context nil",
            "false 'NotAService' is not a valid Service name",
        ],
    );
}

#[test]
fn getfenv_of_the_thread_and_of_an_engine_function_is_the_calling_scripts_environment() {
    let lines = run(
        Options::default(),
        r##"
        local Env = getfenv(1)
        print("script", getfenv(0) == Env, getfenv() == Env, getfenv(print) == Env, getfenv(0).script == script, type(getfenv(0).warn))
        local function Inner() return getfenv(1), getfenv(2), getfenv(0) end
        local Own = setmetatable({}, { __index = Env })
        setfenv(Inner, Own)
        local One, Two, Zero = Inner()
        print("levels", One == Own, Two == Env, Zero == Own, getfenv(Inner) == Own)
        task.spawn(function() print("spawned", getfenv(0) == Env) end)
        local Event = Instance.new("BindableEvent")
        Event.Event:Connect(function() print("handler", getfenv(0) == Env) end)
        Event:Fire()
        print("pcall", select(2, pcall(getfenv, 0)) == Env, coroutine.wrap(function() return getfenv(0) == Env end)())
        print("invalid", select(2, pcall(getfenv, 50)), select(2, pcall(getfenv, "x")))
        "##,
    );
    check(
        &lines,
        &[
            "script true true true true function",
            "levels true true true true",
            "spawned true",
            "pcall true true",
            "invalid invalid argument #1 to 'getfenv' (invalid level) invalid argument #1 to 'getfenv' (number expected, got string)",
            "handler true",
        ],
    );
}

#[test]
fn loadmodule_gives_a_json_module_a_function_that_decodes_it() {
    let folder = std::env::temp_dir().join(format!("ree-loadmodule-json-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Data.json"), r##"{"Name": "Sample", "Values": [1, 2, 3]}"##).unwrap();
    let engine = Engine::new(Options { load_module: true, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    super::project::mount_at(&engine, &super::project::Mount::parse(&format!("ReplicatedStorage/Mounted={}", folder.to_string_lossy()), false).unwrap()).unwrap();
    let lines = script(
        &lua,
        &engine,
        r##"
        local Module = game:GetService("ReplicatedStorage"):FindFirstChild("Data", true)
        local Load, Problem, Cleanup = debug.loadmodule(Module)
        local First, Second = Load(), Load()
        print(type(Load), Problem, type(Cleanup), First.Name, #First.Values, First ~= Second, debug.info(Load, "s") == Module:GetFullName())
        print(setfenv(Load, setmetatable({}, { __index = getfenv(Load) })) == Load, Load().Values[3], require(Module).Name)
        "##,
    );
    let _ = std::fs::remove_dir_all(&folder);
    check(&lines, &["function nil function Sample 3 true true", "true 3 Sample"]);
}

#[test]
fn an_edit_run_context_is_both_sides() {
    let source = r##"local RunService = game:GetService("RunService") print(RunService:IsClient(), RunService:IsServer(), RunService:IsRunning(), game:GetService("Players").LocalPlayer, #game:GetService("Players"):GetPlayers())"##;
    // (the command line's edit run context has no local player, as Studio's edit DataModel has none)
    check(&run(Options { run_context: RunContext::Edit, local_player: false, ..Options::default() }, source), &["true true false nil 0"]);
    check(&run(Options::default(), source), &["true false false Player 1"]);
}

#[test]
fn native_loadmodule_preserves_sandbox_overrides_and_falls_back_for_other_globals() {
    let source = format!(
        "{MODULES}{}",
        r##"
        local Worker = Make("Worker", [[
            local Total = 0
            for I = 1, 20 do
                Total += math.abs(-I)
            end
            return {
                Total = Total,
                ScriptName = script.Name,
                Clock = os.clock(),
                Rand = math.random(),
                PrintType = type(print),
                TypeOfNum = type(Total),
            }
        ]])
        local Fn = debug.loadmodule(Worker)
        local Default = getfenv(Fn)
        local Sandbox = table.freeze(setmetatable({
            script = { Name = "MockedScript" },
            os = setmetatable({ clock = function() return 42.5 end }, { __index = Base.os }),
        }, table.freeze({ __index = Base })))
        local Mocked = setmetatable({
            print = function() end,
            math = setmetatable({ random = function() return 0.75 end }, { __index = Sandbox.math }),
        }, table.freeze({ __index = Sandbox }))
        setfenv(Fn, Mocked)
        local Out = Fn()
        print("sandbox", Out.Total, Out.ScriptName, Out.Clock, Out.Rand, Out.PrintType, Out.TypeOfNum)

        -- Inserting a non-whitelisted key into the writable sandbox clears safeenv immediately.
        Mocked.type = function() return "mutated" end
        local OutMutated = Fn()
        print("mutated", OutMutated.TypeOfNum)

        -- Shadowing a non-whitelisted builtin (type) before setfenv clears safeenv too.
        local Shadow = setmetatable({
            type = function() return "shadowed" end,
        }, table.freeze({ __index = Sandbox }))
        setfenv(Fn, Shadow)
        local OutShadow = Fn()
        print("shadow", OutShadow.Total, OutShadow.TypeOfNum)

        -- Replacing the metatable of a safe sandbox clears safeenv.
        local Rebound = setmetatable({}, table.freeze({ __index = Sandbox }))
        setfenv(Fn, Rebound)
        setmetatable(Rebound, { __index = setmetatable({ type = function() return "rebound" end }, { __index = Sandbox }) })
        print("rebound", Fn().TypeOfNum)

        -- An unfrozen metatable is rejected so later __index swaps are observed.
        local MutableMt = { __index = Sandbox }
        local Swapped = setmetatable({}, MutableMt)
        setfenv(Fn, Swapped)
        MutableMt.__index = setmetatable({ type = function() return "swapped" end }, { __index = Sandbox })
        print("swapped", Fn().TypeOfNum)

        -- Modules that declare globals still run cleanly in the writable sandbox.
        local GlobalWriter = debug.loadmodule(Make("GlobalWriter", "function helper(x) return x + 1 end return helper(4)"))
        local WriterEnv = setmetatable({}, table.freeze({ __index = Sandbox }))
        setfenv(GlobalWriter, WriterEnv)
        print("globalwriter", GlobalWriter(), type(WriterEnv.helper))

        -- Repeated loadmodule on the same source reuses bytecode and returns independent environments.
        local Again = debug.loadmodule(Worker)
        print("reuse", Again ~= Fn, getfenv(Again) ~= Default, Again().ScriptName)
        "##
    );
    let lines = run_with_base(Options { load_module: true, native: true, ..Options::default() }, &source);
    check(
        &lines,
        &[
            "sandbox 210 MockedScript 42.5 0.75 function number",
            "mutated mutated",
            "shadow 210 shadowed",
            "rebound rebound",
            "swapped swapped",
            "globalwriter 5 function",
            "reuse true true Worker",
        ],
    );
}

#[test]
fn native_mode_disables_work_counting_and_reuses_json_cache() {
    let folder = std::env::temp_dir().join(format!("ree-native-json-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Data.json"), r##"{"List": [10, 20, 30]}"##).unwrap();
    let engine = Engine::new(Options { load_module: true, native: true, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    super::project::mount_at(&engine, &super::project::Mount::parse(&format!("ReplicatedStorage/Mounted={}", folder.to_string_lossy()), false).unwrap()).unwrap();
    let lines = script(
        &lua,
        &engine,
        r##"
        local Module = game:GetService("ReplicatedStorage"):FindFirstChild("Data", true)
        local A = debug.loadmodule(Module)()
        A.List[1] = 999
        local B = debug.loadmodule(Module)()
        print("fresh", A.List[1], B.List[1], #B.List)
        "##,
    );
    let _ = std::fs::remove_dir_all(&folder);
    check(&lines, &["fresh 999 10 3"]);
}

#[test]
fn native_loadmodule_template_cache_isolates_envs_and_invalidates_on_change() {
    let source = format!(
        "{MODULES}{}",
        r##"
        -- Pure straight-line data module (skips JIT codegen) still clones cleanly and isolates tables/envs.
        local Data = Make("PureData", "return { Value = Marker or 10, Script = script.Name, Items = { 1, 2 } }")
        local First = debug.loadmodule(Data)
        local Second = debug.loadmodule(Data)
        setfenv(First, setmetatable({ Marker = 111 }, { __index = getfenv(First) }))
        setfenv(Second, setmetatable({ Marker = 222 }, { __index = getfenv(Second) }))
        local A = First()
        local B = Second()
        A.Items[1] = 99
        print("data", First ~= Second, A.Value, B.Value, A.Items[1], B.Items[1], A.Script, debug.info(First, "s"))

        -- Editing Source or Name invalidates the cached template immediately.
        Data.Source = "return { Value = 333 }"
        Data.Name = "PureDataV2"
        local Third = debug.loadmodule(Data)
        print("invalidated", Third().Value, debug.info(Third, "s"), debug.info(First, "s"))
        "##
    );
    let lines = run(Options { load_module: true, native: true, ..Options::default() }, &source);
    check(
        &lines,
        &[
            "data true 111 222 99 1 PureData ReplicatedStorage.PureData",
            "invalidated 333 ReplicatedStorage.PureDataV2 ReplicatedStorage.PureData",
        ],
    );
}

/// Without `--native` each `debug.loadmodule` is the module's chunk loaded in its own environment, as `require` loads
/// one: its `script` resolves at load (a shared template's run-time lookups were charged 7.9 ns a read), and its
/// environment is a script's: its own table, under the locked metatable (modules.rs environment_in).
#[test]
fn loadmodule_without_native_loads_each_chunk_in_its_own_environment() {
    let engine = Engine::new(Options { load_module: true, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    // as `ree run` and `ree record` load modules: interpreted, in safe environments whose imports resolve at load
    engine_lua::modules::interpret_modules(&engine);
    engine_lua::start(&lua, &engine).expect("start");
    let source = format!(
        "{MODULES}{}",
        r##"
        -- (os.clock charges a script's work from its first frame)
        task.wait()
        local Counter = Make("Counter", "return function(Count) local N = 0 for _ = 1, Count do if script then N += 1 end end return N end")
        local function Cost(Run)
            local Start = os.clock()
            Run(1000)
            return os.clock() - Start
        end
        local Required = Cost(require(Counter))
        local Loaded = Cost(debug.loadmodule(Counter)())
        local Again = Cost(debug.loadmodule(Counter)())
        print("script reads cost as required", Required > 0, math.abs(Loaded - Required) < 1e-7, math.abs(Again - Required) < 1e-7)

        local Reader = Make("Reader", "return function() return getfenv(1) end")
        local First = debug.loadmodule(Reader)()()
        local Second = debug.loadmodule(Reader)()()
        print("environments", rawequal(First, Second), getmetatable(First), pcall(setmetatable, First, {}))
        "##
    );
    let lines = script(&lua, &engine, &source);
    check(&lines, &["script reads cost as required true true true", "environments false The metatable is locked false cannot change a protected metatable"]);
}

/// What follows a wait in a test script runs and is checked: the helper steps frames until the script ends (an assert after a
/// wait past the helper's second used to pass unchecked, the call left waiting).
#[test]
#[should_panic(expected = "checked after the wait")]
fn a_test_script_is_checked_after_it_waits() {
    run(Options::default(), "task.wait(2) assert(false, 'checked after the wait')");
}

/// Requiring a file-backed ModuleScript pins the first text read on the instance so overwriting
/// the file mid-run never changes `.Source`, a later `Clone()` require, or an Actor VM's require.
#[test]
fn require_pins_first_read_source_for_source_property_clones_and_actors() {
    let folder = std::env::temp_dir().join(format!(
        "ree-pin-source-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let config_file = folder.join("Config.luau");
    let first_text = "return \"first-read\"";
    let second_text = "return \"second-read-modified-on-disk\"";
    std::fs::write(&config_file, first_text).unwrap();

    // (play mode: an edit DataModel starts no script)
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    super::project::mount_at(&engine, &super::project::Mount::parse(&format!("ReplicatedStorage/Mounted={}", folder.to_string_lossy()), false).unwrap()).unwrap();

    let overwrite_path = config_file.clone();
    let overwrite_fn = lua
        .create_function(move |_, ()| {
            std::fs::write(&overwrite_path, second_text).map_err(LuaError::external)?;
            Ok(())
        })
        .unwrap();
    engine.lua.borrow().shared.as_ref().unwrap().raw_set("__overwriteConfig", overwrite_fn).unwrap();

    let lines = script(
        &lua,
        &engine,
        r##"
        local RS = game:GetService("ReplicatedStorage")
        local Config = RS:FindFirstChild("Config", true)
        local Initial = require(Config)
        __overwriteConfig()
        local Cloned = Config:Clone()
        Cloned.Parent = RS
        local FromClone = require(Cloned)
        local SourceProp = Config.Source
        local Actor = Instance.new("Actor")
        local Worker = Instance.new("LocalScript")
        Worker.Source = [[
            local Config = game:GetService("ReplicatedStorage"):FindFirstChild("Config", true)
            print("actor", require(Config))
        ]]
        Worker.Parent = Actor
        -- (a LocalScript runs in the player's PlayerScripts, as a client runs it: scripts.rs)
        Actor.Parent = game:GetService("Players").LocalPlayer:FindFirstChildOfClass("PlayerScripts")
        print("main", Initial, FromClone, SourceProp)
        "##,
    );
    let _ = std::fs::remove_dir_all(&folder);
    check(
        &lines,
        // (the Actor's worker starts at its turn in the deferred queue, after the main script's line: scripts.rs)
        &[
            "main first-read first-read return \"first-read\"",
            "actor first-read",
        ],
    );
}
