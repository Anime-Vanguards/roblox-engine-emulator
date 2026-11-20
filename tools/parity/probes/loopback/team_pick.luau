-- parity: loopback
-- Which team a joining player gets among several: a team that is not auto-assignable first in Teams (skipped), then four
-- auto-assignable empty teams whose child order, names and colour numbers each put a different one first; an
-- auto-assignable team outside Teams; and the player's TeamColor and Neutral, on each side.
local Players = game:GetService("Players")
local function State(Player)
	return `team={if Player.Team then Player.Team.Name else "nil"} color={Player.TeamColor.Name} neutral={Player.Neutral}`
end
--- server
local Teams = game:GetService("Teams")
local function Make(Name, Color, Auto, Parent)
	local Team = Instance.new("Team")
	Team.Name = Name
	Team.TeamColor = BrickColor.new(Color)
	Team.AutoAssignable = Auto
	Team.Parent = Parent
	return Team
end
Make("Outside", "Really black", true, workspace)
Make("Fixed", "Bright orange", false, Teams)
Make("Zeta", "Bright blue", true, Teams)
Make("Alpha", "Bright yellow", true, Teams)
Make("Mid", "Bright red", true, Teams)
Make("Kappa", "Lime green", true, Teams)
local Player = P.player()
for _ = 1, 3 do
	task.wait()
end
print("joined " .. State(Player))
local Parts = {}
for _, Team in Teams:GetTeams() do
	table.insert(Parts, Team.Name .. "=" .. #Team:GetPlayers())
end
print("members " .. table.concat(Parts, " ") .. " | outside " .. P.try(function()
	return #workspace.Outside:GetPlayers()
end))
P.meet("joined")
--- client
local Player = Players.LocalPlayer
print("first " .. State(Player))
P.meet("joined")
print("joined " .. State(Player))
