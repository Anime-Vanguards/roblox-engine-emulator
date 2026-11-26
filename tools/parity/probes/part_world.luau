-- The part methods that read the world (torque conversions, GetVelocityAtPosition, GetClosestPointOnSurface, CanCollideWith,
-- network ownership, IsGrounded, GetTouchingParts, GetRenderCFrame, Resize), the old Model methods (GetModelCFrame, GetModelSize,
-- the identity orientation, move and MoveTo), persistent players and Model:PivotTo to the bit.
P.sync()
workspace.Gravity = 0
local RS = game:GetService("ReplicatedStorage")
local Players = game:GetService("Players")
local PhysicsService = game:GetService("PhysicsService")
local Spot = 0
local function Part(Name, Anchored, Parent, Size, At, Class)
	Spot += 1
	local Made = Instance.new(Class or "Part")
	Made.Name = Name
	Made.Size = Size or Vector3.new(1, 1, 1)
	Made.Anchored = Anchored or false
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = At or CFrame.new(300 + 5 * Spot, 60, 40)
	if Parent ~= "none" then
		Made.Parent = Parent or workspace
	end
	return Made
end
local function V(A)
	return P.f32s(A.X, A.Y, A.Z)
end
local function CF(C)
	return P.f32s(C:GetComponents())
end
local function Names(List)
	local Out = {}
	for _, Each in List do
		table.insert(Out, if typeof(Each) == "Instance" then Each.Name else tostring(Each))
	end
	return "{" .. table.concat(Out, ",") .. "}"
end
local function Show(A, ...)
	if typeof(A) == "Vector3" then
		return V(A)
	elseif typeof(A) == "CFrame" then
		return CF(A)
	elseif typeof(A) == "Instance" then
		return A.Name
	elseif type(A) == "table" then
		return Names(A)
	elseif type(A) == "number" then
		return P.f32(A)
	end
	local Rest = table.pack(...)
	if Rest.n > 0 then
		local Out = { tostring(A) }
		for Index = 1, Rest.n do
			table.insert(Out, tostring(Rest[Index]))
		end
		return table.concat(Out, "|")
	end
	return tostring(A)
end
local function Try(F)
	local Results = table.pack(pcall(F))
	if not Results[1] then
		return "error " .. P.err(Results[2])
	end
	return Show(table.unpack(Results, 2, Results.n))
end
local Made = {}
local function Clean()
	for _, Each in Made do
		pcall(function() Each:Destroy() end)
	end
	table.clear(Made)
end
local function Keep(...)
	for _, Each in { ... } do
		table.insert(Made, Each)
	end
	return ...
end
local Turn = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7)

