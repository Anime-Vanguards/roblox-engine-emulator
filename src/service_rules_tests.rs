//! Service and instance rules measured on a 0.747 playing client and a server (parity package X2): what Instance.new refuses or
//! makes, the services a game script gets, a TextChannelWindow's children, Faces and Axes casts, the Player members rbx-dom lacks,
//! GuiService.CoreGuiNavigationEnabled and VoiceChatInternal.VoiceChatState, Roblox's top bar (TopbarInset, GetInsetArea) and
//! CollectionService:CreateCollection (tools/parity/probes instance_new_refusals, service_access, parent_rules, player_members,
//! gui_members, topbar_inset, inset_area, collection_*).

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

/// A playing client with Roblox's top bar: a 58 px inset on a 1280 x 720 screen
fn topbar() -> Options {
    let mut options = play();
    options.profile.device.topbar_inset = Some(58.0);
    options.profile.device.viewport = Some((1280.0, 720.0));
    options
}

/// Runs `source` as a script of a new engine to its end, the clock `seconds` on first: the printed lines. A script error fails
/// the test.
#[inline(never)]
fn printed(options: Options, source: &str, seconds: f64) -> Vec<String> {
    let (lua, engine) = start(options);
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
    let lines = engine.state.borrow().output.entries.iter().filter(|e| e.kind == "print").map(|e| e.message.to_string()).collect();
    lines
}

const TRY: &str = r#"
local function try(f, ...)
    local ok, result = pcall(f, ...)
    if ok then
        return if typeof(result) == "Instance" then "inst " .. result.ClassName .. "/" .. result.Name else "ok " .. tostring(result)
    end
    return "error " .. tostring(result)
end
"#;

/// A class whose New permit needs a capability a game script lacks is refused naming the first it lacks, three with words of their
/// own; five services are made as plain instances, which go under no instance. ree made AuroraScript, Breakpoint, PluginAction and
/// the rest, refused the five services and answered "Unable to create" for the classes rbx-dom does not know.
#[test]
fn instance_new_refuses_what_a_game_script_may_not_make_and_makes_five_services() {
    let source = format!(
        "{TRY}\nfor _, Name in {{ 'Breakpoint', 'DebuggerWatch', 'PluginAction', 'AnimationPlayer', 'CSGDictionaryService', 'AuroraScript', 'InternalSyncItem', 'ParabolaAdornment', 'TestService', 'Part' }} do print(Name, try(function() return Instance.new(Name).ClassName end)) end\n\
         local Made = Instance.new('MemoryStoreService') print('made', Made.Parent, try(function() Made.Parent = workspace end))"
    );
    let lines = printed(play(), &source, 0.1);
    assert_eq!(
        lines,
        [
            "Breakpoint error The current thread cannot create 'Breakpoint' (lacking capability RobloxScript)",
            "DebuggerWatch error The current thread cannot create 'DebuggerWatch' (lacking capability LocalUser)",
            "PluginAction error The current thread cannot create 'PluginAction' (lacking capability Plugin)",
            "AnimationPlayer error The current thread cannot create 'AnimationPlayer' (lacking capability RobloxScript)",
            "CSGDictionaryService error The current thread cannot create 'CSGDictionaryService' (lacking capability RobloxEngine)",
            "AuroraScript error This type can not be instantiated",
            "InternalSyncItem error Internal Permission is required for this feature.",
            "ParabolaAdornment error ParabolaAdornment may only be instantiated by CoreScripts at this time",
            "TestService ok TestService",
            "Part ok Part",
            "made nil error Invalid parent for Service. MemoryStoreService cannot be parented to Workspace .",
        ]
    );
    // (a plugin's code, the edit run context, holds Plugin)
    let edit = printed(Options::default(), &format!("{TRY}\nprint(try(function() return Instance.new('PluginAction').ClassName end))"), 0.1);
    assert_eq!(edit, ["ok PluginAction"]);
}

