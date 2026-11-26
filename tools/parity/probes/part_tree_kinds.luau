-- A free assembly's tree with joint kinds mixed (Motor6D, WeldConstraint, Snap, Glue), the default pivot's part, a Motor6D's
-- Transform back to identity, one-attachment constraints in a part's joint list.
P.sync()
workspace.Gravity = 0
local Spot = 0
local function Part(Name, Size, Parent)
	Spot += 1
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = Size or Vector3.new(1, 1, 1)
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = CFrame.new(300 + 4 * Spot, 60, 40)
	Made.Parent = Parent or workspace
	return Made
end
local function Names(List)
	local Out = {}
	for _, Each in List do
		table.insert(Out, Each.Name)
	end
	return table.concat(Out, ",")
end
local All = {}
local function Clean()
	for _, Each in All do
		pcall(function() Each:Destroy() end)
	end
	table.clear(All)
end
-- parts "Name[:size][!priority]" made in order; joints "XY[:Kind]" made in order (X Part0)
local function Case(Label, PartsText, Edges)
	local By, Joints = {}, {}
	for Spec in string.gmatch(PartsText, "%S+") do
		local Name, Size, Priority = string.match(Spec, "^(%a+):?(%d*)!?(%d*)$")
		local Side = tonumber(Size) or 1
		By[Name] = Part(Name, Vector3.new(Side, Side, Side))
		if Priority ~= "" then
			By[Name].RootPriority = tonumber(Priority)
		end
		table.insert(All, By[Name])
	end
	for Spec in string.gmatch(Edges, "%S+") do
		local Pair, Kind = string.match(Spec, "^(%a%a):?(%w*)$")
		local A, B = By[string.sub(Pair, 1, 1)], By[string.sub(Pair, 2, 2)]
		local W = Instance.new(if Kind ~= "" then Kind else "Weld")
		W.Name = Pair
		W.Part0, W.Part1 = A, B
		if W:IsA("JointInstance") then
			W.C0 = A.CFrame:ToObjectSpace(B.CFrame)
		end
		W.Parent = A
		table.insert(Joints, W)
	end
	local Root = nil
	for _, Each in By do
		Root = Each.AssemblyRootPart
		break
	end
	local Off = {}
	for _, W in Joints do
		if W:IsA("JointInstance") and not W.Active then
			table.insert(Off, W.Name)
		end
	end
	print("tree", Label, "root", Root.Name, "cpt", Names(Root:GetConnectedParts(true)), "inactive", table.concat(Off, ","))
	Clean()
end
Case("kind-tri", "H:3 A B", "HA:Motor6D HB AB")
Case("kind-tri2", "H:3 A B", "HA HB:Motor6D AB")
Case("kind-tri3", "H:3 A B", "HA HB AB:Motor6D")
Case("kind-tri4", "H:3 A B", "HA:Motor6D HB:Motor6D AB")
Case("kind-sq", "H:3 A X B", "HA HB AX:Motor6D BX")
Case("kind-sq2", "H:3 A X B", "HA HB AX BX:Motor6D")
Case("kind-wc", "H:3 A X B", "HA HB AX:WeldConstraint BX")
Case("kind-wc2", "H:3 A X B", "HA HB AX BX:WeldConstraint")
Case("kind-root", "H:3 A X B", "HA:Motor6D HB:Motor6D AX BX")
Case("kind-root2", "H:3 A X B", "HA HB:Motor6D AX:Motor6D BX:Motor6D")
Case("kind-big", "H:3 A X:2 B", "HA HB AX:Motor6D BX")
Case("kind-snap", "H:3 A B", "HA:Snap HB:Glue AB:ManualWeld")