-- torque, angular acceleration and velocity at a point
do
	local A = Keep(Part("A", false, nil, Vector3.new(2, 4, 6), CFrame.new(300, 60, 40)))
	local B = Keep(Part("B", false, nil, Vector3.new(2, 4, 6), CFrame.new(310, 60, 40) * Turn))
	local C = Keep(Part("C", false, nil, Vector3.new(1, 1, 1), CFrame.new(312, 61, 40)))
	local W = Keep(Instance.new("Weld"))
	W.Part0, W.Part1, W.C0 = B, C, B.CFrame:ToObjectSpace(C.CFrame)
	W.Parent = B
	local D = Keep(Part("D", true, nil, Vector3.new(2, 4, 6)))
	local E = Keep(Part("E", false, "none", Vector3.new(2, 4, 6)))
	local F = Keep(Part("F", false, nil, Vector3.new(2, 4, 6)))
	F.Massless = true
	local G = Keep(Part("G", false, nil, Vector3.new(3, 3, 3), CFrame.new(330, 60, 40) * Turn, "Part"))
	G.Shape = Enum.PartType.Ball
	local H = Keep(Part("H", false, nil, Vector3.new(2, 4, 6), CFrame.new(340, 60, 40) * Turn, "WedgePart"))
	for _, Each in { A, B, C, D, E, F, G, H } do
		print("torque", Each.Name, Try(function() return Each:AngularAccelerationToTorque(Vector3.new(1, 2, 3)) end),
			Try(function() return Each:AngularAccelerationToTorque(Vector3.new(1, 2, 3), Vector3.new(0.5, -1, 2)) end),
			Try(function() return Each:TorqueToAngularAcceleration(Vector3.new(10, 20, 30)) end),
			Try(function() return Each:TorqueToAngularAcceleration(Vector3.new(10, 20, 30), Vector3.new(0.5, -1, 2)) end))
	end
	B.AssemblyLinearVelocity = Vector3.new(1, 2, 3)
	B.AssemblyAngularVelocity = Vector3.new(0.4, 0.5, 0.6)
	A.AssemblyLinearVelocity = Vector3.new(-1, 0, 2)
	for _, Each in { A, B, C, D, E } do
		print("velocity-at", Each.Name, Try(function() return Each:GetVelocityAtPosition(Vector3.new(311, 62, 41)) end), Try(function() return Each:GetVelocityAtPosition(Each.Position) end))
	end
	print("torque-args", Try(function() return A:AngularAccelerationToTorque() end), Try(function() return A:AngularAccelerationToTorque("x") end), Try(function() return A:AngularAccelerationToTorque(Vector3.one, 5) end), Try(function() return A:GetVelocityAtPosition() end), Try(function() return A:GetVelocityAtPosition(CFrame.new()) end))
	Clean()
end

-- closest point on the surface
do
	local Shapes = {
		{ "block", "Part", Enum.PartType.Block },
		{ "ball", "Part", Enum.PartType.Ball },
		{ "cylinder", "Part", Enum.PartType.Cylinder },
		{ "wedge-shape", "Part", Enum.PartType.Wedge },
		{ "cornerwedge-shape", "Part", Enum.PartType.CornerWedge },
		{ "wedge", "WedgePart" },
		{ "cornerwedge", "CornerWedgePart" },
		{ "truss", "TrussPart" },
		{ "mesh", "MeshPart" },
	}
	for _, Shape in Shapes do
		local A = Keep(Part("A", true, nil, Vector3.new(2, 4, 6), CFrame.new(400, 60, 40) * Turn, Shape[2]))
		if Shape[3] then
			A.Shape = Shape[3]
		end
		local Out = {}
		for _, Point in { Vector3.new(400, 60, 40), Vector3.new(410, 60, 40), Vector3.new(400.3, 60.2, 40.1), Vector3.new(401, 62, 43), Vector3.new(395, 55, 38), Vector3.new(400, 70, 40) } do
			table.insert(Out, Try(function() return A:GetClosestPointOnSurface(Point) end))
		end
		print("closest", Shape[1], table.concat(Out, " / "))
		Clean()
	end
	local A = Keep(Part("A", true, "none", Vector3.new(2, 4, 6)))
	print("closest-out", Try(function() return A:GetClosestPointOnSurface(Vector3.new(0, 0, 0)) end), Try(function() return A:GetClosestPointOnSurface() end))
	Clean()
end

