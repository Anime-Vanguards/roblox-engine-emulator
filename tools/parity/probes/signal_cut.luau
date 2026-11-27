-- A queued handler whose connection is disconnected (or whose :Once() fired) before it runs, and a thread waiting on a signal
-- that fires past the re-entrancy cut: what runs and what is logged, in order.
local LogService = game:GetService("LogService")
P.sync()
local Seen = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind ~= Enum.MessageType.MessageOutput then
		table.insert(Seen, (string.gsub(string.split(Message, "\n")[1], " on line %d+ in .*$", "")))
	end
end)
local function Logged()
	for _ = 1, 4 do
		task.wait()
	end
	local Text = table.concat(Seen, " | ")
	table.clear(Seen)
	return "[" .. Text .. "]"
end
local function Case(Label, Run)
	local Log = {}
	local function Note(Text)
		table.insert(Log, Text)
	end
	P.sync()
	local Ok, Problem = pcall(Run, Note)
	if not Ok then
		Note("error " .. P.err(Problem))
	end
	local Text = Logged()
	print(Label .. ": " .. table.concat(Log, ",") .. " " .. Text)
end
Case("connect, fire, disconnect", function(Note)
	local E = Instance.new("BindableEvent")
	local C = E.Event:Connect(function(V)
		Note("ran " .. tostring(V))
	end)
	E:Fire(1)
	C:Disconnect()
	Note("connected " .. tostring(C.Connected))
end)
Case("connect, fire twice, disconnect", function(Note)
	local E = Instance.new("BindableEvent")
	local C = E.Event:Connect(function(V)
		Note("ran " .. tostring(V))
	end)
	E:Fire(1)
	E:Fire(2)
	C:Disconnect()
end)
Case("once, fire, disconnect", function(Note)
	local E = Instance.new("BindableEvent")
	local C = E.Event:Once(function(V)
		Note("ran " .. tostring(V))
	end)
	E:Fire(1)
	Note("connected " .. tostring(C.Connected))
	C:Disconnect()
	Note("connected " .. tostring(C.Connected))
end)
Case("once, fire twice", function(Note)
	local E = Instance.new("BindableEvent")
	local C = E.Event:Once(function(V)
		Note("ran " .. tostring(V))
	end)
	E:Fire(1)
	E:Fire(2)
	Note("connected " .. tostring(C.Connected))
end)
Case("a handler disconnects an older one queued", function(Note)
	local E = Instance.new("BindableEvent")
	local Old
	Old = E.Event:Connect(function(V)
		Note("old " .. tostring(V))
	end)
	E.Event:Connect(function(V)
		Note("new " .. tostring(V))
		Old:Disconnect()
	end)
	E:Fire(1)
end)
Case("a handler disconnects itself, two fires queued", function(Note)
	local E = Instance.new("BindableEvent")
	local C
	C = E.Event:Connect(function(V)
		Note("ran " .. tostring(V))
		C:Disconnect()
	end)
	E:Fire(1)
	E:Fire(2)
end)
Case("disconnect, then the same function connected again", function(Note)
	local E = Instance.new("BindableEvent")
	local function H(V)
		Note("ran " .. tostring(V))
	end
	local C = E.Event:Connect(H)
	E:Fire(1)
	C:Disconnect()
	E.Event:Connect(H)
end)
Case("fire, destroy", function(Note)
	local E = Instance.new("BindableEvent")
	E.Event:Connect(function(V)
		Note("ran " .. tostring(V))
	end)
	E:Fire(1)
	E:Destroy()
end)
Case("Changed, disconnect", function(Note)
	local V = Instance.new("IntValue")
	local C = V.Changed:Connect(function(X)
		Note("ran " .. tostring(X))
	end)
	V.Value = 5
	C:Disconnect()
end)
-- a thread waiting on an event fired at the end of a task.defer chain Depth steps long
for _, Depth in { 79, 80 } do
	Case("a waiting thread, fired at step " .. Depth, function(Note)
		local E = Instance.new("BindableEvent")
		local Waiter = task.spawn(function()
			local V = E.Event:Wait()
			Note("waiter " .. tostring(V))
		end)
		E.Event:Connect(function(V)
			Note("handler " .. tostring(V))
		end)
		local Steps = 0
		local function Next()
			Steps += 1
			if Steps < Depth then
				task.defer(Next)
			else
				E:Fire(Steps)
				-- (a task.defer here logs its own cut at once: the order shows when the fire logged)
				task.defer(function() end)
				Note("fired " .. coroutine.status(Waiter))
			end
		end
		task.defer(Next)
		for _ = 1, 3 do
			task.wait()
		end
		E:Fire("again")
		for _ = 1, 2 do
			task.wait()
		end
		Note("later " .. coroutine.status(Waiter))
	end)
end
Case("a self-firing event with a thread waiting again each time", function(Note)
	local E = Instance.new("BindableEvent")
	local Resumed = 0
	task.spawn(function()
		while Resumed < 200 do
			E.Event:Wait()
			Resumed += 1
		end
	end)
	local Fired = 0
	E.Event:Connect(function()
		Fired += 1
		if Fired < 120 then
			E:Fire()
		end
	end)
	E:Fire()
	for _ = 1, 3 do
		task.wait()
	end
	Note("fired " .. Fired .. " resumed " .. Resumed)
end)
Case("once past the cut", function(Note)
	local E = Instance.new("BindableEvent")
	local C = E.Event:Once(function(V)
		Note("once " .. tostring(V))
	end)
	local Steps = 0
	local function Next()
		Steps += 1
		if Steps < 80 then
			task.defer(Next)
		else
			E:Fire("deep")
			Note("connected " .. tostring(C.Connected))
		end
	end
	task.defer(Next)
	for _ = 1, 3 do
		task.wait()
	end
	E:Fire("again")
	for _ = 1, 2 do
		task.wait()
	end
	Note("connected later " .. tostring(C.Connected))
end)
Listener:Disconnect()
