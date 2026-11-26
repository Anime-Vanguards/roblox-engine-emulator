-- BreakJoints / MakeJoints of a part and a model, JointInstance.Active, GetNoCollisionConstraints, Attachment GetAxis / SetAxis /
-- GetSecondaryAxis / SetSecondaryAxis / GetConstraints, Motor:SetDesiredAngle and a legacy Motor's CurrentAngle written (the
-- frames a motor turns by vary with a client's frame timing, so its turning is printed where it ends: part_rules.luau).
P.sync()
workspace.Gravity = 0
local JS = game:GetService("JointsService")
local RS = game:GetService("ReplicatedStorage")
local Spot = 0
local function Part(Name, Anchored, Parent, Size, At)
	Spot += 1
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = Size or Vector3.new(1, 1, 1)
	Made.Anchored = Anchored or false
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = At or CFrame.new(300 + 3 * Spot, 60, 40)
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
	if typeof(A) == "Instance" then
		return A.Name
	elseif type(A) == "table" then
		return Names(A)
	elseif type(A) == "number" then
		return P.f32(A)
	elseif typeof(A) == "Vector3" then
		return P.f32s(A.X, A.Y, A.Z)
	end
	return tostring(A)
end
local function State(Each)
	local Parent = Each.Parent
	local Ok = pcall(function()
		Each.Parent = Parent
	end)
	return Each.Name .. "@" .. (if Parent then Parent.Name else "nil") .. (if Ok then "" else "!locked")
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

-- BreakJoints of a part: which of its joints go, and how
do
	local A, B, C = Keep(Part("A"), Part("B"), Part("C"))
	local List = {
		Keep(Joint("Weld", A, B)),
		Keep(Joint("WeldConstraint", A, C)),
		Keep(Joint("Motor6D", B, A, B)),
		Keep(Joint("Snap", A, C, JS)),
		Keep(Joint("HingeConstraint", A, B)),
		Keep(Joint("NoCollisionConstraint", A, B)),
		Keep(Joint("Weld", B, C)),
		Keep(Joint("Rotate", A, B)),
		Keep(Joint("Weld", A, nil)),
	}
	local Off = Keep(Joint("Weld", A, C))
	Off.Enabled = false
	table.insert(List, Off)
	local Away = Keep(Joint("Weld", A, C, RS))
	Away.Name = "WeldInRS"
	table.insert(List, Away)
	local Gone = {}
	for _, Each in List do
		Each.AncestryChanged:Connect(function()
			table.insert(Gone, Each.Name)
		end)
	end
	A:BreakJoints()
	local Out = {}
	for _, Each in List do
		table.insert(Out, State(Each))
	end
	print("part-break", table.concat(Out, " "))
	task.wait()
	print("part-break events", table.concat(Gone, ","))
	Clean()
end

-- BreakJoints out of the world, and the lower-case alias
do
	local A, B = Keep(Part("A", false, "none"), Part("B", false, "none"))
	local W = Keep(Joint("Weld", A, B))
	A:BreakJoints()
	print("out-break", State(W))
	Clean()
	local A, B = Keep(Part("A"), Part("B"))
	local W = Keep(Joint("Weld", A, B))
	A:breakJoints()
	print("lower-break", State(W))
	Clean()
end

-- Model:BreakJoints
do
	local M = Keep(Instance.new("Model"))
	M.Name = "M"
	M.Parent = workspace
	local A, B, C = Keep(Part("A", false, M), Part("B", false, M), Part("C"))
	local List = { Keep(Joint("Weld", A, B, M)), Keep(Joint("WeldConstraint", B, C)), Keep(Joint("Weld", C, nil)), Keep(Joint("Motor6D", A, B, JS)) }
	M:BreakJoints()
	local Out = {}
	for _, Each in List do
		table.insert(Out, State(Each))
	end
	print("model-break", table.concat(Out, " "))
	Clean()
end

