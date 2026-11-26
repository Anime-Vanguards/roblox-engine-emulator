-- Service methods a game script calls, given values of each kind: what each answers or raises, and what the engine warns.
-- Players:GetPlayerByUserId, CollectionService's tag methods and signals, RunService:UnbindFromRenderStep of a shared name,
-- HttpService:UrlEncode of every byte, Instance:QueryDescendants' selector, the methods that take a ContentId, and StarterGui's
-- core GUI switches and GetCore / SetCore. Each call runs in a thread of its own (waited for a few frames), so a call that
-- yields shows as pending, and a warning's stack holds the probe's frames only.
local LogService = game:GetService("LogService")
local RunService = game:GetService("RunService")
local Players = game:GetService("Players")
local CollectionService = game:GetService("CollectionService")
local HttpService = game:GetService("HttpService")
local StarterGui = game:GetService("StarterGui")
local Server = RunService:IsServer()
P.sync()

local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local function Normal(Text)
	Text = string.gsub(Text, "Script '" .. Escaped .. "', Line (%d+)", function(Line)
		return "Script '<probe>', Line @" .. (tonumber(Line) - P.base)
	end)
	Text = string.gsub(Text, "\n%s*", " / ")
	return (string.gsub(Text, "0x%x+", "ADDR"))
end
local Seen = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageWarning or Kind == Enum.MessageType.MessageError then
		table.insert(Seen, "[" .. Normal(Message) .. "]")
	end
end)

local function Show(Value)
	local Kind = typeof(Value)
	if Kind == "Instance" then
		return Value.ClassName
	elseif Kind == "number" then
		return P.num(Value)
	elseif Kind == "string" then
		return "'" .. Value .. "'"
	elseif Kind == "nil" or Kind == "boolean" then
		return tostring(Value)
	elseif Kind == "EnumItem" then
		return tostring(Value)
	elseif Kind == "RBXScriptSignal" then
		return "signal " .. tostring(Value)
	elseif Kind == "table" then
		return "table " .. #Value
	end
	return Kind
end
local function Outcome(...)
	local Results = table.pack(...)
	if not Results[1] then
		return "error " .. P.err(Results[2])
	end
	local Parts = {}
	for Index = 2, Results.n do
		table.insert(Parts, Show(Results[Index]))
	end
	return "ok " .. table.concat(Parts, " ")
end
local Nothing = newproxy()
-- one call in a thread of its own (waited for at most 30 frames), then what the engine warned
local function Call(Function, Value)
	local Result
	task.spawn(function()
		if Value == Nothing then
			Result = Outcome(pcall(Function))
		else
			Result = Outcome(pcall(Function, Value))
		end
	end)
	for _ = 1, 30 do
		if Result then
			break
		end
		task.wait()
	end
	task.wait()
	task.wait()
	local Log = table.concat(Seen, " ")
	table.clear(Seen)
	return Result or "pending", Log
end
local function Run(Label, Values, Function)
	for _, Case in Values do
		local Result, Log = Call(Function, Case[2])
		print(Label, Case[1], Result, Log)
	end
end
local Kinds = {
	{ "string", "x" },
	{ "numeric", "5" },
	{ "empty", "" },
	{ "int", 5 },
	{ "fraction", 1.5 },
	{ "negative", -1 },
	{ "nan", 0 / 0 },
	{ "huge", 2 ^ 63 },
	{ "true", true },
	{ "nil", nil },
	{ "none", Nothing },
	{ "table", {} },
	{ "dict", { a = 1 } },
	{ "instance", workspace },
	{ "vector", Vector3.new(1, 2, 3) },
	{ "enumitem", Enum.Material.Plastic },
	{ "function", print },
}

