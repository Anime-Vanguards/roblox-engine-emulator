-- The animation classes' other members: the providers' register and web members in a running game, the members only
-- Roblox's scripts or plugins may call, Animator:GetTrackByAnimationId / ApplyJointVelocities /
-- RegisterEvaluationParallelCallback, a track's parameters and targets, AnimationNodeDefinition's input pins, IKControl's chain
-- reads, and HumanoidRigDescription's and DigitsRigDescription's methods.
P.sync()
local function Show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return "number " .. P.num(Value)
	elseif Kind == "table" then
		local Parts = {}
		for Key, Item in Value do
			table.insert(Parts, tostring(Key) .. "=" .. (if type(Item) == "table" then "table" else typeof(Item) .. ":" .. tostring(Item)))
		end
		table.sort(Parts)
		return "table#" .. #Value .. "{" .. table.concat(Parts, ",") .. "}"
	elseif Kind == "Instance" then
		return "Instance " .. Value.ClassName .. " " .. Value.Name
	end
	return Kind .. " " .. tostring(Value)
end
local function Try(Label, F, ...)
	local Results = table.pack(pcall(F, ...))
	if not Results[1] then
		print(Label, "error", P.err(Results[2]))
		return
	end
	local Parts = {}
	for Index = 2, Results.n do
		table.insert(Parts, Show(Results[Index]))
	end
	print(Label, "ok", Results.n - 1, table.concat(Parts, " ; "))
end
-- (a call that may wait: its outcome and whether it came within the frame)
local function Waited(Label, F)
	local Done, Frames, Out = false, 0, nil
	task.spawn(function()
		Out = P.try(F)
		Done = true
	end)
	while not Done and Frames < 300 do
		task.wait()
		Frames += 1
	end
	print(Label, if Done then Out else "no answer", if Frames == 0 then "at once" else "later")
end
-- (a call where the thread cannot yield: a table.sort comparator)
local function Sorted(Label, F)
	local Line
	table.sort({ 2, 1 }, function(A, B)
		if not Line then
			Line = P.try(F)
		end
		return A < B
	end)
	print("refused " .. Label .. " " .. Line)
end
local KSP = game:GetService("KeyframeSequenceProvider")
local ACP = game:GetService("AnimationClipProvider")

-- registering in a running game
local KS = Instance.new("KeyframeSequence")
Instance.new("Keyframe").Parent = KS
Try("RegisterKeyframeSequence", function() return KSP:RegisterKeyframeSequence(KS) end)
Try("RegisterKeyframeSequence nil", function() return KSP:RegisterKeyframeSequence(nil) end)
Try("RegisterActiveKeyframeSequence", function() return KSP:RegisterActiveKeyframeSequence(KS) end)
Try("RegisterAnimationClip", function() return ACP:RegisterAnimationClip(KS) end)
Try("RegisterActiveAnimationClip", function() return ACP:RegisterActiveAnimationClip(KS) end)

-- members a game script may not call (with no arguments, on a fresh instance)
local Locked = {
	{ "KeyframeSequenceProvider", { "GetKeyframeSequence", "GetKeyframeSequenceById", "GetMemStats" } },
	{ "AnimationClipProvider", { "GetAnimationClip", "GetAnimationClipById", "GetAnimationNodeDefinition", "GetAnimationNodeTypes", "GetAnimationValueNodeDefinition", "GetAnimationValueNodeTypes", "GetMemStats", "ReloadAnimationClipAsync" } },
}
for _, Entry in Locked do
	local Service = game:GetService(Entry[1])
	for _, Name in Entry[2] do
		Try(Entry[1] .. "." .. Name, function() return Service[Name](Service) end)
	end
