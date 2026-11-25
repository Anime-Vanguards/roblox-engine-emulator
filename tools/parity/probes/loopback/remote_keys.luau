-- parity: loopback
-- Remotes each way with dictionary keys whose __tostring fails (nil, an error, a table raised, a yield, two such keys, a key that
-- clears its table) or that name a datatype, and datatype values alone and in tables: what the sending call raises, how often a
-- key's __tostring runs, and what arrives. Then InvokeServer with such a key among its arguments and among its callback's
-- results, and what a client's output shows for a FireServer that raises uncaught.
local Storage = game:GetService("ReplicatedStorage")
local LogService = game:GetService("LogService")
local function Clean(Text)
	return (string.gsub(tostring(Text), "0x%x+", "ADDR"))
end
-- a received value as text, a table's entries sorted
local function Shown(Value)
	if type(Value) ~= "table" then
		return typeof(Value) .. ":" .. P.err(Clean(Value))
	end
	local Entries = {}
	for Key, Item in Value do
		table.insert(Entries, typeof(Key) .. ":" .. Clean(Key) .. "=" .. Shown(Item))
	end
	table.sort(Entries)
	return "{" .. table.concat(Entries, ", ") .. "}"
end
local function Failed(Problem)
	return "error " .. typeof(Problem) .. (if type(Problem) == "table" then "" else " " .. P.err(Clean(Problem)))
end
local Calls = 0
local function Meta(Result)
	return {
		__tostring = function()
			Calls += 1
			if type(Result) == "function" then
				return Result()
			end
			return Result
		end,
	}
end
local function Keyed(Result)
	return { [setmetatable({}, Meta(Result))] = 1 }
end
local Signal = workspace.Changed
local Connection = Signal:Connect(function() end)
Connection:Disconnect()
local Cases = {
	{ "tostring nil", function() return Keyed(nil) end },
	{ "tostring number", function() return Keyed(42) end },
	{ "tostring error", function() return Keyed(function() error("boom") end) end },
	{ "tostring error table", function() return Keyed(function() error({ code = 1 }) end) end },
	{ "tostring task.wait", function() return Keyed(function() task.wait() return "after" end) end },
	{ "two tostring nil", function() return { [setmetatable({}, Meta(nil))] = 1, [setmetatable({}, Meta(nil))] = 2 } end },
	{ "clears table", function()
		local Table = { a = 1, b = 2, c = 3 }
		Table[setmetatable({}, Meta(function() table.clear(Table) return "cleared" end))] = 4
		return Table
	end },
	{ "numbers named same", function() return { [1.5] = "a", [1.25] = "b" } end },
	{ "key CFrame", function() return { [CFrame.new(1, 2, 3)] = 1 } end },
	{ "key BrickColor", function() return { [BrickColor.new("Bright red")] = 1 } end },
	{ "key Rect", function() return { [Rect.new(1, 2, 3, 4)] = 1 } end },
	{ "key Enum", function() return { [Enum.Material] = 1 } end },
	{ "key Enums", function() return { [Enum] = 1 } end },
	{ "key RBXScriptSignal", function() return { [Signal] = 1 } end },
	{ "key RBXScriptConnection", function() return { [Connection] = 1 } end },
	{ "key RotationCurveKey", function() return { [RotationCurveKey.new(0, CFrame.new(), Enum.KeyInterpolationMode.Linear)] = 1 } end },
	{ "key newproxy", function() local Value = newproxy(true) getmetatable(Value).__tostring = function() return "proxy" end return { [Value] = 1 } end },
	{ "key Axes", function() return { [Axes.new(Enum.Axis.X)] = 1 } end },
	{ "value Axes", function() return Axes.new(Enum.Axis.X, Enum.Axis.Y) end },
	{ "value Axes of a face", function() return Axes.new(Enum.NormalId.Top) end },
	{ "value Faces", function() return Faces.new(Enum.NormalId.Top, Enum.NormalId.Back) end },
	{ "value PathWaypoint", function() return PathWaypoint.new(Vector3.new(1, 2, 3), Enum.PathWaypointAction.Jump, "label") end },
	{ "value PathWaypoint plain", function() return PathWaypoint.new() end },
	{ "value ColorSequenceKeypoint", function() return ColorSequenceKeypoint.new(0.5, Color3.new(0, 0, 1)) end },
	{ "value NumberSequenceKeypoint", function() return NumberSequenceKeypoint.new(0.5, 2, 0.25) end },
	{ "value SecurityCapabilities", function() return SecurityCapabilities.new() end },
	{ "value SecurityCapabilities of two", function() return SecurityCapabilities.new(Enum.SecurityCapability.UI, Enum.SecurityCapability.Basic) end },
	{ "value RotationCurveKey", function() return RotationCurveKey.new(1, CFrame.new(1, 2, 3), Enum.KeyInterpolationMode.Cubic) end },
	{ "value ValueCurveKey", function() return ValueCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear) end },
	{ "value FloatCurveKey", function() return FloatCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear) end },
	{ "value Path2DControlPoint", function() return Path2DControlPoint.new(UDim2.new(0, 1, 0, 2)) end },
	{ "value DockWidgetPluginGuiInfo", function() return DockWidgetPluginGuiInfo.new() end },
	{ "value CatalogSearchParams", function() return CatalogSearchParams.new() end },
	{ "value OverlapParams", function() return OverlapParams.new() end },
	{ "value Content", function() return Content.fromUri("rbxassetid://1") end },
	{ "value Region3", function() return Region3.new(Vector3.new(0, 0, 0), Vector3.new(2, 2, 2)) end },
	{ "value Region3int16", function() return Region3int16.new(Vector3int16.new(0, 0, 0), Vector3int16.new(2, 2, 2)) end },
	{ "value Vector2int16", function() return Vector2int16.new(1, 2) end },
	{ "value Vector3int16", function() return Vector3int16.new(1, 2, 3) end },
	{ "value RBXScriptSignal", function() return Signal end },
	{ "value newproxy", function() return newproxy(true) end },
	{ "in array Axes", function() return { Axes.new(Enum.Axis.Z), 2 } end },
	{ "in array Faces", function() return { Faces.new(Enum.NormalId.Bottom), 2 } end },
	{ "in dict PathWaypoint", function() return { W = PathWaypoint.new(Vector3.new(4, 5, 6), Enum.PathWaypointAction.Walk), N = 2 } end },
	{ "in dict keypoints", function() return { C = ColorSequenceKeypoint.new(0, Color3.new(1, 0, 0)), N = NumberSequenceKeypoint.new(1, 3) } end },
}
local function Index(Name)
	for Position, Case in Cases do
		if Case[1] == Name then
			return Position
		end
	end
	error("no case " .. Name)
