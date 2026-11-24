-- parity: loopback
-- What a client's copy of a server WorldModel has of the WorldModel's own collision groups: one in the Workspace before the
-- player joins, one that comes in while it plays, a later change of the first one's groups, a part's CollisionGroupId in
-- each, the Workspace's groups beside them, and whether the client may change them. Each change has 30 frames to reach
-- the client before it reads, and the server changes nothing while the client reads.
local function Groups(Root)
	local Ok, List = pcall(Root.GetRegisteredCollisionGroups, Root)
	if not Ok then
		return "error " .. P.err(List)
	elseif type(List) ~= "table" then
		return typeof(List)
	end
	local Parts = {}
	for _, Entry in List do
		table.insert(Parts, tostring(Entry.name) .. "=" .. P.num(Entry.mask))
	end
	return #List .. " " .. table.concat(Parts, " ")
end
local function Id(World)
	local Part = World and World:FindFirstChild("P")
	return if Part then tostring(Part.CollisionGroupId) else "no part"
end
local function Settle()
	for _ = 1, 30 do
		task.wait()
	end
end
local function NewWorld(Name, Groups, PartGroup)
	local World = Instance.new("WorldModel")
	World.Name = Name
	for _, Group in Groups do
		World:RegisterCollisionGroup(Group)
	end
	local Part = Instance.new("Part")
	Part.Name = "P"
	Part.Anchored = true
	Part.CollisionGroup = PartGroup
	Part.Parent = World
	return World
end
--- server
local Early = NewWorld("LoopWorldEarly", { "EarlyA", "EarlyB" }, "EarlyB")
Early:CollisionGroupSetCollidable("EarlyA", "EarlyB", false)
Early.Parent = workspace
task.wait()
print("boot " .. Groups(Early) .. " | " .. Id(Early))
P.player()
P.meet("joined")
P.meet("read-join")
local Late = NewWorld("LoopWorldLate", { "LateA" }, "LateA")
Late.Parent = workspace
Settle()
P.meet("late")
P.meet("read-late")
Early:RegisterCollisionGroup("EarlyC")
Early:UnregisterCollisionGroup("EarlyA")
Early:RenameCollisionGroup("EarlyB", "EarlyD")
Early:CollisionGroupSetCollidable("EarlyC", "EarlyD", false)
Settle()
P.meet("changed")
P.meet("read-changed")
print("end " .. Groups(Early) .. " | " .. Id(Early))
Early:Destroy()
Late:Destroy()
--- client
P.meet("joined")
local Early = workspace:WaitForChild("LoopWorldEarly", 10)
print("join " .. (if Early then Groups(Early) .. " | " .. Id(Early) else "missing"))
print("join-workspace " .. Groups(workspace))
print("join-register " .. P.try(function()
	return Early:RegisterCollisionGroup("ClientA")
end))
print("join-set " .. P.try(function()
	return Early:CollisionGroupSetCollidable("Default", "EarlyA", false)
end))
P.meet("read-join")
P.meet("late")
local Late = workspace:WaitForChild("LoopWorldLate", 10)
print("late " .. (if Late then Groups(Late) .. " | " .. Id(Late) else "missing"))
P.meet("read-late")
P.meet("changed")
print("changed " .. Groups(Early) .. " | " .. Id(Early))
print("changed-collidable " .. P.try(function()
	return Early:CollisionGroupsAreCollidable("EarlyC", "EarlyD"), Early:IsCollisionGroupRegistered("EarlyA")
end))
P.meet("read-changed")
