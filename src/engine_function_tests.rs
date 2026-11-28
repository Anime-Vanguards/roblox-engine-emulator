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
    // (a test script lets its players in as a harness does: Context.Join, record/run.rs)
    env.set("Context", crate::record::run::players_context(&lua, &engine)).expect("Context");
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

/// Outside an Actor task.synchronize and desynchronize raise (where the thread can yield or not); ConnectParallel there raises
/// that the calling script must be under an Actor; a server's RenderStepped:Wait raises as its Connect does. A client's
/// RemoteFunction:InvokeClient checks the player, then refuses where it cannot yield, then refuses on a client; PreloadAsync of
/// nothing to load returns at once.
#[test]
fn calls_a_script_may_not_make_where_it_runs() {
    let client = run(
        RunContext::Client,
        r##"
        assert(select(2, pcall(task.synchronize)) == "task.synchronize() may only be called from a script that is a descendant of an Actor")
        assert(select(2, pcall(task.desynchronize)) == "task.desynchronize() may only be called from a script that is a descendant of an Actor")
        local Ok, Message = pcall(function() return game:GetService("RunService").Heartbeat:ConnectParallel(function() end) end)
        assert(not Ok and Message == "Event.ConnectParallel: The calling script must be rooted under an Actor.", Message)
        local Remote = Instance.new("RemoteFunction") Remote.Parent = workspace -- (a parentless one refuses nothing on the wrong side)
        local Player = game:GetService("Players").LocalPlayer
        assert(select(2, pcall(Remote.InvokeClient, Remote, nil)) == "Argument 1 missing or nil")
        assert(select(2, pcall(Remote.InvokeClient, Remote, workspace)) == "Argument 1 is invalid: expected 'Player' instance type")
        assert(select(2, pcall(Remote.InvokeClient, Remote, Player)) == "InvokeClient can only be called from the server")
        local Refused
        table.sort({ 2, 1 }, function(A, B)
            Refused = Refused or select(2, pcall(Remote.InvokeClient, Instance.new("RemoteFunction"), Player))
            return A < B
        end)
        assert(Refused == "thread is not yieldable", Refused)
        local Deferred = false
        task.defer(function() Deferred = true end)
        game:GetService("ContentProvider"):PreloadAsync({})
        assert(not Deferred, "PreloadAsync of nothing returns at once")
        print("done")
        "##,
        0.1,
    );
    assert!(client.iter().any(|line| line == "print: done") && errors(&client).is_empty(), "{client:?}");
    let server = run(
        RunContext::Server,
        r##"
        local Ok, Message = pcall(function() return game:GetService("RunService").RenderStepped:Wait() end)
        assert(not Ok and Message == "RenderStepped event can only be used from local scripts", Message)
        print("done")
        "##,
        0.1,
    );
    assert!(server.iter().any(|line| line == "print: done") && errors(&server).is_empty(), "{server:?}");
}

/// A memory category is the thread's own: it starts as its script's Name, set answers the one it replaces and takes a string or a
/// number, reset goes back to the Name, and a thread task.spawn, the legacy spawn or coroutine.create makes starts with its maker's.
#[test]
fn memory_categories_are_a_threads_own() {
    let lines = run(
        RunContext::Server,
        r##"
        assert(debug.getmemorycategory() == "Test")
        assert(debug.setmemorycategory("abc") == "Test" and debug.getmemorycategory() == "abc")
        assert(debug.setmemorycategory(5) == "abc" and debug.getmemorycategory() == "5")
        assert(select(2, pcall(debug.setmemorycategory, {})) == "invalid argument #1 to 'setmemorycategory' (string expected, got table)")
        assert(select(2, pcall(debug.setmemorycategory)) == "missing argument #1 to 'setmemorycategory' (string expected)")
        assert(select("#", debug.resetmemorycategory()) == 0 and debug.getmemorycategory() == "Test")
        debug.setmemorycategory("outer")
        local Seen = {}
        task.spawn(function()
            table.insert(Seen, debug.getmemorycategory())
            debug.setmemorycategory("inner")
        end)
        coroutine.resume(coroutine.create(function() table.insert(Seen, debug.getmemorycategory()) end))
        spawn(function() table.insert(Seen, debug.getmemorycategory()) end)
        task.wait(0.1)
        assert(table.concat(Seen, ",") == "outer,outer,outer" and debug.getmemorycategory() == "outer", table.concat(Seen, ","))
        assert(type(debug.dumpheap) == "function" and type(debug.dumprefs) == "function" and type(debug.tracerefs) == "function")
        print("done")
        "##,
        0.2,
    );
    assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{lines:?}");
}

