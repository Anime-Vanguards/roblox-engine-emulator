-- BasePart:GetRootPart, GetConnectedParts(recursive) and GetJoints on a playing client and its server: lone parts in and out of
-- the world, pairs, chains, anchored chains and bridges, every joint and constraint kind, joints that do not link, WorldModels,
-- stars and rings, joints parented out of order, two joints between one pair, and the argument casts.
P.sync()
workspace.Gravity = 0
local JS = game:GetService("JointsService")
local RS = game:GetService("ReplicatedStorage")
local Spot = 0
local function Part(Name, Anchored, Parent, Size)
	Spot += 1
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = Size or Vector3.new(1, 1, 1)
	Made.Anchored = Anchored or false
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = CFrame.new(300 + 3 * Spot, 60, 40)
	if Parent ~= "none" then
		Made.Parent = Parent or workspace
	end
	return Made
end
local function Joint(Kind, A, B, Parent)
	local Made = Instance.new(Kind)
	Made.Name = Kind .. "_" .. (A and A.Name or "nil") .. "_" .. (B and B.Name or "nil")
	if Made:IsA("JointInstance") then
		Made.Part0, Made.Part1 = A, B
		if A and B then
			Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
		end
	elseif Made:IsA("WeldConstraint") or Made:IsA("NoCollisionConstraint") then
		Made.Part0, Made.Part1 = A, B
	else
		-- an attachment-based constraint
		local A0 = Instance.new("Attachment")
		A0.Name = "Att0_" .. Made.Name
		A0.Parent = A
		Made.Attachment0 = A0
		if B then
			local A1 = Instance.new("Attachment")
			A1.Name = "Att1_" .. Made.Name
			A1.Parent = B
			Made.Attachment1 = A1
		end
	end
	if Parent ~= "none" then
		Made.Parent = Parent or A
	end
	return Made
end
local function Names(List)
	if type(List) ~= "table" then
		return tostring(List)
	end
	local Out = {}
	for _, Each in List do
		table.insert(Out, if typeof(Each) == "Instance" then Each.Name else tostring(Each))
	end
	return "{" .. table.concat(Out, ",") .. "}"
end
local function Try(F, ...)
	local Ok, A = pcall(F, ...)
	if not Ok then
		return "error " .. P.err(A)
	end
	return if typeof(A) == "Instance" then A.Name elseif type(A) == "table" then Names(A) else tostring(A)
end
local function Show(Label, Parts)
	for _, Each in Parts do
		print(Label, Each.Name, "root", Try(function() return Each:GetRootPart() end), "arp", Try(function() return Each.AssemblyRootPart end),
			"cp", Try(function() return Each:GetConnectedParts() end),
			"cpf", Try(function() return Each:GetConnectedParts(false) end),
			"cpt", Try(function() return Each:GetConnectedParts(true) end),
			"joints", Try(function() return Each:GetJoints() end))
	end
end
local Made = {}
local function Clean()
	for _, Each in Made do
		Each:Destroy()
	end
	table.clear(Made)
end
local function Keep(...)
	for _, Each in { ... } do
		table.insert(Made, Each)
	end
	return ...
end

-- lone parts, in and out of the world
do
	local A = Keep(Part("A"))
	local B = Keep(Part("B", true))
	local C = Keep(Part("C", false, "none"))
	local D = Keep(Part("D", false, RS))
	Show("lone", { A, B, C, D })
	Clean()
end

-- a pair and a chain of welds
do
	local A, B = Keep(Part("A"), Part("B", false, nil, Vector3.new(2, 2, 2)))
	Keep(Joint("Weld", A, B))
	Show("pair", { A, B })
	Clean()
	local A, B, C, D = Keep(Part("A"), Part("B"), Part("C"), Part("D"))
	Keep(Joint("Weld", A, B), Joint("Weld", B, C), Joint("Weld", C, D))
	Show("chain", { A, B, C, D })
	Clean()
end

-- anchored parts
do
	local A, U, V = Keep(Part("A", true), Part("U"), Part("V"))
	Keep(Joint("Weld", A, U), Joint("Weld", U, V))
	Show("anchored-chain", { A, U, V })
	Clean()
	local A, B = Keep(Part("A", true), Part("B", true))
	Keep(Joint("Weld", A, B))
	Show("two-anchored", { A, B })
	Clean()
	local A, U, B, V = Keep(Part("A", true), Part("U"), Part("B", true), Part("V"))
	Keep(Joint("Weld", A, U), Joint("Weld", U, B), Joint("Weld", B, V))
	Show("anchored-bridge", { A, U, B, V })
	Clean()
end

-- each joint kind between two unanchored parts
for _, Kind in { "Weld", "Snap", "ManualWeld", "Glue", "ManualGlue", "Motor", "Motor6D", "WeldConstraint", "Rotate", "RotateP", "RotateV", "VelocityMotor" } do
	local A, B = Keep(Part("A"), Part("B"))
	Keep(Joint(Kind, A, B))
	Show("kind-" .. Kind, { A, B })
	Clean()
end

-- constraints
for _, Kind in { "HingeConstraint", "BallSocketConstraint", "RopeConstraint", "RodConstraint", "SpringConstraint", "PrismaticConstraint", "RigidConstraint", "NoCollisionConstraint", "AlignPosition", "AlignOrientation", "UniversalConstraint", "CylindricalConstraint", "PlaneConstraint", "TorsionSpringConstraint", "SlidingBallConstraint" } do
	local A, B = Keep(Part("A"), Part("B"))
	local Ok, Error = pcall(function()
		Keep(Joint(Kind, A, B))
	end)
	if not Ok then
		print("constraint", Kind, "error", P.err(Error))
	end
	Show("con-" .. Kind, { A, B })
	Clean()
