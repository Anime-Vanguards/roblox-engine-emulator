-- typeof, type and tostring of every kind of value; metamethods; coroutines and threads.
local Part = Instance.new("Part")
local Signal = Part.Changed
local Connection = Signal:Connect(function() end)
local Values = {
	{ "nil", function() return nil end }, { "bool", function() return true end }, { "number", function() return 1.5 end }, { "string", function() return "s" end }, { "table", function() return {} end }, { "function", function() return print end },
	{ "thread", function() return coroutine.create(print) end }, { "userdata", function() return newproxy() end }, { "buffer", function() return buffer.create(1) end },
	{ "vector", function() return vector.create(1, 2, 3) end }, { "Instance", function() return Part end }, { "Workspace", function() return workspace end }, { "DataModel", function() return game:GetService("Workspace").Parent == game end },
	{ "Vector3", function() return Vector3.new(1, 2, 3) end }, { "Vector2", function() return Vector2.new(1, 2) end }, { "Vector3int16", function() return Vector3int16.new(1, 2, 3) end },
	{ "Vector2int16", function() return Vector2int16.new(1, 2) end }, { "CFrame", function() return CFrame.new(1, 2, 3) end }, { "Color3", function() return Color3.new(1, 0.5, 0) end },
	{ "BrickColor", function() return BrickColor.new("Bright red") end }, { "UDim", function() return UDim.new(0.5, 3) end }, { "UDim2", function() return UDim2.new(1, 2, 3, 4) end },
	{ "Rect", function() return Rect.new(1, 2, 3, 4) end }, { "NumberRange", function() return NumberRange.new(1, 2) end }, { "NumberSequence", function() return NumberSequence.new(0, 1) end },
	{ "NumberSequenceKeypoint", function() return NumberSequenceKeypoint.new(0, 1, 0.5) end }, { "ColorSequence", function() return ColorSequence.new(Color3.new(1, 0, 0), Color3.new(0, 0, 1)) end },
	{ "ColorSequenceKeypoint", function() return ColorSequenceKeypoint.new(0, Color3.new(1, 0, 0)) end }, { "Ray", function() return Ray.new(Vector3.zero, Vector3.xAxis) end },
	{ "Region3", function() return Region3.new(Vector3.zero, Vector3.one) end }, { "Region3int16", function() return Region3int16.new(Vector3int16.new(), Vector3int16.new(1, 1, 1)) end },
	{ "TweenInfo", function() return TweenInfo.new(1) end }, { "Axes", function() return Axes.new(Enum.Axis.X) end }, { "Faces", function() return Faces.new(Enum.NormalId.Top) end },
	{ "PhysicalProperties", function() return PhysicalProperties.new(1, 0.3, 0.5) end }, { "Enum", function() return Enum.Material end }, { "Enums", function() return Enum end },
	{ "EnumItem", function() return Enum.Material.Plastic end }, { "RBXScriptSignal", function() return Signal end }, { "RBXScriptConnection", function() return Connection end },
	{ "Random", function() return Random.new(1) end }, { "DateTime", function() return DateTime.fromUnixTimestamp(0) end }, { "Font", function() return Font.new("rbxasset://fonts/families/Arial.json") end },
	{ "Content", function() return Content.fromUri("rbxassetid://1") end }, { "PathWaypoint", function() return PathWaypoint.new(Vector3.zero, Enum.PathWaypointAction.Walk) end },
	{ "CatalogSearchParams", function() return CatalogSearchParams.new() end }, { "RaycastParams", function() return RaycastParams.new() end }, { "OverlapParams", function() return OverlapParams.new() end },
	{ "SharedTable", function() return SharedTable.new() end }, { "Path2DControlPoint", function() return Path2DControlPoint.new() end }, { "FloatCurveKey", function() return FloatCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear) end },
	{ "RotationCurveKey", function() return RotationCurveKey.new(0, CFrame.new(), Enum.KeyInterpolationMode.Linear) end }, { "SecurityCapabilities", function() return SecurityCapabilities.new() end },
}
for _, Pair in Values do
	local Made, Value = pcall(Pair[2])
	if not Made then
		print("value " .. Pair[1] .. " E(" .. P.err(Value) .. ")")
		continue
	end
	local Text = string.gsub(tostring(Value), "0x%x+", "ADDR")
	print("value " .. Pair[1] .. " typeof=" .. typeof(Value) .. " type=" .. type(Value) .. " tostring=" .. Text)
