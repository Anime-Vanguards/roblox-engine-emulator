-- WorldRoot's collision-group methods: what they are, what they return, whether the Workspace's share PhysicsService's
-- registry, whether a WorldModel has its own (Clone, Destroy), and which side may change them. Removes every group it
-- makes.
local PhysicsService = game:GetService("PhysicsService")
local RunService = game:GetService("RunService")
local Side = if RunService:IsServer() then "server" else "client"
print("side " .. Side)

local Methods = { "CollisionGroupSetCollidable", "CollisionGroupsAreCollidable", "GetMaxCollisionGroups", "GetRegisteredCollisionGroups", "IsCollisionGroupRegistered", "RegisterCollisionGroup", "RenameCollisionGroup", "UnregisterCollisionGroup" }

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

local function Call(Root, Name, ...)
	local Arguments = table.pack(...)
	return P.try(function()
		return Root[Name](Root, table.unpack(Arguments, 1, Arguments.n))
	end)
end

-- 1. the members themselves
for _, Name in Methods do
	print("member " .. Name .. " " .. P.try(function()
		local Function = workspace[Name]
		return typeof(Function), Function == workspace[Name], Function == PhysicsService[Name]
	end))
end

-- 2. reads
print("max " .. P.try(function()
	local Max = workspace:GetMaxCollisionGroups()
	return Max, typeof(Max), math.type and math.type(Max) or "-"
end))
print("max-args " .. Call(workspace, "GetMaxCollisionGroups", 1, 2, 3))
print("list " .. Names(workspace))
print("list-shape " .. P.try(function()
	local List = workspace:GetRegisteredCollisionGroups()
	local Entry = List[1]
	local Keys = {}
	for Key in Entry do
		table.insert(Keys, tostring(Key))
	end
	table.sort(Keys)
	return typeof(List), #List, typeof(Entry), table.concat(Keys, "+"), typeof(Entry.mask), typeof(Entry.name), List == workspace:GetRegisteredCollisionGroups(), getmetatable(List) == nil, table.isfrozen(List), table.isfrozen(Entry)
end))
print("registered-Default " .. Call(workspace, "IsCollisionGroupRegistered", "Default"))
print("registered-Nope " .. Call(workspace, "IsCollisionGroupRegistered", "Nope"))
print("collidable-Default " .. Call(workspace, "CollisionGroupsAreCollidable", "Default", "Default"))
print("collidable-unknown " .. Call(workspace, "CollisionGroupsAreCollidable", "Nope", "Default"))

-- 3. writes through workspace, seen through PhysicsService and back
print("register-W " .. Call(workspace, "RegisterCollisionGroup", "WsA"))
print("seen-by-physics " .. Call(PhysicsService, "IsCollisionGroupRegistered", "WsA"))
print("register-P " .. Call(PhysicsService, "RegisterCollisionGroup", "PsA"))
print("seen-by-workspace " .. Call(workspace, "IsCollisionGroupRegistered", "PsA"))
print("set-W " .. Call(workspace, "CollisionGroupSetCollidable", "WsA", "PsA", false))
print("are-P " .. Call(PhysicsService, "CollisionGroupsAreCollidable", "WsA", "PsA"))
print("are-W " .. Call(workspace, "CollisionGroupsAreCollidable", "PsA", "WsA"))
print("list-after " .. Names(workspace))
print("list-physics " .. Names(PhysicsService))
print("rename-W " .. Call(workspace, "RenameCollisionGroup", "WsA", "WsB"))
print("renamed-physics " .. Call(PhysicsService, "IsCollisionGroupRegistered", "WsB"))
print("unregister-W " .. Call(workspace, "UnregisterCollisionGroup", "WsB"))
print("unregister-W2 " .. Call(workspace, "UnregisterCollisionGroup", "PsA"))
print("list-end " .. Names(PhysicsService))

-- 4. a WorldModel
local Model = Instance.new("WorldModel")
print("model-use-workspace " .. P.try(function()
	return Model.UseWorkspaceCollisionGroups
end))
print("model-list " .. Names(Model))
print("model-max " .. Call(Model, "GetMaxCollisionGroups"))
print("model-register " .. Call(Model, "RegisterCollisionGroup", "MdA"))
print("model-list-after " .. Names(Model))
print("model-seen-by-workspace " .. Call(workspace, "IsCollisionGroupRegistered", "MdA"))
print("model-seen-by-physics " .. Call(PhysicsService, "IsCollisionGroupRegistered", "MdA"))
print("model-set " .. Call(Model, "CollisionGroupSetCollidable", "MdA", "Default", false))
print("model-are " .. Call(Model, "CollisionGroupsAreCollidable", "MdA", "Default"))
print("model-workspace-list " .. Names(workspace))
print("workspace-register-for-model " .. Call(workspace, "RegisterCollisionGroup", "WsOnly"))
print("model-sees-workspace-group " .. Call(Model, "IsCollisionGroupRegistered", "WsOnly"))
print("model-use-true " .. P.try(function()
	Model.UseWorkspaceCollisionGroups = true
	return Model.UseWorkspaceCollisionGroups
end))
print("model-list-use-true " .. Names(Model))
print("model-registered-WsOnly-use-true " .. Call(Model, "IsCollisionGroupRegistered", "WsOnly"))
print("model-register-use-true " .. Call(Model, "RegisterCollisionGroup", "MdB"))
print("model-list-use-true-after " .. Names(Model))
print("workspace-list-use-true " .. Names(workspace))
print("model-use-false " .. P.try(function()
	Model.UseWorkspaceCollisionGroups = false
	return Model.UseWorkspaceCollisionGroups
end))
print("model-list-use-false " .. Names(Model))
print("model-in-workspace " .. P.try(function()
	Model.Parent = workspace
	return Names(Model)
end))
local Other = Instance.new("WorldModel")
print("other-model-list " .. Names(Other))
local Copy = P.try(function()
	return Names(Model:Clone())
end)
print("model-clone " .. Copy)
Other:Destroy()
Model:Destroy()
print("model-destroyed-list " .. Names(Model))
print("model-destroyed-register " .. Call(Model, "RegisterCollisionGroup", "MdC"))
print("model-destroyed-list-after " .. Names(Model))
pcall(function()
	workspace:UnregisterCollisionGroup("WsOnly")
end)
print("end " .. Names(workspace))
