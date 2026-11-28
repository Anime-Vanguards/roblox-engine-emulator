-- IsPropertyModified, ResetPropertyToDefault, GetStyled and GetPropertyChangedSignal on properties a game script lacks a capability
-- for, hidden ones and read-only ones; a key spelled with a lower-case first letter; and a read-only property's write, whose value is
-- cast first (a playing client's and a server's answers are the same).
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
local Folder = Instance.new("Folder")
local Emitter = Instance.new("AudioEmitter")
local Frame = Instance.new("Frame")
local Chat = game:GetService("Chat")
local Http = game:GetService("HttpService")
local Lighting = game:GetService("Lighting")
local Cases = {
	{ "Folder", Folder, { "DataCost", "RobloxLocked", "UniqueId", "SourceAssetId", "Tags", "Attributes", "HistoryId", "ClassName", "Name" } },
	{ "AudioEmitter", Emitter, { "AngleAttenuation" } },
	{ "Frame", Frame, { "AbsoluteSize", "ClippedRect" } },
	{ "Chat", Chat, { "LoadDefaultChat", "IsAutoMigrated" } },
	{ "HttpService", Http, { "HttpEnabled" } },
	{ "Lighting", Lighting, { "LightingStyle", "Technology" } },
}
for _, Case in Cases do
	local Label, Target, Names = Case[1], Case[2], Case[3]
	for _, Name in Names do
		try(Label .. "." .. Name .. " modified", function() return Target:IsPropertyModified(Name) end)
		try(Label .. "." .. Name .. " reset", function() return Target:ResetPropertyToDefault(Name) end)
		try(Label .. "." .. Name .. " styled", function() return typeof(Target:GetStyled(Name)) end)
		try(Label .. "." .. Name .. " signal", function() return typeof(Target:GetPropertyChangedSignal(Name)) end)
	end
end
-- a key spelled with a lower-case first letter, a method call of one, and a key a child of that name answers otherwise
try("lower dataCost", function() return (Folder :: any).dataCost end)
try("lower robloxLocked write", function() (Folder :: any).robloxLocked = true end)
try("call DataCost", function() return (Folder :: any):DataCost() end)
try("call LoadDefaultChat", function() return (Chat :: any):LoadDefaultChat() end)
-- read-only writes: the value's type is checked first?
local Part = Instance.new("Part")
try("ClassName table", function() (Folder :: any).ClassName = {} end)
try("ClassName number", function() (Folder :: any).ClassName = 5 end)
try("AbsoluteSize table", function() (Frame :: any).AbsoluteSize = {} end)
try("AbsoluteSize number", function() (Frame :: any).AbsoluteSize = 5 end)
try("AbsoluteSize valid", function() (Frame :: any).AbsoluteSize = Vector2.zero end)
try("AssemblyMass table", function() (Part :: any).AssemblyMass = {} end)
try("AssemblyMass string", function() (Part :: any).AssemblyMass = "x" end)
try("ResizeableFaces number", function() (Part :: any).ResizeableFaces = 1 end)
try("ResizeIncrement string", function() (Part :: any).ResizeIncrement = "x" end)
try("DistributedGameTime string", function() (workspace :: any).DistributedGameTime = "x" end)
try("Terrain IsSmooth table", function() (workspace.Terrain :: any).IsSmooth = {} end)
try("TextBounds table", function() (Instance.new("TextLabel") :: any).TextBounds = {} end)
try("AbsoluteSize nil", function() (Frame :: any).AbsoluteSize = nil end)
try("PlaceId string", function() (game :: any).PlaceId = "x" end)
try("PlaceId table", function() (game :: any).PlaceId = {} end)
try("LoadDefaultChat table", function() (Chat :: any).LoadDefaultChat = {} end)
try("Technology table", function() (Lighting :: any).Technology = {} end)
