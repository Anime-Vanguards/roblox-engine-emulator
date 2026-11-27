//! A script's environment, the one global `require` and the chunk names scripts of the same source share, through the script API
//! (lua/modules.rs environment_in, compile_in_with and resolve_path; lua/builtins.rs require_function). The lines are what a 0.747
//! playing client printed for the same scripts (tools/parity/probes/script_env.luau, chunk_sharing.luau and
//! chunk_sharing_loadmodule.luau; its server printed the same).

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Engine, Options,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

/// What a test script left: the lines it printed and the errors logged
struct Ran {
    prints: Vec<String>,
    errors: Vec<String>,
}

/// Runs `source` as a script of a fresh engine with `options`, then its clock until it ends.
#[inline(never)]
fn run(options: Options, source: &str) -> Ran {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::modules::interpret_modules(&engine);
    engine_lua::start(&lua, &engine).expect("start");
    script(&lua, &engine, source)
}

#[inline(never)]
fn script(lua: &Lua, engine: &Rc<Engine>, source: &str) -> Ran {
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        super::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
    let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(lua, engine, function, LuaMultiValue::new()).expect("call");
    if let CallOutcome::Failed(message, _) = call.outcome().unwrap() {
        panic!("test script failed: {message}");
    }
    let now = engine.state.borrow().sched.now;
    scheduler::run_until(lua, engine, now + 1.0).unwrap();
    if let Err(message) = call.finish(lua, engine) {
        panic!("test script failed: {message}");
    }
    let entries = &engine.state.borrow().output.entries;
    let of = |kind: &str| entries.iter().filter(|e| e.kind == kind).map(|e| e.message.to_string()).collect();
    Ran { prints: of("print"), errors: of("error") }
}

#[inline(never)]
fn check(lines: &[String], wanted: &[&str]) {
    assert_eq!(lines.iter().map(String::as_str).collect::<Vec<_>>(), wanted, "{lines:#?}");
}

fn play() -> Options {
    Options { mode: "play".into(), ..Options::default() }
}

const HELPERS: &str = r##"
    local ReplicatedStorage = game:GetService("ReplicatedStorage")
    local Folder = Instance.new("Folder")
    Folder.Name = "ParityEnv"
    Folder.Parent = ReplicatedStorage
    local function Clean(Text)
        return (string.gsub(tostring(Text), "0x%x+", "ADDR"))
    end
    local function Make(Name, Source, Parent, Class)
        local Script = Instance.new(Class or "ModuleScript")
        Script.Name = Name
        Script.Source = Source
        Script.Parent = Parent or Folder
        return Script
    end
    local function Try(Function, ...)
        local Results = table.pack(pcall(Function, ...))
        for Index = 2, Results.n do
            Results[Index] = Clean(Results[Index])
        end
        return (if Results[1] then "ok " else "error ") .. table.concat(Results, " ", 2, Results.n)
    end
"##;

