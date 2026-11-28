-- Decal's Texture, TextureContent, ColorMap and ColorMapContent: what each write fires and what the four read after it, an
-- EditableImage shown through one, and a clone (a playing client's and a server's answers are the same).
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

-- Decal: what each write fires and what the four read after it
local Four = { "ColorMap", "Texture", "ColorMapContent", "TextureContent" }
local function state(Decal)
	return Decal.ColorMap .. "," .. Decal.Texture .. "," .. tostring(Decal.ColorMapContent.Uri) .. "," .. tostring(Decal.TextureContent.Uri)
end
for _, ClassName in { "Decal", "Texture" } do
	for _, Written in Four do
		try(ClassName .. " write " .. Written, function()
			local Decal = Instance.new(ClassName)
			local Seen = {}
			Decal.Changed:Connect(function(Name)
				table.insert(Seen, "C:" .. Name)
			end)
			for _, Name in Four do
				Decal:GetPropertyChangedSignal(Name):Connect(function()
					table.insert(Seen, "S:" .. Name)
				end)
			end
			local Value = if string.find(Written, "Content") then Content.fromUri("rbxassetid://11") else "rbxassetid://11"
			Decal[Written] = Value
			task.wait()
			local First = table.concat(Seen, ",")
			table.clear(Seen)
			Decal[Written] = Value
			task.wait()
			local Again = table.concat(Seen, ",")
			return state(Decal), First, Again
		end)
	end
	try(ClassName .. " defaults", function()
		local Decal = Instance.new(ClassName)
		return state(Decal), content(Decal.ColorMapContent), content(Decal.TextureContent)
	end)
	try(ClassName .. " image via ColorMapContent", function()
		local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
		local Decal = Instance.new(ClassName)
		Decal.ColorMapContent = Content.fromObject(Image)
		return content(Decal.ColorMapContent), content(Decal.TextureContent), Decal.ColorMap, Decal.Texture,
			tostring(Decal.TextureContent.Object == Image)
	end)
	try(ClassName .. " image via TextureContent then ColorMap", function()
		local Image = AssetService:CreateEditableImage({ Size = Vector2.new(2, 2) })
		local Decal = Instance.new(ClassName)
		Decal.TextureContent = Content.fromObject(Image)
		local Before = content(Decal.ColorMapContent)
		Decal.ColorMap = "rbxassetid://12"
		return Before, content(Decal.TextureContent), Decal.Texture
	end)
	try(ClassName .. " clone", function()
		local Decal = Instance.new(ClassName)
		Decal.ColorMap = "rbxassetid://13"
		local Copy = Decal:Clone()
		return state(Copy)
	end)
end
