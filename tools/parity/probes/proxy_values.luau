-- The value types a client makes that ree builds as proxies (RaycastParams, OverlapParams, RaycastResult, Axes, Faces,
-- CatalogSearchParams, Path2DControlPoint, PathWaypoint, DockWidgetPluginGuiInfo, SecurityCapabilities, ValueCurveKey):
-- their members, the errors for keys they have no member for, tostring, equality, constructors, and the names the VM's
-- own errors give them.
local function try(Function, ...)
	return P.try(Function, ...)
end
local function joined(List)
	local Texts = table.create(#List)
	for Index, Value in List do
		Texts[Index] = tostring(Value)
	end
	return table.concat(Texts, " | ")
end

local Holder = Instance.new("Model")
Holder.Name = "Holder"
local Box = Instance.new("Part")
Box.Name = "Probe Box"
Box.Anchored = true
Box.Size = Vector3.new(4, 1, 4)
Box.Position = Vector3.new(1 / 3, 900, -2.25)
Box.Parent = Holder
Holder.Parent = workspace
local Tilted = Instance.new("Part")
Tilted.Anchored = true
Tilted.Size = Vector3.new(4, 1, 4)
Tilted.CFrame = CFrame.new(50, 900, 0) * CFrame.Angles(0, 0, math.rad(30))
Tilted.Material = Enum.Material.Neon
Tilted.Parent = workspace
local Plain = Instance.new("Part")
Plain.Anchored = true
Plain.Size = Vector3.new(4, 1, 4)
Plain.Position = Vector3.new(0, 900, 100)
Plain.Parent = workspace
task.wait()
local Params = RaycastParams.new()
Params.FilterType = Enum.RaycastFilterType.Include
Params.FilterDescendantsInstances = { Holder, Tilted, Plain }
local function cast(From, Direction)
	return workspace:Raycast(From, Direction, Params)
end
local Down = Vector3.new(0, -100, 0)

-- RaycastResult: tostring (the part's Name, then its vectors with 9 significant digits and the material's name)
print("result-tostring " .. joined({
	tostring(cast(Vector3.new(1 / 3, 950, -2.25), Down)),
	tostring(cast(Vector3.new(50, 950, 0), Down)),
	tostring(cast(Vector3.new(0, 950, 100), Down)),
	tostring(cast(Vector3.new(0.1, 950, 100.7), Down)),
	tostring(cast(Vector3.new(1e-7, 950, 100), Down)),
	tostring(cast(Vector3.new(-20, 900.25, 100), Vector3.new(100, 0, 0))),
	tostring(cast(Vector3.new(0.25, 850, 99.5), Vector3.new(0, 100, 0))),
}))
local Hit = cast(Vector3.new(1 / 3, 950, -2.25), Down)
print("result-name " .. try(function()
	Box.Name = "Renamed"
	local Renamed = tostring(Hit)
	Box.Name = ""
	local Unnamed = tostring(Hit)
	Box.Name = "Probe Box"
	return Renamed, Unnamed
end))
print("result-members " .. try(function()
	return Hit.Instance, P.f32s(Hit.Position.X, Hit.Position.Y, Hit.Position.Z), P.f32s(Hit.Normal.X, Hit.Normal.Y, Hit.Normal.Z), Hit.Distance, Hit.Material
end))
print("result-eq " .. try(function()
	local Again = cast(Vector3.new(1 / 3, 950, -2.25), Down)
	local Other = cast(Vector3.new(1, 950, -2.25), Down)
	local Elsewhere = cast(Vector3.new(0, 950, 100), Down)
	return Hit == Again, Hit == Other, Hit == Elsewhere, Hit == Hit, Hit == Params
end))
print("result-writes " .. joined({
	try(function() Hit.Instance = Box end),
	try(function() Hit.Position = Vector3.zero end),
	try(function() Hit.Normal = Vector3.zero end),
	try(function() Hit.Distance = 1 end),
	try(function() Hit.Material = Enum.Material.Wood end),
}))

-- Keys a type has no member for: a read, a lowercased member, a number key (twice), a boolean key, a method call; then the
-- writes of an unknown name, a number key and a boolean key
local Values = {
	{ "RaycastParams", function() return RaycastParams.new() end, "filterType" },
	{ "OverlapParams", function() return OverlapParams.new() end, "maxParts" },
	{ "RaycastResult", function() return Hit end, "instance" },
	{ "Axes", function() return Axes.new(Enum.Axis.X) end, "x" },
	{ "Faces", function() return Faces.new(Enum.NormalId.Top) end, "top" },
	{ "CatalogSearchParams", function() return CatalogSearchParams.new() end, "searchKeyword" },
	{ "Path2DControlPoint", function() return Path2DControlPoint.new() end, "position" },
	{ "PathWaypoint", function() return PathWaypoint.new(Vector3.zero, Enum.PathWaypointAction.Walk) end, "label" },
	{ "DockWidgetPluginGuiInfo", function() return DockWidgetPluginGuiInfo.new() end, "InitialDockState" },
	{ "SecurityCapabilities", function() return SecurityCapabilities.new() end, "contains" },
	{ "ValueCurveKey", function() return ValueCurveKey.new(0, 1) end, "time" },
}
for _, Case in Values do
	local Made, Value = pcall(Case[2])
	if not Made then
		print("unknown " .. Case[1] .. " E(" .. P.err(Value) .. ")")
		continue
	end
	print("unknown " .. Case[1] .. " " .. joined({
		try(function() return Value.Nope end),
		try(function() return Value[Case[3]] end),
		try(function() return Value[1] end),
		try(function() return Value[1.5] end),
		try(function() return Value[true] end),
		try(function() return Value:Nope() end),
	}))
	print("unknown-write " .. Case[1] .. " " .. joined({
		try(function() Value.Nope = 1 end),
		try(function() Value[1] = 1 end),
		try(function() Value[true] = 1 end),
	}))
end

-- Writes of members a script may not assign, and of a method
print("read-only " .. joined({
	try(function() Axes.new(Enum.Axis.X).X = false end),
	try(function() Axes.new(Enum.Axis.X).Top = true end),
	try(function() Faces.new(Enum.NormalId.Top).Front = true end),
	try(function() PathWaypoint.new().Position = Vector3.one end),
	try(function() PathWaypoint.new().Action = Enum.PathWaypointAction.Jump end),
	try(function() PathWaypoint.new().Label = "x" end),
	try(function() RaycastParams.new().AddToFilter = 1 end),
	try(function() SecurityCapabilities.new().Contains = 1 end),
}))

-- Members a script reads and writes, with their defaults
print("members RaycastParams " .. try(function()
	local Value = RaycastParams.new()
	local Written = RaycastParams.new()
	Written.Margin = 2
	Written.BruteForceAllSlow = true
	return Value.FilterType, Value.IgnoreWater, Value.CollisionGroup, Value.RespectCanCollide, Value.BruteForceAllSlow, Value.Margin, Value.SizeGradient, Value.ExcludeInstances, Value.IncludeInstances, #Value.FilterDescendantsInstances, type(Value.AddToFilter), Written.Margin, Written.BruteForceAllSlow
end))
print("members OverlapParams " .. try(function()
	local Value = OverlapParams.new()
	return Value.FilterType, Value.MaxParts, Value.Tolerance, Value.CollisionGroup, Value.RespectCanCollide, Value.BruteForceAllSlow, Value.ExcludeInstances, Value.IncludeInstances, #Value.FilterDescendantsInstances, type(Value.AddToFilter)
end))
print("members CatalogSearchParams " .. try(function()
	local Value = CatalogSearchParams.new()
	local Before = { Value.SearchKeyword, Value.MinPrice, Value.MaxPrice, Value.SortType, Value.SortAggregation, Value.CategoryFilter, Value.SalesTypeFilter, #Value.BundleTypes, #Value.AssetTypes, Value.IncludeOffSale, Value.CreatorName, Value.CreatorType, Value.CreatorId, Value.Limit }
	Value.SearchKeyword = "hat"
	Value.Limit = 10
	return joined(Before) .. " / " .. Value.SearchKeyword .. " " .. Value.Limit
end))
print("members PathWaypoint " .. try(function()
	local Value = PathWaypoint.new(Vector3.new(1, 2, 3), Enum.PathWaypointAction.Jump, "lbl")
	local Default = PathWaypoint.new()
	return Value.Position, Value.Action, Value.Label, Default.Position, Default.Action, Default.Label
end))

-- The names the VM's own errors give each type
for _, Case in Values do
	local Made, Value = pcall(Case[2])
	if not Made then
		continue
	end
	print("vm-names " .. Case[1] .. " " .. joined({
		try(function() return #Value end),
		try(function() return Value + 1 end),
		try(function() return 1 + Value end),
		try(function() return -Value end),
		try(function() return Value < Value end),
		try(function() return Value .. "s" end),
		try(function() return Value() end),
		try(function() for _ in Value do end end),
		try(function() return math.floor(Value) end),
		try(function() return string.len(Value) end),
		try(function() return pairs(Value) end),
	}))
end

-- DockWidgetPluginGuiInfo: tostring (enabled flags as 1 / 0, sizes as 32-bit integers), equality, and its arguments
local S = Enum.InitialDockState
local function dock(...)
	return tostring(DockWidgetPluginGuiInfo.new(...))
end
print("dock-tostring " .. joined({
	try(dock),
	try(dock, S.Float, true, false, 200.5, 300, 150, 150),
	try(dock, S.Top, false, true, -5.5, 7.9, 0.5, -0.5),
	try(dock, S.Bottom, false, false, 3e9, -3e9, 1 / 0, 0 / 0),
	try(dock, S.Top, false, true, 2147483647.5, 2147483648, -2147483648.5, -2147483649),
	try(dock, S.Bottom, false, false, 1, 2, 3, 4, 5),
	try(dock, S.Left, true),
	try(dock, S.Left, true, false, 10),
}))
print("dock-state " .. joined({ try(dock, 1), try(dock, 4.9), try(dock, 2.5), try(dock, -0.5), try(dock, 3.999), try(dock, 5), try(dock, -1), try(dock, 0 / 0), try(dock, 1e10), try(dock, "Top"), try(dock, "2"), try(dock, true), try(dock, nil), try(dock, Enum.Material.Plastic) }))
print("dock-arguments " .. joined({
	try(dock, S.Top, nil),
	try(dock, S.Top, 1),
	try(dock, S.Top, "yes"),
	try(dock, S.Top, true, nil),
	try(dock, S.Top, true, "x"),
	try(dock, S.Top, true, true, nil),
	try(dock, S.Top, true, true, "12"),
	try(dock, S.Top, true, true, true),
	try(dock, S.Top, true, true, 1, "x"),
	try(dock, S.Top, true, true, 1, 2, "x"),
	try(dock, S.Top, true, true, 1, 2, 3, "x"),
	try(dock, S.Top, true, true, 1, 2, 3, nil),
}))
print("dock-eq " .. try(function()
	return DockWidgetPluginGuiInfo.new() == DockWidgetPluginGuiInfo.new(), DockWidgetPluginGuiInfo.new(S.Left) == DockWidgetPluginGuiInfo.new(S.Right),
		DockWidgetPluginGuiInfo.new(S.Left, true, false, 1, 2, 3, 4) == DockWidgetPluginGuiInfo.new(S.Left, true, false, 1, 2, 3, 4), DockWidgetPluginGuiInfo.new(1) == DockWidgetPluginGuiInfo.new(S.Bottom),
		DockWidgetPluginGuiInfo.new() == RaycastParams.new(), typeof(DockWidgetPluginGuiInfo.new())
end))
print("dock-members " .. joined({
	try(function() return DockWidgetPluginGuiInfo.new().InitialEnabled end),
	try(function() return DockWidgetPluginGuiInfo.new().FloatingXSize end),
	try(function() return DockWidgetPluginGuiInfo.new().MinHeight end),
	try(function() DockWidgetPluginGuiInfo.new().FloatingXSize = 10 end),
	try(function() DockWidgetPluginGuiInfo.new().InitialDockState = S.Top end),
}))

-- SecurityCapabilities: the library, each capability alone (two print as another: one set holds the same capability), sets,
-- the methods (Add and Remove answer a new set), equality, and the errors
local SC = Enum.SecurityCapability
print("capabilities-library " .. try(function()
	return typeof(SecurityCapabilities), type(SecurityCapabilities.new), type(SecurityCapabilities.fromCurrent), SecurityCapabilities.Nope
end))
local Items = {}
for _, Item in SC:GetEnumItems() do
	table.insert(Items, Item.Name .. "=" .. try(function()
		local Value = SecurityCapabilities.new(Item)
		return tostring(Value), Value:Contains(Item)
	end))
end
print("capabilities-items " .. table.concat(Items, ", "))
print("capabilities-all " .. try(function()
	local All = {}
	for _, Item in SC:GetEnumItems() do
		table.insert(All, Item)
	end
	return tostring(SecurityCapabilities.new(table.unpack(All)))
end))
print("capabilities-sets " .. joined({
	try(function() return tostring(SecurityCapabilities.new()) end),
	try(function() return tostring(SecurityCapabilities.new(SC.RunClientScript, SC.Network, SC.Basic)) end),
	try(function() return tostring(SecurityCapabilities.new(SC.Network, SC.RunClientScript, SC.Network)) end),
	try(function() return tostring(SecurityCapabilities.fromCurrent()) end),
	try(function() return tostring(SecurityCapabilities.fromCurrent(SC.UI)) end),
	try(function() return type(SecurityCapabilities.new()), typeof(SecurityCapabilities.new()), getmetatable(SecurityCapabilities.new()) end),
}))
print("capabilities-methods " .. joined({
	try(function()
		local Value = SecurityCapabilities.new(SC.Network, SC.UI)
		return Value:Contains(SC.Network), Value:Contains(SC.Physics), Value:Contains(SC.Network, SC.UI), Value:Contains(SC.Network, SC.Physics), Value:Contains()
	end),
	try(function()
		return SecurityCapabilities.new(SC.Unassigned):Contains(SC.AssetRequire), SecurityCapabilities.new(SC.AssetRequire):Contains(SC.LoadUnownedAsset), SecurityCapabilities.new(SC.Avatar):Contains(SC.AvatarAppearance)
	end),
	try(function()
		local Value = SecurityCapabilities.new(SC.Network)
		local Added = Value:Add(SC.UI)
		return tostring(Value), tostring(Added), typeof(Added), rawequal(Value, Added)
	end),
	try(function() return tostring(SecurityCapabilities.new():Add(SC.UI, SC.Network, SC.UI)) end),
	try(function()
		local Value = SecurityCapabilities.new(SC.UI)
		local Same = Value:Add()
		return tostring(Same), rawequal(Value, Same), Value == Same
	end),
	try(function()
		local Value = SecurityCapabilities.new(SC.Network, SC.UI)
		local Removed = Value:Remove(SC.UI)
		return tostring(Value), tostring(Removed), tostring(Value:Remove(SC.Physics)), tostring(Value:Remove())
	end),
	try(function() local Value = SecurityCapabilities.new(SC.UI) return Value.Contains(Value, SC.UI) end),
}))
print("capabilities-eq " .. try(function()
	return SecurityCapabilities.new(SC.Network, SC.UI) == SecurityCapabilities.new(SC.UI, SC.Network), SecurityCapabilities.new(SC.UI) == SecurityCapabilities.new(SC.Network),
		SecurityCapabilities.new() == SecurityCapabilities.new(), SecurityCapabilities.new() == RaycastParams.new(), SecurityCapabilities.fromCurrent() == SecurityCapabilities.fromCurrent(),
		rawequal(SecurityCapabilities.fromCurrent(), SecurityCapabilities.fromCurrent())
end))
print("capabilities-errors " .. joined({
	try(function() return SecurityCapabilities.new("Network") end),
	try(function() return SecurityCapabilities.new(10) end),
	try(function() return SecurityCapabilities.new(Enum.Material.Plastic) end),
	try(function() return SecurityCapabilities.new(nil) end),
	try(function() return SecurityCapabilities.new(SC.UI, nil, SC.Network) end),
	try(function() return SecurityCapabilities.new(SecurityCapabilities.new(SC.UI)) end),
	try(function() return SecurityCapabilities.new(SC.Network):Contains("Network") end),
	try(function() return SecurityCapabilities.new(SC.Network):Contains(nil) end),
	try(function() return SecurityCapabilities.new():Add("UI") end),
	try(function() return SecurityCapabilities.new():Remove(Enum.Material.Plastic) end),
	try(function() return SecurityCapabilities.new(SC.Network).Contains(nil, SC.Network) end),
	try(function() return SecurityCapabilities.new(SC.Network).Contains(RaycastParams.new(), SC.Network) end),
	try(function() return SecurityCapabilities.new().Add(5, SC.UI) end),
	try(function() return SecurityCapabilities.new().Remove(nil, SC.UI) end),
}))

-- ValueCurveKey: its fields (Time a float, Interpolation Constant unless given, tangents nil until set), tostring, the
-- constructor's arguments, the writes, equality, and a table value (copied as a value an event carries)
local KI = Enum.KeyInterpolationMode
local function key(...)
	local Value = ValueCurveKey.new(...)
	return typeof(Value), tostring(Value), Value.Time, Value.Value, Value.Interpolation, Value.LeftTangent, Value.RightTangent
end
print("key-library " .. try(function() return typeof(ValueCurveKey), type(ValueCurveKey.new), ValueCurveKey.Nope end))
print("key-new " .. joined({
	try(key, 0, 1, KI.Linear),
	try(key, 1 / 3, 2.5, KI.Cubic),
	try(key, 0.5, Vector3.new(1, 2, 3), KI.Constant),
	try(key, 0.5, CFrame.new(1, 2, 3), KI.Linear),
	try(key, 0.5, true, KI.Constant),
	try(key, 0.5, "hi"),
	try(key, 0.5, UDim2.new(1, 2, 3, 4)),
	try(key, 0, Box),
	try(key, 0, Enum.Material.Plastic),
	try(key, 0, 0 / 0),
	try(key, 0, false),
}))
print("key-time " .. joined({ try(key, "2", 1), try(key, "x", 1), try(key, nil, 1), try(key, true, 1), try(key, 1 / 0, 1), try(key, -2.5, 1), try(key, "0x10", 1), try(key, 1e300, 1), try(key, -1e40, 1), try(key, -0, 1) }))
print("key-errors " .. joined({
	try(key),
	try(key, 0.5),
	try(key, 0, 1, KI.Linear, "extra"),
	try(key, 0, nil, KI.Linear),
	try(key, 0, 1, nil),
	try(key, 0, 1, "Linear"),
	try(key, 0, 1, 1),
	try(key, 0, 1, Enum.Material.Plastic),
	try(function() local Cycle = {} Cycle.Self = Cycle return ValueCurveKey.new(0, Cycle) end),
}))
print("key-table " .. try(function()
	local Source = { 1, 2, Name = { Deep = 3 } }
	local Value = ValueCurveKey.new(0, Source)
	local First, Second = Value.Value, Value.Value
	Source[1] = 10
	local Keyed = ValueCurveKey.new(0, { [Box] = 1 }).Value
	local Meta = ValueCurveKey.new(0, setmetatable({ Name = 1 }, { __index = function() return "meta" end })).Value
	return rawequal(First, Second), First[1], #First, First.Name, rawequal(Value.Value, Source), next(Keyed), Meta.Other, getmetatable(Meta)
end))
local function written(Mode, Field, Value)
	return try(function()
		local Key = ValueCurveKey.new(5, 1, Mode)
		Key[Field] = Value
		return Key[Field]
	end)
end
print("key-time-writes " .. joined({ written(KI.Linear, "Time", 3), written(KI.Linear, "Time", "4"), written(KI.Linear, "Time", " 7 "), written(KI.Linear, "Time", 1 / 3), written(KI.Linear, "Time", "x"), written(KI.Linear, "Time", nil), written(KI.Linear, "Time", true), written(KI.Linear, "Time", Vector3.one) }))
print("key-value-writes " .. joined({ written(KI.Linear, "Value", Vector3.new(1, 2, 3)), written(KI.Linear, "Value", nil), written(KI.Linear, "Value", "s") }))
print("key-interpolation-writes " .. joined({ written(KI.Constant, "Interpolation", KI.Cubic), written(KI.Cubic, "Interpolation", nil), written(KI.Cubic, "Interpolation", "Constant"), written(KI.Cubic, "Interpolation", 0), written(KI.Cubic, "Interpolation", Enum.Material.Plastic) }))
for _, Mode in { KI.Constant, KI.Linear, KI.Cubic } do
	print("key-tangent-writes " .. Mode.Name .. " " .. joined({
		written(Mode, "LeftTangent", 2.5), written(Mode, "LeftTangent", 1 / 3), written(Mode, "LeftTangent", "2"), written(Mode, "LeftTangent", "x"), written(Mode, "LeftTangent", true), written(Mode, "LeftTangent", nil),
		written(Mode, "RightTangent", 2.5), written(Mode, "RightTangent", "x"), written(Mode, "RightTangent", nil),
	}))
end
print("key-tangents " .. joined({
	try(function()
		local Key = ValueCurveKey.new(0, 1, KI.Cubic)
		Key.LeftTangent = 2
		Key.LeftTangent = nil
		Key.RightTangent = 3
		Key.RightTangent = nil
		return Key.LeftTangent, Key.RightTangent
	end),
	try(function()
		local Key = ValueCurveKey.new(0, 1, KI.Cubic)
		Key.RightTangent = 3
		Key.LeftTangent = 4
		Key.Interpolation = KI.Linear
		local Linear = { Key.RightTangent, Key.LeftTangent }
		Key.Interpolation = KI.Cubic
		return Linear[1], Linear[2], Key.RightTangent, Key.LeftTangent, tostring(Key)
	end),
}))
print("key-eq " .. joined({
	try(function() return ValueCurveKey.new(0, 1, KI.Linear) == ValueCurveKey.new(0, 1, KI.Linear), ValueCurveKey.new(0, 1) == ValueCurveKey.new(0, "1"), ValueCurveKey.new(0, 1) == ValueCurveKey.new(0, 1, KI.Linear), ValueCurveKey.new(0, 1) == ValueCurveKey.new(0, 1, KI.Constant), ValueCurveKey.new(0, 1) == ValueCurveKey.new(1, 1) end),
	try(function() local T = {} return ValueCurveKey.new(0, T) == ValueCurveKey.new(0, T), ValueCurveKey.new(0, Vector3.one) == ValueCurveKey.new(0, Vector3.one), ValueCurveKey.new(0, print) == ValueCurveKey.new(0, print), ValueCurveKey.new(0, Box) == ValueCurveKey.new(0, Box) end),
	try(function() return ValueCurveKey.new(0, 0 / 0) == ValueCurveKey.new(0, 0 / 0), ValueCurveKey.new(0 / 0, 1) == ValueCurveKey.new(0 / 0, 1), ValueCurveKey.new(0, 0) == ValueCurveKey.new(-0, -0), ValueCurveKey.new(0, 1) == FloatCurveKey.new(0, 1, KI.Constant) end),
	try(function() local Key = ValueCurveKey.new(0, {}) return Key == Key end),
	try(function()
		local A, B = ValueCurveKey.new(0, 1, KI.Cubic), ValueCurveKey.new(0, 1, KI.Cubic)
		A.LeftTangent = 1
		local Before = A == B
		B.LeftTangent = 1
		local After = A == B
		A.RightTangent, B.RightTangent = "x", "x"
		return Before, After, A == B
	end),
	try(function()
		local A, B = ValueCurveKey.new(5, 1), ValueCurveKey.new(5, 1)
		A.Time, B.Time = "x", "x"
		return A == B, A == ValueCurveKey.new(0, 1)
	end),
}))
print("globals " .. typeof(SecurityCapabilities) .. " " .. typeof(ValueCurveKey) .. " " .. typeof(DockWidgetPluginGuiInfo))