/// A game script cannot make a Player on a client or a server: refused for WritePlayer before the parent argument is looked at,
/// a copy with Instance.fromExisting too, and Clone answers nil (measured, roblox-sim 0.747, identity 2 and 4:
/// tools/parity/probes/player_new.luau). ree made one until its harnesses had Context.Join. The edit run context still makes
/// one (Studio's edit mode is not measured).
#[test]
fn a_game_script_cannot_make_a_player() {
    let refused = "error The current thread cannot create 'Player' (lacking capability WritePlayer)";
    let source = format!(
        "{TRY}\nlocal Players = game:GetService('Players')\n\
         print(try(Instance.new, 'Player'), try(Instance.new, 'Player', Players), try(Instance.new, 'Player', 5), try(Instance.new, 'Player', {{}}), try(Instance.new, 'player'))\n\
         local Me = Players.LocalPlayer\n\
         if Me then print(try(Instance.fromExisting, Me), try(Me.Clone, Me)) end"
    );
    assert_eq!(
        printed(play(), &source, 0.1),
        [format!("{refused} {refused} {refused} {refused} error Unable to create an Instance of type \"player\""), format!("{refused} ok nil")]
    );
    assert_eq!(printed(server(), &source, 0.1), [format!("{refused} {refused} {refused} {refused} error Unable to create an Instance of type \"player\"")]);
    let edit = printed(Options::default(), &format!("{TRY}\nprint(try(function() return Instance.new('Player').ClassName end))"), 0.1);
    assert_eq!(edit, ["ok Player"]);
}

/// A game script gets nil for the services Roblox keeps for itself, Studio and plugins (nothing made), finds a client's starting
/// services before any GetService, reads a few GetService makes as "Instance", and is refused the network service its side lacks.
#[test]
fn a_game_script_gets_the_services_a_client_gives_it() {
    let source = format!(
        "{TRY}\nfor _, Name in {{ 'StudioService', 'LoginService', 'LuauExpressionService', 'AccountService' }} do print(Name, try(game.GetService, game, Name), try(game.FindService, game, Name), try(game.FindFirstChildOfClass, game, Name)) end\n\
         print('starting', try(game.FindService, game, 'MarketplaceService'), try(function() return (game :: any).PolicyService end))\n\
         print('index', try(function() return (game :: any).CorePackages end), (string.gsub(try(function() return (game :: any).LoginService end), 'DataModel %S+', 'DataModel')))\n\
         print('named', try(game.GetService, game, 'PackageService'))\n\
         print('network', try(game.GetService, game, 'NetworkServer'), try(game.FindService, game, 'NetworkClient'))"
    );
    let lines = printed(play(), &source, 0.1);
    assert_eq!(
        lines,
        [
            "StudioService ok nil ok nil ok nil",
            "LoginService ok nil ok nil ok nil",
            "LuauExpressionService ok nil ok nil ok nil",
            "AccountService ok nil ok nil ok nil",
            "starting inst MarketplaceService/MarketplaceService inst PolicyService/PolicyService",
            "index ok nil error LoginService is not a valid member of DataModel",
            "named inst PackageService/Instance",
            "network error The current thread cannot create 'NetworkServer' (lacking capability RobloxEngine) inst NetworkClient/NetworkClient",
        ]
    );
    let server = printed(server(), &format!("{TRY}\nprint(try(game.GetService, game, 'NetworkClient'), try(game.FindService, game, 'ClusterPacketCache'), try(game.GetService, game, 'SpawnerService'))"), 0.1);
    assert_eq!(server, ["error The current thread cannot create 'NetworkClient' (lacking capability Plugin) inst ClusterPacketCache/ClusterPacketCache ok nil"]);
    // (a plugin's code keeps them: the edit run context)
    let edit = printed(Options::default(), &format!("{TRY}\nprint(try(game.GetService, game, 'StudioService'))"), 0.1);
    assert_eq!(edit, ["inst StudioService/StudioService"]);
}

