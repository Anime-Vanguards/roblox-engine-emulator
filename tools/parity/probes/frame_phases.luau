-- The order of a client frame's events and where queued work (deferred threads, waits, delays) runs between them.
local RunService = game:GetService("RunService")
P.sync()

local Log = {}
local function Note(Text)
	table.insert(Log, Text)
end
local Connections = {}
local Events = { "PreRender", "RenderStepped", "PreAnimation", "Stepped", "PreSimulation", "PostSimulation", "Heartbeat" }
local Recording = false
for _, Name in Events do
	table.insert(Connections, RunService[Name]:Connect(function()
		if not Recording then
			return
		end
		Note(Name)
		-- where a deferred thread queued by this handler runs
		task.defer(function()
			Note("d<" .. Name)
		end)
	end))
end
RunService:BindToRenderStep("parity-first", Enum.RenderPriority.First.Value, function()
	if Recording then
		Note("bind-first")
	end
end)
RunService:BindToRenderStep("parity-camera", Enum.RenderPriority.Camera.Value, function()
	if Recording then
		Note("bind-camera")
	end
end)
RunService:BindToRenderStep("parity-last", Enum.RenderPriority.Last.Value, function()
	if Recording then
		Note("bind-last")
	end
end)
RunService:BindToRenderStep("parity-same-a", 500, function()
	if Recording then
		Note("bind-500a")
	end
end)
RunService:BindToRenderStep("parity-same-b", 500, function()
	if Recording then
		Note("bind-500b")
	end
end)

-- one waiting thread and one delay per frame, to place their resumptions among the events
local Stop = false
task.spawn(function()
	while not Stop do
		task.wait()
		if Recording then
			Note("wait")
			task.defer(function()
				Note("d<wait")
			end)
		end
	end
end)
local function Delay()
	if Stop then
		return
	end
	if Recording then
		Note("delay")
	end
	task.delay(0, Delay)
end
task.delay(0, Delay)
task.wait()
task.wait()
Recording = true
task.wait()
task.wait()
task.wait()
Recording = false
Stop = true
for _, Connection in Connections do
	Connection:Disconnect()
end
for _, Name in { "parity-first", "parity-camera", "parity-last", "parity-same-a", "parity-same-b" } do
	RunService:UnbindFromRenderStep(Name)
end
-- A headless client renders on its own cadence (no render step, or two, between simulation steps), so the render step's
-- own order and the simulation step's order are shown apart: the render step from a "bind-first" to the entries before
-- the next simulation event, the simulation step from a PreAnimation to the next PreAnimation, render entries left out.
local RenderNames = { ["bind-first"] = true, ["bind-camera"] = true, ["bind-last"] = true, ["bind-500a"] = true, ["bind-500b"] = true, RenderStepped = true, PreRender = true, ["d<RenderStepped"] = true, ["d<PreRender"] = true }
local RenderStep = {}
local First = table.find(Log, "bind-first")
if First then
	for Index = First, #Log do
		local Entry = Log[Index]
		if Index > First and not RenderNames[Entry] then
			break
		end
		table.insert(RenderStep, Entry)
	end
end
print("render-step " .. table.concat(RenderStep, ","))
local Simulation = {}
local Start = table.find(Log, "PreAnimation")
if Start then
	for Index = Start, #Log do
		local Entry = Log[Index]
		if Index > Start and Entry == "PreAnimation" then
			break
		end
		if not RenderNames[Entry] then
			table.insert(Simulation, Entry)
		end
	end
end
print("simulation-step " .. table.concat(Simulation, ","))

-- the minimum a wait and a delay take, in frames
P.sync()
local Count = 0
local Counter = RunService.Heartbeat:Connect(function()
	Count += 1
end)
local Results = {}
for _, Seconds in { 0, 0.001, 0.01, 0.02, 0.025, 0.04, 0.06, 0.09, 0.11 } do
	local From = Count
	task.wait(Seconds)
	table.insert(Results, string.format("task.wait(%s)=%d", tostring(Seconds), Count - From))
end
for _, Seconds in { 0, 0.02, 0.04 } do
	local From = Count
	wait(Seconds)
	table.insert(Results, string.format("wait(%s)=%d", tostring(Seconds), Count - From))
end
for _, Seconds in { 0, 0.02, 0.04 } do
	local From = Count
	local Done = false
	task.delay(Seconds, function()
		table.insert(Results, string.format("task.delay(%s)=%d", tostring(Seconds), Count - From))
		Done = true
	end)
	repeat
		task.wait()
	until Done
end
for _, Seconds in { 0, 0.02 } do
	local From = Count
	local Done = false
	delay(Seconds, function()
		table.insert(Results, string.format("delay(%s)=%d", tostring(Seconds), Count - From))
		Done = true
	end)
	repeat
		task.wait()
	until Done
end
do
	local From = Count
	local Done = false
	spawn(function()
		table.insert(Results, string.format("spawn=%d", Count - From))
		Done = true
	end)
	repeat
		task.wait()
	until Done
end
Counter:Disconnect()
print("minimums " .. table.concat(Results, ","))

-- Heartbeat's and Stepped's arguments
P.sync()
local Args = {}
local One = RunService.Heartbeat:Once(function(...)
	table.insert(Args, "Heartbeat " .. select("#", ...) .. " " .. type((...)))
end)
local Two = RunService.Stepped:Once(function(...)
	local A, B = ...
	table.insert(Args, "Stepped " .. select("#", ...) .. " " .. type(A) .. " " .. type(B) .. " " .. tostring(B > 0 and B < 0.1))
end)
local Three = RunService.RenderStepped:Once(function(...)
	table.insert(Args, "RenderStepped " .. select("#", ...) .. " " .. type((...)))
end)
local Four = RunService.PreSimulation:Once(function(...)
	table.insert(Args, "PreSimulation " .. select("#", ...))
end)
local Five = RunService.PreAnimation:Once(function(...)
	table.insert(Args, "PreAnimation " .. select("#", ...))
end)
local Six = RunService.PostSimulation:Once(function(...)
	table.insert(Args, "PostSimulation " .. select("#", ...))
end)
local Seven = RunService.PreRender:Once(function(...)
	table.insert(Args, "PreRender " .. select("#", ...))
end)
task.wait()
task.wait()
table.sort(Args)
print("args " .. table.concat(Args, ","))
print("once-connected " .. tostring(One.Connected) .. tostring(Two.Connected) .. tostring(Three.Connected))
print("render-step errors " .. P.try(RunService.BindToRenderStep, RunService, "x", "high", function() end))
print("unbind-missing " .. P.try(RunService.UnbindFromRenderStep, RunService, "never-bound"))
