-- parity: loopback
-- How a player's Team, TeamColor and Neutral follow one another when the server sets one of them (a team, no team, a team's
-- colour, no team's colour, Neutral either way), what a team's colour change, a second team of the same colour, moving a
-- team out of Teams and destroying it do to the player on it, which signals fire for each (the player's property signals,
-- Team.PlayerAdded / PlayerRemoved), what Team:GetPlayers and Teams:GetTeams answer, and what the client sees after each
-- (the server waits for the client's line before its next step).
local Players = game:GetService("Players")
local function Name(Team)
	return if Team then Team.Name else "nil"
end
local function State(Player)
	return `team={Name(Player.Team)} color={Player.TeamColor.Name} neutral={Player.Neutral}`
end
local function Count(Team)
	if not Team then
		return "none"
	end
	return P.try(function()
		return #Team:GetPlayers()
	end)
end
local function Members(Teams)
	local Parts = {}
	for _, Team in Teams:GetTeams() do
		table.insert(Parts, Team.Name .. "=" .. Count(Team))
	end
	return table.concat(Parts, " ")
end
local Events = {}
local function Watch(Team)
	Team.PlayerAdded:Connect(function(Player)
		table.insert(Events, Team.Name .. ".PlayerAdded " .. State(Player))
	end)
	Team.PlayerRemoved:Connect(function(Player)
		table.insert(Events, Team.Name .. ".PlayerRemoved " .. State(Player))
	end)
end
local function WatchPlayer(Player)
	for _, Property in { "Team", "TeamColor", "Neutral" } do
		Player:GetPropertyChangedSignal(Property):Connect(function()
			table.insert(Events, "changed " .. Property .. " " .. State(Player))
		end)
	end
end
local function Flush(Label)
	print(Label .. " events: " .. table.concat(Events, " / "))
	table.clear(Events)
end
local Steps = {
	"joined", "team blue", "team nil", "neutral false", "color red", "color black", "neutral true", "team blue neutral true",
	"blue recolored", "twin made", "color twin", "team twin", "blue moved out", "team outside", "blue moved back",
	"blue destroyed", "team destroyed",
}
--- server
local Teams = game:GetService("Teams")
local function Make(TeamName, Color, Auto)
	local Team = Instance.new("Team")
	Team.Name = TeamName
	Team.TeamColor = BrickColor.new(Color)
	Team.AutoAssignable = Auto
	Watch(Team)
	Team.Parent = Teams
	return Team
end
local Red = Make("Red", "Bright red", true)
local Blue = Make("Blue", "Bright blue", false)
local Twin
local Player = P.player()
WatchPlayer(Player)
local Actions = {
	["joined"] = function() end,
	["team blue"] = function()
		Player.Team = Blue
	end,
	["team nil"] = function()
		Player.Team = nil
	end,
	["neutral false"] = function()
		Player.Neutral = false
	end,
	["color red"] = function()
		Player.TeamColor = BrickColor.new("Bright red")
	end,
	["color black"] = function()
		Player.TeamColor = BrickColor.new("Really black")
	end,
	["neutral true"] = function()
		Player.Neutral = true
	end,
	["team blue neutral true"] = function()
		Player.Team = Blue
		print("  set team " .. State(Player))
		Player.Neutral = true
	end,
	["blue recolored"] = function()
		Player.Team = Blue
		Blue.TeamColor = BrickColor.new("Lime green")
	end,
	["twin made"] = function()
		Twin = Make("Twin", "Bright red", false)
		Player.Team = Red
	end,
	["color twin"] = function()
		Player.Team = Blue
		Player.TeamColor = BrickColor.new("Bright red")
	end,
	["team twin"] = function()
		Player.Team = Twin
	end,
	["blue moved out"] = function()
		Player.Team = Blue
		Blue.Parent = workspace
	end,
	["team outside"] = function()
		Player.Team = Blue
	end,
	["blue moved back"] = function()
		Blue.Parent = Teams
	end,
	["blue destroyed"] = function()
		Player.Team = Blue
		Blue:Destroy()
	end,
	["team destroyed"] = function()
		Player.Team = Blue
	end,
}
for _ = 1, 3 do
	task.wait()
end
for _, Step in Steps do
	local Ran = P.try(Actions[Step])
	print(Step .. " now " .. State(Player) .. (if string.sub(Ran, 1, 2) == "ok" then "" else " | " .. Ran))
	for _ = 1, 3 do
		task.wait()
	end
	print(Step .. " then " .. State(Player) .. " | " .. Members(Teams) .. " | blue players " .. Count(Blue))
	Flush(Step)
	P.meet(Step)
	P.meet(Step .. " seen")
end
local Folder = Instance.new("Folder")
Folder.Parent = Teams
local Loose = Instance.new("Team")
Loose.Name = "Loose"
Loose.Parent = workspace
print("get teams " .. Members(Teams) .. " | children " .. #Teams:GetChildren())
P.meet("end")
--- client
local Player = Players.LocalPlayer
local Teams = game:GetService("Teams")
WatchPlayer(Player)
local Seen = {}
local function WatchTeams()
	for _, Team in Teams:GetTeams() do
		if not Seen[Team] then
			Seen[Team] = true
			Watch(Team)
		end
	end
end
WatchTeams()
Teams.ChildAdded:Connect(function()
	WatchTeams()
end)
for _, Step in Steps do
	P.meet(Step)
	WatchTeams()
	local Blue = Teams:FindFirstChild("Blue") or workspace:FindFirstChild("Blue")
	print(Step .. " " .. State(Player) .. " | " .. Members(Teams) .. " | blue players " .. Count(Blue))
	Flush(Step)
	P.meet(Step .. " seen")
end
P.meet("end")
