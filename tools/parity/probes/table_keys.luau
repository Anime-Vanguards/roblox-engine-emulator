-- Dictionary keys that are neither strings nor numbers, as BindableEvent:Fire, BindableFunction:Invoke, HttpService:JSONEncode
-- and a LocalizationTable's FormatByKey and SetEntries name them: a key whose __tostring returns something other than a string,
-- raises (a string, a table, nil), yields or changes the table being copied; keys that end up with the same name; and a key of
-- every datatype a script can make. Each row prints the call's outcome, what arrived and how often the key's __tostring ran.
local HttpService = game:GetService("HttpService")
local function Clean(Text)
	return (string.gsub(tostring(Text), "0x%x+", "ADDR"))
end
-- a received value as text, a table's entries sorted (a table key's place in a table is its address's)
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
-- a JSON object's entries sorted, for the same reason (the rows' keys and values hold no `,"`)
local function SortedJson(Text)
	if string.sub(Text, 1, 1) ~= "{" then
		return Text
	end
	local Entries = string.split(string.sub(Text, 2, -2), ',"')
	for Index = 2, #Entries do
		Entries[Index] = '"' .. Entries[Index]
	end
	table.sort(Entries)
	return "{" .. table.concat(Entries, ",") .. "}"
end
local Thrown = {}
local function Outcome(Ok, Result, Json)
	if Ok then
		return "ok " .. (if Json then SortedJson(Clean(Result)) else Shown(Result))
	end
	if type(Result) == "table" then
		return "error the table raised: " .. tostring(rawequal(Result, Thrown.Last))
	end
	return "error " .. typeof(Result) .. " " .. P.err(Clean(Result))
end
local Calls = 0
-- a metatable whose __tostring counts its calls and gives Result (or what the function Result gives)
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
local function Proxy(Result)
	local Value = newproxy(true)
	getmetatable(Value).__tostring = Meta(Result).__tostring
	return Value
end
local Cases = {
	{ "tostring nil", function() return Keyed(nil) end },
	{ "tostring number", function() return Keyed(42) end },
	{ "tostring string", function() return Keyed("custom") end },
	{ "tostring boolean", function() return Keyed(true) end },
	{ "tostring table", function() return Keyed({}) end },
	{ "tostring error", function() return Keyed(function() error("boom") end) end },
	{ "tostring error level 0", function() return Keyed(function() error("plain boom", 0) end) end },
	{ "tostring error table", function() return Keyed(function() local Problem = { code = 1 } Thrown.Last = Problem error(Problem) end) end },
	{ "tostring error nil", function() return Keyed(function() error(nil) end) end },
	{ "tostring error number", function() return Keyed(function() error(7) end) end },
	{ "tostring task.wait", function() return Keyed(function() task.wait() return "after" end) end },
	{ "tostring coroutine.yield", function() return Keyed(function() coroutine.yield() return "after" end) end },
	{ "tostring index error", function() return Keyed(function() local Missing = nil return Missing.Field end) end },
	{ "two tostring nil", function() return { [setmetatable({}, Meta(nil))] = 1, [setmetatable({}, Meta(nil))] = 2 } end },
	{ "proxy tostring nil", function() return { [Proxy(nil)] = 1 } end },
	{ "proxy tostring error", function() return { [Proxy(function() error("proxy boom") end)] = 1 } end },
	{ "nested tostring nil", function() return { Inner = Keyed(nil) } end },
	{ "array key tostring nil", function() return { 10, 20, [setmetatable({}, Meta(nil))] = 3 } end },
	{ "value tostring nil", function() return { Value = setmetatable({}, Meta(nil)) } end },
	{ "clears table", function()
		local Table = { a = 1, b = 2, c = 3 }
		Table[setmetatable({}, Meta(function() table.clear(Table) return "cleared" end))] = 4
		return Table
	end },
	{ "removes own entry", function()
		local Table = { a = 1 }
		local Key
		Key = setmetatable({}, Meta(function() Table[Key] = nil return "removed" end))
		Table[Key] = 4
		return Table
	end },
	{ "two named same", function() return { [setmetatable({}, Meta("same"))] = 1, [setmetatable({}, Meta("same"))] = 1 } end },
	{ "numbers named same", function() return { [1.5] = "a", [1.25] = "b" } end },
	{ "string and number named same", function() return { ["1"] = "s", [1.5] = "n" } end },
	{ "number and string named same", function() return { [-1.5] = "n", ["-1"] = "s" } end },
	{ "instances named same", function() local A, B = Instance.new("Folder"), Instance.new("Folder") return { [A] = 1, [B] = 1 } end },
}
local Event = Instance.new("BindableEvent")
local Got = "none"
Event.Event:Connect(function(Value)
	Got = Value
end)
local Function = Instance.new("BindableFunction")
Function.OnInvoke = function(Value)
	return Value
