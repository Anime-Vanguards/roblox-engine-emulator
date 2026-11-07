-- RBXScriptSignal: handler order, Once, connecting and disconnecting during a fire, Wait, what Fire passes, re-entrancy.
P.sync()
local Log = {}
local function Note(Text)
	table.insert(Log, Text)
end
local function Flush(Label)
	print(Label .. " " .. table.concat(Log, ","))
	table.clear(Log)
end

-- 1. handler order on a BindableEvent and on a property signal
local Bindable = Instance.new("BindableEvent")
for Index = 1, 5 do
	Bindable.Event:Connect(function()
		Note("h" .. Index)
	end)
end
Bindable:Fire()
Note("after-fire")
task.wait()
Flush("bindable-order")
local Part = Instance.new("Part")
for Index = 1, 4 do
	Part:GetPropertyChangedSignal("Transparency"):Connect(function()
		Note("p" .. Index)
	end)
	Part.Changed:Connect(function(Name)
		if Name == "Transparency" then
			Note("c" .. Index)
		end
	end)
end
Part.Transparency = 0.5
Note("after-write")
task.wait()
Flush("property-order")

-- 2. Once, and a Once that fires twice in a batch
local OnceEvent = Instance.new("BindableEvent")
local OnceConnection = OnceEvent.Event:Once(function(Value)
	Note("once " .. tostring(Value))
end)
OnceEvent.Event:Connect(function(Value)
	Note("normal " .. tostring(Value))
end)
print("once-connected-before " .. tostring(OnceConnection.Connected))
OnceEvent:Fire(1)
OnceEvent:Fire(2)
print("once-connected-after-fire " .. tostring(OnceConnection.Connected))
task.wait()
print("once-connected-later " .. tostring(OnceConnection.Connected))
Flush("once")

-- 3. disconnecting a later handler from an earlier one, connecting during a fire
local Event = Instance.new("BindableEvent")
local Later
Event.Event:Connect(function(Value)
	Note("first " .. Value)
	if Later then
		Later:Disconnect()
		Later = nil
		Note("disconnected-later")
	end
	if Value == 1 then
		Event.Event:Connect(function(Inner)
			Note("added " .. Inner)
		end)
	end
end)
Later = Event.Event:Connect(function(Value)
	Note("later " .. Value)
end)
Event:Fire(1)
task.wait()
Event:Fire(2)
task.wait()
Flush("disconnect-during-fire")

-- disconnect before the deferred handler runs
local Pending = Instance.new("BindableEvent")
local PendingConnection = Pending.Event:Connect(function()
	Note("pending-ran")
end)
Pending:Fire()
PendingConnection:Disconnect()
task.wait()
Flush("disconnect-before-run")

-- 4. Wait: what it returns, where it resumes against handlers
local WaitEvent = Instance.new("BindableEvent")
WaitEvent.Event:Connect(function()
	Note("handler-a")
end)
task.spawn(function()
	local A, B, C = WaitEvent.Event:Wait()
	Note("waited " .. tostring(A) .. " " .. tostring(B) .. " " .. tostring(C))
end)
WaitEvent.Event:Connect(function()
	Note("handler-b")
end)
WaitEvent:Fire("x", nil, 3)
Note("fired")
task.wait()
Flush("wait")

-- 5. what Fire passes: tables are copied, metatables dropped, mixed tables, functions, instances
local Copy = Instance.new("BindableEvent")
local Sent = { 1, 2, Nested = { 3 } }
setmetatable(Sent, { __index = function() return "meta" end })
local Received
Copy.Event:Connect(function(Value, Second, Third)
	Received = { Value, Second, Third }
end)
Copy:Fire(Sent, Part, function() end)
task.wait()
local Got = Received[1]
print("table-same " .. tostring(Got == Sent) .. " nested-same " .. tostring(Got and Got.Nested == Sent.Nested) .. " meta " .. tostring(getmetatable(Got) ~= nil) .. " missing " .. tostring(Got and Got.Missing))
print("instance-same " .. tostring(Received[2] == Part) .. " function " .. type(Received[3]))
local Mixed
Copy.Event:Once(function(Value)
	Mixed = Value
end)
local MixedTable = { 1, 2, Key = "v" }
print("fire-mixed " .. P.try(Copy.Fire, Copy, MixedTable))
task.wait()
if type(Mixed) == "table" then
	local Keys = {}
	for Key in Mixed do
		table.insert(Keys, tostring(Key))
	end
	table.sort(Keys)
	print("mixed-received " .. table.concat(Keys, ","))
else
	print("mixed-received " .. type(Mixed))
end
local Sparse
Copy.Event:Once(function(Value)
	Sparse = Value
end)
print("fire-sparse " .. P.try(Copy.Fire, Copy, { [1] = "a", [3] = "c" }))
task.wait()
if type(Sparse) == "table" then
	print("sparse-received " .. tostring(Sparse[1]) .. " " .. tostring(Sparse[2]) .. " " .. tostring(Sparse[3]))
end
local NumberKeys
Copy.Event:Once(function(Value)
	NumberKeys = Value
end)
print("fire-float-keys " .. P.try(Copy.Fire, Copy, { [1.5] = "a" }))
task.wait()
print("float-keys-received " .. type(NumberKeys))
local Cyclic = {}
Cyclic.Self = Cyclic
print("fire-cyclic " .. P.try(Copy.Fire, Copy, Cyclic))
task.wait()

-- 6. re-entrancy: a handler that fires its own event, and a Changed handler that writes its own property
local Reentrant = Instance.new("BindableEvent")
local Depth = 0
Reentrant.Event:Connect(function()
	Depth += 1
	if Depth < 200 then
		Reentrant:Fire()
	end
end)
Reentrant:Fire()
local FramesTaken = 0
for _ = 1, 5 do
	task.wait()
	FramesTaken += 1
end
print("bindable-reentrant depth " .. Depth)
local Counter = Instance.new("IntValue")
local Writes = 0
Counter.Changed:Connect(function()
	Writes += 1
	if Writes < 200 then
		Counter.Value += 1
	end
end)
Counter.Value = 1
for _ = 1, 5 do
	task.wait()
end
print("changed-reentrant writes " .. Writes .. " value " .. Counter.Value)

-- 7. the connection object
local Signal = Instance.new("BindableEvent").Event
local Connection = Signal:Connect(function() end)
print("connection " .. typeof(Connection) .. " " .. tostring(Connection.Connected))
Connection:Disconnect()
Connection:Disconnect()
print("disconnected " .. tostring(Connection.Connected))
print("signal tostring " .. tostring(Signal) .. " | " .. tostring(Part.Changed) .. " | " .. tostring(Part:GetPropertyChangedSignal("Size")))
print("connect-nonfunction " .. P.try(Signal.Connect, Signal, 5))
print("connect-nil " .. P.try(Signal.Connect, Signal, nil))
print("bad-property-signal " .. P.try(Part.GetPropertyChangedSignal, Part, "NotAProperty"))
print("signal-equal " .. tostring(Part.Changed == Part.Changed) .. " " .. tostring(Part:GetPropertyChangedSignal("Size") == Part:GetPropertyChangedSignal("Size")))
local Errors = Instance.new("BindableEvent")
Errors.Event:Connect(function()
	error("handler failed")
end)
Errors.Event:Connect(function()
	Note("second-handler-still-runs")
end)
Errors:Fire()
task.wait()
Flush("handler-error")
