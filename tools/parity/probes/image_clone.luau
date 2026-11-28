-- A clone of an instance one of whose properties shows an EditableImage: the copy shows the image too (a playing client's and a
-- server's answers are the same).
local AssetService = game:GetService("AssetService")
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
local function content(Value)
	if typeof(Value) ~= "Content" then
		return typeof(Value) .. ":" .. tostring(Value)
	end
	local Object = Value.Object
	return tostring(Value.SourceType) .. " uri=" .. tostring(Value.Uri) .. " object=" .. (if Object then Object.ClassName else "nil")
end

-- a clone of a property showing an EditableImage
try("clone label", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Label = Instance.new("ImageLabel")
	Label.ImageContent = Content.fromObject(Image)
	local Copy = Label:Clone()
	local Line = { content(Copy.ImageContent), tostring(Copy.ImageContent.Object == Image), "string=" .. Copy.Image }
	Image:WritePixelsBuffer(Vector2.zero, Vector2.new(2, 2), buffer.create(16))
	task.wait()
	table.insert(Line, "written=" .. content(Copy.ImageContent))
	Copy.Image = "rbxassetid://5"
	table.insert(Line, "copy-string=" .. content(Copy.ImageContent) .. " original=" .. content(Label.ImageContent))
	return table.concat(Line, " ")
end)
try("clone after destroy", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Label = Instance.new("ImageLabel")
	Label.ImageContent = Content.fromObject(Image)
	Image:Destroy()
	task.wait()
	local Copy = Label:Clone()
	return content(Copy.ImageContent), "string=" .. Copy.Image
end)
try("clone nested", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Frame = Instance.new("Frame")
	local Button = Instance.new("ImageButton")
	Button.HoverImageContent = Content.fromObject(Image)
	Button.Parent = Frame
	local Copy = Frame:Clone()
	local Inner = Copy:FindFirstChildOfClass("ImageButton")
	return content(Inner.HoverImageContent), tostring(Inner.HoverImageContent.Object == Image), content(Inner.ImageContent)
end)
try("clone decal colormap", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Decal = Instance.new("Decal")
	Decal.ColorMapContent = Content.fromObject(Image)
	local Copy = Decal:Clone()
	return content(Copy.ColorMapContent), content(Copy.TextureContent), tostring(Copy.TextureContent.Object == Image)
end)
try("clone mesh part", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Part = Instance.new("MeshPart")
	Part.TextureContent = Content.fromObject(Image)
	local Copy = Part:Clone()
	return content(Copy.TextureContent), tostring(Copy.TextureContent.Object == Image), "string=" .. Copy.TextureID
end)
try("clone surface appearance", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Surface = AssetService:CreateSurfaceAppearanceAsync({ ColorMap = Content.fromObject(Image) })
	local Copy = Surface:Clone()
	return content(Copy.ColorMapContent), tostring(Copy.ColorMapContent.Object == Image)
end)
try("clone changed", function()
	local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
	local Label = Instance.new("ImageLabel")
	Label.ImageContent = Content.fromObject(Image)
	local Copy = Label:Clone()
	local Seen = {}
	Copy.Changed:Connect(function(Name)
		table.insert(Seen, Name)
	end)
	Image:WritePixelsBuffer(Vector2.zero, Vector2.new(2, 2), buffer.create(16))
	task.wait()
	task.wait()
	return table.concat(Seen, ",")
end)