/// A script's environment holds `_G`, `script` and `shared` and nothing else, under a locked metatable whose globals hold the one
/// `require` every script reads.
#[test]
fn a_scripts_environment_holds_its_globals_under_a_locked_metatable() {
    let source = format!(
        "{HELPERS}{}",
        r##"
        local EnvA = require(Make("EnvA", "local R = require return { env = getfenv(1), req = R, thread = getfenv(0), keys = function() local K = {} for k in next, getfenv(1) do K[#K + 1] = tostring(k) end return table.concat(K, ',') end } -- env-a"))
        local EnvB = require(Make("EnvB", "return { env = getfenv(1), req = require } -- env-b"))
        local Env = EnvA.env
        local Keys = {}
        for Key in next, EnvB.env do
            table.insert(Keys, tostring(Key))
        end
        print("keys " .. EnvA.keys() .. " | " .. table.concat(Keys, ","))
        local Meta = getmetatable(Env)
        print("metatable " .. type(Meta) .. " " .. Clean(Meta))
        print("setmetatable " .. Try(setmetatable, Env, {}) .. " | nil " .. Try(setmetatable, Env, nil))
        print("frozen " .. tostring(table.isfrozen(Env)) .. " write " .. Try(function()
            Env.ParityWritten = 1
            return rawget(Env, "ParityWritten")
        end))
        print("raw require " .. type(rawget(Env, "require")) .. " script " .. Clean(rawget(Env, "script")) .. " _G " .. type(rawget(Env, "_G")) .. " shared " .. type(rawget(Env, "shared")) .. " game " .. type(rawget(Env, "game")))
        print("require one " .. tostring(rawequal(EnvA.req, EnvB.req)) .. " through-index " .. tostring(rawequal(Env.require, EnvA.req)) .. " here " .. tostring(rawequal(require, EnvA.req)))
        print("_G one " .. tostring(rawequal(rawget(Env, "_G"), rawget(EnvB.env, "_G"))) .. " here " .. tostring(rawequal(rawget(Env, "_G"), _G)) .. " shared one " .. tostring(rawequal(rawget(Env, "shared"), rawget(EnvB.env, "shared"))) .. " here " .. tostring(rawequal(rawget(Env, "shared"), shared)))
        print("chunk thread-env " .. tostring(rawequal(EnvA.thread, Env)))
        print("require " .. tostring(debug.info(EnvA.req, "s")) .. " " .. tostring(debug.info(EnvA.req, "n")))
        "##
    );
    check(
        &run(Options::default(), &source).prints,
        &[
            "keys _G,script,shared | _G,script,shared",
            "metatable string The metatable is locked",
            "setmetatable error cannot change a protected metatable | nil error cannot change a protected metatable",
            "frozen false write ok 1",
            "raw require nil script EnvA _G table shared table game nil",
            "require one true through-index true here true",
            "_G one true here true shared one true here true",
            "chunk thread-env true",
            "require [C] require",
        ],
    );
}

