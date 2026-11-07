-- The whole text a playing client's LogService.MessageOut gets for a script error (the traceback lines after the
-- message), with ScriptContext.Error's stack for the same error, for errors raised at several depths and from handlers.
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
P.sync()
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
local function Lines(Text)
	return table.concat(string.split(Normal(Text), "\n"), " / ")
end
local Seen = {}
local Traces = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if string.find(Message, "parity%-text") then
		table.insert(Seen, Kind.Name .. "[" .. Lines(Message) .. "]")
	end
end)
local ErrorListener = ScriptContext.Error:Connect(function(Message, Trace)
	if string.find(Message, "parity%-text") then
		table.insert(Traces, "[" .. Lines(Trace) .. "]")
	end
end)
local function Flush(Label)
	for _ = 1, 3 do
		task.wait()
	end
	print(Label .. " out " .. table.concat(Seen, " ") .. " trace " .. table.concat(Traces, " "))
	table.clear(Seen)
	table.clear(Traces)
end

local function Named()
	error("parity-text named")
end
local function Outer()
	Named()
end
task.spawn(Outer)
Flush("nested-functions")
task.spawn(error, "parity-text direct")
Flush("direct")
task.spawn(function()
	error("parity-text level0", 0)
end)
Flush("level0")
task.spawn(function()
	local function Inner()
		error("parity-text level2", 2)
	end
	Inner()
end)
Flush("level2")
task.spawn(function()
	local Missing = nil
	return Missing["parity-text field"]
end)
Flush("index")
local Event = Instance.new("BindableEvent")
Event.Event:Connect(function()
	error("parity-text handler")
end)
Event:Fire()
Flush("handler")
task.defer(function()
	error("parity-text deferred")
end)
Flush("deferred")
local Module = coroutine.wrap(function()
	error("parity-text wrapped")
end)
task.spawn(Module)
Flush("coroutine-wrap")
local Getfenv = getfenv(1)
Getfenv.warn("parity-text warn", 1, nil)
Getfenv.print("parity-text print\twith tab")
Flush("warn-print")
Listener:Disconnect()
ErrorListener:Disconnect()
