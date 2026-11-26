-- parity: loopback
-- Where what the other side sent runs in the receiving side's frame and the time() it reads there: a RemoteEvent's handlers,
-- a RemoteFunction's callback and the return of the call on both sides, and a replicated property's Changed on the client.
-- All of them run at the frame's start, before PreAnimation (with the legacy wait() resumptions), and read the side's last
-- frame's time() (the time() of its Heartbeat before them), equal to its DistributedGameTime; time() moves at PreAnimation.
-- Each kind's place is the simulation event before it and the one after it (the render step is left out).
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local COUNT = 6
local Seen, Order = {}, {}
local LastStep, LastBeat = "none", nil
local Placing, Timing = {}, {}
local Running = true
local function Note(Kind)
	local Entry = Seen[Kind]
	if not Entry then
		Entry = { Count = 0, Before = 0, After = 0, Game = 0, Places = {} }
		Seen[Kind] = Entry
		table.insert(Order, Kind)
	end
	local Now = time()
	Entry.Count += 1
	if Now == LastBeat then
		Entry.Before += 1
	end
	if workspace.DistributedGameTime == Now then
		Entry.Game += 1
	end
	table.insert(Placing, { Entry, LastStep })
	table.insert(Timing, { Entry, Now })
end
local function Step(Name)
	for _, Waiting in Placing do
		Waiting[1].Places[Waiting[2] .. " > " .. Name] = true
	end
	table.clear(Placing)
	LastStep = Name
end
local function Track()
	for _, Name in { "PreAnimation", "Stepped", "PreSimulation", "PostSimulation" } do
		RunService[Name]:Connect(function()
			if Running then
				Step(Name)
			end
		end)
	end
	RunService.Heartbeat:Connect(function()
		if not Running then
			return
		end
		Step("Heartbeat")
		local Now = time()
		for _, Waiting in Timing do
			if Waiting[2] == Now then
				Waiting[1].After += 1
			end
		end
		table.clear(Timing)
		LastBeat = Now
	end)
end
local function Report()
	-- (two more frames place and time the last arrivals)
	RunService.Heartbeat:Wait()
	RunService.Heartbeat:Wait()
	Running = false
	table.sort(Order)
	for _, Kind in Order do
		local Entry = Seen[Kind]
		local Places = {}
		for Place in Entry.Places do
			table.insert(Places, Place)
		end
		table.sort(Places)
		print(string.format("%s ran %d times, between %s; time() of the Heartbeat before %s, of the Heartbeat after %s; DistributedGameTime equal %s", Kind,
			Entry.Count, table.concat(Places, " | "), tostring(Entry.Before == Entry.Count), tostring(Entry.After == Entry.Count), tostring(Entry.Game == Entry.Count)))
	end
end
--- server
local Player = P.player()
local Event = Instance.new("RemoteEvent")
Event.Name = "Event"
Event.Parent = Storage
local Call = Instance.new("RemoteFunction")
Call.Name = "Call"
Call.Parent = Storage
local Value = Instance.new("IntValue")
Value.Name = "Value"
Value.Parent = Storage
Track()
Event.OnServerEvent:Connect(function()
	Note("OnServerEvent")
end)
Call.OnServerInvoke = function()
	Note("OnServerInvoke")
	return true
end
P.meet("ready")
local Index = 0
local Connection
Connection = RunService.Heartbeat:Connect(function()
	Index += 1
	if Index > COUNT then
		Connection:Disconnect()
		return
	end
	Event:FireClient(Player, Index)
	Value.Value = Index
end)
for _ = 1, COUNT do
	task.wait()
	Call:InvokeClient(Player)
	Note("InvokeClient returned")
end
P.meet("sent")
P.meet("invoked")
for _ = 1, 10 do
	task.wait()
end
Report()
--- client
local Event = Storage:WaitForChild("Event")
local Call = Storage:WaitForChild("Call")
local Value = Storage:WaitForChild("Value")
Track()
Event.OnClientEvent:Connect(function()
	Note("OnClientEvent")
end)
Call.OnClientInvoke = function()
	Note("OnClientInvoke")
	return true
end
Value.Changed:Connect(function()
	Note("Changed")
end)
P.meet("ready")
local Index = 0
local Connection
Connection = RunService.Heartbeat:Connect(function()
	Index += 1
	if Index > COUNT then
		Connection:Disconnect()
		return
	end
	Event:FireServer(Index)
end)
P.meet("sent")
for _ = 1, 10 do
	task.wait()
end
for _ = 1, COUNT do
	task.wait()
	Call:InvokeServer()
	Note("InvokeServer returned")
end
P.meet("invoked")
for _ = 1, 10 do
	task.wait()
end
Report()
