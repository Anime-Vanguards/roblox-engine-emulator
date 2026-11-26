-- Argument checks taken until now without a measurement: AnalyticsService's Log methods, Chat:Chat,
-- MarketplaceService:PromptSubscriptionPurchase, LocalizationService:GetTranslatorForLocaleAsync and a Translator's
-- methods, given good values and bad ones; the order of a stub's own argument checks and the refusal to wait where a
-- thread cannot yield; BulkMoveTo's bad part against a bad frame at one index; which of an attribute name's checks comes
-- first; the errors of a function the legacy spawn and delay run; and RenderStepped's Wait and ConnectParallel.
local Players = game:GetService("Players")
local LogService = game:GetService("LogService")
local RunService = game:GetService("RunService")
local Server = RunService:IsServer()
P.sync()
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local function Normal(Text)
	Text = string.gsub(Text, Escaped .. ":(%d+):", function(Line)
		return "@" .. (tonumber(Line) - P.base) .. ":"
	end)
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
	elseif Kind == "string" or Kind == "nil" or Kind == "boolean" then
		return tostring(Value)
	end
	return Kind
end
-- one call in a thread of its own (waited for at most half a second), its answer and what the engine logged meanwhile
local function Call(Label, Function, ...)
	local Args = table.pack(...)
	local Result
	task.spawn(function()
		local Results = table.pack(pcall(Function, table.unpack(Args, 1, Args.n)))
		if Results[1] then
			local Parts = {}
			for Index = 2, Results.n do
				table.insert(Parts, Show(Results[Index]))
			end
			Result = "ok " .. table.concat(Parts, " ")
		else
			Result = "error " .. P.err(Results[2])
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
	print(Label .. " " .. (Result or "pending") .. " " .. table.concat(Seen, " "))
	table.clear(Seen)
end

local Player = Players.LocalPlayer or Players:GetPlayers()[1]
print("player " .. tostring(Player ~= nil))
local Analytics = game:GetService("AnalyticsService")
local Calls = {
	{ "LogCustomEvent", { {}, { Player }, { Player, "evt" }, { Player, "evt", 5 }, { Player, "evt", "x" }, { Player, "evt", 5, { a = 1 } }, { Player, "evt", 5, "x" },
		{ Player, 5 }, { nil, "evt" }, { "x", "evt" }, { workspace, "evt" }, { Player, "" }, { Player, string.rep("e", 300) } } },
	{ "LogEconomyEvent", { {}, { Player, Enum.AnalyticsEconomyFlowType.Source, "Coins", 10, 100, "Gameplay" }, { Player, "Source", "Coins", 10, 100, "Gameplay" },
		{ Player, Enum.AnalyticsEconomyFlowType.Sink, "Coins", "x", 100, "Gameplay" }, { Player, Enum.AnalyticsEconomyFlowType.Sink, "Coins", 10 },
		{ Player, 5, "Coins", 10, 100, "Gameplay" }, { Player, Enum.AnalyticsEconomyFlowType.Source, "Coins", 10, 100, "Gameplay", "sku", { a = 1 } },
		{ Player, Enum.AnalyticsEconomyFlowType.Source, "Coins", -10, 100, "Gameplay" } } },
	{ "LogProgressionEvent", { {}, { Player, "path", Enum.AnalyticsProgressionStatus.Begin, 1 }, { Player, "path", "Begin", 1 }, { Player, "path", Enum.AnalyticsProgressionStatus.Begin, "x" },
		{ Player, "path", Enum.AnalyticsProgressionStatus.Begin }, { Player, "path", 5, 1 } } },
	{ "LogFunnelStepEvent", { {}, { Player, "funnel", "session", 1, "step" }, { Player, "funnel" }, { Player, "funnel", "session", "x" }, { Player, 5, "session", 1 } } },
	{ "LogOnboardingFunnelStepEvent", { {}, { Player, 1, "step" }, { Player, 1 }, { Player, "x" }, { Player, 1, 5 }, { nil, 1 } } },
}
for _, Entry in Calls do
	for Index, Args in Entry[2] do
		Call("analytics " .. Entry[1] .. " " .. Index, function(...)
			return Analytics[Entry[1]](Analytics, ...)
		end, table.unpack(Args, 1, 8))
	end
end

-- Chat:Chat
local Chat = game:GetService("Chat")
local Speaker = Instance.new("Part")
Speaker.Anchored = true
Speaker.Position = Vector3.new(500, 20, 500)
Speaker.Parent = workspace
for Index, Args in { {}, { Speaker, "hi" }, { Speaker, "hi", Enum.ChatColor.Red }, { Speaker, "hi", "Red" }, { Speaker, "hi", 5 }, { Speaker }, { Speaker, 5 }, { nil, "hi" }, { 5, "hi" },
	{ Instance.new("Folder"), "hi" }, { Speaker, "" } } do
	Call("chat " .. Index, function(...)
		return Chat:Chat(...)
	end, table.unpack(Args, 1, 3))
end

-- MarketplaceService:PromptSubscriptionPurchase
local Marketplace = game:GetService("MarketplaceService")
for Index, Args in { {}, { Player }, { Player, "EXP-1" }, { Player, 5 }, { nil, "EXP-1" }, { "x", "EXP-1" }, { workspace, "EXP-1" }, { Player, {} } } do
	Call("subscription " .. Index, function(...)
		return Marketplace:PromptSubscriptionPurchase(...)
	end, table.unpack(Args, 1, 2))
end

-- LocalizationService and a Translator (bad values only for the async call: a good one waits for the web)
local Localization = game:GetService("LocalizationService")
for Index, Args in { {}, { nil }, { {} }, { true }, { workspace } } do
	Call("translator-for-locale " .. Index, function(...)
		return Localization:GetTranslatorForLocaleAsync(...)
	end, table.unpack(Args, 1, 1))
end
local Table = Instance.new("LocalizationTable")
Table:SetEntries({ { Key = "k", Source = "Hello {1}", Context = "", Values = { ["fr-fr"] = "Bonjour {1}" } } })
local Translator = Table:GetTranslator("fr-fr")
for Index, Args in { {}, { "k" }, { "k", { "x" } }, { "k", "x" }, { "k", 5 }, { 5, { "x" } }, { nil, { "x" } }, { "missing", {} }, { {}, {} } } do
	Call("format-by-key " .. Index, function(...)
		return Translator:FormatByKey(...)
	end, table.unpack(Args, 1, 2))
end
for Index, Args in { {}, { workspace, "Hello {1}" }, { workspace }, { nil, "Hello {1}" }, { "x", "Hello {1}" }, { workspace, 5 }, { workspace, {} } } do
	Call("translate " .. Index, function(...)
		return Translator:Translate(...)
	end, table.unpack(Args, 1, 2))
end

-- a stub's argument checks against the refusal to wait (a table.sort comparator cannot yield)
local function Sorted(Label, Function)
	local Line
	table.sort({ 2, 1 }, function(A, B)
		if not Line then
			local Ok, Message = pcall(Function)
			Line = if Ok then "ok" else "error " .. P.err(Message)
		end
		return A < B
	end)
	print("refused " .. Label .. " " .. Line)
end
local Assets = game:GetService("AssetService")
local Keyframes = game:GetService("KeyframeSequenceProvider")
local Clips = game:GetService("AnimationClipProvider")
Sorted("CreateEditableMeshAsync-none", function()
	return Assets:CreateEditableMeshAsync()
end)
Sorted("CreateEditableMeshAsync-number", function()
	return Assets:CreateEditableMeshAsync(5)
end)
Sorted("CreateEditableMeshAsync-table", function()
	return Assets:CreateEditableMeshAsync({})
end)
Sorted("CreateEditableMeshAsync-content", function()
	return Assets:CreateEditableMeshAsync(Content.fromAssetId(1))
end)
if Player then
	Sorted("IsInGroup-none", function()
		return Player:IsInGroup()
	end)
	Sorted("IsInGroup-text", function()
		return Player:IsInGroup("x")
	end)
	Sorted("IsInGroup-number", function()
		return Player:IsInGroup(77)
	end)
	Sorted("GetRankInGroup-table", function()
		return Player:GetRankInGroup({})
	end)
end
Sorted("GetKeyframeSequenceAsync-none", function()
	return Keyframes:GetKeyframeSequenceAsync()
end)
Sorted("GetKeyframeSequenceAsync-number", function()
	return Keyframes:GetKeyframeSequenceAsync(5)
end)
Sorted("GetKeyframeSequenceAsync-id", function()
	return Keyframes:GetKeyframeSequenceAsync("rbxassetid://1")
end)
Sorted("GetAnimationClipAsync-none", function()
	return Clips:GetAnimationClipAsync()
end)
Sorted("GetAnimationClipAsync-id", function()
	return Clips:GetAnimationClipAsync("rbxassetid://1")
end)
Sorted("GetTranslatorForLocaleAsync-none", function()
	return Localization:GetTranslatorForLocaleAsync()
end)
Sorted("GetTranslatorForLocaleAsync-table", function()
	return Localization:GetTranslatorForLocaleAsync({})
end)
Sorted("GetTranslatorForLocaleAsync-locale", function()
	return Localization:GetTranslatorForLocaleAsync("fr-fr")
end)

-- a remote call's argument that cannot be sent, against the refusal to wait and the player check
local RemoteFunction = Instance.new("RemoteFunction")
RemoteFunction.Name = "ParityArgsFunction"
RemoteFunction.Parent = game:GetService("ReplicatedStorage")
local RemoteEvent = Instance.new("RemoteEvent")
RemoteEvent.Name = "ParityArgsEvent"
RemoteEvent.Parent = game:GetService("ReplicatedStorage")
local Cyclic = {}
Cyclic.Self = Cyclic
Sorted("InvokeServer-cyclic", function()
	return RemoteFunction:InvokeServer(Cyclic)
end)
Sorted("InvokeServer-function", function()
	return RemoteFunction:InvokeServer(print)
end)
Sorted("InvokeClient-cyclic", function()
	return RemoteFunction:InvokeClient(Player, Cyclic)
end)
Sorted("InvokeClient-nil-cyclic", function()
	return RemoteFunction:InvokeClient(nil, Cyclic)
end)
-- (what the loaders above logged once their loads ended)
task.wait(1)
print("loaders-logged " .. table.concat(Seen, " "))
table.clear(Seen)
Call("FireServer-cyclic", function()
	return RemoteEvent:FireServer(Cyclic)
end)
Call("FireClient-cyclic", function()
	return RemoteEvent:FireClient(Player, Cyclic)
end)
Call("FireClient-nil-cyclic", function()
	return RemoteEvent:FireClient(nil, Cyclic)
end)
Call("FireAllClients-cyclic", function()
	return RemoteEvent:FireAllClients(Cyclic)
end)

-- BulkMoveTo: a bad part and a bad frame at one index
local Moved = Instance.new("Part")
Moved.Anchored = true
Moved.Parent = workspace
local Mode = Enum.BulkMoveMode.FireCFrameChanged
for Index, Case in {
	{ { 5 }, { "x" } },
	{ { Moved, 5 }, { CFrame.new(), "x" } },
	{ { "x" }, { 5 } },
	{ { workspace }, { "x" } },
	{ { Moved }, { "x" } },
	{ { 5 }, { CFrame.new() } },
	{ { Moved, Moved }, { CFrame.new() } },
	{ { Moved }, { CFrame.new(), "x" } },
	{ { {} }, { {} } },
} do
	Call("bulk " .. Index, function()
		return workspace:BulkMoveTo(Case[1], Case[2], Mode)
	end)
end

-- an attribute name that fails several checks
local Holder = Instance.new("Folder")
for _, Name in { "RBX$", "RBX " .. string.rep("a", 100), string.rep("a", 99) .. "$$", "$" .. string.rep("a", 100), "RBX" .. string.rep("a", 98), "rbx$", "RBX", "" } do
	Call("attribute-name " .. #Name .. " " .. string.sub(Name, 1, 6), function()
		return Holder:SetAttribute(Name, 1)
	end)
end

-- the legacy spawn and delay: what a function they run that errors logs
Call("legacy-spawn-error", function()
	spawn(function()
		error("spawned")
	end)
	task.wait(0.2)
end)
Call("legacy-delay-error", function()
	delay(0, function()
		error({ "a table" })
	end)
	task.wait(0.2)
end)
Call("legacy-spawn-thread", function()
	spawn(coroutine.create(function()
		error("in thread", 0)
	end))
	task.wait(0.2)
end)

-- RenderStepped: Wait and ConnectParallel (a server refuses Connect)
Call("renderstepped-wait", function()
	return typeof(RunService.RenderStepped:Wait())
end)
Call("renderstepped-connectparallel", function()
	local Connection = RunService.RenderStepped:ConnectParallel(function() end)
	Connection:Disconnect()
	return "connected"
end)
Call("heartbeat-connectparallel", function()
	local Connection = RunService.Heartbeat:ConnectParallel(function() end)
	Connection:Disconnect()
	return "connected"
end)
Speaker:Destroy()
Moved:Destroy()
RemoteFunction:Destroy()
RemoteEvent:Destroy()
Listener:Disconnect()
