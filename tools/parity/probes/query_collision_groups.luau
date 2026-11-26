-- params.CollisionGroup in the spatial queries (world/query.rs, services/physics.rs): a server registers groups, some pairs
-- not collidable, and casts and overlap queries with each group's params (and a name not registered) meet parts in each group;
-- the caster's own group, the legacy FindPartOnRay and Region3 queries, and a registry change after the params were set. A
-- client may not register groups: it prints whether the server's are there (none in a plain client run). Removes its groups.
P.sync()
local PhysicsService = game:GetService("PhysicsService")
local RunService = game:GetService("RunService")
if not RunService:IsServer() then
	print("client: registered", PhysicsService:IsCollisionGroupRegistered("QA_A"))
	return
end
for _, Name in { "QA_A", "QA_B", "QA_C" } do
	PhysicsService:RegisterCollisionGroup(Name)
end
PhysicsService:CollisionGroupSetCollidable("QA_A", "QA_B", false)
PhysicsService:CollisionGroupSetCollidable("QA_C", "QA_C", false)
PhysicsService:CollisionGroupSetCollidable("Default", "QA_C", false)
local Root = Instance.new("Folder")
Root.Parent = workspace
local Groups = { "Default", "QA_A", "QA_B", "QA_C" }
local Parts = {}
for Index, Group in Groups do
	local Made = Instance.new("Part")
	Made.Name = "in_" .. Group
	Made.Anchored = true
	Made.Size = Vector3.new(10, 1, 10)
	Made.Position = Vector3.new(300 + 40 * Index, 90, 300)
	Made.CollisionGroup = Group
	Made.Parent = Root
	Parts[Index] = Made
end
local Off = Instance.new("Part")
Off.Name = "in_QA_B_CanCollideOff"
Off.Anchored = true
Off.CanCollide = false
Off.Size = Vector3.new(10, 1, 10)
Off.Position = Vector3.new(300 + 40 * 5, 90, 300)
Off.CollisionGroup = "QA_B"
Off.Parent = Root
table.insert(Parts, Off)
local Caster = Instance.new("Part")
Caster.Anchored = true
Caster.CanQuery = false
Caster.CanCollide = false
Caster.Size = Vector3.new(1, 1, 1)
Caster.Position = Vector3.new(0, 300, 0)
Caster.Parent = Root
local Probe = Instance.new("Part")
Probe.Anchored = true
Probe.CanQuery = false
Probe.CanCollide = false
Probe.Size = Vector3.new(2, 2, 2)
Probe.Parent = Root
task.wait()

local function Name(Result)
	return if Result then "hit" else "miss"
end
local function Mine(List, Part)
	for _, Item in List do
		if Item == Part then
			return "found"
		end
	end
	return "none"
end
for _, Group in { "Default", "QA_A", "QA_B", "QA_C", "Nope" } do
	local Params = RaycastParams.new()
	local Overlap = OverlapParams.new()
	print("set", Group, P.try(function() Params.CollisionGroup = Group end), P.try(function() Overlap.CollisionGroup = Group end), Params.CollisionGroup)
	for _, Part in Parts do
		local Top = Part.Position + Vector3.new(0, 10, 0)
		local Down = Vector3.new(0, -20, 0)
		Caster.CFrame = CFrame.new(Top)
		Probe.CFrame = CFrame.new(Part.Position)
		local Out = {}
		local function Add(Label, F)
			local Ok, Result = pcall(F)
			table.insert(Out, Label .. " " .. (if Ok then Result else "error " .. P.err(Result)))
		end
		Add("ray", function() return Name(workspace:Raycast(Top, Down, Params)) end)
		Add("block", function() return Name(workspace:Blockcast(CFrame.new(Top), Vector3.new(1, 1, 1), Down, Params)) end)
		Add("sphere", function() return Name(workspace:Spherecast(Top, 0.5, Down, Params)) end)
		Add("shape", function() return Name(workspace:Shapecast(Caster, Down, Params)) end)
		Add("box", function() return Mine(workspace:GetPartBoundsInBox(CFrame.new(Part.Position), Vector3.new(2, 2, 2), Overlap), Part) end)
		Add("radius", function() return Mine(workspace:GetPartBoundsInRadius(Part.Position, 1, Overlap), Part) end)
		Add("inpart", function() return Mine(workspace:GetPartsInPart(Probe, Overlap), Part) end)
		print(Group, Part.Name, table.concat(Out, " | "))
	end
end
-- the caster's or query part's own group
for _, Group in { "Default", "QA_A", "QA_C" } do
	Caster.CollisionGroup = Group
	Probe.CollisionGroup = Group
	for _, Part in Parts do
		local Top = Part.Position + Vector3.new(0, 10, 0)
		Caster.CFrame = CFrame.new(Top)
		Probe.CFrame = CFrame.new(Part.Position)
		print("own", Group, Part.Name, "shape", Name(workspace:Shapecast(Caster, Vector3.new(0, -20, 0))), "inpart", Mine(workspace:GetPartsInPart(Probe), Part))
	end
end
Caster.CollisionGroup = "Default"
Probe.CollisionGroup = "Default"
-- the legacy queries (no params)
for _, Part in Parts do
	local Top = Part.Position + Vector3.new(0, 10, 0)
	local Hit = workspace:FindPartOnRay(Ray.new(Top, Vector3.new(0, -20, 0)))
	local Region = Region3.new(Part.Position - Vector3.new(1, 1, 1), Part.Position + Vector3.new(1, 1, 1))
	print("legacy", Part.Name, "fpor", if Hit then "hit" else "miss", "region", Mine(workspace:FindPartsInRegion3(Region, nil, 100), Part), "empty", tostring(workspace:IsRegion3Empty(Region)))
end
-- a group changed after the params were set
local Params = RaycastParams.new()
Params.CollisionGroup = "QA_A"
PhysicsService:CollisionGroupSetCollidable("QA_A", "QA_B", true)
print("after set collidable", Name(workspace:Raycast(Parts[3].Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0), Params)))
PhysicsService:CollisionGroupSetCollidable("QA_A", "QA_B", false)
PhysicsService:UnregisterCollisionGroup("QA_A")
print("after unregister", P.try(function() return Name(workspace:Raycast(Parts[3].Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0), Params)) end), Params.CollisionGroup)
print("after unregister C part", P.try(function() return Name(workspace:Raycast(Parts[4].Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0), Params)) end))
Root:Destroy()
for _, Name in { "QA_B", "QA_C" } do
	PhysicsService:UnregisterCollisionGroup(Name)
end