-- the default pivot's part: by volume, by largest face, by the root's rule?
do
	local Turn1, Turn2 = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7), CFrame.fromEulerAnglesXYZ(-0.2, 1.1, 0.4)
	local function Pivot(Label, Specs)
		local M = Instance.new("Model")
		for Index, Spec in Specs do
			local P = Instance.new(Spec.Class or "Part")
			P.Name = "P" .. Index
			P.Size = Spec.Size
			P.Anchored = true
			P.CFrame = CFrame.new(800 + 6 * Index, 50, 40) * (if Index == 1 then Turn1 else Turn2)
			if Spec.Name then
				P.Name = Spec.Name
			end
			if Spec.Massless then
				P.Massless = true
			end
			P.Parent = M
		end
		M.Parent = workspace
		local R = M:GetPivot().Rotation
		print("pivot-part", Label, if R == Turn1.Rotation then "P1" elseif R == Turn2.Rotation then "P2" else P.f32s(R:GetComponents()))
		M:Destroy()
	end
	Pivot("volume-vs-face", { { Size = Vector3.new(10, 10, 0.2) }, { Size = Vector3.new(3, 3, 3) } })
	Pivot("face-vs-volume", { { Size = Vector3.new(3, 3, 3) }, { Size = Vector3.new(10, 10, 0.2) } })
	Pivot("equal", { { Size = Vector3.new(2, 2, 2) }, { Size = Vector3.new(2, 2, 2) } })
	Pivot("equal-rev", { { Size = Vector3.new(2, 2, 2) }, { Size = Vector3.new(2, 2, 2.0001) } })
	Pivot("long-thin", { { Size = Vector3.new(20, 0.5, 0.5) }, { Size = Vector3.new(2, 2, 2) } })
	Pivot("hrp", { { Size = Vector3.new(1, 1, 1), Name = "HumanoidRootPart" }, { Size = Vector3.new(2, 2, 2) } })
	Pivot("seat", { { Size = Vector3.new(2, 1, 2), Class = "Seat" }, { Size = Vector3.new(2, 2, 2) } })
	Pivot("massless", { { Size = Vector3.new(4, 4, 4), Massless = true }, { Size = Vector3.new(2, 2, 2) } })
	Pivot("ball", { { Size = Vector3.new(3, 3, 3), Class = "Part" }, { Size = Vector3.new(2, 2, 2) } })
end

-- a Motor6D's Transform back to identity, with CurrentAngle set
do
	local Base = Part("Base", Vector3.new(2, 2, 2))
	Base.Anchored = true
	local Arm = Part("Arm", Vector3.new(1, 1, 4))
	table.insert(All, Base)
	table.insert(All, Arm)
	local M = Instance.new("Motor6D")
	M.Part0, M.Part1 = Base, Arm
	M.C0 = CFrame.new(0, 0, 1.5)
	M.C1 = CFrame.new(0, 0, -1.5)
	M.Parent = Base
	M.CurrentAngle = 0.4
	print("m6d start", P.f32s(Arm.CFrame:GetComponents()))
	M.Transform = CFrame.Angles(0.3, 0, 0)
	task.wait()
	print("m6d transform", P.f32s(Arm.CFrame:GetComponents()))
	M.Transform = CFrame.new()
	task.wait()
	M.CurrentAngle = 0.6
	print("m6d angle-after", P.f32s(Arm.CFrame:GetComponents()))
	task.wait()
	print("m6d angle-after-frame", P.f32s(Arm.CFrame:GetComponents()))
	M.DesiredAngle = 0.6
	M.MaxVelocity = 1
	M.Transform = CFrame.Angles(0, 0.2, 0)
	task.wait()
	task.wait()
	print("m6d transform-with-desired", P.f32(M.CurrentAngle), P.f32s(Arm.CFrame:GetComponents()))
	Clean()
end

-- one-attachment constraints leaving a part's list
do
	local A, B = Part("A"), Part("B")
	table.insert(All, A)
	table.insert(All, B)
	local Att, Other = Instance.new("Attachment"), Instance.new("Attachment")
	Att.Parent, Other.Parent = A, B
	local function Con(Kind, Name, A0, A1)
		local Made = Instance.new(Kind)
		Made.Name = Name
		Made.Attachment0, Made.Attachment1 = A0, A1
		Made.Parent = A
		return Made
	end
	local H1 = Con("HingeConstraint", "H1", Att, Other)
	local V2 = Con("VectorForce", "V2", Att, nil)
	local R3 = Con("RopeConstraint", "R3", Att, Other)
	local T4 = Con("Torque", "T4", Att, nil)
	local S5 = Con("SpringConstraint", "S5", Att, Other)
	print("single", "made", Names(A:GetJoints()))
	V2.Enabled = false
	print("single", "v2-off", Names(A:GetJoints()))
	V2.Enabled = true
	print("single", "v2-on", Names(A:GetJoints()))
	H1.Enabled = false
	print("single", "h1-off", Names(A:GetJoints()))
	local W = Instance.new("Weld")
	W.Name = "W6"
	W.Part0, W.Part1 = A, B
	W.Parent = A
	print("single", "weld", Names(A:GetJoints()), Names(A:GetConnectedParts()))
	local Hinge2 = Con("HingeConstraint", "H7", Att, nil)
	print("single", "hinge-one", Names(A:GetJoints()))
	Clean()
end
workspace.Gravity = 196.2
