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

-- 5b. the copy in detail: which keys survive and as what, nested and shared tables, values inside tables, trailing
-- nils, and BindableFunction:Invoke's arguments and results
local function KeyText(Key)
	local Kind = typeof(Key)
	if Kind == "string" or Kind == "number" or Kind == "boolean" then
		return Kind .. ":" .. (string.gsub(tostring(Key), "0x%x+", "ADDR"))
	end
	return Kind
end
local function Describe(Value, Depth)
	if type(Value) ~= "table" then
		if type(Value) == "string" or type(Value) == "number" or type(Value) == "boolean" then
			return tostring(Value)
		end
		return typeof(Value)
	end
	if (Depth or 0) > 3 then
		return "{...}"
	end
	local Entries = {}
	for Key, Item in Value do
		table.insert(Entries, KeyText(Key) .. "=" .. Describe(Item, (Depth or 0) + 1))
	end
	table.sort(Entries)
	local Flags = (if getmetatable(Value) ~= nil then " meta" else "") .. (if table.isfrozen(Value) then " frozen" else "")
	return "{" .. table.concat(Entries, " ") .. Flags .. "}"
end
local Copier = Instance.new("BindableEvent")
local Arrived
Copier.Event:Connect(function(...)
	Arrived = table.pack(...)
end)
local function Through(Label, ...)
	Arrived = nil
	local Outcome = P.try(Copier.Fire, Copier, ...)
	task.wait()
	if Arrived == nil then
		print("copy " .. Label .. " " .. Outcome .. " | nothing arrived")
		return nil
	end
	local Parts = {}
	for Index = 1, Arrived.n do
		table.insert(Parts, Describe(Arrived[Index]))
	end
	print("copy " .. Label .. " " .. Outcome .. " | n=" .. Arrived.n .. " " .. table.concat(Parts, " , "))
	return Arrived
end
Through("float-key", { [1.5] = "a" })
Through("bool-key", { [true] = "t" })
Through("instance-key", { [Part] = "p" })
Through("function-key", { [math.floor] = 1 })
Through("gap-start", { [2] = "b", [5] = "e" })
Through("zero-key", { [0] = "z", "a", "b" })
Through("negative-key", { [-1] = "n" })
Through("fraction-keys", { [-1.5] = "a", [2.7] = "b" })
Through("big-keys", { [2 ^ 31] = "a", [1e20] = "b", [math.huge] = "c" })
Through("table-key", { [{}] = 1 })
Through("vector-key", { [Vector3.new(1, 2, 3)] = 1 })
Through("enum-key", { [Enum.Material.Plastic] = 1 })
Through("thread-key", { [coroutine.create(function() end)] = 1 })
Through("buffer-key", { [buffer.create(1)] = 1 })
Through("string-and-number-key", { ["1"] = "s", [1.5] = "n" })
Through("thread-arg", coroutine.create(function() end))
Through("thread-in-array", { "a", coroutine.create(function() end), "c" })
Through("long-sparse", { [1] = "a", [2] = "b", [100] = "z" })
Through("string-number-keys", { ["1"] = "x", ["2"] = "y" })
Through("constructor-hole", { "a", nil, "c" })
Through("nested-mixed", { { 1, 2, K = 1 } })
Through("nested-meta", { setmetatable({ 1 }, { __index = {} }) })
Through("frozen", table.freeze({ 1 }))
Through("function-value", { F = math.floor })
Through("thread-value", { T = coroutine.create(function() end) })
Through("instance-value", { I = Part })
Through("datatype-values", { V = Vector3.new(1, 2, 3), C = Color3.new(1, 0, 0), E = Enum.Material.Plastic })
Through("buffer-value", { B = buffer.create(2) })
Through("empty", {})
Through("trailing-nil", "x", nil)
Through("only-nils", nil, nil)
local Shared = { 1 }
local Got = Through("shared-subtable", { A = Shared, B = Shared })
print("copy shared-subtable same " .. tostring(Got ~= nil and Got[1].A == Got[1].B))
local Twice = { 1 }
Got = Through("same-table-twice", Twice, Twice)
print("copy same-table-twice same " .. tostring(Got ~= nil and Got[1] == Got[2]))
local Buffer = buffer.create(2)
Got = Through("buffer-arg", Buffer)
print("copy buffer-arg same " .. tostring(Got ~= nil and Got[1] == Buffer))
local Callback = function() end
Got = Through("function-arg", Callback)
print("copy function-arg same " .. tostring(Got ~= nil and Got[1] == Callback))
-- one copy per fire, or one per handler (and what a waiting thread gets)
local Pair = Instance.new("BindableEvent")
local Seen = {}
Pair.Event:Connect(function(Value)
	table.insert(Seen, Value)
end)
Pair.Event:Connect(function(Value)
	table.insert(Seen, Value)
end)
local Waited
task.spawn(function()
	Waited = Pair.Event:Wait()
end)
local ToPair = { 1 }
Pair:Fire(ToPair)
task.wait()
print("copy two-handlers same " .. tostring(Seen[1] == Seen[2]) .. " wait-is-sent " .. tostring(Waited == ToPair) .. " wait-is-handler " .. tostring(Waited == Seen[1] or Waited == Seen[2]))
local Invoked = Instance.new("BindableFunction")
local Inside
Invoked.OnInvoke = function(Value)
	Inside = Value
	return Value, setmetatable({ 1, Key = "k" }, {})
end
local Argument = { 1, Key = "k" }
local Back, Second = Invoked:Invoke(Argument)
print("invoke arg-same " .. tostring(Inside == Argument) .. " back-same " .. tostring(Back == Argument) .. " back-is-inside " .. tostring(Back == Inside))
print("invoke inside " .. Describe(Inside) .. " back " .. Describe(Back) .. " second " .. Describe(Second))
print("invoke cyclic " .. P.try(Invoked.Invoke, Invoked, Cyclic))
Invoked.OnInvoke = function()
	return Cyclic
end
print("invoke cyclic-result " .. P.try(Invoked.Invoke, Invoked))

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
-- what firing a connection to a value that is not a function does
local NotFunctions = Instance.new("BindableEvent")
local Messages = {}
local MessageWatch = game:GetService("LogService").MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageError then
		table.insert(Messages, P.err((string.split(Message, "\n"))[1]))
	end
end)
local NilConnection = NotFunctions.Event:Connect(nil)
NotFunctions.Event:Connect(5)
NotFunctions.Event:Connect(function()
	Note("function-handler")
end)
NotFunctions:Fire()
task.wait()
task.wait()
MessageWatch:Disconnect()
print("connect-nil-fire connected " .. tostring(NilConnection.Connected) .. " logged " .. #Messages .. " " .. table.concat(Messages, " | "))
Flush("connect-nil-handlers")
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