/// CoreGui is nil to a playing client's and a server's game script every way it can look (GetService, FindService, an index,
/// FindFirstChildOfClass, GetChildren), and a play run makes none; a plugin's code, the edit run context, gets it. ree answered it
/// in a play run too while rur's story host mounted play-mode stories there (rur H1 moved them to the local player's PlayerGui).
#[test]
fn coregui_is_nil_to_a_game_script_and_there_for_a_plugin() {
    let source = format!(
        "{TRY}\nlocal Listed = false\nfor _, Each in game:GetChildren() do if Each.ClassName == 'CoreGui' then Listed = true end end\n\
         print(try(game.GetService, game, 'CoreGui'), try(game.FindService, game, 'CoreGui'), try(function() return (game :: any).CoreGui end), try(game.FindFirstChildOfClass, game, 'CoreGui'), Listed)"
    );
    for options in [play(), server()] {
        assert_eq!(printed(options, &source, 0.1), ["ok nil ok nil ok nil ok nil false"]);
    }
    assert_eq!(printed(Options::default(), &source, 0.1), ["inst CoreGui/CoreGui inst CoreGui/CoreGui inst CoreGui/CoreGui inst CoreGui/CoreGui true"]);
}

/// A TextChannelWindow takes scripts and three UI objects as children only; a Faces or an Axes property takes its own type only.
#[test]
fn a_text_channel_window_takes_few_children_and_faces_properties_cast() {
    let source = format!(
        "{TRY}\nlocal W = Instance.new('TextChannelWindow')\n\
         for _, Class in {{ 'Folder', 'UICorner', 'LocalScript', 'Frame' }} do print(Class, try(function() Instance.new(Class).Parent = W end)) end\n\
         local H, A = Instance.new('Handles'), Instance.new('ArcHandles')\n\
         print(try(function() H.Faces = Axes.new(Enum.Axis.X) end), try(function() A.Axes = Faces.new(Enum.NormalId.Top) end), try(function() H.Faces = 5 end), try(function() H.Faces = Faces.new(Enum.NormalId.Top) end), tostring(H.Faces))\n\
         print(try(function() (Instance.new('Part') :: any).ResizeableFaces = 5 end))"
    );
    let lines = printed(play(), &source, 0.1);
    let refusal = "TextChannelWindow only accepts Script, LocalScript, ModuleScript, UICorner, UIAspectRatioConstraint, or UISizeConstraint as children";
    assert_eq!(
        lines,
        [
            format!("Folder error {refusal}"),
            "UICorner ok nil".into(),
            "LocalScript ok nil".into(),
            format!("Frame error {refusal}"),
            "error Unable to assign property Faces. Faces expected, got Axes error Unable to assign property Axes. Axes expected, got Faces error Unable to assign property Faces. Faces expected, got number ok nil Top".into(),
            "error Unable to assign property ResizeableFaces. Faces expected, got number".into(),
        ]
    );
}

/// The Player members a game script reads that rbx-dom lacks, CoreGuiNavigationEnabled (true, writes ignored) and VoiceChatState
/// (its own security error); ree raised "is not a valid member" for each.
#[test]
fn players_read_the_members_a_client_gives_them() {
    let source = format!(
        "{TRY}\nlocal P = game:GetService('Players').LocalPlayer\n\
         print(P.AgeChecked, P.DeviceTrustLevel, P.FrustumStreaming, P.HasRobloxSubscription, typeof(P.User), tostring(P.User), P.User.Id, P.User == P.User, rawequal(P.User, P.User))\n\
         print(try(function() (P :: any).AgeChecked = Enum.AgeCheckStatus.Checked end), try(function() (P :: any).DeviceTrustLevel = Enum.PlayerDeviceTrustLevel.Full end), try(function() P.FrustumStreaming = Enum.FrustumStreamingMode.Enabled end), P.FrustumStreaming, P:IsPropertyModified('FrustumStreaming'), try(P.ResetPropertyToDefault, P, 'FrustumStreaming'))\n\
         local G = game:GetService('GuiService')\n\
         print(try(function() (G :: any).CoreGuiNavigationEnabled = false end), (G :: any).CoreGuiNavigationEnabled)\n\
         local V = game:GetService('VoiceChatInternal')\n\
         print(try(function() return (V :: any).VoiceChatState end), try(function() (V :: any).VoiceChatState = Enum.VoiceChatState.Idle end))"
    );
    let lines = printed(play(), &source, 0.1);
    assert_eq!(
        lines,
        [
            "Enum.AgeCheckStatus.Unchecked Enum.PlayerDeviceTrustLevel.Pending Enum.FrustumStreamingMode.Default false User U1.AQAAAAAAAAAAAAAAAAAAAAA 0 true false",
            "error The current thread cannot write 'AgeChecked' (lacking capability RobloxEngine) error Unable to assign property DeviceTrustLevel. Property is read only ok nil Enum.FrustumStreamingMode.Enabled false error Variant cast failed",
            "ok nil true",
            "error VoiceChatInternal API security check failed error Unable to assign property VoiceChatState. Property is read only",
        ]
    );
}

