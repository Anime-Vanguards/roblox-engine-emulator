-- parity: loopback
-- InstanceHandle through remotes, each way: a handle of an instance both sides have, of one only the sender has, of a
-- destroyed one and of none, alone, in an array, as a dictionary's value and as its key, through a RemoteEvent, an
-- UnreliableRemoteEvent and a RemoteFunction's arguments and results, and as a replicated attribute (set before the client
-- joins and after). The receiver prints what each case arrived as.
local Storage = game:GetService("ReplicatedStorage")
local function Dump(Value)
	local Kind = typeof(Value)
	if Kind == "InstanceHandle" then
		local Ok, Got = pcall(function()
			return Value:Get()
		end)
		if not Ok then
			return "InstanceHandle get-error " .. P.err(Got)
		end
		return "InstanceHandle(" .. (if Got == nil then "nil" else Got.Name .. " in " .. tostring(Got.Parent)) .. ")"
	elseif Kind == "Instance" then
		return "Instance " .. Value.Name .. " in " .. tostring(Value.Parent)
	elseif Kind == "table" then
		local Entries = {}
		for Key, Item in Value do
			table.insert(Entries, (if typeof(Key) == "string" or typeof(Key) == "number" then tostring(Key) else typeof(Key)) .. "=" .. Dump(Item))
		end
		table.sort(Entries)
		return "{" .. table.concat(Entries, ", ") .. "}"
	elseif Kind == "nil" or Kind == "boolean" or Kind == "number" or Kind == "string" then
		return Kind .. " " .. tostring(Value)
	end
	return Kind
end
local function Cases(Own)
	return {
		{ "shared", function() return InstanceHandle.new(Own.Shared) end },
		{ "own", function() return InstanceHandle.new(Own.Mine) end },
		{ "loose", function() return InstanceHandle.new(Own.Loose) end },
		{ "destroyed", function() return InstanceHandle.new(Own.Gone) end },
		{ "empty", function() return InstanceHandle.new(nil) end },
		{ "empty-then-1", function() return InstanceHandle.new(nil), 1 end },
		{ "array", function() return { InstanceHandle.new(Own.Shared), InstanceHandle.new(nil), 3 } end },
		{ "value", function() return { Key = InstanceHandle.new(Own.Shared), Empty = InstanceHandle.new(nil) } end },
		{ "key", function() return { [InstanceHandle.new(Own.Shared)] = 1 } end },
		{ "player", function() return InstanceHandle.new(Own.Player) end },
	}
end
local function Receive(Index, ...)
	local Parts = { "n=" .. select("#", ...) }
	for Position = 1, select("#", ...) do
		table.insert(Parts, Dump((select(Position, ...))))
	end
	return "case " .. Index .. " " .. Cases({})[Index][1] .. " " .. table.concat(Parts, " | ")
end
local function Send(Label, Fire, Own)
	for Index, Case in Cases(Own) do
		local Ok, Problem = pcall(Fire, Index, Case[2]())
		if not Ok then
			print("send " .. Label .. " " .. Index .. " " .. Case[1] .. " error " .. P.err(Problem))
		end
	end
end
--- server
local Player = P.player()
local Event = Instance.new("RemoteEvent")
Event.Name = "Handles"
Event.Parent = Storage
local Unreliable = Instance.new("UnreliableRemoteEvent")
Unreliable.Name = "HandlesUnreliable"
Unreliable.Parent = Storage
local Function = Instance.new("RemoteFunction")
Function.Name = "HandlesFunction"
Function.Parent = Storage
local Shared = Instance.new("Part")
Shared.Name = "Shared"
Shared.Anchored = true
Shared.Position = Vector3.new(200, 50, 200)
Shared.Parent = workspace
local Mine = Instance.new("Folder")
Mine.Name = "ServerOnly"
Mine.Parent = game:GetService("ServerStorage")
local Gone = Instance.new("Folder")
Gone.Name = "Gone"
Gone.Parent = Storage
-- attributes the client gets with the join, and ones set once it is in
local Joined = Instance.new("Folder")
Joined.Name = "JoinedHolder"
local Later = Instance.new("Folder")
Later.Name = "LaterHolder"
Later.Parent = Storage
local function Attributes(Holder)
	Holder:SetAttribute("Shared", InstanceHandle.new(Shared))
	Holder:SetAttribute("Own", InstanceHandle.new(Mine))
	Holder:SetAttribute("Loose", InstanceHandle.new(Instance.new("Folder")))
	Holder:SetAttribute("Empty", InstanceHandle.new(nil))
	Holder:SetAttribute("Instance", Shared)
	Holder:SetAttribute("OwnInstance", Mine)
