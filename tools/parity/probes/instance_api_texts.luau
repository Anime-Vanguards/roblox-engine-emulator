-- Instance API errors and locks: wrong arguments to PivotTo and Instance.new, a key that is no string, property names
-- that name no property (or one a script cannot use), an attribute signal's text, Clone of what refuses it, the parent
-- rules some classes keep, and a part's current physical properties and resizeable faces.
local function Try(Label, Function, ...)
	print(Label .. " " .. P.try(Function, ...))
end
local Part = Instance.new("Part")
Part.Name = "Holder"
Part.Parent = workspace
local Model = Instance.new("Model")
Model.Name = "Box"
Model.Parent = workspace
Instance.new("Part", Model).Name = "Inner"

-- PivotTo's argument
for _, Case in { { "nil", nil }, { "string", "x" }, { "number", 5 }, { "Vector3", Vector3.one }, { "table", {} }, { "Instance", workspace } } do
	Try("pivot part " .. Case[1], function()
		Part:PivotTo(Case[2])
	end)
	Try("pivot model " .. Case[1], function()
		Model:PivotTo(Case[2])
	end)
end
Try("pivot none", function()
	Part:PivotTo()
end)

-- Instance.new's class name
for _, Case in { { "nil", nil }, { "boolean", true }, { "table", {} }, { "Vector3", Vector3.one }, { "Instance", workspace }, { "number", 5 } } do
	Try("new " .. Case[1], function()
		return Instance.new(Case[2])
	end)
end
Try("new none", function()
	return Instance.new()
end)

-- a key that is no string
for _, Key in { 5, 5.5, -1, 0.1, 0.3, 1 / 3, 1e-7, -1.5e-10, 123456.789, 1e15, 1e16, 1e20, 1e21, 2 ^ 53, 2 ^ 63, 1e300, math.huge, -math.huge, 0 / 0, -0 } do
	Try("key " .. P.num(Key), function()
		return Part[Key]
	end)
	Try("key-write " .. P.num(Key), function()
		Part[Key] = 1
	end)
end
for _, Case in { { "boolean", true }, { "table", {} }, { "Vector3", Vector3.one }, { "Instance", workspace }, { "function", print }, { "EnumItem", Enum.Material.Plastic } } do
	Try("key " .. Case[1], function()
		return Part[Case[2]]
	end)
	Try("key-write " .. Case[1], function()
		Part[Case[2]] = 1
	end)
end
local Numbered = Instance.new("Folder")
Instance.new("Folder", Numbered).Name = "5"
Try("key child-named-5", function()
	return Numbered[5]
end)

-- property names that name no property
local Folder = Instance.new("Folder")
local Table = Instance.new("LocalizationTable")
local Weld = Instance.new("WeldConstraint")
local Names = {
	{ "Folder", Folder, "Nope" },
	{ "Folder", Folder, "Name" },
	{ "Folder", Folder, "name" },
	{ "Folder", Folder, "Changed" },
	{ "Folder", Folder, "Destroy" },
	{ "Part", Part, "Nope" },
	{ "Part", Part, "Color" },
	{ "Part", Part, "Origin" },
	{ "Part", Part, "size" },
	{ "Part", Part, "Color3uint8" },
	{ "Model", Model, "WorldPivotData" },
	{ "Model", Model, "LevelOfDetail" },
	{ "LocalizationTable", Table, "Contents" },
	{ "WeldConstraint", Weld, "State" },
	{ "Workspace", workspace, "SignalBehavior" },
	{ "Workspace", workspace, "StreamingMinRadius" },
	{ "Workspace", workspace, "LevelOfDetail" },
}
for _, Case in Names do
	local Label = Case[1] .. " " .. Case[3]
	Try("modified " .. Label, function()
		return Case[2]:IsPropertyModified(Case[3])
	end)
	Try("reset " .. Label, function()
		Case[2]:ResetPropertyToDefault(Case[3])
	end)
	Try("styled " .. Label, function()
		return Case[2]:GetStyled(Case[3])
	end)
	Try("signal " .. Label, function()
		return Case[2]:GetPropertyChangedSignal(Case[3]) ~= nil
	end)