end
Connection:Disconnect()
print("typeof-metatable " .. typeof(setmetatable({}, { __type = "Custom" })) .. " " .. typeof(newproxy(true)))
print("typeof-noarg " .. P.try(typeof) .. " | " .. P.try(type))
print("tostring-more " .. tostring(-0) .. " " .. tostring(1e300 * 10) .. " " .. tostring(CFrame.Angles(0, math.pi / 2, 0)) .. " | " .. tostring(Color3.fromRGB(255, 128, 0)) .. " | " .. tostring(Vector3.new(0.1, -0, 1e20)) .. " | " .. tostring(UDim2.fromScale(0.5, 0.25)) .. " | " .. tostring(Enum.KeyCode.A) .. " | " .. tostring(BrickColor.random and "has-random"))
print("enum " .. Enum.Material.Plastic.Name .. " " .. Enum.Material.Plastic.Value .. " " .. tostring(Enum.Material.Plastic.EnumType) .. " " .. tostring(Enum.Material.Plastic == Enum.Material.Plastic) .. " " .. tostring(Enum.Material:FromName("Plastic")) .. " " .. tostring(Enum.Material:FromValue(256)) .. " " .. tostring(Enum.Material:FromName("Nope")))
print("enum-items " .. #Enum.SortOrder:GetEnumItems() .. " " .. (function()
	local Names = {}
	for _, Item in Enum.EasingDirection:GetEnumItems() do
		table.insert(Names, Item.Name .. "=" .. Item.Value)
	end
	return table.concat(Names, ",")
end)())
print("enum-compare " .. tostring(Enum.Material.Plastic == 256) .. " " .. P.try(function()
	return Enum.Material.Plastic < Enum.Material.Wood
end))
print("enum-assign-forms " .. (function()
	local Target = Instance.new("Part")
	local Results = {}
	for _, Value in { "Wood", 512, Enum.Material.Slate, "wood", 512.5, "512" } do
		local Ok, Message = pcall(function()
			Target.Material = Value
		end)
		table.insert(Results, if Ok then tostring(Target.Material) else "E(" .. P.err(Message) .. ")")
	end
	return table.concat(Results, " ")
end)())

-- metamethods
local Log = {}
local Meta = {}
Meta.__index = function(_, Key)
	table.insert(Log, "index " .. tostring(Key))
	return Key
end
Meta.__newindex = function(Table, Key, Value)
	table.insert(Log, "newindex " .. tostring(Key))
	rawset(Table, Key, Value)
end
Meta.__call = function(_, ...)
	return select("#", ...)
end
Meta.__concat = function(A, B)
	return "concat(" .. type(A) .. "," .. type(B) .. ")"
end
Meta.__unm = function()
	return "unm"
end
Meta.__len = function()
	return 99
end
Meta.__eq = function()
	table.insert(Log, "eq")
	return true
end
Meta.__lt = function()
	return true
end
Meta.__le = function()
	return false
end
Meta.__add = function(A, B)
	return "add(" .. type(A) .. "," .. type(B) .. ")"
end
Meta.__mod = function()
	return "mod"
end
Meta.__idiv = function()
	return "idiv"
end
Meta.__pow = function()
	return "pow"
end
Meta.__tostring = function()
	return "TOSTRING"
end
Meta.__iter = function(Table)
	return next, { "iterated" }
