-- The pixels EditableImage's draw methods leave: DrawImageTransformed's sampling, each combine type byte by byte, DrawRectangle's
-- colour and transparency, circle and line coverage, and the draw methods' required arguments (a playing client).
local AssetService = game:GetService("AssetService")

-- pixels: an image's RGBA bytes, row by row
local function dump(Label, Image)
	local Size = Image.Size
	local Pixels = Image:ReadPixelsBuffer(Vector2.zero, Size)
	for Y = 0, Size.Y - 1 do
		local Row = {}
		for X = 0, Size.X - 1 do
			local Offset = (Y * Size.X + X) * 4
			table.insert(Row, string.format("%02x%02x%02x%02x", buffer.readu8(Pixels, Offset), buffer.readu8(Pixels, Offset + 1), buffer.readu8(Pixels, Offset + 2), buffer.readu8(Pixels, Offset + 3)))
		end
		print(Label .. " " .. Y .. " " .. table.concat(Row, " "))
	end
end
local function image(Width, Height, Pixels)
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(Width, Height) })
	if Pixels then
		local Bytes = buffer.create(Width * Height * 4)
		for Index, Pixel in Pixels do
			for Channel = 1, 4 do
				buffer.writeu8(Bytes, (Index - 1) * 4 + Channel - 1, Pixel[Channel])
			end
		end
		Image:WritePixelsBuffer(Vector2.zero, Vector2.new(Width, Height), Bytes)
	end
	return Image
end
local function source()
	return image(2, 2, { { 255, 0, 0, 255 }, { 0, 255, 0, 255 }, { 0, 0, 255, 255 }, { 255, 255, 255, 255 } })
end
local function translucent()
	return image(2, 2, { { 255, 0, 0, 128 }, { 0, 255, 0, 64 }, { 0, 0, 255, 255 }, { 255, 255, 255, 0 } })
end
local function draw(Label, Function)
	local Ok, Message = pcall(Function)
	if not Ok then
		print(Label .. " error " .. P.err(Message))
	end
end
local Transforms = {
	{ "t-scale2", Vector2.new(4, 4), Vector2.new(2, 2), 0, {} },
	{ "t-scale2-pixel", Vector2.new(4, 4), Vector2.new(2, 2), 0, { SamplingMode = Enum.ResamplerMode.Pixelated } },
	{ "t-scale1", Vector2.new(4, 4), Vector2.new(1, 1), 0, {} },
	{ "t-half-pos", Vector2.new(3.5, 3.5), Vector2.new(1, 1), 0, {} },
	{ "t-rot90", Vector2.new(4, 4), Vector2.new(2, 2), 90, {} },
	{ "t-rot45", Vector2.new(4, 4), Vector2.new(2, 2), 45, {} },
	{ "t-rot45-pixel", Vector2.new(4, 4), Vector2.new(2, 2), 45, { SamplingMode = Enum.ResamplerMode.Pixelated } },
	{ "t-pivot0", Vector2.new(1, 1), Vector2.new(2, 2), 0, { PivotPoint = Vector2.zero } },
	{ "t-wide", Vector2.new(4, 4), Vector2.new(3, 1.5), 0, {} },
	{ "t-mirror", Vector2.new(4, 4), Vector2.new(-2, 2), 0, {} },
	{ "t-shrink", Vector2.new(4, 4), Vector2.new(0.5, 0.5), 0, {} },
}
for _, Case in Transforms do
	draw(Case[1], function()
		local Target = image(8, 8)
		Target:DrawImageTransformed(Case[2], Case[3], Case[4], source(), Case[5])
		dump(Case[1], Target)
	end)
end
for _, Combine in { "AlphaBlend", "BlendSourceOver", "Overwrite", "Add", "Multiply" } do
	draw("t-combine-" .. Combine, function()
		local Target = image(4, 2, table.create(8, { 10, 100, 200, 160 }))
		Target:DrawImageTransformed(Vector2.new(2, 1), Vector2.new(1, 1), 0, translucent(), { CombineType = Enum.ImageCombineType[Combine] })
		dump("t-combine-" .. Combine, Target)
	end)
