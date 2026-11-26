//! Engine functions' arguments and answers as a 0.747 playing client and a server give them (tools/parity/probes
//! legacy_spawn_args, tween_args, primary_part_cframe, method_values, humanoid_description_emotes, memory_category, yield_refused,
//! arg_checks): the legacy spawn and delay, TweenService:Create and TweenInfo, the GUI tween methods, enum arguments,
//! task.synchronize outside an Actor, memory categories, method values, a Luau method's self, Model's primary part CFrame,
//! HumanoidDescription's emotes, and the stubs that only check their arguments.

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

fn errors(lines: &[String]) -> Vec<&String> {
    lines.iter().filter(|line| line.starts_with("error")).collect()
}

/// The legacy spawn and delay take any value; when it is due a table with __call is called with itself and the two numbers, and
/// any other value logs `attempt to call a <type> value` with no stack (MessageOut's text the message, a line break and four
/// spaces; ScriptContext.Error the message, an empty trace and the calling script). spawn() and delay with fewer than two values
/// raise.
#[test]
fn legacy_spawn_and_delay_call_any_value_when_it_is_due() {
    for context in [RunContext::Client, RunContext::Server] {
        let lines = run(
            context,
            r##"
            local Out, Errors = {}, {}
            game:GetService("LogService").MessageOut:Connect(function(Message, Kind)
                if Kind == Enum.MessageType.MessageError then table.insert(Out, Message) end
            end)
            game:GetService("ScriptContext").Error:Connect(function(Message, Trace, Script)
                table.insert(Errors, Message .. "|" .. Trace .. "|" .. tostring(Script))
            end)
            local Got
            local Callable = setmetatable({}, { __call = function(Self, ...) Got = { rawequal(Self, Got), select("#", ...), typeof((...)) } end })
            Got = Callable
            spawn(5)
            delay(0, coroutine.create(function() end))
            spawn(Callable)
            assert(select(2, pcall(spawn)) == "Spawn function requires 1 argument")
            assert(select(2, pcall(Spawn)) == "Spawn function requires 1 argument")
            assert(select(2, pcall(delay, 1)) == "delay function requires 2 arguments")
            assert(pcall(spawn, nil) and pcall(delay, "x", nil))
            task.wait(0.3)
            assert(Got[1] == true and Got[2] == 2 and Got[3] == "number", "the callable gets itself and two numbers")
            local Text = table.concat(Out, ";")
            assert(string.find(Text, "attempt to call a number value\n    ", 1, true) and not string.find(Text, "Stack Begin", 1, true), Text)
            assert(string.find(Text, "attempt to call a thread value\n    ", 1, true), Text)
            assert(string.find(table.concat(Errors, ";"), "attempt to call a number value||Test", 1, true), table.concat(Errors, ";"))
            print("done")
            "##,
            1.0,
        );
        assert!(lines.iter().any(|line| line == "print: done"), "{context:?}: {lines:?}");
        assert!(lines.contains(&"error: attempt to call a number value".to_string()), "{lines:?}");
        assert!(lines.contains(&"error: attempt to call a nil value".to_string()), "{lines:?}");
    }
}