end
local LockedInstances = {
	{ "Animator", { "GetPlayingAnimationTracksCoreScript", "LoadAnimationCoreScript", "LoadStreamAnimation", "LoadStreamAnimationForSelfieView_deprecated", "LoadStreamAnimationV2", "StepAnimationsInternal", "SynchronizeWith" } },
	{ "IKControl", { "Solve" } },
	{ "HumanoidRigDescription", { "AutoRig", "GetContainedJointLabels", "GetJoint", "GetJointLabels", "GetJointRangeMax", "GetJointRangeMin", "GetJointSize", "GetR15JointLabels", "GetR6JointLabels", "GetTposeAdjustment", "SetJoint", "SetJointRangeMax", "SetJointRangeMin", "SetJointSize", "SetTposeAdjustment", "ShowVolumes" } },
	{ "DigitsRigDescription", { "GetJoint", "GetJointLabels", "GetTposeAdjustment", "SetJoint", "SetTposeAdjustment" } },
}
for _, Entry in LockedInstances do
	local Made = Instance.new(Entry[1])
	for _, Name in Entry[2] do
		Try(Entry[1] .. "." .. Name, function() return Made[Name](Made) end)
	end
end

-- the web members
local Player = game:GetService("Players").LocalPlayer
local UserId = if Player then Player.UserId else 1
for _, Provider in { KSP, ACP } do
	local Name = Provider.ClassName
	Waited(Name .. " GetAnimationsAsync", function() return Provider:GetAnimationsAsync(UserId) end)
	Waited(Name .. " GetAnimations", function() return Provider:GetAnimations(UserId) end)
	Waited(Name .. " GetAnimationsAsync number string", function() return Provider:GetAnimationsAsync("1") end)
	Waited(Name .. " GetAnimationsAsync bool", function() return Provider:GetAnimationsAsync(true) end)
	Try(Name .. " GetAnimationsAsync nil", function() return Provider:GetAnimationsAsync(nil) end)
	Try(Name .. " GetAnimationsAsync none", function() return Provider:GetAnimationsAsync() end)
	Try(Name .. " GetAnimationsAsync string", function() return Provider:GetAnimationsAsync("x") end)
	Try(Name .. " GetAnimationsAsync table", function() return Provider:GetAnimationsAsync({}) end)
	Try(Name .. " GetAnimationsAsync enum", function() return Provider:GetAnimationsAsync(Enum.Material.Plastic) end)
	Try(Name .. " GetAnimations string", function() return Provider:GetAnimations("x") end)
	Sorted(Name .. " GetAnimationsAsync", function() return Provider:GetAnimationsAsync(1) end)
	Sorted(Name .. " GetAnimationsAsync cast", function() return Provider:GetAnimationsAsync("x") end)
end
for _, Id in { "rbxassetid://1", "rbxassetid://0", "", "x" } do
	Waited("GetClipEvaluatorAsync " .. Id, function() return ACP:GetClipEvaluatorAsync(Id) end)
end
Try("GetClipEvaluatorAsync nil", function() return ACP:GetClipEvaluatorAsync(nil) end)
Try("GetClipEvaluatorAsync number", function() return ACP:GetClipEvaluatorAsync(1) end)
Sorted("GetClipEvaluatorAsync", function() return ACP:GetClipEvaluatorAsync("rbxassetid://1") end)

-- an Animator (its rig outside the DataModel: a server fetches no rbxassetid id of a rig in it)
local Rig = Instance.new("Model")
local Base = Instance.new("Part")
Base.Name = "Base"
Base.Parent = Rig
local Limb = Instance.new("Part")
Limb.Name = "Limb"
Limb.Parent = Rig
local Joint = Instance.new("Motor6D")
Joint.Part0, Joint.Part1 = Base, Limb
Joint.Parent = Base
local Controller = Instance.new("AnimationController")
Controller.Parent = Rig
local Animator = Instance.new("Animator")
Animator.Parent = Controller
print("animator props", Show(Animator.RootMotion), Show(Animator.RootMotionWeight), Show(Animator.EvaluationThrottled), Show(Animator.PreferLodEnabled))
Try("write RootMotion", function() Animator.RootMotion = CFrame.new(1, 0, 0) end)
Try("GetTrackByAnimationId none", function() return Animator:GetTrackByAnimationId("rbxassetid://0") end)
Try("GetTrackByAnimationId nil", function() return Animator:GetTrackByAnimationId(nil) end)
Try("GetTrackByAnimationId number", function() return Animator:GetTrackByAnimationId(0) end)
Try("GetTrackByAnimationId empty", function() return Animator:GetTrackByAnimationId("") end)
local function Load(Name)
	local Animation = Instance.new("Animation")
	Animation.Name = Name
	Animation.AnimationId = "rbxassetid://0"
	return Animator:LoadAnimation(Animation), Animation
