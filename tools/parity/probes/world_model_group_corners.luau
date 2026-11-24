-- The corners of a WorldModel's own collision groups and its parts' CollisionGroupId: a WorldModel under a parentless
-- Folder or Part, moves in and out of the DataModel, UseWorkspaceCollisionGroups changes inside and outside it, an id a
-- script wrote, and a WorldModel inside a WorldModel. Removes everything it makes.
local PhysicsService = game:GetService("PhysicsService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local IsServer = RunService:IsServer()
print("side " .. (if IsServer then "server" else "client"))
print("start " .. P.try(function()
	return #workspace:GetRegisteredCollisionGroups()
end))

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

-- 1. a client and a WorldModel outside the DataModel but under something
do
	local Folder = Instance.new("Folder")
	local World = Instance.new("WorldModel")
	World.Parent = Folder
	print("in-loose-folder " .. P.try(function() return World:RegisterCollisionGroup("A") end) .. " " .. Names(World))
	local Part = Instance.new("Part")
	Part.Parent = Folder
	local Inner = Instance.new("WorldModel")
	Inner.Parent = Part
	print("under-loose-part " .. P.try(function() return Inner:RegisterCollisionGroup("A") end))
	Folder.Parent = ReplicatedStorage
	print("folder-moved-in " .. P.try(function() return World:RegisterCollisionGroup("B") end) .. " " .. Names(World))
	Folder.Parent = nil
	print("folder-moved-out " .. P.try(function() return World:RegisterCollisionGroup("B") end) .. " " .. Names(World))
	Folder:Destroy()
	print("destroyed " .. P.try(function() return World:RegisterCollisionGroup("C") end) .. " " .. Names(World))
end

-- 2. parts
local Fired = {}
local function Watch(Part)
	Part.Changed:Connect(function(Name)
		if Name == "CollisionGroupId" then
			table.insert(Fired, Part.Name)
		end
	end)
end
local function NewPart(Name, Group, Parent)
	local Part = Instance.new("Part")
	Part.Name = Name
	Part.Anchored = true
	Part.CollisionGroup = Group
	Watch(Part)
	Part.Parent = Parent
	return Part
end
local World = Instance.new("WorldModel")
World:RegisterCollisionGroup("Pa")
local A = NewPart("A", "Pa", World)
local W = NewPart("W", "Pw", World)
local Parts = { A, W }
local function Step(Label, Do)
	table.clear(Fired)
	local Outcome = P.try(Do)
	task.wait()
	task.wait()
	task.wait()
	local Ids = {}
	for _, Part in Parts do
		table.insert(Ids, Part.Name .. "=" .. tostring(Part.CollisionGroupId))
	end
	print(Label .. " " .. Outcome .. " | " .. table.concat(Ids, " ") .. " | " .. table.concat(Fired, ","))
end
if IsServer then
	PhysicsService:RegisterCollisionGroup("Pw")
end
Step("p-outside", function() end)
Step("p-outside-toggle", function() World.UseWorkspaceCollisionGroups = true end)
Step("p-outside-toggle-back", function() World.UseWorkspaceCollisionGroups = false end)
Step("p-outside-unregister", function() World:UnregisterCollisionGroup("Pa") end)
Step("p-outside-register", function() World:RegisterCollisionGroup("Pa") end)
Step("p-in", function() World.Parent = workspace end)
Step("p-toggle-true", function() World.UseWorkspaceCollisionGroups = true end)
Step("p-true-name-write", function() A.CollisionGroup = "Pw" end)
Step("p-true-name-write-back", function() A.CollisionGroup = "Pa" end)
local E = NewPart("E", "Pw", nil)
table.insert(Parts, E)
Step("p-true-part-enters", function() E.Parent = World end)
Step("p-toggle-false", function() World.UseWorkspaceCollisionGroups = false end)
Step("p-toggle-false-again", function() World.UseWorkspaceCollisionGroups = false end)
Step("p-false-script-id", function() A.CollisionGroupId = 7 end)
Step("p-false-toggle-over-script-id", function()
	World.UseWorkspaceCollisionGroups = true
	World.UseWorkspaceCollisionGroups = false
end)
Step("p-moved-within", function()
	A.CollisionGroupId = 9
	local Model = Instance.new("Model")
	Model.Parent = World
	A.Parent = Model
end)
Step("p-world-out", function() World.Parent = nil end)
Step("p-out-toggle", function() World.UseWorkspaceCollisionGroups = true end)
Step("p-world-in-true", function() World.Parent = workspace end)
Step("p-world-out-again", function()
	World.Parent = nil
	World.UseWorkspaceCollisionGroups = false
end)
Step("p-world-in-false", function() World.Parent = workspace end)
-- an inner WorldModel with UseWorkspaceCollisionGroups on inside an outer one
local Inner = Instance.new("WorldModel")
Inner:RegisterCollisionGroup("Pw")
Inner:RegisterCollisionGroup("Pa")
Inner.UseWorkspaceCollisionGroups = true
local I = NewPart("I", "Pw", Inner)
table.insert(Parts, I)
Step("p-inner-true-enters", function() Inner.Parent = World end)
Step("p-inner-false", function() Inner.UseWorkspaceCollisionGroups = false end)
World:Destroy()
pcall(function()
	PhysicsService:UnregisterCollisionGroup("Pw")
end)
print("end " .. Names(workspace))
