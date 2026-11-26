-- GetConnectedParts(true) in free assemblies: triangles, squares, chords and rings welded in different orders, placed a weld at a
-- time or taken into the Workspace as one model.
P.sync()
workspace.Gravity = 0
local Spot = 0
local function Part(Name, Big)
	Spot += 1
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = if Big then Vector3.new(3, 3, 3) else Vector3.new(1, 1, 1)
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = CFrame.new(300 + 3 * Spot, 60, 40)
	return Made
end
local function Names(List)
	local Out = {}
	for _, Each in List do
		table.insert(Out, Each.Name)
	end
	return table.concat(Out, ",")
end
-- parts made in the order of PartsText (H is big, the root); welds made in the order of Edges ("XY": Part0 X, Part1 Y);
-- Placed: parts in the Workspace first, then each weld parented in turn; else the whole model taken in at once
local function Case(Label, PartsText, Edges, Mode)
	local Model = Instance.new("Model")
	local By = {}
	for Name in string.gmatch(PartsText, "%S+") do
		By[Name] = Part(Name, Name == "H")
		By[Name].Parent = Model
	end
	if Mode ~= "model" then
		Model.Parent = workspace
	end
	for Pair in string.gmatch(Edges, "%S+") do
		local W = Instance.new("Weld")
		W.Name = Pair
		local A, B = By[string.sub(Pair, 1, 1)], By[string.sub(Pair, 2, 2)]
		W.Part0, W.Part1 = A, B
		W.C0 = A.CFrame:ToObjectSpace(B.CFrame)
		W.Parent = A
	end
	if Mode == "model" then
		Model.Parent = workspace
	end
	print(Label, Mode or "placed", By.H.AssemblyRootPart.Name, Names(By.H:GetConnectedParts(true)), "H", Names(By.H:GetJoints()))
	Model:Destroy()
end
for _, Mode in { "placed", "model" } do
	Case("tri1", "H A B", "HA HB AB", Mode)
	Case("tri2", "H A B", "HA AB HB", Mode)
	Case("tri3", "H A B", "AB HA HB", Mode)
	Case("tri4", "H A B", "AB HB HA", Mode)
	Case("sq1", "H A X B", "HA HB AX BX", Mode)
	Case("sq2", "H A X B", "HA AX XB BH", Mode)
	Case("sq3", "H A X B", "HA HB BX AX", Mode)
	Case("sq4", "H A X B", "AX XB HA HB", Mode)
	Case("sq5", "H A X B", "XA XB AH BH", Mode)
	Case("chord-a", "H A B C D", "HA HB BC BD DA", Mode)
	Case("chord-b", "H A B C D", "HA HB BD BC DA", Mode)
	Case("chord-c", "H A B C D", "HB HA BC BD DA", Mode)
	Case("chord-d", "H A B C D", "HA HB BC BD AD", Mode)
	Case("chord-e", "H A B C D", "DA HA HB BC BD", Mode)
	Case("ring-a", "H A B C D E", "HA AB HC CD DE BE", Mode)
	Case("ring-b", "E D C B A H", "HA AB HC CD DE BE", Mode)
	Case("ring-c", "H A B C D E", "AB BC CD DE EH HA", Mode)
	Case("ring-d", "H A B C D E", "HA AB BC CD DE EH", Mode)
	Case("ring-e", "H A B C D E", "HA EH AB DE BC CD", Mode)
	Case("pend", "H A B C D", "HA AB BC CD", Mode)
	Case("pend-rev", "H A B C D", "CD BC AB HA", Mode)
	Case("star-tail", "H A B C D E", "HA HB HC AD DE", Mode)
	Case("star-tail-rev", "H A B C D E", "DE AD HC HB HA", Mode)
end
workspace.Gravity = 196.2
