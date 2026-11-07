-- LogService's logging methods in a playing client: what MessageOut gets from Output, Info, Warn, Error and Log (the text with
-- its {key} placeholders filled, the MessageType, the context argument), their argument errors, the table Error throws, an
-- Error no script catches, and messages that are not strings. Errors are raised in spawned threads, whose stacks hold the
-- probe's own frames only.
local LogService = game:GetService("LogService")
P.sync()
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local function Normal(Text)
	Text = string.gsub(Text, "Script '" .. Escaped .. "', Line (%d+)", function(Line)
		return "Script '<probe>', Line @" .. (tonumber(Line) - P.base)
	end)
	return (string.gsub(Text, "0x%x+", "ADDR"))
end
local function Describe(Value, Depth)
	Depth = Depth or 0
	local Kind = typeof(Value)
	if Kind == "table" then
		if Depth > 4 then
			return "{...}"
		end
		local Keys = {}
		for Key, Item in Value do
			table.insert(Keys, tostring(Key) .. ":" .. typeof(Key) .. "=" .. Describe(Item, Depth + 1))
		end
		table.sort(Keys)
		return (if table.isfrozen(Value) then "frozen" else "") .. "{" .. table.concat(Keys, ",") .. "}"
	elseif Kind == "number" then
		return P.num(Value)
	elseif Kind == "string" then
		return string.format("%q", Normal(Value))
	end
	return Kind .. ":" .. Normal(tostring(Value))
end

