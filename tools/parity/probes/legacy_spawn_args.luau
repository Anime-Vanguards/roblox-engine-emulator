-- The legacy spawn and delay given a value that is no function (what each logs when it is due), their arity errors, odd delays,
-- and the exact text a client logs for a value that cannot be called, with what ScriptContext.Error gets.
local LogService = game:GetService("LogService")
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
	if Kind ~= Enum.MessageType.MessageOutput then
		table.insert(Seen, Kind.Name .. "[" .. Normal(Message) .. "]")
	end
end)
local Callable
Callable = setmetatable({}, { __call = function(Self, ...)
	local Parts = { tostring(rawequal(Self, Callable)) }
	for Index = 1, select("#", ...) do
		table.insert(Parts, typeof((select(Index, ...))))
	end
	table.insert(Seen, "called(" .. table.concat(Parts, ",") .. ")")
end })
local function Ran(...)
	table.insert(Seen, "ran(" .. select("#", ...) .. ")")
end
local function Case(Label, Call)
	table.clear(Seen)
	local Result = P.try(Call)
	task.wait(0.2)
	print(Label .. " " .. Result .. " " .. table.concat(Seen, " "))
end
Case("spawn-number", function() return spawn(5) end)
Case("spawn-nil", function() return spawn(nil) end)
Case("spawn-none", function() return spawn() end)
Case("spawn-string", function() return spawn("print") end)
Case("spawn-table", function() return spawn({}) end)
Case("spawn-bool", function() return spawn(false) end)
Case("spawn-instance", function() return spawn(workspace) end)
Case("spawn-callable", function() return spawn(Callable) end)
Case("spawn-function", function() return spawn(Ran) end)
Case("spawn-thread", function() return spawn(coroutine.create(Ran)) end)
Case("Spawn-number", function() return Spawn(5) end)
Case("delay-number", function() return delay(0, 5) end)
Case("delay-nil", function() return delay(0, nil) end)
Case("delay-none", function() return delay(0) end)
Case("delay-nothing", function() return delay() end)
Case("delay-table", function() return delay(0, {}) end)
Case("delay-callable", function() return delay(0, Callable) end)
Case("delay-thread", function() return delay(0, coroutine.create(Ran)) end)
Case("Delay-number", function() return Delay(0, 5) end)
Case("delay-seconds-string", function() return delay("x", Ran) end)
Case("delay-seconds-numstring", function() return delay("0.01", Ran) end)
Case("delay-seconds-nil", function() return delay(nil, Ran) end)
Case("delay-seconds-table", function() return delay({}, Ran) end)
Case("delay-seconds-bool", function() return delay(true, Ran) end)
Case("delay-seconds-negative", function() return delay(-1, Ran) end)
Case("delay-seconds-vector", function() return delay(Vector3.one, Ran) end)
-- the error's stack when the uncalled value raises in a thread
Case("spawn-error-stack", function()
	spawn(setmetatable({}, { __call = function()
		error("from call")
	end }))
end)
Listener:Disconnect()
-- arity
print("arity spawn-nil-nil " .. P.try(spawn, nil, nil))
print("arity delay-nil-nil " .. P.try(delay, nil, nil))
print("arity delay-one-nil " .. P.try(delay, nil))
print("arity Delay-none " .. P.try(Delay))
print("arity Spawn-none " .. P.try(Spawn))
-- the line a value that cannot be called logs (its raw text) and ScriptContext.Error's message and trace
local ScriptContext = game:GetService("ScriptContext")
local Raw = {}
local RawOut = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageError then
		table.insert(Raw, "out " .. string.format("%q", Message))
	end
end)
local RawError = ScriptContext.Error:Connect(function(Message, Trace)
	table.insert(Raw, "error " .. string.format("%q", Message) .. " trace " .. string.format("%q", Trace))
end)
for _, Case in { { "spawn-number", function() spawn(5) end }, { "delay-thread", function() delay(0, coroutine.create(function() end)) end } } do
	table.clear(Raw)
	Case[2]()
	task.wait(0.2)
	table.sort(Raw)
	print("raw " .. Case[1] .. " " .. table.concat(Raw, " | "))
end
RawOut:Disconnect()
RawError:Disconnect()