/// A signal's, a connection's, a vector's, a Random's and an EnumItem's method read as a value is a new function at every read,
/// which still works; an Instance's and an Enum's are one function.
#[test]
fn method_values_are_new_functions_where_a_clients_are() {
    let lines = run(
        RunContext::Client,
        r##"
        local Event = Instance.new("BindableEvent")
        local Connection = Event.Event:Connect(function() end)
        local V = Vector3.new(1, 2, 3)
        local Rng = Random.new(1)
        assert(not rawequal(Event.Event.Connect, Event.Event.Connect) and not rawequal(Connection.Disconnect, Connection.Disconnect))
        assert(not rawequal(V.Lerp, V.Lerp) and not rawequal(V.Dot, V.Dot) and not rawequal(Rng.NextNumber, Rng.NextNumber))
        assert(not rawequal(Enum.Material.Plastic.IsA, Enum.Material.Plastic.IsA))
        assert(rawequal(workspace.FindFirstChild, workspace.FindFirstChild) and rawequal(Enum.Material.GetEnumItems, Enum.Material.GetEnumItems))
        assert(V.Unit == V.Unit and V.Magnitude > 0 and V.Lerp(Vector3.zero, Vector3.one, 0.5) == Vector3.one / 2)
        local Fired = 0
        Event.Event.Connect(Event.Event, function() Fired += 1 end)
        Event:Fire()
        task.wait()
        assert(Fired == 1)
        assert(select(2, pcall(function() return V.Nope end)) == "Nope is not a valid member of Vector3")
        local _, Problem = pcall(function() return V[1] end)
        assert(string.find(Problem, ":%d+: invalid argument #2 %(string expected, got number%)$"), Problem)
        print("done")
        "##,
        0.1,
    );
    assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{lines:?}");
}

/// A method written in the engine's Luau checks its self as the Rust methods do: nil, another value or an instance of another
/// class raises "Expected ':' not '.' calling member function <name>" before the arguments are looked at; a subclass's instance
/// passes.
#[test]
fn a_luau_method_checks_its_self() {
    let lines = run(
        RunContext::Client,
        r##"
        local Registry = game:GetService("SharedTableRegistry")
        local Part = Instance.new("Part")
        assert(select(2, pcall(Registry.GetSharedTable, Part, "x")) == "Expected ':' not '.' calling member function GetSharedTable")
        assert(select(2, pcall(Registry.GetSharedTable, nil, "x")) == "Expected ':' not '.' calling member function GetSharedTable")
        assert(select(2, pcall(Registry.GetSharedTable, "x", nil)) == "Expected ':' not '.' calling member function GetSharedTable")
        local Model = Instance.new("Model")
        assert(select(2, pcall(Model.GetPrimaryPartCFrame, Part)) == "Expected ':' not '.' calling member function GetPrimaryPartCFrame")
        assert(select(2, pcall(Model.GetBoundingBox, Part)) == "Expected ':' not '.' calling member function GetBoundingBox")
        local Ok, Problem = pcall(Model.GetBoundingBox, Instance.new("WorldModel"))
        assert(Ok or not string.find(Problem, "Expected ':'", 1, true), "a subclass's instance passes")
        print("done")
        "##,
        0.1,
    );
    assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{lines:?}");
}

