-- Whether two reads of a property holding a datatype give the same value (rawequal, and as a table key), for each
-- datatype a property holds, unchanged between the reads, after a write of an equal value, and read through
-- GetAttribute; the value a write was given against the one read back.
local Part = Instance.new("Part")
Part.Parent = workspace
Part.CustomPhysicalProperties = PhysicalProperties.new(1, 0.3, 0.5)
local Frame = Instance.new("Frame")
local Label = Instance.new("TextLabel")
local Image = Instance.new("ImageLabel")
local Emitter = Instance.new("ParticleEmitter")
local Padding = Instance.new("UIPadding")
local Ray = Instance.new("RayValue")
local Cases = {
	{ "Color3", Part, "Color" },
	{ "Vector3", Part, "Size" },
	{ "CFrame", Part, "CFrame" },
	{ "Position", Part, "Position" },
	{ "BrickColor", Part, "BrickColor" },
	{ "PhysicalProperties", Part, "CustomPhysicalProperties" },
	{ "UDim2", Frame, "Size" },
	{ "Vector2", Frame, "AnchorPoint" },
	{ "UDim", Padding, "PaddingLeft" },
	{ "Rect", Image, "SliceCenter" },
	{ "Font", Label, "FontFace" },
	{ "NumberSequence", Emitter, "Transparency" },
	{ "ColorSequence", Emitter, "Color" },
	{ "NumberRange", Emitter, "Lifetime" },
	{ "Ray", Ray, "Value" },
	{ "EnumItem", Part, "Material" },
	{ "string", Part, "Name" },
}
for _, Case in Cases do
	local Holder, Name = Case[2], Case[3]
	local First = Holder[Name]
	local Second = Holder[Name]
	if First == nil then
		print(Case[1] .. " reads nil")
		continue
	end
	local Keys = {}
	Keys[First] = true
	local Line = Case[1] .. " rawequal " .. tostring(rawequal(First, Second)) .. " key " .. tostring(Keys[Second] == true) .. " eq " .. tostring(First == Second)
	Holder[Name] = First
	Line ..= " after-write " .. tostring(rawequal(First, Holder[Name]))
	print(Line)
end
-- the value given to a write against the one read back
local Given = Color3.new(0.25, 0.5, 0.75)
Part.Color = Given
print("written-color " .. tostring(rawequal(Given, Part.Color)))
local GivenSize = Vector3.new(2, 3, 4)
Part.Size = GivenSize
print("written-size " .. tostring(rawequal(GivenSize, Part.Size)))
local GivenUDim2 = UDim2.new(0, 5, 0, 6)
Frame.Size = GivenUDim2
print("written-udim2 " .. tostring(rawequal(GivenUDim2, Frame.Size)))
-- attributes
for _, Case in {
	{ "Color3", Color3.new(1, 0, 0) },
	{ "Vector3", Vector3.new(1, 2, 3) },
	{ "UDim2", UDim2.new(1, 2, 3, 4) },
	{ "CFrame", CFrame.new(1, 2, 3) },
	{ "NumberRange", NumberRange.new(1, 2) },
} do
	Part:SetAttribute("A", Case[2])
	local First, Second = Part:GetAttribute("A"), Part:GetAttribute("A")
	print("attribute " .. Case[1] .. " rawequal " .. tostring(rawequal(First, Second)) .. " given " .. tostring(rawequal(First, Case[2])))
end
-- a value object's Value
local Holder = Instance.new("Vector3Value")
Holder.Value = Vector3.new(1, 1, 1)
print("vector3value " .. tostring(rawequal(Holder.Value, Holder.Value)))
local CFrameHolder = Instance.new("CFrameValue")
print("cframevalue " .. tostring(rawequal(CFrameHolder.Value, CFrameHolder.Value)))
Part:Destroy()
