-- The order of GetConnectedParts(true) (free trees, chords and rings, RootPriority, anchored components walked from each part) and
-- of a part's joint list (GetJoints, GetConnectedParts()) as joints of every kind come and go, move between places and parts leave
-- and come back.
P.sync()
workspace.Gravity = 0
local RS = game:GetService("ReplicatedStorage")
local JS = game:GetService("JointsService")
local Spot = 0
local function Part(Name, Anchored, Size)
	Spot += 1
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = Size or Vector3.new(1, 1, 1)
	Made.Anchored = Anchored or false
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = CFrame.new(300 + 3 * Spot, 60, 40)
	Made.Parent = workspace
	return Made
end
local function Joint(Kind, A, B, Parent)
	local Made = Instance.new(Kind)
	Made.Name = Kind == "Weld" and (A.Name .. B.Name) or (Kind .. "_" .. A.Name .. B.Name)
	if Made:IsA("JointInstance") then
		Made.Part0, Made.Part1 = A, B
		Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
	elseif Made:IsA("WeldConstraint") then
		Made.Part0, Made.Part1 = A, B
	else
		local A0 = Instance.new("Attachment")
		A0.Parent = A
		local A1 = Instance.new("Attachment")
		A1.Parent = B
		Made.Attachment0, Made.Attachment1 = A0, A1
	end
	if Parent ~= "none" then
		Made.Parent = Parent or A
	end
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
		Each:Destroy()
	end
	table.clear(All)
end
-- builds parts by name (a leading * anchors it, a trailing + makes it big) and welds from "AB CD ..." pairs, in that order
local function Build(PartsText, Edges)
	local By = {}
	for Name in string.gmatch(PartsText, "%S+") do
		local Anchored = string.sub(Name, 1, 1) == "*"
		local Big = string.sub(Name, -1) == "+"
		local Clean_ = string.gsub(Name, "[%*%+]", "")
		By[Clean_] = Part(Clean_, Anchored, if Big then Vector3.new(3, 3, 3) else nil)
		table.insert(All, By[Clean_])
	end
	for Pair in string.gmatch(Edges, "%S+") do
		Joint("Weld", By[string.sub(Pair, 1, 1)], By[string.sub(Pair, 2, 2)])
	end
	return By
end
local function Walks(Label, By, Order)
	for Name in string.gmatch(Order, "%S+") do
		local Each = By[Name]
		print(Label, Name, "root", Each.AssemblyRootPart.Name, "cpt", Names(Each:GetConnectedParts(true)), "cp", Names(Each:GetConnectedParts()))
	end
end

-- free trees and graphs (H the big root)
Walks("tree", Build("H+ A B C D E F", "HA HB AC AD BE EF"), "H A D F")
Clean()
Walks("chord", Build("H+ A B C D", "HA HB BC BD DA"), "H C")
Clean()
Walks("chord2", Build("H+ A B C D E", "HA HB BC BD DA AE"), "H E")
Clean()
Walks("ring6", Build("H+ A B C D E", "HA AB BC CD DE EH"), "H C")
Clean()
Walks("deep", Build("H+ A B C D E F G", "HA AB BC HD DE EF FG CG"), "H G")
Clean()
-- the root in the middle, made last, picked by RootPriority
do
	local By = Build("A B C D E", "AB BC CD DE")
	By.C.RootPriority = 5
	Walks("priority", By, "A C E")
	Clean()
	local By = Build("A B C D E", "AB BC CD DE")
	By.E.RootPriority = 5
	Walks("priority-end", By, "A C E")
	Clean()
end
-- anchored components
Walks("anch-tree", Build("*X A B C D", "XA XB AC BD"), "X A C D")
Clean()
Walks("anch-mid", Build("A B *X C D", "AB BX XC CD"), "A X D")
Clean()
Walks("anch-chord", Build("*H A B C D", "HA HB BC BD DA"), "H C A")
Clean()
Walks("anch-two", Build("*X A B *Y C", "XA AB BY YC XC"), "X A B Y C")
Clean()
Walks("anch-leaf", Build("H+ A B *C D", "HA HB BC CD"), "H A C D")
Clean()

