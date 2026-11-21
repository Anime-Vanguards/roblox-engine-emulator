-- How deep a table may nest when BindableEvent:Fire, BindableFunction:Invoke (its arguments and its callback's results),
-- a ValueCurveKey's Value and, on a server, the data stores, memory stores and MessagingService copy it: the deepest
-- nesting each takes (found by bisection), the error one level past it, how the table's shape, the call's other
-- arguments and the caller's own stack move the limit, what a receiver gets at the limit, which error wins for a cycle
-- that closes one level past it (and for JSONEncode, whose limit json_limits.luau has), which wins in a store beside a
-- value it cannot hold, and the error for a table nested 100,000 deep.
local function Nest(Depth, Key)
	local Root = {}
	local At = Root
	for _ = 2, Depth do
		local Inner = {}
		At[Key or 1] = Inner
		At = Inner
	end
	return Root
end
-- a chain of Depth tables whose innermost holds the outermost
local function Ring(Depth)
	local Root = {}
	local At = Root
	for _ = 2, Depth do
		local Inner = {}
		At[1] = Inner
		At = Inner
	end
	At[1] = Root
	return Root
end
local function Height(Value, Key)
	local Count = 0
	while type(Value) == "table" and Count < 1000000 do
		Count += 1
		Value = Value[Key or 1]
	end
	return Count
end
-- the deepest nesting in [Low, High] that Works takes, given that Low does
local function Deepest(Works, Low, High)
	while Low < High do
		local Middle = (Low + High + 1) // 2
		if Works(Middle) then
			Low = Middle
		else
			High = Middle - 1
		end
	end
	return Low
end
local function Deeper(Level, Call)
	if Level > 0 then
		local Found = Deeper(Level - 1, Call)
		return Found
	end
	return Call()
end

-- BindableEvent:Fire: each form bisected, the error one past, and the greatest height its handler got
local Event = Instance.new("BindableEvent")
local Got = {}
Event.Event:Connect(function(...)
	local Args = table.pack(...)
	local Best = 0
	for Index = 1, Args.n do
		Best = math.max(Best, Height(Args[Index]), Height(Args[Index], "a"))
	end
	table.insert(Got, Best)
end)
local Forms = {
	{ "fire", function(T) Event:Fire(T) end },
	{ "fire-dict", function(_, D) Event:Fire(D) end },
	{ "fire-2nd", function(T) Event:Fire(1, T) end },
	{ "fire-6th", function(T) Event:Fire(1, 2, 3, 4, 5, T) end },
	{ "fire-20th", function(T) Event:Fire(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, T) end },
	{ "fire-1st-of-2", function(T) Event:Fire(T, 1) end },
	{ "fire-two-deep", function(T, _, Depth) Event:Fire(T, Nest(Depth)) end },
	{ "fire-under-dict", function(T) Event:Fire({ Key = T }) end },
	{ "fire-from-deeper", function(T) Deeper(60, function() Event:Fire(T) end) end },
	{ "fire-dict-2nd", function(_, D) Event:Fire(1, D) end },
	{ "fire-at-2", function(T) Event:Fire({ 1, T }) end },
	{ "fire-at-key-2", function(T) Event:Fire({ [2] = T }) end },
	{ "fire-dict-with-more", function(T) Event:Fire({ a = T, b = 1, c = {} }) end },
}
local FireDeepest = 0
for _, Form in Forms do
	local Name, Send = Form[1], Form[2]
	local function Works(Depth)
		return (pcall(Send, Nest(Depth), Nest(Depth, "a"), Depth))
	end
	local Deep = Deepest(Works, 1, 2000)
	if Name == "fire" then
		FireDeepest = Deep
	end
	task.wait()
	table.clear(Got)
	Works(Deep)
	task.wait()
	local Past = P.try(Send, Nest(Deep + 1), Nest(Deep + 1, "a"), Deep + 1)
	print(Name .. " deepest " .. Deep .. ", got " .. tostring(Got[#Got]) .. "; one more: " .. Past)
end
print("fire-error-position " .. P.try(function()
	Event:Fire(Nest(FireDeepest + 1))
end))
print("fire-ring-at-limit " .. P.try(function()
	Event:Fire(Ring(FireDeepest))
end))
print("fire-ring-one-more " .. P.try(function()
	Event:Fire(Ring(FireDeepest + 1))
end))
print("fire-ring-small " .. P.try(function()
	Event:Fire(Ring(3))
end))
-- a shallow table after many arguments
local function After(Count, Last)
	local Args = table.create(Count, 0)
	if Last ~= nil then
		table.insert(Args, Last)
	end
	return Args
end
for _, Case in { { 298, {} }, { 299, {} }, { 300, {} }, { 297, { {} } }, { 298, { {} } }, { 400, nil }, { 400, {} }, { 150, Nest(150) } } do
	local Args = After(Case[1], Case[2])
	print("fire-" .. Case[1] .. "-numbers-then-" .. (if Case[2] then Height(Case[2]) else 0) .. " " .. P.try(function()
		Event:Fire(table.unpack(Args, 1, #Args))
	end))
end
task.wait()

-- BindableFunction:Invoke's arguments and its callback's results
local Echo = Instance.new("BindableFunction")
Echo.OnInvoke = function(...)
	local Args = table.pack(...)
	local Best = 0
	for Index = 1, Args.n do
		Best = math.max(Best, Height(Args[Index]), Height(Args[Index], "a"))
	end
	return Best
end
local Maker = Instance.new("BindableFunction")
Maker.OnInvoke = function(Depth, Second)
	if Second == "dict" then
		return Nest(Depth, "a")
	elseif Second then
		return 1, Nest(Depth)
	end
	return Nest(Depth)
end
local Calls = {
	{ "invoke-arg", function(T) return Echo:Invoke(T) end },
	{ "invoke-2nd-arg", function(T) return Echo:Invoke(1, T) end },
	{ "invoke-result", function(_, Depth) return Height(Maker:Invoke(Depth)) end },
	{ "invoke-2nd-result", function(_, Depth) return Height(select(2, Maker:Invoke(Depth, true))) end },
	{ "invoke-dict-arg", function(_, Depth) return Echo:Invoke(Nest(Depth, "a")) end },
	{ "invoke-dict-result", function(_, Depth) return Height(Maker:Invoke(Depth, "dict"), "a") end },
}
for _, Call in Calls do
	local Name, Run = Call[1], Call[2]
	local function Works(Depth)
		return (pcall(Run, Nest(Depth), Depth))
	end
	local Deep = Deepest(Works, 1, 2000)
	print(Name .. " deepest " .. Deep .. ", got " .. P.try(Run, Nest(Deep), Deep) .. "; one more: " .. P.try(Run, Nest(Deep + 1), Deep + 1))
end
print("invoke-ring-at-limit " .. P.try(function()
	return Echo:Invoke(Ring(FireDeepest))
end))
print("invoke-299-numbers-then-1 " .. P.try(function()
	return Echo:Invoke(table.unpack(After(299, {})))
end))
print("invoke-298-numbers-then-1 " .. P.try(function()
	return Echo:Invoke(table.unpack(After(298, {})))
end))

-- ValueCurveKey: made with a nested Value, a nested Value written, and the Value read back
local Keys = {
	{ "key-new", function(T) return Height(ValueCurveKey.new(0, T).Value) end },
	{ "key-new-3", function(T) return Height(ValueCurveKey.new(0, T, Enum.KeyInterpolationMode.Linear).Value) end },
	{ "key-new-dict", function(_, D) return Height(ValueCurveKey.new(0, D).Value, "a") end },
	{ "key-write", function(T)
		local Key = ValueCurveKey.new(0, 1)
		Key.Value = T
		return Height(Key.Value)
	end },
}
for _, Case in Keys do
	local Name, Run = Case[1], Case[2]
	local function Works(Depth)
		return (pcall(Run, Nest(Depth), Nest(Depth, "a")))
	end
	local Deep = Deepest(Works, 1, 2000)
	print(Name .. " deepest " .. Deep .. ", got " .. P.try(Run, Nest(Deep), Nest(Deep, "a")) .. "; one more: " .. P.try(Run, Nest(Deep + 1), Nest(Deep + 1, "a")))
end

-- JSONEncode beside them (json_limits.luau has it bisected)
local Http = game:GetService("HttpService")
print("json-encode 300 " .. P.try(function() return #Http:JSONEncode(Nest(300)) end) .. ", 301 " .. P.try(function() return #Http:JSONEncode(Nest(301)) end))
print("json-ring-300 " .. P.try(function() return #Http:JSONEncode(Ring(300)) end) .. ", 301 " .. P.try(function() return #Http:JSONEncode(Ring(301)) end))

-- A server's data stores, memory stores and MessagingService (in a playing server; a client cannot reach them)
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
	local Store = game:GetService("DataStoreService"):GetDataStore("ParityDepth")
	local Memory = game:GetService("MemoryStoreService")
	local Map, Hash, Queue = Memory:GetSortedMap("ParityDepth"), Memory:GetHashMap("ParityDepth"), Memory:GetQueue("ParityDepth")
	local Messaging = game:GetService("MessagingService")
	local Stores = {
		{ "datastore-set", function(T) Store:SetAsync("k", T) end },
		{ "datastore-set-dict", function(_, D) Store:SetAsync("k", D) end },
		{ "datastore-set-options", function(T) Store:SetAsync("k", T, { 1 }, Instance.new("DataStoreSetOptions")) end },
		{ "datastore-update", function(T) Store:UpdateAsync("k", function() return T end) end },
		{ "datastore-update-dict", function(_, D) Store:UpdateAsync("k", function() return D end) end },
		{ "datastore-update-3", function(T) Store:UpdateAsync("k", function() return T, { 1 }, nil end) end },
		{ "sortedmap-set", function(T) Map:SetAsync("k", T, 30) end },
		{ "sortedmap-set-dict", function(_, D) Map:SetAsync("k", D, 30) end },
		{ "sortedmap-update", function(T) Map:UpdateAsync("k", function() return T end, 30) end },
		{ "hashmap-set", function(T) Hash:SetAsync("k", T, 30) end },
		{ "hashmap-update", function(T) Hash:UpdateAsync("k", function() return T end, 30) end },
		{ "queue-add", function(T) Queue:AddAsync(T, 30) end },
		{ "queue-add-priority", function(T) Queue:AddAsync(T, 30, 1) end },
		{ "messaging", function(T) Messaging:PublishAsync("ParityDepth", T) end },
		{ "messaging-dict", function(_, D) Messaging:PublishAsync("ParityDepth", D) end },
	}
	for _, Case in Stores do
		local Name, Run = Case[1], Case[2]
		local function Works(Depth)
			return (pcall(Run, Nest(Depth), Nest(Depth, "a")))
		end
		local Deep = Deepest(Works, 100, 400)
		print(Name .. " deepest " .. Deep .. "; one more: " .. Bounded(function() Run(Nest(Deep + 1), Nest(Deep + 1, "a")) end))
	end
	print("datastore-set-ring-small " .. Bounded(function() Store:SetAsync("k", Ring(3)) end))
	print("datastore-set-function-then-deep " .. Bounded(function() Store:SetAsync("k", { print, Nest(400) }) end))
	print("datastore-set-deep-then-function " .. Bounded(function() Store:SetAsync("k", { Nest(400), print }) end))
	print("datastore-set-function-then-ring " .. Bounded(function() Store:SetAsync("k", { print, Ring(3) }) end))
	print("datastore-set-ring-then-function " .. Bounded(function() Store:SetAsync("k", { Ring(3), print }) end))
	print("datastore-set-deep-ring " .. Bounded(function() Store:SetAsync("k", Ring(300)) end))
	print("messaging-ring " .. Bounded(function() Messaging:PublishAsync("ParityDepth", Ring(3)) end))
	print("sortedmap-ring " .. Bounded(function() Map:SetAsync("k", Ring(3), 30) end))
	print("datastore-set-huge " .. Bounded(function() Store:SetAsync("k", Nest(100000)) end))
end

-- 100,000 deep
local Huge = Nest(100000)
print("huge fire " .. P.try(function() Event:Fire(Huge) end))
print("huge invoke-arg " .. P.try(function() return Echo:Invoke(Huge) end))
print("huge invoke-result " .. P.try(function() return Height(Maker:Invoke(100000)) end))
print("huge key-new " .. P.try(function() return ValueCurveKey.new(0, Huge) end))
print("huge json-encode " .. P.try(function() return #Http:JSONEncode(Huge) end))
