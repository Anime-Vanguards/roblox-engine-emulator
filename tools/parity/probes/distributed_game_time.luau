-- Workspace.DistributedGameTime in a running game, a playing client's and a server's alike: the game time as of the frame's
-- simulation step. time() moves at PreAnimation, DistributedGameTime after PreSimulation (PreAnimation, Stepped and
-- PreSimulation read the last frame's game time; PostSimulation, the task.wait resumptions and Heartbeat the frame's, equal to
-- time()). A script may write it, with a number property's coercion; what it wrote holds until the next simulation step. A server
-- fires Changed for every step, a client only for a script's write. Read through the global (an import) and through a local.
local RunService = game:GetService("RunService")
local W = workspace
print("start", typeof(workspace.DistributedGameTime), workspace.DistributedGameTime == time(), W.DistributedGameTime == time())
task.wait(0.1)
print("after a wait", workspace.DistributedGameTime == time(), W.DistributedGameTime == time(), workspace.DistributedGameTime > 0)

-- Each phase over three frames: whether it read the last frame's game time (the time() of the last Heartbeat) or the frame's
local Phases = { "PreAnimation", "Stepped", "PreSimulation", "PostSimulation", "task.wait", "Heartbeat" }
local Seen = {}
for _, Name in Phases do
	Seen[Name] = { Count = 0, Last = 0, Frame = 0 }
end
local Going, Done, Beats = false, false, 0
RunService.Heartbeat:Wait()
local Last = time()
local function Note(Name)
	if Going and not Done then
		local Entry = Seen[Name]
		Entry.Count += 1
		if workspace.DistributedGameTime == Last then
			Entry.Last += 1
		end
		if workspace.DistributedGameTime == time() and time() > Last then
			Entry.Frame += 1
		end
	end
end
local Connections = {}
for _, Name in { "PreAnimation", "Stepped", "PreSimulation", "PostSimulation" } do
	table.insert(Connections, RunService[Name]:Connect(function()
		if Name == "PreAnimation" then
			Going = true
		end
		Note(Name)
	end))
end
table.insert(Connections, RunService.Heartbeat:Connect(function()
	Note("Heartbeat")
	if Going then
		Beats += 1
		Done = Beats >= 3
	end
	Last = time()
end))
task.spawn(function()
	while not Done do
		task.wait()
		Note("task.wait")
	end
end)
while not Done do
	RunService.Heartbeat:Wait()
end
for _, Connection in Connections do
	Connection:Disconnect()
end
for _, Name in Phases do
	local Entry = Seen[Name]
	print(Name, "reads", Entry.Count, "the last frame's", Entry.Last, "the frame's", Entry.Frame)
end

-- A write holds until the next simulation step
RunService.Heartbeat:Wait()
W.DistributedGameTime = 1000
print("written", workspace.DistributedGameTime, W.DistributedGameTime)
local Held = {}
local Stepped = RunService.Stepped:Connect(function()
	table.insert(Held, "Stepped " .. tostring(W.DistributedGameTime))
end)
local Post = RunService.PostSimulation:Connect(function()
	table.insert(Held, "PostSimulation equals time() " .. tostring(W.DistributedGameTime == time()))
end)
RunService.Heartbeat:Wait()
Stepped:Disconnect()
Post:Disconnect()
for _, Line in Held do
	print("after the write:", Line)
end
for _, Value in { "x", "12", -5, true } do
	W.DistributedGameTime = Value
	print("written", typeof(Value), tostring(Value), "reads", workspace.DistributedGameTime)
end

-- Changed: every step on a server, none on a client; a script's write on both
local Fired, Named = 0, 0
local Property = W:GetPropertyChangedSignal("DistributedGameTime"):Connect(function()
	Fired += 1
end)
local Changed = W.Changed:Connect(function(Name)
	if Name == "DistributedGameTime" then
		Named += 1
	end
end)
for _ = 1, 3 do
	RunService.Heartbeat:Wait()
end
print("three frames: property signal", Fired, "Changed", Named)
Fired, Named = 0, 0
W.DistributedGameTime = 7
RunService.Heartbeat:Wait()
print("a write and a frame: property signal", Fired, "Changed", Named)
Property:Disconnect()
Changed:Disconnect()
