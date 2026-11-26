-- Which joints a free assembly's tree keeps (GetConnectedParts(true)'s order and JointInstance.Active): the root by RootPriority,
-- bigger parts that are not the root, cycles away from the root.
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
		local Pair, Kind = string.match(Spec, "^(%a%a):?(%a*)$")
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
-- the root by RootPriority, all one size
Case("prio-ring", "H!1 A B C D E", "HA AB BC CD DE EH")
Case("prio-ring2", "A B C D E H!1", "HA AB BC CD DE EH")
Case("prio-sq", "H!1 A X B", "HA HB AX BX")
Case("prio-sq3", "H!1 A X B", "HA HB BX AX")
-- a big part that is not the root
Case("big-ring", "H!1 A B:2 C D E", "HA AB BC CD DE EH")
Case("big-sq", "H!1 A X:2 B", "HA HB AX BX")
Case("big-sq3", "H!1 A X:2 B", "HA HB BX AX")
Case("big-sq-a", "H!1 A:2 X B", "HA HB AX BX")
-- the root by size, another big part
Case("two-big-sq", "H:4 A:2 X B", "HA HB AX BX")
Case("two-big-sq3", "H:4 A X:2 B", "HA HB BX AX")
-- a cycle away from the root
Case("far-tri", "H:3 A B C", "HA AB BC CA")
Case("far-tri2", "H:3 A B C", "HA CA AB BC")
Case("far-sq", "H:3 A B C D", "HA AB AC BD CD")
workspace.Gravity = 196.2
