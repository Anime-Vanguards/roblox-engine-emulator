//! Engine tests through the script API (ports of runtime/tests/{datatypes,modules,scheduler,
//! smoke}.luau and the probe expectations the differential harness checks against the Luau
//! runtime). Each test starts a fresh engine without a project.

use std::rc::Rc;

use mlua::prelude::*;

use super::{
    Engine, Options,
    lua::{self as engine_lua, globals::CallOutcome},
    scheduler,
};

fn start() -> (Lua, Rc<Engine>) {
    start_with(Options::default())
}

fn start_with(options: Options) -> (Lua, Rc<Engine>) {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    // ModuleScript.Source is not scriptable: tests set sources through the engine
    let e = engine.clone();
    let set_source = lua
        .create_function(move |_, (module, source): (LuaAnyUserData, LuaString)| {
            let id = engine_lua::raw::instance_of_ud(&module).ok_or_else(|| LuaError::runtime("Instance expected"))?;
            e.state.borrow_mut().dom.at_mut(id).set_source(Some(Rc::from(&*source.as_bytes())));
            Ok(())
        })
        .unwrap();
    engine.lua.borrow().shared.as_ref().unwrap().raw_set("__setSource", set_source).unwrap();
    (lua, engine)
}

/// Runs `source` on a scheduler thread with the shared globals to its end (while it waits the engine steps frames, failing the
/// test when it is still waiting after a minute of them: lua/globals.rs Call::finish); returns the output lines ("kind: message")
/// after flushing deferred work.
fn run(lua: &Lua, engine: &Rc<Engine>, source: &str) -> Vec<String> {
    let call = start_script(lua, engine, source);
    if let Err(message) = call.finish(lua, engine) {
        panic!("test script failed: {message}");
    }
    lines(engine)
}

/// Runs `source` on a scheduler thread with the shared globals until it first yields or ends, for a test that steps the frames
/// after it itself (what follows a wait runs in those frames, and only the test's own checks look at it). Panics with the
/// script's error.
fn start_script(lua: &Lua, engine: &Rc<Engine>, source: &str) -> engine_lua::globals::Call {
    // A script environment (script = a detached ModuleScript named Test, require)
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
    call
}

fn lines(engine: &Engine) -> Vec<String> {
    engine.state.borrow().output.entries.iter().map(|e| format!("{}: {}", e.kind, e.message)).collect()
}

fn assert_script(source: &str) {
    let (lua, engine) = start();
    let output = run(&lua, &engine, source);
    let errors: Vec<&String> = output.iter().filter(|l| l.starts_with("error")).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// Scripts compute in Roblox's float mode (lua/float_mode.rs; measured in Studio 0.747: tools/parity/probes/float_flush.luau): a
/// subnormal result is zero while a subnormal operand still reads as itself, on a server everywhere and on a client and in edit
/// mode everywhere but the render step (RenderStepped, PreRender, a binding, what they defer, a tween's Changed handlers on a
/// client), which keeps subnormals; constants fold to zero in every context. The host's own arithmetic keeps them throughout.
#[test]
fn scripts_flush_subnormal_results_where_roblox_does_and_keep_them_in_a_clients_render_step() {
    use super::RunContext;
    let script = r##"
        local Stored = buffer.create(8)
        buffer.writeu32(Stored, 0, 1)
        buffer.writeu32(Stored, 4, 0)
        local function Mode()
            local Sub = buffer.readf64(Stored, 0) -- the smallest subnormal, read at run time
            -- (times 3: native code drops a multiplication by 1, as Luau's code generator does in Roblox's native code too)
            return (if Sub * 3 ~= 0 then "keep" else "flush") .. (if Sub > 0 then "" else " operands")
        end
        local RunService = game:GetService("RunService")
        local Seen = { start = Mode(), folded = if 2 ^ -1074 == 0 and 5e-324 * 1 == 0 then "flush" else "keep" }
        _G.Seen = Seen
        local function Note(Name)
            Seen[Name] = Seen[Name] or Mode()
        end
        RunService.Heartbeat:Connect(function() Note("heartbeat") end)
        local Value = Instance.new("NumberValue")
        Value.Changed:Connect(function() Note("tween") end)
        game:GetService("TweenService"):Create(Value, TweenInfo.new(0.1), { Value = 1 }):Play()
        pcall(function()
            RunService.RenderStepped:Connect(function()
                Note("renderStepped")
                task.defer(function() Note("renderDeferred") end)
            end)
            RunService:BindToRenderStep("Probe", 100, function() Note("binding") end)
        end)
        task.delay(0.05, function() Note("delay") end)
        "##;
    let flushes = super::lua::float_mode::supported();
    for (mode, run_context, render, tween) in [("play", RunContext::Client, true, "keep"), ("play", RunContext::Server, false, "flush"), ("edit", RunContext::Edit, true, "keep")] {
        let (lua, engine) = start_with(Options { mode: mode.into(), run_context, ..Options::default() });
        run(&lua, &engine, script);
        scheduler::run_until(&lua, &engine, 0.3).unwrap();
        let output = run(&lua, &engine, "local Lines = {} for Name, Mode in _G.Seen do table.insert(Lines, Name .. '=' .. Mode) end table.sort(Lines) print(table.concat(Lines, ','))");
        let got: Vec<String> = output.last().unwrap().trim_start_matches("print: ").split(',').map(String::from).collect();
        let (flush, keep) = if flushes { ("flush", "keep") } else { ("keep", "keep") };
        let mut want: Vec<String> = vec![format!("delay={flush}"), format!("folded={flush}"), format!("heartbeat={flush}"), format!("start={flush}"), format!("tween={}", if flushes { tween } else { keep })];
        if render {
            want.extend([format!("binding={keep}"), format!("renderDeferred={keep}"), format!("renderStepped={keep}")]);
        }
        want.sort();
        assert_eq!(got, want, "{mode} {run_context:?}");
        assert_eq!(std::hint::black_box(5e-324_f64) * std::hint::black_box(1.0), 5e-324, "the host's arithmetic keeps subnormals ({mode} {run_context:?})");
    }
}

/// A playing client's mouse and gamepad sensitivities start at 1 (measured: tools/parity/probes/settings.luau); they
/// read 0 before (a service has no reflection defaults, and rbx-dom leaves UserGameSettings' numbers out).
#[test]
fn services_start_at_a_clients_sensitivities() {
    assert_script(
        r#"
        local Settings = UserSettings():GetService("UserGameSettings")
        assert(Settings.MouseSensitivity == 1 and Settings.GamepadCameraSensitivity == 1, tostring(Settings.MouseSensitivity))
        assert(game:GetService("UserInputService").MouseDeltaSensitivity == 1)
        "#,
    );
}

#[test]
fn instances_properties_and_errors() {
    assert_script(
        r#"
        local F = Instance.new("Frame")
        assert(F.Name == "Frame" and F.ClassName == "Frame" and F.Parent == nil)
        assert(typeof(F) == "Instance" and type(F) == "userdata")
        assert(getmetatable(F) == "The metatable is locked")
        assert(tostring(F) == "Frame")
        assert(F.BackgroundTransparency == 0 and F.Visible == true and F.ZIndex == 1)
        F.BackgroundTransparency = 0.1
        assert(F.BackgroundTransparency == 0.10000000149011612)
        F.Size = UDim2.fromOffset(10.9, -3.2)
        assert(F.Size == UDim2.new(0, 10, 0, -3))
        local Label = Instance.new("TextLabel")
        Label.TextSize = 10.7
        assert(Label.TextSize == 10)
        Label.TextSize = 150
        assert(Label.TextSize == 100)
        local UIScale = Instance.new("UIScale")
        UIScale.Scale = 0
        assert(UIScale.Scale == 1.0000000116860974e-07)
        UIScale.Scale = -1
        assert(UIScale.Scale == 1.0000000116860974e-07)
        UIScale.Scale = math.huge
        assert(UIScale.Scale == 1000)
        UIScale.Scale = 0.885
        assert(UIScale.Scale == 0.88499999046325684)
        Label.Name = 12
        assert(Label.Name == "12")
        Label.Parent = F
        assert(F:FindFirstChild("12") == Label and F["12"] == Label)
        assert(Label:GetFullName() == "Frame.12")
        assert(Label:IsDescendantOf(F) and F:IsAncestorOf(Label) and Label:IsA("GuiObject") and not Label:IsA("Frame"))
        assert(Label:FindFirstAncestorOfClass("Frame") == F and F:FindFirstChildWhichIsA("GuiObject") == Label)
        assert(#F:GetChildren() == 1 and #F:GetDescendants() == 1)

        local function Message(Callback)
            local Ok, Err = pcall(Callback)
            assert(not Ok and type(Err) == "string")
            -- a script position prefix is kept as "@: "
            return (string.gsub(Err, "^Test:%d+: ", "@: "))
        end
        assert(Message(function() return F.Nope end) == 'Nope is not a valid member of Frame "Frame"')
        assert(Message(function() F.Size = 5 end) == "Unable to assign property Size. UDim2 expected, got number")
        -- (a name the enum lacks: measured, tools/parity/probes/values.luau's `Invalid value "wood" for enum Material`)
        assert(Message(function() F.BorderMode = "Nope" end) == 'Unable to assign property BorderMode. Invalid value "Nope" for enum BorderMode')
        assert(Message(function() F.BorderMode = Enum.Font.Arial end) == "Unable to assign property BorderMode. EnumItem of type BorderMode expected, got an EnumItem of type Font")
        assert(Message(function() F.AbsoluteSize = Vector2.new() end) == "Unable to assign property AbsoluteSize. Property is read only")
        -- (any value goes into a boolean or number property, as in a playing client: Lua truth, Luau's number reading or 0)
        F.Visible = 0
        assert(F.Visible == true)
        F.BackgroundTransparency = "0.5"
        assert(F.BackgroundTransparency == 0.5)
        F.BackgroundTransparency = "x"
        assert(F.BackgroundTransparency == 0)
        assert(Message(function() F.Parent = 5 end) == "@: invalid argument #3 (Instance expected, got number)")
        -- (measured, tools/parity/probes/hierarchy.luau)
        assert(Message(function() F.Parent = F end) == "Attempt to set Frame as its own parent")
        assert(Message(function() return Instance.new("NotAClass") end) == 'Unable to create an Instance of type "NotAClass"')
        assert(Message(function() return F:GetPropertyChangedSignal("Nope") end) == "Nope is not a valid property name.")
        assert(Message(function() F:SetAttribute("A", {}) end) == "Array is not a supported attribute type")
        assert(Message(function() return F.Changed.Nope end) == "Nope is not a valid member of RBXScriptSignal")
        assert(Message(function() return F[1] end) == "@: invalid argument #2 (string expected, got number)")
        assert(Message(function() return game:GetService("Nope") end) == "'Nope' is not a valid Service name")
        F.BorderMode = "Inset"
        assert(F.BorderMode == Enum.BorderMode.Inset)
        F.BorderMode = 0
        assert(F.BorderMode == Enum.BorderMode.Outline)
        "#,
    );
}

#[test]
fn not_scriptable_properties_are_no_members() {
    // Studio's dump permits no Read on PVInstance.Origin, PVInstance."Pivot Offset" or
    // Model.WorldPivotData: a script indexing them finds a child of that name, else no member
    assert_script(
        r#"
        local Part = Instance.new("Part")
        local Origin = Instance.new("Attachment")
        Origin.Name = "Origin"
        Origin.Parent = Part
        assert(Part.Origin == Origin)
        local Model = Instance.new("Model")
        local Ok, Err = pcall(function() return Model.WorldPivotData end)
        assert(not Ok and string.find(Err, "WorldPivotData is not a valid member of Model", 1, true), Err)
        Ok = pcall(function() Model.Origin = CFrame.new() end)
        assert(not Ok)
        -- the pivot is still stored and moved as before
        Model.WorldPivot = CFrame.new(1, 2, 3)
        assert(Model:GetPivot() == CFrame.new(1, 2, 3))
        assert(Part.PivotOffset == CFrame.identity)
        "#,
    );
}

/// A key with its first letter lower-cased reaches the member it spells otherwise: a client resolves a read by the exact member,
/// then a child of that name, then the member named with the first letter upper-cased (every method, property, event and callback
/// of the API dump answers so: 2,175 methods, 802 events, 2,352 readable properties, a headless client of roblox-sim, Studio 0.744);
/// a method call by the exact method, then the upper-cased one, and nothing else; a write by the exact name only.
/// tools/parity/probes/camelcase.luau holds the same cases against the client's output.
#[test]
fn lower_case_first_letters_reach_members_on_reads_and_calls() {
    assert_script(
        r#"
        local function Message(Callback)
            local Ok, Err = pcall(Callback)
            assert(not Ok and type(Err) == "string", "expected an error")
            return (string.gsub(Err, "^Test:%d+: ", "@: "))
        end
        local Holder = Instance.new("Folder")
        Holder.Name = "Holder"
        local Part = Instance.new("Part")
        Part.Name = "P"
        Part.Parent = Holder

        -- reads: the same method function, the same signal, the property's value, inherited members, services
        assert(rawequal(Part.getPropertyChangedSignal, Part.GetPropertyChangedSignal))
        assert(rawequal(Part.getFullName, Part.GetFullName) and rawequal(Part.isAncestorOf, Part.IsAncestorOf))
        assert(Part.anchored == false and Part.name == "P" and Part.parent == Holder and Part.position == Part.Position)
        assert(Part.touched == Part.Touched and tostring(Part.touched) == "Signal Touched")
        assert(tostring(Part.changed) == "Signal Changed" and tostring(Part.destroying) == "Signal Destroying")
        assert(Instance.new("MeshPart").canCollide == true and Instance.new("Frame").visible == true)
        assert(workspace.gravity == workspace.Gravity)
        -- (BasePart.size, the dump's own lower-case name, permits no script: the read finds Size)
        assert(Part.size == Part.Size)

        -- writes take exact names only, and change nothing
        assert(Message(function() Part.anchored = true end) == 'anchored is not a valid member of Part "Holder.P"')
        assert(Message(function() Part.name = "Q" end) == 'name is not a valid member of Part "Holder.P"')
        assert(Message(function() Part.size = Vector3.one end) == 'size is not a valid member of Part "Holder.P"')
        assert(Part.Anchored == false and Part.Name == "P" and Part.Size.Y == 1.2000000476837158)

        -- a child with the key's own name comes first; children answer to their own names only; the exact member beats a child
        local Box = Instance.new("Part")
        Box.Name = "Box"
        for _, Name in { "anchored", "getPropertyChangedSignal", "Anchored", "foo" } do
            local Child = Instance.new("Folder")
            Child.Name = Name
            Child.Parent = Box
        end
        assert(Box.anchored.ClassName == "Folder" and Box.getPropertyChangedSignal.ClassName == "Folder")
        assert(Box.Anchored == false)
        assert(Message(function() return Box.Foo end) == 'Foo is not a valid member of Part "Box"')

        -- method calls: the method by either spelling; children and other members are no methods
        assert(tostring(Part:getPropertyChangedSignal("Name")) == "Signal NameChanged")
        assert(Part:getPropertyChangedSignal("Name") == Part:GetPropertyChangedSignal("Name"))
        assert(Part:getFullName() == "Holder.P" and Holder:findFirstChildOfClass("Part") == Part)
        assert(tostring(Box:getPropertyChangedSignal("Name")) == "Signal NameChanged")
        Part:setAttribute("A", 5)
        assert(Part:getAttribute("A") == 5)
        assert(Message(function() return Part:Anchored() end) == 'Anchored is not a valid member of Part "Holder.P"')
        assert(Message(function() return Part:anchored() end) == 'anchored is not a valid member of Part "Holder.P"')
        assert(Message(function() return Part:touched() end) == 'touched is not a valid member of Part "Holder.P"')
        assert(Message(function() return Box:anchored() end) == 'anchored is not a valid member of Part "Box"')
        assert(Message(function() return Box:foo() end) == 'foo is not a valid member of Part "Box"')
        assert(Message(function() return Part.getPropertyChangedSignal("Name") end) == "Expected ':' not '.' calling member function GetPropertyChangedSignal")

        -- the first letter only, towards upper case only; the error names the key as written
        local Http = game:GetService("HttpService")
        assert(Http:jSONEncode({ 1 }) == "[1]")
        assert(Message(function() return Http:jsonEncode({ 1 }) end) == 'jsonEncode is not a valid member of HttpService "HttpService"')
        assert(Message(function() return Part.ANCHORED end) == 'ANCHORED is not a valid member of Part "Holder.P"')
        assert(Message(function() return Part.cframe end) == 'cframe is not a valid member of Part "Holder.P"')
        assert(Message(function() return Part.fooBar end) == 'fooBar is not a valid member of Part "Holder.P"')
        assert(Message(function() return Part:fooBar() end) == 'fooBar is not a valid member of Part "Holder.P"')

        -- callbacks: a read names the callback by its own name; a write and a call need their exact names
        local Function = Instance.new("BindableFunction")
        Function.Name = "F"
        local Callback = "OnInvoke is a callback member of BindableFunction; you can only set the callback value, get is not available"
        assert(Message(function() return Function.onInvoke end) == Callback)
        assert(Message(function() Function.onInvoke = function() end end) == 'onInvoke is not a valid member of BindableFunction "F"')
        assert(Message(function() return Function:onInvoke() end) == 'onInvoke is not a valid member of BindableFunction "F"')
        Function.OnInvoke = function()
            return 8
        end
        assert(Function:invoke() == 8)
        "#,
    );
}

/// The API dump's own lower-case names (deprecated aliases) are the members they stand for, as a client treats them
/// (tools/parity/probes/aliases.luau): a property alias reads and writes the other's value and Changed names the other; a method
/// alias does the other's work through a function of its own; childAdded and a value object's changed are signals of their own
/// that fire with ChildAdded and Changed.
#[test]
fn the_dumps_lower_case_aliases_are_the_members_they_stand_for() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Players = game:GetService("Players")
        assert(game:getService("Players") == Players and game:service("Players") == Players)
        assert(typeof(Players:getPlayers()) == "table" and typeof(Players:players()) == "table")
        assert(not rawequal(game.getService, game.GetService) and not rawequal(Players.getPlayers, Players.GetPlayers))
        assert(game.workspace == workspace and game.lighting == game:GetService("Lighting"))
        local Part = Instance.new("Part")
        Part.brickColor = BrickColor.new("Really red")
        assert(Part.BrickColor.Name == "Really red")
        Part.BrickColor = BrickColor.new("Lime green")
        assert(Part.brickColor.Name == "Lime green")
        local Humanoid = Instance.new("Humanoid")
        Humanoid.maxHealth = 50
        assert(Humanoid.MaxHealth == 50 and Humanoid.maxHealth == 50)
        local Folder = Instance.new("Folder")
        Folder.archivable = false
        assert(Folder.Archivable == false)
        local Ok, Err = pcall(function() Part.className = "X" end)
        assert(not Ok and string.find(Err, "Unable to assign property className. Property is read only", 1, true), Err)
        assert(tostring(Folder.childAdded) == "Signal childAdded" and Folder.childAdded ~= Folder.ChildAdded)
        _G.Names = {}
        Part.Changed:Connect(function(Name) table.insert(_G.Names, Name) end)
        Part.brickColor = BrickColor.new("Really blue")
        _G.Added = {}
        Folder.childAdded:Connect(function(Child) table.insert(_G.Added, Child.Name) end)
        local Value = Instance.new("IntValue")
        _G.Values = {}
        Value.changed:Connect(function(New) table.insert(_G.Values, New) end)
        local Child = Instance.new("Folder")
        Child.Name = "Kid"
        Child.Parent = Folder
        Value.Value = 7
        "#,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        -- (the property's own names, as for the exact name: a client's Changed names BrickColor and the Color it moves)
        assert(table.concat(_G.Names, ",") == "BrickColor,Color", table.concat(_G.Names, ","))
        assert(table.concat(_G.Added, ",") == "Kid" and table.concat(_G.Values, ",") == "7")
        "#,
    );
}

/// The report's case: a property signal by its lower-case method name, both handlers run, the newest connection first (as the
/// report's Studio output and a client print them).
#[test]
fn a_property_signal_by_a_lower_case_method_name_fires() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local part = Instance.new("Part")
        part:GetPropertyChangedSignal("Position"):Connect(function()
            print("one")
        end)
        part:getPropertyChangedSignal("Position"):Connect(function()
            print("two")
        end)
        part.Position += Vector3.new(0, 1, 0)
        "#,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    assert_eq!(lines(&engine), vec!["print: two".to_string(), "print: one".to_string()]);
}

/// What reaching a member by its lower-case first letter costs on the work clock: the member's own price and cost.rs LOWER_FIRST
/// (a client, +33 to +47 ns over the exact name for reads and calls alike).
#[test]
fn a_lower_case_first_letter_costs_a_second_lookup() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Part = Instance.new("Part")
        local function Per(Body)
            local start = os.clock()
            for _ = 1, 1000 do
                Body()
            end
            return (os.clock() - start) / 1000 * 1e9
        end
        -- (each spelling once first: a member's first use in a frame costs more)
        local _, _, _ = Part.Anchored, Part.anchored, Part:isAncestorOf(Part)
        local Exact = Per(function() return Part.Anchored end)
        local Lower = Per(function() return Part.anchored end)
        assert(math.abs(Lower - Exact - 40) < 0.5, string.format("a read: %.2f ns more", Lower - Exact))
        local ExactCall = Per(function() return Part:IsAncestorOf(Part) end)
        local LowerCall = Per(function() return Part:isAncestorOf(Part) end)
        assert(math.abs(LowerCall - ExactCall - 40) < 0.5, string.format("a call: %.2f ns more", LowerCall - ExactCall))
        "#,
    );
}

/// (A parent change fires DescendantAdded, then ChildAdded, then AncestryChanged, then the instance's own Changed("Parent"):
/// measured in Studio, see a_model_arriving_in_a_folder_fires_its_events_in_studios_order; the order here was ChildAdded first and
/// Changed before both.)
#[test]
fn deferred_events_and_order() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local F = Instance.new("Frame")
        F:GetPropertyChangedSignal("Size"):Connect(function() print("size") end)
        F.Changed:Connect(function(Name) print("changed " .. Name) end)
        F.Size = UDim2.fromOffset(10, 10)
        print("before flush")
        local Folder = Instance.new("Folder")
        Folder.ChildAdded:Connect(function(Child) print("added " .. Child.Name) end)
        Folder.DescendantAdded:Connect(function(Child) print("descendant " .. Child.Name) end)
        F.Parent = Folder
        local Value = Instance.new("IntValue")
        Value.Changed:Connect(function(New) print("value " .. New) end)
        Value:GetPropertyChangedSignal("Value"):Connect(function() print("value signal") end)
        Value.Value = 5
        Value.Name = "Renamed"
        local E = Instance.new("BindableEvent")
        E.Event:Connect(function(X) print("A" .. X) end)
        E.Event:Connect(function(X) print("B" .. X) end)
        E.Event:Once(function(X) print("once" .. X) end)
        E:Fire(1)
        E:Fire(2)
        "#,
    );
    let output = lines(&engine);
    assert_eq!(
        output,
        vec![
            "print: before flush",
            "print: changed Size",
            "print: size",
            "print: descendant Frame",
            "print: added Frame",
            "print: changed Parent",
            "print: value signal",
            "print: value 5",
            "print: once1",
            "print: B1",
            "print: A1",
            "print: B2",
            "print: A2",
        ]
    );
}

#[test]
fn scheduler_chains_depth_and_timers() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Defers = 0
        local function DeferNext()
            Defers += 1
            if Defers < 100 then task.defer(DeferNext) end
        end
        task.defer(DeferNext)
        local Loop = Instance.new("BindableEvent")
        local Fires = 0
        Loop.Event:Connect(function()
            Fires += 1
            Loop:Fire()
        end)
        Loop:Fire()
        task.defer(function() print("defers " .. Defers) end)
        task.delay(0.05, function() print("delay " .. Fires) end)
        task.spawn(function()
            local Waited = task.wait(0.1)
            print("waited", Waited > 0.09)
        end)
        "#,
    );
    scheduler::run_until(&lua, &engine, 0.3).unwrap();
    let output = lines(&engine);
    // One depth for events and task.defer, cut at 80 (measured in a playing client, Studio 0.744): the self-firing
    // event's 81st handler is refused for its handler, the task.defer chain's 81st call logs and queues nothing
    assert!(
        output.contains(&"error: Maximum event re-entrancy depth exceeded for BindableEvent.Event when calling anonymous function on line 10 in Test".to_string()),
        "{output:?}"
    );
    assert!(output.contains(&"error: Maximum re-entrancy depth (80) exceeded calling task.defer".to_string()), "{output:?}");
    assert!(output.contains(&"print: delay 80".to_string()), "{output:?}");
    assert!(output.contains(&"print: waited true".to_string()), "{output:?}");
    assert!(output.iter().any(|l| l.starts_with("print: defers")), "{output:?}");
}

#[test]
fn a_chain_of_events_and_defers_shares_one_depth() {
    // Measured in a playing client (Studio 0.744): an event fired from a deferred thread and a task.defer from a
    // handler count on one depth, task.spawn carries it along, and the chain stops at 80 steps
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Steps = 0
        local Event = Instance.new("BindableEvent")
        local function Next()
            Steps += 1
            if Steps < 300 then task.spawn(function() Event:Fire() end) end
        end
        Event.Event:Connect(function()
            Steps += 1
            if Steps < 300 then task.defer(Next) end
        end)
        Event:Fire()
        task.delay(0.05, function() print("steps " .. Steps) end)
        local Value = Instance.new("IntValue")
        Value:GetPropertyChangedSignal("Value"):Connect(function() Value.Value += 1 end)
        Value.Value = 1
        task.delay(0.05, function() print("value " .. Value.Value) end)
        print(tostring(Value:GetPropertyChangedSignal("Value")))
        "#,
    );
    scheduler::run_until(&lua, &engine, 0.2).unwrap();
    let output = lines(&engine);
    assert!(output.contains(&"print: steps 80".to_string()), "{output:?}");
    assert!(output.contains(&"print: value 81".to_string()), "{output:?}");
    assert!(output.contains(&"print: Signal ValueChanged".to_string()), "{output:?}");
    assert!(output.iter().any(|line| line.starts_with("error: Maximum event re-entrancy depth exceeded for Object.ValueChanged when calling anonymous function on line 15 in Test")), "{output:?}");
}

#[test]
fn a_parent_write_of_the_parent_it_has_is_no_change() {
    // Measured in a playing client (Studio 0.748): setting Parent to the parent an instance already has passes the Parent lock
    // without a word or an event (a destroyed instance and its destroyed child, the DataModel, the UserSettings root); any
    // other parent still meets the lock, and a service or Terrain still refuses even its own parent
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local function Try(F)
            local Ok, Err = pcall(F)
            return if Ok then "ok" else (string.gsub(tostring(Err), "^[^:]*:%d+: ", ""))
        end
        local A = Instance.new("Part")
        local A1 = Instance.new("Folder")
        A1.Parent = A
        A.Parent = workspace
        A:Destroy()
        A.Changed:Connect(function(Name) print("changed " .. Name) end)
        A1.Changed:Connect(function(Name) print("child changed " .. Name) end)
        print(Try(function() A.Parent = nil end), Try(function() A1.Parent = nil end))
        print(Try(function() A.Parent = workspace end))
        print(Try(function() A.Parent = nil end))
        print(Try(function() game.Parent = nil end), Try(function() UserSettings().Parent = nil end))
        print(Try(function() game:GetService("Lighting").Parent = game end), (pcall(function() workspace.Terrain.Parent = workspace end)))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: ok ok",
            "print: The Parent property of Part is locked, current parent: NULL, new parent Workspace",
            "print: ok",
            "print: ok ok",
            "print: Cannot change Parent of type Lighting false",
        ]
    );
}

#[test]
fn a_parent_write_meets_the_class_lock_then_no_change_then_the_instance_lock() {
    // Measured in a playing client and a server (Studio 0.747, tools/parity/probes/parent_locks.luau; a 0.748 client reads the
    // same): what is locked by class (most services, Terrain, a player's PlayerGui and PlayerScripts) refuses every write, the
    // parent it has and itself too; then the parent it has is no change; then what is locked as an instance (29 services such
    // as Workspace and Run Service, the PlayerMouse) refuses naming its parent and the new one by Name. A settings root made its
    // own parent says so before its capability error; the Player, its Backpack and the Camera have no lock
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    run(
        &lua,
        &engine,
        r#"
        local function Try(F)
            local Ok, Err = pcall(F)
            if Ok then
                return "ok"
            end
            local Text = string.gsub(tostring(Err), "^[^:]*:%d+: ", "")
            return (string.gsub(Text, "current parent: " .. string.gsub(game.Name, "%p", "%%%0") .. ",", "current parent: <game>,"))
        end
        local Deep = Instance.new("Folder")
        Deep.Name = "Deep"
        Deep.Parent = Instance.new("Folder", game:GetService("ReplicatedStorage"))
        print(Try(function() workspace.Parent = game end), Try(function() workspace.Parent = nil end))
        print(Try(function() workspace.Parent = Deep end))
        local Run = game:GetService("RunService")
        print(Try(function() Run.Parent = Run end))
        print(Try(function() game:GetService("ReplicatedStorage").Parent = game end), Try(function() game:GetService("Chat").Parent = nil end))
        print(Try(function() workspace.Terrain.Parent = workspace end), Try(function() workspace.Terrain.Parent = workspace.Terrain end))
        local Me = game:GetService("Players").LocalPlayer
        local Gui, Scripts = Me:FindFirstChildOfClass("PlayerGui"), Me:FindFirstChildOfClass("PlayerScripts")
        print(Try(function() Gui.Parent = Me end), Try(function() Scripts.Parent = nil end))
        print(Try(function() Me.Parent = Me.Parent end), Try(function() Me.Backpack.Parent = Me end), Try(function() workspace.CurrentCamera.Parent = workspace end))
        print(Try(function() workspace.CurrentCamera.Parent = workspace.CurrentCamera end))
        local User = UserSettings()
        print(Try(function() User.Parent = User end), Try(function() User.Parent = workspace end))
        print(Try(function() Me:GetMouse().Parent = workspace end))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: ok The Parent property of Workspace is locked, current parent: <game>, new parent NULL",
            "print: The Parent property of Workspace is locked, current parent: <game>, new parent Deep",
            "print: The Parent property of Run Service is locked, current parent: <game>, new parent Run Service",
            "print: Cannot change Parent of type ReplicatedStorage Cannot change Parent of type Chat",
            "print: Cannot change Parent of type Terrain Cannot change Parent of type Terrain",
            "print: Cannot change Parent of type PlayerGui Cannot change Parent of type PlayerScripts",
            "print: ok ok ok",
            "print: Attempt to set Workspace.Camera as its own parent",
            "print: Attempt to set GlobalBasicSettings as its own parent The current thread cannot set 'UserSettings parent' (lacking capability RobloxScript)",
            "print: The Parent property of Instance is locked, current parent: NULL, new parent Workspace",
        ]
    );
}

#[test]
fn destroy_of_what_is_locked_by_class() {
    // Measured in a playing client and a server (Studio 0.747, tools/parity/probes/parent_locks.luau): Terrain refuses Destroy
    // and stays; a player's PlayerGui and PlayerScripts are destroyed and keep their class's lock
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    run(
        &lua,
        &engine,
        r#"
        local function Try(F)
            local Ok, Err = pcall(F)
            return if Ok then "ok" else (string.gsub(tostring(Err), "^[^:]*:%d+: ", ""))
        end
        print(Try(function() workspace.Terrain:Destroy() end), workspace.Terrain.Parent == workspace)
        local Me = game:GetService("Players").LocalPlayer
        for _, Class in { "PlayerGui", "PlayerScripts" } do
            local Object = Me:FindFirstChildOfClass(Class)
            print(Try(function() Object:Destroy() end), Object.Parent, Try(function() Object.Parent = nil end), Try(function() Object.Parent = Me end))
        end
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: Cannot Destroy() Terrain true",
            "print: ok nil Cannot change Parent of type PlayerGui Cannot change Parent of type PlayerGui",
            "print: ok nil Cannot change Parent of type PlayerScripts Cannot change Parent of type PlayerScripts",
        ]
    );
}

#[test]
fn destroy_and_clone() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        local Child = Instance.new("Frame", Folder)
        Child:SetAttribute("Key", 3)
        Child:AddTag("Tagged")
        local Copy = Folder:Clone()
        assert(Copy ~= Folder and Copy:FindFirstChild("Frame"):GetAttribute("Key") == 3)
        assert(Copy.Frame:HasTag("Tagged"))
        Child.Destroying:Connect(function() print("destroying", Child.Parent == Folder) end)
        Child.Changed:Connect(function(Name) print("changed after destroy " .. Name) end)
        Folder:Destroy()
        print("parent", Child.Parent)
        local Ok, Err = pcall(function() Child.Parent = Copy end)
        print(Ok, Err)
        Child.Archivable = false
        print("clone", Child:Clone())
        "#,
    );
    let output = lines(&engine);
    // Destroying is deferred like every event (measured in a playing client, Studio 0.744): its handler runs after
    // Destroy returned, sees the instance unparented, and comes before the instance's own removal events.
    assert_eq!(
        output,
        vec![
            "print: parent nil",
            "print: false The Parent property of Frame is locked, current parent: NULL, new parent Folder",
            "print: clone nil",
            "print: destroying false",
            "print: changed after destroy Parent",
        ]
    );
}

#[test]
fn clear_all_children_destroys_the_last_child_first() {
    // Measured in a playing client (Studio 0.744): ChildRemoved fires for C3, C2, C1
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        for Index = 1, 3 do
            local Child = Instance.new("Folder")
            Child.Name = "C" .. Index
            Child.Parent = Folder
        end
        Folder.ChildRemoved:Connect(function(Child) print(Child.Name) end)
        Folder:ClearAllChildren()
        "#,
    );
    assert_eq!(lines(&engine), vec!["print: C3", "print: C2", "print: C1"]);
}

#[test]
fn an_engine_function_that_returns_nothing_returns_no_values() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/signals.luau and task_order.luau):
    // `pcall(Event.Fire, Event, {})` and `pcall(task.cancel, thread)` give only `true`, whether the method
    // is called with `:` or read first
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local Event = Instance.new("BindableEvent")
        local Folder = Instance.new("Folder")
        print(select("#", Event:Fire({})), select("#", Event.Fire(Event, {})), select("#", Folder.ClearAllChildren(Folder)))
        print(select("#", pcall(task.cancel, task.defer(function() end))), select("#", Folder.FindFirstChild(Folder, "None")))
        "##,
    );
    assert_eq!(lines(&engine), vec!["print: 0 0 0", "print: 1 1"]);
}

#[test]
fn set_attribute_checks_names_and_types_as_roblox_does() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/instances.luau)
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        for _, Name in { "", "with space", "dash-name", "dot.name", "a/b", "RBXName", "rbxname", "123", string.rep("x", 100), string.rep("x", 101), "$x" } do
            local Ok, Message = pcall(Folder.SetAttribute, Folder, Name, 1)
            print(#Name, Ok, Message)
        end
        print(pcall(function() Folder:SetAttribute("with space", "fast path") end))
        print(pcall(Folder.SetAttribute, Folder, "A", {}))
        print(pcall(Folder.SetAttribute, Folder, "A", print))
        print(pcall(Folder.SetAttribute, Folder, "A", buffer.create(1)))
        "#,
    );
    let long = "x".repeat(101);
    assert_eq!(
        lines(&engine),
        vec![
            "print: 0 false Attribute name cannot be empty (\"\")".to_string(),
            "print: 10 false Attribute name contains illegal character ' ' (\"with space\")".to_string(),
            "print: 9 true nil".to_string(),
            "print: 8 true nil".to_string(),
            "print: 3 true nil".to_string(),
            "print: 7 false Attempt to assign attribute RBXName. CoreScript permission required".to_string(),
            "print: 7 true nil".to_string(),
            "print: 3 true nil".to_string(),
            "print: 100 true nil".to_string(),
            format!("print: 101 false Attribute name exceeds 100 character limit (\"{long}\")"),
            "print: 2 false Attribute name contains illegal character '$' (\"$x\")".to_string(),
            "print: false Attribute name contains illegal character ' ' (\"with space\")".to_string(),
            "print: false Array is not a supported attribute type".to_string(),
            "print: false Function is not a supported attribute type".to_string(),
            "print: false buffer is not a supported attribute type".to_string(),
        ]
    );
}

/// An attribute can hold an Instance (Studio 0.744 takes one; measured in a playing client,
/// tools/parity/probes/instance_attributes.luau): a read gives a new InstanceHandle that prints the instance's current
/// name and has no members, the attribute keeps a destroyed or loose instance, and a handle can be stored again.
#[test]
fn an_attribute_holds_an_instance_as_a_handle() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Holder = Instance.new("Folder")
        Holder.Parent = workspace
        local Target = Instance.new("Part")
        Target.Name = "Target"
        Target.Parent = workspace
        Holder:SetAttribute("Ref", Target)
        local Read = Holder:GetAttribute("Ref")
        print(typeof(Read), type(Read), tostring(Read), Read == Target, Holder:GetAttribute("Ref") == Read, rawequal(Holder:GetAttribute("Ref"), Read))
        print(pcall(function() return Read.Name end))
        print(pcall(function() return Read:IsA("Part") end))
        print(typeof(Holder:GetAttributes().Ref), ({ [Read] = true })[Holder:GetAttribute("Ref")])
        Target.Name = "Renamed"
        Target:Destroy()
        print(tostring(Holder:GetAttribute("Ref")))
        Holder:SetAttribute("Loose", Instance.new("Folder"))
        Holder:SetAttribute("Again", Holder:GetAttribute("Ref"))
        print(tostring(Holder:GetAttribute("Loose")), typeof(Holder:GetAttribute("Again")), tostring(Holder:GetAttribute("Again")))
        "#,
    );
    // the loose folder and the destroyed part are kept by the attributes alone (the Lua values made above are gone)
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
    engine_lua::raw::collect(&lua, &engine).unwrap();
    run(&lua, &engine, r#"print(tostring(workspace.Folder:GetAttribute("Loose")), tostring(workspace.Folder:GetAttribute("Ref")))"#);
    assert_eq!(
        lines(&engine),
        vec![
            "print: InstanceHandle userdata Target false true false",
            "print: false Name is not a valid member of InstanceHandle",
            "print: false IsA is not a valid member of InstanceHandle",
            "print: InstanceHandle nil",
            "print: Renamed",
            "print: Folder InstanceHandle Renamed",
            "print: Folder Renamed",
        ]
    );
}

/// The InstanceHandle datatype as a playing client has it (Studio 0.748, a client and a server alike:
/// tools/parity/probes/instance_handle.luau; lua/types.rs LInstanceHandle): a frozen `InstanceHandle` library holding `new`,
/// which takes an Instance or nil and refuses anything else without a position; `Get`, a new function at each read; a handle of
/// none that prints nil; the member errors; `==` by instance; and the copies an attribute, a Bindable and a SharedTable give.
#[test]
fn an_instance_handle_holds_an_instance_or_none_as_a_client_does() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Target = Instance.new("Part")
        Target.Name = "Target"
        Target.Parent = workspace
        local Keys = {}
        for Key, Value in InstanceHandle do
            table.insert(Keys, Key .. "=" .. typeof(Value))
        end
        print(type(InstanceHandle), table.isfrozen(InstanceHandle), table.concat(Keys, ","), (select(2, pcall(function() InstanceHandle.Foo = 1 end))))
        local Handle, Empty = InstanceHandle.new(Target), InstanceHandle.new(nil)
        print(typeof(Handle), tostring(Handle), typeof(Empty), tostring(Empty), tostring(InstanceHandle.new()), rawequal(Handle:Get(), Target), Empty:Get())
        print(select(2, pcall(InstanceHandle.new, 5)), (select(2, pcall(function() return InstanceHandle.new(Handle) end))), tostring(InstanceHandle.new(Target, 5)))
        print(Handle.Get == Handle.Get, Handle.Get(Empty), (select(2, pcall(Handle.Get))), (select(2, pcall(Handle.Get, Vector3.one))))
        print((select(2, pcall(function() return Handle.Name end))), (select(2, pcall(function() return Handle:Set() end))), (select(2, pcall(function() Handle.Get = 1 end))), (select(2, pcall(function() Handle[1] = 1 end))))
        print(Handle == InstanceHandle.new(Target), rawequal(Handle, InstanceHandle.new(Target)), Empty == InstanceHandle.new(), Handle == Empty, Handle == Target, ({ [Handle] = 1 })[InstanceHandle.new(Target)])
        Target.Name = "Renamed"
        Target:Destroy()
        print(tostring(Handle), rawequal(Handle:Get(), Target), tostring(InstanceHandle.new(Target)))
        local Holder = Instance.new("Folder")
        local Changes = 0
        Holder.AttributeChanged:Connect(function() Changes += 1 end)
        Holder:SetAttribute("Empty", Empty)
        Holder:SetAttribute("Empty", InstanceHandle.new())
        Holder:SetAttribute("Ref", Handle)
        task.wait()
        local Read = Holder:GetAttribute("Empty")
        print(typeof(Read), Read:Get(), typeof(Holder:GetAttributes().Empty), typeof(Holder:Clone():GetAttribute("Empty")), rawequal(Holder:GetAttribute("Ref"):Get(), Target), Changes)
        local Bindable = Instance.new("BindableEvent")
        Bindable.Event:Connect(function(Alone, List, Keyed)
            local Key = next(Keyed)
            print(rawequal(Alone, Handle), Alone == Handle, rawequal(List[1], Handle), List[1] == Handle, Key)
        end)
        Bindable:Fire(Handle, { Handle }, { [Empty] = 1 })
        task.wait()
        local Shared = SharedTable.new({ Handle })
        print(rawequal(Shared[1], Handle), Shared[1] == Handle, rawequal(Shared[1], Shared[1]))
        local Http = game:GetService("HttpService")
        print(Http:JSONEncode({ Handle }), Http:JSONEncode({ [Handle] = 1 }))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: table true new=function Test:9: attempt to modify a readonly table",
            "print: InstanceHandle Target InstanceHandle nil nil true nil",
            "print: InstanceHandle.new expects an Instance argument InstanceHandle.new expects an Instance argument Target",
            "print: false nil missing argument #1 to 'Get' (InstanceHandle expected) invalid argument #1 to 'Get' (InstanceHandle expected, got Vector3)",
            "print: Name is not a valid member of InstanceHandle Set is not a valid member of InstanceHandle Get is not a valid member of InstanceHandle 1 is not a valid member of InstanceHandle",
            "print: true false true false false nil",
            "print: Renamed true Renamed",
            "print: InstanceHandle nil InstanceHandle InstanceHandle true 2",
            "print: false true false true <InstanceRef> (nil)",
            "print: false true false",
            r#"print: [null] {"<InstanceRef> (Renamed)":1}"#,
        ]
    );
}

/// An Object parameter given a value that is not an Instance, as a playing client casts it (Studio 0.748, a client and a
/// server alike: tools/parity/probes/object_parameter_casts.luau): Instance.new's parent raises Luau's argument error naming the
/// value's typeof, at the calling line, after the class name's check; IsDescendantOf and IsAncestorOf take nil, a thread, an
/// Enum and a newproxy as nil, raise `Unable to cast value to Object` for any other value and `Argument 1 missing or nil` for
/// no argument, without a position. ree took any of those values as no parent, and answered false (true for IsDescendantOf
/// with no argument).
#[test]
fn object_parameters_cast_a_value_as_a_client_does() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        print((select(2, pcall(function() return Instance.new("Folder", 5) end))))
        print((select(2, pcall(Instance.new, "Folder", InstanceHandle.new(workspace)))))
        print((select(2, pcall(Instance.new, "Folder", workspace.Changed))), (select(2, pcall(Instance.new, "Folder", coroutine.create(print)))))
        print((select(2, pcall(Instance.new, "NoSuchClass", 5))), Instance.new("Folder", nil).Parent)
        print(pcall(function() return workspace:IsAncestorOf(5) end))
        print(pcall(function() return Folder:IsDescendantOf(InstanceHandle.new(workspace)) end))
        print(pcall(workspace.IsAncestorOf, workspace, Vector3.one))
        print(Folder:IsDescendantOf(nil), Folder:IsDescendantOf(Enum.Material), Folder:IsDescendantOf(coroutine.create(print)), Folder:IsDescendantOf(newproxy()), workspace:IsAncestorOf(Enum), workspace:IsAncestorOf(nil))
        print(pcall(function() return Folder:IsDescendantOf() end))
        print(pcall(function() return workspace:IsAncestorOf() end))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: Test:3: invalid argument #2 to 'new' (Instance expected, got number)",
            "print: invalid argument #2 to 'new' (Instance expected, got InstanceHandle)",
            "print: invalid argument #2 to 'new' (Instance expected, got RBXScriptSignal) invalid argument #2 to 'new' (Instance expected, got thread)",
            "print: Unable to create an Instance of type \"NoSuchClass\" nil",
            "print: false Unable to cast value to Object",
            "print: false Unable to cast value to Object",
            "print: false Unable to cast value to Object",
            "print: true true true true false false",
            "print: false Argument 1 missing or nil",
            "print: false Argument 1 missing or nil",
        ]
    );
}

/// Texts and forms measured in a playing client (Studio 0.744; tools/parity/probes/hierarchy.luau, values.luau,
/// coercion.luau, frame_phases.luau, lifetime.luau; `typeof()` from a function is at its line: Studio 0.747,
/// engine_functions.luau).
#[test]
fn parents_enums_and_casts_answer_as_a_playing_client_does() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local function Try(F) local Ok, Message = pcall(F) return if Ok then "ok" else Message end
        local Self = Instance.new("Folder")
        Self.Name = "Self"
        print(Try(function() Self.Parent = Self end))
        print(Try(function() game:GetService("Lighting").Parent = workspace end), Try(function() game:GetService("Lighting"):Destroy() end))
        print(game:GetService("Lighting").Parent == game, (string.gsub(Try(function() game.Parent = workspace end), game.Name, "<game>")))
        local Part = Instance.new("Part")
        for _, Value in { "512", 512.5, "wood" } do
            print(Try(function() Part.Material = Value end), tostring(Part.Material))
        end
        print(Try(function() Part.CFrame = Vector3.zero end))
        print(Try(function() typeof() end), type(getmetatable("")))
        print(Try(function() game:GetService("RunService"):BindToRenderStep("x", "high", function() end) end))
        local Debris = game:GetService("Debris")
        print(Try(function() Debris:AddItem({}, 1) end), Try(function() Debris:AddItem(nil, 1) end), Try(function() Debris:AddItem(Part, "x") end))
        local Gone = Instance.new("Part")
        Gone.Parent = workspace
        Debris:AddItem(Gone, 0)
        print(Gone.Parent)
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: Attempt to set Self as its own parent",
            "print: Cannot change Parent of type Lighting The Parent property of Lighting is locked",
            "print: true The Parent property of <game> is locked, current parent: NULL, new parent Workspace",
            "print: ok Enum.Material.Wood",
            "print: ok Enum.Material.Wood",
            "print: Unable to assign property Material. Invalid value \"wood\" for enum Material Enum.Material.Wood",
            "print: Unable to assign property CFrame. CoordinateFrame expected, got Vector3",
            "print: Test:13: missing argument #1 string",
            "print: Unable to cast string to int",
            "print: Unable to cast value to Object ok Unable to cast string to double",
            "print: nil",
        ]
    );
}

/// A new instance reads its class's default name, which Studio does not always make the class's own (a BindableEvent
/// is "Event", measured in a playing client; a BoolValue "Value" and a WedgePart "Wedge" from Studio's defaults), and
/// the DataModel's full name is its Name (measured: tools/parity/probes/instances.luau).
#[test]
fn new_instances_read_their_default_names_and_the_data_model_its_full_name() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Value = Instance.new("BoolValue")
        print(Instance.new("BindableEvent").Name, Value.Name, Instance.new("WedgePart").Name, Instance.new("Folder").Name, Value:GetFullName())
        Value.Name = "Named"
        print(Value.Name, game:GetFullName() == game.Name, workspace:GetFullName())
        "#,
    );
    assert_eq!(lines(&engine), vec!["print: Event Value Wedge Folder Value", "print: Named true Workspace"]);
}

#[test]
fn instance_methods_answer_and_raise_as_roblox_does() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/instances.luau), with `:` and read with `.`
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        print(pcall(Folder.FindFirstChild, Folder, nil))
        print(pcall(function() return Folder:FindFirstChild(nil) end))
        print(Folder:IsDescendantOf(nil), Folder.IsDescendantOf(Folder, nil), Folder:IsDescendantOf(workspace))
        print(pcall(function() return Folder:FindFirstDescendant("x") end))
        print(pcall(function() return game:GetService("Lighting"):Clone() end))
        Folder:SetAttribute(5, 1)
        print(Folder:GetAttribute(5), Folder.GetAttribute(Folder, 5), Folder:GetAttribute("5"))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: false Argument 1 missing or nil",
            "print: false Argument 1 missing or nil",
            "print: true true false",
            "print: false FindFirstDescendant is not enabled",
            "print: false Lighting cannot be cloned",
            "print: 1 1 1",
        ]
    );
}

#[test]
fn a_part_is_at_least_a_thousandth_of_a_stud_on_every_axis() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/changed.luau): Size = (0, 0, 0) reads 0.001 each
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Part = Instance.new("Part")
        Part.Size = Vector3.new(0, 0, 0)
        print(string.format("%.9g %.9g %.9g", Part.Size.X, Part.Size.Y, Part.Size.Z))
        local Wedge = Instance.new("WedgePart")
        Wedge.Size = Vector3.new(4, 0, 2)
        print(Wedge.Size.X, string.format("%.9g", Wedge.Size.Y), Wedge.Size.Z)
        "#,
    );
    assert_eq!(lines(&engine), vec!["print: 0.00100000005 0.00100000005 0.00100000005", "print: 4 0.00100000005 2"]);
}

#[test]
fn writing_nan_over_nan_fires_changed_again() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/changed.luau): a property, a NumberValue and an
    // attribute alike
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Part = Instance.new("Part")
        local Value = Instance.new("NumberValue")
        local Fired = { Part = 0, Value = 0, Attribute = 0 }
        Part:GetPropertyChangedSignal("Transparency"):Connect(function() Fired.Part += 1 end)
        Value.Changed:Connect(function() Fired.Value += 1 end)
        Part:GetAttributeChangedSignal("A"):Connect(function() Fired.Attribute += 1 end)
        for _ = 1, 2 do
            Part.Transparency = 0 / 0
            Value.Value = 0 / 0
            Part:SetAttribute("A", 0 / 0)
        end
        Part.Transparency = 0.5
        Part.Transparency = 0.5
        task.defer(function() print(Fired.Part, Fired.Value, Fired.Attribute) end)
        "#,
    );
    assert_eq!(lines(&engine), vec!["print: 3 2 2"]);
}

#[test]
fn random_draws_unit_vectors_shuffles_and_bounds_as_measured() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/random.luau, seed 0 and Random.new(9))
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Generator = Random.new(0)
        for _ = 1, 3 do Generator:NextNumber() end
        for _ = 1, 3 do Generator:NextNumber(-10, 10) end
        for _ = 1, 3 do Generator:NextInteger(1, 1000) end
        print(Generator:NextInteger(-2 ^ 40, 2 ^ 40))
        local Unit = Generator:NextUnitVector()
        print(string.format("%.9g %.9g %.9g", Unit.X, Unit.Y, Unit.Z))
        local List = { 1, 2, 3, 4, 5, 6, 7, 8 }
        Generator:Shuffle(List)
        print(table.concat(List, ","))
        local Other = Random.new(9)
        print(Other:NextInteger(5, 1), string.format("%.17g", Other:NextNumber(5, 1)), Other:NextInteger(1.5, 2.5))
        print(pcall(Other.Shuffle, Other, { 1, nil, 3 }))
        print(string.format("%.17g", Other:NextInteger(-2 ^ 63, 2 ^ 63)))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: 290622939254",
            "print: -0.245626748 -0.670917571 0.699669242",
            "print: 8,6,2,4,7,5,1,3",
            "print: 4 2.5745788259440792 2",
            "print: false Attempted to shuffle table containing nil values. This is not allowed because the table length may change.",
            "print: -9.2233720368547758e+18",
        ]
    );
}

/// utf8.nfcnormalize and utf8.nfdnormalize as a playing client and a server answer them (Studio 0.747,
/// tools/parity/probes/utf8_normalize.luau; lua/normalize.rs), where both returned their argument: nfc composes with the
/// code point before it only (no reordering, singletons and exclusions stay, any syllable takes a trailing consonant),
/// nfd decomposes in place with a U+0000 after a singleton; numbers are read as text, and the argument and invalid UTF-8
/// errors are raised at the calling line.
#[test]
fn utf8_normalization_follows_a_clients_rules() {
    let script = r##"
        local U = utf8.char
        local function Codes(Text)
            local Out = {}
            for _, Code in utf8.codes(Text) do
                table.insert(Out, string.format("%X", Code))
            end
            return table.concat(Out, " ")
        end
        print("nfc", Codes(utf8.nfcnormalize(U(0x65, 0x301))), Codes(utf8.nfcnormalize(U(0x61, 0x302, 0x323))), Codes(utf8.nfcnormalize(U(0x61, 0x316, 0x301))))
        print("nfc kept", Codes(utf8.nfcnormalize(U(0x212B, 0x958, 0xF900))), Codes(utf8.nfcnormalize(U(0x11935, 0x11930))))
        print("nfc hangul", Codes(utf8.nfcnormalize(U(0x1100, 0x1161, 0x11A8))), Codes(utf8.nfcnormalize(U(0xAC01, 0x11A8))), Codes(utf8.nfcnormalize(U(0xAC00, 0x11A7))))
        print("nfd", Codes(utf8.nfdnormalize(U(0x1EC7))), Codes(utf8.nfdnormalize(U(0x61, 0x301, 0x323))), Codes(utf8.nfdnormalize(U(0xD7A3))))
        print("nfd singletons", Codes(utf8.nfdnormalize(U(0x212B))), Codes(utf8.nfdnormalize(U(0x2126, 0x62))), Codes(utf8.nfdnormalize(U(0x1F71))))
        print("values", select("#", utf8.nfcnormalize("abc")), utf8.nfcnormalize(123), utf8.nfdnormalize(-0.5), utf8.nfcnormalize(""))
        print(pcall(utf8.nfcnormalize))
        print(pcall(utf8.nfdnormalize, nil))
        print(pcall(utf8.nfcnormalize, workspace))
        print(pcall(function() return utf8.nfcnormalize("\255") end))
        print(pcall(function() return utf8.nfdnormalize("e\204") end))
        print(pcall(function() return utf8.nfdnormalize(true) end))
    "##;
    let (lua, engine) = start();
    assert_eq!(
        run(&lua, &engine, script),
        [
            "print: nfc E9 E2 323 61 316 301",
            "print: nfc kept 212B 958 F900 11935 11930",
            "print: nfc hangul AC01 AC02 AC00",
            "print: nfd 65 323 302 61 301 323 1112 1175 11C2",
            "print: nfd singletons 41 30A 0 3A9 0 62 3B1 301 0",
            "print: values 1 123 -0.5 ",
            "print: false missing argument #1 to 'nfcnormalize' (string expected)",
            "print: false invalid argument #1 to 'nfdnormalize' (string expected, got nil)",
            "print: false invalid argument #1 to 'nfcnormalize' (string expected, got Instance)",
            "print: false Test:19: String contains invalid UTF-8",
            "print: false Test:20: End of string is invalid UTF-8",
            "print: false Test:21: invalid argument #1 to 'nfdnormalize' (string expected, got boolean)",
        ]
    );
}

#[test]
fn the_task_library_raises_robloxs_errors() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/task_order.luau): no position on any of them,
    // and a thread queued twice is refused the second time and resumes once
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local function Try(...)
            local Ok, Message = pcall(...)
            print(Ok, Message)
        end
        Try(task.spawn, 5)
        Try(task.defer, "x")
        Try(task.delay, 0, nil)
        Try(task.delay, "x", function() end)
        Try(task.wait, "x")
        Try(task.cancel, 5)
        Try(task.cancel, coroutine.running())
        local Normal
        Normal = coroutine.create(function()
            coroutine.resume(coroutine.create(function() Try(task.cancel, Normal) end))
        end)
        coroutine.resume(Normal)
        local Twice = coroutine.create(function(Value)
            print("twice", Value)
            coroutine.yield()
            print("twice-second")
        end)
        task.defer(Twice, "a")
        Try(task.defer, Twice, "b")
        Try(task.spawn, Twice, "c")
        local Delayed = coroutine.create(function() end)
        task.delay(0, Delayed)
        Try(task.defer, Delayed)
        Try(task.delay, 0, Delayed)
        print((pcall(task.delay, "0.5", function() end)))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: false invalid argument #1 to 'spawn' (function or thread expected)",
            "print: false invalid argument #1 to 'defer' (function or thread expected)",
            "print: false invalid argument #2 to 'delay' (function or thread expected)",
            "print: false invalid argument #1 to 'delay' (number expected, got string)",
            "print: false invalid argument #1 to 'wait' (number expected, got string)",
            "print: false invalid argument #1 to 'cancel' (thread expected, got number)",
            "print: false cannot cancel thread",
            "print: false cannot cancel thread",
            "print: false Cannot call task.defer on a thread that is already 'deferred' in the task library",
            "print: false Cannot call task.spawn on a thread that is already 'deferred' in the task library",
            "print: false Cannot call task.defer on a thread that is already 'delayed' in the task library",
            "print: false Cannot call task.delay on a thread that is already 'delayed' in the task library",
            "print: true",
            "print: twice a",
        ]
    );
}

/// The refusal of a call that would yield where the thread cannot (a table.sort comparator here; a string.gsub callback and
/// an __index metamethod alike), as tools/parity/probes/yield_refused.luau measured in a 0.747 playing client and server.
const REFUSED: &str = r#"
    local function Refused(Body)
        local Message
        table.sort({ 2, 1 }, function(A, B)
            if not Message then
                local Ok, Problem = pcall(Body)
                Message = if Ok then "ok" else Problem
            end
            return A < B
        end)
        return Message
    end
"#;

/// A wait where the thread cannot yield raises a client's error before it registers anything, so the thread's later waits
/// take their own time and return their own results; WaitForChild of a child that exists and an Invoke whose handler does
/// not yield answer, and a handler that yields goes on on its own thread. (The refusal was the VM's "attempt to yield across
/// metamethod/C-call boundary", after the wait had registered: its timer, the event's fire or the child's arrival resumed the
/// thread's next wait early with the refused wait's values.)
#[test]
fn a_wait_where_the_thread_cannot_yield_raises_and_leaves_nothing_behind() {
    let (lua, engine) = start();
    let source = format!(
        "{REFUSED}{}",
        r#"
        local Event = Instance.new("BindableEvent")
        local Folder = Instance.new("Folder")
        local Existing = Instance.new("Folder")
        Existing.Name = "Existing"
        Existing.Parent = Folder
        local Quiet = Instance.new("BindableFunction")
        Quiet.OnInvoke = function(Value) return Value * 2 end
        local Slow = Instance.new("BindableFunction")
        Slow.OnInvoke = function() task.wait(0.05) print("handler went on", coroutine.isyieldable()) return 1 end
        local Unhandled = Instance.new("BindableFunction")
        print("task.wait", Refused(function() task.wait(0.1) end))
        print("task.wait-string", Refused(function() task.wait("x") end))
        print("wait", Refused(function() wait(0.1) end))
        print("Wait", Refused(function() Event.Event:Wait() end))
        print("Wait-indexed", Refused(function() Event.Event.Wait(Event.Event) end))
        print("WaitForChild", Refused(function() Folder:WaitForChild("Missing", 0.1) end))
        print("WaitForChild-nil", Refused(function() Folder:WaitForChild(nil) end))
        print("WaitForChild-existing", Refused(function() assert(Folder:WaitForChild("Existing") == Existing) end))
        print("Invoke-quiet", Refused(function() assert(Quiet:Invoke(21) == 42) end))
        print("Invoke-slow", Refused(function() Slow:Invoke() end))
        print("Invoke-unhandled", Refused(function() Unhandled:Invoke() end))
        print("coroutine.yield", Refused(function() coroutine.yield() end))
        print("task.wait-string-yieldable", select(2, pcall(function() task.wait("x") end)))
        task.delay(0.05, function() Event:Fire("refused") end)
        local Started = os.clock()
        local Waited = task.wait(0.2)
        print("next wait", Waited >= 0.19, os.clock() - Started >= 0.19)
        task.delay(0.1, function() Event:Fire("later") end)
        print("next Wait", Event.Event:Wait())
        task.delay(0.1, function()
            local Child = Instance.new("Folder")
            Child.Name = "Late"
            Child.Parent = Folder
        end)
        print("next WaitForChild", Folder:WaitForChild("Late").Name)
        "#
    );
    run(&lua, &engine, &source);
    scheduler::run_until(&lua, &engine, 1.0).unwrap();
    assert_eq!(
        lines(&engine),
        vec![
            "print: task.wait thread is not yieldable",
            "print: task.wait-string thread is not yieldable",
            "print: wait thread is not yieldable",
            "print: Wait thread cannot be yielded to Wait",
            "print: Wait-indexed thread cannot be yielded to Wait",
            "print: WaitForChild thread is not yieldable",
            "print: WaitForChild-nil Argument 1 missing or nil",
            "print: WaitForChild-existing ok",
            "print: Invoke-quiet ok",
            "print: Invoke-slow thread is not yieldable",
            "print: Invoke-unhandled thread is not yieldable",
            "print: coroutine.yield attempt to yield across metamethod/C-call boundary",
            "print: task.wait-string-yieldable Test:36: invalid argument #1 to 'wait' (number expected, got string)",
            "print: handler went on true",
            "print: next wait true true",
            "print: next Wait later",
            "print: next WaitForChild Late",
        ]
    );
}

/// require where the thread cannot yield: of a module another thread is loading it raises `thread is not yieldable`; a module
/// that must load loads on a thread of its own, which owns it (a require of itself from its chunk is recursive), and the
/// require gets its result when it loads without yielding, else raises `thread is not yieldable` while it goes on, and a later
/// require gets its result (measured in a 0.747 playing client and server: tools/parity/probes/yield_refused_require.luau).
#[test]
fn a_require_where_the_thread_cannot_yield_loads_the_module_on_a_thread_of_its_own() {
    let (lua, engine) = start();
    let source = format!(
        "{REFUSED}{}",
        r#"
        local function Make(Name, Source)
            local Module = Instance.new("ModuleScript")
            Module.Name = Name
            __setSource(Module, Source)
            Module.Parent = workspace
            return Module
        end
        local Loading = Make("Loading", "task.wait(0.1) return 'loaded'")
        task.spawn(function() print("first", require(Loading)) end)
        print("loading", Refused(function() require(Loading) end))
        local Waits = Make("Waits", "task.wait(0.1) print('module went on') return 'waited'")
        print("waits", Refused(function() require(Waits) end))
        local Plain = Make("Plain", "return 'plain'")
        print("plain", Refused(function() assert(require(Plain) == "plain") end))
        local Recursive = Make("Recursive", "return require(script)")
        print("recursive", Refused(function() require(Recursive) end))
        local Started = os.clock()
        task.wait(0.3)
        print("next wait", os.clock() - Started >= 0.29)
        print("again", require(Loading), require(Waits))
        "#
    );
    run(&lua, &engine, &source);
    scheduler::run_until(&lua, &engine, 1.0).unwrap();
    assert_eq!(
        lines(&engine),
        vec![
            "print: loading thread is not yieldable",
            "print: waits thread is not yieldable",
            "print: plain ok",
            "error: Requested module was required recursively",
            "print: recursive Requested module experienced an error while loading",
            "print: first loaded",
            "print: module went on",
            "print: next wait true",
            "print: again loaded waited",
        ]
    );
}

#[test]
fn a_once_connection_reads_disconnected_from_the_fire_on() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/signals.luau): Connected is false right after the
    // Fire, the handler still runs once, and a second Fire before it runs does not reach it
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Event = Instance.new("BindableEvent")
        local Connection = Event.Event:Once(function(Value) print("once", Value) end)
        Event.Event:Connect(function(Value) print("normal", Value) end)
        print("before", Connection.Connected)
        Event:Fire(1)
        Event:Fire(2)
        print("after-fire", Connection.Connected)
        task.defer(function() print("later", Connection.Connected) end)
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec!["print: before true", "print: after-fire false", "print: normal 1", "print: once 1", "print: normal 2", "print: later false"]
    );
}

#[test]
fn connecting_a_value_that_is_not_a_function_logs_and_gives_a_disconnected_connection() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/signals.luau): Connect(nil) and Connect(5) raise
    // nothing; the message is logged, the connection is not connected and firing the signal skips it
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Event = Instance.new("BindableEvent")
        local Connection = Event.Event:Connect(nil)
        print(typeof(Connection), Connection.Connected)
        Event.Event:Once(5)
        Event.Event:Connect(function() print("handler") end)
        Event:Fire()
        "#,
    );
    let message = "error: Attempt to connect failed: Passed value is not a function";
    assert_eq!(lines(&engine), vec![message, "print: RBXScriptConnection false", message, "print: handler"]);
}

#[test]
fn bindables_copy_tables_for_every_receiver_as_roblox_does() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/signals.luau): lua/bindable.rs
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local Event = Instance.new("BindableEvent")
        local Got = {}
        Event.Event:Connect(function(...) table.insert(Got, table.pack(...)) end)
        Event.Event:Connect(function(...) table.insert(Got, table.pack(...)) end)
        local Sent = setmetatable({ 1, 2, Key = "dropped", Nested = { 3 } }, { __index = function() return "meta" end })
        Event:Fire(Sent)
        Sent[1] = "changed after the fire"
        task.wait()
        local A, B = Got[1][1], Got[2][1]
        print(#Got, A == Sent, A == B, getmetatable(A), A[1], A[2], A.Key, A.Missing)
        local function Keys(Value)
            local Out = {}
            for Key, Item in Value do
                table.insert(Out, typeof(Key) .. ":" .. string.gsub(tostring(Key), "0x%x+", "ADDR") .. "=" .. tostring(Item))
            end
            table.sort(Out)
            return table.concat(Out, " ")
        end
        table.clear(Got)
        Event:Fire({ [1.5] = "a", [-2.7] = "b", [1e20] = "c", Name = "d" }, coroutine.create(function() end), { Part = Instance.new("Part") })
        task.wait()
        local Args = Got[1]
        print(Keys(Args[1]), Args.n, Args[2], typeof(Args[3].Part))
        table.clear(Got)
        Event:Fire({ [{}] = 1 })
        Event:Fire({ [Enum.Material.Plastic] = 1, [Vector3.new(1, 2, 3)] = 2 })
        task.wait()
        print(Keys(Got[1][1]), "|", Keys(Got[3][1]))
        local Cyclic = {}
        Cyclic.Self = Cyclic
        print(pcall(Event.Fire, Event, Cyclic))
        print(pcall(Event.Fire, Event, { [true] = 1 }))
        local Bytes = buffer.create(1)
        local Arrived
        Event.Event:Once(function(_, Value) Arrived = Value end)
        Event:Fire(nil, Bytes)
        task.wait()
        print(Arrived ~= Bytes, buffer.len(Arrived))
        local Function = Instance.new("BindableFunction")
        local Inside
        Function.OnInvoke = function(Value)
            Inside = Value
            return Value, setmetatable({ 1, Key = "k" }, {})
        end
        local Argument = { 1, Key = "k" }
        local Back, Second = Function:Invoke(Argument)
        print(Inside == Argument, Back == Inside, Inside.Key, Back[1], getmetatable(Second), Second.Key, Second[1])
        print(pcall(Function.Invoke, Function, Cyclic))
        "##,
    );
    for _ in 0..12 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    assert_eq!(
        lines(&engine),
        vec![
            "print: 2 false false nil 1 2 nil nil",
            "print: string:-2=b string:-9223372036854775808=c string:1=a string:Name=d 3 nil Instance",
            "print: string:<Table> (table: ADDR)=1 | string:<Vector3> (1, 2, 3)=2 string:<token> (Enum.Material.Plastic)=1",
            "print: false tables cannot be cyclic",
            "print: false Invalid table key type used",
            "print: true 1",
            "print: false false nil 1 nil nil 1",
            "print: false tables cannot be cyclic",
        ]
    );
}

/// A dictionary key's `__tostring` runs on the sending thread, and an error it raises is the sending call's, as it was raised, the
/// first failing key's alone, with nothing sent: Fire, Invoke and JSONEncode alike; a key whose `__tostring` cleared its table
/// raises `invalid key to 'next'`; a CFrame key is named `<CoordinateFrame>`; and JSONEncode keeps entries whose keys share a name
/// (measured, a 0.747 playing client and server: tools/parity/probes/table_keys.luau). On main each failing key arrived named
/// `<Table> ()`, the cleared table's remains arrived, and JSONEncode kept one entry.
#[test]
fn a_keys_failing_tostring_is_raised_from_the_sending_call() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r##"
        local HttpService = game:GetService("HttpService")
        local Event = Instance.new("BindableEvent")
        local Arrived = 0
        Event.Event:Connect(function() Arrived += 1 end)
        local Function = Instance.new("BindableFunction")
        Function.OnInvoke = function(Value) return Value end
        local Calls = 0
        local Problem = { code = 1 }
        local function Key(Result)
            return setmetatable({}, { __tostring = function() Calls += 1 return Result() end })
        end
        for _, Call in { function(T) return Event:Fire(T) end, function(T) return Function:Invoke(T) end, function(T) return HttpService:JSONEncode(T) end } do
            Calls = 0
            local Ok, Message = pcall(Call, { [Key(function() return nil end)] = 1, [Key(function() return nil end)] = 2 })
            local _, Raised = pcall(Call, { [Key(function() error(Problem) end)] = 1 })
            local Clearing = { a = 1, b = 2 }
            Clearing[Key(function() table.clear(Clearing) return "x" end)] = 3
            local _, Cleared = pcall(Call, Clearing)
            print(Ok, Message, Calls, rawequal(Raised, Problem), Cleared)
        end
        task.wait()
        print(Arrived, HttpService:JSONEncode({ [CFrame.new()] = 1 }), HttpService:JSONEncode({ [1.5] = "a", [1.25] = "b" }))
        "##,
    );
    assert_eq!(
        output,
        vec![
            "print: false '__tostring' must return a string 3 true invalid key to 'next'",
            "print: false '__tostring' must return a string 3 true invalid key to 'next'",
            "print: false '__tostring' must return a string 3 true invalid key to 'next'",
            r#"print: 0 {"<CoordinateFrame> (0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1)":1} {"1":"b","1":"a"}"#,
        ]
    );
}

/// A BindableFunction's callback runs on a thread of its own; an error it raises before it first yields is logged as an uncaught
/// error is (an error line, ScriptContext.Error), a value that is no string as "Error occurred, no output from Luau.", and one
/// raised after a yield is not; the caller gets the error as text, empty for a value that is no string. A spawned thread's
/// uncaught table error fires ScriptContext.Error too. Measured in a 0.747 playing client and server:
/// tools/parity/probes/loopback/remote_callback_errors.luau. On main the callback ran on its caller's thread, nothing was logged,
/// a table raised reached the caller as that table, and a spawned thread's table error fired no ScriptContext.Error.
#[test]
fn a_bindable_functions_callback_error_is_logged_until_it_yields() {
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r##"
        local Reported = {}
        game:GetService("ScriptContext").Error:Connect(function(Message) table.insert(Reported, Message) end)
        local Function = Instance.new("BindableFunction")
        local Caller = coroutine.running()
        local Same
        Function.OnInvoke = function(Kind)
            Same = coroutine.running() == Caller
            if Kind == "table" then
                error({ code = 1 })
            elseif Kind == "string" then
                error("callback failed", 0)
            elseif Kind == "later" then
                task.wait()
                error("failed later", 0)
            end
            return Kind
        end
        local _, Problem = pcall(Function.Invoke, Function, "table")
        print(typeof(Problem), #Problem)
        print(pcall(Function.Invoke, Function, "string"))
        print(pcall(Function.Invoke, Function, "later"))
        print(Function:Invoke("fine"), Same)
        task.spawn(function() error({}) end)
        task.wait()
        print(table.concat(Reported, " | "))
        "##,
    );
    assert_eq!(
        output,
        vec![
            "error: Error occurred, no output from Luau.",
            "print: string 0",
            "error: callback failed",
            "print: false callback failed",
            "print: false failed later",
            "print: fine false",
            "error: Error occurred, no output from Luau.",
            "print: Error occurred, no output from Luau. | callback failed | Error occurred, no output from Luau.",
        ]
    );
}

/// A table an event or a curve key copies is walked on the Lua stack above the call's own values (the outermost at the
/// frame's height, an array's table one slot up, a dictionary's two), and one above slot 300 raises "Max lua stack size
/// reached" without a position, before a cycle through it does (measured in a 0.747 playing client and server,
/// tools/parity/probes/table_depth.luau), however deep the table is: 100,000 levels raise the same error.
#[test]
fn copies_refuse_tables_nested_past_the_lua_stack() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local function Nest(Depth, Key)
            local Root = {}
            local At = Root
            for _ = 2, Depth do
                local Inner = {}
                At[Key or 1] = Inner
                At = Inner
            end
            return Root
        end
        local function Ring(Depth)
            local Root = Nest(Depth)
            local At = Root
            while At[1] do
                At = At[1]
            end
            At[1] = Root
            return Root
        end
        local function Height(Value, Key)
            local Count = 0
            while type(Value) == "table" do
                Count += 1
                Value = Value[Key or 1]
            end
            return Count
        end
        local function Try(Name, Call)
            local Ok, Result = pcall(Call)
            print(Name, Ok, Result)
        end
        local Event = Instance.new("BindableEvent")
        local Got = {}
        Event.Event:Connect(function(...)
            local Last = select(select("#", ...), ...)
            table.insert(Got, math.max(Height(Last), Height(Last, "a")))
        end)
        Try("fire 299", function() Event:Fire(Nest(299)) end)
        Try("fire 300", function() Event:Fire(Nest(300)) end)
        Try("fire 2nd 298", function() Event:Fire(1, Nest(298)) end)
        Try("fire 2nd 299", function() Event:Fire(1, Nest(299)) end)
        Try("fire dict 150", function() Event:Fire(Nest(150, "a")) end)
        Try("fire dict 151", function() Event:Fire(Nest(151, "a")) end)
        Try("fire under dict 297", function() Event:Fire({ Key = Nest(297) }) end)
        Try("fire under dict 298", function() Event:Fire({ Key = Nest(298) }) end)
        local Many = table.create(400, 0)
        table.insert(Many, {})
        Try("fire after 400", function() Event:Fire(table.unpack(Many)) end)
        Try("fire ring 299", function() Event:Fire(Ring(299)) end)
        Try("fire ring 3", function() Event:Fire(Ring(3)) end)
        Try("fire 100000", function() Event:Fire(Nest(100000)) end)
        task.wait()
        print("received", table.concat(Got, " "))
        local Echo = Instance.new("BindableFunction")
        Echo.OnInvoke = function(Value) return Height(Value) end
        local Maker = Instance.new("BindableFunction")
        Maker.OnInvoke = function(Depth) return Nest(Depth) end
        Try("invoke 299", function() return Echo:Invoke(Nest(299)) end)
        Try("invoke 300", function() return Echo:Invoke(Nest(300)) end)
        Try("invoke 100000", function() return Echo:Invoke(Nest(100000)) end)
        Try("result 300", function() return Height(Maker:Invoke(300)) end)
        Try("result 301", function() return Height(Maker:Invoke(301)) end)
        Try("result 100000", function() return Height(Maker:Invoke(100000)) end)
        Try("key 299", function() return Height(ValueCurveKey.new(0, Nest(299)).Value) end)
        Try("key 300", function() return ValueCurveKey.new(0, Nest(300)) end)
        Try("key mode 298", function() return Height(ValueCurveKey.new(0, Nest(298), Enum.KeyInterpolationMode.Linear).Value) end)
        Try("key mode 299", function() return ValueCurveKey.new(0, Nest(299), Enum.KeyInterpolationMode.Linear) end)
        local Key = ValueCurveKey.new(0, 1)
        Try("key write 298", function() Key.Value = Nest(298) return Height(Key.Value) end)
        Try("key write 299", function() Key.Value = Nest(299) end)
        Try("key 100000", function() return ValueCurveKey.new(0, Nest(100000)) end)
        "##,
    );
    for _ in 0..4 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let deep = "Max lua stack size reached";
    let wanted = [
        "print: fire 299 true nil".to_string(),
        format!("print: fire 300 false {deep}"),
        "print: fire 2nd 298 true nil".into(),
        format!("print: fire 2nd 299 false {deep}"),
        "print: fire dict 150 true nil".into(),
        format!("print: fire dict 151 false {deep}"),
        "print: fire under dict 297 true nil".into(),
        format!("print: fire under dict 298 false {deep}"),
        "print: fire after 400 true nil".into(),
        format!("print: fire ring 299 false {deep}"),
        "print: fire ring 3 false tables cannot be cyclic".into(),
        format!("print: fire 100000 false {deep}"),
        "print: received 299 298 150 1 1".into(),
        "print: invoke 299 true 299".into(),
        format!("print: invoke 300 false {deep}"),
        format!("print: invoke 100000 false {deep}"),
        "print: result 300 true 300".into(),
        format!("print: result 301 false {deep}"),
        format!("print: result 100000 false {deep}"),
        "print: key 299 true 299".into(),
        format!("print: key 300 false {deep}"),
        "print: key mode 298 true 298".into(),
        format!("print: key mode 299 false {deep}"),
        "print: key write 298 true 298".into(),
        format!("print: key write 299 false {deep}"),
        format!("print: key 100000 false {deep}"),
    ];
    assert_eq!(lines(&engine), wanted);
}

/// A table is copied once for each place it appears, as Roblox copies it (lua/bindable.rs), so a small table can stand for
/// a copy of any size, and ree stops a copy past its guard (bindable.rs `COPY_VALUES`: the outermost table, every array
/// slot and dictionary entry walked, a table each time it appears, the call's values together) with its own error. Each
/// big table here holds 200 references to a table of 100,000 threads first (threads arrive as nil, so their slots make
/// nothing) and only then 100,000 references to a table of 100,000 numbers: 10^10 numbers without the guard, as a tree of
/// `{ t, t }` 40 deep is 2^40 tables; the guard stops each copy before a number is copied. A copy below it arrives whole.
#[test]
fn copies_stop_at_rees_guard_on_a_table_copied_for_each_place_it_appears() {
    let (lua, engine) = start();
    let guard = crate::lua::bindable::COPY_VALUES;
    run(
        &lua,
        &engine,
        &r##"
        local Thread = coroutine.running()
        local Threads = table.create(100000, Thread)
        local Numbers = table.create(100000, 1)
        local Huge = table.create(100200)
        for Index = 1, 200 do
            Huge[Index] = Threads
        end
        for Index = 201, 100200 do
            Huge[Index] = Numbers
        end
        local Tree = {}
        for _ = 1, 40 do
            Tree = { Threads, Tree, Tree }
        end
        local function Try(Name, Call)
            print(Name, pcall(Call))
        end
        local Event = Instance.new("BindableEvent")
        local Got = {}
        Event.Event:Connect(function(Value)
            local Last = Value[#Value]
            table.insert(Got, `{#Value} {if type(Last) == "table" then #Last else 0} {Value[1] ~= Value[2]}`)
        end)
        Try("fire huge", function() Event:Fire(Huge) end)
        Try("fire tree", function() Event:Fire(Tree) end)
        Try("fire at the guard", function() Event:Fire(table.create(GUARD - 1, Thread)) end)
        Try("fire past the guard", function() Event:Fire(table.create(GUARD, Thread)) end)
        Try("fire two halves", function() Event:Fire(table.create(GUARD // 2, Thread), table.create(GUARD // 2, Thread)) end)
        local Fan = table.create(300, table.create(1000, 1))
        Try("fire fan", function() Event:Fire(Fan) end)
        task.wait()
        print("received", table.concat(Got, ", "))
        local Echo = Instance.new("BindableFunction")
        Echo.OnInvoke = function(Value) return #Value end
        local Maker = Instance.new("BindableFunction")
        Maker.OnInvoke = function() return Huge end
        Try("invoke huge", function() return Echo:Invoke(Huge) end)
        Try("invoke fan", function() return Echo:Invoke(Fan) end)
        Try("result huge", function() return Maker:Invoke() end)
        Try("key huge", function() return ValueCurveKey.new(0, Huge) end)
        local Key = ValueCurveKey.new(0, 1)
        Try("key write tree", function() Key.Value = Tree end)
        Try("key fan", function() return #ValueCurveKey.new(0, Fan).Value end)
        "##
        .replace("GUARD", &guard.to_string()),
    );
    for _ in 0..4 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let past = crate::lua::bindable::too_large(guard, "values");
    let wanted = [
        format!("print: fire huge false {past}"),
        format!("print: fire tree false {past}"),
        "print: fire at the guard true".into(),
        format!("print: fire past the guard false {past}"),
        format!("print: fire two halves false {past}"),
        "print: fire fan true".into(),
        // (the threads arrive as nil: an empty table)
        "print: received 0 0 false, 300 1000 true".into(),
        format!("print: invoke huge false {past}"),
        "print: invoke fan true 300".into(),
        format!("print: result huge false {past}"),
        format!("print: key huge false {past}"),
        format!("print: key write tree false {past}"),
        "print: key fan true 300".into(),
    ];
    assert_eq!(lines(&engine), wanted);
}

#[test]
fn number_and_boolean_properties_take_any_value_as_in_roblox() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/coercion.luau)
    assert_script(
        r#"
        local Int = Instance.new("IntValue")
        local function IntOf(Value) Int.Value = Value return Int.Value end
        assert(IntOf(2.5) == 3 and IntOf(-2.5) == -3 and IntOf(2.7) == 3 and IntOf(-0.5) == -1)
        assert(IntOf(1e20) == 0 and IntOf(0 / 0) == 0 and IntOf(math.huge) == 0)
        assert(IntOf("0x10") == 16 and IntOf(" 5 ") == 5 and IntOf("five") == 0 and IntOf(true) == 0 and IntOf(nil) == 0)
        local Frame = Instance.new("Frame")
        local function ZOf(Value) Frame.ZIndex = Value return Frame.ZIndex end
        assert(ZOf(2.7) == 2 and ZOf(-2.7) == -2 and ZOf(2 ^ 31) == -2147483648 and ZOf(0 / 0) == -2147483648)
        assert(1 / ZOf(-0.5) == math.huge, "an integer reads no -0")
        Frame.BorderSizePixel = 1000
        assert(Frame.BorderSizePixel == 100)
        Frame.BorderSizePixel = -3
        assert(Frame.BorderSizePixel == 0)
        local Label = Instance.new("TextLabel")
        Label.TextSize = 2 ^ 31
        assert(Label.TextSize == 1)
        local Bool = Instance.new("BoolValue")
        for _, Truthy in { 0, "", {}, Vector3.zero } do
            Bool.Value = false
            Bool.Value = Truthy
            assert(Bool.Value == true)
        end
        Bool.Value = nil
        assert(Bool.Value == false)
        "#,
    );
}

#[test]
fn enums_and_engine_datatypes() {
    assert_script(
        r#"
        assert(Enum.Font.GothamSemibold == Enum.Font.GothamMedium)
        -- legacy names resolve to the item that carries the current name, and nothing lists them (measured, a 0.747 client:
        -- tools/parity/probes/enum_legacy.luau). KeyCode's value 0 is None; Unknown is its legacy name
        local Unknown = Enum.KeyCode.Unknown
        assert(Unknown == Enum.KeyCode.None and Unknown.Name == "None" and Unknown.Value == 0 and tostring(Unknown) == "Enum.KeyCode.None")
        assert(Enum.KeyCode:FromValue(0).Name == "None" and Enum.KeyCode:GetEnumItems()[1].Name == "None")
        assert(Enum.KeyCode.Touch == Enum.KeyCode.TouchPosition and Enum.Material["Corroded Metal"] == Enum.Material.CorrodedMetal)
        assert(Enum.Axis.Left == Enum.Axis.X and Enum.Axis.Top.Name == "Y" and Enum.SurfaceType.Bumps == Enum.SurfaceType.Glue)
        assert(Enum.QualityLevel["Level  1"] == Enum.QualityLevel.Level01 and Enum.InputType.Throtle == Enum.InputType.NoInput)
        for _, Item in Enum.Axis:GetEnumItems() do
            assert(Item.Name ~= "Left" and Item.Name ~= "Top")
        end
        assert(typeof(Enum) == "Enums" and typeof(Enum.Font) == "Enum" and typeof(Enum.Font.Arial) == "EnumItem")
        assert(tostring(Enum) == "Enums" and tostring(Enum.Font) == "Font" and tostring(Enum.Font.Arial) == "Enum.Font.Arial")
        assert(Enum.Font.Arial.EnumType == Enum.Font and Enum.Font.Arial:IsA("Font") and not Enum.Font.Arial:IsA("Nope"))
        assert(Enum.Font:FromName("Arial") == Enum.Font.Arial and Enum.Font:FromValue(Enum.Font.Arial.Value) == Enum.Font.Arial)
        local Seen = {}
        Seen[Enum.Font.Arial] = true
        assert(Seen[Enum.Font.Arial])
        assert(getmetatable(Enum.Font.Arial) == "The metatable is locked")
        local Face = Font.new("rbxasset://fonts/families/GothamSSm.json", Enum.FontWeight.Bold)
        assert(typeof(Face) == "Font" and Face.Bold and Face.Style == Enum.FontStyle.Normal)
        assert(tostring(Face) == "Font { Family = rbxasset://fonts/families/GothamSSm.json, Weight = Bold, Style = Normal }")
        assert(Face == Font.new("rbxasset://fonts/families/GothamSSm.json", "Bold"))
        local Label = Instance.new("TextLabel")
        Label.FontFace = Face
        Face.Bold = false
        assert(Label.FontFace.Bold and Label.FontFace ~= Face)
        assert(tostring(Content.none) == "Content{SourceType=None}" and Content.fromAssetId(5).Uri == "rbxassetid://5")
        local Info = TweenInfo.new(2, Enum.EasingStyle.Linear)
        assert(Info.Time == 2 and Info.EasingDirection == Enum.EasingDirection.Out and Info.RepeatCount == 0)
        local R = Random.new(7)
        assert(R:NextNumber() == 0.8737085924861333 and R:NextNumber() == 0.4321572161309693)
        assert(Random.new(7):NextInteger(1, 100) == 2 or true)
        assert(typeof(DateTime.now()) == "DateTime" and DateTime.fromUnixTimestamp(0):ToIsoDate() == "1970-01-01T00:00:00Z")
        assert(UDim2.new(".5", "-3.9", false, 9.8) == UDim2.new(0.5, -3, 0, 9))
        assert(Color3.fromRGB(300, -5, 128.6).R == Color3.new(300 / 255, 0, 0).R)
        local Ok, Err = pcall(UDim2.new, UDim.new(), 5)
        assert(not Ok and Err == "Invalid arguments to constructor")
        assert(getmetatable(Vector2) == "The metatable is locked")
        assert(typeof(vector.create(1, 2, 3)) == "Vector3")
        assert(ColorSequence.new(Color3.new(1, 0, 0), Color3.new(0, 0, 1)).Keypoints[2].Value == Color3.new(0, 0, 1))
        "#,
    );
}

#[test]
fn a_modules_chunk_is_named_by_its_full_name_as_in_roblox() {
    // Measured in a playing client (Studio 0.744, tools/parity/probes/chunknames.luau): debug.info, error positions
    // and tracebacks name a module by its full name, without Luau's `[string "..."]` wrapping.
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        Folder.Name = "Probes"
        Folder.Parent = game:GetService("ReplicatedStorage")
        local M = Instance.new("ModuleScript")
        M.Name = "Chunk"
        __setSource(M, [[local M = {}
function M.info() return debug.info(1, 's') end
function M.fail() error('inside') end
function M.trace() return debug.traceback('t') end
return M]])
        M.Parent = Folder
        local Module = require(M)
        print(Module.info())
        print(select(2, pcall(Module.fail)))
        print((string.split(Module.trace(), "\n"))[2])
        "#,
    );
    let output = lines(&engine);
    assert_eq!(output[0], "print: ReplicatedStorage.Probes.Chunk");
    assert_eq!(output[1], "print: ReplicatedStorage.Probes.Chunk:3: inside");
    assert_eq!(output[2], "print: ReplicatedStorage.Probes.Chunk:4 function trace");
}

#[test]
fn modules_require_and_environments() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        Folder.Parent = game:GetService("ReplicatedStorage")
        local function Module(Name, Source)
            local M = Instance.new("ModuleScript")
            M.Name = Name
            __setSource(M, Source)
            M.Parent = Folder
            return M
        end
        Module("Sibling", "return { Value = 42 }")
        Module("Caller", "local S = require('./Sibling') return { S = S, Script = script }")
        Module("Cycle", "return require(script)")
        Module("Two", "return 1, 2")
        Module("Broken", "error('boom', 0)")
        Module("Yields", "task.wait(0.05) return 'late'")
        local Caller = require(Folder.Caller)
        print(Caller.S.Value, Caller.Script == Folder.Caller, require(Folder.Sibling) == Caller.S)
        print(pcall(require, Folder.Cycle))
        print(pcall(require, Folder.Two))
        print(pcall(require, Folder.Broken))
        print(pcall(require, Folder.Broken))
        print(pcall(require, 5))
        task.spawn(function() print("second", require(Folder.Yields)) end)
        print("first", require(Folder.Yields))
        "#,
    );
    scheduler::run_until(&lua, &engine, 0.2).unwrap();
    let output = lines(&engine);
    assert_eq!(output[0], "print: 42 true true");
    assert!(output.contains(&"error: Requested module was required recursively".to_string()), "{output:?}");
    assert!(output.contains(&"error: boom".to_string()), "{output:?}");
    let prints: Vec<&str> = output.iter().filter(|l| l.starts_with("print: ")).map(String::as_str).collect();
    assert!(prints[1].ends_with("Requested module experienced an error while loading"), "{prints:?}");
    assert!(prints[2].ends_with("Module code did not return exactly one value"), "{prints:?}");
    assert!(prints[3].ends_with("Requested module experienced an error while loading"), "{prints:?}");
    assert!(prints[4].ends_with("Requested module experienced an error while loading"), "{prints:?}");
    // (a playing client's text: measured, tools/parity/probes/errors.luau)
    assert!(prints[5].ends_with("require(assetId) cannot be called from a client.  assetId = 5"), "{prints:?}");
    assert!(prints.contains(&"print: first late") && prints.contains(&"print: second late"), "{prints:?}");
}

/// Bytecode kept from an earlier compile that does not load does not fail its module: the source is compiled again. (A cold
/// `golden --all` under heavy write load read module store pages back zero-filled, and Luau loads zero-led bytes as an error
/// whose text is only the chunk name, so stories failed with a shared module's name and no reason.) A source
/// that starts with a zero byte is text, as Roblox compiles it: Luau's lexer ends it there and the module returns nothing,
/// rather than mlua taking it for bytecode and Luau naming only the chunk.
#[test]
fn a_module_whose_kept_bytecode_does_not_load_compiles_its_source() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        Folder.Name = "Mods"
        Folder.Parent = game:GetService("ReplicatedStorage")
        local Kept = Instance.new("ModuleScript")
        Kept.Name = "Kept"
        __setSource(Kept, "return 'from source'")
        Kept.Parent = Folder
        local Zero = Instance.new("ModuleScript")
        Zero.Name = "Zero"
        __setSource(Zero, "\0\0return 1")
        Zero.Parent = Folder
        "#,
    );
    let kept = {
        let state = engine.state.borrow();
        let storage = state.dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
        let folder = state.dom.find_child(storage, b"Mods").unwrap();
        state.dom.find_child(folder, b"Kept").unwrap()
    };
    engine_lua::modules::keep_bytecode(&engine, kept, vec![0; 64]);
    let output = run(
        &lua,
        &engine,
        r#"
        local Mods = game:GetService("ReplicatedStorage").Mods
        print(require(Mods.Kept))
        print(pcall(require, Mods.Zero))
        "#,
    );
    let prints: Vec<&str> = output.iter().filter(|l| l.starts_with("print: ")).map(String::as_str).collect();
    assert_eq!(prints[0], "print: from source", "{output:?}");
    assert!(prints[1].ends_with("Module code did not return exactly one value"), "{output:?}");
}

#[test]
fn wait_for_child_and_services() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        task.spawn(function()
            local Child = Folder:WaitForChild("Late")
            print("got", Child.Name)
        end)
        task.spawn(function()
            print("timeout", Folder:WaitForChild("Never", 0.1))
        end)
        task.delay(0.05, function()
            local Late = Instance.new("Frame")
            Late.Name = "Late"
            Late.Parent = Folder
        end)
        print(game:GetService("Workspace") == workspace, workspace.CurrentCamera.Name, game.Players.LocalPlayer.Name)
        print(game:GetService("RunService"):IsStudio(), game:GetService("RunService"):IsEdit())
        print(workspace.CurrentCamera.ViewportSize)
        "#,
    );
    scheduler::run_until(&lua, &engine, 0.3).unwrap();
    let output = lines(&engine);
    assert_eq!(output[0], "print: true Camera Player");
    assert_eq!(output[1], "print: true true");
    assert_eq!(output[2], "print: 1920, 1080");
    assert!(output.contains(&"print: got Late".to_string()), "{output:?}");
    assert!(output.contains(&"print: timeout nil".to_string()), "{output:?}");
}

/// Instance lifetime (lua/raw.rs): unreachable parentless instances are freed; instances in the
/// DataModel, referenced by scripts or by other instances' properties keep their userdata
/// identity (weak-keyed tables included).
#[test]
fn instance_lifetime_and_identity() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Keep = Instance.new("Folder")
        Keep.Name = "Keep"
        Keep.Parent = workspace
        local Weak = setmetatable({}, { __mode = "k" })
        Weak[Keep] = true
        for Index = 1, 200 do
            local Child = Instance.new("Frame")
            Child.Name = "Child" .. Index
            Child.Parent = Keep
            local Loose = Instance.new("Frame")
            Loose.Name = "Loose" .. Index
            local Dead = Instance.new("Frame")
            Dead.Parent = Keep
            Dead:Destroy()
        end
        _G.Weak = Weak
        _G.Held = Instance.new("Frame")
        local Value = Instance.new("ObjectValue")
        Value.Parent = workspace
        Value.Value = Instance.new("Frame")
        Value.Value.Name = "Referenced"
        "#,
    );
    assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
    let before = engine.state.borrow().dom.live_count();
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
    let freed = engine_lua::raw::collect(&lua, &engine).unwrap();
    let after = engine.state.borrow().dom.live_count();
    assert!(freed >= 400, "freed {freed}");
    assert_eq!(after, before - freed);
    let output = run(
        &lua,
        &engine,
        r#"
        local Keep = workspace.Keep
        assert(#Keep:GetChildren() == 200)
        assert(_G.Weak[Keep], "identity of an instance in the DataModel")
        assert(_G.Held.Parent == nil and _G.Held.Name == "Frame")
        assert(workspace.Value.Value.Name == "Referenced")
        local First = Keep:GetChildren()
        for Index, Child in Keep:GetChildren() do
            assert(Child == First[Index] and Child.Name == "Child" .. Index)
        end
        First[1] = nil
        table.clear(First)
        assert(#Keep:GetChildren() == 200, "GetChildren returns a fresh table")
        local New = Instance.new("Frame")
        New.Parent = Keep
        assert(#Keep:GetChildren() == 201 and Keep:GetChildren()[201] == New)
        New.Parent = nil
        assert(#Keep:GetChildren() == 200)
        assert(typeof(New) == "Instance" and typeof(Vector3.new()) == "Vector3" and typeof(vector.create(1, 2, 3)) == "Vector3")
        assert(typeof(newproxy(true)) == "userdata" and typeof({}) == "table" and typeof(nil) == "nil" and typeof(1) == "number")
        assert(typeof(Enum.Font.Arial) == "EnumItem" and typeof(New.Changed) == "RBXScriptSignal" and typeof(UDim2.new()) == "UDim2")
        "#,
    );
    assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
}

/// A parentless tree a script still references through one instance is kept by every collection
/// (the instance that held it is asked first the next time, lua/raw.rs find_garbage), goes when
/// that instance leaves it, and the instance itself when the script lets go.
#[test]
fn a_held_tree_is_freed_once_its_holder_leaves() {
    let (lua, engine) = start();
    // (every run makes a script instance that the next collection frees, so the counts compared are
    // the live instances after each collection)
    let live = |script: &str| {
        let output = run(&lua, &engine, script);
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
        lua.gc_collect().unwrap();
        lua.gc_collect().unwrap();
        engine_lua::raw::collect(&lua, &engine).unwrap();
        engine.state.borrow().dom.live_count()
    };
    // 1 + 50 + 200 instances; two of them referenced
    let built = live(
        r#"
        local Root = Instance.new("Folder")
        for Index = 1, 50 do
            local Child = Instance.new("Frame")
            Child.Parent = Root
            for Inner = 1, 4 do
                Instance.new("Frame").Parent = Child
            end
        end
        _G.Holder = Root:GetChildren()[30]:GetChildren()[2]
        _G.Other = Root:GetChildren()[10]
        "#,
    );
    // Another userdata of the tree dies: the tree is looked at again and still held
    assert_eq!(live("_G.Other = nil"), built);
    assert_eq!(live("assert(_G.Holder.Parent.Parent.ClassName == 'Folder')"), built);
    // The holder leaves (its old parent's userdata dies, so the tree is looked at): the rest goes
    assert_eq!(live("local Parent = _G.Holder.Parent _G.Holder.Parent = nil"), built - 250);
    assert_eq!(live("assert(_G.Holder.Parent == nil and _G.Holder.ClassName == 'Frame')"), built - 250);
    assert_eq!(live("_G.Holder = nil"), built - 251);
}

/// A tree that lost its last userdata while an instance property still referenced it is looked at again: the
/// property is cleared later, nothing else signals it, and the tree goes (labels whose tween or holder ended after the
/// script let go of them stayed counted for a whole run).
#[test]
fn a_tree_kept_by_a_property_is_freed_when_the_property_clears() {
    let (lua, engine) = start();
    let live = |script: &str| {
        let output = run(&lua, &engine, script);
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
        lua.gc_collect().unwrap();
        lua.gc_collect().unwrap();
        engine_lua::raw::collect(&lua, &engine).unwrap();
        engine.state.borrow().dom.live_count()
    };
    let empty = live("");
    // 1 + 20 instances in a tree no script holds, one ObjectValue in the Workspace naming its root (and the
    // two trees' roots, a second tree it names through the first)
    let built = live(
        r#"
        local Root = Instance.new("Folder")
        for Index = 1, 20 do
            Instance.new("Frame").Parent = Root
        end
        local Second = Instance.new("Frame")
        Second.Parent = Instance.new("Folder")
        local Value = Instance.new("ObjectValue")
        Value.Name = "Pointer"
        Value.Parent = workspace
        Value.Value = Root
        local Chain = Instance.new("ObjectValue")
        Chain.Parent = Root
        Chain.Value = Second.Parent
        "#,
    );
    // (the tree is 22 instances with the chain's ObjectValue, the second tree 2, the pointer 1)
    assert_eq!(built, empty + 22 + 2 + 1, "both trees and the pointer");
    assert_eq!(live(""), built, "a property still names the tree");
    assert_eq!(live("workspace.Pointer.Value = nil"), built - 22 - 2, "the property cleared: both trees go");
    assert_eq!(live(""), built - 22 - 2);
}

/// A tree whose only holder was a tween's pin is looked at again when the pin goes: a frame tweened and dropped
/// while outside the DataModel is held until its tween is collected, and nothing else signals that (the trail labels
/// of removed bars, 25,000 of them in a 34-minute run).
#[test]
fn a_tree_pinned_by_a_tween_is_freed_when_the_tween_goes() {
    let (lua, engine) = start();
    let live = |script: &str| {
        let output = run(&lua, &engine, script);
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
        lua.gc_collect().unwrap();
        lua.gc_collect().unwrap();
        engine_lua::raw::collect(&lua, &engine).unwrap();
        engine.state.borrow().dom.live_count()
    };
    let empty = live("");
    // 1 + 3 instances outside the DataModel, the root tweened (the tween pins it) and then dropped with its tween
    let built = live(
        r#"
        local Root = Instance.new("Frame")
        for Index = 1, 3 do
            Instance.new("Frame").Parent = Root
        end
        local Tween = game:GetService("TweenService"):Create(Root, TweenInfo.new(1), { Size = UDim2.fromOffset(5, 5) })
        "#,
    );
    assert!(built >= empty, "{built} {empty}");
    assert_eq!(live(""), empty, "the tween went with the script; its pin was the tree's last holder");
    assert_eq!(live(""), empty);
}

/// Destroying a Model clears its PrimaryPart, as Studio does (5 of 5 measured); unparenting it keeps the part, and
/// a destroyed ancestor destroys the Models below it too.
#[test]
fn a_destroyed_model_lets_go_of_its_primary_part() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local function Rig()
            local Model = Instance.new("Model")
            local Root = Instance.new("Part")
            Root.Parent = Model
            Model.PrimaryPart = Root
            return Model, Root
        end
        local Kept, KeptRoot = Rig()
        Kept.Parent = workspace
        Kept.Parent = nil
        assert(Kept.PrimaryPart == KeptRoot, "unparenting keeps the PrimaryPart")
        local Gone = Rig()
        Gone.Parent = workspace
        Gone:Destroy()
        assert(Gone.PrimaryPart == nil, "destroy clears the PrimaryPart")
        local Folder = Instance.new("Folder")
        local Inner = Rig()
        Inner.Parent = Folder
        Folder:Destroy()
        assert(Inner.PrimaryPart == nil, "a Model below a destroyed instance is destroyed too")
        local Bare = Instance.new("Model")
        Bare:Destroy()
        assert(Bare.PrimaryPart == nil)
        "#,
    );
    assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
}

/// A disconnected connection lets go of its handler (signal.rs): a removed UI whose handlers
/// captured its instances is freed once they are disconnected, a :Once() handler after it ran, and
/// a UI whose handlers stay connected is kept (the instance <-> handler cycle, as in Roblox).
#[test]
fn disconnected_handlers_release_what_they_capture() {
    let (lua, engine) = start();
    let live = |script: &str| {
        let output = run(&lua, &engine, script);
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
        scheduler::flush_deferred(&lua, &engine).unwrap();
        lua.gc_collect().unwrap();
        lua.gc_collect().unwrap();
        engine_lua::raw::collect(&lua, &engine).unwrap();
        engine.state.borrow().dom.live_count()
    };
    let empty = live("");
    // Three trees of 1 + 10 instances outside the DataModel, every child's handler capturing its
    // tree: disconnected by the script, a :Once() handler that ran, and one left connected
    let built = live(
        r#"
        local function Build(Bind)
            local Root = Instance.new("Frame")
            for Index = 1, 10 do
                local Button = Instance.new("TextButton")
                Button.Parent = Root
                Bind(Button, function() return Root, Button end)
            end
        end
        Build(function(Button, Handler)
            local Connection = Button:GetPropertyChangedSignal("Text"):Connect(Handler)
            Button.Text = "changed"
            Connection:Disconnect()
            assert(not Connection.Connected)
        end)
        Build(function(Button, Handler)
            Button:GetPropertyChangedSignal("Text"):Once(Handler)
            Button.Text = "changed"
        end)
        Build(function(Button, Handler)
            Button:GetPropertyChangedSignal("Text"):Connect(Handler)
        end)
        "#,
    );
    // (and the script that made them, which the connected handlers' environment references)
    assert_eq!(built, empty + 11 + 1, "only the tree with connected handlers is kept");
    assert_eq!(live(""), built);
}

/// `--mount Service/Name=<dir|file>` / `--instance Service/Name=<Class>` (project.rs mount_at, the
/// mounts cli/rojo.mjs makes of Rojo project files): directories with init scripts, plain folders,
/// single files, service-level merges and class nodes.
#[test]
fn explicit_mounts() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("rur-mount-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/Components")).unwrap();
    std::fs::create_dir_all(dir.join("shared")).unwrap();
    std::fs::write(dir.join("src/init.server.luau"), "-- entry").unwrap();
    std::fs::write(dir.join("src/Components/Label.luau"), "return 1").unwrap();
    std::fs::write(dir.join("shared/Util.luau"), "return 2").unwrap();
    std::fs::write(dir.join("single.luau"), "return 3").unwrap();
    let (lua, engine) = start();
    let path = |p: &str| dir.join(p).to_string_lossy().into_owned();
    for (spec, class) in [
        (format!("ReplicatedStorage/MyPlugin={}", path("src")), false),
        (format!("ReplicatedStorage={}", path("shared")), false),
        (format!("ServerStorage/Deep/Single={}", path("single.luau")), false),
        ("StarterGui/Hud=ScreenGui".to_string(), true),
    ] {
        mount_at(&engine, &Mount::parse(&spec, class).unwrap()).unwrap();
    }
    assert!(Mount::parse("NoValue", false).is_err());
    assert!(mount_at(&engine, &Mount::parse(&format!("NotAService/X={}", path("src")), false).unwrap()).is_err());
    let output = run(&lua, &engine, r#"
        local RS = game:GetService("ReplicatedStorage")
        assert(RS.MyPlugin.ClassName == "Script", RS.MyPlugin.ClassName)
        assert(RS.MyPlugin.Components.ClassName == "Folder")
        assert(require(RS.MyPlugin.Components.Label) == 1)
        assert(require(RS.Util) == 2)
        assert(require(game:GetService("ServerStorage").Deep.Single) == 3)
        assert(game:GetService("StarterGui").Hud.ClassName == "ScreenGui")
    "#);
    assert!(output.iter().all(|line| !line.starts_with("error")), "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A directory holding `default.project.json` mounts as that project, as Rojo syncs it (Wally's
/// `Packages/_Index/<package>/<name>/`): the tree's `$path` (a directory or a file), ahead of an `init`
/// script beside it, or a `$className` root with child nodes; named by the project's `name`, else by the
/// directory, and by the mount's own name when the directory is mounted itself.
#[test]
fn nested_default_projects_mount_as_their_tree() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-nested-project-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |relative: &str, text: &str| {
        let file = dir.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write("Packages/lib.lua", r#"return require(script.Parent._Index["scope_lib@1.0.0"]["lib"])"#);
    write("Packages/_Index/scope_lib@1.0.0/lib/default.project.json", r#"{ "name": "lib", "tree": { "$path": "src" } }"#);
    write("Packages/_Index/scope_lib@1.0.0/lib/src/init.luau", r#"return "lib""#);
    write("Packages/_Index/scope_lib@1.0.0/lib/src/Util.luau", "return 1");
    write("Packages/_Index/scope_lib@1.0.0/lib/build.project.json", r#"{ "tree": { "$path": "src" } }"#);
    write("Packages/_Index/scope_file@1.0.0/file/default.project.json", r#"{ "tree": { "$path": "src/file.luau" } }"#);
    write("Packages/_Index/scope_named@1.0.0/named/default.project.json", r#"{ "name": "Renamed", "tree": { "$path": "src" } }"#);
    write("Packages/_Index/scope_named@1.0.0/named/src/init.luau", r#"return "renamed""#);
    write("Packages/_Index/scope_file@1.0.0/file/src/file.luau", r#"return "file""#);
    write("Packages/_Index/scope_both@1.0.0/both/default.project.json", r#"{ "name": "both", "tree": { "$path": "src" } }"#);
    write("Packages/_Index/scope_both@1.0.0/both/init.luau", r#"return "root init""#);
    write("Packages/_Index/scope_both@1.0.0/both/src/init.luau", r#"return "src""#);
    write("Packages/_Index/scope_tree@1.0.0/tree/default.project.json", r#"{ "name": "tree", "tree": { "$className": "Folder", "A": { "$path": "a.luau" } } }"#);
    write("Packages/_Index/scope_tree@1.0.0/tree/a.luau", r#"return "a""#);
    let (lua, engine) = start();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Packages={}", dir.join("Packages").to_string_lossy()), false).unwrap()).unwrap();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Direct={}", dir.join("Packages/_Index/scope_named@1.0.0/named").to_string_lossy()), false).unwrap()).unwrap();
    let output = run(&lua, &engine, r#"
        local Packages = game:GetService("ReplicatedStorage").Packages
        local Index = Packages._Index
        local Lib = Index["scope_lib@1.0.0"].lib
        assert(Lib.ClassName == "ModuleScript", Lib.ClassName)
        assert(require(Packages.lib) == "lib")
        assert(require(Lib.Util) == 1)
        assert(Lib:FindFirstChild("src") == nil and Lib:FindFirstChild("default.project") == nil and Lib:FindFirstChild("build.project") == nil)
        local File = Index["scope_file@1.0.0"].file
        assert(File.ClassName == "ModuleScript" and require(File) == "file")
        local Named = Index["scope_named@1.0.0"]
        assert(Named:FindFirstChild("named") == nil and require(Named.Renamed) == "renamed", "the project's name")
        assert(require(game:GetService("ReplicatedStorage").Direct) == "renamed", "a mounted directory keeps the mount's name")
        assert(require(Index["scope_both@1.0.0"].both) == "src", "the project file wins over init.luau")
        local Tree = Index["scope_tree@1.0.0"].tree
        assert(Tree.ClassName == "Folder" and require(Tree.A) == "a")
        assert(Tree:FindFirstChild("a") == nil, "only the tree's nodes")
    "#);
    assert!(output.iter().all(|line| !line.starts_with("error")), "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Entries whose names start with a dot mount as `rojo build` builds this tree (7.6.1 and 7.7.0-rc.1, the same tree): folders,
/// scripts, JSON modules and a nested project's folders, which a package manager's link modules require through (pesde's
/// `Packages/.pesde`). A file named only by a suffix is named as Rojo names it (`.luau` a ModuleScript ".luau", `.server.luau` a
/// Script ""), and files no sync rule takes (`.gitignore`, `.DS_Store`, `.env`, `.meta.json`, a submodule's `.git` file) make
/// nothing. A `.git` directory, which Rojo builds as any folder, mounts as an empty Folder, reported.
#[test]
fn names_starting_with_a_dot_mount_as_rojo_builds_them() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-dot-names-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |relative: &str, text: &str| {
        let file = dir.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write("Packages/link.luau", r#"return require(script.Parent:FindFirstChild(".hidden"):FindFirstChild("value"))"#);
    write("Packages/.hidden/value.luau", r#"return "loaded""#);
    write("Packages/.dotfile.luau", r#"return "dotfile""#);
    write("Packages/.dotserver.server.luau", r#"print("server")"#);
    write("Packages/.dotclient.client.lua", r#"print("client")"#);
    write("Packages/.luau", r#"return "luau""#);
    write("Packages/.lua", r#"return "lua""#);
    write("Packages/.server.luau", r#"print("bare")"#);
    write("Packages/.config.json", r#"{ "a": 1 }"#);
    write("Packages/.meta.json", r#"{ "className": "Configuration" }"#);
    write("Packages/.gitignore", "node_modules");
    write("Packages/.DS_Store", "x");
    write("Packages/.env", "A=1");
    write("Packages/.vscode/settings.json", r#"{ "tab": 4 }"#);
    write("Packages/.pkg/init.luau", r#"return "pkg""#);
    write("Packages/.pkg/inner.luau", r#"return "inner""#);
    std::fs::create_dir_all(dir.join("Packages/.empty")).unwrap();
    write("Packages/.git/HEAD", "ref: refs/heads/main");
    write("Packages/.git/objects/ab/cdef", "blob");
    write("Packages/.git/x.luau", r#"return "in git""#);
    write("Packages/.git/hooks/y.json", r#"{ "b": 2 }"#);
    write("Packages/sub/.git", "gitdir: ../.git/modules/sub");
    write("Packages/sub/m.luau", r#"return "sub""#);
    write("Packages/.pesde/scope+lib/1.0.0/lib/default.project.json", r#"{ "name": "lib", "tree": { "$path": ".src" } }"#);
    write("Packages/.pesde/scope+lib/1.0.0/lib/.src/init.luau", r#"return "lib""#);
    write("Packages/pesde.luau", r#"return require(script.Parent[".pesde"]["scope+lib"]["1.0.0"].lib)"#);
    let root = dir.to_string_lossy().replace('\\', "/");
    let (lua, engine) = start();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Packages={root}/Packages"), false).unwrap()).unwrap();
    let output = run(&lua, &engine, r#"
        local Packages = game:GetService("ReplicatedStorage").Packages
        local function list(instance, depth)
            local children = instance:GetChildren()
            table.sort(children, function(a, b) return a.Name < b.Name end)
            for _, child in children do
                print("tree", string.rep("  ", depth) .. child.ClassName .. " " .. string.format("%q", child.Name))
                list(child, depth + 1)
            end
        end
        list(Packages, 0)
        print("loads", require(Packages.link), require(Packages.pesde), require(Packages[".pkg"]), require(Packages[".luau"]), require(Packages[".lua"]), require(Packages[".config"]).a)
    "#);
    // Rojo's tree, sorted by name, but for what its .git holds (Folder hooks > ModuleScript y, Folder objects > Folder ab, ModuleScript x)
    let expected = [
        r#"Script """#, r#"ModuleScript ".config""#, r#"LocalScript ".dotclient""#, r#"ModuleScript ".dotfile""#, r#"Script ".dotserver""#,
        r#"Folder ".empty""#, r#"Folder ".git""#, r#"Folder ".hidden""#, r#"  ModuleScript "value""#, r#"ModuleScript ".lua""#,
        r#"ModuleScript ".luau""#, r#"Folder ".pesde""#, r#"  Folder "scope+lib""#, r#"    Folder "1.0.0""#, r#"      ModuleScript "lib""#,
        r#"ModuleScript ".pkg""#, r#"  ModuleScript "inner""#, r#"Folder ".vscode""#, r#"  ModuleScript "settings""#, r#"ModuleScript "link""#,
        r#"ModuleScript "pesde""#, r#"Folder "sub""#, r#"  ModuleScript "m""#,
    ];
    let tree: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("print: tree ")).collect();
    assert_eq!(tree, expected, "{output:?}");
    assert!(output.contains(&"print: loads loaded lib pkg luau lua 1".to_string()), "{output:?}");
    let reported: Vec<&String> = output.iter().filter(|line| line.starts_with("warn") || line.starts_with("error")).collect();
    assert_eq!(reported, [&format!("warn: {root}/Packages/.git: a Git repository's own directory; mounted as an empty Folder (Rojo builds what is in it)")], "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `default.project.json` can come from a third-party package, so a `$path` stays inside the folder
/// holding it. One that is absolute or has a `..` segment is reported and not followed, on a tree's root
/// or a child node. A required `$path` that names nothing and a project file that can't be read are
/// reported too, and each such directory mounts as it is.
#[test]
fn nested_project_paths_stay_inside_the_project_folder() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-project-path-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |relative: &str, text: &str| {
        let file = dir.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write("Outside.json", r#"{ "outside": true }"#);
    write("Outside/Secret.luau", "return 1");
    let root = dir.to_string_lossy().replace('\\', "/");
    let absolute = format!("{root}/Outside.json");
    let refused = ["..", "../x", "../../Outside.json", "../../Outside", "src/../..", r"src\..\..\Outside.json", "/x", r"\x", "//server/share/x", r"\\server\share\x", "C:/x", r"C:\x", "C:x", absolute.as_str()];
    // Each under a mount of its own: a `..` that is followed would otherwise mount the other cases again at every level
    for (index, path) in refused.iter().enumerate() {
        write(&format!("Refused{index}/package/default.project.json"), &serde_json::json!({ "tree": { "$path": path } }).to_string());
        write(&format!("Refused{index}/package/src/Inside.luau"), "return 1");
        write(&format!("Refused{index}/package/x/init.luau"), "return 1");
    }
    write("Packages/child/default.project.json", r#"{ "tree": { "$className": "Folder", "Escape": { "$path": "../../Outside.json" }, "Kept": { "$path": "Kept.luau" } } }"#);
    write("Packages/child/Kept.luau", r#"return "kept""#);
    for (package, project) in [("garbled", "not json"), ("treeless", r#"{ "name": "treeless" }"#), ("missing", r#"{ "tree": { "$path": "gone" } }"#), ("optional", r#"{ "tree": { "$path": { "optional": "gone" } } }"#)] {
        write(&format!("Packages/{package}/default.project.json"), project);
        write(&format!("Packages/{package}/Inside.luau"), "return 1");
    }
    let (lua, engine) = start();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Packages={root}/Packages"), false).unwrap()).unwrap();
    for index in 0..refused.len() {
        mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Refused{index}={root}/Refused{index}"), false).unwrap()).unwrap();
    }
    let output = run(&lua, &engine, r#"
        local ReplicatedStorage = game:GetService("ReplicatedStorage")
        local Packages = ReplicatedStorage.Packages
        for _, case in ReplicatedStorage:GetChildren() do
            if case.Name:find("^Refused") then
                assert(case.package.ClassName == "Folder" and case.package:FindFirstChild("src"), case.Name)
            end
        end
        for _, descendant in ReplicatedStorage:GetDescendants() do
            assert(descendant.Name ~= "Outside" and descendant.Name ~= "Secret", descendant:GetFullName())
        end
        assert(Packages.child:FindFirstChild("Escape") == nil and require(Packages.child.Kept) == "kept", "a refused child node")
        for _, name in { "garbled", "treeless", "missing", "optional" } do
            assert(Packages[name].ClassName == "Folder" and Packages[name]:FindFirstChild("Inside"), name)
        end
        print("ok")
    "#);
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    let says = |package: &str, message: &str| format!("warn: {root}/{package}/default.project.json: {message}");
    let mut expected: Vec<String> = refused.iter().enumerate().map(|(index, path)| says(&format!("Refused{index}/package"), &format!("$path {path} leaves the project's folder; not mounted"))).collect();
    expected.push(says("Packages/child", "$path ../../Outside.json leaves the project's folder; not mounted"));
    expected.push(says("Packages/missing", "$path gone names nothing ree can mount; not mounted"));
    for package in ["Packages/garbled", "Packages/treeless"] {
        expected.push(says(package, "not a project ree can read (JSON with a \"tree\" object); the directory mounts as it is"));
    }
    let reported: Vec<&String> = output.iter().filter(|line| line.contains("default.project.json")).collect();
    for line in &expected {
        assert!(reported.contains(&line), "{line} in {reported:?}");
    }
    assert_eq!(reported.len(), expected.len(), "{reported:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project inside more than 32 others is left as an empty Folder and reported, the depth where
/// roblox-ui-renderer's cli/rojo.mjs stops, so a chain of projects ends there.
#[test]
fn nested_projects_stop_past_the_nesting_limit() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-project-nesting-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let chain = |name: &str, projects: usize| {
        let mut folder = dir.join("Packages").join(name);
        for _ in 0..projects {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("default.project.json"), r#"{ "tree": { "$path": "n" } }"#).unwrap();
            folder = folder.join("n");
        }
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("init.luau"), r#"return "end""#).unwrap();
    };
    chain("Fits", 33);
    chain("TooDeep", 34);
    let root = dir.to_string_lossy().replace('\\', "/");
    let (lua, engine) = start();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Packages={root}/Packages"), false).unwrap()).unwrap();
    let output = run(&lua, &engine, r#"
        local Packages = game:GetService("ReplicatedStorage").Packages
        assert(require(Packages.Fits) == "end", "a chain within the limit")
        assert(Packages.TooDeep.ClassName == "Folder" and #Packages.TooDeep:GetChildren() == 0, "a chain past the limit")
        print("ok")
    "#);
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    let expected = format!("warn: {root}/Packages/TooDeep/{}default.project.json: project nesting too deep (past 32 levels); not mounted", "n/".repeat(33));
    let stopped: Vec<&String> = output.iter().filter(|line| line.contains("too deep")).collect();
    assert_eq!(stopped, [&expected]);
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A tree's child nodes can each `$path` to the same project, so the mounts would double at every level
/// while the depth stays within its limit: one mount follows 10000 projects at most and reports the rest.
#[test]
fn a_mount_stops_following_projects_past_its_limit() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-project-fan-out-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut folder = dir.join("Doubling");
    for _ in 0..15 {
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("default.project.json"), r#"{ "tree": { "$className": "Folder", "A": { "$path": "n" }, "B": { "$path": "n" } } }"#).unwrap();
        folder = folder.join("n");
    }
    std::fs::create_dir_all(&folder).unwrap();
    let root = dir.to_string_lossy().replace('\\', "/");
    let (lua, engine) = start();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Doubling={root}/Doubling"), false).unwrap()).unwrap();
    let output = run(&lua, &engine, r#"
        -- two nodes under each of the 10000 projects followed
        local count = #game:GetService("ReplicatedStorage").Doubling:GetDescendants()
        assert(count == 20000, tostring(count))
        print("ok")
    "#);
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    let stopped = output.iter().filter(|line| line.starts_with("warn:") && line.ends_with("more than 10000 projects in one mount; not mounted")).count();
    assert!((1..=32).contains(&stopped), "{stopped} reported");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Rojo's JSON models (`*.model.json`, Rojo 7's format: rojo.space/docs/v7/sync-details/#json-models
/// and its json_model.rs): one instance named after the file (its own top-level name ignored) or by
/// its project node, lowercase or legacy capitalized keys, a child without a name taking its class's,
/// properties implicit (by the property's type) or explicit, attributes and Tags, kept as `rojo build`
/// writes them (Position does not serialize: the CFrame places the part). A file of white space makes
/// nothing; one Rojo would reject is reported and not mounted.
#[test]
fn rojo_json_models_mount_as_their_instances() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-json-model-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/init.luau"), "return script.Remote").unwrap();
    std::fs::write(dir.join("lib/Remote.model.json"), r#"{ "className": "RemoteEvent" }"#).unwrap();
    std::fs::write(
        dir.join("lib/Tool.model.json"),
        r#"{
            "Name": "Ignored",
            "ClassName": "Folder",
            "Children": [
                {
                    "Name": "RootPart",
                    "ClassName": "Part",
                    "Properties": {
                        "Size": [4, 4, 4], "Anchored": true, "Material": "Neon", "Color": [1, 0, 0],
                        "CFrame": [1, 2, 3, 1, 0, 0, 0, 1, 0, 0, 0, 1], "Position": [9, 9, 9]
                    }
                },
                { "ClassName": "RemoteFunction" }
            ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/Config.model.json"),
        r#"{
            "className": "Configuration",
            "properties": { "Tags": ["Hitbox", "Enemy"] },
            "attributes": { "Speed": 16, "Label": "fast", "On": true, "Offset": { "Vector3": [0, 5, 0] } },
            "children": [
                { "name": "Count", "className": "IntValue", "properties": { "Value": 5 } },
                { "name": "Title", "className": "StringValue", "properties": { "Value": { "String": "hello" } } }
            ]
        }"#,
    )
    .unwrap();
    std::fs::write(dir.join("lib/Empty.model.json"), " \n").unwrap();
    std::fs::write(dir.join("lib/Broken.model.json"), r#"{ "className": "Part", "properties": { "NotAProperty": 5 } }"#).unwrap();
    let (lua, engine) = start();
    let path = |p: &str| dir.join(p).to_string_lossy().into_owned();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Lib={}", path("lib")), false).unwrap()).unwrap();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Named={}", path("lib/Remote.model.json")), false).unwrap()).unwrap();
    let output = run(&lua, &engine, r#"
        local RS = game:GetService("ReplicatedStorage")
        local Lib = RS.Lib
        assert(Lib.ClassName == "ModuleScript" and Lib.Remote.ClassName == "RemoteEvent")
        assert(require(Lib) == Lib.Remote, "script.Remote")
        local Tool = Lib.Tool
        assert(Tool.ClassName == "Folder" and Tool.Name == "Tool", Tool.Name)
        local Root = Tool.RootPart
        assert(Root.ClassName == "Part" and Root.Size == Vector3.new(4, 4, 4) and Root.Anchored and Root.Material == Enum.Material.Neon)
        assert(Root.Color == Color3.new(1, 0, 0), tostring(Root.Color))
        assert(Root.Position == Vector3.new(1, 2, 3), tostring(Root.Position))
        assert(Tool.RemoteFunction.ClassName == "RemoteFunction", "a child without a name")
        local Config = Lib.Config
        assert(Config:HasTag("Hitbox") and Config:HasTag("Enemy"), "Tags")
        assert(Config:GetAttribute("Speed") == 16 and Config:GetAttribute("Label") == "fast" and Config:GetAttribute("On") == true)
        assert(Config:GetAttribute("Offset") == Vector3.new(0, 5, 0))
        assert(Config.Count.Value == 5 and Config.Title.Value == "hello")
        assert(Lib:FindFirstChild("Empty") == nil and Lib:FindFirstChild("Broken") == nil)
        assert(RS.Named.ClassName == "RemoteEvent" and RS:FindFirstChild("Remote") == nil, "named by its project node")
    "#);
    assert!(output.iter().all(|line| !line.starts_with("error")), "{output:?}");
    let reported: Vec<&String> = output.iter().filter(|line| line.contains("Broken.model.json")).collect();
    assert!(reported.len() == 1 && reported[0].contains("unknown property Part.NotAProperty"), "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Model files hold properties and enum items the engine's reflection (data/reflection.json, a newer Studio) has and
/// rbx-dom's database does not: an InputBinding's Type and PrimaryModifier, KeyCode.MouseWheel, a SurfaceAppearance's
/// ResampleMode. A Rojo JSON model gives them implicitly or explicitly (`rojo build` 7.7 takes both), and binary and XML
/// models keep them; each reads back as the engine has it (rbx-dom's older database dropped the values).
#[test]
fn models_keep_what_the_engines_reflection_has_and_rbx_dom_lacks() {
    use super::project::{Mount, mount_at};
    use rbx_dom_weak::{InstanceBuilder, WeakDom, types::Enum};
    let dir = std::env::temp_dir().join(format!("ree-newer-model-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/Input.model.json"),
        r#"{
            "className": "InputContext",
            "children": [
                {
                    "name": "Zoom", "className": "InputAction", "properties": { "Type": "Direction1D" },
                    "children": [
                        { "name": "Wheel", "className": "InputBinding", "properties": { "KeyCode": "MouseWheel", "Type": "Scriptable", "PrimaryModifier": "LeftShift" } }
                    ]
                }
            ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/Explicit.model.json"),
        r#"{ "className": "InputBinding", "properties": { "KeyCode": { "Enum": 1035 }, "PrimaryModifier": { "Enum": 304 }, "Type": { "Enum": 1 } } }"#,
    )
    .unwrap();
    std::fs::write(dir.join("lib/Look.model.json"), r#"{ "className": "SurfaceAppearance", "properties": { "ResampleMode": "Pixelated" } }"#).unwrap();
    std::fs::write(
        dir.join("lib/Xml.rbxmx"),
        r#"<roblox version="4">
	<Item class="InputBinding" referent="RBX0">
		<Properties>
			<string name="Name">Xml</string>
			<token name="KeyCode">1035</token>
			<token name="PrimaryModifier">304</token>
			<token name="Type">1</token>
		</Properties>
	</Item>
</roblox>"#,
    )
    .unwrap();
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let binding = InstanceBuilder::new("InputBinding")
        .with_name("Binary")
        .with_property("KeyCode", Enum::from_u32(1035))
        .with_property("PrimaryModifier", Enum::from_u32(304))
        .with_property("Type", Enum::from_u32(1));
    let look = InstanceBuilder::new("SurfaceAppearance").with_name("BinaryLook").with_property("ResampleMode", Enum::from_u32(1));
    let top = dom.insert(dom.root_ref(), InstanceBuilder::new("Folder").with_name("Binary").with_children([binding, look]));
    let mut file = Vec::new();
    rbx_binary::to_writer(&mut file, &dom, &[top]).unwrap();
    std::fs::write(dir.join("lib/Binary.rbxm"), file).unwrap();
    let (lua, engine) = start();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Lib={}", dir.join("lib").to_string_lossy()), false).unwrap()).unwrap();
    let output = run(&lua, &engine, r#"
        local Lib = game:GetService("ReplicatedStorage").Lib
        local function check(Binding)
            assert(Binding.KeyCode == Enum.KeyCode.MouseWheel, Binding.Name .. " KeyCode " .. tostring(Binding.KeyCode))
            assert(Binding.PrimaryModifier == Enum.KeyCode.LeftShift, Binding.Name .. " PrimaryModifier " .. tostring(Binding.PrimaryModifier))
            assert(Binding.Type == Enum.InputBindingType.Scriptable, Binding.Name .. " Type " .. tostring(Binding.Type))
        end
        assert(Lib.Input.ClassName == "InputContext" and Lib.Input.Zoom.Type == Enum.InputActionType.Direction1D, "the action's Type")
        check(Lib.Input.Zoom.Wheel)
        check(Lib.Explicit)
        check(Lib.Xml)
        check(Lib.Binary.Binary)
        assert(Lib.Look.ResampleMode == Enum.ResamplerMode.Pixelated, "ResampleMode " .. tostring(Lib.Look.ResampleMode))
        assert(Lib.Binary.BinaryLook.ResampleMode == Enum.ResamplerMode.Pixelated, "binary ResampleMode")
        print("ok")
    "#);
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    assert!(output.iter().all(|line| !line.starts_with("warn") && !line.starts_with("error")), "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A model file whose Folder `name` holds the ModuleScript `Module`, whose Source returns a long string (long enough
/// for an archive writer to keep it outside the scene), and that string.
fn archived_module(name: &str) -> (Vec<u8>, String) {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let text = "abcdefghij".repeat(40);
    let module = InstanceBuilder::new("ModuleScript").with_name("Module").with_property("Source", format!("return \"{text}\"\n"));
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let top = dom.insert(dom.root_ref(), InstanceBuilder::new("Folder").with_name(name).with_child(module));
    let mut file = Vec::new();
    rbx_binary::to_writer(&mut file, &dom, &[top]).unwrap();
    (file, text)
}

/// `bytes` with the CRC-32 of the ZIP member `name` written again in its local header and its central directory entry
/// (after a test changed the member's bytes without moving them).
fn rezip_crc(mut bytes: Vec<u8>, name: &str) -> Vec<u8> {
    let crc32 = |data: &[u8]| {
        let mut crc = !0u32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    };
    let u16_at = |bytes: &[u8], at: usize| usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
    let u32_at = |bytes: &[u8], at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let (mut local, mut central) = (None, None);
    for at in 0..bytes.len().saturating_sub(46) {
        if bytes[at..].starts_with(b"PK\x03\x04") && bytes[at + 30..].starts_with(name.as_bytes()) && u16_at(&bytes, at + 26) == name.len() {
            local.get_or_insert(at);
        }
        if bytes[at..].starts_with(b"PK\x01\x02") && bytes[at + 46..].starts_with(name.as_bytes()) && u16_at(&bytes, at + 28) == name.len() {
            central.get_or_insert(at);
        }
    }
    let (local, central) = (local.expect("local header"), central.expect("central directory entry"));
    let start = local + 30 + u16_at(&bytes, local + 26) + u16_at(&bytes, local + 28);
    let crc = crc32(&bytes[start..start + u32_at(&bytes, local + 22)]).to_le_bytes();
    bytes[local + 14..local + 18].copy_from_slice(&crc);
    bytes[central + 16..central + 20].copy_from_slice(&crc);
    bytes
}

/// A Roblox archive (.rbx) mounts as the model it holds, with the values its writer kept outside the scene put back: a
/// script's Source, which rbx_binary reads as empty from the stored scene alone. It mounts by `--mount` and by
/// project::load_model under its own extension, and as a model file in a mounted directory when it is named like one.
#[test]
fn archives_mount_with_their_scripts_sources() {
    use super::project::{Mount, load_model, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-archive-mount-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let (model, text) = archived_module("Lib");
    let archive = ror_archive::Builder::model(model).externalize(256).build().unwrap();
    // the stored scene alone: the source is kept outside it, and the parser drops the property without an error
    let stored = rbx_binary::from_reader(ror_archive::Archive::open(&archive).unwrap().root().unwrap().bytes).unwrap();
    let module = stored.descendants().find(|instance| instance.name == "Module").expect("Module");
    assert!(!matches!(module.properties.get(&"Source".into()), Some(rbx_dom_weak::types::Variant::String(source)) if !source.is_empty()), "the stored scene holds the source");
    std::fs::write(dir.join("Lib.rbx"), &archive).unwrap();
    std::fs::write(dir.join("lib/Inner.rbxm"), &archive).unwrap();
    let (lua, engine) = start();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Archived={}", dir.join("Lib.rbx").to_string_lossy()), false).unwrap()).unwrap();
    mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/Dir={}", dir.join("lib").to_string_lossy()), false).unwrap()).unwrap();
    let loaded = load_model(&engine, &dir.join("Lib.rbx")).unwrap();
    assert_eq!(loaded.len(), 1);
    let output = run(&lua, &engine, r#"
        local Storage = game:GetService("ReplicatedStorage")
        print(require(Storage.Archived.Module), require(Storage.Dir.Lib.Module))
    "#);
    assert_eq!(output, [format!("print: {text} {text}")]);
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An archive that does not read (here a member that fails its CRC-32) is a warning that names the file and says why,
/// under its own extension and under a model file's, where a model file that does not decode was left out without a word;
/// and what the archive's reader read past is a warning that names the file, the model mounting all the same.
#[test]
fn archives_that_do_not_read_or_note_something_say_so() {
    use super::project::{Mount, mount_at};
    let dir = std::env::temp_dir().join(format!("ree-archive-warn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let (model, text) = archived_module("Lib");
    let archive = ror_archive::Builder::model(model).externalize(256).build().unwrap();
    let mut damaged = archive.clone();
    let at = damaged.windows(text.len()).position(|window| window == text.as_bytes()).expect("the source's object");
    damaged[at] ^= 1;
    std::fs::write(dir.join("Broken.rbx"), &damaged).unwrap();
    std::fs::write(dir.join("lib/Broken.rbxm"), &damaged).unwrap();
    // a digest algorithm the reader does not know: its objects are used unchecked, and it says so
    let unknown = archive.windows(10).position(|window| window == b"blake3-128").expect("the header");
    let mut noted = archive.clone();
    noted[unknown..unknown + 10].copy_from_slice(b"blake3-12x");
    std::fs::write(dir.join("Noted.rbx"), rezip_crc(noted, "manifest.jsonl")).unwrap();
    let (lua, engine) = start();
    for (at, file) in [("Broken", dir.join("Broken.rbx")), ("Dir", dir.join("lib")), ("Noted", dir.join("Noted.rbx"))] {
        mount_at(&engine, &Mount::parse(&format!("ReplicatedStorage/{at}={}", file.to_string_lossy()), false).unwrap()).unwrap();
    }
    let output = run(&lua, &engine, r#"
        local Storage = game:GetService("ReplicatedStorage")
        print(Storage:FindFirstChild("Broken"), #Storage.Dir:GetChildren(), require(Storage.Noted.Module) ~= nil)
    "#);
    let warned = |file: &str, says: &str| output.iter().any(|line| line.starts_with("warn: ") && line.contains(file) && line.contains(says));
    assert!(warned("Broken.rbx", "Roblox archive: invalid .rbx container"), "{output:?}");
    assert!(warned("Broken.rbxm", "Roblox archive: invalid .rbx container"), "{output:?}");
    assert!(warned("Noted.rbx", "Roblox archive: ") && warned("Noted.rbx", "blake3-12x"), "{output:?}");
    assert_eq!(output.last().map(String::as_str), Some("print: nil 0 true"), "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Everything the engine holds per instance, signal and connection, and the Luau heap.
#[derive(Debug, PartialEq, Clone, Copy)]
struct Held {
    instances: usize,
    userdata: usize,
    values: usize,
    signals: usize,
    connections: usize,
}

fn held(lua: &Lua, engine: &Rc<Engine>) -> (Held, usize) {
    scheduler::flush_deferred(lua, engine).unwrap();
    for _ in 0..3 {
        lua.gc_collect().unwrap();
        engine_lua::raw::collect(lua, engine).unwrap();
    }
    let state = engine.state.borrow();
    let (signals, connections) = state.signals.counts();
    let held = Held {
        instances: state.dom.live_count(),
        userdata: engine_lua::raw::stats(engine).0,
        values: engine_lua::raw::value_cache_len(engine),
        signals,
        connections,
    };
    (held, lua.used_memory())
}

/// A Content made by Content.fromObject keeps its object alive, as Roblox's does (measured in a 0.747 client: an
/// EditableImage only a Content holds is still the Content's object, and an image's content, after the collector ran):
/// an EditableImage only the Content holds survives the instance collector, can be shown, and is freed once the Content
/// is gone. Before, the collector freed it and the assignment raised an engine error (stale instance id).
#[test]
fn a_content_keeps_its_object_alive() {
    let (lua, engine) = start();
    run(&lua, &engine, r#"
        shared.Kept = Content.fromObject(game:GetService("AssetService"):CreateEditableImage({ Size = Vector2.new(4, 4) }))
    "#);
    let (before, _) = held(&lua, &engine);
    let output = run(&lua, &engine, r#"
        local Kept = shared.Kept
        local Label = Instance.new("ImageLabel")
        Label.ImageContent = Kept
        print("kept", typeof(Kept.Object), Kept.Object.ClassName, Kept.SourceType)
        shared.Kept = nil
        Label:Destroy()
    "#);
    assert!(output.iter().any(|line| line == "print: kept Instance EditableImage Enum.ContentSourceType.Object"), "{output:?}");
    assert!(output.iter().all(|line| !line.starts_with("error")), "{output:?}");
    let (after, _) = held(&lua, &engine);
    assert!(after.instances < before.instances, "the image is freed once its Content is gone: {before:?} -> {after:?}");
}

/// A UI in the DataModel, every button's handlers capturing the whole tree (the cycle Roblox never
/// collects), tagged and with attributes: built `count` times by `source`. (roblox-ui-renderer's
/// copy of this test also lays it out: its layout records are freed too.)
const BUILD_UI: &str = r#"
    local Gui = Instance.new("ScreenGui")
    Gui.Name = "LifetimeGui"
    local Root = Instance.new("Frame")
    Root.Name = "Root"
    Root.Size = UDim2.fromOffset(400, 300)
    Root.Parent = Gui
    local Layout = Instance.new("UIListLayout")
    Layout.Parent = Root
    local State = { Root = Root, Buttons = {} }
    for Index = 1, 40 do
        local Button = Instance.new("TextButton")
        Button.Name = "Button" .. Index
        Button.Size = UDim2.new(1, 0, 0, 20)
        Button.Text = "Row " .. Index
        Button:AddTag("LifetimeRow")
        Button:SetAttribute("Index", Index)
        Instance.new("UICorner").Parent = Button
        local Stroke = Instance.new("UIStroke")
        Stroke.Parent = Button
        Button.Parent = Root
        State.Buttons[Index] = Button
        Button.Activated:Connect(function()
            State.Root.Visible = not State.Root.Visible
        end)
        Button:GetPropertyChangedSignal("Text"):Connect(function()
            Stroke.Thickness = #Button.Text
        end)
        Button.AncestryChanged:Connect(function()
            return Button, Root
        end)
        Button:GetAttributeChangedSignal("Index"):Connect(function()
            return Gui
        end)
        Button.MouseEnter:Once(function()
            return State
        end)
    end
    Root.Changed:Connect(function()
        return State
    end)
    Gui.Parent = game:GetService("StarterGui")
    -- (datatype values read: the converted-value cache)
    assert(Root.Size == UDim2.fromOffset(400, 300) and State.Buttons[3].Size == UDim2.new(1, 0, 0, 20))
    State.Buttons[1].Text = "changed"
"#;

/// Destroy disconnects every connection of the instance and its descendants, and a destroyed
/// tree nothing references is freed whole: the instances, their userdata and converted values,
/// their signals and connections with the handlers, and the Luau heap. Built
/// and destroyed again and again, nothing accumulates (slots are reused).
#[test]
fn a_destroyed_tree_is_freed_whole() {
    let (lua, engine) = start();
    let cycle = |destroy: &str| {
        let output = run(&lua, &engine, &format!("{BUILD_UI}\n{destroy}"));
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
    };
    let destroy = "Gui:Destroy()";
    // (a first cycle makes what is made once: member dispatch entries, signal userdata of services)
    cycle(destroy);
    let (baseline, baseline_heap) = held(&lua, &engine);
    let slots = engine.state.borrow().dom.slot_count();
    let capacity = engine.state.borrow().signals.capacity();
    for round in 0..25 {
        cycle(destroy);
        let (now, heap) = held(&lua, &engine);
        assert_eq!(now, baseline, "round {round}");
        assert!(heap < baseline_heap + 64 * 1024, "round {round}: Luau heap {heap} from {baseline_heap}");
    }
    assert_eq!(engine.state.borrow().dom.slot_count(), slots, "instance slots are reused");
    assert_eq!(engine.state.borrow().signals.capacity(), capacity, "signal and connection slots are reused");

    // Removed and not destroyed, the tree stays: its handlers are connected and capture it (what
    // Roblox does; lua/leaks.rs reports it). 1 ScreenGui, Root, 1 layout, 40 buttons with 2 children;
    // 5 connections a button and Root.Changed's, and the script the handlers' environment references
    cycle("Gui.Parent = nil");
    let (now, _) = held(&lua, &engine);
    assert_eq!(now.instances, baseline.instances + 3 + 40 * 3 + 1);
    assert_eq!(now.connections, baseline.connections + 40 * 5 + 1);
    let report = engine_lua::leaks::report(&lua, &engine).unwrap();
    assert_eq!(report["outsideDataModel"]["heldByConnectedHandlers"], 123, "{report:#}");
    let handlers = report["handlers"].as_array().unwrap();
    assert_eq!(handlers.len(), 6, "{report:#}");
    assert!(handlers.iter().all(|handler| handler["trees"] == 1 && handler["handler"].as_str().unwrap().starts_with("Test:")), "{report:#}");
    let activated = handlers.iter().find(|handler| handler["signal"] == "Activated").unwrap();
    assert_eq!((&activated["connections"], &activated["class"]), (&serde_json::json!(40), &serde_json::json!("TextButton")));
    assert!(activated["captures"].as_str().unwrap().contains("field"), "{activated:#}");
}

/// What Studio 0.740 does with instances outside the DataModel that nothing references but their
/// own connected handlers (measured: weak tables and garbage churn for 30 s): kept when a handler
/// captures the instance, another instance of its tree, or an instance whose handler captures it
/// back; collected when no handler captures them, after Disconnect and after Destroy (of the
/// instance or of an ancestor). The engine does the same.
#[test]
fn connected_handlers_keep_their_own_trees_as_in_studio() {
    let (lua, engine) = start();
    let live = |script: &str| {
        let output = run(&lua, &engine, script);
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
        held(&lua, &engine).0.instances
    };
    let empty = live("");
    // A: a handler capturing its own instance (1 instance kept)
    let a = live(r#"local F = Instance.new("Frame") F:GetPropertyChangedSignal("Name"):Connect(function() return F end)"#);
    assert_eq!(a, empty + 1 + 1, "A (and the script its handler's environment references)");
    // B: the same, destroyed; C: disconnected; D: a handler capturing nothing of the tree
    let b = live(r#"local F = Instance.new("Frame") F:GetPropertyChangedSignal("Name"):Connect(function() return F end) F:Destroy()"#);
    assert_eq!(b, a, "B: destroyed");
    let c = live(r#"local F = Instance.new("Frame") F:GetPropertyChangedSignal("Name"):Connect(function() return F end):Disconnect()"#);
    assert_eq!(c, a, "C: disconnected");
    let d = live(r#"local F = Instance.new("Frame") local S = {} F:GetPropertyChangedSignal("Name"):Connect(function() return S end)"#);
    assert_eq!(d, a, "D: no capture");
    // E: a child's handler captures the root (2 kept); F: a child's handler captures the child
    let e = live(r#"local R = Instance.new("Frame") local C = Instance.new("TextButton") C.Parent = R C.Activated:Connect(function() return R end)"#);
    assert_eq!(e, a + 2 + 1, "E: child captures root");
    let f = live(r#"local R = Instance.new("Frame") local C = Instance.new("TextButton") C.Parent = R C.MouseEnter:Connect(function() return C end)"#);
    assert_eq!(f, e + 2 + 1, "F: child captures itself");
    // G: two parentless instances whose handlers capture each other
    let g = live(r#"local A, B = Instance.new("Frame"), Instance.new("Frame") A.Changed:Connect(function() return B end) B.Changed:Connect(function() return A end)"#);
    assert_eq!(g, f + 2 + 1, "G: cross");
    // I: the root destroyed: its descendants' connections go with it
    let i = live(r#"local R = Instance.new("Frame") local C = Instance.new("TextButton") C.Parent = R C.Activated:Connect(function() return R end) R:Destroy()"#);
    assert_eq!(i, g, "I: root destroyed");
}

/// Connection and signal slots are reused; what a script or a queued handler job still holds of a
/// reused slot is a disconnected connection / a signal nothing fires, never the slot's new owner.
#[test]
fn reused_signal_slots_do_not_revive_stale_handles() {
    let (lua, engine) = start();
    // (each script's deferred handlers have run when `run` returns)
    let script = |source: &str| {
        let output = run(&lua, &engine, source);
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
    };
    script(
        r#"
        local Event = Instance.new("BindableEvent")
        local Log = {}
        _G.Event, _G.Log = Event, Log
        local First = Event.Event:Connect(function() table.insert(Log, "first") end)
        First:Disconnect()
        local Second = Event.Event:Connect(function() table.insert(Log, "second") end)
        _G.Second = Second
        assert(not First.Connected and Second.Connected)
        First:Disconnect()
        assert(Second.Connected, "a stale connection disconnected the slot's new connection")
        Event:Fire()
        "#,
    );
    // A handler queued by a fire, disconnected before it runs, its slot taken by a new connection
    script(
        r#"
        local Event, Log = _G.Event, _G.Log
        assert(table.concat(Log, ",") == "second", table.concat(Log, ","))
        table.clear(Log)
        _G.Second:Disconnect()
        local Queued = Event.Event:Connect(function() table.insert(Log, "queued") end)
        Event:Fire()
        Queued:Disconnect()
        _G.Later = Event.Event:Connect(function() table.insert(Log, "later") end)
        "#,
    );
    script(r#"assert(#_G.Log == 0, table.concat(_G.Log, ",")) _G.Event:Fire()"#);
    script(
        r#"
        assert(table.concat(_G.Log, ",") == "later", table.concat(_G.Log, ","))
        _G.Later:Disconnect()
        _G.Stale = Instance.new("Frame").Changed
        "#,
    );
    // The Frame is freed (only its signal is held): the signal's slot goes to the next signal made
    held(&lua, &engine);
    script(
        r#"
        _G.Fired = 0
        local Frame = Instance.new("Frame")
        _G.Frame = Frame
        local Live = Frame.Changed:Connect(function() _G.Fired += 1 end)
        local Dead = _G.Stale:Connect(function() _G.Fired += 100 end)
        assert(Live.Connected and not Dead.Connected)
        assert(tostring(_G.Stale) == "Signal" and tostring(Frame.Changed) == "Signal Changed")
        Frame.Name = "Renamed"
        "#,
    );
    script("assert(_G.Fired == 1, _G.Fired)");
}

/// CollectionService as measured in Studio 0.740 (the same script run there; services/mod.rs
/// CollectionState): a tag counts while its instance is in the DataModel.
#[test]
fn collection_service_follows_the_data_model() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local CS = game:GetService("CollectionService")
        local Log = {}
        local function On(Signal, Label)
            Signal:Connect(function(Target) table.insert(Log, Label .. " " .. Target.Name) end)
        end
        On(CS:GetInstanceAddedSignal("A"), "+A")
        On(CS:GetInstanceRemovedSignal("A"), "-A")
        On(CS:GetInstanceAddedSignal("B"), "+B")
        On(CS:GetInstanceRemovedSignal("B"), "-B")
        local Host = Instance.new("Folder")
        Host.Parent = workspace
        local Host2 = Instance.new("Folder")
        Host2.Parent = workspace
        local function Tagged(Tag)
            local Names = {}
            for _, Target in CS:GetTagged(Tag) do table.insert(Names, Target.Name) end
            table.sort(Names)
            return table.concat(Names, ",")
        end
        -- (the test runs a step's action, lets its deferred handlers run, then checks it)
        local Steps = {}
        _G.Steps, _G.Checked = Steps, 0
        local function Step(Expected, ExpectedA, ExpectedB, Action)
            table.insert(Steps, { Expected = Expected, A = ExpectedA, B = ExpectedB, Action = Action })
        end
        function _G.Run(Index)
            table.clear(Log)
            Steps[Index].Action()
        end
        function _G.Check(Index)
            local Step = Steps[Index]
            local Got = table.concat(Log, " ")
            assert(Got == Step.Expected, `step {Index}: expected [{Step.Expected}], got [{Got}]`)
            assert(Tagged("A") == Step.A, `step {Index}: GetTagged A: expected [{Step.A}], got [{Tagged("A")}]`)
            assert(Tagged("B") == Step.B, `step {Index}: GetTagged B: expected [{Step.B}], got [{Tagged("B")}]`)
            _G.Checked += 1
        end

        local a = Instance.new("Frame")
        a.Name = "a"
        Step("", "", "", function() a:AddTag("A") end)
        Step("+A a", "a", "", function() a.Parent = Host end)
        Step("", "a", "", function() a.Parent = Host2 end)
        Step("-A a", "", "", function() a.Parent = nil end)
        Step("", "", "", function() a:RemoveTag("A") end)
        Step("+A a", "a", "", function() a:AddTag("A") a.Parent = Host end)
        Step("-A a", "", "", function() a:RemoveTag("A") end)
        Step("+A a", "a", "", function() a:AddTag("A") end)
        Step("", "a", "", function() a:AddTag("A") end)
        Step("-A a", "", "", function() a:Destroy() end)
        Step("", "", "", function() assert(a:HasTag("A") and #a:GetTags() == 1, "Destroy keeps the tags") end)

        -- A tagged subtree: root[A,B], c1[B,A] > g1[A], c2[A]
        local root = Instance.new("Frame") root.Name = "root"
        local c1 = Instance.new("Frame") c1.Name = "c1" c1.Parent = root
        local c2 = Instance.new("Frame") c2.Name = "c2" c2.Parent = root
        local g1 = Instance.new("Frame") g1.Name = "g1" g1.Parent = c1
        g1:AddTag("A") c2:AddTag("A") root:AddTag("A") c1:AddTag("B") c1:AddTag("A") root:AddTag("B")
        Step("+A root +B root +B c1 +A c1 +A g1 +A c2", "c1,c2,g1,root", "c1,root", function() root.Parent = Host end)
        Step("-A root -B root -B c1 -A c1 -A g1 -A c2", "", "", function() root.Parent = nil end)
        Step("+A root +B root +B c1 +A c1 +A g1 +A c2", "c1,c2,g1,root", "c1,root", function() root.Parent = Host end)
        Step("-B c1 -A c1 -A g1", "c2,root", "root", function() c1.Parent = nil end)
        Step("-A root -B root -A c2", "", "", function() root:Destroy() end)
        Step("", "", "", function() c1:Destroy() end)

        -- Entering through a parent, ClearAllChildren, and in and out within one step
        local outside = Instance.new("Folder") outside.Name = "outside"
        local x = Instance.new("Frame") x.Name = "x" x:AddTag("A")
        Step("", "", "", function() x.Parent = outside end)
        Step("+A x", "x", "", function() outside.Parent = Host end)
        Step("-A x", "", "", function() Host:ClearAllChildren() end)
        local y = Instance.new("Frame") y.Name = "y" y:AddTag("A")
        Step("+A y -A y", "", "", function() y.Parent = Host y.Parent = nil end)
        -- A clone keeps the tags and enters like any tagged instance
        local z = y:Clone() z.Name = "z"
        Step("+A z", "z", "", function() z.Parent = Host end)
        "#,
    );
    assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
    for step in 1..=22 {
        run(&lua, &engine, &format!("_G.Run({step})"));
        let output = run(&lua, &engine, &format!("_G.Check({step})"));
        assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
    }
    let output = run(&lua, &engine, "assert(_G.Checked == #_G.Steps and #_G.Steps == 22, #_G.Steps)");
    assert!(output.iter().all(|l| !l.starts_with("error")), "{output:?}");
}

/// Vector3 is the native vector type, as in Roblox (the values below are Studio 0.740's for the
/// same expressions): Vide's spring keeps a Vector3 source's value as a vector.
#[test]
fn vector3_is_the_native_vector() {
    assert_script(
        r#"
        local function Same(Value, Expected)
            assert(tostring(Value) == Expected, `{Value} ~= {Expected}`)
        end
        local a, b = Vector3.new(1, 2, 3), Vector3.new(4, -5, 6.5)
        assert(type(a) == "vector" and typeof(a) == "Vector3" and typeof(vector.create(1, 2, 3)) == "Vector3")
        assert(type(Vector3.zero) == "vector" and Vector3.new() == vector.zero and Vector3.one == vector.one)
        assert(a.X == 1 and a.y == 2 and a.Z == 3 and a == vector.create(1, 2, 3))
        Same(b.Unit, "0.4383973181247711, -0.5479966402053833, 0.7123956680297852")
        assert(b.Unit == vector.normalize(b) and a.Magnitude == vector.magnitude(a))
        assert(string.format("%.5f", a.Magnitude) == "3.74166")
        Same(a + b, "5, -3, 9.5") Same(a - b, "-3, 7, -3.5") Same(a * b, "4, -10, 19.5")
        Same(a / b, "0.25, -0.4000000059604645, 0.4615384638309479")
        Same(a * 2, "2, 4, 6") Same(2 * a, "2, 4, 6") Same(a / 2, "0.5, 1, 1.5") Same(-a, "-1, -2, -3") Same(a // 2, "0, 1, 1")
        assert(a:Dot(b) == 13.5)
        Same(a:Cross(b), "28, 5.5, -13") Same(a:Lerp(b, 0.25), "1.75, 0.25, 3.875")
        assert(a:FuzzyEq(Vector3.new(1, 2, 3.000001)) and not a:FuzzyEq(b))
        Same(b:Abs(), "4, 5, 6.5") Same(b:Ceil(), "4, -5, 7") Same(b:Floor(), "4, -5, 6") Same(b:Sign(), "1, -1, 1")
        Same(a:Max(b), "4, 2, 6.5") Same(a:Min(b), "1, -5, 3") Same(Vector3.max(a, b), "4, 2, 6.5") Same(Vector3.min(a, b), "1, -5, 3")
        assert(string.format("%.5f", a:Angle(b)) == "1.16425")
        assert(({ [a] = "found" })[Vector3.new(1, 2, 3)] == "found", "vectors are table keys by value")
        Same(Vector3.new(0.1, 1 / 3, 1e10), "0.10000000149011612, 0.3333333432674408, 10000000000")
        Same(Vector3.FromNormalId(Enum.NormalId.Top), "0, 1, 0") Same(Vector3.FromAxis(Enum.Axis.Z), "0, 0, 1")
        assert(getmetatable(a) == "The metatable is locked")
        assert(not pcall(function() return a.Nope end))

        -- Through Lune's CFrame / Ray / Region3 and Instance properties
        local cf = CFrame.new(1, 2, 3) * CFrame.Angles(0, math.pi / 2, 0)
        assert(typeof(cf.Position) == "Vector3" and type(cf.Position) == "vector" and cf.Position == a)
        Same(cf * a, "4, 4, 1.9999998807907104") Same(cf:PointToWorldSpace(a), "4, 4, 1.9999998807907104")
        Same(cf:VectorToWorldSpace(a), "3, 2, -1.0000001192092896") Same((cf + a).Position, "2, 4, 6") Same((cf - a).Position, "0, 0, 0")
        assert(CFrame.new(a).Position == a and CFrame.new(a, b).Position == a)
        local ray = Ray.new(a, b)
        assert(ray.Origin == a and ray.Direction == b and type(ray.Unit.Direction) == "vector")
        assert(string.format("%.5f", ray:Distance(Vector3.zero)) == "3.74166")
        Same(Region3.new(a, a + Vector3.one * 4).Size, "4, 4, 4")
        local part = Instance.new("Part")
        part.Size = Vector3.new(2, 4, 6)
        part.Position = vector.create(1, 2, 3)
        assert(type(part.Size) == "vector" and part.Size == Vector3.new(2, 4, 6) and part.Position == a)
        part:SetAttribute("V", Vector3.new(7, 8, 9))
        assert(part:GetAttribute("V") == Vector3.new(7, 8, 9) and typeof(part:GetAttribute("V")) == "Vector3")
        "#,
    );
}

#[test]
fn scale_to_scales_effect_geometry() {
    assert_script(
        r#"
        local Model = Instance.new("Model")
        local Part = Instance.new("Part")
        Part.Parent = Model
        Model.PrimaryPart = Part
        local Emitter = Instance.new("ParticleEmitter")
        Emitter.Size = NumberSequence.new({NumberSequenceKeypoint.new(0, 2, 0.5), NumberSequenceKeypoint.new(1, 4)})
        Emitter.Speed = NumberRange.new(10, 20)
        Emitter.Acceleration = Vector3.new(0, -8, 0)
        Emitter.ZOffset = 1
        Emitter.Parent = Part
        local Beam = Instance.new("Beam")
        Beam.Width0, Beam.Width1, Beam.CurveSize0, Beam.ZOffset = 2, 3, 4, 1
        Beam.Parent = Part
        local Light = Instance.new("PointLight")
        Light.Range = 20
        Light.Parent = Part
        local Stretched, Wrapped = Instance.new("Trail"), Instance.new("Trail")
        Stretched.TextureLength, Wrapped.TextureLength = 4, 4
        Wrapped.TextureMode = Enum.TextureMode.Wrap
        Stretched.Parent, Wrapped.Parent = Part, Part
        local Texture = Instance.new("Texture")
        Texture.StudsPerTileU, Texture.StudsPerTileV = 2, 6
        Texture.Parent = Part
        local Surface = Instance.new("SurfaceGui")
        Surface.SizingMode = Enum.SurfaceGuiSizingMode.PixelsPerStud
        Surface.PixelsPerStud = 50
        Surface.Parent = Part
        local Billboard = Instance.new("BillboardGui")
        Billboard.Size = UDim2.new(4, 10, 2, 20)
        Billboard.StudsOffset = Vector3.new(0, 6, 0)
        Billboard.Parent = Part
        Model:ScaleTo(0.5)
        local Points = Emitter.Size.Keypoints
        assert(Points[1].Value == 1 and Points[1].Envelope == 0.25 and Points[2].Value == 2, "emitter size")
        assert(Emitter.Speed.Min == 5 and Emitter.Speed.Max == 10, "emitter speed")
        assert(Emitter.Acceleration == Vector3.new(0, -4, 0), "emitter acceleration")
        assert(Beam.Width0 == 1 and Beam.Width1 == 1.5 and Beam.CurveSize0 == 2, "beam widths")
        assert(Light.Range == 20, "a light's range stays (measured in Studio)")
        assert(Emitter.ZOffset == 1 and Beam.ZOffset == 1, "zoffsets stay (measured in Studio)")
        assert(Stretched.TextureLength == 4 and Wrapped.TextureLength == 2, "trail texture lengths (a repeat count in Stretch mode)")
        assert(Texture.StudsPerTileU == 1 and Texture.StudsPerTileV == 3, "texture tiles")
        assert(Surface.PixelsPerStud == 100, "surface gui pixels per stud")
        assert(Billboard.Size == UDim2.new(2, 10, 1, 20) and Billboard.StudsOffset == Vector3.new(0, 3, 0), "billboard size and offset")
        local Copy = Model:Clone()
        assert(Copy:GetScale() == 0.5, "a clone keeps its model's scale (ScaleFactor is serialized)")
        Copy:ScaleTo(1)
        assert(Copy:FindFirstChildWhichIsA("BasePart", true):FindFirstChildOfClass("ParticleEmitter").Speed.Max == 20, "a clone scales back from its own scale")
        Model:ScaleTo(1)
        assert(Emitter.Speed.Max == 20 and Light.Range == 20, "back to the authored values")
        "#,
    );
}

/// Model:ScaleTo scales a SpecialMesh's Scale only for a file mesh: the other shapes follow their
/// part's Size (measured in Studio 2026-10-11 with a 2x1x1 part, Scale 1.25 and ScaleTo(2): Head and
/// Sphere stay 1.25, FileMesh becomes 2.5). Characters' heads (Head meshes) grew into domes.
#[test]
fn scale_to_scales_only_file_meshes() {
    assert_script(
        r#"
        local Model = Instance.new("Model")
        local Meshes = {}
        for _, Type in {Enum.MeshType.Head, Enum.MeshType.Sphere, Enum.MeshType.Brick, Enum.MeshType.FileMesh} do
            local Part = Instance.new("Part")
            Part.Size = Vector3.new(2, 1, 1)
            Part.Name = Type.Name
            Part.Parent = Model
            local Mesh = Instance.new("SpecialMesh")
            Mesh.MeshType = Type
            Mesh.Scale = Vector3.new(1.25, 1.25, 1.25)
            Mesh.Parent = Part
            Meshes[Type.Name] = Mesh
        end
        Model:ScaleTo(2)
        assert(Model.Head.Size == Vector3.new(4, 2, 2), "the part grew")
        for _, Name in {"Head", "Sphere", "Brick"} do
            assert(Meshes[Name].Scale == Vector3.new(1.25, 1.25, 1.25), Name .. " mesh scaled")
        end
        assert(Meshes.FileMesh.Scale == Vector3.new(2.5, 2.5, 2.5), "the file mesh did not scale")
        -- a clone keeps the types and scales, and scales back the same way
        local Copy = Model:Clone()
        assert(Copy.Head:FindFirstChildOfClass("SpecialMesh").MeshType == Enum.MeshType.Head, "the clone's mesh type")
        Copy:ScaleTo(1)
        assert(Copy.Head:FindFirstChildOfClass("SpecialMesh").Scale == Vector3.new(1.25, 1.25, 1.25), "the clone's head mesh scaled")
        assert(Copy.FileMesh:FindFirstChildOfClass("SpecialMesh").Scale == Vector3.new(1.25, 1.25, 1.25), "the clone's file mesh")
        "#,
    );
}

/// A model converted once makes the same instances in every engine and every time (`ree serve`
/// keeps bundle plans between its engines): references point at the instantiation's own copies,
/// unknown classes warn each time, and the values read back as the file holds them.
#[test]
fn a_model_plan_makes_the_same_instances_each_time() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{Attributes, CFrame, Color3, Content, Enum, Matrix3, Tags, Variant, Vector3},
    };
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Rig"));
    let mut attributes = Attributes::new();
    attributes.insert("Speed".into(), Variant::Float64(4.5));
    let mut tags = Tags::new();
    tags.push("Hazard");
    let torso = dom.insert(
        model,
        InstanceBuilder::new("Part")
            .with_name("Torso")
            .with_property("Size", Vector3::new(2.0, 2.0, 1.0))
            .with_property("Color", Color3::new(1.0, 0.5, 0.0))
            .with_property("CFrame", CFrame::new(Vector3::new(0.0, 3.0, 0.0), Matrix3::identity()))
            .with_property("Material", Enum::from_u32(272))
            .with_property("Attributes", attributes)
            .with_property("Tags", tags)
            .with_property("NotAProperty", Variant::Int32(7)),
    );
    let head = dom.insert(model, InstanceBuilder::new("MeshPart").with_name("Head"));
    dom.insert(head, InstanceBuilder::new("Decal").with_name("Face").with_property("TextureContent", Content::from_uri("rbxassetid://123")));
    // newer files keep only the Content form (an uploaded bundle's Animation), or an empty string beside it
    dom.insert(model, InstanceBuilder::new("Animation").with_name("Swing").with_property("AnimationContent", Content::from_uri("rbxassetid://12345678901234")));
    dom.insert(
        head,
        InstanceBuilder::new("SpecialMesh")
            .with_name("Shape")
            .with_property("MeshId", Variant::ContentId("".into()))
            .with_property("MeshContent", Content::from_uri("rbxassetid://456")),
    );
    dom.insert(torso, InstanceBuilder::new("Weld").with_name("Neck").with_property("Part0", torso).with_property("Part1", head));
    dom.insert(model, InstanceBuilder::new("ModuleScript").with_name("Logic").with_property("Source", "return 5"));
    dom.insert(model, InstanceBuilder::new("NotAClass").with_name("Odd"));
    let plan = super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children());
    let check = r#"
        local Rig = workspace.Rig
        local Torso, Head = Rig.Torso, Rig.Head
        assert(Torso.Size == Vector3.new(2, 2, 1) and Torso.Position == Vector3.new(0, 3, 0), "part values")
        assert(Torso.Material == Enum.Material.SmoothPlastic, "enum")
        assert(Torso:GetAttribute("Speed") == 4.5 and Torso:HasTag("Hazard"), "attributes and tags")
        assert(Head.Face.Texture == "rbxassetid://123", "a ContentId from its Content sibling")
        assert(Rig.Swing.AnimationId == "rbxassetid://12345678901234", "an AnimationId from AnimationContent")
        assert(Head.Shape.MeshId == "rbxassetid://456" and Head.Shape.MeshContent.Uri == "rbxassetid://456", "an empty MeshId from MeshContent")
        assert(Torso.Neck.Part0 == Torso and Torso.Neck.Part1 == Head, "references within the copy")
        assert(require(Rig.Logic) == 5, "source")
        assert(Rig.Odd.ClassName == "Folder", "unknown class")
        print(#Rig:GetDescendants())
    "#;
    let mut outputs = Vec::new();
    for _ in 0..2 {
        let (lua, engine) = start();
        let workspace = engine.world().workspace;
        let roots = plan.instantiate(&engine, Some(workspace));
        assert_eq!(roots.len(), 1);
        let again = plan.instantiate(&engine, None);
        assert_ne!(again[0], roots[0], "a second instantiation makes new instances");
        outputs.push(run(&lua, &engine, check));
        drop(lua);
        engine.teardown();
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[0].iter().filter(|line| line.starts_with("warn: snapshot: unknown class NotAClass")).count(), 2, "{:?}", outputs[0]);
    assert!(outputs[0].contains(&"print: 8".to_string()), "{:?}", outputs[0]);
}

/// A Content property and its string asset id are one value, each set and read through the other,
/// whatever the string one is called. Measured in Studio (2026-10-12) on the pairs
/// below; Decal, ParticleEmitter and ImageLabel already matched.
#[test]
fn content_properties_alias_their_string_asset_ids() {
    assert_script(
        r#"
        local function Check(ClassName, ContentName, StringName)
            local A = Instance.new(ClassName)
            A[ContentName] = Content.fromUri("rbxassetid://123")
            assert(A[StringName] == "rbxassetid://123", `{ClassName}.{ContentName} reads through {StringName}`)
            local B = Instance.new(ClassName)
            B[StringName] = "rbxassetid://456"
            assert(B[ContentName].Uri == "rbxassetid://456", `{ClassName}.{StringName} reads through {ContentName}`)
        end
        Check("Animation", "AnimationContent", "AnimationId")
        Check("SpecialMesh", "MeshContent", "MeshId")
        Check("SpecialMesh", "TextureContent", "TextureId")
        Check("Sound", "AudioContent", "SoundId")
        Check("Decal", "TextureContent", "Texture")
        Check("ParticleEmitter", "TextureContent", "Texture")
        Check("ImageLabel", "ImageContent", "Image")
        Check("Tool", "TextureContent", "TextureId")
        Check("ShirtGraphic", "TextureContent", "Graphic")
        Check("Sky", "SkyboxBackContent", "SkyboxBk")
        Check("Sky", "SkyboxUpContent", "SkyboxUp")
        Check("Sky", "SunTextureContent", "SunTextureId")
        local Cleared = Instance.new("Animation")
        Cleared.AnimationContent = Content.fromUri("rbxassetid://123")
        Cleared.AnimationId = ""
        assert(Cleared.AnimationContent.SourceType == Enum.ContentSourceType.None, "an empty id is no content")
        local Part = Instance.new("MeshPart")
        Part.TextureContent = Content.fromUri("rbxassetid://666")
        assert(Part.TextureID == "rbxassetid://666", "MeshPart.TextureContent reads through TextureID")
    "#,
    );
}

/// nil clears CustomPhysicalProperties (an effect module that copies it from a part that has none), while
/// other datatype properties refuse it. Measured in Studio (2026-10-12).
#[test]
fn custom_physical_properties_take_nil() {
    assert_script(
        r#"
        local Part = Instance.new("Part")
        assert(Part.CustomPhysicalProperties == nil, "none by default")
        Part.CustomPhysicalProperties = nil
        Part.CustomPhysicalProperties = PhysicalProperties.new(1, 0.3, 0.5)
        assert(Part.CustomPhysicalProperties ~= nil, "set")
        Part.CustomPhysicalProperties = nil
        assert(Part.CustomPhysicalProperties == nil, "cleared")
        local Mesh = Instance.new("MeshPart")
        Mesh.CustomPhysicalProperties = Part.CustomPhysicalProperties
        local Ok, Message = pcall(function() Part.CustomPhysicalProperties = 5 end)
        assert(not Ok and string.find(Message, "PhysicalProperties expected, got number", 1, true), Message)
        local SizeOk, SizeMessage = pcall(function() Part.Size = nil end)
        assert(not SizeOk and string.find(SizeMessage, "Vector3 expected, got nil", 1, true), SizeMessage)
    "#,
    );
}

/// A model imported lazily (an asset manifest bundle: its instances made as scripts look at them)
/// reads exactly as one made at once: names, order, values, references, debug ids (serials),
/// GetTagged, what a method sees and the events of moving it into the world.
/// An instance made of a lazily imported plan node reads its properties through the import, and so do
/// its unchanged copies, until a write gives it its own: every read, listing and copy is what an eager
/// import's instance holds.
#[test]
fn lazily_made_instances_share_their_nodes_properties_until_written() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{Attributes, CFrame, Color3, Matrix3, Variant, Vector3},
    };
    use super::dom::Props;
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    for index in 0..2 {
        let model = dom.insert(root, InstanceBuilder::new("Model").with_name(format!("Unit{index}")));
        let torso = dom.insert(
            model,
            InstanceBuilder::new("Part")
                .with_name("Torso")
                .with_property("Size", Vector3::new(2.0, 2.0, 1.0))
                .with_property("Color", Color3::new(0.25, 0.5, 1.0))
                .with_property("Transparency", 0.5f32)
                .with_property("CFrame", CFrame::new(Vector3::new(index as f32, 3.0, 0.0), Matrix3::identity()))
                .with_property("Attributes", {
                    let mut attributes = Attributes::new();
                    attributes.insert("Speed".to_string(), Variant::Float64(index as f64 + 0.5));
                    attributes.insert("Label".to_string(), Variant::String(format!("unit {index}")));
                    attributes
                }),
        );
        dom.insert(torso, InstanceBuilder::new("Attachment").with_name("Grip"));
        dom.insert(torso, InstanceBuilder::new("Weld").with_name("Self").with_property("Part0", torso));
    }
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let check = r#"
        local Folder = game:GetService("ReplicatedStorage").Bundle
        local Torso = Folder.Unit0.Torso
        print(Torso.Name, Torso.Size, Torso.Color, Torso.Transparency, Torso.CFrame, Torso.Grip.Name, Torso.Self.Part0 == Torso)
        print(Torso:GetAttribute("Speed"), Torso:GetAttribute("Label"), Torso:GetAttribute("Missing"))
        local Attributes = Folder.Unit1.Torso:GetAttributes()
        print(Attributes.Speed, Attributes.Label)
        local Copy = Folder.Unit1:Clone()
        print(Copy.Torso.Size, Copy.Torso.Transparency, Copy.Torso.Self.Part0 == Copy.Torso)
        Copy.Name = "Copy"
        Copy.Torso.Transparency = 0.25
        Copy.Torso:SetAttribute("Speed", 9)
        print(Copy.Torso.Transparency, Folder.Unit1.Torso.Transparency, Copy.Name, Folder.Unit1.Name, Copy.Torso:GetAttribute("Speed"), Folder.Unit1.Torso:GetAttribute("Speed"))
        Torso.Name = "Renamed"
        print(Torso.Name, Torso.Size, Folder.Unit0:FindFirstChild("Renamed") == Torso)
        Copy.Parent = workspace
    "#;
    let mut listings = Vec::new();
    for lazy in [false, true] {
        let (lua, engine) = start();
        let folder = {
            let mut state = engine.state.borrow_mut();
            let class = engine.reflection.class_id("Folder").unwrap();
            let folder = super::instance::create_named(&mut state, &engine.reflection, class, "Bundle");
            let storage = state.dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
            state.dom.attach(folder, storage);
            folder
        };
        if lazy {
            plan.instantiate_lazy(&mut engine.state.borrow_mut(), &engine.reflection, folder);
            // what a read makes of the first unit: its nodes read through the import, but the weld (a reference)
            run(&lua, &engine, "local _ = game:GetService('ReplicatedStorage').Bundle.Unit0.Torso.Self");
            let state = engine.state.borrow();
            let unit = state.dom.find_child(folder, b"Unit0").unwrap();
            let torso = state.dom.find_child(unit, b"Torso").unwrap();
            assert!(matches!(state.dom.at(unit).props, Props::Plan(..)));
            assert!(matches!(state.dom.at(torso).props, Props::Plan(..)));
            assert!(matches!(state.dom.at(torso).attributes, super::dom::Attributes::Plan(..)), "attributes read through the import");
            assert!(matches!(state.dom.at(state.dom.find_child(torso, b"Self").unwrap()).props, Props::Own(_)), "a node with a reference has its own");
        } else {
            plan.instantiate(&engine, Some(folder));
        }
        let output = run(&lua, &engine, check);
        let state = engine.state.borrow();
        if lazy {
            let unit = state.dom.find_child(folder, b"Unit1").unwrap();
            assert!(matches!(state.dom.at(state.dom.find_child(unit, b"Torso").unwrap()).props, Props::Plan(..)), "untouched by its copy's writes");
            let copy = state.dom.find_child(engine.world().workspace, b"Copy").unwrap();
            assert!(matches!(state.dom.at(copy).props, Props::Own(_)), "renamed");
            let copied_torso = state.dom.find_child(copy, b"Torso").unwrap();
            assert!(matches!(state.dom.at(copied_torso).attributes, super::dom::Attributes::Own(_)), "an attribute set");
            assert!(matches!(state.dom.at(state.dom.find_child(unit, b"Torso").unwrap()).attributes, super::dom::Attributes::Plan(..)), "untouched by its copy's");
            let grip = state.dom.find_child(state.dom.find_child(copy, b"Torso").unwrap(), b"Grip").unwrap();
            assert!(matches!(state.dom.at(grip).props, Props::Plan(..)), "an unchanged copy shares");
        }
        let mut listing = vec![output.join("\n")];
        for index in 0..state.dom.slot_count() as u32 {
            if let Some(id) = state.dom.id_at_slot(index) {
                if state.dom.is_descendant_of(id, folder) || state.dom.is_descendant_of(id, engine.world().workspace) {
                    let instance = state.dom.at(id);
                    let props: Vec<String> = instance
                        .props
                        .iter()
                        .map(|(name, value)| match value.as_instance() {
                            Some(target) => format!("{}={}", engine.reflection.name(name), state.dom.full_name(&engine.reflection, target)),
                            None => format!("{}={value:?}", engine.reflection.name(name)),
                        })
                        .collect();
                    let attributes: Vec<String> = instance.attributes.iter().map(|(name, value)| format!("{}={value:?}", name.to_str_lossy())).collect();
                    listing.push(format!("{} {} {} [{}] {}", state.dom.full_name(&engine.reflection, id), props.join(" "), instance.props.len(), attributes.join(" "), instance.attributes.len()));
                }
            }
        }
        listing.sort();
        listings.push(listing);
        drop(state);
        drop(lua);
        engine.teardown();
    }
    assert_eq!(listings[0], listings[1]);
    assert!(listings[0].iter().any(|line| line.contains("print: Renamed 2, 2, 1 true")), "{:?}", listings[0]);
}

#[test]
fn a_lazy_import_reads_as_an_eager_one() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{CFrame, Matrix3, Tags, Variant, Vector3},
    };
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    for index in 0..3 {
        let model = dom.insert(root, InstanceBuilder::new("Model").with_name(format!("Unit{index}")));
        let torso = dom.insert(
            model,
            InstanceBuilder::new("Part")
                .with_name("Torso")
                .with_property("Size", Vector3::new(2.0, 2.0, 1.0))
                .with_property("CFrame", CFrame::new(Vector3::new(index as f32 * 5.0, 3.0, 0.0), Matrix3::identity())),
        );
        let arm = dom.insert(torso, InstanceBuilder::new("Part").with_name("Arm").with_property("CFrame", CFrame::new(Vector3::new(index as f32 * 5.0 + 1.5, 3.0, 0.0), Matrix3::identity())));
        dom.insert(arm, InstanceBuilder::new("Attachment").with_name("Grip"));
        let mut tags = Tags::new();
        tags.push("Glow");
        dom.insert(arm, InstanceBuilder::new("PointLight").with_name("Light").with_property("Tags", tags));
        dom.insert(torso, InstanceBuilder::new("Weld").with_name("Shoulder").with_property("Part0", torso).with_property("Part1", arm));
        dom.insert(model, InstanceBuilder::new("ObjectValue").with_name("Target").with_property("Value", Variant::Ref(arm)));
        let extras = dom.insert(model, InstanceBuilder::new("Folder").with_name("Extras"));
        for part in 0..2 {
            dom.insert(extras, InstanceBuilder::new("Part").with_name(format!("Bit{part}")));
        }
        dom.get_by_ref_mut(model).unwrap().properties.insert("PrimaryPart".into(), Variant::Ref(torso));
    }
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let check = r#"
        local Folder = game:GetService("ReplicatedStorage").Bundle
        local CollectionService = game:GetService("CollectionService")
        print(#CollectionService:GetTagged("Glow"))
        local Added = 0
        workspace.DescendantAdded:Connect(function() Added += 1 end)
        for _, Unit in Folder:GetChildren() do
            print(Unit.Name, Unit:GetDebugId(), Unit.PrimaryPart and Unit.PrimaryPart.Name)
            print(Unit.Target.Value:GetFullName(), Unit.Target.Value:GetDebugId())
        end
        local First = Folder.Unit0
        print(First:GetBoundingBox())
        First.Parent = workspace
        task.wait()
        print("added", Added)
        for _, Item in Folder.Unit1:GetDescendants() do
            print(Item:GetFullName(), Item.ClassName, Item:GetDebugId())
        end
        local Copy = Folder.Unit2:Clone()
        print(#Copy:GetDescendants(), Copy.Torso.Shoulder.Part1 == Copy.Torso.Arm)
    "#;
    let mut outputs = Vec::new();
    for lazy in [false, true] {
        let (lua, engine) = start();
        let folder = {
            let mut state = engine.state.borrow_mut();
            let class = engine.reflection.class_id("Folder").unwrap();
            let folder = super::instance::create_named(&mut state, &engine.reflection, class, "Bundle");
            let storage = state.dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
            state.dom.attach(folder, storage);
            folder
        };
        if lazy {
            plan.instantiate_lazy(&mut engine.state.borrow_mut(), &engine.reflection, folder);
            assert!(engine.state.borrow().project.lazy_made < plan.len() as u32, "made as looked at");
        } else {
            plan.instantiate(&engine, Some(folder));
        }
        outputs.push(run(&lua, &engine, check));
        drop(lua);
        engine.teardown();
    }
    assert_eq!(outputs[0], outputs[1]);
    assert!(outputs[0].contains(&"print: 3".to_string()), "{:?}", outputs[0]);

    // a reference from one root into another: the import is made whole at once (the reference
    // keeps its target alive before any script can drop it)
    let target = dom.root().children()[1];
    let arm = dom.get_by_ref(dom.get_by_ref(target).unwrap().children()[0]).unwrap().children()[0];
    let first = dom.root().children()[0];
    dom.insert(first, InstanceBuilder::new("ObjectValue").with_name("Other").with_property("Value", Variant::Ref(arm)));
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let (lua, engine) = start();
    let workspace_folder = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("Folder").unwrap();
        let folder = super::instance::create_named(&mut state, &engine.reflection, class, "Bundle");
        let storage = state.dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
        state.dom.attach(folder, storage);
        folder
    };
    plan.instantiate_lazy(&mut engine.state.borrow_mut(), &engine.reflection, workspace_folder);
    assert_eq!(engine.state.borrow().project.lazy_made, plan.len() as u32, "made whole");
    drop(lua);
    engine.teardown();
}

/// A lazily imported model's welds hold its parts together as the same model made with the place does. Its rigid joints made after
/// the joint index first looked at the place (a script's assembly read: world/joints.rs ensure_scanned) are indexed as they are
/// made, and what a lazy import makes in a world comes with everything below it (plan.rs make_level): the welded parts of a unit
/// imported into the Workspace after that read (a bundle mounted there imports when a script first looks at it), or reached in a
/// WorldModel inside an import after it, read their assembly, move together and hold each other up, before the script has reached
/// the last of them too.
#[test]
fn a_lazy_imports_welds_hold_its_parts_together() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{CFrame, Matrix3, Ref, Variant, Vector3},
    };
    // a unit: an anchored Torso, an Arm welded to it beside it, a Hand welded below the Arm (the welds a level apart); in the
    // Workspace the plan's root, in the other a Folder's WorldModel's
    let plan = |in_world_model: bool| {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let at = |x: f32, y: f32| CFrame::new(Vector3::new(x, y, 0.0), Matrix3::identity());
        let mut parent: Ref = dom.root_ref();
        if in_world_model {
            let viewport = dom.insert(parent, InstanceBuilder::new("Folder").with_name("Viewport"));
            parent = dom.insert(viewport, InstanceBuilder::new("WorldModel").with_name("World"));
        }
        let model = dom.insert(parent, InstanceBuilder::new("Model").with_name("Unit"));
        let torso = dom.insert(model, InstanceBuilder::new("Part").with_name("Torso").with_property("Anchored", true).with_property("Size", Vector3::new(2.0, 2.0, 1.0)).with_property("CFrame", at(0.0, 10.0)));
        let arm = dom.insert(model, InstanceBuilder::new("Part").with_name("Arm").with_property("Size", Vector3::new(1.0, 2.0, 1.0)).with_property("CFrame", at(1.5, 10.0)));
        let hand = dom.insert(arm, InstanceBuilder::new("Part").with_name("Hand").with_property("Size", Vector3::new(1.0, 1.0, 1.0)).with_property("CFrame", at(1.5, 8.5)));
        dom.insert(model, InstanceBuilder::new("Weld").with_name("Shoulder").with_property("Part0", Variant::Ref(torso)).with_property("Part1", Variant::Ref(arm)).with_property("C0", at(1.5, 0.0)));
        dom.insert(arm, InstanceBuilder::new("Weld").with_name("Wrist").with_property("Part0", Variant::Ref(arm)).with_property("Part1", Variant::Ref(hand)).with_property("C0", at(0.0, -1.5)));
        std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()))
    };
    // (a weld of the script's own, then a read of its assembly: the joint index looks at the place now)
    let scan = r#"
        local A = Instance.new("Part")
        A.Anchored = true
        A.Parent = workspace
        local B = Instance.new("Part")
        B.Parent = workspace
        local Weld = Instance.new("Weld")
        Weld.Part0, Weld.Part1 = A, B
        Weld.Parent = A
        assert(B.AssemblyRootPart == A, "the script's weld links its parts")
    "#;
    // (the Arm's assembly read and the Torso moved before the script reaches the Hand)
    let check = r#"
        local Bundle = workspace:FindFirstChild("Bundle")
        local Unit = if Bundle then Bundle.Unit else game:GetService("ReplicatedStorage").Bundle.Viewport.World.Unit
        local Torso, Arm = Unit.Torso, Unit.Arm
        print("first", Arm.AssemblyMass)
        Torso.CFrame = Torso.CFrame + Vector3.new(0, 1, 0)
        local Hand = Arm.Hand
        print("hand", Hand.Position)
        local function Name(Part) return Part and Part.Name end
        print("roots", Name(Torso.AssemblyRootPart), Name(Arm.AssemblyRootPart), Name(Hand.AssemblyRootPart))
        print("mass", Arm.AssemblyMass, Hand.AssemblyMass, Hand.AssemblyCenterOfMass)
        Torso.CFrame = Torso.CFrame + Vector3.new(0, 5, 0)
        print("moved", Arm.Position, Hand.Position)
        task.wait(0.5)
        print("held", Arm.Position, Hand.Position)
    "#;
    for in_world_model in [false, true] {
        let plan = plan(in_world_model);
        let mut outputs = Vec::new();
        for lazy in [false, true] {
            let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
            let root = {
                let mut state = engine.state.borrow_mut();
                let parent = if in_world_model { state.dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap() } else { engine.world().workspace };
                let root = super::instance::create_named(&mut state, &engine.reflection, engine.reflection.class_id("Folder").unwrap(), "Bundle");
                state.dom.attach(root, parent);
                root
            };
            // eagerly: made with the place, before any script runs; lazily: the WorldModel's import with the place (its Folder
            // made), the Workspace's once the script's read looked at the place
            if !lazy {
                plan.instantiate(&engine, Some(root));
            } else if in_world_model {
                plan.instantiate_lazy(&mut engine.state.borrow_mut(), &engine.reflection, root);
                assert_eq!(engine.state.borrow().project.lazy_made, 1, "the import's Folder alone");
            }
            run(&lua, &engine, scan);
            if lazy && !in_world_model {
                plan.instantiate_lazy(&mut engine.state.borrow_mut(), &engine.reflection, root);
                assert_eq!(engine.state.borrow().project.lazy_made, plan.len() as u32, "made whole in the Workspace");
            }
            let output = run(&lua, &engine, check);
            if lazy {
                assert_eq!(engine.state.borrow().project.lazy_made, plan.len() as u32, "made whole once reached in its world");
            }
            super::world::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index holds together");
            outputs.push(output);
            drop(lua);
            engine.teardown();
        }
        assert_eq!(outputs[0], outputs[1], "lazily made as made with the place (in a WorldModel: {in_world_model})");
        for line in ["print: hand 1.5, 9.5, 0", "print: roots Torso Torso Torso", "print: moved 1.5, 16, 0 1.5, 14.5, 0", "print: held 1.5, 16, 0 1.5, 14.5, 0"] {
            assert!(outputs[0].iter().any(|printed| printed == line), "{line} (in a WorldModel: {in_world_model}): {:?}", outputs[0]);
        }
    }
}

/// Tags a newer Studio writes as a SharedString (as uploaded assets can hold them; project::decode_model
/// reads the property as one) are the instance's tags, as when they are written as Tags.
#[test]
fn tags_held_as_a_shared_string_are_tags() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{SharedString, Variant},
    };
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let model = dom.insert(dom.root_ref(), InstanceBuilder::new("Model").with_name("Mount").with_property("Tags", Variant::SharedString(SharedString::new(b"RemoveFromBoundingBox\0RemoveFromLobby".to_vec()))));
    dom.insert(model, InstanceBuilder::new("Part").with_name("Plain").with_property("Tags", Variant::SharedString(SharedString::new(Vec::new()))));
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let (lua, engine) = start();
    let storage = engine.state.borrow().dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
    plan.instantiate(&engine, Some(storage));
    let output = run(&lua, &engine, r#"
        local Mount = game:GetService("ReplicatedStorage").Mount
        print(table.concat(Mount:GetTags(), ","), #Mount.Plain:GetTags(), #game:GetService("CollectionService"):GetTagged("RemoveFromLobby"))
    "#);
    assert_eq!(output, ["print: RemoveFromBoundingBox,RemoveFromLobby 0 1"]);
    drop(lua);
    engine.teardown();
}

/// A folder named `name` in `parent` (made as the engine builds its own: no events).
fn folder_in(engine: &Engine, parent: super::InstanceId, name: &str) -> super::InstanceId {
    let mut state = engine.state.borrow_mut();
    let class = engine.reflection.class_id("Folder").unwrap();
    let folder = super::instance::create_named(&mut state, &engine.reflection, class, name);
    state.dom.attach(folder, parent);
    folder
}

/// A folder named `name` in ReplicatedStorage.
fn storage_folder(engine: &Engine, name: &str) -> super::InstanceId {
    let storage = engine.state.borrow().dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
    folder_in(engine, storage, name)
}

/// A plan of one root made as the instance that stood in its place (a manifest bundle's
/// placeholder: `ModelPlan::instantiate_lazy_as`) reads as the root made at once there and named as
/// the game names an installed bundle (its class, values, attributes, tags, children, joints,
/// clones and moves); the placeholder's own name and identity stay.
#[test]
fn a_placeholder_made_its_plans_root_reads_as_the_root_made_at_once() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{Attributes, CFrame, Matrix3, Tags, Variant, Vector3},
    };
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let mut attributes = Attributes::new();
    attributes.insert("Reach".into(), Variant::Float64(5.0));
    let mut tags = Tags::new();
    tags.push("Machine");
    let model = dom.insert(dom.root_ref(), InstanceBuilder::new("Model").with_name("Machine File").with_property("Attributes", attributes).with_property("Tags", tags));
    let place = |y: f32| CFrame::new(Vector3::new(1.0, y, 3.0), Matrix3::identity());
    let body = dom.insert(model, InstanceBuilder::new("MeshPart").with_name("Body").with_property("Size", Vector3::new(4.0, 5.0, 6.0)).with_property("CFrame", place(2.0)));
    let cab = dom.insert(model, InstanceBuilder::new("Part").with_name("Cab").with_property("CFrame", place(5.0)));
    dom.insert(cab, InstanceBuilder::new("Weld").with_name("Weld").with_property("Part0", Variant::Ref(body)).with_property("Part1", Variant::Ref(cab)));
    let bone = dom.insert(body, InstanceBuilder::new("Bone").with_name("Root"));
    dom.insert(bone, InstanceBuilder::new("Bone").with_name("Neck"));
    dom.get_by_ref_mut(model).unwrap().properties.insert("PrimaryPart".into(), Variant::Ref(body));
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let check = r#"
        local Machines = game:GetService("ReplicatedStorage").Machines
        local Crane = Machines["Tall Crane"]
        print(Crane.ClassName, Crane.Name, Crane.PrimaryPart:GetFullName(), Crane:GetAttribute("Reach"), table.concat(Crane:GetTags(), ","))
        print(#game:GetService("CollectionService"):GetTagged("Machine"))
        for _, Item in Crane:GetDescendants() do
            print(Item:GetFullName(), Item.ClassName)
        end
        print(Crane.Cab.Weld.Part0 == Crane.Body, Crane:GetPivot())
        print(Crane:GetBoundingBox())
        local Copy = Crane:Clone()
        print(Copy.Name, Copy.PrimaryPart == Copy.Body, #Copy:GetDescendants())
        Crane.Parent = workspace
        print(Crane:GetFullName(), #workspace:GetDescendants())
    "#;
    let mut outputs = Vec::new();
    for at_once in [true, false] {
        let (lua, engine) = start();
        let machines = storage_folder(&engine, "Machines");
        if at_once {
            let roots = plan.instantiate(&engine, Some(machines));
            let mut state = engine.state.borrow_mut();
            super::instance::raw_set_silent(&mut state, &engine.reflection, roots[0], super::K::Name as super::NameId, super::value::Value::String("Tall Crane".into()));
        } else {
            let placeholder = folder_in(&engine, machines, "Tall Crane");
            plan.instantiate_lazy_as(&mut engine.state.borrow_mut(), &engine.reflection, placeholder);
            assert_eq!(engine.state.borrow().project.lazy_made, 2, "the root's children now, the rest as it is looked at");
        }
        outputs.push(run(&lua, &engine, check));
        drop(lua);
        engine.teardown();
    }
    assert_eq!(outputs[0], outputs[1]);
    assert!(outputs[0][0].starts_with("print: Model Tall Crane ReplicatedStorage.Machines.Tall Crane.Body 5 Machine"), "{:?}", outputs[0]);
}

/// What an import that makes a root's children at once makes then, as a folder bundle's roots were
/// made: a folder root's children (each made whole only when it, or something in it, moves) unless
/// one references into another; any other root is one region (made whole before anything in it
/// moves); a loose bundle's members go into the folder given, the root itself never made (a member's
/// reference to it reads nil).
#[test]
fn a_roots_children_import_as_a_folder_bundles_roots_did() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{Ref, Variant},
    };
    let units = |cross: bool| {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let folder = dom.insert(dom.root_ref(), InstanceBuilder::new("Folder").with_name("Units"));
        let mut arms = Vec::new();
        for index in 0..2 {
            let unit = dom.insert(folder, InstanceBuilder::new("Model").with_name(format!("Unit{index}")));
            let torso = dom.insert(unit, InstanceBuilder::new("Part").with_name("Torso"));
            arms.push(dom.insert(torso, InstanceBuilder::new("Part").with_name("Arm")));
        }
        if cross {
            let first = dom.get_by_ref(folder).unwrap().children()[0];
            dom.insert(first, InstanceBuilder::new("ObjectValue").with_name("Other").with_property("Value", Variant::Ref(arms[1])));
        }
        std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()))
    };
    let made = |plan: &std::sync::Arc<super::project::ModelPlan>, script: &str| {
        let (lua, engine) = start();
        let placeholder = storage_folder(&engine, "Units");
        plan.instantiate_lazy_as(&mut engine.state.borrow_mut(), &engine.reflection, placeholder);
        let before = engine.state.borrow().project.lazy_made;
        run(&lua, &engine, script);
        let after = engine.state.borrow().project.lazy_made;
        drop(lua);
        engine.teardown();
        (before, after)
    };
    let move_first = r#"game:GetService("ReplicatedStorage").Units.Unit0.Parent = workspace"#;
    let plan = units(false);
    assert_eq!(made(&plan, move_first), (2, 4), "the folder's children now; the moved one whole, the other not");
    let plan = units(true);
    assert_eq!(made(&plan, move_first).0, plan.len() as u32 - 1, "a reference across them: everything at once");

    // a model's children reference each other: one region, made whole when one of them moves
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let model = dom.insert(dom.root_ref(), InstanceBuilder::new("Model").with_name("Crane"));
    let body = dom.insert(model, InstanceBuilder::new("Part").with_name("Body"));
    dom.insert(body, InstanceBuilder::new("Attachment").with_name("Grip"));
    let cab = dom.insert(model, InstanceBuilder::new("Part").with_name("Cab"));
    dom.insert(cab, InstanceBuilder::new("Weld").with_name("Weld").with_property("Part0", Variant::Ref(body)).with_property("Part1", Variant::Ref(cab)));
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let (lua, engine) = start();
    let placeholder = storage_folder(&engine, "Crane");
    plan.instantiate_lazy_as(&mut engine.state.borrow_mut(), &engine.reflection, placeholder);
    assert_eq!(engine.state.borrow().project.lazy_made, 2, "its children now");
    run(&lua, &engine, r#"game:GetService("ReplicatedStorage").Crane.Cab.Parent = workspace"#);
    assert_eq!(engine.state.borrow().project.lazy_made, plan.len() as u32 - 1, "made whole before a part of it moved");
    drop(lua);
    engine.teardown();

    // a loose bundle: its members
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let content = dom.insert(dom.root_ref(), InstanceBuilder::new("Folder").with_name("Content"));
    let sword = dom.insert(content, InstanceBuilder::new("Model").with_name("Sword"));
    dom.insert(sword, InstanceBuilder::new("ObjectValue").with_name("Owner").with_property("Value", Variant::Ref(content)));
    dom.insert(sword, InstanceBuilder::new("ObjectValue").with_name("Empty").with_property("Value", Variant::Ref(Ref::none())));
    dom.insert(content, InstanceBuilder::new("Part").with_name("Shield"));
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let (lua, engine) = start();
    let items = storage_folder(&engine, "Items");
    let members = plan.instantiate_lazy_members(&mut engine.state.borrow_mut(), &engine.reflection, items);
    assert_eq!(members.len(), 2);
    let output = run(&lua, &engine, r#"
        local Items = game:GetService("ReplicatedStorage").Items
        print(#Items:GetChildren(), Items.Sword.ClassName, Items.Shield.ClassName, Items:FindFirstChild("Content"), Items.Sword.Owner.Value, Items.Sword.Empty.Value)
    "#);
    assert_eq!(output, ["print: 2 Model Part nil nil nil"]);
    drop(lua);
    engine.teardown();
}

/// A rig imported lazily (a bundle template) animates as one made at once: LoadAnimation on its
/// Animator makes the rest of the model first, so the Animator step finds every Motor6D, and the
/// template moved into the world poses the same.
#[test]
fn a_lazily_imported_rig_animates_as_an_eager_one() {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{CFrame, Matrix3, Variant, Vector3},
    };
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let place = |x: f32, y: f32| CFrame::new(Vector3::new(x, y, 0.0), Matrix3::identity());
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Rig"));
    let hrp = dom.insert(model, InstanceBuilder::new("Part").with_name("HumanoidRootPart").with_property("Anchored", true).with_property("CFrame", place(0.0, 3.0)));
    let torso = dom.insert(model, InstanceBuilder::new("Part").with_name("Torso").with_property("CFrame", place(0.0, 3.0)));
    let arm = dom.insert(model, InstanceBuilder::new("Part").with_name("Right Arm").with_property("Size", Vector3::new(1.0, 2.0, 1.0)).with_property("CFrame", place(1.5, 3.0)));
    dom.insert(hrp, InstanceBuilder::new("Motor6D").with_name("RootJoint").with_property("Part0", Variant::Ref(hrp)).with_property("Part1", Variant::Ref(torso)));
    dom.insert(
        torso,
        InstanceBuilder::new("Motor6D")
            .with_name("Right Shoulder")
            .with_property("Part0", Variant::Ref(torso))
            .with_property("Part1", Variant::Ref(arm))
            .with_property("C0", place(1.5, 0.5))
            .with_property("C1", place(0.0, 0.5)),
    );
    let humanoid = dom.insert(model, InstanceBuilder::new("Humanoid"));
    dom.insert(humanoid, InstanceBuilder::new("Animator"));
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let check = r#"
        local Provider = game:GetService("KeyframeSequenceProvider")
        local Sequence = Instance.new("KeyframeSequence")
        for _, Entry in { {0, 0}, {1, 1} } do
            local Keyframe = Instance.new("Keyframe")
            Keyframe.Time = Entry[1]
            local Root = Instance.new("Pose")
            Root.Name = "HumanoidRootPart"
            Root.Weight = 0
            Root.Parent = Keyframe
            local Torso = Instance.new("Pose")
            Torso.Name = "Torso"
            Torso.Weight = 0
            Torso.Parent = Root
            local Arm = Instance.new("Pose")
            Arm.Name = "Right Arm"
            Arm.CFrame = CFrame.Angles(Entry[2], 0, 0)
            Arm.Parent = Torso
            Keyframe.Parent = Sequence
        end
        local Animation = Instance.new("Animation")
        Animation.AnimationId = Provider:RegisterKeyframeSequence(Sequence)
        local Template = game:GetService("ReplicatedStorage").Bundle.Rig
        local Track = Template.Humanoid.Animator:LoadAnimation(Animation)
        Template.Parent = workspace
        Track:Play(0)
        task.wait(0.5)
        local Shoulder = Template.Torso["Right Shoulder"]
        print(Track.TimePosition, Shoulder.Transform, Template["Right Arm"].CFrame)
        for _, Item in Template:GetDescendants() do
            print(Item:GetFullName(), Item:GetDebugId())
        end
    "#;
    let mut outputs = Vec::new();
    for lazy in [false, true] {
        let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
        let lua = Lua::new();
        engine_lua::start(&lua, &engine).expect("start");
        let folder = {
            let mut state = engine.state.borrow_mut();
            let class = engine.reflection.class_id("Folder").unwrap();
            let folder = super::instance::create_named(&mut state, &engine.reflection, class, "Bundle");
            let storage = state.dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
            state.dom.attach(folder, storage);
            folder
        };
        if lazy {
            plan.instantiate_lazy(&mut engine.state.borrow_mut(), &engine.reflection, folder);
            assert!(engine.state.borrow().project.lazy_made < plan.len() as u32, "made as looked at");
        } else {
            plan.instantiate(&engine, Some(folder));
        }
        let script = {
            let mut state = engine.state.borrow_mut();
            let class = engine.reflection.class_id("ModuleScript").unwrap();
            super::instance::create_named(&mut state, &engine.reflection, class, "Test")
        };
        let env = engine_lua::modules::environment(&lua, &engine, script).expect("environment");
        let function = lua.load(check).set_name("=Test").set_environment(env).into_function().expect("compile");
        let call = engine_lua::globals::call(&lua, &engine, function, LuaMultiValue::new()).expect("call");
        crate::scheduler::run_until(&lua, &engine, 1.0).expect("run");
        if let Err(message) = call.finish(&lua, &engine) {
            panic!("test script failed: {message}");
        }
        outputs.push(lines(&engine));
        drop(lua);
        engine.teardown();
    }
    assert_eq!(outputs[0], outputs[1]);
    assert!(outputs[0].iter().any(|line| line.contains("Right Shoulder")), "{:?}", outputs[0]);
}

/// A model file's stored scale (ScaleFactor, rbx-dom's Scale) is the model's GetScale: effect bundles
/// can save orbs at 0.01 and grow them with ScaleTo(1).
#[test]
fn a_model_file_keeps_its_scale() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom, types::{Variant, Vector3}};
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let model = dom.insert(root, InstanceBuilder::new("Model").with_name("Orb").with_property("Scale", Variant::Float32(0.25)));
    dom.insert(model, InstanceBuilder::new("Part").with_name("Core").with_property("Size", Vector3::new(1.0, 1.0, 1.0)));
    let plan = super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children());
    let (lua, engine) = start();
    plan.instantiate(&engine, Some(engine.world().workspace));
    let output = run(
        &lua,
        &engine,
        r#"
        local Orb = workspace.Orb
        assert(Orb:GetScale() == 0.25, "the stored scale")
        Orb:ScaleTo(1)
        assert(Orb.Core.Size == Vector3.new(4, 4, 4), "ScaleTo(1) from the stored scale")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// A place's snapshot lists its non-script content by name and class only, so a module's fallback Rig
/// (the rig unresolved entities clone, stored at Model scale 0.43 in the places measured) was a
/// default Model: scale 1, no pivot, no parts' sizes, and a ScaleTo(1) guard looked like a 9.74 ms
/// gain that Studio did not give. `data/place-content/<Place>/<parent path>/*.rbxm` (Studio's own
/// serialization of the instance) now replaces the placeholder with its stored properties,
/// attributes and tags.
#[test]
fn place_content_replaces_a_snapshot_placeholder_with_the_stored_instance() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom, types::{Attributes, CFrame, Matrix3, Variant, Vector3}};
    let dir = std::env::temp_dir().join(format!("ree-place-content-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (project, repo) = (dir.join("project"), dir.join("repo"));
    std::fs::create_dir_all(project.join("place/ReplicatedStorage/Shared")).unwrap();
    std::fs::create_dir_all(project.join(super::project::SNAPSHOTS_DIR)).unwrap();
    std::fs::write(project.join("place/ReplicatedStorage/Shared/RigLibrary.luau"), "return {}").unwrap();
    std::fs::write(project.join(super::project::PLACES_FILE), r#"{"environments":[{"name":"Lobby","projectFile":"place/default.project.json"}]}"#).unwrap();
    let node = |name: &str, class: &str, parent: i64| format!(r#"{{"name":"{name}","className":"{class}","parent":{parent}}}"#);
    let nodes = [
        node("game", "DataModel", -1),
        node("ReplicatedStorage", "ReplicatedStorage", 0),
        node("Shared", "Folder", 1),
        node("RigLibrary", "ModuleScript", 2),
        node("Rig", "Model", 3),
        node("HumanoidRootPart", "Part", 4),
        node("Hat", "Part", 4),
    ];
    std::fs::write(project.join(super::project::SNAPSHOTS_DIR).join("Lobby.json"), format!(r#"{{"nodes":[{}]}}"#, nodes.join(","))).unwrap();
    // the Rig as Studio stores it
    let mut place = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = place.root_ref();
    let at = |x, y, z| Variant::OptionalCFrame(Some(CFrame::new(Vector3::new(x, y, z), Matrix3::identity())));
    let rig = place.insert(
        root,
        InstanceBuilder::new("Model")
            .with_name("Rig")
            .with_property("Scale", Variant::Float32(0.43))
            .with_property("WorldPivotData", at(5.0, 6.0, 7.0))
            .with_property("ModelStreamingMode", Variant::Enum(rbx_dom_weak::types::Enum::from_u32(2)))
            .with_property("Attributes", Attributes::new().with("Kind", Variant::String("fallback".into())))
            .with_property("Tags", rbx_dom_weak::types::Tags::from(vec!["Fallback".to_string()])),
    );
    place.insert(rig, InstanceBuilder::new("Part").with_name("HumanoidRootPart").with_property("Size", Vector3::new(2.0, 2.0, 1.0)));
    let content = repo.join("data/place-content/Lobby/ReplicatedStorage/Shared/RigLibrary");
    std::fs::create_dir_all(&content).unwrap();
    let mut bytes = Vec::new();
    rbx_binary::to_writer(&mut bytes, &place, &[rig]).unwrap();
    std::fs::write(content.join("Rig.rbxm"), bytes).unwrap();

    let engine = Engine::new(Options { repo, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    let spec = super::mount::ProjectSpec { roots: vec![project.join("place").to_string_lossy().replace('\\', "/")], ..Default::default() };
    super::mount::mount_project(&lua, &engine, &spec, std::time::Instant::now()).expect("mount");
    let output = run(
        &lua,
        &engine,
        r#"
        local Library = game:GetService("ReplicatedStorage").Shared.RigLibrary
        local Rig = Library:FindFirstChild("Rig")
        assert(Rig, "the Rig")
        assert(#Library:GetChildren() == 1, "one Rig, not the stored one beside the placeholder")
        assert(math.abs(Rig:GetScale() - 0.43) < 1e-6, "the stored scale " .. Rig:GetScale())
        assert(Rig.WorldPivot.Position == Vector3.new(5, 6, 7), "the stored pivot")
        assert(Rig.ModelStreamingMode == Enum.ModelStreamingMode.Persistent, "the stored streaming mode " .. tostring(Rig.ModelStreamingMode))
        assert(Rig:GetAttribute("Kind") == "fallback", "the stored attribute")
        assert(Rig:HasTag("Fallback"), "the stored tag")
        assert(Rig.HumanoidRootPart.Size == Vector3.new(2, 2, 1), "the stored part size")
        assert(Rig:FindFirstChild("Hat") == nil, "the file is the whole subtree: a child only the (older) snapshot lists is not added")
        assert(#Rig:GetChildren() == 1, "the Rig's children are the file's")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An enum property refuses what it cannot read with a playing client's and a server's texts (Studio 0.744,
/// tools/parity/probes/enum_assignment.luau): nil and other types "EnumItem, number, or string expected", a value no item
/// has "Invalid value", truncated and unquoted for a number or a numeric string.
#[test]
fn enum_properties_refuse_with_the_measured_texts() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local function Try(F) local Ok, Message = pcall(F) return if Ok then "ok" else Message end
        local Part, Label = Instance.new("Part"), Instance.new("TextLabel")
        print(Try(function() Part.Material = nil end))
        print(Try(function() Label.Font = Vector3.zero end))
        print(Try(function() Label.Font = "256" end))
        print(Try(function() Label.Font = 512.5 end))
        print(Try(function() Label.Font = "Wood" end))
        "#,
    );
    assert_eq!(
        lines(&engine),
        vec![
            "print: Unable to assign property Material. EnumItem, number, or string expected, got nil",
            "print: Unable to assign property Font. EnumItem, number, or string expected, got Vector3",
            "print: Unable to assign property Font. Invalid value 256 for enum Font",
            "print: Unable to assign property Font. Invalid value 512 for enum Font",
            "print: Unable to assign property Font. Invalid value \"Wood\" for enum Font",
        ]
    );
}

/// A directory of the source tree that holds only scripts stands for the instance the place keeps them in: the place's
/// hierarchy lists that path as a Part (a published place holds a Part with its scripts inside while the source tree has
/// only the scripts), so the directory is a Part, Studio-only, with its scripts and the hierarchy's other children in it,
/// and a clone of it is a Part a script can move; it was a Folder, and setting its CFrame raised `CFrame is not a valid
/// member of Folder`. A directory the hierarchy lists as a Folder or a script, or does not list, stays a Folder, and
/// place content's stored instance keeps the directory's scripts.
#[test]
fn a_directory_of_scripts_is_the_instance_the_place_keeps_them_in() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom, types::{Variant, Vector3}};
    let dir = std::env::temp_dir().join(format!("ree-dir-classes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (project, repo) = (dir.join("project"), dir.join("repo"));
    let skill = project.join("place/ReplicatedStorage/Effects/Skill");
    for sub in ["Frog/Inner", "Plain", "Lonely", "Coded", "Wind"] {
        std::fs::create_dir_all(skill.join(sub)).unwrap();
    }
    std::fs::write(skill.join("Frog/SelfDestruct.server.luau"), "return nil").unwrap();
    std::fs::write(skill.join("Frog/Inner/Helper.luau"), "return 1").unwrap();
    std::fs::write(skill.join("Plain/A.luau"), "return 1").unwrap();
    std::fs::write(skill.join("Lonely/B.luau"), "return 1").unwrap();
    std::fs::write(skill.join("Coded/C.luau"), "return 1").unwrap();
    std::fs::write(skill.join("Wind/Textures.server.luau"), "-- the source tree's").unwrap();
    std::fs::create_dir_all(project.join(super::project::SNAPSHOTS_DIR)).unwrap();
    std::fs::write(project.join(super::project::PLACES_FILE), r#"{"environments":[{"name":"Lobby","projectFile":"place/default.project.json"}]}"#).unwrap();
    let node = |name: &str, class: &str, parent: i64| format!(r#"{{"name":"{name}","className":"{class}","parent":{parent}}}"#);
    let nodes = [
        node("game", "DataModel", -1),
        node("ReplicatedStorage", "ReplicatedStorage", 0),
        node("Effects", "Folder", 1),
        node("Skill", "Folder", 2),
        node("Frog", "Part", 3),
        node("SelfDestruct", "Script", 4),
        node("Spark", "ParticleEmitter", 4),
        node("Inner", "Attachment", 4),
        node("Plain", "Folder", 3),
        node("Coded", "ModuleScript", 3),
        node("Wind", "Part", 3),
    ];
    std::fs::write(project.join(super::project::SNAPSHOTS_DIR).join("Lobby.json"), format!(r#"{{"nodes":[{}]}}"#, nodes.join(","))).unwrap();
    // Wind as the place stores it: a sized Part with a mesh, and an older copy of its script
    let mut place = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = place.root_ref();
    let wind = place.insert(root, InstanceBuilder::new("Part").with_name("Wind").with_property("Size", Vector3::new(2.0, 3.0, 4.0)));
    place.insert(wind, InstanceBuilder::new("SpecialMesh").with_name("Mesh"));
    place.insert(wind, InstanceBuilder::new("Script").with_name("Textures").with_property("Source", Variant::String("-- the place's older copy".into())));
    let content = repo.join("data/place-content/Lobby/ReplicatedStorage/Effects/Skill");
    std::fs::create_dir_all(&content).unwrap();
    let mut bytes = Vec::new();
    rbx_binary::to_writer(&mut bytes, &place, &[wind]).unwrap();
    std::fs::write(content.join("Wind.rbxm"), bytes).unwrap();

    let engine = Engine::new(Options { repo, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    let spec = super::mount::ProjectSpec { roots: vec![project.join("place").to_string_lossy().replace('\\', "/")], ..Default::default() };
    super::mount::mount_project(&lua, &engine, &spec, std::time::Instant::now()).expect("mount");
    let output = run(
        &lua,
        &engine,
        r#"
        local Skill = game:GetService("ReplicatedStorage").Effects.Skill
        local Frog = Skill.Frog
        assert(Frog.ClassName == "Part" and Frog:IsA("BasePart"), "the place's class " .. Frog.ClassName)
        assert(Frog.SelfDestruct.ClassName == "Script", "its script")
        assert(Frog.Inner.ClassName == "Attachment" and Frog.Inner.Helper.ClassName == "ModuleScript", "a nested directory takes its class too")
        assert(Frog:FindFirstChild("Spark") and Frog.Spark.ClassName == "ParticleEmitter", "the hierarchy's other children")
        local Copy = Frog:Clone()
        Copy.Parent = workspace
        Copy.CFrame = CFrame.new(1, 2, 3)
        assert(Copy.ClassName == "Part" and Copy.Position == Vector3.new(1, 2, 3) and Copy:FindFirstChild("SelfDestruct"), "a clone is a Part with its script")
        assert(Skill.Plain.ClassName == "Folder" and Skill.Lonely.ClassName == "Folder" and Skill.Coded.ClassName == "Folder", "Folder, unlisted and script paths stay Folders")
        local Wind = Skill.Wind
        local Count = 0
        for _, Child in Skill:GetChildren() do
            if Child.Name == "Wind" then
                Count += 1
            end
        end
        assert(Count == 1, "one Wind")
        assert(Wind.ClassName == "Part" and Wind.Size == Vector3.new(2, 3, 4) and Wind:FindFirstChild("Mesh"), "place content's stored Wind")
        local Scripts = 0
        for _, Child in Wind:GetChildren() do
            if Child.Name == "Textures" then
                Scripts += 1
            end
        end
        assert(Scripts == 1, "the source tree's script replaces the stored copy")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    let state = engine.state.borrow();
    let textures = state.dom.descendants(engine.world().game).into_iter().find(|&id| state.dom.at(id).name.as_bytes() == b"Textures").expect("Textures");
    assert!(state.dom.at(textures).source_path().is_some_and(|path| path.ends_with("Wind/Textures.server.luau")), "the source tree's script");
    drop(state);
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A model file's stored pivot (WorldPivotData) is the model's pivot, not its parts' centre, and a
/// nested model's stored pivot moves with its ancestor: a scene moved with PivotTo can aim its camera
/// rig from a nested rig's pivot.
#[test]
fn a_model_file_keeps_its_pivot() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom, types::{CFrame, Matrix3, Variant, Vector3}};
    let at = |x, y, z| Variant::OptionalCFrame(Some(CFrame::new(Vector3::new(x, y, z), Matrix3::identity())));
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let scene = dom.insert(root, InstanceBuilder::new("Model").with_name("Scene").with_property("WorldPivotData", at(100.0, 0.0, 0.0)));
    dom.insert(scene, InstanceBuilder::new("Part").with_name("Far").with_property("CFrame", CFrame::new(Vector3::new(500.0, 0.0, 0.0), Matrix3::identity())));
    let rig = dom.insert(scene, InstanceBuilder::new("Model").with_name("Rig").with_property("WorldPivotData", at(100.0, 2.0, 0.0)));
    dom.insert(rig, InstanceBuilder::new("Part").with_name("Root").with_property("CFrame", CFrame::new(Vector3::new(100.0, 2.0, 0.0), Matrix3::identity())));
    let plan = super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children());
    let (lua, engine) = start();
    plan.instantiate(&engine, Some(engine.world().workspace));
    let output = run(
        &lua,
        &engine,
        r#"
        local Scene = workspace.Scene
        assert(Scene:GetPivot().Position == Vector3.new(100, 0, 0), "the stored pivot, not the parts' centre")
        assert(Scene.WorldPivot.Position == Vector3.new(100, 0, 0), "WorldPivot reads the stored pivot")
        Scene:PivotTo(CFrame.new(0, 50, 0))
        assert(Scene.Far.Position == Vector3.new(400, 50, 0), "parts move by the pivot's offset")
        assert(Scene.Rig:GetPivot().Position == Vector3.new(0, 52, 0), "a nested stored pivot moves too")
        assert(Scene:Clone():GetPivot().Position == Vector3.new(0, 50, 0), "a clone keeps the moved pivot")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// Measured in Studio (2026-10-16): an unset pivot follows the parts' centre, a set one stays, a
/// PrimaryPart overrides it without changing WorldPivot, and ScaleTo scales nested stored pivots.
#[test]
fn model_pivots_follow_studio() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local M = Instance.new("Model")
        assert(M:GetPivot().Position == Vector3.zero and M.WorldPivot.Position == Vector3.zero, "empty")
        local A = Instance.new("Part"); A.Size = Vector3.new(2, 2, 2); A.CFrame = CFrame.new(10, 0, 0); A.Parent = M
        local B = Instance.new("Part"); B.Size = Vector3.new(2, 2, 2); B.CFrame = CFrame.new(30, 0, 0); B.Parent = M
        assert(M:GetPivot().Position == Vector3.new(20, 0, 0), "unset: the parts' centre")
        M.WorldPivot = CFrame.new(0, 100, 0)
        assert(M:GetPivot().Position == Vector3.new(0, 100, 0), "set")
        local Inner = Instance.new("Model"); Inner.WorldPivot = CFrame.new(5, 5, 5); Inner.Parent = M
        local C = Instance.new("Part"); C.CFrame = CFrame.new(50, 0, 0); C.Parent = Inner
        assert(Inner:GetPivot().Position == Vector3.new(5, 5, 5), "a set pivot ignores new parts")
        M:PivotTo(CFrame.new(0, 200, 0))
        assert(Inner:GetPivot().Position == Vector3.new(5, 105, 5) and A.Position == Vector3.new(10, 100, 0), "moved")
        M.PrimaryPart = B
        assert(M:GetPivot().Position == Vector3.new(30, 100, 0) and M.WorldPivot.Position == Vector3.new(0, 200, 0), "primary")
        M.PrimaryPart = nil
        assert(M:GetPivot().Position == Vector3.new(0, 200, 0), "primary cleared")
        local S = Instance.new("Model"); S.WorldPivot = CFrame.new()
        local SI = Instance.new("Model"); SI.WorldPivot = CFrame.new(10, 0, 0); SI.Parent = S
        local SP = Instance.new("Part"); SP.CFrame = CFrame.new(10, 0, 0); SP.Parent = SI
        S:ScaleTo(2)
        assert(SI:GetPivot().Position == Vector3.new(20, 0, 0) and SP.Position == Vector3.new(20, 0, 0), "scaled")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// Studio's own numbers (2026-10-16): a part's PivotTo honours its PivotOffset and carries every part
/// under it (directly, through a Folder, an Attachment and a Model) as one rigid move.
#[test]
fn a_part_pivot_carries_the_parts_under_it() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local function at(Part, X, Y, Z) return (Part.Position - Vector3.new(X, Y, Z)).Magnitude < 1e-3 end
        local A = Instance.new("Part"); A.Anchored = true; A.CFrame = CFrame.new(1, 0, 0); A.PivotOffset = CFrame.new(0, 3, 0)
        local B = Instance.new("Part"); B.Anchored = true; B.CFrame = CFrame.new(5, 0, 0); B.Parent = A
        local F = Instance.new("Folder"); F.Parent = A
        local C = Instance.new("Part"); C.Anchored = true; C.CFrame = CFrame.new(9, 0, 0); C.Parent = F
        local Att = Instance.new("Attachment"); Att.Parent = A
        local D = Instance.new("Part"); D.Anchored = true; D.CFrame = CFrame.new(13, 0, 0); D.Parent = Att
        local M = Instance.new("Model"); M.Parent = A
        local E = Instance.new("Part"); E.Anchored = true; E.CFrame = CFrame.new(17, 0, 0); E.Parent = M
        A.Parent = workspace
        A:PivotTo(CFrame.new(100, 0, 0) * CFrame.Angles(0, math.rad(90), 0))
        assert(at(A, 100, -3, 0) and (A:GetPivot().Position - Vector3.new(100, 0, 0)).Magnitude < 1e-3, "the part")
        assert(at(B, 100, -3, -4), "a part under it")
        assert(at(C, 100, -3, -8), "through a Folder")
        assert(at(D, 100, -3, -12), "through an Attachment")
        assert(at(E, 100, -3, -16), "through a Model")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// GetTagged lists the tagged instances of the DataModel in GetDescendants' order, whatever gave
/// them their tags (AddTag, Clone) and however they came and went since.
#[test]
fn get_tagged_follows_the_data_model() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local CollectionService = game:GetService("CollectionService")
        local A = Instance.new("Folder")
        A.Name = "A"
        A.Parent = workspace
        local B = Instance.new("Part")
        B.Name = "B"
        B.Parent = A
        local C = Instance.new("Part")
        C.Name = "C"
        C.Parent = workspace
        CollectionService:AddTag(C, "T")
        CollectionService:AddTag(B, "T")
        local Outside = Instance.new("Part")
        Outside.Name = "Outside"
        CollectionService:AddTag(Outside, "T")
        local Copy = A:Clone()
        Copy.Name = "Copy"
        Copy.Parent = workspace
        local function Names()
            local List = {}
            for _, Tagged in CollectionService:GetTagged("T") do
                table.insert(List, Tagged:GetFullName())
            end
            return table.concat(List, " ")
        end
        print(Names())
        CollectionService:RemoveTag(C, "T")
        B:Destroy()
        print(Names())
        Outside.Parent = Copy
        A.Parent = nil
        Copy.Parent = A
        A.Parent = workspace
        print(Names(), #CollectionService:GetTagged("Other"))
        "#,
    );
    assert_eq!(output, ["print: Workspace.A.B Workspace.C Workspace.Copy.B", "print: Workspace.Copy.B", "print: Workspace.A.Copy.B Workspace.A.Copy.Outside 0"]);
}

/// A recursive FindFirstChild / FindFirstChildWhichIsA over a lazy import finds what it finds over
/// an eager one (the same instance, in the same search order), and makes nothing it only passes
/// over: a search that finds nothing in a bundle makes none of it.
#[test]
fn a_recursive_find_in_a_lazy_import_finds_the_same_and_makes_nothing_else() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    for index in 0..3 {
        let model = dom.insert(root, InstanceBuilder::new("Model").with_name(format!("Unit{index}")));
        let torso = dom.insert(model, InstanceBuilder::new("Part").with_name("Torso"));
        let arm = dom.insert(torso, InstanceBuilder::new("Part").with_name("Arm"));
        dom.insert(arm, InstanceBuilder::new("Attachment").with_name(if index == 1 { "Grip" } else { "Hold" }));
        dom.insert(model, InstanceBuilder::new("Folder").with_name("Grip"));
        let deep = dom.insert(model, InstanceBuilder::new("Folder").with_name("Deep"));
        dom.insert(deep, InstanceBuilder::new("Animation").with_name(format!("Clip{index}")));
    }
    let plan = std::sync::Arc::new(super::project::ModelPlan::new(&super::Reflection::shared().unwrap(), &dom, dom.root().children()));
    let check = r#"
        local Folder = game:GetService("ReplicatedStorage").Bundle
        local function Name(Item) return if Item then Item:GetFullName() .. " " .. Item:GetDebugId() else "nil" end
        print("miss", Name(Folder:FindFirstChild("Nothing", true)), Name(Folder:FindFirstChildWhichIsA("Sound", true)))
        print(Name(Folder:FindFirstChild("Grip", true)))
        print(Name(Folder:FindFirstChild("Clip2", true)))
        print(Name(Folder.Unit1:FindFirstChild("Grip", true)))
        print(Name(Folder:FindFirstChildWhichIsA("Attachment", true)))
        print(Name(Folder:FindFirstChildWhichIsA("Animation", true)))
        Folder.Unit0.Torso.Arm.Hold.Name = "Grip"
        print(Name(Folder.Unit0.Torso:FindFirstChild("Grip", true)))
        print(#Folder:GetDescendants())
    "#;
    let miss = r#"
        local Folder = game:GetService("ReplicatedStorage").Bundle
        print(Folder:FindFirstChild("Nothing", true), Folder:FindFirstChildWhichIsA("Sound", true))
    "#;
    let mut outputs = Vec::new();
    for lazy in [false, true] {
        for source in [check, miss] {
            let (lua, engine) = start();
            let folder = {
                let mut state = engine.state.borrow_mut();
                let class = engine.reflection.class_id("Folder").unwrap();
                let folder = super::instance::create_named(&mut state, &engine.reflection, class, "Bundle");
                let storage = state.dom.find_child(engine.world().game, b"ReplicatedStorage").unwrap();
                state.dom.attach(folder, storage);
                folder
            };
            if lazy {
                plan.instantiate_lazy(&mut engine.state.borrow_mut(), &engine.reflection, folder);
            } else {
                plan.instantiate(&engine, Some(folder));
            }
            let made = engine.state.borrow().project.lazy_made;
            outputs.push(run(&lua, &engine, source));
            if lazy && source == miss {
                assert_eq!(engine.state.borrow().project.lazy_made, made, "a search that finds nothing makes nothing");
            }
            drop(lua);
            engine.teardown();
        }
    }
    assert_eq!(outputs[0], outputs[2]);
    assert_eq!(outputs[1], outputs[3]);
    assert!(outputs[0].iter().any(|line| line.contains("Bundle.Unit0.Grip")), "{:?}", outputs[0]);
}

/// EditableMesh as measured in Studio (2026-10-16): kind-tagged ids, AddTriangle's own corner
/// attributes with an automatic face normal, RemoveUnused's removed ids, 1/255 bone weights,
/// clones keeping ids, and MeshParts that keep MeshSize from when they were made (ApplyMesh copies
/// it and keeps Size) while pointing at snapshots of the mesh.
#[test]
fn editable_meshes_follow_studio() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local m = AssetService:CreateEditableMesh()
        local a = m:AddVertex(Vector3.new(10, 0, 0))
        local b = m:AddVertex(Vector3.new(12, 0, 0))
        local c = m:AddVertex(Vector3.new(10, 0, 4))
        local f = m:AddTriangle(a, c, b)
        assert(a == 4294967297 and b == 4294967298 and f == 21474836481, "kind-tagged ids")
        assert(#m:GetFaceUVs(f) == 3 and #m:GetNormals() == 3 and #m:GetColors() == 3 and #m:GetUVs() == 3, "corner attributes")
        assert(m:GetNormal(m:GetFaceNormals(f)[1]) == Vector3.new(0, 1, 0), "the face's normal")
        assert(m:GetColorAlpha(m:GetFaceColors(f)[1]) == 1 and m:GetUV(m:GetFaceUVs(f)[1]) == Vector2.zero, "white, (0, 0)")
        assert(m:GetNormals()[1] == 8589934593 and m:GetColors()[1] == 12884901889 and m:GetUVs()[1] == 17179869185, "kinds")
        local bone = m:AddBone({ Name = "Root", CFrame = CFrame.new(1, 2, 3), Virtual = false })
        assert(bone == 25769803777 and m:GetBoneName(bone) == "Root" and m:GetBoneParent(bone) == 0, "bones")
        m:SetVertexBones(a, { bone })
        m:SetVertexBoneWeights(a, { 0.25 })
        assert(math.abs(m:GetVertexBoneWeights(a)[1] - 64 / 255) < 1e-6, "weights in 1/255 steps")
        local stray = m:AddVertex(Vector3.new(9, 9, 9))
        local removed = m:RemoveUnused()
        assert(#removed == 1 and removed[1] == stray and #m:GetVertices() == 3, "RemoveUnused")
        local fixed = AssetService:CreateEditableMeshAsync(Content.fromObject(m), { FixedSize = true })
        assert(fixed.FixedSize and fixed:GetVertices()[2] == b and #fixed:GetFaces() == 1, "a clone keeps ids")
        assert(not pcall(function() fixed:AddVertex(Vector3.zero) end), "a fixed size mesh takes no vertices")
        local part = AssetService:CreateMeshPartAsync(Content.fromObject(m))
        assert(part.MeshSize == Vector3.new(2, 0.001, 4) and part.Size == part.MeshSize, "MeshSize: the box, at least 0.001")
        assert(string.find(part.MeshId, "^file://"), "a snapshot: " .. part.MeshId)
        m:BatchSetValues({ b }, { Vector3.new(20, 0, 0) })
        assert(m:GetPosition(b) == Vector3.new(20, 0, 0) and part.MeshSize == Vector3.new(2, 0.001, 4), "MeshSize stays")
        local other = Instance.new("MeshPart")
        other.Size = Vector3.new(1, 1, 1)
        other:ApplyMesh(part)
        assert(other.MeshSize == part.MeshSize and other.Size == Vector3.new(1, 1, 1) and other.MeshId == part.MeshId, "ApplyMesh")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// A new face's corner takes the normal, uv and colour the newest face over its vertex uses there, and an automatic normal
/// several corners share sums their faces' unit normals weighed by the square of the sine of each corner's angle (measured
/// in a 0.747 client: tools/parity/probes/editable_mesh_batch.luau; the values are that client's to 3e-7). Before, every
/// AddTriangle corner had attributes of its own, and a shared normal was area-weighted.
#[test]
fn editable_mesh_faces_share_their_vertices_corners() {
    assert_script(
        r#"
        local AssetService = game:GetService("AssetService")
        local function near(a, b) return (a - b).Magnitude < 1e-6 end
        local m = AssetService:CreateEditableMesh()
        local v = {}
        for _, p in { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(0, 0, 1), Vector3.new(0, 1, 0) } do
            table.insert(v, m:AddVertex(p))
        end
        local a = m:AddTriangle(v[1], v[2], v[3])
        local b = m:AddTriangle(v[1], v[3], v[4])
        local na, nb = m:GetFaceNormals(a), m:GetFaceNormals(b)
        assert(#m:GetNormals() == 4 and #m:GetUVs() == 4 and #m:GetColors() == 4, "4 corners' worth: " .. #m:GetNormals())
        assert(nb[1] == na[1] and nb[2] == na[3] and m:GetFaceUVs(b)[2] == m:GetFaceUVs(a)[3] and m:GetFaceColors(b)[1] == m:GetFaceColors(a)[1], "shared")
        assert(near(m:GetNormal(nb[1]), Vector3.new(0.632455528, -0.632455528, 0.44721359)), tostring(m:GetNormal(nb[1])))
        assert(near(m:GetNormal(nb[2]), Vector3.new(0.392232329, -0.392232329, 0.832050323)), tostring(m:GetNormal(nb[2])))
        assert(near(m:GetNormal(nb[3]), Vector3.new(0.707106769, -0.707106769, 0)), tostring(m:GetNormal(nb[3])))
        -- the newest face's corner, a normal set on it included
        local explicit = m:AddNormal(Vector3.new(1, 0, 0))
        m:SetFaceNormals(b, { explicit, nb[2], nb[3] })
        local c = m:AddTriangle(v[1], v[5], v[3])
        assert(m:GetFaceNormals(c)[1] == explicit and m:GetFaceNormals(c)[3] == nb[2], "the newest face's")
        -- a vertex no face uses any more gets new ones
        local fresh = AssetService:CreateEditableMesh()
        local p, q, r = fresh:AddVertex(Vector3.zero), fresh:AddVertex(Vector3.xAxis), fresh:AddVertex(Vector3.yAxis)
        fresh:RemoveFace(fresh:AddTriangle(p, q, r))
        local again = fresh:AddTriangle(p, q, r)
        assert(#fresh:GetNormals() == 6 and fresh:GetFaceNormals(again)[1] == 8589934596, "new after the face went")
        "#,
    );
}

/// CreateMeshPartAsync refuses an EditableMesh whose box is wider than 2048 studs on any axis with Studio's error and
/// makes no part (Studio 0.748, a play-test client: tools/parity/probes/mesh_extents.luau): the box is every vertex's, a
/// face's or not, wherever it lies, and 2048 itself passes.
#[test]
fn mesh_parts_refuse_a_mesh_past_2048_studs() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        -- a mesh whose box runs from `low` to `low + size`: a triangle at its centre, and each far corner a zero-area
        -- triangle or, `loose`, a vertex no face uses
        local function make(low, size, loose)
            local m = AssetService:CreateEditableMesh()
            local centre = low + size / 2
            m:AddTriangle(m:AddVertex(centre + Vector3.new(-4, 0, -4)), m:AddVertex(centre + Vector3.new(0, 0, 4)), m:AddVertex(centre + Vector3.new(4, 0, -4)))
            for _, corner in { low, low + size } do
                if loose then
                    m:AddVertex(corner)
                else
                    m:AddTriangle(m:AddVertex(corner), m:AddVertex(corner), m:AddVertex(corner))
                end
            end
            return pcall(function()
                return AssetService:CreateMeshPartAsync(Content.fromObject(m), { CollisionFidelity = Enum.CollisionFidelity.Box, RenderFidelity = Enum.RenderFidelity.Precise })
            end)
        end
        local function refused(low, size, loose)
            local ok, problem = make(low, size, loose)
            assert(not ok and problem == "Mesh extents size too large. Maximum is (2048, 2048, 2048).", tostring(size) .. ": " .. tostring(problem))
        end
        local function made(low, size)
            local ok, part = make(low, size)
            assert(ok and part.ClassName == "MeshPart" and part.MeshSize == size and part.Size == size, tostring(size) .. ": " .. tostring(part))
        end
        local function centred(size)
            return -size / 2, size
        end
        -- about the origin, and each axis alone
        refused(centred(Vector3.new(2112, 520, 2112)))
        made(centred(Vector3.new(2048, 520, 2048)))
        for _, axis in { Vector3.xAxis, Vector3.yAxis, Vector3.zAxis } do
            local rest = (Vector3.one - axis) * 520
            refused(centred(rest + axis * 2112))
            made(centred(rest + axis * 2048))
        end
        refused(centred(Vector3.new(2048.01, 520, 520)))
        -- off the origin: the box's width counts, not how far its vertices are
        made(Vector3.new(5000, -50, -50), Vector3.new(100, 100, 100))
        made(Vector3.new(0, -260, -260), Vector3.new(2048, 520, 520))
        refused(Vector3.new(0, -260, -260), Vector3.new(2112, 520, 520))
        -- the far corners as vertices no face uses
        local low, size = centred(Vector3.new(2112, 520, 520))
        refused(low, size, true)
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// EditableMesh:BatchAdd as a 0.747 client answers it (tools/parity/probes/editable_mesh_batch.luau): the new ids in the
/// values' order; colours with their alphas; faces numbered backwards (the first face the batch's last id), listed by
/// GetFaces in the order given, their corners as AddTriangle makes them; a value of the wrong type raises at the caller
/// with the values before it set and the rest made; a bad face makes none; the client's errors.
#[test]
fn editable_mesh_batch_add_follows_a_client() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local m = AssetService:CreateEditableMesh()
        local v = m:BatchAdd(Enum.MeshAttribute.Vertex, { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(1, 1, 0) })
        assert(#v == 4 and v[1] == 4294967297 and v[4] == 4294967300 and m:GetPosition(v[2]) == Vector3.new(1, 0, 0), "vertices")
        local n = m:BatchAdd(Enum.MeshAttribute.Normal, { Vector3.new(0, 0, 1), nil, Vector3.new(0, 1, 0) })
        assert(#n == 3 and n[1] == 8589934593 and m:GetNormal(n[1]) == Vector3.new(0, 0, 1), "normals, a hole automatic")
        local u = m:BatchAdd(Enum.MeshAttribute.UV, { Vector2.new(0.5, 0.25) })
        assert(u[1] == 17179869185 and m:GetUV(u[1]) == Vector2.new(0.5, 0.25), "uvs")
        local c = m:BatchAdd(Enum.MeshAttribute.Color, { Color3.new(1, 0, 0), Color3.new(0, 0, 1) }, { 0.5, 2 })
        assert(c[1] == 12884901889 and m:GetColor(c[1]) == Color3.new(1, 0, 0), "colours")
        assert(math.abs(m:GetColorAlpha(c[1]) - 128 / 255) < 1e-6 and m:GetColorAlpha(c[2]) == 1, "alphas in 1/255 steps, clamped")
        local f = m:BatchAdd(Enum.MeshAttribute.Face, { { v[1], v[2], v[3] }, { v[2], v[4], v[3] } })
        assert(f[1] == 21474836482 and f[2] == 21474836481, "faces numbered backwards")
        local listed = m:GetFaces()
        assert(listed[1] == f[1] and listed[2] == f[2], "listed in the order given")
        assert(m:GetFaceNormals(f[2])[1] == m:GetFaceNormals(f[1])[2], "corners shared over a vertex")
        assert(#m:BatchAdd(Enum.MeshAttribute.Vertex, {}) == 0, "none")
        local function fails(f, ...) local ok, e = pcall(f, ...) assert(not ok) print("error", e) end
        local before = #m:GetVertices()
        fails(function() m:BatchAdd(Enum.MeshAttribute.Vertex, { Vector3.new(5, 5, 5), "x", Vector3.new(6, 6, 6) }) end)
        local after = m:GetVertices()
        assert(#after == before + 3 and m:GetPosition(after[before + 1]) == Vector3.new(5, 5, 5) and m:GetPosition(after[before + 3]) == Vector3.zero, "made, then set in order")
        local faces = #m:GetFaces()
        fails(function() m:BatchAdd(Enum.MeshAttribute.Face, { { v[1], v[2], v[3] }, { v[1], v[2] } }) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.Face, { { v[1], v[2], n[1] } }) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.Face, { { v[1], v[2], 12345 } }) end)
        assert(#m:GetFaces() == faces, "a bad face makes none")
        fails(function() m:BatchAdd(Enum.MeshAttribute.Color, { Color3.new() }, {}) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.Color, { Color3.new() }) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.Vertex, {}, {}) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.UV, 5) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.Face, 5) end)
        fails(function() m:BatchAdd("Vertex", {}) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.Vertex) end)
        fails(function() m:BatchAdd(Enum.MeshAttribute.UV, { Vector3.new() }) end)
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    let errors: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("print: error ")).collect();
    let wanted = [
        "Test:21: invalid argument #-1 (Vector3 expected, got string)",
        "BatchAdd(Enum.MeshAttribute.Face, ...): element 2 must have 3 or 4 vertex ids, got 2",
        "Unexpected EditableMesh id type. Expected Vertex id, received Normal id",
        "Unexpected EditableMesh id type. Expected Vertex id, received value that doesn't appear to be an id",
        "BatchAdd(Enum.MeshAttribute.Color, ...): colors and alphas arrays must be the same length",
        "BatchAdd(Enum.MeshAttribute.Color, ...): expects 2 arguments (colors, alphas)",
        "BatchAdd(Enum.MeshAttribute.Vertex, ...): expects 1 argument (positions)",
        "BatchAdd(Enum.MeshAttribute.UV, ...): uvs must be an array",
        "BatchAdd(Enum.MeshAttribute.Face, ...): vertexIds must be a 2D array",
        "Invalid value for enum 'MeshAttribute'",
        "BatchAdd: expects at least 2 arguments (attr, data)",
        "Test:36: invalid argument #4 (Vector2 expected, got Vector3)",
    ];
    assert_eq!(errors, wanted, "{output:?}");
}

/// EditableMesh's per-vertex batches as a 0.747 client answers them (tools/parity/probes/editable_mesh_vertex_batch.luau,
/// public issue 18: all three answered nil, a fixed-size copy's included): BatchGetVertexAttributes lists per vertex one
/// entry per corner over it, the newest corner first; BatchGetVertexFaceAttributes reads one corner each; and
/// BatchSetVertexFaceAttributes sets corners in turn (an error leaves the ones before it set) and brings a changed corner
/// to the front, as the single-item methods do.
#[test]
fn editable_mesh_vertex_batches_follow_a_client() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local function show(t) local out = {} for _, x in t do table.insert(out, if type(x) == "table" then "{" .. show(x) .. "}" else string.format("%d:%d", x // 4294967296, x % 4294967296)) end return table.concat(out, ",") end
        local m = AssetService:CreateEditableMesh()
        local v = {}
        for _, p in { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1), Vector3.new(2, 2, 2) } do
            table.insert(v, m:AddVertex(p))
        end
        local a, b, c = m:AddTriangle(v[1], v[2], v[3]), m:AddTriangle(v[1], v[3], v[4]), m:AddTriangle(v[2], v[3], v[5])
        local n = m:AddNormal(Vector3.xAxis)
        local old = m:GetFaceNormals(b)
        m:SetFaceNormals(b, { n, old[2], old[3] })
        print("normals", show(m:BatchGetVertexAttributes(Enum.MeshAttribute.Normal, v)))
        print("faces", show(m:BatchGetVertexAttributes(Enum.MeshAttribute.Face, v)))
        print("single", show(m:GetVertexNormals(v[2])), show(m:GetVertexFaces(v[3])))
        local copy = AssetService:CreateEditableMeshAsync(Content.fromObject(m))
        print("fixed copy", copy.FixedSize, show(copy:BatchGetVertexAttributes(Enum.MeshAttribute.UV, { v[1], v[6] })))
        print("corners", show(m:BatchGetVertexFaceAttributes(Enum.MeshAttribute.Normal, { v[1], v[1], v[3] }, { a, b, c })), show({ m:GetVertexFaceNormal(v[1], b) }))
        -- set in turn: the first pair is set when the second fails
        local z = m:AddNormal(Vector3.zAxis)
        print("set", pcall(m.BatchSetVertexFaceAttributes, m, { v[2], v[5] }, { a, a }, { z, z }))
        print("after", show(m:GetFaceNormals(a)), show(m:BatchGetVertexAttributes(Enum.MeshAttribute.Face, { v[1] })))
        m:SetVertexFaceUV(v[3], b, m:AddUV(Vector2.one))
        print("moved", show(m:GetVertexFaces(v[3])))
        print("adjacent", show(m:GetAdjacentVertices(v[1])), show(m:GetAdjacentVertices(v[3])))
        for _, call in {
            function() return m:BatchGetVertexAttributes(Enum.MeshAttribute.Vertex, v) end,
            function() return m:BatchGetVertexAttributes(Enum.MeshAttribute.Normal, { v[1], 12345 }) end,
            function() return m:BatchGetVertexAttributes(Enum.MeshAttribute.Normal, { "x" }) end,
            function() return m:BatchGetVertexAttributes(Enum.KeyCode.A, v) end,
            function() return m:BatchGetVertexFaceAttributes(Enum.MeshAttribute.Normal, { v[5] }, { a }) end,
            function() return m:BatchGetVertexFaceAttributes(Enum.MeshAttribute.Normal, { v[1] }, {}) end,
            function() return m:BatchSetVertexFaceAttributes({ v[1] }, { a }, { v[2] }) end,
            function() return m:GetVertexFaceNormal(v[5], a) end,
        } do
            print("error", select(2, pcall(call)))
        end
        "#,
    );
    let lines: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("print: ")).collect();
    let wanted = [
        "normals {2:6,2:1},{2:2,2:2},{2:3,2:3,2:3},{2:4},{2:5},{}",
        "faces {5:2,5:1},{5:3,5:1},{5:3,5:2,5:1},{5:2},{5:3},{}",
        "single 2:2,2:2 5:3,5:2,5:1",
        "fixed copy true {4:1,4:1},{}",
        "corners 2:1,2:6,2:3 2:6",
        "set false BatchSetVertexFaceAttributes: vertexId not found on requested face at index 2",
        "after 2:1,2:7,2:3 {5:2,5:1}",
        "moved 5:2,5:3,5:1",
        "adjacent 1:4,1:3,1:2 1:1,1:4,1:2,1:5,1:2",
        "error BatchGetVertexAttributes: Enum.MeshAttribute.Vertex is not supported",
        "error Unexpected EditableMesh id type. Expected Vertex id, received value that doesn't appear to be an id",
        "error BatchGetVertexAttributes: id at index 1 is not a number",
        "error Invalid type 'KeyCode' for enum 'MeshAttribute'",
        "error BatchGetVertexFaceAttributes: vertexId not found on requested face at index 1",
        "error BatchGetVertexFaceAttributes: vertexIds and faceIds arrays must be the same length",
        "error BatchSetVertexFaceAttributes: unsupported attribute ID type 1",
        "error vertexId not found on requested face",
    ];
    assert_eq!(lines, wanted, "{output:?}");
}

/// BatchRemove and the slots removed elements leave, as a 0.747 client has them (editable_mesh_vertex_batch.luau): it
/// removes faces only, every id checked first; what they used stays until RemoveUnused; a freed slot comes back, the last
/// freed first, under its next generation (`13:3`), so the old id is gone; the lists are dense arrays a removed item's
/// place in is taken by the last one. Before, BatchRemove did nothing and ids were never reused.
#[test]
fn editable_mesh_batch_remove_frees_slots_as_a_client() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local function show(t) local out = {} for _, x in t do table.insert(out, string.format("%d:%d", x // 4294967296, x % 4294967296)) end return table.concat(out, ",") end
        local function counts(m) return #m:GetVertices() .. "v " .. #m:GetNormals() .. "n " .. #m:GetFaces() .. "f" end
        local m = AssetService:CreateEditableMesh()
        local v = {}
        for _, p in { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1), Vector3.new(2, 2, 2) } do
            table.insert(v, m:AddVertex(p))
        end
        local f = { m:AddTriangle(v[1], v[2], v[3]), m:AddTriangle(v[1], v[3], v[4]), m:AddTriangle(v[2], v[3], v[5]) }
        print("bad", pcall(m.BatchRemove, m, { f[1], v[6] }))
        print("kept", counts(m))
        print("removed", m:BatchRemove({ f[3], f[1], f[1] }), counts(m), show(m:GetFaces()))
        print("again", pcall(m.BatchRemove, m, { f[1] }))
        local g = { m:AddTriangle(v[1], v[2], v[3]), m:AddTriangle(v[2], v[3], v[5]), m:AddTriangle(v[4], v[5], v[6]) }
        print("reused", show(g), show(m:GetFaces()))
        print("stale", pcall(m.GetFaceVertices, m, f[3]))
        local batch = m:BatchAdd(Enum.MeshAttribute.Face, { { v[1], v[2], v[4] } })
        m:RemoveFace(g[1])
        print("batch", show(batch), show(m:BatchAdd(Enum.MeshAttribute.Face, { { v[1], v[2], v[3] }, { v[1], v[4], v[5] } })))
        m:BatchRemove(m:GetFaces())
        local removed = m:RemoveUnused()
        local w = { m:AddVertex(Vector3.one), m:AddVertex(Vector3.zero) }
        print("unused", #removed, show({ removed[1], removed[6], removed[7] }), counts(m), show(w))
        local plain = AssetService:CreateEditableMesh({ FixedSize = true })
        print("options", plain.FixedSize, AssetService:CreateEditableMeshAsync(Content.fromObject(plain), { FixedSize = false }).FixedSize)
        local fixed = AssetService:CreateEditableMeshAsync(Content.fromObject(plain))
        for _, call in {
            function() return fixed:BatchRemove({}) end,
            function() return fixed:AddNormal(Vector3.one) end,
            function() return fixed:RemoveUnused() end,
            function() return m:RemoveFace(0) end,
            function() return m:AddTriangle(w[1], w[2], v[6]) end,
            function() return m:AddBone({ Name = "b", CFrame = CFrame.new() }) end,
        } do
            print("error", select(2, pcall(call)))
        end
        "#,
    );
    let lines: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("print: ")).collect();
    let wanted = [
        "bad false Unexpected EditableMesh id type. Expected Face id, received Vertex id",
        "kept 6v 5n 3f",
        "removed nil 6v 5n 1f 5:2",
        "again false Invalid faceId",
        "reused 13:1,13:3,5:4 5:2,13:1,13:3,5:4",
        "stale false Invalid id",
        "batch 5:5 5:6,21:1",
        "unused 30 1:1,1:6,2:1 2v 0n 0f 9:6,9:5",
        "options false false",
        "error EditableMesh is fixed-size, adding or removing elements is not allowed in-experience",
        "error EditableMesh is fixed-size, adding or removing elements is not allowed in-experience",
        "error EditableMesh is fixed-size, adding or removing elements is not allowed in-experience",
        "error Unexpected EditableMesh id type. Expected Face id, received value that doesn't appear to be an id",
        "error Unable to add new triangle, vertex id 4294967302 is invalid",
        "error Missing bone Virtual value",
    ];
    assert_eq!(lines, wanted, "{output:?}");
}

/// CreateMeshPartAsync of a mesh file the project holds, as a game's asset id modules can name one until
/// it is uploaded: its repo-relative path and a relative file:// URI are read from the project's folder
/// (Options::project), whatever the current directory, and give the mesh's size as an absolute URI
/// does; content it cannot load raises Studio's error (measured 2026-11-03: "Failed to load mesh
/// asset.", no position), never a 1x1x1 part
#[cfg(feature = "meshes")]
#[test]
fn mesh_parts_from_local_files_read_the_project() {
    let project = std::env::temp_dir().join(format!("ree-local-meshes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(project.join("output").join("meshes")).unwrap();
    let (lua, engine) = start_with(Options { project: Some(project.clone()), ..Options::default() });
    // the mesh file: an EditableMesh's snapshot, copied into the project
    let made = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local m = AssetService:CreateEditableMesh()
        m:AddTriangle(m:AddVertex(Vector3.zero), m:AddVertex(Vector3.new(3, 0, 0)), m:AddVertex(Vector3.new(0, 2, 1)))
        print(AssetService:CreateMeshPartAsync(Content.fromObject(m)).MeshId)
        "#,
    );
    let snapshot = made.iter().find_map(|line| line.strip_prefix("print: file://")).expect("a snapshot").to_string();
    let file = project.join("output").join("meshes").join("Box.mesh");
    std::fs::copy(&snapshot, &file).unwrap();
    let absolute = file.to_string_lossy().replace('\\', "/");
    let output = run(
        &lua,
        &engine,
        &format!(
            r#"
            local AssetService = game:GetService("AssetService")
            for _, uri in {{ "output/meshes/Box.mesh", "file://output/meshes/Box.mesh", "file://{absolute}", "file:///{absolute}" }} do
                local part = AssetService:CreateMeshPartAsync(Content.fromUri(uri))
                assert(part.MeshSize == Vector3.new(3, 2, 1) and part.Size == part.MeshSize, uri .. ": " .. tostring(part.MeshSize))
                assert(part.MeshId == uri, "the content as given: " .. part.MeshId)
            end
            for _, uri in {{ "output/meshes/Missing.mesh", "file://output/meshes/Missing.mesh", "rbxasset://fonts/none.mesh" }} do
                local ok, message = pcall(function() return AssetService:CreateMeshPartAsync(Content.fromUri(uri)) end)
                assert(not ok and string.find(message, "Failed to load mesh asset. ", 1, true) == 1, uri .. ": " .. tostring(message))
            end
            print("ok")
            "#
        ),
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");

    // what names a local file, by ror's rules
    let options = &engine.options;
    assert_eq!(options.local_file("output/meshes/Box.mesh"), Some(project.join("output/meshes/Box.mesh")));
    assert_eq!(options.local_file(" FILE://output/x.png "), Some(project.join("output/x.png")));
    if cfg!(windows) {
        assert_eq!(options.local_file("file:///C:/x%20y/a.mesh"), Some(std::path::PathBuf::from("C:/x y/a.mesh")));
    }
    assert_eq!(options.local_file("output\\meshes\\Missing.mesh"), Some(project.join("output\\meshes\\Missing.mesh")), "a path, found or not");
    std::fs::write(project.join("Loose.mesh"), b"").unwrap();
    assert_eq!(options.local_file("Loose.mesh"), Some(project.join("Loose.mesh")), "a file the project holds");
    for uri in ["Nothing.mesh", "rbxassetid://123", "123", "http://www.roblox.com/asset/?id=123", "www.roblox.com/asset/?id=123", "rbxasset://textures/x.png", "rbxthumb://type=Asset&id=1&w=150&h=150", "file://", ""] {
        assert_eq!(options.local_file(uri), None, "{uri}");
    }
    let _ = std::fs::remove_dir_all(&project);
}

/// A mesh content id read as Studio 0.747's client reads it (2026-11-24, CreateMeshPartAsync of an asset whose only
/// version is 1; crate::content_id::asset): `rbxassetid://N&version=V` is version V, which comes from its own
/// cache file (`<id>-v<version>`), never the latest's; `?version=V`, a slash after the id, the scheme in capitals,
/// whitespace before it and bare digits load nothing; and CreateEditableMeshAsync raises for an id of no protocol
#[cfg(feature = "meshes")]
#[test]
fn pinned_mesh_versions_load_their_own_version() {
    let repo = std::env::temp_dir().join(format!("ree-pinned-meshes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let (lua, engine) = start_with(Options { repo: repo.clone(), ..Options::default() });
    let made = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        for _, size in { Vector3.new(3, 2, 1), Vector3.new(5, 4, 1) } do
            local m = AssetService:CreateEditableMesh()
            m:AddTriangle(m:AddVertex(Vector3.zero), m:AddVertex(Vector3.new(size.X, 0, 0)), m:AddVertex(Vector3.new(0, size.Y, size.Z)))
            print(AssetService:CreateMeshPartAsync(Content.fromObject(m)).MeshId)
        end
        "#,
    );
    let snapshots: Vec<&str> = made.iter().filter_map(|line| line.strip_prefix("print: file://")).collect();
    assert_eq!(snapshots.len(), 2, "{made:?}");
    let cache = crate::state_dir(&repo).join("meshes");
    std::fs::create_dir_all(&cache).unwrap();
    // the latest version is the 3 x 2 x 1 mesh, version 2 the 5 x 4 x 1 one
    std::fs::copy(snapshots[0], cache.join("4747")).unwrap();
    std::fs::copy(snapshots[1], cache.join("4747-v2")).unwrap();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local Latest, Second = Vector3.new(3, 2, 1), Vector3.new(5, 4, 1)
        for uri, size in {
            ["rbxassetid://4747"] = Latest,
            ["rbxassetid://4747&version=2"] = Second,
            ["rbxassetid://4747&Version=2&foo=bar"] = Second,
            ["rbxassetid://4747&version=%32"] = Second,
            ["rbxassetid:// 4747 &version=2"] = Second,
            ["rbxassetid://4747&version=2&version=abc&version=1"] = Second,
            ["rbxassetid://4747&version=0"] = Latest,
            ["rbxassetid://4747&version=abc&version=2"] = Latest,
            ["rbxassetid://9&id=4747&version=2"] = Second,
            ["http://www.roblox.com/asset/?id=4747&version=2"] = Second,
            ["rbxassetid://04747 "] = Latest,
        } do
            local part = AssetService:CreateMeshPartAsync(Content.fromUri(uri))
            assert(part.MeshSize == size, uri .. ": " .. tostring(part.MeshSize))
            assert(part.MeshId == uri, "the content as given: " .. part.MeshId)
        end
        for _, uri in {
            "rbxassetid://4747?version=2", "rbxassetid://4747/", "rbxassetid://4747/&version=2", "rbxassetid://4747&version=2#x",
            "rbxassetid://4747&assetversionid=2", "RBXASSETID://4747", " rbxassetid://4747", "4747", "www.roblox.com/asset/?id=4747",
            -- (a version not in the cache: this engine downloads nothing, and the latest's file is not it)
            "rbxassetid://4747&version=3",
        } do
            local ok, message = pcall(function() return AssetService:CreateMeshPartAsync(Content.fromUri(uri)) end)
            assert(not ok and string.find(message, "Failed to load mesh asset. ", 1, true) == 1, uri .. ": " .. tostring(message))
        end
        local mesh = AssetService:CreateEditableMeshAsync(Content.fromUri("rbxassetid://4747&version=2"))
        local top = 0
        for _, v in mesh:GetVertices() do top = math.max(top, mesh:GetPosition(v).X) end
        assert(top == 5, "the EditableMesh of version 2: " .. top)
        for _, uri in { " rbxassetid://4747", "RBXASSETID://4747", "4747" } do
            local ok, message = pcall(function() return AssetService:CreateEditableMeshAsync(Content.fromUri(uri)) end)
            assert(not ok and message == "Failed to load mesh " .. uri, uri .. ": " .. tostring(message))
        end
        print("ok")
        "#,
    );
    let _ = std::fs::remove_dir_all(&repo);
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// HasSkinnedMesh as measured in Studio (2026-10-17): a part made from an EditableMesh is skinned
/// when some vertex weighs a bone (bones no vertex weighs leave it false), ApplyMesh copies it either
/// way, and a fresh MeshPart is not
#[test]
fn mesh_parts_from_skinned_meshes_are_skinned() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local function build(bones, weights)
            local m = AssetService:CreateEditableMesh()
            local root = bones and m:AddBone({ Name = "R", CFrame = CFrame.identity, Virtual = false })
            local a, b, c = m:AddVertex(Vector3.zero), m:AddVertex(Vector3.new(4, 0, 0)), m:AddVertex(Vector3.new(0, 4, 0))
            if weights then
                for _, v in { a, b, c } do m:SetVertexBones(v, { root }); m:SetVertexBoneWeights(v, { 1 }) end
            end
            m:AddTriangle(a, b, c)
            return AssetService:CreateMeshPartAsync(Content.fromObject(m))
        end
        local plain, boneless, skinned = build(false, false), build(true, false), build(true, true)
        assert(not plain.HasSkinnedMesh and not boneless.HasSkinnedMesh and skinned.HasSkinnedMesh, "made")
        local fresh = Instance.new("MeshPart")
        assert(not fresh.HasSkinnedMesh, "fresh")
        fresh:ApplyMesh(skinned)
        assert(fresh.HasSkinnedMesh, "applied skinned")
        fresh:ApplyMesh(plain)
        assert(not fresh.HasSkinnedMesh, "applied plain")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// ApplyMesh from a part a model file holds gives the mesh's size the file keeps as InitialSize, as
/// MeshSize reads it (an effect that applies baked portal masks from its bundle: the masks lost their
/// InitialSize, and ror scaled their 0.000006-stud-thin mesh by its own bounds, opening the portal's
/// window far wider than Studio shows it)
#[test]
fn apply_mesh_from_a_file_part_keeps_its_size() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom, types::Vector3};
    let dir = std::env::temp_dir().join(format!("ree-apply-mesh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Baked.rbxm");
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let part = dom.insert(
        root,
        InstanceBuilder::new("MeshPart")
            .with_name("Baked")
            .with_property("MeshId", rbx_dom_weak::types::ContentId::from("rbxassetid://23456789012345"))
            .with_property("InitialSize", Vector3::new(26.0, 54.0, 0.001))
            .with_property("Size", Vector3::new(13.0, 27.0, 0.001)),
    );
    let mut bytes = Vec::new();
    rbx_binary::to_writer(&mut bytes, &dom, &[part]).unwrap();
    std::fs::write(&file, bytes).unwrap();
    let (lua, engine) = start();
    super::project::mount_at(&engine, &super::project::Mount::parse(&format!("ReplicatedStorage/Baked={}", file.to_string_lossy()), false).unwrap()).unwrap();
    let output = run(&lua, &engine, r#"
        local Baked = game:GetService("ReplicatedStorage").Baked
        assert(Baked.MeshSize == Vector3.new(26, 54, 0.001), "the file's InitialSize: " .. tostring(Baked.MeshSize))
        local Mask = Instance.new("MeshPart")
        Mask.Size = Vector3.new(1, 1, 1)
        Mask:ApplyMesh(Baked)
        assert(Mask.MeshSize == Baked.MeshSize and Mask.Size == Vector3.new(1, 1, 1), "ApplyMesh: " .. tostring(Mask.MeshSize))
        print("ok")
    "#);
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An EditableMesh snapshot is a FileMesh 4.00 ror reads back: positions, colours, uvs, and skinning
/// split into subsets of at most 26 bones with the vertices' weights.
#[cfg(feature = "meshes")]
#[test]
fn editable_mesh_snapshots_decode_in_ror() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local m = AssetService:CreateEditableMesh()
        local bones = {}
        for i = 1, 40 do
            bones[i] = m:AddBone({ Name = "B" .. i, CFrame = CFrame.new(i, 0, 0), Virtual = false })
            if i > 1 then m:SetBoneParent(bones[i], bones[i - 1]) end
        end
        for i = 1, 40 do
            local a = m:AddVertex(Vector3.new(i, 0, 0))
            local b = m:AddVertex(Vector3.new(i, 1, 0))
            local c = m:AddVertex(Vector3.new(i, 0, 1))
            for _, v in { a, b, c } do
                m:SetVertexBones(v, { bones[i], bones[(i % 40) + 1] })
                m:SetVertexBoneWeights(v, { 0.75, 0.25 })
            end
            local f = m:AddTriangle(a, b, c)
            m:SetColor(m:GetFaceColors(f)[1], Color3.new(1, 0, 0))
            m:SetUV(m:GetFaceUVs(f)[2], Vector2.new(0.5, 0.25))
        end
        local part = AssetService:CreateMeshPartAsync(Content.fromObject(m))
        print(string.sub(part.MeshId, 8))
        "#,
    );
    let path = output.iter().find_map(|line| line.strip_prefix("print: ")).expect("a snapshot path");
    let bytes = std::fs::read(path).expect("the snapshot");
    let mesh = ror_mesh::decode_mesh(&bytes).expect("ror decodes it");
    assert_eq!(mesh.positions.len(), 120);
    assert_eq!(mesh.indices.len(), 120);
    let skinning = mesh.skinning.as_ref().expect("skinned");
    assert_eq!(skinning.bones.len(), 40);
    assert_eq!(skinning.bones[1].parent, Some(0));
    assert_eq!(skinning.bones[39].name, "B40");
    assert!(skinning.subsets.len() >= 2 && skinning.subsets.iter().all(|s| s.bone_indices.len() <= 26));
    for (vertex, position) in mesh.positions.iter().enumerate() {
        let i = position[0].round() as usize;
        let joints = skinning.joints[vertex];
        let weights = skinning.weights[vertex];
        assert_eq!(skinning.bones[joints[0] as usize].name, format!("B{i}"));
        assert!((weights[0] - 0.75).abs() < 0.01 && (weights[1] - 0.25).abs() < 0.01, "{weights:?}");
    }
    assert!(mesh.colors.iter().any(|c| c[0] == 255 && c[1] == 0));
    assert!(mesh.uvs.iter().any(|uv| uv[0] == 0.5 && uv[1] == 0.25));
}

/// EditableMesh colours as measured in Studio (2026-10-16): BatchSetValues of a colour id sets its
/// alpha from a number and its colour from a Color3, alphas keep 1/255 steps.
#[test]
fn editable_mesh_colours_take_numbers_as_alpha() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local m = game:GetService("AssetService"):CreateEditableMesh()
        local f = m:AddTriangle(m:AddVertex(Vector3.zero), m:AddVertex(Vector3.xAxis), m:AddVertex(Vector3.yAxis))
        local c = m:GetFaceColors(f)[1]
        m:SetColor(c, Color3.new(1, 0, 0))
        m:BatchSetValues({ c }, { 0.25 })
        assert(m:GetColor(c) == Color3.new(1, 0, 0) and math.abs(m:GetColorAlpha(c) - 64 / 255) < 1e-6, "a number: the alpha")
        m:BatchSetValues({ c }, { Color3.new(0, 1, 0) })
        assert(m:GetColor(c) == Color3.new(0, 1, 0) and math.abs(m:GetColorAlpha(c) - 64 / 255) < 1e-6, "a Color3: the colour")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// AssetService:CreateDataModelContentAsync as a 0.747 client answers it (tools/parity/probes/data_model_content.luau): it
/// yields, then gives Enum.CreateContentResult.Success and an Opaque Content (no Uri, no Object, each call's its own), which
/// CreateMeshPartAsync and CreateEditableMeshAsync take, a copy that later edits and the mesh's Destroy do not reach; other
/// Content and a mesh without faces are refused with the client's errors.
#[cfg(feature = "meshes")]
#[test]
fn data_model_content_is_a_copy_of_a_mesh() {
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local AssetService = game:GetService("AssetService")
        local m = AssetService:CreateEditableMesh()
        local a, b, c = m:AddVertex(Vector3.new(0, 0, 0)), m:AddVertex(Vector3.new(2, 0, 0)), m:AddVertex(Vector3.new(0, 3, 0))
        m:AddTriangle(a, b, c)
        task.spawn(function()
            local Deferred = false
            task.defer(function() Deferred = true end)
            local result, made = AssetService:CreateDataModelContentAsync(Content.fromObject(m))
            local _, again = AssetService:CreateDataModelContentAsync(Content.fromObject(m), {})
            print("made", Deferred, result, tostring(made), made.SourceType, made.Uri, made.Object, made == made, made == again)
            m:SetPosition(a, Vector3.new(-1, 0, 0))
            m:Destroy()
            local part = AssetService:CreateMeshPartAsync(made)
            print("part", part.MeshSize == Vector3.new(2, 3, 0.001))
            local copy = AssetService:CreateEditableMeshAsync(made)
            print("copy", #copy:GetVertices(), #copy:GetFaces(), copy:GetPosition(copy:GetVertices()[1]) == Vector3.zero)
            for _, source in { Content.fromUri("rbxassetid://1"), Content.none, Content.fromObject(AssetService:CreateEditableMesh()) } do
                print("refused", pcall(AssetService.CreateDataModelContentAsync, AssetService, source))
            end
            print("done")
        end)
        "#,
    );
    assert!(!output.iter().any(|line| line.starts_with("print: made")), "it yields: {output:?}");
    scheduler::run_until(&lua, &engine, 0.5).unwrap();
    let lines = lines(&engine);
    for wanted in [
        "print: made true Enum.CreateContentResult.Success Content{SourceType=Opaque} Enum.ContentSourceType.Opaque nil nil true false",
        "print: part true",
        "print: copy 3 1 true",
        "print: refused false The input content type is not supported by AssetService::CreateDataModelContentAsync yet!",
        "print: refused false Error creating FileMeshData: Empty Result",
    ] {
        assert!(lines.iter().any(|line| line == wanted), "{wanted}: {lines:?}");
    }
}

/// The work clock: os.clock advances within a frame by what the engine calls a script makes cost
/// Studio (crate::cost: Instance.new, a write, entering the DataModel), and a new frame starts at
/// its own virtual time; a mesh asset's CreateEditableMeshAsync yields (its first fetch 0.55 s).
#[test]
fn the_work_clock_charges_what_studio_spends() {
    let (lua, engine) = start();
    // (work is charged from the first frame on: a build before it is not)
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local t0 = os.clock()
        local folder = Instance.new("Folder")
        folder.Parent = workspace
        local t1 = os.clock()
        for _ = 1, 100 do
            local part = Instance.new("Part")
            part.Parent = folder
        end
        local t2 = os.clock()
        print(string.format("folder %.1f parts %.1f", (t1 - t0) * 1e6, (t2 - t1) * 1e6))
        -- 10,000 more parts (24 ms) run past the frame: the next frame reads on from there
        for _ = 1, 10000 do
            Instance.new("Part").Parent = folder
        end
        local t4 = os.clock()
        task.spawn(function()
            game:GetService("RunService").Heartbeat:Wait()
            local t3 = os.clock()
            print(string.format("next frame %s", tostring(t3 > t4 and t3 - t4 < 1 / 60)))
        end)
        "#,
    );
    // (the parity bench: a Folder 0.37 us to make, 0.50 to enter the workspace; a Part 0.83 and 1.47; and the frame's first entry
    // into the workspace 13.5 more, its first part's 12.5: the engine's code for them running cold, cost.rs PATH_FIRST; each entry's
    // chain apart, cost.rs reparent_chains: a Part joining the Folder's 3 ancestors)
    assert!(output.contains(&"print: folder 14.4 parts 266.8".to_string()), "{output:?}");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    assert!(lines(&engine).contains(&"print: next frame true".to_string()), "{:?}", lines(&engine));
}

/// Luau helpers for the move and ancestor-read prices below: `Folder`, `Chain` (a chain of Folders, its top and bottom), `Move` (a
/// Parent write's price in us, over round trips after a first one, which takes the frame's cold first entry), `Each` (a call's price
/// in us, less its loop) and `Within`.
const MOVE_HELPERS: &str = r#"
    local RS = game:GetService("ReplicatedStorage")
    local function Folder(Name, Parent)
        local F = Instance.new("Folder")
        F.Name = Name
        F.Parent = Parent
        return F
    end
    local function Chain(Depth, Parent, Name)
        local Top = Folder(Name or "C", Parent)
        local Node = Top
        for _ = 2, Depth do
            Node = Folder(Name or "C", Node)
        end
        return Top, Node
    end
    local function Move(Object, A, B)
        Object.Parent = A
        Object.Parent = B
        Object.Parent = A
        local N = 10
        local Start = os.clock()
        for _ = 1, N do
            Object.Parent = B
            Object.Parent = A
        end
        local Middle = os.clock()
        for _ = 1, N do
        end
        return ((Middle - Start) - (os.clock() - Middle)) / (2 * N) * 1e6
    end
    local function Each(Call)
        local N = 100
        Call()
        local Start = os.clock()
        for _ = 1, N do
            Call()
        end
        local Middle = os.clock()
        for _ = 1, N do
        end
        return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
    end
    local function Within(Label, Value, Low, High)
        assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
    end
"#;

/// A Parent write pays for what it carries (cost.rs reparented, WORLD_WORKSPACE_PART, WORLD_MODEL_PART, PLACED_PART): a playing
/// client's move of a Model between two Folders of ReplicatedStorage grows by 0.075 us a part, by 0.90 and 0.20 more for each part it
/// takes into or out of the Workspace (its world, and what the client draws), by 0.70 for each it takes into or out of a WorldModel's
/// world, and by 1.85 for each GUI object; a WorldModel keeps its parts' world, so moving one into the Workspace adds only what is
/// drawn. Outside the DataModel a move pays 0.047 an instance. Each move paid a flat 0.51 us (0.15 outside) whatever it carried.
#[test]
fn a_move_pays_for_the_instances_it_carries_and_the_worlds_they_change() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"{MOVE_HELPERS}
        local function Model(Class, Parts)
            local M = Instance.new(Class)
            for _ = 1, Parts do
                local P = Instance.new("Part")
                P.Anchored = true
                P.Parent = M
            end
            return M
        end
        local A, B, Placed = Folder("A", RS), Folder("B", RS), Folder("A", workspace)
        local World, PlacedWorld = Instance.new("WorldModel"), Instance.new("WorldModel")
        World.Parent = RS
        PlacedWorld.Parent = workspace
        local Empty, Parts = Model("Model", 0), Model("Model", 1000)
        Within("an empty Model between two Folders", Move(Empty, A, B), 0.3, 1.0)
        Within("1,000 parts more", Move(Parts, A, B) - Move(Empty, A, B), 74.9, 75.1)
        Within("into and out of the Workspace", Move(Parts, A, Placed) - Move(Empty, A, Placed), 1174.9, 1175.1)
        Within("into and out of a WorldModel", Move(Parts, A, World) - Move(Empty, A, World), 774.9, 775.1)
        Within("between the Workspace and a WorldModel in it", Move(Parts, Placed, PlacedWorld) - Move(Empty, Placed, PlacedWorld), 1674.9, 1675.1)
        local Held, EmptyWorld = Model("WorldModel", 1000), Model("WorldModel", 0)
        Within("a WorldModel of 1,000 parts into the Workspace", Move(Held, A, Placed) - Move(EmptyWorld, A, Placed), 274.9, 275.1)
        local Gui, EmptyGui = Instance.new("ScreenGui"), Instance.new("ScreenGui")
        for _ = 1, 100 do
            Instance.new("Frame").Parent = Gui
        end
        Within("100 Frames more", Move(Gui, A, B) - Move(EmptyGui, A, B), 184.9, 185.1)
        local LooseA, LooseB = Instance.new("Folder"), Instance.new("Folder")
        Within("1,000 parts more outside the DataModel", Move(Parts, LooseA, LooseB) - Move(Empty, LooseA, LooseB), 46.9, 47.1)
        "#
        ),
    );
}

/// A Parent write pays for the two ancestor chains it walks (cost.rs reparent_chains), and what it carries for its depth
/// (REPARENTED_KEPT, REPARENTED_NESTED, REPARENTED_LOOSE_NESTED, MOVED_NESTED): a playing client's move of a Part between the bottoms
/// of two chains 100 Folders deep in ReplicatedStorage costs 31 us more than between two Folders beside each other, and between two
/// Folders at the bottom of one such chain 3.5 us more (each ancestor both keep 26.5 ns, each it leaves 5.2 and joins 23, each it
/// leaves times the new chain's length 0.95 and each it joins times the old one's 1.85, and the Part 6.0 for each of the 99 more
/// ancestors it keeps), a Folder of 100 Folders there 63 us more; a chain of 100 Folders costs 7.8 us more to move than a flat Folder of
/// 99 (1.6 ns for each level an instance sits below the moved one past the first), 7.3 outside the DataModel (1.5) and 38 into and out
/// of it (7.8). Each paid 0.51 us however deep, 0.15 outside the DataModel.
#[test]
fn a_move_pays_for_the_ancestor_chains_it_walks() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"{MOVE_HELPERS}
        local Part = Instance.new("Part")
        local _, Deep1 = Chain(100, RS)
        local _, Deep2 = Chain(100, RS)
        local _, Shallow1 = Chain(1, RS)
        local _, Shallow2 = Chain(1, RS)
        Within("between two chains 100 deep", Move(Part, Deep1, Deep2) - Move(Part, Shallow1, Shallow2), 31.30, 31.39)
        local DeepA, DeepB, ShallowA, ShallowB = Folder("A", Deep1), Folder("B", Deep1), Folder("A", Shallow1), Folder("B", Shallow1)
        Within("beside each other 100 deep", Move(Part, DeepA, DeepB) - Move(Part, ShallowA, ShallowB), 3.46, 3.53)
        local Flat = Instance.new("Folder")
        for _ = 1, 100 do
            Instance.new("Folder").Parent = Flat
        end
        Within("a Folder of 100 Folders beside each other 100 deep", Move(Flat, DeepA, DeepB) - Move(Flat, ShallowA, ShallowB), 62.85, 62.93)
        Flat:GetChildren()[1]:Destroy()
        local Nested = Chain(100, nil)
        local A, B = Folder("A", RS), Folder("B", RS)
        Within("a chain of 100 Folders", Move(Nested, A, B) - Move(Flat, A, B), 7.72, 7.80)
        local LooseA, LooseB = Instance.new("Folder"), Instance.new("Folder")
        Within("a chain of 100 Folders outside the DataModel", Move(Nested, LooseA, LooseB) - Move(Flat, LooseA, LooseB), 7.24, 7.32)
        Within("a chain of 100 Folders into and out of the DataModel", Move(Nested, A, nil) - Move(Flat, A, nil), 37.80, 37.88)
        "#
        ),
    );
}

/// Taking a child out of its parent's list (a Parent write, a move, Destroy) pays 0.2 ns for each place a playing client's search
/// for it passes (cost.rs CHILD_SEARCHED), in the order the client keeps the list in (dom.rs Children): the client looks at its
/// last child first, then from the start, and takes a child out of a list of more than 20 by moving its last child into the place.
/// Out of 1,000 Folders the 500th pays 499 places; the client has then moved the 1,000th there, so the 1,000th, the last child
/// GetChildren lists, pays 499 places too, and so does the 999th after it (destroyed); the first pays nothing (moved), and so does
/// the 996th, the client's last by then. Out of 20 the client closes the gap: the 5th pays 4 places and the last nothing after it;
/// out of 21 the 3rd pays 2 and the client moves the 21st there, which pays 2 as well. GetChildren lists the rest in the client's
/// order: of the 1,000, the 997th first, the 995th where the 300th was and the 994th where the 500th was.
#[test]
fn taking_out_a_child_pays_for_the_places_a_clients_search_passes() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"{MOVE_HELPERS}
        local function List(N)
            local Holder = Instance.new("Folder")
            local Made = {{}}
            for I = 1, N do
                local F = Instance.new("Folder")
                F.Name = tostring(I)
                F.Parent = Holder
                Made[I] = F
            end
            return Holder, Made
        end
        local Other = Instance.new("Folder")
        local function Took(Child, How)
            local _ = Child.Name
            local Start = os.clock()
            if How == "destroy" then
                Child:Destroy()
            elseif How == "move" then
                Child.Parent = Other
            else
                Child.Parent = nil
            end
            return (os.clock() - Start) * 1e6
        end
        local function Lone()
            local Holder = Instance.new("Folder")
            local Only = Instance.new("Folder")
            Only.Parent = Holder
            return Only
        end
        -- (the frame's first of each pays its cold price)
        for _, How in {{ "nil", "move", "destroy" }} do
            Took(Lone(), How)
        end
        -- what taking Child out costs beyond taking a lone child out the same way (os.clock's last bit is 1.8e-12 s)
        local function Extra(Label, Child, How, Places)
            local Base = Took(Lone(), How)
            local Expected = Places * 0.0002
            Within(Label, Took(Child, How) - Base, Expected - 1e-5, Expected + 1e-5)
        end
        local function Names(Holder)
            local Out = {{}}
            for _, Child in Holder:GetChildren() do
                table.insert(Out, Child.Name)
            end
            return table.concat(Out, ",")
        end
        local Holder, Made = List(1000)
        Extra("the 500th of 1,000", Made[500], "nil", 499)
        Extra("the 1,000th, last, where the 500th was", Made[1000], "nil", 499)
        Extra("the 999th, last, destroyed where the 1,000th was", Made[999], "destroy", 499)
        Extra("the first, moved", Made[1], "move", 0)
        Extra("the 996th, the client's last", Made[996], "nil", 0)
        Extra("the 300th, moved", Made[300], "move", 299)
        Extra("the 998th, last, where the 999th was", Made[998], "nil", 499)
        local Expected = {{ "997" }}
        for I = 2, 993 do
            table.insert(Expected, tostring(if I == 300 then 995 elseif I == 500 then 994 else I))
        end
        assert(Names(Holder) == table.concat(Expected, ","), "the rest in the client's order")
        local Short, Twenty = List(20)
        Extra("the 5th of 20", Twenty[5], "nil", 4)
        Extra("the last of 19", Twenty[20], "nil", 0)
        Extra("the 19th, last, destroyed", Twenty[19], "destroy", 0)
        Extra("the first", Twenty[1], "move", 0)
        Extra("the 10th", Twenty[10], "nil", 7)
        assert(Names(Short) == "2,3,4,6,7,8,9,11,12,13,14,15,16,17,18", Names(Short))
        local Edge, TwentyOne = List(21)
        Extra("the 3rd of 21", TwentyOne[3], "nil", 2)
        Extra("the 21st, last, where the 3rd was", TwentyOne[21], "nil", 2)
        assert(Names(Edge) == "1,2,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20", Names(Edge))
        "#
        ),
    );
}

/// IsDescendantOf, IsAncestorOf and the FindFirstAncestor finds pay 1.2 ns for each ancestor they look at, and GetFullName 4.2 ns
/// for each name it joins and 0.14 for each byte (cost.rs ANCESTOR_STEP, full_name), as a playing client's calls grow with the
/// depth: on a Part 1,000 Folders deep, 1.2 us and 5.7 us more than on one Folder deep (names of 10 bytes), through the method's C
/// fast path and a dot call alike, and an error that names the Part by its full name as much; IsDescendantOf(workspace) too, as a
/// client walks to the Workspace (the engine's own checks read the homes the dom keeps). Each paid its call's flat price.
#[test]
fn ancestor_reads_pay_for_each_ancestor_and_full_names_for_each_name() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"{MOVE_HELPERS}
        local Top, Bottom = Chain(1000, workspace, "FolderName")
        local ShallowTop = Folder("FolderName", workspace)
        local function Leaf(Parent)
            local P = Instance.new("Part")
            P.Name = "PartNameXY"
            P.Parent = Parent
            return P
        end
        local Deep, Shallow = Leaf(Bottom), Leaf(ShallowTop)
        local Steps = 999 * 0.0012
        local function Deeper(Label, Call, Expected)
            Within(Label, Each(function() Call(Deep, Top) end) - Each(function() Call(Shallow, ShallowTop) end), Expected - 0.002, Expected + 0.002)
        end
        Deeper("IsDescendantOf its top", function(Leaf, Top) Leaf:IsDescendantOf(Top) end, Steps)
        Deeper("IsDescendantOf ReplicatedStorage", function(Leaf) Leaf:IsDescendantOf(RS) end, Steps)
        Deeper("IsDescendantOf, a dot call", function(Leaf, Top) Leaf.IsDescendantOf(Leaf, Top) end, Steps)
        Deeper("IsAncestorOf", function(Leaf, Top) Top:IsAncestorOf(Leaf) end, Steps)
        Deeper("IsAncestorOf, a dot call", function(Leaf, Top) Top.IsAncestorOf(Top, Leaf) end, Steps)
        Deeper("IsDescendantOf(workspace)", function(Leaf) Leaf:IsDescendantOf(workspace) end, Steps)
        Deeper("FindFirstAncestor", function(Leaf) Leaf:FindFirstAncestor("Nope") end, Steps)
        Deeper("FindFirstAncestor, a dot call", function(Leaf) Leaf.FindFirstAncestor(Leaf, "Nope") end, Steps)
        Deeper("FindFirstAncestorOfClass", function(Leaf) Leaf:FindFirstAncestorOfClass("Model") end, Steps)
        Deeper("FindFirstAncestorOfClass, a dot call", function(Leaf) Leaf.FindFirstAncestorOfClass(Leaf, "Model") end, Steps)
        Deeper("FindFirstAncestorWhichIsA", function(Leaf) Leaf:FindFirstAncestorWhichIsA("Model") end, Steps)
        Deeper("FindFirstAncestorWhichIsA, a dot call", function(Leaf) Leaf.FindFirstAncestorWhichIsA(Leaf, "Model") end, Steps)
        -- 1,002 names and 11,020 bytes against 3 and 31
        local Named = 999 * 0.0042 + 10989 * 0.00014
        Deeper("GetFullName", function(Leaf) Leaf:GetFullName() end, Named)
        Deeper("GetFullName, a dot call", function(Leaf) Leaf.GetFullName(Leaf) end, Named)
        Deeper("an error naming it", function(Leaf) pcall(function() return Leaf.Nope end) end, Named)
        "#
        ),
    );
}

/// Two runs of one scene make the same writes. A table keyed by Instances, tables, functions or threads is visited in the
/// order of the keys' hashes, which used to be their addresses: the handlers of a Heartbeat loop ran in another order each run, and
/// with the work clock each one's os.clock() read, and the value it wrote, moved with it. The runs here are live at the same time on
/// two threads with a different amount of heap before the scene, so no address repeats; each prints the order it visited every kind
/// of key and what the handlers wrote.
#[test]
fn identical_runs_visit_object_keys_in_the_same_order() {
    const SCENE: &str = r#"
        local RunService = game:GetService("RunService")
        local ByInstance, ByTable, ByFunction, ByThread = {}, {}, {}, {}
        local Parts = {}
        for Index = 1, 60 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Parent = workspace
            Parts[Index] = Part
            ByInstance[Part] = Index
            ByTable[{ Part = Part, Index = Index }] = Index
            ByFunction[function()
                Part.Position = Vector3.new(Index, os.clock() * 1e6 % 1000, 0)
            end] = Index
            ByThread[coroutine.create(function() end)] = Index
        end
        local Frames = 0
        RunService.Heartbeat:Connect(function()
            Frames += 1
            local Orders = { Instance = {}, Table = {}, Function = {}, Thread = {} }
            for _, Index in ByInstance do table.insert(Orders.Instance, Index) end
            for Key in ByTable do
                table.insert(Orders.Table, Key.Index)
                Key.Part.Orientation = Vector3.new(0, os.clock() * 1e6 % 360, 0)
            end
            for Function, Index in ByFunction do
                table.insert(Orders.Function, Index)
                Function()
            end
            for _, Index in ByThread do table.insert(Orders.Thread, Index) end
            if Frames == 3 then
                for Kind, Order in Orders do print(`{Kind} {table.concat(Order, ",")}`) end
                for Index, Part in Parts do
                    print(string.format("%d %.17g %.17g", Index, Part.Position.Y, Part.Orientation.Y))
                end
            end
        end)
    "#;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let runs: Vec<_> = [0usize, 5000]
        .into_iter()
        .map(|pad| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let (lua, engine) = start();
                let padding: Vec<Vec<u8>> = (0..pad).map(|size| vec![0u8; size % 97 + 1]).collect();
                crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
                run(&lua, &engine, SCENE);
                for _ in 0..4 {
                    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
                }
                let output = lines(&engine);
                barrier.wait(); // neither run ends before the other has run: their objects never share an address
                drop(padding);
                output
            })
        })
        .collect();
    let outputs: Vec<Vec<String>> = runs.into_iter().map(|run| run.join().unwrap()).collect();
    assert!(outputs[0].len() > 60, "the scene printed nothing: {:?}", outputs[0]);
    assert_eq!(outputs[0], outputs[1], "two runs of one scene differ");
}

/// GuiService:GetInsetArea answered nil, so a panel that sizes its backdrop from it failed on `.Width`.
#[test]
fn gui_service_reports_the_inset_area() {
    assert_script(
        r#"
        local Gui = game:GetService("GuiService")
        local Whole = Gui:GetInsetArea(Enum.ScreenInsets.None)
        assert(typeof(Whole) == "Rect" and Whole.Width > 0 and Whole.Height > 0, "no area")
        assert(Whole.Width == Gui:GetInsetArea(Enum.ScreenInsets.DeviceSafeInsets).Width)
        assert(Gui:GetInsetArea(Enum.ScreenInsets.TopbarSafeInsets).Height <= Whole.Height)
        "#,
    );
}

/// REE_FROZEN_CLOCK=1: work moves no clock reading, so two code versions that differ in speed read the same os.clock and a
/// script that writes from the time it reads (a spring, an animation phase) writes the same values.
#[test]
fn a_frozen_clock_ignores_the_work_between_reads() {
    const SCRIPT: &str = r#"
        local Before = os.clock()
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        for _ = 1, 400 do Instance.new("Part").Parent = Folder end
        print(string.format("moved %s", tostring(os.clock() - Before > 1e-4)))
    "#;
    for (frozen, expected) in [(false, "moved true"), (true, "moved false")] {
        let (lua, engine) = start();
        engine.state.borrow_mut().sched.frozen_clock = frozen;
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let output = run(&lua, &engine, SCRIPT);
        assert!(output.contains(&format!("print: {expected}")), "frozen {frozen}: {output:?}");
    }
}

/// A frozen clock is the frame's time for every thread that reads it, however many read it in a frame, as the bench's
/// frame-locked clock is: 400 threads timed on os.clock, as a heavy attack's copies are, all read it the same within each
/// frame. (The reads counted against the allowance for the whole phase, so from the 257th on each moved it 10 us, and an arm
/// that read more ran its copies ahead.) A busy loop in one resumption still moves it, so it ends.
#[test]
fn a_frozen_clock_reads_the_same_for_every_thread_in_a_frame() {
    let (lua, engine) = start();
    engine.state.borrow_mut().sched.frozen_clock = true;
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local RunService = game:GetService("RunService")
        local Reads = { {}, {} }
        _G.FrozenReads = Reads
        for _ = 1, 400 do
            task.spawn(function()
                for Frame = 1, 2 do
                    RunService.Heartbeat:Wait()
                    table.insert(Reads[Frame], os.clock())
                end
            end)
        end
        "#,
    );
    for _ in 0..3 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = run(
        &lua,
        &engine,
        r#"
        local function Spread(List)
            local Low, High = math.huge, -math.huge
            for _, Value in List do
                Low, High = math.min(Low, Value), math.max(High, Value)
            end
            return High - Low
        end
        local Reads = _G.FrozenReads
        print(string.format("reads %d %d spread %.9f %.9f", #Reads[1], #Reads[2], Spread(Reads[1]), Spread(Reads[2])))
        local Start, Count = os.clock(), 0
        while os.clock() - Start < 0.001 do
            Count += 1
        end
        print(string.format("busy loop ended after %d reads", Count))
        "#,
    );
    assert!(output.contains(&"print: reads 400 400 spread 0.000000000 0.000000000".to_string()), "{output:?}");
    assert!(output.iter().any(|line| line.starts_with("print: busy loop ended after ")), "{output:?}");
}

/// A frozen clock stops the wall clock too: tick(), os.time() and DateTime.now() read the same in a run an hour later, so two
/// differential runs that straddle an hour boundary still write the same values.
#[test]
fn a_frozen_clock_pins_the_wall_clock() {
    const SCRIPT: &str = r#"print(`wall {os.time()} {DateTime.now().UnixTimestamp} {math.floor(tick())} {math.floor(workspace:GetServerTimeNow())}`)"#;
    let (lua, engine) = start();
    {
        let mut state = engine.state.borrow_mut();
        // (as if the run began an hour earlier)
        state.sched.epoch_millis -= 3_600_000.0;
        state.sched.freeze_wall_clock();
    }
    let first = run(&lua, &engine, SCRIPT).into_iter().find(|line| line.starts_with("print: wall")).expect("wall line");
    let (lua, engine) = start();
    engine.state.borrow_mut().sched.freeze_wall_clock();
    let second = run(&lua, &engine, SCRIPT).into_iter().find(|line| line.starts_with("print: wall")).expect("wall line");
    assert_eq!(first, second);
    assert!(first.contains("1789999200"), "{first}");
}

/// A table keyed by Instances is visited in the same order in every run, whatever slots the instances got. A slot is
/// reused when the collector frees an instance, and what it has freed by then depends on the heap, so the same scene gave its
/// instances other slots in another run and `pairs` visited them in another order. Both engines here make 300 instances before the
/// scene; the second lets them go, which hands the scene's instances the freed slots in the opposite order (the serials stay equal).
#[test]
fn instance_keys_are_visited_in_the_same_order_whatever_their_slots() {
    const SCENE: &str = r#"
        local Keys, Order = {}, {}
        for Index = 1, 80 do
            local Part = Instance.new("Part")
            Part.Parent = workspace
            Keys[Part] = Index
        end
        for _, Index in Keys do table.insert(Order, Index) end
        print("order " .. table.concat(Order, ","))
    "#;
    let outputs: Vec<Vec<String>> = [false, true]
        .into_iter()
        .map(|release| {
            let (lua, engine) = start();
            crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
            run(&lua, &engine, r#"_G.Held = {} for _ = 1, 300 do table.insert(_G.Held, Instance.new("Folder")) end"#);
            // (a script run makes an instance too: both engines run one, so the serials match)
            run(&lua, &engine, if release { "_G.Held = nil" } else { "_G.Unused = nil" });
            lua.gc_collect().unwrap();
            lua.gc_collect().unwrap();
            let freed = engine_lua::raw::collect(&lua, &engine).unwrap();
            assert_eq!(freed >= 300, release, "freed {freed}");
            run(&lua, &engine, SCENE);
            lines(&engine).into_iter().filter(|line| line.starts_with("print: order")).collect()
        })
        .collect();
    assert_eq!(outputs[0].len(), 1, "{:?}", outputs[0]);
    assert_eq!(outputs[0], outputs[1], "the scene's instances took other slots and were visited in another order");
}

/// The first touch of an instance in a frame costs more than the next ones: 45 ns for a static instance (the parity bench,
/// tools/cost-parity: a part of 100 read once each a frame 74.6 ns against 29.4 hot), where a Studio playtest's animated rigs read
/// about 280 more (a bridge_ops probe, 28 R15 rigs: Part.CFrame touching five parts a rig once 322 ns, eight times each 80; a
/// Motor6D.Transform write 344 / 92). A loop on one instance never pays it; a bridge over many instances pays it on each, every frame.
#[test]
fn the_work_clock_charges_the_first_touch_of_an_instance_in_a_frame() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Rigs = Instance.new("Folder")
        Rigs.Name = "Rigs"
        Rigs.Parent = workspace
        for Index = 1, 140 do
            local Part = Instance.new("Part")
            Part.Parent = Rigs
            local Motor = Instance.new("Motor6D")
            Motor.Parent = Part
        end
    "#,
    );
    // (a new frame: everything above is cold again)
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Parts = workspace.Rigs:GetChildren()
        local Motors = {}
        for _, Part in Parts do
            table.insert(Motors, Part:FindFirstChildOfClass("Motor6D"))
        end
        local function Per(Body)
            local start = os.clock()
            Body()
            return (os.clock() - start) / #Parts * 1e9
        end
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.0f ns, Studio %d-%d", Name, Value, Low, High))
        end
        -- (the Part:FindFirstChildOfClass above touched every Part and the Motors are cold)
        local Cold = Per(function() for _, Motor in Motors do local _ = Motor.C0 end end)
        local Hot = Per(function() for _, Motor in Motors do local _ = Motor.C0 end end)
        Within("first read of a Motor6D.C0", Cold, 70, 150)
        Within("second read of it", Hot, 25, 80)
        local Write = Per(function() for _, Motor in Motors do Motor.Transform = CFrame.identity end end)
        Within("a write to an instance read this frame", Write, 40, 110)
        local ColdIsA = Per(function() for _, Part in Parts do local _ = Part:IsA("BasePart") end end)
        Within("IsA on a part touched this frame", ColdIsA, 15, 80)
    "#,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Motors = {}
        for _, Part in workspace.Rigs:GetChildren() do
            table.insert(Motors, Part:FindFirstChildOfClass("Motor6D"))
        end
        local start = os.clock()
        for _, Motor in Motors do
            Motor.Transform = CFrame.identity
        end
        local Cold = (os.clock() - start) / #Motors * 1e9
        assert(Cold >= 80 and Cold <= 160, string.format("a Motor6D.Transform write to a cold Motor6D: %.0f ns, the bench's 60 + 45", Cold))
        -- an instance made in this frame is hot: its writes cost the hot price
        local Made = {}
        for Index = 1, #Motors do
            Made[Index] = Instance.new("Motor6D")
        end
        local made = os.clock()
        for _, Motor in Made do
            Motor.Transform = CFrame.identity
        end
        local Fresh = (os.clock() - made) / #Made * 1e9
        assert(Fresh >= 30 and Fresh <= 110, string.format("a Motor6D.Transform write to a Motor6D made this frame: %.0f ns, hot 61", Fresh))
    "#,
    );
}

/// A write to an instance nothing touched this frame costs more than the first touch alone (a cold_write probe, a match place's
/// client, five runs, fitted per write): writing 100 and 300 distinct Parts once each against writing
/// one Part as often, the difference per write is Transparency +388 ns, Color +357, CFrame +773, Name +559, SetAttribute +830, and on a
/// Frame Size +475..525, Position +600, BackgroundTransparency +380 (noisier). ree charged
/// the first touch's 280 for every one (a roster of 100 Parts was priced at about half of Studio), so the roster read 100-550 ns a
/// write low. Bands of 1.3x around Studio's differences, but Transparency's: the parity bench (tools/cost-parity, a rendered client
/// of a game's match place) reads a roster of 100 parts' Transparency +143 ns over the hot write (cost.rs cold_write_extra).
#[test]
fn a_write_to_a_cold_instance_costs_what_a_dirtied_instance_costs() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        Folder.Name = "Roster"
        Folder.Parent = workspace
        for Index = 1, 300 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Parent = Folder
        end
        local Gui = Instance.new("Folder")
        Gui.Name = "Gui"
        Gui.Parent = workspace
        for Index = 1, 300 do
            local Item = Instance.new("Frame")
            Item.Parent = Gui
        end
        "#,
    );
    // (each kind in a frame of its own: the Parts a kind wrote are warm for the next one). The Frames' bands are wider: Studio's
    // GUI readings spread more (Position 781-1550 ns at 100 writes).
    let kinds: [(&str, &str, u32, u32, &str); 8] = [
        ("Transparency", "Roster", 100, 200, "Part.Transparency = 0.25 + (Index % 2) * 0.5"),
        ("Color", "Roster", 300, 470, "Part.Color = Color3.new((Index % 2) * 0.5, 0.25, 0.75)"),
        ("CFrame", "Roster", 600, 1000, "Part.CFrame = CFrame.new(Index % 2 + 1, 0, 0)"),
        ("Name", "Roster", 430, 730, "Part.Name = 'N' .. (Index % 2)"),
        ("SetAttribute", "Roster", 640, 1080, "Part:SetAttribute('A', Index)"),
        ("Frame.Size", "Gui", 400, 900, "Part.Size = UDim2.fromOffset(10 + Index % 2, 10)"),
        ("Frame.Position", "Gui", 400, 800, "Part.Position = UDim2.fromOffset(1 + Index % 2, 1)"),
        ("Frame.BackgroundTransparency", "Gui", 300, 500, "Part.BackgroundTransparency = 0.25 + (Index % 2) * 0.5"),
    ];
    for (name, roster, low, high, write) in kinds {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(
            &lua,
            &engine,
            &format!(
                r#"
                local Roster = workspace.{roster}:GetChildren()
                local function Write(Part, Index) {write} end
                -- one Part written 300 times, then 300 Parts written once: the same instructions, so the difference is the cold instances
                local One = Roster[1]
                Write(One, 0)
                local start = os.clock()
                for Index = 1, 300 do
                    Write(One, Index)
                end
                local Hot = (os.clock() - start) / 300 * 1e9
                local middle = os.clock()
                for Index = 1, 300 do
                    Write(Roster[Index], Index)
                end
                local Cold = (os.clock() - middle) / 300 * 1e9
                local Extra = Cold - Hot
                assert(Extra >= {low} and Extra <= {high}, string.format("{name}: a cold write costs +%.0f ns over a hot one, Studio {low}-{high} (hot %.0f, cold %.0f)", Extra, Hot, Cold))
                print("ok {name}")
                "#
            ),
        );
        assert!(lines(&engine).contains(&format!("print: ok {name}")), "{name}: {:?}", lines(&engine));
    }
}

/// The work clock prices instance operations from a client's own numbers: the parity bench (tools/cost-parity, a rendered client
/// of a game's match place) where it has a row, else an op_cost probe run in a lobby playtest's client (the median of seven rounds
/// less an empty loop, 2026-10-23), bands of about +-25 % around them. Before, every property write cost a flat 0.5 us, every read
/// 0.06, a Clone 7 us an instance, and ScaleTo, Connect, task.spawn, BulkMoveTo and QueryDescendants almost nothing or far too much
/// (a ScaleTo to the scale a model has cost as much as a rescale).
#[test]
fn the_work_clock_prices_instance_operations_as_the_studio_bench_measured() {
    let (lua, engine) = start();
    // (interpreted as `ree record` and `ree run` run modules: the loops' own instructions are charged)
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local function Per(Count, Body)
            local start = os.clock()
            Body(Count)
            return (os.clock() - start) / Count * 1e9
        end
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.0f ns, Studio %d-%d", Name, Value, Low, High))
        end
        local A, B = CFrame.new(10, 20, 30), CFrame.new(10, 20, 31)
        local Placed = Instance.new("Part")
        Placed.Parent = workspace
        local Loose = Instance.new("Part")
        local Motor, Bone = Instance.new("Motor6D"), Instance.new("Bone")
        Motor.Parent, Bone.Parent = Placed, Placed

        -- property writes (the bench: CFrame in the workspace 1,430, Size 1,100 from Studio's ratio to it, Name 300, Motor6D.C0 140;
        -- a lobby client: loose 163, same value 52, Color 547, Transparency 254, Bone.Transform 68)
        Within("CFrame write placed", Per(2000, function(N) for _ = 1, N / 2 do Placed.CFrame = A Placed.CFrame = B end end), 1100, 1800)
        Within("CFrame write loose", Per(2000, function(N) for _ = 1, N / 2 do Loose.CFrame = A Loose.CFrame = B end end), 110, 260)
        Within("CFrame write same value", Per(2000, function(N) for _ = 1, N do Placed.CFrame = B end end), 30, 110)
        Within("Size write placed", Per(2000, function(N) for _ = 1, N / 2 do Placed.Size = Vector3.one Placed.Size = Vector3.new(1, 1, 2) end end), 850, 1400)
        Within("Color write", Per(2000, function(N) for _ = 1, N / 2 do Placed.Color = Color3.new(1, 0, 0) Placed.Color = Color3.new(0, 1, 0) end end), 430, 700)
        Within("Transparency write", Per(2000, function(N) for _ = 1, N / 2 do Placed.Transparency = 0.25 Placed.Transparency = 0.75 end end), 190, 330)
        Within("Name write", Per(2000, function(N) for _ = 1, N / 2 do Placed.Name = "A" Placed.Name = "B" end end), 230, 400)
        Within("Motor6D.C0 write", Per(2000, function(N) for _ = 1, N / 2 do Motor.C0 = A Motor.C0 = B end end), 110, 190)
        Within("Bone.Transform write", Per(2000, function(N) for _ = 1, N / 2 do Bone.Transform = A Bone.Transform = B end end), 40, 110)
        Within("property read", Per(2000, function(N) for _ = 1, N do local _ = Placed.Transparency end end), 25, 60)

        -- attributes (the bench: a changed value 0.43 us with the call, the same 77 ns)
        Within("SetAttribute changes", Per(2000, function(N) for _ = 1, N / 2 do Placed:SetAttribute("A", 1) Placed:SetAttribute("A", 2) end end), 340, 580)
        Within("SetAttribute same", Per(2000, function(N) for _ = 1, N do Placed:SetAttribute("A", 2) end end), 50, 130)

        -- Clone, by class (the bench: a MeshPart 5.83 us, a Part 3.44)
        local Many = Instance.new("Model")
        for _ = 1, 50 do Instance.new("MeshPart").Parent = Many end
        Within("Clone of 50 MeshParts", Per(20, function(N) for _ = 1, N do Many:Clone() end end), 240000, 310000)
        local Parts = Instance.new("Model")
        for _ = 1, 50 do Instance.new("Part").Parent = Parts end
        Within("Clone of 50 Parts", Per(20, function(N) for _ = 1, N do Parts:Clone() end end), 140000, 215000)

        -- Instance.new (the bench: Folder 0.36 us, Part 0.83, TextLabel 1.44)
        Within("Instance.new Folder", Per(1000, function(N) for _ = 1, N do Instance.new("Folder") end end), 300, 650)
        Within("Instance.new Part", Per(1000, function(N) for _ = 1, N do Instance.new("Part") end end), 650, 1100)
        Within("Instance.new TextLabel", Per(1000, function(N) for _ = 1, N do Instance.new("TextLabel") end end), 1100, 1900)

        -- ScaleTo of 29 parts (client: a rescale in the workspace 35.0 us, out of it 9.0, to the scale it has 6.0)
        local function Model(Count, Parent)
            local Rig = Instance.new("Model")
            for _ = 1, Count do
                local Part = Instance.new("Part")
                Part.Anchored = true
                Part.Parent = Rig
            end
            Rig.PrimaryPart = Rig:FindFirstChildWhichIsA("BasePart")
            Rig.Parent = Parent
            return Rig
        end
        local Standing, Held = Model(29, workspace), Model(29, nil)
        Within("ScaleTo placed", Per(100, function(N) for _ = 1, N / 2 do Standing:ScaleTo(2) Standing:ScaleTo(1) end end), 28000, 44000)
        Within("ScaleTo loose", Per(100, function(N) for _ = 1, N / 2 do Held:ScaleTo(2) Held:ScaleTo(1) end end), 7000, 12000)
        Within("ScaleTo same scale", Per(100, function(N) for _ = 1, N do Standing:ScaleTo(1) end end), 4500, 9000)
        -- (the bench: Model:PivotTo of 10 parts 10.65 us, 1.05 a part)
        Within("Model:PivotTo", Per(100, function(N) for _ = 1, N / 2 do Standing:PivotTo(A) Standing:PivotTo(B) end end), 24000, 38000)
        Within("Model:GetPivot", Per(1000, function(N) for _ = 1, N do Standing:GetPivot() end end), 20, 150)

        -- a fallback rig's ScaleTo(1) from its stored scale 0.43, out of the DataModel (in Studio: a rig of 1 MeshPart,
        -- 7 Bones, 19 Attachments and an AnimationController with its Animator 14.7 us, a rig of 7 Parts, 6 Motor6Ds,
        -- 19 Attachments, a Decal, a SpecialMesh, a controller and its Animator 26.9 us)
        local function Rig(Parts, Bones, Joints)
            local Model = Instance.new("Model")
            local Root
            for Index = 1, Parts do
                local Part = Instance.new(if Index == 1 and Bones > 0 then "MeshPart" else "Part")
                Part.Name = "Part" .. Index
                Part.Anchored = true
                Part.Parent = Model
                Root = Root or Part
                for Slot = 1, math.floor(19 / Parts) + (if Index == 1 then 19 % Parts else 0) do
                    Instance.new("Attachment").Parent = Part
                end
                if Index > 1 and Joints > 0 and Index - 1 <= Joints then
                    local Joint = Instance.new("Motor6D")
                    Joint.Part0, Joint.Part1 = Root, Part
                    Joint.Parent = Part
                end
            end
            for _ = 1, Bones do
                Instance.new("Bone").Parent = Root
            end
            local Controller = Instance.new("AnimationController")
            Controller.Parent = Model
            Instance.new("Animator").Parent = Controller
            Model.PrimaryPart = Root
            Model:ScaleTo(0.43)
            return Model
        end
        local Lobby, Match = Rig(1, 7, 0), Rig(7, 0, 6)
        Within("lobby rig ScaleTo(1)", Per(1, function() Lobby:ScaleTo(1) end), 11000, 19000)
        Within("match rig ScaleTo(1)", Per(1, function() Match:ScaleTo(1) end), 21000, 33000)

        -- BulkMoveTo of 100 parts (the bench: FireCFrameChanged 1.05 us a part; every event 2.58, the events' share a lobby client's)
        local List, Here, There = {}, {}, {}
        for Index = 1, 100 do
            List[Index] = Instance.new("Part")
            List[Index].Parent = workspace
            Here[Index], There[Index] = CFrame.new(Index, 0, 0), CFrame.new(Index, 1, 0)
        end
        Within("BulkMoveTo", Per(20, function(N) for _ = 1, N / 2 do workspace:BulkMoveTo(List, Here, Enum.BulkMoveMode.FireCFrameChanged) workspace:BulkMoveTo(List, There, Enum.BulkMoveMode.FireCFrameChanged) end end), 85000, 130000)
        Within("BulkMoveTo all events", Per(20, function(N) for _ = 1, N / 2 do workspace:BulkMoveTo(List, Here) workspace:BulkMoveTo(List, There) end end), 200000, 320000)

        -- QueryDescendants over 50 parts (client 3.6 us for 100, edit 2.3)
        Within("QueryDescendants", Per(200, function(N) for _ = 1, N do Many:QueryDescendants(">>MeshPart") end end), 900, 2400)

        -- signals and threads (the bench: Connect + Disconnect 0.50 us, task.spawn 0.72 a thread, Fire with nothing connected 52 ns)
        local Event = Instance.new("BindableEvent")
        local function Noop() end
        Within("Connect + Disconnect", Per(1000, function(N) for _ = 1, N do Event.Event:Connect(Noop):Disconnect() end end), 450, 720)
        Within("task.spawn", Per(1000, function(N) for _ = 1, N do task.spawn(Noop) end end), 600, 950)
        Within("coroutine.wrap", Per(1000, function(N) for _ = 1, N do coroutine.wrap(Noop)() end end), 600, 950)
        Within("BindableEvent:Fire", Per(1000, function(N) for _ = 1, N do Event:Fire(1) end end), 40, 100)

        -- parenting inside the DataModel (the bench: a Part from one Folder of the workspace to another 0.51 us)
        local One, Two = Instance.new("Folder"), Instance.new("Folder")
        One.Parent, Two.Parent = workspace, workspace
        Loose.Parent = One
        Within("Parent between folders", Per(1000, function(N) for _ = 1, N / 2 do Loose.Parent = Two Loose.Parent = One end end), 400, 650)
        print("priced")
        "#,
    );
    assert!(output.contains(&"print: priced".to_string()), "{output:?}");
}

/// Studio has Instance:QueryDescendants and no QueryFirstDescendant ("QueryFirstDescendant is not a valid member of
/// Folder", an empty test place, 2026-10-23); ree had its own, which let a script run here that errors there (neither
/// rur nor the games it was checked against use it).
#[test]
fn query_first_descendant_is_not_a_member_as_in_studio() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        Instance.new("Part").Parent = Folder
        local Ok, Message = pcall(function()
            return Folder:QueryFirstDescendant("Part")
        end)
        assert(not Ok, "QueryFirstDescendant ran")
        assert(string.find(Message, "QueryFirstDescendant is not a valid member of Folder", 1, true), Message)
        assert(#Folder:QueryDescendants("Part") == 1)
        print("studio's members")
        "#,
    );
    assert!(output.contains(&"print: studio's members".to_string()), "{output:?}");
}

/// TextService:GetTextSize returned nil in a plain `ree run` (no host text layer), so a settings window failed on `.X`.
/// It returns Studio's measurement now: "Hello" 14 FredokaOne is 31 x 14 (an empty test place, 2026-10-23).
#[test]
fn text_service_returns_a_vector2_without_a_text_layer() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local TextService = game:GetService("TextService")
        local Size = TextService:GetTextSize("Hello", 14, Enum.Font.FredokaOne, Vector2.new(1000, 1000))
        assert(typeof(Size) == "Vector2", typeof(Size))
        assert(math.abs(Size.X - 31) <= 2 and Size.Y == 14, tostring(Size))
        local Wrapped = TextService:GetTextSize("The quick brown fox jumps over the lazy dog and keeps running", 14, Enum.Font.SourceSans, Vector2.new(100, 1000))
        assert(Wrapped.Y == 56 and math.abs(Wrapped.X - 98) <= 3, tostring(Wrapped))
        print("measured")
        "#,
    );
    assert!(output.contains(&"print: measured".to_string()), "{output:?}");
}

/// TextService:GetTextBoundsAsync returned nothing without a host's text layer (it was an unemulated stub), so a UI view that read the
/// bounds failed. It measures as GetTextSize does now, with the GetTextBoundsParams' Font face, Size and Width (measured in a playing
/// client, Studio 0.747: tools/parity/probes/text_bounds.luau).
#[test]
fn text_service_measures_text_bounds_without_a_text_layer() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local TextService = game:GetService("TextService")
        local Params = Instance.new("GetTextBoundsParams")
        Params.Text = "Hello world"
        Params.Font = Font.fromEnum(Enum.Font.FredokaOne)
        Params.Size = 20
        local Bounds = TextService:GetTextBoundsAsync(Params)
        assert(typeof(Bounds) == "Vector2", typeof(Bounds))
        assert(Bounds == Vector2.new(94, 20), tostring(Bounds))
        assert(Bounds == TextService:GetTextSize("Hello world", 20, Enum.Font.FredokaOne, Vector2.new(1e6, 1e6)))
        -- a face no Enum.Font names measures as the one its family has
        Params.Font = Font.new("rbxasset://fonts/families/FredokaOne.json", Enum.FontWeight.Bold)
        assert(TextService:GetTextBoundsAsync(Params) == Vector2.new(94, 20))
        -- Width wraps, and Size stores 1 to 100, whole
        Params.Text = "The quick brown fox jumps over the lazy dog and keeps running"
        Params.Size = 14.99
        Params.Width = 100
        assert(Params.Size == 14, Params.Size)
        assert(TextService:GetTextBoundsAsync(Params) == Vector2.new(98, 56), tostring(TextService:GetTextBoundsAsync(Params)))
        Params.Size = 500
        assert(Params.Size == 100)
        Params.Size = 0.2
        assert(Params.Size == 1)
        local Empty = Instance.new("GetTextBoundsParams")
        assert(TextService:GetTextBoundsAsync(Empty) == Vector2.new(0, 20), tostring(TextService:GetTextBoundsAsync(Empty)))
        local function Raised(...)
            local Ok, Message = pcall(TextService.GetTextBoundsAsync, TextService, ...)
            assert(not Ok)
            return Message
        end
        assert(Raised(nil) == "Argument 1 missing or nil")
        assert(Raised(Instance.new("Part")) == "Argument 1 is invalid: expected 'GetTextBoundsParams' instance type")
        assert(Raised({}) == "Unable to cast value to Object")
        Empty.Font = Font.new("")
        assert(Raised(Empty) == "Font with empty asset ID passed to TextService:GetTextBoundsAsync()")
        print("measured")
        "#,
    );
    assert!(output.contains(&"print: measured".to_string()), "{output:?}");
    // a face of its own the tables lack is measured as the nearest and said; the faces above were all measurable
    assert!(engine.state.borrow().output.missing.is_empty(), "{:?}", engine.state.borrow().output.missing);
    run(
        &lua,
        &engine,
        r#"
        local Params = Instance.new("GetTextBoundsParams")
        Params.Text = "Hello"
        Params.Font = Font.new("rbxasset://fonts/families/GothamSSm.json", Enum.FontWeight.SemiBold)
        game:GetService("TextService"):GetTextBoundsAsync(Params)
        Params.Font = Font.fromEnum(Enum.Font.SourceSans)
        Params.RichText = true
        Params.Text = "<b>Hello</b>"
        game:GetService("TextService"):GetTextBoundsAsync(Params)
        "#,
    );
    let missing: Vec<String> = engine.state.borrow().output.missing.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(
        missing,
        [
            "TextService.GetTextBoundsAsync of rbxasset://fonts/families/GothamSSm.json SemiBold Normal (a face the text tables lack; measured as GothamBold)",
            "TextService.GetTextBoundsAsync of rich text markup (measured as plain text)",
        ]
    );
}

/// The first GetTextBoundsAsync of a face a client has not loaded yields while it loads, and a call that cannot yield (from a
/// metamethod) raises; the faces a client starts with, and a face that resolves to a loaded one, do not yield (measured in a playing
/// client, Studio 0.747: tools/parity/probes/text_bounds.luau).
#[test]
fn text_bounds_wait_for_a_face_to_load() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local TextService = game:GetService("TextService")
        local function Params(Face)
            local Made = Instance.new("GetTextBoundsParams")
            Made.Text = "Hello"
            Made.Font = Face
            return Made
        end
        local function Status(Face)
            local Thread = coroutine.create(function()
                print("bounds", TextService:GetTextBoundsAsync(Params(Face)))
            end)
            coroutine.resume(Thread)
            return coroutine.status(Thread)
        end
        local function FromMetamethod(Face)
            local Meta = setmetatable({}, { __index = function()
                return TextService:GetTextBoundsAsync(Params(Face))
            end })
            local Ok, Result = pcall(function()
                return Meta.Bounds
            end)
            return if Ok then "ok" else Result
        end
        print("first", Status(Font.fromEnum(Enum.Font.FredokaOne)))
        print("again", Status(Font.fromEnum(Enum.Font.FredokaOne)))
        print("bold", FromMetamethod(Font.new("rbxasset://fonts/families/FredokaOne.json", Enum.FontWeight.Bold)))
        print("SourceSans", Status(Font.fromEnum(Enum.Font.SourceSans)))
        print("SourceSansPro Medium", FromMetamethod(Font.new("rbxasset://fonts/families/SourceSansPro.json", Enum.FontWeight.Medium)))
        print("Creepster", FromMetamethod(Font.fromEnum(Enum.Font.Creepster)))
        task.defer(function()
            print("Creepster later", FromMetamethod(Font.fromEnum(Enum.Font.Creepster)))
        end)
        "#,
    );
    let printed: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("print: ")).collect();
    assert_eq!(
        printed,
        [
            "first suspended",
            "bounds 43, 20",
            "again dead",
            "bold ok",
            "bounds 36, 20",
            "SourceSans dead",
            "SourceSansPro Medium ok",
            "Creepster thread is not yieldable",
            // (the first call, resumed at the next resumption point)
            "bounds 43, 20",
            "Creepster later ok",
        ],
        "{output:?}"
    );
}

/// The work clock charges a script's own Luau as the interpreter runs it (crate::cost::luau): its
/// instructions, calls, objects and table growth, and a datatype operation the engine's Luau serves
/// (whose own instructions are not charged); Vector3.new is the VM's fast call, as Roblox compiles it.
#[test]
fn the_work_clock_charges_a_scripts_luau() {
    let (lua, engine) = start();
    // (interpreted as `ree record` runs modules: a safe environment, no native code)
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local t0 = os.clock()
        local x = 0
        for i = 1, 10000 do
            x += i
        end
        local t1 = os.clock()
        local list = {}
        for i = 1, 1000 do
            list[i] = { Value = i }
        end
        local t2 = os.clock()
        local c, v = CFrame.new(), nil
        for i = 1, 1000 do
            c *= CFrame.new(1, 0, 0)
            v = Vector3.new(i, 0, 0)
        end
        local t3 = os.clock()
        print(string.format("loop %.1f tables %.1f cframes %.1f", (t1 - t0) * 1e6, (t2 - t1) * 1e6, (t3 - t2) * 1e6))
        "#,
    );
    // (the cframes loop was 58.5 with every datatype operation a flat 15 ns; CFrame.new, `*` and Vector3.new were priced from the
    // lobby client's op_cost loop (101.5: `*` 60 ns), `*` from a rig frame since (21 ns, 39 us less over the 1,000 products:
    // lune-roblox cframe.rs `price`), and LUAU_HOSTCALL calibrated to 12 ns on the client parity bench (-3.0 us over 1,000 calls).
    // The instructions are priced by kind since (cost.rs INTERPRETED_OPS: an ADD or FORNLOOP 0.48 ns,
    // where every instruction cost 1.6: the loop 32.0 then). The tables loop read 65.5 until the allocation prices were fitted on the
    // parity bench's allocation, churn and growth rows at several sizes (an object 14.6 ns in all where it was 22, 8.6 of it the
    // collector's; an allocated byte 0.061 and a byte of table storage 0.079 more, where storage was 0.4; an array slot growth moves
    // nothing, where it was 12))
    assert!(output.contains(&"print: loop 9.6 tables 32.1 cframes 44.9".to_string()), "{output:?}");
}

/// What a library or engine call does by the size of its input is work the clock charges (patches/luau/15-input-size-work.patch,
/// cost.rs SCANNED_SLOT and below), so each of the audit's workloads grows on the clock as a client's time grows with it (the parity
/// bench's `libsize` and `json` rows, a client's fastest session): table.find of a missing value 14 ns at 10 elements and 41 us at
/// 100,000; `s ..= piece` growing with the square of its pieces where table.concat grows with them; a list drained from its front or
/// filled at it, each call moving what is left; string.upper / gsub / rep on 1 MB 570-850 times their 1 KB figure; JSONEncode and
/// JSONDecode of 1,000 records 83 and 91 times 10's. A flat price a call read each pair alike (table.find 6.19 us at every size).
#[test]
fn library_work_grows_with_its_input() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local HttpService = game:GetService("HttpService")
        -- (each workload once before it is timed: a frame's first call of a C function costs its cold code, LUAU_COLD_CFUNC)
        local function us(f, ...)
            f(...)
            local t0 = os.clock()
            f(...)
            return (os.clock() - t0) * 1e6
        end
        local function list(n)
            local t = table.create(n)
            for i = 1, n do
                t[i] = i
            end
            return t
        end
        local results = {}
        for _, n in { 10, 1000, 100000 } do
            local t = list(n)
            results["find" .. n] = us(function()
                for _ = 1, 10 do
                    table.find(t, -1)
                end
            end) / 10
        end
        for _, n in { 1000, 4000 } do
            results["concat" .. n] = us(function()
                local s = ""
                for _ = 1, n do
                    s ..= "abcdefgh"
                end
            end)
            results["tconcat" .. n] = us(function()
                local parts = table.create(n)
                for i = 1, n do
                    parts[i] = "abcdefgh"
                end
                local _ = table.concat(parts)
            end)
        end
        for _, n in { 100, 1000 } do
            results["remove" .. n] = us(function()
                local t = list(n)
                for _ = 1, n do
                    table.remove(t, 1)
                end
            end) / n
            results["insert" .. n] = us(function()
                local t = {}
                for i = 1, n do
                    table.insert(t, 1, i)
                end
            end) / n
        end
        for _, size in { 1024, 1048576 } do
            local s = string.rep("a", size)
            results["upper" .. size] = us(string.upper, s)
            results["gsub" .. size] = us(string.gsub, s, "a", "b")
            results["rep" .. size] = us(string.rep, "b", size)
        end
        for _, n in { 10, 1000 } do
            local records = {}
            for i = 1, n do
                records[i] = { id = i, name = "item" .. i, value = i * 0.5, tags = { "a", "b" } }
            end
            local text = HttpService:JSONEncode(records)
            results["encode" .. n] = us(function()
                HttpService:JSONEncode(records)
            end)
            results["decode" .. n] = us(function()
                HttpService:JSONDecode(text)
            end)
        end
        for name, value in results do
            print(string.format("%s %.4f", name, value))
        end
        "#,
    );
    let figure = |name: &str| -> f64 {
        let prefix = format!("print: {name} ");
        output.iter().find_map(|line| line.strip_prefix(&prefix)).unwrap_or_else(|| panic!("no {name} in {output:?}")).parse().unwrap()
    };
    let ratio = |big: &str, small: &str| figure(big) / figure(small);
    // table.find compares every element: a client 14.3 ns at 10, 41.4 us at 100,000
    assert!(ratio("find100000", "find10") > 1000.0, "{output:?}");
    assert!((0.5..2.0).contains(&(figure("find100000") / 41.41)), "table.find of 100,000 elements {} us against a client's 41.41", figure("find100000"));
    // building a string piece by piece copies what it has built each time; table.concat copies each piece once
    assert!(ratio("concat4000", "concat1000") > 8.0, "{output:?}");
    assert!(ratio("tconcat4000", "tconcat1000") < 6.0, "{output:?}");
    // each table.remove(t, 1) and table.insert(t, 1, v) moves the rest of the list
    assert!(ratio("remove1000", "remove100") > 2.5 && ratio("insert1000", "insert100") > 2.5, "{output:?}");
    // string work on 1 MB against 1 KB (a client: upper 1.48 ms and 1.74 us, gsub 1.54 ms and 2.44 us, rep 234 us and 0.41 us)
    for name in ["upper", "gsub", "rep"] {
        assert!(ratio(&format!("{name}1048576"), &format!("{name}1024")) > 300.0, "{name}: {output:?}");
    }
    // JSON by its text and values (a client: 10 records' encode 5.6 us, 1,000's 463 us)
    assert!(ratio("encode1000", "encode10") > 40.0 && ratio("decode1000", "decode10") > 40.0, "{output:?}");
}

/// The collector steps at the same points however fast the host runs. Luau paces it by how long a cycle took (lgc.cpp: the allocation
/// rate and the mark phase's length set the next cycle's trigger), read from the wall clock, so on a busy machine it stepped at other
/// points in every run, and what the work clock counts moved with it: an allocating fast call (`string.sub`) gives way to its C function
/// while a step is due, a string is found alive or made again. A cutscene that slices its setup by an os.clock budget recorded
/// differently in 1 run of 20 to 2 of 3. It paces by the work clock now (cost.rs `install_gc_clock`): here a host that stalls between
/// the script's rounds (sleeping, as a loaded machine does) must not move a single heap reading.
#[test]
fn the_collector_steps_at_the_same_points_however_fast_the_host_runs() {
    let readings = |stall: bool| -> Vec<String> {
        let (lua, engine) = start();
        engine_lua::modules::interpret_all(&lua, &engine);
        let host_stall = lua
            .create_function(move |_, ()| {
                if stall {
                    std::thread::sleep(std::time::Duration::from_millis(3));
                }
                Ok(())
            })
            .unwrap();
        engine.lua.borrow().shared.as_ref().unwrap().raw_set("HostStall", host_stall).unwrap();
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(
            &lua,
            &engine,
            r#"
            local keep, counts, subs = {}, {}, 0
            local text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
            for round = 1, 40 do
                for i = 1, 3000 do
                    keep[i % 400 + 1] = { i, string.sub(text, i % 26 + 1, i % 26 + 1) .. i }
                end
                HostStall()
                table.insert(counts, string.format("%.3f", collectgarbage("count")))
            end
            print(table.concat(counts, ","))
            print(string.format("%.9f", os.clock()))
            "#,
        )
    };
    let quick = readings(false);
    let stalled = readings(true);
    assert_eq!(quick, stalled, "the collector stepped at other points when the host stalled");
}

/// Reading the clock costs what the call and its loop cost, however many reads a frame makes: every read
/// past 256 in a phase used to advance it 10 us more (2,000 brackets of a spawn replay read
/// 17 ms of clock for under 1 ms of work). A loop that only reads the clock still ends, on its own charges;
/// before the first frame (nothing is charged) the clock stands still and the drift stays.
#[test]
fn clock_reads_cost_their_work_not_a_flat_fee() {
    let (lua, engine) = start();
    // (interpreted as `ree record` and `ree run` run modules: the loop's own instructions are charged)
    engine_lua::modules::interpret_all(&lua, &engine);
    let before = run(
        &lua,
        &engine,
        r#"
        local t = os.clock()
        for _ = 1, 2000 do
            local _ = os.clock()
        end
        print(string.format("before the first frame %.1f", (os.clock() - t) * 1e3))
        "#,
    );
    let drift: f64 = before.iter().find_map(|line| line.strip_prefix("print: before the first frame ")).unwrap_or_else(|| panic!("{before:?}")).parse().unwrap();
    assert!(drift > 15.0, "reads before the first frame still end a busy loop: {drift} ms");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local t0 = os.clock()
        for _ = 1, 2000 do
            local _ = os.clock()
        end
        local reads = (os.clock() - t0) * 1e3
        local t1 = os.clock()
        local spins = 0
        while os.clock() - t1 < 0.002 do
            spins += 1
        end
        print(string.format("reads %.3f spins %s", reads, tostring(spins > 100)))
        "#,
    );
    let line = output.iter().find(|line| line.starts_with("print: reads")).unwrap_or_else(|| panic!("{output:?}"));
    let ms: f64 = line.split_whitespace().nth(2).unwrap().parse().unwrap();
    assert!(ms < 1.0, "2,000 reads took {ms} ms of clock: {line}");
    assert!(line.ends_with("spins true"), "{line}");
}

/// Studio runs a signal's handlers newest first (measured again 2026-10-23 in an empty test place and a lobby
/// playtest client: the BindableEvent, Changed, GetPropertyChangedSignal, ChildAdded, AttributeChanged,
/// Heartbeat, Stepped, PreSimulation, PostSimulation, PreAnimation, PreRender and RenderStepped handlers
/// connected A, B, C ran C, B, A), BindToRenderStep runs ascending priority and, within one, in bind order,
/// and QueryDescendants answers in tree order, a selector list one selector after the other (an instance two
/// match is listed twice), with no combinator in a space (filters on either side apply to the same instance)
/// and Studio's messages for a malformed query (a handler_order probe).
#[test]
fn signal_handlers_and_queries_run_in_studios_order() {
    // (a running game: the simulation events do not fire in edit mode)
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local RunService = game:GetService("RunService")
        local Seen = {}
        _G.Seen = Seen
        local function Three(Name, Event)
            Seen[Name] = {}
            for _, Id in { "A", "B", "C" } do
                Event:Connect(function() table.insert(Seen[Name], Id) end)
            end
        end
        local Bindable = Instance.new("BindableEvent")
        local Part = Instance.new("Part")
        Part.Parent = workspace
        Three("Bindable", Bindable.Event)
        Three("Changed", Part.Changed)
        Three("Property", Part:GetPropertyChangedSignal("Transparency"))
        Three("ChildAdded", workspace.ChildAdded)
        Three("Attribute", Part.AttributeChanged)
        for _, Name in { "Heartbeat", "Stepped", "PreSimulation", "PostSimulation", "PreAnimation", "PreRender", "RenderStepped" } do
            Three(Name, RunService[Name])
        end
        Bindable:Fire()
        Part.Transparency = 0.5
        Instance.new("Folder").Parent = workspace
        Part:SetAttribute("X", 1)
        local Bound = {}
        for _, Entry in { { "Late", 200 }, { "Same1", 100 }, { "Early", 50 }, { "Same2", 100 } } do
            RunService:BindToRenderStep("T" .. Entry[1], Entry[2], function()
                if not table.find(Bound, Entry[1]) then table.insert(Bound, Entry[1]) end
            end)
        end
        _G.Bound = Bound
        "##,
    );
    for _ in 0..3 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = run(
        &lua,
        &engine,
        r##"
        for Name, Order in _G.Seen do
            assert(#Order >= 3 and table.concat(Order, ",", 1, 3) == "C,B,A", Name .. " ran " .. table.concat(Order, ","))
        end
        assert(table.concat(_G.Bound, ",") == "Early,Same1,Same2,Late", table.concat(_G.Bound, ","))

        local Root = Instance.new("Folder")
        local function Make(Class, Name, Parent)
            local Child = Instance.new(Class)
            Child.Name = Name
            Child.Parent = Parent
            return Child
        end
        local M = Make("Model", "M", Root)
        local F = Make("Folder", "F", M)
        Make("Part", "X", F)
        Make("Part", "Y", M)
        local N = Make("Model", "N", Root)
        Make("Part", "Z", N)
        game:GetService("CollectionService"):AddTag(F, "T")
        game:GetService("CollectionService"):AddTag(M, "T")
        local function Query(Selector)
            local Ok, Found = pcall(function() return Root:QueryDescendants(Selector) end)
            if not Ok then return "ERROR " .. tostring(Found) end
            local Names = {}
            for _, Item in Found do table.insert(Names, Item.Name) end
            return table.concat(Names, ",")
        end
        -- (every row measured in Studio, 2026-10-23)
        for Selector, Expected in {
            ["Model > Part"] = "Y,Z", ["Model>Part"] = "Y,Z", ["Model  >  Part"] = "Y,Z", ["Model >> Part"] = "X,Y,Z",
            ["Model > Folder > Part"] = "X", ["Folder > Part"] = "X", ["#M >> #X"] = "X", ["#M > #X"] = "",
            ["Part Part"] = "X,Y,Z", [".T Part"] = "", ["Model .T"] = "M", ["Part #X"] = "X", ["#X Part"] = "X", ["#M #X"] = "",
            ["Model :has(>Part)"] = "M,N", ["Part :not(#Y)"] = "X,Z", ["Model Folder"] = "", [".T"] = "M,F", ["T"] = "",
            ["Model, Folder, Part"] = "M,N,F,X,Y,Z", ["Part, Model"] = "X,Y,Z,M,N", ["Part, Part"] = "X,Y,Z,X,Y,Z", ["Part, #X"] = "X,Y,Z,X",
            ["> Part"] = "", [">> Part"] = "X,Y,Z", ["Model > Part:not(#Y)"] = "Z", ["Model:not(:has(> Part))"] = "", [""] = "", [" "] = "",
            ["*"] = "ERROR Expected at least one filter", ["Part,"] = "ERROR Expected at least one combinator or a filter",
            [",Part"] = "ERROR Expected at least one filter", ["Part >"] = "ERROR Expected at least one filter", ["#"] = "ERROR Expected identifier for a name",
            ["Part:bad"] = "ERROR Pseudo-class 'bad' filter is not supported", ["Part)"] = "ERROR Unexpected trailing character: ')'",
            ["[Anchored]"] = "ERROR comparison operator expected after property name", ["Part[Size = 1]"] = "ERROR Property Size of type Vector3 is not supported for comparison",
        } do
            assert(Query(Selector) == Expected, string.format("%q gave %q, Studio %q", Selector, Query(Selector), Expected))
        end
        print("ok")
        "##,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// SharedTableRegistry:GetSharedTable makes the table of a new name and returns the same one after (measured
/// in Studio 2026-10-23); ree returned nil, so a rig module's bridge host counts were nil and its
/// PreAnimation handler failed 1,199 times in a lobby's boot.
#[test]
fn the_shared_table_registry_makes_a_table_per_name() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Registry = game:GetService("SharedTableRegistry")
        local Counts = Registry:GetSharedTable("Counts")
        assert(typeof(Counts) == "SharedTable", typeof(Counts))
        Counts["1"] = 7
        assert(Registry:GetSharedTable("Counts") == Counts and Registry:GetSharedTable("Counts")["1"] == 7, "the same table")
        assert(Registry:GetSharedTable("Other") ~= Counts, "a table per name")
        local Given = SharedTable.new({ k = 1 })
        Registry:SetSharedTable("Given", Given)
        assert(Registry:GetSharedTable("Given") == Given, "a set table is returned")
        print("ok")
        "#,
    );
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
}

/// A harness can add players to a run (a lobby's other players): a Player parented to Players is listed by
/// GetPlayers, found by GetPlayerByUserId and (through its Character) by GetPlayerFromCharacter, and fires
/// PlayerAdded, PlayerRemoving and CharacterAdded; the local player stays first and fires none of them.
#[test]
fn players_a_harness_adds_join_the_game() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Players = game:GetService("Players")
        Players.PlayerAdded:Connect(function(Player) print("added " .. Player.Name) end)
        Players.PlayerRemoving:Connect(function(Player) print("removing " .. Player.Name) end)
        local Local = Players.LocalPlayer
        assert(#Players:GetPlayers() == 1 and Players:GetPlayers()[1] == Local)
        local Other = Instance.new("Player")
        Other.Name = "Other"
        Other.UserId = 77
        Other.Parent = Players
        Other.CharacterAdded:Connect(function(Character) print("character " .. Character.Name) end)
        local Body = Instance.new("Model")
        Body.Name = "Body"
        Body.Parent = workspace
        assert(Players:GetPlayerFromCharacter(Body) == nil, "no player wears the model yet")
        Other.Character = Body
        local All = Players:GetPlayers()
        assert(#All == 2 and All[1] == Local and All[2] == Other, "GetPlayers: local first, then the added player")
        assert(Players:GetPlayerByUserId(77) == Other and Players:GetPlayerByUserId(78) == nil)
        assert(Players:GetPlayerFromCharacter(Body) == Other, "GetPlayerFromCharacter")
        Other.Parent = nil
        assert(#Players:GetPlayers() == 1, "a removed player is gone")
        "#,
    );
    let printed: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("print: ")).collect();
    assert_eq!(printed, ["added Other", "character Body", "removing Other"], "{output:?}");
}

/// BindableFunction.OnInvoke is assignable (the Studio dump lists no callbacks) and :Invoke runs it
/// with the caller's arguments and returns its results; a reassignment replaces it.
#[test]
fn bindable_function_on_invoke_runs_on_invoke() {
    assert_script(
        r#"
        local Function = Instance.new("BindableFunction")
        local Calls = 0
        Function.OnInvoke = function(A, B)
            Calls += 1
            return A + B, "sum"
        end
        local Sum, Label = Function:Invoke(2, 3)
        assert(Sum == 5 and Label == "sum" and Calls == 1, "Invoke did not run OnInvoke")
        Function.OnInvoke = function() return "second" end
        assert(Function:Invoke() == "second", "reassigning OnInvoke kept the first callback")
        assert(Function:Clone():Invoke() == "second", "a clone lost OnInvoke")
        assert(not pcall(function() Function.OnClientInvoke = function() end end), "OnClientInvoke is no BindableFunction member")
        "#,
    );
}

/// Every callback Roblox's API dump lets a game script assign (Security None; the Studio dump lists
/// none) is a callback member: assigned without an error, a read raising Studio's message. A client
/// sets RemoteFunction.OnClientInvoke (ClientCast's client script, a game's own handlers); offline
/// nothing invokes it, nor the others.
#[test]
fn script_callbacks_are_assignable() {
    assert_script(
        r#"
        local Remote = Instance.new("RemoteFunction")
        Remote.OnClientInvoke = function() return 1 end
        Remote.OnServerInvoke = function() return 2 end
        Remote.OnClientInvoke = nil
        local Ok, Message = pcall(function() return Remote.OnClientInvoke end)
        assert(not Ok and string.find(Message, "OnClientInvoke is a callback member of RemoteFunction; you can only set the callback value, get is not available", 1, true), Message)
        assert(not pcall(function() Remote.OnInvoke = function() end end), "OnInvoke is no RemoteFunction member")
        game:GetService("MarketplaceService").ProcessReceipt = function() return Enum.ProductPurchaseDecision.NotProcessedYet end
        local Chat = game:GetService("TextChatService")
        Chat.OnIncomingMessage = function() end
        Chat.OnBubbleAdded = function() end
        Chat.OnChatWindowAdded = function() end
        local Channel = Instance.new("TextChannel")
        Channel.OnIncomingMessage = function() end
        Channel.ShouldDeliverCallback = function() return true end
        game:GetService("SocialService").OnCallInviteInvoked = function() end
        game.OnClose = function() end
        "#,
    );
}

/// Actors (every expectation measured in a Studio client with an actor probe, 2026-10-23). A Script under an
/// Actor starts inside the assignment that
/// puts it in the DataModel (before the line after `Actor.Parent = ...`), or when it is enabled there; its `_G`
/// and its module copies are its own (the main VM's stay nil / 1); `script:GetActor()` is its Actor, a module's
/// is nil; ConnectParallel raises outside an Actor; a signal's parallel handlers run after its serial ones, and
/// task.synchronize / task.desynchronize move all of them together: both parallel, both synchronized, both
/// desynchronized, newest connection first.
#[test]
fn actors_run_their_scripts_in_a_vm_of_their_own_in_studios_order() {
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local RunService = game:GetService("RunService")
        local Shared = game:GetService("SharedTableRegistry"):GetSharedTable("ActorTest")
        SharedTable.update(Shared, "N", function() return 0 end)
        local function Add(Text)
            local Index = SharedTable.increment(Shared, "N", 1) + 1
            Shared["L" .. Index] = Text
        end
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local Module = Instance.new("ModuleScript")
        Module.Name = "Probe"
        Module.Source = "local Count = 0 local Actor = script:GetActor() return { Bump = function() Count += 1 return Count end, Actor = if Actor then Actor.Name else 'none' }"
        Module.Parent = Folder
        local Template = Instance.new("Script")
        Template.RunContext = Enum.RunContext.Client
        Template.Enabled = false
        Template.Source = [==[
            local RunService = game:GetService("RunService")
            local Shared = game:GetService("SharedTableRegistry"):GetSharedTable("ActorTest")
            local Actor = script:GetActor()
            local Probe = require(Actor.Parent.Probe)
            local function Add(Text)
                local Index = SharedTable.increment(Shared, "N", 1) + 1
                Shared["L" .. Index] = Actor.Name .. " " .. Text
            end
            _G.Seen = (_G.Seen or 0) + 1
            Add("start actor=" .. tostring(script:GetActor() == Actor) .. " module-actor=" .. Probe.Actor .. " bump=" .. Probe.Bump() .. " _G.Seen=" .. _G.Seen .. " sync=" .. tostring(pcall(task.synchronize)))
            RunService.PreSimulation:Connect(function() Add("serial") end)
            RunService.PreSimulation:ConnectParallel(function()
                Add("parallel")
                task.synchronize()
                Add("synchronized")
                task.desynchronize()
                Add("desynchronized")
            end)
            RunService.Heartbeat:ConnectParallel(function() Add("heartbeat") end)
        ]==]
        Template.Parent = Folder
        RunService.PreSimulation:Connect(function() Add("main first") end)
        for Index = 1, 2 do
            local Actor = Instance.new("Actor")
            Actor.Name = "Actor" .. Index
            local Worker = Template:Clone()
            Worker.Parent = Actor
            Worker.Enabled = true
            Add("placing " .. Actor.Name)
            Actor.Parent = Folder
            Add("placed " .. Actor.Name)
        end
        RunService.PreSimulation:Connect(function() Add("main second") end)
        Add("main bump=" .. require(Module).Bump() .. " _G.Seen=" .. tostring(_G.Seen))
        local Ok, Message = pcall(function() RunService.Heartbeat:ConnectParallel(function() end) end)
        Add("main ConnectParallel " .. tostring(Ok) .. " " .. tostring(Message):gsub("^.-:%d+: ", ""))
        -- an Actor whose worker is disabled starts it when it is enabled
        local Late = Instance.new("Actor")
        Late.Name = "Late"
        local Worker = Template:Clone()
        Worker.Parent = Late
        Late.Parent = Folder
        Add("late placed")
        Worker.Enabled = true
        Add("late enabled")
        "##,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r##"
        local Shared = game:GetService("SharedTableRegistry"):GetSharedTable("ActorTest")
        local Lines = {}
        for Index = 1, SharedTable.size(Shared) do
            if Shared["L" .. Index] then table.insert(Lines, Shared["L" .. Index]) end
        end
        print(table.concat(Lines, "|"))
        "##,
    );
    let log = output.iter().find_map(|line| line.strip_prefix("print: ")).expect("a log").to_string();
    let log: Vec<&str> = log.split('|').collect();
    let expected = [
        "placing Actor1",
        "Actor1 start actor=true module-actor=none bump=1 _G.Seen=1 sync=true",
        "placed Actor1",
        "placing Actor2",
        "Actor2 start actor=true module-actor=none bump=1 _G.Seen=1 sync=true",
        "placed Actor2",
        "main bump=1 _G.Seen=nil",
        "main ConnectParallel false Function Event.ConnectParallel can only be used inside scripts",
        "late placed",
        "Late start actor=true module-actor=none bump=1 _G.Seen=1 sync=true",
        "late enabled",
        // PreSimulation: the serial handlers newest first, then the parallel ones (newest first) in Studio's phases
        "Late serial",
        "main second",
        "Actor2 serial",
        "Actor1 serial",
        "main first",
        "Late parallel",
        "Actor2 parallel",
        "Actor1 parallel",
        "Late synchronized",
        "Actor2 synchronized",
        "Actor1 synchronized",
        "Late desynchronized",
        "Actor2 desynchronized",
        "Actor1 desynchronized",
        "Late heartbeat",
        "Actor2 heartbeat",
        "Actor1 heartbeat",
    ];
    assert_eq!(log, expected);
}

/// Script.Source and Script.RunContext are plugin-writable in Studio and ree has no plugin boundary: a script can
/// set them (the Actor probe builds its workers that way), and a module's new source is the one required.
#[test]
fn script_source_and_run_context_are_writable() {
    assert_script(
        r#"
        local Module = Instance.new("ModuleScript")
        Module.Source = "return 41 + 1"
        assert(require(Module) == 42, "a module's source is the one written")
        local Worker = Instance.new("Script")
        Worker.RunContext = Enum.RunContext.Client
        assert(Worker.RunContext == Enum.RunContext.Client, "RunContext was not written")
        assert(Worker.Enabled == true, "a Script starts enabled")
        Worker.Enabled = false
        assert(Worker.Enabled == false)
        "#,
    );
}

/// The work clock prices datatype operations from Studio's own numbers: the op_cost probe's class
/// `datatype` in a lobby playtest's client (the median of four runs, 2026-10-23), bands of
/// about +-35 % around them; before, every operation the engine served was a flat 15 ns (a CFrame product 17 ns where
/// Studio takes 62, a Lerp 48 where it takes 110, a Position read 17 where it takes 1.3). The allocating CFrame
/// operations vary 2-3x between runs in Studio (a client's heap makes the sweep dearer), so the bands are wide there.
#[test]
fn the_work_clock_prices_datatype_operations_as_the_studio_bench_measured() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Body)
            local N = 2000
            local start = os.clock()
            Body(N)
            local middle = os.clock()
            for _ = 1, N do end
            return ((middle - start) - (os.clock() - middle)) / N * 1e9
        end
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.1f ns, Studio %d-%d", Name, Value, Low, High))
        end
        local A = CFrame.new(1, 2, 3) * CFrame.Angles(0.3, 0.4, 0.5)
        local B = CFrame.new(4, 5, 6) * CFrame.Angles(0.6, 0.1, 0.2)
        local V, W = Vector3.new(1, 2, 3), Vector3.new(4, 5, 6)
        local K, L = Color3.new(0.2, 0.4, 0.6), Color3.new(0.8, 0.6, 0.4)

        -- the parity bench's figures (tools/cost-parity, a rendered client of a game's match place, nine sessions: each loop's ns a step,
        -- the loop included) where it has a row outside its held-out third, bands of about 1.3x around them; the rig frame's (a match
        -- place's playtest client) for the operations it has no row for
        Within("CFrame.Position", Per(function(N) for _ = 1, N do local _ = A.Position end end), 0, 6)
        -- the product and the inverse are priced as a handler meets them, a few times a frame (rig frame, 24.6 and
        -- 24.5 ns); the op_cost loop of the lobby client reads 62 and 69, an empty test place's 20 and 20
        Within("CFrame * CFrame", Per(function(N) for _ = 1, N do local _ = A * B end end), 17, 40)
        Within("CFrame * Vector3", Per(function(N) for _ = 1, N do local _ = A * V end end), 8, 17)
        Within("CFrame:Inverse", Per(function(N) for _ = 1, N do local _ = A:Inverse() end end), 17, 40)
        -- ToAxisAngle, ToWorldSpace and PointToWorldSpace from the rig frame as well (25.4, 26.6 and 24.4 ns: six runs of six
        -- readings in a match place's client), bands of 1.3x around them; the lobby client's op_cost loop reads 18, 55 and 12
        Within("CFrame:ToAxisAngle", Per(function(N) for _ = 1, N do local _, _ = A:ToAxisAngle() end end), 19, 33)
        Within("CFrame:ToWorldSpace", Per(function(N) for _ = 1, N do local _ = A:ToWorldSpace(B) end end), 20, 35)
        -- the bench: Lerp 75.7, ToObjectSpace 20.2, GetComponents 22.0, ToEulerAnglesXYZ 22.1, Rotation 18.1, + Vector3 22.1, Angles 41.3
        Within("CFrame:Lerp", Per(function(N) for _ = 1, N do local _ = A:Lerp(B, 0.5) end end), 58, 98)
        Within("CFrame:ToObjectSpace", Per(function(N) for _ = 1, N do local _ = A:ToObjectSpace(B) end end), 15, 27)
        Within("CFrame:PointToWorldSpace", Per(function(N) for _ = 1, N do local _ = A:PointToWorldSpace(V) end end), 14, 32)
        Within("CFrame:GetComponents", Per(function(N) for _ = 1, N do local _ = A:GetComponents() end end), 16, 29)
        Within("CFrame:ToEulerAnglesXYZ", Per(function(N) for _ = 1, N do local _, _, _ = A:ToEulerAnglesXYZ() end end), 15, 29)
        Within("CFrame.Rotation", Per(function(N) for _ = 1, N do local _ = A.Rotation end end), 13, 24)
        Within("CFrame + Vector3", Per(function(N) for _ = 1, N do local _ = A + V end end), 16, 29)
        -- FuzzyEq compares both parts of a pair a micro-stud apart (the bench: 26.5) and stops at the position of one a stud
        -- apart (15.5)
        Within("CFrame:FuzzyEq near", Per(function(N) local C = A * CFrame.new(1e-6, 0, 0) for _ = 1, N do local _ = A:FuzzyEq(C) end end), 20, 34)
        Within("CFrame:FuzzyEq moved", Per(function(N) for _ = 1, N do local _ = A:FuzzyEq(B) end end), 11, 21)
        Within("CFrame.Angles", Per(function(N) for _ = 1, N do local _ = CFrame.Angles(0.1, 0.2, 0.3) end end), 30, 54)
        -- (fromEulerAnglesXYZ has no row: Angles' price)
        Within("CFrame.fromEulerAnglesXYZ", Per(function(N) for _ = 1, N do local _ = CFrame.fromEulerAnglesXYZ(0.1, 0.2, 0.3) end end), 30, 54)
        -- (lookAt's row is held out, 43.3: its price stays the playtest client's)
        Within("CFrame.lookAt", Per(function(N) for _ = 1, N do local _ = CFrame.lookAt(V, W) end end), 60, 115)
        -- the bench: Color3:Lerp 29.4, UDim2.fromOffset 32.1, ColorSequence.new of two colours 39.5, NumberSequence.new of a number
        -- 63.4 and of two 30.3 (the collector's phase moves those two apart: a wide band)
        Within("Color3:Lerp", Per(function(N) for _ = 1, N do local _ = K:Lerp(L, 0.5) end end), 22, 39)
        Within("UDim2.fromOffset", Per(function(N) for _ = 1, N do local _ = UDim2.fromOffset(10, 10) end end), 24, 42)
        Within("ColorSequence.new", Per(function(N) for _ = 1, N do local _ = ColorSequence.new(K, L) end end), 30, 52)
        Within("NumberSequence.new", Per(function(N) for _ = 1, N do local _ = NumberSequence.new(0.5) end end), 28, 82)
        -- InstanceHandle (cost.rs HANDLE_GET, HANDLE_EQ; each row's fastest of six sessions): Get of a parented part 30.2, of
        -- none 12.2, == of two handles of one instance 25.2, new of an instance 45.7 (ree's served call, 55.6) and of none 27.6
        local Handle, Same, Empty = InstanceHandle.new(workspace), InstanceHandle.new(workspace), InstanceHandle.new(nil)
        Within("InstanceHandle:Get", Per(function(N) for _ = 1, N do local _ = Handle:Get() end end), 23, 39)
        Within("InstanceHandle:Get of none", Per(function(N) for _ = 1, N do local _ = Empty:Get() end end), 8, 16)
        Within("InstanceHandle ==", Per(function(N) for _ = 1, N do local _ = Handle == Same end end), 19, 33)
        Within("InstanceHandle.new", Per(function(N) for _ = 1, N do local _ = InstanceHandle.new(workspace) end end), 35, 70)
        Within("InstanceHandle.new(nil)", Per(function(N) for _ = 1, N do local _ = InstanceHandle.new(nil) end end), 18, 36)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// Vector2 and UDim2 values cost what a client pays for them (lune-roblox `datatype_price`; the parity bench, tools/cost-parity, a
/// rendered client of a game's match place, nine sessions, ns a loop step): a Vector2's X read 3.4 ns, its deprecated x the same (ree
/// served the lowercase names through a Luau wrapper, charged as a host call: 17.9 ns each), `+` 15.3 and `==` 9.8 (ree: 1.0 and 1.4,
/// the instruction alone), a UDim2's X.Offset 32.3 (a new UDim; ree: 5.4) and `-` 28.0 (ree: 1.0), a ColorSequence's Keypoints of five
/// 145.1 (a new table of new keypoints; ree: 2.9).
#[test]
fn vector2_and_udim2_members_and_arithmetic_cost_what_a_client_pays() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Body)
            local N = 2000
            local start = os.clock()
            Body(N)
            local middle = os.clock()
            for _ = 1, N do end
            return ((middle - start) - (os.clock() - middle)) / N * 1e9
        end
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.1f ns, a client %d-%d", Name, Value, Low, High))
        end
        local A, B = Vector2.new(3, 4), Vector2.new(1, 2)
        assert(A.x == A.X and A.y == A.Y and A.magnitude == A.Magnitude and A.unit == A.Unit, "the lowercase names read the same values")
        Within("Vector2.X", Per(function(N) for _ = 1, N do local _ = A.X end end), 1, 5)
        Within("Vector2.x", Per(function(N) for _ = 1, N do local _ = A.x end end), 1, 5)
        Within("Vector2 + Vector2", Per(function(N) for _ = 1, N do local _ = A + B end end), 11, 21)
        Within("Vector2 == Vector2", Per(function(N) for _ = 1, N do local _ = A == B end end), 6, 14)
        local U, V = UDim2.new(0.5, 10, 0.25, 4), UDim2.new(0.1, 2, 0.3, 8)
        Within("UDim2.X.Offset", Per(function(N) for _ = 1, N do local _ = U.X.Offset end end), 14, 34)
        Within("UDim2 - UDim2", Per(function(N) for _ = 1, N do local _ = U - V end end), 20, 36)
        local K = Color3.new(1, 0.5, 0)
        local S = ColorSequence.new({
            ColorSequenceKeypoint.new(0, K), ColorSequenceKeypoint.new(0.25, K), ColorSequenceKeypoint.new(0.5, K),
            ColorSequenceKeypoint.new(0.75, K), ColorSequenceKeypoint.new(1, K),
        })
        Within("ColorSequence.Keypoints", Per(function(N) for _ = 1, N do local _ = S.Keypoints end end), 105, 185)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// A GUI object's first touch in a frame costs a laid-out GUI's (cost.rs COLD_TOUCH_GUI): a client lays out and draws every GUI object
/// every frame, so a walk over many finds each one cold (a rendered client of a game's lobby, a grid of 300 cards: AbsolutePosition of
/// distinct cards 273 ns against 71 for one card, a UIGradient's Color read on distinct gradients 416 against 77, a Frame's
/// BackgroundColor3 written on distinct frames 698 against 318). ree charged a part's 45 ns for every class.
#[test]
fn a_gui_objects_first_touch_in_a_frame_costs_what_a_drawn_gui_pays() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Gui = Instance.new("ScreenGui")
        Gui.Name = "Cards"
        Gui.Parent = game:GetService("StarterGui")
        for Index = 1, 200 do
            local Card = Instance.new("Frame")
            Card.Parent = Gui
            Instance.new("UIGradient").Parent = Card
        end
        "#,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Cards = game:GetService("StarterGui").Cards:GetChildren()
        local Gradients = {}
        local function Per(Body)
            local start = os.clock()
            Body()
            return (os.clock() - start) / #Cards * 1e9
        end
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.0f ns, a client %d-%d", Name, Value, Low, High))
        end
        -- (GetChildren listed the cards without touching them: each is cold)
        local Cold = Per(function() for _, Card in Cards do local _ = Card.Visible end end)
        local Hot = Per(function() for _, Card in Cards do local _ = Card.Visible end end)
        Within("first read of a Frame", Cold, 230, 400)
        Within("second read of it", Hot, 20, 80)
        for Index, Card in Cards do
            Gradients[Index] = Card:FindFirstChildOfClass("UIGradient")
        end
        local Write = Per(function() for _, Gradient in Gradients do Gradient.Offset = Vector2.new(0.5, 0) end end)
        -- (a UIGradient write 0.20 us, a first touch 0.27, its dirtying 0.075: the grid's Offset on distinct gradients 539 ns)
        Within("first write of a UIGradient", Write, 420, 720)
        "#,
    );
}

/// Cold per-frame Luau (cost.rs LUAU_COLD_TABLE and below): a table, Luau closure or function a frame's script work touches first
/// costs what a client pays for finding it out of its caches, once a frame; a table made in the frame is not cold, and a walk run
/// twice in a frame pays it once. Two runs of the same script read the same clock.
#[test]
fn a_frames_first_touch_of_a_table_or_function_costs_what_a_client_pays_once_a_frame() {
    fn walks() -> Vec<String> {
        let (lua, engine) = start();
        engine_lua::modules::interpret_all(&lua, &engine);
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(
            &lua,
            &engine,
            r#"
            _G.Records = {}
            for Index = 1, 200 do
                _G.Records[Index] = { A = Index, B = true }
            end
            _G.Walk = function()
                local Sum = 0
                for _, Record in _G.Records do
                    Sum += Record.A
                end
                return Sum
            end
            "#,
        );
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let source = r#"
            local function Per(Body)
                local Start = os.clock()
                Body()
                return (os.clock() - Start) / 200 * 1e9
            end
            local Cold = Per(_G.Walk)
            local Hot = Per(_G.Walk)
            local Fresh = {}
            for Index = 1, 200 do
                Fresh[Index] = { A = Index, B = true }
            end
            local Made = Per(function()
                local Sum = 0
                for _, Record in Fresh do
                    Sum += Record.A
                end
            end)
            print(string.format("%.6f %.6f %.6f", Cold, Hot, Made))
        "#;
        let first = run(&lua, &engine, source);
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let mut next = run(&lua, &engine, source);
        next.drain(..first.len());
        [first, next].concat()
    }
    let one = walks();
    assert_eq!(one, walks(), "the same script reads the same clock in every run");
    for line in &one {
        let values: Vec<f64> = line.split_once(": ").unwrap().1.split(' ').map(|v| v.parse().unwrap()).collect();
        let (cold, hot, made) = (values[0], values[1], values[2]);
        // 200 records, the list and the walk's closure and function cold: 200 ns a record and about 1.3 ns of the rest (and, the frame
        // the walk first runs, its code not run the frame before: cost.rs LUAU_RARE_PROTO, 3.5 ns a record)
        let extra = cold - hot;
        assert!((200.0..206.0).contains(&extra), "a cold record {extra:.1} ns over the hot walk, a client 140-210");
        assert!((made - hot).abs() < 3.0, "a table made in the frame is hot: {made:.1} against {hot:.1} ns");
    }
}

/// Counting cold touches marks a bit of each object's header and keeps its lists off the VM's heap: what a script reads of memory
/// (collectgarbage("count"), gcinfo) is the same with it on and off, frame by frame, while tables are made and collected.
#[test]
fn counting_cold_touches_leaves_what_a_script_reads_of_memory_as_it_was() {
    fn readings(tracking: bool) -> Vec<String> {
        let (lua, engine) = start();
        engine_lua::modules::interpret_all(&lua, &engine);
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        if !tracking {
            crate::cost::cold_tracking(&lua, false);
        }
        run(&lua, &engine, "_G.Kept = {} _G.Readings = {}");
        for frame in 0..40 {
            run(
                &lua,
                &engine,
                &format!(
                    r#"
                    for Index = 1, 400 do
                        local Record = {{ A = Index, B = {{ Index }} }}
                        if Index % 3 == 0 then
                            table.insert(_G.Kept, Record)
                        end
                    end
                    -- (cold calls: a C function's, a datatype's, a function run every other frame)
                    local _ = math.noise({frame} * 0.1) + CFrame.new({frame}, 0, 0).X
                    if {frame} % 2 == 0 then
                        _G.Every = _G.Every or function(X) return X * 2 end
                        local _ = _G.Every({frame})
                    end
                    for _, Record in _G.Kept do
                        Record.A += 1
                    end
                    if {frame} % 10 == 9 then
                        table.clear(_G.Kept)
                    end
                    table.insert(_G.Readings, collectgarbage("count") .. "/" .. gcinfo())
                    "#
                ),
            );
            crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        }
        run(&lua, &engine, r#"print(table.concat(_G.Readings, " "))"#)
    }
    let on = readings(true);
    assert_eq!(on, readings(false), "memory readings with cold touches counted and without");
    assert_eq!(on.last().unwrap().split_once(": ").unwrap().1.split(' ').count(), 40);
}

/// A member's first use in a frame (cost.rs FIRST_READ, FIRST_WRITE, FIRST_CALL): the engine's code for a property or method runs
/// cold after the frame's render and physics steps, so the first write, read or call of it in a frame costs a client more than the
/// next one on another instance; in the next frame the first one pays again.
#[test]
fn a_members_first_use_in_a_frame_costs_the_engines_code_running_cold() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Gui = Instance.new("ScreenGui")
        Gui.Parent = game:GetService("StarterGui")
        _G.A, _G.B = Instance.new("Frame"), Instance.new("Frame")
        _G.A.Parent, _G.B.Parent = Gui, Gui
        "#,
    );
    let source = r#"
        local A, B = _G.A, _G.B
        -- (both instances touched first, so neither use below pays an instance's first touch)
        local _, _ = A.Name, B.Name
        local function Two(Use)
            local T0 = os.clock()
            Use(A)
            local T1 = os.clock()
            Use(B)
            local T2 = os.clock()
            return ((T1 - T0) - (T2 - T1)) * 1e6
        end
        local Write = Two(function(F) F.BackgroundTransparency = F.BackgroundTransparency + 0.25 end)
        local Read = Two(function(F) local _ = F.Size end)
        local Call = Two(function(F) local _ = F:GetAttribute("Missing") end)
        print(string.format("%.4f %.4f %.4f", Write, Read, Call))
    "#;
    // (a first frame for what is done once: a member's lookup, its value's first conversion)
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, source);
    for _ in 0..2 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let output = run(&lua, &engine, source);
        let values: Vec<f64> = output.last().unwrap().split_once(": ").unwrap().1.split(' ').map(|v| v.parse().unwrap()).collect();
        eprintln!("first use over the second (us): write {:.3}, read {:.3}, call {:.3}", values[0], values[1], values[2]);
        // (the write's own read of BackgroundTransparency is a first read too: a write's first use 1.4 us and a read's 0.5)
        assert!((values[0] - 1.9).abs() < 0.05, "a property's first write in a frame {:.2} us over the second, a client 1.1-1.65 (+ its read)", values[0]);
        assert!((values[1] - 0.5).abs() < 0.05, "a property's first read in a frame {:.2} us over the second, a client 0.3-1.5", values[1]);
        assert!((values[2] - 0.8).abs() < 0.05, "a method's first call in a frame {:.2} us over the second, a client 0.6-0.95", values[2]);
    }
}

/// The engine paths an effect's cast runs (cost.rs Path, first_cloned, first_destroyed): the frame's first clone, entry into the
/// Workspace, destroy, PivotTo, BulkMoveTo and Emit run the engine's code cold (a clone and a destroy each class's code too), so they
/// cost a client more than the same call right after on another instance; in the next frame the first pays again.
#[test]
fn an_effects_engine_paths_cost_their_code_running_cold_at_a_frames_first_use() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Template()
            local P = Instance.new("Part")
            P.Anchored = true
            local A = Instance.new("Attachment")
            A.Parent = P
            local E = Instance.new("ParticleEmitter")
            E.Parent = P
            return P
        end
        _G.A, _G.B = Template(), Template()
        _G.Folder = Instance.new("Folder")
        _G.Folder.Parent = workspace
        _G.Moved = {}
        for I = 1, 4 do
            local P = Instance.new("Part")
            P.Anchored = true
            P.Parent = workspace
            _G.Moved[I] = P
        end
        _G.Emitters = { Instance.new("ParticleEmitter"), Instance.new("ParticleEmitter") }
        _G.Emitters[1].Parent, _G.Emitters[2].Parent = _G.Moved[1], _G.Moved[2]
        "#,
    );
    let source = r#"
        local function Two(First, Second)
            local T0 = os.clock()
            local X = First()
            local T1 = os.clock()
            local Y = Second()
            local T2 = os.clock()
            return ((T1 - T0) - (T2 - T1)) * 1e6, X, Y
        end
        local M, E = _G.Moved, _G.Emitters
        local Here = CFrame.new(0, 50, 0)
        local Clone, CA, CB = Two(function() return _G.A:Clone() end, function() return _G.B:Clone() end)
        local Enter = Two(function() CA.Parent = _G.Folder end, function() CB.Parent = _G.Folder end)
        local Destroy = Two(function() CA:Destroy() end, function() CB:Destroy() end)
        local Pivot = Two(function() M[1]:PivotTo(Here) end, function() M[2]:PivotTo(Here) end)
        local Bulk = Two(function() workspace:BulkMoveTo({ M[3] }, { Here }) end, function() workspace:BulkMoveTo({ M[4] }, { Here }) end)
        local Emit = Two(function() E[1]:Emit(1) end, function() E[2]:Emit(1) end)
        print(string.format("%.3f %.3f %.3f %.3f %.3f %.3f", Clone, Enter, Destroy, Pivot, Bulk, Emit))
    "#;
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, source);
    for _ in 0..2 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let output = run(&lua, &engine, source);
        let values: Vec<f64> = output.last().unwrap().split_once(": ").unwrap().1.split(' ').map(|v| v.parse().unwrap()).collect();
        // (a method's first call 0.8 us more as well: cost.rs FIRST_CALL)
        assert!((values[0] - (0.8 + 25.0 + 30.0 + 12.0 + 35.0)).abs() < 0.1, "a frame's first clone of a part with an attachment and an emitter {:.2} us over the second, a client 100", values[0]);
        assert!((values[1] - 26.0).abs() < 0.1, "a frame's first part into the Workspace {:.2} us over the second, a client 26", values[1]);
        assert!((values[2] - (0.8 + 10.0 + 36.0 + 11.0 + 11.0)).abs() < 0.1, "a frame's first destroy of the same {:.2} us over the second, a client 62", values[2]);
        assert!((values[3] - 16.8).abs() < 0.1, "a frame's first PivotTo {:.2} us over the second, a client 17-23", values[3]);
        assert!((values[4] - 28.8).abs() < 0.1, "a frame's first BulkMoveTo {:.2} us over the second, a client 25-30", values[4]);
        assert!((values[5] - 5.2).abs() < 0.1, "a frame's first Emit {:.2} us over the second, a client 5.2", values[5]);
    }
}

/// A destroy undoes the engine's frame work on an instance in the Workspace since an earlier frame (cost.rs destroy_live): a part that
/// entered it a frame before costs more to destroy than one parented in the same frame, which the parity corpus destroys.
#[test]
fn an_instance_in_the_workspace_since_an_earlier_frame_costs_more_to_destroy() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, r#"_G.Old = Instance.new("Part") _G.Old.Parent = workspace"#);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local function Fresh()
            local P = Instance.new("Part")
            P.Parent = workspace
            return P
        end
        Fresh():Destroy()
        local Young = Fresh()
        local T0 = os.clock()
        _G.Old:Destroy()
        local T1 = os.clock()
        Young:Destroy()
        local T2 = os.clock()
        print(string.format("%.3f", ((T1 - T0) - (T2 - T1)) * 1e6))
        "#,
    );
    let extra: f64 = output.last().unwrap().split_once(": ").unwrap().1.parse().unwrap();
    // (the old part's touch is a first touch of the frame too: cost.rs COLD_TOUCH)
    assert!((extra - 4.445).abs() < 0.02, "a part a frame old {extra:.3} us dearer to destroy than one parented now, a client 4.4");
}

/// A lone instance's first touch in a frame (cost.rs COLD_TOUCH_ALONE): parts made one after another and touched in that order pay
/// the plain first touch; parts made among other instances pay the lone price, unless they were touched the frame before.
#[test]
fn a_lone_instances_first_touch_costs_more_than_one_made_with_the_last() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        _G.Together, _G.Lone = {}, {}
        for I = 1, 21 do
            _G.Together[I] = Instance.new("Part")
        end
        for I = 1, 21 do
            _G.Lone[I] = Instance.new("Part")
            for _ = 1, 3 do
                Instance.new("Folder")
            end
        end
        _G.Other = Instance.new("Folder")
        "#,
    );
    let source = r#"
        local function Pass(List)
            local T0 = os.clock()
            for I = 2, #List do
                local _ = List[I]:GetAttribute("Missing")
            end
            return (os.clock() - T0) / (#List - 1) * 1e6
        end
        local _ = _G.Other:GetAttribute("Missing")
        local _ = _G.Together[1]:GetAttribute("Missing")
        local Together = Pass(_G.Together)
        local _ = _G.Lone[1]:GetAttribute("Missing")
        local Lone = Pass(_G.Lone)
        print(string.format("%.4f", Lone - Together))
    "#;
    for _ in 0..2 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = run(&lua, &engine, source);
    let lone: f64 = output.last().unwrap().split_once(": ").unwrap().1.parse().unwrap();
    assert!((lone - 0.275).abs() < 0.005, "a lone part's first touch {lone:.3} us over one made with the last, a client 0.29");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(&lua, &engine, source);
    let again: f64 = output.last().unwrap().split_once(": ").unwrap().1.parse().unwrap();
    assert!(again.abs() < 0.005, "touched the frame before, a lone part pays the plain first touch: {again:.3} us more");
}

/// A move of a part no code touched in this frame or the one before reads it cold (cost.rs BULK_COLD_PART); a part a script moves every
/// frame pays none.
#[test]
fn a_part_moved_every_frame_pays_no_cold_move() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Group()
            local Parts, Places = {}, {}
            for I = 1, 8 do
                local P = Instance.new("Part")
                P.Anchored = true
                P.Parent = workspace
                Parts[I], Places[I] = P, CFrame.new(I * 4, 20, 0)
            end
            return { Parts, Places }
        end
        _G.Moved, _G.Other = Group(), Group()
        "#,
    );
    let source = r#"
        workspace:BulkMoveTo(_G.Other[1], _G.Other[2])
        local T0 = os.clock()
        workspace:BulkMoveTo(_G.Moved[1], _G.Moved[2])
        print(string.format("%.3f", (os.clock() - T0) * 1e6))
    "#;
    for _ in 0..2 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = run(&lua, &engine, source);
    let cold: f64 = output.last().unwrap().split_once(": ").unwrap().1.parse().unwrap();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(&lua, &engine, source);
    let warm: f64 = output.last().unwrap().split_once(": ").unwrap().1.parse().unwrap();
    assert!((cold - warm - 8.0 * 1.25).abs() < 0.01, "8 parts untouched for two frames {:.3} us dearer to move than the same moved the frame before", cold - warm);
}

/// A script's first call in a frame of a C function and of a datatype's operation (cost.rs LUAU_COLD_CFUNC, LUAU_COLD_HOSTFUNC), and a
/// function's first run in a frame when its code did not run the frame before (LUAU_RARE_PROTO): the code runs cold.
#[test]
fn a_frames_first_calls_of_library_and_datatype_functions_cost_their_code_running_cold() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    // (the prelude makes a library function's wrapper at a script's first use of it, and what a frame makes reads warm in it)
    run(&lua, &engine, "_G.Rare = function(X) return X + 1 end local _ = CFrame.new(0, 0, 0)");
    let source = r#"
        local function Two(First, Second)
            local T0 = os.clock()
            First()
            local T1 = os.clock()
            Second()
            local T2 = os.clock()
            return ((T1 - T0) - (T2 - T1)) * 1e6
        end
        local C = Two(function() return math.noise(0.5) end, function() return math.noise(1.5) end)
        local Host = Two(function() return CFrame.new(1, 2, 3) end, function() return CFrame.new(3, 2, 1) end)
        local T0 = os.clock()
        _G.Rare(1)
        local Rare = (os.clock() - T0) * 1e6
        print(string.format("%.4f %.4f %.4f", C, Host, Rare))
    "#;
    let mut rare = Vec::new();
    for _ in 0..2 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let output = run(&lua, &engine, source);
        let values: Vec<f64> = output.last().unwrap().split_once(": ").unwrap().1.split(' ').map(|v| v.parse().unwrap()).collect();
        assert!((values[0] - 2.5).abs() < 0.02, "a frame's first math.noise {:.3} us over the second, a client 0.9-4.0 for a C function", values[0]);
        assert!((values[1] - 0.9).abs() < 0.02, "a frame's first CFrame.new {:.3} us over the second, a client 0.55-2.5 for a datatype's", values[1]);
        rare.push(values[2]);
    }
    // (the first frame's call found the function's code not run the frame before; the second's found it run)
    assert!((rare[0] - rare[1] - 0.7).abs() < 0.01, "a function run first in a frame, not the frame before, {:.3} us dearer", rare[0] - rare[1]);
}

/// Vector3 members and methods on the work clock (the op_cost probe's `datatype` rows in a lobby playtest's client, the
/// medians of its runs): Magnitude 10.4 ns, Unit 12.3, Dot 9.4, Cross 9.6,
/// Max 12.2, Lerp 13.3, FuzzyEq 13.5. ree charged every one a function call to the engine's Luau (Magnitude and Unit 18,
/// Dot and Cross 33, Max 33, Lerp 45, FuzzyEq 43); the VM answers the two members itself now and the methods are a table
/// lookup. Lerp and FuzzyEq are still Luau functions (a host call each), so they only come down to about 21 and 19.
#[test]
fn vector3_members_and_methods_cost_what_studio_spent() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Body)
            local N = 2000
            local start = os.clock()
            Body(N)
            local middle = os.clock()
            for _ = 1, N do end
            return ((middle - start) - (os.clock() - middle)) / N * 1e9
        end
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.1f ns, Studio %d-%d", Name, Value, Low, High))
        end
        local V, W = Vector3.new(1, 2, 3), Vector3.new(4, 5, 6)
        -- Magnitude is calibrated on the client parity bench (9.0 ns + loop overhead; 10.4 in the op_cost loop)
        Within("Magnitude", Per(function(N) for _ = 1, N do local _ = V.Magnitude end end), 9, 17)
        Within("magnitude", Per(function(N) for _ = 1, N do local _ = V.magnitude end end), 9, 17)
        Within("Unit", Per(function(N) for _ = 1, N do local _ = V.Unit end end), 8, 16)
        Within("X", Per(function(N) for _ = 1, N do local _ = V.X end end), 0, 5)
        Within("Dot", Per(function(N) for _ = 1, N do local _ = V:Dot(W) end end), 7, 13)
        Within("Cross", Per(function(N) for _ = 1, N do local _ = V:Cross(W) end end), 7, 13)
        Within("Max", Per(function(N) for _ = 1, N do local _ = V:Max(W) end end), 8, 16)
        -- Lerp and FuzzyEq from the rig frame (a match place's client, medians of three runs: 31.2 and 35.9 ns; the op_cost
        -- loop of the lobby client reads 13.3 and 13.5), bands of 1.3x around them
        -- (the parity bench: Lerp 14.9 ns whole, where its counts read about 21)
        Within("Lerp", Per(function(N) for _ = 1, N do local _ = V:Lerp(W, 0.5) end end), 12, 28)
        Within("FuzzyEq", Per(function(N) for _ = 1, N do local _ = V:FuzzyEq(W) end end), 28, 47)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// A property write with a handler connected to Changed or GetPropertyChangedSignal costs the writer more than the same write with
/// none (op_cost signal rows in a match place's client, two sessions: a Transparency write alone 316-352 ns, with one Changed
/// connection +227..354, with a property signal connection +79..213, with both +365..410; the lobby client +318 for Changed, edit +474).
/// ree charged the write alone (the handler's thread resumes outside the writer's charge): 252 ns with a connection, 252 without.
#[test]
fn a_changed_connection_costs_the_writer_the_queueing_of_its_handler() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Part = Instance.new("Part")
        Part.Parent = workspace
        local function Noop() end
        local function Writes(Name)
            local N = 2000
            local start = os.clock()
            for _ = 1, N // 2 do
                Part.Transparency = 0.25
                Part.Transparency = 0.75
            end
            return (os.clock() - start) / N * 1e9
        end
        local Alone = Writes()
        local Changed = Part.Changed:Connect(Noop)
        local WithChanged = Writes()
        Changed:Disconnect()
        local Property = Part:GetPropertyChangedSignal("Transparency"):Connect(Noop)
        local WithProperty = Writes()
        local Both = Part.Changed:Connect(Noop)
        local WithBoth = Writes()
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: +%.0f ns, Studio +%d..%d", Name, Value, Low, High))
        end
        Within("Changed", WithChanged - Alone, 150, 400)
        Within("property signal", WithProperty - Alone, 100, 300)
        Within("both", WithBoth - Alone, 300, 500)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// The rest test a proxy rig module runs on every idle motor, priced as a bridge frame meets it (a cut that made it cheaper):
/// a bridge_ops probe takes each motor's Transform (the identity) from a rig once a frame and runs the test in
/// its old form (Position.Magnitude, then the angle of `Transform:Inverse() * CFrame.identity`) and in the new one
/// (`Transform == CFrame.identity` returns first), in an empty test place's client and in a match place's (three runs): the old
/// form reads 91-129 ns (x1 108-129, x8 91-99), the new one 16-28, so a cut of 70-100 ns per idle motor. ree read 182
/// and 6 (a cut of 176) while it priced the operations from the op_cost loop of the lobby client (Inverse 69, `*` 62,
/// `==` 5), and Studio's whole rig-cut gain came out 2.6x smaller than ree's.
#[test]
fn the_rest_test_of_an_idle_motor_costs_what_a_rig_frame_measured() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Body)
            local N = 2000
            local start = os.clock()
            Body(N)
            local middle = os.clock()
            for _ = 1, N do end
            return ((middle - start) - (os.clock() - middle)) / N * 1e9
        end
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.1f ns, Studio %d-%d", Name, Value, Low, High))
        end
        local Identity = CFrame.identity
        local Transform = CFrame.identity * CFrame.identity
        local function GetAngle(Value)
            local _, Angle = Value:ToAxisAngle()
            return math.abs(Angle)
        end
        local Old = Per(function(N)
            for _ = 1, N do
                local AtRest = if Transform.Position.Magnitude > 0.0001 then false else GetAngle(Transform:Inverse() * Identity) <= 0.0001
            end
        end)
        local New = Per(function(N)
            for _ = 1, N do
                local AtRest = if Transform == Identity then true elseif Transform.Position.Magnitude > 0.0001 then false else GetAngle(Transform:Inverse()) <= 0.0001
            end
        end)
        Within("rest test before the cut", Old, 85, 140)
        Within("rest test after the cut", New, 15, 40)
        Within("the cut", Old - New, 60, 105)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// The deferred handlers of a write made in Stepped or PreSimulation run between PreSimulation and PostSimulation, before the
/// Motor6D batch (a deferred_phase probe in a match place's client): a write in PreRender, PreAnimation,
/// PostSimulation or Heartbeat had its order right; ree flushed after PostSimulation. The marks of one steady frame are Studio's.
#[test]
fn a_presimulation_writes_deferred_handlers_run_before_postsimulation_and_the_motor_batch() {
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local RunService = game:GetService("RunService")
        local Part = Instance.new("Part")
        Part.Anchored = true
        Part.Parent = workspace
        local Marks, Recording = {}, false
        local function Mark(Name) if Recording then table.insert(Marks, Name) end end
        Part.Changed:Connect(function() Mark("C") end)
        Part.AttributeChanged:Connect(function() Mark("A") end)
        workspace.ChildAdded:Connect(function() Mark("K") end)
        for Name, Letter in { PreRender = "R", PreAnimation = "N", Stepped = "S", PreSimulation = "P", PostSimulation = "Q", Heartbeat = "H" } do
            RunService[Name]:Connect(function() Mark(Letter) end)
        end
        _G.Sequences = {}
        local Index = 0
        local Writers = { "Stepped", "PreSimulation" }
        local Writer
        local function Write()
            Mark(">")
            Part.Transparency = if Part.Transparency > 0.4 then 0.1 else 0.6
            Part:SetAttribute("B", os.clock())
            Instance.new("Folder").Parent = workspace
        end
        task.spawn(function()
            for _, Name in Writers do
                local Connection = RunService[Name]:Connect(Write)
                for _ = 1, 3 do RunService.Heartbeat:Wait() end
                Marks, Recording = {}, true
                RunService.Heartbeat:Wait()
                Recording = false
                _G.Sequences[Name] = table.concat(Marks)
                Connection:Disconnect()
            end
            -- a Motor6D.Transform a deferred handler writes is applied the same frame
            local Base = Instance.new("Part")
            Base.Anchored = true
            Base.Position = Vector3.new(0, 100, 0)
            Base.Parent = workspace
            local Arm = Instance.new("Part")
            Arm.Position = Vector3.new(0, 100, 0)
            Arm.Parent = workspace
            local Motor = Instance.new("Motor6D")
            Motor.Part0, Motor.Part1, Motor.Parent = Base, Arm, Base
            local Round, Same = 0, {}
            RunService.PreSimulation:Connect(function()
                Round += 1
                Part.Transparency = if Part.Transparency > 0.4 then 0.1 else 0.6
            end)
            Part.Changed:Connect(function(Property)
                if Property == "Transparency" then Motor.Transform = CFrame.new(0, 0, Round) end
            end)
            RunService.PostSimulation:Connect(function()
                Same[Round] = math.abs(Arm.Position.Z - Base.Position.Z - Round) < 0.01
            end)
            for _ = 1, 8 do RunService.Heartbeat:Wait() end
            _G.Same = Same[5] and Same[6]
            _G.Done = true
        end)
        "##,
    );
    for _ in 0..40 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    run(
        &lua,
        &engine,
        r##"
        assert(_G.Done, "the probe did not finish")
        assert(_G.Sequences.Stepped == "HRN>SPCAKQ", "write in Stepped: " .. tostring(_G.Sequences.Stepped) .. ", Studio HRN>SPCAKQ")
        assert(_G.Sequences.PreSimulation == "HRNS>PCAKQ", "write in PreSimulation: " .. tostring(_G.Sequences.PreSimulation) .. ", Studio HRNS>PCAKQ")
        assert(_G.Same, "a Motor6D.Transform written by the deferred handler was not applied in the same frame (Studio: it is)")
        print("ok")
        "##,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// A signal's firer and its handlers' threads on the work clock (a handler_cost probe, a match place's playtest client, three
/// runs, 100 instances triggered a frame, one handler each): Changed queues for 0.2 us and its handler's thread costs 1.83 us,
/// AttributeChanged 0.55 and 1.78, ChildAdded 0.40 and 1.50, DescendantAdded 0.27 and 1.44, each scaled by the quarter the parity
/// bench's client reads them at (cost.rs signal_costs: a heard BindableEvent:Fire through its handler 0.64 us) but Changed's
/// queueing, which it reads directly. ree charged the thread nothing before, so a handler-heavy frame (a roster rebuild on every
/// ChildAdded) read low. Bands are +-35 %.
#[test]
fn signals_cost_their_firer_the_queueing_and_each_handler_its_thread() {
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local RunService = game:GetService("RunService")
        local N, PER = 100, 20
        local Parts, Items = {}, {}
        for Index = 1, N do
            local Part = Instance.new("Part")
            Part.Parent = workspace
            Parts[Index] = Part
            Items[Index] = Instance.new("Folder")
        end
        local Container = Instance.new("Folder")
        Container.Parent = workspace
        local Last, Count = 0, 0
        local function Handler() Last = os.clock() Count += 1 end
        local function Each(Get) local List = {} for Index = 1, N do table.insert(List, Get(Parts[Index]):Connect(Handler)) end return List end
        local Kinds = {
            { "Changed", function() return Each(function(P) return P.Changed end) end,
                function(Frame) for Index = 1, N do Parts[Index].Transparency = (Frame % 2) * 0.5 end end },
            { "AttributeChanged", function() return Each(function(P) return P.AttributeChanged end) end,
                function(Frame) for Index = 1, N do Parts[Index]:SetAttribute("A", Frame) end end },
            { "ChildAdded", function() return { Container.ChildAdded:Connect(Handler) } end,
                function() for Index = 1, N do Items[Index].Parent = Container end end,
                function() for Index = 1, N do Items[Index].Parent = nil end end },
            { "DescendantAdded", function() return { Container.DescendantAdded:Connect(Handler) } end,
                function() for Index = 1, N do Items[Index].Parent = Container end end,
                function() for Index = 1, N do Items[Index].Parent = nil end end },
        }
        local function Median(List) table.sort(List) return List[(#List + 1) // 2] end
        _G.Rows = {}
        task.spawn(function()
            for _, Kind in Kinds do
                local Alone, Queued, Run = {}, {}, {}
                for _, Connected in { false, true } do
                    local List = if Connected then Kind[2]() else {}
                    for Frame = 1, PER do
                        RunService.PreSimulation:Wait()
                        Count = 0
                        local T0 = os.clock()
                        Kind[3](Frame)
                        local T1 = os.clock()
                        RunService.PostSimulation:Wait()
                        if Frame > 4 then
                            if Connected then
                                table.insert(Queued, T1 - T0)
                                if Count == N then table.insert(Run, Last - T1) end
                            else
                                table.insert(Alone, T1 - T0)
                            end
                        end
                        if Kind[4] then Kind[4]() end
                    end
                    for _, Connection in List do Connection:Disconnect() end
                end
                local Base = Median(Alone)
                _G.Rows[Kind[1]] = { Queued = (Median(Queued) - Base) / N * 1e9, Run = Median(Run) / N * 1e9 }
            end
            _G.Done = true
        end)
        "##,
    );
    for _ in 0..200 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    run(
        &lua,
        &engine,
        r##"
        assert(_G.Done, "the probe did not finish")
        local function Within(Name, Value, Low, High)
            assert(Value >= Low and Value <= High, string.format("%s: %.0f ns, Studio %d-%d", Name, Value, Low, High))
        end
        local R = _G.Rows
        Within("Changed queued", R.Changed.Queued, 150, 400)
        Within("Changed handler thread", R.Changed.Run, 295, 610)
        Within("AttributeChanged queued", R.AttributeChanged.Queued, 90, 185)
        Within("AttributeChanged handler thread", R.AttributeChanged.Run, 290, 595)
        Within("ChildAdded queued", R.ChildAdded.Queued, 65, 135)
        Within("ChildAdded handler thread", R.ChildAdded.Run, 240, 500)
        Within("DescendantAdded queued", R.DescendantAdded.Queued, 44, 90)
        Within("DescendantAdded handler thread", R.DescendantAdded.Run, 230, 480)
        print("ok")
        "##,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// CFrame:ToEulerAngles(order) and CFrame.fromEulerAngles(rx, ry, rz, order) exist and agree with Studio for all six
/// Enum.RotationOrder items (an euler probe in an empty test place, 2026-10-23; ree had neither, and
/// ToEulerAnglesYXZ / ToOrientation were free on the work clock): the angles come back in x, y, z order whatever the
/// rotation order, an unknown order raises Studio's message, and the default is XYZ.
#[test]
fn cframe_euler_angles_follow_the_rotation_order_as_studio_does() {
    assert_script(
        r#"
        local function Close(Name, Got, Want)
            for Index = 1, 3 do
                assert(math.abs(Got[Index] - Want[Index]) < 2e-6, string.format("%s[%d]: %.6f, Studio %.6f", Name, Index, Got[Index], Want[Index]))
            end
        end
        local Cf = CFrame.new(1, 2, 3) * CFrame.Angles(0.3, 0.4, 0.5)
        -- Studio, cf1 of the probe
        Close("default", { Cf:ToEulerAngles() }, { 0.3, 0.4, 0.5 })
        Close("XYZ", { Cf:ToEulerAngles(Enum.RotationOrder.XYZ) }, { 0.3, 0.4, 0.5 })
        Close("XZY", { Cf:ToEulerAngles(Enum.RotationOrder.XZY) }, { 0.509615, 0.448958, 0.457359 })
        Close("YZX", { Cf:ToEulerAngles(Enum.RotationOrder.YZX) }, { 0.334475, 0.224767, 0.593186 })
        Close("YXZ", { Cf:ToEulerAngles(Enum.RotationOrder.YXZ) }, { 0.275670, 0.416649, 0.619884 })
        Close("ZXY", { Cf:ToEulerAngles(Enum.RotationOrder.ZXY) }, { 0.453041, 0.207013, 0.513380 })
        Close("ZYX", { Cf:ToEulerAngles(Enum.RotationOrder.ZYX) }, { 0.461591, 0.185872, 0.605050 })
        -- a CFrame with large angles
        local Big = CFrame.Angles(-0.7, 1.1, 2.9)
        Close("big XZY", { Big:ToEulerAngles(Enum.RotationOrder.XZY) }, { 2.225426, 2.029783, 0.108737 })
        Close("big ZYX", { Big:ToEulerAngles(Enum.RotationOrder.ZYX) }, { 1.156342, -0.532523, 2.107389 })
        -- fromEulerAngles: Studio's components for (0.3, 0.4, 0.5) in each order
        local function Rotation(Order)
            local Made = if Order then CFrame.fromEulerAngles(0.3, 0.4, 0.5, Order) else CFrame.fromEulerAngles(0.3, 0.4, 0.5)
            local Parts = { Made:GetComponents() }
            return { Parts[4], Parts[5], Parts[6], Parts[10], Parts[11], Parts[12] }
        end
        Close("from XYZ", Rotation(), { 0.808307, -0.441580, 0.389418, -0.184803, 0.437702, 0.879923 })
        Close("from XZY", Rotation(Enum.RotationOrder.XZY), { 0.808307, -0.479426, 0.341747, -0.241530, 0.259343, 0.935096 })
        Close("from YXZ", Rotation(Enum.RotationOrder.YXZ), { 0.863480, -0.340587, 0.372026, -0.211251, 0.425568, 0.879923 })
        Close("from ZYX", Rotation(Enum.RotationOrder.ZYX), { 0.808307, -0.357020, 0.468163, -0.389418, 0.272192, 0.879923 })
        -- a round trip in every order
        for _, Order in Enum.RotationOrder:GetEnumItems() do
            local Rx, Ry, Rz = Cf:ToEulerAngles(Order)
            local Again = CFrame.fromEulerAngles(Rx, Ry, Rz, Order)
            Close("round trip " .. Order.Name, { Again.RightVector.X, Again.UpVector.Y, Again.LookVector.Z }, { Cf.RightVector.X, Cf.UpVector.Y, Cf.LookVector.Z })
        end
        local Ok, Message = pcall(function() return Cf:ToEulerAngles("XYZ") end)
        assert(not Ok and Message == "Invalid value for enum 'RotationOrder'", tostring(Message))
        -- (nil passed as the order is no order, as a 0.747 playing client answers: tools/parity/probes/datatype_errors.luau)
        Ok, Message = pcall(function() return CFrame.fromEulerAngles(0.3, 0.4, 0.5, nil) end)
        assert(not Ok and Message == "Invalid value for enum 'RotationOrder'", tostring(Message))
        "#,
    );
}

/// Made-up place ids for the work clock's place prices ([`priced_places`])
const LOBBY: f64 = 1.0;
const MATCH: f64 = 2.0;
const TEST_MATCH: f64 = 3.0;

/// The measured place prices (cost.rs) on made-up places: a lobby with a localization table (56.1 us of lookup for a
/// one-letter text) and two match places of the same experience (61.2 us, and the geometry write factor).
fn priced_places() -> crate::cost::PlacePrices {
    crate::cost::PlacePrices { localized: vec![(LOBBY, 55.66 + 0.4286), (MATCH, 61.2), (TEST_MATCH, 61.2)], matched: vec![MATCH, TEST_MATCH] }
}

/// What a Text write costs in `place_id` with `prices` in a client (`mode` "play") or an edit DataModel ("edit"), in us a write:
/// letters with AutoLocalize on, AutoLocalize off, digits only, forty characters
fn text_write_prices(place_id: f64, prices: crate::cost::PlacePrices, mode: &str) -> Vec<f64> {
    let engine = Engine::new(Options { place_id, place_prices: prices, mode: mode.into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Label, A, B)
            local start = os.clock()
            for _ = 1, 100 do
                Label.Text = A
                Label.Text = B
            end
            return (os.clock() - start) / 200 * 1e6
        end
        local On = Instance.new("TextLabel")
        local Off = Instance.new("TextLabel")
        Off.AutoLocalize = false
        local Forty = string.rep("a", 40)
        print(string.format("%.2f %.2f %.2f %.2f", Per(On, "Hello", "World"), Per(Off, "Hello", "World"), Per(On, "1234", "5678"), Per(On, Forty, string.rep("b", 40))))
        "#,
    );
    let line = lines(&engine).into_iter().find_map(|line| line.strip_prefix("print: ").map(str::to_string)).expect("a result");
    line.split(' ').map(|number| number.parse::<f64>().unwrap()).collect::<Vec<f64>>()
}

/// A client looks a Text write up for translation when AutoLocalize is on and the text has a letter in it, whatever the place holds
/// (the parity bench, tools/cost-parity: "Hello" / "World" 35.0 us a write in a rendered client of a game's match place, 35.1-36.2 in
/// clients of an empty place with no localization table, headless or rendered; 0.47 with digits only), so every client pays
/// cost.rs LOCALIZATION_LOOKUP (32.8 us for one letter, 0.43 a letter more, as a lobby playtest client measured: 58.4-62.2 us for
/// "Hello" / "World" and 71.7-75.5 for forty characters there, with Studio's own text capture in it); a place [`crate::cost::PlacePrices`]
/// lists pays its own figure. An edit DataModel (an empty place's measured 1.0 us) and a server have no translator. ree charged the
/// lookup only in the listed places, so a client of any other place paid 1/75 of a lettered HUD's rewrite.
#[test]
fn a_client_pays_the_autolocalize_lookup_on_a_lettered_text_write() {
    let any = text_write_prices(0.0, crate::cost::PlacePrices::default(), "play");
    assert!((30.0..40.0).contains(&any[0]), "a client, letters: {any:?}");
    assert!((0.3..1.2).contains(&any[1]), "a client, AutoLocalize off: {any:?}");
    assert!((0.3..1.2).contains(&any[2]), "a client, digits: {any:?}");
    assert!((45.0..56.0).contains(&any[3]), "a client, forty characters: {any:?}");
    let lobby = text_write_prices(LOBBY, priced_places(), "play");
    assert!((50.0..68.0).contains(&lobby[0]) && (63.0..84.0).contains(&lobby[3]), "a listed place pays its own lookup: {lobby:?}");
    let edit = text_write_prices(LOBBY, priced_places(), "edit");
    assert!(edit.iter().all(|us| (0.3..1.5).contains(us)), "an edit DataModel looks nothing up: {edit:?}");
}

/// `REE_LOCALIZED_PLACES` / `REE_MATCH_PLACES` (launch.rs) add places to the prices: `<id>[=<us>]` and `<id>`, a place
/// listed already keeps its entry, nothing set changes nothing, and a value that is not a number is refused.
#[test]
fn place_prices_take_the_places_a_command_line_adds() {
    let base = crate::cost::PlacePrices { localized: vec![(LOBBY, 56.1)], matched: Vec::new() };
    let added = base.clone().with(Some("2, 3=40.5,1=99"), Some("3,4")).unwrap();
    assert_eq!(added.localized, [(LOBBY, 56.1), (2.0, crate::cost::LOCALIZATION_LOOKUP), (3.0, 40.5)]);
    assert_eq!(added.matched, [3.0, 4.0]);
    assert_eq!(base.clone().with(None, Some("")).unwrap(), base);
    assert!(base.clone().with(Some("lobby"), None).unwrap_err().contains("REE_LOCALIZED_PLACES"));
    assert!(base.with(None, Some("2,x")).unwrap_err().contains("REE_MATCH_PLACES"));
}

/// What fires when a model arrives in a watched folder, in Studio's order (an arrival probe, 166 lines identical in ree, in an
/// empty test place's edit DataModel (Immediate signals) and in its client, and in a lobby's client (Deferred signals),
/// 2026-10-23): for each instance of the subtree in GetDescendants' order the
/// DescendantAdded of each ancestor, nearest first, then ChildAdded, then AncestryChanged children before parents, then the
/// moved instance's own Changed("Parent"). A move between two folders under one root tells the root nothing, and the old folder
/// gets DescendantRemoving for the subtree (depth first) and then ChildRemoved before the new one's events. ree fired
/// ChildAdded first, DescendantAdded breadth first and per ancestor, AncestryChanged in reverse, Changed(Parent) before all.
#[test]
fn a_model_arriving_in_a_folder_fires_its_events_in_studios_order() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Log = {}
        _G.Log = Log
        local function Add(Text) table.insert(Log, Text) end
        local Root = Instance.new("Folder") Root.Name = "Root" Root.Parent = workspace
        local Units = Instance.new("Folder") Units.Name = "Units" Units.Parent = Root
        local Other = Instance.new("Folder") Other.Name = "Other" Other.Parent = Root
        local function Model(Name, ...) local M = Instance.new("Model") M.Name = Name for _, C in { ... } do C.Parent = M end return M end
        local U = Model("U", Model("A", Model("A1"), Model("A2")), Model("B", Model("B1")), Model("C"))
        for _, Node in U:GetDescendants() do
            Node.AncestryChanged:Connect(function(Child, Parent) Add("Ancestry " .. Node.Name .. " " .. Child.Name .. " " .. (Parent and Parent.Name or "nil")) end)
        end
        U.AncestryChanged:Connect(function(Child, Parent) Add("Ancestry U " .. Child.Name .. " " .. (Parent and Parent.Name or "nil")) end)
        U.Changed:Connect(function(Property) Add("U.Changed " .. Property) end)
        U:GetPropertyChangedSignal("Parent"):Connect(function() Add("U.Parent") end)
        for _, Folder in { Units, Other } do
            Folder.ChildAdded:Connect(function(C) Add(Folder.Name .. ".ChildAdded " .. C.Name) end)
            Folder.ChildRemoved:Connect(function(C) Add(Folder.Name .. ".ChildRemoved " .. C.Name) end)
            Folder.DescendantAdded:Connect(function(D) Add(Folder.Name .. ".DescendantAdded " .. D.Name) end)
            Folder.DescendantRemoving:Connect(function(D) Add(Folder.Name .. ".DescendantRemoving " .. D.Name) end)
        end
        Root.DescendantAdded:Connect(function(D) Add("Root.DescendantAdded " .. D.Name) end)
        Root.DescendantRemoving:Connect(function(D) Add("Root.DescendantRemoving " .. D.Name) end)
        U.Parent = Units
        U.Parent = Other
        "#,
    );
    let output = run(&lua, &engine, r#"print(table.concat(_G.Log, "|"))"#);
    let log = output.iter().find_map(|line| line.strip_prefix("print: ")).expect("a log");
    let arrival = "Units.DescendantAdded U|Root.DescendantAdded U|Units.DescendantAdded A|Root.DescendantAdded A|Units.DescendantAdded A1|Root.DescendantAdded A1|\
Units.DescendantAdded A2|Root.DescendantAdded A2|Units.DescendantAdded B|Root.DescendantAdded B|Units.DescendantAdded B1|Root.DescendantAdded B1|\
Units.DescendantAdded C|Root.DescendantAdded C|Units.ChildAdded U|\
Ancestry A1 U Units|Ancestry A2 U Units|Ancestry A U Units|Ancestry B1 U Units|Ancestry B U Units|Ancestry C U Units|Ancestry U U Units|U.Changed Parent|U.Parent";
    let moved = "Units.DescendantRemoving U|Units.DescendantRemoving A|Units.DescendantRemoving A1|Units.DescendantRemoving A2|Units.DescendantRemoving B|\
Units.DescendantRemoving B1|Units.DescendantRemoving C|Units.ChildRemoved U|\
Other.DescendantAdded U|Other.DescendantAdded A|Other.DescendantAdded A1|Other.DescendantAdded A2|Other.DescendantAdded B|Other.DescendantAdded B1|\
Other.DescendantAdded C|Other.ChildAdded U|\
Ancestry A1 U Other|Ancestry A2 U Other|Ancestry A U Other|Ancestry B1 U Other|Ancestry B U Other|Ancestry C U Other|Ancestry U U Other|U.Changed Parent|U.Parent";
    assert_eq!(log, format!("{arrival}|{moved}"));
}

/// SharedTable.increment adds to a key that exists and returns the old value, and raises for one that does not (Studio, 2026-10-23: `Unable to
/// increment the value for key "M" because it does not exist in the SharedTable.`; ree made the key).
#[test]
fn shared_table_increment_needs_the_key_as_studio_does() {
    assert_script(
        r#"
        local Counts = SharedTable.new({ N = 5 })
        assert(SharedTable.increment(Counts, "N", 2) == 5 and Counts.N == 7, "increment adds and returns the old value")
        local Ok, Message = pcall(SharedTable.increment, Counts, "M", 1)
        assert(not Ok and string.find(tostring(Message), 'Unable to increment the value for key "M" because it does not exist in the SharedTable.', 1, true), tostring(Message))
        assert(Counts.M == nil, "a failed increment gave the key a value")
        "#,
    );
}

/// A SharedTable is a userdata, as in Roblox: the table functions refuse it and the VM names it, it takes Roblox's keys and
/// values, converts tables, and raises Roblox's messages, the key's at the script's line (measured in a playing client, Studio
/// 0.748; ree's was a plain table that took anything). A frozen table converts a table written to it before it says it is
/// frozen, as a client's does (two runs; ree said it was frozen first).
#[test]
fn a_shared_table_is_a_userdata_with_roblox_keys_and_values() {
    assert_script(
        r##"
        local function Fails(Message, F, ...)
            local Ok, E = pcall(F, ...)
            assert(not Ok and string.gsub(tostring(E), "^[^\n]-:%d+: ", "") == Message, tostring(E))
        end
        local function Positioned(F)
            local _, E = pcall(F)
            return string.match(tostring(E), "^[^\n]-:%d+: ") ~= nil
        end
        local S = SharedTable.new({ a = 1, b = "two", [1] = 10, [2] = 20 }) :: any
        assert(type(S) == "userdata" and typeof(S) == "SharedTable" and getmetatable(S) == "The metatable is locked")
        assert(string.match(tostring(S), "^SharedTable: 0x%x+$") and #tostring(S) == 31, tostring(S))
        Fails("attempt to get length of a SharedTable value", function() return #S end)
        Fails("invalid argument #1 to 'next' (table expected, got SharedTable)", next, S)
        Fails("invalid argument #1 to 'rawget' (table expected, got SharedTable)", rawget, S, "a")
        Fails("invalid argument #1 to 'insert' (table expected, got SharedTable)", table.insert, S, 1)
        Fails("attempt to call missing method 'size' of SharedTable", function() return S:size() end)
        Fails("attempt to perform arithmetic (add) on SharedTable and number", function() return S + 1 end)
        -- keys: strings, and integers from 0 to 2^32 - 1
        assert(S.a == 1 and S[1] == 10 and S["1"] == nil and S.missing == nil)
        S[3.0] = "three"
        S[4294967295] = true
        assert(S[3] == "three" and S[4294967295] == true)
        Fails("SharedTable keys can only be strings or numbers.", function() S[true] = 1 end)
        Fails("SharedTable keys can only be strings or numbers.", function() return S[Vector3.one] end)
        for _, Key in { -1, 1.5, 2 ^ 32, 0 / 0, math.huge } do
            Fails("Numeric SharedTable keys must be nonnegative integers less than 2^32", function() S[Key] = 1 end)
        end
        Fails("Numeric SharedTable keys must be nonnegative integers less than 2^32", function() return S[-1] end)
        -- values: kept, copied, converted, dropped or refused
        S.v, S.c, S.u = Vector3.new(1, 2, 3), CFrame.new(1, 2, 3), UDim2.new(1, 2, 3, 4)
        assert(S.v == Vector3.new(1, 2, 3) and typeof(S.c) == "CFrame" and S.u == UDim2.new(1, 2, 3, 4))
        local B = buffer.create(1)
        S.buffer = B
        buffer.writeu8(B, 0, 7)
        assert(buffer.readu8(S.buffer, 0) == 0 and S.buffer ~= B and S.buffer ~= S.buffer)
        S.t = { x = { y = 1 } }
        assert(typeof(S.t) == "SharedTable" and typeof(S.t.x) == "SharedTable" and S.t.x.y == 1 and S.t == S.t)
        local Inner = SharedTable.new()
        S.inner = Inner
        assert(S.inner == Inner)
        local Sub = { 1 }
        local Twice = SharedTable.new({ a = Sub, b = Sub, inner = Inner })
        assert(Twice.a ~= Twice.b and Twice.inner == Inner)
        S.thread, S.enum, S.curve = coroutine.create(print), Enum.KeyCode, RotationCurveKey.new(0, CFrame.new(), Enum.KeyInterpolationMode.Linear)
        assert(S.thread == nil and S.enum == nil and S.curve == nil)
        Fails("Functions can not be stored in a SharedTable", function() S.f = print end)
        Fails("Instances can not be stored in a SharedTable", function() S.i = workspace end)
        Fails("DateTimes can not be stored in a SharedTable", function() S.d = DateTime.now() end)
        Fails("RaycastParamss can not be stored in a SharedTable", function() S.r = RaycastParams.new() end)
        Fails("EventInstances can not be stored in a SharedTable", function() S.e = workspace.Changed end)
        Fails("ValueCurveKeys can not be stored in a SharedTable", function() S.k = ValueCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear) end)
        S.capabilities = SecurityCapabilities.new()
        assert(typeof(S.capabilities) == "SecurityCapabilities")
        Fails("Functions can not be stored in a SharedTable", function() S.t = { f = print } end)
        local Cyclic = {}
        Cyclic.Self = Cyclic
        Fails("Cannot convert a cyclic table to a SharedTable", function() S.cyclic = Cyclic end)
        S.a = nil
        assert(S.a == nil)
        -- the key first, then a table value's conversion, then the frozen flag, then any other value (a client's frozen table
        -- written { print } or a cyclic table raised the conversion's error, two runs); only the key's errors (and the library's
        -- arguments) are positioned
        local F = SharedTable.cloneAndFreeze(SharedTable.new({ y = 2 })) :: any
        Fails("SharedTable is frozen", function() F.y = 3 end)
        Fails("SharedTable is frozen", function() F.y = print end)
        Fails("SharedTable is frozen", function() F.y = { 1 } end)
        Fails("Functions can not be stored in a SharedTable", function() F.y = { print } end)
        Fails("Cannot convert a cyclic table to a SharedTable", function() F.y = Cyclic end)
        Fails("SharedTable keys can only be strings or numbers.", function() F[true] = 1 end)
        assert(F.y == 2)
        assert(Positioned(function() S[true] = 1 end) and Positioned(function() return SharedTable.size(1 :: any) end))
        assert(not Positioned(function() F.y = 3 end) and not Positioned(function() S.f = print end))
        "##,
    );
}

/// A SharedTable's loop walks a snapshot taken when it starts: the integer keys ascending, then the string keys in the order
/// of Roblox's hash trie, whatever order they were added in (each order below printed by a playing client, Studio 0.748).
#[test]
fn a_shared_table_iterates_in_robloxs_order() {
    assert_script(
        r##"
        local function Keys(S)
            local Out = {}
            for K in S do
                table.insert(Out, tostring(K))
            end
            return table.concat(Out, " ")
        end
        local function Fill(List)
            local S = SharedTable.new()
            for _, K in List do
                S[K] = 1
            end
            return S
        end
        local Letters = string.split("a b c d e f g h i j k l m n o p", " ")
        local Reversed = {}
        for I = #Letters, 1, -1 do
            table.insert(Reversed, Letters[I])
        end
        assert(Keys(Fill(Letters)) == "e i b g k l a p n c d h m f j o", Keys(Fill(Letters)))
        assert(Keys(Fill(Reversed)) == "e i b g k l a p n c d h m f j o")
        assert(Keys(Fill(string.split("alpha beta gamma delta epsilon zeta eta theta", " "))) == "delta gamma alpha theta epsilon zeta beta eta")
        local Many = {}
        for I = 1, 40 do
            Many[I] = "k" .. I
        end
        local Want = "k17 k37 k39 k35 k32 k22 k14 k30 k16 k36 k33 k38 k15 k34 k23 k1 k10 k29 k7 k25 k19 k20 k2 k12 k9 k27 k5 k28 k11 k24 k6 k8 k13 k26 k4 k21 k3 k18 k40 k31"
        assert(Keys(Fill(Many)) == Want, Keys(Fill(Many)))
        assert(Keys(Fill({ 9, 3, 100, 0, 7, 4294967295, 12 })) == "0 3 7 9 12 100 4294967295")
        assert(Keys(SharedTable.new({ b = 1, a = 2, [3] = "c", [1] = "a", z = 3, [2] = "b", [10] = "j" })) == "1 2 3 10 b a z")
        local S = Fill({ "a", "b", "c", "d" })
        S.b = nil
        S.e = 1
        assert(Keys(S) == "e a c d")
        S.b = 1
        assert(Keys(S) == "e b a c d")
        -- a snapshot: values changed, keys removed or added during the loop are not seen
        local T = SharedTable.new({ [1] = "a", [2] = "b" })
        local Seen = {}
        for K, V in T do
            if K == 1 then
                T[2] = "changed"
            end
            table.insert(Seen, V)
        end
        assert(table.concat(Seen, " ") == "a b" and T[2] == "changed")
        local U, Count = SharedTable.new({ a = 1, b = 2, c = 3 }), 0
        for K in U do
            Count += 1
            U[K .. "x"] = 1
        end
        assert(Count == 3 and SharedTable.size(U) == 6)
        "##,
    );
}

/// The SharedTable library as a playing client has it (Studio 0.748): its copies, freezing, increment and update, and their
/// checks in Roblox's order with Roblox's messages.
#[test]
fn the_shared_table_library_follows_roblox() {
    assert_script(
        r##"
        local function Fails(Message, F, ...)
            local Ok, E = pcall(F, ...)
            assert(not Ok and string.gsub(tostring(E), "^[^\n]-:%d+: ", "") == Message, tostring(E))
        end
        local Any = SharedTable :: any
        assert(SharedTable.size(SharedTable.new()) == 0 and SharedTable.size(SharedTable.new({ a = 1 }, 5)) == 1)
        Fails("invalid argument #1 to 'new' (table expected, got nil)", Any.new, nil)
        Fails("invalid argument #1 to 'new' (table expected, got SharedTable)", Any.new, SharedTable.new())
        Fails("attempt to modify a readonly table", function() Any.extra = 1 end)
        -- copies: a shallow one shares the SharedTables it holds, a deep one copies them; cloneAndFreeze freezes the copies
        local S = SharedTable.new({ t = { 1 }, n = 1 })
        local Shallow, Deep = SharedTable.clone(S), SharedTable.clone(S, true)
        assert(Shallow ~= S and Shallow.t == S.t and Deep.t ~= S.t and Deep.t[1] == 1)
        local Frozen, DeepFrozen = SharedTable.cloneAndFreeze(S), SharedTable.cloneAndFreeze(S, true)
        assert(SharedTable.isFrozen(Frozen) and not SharedTable.isFrozen(Frozen.t) and Frozen.t == S.t)
        assert(SharedTable.isFrozen(DeepFrozen.t) and DeepFrozen.t ~= S.t and not SharedTable.isFrozen(SharedTable.clone(Frozen)))
        -- a deep copy copies each SharedTable it reaches once (a playing client, Studio 0.748): a table held twice is one copy, one
        -- that holds itself (twice) holds its copy, a ring stays a ring, and shallow clones are copies of their own whose shared
        -- table is copied once; frozen too by cloneAndFreeze
        local A = SharedTable.new({ x = 1 })
        local Twice = SharedTable.new()
        Twice.a, Twice.b, Twice[1] = A, A, A
        local D, DF = SharedTable.clone(Twice, true), SharedTable.cloneAndFreeze(Twice, true)
        assert(D.a == D.b and D.a == D[1] and D.a ~= A and DF.a == DF.b and SharedTable.isFrozen(DF.a) and not SharedTable.isFrozen(D.a))
        local Own = SharedTable.new()
        Own.self, Own.again = Own, Own
        local DOwn = SharedTable.clone(Own, true)
        assert(DOwn.self == DOwn and DOwn.again == DOwn and DOwn.self ~= Own)
        local R1, R2 = SharedTable.new(), SharedTable.new()
        R1.next, R2.next = R2, R1
        local DR = SharedTable.clone(R1, true)
        assert(DR.next.next == DR and DR.next ~= R2)
        local Inner = SharedTable.new({ y = 1 })
        local T = SharedTable.new({ inner = Inner, n = 1 })
        local V = SharedTable.new()
        V.c1, V.c2, V.t = SharedTable.clone(T), SharedTable.clone(T), T
        local DV = SharedTable.clone(V, true)
        assert(DV.c1 ~= DV.c2 and DV.c1 ~= DV.t and DV.c1.inner == DV.c2.inner and DV.c2.inner == DV.t.inner and DV.c1.inner ~= Inner)
        local Empty = SharedTable.clone(SharedTable.new(), true)
        assert(SharedTable.size(Empty) == 0)
        Fails("invalid argument #1 to 'clone' (SharedTable expected, got table)", Any.clone, {})
        Fails("invalid argument #2 to 'clone' (boolean expected, got string)", Any.clone, S, "x")
        Fails("invalid argument #2 to 'cloneAndFreeze' (boolean expected, got nil)", Any.cloneAndFreeze, S, nil)
        Fails("invalid argument #1 to 'size' (SharedTable expected, got string)", Any.size, "x")
        Fails("invalid argument #1 to 'isFrozen' (SharedTable expected, got table)", Any.isFrozen, {})
        Fails("SharedTable is frozen", SharedTable.clear, Frozen)
        -- a shallow copy and its table each see only their own writes, clear and increment
        local Base = SharedTable.new({ n = 1, t = { 1 } })
        local Copy, FrozenCopy = SharedTable.clone(Base), SharedTable.cloneAndFreeze(Base)
        Base.n = 2
        Copy.x = 1
        assert(Copy.n == 1 and FrozenCopy.n == 1 and Base.x == nil and Base.n == 2 and Copy.t == Base.t)
        assert(not SharedTable.isFrozen(Base) and SharedTable.isFrozen(FrozenCopy) and not SharedTable.isFrozen(Copy))
        SharedTable.clear(Copy)
        SharedTable.increment(Base, "n", 1)
        assert(SharedTable.size(Copy) == 0 and SharedTable.size(Base) == 2 and Base.n == 3 and FrozenCopy.n == 1)
        local C = SharedTable.new({ a = 1, b = 2 })
        SharedTable.clear(C)
        assert(SharedTable.size(C) == 0)
        -- increment: the key, the frozen flag, the delta, then the entry
        local N = SharedTable.new({ n = 5, s = "x", [7] = 1 })
        assert(SharedTable.increment(N, "n", 2) == 5 and N.n == 7 and SharedTable.increment(N, 7, 1.5) == 1 and N[7] == 2.5)
        Fails('Unable to increment the value for key "m" because it does not exist in the SharedTable.', SharedTable.increment, N, "m", 1)
        Fails('Unable to increment the value for key "5" because it does not exist in the SharedTable.', SharedTable.increment, N, 5, 1)
        Fails('Unable to increment the value for key "s" because it is not a number value.', SharedTable.increment, N, "s", 1)
        Fails("Variant cast failed", Any.increment, N, "n", "2")
        Fails("Variant cast failed", Any.increment, N, "m", nil)
        Fails("SharedTable is frozen", Any.increment, SharedTable.cloneAndFreeze(N), "n", "2")
        Fails("SharedTable keys can only be strings or numbers.", Any.increment, N, true, 1)
        -- update: the callback gets the old value and its result is stored as a write would; its error reaches the caller
        local U = SharedTable.new({ n = 2 })
        local Count, Old
        SharedTable.update(U, "n", function(...)
            Count, Old = select("#", ...), ...
            return Old * 3
        end)
        assert(Count == 1 and Old == 2 and U.n == 6)
        SharedTable.update(U, "m", function(V) Old = V return { 1 } end)
        assert(Old == nil and typeof(U.m) == "SharedTable")
        SharedTable.update(U, "n", function() return nil end)
        assert(U.n == nil)
        Fails("Functions can not be stored in a SharedTable", SharedTable.update, U, "m", function() return print end)
        Fails("Third argument to SharedTable::Update should be a function.", Any.update, SharedTable.cloneAndFreeze(U), "m", 5)
        Fails("SharedTable is frozen", SharedTable.update, SharedTable.cloneAndFreeze(U), "m", function() return 1 end)
        local Thrown = { code = 1 }
        local Ok, E = pcall(SharedTable.update, U, "m", function() error(Thrown) end)
        assert(not Ok and E == Thrown)
        local Ok2, E2 = pcall(SharedTable.update, U, "m", function() error("boom") end)
        assert(not Ok2 and string.match(E2, "^Test:%d+: boom$"), E2)
        "##,
    );
}

/// SharedTableRegistry:SetSharedTable takes a SharedTable, or nil to forget the name (Studio 0.748: anything else raises
/// "Unable to cast double to SharedTable", a table named an Array or a Dictionary); a SharedTable a BindableEvent or a
/// BindableFunction delivers, alone or in a table, is a new handle of the same table (Studio 0.748, a playing client, two runs:
/// `==` true, rawequal false, tostring different, for Fire and for Invoke's arguments and results). ree delivered the handle sent.
#[test]
fn shared_tables_meet_the_registry_and_bindables_as_in_roblox() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r##"
        local Registry = game:GetService("SharedTableRegistry")
        local function Cast(Value)
            local _, E = pcall(Registry.SetSharedTable, Registry, "x", Value)
            return tostring(E)
        end
        print(Cast(5), Cast("s"), Cast(true), Cast({}), Cast({ a = 1 }), Cast(workspace))
        local Given = SharedTable.new({ k = 1 })
        Registry:SetSharedTable("Given", Given)
        local Same = Registry:GetSharedTable("Given") == Given
        Registry:SetSharedTable("Given", nil)
        print(Same, Registry:GetSharedTable("Given") ~= Given)
        local Event = Instance.new("BindableEvent")
        Event.Event:Connect(function(Alone, Held)
            print("fired", Alone == Given, Held.s == Given, rawequal(Alone, Given), rawequal(Held.s, Given), tostring(Alone) == tostring(Given))
        end)
        Event:Fire(Given, { s = Given })
        local Function = Instance.new("BindableFunction")
        Function.OnInvoke = function(Got, Held) return Got, rawequal(Got, Given), Got == Given, rawequal(Held[1], Given), Held[1] == Given end
        local Back, Raw, Same, HeldRaw, HeldSame = Function:Invoke(Given, { Given })
        print("invoked", Raw, Same, HeldRaw, HeldSame, rawequal(Back, Given), Back == Given)
        "##,
    );
    assert!(output.contains(&"print: Unable to cast double to SharedTable Unable to cast string to SharedTable Unable to cast bool to SharedTable Unable to cast Array to SharedTable Unable to cast Dictionary to SharedTable Unable to cast Instance to SharedTable".to_string()), "{output:?}");
    assert!(output.contains(&"print: true true".to_string()), "{output:?}");
    assert!(output.contains(&"print: invoked false true false true false true".to_string()), "{output:?}");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    assert!(lines(&engine).contains(&"print: fired true true false false false".to_string()), "{:?}", lines(&engine));
}

/// SharedTableRegistry casts a name to a string as a playing client does (Studio 0.748): a number as `%.20g` writes it, a
/// subnormal one too (its digits, where the scripts' float mode flushed Luau's `string.format` to "0"), NaN and the infinities
/// as "NAN", "INF" and "-INF"; any other value warns ("argument #1 expects a string, but boolean was passed") and is named as
/// the engine writes its type (a Vector3 with nine digits a component, where tostring writes doubles; a buffer by its
/// contents and every SharedTable by one name, which no text reaches). SetSharedTable checks its value before its name, and
/// names a value it refuses as a client's cast does.
#[test]
fn shared_table_registry_names_cast_as_a_client_does() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r##"
        local Registry = game:GetService("SharedTableRegistry")
        local function Err(...)
            local Ok, E = pcall(...)
            return if Ok then "ok" else tostring(E)
        end
        print(Err(Registry.SetSharedTable, Registry, nil, 5), "|", Err(Registry.SetSharedTable, Registry), "|", Err(Registry.GetSharedTable, Registry))
        local function Named(Name, Text)
            local T = SharedTable.new()
            Registry:SetSharedTable(Name, T)
            return Registry:GetSharedTable(Text) == T and Registry:GetSharedTable(Name) == T
        end
        print("numbers", Named(15, "15"), Named(0.1, "0.10000000000000000555"), Named(1 / 3, "0.33333333333333331483"), Named(2 ^ 63, "9223372036854775808"),
            Named(1e20, "1e+20"), Named(-0.0, "-0"), Named(0 / 0, "NAN"), Named(math.huge, "INF"), Named(-math.huge, "-INF"))
        print("subnormal", Named(tonumber("1e-310"), "9.9999999999999694493e-311"), Named(tonumber("5e-324"), "4.9406564584124654418e-324"),
            Named(tonumber("2.2250738585072009e-308"), "2.225073858507200889e-308"))
        print("others", Named(true, "true"), Named({ 1 }, "{}"), Named(workspace, "Instance"), Named(print, "function"), Named(coroutine.create(print), "nil"),
            Named(Enum.Material, "nil"), Named(newproxy(), "nil"), Named(Enum.Material.Plastic, "Plastic"), Named(Vector3.new(1, 2, 3), "1, 2, 3"))
        print("vectors", Named(Vector3.new(0.1, 1 / 3, 2), "0.100000001, 0.333333343, 2"), Named(Vector3.new(1e20, 123456.789, -0.0), "1.00000002e+20, 123456.789, -0"),
            Named(Vector3.new(0 / 0, math.huge, -math.huge), "NAN, INF, -INF"), Named(Vector2.new(0.1, 1 / 3), "0.100000001, 0.333333343"),
            Named(CFrame.new(0.1, 0, 0), "0.100000001, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1"))
        local T, Other = SharedTable.new(), SharedTable.new()
        Registry:SetSharedTable(buffer.fromstring("abc"), T)
        Registry:SetSharedTable(SharedTable.new(), Other)
        print("apart", Registry:GetSharedTable(buffer.fromstring("abc")) == T, Registry:GetSharedTable(buffer.fromstring("abd")) == T,
            Registry:GetSharedTable("abc") == T, Registry:GetSharedTable(SharedTable.new({ 1 })) == Other)
        local function Cast(Value)
            return Err(Registry.SetSharedTable, Registry, "x", Value)
        end
        print(Cast(print), "|", Cast(coroutine.create(print)), "|", Cast(Enum.Material.Plastic), "|", Cast(Enum), "|", Cast(newproxy()), "|", Cast(CFrame.new()))
        "##,
    );
    assert!(output.contains(&"print: Unable to cast double to SharedTable | Argument 1 missing or nil | Argument 1 missing or nil".to_string()), "{output:?}");
    assert!(output.contains(&"print: numbers true true true true true true true true true".to_string()), "{output:?}");
    assert!(output.contains(&"print: subnormal true true true".to_string()), "{output:?}");
    assert!(output.contains(&"print: others true true true true true true true true true".to_string()), "{output:?}");
    assert!(output.contains(&"print: vectors true true true true true".to_string()), "{output:?}");
    assert!(output.contains(&"print: apart true false false true".to_string()), "{output:?}");
    assert!(
        output.contains(&"print: Unable to cast Function to SharedTable | Unable to cast null to SharedTable | Unable to cast Material to SharedTable | Unable to cast null to SharedTable | Unable to cast null to SharedTable | Unable to cast CoordinateFrame to SharedTable".to_string()),
        "{output:?}"
    );
    let warned = |kind: &str| output.iter().any(|line| line.contains(&format!("argument #1 expects a string, but {kind} was passed")) && !line.starts_with("print"));
    assert!(["boolean", "table", "Instance", "function", "thread", "Enum", "userdata", "EnumItem", "Vector3", "buffer", "SharedTable"].iter().all(|kind| warned(kind)), "{output:?}");
    assert!(!warned("number"), "{output:?}");
}

/// A Vector3 and a number given to a method's string parameter are written as a client writes them (measured, Studio 0.748:
/// MarkerCurve:InsertMarkerAtTime's value and LogService:Info's message on a playing client, PhysicsService's group names on
/// a server): a Vector3 with nine significant digits a component, NaN and the infinities as "NAN", "INF" and "-INF" (tostring
/// writes doubles and "nan"), and a number as `%.20g` writes it, a subnormal one with its digits. LogService wrote numbers as
/// tostring does ("0.1", "1e-310") and a Vector3 as tostring does, as MarkerCurve and PhysicsService wrote a Vector3.
#[test]
fn string_parameters_cast_a_vector3_and_a_number_as_a_client_does() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Values = { Vector3.new(0.1, 1 / 3, 2), Vector3.new(0 / 0, math.huge, -math.huge), tonumber("1e-310"), 0.1 }
        local Curve = Instance.new("MarkerCurve")
        local PhysicsService = game:GetService("PhysicsService")
        local LogService = game:GetService("LogService")
        for I, Value in Values do
            Curve:InsertMarkerAtTime(I, Value)
            PhysicsService:RegisterCollisionGroup(Value)
            LogService:Info(Value)
        end
        local Markers, Groups = {}, {}
        for _, Marker in Curve:GetMarkers() do
            table.insert(Markers, Marker.Value)
        end
        for _, Group in PhysicsService:GetRegisteredCollisionGroups() do
            table.insert(Groups, Group.name)
        end
        print("markers", table.concat(Markers, " | "))
        print("groups", table.concat(Groups, " | "))
        "#,
    );
    let written = "0.100000001, 0.333333343, 2 | NAN, INF, -INF | 9.9999999999999694493e-311 | 0.10000000000000000555";
    assert!(output.contains(&format!("print: markers {written}")), "{output:?}");
    assert!(output.contains(&format!("print: groups Default | {written}")), "{output:?}");
    let logged: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("info: ")).collect();
    assert_eq!(logged.join(" | "), written, "{output:?}");
}

/// An Instance method's string parameter reads any other value as a client reads it (measured, Studio 0.747, a playing client and
/// a server: tools/parity/probes/string_parameter_methods.luau): a number as `%.20g` writes it, anything else as the engine
/// writes its type ("true", "Instance", an EnumItem's name, "nil" for an Enum, "EventInstance" for a signal, "{}") after a
/// warning with the script's stack, `argument #1 expects a string, but <type> was passed`, and the method then answers as for
/// that text; nil raises "Argument 1 missing or nil"; WaitForChild, which yields, casts without the warning. ree matched no
/// name but a string's: FindFirstChild(5) was nil, IsA(workspace) false, GetAttribute(nil) nil, and nothing warned.
#[test]
fn instance_methods_read_a_string_parameter_as_a_client_does() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Holder = Instance.new("Folder")
        for _, Text in { "5", "0.10000000000000000555", "true", "Instance", "Camera", "nil", "EventInstance", "{}", "1e+20" } do
            local Child = Instance.new("Folder")
            Child.Name = Text
            Child.Parent = Holder
            pcall(Holder.SetAttribute, Holder, Text, Text)
            Holder:AddTag(Text)
        end
        local Root = Instance.new("Camera")
        Root.Parent = Holder
        local function Name(Found)
            return if Found then Found.Name else "none"
        end
        print("find", Name(Holder:FindFirstChild(5)), Name(Holder:FindFirstChild(0.1)), Name(Holder:FindFirstChild(1e20)), Name(Holder:FindFirstChild(true)),
            Name(Holder:FindFirstChild(workspace)), Name(Holder:FindFirstChild(Enum.RenderPriority.Camera)), Name(Holder:FindFirstChild(Enum.Material)),
            Name(Holder:FindFirstChild(workspace.Changed)), Name(Holder:FindFirstChild({})))
        print("dot", Name(Holder.FindFirstChild(Holder, 5)), Name(Holder.FindFirstChild(Holder, true)))
        print("wait", Name(Holder:WaitForChild(5)), Name(Holder:WaitForChild(true)), Name(Holder:WaitForChild(workspace)))
        print("class", Root:IsA(workspace), Root:IsA(Enum.RenderPriority.Camera), Root:IsA(5), Name(Holder:FindFirstChildWhichIsA(workspace)),
            Name(Holder:FindFirstChildOfClass(Enum.RenderPriority.Camera)), Name(Holder:FindFirstChildOfClass(true)))
        print("attribute", Holder:GetAttribute(5), Holder:GetAttribute(0.1), Holder:GetAttribute(true))
        print("tag", Holder:HasTag(5), Holder:HasTag(true), Holder:HasTag(workspace), Holder:HasTag(7))
        print("missing", select(2, pcall(Holder.GetAttribute, Holder, nil)), select(2, pcall(Holder.IsA, Holder)),
            select(2, pcall(function() return Root:IsA(nil) end)), select(2, pcall(function() return Holder:HasTag() end)),
            select(2, pcall(Holder.FindFirstAncestor, Holder, nil)))
        print("property", select(2, pcall(Holder.GetPropertyChangedSignal, Holder, 0.1)), select(2, pcall(Holder.GetPropertyChangedSignal, Holder, Vector3.new(0.1, 1 / 3, 2))))
        local Calls = {}
        function Calls.Nested()
            return Holder:FindFirstAncestor(true)
        end
        Calls.Nested()
        "#,
    );
    assert!(output.contains(&"print: find 5 0.10000000000000000555 1e+20 true Instance Camera nil EventInstance {}".to_string()), "{output:?}");
    assert!(output.contains(&"print: dot 5 true".to_string()), "{output:?}");
    assert!(output.contains(&"print: wait 5 true Instance".to_string()), "{output:?}");
    assert!(output.contains(&"print: class true true false 5 Camera none".to_string()), "{output:?}");
    assert!(output.contains(&"print: attribute 5 0.10000000000000000555 true".to_string()), "{output:?}");
    assert!(output.contains(&"print: tag true true true false".to_string()), "{output:?}");
    assert!(output.contains(&format!("print: missing{}", " Argument 1 missing or nil".repeat(5))), "{output:?}");
    assert!(
        output.contains(&"print: property 0.10000000000000000555 is not a valid property name. 0.100000001, 0.333333343, 2 is not a valid property name.".to_string()),
        "{output:?}"
    );
    // a warning for each value that is neither a string nor a number, WaitForChild's none
    let warned: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("warn: argument #1 expects a string, but ")).collect();
    let kinds = "boolean Instance EnumItem Enum RBXScriptSignal table boolean Instance EnumItem Instance EnumItem boolean boolean boolean Instance Vector3 boolean";
    assert_eq!(warned.iter().map(|line| line.trim_end_matches(" was passed")).collect::<Vec<_>>().join(" "), kinds, "{output:?}");
    // with the stack of the script's call, as an engine warning carries it
    let state = engine.state.borrow();
    let nested = state.output.entries.iter().rev().find(|entry| entry.kind == "warn").expect("the nested call's warning");
    let trace = nested.trace.as_deref().unwrap_or_default();
    assert!(trace.starts_with("Test:30 function Nested\nTest:32\n"), "{trace:?}");
}

/// A service method's string parameter reads any other value as a client reads it (measured as above): GetService and FindService
/// raise `'<text>' is not a valid Service name` for a number's, a boolean's, a table's or a datatype's text and answer nil for an
/// Instance, an EnumItem, an Enum, a function or a signal unless its text names a service (Enum.CoreGuiType.Chat is the Chat
/// service), after the warning; CollectionService's tag is its second argument (nil "Argument 3 missing or nil"); a method that
/// yields (Players:GetUserIdFromNameAsync, StarterGui:GetCore) reads it without the warning. ree raised `'' is not a valid Service
/// name` for every value that was no string and read no other value as its text.
#[test]
fn service_methods_read_a_string_parameter_as_a_client_does() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local function Try(Function, ...)
            local Ok, Result = pcall(Function, ...)
            if not Ok then
                return Result
            elseif typeof(Result) == "Instance" then
                return Result.ClassName
            end
            return tostring(Result)
        end
        print("get", Try(game.GetService, game, 5), Try(game.GetService, game, true), Try(game.GetService, game, workspace), Try(game.GetService, game, Enum.Material.Plastic),
            Try(game.GetService, game, Enum.CoreGuiType.Chat), Try(game.GetService, game, print), Try(game.GetService, game, nil), Try(game.GetService, game, "Lighting"))
        print("find", Try(game.FindService, game, 0.1), Try(game.FindService, game, workspace.Changed), Try(game.FindService, game, Enum.CoreGuiType.Chat), Try(game.FindService, game))
        local CollectionService = game:GetService("CollectionService")
        local Part = Instance.new("Part")
        CollectionService:AddTag(Part, 5)
        CollectionService:AddTag(Part, true)
        CollectionService:AddTag(Part)
        local Tags = Part:GetTags()
        table.sort(Tags)
        print("tags", table.concat(Tags, ","), CollectionService:HasTag(Part, 5), Try(CollectionService.HasTag, CollectionService, Part, nil), #CollectionService:GetTagged(0.5))
        local HttpService = game:GetService("HttpService")
        print("http", HttpService:JSONDecode(true), #HttpService:JSONDecode({ 1, 2 }), HttpService:UrlEncode(5), HttpService:UrlEncode(workspace))
        print("class", game:GetService("ReflectionService"):GetClass(workspace).Name, Try(game:GetService("ReflectionService").GetClass, game:GetService("ReflectionService"), nil))
        print("render", Try(game:GetService("RunService").BindToRenderStep, game:GetService("RunService"), nil, 1, function() end))
        print("yields", game:GetService("Players"):GetUserIdFromNameAsync(true), Try(game:GetService("Players").GetUserIdFromNameAsync, game:GetService("Players"), nil))
        "#,
    );
    assert!(
        output.contains(&"print: get '5' is not a valid Service name 'true' is not a valid Service name nil nil Chat nil Argument 1 missing or nil Lighting".to_string()),
        "{output:?}"
    );
    assert!(output.contains(&"print: find '0.10000000000000000555' is not a valid Service name nil Chat Argument 1 missing or nil".to_string()), "{output:?}");
    assert!(output.contains(&"print: tags 5,true true Argument 3 missing or nil 0".to_string()), "{output:?}");
    assert!(output.contains(&"print: http true 0 5 Instance".to_string()), "{output:?}");
    assert!(output.contains(&"print: class Instance Argument 2 missing or nil".to_string()), "{output:?}");
    assert!(output.contains(&"print: render Argument 1 missing or nil".to_string()), "{output:?}");
    assert!(output.contains(&"print: yields 1 Argument 1 missing or nil".to_string()), "{output:?}");
    let warned: Vec<&str> = output.iter().filter_map(|line| line.strip_prefix("warn: ")).collect();
    let expected = [
        "#1 expects a string, but boolean", "#1 expects a string, but Instance", "#1 expects a string, but EnumItem", "#1 expects a string, but EnumItem",
        "#1 expects a string, but function", "#1 expects a string, but RBXScriptSignal", "#1 expects a string, but EnumItem", "#2 expects a string, but boolean",
        "#1 expects a string, but Instance", "#1 expects a string, but boolean", "#1 expects a string, but table", "#1 expects a string, but Instance",
        "#1 expects a string, but Instance",
    ];
    assert_eq!(warned, expected.iter().map(|kind| format!("argument {kind} was passed")).collect::<Vec<_>>(), "{output:?}");
}

/// A number given to a base method's string parameter is read in the method's inline call (lua/raw.rs param_text), so it costs
/// the work clock what its text does (convert::cast_number_text calls nothing in the VM): the cast adds no call of its own.
#[test]
fn a_number_given_to_a_string_parameter_costs_what_its_text_does() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local Holder = Instance.new("Folder")
        for Index = 1, 5 do
            local Child = Instance.new("Folder")
            Child.Name = tostring(Index)
            Child.Parent = Holder
        end
        local function Cost(Value)
            Holder:FindFirstChild(Value)
            Holder:IsA(Value)
            Holder:HasTag(Value)
            local Start = os.clock()
            for _ = 1, 100 do
                Holder:FindFirstChild(Value)
                Holder:IsA(Value)
                Holder:HasTag(Value)
            end
            return os.clock() - Start
        end
        -- (each once first: the first loop of a frame pays what it is the first to touch)
        Cost("5")
        Cost(5)
        local Text, Number = Cost("5"), Cost(5)
        print("prices", math.abs(Text - Number) < 1e-9, Text > 1e-7, Holder:FindFirstChild(5) == Holder:FindFirstChild("5"))
        "#,
    );
    assert!(output.contains(&"print: prices true true true".to_string()), "{output:?}");
}

/// A table past ree's guard on a copy by its slots alone raises the guard's error before its slots are walked: the slots are
/// read at once (profiler::vm::table_slots) and every one counts against the guard, so the walk could only reach the same
/// error. The old code walked and priced every slot first (5,000,000 empty slots charged 1 ms, at 0.2 ns a slot).
#[test]
fn a_table_past_the_guard_raises_before_its_slots_are_walked() {
    assert_shared_prices(
        r#"
        local Big = table.create(5000000)
        Big[1] = 1
        local S = SharedTable.new()
        for _, Call in { function() return SharedTable.new(Big) end, function() S.k = Big end, function() return SharedTable.new({ Big }) end } do
            local Start = os.clock()
            local Ok, E = pcall(Call)
            local Took = (os.clock() - Start) * 1e3
            assert(not Ok and string.find(tostring(E), "more than 4194304 values", 1, true), tostring(E))
            assert(Took < 0.1, `charged {Took} ms before raising`)
        end
        "#,
    );
}

/// SharedTableRegistry pays for what it copies and runs nothing of a script's: a buffer used as a name is copied into the name
/// at the VM's price for a string the script built (cost.rs LUAU_STRING_BYTE, 0.169 ns a byte: 10 MB is 1.69 ms), since a Get
/// keeps the name for the session; SetSharedTable names a table it refuses by its raw length, as SetAttribute does, so a
/// `__len` neither runs nor changes the name (measured, a playing client, Studio 0.748, two runs: `{ a = 1 }` with a `__len` of
/// 3 is a Dictionary, `{}` with one that raises an Array, and no `__len` ran). The old code copied the buffer for a method
/// call's price and called `__len`.
#[test]
fn shared_table_registry_pays_for_a_buffer_name_and_runs_no_len() {
    assert_shared_prices(
        r#"
        local Registry = game:GetService("SharedTableRegistry")
        local Big = buffer.create(10000000)
        local Start = os.clock()
        Registry:GetSharedTable(Big)
        local Took = (os.clock() - Start) * 1e3
        Registry:SetSharedTable(Big, nil)
        assert(Took > 1.6 and Took < 3, `a 10 MB buffer name cost {Took} ms`)
        local Ran = false
        local _, Dictionary = pcall(Registry.SetSharedTable, Registry, "x", setmetatable({ a = 1 }, { __len = function() Ran = true return 3 end }))
        local _, Empty = pcall(Registry.SetSharedTable, Registry, "x", setmetatable({}, { __len = function() error("boom") end }))
        local _, Array = pcall(Registry.SetSharedTable, Registry, "x", setmetatable({ 1 }, { __len = function() Ran = true return 0 end }))
        assert(Dictionary == "Unable to cast Dictionary to SharedTable" and Empty == "Unable to cast Array to SharedTable", `{Dictionary} {Empty}`)
        assert(Array == "Unable to cast Array to SharedTable" and not Ran, `{Array} {Ran}`)
        "#,
    );
}

/// SharedTable.update and increment are one C function each in a script's frames, as a client's are (measured, Studio 0.748, a
/// playing client, two runs): `debug.info` gives `[C]`, line -1 and their names; in update's callback level 2 is update itself
/// and level 3 its caller, so through pcall `error(m, 2)` and `error(m, 3)` add no position and `error(m, 4)` the script's line;
/// a callback's error reaches an `xpcall` handler at the raise, the callback's line first in its traceback, a table error as it
/// was raised; increment's error has no frame of its own in a handler's traceback. The old code ran them as engine Luau over
/// a Rust function (the `ree.sharedtable` chunk): one Luau frame more (level 3 gave its position, a traceback listed it), and a
/// callback's error was raised again from it, so a handler's traceback had no line of the callback's.
#[test]
fn shared_table_update_and_increment_are_c_frames() {
    assert_script(
        r#"
        local S = SharedTable.new({ n = 1 })
        for _, Name in { "update", "increment" } do
            local Source, Line, Named = debug.info(SharedTable[Name], "sln")
            assert(Source == "[C]" and Line == -1 and Named == Name, `{Name}: {Source} {Line} {Named}`)
        end
        local Caller = debug.info(1, "s")
        SharedTable.update(S, "n", function(V)
            assert(debug.info(2, "f") == SharedTable.update and debug.info(2, "s") == "[C]", "level 2 is update")
            assert(debug.info(3, "s") == Caller, `level 3 is {debug.info(3, "s")}`)
            return V
        end)
        local Raised, CallbackLine, CallLine = {}, nil, nil
        for Level = 1, 4 do
            CallLine = debug.info(1, "l") + 1
            local _, E = pcall(SharedTable.update, S, "n", function() CallbackLine = debug.info(1, "l") error("boom", Level) end)
            table.insert(Raised, E)
        end
        assert(Raised[1] == `{Caller}:{CallbackLine}: boom` and Raised[2] == "boom" and Raised[3] == "boom", table.concat(Raised, " | "))
        assert(Raised[4] == `{Caller}:{CallLine}: boom`, `{Raised[4]} against line {CallLine}`)
        local function Handler(Message)
            return { Message = Message, Trace = debug.traceback() }
        end
        local Line
        local _, Got = xpcall(function()
            SharedTable.update(S, "n", function()
                Line = debug.info(1, "l") error({ tag = "table error" })
            end)
        end, Handler)
        assert(type(Got.Message) == "table" and Got.Message.tag == "table error", "the table raised")
        assert(string.find(Got.Trace, `{Caller}:{Line}`, 1, true) and not string.find(Got.Trace, "sharedtable", 1, true), Got.Trace)
        local _, Failed = xpcall(function()
            SharedTable.increment(S, "missing", 1)
        end, Handler)
        assert(Failed.Message == 'Unable to increment the value for key "missing" because it does not exist in the SharedTable.', Failed.Message)
        assert(not string.find(Failed.Trace, "sharedtable", 1, true), Failed.Trace)
        "#,
    );
}

/// A script's call to SharedTable.update or increment is one C call (cost.rs LUAU_CCALL), as to the library's other functions:
/// their Rust is called from their C functions, which the engine marks as its own, so it is not counted again, and update's
/// callback's instructions are the script's. The old code ran them as engine Luau over a Rust function, a call into the
/// engine's Luau each (LUAU_HOSTCALL, 9.23 ns more).
#[test]
fn shared_table_update_and_increment_are_one_c_call() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"
            local Module = Instance.new("ModuleScript")
            __setSource(Module, {source:?})
            _G.Calls = require(Module)
            "#,
            source = "local S = SharedTable.new({ n = 0 })
local function Same(V) return V end
return function(N) for _ = 1, N do SharedTable.update(S, \"n\", Same) SharedTable.increment(S, \"n\", 1) end return SharedTable.size(S) end
"
        ),
    );
    let before = vm_counts(&engine);
    let output = run(&lua, &engine, "print(_G.Calls(1000))");
    let after = vm_counts(&engine);
    assert!(output.contains(&"print: 1".to_string()), "{output:?}");
    let (ccalls, hostcalls) = (after.ccalls - before.ccalls, after.hostcalls - before.hostcalls);
    assert!((2000..2010).contains(&ccalls) && hostcalls < 5, "{ccalls} C calls and {hostcalls} engine Luau calls for 1,000 updates and increments");
}

/// An update whose callback raised lets go of its copy of the entries as the error leaves it, as one that stored its result
/// does, so the table's next write copies no node it would share with that copy: it costs what a write right after a stored
/// update costs. (update's copy is a local of its C function, dropped as the error unwinds through it; one held on would make
/// the write pay for its key's path, 0.78 us at 1,000 entries, as a write right after a clone does.)
#[test]
fn an_update_whose_callback_raised_lets_go_of_its_copy() {
    assert_shared_prices(
        r#"
        local T = SharedTable.new()
        for I = 1, 1000 do
            T["k" .. I] = I
        end
        local function Once(Body)
            local Start = os.clock()
            Body()
            return (os.clock() - Start) * 1e6
        end
        T.k2 = -1
        SharedTable.update(T, "k1", function(V) return V end)
        local Stored = Once(function() T.k2 = 0 end)
        assert(not pcall(SharedTable.update, T, "k1", function() error("raised") end))
        local Raised = Once(function() T.k2 = 1 end)
        local Cloned = SharedTable.clone(T)
        local Shared = Once(function() T.k2 = 2 end)
        Within("a write after a raised update beyond one after a stored update", Raised - Stored, -0.01, 0.01)
        Within("a write after a clone beyond one after a stored update", Shared - Stored, 0.1, 5)
        "#,
    );
}

/// A value a SharedTable pools (one the engine has no value of its own for) reads back as a new object each time, `==` to the
/// one written (Studio 0.748, a playing client: Ray, Region3, PhysicalProperties, NumberSequenceKeypoint, ColorSequenceKeypoint,
/// TweenInfo, Faces, Axes and Path2DControlPoint were never rawequal to the value written or to another read, and in a second
/// session, two runs, Region3int16, Vector2int16, Vector3int16 and SecurityCapabilities, which ree read back as the very object
/// written), through a read, a loop and a clone alike.
#[test]
fn pooled_shared_table_values_read_back_as_new_objects() {
    assert_script(
        r#"
        local Values = {
            Ray.new(Vector3.new(1, 2, 3), Vector3.new(0.1, 0.2, 0.3)),
            Region3.new(Vector3.new(0.1, 0.2, 0.3), Vector3.new(4, 5, 6)),
            PhysicalProperties.new(0.7, 0.3, 0.5, 1, 1),
            NumberSequenceKeypoint.new(0.5, 0.25, 0.1),
            ColorSequenceKeypoint.new(0.5, Color3.new(0.1, 0.2, 0.3)),
            TweenInfo.new(1.5, Enum.EasingStyle.Quad, Enum.EasingDirection.In, 2, true, 0.25),
            Faces.new(Enum.NormalId.Top, Enum.NormalId.Left),
            Axes.new(Enum.Axis.X, Enum.NormalId.Top),
            Path2DControlPoint.new(UDim2.new(1, 2, 3, 4), UDim2.new(0, 1, 0, 1), UDim2.new(0, 2, 0, 2)),
            Region3int16.new(Vector3int16.new(1, 2, 3), Vector3int16.new(4, 5, 6)),
            Vector2int16.new(1, 2),
            Vector3int16.new(1, 2, 3),
            SecurityCapabilities.new(Enum.SecurityCapability.Network, Enum.SecurityCapability.UI),
        }
        local S = SharedTable.new()
        for I, V in Values do
            S[I] = V
            local A, B = S[I], S[I]
            assert(typeof(A) == typeof(V) and A == V and B == V and A == B and tostring(A) == tostring(V), typeof(V))
            assert(not rawequal(A, V) and not rawequal(A, B), typeof(V) .. " is a new object on each read")
        end
        for I, V in S do
            assert(V == Values[I] and not rawequal(V, Values[I]), "a loop's value")
        end
        local C = SharedTable.clone(S)
        assert(C[6] == Values[6] and not rawequal(C[6], S[6]), "a clone's read")
        assert(S[2].CFrame == Values[2].CFrame and S[2].Size == Values[2].Size, "the Region3 kept to the bit")
        "#,
    );
}

/// A SharedTable keeps its values in the engine's own data, so filling one takes none of mlua's reference slots (lua/refs.rs):
/// strings, numbers, nested tables, the value types the engine has values for, and those it pools.
#[test]
fn a_shared_table_holds_no_reference_slots() {
    let (lua, engine) = start();
    let gauge = crate::lua::refs::Gauge::find(&lua).expect("the reference gauge");
    run(&lua, &engine, r#"game:GetService("SharedTableRegistry"):SetSharedTable("Many", SharedTable.new())"#);
    let before = gauge.count();
    run(
        &lua,
        &engine,
        r#"
        local S = game:GetService("SharedTableRegistry"):GetSharedTable("Many")
        for I = 1, 10000 do
            S["k" .. I] = "value " .. I
            S[I] = { I, "nested" }
            S["c" .. I] = CFrame.new(I, 0, 0)
            S["t" .. I] = TweenInfo.new(I)
        end
        assert(SharedTable.size(S) == 40000 and S.k7 == "value 7" and S[7][2] == "nested" and S.t7.Time == 7)
        "#,
    );
    let after = gauge.count();
    assert!(after - before < 50, "{before} slots held before, {after} after");
}

/// A SharedTable that holds itself lets its handle go once no script holds it, as a playing client does (Studio 0.748: the
/// handles of 300 such SharedTables were collected; their buffers' memory, as Roblox keeps it, stayed).
#[test]
fn a_shared_table_that_holds_itself_lets_its_handle_go() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        shared.Watched = setmetatable({}, { __mode = "k" })
        do
            local S = SharedTable.new()
            S.self = S
            S.other = SharedTable.new({ back = S })
            shared.Watched[S] = true
            shared.Watched[S.other] = true
        end
        "#,
    );
    lua.gc_collect().expect("collect");
    lua.gc_collect().expect("collect");
    run(
        &lua,
        &engine,
        r#"
        local Count = 0
        for _ in shared.Watched do
            Count += 1
        end
        assert(Count == 0, Count .. " handles still held")
        "#,
    );
}

/// A table converted to a SharedTable sits on the Lua stack above the call's own values, each table inside another two slots
/// higher, and one above slot 300 raises "Max lua stack size reached" without a position, after the cycle check (measured in a
/// playing client, Studio 0.748): `new` takes arrays and dictionaries nested 150 deep, one level less for every two more
/// arguments; a write and update's result 149.
#[test]
fn shared_table_conversions_stop_where_robloxs_stack_does() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local function Nest(Depth, Dict)
            local T = {}
            for _ = 2, Depth do
                T = if Dict then { x = T } else { T }
            end
            return T
        end
        local function Ring(Depth)
            local Outer = {}
            local Inner = Outer
            for _ = 2, Depth do
                local Next = {}
                Inner[1] = Next
                Inner = Next
            end
            Inner[1] = Outer
            return Outer
        end
        local function Fails(Message, F, ...)
            local Ok, E = pcall(F, ...)
            if Ok or E ~= Message then
                error(tostring(E), 2)
            end
        end
        local Full, Cyclic = "Max lua stack size reached", "Cannot convert a cyclic table to a SharedTable"
        SharedTable.new(Nest(150))
        SharedTable.new(Nest(150, true))
        SharedTable.new(Nest(150), 1)
        Fails(Full, SharedTable.new, Nest(151))
        Fails(Full, SharedTable.new, Nest(151, true))
        Fails(Full, SharedTable.new, Nest(150), 1, 2)
        SharedTable.new(Nest(149), 1, 2, 3)
        Fails(Full, SharedTable.new, Nest(149), 1, 2, 3, 4)
        SharedTable.new(Nest(148), 1, 2, 3, 4)
        local S = SharedTable.new()
        S.k = Nest(149)
        S[1] = Nest(149, true)
        Fails(Full, function() S.k = Nest(150) end)
        SharedTable.update(S, "k", function() return Nest(149) end)
        Fails(Full, SharedTable.update, S, "k", function() return Nest(150) end)
        Fails(Full, SharedTable.update, S, "k", function() return Nest(149) end, 1)
        -- a cycle closing at the limit raises the cyclic message, one closing past it the stack's
        Fails(Cyclic, SharedTable.new, Ring(150))
        Fails(Full, SharedTable.new, Ring(151))
        Fails(Cyclic, function() S.k = Ring(149) end)
        Fails(Full, function() S.k = Ring(150) end)
        -- SharedTables already made add no depth, and a table met twice is no cycle
        local Holder = Nest(149)
        local Inner = Holder
        while next(Inner) do
            Inner = Inner[1]
        end
        Inner[1] = SharedTable.new(Nest(150))
        SharedTable.new(Holder)
        local Leaf = Nest(140)
        SharedTable.new({ a = Leaf, b = Leaf })
        -- a table nested 300,000 deep fails as soon as it is too deep
        local Deep = {}
        for _ = 2, 300000 do
            Deep = { Deep }
        end
        Fails(Full, SharedTable.new, Deep)
        "#,
    );
}

/// A table is converted once for each place it appears, as Roblox converts it, so one small table can stand for a conversion
/// of any size; ree stops it past its guard on a copy (bindable.rs `COPY_VALUES`: the value written, and every entry walked, a
/// table each time it appears) with the guard's error, as every other copy of a script's table does: `new`, a write and
/// update's result. The big tables here hold a table of 100,000 threads 200 times first (threads are dropped, so their entries
/// make nothing) and then a table of 100,000 numbers 100,000 times, and a tree of `{ t, t }` 40 deep is 2^40 tables: each stops
/// before a number is stored. A conversion at the guard goes through.
#[test]
fn shared_table_conversions_stop_at_rees_guard_on_a_copy() {
    let (lua, engine) = start();
    let guard = crate::lua::bindable::COPY_VALUES;
    run(
        &lua,
        &engine,
        &r#"
        local Thread = coroutine.running()
        local Threads = table.create(100000, Thread)
        local Numbers = table.create(100000, 1)
        local Huge = table.create(100200)
        for Index = 1, 200 do
            Huge[Index] = Threads
        end
        for Index = 201, 100200 do
            Huge[Index] = Numbers
        end
        local Tree = {}
        for _ = 1, 40 do
            Tree = { Threads, Tree, Tree }
        end
        local Message = "ree: this copy would make more than GUARD values (a table is copied once for each place it appears)"
        local function Fails(F, ...)
            local Ok, E = pcall(F, ...)
            if Ok or E ~= Message then
                error(tostring(E), 2)
            end
        end
        Fails(SharedTable.new, Huge)
        Fails(SharedTable.new, Tree)
        local S = SharedTable.new()
        Fails(function() S.k = Huge end)
        Fails(SharedTable.update, S, "k", function() return Tree end)
        assert(S.k == nil)
        -- the value and GUARD - 1 entries: at the guard; one entry more is past it
        assert(SharedTable.size(SharedTable.new(table.create(GUARD - 1, Thread))) == 0)
        Fails(SharedTable.new, table.create(GUARD, Thread))
        "#
        .replace("GUARD", &guard.to_string()),
    );
}

/// A deep copy walks only to the SharedTables the tables it copies hold, while the work clock is charged for every entry of every
/// table, as Studio copies them (a playing client, Studio 0.748: 100 shallow clones of 10,000 entries 10.5 ms, 9.3 to 10.9 ns an
/// entry from 10 clones of 1,000 entries to 1,000 of 1,000; one SharedTable among 10,000 entries the same as none). 20,000 shallow
/// clones of a table of 1,000,000 numbers and one SharedTable are 2 * 10^10 entries: charged 200 s, walked in a moment.
#[test]
fn a_deep_copy_of_shallow_clones_walks_only_to_the_tables_they_hold() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local Big = {}
        for I = 1, 1000000 do
            Big["k" .. I] = I
        end
        Big.inner = { 1 }
        local T = SharedTable.new(Big)
        local U = SharedTable.new()
        for I = 1, 20000 do
            U[I] = SharedTable.clone(T)
        end
        shared.T, shared.U = T, U
        "#,
    );
    let started = std::time::Instant::now();
    let output = run(
        &lua,
        &engine,
        r#"
        local T, U = shared.T, shared.U
        local Before = os.clock()
        local D = SharedTable.clone(U, true)
        local Spent = os.clock() - Before
        print(string.format("%.6f", Spent))
        assert(D[1] ~= T and D[1] ~= D[2] and D[1].inner == D[20000].inner and D[1].inner ~= T.inner and D[1].k7 == 7 and D[1].inner[1] == 1)
        "#,
    );
    let elapsed = started.elapsed();
    let spent: f64 = output.iter().find_map(|line| line.strip_prefix("print: ")?.parse().ok()).expect("the price");
    let expected = 20_000.0 * 1_000_001.0 * crate::cost::SHARED_DEEP_STRING_ENTRY
        + 20_001.0 * crate::cost::SHARED_DEEP_INTEGER_ENTRY
        + 40_000.0 * crate::cost::SHARED_DEEP_TABLE;
    assert!((spent - expected).abs() < 1e-6 * expected, "charged {spent} s, Studio's per-entry price {expected} s");
    assert!(elapsed < std::time::Duration::from_secs(20), "the copy walked every entry: {elapsed:?}");
}

/// A chain of SharedTables a script builds one write at a time, each holding the next, is deep-copied and let go of whatever its
/// length (Studio deep-copies a chain 1,000 long): neither nests a call once a link, which would overflow the engine's stack.
#[test]
fn a_long_chain_of_shared_tables_is_copied_and_let_go_of() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Head = SharedTable.new()
        for _ = 1, 100000 do
            local Next = SharedTable.new()
            Next.next = Head
            Head = Next
        end
        local Copy = SharedTable.clone(Head, true)
        local Length, At = 0, Copy
        while At.next do
            Length += 1
            At = At.next
        end
        assert(Length == 100000, Length)
        "#,
    );
    lua.gc_collect().expect("collect");
    lua.gc_collect().expect("collect");
}

/// SharedTable's bulk calls cost the work clock what they cost a playing client, by the entries they handle (Studio 0.748, cost.rs
/// SHARED_NEW_STRING_ENTRY): new from 1,000 string-keyed entries 0.32 ms, and the same table written to a key as much, a loop over
/// them 36 us, a deep clone 10 us; a shallow clone the same at 1,000 entries as at 10; a conversion that fails at its last entry
/// what it walked.
#[test]
fn shared_table_bulk_calls_cost_what_studio_spent() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Body)
            local N = 20
            local start = os.clock()
            Body(N)
            local middle = os.clock()
            for _ = 1, N do end
            return ((middle - start) - (os.clock() - middle)) / N * 1e6
        end
        local Big, Small = {}, {}
        for I = 1, 1000 do
            Big["k" .. I] = I
        end
        for I = 1, 10 do
            Small["k" .. I] = I
        end
        local S, T = SharedTable.new(Big), SharedTable.new(Small)
        local New = Per(function(N) for _ = 1, N do SharedTable.new(Big) end end)
        local Loop = Per(function(N) for _ = 1, N do for _ in S do end end end)
        local Deep = Per(function(N) for _ = 1, N do SharedTable.clone(S, true) end end)
        local CloneBig = Per(function(N) for _ = 1, N do SharedTable.clone(S) end end)
        local CloneSmall = Per(function(N) for _ = 1, N do SharedTable.clone(T) end end)
        local Target = SharedTable.new()
        local Written = Per(function(N) for _ = 1, N do Target.k = Big end end)
        local Failing = {}
        for I = 1, 999 do
            Failing[I] = I
        end
        Failing[1000] = print
        local Failed = Per(function(N) for _ = 1, N do pcall(SharedTable.new, Failing) end end)
        assert(New > 280 and New < 380, "new from 1,000 entries (us): " .. New)
        assert(Loop > 30 and Loop < 45, "a loop over 1,000 entries (us): " .. Loop)
        assert(Deep > 8 and Deep < 14, "a deep clone of 1,000 entries (us): " .. Deep)
        -- Studio copies every entry of every table a deep copy reaches, shallow clones included: 100 clones of 1,000 entries
        -- 925 and 1,573 us (two sessions), so 100,000 entries' worth, however little ree's copy walks
        local Clones = SharedTable.new()
        for I = 1, 100 do
            Clones[I] = SharedTable.clone(S)
        end
        local DeepClones = Per(function(N) for _ = 1, N do SharedTable.clone(Clones, true) end end)
        assert(DeepClones > 850 and DeepClones < 1300, "a deep clone of 100 shallow clones of 1,000 entries (us): " .. DeepClones)
        assert(math.abs(CloneBig - CloneSmall) < 1, "a shallow clone at 1,000 and 10 entries (us): " .. CloneBig .. " " .. CloneSmall)
        assert(Written > 280 and Written < 380, "1,000 entries written to a key (us): " .. Written)
        assert(Failed > 150 and Failed < 250, "new failing at its 1,000th entry (us): " .. Failed)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// A SharedTable write costs the work clock what a playing client spent on it (Studio 0.748, cost.rs SHARED_OVERWRITE_STRING): an
/// overwrite 0.07-0.09 us with a string key and 0.10-0.11 with an integer key, a key added 0.31-0.45 and 0.19-0.23, one taken out
/// 0.23-0.39 and 0.15-0.23; the first write after a shallow clone of 1,000 string-keyed entries 0.68-0.81, as it copies the path
/// the clone shares, while a 10-entry clone's second write, whose path is already its own, costs an overwrite again (0.08).
#[test]
fn shared_table_writes_cost_what_studio_spent() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Body)
            local N = 200
            local start = os.clock()
            Body(N)
            local middle = os.clock()
            for _ = 1, N do end
            return ((middle - start) - (os.clock() - middle)) / N * 1e6
        end
        local Strings, Integers, Small = {}, {}, {}
        for I = 1, 1000 do
            Strings["k" .. I] = I
            Integers[I] = I
        end
        for I = 1, 10 do
            Small["k" .. I] = I
        end
        local Fresh = {}
        for I = 1, 200 do
            Fresh[I] = "n" .. I
        end
        local S, T = SharedTable.new(Strings), SharedTable.new(Integers)
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        Within("a string key overwritten", Per(function(N) for _ = 1, N do S.k1 = 2 end end), 0.06, 0.11)
        Within("a string key added", Per(function(N) for I = 1, N do S[Fresh[I]] = 1 end end), 0.25, 0.45)
        Within("a string key taken out", Per(function(N) for I = 1, N do S[Fresh[I]] = nil end end), 0.23, 0.39)
        Within("an integer key overwritten", Per(function(N) for _ = 1, N do T[1] = 2 end end), 0.09, 0.14)
        Within("an integer key added", Per(function(N) for I = 1, N do T[1000 + I] = 1 end end), 0.15, 0.25)
        Within("an integer key taken out", Per(function(N) for I = 1, N do T[1000 + I] = nil end end), 0.15, 0.25)
        local Clones, Smalls = {}, {}
        local Base = SharedTable.new(Small)
        for I = 1, 200 do
            Clones[I] = SharedTable.clone(S)
            Smalls[I] = SharedTable.clone(Base)
        end
        Within("the first write after a clone of 1,000", Per(function(N) for I = 1, N do Clones[I].k1 = 3 end end), 0.6, 0.9)
        Per(function(N) for I = 1, N do Smalls[I].k1 = 3 end end)
        Within("a 10-entry clone's second write", Per(function(N) for I = 1, N do Smalls[I].k2 = 3 end end), 0.06, 0.11)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// The parallel bridge's machinery costs what Studio spent (an empty test place's client, an actor_cost probe, 2026-10-23): a BindableFunction:Invoke
/// into an Actor's OnInvoke 1,025 ns, SharedTable.increment 370-429; ree counted 81 and 32 (a table read, 29 ns in Studio, is close to ree's 3).
#[test]
fn a_call_into_an_actor_and_a_shared_increment_cost_what_studio_spent() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Per(Body)
            local N = 500
            local start = os.clock()
            Body(N)
            local middle = os.clock()
            for _ = 1, N do end
            return ((middle - start) - (os.clock() - middle)) / N * 1e9
        end
        local Actor = Instance.new("Actor")
        local Call = Instance.new("BindableFunction")
        Call.Parent = Actor
        Call.OnInvoke = function(A) return A end
        local Plain = Instance.new("BindableFunction")
        Plain.OnInvoke = function(A) return A end
        local Shared = SharedTable.new({ N = 0 })
        local Invoke, Increment, Same = Per(function(N) for _ = 1, N do Call:Invoke(1, 2) end end), Per(function(N) for _ = 1, N do SharedTable.increment(Shared, "N", 1) end end), Per(function(N) for _ = 1, N do Plain:Invoke(1, 2) end end)
        assert(Invoke > 850 and Invoke < 1300, "Invoke into an Actor: " .. Invoke)
        assert(Increment > 300 and Increment < 520, "SharedTable.increment: " .. Increment)
        assert(Same < 300, "a call outside an Actor keeps its price: " .. Same)
        print("ok")
        "#,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// The Luau a SharedTable price test measures with: `Per(N, Body)`, what `Body(N)` cost the work clock a call (us), an empty loop
/// taken off, and `Within(Label, Value, Low, High)`.
const SHARED_PER: &str = r#"
local function Per(N, Body)
    local Start = os.clock()
    Body(N)
    local Middle = os.clock()
    for _ = 1, N do end
    return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
end
local function Within(Label, Value, Low, High)
    assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
end
"#;

/// Runs a SharedTable price test (`SHARED_PER` first) with the work clock running, and checks it printed "ok".
fn assert_shared_prices(source: &str) {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, &format!("{SHARED_PER}\n{source}\nprint(\"ok\")"));
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// A SharedTable call costs the bytes of its key, hashed once a call, and of the strings and buffers it copies in or makes, as
/// a playing client spends them (Studio 0.748, three sessions; cost.rs SHARED_KEY_BYTE and after): with a 1 MB key a read
/// 926-1,060 us, an overwrite 1,118-1,243, increment 1,288-1,455, update 1,633-1,927; a 1 MB string written 322-781 and read
/// 294-357, a 1 MB buffer read 279-371; new of a 1 MB key 1,074 a key, of a 1 MB string 288-292; a loop's step over a 1 MB key
/// 142-146. ree charged 0.02 to 0.4 us for each, whatever the size. A string written or read steps at 65,488 bytes, a buffer
/// written at 65,504 (a bisection, two sessions: a string of 65,440 bytes written in 1.7 to 2.0 us and read in 11.5, one of 65,488
/// written in 88 to 138 and read in 71 to 104; a buffer of 65,488 written in 1.4, of 65,504 in 30 to 32); ree stepped both at
/// 65,504 and priced reads by the VM's price a byte at any size.
#[test]
fn shared_table_key_and_value_bytes_cost_what_studio_spent() {
    assert_shared_prices(
        r#"
        local L = 1048576
        local K = string.rep("a", L - 1) .. "k"
        local V = string.rep("v", L - 1) .. "w"
        local S = SharedTable.new({ x = 1 })
        S[K] = 1
        S.v = V
        S.b = buffer.create(L)
        local function Same(X) return X end
        Within("a 1 MB key read", Per(4, function(N) for _ = 1, N do local _ = S[K] end end), 600, 1400)
        Within("a 1 MB key overwritten", Per(4, function(N) for _ = 1, N do S[K] = 2 end end), 700, 1600)
        Within("a 1 MB key incremented", Per(4, function(N) for _ = 1, N do SharedTable.increment(S, K, 1) end end), 700, 1800)
        Within("a 1 MB key updated", Per(4, function(N) for _ = 1, N do SharedTable.update(S, K, Same) end end), 700, 2400)
        Within("a 1 MB string written", Per(4, function(N) for _ = 1, N do S.v = V end end), 300, 1000)
        Within("a 1 MB string read", Per(4, function(N) for _ = 1, N do local _ = S.v end end), 150, 450)
        Within("a 1 MB buffer read", Per(4, function(N) for _ = 1, N do local _ = S.b end end), 250, 700)
        local Keyed, Valued = { [K] = 1 }, { V }
        Within("new with a 1 MB key", Per(4, function(N) for _ = 1, N do SharedTable.new(Keyed) end end), 650, 1500)
        Within("new with a 1 MB string", Per(4, function(N) for _ = 1, N do SharedTable.new(Valued) end end), 150, 500)
        local Loop = SharedTable.new(Keyed)
        Within("a loop over one 1 MB key", Per(4, function(N) for _ = 1, N do for _ in Loop do end end end), 70, 250)
        -- a string written steps where a client's does: 65,480 bytes 1.39 us, 65,488 88 to 138, 65,504 44.8; a buffer 16 bytes
        -- later: 65,488 bytes 1.4 us, 65,504 30 to 32
        local Under, At, Over = string.rep("u", 65480), string.rep("t", 65488), string.rep("o", 65504)
        Within("a string of 65,480 bytes written", Per(20, function(N) for _ = 1, N do S.v = Under end end), 0, 2)
        Within("a string of 65,488 bytes written", Per(20, function(N) for _ = 1, N do S.v = At end end), 25, 150)
        Within("a string of 65,504 bytes written", Per(20, function(N) for _ = 1, N do S.v = Over end end), 25, 70)
        local Small, Large = buffer.create(65488), buffer.create(65504)
        Within("a buffer of 65,488 bytes written", Per(20, function(N) for _ = 1, N do S.w = Small end end), 0, 3)
        Within("a buffer of 65,504 bytes written", Per(20, function(N) for _ = 1, N do S.w = Large end end), 25, 70)
        -- and a string read steps where a string written does: 65,440 bytes 11.5 us, 65,488 71 to 104
        S.r, S.t = string.rep("r", 65440), At
        Within("a string of 65,440 bytes read", Per(20, function(N) for _ = 1, N do local _ = S.r end end), 5, 20)
        Within("a string of 65,488 bytes read", Per(20, function(N) for _ = 1, N do local _ = S.t end end), 25, 110)
        "#,
    );
}

/// A key added to a slot another key holds moves both down, which hashes the key held again, once (a playing client, Studio
/// 0.748: "s58" added and taken out beside a 1 MB key whose first slot it shares 793 us, beside one it does not 0.16; ree charged
/// 0.62 us either way).
#[test]
fn a_slot_split_hashes_the_held_key_again() {
    assert_shared_prices(
        r#"
        -- (the first 1 MB key's hash and "s58"'s share their lowest five bits, the second's do not)
        local S, T = SharedTable.new(), SharedTable.new()
        S[string.rep("a", 1048575) .. "k"] = 1
        T[string.rep("a", 1048575) .. "m"] = 1
        local function AddRemove(Table) return Per(1, function() Table.s58 = 1 Table.s58 = nil end) end
        Within("s58 added and taken out beside a 1 MB key in its slot", AddRemove(S), 500, 1100)
        Within("s58 added and taken out beside a 1 MB key elsewhere", AddRemove(T), 0, 2)
        "#,
    );
}

/// An update whose callback is called again, as the table changed while it ran, looks its key up again each time, and pays for
/// hashing it again (not measured in Studio: every lookup a client makes hashes its key, measured for reads, 0.81 ns a byte). ree
/// charged the key once a call however often the callback ran: 100 calls with a 1 MB key 0.9 ms.
#[test]
fn an_update_called_again_pays_for_its_key_again() {
    assert_shared_prices(
        r#"
        local K = string.rep("k", 1048576)
        local S = SharedTable.new()
        S[K] = 0
        local Calls = 0
        local Spent = Per(1, function()
            SharedTable.update(S, K, function(V)
                Calls += 1
                if Calls < 100 then
                    S[1] = Calls
                end
                return V
            end)
        end)
        assert(Calls == 100, Calls)
        -- (at least 100 hashes of 1 MB, 84.9 ms)
        Within("update of a 1 MB key whose callback changed the table on its first 99 calls", Spent, 84000, 200000)
        "#,
    );
}

/// Updates nested in their callbacks go as deep as a client's, one C call a level, and stop with the VM's own message (a playing
/// client, Studio 0.748: depth 199 in a script, then "C stack overflow"). The depth is the VM's limit of 200 C calls less those
/// the thread already holds, which depends on how the engine resumed it (195 to 199 here). ree called each callback two C calls
/// deep: depth 100, "runtime error: C stack overflow".
#[test]
fn nested_updates_stop_where_robloxs_c_stack_does() {
    assert_script(
        r#"
        local S = SharedTable.new()
        local Depth = 0
        local function Deeper()
            Depth += 1
            if Depth < 1000 then
                SharedTable.update(S, "k" .. Depth, Deeper)
            end
            return Depth
        end
        local Ok, E = pcall(Deeper)
        assert(not Ok and E == "C stack overflow" and Depth >= 190 and Depth <= 199, `{Ok} {E} {Depth}`)
        -- an error the callback raises still comes back as it was raised
        local Thrown = {}
        local Ok2, E2 = pcall(SharedTable.update, S, "x", function() error(Thrown) end)
        assert(not Ok2 and rawequal(E2, Thrown))
        "#,
    );
}

/// SharedTable.update holds a copy of the table while it runs, as Roblox's does, so its write copies the key's path, as a first
/// write after a clone does (a playing client, Studio 0.748: update 0.58 / 0.78 / 1.2 us at 10 / 1,000 / 100,000 entries, an
/// overwrite 0.065 / 0.078 / 0.088, the first write after a clone 0.48 / 0.82 / 1.0; ree charged 0.088 at every size).
#[test]
fn shared_table_update_copies_its_path_as_in_roblox() {
    assert_shared_prices(
        r#"
        local function Same(X) return X end
        for _, Count in { 10, 1000, 100000 } do
            local T = {}
            for I = 1, Count do
                T["k" .. I] = I
            end
            local S = SharedTable.new(T)
            local Update = Per(200, function(N) for _ = 1, N do SharedTable.update(S, "k1", Same) end end)
            local Overwrite = Per(200, function(N) for _ = 1, N do S.k1 = 1 end end)
            assert(Update > Overwrite + 0.2, `update {Update} and an overwrite {Overwrite} (us) at {Count} entries`)
            if Count == 1000 then
                Within("update of a 1,000-entry table", Update, 0.4, 1.0)
            end
        end
        "#,
    );
}

/// The Luau of 2^n keys whose 64-bit FNV-1a hashes are all equal: `Key(I)` takes `A[J]` or `B[J]` by bit `J - 1` of `I` (real
/// collisions, pairs of 16-byte blocks of equal hash found offline, chained: 3 pairs give 8 keys of 48 bytes, 10 give 1,024 of 160).
const COLLIDING_KEYS: &str = r#"
local A = { "amacobmcimlpggfd", "gajldeibogfilcdn", "oaomcbcfamgpdfnp", "khjepnbhcdemjbkn", "nfladgmnibaleboe", "bbjhbacpjdmklnif", "leihpdkifpiicohb", "aipaghcphiiglmmm", "ainkaeaampcncieb", "bpldhobacbicbjla" }
local B = { "jadnfnainmlpjeik", "picgfcflgnjiaddi", "okpppjoaaachdced", "ggjepnplckhjkffl", "nhcpmfbfeilolakc", "fiimifegkmekkjco", "lmabnookgcnneemf", "ajahofibjdciefbh", "ekioneijhelamfcp", "mpkhfagpfpinikcj" }
local function Key(I, Pairs)
    local Parts = {}
    for J = 1, Pairs do
        Parts[J] = if bit32.btest(I, bit32.lshift(1, J - 1)) then B[J] else A[J]
    end
    return table.concat(Parts)
end
"#;

/// Keys whose 64-bit hashes are equal walk as a playing client walks them (Studio 0.748, eight such keys, each sequence printed
/// by the client): the newest written first; a removal keeps the others' order; an overwrite or increment in the table's own
/// list stays in place, while a write or removal after a shallow clone moves the key written first (the clone keeps its order);
/// update moves its key first; a deep clone keeps the order; new(t) walks in the reverse of t's pairs order. A loop that ran to its
/// end still shares the list until the collector takes it, so a write right after one moves the key first as after a clone
/// (order.luau, each write right after a walk: "overwrite 5: 5,3,7,6,4,2,1,0", "increment 0, update 1: 1,0,5,3,7,6,4,2"; ree's
/// ended loop let go of the list and printed 3,7,6,5,4,2,1,0 and 1,3,7,6,5,4,2,0). ree walked them by their bytes.
#[test]
fn colliding_string_keys_walk_as_roblox_orders_them() {
    assert_script(&format!(
        "{COLLIDING_KEYS}\n{}",
        r#"
        local Keys, IndexOf = {}, {}
        for I = 0, 7 do
            Keys[I] = Key(I, 3)
            IndexOf[Keys[I]] = I
        end
        local function Walk(S)
            local Seen = {}
            for K in S do table.insert(Seen, IndexOf[K] or K) end
            return table.concat(Seen, ",")
        end
        local function Fill()
            local S = SharedTable.new()
            for I = 0, 7 do S[Keys[I]] = I end
            return S
        end
        local S1 = Fill()
        local S2 = Fill() S2[Keys[3]] = nil local W2 = Walk(S2) S2[Keys[3]] = 3
        local S3 = Fill() S3[Keys[5]] = 55
        local S4 = Fill() local C4 = SharedTable.clone(S4) S4[Keys[5]] = 55
        local S5 = Fill() SharedTable.update(S5, Keys[2], function(V) return V end)
        local S6 = Fill() SharedTable.increment(S6, Keys[1], 1)
        local S7 = Fill() local C7 = SharedTable.clone(S7) S7[Keys[6]] = nil
        local S8 = Fill() local C8 = SharedTable.clone(S8) SharedTable.increment(S8, Keys[1], 1)
        local Walks = {
            Walk(S1), W2, Walk(S2), Walk(S3), Walk(S4), Walk(C4), Walk(S5), Walk(S6), Walk(S7), Walk(C7), Walk(S8), Walk(C8),
            Walk(SharedTable.clone(S1, true)), Walk(SharedTable.cloneAndFreeze(S1, true)),
        }
        assert(table.concat(Walks, " | ") == "7,6,5,4,3,2,1,0 | 7,6,5,4,2,1,0 | 3,7,6,5,4,2,1,0 | 7,6,5,4,3,2,1,0 | 5,7,6,4,3,2,1,0 | 7,6,5,4,3,2,1,0 | 2,7,6,5,4,3,1,0 | 7,6,5,4,3,2,1,0 | 7,5,4,3,2,1,0 | 7,6,5,4,3,2,1,0 | 1,7,6,5,4,3,2,0 | 7,6,5,4,3,2,1,0 | 7,6,5,4,3,2,1,0 | 7,6,5,4,3,2,1,0", table.concat(Walks, " | "))
        assert(SharedTable.size(S1) == 8 and S1[Keys[0]] == 0 and S1[Keys[7]] == 7 and S3[Keys[5]] == 55 and C4[Keys[5]] == 5 and S7[Keys[6]] == nil)
        -- new(t) adds t's keys in its pairs order, so they walk the other way
        local Plain = {}
        for _, I in { 5, 2, 7, 0, 3, 6, 1, 4 } do Plain[Keys[I]] = I end
        local Reversed = {}
        for K in pairs(Plain) do table.insert(Reversed, 1, IndexOf[K]) end
        assert(Walk(SharedTable.new(Plain)) == table.concat(Reversed, ","))
        -- among other keys: "alpha" shares the list's first slot, "beta" does not
        local M = SharedTable.new()
        M.alpha = 1
        for I = 0, 3 do M[Keys[I]] = I end
        M.beta = 2
        assert(Walk(M) == "beta,alpha,3,2,1,0", Walk(M))
        -- each write right after a loop that ran to its end, as order.luau did
        local O = Fill()
        local Seq = { Walk(O) }
        O[Keys[3]] = nil
        table.insert(Seq, Walk(O))
        O[Keys[3]] = 3
        table.insert(Seq, Walk(O))
        O[Keys[5]] = 55
        table.insert(Seq, Walk(O))
        SharedTable.increment(O, Keys[0], 1)
        SharedTable.update(O, Keys[1], function(V) return V end)
        table.insert(Seq, Walk(O))
        local Want = "7,6,5,4,3,2,1,0 | 7,6,5,4,2,1,0 | 3,7,6,5,4,2,1,0 | 5,3,7,6,4,2,1,0 | 1,0,5,3,7,6,4,2"
        assert(table.concat(Seq, " | ") == Want, table.concat(Seq, " | "))
        assert(O[Keys[5]] == 55 and O[Keys[0]] == 1 and O[Keys[1]] == 1)
        "#
    ));
}

/// Keys whose 64-bit hashes are equal cost what a playing client spends on them, by the entries of their list (Studio 0.748,
/// 1,024 keys of 208 bytes: the oldest read 2.6-13.7 us, a key added 39.5, the newest taken out 30.3, the first write after a
/// shallow clone 88-94; update on its own list about 150, 290-306 at 2,048 keys): a list is searched from the front, rebuilt when
/// a key joins or leaves it and copied when a clone shares it. ree charged 0.03 / 0.32 / 0.32 / 3.5 / 0.09 us.
#[test]
fn colliding_keys_cost_per_entry_as_in_roblox() {
    assert_shared_prices(&format!(
        "{COLLIDING_KEYS}\n{}",
        r#"
        local Keys = {}
        for I = 0, 1023 do Keys[I] = Key(I, 10) end
        local T = SharedTable.new()
        for I = 0, 1022 do T[Keys[I]] = I end
        local function Same(X) return X end
        Within("the oldest of 1,023 keys read", Per(20, function(N) for _ = 1, N do local _ = T[Keys[0]] end end), 2, 15)
        Within("a key added to 1,023", Per(1, function() T[Keys[1023]] = 1 end), 20, 60)
        Within("the newest of 1,024 taken out", Per(1, function() T[Keys[1023]] = nil end), 15, 60)
        local C = SharedTable.clone(T)
        Within("the first write after a clone", Per(1, function() C[Keys[1022]] = 0 end), 50, 130)
        Within("update on its own list", Per(1, function() SharedTable.update(T, Keys[0], Same) end), 80, 220)
        assert(T[Keys[0]] == 0 and C[Keys[1022]] == 0 and T[Keys[1022]] == 1022 and SharedTable.size(T) == 1023)
        "#
    ));
}

/// A loop walks a snapshot of the trie an entry a step, so one stopped at its first entry costs the same at any size, as a
/// playing client's does (Studio 0.748: 0.24 to 0.29 us from 10 entries to 100,000, 0.38 to 0.73 from 3 to 200,000); ree took
/// its snapshot entry by entry and charged every entry at the start (2.9 ms at 100,000).
#[test]
fn a_shared_table_loop_costs_its_start_and_each_step() {
    assert_shared_prices(
        r#"
        local Big = {}
        for I = 1, 100000 do
            Big["k" .. I] = I
        end
        local S, T = SharedTable.new(Big), SharedTable.new({ a = 1, b = 2, c = 3 })
        local function Stopped(Table) return Per(20, function(N) for _ = 1, N do for _ in Table do break end end end) end
        Stopped(S)
        Stopped(T)
        local First, Small = Stopped(S), Stopped(T)
        Within("a loop over 100,000 entries stopped at its first", First, 0.3, 1.0)
        assert(math.abs(First - Small) < 0.1, `{First} {Small}`)
        "#,
    );
}

/// SharedTable.clear costs the entries it frees when the table alone holds them, as a playing client's does (Studio 0.748, two
/// runs of each setup: 262 to 478 us at 10,000 entries, by how recently the tables were made, 37 to 41 at 1,000; cost.rs
/// SHARED_CLEARED_ENTRY), and a shallow clone's only the call (0.02 to 0.20 us); ree charged the call alone for both, then 65 ns
/// an entry from an earlier session (553 to 596 us at 10,000).
#[test]
fn clearing_a_shared_table_costs_the_entries_it_frees() {
    assert_shared_prices(
        r#"
        local T = {}
        for I = 1, 10000 do
            T["k" .. I] = I
        end
        local S = SharedTable.new(T)
        local Owned, Clones = {}, {}
        for I = 1, 5 do
            Owned[I] = SharedTable.new(T)
            Clones[I] = SharedTable.clone(S)
        end
        local At = 0
        local ClearOwned = Per(5, function(N) for _ = 1, N do At += 1 SharedTable.clear(Owned[At]) end end)
        At = 0
        local ClearShared = Per(5, function(N) for _ = 1, N do At += 1 SharedTable.clear(Clones[At]) end end)
        Within("clear of 10,000 entries the table alone holds", ClearOwned, 250, 500)
        Within("clear of a shallow clone", ClearShared, 0, 1)
        assert(SharedTable.size(Owned[5]) == 0 and SharedTable.size(Clones[5]) == 0 and SharedTable.size(S) == 10000)
        "#,
    );
}

/// Each time a SharedTable reaches a script it is a new userdata, as in a playing client (Studio 0.748: `S.t` read twice, a
/// loop's value against a read, a value written and read back, the registry's Get called twice are `==` but not rawequal, their
/// tostring differs, and a table keyed by one is not found by another; two SharedTables, a clone and its original among them,
/// are not `==`). ree kept one handle a table.
#[test]
fn each_shared_table_a_script_gets_is_a_new_handle() {
    assert_script(
        r#"
        local S = SharedTable.new({ t = {} })
        local A, B = S.t, S.t
        assert(A == B and not rawequal(A, B) and tostring(A) ~= tostring(B))
        local Keyed = {}
        Keyed[A] = true
        assert(Keyed[B] == nil and Keyed[A] == true)
        local FromLoop
        for _, V in S do
            FromLoop = V
        end
        assert(FromLoop == A and not rawequal(FromLoop, A))
        local N = SharedTable.new()
        S.n = N
        assert(S.n == N and not rawequal(S.n, N))
        local C = SharedTable.clone(S)
        assert(S.t ~= N and (S.t :: any) ~= {} and C ~= S and C.t == S.t and not rawequal(C.t, S.t))
        local Registry = game:GetService("SharedTableRegistry")
        local R1, R2 = Registry:GetSharedTable("Handles"), Registry:GetSharedTable("Handles")
        assert(R1 == R2 and not rawequal(R1, R2) and R1 ~= S)
        "#,
    );
}

/// SharedTable.update calls its callback again, with the value then current, whenever a write, increment, clear or update of the
/// table completed while it ran, and stores the result of a call during which none did (a playing client, Studio 0.748, each
/// callback changing the table on its first call only: a write of the same key, of another key, of the same value again, an
/// increment, a nested update and a clear give two calls, the second seeing the current value; a read and a clone of it, a
/// SharedTable made, and a write to another SharedTable one). ree called it once and stored its result over what it wrote. One
/// that changes nothing counts too (the same client, two runs of two probes: nil or a thread written to a key the table does not
/// hold, a clear of an empty table and a nested update storing nil there each gave two calls, nil written on the first two calls
/// three), while a call that raised does not (a function written, a failed increment, a nested update whose callback raised or
/// returned a function: one call); ree counted none of the first.
#[test]
fn shared_table_update_calls_again_when_the_table_changed() {
    assert_script(
        r#"
        local S = SharedTable.new()
        local Log = {}
        local function Counted(Key, First)
            local Calls, Seen = 0, {}
            SharedTable.update(S, Key, function(V)
                Calls += 1
                table.insert(Seen, tostring(V))
                if Calls == 1 then
                    First()
                end
                if Calls > 50 then
                    error("stop")
                end
                return "result" .. Calls
            end)
            table.insert(Log, `{Key} {Calls} {table.concat(Seen, ",")} {tostring(S[Key])}`)
        end
        S.a = "a0"
        Counted("a", function() S.a = "inner" end)
        S.b = "b0"
        Counted("b", function() S.other = "x" end)
        S.c = "c0"
        Counted("c", function() local _ = S.c local _ = SharedTable.clone(S) end)
        S.d = 1
        Counted("d", function() SharedTable.increment(S, "d", 1) end)
        S.e = "e0"
        Counted("e", function() SharedTable.update(S, "e", function() return "nested" end) end)
        S.f = "f0"
        Counted("f", function() S.f = "f0" end)
        S.g = "g0"
        Counted("g", function() SharedTable.clear(S) end)
        S.h = "h0"
        Counted("h", function() local T = SharedTable.new() T.x = 1 end)
        S.t = SharedTable.new({ x = 1 })
        S.i = "i0"
        Counted("i", function() S.t.x = 2 end)
        local Want = "a 2 a0,inner result2|b 2 b0,b0 result2|c 1 c0 result1|d 2 1,2 result2|e 2 e0,nested result2|f 2 f0,f0 result2|g 2 g0,nil result2|h 1 h0 result1|i 1 i0 result1"
        assert(table.concat(Log, "|") == Want, table.concat(Log, "|"))
        assert(S.other == nil and S.t.x == 2)
        -- a change that changes nothing, made on the first call (or the first two), counts; a call that raised does not (capped
        -- at 51 calls)
        local function Calls(Table, Change, Times)
            local Count = 0
            pcall(SharedTable.update, Table, "k", function()
                Count += 1
                if Count > 50 then
                    error("stop")
                end
                if Count <= (Times or 1) then
                    Change()
                end
                return Count
            end)
            return Count
        end
        local N, E = SharedTable.new({ x = 1 }), SharedTable.new()
        local NoOps = {
            Calls(N, function() N.missing = nil end),
            Calls(N, function() N.missing = coroutine.create(print) end),
            Calls(E, function() SharedTable.clear(E) end),
            Calls(N, function() SharedTable.update(N, "missing", function() return nil end) end),
            Calls(N, function() N.missing = nil end, 2),
        }
        assert(table.concat(NoOps, ",") == "2,2,2,2,3", table.concat(NoOps, ","))
        assert(N.k == 3 and E.k == 2 and N.missing == nil)
        local Raised = {
            Calls(N, function() pcall(function() N.f = print end) end),
            Calls(N, function() pcall(SharedTable.increment, N, "missing", 1) end),
            Calls(N, function() pcall(SharedTable.update, N, "y", function() error("e") end) end),
            Calls(N, function() pcall(SharedTable.update, N, "y", function() return print end) end),
        }
        assert(table.concat(Raised, ",") == "1,1,1,1", table.concat(Raised, ","))
        -- the result is converted right after each call, so one that cannot be stored raises at once, before the change check, and
        -- the change the callback made stays (the same client: "Functions can not be stored in a SharedTable" after one call; ree
        -- called it again and stored the second result)
        local R, Count = SharedTable.new({ a = 0 }), 0
        local Ok, Message = pcall(SharedTable.update, R, "a", function()
            Count += 1
            if Count == 1 then
                R.b = 1
                return print
            end
            return 5
        end)
        assert(not Ok and Message == "Functions can not be stored in a SharedTable" and Count == 1 and R.a == 0 and R.b == 1, `{Message} {Count} {R.a}`)
        "#,
    );
}

/// A failed increment of a key the table does not hold leaves the key with no value, as a playing client's does (Studio 0.748,
/// two runs of each probe, after the key, delta and frozen checks pass): size counts it, a loop gives it as (key, nil) in its
/// key's place, clones keep it, a read and update's callback get nil, another increment still says it does not exist, and nil
/// written, a dropped value, update's nil result and clear take it out; it is not a change for update. Only in a table no engine
/// value was made of, though: a client marks a SharedTable for good once it stores it in a SharedTable (a frozen one too), the
/// registry, a bindable's copy, SetAttribute, print or warn, deep-copies it or updates it, and a marked one gets no such key (the
/// same client, two runs of three probes, each case below; inside an update's callback too). ree left nothing, then left the key
/// in every table.
#[test]
fn a_failed_increment_leaves_its_key_with_no_value() {
    assert_script(
        r#"
        local function Dump(S)
            local Out = {}
            for K, V in S do
                table.insert(Out, `{K}={V}`)
            end
            return table.concat(Out, " ")
        end
        local S = SharedTable.new({ a = 1, [1] = 10, [2] = 20 })
        local Ok, E = pcall(SharedTable.increment, S, "e", 1)
        assert(not Ok and E == 'Unable to increment the value for key "e" because it does not exist in the SharedTable.', E)
        pcall(SharedTable.increment, S, 7, 1)
        pcall(SharedTable.increment, S, 0, -1)
        assert(SharedTable.size(S) == 6 and S.e == nil and S[7] == nil and S[0] == nil, SharedTable.size(S))
        assert(Dump(S) == "0=nil 1=10 2=20 7=nil e=nil a=1", Dump(S))
        local Ok2, E2 = pcall(SharedTable.increment, S, "e", 1)
        assert(not Ok2 and E2 == E and SharedTable.size(S) == 6)
        -- copies keep it; one left in a shallow clone is the clone's alone
        assert(Dump(SharedTable.clone(S)) == Dump(S) and Dump(SharedTable.clone(S, true)) == Dump(S) and Dump(SharedTable.cloneAndFreeze(S, true)) == Dump(S))
        local C = SharedTable.clone(S)
        pcall(SharedTable.increment, C, "c", 1)
        assert(SharedTable.size(C) == 7 and SharedTable.size(S) == 6)
        -- update's callback gets nil, and its nil result takes it out, as nil and a dropped value written do
        local Got = "unset"
        SharedTable.update(S, "e", function(V)
            Got = V
            return nil
        end)
        S[7] = nil
        S[0] = coroutine.create(print)
        assert(Got == nil and Dump(S) == "1=10 2=20 a=1", Dump(S))
        -- a value written over it is stored as any other
        local W = SharedTable.new()
        pcall(SharedTable.increment, W, "w", 1)
        assert(SharedTable.size(W) == 1)
        W.w = 5
        assert(SharedTable.increment(W, "w", 1) == 5 and W.w == 6 and SharedTable.size(W) == 1)
        -- not left when the frozen flag, the delta or the key refuse the call, or the entry is not a number
        local F = SharedTable.cloneAndFreeze(SharedTable.new())
        local T = SharedTable.new({ s = "x" })
        pcall(SharedTable.increment, F, "q", 1)
        pcall(SharedTable.increment, T, "q", "1")
        pcall(SharedTable.increment, T, "q", nil)
        pcall(SharedTable.increment :: any, T, "q")
        pcall(SharedTable.increment :: any, T, true, 1)
        pcall(SharedTable.increment, T, "s", 1)
        assert(SharedTable.size(F) == 0 and SharedTable.size(T) == 1)
        -- clear takes it out, and a loop that adds up the values fails on it, as a client's does
        local L = SharedTable.new({ n = 1 })
        pcall(SharedTable.increment, L, "zz", 1)
        assert(not pcall(function()
            local Total = 0
            for _, V in L do
                Total += V
            end
        end))
        SharedTable.clear(L)
        assert(SharedTable.size(L) == 0)
        -- an update marks its table first, so a failed increment in its callback leaves nothing, inside it or after (it is not a
        -- change for update either), while a key with no value left before stays
        local U, Calls, Inside = SharedTable.new({ a = 1 }), 0, nil
        SharedTable.update(U, "a", function(V)
            Calls += 1
            pcall(SharedTable.increment, U, "zz", 1)
            Inside = SharedTable.size(U)
            return V + 1
        end)
        assert(Calls == 1 and Inside == 1 and U.a == 2 and SharedTable.size(U) == 1, `{Calls} {Inside} {SharedTable.size(U)}`)
        pcall(SharedTable.increment, U, "zz", 1)
        local K = SharedTable.new({ a = 1 })
        pcall(SharedTable.increment, K, "zz", 1)
        SharedTable.update(K, "a", function(V) return V + 1 end)
        pcall(SharedTable.increment, K, "yy", 1)
        assert(SharedTable.size(U) == 1 and SharedTable.size(K) == 2 and K.a == 2, `{SharedTable.size(U)} {SharedTable.size(K)}`)
        -- a table an engine value was made of gets no such key (each measured): the size after a failed increment of "zz"
        local Reg = game:GetService("SharedTableRegistry")
        local function Left(Prepare)
            local S = SharedTable.new({ a = 1 })
            local Target = Prepare(S) or S
            pcall(SharedTable.increment, Target, "zz", 1)
            return SharedTable.size(Target)
        end
        local function Nested() return { inner = { a = 1 } } end
        local Frozen = function() return SharedTable.cloneAndFreeze(SharedTable.new()) :: any end
        local Marked = {
            Left(function(S) Reg:SetSharedTable("set", S) Reg:SetSharedTable("set", nil) end),
            Left(function(S) local G = Reg:GetSharedTable("made") Reg:SetSharedTable("made", nil) return G end),
            Left(function(S) pcall(function() Reg:SetSharedTable(nil :: any, S) end) end),
            Left(function(S) local T = SharedTable.new() T.x = S end),
            Left(function(S) SharedTable.new({ S }) end),
            Left(function(S) return SharedTable.new(Nested()).inner end),
            Left(function(S) local T = SharedTable.new() T.x = Nested() return T.x.inner end),
            Left(function(S) local T = SharedTable.new() SharedTable.update(T, "x", function() return S end) end),
            Left(function(S) local F = Frozen() pcall(function() F.x = S end) end),
            Left(function(S) local F = Frozen() pcall(function() F.x = { { S } } end) end),
            Left(function(S) local T = SharedTable.new() pcall(function() T.x = { S, print } end) end),
            Left(function(S) local B = Instance.new("BindableEvent") B:Fire(S) B:Destroy() end),
            Left(function(S) local B = Instance.new("BindableEvent") B:Fire({ x = S }) B:Destroy() end),
            Left(function(S) local B = Instance.new("BindableFunction") B.OnInvoke = function() return S end B:Invoke() B:Destroy() end),
            Left(function(S) local B = Instance.new("BindableFunction") B.OnInvoke = function() return 1 end B:Invoke({ S }) B:Destroy() end),
            Left(function(S) SharedTable.clone(S, true) end),
            Left(function(S) return SharedTable.clone(SharedTable.new(Nested()), true).inner end),
            Left(function(S) SharedTable.update(S, "a", function(V) return V end) end),
            Left(function(S) pcall(SharedTable.update, S, "a", function() error("x") end) end),
            Left(function(S) print(S) end),
            Left(function(S) warn(S) end),
            Left(function(S) local P = Instance.new("Folder") pcall(P.SetAttribute, P, "x", S) P:Destroy() end),
        }
        assert(table.concat(Marked, ",") == "1,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1", table.concat(Marked, ","))
        -- and nothing else marks one, a copy of a marked table neither
        local Unmarked = {
            Left(function(S) end),
            Left(function(S) return SharedTable.clone(S) end),
            Left(function(S) return SharedTable.clone(S, true) end),
            Left(function(S) local T = SharedTable.new() T.x = S return SharedTable.clone(S, true) end),
            Left(function(S) local _ = tostring(S) .. SharedTable.size(S) for _ in S do end end),
            Left(function(S) local T = SharedTable.new() pcall(function() T[-1] = S end) end),
            Left(function(S) local T = SharedTable.new() pcall(function() T.x = { print, S } end) end),
            Left(function(S) pcall(SharedTable.update, S, "a", 5 :: any) end),
            Left(function(S) pcall(SharedTable.update, S, true :: any, function(V) return V end) end),
            Left(function(S) pcall(function() game:GetService("HttpService"):JSONEncode(S) end) end),
            Left(function(S) local V = Instance.new("StringValue") pcall(function() V.Value = S :: any end) V:Destroy() end),
            Left(function(S) local T = SharedTable.new({ x = 1 }) SharedTable.update(T, "x", function(V) local _ = SharedTable.size(S) return V end) end),
        }
        assert(table.concat(Unmarked, ",") == "2,2,2,2,2,2,2,2,2,2,2,2", table.concat(Unmarked, ","))
        "#,
    );
}

/// A failed increment that leaves its key in a list of colliding keys searches the list once, as a write adding the key does
/// (cost.rs SHARED_LIST_SCANNED; ree searched it again to add the key, 7.5 us more at 1,023 keys), past its error's own price
/// (cost.rs RAISED).
#[test]
fn a_failed_increment_searches_a_colliding_list_once() {
    assert_shared_prices(&format!(
        "{COLLIDING_KEYS}\nlocal Raised = {:e}\n{}",
        crate::cost::RAISED * 1e6,
        r#"
        local Keys = {}
        for I = 0, 1023 do Keys[I] = Key(I, 10) end
        local T = SharedTable.new()
        for I = 0, 1022 do T[Keys[I]] = I end
        local Added = Per(1, function() T[Keys[1023]] = 1 end)
        T[Keys[1023]] = nil
        local Failed = Per(1, function() pcall(SharedTable.increment, T, Keys[1023], 1) end)
        assert(SharedTable.size(T) == 1024)
        Within("a failed increment's key added to 1,023 beyond a write's and its error's", Failed - Added - Raised, -1, 1)
        "#
    ));
}

/// The SharedTable library's argument errors as a playing client words and orders them (Studio 0.748, two runs of each probe): a
/// call with no argument at all raises Luau's "missing argument #1 to 'size' (SharedTable expected)" at the script's line, nil
/// "invalid argument #1 ... got nil"; clone and cloneAndFreeze given two arguments or more check `deep` before the table;
/// increment with its delta left out raises "Failed to get value for insertion into SharedTable", after the key's check and
/// before the frozen flag's, a nil delta "Variant cast failed". ree said "got no value", checked the table first, and gave
/// "Variant cast failed" or "SharedTable is frozen" for a delta left out. The library's keys loop in a client's order too.
#[test]
fn shared_table_library_arguments_are_checked_as_a_client_does() {
    assert_script(
        r##"
        local function Fails(Message, F, ...)
            local Ok, E = pcall(F, ...)
            assert(not Ok and string.gsub(tostring(E), "^[^\n]-:%d+: ", "") == Message, tostring(E))
        end
        local function Positioned(F)
            local _, E = pcall(F)
            return string.match(tostring(E), "^[^\n]-:%d+: ") ~= nil
        end
        local Any = SharedTable :: any
        local S = SharedTable.new({ a = 1 })
        local F = SharedTable.cloneAndFreeze(S)
        for _, Name in { "clone", "cloneAndFreeze", "isFrozen", "size", "clear", "increment", "update" } do
            Fails(`missing argument #1 to '{Name}' (SharedTable expected)`, Any[Name])
            Fails(`invalid argument #1 to '{Name}' (SharedTable expected, got nil)`, Any[Name], nil)
            assert(Positioned(function() return Any[Name]() end), Name)
        end
        Fails("invalid argument #2 to 'clone' (boolean expected, got number)", Any.clone, 5, 5)
        Fails("invalid argument #2 to 'cloneAndFreeze' (boolean expected, got nil)", Any.cloneAndFreeze, nil, nil)
        Fails("invalid argument #2 to 'clone' (boolean expected, got Instance)", Any.clone, workspace, workspace)
        Fails("invalid argument #1 to 'clone' (SharedTable expected, got number)", Any.clone, 5)
        Fails("invalid argument #1 to 'cloneAndFreeze' (SharedTable expected, got number)", Any.cloneAndFreeze, 5, true)
        Fails("Failed to get value for insertion into SharedTable", Any.increment, S, "a")
        Fails("Failed to get value for insertion into SharedTable", Any.increment, S, "missing")
        Fails("Failed to get value for insertion into SharedTable", Any.increment, F, "a")
        Fails("SharedTable keys can only be strings or numbers.", Any.increment, S, true)
        Fails("Variant cast failed", Any.increment, S, "a", nil)
        Fails("SharedTable is frozen", Any.increment, F, "a", nil)
        assert(not Positioned(function() return Any.increment(S, "a") end) and SharedTable.size(S) == 1 and S.a == 1)
        local Names = {}
        for Name in SharedTable do
            table.insert(Names, Name)
        end
        assert(table.concat(Names, ",") == "cloneAndFreeze,clear,clone,isFrozen,size,increment,update,new", table.concat(Names, ","))
        "##,
    );
}

/// A SharedTable stores a copy of a Path2DControlPoint, every way in, as a playing client does (Studio 0.748, two runs: a write,
/// new, a table nested in new or in a write, update's result alone and in a table; changing the script's object afterwards left
/// each stored value as it was, and a read is a copy as well). ree pooled the script's own object, so the change showed through.
#[test]
fn a_shared_table_stores_a_copy_of_a_path2d_control_point() {
    assert_script(
        r#"
        local function Point()
            return Path2DControlPoint.new(UDim2.new(0, 1, 0, 1), UDim2.new(0, 2, 0, 2), UDim2.new(0, 3, 0, 3))
        end
        local Want = Point()
        local A, B, C, D, E, G = Point(), Point(), Point(), Point(), Point(), Point()
        local S = SharedTable.new({ b = B, n = { c = C } })
        S.a = A
        S.t = { d = D }
        SharedTable.update(S, "e", function() return E end)
        SharedTable.update(S, "g", function() return { g = G } end)
        for _, Changed in { A, B, C, D, E, G } do
            Changed.Position = UDim2.new(0, 50, 0, 50)
        end
        assert(S.a == Want and S.b == Want and S.n.c == Want and S.t.d == Want and S.e == Want and S.g.g == Want, tostring(S.a))
        local Read = S.a
        Read.LeftTangent = UDim2.new(0, 70, 0, 70)
        assert(S.a == Want and Read ~= Want)
        "#,
    );
}

/// A SharedTable read costs the object it makes for the script and an integer key it finds, as a playing client's does (Studio
/// 0.748, medians of 7 batches, two runs; cost.rs SHARED_MADE_VALUE and after): a number 0.023-0.024 us, a CFrame 0.038-0.053,
/// an EnumItem 0.039, a TweenInfo 0.047-0.053, a Region3 0.056-0.059, a SharedTable 0.047-0.057; a number by an integer key
/// 0.049-0.052; a 16-byte string by a short string key 0.063-0.069 and by an integer key 0.088-0.099. ree charged 0.018-0.022
/// for each. A loop's step pays for its value's object as a read does.
#[test]
fn shared_table_reads_cost_the_objects_they_make() {
    assert_shared_prices(
        r#"
        local T = SharedTable.new({ x = 1 })
        local function Read(Value)
            T.v = Value
            return Per(2000, function(N) for _ = 1, N do local _ = T.v end end)
        end
        Within("a number read", Read(12345.5), 0.015, 0.03)
        Within("a CFrame read", Read(CFrame.new(1, 2, 3)), 0.03, 0.06)
        Within("an EnumItem read", Read(Enum.Material.Plastic), 0.03, 0.05)
        Within("a TweenInfo read", Read(TweenInfo.new(1)), 0.04, 0.065)
        Within("a Region3 read", Read(Region3.new(Vector3.zero, Vector3.one)), 0.045, 0.07)
        Within("a SharedTable read", Read(SharedTable.new({ a = 1 })), 0.04, 0.065)
        local Array = SharedTable.new({ 1, 2, 3, 4, 5, 6, 7, 8, 9, 10 })
        Within("a number read by an integer key", Per(2000, function(N) for _ = 1, N do local _ = Array[5] end end), 0.04, 0.06)
        T.s, Array[5] = "abcdefghijklmnop", "abcdefghijklmnop"
        Within("a 16-byte string read by a string key", Per(2000, function(N) for _ = 1, N do local _ = T.s end end), 0.05, 0.08)
        Within("a 16-byte string read by an integer key", Per(2000, function(N) for _ = 1, N do local _ = Array[5] end end), 0.075, 0.11)
        local function Filled(Make)
            local Plain = {}
            for I = 1, 1000 do
                Plain["k" .. I] = Make(I)
            end
            return SharedTable.new(Plain)
        end
        local function Loop(Table) return Per(20, function(N) for _ = 1, N do for _ in Table do end end end) / 1000 end
        local Numbers = Loop(Filled(function(I) return I + 0.5 end))
        Within("a loop's step over CFrames beyond one over numbers", Loop(Filled(function(I) return CFrame.new(I, 0, 0) end)) - Numbers, 0.019, 0.026)
        Within("a loop's step over Rays beyond one over numbers", Loop(Filled(function(I) return Ray.new(Vector3.new(I, 0, 0), Vector3.yAxis) end)) - Numbers, 0.019, 0.026)
        Within("a loop's step over TweenInfos beyond one over numbers", Loop(Filled(function(I) return TweenInfo.new(I) end)) - Numbers, 0.019, 0.026)
        Within("a loop's step over Path2DControlPoints beyond one over numbers", Loop(Filled(function(I) return Path2DControlPoint.new(UDim2.fromOffset(I, 0)) end)) - Numbers, 0.019, 0.026)
        Within("a loop's step over SharedTables beyond one over numbers", Loop(Filled(function(I) return { a = I } end)) - Numbers, 0.019, 0.026)
        "#,
    );
}

/// A loop's step pays one price for the object it makes, whatever its kind, as a playing client's does (Studio 0.748, os.clock,
/// loops over 1,000 and 10,000 string-keyed entries, two runs: 0.019 to 0.026 us beyond a step over numbers at 1,000 entries for a
/// CFrame, a Ray, a TweenInfo, a Path2DControlPoint and a SharedTable alike; cost.rs SHARED_LOOP_MADE_VALUE), and an update's old
/// value (and its result) a read's price: the VM's count of a copy's own calls is not the script's. ree charged a step a read's
/// price, a TweenInfo's and a Path2DControlPoint's with the VM's count of the copy's calls on top (0.0415 and 0.045 beyond a
/// number's), and an update of a TweenInfo or a Path2DControlPoint 0.023 and 0.030 us more than its read's price.
#[test]
fn shared_table_loop_steps_and_update_values_cost_their_object_once() {
    assert_shared_prices(
        r#"
        local T = SharedTable.new({ x = 1 })
        local function Same(V) return V end
        local function Updated(Value)
            T.v = Value
            return Per(2000, function(N) for _ = 1, N do SharedTable.update(T, "v", Same) end end)
        end
        -- (a read's prices: a TweenInfo or a Path2DControlPoint 0.03 us, a CFrame or a Ray 0.017; a first round takes the first
        -- calls' cold costs out of the comparison)
        Updated(1)
        local Frame = Updated(CFrame.new(1, 2, 3))
        Within("an update's old TweenInfo beyond a CFrame", Updated(TweenInfo.new(1)) - Frame, 0.012, 0.014)
        Within("an update's old Path2DControlPoint beyond a CFrame", Updated(Path2DControlPoint.new()) - Frame, 0.012, 0.014)
        Within("an update's old Ray beyond a CFrame", Updated(Ray.new(Vector3.zero, Vector3.one)) - Frame, -0.001, 0.001)
        "#,
    );
}

/// A conversion walks every slot of a table's array and hash parts, empty or not, and pays for the empty ones as a playing client
/// does (Studio 0.748, two runs: new of `table.create(n)` holding one entry 20 us at 100,000, 196 to 213 at 1,000,000, 529 to 533
/// at 2,000,000, about 0.2 ns a slot; of `{ x = 1 }` 0.33 to 0.37; an empty hash node is priced as a slot, not measured); they count
/// against ree's guard on a copy as a copy's holes do. ree charged one entry, then 1.16 ns a slot from an earlier session's one row
/// (`table.create(4000000)`, 4,634 us).
#[test]
fn a_shared_table_conversion_pays_for_the_empty_slots_it_walks() {
    let guard = crate::lua::bindable::COPY_VALUES;
    assert_shared_prices(
        &r#"
        local Sparse = table.create(1000000)
        Sparse.x = 1
        Within("new of a 1,000,000-slot array holding one entry", Per(5, function(N) for _ = 1, N do SharedTable.new(Sparse) end end), 150, 260)
        Within("new of { x = 1 }", Per(5, function(N) for _ = 1, N do SharedTable.new({ x = 1 }) end end), 0, 5)
        local Target = SharedTable.new()
        Within("a 1,000,000-slot array written to a key", Per(5, function(N) for _ = 1, N do Target.k = Sparse end end), 150, 260)
        local Huge = table.create(GUARD + 1)
        Huge.x = 1
        local Ok, E = pcall(SharedTable.new, Huge)
        assert(not Ok and E == "ree: this copy would make more than GUARD values (a table is copied once for each place it appears)", tostring(E))
        "#
        .replace("GUARD", &guard.to_string()),
    );
}

/// SharedTable.increment's messages quote the key as a playing client's do (Studio 0.748: "a\0b" quoted "a", a message of a 1 MB
/// key 1,000,000 bytes long), and their bytes are priced as a string made (a client's increment of a missing 16 MB key took 1.6 ns
/// a byte, its message included). ree quoted the whole key and charged neither.
#[test]
fn shared_table_increment_messages_quote_the_key_as_roblox_does() {
    assert_shared_prices(
        r#"
        local S = SharedTable.new({ s = "x" })
        local function Message(Key)
            local _, E = pcall(SharedTable.increment, S, Key, 1)
            return tostring(E)
        end
        assert(Message("a\0b") == 'Unable to increment the value for key "a" because it does not exist in the SharedTable.', Message("a\0b"))
        assert(Message("s") == 'Unable to increment the value for key "s" because it is not a number value.')
        -- (the key's own bytes, UTF-8 or not)
        assert(Message("a\xffb") == 'Unable to increment the value for key "a\xffb" because it does not exist in the SharedTable.', Message("a\xffb"))
        S["b\xff"] = "x"
        assert(Message("b\xff") == 'Unable to increment the value for key "b\xff" because it is not a number value.', Message("b\xff"))
        local Long = string.rep("k", 1048576)
        local Text = Message(Long)
        assert(#Text == 1000000 and string.sub(Text, 1, 39) == 'Unable to increment the value for key "' and string.sub(Text, -3) == "kkk", #Text)
        Within("increment of a missing 1 MB key", Per(2, function(N) for _ = 1, N do pcall(SharedTable.increment, S, Long, 1) end end), 700, 2000)
        "#,
    );
}

/// SocialService:GetUpcomingExperienceEventsAsync and GroupService:GetRolesInGroupAsync returned nil, so a lobby's event handler
/// failed on `#data` and its group check on `.IsMember`. Studio: an empty list, and a table with
/// IsMember false and an empty Roles list (an empty test place, 2026-10-23).
#[test]
fn social_and_group_services_answer_as_studio_does_for_an_offline_player() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Events = game:GetService("SocialService"):GetUpcomingExperienceEventsAsync()
        assert(typeof(Events) == "table" and #Events == 0, "events")
        local Roles = game:GetService("GroupService"):GetRolesInGroupAsync(1, 1234567)
        assert(typeof(Roles) == "table" and Roles.IsMember == false and #Roles.Roles == 0, "roles")
        print("studio's answers")
        "#,
    );
    assert!(output.contains(&"print: studio's answers".to_string()), "{output:?}");
}

/// Player's group methods, the Async ones (public issue 19: "IsInGroupAsync is not a valid member of Player") and the older
/// ones, as a 0.747 client and server answer for the offline player (tools/parity/probes/loopback/player_groups.luau): in no
/// group, the six sharing the ids it was asked about (the first call naming one yields, later ones by any of the six do not),
/// the id cast as an int64 argument, and the errors of a cast and of a call without the player
#[test]
fn players_group_methods_answer_as_a_client_does_for_the_offline_player() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Player = game:GetService("Players").LocalPlayer
        local function Call(Name, ...)
            local Ran = false
            task.defer(function() Ran = true end)
            local Ok, Answer = pcall(Player[Name], Player, ...)
            return tostring(Answer) .. (if not Ok then "!" elseif Ran then " yielded" else "")
        end
        print(Call("IsInGroupAsync", 7), Call("IsInGroupAsync", 7), Call("GetRankInGroup", 7), Call("GetRoleInGroupAsync", 7))
        print(Call("GetRankInGroupAsync", 8), Call("GetRoleInGroupAsync", 8), Call("IsInGroup", 8), Call("GetRoleInGroup", 9))
        print(Call("IsInGroupAsync", 10.5), Call("IsInGroupAsync", 11), Call("IsInGroupAsync", true), Call("IsInGroupAsync", 1))
        print(Call("IsInGroupAsync", 0 / 0), Call("IsInGroupAsync", 0), Call("IsInGroupAsync", 1e20), Call("IsInGroupAsync", -2 ^ 63))
        print(Call("IsInGroupAsync", " 012 "), Call("IsInGroupAsync", 12), Call("IsInGroupAsync", "9007199254740993"), Call("IsInGroupAsync", 2 ^ 53))
        print(Call("IsInGroupAsync", "+13"), Call("IsInGroupAsync", "13.0"), Call("IsInGroupAsync", "9223372036854775808"), Call("IsInGroupAsync", "-9223372036854775808"))
        print(Call("GetRankInGroupAsync"), Call("GetRankInGroupAsync", {}), Call("GetRankInGroupAsync", { a = 1 }), Call("GetRankInGroupAsync", Enum.KeyCode.A), Call("GetRankInGroupAsync", print))
        print(pcall(Player.IsInGroupAsync, workspace, 7))
        print(pcall(Player.GetRoleInGroupAsync))
        print(Player.IsInGroupAsync == Player.IsInGroupAsync, Player.IsInGroupAsync == Player.IsInGroup)
        "#,
    );
    for _ in 0..4 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = lines(&engine);
    assert_eq!(
        output,
        [
            "print: false yielded false 0 Guest",
            "print: 0 yielded Guest false Guest yielded",
            "print: false yielded false false yielded false",
            "print: false yielded false false yielded false",
            "print: false yielded false false yielded false yielded",
            "print: Unable to cast string to int64! Unable to cast string to int64! Unable to cast string to int64! false",
            "print: Argument 1 missing or nil! Unable to cast Array to int64! Unable to cast Dictionary to int64! Unable to cast KeyCode to int64! Unable to cast Function to int64!",
            "print: false Expected ':' not '.' calling member function IsInGroupAsync",
            "print: false Expected ':' not '.' calling member function GetRoleInGroupAsync",
            "print: true false",
        ],
        "{output:?}"
    );
}

/// A one-letter Text write's and a workspace part's CFrame write's price in a client of `place_id` with `prices` and `parts` parts
/// in the workspace, in us a write
fn place_write_prices(place_id: f64, parts: usize, prices: crate::cost::PlacePrices) -> Vec<f64> {
    let engine = Engine::new(Options { place_id, place_prices: prices, mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"
        for _ = 1, {parts} do
            local P = Instance.new("Part")
            P.Anchored = true
            P.Parent = workspace
        end
        local Label = Instance.new("TextLabel")
        local Target = Instance.new("Part")
        Target.Parent = workspace
        local A, B = CFrame.new(1, 2, 3), CFrame.new(3, 2, 1)
        local Start = os.clock()
        for _ = 1, 200 do
            Label.Text = "a"
            Label.Text = "b"
        end
        local Text = (os.clock() - Start) / 400 * 1e6
        Start = os.clock()
        for _ = 1, 200 do
            Target.CFrame = A
            Target.CFrame = B
        end
        print(string.format("%.2f %.2f", Text, (os.clock() - Start) / 400 * 1e6))
        "#
        ),
    );
    let line = lines(&engine).into_iter().find_map(|line| line.strip_prefix("print: ").map(str::to_string)).unwrap_or_else(|| panic!("a result: {:?}", lines(&engine)));
    line.split(' ').map(|number| number.parse::<f64>().unwrap()).collect::<Vec<f64>>()
}

/// A listed place pays its own lookup (a place_size_writes probe, a match place's playtest client, five sessions: a one-letter text
/// 56-59 us a write), and a listed match place scales a part's geometry write in the workspace (`REE_MATCH_PLACES`: Studio playtests
/// read 1.69-1.83 us there against a lobby's 1.10; the parity bench's match-place client 1.43, ree's default price). A part count or a GUI
/// tree size does not move any write price (same probe, 0 to 20,000 parts and 0 to 6,000 frames: CFrame 1.58-1.72 us, Size 0.31-0.37, flat).
#[test]
fn a_listed_place_pays_its_own_prices_and_a_bigger_place_costs_no_more() {
    for place in [MATCH, TEST_MATCH] {
        let small = place_write_prices(place, 0, priced_places());
        assert!((50.0..68.0).contains(&small[0]), "place {place}, Text: {small:?}");
    }
    let (small, big) = (place_write_prices(MATCH, 0, priced_places()), place_write_prices(MATCH, 3000, priced_places()));
    assert!((small[1] - big[1]).abs() < 0.05 * small[1], "a CFrame write in a place of 3,000 parts: {big:?}, of none: {small:?}");
    assert!((1.9..2.6).contains(&small[1]), "a listed match place, CFrame: {small:?}");
    let lobby = place_write_prices(LOBBY, 0, priced_places());
    assert!((1.2..1.7).contains(&lobby[1]), "lobby, CFrame: {lobby:?}");
}


/// The signals the first handler_cost probe left unpriced, and a signal with several handlers (a second one, a match place's client,
/// three runs, 100 instances triggered a frame by a thread in PreSimulation): a BindableEvent:Fire with a handler queues for 0.58 us
/// (1.73 us the handler's thread), ChildRemoved 0.42 / 1.50, DescendantRemoving 0.35 / 1.58, AncestryChanged 0.65 / 2.17. A signal with
/// 10 handlers on each of 100 parts queued for 4.55-4.99 us a fire (each handler beyond the first 0.45) and its handlers' threads cost
/// 0.87-1.01 us each where a lone handler's cost 1.6-2.8; 100 handlers on 10 parts 43.0-45.1 and 0.91-1.04. The handlers of a frame event
/// run 0.63-0.73 us apart (100 on PreSimulation, Heartbeat and PreRender). ree charged the Bindable, the removals and AncestryChanged
/// nothing, and a handler beyond the first the queueing of none and the thread of a lone one. Bands are +-35 % of Studio's medians.
#[test]
fn bindable_removal_ancestry_and_many_handler_signals_cost_what_studio_measured() {
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local RunService = game:GetService("RunService")
        local N, PER = 100, 20
        local Parts, Items, Events = {}, {}, {}
        for Index = 1, N do
            local Part = Instance.new("Part")
            Part.Parent = workspace
            Parts[Index] = Part
            Items[Index] = Instance.new("Folder")
            Events[Index] = Instance.new("BindableEvent")
        end
        local Root = Instance.new("Folder")
        Root.Parent = workspace
        local Container = Instance.new("Folder")
        Container.Parent = Root
        local Container2 = Instance.new("Folder")
        Container2.Parent = Root
        local Last, Count = 0, 0
        local function Handler() Last = os.clock() Count += 1 end
        local function AllIn() for Index = 1, N do Items[Index].Parent = Container end end
        local function Many(Per, Fires)
            return function()
                local List = {}
                for Index = 1, Fires do for _ = 1, Per do table.insert(List, Parts[Index].Changed:Connect(Handler)) end end
                return List
            end
        end
        local Write = function(Fires) return function(Frame) for Index = 1, Fires do Parts[Index].Transparency = (Frame % 2) * 0.5 end end end
        -- { name, connect, trigger, cleanup, setup, handlers per signal, fires }
        local Kinds = {
            { "Bindable", function() local List = {} for Index = 1, N do table.insert(List, Events[Index].Event:Connect(Handler)) end return List end,
                function(Frame) for Index = 1, N do Events[Index]:Fire(Frame) end end },
            { "ChildRemoved", function() return { Container.ChildRemoved:Connect(Handler) } end,
                function() for Index = 1, N do Items[Index].Parent = nil end end, AllIn, AllIn },
            { "DescendantRemoving", function() return { Root.DescendantRemoving:Connect(Handler) } end,
                function() for Index = 1, N do Items[Index].Parent = nil end end, AllIn, AllIn },
            { "AncestryChanged", function() local List = {} for Index = 1, N do table.insert(List, Items[Index].AncestryChanged:Connect(Handler)) end return List end,
                function(Frame) local To = if Frame % 2 == 0 then Container else Container2 for Index = 1, N do Items[Index].Parent = To end end, nil, AllIn },
            { "Changed x10", Many(10, N), Write(N), nil, nil, 10, N },
            { "Changed x100", Many(100, 10), Write(10), nil, nil, 100, 10 },
        }
        local function Median(List) table.sort(List) return List[(#List + 1) // 2] end
        _G.Rows = {}
        task.spawn(function()
            for _, Kind in Kinds do
                local H, Fires = Kind[6] or 1, Kind[7] or N
                local Alone, Queued, Run = {}, {}, {}
                for _, Connected in { false, true } do
                    if Kind[5] then Kind[5]() end
                    local List = if Connected then Kind[2]() else {}
                    for Frame = 1, PER do
                        RunService.PreSimulation:Wait()
                        Count = 0
                        local T0 = os.clock()
                        Kind[3](Frame)
                        local T1 = os.clock()
                        RunService.PostSimulation:Wait()
                        if Frame > 4 then
                            if Connected then
                                table.insert(Queued, T1 - T0)
                                if Count == Fires * H then table.insert(Run, Last - T1) end
                            else
                                table.insert(Alone, T1 - T0)
                            end
                        end
                        if Kind[4] then Kind[4]() end
                    end
                    for _, Connection in List do Connection:Disconnect() end
                end
                _G.Rows[Kind[1]] = { Queued = (Median(Queued) - Median(Alone)) / Fires * 1e9, Run = Median(Run) / (Fires * H) * 1e9 }
            end
            -- the handlers of a frame event, 100 on each: first line to last, over 99
            _G.Span = {}
            for _, EventName in { "PreSimulation", "Heartbeat", "PreRender" } do
                local First, Final, Seen, Spans = 0, 0, 0, {}
                local List = {}
                for _ = 1, N do
                    table.insert(List, RunService[EventName]:Connect(function()
                        local Now = os.clock()
                        if Seen == 0 then First = Now end
                        Seen += 1
                        Final = Now
                    end))
                end
                for Frame = 1, PER do
                    RunService.PostSimulation:Wait()
                    Seen = 0
                    RunService.PostSimulation:Wait()
                    if Frame > 4 and Seen == N then table.insert(Spans, (Final - First) / (N - 1) * 1e9) end
                end
                for _, Connection in List do Connection:Disconnect() end
                _G.Span[EventName] = Median(Spans)
            end
            _G.Done = true
        end)
        "##,
    );
    for _ in 0..400 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    run(
        &lua,
        &engine,
        r##"
        assert(_G.Done, "the probe did not finish")
        -- ns: Studio's medians, queued per fire and run per handler thread, at the quarter the parity bench reads them at (cost.rs
        -- signal_costs; a Changed fire's own queueing 200 ns, unscaled)
        local Q = 0.247
        local Studio = {
            Bindable = { 580 * Q, 1730 * Q }, ChildRemoved = { 420 * Q, 1500 * Q }, DescendantRemoving = { 350 * Q, 1580 * Q },
            AncestryChanged = { 650 * Q, 2170 * Q }, ["Changed x10"] = { 200 + 9 * 450 * Q, 950 * Q }, ["Changed x100"] = { 200 + 99 * 450 * Q, 943 * Q },
        }
        for Name, Row in _G.Rows do
            local Want = Studio[Name]
            assert(Row.Queued > Want[1] * 0.65 and Row.Queued < Want[1] * 1.35, string.format("%s queued %.0f ns, Studio %d", Name, Row.Queued, Want[1]))
            assert(Row.Run > Want[2] * 0.65 and Row.Run < Want[2] * 1.35, string.format("%s run %.0f ns, Studio %d", Name, Row.Run, Want[2]))
        end
        for Name, Span in _G.Span do
            assert(Span > 450 and Span < 950, string.format("%s handlers %.0f ns apart, Studio 630-730", Name, Span))
        end
        print("ok")
        "##,
    );
    assert!(lines(&engine).contains(&"print: ok".to_string()), "{:?}", lines(&engine));
}

/// The boundary profiler (src/boundary.rs) books each crossing from a script into the engine to the line of the game's Luau
/// that made it, with the cost the work clock charged: a property read and write by Class.Member, a method, an event connect,
/// a datatype operation (booked to the call before it, among the VM's host calls), a task thread; a write of what the property holds counts as unchanged
/// and a read of a property the same function read this frame as a repeat. Nothing is counted when it is off.
#[test]
fn the_boundary_profiler_counts_crossings_per_call_site() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("ree-boundary-{}-{stamp}.json", std::process::id()));
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    assert!(!crate::boundary::active());
    crate::boundary::begin(crate::boundary::Config { out: path.clone(), top: 5, phase: None });
    crate::luau_profile::install(&lua);
    scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let script = [
        "local Part = Instance.new(\"Part\")",
        "Part.Parent = workspace",
        "local function Frame()",
        "    local a = Part.Transparency",
        "    local b = Part.Transparency",
        "    Part.Transparency = 0.5",
        "    Part.Transparency = 0.5",
        "    Part:GetAttribute(\"X\")",
        "    local c = CFrame.new(1, 2, 3)",
        "    local d = c * c",
        "    Part.Changed:Connect(function() end)",
        "    task.spawn(function() end)",
        "end",
        "Frame()",
        "",
    ]
    .join("\n");
    run(&lua, &engine, &script);
    let text = crate::boundary::finish().expect("report");
    assert!(!crate::boundary::active());
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("report file")).expect("json");
    let _ = std::fs::remove_file(&path);
    let row = |kind: &str, member: &str, line: i64| -> serde_json::Value {
        report["by_site_count"]
            .as_array()
            .unwrap()
            .iter()
            .chain(report["by_site_host_calls"].as_array().unwrap())
            .find(|row| row["kind"] == kind && row["member"] == member && row["line"] == line && row["script"] == "Test")
            .cloned()
            .unwrap_or_else(|| panic!("no {kind} {member} at line {line}\n{text}"))
    };
    // reads: two lines, the second a repeat of the first (the same instance and property, this function, this frame)
    assert_eq!((row("get", "BasePart.Transparency", 4)["count"].as_u64(), row("get", "BasePart.Transparency", 4)["repeat_reads"].as_u64()), (Some(1), Some(0)));
    assert_eq!(row("get", "BasePart.Transparency", 5)["repeat_reads"].as_u64(), Some(1));
    // writes: the second stores what the property holds
    assert_eq!(row("set", "BasePart.Transparency", 6)["unchanged_writes"].as_u64(), Some(0));
    assert_eq!(row("set", "BasePart.Transparency", 7)["unchanged_writes"].as_u64(), Some(1));
    assert!(row("set", "BasePart.Transparency", 6)["cost_ms_per_s"].as_f64().unwrap() > row("set", "BasePart.Transparency", 7)["cost_ms_per_s"].as_f64().unwrap());
    // a method, an event connect with the signal's name, a task thread, and the datatype operations booked to the line before them
    assert_eq!(row("method", "Instance.GetAttribute", 8)["count"].as_u64(), Some(1));
    assert_eq!(row("event", "RBXScriptSignal.Connect(Changed)", 11)["count"].as_u64(), Some(1));
    assert_eq!(row("method", "task.spawn", 12)["count"].as_u64(), Some(1));
    let host = "VM.host call (datatype op or engine function)";
    let datatype = row("datatype", host, 9);
    assert!(datatype["count"].as_u64().unwrap() >= 1 && datatype["cost_ms_per_s"].as_f64().unwrap() > 0.0, "{datatype}");
    // (task.spawn's call is counted apart from the crossings; the line before CFrame.new has none, as the
    // import is resolved when the script loads: it used to be looked up through the engine's globals at each run)
    assert_eq!(report["crossings"].as_u64(), Some(report["by_site_count"].as_array().unwrap().iter().filter(|row| row["kind"] != "datatype").map(|row| row["count"].as_u64().unwrap()).sum()));
    // (the datatype operations; task.spawn is a C function, a C call as a client's is, where the prelude's was a host call; an
    // import's lookup through the engine's globals made one more before imports were resolved at load)
    assert!(report["host_calls"].as_u64().unwrap() >= 1);
    // the Instance API's charges are accounted for, by member and by module, over one frame of 1/60 s
    assert!(report["attributed_share_of_instance_api_charges"].as_f64().unwrap() > 0.9, "{text}");
    assert!((report["emulated_seconds"].as_f64().unwrap() - 1.0 / 60.0).abs() < 1e-9);
    assert_eq!(report["by_module"][0]["script"], "Test");
    let members = report["by_member"].as_array().unwrap();
    let transparency = members.iter().find(|row| row["member"] == "BasePart.Transparency" && row["kind"] == "get").unwrap();
    assert_eq!((transparency["reads_per_frame"].as_f64(), transparency["distinct_per_frame"].as_f64()), (Some(2.0), Some(1.0)));
    // off, a run records nothing and the report is gone
    let (lua, engine) = start();
    run(&lua, &engine, "local Part = Instance.new('Part') local _ = Part.Transparency");
    assert!(crate::boundary::finish().is_none());
}

/// The profiler reports the phases asked for added together (`--profile-phase a,b`, or `all`), and a phase in which no virtual time
/// passed (a boot that runs inside one frame) is reported per run, not per second; unnamed, it reports `steady` when the harness
/// named one. The harness names phases through Context.ProfilePhase, which is luau_profile::set_phase.
#[test]
fn the_boundary_profiler_adds_the_phases_asked_for_and_reports_a_boot_per_run() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let read = |path: &std::path::Path| -> serde_json::Value {
        let report = serde_json::from_str(&std::fs::read_to_string(path).expect("report file")).expect("json");
        let _ = std::fs::remove_file(path);
        report
    };
    let probe = "local Part = Instance.new(\"Part\")\nlocal _ = Part.Transparency\nlocal _ = Part.Transparency\n";
    for (wanted, phase, seconds, zero_time, crossings) in [(Some("boot,steady"), "boot,steady", 1.0 / 60.0, false, 6u64), (Some("boot"), "boot", 0.0, true, 3), (None, "steady", 1.0 / 60.0, false, 3)] {
        let path = std::env::temp_dir().join(format!("ree-boundary-phases-{}-{stamp}-{}.json", std::process::id(), wanted.unwrap_or("none")));
        let (lua, engine) = start();
        engine_lua::modules::interpret_all(&lua, &engine);
        crate::boundary::begin(crate::boundary::Config { out: path.clone(), top: 5, phase: wanted.map(str::to_string) });
        crate::luau_profile::install(&lua);
        // a boot inside the first frame's time (none has passed), then a frame of steady state
        crate::boundary::set_phase("boot");
        run(&lua, &engine, probe);
        crate::boundary::set_phase("steady");
        scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(&lua, &engine, probe);
        crate::boundary::finish().expect("report");
        let report = read(&path);
        assert_eq!(report["phase"], phase);
        assert!((report["emulated_seconds"].as_f64().unwrap() - seconds).abs() < 1e-9, "{wanted:?}: {report}");
        assert_eq!(report["zero_time"], zero_time);
        assert_eq!(report["crossings"].as_u64(), Some(crossings), "{wanted:?}");
        assert_eq!(report["phases"].as_array().unwrap().len(), 3, "startup, boot and steady");
    }
}

/// A Luau function iterating a generic for can yield, as on Roblox (the stock VM called it as a C call and raised
/// "attempt to yield across metamethod/C-call boundary"): task.wait and coroutine.yield inside it, any number of
/// loop variables, break, nested loops, varargs, an error inside it caught by pcall with its line, and C
/// iterators (string.gmatch) and plain loops unchanged. Lua 5.1 style iterators (next, state, index) keep their
/// index. The interpreter's FORGLOOP does it (as `run` and `record` interpret every module); natively compiled code
/// keeps the stock C call.
#[test]
fn a_luau_iterator_of_a_generic_for_can_yield() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    run(
        &lua,
        &engine,
        r#"
        local function Frames(Count)
            local Index = 0
            return function()
                Index += 1
                if Index > Count then
                    return nil
                end
                task.wait(0.01)
                return Index, Index * 10, "x" .. Index
            end
        end
        local function Stateful(State, Index)
            task.wait()
            if Index < State then
                return Index + 1
            end
            return nil
        end
        local function Varargs(...)
            local Arguments = table.pack(...)
            coroutine.yield()
            if Arguments[2] >= 2 then
                return nil
            end
            return Arguments[2] + 1
        end
        task.spawn(function()
            local Seen = {}
            for Index, Tens, Name in Frames(3) do
                table.insert(Seen, `{Index}:{Tens}:{Name}`)
            end
            for Index in Frames(5) do
                if Index == 2 then
                    break
                end
                table.insert(Seen, `b{Index}`)
            end
            for Outer in Frames(2) do
                for Inner in Frames(2) do
                    table.insert(Seen, `{Outer}{Inner}`)
                end
            end
            for Index in Stateful, 3, 0 do
                table.insert(Seen, `s{Index}`)
            end
            local Ok, Message = pcall(function()
                for _ in function() task.wait() error("inside the iterator") end do
                end
            end)
            table.insert(Seen, `{Ok} {string.match(Message, ":(%d+): inside the iterator$") ~= nil}`)
            for Word in string.gmatch("a b", "%a") do
                table.insert(Seen, Word)
            end
            for Index, Value in { 7, 8 } do
                table.insert(Seen, `{Index}={Value}`)
            end
            print(table.concat(Seen, " "))
        end)
        local Thread = coroutine.create(function()
            local Seen = {}
            for Index in Varargs, nil, 0 do
                table.insert(Seen, Index)
            end
            return table.concat(Seen, ",")
        end)
        local Resumes = 0
        local Ok, Result = coroutine.resume(Thread)
        while coroutine.status(Thread) == "suspended" do
            Resumes += 1
            Ok, Result = coroutine.resume(Thread)
        end
        print("coroutine", Ok, Result, Resumes)
        "#,
    );
    scheduler::run_until(&lua, &engine, 1.0).unwrap();
    let output = lines(&engine);
    assert!(output.contains(&"print: coroutine true 1,2 3".to_string()), "{output:?}");
    assert!(output.contains(&"print: 1:10:x1 2:20:x2 3:30:x3 b1 11 12 21 22 s1 s2 s3 false true a b 1=7 2=8".to_string()), "{output:?}");
    assert!(!output.iter().any(|line| line.starts_with("error")), "{output:?}");
}

/// HttpService:JSONEncode writes an empty table as an array, "[]", as Roblox's does, at the top and inside (an object's keys
/// in the table's own iteration order, as a playing client writes them: tools/parity/probes/json_more.luau).
#[test]
fn json_encode_writes_an_empty_table_as_an_array() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local HttpService = game:GetService("HttpService")
        print(HttpService:JSONEncode({}), HttpService:JSONEncode({ A = {}, B = { 1 }, C = { D = {} } }))
        print(HttpService:JSONEncode(HttpService:JSONDecode("{}")), #HttpService:JSONDecode("[]"))
        "#,
    );
    assert!(output.contains(&r#"print: [] {"A":[],"C":{"D":[]},"B":[1]}"#.to_string()), "{output:?}");
    assert!(output.contains(&"print: [] 0".to_string()), "{output:?}");
}

/// HttpService:JSONEncode of a table inside itself, or nested past 300 levels, is the script's error, with Roblox's
/// messages (measured in a playing client, Studio 0.744: tools/parity/probes/json_limits.luau; an unbounded copy once
/// recursed until the stack overflowed and the process ended).
#[test]
fn json_encode_refuses_cycles_and_deep_nesting() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local HttpService = game:GetService("HttpService")
        local Cycle = { A = {} }
        Cycle.A.Back = Cycle
        print(pcall(function() return HttpService:JSONEncode(Cycle) end))
        local Shared = { 1 }
        print(HttpService:JSONEncode({ A = Shared, B = Shared }))
        local function Nest(Depth)
            local Root = {}
            local At = Root
            for _ = 2, Depth do
                At[1] = {}
                At = At[1]
            end
            return Root
        end
        print(#HttpService:JSONEncode(Nest(300)), pcall(function() return HttpService:JSONEncode(Nest(301)) end))
        print(pcall(function() return HttpService:JSONEncode(Nest(100000)) end))
        local Ring = Nest(300)
        local At = Ring
        while At[1] do
            At = At[1]
        end
        At[1] = Ring
        print("ring", pcall(function() return HttpService:JSONEncode(Ring) end))
        "#,
    );
    assert!(output.contains(&"print: false tables cannot be cyclic".to_string()), "{output:?}");
    assert!(output.contains(&r#"print: {"A":[1],"B":[1]}"#.to_string()), "{output:?}");
    let deep = "Max lua stack size reached";
    assert!(output.contains(&format!("print: 600 false {deep}")), "{output:?}");
    assert!(output.contains(&format!("print: false {deep}")), "{output:?}");
    // (a cycle closing one level past the limit raises the depth's error, as in a 0.747 client: table_depth.luau)
    assert!(output.contains(&format!("print: ring false {deep}")), "{output:?}");
}

/// JSONEncode writes a table out each time it appears, as Roblox does (a 0.747 client wrote 10,000,000 values, 20 MB, in
/// 0.31 s), and stops past ree's guard on a copy (lua/bindable.rs `COPY_VALUES`, every value written counted) with its own
/// error: the same 100,000 zeros held 100,000 times (10^10 values, 20 GB of text without the guard) and a tree of
/// `{ t, t }` 40 deep raise it once 4,194,304 values are written (about 8 MB); a table of 1,000 zeros held 1,000 times is
/// written whole.
#[test]
fn json_encode_stops_at_rees_guard_on_a_table_written_for_each_place_it_appears() {
    let (lua, engine) = start();
    let guard = crate::lua::bindable::COPY_VALUES;
    let output = run(
        &lua,
        &engine,
        &r#"
        local HttpService = game:GetService("HttpService")
        local Huge = table.create(100000, table.create(100000, 0))
        local Tree = {}
        for _ = 1, 40 do
            Tree = { Tree, Tree }
        end
        local function Try(Name, Value)
            print(Name, pcall(function() return #HttpService:JSONEncode(Value) end))
        end
        Try("huge", Huge)
        Try("tree", Tree)
        Try("dictionaries", table.create(100000, { a = table.create(100000, 0) }))
        Try("at the guard", table.create(GUARD - 1, 0))
        Try("past the guard", table.create(GUARD, 0))
        Try("fan", table.create(1000, table.create(1000, 0)))
        "#
        .replace("GUARD", &guard.to_string()),
    );
    let past = crate::lua::bindable::too_large(guard, "values");
    let wanted = [
        format!("print: huge false {past}"),
        format!("print: tree false {past}"),
        format!("print: dictionaries false {past}"),
        format!("print: at the guard true {}", 2 * (guard - 1) + 1),
        format!("print: past the guard false {past}"),
        "print: fan true 2002001".to_string(),
    ];
    assert_eq!(output, wanted);
}

/// HttpService:JSONEncode and JSONDecode as a playing client runs them (Studio 0.744, tools/parity/probes/json.luau,
/// json_numbers.luau, json_more.luau, json_limits.luau): arrays to the border with holes as null, object keys named as a
/// bindable names them, unsupported values as null, numbers in Grisu2's digits with the client's rounding, a buffer and
/// non-finite numbers as Roblox's objects; a strict decoder with correctly rounded numbers and Roblox's nesting limits.
#[test]
fn json_encodes_and_decodes_as_a_playing_client_does() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Http = game:GetService("HttpService")
        local function Is(Got, Want)
            assert(Got == Want, `{Want} came out as {Got}`)
        end
        Is(Http:JSONEncode({ 1, nil, 3 }), "[1,null,3]")
        Is(Http:JSONEncode({ 1, 2, Key = 3 }), "[1,2]")
        Is(Http:JSONEncode({ [0] = "z" }), '{"0":"z"}')
        Is(Http:JSONEncode({ [1.5] = "x" }), '{"1":"x"}')
        Is(Http:JSONEncode({ [-0.5] = 1 }), '{"0":1}')
        Is(Http:JSONEncode({ [workspace] = 1 }), '{"<Instance> (Workspace)":1}')
        Is(Http:JSONEncode({ print, workspace, Vector3.new(1, 2, 3), Enum.Material.Plastic }), "[null,null,null,null]")
        Is(Http:JSONEncode({ a = workspace }), '{"a":null}')
        Is(Http:JSONEncode(buffer.create(2)), '{"m":null,"t":"buffer","base64":"AAA="}')
        Is(Http:JSONEncode({ math.huge, -math.huge, 0 / 0 }), '[{"m":null,"t":"numeric","v":"inf"},{"m":null,"t":"numeric","v":"-inf"},{"m":null,"t":"numeric","v":"nan"}]')
        Is(Http:JSONEncode({ -0, 1e20, 1e21, 2 ^ 63, 1e-7, 0.1 + 0.2 }), "[-0,100000000000000000000,1e21,9223372036854776000,1e-7,0.30000000000000007]")
        Is(Http:JSONEncode("\b\f\v\0\27\127/"), '"\\b\\f\\u000B\\u0000\\u001B\127/"')
        assert(not pcall(Http.JSONEncode, Http, "\255"))
        Is(select(2, pcall(Http.JSONEncode, Http, { [true] = 1 })), "Invalid table key type used")
        Is(1 / Http:JSONDecode("-0"), math.huge)
        Is(1 / Http:JSONDecode("-0.0"), -math.huge)
        Is(Http:JSONDecode("195.66666666666666"), 195.66666666666666)
        for _, Bad in { "1e400", '"\\ud83d"', '"tab\tin"', "[1,]", "01", ".5", "5.", "NaN", "" } do
            local Decoded, Error = pcall(Http.JSONDecode, Http, Bad)
            assert(not Decoded and Error == "Can't parse JSON", Bad)
        end
        Is(Http:JSONDecode('"\255"'), "\255")
        local Holes = Http:JSONDecode("[null,1]")
        assert(Holes[1] == nil and Holes[2] == 1)
        Is(Http:JSONDecode('{"1":"x"}')["1"], "x")
        assert(pcall(Http.JSONDecode, Http, string.rep("[", 401) .. string.rep("]", 401)))
        Is(select(2, pcall(Http.JSONDecode, Http, string.rep("[", 402) .. string.rep("]", 402))), "WebParser conversion stack overflow")
        Is(select(2, pcall(Http.JSONDecode, Http, string.rep("[", 1025) .. string.rep("]", 1025))), "Can't parse JSON")
        assert(next(Http:JSONDecode({ 1, 2 })) == nil)
        Is(Http:JSONDecode(12.5), 12.5)
        "#,
    );
    assert!(!output.iter().any(|line| line.starts_with("error")), "{output:?}");
}

/// A playing client's LogService.MessageOut gets each line after the frame's Heartbeat handlers and the deferred work
/// they leave, with its MessageType, a script error with its stack lines; a line a MessageOut handler logs reaches no
/// handler (measured in a playing client, Studio 0.744: tools/parity/probes/logservice.luau, logservice_text.luau).
#[test]
fn a_playing_clients_message_out_arrives_after_the_frames_heartbeat() {
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Log = {}
        _G.Log = Log
        game:GetService("RunService").Heartbeat:Connect(function()
            table.insert(Log, "|hb")
        end)
        game:GetService("LogService").MessageOut:Connect(function(Message, Kind)
            table.insert(Log, Kind.Name .. "[" .. table.concat(string.split(Message, "\n"), "/") .. "]")
            if Message == "outer" then
                warn("inner")
            end
        end)
        warn("outer")
        table.insert(Log, "after-call")
        task.defer(table.insert, Log, "deferred")
        task.spawn(error, "spawned")
        "##,
    );
    for _ in 0..2 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = run(&lua, &engine, r#"print(table.concat(_G.Log, ","))"#);
    let want = "print: after-call,deferred,|hb,MessageWarning[outer],MessageError[spawned/    Stack Begin/    Stack End],|hb";
    assert!(output.contains(&want.to_string()), "{output:?}");
}

#[test]
fn log_history_is_ordered_bounded_and_returns_independent_tables() {
    for server in [false, true] {
        let (lua, engine) = start_with(Options { mode: "play".into(), run_context: if server { super::RunContext::Server } else { super::RunContext::Client }, ..Options::default() });
        let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        run(&lua, &engine, r#"
            local Log = game:GetService("LogService")
            assert(#Log:GetLogHistory() == 0)
            print("first", 2)
            warn("second")
            Log:Info("third")
            pcall(Log.Error, Log, "fourth")
            task.spawn(error, "fifth")
            local History = Log:GetLogHistory()
            assert(#History == 5)
            local Kinds = { "MessageOutput", "MessageWarning", "MessageInfo", "MessageError", "MessageError" }
            for Index, Entry in History do
                assert(Entry.messageType == Enum.MessageType[Kinds[Index]])
                assert(type(Entry.timestamp) == "number" and Entry.timestamp > 1700000000)
            end
            assert(History[1].message == "first 2" and History[2].message == "second")
            assert(History[3].message == "third" and History[4].message == "fourth")
            assert(string.find(History[5].message, "fifth", 1, true))
            History[1].message = "changed"
            History[2] = nil
            local Again = Log:GetLogHistory()
            assert(Again ~= History and Again[1] ~= History[1])
            assert(#Again == 5 and Again[1].message == "first 2")
            _G.LogListener = Log.MessageOut:Connect(function(Message)
                if Message == "outer" then warn("inner") end
            end)
            print("outer")
        "#);
        for _ in 0..2 {
            scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        }
        run(&lua, &engine, r#"
            _G.LogListener:Disconnect()
            local Last = game:GetService("LogService"):GetLogHistory()
            assert(Last[#Last].message == "inner")
        "#);
        let after = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        assert!(engine.state.borrow().log_history.iter().all(|entry| entry.timestamp >= before && entry.timestamp <= after));
        for index in 0..super::LOG_HISTORY_CAP + 3 {
            engine.output("info", index.to_string(), None);
        }
        run(&lua, &engine, r#"
            local History = game:GetService("LogService"):GetLogHistory()
            assert(#History == 10000)
            assert(History[1].message == "3" and History[10000].message == "10002")
        "#);
    }
}

/// LogService's logging methods in a playing client (measured, Studio 0.744: tools/parity/probes/logservice_methods.luau): a
/// line of the method's MessageType with its {key} placeholders filled ({{ and }} escapes, a missing key kept), which
/// MessageOut gets after the frame's Heartbeat with a converted copy of the context as its third argument (every line gets
/// one: an empty table without a context); Error logs its line, then throws a table (message, template, context, stack) whose
/// tostring is the message, and one nothing catches logs its stack as MessageInfo lines; a message that is not a string warns
/// first. The run's output keeps a MessageInfo line as kind `info`.
#[test]
fn log_service_methods_log_with_their_context_and_error_throws_a_table() {
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Log = {}
        _G.Log = Log
        local LogService = game:GetService("LogService")
        LogService.MessageOut:Connect(function(...)
            local Message, Kind, Context = ...
            local Keys = {}
            for Key, Value in Context do
                table.insert(Keys, tostring(Key) .. "=" .. tostring(Value))
            end
            table.sort(Keys)
            table.insert(Log, Kind.Name .. "[" .. table.concat(string.split(Message, "\n"), "/") .. "]" .. select("#", ...) .. "{" .. table.concat(Keys, ";") .. "}")
        end)
        local Part = Instance.new("Part")
        Part.Name = "P"
        Part.Parent = workspace
        LogService:Info("i a={a} {{x}} }} {missing}", { a = 1 })
        LogService:Output("o {p}", { p = Part, f = print })
        LogService:Warn("w {1}", { "x" })
        LogService:Log(Enum.MessageType.MessageInfo, "l {a}", { a = 2 })
        print("p")
        local Ok, Thrown = pcall(LogService.Error, LogService, "e {a}", { a = 3 })
        print("thrown", Ok, typeof(Thrown), Thrown.message, Thrown.template, Thrown.context.a, tostring(Thrown), string.sub(Thrown.stack, 1, 11))
        print("errors", select(2, pcall(LogService.Info, LogService)), select(2, pcall(LogService.Log, LogService, 7, "x")))
        task.spawn(function()
            LogService:Error("u")
        end)
        task.spawn(function()
            LogService:Info(true)
        end)
        "##,
    );
    for _ in 0..2 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = run(&lua, &engine, r#"print(table.concat(_G.Log, ","))"#);
    assert!(output.contains(&"print: thrown false table e 3 e {a} 3 e 3 Stack Begin".to_string()), "{output:?}");
    assert!(output.contains(&"print: errors Argument 2 missing or nil Unable to cast double to MessageType".to_string()), "{output:?}");
    let heard = output.last().expect("the log").clone();
    let want = [
        "MessageInfo[i a=1 {x} } {missing}]3{a=1}",
        "MessageOutput[o P]3{f=<function>;p=Workspace.P}",
        "MessageWarning[w {1}]3{1=x}",
        "MessageInfo[l 2]3{a=2}",
        "MessageOutput[p]3{}",
        "MessageError[e 3]3{a=3}",
        "MessageError[u]3{},MessageInfo[Stack Begin]3{},MessageInfo[Script 'Test', Line 26]3{},MessageInfo[Stack End]3{}",
        "MessageWarning[argument #1 expects a string, but boolean was passed/    Stack Begin/    Script 'Test', Line 29/    Stack End]3{},MessageInfo[true]3{}",
    ];
    for part in want {
        assert!(heard.contains(part), "{part} in {heard}");
    }
    let lines: Vec<String> = engine.state.borrow().output.entries.iter().map(|e| format!("{}: {}", e.kind, e.message)).collect();
    for line in ["info: i a=1 {x} } {missing}", "print: o P", "warn: w {1}", "info: l 2", "error: e 3", "error: u", "info: Stack Begin", "info: Stack End"] {
        assert!(lines.contains(&line.to_string()), "{line} in {lines:?}");
    }
    // (the uncaught error logs no second error line of its own)
    assert_eq!(lines.iter().filter(|line| line.starts_with("error: u")).count(), 1, "{lines:?}");
}

/// A part fires the Changed events Roblox fires beside a write, whatever their own values (measured in a playing client,
/// Studio 0.744: tools/parity/probes/derived_changed.luau), and its BrickColor is a view of its Color.
#[test]
fn a_part_fires_the_events_roblox_fires_beside_a_write() {
    let (lua, engine) = start();
    // the writes in one call, the events (deferred, run at its flush) read in the next
    run(
        &lua,
        &engine,
        r##"
        _G.Logs = {}
        local function Case(Write)
            local Part = Instance.new("Part")
            Part.Anchored = true
            local Log = {}
            table.insert(_G.Logs, Log)
            Part.Changed:Connect(function(Name)
                table.insert(Log, Name)
            end)
            Write(Part)
        end
        Case(function(Part) Part.Position = Vector3.new(1, 2, 3) end)
        Case(function(Part) Part.Size = Vector3.new(2, 2.4, 2) end)
        Case(function(Part) Part.Color = Color3.fromRGB(163, 162, 166) end)
        Case(function(Part) Part.Material = Enum.Material.SmoothPlastic end)
        Case(function(Part) Part.CustomPhysicalProperties = PhysicalProperties.new(2, 0.3, 0.5) end)
        Case(function(Part) Part.Anchored = false end)
        "##,
    );
    let output = run(
        &lua,
        &engine,
        r##"
        for _, Log in _G.Logs do
            print(table.concat(Log, ","))
        end
        local Part = Instance.new("Part")
        Part.BrickColor = BrickColor.new("Bright blue")
        Part.Color = Color3.new(1, 0, 0)
        print(tostring(Part.BrickColor), (Part.Color:ToHex()))
        Part.BrickColor = BrickColor.new("Bright blue")
        print(tostring(Part.BrickColor), (Part.Color:ToHex()))
        "##,
    );
    for want in [
        "print: CFrame,Position,AssemblyCenterOfMass,Orientation,Rotation",
        "print: Size,AssemblyMass,Mass",
        "print: BrickColor,Color",
        "print: Material,CurrentPhysicalProperties",
        "print: CurrentPhysicalProperties,CustomPhysicalProperties,AssemblyMass,AssemblyCenterOfMass",
        "print: Anchored,AssemblyMass,AssemblyCenterOfMass",
        "print: Really red ff0000",
        "print: Bright blue 0d69ac",
    ] {
        assert!(output.contains(&want.to_string()), "{want}: {output:?}");
    }
}

/// Lowers the engine's limits on held slots (lua/refs.rs) to what it holds now plus `work` / `wait`.
fn limit_held(engine: &Engine, work: i32, wait: i32) {
    let refs = engine.lua.borrow();
    let gauge = refs.gauge.as_ref().expect("the VM's reference thread");
    let held = gauge.count();
    gauge.set_limits(held + work, held + wait);
}

/// The gauge finds mlua's reference thread and counts what Rust holds: values made and dropped from Rust move it.
#[test]
fn the_gauge_counts_what_rust_holds() {
    let (lua, engine) = start();
    let count = || engine.lua.borrow().gauge.as_ref().expect("the VM's reference thread").count();
    let before = count();
    let held: Vec<LuaTable> = (0..1000).map(|_| lua.create_table().unwrap()).collect();
    assert_eq!(count(), before + 1000);
    drop(held);
    assert_eq!(count(), before);
}

/// A loop that queues work without end errors in the script that queued it once the engine holds its limit (a
/// task.defer per hit in a loop that never yielded ran mlua's reference stack full, and the panic ended the run with
/// exit 101). What it queued still runs, and the other scripts go on.
#[test]
fn a_runaway_queue_is_its_scripts_error() {
    let (lua, engine) = start();
    limit_held(&engine, 20_000, 25_000);
    run(
        &lua,
        &engine,
        r#"
        local Queued, Ran = 0, 0
        task.delay(0.1, function() print("ran all", Queued > 15000, Ran == Queued) end)
        task.spawn(function()
            for Index = 1, 1000000 do
                task.defer(function() Ran += 1 end)
                Queued = Index
            end
            print("never")
        end)
        "#,
    );
    scheduler::run_until(&lua, &engine, 0.3).unwrap();
    let entries = engine.state.borrow().output.entries.clone();
    let refused: Vec<_> = entries.iter().filter(|entry| entry.kind == "error").collect();
    assert_eq!(refused.len(), 1, "{entries:?}");
    assert!(refused[0].message.starts_with("Test:6: task.defer: too many threads and values held for scripts ("), "{entries:?}");
    assert!(refused[0].trace.as_deref().is_some_and(|trace| trace.contains("Test:6")), "{entries:?}");
    let output = lines(&engine);
    assert!(output.contains(&"print: ran all true true".to_string()), "{output:?}");
    assert!(!output.contains(&"print: never".to_string()), "{output:?}");
}

/// A thread made with coroutine.create holds no slot until it waits: a loop of them that wait is refused at task.wait,
/// past the higher mark for waits (and a task.spawn past the lower mark meanwhile is refused too).
#[test]
fn a_runaway_of_waiting_coroutines_is_refused_at_its_wait() {
    let (lua, engine) = start();
    limit_held(&engine, 20_000, 25_000);
    run(
        &lua,
        &engine,
        r#"
        local Started, Done = 0, 0
        task.delay(0.2, function() print("done", Started > 20000, Done == Started - 1) end)
        task.spawn(function()
            for Index = 1, 1000000 do
                Started = Index
                coroutine.wrap(function()
                    task.wait(0.05)
                    Done += 1
                end)()
            end
        end)
        print("spawn", pcall(function() task.spawn(function() end) end))
        "#,
    );
    scheduler::run_until(&lua, &engine, 0.5).unwrap();
    let entries = engine.state.borrow().output.entries.clone();
    let refused: Vec<_> = entries.iter().filter(|entry| entry.kind == "error").collect();
    assert_eq!(refused.len(), 1, "{entries:?}");
    assert!(refused[0].message.contains("Test:8: task.wait: too many threads and values held for scripts ("), "{entries:?}");
    let output = lines(&engine);
    assert!(output.iter().any(|line| line.starts_with("print: spawn false Test:13: task.spawn: too many threads")), "{output:?}");
    assert!(output.contains(&"print: done true true".to_string()), "{output:?}");
}

/// What still runs mlua's reference stack full (a call with a million string arguments: mlua makes a slot of each before
/// the engine sees them) errors in the thread that made the call, with its traceback, and its spawner goes on.
#[test]
fn a_full_reference_stack_is_the_resumed_threads_error() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Strings = table.create(999950, "")
        for Index = 1, #Strings do
            Strings[Index] = tostring(Index)
        end
        task.spawn(function()
            task.cancel(table.unpack(Strings))
            print("never")
        end)
        print("spawner goes on")
        "#,
    );
    let entries = engine.state.borrow().output.entries.clone();
    let refused: Vec<_> = entries.iter().filter(|entry| entry.kind == "error").collect();
    assert_eq!(refused.len(), 1, "{entries:?}");
    assert_eq!(refused[0].message, "too many threads and values held for scripts (ree holds at most 1000000)");
    assert!(refused[0].trace.as_deref().is_some_and(|trace| trace.contains("Test:7")), "{entries:?}");
    let output = lines(&engine);
    assert!(output.contains(&"print: spawner goes on".to_string()), "{output:?}");
    assert!(!output.contains(&"print: never".to_string()), "{output:?}");
}

/// ReflectionService:GetClasses and GetClass as a game script gets them (measured in a playing client and a server game
/// script, Studio 0.744): the dump's classes in name order, each call new frozen entries (Name, Superclass, Serialized only when
/// true, Subclasses only when there are some, those a script cannot get included, Display, Permits); the classes only rbx-dom
/// describes are nil; the filter's IsA and ExcludeDisplay, and its measured errors. Permits holds no SecurityCapabilities in ree:
/// reading it reports itself as not emulated.
#[test]
fn reflection_service_lists_the_dumps_classes_as_a_game_script_gets_them() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local RS = game:GetService("ReflectionService")
        local List = RS:GetClasses()
        local Previous = ""
        for _, Entry in List do
            assert(Entry.Name > Previous and table.isfrozen(Entry), Entry.Name)
            Previous = Entry.Name
        end
        print("list", #List > 600, table.isfrozen(List), RS:GetClasses() ~= List, RS:GetClasses()[1] ~= List[1], List[1].Name)
        local Part = RS:GetClass("Part")
        print("Part", Part.Name, Part.Superclass, Part.Serialized, table.concat(Part.Subclasses, ","), Part.Display.Category, Part.Display.DeprecationMessage)
        print("frozen", table.isfrozen(Part), table.isfrozen(Part.Subclasses), table.isfrozen(Part.Display), table.isfrozen(Part.Permits))
        local Accessory = RS:GetClass("Accessory")
        print("Accessory", Accessory.Subclasses, Accessory.Serialized, Accessory.Superclass)
        local Object = RS:GetClass("Object")
        print("Object", Object.Superclass, Object.Serialized, #Object.Subclasses > 0)
        print("BasePlayerGui", table.concat(RS:GetClass("BasePlayerGui").Subclasses, ","))
        print("Glue", RS:GetClass("Glue").Display.DeprecationMessage, RS:GetClass("LogService").Serialized)
        print("nil", RS:GetClass("Player"), RS:GetClass("CoreGui"), RS:GetClass("UserGameSettings"), RS:GetClass("part"), RS:GetClass("Nope"), RS:GetClass(5))
        print("IsA", #RS:GetClasses({ IsA = "BasePart" }), #RS:GetClasses({ IsA = "Nope" }), #RS:GetClasses({ IsA = "basepart" }), #RS:GetClasses({ IsA = "Player" }), #RS:GetClasses({ IsA = "Object" }) == #List)
        local NoDisplay = RS:GetClasses({ ExcludeDisplay = true, Unknown = 1 })
        print("ExcludeDisplay", #NoDisplay == #List, NoDisplay[1].Display, RS:GetClasses({ ExcludeDisplay = false })[1].Display ~= nil)
        print("GetClass filtered", RS:GetClass("Part", { IsA = "BasePart" }).Name, RS:GetClass("Part", { IsA = "GuiObject" }), RS:GetClass("Part", { ExcludeDisplay = true }).Display)
        print("errors", select(2, pcall(RS.GetClasses, RS, 5)))
        print("errors", select(2, pcall(RS.GetClasses, RS, { "BasePart" })))
        print("errors", select(2, pcall(RS.GetClasses, RS, { 1, IsA = "BasePart" })))
        print("errors", select(2, pcall(RS.GetClasses, RS, { IsA = 5 })))
        print("errors", select(2, pcall(RS.GetClasses, RS, { ExcludeDisplay = 1 })))
        print("errors", select(2, pcall(RS.GetClasses, RS, { Security = "x" })))
        print("errors", select(2, pcall(RS.GetClass, RS)))
        print("errors", select(2, pcall(RS.GetClass, RS, "Part", true)))
        print("errors", (string.gsub(select(2, pcall(function() List[1] = 1 end)), "^Test:%d+: ", "")))
        print("errors", select(2, pcall(RS.GetClasses)))
        for _ in Part.Permits do
            print("never")
        end
        print("Permits", typeof(Part.Permits), Part.Permits.New, next(Part.Permits))
        "#,
    );
    let expected = [
        "print: list true true true true Accessory",
        "print: Part Part FormFactorPart true FlagStand,Platform,Seat,SkateboardPlatform,SpawnLocation General nil",
        "print: frozen true true true true",
        "print: Accessory nil true Accoutrement",
        "print: Object nil nil true",
        "print: BasePlayerGui CoreGui,PlayerGui,StarterGui",
        "print: Glue Use `WeldConstraint` instead. nil",
        "print: nil nil nil nil nil nil nil",
        "print: IsA 19 0 0 0 true",
        "print: ExcludeDisplay true nil true",
        "print: GetClass filtered Part nil nil",
        "print: errors Unable to cast double to Dictionary",
        "print: errors Unable to cast Array to Dictionary",
        "print: errors Unable to cast Array to Dictionary",
        "print: errors ReflectionClassFilter's IsA field is expected to be type string",
        "print: errors ReflectionClassFilter's ExcludeDisplay field is expected to be type bool",
        "print: errors ReflectionClassFilter's Security field is expected to be type SecurityCapabilities",
        "print: errors Argument 2 missing or nil",
        "print: errors Unable to cast bool to Dictionary",
        "print: errors attempt to modify a readonly table",
        "print: errors Expected ':' not '.' calling member function GetClasses",
        // (the first use of an unemulated member writes its line, before the print it is an argument of)
        "missing: ReflectedClass.Permits is not emulated",
        "print: Permits table nil nil",
    ];
    assert_eq!(output, expected);
    let missing = engine.state.borrow().output.missing.clone();
    assert_eq!(missing, vec![("ReflectedClass.Permits".to_string(), 2)], "{missing:?}");
    // the dump's classes and no others; IsA counts a class and its subclasses
    let reflected = engine.reflection.classes.iter().filter(|class| class.reflected).count();
    assert_eq!(reflected, 660);
}

/// A vide-like module that builds its element names from ReflectionService:GetClasses() when it is required (stock vide's
/// jsx.luau maps every lower-cased class name to the class name) loads, and makes instances through that map.
#[test]
fn a_module_that_maps_every_class_name_at_require_loads() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r##"
        local Module = Instance.new("ModuleScript")
        Module.Name = "jsx"
        __setSource(Module, [[
local tags = {}
if game then
	for _, class in game:GetService("ReflectionService"):GetClasses() do
		tags[string.lower(class.Name)] = class.Name
	end
end
local function jsx(tag, props, ...)
	props = props or {}
	tag = tags[tag] or tag
	local instance = Instance.new(tag)
	for key, value in props do
		instance[key] = value
	end
	for index = 1, select("#", ...) do
		select(index, ...).Parent = instance
	end
	return instance
end
return jsx
]])
        local jsx = require(Module)
        local Root = jsx("frame", { Name = "Root" }, jsx("textlabel", { Text = "hi" }), jsx("uicorner", {}))
        local Label = Root:FindFirstChildOfClass("TextLabel")
        print("jsx", Root.ClassName, Root.Name, Label.ClassName, Label.Text, Root:FindFirstChildOfClass("UICorner") ~= nil)
        "##,
    );
    assert_eq!(output, vec!["print: jsx Frame Root TextLabel hi true".to_string()]);
}

#[test]
fn an_executable_belongs_to_the_nearest_checkout_above_it_wherever_it_runs() {
    let root = std::env::temp_dir().join(format!("ree-checkout-test-{}", std::process::id()));
    let repo = root.join("ree");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("Cargo.lock"), "").unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"roblox-engine-emulator\"\n").unwrap();
    for exe in ["target/release/ree.exe", "target/release/deps/ree-0a1b.exe", ".cache/bin/ree-0.1.0-1-2.exe"] {
        assert_eq!(super::checkout_of(&repo.join(exe)), Some(repo.clone()), "{exe}");
    }
    let verbatim = std::path::PathBuf::from(format!(r"\\?\{}", repo.join("target").join("release").join("ree.exe").display()));
    if cfg!(windows) {
        assert_eq!(super::checkout_of(&verbatim), Some(repo.clone()), "a service's --source is canonical");
    }
    // a service copy in a state dir outside any checkout has none: serve takes its starter's (--source)
    assert_eq!(super::checkout_of(&root.join("state/bin/ree-0.1.0-1-2.exe")), None);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn checkout_of_still_finds_a_hosts_nearest_lockfile() {
    let root = std::env::temp_dir().join(format!("ree-host-lockfile-test-{}", std::process::id()));
    let host = root.join("host");
    std::fs::create_dir_all(&host).unwrap();
    std::fs::write(root.join("Cargo.lock"), "").unwrap();
    std::fs::write(host.join("Cargo.lock"), "").unwrap();
    let exe = host.join("target/release/host.exe");
    assert_eq!(super::checkout_of(&exe), Some(host.clone()), "a host needs only its lockfile");
    std::fs::write(host.join("Cargo.toml"), "[package]\nname = \"another-tool\"\n").unwrap();
    assert_eq!(super::checkout_of(&exe), Some(host));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_unrelated_cargo_project_is_not_the_executables_checkout() {
    let root = std::env::temp_dir().join(format!("ree-foreign-checkout-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let exe = root.join("bin/ree.exe");
    let manifest = root.join("Cargo.toml");
    std::fs::write(root.join("Cargo.lock"), "").unwrap();
    assert_eq!(super::ree_checkout_of(&exe), None, "a lockfile without a manifest");
    // a host that depends on ree names it under [dependencies], outside [package]
    std::fs::write(&manifest, "[package]\nname = \"another-tool\"\n[dependencies]\nroblox-engine-emulator = \"0.1.0\"\n").unwrap();
    assert_eq!(super::ree_checkout_of(&exe), None, "a host's manifest does not name this package");
    // a literal string and a trailing comment still name the package
    std::fs::write(&manifest, "[package]\nname = 'roblox-engine-emulator' # this package\n").unwrap();
    assert_eq!(super::ree_checkout_of(&exe), Some(root.clone()));
    // the search goes on past a lockfile that has no ree manifest beside it
    let nested = root.join("target/nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("Cargo.lock"), "").unwrap();
    assert_eq!(super::ree_checkout_of(&nested.join("bin/ree.exe")), Some(root.clone()));
    std::fs::remove_dir_all(&root).unwrap();
}

/// Standalone state uses the platform's per-user cache, unless the environment names one.
#[test]
fn standalone_caches_are_per_user() {
    let root = std::env::temp_dir().join("ree-cache-defaults");
    let path = |name: &str| root.join(name);
    // a made-up environment: nothing here reads or creates the real per-user folders
    let environment = [("LOCALAPPDATA", path("local")), ("HOME", path("home")), ("XDG_CACHE_HOME", path("xdg"))];
    let var = |name: &str| environment.iter().find(|(key, _)| *key == name).map(|(_, value)| value.as_os_str().to_owned());
    let with = |set: &'static str, value: &'static str| move |name: &str| if name == set { Some(path(value).into_os_string()) } else { var(name) };

    // a standalone run: each platform's own cache folder
    assert_eq!(super::standalone_state_dir("windows", var).unwrap(), path("local/ree"));
    assert_eq!(super::standalone_state_dir("linux", var).unwrap(), path("xdg/ree"));
    assert_eq!(super::standalone_state_dir("macos", var).unwrap(), path("home/Library/Caches/ree"));
    // Linux falls back to ~/.cache when XDG_CACHE_HOME is unset or relative
    let no_xdg = |name: &str| if name == "XDG_CACHE_HOME" { None } else { var(name) };
    assert_eq!(super::standalone_state_dir("linux", no_xdg).unwrap(), path("home/.cache/ree"));
    let relative_xdg = |name: &str| if name == "XDG_CACHE_HOME" { Some("relative".into()) } else { var(name) };
    assert_eq!(super::standalone_state_dir("linux", relative_xdg).unwrap(), path("home/.cache/ree"));

    // explicit overrides take priority over the per-user cache
    for key in ["RUR_STATE_DIR", "RUR_CACHE_DIR"] {
        assert_eq!(super::standalone_state_dir("linux", with(key, "override")).unwrap(), path("override"));
    }
    // RUR_STATE_DIR comes before RUR_CACHE_DIR
    let both = |name: &str| match name {
        "RUR_STATE_DIR" => Some(path("first").into_os_string()),
        "RUR_CACHE_DIR" => Some(path("second").into_os_string()),
        _ => var(name),
    };
    assert_eq!(super::standalone_state_dir("linux", both).unwrap(), path("first"));
}

#[test]
fn standalone_state_stays_selected_after_environment_normalization() {
    use crate::launch::resolve_path_variable;
    use std::ffi::OsString;
    use std::sync::OnceLock;
    let root = std::path::absolute("cache-test").unwrap();
    for os in ["windows", "linux", "macos"] {
        // absolute HOME and LOCALAPPDATA, plus a relative XDG_CACHE_HOME that the cache lookup ignores
        let client_environment = |name: &str| match name {
            "LOCALAPPDATA" => Some(root.join("local").into_os_string()),
            "HOME" => Some(root.join("home").into_os_string()),
            "XDG_CACHE_HOME" => Some(OsString::from("relative")),
            _ => None,
        };
        let expected = super::standalone_state_dir(os, client_environment).unwrap();

        // the client's first lookup pins the state dir, before a service resolves its paths or changes folder
        let state = OnceLock::new();
        assert_eq!(super::resolve_standalone_state(&state, os, client_environment).unwrap(), expected);

        // what a service or worker gets: the ignored relative folder is absolute now, so a fresh Linux lookup would pick it as the cache
        let forwarded_environment = |name: &str| client_environment(name).and_then(resolve_path_variable);
        assert_eq!(forwarded_environment("XDG_CACHE_HOME"), Some(std::path::absolute("relative").unwrap().into_os_string()));
        assert_eq!(super::resolve_standalone_state(&state, os, forwarded_environment).unwrap(), expected, "{os}: forwarding paths must not change the pinned cache");

        // a later job does not consult even a completely different environment
        let changed_environment = |_: &str| Some(root.join("changed").into_os_string());
        assert_eq!(super::resolve_standalone_state(&state, os, changed_environment).unwrap(), expected);

        // with only relative folders there is no cache, so the lookup fails and pins nothing
        let relative_environment = |name: &str| if matches!(name, "HOME" | "LOCALAPPDATA") { Some(OsString::from("relative")) } else { None };
        let unpinned = OnceLock::new();
        assert!(super::resolve_standalone_state(&unpinned, os, relative_environment).is_err(), "{os}: relative per-user folders stay unset");
        assert!(unpinned.get().is_none());
    }
}

/// The VM's work counts so far (a copy: the counters are plain numbers)
fn vm_counts(engine: &Engine) -> crate::cost::VmCounters {
    let pointer = engine.state.borrow().sched.vm_counters.expect("VM counters");
    unsafe { std::ptr::read(pointer) }
}

/// The first number of the first `print` line `run` collected
fn printed_number(output: &[String]) -> f64 {
    let line = output.iter().find_map(|line| line.strip_prefix("print: ")).unwrap_or_else(|| panic!("a result: {output:?}"));
    line.split(' ').next().unwrap().parse().unwrap_or_else(|_| panic!("a number: {line}"))
}

/// A script's environment is safe, as Roblox makes every script's (the parity bench, tools/cost-parity: two imports a loop step
/// 4.6 ns in a module, 21.4 in one that called getfenv first; `typeof(x)` 4.7 and 25.9): its imports are resolved when it loads,
/// `typeof` is the VM's fast call, which names ree's values as ree's typeof does (a constant Vector3, an Instance, a proxy datatype,
/// SharedTable), and getfenv ends both for its environment. ree loaded each script on a thread whose globals were not its
/// environment, so every import was looked up at each run, and its compiler kept typeof off the fast path (a C call, 23 ns).
#[test]
fn a_scripts_environment_resolves_imports_at_load_and_takes_fast_calls() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let source = |prefix: &str| {
        format!(
            "{prefix}local M = {{}}\nfunction M.Run(N) local s = 0 for i = 1, N do local g, v = game, Vector3.new if typeof(i) == \"number\" and g and v then s += 1 end end return s end\n\
             function M.Names() return typeof(Vector3.new(1, 2, 3)), typeof(workspace), typeof(RaycastParams.new()), typeof(SharedTable.new()), typeof({{}}), typeof(newproxy(true)) end\nreturn M\n"
        )
    };
    run(
        &lua,
        &engine,
        &format!(
            r#"
            local Folder = Instance.new("Folder")
            local Safe, Unsafe = Instance.new("ModuleScript"), Instance.new("ModuleScript")
            Safe.Parent, Unsafe.Parent = Folder, Folder
            __setSource(Safe, {safe:?})
            __setSource(Unsafe, {unsafe:?})
            _G.Safe, _G.Unsafe = require(Safe), require(Unsafe)
            "#,
            safe = source(""),
            unsafe = source("local _ = getfenv()\n")
        ),
    );
    let sample = |module: &str| {
        let before = vm_counts(&engine);
        run(&lua, &engine, &format!("_G.{module}.Run(1000)"));
        let after = vm_counts(&engine);
        (after.ops[21] - before.ops[21], after.ops[73] - before.ops[73], after.slowimports - before.slowimports)
    };
    // (CALL is opcode 21, FASTCALL1 73; the snippet that calls Run makes a call of its own)
    let (calls, fast, slow) = sample("Safe");
    assert!(calls < 10 && fast >= 1000 && slow == 0, "a safe environment: {calls} calls, {fast} fast calls, {slow} imports looked up");
    let (calls, slow) = {
        let (calls, _, slow) = sample("Unsafe");
        (calls, slow)
    };
    assert!(calls >= 1000 && slow >= 3000, "after getfenv: {calls} calls, {slow} imports looked up");
    let output = run(&lua, &engine, "print(_G.Safe.Names()) print(_G.Unsafe.Names())");
    let names = "print: Vector3 Instance RaycastParams SharedTable table userdata";
    assert_eq!(output.iter().filter(|line| line.as_str() == names).count(), 2, "{output:?}");
}

/// `math.isnan`, `math.isinf` and `math.isfinite` (Luau's since 0.701) are builtins a script calls as fast
/// calls, as a client does (the parity bench's `arith.isnan` rows: a loop step calling one 4.9-6.2 ns in a module, 20-22 after
/// getfenv), and getfenv makes them plain C calls; the answers are the same either way.
#[test]
fn a_scripts_number_classifiers_are_fast_calls() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let source = |prefix: &str| {
        format!(
            "{prefix}return function(N) local s = 0 for i = 1, N do local x = i * 0.5 if math.isnan(x) then s += 1 end if math.isinf(x) then s += 2 end if math.isfinite(x) then s += 4 end end return s end\n"
        )
    };
    run(
        &lua,
        &engine,
        &format!(
            r#"
            local Folder = Instance.new("Folder")
            local Safe, Unsafe = Instance.new("ModuleScript"), Instance.new("ModuleScript")
            Safe.Parent, Unsafe.Parent = Folder, Folder
            __setSource(Safe, {safe:?})
            __setSource(Unsafe, {unsafe:?})
            _G.Safe, _G.Unsafe = require(Safe), require(Unsafe)
            "#,
            safe = source(""),
            unsafe = source("local _ = getfenv()\n")
        ),
    );
    let sample = |module: &str| {
        let before = vm_counts(&engine);
        let output = run(&lua, &engine, &format!("print(_G.{module}(1000))"));
        let after = vm_counts(&engine);
        (output, after.ops[21] - before.ops[21], after.ops[73] - before.ops[73], after.ccalls - before.ccalls)
    };
    // (CALL is opcode 21, FASTCALL1 73; the snippet that calls the module makes a call or two of its own, and print one C call)
    let (output, calls, fast, ccalls) = sample("Safe");
    assert!(output.contains(&"print: 4000".to_string()), "{output:?}");
    assert!(calls < 10 && fast >= 3000 && ccalls < 10, "a safe environment: {calls} calls, {fast} fast calls, {ccalls} C calls");
    let (output, calls, _, ccalls) = sample("Unsafe");
    assert!(output.contains(&"print: 4000".to_string()), "{output:?}");
    assert!(calls >= 3000 && ccalls >= 3000, "after getfenv: {calls} calls, {ccalls} C calls");
}

/// Luau 0.747's compiler makes every pcall / xpcall a fast protected call (FASTPCALL, which skips the import and the C call of the
/// fallback it carries); a 0.747 client still pays what that call cost (the parity bench's lang.call.pcall: 18.3 ns, 17.9 in a 0.744
/// client that made the call), so the call is counted as the C call it stands for: one a pcall, as before.
#[test]
fn a_fast_protected_call_counts_the_c_call_it_stands_for() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"
            local Module = Instance.new("ModuleScript")
            __setSource(Module, {source:?})
            _G.Protected = require(Module)
            "#,
            source = "local function f(x) return x end
return function(N) local s = 0 for i = 1, N do local ok, v = pcall(f, i) if ok then s += v end local ok2 = xpcall(f, print, i) if ok2 then s += 1 end end return s end
"
        ),
    );
    let before = vm_counts(&engine);
    let output = run(&lua, &engine, "print(_G.Protected(1000))");
    let after = vm_counts(&engine);
    assert!(output.contains(&"print: 501500".to_string()), "{output:?}");
    // (FASTPCALL is opcode 89, CALL 21 and CALLFB 87: the fast path skips the fallback's call; the snippet that calls the module and
    // print make a few calls of their own)
    let (fast, calls, ccalls) = (after.ops[89] - before.ops[89], after.ops[21] + after.ops[87] - before.ops[21] - before.ops[87], after.ccalls - before.ccalls);
    assert!(fast >= 2000 && calls < 10, "{fast} fast protected calls, {calls} calls: the fast path ran");
    assert!((2000..2010).contains(&ccalls), "{ccalls} C calls for 2,000 protected calls");
}

/// A numeric loop the compiler unrolls starts at its start as written, as the interpreter's own loop does: `for i = -0, 1` reads -0
/// first in a 0.748 client and server (tools/parity/probes/loop_start.luau), and a live 0.748 client serves
/// FFlagLuauCompileLoopUnrollNegativeZero on (data/luau-flags.json), which keeps an unrolled first index as written where it was once
/// `-0 + 0 * 1`, which is 0 (the engine compiles at -O2, which unrolls such loops). A start the compiler cannot see (`tonumber`) is
/// never unrolled and reads -0 either way.
#[test]
fn an_unrolled_loop_starts_at_its_start_as_written() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        for i = -0, 1 do print("unrolled", 1 / i) end
        local start = -0
        for i = start, 1 do print("constant local", 1 / i) end
        for i = tonumber("-0"), 1 do print("run time", 1 / i) end
        "#,
    );
    let expected: Vec<String> = ["unrolled", "constant local", "run time"]
        .iter()
        .flat_map(|case| [format!("print: {case} -inf"), format!("print: {case} 1")])
        .collect();
    assert_eq!(output, expected);
}

/// A `--!native` module's instructions are priced as a client's machine code takes them (cost.rs NATIVE_OPS: a keyed table read or
/// write 0.42 ns, an addition or loop step 0.43; interpreted 2.45 and 0.48), whether or not this VM compiles the module: the parity
/// bench's native rows run 0.8-7.8x faster than the same loops interpreted.
#[test]
fn a_native_modules_instructions_cost_what_native_code_takes() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let source = |prefix: &str| {
        format!(
            "{prefix}return function(N) local t = {{ x = 0 }} local t0 = os.clock() for i = 1, N do t.x = t.x + i end return (os.clock() - t0) / N * 1e9 end\n"
        )
    };
    let output = run(
        &lua,
        &engine,
        &format!(
            r#"
            local Interpreted, Native = Instance.new("ModuleScript"), Instance.new("ModuleScript")
            __setSource(Interpreted, {interpreted:?})
            __setSource(Native, {native:?})
            print(string.format("%.2f %.2f", require(Interpreted)(10000), require(Native)(10000)))
            "#,
            interpreted = source(""),
            native = source("--!native\n")
        ),
    );
    let line = output.iter().find_map(|line| line.strip_prefix("print: ")).unwrap_or_else(|| panic!("{output:?}"));
    let (interpreted, native): (f64, f64) = {
        let mut numbers = line.split(' ').map(|n| n.parse::<f64>().unwrap());
        (numbers.next().unwrap(), numbers.next().unwrap())
    };
    assert!((5.0..7.0).contains(&interpreted), "interpreted: {interpreted} ns a step");
    assert!((1.3..2.2).contains(&native), "native: {native} ns a step");
}

/// A ray pays for the parts its path crosses, whatever its filter admits, a bounds query for the parts it returns, and GetPartsInPart
/// for the parts it returns and the size of the part it is given (cost.rs RAY_MET, RAY_CROSSED, QUERY_BOX_PART, QUERY_PART_EXACT,
/// QUERY_PART_SIZE: the parity bench's rays and queries, on geometry both sides find the same parts in, beyond the Luau around the
/// call: a miss with an Include filter 0.27 us, a ray through 24 stacked parts 1.50, an empty box 0.15, a box of 10 parts 1.54, a
/// 2-stud part touching nothing 0.27 and a 40-stud one 1.26, the 23 others of a stack 17.3; a headless 0.747 client's miss with an
/// Include filter of one part 0.31 to 0.33, with none 0.19 to 0.23: the filter's list, PARAMS_LIST).
#[test]
fn a_spatial_query_pays_for_the_parts_it_meets() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local At = Vector3.new(1000, 500, 0)
        local Stack = {}
        for Index = 1, 24 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Size = Vector3.new(2, 2, 2)
            Part.CFrame = CFrame.new(At)
            Part.Parent = workspace
            Stack[Index] = Part
        end
        local function Per(Body)
            local N = 200
            local start = os.clock()
            for _ = 1, N do Body() end
            return (os.clock() - start) / N * 1e9
        end
        local Down, Far = Vector3.new(0, -20, 0), Vector3.new(5000, 5000, 5000)
        local Include = RaycastParams.new()
        Include.FilterType = Enum.RaycastFilterType.Include
        Include.FilterDescendantsInstances = { Stack[1] }
        -- (an untimed query takes the new parts into the spatial index first: cost.rs QUERY_TAKEN_PART)
        workspace:Raycast(Far, Down)
        local Miss = Per(function() return workspace:Raycast(Far, Down, Include) end)
        local Through = Per(function() return workspace:Raycast(At + Vector3.new(0, 10, 0), Down) end)
        local Filtered = Per(function() return workspace:Raycast(At + Vector3.new(0, 10, 0), Down, Include) end)
        local Empty = Per(function() return workspace:GetPartBoundsInBox(CFrame.new(Far), Vector3.new(4, 4, 4)) end)
        local Found = Per(function() return workspace:GetPartBoundsInBox(CFrame.new(At), Vector3.new(4, 4, 4)) end)
        local function Lone(Side, Z)
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Size = Vector3.new(Side, Side, Side)
            Part.CFrame = CFrame.new(At + Vector3.new(0, 0, Z))
            Part.Parent = workspace
            return Part
        end
        local Small, Large = Lone(2, 200), Lone(40, 500)
        workspace:Raycast(Far, Down)
        local Alone = Per(function() return workspace:GetPartsInPart(Small) end)
        local Wide = Per(function() return workspace:GetPartsInPart(Large) end)
        local Inside = Per(function() return workspace:GetPartsInPart(Stack[1]) end)
        print(string.format("%.0f %.0f %.0f %.0f %.0f %.0f %.0f %.0f", Miss, Through, Filtered, Empty, Found, Alone, Wide, Inside))
        "#,
    );
    let line = output.iter().find_map(|line| line.strip_prefix("print: ")).unwrap_or_else(|| panic!("{output:?}"));
    let numbers: Vec<f64> = line.split(' ').map(|n| n.parse().unwrap()).collect();
    // a call 0.19 us and the Include filter's list 0.108 (cost.rs PARAMS_LIST, PARAMS_LISTED); a path through 24 parts 0.175 + 23 x
    // 0.042 more, and the filter's list again; a box 0.15, and 0.115 a part it returns and 25 ns the one cell holding the 24 (the
    // grid's prices, cost.rs QUERY_BOX_PART, QUERY_CELL: the stack cost a client 3.80 us, stacked parts dearer than spread ones,
    // not modelled); GetPartsInPart 0.243 + 0.011 a stud of the root of its part's surface area, and 0.74 a part it returns
    let list = (crate::cost::PARAMS_LIST + crate::cost::PARAMS_LISTED) * 1e9;
    assert!((250.0..360.0).contains(&numbers[0]), "a miss: {line}");
    assert!((1300.0..1650.0).contains(&numbers[1]) && (numbers[2] - numbers[1] - list).abs() < 100.0, "through the stack, filtered or not: {line}");
    assert!((140.0..220.0).contains(&numbers[3]), "an empty box: {line}");
    assert!((2800.0..3300.0).contains(&numbers[4]), "a box of 24 parts: {line}");
    assert!((270.0..360.0).contains(&numbers[5]), "a 2-stud part touching nothing: {line}");
    assert!((1200.0..1500.0).contains(&numbers[6]), "a 40-stud part touching nothing: {line}");
    assert!((12500.0..15500.0).contains(&numbers[7]), "a part among 23 others: {line}");
}

/// A cast stops at its first hit, as a playing client's does (roblox-sim, Studio 0.747, a headless client: 24 walls 2 studs apart
/// behind a Raycast's hit 0.47 us against 0.42 for the wall alone, a Spherecast's 0.56 against 0.48, a Blockcast's 0.73 against 0.66,
/// a Shapecast's alike): the parts behind it cost nothing, filtered out or not, but those a Raycast's walk tests in the grid cell
/// of its hit (cost.rs RAY_TESTED). The parts before it that its filter leaves out cost
/// what a ray's do whatever the cast's shape (23 excluded walls in front 0.027 to 0.031 us each for every kind), and the parts tied
/// with the hit the cast's own price (cost.rs RAY_CROSSED, SPHERE_CROSSED, BLOCK_CROSSED; the stack of a_spatial_query_pays_for_the_parts_it_meets).
#[test]
fn a_cast_pays_for_the_parts_up_to_its_hit() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local O = Vector3.new(300, 100, 300)
        local From, Dir = O + Vector3.new(0, 0, 50), Vector3.new(0, 0, -400)
        local Scene = Instance.new("Folder")
        Scene.Parent = workspace
        local Front = Instance.new("Folder")
        local function Wall(Z, Parent)
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Size = Vector3.new(40, 40, 0.5)
            Part.CFrame = CFrame.new(O + Vector3.new(0, 0, Z))
            Part.Parent = Parent
            return Part
        end
        local Box = Instance.new("Part")
        Box.Anchored = true
        Box.Size = Vector3.new(2, 2, 2)
        Box.CFrame = CFrame.new(From)
        local Casts = {
            Raycast = function(Params) return workspace:Raycast(From, Dir, Params) end,
            Spherecast = function(Params) return workspace:Spherecast(From, 1, Dir, Params) end,
            Blockcast = function(Params) return workspace:Blockcast(CFrame.new(From), Vector3.new(2, 2, 2), Dir, Params) end,
            Shapecast = function(Params) return workspace:Shapecast(Box, Dir, Params) end,
        }
        -- every scene excludes Front (empty but in the excluded walls' scenes), so each pays for the same filter
        local function Build(Kind)
            Front.Parent = nil
            Front:ClearAllChildren()
            Scene:ClearAllChildren()
            if Kind == "one" then
                Wall(0, Scene)
            elseif Kind == "behind" then
                for I = 1, 24 do Wall(-2 * (I - 1), Scene) end
            elseif Kind == "excluded_behind" then
                Wall(0, Scene)
                for I = 2, 24 do Wall(-2 * (I - 1), Front) end
            elseif Kind == "excluded_in_front" then
                for I = 1, 23 do Wall(2 * I, Front) end
                Wall(0, Scene)
            elseif Kind == "tied" then
                for _ = 1, 24 do Wall(0, Scene) end
            end
            Front.Parent = Scene
            local Params = RaycastParams.new()
            Params.FilterType = Enum.RaycastFilterType.Exclude
            Params.FilterDescendantsInstances = { Front }
            return Params
        end
        local function Per(Cast, Params)
            local N = 100
            local Start = os.clock()
            for _ = 1, N do Cast(Params) end
            return (os.clock() - Start) / N * 1e9
        end
        -- (a first pass for the frame's first calls, which cost more)
        local Params = Build("one")
        for _, Cast in Casts do Per(Cast, Params) end
        for _, Kind in { "one", "behind", "excluded_behind", "excluded_in_front", "tied" } do
            Params = Build(Kind)
            for Name, Cast in Casts do
                local Hit = Cast(Params)
                assert(Hit and Hit.Instance.Parent == Scene and math.abs(Hit.Position.Z - (O.Z + 0.25)) < 1e-3, `{Name} {Kind}: {Hit and Hit.Position}`)
                print(Name, Kind, string.format("%.1f", Per(Cast, Params)))
            end
        end
        "#,
    );
    let mut ns = std::collections::HashMap::new();
    for line in output.iter().filter_map(|line| line.strip_prefix("print: ")) {
        let parts: Vec<&str> = line.split(' ').collect();
        ns.insert((parts[0].to_string(), parts[1].to_string()), parts[2].parse::<f64>().unwrap());
    }
    let at = |cast: &str, kind: &str| *ns.get(&(cast.to_string(), kind.to_string())).unwrap_or_else(|| panic!("{cast} {kind}: {output:?}"));
    for (cast, each) in [("Raycast", crate::cost::RAY_CROSSED), ("Spherecast", crate::cost::SPHERE_CROSSED), ("Blockcast", crate::cost::BLOCK_CROSSED), ("Shapecast", crate::cost::BLOCK_CROSSED)] {
        let one = at(cast, "one");
        // (a Raycast's walk tests the 6 walls behind its hit in the hit's own grid cell, of 16 studs, as a client's does: cost.rs
        // RAY_TESTED; the client's 23 walls behind cost it 0.05 us)
        let behind = if cast == "Raycast" { 6.0 * crate::cost::RAY_TESTED * 1e9 } else { 0.0 };
        assert!((at(cast, "behind") - one - behind).abs() < 0.5, "{cast}: 23 walls behind the hit cost {} ns", at(cast, "behind") - one);
        assert!((at(cast, "excluded_behind") - one - behind).abs() < 0.5, "{cast}: 23 excluded walls behind the hit cost {} ns", at(cast, "excluded_behind") - one);
        let front = at(cast, "excluded_in_front") - one;
        // (each excluded wall's filter walk one step up to the excluded folder, at its depth's price: cost.rs filter_step)
        let excluded = 23.0 * (crate::cost::RAY_CROSSED + crate::cost::filter_step(4)) * 1e9;
        assert!((front - excluded).abs() < 2.0, "{cast}: 23 excluded walls in front of the hit cost {front} ns, not {excluded}");
        // (and each tied wall's filter walk, the list's one entry, to the top: 3 steps)
        let tied = at(cast, "tied") - one;
        let tied_price = 23.0 * (each + 3.0 * crate::cost::filter_step(3)) * 1e9;
        assert!((tied - tied_price).abs() < 2.0, "{cast}: 23 walls tied with the hit cost {tied} ns, not {tied_price}");
    }
}

/// `--device phone` (Options::device) emulates a touch device and scales the script work the clock charges by a sim-to-phone
/// calibration of script time (cost.rs PHONE_SCRIPTS, 2.77): a conversion of the desktop prices, not a measurement on a phone.
#[test]
fn a_phone_run_pays_the_calibrated_factor_on_its_script_work() {
    let per = |device: &str| -> (f64, String) {
        let (lua, engine) = start_with(Options { device: device.into(), ..Options::default() });
        engine_lua::modules::interpret_all(&lua, &engine);
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let output = run(
            &lua,
            &engine,
            r#"
            local start = os.clock()
            for _ = 1, 1000 do Instance.new("Folder") end
            print(string.format("%.1f %s", (os.clock() - start) / 1000 * 1e9, tostring(game:GetService("UserInputService").TouchEnabled)))
            "#,
        );
        let line = output.iter().find_map(|line| line.strip_prefix("print: ")).unwrap_or_else(|| panic!("{output:?}")).to_string();
        (printed_number(&output), line.split(' ').nth(1).unwrap().to_string())
    };
    let (desktop, desktop_touch) = per("desktop");
    let (phone, phone_touch) = per("phone");
    assert!((phone / desktop - crate::cost::PHONE_SCRIPTS).abs() < 0.02, "a phone {phone} ns a Folder, a desktop {desktop}");
    assert_eq!((desktop_touch.as_str(), phone_touch.as_str()), ("false", "true"));
}

/// `RemoveTag` is priced from its recorded client row (`method.CollectionService.RemoveTag` = 170.1 ns whole,
/// 155 ns beyond the Luau call), not `AddTag`'s 390 ns: so `RemoveTag` reads ~170 ns and ranks strictly below `AddTag`
/// and below `AddTag + RemoveTag` (fixing the 2.38x overprice and broken ranking).
#[test]
fn remove_tag_is_priced_from_its_recorded_row() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local CS = game:GetService("CollectionService")
        local folders = table.create(400)
        for i = 1, 400 do
            folders[i] = Instance.new("Folder")
        end
        -- Warm first-use cold path for AddTag and RemoveTag once
        CS:AddTag(folders[1], "Warm")
        CS:RemoveTag(folders[1], "Warm")
        local t0 = os.clock()
        for i = 1, 400 do
            CS:AddTag(folders[i], "BenchTag")
        end
        local add_ns = (os.clock() - t0) / 400 * 1e9
        local t1 = os.clock()
        for i = 1, 400 do
            CS:RemoveTag(folders[i], "BenchTag")
        end
        local remove_ns = (os.clock() - t1) / 400 * 1e9
        local p = Instance.new("Part")
        p.Parent = workspace
        p:AddTag("ToggleWarm")
        p:RemoveTag("ToggleWarm")
        local t2 = os.clock()
        for _ = 1, 200 do
            p:AddTag("Toggle")
            p:RemoveTag("Toggle")
        end
        local pair_ns = (os.clock() - t2) / 400 * 1e9
        print(string.format("%.2f %.2f %.2f", add_ns, remove_ns, pair_ns))
        "#,
    );
    let line = output.iter().find_map(|line| line.strip_prefix("print: ")).unwrap_or_else(|| panic!("{output:?}"));
    let vals: Vec<f64> = line.split(' ').map(|n| n.parse().unwrap()).collect();
    let (add_ns, remove_ns, pair_ns) = (vals[0], vals[1], vals[2]);
    assert!((remove_ns - 170.0).abs() < 10.0, "RemoveTag expected ~170 ns, got {remove_ns} ({line})");
    assert!(((add_ns - remove_ns) - 235.0).abs() < 5.0, "AddTag - RemoveTag expected 235 ns, got {} ({line})", add_ns - remove_ns);
    assert!(remove_ns < pair_ns * 0.70, "RemoveTag ({remove_ns}) must rank well below AddTag+RemoveTag average ({pair_ns})");
}

/// Destroying a placed subtree charges DESTROYED_GUI / DESTROYED_PART for its descendants too (not only the root before detach),
/// and Model:GetExtentsSize charges MODEL_EXTENTS_QUERY once rather than multi-charging MODEL_QUERY through GetBoundingBox/GetPivot.
#[test]
fn destroy_subtree_keeps_placed_and_model_extents_charges_once() {
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        r#"
        local Gui = Instance.new("ScreenGui")
        Gui.Parent = game:GetService("StarterGui")
        local function MakeCard(Parent)
            local Root = Instance.new("Frame")
            for _ = 1, 5 do
                local Child = Instance.new("Frame")
                Child.Parent = Root
            end
            Root.Parent = Parent
            return Root
        end
        -- Warm the frame's first-destroy cold path once
        MakeCard(Gui):Destroy()
        MakeCard(nil):Destroy()
        local Placed = MakeCard(Gui)
        local t0 = os.clock()
        Placed:Destroy()
        local PlacedUs = (os.clock() - t0) * 1e6
        local Loose = MakeCard(nil)
        local t1 = os.clock()
        Loose:Destroy()
        local LooseUs = (os.clock() - t1) * 1e6

        local Model = Instance.new("Model")
        local Part = Instance.new("Part")
        Part.Anchored = true
        Part.Parent = Model
        Model.PrimaryPart = Part
        Model.Parent = workspace
        local _ = Model:GetExtentsSize()
        local t2 = os.clock()
        for _ = 1, 1000 do
            local _ = Model:GetExtentsSize()
        end
        local ExtentsNs = (os.clock() - t2) / 1000 * 1e9
        print(string.format("%.2f %.2f %.1f", PlacedUs, LooseUs, ExtentsNs))
        "#,
    );
    let line = output.iter().find_map(|line| line.strip_prefix("print: ")).unwrap_or_else(|| panic!("{output:?}"));
    let vals: Vec<f64> = line.split(' ').map(|n| n.parse().unwrap()).collect();
    // 6 placed GuiObjects vs 6 loose: must pay at least 6 * DESTROYED_GUI (~5.34 us) more beyond leaving the DataModel.
    assert!(vals[0] - vals[1] > 5.0 * crate::cost::DESTROYED_GUI * 1e6, "placed vs loose destroy: {line}");
    // Single MODEL_EXTENTS_QUERY (8 ns) + call overhead (~15 ns), well below the old 4x MODEL_QUERY (~75+ ns).
    assert!(vals[2] < 45.0, "GetExtentsSize took {line} ns");
}

/// A link to directory `target` at `link`: a junction on Windows, which needs no privilege (a directory
/// symlink does), a symlink elsewhere.
fn link_directory(target: &std::path::Path, link: &std::path::Path) {
    #[cfg(windows)]
    {
        // mklink reads a "/" as the start of a switch
        let native = |path: &std::path::Path| path.to_string_lossy().replace('/', "\\");
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J", &native(link), &native(target)]).stdout(std::process::Stdio::null()).status().unwrap();
        assert!(made.success(), "mklink /J {} {}", link.display(), target.display());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// A link back to a directory on the way down to it is not followed: the directory would hold itself
/// without end, and two such links made the scan list 2^depth copies (a hang). Every other link mounts
/// what it leads to, and each link left out says where it leads.
#[test]
fn a_mount_leaves_out_a_link_back_up_its_way_down() {
    let dir = std::env::temp_dir().join(format!("ree-link-loops-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for folder in ["Folder", "A", "B"] {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
    }
    std::fs::write(dir.join("Folder/a.luau"), "return 1").unwrap();
    std::fs::write(dir.join("B/b.luau"), "return 2").unwrap();
    link_directory(&dir, &dir.join("Folder/Back"));
    link_directory(&dir, &dir.join("Folder/Again"));
    link_directory(&dir.join("B"), &dir.join("A/ToB"));
    link_directory(&dir.join("A"), &dir.join("B/ToA"));
    let root = dir.to_string_lossy().replace('\\', "/");
    let (lua, engine) = start();
    engine.state.borrow_mut().project.listings = Some(Vec::new());
    super::project::mount_at(&engine, &super::project::Mount::parse(&format!("ReplicatedStorage/Looped={root}"), false).unwrap()).unwrap();
    // a prepared run's listing of it reads the same again (the links left out have no stamps to compare)
    let listings = engine.state.borrow_mut().project.listings.take().unwrap();
    assert!(!listings.is_empty() && listings.iter().all(|listing| listing.unchanged(&Default::default())));
    let output = run(&lua, &engine, r#"
        local Looped = game:GetService("ReplicatedStorage").Looped
        assert(Looped.Folder:FindFirstChild("a") and not Looped.Folder:FindFirstChild("Back") and not Looped.Folder:FindFirstChild("Again"), "Folder")
        -- each link is followed once, up to the link that leads back
        assert(Looped.A.ToB:FindFirstChild("b") and not Looped.A.ToB:FindFirstChild("ToA"), "A.ToB")
        assert(Looped.B:FindFirstChild("b") and Looped.B:FindFirstChild("ToA") and not Looped.B.ToA:FindFirstChild("ToB"), "B.ToA")
        print("ok")
    "#);
    assert!(output.contains(&"print: ok".to_string()), "{output:?}");
    let left_out: Vec<&String> = output.iter().filter(|line| line.contains("a link back to")).collect();
    let says = |link: &str, above: &str| format!("warn: {root}/{link}: a link back to {root}{above}, which would hold itself; not mounted");
    for expected in [says("Folder/Again", ""), says("Folder/Back", ""), says("A/ToB/ToA", "/A"), says("B/ToA/ToB", "/B")] {
        assert!(left_out.contains(&&expected), "{expected} in {left_out:?}");
    }
    assert_eq!(left_out.len(), 4, "{left_out:?}");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Place content's walk leaves out a link back up its way down, as a mount does.
#[test]
fn place_content_leaves_out_a_link_back_up_its_way_down() {
    let repo = std::env::temp_dir().join(format!("ree-place-content-loops-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    let place = repo.join("data/place-content/Lobby");
    std::fs::create_dir_all(place.join("ReplicatedStorage")).unwrap();
    link_directory(&place, &place.join("ReplicatedStorage/Back"));
    link_directory(&place, &place.join("ReplicatedStorage/Again"));
    let engine = Engine::new(Options { repo: repo.clone(), ..Options::default() }).expect("engine");
    assert_eq!(super::project::mount_place_content(&engine, "Lobby.json"), 0);
    let left_out: Vec<String> = lines(&engine).into_iter().filter(|line| line.contains("a link back to")).collect();
    assert_eq!(left_out.len(), 2, "{left_out:?}");
    let _ = std::fs::remove_dir_all(&repo);
}

/// Enum.EasingStyle as Studio 0.747 lists it (Bezier = 11 and Spring = 12 after Cubic), and what takes one: TweenInfo.new,
/// TweenService:GetValue and GuiObject's Tween methods refuse the two in Studio's words, UIPageLayout.EasingStyle keeps its
/// value, and a Pose's easing is another enum (measured in Studio 0.747.0.7471481; tools/parity/probes/api_747.luau).
#[test]
fn spring_and_bezier_are_easing_styles_nothing_plays() {
    assert_script(
        r#"
        local Names = {}
        for _, Item in Enum.EasingStyle:GetEnumItems() do table.insert(Names, Item.Name .. "=" .. Item.Value) end
        assert(table.concat(Names, " ") == "Linear=0 Sine=1 Back=2 Quad=3 Quart=4 Quint=5 Bounce=6 Elastic=7 Exponential=8 Circular=9 Cubic=10 Bezier=11 Spring=12", table.concat(Names, " "))
        assert(tostring(Enum.EasingStyle.Spring) == "Enum.EasingStyle.Spring" and Enum.EasingStyle:FromValue(11) == Enum.EasingStyle.Bezier and Enum.EasingStyle:FromValue(13) == nil)
        local function fails(message, f, ...)
            local Ok, Problem = pcall(f, ...)
            assert(not Ok and Problem == message, tostring(Problem))
        end
        local TweenService = game:GetService("TweenService")
        local Refused = " does not support Enum.EasingStyle.Spring or Enum.EasingStyle.Bezier"
        for _, Name in { "Spring", "Bezier" } do
            local Style = Enum.EasingStyle[Name]
            fails("TweenInfo.new" .. Refused, TweenInfo.new, 1, Style)
            fails("TweenInfo.new" .. Refused, TweenInfo.new, 1, Style.Value)
            fails("TweenInfo.new" .. Refused, TweenInfo.new, 1, Style, "bad")
            fails("TweenInfo.new second argument expects Enum.EasingStyle input", TweenInfo.new, 1, Name)
            fails("TweenInfo.new first argument expects a number for time.", TweenInfo.new, "x", Style)
            for _, Alpha in { 0, 0.5, 1, -1 } do
                fails("TweenService:GetValue" .. Refused, TweenService.GetValue, TweenService, Alpha, Style, Enum.EasingDirection.In)
            end
            fails("TweenService:GetValue" .. Refused, TweenService.GetValue, TweenService, 0.5, Name, "Out")
            fails("TweenService:GetValue" .. Refused, TweenService.GetValue, TweenService, 0.5, Style.Value, 2)
            fails("Unable to cast string to EasingDirection", TweenService.GetValue, TweenService, 0.5, Style, "bad")
            fails("Argument 1 missing or nil", TweenService.GetValue, TweenService, nil, Style, Enum.EasingDirection.In)
            fails("Argument 3 missing or nil", TweenService.GetValue, TweenService, 0.5, Style)
        end
        fails("Unable to cast double to EasingStyle", TweenService.GetValue, TweenService, 0.5, 13, 0)
        fails("Unable to cast EasingDirection to EasingStyle", TweenService.GetValue, TweenService, 0.5, Enum.EasingDirection.In, Enum.EasingDirection.In)
        fails("Unable to cast string to float", TweenService.GetValue, TweenService, "x", Enum.EasingStyle.Linear, Enum.EasingDirection.In)
        assert(TweenService:GetValue(0.5, "Quad", "In") == 0.25 and TweenService:GetValue(0.5, 3, 0) == 0.25)

        local Layout = Instance.new("UIPageLayout")
        local Changes = 0
        Layout:GetPropertyChangedSignal("EasingStyle"):Connect(function() Changes += 1 end)
        assert(Layout.EasingStyle == Enum.EasingStyle.Back)
        Layout.EasingStyle = Enum.EasingStyle.Spring
        Layout.EasingStyle = "Bezier"
        Layout.EasingStyle = 12
        assert(Layout.EasingStyle == Enum.EasingStyle.Back)
        Layout.EasingStyle = Enum.EasingStyle.Quad
        Layout.EasingStyle = Enum.EasingStyle.Bezier
        assert(Layout.EasingStyle == Enum.EasingStyle.Quad)
        fails("Unable to assign property EasingStyle. Invalid value 13 for enum EasingStyle", function() Layout.EasingStyle = 13 end)
        task.wait()
        assert(Changes == 1, Changes)

        local Pose = Instance.new("Pose")
        fails("Unable to assign property EasingStyle. EnumItem of type PoseEasingStyle expected, got an EnumItem of type EasingStyle", function() Pose.EasingStyle = Enum.EasingStyle.Spring end)
        fails('Unable to assign property EasingStyle. Invalid value "Spring" for enum PoseEasingStyle', function() Pose.EasingStyle = "Spring" end)
        "#,
    );
}

/// TweenInfo.new checks every argument it is given, nil included, in order (Studio 0.747): only arguments left off take
/// their defaults, a style or direction is an item or a number cut to an item's value, and RepeatCount is whole.
#[test]
fn tween_info_new_checks_each_argument_it_is_given() {
    assert_script(
        r#"
        local Quad, In = Enum.EasingStyle.Quad, Enum.EasingDirection.In
        local function fails(message, ...)
            local Ok, Problem = pcall(TweenInfo.new, ...)
            assert(not Ok and Problem == message, tostring(Problem))
        end
        assert(tostring(TweenInfo.new()) == "Time:1 DelayTime:0 RepeatCount:0 Reverses:False EasingDirection:Out EasingStyle:Quad")
        assert(tostring(TweenInfo.new(2)) == "Time:2 DelayTime:0 RepeatCount:0 Reverses:False EasingDirection:Out EasingStyle:Quad")
        assert(tostring(TweenInfo.new(1, Quad, In, 2, true, 1, "extra")) == "Time:1 DelayTime:1 RepeatCount:2 Reverses:True EasingDirection:In EasingStyle:Quad")
        fails("TweenInfo.new first argument expects a number for time.", nil)
        fails("TweenInfo.new first argument expects a number for time.", "1")
        fails("TweenInfo.new first argument expects a number for time.", true)
        fails("TweenInfo.new first argument expects a number for time.", nil, Quad)
        fails("TweenInfo.new second argument expects Enum.EasingStyle input", 1, nil)
        fails("TweenInfo.new second argument expects Enum.EasingStyle input", 1, nil, In)
        fails("TweenInfo.new second argument expects Enum.EasingStyle input", 1, "Quad")
        fails("TweenInfo.new second argument expects Enum.EasingStyle input", 1, false)
        fails("TweenInfo.new second argument expects Enum.EasingStyle input", 1, 13)
        fails("TweenInfo.new second argument expects Enum.EasingStyle input", 1, -1)
        fails("TweenInfo.new second argument expects Enum.EasingStyle input", 1, In)
        fails("TweenInfo.new third argument expects Enum.EasingDirection input", 1, Quad, nil)
        fails("TweenInfo.new third argument expects Enum.EasingDirection input", 1, Quad, nil, 2)
        fails("TweenInfo.new third argument expects Enum.EasingDirection input", 1, Quad, "In")
        fails("TweenInfo.new third argument expects Enum.EasingDirection input", 1, Quad, 3)
        fails("TweenInfo.new third argument expects Enum.EasingDirection input", 1, Quad, Quad)
        fails("TweenInfo.new fourth arg should be a number for RepeatCount.", 1, Quad, In, nil)
        fails("TweenInfo.new fourth arg should be a number for RepeatCount.", 1, Quad, In, "3")
        fails("TweenInfo.new fifth arg should be a boolean for Reverses.", 1, Quad, In, 2, nil, 1)
        fails("TweenInfo.new fifth arg should be a boolean for Reverses.", 1, Quad, In, 2, 1)
        fails("TweenInfo.new sixth arg should be a number for DelayTime.", 1, Quad, In, 2, true, nil)
        fails("TweenInfo.new sixth arg should be a number for DelayTime.", 1, Quad, In, 2, true, "0.5")
        assert(TweenInfo.new(1, 3).EasingStyle == Quad and TweenInfo.new(1, 2.5).EasingStyle == Enum.EasingStyle.Back and TweenInfo.new(1, 10.9).EasingStyle == Enum.EasingStyle.Cubic)
        assert(TweenInfo.new(1, Quad, 0).EasingDirection == In and TweenInfo.new(1, Quad, 1.9).EasingDirection == Enum.EasingDirection.Out)
        assert(TweenInfo.new(1, Quad, In, 2.7).RepeatCount == 2 and TweenInfo.new(1, Quad, In, -1).RepeatCount == -1)
        assert(TweenInfo.new(math.huge).Time == math.huge and TweenInfo.new(-1).Time == -1)
        "#,
    );
}

/// GuiObject's Tween methods cast their arguments, refuse Spring and Bezier, and refuse an object outside the DataModel
/// (Studio 0.747): nothing moves and no callback runs when one raises.
#[test]
fn gui_tween_methods_cast_refuse_and_need_the_data_model() {
    assert_script(
        r#"
        local function fails(message, f, ...)
            local Ok, Problem = pcall(f, ...)
            assert(not Ok and Problem == message, tostring(Problem))
        end
        local Out, Quad, Spring = Enum.EasingDirection.Out, Enum.EasingStyle.Quad, Enum.EasingStyle.Spring
        local Holder = Instance.new("Folder")
        Holder.Parent = workspace
        local Frame = Instance.new("Frame")
        Frame.Parent = Holder
        local Called = false
        local To = UDim2.fromOffset(5, 5)
        for _, Style in { Spring, Enum.EasingStyle.Bezier, "Spring", 11 } do
            fails("GuiObject:TweenPosition does not support Enum.EasingStyle.Spring or Enum.EasingStyle.Bezier", Frame.TweenPosition, Frame, To, Out, Style, 0.05, true, function() Called = true end)
            fails("GuiObject:TweenSize does not support Enum.EasingStyle.Spring or Enum.EasingStyle.Bezier", Frame.TweenSize, Frame, To, Out, Style, 0.05, true)
            fails("GuiObject:TweenSizeAndPosition does not support Enum.EasingStyle.Spring or Enum.EasingStyle.Bezier", Frame.TweenSizeAndPosition, Frame, To, To, Out, Style, 0.05, true)
        end
        fails("Unable to cast string to UDim2", Frame.TweenPosition, Frame, "x", Out, Spring, 0.05, true)
        fails("Argument 1 missing or nil", Frame.TweenPosition, Frame, nil, Out, Spring, 0.05, true)
        fails("Unable to cast string to UDim2", Frame.TweenSizeAndPosition, Frame, To, "x", Out, Quad, 0.05, true)
        fails("Unable to cast string to EasingDirection", Frame.TweenPosition, Frame, To, "bad", Spring, 0.05, true)
        fails("Unable to cast double to EasingStyle", Frame.TweenPosition, Frame, To, Out, 13, 0.05, true)
        fails("Unable to cast string to float", Frame.TweenPosition, Frame, To, Out, Spring, "x", true)
        local Loose = Instance.new("Frame")
        local Unplaced = Instance.new("Frame")
        Unplaced.Parent = Instance.new("ScreenGui")
        fails("Can only tween objects in the workspace", Loose.TweenPosition, Loose, To, Out, Quad, 0.05, true)
        fails("Can only tween objects in the workspace", Loose.TweenSize, Loose, To)
        fails("Can only tween objects in the workspace", Unplaced.TweenSizeAndPosition, Unplaced, To, To)
        fails("GuiObject:TweenPosition does not support Enum.EasingStyle.Spring or Enum.EasingStyle.Bezier", Loose.TweenPosition, Loose, To, Out, Spring)
        task.wait()
        assert(Frame.Position == UDim2.new() and Frame.Size == UDim2.new() and Loose.Position == UDim2.new() and not Called)
        assert(Frame:TweenPosition(To, Out, Quad, 0.05, true, function() Called = true end) == true and Frame.Position == To)
        assert(Frame:TweenSize(To) == true and Frame.Size == To)
        task.wait()
        assert(Called)
        "#,
    );
}

/// What Studio 0.747 does with the properties its dump adds beyond storing them: a Camera's projection properties drop
/// every write, and some numbers hold a range (ParticleEmitter.TimeScale, ParticleEmitter2D's, UIBlur.Transparency,
/// InputBinding.Deadzone). Every other new property of a creatable class read and wrote back as stored there (131 of them,
/// each written once and read again).
#[test]
fn new_properties_of_0_747_ignore_and_clamp_as_studio_does() {
    assert_script(
        r#"
        local Camera = Instance.new("Camera")
        local Fired = 0
        Camera.Changed:Connect(function() Fired += 1 end)
        assert(Camera.ProjectionType == Enum.ProjectionType.Perspective and Camera.OrthographicMode == Enum.OrthographicMode.Vertical and Camera.OrthographicSize == 35 and Camera.ViewStretch == 1)
        Camera.ProjectionType = Enum.ProjectionType.Orthographic
        Camera.ProjectionType = "Orthographic"
        Camera.OrthographicMode = Enum.OrthographicMode.Horizontal
        Camera.OrthographicSize = 10
        Camera.OrthographicSize = "x"
        Camera.ViewStretch = 2
        Camera.ViewStretch = nil
        assert(Camera.ProjectionType == Enum.ProjectionType.Perspective and Camera.OrthographicMode == Enum.OrthographicMode.Vertical and Camera.OrthographicSize == 35 and Camera.ViewStretch == 1)
        local Ok, Problem = pcall(function() Camera.ProjectionType = 5 end)
        assert(not Ok and Problem == "Unable to assign property ProjectionType. Invalid value 5 for enum ProjectionType", Problem)
        Ok, Problem = pcall(function() Camera.OrthographicMode = Enum.EasingStyle.Quad end)
        assert(not Ok and Problem == "Unable to assign property OrthographicMode. EnumItem of type OrthographicMode expected, got an EnumItem of type EasingStyle", Problem)
        task.wait()
        assert(Fired == 0, Fired)

        local function set(Object, Property, Value)
            Object[Property] = Value
            return Object[Property]
        end
        local Emitter = Instance.new("ParticleEmitter")
        assert(set(Emitter, "TimeScale", -1) == 0 and set(Emitter, "TimeScale", 0.25) == 0.25 and set(Emitter, "TimeScale", 7) == 1)
        local Flat = Instance.new("ParticleEmitter2D")
        local Changes = 0
        Flat:GetPropertyChangedSignal("TimeScale"):Connect(function() Changes += 1 end)
        Flat.TimeScale = 0.5
        Flat.TimeScale = 5
        Flat.TimeScale = 9
        Flat.TimeScale = 1
        task.wait()
        assert(Changes == 2 and Flat.TimeScale == 1, Changes)
        assert(set(Flat, "TimeScale", -1) == 0 and set(Flat, "Brightness", -5) == 0 and set(Flat, "Brightness", 1e9) == 10000)
        assert(set(Flat, "Blend", 5) == 1 and set(Flat, "Blend", -1) == 0 and set(Flat, "FlipbookSizeX", 0) == 1 and set(Flat, "FlipbookSizeY", 100000) == 64)
        assert(set(Flat, "Rate", -5) == -5 and set(Flat, "Drag", -5) == -5 and set(Flat, "LightInfluence", 5) == 5 and set(Flat, "ZIndex", 2.7) == 2)
        local Blur = Instance.new("UIBlur")
        assert(Blur.Strength == Enum.UIBlurStrength.Heavy and Blur.Enabled and Blur.Color == Color3.new(1, 1, 1))
        assert(set(Blur, "Transparency", 1.5) == 1 and set(Blur, "Transparency", -0.5) == 0 and set(Blur, "Transparency", 0.5) == 0.5)
        local Binding = Instance.new("InputBinding")
        assert(Binding.Deadzone == NumberRange.new(0.1, 1), tostring(Binding.Deadzone))
        assert(set(Binding, "Deadzone", NumberRange.new(2, 3)) == NumberRange.new(1, 1) and set(Binding, "Deadzone", NumberRange.new(-3, 0.5)) == NumberRange.new(0, 0.5))

        -- stored as written, and defaults as the dump has them
        assert(set(Instance.new("Decal"), "ResampleMode", Enum.ResamplerMode.Pixelated) == Enum.ResamplerMode.Pixelated)
        assert(Instance.new("AudioDeviceInput").Active == false and Instance.new("SurfaceAppearance").SurfaceSlot == -1 and Instance.new("UIDragDetector").Selectable == true)
        assert(game.ServerRegion == "" and workspace.AuthorityMode == Enum.AuthorityMode.Automatic)
        -- gone with 0.747
        assert(not pcall(function() return Enum.PrimalPhysicsSolver end) and not pcall(function() return workspace.PrimalPhysicsSolver end))
        assert(not pcall(Instance.new, "PlayerDataRecord"))
        "#,
    );
}

/// EditableImage's Draw methods refuse the combine types and alpha types Studio 0.747 lists and does not play yet.
#[test]
fn editable_image_refuses_the_blends_studio_does_not_play_yet() {
    assert_script(
        r#"
        local AssetService = game:GetService("AssetService")
        local Image = AssetService:CreateEditableImage({ Size = Vector2.new(4, 4) })
        local Other = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
        local Red = Color3.new(1, 0, 0)
        Image:DrawRectangle(Vector2.zero, Vector2.new(4, 4), Color3.new(0, 0, 1), 0, Enum.ImageCombineType.Overwrite)
        local Before = buffer.tostring(Image:ReadPixelsBuffer(Vector2.zero, Vector2.new(4, 4)))
        local function fails(message, f, ...)
            local Ok, Problem = pcall(f, ...)
            assert(not Ok and Problem == message, tostring(Problem))
        end
        local Add, Lock, Smooth = Enum.ImageCombineType.Add, Enum.ImageAlphaType.LockCanvasAlpha, Enum.AntiAliasing.Enabled
        for _, Name in { "Darken", "Lighten", "Screen", "Overlay", "SoftLight", "HardLight", "Difference" } do
            local Combine = Enum.ImageCombineType[Name]
            local Refused = ": the requested Enum.ImageCombineType is not yet supported"
            fails("EditableImage:DrawImage" .. Refused, Image.DrawImage, Image, Vector2.zero, Other, Combine)
            fails("EditableImage:DrawImage" .. Refused, Image.DrawImage, Image, Vector2.zero, Other, Name)
            fails("EditableImage:DrawImage" .. Refused, Image.DrawImage, Image, Vector2.zero, Other, Combine.Value)
            fails("EditableImage:DrawImage" .. Refused, Image.DrawImage, Image, Vector2.zero, Other, Combine, Lock)
            fails("EditableImage:DrawRectangle" .. Refused, Image.DrawRectangle, Image, Vector2.zero, Vector2.one, Red, 0, Combine)
            fails("EditableImage:DrawCircle" .. Refused, Image.DrawCircle, Image, Vector2.one, 1, Red, 0, Combine)
            fails("EditableImage:DrawLine" .. Refused, Image.DrawLine, Image, Vector2.zero, Vector2.one, Red, 0, Combine)
            fails("EditableImage:DrawImageTransformed" .. Refused, Image.DrawImageTransformed, Image, Vector2.one, Vector2.one, 0, Other, { CombineType = Combine })
        end
        assert(Enum.ImageCombineType.Darken.Value == 8 and Enum.ImageCombineType.Difference.Value == 14)
        for _, Alpha in { Lock, Enum.ImageAlphaType.LockCanvasColor } do
            local Refused = ": Enum.ImageAlphaType other than Default is not yet supported"
            fails("EditableImage:DrawImage" .. Refused, Image.DrawImage, Image, Vector2.zero, Other, Add, Alpha)
            fails("EditableImage:DrawRectangle" .. Refused, Image.DrawRectangle, Image, Vector2.zero, Vector2.one, Red, 0, Add, Alpha)
            fails("EditableImage:DrawCircle" .. Refused, Image.DrawCircle, Image, Vector2.one, 1, Red, 0, Add, Smooth, Alpha)
            fails("EditableImage:DrawLine" .. Refused, Image.DrawLine, Image, Vector2.zero, Vector2.one, Red, 0, Add, Smooth, Alpha)
            fails("EditableImage:DrawImageTransformed" .. Refused, Image.DrawImageTransformed, Image, Vector2.one, Vector2.one, 0, Other, { CombineType = Add, AlphaBlendType = Alpha })
        end
        assert(buffer.tostring(Image:ReadPixelsBuffer(Vector2.zero, Vector2.new(4, 4))) == Before, "a refused draw leaves the image")
        Image:DrawRectangle(Vector2.zero, Vector2.one, Red, 0, Enum.ImageCombineType.Overwrite, Enum.ImageAlphaType.Default)
        assert(buffer.readu8(Image:ReadPixelsBuffer(Vector2.zero, Vector2.one), 0) == 255)
        "#,
    );
}

/// The streaming and player settings Studio 0.747's ReflectionService lists no more are still stored (a place file's
/// values, the engine's own reads), and a game script may neither read nor write them (a playing client, Studio 0.746:
/// tools/parity/probes/api_747.luau).
#[test]
fn settings_the_0_747_dump_leaves_out_are_still_stored() {
    let reflection = crate::Reflection::shared().unwrap();
    assert_eq!(reflection.version, "0.747.0.7471481");
    for (class, name) in [
        ("Workspace", "StreamingMinRadius"),
        ("Workspace", "StreamingTargetRadius"),
        ("Workspace", "StreamOutBehavior"),
        ("Workspace", "StreamingIntegrityMode"),
        ("Workspace", "PredictiveStreamingMode"),
        ("Players", "BanningEnabled"),
        ("Players", "UseStrafingAnimations"),
        ("StarterPlayer", "CreateDefaultPlayerModule"),
    ] {
        let class_id = reflection.class_id(class).unwrap();
        let property = reflection.class_property(class_id, reflection.name_id(name).unwrap()).unwrap_or_else(|| panic!("{class}.{name}"));
        assert!(property.serialized, "{class}.{name}");
    }
    let behavior = reflection.class_property(reflection.class_id("Workspace").unwrap(), reflection.name_id("StreamOutBehavior").unwrap()).unwrap();
    assert_eq!(behavior.enum_type, reflection.enum_id("StreamOutBehavior"));
    // every Content property with a string twin is still a view of it, whichever of the two the dump calls serialized
    assert_script(
        r#"
        local Label = Instance.new("ImageLabel")
        Label.Image = "rbxassetid://5"
        assert(Label.ImageContent.Uri == "rbxassetid://5")
        Label.ImageContent = Content.fromUri("rbxassetid://6")
        assert(Label.Image == "rbxassetid://6")
        local Sound = Instance.new("Sound")
        Sound.AudioContent = Content.fromUri("rbxassetid://7")
        assert(Sound.SoundId == "rbxassetid://7")
        local Mesh = Instance.new("MeshPart")
        Mesh.TextureID = "rbxassetid://8"
        assert(Mesh.TextureContent.Uri == "rbxassetid://8")
        local function fails(message, f)
            local Ok, Problem = pcall(f)
            assert(not Ok and Problem == message, tostring(Problem))
        end
        fails("The current thread cannot read 'StreamingMinRadius' (lacking capability PluginOrOpenCloud)", function() return workspace.StreamingMinRadius end)
        fails("The current thread cannot write 'StreamingTargetRadius' (lacking capability PluginOrOpenCloud)", function() workspace.StreamingTargetRadius = 64 end)
        fails("The current thread cannot read 'BanningEnabled' (lacking capability PluginOrOpenCloud)", function() return game:GetService("Players").BanningEnabled end)
        fails("The current thread cannot write 'CreateDefaultPlayerModule' (lacking capability PluginOrOpenCloud)", function() game:GetService("StarterPlayer").CreateDefaultPlayerModule = false end)
        fails("The current thread cannot read 'PlayerScriptsUseInputActionSystem' (lacking capability RobloxScript)", function() return workspace.PlayerScriptsUseInputActionSystem end)
        "#,
    );
}

/// The fast-flag methods and GuiService:GetResolutionScale are there for a game script to read and raise when it calls
/// them, in every run context, with the client's text and capability (reflection.rs CAPABILITY_CALLS, measured in a
/// playing client and a server, Studio 0.747: tools/parity/probes/engine_capabilities.luau), where they answered nil and
/// reported themselves missing: a settings helper that takes its default when the define raises went on with nil, and a
/// scheduler that then read the setting divided by nil.
#[test]
fn engine_only_methods_raise_a_game_scripts_capability_error_in_every_run_context() {
    use super::RunContext;
    let script = r#"
        local GuiService = game:GetService("GuiService")
        local Calls = {
            DefineFastFlag = function() return game:DefineFastFlag("TestFlag", true) end,
            DefineFastInt = function() return game:DefineFastInt("TestInt", 7) end,
            DefineFastString = function() return game:DefineFastString("TestString", "seven") end,
            GetFastFlag = function() return game:GetFastFlag("TestFlag") end,
            GetFastInt = function() return game:GetFastInt("TestInt") end,
            GetFastString = function() return game:GetFastString("TestString") end,
            GetEngineFeature = function() return game:GetEngineFeature("TestFeature") end,
            GetResolutionScale = function() return GuiService:GetResolutionScale() end,
        }
        for _, Name in { "DefineFastFlag", "DefineFastInt", "DefineFastString", "GetFastFlag", "GetFastInt", "GetFastString", "GetEngineFeature", "GetResolutionScale" } do
            local Owner = if Name == "GetResolutionScale" then GuiService else game
            local Ok, Message = pcall(Calls[Name])
            local NoSelf = select(2, pcall(Owner[Name]))
            print(Name, typeof(Owner[Name]), Ok, type(Message), Message, NoSelf == Message)
        end

        -- a settings helper: define the setting under pcall and keep the default when the call raises
        local function Setting(Kind, Name, Default)
            local Ok, Value = pcall(function()
                return game["Define" .. Kind](game, Name, Default)
            end)
            if Ok then
                return Value
            end
            return Default
        end
        print("settings", Setting("FastInt", "TestFrameRate", 60), Setting("FastFlag", "TestEnabled", true), Setting("FastString", "TestMode", "on"))

        -- a scheduler that reads its frame budget through such a helper: the getter it keeps reads the engine's value only
        -- when the define went through, and the budget divides by it
        local function IntGetter(Name, Default)
            if pcall(function() return game:DefineFastInt(Name, Default) end) then
                return function() return game:GetFastInt(Name) end
            end
            return function() return Default end
        end
        local TargetFps = IntGetter("TestTargetFps", 50)
        print("frame budget", 1000 / TargetFps())
    "#;
    let mut want: Vec<String> = [("DefineFastFlag", "RobloxScript"), ("DefineFastInt", "RobloxScript"), ("DefineFastString", "RobloxScript"), ("GetFastFlag", "LocalUser"), ("GetFastInt", "LocalUser"), ("GetFastString", "LocalUser"), ("GetEngineFeature", "LocalUser"), ("GetResolutionScale", "RobloxScript")]
        .iter()
        .map(|(name, capability)| format!("print: {name} function false string The current thread cannot call '{name}' (lacking capability {capability}) true"))
        .collect();
    want.push("print: settings 60 true on".into());
    want.push("print: frame budget 20".into());
    for (mode, run_context) in [("play", RunContext::Client), ("play", RunContext::Server), ("edit", RunContext::Edit), ("edit", RunContext::Client)] {
        let (lua, engine) = start_with(Options { mode: mode.into(), run_context, ..Options::default() });
        let output = run(&lua, &engine, script);
        assert_eq!(output, want, "{mode} {run_context:?}");
    }
}

/// Workspace's and WorldRoot's engine-only methods raise a game script's capability error as a playing client and its server
/// do (Studio 0.748): the method reads as a function, a call raises before the arguments or the instance are looked at, and a
/// WorldRoot one is the same function on the Workspace and on a WorldModel. They answered nil and reported themselves
/// missing. A plugin's code (edit mode) may call the six that lack only Plugin, which still report themselves not emulated.
/// The Workspace's BreakJoints and MakeJoints are its own: a Model's are other members, which a game script may call, and so
/// are the lower-case `breakJoints` and `makeJoints` the Workspace inherits from Model (a playing client and its server run
/// them with no capability check), as Instance's `remove` is on Selection, whose own Remove lacks Plugin.
#[test]
fn workspace_and_world_root_engine_only_methods_raise_a_game_scripts_capability_error() {
    use super::RunContext;
    // (name, the capability a game script lacks, declared by WorldRoot)
    const METHODS: &[(&str, &str, bool)] = &[
        ("GetAwakeContactNormals", "RobloxScript", true),
        ("GetAwakeContactParts", "RobloxScript", true),
        ("GetAwakeContactPositions", "RobloxScript", true),
        ("GetAwakeRootParts", "RobloxScript", true),
        ("SetInsertPoint", "RobloxScript", true),
        ("CacheCurrentTerrain", "RobloxScript", true),
        ("ClearCachedTerrain", "RobloxScript", true),
        ("RaycastCachedTerrain", "RobloxScript", true),
        ("Simulate", "RobloxEngine", true),
        ("StepPhysics", "Plugin", true),
        ("IKMoveTo", "Plugin", true),
        ("CalculateJumpDistance", "RobloxScript", false),
        ("CalculateJumpHeight", "RobloxScript", false),
        ("CalculateJumpPower", "RobloxScript", false),
        ("SetAvatarUnificationMode", "RobloxScript", false),
        ("SetMeshPartHeadsAndAccessories", "RobloxScript", false),
        ("ZoomToExtents", "Plugin", false),
        ("ApplyRecommendedStreamingSettings", "Plugin", false),
        ("ExperimentalSolverIsEnabled", "LocalUser", false),
        ("SetPhysicsThrottleEnabled", "LocalUser", false),
        ("BreakJoints", "Plugin", false),
        ("MakeJoints", "Plugin", false),
    ];
    let list: Vec<String> = METHODS.iter().map(|(name, _, root)| format!("{{ \"{name}\", {root} }}")).collect();
    let script = r#"
        local Model = Instance.new("WorldModel")
        for _, Entry in { METHODS } do
            local Name, Root = Entry[1], Entry[2]
            local Ok, Message = pcall(workspace[Name], workspace)
            if Ok then
                print(Name, "answers", Message)
                continue
            end
            -- arguments of any type, no instance, or another instance: the same error
            local Typed = select(2, pcall(workspace[Name], workspace, {}, {}))
            local NoSelf = select(2, pcall(workspace[Name]))
            local OtherSelf = select(2, pcall(workspace[Name], game))
            local Line = { Name, typeof(workspace[Name]), tostring(Ok), type(Message), Message, tostring(Typed == Message and NoSelf == Message and OtherSelf == Message) }
            if Root then
                local ModelOk, ModelMessage = pcall(Model[Name], Model)
                table.insert(Line, tostring(Model[Name] == workspace[Name]))
                table.insert(Line, tostring(not ModelOk and ModelMessage == Message))
            end
            print(table.concat(Line, " "))
        end
        local Plain = Instance.new("Model")
        for _, Name in { "BreakJoints", "MakeJoints" } do
            local Ok, Message = pcall(Plain[Name], Plain)
            print("Model", Name, typeof(Plain[Name]), tostring(Plain[Name] ~= workspace[Name]), tostring(Ok or not string.find(Message, "lacking capability", 1, true)))
        end
        -- Model's lower-case breakJoints and makeJoints, which the Workspace inherits, are Model's members, not the
        -- Workspace's own BreakJoints and MakeJoints; Instance's remove is not Selection's own Remove
        for _, Name in { "breakJoints", "makeJoints" } do
            local Ok, Message = pcall((workspace :: any)[Name], workspace)
            print("workspace", Name, tostring(Ok or not string.find(Message, "lacking capability", 1, true)))
        end
        local Selection = game:GetService("Selection")
        local _, Removed = pcall((Selection :: any).remove, Selection)
        print("Selection remove", tostring(string.find(Removed, "The Parent property of Selection is locked", 1, true) == 1))
    "#
    .replace("METHODS", &list.join(", "));
    for (mode, run_context) in [("play", RunContext::Client), ("play", RunContext::Server), ("edit", RunContext::Edit), ("edit", RunContext::Client)] {
        let mut want = Vec::new();
        for &(name, capability, root) in METHODS {
            if mode == "edit" && capability == "Plugin" {
                want.push(format!("missing: Workspace.{name} is not emulated"));
                want.push(format!("print: {name} answers nil"));
                continue;
            }
            let both = if root { " true true" } else { "" };
            want.push(format!("print: {name} function false string The current thread cannot call '{name}' (lacking capability {capability}) true{both}"));
        }
        for name in ["BreakJoints", "MakeJoints"] {
            want.push(format!("missing: Model.{name} is not emulated"));
            want.push(format!("print: Model {name} function true true"));
        }
        for name in ["breakJoints", "makeJoints"] {
            want.push(format!("missing: Workspace.{name} is not emulated"));
            want.push(format!("print: workspace {name} true"));
        }
        want.push("print: Selection remove true".into());
        let (lua, engine) = start_with(Options { mode: mode.into(), run_context, ..Options::default() });
        let output = run(&lua, &engine, &script);
        assert_eq!(output, want, "{mode} {run_context:?}");
    }
}

/// What Studio 0.747's dump adds and ree does not emulate reports itself (an Output line of kind `missing` at its first use)
/// and answers nil; what the dump drops is no member any more.
#[test]
fn methods_new_in_0_747_report_themselves_as_not_emulated() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        print(game:GetService("CollectionService"):GetCollections(Instance.new("Part")))
        print(game:GetService("RunService"):BindToSimulationParallel(function() end, Enum.StepFrequency.Hz60, 1))
        local Emitter = Instance.new("ParticleEmitter2D")
        print(Emitter:Emit(3), Emitter:Clear())
        print(Instance.new("Sound"):GetPlaybackState(), Instance.new("TextDocument"):ReadAsync())
        print(game:GetService("FeatureMatchingService").ClassName, Instance.new("UIBlur"):IsA("UIComponent"), Instance.new("AvatarRules").ClassName)
        print(pcall(function() return Instance.new("AnimationRigData"):LoadFromHumanoid() end))
        "#,
    );
    assert_eq!(
        output,
        [
            "missing: CollectionService.GetCollections is not emulated",
            "print: ",
            "missing: RunService.BindToSimulationParallel is not emulated",
            "print: ",
            "missing: ParticleEmitter2D.Emit is not emulated",
            "missing: ParticleEmitter2D.Clear is not emulated",
            "print: nil",
            "missing: Sound.GetPlaybackState is not emulated",
            "missing: TextDocument.ReadAsync is not emulated",
            "print: nil",
            "print: FeatureMatchingService true AvatarRules",
            "print: false LoadFromHumanoid is not a valid member of AnimationRigData \"AnimationRigData\"",
        ]
    );
    let missing: Vec<String> = engine.state.borrow().output.missing.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(
        missing,
        ["CollectionService.GetCollections", "RunService.BindToSimulationParallel", "ParticleEmitter2D.Emit", "ParticleEmitter2D.Clear", "Sound.GetPlaybackState", "TextDocument.ReadAsync"]
    );
}

#[test]
fn a_color3_tween_blends_its_channels_linearly() {
    // Measured in roblox-sim's 0.747 client and server with live flags (tools/parity/probes/tween_color.luau, 1,485
    // channels bit-exact over every style and direction): each channel is a + (b - a) * eased in float32 on every class's
    // Color3 properties, so Back's overshoot goes below 0; the end is that blend at 1 (a hair off the goal itself) and a
    // reversed tween ends on the blend at 0, the start itself. Roblox's DFFlagStyleTransitionColorSpace, off since
    // 2026-11-13, blended the squares instead.
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    engine.state.borrow_mut().sched.set_main_thread(&lua.current_thread());
    run(
        &lua,
        &engine,
        r#"
        local TweenService = game:GetService("TweenService")
        local A, B = Color3.fromRGB(200, 40, 40), Color3.fromRGB(40, 40, 200)
        local function Make(Class, Property, From, To, Info)
            local Object = Instance.new(Class)
            Object[Property] = From
            local Tween = TweenService:Create(Object, Info, { [Property] = To })
            Tween:Play()
            return Object, Tween
        end
        _G.Frame = Make("Frame", "BackgroundColor3", A, B, TweenInfo.new(1, Enum.EasingStyle.Linear))
        _G.Eased = Make("NumberValue", "Value", 0, 1, TweenInfo.new(1, Enum.EasingStyle.Linear))
        _G.Light = Make("PointLight", "Color", A, B, TweenInfo.new(1, Enum.EasingStyle.Linear))
        _G.Back = Make("Color3Value", "Value", Color3.new(0, 0, 0), Color3.new(1, 1, 1), TweenInfo.new(1, Enum.EasingStyle.Back, Enum.EasingDirection.In))
        _G.Ending = Make("Color3Value", "Value", A, B, TweenInfo.new(0.05))
        _G.Reversed = Make("Color3Value", "Value", Color3.new(-0.5, 0.3, 2), Color3.new(0.2, -0.7, 0.1), TweenInfo.new(0.05, Enum.EasingStyle.Linear, Enum.EasingDirection.In, 0, true))
        _G.Lerp = A:Lerp(B, 0.5)
        "#,
    );
    for _ in 0..12 {
        scheduler::step(&lua, &engine, Some(1.0 / 60.0)).unwrap();
    }
    let output = run(
        &lua,
        &engine,
        r#"
        local function F(X) return Vector3.new(X, 0, 0).X end
        local function Linear(A, B, T)
            return F(A + F(F(B - A) * F(T)))
        end
        local A, B = Color3.fromRGB(200, 40, 40), Color3.fromRGB(40, 40, 200)
        local T = _G.Eased.Value
        assert(T > 0.1 and T < 0.3, "eased " .. T)
        for _, Color in { _G.Frame.BackgroundColor3, _G.Light.Color } do
            for _, C in { "R", "G", "B" } do
                assert(Color[C] == Linear(A[C], B[C], T), C .. " " .. Color[C] .. " " .. Linear(A[C], B[C], T))
            end
        end
        assert(_G.Frame.BackgroundColor3.R < 0.7 and _G.Frame.BackgroundColor3.B < 0.3, "linear, not squared")
        local Back = _G.Back.Value
        assert(Back.R < -0.01 and Back.R == Back.G and Back.R == Back.B, "Back's overshoot goes below 0 " .. tostring(Back))
        local Ending = _G.Ending.Value
        assert(Ending.R == Linear(A.R, B.R, 1) and Ending.R ~= B.R and Ending.B == Linear(A.B, B.B, 1), "the end is the blend at 1")
        local Reversed = _G.Reversed.Value
        assert(Reversed.R == -0.5 and Reversed.G == F(0.3) and Reversed.B == 2, "reversed end " .. tostring(Reversed))
        assert(math.abs(_G.Lerp.R - 120 / 255) < 1e-6, "Color3:Lerp stays linear")
        print("ok")
        "#,
    );
    assert!(output.iter().any(|line| line.ends_with("ok")), "{output:?}");
}

/// The Input Action System as a 0.747 client answers scripts (tools/parity/probes/input_actions.luau): GetState's defaults;
/// Fire sets the state at once and queues Pressed / Released, then StateChanged; a value the state holds changes nothing;
/// Fire acts in an enabled context anywhere (or with no context, in the DataModel) and nowhere else; a Scriptable binding's
/// Fire is its action's; disabling, moving or retyping resets; the client's errors. Before, Fire did nothing and GetState
/// answered nil.
#[test]
fn input_actions_keep_their_state_and_fire() {
    let (lua, engine) = start();
    run(&lua, &engine, r#"
        local RS = game:GetService("ReplicatedStorage")
        local Log = {}
        shared.Log = Log
        local function watch(label, action)
            action.Pressed:Connect(function() table.insert(Log, `{label} Pressed`) end)
            action.Released:Connect(function() table.insert(Log, `{label} Released`) end)
            action.StateChanged:Connect(function(value) table.insert(Log, `{label} StateChanged {value}`) end)
        end
        local function make(kind, parent)
            local context = Instance.new("InputContext")
            local action = Instance.new("InputAction")
            if kind then action.Type = kind end
            action.Parent = context
            context.Parent = parent
            return context, action
        end
        local defaults = {}
        for _, kind in Enum.InputActionType:GetEnumItems() do
            local action = Instance.new("InputAction")
            action.Type = kind
            table.insert(defaults, tostring(action:GetState()))
        end
        assert(table.concat(defaults, ";") == "false;0;0, 0;0, 0, 0;-1, -1", table.concat(defaults, ";"))
        local context, a = make(nil, RS)
        watch("a", a)
        a:Fire(true)
        assert(a:GetState() == true and #Log == 0, "at once; the events deferred")
        a:Fire(true)
        a:Fire(false)
        local _, one = make(Enum.InputActionType.Direction1D, nil)
        watch("one", one)
        one:Fire(0.7)
        assert(one:GetState() == 0.699999988079071, "a float: " .. one:GetState())
        local loose = Instance.new("InputAction")
        loose:Fire(true)
        assert(loose:GetState() == false, "no context, outside the DataModel")
        local inGame = Instance.new("InputAction")
        inGame.Parent = workspace
        inGame:Fire(true)
        assert(inGame:GetState() == true, "no context, in the DataModel")
        local errors = {}
        for _, case in { { a, 1 }, { a }, { one, true } } do
            table.insert(errors, select(2, pcall(case[1].Fire, case[1], case[2])))
        end
        local binding = Instance.new("InputBinding")
        binding.Parent = a
        table.insert(errors, select(2, pcall(binding.Fire, binding, true)))
        table.insert(errors, select(2, pcall(context.GetInputActions, context)))
        shared.Errors = table.concat(errors, "|")
        binding.Type = Enum.InputBindingType.Scriptable
        binding:Fire(true)
        assert(a:GetState() == true, "a Scriptable binding's Fire")
        context.Enabled = false
        assert(a:GetState() == false, "a disabled context resets")
        a:Fire(true)
        assert(a:GetState() == false, "nor fires")
        context.Enabled = true
        a:Fire(true)
        context.Parent = workspace
        assert(a:GetState() == false, "a moved context resets")
        a:Fire(true)
        a.Type = Enum.InputActionType.Direction2D
        assert(a:GetState() == Vector2.zero, "a new type's default")
    "#);
    let output = run(&lua, &engine, r#"print("log", table.concat(shared.Log, ",")) print("errors", shared.Errors)"#);
    let log = "a Pressed,a StateChanged true,a Released,a StateChanged false,one StateChanged 0.699999988079071,a Pressed,a StateChanged true,\
        a Released,a StateChanged false,a Pressed,a StateChanged true,a Released,a StateChanged false,a Pressed,a StateChanged true,\
        a StateChanged 0, 0";
    assert!(output.iter().any(|line| line == &format!("print: log {log}")), "{output:?}");
    let errors = "Action:Fire() on InputActionType.Bool should be called with a bool state|Argument 1 missing or nil|\
        Action:Fire() on InputActionType.Direction1D should be called with a number state|\
        InputBinding:Fire() can only be called when Type is Scriptable|\
        The current thread cannot call 'GetInputActions' (lacking capability RobloxScript)";
    assert!(output.iter().any(|line| line == &format!("print: errors {errors}")), "{output:?}");
}

/// A host's device input reaches bound actions through services::input_actions::deliver as a 0.747 client delivers keys
/// and buttons (measured with a client's own key and mouse input): a key's binding presses a Bool and gives a
/// Direction1D its Scale; modifiers must be down first and their release releases; composites sum and clamp; a sinking
/// context of a higher priority keeps the input from lower ones, equal priorities all get it, the newest context first;
/// nothing under StarterGui; a binding whose key changes while held releases; a key going up reaches what it holds.
#[test]
fn device_input_reaches_bound_actions() {
    let (lua, engine) = start();
    run(&lua, &engine, r#"
        local Log = {}
        shared.Log = Log
        local function make(label, parent, props, kind, priority, sink)
            local context = Instance.new("InputContext")
            if priority then context.Priority = priority end
            if sink then context.Sink = true end
            local action = Instance.new("InputAction")
            if kind then action.Type = kind end
            action.Parent = context
            local binding = Instance.new("InputBinding")
            for key, value in props do binding[key] = value end
            binding.Parent = action
            context.Parent = parent
            action.Pressed:Connect(function() table.insert(Log, `{label}+`) end)
            action.Released:Connect(function() table.insert(Log, `{label}-`) end)
            action.StateChanged:Connect(function(value) if typeof(value) ~= "boolean" then table.insert(Log, `{label}={value}`) end end)
            shared[label] = { context = context, action = action, binding = binding }
        end
        local RS = game:GetService("ReplicatedStorage")
        make("e", RS, { KeyCode = Enum.KeyCode.E })
        make("e2", RS, { KeyCode = Enum.KeyCode.E })
        make("shiftE", RS, { KeyCode = Enum.KeyCode.E, PrimaryModifier = Enum.KeyCode.LeftShift })
        make("q-high", RS, { KeyCode = Enum.KeyCode.Q }, nil, 2000, true)
        make("q-low", RS, { KeyCode = Enum.KeyCode.Q })
        make("f", RS, { KeyCode = Enum.KeyCode.F, Scale = 2.5 }, Enum.InputActionType.Direction1D)
        make("wasd", RS, { Up = Enum.KeyCode.W, Down = Enum.KeyCode.S, Left = Enum.KeyCode.A, Right = Enum.KeyCode.D }, Enum.InputActionType.Direction2D)
        make("starter", game:GetService("StarterGui"), { KeyCode = Enum.KeyCode.R })
        make("click", RS, { KeyCode = Enum.KeyCode.MouseLeftButton })
    "#);
    let key = |name: &str| engine.reflection.enum_item("KeyCode", name).unwrap();
    let mut steps: Vec<String> = Vec::new();
    let mut step = |label: &str, events: &[(&str, bool)], after: &str| {
        for (name, down) in events {
            crate::services::input_actions::deliver(&engine, key(name), *down);
            scheduler::flush_deferred(&lua, &engine).unwrap();
        }
        if !after.is_empty() {
            run(&lua, &engine, after);
        }
        engine.state.borrow_mut().output.entries.clear();
        let output = run(&lua, &engine, r#"print("log", table.concat(shared.Log, ",")) table.clear(shared.Log)"#);
        let line = output.iter().find_map(|line| line.strip_prefix("print: log ")).unwrap_or_default().to_string();
        steps.push(format!("{label}: {line}"));
    };
    step("E", &[("E", true), ("E", false)], "");
    step("shift E", &[("LeftShift", true), ("E", true), ("LeftShift", false), ("E", false)], "");
    step("E shift", &[("E", true), ("LeftShift", true), ("E", false), ("LeftShift", false)], "");
    step("Q", &[("Q", true), ("Q", false)], "");
    step("F", &[("F", true), ("F", false)], "");
    step("W D", &[("W", true), ("D", true), ("W", false), ("D", false)], "");
    step("R under StarterGui", &[("R", true), ("R", false)], "");
    step("click", &[("MouseLeftButton", true), ("MouseLeftButton", false)], "");
    step("E held, rebound", &[("E", true)], "shared.e.binding.KeyCode = Enum.KeyCode.T");
    step("E up", &[("E", false)], "");
    let wanted = [
        "E: e2+,e+,e2-,e-",
        "shift E: shiftE+,e2+,e+,shiftE-,e2-,e-",
        "E shift: e2+,e+,e2-,e-",
        "Q: q-high+,q-high-",
        "F: f=2.5,f=0",
        "W D: wasd=0, 1,wasd=0.707106769, 0.707106769,wasd=1, 0,wasd=0, 0",
        "R under StarterGui: ",
        "click: click+,click-",
        "E held, rebound: e2+,e+,e-",
        "E up: e2-",
    ];
    assert_eq!(steps, wanted);
}

/// PhysicsService's collision group registry on a server (measured in roblox-sim's 0.747 server: tools/parity/probes/
/// collision_groups.luau and collision_group_args.luau): Default alone at the start, groups at the lowest free id, the
/// masks a collidable pair gives, a rename, an unregister and the id a later group takes, the casts, the errors, the
/// 32-group limit and the deprecated CreateCollisionGroup / GetCollisionGroups. A game that walks the groups each frame
/// iterated nil before.
#[test]
fn collision_groups_register_collide_rename_and_unregister_on_a_server() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local function Groups()
            local Parts = {}
            for _, Group in PhysicsService:GetRegisteredCollisionGroups() do
                table.insert(Parts, Group.name .. "=" .. Group.mask)
            end
            return table.concat(Parts, " ")
        end
        local function Try(...)
            local Ok, Problem = pcall(...)
            return if Ok then "ok" else Problem
        end
        print(PhysicsService:GetMaxCollisionGroups(), Groups())
        PhysicsService:RegisterCollisionGroup("GroupA")
        PhysicsService:RegisterCollisionGroup("GroupB")
        PhysicsService:RegisterCollisionGroup("GroupA")
        PhysicsService:CollisionGroupSetCollidable("GroupA", "GroupB", false)
        PhysicsService:CollisionGroupSetCollidable("GroupB", "GroupB", false)
        print(Groups(), PhysicsService:IsCollisionGroupRegistered("GroupA"), PhysicsService:IsCollisionGroupRegistered("groupa"))
        print(PhysicsService:CollisionGroupsAreCollidable("GroupB", "GroupA"), PhysicsService:CollisionGroupsAreCollidable("GroupA", "Default"), PhysicsService:CollisionGroupsAreCollidable("GroupA", "Nope"))
        PhysicsService:RenameCollisionGroup("GroupA", "Renamed")
        PhysicsService:UnregisterCollisionGroup("GroupB")
        PhysicsService:RegisterCollisionGroup("GroupC")
        print(Groups())
        print(Try(PhysicsService.RegisterCollisionGroup, PhysicsService, ""))
        print(Try(PhysicsService.RegisterCollisionGroup, PhysicsService, "Default"))
        print(Try(PhysicsService.RegisterCollisionGroup, PhysicsService, string.rep("x", 101)))
        print(Try(PhysicsService.RegisterCollisionGroup, PhysicsService, string.rep("x", 100)))
        print(Try(PhysicsService.CollisionGroupSetCollidable, PhysicsService, "GroupC", "Nope", false))
        print(Try(PhysicsService.CollisionGroupSetCollidable, PhysicsService, "Nope", "Default", "no"))
        print(Try(PhysicsService.RenameCollisionGroup, PhysicsService, nil, nil))
        print(Try(PhysicsService.GetCollisionGroupId, PhysicsService, "Nope"))
        print(Try(PhysicsService.GetCollisionGroupName, PhysicsService, 31.5))
        print(PhysicsService:GetCollisionGroupName(1.5), PhysicsService:GetCollisionGroupName("1"), PhysicsService:GetCollisionGroupName(9), PhysicsService:GetCollisionGroupId("GroupC"))
        PhysicsService:RegisterCollisionGroup(5)
        PhysicsService:RegisterCollisionGroup(true)
        PhysicsService:CollisionGroupSetCollidable("5", "true", 0)
        local Off = PhysicsService:CollisionGroupsAreCollidable("5", "true")
        PhysicsService:CollisionGroupSetCollidable("5", "true", "TRUE")
        print(PhysicsService:IsCollisionGroupRegistered(5), PhysicsService:IsCollisionGroupRegistered("true"), Off, PhysicsService:CollisionGroupsAreCollidable("5", "true"))
        for Index = 1, 26 do
            PhysicsService:RegisterCollisionGroup("Fill" .. Index)
        end
        print(#PhysicsService:GetRegisteredCollisionGroups(), Try(PhysicsService.RegisterCollisionGroup, PhysicsService, "Over"), Try(PhysicsService.RegisterCollisionGroup, PhysicsService, "GroupC"))
        PhysicsService:UnregisterCollisionGroup("Fill1")
        local Id = PhysicsService:CreateCollisionGroup("Made")
        local Entry = PhysicsService:GetCollisionGroups()[Id + 1]
        print(Id, Entry.id, Entry.name, Entry.mask, Try(PhysicsService.CreateCollisionGroup, PhysicsService, "Made"))
        "#,
    );
    assert_eq!(
        output,
        [
            "print: 32 Default=-1",
            "print: Default=-1 GroupA=-5 GroupB=-7 true false",
            "print: false true true",
            "print: Default=-1 Renamed=-1 GroupC=-1",
            "print: Collision group requires a valid name.",
            "print: Can not use/change the Default collision group.",
            "print: Collision group name length cannot exceed 100 chars",
            "print: ok",
            "print: Both collision groups must be registered.",
            "print: Unable to cast string to bool",
            "print: Argument 2 missing or nil",
            "print: Collision group not found.",
            "print: The Collision group Id must be between 0 and 32",
            "print: GroupC Renamed  2",
            // (RegisterCollisionGroup(true): a group name is a string parameter, which a server warns about before it casts it)
            "warn: argument #1 expects a string, but boolean was passed",
            "print: true true false true",
            "print: 32 Cannot register more than 32 collision groups in this world. Cannot register more than 32 collision groups in this world.",
            "print: 6 6 Made -1 Cannot register more than 32 collision groups.",
        ]
    );
    let missing: Vec<String> = engine.state.borrow().output.missing.iter().map(|(name, _)| name.clone()).filter(|name| name.contains("PhysicsService")).collect();
    assert!(missing.is_empty(), "{missing:?}");
}

/// A playing client reads the registry and may not change it: the name checks of Create and Remove come first, the
/// argument casts before everything (measured in roblox-sim's 0.747 client); SetPartCollisionGroup works there. An edit
/// DataModel (ree's default mode, an edit-mode client too) may register.
#[test]
fn a_playing_client_reads_collision_groups_and_may_not_change_them() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local function Try(...)
            local Ok, Problem = pcall(...)
            return if Ok then "ok" else Problem
        end
        print(PhysicsService:GetMaxCollisionGroups(), #PhysicsService:GetRegisteredCollisionGroups(), PhysicsService:IsCollisionGroupRegistered("Default"), PhysicsService:CollisionGroupsAreCollidable("Default", "Nope"))
        for _, Name in { "RegisterCollisionGroup", "UnregisterCollisionGroup", "CreateCollisionGroup", "RemoveCollisionGroup" } do
            print(Name, Try(PhysicsService[Name], PhysicsService, "GroupA"), Try(PhysicsService[Name], PhysicsService, ""), Try(PhysicsService[Name], PhysicsService, nil))
        end
        print(Try(PhysicsService.RenameCollisionGroup, PhysicsService, "GroupA", "GroupB"), Try(PhysicsService.CollisionGroupSetCollidable, PhysicsService, "Default", "Default", false), Try(PhysicsService.CollisionGroupSetCollidable, PhysicsService, "Default", "Default", "no"))
        local Part = Instance.new("Part")
        PhysicsService:SetPartCollisionGroup(Part, "GroupA")
        print(Part.CollisionGroup, PhysicsService:CollisionGroupContainsPart("GroupA", Part), Try(PhysicsService.CollisionGroupContainsPart, PhysicsService, "GroupA", Instance.new("Folder")), #PhysicsService:GetRegisteredCollisionGroups())
        "#,
    );
    let refused = "This API can only be used on the server!";
    assert_eq!(
        output,
        [
            "print: 32 1 true true".to_string(),
            format!("print: RegisterCollisionGroup {refused} {refused} Argument 1 missing or nil"),
            format!("print: UnregisterCollisionGroup {refused} {refused} Argument 1 missing or nil"),
            format!("print: CreateCollisionGroup {refused} Collision group requires a valid name. Argument 1 missing or nil"),
            format!("print: RemoveCollisionGroup {refused} Collision group requires a valid name. Argument 1 missing or nil"),
            format!("print: {refused} {refused} Unable to cast string to bool"),
            "print: GroupA true Parameter 2 must be BasePart in CollisionGroupContainsPart. 1".to_string(),
        ]
    );
    for (mode, run_context) in [("edit", RunContext::Client), ("edit", RunContext::Edit)] {
        let (lua, engine) = start_with(Options { mode: mode.into(), run_context, ..Options::default() });
        let output = run(&lua, &engine, r#"game:GetService("PhysicsService"):RegisterCollisionGroup("GroupA") print(#game:GetService("PhysicsService"):GetRegisteredCollisionGroups())"#);
        assert_eq!(output, ["print: 2"], "{mode} {run_context:?}");
    }
}

/// A part's CollisionGroupId is its group's id while it is in the Workspace (-1 for a name that is not registered): worked
/// out on the way in (inside a model), when its name changes there and when the groups change, over a script's id, and
/// left as it is outside; each change fires Changed. CollisionGroup ignores "" and keeps 100 bytes. (Measured in
/// roblox-sim's 0.747 server: tools/parity/probes/collision_group_args.luau.)
#[test]
fn a_parts_collision_group_id_follows_the_registry_in_the_workspace() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        PhysicsService:RegisterCollisionGroup("GroupA")
        local Model = Instance.new("Model")
        local Inner = Instance.new("Part")
        local Changes = {}
        Inner.Changed:Connect(function(Name)
            if string.find(Name, "^CollisionGroup") then
                table.insert(Changes, Name)
            end
        end)
        Inner.CollisionGroup = "GroupA"
        Inner.Parent = Model
        print("outside", Inner.CollisionGroupId)
        Model.Parent = workspace
        print("entered", Inner.CollisionGroupId)
        Inner.CollisionGroup = "GroupB"
        print("unknown", Inner.CollisionGroupId)
        PhysicsService:RegisterCollisionGroup("GroupB")
        print("registered", Inner.CollisionGroupId)
        Inner.CollisionGroupId = 9
        PhysicsService:CollisionGroupSetCollidable("GroupA", "GroupB", false)
        print("collidable", Inner.CollisionGroupId)
        PhysicsService:UnregisterCollisionGroup("GroupA")
        print("unregistered", Inner.CollisionGroupId)
        PhysicsService:RenameCollisionGroup("GroupB", "GroupC")
        print("renamed", Inner.CollisionGroup, Inner.CollisionGroupId)
        Inner.CollisionGroup = ""
        print("empty", Inner.CollisionGroup)
        Inner.CollisionGroup = string.rep("L", 99) .. "\u{00FC}"
        print("cut", #Inner.CollisionGroup)
        Model.Parent = nil
        Inner.CollisionGroup = "GroupC"
        print("left", Inner.CollisionGroup, Inner.CollisionGroupId)
        task.defer(function()
            print("changes", table.concat(Changes, " "))
        end)
        "#,
    );
    let changes = ["CollisionGroup", "CollisionGroupId", "CollisionGroup", "CollisionGroupId", "CollisionGroupId", "CollisionGroupId", "CollisionGroupId", "CollisionGroupId", "CollisionGroup", "CollisionGroup"];
    assert_eq!(
        output,
        [
            "print: outside 0".to_string(),
            "print: entered 1".into(),
            "print: unknown -1".into(),
            "print: registered 2".into(),
            "print: collidable 9".into(),
            "print: unregistered 2".into(),
            "print: renamed GroupB -1".into(),
            "print: empty GroupB".into(),
            "print: cut 100".into(),
            "print: left GroupC -1".into(),
            format!("print: changes {}", changes.join(" ")),
        ]
    );
}

/// Each engine (DataModel) has its own collision groups.
#[test]
fn each_engine_has_its_own_collision_groups() {
    use super::RunContext;
    let server = || start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let (first_lua, first) = server();
    let (second_lua, second) = server();
    let count = r#"print(#game:GetService("PhysicsService"):GetRegisteredCollisionGroups())"#;
    assert_eq!(run(&first_lua, &first, &format!(r#"game:GetService("PhysicsService"):RegisterCollisionGroup("GroupA") {count}"#)), ["print: 2"]);
    assert_eq!(run(&second_lua, &second, count), ["print: 1"]);
}

/// A group change works out every part's CollisionGroupId in the Workspace again, and costs the work clock what a server spends
/// on that (roblox-sim, Studio 0.747: cost.rs COLLISION_GROUP_PART, COLLISION_GROUP_CHANGED): 1.7 ns a part whose id stays and
/// 0.38 us more for each whose id changes, so a register or unregister beside 5,000 parts reads 8.5 us more than beside none,
/// and with 1,000 of them in the group 380 more again (each read a flat 0.08 us). CollisionGroupSetCollidable works out no ids
/// and stays at its call's price.
#[test]
fn a_collision_group_change_costs_what_a_server_spends_on_the_workspaces_parts() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local function Each(Change)
            local N = 10
            local Start = os.clock()
            for _ = 1, N do
                Change()
            end
            local Middle = os.clock()
            for _ = 1, N do
            end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local function Toggle()
            PhysicsService:RegisterCollisionGroup("A")
            PhysicsService:UnregisterCollisionGroup("A")
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        local Alone = Each(Toggle)
        Within("a register and an unregister with no parts", Alone, 0.1, 0.4)
        local Folder = Instance.new("Folder")
        for _ = 1, 5000 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Parent = Folder
        end
        Folder.Parent = workspace
        Within("a change beside 5,000 parts", (Each(Toggle) - Alone) / 2, 8.3, 8.7)
        for Index, Part in Folder:GetChildren() do
            if Index <= 1000 then
                Part.CollisionGroup = "A"
            end
        end
        Within("a change of 1,000 of their ids", (Each(Toggle) - Alone) / 2, 8.5 + 370, 8.5 + 390)
        Within("a collidable pair", Each(function()
            PhysicsService:CollisionGroupSetCollidable("Default", "Default", true)
        end), 0.0, 0.3)
        "#,
    );
}

/// A tree coming into the Workspace works out its parts' CollisionGroupId, at what a group change spends on them (cost.rs
/// COLLISION_GROUP_PART, COLLISION_GROUP_CHANGED): 1,000 parts of a registered group, whose ids change from Default's as they come
/// in, read 380 us more than 1,000 of Default, whose ids stay. Taken out and brought back, they keep their ids and read alike. The
/// ids used to change at no charge.
#[test]
fn parts_coming_into_the_workspace_pay_for_the_collision_group_ids_they_take() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local RS = game:GetService("ReplicatedStorage")
        PhysicsService:RegisterCollisionGroup("A")
        local function Model(Group)
            local M = Instance.new("Model")
            for _ = 1, 1000 do
                local Part = Instance.new("Part")
                Part.Anchored = true
                Part.CollisionGroup = Group
                Part.Parent = M
            end
            M.Parent = RS
            return M
        end
        local Grouped, Plain = Model("A"), Model("Default")
        -- (the frame's first entry into the Workspace runs cold: cost.rs Path::Enter)
        Instance.new("Part").Parent = workspace
        local function Enter(M)
            local Start = os.clock()
            M.Parent = workspace
            return (os.clock() - Start) * 1e6
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        Within("1,000 ids that change", Enter(Grouped) - Enter(Plain), 379.9, 380.1)
        assert(Grouped:GetChildren()[1].CollisionGroupId == 1, "the group's id")
        Grouped.Parent, Plain.Parent = RS, RS
        Within("1,000 ids that stay", Enter(Grouped) - Enter(Plain), -0.1, 0.1)
        "#,
    );
}

/// A group change works out the ids of the parts the spatial queries' broadphase holds, the Workspace's, and of the Terrain:
/// the Workspace's other instances are not looked at, as a server's change costs next to nothing more beside 100,000 Folders
/// (roblox-sim, Studio 0.747, a server Script, a register and an unregister timed together, the median of 11 samples: 1.8-3.4
/// us beside nothing, 4.0-6.8 beside 100,000 Folders flat or in chains of 100, 5.5 beside 1,000 parts and 8.4-9.5 beside both,
/// 94 beside 20,000 parts). 100 changes beside 100,000 Folders and 10 parts look at those 10 parts each, and the broadphase,
/// made by the first change, walks nothing more; each change walked every instance of the Workspace before.
#[test]
fn a_collision_group_change_looks_at_the_workspaces_parts_alone() {
    use super::RunContext;
    use crate::{
        services::physics::LOOKED_AT,
        world::broadphase::{SYNC_VISITS, WORKSPACE_WALKS},
    };
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        for _ = 1, 100000 do
            Instance.new("Folder").Parent = Folder
        end
        for _ = 1, 10 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Parent = Folder
        end
        Folder.Parent = workspace
        game:GetService("PhysicsService"):RegisterCollisionGroup("Kept")
        "#,
    );
    LOOKED_AT.with(|looked| looked.set(0));
    WORKSPACE_WALKS.with(|walks| walks.set(0));
    SYNC_VISITS.with(|visits| visits.set(0));
    let output = run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        for _ = 1, 50 do
            PhysicsService:RegisterCollisionGroup("A")
            PhysicsService:UnregisterCollisionGroup("A")
        end
        print(#workspace:GetDescendants())
        "#,
    );
    assert_eq!(output.last().map(String::as_str), Some("print: 100012"), "the Workspace's instances");
    assert_eq!(LOOKED_AT.with(|looked| looked.get()), 100 * 10, "the 10 parts, 100 times");
    assert_eq!(WORKSPACE_WALKS.with(|walks| walks.get()), 0, "no walk of the Workspace");
    assert_eq!(SYNC_VISITS.with(|visits| visits.get()), 0, "nothing moved");
}

/// A group change fires its parts' CollisionGroupId changes in the order the Workspace's descendants are listed, as the walk of
/// the Workspace it replaced did, whatever order the broadphase holds them in: parts put in folders attached before them, below
/// a part, 50 folders deep, with bounds that are not finite, moved and destroyed once the broadphase was made, and the Terrain;
/// a part that never came into the Workspace keeps the id it was made with, and a part in a WorldModel keeps the id its
/// WorldModel's own groups gave it, though the broadphase holds it.
#[test]
fn a_collision_group_change_fires_its_parts_changes_in_the_workspaces_order() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        PhysicsService:RegisterCollisionGroup("A")
        local Label, Order = {}, {}
        local function Watch(Part, Name)
            Label[Part] = Name
            Part:GetPropertyChangedSignal("CollisionGroupId"):Connect(function()
                table.insert(Order, Name)
            end)
        end
        local First, Second = Instance.new("Folder"), Instance.new("Model")
        First.Parent = workspace
        Second.Parent = workspace
        local World = Instance.new("WorldModel")
        World.Parent = First
        local Deep = First
        for _ = 1, 50 do
            local Folder = Instance.new("Folder")
            Folder.Parent = Deep
            Deep = Folder
        end
        local Parts = {}
        for Index = 1, 12 do
            local Part = Instance.new(if Index % 3 == 0 then "WedgePart" else "Part")
            Part.Anchored = true
            Part.CollisionGroup = "A"
            Watch(Part, "P" .. Index)
            Parts[Index] = Part
        end
        Parts[12].Parent = First
        Parts[1].Parent = Second
        Parts[2].Parent = First
        Parts[3].Parent = Parts[1]
        Parts[4].Parent = workspace
        Parts[5].Parent = World
        Parts[6].Position = Vector3.new(0 / 0, 0, 0)
        Parts[6].Parent = Second
        Parts[7].Parent = Deep
        Parts[8].Parent = Second
        Parts[9].Parent = game:GetService("ReplicatedStorage")
        Parts[10].Parent = First
        Parts[11].Parent = workspace
        workspace.Terrain.CollisionGroup = "A"
        Watch(workspace.Terrain, "Terrain")
        -- (the broadphase is made here; what follows reaches it through the dom's log)
        PhysicsService:RegisterCollisionGroup("B")
        Parts[8].Parent = First
        Parts[2].Parent = Second
        Parts[12]:Destroy()
        Parts[10].Parent = Deep
        local function Listed()
            local List = {}
            for _, Descendant in workspace:GetDescendants() do
                if Label[Descendant] and not Descendant:FindFirstAncestorWhichIsA("WorldModel") then
                    table.insert(List, Label[Descendant])
                end
            end
            return table.concat(List, " ")
        end
        local function Ids()
            local List = {}
            for Index = 1, 11 do
                table.insert(List, Parts[Index].CollisionGroupId)
            end
            table.insert(List, workspace.Terrain.CollisionGroupId)
            return table.concat(List, " ")
        end
        local function Change(Label, Make)
            table.clear(Order)
            Make()
            local Expected = Listed()
            task.defer(function()
                local Fired = table.concat(Order, " ")
                assert(Fired == Expected, `{Label}: {Fired} against {Expected}`)
                print(Label, Fired)
                print("ids", Ids())
            end)
        end
        task.defer(function()
            Change("unregistered", function()
                PhysicsService:UnregisterCollisionGroup("A")
            end)
            task.defer(function()
                Change("renamed", function()
                    PhysicsService:RenameCollisionGroup("B", "A")
                end)
            end)
        end)
        "#,
    );
    let order = "P7 P10 P8 P1 P3 P6 P2 P4 P11 Terrain";
    assert_eq!(
        output,
        [
            format!("print: unregistered {order}"),
            "print: ids -1 -1 -1 -1 -1 -1 -1 -1 0 -1 -1 -1".into(),
            format!("print: renamed {order}"),
            "print: ids 2 2 2 2 -1 2 2 2 0 2 2 2".into(),
        ]
    );
}

/// Putting a group change's changed parts in the Workspace's order walks up from them until their paths meet (broadphase.rs
/// sort_by_order), ree's own work, which costs the work clock as a spatial query's ordering does (cost.rs QUERY_ORDER_STEP past
/// QUERY_ORDER_FREE steps a part): two parts of a group, each at the bottom of a 1,000-deep chain of folders of its own, read
/// 40 us more a change than the same two in one folder, where the walk takes a step each and costs nothing. The walk of the
/// Workspace that ordered them before cost nothing however deep they were.
#[test]
fn ordering_a_group_changes_parts_deep_in_chains_of_their_own_costs_its_walk() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        PhysicsService:RegisterCollisionGroup("A")
        local function Grouped(Parent)
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.CollisionGroup = "A"
            Part.Parent = Parent
        end
        local function Chain(Depth)
            local Root = Instance.new("Folder")
            local At = Root
            for _ = 2, Depth do
                local Folder = Instance.new("Folder")
                Folder.Parent = At
                At = Folder
            end
            Grouped(At)
            return Root
        end
        local function Each()
            local N = 10
            local Start = os.clock()
            for _ = 1, N do
                PhysicsService:UnregisterCollisionGroup("A")
                PhysicsService:RegisterCollisionGroup("A")
            end
            local Middle = os.clock()
            for _ = 1, N do
            end
            return ((Middle - Start) - (os.clock() - Middle)) / N / 2 * 1e6
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        local Flat = Instance.new("Folder")
        Grouped(Flat)
        Grouped(Flat)
        Flat.Parent = workspace
        local Near = Each()
        Flat.Parent = nil
        local First, Second = Chain(1000), Chain(1000)
        First.Parent, Second.Parent = workspace, workspace
        Within("two parts 1,000 deep in chains of their own", Each() - Near, 39.5, 40.5)
        "#,
    );
}

/// The spatial index reads no part's collision group (world/query.rs: the queries model none), so a write of CollisionGroup or
/// CollisionGroupId does not log the part for it (dom.rs Dom::keep_unread): a group change that gives 1,000 parts new ids, and a
/// script's writes of both properties, leave its log empty, where they logged every part, which the next query or group change
/// read again. A write of another property still logs the part.
#[test]
fn collision_group_writes_leave_the_spatial_index_log_alone() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let logged = |engine: &Engine| engine.state.borrow().dom.watch.as_ref().map(|watch| watch.written.len());
    run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        PhysicsService:RegisterCollisionGroup("A")
        local Folder = Instance.new("Folder")
        Folder.Name = "Grouped"
        for _ = 1, 1000 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.CollisionGroup = "A"
            Part.Parent = Folder
        end
        Folder.Parent = workspace
        workspace:Raycast(Vector3.zero, Vector3.yAxis)
        PhysicsService:UnregisterCollisionGroup("A")
        assert(Folder:GetChildren()[1].CollisionGroupId == -1, "the ids changed")
        "#,
    );
    assert_eq!(logged(&engine), Some(0), "a group change's ids");
    run(
        &lua,
        &engine,
        r#"
        local Part = workspace.Grouped:GetChildren()[1]
        Part.CollisionGroup = "B"
        Part.CollisionGroupId = 7
        "#,
    );
    assert_eq!(logged(&engine), Some(0), "a script's writes of both");
    run(&lua, &engine, "workspace.Grouped:GetChildren()[1].Transparency = 0.5");
    assert_eq!(logged(&engine), Some(1), "another property's write");
}

/// A WorldModel's change log leaves out its parts' collision-group writes as the Workspace's does, before the Workspace's index
/// is made too (dom.rs Dom::keep_unread): a WorldModel in ReplicatedStorage queried once, with the Workspace never queried, keeps
/// its log empty through its own group change giving 100 parts new ids, a change of UseWorkspaceCollisionGroups working them out
/// again and a script's writes of both properties, where each logged the parts it wrote, which the next query read again. A
/// write of another property still logs the part, and the index still matches a walk of its world.
#[test]
fn collision_group_writes_leave_a_worldmodels_log_alone_before_the_workspace_is_indexed() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    run(
        &lua,
        &engine,
        r#"
        local World = Instance.new("WorldModel")
        World.Name = "World"
        World.Parent = game:GetService("ReplicatedStorage")
        for I = 1, 100 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.CollisionGroup = "A"
            Part.Position = Vector3.new(I * 3, 0, 0)
            Part.Parent = World
        end
        World:IsRegion3Empty(Region3.new(Vector3.new(-1, -1, -1), Vector3.new(1, 1, 1)))
        "#,
    );
    let world = {
        let state = engine.state.borrow();
        let store = state.dom.find_child(engine.world().game, b"ReplicatedStorage").expect("ReplicatedStorage");
        state.dom.find_child(store, b"World").expect("World")
    };
    let logged = || {
        let mut state = engine.state.borrow_mut();
        assert!(state.dom.watch.is_none(), "the Workspace's index is not made");
        let log = state.dom.world_log(world).expect("followed");
        (log.moved.len(), log.written.len())
    };
    assert_eq!(logged(), (0, 0), "taken by the query");
    run(
        &lua,
        &engine,
        r#"
        local World = game:GetService("ReplicatedStorage").World
        World:RegisterCollisionGroup("A")
        assert(World:GetChildren()[1].CollisionGroupId == 1, "the ids changed")
        "#,
    );
    assert_eq!(logged(), (0, 0), "a WorldModel's group change's ids");
    run(
        &lua,
        &engine,
        r#"
        local World = game:GetService("ReplicatedStorage").World
        World.UseWorkspaceCollisionGroups = true
        assert(World:GetChildren()[1].CollisionGroupId == -1, "the ids worked out again")
        "#,
    );
    assert_eq!(logged(), (0, 0), "UseWorkspaceCollisionGroups' ids");
    run(
        &lua,
        &engine,
        r#"
        local Part = game:GetService("ReplicatedStorage").World:GetChildren()[1]
        Part.CollisionGroup = "B"
        Part.CollisionGroupId = 7
        "#,
    );
    assert_eq!(logged(), (0, 0), "a script's writes of both");
    run(&lua, &engine, r#"game:GetService("ReplicatedStorage").World:GetChildren()[2].Transparency = 0.5"#);
    assert_eq!(logged(), (0, 1), "another property's write");
    crate::world::query::model_index_matches_a_walk(&engine, world).unwrap();
}

/// WorldRoot's collision-group methods on a playing client (Studio 0.748): the Workspace's read the DataModel's groups,
/// PhysicsService's, and may not change them; a WorldModel has groups of its own, Default alone at first, which a client
/// may change while the WorldModel is outside the DataModel (refused in it, before the name is checked), and a Clone of it
/// gets a copy. They reported themselves as not emulated and answered nil.
#[test]
fn world_roots_collision_groups_are_the_workspaces_and_each_world_models_own() {
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local function Groups(Root)
            local Parts = {}
            for _, Group in Root:GetRegisteredCollisionGroups() do
                table.insert(Parts, Group.name .. "=" .. Group.mask)
            end
            return table.concat(Parts, " ")
        end
        local function Try(...)
            local Ok, Problem = pcall(...)
            return if Ok then "ok" else Problem
        end
        print(workspace:GetMaxCollisionGroups(), Groups(workspace), workspace:IsCollisionGroupRegistered("Default"), workspace:CollisionGroupsAreCollidable("Default", "Nope"))
        print(Try(workspace.RegisterCollisionGroup, workspace, "WsA"), Try(workspace.CollisionGroupSetCollidable, workspace, "Default", "Default", false), Try(workspace.RenameCollisionGroup, workspace, "Default", "WsB"), Try(workspace.UnregisterCollisionGroup, workspace, "WsA"))
        local World = Instance.new("WorldModel")
        World:RegisterCollisionGroup("MdA")
        World:CollisionGroupSetCollidable("MdA", "Default", false)
        print(Groups(World), World:CollisionGroupsAreCollidable("Default", "MdA"), World:IsCollisionGroupRegistered("MdA"), workspace:IsCollisionGroupRegistered("MdA"), PhysicsService:IsCollisionGroupRegistered("MdA"), Groups(workspace))
        World:RegisterCollisionGroup("MdB")
        World:RenameCollisionGroup("MdB", "MdC")
        local Copy = World:Clone()
        Copy:UnregisterCollisionGroup("MdC")
        print(Groups(World), Groups(Copy), Groups(Instance.new("WorldModel")), World:GetMaxCollisionGroups())
        World:Destroy()
        World:RegisterCollisionGroup("MdD")
        print(Groups(World))
        local Kept = Instance.new("WorldModel")
        local Holder = Instance.new("Folder")
        Kept.Parent = Holder
        Kept:RegisterCollisionGroup("Loose")
        Holder.Parent = game:GetService("ReplicatedStorage")
        print(Try(Kept.RegisterCollisionGroup, Kept, ""), Try(Kept.UnregisterCollisionGroup, Kept, "Loose"), Try(Kept.CollisionGroupSetCollidable, Kept, "Default", "Default", false), Groups(Kept), Kept:IsCollisionGroupRegistered("Loose"))
        Holder.Parent = nil
        print(Try(Kept.RegisterCollisionGroup, Kept, "Out"), Groups(Kept))
        "#,
    );
    let refused = "This API can only be used on the server!";
    assert_eq!(
        output,
        [
            "print: 32 Default=-1 true true".to_string(),
            format!("print: {refused} {refused} {refused} {refused}"),
            "print: Default=-3 MdA=-2 false true false false Default=-1".into(),
            "print: Default=-3 MdA=-2 MdC=-1 Default=-3 MdA=-2 Default=-1 32".into(),
            "print: Default=-3 MdA=-2 MdC=-1 MdD=-1".into(),
            format!("print: {refused} {refused} {refused} Default=-1 Loose=-1 true"),
            "print: ok Default=-1 Loose=-1 Out=-1".into(),
        ]
    );
    let missing: Vec<String> = engine.state.borrow().output.missing.iter().map(|(name, _)| name.clone()).filter(|name| name.contains("CollisionGroup")).collect();
    assert!(missing.is_empty(), "{missing:?}");
}

/// WorldRoot's collision groups on a server (Studio 0.748): the Workspace's and PhysicsService's are one registry; a name
/// that is neither a string nor a number warns, the last argument first; a WorldModel with UseWorkspaceCollisionGroups
/// true warns once, by its full name, at its first call past the casts; its parts' CollisionGroupId follows its own groups
/// while it is in the DataModel (the Workspace's reach none of them), the Workspace's while UseWorkspaceCollisionGroups is
/// true, a change of which fires no Changed, and a WorldModel that comes in with it true fires Changed only for what its
/// own groups change.
#[test]
fn world_roots_collision_groups_on_a_server_share_the_registry_warn_and_follow_parts() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local function Groups(Root)
            local Parts = {}
            for _, Group in Root:GetRegisteredCollisionGroups() do
                table.insert(Parts, Group.name .. "=" .. Group.mask)
            end
            return table.concat(Parts, " ")
        end
        workspace:RegisterCollisionGroup("WsA")
        PhysicsService:RegisterCollisionGroup("PsA")
        workspace:CollisionGroupSetCollidable("WsA", "PsA", false)
        print(PhysicsService:IsCollisionGroupRegistered("WsA"), PhysicsService:CollisionGroupsAreCollidable("PsA", "WsA"), Groups(workspace), Groups(PhysicsService))
        workspace:UnregisterCollisionGroup("WsA")
        PhysicsService:UnregisterCollisionGroup("PsA")
        print("cast", workspace:CollisionGroupsAreCollidable(true, Vector3.one))
        local Warned = Instance.new("WorldModel")
        Warned.Name = "Warned"
        Warned.UseWorkspaceCollisionGroups = true
        Warned.Parent = workspace
        print("first", pcall(Warned.RegisterCollisionGroup, Warned, nil))
        print("then", Warned:GetMaxCollisionGroups(), Warned:GetMaxCollisionGroups())

        local World = Instance.new("WorldModel")
        World:RegisterCollisionGroup("Pa")
        local Fired = {}
        local function Part(Name, Group)
            local Made = Instance.new("Part")
            Made.Name = Name
            Made.CollisionGroup = Group
            Made.Changed:Connect(function(Property)
                if Property == "CollisionGroupId" then
                    table.insert(Fired, Name)
                end
            end)
            Made.Parent = World
            return Made
        end
        local A, W = Part("A", "Pa"), Part("W", "Pw")
        PhysicsService:RegisterCollisionGroup("Pw")
        local function Step(Label)
            local Ids = A.CollisionGroupId .. " " .. W.CollisionGroupId
            task.defer(function()
                print(Label, Ids, table.concat(Fired, ","))
                table.clear(Fired)
            end)
        end
        Step("outside")
        World.Parent = workspace
        Step("entered")
        World.UseWorkspaceCollisionGroups = true
        Step("uses-workspace")
        A.CollisionGroup = "Pw"
        Step("named")
        A.CollisionGroup = "Pa"
        World:RegisterCollisionGroup("Pc")
        workspace:UnregisterCollisionGroup("Pw")
        Step("registries")
        workspace:RegisterCollisionGroup("Pw")
        World.UseWorkspaceCollisionGroups = false
        Step("own-again")
        World.Parent = nil
        A.CollisionGroupId = 9
        World.UseWorkspaceCollisionGroups = true
        Step("away")
        World.Parent = workspace
        Step("came-back")
        "#,
    );
    let output: Vec<&str> = output.iter().map(String::as_str).collect();
    let warning = "UseWorkspaceCollisionGroups is true, so this WorldModel's own collision groups are not the ones used for its simulation. If you meant to access or change the simulated groups, set UseWorkspaceCollisionGroups to false or edit them on the Workspace.";
    assert_eq!(
        output,
        [
            "print: true false Default=-1 WsA=-5 PsA=-3 Default=-1 WsA=-5 PsA=-3",
            "warn: argument #2 expects a string, but Vector3 was passed",
            "warn: argument #1 expects a string, but boolean was passed",
            "print: cast true",
            "print: first false Argument 1 missing or nil",
            &format!("warn: Workspace.Warned: {warning}"),
            "print: then 32 32",
            // (World's first call while UseWorkspaceCollisionGroups is true: RegisterCollisionGroup("Pc"))
            &format!("warn: Workspace.WorldModel: {warning}"),
            "print: outside 0 0 ",
            "print: entered 1 -1 A,W",
            "print: uses-workspace -1 1 ",
            "print: named 1 1 A",
            "print: registries -1 1 A",
            "print: own-again 1 -1 ",
            "print: away 9 -1 A",
            "print: came-back -1 1 A",
        ]
    );
}

/// A WorldModel's own registry comes with its first change: a call that changes nothing (an unknown name unregistered or
/// renamed, a refused CollisionGroupSetCollidable, a flag set as it was) makes none, so a loopback client is sent none and
/// a Clone copies none, while one that is back to Default alone is still sent. A client sent Default alone, the
/// DataModel's or a WorldModel's, does not start its parts' tracking: groups it was sent start it, one rule for both.
#[test]
fn a_world_models_registry_comes_with_its_first_change_and_default_alone_starts_no_tracking() {
    use super::RunContext;
    use crate::services::physics;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        World:UnregisterCollisionGroup("Nope")
        World:RenameCollisionGroup("Nope", "Other")
        World:CollisionGroupSetCollidable("Default", "Default", true)
        print(pcall(World.CollisionGroupSetCollidable, World, "Default", "Nope", false))
        print(World:IsCollisionGroupRegistered("Default"), #World:Clone():GetRegisteredCollisionGroups())
        "#,
    );
    assert_eq!(output, ["print: false Both collision groups must be registered.", "print: true 1"]);
    assert!(physics::model_snapshots(&engine).is_empty());
    run(&lua, &engine, r#"workspace.WorldModel:RegisterCollisionGroup("Gone") workspace.WorldModel:UnregisterCollisionGroup("Gone")"#);
    let sent = physics::model_snapshots(&engine);
    assert_eq!(sent.len(), 1);
    let default_alone = sent[0].1.clone();

    let (client_lua, client) = start_with(Options { mode: "play".into(), ..Options::default() });
    run(&client_lua, &client, r#"Instance.new("WorldModel").Parent = workspace"#);
    let model = {
        let state = client.state.borrow();
        let world_model = client.reflection.class_id("WorldModel").unwrap();
        let workspace = client.world.get().unwrap().workspace;
        state.dom.children_of(workspace).iter().copied().find(|&id| state.dom.at(id).class == world_model).unwrap()
    };
    physics::apply_snapshot(&client, default_alone.clone());
    physics::apply_model_snapshot(&client, model, default_alone);
    assert!(!physics::tracking(&client));
    run(&lua, &engine, r#"workspace.WorldModel:RegisterCollisionGroup("Kept")"#);
    physics::apply_model_snapshot(&client, model, physics::model_snapshots(&engine)[0].1.clone());
    assert!(physics::tracking(&client));
    assert_eq!(run(&client_lua, &client, r#"print(workspace.WorldModel:IsCollisionGroupRegistered("Kept"))"#), ["print: true"]);
}

/// A WorldModel's own group change costs what the Workspace's does (cost.rs COLLISION_GROUP_PART, COLLISION_GROUP_CHANGED)
/// on the parts it works out again: its own, not the Workspace's nor those of a WorldModel inside it. The Workspace's change
/// takes its parts from the spatial queries' broadphase, which holds its WorldModels' parts too: it looks at each, at a part's
/// price, and works none of them out. A change of UseWorkspaceCollisionGroups costs the same on the WorldModel's parts.
#[test]
fn a_world_models_group_change_and_use_workspace_toggle_cost_what_a_change_does_on_its_parts() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local function Each(Change)
            local N = 10
            local Start = os.clock()
            for _ = 1, N do
                Change()
            end
            local Middle = os.clock()
            for _ = 1, N do
            end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        local function Parts(Count, Parent)
            local Folder = Instance.new("Folder")
            for _ = 1, Count do
                local Part = Instance.new("Part")
                Part.Anchored = true
                Part.Parent = Folder
            end
            Folder.Parent = Parent
            return Folder
        end
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        local Empty = Instance.new("WorldModel")
        Empty.Parent = workspace
        local function Toggle(Root, Name)
            return function()
                Root:RegisterCollisionGroup(Name)
                Root:UnregisterCollisionGroup(Name)
            end
        end
        local function Flip(Root)
            return function()
                Root.UseWorkspaceCollisionGroups = not Root.UseWorkspaceCollisionGroups
            end
        end
        local Alone = Each(Toggle(World, "A"))
        local SharedAlone = Each(Toggle(PhysicsService, "B"))
        local FlipAlone = Each(Flip(Empty))
        Within("a WorldModel's register and unregister with no parts", Alone, 0.1, 0.4)
        local Own = Parts(5000, World)
        Parts(5000, workspace)
        local Inner = Instance.new("WorldModel")
        Parts(5000, Inner)
        Inner.Parent = World
        Within("a WorldModel's change beside its 5,000 parts", (Each(Toggle(World, "A")) - Alone) / 2, 8.3, 8.7)
        Within("the Workspace's change beside its own 5,000 and its WorldModels' 10,000", (Each(Toggle(PhysicsService, "B")) - SharedAlone) / 2, 25.3, 25.7)
        for Index, Part in Own:GetChildren() do
            if Index <= 1000 then
                Part.CollisionGroup = "A"
            end
        end
        Within("a change of 1,000 of their ids", (Each(Toggle(World, "A")) - Alone) / 2, 8.5 + 370, 8.5 + 390)
        World:RegisterCollisionGroup("A")
        Within("UseWorkspaceCollisionGroups changing 1,000 of their ids", Each(Flip(World)) - FlipAlone, 8.5 + 370, 8.5 + 390)
        Within("a collidable pair", Each(function()
            World:CollisionGroupSetCollidable("A", "Default", true)
        end), 0.0, 0.3)
        "#,
    );
}

/// A tree that moves between the Workspace and a WorldModel, or comes into a WorldModel's world, works out its parts'
/// CollisionGroupId at what a group change spends on them (cost.rs COLLISION_GROUP_PART, COLLISION_GROUP_CHANGED), as a tree
/// coming into the Workspace does: 1,000 parts of a group whose id differs between the two worlds read 380 us more than 1,000
/// of Default, whose ids stay, each way and from outside the worlds. Into a WorldModel that uses the Workspace's groups they
/// read 760 us more (the WorldModel's own groups change each id, then the Workspace's change it back), and that second pass
/// costs 1.7 ns a part. The ids used to change at no charge.
#[test]
fn parts_moving_into_a_world_model_pay_for_the_collision_group_ids_they_take() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        local RS = game:GetService("ReplicatedStorage")
        -- (A is 2 in the Workspace's groups and 1 in each WorldModel's own)
        PhysicsService:RegisterCollisionGroup("X")
        PhysicsService:RegisterCollisionGroup("A")
        local function World(UseWorkspace)
            local W = Instance.new("WorldModel")
            W:RegisterCollisionGroup("A")
            W.UseWorkspaceCollisionGroups = UseWorkspace
            W.Parent = workspace
            return W
        end
        local Own, Shared = World(false), World(true)
        local function Model(Group)
            local M = Instance.new("Model")
            for _ = 1, 1000 do
                local Part = Instance.new("Part")
                Part.Anchored = true
                Part.CollisionGroup = Group
                Part.Parent = M
            end
            M.Parent = workspace
            return M
        end
        -- (the frame's first entry into the Workspace, which runs cold, is theirs: cost.rs Path::Enter)
        local Grouped, Plain = Model("A"), Model("Default")
        local function Move(M, To)
            local Start = os.clock()
            M.Parent = To
            return (os.clock() - Start) * 1e6
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        local function Id()
            return Grouped:GetChildren()[1].CollisionGroupId
        end
        assert(Id() == 2, "the Workspace's id")
        local PlainIntoOwn = Move(Plain, Own)
        Within("from the Workspace into a WorldModel, 1,000 ids that change", Move(Grouped, Own) - PlainIntoOwn, 379.9, 380.1)
        assert(Id() == 1, "the WorldModel's own id")
        Within("back into the Workspace", Move(Grouped, workspace) - Move(Plain, workspace), 379.9, 380.1)
        assert(Id() == 2, "the Workspace's id again")
        local PlainIntoShared = Move(Plain, Shared)
        Within("into a WorldModel that uses the Workspace's groups, both passes", Move(Grouped, Shared) - PlainIntoShared, 759.9, 760.1)
        assert(Id() == 2, "the Workspace's id, through the WorldModel's own")
        Within("the second pass on 1,000 parts", PlainIntoShared - PlainIntoOwn, 1.69, 1.71)
        Grouped.Parent, Plain.Parent = RS, RS
        Within("from outside the worlds into a WorldModel", Move(Grouped, Own) - Move(Plain, Own), 379.9, 380.1)
        assert(Id() == 1, "the WorldModel's own id again")
        "#,
    );
}

/// Whether a WorldModel outside the Workspace is in the DataModel is found by a walk up its ancestors, which a call, a part's
/// CollisionGroup write and a change of UseWorkspaceCollisionGroups pay for as IsDescendantOf does (cost.rs ANCESTOR_STEP, 1.2 ns
/// an ancestor): with the WorldModel at the bottom of 1,001 Folders in ReplicatedStorage rather than 1, a part's name write costs
/// 1.2 us more, a Register and Unregister 2.4 (each works the parts out again), a change of UseWorkspaceCollisionGroups 1.2, and
/// a client's refused Register 1.2. A client's Register and Unregister of a WorldModel at the bottom of a parentless chain of
/// 1,001 Folders rather than 1, which it may change, cost 2.4 us more: one walk each, which finds it outside the DataModel, so
/// no parts are looked for (two walks each cost 4.8). The warning a WorldModel with UseWorkspaceCollisionGroups true gives
/// names it at what GetFullName spends (cost.rs full_name: 4.2 ns a name, 0.14 a byte). The walk used to be free, and the name
/// too.
#[test]
fn a_world_models_walk_up_to_the_data_model_costs_what_its_ancestors_do() {
    use super::RunContext;
    const SETUP: &str = r#"
        local RS = game:GetService("ReplicatedStorage")
        -- (a WorldModel with a part, at the bottom of Depth Folders named F in ReplicatedStorage, or in no parent: Loose)
        local function Chain(Depth, Loose)
            local Top = Instance.new("Folder")
            Top.Name = "F"
            local At = Top
            for _ = 2, Depth do
                local Folder = Instance.new("Folder")
                Folder.Name = "F"
                Folder.Parent = At
                At = Folder
            end
            local World = Instance.new("WorldModel")
            World.Parent = At
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Parent = World
            if not Loose then
                Top.Parent = RS
            end
            return World, Part
        end
        local function Each(Change)
            local N = 10
            local Start = os.clock()
            for _ = 1, N do
                Change()
            end
            local Middle = os.clock()
            for _ = 1, N do
            end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        -- (each run once first: the frame's first of a kind runs cold)
        local function Deeper(Label, Shallow, Deep, Expected)
            Each(Shallow)
            Each(Deep)
            Within(Label, Each(Deep) - Each(Shallow), Expected - 0.01, Expected + 0.01)
        end
    "#;
    // (what 1,000 more ancestors cost a walk up, two walks, and the warning's 1,000 more names and 2,000 more bytes, in us)
    let step = 1000.0 * crate::cost::ANCESTOR_STEP * 1e6;
    let twice = 2.0 * step;
    let named = crate::cost::full_name(1000, 2000) * 1e6;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"{SETUP}
        local ShallowWorld, ShallowPart = Chain(1)
        local DeepWorld, DeepPart = Chain(1001)
        local function Write(Part)
            local Flip = false
            return function()
                Flip = not Flip
                Part.CollisionGroup = if Flip then "A" else "B"
            end
        end
        Deeper("a part's CollisionGroup write", Write(ShallowPart), Write(DeepPart), {step:e})
        local function Toggle(World)
            return function()
                World:RegisterCollisionGroup("A")
                World:UnregisterCollisionGroup("A")
            end
        end
        Deeper("a Register and an Unregister", Toggle(ShallowWorld), Toggle(DeepWorld), {twice:e})
        local function Flip(World)
            return function()
                World.UseWorkspaceCollisionGroups = not World.UseWorkspaceCollisionGroups
            end
        end
        Deeper("a change of UseWorkspaceCollisionGroups", Flip(ShallowWorld), Flip(DeepWorld), {step:e})
        -- (the warning: each first call of two fresh WorldModels that use the Workspace's groups, one 1,000 Folders deeper; its
        -- full name has 1,000 more names and 2,000 more bytes, "F." each)
        local function First(World)
            World.UseWorkspaceCollisionGroups = true
            local Start = os.clock()
            World:IsCollisionGroupRegistered("Default")
            return (os.clock() - Start) * 1e6
        end
        First((Chain(1)))
        local Shallow, Deep = First((Chain(1))), First((Chain(1001)))
        Within("the warning's full name", Deep - Shallow, {named:e} - 0.01, {named:e} + 0.01)
        "#
        ),
    );

    let (client_lua, client) = start_with(Options { mode: "play".into(), ..Options::default() });
    engine_lua::modules::interpret_all(&client_lua, &client);
    crate::scheduler::step(&client_lua, &client, Some(1.0 / 60.0)).expect("step");
    run(
        &client_lua,
        &client,
        &format!(
            r#"{SETUP}
        local ShallowWorld = Chain(1)
        local DeepWorld = Chain(1001)
        local function Refused(World)
            return function()
                assert(not pcall(World.RegisterCollisionGroup, World, "A"), "refused in the DataModel")
            end
        end
        Deeper("a client's refused Register", Refused(ShallowWorld), Refused(DeepWorld), {step:e})
        local function Toggle(World)
            return function()
                World:RegisterCollisionGroup("A")
                World:UnregisterCollisionGroup("A")
            end
        end
        Deeper("a client's Register and Unregister outside the DataModel", Toggle((Chain(1, true))), Toggle((Chain(1001, true))), {twice:e})
        "#
        ),
    );
}

/// A part's CollisionGroup write below WorldModels pays for the WorldModels above it what any part write pays, in
/// lua/instance.rs `assign` (`changed_levels`; cost.rs `world_levels`, `part_changes`). The write's setter is
/// services/physics.rs `write_name`, and `assign` charges the levels after it, for three changes when the setter changed the
/// part's CollisionGroupId and two when it did not, as a playing client counts them (Studio 0.748, in the Workspace and in
/// ReplicatedStorage alike). Below 30 nested WorldModels, against a part with none in the same place: a name with another id
/// costs `world_levels(30, 3)` more (3.402 us), and a name with the same id (two not registered) `world_levels(30, 2)` more
/// (2.268 us), in the Workspace and in ReplicatedStorage, where the walk up from the innermost WorldModel to the DataModel
/// costs more too (31 ancestors at cost.rs `ANCESTOR_STEP`: 37.2 ns), which finds the world whose groups give the part its id.
/// Below 30 in no parent a name with another id costs `world_levels(30, 2)` more, and the walk up its 29 ancestors: no world
/// there works its id out. The prices are cost.rs's, passed in. The Workspace's write that kept its id used to pay three
/// changes, and ReplicatedStorage's that changed it two.
#[test]
fn a_collision_group_write_below_nested_world_models_pays_for_them() {
    use super::RunContext;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    // (what a write below 30 WorldModels pays more, in us: a write that changes the part's id and one that keeps it, and the
    // walks up from the innermost WorldModel, to the DataModel and to the top of a chain in no parent)
    let levels = |id_changed: bool| crate::cost::world_levels(30, crate::cost::part_changes("CollisionGroup", id_changed)) * 1e6;
    let step = crate::cost::ANCESTOR_STEP * 1e6;
    run(
        &lua,
        &engine,
        &format!("shared.Changed, shared.Kept, shared.Walk, shared.LooseWalk = {:e}, {:e}, {:e}, {:e}", levels(true), levels(false), 31.0 * step, 29.0 * step),
    );
    run(
        &lua,
        &engine,
        r#"
        local RS = game:GetService("ReplicatedStorage")
        local function Part(Parent)
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Parent = Parent
            return Part
        end
        -- (a part at the bottom of Levels WorldModels, each in the one before, the outermost in Parent)
        local function Nested(Levels, Parent)
            local Top = Instance.new("WorldModel")
            local At = Top
            for _ = 2, Levels do
                local World = Instance.new("WorldModel")
                World.Parent = At
                At = World
            end
            local Bottom = Part(At)
            Top.Parent = Parent
            return Bottom
        end
        -- (A and B are not registered: the part's id stays -1 between them; Default's is 0)
        local function Write(Part, First, Second)
            local Flip = false
            return function()
                Flip = not Flip
                Part.CollisionGroup = if Flip then First else Second
            end
        end
        local function Each(Change)
            local N = 10
            local Start = os.clock()
            for _ = 1, N do
                Change()
            end
            local Middle = os.clock()
            for _ = 1, N do
            end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        -- (each run once first: the frame's first of a kind runs cold)
        local function Beside(Label, Plain, Below, Expected)
            Each(Plain)
            Each(Below)
            local Value = Each(Below) - Each(Plain)
            assert(Value >= Expected - 0.01 and Value <= Expected + 0.01, `{Label} (us): {Value}, not {Expected}`)
        end
        local function Both(Label, Parent, Changed, Kept)
            local Plain, Below = Part(Parent), Nested(30, Parent)
            Beside(`{Label}, a name with another id`, Write(Plain, "Default", "A"), Write(Below, "Default", "A"), Changed)
            Beside(`{Label}, a name with the same id`, Write(Plain, "A", "B"), Write(Below, "A", "B"), Kept)
        end
        Both("below 30 WorldModels in the Workspace", workspace, shared.Changed, shared.Kept)
        Both("below 30 WorldModels in ReplicatedStorage", RS, shared.Changed + shared.Walk, shared.Kept + shared.Walk)
        Both("below 30 WorldModels in no parent", nil, shared.Kept + shared.LooseWalk, shared.Kept + shared.LooseWalk)
        "#,
    );
}

/// On a loopback server a WorldModel's group changes in a frame log one marker, at the first (loopback/replicate.rs
/// Change::WorldGroups), and the flush sends its groups as the frame left them in that place (`physics::flushed_model`), saying
/// whether one of the changes left them other than Default alone (`Snapshot::touched`), as the DataModel's do: a frame that
/// registers and unregisters a group sends Default alone, touched, which starts a client's tracking, where Default alone
/// untouched (a WorldModel sent whole with no change since) does not. The next change logs a marker again; a WorldModel sent
/// whole in the same flush takes the frame's changes with its tree (`physics::sent_models`), so its marker sends nothing more.
#[test]
fn a_world_models_frame_of_group_changes_is_sent_once_and_says_whether_it_was_touched() {
    use super::RunContext;
    use crate::loopback::replicate::Change;
    use crate::services::physics;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine.state.borrow_mut().loopback_log = Some(Vec::new());
    run(
        &lua,
        &engine,
        r#"
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        for _ = 1, 25 do
            World:RegisterCollisionGroup("A")
            World:RenameCollisionGroup("A", "B")
            World:UnregisterCollisionGroup("B")
        end
        "#,
    );
    let markers = |engine: &Engine| -> Vec<crate::InstanceId> {
        let state = engine.state.borrow();
        state.loopback_log.as_ref().unwrap().iter().filter_map(|change| if let Change::WorldGroups(id) = change { Some(*id) } else { None }).collect()
    };
    let logged = markers(&engine);
    assert_eq!(logged.len(), 1, "one marker for the frame's changes");
    let model = logged[0];
    let snapshot = physics::flushed_model(&engine, model).expect("its groups");
    assert!(snapshot.touched && snapshot.names[1..].iter().all(Option::is_none) && snapshot.masks[0] == u32::MAX);
    assert!(physics::flushed_model(&engine, model).is_none(), "nothing more to send");
    physics::flush_done(&engine);
    let whole = physics::sent_models(&engine, [model].into_iter());
    assert!(whole.len() == 1 && whole[0].0 == 0 && !whole[0].1.touched, "sent whole, unchanged since: its snapshot decides alone");

    engine.state.borrow_mut().loopback_log = Some(Vec::new());
    run(&lua, &engine, r#"workspace.WorldModel:RegisterCollisionGroup("C")"#);
    assert_eq!(markers(&engine), [model], "the next change logs a marker again");
    let with_tree = physics::sent_models(&engine, [model].into_iter());
    assert!(with_tree.len() == 1 && with_tree[0].1.touched && with_tree[0].1.names[1].as_deref() == Some(&b"C"[..]), "a tree takes the frame's changes");
    assert!(physics::flushed_model(&engine, model).is_none(), "so its marker sends nothing more");

    let client_with = |snapshot: crate::services::physics::Snapshot| {
        let (client_lua, client) = start_with(Options { mode: "play".into(), ..Options::default() });
        run(&client_lua, &client, r#"Instance.new("WorldModel").Parent = workspace"#);
        let copy = {
            let state = client.state.borrow();
            let world_model = client.reflection.class_id("WorldModel").unwrap();
            let workspace = client.world.get().unwrap().workspace;
            state.dom.children_of(workspace).iter().copied().find(|&id| state.dom.at(id).class == world_model).unwrap()
        };
        physics::apply_model_snapshot(&client, copy, snapshot);
        physics::tracking(&client)
    };
    assert!(client_with(snapshot), "a touched frame starts the client's tracking");
    assert!(!client_with(whole[0].1.clone()), "Default alone, untouched, does not");
}

/// A loopback server's WorldModels with group changes to send (services/physics.rs Groups::unsent) keep no table a burst grew
/// them to (dom.rs clear_map, as clear_set empties a set): after a frame changed the groups of 100,000 WorldModels the client
/// has no copy of, each later flush that empties one such entry costs about what it cost before the burst, where it cleared
/// a table of 131,072 slots (0.5 us of host time a flush, 0.02 before), and the table is no larger than a frame of one change
/// needs. A frame that sends what it logged keeps its table, the size of what it logged, rather than dropping it and growing
/// it again the next frame. Each round's flush is timed alone and the fastest nine tenths averaged, so a host that pauses the
/// test does not decide it.
#[test]
fn a_burst_of_world_model_group_changes_leaves_the_loopback_flush_no_work_after_it() {
    use super::RunContext;
    use crate::loopback::replicate::Change;
    use crate::services::physics;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine.state.borrow_mut().loopback_log = Some(Vec::new());
    run(
        &lua,
        &engine,
        r#"
        local Store = game:GetService("ServerStorage")
        local function Worlds(Count)
            local Made = table.create(Count)
            for I = 1, Count do
                local World = Instance.new("WorldModel")
                World.Parent = Store
                Made[I] = World
            end
            return Made
        end
        local Burst, Steady, One = Worlds(100000), Worlds(5000), Worlds(1)
        local function Toggle(List)
            for _, World in List do
                if World:IsCollisionGroupRegistered("A") then
                    World:UnregisterCollisionGroup("A")
                else
                    World:RegisterCollisionGroup("A")
                end
            end
        end
        function shared.Round()
            Toggle(One)
        end
        function shared.Burst()
            Toggle(Burst)
        end
        function shared.Steady()
            Toggle(Steady)
        end
        "#,
    );
    // (a frame's changes and its flush: `send` stands for the client's copies, which take their groups and leave the table)
    let frame = |source: &str, send: bool| -> f64 {
        run(&lua, &engine, source);
        let markers: Vec<crate::InstanceId> = {
            let mut state = engine.state.borrow_mut();
            let log = state.loopback_log.as_mut().unwrap();
            let markers = log.iter().filter_map(|change| if let Change::WorldGroups(id) = change { Some(*id) } else { None }).collect();
            log.clear();
            markers
        };
        if send {
            for id in markers {
                assert!(physics::flushed_model(&engine, id).is_some());
            }
        }
        let start = std::time::Instant::now();
        physics::flush_done(&engine);
        start.elapsed().as_secs_f64()
    };
    // (a flush's host time is a few ticks of the timer at most: many rounds, the slowest tenth left out)
    let typical = |rounds: usize| -> f64 {
        let mut times: Vec<f64> = (0..rounds).map(|_| frame("shared.Round()", false)).collect();
        times.sort_by(f64::total_cmp);
        let kept = rounds * 9 / 10;
        times[..kept].iter().sum::<f64>() / kept as f64
    };
    let before = typical(4000);
    frame("shared.Burst()", false);
    let after = typical(4000);
    eprintln!("a flush of one change took {:.3} us of host time before the burst, {:.3} us after it (the fastest 3,600 of 4,000)", before * 1e6, after * 1e6);
    assert!(after < 3.0 * before + 0.15e-6, "a flush of one change took {:.3} us of host time after 100,000 WorldModels' groups changed, {:.3} us before", after * 1e6, before * 1e6);
    let room = physics::unsent_room(&engine);
    assert!(room < 4096, "after the burst, a frame of one change leaves the table room for {room}");

    frame("shared.Steady()", true);
    let kept = physics::unsent_room(&engine);
    frame("shared.Steady()", true);
    assert!(kept >= 5000 && physics::unsent_room(&engine) == kept, "a frame of 5,000 changes sent keeps its table: room for {kept}, then {}", physics::unsent_room(&engine));
}

/// A WorldModel's group change reads no index of its parts: it walks its descendants for them and pays for each other instance
/// the walk looks at what a recursive find pays for a node (cost.rs FIND_NODE, 3.8 ns): beside 10,000 Folders a WorldModel's
/// Register and Unregister cost 76 us more and a change of its UseWorkspaceCollisionGroups 38, and a loopback client's copy that
/// takes groups with a new name pays 38 us for its walk and what a client spends on the part whose id it works out (cost.rs
/// CLIENT_GROUP_PART: the id stays). The Workspace's change takes its parts from the spatial queries' broadphase: it looks at
/// the WorldModels' two parts and at none of the Folders. The walk used to be free.
#[test]
fn a_world_models_walk_to_its_parts_pays_for_the_instances_it_looks_at() {
    use super::RunContext;
    use crate::services::physics;
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    const WORLD: &str = r#"
        local function World(Folders)
            local W = Instance.new("WorldModel")
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Parent = W
            for _ = 1, Folders do
                Instance.new("Folder").Parent = W
            end
            W.Parent = workspace
            return W
        end
    "#;
    run(
        &lua,
        &engine,
        &format!(
            r#"{WORLD}
        local function Each(Change)
            local N = 10
            local Start = os.clock()
            for _ = 1, N do
                Change()
            end
            local Middle = os.clock()
            for _ = 1, N do
            end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{{Label}} (us): {{Value}}`)
        end
        -- (each run once first: the frame's first of a kind runs cold)
        local function Beside(Label, Bare, Full, Expected)
            Each(Bare)
            Each(Full)
            Within(Label, Each(Full) - Each(Bare), Expected - 0.05, Expected + 0.05)
        end
        local Bare, Full = World(0), World(10000)
        Bare.Name, Full.Name = "Bare", "Full"
        local function Toggle(W)
            return function()
                W:RegisterCollisionGroup("A")
                W:UnregisterCollisionGroup("A")
            end
        end
        Beside("a Register and an Unregister beside 10,000 Folders", Toggle(Bare), Toggle(Full), 76)
        local function Flip(W)
            return function()
                W.UseWorkspaceCollisionGroups = not W.UseWorkspaceCollisionGroups
            end
        end
        Beside("a change of UseWorkspaceCollisionGroups beside them", Flip(Bare), Flip(Full), 38)
        Full:RegisterCollisionGroup("Kept")
        "#
        ),
    );
    let looked = || physics::LOOKED_AT.with(|looked| looked.get());
    let before = looked();
    run(&lua, &engine, r#"game:GetService("PhysicsService"):RegisterCollisionGroup("W") game:GetService("PhysicsService"):UnregisterCollisionGroup("W")"#);
    assert_eq!(looked() - before, 4, "the Workspace's two changes look at the WorldModels' two parts, and at no Folder");

    let snapshot = physics::model_snapshots(&engine).into_iter().map(|(_, snapshot)| snapshot).find(|snapshot| snapshot.names[1].as_deref() == Some(&b"Kept"[..])).expect("Full's groups");
    let (client_lua, client) = start_with(Options { mode: "play".into(), ..Options::default() });
    run(&client_lua, &client, &format!("{WORLD}\nWorld(10000)"));
    let copy = {
        let state = client.state.borrow();
        let world_model = client.reflection.class_id("WorldModel").unwrap();
        let workspace = client.world.get().unwrap().workspace;
        state.dom.children_of(workspace).iter().copied().find(|&id| state.dom.at(id).class == world_model).unwrap()
    };
    let work = || client.state.borrow().sched.work_total();
    let before = work();
    physics::apply_model_snapshot(&client, copy, snapshot);
    let spent = work() - before;
    assert!((spent - 10_000.0 * crate::cost::FIND_NODE - crate::cost::CLIENT_GROUP_PART).abs() < 1e-9, "the copy's walk and its part: {spent}");
}

/// A loopback client taking in a snapshot of its server's groups pays for the parts it works out what a 0.747 client spends on
/// them (cost.rs CLIENT_GROUP_PART, CLIENT_GROUP_CHANGED: about 1.9 us a part whose id changes, 30 ns one whose id stays): a
/// snapshot that registers the group 1,000 parts name costs 1.9 ms, one that adds a group no part names 30 us, one that changes
/// a mask alone nothing, and a snapshot of what the client holds nothing, looking at no part. The join's snapshot of a server
/// that never touched its groups, Default alone, leaves a client that never touched its own as it was: no part is looked at
/// and no spatial index made. The parts used to cost a client nothing, and the join's Default alone looked at every part.
#[test]
fn a_loopback_client_pays_for_the_parts_a_snapshot_of_its_servers_groups_works_out() {
    use super::RunContext;
    use crate::services::physics;
    let (server_lua, server) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    let untouched = physics::snapshot(&server).expect("the server's groups");
    run(&server_lua, &server, r#"game:GetService("PhysicsService"):RegisterCollisionGroup("G")"#);
    let registered = physics::snapshot(&server).expect("the server's groups");
    run(&server_lua, &server, r#"game:GetService("PhysicsService"):RegisterCollisionGroup("H")"#);
    let other = physics::snapshot(&server).expect("the server's groups");
    run(&server_lua, &server, r#"game:GetService("PhysicsService"):CollisionGroupSetCollidable("G", "H", false)"#);
    let masked = physics::snapshot(&server).expect("the server's groups");

    const PARTS: &str = r#"
        for I = 1, 1000 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Position = Vector3.new(I * 3, 0, 0)
            Part.Name = "P" .. I
            Part.Parent = workspace
        end
    "#;
    // (a client that never touched collision groups: the join's Default alone changes nothing)
    let (fresh_lua, fresh) = start_with(Options { mode: "play".into(), ..Options::default() });
    run(&fresh_lua, &fresh, PARTS);
    let looked = || physics::LOOKED_AT.with(|looked| looked.get());
    let work = |engine: &Engine| engine.state.borrow().sched.work_total();
    let indexed = || fresh.state.borrow().dom.watch.is_some();
    let (before, spent, was_indexed) = (looked(), work(&fresh), indexed());
    physics::apply_snapshot(&fresh, untouched.clone());
    assert_eq!((looked() - before, work(&fresh) - spent), (0, 0.0), "the join's Default alone looks at no part");
    assert!(!physics::tracking(&fresh) && indexed() == was_indexed, "nor tracks them, nor makes the spatial index");

    let (client_lua, client) = start_with(Options { mode: "play".into(), ..Options::default() });
    run(&client_lua, &client, &format!(r#"{PARTS}
        for _, Part in workspace:GetChildren() do if Part:IsA("Part") then Part.CollisionGroup = "G" end end"#));
    let paid = |snapshot: &physics::Snapshot| {
        let (before, spent) = (looked(), work(&client));
        physics::apply_snapshot(&client, snapshot.clone());
        (looked() - before, work(&client) - spent)
    };
    let (seen, spent) = paid(&registered);
    let expected = 1000.0 * (crate::cost::CLIENT_GROUP_PART + crate::cost::CLIENT_GROUP_CHANGED);
    // (the Workspace's Terrain is looked at too)
    assert!(seen >= 1000 && (spent - expected).abs() < 1e-7, "the group 1,000 parts name registered: {seen} looked at, {spent} s where {expected} expected");
    let (seen, spent) = paid(&other);
    assert!(seen >= 1000 && (spent - 1000.0 * crate::cost::CLIENT_GROUP_PART).abs() < 1e-7, "a group no part names: {seen} looked at, {spent} s");
    assert_eq!(paid(&masked), (0, 0.0), "a mask alone");
    assert_eq!(paid(&masked), (0, 0.0), "what the client holds");
    run(&client_lua, &client, r#"assert(workspace.P1.CollisionGroupId == 1, `the ids follow the groups: {workspace.P1.CollisionGroupId}`)"#);
}

/// On a loopback server a frame's collision-group changes reach the client as one snapshot of the registry as the frame left it,
/// in the place of the first change (loopback/replicate.rs flush): 101 changes log one marker (each logged a snapshot of its own,
/// so the client took every part's id in 101 times), the flush reads the registry then, and the next change logs a marker again.
#[test]
fn a_frames_collision_group_changes_log_one_snapshot_on_a_loopback_server() {
    use super::RunContext;
    use crate::loopback::{bridge::Msg, replicate::Change};
    let (lua, engine) = start_with(Options { run_context: RunContext::Server, ..Options::default() });
    engine.state.borrow_mut().loopback_log = Some(Vec::new());
    let logged = |engine: &Engine| engine.state.borrow().loopback_log.as_ref().unwrap().iter().filter(|change| matches!(change, Change::CollisionGroups | Change::Message(Msg::CollisionGroups(_)))).count();
    run(
        &lua,
        &engine,
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        for _ = 1, 25 do
            PhysicsService:RegisterCollisionGroup("A")
            PhysicsService:CollisionGroupSetCollidable("A", "Default", false)
            PhysicsService:RenameCollisionGroup("A", "B")
            PhysicsService:UnregisterCollisionGroup("B")
        end
        PhysicsService:RegisterCollisionGroup("Kept")
        "#,
    );
    assert_eq!(logged(&engine), 1, "one marker for the frame's changes");
    let snapshot = crate::services::physics::flushed(&engine).expect("the frame's snapshot");
    let names: Vec<Option<&[u8]>> = snapshot.names.iter().map(Option::as_deref).collect();
    assert_eq!(&names[..3], [Some(&b"Default"[..]), Some(&b"Kept"[..]), None]);
    assert!(snapshot.touched && snapshot.masks.iter().all(|&mask| mask == u32::MAX));
    assert!(crate::services::physics::flushed(&engine).is_none(), "nothing more to send");
    engine.state.borrow_mut().loopback_log.as_mut().unwrap().clear();
    run(&lua, &engine, r#"game:GetService("PhysicsService"):CollisionGroupSetCollidable("Kept", "Default", false)"#);
    assert_eq!(logged(&engine), 1, "the next frame's change");
}

/// settings() is refused to a game script, which a play run's scripts are, and the settings services are no children of the
/// DataModel's or of UserSettings()' (measured in a 0.747 client and server: tools/parity/probes/global_settings.luau). It was a
/// table of tables, and GetService made a settings service, or any other, under either.
#[test]
fn settings_is_refused_to_a_game_script_and_no_other_provider_holds_a_settings_service() {
    let (lua, engine) = start_with(Options { mode: "play".into(), ..Options::default() });
    let output = run(
        &lua,
        &engine,
        r#"
        local Ok, Problem = pcall(settings)
        assert(not Ok and Problem == "The current thread cannot call 'settings' (lacking capability Plugin)", tostring(Problem))
        assert(select(2, pcall(settings, 1, "x")) == Problem)
        -- the DataModel: a settings service a game script may not have is nil, another refused
        assert(game:GetService("RenderSettings") == nil and game:FindService("RenderSettings") == nil)
        assert(select(2, pcall(game.GetService, game, "TaskScheduler")) == "Invalid parent for Service. TaskScheduler cannot be parented to " .. game.Name .. " .")
        assert(select(2, pcall(game.GetService, game, "UserGameSettings")) == "Invalid parent for Service. UserGameSettings cannot be parented to " .. game.Name .. " .")
        assert(game:GetService("GlobalSettings") == nil and game:FindService("UserSettings") == nil)
        -- UserSettings() holds its own service and makes no other
        local User = UserSettings()
        assert(User:GetService("Players") == nil and User:GetService("RenderSettings") == nil and User:GetService("StudioService") == nil)
        assert(select(2, pcall(User.GetService, User, "Lighting")) == "Not allowed to add that under settings")
        assert(select(2, pcall(User.GetService, User, "LuaSettings")) == "Invalid parent for Service. LuaSettings cannot be parented to GlobalBasicSettings .")
        assert(select(2, pcall(User.GetService, User, "NetworkClient")) == "The current thread cannot create 'NetworkClient' (lacking capability Plugin)")
        assert(select(2, pcall(User.FindService, User, "NotAService")) == "'NotAService' is not a valid Service name")
        assert(#User:GetChildren() == 1)
        -- its tree is locked
        assert(select(2, pcall(function() User.Parent = workspace end)) == "The current thread cannot set 'UserSettings parent' (lacking capability RobloxScript)")
        assert(select(2, pcall(function() User.Parent = User end)) == "Attempt to set GlobalBasicSettings as its own parent")
        assert(select(2, pcall(function() User.GameSettings.Parent = workspace end)) == "Cannot change Parent of type UserGameSettings")
        assert(select(2, pcall(function() User.GameSettings.Parent = User end)) == "Cannot change Parent of type UserGameSettings")
        assert(select(2, pcall(function() Instance.new("Folder").Parent = User end)) == "Not allowed to add that under settings")
        assert(select(2, pcall(function() Instance.new("Folder").Parent = User.GameSettings end)) == "Not allowed to add that under settings")
        assert(select(2, pcall(User.Clone, User)) == "GlobalBasicSettings cannot be cloned")
        User.GameSettings.Name = "Renamed"
        assert(User.GameSettings.Name == "GameSettings")
        "#,
    );
    assert!(output.iter().all(|line| !line.starts_with("error")), "{output:?}");
}

/// In edit mode, where a plugin's code runs, settings() is the GlobalSettings object, the same every call, with its eight
/// settings services under a client's names and their values (measured at identity 4: tools/parity/probes/global_settings_tree.luau
/// and global_settings_services.luau). What ree does not emulate reports itself.
#[test]
fn settings_is_the_global_settings_object_a_plugin_gets() {
    let (lua, engine) = start();
    let output = run(
        &lua,
        &engine,
        r#"
        local Root = settings()
        assert(typeof(Root) == "Instance" and Root.ClassName == "GlobalSettings" and Root.Name == "Global Settings" and tostring(Root) == "Global Settings")
        assert(settings() == Root and Root:GetFullName() == "Global Settings" and Root.Parent == nil and Root:IsA("GenericSettings") and not Root:IsA("UserSettings"))
        local Names = {}
        for _, Child in Root:GetChildren() do
            table.insert(Names, Child.ClassName .. ":" .. Child.Name)
        end
        assert(table.concat(Names, ",") == "GameSettings:Game Options,LuaSettings:Lua,DebugSettings:Diagnostics,PhysicsSettings:Physics,Studio:Studio,NetworkSettings:Network,RenderSettings:Rendering,TaskScheduler:Task Scheduler", table.concat(Names, ","))
        local Rendering = Root.Rendering
        assert(Root:GetService("RenderSettings") == Rendering and Root:FindService("RenderSettings") == Rendering and Rendering:GetFullName() == "Rendering")
        assert(Rendering.QualityLevel == Enum.QualityLevel.Automatic and Rendering.MeshCacheSize == 33554432 and Rendering.EnableFRM and Rendering:GetMaxQualityLevel() == 22)
        assert(Root.Physics.AllowSleep and Root.Diagnostics.TickCountPreciseOverride == Enum.TickCountSampleMethod.Precise and Root.Network.RandomizeJoinInstanceOrder)
        assert(select(2, pcall(function() return Root.NotAMember end)) == 'NotAMember is not a valid member of GlobalSettings "Global Settings"')
        assert(select(2, pcall(function() return Root.RenderSettings end)) == 'RenderSettings is not a valid member of GlobalSettings "Global Settings"')
        -- writes: an enum setting takes an item's value; what a plugin may not touch refuses; a dropped write keeps the value
        Rendering.QualityLevel = 3
        assert(Rendering.QualityLevel == Enum.QualityLevel.Level03)
        assert(select(2, pcall(function() Root.Network.HttpProxyURL = "x" end)) == "The current thread cannot write 'HttpProxyURL' (lacking capability RobloxScript)")
        assert(select(2, pcall(function() return Root.Physics.AreTimestepsShown end)) == "The current thread cannot read 'AreTimestepsShown' (lacking capability RobloxScript)")
        Root["Game Options"].VideoCaptureEnabled = false
        Rendering.Name = "Renamed"
        assert(Root["Game Options"].VideoCaptureEnabled and Rendering.Name == "Rendering")
        -- the tree is locked
        assert(select(2, pcall(function() Rendering.Parent = workspace end)) == "Cannot change Parent of type RenderSettings")
        assert(select(2, pcall(function() Root.Parent = workspace end)) == "The current thread cannot set 'settings parent' (lacking capability RobloxScript)")
        assert(select(2, pcall(function() Root.Parent = Root end)) == "Attempt to set Global Settings as its own parent")
        assert(select(2, pcall(function() Rendering.Parent = Root end)) == "Cannot change Parent of type RenderSettings")
        assert(select(2, pcall(function() Instance.new("Folder").Parent = Root end)) == "Not allowed to add that under settings")
        assert(select(2, pcall(Root.Clone, Root)) == "Global Settings cannot be cloned" and select(2, pcall(Rendering.Clone, Rendering)) == "Rendering cannot be cloned")
        assert(select(2, pcall(Root.Lua.Destroy, Root.Lua)) == "The Parent property of Lua is locked")
        assert(select(2, pcall(Root.Destroy, Root)) == "The Parent property of Game Options is locked" and #Root:GetChildren() == 8)
        -- the other providers' services
        assert(select(2, pcall(Root.GetService, Root, "UserGameSettings")) == "Invalid parent for Service. UserGameSettings cannot be parented to Global Settings .")
        assert(select(2, pcall(Root.GetService, Root, "Lighting")) == "Not allowed to add that under settings")
        assert(Root:GetService("Workspace") == nil and #Root:GetChildren() == 8)
        assert(select(2, pcall(game.GetService, game, "RenderSettings")) == "Invalid parent for Service. RenderSettings cannot be parented to " .. game.Name .. " .")
        assert(select(2, pcall(UserSettings().GetService, UserSettings(), "Studio")) == "Invalid parent for Service. Studio cannot be parented to GlobalBasicSettings .")
        -- RenderSettings' writes fire no Changed, PhysicsSettings' do (read after the deferred events ran)
        _G.Seen = {}
        Rendering.Changed:Connect(function(Name)
            table.insert(_G.Seen, Name)
        end)
        Root.Physics.Changed:Connect(function(Name)
            table.insert(_G.Seen, Name)
        end)
        Rendering.QualityLevel = Enum.QualityLevel.Level07
        Root.Physics.AllowSleep = false
        -- a live reading, Studio's settings and the flag methods report themselves
        local _ = Root.Diagnostics.InstanceCount, Root.Studio["Tab Width"], Root:GetFFlag("Anything")
        "#,
    );
    assert!(output.iter().all(|line| !line.starts_with("error")), "{output:?}");
    assert_eq!(run(&lua, &engine, r#"print(table.concat(_G.Seen, ","))"#).last().map(String::as_str), Some("print: AllowSleep"));
    let missing: Vec<String> = engine.state.borrow().output.missing.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(missing, ["DebugSettings.InstanceCount", "Studio.Tab Width", "GlobalSettings.GetFFlag"]);
}

/// settings() makes its tree on its first call, so a run that never calls it numbers its instances as before
#[test]
fn settings_makes_its_tree_on_the_first_call() {
    let (lua, engine) = start();
    let class = engine.reflection.class_id("GlobalSettings").unwrap();
    run(&lua, &engine, "local _ = UserSettings ~= nil and workspace");
    assert_eq!(engine.state.borrow_mut().dom.count_of_class(class), 0);
    let before = engine.state.borrow().dom.live_count();
    run(&lua, &engine, "assert(settings() == settings())");
    // (the run's own script, the root and its eight services)
    assert_eq!(engine.state.borrow().dom.live_count(), before + 1 + 9);
    assert_eq!(engine.state.borrow_mut().dom.count_of_class(class), 1);
}

/// A method called with another value as self raises "Expected ':' not '.'" unless that value is an instance of the class that
/// declares the method, as a missing self does and before the arguments are looked at; a subclass's instance and an ancestor's
/// member take it (measured in a playing client, Studio 0.748).
#[test]
fn a_method_takes_only_its_declaring_classs_instances_as_self() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local function Fails(Name, F)
            local Ok, E = pcall(F)
            if Ok or E ~= "Expected ':' not '.' calling member function " .. Name then
                error(tostring(E), 2)
            end
        end
        local Part = Instance.new("Part")
        Part.Anchored = true
        Part.Parent = workspace
        local Mesh = Instance.new("MeshPart")
        Mesh.Anchored = true
        Mesh.Parent = workspace
        local Model = Instance.new("Model")
        Model.Parent = workspace
        local Cam = Instance.new("Camera")
        local Folder = Instance.new("Folder")
        Fails("GetRoll", function() return (Cam :: any).GetRoll(workspace) end)
        Fails("GetRoll", function() return (Cam :: any).GetRoll(Part) end)
        Fails("GetRoll", function() return (Cam :: any).GetRoll("x") end)
        Fails("WorldToViewportPoint", function() return (Cam :: any).WorldToViewportPoint(Part) end)
        Fails("GetMass", function() return (Part :: any).GetMass(workspace) end)
        Fails("GetMass", function() return (Part :: any).GetMass(Folder) end)
        Fails("ApplyImpulse", function() return (Part :: any).ApplyImpulse(Model, Vector3.zero) end)
        Fails("GetService", function() return (game :: any).GetService(workspace, "Players") end)
        Fails("GetPlayers", function() return (game:GetService("Players") :: any).GetPlayers(workspace) end)
        Fails("GetServerTimeNow", function() return (workspace :: any).GetServerTimeNow(Part) end)
        Fails("Raycast", function() return (workspace :: any).Raycast(game, Vector3.zero, Vector3.one) end)
        Fails("Create", function() return (game:GetService("TweenService") :: any).Create(workspace, Part, TweenInfo.new(), {}) end)
        -- a subclass's instance, and the members of an ancestor of the class, take any instance of it
        assert((Part :: any).GetMass(Mesh) == Mesh:GetMass() and (Mesh :: any).GetMass(Part) == Part:GetMass())
        assert((workspace :: any).Raycast(Instance.new("WorldModel"), Vector3.zero, Vector3.one) == nil)
        assert(game.FindFirstChild(workspace, "x") == nil and #workspace.GetChildren(Folder) == 0 and Folder.IsA(Part, "BasePart"))
        assert((Part :: any).Clone(Folder).ClassName == "Folder")
        "#,
    );
}

/// Real-client parity behaviors: Model:MoveTo ignores models outside the DataModel, BasePart.Color quantizes to u8/255,
/// collectgarbage / Instance.new(number) / EnumItem.__newindex / TweenService:Create type-mismatch error texts,
/// and playing-client ScriptContext.Error with script and formatted trace, a table error's as "Error occurred, no output from
/// Luau." (Studio 0.747, a playing client and server: tools/parity/probes/loopback/remote_callback_errors.luau's spawned rows).
#[test]
fn parity_behaviors_move_to_color_quantization_errors_and_client_script_context() {
    let options = Options { mode: "play".into(), ..Options::default() };
    let (lua, engine) = start_with(options);
    run(
        &lua,
        &engine,
        r#"
        -- Model:MoveTo outside DataModel is a no-op
        local LooseModel = Instance.new("Model")
        local LoosePart = Instance.new("Part")
        LoosePart.CFrame = CFrame.new(1, 2, 3)
        LoosePart.Parent = LooseModel
        LooseModel.PrimaryPart = LoosePart
        LooseModel:MoveTo(Vector3.new(20, 30, 40))
        assert(LoosePart.Position == Vector3.new(1, 2, 3), tostring(LoosePart.Position))

        -- BasePart.Color truncates each channel to floor(c * 255) / 255
        local P = Instance.new("Part")
        P.Color = Color3.new(0.5, 0.25, 0.999)
        assert(P.Color == Color3.fromRGB(127, 63, 254), tostring(P.Color))

        -- Instance.new(number), EnumItem.__newindex, TweenService type mismatch, collectgarbage
        local Ok1, Err1 = pcall(Instance.new, 5 :: any)
        assert(not Ok1 and Err1 == 'Unable to create an Instance of type "5"', tostring(Err1))
        local Ok2, Err2 = pcall(function() (Enum.Material.Plastic :: any).Value = 1 end)
        assert(not Ok2 and Err2 == "Value is not a valid member of EnumItem", tostring(Err2))
        local Ok3, Err3 = pcall(function()
            game:GetService("TweenService"):Create(P, TweenInfo.new(1), { Size = 5 })
        end)
        assert(not Ok3 and Err3 == "TweenService:Create property named 'Size' cannot be tweened due to type mismatch (property is a 'Vector3', but given type is 'double')", tostring(Err3))
        local Ok4, Err4 = pcall(collectgarbage)
        assert(not Ok4 and Err4 == "missing argument #1 to 'collectgarbage' (string expected)", tostring(Err4))
        local Ok5, Err5 = pcall(collectgarbage, "collect")
        assert(not Ok5 and Err5 == "collectgarbage must be called with 'count'; use gcinfo() instead", tostring(Err5))

        -- Playing-client ScriptContext.Error fires with real script and formatted trace; a non-string error with its stand-in text
        local Seen = {}
        game:GetService("ScriptContext").Error:Connect(function(Msg, Trace, Src)
            table.insert(Seen, { Msg = Msg, Trace = Trace, HasScript = typeof(Src) == "Instance" })
        end)
        _G.SeenErrors = Seen
        task.spawn(function()
            error("client boom", 0)
        end)
        task.spawn(function()
            error({ code = 1 })
        end)
        "#,
    );
    scheduler::run_until(&lua, &engine, 0.1).unwrap();
    let out = run(
        &lua,
        &engine,
        r#"
        local Seen = _G.SeenErrors
        assert(#Seen == 2, tostring(#Seen))
        assert(Seen[1].Msg == "client boom", tostring(Seen[1].Msg))
        assert(Seen[2].Msg == "Error occurred, no output from Luau.", tostring(Seen[2].Msg))
        assert(Seen[1].HasScript == true)
        assert(string.find(Seen[1].Trace, ", line ", 1, true) ~= nil, tostring(Seen[1].Trace))
        print("ok")
        "#,
    );
    assert!(out.contains(&"print: ok".to_string()), "{out:?}");
    let all = lines(&engine);
    assert!(all.iter().any(|l| l.contains("Error occurred, no output from Luau.")), "{all:?}");
}

/// Every engine function a script reads is one C function named as a client names it, with Luau's arity of a C function
/// (measured, Studio 0.747, a playing client and a server: tools/parity/probes/engine_functions.luau): a global, a library's
/// function, a datatype's constructor and method, an enum's, an Instance's and a service's method read as values (one written
/// in the engine's Luau too), a signal's. The old code served most of them as the prelude's Luau (`rur.prelude` with its lines,
/// a wrapper of no name), the service stubs' Luau or unnamed mlua functions.
#[test]
fn engine_functions_are_named_c_functions() {
    assert_script(
        r#"
        local Part = Instance.new("Part")
        local Cases = {
            { print, "print" }, { warn, "warn" }, { require, "require" }, { wait, "wait" }, { Wait, "Wait" }, { spawn, "spawn" },
            { Spawn, "Spawn" }, { delay, "delay" }, { tick, "tick" }, { time, "time" }, { elapsedTime, "elapsedTime" },
            { settings, "settings" }, { UserSettings, "UserSettings" }, { version, "version" }, { collectgarbage, "collectgarbage" },
            { loadstring, "loadstring" }, { task.wait, "wait" }, { task.spawn, "spawn" }, { task.defer, "defer" },
            { task.delay, "delay" }, { task.cancel, "cancel" }, { task.synchronize, "synchronize" }, { coroutine.create, "create" },
            { coroutine.wrap, "wrap" }, { debug.profilebegin, "profilebegin" }, { utf8.graphemes, "graphemes" },
            { utf8.nfcnormalize, "nfcnormalize" }, { os.clock, "clock" }, { os.time, "time" }, { os.date, "date" },
            { Instance.fromExisting, "fromExisting" }, { Vector3.new, "new" }, { Vector3.fromNormalId, "fromNormalId" },
            { CFrame.new, "new" }, { CFrame.Angles, "Angles" }, { Color3.fromRGB, "fromRGB" }, { UDim2.fromOffset, "fromOffset" },
            { BrickColor.new, "new" }, { PhysicalProperties.new, "newPhysicalPropertiesWithClampedMsgClosure" },
            { TweenInfo.new, "new" }, { Random.new, "new" }, { DateTime.now, "now" }, { Font.new, "new" }, { RaycastParams.new, "new" },
            { Path2DControlPoint.new, "new" }, { SharedTable.new, "new" }, { SharedTable.size, "size" },
            { CFrame.new().Lerp, "Lerp" }, { Color3.new().ToHex, "ToHex" }, { Vector3.one.FuzzyEq, "FuzzyEq" },
            { Random.new(1).NextInteger, "NextInteger" }, { Enum.GetEnums, "GetEnums" }, { Enum.Material.GetEnumItems, "GetEnumItems" },
            { game.GetService, "GetService" }, { workspace.Raycast, "Raycast" }, { Part.GetPivot, "GetPivot" },
            { game:GetService("SharedTableRegistry").GetSharedTable, "GetSharedTable" },
            { game:GetService("LogService").GetLogHistory, "GetLogHistory" }, { Part.Touched.Connect, "Connect" },
        }
        for Index, Case in Cases do
            local Source, Line, Name = debug.info(Case[1], "sln")
            local Arity, Vararg = debug.info(Case[1], "a")
            assert(Source == "[C]" and Line == -1 and Name == Case[2] and Arity == 0 and Vararg, `{Index} {Case[2]}: {Source} {Line} {Name} {Arity} {Vararg}`)
        end
        "#,
    );
}

/// An error raised inside an engine function has no frame of its own, as a client's has none (measured, Studio 0.747:
/// engine_functions.luau): an xpcall handler's traceback goes from the handler straight to the script line that called it,
/// whether the function's work is Rust (task.spawn, SharedTable.size, a method read as a value), the engine's Luau (a service
/// method written in it, called with `.` or `:`, a datatype constructor, a vector method in both forms) or a C body (coroutine,
/// typeof), and an argument error is at that line; one raised in a callback the engine function calls (print's `__tostring`)
/// lists the callback's line first, raised from there, unprotected. The old code listed the prelude's Pass and wrapper, the
/// service stubs' lines, and raised task.spawn's, typeof's and collectgarbage's argument errors without a position.
#[test]
fn an_error_inside_an_engine_function_has_no_frame_of_its_own() {
    assert_script(
        r#"
        local Chunk = debug.info(1, "s")
        local Registry = game:GetService("SharedTableRegistry")
        local function Handler(Message)
            return { Message = tostring(Message), Trace = debug.traceback() }
        end
        local Cases = {
            { function() task.spawn(5) end, true },
            { function() task.delay("x", print) end, true },
            { function() SharedTable.size(5) end, true },
            { function() game.GetService(game, "NotAService") end, false },
            { function() Registry:GetSharedTable(nil) end, false },
            { function() Registry.GetSharedTable(Registry, nil) end, false },
            { function() CFrame.new("x") end, true },
            { function() Path2DControlPoint.new(5) end, true },
            { function() coroutine.create(5) end, true },
            { function() typeof() end, true },
            { function() collectgarbage("collect") end, true },
            { function() Vector3.one.FuzzyEq(Vector3.one, 5) end, true },
            { function() Vector3.one:FuzzyEq(5) end, true },
        }
        for Index, Case in Cases do
            local Line = debug.info(Case[1], "l")
            local _, Got = xpcall(Case[1], Handler)
            local Frames = string.split(Got.Trace, "\n")
            assert(string.find(Frames[1], "function Handler", 1, true) and Frames[2] == `{Chunk}:{Line}`, `{Index}: {Got.Trace}`)
            local Positioned = string.sub(Got.Message, 1, #Chunk + #tostring(Line) + 3) == `{Chunk}:{Line}: `
            assert(Positioned == Case[2], `{Index}: {Got.Message}`)
        end
        local Inner, Outer
        local Thrower = setmetatable({}, { __tostring = function() Inner = debug.info(1, "l") error("from tostring") end })
        local _, Got = xpcall(function() Outer = debug.info(1, "l") print(Thrower) end, Handler)
        local Frames = string.split(Got.Trace, "\n")
        assert(Frames[2] == `{Chunk}:{Inner} function __tostring` and Frames[3] == `{Chunk}:{Outer}`, Got.Trace)
        assert(Got.Message == `{Chunk}:{Inner}: from tostring`, Got.Message)
        "#,
    );
}

/// A callback an engine function calls from its own frame finds it one frame up, as a client's does (measured, Studio 0.747:
/// in a `__tostring` print calls, level 2 is `[C] print`), and a thread waiting in an engine function shows only the line that
/// called it. The old code's print was the prelude's Luau (Stringify, then Print, above the `__tostring`), and a waiting thread
/// listed the prelude's wait.
#[test]
fn engine_functions_are_the_frames_their_callbacks_and_waiters_see() {
    assert_script(
        r#"
        local Seen
        pcall(print, setmetatable({}, { __tostring = function() Seen = table.pack(debug.info(2, "sln")) error("stop") end }))
        assert(Seen[1] == "[C]" and Seen[2] == -1 and Seen[3] == "print", `{Seen[1]} {Seen[2]} {Seen[3]}`)
        local Chunk = debug.info(1, "s")
        local Event = Instance.new("BindableEvent")
        local Folder = Instance.new("Folder")
        local Waiters = {
            { function() task.wait(1) end, debug.info(1, "l") },
            { function() wait(1) end, debug.info(1, "l") },
            { function() Event.Event:Wait() end, debug.info(1, "l") },
            { function() Folder:WaitForChild("Missing", 1) end, debug.info(1, "l") },
            { function() Folder.WaitForChild(Folder, "Missing", 1) end, debug.info(1, "l") },
        }
        for Index, Waiter in Waiters do
            local Thread = task.spawn(Waiter[1])
            local Trace = string.gsub(debug.traceback(Thread), "\n$", "")
            assert(coroutine.status(Thread) == "suspended" and Trace == `{Chunk}:{Waiter[2]}`, `{Index}: {Trace}`)
        end
        "#,
    );
}

/// The engine functions that wait yield from their C function and return what the thread is resumed with, as before: task.wait
/// its elapsed time, wait two numbers, a signal's Wait the values fired, WaitForChild the child, and a require of a module that
/// another thread is loading waits for it, while one whose chunk waits keeps its requirer waiting.
#[test]
fn engine_functions_that_wait_yield_from_their_c_function() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r#"
        local Event = Instance.new("BindableEvent")
        local Folder = Instance.new("Folder")
        local Elapsed = task.wait(0.05)
        print("task.wait", type(Elapsed), Elapsed > 0)
        local A, B = wait(0.05)
        print("wait", type(A), type(B))
        task.delay(0.02, function() Event:Fire("fired", 2) end)
        print("signal", Event.Event:Wait())
        task.delay(0.02, function() local Child = Instance.new("Folder") Child.Name = "Child" Child.Parent = Folder end)
        print("child", Folder:WaitForChild("Child").Name, (Folder.WaitForChild(Folder, "Child").Name))
        local Module = Instance.new("ModuleScript")
        __setSource(Module, "task.wait(0.02) return 'late'")
        local Other
        task.spawn(function() Other = require(Module) end)
        print("module", require(Module), Other)
        "#,
    );
    for _ in 0..30 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let output = lines(&engine);
    for expected in ["print: task.wait number true", "print: wait number number", "print: signal fired 2", "print: child Child Child", "print: module late late"] {
        assert!(output.contains(&expected.to_string()), "{expected}: {output:?}");
    }
}

/// A script's call of an engine function is one call, whatever its work is: the Rust or engine Luau it runs for the call is part
/// of it (patches/luau/20-engine-functions.patch). A library's function (SharedTable's, print) is a C function's call and first
/// call in a frame (cost.rs LUAU_CCALL, LUAU_COLD_CFUNC), as a client's is, where the engine Luau that stood for print was a host
/// call; what the engine serves (a datatype's constructor, a service method read as a value) keeps the price of the engine Luau
/// that stood for it, a host call (LUAU_HOSTCALL, the measured price of a datatype's operation) and its first call (LUAU_COLD_HOSTFUNC).
#[test]
fn an_engine_function_is_one_call_priced_by_its_kind() {
    use crate::cost::{COLD_CFUNCS_SLOT, COLD_HOSTFUNCS_SLOT};
    let (lua, engine) = start();
    engine_lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    // (each function made a frame before: one a frame makes reads warm in it)
    run(&lua, &engine, "local _ = CFrame.new, SharedTable.new, SharedTable.size, game:GetService('SharedTableRegistry').GetSharedTable");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    // (each script waits for the next frame, which the test steps itself, counting what its calls take there)
    let measure = |calls: &str| {
        let call = start_script(&lua, &engine, &format!("task.wait()\n{calls}"));
        let before = vm_counts(&engine);
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let after = vm_counts(&engine);
        assert!(matches!(call.outcome().unwrap(), CallOutcome::Finished(_)), "the calls ran in the frame counted");
        (
            after.ccalls - before.ccalls,
            after.hostcalls - before.hostcalls,
            after.hostops[COLD_CFUNCS_SLOT] - before.hostops[COLD_CFUNCS_SLOT],
            after.hostops[COLD_HOSTFUNCS_SLOT] - before.hostops[COLD_HOSTFUNCS_SLOT],
        )
    };
    // (C calls, host calls, first calls of C functions, first calls of served ones)
    let constructor = measure("local C = CFrame.new(1, 2, 3) C = CFrame.new(3, 2, 1)");
    assert_eq!(constructor, (0, 2, 0, 1), "CFrame.new twice");
    let library = measure("local T = SharedTable.new() SharedTable.size(T) SharedTable.size(T) print(1) print(2)");
    assert_eq!(library, (5, 0, 3, 0), "SharedTable.new, size twice, print twice");
    let method = measure("local R = game:GetService('SharedTableRegistry') local Get = R.GetSharedTable Get(R, 'a') Get(R, 'b')");
    assert_eq!((method.0, method.1, method.3), (1, 2, 1), "GetService, then GetSharedTable read as a value twice: {method:?}");
}

/// What follows a wait in a test script runs and is checked: the helper steps frames until the script ends (an assert after a
/// task.wait() used to pass unchecked, the call left waiting).
#[test]
#[should_panic(expected = "checked after the wait")]
fn a_test_script_is_checked_after_it_waits() {
    assert_script("task.wait() assert(false, 'checked after the wait')");
}

/// A test script still waiting after a minute of frames fails its test (lua/globals.rs Call::finish).
#[test]
#[should_panic(expected = "still waiting")]
fn a_test_script_that_never_ends_fails_its_test() {
    assert_script("while true do task.wait() end");
}

/// EncodingService's hashes cost the work clock what its other members do: the price of a call the cost table does not name
/// (cost.rs CALL), whatever the input's size. A playing client spends more (Studio 0.747, a headless client: 0.32 to 0.67 us for a
/// 3-byte input, and 0.076 ns a byte for Blake3, 0.59 for Blake2b, 0.88 for Sha1, 1.2 for Md5 and 2.1 for Sha256), as it does for
/// CompressBuffer (1.4 us for 3 bytes) and Base64Encode (0.45 ns a byte), which ree prices the same way.
#[test]
fn hashes_cost_the_work_clock_what_other_encoding_calls_do() {
    let (lua, engine) = start();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let output = run(
        &lua,
        &engine,
        &format!(
            r#"{MOVE_HELPERS}
        local EncodingService = game:GetService("EncodingService")
        local Small, Large, Text = buffer.fromstring("abc"), buffer.create(100000), "abc"
        local Md5, Zstd = Enum.HashAlgorithm.Md5, Enum.CompressionAlgorithm.Zstd
        local Compress = Each(function() EncodingService:CompressBuffer(Small, Zstd) end)
        local Hash = Each(function() EncodingService:ComputeBufferHash(Small, Md5) end)
        local Long = Each(function() EncodingService:ComputeBufferHash(Large, Md5) end)
        local Hashed = Each(function() EncodingService:ComputeStringHash(Text, Md5) end)
        print(string.format("compress %.4f buffer %.4f large %.4f string %.4f", Compress, Hash, Long, Hashed))
        print("answers", typeof(EncodingService:ComputeBufferHash(Small, Md5)), typeof(EncodingService:ComputeStringHash(Text, Md5)))
        "#
        ),
    );
    assert!(output.contains(&"print: answers buffer string".to_string()), "{output:?}");
    let line = output.iter().find(|line| line.starts_with("print: compress")).expect("the prices");
    let prices: Vec<&str> = line.split(' ').skip(2).step_by(2).collect();
    assert!(prices.iter().all(|price| *price == prices[0]), "{line}");
    // (each the call's price and the Luau around it)
    let price: f64 = prices[0].parse().expect("a price");
    assert!((crate::cost::CALL * 1e6..0.2).contains(&price), "{line}");
    assert_eq!(crate::cost::method("ComputeBufferHash"), crate::cost::CALL);
    assert_eq!(crate::cost::method("ComputeStringHash"), crate::cost::CALL);
}