/// require resolves a path from the script the nearest Luau function's environment names, as that environment holds it at the
/// call (a setfenv or a write of `script` moves it), and raises where no Luau function called it.
#[test]
fn require_finds_the_calling_script_at_each_call() {
    let source = format!(
        "{HELPERS}{}",
        r##"
        local F1 = Instance.new("Folder")
        F1.Name = "F1"
        F1.Parent = Folder
        local F2 = Instance.new("Folder")
        F2.Name = "F2"
        F2.Parent = Folder
        Make("Sib", "return 'F1-sib' -- env-s1", F1)
        Make("Sib", "return 'F2-sib' -- env-s2", F2)
        local Other = Make("Other", "return 'F2-other' -- env-o", F2)
        local CallerModule = Make("Caller", table.concat({
            "local R = require",
            "local M = {}",
            "function M.call(p) return require(p) end",
            "function M.upvalue(p) return R(p) end",
            "function M.pcalled(p) return select(2, pcall(require, p)) end",
            "function M.set(s) script = s end",
            "function M.rawset(s) rawset(getfenv(1), 'script', s) end",
            "function M.make() return function(p) return R(p) end end",
            "function M.wrapped(p) return coroutine.wrap(R)(p) end",
            "function M.spawned(p, Out) task.spawn(function() Out.v = select(2, pcall(R, p)) end) end",
            "function M.bare(p) local co = coroutine.create(R) return select(2, coroutine.resume(co, p)) end",
            "return M -- env-caller",
        }, "\n"), F1)
        local Caller = require(CallerModule)
        print("caller call " .. Try(Caller.call, "./Sib") .. " | upvalue " .. Try(Caller.upvalue, "./Sib") .. " | pcalled " .. Try(Caller.pcalled, "./Sib"))
        print("caller wrapped " .. Try(Caller.wrapped, "./Sib"))
        print("caller bare-thread " .. Try(Caller.bare, "./Sib") .. " | instance " .. Try(Caller.bare, Other) .. " | nothing " .. Try(Caller.bare))
        local Out = {}
        Caller.spawned("./Sib", Out)
        task.wait()
        print("caller spawned " .. Clean(Out.v))
        local Moved = Caller.make()
        setfenv(Moved, setmetatable({ script = Other }, { __index = getfenv(Moved) }))
        print("setfenv script " .. Try(Moved, "./Sib"))
        local Bare = Caller.make()
        setfenv(Bare, {})
        print("setfenv empty " .. Try(Bare, "./Sib") .. " | instance " .. Try(Bare, Other))
        for _, Path in { "@self", "@self/x", "@game/ReplicatedStorage/ParityEnv/F1/Sib" } do
            print("setfenv empty [" .. Path .. "] " .. Try(Bare, Path))
        end
        local Indexed = Caller.make()
        setfenv(Indexed, setmetatable({}, { __index = { script = Other } }))
        print("setfenv indexed " .. Try(Indexed, "./Sib"))
        local Folderish = Caller.make()
        setfenv(Folderish, { script = F2 })
        print("setfenv folder " .. Try(Folderish, "./Sib"))
        Caller.set(Other)
        print("script written " .. Try(Caller.call, "./Sib") .. " | upvalue " .. Try(Caller.upvalue, "./Sib"))
        Caller.rawset(CallerModule)
        print("script rawset " .. Try(Caller.call, "./Sib"))
        Caller.set(nil)
        print("script nil " .. Try(Caller.call, "./Sib") .. " | instance " .. Try(Caller.call, Other))
        "##
    );
    check(
        &run(Options::default(), &source).prints,
        &[
            "caller call ok F1-sib | upvalue ok F1-sib | pcalled ok F1-sib",
            "caller wrapped error ReplicatedStorage.ParityEnv.F1.Caller:9: require must be called by a Luau function in this context (e.g. function() require(...) end)",
            "caller bare-thread ok require must be called by a Luau function in this context (e.g. function() require(...) end) | instance ok F2-other | nothing ok Attempted to call require with invalid argument(s).",
            "caller spawned F1-sib",
            "setfenv script ok F2-sib",
            "setfenv empty error error requiring \"./Sib\": could not get parent of requiring context | instance ok F2-other",
            "setfenv empty [@self] error Unable to require module from given path '@self'",
            "setfenv empty [@self/x] error error requiring \"@self/x\": could not resolve child component \"x\"",
            "setfenv empty [@game/ReplicatedStorage/ParityEnv/F1/Sib] ok F1-sib",
            "setfenv indexed error error requiring \"./Sib\": could not get parent of requiring context",
            "setfenv folder error error requiring \"./Sib\": could not resolve child component \"Sib\"",
            "script written ok F2-sib | upvalue ok F2-sib",
            "script rawset ok F1-sib",
            "script nil error error requiring \"./Sib\": could not get parent of requiring context | instance ok F2-other",
        ],
    );
}

