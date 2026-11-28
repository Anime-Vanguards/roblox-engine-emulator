-- Making a Player: a game script cannot, on a client or a server, refused before the parent argument is looked at; nor copy one
-- (Instance.fromExisting); Clone answers nil; a class name in another case is no class (src/instance.rs new_refusal)
local Players = game:GetService("Players")
print("new", P.try(function() return Instance.new("Player") end))
print("new in Players", P.try(function() return Instance.new("Player", Players) end))
print("new bad parent", P.try(function() return Instance.new("Player", 5) end))
print("new table parent", P.try(function() return Instance.new("Player", {}) end))
print("new lower", P.try(function() return Instance.new("player") end))
local Existing = Players.LocalPlayer or Players:GetPlayers()[1]
if not Existing then
	-- (ree's server run has none: its harness lets one in, Context.Join; the real engine's job has no Context)
	Existing = Context.Join()
end
print("fromExisting", P.try(function() local Copy = Instance.fromExisting(Existing) return Copy and Copy.ClassName end))
print("Clone", P.try(function() local Copy = Existing:Clone() return Copy and Copy.ClassName end))
print("archivable", Existing.Archivable)
print("archivable clone", P.try(function()
	local Was = Existing.Archivable
	Existing.Archivable = true
	local Copy = Existing:Clone()
	Existing.Archivable = Was
	return Copy and Copy.ClassName, Copy and Copy.Parent
end))
print("players", #Players:GetPlayers())
