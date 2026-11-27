-- KeyframeSequence AddKeyframe / GetKeyframes / RemoveKeyframe, Keyframe AddPose / GetPoses / RemovePose / AddMarker /
-- GetMarkers / RemoveMarker and Pose AddSubPose / GetSubPoses / RemoveSubPose: what each does to the tree, the lists'
-- order, what they take and refuse, and the events they fire.
P.sync()
local function Make(Class, Name, Props)
	local Made = Instance.new(Class)
	Made.Name = Name
	for Key, Value in Props or {} do
		Made[Key] = Value
	end
	return Made
end
local function Names(List)
	local Out = {}
	for _, Item in List do
		table.insert(Out, Item.Name .. ":" .. Item.ClassName)
	end
	return "[" .. table.concat(Out, ",") .. "]"
end
local function Kids(Of)
	return Names(Of:GetChildren())
end
local function Try(Label, F, ...)
	-- (the place's own name, which a Parent lock's message names, is the capture's)
	print(Label, (string.gsub(P.try(F, ...), "current parent: " .. game.Name .. ",", "current parent: <place>,")))
end

-- KeyframeSequence
local KS = Make("KeyframeSequence", "KS")
local K1 = Make("Keyframe", "K1", { Time = 1 })
local K2 = Make("Keyframe", "K2", { Time = 0.5 })
local K3 = Make("Keyframe", "K3", { Time = 2 })
local K4 = Make("Keyframe", "K4", { Time = 0.5 })
print("add returns", select("#", KS:AddKeyframe(K1)))
KS:AddKeyframe(K3)
KS:AddKeyframe(K2)
KS:AddKeyframe(K4)
print("parent", K1.Parent == KS, K2.Parent == KS)
print("children", Kids(KS))
print("keyframes", Names(KS:GetKeyframes()))
K3.Time = 0.1
print("after time", Names(KS:GetKeyframes()))
local Folder = Make("Folder", "F")
Folder.Parent = KS
local Deep = Make("Keyframe", "Deep", { Time = 0.2 })
Deep.Parent = Folder
local Odd = Make("Part", "Odd")
Odd.Parent = KS
print("with others", Names(KS:GetKeyframes()))
print("same table", KS:GetKeyframes() == KS:GetKeyframes())
-- re-adding one already there
KS:AddKeyframe(K1)
print("readd", Kids(KS))
-- one from elsewhere
local KS2 = Make("KeyframeSequence", "KS2")
local Moved = Make("Keyframe", "Moved", { Time = 3 })
Moved.Parent = KS2
KS:AddKeyframe(Moved)
print("moved", Moved.Parent and Moved.Parent.Name, Kids(KS2))
-- what it takes
Try("add nil", function() KS:AddKeyframe(nil) end)
Try("add none", function() KS:AddKeyframe() end)
Try("add number", function() KS:AddKeyframe(5) end)
Try("add string", function() KS:AddKeyframe("K1") end)
Try("add table", function() KS:AddKeyframe({}) end)
Try("add part", function() local X = Make("Part", "XP") KS:AddKeyframe(X) return X.Parent and X.Parent.Name end)
Try("add folder", function() local X = Make("Folder", "XF") KS:AddKeyframe(X) return X.Parent and X.Parent.Name end)
Try("add pose", function() local X = Make("Pose", "XPose") KS:AddKeyframe(X) return X.Parent and X.Parent.Name end)
Try("add marker", function() local X = Make("KeyframeMarker", "XM") KS:AddKeyframe(X) return X.Parent and X.Parent.Name end)
Try("add self", function() KS:AddKeyframe(KS) end)
Try("add sequence", function() local X = Make("KeyframeSequence", "XS") KS:AddKeyframe(X) return X.Parent and X.Parent.Name end)
Try("add ancestor", function() local Outer = Make("Keyframe", "Outer") KS.Parent = Outer KS:AddKeyframe(Outer) end)
KS.Parent = nil
Try("add destroyed", function() local X = Make("Keyframe", "XD") X:Destroy() KS:AddKeyframe(X) return X.Parent end)
Try("add service", function() KS:AddKeyframe(workspace) end)
Try("dot call", function() KS.AddKeyframe(K1) end)
Try("dot other", function() KS.AddKeyframe(K1, K2) end)
-- removing
print("remove returns", select("#", KS:RemoveKeyframe(K2)))
print("removed", K2.Parent, Kids(KS))
Try("remove again", function() KS:RemoveKeyframe(K2) return K2.Parent end)
Try("remove other's", function() local X = Make("Keyframe", "XO") X.Parent = KS2 KS:RemoveKeyframe(X) return X.Parent and X.Parent.Name end)
Try("remove part child", function() KS:RemoveKeyframe(Odd) return Odd.Parent and Odd.Parent.Name end)
Try("remove deep", function() KS:RemoveKeyframe(Deep) return Deep.Parent and Deep.Parent.Name end)
Try("remove nil", function() KS:RemoveKeyframe(nil) end)
Try("remove number", function() KS:RemoveKeyframe(1) end)
Try("remove pose child", function() local X = Make("Pose", "PC") X.Parent = KS KS:RemoveKeyframe(X) return X.Parent and X.Parent.Name end)
Try("remove self", function() KS:RemoveKeyframe(KS) return KS.Parent end)

-- Keyframe poses
local K = Make("Keyframe", "K")
local PA = Make("Pose", "A")
local PB = Make("Pose", "B")
local NP = Make("NumberPose", "N")
print("addpose returns", select("#", K:AddPose(PB)))
K:AddPose(PA)
Try("addpose numberpose", function() K:AddPose(NP) return NP.Parent and NP.Parent.Name end)
Try("addpose part", function() local X = Make("Part", "XP") K:AddPose(X) return X.Parent and X.Parent.Name end)
Try("addpose keyframe", function() local X = Make("Keyframe", "XK") K:AddPose(X) return X.Parent and X.Parent.Name end)
Try("addpose marker", function() local X = Make("KeyframeMarker", "XM") K:AddPose(X) return X.Parent and X.Parent.Name end)
Try("addpose nil", function() K:AddPose(nil) end)
Try("addpose string", function() K:AddPose("A") end)
Try("addpose self", function() K:AddPose(K) end)
local Sub = Make("Pose", "Sub")
Sub.Parent = PA
print("kids", Kids(K))
print("poses", Names(K:GetPoses()))
print("markers none", Names(K:GetMarkers()))
Try("removepose", function() K:RemovePose(PB) return PB.Parent end)
Try("removepose sub", function() K:RemovePose(Sub) return Sub.Parent and Sub.Parent.Name end)
Try("removepose numberpose", function() K:RemovePose(NP) return NP.Parent end)
Try("removepose part", function() local X = K:FindFirstChild("XP") if X then K:RemovePose(X) return X.Parent end return "absent" end)
Try("removepose nil", function() K:RemovePose(nil) end)
Try("removepose outside", function() local X = Make("Pose", "Out") K:RemovePose(X) return X.Parent end)
print("poses after", Names(K:GetPoses()), Kids(K))

-- Keyframe markers
local M1 = Make("KeyframeMarker", "M1", { Value = "one" })
local M2 = Make("KeyframeMarker", "M2", { Value = "two" })
print("addmarker returns", select("#", K:AddMarker(M2)))
K:AddMarker(M1)
Try("addmarker pose", function() local X = Make("Pose", "MP") K:AddMarker(X) return X.Parent and X.Parent.Name end)
Try("addmarker part", function() local X = Make("Part", "MX") K:AddMarker(X) return X.Parent and X.Parent.Name end)
Try("addmarker nil", function() K:AddMarker(nil) end)
Try("addmarker number", function() K:AddMarker(2) end)
print("markers", Names(K:GetMarkers()))
print("poses with markers", Names(K:GetPoses()))
Try("removemarker", function() K:RemoveMarker(M2) return M2.Parent end)
Try("removemarker pose", function() K:RemoveMarker(PA) return PA.Parent and PA.Parent.Name end)
Try("removemarker nil", function() K:RemoveMarker(nil) end)
print("markers after", Names(K:GetMarkers()), Kids(K))

-- Pose sub-poses
local Root = Make("Pose", "Root")
local S1 = Make("Pose", "S1")
local S2 = Make("Pose", "S2")
print("addsub returns", select("#", Root:AddSubPose(S2)))
Root:AddSubPose(S1)
Try("addsub numberpose", function() local X = Make("NumberPose", "SN") Root:AddSubPose(X) return X.Parent and X.Parent.Name end)
Try("addsub part", function() local X = Make("Part", "SX") Root:AddSubPose(X) return X.Parent and X.Parent.Name end)
Try("addsub keyframe", function() local X = Make("Keyframe", "SK") Root:AddSubPose(X) return X.Parent and X.Parent.Name end)
Try("addsub nil", function() Root:AddSubPose(nil) end)
Try("addsub self", function() Root:AddSubPose(Root) end)
Try("addsub parent", function() local Up = Make("Pose", "Up") Root.Parent = Up Root:AddSubPose(Up) end)
Root.Parent = nil
print("subposes", Names(Root:GetSubPoses()))
Try("removesub", function() Root:RemoveSubPose(S2) return S2.Parent end)
Try("removesub part", function() local X = Root:FindFirstChild("SX") if X then Root:RemoveSubPose(X) return X.Parent end return "absent" end)
Try("removesub nil", function() Root:RemoveSubPose(nil) end)
Try("removesub outside", function() local X = Make("Pose", "SO") Root:RemoveSubPose(X) return X.Parent end)
print("subposes after", Names(Root:GetSubPoses()), Kids(Root))

-- NumberPose (no such methods) and a destroyed parent
Try("numberpose getsub", function() return NP:GetSubPoses() end)
Try("into destroyed", function() local Gone = Make("Keyframe", "Gone") Gone:Destroy() local X = Make("Pose", "GX") Gone:AddPose(X) return X.Parent end)
Try("locked pose", function() local Gone = Make("Pose", "LP") Gone:Destroy() K:AddPose(Gone) return Gone.Parent end)

-- events: ChildAdded / ChildRemoved / AncestryChanged, deferred
local Log = {}
local EK = Make("Keyframe", "EK")
local EP = Make("Pose", "EP")
EK.ChildAdded:Connect(function(Child) table.insert(Log, "added " .. Child.Name) end)
EK.ChildRemoved:Connect(function(Child) table.insert(Log, "removed " .. Child.Name) end)
EP.AncestryChanged:Connect(function(_, Parent) table.insert(Log, "ancestry " .. (Parent and Parent.Name or "nil")) end)
EP:GetPropertyChangedSignal("Parent"):Connect(function() table.insert(Log, "parent changed") end)
EK:AddPose(EP)
table.insert(Log, "after add")
EK:RemovePose(EP)
table.insert(Log, "after remove")
task.wait()
print("events", table.concat(Log, "; "))
