-- parity: loopback
-- How a buffer counts against an UnreliableRemoteEvent's 1000-byte ceiling: buffers of repeated bytes, short patterns,
-- random bytes and random bytes beside zeros, alone and many to a table, swept across the cutoff each way, with a reliable
-- control. Printed: per kind, the sizes that arrived and the sender's "went over by" counts in send order.
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
local Over = {}
LogService.MessageOut:Connect(function(Message)
	local Count = string.match(Message, "Payload went over by: (%d+) bytes")
	if Count then
		table.insert(Over, Count)
	end
end)
local function Random(Size, Seed)
	local Rng = P.rng(Seed)
	local Bytes = table.create(Size)
	for Index = 1, Size do
		Bytes[Index] = string.char(Rng.u32() % 256)
	end
	return table.concat(Bytes)
end
local function Range(Low, High, Step)
	local List = {}
	for Size = Low, High, Step do
		table.insert(List, Size)
	end
	return List
end
local BIG = { 1000, 1200, 1500, 2000, 3000, 5000, 8000, 10000, 15000, 20000, 30000, 50000, 70000, 100000, 150000, 200000, 300000, 500000, 1000000 }
local function Alphabet(Size, Count, Seed)
	local Rng = P.rng(Seed)
	local Bytes = table.create(Size)
	for Index = 1, Size do
		Bytes[Index] = string.char(65 + Rng.u32() % Count)
	end
	return table.concat(Bytes)
