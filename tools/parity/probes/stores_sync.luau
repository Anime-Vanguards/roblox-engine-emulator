-- The data stores' and memory stores' synchronous members, on a client and a server: the constructors (names and scopes cast,
-- the store a child of its service), the request budget's cast and SetRateLimitForRequestType's checks, and what a client is refused.
P.sync()
local function Show(V)
	if typeof(V) == "Instance" then
		return "Instance " .. V.ClassName .. " '" .. V.Name .. "' parent " .. tostring(V.Parent and V.Parent.ClassName)
	end
	return typeof(V) .. " " .. tostring(V)
end
local function Try(Label, F)
	local Results = table.pack(pcall(F))
	local Parts = {}
	for Index = 2, Results.n do
		Parts[#Parts + 1] = Show(Results[Index])
	end
	print(Label .. " " .. (if Results[1] then "ok " .. table.concat(Parts, ", ") else "error " .. P.err(Results[2])))
end
local DSS = game:GetService("DataStoreService")
local MSS = game:GetService("MemoryStoreService")
local Names = { { "nil", nil }, { "string", "s1" }, { "number", 5 }, { "fraction", 1.5 }, { "bool", true }, { "table", {} }, { "array", { 1, 2 } },
	{ "vector", Vector3.new(1, 2, 3) }, { "instance", workspace }, { "enum", Enum.KeyCode.A }, { "empty", "" }, { "long", string.rep("a", 51) }, { "50", string.rep("b", 50) } }
for _, Case in Names do
	Try("GetDataStore " .. Case[1], function()
		return DSS:GetDataStore(Case[2])
	end)
end
for _, Case in Names do
	Try("GetDataStore scope " .. Case[1], function()
		return DSS:GetDataStore("scoped", Case[2])
	end)
end
for _, Case in Names do
	Try("GetOrderedDataStore " .. Case[1], function()
		return DSS:GetOrderedDataStore(Case[2])
	end)
end
Try("GetDataStore options number", function()
	return DSS:GetDataStore("o", nil, 5)
end)
Try("GetDataStore options instance", function()
	return DSS:GetDataStore("o", nil, Instance.new("Folder"))
end)
Try("GetGlobalDataStore", function()
	return DSS:GetGlobalDataStore()
end)
Try("same store", function()
	return DSS:GetDataStore("s1") == DSS:GetDataStore("s1"), DSS:GetDataStore("s1") == DSS:GetDataStore("s1", "global")
end)
Try("children", function()
	local Names = {}
	for _, Child in DSS:GetChildren() do
		table.insert(Names, Child.ClassName .. ":" .. Child.Name)
	end
	table.sort(Names)
	return #Names, table.concat(Names, ",")
end)
for _, Case in { { "nil", nil }, { "number", 0 }, { "string", "GetAsync" }, { "enum", Enum.DataStoreRequestType.GetAsync }, { "other enum", Enum.KeyCode.A } } do
	Try("budget " .. Case[1], function()
		local B = DSS:GetRequestBudgetForRequestType(Case[2])
		return type(B)
	end)
end
for _, Case in { { "none", {} }, { "one", { Enum.DataStoreRequestType.GetAsync } }, { "two", { Enum.DataStoreRequestType.GetAsync, 10 } },
	{ "three", { Enum.DataStoreRequestType.GetAsync, 10, 2 } }, { "three nil type", { nil, 10, 2 } }, { "three number type", { 0, 10, 2 } },
	{ "three string type", { "GetAsync", 10, 2 } }, { "three bad base", { Enum.DataStoreRequestType.GetAsync, "x", 2 } },
	{ "three bad per", { Enum.DataStoreRequestType.GetAsync, 10, "x" } }, { "updateasync", { Enum.DataStoreRequestType.UpdateAsync, 10, 2 } },
	{ "onupdate", { Enum.DataStoreRequestType.OnUpdate, 10, 2 } }, { "over", { Enum.DataStoreRequestType.GetAsync, 61, 2 } },
	{ "negative", { Enum.DataStoreRequestType.GetAsync, -1, 2 } }, { "fraction", { Enum.DataStoreRequestType.GetAsync, 10.5, 2 } } } do
	Try("ratelimit " .. Case[1], function()
		local A = Case[2]
		return DSS:SetRateLimitForRequestType(A[1], A[2], A[3])
	end)
end
for _, Case in Names do
	Try("GetSortedMap " .. Case[1], function()
		return MSS:GetSortedMap(Case[2])
	end)
end
for _, Case in Names do
	Try("GetDistributedCounter " .. Case[1], function()
		return MSS:GetDistributedCounter(Case[2])
	end)
end
Try("GetQueue timeout string", function()
	return MSS:GetQueue("q", "x")
end)
Try("GetQueue no name", function()
	return MSS:GetQueue()
end)
Try("GetHashMap no name", function()
	return MSS:GetHashMap()
end)
Try("GetQueue name", function()
	local Q = MSS:GetQueue("q")
	return Q, Q == MSS:GetQueue("q")
end)
Try("GetHashMap name", function()
	local H = MSS:GetHashMap("h")
	return H, H == MSS:GetHashMap("h")
end)
Try("memory children", function()
	return #MSS:GetChildren()
end)
