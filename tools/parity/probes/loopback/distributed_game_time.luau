-- parity: loopback
-- Workspace.DistributedGameTime is each side's own game time: in a remote's handler a client's is the time() of its own last
-- Heartbeat (its last simulation step), not the value its server read (a real client's runs behind its server's, as its
-- time() does: known-loopback.json's server_time lines).
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
--- server
local Player = P.player()
local Clock = Instance.new("RemoteEvent")
Clock.Name = "GameTime"
Clock.Parent = Storage
P.meet("ready")
for _ = 1, 3 do
	RunService.Heartbeat:Wait()
	print("sent its own time()", workspace.DistributedGameTime == time())
	Clock:FireClient(Player, time(), workspace.DistributedGameTime)
end
P.meet("sent")
--- client
local Clock = Storage:WaitForChild("GameTime")
local Lines = {}
local Beat = nil
RunService.Heartbeat:Connect(function()
	Beat = time()
end)
Clock.OnClientEvent:Connect(function(Time, Distributed)
	table.insert(Lines, "the client's is its own last step's " .. tostring(workspace.DistributedGameTime == Beat) .. ", the server's sent its time() " .. tostring(Distributed == Time))
end)
P.meet("ready")
P.meet("sent")
for _ = 1, 10 do
	task.wait()
end
for _, Line in Lines do
	print(Line)
end
