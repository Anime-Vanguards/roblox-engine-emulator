//! Small rules measured on a 0.747 playing client and a server: the animation providers' registration in a live game,
//! GuiService:GetScreenResolution and TouchControlsEnabled, a server's cameras, GetTranslatorForLocaleAsync's wait, the
//! datatypes' method values, where Euler angles lock and a part's rounded angles, Attachment axis writes, and the mouse wheel's
//! InputActions (tools/parity/probes animation_members, gui_screen, camera_headless, translator_wait, method_values, euler_lock,
//! attachment_axes).

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Engine, Options, RunContext,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

#[inline(never)]
fn start(options: Options) -> (Lua, Rc<Engine>) {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

fn play() -> Options {
    Options { mode: "play".into(), ..Options::default() }
}

fn server() -> Options {
    Options { mode: "play".into(), run_context: RunContext::Server, local_player: false, ..Options::default() }
}

/// Runs `source` as a script of `engine`, then the clock `seconds` on and the script to its end: the output lines ("kind:
/// message"). A script error fails the test.
#[inline(never)]
fn run_in(lua: &Lua, engine: &Rc<Engine>, source: &str, seconds: f64) -> Vec<String> {
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
    scheduler::run_until(lua, engine, now + seconds).unwrap();
    if let Err(message) = call.finish(lua, engine) {
        panic!("test script failed: {message}");
    }
    let lines = engine.state.borrow().output.entries.iter().map(|e| format!("{}: {}", e.kind, e.message)).collect();
    lines
}

fn run(options: Options, source: &str, seconds: f64) -> Vec<String> {
    let (lua, engine) = start(options);
    run_in(&lua, &engine, source, seconds)
}

fn done(lines: &[String]) {
    assert!(lines.iter().any(|line| line == "print: done") && !lines.iter().any(|line| line.starts_with("error")), "{lines:?}");
}

/// A live game (`--studio false`) refuses the providers' registration before its argument; a Studio session registers.
#[test]
fn registering_a_clip_is_refused_in_a_live_game() {
    let source = r#"
        local KSP = game:GetService("KeyframeSequenceProvider")
        local ACP = game:GetService("AnimationClipProvider")
        local KS = Instance.new("KeyframeSequence")
        local function Try(F) local Ok, R = pcall(F) return if Ok then "ok " .. typeof(R) else R end
        print(Try(function() return KSP:RegisterKeyframeSequence(KS) end))
        print(Try(function() return KSP:RegisterKeyframeSequence(nil) end))
        print(Try(function() return KSP:RegisterActiveKeyframeSequence(KS) end))
        print(Try(function() return ACP:RegisterAnimationClip(KS) end))
        print(Try(function() return ACP:RegisterActiveAnimationClip(KS) end))
    "#;
    let live = run(Options { studio: false, ..play() }, source, 0.0);
    assert_eq!(
        live,
        [
            "print: Usage error: RegisterKeyframeSequence can only be called in Studio.",
            "print: Usage error: RegisterKeyframeSequence can only be called in Studio.",
            "print: Usage error: RegisterActiveKeyframeSequence can only be used in Solo mode.",
            "print: Usage error: RegisterAnimationClip can only be called in Studio.",
            "print: Usage error: RegisterActiveKeyframeSequence can only be used in Solo mode.",
        ]
    );
    let studio = run(play(), source, 0.0);
    assert_eq!(studio[0], "print: ok string", "{studio:?}");
    assert_eq!(studio[2], "print: ok string", "{studio:?}");
}

/// GetScreenResolution is a core script's: a game script gets the capability error on both sides. TouchControlsEnabled starts true;
/// a write takes the value's truth and fires only when it changes it.
#[test]
fn screen_resolution_and_touch_controls_answer_as_a_client() {
    for options in [play(), server()] {
        let lines = run(
            options,
            r#"
            local GS = game:GetService("GuiService")
            local Ok, Problem = pcall(function() return GS:GetScreenResolution() end)
            assert(not Ok and Problem == "The current thread cannot call 'GetScreenResolution' (lacking capability RobloxScript)", Problem)
            assert(GS.TouchControlsEnabled == true, "starts on")
            local Log = {}
            GS:GetPropertyChangedSignal("TouchControlsEnabled"):Connect(function() table.insert(Log, tostring(GS.TouchControlsEnabled)) end)
            for _, Box in { { false }, { false }, { true }, { 0 }, { "" }, { nil }, { {} }, { true } } do
                GS.TouchControlsEnabled = Box[1]
                task.wait()
            end
            assert(table.concat(Log, ",") == "false,true,false,true", table.concat(Log, ","))
            local UIS = game:GetService("UserInputService")
            UIS.ModalEnabled = true
            assert(UIS.ModalEnabled == true and GS.TouchControlsEnabled == false, "modal hides the touch controls")
            UIS.ModalEnabled = false
            assert(GS.TouchControlsEnabled == true, "and shows them")
            print("done")
            "#,
            1.0,
        );
        done(&lines);
    }
}