end
for _, Case in { { "nil", nil }, { "number", 5 }, { "boolean", true } } do
	Try("modified-arg " .. Case[1], function()
		return Folder:IsPropertyModified(Case[2])
	end)
	Try("styled-arg " .. Case[1], function()
		return Folder:GetStyled(Case[2])
	end)
end

-- signals' text
print("tostring attribute " .. tostring(Folder:GetAttributeChangedSignal("n")))
print("tostring attribute-dotted " .. tostring(Folder:GetAttributeChangedSignal("a.b")))
print("tostring attribute-number " .. tostring(Folder:GetAttributeChangedSignal(5)))
print("tostring attributechanged " .. tostring(Folder.AttributeChanged))
print("tostring property " .. tostring(Folder:GetPropertyChangedSignal("Name")))
print("tostring changed " .. tostring(Folder.Changed))

-- Clone of what refuses it
Try("clone Terrain", function()
	return workspace.Terrain:Clone()
end)
print("archivable Terrain " .. tostring(workspace.Terrain.Archivable))
local StarterPlayer = game:GetService("StarterPlayer")
for _, Name in { "StarterPlayerScripts", "StarterCharacterScripts" } do
	local Found = StarterPlayer:FindFirstChild(Name)
	print("starter " .. Name .. " " .. tostring(Found ~= nil))
	if Found then
		Try("clone " .. Name, function()
			return Found:Clone()
		end)
	end
	Try("clone loose " .. Name, function()
		return Instance.new(Name):Clone()
	end)
end
local LocalPlayer = game:GetService("Players").LocalPlayer
if LocalPlayer then
	for _, Name in { "PlayerGui", "PlayerScripts", "Backpack" } do
		local Found = LocalPlayer:FindFirstChild(Name)
		if Found then
			Try("clone " .. Name, function()
				return Found:Clone()
			end)
			print("archivable " .. Name .. " " .. tostring(Found.Archivable))
		else
			print("clone " .. Name .. " absent")
		end
	end
else
	print("clone player none")
end

-- GetTextBoundsParams
local Params = Instance.new("GetTextBoundsParams")
Try("params clone", function()
	return Params:Clone()
end)
print("params archivable " .. tostring(Params.Archivable))
Try("params parent", function()
	Params.Parent = workspace
	return Params.Parent
end)
Try("params parent-nil", function()
	Params.Parent = nil
	return Params.Parent
end)
Try("params new-parented", function()
	return Instance.new("GetTextBoundsParams", Folder)
end)
Try("params destroy", function()
	Params:Destroy()
end)
local Holding = Instance.new("Folder")
Try("params folder clone", function()
	return #Holding:Clone():GetChildren()
end)

