-- An EditableImage's Content in the properties that take an image, read back (and its string twin), before and after the
-- image is written; Decal's Texture and ColorMap (a playing client).
local AssetService = game:GetService("AssetService")
local function content(Value)
	if typeof(Value) ~= "Content" then
		return typeof(Value) .. ":" .. tostring(Value)
	end
	local Object = Value.Object
	return tostring(Value.SourceType) .. " uri=" .. tostring(Value.Uri) .. " object=" .. (if Object then Object.ClassName else "nil")
end
local function try(Label, Function)
	local Results = table.pack(pcall(Function))
	if Results[1] then
		local Out = {}
		for Index = 2, Results.n do
			table.insert(Out, tostring(Results[Index]))
		end
		print(Label .. " ok " .. table.concat(Out, " | "))
	else
		print(Label .. " error " .. P.err(Results[2]))
	end
end

-- a Content property given an image's Content, read back (and its string twin), before and after the image is written
local Cases = {
	{ "ImageLabel", "ImageContent", "Image" },
	{ "ImageButton", "ImageContent", "Image" },
	{ "ImageButton", "HoverImageContent", "HoverImage" },
	{ "ImageButton", "PressedImageContent", "PressedImage" },
	{ "ImageHandleAdornment", "ImageContent", "Image" },
	{ "Decal", "ColorMapContent", "ColorMap" },
	{ "Decal", "TextureContent", "Texture" },
	{ "Texture", "ColorMapContent", "Texture" },
	{ "MeshPart", "TextureContent", "TextureID" },
	{ "SpecialMesh", "TextureContent", "TextureId" },
	{ "ParticleEmitter", "TextureContent", "Texture" },
	{ "Beam", "TextureContent", "Texture" },
	{ "Trail", "TextureContent", "Texture" },
}
for _, Case in Cases do
	try(Case[1] .. "." .. Case[2], function()
		local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
		local Holder = Instance.new(Case[1])
		local Given = Content.fromObject(Image)
		Holder[Case[2]] = Given
		local Read = Holder[Case[2]]
		local Line = { content(Read), "same=" .. tostring(Read.Object == Image), "eq=" .. tostring(Read == Given), "string=" .. tostring(Holder[Case[3]]) }
		Image:WritePixelsBuffer(Vector2.zero, Vector2.new(2, 2), buffer.create(16))
		task.wait()
		table.insert(Line, "written=" .. content(Holder[Case[2]]))
		table.insert(Line, "written-string=" .. tostring(Holder[Case[3]]))
		Holder[Case[3]] = "rbxassetid://5"
		table.insert(Line, "after-string=" .. content(Holder[Case[2]]))
		Holder[Case[2]] = Given
		Holder[Case[2]] = Content.none
		table.insert(Line, "after-none=" .. content(Holder[Case[2]]) .. " " .. tostring(Holder[Case[3]]))
		return table.concat(Line, " ")
	end)
end
try("destroyed", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Label = Instance.new("ImageLabel")
	Label.ImageContent = Content.fromObject(Image)
	Image:Destroy()
	task.wait()
	return content(Label.ImageContent), tostring(Label.Image)
end)
try("two-images", function()
	local A = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local B = AssetService:CreateEditableImage({ Size = Vector2.new(3, 3) })
	local Label = Instance.new("ImageLabel")
	Label.ImageContent = Content.fromObject(A)
	Label.ImageContent = Content.fromObject(B)
	return content(Label.ImageContent), tostring(Label.ImageContent.Object == B)
end)
try("clone", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Label = Instance.new("ImageLabel")
	Label.ImageContent = Content.fromObject(Image)
	local Copy = Label:Clone()
	return content(Copy.ImageContent), tostring(Copy.ImageContent.Object == Image)
end)
try("surface-appearance", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Surface = AssetService:CreateSurfaceAppearanceAsync({ ColorMap = Content.fromObject(Image) })
	return content(Surface.ColorMapContent), tostring(Surface.ColorMapContent.Object == Image), content(Surface.NormalMapContent)
end)
try("changed-signal", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Label = Instance.new("ImageLabel")
	local Seen = {}
	Label.Changed:Connect(function(Name)
		table.insert(Seen, Name)
	end)
	Label.ImageContent = Content.fromObject(Image)
	task.wait()
	table.sort(Seen)
	local First = table.concat(Seen, ",")
	table.clear(Seen)
	Image:WritePixelsBuffer(Vector2.zero, Vector2.new(2, 2), buffer.create(16))
	task.wait()
	task.wait()
	table.sort(Seen)
	return First, table.concat(Seen, ",")
end)

-- Decal's Texture and ColorMap
local function show(Value)
	return tostring(Value)
end

try("decal-alias", function()
	local Decal = Instance.new("Decal")
	Decal.ColorMap = "rbxassetid://7"
	local A = Decal.Texture .. " " .. tostring(Decal.TextureContent.Uri) .. " " .. tostring(Decal.ColorMapContent.Uri)
	Decal.Texture = "rbxassetid://8"
	local B = Decal.ColorMap .. " " .. tostring(Decal.TextureContent.Uri) .. " " .. tostring(Decal.ColorMapContent.Uri)
	Decal.TextureContent = Content.fromUri("rbxassetid://9")
	local C = Decal.ColorMap .. " " .. Decal.Texture .. " " .. tostring(Decal.ColorMapContent.Uri)
	return A, B, C
end)
