-- Where deferred work runs: threads deferred while deferred threads run, events fired from handlers, task.defer from a
-- handler, and the order of a frame's waiting threads, delays and the flush after them.
local RunService = game:GetService("RunService")
P.sync()
local Log = {}
local function Note(Text)
	table.insert(Log, Text)
end
local Marks = {}
for _, Name in { "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	table.insert(Marks, RunService[Name]:Connect(function()
		Note("[" .. Name .. "]")
	end))
end
local function Done(Label, Frames)
	for _ = 1, Frames or 2 do
		task.wait()
	end
	-- from the first entry up to the second [Heartbeat]
	local Out = {}
	local Beats = 0
	for _, Entry in Log do
		if Entry == "[Heartbeat]" then
			Beats += 1
			if Beats > 2 then
				break
			end
		end
		table.insert(Out, Entry)
	end
	print(Label .. " " .. table.concat(Out, ","))
	table.clear(Log)
end

-- 1. a chain of task.defer from the wait resumption point
P.sync()
table.clear(Log)
Note("start")
task.defer(function()
	Note("d1")
	task.defer(function()
		Note("d2")
		task.defer(function()
			Note("d3")
		end)
	end)
end)
Done("defer-chain", 3)

-- 2. a chain of events: a handler fires another event, whose handler fires a third
P.sync()
table.clear(Log)
local E1, E2, E3 = Instance.new("BindableEvent"), Instance.new("BindableEvent"), Instance.new("BindableEvent")
E1.Event:Connect(function()
	Note("e1")
	E2:Fire()
end)
E2.Event:Connect(function()
	Note("e2")
	E3:Fire()
end)
E3.Event:Connect(function()
	Note("e3")
end)
Note("start")
E1:Fire()
Done("event-chain", 3)

-- 3. a handler that defers, and a deferred thread that fires an event
P.sync()
table.clear(Log)
local F1, F2 = Instance.new("BindableEvent"), Instance.new("BindableEvent")
F1.Event:Connect(function()
	Note("f1")
	task.defer(function()
		Note("f1-defer")
	end)
	F2:Fire()
end)
F2.Event:Connect(function()
	Note("f2")
end)
Note("start")
task.defer(function()
	Note("d")
	F1:Fire()
end)
F1:Fire()
Done("mixed", 3)

-- 4. a property write from a deferred thread and from a handler
P.sync()
table.clear(Log)
local Value = Instance.new("IntValue")
Value.Changed:Connect(function(New)
	Note("changed" .. New)
	if New == 1 then
		task.defer(function()
			Note("defer-from-changed")
			Value.Value = 2
		end)
	end
end)
Note("start")
Value.Value = 1
Done("property", 3)

-- 5. spawn from a deferred thread, and a thread waiting on a signal
P.sync()
table.clear(Log)
local Signal = Instance.new("BindableEvent")
task.spawn(function()
	Signal.Event:Wait()
	Note("waiter")
	task.defer(function()
		Note("waiter-defer")
	end)
end)
Signal.Event:Connect(function()
	Note("handler")
end)
Note("start")
task.defer(function()
	Note("d")
	task.spawn(function()
		Note("spawned")
	end)
	Signal:Fire()
	Note("d-end")
end)
Done("spawn-and-wait", 3)

-- 6. waits and delays that come due together, with deferred work from each
P.sync()
table.clear(Log)
task.delay(0, function()
	Note("delay-a")
	task.defer(function()
		Note("delay-a-defer")
	end)
end)
task.spawn(function()
	task.wait()
	Note("wait-a")
	task.defer(function()
		Note("wait-a-defer")
	end)
end)
task.delay(0, function()
	Note("delay-b")
end)
task.spawn(function()
	task.wait()
	Note("wait-b")
end)
task.spawn(function()
	wait()
	Note("legacy-wait")
end)
delay(0, function()
	Note("legacy-delay")
end)
spawn(function()
	Note("legacy-spawn")
end)
Note("start")
Done("due-together", 3)

-- 7. the same from inside a Heartbeat handler (another resumption point)
P.sync()
table.clear(Log)
RunService.Heartbeat:Once(function()
	Note("hb-handler")
	task.defer(function()
		Note("hb-defer")
		task.defer(function()
			Note("hb-defer2")
		end)
	end)
	task.delay(0, function()
		Note("hb-delay")
	end)
	task.spawn(function()
		task.wait()
		Note("hb-wait")
	end)
end)
Done("from-heartbeat", 3)

for _, Connection in Marks do
	Connection:Disconnect()
end