/// Model:SetPrimaryPartCFrame puts the primary part at the CFrame exactly (its PivotOffset and the WorldPivot play no part), the
/// model's other parts moved with it; with no PrimaryPart, or one taken out of the model, both methods raise.
#[test]
fn primary_part_cframe_is_the_primary_parts_own() {
    let lines = run(
        RunContext::Client,
        r##"
        local Model = Instance.new("Model")
        local A, B = Instance.new("Part"), Instance.new("Part")
        A.Anchored, B.Anchored = true, true
        A.CFrame, B.CFrame = CFrame.new(1, 2, 3), CFrame.new(5, 2, 3)
        A.Parent, B.Parent = Model, Model
        Model.Parent = workspace
        local Message = "Model:SetPrimaryPartCFrame() failed because no PrimaryPart has been set, or the PrimaryPart no longer exists. Please set Model.PrimaryPart before using this."
        assert(select(2, pcall(Model.SetPrimaryPartCFrame, Model, CFrame.new())) == Message)
        assert(select(2, pcall(Model.GetPrimaryPartCFrame, Model)) == string.gsub(Message, "SetPrimaryPartCFrame", "GetPrimaryPartCFrame"))
        assert(select(2, pcall(Model.SetPrimaryPartCFrame, Model)) == "Argument 1 missing or nil")
        assert(select(2, pcall(Model.SetPrimaryPartCFrame, Model, "x")) == "Unable to cast string to CoordinateFrame")
        Model.PrimaryPart = A
        A.PivotOffset = CFrame.new(0, 1, 0) * CFrame.Angles(0, math.pi / 2, 0)
        Model:SetPrimaryPartCFrame(CFrame.new(0, 20, 0))
        assert(A.CFrame == CFrame.new(0, 20, 0) and B.CFrame == CFrame.new(4, 20, 0), tostring(A.CFrame) .. " / " .. tostring(B.CFrame))
        assert(Model:GetPrimaryPartCFrame() == CFrame.new(0, 20, 0))
        A.Parent = workspace
        assert(select(2, pcall(Model.SetPrimaryPartCFrame, Model, CFrame.new())) == Message)
        print("done")
        "##,
        0.1,
    );
    assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{lines:?}");
}