end

-- a constraint with one attachment, and in a Folder away from its parts
do
	local A = Keep(Part("A"))
	Keep(Joint("VectorForce", A, nil))
	Show("one-attachment", { A })
	Clean()
	local A, B = Keep(Part("A"), Part("B"))
	local F = Keep(Instance.new("Folder"))
	F.Parent = workspace
	Keep(Joint("HingeConstraint", A, B, F), Joint("Weld", A, B, F))
	Show("in-folder", { A, B })
	Clean()
end

-- joints that do not link
do
	local A, B = Keep(Part("A"), Part("B"))
	local W = Keep(Joint("Weld", A, B))
	W.Enabled = false
	Show("disabled", { A, B })
	Clean()
	local A = Keep(Part("A"))
	Keep(Joint("Weld", A, nil))
	Show("part1-nil", { A })
	Clean()
	local A, B = Keep(Part("A"), Part("B"))
	Keep(Joint("Weld", A, B, RS))
	Show("weld-in-rs", { A, B })
	Clean()
	local A, B = Keep(Part("A"), Part("B"))
	Keep(Joint("Weld", A, B, JS))
	Show("weld-in-js", { A, B })
	Clean()
	local A, B = Keep(Part("A"), Part("B"))
	Keep(Joint("WeldConstraint", A, B, JS))
	Show("wc-in-js", { A, B })
	Clean()
	local A, B = Keep(Part("A", false, "none"), Part("B", false, "none"))
	Keep(Joint("Weld", A, B))
	Show("out-of-world", { A, B })
	Clean()
	local A, B = Keep(Part("A", false, RS), Part("B", false, RS))
	Keep(Joint("Weld", A, B))
	Show("in-rs", { A, B })
	Clean()
	local A, B = Keep(Part("A"), Part("B", false, RS))
	Keep(Joint("Weld", A, B, workspace))
	Show("worlds-apart", { A, B })
	Clean()
	local A, B = Keep(Part("A"), Part("B"))
	local H = Keep(Joint("HingeConstraint", A, B))
	H.Enabled = false
	Show("hinge-disabled", { A, B })
	Clean()
end

-- a WorldModel
do
	local WM = Keep(Instance.new("WorldModel"))
	WM.Parent = workspace
	local A, B, C = Keep(Part("A", false, WM), Part("B", false, WM), Part("C", true, WM))
	Keep(Joint("Weld", A, B), Joint("Weld", B, C))
	Show("worldmodel", { A, B, C })
	Clean()
	local WM = Keep(Instance.new("WorldModel"))
	WM.Parent = RS
	local A, B = Keep(Part("A", false, WM), Part("B", false, WM))
	Keep(Joint("Weld", A, B))
	Show("worldmodel-rs", { A, B })
	Clean()
end

-- order: a hub's leaves welded in one order, made in another; a mixed star
do
	local L3, L1, H, L2, L4 = Keep(Part("L3"), Part("L1"), Part("H", false, nil, Vector3.new(3, 3, 3)), Part("L2"), Part("L4"))
	Keep(Joint("Weld", H, L1), Joint("Weld", L2, H), Joint("WeldConstraint", H, L3), Joint("Motor6D", L4, H))
	Show("star", { H, L1, L2, L3, L4 })
	Clean()
	local H, A, B, C, D, E = Keep(Part("H", false, nil, Vector3.new(3, 3, 3)), Part("A"), Part("B"), Part("C"), Part("D"), Part("E"))
	Keep(Joint("Weld", H, A), Joint("Weld", A, B), Joint("Weld", H, C), Joint("Weld", C, D), Joint("Weld", D, E), Joint("Weld", B, E))
	Show("ring", { H, A, B, C, D, E })
	Clean()
	-- welds made in one order, parented in another
	local H, A, B, C = Keep(Part("H", false, nil, Vector3.new(3, 3, 3)), Part("A"), Part("B"), Part("C"))
	local W1, W2, W3 = Keep(Joint("Weld", H, A, "none"), Joint("Weld", H, B, "none"), Joint("Weld", H, C, "none"))
	W3.Parent = H
	W1.Parent = H
	W2.Parent = H
	Show("parent-order", { H, A, B, C })
	Clean()
	-- two welds between the same pair, plus a constraint
	local A, B = Keep(Part("A"), Part("B"))
	Keep(Joint("Weld", A, B), Joint("WeldConstraint", B, A), Joint("HingeConstraint", A, B))
	Show("double", { A, B })
	Clean()
end

-- argument casts and errors
do
	local A, B = Keep(Part("A"), Part("B"))
	Keep(Joint("Weld", A, B))
	for _, Arg in { "x", "", 0, 1, {}, A, Enum.Material.Plastic } do
		print("cast", typeof(Arg), Try(function() return A:GetConnectedParts(Arg) end))
	end
	print("dot-self", Try(function() return A.GetConnectedParts(workspace) end))
	print("dot-nil", Try(function() return A.GetConnectedParts(nil) end))
	print("dot-folder", Try(function() return A.GetRootPart(Instance.new("Folder")) end))
	print("dot-joints", Try(function() return A.GetJoints(game) end))
	print("extra", Try(function() return A:GetRootPart(1, 2) end), Try(function() return A:GetJoints("x") end))
	local L1, L2 = A:GetConnectedParts(true), A:GetConnectedParts(true)
	print("fresh-table", L1 ~= L2, table.isfrozen(L1), #L1)
	Clean()
end
workspace.Gravity = 196.2
