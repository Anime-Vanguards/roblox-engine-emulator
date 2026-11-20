-- parity: loopback
-- A joining player with no team to go to: Teams holds only a team that is not auto-assignable. The player's Team, TeamColor
-- and Neutral on each side, and what each side's Players and Teams answer.
local Players = game:GetService("Players")
local function State(Player)
	return `team={if Player.Team then Player.Team.Name else "nil"} color={Player.TeamColor.Name} neutral={Player.Neutral}`
end
--- server
local Teams = game:GetService("Teams")
local Fixed = Instance.new("Team")
Fixed.Name = "Fixed"
Fixed.TeamColor = BrickColor.new("Bright red")
Fixed.AutoAssignable = false
Fixed.Parent = Teams
local Player = P.player()
for _ = 1, 3 do
	task.wait()
end
print("joined " .. State(Player) .. " fixed players " .. #Fixed:GetPlayers())
P.meet("joined")
--- client
local Player = Players.LocalPlayer
print("first " .. State(Player))
P.meet("joined")
print("joined " .. State(Player))