/// HumanoidDescription keeps its emotes (ids rounded, each once, ascending) and equipped emotes (names take slots 1, 2, ...; slot
/// tables kept, the later of a slot), fires their events, and a clone keeps them.
#[test]
fn humanoid_description_emotes() {
    let lines = run(
        RunContext::Client,
        r##"
        local D = Instance.new("HumanoidDescription")
        local Events = {}
        D.EmotesChanged:Connect(function(Value) table.insert(Events, "E") end)
        D.EquippedEmotesChanged:Connect(function(Value) table.insert(Events, "Q") end)
        D:SetEmotes({ Salute = { 3, 1.5, 1, "x" }, Empty = {}, Bad = "x" })
        D:AddEmote("Salute", 2.4)
        D:AddEmote(5, 6)
        D:RemoveEmote("Nope")
        D:RemoveEmote("5")
        local Emotes = D:GetEmotes()
        assert(table.concat(Emotes.Salute, ",") == "1,2,3" and Emotes.Empty == nil and Emotes.Bad == nil and Emotes["5"] == nil)
        assert(not rawequal(D:GetEmotes(), D:GetEmotes()))
        D:SetEquippedEmotes({ "A", { Slot = 5, Name = "B" }, "C", 7, { Slot = 2.5, Name = "D" } })
        local Equipped = D:GetEquippedEmotes()
        local Text = {}
        for _, Entry in Equipped do table.insert(Text, Entry.Slot .. Entry.Name) end
        assert(table.concat(Text, ",") == "1A,2C,3D,5B", table.concat(Text, ","))
        assert(select(2, pcall(D.SetEmotes, D, { { 1 } })) == "Unable to cast Array to Dictionary")
        assert(select(2, pcall(D.SetEquippedEmotes, D, { a = 1 })) == "Unable to cast Dictionary to Array")
        assert(select(2, pcall(D.AddEmote, D, "X", "x")) == "Unable to cast string to int64")
        assert(select(2, pcall(D.AddEmote, D, nil, 5)) == "Argument 1 missing or nil")
        local Copy = D:Clone()
        assert(table.concat(Copy:GetEmotes().Salute, ",") == "1,2,3" and #Copy:GetEquippedEmotes() == 4)
        task.wait()
        assert(table.concat(Events, "") == "EEEEQ", table.concat(Events, ""))
        print("done")
        "##,
        0.1,
    );
    assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{lines:?}");
}

/// AnalyticsService's Log methods, Chat:Chat and PromptSubscriptionPurchase cast their arguments from the last; the Log methods
/// then warn as a client and a server do.
#[test]
fn stubs_cast_their_arguments_from_the_last() {
    for (context, warning) in [(RunContext::Client, "warn: AnalyticsService can only be executed on the game server."), (RunContext::Server, "warn: AnalyticsService: Invalid status.")] {
        let lines = run(
            context,
            r##"
            local Analytics = game:GetService("AnalyticsService")
            local function A(Name, ...) return select(2, pcall(Analytics[Name], Analytics, ...)) end
            assert(A("LogCustomEvent") == "Argument 2 missing or nil")
            assert(A("LogCustomEvent", nil, "evt", "x") == "Unable to cast string to double")
            assert(A("LogCustomEvent", "x", "evt") == "Unable to cast value to Object")
            assert(A("LogEconomyEvent") == "Argument 6 missing or nil")
            assert(A("LogEconomyEvent", nil, 5, "Coins", 10, 100, "Gameplay") == "Unable to cast double to AnalyticsEconomyFlowType")
            assert(A("LogProgressionEvent", nil, "path", Enum.AnalyticsProgressionStatus.Begin, 1) == "Unable to cast AnalyticsProgressionStatus to AnalyticsProgressionType")
            assert(A("LogOnboardingFunnelStepEvent", nil, "x") == "Unable to cast string to int")
            Analytics:LogCustomEvent(nil, "evt")
            local Chat = game:GetService("Chat")
            local Part = Instance.new("Part")
            assert(select(2, pcall(Chat.Chat, Chat, Part, "hi", 5)) == "Unable to cast double to ChatColor")
            assert(select(2, pcall(Chat.Chat, Chat, nil, "hi")) == "partOrCharacter must be non-nil")
            assert(select(2, pcall(Chat.Chat, Chat, Part, "hi")) == "partOrCharacter must be in the Workspace")
            Part.Parent = workspace
            assert(select(2, pcall(Chat.Chat, Chat, Part, "")) == "message must be non-empty")
            local Market = game:GetService("MarketplaceService")
            assert(select(2, pcall(Market.PromptSubscriptionPurchase, Market)) == "Argument 2 missing or nil")
            assert(select(2, pcall(Market.PromptSubscriptionPurchase, Market, workspace, "EXP-1")) == "First argument to PromptSubscriptionPurchase must be a player")
            print("done")
            "##,
            0.1,
        );
        assert!(lines.iter().any(|line| line == "print: done") && errors(&lines).is_empty(), "{context:?}: {lines:?}");
        assert!(lines.iter().any(|line| line == warning), "{context:?}: {lines:?}");
    }
}

/// Player's friends methods for the offline player, who has no friends, as a 0.747 playing client and a server give them
/// (tools/parity/probes/loopback/player_friends.luau, player_friends_refused.luau): all six are members on both sides; self is
/// checked first, then the argument cast (a user id cut toward zero, NaN -2^63; a count rounded half away from zero, 2^31 -2^31);
/// a negative user id answers false at once; a count over 200 warns; GetFriendsOnline and GetFriendsInUniverseAsync are the
/// local player's alone (a server's never), checked before a negative count; a player's first question of a kind waits a
/// resumption, refused where the thread cannot yield while it is still asked, and later ones answer at once
#[test]
fn players_friends_methods_answer_as_a_client_and_a_server_do_for_the_offline_player() {
    let source = r##"
        local Player = Context.Join()
        local Other = Context.Join({ identity = { userId = 2 } })
        local function Show(Ok, Answer)
            if type(Answer) == "table" then
                Answer = "list" .. #Answer
            end
            return tostring(Answer) .. (if Ok then "" else "!")
        end
        local function Call(Self, Name, ...)
            local Ran = false
            task.defer(function() Ran = true end)
            local Line = Show(pcall(Player[Name], Self, ...))
            return Line .. (if Ran then " yielded" else "")
        end
        local function Refused(Name, ...)
            local Args = table.pack(...)
            local Line
            table.sort({ 2, 1 }, function(A, B)
                Line = Line or Show(pcall(Player[Name], Player, table.unpack(Args, 1, Args.n)))
                return A < B
            end)
            return Line
        end
        print(Refused("IsFriendsWithAsync", -1), Refused("IsFriendsWithAsync", "x"), Refused("GetFriendsOnlineAsync", -1), Refused("IsFriendsWithAsync", 5), Refused("GetFriendsOnlineAsync", 5), Refused("GetFriendsInServerAsync"), Refused("GetFriendsInUniverseAsync"))
        print(Call(Player, "IsFriendsWith", 6), Call(Player, "GetFriendsInServerAsync"))
        task.wait()
        print(Call(Player, "GetFriendsOnlineAsync"), Call(Player, "GetFriendsInUniverseAsync"), Call(Other, "IsFriendsWithAsync", 5), Call(Other, "IsFriendsWith", 5))
        print(Call(Player, "IsFriendsWithAsync", 0 / 0), Call(Player, "IsFriendsWithAsync", "7.5"), Call(Player, "IsFriendsWithAsync", coroutine.create(print)), Call(Player, "IsFriendsWithAsync"), Call(workspace, "IsFriendsWithAsync", "x"))
        print(Call(Player, "GetFriendsOnline", -0.5), Call(Player, "GetFriendsOnline", -0.4), Call(Player, "GetFriendsOnline", 2 ^ 31), Call(Player, "GetFriendsOnline", "2147483648"), Call(Player, "GetFriendsOnline", 201))
        print(Call(Other, "GetFriendsOnline", "x"), Call(Other, "GetFriendsOnline", -1), Call(Other, "GetFriendsInUniverseAsync"), Call(Other, "GetFriendsInServerAsync"))
        "##;
    let local = "GetFriendsOnline must be invoked on the Local Player!";
    let universe = "GetFriendsInUniverse must be invoked on the Local Player!";
    let negative = "GetFriendsOnline maxFriends argument can not be negative!";
    let refused = "thread is not yieldable!";
    let casts = "print: false Unable to cast string to User! Unable to cast null to User! Argument 1 missing or nil! Expected ':' not '.' calling member function IsFriendsWithAsync!";
    let warning = "warn: GetFriendsOnline only returns a maximum of 200 friends";
    assert_eq!(
        run(RunContext::Client, source, 0.2),
        [
            format!("print: false Unable to cast string to User! {negative} {refused} {refused} {refused} {refused}"),
            "print: false yielded list0".into(),
            "print: list0 list0 false yielded false".into(),
            casts.into(),
            warning.into(),
            format!("print: {negative} list0 {negative} Unable to cast string to int! list0"),
            format!("print: Unable to cast string to int! {local} {universe} list0"),
        ]
    );
    assert_eq!(
        run(RunContext::Server, source, 0.2),
        [
            format!("print: false Unable to cast string to User! {local} {refused} {local} {refused} {universe}"),
            "print: false yielded list0".into(),
            format!("print: {local} {universe} false yielded false"),
            casts.into(),
            warning.into(),
            format!("print: {local} {local} {local} Unable to cast string to int! {local}"),
            format!("print: Unable to cast string to int! {local} {universe} list0"),
        ]
    );
}
