-- parity: loopback
-- Which error a remote call raises first, on each side of a play session: an argument that cannot be sent (a table holding itself, a table key), the player
-- argument, the side the call is made on, and a thread that cannot yield, for a remote in the DataModel and a parentless one.
-- Each call from a thread of its own ("pending" while it waits) and from a sort comparator ("refused"). Remotes made here are
-- this side's alone (a server's reach the client too), so nothing answers an invoke. The player: the session's.
local Storage = game:GetService("ReplicatedStorage")
local Cyclic = {}
Cyclic.Self = Cyclic
local BadKey = { [{}] = 1 }
local Part = Instance.new("Part")
local function Make(Class, Parented)
	local Remote = Instance.new(Class)
	Remote.Name = "ParityOrder" .. Class
	if Parented then
		Remote.Parent = Storage
	end
	return Remote
end
local Results = {}
local function Spawned(Label, Function)
	local Line = "pending"
	task.spawn(function()
		local Ok, Message = pcall(Function)
		Line = if Ok then "ok" else "error " .. P.err(Message)
	end)
	table.insert(Results, function()
		print(Label .. " " .. Line)
	end)
end
local function Sorted(Label, Function)
	local Line
	table.sort({ 2, 1 }, function(A, B)
		if not Line then
			local Ok, Message = pcall(Function)
			Line = if Ok then "ok" else "error " .. P.err(Message)
		end
		return A < B
	end)
	print("refused " .. Label .. " " .. Line)
end
local function Run(Player)
for _, Parented in { true, false } do
	local Where = if Parented then "parented" else "parentless"
	local Event = Make("RemoteEvent", Parented)
	local Unreliable = Make("UnreliableRemoteEvent", Parented)
	local Function = Make("RemoteFunction", Parented)
	local Calls = {
		{ "FireServer-cyclic", function() Event:FireServer(Cyclic) end },
		{ "FireServer-badkey", function() Event:FireServer(BadKey) end },
		{ "FireServer-1", function() Event:FireServer(1) end },
		{ "FireClient-nil-cyclic", function() Event:FireClient(nil, Cyclic) end },
		{ "FireClient-part-cyclic", function() Event:FireClient(Part, Cyclic) end },
		{ "FireClient-nil-1", function() Event:FireClient(nil, 1) end },
		{ "FireAllClients-cyclic", function() Event:FireAllClients(Cyclic) end },
		{ "FireAllClients-1", function() Event:FireAllClients(1) end },
		{ "Unreliable-FireServer-cyclic", function() Unreliable:FireServer(Cyclic) end },
		{ "Unreliable-FireClient-nil-cyclic", function() Unreliable:FireClient(nil, Cyclic) end },
		{ "InvokeServer-cyclic", function() return Function:InvokeServer(Cyclic) end },
		{ "InvokeServer-badkey", function() return Function:InvokeServer(BadKey) end },
		{ "InvokeServer-1", function() return Function:InvokeServer(1) end },
		{ "InvokeClient-nil-cyclic", function() return Function:InvokeClient(nil, Cyclic) end },
		{ "InvokeClient-part-cyclic", function() return Function:InvokeClient(Part, Cyclic) end },
		{ "InvokeClient-nil-1", function() return Function:InvokeClient(nil, 1) end },
		{ "InvokeClient-part-1", function() return Function:InvokeClient(Part, 1) end },
		{ "FireClient-player-cyclic", function() Event:FireClient(Player, Cyclic) end },
		{ "InvokeClient-player-cyclic", function() return Function:InvokeClient(Player, Cyclic) end },
		{ "InvokeClient-player-1", function() return Function:InvokeClient(Player, 1) end },
	}
	for _, Call in Calls do
		Spawned(Where .. " " .. Call[1], Call[2])
		Sorted(Where .. " " .. Call[1], Call[2])
	end
end
task.wait()
task.wait()
for _, Report in Results do
	Report()
end
end
--- server
Run(P.player())
P.meet("server done")
--- client
P.meet("server done")
Run(game:GetService("Players").LocalPlayer)
