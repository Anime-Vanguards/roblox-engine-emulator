-- LogService.MessageOut and ScriptContext.Error on a playing client: when a logged line reaches their handlers (against
-- the frame's events, deferred work and the logging call itself), with what text and MessageType, and in what order.
-- The probe logs through the environment's own warn, print and error: the harnesses that run it may shadow print and
-- warn with their own collectors, and only the real ones reach LogService. (This probe's real print adds one line to an
-- engine whose harness collects real prints; nothing compared depends on it.)
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
local Env = getfenv(1)
local RealPrint, RealWarn = Env.print, Env.warn
P.sync()

local Log = {}
local function Mark(Text)
	table.insert(Log, Text)
end
local Marks = {}
for _, Name in { "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	table.insert(Marks, RunService[Name]:Connect(function()
		Mark("|" .. string.sub(Name, 1, 4))
	end))
end
-- while CaptureAll is set every line is kept (a script error's traceback lines carry no marker), with the probe's own
-- script name and lines made comparable
local CaptureAll = false
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local function Normal(Text)
	Text = string.gsub(Text, "Script '" .. Escaped .. "', Line (%d+)", function(Line)
		return "Script '<probe>', Line @" .. (tonumber(Line) - P.base)
	end)
	return (string.gsub(P.err(Text), "0x%x+", "ADDR"))
end
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if CaptureAll or string.find(Message, "parity%-log") then
		local Lines = string.split(Message, "\n")
		Mark("out(" .. Kind.Name .. ":" .. Normal(Lines[1]) .. (if #Lines > 1 then " +" .. (#Lines - 1) .. " lines" else "") .. ")")
	end
end)
local ErrorListener = ScriptContext.Error:Connect(function(Message, Trace, Script)
	if string.find(Message, "parity%-log") then
		Mark("error(" .. P.err(Message) .. ";" .. typeof(Script) .. ")")
	end
end)
local function Flush(Label, Frames)
	for _ = 1, Frames or 3 do
		task.wait()
	end
	-- a frame boundary mark is only kept between entries that are not boundary marks (a headless client's frame count
	-- between the log and its delivery is what is compared, not how many idle frames the wait took)
	local Kept = {}
	for _, Entry in Log do
		table.insert(Kept, Entry)
	end
	while #Kept > 0 and string.sub(Kept[#Kept], 1, 1) == "|" do
		table.remove(Kept)
	end
	print(Label .. " " .. table.concat(Kept, ","))
	table.clear(Log)
end

-- 1. a warning: inside the call, after it, or later? before or after the deferred work queued beside it?
RealWarn("parity-log warn one")
Mark("after-call")
task.defer(Mark, "deferred")
local Event = Instance.new("BindableEvent")
Event.Event:Connect(function()
	Mark("event")
end)
Event:Fire()
Mark("end-of-step")
Flush("warn")

-- 2. a print with several arguments, two warnings in a row
RealPrint("parity-log print", 1, nil, true)
RealWarn("parity-log warn two")
RealWarn("parity-log warn three")
Mark("after-calls")
Flush("print-and-warns")

-- 3. from a Heartbeat handler
local Once
Once = RunService.Heartbeat:Connect(function()
	Once:Disconnect()
	RealWarn("parity-log from heartbeat")
	Mark("hb-after-call")
end)
Flush("from-heartbeat", 4)

-- 4. from a deferred thread
task.defer(function()
	RealWarn("parity-log from defer")
	Mark("defer-after-call")
end)
Flush("from-defer")

-- 5. a script error in a spawned thread: the error line, ScriptContext.Error, and every line logged with it
CaptureAll = true
task.spawn(error, "parity-log spawned error")
Mark("after-spawn")
Flush("spawned-error")
task.spawn(function()
	local Missing = nil
	return Missing["parity-log field"]
end)
Flush("index-error")
task.spawn(function()
	error({ "parity-log table error" })
end)
Flush("table-error")
CaptureAll = false

-- 6. an error in an event handler, and one in a deferred thread
local Failing = Instance.new("BindableEvent")
Failing.Event:Connect(function()
	error("parity-log handler error")
end)
Failing:Fire()
Mark("after-fire")
Flush("handler-error")
task.defer(error, "parity-log deferred error")
Flush("deferred-error")

-- 7. an engine message: a failed Connect logs an error
Event.Event:Connect(nil)
Mark("after-connect")
Flush("engine-message")

-- 8. a warning logged inside a MessageOut handler (one level)
local Nested
Nested = LogService.MessageOut:Connect(function(Message)
	if Message == "parity-log outer" then
		RealWarn("parity-log inner")
		Mark("nested-after-call")
	end
end)
RealWarn("parity-log outer")
Flush("nested")
Nested:Disconnect()

-- 9. what the history holds
local Got, History = pcall(LogService.GetLogHistory, LogService)
local Seen = { "got=" .. tostring(Got) .. ":" .. typeof(History) }
for _, Entry in if Got and type(History) == "table" then History else {} do
	if typeof(Entry) == "table" and type(Entry.message) == "string" and string.find(Entry.message, "parity%-log warn") then
		table.insert(Seen, Entry.messageType.Name .. ":" .. Entry.message .. ":" .. type(Entry.timestamp))
	end
end
print("history " .. table.concat(Seen, ","))

Listener:Disconnect()
ErrorListener:Disconnect()
for _, Connection in Marks do
	Connection:Disconnect()
end