local Seen = {}
local CaptureAll = false
local Listener = LogService.MessageOut:Connect(function(...)
	local Message, Kind, Context = ...
	if CaptureAll or string.find(Message, "^L%d") or string.find(Message, "^Stack ") or string.find(Message, "^Script ") or string.find(Message, "^argument #") then
		table.insert(Seen, Kind.Name .. "[" .. table.concat(string.split(Normal(Message), "\n"), " / ") .. "] n=" .. select("#", ...) .. " ctx=" .. Describe(Context))
	end
end)
local function Flush(Label)
	for _ = 1, 3 do
		task.wait()
	end
	print(Label .. " " .. #Seen)
	for _, Line in Seen do
		print("  " .. Line)
	end
	table.clear(Seen)
end

-- 1. placeholders, escapes and values
local Folder = Instance.new("Folder")
Folder.Name = "LogProbe"
local Part = Instance.new("Part")
Part.Name = "Thing"
Part.Parent = Folder
Folder.Parent = workspace
LogService:Info("L01 a={a} b={b}", { a = 1, b = "two" })
LogService:Info("L02 Value = {{result}}: {val}", { val = 42 })
LogService:Info("L03 x={missing}", { a = 1 })
LogService:Info("L04 x={a} {{a}} }}")
LogService:Info("L05 {{a}} {a}", {})
LogService:Info("L06 {n} {m} {o} {p} {q}", { n = 1.5, m = 1e20, o = 0.1, p = -2.5, q = 1 / 3 })
LogService:Info("L07 {t} {f} {i} {u}", { t = true, f = false, i = Part, u = Instance.new("Part") })
LogService:Info("L08 {v} {c} {e} {u2} {b}", { v = Vector3.new(1, 2, 3), c = Color3.new(1, 0.5, 0), e = Enum.Material.Plastic, u2 = UDim2.new(1, 2, 3, 4), b = BrickColor.new("Bright red") })
LogService:Info("L09 {a{b}} {a {} a}b", { a = 1, b = 2 })
LogService:Info("L10 { a } {a.b} {1}", { [" a "] = 1, a = { b = 2 }, ["a.b"] = 3, "x" })
LogService:Info("L11 {1}", { [1] = "number", ["1"] = "string" })
LogService:Info("L12 {a}{a} {{{a}}} {a}}", { a = "x" })
LogService:Info("L13 {b}", { b = "{a}", a = 2 })
LogService:Info("L14 {a}", setmetatable({}, { __index = function()
	return "meta"
end }))
LogService:Info("L15 {a}", { a = setmetatable({}, { __tostring = function()
	return "custom"
end }) })
LogService:Info("L16 {a}", 5)
LogService:Info("L17 {a}", "text")
LogService:Info("L18 {Name}", workspace)
LogService:Info("L19 plain", { a = 1 }, "extra")
Flush("templates")

-- 2. each method's MessageType
LogService:Output("L20 {a}", { a = 1 })
LogService:Warn("L21 {a}", { a = 1 })
LogService:Log(Enum.MessageType.MessageInfo, "L22 {a}", { a = 1 })
LogService:Log(Enum.MessageType.MessageOutput, "L23")
LogService:Log(Enum.MessageType.MessageWarning, "L24")
LogService:Log("MessageInfo", "L25")
LogService:Log(1, "L26")
Flush("types")

-- 3. argument errors
print("err", P.try(LogService.Info, LogService, nil))
print("err", P.try(LogService.Output, LogService))
print("err", P.try(LogService.Warn, LogService, nil, { a = 1 }))
print("err", P.try(LogService.Error, LogService))
print("err", P.try(LogService.Log, LogService))
print("err", P.try(LogService.Log, LogService, Enum.MessageType.MessageInfo))
print("err", P.try(LogService.Log, LogService, Enum.MessageType.MessageInfo, nil))
print("err", P.try(LogService.Log, LogService, Enum.Material.Plastic, "L27"))
print("err", P.try(LogService.Log, LogService, 7, "L28"))
print("err", P.try(LogService.Log, LogService, "Nope", "L29"))
print("err", P.try(LogService.Info, "L30 no self"))
print("err", P.try(LogService.Error, "L31 no self"))
Flush("argument-errors")

-- 4. the context MessageOut and the thrown table carry
local Cyclic = {}
Cyclic.self = Cyclic
local Given = {
	s = "str",
	n = 2,
	f = print,
	i = Part,
	t = { 1, 2, { x = Part, g = print } },
	c = Cyclic,
	th = coroutine.create(print),
	en = Enum.Material,
	item = Enum.Material.Plastic,
	v = Vector3.new(1, 2, 3),
	[2.7] = "frac",
	[-1] = "negative",
	[1e10] = "big",
	[true] = "bool",
	[3] = "number three",
	["3"] = "string three",
}
LogService:Info("L32 context", Given)
local Frozen = table.freeze({ a = 1 })
LogService:Info("L33 frozen", Frozen)
LogService:Info("L34 meta", setmetatable({ real = 1 }, { __index = function()
	return "x"
end }))
Flush("context")

-- 5. Error and Log(MessageError): the thrown table
task.spawn(function()
	local Ok, Thrown = pcall(function()
		LogService:Error("L35 fail {s} {missing}", Given)
	end)
	local Keys = {}
	for Key in Thrown do
		table.insert(Keys, Key)
	end
	print("thrown", Ok, typeof(Thrown), table.concat(Keys, ","), table.isfrozen(Thrown))
	print("thrown fields", Thrown.message, Thrown.template, (string.gsub(Normal(Thrown.stack), "\n", " / ")))
	print("thrown context", Describe(Thrown.context), Thrown.context == Given)
	local Meta = getmetatable(Thrown)
	local MetaKeys = {}
	for Key, Value in Meta do
		table.insert(MetaKeys, Key .. ":" .. typeof(Value))
	end
	print("thrown meta", table.concat(MetaKeys, ","), table.isfrozen(Meta), tostring(Thrown))
	local _, Other = pcall(LogService.Error, LogService, "L36 other")
	print("thrown each", getmetatable(Other) ~= Meta, Describe(Other.context), Other.template)
	for _, Value in { 5, "", false, {} } do
		Other.message = Value
		print("thrown tostring", typeof(Value), tostring(Other))
	end
	Other.message = nil
	print("thrown tostring nil", tostring(Other))
	print("thrown concat", P.try(function()
		return "x" .. Thrown
	end))
	local _, Logged = pcall(LogService.Log, LogService, Enum.MessageType.MessageError, "L37 log {a}", { a = 1 })
	print("thrown log", typeof(Logged), Logged.message, Logged.template, Describe(Logged.context))
	local _, Bare = pcall(LogService.Error, LogService, "L38 {a}", 5)
	print("thrown bare", Bare.message, Describe(Bare.context))
end)
Flush("thrown")

-- 6. an Error no script catches: its own line, then its stack as MessageInfo lines
task.spawn(function()
	LogService:Error("L39 uncaught {a}", { a = 1 })
end)
Flush("uncaught")
task.defer(function()
	local _, Thrown = pcall(LogService.Error, LogService, "L40 caught then thrown")
	error(Thrown)
end)
Flush("rethrown")

-- 7. messages that are not strings (every line MessageOut gets, the warnings included)
CaptureAll = true
task.spawn(function()
	LogService:Info(5)
	LogService:Output(12.5)
	LogService:Info(true)
	LogService:Info({ "L41" })
	LogService:Info(Part)
	LogService:Info(print)
	LogService:Info(Enum.Material)
	LogService:Info(Vector3.new(1, 2, 3))
	LogService:Info(false, { a = 1 })
end)
Flush("non-strings")
CaptureAll = false

Listener:Disconnect()
Folder:Destroy()