-- classes that keep a parent rule
local Parents = {
	Part = function()
		return Instance.new("Part")
	end,
	WedgePart = function()
		return Instance.new("WedgePart")
	end,
	MeshPart = function()
		return Instance.new("MeshPart")
	end,
	Folder = function()
		return Instance.new("Folder")
	end,
	Model = function()
		return Instance.new("Model")
	end,
	Humanoid = function()
		return Instance.new("Humanoid")
	end,
	AnimationController = function()
		return Instance.new("AnimationController")
	end,
	Bone = function()
		local Bone = Instance.new("Bone")
		Bone.Parent = Instance.new("Part")
		return Bone
	end,
	Attachment = function()
		return Instance.new("Attachment")
	end,
	Seat = function()
		return Instance.new("Seat")
	end,
	SpawnLocation = function()
		return Instance.new("SpawnLocation")
	end,
	TrussPart = function()
		return Instance.new("TrussPart")
	end,
}
local ParentNames = { "Part", "WedgePart", "MeshPart", "Folder", "Model", "Humanoid", "AnimationController", "Bone", "Attachment", "Seat", "SpawnLocation", "TrussPart" }
for _, Class in { "AdPortal", "Animator", "Bone", "WrapLayer", "WrapTarget" } do
	for _, ParentName in ParentNames do
		local Child = Instance.new(Class)
		local Target = Parents[ParentName]()
		Try("parent " .. Class .. " " .. ParentName, function()
			Child.Parent = Target
			return Child.Parent and Child.Parent.ClassName
		end)
	end
	local Child = Instance.new(Class)
	Try("parent " .. Class .. " workspace", function()
		Child.Parent = workspace
	end)
	Try("parent " .. Class .. " nil", function()
		Child.Parent = nil
		return Child.Parent
	end)
	Try("parent " .. Class .. " new", function()
		return Instance.new(Class, Instance.new("Folder"))
	end)
	-- a child that took its place, then its parent moved into a folder, and then the child taken out to a folder
	local Placed = Instance.new(Class)
	local Right = Parents[if Class == "Animator" then "Humanoid" elseif Class == "AdPortal" then "Part" elseif Class == "Bone" then "Part" else "MeshPart"]()
	Placed.Parent = Right
	Try("parent " .. Class .. " carried", function()
		Right.Parent = Instance.new("Folder")
		return Placed.Parent.ClassName
	end)
	Try("parent " .. Class .. " clone", function()
		return Right:Clone():FindFirstChildOfClass(Class) ~= nil
	end)
	Try("parent " .. Class .. " moved-out", function()
		Placed.Parent = Instance.new("Folder")
	end)
end

-- a part's current physical properties and resizeable faces
for _, Material in { "Plastic", "Wood", "Neon", "Ice", "Concrete", "Granite", "Air", "Water" } do
	local Piece = Instance.new("Part")
	Try("physical " .. Material, function()
		Piece.Material = Enum.Material[Material]
		return Piece.CurrentPhysicalProperties
	end)
end
local Custom = Instance.new("Part")
Custom.CustomPhysicalProperties = PhysicalProperties.new(2, 0.4, 0.6, 3, 4)
print("physical custom " .. tostring(Custom.CurrentPhysicalProperties))
Custom.CustomPhysicalProperties = nil
print("physical custom-cleared " .. tostring(Custom.CurrentPhysicalProperties))
for _, Class in { "Part", "WedgePart", "CornerWedgePart", "TrussPart", "MeshPart", "SpawnLocation", "Seat" } do
	local Piece = Instance.new(Class)
	print("faces " .. Class .. " " .. tostring(Piece.ResizeableFaces) .. " " .. P.num(Piece.ResizeIncrement))
end
for _, Shape in { "Ball", "Cylinder", "Wedge", "CornerWedge" } do
	local Piece = Instance.new("Part")
	Piece.Shape = Enum.PartType[Shape]
	print("faces shape " .. Shape .. " " .. tostring(Piece.ResizeableFaces) .. " " .. P.num(Piece.ResizeIncrement))
end
-- a Size write with one axis that is not a number
local Sized = Instance.new("Part")
Sized.Size = Vector3.new(3, 4, 5)
for _, Size in { Vector3.new(1, 0 / 0, 1), Vector3.new(0 / 0, 0 / 0, 0 / 0), Vector3.new(4000, 1, 1) } do
	Sized.Size = Size
	print("size-write " .. P.f32s(Size.X, Size.Y, Size.Z) .. " reads " .. P.f32s(Sized.Size.X, Sized.Size.Y, Sized.Size.Z))
end
-- number keys at the edges of the fixed notation
for _, Key in { 1e-6, 1.5e-6, -1e-6, 1e-5, 9.99e20, 1.5e21, 123e-9, 5e-324, 1.7976931348623157e308 } do
	Try("key-edge " .. P.num(Key), function()
		return Part[Key]
	end)
end
Part:Destroy()
Model:Destroy()
