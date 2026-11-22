-- parity: loopback
-- How a table that appears many times crosses a remote, each way: once for each place it appears, with no limit of its own
-- (one table of 1,000 numbers held 1,000 times, 9 MB, arrives whole), an UnreliableRemoteEvent's warning counting every
-- byte of such a payload, and a cycle after it found at the sender.
local Storage = game:GetService("ReplicatedStorage")
local LogService = game:GetService("LogService")
local Logged = {}
LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageWarning then
		table.insert(Logged, (string.gsub(Message, "\n.*", "")))
	end
end)
local function Fan(Count, Size)
	return table.create(Count, table.create(Size, 1))
end
local function Ring()
	local Value = {}
	Value[1] = Value
	return Value
end
local function Shape(Value)
	local Numbers = 0
	for _, Item in Value do
		Numbers += #Item
	end
	return `{#Value} {#Value[#Value]} {Numbers} {Value[1] ~= Value[2]}`
end
local Got = {}
local function WaitFor(Count)
	for _ = 1, 900 do
		if #Got >= Count then
			break
		end
		task.wait()
	end
	local Text = table.concat(Got, ", ")
	table.clear(Got)
	return Text
end
local function Send(Who, Fire, Unreliable)
	print(`{Who} reliable fan 1000x1000: ` .. P.try(Fire, Fan(1000, 1000)))
	print(`{Who} reliable fan then ring: ` .. P.try(Fire, { Fan(1000, 1000), Ring() }))
	print(`{Who} unreliable fan 10x1000: ` .. P.try(Unreliable, Fan(10, 1000)))
	print(`{Who} unreliable fan 1000x1000: ` .. P.try(Unreliable, Fan(1000, 1000)))
	print(`{Who} unreliable fan then ring: ` .. P.try(Unreliable, { Fan(1000, 1000), Ring() }))
	for _ = 1, 10 do
		task.wait()
	end
	print(`{Who} warnings: ` .. table.concat(Logged, " / "))
	table.clear(Logged)
end
--- server
local Player = P.player()
local Event = Instance.new("RemoteEvent")
Event.Name = "Breadth"
Event.Parent = Storage
local Unreliable = Instance.new("UnreliableRemoteEvent")
Unreliable.Name = "BreadthUnreliable"
Unreliable.Parent = Storage
local Call = Instance.new("RemoteFunction")
Call.Name = "BreadthCall"
Call.OnServerInvoke = function(_, Value)
	if Value == "fan" then
		return Fan(1000, 1000)
	end
	return Shape(Value)
end
Call.Parent = Storage
Event.OnServerEvent:Connect(function(_, Value)
	table.insert(Got, Shape(Value))
end)
P.meet("ready")
Send("server", function(Value) Event:FireClient(Player, Value) end, function(Value) Unreliable:FireClient(Player, Value) end)
P.meet("server sent")
P.meet("client sent")
print("server got: " .. WaitFor(1))
P.meet("done")
--- client
local Event = Storage:WaitForChild("Breadth")
local Unreliable = Storage:WaitForChild("BreadthUnreliable")
local Call = Storage:WaitForChild("BreadthCall")
Event.OnClientEvent:Connect(function(Value)
	table.insert(Got, Shape(Value))
end)
P.meet("ready")
P.meet("server sent")
print("client got: " .. WaitFor(1))
Send("client", function(Value) Event:FireServer(Value) end, function(Value) Unreliable:FireServer(Value) end)
print("client invoke fan 1000x1000 argument: " .. P.try(function() return Call:InvokeServer(Fan(1000, 1000)) end))
print("client invoke fan 1000x1000 result: " .. P.try(function() return Shape(Call:InvokeServer("fan")) end))
P.meet("client sent")
P.meet("done")
