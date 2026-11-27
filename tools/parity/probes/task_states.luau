-- The task library given a thread another of its calls holds (deferred, delayed, waiting in task.wait) or one it cannot resume:
-- what each call answers, and the whole error a client logs for a thread it cannot resume (its kind, its stack, ScriptContext.Error).
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
P.sync()
local Chunk = P.chunk()
-- (the probe's own lines: a frame past them is whatever runs the probe)
local ProbeLines = 190
-- a logged stack's frames of the probe itself, as "@<line>[ - function <name>]"; the frames of whatever runs the probe are left out
local function Frames(Text)
	local Out = {}
	for _, Line in string.split(Text, "\n") do
		local Name, Number, Rest = string.match(Line, "^%s*Script '(.-)', Line (%d+)(.*)$")
		if Name == Chunk then
			local At = tonumber(Number) - P.base
			if At > 0 and At <= ProbeLines then
				table.insert(Out, "@" .. At .. Rest)
			end
		end
	end
	return table.concat(Out, ",")
end
local function TraceFrames(Trace)
	local Out = {}
	for _, Line in string.split(Trace, "\n") do
		local Name, Number, Rest = string.match(Line, "^(.-), line (%d+)(.*)$")
		if Name == Chunk then
			local At = tonumber(Number) - P.base
			if At > 0 and At <= ProbeLines then
				table.insert(Out, "@" .. At .. Rest)
			end
		end
	end
	return table.concat(Out, ",")
end
local Seen = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind ~= Enum.MessageType.MessageOutput then
		table.insert(Seen, Kind.Name .. ": " .. string.split(Message, "\n")[1] .. " {" .. Frames(Message) .. "}")
	end
end)
local ErrorListener = ScriptContext.Error:Connect(function(Message, Trace)
	table.insert(Seen, "ScriptContext.Error: " .. Message .. " {" .. TraceFrames(Trace) .. "}")
end)
local function Logged()
	for _ = 1, 4 do
		task.wait()
	end
	local Text = table.concat(Seen, " | ")
	table.clear(Seen)
	return "[" .. Text .. "]"
end
local function Errored()
	local Thread = coroutine.create(function()
		error("inner", 0)
	end)
	coroutine.resume(Thread)
	return Thread
end
local function Deep()
	local function F(K)
		if K == 0 then
			error("inner", 0)
		end
		F(K - 1)
	end
	local Thread = coroutine.create(function()
		F(2)
	end)
	coroutine.resume(Thread)
	return Thread
end

-- the error a client logs, in full
local Marks = {}
local T = Errored()
table.insert(Marks, "before")
task.spawn(T)
table.insert(Marks, "after")
print("errored spawn " .. table.concat(Marks, ",") .. " " .. Logged())
print("errored three deep spawn " .. Logged() .. P.try(function()
	return typeof(task.spawn(Deep()))
end) .. " " .. Logged())
T = Errored()
table.clear(Marks)
task.defer(T)
table.insert(Marks, "after defer")
task.defer(table.insert, Marks, "deferred mark")
print("errored defer " .. Logged() .. " " .. table.concat(Marks, ","))
print("errored delay " .. P.try(function()
	return typeof(task.delay(0, Errored()))
end) .. " " .. Logged())
print("running spawn " .. P.try(function()
	return task.spawn(coroutine.running()) == coroutine.running()
end) .. " " .. Logged())

-- a thread another call holds
local DeferSelf, DeferSelfWait = "pending", "pending"
task.spawn(function()
	DeferSelf = P.try(function()
		return task.defer(coroutine.running()) == coroutine.running()
	end)
	DeferSelfWait = P.try(task.wait)
end)
print("running defer then wait " .. DeferSelf .. " / " .. DeferSelfWait .. " " .. Logged())
local DeferTwice = "pending"
task.spawn(function()
	task.defer(coroutine.running())
	DeferTwice = P.try(function()
		return typeof(task.defer(coroutine.running()))
	end)
end)
print("running defer twice " .. DeferTwice .. " " .. Logged())
local function Deferred()
	local Thread = coroutine.create(function() end)
	task.defer(Thread)
	return Thread
end
local function Delayed()
	return task.delay(0.2, function() end)
end
local function Waiting()
	return task.spawn(function()
		task.wait(0.3)
	end)
end
for _, Case in { { "deferred", Deferred }, { "delayed", Delayed }, { "waiting", Waiting } } do
	local Thread = Case[2]()
	print(Case[1] .. " spawn " .. P.try(function()
		return typeof(task.spawn(Thread))
	end) .. " " .. coroutine.status(Thread) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " spawn-args " .. P.try(function()
		return typeof(task.spawn(Thread, 1))
	end) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " defer " .. P.try(function()
		return typeof(task.defer(Thread))
	end) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " delay " .. P.try(function()
		return typeof(task.delay(0, Thread))
	end) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " cancel " .. P.try(task.cancel, Thread) .. " " .. coroutine.status(Thread) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " resume " .. P.try(coroutine.resume, Thread) .. " " .. Logged())
end
task.wait(0.5)
-- one that waits elsewhere
local Legacy = task.spawn(function()
	wait(0.3)
end)
print("legacy-waiting spawn " .. P.try(function()
	return typeof(task.spawn(Legacy))
end) .. " " .. coroutine.status(Legacy) .. " " .. Logged())
local Signal = Instance.new("BindableEvent")
local SignalWaiter = task.spawn(function()
	Signal.Event:Wait()
end)
print("signal-waiting spawn " .. P.try(function()
	return typeof(task.spawn(SignalWaiter))
end) .. " " .. coroutine.status(SignalWaiter) .. " " .. Logged())
local Yielded = task.spawn(function()
	coroutine.yield()
end)
print("yielded spawn-args " .. P.try(function()
	return typeof(task.spawn(Yielded, 1))
end) .. " " .. coroutine.status(Yielded) .. " " .. Logged())
task.wait(0.5)
print("after the waits " .. Logged())
Listener:Disconnect()
ErrorListener:Disconnect()