-- MakeJoints: surfaces, touching, and where the joints go
do
	local function Pair(SurfaceA, SurfaceB, Gap)
		local A = Part("A", false, nil, Vector3.new(4, 1, 4), CFrame.new(400, 60, 40))
		local B = Part("B", false, nil, Vector3.new(4, 1, 4), CFrame.new(400, 61 + (Gap or 0), 40))
		A.TopSurface = SurfaceA
		B.BottomSurface = SurfaceB
		return Keep(A, B)
	end
	for _, Case in {
		{ "smooth", Enum.SurfaceType.Smooth, Enum.SurfaceType.Smooth },
		{ "weld", Enum.SurfaceType.Weld, Enum.SurfaceType.Smooth },
		{ "studs-inlet", Enum.SurfaceType.Studs, Enum.SurfaceType.Inlet },
		{ "glue", Enum.SurfaceType.Glue, Enum.SurfaceType.Smooth },
		{ "universal", Enum.SurfaceType.Universal, Enum.SurfaceType.Smooth },
		{ "hinge", Enum.SurfaceType.Hinge, Enum.SurfaceType.Smooth },
		{ "motor", Enum.SurfaceType.Motor, Enum.SurfaceType.Smooth },
		{ "weld-gap", Enum.SurfaceType.Weld, Enum.SurfaceType.Smooth, 0.5 },
	} do
		local A, B = Pair(Case[2], Case[3], Case[4])
		local Before = #workspace:GetDescendants()
		local Ok, Error = pcall(function()
			A:MakeJoints()
		end)
		local Found = {}
		local All = workspace:GetDescendants()
		for _, Each in JS:GetDescendants() do
			table.insert(All, Each)
		end
		for _, Each in All do
			if (Each:IsA("JointInstance") or Each:IsA("WeldConstraint")) and (Each.Part0 == A or Each.Part1 == A or Each.Part0 == B or Each.Part1 == B) then
				Keep(Each)
				table.insert(Found, Each.ClassName .. ":" .. Each.Name .. "@" .. (if Each.Parent then Each.Parent.Name else "nil") .. " p0=" .. (if Each.Part0 then Each.Part0.Name else "nil") .. " p1=" .. (if Each.Part1 then Each.Part1.Name else "nil"))
			end
		end
		print("make", Case[1], Ok, if Ok then "" else P.err(Error), #workspace:GetDescendants() - Before, table.concat(Found, " "), "root", A.AssemblyRootPart and A.AssemblyRootPart.Name)
		Clean()
	end
	-- Model:MakeJoints
	local M = Keep(Instance.new("Model"))
	M.Parent = workspace
	local A = Keep(Part("A", false, M, Vector3.new(4, 1, 4), CFrame.new(400, 60, 40)))
	local B = Keep(Part("B", false, M, Vector3.new(4, 1, 4), CFrame.new(400, 61, 40)))
	A.TopSurface = Enum.SurfaceType.Weld
	print("model-make", Try(function() M:MakeJoints() end), "root", A.AssemblyRootPart.Name, B.AssemblyRootPart.Name)
	Clean()
	local A = Keep(Part("A", false, "none", Vector3.new(4, 1, 4), CFrame.new(400, 60, 40)))
	local B = Keep(Part("B", false, "none", Vector3.new(4, 1, 4), CFrame.new(400, 61, 40)))
	A.TopSurface = Enum.SurfaceType.Weld
	print("out-make", Try(function() A:MakeJoints() end), #A:GetChildren(), #B:GetChildren())
	Clean()
end

-- JointInstance.Active
do
	local A, B, C = Keep(Part("A"), Part("B"), Part("C", true))
	local W = Keep(Joint("Weld", A, B))
	print("active weld", W.Active)
	W.Enabled = false
	print("active disabled", W.Active)
	W.Enabled = true
	print("active enabled", W.Active)
	W.Part1 = nil
	print("active part1-nil", W.Active)
	W.Part1 = B
	W.Parent = RS
	print("active in-rs", W.Active)
	W.Parent = JS
	print("active in-js", W.Active)
	W.Parent = A
	local W2 = Keep(Joint("Weld", A, B))
	print("active second", W.Active, W2.Active)
	local R = Keep(Joint("Rotate", A, C))
	print("active rotate", R.Active)
	local M = Keep(Joint("Motor6D", C, A))
	print("active motor-to-anchored", M.Active)
	local T = Keep(Joint("Weld", C, Part("D", true)))
	print("active anchored-pair", T.Active)
	A.Parent = RS
	print("active part-out", W.Active, W2.Active, M.Active)
	local Loose = Instance.new("Weld")
	print("active loose", Loose.Active)
	Loose:Destroy()
	local G = Keep(Joint("Glue", B, C))
	print("active glue", G.Active)
	Clean()
end

-- GetNoCollisionConstraints
do
	local A, B, C = Keep(Part("A"), Part("B"), Part("C"))
	local N2 = Keep(Joint("NoCollisionConstraint", B, A))
	local N1 = Keep(Joint("NoCollisionConstraint", A, C))
	local N3 = Keep(Joint("NoCollisionConstraint", A, B, RS))
	local N4 = Keep(Joint("NoCollisionConstraint", A, B))
	N4.Enabled = false
	print("nocollide", Try(function() return A:GetNoCollisionConstraints() end), Try(function() return B:GetNoCollisionConstraints() end), Try(function() return C:GetNoCollisionConstraints() end))
	Clean()
end

-- Attachment axes and constraints
do
	local A, B = Keep(Part("A"), Part("B"))
	local Att = Instance.new("Attachment")
	Att.Name = "Att"
	Att.Parent = A
	Keep(Att)
	print("axis", Try(function() return Att:GetAxis() end), Try(function() return Att:GetSecondaryAxis() end))
	print("setaxis", Try(function() Att:SetAxis(Vector3.new(0, 1, 0)) end), Try(function() return Att:GetAxis() end), Try(function() return Att.Axis end), Try(function() return Att:GetSecondaryAxis() end))
	print("setsecondary", Try(function() Att:SetSecondaryAxis(Vector3.new(0, 0, 2)) end), Try(function() return Att:GetAxis() end), Try(function() return Att:GetSecondaryAxis() end), P.f32s(Att.CFrame:GetComponents()))
	print("setaxis-bad", Try(function() Att:SetAxis("x") end), Try(function() Att:SetAxis() end), Try(function() Att:SetAxis(Vector3.zero) end), P.f32s(Att.CFrame:GetComponents()))
	local Other = Instance.new("Attachment")
	Other.Name = "Other"
	Other.Parent = B
	Keep(Other)
	local H = Instance.new("HingeConstraint")
	H.Name = "H"
	H.Attachment0, H.Attachment1 = Att, Other
	H.Parent = A
	local Rope = Instance.new("RopeConstraint")
	Rope.Name = "Rope"
	Rope.Attachment1, Rope.Attachment0 = Att, Other
	Rope.Parent = workspace
	local Off = Instance.new("SpringConstraint")
	Off.Name = "Off"
	Off.Attachment0 = Att
	Off.Enabled = false
	Off.Parent = workspace
	local Out = Instance.new("BallSocketConstraint")
	Out.Name = "Out"
	Out.Attachment0 = Att
	Keep(H, Rope, Off, Out)
	local VF = Instance.new("VectorForce")
	VF.Name = "VF"
	VF.Attachment0 = Att
	VF.Parent = A
	Keep(VF)
	print("constraints", Try(function() return Att:GetConstraints() end), Try(function() return Other:GetConstraints() end))
	print("part-joints", Try(function() return A:GetJoints() end), Try(function() return B:GetJoints() end))
	Clean()
end

-- Motor: SetDesiredAngle, CurrentAngle over frames
do
	local Base = Keep(Part("Base", true, nil, Vector3.new(2, 2, 2), CFrame.new(500, 60, 40)))
	local Arm = Keep(Part("Arm", false, nil, Vector3.new(1, 1, 4), CFrame.new(500, 60, 43)))
	local M = Keep(Instance.new("Motor"))
	M.Part0, M.Part1 = Base, Arm
	M.C0 = CFrame.new(0, 0, 1.5)
	M.C1 = CFrame.new(0, 0, -1.5)
	M.Parent = Base
	print("motor start", P.f32(M.CurrentAngle), P.f32(M.DesiredAngle), P.f32(M.MaxVelocity))
	print("motor set", Try(function() M:SetDesiredAngle(1) end), P.f32(M.DesiredAngle), P.f32(M.CurrentAngle))
	for Frame = 1, 6 do
		task.wait()
		print("motor frame", Frame, P.f32(M.CurrentAngle), P.f32s(Arm.CFrame:GetComponents()))
	end
	M.MaxVelocity = 0.5
	M.DesiredAngle = -1
	for Frame = 1, 6 do
		task.wait()
	end
	M.CurrentAngle = 0.25
	print("motor write current", P.f32(M.CurrentAngle), P.f32s(Arm.CFrame:GetComponents()))
	task.wait()
	print("motor casts", Try(function() M:SetDesiredAngle("2") end), P.f32(M.DesiredAngle), Try(function() M:SetDesiredAngle() end), Try(function() M:SetDesiredAngle({}) end))
	M.MaxVelocity = 0
	M.DesiredAngle = 3
	task.wait()
	task.wait()
	M.MaxVelocity = 10
	M.DesiredAngle = 100
	task.wait()
	task.wait()
	-- a Motor6D's DesiredAngle
	local Arm2 = Keep(Part("Arm2", false, nil, Vector3.new(1, 1, 4), CFrame.new(510, 60, 43)))
	local Base2 = Keep(Part("Base2", true, nil, Vector3.new(2, 2, 2), CFrame.new(510, 60, 40)))
	local M6 = Keep(Instance.new("Motor6D"))
	M6.Part0, M6.Part1 = Base2, Arm2
	M6.C0 = CFrame.new(0, 0, 1.5)
	M6.C1 = CFrame.new(0, 0, -1.5)
	M6.Parent = Base2
	M6.MaxVelocity = 0.1
	M6.DesiredAngle = 1
	for Frame = 1, 3 do
		task.wait()
	end
	-- not in the world
	local M3 = Instance.new("Motor")
	M3.MaxVelocity = 0.1
	M3.DesiredAngle = 1
	task.wait()
	print("motor loose", P.f32(M3.CurrentAngle))
	M3:Destroy()
	Clean()
end
workspace.Gravity = 196.2
