-- Instances: defaults, child order, lookups, Clone, attributes and tags.
P.sync()
local function Describe(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return P.num(Value)
	elseif Kind == "Vector3" then
		return "V3(" .. P.f32s(Value.X, Value.Y, Value.Z) .. ")"
	elseif Kind == "Vector2" then
		return "V2(" .. P.f32s(Value.X, Value.Y) .. ")"
	elseif Kind == "Color3" then
		return "C3(" .. P.f32s(Value.R, Value.G, Value.B) .. ")"
	elseif Kind == "CFrame" then
		return "CF(" .. P.f32s(Value:GetComponents()) .. ")"
	elseif Kind == "Instance" then
		return "Instance(" .. Value:GetFullName() .. ")"
	end
	return Kind .. "(" .. tostring(Value) .. ")"
end

-- 1. defaults of common classes
local Defaults = {
	Part = { "Size", "Anchored", "CanCollide", "CanQuery", "CanTouch", "CastShadow", "Color", "Material", "Transparency", "Reflectance", "Shape", "TopSurface", "BottomSurface", "Massless", "Locked", "CollisionGroup", "BrickColor", "CFrame", "Name", "Archivable" },
	MeshPart = { "Size", "CanCollide", "RenderFidelity", "CollisionFidelity", "DoubleSided", "MeshId", "TextureID", "Material" },
	Model = { "PrimaryPart", "LevelOfDetail", "ModelStreamingMode", "WorldPivot", "Name" },
	Attachment = { "CFrame", "Visible", "Axis", "SecondaryAxis", "WorldPosition" },
	Frame = { "Size", "Position", "BackgroundColor3", "BackgroundTransparency", "BorderColor3", "BorderSizePixel", "BorderMode", "Visible", "ZIndex", "AnchorPoint", "ClipsDescendants", "LayoutOrder", "Rotation", "AutomaticSize", "SizeConstraint", "Active", "Selectable", "Interactable" },
	TextLabel = { "Text", "TextSize", "TextColor3", "Font", "TextScaled", "TextWrapped", "TextXAlignment", "TextYAlignment", "RichText", "TextTransparency", "TextStrokeTransparency", "LineHeight", "MaxVisibleGraphemes", "AutoLocalize", "Size", "BackgroundColor3" },
	TextButton = { "Text", "AutoButtonColor", "Modal", "Selected", "Style", "TextSize", "Size" },
	TextBox = { "Text", "ClearTextOnFocus", "MultiLine", "PlaceholderText", "TextEditable", "ShowNativeInput" },
	ImageLabel = { "Image", "ImageColor3", "ImageTransparency", "ScaleType", "SliceCenter", "SliceScale", "TileSize", "ResampleMode", "ImageRectOffset", "ImageRectSize" },
	ScrollingFrame = { "CanvasSize", "CanvasPosition", "ScrollBarThickness", "ScrollingDirection", "ElasticBehavior", "AutomaticCanvasSize", "ScrollingEnabled", "VerticalScrollBarInset", "HorizontalScrollBarPosition", "VerticalScrollBarPosition", "ScrollBarImageColor3", "ScrollBarImageTransparency" },
	ScreenGui = { "ResetOnSpawn", "IgnoreGuiInset", "ZIndexBehavior", "DisplayOrder", "Enabled", "ScreenInsets", "ClipToDeviceSafeArea", "SafeAreaCompatibility" },
	UIListLayout = { "Padding", "FillDirection", "SortOrder", "HorizontalAlignment", "VerticalAlignment", "Wraps", "HorizontalFlex", "VerticalFlex", "ItemLineAlignment" },
	UIGridLayout = { "CellSize", "CellPadding", "FillDirectionMaxCells", "StartCorner" },
	UIPadding = { "PaddingTop", "PaddingLeft" },
	UICorner = { "CornerRadius" },
	UIStroke = { "Thickness", "Color", "ApplyStrokeMode", "LineJoinMode", "Transparency", "Enabled" },
	UIScale = { "Scale" },
	UIAspectRatioConstraint = { "AspectRatio", "AspectType", "DominantAxis" },
	UIGradient = { "Rotation", "Offset", "Enabled" },
	ParticleEmitter = { "Rate", "Lifetime", "Speed", "SpreadAngle", "Enabled", "LightEmission", "Texture", "ZOffset", "Rotation", "RotSpeed", "Drag", "TimeScale", "Orientation", "EmissionDirection", "Shape", "Squash", "Brightness", "LockedToPart", "VelocityInheritance", "WindAffectsDrag", "FlipbookLayout" },
	Beam = { "Width0", "Width1", "Segments", "TextureLength", "TextureMode", "TextureSpeed", "CurveSize0", "FaceCamera", "LightEmission", "Brightness", "ZOffset", "Enabled" },
	Trail = { "Lifetime", "MinLength", "MaxLength", "WidthScale", "FaceCamera", "Enabled", "TextureLength", "TextureMode" },
	PointLight = { "Brightness", "Range", "Color", "Shadows", "Enabled" },
	SpotLight = { "Angle", "Face", "Range" },
	Sound = { "Volume", "PlaybackSpeed", "Looped", "Playing", "TimePosition", "RollOffMaxDistance", "RollOffMinDistance", "RollOffMode", "SoundId", "PlayOnRemove" },
	Highlight = { "FillColor", "FillTransparency", "OutlineColor", "OutlineTransparency", "DepthMode", "Enabled" },
	BillboardGui = { "Size", "StudsOffset", "AlwaysOnTop", "MaxDistance", "LightInfluence", "Brightness", "ClipsDescendants", "ZIndexBehavior" },
	SurfaceGui = { "Face", "PixelsPerStud", "SizingMode", "CanvasSize", "LightInfluence", "AlwaysOnTop", "ZOffset" },
	Humanoid = { "Health", "MaxHealth", "WalkSpeed", "JumpPower", "JumpHeight", "UseJumpPower", "HipHeight", "RigType", "DisplayDistanceType", "AutoRotate", "BreakJointsOnDeath", "RequiresNeck", "EvaluateStateMachine" },
	Animation = { "AnimationId" },
	Camera = { "FieldOfView", "CameraType", "FieldOfViewMode", "HeadScale", "DiagonalFieldOfView", "MaxAxisFieldOfView" },
	Weld = { "C0", "C1", "Enabled" },
	Motor6D = { "C0", "MaxVelocity", "DesiredAngle", "Transform" },
	WeldConstraint = { "Enabled", "Active" },
	NumberValue = { "Value" },
	IntValue = { "Value" },
	StringValue = { "Value" },
	BoolValue = { "Value" },
	ObjectValue = { "Value" },
	CFrameValue = { "Value" },
	Color3Value = { "Value" },
	Vector3Value = { "Value" },
	BindableEvent = { "Name", "Archivable" },
	Folder = { "Name" },
	Configuration = { "Name" },
	Decal = { "Texture", "Face", "Transparency", "Color3", "ZIndex" },
	Texture = { "StudsPerTileU", "StudsPerTileV", "OffsetStudsU" },
	SpecialMesh = { "MeshType", "Scale", "Offset", "VertexColor" },
	Explosion = { "BlastRadius", "BlastPressure", "DestroyJointRadiusPercent", "ExplosionType", "Visible" },
	Fire = { "Size", "Heat", "Color", "SecondaryColor", "TimeScale" },
	Smoke = { "Size", "Opacity", "RiseVelocity", "Color" },
	Sparkles = { "SparkleColor", "TimeScale" },
	Atmosphere = { "Density", "Offset", "Color", "Decay", "Glare", "Haze" },
	BloomEffect = { "Intensity", "Size", "Threshold" },
	ColorCorrectionEffect = { "Brightness", "Contrast", "Saturation", "TintColor" },
	BlurEffect = { "Size" },
	ProximityPrompt = { "ActionText", "HoldDuration", "MaxActivationDistance", "KeyboardKeyCode", "RequiresLineOfSight", "Style", "Exclusivity" },
	ClickDetector = { "MaxActivationDistance", "CursorIcon" },
	ViewportFrame = { "Ambient", "LightColor", "LightDirection", "ImageColor3", "ImageTransparency" },
	CanvasGroup = { "GroupColor3", "GroupTransparency" },
	VideoFrame = { "Looped", "Playing", "Volume" },
	Tool = { "CanBeDropped", "Enabled", "ManualActivationOnly", "RequiresHandle", "ToolTip", "Grip" },
	Accessory = { "AccessoryType" },
	AnimationController = { "Name" },
	Animator = { "Name" },
	Bone = { "CFrame", "Transform" },
	AlignPosition = { "MaxForce", "MaxVelocity", "Responsiveness", "Mode", "RigidityEnabled" },
	LinearVelocity = { "MaxForce", "VectorVelocity", "VelocityConstraintMode", "RelativeTo" },
}
local Classes = {}
for Class in Defaults do
	table.insert(Classes, Class)
end
table.sort(Classes)
for _, Class in Classes do
	local Ok, Made = pcall(Instance.new, Class)
	if not Ok then
		print("default " .. Class .. " cannot create: " .. P.err(Made))
		continue
	end
	local Parts = {}
	for _, Property in Defaults[Class] do
		local Read, Value = pcall(function()
			return Made[Property]
		end)
		table.insert(Parts, Property .. "=" .. (if Read then Describe(Value) else "error:" .. P.err(Value)))
	end
	print("default " .. Class .. " " .. table.concat(Parts, " "))
end

-- 2. child order, name collisions, lookups
local Parent = Instance.new("Folder")
local Names = { "b", "a", "c", "a", "B", "a" }
local Made = {}
for Index, Name in Names do
	local Child = Instance.new(if Index % 2 == 0 then "Part" else "Folder")
	Child.Name = Name
	Child.Parent = Parent
	Made[Index] = Child
	Child:SetAttribute("Index", Index)
end
local function Order(List)
	local Out = {}
	for _, Child in List do
		table.insert(Out, Child.Name .. ":" .. tostring(Child:GetAttribute("Index")))
	end
	return table.concat(Out, ",")
end
print("children " .. Order(Parent:GetChildren()))
print("findfirstchild a -> " .. Parent:FindFirstChild("a"):GetAttribute("Index"))
Made[2].Parent = nil
Made[2].Parent = Parent
print("children-after-readd " .. Order(Parent:GetChildren()))
print("findfirstchild a after readd -> " .. Parent:FindFirstChild("a"):GetAttribute("Index"))
print("indexing a -> " .. Parent.a:GetAttribute("Index"))
Made[4].Name = "z"
print("rename-keeps-order " .. Order(Parent:GetChildren()))
print("ofclass Part -> " .. Parent:FindFirstChildOfClass("Part"):GetAttribute("Index") .. " whichisa BasePart -> " .. Parent:FindFirstChildWhichIsA("BasePart"):GetAttribute("Index"))
print("findfirstchild B case -> " .. tostring(Parent:FindFirstChild("B") and Parent:FindFirstChild("B"):GetAttribute("Index")))
print("findfirstchild missing -> " .. tostring(Parent:FindFirstChild("missing")))
local Deep = Instance.new("Folder")
Deep.Name = "deep"
Deep.Parent = Made[1]
local Deeper = Instance.new("Folder")
Deeper.Name = "target"
Deeper.Parent = Deep
local Shallow = Instance.new("Folder")
Shallow.Name = "target"
Shallow.Parent = Made[3]
print("recursive find -> " .. Parent:FindFirstChild("target", true):GetFullName())
print("descendants " .. #Parent:GetDescendants())
local DescendantNames = {}
for _, Descendant in Parent:GetDescendants() do
	table.insert(DescendantNames, Descendant.Name)
end
print("descendant-order " .. table.concat(DescendantNames, ","))
print("findfirstdescendant -> " .. P.try(function()
	return Parent:FindFirstDescendant("target"):GetFullName()
end))
print("findfirstancestor -> " .. tostring(Deeper:FindFirstAncestor("b") and Deeper:FindFirstAncestor("b").Name) .. " ofclass " .. tostring(Deeper:FindFirstAncestorOfClass("Folder").Name) .. " whichisa " .. tostring(Deeper:FindFirstAncestorWhichIsA("Instance").Name))
print("isdescendantof " .. tostring(Deeper:IsDescendantOf(Parent)) .. " " .. tostring(Parent:IsAncestorOf(Deeper)) .. " self " .. tostring(Parent:IsDescendantOf(Parent)) .. " nil " .. P.try(Parent.IsDescendantOf, Parent, nil))
print("fullname " .. Deeper:GetFullName() .. " | " .. workspace:GetFullName() .. " | game is its name " .. tostring(game:GetFullName() == game.Name))
print("waitforchild-hit " .. Parent:WaitForChild("c").Name)
print("waitforchild-timeout " .. tostring(Parent:WaitForChild("never", 0.05)))
print("findfirstchild-number " .. P.try(Parent.FindFirstChild, Parent, 5))
print("findfirstchild-nil " .. P.try(Parent.FindFirstChild, Parent, nil))

-- 3. Clone
local Source = Instance.new("Part")
Source.Name = "Source"
Source:SetAttribute("Num", 5)
Source:SetAttribute("Str", "x")
Source:SetAttribute("Vec", Vector3.new(1, 2, 3))
Source:AddTag("TagA")
Source:AddTag("TagB")
local Kept = Instance.new("Folder")
Kept.Name = "Kept"
Kept.Parent = Source
local Skipped = Instance.new("Folder")
Skipped.Name = "Skipped"
Skipped.Archivable = false
Skipped.Parent = Source
local Under = Instance.new("Folder")
Under.Name = "UnderSkipped"
Under.Parent = Skipped
local Pointer = Instance.new("ObjectValue")
Pointer.Name = "Pointer"
Pointer.Value = Kept
Pointer.Parent = Source
local Outside = Instance.new("ObjectValue")
Outside.Name = "Outside"
Outside.Value = workspace
Outside.Parent = Source
local Weld = Instance.new("Weld")
Weld.Part0 = Source
Weld.Parent = Source
Source.Parent = workspace
local Copy = Source:Clone()
local Attributes = {}
for Name, Value in Copy:GetAttributes() do
	table.insert(Attributes, Name .. "=" .. Describe(Value))
end
table.sort(Attributes)
local Tags = Copy:GetTags()
table.sort(Tags)
local CopyChildren = {}
for _, Child in Copy:GetChildren() do
	table.insert(CopyChildren, Child.Name)
end
print("clone parent " .. tostring(Copy.Parent) .. " name " .. Copy.Name .. " attributes " .. table.concat(Attributes, ",") .. " tags " .. table.concat(Tags, ","))
print("clone children " .. table.concat(CopyChildren, ","))
print("clone pointer-remapped " .. tostring(Copy.Pointer.Value == Copy.Kept) .. " outside-kept " .. tostring(Copy.Outside.Value == workspace) .. " weld-part0 " .. tostring(Copy.Weld.Part0 == Copy))
Source.Archivable = false
print("clone-unarchivable " .. tostring(Source:Clone()))
Source.Archivable = true
local Service = game:GetService("Lighting")
print("clone-service " .. P.try(function()
	return Service:Clone()
end))
print("clone-player-service " .. P.try(function()
	return game:GetService("Players"):Clone()
end))
Source:Destroy()

-- 4. attribute types and limits
local Holder = Instance.new("Folder")
local Values = {
	{ "string", "s" }, { "number", 1.5 }, { "integer", 3 }, { "boolean", true }, { "UDim", UDim.new(1, 2) },
	{ "UDim2", UDim2.new(1, 2, 3, 4) }, { "BrickColor", BrickColor.new("Bright red") }, { "Color3", Color3.new(1, 0.5, 0) },
	{ "Vector2", Vector2.new(1, 2) }, { "Vector3", Vector3.new(1, 2, 3) }, { "CFrame", CFrame.new(1, 2, 3) },
	{ "NumberSequence", NumberSequence.new(1) }, { "ColorSequence", ColorSequence.new(Color3.new(1, 0, 0)) },
	{ "NumberRange", NumberRange.new(1, 2) }, { "Rect", Rect.new(1, 2, 3, 4) }, { "Font", Font.new("rbxasset://fonts/families/Arial.json") },
	{ "EnumItem", Enum.Material.Plastic }, { "table", {} }, { "Instance", workspace }, { "function", print },
	{ "Vector3int16", Vector3int16.new(1, 2, 3) }, { "Ray", Ray.new(Vector3.zero, Vector3.one) }, { "inf", math.huge },
	{ "nan", 0 / 0 }, { "Region3", Region3.new(Vector3.zero, Vector3.one) }, { "buffer", buffer.create(1) },
	{ "vector", vector.create(1, 2, 3) },
}
for _, Pair in Values do
	local Result = P.try(Holder.SetAttribute, Holder, "A", Pair[2])
	local Read = Holder:GetAttribute("A")
	print("attribute " .. Pair[1] .. " " .. Result .. " -> " .. Describe(Read))
	Holder:SetAttribute("A", nil)
end
for _, Name in { "", "with space", "dash-name", "under_score", "dot.name", "RBXName", "rbxname", "123", "Ünï", string.rep("x", 100), string.rep("x", 101), "a/b", "$x" } do
	print("attribute-name " .. string.format("%q", Name) .. " " .. P.try(Holder.SetAttribute, Holder, Name, 1))
end
print("attribute-name-number " .. P.try(Holder.SetAttribute, Holder, 5, 1))
print("getattribute-missing " .. tostring(Holder:GetAttribute("missing")))
print("getattribute-nonstring " .. P.try(Holder.GetAttribute, Holder, 5))
for Index, Name in { "zeta", "alpha", "mid", "beta" } do
	Holder:SetAttribute(Name, Index)
end
local Listed = {}
for Name in Holder:GetAttributes() do
	table.insert(Listed, Name)
end
print("getattributes-count " .. #Listed)
print("long-string-attribute " .. P.try(Holder.SetAttribute, Holder, "Long", string.rep("y", 300000)))

-- 5. tags
local Tagged = Instance.new("Folder")
Tagged:AddTag("b")
Tagged:AddTag("a")
Tagged:AddTag("b")
print("tags " .. table.concat(Tagged:GetTags(), ",") .. " has " .. tostring(Tagged:HasTag("a")) .. " " .. tostring(Tagged:HasTag("c")))
Tagged:RemoveTag("missing")
Tagged:RemoveTag("b")
print("tags-after-remove " .. table.concat(Tagged:GetTags(), ","))
print("addtag-number " .. P.try(Tagged.AddTag, Tagged, 5))
print("addtag-empty " .. P.try(Tagged.AddTag, Tagged, ""))
local CollectionService = game:GetService("CollectionService")
Tagged.Parent = workspace
print("gettagged " .. #CollectionService:GetTagged("a") .. " parentless " .. (function()
	local Loose = Instance.new("Folder")
	Loose:AddTag("loose-tag")
	return #CollectionService:GetTagged("loose-tag")
end)())
Tagged:Destroy()
