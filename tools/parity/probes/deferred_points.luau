-- Each invocation point's flush: does work deferred during it run there (a 3-step defer chain started from each
-- RunService event, from a delay, from a waiting thread and from legacy wait/spawn)? How long can a task.defer chain run,
-- and what does the engine say when an event chain is cut?
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
P.sync()
local Log = {}
local Marks = {}
for _, Name in { "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	table.insert(Marks, RunService[Name]:Connect(function()
		table.insert(Log, "|" .. string.sub(Name, 1, 4))
	end))
end
local function Chain(Tag)
	table.insert(Log, Tag .. "0")
	local Step = 0
	local function Next()
		Step += 1
		table.insert(Log, Tag .. Step)
		if Step < 6 then
			task.defer(Next)
		end
	end
	task.defer(Next)
end
local function Show(Label)
	for _ = 1, 3 do
		task.wait()
	end
	-- the window from the chain's start to two Heartbeats on
	local Out, Beats, Started = {}, 0, false
	for _, Entry in Log do
		if not Started and string.sub(Entry, 1, 1) ~= "|" then
			Started = true
		end
		if Started then
			if Entry == "|Hear" then
				Beats += 1
				if Beats > 2 then
					break
				end
			end
			table.insert(Out, Entry)
		end
	end
	print(Label .. " " .. table.concat(Out, " "))
	table.clear(Log)
end
for _, Name in { "PreRender", "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat", "Stepped", "RenderStepped" } do
	P.sync()
	table.clear(Log)
	RunService[Name]:Once(function()
		Chain("c")
	end)
	Show("from-" .. Name)
end
P.sync()
table.clear(Log)
task.delay(0, function()
	Chain("c")
end)
Show("from-delay")
P.sync()
table.clear(Log)
task.spawn(function()
	task.wait()
	Chain("c")
end)
Show("from-wait")
P.sync()
table.clear(Log)
task.spawn(function()
	wait()
	Chain("c")
end)
Show("from-legacy-wait")
P.sync()
table.clear(Log)
spawn(function()
	Chain("c")
end)
Show("from-legacy-spawn")
P.sync()
table.clear(Log)
RunService:BindToRenderStep("parity-chain", 100, function()
	RunService:UnbindFromRenderStep("parity-chain")
	Chain("c")
end)
Show("from-bindtorenderstep")
for _, Connection in Marks do
	Connection:Disconnect()
end

-- a long task.defer chain: is it cut?
P.sync()
local Steps = 0
local Frames = 0
local Counter = RunService.Heartbeat:Connect(function()
	Frames += 1
end)
local AtFrame = {}
local function Next()
	Steps += 1
	AtFrame[Frames] = (AtFrame[Frames] or 0) + 1
	if Steps < 1000 then
		task.defer(Next)
	end
end
task.defer(Next)
for _ = 1, 4 do
	task.wait()
end
Counter:Disconnect()
local PerFrame = {}
for Frame = 0, Frames do
	if AtFrame[Frame] then
		table.insert(PerFrame, Frame .. ":" .. AtFrame[Frame])
	end
end
print("defer-chain-1000 steps " .. Steps .. " per-frame " .. table.concat(PerFrame, ","))

-- the message when an event chain is cut
P.sync()
local Messages = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if string.find(Message, "e%-entrancy") or string.find(Message, "depth") then
		table.insert(Messages, tostring(Kind) .. " " .. Message)
	end
end)
local Event = Instance.new("BindableEvent")
Event.Name = "ParityLoop"
local Count = 0
Event.Event:Connect(function()
	Count += 1
	if Count < 500 then
		Event:Fire()
	end
end)
Event:Fire()
for _ = 1, 3 do
	task.wait()
end
Listener:Disconnect()
local Escaped = string.gsub(P.chunk(), "%p", "%%%0")
for Index, Message in Messages do
	Messages[Index] = string.gsub(Message, "on line (%d+) in " .. Escaped, function(Line)
		return "on line @" .. (tonumber(Line) - P.base) .. " in <probe>"
	end)
end
print("cut-event count " .. Count .. " messages " .. #Messages .. " " .. table.concat(Messages, " | "))
-- after a cut, does the event work again?
local After = 0
local Again = Event.Event:Connect(function()
	After += 1
end)
Count = 1000
Event:Fire()
task.wait()
task.wait()
Again:Disconnect()
print("after-cut fires " .. After)

-- the cut of a task.defer chain: does the 81st task.defer raise to its caller, and what is logged?
P.sync()
local Logged = {}
local Watch = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageError then
		local First = (string.split(Message, "\n"))[1]
		First = string.gsub(First, "on line (%d+) in " .. Escaped, function(Line)
			return "on line @" .. (tonumber(Line) - P.base) .. " in <probe>"
		end)
		table.insert(Logged, First)
	end
end)
local Depth = 0
local CutResult
local function DeferStep()
	Depth += 1
	if Depth < 200 then
		local Ok, Message = pcall(task.defer, DeferStep)
		if not Ok then
			CutResult = "raised at step " .. Depth .. ": " .. P.err(Message)
		end
	end
end
task.defer(DeferStep)
for _ = 1, 3 do
	task.wait()
end
print("defer-cut steps " .. Depth .. " " .. tostring(CutResult))
-- an event chain through two signals and a defer in between: one counter for all
local Mixed = 0
local First, Second = Instance.new("BindableEvent"), Instance.new("BindableEvent")
First.Event:Connect(function()
	Mixed += 1
	if Mixed < 300 then
		task.defer(function()
			Mixed += 1
			if Mixed < 300 then
				Second:Fire()
			end
		end)
	end
end)
Second.Event:Connect(function()
	Mixed += 1
	if Mixed < 300 then
		First:Fire()
	end
end)
First:Fire()
for _ = 1, 3 do
	task.wait()
end
print("mixed-chain steps " .. Mixed)
-- a named handler, and a property signal
local Value = Instance.new("IntValue")
local function OnValue()
	if Value.Value < 300 then
		Value.Value += 1
	end
end
Value:GetPropertyChangedSignal("Value"):Connect(OnValue)
Value.Value = 1
for _ = 1, 3 do
	task.wait()
end
print("named-handler-chain value " .. Value.Value)
Watch:Disconnect()
print("logged " .. #Logged .. " " .. table.concat(Logged, " | "))
