-- parity: loopback
-- A client's spatial queries with params.CollisionGroup over the groups its server registered, the parts made by the server:
-- each params group against a part in each group, a name not registered, and FindPartOnRay (Default's group).
local PhysicsService = game:GetService("PhysicsService")
local function Settle()
	for _ = 1, 30 do
		task.wait()
	end
end
--- server
for _, Name in { "QL_A", "QL_B", "QL_C" } do
	PhysicsService:RegisterCollisionGroup(Name)
end
PhysicsService:CollisionGroupSetCollidable("QL_A", "QL_B", false)
PhysicsService:CollisionGroupSetCollidable("QL_C", "QL_C", false)
PhysicsService:CollisionGroupSetCollidable("Default", "QL_C", false)
local Root = Instance.new("Folder")
Root.Name = "QLRoot"
for Index, Group in { "Default", "QL_A", "QL_B", "QL_C" } do
	local Made = Instance.new("Part")
	Made.Name = "in_" .. Group
	Made.Anchored = true
	Made.Size = Vector3.new(10, 1, 10)
	Made.Position = Vector3.new(300 + 40 * Index, 90, 300)
	Made.CollisionGroup = Group
	Made.Parent = Root
end
Root.Parent = workspace
P.player()
Settle()
P.meet("ready")
P.meet("done")
Root:Destroy()
for _, Name in { "QL_A", "QL_B", "QL_C" } do
	PhysicsService:UnregisterCollisionGroup(Name)
end
--- client
P.meet("ready")
local Root = workspace:WaitForChild("QLRoot")
for _, Group in { "Default", "QL_A", "QL_B", "QL_C", "Nope" } do
	local Params = RaycastParams.new()
	Params.CollisionGroup = Group
	local Overlap = OverlapParams.new()
	Overlap.CollisionGroup = Group
	local Out = {}
	for Index, Name in { "Default", "QL_A", "QL_B", "QL_C" } do
		local Part = Root:FindFirstChild("in_" .. Name)
		local Hit = workspace:Raycast(Part.Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0), Params)
		local Found = workspace:GetPartBoundsInBox(Part.CFrame, Vector3.new(2, 2, 2), Overlap)
		table.insert(Out, Name .. "=" .. (if Hit then "hit" else "miss") .. "/" .. #Found)
	end
	print(Group, table.concat(Out, " "))
end
local Part = Root:FindFirstChild("in_QL_C")
print("fpor C", workspace:FindPartOnRay(Ray.new(Part.Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0))) ~= nil)
P.meet("done")
