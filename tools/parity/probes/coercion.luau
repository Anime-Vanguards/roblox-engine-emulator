-- What a property write does with a value of another type: numbers into integer properties (rounding, range),
-- strings into numbers, anything into booleans and strings, and the errors when a value cannot be used.
local function Describe(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return P.num(Value)
	elseif Kind == "string" then
		return string.format("%q", Value)
	elseif Kind == "Vector3" then
		return "Vector3"
	end
	return Kind .. ":" .. tostring(Value)
end
local Targets = {
	{ "IntValue.Value", function() return Instance.new("IntValue") end, "Value" },
	{ "NumberValue.Value", function() return Instance.new("NumberValue") end, "Value" },
	{ "Frame.ZIndex", function() return Instance.new("Frame") end, "ZIndex" },
	{ "Frame.LayoutOrder", function() return Instance.new("Frame") end, "LayoutOrder" },
	{ "Frame.BorderSizePixel", function() return Instance.new("Frame") end, "BorderSizePixel" },
	{ "Part.Transparency", function() return Instance.new("Part") end, "Transparency" },
	{ "TextLabel.TextSize", function() return Instance.new("TextLabel") end, "TextSize" },
	{ "BoolValue.Value", function() return Instance.new("BoolValue") end, "Value" },
	{ "Part.Anchored", function() return Instance.new("Part") end, "Anchored" },
	{ "StringValue.Value", function() return Instance.new("StringValue") end, "Value" },
	{ "Part.Name", function() return Instance.new("Part") end, "Name" },
	{ "TextLabel.Text", function() return Instance.new("TextLabel") end, "Text" },
}
local Inputs = { 2.7, -2.7, 2.5, -2.5, 0.5, 1.5, -0.5, 2 ^ 31, 2 ^ 31 + 0.5, -2 ^ 31 - 1, 2 ^ 53 + 1, 1e20, 0 / 0, math.huge, -math.huge, "5", "2.7", " 5 ", "0x10", "1e3", "five", "", true, false, 0, 1, {}, Vector3.new(1, 2, 3) }
for _, Target in Targets do
	local Out = {}
	for _, Input in Inputs do
		local Instance = Target[2]()
		local Ok, Message = pcall(function()
			Instance[Target[3]] = Input
		end)
		table.insert(Out, if Ok then Describe(Instance[Target[3]]) else "E(" .. P.err(Message) .. ")")
	end
	print("coerce " .. Target[1] .. " " .. table.concat(Out, " "))
end
-- nil into each kind
local Out = {}
for _, Target in Targets do
	local Instance = Target[2]()
	local Ok, Message = pcall(function()
		Instance[Target[3]] = nil
	end)
	table.insert(Out, Target[1] .. "=" .. (if Ok then Describe(Instance[Target[3]]) else "E(" .. P.err(Message) .. ")"))
end
print("coerce-nil " .. table.concat(Out, " "))
-- datatype properties given another datatype or a table
local Part = Instance.new("Part")
local Frame = Instance.new("Frame")
for _, Case in {
	{ "Size=table", function() Part.Size = { 1, 2, 3 } end },
	{ "Size=Vector2", function() Part.Size = Vector2.new(1, 2) end },
	{ "Size=vector", function() Part.Size = vector.create(4, 5, 6) end },
	{ "CFrame=Vector3", function() Part.CFrame = Vector3.new(1, 2, 3) end },
	{ "Color=BrickColor", function() Part.Color = BrickColor.new("Bright red") end },
	{ "BrickColor=Color3", function() Part.BrickColor = Color3.new(1, 0, 0) end },
	{ "BrickColor=number", function() Part.BrickColor = 21 end },
	{ "BrickColor=string", function() Part.BrickColor = "Bright blue" end },
	{ "UDim2=UDim", function() Frame.Size = UDim.new(1, 2) end },
	{ "UDim2=table", function() Frame.Size = { 1, 2, 3, 4 } end },
	{ "Position=string", function() Frame.Position = "1,2,3,4" end },
	{ "Rotation=string", function() Frame.Rotation = "45" end },
	{ "Font=string", function() Instance.new("TextLabel").Font = "Arial" end },
	{ "FontFace=string", function() Instance.new("TextLabel").FontFace = "Arial" end },
	{ "Material=string-number", function() Part.Material = "256" end },
} do
	print("datatype-write " .. Case[1] .. " " .. P.try(Case[2]))
end
print("after-writes " .. tostring(Part.Size) .. " | " .. tostring(Part.BrickColor) .. " | " .. P.f32s(Part.Color.R, Part.Color.G, Part.Color.B))