end
Attributes(Joined)
Joined.Parent = Storage
local Got = {}
Event.OnServerEvent:Connect(function(_, Index, ...)
	table.insert(Got, "event " .. Receive(Index, ...))
end)
Unreliable.OnServerEvent:Connect(function(_, Index, ...)
	table.insert(Got, "unreliable " .. Receive(Index, ...))
end)
Function.OnServerInvoke = function(_, Index, ...)
	table.insert(Got, "invoke " .. Receive(Index, ...))
	return Cases({ Shared = Shared, Mine = Mine, Loose = Instance.new("Folder"), Gone = Gone, Player = Player })[Index][2]()
end
P.meet("ready")
Attributes(Later)
Gone:Destroy()
local Own = { Shared = Shared, Mine = Mine, Loose = Instance.new("Folder"), Gone = Gone, Player = Player }
Send("event", function(...)
	Event:FireClient(Player, ...)
end, Own)
Send("unreliable", function(...)
	Unreliable:FireClient(Player, ...)
end, Own)
P.meet("sent")
P.meet("client sent")
for _ = 1, 10 do
	task.wait()
end
for _, Line in Got do
	print(Line)
end
--- client
local Event = Storage:WaitForChild("Handles")
local Unreliable = Storage:WaitForChild("HandlesUnreliable")
local Function = Storage:WaitForChild("HandlesFunction")
local Shared = workspace:WaitForChild("Shared")
local Gone = Storage:WaitForChild("Gone")
local Got = {}
Event.OnClientEvent:Connect(function(Index, ...)
	table.insert(Got, "event " .. Receive(Index, ...))
end)
Unreliable.OnClientEvent:Connect(function(Index, ...)
	table.insert(Got, "unreliable " .. Receive(Index, ...))
end)
P.meet("ready")
P.meet("sent")
for _ = 1, 10 do
	task.wait()
end
for _, Line in Got do
	print(Line)
end
for _, Holder in { Storage:WaitForChild("JoinedHolder"), Storage:WaitForChild("LaterHolder") } do
	local Names = {}
	for Name in Holder:GetAttributes() do
		table.insert(Names, Name)
	end
	table.sort(Names)
	for _, Name in Names do
		print("attribute " .. Holder.Name .. " " .. Name .. " " .. Dump(Holder:GetAttribute(Name)))
	end
	print("attribute " .. Holder.Name .. " names " .. table.concat(Names, ","))
end
local Mine = Instance.new("Folder")
Mine.Name = "ClientOnly"
Mine.Parent = Storage
local Own = { Shared = Shared, Mine = Mine, Loose = Instance.new("Folder"), Gone = Gone, Player = P.player() }
Send("event", function(...)
	Event:FireServer(...)
end, Own)
Send("unreliable", function(...)
	Unreliable:FireServer(...)
end, Own)
for Index, Case in Cases(Own) do
	local Ok, Problem = pcall(function(...)
		print("invoked " .. Receive(Index, Function:InvokeServer(Index, ...)))
	end, Case[2]())
	if not Ok then
		print("send invoke " .. Index .. " " .. Case[1] .. " error " .. P.err(Problem))
	end
end
P.meet("client sent")
