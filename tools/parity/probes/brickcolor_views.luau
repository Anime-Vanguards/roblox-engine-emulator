-- The BrickColor views of a colour property (a part's BrickColor from Color, a GUI object's BackgroundColor,
-- BorderColor and TextColor from their Color3) given the colour halfway between two palette colours, and what a part
-- keeps of a Color write.
local Palette = {}
for Index = 0, 127 do
	Palette[Index] = BrickColor.palette(Index)
end
local function Middle(A, B)
	return Color3.new((A.R + B.R) / 2, (A.G + B.G) / 2, (A.B + B.B) / 2)
end

local Frame = Instance.new("Frame")
local Label = Instance.new("TextLabel")
local Viewer = Instance.new("Part")
for First = 0, 3 do
	local Background, Border, Text, Constructed = {}, {}, {}, {}
	local A = Palette[First].Color
	for Second = First + 1, 127 do
		local Color = Middle(A, Palette[Second].Color)
		Frame.BackgroundColor3 = Color
		Frame.BorderColor3 = Color
		Label.TextColor3 = Color
		table.insert(Background, tostring(Frame.BackgroundColor.Number))
		table.insert(Border, tostring(Frame.BorderColor.Number))
		table.insert(Text, tostring(Label.TextColor.Number))
		table.insert(Constructed, tostring(BrickColor.new(Color).Number))
	end
	print("view-background " .. Palette[First].Number .. " " .. table.concat(Background, ","))
	print("view-border " .. Palette[First].Number .. " " .. table.concat(Border, ","))
	print("view-text " .. Palette[First].Number .. " " .. table.concat(Text, ","))
	print("constructed " .. Palette[First].Number .. " " .. table.concat(Constructed, ","))
end

-- what a part and a frame keep of a colour, and the part's BrickColor of it
for _, Components in { { 0.5, 0.25, 0.75 }, { 0.1, 0.2, 0.3 }, { 0.999, 0.001, 0.0019 }, { 0.0020, 0.0039, 0.00392156886 }, { 1.5, -0.5, 2 },
	{ 0 / 0, 0.5, 0.5 }, { math.huge, -math.huge, 0.5 } } do
	local Color = Color3.new(table.unpack(Components))
	print("part-color " .. P.f32s(Color.R, Color.G, Color.B) .. " " .. P.try(function()
		Viewer.Color = Color
		return P.f32s(Viewer.Color.R, Viewer.Color.G, Viewer.Color.B) .. " brick " .. Viewer.BrickColor.Number
	end) .. " | frame " .. P.try(function()
		Frame.BackgroundColor3 = Color
		return P.f32s(Frame.BackgroundColor3.R, Frame.BackgroundColor3.G, Frame.BackgroundColor3.B) .. " brick " .. Frame.BackgroundColor.Number
	end))
end
