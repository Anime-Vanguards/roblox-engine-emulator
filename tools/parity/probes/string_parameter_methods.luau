-- Instance and service methods with a string parameter, each given values of every kind: what the call answers, and the
-- warning the engine logs before it casts a value that is neither a string nor a number (`argument #n expects a string, but
-- <type> was passed`, with the calling script's stack). The lookups print what they found, so the text a value was cast to
-- shows: the holder has a child, an attribute and a tag named by every text a value may become. Each call runs in a thread of
-- its own, so a warning's stack holds the probe's frames only. Where the engine answers a string differently from what is
-- checked here (an unknown property, a core setting, a teleport setting, a text's width), only the warning is printed.
local LogService = game:GetService("LogService")
local RunService = game:GetService("RunService")
local CollectionService = game:GetService("CollectionService")
local HttpService = game:GetService("HttpService")
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
	if Kind == Enum.MessageType.MessageWarning and (string.sub(Message, 1, 10) == "argument #" or string.sub(Message, 1, 14) == "Infinite yield") then
		table.insert(Seen, "[" .. Normal(Message) .. "]")
	end
end)

local function Show(Value)
	local Kind = typeof(Value)
	if Kind == "Instance" then
		return Value.ClassName .. " '" .. Value.Name .. "'"
	elseif Kind == "number" then
		return P.num(Value)
	elseif Kind == "string" then
		return "'" .. Value .. "'"
	elseif Kind == "nil" or Kind == "boolean" then
		return tostring(Value)
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

-- the values (a subnormal made at run time: the compiler folds constants in the flushing float mode)
local Tiny = tonumber("1e-310")
local NegativeZero = tonumber("-0")
print("flushes", Tiny * 3 == 0)
local Nothing = newproxy()
local All = {
	{ "int", 5 },
	{ "tenth", 0.1 },
	{ "negzero", NegativeZero },
	{ "subnormal", Tiny },
	{ "nan", 0 / 0 },
	{ "inf", math.huge },
	{ "neginf", -math.huge },
	{ "2^63", 2 ^ 63 },
	{ "1e20", 1e20 },
	{ "true", true },
	{ "false", false },
	{ "nil", nil },
	{ "none", Nothing },
	{ "table", {} },
	{ "instance", workspace },
	{ "vector", Vector3.new(0.1, 1 / 3, 2) },
	{ "vector-nan", Vector3.new(0 / 0, math.huge, -math.huge) },
	{ "enumitem", Enum.RenderPriority.Camera },
	{ "enum", Enum.RenderPriority },
	{ "enums", Enum },
	{ "cfunction", print },
	{ "function", function() end },
	{ "thread", coroutine.create(function() end) },
	{ "newproxy", newproxy() },
	{ "signal", workspace.Changed },
	{ "buffer", buffer.create(2) },
	{ "cframe", CFrame.new(1, 2, 3) },
	{ "color3", Color3.new(0.1, 0.2, 0.3) },
	{ "udim2", UDim2.new(1, 2, 3, 4) },
	{ "vector2", Vector2.new(0.1, 2) },
	{ "brickcolor", BrickColor.new("Bright red") },
}
local Some = {}
for _, Case in All do
	if table.find({ "int", "tenth", "true", "nil", "none", "table", "instance", "vector", "enumitem", "enum", "signal" }, Case[1]) then
		table.insert(Some, Case)
	end
end

-- the texts a value may become, each the name of a child, an ancestor, an attribute (where it may name one) and a tag
local Texts = {
	"5", "0.10000000000000000555", "-0", "9.9999999999999694493e-311", "NAN", "INF", "-INF", "9223372036854775808",
	"1e+20", "true", "false", "{}", "Instance", "0.100000001, 0.333333343, 2", "NAN, INF, -INF", "Camera", "nil",
	"function", "EventInstance", "buffer { Size = 2 }", "1, 2, 3, 1, 0, 0, 0, 1, 0, 0, 0, 1", "0.1, 0.2, 0.3",
	"{1, 2}, {3, 4}", "0.100000001, 2", "Bright red",
}
local Holder = Instance.new("Folder")
Holder.Name = "Holder"
local Root = Instance.new("Camera")
Root.Name = "Root"
Root.Parent = Holder
local Leaf = Root
for _, Text in Texts do
	local Child = Instance.new("Folder")
	Child.Name = Text
	Child.Parent = Holder
	pcall(Holder.SetAttribute, Holder, Text, Text)
	Holder:AddTag(Text)
	local Node = Instance.new("Folder")
	Node.Name = Text
	Node.Parent = Leaf
	Leaf = Node
end
local Last = Instance.new("Folder")
Last.Name = "Last"
Last.Parent = Leaf
Leaf = Last
Holder.Parent = workspace

-- one call in a thread of its own (waited for at most half a second), then what the engine warned
local function Call(Function, Case)
	local Result
	task.spawn(function()
		if Case[2] == Nothing then
			Result = Outcome(pcall(Function))
		else
			Result = Outcome(pcall(Function, Case[2]))
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
-- every value given to Function: the answer (and Reveal's line, after it), or with Quiet the warning and a missing argument only
local function Run(Label, Values, Function, Reveal, Setup, Quiet)
	for _, Case in Values do
		if Setup then
			Setup()
		end
		local Result, Log = Call(Function, Case)
		if Quiet and not string.find(Result, "missing") then
			Result = "-"
		end
		if Reveal then
			Result ..= " | " .. Reveal()
		end
		print(Label, Case[1], Result, Log)
	end
end

local Fresh
local function NewFresh()
	if Fresh then
		Fresh:Destroy()
	end
	Fresh = Instance.new("Folder")
	Fresh.Name = "Fresh"
end
local function Tags()
	local Names = {}
	for _, Tag in Fresh:GetTags() do
		table.insert(Names, "'" .. Tag .. "'")
	end
	table.sort(Names)
	return "tags " .. table.concat(Names, ",")
end
local function Attributes()
	local Names = {}
	for Key in Fresh:GetAttributes() do
		table.insert(Names, "'" .. Key .. "'")
	end
	table.sort(Names)
	return "attributes " .. table.concat(Names, ",")
end
local function Tagged()
	NewFresh()
	Fresh:AddTag("5")
	Fresh:AddTag("true")
	Fresh:AddTag("Instance")
end

-- the base methods
Run("FindFirstChild", All, function(...)
	return Holder:FindFirstChild(...)
end)
Run("findFirstChild", Some, function(...)
	return Holder:findFirstChild(...)
end)
Run("WaitForChild", Some, function(...)
	if select("#", ...) == 0 then
		return Holder:WaitForChild()
	end
	return Holder:WaitForChild(..., 0.05)
end)
Run("FindFirstAncestor", Some, function(...)
	return Leaf:FindFirstAncestor(...)
end)
Run("FindFirstAncestorOfClass", Some, function(...)
	return Leaf:FindFirstAncestorOfClass(...)
end)
Run("FindFirstAncestorWhichIsA", Some, function(...)
	return Leaf:FindFirstAncestorWhichIsA(...)
end)
Run("FindFirstChildOfClass", Some, function(...)
	return Holder:FindFirstChildOfClass(...)
end)
Run("FindFirstChildWhichIsA", Some, function(...)
	return Holder:FindFirstChildWhichIsA(...)
end)
Run("IsA", Some, function(...)
	return Root:IsA(...)
end)
Run("isA", Some, function(...)
	return Root:isA(...)
end)
Run("FindFirstDescendant", Some, function(...)
	return Holder:FindFirstDescendant(...)
end)
Run("GetAttribute", All, function(...)
	return Holder:GetAttribute(...)
end)
Run("SetAttribute", Some, function(...)
	return Fresh:SetAttribute(..., 1)
end, Attributes, NewFresh)
Run("GetAttributeChangedSignal", Some, function(...)
	return Holder:GetAttributeChangedSignal(...)
end)
Run("GetPropertyChangedSignal", All, function(...)
	return Holder:GetPropertyChangedSignal(...)
end)
Run("AddTag", All, function(...)
	return Fresh:AddTag(...)
end, Tags, NewFresh)
Run("HasTag", Some, function(...)
	return Holder:HasTag(...)
end)
Run("RemoveTag", Some, function(...)
	return Fresh:RemoveTag(...)
end, Tags, Tagged)
Run("IsPropertyModified", Some, function(...)
	return Holder:IsPropertyModified(...)
end, nil, nil, true)
Run("ResetPropertyToDefault", Some, function(...)
	return Holder:ResetPropertyToDefault(...)
end, nil, nil, true)
Run("GetStyled", Some, function(...)
	return Holder:GetStyled(...)
end, nil, nil, true)

-- the stack by how the method is called, and two values cast in one call (the last argument first)
local Stack = { "true", true }
Run("stack-direct", { Stack }, function(...)
	return pcall(Holder.FindFirstChild, Holder, ...)
end)
local function Named(...)
	return Holder:FindFirstChild(...)
end
local function Outer(...)
	return Named(...)
end
Run("stack-nested", { Stack }, Outer)
Run("stack-dot", { Stack }, function(...)
	return Holder.FindFirstChild(Holder, ...)
end)
Run("CollisionGroupsAreCollidable", { Stack }, function(...)
	return game:GetService("PhysicsService"):CollisionGroupsAreCollidable(..., {})
end)

-- services
Run("GetService", All, function(...)
	return game:GetService(...)
end)
Run("service", Some, function(...)
	return game:service(...)
end)
Run("FindService", Some, function(...)
	return game:FindService(...)
end)
Run("GetService", { { "chat-enumitem", Enum.CoreGuiType.Chat }, { "material-enumitem", Enum.Material.Plastic } }, function(...)
	return game:GetService(...)
end)
Run("FindService", { { "chat-enumitem", Enum.CoreGuiType.Chat } }, function(...)
	return game:FindService(...)
end)
Run("CS.AddTag", All, function(...)
	return CollectionService:AddTag(Fresh, ...)
end, Tags, NewFresh)
Run("CS.HasTag", Some, function(...)
	return CollectionService:HasTag(Holder, ...)
end)
Run("CS.RemoveTag", Some, function(...)
	return CollectionService:RemoveTag(Fresh, ...)
end, Tags, Tagged)
Run("CS.GetTagged", Some, function(...)
	return #CollectionService:GetTagged(...)
end)
Run("CS.GetInstanceAddedSignal", Some, function(...)
	return CollectionService:GetInstanceAddedSignal(...)
end)
Run("CS.GetInstanceRemovedSignal", Some, function(...)
	return CollectionService:GetInstanceRemovedSignal(...)
end)
Run("HttpService.JSONDecode", Some, function(...)
	return HttpService:JSONDecode(...)
end)
Run("HttpService.UrlEncode", { All[1], All[10], All[12], All[13], All[14], All[15], All[18], All[19], All[25] }, function(...)
	return HttpService:UrlEncode(...)
end)
Run("ReflectionService.GetClass", Some, function(...)
	local Class = game:GetService("ReflectionService"):GetClass(...)
	return Class and Class.Name
end)
Run("SharedTableRegistry.GetSharedTable", Some, function(...)
	return game:GetService("SharedTableRegistry"):GetSharedTable(...)
end)
Run("PhysicsService.IsCollisionGroupRegistered", Some, function(...)
	return game:GetService("PhysicsService"):IsCollisionGroupRegistered(...)
end)
Run("RunService.BindToRenderStep", Some, function(...)
	RunService:BindToRenderStep(..., 1, function() end)
	RunService:UnbindFromRenderStep(...)
	return "bound"
end)
Run("RunService.UnbindFromRenderStep", Some, function(...)
	return RunService:UnbindFromRenderStep(...)
end)
Run("GuiService.RemoveSelectionGroup", Some, function(...)
	return game:GetService("GuiService"):RemoveSelectionGroup(...)
end)
local ContextActionService = game:GetService("ContextActionService")
Run("CAS.UnbindAction", Some, function(...)
	return ContextActionService:UnbindAction(...)
end)
Run("CAS.GetBoundActionInfo", Some, function(...)
	local Info = ContextActionService:GetBoundActionInfo(...)
	return next(Info) == nil
end)
Run("CAS.BindAction", Some, function(...)
	return ContextActionService:BindAction(..., function() end, false)
end, nil, nil, true)
Run("StarterGui.SetCore", Some, function(...)
	return game:GetService("StarterGui"):SetCore(..., nil)
end, nil, nil, true)
-- (GetCore waits on a server until the core scripts register the name, and they never do there)
if not Server then
	Run("StarterGui.GetCore", Some, function(...)
		return game:GetService("StarterGui"):GetCore(...)
	end, nil, nil, true)
end
Run("TeleportService.GetTeleportSetting", Some, function(...)
	return game:GetService("TeleportService"):GetTeleportSetting(...)
end, nil, nil, true)
Run("TextService.GetTextSize", Some, function(...)
	return typeof(game:GetService("TextService"):GetTextSize(..., 14, Enum.Font.SourceSans, Vector2.new(1000, 1000)))
end)
local Curve = Instance.new("MarkerCurve")
Run("MarkerCurve.InsertMarkerAtTime", Some, function(...)
	return Curve:InsertMarkerAtTime(1, ...)
end, nil, nil, true)
-- (a track of an id that names no asset: a client's, as a server's LoadAnimation of one raises)
local Made, Track = false, nil
if not Server then
	Made, Track = pcall(function()
		local Model = Instance.new("Model")
		local Controller = Instance.new("AnimationController")
		Controller.Parent = Model
		local Animator = Instance.new("Animator")
		Animator.Parent = Controller
		Model.Parent = workspace
		local Animation = Instance.new("Animation")
		Animation.AnimationId = "rbxassetid://0"
		return Animator:LoadAnimation(Animation)
	end)
	print("track", Made)
end
if Made then
	Run("AnimationTrack.GetMarkerReachedSignal", Some, function(...)
		return Track:GetMarkerReachedSignal(...)
	end)
end

-- CollectionService's tag methods given no tag at all
Run("CS.AddTag-instance-only", { { "fresh", true } }, function()
	return CollectionService:AddTag(Fresh)
end, Tags, NewFresh)
Run("CS.RemoveTag-instance-only", { { "tagged", true } }, function()
	return CollectionService:RemoveTag(Fresh)
end, Tags, Tagged)
Run("CS.HasTag-instance-only", { { "untagged", Root }, { "tagged", Holder } }, function(Target)
	return CollectionService:HasTag(Target)
end)

-- WaitForChild's infinite-yield warning names the text
task.spawn(function()
	Holder:WaitForChild(Vector3.new(0.5, 0.25, 4))
end)
task.spawn(function()
	Holder:WaitForChild(1.5)
end)
task.wait(6)
print("infinite-yield", table.concat(Seen, " "))
table.clear(Seen)
Listener:Disconnect()
Holder:Destroy()