/// TweenService:Create casts the goals, the TweenInfo and the instance in that order; a non-empty string is a TweenInfo of its
/// number's time; no instance and no goals make a tween that plays without end. TweenInfo writes 0/0 as MSVC does and holds
/// RepeatCount in 32 bits. The GUI tween methods cast their callback and override first, then the time, style, direction and
/// UDim2s.
#[test]
fn tween_methods_cast_their_arguments_as_a_client() {
    let lines = run(
        RunContext::Client,
        r##"
        local TweenService = game:GetService("TweenService")
        local V = Instance.new("NumberValue")
        local function E(...) return select(2, pcall(TweenService.Create, TweenService, ...)) end
        assert(E(nil, nil, nil) == "Argument 3 missing or nil")
        assert(E("x", "info", "x") == "Unable to cast string to Dictionary")
        assert(E(V, TweenInfo.new(), { 1 }) == "Unable to cast Array to Dictionary")
        assert(E(V, 5, { [true] = 1 }) == "Invalid table key type used")
        assert(E("x", 5, {}) == "Unable to cast double to TweenInfo")
        assert(E(V, {}, {}) == "Unable to cast Array to TweenInfo" and E(V, { a = 1 }, {}) == "Unable to cast Dictionary to TweenInfo")
        assert(E(V, Enum.EasingStyle.Linear, {}) == "Unable to cast EasingStyle to TweenInfo" and E(V, newproxy(), {}) == "Unable to cast null to TweenInfo")
        assert(E(V, "", {}) == "Unable to cast string to TweenInfo")
        assert(E(5, "info", {}) == "Unable to cast value to Object")
        assert(E(nil, TweenInfo.new(), { Value = 1 }) == "TweenService:Create failed because Instance is null")
        local Quick = TweenService:Create(V, "x", { Value = 10 })
        assert(Quick.TweenInfo.Time == 0)
        Quick:Play()
        local Loose = TweenService:Create(nil, TweenInfo.new(0.05), {})
        assert(Loose.Instance == nil)
        Loose:Play()
        task.wait(0.3)
        assert(V.Value == 10 and Quick.PlaybackState == Enum.PlaybackState.Completed)
        assert(Loose.PlaybackState == Enum.PlaybackState.Playing, tostring(Loose.PlaybackState))
        Loose:Cancel()
        assert(Loose.PlaybackState == Enum.PlaybackState.Cancelled)
        assert(string.find(tostring(TweenInfo.new(0 / 0)), "Time:-nan(ind) ", 1, true) and string.find(tostring(TweenInfo.new(-(0 / 0))), "Time:nan ", 1, true))
        assert(TweenInfo.new(1, Enum.EasingStyle.Linear, Enum.EasingDirection.In, 2 ^ 40).RepeatCount == -2147483648)
        assert(TweenInfo.new(1, Enum.EasingStyle.Linear, Enum.EasingDirection.In, 0 / 0).RepeatCount == -2147483648)
        local Frame = Instance.new("Frame")
        Frame.Parent = game:GetService("ReplicatedStorage")
        local To = UDim2.fromOffset(5, 5)
        local function G(...) return select(2, pcall(Frame.TweenPosition, Frame, ...)) end
        assert(G(To, nil, nil, nil, "x") == "Unable to cast string to bool" and G(To, nil, nil, nil, {}) == "Unable to cast Array to bool")
        assert(G(To, nil, nil, nil, true, 5) == "Unable to cast double to Function" and G(To, nil, nil, nil, nil, setmetatable({}, { __call = print })) == "Unable to cast Array to Function")
        assert(G("x", nil, nil, nil, "x", "x") == "Unable to cast string to Function")
        assert(G(To, "bad", 13, "x") == "Unable to cast string to float" and G(To, "bad", 13) == "Unable to cast double to EasingStyle")
        assert(select(2, pcall(Frame.TweenSizeAndPosition, Frame, "x", 5)) == "Unable to cast double to UDim2")
        assert(G(To, nil, nil, 0.01, 0) == true and G(To, nil, nil, 0.01, false, print) == true)
        print("done")
        "##,
        0.5,
    );
    assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{lines:?}");
}

/// A method's enum argument takes a number cut toward zero to an item's value (SetCameraPanMode(0.5) is Classic's 0, 1.99
/// EdgeBump's 1; 2.5 is no item); a property write did already.
#[test]
fn enum_arguments_take_a_fraction_cut_toward_zero() {
    let lines = run(
        RunContext::Client,
        r##"
        local Camera = workspace.CurrentCamera
        assert(pcall(Camera.SetCameraPanMode, Camera, 0.5) and pcall(Camera.SetCameraPanMode, Camera, 1.99) and pcall(Camera.SetCameraPanMode, Camera, -0.99))
        assert(select(2, pcall(Camera.SetCameraPanMode, Camera, 2.5)) == "Unable to cast double to CameraPanMode")
        local Part = Instance.new("Part")
        Part.Parent = workspace
        assert(pcall(workspace.BulkMoveTo, workspace, { Part }, { CFrame.new() }, 0.5))
        print("done")
        "##,
        0.1,
    );
    assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{lines:?}");
}
