//! A loopback run's pair (src/loopback): a server and its client in one process. Each test runs a harness with Server and
//! Client sides through `record::run` as `ree run --run-context loopback` does, and reads both sides' output.

use std::path::{Path, PathBuf};

use crate::{
    launch::Host,
    record::run::{self, RecordArgs},
};

/// A scratch folder of its own for a test
#[inline(never)]
fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ree-loopback-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `ree run --run-context loopback` of `harness` (and `extra` arguments) with the project `roots` (none: an empty one): its
/// summary
#[inline(never)]
pub(crate) fn run_pair(name: &str, harness: &str, roots: Option<&Path>, extra: &[&str]) -> serde_json::Value {
    let dir = scratch(name);
    let file = dir.join(format!("{name}.luau"));
    std::fs::write(&file, harness).unwrap();
    let empty = dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let roots = roots.map(Path::to_path_buf).unwrap_or(empty);
    let mut words: Vec<String> = ["--harness", &file.to_string_lossy(), "--roots", &roots.to_string_lossy(), "--seconds", "2", "--run-context", "loopback"].iter().map(|s| s.to_string()).collect();
    words.extend(extra.iter().map(|s| s.to_string()));
    let args = run::parse(&words, false).unwrap();
    let summary = run::run_only(&args, &Host::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    summary
}

/// The output lines of a summary as `side kind: message`
#[inline(never)]
pub(crate) fn lines(summary: &serde_json::Value) -> Vec<String> {
    summary["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| format!("{} {}: {}", line["side"].as_str().unwrap_or("-"), line["kind"].as_str().unwrap_or("?"), line["message"].as_str().unwrap_or("")))
        .collect()
}

#[inline(never)]
fn has(lines: &[String], wanted: &str) -> bool {
    lines.iter().any(|line| line == wanted)
}

/// Each side answers RunService and Players as its own: the server has the joining player in Players and no LocalPlayer,
/// the client its LocalPlayer of the same name and UserId; PlayerAdded fires on the server for the join
#[test]
fn each_side_is_its_own_and_the_player_joins_the_server() {
    let summary = run_pair(
        "sides",
        r##"
local RunService = game:GetService("RunService")
local Players = game:GetService("Players")
return {
	Server = function(Context)
		Players.PlayerAdded:Connect(function(Player)
			print("added", Player.Name, Player.UserId, Player == Context.Player)
		end)
		print("server", Context.Side, RunService:IsServer(), RunService:IsClient(), Players.LocalPlayer, #Players:GetPlayers())
		task.wait()
		print("players", #Players:GetPlayers(), Players:GetPlayers()[1].Name)
	end,
	Client = function(Context)
		print("client", Context.Side, RunService:IsServer(), RunService:IsClient(), Players.LocalPlayer.Name, Players.LocalPlayer.UserId)
		task.wait(0.1)
		print("@done")
	end,
}
"##,
        None,
        &["--player", "Tester"],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server server true false nil 0"), "{lines:?}");
    assert!(has(&lines, "server print: added Tester 1 true"), "{lines:?}");
    assert!(has(&lines, "server print: players 1 Tester"), "{lines:?}");
    assert!(has(&lines, "client print: client client false true Tester 1"), "{lines:?}");
    assert_eq!(summary["runContext"], "loopback");
}

/// Both sides of a pair refuse the fast-flag methods to a game script (reflection.rs CAPABILITY_CALLS), so a scheduler that
/// reads its frame budget through a define-under-pcall helper takes its default on each side, where the define answered
/// nil and the budget then divided by the nil the getter read
#[test]
fn each_side_refuses_the_fast_flag_methods_and_a_scheduler_takes_its_default() {
    let summary = run_pair(
        "fast-flags",
        r##"
local function IntGetter(Name, Default)
	local Ok, Problem = pcall(function()
		return game:DefineFastInt(Name, Default)
	end)
	if Ok then
		return function()
			return game:GetFastInt(Name)
		end
	end
	print("define", Problem)
	return function()
		return Default
	end
end
local function Budget(Side)
	local TargetFps = IntGetter("TestTargetFps", 50)
	print(Side, "budget", 1000 / TargetFps(), select(2, pcall(game.GetFastInt, game, "TestTargetFps")))
end
return {
	Server = function(Context)
		Budget("server")
	end,
	Client = function(Context)
		Budget("client")
		task.wait()
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    for side in ["server", "client"] {
        assert!(has(&lines, &format!("{side} print: define The current thread cannot call 'DefineFastInt' (lacking capability RobloxScript)")), "{lines:?}");
        assert!(has(&lines, &format!("{side} print: {side} budget 20 The current thread cannot call 'GetFastInt' (lacking capability LocalUser)")), "{lines:?}");
    }
}

/// A loopback run refuses edit mode and a harness of another shape; `--latency` is a loopback run's
#[test]
fn launch_refuses_what_a_loopback_run_cannot_be() {
    let words = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(run::parse(&words(&["--harness", "x.luau", "--latency", "2"]), false).unwrap_err().contains("loopback run's"));
    assert!(run::parse(&words(&["--harness", "x.luau", "--run-context", "loopback", "--latency", "0"]), false).unwrap_err().contains("1 or more"));
    assert!(run::parse(&words(&["--harness", "x.luau", "--record-side", "server"]), true).is_err());
    let args: RecordArgs = run::parse(&words(&["--run-context", "loopback", "--mode", "edit"]), false).unwrap();
    assert!(run::run_only(&args, &Host::default()).unwrap_err().contains("needs --mode play"));
    let summary = run_pair("shape", "return function(Context) print('one side') end", None, &[]);
    assert_eq!(summary["status"], "error");
    assert!(summary["failure"].as_str().unwrap_or_default().contains("a loopback harness returns { Server"), "{summary}");
}

/// A frame is the server's, then the client's, with the same dt; a run ends when either side prints `@done`, and both
/// sides' lines come in one list in the order written, each with its side
#[test]
fn the_pair_steps_together_and_either_side_ends_the_run() {
    let summary = run_pair(
        "steps",
        r##"
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		local Frames = 0
		RunService.Heartbeat:Connect(function(Delta)
			Frames += 1
			print("server beat", Frames, string.format("%.4f", Delta))
			if Frames == 3 then
				print("@done")
			end
		end)
	end,
	Client = function(Context)
		local Frames = 0
		RunService.Heartbeat:Connect(function(Delta)
			Frames += 1
			print("client beat", Frames, string.format("%.4f", Delta))
		end)
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    let beats: Vec<&String> = lines.iter().filter(|line| line.contains(" beat ")).collect();
    assert_eq!(
        beats,
        [
            "server print: server beat 1 0.0167",
            "client print: client beat 1 0.0167",
            "server print: server beat 2 0.0167",
            "client print: client beat 2 0.0167",
            "server print: server beat 3 0.0167",
            "client print: client beat 3 0.0167",
        ],
        "{lines:?}"
    );
    assert_eq!(summary["done"], true);
    assert_eq!(summary["frames"], 4, "frame 0 and three frames");
}

/// A project with a shared module in ReplicatedStorage, a secret in ServerStorage and a server script: the join copies
/// what replicates, keeps what does not, and the harness's own RURFixtures stays each side's
#[inline(never)]
fn project(name: &str) -> PathBuf {
    let root = scratch(name);
    for (path, text) in [
        ("ReplicatedStorage/Shared/Counter.luau", "local Count = 0 return { Bump = function() Count += 1 return Count end }"),
        ("ServerStorage/Secret.luau", "return 'secret'"),
        ("ServerScriptService/Boot.server.luau", "print('server boot')"),
        ("Workspace/Spot.model.json", "{\"className\": \"Part\", \"properties\": {\"Anchored\": true}}"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    root
}

/// The join: the client has a copy of the replicated containers (a project module it requires, a rename and an attribute
/// the server's harness made before the join), nothing of ServerStorage or ServerScriptService, and its own RURFixtures;
/// each side requires its own copy of a shared module
#[test]
fn the_join_copies_the_replicated_containers() {
    let root = project("join");
    let summary = run_pair(
        "join",
        r##"
local Storage = game:GetService("ReplicatedStorage")
return {
	Server = function(Context)
		local Counter = require(Storage.Shared.Counter)
		print("server bumps", Counter.Bump(), Counter.Bump())
		workspace.Spot.Name = "Renamed"
		workspace.Renamed:SetAttribute("Mark", 7)
		local Folder = Instance.new("Folder")
		Folder.Name = "MadeBeforeJoin"
		Folder.Parent = Storage
	end,
	Client = function(Context)
		local Counter = require(Storage.Shared.Counter)
		print("client bumps", Counter.Bump())
		print("renamed", workspace:FindFirstChild("Renamed") ~= nil, workspace.Renamed:GetAttribute("Mark"), workspace.Renamed.Anchored)
		print("made", Storage:FindFirstChild("MadeBeforeJoin") ~= nil)
		print("server storage", #game:GetService("ServerStorage"):GetChildren(), "scripts", #game:GetService("ServerScriptService"):GetChildren())
		print("fixtures", #Storage.RURFixtures:GetChildren(), Storage.RURFixtures:GetChildren()[1].Name)
		print("@done")
	end,
}
"##,
        Some(&root),
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server bumps 1 2"), "{lines:?}");
    assert!(has(&lines, "client print: client bumps 1"), "{lines:?}");
    assert!(has(&lines, "client print: renamed true 7 true"), "{lines:?}");
    assert!(has(&lines, "client print: made true"), "{lines:?}");
    assert!(has(&lines, "client print: server storage 0 scripts 0"), "{lines:?}");
    assert!(has(&lines, "client print: fixtures 1 join"), "{lines:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A host that makes its own client (rur's stories): the server started alone, the host's client readied, started with
/// no project and linked; `begin_server` starts the server's part of the harness, which holds the join for a few frames,
/// and the player joins once it lets it in. An error the server's harness raises at once is returned.
#[test]
fn a_host_links_its_own_client_and_the_servers_harness_holds_the_join() {
    use crate::{Emulator, Engine, Options, RunContext, loopback, mount::ProjectSpec};
    let dir = scratch("host");
    let harness = dir.join("Host.luau");
    std::fs::write(&harness, "return { Server = function(Context) Context.HoldJoin() print('server holds') task.wait(0.05) Context.Join() print('server lets in') end }").unwrap();
    let failing = dir.join("Failing.luau");
    std::fs::write(&failing, "return { Server = function(Context) error('the server harness failed') end }").unwrap();
    let options = Options { mode: "play".into(), run_context: RunContext::Loopback, ..Options::default() };
    let pair = |harness: &Path| {
        let server = loopback::start_server(&Engine::new, &options, &ProjectSpec::default(), &|_| {}).unwrap();
        let seed = server.engine.state.borrow().sched.random_seed;
        let engine = Engine::new(loopback::side_options(&options, loopback::Side::Client)).unwrap();
        loopback::ready_client(&engine);
        assert_eq!(engine.state.borrow().sched.random_seed, seed + 1.0, "the client's own sequence");
        let client = Emulator::start(engine, &ProjectSpec::default()).unwrap();
        loopback::link(server, &client.lua, &client.engine, loopback::Spec::default()).unwrap();
        let begun = run::begin_server(&client.lua, &client.engine, Some(harness), 1.0 / 60.0, 600);
        (client, begun)
    };
    let (client, begun) = pair(&harness);
    begun.unwrap();
    let bridge = client.engine.loopback().unwrap().bridge.clone();
    assert!(bridge.joined.get(), "the player joined");
    assert!(bridge.joined_at.get() >= 3, "the server stepped alone until it let the player in (joined at {})", bridge.joined_at.get());
    let printed: Vec<String> = client.engine.state.borrow().output.entries.iter().map(|entry| entry.message.clone()).collect();
    assert!(printed.iter().any(|line| line == "server holds") && printed.iter().any(|line| line == "server lets in"), "{printed:?}");
    let (_failed, begun) = pair(&failing);
    assert!(begun.is_err_and(|message| message.contains("the server harness failed")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A module both sides require is compiled once in the run: the second side loads the bytecode the first one's compile
/// made (each side still runs its own copy of it)
#[test]
fn a_shared_module_compiles_once() {
    let root = project("compile");
    let dir = scratch("compile-run");
    let harness = dir.join("Compile.luau");
    std::fs::write(&harness, "local Storage = game:GetService('ReplicatedStorage') return { Server = function() print('server', require(Storage.Shared.Counter).Bump()) end, Client = function() print('client', require(Storage.Shared.Counter).Bump()) print('@done') end }").unwrap();
    let words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", "1", "--run-context", "loopback"].iter().map(|s| s.to_string()).collect();
    let args = run::parse(&words, false).unwrap();
    let (done, emulator) = run::execute(&args, &Host::default(), false).unwrap();
    let lines = lines(&done.summary);
    assert!(has(&lines, "server print: server 1") && has(&lines, "client print: client 1"), "{lines:?}");
    let (_, server) = emulator.engine.peer_server().unwrap();
    assert_eq!(emulator.engine.modules.borrow().text_compiles, 0, "the client compiled a module's text");
    assert_eq!(server.modules.borrow().text_compiles, 0, "the server's require loaded the process's bytecode too");
    drop(server);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project whose place has a Studio hierarchy snapshot: Studio-only placeholders below ReplicatedStorage and the Workspace
/// (a Folder of remotes, an asset, a map), and one module on disk. Returns the root to mount.
#[inline(never)]
fn studio_project(name: &str) -> PathBuf {
    let dir = scratch(name);
    let root = dir.join("place");
    std::fs::create_dir_all(root.join("ReplicatedStorage")).unwrap();
    std::fs::write(root.join("ReplicatedStorage/Note.luau"), "return 'note'").unwrap();
    std::fs::create_dir_all(dir.join(crate::project::SNAPSHOTS_DIR)).unwrap();
    std::fs::write(dir.join(crate::project::PLACES_FILE), r#"{"environments":[{"name":"Lobby","projectFile":"place/default.project.json"}]}"#).unwrap();
    let node = |name: &str, class: &str, parent: i64| format!(r#"{{"name":"{name}","className":"{class}","parent":{parent}}}"#);
    let nodes = [
        node("game", "DataModel", -1),
        node("ReplicatedStorage", "ReplicatedStorage", 0),
        node("Workspace", "Workspace", 0),
        node("Remotes", "Folder", 1),
        node("Ping", "RemoteEvent", 3),
        node("Assets", "Folder", 1),
        node("Sword", "Model", 5),
        node("Handle", "Part", 6),
        node("Map", "Model", 2),
        node("Door", "Part", 8),
        node("Lamp", "Part", 8),
        node("Bulb", "Part", 10),
        node("Props", "Folder", 1),
        node("Crate", "Part", 12),
        node("Lid", "Part", 13),
    ];
    std::fs::write(dir.join(crate::project::SNAPSHOTS_DIR).join("Lobby.json"), format!(r#"{{"nodes":[{}]}}"#, nodes.join(","))).unwrap();
    root
}

/// The place's Studio-only instances stay lazy at the join: the client makes those its scripts look at itself, as a plain
/// client run does, and the server's own (made after the join, when its scripts look) pair with the client's when a
/// message names them: a remote both sides found, a part the server passes and writes, a part the client passes back
/// that the server had not made. The server never makes what only the client looked at.
#[test]
fn studio_only_instances_stay_lazy_and_pair_as_they_are_named() {
    let root = studio_project("studio");
    let dir = scratch("studio-run");
    let harness = dir.join("Studio.luau");
    std::fs::write(
        &harness,
        r##"
local Storage = game:GetService("ReplicatedStorage")
return {
	Server = function(Context)
		task.wait(0.1)
		local Ping = Storage.Remotes.Ping
		Ping.OnServerEvent:Connect(function(Player, Part, Same)
			print("server got", Part and Part:GetFullName(), Part == Storage.Props.Crate.Lid, Same == Ping)
		end)
		local Crate = Storage.Props.Crate
		Crate.Transparency = 0.5
		Crate:SetAttribute("Open", true)
		Ping:FireAllClients(Crate, Ping)
	end,
	Client = function(Context)
		local Map = workspace:WaitForChild("Map")
		local Names = {}
		for _, Part in Map:GetDescendants() do
			table.insert(Names, Part.Name)
		end
		table.sort(Names)
		print("client map", table.concat(Names, " "))
		local Sword = Storage.Assets.Sword
		print("client sword", Sword.Handle.ClassName)
		local Ping = Storage.Remotes.Ping
		Ping.OnClientEvent:Connect(function(Crate, Same)
			task.wait()
			print("client got", Crate == Storage.Props.Crate, Crate.Transparency, Crate:GetAttribute("Open"), Same == Ping)
			Ping:FireServer(Crate.Lid, Ping)
		end)
	end,
}
"##,
    )
    .unwrap();
    let words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", "1", "--run-context", "loopback"].iter().map(|s| s.to_string()).collect();
    let args = run::parse(&words, false).unwrap();
    let (done, emulator) = run::execute(&args, &Host::default(), false).unwrap();
    let lines = lines(&done.summary);
    assert!(has(&lines, "client print: client map Bulb Door Lamp"), "{lines:?}");
    assert!(has(&lines, "client print: client sword Part"), "{lines:?}");
    assert!(has(&lines, "client print: client got true 0.5 true true"), "{lines:?}");
    assert!(has(&lines, "server print: server got ReplicatedStorage.Props.Crate.Lid true true"), "{lines:?}");
    // the server never made the asset only the client looked at; both sides made Studio-only instances after the join
    let (_, server) = emulator.engine.peer_server().unwrap();
    assert!(server.loopback().unwrap().lazy.borrow().made() >= 3, "the server made its remote, the crate and its lid after the join");
    assert!(emulator.engine.loopback().unwrap().lazy.borrow().made() >= 5, "the client made its own");
    {
        let state = server.state.borrow();
        let storage = crate::services::find_service(&server, server.world().game, "ReplicatedStorage").unwrap();
        let sword = state.dom.find_child(storage, b"Assets").and_then(|assets| state.dom.find_child(assets, b"Sword"));
        assert!(sword.is_none_or(|sword| state.dom.find_child(sword, b"Handle").is_none()), "the server made the client's asset");
    }
    drop(server);
    drop(emulator);
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// After the join every change the server makes in the replicated containers arrives at the client's next frame, as its
/// value when the server's frame ended: three writes give one Changed with the last, a made child arrives whole, a move
/// out of the containers leaves the client's copy unparented (the same instance comes back), a destroy destroys it; a
/// write to ServerStorage sends nothing, and a client's own write stays until the server writes the property again
#[test]
fn the_servers_changes_arrive_at_the_clients_next_frame() {
    let summary = run_pair(
        "changes",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
end
return {
	Server = function(Context)
		local Value = Instance.new("IntValue")
		Value.Name = "Value"
		Value.Parent = Storage
		local Hidden = Instance.new("IntValue")
		Hidden.Parent = game:GetService("ServerStorage")
		Frames(3)
		Value.Value = 1
		Value.Value = 2
		Value.Value = 3
		Hidden.Value = 9
		Frames(3)
		local Child = Instance.new("IntValue")
		Child.Name = "Child"
		Child.Parent = Value
		Child.Value = 5
		Frames(3)
		Value.Parent = game:GetService("ServerStorage")
		Frames(3)
		Value.Parent = Storage
		Frames(3)
		Value.Value = 4
		Frames(3)
		Value:Destroy()
	end,
	Client = function(Context)
		local Value = Storage:WaitForChild("Value")
		local Seen = {}
		Value.Changed:Connect(function(New)
			table.insert(Seen, "Changed " .. New)
		end)
		Value.ChildAdded:Connect(function(Child)
			table.insert(Seen, "ChildAdded " .. Child.Name .. " " .. Child.Value)
		end)
		Value.AncestryChanged:Connect(function(_, Parent)
			table.insert(Seen, "Parent " .. tostring(Parent))
		end)
		Value.Destroying:Connect(function()
			table.insert(Seen, "Destroying")
		end)
		Value.Value = 100
		Frames(20)
		print("seen", table.concat(Seen, " / "))
		print("gone", Storage:FindFirstChild("Value") == nil, Value.Parent)
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(
        has(&lines, "client print: seen Changed 100 / Changed 3 / ChildAdded Child 5 / Parent nil / Parent ReplicatedStorage / Changed 4 / Destroying / Parent nil"),
        "{lines:?}"
    );
    assert!(has(&lines, "client print: gone true nil"), "{lines:?}");
}

/// Remote events each way: OnServerEvent gets the player first; the arguments arrive as the call left them (a table, a
/// buffer, an instance the client has, nil for one it lacks); the wrong side's calls raise with the measured texts;
/// events that reach a remote nothing listens to wait for its first connection
#[test]
fn remote_events_cross_each_way() {
    let summary = run_pair(
        "remotes",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		local Remote = Instance.new("RemoteEvent")
		Remote.Name = "Remote"
		Remote.Parent = Storage
		local Late = Instance.new("RemoteEvent")
		Late.Name = "Late"
		Late.Parent = Storage
		local Shared = Instance.new("Part")
		Shared.Name = "Shared"
		Shared.Parent = workspace
		local Hidden = Instance.new("Part")
		Hidden.Parent = game:GetService("ServerStorage")
		Remote.OnServerEvent:Connect(function(Player, Text, Table)
			print("server got", Player.Name, Text, Table.a, Table[1])
			local Data = { 1, 2, x = 3 }
			Remote:FireClient(Player, Data, buffer.fromstring("ab"), Shared, Hidden, nil)
			Data[1] = 9
		end)
		print("server FireServer", pcall(Remote.FireServer, Remote))
		-- (before the join nobody is there to get it; after it the client's Late waits for a connection)
		Late:FireAllClients(0)
		task.wait()
		for Index = 1, 3 do
			Late:FireAllClients(Index)
		end
	end,
	Client = function(Context)
		local Remote = Storage:WaitForChild("Remote")
		Remote.OnClientEvent:Connect(function(...)
			local Data, Buffer, Shared, Hidden = ...
			print("client got", Data[1], Data[2], Data.x, buffer.tostring(Buffer), Shared and Shared.Name, Hidden, select("#", ...))
		end)
		print("client FireClient", pcall(Remote.FireClient, Remote, game.Players.LocalPlayer))
		print("client connect", pcall(function() Remote.OnServerEvent:Connect(print) end))
		Remote:FireServer("hello", { a = 1 })
		for _ = 1, 10 do
			RunService.Heartbeat:Wait()
		end
		local Got = {}
		Storage.Late.OnClientEvent:Connect(function(Index)
			table.insert(Got, Index)
		end)
		Storage.Late.OnClientEvent:Connect(function(Index)
			table.insert(Got, "second " .. Index)
		end)
		RunService.Heartbeat:Wait()
		print("late", table.concat(Got, " "))
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server FireServer false FireServer can only be called from the client"), "{lines:?}");
    assert!(has(&lines, "client print: client FireClient false FireClient can only be called from the server"), "{lines:?}");
    assert!(has(&lines, "client print: client connect false OnServerEvent can only be used on the server"), "{lines:?}");
    assert!(has(&lines, "server print: server got Player hello 1 nil"), "{lines:?}");
    assert!(has(&lines, "client print: client got 1 2 nil ab Shared nil 5"), "{lines:?}");
    assert!(has(&lines, "client print: late 1 2 3"), "{lines:?}");
}

/// A SharedTable sent through a remote arrives as nil, each way, as a client and its server measured (Studio 0.748, two runs:
/// loopback/wire.rs): alone, and inside a table, where it leaves a hole in an array and drops its dictionary entry. On main,
/// where SharedTable was a Lua table, it arrived as a copied table.
#[test]
fn shared_tables_cross_remotes_as_nil_each_way() {
    let summary = run_pair(
        "shared-table-remotes",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Show(...)
	local Alone, Array, Dictionary = ...
	return `{tostring(Alone)} [{tostring(Array[1])} {tostring(Array[2])} {tostring(Array[3])}] [{tostring(Dictionary.s)} {tostring(Dictionary.n)}] {select("#", ...)}`
end
return {
	Server = function(Context)
		local Remote = Instance.new("RemoteEvent")
		Remote.Name = "SharedTables"
		Remote.Parent = Storage
		Remote.OnServerEvent:Connect(function(Player, ...)
			print("server got", Show(...))
			local S = SharedTable.new({ x = 1 })
			Remote:FireClient(Player, S, { 1, S, 3 }, { s = S, n = 2 })
		end)
	end,
	Client = function(Context)
		local Remote = Storage:WaitForChild("SharedTables")
		Remote.OnClientEvent:Connect(function(...)
			print("client got", Show(...))
		end)
		local S = SharedTable.new({ x = 1 })
		Remote:FireServer(S, { 1, S, 3 }, { s = S, n = 2 })
		for _ = 1, 10 do
			RunService.Heartbeat:Wait()
		end
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server got nil [1 nil 3] [nil 2] 3"), "{lines:?}");
    assert!(has(&lines, "client print: client got nil [1 nil 3] [nil 2] 3"), "{lines:?}");
}

/// Over remotes, each way: Axes, Faces, PathWaypoint, SecurityCapabilities and the sequence keypoints arrive with the same text,
/// alone and in a table, a key with a client's own type name arrives so named, and a key whose `__tostring` fails raises from
/// the sending call and sends nothing; an error a RemoteFunction's callback raises before it first yields is logged on the
/// callback's side as an uncaught error is, one after a yield is not (measured, a 0.747 play session's server and client:
/// tools/parity/probes/loopback/remote_keys.luau, remote_callback_errors.luau). On main those values arrived as nil, the key as
/// `<Table> ()`, and the server logged nothing.
#[test]
fn remotes_carry_proxy_values_and_callbacks_log_their_errors() {
    let summary = run_pair(
        "remote-proxies-and-errors",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Values()
	return Axes.new(Enum.Axis.X, Enum.NormalId.Top), Faces.new(Enum.NormalId.Top, Enum.NormalId.Back),
		PathWaypoint.new(Vector3.new(1, 2, 3), Enum.PathWaypointAction.Jump, "label"),
		SecurityCapabilities.new(Enum.SecurityCapability.UI, Enum.SecurityCapability.Basic), ColorSequenceKeypoint.new(0.5, Color3.new(0, 0, 1)),
		{ W = NumberSequenceKeypoint.new(0.5, 2, 0.25), [CFrame.new()] = Faces.new(Enum.NormalId.Left) }
end
local function Show(...)
	local Parts = {}
	for Index = 1, select("#", ...) do
		local Value = select(Index, ...)
		if type(Value) == "table" then
			local Keys = {}
			for Key, Item in Value do
				table.insert(Keys, `{Key}={typeof(Item)} {Item}`)
			end
			table.sort(Keys)
			Value = table.concat(Keys, ",")
		end
		table.insert(Parts, `{typeof(Value)} {Value}`)
	end
	return table.concat(Parts, " | ")
end
local Failing = setmetatable({}, { __tostring = function() return nil end })
return {
	Server = function(Context)
		local Remote = Instance.new("RemoteEvent")
		Remote.Name = "Values"
		Remote.Parent = Storage
		local Callback = Instance.new("RemoteFunction")
		Callback.Name = "Callback"
		Callback.OnServerInvoke = function(_, Kind)
			if Kind == "later" then
				task.wait()
				error("failed later", 0)
			end
			error("callback failed", 0)
		end
		Callback.Parent = Storage
		Remote.OnServerEvent:Connect(function(Player, ...)
			print("server got", Show(...))
			Remote:FireClient(Player, Values())
			print("server sent", pcall(Remote.FireClient, Remote, Player, { [Failing] = 1 }))
		end)
	end,
	Client = function(Context)
		local Remote = Storage:WaitForChild("Values")
		local Callback = Storage:WaitForChild("Callback")
		Remote.OnClientEvent:Connect(function(...)
			print("client got", Show(...))
		end)
		Remote:FireServer(Values())
		print("client sent", pcall(Remote.FireServer, Remote, { [Failing] = 1 }))
		print("invoked", pcall(Callback.InvokeServer, Callback, "now"))
		print("invoked", pcall(Callback.InvokeServer, Callback, "later"))
		for _ = 1, 10 do
			RunService.Heartbeat:Wait()
		end
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    let got = "got Axes X, Y | Faces Top, Back | PathWaypoint {1, 2, 3}, Jump, label | SecurityCapabilities Basic | UI | ColorSequenceKeypoint 0.5 0 0 1 0  | string <CoordinateFrame> (0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1)=Faces Left,W=NumberSequenceKeypoint 0.5 2 0.25 ";
    assert!(has(&lines, &format!("server print: server {got}")), "{lines:?}");
    assert!(has(&lines, &format!("client print: client {got}")), "{lines:?}");
    assert!(has(&lines, "server print: server sent false '__tostring' must return a string"), "{lines:?}");
    assert!(has(&lines, "client print: client sent false '__tostring' must return a string"), "{lines:?}");
    assert!(has(&lines, "client print: invoked false callback failed"), "{lines:?}");
    assert!(has(&lines, "client print: invoked false failed later"), "{lines:?}");
    assert!(has(&lines, "server error: callback failed"), "{lines:?}");
    assert!(!lines.iter().any(|line| line.contains("error: failed later")), "{lines:?}");
}

/// An InstanceHandle crosses a remote as the receiver's own (measured, Studio 0.748, a play session's server and client:
/// tools/parity/probes/loopback/remote_instance_handles.luau, loopback/wire.rs): a new handle of the receiver's copy, or of
/// none when the receiver lacks the instance, alone and in a table, each way, a key named `<InstanceRef> (Name)`; and an
/// attribute holding an instance or a handle reaches the client as a handle of its copy, or of none (loopback/replicate.rs),
/// set before the join and after. On main a handle arrived as nil and the attribute held the server's instance.
#[test]
fn instance_handles_cross_remotes_and_attributes_as_the_receivers_own() {
    let summary = run_pair(
        "instance-handle-remotes",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Show(Value)
	if typeof(Value) ~= "InstanceHandle" then
		return typeof(Value)
	end
	local Got = Value:Get()
	return if Got then Got.Name else "none"
end
local function Shows(Alone, List, Keyed)
	return Show(Alone) .. " [" .. Show(List[1]) .. " " .. Show(List[2]) .. "] " .. tostring(next(Keyed))
end
local function Attributes(Holder)
	local Names = {}
	for Name, Value in Holder:GetAttributes() do
		table.insert(Names, Name .. "=" .. Show(Value))
	end
	table.sort(Names)
	return table.concat(Names, " ")
end
return {
	Server = function(Context)
		local Shared = Instance.new("Part")
		Shared.Name = "Shared"
		Shared.Parent = workspace
		local Mine = Instance.new("Folder")
		Mine.Name = "ServerOnly"
		Mine.Parent = game:GetService("ServerStorage")
		local Joined = Instance.new("Folder")
		Joined.Name = "Joined"
		Joined:SetAttribute("Shared", InstanceHandle.new(Shared))
		Joined:SetAttribute("Own", Mine)
		Joined:SetAttribute("Empty", InstanceHandle.new(nil))
		Joined.Parent = Storage
		local Later = Instance.new("Folder")
		Later.Name = "Later"
		Later.Parent = Storage
		local Remote = Instance.new("RemoteEvent")
		Remote.Name = "Handles"
		Remote.Parent = Storage
		Remote.OnServerEvent:Connect(function(Player, ...)
			print("server got", Shows(...))
			Later:SetAttribute("Shared", Shared)
			Later:SetAttribute("Own", InstanceHandle.new(Mine))
			Remote:FireClient(Player, InstanceHandle.new(Shared), { InstanceHandle.new(Mine), InstanceHandle.new(nil) }, { [InstanceHandle.new(Shared)] = 1 })
		end)
	end,
	Client = function(Context)
		local Remote = Storage:WaitForChild("Handles")
		print("client joined", Attributes(Storage:WaitForChild("Joined")))
		Remote.OnClientEvent:Connect(function(...)
			print("client got", Shows(...))
		end)
		local Mine = Instance.new("Folder")
		Mine.Name = "ClientOnly"
		Mine.Parent = Storage
		Remote:FireServer(InstanceHandle.new(workspace:WaitForChild("Shared")), { InstanceHandle.new(Mine), InstanceHandle.new(nil) }, { [InstanceHandle.new(Mine)] = 1 })
		for _ = 1, 10 do
			RunService.Heartbeat:Wait()
		end
		print("client later", Attributes(Storage.Later))
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: client joined Empty=none Own=none Shared=Shared"), "{lines:?}");
    assert!(has(&lines, "server print: server got Shared [none none] <InstanceRef> (ClientOnly)"), "{lines:?}");
    assert!(has(&lines, "client print: client got Shared [none none] <InstanceRef> (Shared)"), "{lines:?}");
    assert!(has(&lines, "client print: client later Own=none Shared=Shared"), "{lines:?}");
}

/// A remote copies a table once for each place it appears, as Roblox does (a 0.747 session carried one table of 1,000
/// numbers held 1,000 times each way, and an UnreliableRemoteEvent's warning counted every byte of it), and ree stops a copy
/// past its guard (lua/bindable.rs `COPY_VALUES`) at the sender with its own error. The big table holds 200 references to a
/// table of 100,000 functions first (functions arrive as nil, so their slots make nothing) and then 100,000 references to a
/// table of 100,000 numbers, 10^10 numbers without the guard: FireServer, an unreliable FireServer, InvokeServer, a
/// callback's results and FireClient raise the guard's error before a number is copied. A copy below it arrives whole.
#[test]
fn remotes_stop_at_rees_guard_on_a_table_copied_for_each_place_it_appears() {
    let summary = run_pair(
        "remote-breadth",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local function Huge()
	local Functions = table.create(100000, print)
	local Numbers = table.create(100000, 1)
	local Value = table.create(100200)
	for Index = 1, 200 do
		Value[Index] = Functions
	end
	for Index = 201, 100200 do
		Value[Index] = Numbers
	end
	return Value
end
return {
	Server = function(Context)
		local Event = Instance.new("RemoteEvent")
		Event.Name = "Breadth"
		Event.Parent = Storage
		local Unreliable = Instance.new("UnreliableRemoteEvent")
		Unreliable.Name = "BreadthUnreliable"
		Unreliable.Parent = Storage
		local Call = Instance.new("RemoteFunction")
		Call.Name = "BreadthCall"
		Call.OnServerInvoke = function(_, Value)
			if Value == "huge" then
				return Huge()
			end
			return #Value
		end
		Call.Parent = Storage
		Event.OnServerEvent:Connect(function(Player, Value)
			print("got", #Value, #Value[#Value], Value[1] ~= Value[2])
			print("client huge", pcall(Event.FireClient, Event, Player, Huge()))
		end)
	end,
	Client = function(Context)
		local Event = Storage:WaitForChild("Breadth")
		local Unreliable = Storage:WaitForChild("BreadthUnreliable")
		local Call = Storage:WaitForChild("BreadthCall")
		print("server huge", pcall(Event.FireServer, Event, Huge()))
		print("unreliable huge", pcall(Unreliable.FireServer, Unreliable, Huge()))
		print("invoke huge", pcall(Call.InvokeServer, Call, Huge()))
		print("invoke result huge", pcall(Call.InvokeServer, Call, "huge"))
		print("invoke fan", pcall(Call.InvokeServer, Call, table.create(300, table.create(1000, 1))))
		Event:FireServer(table.create(300, table.create(1000, 1)))
		for _ = 1, 10 do
			task.wait()
		end
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    let past = crate::lua::bindable::too_large(crate::lua::bindable::COPY_VALUES, "values");
    for wanted in [
        format!("client print: server huge false {past}"),
        format!("client print: unreliable huge false {past}"),
        format!("client print: invoke huge false {past}"),
        format!("client print: invoke result huge false {past}"),
        "client print: invoke fan true 300".into(),
        "server print: got 300 1000 true".into(),
        format!("server print: client huge false {past}"),
    ] {
        assert!(has(&lines, &wanted), "{wanted}: {lines:?}");
    }
}

/// A remote's tables are walked on the Lua stack as an event's are (bindable.rs `nested_height`), from the call's frame:
/// `self` and the arguments (the player among them for FireClient), a callback's results alone. So a dictionary nests half
/// as deep as an array, each further argument takes a level, and a callback may return an array 300 deep (measured each
/// way in a 0.747 play session, tools/parity/probes/loopback/remote_nesting.luau); 100,000 levels raise the same error
#[test]
fn remotes_refuse_tables_nested_past_the_lua_stack() {
    let summary = run_pair(
        "remote-depth",
        r##"
local Storage = game:GetService("ReplicatedStorage")
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
local function Height(Value)
	local Count = 0
	while type(Value) == "table" do
		Count += 1
		Value = Value[1]
	end
	return Count
end
return {
	Server = function(Context)
		local Event = Instance.new("RemoteEvent")
		Event.Name = "Depth"
		Event.Parent = Storage
		local Call = Instance.new("RemoteFunction")
		Call.Name = "DepthCall"
		Call.OnServerInvoke = function(_, Depth)
			return Nest(Depth)
		end
		Call.Parent = Storage
		local Ready = Instance.new("RemoteEvent")
		Ready.Name = "Ready"
		Ready.Parent = Storage
		local Player = Ready.OnServerEvent:Wait()
		print("client dict 149", pcall(Event.FireClient, Event, Player, Nest(149, "a")))
		print("client dict 150", pcall(Event.FireClient, Event, Player, Nest(150, "a")))
		print("all 299", pcall(Event.FireAllClients, Event, Nest(299)))
		print("all 300", pcall(Event.FireAllClients, Event, Nest(300)))
		print("client 100000", pcall(Event.FireClient, Event, Player, Nest(100000)))
	end,
	Client = function(Context)
		local Event = Storage:WaitForChild("Depth")
		local Call = Storage:WaitForChild("DepthCall")
		print("server dict 150", pcall(Event.FireServer, Event, Nest(150, "a")))
		print("server dict 151", pcall(Event.FireServer, Event, Nest(151, "a")))
		print("server 2nd 298", pcall(Event.FireServer, Event, 1, Nest(298)))
		print("server 2nd 299", pcall(Event.FireServer, Event, 1, Nest(299)))
		print("invoke arg 300", pcall(Call.InvokeServer, Call, Nest(300)))
		print("invoke 300", pcall(function() return Height(Call:InvokeServer(300)) end))
		print("invoke 301", pcall(function() return Height(Call:InvokeServer(301)) end))
		print("server 100000", pcall(Event.FireServer, Event, Nest(100000)))
		Storage:WaitForChild("Ready"):FireServer()
		for _ = 1, 10 do
			task.wait()
		end
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    let deep = "Max lua stack size reached";
    for wanted in [
        "client print: server dict 150 true".to_string(),
        format!("client print: server dict 151 false {deep}"),
        "client print: server 2nd 298 true".into(),
        format!("client print: server 2nd 299 false {deep}"),
        format!("client print: invoke arg 300 false {deep}"),
        "client print: invoke 300 true 300".into(),
        format!("client print: invoke 301 false {deep}"),
        format!("client print: server 100000 false {deep}"),
        "server print: client dict 149 true".into(),
        format!("server print: client dict 150 false {deep}"),
        "server print: all 299 true".into(),
        format!("server print: all 300 false {deep}"),
        format!("server print: client 100000 false {deep}"),
    ] {
        assert!(has(&lines, &wanted), "{wanted}: {lines:?}");
    }
}

/// RemoteFunction both ways: results and nils kept, the callback's error raised in the caller with its position, a call
/// that waits for the callback to be set; when the player leaves the server's InvokeClient calls raise, and no call is
/// left held by either side
#[test]
fn remote_functions_answer_and_nothing_is_left_waiting() {
    let summary = run_pair(
        "invokes",
        r##"
local Storage = game:GetService("ReplicatedStorage")
return {
	Server = function(Context)
		local Function = Instance.new("RemoteFunction")
		Function.Name = "Function"
		Function.OnServerInvoke = function(Player, Kind)
			if Kind == "error" then
				error("server failed")
			end
			task.wait(0.05)
			return Kind, nil, Player.Name
		end
		Function.Parent = Storage
		local Later = Instance.new("RemoteFunction")
		Later.Name = "Later"
		Later.Parent = Storage
		task.delay(0.2, function()
			Later.OnServerInvoke = function(_, Value)
				return Value * 2
			end
		end)
		local Stuck = Instance.new("RemoteFunction")
		Stuck.Name = "Stuck"
		Stuck.Parent = Storage
		task.wait(0.4)
		task.spawn(function()
			print("stuck", pcall(Stuck.InvokeClient, Stuck, Context.Player))
		end)
		task.wait(0.1)
		Context.Leave()
		task.wait(0.1)
		print("@done")
	end,
	Client = function(Context)
		local Function = Storage:WaitForChild("Function")
		Storage:WaitForChild("Stuck").OnClientInvoke = function()
			task.wait(100)
		end
		print("ok", select("#", Function:InvokeServer("plain")), Function:InvokeServer("plain"))
		local Ok, Message = pcall(Function.InvokeServer, Function, "error")
		print("error", Ok, (string.gsub(Message, "^.-:%d+: ", "<position>: ")))
		print("later", Storage:WaitForChild("Later"):InvokeServer(21))
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert!(has(&lines, "client print: ok 3 plain nil Player"), "{lines:?}");
    assert!(has(&lines, "client print: error false <position>: server failed"), "{lines:?}");
    assert!(has(&lines, "client print: later 42"), "{lines:?}");
    assert!(has(&lines, "server print: stuck false InvokeClient: Player left the game before RemoteFunction returned"), "{lines:?}");
}


/// With no harness a loopback run boots both sides from the place's own scripts: the server's Scripts (Legacy under
/// ServerScriptService and Workspace, Server-context anywhere but ServerStorage), the client's LocalScripts (ReplicatedFirst,
/// StarterPlayerScripts' copies in PlayerScripts) and Client-context Scripts; each once per side, a shared module once per
/// side, and a script on the wrong side never
#[test]
fn a_place_boots_both_sides_from_its_own_scripts() {
    let root = scratch("boot");
    for (path, text) in [
        ("ReplicatedStorage/Shared/Counter.luau", "local Count = 0 return { Bump = function() Count += 1 return Count end }"),
        ("ServerScriptService/Main.server.luau", "local Counter = require(game:GetService('ReplicatedStorage').Shared.Counter) print('server main', game:GetService('RunService'):IsServer(), Counter.Bump())"),
        ("Workspace/Spinner.server.luau", "print('workspace script', game:GetService('RunService'):IsServer())"),
        ("ServerStorage/Stored.server.luau", "print('WRONG: a ServerStorage script ran')"),
        ("ReplicatedStorage/Stray.client.luau", "print('WRONG: a LocalScript in ReplicatedStorage ran')"),
        ("ReplicatedFirst/First.client.luau", "print('client first', game:GetService('RunService'):IsClient())"),
        ("StarterPlayer/StarterPlayerScripts/Main.client.luau", "local Counter = require(game:GetService('ReplicatedStorage').Shared.Counter) print('client main', Counter.Bump(), script.Parent.ClassName) task.wait(0.1) print('@done')"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let dir = scratch("boot-run");
    let words: Vec<String> = ["--roots", &root.to_string_lossy(), "--seconds", "1", "--run-context", "loopback"].iter().map(|s| s.to_string()).collect();
    let args = run::parse(&words, false).unwrap();
    let summary = run::run_only(&args, &Host::default()).unwrap();
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    for wanted in ["server print: server main true 1", "server print: workspace script true", "client print: client first true", "client print: client main 1 PlayerScripts"] {
        assert_eq!(lines.iter().filter(|line| *line == wanted).count(), 1, "{wanted}: {lines:?}");
    }
    assert!(!lines.iter().any(|line| line.contains("WRONG")), "{lines:?}");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--latency 2`: what one side sends reaches the other a frame later than by default (an echo from the other side takes
/// two frames more in all); the default is the next frame (measured: an echo comes back at the sender's next frame start)
#[test]
fn latency_delays_each_way_by_its_frames() {
    let harness = r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		local Echo = Instance.new("RemoteEvent")
		Echo.Name = "Echo"
		Echo.Parent = Storage
		Echo.OnServerEvent:Connect(function(Player, Index)
			Echo:FireClient(Player, Index)
		end)
	end,
	Client = function(Context)
		local Echo = Storage:WaitForChild("Echo")
		local Frame = 0
		RunService.PreSimulation:Connect(function()
			Frame += 1
		end)
		local Sent
		Echo.OnClientEvent:Connect(function()
			print("round trip", Frame - Sent)
			print("@done")
		end)
		RunService.Heartbeat:Once(function()
			Sent = Frame
			Echo:FireServer(1)
		end)
	end,
}
"##;
    let plain = lines(&run_pair("latency1", harness, None, &[]));
    assert!(has(&plain, "client print: round trip 0"), "{plain:?}");
    let slow = lines(&run_pair("latency2", harness, None, &["--latency", "2"]));
    assert!(has(&slow, "client print: round trip 2"), "{slow:?}");
}

/// Context.HoldJoin keeps the player out until the server's harness calls Context.Join: the server steps alone meanwhile,
/// and the client's harness starts at the join, its clock from 0 then (a client's DataModel starts after the server's)
#[test]
fn a_held_join_waits_for_the_servers_harness() {
    let summary = run_pair(
        "held",
        r##"
local RunService = game:GetService("RunService")
local Players = game:GetService("Players")
return {
	Server = function(Context)
		Context.HoldJoin()
		for _ = 1, 5 do
			RunService.Heartbeat:Wait()
		end
		print("players before the join", #Players:GetPlayers(), math.floor(time() * 60 + 0.5))
		Context.Join()
		RunService.Heartbeat:Wait()
		RunService.Heartbeat:Wait()
		print("players after", #Players:GetPlayers())
	end,
	Client = function(Context)
		print("client starts at", math.floor(time() * 60 + 0.5))
		task.wait(0.1)
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: players before the join 0 5"), "{lines:?}");
    assert!(has(&lines, "server print: players after 1"), "{lines:?}");
    assert!(has(&lines, "client print: client starts at 0"), "{lines:?}");
}

/// Two loopback recordings of one harness are byte-identical (scene, playback, events, camera, summary); events.json holds
/// both sides' lines, each with its side; `--record-side server` records the server's world
#[test]
fn loopback_recordings_are_deterministic() {
    let dir = scratch("record");
    let harness = dir.join("Recorded.luau");
    std::fs::write(
        &harness,
        r##"
local Storage = game:GetService("ReplicatedStorage")
return {
	Server = function(Context)
		local Remote = Instance.new("RemoteEvent")
		Remote.Name = "Remote"
		Remote.Parent = Storage
		local Part = Instance.new("Part")
		Part.Name = "Mover"
		Part.Anchored = true
		Part.Parent = workspace
		Remote.OnServerEvent:Connect(function(Player, X)
			Part.Position = Vector3.new(X, 0, 0)
			print("moved", X)
		end)
	end,
	Client = function(Context)
		local Remote = Storage:WaitForChild("Remote")
		for X = 1, 5 do
			Remote:FireServer(X)
			task.wait(0.05)
		end
		local Own = Instance.new("Part")
		Own.Name = "ClientOnly"
		Own.Parent = workspace
	end,
}
"##,
    )
    .unwrap();
    let empty = dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let record = |out: &Path, extra: &[&str]| {
        let mut words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &empty.to_string_lossy(), "--seconds", "0.5", "--run-context", "loopback", "--out", &out.to_string_lossy()]
            .iter()
            .map(|s| s.to_string())
            .collect();
        words.extend(extra.iter().map(|s| s.to_string()));
        let args = run::parse(&words, true).unwrap();
        run::record(&args, &Host::default()).unwrap()
    };
    let (a, b, server) = (dir.join("a"), dir.join("b"), dir.join("server"));
    let summary = record(&a, &[]);
    record(&b, &[]);
    for file in ["scene.rbxm", "playback.json", "events.json", "camera.json", "summary.json"] {
        let bytes = |root: &Path| std::fs::read(root.join(file)).unwrap();
        // (summary.json names its folder)
        if file == "summary.json" {
            let text = |root: &Path| String::from_utf8(bytes(root)).unwrap().replace(&root.to_string_lossy().replace('\\', "\\\\"), "OUT");
            assert_eq!(text(&a), text(&b), "{file}");
        } else {
            assert_eq!(bytes(&a), bytes(&b), "{file}");
        }
    }
    assert_eq!(summary["runContext"], "loopback");
    assert_eq!(summary["loopback"]["latency"], 1);
    let events = std::fs::read_to_string(a.join("events.json")).unwrap();
    assert!(events.contains("\"side\": \"server\"") && events.contains("moved"), "{events}");
    let scene = |root: &Path| std::fs::read(root.join("scene.rbxm")).unwrap();
    let has_name = |bytes: &[u8], name: &str| bytes.windows(name.len()).any(|window| window == name.as_bytes());
    assert!(has_name(&scene(&a), "ClientOnly") && has_name(&scene(&a), "Mover"), "the client's world holds both");
    record(&server, &["--record-side", "server"]);
    assert!(has_name(&scene(&server), "Mover") && !has_name(&scene(&server), "ClientOnly"), "the server's world has no client part");
    let _ = std::fs::remove_dir_all(&dir);
}

/// CollisionGroups replicate from server to client at join and on later register/rename/unregister/collidable mutations,
/// and Players.PlayerAdded fires before Players.ChildAdded for a joining player (with Team already assigned).
#[test]
fn loopback_replicates_collision_groups_and_orders_player_added_before_child_added() {
    let summary = run_pair(
        "cg-team-order",
        r##"
local PhysicsService = game:GetService("PhysicsService")
local Players = game:GetService("Players")
local Teams = game:GetService("Teams")
local RunService = game:GetService("RunService")
return {
	Server = function()
		local Team = Instance.new("Team")
		Team.Name = "Red"
		Team.TeamColor = BrickColor.new("Bright red")
		Team.AutoAssignable = true
		Team.Parent = Teams
		local Order = {}
		Players.PlayerAdded:Connect(function(Player)
			table.insert(Order, "PlayerAdded:" .. (Player.Team and Player.Team.Name or "nil"))
		end)
		Players.ChildAdded:Connect(function(Child)
			if Child:IsA("Player") then
				table.insert(Order, "ChildAdded:" .. (Child.Team and Child.Team.Name or "nil"))
			end
		end)
		PhysicsService:RegisterCollisionGroup("Early")
		PhysicsService:CollisionGroupSetCollidable("Default", "Early", false)
		task.wait(0.1)
		print("order", table.concat(Order, ","))
		PhysicsService:RegisterCollisionGroup("Late")
		PhysicsService:CollisionGroupSetCollidable("Early", "Late", false)
		task.wait(0.1)
		PhysicsService:RenameCollisionGroup("Early", "Renamed")
		PhysicsService:UnregisterCollisionGroup("Late")
	end,
	Client = function()
		print("join", PhysicsService:IsCollisionGroupRegistered("Early"), PhysicsService:CollisionGroupsAreCollidable("Default", "Early"))
		for _ = 1, 10 do
			RunService.Heartbeat:Wait()
		end
		print("late", PhysicsService:IsCollisionGroupRegistered("Late"), PhysicsService:CollisionGroupsAreCollidable("Early", "Late"))
		for _ = 1, 10 do
			RunService.Heartbeat:Wait()
		end
		print("changed", PhysicsService:IsCollisionGroupRegistered("Renamed"), PhysicsService:IsCollisionGroupRegistered("Early"), PhysicsService:IsCollisionGroupRegistered("Late"))
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: order PlayerAdded:Red,ChildAdded:Red"), "{lines:?}");
    assert!(has(&lines, "client print: join true false"), "{lines:?}");
    assert!(has(&lines, "client print: late true false"), "{lines:?}");
    assert!(has(&lines, "client print: changed true false false"), "{lines:?}");
}

/// A frame's collision-group changes reach the client as one snapshot of the registry as the frame left it: 20 registers and
/// unregisters of a part's group and one more register change the client's copy of its CollisionGroupId once (it changed 41
/// times, a snapshot a change, the client taking every part's id in each time). A snapshot in which no group's name changed (a
/// collidable pair) leaves the ids, as CollisionGroupSetCollidable leaves them on the server: a client script's write stays.
#[test]
fn a_frames_collision_group_changes_reach_the_client_once() {
    let summary = run_pair(
        "cg-once",
        r##"
local PhysicsService = game:GetService("PhysicsService")
local RunService = game:GetService("RunService")
return {
	Server = function()
		local Part = Instance.new("Part")
		Part.Name = "Grouped"
		Part.Anchored = true
		Part.CollisionGroup = "G"
		Part.Parent = workspace
		task.wait(0.2)
		for _ = 1, 20 do
			PhysicsService:RegisterCollisionGroup("G")
			PhysicsService:UnregisterCollisionGroup("G")
		end
		PhysicsService:RegisterCollisionGroup("G")
		task.wait(0.3)
		PhysicsService:CollisionGroupSetCollidable("G", "Default", false)
	end,
	Client = function()
		local Part = workspace:WaitForChild("Grouped")
		local Changes = 0
		Part:GetPropertyChangedSignal("CollisionGroupId"):Connect(function()
			Changes += 1
		end)
		print("start", Part.CollisionGroupId)
		while not PhysicsService:IsCollisionGroupRegistered("G") do
			RunService.Heartbeat:Wait()
		end
		for _ = 1, 3 do
			RunService.Heartbeat:Wait()
		end
		print("registered", Part.CollisionGroupId, Changes)
		Part.CollisionGroupId = 9
		while PhysicsService:CollisionGroupsAreCollidable("G", "Default") do
			RunService.Heartbeat:Wait()
		end
		for _ = 1, 3 do
			RunService.Heartbeat:Wait()
		end
		print("collidable", Part.CollisionGroupId)
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: start -1"), "{lines:?}");
    assert!(has(&lines, "client print: registered 1 1"), "{lines:?}");
    assert!(has(&lines, "client print: collidable 9"), "{lines:?}");
}

/// A server WorldModel's own collision groups reach the client's copy of it (Studio 0.748, a playing client and its server):
/// at the join, when it comes into the Workspace, and when they change later; the client may not change them.
#[test]
fn loopback_replicates_a_world_models_own_collision_groups() {
    let summary = run_pair(
        "cg-world-model",
        r##"
local RunService = game:GetService("RunService")
local function Groups(Root)
	local Parts = {}
	for _, Group in Root:GetRegisteredCollisionGroups() do
		table.insert(Parts, Group.name .. "=" .. Group.mask)
	end
	return table.concat(Parts, " ")
end
return {
	Server = function()
		local Early = Instance.new("WorldModel")
		Early.Name = "Early"
		Early:RegisterCollisionGroup("EarlyA")
		Early.Parent = workspace
		task.wait(0.1)
		local World = Instance.new("WorldModel")
		World.Name = "World"
		World:RegisterCollisionGroup("RepA")
		World:RegisterCollisionGroup("RepB")
		World:CollisionGroupSetCollidable("RepA", "RepB", false)
		World.Parent = workspace
		task.wait(0.2)
		World:RegisterCollisionGroup("RepC")
		World:UnregisterCollisionGroup("RepA")
	end,
	Client = function()
		print("join", Groups(workspace:WaitForChild("Early")))
		local World = workspace:WaitForChild("World")
		print("came", Groups(World))
		for _ = 1, 20 do
			RunService.Heartbeat:Wait()
		end
		print("later", Groups(World), select(2, pcall(World.RegisterCollisionGroup, World, "X")))
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: join Default=-1 EarlyA=-1"), "{lines:?}");
    assert!(has(&lines, "client print: came Default=-1 RepA=-5 RepB=-3"), "{lines:?}");
    assert!(has(&lines, "client print: later Default=-1 RepB=-1 RepC=-1 This API can only be used on the server!"), "{lines:?}");
}

/// A client's copy of a server WorldModel takes its parts' ids again only when a group's name changed, as the DataModel's
/// registry does: a client script's CollisionGroupId write on a part of the copy stays through the server's change of a
/// collidable pair there, and the rename of another group, which leaves the part's id on the server (so nothing replicates
/// it), works the id out again on the client: the server's 1 replaces the 9 the client wrote.
#[test]
fn a_world_models_collidable_pair_leaves_a_clients_ids_and_a_rename_does_not() {
    let summary = run_pair(
        "cg-world-pairs",
        r##"
local RunService = game:GetService("RunService")
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
end
return {
	Server = function()
		local World = Instance.new("WorldModel")
		World.Name = "Pairs"
		World:RegisterCollisionGroup("G")
		World:RegisterCollisionGroup("K")
		local Part = Instance.new("Part")
		Part.Name = "P"
		Part.Anchored = true
		Part.CollisionGroup = "G"
		Part.Parent = World
		World.Parent = workspace
		task.wait(0.3)
		World:CollisionGroupSetCollidable("G", "Default", false)
		task.wait(0.3)
		World:RenameCollisionGroup("K", "L")
	end,
	Client = function()
		local World = workspace:WaitForChild("Pairs")
		local Part = World:WaitForChild("P")
		Frames(3)
		print("start", Part.CollisionGroupId)
		Part.CollisionGroupId = 9
		while World:CollisionGroupsAreCollidable("G", "Default") do
			RunService.Heartbeat:Wait()
		end
		Frames(3)
		print("collidable", Part.CollisionGroupId)
		while not World:IsCollisionGroupRegistered("L") do
			RunService.Heartbeat:Wait()
		end
		Frames(3)
		print("renamed", Part.CollisionGroupId)
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: start 1"), "{lines:?}");
    assert!(has(&lines, "client print: collidable 9"), "{lines:?}");
    assert!(has(&lines, "client print: renamed 1"), "{lines:?}");
}

/// A WorldModel's own groups reach a loopback client in the place of their first change in a frame, as the DataModel's do,
/// and a copy coming back into the replicated containers takes the groups it has now before it enters, so a client that
/// follows groups fires a part's CollisionGroupId Changed as often as its server does. In one server frame a WorldModel in the
/// Workspace registers X and a Workspace part of group X moves into it: the server fires Changed once, and the client did three
/// times (the replicated id, the move worked out against the copy's old groups, then the groups sent at the batch's end). A
/// WorldModel taken to ServerStorage registers W, which reaches its part there, and comes back: the client's copy has W (before,
/// it never got W), and the 9 a client script wrote to the part's id while it was away changes once, to 1, as the copy takes
/// W before it enters (with W after its Parent, the part took -1 from the old groups as it entered, then the replicated 1).
#[test]
fn a_world_models_groups_reach_a_client_in_their_place_and_its_parts_change_as_often_as_on_the_server() {
    let summary = run_pair(
        "cg-world-order",
        r##"
local RunService = game:GetService("RunService")
local PhysicsService = game:GetService("PhysicsService")
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
end
local function Counted(Part)
	local Count = 0
	Part:GetPropertyChangedSignal("CollisionGroupId"):Connect(function()
		Count += 1
	end)
	return function()
		return Count
	end
end
return {
	Server = function()
		-- (the client follows groups from the first snapshot that holds more than Default)
		PhysicsService:RegisterCollisionGroup("Y")
		local World = Instance.new("WorldModel")
		World.Name = "Order"
		World.Parent = workspace
		local Part = Instance.new("Part")
		Part.Name = "P"
		Part.Anchored = true
		Part.CollisionGroup = "X"
		Part.Parent = workspace
		local Away = Instance.new("WorldModel")
		Away.Name = "Away"
		local Kept = Instance.new("Part")
		Kept.Name = "K"
		Kept.Anchored = true
		Kept.CollisionGroup = "W"
		Kept.Parent = Away
		Away.Parent = workspace
		local Moved, KeptChanged = Counted(Part), Counted(Kept)
		task.wait(0.5)
		World:RegisterCollisionGroup("X")
		Part.Parent = World
		task.wait(0.3)
		Away.Parent = game:GetService("ServerStorage")
		task.wait(0.3)
		Away:RegisterCollisionGroup("W")
		task.wait(0.3)
		Away.Parent = workspace
		-- (before the client's run ends: it waits for this move)
		print("moved", Moved(), Part.CollisionGroupId)
		print("back", KeptChanged(), Kept.CollisionGroupId)
	end,
	Client = function()
		local World = workspace:WaitForChild("Order")
		local Part = workspace:WaitForChild("P")
		local Away = workspace:WaitForChild("Away")
		local Kept = Away:WaitForChild("K")
		Frames(3)
		local Moved, KeptChanged = Counted(Part), Counted(Kept)
		print("before", Part.CollisionGroupId, Kept.CollisionGroupId)
		while Part.Parent ~= World do
			RunService.Heartbeat:Wait()
		end
		Frames(3)
		print("moved", Moved(), Part.CollisionGroupId)
		while Away.Parent ~= nil do
			RunService.Heartbeat:Wait()
		end
		-- (a client script's write while the copy is away, counted from after its own Changed)
		Kept.CollisionGroupId = 9
		Frames(1)
		local Away9 = KeptChanged()
		while Away.Parent ~= workspace do
			RunService.Heartbeat:Wait()
		end
		Frames(3)
		print("back", KeptChanged() - Away9, Kept.CollisionGroupId, Away:IsCollisionGroupRegistered("W"))
		print("@done")
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    assert_eq!(summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: moved 1 1"), "{lines:?}");
    assert!(has(&lines, "server print: back 1 1"), "{lines:?}");
    assert!(has(&lines, "client print: before -1 -1"), "{lines:?}");
    assert!(has(&lines, "client print: moved 1 1"), "{lines:?}");
    assert!(has(&lines, "client print: back 1 1 true"), "{lines:?}");
}

/// A manifest bundle source for the bundle tests: a model file per key, uploaded bundles, and each fetch noted
#[cfg(feature = "bundles")]
struct Bundles {
    files: std::rc::Rc<Vec<(String, PathBuf)>>,
    fetched: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    /// The keys asked for without fetching ([`BundleSource::cached`])
    peeked: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    /// What the source's files hold
    shape: crate::project::model_assets::BundleShape,
    /// Whether it names a bundle's file before it holds it ([`BundleSource::expected`])
    names_files: bool,
    /// The files it downloaded: a missing file with a `<file>.download` beside it is copied from that when fetched
    downloads: std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>>,
}

#[cfg(feature = "bundles")]
impl crate::project::model_assets::BundleSource for Bundles {
    fn catalog(&mut self, _: &Path) -> Result<Vec<String>, String> {
        Ok(self.files.iter().map(|(key, _)| key.clone()).collect())
    }

    fn bundle(&mut self, _: &Path, key: &str) -> Result<PathBuf, String> {
        self.fetched.borrow_mut().push(key.to_string());
        let file = self.files.iter().find(|(held, _)| held == key).map(|(_, file)| file.clone()).ok_or_else(|| format!("no bundle {key}"))?;
        // (one the source cannot fetch while its file is missing)
        if key.ends_with("Offline") && !file.is_file() {
            return Err("offline and not cached".into());
        }
        let download = PathBuf::from(format!("{}.download", file.display()));
        if !file.is_file() && download.is_file() {
            std::fs::copy(&download, &file).map_err(|error| error.to_string())?;
            self.downloads.borrow_mut().push(file.clone());
        }
        Ok(file)
    }

    fn cached(&mut self, _: &Path, key: &str) -> Option<PathBuf> {
        self.peeked.borrow_mut().push(key.to_string());
        self.files.iter().find(|(held, _)| held == key).map(|(_, file)| file.clone()).filter(|file| file.is_file())
    }

    fn expected(&mut self, _: &Path, key: &str) -> Option<PathBuf> {
        self.files.iter().find(|(held, _)| held == key).map(|(_, file)| file.clone()).filter(|_| self.names_files)
    }

    fn shape(&self) -> crate::project::model_assets::BundleShape {
        self.shape
    }
}

/// A project with an asset manifest (its bundles come from [`Bundles`]): a machine (a Model with a PrimaryPart, an attribute
/// and a tag) at three paths and a loose bundle of a model and a part. Returns the folder, the root to mount and the files.
#[cfg(feature = "bundles")]
#[inline(never)]
fn bundle_project(name: &str) -> (PathBuf, PathBuf, Vec<(String, PathBuf)>) {
    use rbx_dom_weak::{
        InstanceBuilder, WeakDom,
        types::{Attributes, Tags, Variant, Vector3},
    };
    let dir = scratch(name);
    let project = dir.join("project");
    let manifest = project.join(crate::project::model_assets::MANIFEST);
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(&manifest, "return {}\n").unwrap();
    let root = project.join("src");
    std::fs::create_dir_all(&root).unwrap();
    let write = |file: &Path, dom: &WeakDom| {
        let mut bytes = Vec::new();
        rbx_binary::to_writer(&mut bytes, dom, dom.root().children()).unwrap();
        std::fs::write(file, bytes).unwrap();
    };
    let machine = dir.join("machine.rbxm");
    {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let mut attributes = Attributes::new();
        attributes.insert("Reach".into(), Variant::Float64(5.0));
        let mut tags = Tags::new();
        tags.push("Machine");
        let model = dom.insert(dom.root_ref(), InstanceBuilder::new("Model").with_name("Machine").with_property("Attributes", attributes).with_property("Tags", tags));
        let body = dom.insert(model, InstanceBuilder::new("Part").with_name("Body").with_property("Size", Vector3::new(4., 5., 6.)));
        dom.insert(model, InstanceBuilder::new("Part").with_name("Lamp"));
        dom.get_by_ref_mut(model).unwrap().properties.insert("PrimaryPart".into(), Variant::Ref(body));
        write(&machine, &dom);
    }
    let loose = dir.join("loose.rbxm");
    {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let content = dom.insert(dom.root_ref(), InstanceBuilder::new("Folder").with_name("Content"));
        dom.insert(content, InstanceBuilder::new("Model").with_name("Sword"));
        dom.insert(content, InstanceBuilder::new("Part").with_name("Shield"));
        write(&loose, &dom);
    }
    // a folder bundle (a Folder with an attribute and a model) and a bundle at a path below it
    let units = dir.join("units.rbxm");
    {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let mut attributes = Attributes::new();
        attributes.insert("Version".into(), Variant::Float64(3.0));
        let folder = dom.insert(dom.root_ref(), InstanceBuilder::new("Folder").with_name("Units").with_property("Attributes", attributes));
        let old = dom.insert(folder, InstanceBuilder::new("Model").with_name("Old"));
        dom.insert(old, InstanceBuilder::new("Part").with_name("Torso"));
        write(&units, &dom);
    }
    let nested = dir.join("nested.rbxm");
    {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let model = dom.insert(dom.root_ref(), InstanceBuilder::new("Model").with_name("Nested"));
        dom.insert(model, InstanceBuilder::new("Part").with_name("Head"));
        write(&nested, &dom);
    }
    let files = vec![
        ("Models/Items/_Loose".to_string(), loose),
        // (a bundle whose file is missing: it does not load)
        ("Models/Machines/Broken".to_string(), dir.join("missing.rbxm")),
        ("Models/Machines/ServerFirst".to_string(), machine.clone()),
        ("Models/Machines/Touched".to_string(), machine.clone()),
        ("Models/Machines/Untouched".to_string(), machine),
        ("Models/Units".to_string(), units),
        ("Models/Units/Nested".to_string(), nested),
    ];
    (dir, root, files)
}

/// `ree run --run-context loopback` of `harness` on [`bundle_project`]'s project with its bundles mounted at `mount`
/// (`--assets-at`): the run, its emulator (which holds the server) and the bundles fetched, in order
#[cfg(feature = "bundles")]
fn run_bundles(name: &str, harness: &str, mount: &str, extra: &[&str]) -> (run::Done, crate::Emulator, Vec<String>, PathBuf) {
    run_bundles_in(name, harness, mount, extra, "1", true)
}

/// [`run_bundles`] for `seconds`, a loopback run or a plain one
#[cfg(feature = "bundles")]
fn run_bundles_in(name: &str, harness: &str, mount: &str, extra: &[&str], seconds: &str, loopback: bool) -> (run::Done, crate::Emulator, Vec<String>, PathBuf) {
    let (dir, root, files) = bundle_project(name);
    let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host(files, &fetched);
    let file = dir.join("Harness.luau");
    std::fs::write(&file, harness).unwrap();
    let mut words: Vec<String> = ["--harness", &file.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", seconds, "--assets-at", mount, "--assets", "Models"].iter().map(|s| s.to_string()).collect();
    if loopback {
        words.extend(["--run-context".to_string(), "loopback".to_string()]);
    }
    words.extend(extra.iter().map(|s| s.to_string()));
    let args = run::parse(&words, false).unwrap();
    let (done, emulator) = run::execute(&args, &host, false).unwrap();
    let fetched = fetched.borrow().clone();
    (done, emulator, fetched, dir)
}

/// The instance at `path` (names from the service `service`) of `engine`, as its dom holds it (no script's look)
#[cfg(feature = "bundles")]
fn at_path(engine: &crate::Engine, service: &str, path: &[&str]) -> Option<crate::InstanceId> {
    let start = crate::services::find_service(engine, engine.world().game, service)?;
    let state = engine.state.borrow();
    path.iter().try_fold(start, |at, name| state.dom.find_child(at, name.as_bytes()))
}

/// The class name and child count of `id` in `engine`
#[cfg(feature = "bundles")]
fn shape(engine: &crate::Engine, id: crate::InstanceId) -> (String, usize) {
    let state = engine.state.borrow();
    let instance = state.dom.at(id);
    (engine.reflection.class(instance.class).name.to_string(), instance.children.len())
}

/// A test host whose engines take their bundles from [`Bundles`], each fetch noted in `fetched`
#[cfg(feature = "bundles")]
fn bundle_host(files: Vec<(String, PathBuf)>, fetched: &std::rc::Rc<std::cell::RefCell<Vec<String>>>) -> Host {
    bundle_host_peeking(files, fetched, &Default::default())
}

/// [`bundle_host`], noting in `peeked` each key the source is asked for without fetching
#[cfg(feature = "bundles")]
fn bundle_host_peeking(files: Vec<(String, PathBuf)>, fetched: &std::rc::Rc<std::cell::RefCell<Vec<String>>>, peeked: &std::rc::Rc<std::cell::RefCell<Vec<String>>>) -> Host {
    bundle_host_as(files, fetched, peeked, crate::project::model_assets::BundleShape::Uploaded, true, &Default::default())
}

/// [`bundle_host_peeking`] of a source whose files hold `shape`, that names its files or not (`names_files`), noting in
/// `downloads` the files it downloads
#[cfg(feature = "bundles")]
fn bundle_host_as(
    files: Vec<(String, PathBuf)>, fetched: &std::rc::Rc<std::cell::RefCell<Vec<String>>>, peeked: &std::rc::Rc<std::cell::RefCell<Vec<String>>>, shape: crate::project::model_assets::BundleShape, names_files: bool,
    downloads: &std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>>,
) -> Host {
    let files = std::rc::Rc::new(files);
    let (fetched, peeked, downloads) = (fetched.clone(), peeked.clone(), downloads.clone());
    Host {
        new_engine: std::rc::Rc::new(move |options| {
            let engine = crate::Engine::new(options)?;
            engine.set_bundle_source(Box::new(Bundles { files: files.clone(), fetched: fetched.clone(), peeked: peeked.clone(), shape, names_files, downloads: downloads.clone() }));
            Ok(engine)
        }),
        ..Host::default()
    }
}

/// `ree run` of the harness `text` (a plain run, or a loopback one) on the project at `root`, with `host`'s bundles mounted at
/// ReplicatedStorage.Library: its summary's lines, and the emulator
#[cfg(feature = "bundles")]
fn run_on(dir: &Path, root: &Path, host: &Host, text: &str, loopback: bool, extra: &[&str]) -> (Vec<String>, crate::Emulator) {
    // (`--assets-at` and `--seconds` among `extra` take the place of the defaults)
    let harness = dir.join(format!("Harness-{}.luau", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::write(&harness, text).unwrap();
    let given = |flag: &str| extra.iter().position(|word| *word == flag).and_then(|at| extra.get(at + 1)).copied();
    let (mount, seconds) = (given("--assets-at").unwrap_or("ReplicatedStorage.Library"), given("--seconds").unwrap_or("1"));
    let extra: Vec<&str> = extra.chunks(2).filter(|pair| pair[0] != "--assets-at" && pair[0] != "--seconds").flatten().copied().collect();
    let mut words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", seconds, "--assets-at", mount, "--assets", "Models"].iter().map(|s| s.to_string()).collect();
    if loopback {
        words.extend(["--run-context".to_string(), "loopback".to_string()]);
    }
    words.extend(extra.iter().map(|s| s.to_string()));
    let args = run::parse(&words, false).unwrap();
    let (done, emulator) = run::execute(&args, host, false).unwrap();
    let mut lines = lines(&done.summary);
    lines.insert(0, format!("status {}", done.summary["status"].as_str().unwrap_or("?")));
    (lines, emulator)
}

/// The manifest of [`bundle_project`]'s project at `root`, rewritten to pin an asset version (ids from `base`) for each of `keys`
#[cfg(feature = "bundles")]
fn pin_versions(root: &Path, base: u64, keys: &[&str]) {
    let manifest = root.parent().unwrap().join(crate::project::model_assets::MANIFEST);
    let lines: Vec<String> = keys.iter().enumerate().map(|(at, key)| format!("\t[\"{key}\"] = {{ AssetId = {}, Version = 1 }},", base + at as u64)).collect();
    std::fs::write(manifest, format!("return {{\n{}\n}}\n", lines.join("\n"))).unwrap();
}

/// An asset id base no other run of the tests uses (the tag index in the state dir is shared)
#[cfg(feature = "bundles")]
fn fresh_asset_ids() -> u64 {
    9_000_000_000 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() % 1_000_000_000) as u64 * 100
}

/// Manifest bundles stay lazy across the pair: the join copies a waiting bundle's placeholder and fetches nothing. What the
/// client finds or looks inside, the server imports then, and the client's copy takes what it made at once (the class,
/// values, attributes, tags and children a plain client run finds, a loose bundle's members in their folder); what the server
/// finds first goes to the client's copy as it is made, and its write after it at the client's next frame; each side's bundle
/// instances are the other's counterparts (a remote's argument maps), and the server's writes to them replicate. A bundle
/// nothing looked at is never fetched or made on either side.
#[cfg(feature = "bundles")]
#[test]
fn bundles_stay_lazy_and_the_server_imports_what_either_side_looks_at() {
    let (dir, root, files) = bundle_project("bundles");
    let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host(files, &fetched);
    let harness = dir.join("Bundles.luau");
    std::fs::write(
        &harness,
        r##"
local Storage = game:GetService("ReplicatedStorage")
return {
	Server = function(Context)
		local Remote = Instance.new("RemoteEvent")
		Remote.Name = "Ping"
		Remote.Parent = Storage
		Remote.OnServerEvent:Connect(function(Player, Body)
			local Touched = Storage.Library.Models.Machines.Touched
			print("server got", Body == Touched.Body, Body:GetFullName())
			Body.Transparency = 0.5
			Remote:FireAllClients("written")
		end)
		task.wait(0.2)
		local Machine = Storage.Library.Models.Machines.ServerFirst
		Machine.Lamp.Transparency = 0.25
		Remote:FireAllClients("server first")
	end,
	Client = function(Context)
		local Remote = Storage:WaitForChild("Ping")
		local Machines = Storage.Library.Models.Machines
		local Touched = Machines.Touched
		print("client touched", Touched.ClassName, Touched.PrimaryPart == Touched.Body, Touched.Body.Size, Touched:GetAttribute("Reach"), Touched:HasTag("Machine"), #Touched:GetChildren(), Machines:FindFirstChild("Touched") == Touched)
		local Names = {}
		for _, Item in Storage.Library.Models.Items:GetChildren() do
			table.insert(Names, Item.Name .. ":" .. Item.ClassName)
		end
		table.sort(Names)
		print("client items", table.concat(Names, " "))
		Remote.OnClientEvent:Connect(function(Tag)
			if Tag == "written" then
				print("client written", Touched.Body.Transparency)
			else
				local Machine = Machines.ServerFirst
				print("client server first", Machine.ClassName, Machine.Lamp.Transparency, Machine.PrimaryPart == Machine.Body, Machine:GetAttribute("Reach"))
				print("@done")
			end
		end)
		Remote:FireServer(Touched.Body)
	end,
}
"##,
    )
    .unwrap();
    let words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", "1", "--run-context", "loopback", "--assets-at", "ReplicatedStorage.Library", "--assets", "Models"].iter().map(|s| s.to_string()).collect();
    let args = run::parse(&words, false).unwrap();
    let (done, emulator) = run::execute(&args, &host, false).unwrap();
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: client touched Model true 4, 5, 6 5 true 2 true"), "{lines:?}");
    assert!(has(&lines, "client print: client items Shield:Part Sword:Model"), "{lines:?}");
    assert!(has(&lines, "server print: server got true ReplicatedStorage.Library.Models.Machines.Touched.Body"), "{lines:?}");
    assert!(has(&lines, "client print: client written 0.5"), "{lines:?}");
    assert!(has(&lines, "client print: client server first Model 0.25 true 5"), "{lines:?}");
    // fetched in the order looked at, each once: nothing at the join, the untouched one never
    assert_eq!(*fetched.borrow(), ["Models/Machines/Touched", "Models/Items/_Loose", "Models/Machines/ServerFirst"], "fetched");
    let (_, server) = emulator.engine.peer_server().unwrap();
    let client = &emulator.engine;
    let untouched = |engine: &crate::Engine| {
        let state = engine.state.borrow();
        let storage = crate::services::find_service(engine, engine.world().game, "ReplicatedStorage").unwrap();
        let machines = ["Library", "Models", "Machines"].iter().try_fold(storage, |at, name| state.dom.find_child(at, name.as_bytes())).unwrap();
        state.dom.find_child(machines, b"Untouched").unwrap()
    };
    let (server_untouched, client_untouched) = (untouched(&server), untouched(client));
    for (side, engine, id) in [("server", &server, server_untouched), ("client", client, client_untouched)] {
        let state = engine.state.borrow();
        let instance = state.dom.at(id);
        assert!(instance.has(crate::dom::flags::BUNDLE_ROOT) && &*engine.reflection.class(instance.class).name == "Folder", "the {side}'s untouched bundle was made");
        assert!(instance.children.is_empty(), "the {side}'s untouched bundle has children");
    }
    assert!(server.state.borrow().project.model_assets.waits_at(server_untouched), "the server's untouched bundle waits no more");
    assert!(client.state.borrow().project.model_assets.waits_remotely(client_untouched), "the client's copy waits no more");
    assert_eq!(crate::loopback::counterpart(client, client_untouched), Some(server_untouched));
    drop(server);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A bundle that waits in a world (here the Workspace) is imported at the join and copied whole, as before: a world's walks,
/// queries and recorder read it with no script looking, so the client's copy is the bundle's own from the start
#[cfg(feature = "bundles")]
#[test]
fn a_bundle_waiting_in_a_world_is_imported_at_the_join() {
    let (dir, root, files) = bundle_project("bundles-world");
    let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host(files, &fetched);
    let harness = dir.join("World.luau");
    std::fs::write(&harness, "return { Server = function(Context) end, Client = function(Context) print('@done') end }").unwrap();
    let words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", "1", "--run-context", "loopback", "--assets-at", "Workspace.Library", "--assets", "Models"].iter().map(|s| s.to_string()).collect();
    let args = run::parse(&words, false).unwrap();
    let (done, emulator) = run::execute(&args, &host, false).unwrap();
    assert_eq!(done.summary["status"], "ok", "{:?}", lines(&done.summary));
    assert_eq!(fetched.borrow().len(), 7, "every bundle in the Workspace was imported at the join: {:?}", fetched.borrow());
    let client = &emulator.engine;
    let state = client.state.borrow();
    let machines = ["Library", "Models", "Machines"].iter().try_fold(client.world().workspace, |at, name| state.dom.find_child(at, name.as_bytes())).unwrap();
    let untouched = state.dom.find_child(machines, b"Untouched").unwrap();
    let instance = state.dom.at(untouched);
    assert_eq!(&*client.reflection.class(instance.class).name, "Model", "the client's copy is the bundle's root");
    assert!(!instance.has(crate::dom::flags::BUNDLE_ROOT) && instance.children.len() == 2, "the client's copy is whole");
    assert!(!state.project.model_assets.waits_remotely(untouched), "the client's copy waits for its server");
    drop(state);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A child a server script parents into a folder where loose bundles wait arrives as the server's changes do (its ChildAdded at
/// the client's next frame), and the bundles' members come first in both lists, as in the game, where they are in place: the
/// parenting imports them, and the client's copy takes them at once (no event); the import copies only what it made
#[cfg(feature = "bundles")]
#[test]
fn a_script_child_in_a_bundle_folder_arrives_as_a_change_after_the_bundles_members() {
    let (done, emulator, fetched, dir) = run_bundles(
        "bundles-script-child",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
end
local function Names(Folder)
	local Out = {}
	for _, Child in Folder:GetChildren() do
		table.insert(Out, Child.Name)
	end
	return table.concat(Out, " ")
end
return {
	Server = function(Context)
		Frames(3)
		local Items = Storage.Library.Models.Items
		local Made = Instance.new("Model")
		Made.Name = "Made"
		Made.Parent = Items
		print("server", Names(Items))
	end,
	Client = function(Context)
		local Items = Storage.Library.Models.Items
		local Added = {}
		Items.ChildAdded:Connect(function(Child)
			table.insert(Added, Child.Name)
		end)
		Frames(6)
		print("client", Names(Items), "added", table.concat(Added, " "))
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &[],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server Sword Shield Made"), "{lines:?}");
    assert!(has(&lines, "client print: client Sword Shield Made added Made"), "{lines:?}");
    assert_eq!(fetched, ["Models/Items/_Loose"]);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A bundle the server imports while its client's copy is on its way reaches the client with it (two frames of latency): one
/// revealed in the frame its folder came into the containers goes whole in that frame's tree, and one revealed in the next
/// frame, after that tree was sealed, is taken by the copy when the tree arrives, before it enters (its ChildAdded and
/// DescendantAdded include the bundle's instances)
#[cfg(feature = "bundles")]
#[test]
fn a_bundle_imported_before_its_tree_reaches_the_client_arrives_with_it() {
    let (done, emulator, fetched, dir) = run_bundles(
        "bundles-tree",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		task.wait(0.1)
		game:GetService("ServerStorage").Library.Models.Parent = Storage
		local Touched = Storage.Models.Machines.Touched
		print("server touched", Touched.ClassName, #Touched:GetChildren())
		task.wait()
		local Late = Storage.Models.Machines.ServerFirst
		print("server late", Late.ClassName, #Late:GetChildren())
	end,
	Client = function(Context)
		local Added = 0
		Storage.DescendantAdded:Connect(function()
			Added += 1
		end)
		local Machines = Storage:WaitForChild("Models"):WaitForChild("Machines")
		local Touched, Late = Machines.Touched, Machines.ServerFirst
		print("client", Touched.ClassName, #Touched:GetChildren(), Late.ClassName, #Late:GetChildren(), Late.PrimaryPart == Late.Body)
		task.wait()
		print("client added", Added)
		print("@done")
	end,
}
"##,
        "ServerStorage.Library",
        &["--latency", "2"],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server touched Model 2"), "{lines:?}");
    assert!(has(&lines, "server print: server late Model 2"), "{lines:?}");
    assert!(has(&lines, "client print: client Model 2 Model 2 true"), "{lines:?}");
    // (the tree's Models, Items, Units and their folders, the three machines, Broken, both machines' two parts each)
    assert!(lines.iter().any(|line| line.starts_with("client print: client added ") && line != "client print: client added 0"), "{lines:?}");
    assert_eq!(fetched, ["Models/Machines/Touched", "Models/Machines/ServerFirst"]);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A subtree holding waiting bundles that enters a world after the join imports them first, on either side, as a world's walks
/// and recorder read it whole with no script looking: the server's move into its Workspace (its client's copy moving with it),
/// and the client's own move of its copy into its Workspace (its server imports, the copy takes the members)
#[cfg(feature = "bundles")]
#[test]
fn a_bundle_entering_a_world_after_the_join_is_imported_on_either_side() {
    let (done, emulator, fetched, dir) = run_bundles(
        "bundles-world-entry",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		task.wait(0.1)
		Storage.Library.Models.Machines.Parent = workspace
	end,
	Client = function(Context)
		Storage.Library.Models.Items.Parent = workspace
		workspace:WaitForChild("Machines")
		task.wait()
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &[],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    let (_, server) = emulator.engine.peer_server().unwrap();
    let client = &emulator.engine;
    for (side, engine) in [("server", &server), ("client", client)] {
        for name in ["Touched", "Untouched", "ServerFirst"] {
            let id = at_path(engine, "Workspace", &["Machines", name]).unwrap_or_else(|| panic!("{side} {name}"));
            assert_eq!(shape(engine, id), ("Model".to_string(), 2), "the {side}'s {name} in its world");
        }
    }
    let items = at_path(client, "Workspace", &["Items"]).unwrap();
    assert_eq!(shape(client, items), ("Folder".to_string(), 2), "the client's Items in its world");
    let server_items = at_path(&server, "ReplicatedStorage", &["Library", "Models", "Items"]).unwrap();
    assert_eq!(shape(&server, server_items), ("Folder".to_string(), 2), "the server's Items, imported for the client's move");
    assert!(fetched.iter().any(|key| key == "Models/Machines/Untouched") && fetched.iter().any(|key| key == "Models/Items/_Loose"), "{fetched:?}");
    assert!(!fetched.iter().any(|key| key == "Models/Units"), "a bundle that stayed out of the worlds: {fetched:?}");
    drop(server);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// CollectionService lists what waiting bundles hold as the game has them in place: GetTagged imports first the waiting bundles
/// whose instances carry the tag (and none other), GetAllTags those carrying any tag, on either side; a placeholder passed to
/// an engine method (AddTag) is the bundle's instance first
#[cfg(feature = "bundles")]
#[test]
fn collection_service_finds_the_tags_waiting_bundles_hold() {
    let (done, emulator, fetched, dir) = run_bundles(
        "bundles-tags",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local CollectionService = game:GetService("CollectionService")
return {
	Server = function(Context)
		task.wait(0.2)
		print("server tagged", #CollectionService:GetTagged("Machine"))
	end,
	Client = function(Context)
		local Tagged = CollectionService:GetTagged("Machine")
		local Names = {}
		for _, Machine in Tagged do
			table.insert(Names, Machine.Name .. ":" .. Machine.ClassName)
		end
		table.sort(Names)
		print("client tagged", table.concat(Names, " "))
		print("client none", #CollectionService:GetTagged("Nothing"), table.find(CollectionService:GetAllTags(), "Machine") ~= nil)
		-- (the folder bundle's first child is the nested bundle's placeholder, listed, not touched)
		local Placeholder = Storage.Library.Models.Units:GetChildren()[1]
		CollectionService:AddTag(Placeholder, "Seen")
		local Items = #Storage.Library.Models.Items:GetChildren()
		print("client added", Placeholder.ClassName, Placeholder:HasTag("Seen"), #Placeholder:GetChildren(), Items)
		task.wait(0.3)
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &[],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server tagged 3"), "{lines:?}");
    assert!(has(&lines, "client print: client tagged ServerFirst:Model Touched:Model Untouched:Model"), "{lines:?}");
    assert!(has(&lines, "client print: client none 0 true"), "{lines:?}");
    assert!(has(&lines, "client print: client added Model true 1 2"), "{lines:?}");
    // (each machine imported once, the tags read from the files the source holds; the placeholder AddTag took imported then,
    // before the look inside Items)
    assert_eq!(fetched.iter().filter(|key| *key == "Models/Machines/Untouched").count(), 1, "{fetched:?}");
    let last = |key: &str| fetched.iter().rposition(|held| held == key).unwrap_or_else(|| panic!("{key} was not fetched: {fetched:?}"));
    assert!(last("Models/Units") < last("Models/Units/Nested") && last("Models/Units/Nested") < last("Models/Items/_Loose"), "{fetched:?}");
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A bundle's copy takes it with no event, in the step the server imports it and as it was then (with two frames of latency, the
/// server's write right after the import reaches the client a step later), once (a look inside on the server, which runs the
/// Studio-children pass, makes no second copy)
#[cfg(feature = "bundles")]
#[test]
fn a_bundles_copy_fires_no_event_and_takes_it_in_the_step_it_is_imported() {
    let (done, emulator, fetched, dir) = run_bundles(
        "bundles-step",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
end
return {
	Server = function(Context)
		Frames(5)
		local Machine = Storage.Library.Models.Machines.ServerFirst
		Machine.Lamp.Transparency = 0.25
		print("server children", #Machine:GetChildren())
	end,
	Client = function(Context)
		local Added = 0
		Storage.Library.DescendantAdded:Connect(function()
			Added += 1
		end)
		local Touched = Storage.Library.Models.Machines.Touched
		print("client touched", #Touched:GetChildren())
		Frames(5)
		local Machine = Storage.Library.Models.Machines.ServerFirst
		print("client same step", Machine.ClassName, Machine.Lamp.Transparency, #Machine:GetChildren())
		Frames(1)
		print("client next step", Machine.Lamp.Transparency, #Machine:GetChildren())
		Frames(2)
		print("client added", Added)
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &["--latency", "2"],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: client touched 2"), "{lines:?}");
    assert!(has(&lines, "server print: server children 2"), "{lines:?}");
    assert!(has(&lines, "client print: client same step Model 0 2"), "{lines:?}");
    assert!(has(&lines, "client print: client next step 0.25 2"), "{lines:?}");
    assert!(has(&lines, "client print: client added 0"), "{lines:?}");
    assert_eq!(fetched, ["Models/Machines/Touched", "Models/Machines/ServerFirst"]);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A bundle that does not load leaves its copy a Folder (the server's warning, fetched once), a bundle at a path below a folder
/// bundle lands in it on both sides and pairs with the server's, and a copy the client destroyed takes nothing when the server
/// imports it later
#[cfg(feature = "bundles")]
#[test]
fn a_broken_a_nested_and_a_destroyed_bundle_copy() {
    let (done, emulator, fetched, dir) = run_bundles(
        "bundles-edges",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		local Remote = Instance.new("RemoteEvent")
		Remote.Name = "Ping"
		Remote.Parent = Storage
		Remote.OnServerEvent:Connect(function(Player, Head, Torso)
			local Units = Storage.Library.Models.Units
			print("server got", Head == Units.Nested.Head, Torso == Units.Old.Torso)
		end)
		task.wait(0.2)
		local Machine = Storage.Library.Models.Machines.ServerFirst
		Machine.Lamp.Transparency = 0.5
		print("server after", Machine.ClassName, #Machine:GetChildren())
	end,
	Client = function(Context)
		local Remote = Storage:WaitForChild("Ping")
		local Machines = Storage.Library.Models.Machines
		local Broken = Machines.Broken
		print("client broken", Broken.ClassName, #Broken:GetChildren(), Machines.Broken.ClassName)
		local Units = Storage.Library.Models.Units
		local Head = Units.Nested.Head
		print("client units", Units.ClassName, Units:GetAttribute("Version"), #Units:GetChildren(), Units.Nested.ClassName, Units.Old.Torso.ClassName)
		Remote:FireServer(Head, Units.Old.Torso)
		Machines:Destroy()
		task.wait(0.4)
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &[],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: client broken Folder 0 Folder"), "{lines:?}");
    assert!(lines.iter().any(|line| line.starts_with("server warn: modelAssets Models/Machines/Broken")), "{lines:?}");
    assert!(has(&lines, "client print: client units Folder 3 2 Model Part"), "{lines:?}");
    assert!(has(&lines, "server print: server got true true"), "{lines:?}");
    assert!(has(&lines, "server print: server after Model 2"), "{lines:?}");
    assert_eq!(fetched.iter().filter(|key| *key == "Models/Machines/Broken").count(), 1, "{fetched:?}");
    let (_, server) = emulator.engine.peer_server().unwrap();
    let machine = at_path(&server, "ReplicatedStorage", &["Library", "Models", "Machines", "ServerFirst"]).unwrap();
    let copy = crate::loopback::counterpart(&server, machine);
    assert!(copy.is_none_or(|copy| emulator.engine.state.borrow().dom.get(copy).is_none_or(|instance| instance.has(crate::dom::flags::DESTROYED) && instance.children.is_empty())), "the destroyed copy took the bundle");
    drop(server);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A waiting bundle a script destroyed (its folder cleared, the placeholders freed by the collector) is gone for
/// CollectionService: GetTagged and GetAllTags neither read nor import it, and nothing fails
#[cfg(feature = "bundles")]
#[test]
fn a_destroyed_bundle_is_gone_for_collection_service() {
    let (done, emulator, fetched, dir) = run_bundles_in(
        "bundles-destroyed",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local CollectionService = game:GetService("CollectionService")
return {
	Run = function(Context)
		Storage.Library.Models:ClearAllChildren()
		task.wait(1.5)
		print("tagged", #CollectionService:GetTagged("Machine"), #CollectionService:GetAllTags())
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &[],
        "3",
        false,
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "- print: tagged 0 0"), "{lines:?}");
    assert!(fetched.is_empty(), "a destroyed bundle was read: {fetched:?}");
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A bundle the server imports while its client copy is on its way (two frames of latency) reaches the client as it was at the
/// import, and what the server did after it in that frame follows as its changes: a child made under one of the bundle's
/// instances arrives once (its ChildAdded), a part moved out of it arrives where it went, a write arrives with its Changed
#[cfg(feature = "bundles")]
#[test]
fn an_import_while_its_copy_is_on_its_way_arrives_as_it_was_then() {
    let (done, emulator, _, dir) = run_bundles(
        "bundles-in-flight",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		task.wait(0.1)
		game:GetService("ServerStorage").Library.Models.Parent = Storage
		task.wait()
		local Machine = Storage.Models.Machines.ServerFirst
		local Extra = Instance.new("Part")
		Extra.Name = "Extra"
		Extra.Parent = Machine.Body
		Machine.Lamp.Parent = Storage
		Machine.Body.Transparency = 0.5
	end,
	Client = function(Context)
		local Machine = Storage:WaitForChild("Models"):WaitForChild("Machines"):WaitForChild("ServerFirst")
		local Body = Machine:WaitForChild("Body")
		local Changed, Added = 0, 0
		Body.Changed:Connect(function()
			Changed += 1
		end)
		Body.ChildAdded:Connect(function()
			Added += 1
		end)
		print("client arrived", Machine.ClassName, #Machine:GetChildren(), Body.Transparency, #Body:GetChildren())
		for _ = 1, 4 do
			RunService.Heartbeat:Wait()
		end
		print("client after", #Machine:GetChildren(), Body.Transparency, #Body:GetChildren(), Storage:FindFirstChild("Lamp") ~= nil, Changed > 0, Added)
		print("@done")
	end,
}
"##,
        "ServerStorage.Library",
        &["--latency", "2"],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: client arrived Model 2 0 0"), "{lines:?}");
    assert!(has(&lines, "client print: client after 1 0.5 1 true true 1"), "{lines:?}");
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Instance.new` with a parent where loose bundles wait imports them first, as a Parent write does: the bundles' members come
/// before the new instance on both sides
#[cfg(feature = "bundles")]
#[test]
fn instance_new_into_a_bundle_folder_comes_after_the_bundles_members() {
    let (done, emulator, _, dir) = run_bundles(
        "bundles-instance-new",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Names(Folder)
	local Out = {}
	for _, Child in Folder:GetChildren() do
		table.insert(Out, Child.Name)
	end
	return table.concat(Out, " ")
end
return {
	Server = function(Context)
		task.wait(0.1)
		local Items = Storage.Library.Models.Items
		Instance.new("Model", Items).Name = "Made"
		print("server", Names(Items))
	end,
	Client = function(Context)
		-- (no look inside Items before the server's Instance.new: the parenting is what imports)
		task.wait(0.3)
		print("client", Names(Storage.Library.Models.Items))
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &[],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "server print: server Sword Shield Made"), "{lines:?}");
    assert!(has(&lines, "client print: client Sword Shield Made"), "{lines:?}");
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A placeholder passed to an engine method read as a value and called later is the bundle's instance first, as a `:` call's is
#[cfg(feature = "bundles")]
#[test]
fn a_method_read_as_a_value_takes_a_placeholder_as_its_bundle() {
    let (done, emulator, fetched, dir) = run_bundles(
        "bundles-method-value",
        r##"
local Storage = game:GetService("ReplicatedStorage")
local CollectionService = game:GetService("CollectionService")
return {
	Client = function(Context)
		-- (the machines in their mount's order: Broken, ServerFirst, Touched, Untouched; listed, not touched)
		local Placeholder = Storage.Library.Models.Machines:GetChildren()[3]
		local AddTag = CollectionService.AddTag
		AddTag(CollectionService, Placeholder, "Seen")
		local Items = #Storage.Library.Models.Items:GetChildren()
		print("client", Placeholder.Name, Placeholder.ClassName, Placeholder:HasTag("Seen"), Items)
		print("@done")
	end,
}
"##,
        "ReplicatedStorage.Library",
        &[],
    );
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "client print: client Touched Model true 2"), "{lines:?}");
    assert_eq!(fetched, ["Models/Machines/Touched", "Models/Items/_Loose"]);
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A bundle whose tags cannot be read (one the source cannot fetch, one whose file does not load) is named in one warning, and
/// its missing file is noted among the run's reads, so a run prepared from it goes stale once the file is there
#[cfg(feature = "bundles")]
#[test]
fn a_bundle_whose_tags_cannot_be_read_is_warned_and_noted() {
    let (dir, root, mut files) = bundle_project("bundles-tags-unread");
    let offline = dir.join("offline.rbxm");
    files.push(("Models/Machines/Offline".to_string(), offline.clone()));
    let machine = files.iter().find(|(key, _)| key == "Models/Machines/Touched").map(|(_, file)| file.clone()).unwrap();
    let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host(files, &fetched);
    let harness = dir.join("Harness.luau");
    std::fs::write(&harness, "return function(Context) print('tagged', #game:GetService('CollectionService'):GetTagged('Machine')) print('@done') end").unwrap();
    let words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", "1", "--assets-at", "ReplicatedStorage.Library", "--assets", "Models"].iter().map(|s| s.to_string()).collect();
    let args = run::parse(&words, false).unwrap();
    crate::files::start_noting();
    let (done, emulator) = run::execute(&args, &host, false).unwrap();
    let noted = crate::files::stop_noting();
    let lines = lines(&done.summary);
    assert_eq!(done.summary["status"], "ok", "{lines:?}");
    assert!(has(&lines, "- print: tagged 3"), "{lines:?}");
    let warnings: Vec<&String> = lines.iter().filter(|line| line.starts_with("- warn: modelAssets: the tags of")).collect();
    assert_eq!(warnings.len(), 1, "{lines:?}");
    assert!(warnings[0].contains("the tags of 2 bundle(s)") && warnings[0].contains("Models/Machines/Offline") && warnings[0].contains("Models/Machines/Broken"), "{warnings:?}");
    assert!(noted.changed().is_none_or(|why| !why.contains("offline.rbxm")), "{:?}", noted.changed());
    std::fs::copy(&machine, &offline).unwrap();
    assert!(noted.changed().is_some_and(|why| why.contains("offline.rbxm")), "{:?}", noted.changed());
    drop(emulator);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bundles a run imported decode ahead in the next run of the same scene (the mount queues their files), a loopback run's
/// apart from a plain one's of the same harness: a loopback run, a plain run that looks at another bundle (on its first run,
/// the mount's list), then the loopback run again, which queues its own scene's
#[cfg(feature = "bundles")]
#[test]
fn a_loopback_runs_bundles_decode_ahead_apart_from_a_plain_runs() {
    let (dir, root, files) = bundle_project("bundles-ahead");
    let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let peeked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host_peeking(files, &fetched, &peeked);
    let harness = dir.join("Harness.luau");
    std::fs::write(
        &harness,
        r##"
local Storage = game:GetService("ReplicatedStorage")
return {
	Run = function(Context)
		print("plain", Storage.Library.Models.Machines.Untouched.ClassName)
		print("@done")
	end,
	Client = function(Context)
		print("client", Storage.Library.Models.Machines.Touched.ClassName)
		print("@done")
	end,
}
"##,
    )
    .unwrap();
    let run_once = |loopback: bool| -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut words: Vec<String> = ["--harness", &harness.to_string_lossy(), "--roots", &root.to_string_lossy(), "--seconds", "1", "--assets-at", "ReplicatedStorage.Library", "--assets", "Models"].iter().map(|s| s.to_string()).collect();
        if loopback {
            words.extend(["--run-context".to_string(), "loopback".to_string()]);
        }
        fetched.borrow_mut().clear();
        peeked.borrow_mut().clear();
        let args = run::parse(&words, false).unwrap();
        let (done, emulator) = run::execute(&args, &host, false).unwrap();
        drop(emulator);
        (lines(&done.summary), fetched.borrow().clone(), peeked.borrow().clone())
    };
    // (the first: nothing to decode ahead)
    let (first, first_fetched, first_peeked) = run_once(true);
    assert!(has(&first, "client print: client Model"), "{first:?}");
    assert_eq!(first_fetched, ["Models/Machines/Touched"]);
    assert!(first_peeked.is_empty(), "{first_peeked:?}");
    // (a plain run's first: the mount's list, which the loopback run wrote)
    let (plain, plain_fetched, plain_peeked) = run_once(false);
    assert!(has(&plain, "- print: plain Model"), "{plain:?}");
    assert!(plain_fetched.iter().any(|key| key == "Models/Machines/Untouched"), "{plain_fetched:?}");
    assert_eq!(plain_peeked, ["Models/Machines/Touched"]);
    // (the loopback scene's own list, which the plain run did not overwrite)
    let (again, _, again_peeked) = run_once(true);
    assert!(has(&again, "client print: client Model"), "{again:?}");
    assert_eq!(again_peeked, ["Models/Machines/Touched"], "the loopback run decoded ahead another scene's bundles");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A destroyed placeholder a script still holds is its bundle's root, as the game's destroyed instance is: its class, values,
/// attributes and tags, none of its children, no event; in a plain run, on a loopback client that destroyed its own copy, and
/// on one whose server destroyed the instance (and freed it) before the client looked
#[cfg(feature = "bundles")]
#[test]
fn a_destroyed_placeholder_a_script_holds_is_its_bundles_root() {
    let (dir, root, files) = bundle_project("bundles-destroyed-held");
    let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host(files, &fetched);
    let held = r##"
		local Machines = game:GetService("ReplicatedStorage").Library.Models.Machines
		local Held = Machines:GetChildren()[3]
		Machines:ClearAllChildren()
		print("held", Held.ClassName, Held:GetAttribute("Reach"), #Held:GetChildren(), Held:HasTag("Machine"), Held.Parent == nil)
"##;
    let (plain, emulator) = run_on(&dir, &root, &host, &format!("return {{ Run = function(Context) {held} print('@done') end }}"), false, &[]);
    drop(emulator);
    assert!(has(&plain, "- print: held Model 5 0 true true"), "{plain:?}");
    let (pair, emulator) = run_on(
        &dir,
        &root,
        &host,
        &format!(
            r##"
local Storage = game:GetService("ReplicatedStorage")
return {{
	Server = function(Context)
		task.wait(0.1)
		Storage.Library.Models.Units:Destroy()
	end,
	Client = function(Context)
		local Nested = Storage.Library.Models.Units:GetChildren()[1]
		{held}
		task.wait(1.5)
		print("nested", Nested.ClassName, #Nested:GetChildren(), Nested.Parent == nil)
		print("@done")
	end,
}}
"##
        ),
        true,
        &["--seconds", "3"],
    );
    drop(emulator);
    assert!(has(&pair, "client print: held Model 5 0 true true"), "{pair:?}");
    assert!(has(&pair, "client print: nested Model 0 true"), "{pair:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bundle tags are kept apart by the source's shape: a members file holds the root's children alone, without the root's tags,
/// so a run on a members source, then one on an uploaded source of the same asset versions, each finds its own
#[cfg(feature = "bundles")]
#[test]
fn the_tags_of_an_asset_version_are_kept_apart_by_the_sources_shape() {
    use crate::project::model_assets::BundleShape;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let (dir, root, files) = bundle_project("bundles-shapes");
    pin_versions(&root, fresh_asset_ids(), &["Models/Machines/ServerFirst", "Models/Machines/Touched", "Models/Machines/Untouched"]);
    // (a members file of the machine: its body and lamp, the model's tag not among them)
    let members = dir.join("members.rbxm");
    {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        dom.insert(dom.root_ref(), InstanceBuilder::new("Part").with_name("Body"));
        dom.insert(dom.root_ref(), InstanceBuilder::new("Part").with_name("Lamp"));
        let mut bytes = Vec::new();
        rbx_binary::to_writer(&mut bytes, &dom, dom.root().children()).unwrap();
        std::fs::write(&members, bytes).unwrap();
    }
    let member_files: Vec<(String, PathBuf)> = files.iter().map(|(key, file)| (key.clone(), if key.starts_with("Models/Machines/") && file.ends_with("machine.rbxm") { members.clone() } else { file.clone() })).collect();
    let none = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let text = "return function(Context) print('tagged', #game:GetService('CollectionService'):GetTagged('Machine')) print('@done') end";
    let (first, emulator) = run_on(&dir, &root, &bundle_host_as(member_files, &none, &none, BundleShape::Members, true, &Default::default()), text, false, &[]);
    drop(emulator);
    assert!(has(&first, "- print: tagged 0"), "{first:?}");
    let (second, emulator) = run_on(&dir, &root, &bundle_host(files, &none), text, false, &[]);
    drop(emulator);
    assert!(has(&second, "- print: tagged 3"), "{second:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two imports at one instance while its client copy is on its way (a placeholder that is also where a loose bundle's members
/// go): the copy takes the root as the first import left it, so the server's write between the two reaches it as a change
/// (its AttributeChanged), with both imports' instances
#[cfg(feature = "bundles")]
#[test]
fn two_imports_at_a_copy_on_its_way_keep_the_first_imports_root() {
    let (dir, root, mut files) = bundle_project("bundles-two-imports");
    let machine = files.iter().find(|(key, _)| key == "Models/Machines/Touched").map(|(_, file)| file.clone()).unwrap();
    files.push(("Models/Items".to_string(), machine));
    let none = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (lines, emulator) = run_on(
        &dir,
        &root,
        &bundle_host(files, &none),
        r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
return {
	Server = function(Context)
		task.wait(0.1)
		game:GetService("ServerStorage").Library.Models.Parent = Storage
		task.wait()
		local Items = Storage.Models.Items
		local _ = Items.ClassName
		Items:SetAttribute("Owner", 1)
		print("server", #Items:GetChildren())
	end,
	Client = function(Context)
		local Items = Storage:WaitForChild("Models"):WaitForChild("Items")
		local Changed = 0
		Items:GetAttributeChangedSignal("Owner"):Connect(function()
			Changed += 1
		end)
		print("client arrived", Items.ClassName, Items:GetAttribute("Owner"), #Items:GetChildren())
		for _ = 1, 4 do
			RunService.Heartbeat:Wait()
		end
		print("client after", Items:GetAttribute("Owner"), Changed)
		print("@done")
	end,
}
"##,
        true,
        &["--latency", "2", "--assets-at", "ServerStorage.Library"],
    );
    drop(emulator);
    assert!(has(&lines, "server print: server 4"), "{lines:?}");
    assert!(has(&lines, "client print: client arrived Model nil 4"), "{lines:?}");
    assert!(has(&lines, "client print: client after 1 1"), "{lines:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import kept for a client copy on its way stays when the server destroys and frees the instance before the copy's tree
/// arrives (forty frames of latency): the copy arrives as the bundle (its Destroy follows)
#[cfg(feature = "bundles")]
#[test]
fn an_import_kept_for_a_copy_on_its_way_outlives_the_servers_instance() {
    let (dir, root, files) = bundle_project("bundles-kept-freed");
    let none = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (lines, emulator) = run_on(
        &dir,
        &root,
        &bundle_host(files, &none),
        r##"
local Storage = game:GetService("ReplicatedStorage")
return {
	Server = function(Context)
		task.wait(0.1)
		game:GetService("ServerStorage").Library.Models.Parent = Storage
		task.wait()
		-- (the machines imported as they enter the Workspace, with no script holding one: destroyed, the collector frees them)
		local Models = Storage.Models
		Models.Parent = workspace
		Models.Machines:ClearAllChildren()
		print("server gave them up")
	end,
	Client = function(Context)
		local Machine = Storage:WaitForChild("Models"):WaitForChild("Machines"):WaitForChild("ServerFirst")
		print("client arrived", Machine.ClassName, #Machine:GetChildren())
		print("@done")
	end,
}
"##,
        true,
        &["--latency", "40", "--seconds", "3", "--assets-at", "ServerStorage.Library"],
    );
    drop(emulator);
    assert!(has(&lines, "client print: client arrived Model 2"), "{lines:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A tag read that fails on a source that cannot name a bundle's file before it holds it (host sources that do not implement
/// `expected`) leaves the run unchecked: a prepared run could not tell when the file came, so none is made from it
#[cfg(feature = "bundles")]
#[test]
fn a_failed_tag_read_on_a_source_that_cannot_name_its_files_leaves_the_run_unchecked() {
    let (dir, root, mut files) = bundle_project("bundles-unnamed");
    files.push(("Models/Machines/Offline".to_string(), dir.join("offline.rbxm")));
    let none = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host_as(files, &none, &none, crate::project::model_assets::BundleShape::Uploaded, false, &Default::default());
    crate::files::start_noting();
    let (lines, emulator) = run_on(&dir, &root, &host, "return function(Context) print('tagged', #game:GetService('CollectionService'):GetTagged('Machine')) print('@done') end", false, &[]);
    let noted = crate::files::stop_noting();
    drop(emulator);
    assert!(has(&lines, "- print: tagged 3"), "{lines:?}");
    assert_eq!(noted.changed().as_deref(), Some("a bundle's tags could not be read"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two manifest lines that pin one asset version name one file, which a tag lookup fetches once
#[cfg(feature = "bundles")]
#[test]
fn a_tag_lookup_fetches_a_file_two_keys_share_once() {
    let (dir, root, mut files) = bundle_project("bundles-shared-file");
    let machine = files.iter().find(|(key, _)| key == "Models/Machines/Touched").map(|(_, file)| file.clone()).unwrap();
    let shared = dir.join("shared.rbxm");
    std::fs::copy(&machine, dir.join("shared.rbxm.download")).unwrap();
    files.push(("Models/Spares/One".to_string(), shared.clone()));
    files.push(("Models/Spares/Two".to_string(), shared));
    let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let none = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let downloads = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let host = bundle_host_as(files, &fetched, &none, crate::project::model_assets::BundleShape::Uploaded, true, &downloads);
    let (lines, emulator) = run_on(&dir, &root, &host, "return function(Context) print('tagged', #game:GetService('CollectionService'):GetTagged('Machine')) print('@done') end", false, &[]);
    drop(emulator);
    assert!(has(&lines, "- print: tagged 5"), "{lines:?}");
    assert_eq!(downloads.borrow().len(), 1, "{:?}", downloads.borrow());
    // (fetched for the tags once, through the first key; each imported after)
    let count = |key: &str| fetched.borrow().iter().filter(|held| *held == key).count();
    assert_eq!((count("Models/Spares/One"), count("Models/Spares/Two")), (2, 1), "{:?}", fetched.borrow());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A client that connects GetInstanceAddedSignal for a tag gets it for the tagged instances of the waiting bundles a tree brings
/// into its DataModel, as a client of the game does: the bundles carrying the tag import before the tree enters
#[cfg(feature = "bundles")]
#[test]
fn a_tree_bringing_waiting_bundles_fires_the_added_signal_of_their_tags() {
    let (dir, root, files) = bundle_project("bundles-added-signal");
    let none = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (lines, emulator) = run_on(
        &dir,
        &root,
        &bundle_host(files, &none),
        r##"
local Storage = game:GetService("ReplicatedStorage")
local CollectionService = game:GetService("CollectionService")
return {
	Server = function(Context)
		task.wait(0.1)
		game:GetService("ServerStorage").Library.Models.Parent = Storage
	end,
	Client = function(Context)
		local Added = {}
		CollectionService:GetInstanceAddedSignal("Machine"):Connect(function(Machine)
			table.insert(Added, Machine.Name)
		end)
		Storage:WaitForChild("Models")
		task.wait()
		table.sort(Added)
		print("client added", table.concat(Added, " "))
		print("@done")
	end,
}
"##,
        true,
        &["--assets-at", "ServerStorage.Library"],
    );
    drop(emulator);
    assert!(has(&lines, "client print: client added ServerFirst Touched Untouched"), "{lines:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A script that connects GetInstanceRemovedSignal for a tag gets it for the tagged instances of the waiting bundles in a subtree
/// that leaves the DataModel, whichever way it leaves (`Parent = nil`, Remove, Destroy, ClearAllChildren of its parent), as the
/// game's does: the bundles carrying the tag import before it leaves, and the others stay waiting
#[cfg(feature = "bundles")]
#[test]
fn a_subtree_leaving_with_waiting_bundles_fires_the_removed_signal_of_their_tags() {
    let mut wrong = Vec::new();
    for (at, leave) in ["Machines.Parent = nil", "Machines:Remove()", "Machines:Destroy()", "Models:ClearAllChildren()"].into_iter().enumerate() {
        let (dir, root, files) = bundle_project(&format!("bundles-removed-signal-{at}"));
        let fetched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let (lines, emulator) = run_on(
            &dir,
            &root,
            &bundle_host(files, &fetched),
            &format!(
                r##"
local CollectionService = game:GetService("CollectionService")
return function(Context)
	local Removed = {{}}
	CollectionService:GetInstanceRemovedSignal("Machine"):Connect(function(Machine)
		table.insert(Removed, Machine.Name)
	end)
	local Models = game:GetService("ReplicatedStorage").Library.Models
	local Machines = Models.Machines
	{leave}
	task.wait()
	table.sort(Removed)
	print("removed", table.concat(Removed, " "))
	print("@done")
end
"##
            ),
            false,
            &[],
        );
        drop(emulator);
        if !has(&lines, "- print: removed ServerFirst Touched Untouched") {
            wrong.push(format!("{leave}: {lines:?}"));
        }
        // (the bundles carrying no such tag were not imported)
        if fetched.borrow().iter().any(|key| key.starts_with("Models/Units") || key.starts_with("Models/Items")) {
            wrong.push(format!("{leave} imported others: {:?}", fetched.borrow()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A loopback client that connects GetInstanceRemovedSignal for a tag gets it for the tagged instances of the waiting bundles
/// below a server instance that leaves (the server destroys it, or parents it to nil), as a client of the game does, though
/// nothing on the server listens: the bundles carrying the tag import at the client's copy before it leaves (from the server's
/// source when the server's instance is destroyed, else by the server, whose import the copy takes)
#[cfg(feature = "bundles")]
#[test]
fn a_loopback_client_gets_the_removed_signal_of_waiting_bundles_its_server_takes_out() {
    let mut wrong = Vec::new();
    for (at, leave) in ["Machines:Destroy()", "Machines.Parent = nil"].into_iter().enumerate() {
        let (dir, root, files) = bundle_project(&format!("bundles-client-removed-{at}"));
        let none = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let (lines, emulator) = run_on(
            &dir,
            &root,
            &bundle_host(files, &none),
            &format!(
                r##"
local Library = game:GetService("ReplicatedStorage"):WaitForChild("Library")
local CollectionService = game:GetService("CollectionService")
return {{
	Server = function(Context)
		task.wait(0.1)
		local Machines = Library.Models.Machines
		{leave}
	end,
	Client = function(Context)
		local Removed = {{}}
		CollectionService:GetInstanceRemovedSignal("Machine"):Connect(function(Machine)
			table.insert(Removed, Machine.Name)
		end)
		local Machines = Library:WaitForChild("Models"):WaitForChild("Machines")
		Machines.AncestryChanged:Wait()
		task.wait()
		table.sort(Removed)
		print("client removed", table.concat(Removed, " "))
		print("@done")
	end,
}}
"##
            ),
            true,
            &[],
        );
        drop(emulator);
        if !has(&lines, "client print: client removed ServerFirst Touched Untouched") {
            wrong.push(format!("{leave}: {lines:?}"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A loopback client leaving (Context.Leave) takes its player's character with it on the server, as a leave does
/// (services/players.rs): CharacterRemoving there, the character out of the Workspace and Character nil before PlayerRemoving,
/// the character destroyed a second on; the server's scripts pay nothing for it (it is the engine's own step). The
/// character used to stay in the server's Workspace with Character still set.
#[test]
fn a_loopback_client_leaving_takes_its_character_with_it() {
    let summary = run_pair(
        "leave-character",
        r##"
local Players = game:GetService("Players")
return {
	Server = function(Context)
		local Player = Context.Player
		local Body = Instance.new("Model")
		Body.Name = "Body"
		local Part = Instance.new("Part")
		Part.Anchored = true
		Part.Parent = Body
		Body.Parent = workspace
		task.wait(0.2)
		Player.Character = Body
		local Order = {}
		Player.CharacterRemoving:Connect(function(Model) table.insert(Order, "CharacterRemoving " .. Model.Name) end)
		Body.AncestryChanged:Connect(function(_, Parent) table.insert(Order, "Body.AncestryChanged " .. tostring(Parent)) end)
		Player:GetPropertyChangedSignal("Character"):Connect(function() table.insert(Order, "Character " .. tostring(Player.Character)) end)
		Players.PlayerRemoving:Connect(function() table.insert(Order, "PlayerRemoving") end)
		local Gone
		Body.Destroying:Connect(function() Gone = time() end)
		task.wait(0.2)
		Context.Leave()
		task.wait()
		local Left = time()
		task.wait()
		print("order", table.concat(Order, ", "))
		print("after", Player.Parent, Player.Character, Body.Parent)
		task.wait(1.2)
		print("destroyed", Gone ~= nil and string.format("%.2f", Gone - Left))
		print("@done")
	end,
	Client = function(Context)
		task.wait(5)
	end,
}
"##,
        None,
        &[],
    );
    let lines = lines(&summary);
    for wanted in [
        "server print: order CharacterRemoving Body, Body.AncestryChanged nil, Character nil, PlayerRemoving",
        "server print: after nil nil nil",
    ] {
        assert!(has(&lines, wanted), "{wanted}: {lines:?}");
    }
    assert!(lines.iter().any(|line| line.starts_with("server print: destroyed 0.9") || line.starts_with("server print: destroyed 1.0")), "{lines:?}");
}

/// What the other side sent runs at the receiving side's frame start, before PreAnimation, and reads that side's last frame's
/// time() (the time() of its Heartbeat before), equal to its DistributedGameTime: a RemoteEvent's handlers, a RemoteFunction's
/// callback and the call's return, on both sides (Studio 0.747, tools/parity/probes/loopback/remote_time.luau); ree read the
/// frame's time() there
#[test]
fn remote_handlers_read_the_last_frames_time() {
    let harness = r##"
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local function Watch()
	local Beat
	RunService.Heartbeat:Connect(function()
		Beat = time()
	end)
	return function(Kind)
		print(Kind, time() == Beat, workspace.DistributedGameTime == time())
	end
end
return {
	Server = function(Context)
		local Note = Watch()
		local Call = Instance.new("RemoteFunction")
		Call.Name = "Call"
		Call.OnServerInvoke = function()
			Note("OnServerInvoke")
			return true
		end
		Call.Parent = Storage
		local Echo = Instance.new("RemoteEvent")
		Echo.Name = "Echo"
		Echo.OnServerEvent:Connect(function(Player)
			Note("OnServerEvent")
			Echo:FireClient(Player)
			Call:InvokeClient(Player)
			Note("InvokeClient returned")
		end)
		Echo.Parent = Storage
	end,
	Client = function(Context)
		local Note = Watch()
		local Echo = Storage:WaitForChild("Echo")
		local Call = Storage:WaitForChild("Call")
		Call.OnClientInvoke = function()
			Note("OnClientInvoke")
			return true
		end
		Echo.OnClientEvent:Connect(function()
			Note("OnClientEvent")
		end)
		RunService.Heartbeat:Wait()
		Echo:FireServer()
		Call:InvokeServer()
		Note("InvokeServer returned")
		task.wait(0.2)
		print("@done")
	end,
}
"##;
    let lines = lines(&run_pair("remote-time", harness, None, &[]));
    for kind in ["server print: OnServerEvent", "server print: OnServerInvoke", "server print: InvokeClient returned", "client print: OnClientEvent", "client print: OnClientInvoke", "client print: InvokeServer returned"] {
        assert!(has(&lines, &format!("{kind} true true")), "{kind}: {lines:?}");
    }
}
