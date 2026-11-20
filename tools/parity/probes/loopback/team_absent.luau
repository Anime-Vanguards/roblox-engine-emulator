-- parity: loopback
-- A joining player in a place with no team at all (Teams is never asked for here: the capture's place has an empty Teams
-- service, ree's empty project none): the player's Team, TeamColor and Neutral on each side.
local Players = game:GetService("Players")
local function State(Player)
	return `team={if Player.Team then Player.Team.Name else "nil"} color={Player.TeamColor.Name} neutral={Player.Neutral}`
end
--- server
local Player = P.player()
for _ = 1, 3 do
	task.wait()
end
print("joined " .. State(Player))
P.meet("joined")
--- client
local Player = Players.LocalPlayer
print("first " .. State(Player))
P.meet("joined")
print("joined " .. State(Player))