/// What require-by-string paths answer, as a client resolves them.
#[test]
fn require_paths_resolve_as_a_clients_do() {
    let source = format!(
        "{HELPERS}{}",
        r##"
        local F1 = Instance.new("Folder")
        F1.Name = "F1"
        F1.Parent = Folder
        local F2 = Instance.new("Folder")
        F2.Name = "F2"
        F2.Parent = Folder
        Make("Sib", "return 'F1-sib' -- env-s1", F1)
        Make("Sib", "return 'F2-sib' -- env-s2", F2)
        local CallerModule = Make("Caller", "local R = require return { call = function(p) return R(p) end } -- env-caller", F1)
        Make("Child", "return 'caller-child' -- env-child", CallerModule)
        Make("Lower", "return 'F1-lower' -- env-lo", F1)
        local Plain = Instance.new("Folder")
        Plain.Name = "Plain"
        Plain.Parent = F1
        local Call = require(CallerModule).call
        for _, Path in {
            "./Missing", "./Sib/", "./Sib.lua", "./LOWER", "./Plain", "./", ".", "", "..", "../", "@self/Child", "@game",
            "@ReplicatedStorage", "@", "/Sib", ".//Sib", "../../../../../../../../../x", "@SELF/Child", "@game/..", "@self/..",
            ".\\Sib", "@game/ReplicatedStorage/../..", "@game/x/..",
        } do
            print("path [" .. Path .. "] " .. Try(Call, Path))
        end
        local Loose = Instance.new("ModuleScript")
        Loose.Name = "Loose"
        Loose.Source = "local R = require return { call = function(p) return R(p) end } -- env-loose"
        local LooseCall = require(Loose).call
        for _, Path in { "./Sib", "../Sib", "@self/x" } do
            print("parentless [" .. Path .. "] " .. Try(LooseCall, Path))
        end
        "##
    );
    check(
        &run(Options::default(), &source).prints,
        &[
            "path [./Missing] error error requiring \"./Missing\": could not resolve child component \"Missing\"",
            "path [./Sib/] ok F1-sib",
            "path [./Sib.lua] error error requiring \"./Sib.lua\": could not resolve child component \"Sib.lua\"",
            "path [./LOWER] error error requiring \"./LOWER\": could not resolve child component \"LOWER\"",
            "path [./Plain] error Unable to require module from given path './Plain'",
            "path [./] error Unable to require module from given path './'",
            "path [.] error error requiring \".\": require path must start with a valid prefix: ./, ../, or @",
            "path [] error error requiring \"\": require path must start with a valid prefix: ./, ../, or @",
            "path [..] error error requiring \"..\": require path must start with a valid prefix: ./, ../, or @",
            "path [../] error Unable to require module from given path '../'",
            "path [@self/Child] ok caller-child",
            "path [@game] error Unable to require module from given path '@game'",
            "path [@ReplicatedStorage] error error requiring \"@ReplicatedStorage\": @replicatedstorage is not a valid alias",
            "path [@] error error requiring \"@\": @ is not a valid alias",
            "path [/Sib] error error requiring \"/Sib\": require path must start with a valid prefix: ./, ../, or @",
            "path [.//Sib] ok F1-sib",
            "path [../../../../../../../../../x] error error requiring \"../../../../../../../../../x\": could not get parent of component \"..\"",
            "path [@SELF/Child] ok caller-child",
            "path [@game/..] error error requiring \"@game/..\": could not get parent of requiring context",
            "path [@self/..] error Unable to require module from given path '@self/..'",
            "path [.\\Sib] ok F1-sib",
            "path [@game/ReplicatedStorage/../..] error error requiring \"@game/ReplicatedStorage/../..\": could not get parent of component \"..\"",
            "path [@game/x/..] error error requiring \"@game/x/..\": could not resolve child component \"x\"",
            "parentless [./Sib] error error requiring \"./Sib\": could not get parent of requiring context",
            "parentless [../Sib] error error requiring \"../Sib\": could not get parent of requiring context",
            "parentless [@self/x] error error requiring \"@self/x\": could not resolve child component \"x\"",
        ],
    );
}