end
P.sync()
for _, Case in Cases do
	Calls = 0
	Got = "none"
	local Ok, Result = pcall(Event.Fire, Event, Case[2]())
	task.wait()
	print("fire " .. Case[1] .. ": " .. Outcome(Ok, Result) .. " got " .. Shown(Got) .. " calls " .. Calls)
	Calls = 0
	print("invoke " .. Case[1] .. ": " .. Outcome(pcall(Function.Invoke, Function, Case[2]())) .. " calls " .. Calls)
	Calls = 0
	local Encoded, Text = pcall(HttpService.JSONEncode, HttpService, Case[2]())
	print("json " .. Case[1] .. ": " .. Outcome(Encoded, Text, true) .. " calls " .. Calls)
end
-- a LocalizationTable's: FormatByKey's dictionary of arguments (read before the key is looked up) and SetEntries' Values
for _, Case in { Cases[1], Cases[2], Cases[6], Cases[8], Cases[20] } do
	local Table = Instance.new("LocalizationTable")
	Table:SetEntries({ { Key = "k", Source = "a {x} b", Values = { ["es-es"] = "c {x} d" } } })
	local Translator = Table:GetTranslator("es-es")
	Calls = 0
	print("format " .. Case[1] .. ": " .. Outcome(pcall(Translator.FormatByKey, Translator, "k", Case[2]())) .. " calls " .. Calls)
	Calls = 0
	print("format missing key " .. Case[1] .. ": " .. Outcome(pcall(Translator.FormatByKey, Translator, "missing", Case[2]())) .. " calls " .. Calls)
	Calls = 0
	local Ok, Result = pcall(Table.SetEntries, Table, { { Key = "e", Source = "s", Values = Case[2]() } })
	local Keys = {}
	for _, Entry in Table:GetEntries() do
		table.insert(Keys, Entry.Key)
	end
	print("entries " .. Case[1] .. ": " .. Outcome(Ok, Result) .. " calls " .. Calls .. " keys " .. table.concat(Keys, ","))