end
local A1, Animation1 = Load("A1")
local A2 = Load("A2")
local function Found(Label, Id)
	Try(Label, function() local Track = Animator:GetTrackByAnimationId(Id) return Track == A1, Track == A2 end)
end
Found("loaded", "rbxassetid://0")
A2:Play(0)
A1:Play(0)
Found("playing", "rbxassetid://0")
Found("other spelling", "rbxassetid://00")
Found("spaced", " rbxassetid://0")
Animation1.AnimationId = "rbxassetid://00"
Found("changed id, old", "rbxassetid://0")
Found("changed id, new", "rbxassetid://00")
A2:Stop(0.5)
Found("fading", "rbxassetid://0")
task.wait()
Found("fading later", "rbxassetid://0")
A1:Stop(0)
A2:Stop(0)
task.wait()
task.wait()
Found("stopped", "rbxassetid://0")
Found("stopped, new", "rbxassetid://00")
-- (the rig is outside the DataModel)
print("outside tracks", A1.IsPlaying, P.f32(A1.TimePosition), P.f32(A1.WeightCurrent), A2.IsPlaying, P.f32(A2.TimePosition), P.f32(A2.WeightCurrent))
Try("ApplyJointVelocities empty", function() return Animator:ApplyJointVelocities({}) end)
Try("ApplyJointVelocities motors", function() return Animator:ApplyJointVelocities({ Joint, Joint }) end)
Try("ApplyJointVelocities nil", function() return Animator:ApplyJointVelocities(nil) end)
Try("ApplyJointVelocities none", function() return Animator:ApplyJointVelocities() end)
Try("ApplyJointVelocities number", function() return Animator:ApplyJointVelocities(5) end)
Try("ApplyJointVelocities string", function() return Animator:ApplyJointVelocities("x") end)
Try("ApplyJointVelocities dictionary", function() return Animator:ApplyJointVelocities({ a = Joint }) end)
Try("ApplyJointVelocities hole", function() return Animator:ApplyJointVelocities({ [2] = Joint }) end)
Try("ApplyJointVelocities mixed", function() return Animator:ApplyJointVelocities({ Joint, 5 }) end)
Try("ApplyJointVelocities strings", function() return Animator:ApplyJointVelocities({ "x" }) end)
Try("ApplyJointVelocities parts", function() return Animator:ApplyJointVelocities({ Base }) end)
Try("RegisterEvaluationParallelCallback", function() return Animator:RegisterEvaluationParallelCallback(function() end) end)
Try("RegisterEvaluationParallelCallback nil", function() return Animator:RegisterEvaluationParallelCallback(nil) end)
Try("RegisterEvaluationParallelCallback number", function() return Animator:RegisterEvaluationParallelCallback(1) end)
Try("RegisterEvaluationParallelCallback string", function() return Animator:RegisterEvaluationParallelCallback("x") end)
Try("RegisterEvaluationParallelCallback table", function() return Animator:RegisterEvaluationParallelCallback({}) end)