end
for _, Combine in { "BlendSourceOver", "Overwrite", "AlphaBlend", "Add", "Subtract", "Multiply" } do
	draw("r-combine-" .. Combine, function()
		local Target = image(2, 1, { { 10, 100, 200, 160 }, { 0, 0, 0, 0 } })
		Target:DrawRectangle(Vector2.zero, Vector2.new(2, 1), Color3.fromRGB(200, 50, 25), 0.4, Enum.ImageCombineType[Combine])
		dump("r-combine-" .. Combine, Target)
	end)
	draw("i-combine-" .. Combine, function()
		local Target = image(2, 2, table.create(4, { 10, 100, 200, 160 }))
		Target:DrawImage(Vector2.zero, translucent(), Enum.ImageCombineType[Combine])
		dump("i-combine-" .. Combine, Target)
	end)
end
draw("rect-fraction", function()
	local Target = image(6, 6)
	Target:DrawRectangle(Vector2.new(1.5, 1.6), Vector2.new(3.2, 2.7), Color3.new(1, 0, 0), 0)
	dump("rect-fraction", Target)
end)
draw("rect-outside", function()
	local Target = image(4, 4)
	Target:DrawRectangle(Vector2.new(-1, 2), Vector2.new(3, 5), Color3.new(0, 1, 0), 0)
	dump("rect-outside", Target)
end)
draw("image-fraction", function()
	local Target = image(4, 4)
	Target:DrawImage(Vector2.new(1.6, 0.4), source())
	dump("image-fraction", Target)
end)
for _, Case in {
	{ "circle", Vector2.new(4, 4), 2, {} },
	{ "circle-half", Vector2.new(3.5, 3.5), 2, {} },
	{ "circle-hard", Vector2.new(4, 4), 2, { Enum.AntiAliasing.Disabled } },
	{ "circle-r3", Vector2.new(4, 4), 3, {} },
	{ "circle-r0", Vector2.new(4, 4), 0, {} },
} do
	draw(Case[1], function()
		local Target = image(8, 8)
		Target:DrawCircle(Case[2], Case[3], Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver, table.unpack(Case[4]))
		dump(Case[1], Target)
	end)
end
for _, Case in {
	{ "line", Vector2.new(1, 1), Vector2.new(6, 4), {} },
	{ "line-hard", Vector2.new(1, 1), Vector2.new(6, 4), { Enum.AntiAliasing.Disabled } },
	{ "line-half", Vector2.new(0.5, 2.5), Vector2.new(6.5, 2.5), {} },
	{ "line-point", Vector2.new(3, 3), Vector2.new(3, 3), {} },
} do
	draw(Case[1], function()
		local Target = image(8, 6)
		Target:DrawLine(Case[2], Case[3], Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver, table.unpack(Case[4]))
		dump(Case[1], Target)
	end)
end
for _, Case in {
	{ "triangle", Vector2.new(1, 1), Vector2.new(6, 1), Vector2.new(1, 6), {} },
	{ "triangle-hard", Vector2.new(1, 1), Vector2.new(6, 1), Vector2.new(1, 6), { Enum.AntiAliasing.Disabled } },
	{ "triangle-back", Vector2.new(1, 1), Vector2.new(1, 6), Vector2.new(6, 1), {} },
} do
	draw(Case[1], function()
		local Target = image(8, 8)
		Target:DrawTriangle(Case[2], Case[3], Case[4], Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver, table.unpack(Case[5]))
		dump(Case[1], Target)
	end)
end

local function image(Width, Height, Pixels)
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(Width, Height) })
	if Pixels then
		local Bytes = buffer.create(Width * Height * 4)
		for Index, Pixel in Pixels do
			for Channel = 1, 4 do
				buffer.writeu8(Bytes, (Index - 1) * 4 + Channel - 1, Pixel[Channel])
			end
		end
		Image:WritePixelsBuffer(Vector2.zero, Vector2.new(Width, Height), Bytes)
	end
	return Image
