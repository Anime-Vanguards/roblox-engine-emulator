-- ValueCurve and CompositeValueCurve: what each method takes, answers and refuses, and what the curves keep. On a 0.747
-- client and server both are inert: the casts happen and nothing is kept.
P.sync()
local function Show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return "number " .. P.num(Value)
	elseif Kind == "table" then
		local Parts = {}
		for Key, Item in Value do
			table.insert(Parts, tostring(Key) .. "=" .. (if type(Item) == "table" then "table" else tostring(Item)))
		end
		table.sort(Parts)
		return "table#" .. #Value .. "{" .. table.concat(Parts, ",") .. "}"
	elseif Kind == "ValueCurveKey" then
		return "key(" .. Show(Value.Time) .. "|" .. Show(Value.Value) .. "|" .. tostring(Value.Interpolation) .. "|" .. tostring(Value.LeftTangent) .. "|" .. tostring(Value.RightTangent) .. ")"
	end
	return Kind .. " " .. tostring(Value)
end
local function Try(Label, F, ...)
	local Results = table.pack(pcall(F, ...))
	if not Results[1] then
		print(Label, "error", P.err(Results[2]))
		return
	end
	local Parts = {}
	for Index = 2, Results.n do
		table.insert(Parts, Show(Results[Index]))
	end
	print(Label, "ok", Results.n - 1, table.concat(Parts, " ; "))
end
local function State(Label, C)
	print(Label, "Length", C.Length, "ValueType", C.ValueType)
end

local C = Instance.new("ValueCurve")
State("new", C)
Try("ValuesAndTimes", function() return C.ValuesAndTimes end)
Try("get empty", function() return C:GetValueAtTime(0) end)
Try("keys empty", function() return C:GetKeys() end)
Try("indices empty", function() return C:GetKeyIndicesAtTime(0) end)
Try("at 1 empty", function() return C:GetKeyAtIndex(1) end)
Try("insert value", function() return C:InsertKeyValue(1, 10) end)
State("one", C)
Try("insert value 2", function() return C:InsertKeyValue(0, 0, Enum.KeyInterpolationMode.Linear) end)
Try("insert value 3", function() return C:InsertKeyValue(3, 30, Enum.KeyInterpolationMode.Cubic) end)
Try("insert same", function() return C:InsertKeyValue(1, 11) end)
State("three", C)
Try("keys", function() return C:GetKeys() end)
for Index = 0, 4 do
	Try("at " .. Index, function() return C:GetKeyAtIndex(Index) end)
end
for _, Time in { -1, 0, 0.5, 1, 1.5, 3, 4 } do
	Try("value " .. Time, function() return C:GetValueAtTime(Time) end)
	Try("indices " .. Time, function() return C:GetKeyIndicesAtTime(Time) end)
end
Try("insert vector", function() return C:InsertKeyValue(2, Vector3.new(1, 2, 3)) end)
Try("insert string", function() return C:InsertKeyValue(4, "text") end)
Try("insert table", function() return C:InsertKeyValue(8, { 1, 2 }) end)
Try("insert function", function() return C:InsertKeyValue(0, function() end) end)
State("after inserts", C)
Try("keys now", function() return C:GetKeys() end)

-- InsertKeyValue's casts: the mode, then the value, then the time
local Modes = { { "name", "Nope" }, { "item name", "Cubic" }, { "number", 9 }, { "fraction", 1.5 }, { "value", 1 }, { "bool", true }, { "table", {} }, { "vector", Vector3.one }, { "other enum", Enum.Material.Plastic } }
for _, Case in Modes do
	Try("mode " .. Case[1], function() return C:InsertKeyValue(0, 1, Case[2]) end)