-- Players:GetPlayerByUserId
-- (true and false are read as 1 and 0: whether a player has the id 1 is the session's, so those print whether they found it)
local Ids = {}
for _, Case in Kinds do
	if Case[1] ~= "true" then
		table.insert(Ids, Case)
	end
end
table.insert(Ids, { "false", false })
Run("GetPlayerByUserId", Ids, function(...)
	return Players:GetPlayerByUserId(...)
end)
print("GetPlayerByUserId bool", Call(function()
	return Players:GetPlayerByUserId(true) == Players:GetPlayerByUserId(1), Players:GetPlayerByUserId(false) == Players:GetPlayerByUserId(0)
end, Nothing))
local Own = if Server then nil else Players.LocalPlayer
if Own then
	print("GetPlayerByUserId own", Players:GetPlayerByUserId(Own.UserId) == Own, Call(function()
		return Players:GetPlayerByUserId(tostring(Own.UserId)) == Own
	end, Nothing))
end

-- CollectionService's tag methods given a target that is no Instance, then the service's own tags (cleared after each)
local function ServiceTags()
	local Tags = game.GetTags(CollectionService)
	for _, Tag in Tags do
		game.RemoveTag(CollectionService, Tag)
	end
	table.sort(Tags)
	return "own {" .. table.concat(Tags, ",") .. "}"
end
for _, Method in { "AddTag", "RemoveTag", "HasTag", "GetTags" } do
	for _, Case in {
		{ "nil", nil },
		{ "int", 5 },
		{ "string", "x" },
		{ "table", {} },
		{ "vector", Vector3.new() },
		{ "enumitem", Enum.Material.Plastic },
	} do
		if Method == "RemoveTag" or Method == "HasTag" then
			-- (the service holding the tags a call could reach)
			for _, Tag in { "t", "5", "x", "{}", "0, 0, 0", "Plastic", "nil" } do
				game.AddTag(CollectionService, Tag)
			end
		end
		local Result, Log = Call(function(Target)
			return CollectionService[Method](CollectionService, Target, "t")
		end, Case[2])
		print("CS." .. Method, Case[1], Result, Log, ServiceTags())
	end
	print("CS." .. Method, "nothing", Call(function()
		return CollectionService[Method](CollectionService)
	end, Nothing))
end
-- HasTag of an instance with no tag at all, in turn
local Untagged, Tagged = Instance.new("Folder"), Instance.new("Folder")
Tagged:AddTag("t")
for Round = 1, 3 do
	print("CS.HasTag instance-only", Round, Call(function()
		return CollectionService:HasTag(Untagged)
	end, Nothing), Call(function()
		return CollectionService:HasTag(Tagged)
	end, Nothing))
end
-- the tag signals' text and identity, and the tags they take
for _, Case in { { "x", "x" }, { "space", "a b" }, { "empty", "" }, { "int", 5 }, { "nil", nil }, { "none", Nothing }, { "table", {} } } do
	for _, Method in { "GetInstanceAddedSignal", "GetInstanceRemovedSignal" } do
		local Result, Log = Call(function(...)
			local Signal = CollectionService[Method](CollectionService, ...)
			return tostring(Signal), Signal == CollectionService[Method](CollectionService, ...)
		end, Case[2])
		print("CS." .. Method, Case[1], Result, Log)
	end
end
print("CS.TagAdded", tostring(CollectionService.TagAdded), tostring(CollectionService.TagRemoved))

-- RunService:UnbindFromRenderStep of a name one, two or three bindings share
local Ran = 0
local function Bind(Name, Count, Priority)
	for Index = 1, Count do
		RunService:BindToRenderStep(Name, Priority or Index, function()
			Ran += 1
		end)
	end
end
for _, Case in { { "one", 1 }, { "two", 2 }, { "three", 3 }, { "same-priority", 2, 5 } } do
	Bind("u-" .. Case[1], Case[2], Case[3])
	print("Unbind", Case[1], Call(function()
		return RunService:UnbindFromRenderStep("u-" .. Case[1])
	end, Nothing))
	print("Unbind again", Case[1], Call(function()
		return RunService:UnbindFromRenderStep("u-" .. Case[1])
	end, Nothing))
end
print("Unbind never", Call(function()
	return RunService:UnbindFromRenderStep("u-never")
end, Nothing))
Bind("u-run", 2)
RunService:UnbindFromRenderStep("u-run")
Ran = 0
for _ = 1, 5 do
	task.wait()
end
-- (no binding is left to run; the warning reached the log)
print("Unbind leaves", Ran, table.concat(Seen, " "))
table.clear(Seen)

-- HttpService:UrlEncode: each byte that is not written as %XX, and some strings
local Kept = {}
for Byte = 0, 255 do
	local Encoded = HttpService:UrlEncode(string.char(Byte))
	if Encoded ~= string.format("%%%02X", Byte) then
		table.insert(Kept, string.format("%02X=%s", Byte, Encoded))
	end
end
print("UrlEncode kept", table.concat(Kept, " "))
for _, Text in { "0.1-0", "a b+c", "~_.-*'()!", "\u{e9}", "" } do
	print("UrlEncode", HttpService:UrlEncode(Text))
end
Run("UrlEncode", { { "int", 5 }, { "fraction", -0.5 }, { "true", true }, { "nil", nil }, { "none", Nothing } }, function(...)
	return HttpService:UrlEncode(...)
end)

-- Instance:QueryDescendants' selector
local Root = Instance.new("Folder")
Instance.new("Part").Parent = Root
local Named = Instance.new("Folder")
Named.Name = "5"
Named.Parent = Root
Run("QueryDescendants", {
	{ "class", "Part" },
	{ "empty", "" },
	{ "int", 5 },
	{ "true", true },
	{ "false", false },
	{ "nil", nil },
	{ "none", Nothing },
	{ "table", {} },
	{ "instance", workspace },
	{ "vector", Vector3.new() },
	{ "enumitem", Enum.Material.Plastic },
}, function(...)
	return Root:QueryDescendants(...)
end)
print("QueryDescendants method", Call(function()
	return Root.QueryDescendants(Root, true)
end, Nothing))

-- methods that take a ContentId (no strings but the first's: the others load the asset)
local ContentProvider = game:GetService("ContentProvider")
local NotStrings = {
	{ "int", 5 },
	{ "true", true },
	{ "nil", nil },
	{ "none", Nothing },
	{ "table", {} },
	{ "instance", workspace },
	{ "vector", Vector3.new() },
	{ "enumitem", Enum.Material.Plastic },
	{ "content", Content.fromUri("rbxassetid://1") },
}
Run("GetAssetFetchStatus", { { "string", "rbxassetid://1" }, { "empty", "" }, table.unpack(NotStrings) }, function(...)
	return ContentProvider:GetAssetFetchStatus(...)
end)
Run("GetAssetFetchStatusChangedSignal", NotStrings, function(...)
	return ContentProvider:GetAssetFetchStatusChangedSignal(...)
end)
Run("TextService.GetFamilyInfoAsync", NotStrings, function(...)
	return game:GetService("TextService"):GetFamilyInfoAsync(...)
end)
Run("KeyframeSequenceProvider.GetKeyframeSequenceAsync", NotStrings, function(...)
	return game:GetService("KeyframeSequenceProvider"):GetKeyframeSequenceAsync(...)
end)
Run("AnimationClipProvider.GetAnimationClipAsync", NotStrings, function(...)
	return game:GetService("AnimationClipProvider"):GetAnimationClipAsync(...)
end)
Run("InsertService.CreateMeshPartAsync", NotStrings, function(...)
	return game:GetService("InsertService"):CreateMeshPartAsync(..., Enum.CollisionFidelity.Box, Enum.RenderFidelity.Automatic)
end)

-- StarterGui's core GUI switches
local Types = Enum.CoreGuiType:GetEnumItems()
local function Switches()
	local Parts = {}
	for _, Item in Types do
		local Ok, Enabled = pcall(function()
			return StarterGui:GetCoreGuiEnabled(Item)
		end)
		table.insert(Parts, Item.Name .. "=" .. (if Ok then tostring(Enabled) else "error " .. P.err(Enabled)))
	end
	return table.concat(Parts, " ")
end
print("CoreGui start", Switches())
for _, Step in {
	{ Enum.CoreGuiType.Chat, false },
	{ Enum.CoreGuiType.All, false },
	{ Enum.CoreGuiType.Health, true },
	{ Enum.CoreGuiType.All, true },
	{ Enum.CoreGuiType.PlayerList, false },
} do
	print("SetCoreGuiEnabled", Step[1].Name, Step[2], Call(function()
		return StarterGui:SetCoreGuiEnabled(Step[1], Step[2])
	end, Nothing))
	print("CoreGui", Switches())
end
for _, Case in {
	{ "string", "Chat", false },
	{ "int", 3, false },
	{ "huge", 99, false },
	{ "nil", nil, false },
	{ "enable-nil", Enum.CoreGuiType.Health, nil },
	{ "enable-string", Enum.CoreGuiType.Health, "x" },
	{ "enable-int", Enum.CoreGuiType.Health, 0 },
	{ "other-enum", Enum.Material.Plastic, false },
	{ "table", {}, false },
} do
	print("SetCoreGuiEnabled", Case[1], Call(function()
		return StarterGui:SetCoreGuiEnabled(Case[2], Case[3])
	end, Nothing))
	print("CoreGui", Switches())
end
StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.All, true)
Run("GetCoreGuiEnabled", {
	{ "string", "Chat" },
	{ "int", 3 },
	{ "huge", 99 },
	{ "nil", nil },
	{ "none", Nothing },
	{ "other-enum", Enum.Material.Plastic },
	{ "table", {} },
}, function(...)
	return StarterGui:GetCoreGuiEnabled(...)
end)

-- GetCore and SetCore of names the core scripts register and of one they do not
local CoreNames = {
	"Nope", "ChatActive", "PointsNotificationsActive", "BadgesNotificationsActive", "GetBlockedUserIds", "PlayerBlockedEvent",
	"PlayerUnblockedEvent", "PlayerMutedEvent", "PlayerUnmutedEvent", "PlayerFriendedEvent", "PlayerUnfriendedEvent",
	"DevConsoleVisible", "ChatWindowPosition", "ChatWindowSize", "ChatBarDisabled", "AvatarContextMenuEnabled",
	"VREnableControllerModels", "VRLaserPointerMode", "TopbarEnabled", "SendNotification", "ResetButtonCallback",
	"ChatMakeSystemMessage", "CoreGuiChatConnections", "AvatarContextMenuTheme", "AvatarContextMenuTarget",
	"AddAvatarContextMenuOption", "RemoveAvatarContextMenuOption", "PromptSendFriendRequest", "PromptUnfriend",
	"PromptBlockPlayer", "PromptUnblockPlayer", "EmotesMenuOpen", "VRRotationIntensity", "UserGameSettings", "",
}
if not Server then
	for _, Name in CoreNames do
		print("GetCore", Name, Call(function()
			return StarterGui:GetCore(Name)
		end, Nothing))
	end
end
for _, Name in { "Nope", "", "ChatActive", "GetBlockedUserIds", "PointsNotificationsActive" } do
	print("SetCore", Name, Call(function()
		return StarterGui:SetCore(Name, true)
	end, Nothing))
end
Listener:Disconnect()