end
local function row(Image, Alpha)
	local Size = Image.Size
	local Pixels = Image:ReadPixelsBuffer(Vector2.zero, Size)
	local Out = {}
	for Index = 0, Size.X * Size.Y - 1 do
		local Offset = Index * 4
		if Alpha then
			table.insert(Out, string.format("%02x", buffer.readu8(Pixels, Offset + 3)))
		else
			table.insert(Out, string.format("%02x%02x%02x%02x", buffer.readu8(Pixels, Offset), buffer.readu8(Pixels, Offset + 1), buffer.readu8(Pixels, Offset + 2), buffer.readu8(Pixels, Offset + 3)))
		end
	end
	return Out
end
local function lines(Label, Image, Alpha)
	local Size = Image.Size
	local Out = row(Image, Alpha)
	for Y = 0, Size.Y - 1 do
		print(Label .. " " .. Y .. " " .. table.concat(Out, " ", Y * Size.X + 1, (Y + 1) * Size.X))
	end
end
local function draw(Label, Function)
	local Ok, Message = pcall(Function)
	if not Ok then
		print(Label .. " error " .. P.err(Message))
	end
end

-- DrawImage of one row of source pixels over one row of destination pixels, per combine type
local SourceAlphas = { 0, 1, 64, 127, 128, 191, 254, 255 }
local DestAlphas = { 0, 1, 64, 128, 255 }
local Colours = { { { 200, 50, 25 }, { 10, 100, 230 } }, { { 255, 0, 128 }, { 0, 255, 64 } }, { { 3, 250, 129 }, { 255, 255, 255 } } }
for _, Combine in { "BlendSourceOver", "Overwrite", "AlphaBlend", "Add", "Subtract", "Multiply" } do
	for ColourIndex, Pair in Colours do
		draw("image-" .. Combine, function()
			local Source, Dest = {}, {}
			for _, SA in SourceAlphas do
				for _, DA in DestAlphas do
					table.insert(Source, { Pair[1][1], Pair[1][2], Pair[1][3], SA })
					table.insert(Dest, { Pair[2][1], Pair[2][2], Pair[2][3], DA })
				end
			end
			local Target = image(#Dest, 1, Dest)
			Target:DrawImage(Vector2.zero, image(#Source, 1, Source), Enum.ImageCombineType[Combine])
			print("image-" .. Combine .. " " .. ColourIndex .. " " .. table.concat(row(Target), " "))
		end)
	end
end
-- DrawRectangle's colour and transparency, per combine type
for _, Combine in { "BlendSourceOver", "Overwrite", "AlphaBlend", "Add" } do
	draw("rect-" .. Combine, function()
		local Out = {}
		for _, Transparency in { 0, 0.001, 0.25, 0.3, 0.5, 0.75, 0.999, 1, -0.5, 1.5 } do
			for _, DA in { 0, 128, 255 } do
				local Target = image(1, 1, { { 10, 100, 230, DA } })
				Target:DrawRectangle(Vector2.zero, Vector2.one, Color3.new(0.8, 0.2, 0.1), Transparency, Enum.ImageCombineType[Combine])
				table.insert(Out, row(Target)[1])
			end
		end
		print("rect-" .. Combine .. " " .. table.concat(Out, " "))
	end)
end
draw("rect-colour", function()
	local Out = {}
	for _, Colour in { Color3.new(0.5, 0.25, 0.75), Color3.new(1.5, -0.5, 0.001), Color3.new(0.502, 0.998, 0.996) } do
		local Target = image(1, 1)
		Target:DrawRectangle(Vector2.zero, Vector2.one, Colour, 0, Enum.ImageCombineType.Overwrite)
		table.insert(Out, row(Target)[1])
	end
	print("rect-colour " .. table.concat(Out, " "))
end)
-- DrawImageTransformed at scale 1 on pixel centres (its default combine)
draw("transformed-default", function()
	local Source, Dest = {}, {}
	for _, SA in SourceAlphas do
		for _, DA in DestAlphas do
			table.insert(Source, { 200, 50, 25, SA })
			table.insert(Dest, { 10, 100, 230, DA })
		end
	end
	local Target = image(#Dest, 1, Dest)
	Target:DrawImageTransformed(Vector2.new(#Dest / 2, 0.5), Vector2.one, 0, image(#Source, 1, Source))
	print("transformed-default " .. table.concat(row(Target), " "))
end)

-- coverage of circles and lines (alpha only), white on transparent black and on opaque black
for _, Case in {
	{ 1, Vector2.new(5, 5) }, { 2, Vector2.new(5, 5) }, { 3, Vector2.new(5, 5) }, { 4, Vector2.new(5, 5) },
	{ 2, Vector2.new(5.5, 5.5) }, { 3, Vector2.new(5.25, 5.75) }, { 5, Vector2.new(5, 5) }, { 2.5, Vector2.new(5, 5) },
} do
	draw("circle", function()
		local Label = "circle r" .. P.num(Case[1]) .. " " .. P.num(Case[2].X) .. "," .. P.num(Case[2].Y)
		local Target = image(11, 11)
		Target:DrawCircle(Case[2], Case[1], Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver)
		lines(Label, Target, true)
	end)
end
draw("circle-on-black", function()
	local Target = image(7, 7, table.create(49, { 0, 0, 0, 255 }))
	Target:DrawCircle(Vector2.new(3, 3), 2, Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver)
	lines("circle-on-black", Target)
end)
draw("circle-overwrite", function()
	local Target = image(7, 7, table.create(49, { 0, 0, 255, 255 }))
	Target:DrawCircle(Vector2.new(3, 3), 2, Color3.new(1, 0, 0), 0.5, Enum.ImageCombineType.Overwrite)
	lines("circle-overwrite", Target)
end)
draw("circle-hard", function()
	local Target = image(11, 11)
	Target:DrawCircle(Vector2.new(5.5, 5.25), 3, Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver, Enum.AntiAliasing.Disabled)
	lines("circle-hard", Target, true)
end)
for _, Case in {
	{ Vector2.new(1, 1), Vector2.new(8, 4) }, { Vector2.new(1, 8), Vector2.new(8, 1) }, { Vector2.new(2, 1), Vector2.new(3, 8) },
	{ Vector2.new(1.5, 2.25), Vector2.new(8.25, 6.5) }, { Vector2.new(0, 5), Vector2.new(9, 5) }, { Vector2.new(5, 0), Vector2.new(5, 9) },
	{ Vector2.new(4, 4), Vector2.new(4, 4) }, { Vector2.new(4.5, 4.5), Vector2.new(4.5, 4.5) }, { Vector2.new(-2, 3), Vector2.new(12, 6) },
} do
	draw("line", function()
		local Label = "line " .. P.num(Case[1].X) .. "," .. P.num(Case[1].Y) .. "-" .. P.num(Case[2].X) .. "," .. P.num(Case[2].Y)
		local Target = image(10, 10)
		Target:DrawLine(Case[1], Case[2], Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver)
		lines(Label, Target, true)
	end)
end
draw("line-on-black", function()
	local Target = image(10, 6, table.create(60, { 0, 0, 0, 255 }))
	Target:DrawLine(Vector2.new(1, 1), Vector2.new(8, 4), Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver)
	lines("line-on-black", Target)
end)
draw("line-hard-steep", function()
	local Target = image(10, 10)
	Target:DrawLine(Vector2.new(2.5, 1.5), Vector2.new(4.5, 8.5), Color3.new(1, 1, 1), 0, Enum.ImageCombineType.BlendSourceOver, Enum.AntiAliasing.Disabled)
	lines("line-hard-steep", Target, true)
end)

-- arguments the draw methods require, and their checks
local function check(Label, Function)
	local Ok, Message = pcall(Function)
	print(Label .. " " .. (if Ok then "ok" else "error " .. P.err(Message)))
end
local Small = image(4, 4)
local Other = image(2, 2)
check("rect-no-combine", function()
	Small:DrawRectangle(Vector2.zero, Vector2.one, Color3.new(1, 0, 0), 0)
end)
check("rect-no-transparency", function()
	Small:DrawRectangle(Vector2.zero, Vector2.one, Color3.new(1, 0, 0))
end)
check("rect-nil-combine", function()
	Small:DrawRectangle(Vector2.zero, Vector2.one, Color3.new(1, 0, 0), 0, nil)
end)
check("image-no-combine", function()
	Small:DrawImage(Vector2.zero, Other)
end)
check("image-not-image", function()
	Small:DrawImage(Vector2.zero, Instance.new("Part"), Enum.ImageCombineType.Overwrite)
end)
check("image-nil", function()
	Small:DrawImage(Vector2.zero, nil, Enum.ImageCombineType.Overwrite)
end)
check("circle-no-combine", function()
	Small:DrawCircle(Vector2.one, 1, Color3.new(1, 0, 0), 0)
end)
check("circle-negative", function()
	Small:DrawCircle(Vector2.one, -1, Color3.new(1, 0, 0), 0, Enum.ImageCombineType.Overwrite)
end)
check("circle-fraction", function()
	Small:DrawCircle(Vector2.one, 0.4, Color3.new(1, 0, 0), 0, Enum.ImageCombineType.Overwrite)
end)
check("line-no-combine", function()
	Small:DrawLine(Vector2.zero, Vector2.one, Color3.new(1, 0, 0), 0)
end)
check("transformed-no-options", function()
	Small:DrawImageTransformed(Vector2.one, Vector2.one, 0, Other)
end)
check("transformed-nil-image", function()
	Small:DrawImageTransformed(Vector2.one, Vector2.one, 0, nil, {})
end)
check("transformed-bad-option", function()
	Small:DrawImageTransformed(Vector2.one, Vector2.one, 0, Other, { Nope = 1 })
end)
check("transformed-bad-combine", function()
	Small:DrawImageTransformed(Vector2.one, Vector2.one, 0, Other, { CombineType = 5 })
end)
check("transformed-zero-scale", function()
	Small:DrawImageTransformed(Vector2.one, Vector2.new(0, 1), 0, Other, {})
end)
check("projected", function()
	Small:DrawImageProjected(Instance.new("Part"), {}, {})
end)
check("sample-projected", function()
	Small:SampleImageProjected(Instance.new("Part"), Other, {}, {})
end)
check("read-outside", function()
	Small:ReadPixelsBuffer(Vector2.new(3, 3), Vector2.new(2, 2))
end)
check("write-short", function()
	Small:WritePixelsBuffer(Vector2.zero, Vector2.new(2, 2), buffer.create(4))
end)
check("write-long", function()
	Small:WritePixelsBuffer(Vector2.zero, Vector2.new(1, 1), buffer.create(64))
end)
check("write-outside", function()
	Small:WritePixelsBuffer(Vector2.new(3, 0), Vector2.new(2, 1), buffer.create(8))
end)
check("destroyed-draw", function()
	local Gone = image(2, 2)
	Gone:Destroy()
	Gone:DrawRectangle(Vector2.zero, Vector2.one, Color3.new(1, 0, 0), 0, Enum.ImageCombineType.Overwrite)
end)
check("destroyed-read", function()
	local Gone = image(2, 2)
	Gone:Destroy()
	Gone:ReadPixelsBuffer(Vector2.zero, Vector2.one)
end)
check("destroyed-size", function()
	local Gone = image(2, 2)
	Gone:Destroy()
	print("destroyed-size-value", Gone.Size)
end)
check("create-size-zero", function()
	AssetService:CreateEditableImage({ Size = Vector2.new(0, 4) })
end)
check("create-size-big", function()
	AssetService:CreateEditableImage({ Size = Vector2.new(1025, 4) })
end)
check("create-size-fraction", function()
	print("create-size-fraction-value", AssetService:CreateEditableImage({ Size = Vector2.new(2.7, 3.2) }).Size)
end)
check("create-default", function()
	print("create-default-value", AssetService:CreateEditableImage().Size)
end)
