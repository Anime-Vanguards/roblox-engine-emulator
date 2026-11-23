-- Engine functions as scripts see them: what debug.info gives for each ("sln", then "a"), read as a value from a global, a
-- library, a datatype, an Instance, a service, a signal or an enum; and the frames a traceback and the Output list for an error
-- raised inside one, called with `.` and `:`, under pcall and xpcall, and uncaught in a spawned thread. (warn is left out: the
-- capture's client replaces it with Luau of its own.)
local HttpService = game:GetService("HttpService")
local RunService = game:GetService("RunService")
local TweenService = game:GetService("TweenService")
local Players = game:GetService("Players")
local CollectionService = game:GetService("CollectionService")
local Debris = game:GetService("Debris")
local LogService = game:GetService("LogService")
local SharedTableRegistry = game:GetService("SharedTableRegistry")
local PhysicsService = game:GetService("PhysicsService")
local Lighting = game:GetService("Lighting")
local Teams = game:GetService("Teams")
local PathfindingService = game:GetService("PathfindingService")
local TextService = game:GetService("TextService")
local ScriptContext = game:GetService("ScriptContext")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
-- (the engine's print: the probe's own `print` is the harness's collector)
local EnginePrint = getfenv().print
P.sync()

local Part = Instance.new("Part")
Part.Name = "ParityPart"
local Model = Instance.new("Model")
local Event = Instance.new("BindableEvent")
local Function = Instance.new("BindableFunction")
local Connection = Event.Event:Connect(function() end)
local Rng = Random.new(1)
local When = DateTime.fromUnixTimestamp(0)
local V3 = Vector3.new(1, 2, 3)
local V2 = Vector2.new(1, 2)
local CF = CFrame.new(1, 2, 3)
local C3 = Color3.new(0.1, 0.2, 0.3)
local U2 = UDim2.new(1, 2, 3, 4)
local Shared = SharedTable.new()

-- 1. debug.info of each function
local Cases = {
	{ "print", function() return EnginePrint end },
	{ "error", function() return error end },
	{ "require", function() return require end },
	{ "wait", function() return wait end },
	{ "Wait", function() return Wait end },
	{ "delay", function() return delay end },
	{ "Delay", function() return Delay end },
	{ "spawn", function() return spawn end },
	{ "Spawn", function() return Spawn end },
	{ "tick", function() return tick end },
	{ "time", function() return time end },
	{ "elapsedTime", function() return elapsedTime end },
	{ "ElapsedTime", function() return ElapsedTime end },
	{ "typeof", function() return typeof end },
	{ "settings", function() return settings end },
	{ "UserSettings", function() return UserSettings end },
	{ "version", function() return version end },
	{ "Version", function() return Version end },
	{ "stats", function() return stats end },
	{ "Stats", function() return Stats end },
	{ "printidentity", function() return printidentity end },
	{ "PluginManager", function() return PluginManager end },
	{ "loadstring", function() return loadstring end },
	{ "collectgarbage", function() return collectgarbage end },
	{ "gcinfo", function() return gcinfo end },
	{ "ypcall", function() return ypcall end },
	{ "pcall", function() return pcall end },
	{ "xpcall", function() return xpcall end },
	{ "unpack", function() return unpack end },
	{ "getfenv", function() return getfenv end },
	{ "setfenv", function() return setfenv end },
	{ "newproxy", function() return newproxy end },
	{ "tostring", function() return tostring end },
	{ "task.wait", function() return task.wait end },
	{ "task.spawn", function() return task.spawn end },
	{ "task.defer", function() return task.defer end },
	{ "task.delay", function() return task.delay end },
	{ "task.cancel", function() return task.cancel end },
	{ "task.synchronize", function() return task.synchronize end },
	{ "task.desynchronize", function() return task.desynchronize end },
	{ "coroutine.create", function() return coroutine.create end },
	{ "coroutine.resume", function() return coroutine.resume end },
	{ "coroutine.yield", function() return coroutine.yield end },
	{ "coroutine.wrap", function() return coroutine.wrap end },
	{ "coroutine.status", function() return coroutine.status end },
	{ "coroutine.running", function() return coroutine.running end },
	{ "coroutine.isyieldable", function() return coroutine.isyieldable end },
	{ "coroutine.close", function() return coroutine.close end },
	{ "debug.traceback", function() return debug.traceback end },
	{ "debug.info", function() return debug.info end },
	{ "debug.profilebegin", function() return debug.profilebegin end },
	{ "debug.profileend", function() return debug.profileend end },
	{ "debug.setmemorycategory", function() return debug.setmemorycategory end },
	{ "debug.resetmemorycategory", function() return debug.resetmemorycategory end },
	{ "utf8.char", function() return utf8.char end },
	{ "utf8.len", function() return utf8.len end },
	{ "utf8.codes", function() return utf8.codes end },
	{ "utf8.graphemes", function() return utf8.graphemes end },
	{ "utf8.nfcnormalize", function() return utf8.nfcnormalize end },
	{ "utf8.nfdnormalize", function() return utf8.nfdnormalize end },
	{ "os.clock", function() return os.clock end },
	{ "os.time", function() return os.time end },
	{ "os.date", function() return os.date end },
	{ "os.difftime", function() return os.difftime end },
	{ "math.random", function() return math.random end },
	{ "math.randomseed", function() return math.randomseed end },
	{ "math.noise", function() return math.noise end },
	{ "math.clamp", function() return math.clamp end },
	{ "string.format", function() return string.format end },
	{ "table.sort", function() return table.sort end },
	{ "Instance.new", function() return Instance.new end },
	{ "Instance.fromExisting", function() return Instance.fromExisting end },
	{ "Vector3.new", function() return Vector3.new end },
	{ "Vector3.fromNormalId", function() return Vector3.fromNormalId end },
	{ "Vector3.fromAxis", function() return Vector3.fromAxis end },
	{ "Vector2.new", function() return Vector2.new end },
	{ "Vector3int16.new", function() return Vector3int16.new end },
	{ "Vector2int16.new", function() return Vector2int16.new end },
	{ "CFrame.new", function() return CFrame.new end },
	{ "CFrame.Angles", function() return CFrame.Angles end },
	{ "CFrame.fromEulerAnglesXYZ", function() return CFrame.fromEulerAnglesXYZ end },
	{ "CFrame.fromOrientation", function() return CFrame.fromOrientation end },
	{ "CFrame.lookAt", function() return CFrame.lookAt end },
	{ "CFrame.fromMatrix", function() return CFrame.fromMatrix end },
	{ "CFrame.fromAxisAngle", function() return CFrame.fromAxisAngle end },
	{ "Color3.new", function() return Color3.new end },
	{ "Color3.fromRGB", function() return Color3.fromRGB end },
	{ "Color3.fromHSV", function() return Color3.fromHSV end },
	{ "Color3.fromHex", function() return Color3.fromHex end },
	{ "Color3.toHSV", function() return Color3.toHSV end },
	{ "UDim.new", function() return UDim.new end },
	{ "UDim2.new", function() return UDim2.new end },
	{ "UDim2.fromScale", function() return UDim2.fromScale end },
	{ "UDim2.fromOffset", function() return UDim2.fromOffset end },
	{ "BrickColor.new", function() return BrickColor.new end },
	{ "BrickColor.random", function() return BrickColor.random end },
	{ "BrickColor.palette", function() return BrickColor.palette end },
	{ "BrickColor.Red", function() return BrickColor.Red end },
	{ "Ray.new", function() return Ray.new end },
	{ "Rect.new", function() return Rect.new end },
	{ "Region3.new", function() return Region3.new end },
	{ "Region3int16.new", function() return Region3int16.new end },
	{ "NumberRange.new", function() return NumberRange.new end },
	{ "NumberSequence.new", function() return NumberSequence.new end },
	{ "NumberSequenceKeypoint.new", function() return NumberSequenceKeypoint.new end },
	{ "ColorSequence.new", function() return ColorSequence.new end },
	{ "ColorSequenceKeypoint.new", function() return ColorSequenceKeypoint.new end },
	{ "PhysicalProperties.new", function() return PhysicalProperties.new end },
	{ "Faces.new", function() return Faces.new end },
	{ "Axes.new", function() return Axes.new end },
	{ "TweenInfo.new", function() return TweenInfo.new end },
	{ "Random.new", function() return Random.new end },
	{ "DateTime.now", function() return DateTime.now end },
	{ "DateTime.fromUnixTimestamp", function() return DateTime.fromUnixTimestamp end },
	{ "DateTime.fromIsoDate", function() return DateTime.fromIsoDate end },
	{ "Font.new", function() return Font.new end },
	{ "Font.fromEnum", function() return Font.fromEnum end },
	{ "Font.fromName", function() return Font.fromName end },
	{ "Content.fromUri", function() return Content.fromUri end },
	{ "OverlapParams.new", function() return OverlapParams.new end },
	{ "RaycastParams.new", function() return RaycastParams.new end },
	{ "PathWaypoint.new", function() return PathWaypoint.new end },
	{ "Path2DControlPoint.new", function() return Path2DControlPoint.new end },
	{ "CatalogSearchParams.new", function() return CatalogSearchParams.new end },
	{ "SecurityCapabilities.new", function() return SecurityCapabilities.new end },
	{ "FloatCurveKey.new", function() return FloatCurveKey.new end },
	{ "RotationCurveKey.new", function() return RotationCurveKey.new end },
	{ "SharedTable.new", function() return SharedTable.new end },
	{ "SharedTable.size", function() return SharedTable.size end },
	{ "SharedTable.clone", function() return SharedTable.clone end },
	{ "SharedTable.clear", function() return SharedTable.clear end },
	{ "SharedTable.isFrozen", function() return SharedTable.isFrozen end },
	{ "SharedTable.update", function() return SharedTable.update end },
	{ "SharedTable.increment", function() return SharedTable.increment end },
	{ "v3.Lerp", function() return V3.Lerp end },
	{ "v3.Dot", function() return V3.Dot end },
	{ "v3.Cross", function() return V3.Cross end },
	{ "v3.FuzzyEq", function() return V3.FuzzyEq end },
	{ "v3.Max", function() return V3.Max end },
	{ "v2.Lerp", function() return V2.Lerp end },
	{ "cf.Inverse", function() return CF.Inverse end },
	{ "cf.Lerp", function() return CF.Lerp end },
	{ "cf.ToWorldSpace", function() return CF.ToWorldSpace end },
	{ "cf.PointToObjectSpace", function() return CF.PointToObjectSpace end },
	{ "cf.GetComponents", function() return CF.GetComponents end },
	{ "cf.ToEulerAnglesXYZ", function() return CF.ToEulerAnglesXYZ end },
	{ "c3.Lerp", function() return C3.Lerp end },
	{ "c3.ToHex", function() return C3.ToHex end },
	{ "c3.ToHSV", function() return C3.ToHSV end },
	{ "u2.Lerp", function() return U2.Lerp end },
	{ "rng.NextInteger", function() return Rng.NextInteger end },
	{ "rng.NextNumber", function() return Rng.NextNumber end },
	{ "rng.Clone", function() return Rng.Clone end },
	{ "dt.ToIsoDate", function() return When.ToIsoDate end },
	{ "dt.FormatUniversalTime", function() return When.FormatUniversalTime end },
	{ "Enum.GetEnums", function() return Enum.GetEnums end },
	{ "Enum.Material.GetEnumItems", function() return Enum.Material.GetEnumItems end },
	{ "EnumItem.IsA", function() return Enum.Material.Plastic.IsA end },
	{ "game.GetService", function() return game.GetService end },
	{ "game.FindService", function() return game.FindService end },
	{ "game.IsLoaded", function() return game.IsLoaded end },
	{ "game.getService", function() return game.getService end },
	{ "workspace.FindFirstChild", function() return workspace.FindFirstChild end },
	{ "workspace.WaitForChild", function() return workspace.WaitForChild end },
	{ "workspace.GetChildren", function() return workspace.GetChildren end },
	{ "workspace.GetDescendants", function() return workspace.GetDescendants end },
	{ "workspace.IsA", function() return workspace.IsA end },
	{ "workspace.Raycast", function() return workspace.Raycast end },
	{ "workspace.GetPartBoundsInBox", function() return workspace.GetPartBoundsInBox end },
	{ "workspace.GetServerTimeNow", function() return workspace.GetServerTimeNow end },
	{ "part.Destroy", function() return Part.Destroy end },
	{ "part.Clone", function() return Part.Clone end },
	{ "part.ClearAllChildren", function() return Part.ClearAllChildren end },
	{ "part.GetAttribute", function() return Part.GetAttribute end },
	{ "part.SetAttribute", function() return Part.SetAttribute end },
	{ "part.GetAttributes", function() return Part.GetAttributes end },
	{ "part.GetPropertyChangedSignal", function() return Part.GetPropertyChangedSignal end },
	{ "part.GetAttributeChangedSignal", function() return Part.GetAttributeChangedSignal end },
	{ "part.IsDescendantOf", function() return Part.IsDescendantOf end },
	{ "part.FindFirstAncestor", function() return Part.FindFirstAncestor end },
	{ "part.GetFullName", function() return Part.GetFullName end },
	{ "part.AddTag", function() return Part.AddTag end },
	{ "part.HasTag", function() return Part.HasTag end },
	{ "part.GetTags", function() return Part.GetTags end },
	{ "part.GetPivot", function() return Part.GetPivot end },
	{ "part.PivotTo", function() return Part.PivotTo end },
	{ "part.GetMass", function() return Part.GetMass end },
	{ "part.ApplyImpulse", function() return Part.ApplyImpulse end },
	{ "model.GetBoundingBox", function() return Model.GetBoundingBox end },
	{ "model.ScaleTo", function() return Model.ScaleTo end },
	{ "event.Fire", function() return Event.Fire end },
	{ "function.Invoke", function() return Function.Invoke end },
	{ "HttpService.JSONEncode", function() return HttpService.JSONEncode end },
	{ "HttpService.JSONDecode", function() return HttpService.JSONDecode end },
	{ "HttpService.GenerateGUID", function() return HttpService.GenerateGUID end },
	{ "HttpService.UrlEncode", function() return HttpService.UrlEncode end },
	{ "RunService.IsClient", function() return RunService.IsClient end },
	{ "RunService.IsServer", function() return RunService.IsServer end },
	{ "RunService.IsStudio", function() return RunService.IsStudio end },
	{ "RunService.IsRunning", function() return RunService.IsRunning end },
	{ "RunService.BindToRenderStep", function() return RunService.BindToRenderStep end },
	{ "TweenService.Create", function() return TweenService.Create end },
	{ "TweenService.GetValue", function() return TweenService.GetValue end },
	{ "Players.GetPlayers", function() return Players.GetPlayers end },
	{ "Players.GetPlayerFromCharacter", function() return Players.GetPlayerFromCharacter end },
	{ "CollectionService.GetTagged", function() return CollectionService.GetTagged end },
	{ "CollectionService.AddTag", function() return CollectionService.AddTag end },
	{ "CollectionService.GetInstanceAddedSignal", function() return CollectionService.GetInstanceAddedSignal end },
	{ "Debris.AddItem", function() return Debris.AddItem end },
	{ "LogService.GetLogHistory", function() return LogService.GetLogHistory end },
	{ "SharedTableRegistry.GetSharedTable", function() return SharedTableRegistry.GetSharedTable end },
	{ "SharedTableRegistry.SetSharedTable", function() return SharedTableRegistry.SetSharedTable end },
	{ "PhysicsService.RegisterCollisionGroup", function() return PhysicsService.RegisterCollisionGroup end },
	{ "PhysicsService.GetRegisteredCollisionGroups", function() return PhysicsService.GetRegisteredCollisionGroups end },
	{ "Lighting.GetMinutesAfterMidnight", function() return Lighting.GetMinutesAfterMidnight end },
	{ "Teams.GetTeams", function() return Teams.GetTeams end },
	{ "PathfindingService.CreatePath", function() return PathfindingService.CreatePath end },
	{ "TextService.GetTextSize", function() return TextService.GetTextSize end },
	{ "signal.Connect", function() return Event.Event.Connect end },
	{ "signal.Once", function() return Event.Event.Once end },
	{ "signal.Wait", function() return Event.Event.Wait end },
	{ "signal.ConnectParallel", function() return Event.Event.ConnectParallel end },
	{ "connection.Disconnect", function() return Connection.Disconnect end },
}
for _, Case in Cases do
	local Ok, Value = pcall(Case[2])
	if not Ok then
		print("info " .. Case[1] .. " error " .. P.err(Value))
	elseif type(Value) ~= "function" then
		print("info " .. Case[1] .. " " .. typeof(Value))
	else
		local Source, Line, Name = debug.info(Value, "sln")
		local Arity, Vararg = debug.info(Value, "a")
		print("info " .. Case[1] .. " " .. tostring(Source) .. " " .. tostring(Line) .. " " .. tostring(Name) .. " a " .. tostring(Arity) .. " " .. tostring(Vararg))
	end
end
-- the same function from every read
print("same game.GetService " .. tostring(rawequal(game.GetService, game.GetService)))
print("same FindFirstChild workspace/part " .. tostring(rawequal(workspace.FindFirstChild, Part.FindFirstChild)))
print("same Destroy part/model " .. tostring(rawequal(Part.Destroy, Model.Destroy)))
print("same signal.Connect " .. tostring(rawequal(Event.Event.Connect, game.ChildAdded.Connect)))
print("same v3.Lerp " .. tostring(rawequal(V3.Lerp, Vector3.one.Lerp)))
print("same task.wait " .. tostring(rawequal(task.wait, task.wait)))

-- 2. tracebacks: an xpcall handler's traceback (from the handler up to Trace's own frame, the frames of the probe made
-- comparable and others kept as they are), and the error the handler and pcall get
local Prefix = P.chunk() .. ":"
local function Frames(Text)
	local Out = {}
	for Line in string.gmatch(Text, "[^\n]+") do
		if string.sub(Line, 1, #Prefix) == Prefix then
			local Number, Rest = string.match(string.sub(Line, #Prefix + 1), "^(%d+)(.*)$")
			if Number then
				Line = "@" .. (tonumber(Number) - P.base) .. Rest
			end
		end
		table.insert(Out, Line)
		if string.find(Line, "function Trace", 1, true) then
			break
		end
	end
	return table.concat(Out, " / ")
end
local function Trace(Label, Call, ...)
	local Seen
	local Ok, Result = xpcall(Call, function(Message)
		Seen = P.err(Message) .. " | " .. Frames(debug.traceback())
		return Message
	end, ...)
	print("trace " .. Label .. " " .. (if Ok then "ok" else tostring(Seen)))
	print("pcall " .. Label .. " " .. P.try(Call, ...))
end
local Thrower = setmetatable({}, { __tostring = function()
	error("from __tostring")
end })
Trace("SharedTable.size direct", SharedTable.size, 5)
Trace("SharedTable.size", function() return SharedTable.size(5) end)
Trace("SharedTable.clone", function() return SharedTable.clone(5) end)
Trace("SharedTable.update", function() return SharedTable.update(Shared, "k", function() error("from update") end) end)
Trace("SharedTableRegistry:GetSharedTable", function() return SharedTableRegistry:GetSharedTable(nil) end)
Trace("SharedTableRegistry.GetSharedTable", function() return SharedTableRegistry.GetSharedTable(SharedTableRegistry, nil) end)
Trace("task.spawn", function() return task.spawn(5) end)
Trace("task.defer", function() return task.defer(5) end)
Trace("task.delay", function() return task.delay(1, 5) end)
Trace("task.cancel", function() return task.cancel(5) end)
Trace("task.wait", function() return task.wait("x") end)
Trace("spawn", function() return spawn(5) end)
Trace("delay", function() return delay(1, 5) end)
Trace("Random.new", function() return Random.new("x") end)
Trace("rng:NextInteger", function() return Rng:NextInteger("x") end)
Trace("rng.NextInteger", function() return Rng.NextInteger(Rng, "x") end)
Trace("TweenInfo.new", function() return TweenInfo.new("x") end)
Trace("Vector3.new", function() return Vector3.new("x") end)
Trace("Vector2.new", function() return Vector2.new("x") end)
Trace("CFrame.new", function() return CFrame.new("x") end)
Trace("CFrame.Angles", function() return CFrame.Angles("x") end)
Trace("Color3.fromRGB", function() return Color3.fromRGB("x") end)
Trace("Color3.new", function() return Color3.new("x") end)
Trace("UDim2.new", function() return UDim2.new("x") end)
Trace("UDim2.fromScale", function() return UDim2.fromScale("x") end)
Trace("BrickColor.new", function() return BrickColor.new({}) end)
Trace("v3:Lerp", function() return V3:Lerp("x") end)
Trace("cf:Lerp", function() return CF:Lerp("x") end)
Trace("game:GetService", function() return game:GetService("NotAService") end)
Trace("game.GetService", function() return game.GetService(game, "NotAService") end)
Trace("game.GetService direct", game.GetService, game, "NotAService")
Trace("workspace:FindFirstChild", function() return workspace:FindFirstChild() end)
Trace("workspace.FindFirstChild", function() return workspace.FindFirstChild(workspace) end)
Trace("workspace:Raycast", function() return workspace:Raycast("x") end)
Trace("workspace.Raycast", function() return workspace.Raycast(workspace, "x") end)
Trace("part:SetAttribute", function() return Part:SetAttribute("x", {}) end)
Trace("part.SetAttribute", function() return Part.SetAttribute(Part, "x", {}) end)
Trace("HttpService:JSONDecode", function() return HttpService:JSONDecode("{") end)
Trace("HttpService.JSONDecode", function() return HttpService.JSONDecode(HttpService, "{") end)
Trace("HttpService:JSONEncode", function() return HttpService:JSONEncode({ print }) end)
Trace("TweenService:Create", function() return TweenService:Create(Part, "x", {}) end)
Trace("TweenService.Create", function() return TweenService.Create(TweenService, Part, "x", {}) end)
Trace("Debris:AddItem", function() return Debris:AddItem("x") end)
Trace("CollectionService:AddTag", function() return CollectionService:AddTag("x", "y") end)
Trace("event:Fire", function() return Event.Fire(5) end)
Trace("signal:Connect", function() return Event.Event:Connect(5) end)
Trace("signal.Connect", function() return Event.Event.Connect(Event.Event, 5) end)
Trace("require", function() return require(Part) end)
Trace("print", function() return EnginePrint(Thrower) end)
Trace("tostring", function() return tostring(Thrower) end)
Trace("collectgarbage", function() return collectgarbage("collect") end)
Trace("os.time", function() return os.time(5) end)
Trace("utf8.nfcnormalize", function() return utf8.nfcnormalize(5) end)
Trace("Instance.new", function() return Instance.new("NotAClass") end)
Trace("typeof", function() return typeof() end)
Trace("DateTime.fromIsoDate", function() return DateTime.fromIsoDate(5) end)

-- 3. a callback an engine function calls: the levels above it
local function Levels()
	local Out = {}
	for Level = 2, 3 do
		local Source, Line, Name = debug.info(Level, "sln")
		if Source == nil then
			break
		end
		if string.sub(Source, 1, #P.chunk()) == P.chunk() then
			Source, Line = "@", Line - P.base
		end
		table.insert(Out, tostring(Source) .. " " .. tostring(Line) .. " " .. tostring(Name))
	end
	return table.concat(Out, " / ")
end
local Seen
pcall(EnginePrint, setmetatable({}, { __tostring = function()
	Seen = Levels()
	error("stop")
end }))
print("levels print " .. tostring(Seen))
Seen = nil
pcall(SharedTable.update, Shared, "k", function()
	Seen = Levels()
end)
print("levels update " .. tostring(Seen))
Seen = nil
Function.OnInvoke = function()
	Seen = Levels()
end
Function:Invoke()
print("levels invoke " .. tostring(Seen))
Seen = nil
task.spawn(function()
	Seen = Levels() .. " | " .. Frames(debug.traceback())
end)
print("levels task.spawn " .. tostring(Seen))
Seen = nil
spawn(function()
	Seen = Levels()
end)
task.wait(0.1)
print("levels spawn " .. tostring(Seen))

-- 4. uncaught: the Output's text and ScriptContext.Error's stack of an error raised inside an engine function in a spawned
-- thread
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local function Normal(Text)
	Text = string.gsub(Text, "Script '" .. Escaped .. "', Line (%d+)", function(Line)
		return "Script '<probe>', Line @" .. (tonumber(Line) - P.base)
	end)
	Text = string.gsub(Text, Escaped .. ":(%d+)", function(Line)
		return "@" .. (tonumber(Line) - P.base)
	end)
	return (string.gsub(Text, "0x%x+", "ADDR"))
end
local Out = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageError then
		table.insert(Out, (table.concat(string.split(Normal(Message), "\n"), " / ")))
	end
end)
local function Uncaught(Label, Body)
	task.spawn(Body)
	for _ = 1, 3 do
		task.wait()
	end
	print("uncaught " .. Label .. " " .. table.concat(Out, " // "))
	table.clear(Out)
end
Uncaught("SharedTable.size", function()
	SharedTable.size(5)
end)
Uncaught("task.spawn", function()
	task.spawn(5)
end)
Uncaught("game:GetService", function()
	game:GetService("NotAService")
end)
Uncaught("game.GetService", function()
	game.GetService(game, "NotAService")
end)
Uncaught("SharedTableRegistry:GetSharedTable", function()
	SharedTableRegistry:GetSharedTable(nil)
end)
Uncaught("Vector3.new", function()
	Vector3.new("x")
end)
Uncaught("print", function()
	EnginePrint(Thrower)
end)
Uncaught("Random.new", function()
	Random.new("x")
end)
Uncaught("SharedTable.update", function()
	SharedTable.update(Shared, "k", function()
		error("parity uncaught update")
	end)
end)
Listener:Disconnect()
Connection:Disconnect()

-- 5. more errors raised inside engine functions: task.delay's seconds, coroutine.create and wrap, a vector method read as a
-- value, a datatype constructor and a library function written in the engine's own Luau, a service method in it
Trace("task.delay seconds", function() return task.delay("x", print) end)
Trace("coroutine.create", function() return coroutine.create(5) end)
Trace("coroutine.create none", function() return coroutine.create() end)
Trace("coroutine.wrap", function() return coroutine.wrap(5) end)
Trace("v3.FuzzyEq", function() return V3.FuzzyEq(V3, 5) end)
Trace("v3:FuzzyEq", function() return V3:FuzzyEq(5) end)
Trace("Path2DControlPoint.new", function() return Path2DControlPoint.new(5) end)
Trace("GetSharedTable wrong self", function() return SharedTableRegistry.GetSharedTable(Part, "x") end)

-- 6. a thread waiting inside an engine function, and one resumed from it: their tracebacks
local function Waiting(Label, Body)
	local Thread = task.spawn(Body)
	print("waiting " .. Label .. " " .. coroutine.status(Thread) .. " | " .. Frames(debug.traceback(Thread)))
end
Waiting("task.wait", function()
	task.wait(0.2)
end)
Waiting("wait", function()
	wait(0.2)
end)
Waiting("signal:Wait", function()
	Event.Event:Wait()
end)
Waiting("signal.Wait", function()
	Event.Event.Wait(Event.Event)
end)
Waiting("WaitForChild", function()
	workspace:WaitForChild("ParityNever", 0.2)
end)
Waiting("WaitForChild value", function()
	workspace.WaitForChild(workspace, "ParityNever", 0.2)
end)
local Resumed
task.spawn(function()
	task.wait()
	Resumed = Frames(debug.traceback())
end)
task.wait(0.1)
print("resumed task.wait " .. tostring(Resumed))
task.wait(0.3)