end
-- a key of each datatype
local Signal = workspace.Changed
local Connection = Signal:Connect(function() end)
Connection:Disconnect()
local Types = {
	{ "Axes", function() return Axes.new(Enum.Axis.X, Enum.Axis.Z) end },
	{ "BrickColor", function() return BrickColor.new("Bright red") end },
	{ "CatalogSearchParams", function() return CatalogSearchParams.new() end },
	{ "CFrame", function() return CFrame.new(1, 2, 3) end },
	{ "Color3", function() return Color3.new(1, 0, 0) end },
	{ "ColorSequence", function() return ColorSequence.new(Color3.new(0, 1, 0)) end },
	{ "ColorSequenceKeypoint", function() return ColorSequenceKeypoint.new(0.5, Color3.new(0, 0, 1)) end },
	{ "Content", function() return Content.fromUri("rbxassetid://1") end },
	{ "DateTime", function() return DateTime.fromUnixTimestamp(0) end },
	{ "DockWidgetPluginGuiInfo", function() return DockWidgetPluginGuiInfo.new() end },
	{ "Enum", function() return Enum.Material end },
	{ "Enums", function() return Enum end },
	{ "EnumItem", function() return Enum.Material.Plastic end },
	{ "Faces", function() return Faces.new(Enum.NormalId.Top, Enum.NormalId.Left) end },
	{ "FloatCurveKey", function() return FloatCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear) end },
	{ "Font", function() return Font.new("rbxasset://fonts/families/Arial.json") end },
	{ "Instance", function() return workspace end },
	{ "NumberRange", function() return NumberRange.new(1, 2) end },
	{ "NumberSequence", function() return NumberSequence.new(1) end },
	{ "NumberSequenceKeypoint", function() return NumberSequenceKeypoint.new(0.5, 2) end },
	{ "OverlapParams", function() return OverlapParams.new() end },
	{ "Path2DControlPoint", function() return Path2DControlPoint.new() end },
	{ "PathWaypoint", function() return PathWaypoint.new(Vector3.new(1, 2, 3), Enum.PathWaypointAction.Jump, "label") end },
	{ "PhysicalProperties", function() return PhysicalProperties.new(1, 1, 1) end },
	{ "Random", function() return Random.new(1) end },
	{ "Ray", function() return Ray.new(Vector3.new(1, 2, 3), Vector3.new(0, 1, 0)) end },
	{ "RaycastParams", function() return RaycastParams.new() end },
	{ "RBXScriptConnection", function() return Connection end },
	{ "RBXScriptSignal", function() return Signal end },
	{ "Rect", function() return Rect.new(1, 2, 3, 4) end },
	{ "Region3", function() return Region3.new(Vector3.new(0, 0, 0), Vector3.new(2, 2, 2)) end },
	{ "Region3int16", function() return Region3int16.new(Vector3int16.new(0, 0, 0), Vector3int16.new(2, 2, 2)) end },
	{ "RotationCurveKey", function() return RotationCurveKey.new(0, CFrame.new(), Enum.KeyInterpolationMode.Linear) end },
	{ "SecurityCapabilities", function() return SecurityCapabilities.new() end },
	{ "SharedTable", function() return SharedTable.new({ a = 1 }) end },
	{ "TweenInfo", function() return TweenInfo.new(1) end },
	{ "UDim", function() return UDim.new(1, 2) end },
	{ "UDim2", function() return UDim2.new(1, 2, 3, 4) end },
	{ "ValueCurveKey", function() return ValueCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear) end },
	{ "Vector2", function() return Vector2.new(1, 2) end },
	{ "Vector2int16", function() return Vector2int16.new(1, 2) end },
	{ "Vector3", function() return Vector3.new(1, 2, 3) end },
	{ "Vector3int16", function() return Vector3int16.new(1, 2, 3) end },
	{ "vector", function() return vector.create(1, 2, 3) end },
	{ "buffer", function() return buffer.fromstring("hi") end },
	{ "function", function() return print end },
	{ "newproxy", function() return newproxy(false) end },
	{ "newproxy with a metatable", function() return newproxy(true) end },
	{ "newproxy with __tostring", function() return Proxy("proxy text") end },
	{ "thread", function() return coroutine.create(print) end },
	{ "boolean", function() return true end },
}
for _, Entry in Types do
	local Value = Entry[2]()
	local Fired, Problem = pcall(Event.Fire, Event, { [Value] = true })
	Got = "none"
	task.wait()
	print("key " .. Entry[1] .. ": fire " .. Outcome(Fired, Problem) .. " got " .. Shown(Got))
	print("key " .. Entry[1] .. ": invoke " .. Outcome(pcall(Function.Invoke, Function, { [Value] = true })))
	print("key " .. Entry[1] .. ": json " .. Outcome(pcall(HttpService.JSONEncode, HttpService, { [Value] = true })))
end