-- a track's parameters and targets
Try("GetParameter unset", function() return A1:GetParameter("x") end)
Try("GetParameter nil", function() return A1:GetParameter(nil) end)
Try("SetParameter", function() return A1:SetParameter("x", 1) end)
Try("GetParameter set", function() return A1:GetParameter("x") end)
Try("SetParameter no value", function() return A1:SetParameter("p") end)
Try("SetParameter nil value", function() return A1:SetParameter("p", nil) end)
Try("SetParameter nil key", function() return A1:SetParameter(nil, 1) end)
Try("SetParameter number key", function() A1:SetParameter(5, "five") return A1:GetParameter("5"), A1:GetParameter(5) end)
local Kinds = {
	{ "Vector3", Vector3.new(1, 2, 3) }, { "CFrame", CFrame.new(1, 2, 3) }, { "Vector2", Vector2.new(1, 2) }, { "Color3", Color3.new(1, 0, 0) },
	{ "UDim2", UDim2.new(1, 2, 3, 4) }, { "EnumItem", Enum.Material.Wood }, { "int", 7 }, { "float", 0.1 }, { "string", "s" }, { "bool", false },
	{ "NumberRange", NumberRange.new(1, 2) }, { "BrickColor", BrickColor.new("Bright red") }, { "nan", 0 / 0 }, { "Vector3int16", Vector3int16.new(1, 2, 3) },
	{ "buffer", buffer.create(2) }, { "table", { 1 } }, { "Instance", Base }, { "function", print },
}
for _, Case in Kinds do
	local Done, Why = pcall(function() A1:SetParameter("k", Case[2]) end)
	print("parameter", Case[1], if Done then Show(A1:GetParameter("k")) else "error " .. P.err(Why))
end
Try("parameter of a new key, table", function() A1:SetParameter("t", {}) return A1:GetParameter("t") end)
Try("parameter of another track", function() return A2:GetParameter("x") end)
Try("GetParameterDefaults", function() return A1:GetParameterDefaults() end)
Try("GetParameterDefaults twice", function() return A1:GetParameterDefaults() == A1:GetParameterDefaults() end)
Try("GetTargetNames", function() return A1:GetTargetNames() end)
Try("GetTargetInstance", function() return A1:GetTargetInstance("x") end)
Try("SetTargetInstance", function() return A1:SetTargetInstance("x", Base) end)
Try("SetTargetInstance nil", function() return A1:SetTargetInstance("x", nil) end)

-- AnimationNodeDefinition's input pins, a call at a time with its events
local Node = Instance.new("AnimationNodeDefinition")
print("node props", Show(Node.NodeId), Show(Node.NodeType))
Try("NodeId write", function() Node.NodeId = 5 return Node.NodeId end)
Try("NodeType write", function() Node.NodeType = "ClipNode" return Node.NodeType end)
Try("InputPinData", function() return Node.InputPinData end)
local Fired = 0
Node.InputPinsChanged:Connect(function(...) Fired += 1 + select("#", ...) * 100 end)
local function Pins(Label, F)
	local Before = Fired
	local Line = P.try(F)
	task.wait()
	print("pins", Label, Line, Show(Node:GetOrderedInputPinNames()), "events", Fired - Before)
end
Pins("new", function() end)
Pins("add a", function() Node:AddInputPin("a") end)
Pins("add a again", function() Node:AddInputPin("a") end)
Pins("add b", function() Node:AddInputPin("b") end)
Pins("add number", function() Node:AddInputPin(5) end)
Pins("add fraction", function() Node:AddInputPin(1.5) end)
Pins("add empty", function() Node:AddInputPin("") end)
Pins("add nil", function() Node:AddInputPin(nil) end)
Pins("remove a", function() Node:RemoveInputPin("a") end)
Pins("remove absent", function() Node:RemoveInputPin("zz") end)
Pins("remove nil", function() Node:RemoveInputPin(nil) end)
Pins("remove number", function() Node:RemoveInputPin(1.5) end)
Pins("set", function() Node:SetOrderedInputPinNames({ "c", "d" }) end)
Pins("set mixed", function() Node:SetOrderedInputPinNames({ "x", 1 }) end)
Pins("set numbers", function() Node:SetOrderedInputPinNames({ 1, 2 }) end)
Pins("set itself", function() Node:SetOrderedInputPinNames(Node:GetOrderedInputPinNames()) end)
Pins("set twice the same", function() Node:SetOrderedInputPinNames({ "q", "q" }) end)
Pins("set empty", function() Node:SetOrderedInputPinNames({}) end)
Pins("set nil", function() Node:SetOrderedInputPinNames(nil) end)
Pins("set number", function() Node:SetOrderedInputPinNames(5) end)
Pins("set dictionary", function() Node:SetOrderedInputPinNames({ a = 1 }) end)

