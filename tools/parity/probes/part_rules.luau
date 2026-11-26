-- JointInstance.Active on cycles, parallel joints and anchored parts, a primary-less model's default pivot, the identity
-- orientation methods, MoveTo / move over obstacles, a Motor6D's Transform with CurrentAngle, a legacy Motor turning to its
-- DesiredAngle, GetJoints with shared attachments.
P.sync()
workspace.Gravity = 0
local Spot = 0
local function Part(Name, Anchored, Size, At, Parent)
	Spot += 1
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = Size or Vector3.new(1, 1, 1)
	Made.Anchored = Anchored or false
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = At or CFrame.new(300 + 3 * Spot, 60, 40)
	Made.Parent = Parent or workspace
	return Made
end
local function Weld(A, B, Kind, Parent)
	local Made = Instance.new(Kind or "Weld")
	Made.Name = A.Name .. B.Name
	Made.Part0, Made.Part1 = A, B
	if Made:IsA("JointInstance") then
		Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
	end
	Made.Parent = Parent or A
	return Made
end
local function CF(C)
	return P.f32s(C:GetComponents())
end
local function V(A)
	return P.f32s(A.X, A.Y, A.Z)
end
local function Names(List)
	local Out = {}
	for _, Each in List do
		table.insert(Out, Each.Name)
	end
	return table.concat(Out, ",")
end
local All = {}
local function Keep(...)
	for _, Each in { ... } do
		table.insert(All, Each)
	end
	return ...
end
local function Clean()
	for _, Each in All do
		pcall(function() Each:Destroy() end)
	end
	table.clear(All)
end
local function Actives(Label, Joints)
	local Out = {}
	for _, Each in Joints do
		table.insert(Out, Each.Name .. "=" .. tostring(Each.Active))
	end
	print("active", Label, table.concat(Out, " "))
end

-- JointInstance.Active
do
	local H, A, B, C = Keep(Part("H", false, Vector3.new(3, 3, 3)), Part("A"), Part("B"), Part("C"))
	local J = { Weld(H, A), Weld(A, B), Weld(B, C), Weld(C, H) }
	Actives("ring", J)
	J[1].Enabled = false
	Actives("ring-cut", J)
	J[1].Enabled = true
	Actives("ring-back", J)
	Clean()
	local X, U, Y = Keep(Part("X", true), Part("U"), Part("Y", true))
	local J = { Weld(X, U), Weld(U, Y) }
	Actives("two-anchored", J)
	local J3 = Weld(U, Y)
	J3.Name = "UY2"
	table.insert(J, J3)
	Actives("two-anchored+", J)
	Clean()
	local A, B = Keep(Part("A"), Part("B"))
	local J = { Weld(A, B), Weld(A, B, "WeldConstraint"), Weld(B, A, "Motor6D"), Weld(A, B, "Snap") }
	J[2].Name = "WC"
	J[3].Name = "M6"
	J[4].Name = "Snap"
	Actives("parallel", J)
	J[4].Enabled = false
	Actives("parallel-cut", J)
	J[1].Parent = nil
	Actives("parallel-out", J)
	Clean()
	local A, B = Keep(Part("A", true), Part("B"))
	local W = Weld(A, B)
	W.Name = "AB"
	print("active anchored-unanchored", W.Active)
	B.Anchored = true
	print("active both-anchored", W.Active)
	A.Anchored = false
	print("active swapped", W.Active)
	Clean()
end

