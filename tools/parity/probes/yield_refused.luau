-- Waits where the thread cannot yield (a table.sort comparator, a string.gsub callback, an __index metamethod): the error
-- each yielding engine call raises there (its text and position), whether a refused wait leaves anything behind (what
-- the thread's following waits return, and after how many frames), and what still works there (WaitForChild of a child
-- that exists, an Invoke whose handler does not yield, threads started there, the error of a spawned thread that dies on
-- the refusal).
local RunService = game:GetService("RunService")
local ScriptContext = game:GetService("ScriptContext")
P.sync()

local Frames = 0
local FrameConnection = RunService.Heartbeat:Connect(function()
	Frames += 1
end)

-- values as comparable text: numbers by kind (a wait's elapsed time differs run to run), instances by name
local function Show(...)
	local Parts = {}
	for Index = 1, select("#", ...) do
		local Value = select(Index, ...)
		local Kind = typeof(Value)
		if Kind == "number" then
			table.insert(Parts, "number")
		elseif Kind == "Instance" then
			table.insert(Parts, "Instance " .. Value.Name)
		elseif Kind == "string" or Kind == "boolean" or Kind == "nil" then
			table.insert(Parts, tostring(Value))
		else
			table.insert(Parts, Kind)
		end
	end
	return if #Parts == 0 then "(none)" else table.concat(Parts, " ")
end

-- Body's outcome where the thread cannot yield (an error with a position names Body's own line)
local function Refused(Context, Body)
	local Line
	local function Attempt()
		if Line then
			return
		end
		local Results = table.pack(pcall(Body))
		Line = (if coroutine.isyieldable() then "(yieldable) " else "")
			.. (if Results[1] then "ok " .. Show(table.unpack(Results, 2, Results.n)) else "error " .. P.err(Results[2]))
	end
	if Context == "sort" then
		table.sort({ 2, 1 }, function(A, B)
			Attempt()
			return A < B
		end)
	elseif Context == "gsub" then
		string.gsub("x", "x", function()
			Attempt()
			return "y"
		end)
	else
		local _ = setmetatable({}, {
			__index = function()
				Attempt()
				return 0
			end,
		}).Key
	end
	return Line
end

-- a case in a thread of its own, its lines printed once it ends (bounded: a thread that never ends is reported)
local function Case(Label, Body)
	local Lines = {}
	local Done = false
	task.spawn(function()
		Body(function(Text)
			table.insert(Lines, Text)
		end)
		Done = true
	end)
	local Started = os.clock()
	while not Done and os.clock() - Started < 4 do
		task.wait()
	end
	print(Label .. " " .. table.concat(Lines, " | ") .. (if Done then "" else " | unfinished"))
end

-- how long a wait took: no frame, less than its time, or its whole time (by the clock, so a slow frame counts as one)
local function Mark()
	return { Frames, os.clock() }
end
local function Span(Start, Seconds)
	return if Frames == Start[1] then "now" elseif os.clock() - Start[2] < Seconds - 0.03 then "early" else "full"
end

local function Elapsed(Value, Seconds)
	return if type(Value) ~= "number" then Show(Value) elseif Value >= Seconds - 0.01 then "elapsed-full" else "elapsed-short"
end

-- 1. where the thread cannot yield
for _, Context in { "sort", "gsub", "index" } do
	print(Context .. "-yieldable " .. tostring(Refused(Context, function()
		return coroutine.isyieldable()
	end)))
end

-- 2. each yielding call there, in each place
local Folder = Instance.new("Folder")
Folder.Name = "ParityYieldFolder"
local Existing = Instance.new("Folder")
Existing.Name = "Existing"
Existing.Parent = Folder
Folder.Parent = workspace
local Event = Instance.new("BindableEvent")
local Quiet = Instance.new("BindableFunction")
Quiet.OnInvoke = function(Value)
	return "handled " .. tostring(Value)
end
local Slow = Instance.new("BindableFunction")
local SlowLog = {}
Slow.OnInvoke = function(Value)
	table.insert(SlowLog, "start yieldable=" .. tostring(coroutine.isyieldable()))
	task.wait(0.1)
	table.insert(SlowLog, "resumed")
	return "slow " .. tostring(Value)
end
local Unhandled = Instance.new("BindableFunction")
local Remote = Instance.new("RemoteFunction")