end
local function Send(Fire)
	for Position, Case in Cases do
		Calls = 0
		local Ok, Problem = pcall(Fire, Position, Case[2]())
		print("send " .. Case[1] .. ": " .. (if Ok then "ok" else Failed(Problem)) .. " calls " .. Calls)
	end
end
local function Report(Got)
	for Position, Case in Cases do
		print("got " .. Case[1] .. ": " .. (Got[Position] or "nothing"))
	end
end
--- server
local Player = P.player()
local Event = Instance.new("RemoteEvent")
Event.Name = "Keys"
Event.Parent = Storage
local Invoke = Instance.new("RemoteFunction")
Invoke.Name = "KeysInvoke"
Invoke.Parent = Storage
Invoke.OnServerInvoke = function(_, Position, Value)
	if Position == "return a failing key" then
		return Keyed(nil)
	end
	return Shown(Value)
end
local Got = {}
Event.OnServerEvent:Connect(function(_, Position, Value)
	Got[Position] = Shown(Value)
end)
P.meet("ready")
Send(function(Position, Payload)
	Event:FireClient(Player, Position, Payload)
end)
P.meet("server sent")
P.meet("client sent")
for _ = 1, 10 do
	task.wait()
end
Report(Got)
--- client
local Event = Storage:WaitForChild("Keys")
local Invoke = Storage:WaitForChild("KeysInvoke")
local Got = {}
Event.OnClientEvent:Connect(function(Position, Value)
	Got[Position] = Shown(Value)
end)
P.meet("ready")
P.meet("server sent")
for _ = 1, 10 do
	task.wait()
end
Report(Got)
Send(function(Position, Payload)
	Event:FireServer(Position, Payload)
end)
for _, Name in { "tostring nil", "tostring error", "tostring error table", "key CFrame", "value Axes", "value PathWaypoint" } do
	Calls = 0
	local Ok, Result = pcall(Invoke.InvokeServer, Invoke, Index(Name), Cases[Index(Name)][2]())
	print("invoke " .. Name .. ": " .. (if Ok then "ok " .. tostring(Result) else Failed(Result)) .. " calls " .. Calls)
end
local Ok, Result = pcall(Invoke.InvokeServer, Invoke, "return a failing key")
print("invoke results with a failing key: " .. (if Ok then "ok " .. Shown(Result) else Failed(Result)))
-- a FireServer that raises uncaught: the client's output (its lines after the first are the stack)
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local Out = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind ~= Enum.MessageType.MessageOutput then
		local Text = string.gsub(Message, "Script '" .. Escaped .. "', Line (%d+)", function(Line)
			return "Script '<probe>', Line @" .. (tonumber(Line) - P.base)
		end)
		table.insert(Out, Kind.Name .. ":" .. P.text(P.err(Text)))
	end
end)
task.spawn(function()
	Event:FireServer(Index("tostring nil"), Keyed(nil))
end)
for _ = 1, 6 do
	task.wait()
end
Listener:Disconnect()
print("uncaught " .. table.concat(Out, " || "))
P.meet("client sent")