/// With Roblox's top bar the band starts right of the menu and chat buttons (164), at 120 with chat off, moving there in the steps
/// a client's buttons take; GetInsetArea answers in a ScreenGui's coordinates. Without the bar (no inset) both keep their answers.
#[test]
fn the_top_bar_band_follows_chat_in_a_clients_steps() {
    let source = r#"
        local G, SG, RS = game:GetService("GuiService"), game:GetService("StarterGui"), game:GetService("RunService")
        local function rect(r) return `{r.Min.X},{r.Min.Y},{r.Max.X},{r.Max.Y}` end
        RS.Heartbeat:Wait()
        print(rect(G.TopbarInset), rect(G:GetInsetArea(Enum.ScreenInsets.None)), rect(G:GetInsetArea(Enum.ScreenInsets.CoreUISafeInsets)), rect(G:GetInsetArea(Enum.ScreenInsets.TopbarSafeInsets)))
        local Changed = 0
        G:GetPropertyChangedSignal("TopbarInset"):Connect(function() Changed += 1 end)
        local function watch(apply)
            RS.Heartbeat:Wait()
            apply()
            local Seen, Last = {}, rect(G.TopbarInset)
            for Frame = 1, 30 do
                RS.Heartbeat:Wait()
                local Now = rect(G.TopbarInset)
                if Now ~= Last then table.insert(Seen, `+{Frame}={G.TopbarInset.Min.X}`) Last = Now end
            end
            print(table.concat(Seen, " "), Changed)
            Changed = 0
        end
        watch(function() SG:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, false) end)
        print(rect(G:GetInsetArea(Enum.ScreenInsets.TopbarSafeInsets)))
        watch(function() SG:SetCoreGuiEnabled(Enum.CoreGuiType.All, true) end)
        watch(function() SG:SetCoreGuiEnabled(Enum.CoreGuiType.All, false) SG:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, true) end)
    "#;
    let lines = printed(topbar(), source, 2.0);
    assert_eq!(
        lines,
        [
            "164,0,1280,58 0,-58,1280,662 0,0,1280,662 164,-58,1280,0",
            "+2=138 +3=135 +4=128 +5=125 +6=123 +7=122 +8=121 +9=120 8",
            "120,-58,1280,0",
            "+2=145 +3=148 +4=155 +5=158 +6=160 +7=161 +8=162 +9=163 +23=164 9",
            " 0",
        ]
    );
    let plain = printed(play(), "local G = game:GetService('GuiService') print(G.TopbarInset, G:GetInsetArea(Enum.ScreenInsets.TopbarSafeInsets))", 0.1);
    assert_eq!(plain, ["0, 0, 0, 0 0, 0, 1920, 1080"]);
}