-- CanCollideWith
do
	pcall(function()
		PhysicsService:RegisterCollisionGroup("G2a")
		PhysicsService:RegisterCollisionGroup("G2b")
		PhysicsService:CollisionGroupSetCollidable("G2a", "G2b", false)
	end)
	local A, B, C, D, E = Keep(Part("A"), Part("B"), Part("C"), Part("D", true), Part("E", false, "none"))
	for _, Each in { A, B, C, D, E } do
		Each.CanCollide = true
	end
	local function Row(Label)
		print("cancollide", Label, Try(function() return A:CanCollideWith(B) end), Try(function() return B:CanCollideWith(A) end), Try(function() return A:CanCollideWith(C) end), Try(function() return A:CanCollideWith(D) end), Try(function() return A:CanCollideWith(E) end), Try(function() return A:CanCollideWith(A) end))
	end
	Row("plain")
	B.CanCollide = false
	Row("b-off")
	B.CanCollide = true
	local Groups = pcall(function()
		A.CollisionGroup = "G2a"
		B.CollisionGroup = "G2b"
	end)
	Row("groups " .. tostring(Groups))
	A.CollisionGroup = "Default"
	B.CollisionGroup = "Default"
	local W = Keep(Instance.new("Weld"))
	W.Part0, W.Part1 = A, B
	W.Parent = A
	Row("welded")
	W:Destroy()
	local N = Keep(Instance.new("NoCollisionConstraint"))
	N.Part0, N.Part1 = A, B
	N.Parent = A
	Row("nocollide")
	N:Destroy()
	A.CanCollide = false
	Row("a-off")
	print("cancollide-args", Try(function() return A:CanCollideWith() end), Try(function() return A:CanCollideWith(Instance.new("Folder")) end), Try(function() return A:CanCollideWith("x") end))
	Clean()
end

-- network ownership
do
	local A, B, C, D = Keep(Part("A"), Part("B", true), Part("C"), Part("D", false, "none"))
	local W = Keep(Instance.new("Weld"))
	W.Part0, W.Part1 = B, C
	W.Parent = B
	local Me = Players.LocalPlayer or Players:GetPlayers()[1]
	for _, Each in { A, B, C, D } do
		print("owner", Each.Name, Try(function() return Each:GetNetworkOwner() end), Try(function() return Each:GetNetworkOwnershipAuto() end), Try(function() return Each:CanSetNetworkOwnership() end))
		print("owner-set", Each.Name, Try(function() Each:SetNetworkOwner(nil) end), Try(function() return Each:GetNetworkOwner() end), Try(function() return Each:GetNetworkOwnershipAuto() end))
		print("owner-set-player", Each.Name, Try(function() Each:SetNetworkOwner(Me) end), Try(function() return Each:GetNetworkOwner() end), Try(function() Each:SetNetworkOwnershipAuto() end), Try(function() return Each:GetNetworkOwnershipAuto() end))
	end
	print("owner-args", Try(function() A:SetNetworkOwner(Instance.new("Folder")) end), Try(function() A:SetNetworkOwner("x") end))
	Clean()
end

-- IsGrounded, GetTouchingParts, GetRenderCFrame
do
	local A, B, C, D = Keep(Part("A", true, nil, Vector3.new(4, 4, 4), CFrame.new(500, 60, 40)), Part("B", false, nil, Vector3.new(4, 4, 4), CFrame.new(502, 60, 40)), Part("C", false, nil, Vector3.new(2, 2, 2), CFrame.new(520, 60, 40)), Part("D", false, "none"))
	local E = Keep(Part("E", true, nil, Vector3.new(4, 4, 4), CFrame.new(501, 61, 40)))
	for _, Each in { A, B, C, E } do
		Each.CanCollide = true
		Each.CanTouch = true
	end
	local W = Keep(Instance.new("Weld"))
	W.Part0, W.Part1 = A, C
	W.C0 = A.CFrame:ToObjectSpace(C.CFrame)
	W.Parent = A
	for _, Each in { A, B, C, D, E } do
		print("grounded", Each.Name, Try(function() return Each:IsGrounded() end), "touching", Try(function() return Each:GetTouchingParts() end), "render", Try(function() return Each:GetRenderCFrame() end))
	end
	task.wait()
	task.wait()
	for _, Each in { A, B, C, D, E } do
		print("grounded-later", Each.Name, Try(function() return Each:IsGrounded() end), "touching", Try(function() return Each:GetTouchingParts() end))
	end
	Clean()
end

