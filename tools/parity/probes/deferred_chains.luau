-- Long chains of deferred work, to count the invocation points a frame has and see where each step lands: a task.defer
-- chain, a BindableEvent that fires itself, a Changed handler that writes its own property, and a chain of events
-- across two BindableEvents.
local RunService = game:GetService("RunService")
P.sync()
local Log = {}
local Frame = 0
local Marks = {}
for _, Name in { "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	table.insert(Marks, RunService[Name]:Connect(function()
		if Name == "Heartbeat" then
			Frame += 1
		end
		table.insert(Log, "|" .. string.sub(Name, 1, 4))
	end))
end
local function Run(Label, Start)
	P.sync()
	table.clear(Log)
	local Steps = 0
	Start(function()
		Steps += 1
		table.insert(Log, tostring(Steps))
		return Steps
	end)
	for _ = 1, 4 do
		task.wait()
	end
	print(Label .. " steps " .. Steps .. ": " .. table.concat(Log, " "))
end

Run("defer-chain", function(Step)
	local function Next()
		if Step() < 40 then
			task.defer(Next)
		end
	end
	task.defer(Next)
end)

Run("self-firing-event", function(Step)
	local Event = Instance.new("BindableEvent")
	Event.Event:Connect(function()
		if Step() < 120 then
			Event:Fire()
		end
	end)
	Event:Fire()
end)

Run("changed-chain", function(Step)
	local Value = Instance.new("IntValue")
	Value.Changed:Connect(function()
		if Step() < 120 then
			Value.Value += 1
		end
	end)
	Value.Value = 1
end)

Run("ping-pong-events", function(Step)
	local A, B = Instance.new("BindableEvent"), Instance.new("BindableEvent")
	A.Event:Connect(function()
		if Step() < 120 then
			B:Fire()
		end
	end)
	B.Event:Connect(function()
		if Step() < 120 then
			A:Fire()
		end
	end)
	A:Fire()
end)

Run("event-then-defer", function(Step)
	local Event = Instance.new("BindableEvent")
	local function Next()
		if Step() < 40 then
			Event:Fire()
		end
	end
	Event.Event:Connect(function()
		if Step() < 40 then
			task.defer(Next)
		end
	end)
	Event:Fire()
end)

Run("spawn-in-defer-chain", function(Step)
	local function Next()
		if Step() < 30 then
			task.spawn(function()
				task.defer(Next)
			end)
		end
	end
	task.defer(Next)
end)

for _, Connection in Marks do
	Connection:Disconnect()
end
