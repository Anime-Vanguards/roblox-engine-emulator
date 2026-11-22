-- How a table that appears many times is copied when BindableEvent:Fire, BindableFunction:Invoke (its arguments and its
-- callback's results), a ValueCurveKey's Value and JSONEncode copy it: once for each place it appears, each receiver a copy
-- of its own, with no limit of its own (the time grows with the values: lua/bindable.rs). Printed: what arrives from one
-- table of 1,000 numbers held 1,000 times and from a tree of `{ t, t }` 17 deep, JSONEncode's length of them and of the same
-- table held 10,000 times (10,000,000 values, past ree's own guard on a copy), and on a server what a data store, a memory
-- store and MessagingService make of such a value: a cycle after 4 MB of it, and values past their documented limits.
local function Fan(Count, Size)
	return table.create(Count, table.create(Size, 1))
end
local function Tree(Depth)
	local Value = {}
	for _ = 1, Depth do
		Value = { Value, Value }
	end
	return Value
end
local function Ring()
	local Value = {}
	Value[1] = Value
	return Value
end
-- what a fan-out arrived as: its length, its last item's length, the numbers in all, and whether its items are tables of their own
local function Shape(Value)
	local Numbers = 0
	for _, Item in Value do
		Numbers += #Item
	end
	return `{#Value} {#Value[#Value]} {Numbers} {Value[1] ~= Value[2]}`
end
local function Tables(Value)
	local Count = 1
	for _, Item in Value do
		Count += Tables(Item)
	end
	return Count
end

local Event = Instance.new("BindableEvent")
local Got = {}
Event.Event:Connect(function(Value)
	table.insert(Got, Value)
end)
Event.Event:Connect(function(Value)
	table.insert(Got, Value)
end)
local function Fired(Value)
	table.clear(Got)
	Event:Fire(Value)
	task.wait()
	return Got[1], Got[2]
end
local First, Second = Fired(Fan(1000, 1000))
print(`fire fan 1000x1000: {Shape(First)}; the two handlers' copies differ {First ~= Second and First[1] ~= Second[1]}`)
print(`fire tree 17: {Tables(Fired(Tree(17)))} tables`)
local Echo = Instance.new("BindableFunction")
Echo.OnInvoke = function(Value)
	return Shape(Value)
end
print("invoke fan 1000x1000 argument: " .. P.try(function() return Echo:Invoke(Fan(1000, 1000)) end))
local Maker = Instance.new("BindableFunction")
Maker.OnInvoke = function()
	return Fan(1000, 1000)
end
print("invoke fan 1000x1000 result: " .. P.try(function() return Shape(Maker:Invoke()) end))
print("key fan 1000x1000: " .. P.try(function() return Shape(ValueCurveKey.new(0, Fan(1000, 1000)).Value) end))
task.wait()

local Http = game:GetService("HttpService")
for _, Case in { { "fan 1000x1000", function() return Fan(1000, 1000) end }, { "tree 17", function() return Tree(17) end }, { "fan 10000x1000", function() return Fan(10000, 1000) end } } do
	print(`json {Case[1]}: ` .. P.try(function() return #Http:JSONEncode(Case[2]()) end))
	task.wait()
end

-- A server's stores (a client cannot reach them)
if game:GetService("RunService"):IsServer() then
	local function Bounded(Call)
		local Result = "did not return in 600 frames"
		local Done = false
		task.spawn(function()
			Result = P.try(Call)
			Done = true
		end)
		for _ = 1, 600 do
			if Done then
				break
			end
			task.wait()
		end
		return Result
	end
	local Store = game:GetService("DataStoreService"):GetDataStore("ParityBreadth")
	local Map = game:GetService("MemoryStoreService"):GetSortedMap("ParityBreadth")
	local Messaging = game:GetService("MessagingService")
	-- (a table of 1,000 ones is 2,001 bytes of JSON: 2,100 of them are 4,204,101 bytes, past 4 MB)
	print("datastore-set fan 2100x1000 then ring: " .. Bounded(function() Store:SetAsync("k", { Fan(2100, 1000), Ring() }) end))
	print("messaging fan 1000x1000 then ring: " .. Bounded(function() Messaging:PublishAsync("ParityBreadth", { Fan(1000, 1000), Ring() }) end))
	print("datastore-set fan 2100x1000: " .. Bounded(function() Store:SetAsync("k", Fan(2100, 1000)) end))
	print("sortedmap-set fan 20x1000: " .. Bounded(function() Map:SetAsync("k", Fan(20, 1000), 30) end))
	print("messaging fan 1x600: " .. Bounded(function() Messaging:PublishAsync("ParityBreadth", Fan(1, 600)) end))
end