/// A gamepad client's bar holds neither the menu button nor the Unibar (TOPBAR-INSET-GAMEPAD, measured in rendered 0.747 clients
/// running their core scripts: tools/parity/probes/topbar_gamepad.luau): its band starts at 0, TopbarSafeInsets with it, and chat
/// moves nothing; a gamepad beside a keyboard and a mouse (the preferred input KeyboardAndMouse) keeps a desktop's bar.
#[test]
fn a_gamepad_clients_top_bar_has_no_buttons() {
    let source = r#"
        local G, SG, RS = game:GetService("GuiService"), game:GetService("StarterGui"), game:GetService("RunService")
        local function rect(r) return `{r.Min.X},{r.Min.Y},{r.Max.X},{r.Max.Y}` end
        RS.Heartbeat:Wait()
        print(rect(G.TopbarInset), rect(G:GetInsetArea(Enum.ScreenInsets.TopbarSafeInsets)), rect(G:GetInsetArea(Enum.ScreenInsets.CoreUISafeInsets)))
        local Changed = 0
        G:GetPropertyChangedSignal("TopbarInset"):Connect(function() Changed += 1 end)
        RS.Heartbeat:Wait()
        SG:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, false)
        for _ = 1, 30 do RS.Heartbeat:Wait() end
        print(rect(G.TopbarInset), Changed)
    "#;
    let mut gamepad = topbar();
    gamepad.profile.device.kind = Some(crate::client_profile::DeviceKind::Gamepad);
    assert_eq!(printed(gamepad, source, 1.0), ["0,0,1280,58 0,-58,1280,0 0,0,1280,662", "0,0,1280,58 0"]);
    let mut beside = topbar();
    beside.profile.device.inputs.gamepad = Some(true);
    assert_eq!(printed(beside, source, 1.0), ["164,0,1280,58 164,-58,1280,0 0,0,1280,662", "120,0,1280,58 8"]);
}

/// A collection counts what its query matched when it was made, takes in what comes to match at the next deferred point (after the
/// change's own signals for a tag), counts what stops matching no more at once, and raises "Collection does not exist" once
/// destroyed; its callbacks run as members come and go.
#[test]
fn a_collection_follows_its_members_at_deferred_points() {
    let source = format!(
        "{TRY}\nlocal CS = game:GetService('CollectionService')\n\
         local Root = Instance.new('Folder') Root.Parent = workspace\n\
         local A = Instance.new('Part') A:AddTag('T') A.Parent = Root\n\
         local H = CS:CreateCollection('.T', Root)\n\
         local Log = {{}}\n\
         H.OnAdded = function(I) table.insert(Log, 'added ' .. I.Name) end\n\
         H.OnRemoved = function(I) table.insert(Log, 'removed ' .. I.Name) end\n\
         print(typeof(H), tostring(H), H.Query, H.Root == Root, H:GetCount())\n\
         local B = Instance.new('Part') B.Name = 'B' B.Parent = Root\n\
         CS:GetInstanceAddedSignal('T'):Connect(function() table.insert(Log, 'signal ' .. H:GetCount()) end)\n\
         B:AddTag('T')\n\
         print('tagged', H:GetCount())\n\
         task.wait()\n\
         print(table.concat(Log, ', '), H:GetCount(), #CS:GetCollections(B))\n\
         table.clear(Log)\n\
         A:RemoveTag('T')\n\
         print('untagged', H:GetCount())\n\
         task.wait()\n\
         print(table.concat(Log, ', '))\n\
         print(try(function() H.OnAdded = function() end end), try(function() (H :: any).OnFoo = function() end end), try(function() (H :: any).Count = 1 end), try(function() return (H :: any).Count end))\n\
         H:Destroy()\n\
         print(try(H.GetCount, H), try(function() return H.Query end), (string.gsub(try(H.GetCount, nil), '^error [^:]*:%d+: ', 'error ')))"
    );
    let lines = printed(play(), &source, 0.5);
    assert_eq!(
        lines,
        [
            "CollectionHandle Collection .T true 1",
            "tagged 1",
            "added Part, signal 1, added B 2 1",
            "untagged 1",
            "removed Part",
            "error Callback OnAdded already assigned error OnFoo cannot be assigned to error Expected function, got number error Count is not a valid member of CollectionHandle",
            "error Collection does not exist error Collection does not exist error invalid argument #1 to 'GetCount' (CollectionHandle expected, got nil)",
        ]
    );
}
