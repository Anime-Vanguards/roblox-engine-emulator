-- parity: loopback
-- A joining player and one auto-assignable team: when the server gives the player its team (before Players.ChildAdded and
-- PlayerAdded, or after), which signals fire for it in which order, and what the client sees of its own player at its
-- first line and once the join is done.
local Players = game:GetService("Players")
local function State(Player)
	return `team={if Player.Team then Player.Team.Name else "nil"} color={Player.TeamColor.Name} neutral={Player.Neutral}`
end
--- server
local Teams = game:GetService("Teams")
local Red = Instance.new("Team")
Red.Name = "Red"
Red.TeamColor = BrickColor.new("Bright red")
print("red auto " .. tostring(Red.AutoAssignable) .. " color " .. Red.TeamColor.Name)
local Events = {}
Players.ChildAdded:Connect(function(Child)
	if Child:IsA("Player") then
		table.insert(Events, "ChildAdded " .. State(Child))
	end
end)
Players.PlayerAdded:Connect(function(Player)
	table.insert(Events, "PlayerAdded " .. State(Player) .. " red players " .. #Red:GetPlayers())
	for _, Name in { "Team", "TeamColor", "Neutral" } do
		Player:GetPropertyChangedSignal(Name):Connect(function()
			table.insert(Events, "changed " .. Name .. " " .. State(Player))
		end)
	end
end)
Red.PlayerAdded:Connect(function(Player)
	table.insert(Events, "Red.PlayerAdded " .. State(Player))
end)
Red.PlayerRemoved:Connect(function(Player)
	table.insert(Events, "Red.PlayerRemoved " .. State(Player))
end)
Red.Parent = Teams
local Player = P.player()
for _ = 1, 3 do
	task.wait()
end
for _, Line in Events do
	print(Line)
end
print("joined " .. State(Player) .. " red players " .. #Red:GetPlayers() .. " is red " .. tostring(Player.Team == Red))
P.meet("joined")
P.meet("seen")
--- client
local Player = Players.LocalPlayer
print("first " .. State(Player))
local Changes = {}
for _, Name in { "Team", "TeamColor", "Neutral" } do
	Player:GetPropertyChangedSignal(Name):Connect(function()
		table.insert(Changes, Name)
	end)
end
P.meet("joined")
local Teams = game:FindFirstChildOfClass("Teams")
local Red = Teams and Teams:FindFirstChild("Red")
print("joined " .. State(Player) .. " is red " .. tostring(Red ~= nil and Player.Team == Red))
print("red players " .. (if Red then #Red:GetPlayers() else -1) .. " teams " .. (if Teams then #Teams:GetTeams() else -1))
print("changes " .. table.concat(Changes, " "))
P.meet("seen")