-- IKControl's chain, read before any solve
local Chain = Instance.new("Model")
local function Part(Name, X, Anchored, Turn)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = Anchored or false
	Made.CanCollide = false
	Made.CFrame = CFrame.new(X, 300, 0) * (Turn or CFrame.identity)
	Made.Parent = Chain
	return Made
end
local Top = Part("Top", 0, true)
local Root = Part("Root", 1)
local Arm = Part("Arm", 3, false, CFrame.Angles(0, 0, math.rad(30)))
local Hand = Part("Hand", 6)
local Tip = Part("Tip", 8)
local Spare = Part("Spare", 10)
local function Joined(Class, A, B)
	local Made = Instance.new(Class)
	Made.Name = B.Name
	Made.Part0, Made.Part1 = A, B
	if Made:IsA("JointInstance") then
		Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
	end
	Made.Parent = A
	return Made
end
Joined("Motor6D", Top, Root)
Joined("Motor6D", Root, Arm)
Joined("Motor6D", Arm, Hand)
Joined("Weld", Hand, Tip)
local Turned = Joined("Motor6D", Hand, Spare)
Turned.C0 = CFrame.new(1, 0, 0)
Turned.C1 = CFrame.new(-1, 0, 0) * CFrame.Angles(0, math.rad(45), 0)
Chain.Parent = workspace
local Loose = Part("Loose", 40, true)
Loose.Parent = workspace
local Bones = Part("Bones", 20, true)
local function Bone(Name, Parent, At)
	local Made = Instance.new("Bone")
	Made.Name = Name
	Made.Position = At
	Made.Parent = Parent
	return Made
end
local B1 = Bone("B1", Bones, Vector3.new(0, 1, 0))
local B2 = Bone("B2", B1, Vector3.new(0, 2, 0))
local B3 = Bone("B3", B2, Vector3.new(1, 0, 0))
local Mark = Instance.new("Attachment")
Mark.Position = Vector3.new(1, 0, 0)
Mark.Parent = Hand
local function Read(Label, From, To, Parent)
	local IK = Instance.new("IKControl")
	IK.ChainRoot = From
	IK.EndEffector = To
	IK.Parent = Parent
	local Nodes = {}
	for Index = 0, IK:GetChainCount() + 2 do
		local L, W = IK:GetNodeLocalCFrame(Index), IK:GetNodeWorldCFrame(Index)
		table.insert(Nodes, Index .. ":" .. P.f32s(L:GetComponents()) .. "/" .. P.f32s(W:GetComponents()))
	end
	print("ik", Label, IK:GetChainCount(), P.f32(IK:GetChainLength()), table.concat(Nodes, " | "))
	return IK
