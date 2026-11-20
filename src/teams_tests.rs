//! Teams (services/teams.rs) through `ree run`: the team a server gives each joining player, how a player's Team, TeamColor
//! and Neutral follow one another, what a loopback client sees of its own player, and a client-only run's local player.

use std::path::PathBuf;

use crate::{
    launch::Host,
    record::run::{self, RecordArgs},
};

/// A scratch folder of its own for a test
fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ree-teams-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `ree run` of `harness` with `extra` arguments, in an empty project: the output as `side kind: message` lines (side `-`
/// outside a loopback run), none of which may be an error
fn run_ree(name: &str, harness: &str, extra: &[&str]) -> Vec<String> {
    let dir = scratch(name);
    let file = dir.join(format!("{name}.luau"));
    std::fs::write(&file, harness).unwrap();
    let empty = dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let mut words: Vec<String> = ["--harness", &file.to_string_lossy(), "--roots", &empty.to_string_lossy(), "--seconds", "2"].iter().map(|s| s.to_string()).collect();
    words.extend(extra.iter().map(|s| s.replace("{dir}", &dir.to_string_lossy())));
    for (file, text) in models(extra) {
        std::fs::create_dir_all(dir.join(&file).parent().unwrap()).unwrap();
        std::fs::write(dir.join(file), text).unwrap();
    }
    let args: RecordArgs = run::parse(&words, false).unwrap();
    let summary = run::run_only(&args, &Host::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let lines = crate::loopback_tests::lines(&summary);
    let errors: Vec<&String> = lines.iter().filter(|line| line.contains(" error: ")).collect();
    assert!(errors.is_empty(), "{errors:?}\n{lines:?}");
    lines
}

/// The team models a run mounting `{dir}/teams` into Teams reads: a team that is not auto-assignable first, then one that is
fn models(extra: &[&str]) -> Vec<(String, &'static str)> {
    if !extra.iter().any(|word| word.contains("{dir}/teams")) {
        return Vec::new();
    }
    vec![
        ("teams/Fixed.model.json".into(), r#"{ "className": "Team", "properties": { "TeamColor": { "BrickColor": 106 }, "AutoAssignable": false } }"#),
        ("teams/Red.model.json".into(), r#"{ "className": "Team", "properties": { "TeamColor": { "BrickColor": 21 }, "AutoAssignable": true } }"#),
    ]
}

fn printed(lines: &[String]) -> Vec<String> {
    lines.iter().filter_map(|line| line.split_once(" print: ").map(|(_, message)| message.to_string())).collect()
}

const STATE: &str = r#"
local function State(Player)
	return `team={if Player.Team then Player.Team.Name else "nil"} color={Player.TeamColor.Name} neutral={Player.Neutral}`
end
local function Make(Name, Color, Auto, Parent)
	local Team = Instance.new("Team")
	Team.Name = Name
	Team.TeamColor = BrickColor.new(Color)
	Team.AutoAssignable = Auto
	Team.Parent = Parent or game:GetService("Teams")
	return Team
end
"#;

/// A server gives each joining player the auto-assignable team in Teams with the fewest players, the first in Teams' child
/// order among the fewest (an earlier team that is not auto-assignable, and one outside Teams, never), counting a player
/// on every team of its colour, before the team's PlayerAdded and then Players.PlayerAdded fire; the players in already
/// keep theirs (measured with four clients joining one server: services/teams.rs)
#[test]
fn a_server_gives_each_joining_player_the_auto_assignable_team_with_the_fewest_players() {
    let harness = format!("return function(Context)\n{STATE}\n{}\nend\n", r#"
	local Players = game:GetService("Players")
	local Teams = game:GetService("Teams")
	Make("Outside", "Really black", true, workspace)
	Make("Fixed", "Bright orange", false)
	local Zeta = Make("Zeta", "Bright blue", true)
	local Alpha = Make("Alpha", "Bright yellow", true)
	Make("Mid", "Bright red", true)
	Make("Kappa", "Lime green", true)
	local Events = {}
	for _, Team in Teams:GetTeams() do
		Team.PlayerAdded:Connect(function(Player)
			table.insert(Events, Team.Name .. ".PlayerAdded " .. Player.Name)
		end)
	end
	Players.PlayerAdded:Connect(function(Player)
		table.insert(Events, "PlayerAdded " .. Player.Name .. " " .. State(Player))
	end)
	local Joined = {}
	local function Join()
		local Player = Instance.new("Player")
		Player.Name = "P" .. (#Joined + 1)
		print("before " .. State(Player))
		Player.Parent = Players
		print("joining " .. State(Player))
		table.insert(Joined, Player)
		task.wait()
		return Player
	end
	for _ = 1, 5 do
		Join()
	end
	print("events " .. table.concat(Events, " / "))
	-- P1 moves to Alpha; Zeta leaves Teams and comes back last (P5 loses it and gets it back)
	Joined[1].Team = Alpha
	Zeta.Parent = nil
	print("zeta out " .. State(Joined[5]))
	Zeta.Parent = Teams
	print("zeta back " .. State(Joined[5]))
	Join()
	-- a team of Kappa's colour: Kappa's player counts on both
	Make("Twin", "Lime green", true)
	Join()
	Join()
	local Parts = {}
	for _, Player in Joined do
		table.insert(Parts, Player.Name .. "=" .. (if Player.Team then Player.Team.Name else "nil"))
	end
	print("teams " .. table.concat(Parts, " "))
	local Counts = {}
	for _, Team in Teams:GetTeams() do
		table.insert(Counts, Team.Name .. "=" .. #Team:GetPlayers())
	end
	print("counts " .. table.concat(Counts, " "))
"#);
    let lines = printed(&run_ree("spread", &harness, &["--mode", "play", "--run-context", "server"]));
    for wanted in [
        "before team=nil color=White neutral=true",
        "joining team=Zeta color=Bright blue neutral=false",
        "events Zeta.PlayerAdded P1 / PlayerAdded P1 team=Zeta color=Bright blue neutral=false / Alpha.PlayerAdded P2 / PlayerAdded P2 team=Alpha color=Bright yellow neutral=false / Mid.PlayerAdded P3 / PlayerAdded P3 team=Mid color=Bright red neutral=false / Kappa.PlayerAdded P4 / PlayerAdded P4 team=Kappa color=Lime green neutral=false / Zeta.PlayerAdded P5 / PlayerAdded P5 team=Zeta color=Bright blue neutral=false",
        "zeta out team=nil color=Bright blue neutral=false",
        "zeta back team=Zeta color=Bright blue neutral=false",
        "teams P1=Alpha P2=Alpha P3=Mid P4=Kappa P5=Zeta P6=Mid P7=Kappa P8=Zeta",
        "counts Fixed=0 Alpha=2 Mid=2 Kappa=2 Zeta=2 Twin=2",
    ] {
        assert!(lines.iter().any(|line| line == wanted), "missing {wanted:?} in {lines:#?}");
    }
}

/// With no auto-assignable team in Teams a joining player stays neutral (Neutral true, TeamColor White), and a team made
/// auto-assignable later leaves the players in already as they are: only the next one to join gets it
#[test]
fn a_joining_player_stays_neutral_without_an_auto_assignable_team() {
    let harness = format!("return function(Context)\n{STATE}\n{}\nend\n", r#"
	local Players = game:GetService("Players")
	local First = Instance.new("Player")
	First.Parent = Players
	task.wait()
	print("no teams " .. State(First))
	Make("Fixed", "Bright red", false)
	local Second = Instance.new("Player")
	Second.Parent = Players
	task.wait()
	print("fixed only " .. State(Second))
	Make("Late", "Lime green", true)
	task.wait()
	print("late first " .. State(First))
	local Third = Instance.new("Player")
	Third.Parent = Players
	task.wait()
	print("late third " .. State(Third))
"#);
    let lines = printed(&run_ree("neutral", &harness, &["--mode", "play", "--run-context", "server"]));
    for wanted in [
        "no teams team=nil color=White neutral=true",
        "fixed only team=nil color=White neutral=true",
        "late first team=nil color=White neutral=true",
        "late third team=Late color=Lime green neutral=false",
    ] {
        assert!(lines.iter().any(|line| line == wanted), "missing {wanted:?} in {lines:#?}");
    }
}

/// Team follows TeamColor and Neutral: writing Team writes TeamColor, then Neutral false, and gives the first team in Teams
/// of that colour; nil writes Neutral true; a colour no team has leaves Team nil and Neutral false; a team leaving Teams
/// or changing colour takes its players' Team. Each change fires the old team's PlayerRemoved, the new one's PlayerAdded,
/// then Team's signal, after the write's own; and the errors a script can see (measured: tools/parity/probes/teams.luau,
/// loopback/team_links.luau)
#[test]
fn a_players_team_follows_its_team_color_and_neutral() {
    let harness = format!("return function(Context)\n{STATE}\n{}\nend\n", r#"
	local Player = game:GetService("Players").LocalPlayer
	local Red = Make("Red", "Bright red", false)
	local Blue = Make("Blue", "Bright blue", false)
	local Twin = Make("Twin", "Bright red", false)
	local Events = {}
	for _, Team in { Red, Blue, Twin } do
		Team.PlayerAdded:Connect(function()
			table.insert(Events, Team.Name .. ".PlayerAdded")
		end)
		Team.PlayerRemoved:Connect(function()
			table.insert(Events, Team.Name .. ".PlayerRemoved")
		end)
	end
	for _, Name in { "Team", "TeamColor", "Neutral" } do
		Player:GetPropertyChangedSignal(Name):Connect(function()
			table.insert(Events, Name)
		end)
	end
	local function Step(Label, Action)
		local Ok, Problem = pcall(Action)
		local Now = State(Player)
		task.wait()
		print(Label .. " " .. Now .. (if Ok then "" else " | " .. tostring(Problem)) .. " | " .. table.concat(Events, " "))
		table.clear(Events)
	end
	Step("start", function() end)
	Step("team red", function() Player.Team = Red end)
	Step("team blue", function() Player.Team = Blue end)
	Step("team twin", function() Player.Team = Twin end)
	Step("team nil", function() Player.Team = nil end)
	Step("neutral false", function() Player.Neutral = false end)
	Step("color black", function() Player.TeamColor = BrickColor.new("Really black") end)
	Step("color blue", function() Player.TeamColor = BrickColor.new("Bright blue") end)
	Step("blue out", function() Blue.Parent = workspace end)
	Step("team outside", function() Player.Team = Blue end)
	Step("blue back", function() Blue.Parent = game:GetService("Teams") end)
	Step("blue recolored", function() Blue.TeamColor = BrickColor.new("Lime green") end)
	Step("team part", function() Player.Team = workspace end)
	Step("team string", function() Player.Team = "Red" end)
	print("outside players " .. tostring(select(2, pcall(function() return Instance.new("Team"):GetPlayers() end))))
	print("red players " .. #Red:GetPlayers() .. " twin players " .. #Twin:GetPlayers())
"#);
    let lines = printed(&run_ree("links", &harness, &["--mode", "play"]));
    for wanted in [
        "start team=nil color=White neutral=true | ",
        "team red team=Red color=Bright red neutral=false | TeamColor Neutral Red.PlayerAdded Team",
        "team blue team=Blue color=Bright blue neutral=false | TeamColor Red.PlayerRemoved Blue.PlayerAdded Team",
        "team twin team=Red color=Bright red neutral=false | TeamColor Blue.PlayerRemoved Red.PlayerAdded Team",
        "team nil team=nil color=Bright red neutral=true | Neutral Red.PlayerRemoved Team",
        "neutral false team=Red color=Bright red neutral=false | Neutral Red.PlayerAdded Team",
        "color black team=nil color=Really black neutral=false | TeamColor Red.PlayerRemoved Team",
        "color blue team=Blue color=Bright blue neutral=false | TeamColor Blue.PlayerAdded Team",
        "blue out team=nil color=Bright blue neutral=false | Blue.PlayerRemoved Team",
        "team outside team=nil color=Bright blue neutral=false | Setting Player.Team failed because team must be a child of the Teams service | ",
        "blue back team=Blue color=Bright blue neutral=false | Blue.PlayerAdded Team",
        "blue recolored team=nil color=Bright blue neutral=false | Blue.PlayerRemoved Team",
        "team part team=nil color=Bright blue neutral=false | Expected Team got Workspace for Player.Team. | ",
        "team string team=nil color=Bright blue neutral=false | Unable to assign property Team. Team expected, got string | ",
        "outside players Team:GetPlayers() failed because team must be a child of the Teams service",
        "red players 0 twin players 0",
    ] {
        assert!(lines.iter().any(|line| line == wanted), "missing {wanted:?} in {lines:#?}");
    }
}

/// A loopback client finds its player on its team at its first line, and works its Team out from the TeamColor and Neutral
/// the server sends and its own copies of the teams: a team the server moves out of Teams leaves the client's player with
/// Team nil and Neutral false, as on the server (loopback/team_links.luau)
#[test]
fn a_loopback_client_works_its_players_team_out_from_what_the_server_sends() {
    let harness = format!(
        "return {{\n\tServer = function(Context)\n{STATE}\n{}\nend,\n\tClient = function(Context)\n{STATE}\n{}\nend,\n}}\n",
        r#"
	local Red = Make("Red", "Bright red", true)
	local Blue = Make("Blue", "Bright blue", false)
	local Player = game:GetService("Players"):GetPlayers()[1] or game:GetService("Players").PlayerAdded:Wait()
	print("joined " .. State(Player))
	for _ = 1, 5 do task.wait() end
	Player.Team = Blue
	for _ = 1, 5 do task.wait() end
	Blue.Parent = workspace
	for _ = 1, 5 do task.wait() end
	print("server end " .. State(Player))
"#,
        r#"
	local Player = game:GetService("Players").LocalPlayer
	print("first " .. State(Player))
	local Events = {}
	for _, Name in { "Team", "TeamColor", "Neutral" } do
		Player:GetPropertyChangedSignal(Name):Connect(function()
			table.insert(Events, Name .. " " .. State(Player))
		end)
	end
	for _ = 1, 30 do task.wait() end
	print("client end " .. State(Player) .. " | " .. table.concat(Events, " / "))
"#
    );
    let lines = run_ree("loopback", &harness, &["--run-context", "loopback"]);
    for wanted in [
        "server print: joined team=Red color=Bright red neutral=false",
        "client print: first team=Red color=Bright red neutral=false",
        "server print: server end team=nil color=Bright blue neutral=false",
        "client print: client end team=nil color=Bright blue neutral=false | TeamColor team=Blue color=Bright blue neutral=false / Team team=Blue color=Bright blue neutral=false / Team team=nil color=Bright blue neutral=false",
    ] {
        assert!(lines.iter().any(|line| line == wanted), "missing {wanted:?} in {lines:#?}");
    }
}

/// A client-only run's local player has, at its first line and with no events, the team its server gives the first player
/// to join (the project's teams: one that is not auto-assignable, then one that is); in edit mode, where Studio has no
/// server, it stays neutral
#[test]
fn a_client_only_runs_local_player_has_the_team_its_server_would_give() {
    let harness = format!("return function(Context)\n{STATE}\n{}\nend\n", r#"
	local Player = game:GetService("Players").LocalPlayer
	print("first " .. State(Player))
	local Teams = game:FindFirstChildOfClass("Teams")
	print("teams " .. (if Teams then #Teams:GetTeams() else -1))
	local Fired = 0
	Player.Changed:Connect(function() Fired += 1 end)
	task.wait()
	task.wait()
	print("fired " .. Fired)
"#);
    let play = printed(&run_ree("client-play", &harness, &["--mode", "play", "--mount", "Teams={dir}/teams"]));
    assert_eq!(play, ["first team=Red color=Bright red neutral=false", "teams 2", "fired 0"], "{play:#?}");
    let edit = printed(&run_ree("client-edit", &harness, &["--mode", "edit", "--mount", "Teams={dir}/teams"]));
    assert_eq!(edit, ["first team=nil color=White neutral=true", "teams 2", "fired 0"], "{edit:#?}");
}