-- Resize
do
	for _, Case in {
		{ "top-2", Enum.NormalId.Top, 2 },
		{ "front-1", Enum.NormalId.Front, 1 },
		{ "left-neg", Enum.NormalId.Left, -1 },
		{ "back-too-small", Enum.NormalId.Back, -10 },
		{ "bottom-frac", Enum.NormalId.Bottom, 1.7 },
		{ "number-id", 1, 2 },
		{ "string-id", "Top", 2 },
		{ "nil-delta", Enum.NormalId.Top, nil },
	} do
		local A = Keep(Part("A", true, nil, Vector3.new(4, 1, 4), CFrame.new(600, 60, 40) * Turn))
		local Result = Try(function() return A:Resize(Case[2], Case[3]) end)
		print("resize", Case[1], Result, V(A.Size), CF(A.CFrame))
		Clean()
	end
	local A = Keep(Part("A", true, nil, Vector3.new(4, 1, 4), CFrame.new(600, 60, 40)))
	local B = Keep(Part("B", true, nil, Vector3.new(4, 1, 4), CFrame.new(600, 62.5, 40)))
	A.CanCollide, B.CanCollide = true, true
	print("resize-blocked", Try(function() return A:Resize(Enum.NormalId.Top, 3) end), V(A.Size), Try(function() return A:Resize(Enum.NormalId.Top, 1) end), V(A.Size))
	Clean()
	local A = Keep(Part("A", false, "none", Vector3.new(4, 1, 4)))
	print("resize-out", Try(function() return A:resize(Enum.NormalId.Top, 3) end), V(A.Size))
	local Ball = Keep(Part("Ball", true, nil, Vector3.new(2, 2, 2)))
	Ball.Shape = Enum.PartType.Ball
	print("resize-ball", Try(function() return Ball:Resize(Enum.NormalId.Top, 2) end), V(Ball.Size))
	Clean()
end

-- the old Model methods
do
	local function Build(Primary)
		local M = Keep(Instance.new("Model"))
		M.Name = "M"
		local A = Part("A", true, M, Vector3.new(2, 4, 6), CFrame.new(700, 60, 40) * Turn)
		local B = Part("B", true, M, Vector3.new(1, 1, 1), CFrame.new(705, 62, 41))
		if Primary then
			M.PrimaryPart = A
		end
		M.Parent = workspace
		return M, A, B
	end
	for _, Primary in { true, false } do
		local M, A, B = Build(Primary)
		print("model", Primary, "cframe", Try(function() return M:GetModelCFrame() end), "size", Try(function() return M:GetModelSize() end), "pivot", CF(M:GetPivot()), "box", CF((M:GetBoundingBox())))
		M.WorldPivot = CFrame.new(1, 2, 3) * Turn
		print("model-pivot", Primary, "cframe", Try(function() return M:GetModelCFrame() end), "size", Try(function() return M:GetModelSize() end))
		print("model-identity", Primary, Try(function() M:ResetOrientationToIdentity() end), CF(A.CFrame), CF(B.CFrame), CF(M:GetPivot()))
		A.CFrame = CFrame.new(700, 60, 40) * Turn
		print("model-identity2", Primary, Try(function() M:SetIdentityOrientation() end), CF(A.CFrame), CF(B.CFrame), CF(M:GetPivot()))
		A.CFrame = CFrame.new(700, 60, 40) * Turn
		B.CFrame = CFrame.new(705, 62, 41)
		print("model-move", Primary, Try(function() M:move(Vector3.new(710, 70, 50)) end), CF(A.CFrame), CF(B.CFrame), CF(M:GetPivot()))
		A.CFrame = CFrame.new(700, 60, 40) * Turn
		B.CFrame = CFrame.new(705, 62, 41)
		print("model-moveto", Primary, Try(function() M:MoveTo(Vector3.new(710, 70, 50)) end), CF(A.CFrame), CF(B.CFrame), CF(M:GetPivot()))
		Clean()
	end
	local Empty = Keep(Instance.new("Model"))
	Empty.Parent = workspace
	print("model-empty", Try(function() return Empty:GetModelCFrame() end), Try(function() return Empty:GetModelSize() end), Try(function() Empty:ResetOrientationToIdentity() end), Try(function() Empty:move(Vector3.new(1, 2, 3)) end), CF(Empty:GetPivot()))
	local Loose = Keep(Instance.new("Model"))
	Part("A", true, Loose, Vector3.new(2, 2, 2), CFrame.new(5, 5, 5) * Turn)
	print("model-loose", Try(function() return Loose:GetModelCFrame() end), Try(function() return Loose:GetModelSize() end), Try(function() Loose:move(Vector3.new(1, 2, 3)) end), CF(Loose:GetPivot()))
	Clean()
	-- obstructed MoveTo / move
	local Floor = Keep(Part("Floor", true, nil, Vector3.new(20, 2, 20), CFrame.new(800, 60, 40)))
	Floor.CanCollide = true
	for _, Method in { "move", "MoveTo" } do
		local M = Keep(Instance.new("Model"))
		local A = Part("A", true, M, Vector3.new(2, 2, 2), CFrame.new(830, 60, 40))
		A.CanCollide = true
		M.Parent = workspace
		M[Method](M, Vector3.new(800, 60, 40))
		print("model-obstructed", Method, CF(A.CFrame))
	end
	Clean()