end
local A = setmetatable({}, Meta)
local B = setmetatable({}, Meta)
print("meta " .. tostring(A.foo) .. " " .. tostring(A(1, nil, 3)) .. " " .. (A .. "x") .. " " .. ("x" .. A) .. " " .. tostring(-A) .. " " .. #A .. " " .. tostring(A == B) .. " " .. tostring(A == A) .. " " .. tostring(A < B) .. " " .. tostring(A <= B) .. " " .. tostring(A > B) .. " " .. (A + 1) .. " " .. (1 + A) .. " " .. (A % 2) .. " " .. (A // 2) .. " " .. (A ^ 2) .. " " .. tostring(A))
A.bar = 1
A.bar = 2
for Key, Value in A do
	table.insert(Log, "iter " .. tostring(Key) .. "=" .. tostring(Value))
end
print("meta-log " .. table.concat(Log, ","))
print("meta-eq-different " .. tostring(setmetatable({}, { __eq = function() return true end }) == setmetatable({}, { __eq = function() return true end })))
print("meta-tostring-nonstring " .. P.try(tostring, setmetatable({}, { __tostring = function() return 1 end })))
print("meta-tostring-nil " .. P.try(tostring, setmetatable({}, { __tostring = function() return nil end })))
print("meta-name " .. string.gsub(tostring(setmetatable({}, { __name = "Named" })), "0x%x+", "ADDR"))
local Locked = setmetatable({}, { __metatable = "locked!" })
print("metatable-locked " .. tostring(getmetatable(Locked)) .. " " .. P.try(setmetatable, Locked, {}))
print("string-metatable " .. type(getmetatable("")) .. " " .. tostring(getmetatable("").__index == string))
print("instance-metatable " .. tostring(getmetatable(Part)) .. " " .. P.try(setmetatable, Part, {}))
print("rawequal " .. tostring(rawequal(A, B)) .. " rawlen " .. rawlen(A) .. " rawget " .. tostring(rawget(A, "foo")))
print("index-chain " .. tostring(setmetatable({}, { __index = setmetatable({}, { __index = { deep = "found" } }) }).deep))
print("index-loop " .. P.try(function()
	local Loop = {}
	setmetatable(Loop, { __index = Loop })
	return Loop.missing
end))
print("meta-call-nonfunction " .. P.try(function()
	return setmetatable({}, { __call = 5 })()
end))
print("weak-mode " .. P.try(function()
	local Weak = setmetatable({}, { __mode = "k" })
	Weak[{}] = 1
	return getmetatable(Weak).__mode
end))

-- coroutines
local Statuses = {}
local Outer
Outer = coroutine.create(function()
	table.insert(Statuses, "inside:" .. coroutine.status(Outer))
	local Inner = coroutine.create(function()
		table.insert(Statuses, "outer-from-inner:" .. coroutine.status(Outer))
		coroutine.yield()
	end)
	coroutine.resume(Inner)
	table.insert(Statuses, "inner-suspended:" .. coroutine.status(Inner))
	coroutine.yield("y1")
	return "done"
end)
table.insert(Statuses, "new:" .. coroutine.status(Outer))
local Ok, First = coroutine.resume(Outer)
table.insert(Statuses, "after-yield:" .. coroutine.status(Outer) .. ":" .. tostring(First))
local Ok2, Second = coroutine.resume(Outer)
table.insert(Statuses, "after-return:" .. coroutine.status(Outer) .. ":" .. tostring(Second))
print("coroutine-status " .. table.concat(Statuses, ","))
print("coroutine-running " .. typeof(coroutine.running()) .. " " .. select("#", coroutine.running()) .. " yieldable " .. tostring(coroutine.isyieldable()))
print("coroutine-close " .. P.try(function()
	local C = coroutine.create(function()
		coroutine.yield()
	end)
	coroutine.resume(C)
	local Closed = table.pack(coroutine.close(C))
	return tostring(Closed[1]) .. " " .. coroutine.status(C)
end) .. " | " .. P.try(function()
	local C = coroutine.create(function()
		error("boom")
	end)
	coroutine.resume(C)
	local Closed = table.pack(coroutine.close(C))
	return tostring(Closed[1]) .. " " .. P.err(Closed[2]) .. " " .. coroutine.status(C)
end) .. " | " .. P.try(coroutine.close, coroutine.running()))
print("coroutine-wrap-yield-across-pcall " .. P.try(function()
	local W = coroutine.wrap(function()
		local Ok, Value = pcall(function()
			return coroutine.yield("from-pcall")
		end)
		return "resumed " .. tostring(Ok) .. " " .. tostring(Value)
	end)
	return W() .. " | " .. W("back")
end))
print("coroutine-resume-self " .. P.try(function()
	return coroutine.resume(coroutine.running())
end))
print("coroutine-resume-args " .. (function()
	local C = coroutine.create(function(...)
		local N = select("#", ...)
		local Back = table.pack(coroutine.yield(N))
		return Back.n
	end)
	local _, N = coroutine.resume(C, 1, nil, nil)
	local _, M = coroutine.resume(C, nil, nil)
	return N .. " " .. M
end)())
print("coroutine-wrap-error-position " .. P.try(function()
	local W = coroutine.wrap(function()
		error("wrapped failure")
	end)
	W()
end))
print("coroutine-error-object " .. P.try(function()
	local C = coroutine.create(function()
		error({ Code = 3 })
	end)
	local Ok, Value = coroutine.resume(C)
	return tostring(Ok) .. " " .. type(Value) .. " " .. tostring(Value.Code)
end))
print("task-in-coroutine " .. P.try(function()
	local Order = {}
	local C = coroutine.create(function()
		table.insert(Order, "before")
		task.wait()
		table.insert(Order, "after")
	end)
	local Ok, Message = coroutine.resume(C)
	table.insert(Order, "resume-returned " .. tostring(Ok) .. " " .. coroutine.status(C))
	task.wait()
	task.wait()
	table.insert(Order, "end " .. coroutine.status(C))
	return table.concat(Order, ",")
end))
print("resume-waiting-thread " .. P.try(function()
	local Order = {}
	local Waiting = task.spawn(function()
		local Got = task.wait(10)
		table.insert(Order, "woke " .. type(Got))
	end)
	local Ok, Message = coroutine.resume(Waiting, "early")
	table.insert(Order, "resumed " .. tostring(Ok) .. " " .. P.err(Message))
	task.wait()
	table.insert(Order, coroutine.status(Waiting))
	return table.concat(Order, ",")
end))
print("deep-coroutines " .. P.try(function()
	local Depth = 0
	local function Nest()
		Depth += 1
		if Depth < 300 then
			local C = coroutine.create(Nest)
			local Ok, Message = coroutine.resume(C)
			if not Ok then
				error(Message, 0)
			end
		end
	end
	Nest()
	return Depth
end))