/// A module whose source does not compile logs the compiler's message once, under its own name, with no stack and no
/// ScriptContext.Error, and fails this require and every later one, after its Source is fixed too.
#[test]
fn a_module_that_does_not_compile_logs_its_message_and_fails_every_require() {
    let source = format!(
        "{HELPERS}{}",
        r##"
        local Errors = {}
        game:GetService("ScriptContext").Error:Connect(function(Message)
            table.insert(Errors, Message)
        end)
        local Bad = Make("BadSyntax", "local x = 1\nlocal = 2 -- env-bad")
        print("syntax " .. Try(require, Bad))
        print("syntax again " .. Try(require, Bad))
        Bad.Source = "return 'fixed' -- env-fixed"
        print("syntax fixed " .. Try(require, Bad))
        print("syntax twin " .. Try(require, Make("BadTwin", "local x = 1\nlocal = 2 -- env-bad")))
        task.wait()
        print("script errors " .. #Errors)
        "##
    );
    let ran = run(play(), &source);
    check(
        &ran.prints,
        &[
            "syntax error Requested module experienced an error while loading",
            "syntax again error Requested module experienced an error while loading",
            "syntax fixed error Requested module experienced an error while loading",
            "syntax twin error Requested module experienced an error while loading",
            "script errors 0",
        ],
    );
    check(
        &ran.errors,
        &[
            "ReplicatedStorage.ParityEnv.BadSyntax:2: Expected identifier when parsing variable name, got '='",
            "ReplicatedStorage.ParityEnv.BadTwin:2: Expected identifier when parsing variable name, got '='",
        ],
    );
}

/// A module compiled from a source text that an earlier chunk of the VM was compiled from carries that chunk's name while it
/// lives, however its own module is named; a collection that finds the first chunk unreferenced ends it, and a source that does
/// not compile names no chunk.
#[test]
fn a_chunk_carries_the_name_of_the_first_chunk_of_its_text_while_that_lives() {
    let source = format!(
        "{HELPERS}{}",
        r##"
        local function Held(Tag)
            return "script:SetAttribute('Chunk', debug.info(1, 's')) shared.ParityKeep[script] = debug.info(1, 'f') return 0 -- " .. Tag
        end
        local function Req(Module)
            local Ok, Problem = pcall(require, Module)
            if not Ok then
                return "error " .. tostring(Problem)
            end
            return tostring(Module:GetAttribute("Chunk"))
        end
        local function Cycle()
            local Watch = setmetatable({}, { __mode = "k" })
            Watch[{}] = true
            local Rounds = 0
            while next(Watch) ~= nil and Rounds < 4000 do
                local Junk = {}
                for Index = 1, 5000 do
                    Junk[Index] = { Index }
                end
                Rounds += 1
                if Rounds % 50 == 0 then
                    task.wait()
                end
            end
            return next(Watch) == nil
        end
        shared.ParityKeep = {}
        local Renamed = Make("BeforeRename", "return { f = function() return debug.info(1, 's') end } -- share-rename")
        local RenamedValue = require(Renamed)
        Renamed.Name = "AfterRename"
        print("renamed " .. RenamedValue.f() .. " | twin " .. require(Make("Twin", "return { f = function() return debug.info(1, 's') end } -- share-rename")).f())
        print("held " .. Req(Make("Held1", Held("share-held"))) .. " | cycle " .. tostring(Cycle()) .. " twin " .. Req(Make("Held2", Held("share-held"))))
        local Pair1 = Make("Pair1", Held("share-pair"))
        local Pair2 = Make("Pair2", Held("share-pair"))
        print("pair " .. Req(Pair1) .. " | " .. Req(Pair2))
        shared.ParityKeep[Pair1] = nil
        print("pair cycle " .. tostring(Cycle()) .. " third " .. Req(Make("Pair3", Held("share-pair"))) .. " | fourth " .. Req(Make("Pair4", Held("share-pair"))))
        local None1 = Make("None1", Held("share-none"))
        print("none " .. Req(None1))
        shared.ParityKeep[None1] = nil
        print("none cycle " .. tostring(Cycle()) .. " twin " .. Req(Make("None2", Held("share-none"))))
        print("other text " .. Req(Make("Spaced", Held("share-held") .. " ")))
        print("syntax " .. Req(Make("Bad1", "local = 1 -- share-syntax")) .. " | " .. Req(Make("Bad2", "local = 1 -- share-syntax")))
        print("runtime " .. Req(Make("Err1", "error('share-err') -- share-err")) .. " | " .. Req(Make("Err2", "error('share-err') -- share-err")))
        "##
    );
    let ran = run(play(), &source);
    check(
        &ran.prints,
        &[
            "renamed ReplicatedStorage.ParityEnv.BeforeRename | twin ReplicatedStorage.ParityEnv.BeforeRename",
            "held ReplicatedStorage.ParityEnv.Held1 | cycle true twin ReplicatedStorage.ParityEnv.Held1",
            "pair ReplicatedStorage.ParityEnv.Pair1 | ReplicatedStorage.ParityEnv.Pair1",
            "pair cycle true third ReplicatedStorage.ParityEnv.Pair3 | fourth ReplicatedStorage.ParityEnv.Pair3",
            "none ReplicatedStorage.ParityEnv.None1",
            "none cycle true twin ReplicatedStorage.ParityEnv.None2",
            "other text ReplicatedStorage.ParityEnv.Spaced",
            "syntax error Requested module experienced an error while loading | error Requested module experienced an error while loading",
            "runtime error Requested module experienced an error while loading | error Requested module experienced an error while loading",
        ],
    );
    let first_lines: Vec<String> = ran.errors.iter().map(|error| error.lines().next().unwrap_or("").to_string()).collect();
    check(
        &first_lines,
        &[
            "ReplicatedStorage.ParityEnv.Bad1:1: Expected identifier when parsing variable name, got '='",
            "ReplicatedStorage.ParityEnv.Bad2:1: Expected identifier when parsing variable name, got '='",
            "ReplicatedStorage.ParityEnv.Err1:1: share-err",
            "ReplicatedStorage.ParityEnv.Err1:1: share-err",
        ],
    );
}

/// debug.loadmodule's chunks are named by their module at each load: they take no earlier chunk's name and give none.
#[test]
fn loadmodule_chunks_neither_take_nor_give_a_shared_name() {
    let source = format!(
        "{HELPERS}{}",
        r##"
        local Source = "return debug.info(1, 's') -- share-load-a"
        local Loaded = debug.loadmodule(Make("LoadedFirst", Source))
        print("loaded first " .. Loaded() .. " | require " .. require(Make("RequiredAfter", Source)) .. " | load again " .. debug.loadmodule(Make("LoadedAgain", Source))())
        local Other = "return debug.info(1, 's') -- share-load-b"
        print("required first " .. require(Make("RequiredFirst", Other)) .. " | load " .. debug.loadmodule(Make("LoadedAfter", Other))() .. " | require again " .. require(Make("RequiredAgain", Other)))
        "##
    );
    check(
        &run(Options { load_module: true, ..Options::default() }, &source).prints,
        &[
            "loaded first ReplicatedStorage.ParityEnv.LoadedFirst | require ReplicatedStorage.ParityEnv.RequiredAfter | load again ReplicatedStorage.ParityEnv.LoadedAgain",
            "required first ReplicatedStorage.ParityEnv.RequiredFirst | load ReplicatedStorage.ParityEnv.LoadedAfter | require again ReplicatedStorage.ParityEnv.RequiredFirst",
        ],
    );
}

/// An Actor's scripts have a `_G` and a `shared` of their own (the main VM's values are not theirs), an environment of the same
/// shape and their VM's one require.
#[test]
fn an_actors_scripts_have_their_own_globals() {
    let source = format!(
        "{HELPERS}{}",
        r##"
        shared.ParityMark = "main-shared"
        _G.ParityMark = "main-G"
        local Actor = Instance.new("Actor")
        local Inside = Make("ActorScript", table.concat({
            "script:SetAttribute('Shared', tostring(shared.ParityMark)) script:SetAttribute('G', tostring(_G.ParityMark))",
            "local E = getfenv(1) local K = {} for k in next, E do K[#K + 1] = tostring(k) end",
            "script:SetAttribute('Keys', table.concat(K, ',')) script:SetAttribute('Meta', type(getmetatable(E)) .. ' ' .. tostring(getmetatable(E)))",
            "script:SetAttribute('ReqOne', tostring(rawequal(require(script.M).req, require)))",
        }, "\n"), Actor, "LocalScript")
        Make("M", "return { req = require } -- env-actor-m", Inside)
        Actor.Parent = game:GetService("Players").LocalPlayer:WaitForChild("PlayerGui")
        task.wait()
        local function Seen(Name)
            return tostring(Inside:GetAttribute(Name))
        end
        print("actor shared " .. Seen("Shared") .. " _G " .. Seen("G") .. " keys " .. Seen("Keys") .. " metatable " .. Seen("Meta") .. " require one " .. Seen("ReqOne"))
        print("actor after shared " .. tostring(shared.ParityMark) .. " _G " .. tostring(_G.ParityMark))
        "##
    );
    check(
        &run(play(), &source).prints,
        &[
            "actor shared nil _G nil keys _G,script,shared metatable string The metatable is locked require one true",
            "actor after shared main-shared _G main-G",
        ],
    );
}
