-- The warning a group name that is neither a string nor a number gives ("argument #N expects a string, but <typeof> was
-- passed", with the script's stack): which methods give it, for which arguments and in what order, and where it comes
-- among the call's other checks, on PhysicsService, the Workspace and a WorldModel. Each line is the call's outcome, then
-- the first line of each warning LogService.MessageOut brought within three frames (the stack's own lines are left out,
-- and so is the notice a deprecated member gives once a session). In Studio a warning reaches MessageOut within one
-- task.wait(); ree delivers one that has a stack a frame later, and the three frames leave room for that, not for Studio.
-- Removes every group it makes.
local LogService = game:GetService("LogService")
local PhysicsService = game:GetService("PhysicsService")
local RunService = game:GetService("RunService")
print("side " .. (if RunService:IsServer() then "server" else "client"))
print("start " .. P.try(function()
	return #workspace:GetRegisteredCollisionGroups()
end))
local Logged = {}
local Connection = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageWarning and not string.find(Message, "is deprecated", 1, true) then
		table.insert(Logged, (string.split(Message, "\n"))[1])
	end
end)
local function Took(Label, Do)
	table.clear(Logged)
	local Outcome = P.try(Do)
	for _ = 1, 3 do
		task.wait()
	end
	print(Label .. " " .. Outcome .. " | " .. table.concat(Logged, " / "))
end
local function Clean(Object)
	for _, Name in { "true", "{}" } do
		pcall(function()
			Object:UnregisterCollisionGroup(Name)
		end)
	end
end
local World = Instance.new("WorldModel")
for _, Root in { { "physics", PhysicsService }, { "workspace", workspace }, { "model", World } } do
	local Name, Object = Root[1], Root[2]
	Took(Name .. "-register-true", function() return Object:RegisterCollisionGroup(true) end)
	Took(Name .. "-registered-table", function() return Object:IsCollisionGroupRegistered({}) end)
	Took(Name .. "-registered-number", function() return Object:IsCollisionGroupRegistered(5) end)
	Took(Name .. "-are-both", function() return Object:CollisionGroupsAreCollidable(true, Vector3.one) end)
	Took(Name .. "-are-second", function() return Object:CollisionGroupsAreCollidable("Default", workspace) end)
	Took(Name .. "-set-both", function() return Object:CollisionGroupSetCollidable(true, {}, true) end)
	Took(Name .. "-set-bad-bool", function() return Object:CollisionGroupSetCollidable(true, {}, "no") end)
	Took(Name .. "-rename-both", function() return Object:RenameCollisionGroup(true, {}) end)
	Took(Name .. "-unregister-function", function() return Object:UnregisterCollisionGroup(print) end)
	Took(Name .. "-max-extra", function() return Object:GetMaxCollisionGroups(true) end)
	Clean(Object)
end
Took("physics-id-true", function() return PhysicsService:GetCollisionGroupId(true) end)
Took("physics-contains", function() return PhysicsService:CollisionGroupContainsPart(true, Instance.new("Part")) end)
Took("physics-set-part", function() return PhysicsService:SetPartCollisionGroup(Instance.new("Part"), true) end)
Took("physics-create", function() return PhysicsService:CreateCollisionGroup(true) end)
Took("physics-remove", function() return PhysicsService:RemoveCollisionGroup(true) end)
Took("part-property", function()
	local Part = Instance.new("Part")
	Part.CollisionGroup = true
	return Part.CollisionGroup
end)
Clean(PhysicsService)
Connection:Disconnect()
World:Destroy()
print("end " .. #workspace:GetRegisteredCollisionGroups())
