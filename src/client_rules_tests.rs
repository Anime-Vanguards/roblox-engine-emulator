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

/// A server has no window: its start camera and a script's cameras read 1 x 1 at NearPlaneZ -0.5, a camera the engine makes for a
/// CurrentCamera that left the Workspace 800 x 600, one it makes for a CurrentCamera written nil 1 x 1.
#[test]
fn a_servers_cameras_have_no_window() {
    let lines = run(
        server(),
        r#"
        local function Size(C) return C.ViewportSize.X .. "x" .. C.ViewportSize.Y .. " " .. C.NearPlaneZ end
        local Start = workspace.CurrentCamera
        assert(Size(Start) == "1x1 -0.5", Size(Start))
        local Made = Instance.new("Camera")
        Made.Parent = workspace
        workspace.CurrentCamera = Made
        task.wait()
        Made:Destroy()
        task.wait()
        local Replaced = workspace.CurrentCamera
        assert(Replaced ~= Made and Replaced ~= Start and Size(Replaced) == "800x600 -0.5", Size(Replaced))
        Replaced.Parent = nil
        workspace.CurrentCamera = nil
        task.wait()
        local Fresh = workspace.CurrentCamera
        assert(Fresh ~= Replaced and Size(Fresh) == "1x1 -0.5", Size(Fresh))
        print("done")
        "#,
        1.0,
    );
    done(&lines);
    // (a client keeps its window: the start camera has the screen)
    let lines = run(play(), r#"assert(workspace.CurrentCamera.ViewportSize.X > 1 and math.abs(workspace.CurrentCamera.NearPlaneZ + 0.1) < 1e-6) print("done")"#, 0.0);
    done(&lines);
}

/// GetTranslatorForLocaleAsync casts its locale, raises for fewer than two characters at once, refuses a thread that cannot yield,
/// and answers after 10 s, calls made together waiting together.
#[test]
fn translator_for_locale_waits_ten_seconds() {
    let lines = run(
        play(),
        r#"
        local L = game:GetService("LocalizationService")
        local Line
        table.sort({ 2, 1 }, function(A, B)
            if not Line then Line = select(2, pcall(function() return L:GetTranslatorForLocaleAsync({}) end)) end
            return A < B
        end)
        assert(Line == "thread is not yieldable", Line)
        assert(select(2, pcall(function() return L:GetTranslatorForLocaleAsync("x") end)) == "Too few characters in locale id!")
        local Done = {}
        local T0 = time()
        for _, Locale in { "fr-fr", true } do
            task.spawn(function()
                local T = L:GetTranslatorForLocaleAsync(Locale)
                table.insert(Done, T.LocaleId .. " " .. string.format("%.1f", time() - T0))
            end)
        end
        task.wait(9.9)
        assert(#Done == 0, "still waiting")
        task.wait(0.2)
        table.sort(Done)
        assert(table.concat(Done, ",") == "fr-fr 10.0,true 10.0", table.concat(Done, ","))
        print("done")
        "#,
        11.0,
    );
    done(&lines);
}

/// A CFrame's, Color3's, UDim2's and Vector2's method read as a value is a new function at every read, which still works; fields,
/// `:` calls and the unknown-member error are as they were.
#[test]
fn datatype_method_values_are_new_functions() {
    let lines = run(
        play(),
        r#"
        local Cf, C3, U2, V2 = CFrame.new(1, 2, 3), Color3.new(0.1, 0.2, 0.3), UDim2.new(1, 2, 3, 4), Vector2.new(1, 2)
        assert(not rawequal(Cf.Lerp, Cf.Lerp) and not rawequal(Cf.Inverse, CFrame.identity.Inverse) and not rawequal(C3.Lerp, C3.Lerp))
        assert(not rawequal(U2.Lerp, U2.Lerp) and not rawequal(V2.Lerp, V2.Lerp) and type(Cf.Lerp) == "function")
        assert(Cf.Inverse(Cf) == Cf:Inverse() and Cf.Position == Vector3.new(1, 2, 3) and C3.R == C3.r and U2.X.Offset == 2 and V2.X == 1)
        assert(V2.Lerp(Vector2.zero, Vector2.one, 0.5) == Vector2.one / 2)
        assert(select(2, pcall(function() return Cf.Nope end)) == "Nope is not a valid member of CFrame")
        assert(select(2, pcall(function() return V2:Nope() end)) == "Nope is not a valid member of Vector2")
        print("done")
        "#,
        0.0,
    );
    done(&lines);
}

/// Euler angles lock when the middle term is within seven float steps of 1 (eight is open), whatever the rest of the row; a part's
/// Orientation and Rotation are those angles to the thousandth of a degree, halves up; an Attachment's are not rounded.
#[test]
fn euler_angles_lock_seven_steps_from_one_and_a_part_rounds_them() {
    let lines = run(
        play(),
        r#"
        local Half = math.pi / 2
        local function Middle(K)
            local E = 1 - K * 2 ^ -24
            local S = math.sqrt(1 - E * E)
            local _, Y = CFrame.new(0, 0, 0, S, 0, E, 0, 1, 0, -E, 0, S):ToEulerAnglesXYZ()
            local X = CFrame.new(0, 0, 0, 1, 0, 0, 0, S, -E, 0, E, S):ToOrientation()
            return Y, X
        end
        local Y7, X7 = Middle(7)
        local Y8, X8 = Middle(8)
        assert(math.abs(Y7 - Half) < 1e-7 and math.abs(X7 - Half) < 1e-7, "seven steps lock")
        assert(Y8 < Half - 1e-4 and X8 < Half - 1e-4, "eight are open")
        local _, Wide = CFrame.new(0, 0, 0, 0.5, 0.5, 0.9999999, 0, 1, 0, 0, 0, 1):ToEulerAnglesXYZ()
        assert(math.abs(Wide - Half) < 1e-7, "the entry decides")
        local Part = Instance.new("Part")
        Part.CFrame = CFrame.fromOrientation(math.rad(10.12345), math.rad(20.6789), math.rad(30.5555))
        local O = Part.Orientation
        assert(string.format("%.9g %.9g %.9g", O.X, O.Y, O.Z) == "10.1230001 20.6790009 30.5559998", tostring(O))
        Part.CFrame = CFrame.fromOrientation(math.rad(0.00049), math.rad(-0.0004), math.rad(-0.00051))
        O = Part.Orientation
        assert(O.X == 0 and O.Y == 0 and 1 / O.Y > 0 and string.format("%.9g", O.Z) == "-0.00100000005", tostring(O))
        local Att = Instance.new("Attachment")
        Att.CFrame = CFrame.fromOrientation(math.rad(10.12345), 0, 0)
        assert(math.abs(Att.Orientation.X - 10.12345) < 1e-4 and Att.Orientation.X ~= 10.123, tostring(Att.Orientation))
        Part.CFrame = CFrame.fromOrientation(math.rad(89.95), math.rad(30), math.rad(20))
        assert(Part.Orientation == Vector3.new(90, 10, 0), tostring(Part.Orientation))
        print("done")
        "#,
        0.0,
    );
    done(&lines);
}
