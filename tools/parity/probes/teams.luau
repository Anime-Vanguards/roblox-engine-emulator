-- A client's own player and teams the client makes itself: the player's Team, TeamColor and Neutral at the start, how they
-- follow one another when the client sets one, which signals fire, Team:GetPlayers and Teams:GetTeams on the client's
-- teams, and what a wrong value raises. (A server has no local player: the probe says so and stops.)
local Players = game:GetService("Players")
local Player = Players.LocalPlayer
if not Player then
	print("no local player")
	return
end
local function Name(Team)
	return if Team then Team.Name else "nil"
end
local function State()
	return `team={Name(Player.Team)} color={Player.TeamColor.Name} neutral={Player.Neutral}`
end
print("start " .. State())
local Teams = game:GetService("Teams")
local Events = {}
local function Watch(Team)
	Team.PlayerAdded:Connect(function(Who)
		table.insert(Events, Team.Name .. ".PlayerAdded " .. tostring(Who == Player))
	end)
	Team.PlayerRemoved:Connect(function(Who)
		table.insert(Events, Team.Name .. ".PlayerRemoved " .. tostring(Who == Player))
	end)
end
for _, Property in { "Team", "TeamColor", "Neutral" } do
	Player:GetPropertyChangedSignal(Property):Connect(function()
		table.insert(Events, "changed " .. Property .. " " .. State())
	end)
end
local function Make(TeamName, Color)
	local Team = Instance.new("Team")
	Team.Name = TeamName
	Team.TeamColor = BrickColor.new(Color)
	Watch(Team)
	Team.Parent = Teams
	return Team
end
local Red = Make("Red", "Bright red")
local Blue = Make("Blue", "Bright blue")
local function Step(Label, Action)
	Action()
	local Now = State()
	task.wait()
	task.wait()
	local Parts = {}
	for _, Team in Teams:GetTeams() do
		table.insert(Parts, Team.Name .. "=" .. #Team:GetPlayers())
	end
	print(Label .. " now " .. Now .. " then " .. State() .. " | " .. table.concat(Parts, " ") .. " | " .. table.concat(Events, " / "))
	table.clear(Events)
end
Step("made", function() end)
Step("team red", function()
	Player.Team = Red
end)
Step("team blue", function()
	Player.Team = Blue
end)
Step("neutral true", function()
	Player.Neutral = true
end)
Step("color blue", function()
	Player.TeamColor = BrickColor.new("Bright blue")
end)
Step("team nil", function()
	Player.Team = nil
end)
Step("neutral false", function()
	Player.Neutral = false
end)
Step("color white", function()
	Player.TeamColor = BrickColor.new("Institutional white")
end)
Step("red recolored", function()
	Player.Team = Red
	Red.TeamColor = BrickColor.new("Lime green")
end)
Step("red destroyed", function()
	Red:Destroy()
end)
print("team part " .. P.try(function()
	Player.Team = workspace
end))
print("team string " .. P.try(function()
	Player.Team = "Blue"
end))
print("color string " .. P.try(function()
	Player.TeamColor = "Bright red"
end))
print("color number " .. P.try(function()
	Player.TeamColor = 21
end))
print("neutral string " .. P.try(function()
	Player.Neutral = "yes"
end))
print("after errors " .. State())
print("get players arg " .. P.try(function()
	return #Blue:GetPlayers(1)
end))
print("teams methods " .. P.try(function()
	return #Teams:GetTeams()
end))
