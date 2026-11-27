-- parity: loopback
-- When a RemoteFunction's answer arrives against the events sent with it, each way: an event then an invoke from one thread,
-- an invoke (spawned) then an event, and a marker event the callback's side fires before its callback returns and after
-- (task.defer); 30 tries a row from a Heartbeat:Wait() thread, each side counting its own frames at PreSimulation. Printed:
-- the sets of frame gaps between the answer and the event (a round trip's own count follows the two processes' timing).
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local COUNT = 30
local Count = 0
RunService.PreSimulation:Connect(function()
	Count += 1
end)
local function Frame()
	return Count
end
local function Set(Map)
	local Seen, List = {}, {}
	for _, Gap in Map do
		local Text = tostring(Gap)
		if not Seen[Text] then
			Seen[Text] = 0
			table.insert(List, Text)
		end
		Seen[Text] += 1
	end
	table.sort(List)
	for Index, Text in List do
		List[Index] = Text .. "x" .. Seen[Text]
	end
	return table.concat(List, " ")
end
-- The host's callback: records its frame; mode "defer" fires a marker after it returns (task.defer), "inside" fires one
-- before it returns
local function HostCallback(Fire, Record)
	return function(...)
		local Pair, Mode = select(select("#", ...) - 1, ...)
		Record(Pair, Frame())
		if Mode == "defer" then
			task.defer(Fire, "marker", Pair)
		elseif Mode == "inside" then
			Fire("marker", Pair)
		end
		return Pair
	end
end
-- The caller: modes in turn, then what arrived when
local function Caller(Invoke, Fire, OnEvent)
	local Results = {}
	local function Row(Name, Gaps)
		table.insert(Results, Name .. ": " .. Set(Gaps))
	end
	local Marker, Echo = {}, {}
	OnEvent(function(Kind, Pair)
		if Kind == "marker" then
			Marker[Pair] = Frame()
		elseif Kind == "echo" or Kind == "echoed" then
			Echo[Pair] = Frame()
		end
	end)
	-- 1. an event then an invoke, together: the return minus the echo
	local Return, Rounds = {}, {}
	for Pair = 1, COUNT do
		RunService.Heartbeat:Wait()
		local Start = Frame()
		Fire("echo", Pair)
		Invoke(Pair, "plain")
		Return[Pair] = Frame()
		Rounds[Pair] = Frame() - Start
	end
	task.wait(0.3)
	local Gaps = {}
	for Pair, At in Return do
		if Echo[Pair] then
			Gaps[Pair] = At - Echo[Pair]
		else
			Gaps[Pair] = "none"
		end
	end
	Row("event+invoke return-echo", Gaps)
	-- 2. an invoke (spawned) then an event: the return minus the echo
	table.clear(Echo)
	local Returned = {}
	for Pair = 101, 100 + COUNT do
		RunService.Heartbeat:Wait()
		task.spawn(function()
			Invoke(Pair, "plain")
			Returned[Pair] = Frame()
		end)
		Fire("echo", Pair)
	end
	task.wait(0.3)
	Gaps = {}
	for Pair, At in Returned do
		Gaps[Pair] = if Echo[Pair] then At - Echo[Pair] else "none"
	end
	Row("invoke+event return-echo", Gaps)
	-- 3. the host fires a marker after its callback returns, and 4. before
	for _, Mode in { "defer", "inside" } do
		table.clear(Marker)
		local Back = {}
		for Pair = 201, 200 + COUNT do
			RunService.Heartbeat:Wait()
			Invoke(Pair, Mode)
			Back[Pair] = Frame()
		end
		task.wait(0.3)
		Gaps = {}
		for Pair, At in Back do
			Gaps[Pair] = if Marker[Pair] then At - Marker[Pair] else "none"
		end
		Row(Mode .. "-marker return-marker", Gaps)
	end
	-- 5. an event alone, echoed: the round trip
	table.clear(Echo)
	local Sent = {}
	for Pair = 301, 300 + COUNT do
		RunService.Heartbeat:Wait()
		Sent[Pair] = Frame()
		Fire("echo", Pair)
	end
	task.wait(0.3)
	Gaps = {}
	for Pair, At in Sent do
		Gaps[Pair] = if Echo[Pair] then Echo[Pair] - At else "none"
	end
	return table.concat(Results, " ; ")
end
--- server
local Player = P.player()
local Event = Instance.new("RemoteEvent")
Event.Name = "E"
Event.Parent = Storage
local Call = Instance.new("RemoteFunction")
Call.Name = "C"
Call.Parent = Storage
local Back = Instance.new("RemoteFunction")
Back.Name = "B"
Back.Parent = Storage
local CallbackAt, EventAt = {}, {}
Call.OnServerInvoke = HostCallback(function(...)
	Event:FireClient(Player, ...)
end, function(Pair, At)
	CallbackAt[Pair] = At
end)
local OnClientSent
Event.OnServerEvent:Connect(function(_, Kind, Pair)
	if Kind == "echo" then
		EventAt[Pair] = Frame()
		Event:FireClient(Player, "echo", Pair)
	elseif OnClientSent then
		OnClientSent(Kind, Pair)
	end
end)
P.meet("ready")
P.meet("client done")
local Gaps = {}
for Pair, At in CallbackAt do
	if Pair < 200 and EventAt[Pair] then
		Gaps[Pair] = EventAt[Pair] - At
	end
end
print("host: event handler minus callback (rows 1-2): " .. Set(Gaps))
-- the server calls the client
print("server caller " .. Caller(function(Pair, Mode)
	return Back:InvokeClient(Player, Pair, Mode)
end, function(...)
	Event:FireClient(Player, "s", ...)
end, function(Handler)
	OnClientSent = Handler
end))
P.meet("server done")
--- client
local Event = Storage:WaitForChild("E")
local Call = Storage:WaitForChild("C")
local Back = Storage:WaitForChild("B")
local Handlers = {}
local CallbackAt, EventAt = {}, {}
Event.OnClientEvent:Connect(function(Kind, ...)
	if Kind == "s" then
		local Inner, Pair = ...
		if Inner == "echo" then
			EventAt[Pair] = Frame()
			Event:FireServer("echoed", Pair)
		end
		return
	end
	for _, Handler in Handlers do
		Handler(Kind, ...)
	end
end)
Back.OnClientInvoke = HostCallback(function(...)
	Event:FireServer(...)
end, function(Pair, At)
	CallbackAt[Pair] = At
end)
P.meet("ready")
print("client caller " .. Caller(function(Pair, Mode)
	return Call:InvokeServer(Pair, Mode)
end, function(...)
	Event:FireServer(...)
end, function(Handler)
	table.insert(Handlers, Handler)
end))
P.meet("client done")
P.meet("server done")