-- GetJoints and GetConnectedParts(false) of a part as its joints of every kind come and go
do
	local A, B, C, D = Part("A"), Part("B"), Part("C"), Part("D")
	table.insert(All, A)
	table.insert(All, B)
	table.insert(All, C)
	table.insert(All, D)
	local J = {
		Joint("Weld", A, B),
		Joint("HingeConstraint", A, C),
		Joint("WeldConstraint", A, D),
		Joint("Rotate", A, B),
		Joint("Motor6D", C, A),
		Joint("RopeConstraint", D, A),
		Joint("Weld", A, C),
	}
	local function Show(Label)
		print("list", Label, "joints", Names(A:GetJoints()), "cp", Names(A:GetConnectedParts()), "c", Names(C:GetJoints()))
	end
	Show("made")
	J[1].Enabled = false
	Show("weld1-off")
	J[1].Enabled = true
	Show("weld1-on")
	J[2].Enabled = false
	Show("hinge-off")
	J[3].Enabled = false
	Show("wc-off")
	J[2].Enabled = true
	Show("hinge-on")
	J[3].Enabled = true
	Show("wc-on")
	J[4].Parent = RS
	Show("rotate-out")
	J[4].Parent = A
	Show("rotate-in")
	J[5].Part1 = B
	Show("motor-repointed")
	J[5].Part1 = A
	Show("motor-back")
	J[6].Attachment1.Parent = B
	Show("rope-att-moved")
	J[6].Attachment1.Parent = A
	Show("rope-att-back")
	J[7].Parent = JS
	Show("weld7-js")
	J[6].Parent = RS
	Show("rope-rs")
	J[6].Parent = JS
	Show("rope-js")
	J[6].Parent = nil
	Show("rope-nil")
	J[6].Parent = workspace
	Show("rope-ws")
	C.Parent = RS
	Show("c-out")
	C.Parent = workspace
	Show("c-in")
	local Same = Instance.new("HingeConstraint")
	Same.Name = "SameHinge"
	local S0, S1 = Instance.new("Attachment"), Instance.new("Attachment")
	S0.Parent, S1.Parent = A, A
	Same.Attachment0, Same.Attachment1 = S0, S1
	Same.Parent = A
	Show("same-part-hinge")
	local Nested = Instance.new("BallSocketConstraint")
	Nested.Name = "NestedBall"
	local Bone = Instance.new("Bone")
	Bone.Parent = A
	local N0 = Instance.new("Attachment")
	N0.Parent = Bone
	local N1 = Instance.new("Attachment")
	N1.Parent = B
	Nested.Attachment0, Nested.Attachment1 = N0, N1
	Nested.Parent = workspace
	Show("nested-attachment")
	A.Parent = RS
	A.Parent = workspace
	Show("a-out-in")
	Clean()
end
-- a model whose joints enter the world together
do
	local M = Instance.new("Model")
	local A = Instance.new("Part")
	A.Name = "A"
	A.Anchored = true
	A.Parent = M
	local B, C = Instance.new("Part"), Instance.new("Part")
	B.Name, C.Name = "B", "C"
	B.Parent, C.Parent = M, M
	local F = Instance.new("Folder")
	F.Parent = M
	local W1 = Joint("Weld", A, B, F)
	local H = Joint("HingeConstraint", A, C, M)
	local W2 = Joint("WeldConstraint", A, C, A)
	local W3 = Joint("Weld", B, A, B)
	M.Parent = workspace
	table.insert(All, M)
	print("model-in", Names(A:GetJoints()), Names(A:GetConnectedParts()), Names(A:GetConnectedParts(true)))
	Clean()
end
workspace.Gravity = 196.2