local Calls = {
	{ "task.wait", function() return task.wait(0.1) end },
	{ "task.wait-none", function() return task.wait() end },
	{ "task.wait-string", function() return task.wait("x") end },
	{ "task.wait-table", function() return task.wait({}) end },
	{ "wait", function() return wait(0.1) end },
	{ "wait-none", function() return wait() end },
	{ "wait-string", function() return wait("x") end },
	{ "Wait", function() return Event.Event:Wait() end },
	{ "wait-lower", function() return Event.Event:wait() end },
	{ "Wait-indexed", function() return Event.Event.Wait(Event.Event) end },
	{ "Wait-misused", function() return Event.Event.Wait(5) end },
	{ "Heartbeat-Wait", function() return RunService.Heartbeat:Wait() end },
	{ "WaitForChild-missing", function() return Folder:WaitForChild("Missing") end },
	{ "WaitForChild-timeout", function() return Folder:WaitForChild("Missing", 0.1) end },
	{ "WaitForChild-existing", function() return Folder:WaitForChild("Existing") end },
	{ "WaitForChild-nil", function() return Folder:WaitForChild(nil) end },
	{ "WaitForChild-bad-timeout", function() return Folder:WaitForChild("Missing", "x") end },
	{ "Invoke-quiet", function() return Quiet:Invoke(1) end },
	{ "Invoke-slow", function() return Slow:Invoke(2) end },
	{ "Invoke-unhandled", function() return Unhandled:Invoke(3) end },
	{ "InvokeServer", function() return Remote:InvokeServer() end },
	{ "coroutine.yield", function() return coroutine.yield(4) end },
	{ "synchronize", function() return task.synchronize() end },
	{ "desynchronize", function() return task.desynchronize() end },
	{ "PreloadAsync", function() return game:GetService("ContentProvider"):PreloadAsync({}) end },
	{ "PreloadAsync-folder", function() return game:GetService("ContentProvider"):PreloadAsync({ Existing }) end },
}
-- (InvokeClient to the session's player, on a client the local one, and to nil: which error comes first)
local Player = game:GetService("Players"):GetPlayers()[1]
table.insert(Calls, { "InvokeClient", function() return Remote:InvokeClient(Player) end })
table.insert(Calls, { "InvokeClient-nil", function() return Remote:InvokeClient(nil) end })
for _, Context in { "sort", "gsub", "index" } do
	for _, Call in Calls do
		Case(Context .. "-" .. Call[1], function(Note)
			Note(Refused(Context, Call[2]))
		end)
		if Call[1] == "Invoke-slow" then
			-- the handler's own thread: whether it went on after the refusal
			task.wait(0.2)
			print(Context .. "-Invoke-slow-handler " .. table.concat(SlowLog, ","))
			table.clear(SlowLog)
		end
	end
end
-- the same arguments where the thread can yield, for their errors
for _, Name in { "task.wait-string", "task.wait-table", "wait-string", "Wait-misused", "WaitForChild-nil", "InvokeClient-nil" } do
	for _, Call in Calls do
		if Call[1] == Name then
			Case("yieldable-" .. Name, function(Note)
				local Results = table.pack(pcall(Call[2]))
				Note(if Results[1] then "ok " .. Show(table.unpack(Results, 2, Results.n)) else "error " .. P.err(Results[2]))
			end)
		end
	end
end

-- whether PreloadAsync yields where it can, for a list with nothing to load
Case("yieldable-PreloadAsync", function(Note)
	for _, Items in { {}, { Existing } } do
		local Start = Frames
		local Deferred = false
		task.defer(function()
			Deferred = true
		end)
		local Results = Show(game:GetService("ContentProvider"):PreloadAsync(Items))
		Note(`{#Items} {Results} yielded={Deferred} frame-passed={Frames > Start}`)
	end
end)

-- 3. a refused wait, then the thread's following waits: what each returns and after how many frames, while what the
-- refused wait waited for happens (0.1 s on)
local Refusals = {
	{ "task.wait", function(_) return task.wait(0.1) end },
	{ "wait", function(_) return wait(0.1) end },
	{ "Wait", function(Things)
		task.delay(0.1, function() Things.Event:Fire("fired-refused") end)
		return Things.Event.Event:Wait()
	end },
	{ "WaitForChild", function(Things)
		task.delay(0.1, function()
			local Child = Instance.new("Folder")
			Child.Name = "Soon"
			Child.Parent = Things.Folder
		end)
		return Things.Folder:WaitForChild("Soon")
	end },
	{ "WaitForChild-timeout", function(Things) return Things.Folder:WaitForChild("Never", 0.1) end },
	{ "Invoke", function(Things) return Things.Function:Invoke("refused") end },
	{ "coroutine.yield", function(_) return coroutine.yield() end },
}
local Follows = {
	{ "waits", function(_, Note)
		for Index = 1, 3 do
			local Start = Mark()
			local Value = task.wait(0.2)
			Note(`wait{Index} {Span(Start, 0.2)} {Elapsed(Value, 0.2)}`)
		end
	end },
	{ "legacy-wait", function(_, Note)
		local Start = Mark()
		local Value = wait(0.2)
		Note(`wait {Span(Start, 0.2)} {Elapsed(Value, 0.2)}`)
	end },
	{ "event", function(Things, Note)
		local Other = Instance.new("BindableEvent")
		task.delay(0.3, function() Other:Fire("fired-later") end)
		local Start = Mark()
		local Got = Show(Other.Event:Wait())
		Note(`Wait {Span(Start, 0.3)} {Got}`)
	end },
	{ "child", function(Things, Note)
		task.delay(0.3, function()
			local Child = Instance.new("Folder")
			Child.Name = "Later"
			Child.Parent = Things.Folder
		end)
		local Start = Mark()
		local Got = Show(Things.Folder:WaitForChild("Later"))
		Note(`WaitForChild {Span(Start, 0.3)} {Got}`)
	end },
	{ "park", function(_, Note)
		local Thread = coroutine.running()
		task.delay(0.3, function()
			if coroutine.status(Thread) == "suspended" then
				task.spawn(Thread, "timer")
			end
		end)
		local Start = Mark()
		local Got = Show(coroutine.yield())
		Note(`yield {Span(Start, 0.3)} {Got}`)
	end },
}
for _, Refusal in Refusals do
	for _, Follow in Follows do
		local Things = { Event = Instance.new("BindableEvent"), Folder = Instance.new("Folder"), Function = Instance.new("BindableFunction") }
		Things.Function.OnInvoke = function(Value)
			task.wait(0.1)
			return "invoked " .. tostring(Value)
		end
		Case(`after-{Refusal[1]}-{Follow[1]}`, function(Note)
			Note(Refused("sort", function()
				return Refusal[2](Things)
			end))
			Follow[2](Things, Note)
		end)
		-- (what the refused wait waited for has happened before the next case)
		task.wait(0.05)
	end
end

-- 4. threads started where the thread cannot yield
local Started = {}
Case("threads", function(Note)
	Note(Refused("sort", function()
		task.spawn(function()
			table.insert(Started, "spawn")
			task.wait(0.05)
			table.insert(Started, "spawn-after-wait")
		end)
		task.defer(function()
			table.insert(Started, "defer")
		end)
		task.delay(0.05, function()
			table.insert(Started, "delay")
		end)
		delay(0.05, function()
			table.insert(Started, "legacy-delay")
		end)
		local Wrapped = coroutine.wrap(function(First)
			local Second = coroutine.yield(First + 1)
			return Second * 2
		end)
		local A = Wrapped(1)
		local B = Wrapped(10)
		table.insert(Started, `wrap {A} {B}`)
		return "made"
	end))
	task.wait(0.2)
	table.sort(Started)
	Note(table.concat(Started, ","))
end)

-- 5. a spawned thread that dies on the refusal: the error ScriptContext.Error gets, with its trace
local Errors = {}
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local function Trace(Text)
	local Frames = {}
	for _, Line in string.split(Text, "\n") do
		local Number, Rest = string.match(Line, "^" .. Escaped .. ", line (%d+)(.*)$")
		if Number then
			table.insert(Frames, "@" .. (tonumber(Number) - P.base) .. Rest)
		end
	end
	return table.concat(Frames, " / ")
end
local ErrorConnection = ScriptContext.Error:Connect(function(Message, Text)
	if string.find(Message, "yield") then
		table.insert(Errors, P.err(Message) .. " / trace " .. Trace(Text))
	end
end)
task.spawn(function()
	table.sort({ 2, 1 }, function(A, B)
		task.wait(0.1)
		return A < B
	end)
end)
task.spawn(function()
	string.gsub("x", "x", function()
		return Event.Event:Wait()
	end)
end)
for _ = 1, 3 do
	task.wait()
end
print("spawned-errors " .. table.concat(Errors, " || "))
ErrorConnection:Disconnect()
FrameConnection:Disconnect()
Folder:Destroy()