-- a primary-less model's pivot before anything sets it
do
	local Turn1, Turn2 = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7), CFrame.fromEulerAnglesXYZ(-0.2, 1.1, 0.4)
	for _, Case in {
		{ "small-first", { { Vector3.new(1, 1, 1), Turn1 }, { Vector3.new(4, 4, 4), Turn2 } } },
		{ "big-first", { { Vector3.new(4, 4, 4), Turn2 }, { Vector3.new(1, 1, 1), Turn1 } } },
		{ "folder-first", { "folder", { Vector3.new(1, 1, 1), Turn1 }, { Vector3.new(4, 4, 4), Turn2 } } },
		{ "nested-first", { "model", { Vector3.new(1, 1, 1), Turn1 }, { Vector3.new(4, 4, 4), Turn2 } } },
	} do
		local M = Keep(Instance.new("Model"))
		local Index = 0
		for _, Spec in Case[2] do
			Index += 1
			if Spec == "folder" or Spec == "model" then
				local Holder = Instance.new(if Spec == "folder" then "Folder" else "Model")
				Holder.Parent = M
				Part("In", false, Vector3.new(2, 1, 3), CFrame.new(800, 50, 40) * CFrame.Angles(0, 0, 1), Holder)
			else
				Part("P" .. Index, true, Spec[1], CFrame.new(800 + 3 * Index, 50, 40) * Spec[2], M)
			end
		end
		print("default-pivot", Case[1], "loose", CF(M:GetPivot()))
		M.Parent = workspace
		print("default-pivot", Case[1], "placed", CF(M:GetPivot()), "box", CF((M:GetBoundingBox())), V(M:GetExtentsSize()))
		local First = M:FindFirstChildWhichIsA("BasePart", true)
		First.CFrame = First.CFrame * CFrame.Angles(0, 0.5, 0)
		print("default-pivot", Case[1], "turned", CF(M:GetPivot()))
		Clean()
	end
	-- a model whose parts arrive after it is in the Workspace
	local M = Keep(Instance.new("Model"))
	M.Parent = workspace
	print("default-pivot", "empty-placed", CF(M:GetPivot()))
	Part("Q1", true, Vector3.new(1, 1, 1), CFrame.new(820, 50, 40) * Turn1, M)
	print("default-pivot", "one-arrived", CF(M:GetPivot()))
	Part("Q2", true, Vector3.new(4, 4, 4), CFrame.new(825, 50, 40) * Turn2, M)
	print("default-pivot", "two-arrived", CF(M:GetPivot()))
	M:FindFirstChild("Q1"):Destroy()
	print("default-pivot", "first-gone", CF(M:GetPivot()))
	Clean()
end

-- SetIdentityOrientation / ResetOrientationToIdentity
do
	local Turn = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7)
	for _, Primary in { true, false } do
		local M = Keep(Instance.new("Model"))
		local A = Part("A", true, Vector3.new(2, 4, 6), CFrame.new(700, 60, 40) * Turn, M)
		local B = Part("B", true, Vector3.new(1, 1, 1), CFrame.new(705, 62, 41), M)
		if Primary then
			M.PrimaryPart = A
		end
		M.Parent = workspace
		M:SetIdentityOrientation()
		M:PivotTo(M:GetPivot() * CFrame.Angles(0, 1, 0))
		print("identity", Primary, "turned", CF(A.CFrame), CF(B.CFrame))
		M:ResetOrientationToIdentity()
		print("identity", Primary, "reset", CF(A.CFrame), CF(B.CFrame), CF(M:GetPivot()))
		M:PivotTo(M:GetPivot() * CFrame.Angles(0.5, 0, 0))
		M:ResetOrientationToIdentity()
		print("identity", Primary, "reset2", CF(A.CFrame), CF(B.CFrame))
		Clean()
	end
end

-- MoveTo and move over obstacles
do
	local Floor = Keep(Part("Floor", true, Vector3.new(20, 2, 20), CFrame.new(900, 60, 40)))
	Floor.CanCollide = true
	local Off = Keep(Part("Off", true, Vector3.new(20, 2, 20), CFrame.new(950, 60, 40)))
	for _, Method in { "move", "MoveTo" } do
		for _, Case in {
			{ "into-floor", Vector3.new(900, 60, 40) },
			{ "above-floor", Vector3.new(900, 70, 40) },
			{ "touching", Vector3.new(900, 62, 40) },
			{ "off-floor-nocollide", Vector3.new(950, 60, 40) },
			{ "edge", Vector3.new(910.5, 60, 40) },
		} do
			local M = Keep(Instance.new("Model"))
			local A = Part("A", true, Vector3.new(2, 2, 2), CFrame.new(1000, 60, 40), M)
			local B = Part("B", true, Vector3.new(2, 2, 2), CFrame.new(1000, 66, 40), M)
			A.CanCollide = true
			B.CanCollide = Case[1] ~= "edge"
			M.Parent = workspace
			M[Method](M, Case[2])
			print("obstacle", Method, Case[1], CF(A.CFrame), V(B.Position), CF(M:GetPivot()))
			M:Destroy()
		end
	end
	local M = Keep(Instance.new("Model"))
	local A = Part("A", false, Vector3.new(2, 2, 2), CFrame.new(1000, 60, 40) * CFrame.Angles(0, 0, 0.5), M)
	A.CanCollide = true
	M.PrimaryPart = A
	M.Parent = workspace
	M:MoveTo(Vector3.new(900, 60, 40))
	print("obstacle", "turned-unanchored", CF(A.CFrame))
	Clean()
