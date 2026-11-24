-- Where a WorldModel's own collision groups may be changed from, when the UseWorkspaceCollisionGroups warning comes (a
-- first call that fails), and which registry a part's CollisionGroupId follows through every change of the registries.
-- Removes everything it makes.
local LogService = game:GetService("LogService")
local PhysicsService = game:GetService("PhysicsService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local IsServer = RunService:IsServer()
print("side " .. (if IsServer then "server" else "client"))
print("start " .. P.try(function()
	return #workspace:GetRegisteredCollisionGroups()
end))

local Logged = {}
local Connection = LogService.MessageOut:Connect(function(Message, Kind)
	table.insert(Logged, Kind.Name .. ":" .. string.sub(Message, 1, 40))
end)
local function Took(Label, Do)
	table.clear(Logged)
	local Outcome = P.try(Do)
	task.wait()
	print(Label .. " " .. Outcome .. " | " .. table.concat(Logged, " / "))
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
local Holders = {}
local function Holder(Name)
	local Folder = Instance.new("Folder")
	Folder.Name = Name
	table.insert(Holders, Folder)
	return Folder
end

-- 1. where a WorldModel may be changed from
local Places = {
	{ "nil", function() return nil end },
	{ "ReplicatedStorage", function() return ReplicatedStorage end },
	{ "workspace", function() return workspace end },
	{ "workspace-folder", function()
		local Folder = Holder("F")
		Folder.Parent = workspace
		return Folder
	end },
	{ "workspace-model", function()
		local Model = Instance.new("Model")
		table.insert(Holders, Model)
		Model.Parent = workspace
		return Model
	end },
	{ "camera", function() return workspace.CurrentCamera end },
}
if not IsServer then
	table.insert(Places, { "viewport", function()
		local Gui = Instance.new("ScreenGui")
		table.insert(Holders, Gui)
		local Frame = Instance.new("ViewportFrame")
		Frame.Parent = Gui
		Gui.Parent = Players.LocalPlayer:WaitForChild("PlayerGui")
		return Frame
	end })
end
for _, Place in Places do
	local World = Instance.new("WorldModel")
	World.Parent = Place[2]()
	local Label = "place-" .. Place[1]
	print(Label .. " register " .. P.try(function() return World:RegisterCollisionGroup("A") end))
	print(Label .. " register-bad " .. P.try(function() return World:RegisterCollisionGroup("") end))
	print(Label .. " set " .. P.try(function() return World:CollisionGroupSetCollidable("Default", "Default", false) end))
	print(Label .. " rename " .. P.try(function() return World:RenameCollisionGroup("A", "B") end))
	print(Label .. " unregister " .. P.try(function() return World:UnregisterCollisionGroup("B") end))
	print(Label .. " list " .. Names(World))
	World.Parent = nil
	print(Label .. " taken-out " .. P.try(function() return World:RegisterCollisionGroup("C") end) .. " " .. Names(World))
	World:Destroy()
end
-- a WorldModel that leaves the Workspace and one put there after its groups were made
local Moved = Instance.new("WorldModel")
Moved:RegisterCollisionGroup("Early")
Moved.Parent = workspace
print("moved-in " .. P.try(function() return Moved:RegisterCollisionGroup("Late") end) .. " " .. Names(Moved))
print("moved-in-unregister " .. P.try(function() return Moved:UnregisterCollisionGroup("Early") end) .. " " .. Names(Moved))
print("moved-in-read " .. P.try(function() return Moved:IsCollisionGroupRegistered("Early"), Moved:CollisionGroupsAreCollidable("Default", "Default"), Moved:GetMaxCollisionGroups() end))
Moved:Destroy()

-- 2. the warning and errors: a fresh WorldModel with UseWorkspaceCollisionGroups on, its first call failing
for _, Case in {
	{ "nil-name", function(W) return W:RegisterCollisionGroup(nil) end },
	{ "empty-name", function(W) return W:RegisterCollisionGroup("") end },
	{ "unknown-pair", function(W) return W:CollisionGroupSetCollidable("X", "Y", false) end },
	{ "bad-bool", function(W) return W:CollisionGroupSetCollidable("Default", "Default", "no") end },
	{ "registered-nil", function(W) return W:IsCollisionGroupRegistered(nil) end },
	{ "max-dot", function(W) return W.GetMaxCollisionGroups() end },
	{ "list", function(W) return #W:GetRegisteredCollisionGroups() end },
} do
	local World = Instance.new("WorldModel")
	World.UseWorkspaceCollisionGroups = true
	Took("warn-first-" .. Case[1], function() return Case[2](World) end)
	Took("warn-then-" .. Case[1], function() return World:GetMaxCollisionGroups() end)
	World:Destroy()
end
do
	local World = Instance.new("WorldModel")
	World.UseWorkspaceCollisionGroups = true
	World.Parent = workspace
	Took("warn-in-workspace", function() return World:RegisterCollisionGroup("A") end)
	World:Destroy()
end
do
	-- a property read, a write and a Clone do not warn; the copy warns on its own first call
	local World = Instance.new("WorldModel")
	Took("warn-read-write", function()
		World.UseWorkspaceCollisionGroups = true
		return World.UseWorkspaceCollisionGroups
	end)
	Took("warn-clone-only", function() return World:Clone().UseWorkspaceCollisionGroups end)
	World:Destroy()
end

-- 3. parts: which registry their CollisionGroupId follows
local function Ids(...)
	local Out = {}
	for _, Part in { ... } do
		table.insert(Out, tostring(Part.CollisionGroupId))
	end
	return table.concat(Out, " ")
end
local function NewPart(Group, Parent)
	local Part = Instance.new("Part")
	Part.Anchored = true
	Part.CollisionGroup = Group
	Part.Parent = Parent
	return Part
end
local World = Instance.new("WorldModel")
World:RegisterCollisionGroup("Pa")
World:RegisterCollisionGroup("Pb")
local A = NewPart("Pa", World)
local B = NewPart("Pb", World)
local W = NewPart("Pw", World)
local Fired = {}
for _, Part in { A, B, W } do
	Part:GetPropertyChangedSignal("CollisionGroupId"):Connect(function()
		table.insert(Fired, Part.CollisionGroup)
	end)
end
local function Step(Label, Do)
	table.clear(Fired)
	local Outcome = P.try(Do)
	task.wait()
	print(Label .. " " .. Outcome .. " | " .. Ids(A, B, W) .. " | " .. table.concat(Fired, ","))
end
local Store = Holder("Store")
Store.Parent = ReplicatedStorage
Step("parts-replicated-storage", function() World.Parent = Store end)
Step("parts-workspace", function() World.Parent = workspace end)
Step("parts-model-register", function() World:RegisterCollisionGroup("Pc") end)
Step("parts-model-unregister", function() World:UnregisterCollisionGroup("Pa") end)
Step("parts-model-register-again", function() World:RegisterCollisionGroup("Pa") end)
Step("parts-model-rename", function() World:RenameCollisionGroup("Pb", "Pd") end)
Step("parts-model-rename-back", function() World:RenameCollisionGroup("Pd", "Pb") end)
Step("parts-model-collidable", function() World:CollisionGroupSetCollidable("Pa", "Pb", false) end)
Step("parts-workspace-register", function()
	if IsServer then
		workspace:RegisterCollisionGroup("Pw")
	end
	return workspace:IsCollisionGroupRegistered("Pw")
end)
Step("parts-use-true", function() World.UseWorkspaceCollisionGroups = true end)
Step("parts-use-true-model-register", function() World:RegisterCollisionGroup("Pe") end)
Step("parts-use-true-workspace-unregister", function()
	if IsServer then
		workspace:UnregisterCollisionGroup("Pw")
	end
end)
Step("parts-use-true-workspace-register", function()
	if IsServer then
		workspace:RegisterCollisionGroup("Pw")
	end
end)
Step("parts-use-false", function() World.UseWorkspaceCollisionGroups = false end)
Step("parts-name-write", function() W.CollisionGroup = "Pb" end)
Step("parts-name-write-back", function() W.CollisionGroup = "Pw" end)
local Inner = Instance.new("WorldModel")
Inner:RegisterCollisionGroup("Qa")
local N = NewPart("Qa", Inner)
Step("parts-nested", function()
	Inner.Parent = World
	return N.CollisionGroupId
end)
Step("parts-nested-outer-register", function()
	World:RegisterCollisionGroup("Qa")
	return N.CollisionGroupId
end)
Step("parts-moved-to-workspace", function()
	B.Parent = workspace
	return B.CollisionGroupId
end)
Step("parts-moved-back", function()
	B.Parent = World
	return B.CollisionGroupId
end)
Step("parts-model-out", function()
	World.Parent = Store
	World:UnregisterCollisionGroup("Pa")
end)
World:Destroy()
pcall(function()
	workspace:UnregisterCollisionGroup("Pw")
end)
for _, Made in Holders do
	Made:Destroy()
end
Connection:Disconnect()
print("end " .. Names(workspace))