end
Read("top-hand", Top, Hand, Controller)
Read("root-hand", Root, Hand, Controller)
Read("arm-hand", Arm, Hand, nil)
Read("hand-hand", Hand, Hand, nil)
Read("root-arm", Root, Arm, nil)
Read("hand-arm", Hand, Arm, nil)
Read("root-tip, welded", Root, Tip, nil)
Read("loose-hand", Loose, Hand, nil)
Read("root-spare, a C1", Root, Spare, nil)
Read("root-attachment", Root, Mark, nil)
Read("no root", nil, Hand, nil)
Read("no effector", Root, nil, nil)
Read("bones b1-b3", B1, B3, nil)
Read("bones part-b3", Bones, B3, nil)
Read("bones b2-b3", B2, B3, nil)
local IK = Read("targets", Arm, Hand, nil)
Try("raw target", function() return IK:GetRawFinalTarget() end)
Try("smoothed target", function() return IK:GetSmoothedFinalTarget() end)
Try("node index fraction", function() return IK:GetNodeWorldCFrame(1.5) end)
Try("node index number string", function() return IK:GetNodeWorldCFrame("2") end)
Try("node index string", function() return IK:GetNodeLocalCFrame("x") end)
Try("node index table", function() return IK:GetNodeWorldCFrame({}) end)
Try("node index nil", function() return IK:GetNodeLocalCFrame(nil) end)
Chain:Destroy()
Loose:Destroy()
Bones:Destroy()

-- rig descriptions
local HRD = Instance.new("HumanoidRigDescription")
Try("GetR15JointNames", function() return HRD:GetR15JointNames() end)
Try("GetR6JointNames", function() return HRD:GetR6JointNames() end)
Try("GetJointNames", function() return HRD:GetJointNames() end)
Try("GetR15JointNames twice", function() return HRD:GetR15JointNames() == HRD:GetR15JointNames() end)
Try("GetJointFromName unset", function() return HRD:GetJointFromName("Root") end)
Try("GetJointFromName nil", function() return HRD:GetJointFromName(nil) end)
local Seen = {}
for Index, Name in HRD:GetJointNames() do
	local Made = Instance.new("Motor6D")
	Made.Name = "J" .. Index
	HRD[Name] = Made
	local Got = HRD:GetJointFromName(Name)
	table.insert(Seen, Name .. "=" .. (if Got then Got.Name else "nil"))
end
print("joints", table.concat(Seen, ","))
Try("GetJointFromName lower", function() return HRD:GetJointFromName("root") end)
Try("GetJointFromName upper", function() return HRD:GetJointFromName("LEFTHIP") end)
Try("GetJointFromName number", function() return HRD:GetJointFromName(5) end)
Try("GetJointFromName other", function() return HRD:GetJointFromName("Nope") end)
Try("GetJointFromName joint name", function() return HRD:GetJointFromName("J1") end)
local DRD = Instance.new("DigitsRigDescription")
for Index = -1, 7 do
	pcall(function() DRD:SetFingerControl(Index, Vector3.new(0.1 * Index + 0.05, 2, -3)) end)
	pcall(function() DRD:SetFingerTip(Index, Vector3.new(Index, 0.5, -0.5)) end)
end
local Controls, Tips = {}, {}
for Index = -1, 7 do
	table.insert(Controls, Index .. "=" .. tostring(DRD:GetFingerControl(Index)))
	table.insert(Tips, Index .. "=" .. tostring(DRD:GetFingerTip(Index)))
end
print("finger controls", table.concat(Controls, " | "))
print("finger tips", table.concat(Tips, " | "))
Try("finger control inside", function() DRD:SetFingerControl(1, Vector3.new(0.25, 0.5, 0.75)) return DRD:GetFingerControl(1) end)
Try("finger control below", function() DRD:SetFingerControl(1, Vector3.new(-0.25, -1, 0)) return DRD:GetFingerControl(1) end)
Try("finger control fraction", function() return DRD:GetFingerControl(1.5) end)
Try("finger control number string", function() return DRD:GetFingerControl("2") end)
Try("finger control string", function() return DRD:SetFingerControl("x", Vector3.one) end)
Try("finger control nil", function() return DRD:SetFingerControl(1, nil) end)
Try("finger tip vector2", function() return DRD:SetFingerTip(1, Vector2.new(1, 1)) end)
Try("finger tip none", function() return DRD:SetFingerTip(1) end)
Try("finger tip no index", function() return DRD:GetFingerTip() end)
Try("finger props", function() return DRD.ThumbRange, DRD.IndexRange, DRD.ThumbSize end)