end
Try("mode nil", function() return C:InsertKeyValue(0, 1, nil) end)
Try("value nil", function() return C:InsertKeyValue(5, nil) end)
Try("value none", function() return C:InsertKeyValue(5) end)
Try("no arguments", function() return C:InsertKeyValue() end)
Try("time bool", function() return C:InsertKeyValue(true, 1) end)
Try("time nil", function() return C:InsertKeyValue(nil, 1) end)
Try("time number string", function() return C:InsertKeyValue("6", 60) end)
Try("time string", function() return C:InsertKeyValue("x", 1) end)
Try("time table", function() return C:InsertKeyValue({}, 60) end)
Try("time vector", function() return C:InsertKeyValue(Vector3.one, 1) end)
Try("time bad, value nil", function() return C:InsertKeyValue({}, nil) end)
Try("time bad, mode bad", function() return C:InsertKeyValue({}, 1, Enum.Material.Plastic) end)
Try("value nil, mode bad", function() return C:InsertKeyValue(0, nil, Enum.Material.Plastic) end)

-- keys
local D = Instance.new("ValueCurve")
Try("insertkey", function() return D:InsertKey(ValueCurveKey.new(1, Vector3.new(1, 0, 0), Enum.KeyInterpolationMode.Linear)) end)
State("D", D)
Try("insertkey nil", function() return D:InsertKey(nil) end)
Try("insertkey none", function() return D:InsertKey() end)
Try("insertkey number", function() return D:InsertKey(5) end)
Try("insertkey string", function() return D:InsertKey("x") end)
Try("insertkey table", function() return D:InsertKey({}) end)
Try("insertkey float key", function() return D:InsertKey(FloatCurveKey.new(0, 1)) end)
Try("insertkey rotation key", function() return D:InsertKey(RotationCurveKey.new(0, CFrame.new())) end)
Try("insertkey own key", function() return D:InsertKey(D:GetKeyAtIndex(1)) end)
Try("vec 0.25", function() return D:GetValueAtTime(0.25) end)
for _, Case in { { "string", "x" }, { "number string", "2" }, { "table", {} }, { "fraction", 1.5 }, { "negative", -3 }, { "big", 1e10 }, { "bool", true }, { "nan", 0 / 0 } } do
	Try("index " .. Case[1], function() return D:GetKeyAtIndex(Case[2]) end)
end
Try("index nil", function() return D:GetKeyAtIndex(nil) end)
Try("index none", function() return D:GetKeyAtIndex() end)
Try("key same", function() return D:GetKeyAtIndex(1) == D:GetKeyAtIndex(1), rawequal(D:GetKeyAtIndex(1), D:GetKeyAtIndex(1)), tostring(D:GetKeyAtIndex(1)) end)
Try("key write", function() local K = D:GetKeyAtIndex(1) K.Value = 5 K.Time = 2 return K.Value, K.Time, D:GetKeyAtIndex(1).Value end)
Try("key right tangent", function() local K = D:GetKeyAtIndex(1) K.RightTangent = 1 end)
Try("key equals new", function() return D:GetKeyAtIndex(1) == ValueCurveKey.new(0, 1) end)
Try("remove 1", function() return D:RemoveKeyAtIndex(1) end)
Try("remove 0", function() return D:RemoveKeyAtIndex(0) end)
Try("remove negative count", function() return D:RemoveKeyAtIndex(1, -1) end)
Try("remove 5", function() return D:RemoveKeyAtIndex(1, 5) end)
Try("remove string", function() return D:RemoveKeyAtIndex("x") end)
Try("remove nil", function() return D:RemoveKeyAtIndex(nil) end)
Try("remove count string", function() return D:RemoveKeyAtIndex(1, "x") end)
Try("remove count nil", function() return D:RemoveKeyAtIndex(1, nil) end)
Try("remove tables", function() return D:RemoveKeyAtIndex({}, {}) end)
Try("setkeys", function() return D:SetKeys({ ValueCurveKey.new(2, 1), ValueCurveKey.new(1, 0), ValueCurveKey.new(1, 9) }) end)
State("D set", D)
Try("setkeys empty", function() return D:SetKeys({}) end)
Try("setkeys numbers", function() return D:SetKeys({ 1, 2 }) end)
Try("setkeys nil", function() return D:SetKeys(nil) end)
Try("setkeys none", function() return D:SetKeys() end)
Try("setkeys number", function() return D:SetKeys(5) end)
Try("setkeys string", function() return D:SetKeys("x") end)
Try("setkeys dictionary", function() return D:SetKeys({ a = 1 }) end)
Try("value nil", function() return D:GetValueAtTime(nil) end)
Try("value string", function() return D:GetValueAtTime("x") end)
Try("value number string", function() return D:GetValueAtTime("1") end)
Try("value table", function() return D:GetValueAtTime({}) end)
Try("indices nil", function() return D:GetKeyIndicesAtTime(nil) end)
Try("indices table", function() return D:GetKeyIndicesAtTime({}) end)
State("D end", D)

