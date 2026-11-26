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