end

-- a Motor6D's Transform with CurrentAngle
do
	local Base = Keep(Part("Base", true, Vector3.new(2, 2, 2), CFrame.new(1100, 60, 40)))
	local Arm = Keep(Part("Arm", false, Vector3.new(1, 1, 4), CFrame.new(1100, 60, 43)))
	local M = Keep(Instance.new("Motor6D"))
	M.Part0, M.Part1 = Base, Arm
	M.C0 = CFrame.new(0, 0, 1.5) * CFrame.Angles(0.2, 0, 0)
	M.C1 = CFrame.new(0, 0, -1.5)
	M.Parent = Base
	M.CurrentAngle = 0.4
	print("m6d angle", CF(Arm.CFrame))
	M.Transform = CFrame.Angles(0.3, 0, 0)
	task.wait()
	print("m6d angle+transform", P.f32(M.CurrentAngle), CF(Arm.CFrame))
	M.CurrentAngle = 0
	print("m6d transform", CF(Arm.CFrame))
	M.C1 = CFrame.new(0, 0, -1.5)
	print("m6d c1-write", CF(Arm.CFrame))
	Clean()
	-- a Motor's CurrentAngle with Part0 free and Part1 anchored, and its DesiredAngle reached exactly
	local Free = Keep(Part("Free", false, Vector3.new(2, 2, 2), CFrame.new(1110, 60, 40)))
	local Fixed = Keep(Part("Fixed", true, Vector3.new(1, 1, 4), CFrame.new(1110, 60, 43)))
	local M2 = Keep(Instance.new("Motor"))
	M2.Part0, M2.Part1 = Free, Fixed
	M2.C0 = CFrame.new(0, 0, 1.5)
	M2.C1 = CFrame.new(0, 0, -1.5)
	M2.Parent = Free
	M2.CurrentAngle = 0.7
	print("motor reverse", CF(Free.CFrame), CF(Fixed.CFrame))
	M2.MaxVelocity = 0.05
	M2.DesiredAngle = 0.8
	for _ = 1, 8 do
		task.wait()
	end
	print("motor reached", P.f32(M2.CurrentAngle), CF(Free.CFrame))
	M2.DesiredAngle = 0.81
	task.wait()
	task.wait()
	print("motor small step", P.f32(M2.CurrentAngle))
	M2.MaxVelocity = -0.1
	M2.DesiredAngle = 0
	task.wait()
	task.wait()
	M2.MaxVelocity = 0.1
	M2.DesiredAngle = 0 / 0
	task.wait()
	Clean()
end

-- GetJoints with attachments shared between constraints
do
	local A, B = Keep(Part("A"), Part("B"))
	local Att, Other = Instance.new("Attachment"), Instance.new("Attachment")
	Att.Parent, Other.Parent = A, B
	local function Con(Kind, Name, A0, A1, Parent)
		local Made = Instance.new(Kind)
		Made.Name = Name
		Made.Attachment0, Made.Attachment1 = A0, A1
		Made.Parent = Parent
		return Made
	end
	Con("HingeConstraint", "H1", Att, Other, A)
	print("shared", "1", Names(A:GetJoints()), Names(B:GetJoints()))
	Con("RopeConstraint", "R2", Other, Att, workspace)
	print("shared", "2", Names(A:GetJoints()), Names(B:GetJoints()))
	Con("SpringConstraint", "S3", Att, Other, workspace)
	print("shared", "3", Names(A:GetJoints()), Names(B:GetJoints()))
	Con("VectorForce", "V4", Att, nil, A)
	print("shared", "4", Names(A:GetJoints()), Names(B:GetJoints()))
	Con("AlignOrientation", "O5", Other, nil, B)
	print("shared", "5", Names(A:GetJoints()), Names(B:GetJoints()))
	local W = Weld(A, B)
	print("shared", "6", Names(A:GetJoints()), Names(B:GetJoints()))
	Con("Torque", "T7", Att, nil, A)
	print("shared", "7", Names(A:GetJoints()), Names(B:GetJoints()), Names(Att:GetConstraints()), Names(Other:GetConstraints()))
	Clean()
end
workspace.Gravity = 196.2