-- kinds of value between two keys
local Types = {
	{ "number", 0, 10 },
	{ "Vector3", Vector3.new(0, 0, 0), Vector3.new(10, 20, 30) },
	{ "CFrame", CFrame.new(), CFrame.new(10, 0, 0) },
	{ "Color3", Color3.new(0, 0, 0), Color3.new(1, 0.5, 0.25) },
	{ "UDim2", UDim2.new(0, 0, 0, 0), UDim2.new(1, 100, 0.5, 50) },
	{ "string", "a", "b" },
	{ "boolean", false, true },
	{ "EnumItem", Enum.Material.Plastic, Enum.Material.Wood },
	{ "Instance", workspace, game },
	{ "table", { 1 }, { 2 } },
}
for _, Case in Types do
	for _, Mode in { Enum.KeyInterpolationMode.Linear, Enum.KeyInterpolationMode.Constant, Enum.KeyInterpolationMode.Cubic } do
		local E = Instance.new("ValueCurve")
		local Ok, Why = pcall(function()
			E:InsertKeyValue(0, Case[2], Mode)
			E:InsertKeyValue(1, Case[3], Mode)
		end)
		if not Ok then
			print("type", Case[1], Mode.Name, "insert error", P.err(Why))
		else
			print("type", Case[1], Mode.Name, E.ValueType, E.Length, Show(E:GetValueAtTime(0.25)), Show(E:GetValueAtTime(1.5)))
		end
	end
end

-- reads, writes and events
Try("write length", function() C.Length = 3 end)
Try("write type", function() C.ValueType = "number" end)
Try("clone", function() local X = C:Clone() return X.Length, X.ValueType end)
Try("changed", function()
	local Log = {}
	local E = Instance.new("ValueCurve")
	E.Changed:Connect(function(Name) table.insert(Log, Name) end)
	E:InsertKeyValue(0, 1)
	E:SetKeys({ ValueCurveKey.new(0, 1) })
	task.wait()
	return table.concat(Log, ",")
end)

-- CompositeValueCurve
local CV = Instance.new("CompositeValueCurve")
print("composite type", CV.CurveType)
Try("components", function() local List = CV:GetComponentCurves() return type(List), #List, next(List) end)
print("children", #CV:GetChildren())
Try("composite value", function() return CV:GetValueAtTime(0) end)
for _, Kind in Enum.CompositeValueCurveType:GetEnumItems() do
	local X = Instance.new("CompositeValueCurve")
	X.CurveType = Kind
	local Curve = Instance.new("FloatCurve")
	Curve.Name = "X"
	Curve:InsertKey(FloatCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear))
	Curve.Parent = X
	Try("composite " .. Kind.Name, function() return #X:GetComponentCurves(), X:GetValueAtTime(0.5), #X:GetChildren() end)
end
Try("composite time string", function() return CV:GetValueAtTime("x") end)
Try("composite time number string", function() return CV:GetValueAtTime("2") end)
Try("composite time table", function() return CV:GetValueAtTime({}) end)
Try("composite time nil", function() return CV:GetValueAtTime() end)
Try("composite type write", function() CV.CurveType = Enum.CompositeValueCurveType.Vector3 return CV.CurveType end)