end

-- persistent players
do
	local M = Keep(Instance.new("Model"))
	M.Parent = workspace
	local Me = Players.LocalPlayer or Players:GetPlayers()[1]
	print("persistent", Try(function() return M:GetPersistentPlayers() end), Try(function() M:AddPersistentPlayer(Me) end), Try(function() return M:GetPersistentPlayers() end), Try(function() M:RemovePersistentPlayer(Me) end), Try(function() return M:GetPersistentPlayers() end), Try(function() M:AddPersistentPlayer() end))
	Clean()
end

-- FIX-R23: PivotTo of a turned model, its primary part's CFrame to the bit
do
	local function Build(Primary, Offset)
		local M = Keep(Instance.new("Model"))
		local A = Part("A", true, M, Vector3.new(2, 4, 6), CFrame.new(900.3, 60.7, 40.1) * Turn)
		local B = Part("B", true, M, Vector3.new(1, 1, 1), CFrame.new(905.1, 62.2, 41.9) * CFrame.Angles(0.1, 0.2, 0.3))
		if Offset then
			A.PivotOffset = CFrame.new(0.5, 0.25, -1) * CFrame.Angles(0.2, 0, 0.1)
		end
		if Primary then
			M.PrimaryPart = A
		end
		M.Parent = workspace
		return M, A, B
	end
	for _, Case in { { true, false }, { true, true }, { false, false } } do
		for Index, Target in { CFrame.new(10.1, 20.2, 30.3) * CFrame.Angles(1, 2, 3), CFrame.new(-7.77, 1e3, 0.001) * CFrame.fromAxisAngle(Vector3.new(1, 1, 0).Unit, 0.123), CFrame.new(1, 2, 3) } do
			local M, A, B = Build(Case[1], Case[2])
			M:PivotTo(Target)
			print("pivot", Case[1], Case[2], Index, CF(A.CFrame), CF(B.CFrame), CF(M:GetPivot()), M:GetPivot() == Target)
			M:PivotTo(Target)
			print("pivot-again", Case[1], Case[2], Index, CF(A.CFrame), CF(B.CFrame))
			Clean()
		end
	end
	local A = Keep(Part("A", true, nil, Vector3.new(2, 4, 6), CFrame.new(900.3, 60.7, 40.1) * Turn))
	local B = Part("B", true, A, Vector3.new(1, 1, 1), CFrame.new(905.1, 62.2, 41.9) * CFrame.Angles(0.1, 0.2, 0.3))
	A:PivotTo(CFrame.new(10.1, 20.2, 30.3) * CFrame.Angles(1, 2, 3))
	print("part-pivot", CF(A.CFrame), CF(B.CFrame))
	Clean()
end
workspace.Gravity = 196.2