end
-- each kind: its name, its sizes, and the buffer of a size (or a special argument list)
local KINDS = {
	{ "repeated", BIG, function(Size) return buffer.fromstring(string.rep("x", Size)) end },
	{ "zeros", BIG, function(Size) return buffer.create(Size) end },
	{ "pattern-ab", BIG, function(Size) return buffer.fromstring(string.sub(string.rep("ab", Size // 2 + 1), 1, Size)) end },
	{ "block16", BIG, function(Size) return buffer.fromstring(string.sub(string.rep(Random(16, 3), Size // 16 + 1), 1, Size)) end },
	{ "block256", BIG, function(Size) return buffer.fromstring(string.sub(string.rep(Random(256, 4), Size // 256 + 1), 1, Size)) end },
	{ "random", Range(950, 1000, 1), function(Size) return buffer.fromstring(Random(Size, Size)) end },
	{ "random+zeros2000", Range(860, 1000, 2), function(Size) return buffer.fromstring(Random(Size, Size + 1) .. string.rep("\0", 2000 - Size)) end },
	{ "random+zeros20000", Range(860, 1000, 2), function(Size) return buffer.fromstring(Random(Size, Size + 2) .. string.rep("\0", 20000 - Size)) end },
	{ "zeros2000+random", Range(860, 1000, 2), function(Size) return buffer.fromstring(string.rep("\0", 2000 - Size) .. Random(Size, Size + 5)) end },
	{ "small-repeated", Range(900, 1000, 4), function(Size) return buffer.fromstring(string.rep("x", Size)) end },
	{ "pad+zeros2000", Range(900, 1000, 2), "pad" },
	{ "two-buffers", Range(400, 600, 4), "two" },
	{ "nibbles", Range(1700, 2300, 10), function(Size) return buffer.fromstring(Alphabet(Size, 16, Size)) end },
	{ "four", Range(3000, 5000, 40), function(Size) return buffer.fromstring(Alphabet(Size, 4, Size + 1)) end },
	{ "random900+zeros", Range(0, 80, 2), function(Size) return buffer.fromstring(Random(900, 77) .. string.rep("\0", Size)) end },
	{ "random960+zeros", Range(0, 40, 1), function(Size) return buffer.fromstring(Random(960, 78) .. string.rep("\0", Size)) end },
	{ "small-zero-buffers", Range(30, 90, 2), "smallzeros" },
	{ "small-random-buffers", Range(30, 50, 1), "smallrandom" },
	{ "zeros-tiny", Range(1, 40, 1), "tiny" },
}
local Arrived = {}
local function Note(Kind, Size)
	Arrived[Kind] = Arrived[Kind] or {}
	Arrived[Kind][Size] = true
end
local function Args(Kind, Size)
	local Make = KINDS[Kind][3]
	if Make == "smallzeros" then
		local List = {}
		for Index = 1, Size do
			List[Index] = buffer.create(20)
		end
		return List
	elseif Make == "smallrandom" then
		local List = {}
		for Index = 1, Size do
			List[Index] = buffer.fromstring(Random(20, Index))
		end
		return List
	elseif Make == "pad" then
		-- a string (sent as it is) beside a compressible buffer
		return string.rep("s", Size), buffer.create(2000)
	elseif Make == "two" then
		-- two random buffers of Size bytes each
		return buffer.fromstring(Random(Size, Size + 9)), buffer.fromstring(Random(Size, Size + 11))
	elseif Make == "tiny" then
		-- a string near the ceiling beside a small buffer of zeros: does a small buffer compress
		return string.rep("s", 940), buffer.create(Size)
	end
	return Make(Size)
end
local function Sweep(Fire)
	local Report = {}
	for Kind, Info in KINDS do
		table.clear(Over)
		local Sent = 0
		for _, Size in Info[2] do
			Fire(Kind, Size, Args(Kind, Size))
			Sent += 1
			if Sent % 6 == 0 then
				RunService.Heartbeat:Wait()
			end
		end
		task.wait()
		task.wait()
		Report[Kind] = table.concat(Over, " ")
	end
	return Report
end
local function Print(Prefix, Control, Over)
	for Kind, Info in KINDS do
		if Over then
			print(Prefix .. " " .. Info[1] .. ": over " .. Over[Kind])
			continue
		end
		local Got, Missing, Controls = {}, {}, 0
		for _, Size in Info[2] do
			if Control[Kind] and Control[Kind][Size] then
				Controls += 1
			end
			table.insert(if Arrived[Kind] and Arrived[Kind][Size] then Got else Missing, Size)
		end
		print(Prefix .. " " .. Info[1] .. ": arrived " .. table.concat(Got, " ") .. " ; dropped " .. table.concat(Missing, " ") .. " ; controls " .. Controls .. "/" .. #Info[2])
	end
end
--- server
local Player = P.player()
local Unreliable = Instance.new("UnreliableRemoteEvent")
Unreliable.Name = "Unreliable"
Unreliable.Parent = Storage
local Control = Instance.new("RemoteEvent")
Control.Name = "Control"
Control.Parent = Storage
local Controls = {}
Unreliable.OnServerEvent:Connect(function(_, Kind, Size)
	Note(Kind, Size)
end)
Control.OnServerEvent:Connect(function(_, Kind, Size)
	Controls[Kind] = Controls[Kind] or {}
	Controls[Kind][Size] = true
end)
P.meet("ready")
local Over = Sweep(function(Kind, Size, ...)
	Unreliable:FireClient(Player, Kind, Size, ...)
	Control:FireClient(Player, Kind, Size)
end)
P.meet("server sent")
P.meet("client sent")
for _ = 1, 30 do
	task.wait()
end
Print("server-sent", {}, Over)
Print("server-got", Controls, nil)
--- client
local Unreliable = Storage:WaitForChild("Unreliable")
local Control = Storage:WaitForChild("Control")
local Controls = {}
Unreliable.OnClientEvent:Connect(function(Kind, Size)
	Note(Kind, Size)
end)
Control.OnClientEvent:Connect(function(Kind, Size)
	Controls[Kind] = Controls[Kind] or {}
	Controls[Kind][Size] = true
end)
P.meet("ready")
P.meet("server sent")
for _ = 1, 30 do
	task.wait()
end
Print("client-got", Controls, nil)
table.clear(Arrived)
local Over = Sweep(function(Kind, Size, ...)
	Unreliable:FireServer(Kind, Size, ...)
	Control:FireServer(Kind, Size)
end)
Print("client-sent", {}, Over)
P.meet("client sent")
