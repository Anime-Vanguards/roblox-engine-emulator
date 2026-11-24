-- A WorldModel's own collision groups: which calls give the warning UseWorkspaceCollisionGroups brings and how often, its
-- parts' CollisionGroupId, Clone, and the Workspace's groups as its parts see them when changed through the Workspace.
-- Removes every group it makes.
local LogService = game:GetService("LogService")
local PhysicsService = game:GetService("PhysicsService")
local RunService = game:GetService("RunService")
local IsServer = RunService:IsServer()
print("side " .. (if IsServer then "server" else "client"))
print("start " .. P.try(function()
	return #workspace:GetRegisteredCollisionGroups()
end))

local Logged = {}
local Connection = LogService.MessageOut:Connect(function(Message, Kind)
	table.insert(Logged, Kind.Name .. ":" .. Message)
end)
local function Took(Label, Do)
	table.clear(Logged)
	local Outcome = P.try(Do)
	task.wait()
	print(Label .. " " .. Outcome .. " | " .. #Logged .. " | " .. table.concat(Logged, " / "))
end
local function Names(Root)
	local Ok, List = pcall(function()
		return Root:GetRegisteredCollisionGroups()
	end)
	if not Ok then
		return "error " .. P.err(List)
	end
	local Out = {}
	for _, Entry in List do
		table.insert(Out, tostring(Entry.name) .. "=" .. P.num(Entry.mask))
	end
	return table.concat(Out, ",")
end

-- 1. the warning: which calls give it, and how often
P.sync()
local Model = Instance.new("WorldModel")
Took("warn-set-true", function()
	Model.UseWorkspaceCollisionGroups = true
end)
Took("warn-max", function()
	return Model:GetMaxCollisionGroups()
end)
Took("warn-list", function()
	return #Model:GetRegisteredCollisionGroups()
end)
Took("warn-registered", function()
	return Model:IsCollisionGroupRegistered("Default")
end)
Took("warn-are", function()
	return Model:CollisionGroupsAreCollidable("Default", "Default")
end)
Took("warn-register", function()
	return Model:RegisterCollisionGroup("MdA")
end)
Took("warn-register-again", function()
	return Model:RegisterCollisionGroup("MdB")
end)
Took("warn-register-bad", function()
	return Model:RegisterCollisionGroup("")
end)
Took("warn-register-nil", function()
	return Model:RegisterCollisionGroup(nil)
end)
Took("warn-set", function()
	return Model:CollisionGroupSetCollidable("MdA", "MdB", false)
end)
Took("warn-set-unknown", function()
	return Model:CollisionGroupSetCollidable("Nope", "MdB", false)
end)
Took("warn-rename", function()
	return Model:RenameCollisionGroup("MdA", "MdC")
end)
Took("warn-rename-unknown", function()
	return Model:RenameCollisionGroup("Nope", "MdD")
end)
Took("warn-unregister", function()
	return Model:UnregisterCollisionGroup("MdC")
end)
Took("warn-unregister-unknown", function()
	return Model:UnregisterCollisionGroup("Nope")
end)
Took("warn-set-false", function()
	Model.UseWorkspaceCollisionGroups = false
end)
Took("warn-register-false", function()
	return Model:RegisterCollisionGroup("MdE")
end)
Took("warn-set-true-again", function()
	Model.UseWorkspaceCollisionGroups = true
end)
Took("warn-register-true-again", function()
	return Model:RegisterCollisionGroup("MdF")
end)
local Second = Instance.new("WorldModel")
Second.UseWorkspaceCollisionGroups = true
Took("warn-second-register", function()
	return Second:RegisterCollisionGroup("MdA")
end)
Took("warn-second-register-again", function()
	return Second:RegisterCollisionGroup("MdB")
end)
Took("warn-workspace-register", function()
	return workspace:RegisterCollisionGroup("WsW")
end)
pcall(function()
	workspace:UnregisterCollisionGroup("WsW")
end)
Took("warn-clone", function()
	local Copy = Model:Clone()
	return Copy.UseWorkspaceCollisionGroups, Names(Copy)
end)
print("model-list " .. Names(Model))
Second:Destroy()
Model:Destroy()

-- 2. parts in a WorldModel: whose groups their CollisionGroupId follows
local World = Instance.new("WorldModel")
local Part = Instance.new("Part")
Part.Anchored = true
Part.CollisionGroup = "MdA"
Part.Parent = World
print("part-in-new-model " .. P.try(function()
	return Part.CollisionGroup, Part.CollisionGroupId
end))
World:RegisterCollisionGroup("MdZ")
World:RegisterCollisionGroup("MdA")
task.wait()
print("part-after-model-register " .. P.try(function()
	return Part.CollisionGroup, Part.CollisionGroupId
end))
local Late = Instance.new("Part")
Late.Anchored = true
Late.CollisionGroup = "MdA"
Late.Parent = World
print("part-entered-model " .. P.try(function()
	return Late.CollisionGroupId
end))
Late.CollisionGroup = "MdZ"
print("part-renamed-in-model " .. P.try(function()
	return Late.CollisionGroupId
end))
World.Parent = workspace
task.wait()
print("model-in-workspace " .. P.try(function()
	return Part.CollisionGroupId, Late.CollisionGroupId
end))
Late.CollisionGroup = "MdA"
print("part-renamed-in-placed-model " .. P.try(function()
	return Late.CollisionGroupId
end))
pcall(function()
	PhysicsService:RegisterCollisionGroup("WsOnly")
end)
Late.CollisionGroup = "WsOnly"
task.wait()
print("part-workspace-group-in-model " .. P.try(function()
	return Late.CollisionGroupId
end))
World.UseWorkspaceCollisionGroups = true
task.wait()
print("part-use-workspace " .. P.try(function()
	return Part.CollisionGroupId, Late.CollisionGroupId
end))
Late.CollisionGroup = "MdA"
print("part-use-workspace-renamed " .. P.try(function()
	return Late.CollisionGroupId
end))
World.UseWorkspaceCollisionGroups = false
print("model-unregister-placed " .. P.try(function()
	return World:UnregisterCollisionGroup("MdZ")
end))
task.wait()
print("part-after-model-unregister " .. P.try(function()
	return Part.CollisionGroupId, Late.CollisionGroupId
end))
local Loose = Instance.new("Part")
Loose.Anchored = true
Loose.CollisionGroup = "WsOnly"
Loose.Parent = workspace
print("workspace-part " .. P.try(function()
	return Loose.CollisionGroupId
end))
print("workspace-register-moves-ids " .. P.try(function()
	workspace:RegisterCollisionGroup("WsZ")
	Loose.CollisionGroup = "WsZ"
	local Before = Loose.CollisionGroupId
	workspace:RenameCollisionGroup("WsZ", "WsY")
	return Before, Loose.CollisionGroup, Loose.CollisionGroupId
end))
print("physics-sees " .. P.try(function()
	return PhysicsService:GetCollisionGroupId("WsY")
end))
print("workspace-unregister-moves-ids " .. P.try(function()
	workspace:UnregisterCollisionGroup("WsY")
	return Loose.CollisionGroupId
end))

-- 3. Clone of a model that holds a WorldModel, and of a WorldModel's copy
local Holder = Instance.new("Model")
local Inner = Instance.new("WorldModel")
Inner:RegisterCollisionGroup("Held")
Inner.Parent = Holder
print("clone-holder " .. P.try(function()
	return Names(Holder:Clone():FindFirstChildOfClass("WorldModel"))
end))
print("clone-twice " .. P.try(function()
	local Copy = Inner:Clone()
	Copy:RegisterCollisionGroup("CopyOnly")
	return Names(Inner), Names(Copy), Names(Copy:Clone())
end))

Loose:Destroy()
World:Destroy()
Holder:Destroy()
pcall(function()
	PhysicsService:UnregisterCollisionGroup("WsOnly")
end)
Connection:Disconnect()
print("end " .. Names(workspace))
