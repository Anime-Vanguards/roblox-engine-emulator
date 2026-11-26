-- The edge rules datatype_edges.luau leaves open: Euler angles at gimbal lock in every rotation order, lookAt's other
-- fallbacks and up vectors (and lookAlong's), fromMatrix's Z column near zero, Content.fromAssetId's rounding and
-- arguments, Font.fromEnum's refusals, every datatype's metatable, DateTime units past their 32-bit range, and
-- fromIsoDate's trailing text, partial fields and offsets.
local function CF(Value)
	return P.f32s(Value:GetComponents())
end

-- Euler angles: each rotation order's lock (its middle axis at a quarter turn, both ways) and exact quarter turns
local Orders = { "XYZ", "XZY", "YXZ", "YZX", "ZXY", "ZYX" }
local function Angles(C)
	local Parts = {}
	for _, Order in Orders do
		table.insert(Parts, Order .. " " .. P.f32s(C:ToEulerAngles(Enum.RotationOrder[Order])))
	end
	return table.concat(Parts, " | ")
end
local Half = math.pi / 2
for _, Order in Orders do
	local Middle = string.sub(Order, 2, 2)
	for _, Sign in { 1, -1 } do
		local Turn = { X = 0.3, Y = 0.3, Z = 0.3 }
		Turn[string.sub(Order, 1, 1)] = 0.3
		Turn[string.sub(Order, 3, 3)] = 0.2
		Turn[Middle] = Sign * Half
		local C = CFrame.fromEulerAngles(Turn.X, Turn.Y, Turn.Z, Enum.RotationOrder[Order])
		print("lock " .. Order .. " " .. Sign .. " " .. Angles(C))
	end
end
for _, Case in {
	{ "x+", CFrame.new(0, 0, 0, 1, 0, 0, 0, 0, -1, 0, 1, 0) },
	{ "x-", CFrame.new(0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -1, 0) },
	{ "y+", CFrame.new(0, 0, 0, 0, 0, 1, 0, 1, 0, -1, 0, 0) },
	{ "y-", CFrame.new(0, 0, 0, 0, 0, -1, 0, 1, 0, 1, 0, 0) },
	{ "z+", CFrame.new(0, 0, 0, 0, -1, 0, 1, 0, 0, 0, 0, 1) },
	{ "z-", CFrame.new(0, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1) },
	{ "nan", CFrame.new(0, 0, 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0) },
	{ "half-turn", CFrame.Angles(math.pi, 0, 0) },
} do
	print("quarter " .. Case[1] .. " " .. Angles(Case[2]))
end

-- lookAt and lookAlong: along the up vector for other axes, up vectors that are not unit vectors
local Eye = Vector3.new(1, 2, 3)
for _, Case in {
	{ "along-y", Vector3.yAxis, Vector3.yAxis },
	{ "along-minus-y", -Vector3.yAxis, Vector3.yAxis },
	{ "along-z", Vector3.zAxis, Vector3.zAxis },
	{ "along-minus-z", -Vector3.zAxis, Vector3.zAxis },
	{ "along-diagonal", Vector3.new(1, 1, 0), Vector3.new(1, 1, 0) },
	{ "along-diagonal-3", Vector3.new(1, 1, 1), Vector3.new(2, 2, 2) },
	{ "along-x-up-minus-x", Vector3.xAxis, Vector3.new(-3, 0, 0) },
	{ "up-long-z", Vector3.new(3, 4, 5), Vector3.new(0, 0, 7) },
	{ "up-short", Vector3.new(3, 4, 5), Vector3.new(0, 0.001, 0) },
	{ "up-tiny", Vector3.new(3, 4, 5), Vector3.new(0, 1e-20, 0) },
	{ "up-nan", Vector3.new(3, 4, 5), Vector3.new(0 / 0, 1, 0) },
	{ "near-up-z", Vector3.new(1e-6, 0, 1), Vector3.zAxis },
	{ "nearer-up-z", Vector3.new(1e-7, 0, 1), Vector3.zAxis },
} do
	print("lookat " .. Case[1] .. " " .. P.try(function()
		return CF(CFrame.lookAt(Eye, Eye + Case[2], Case[3]))
	end) .. " | along " .. P.try(function()
		return CF(CFrame.lookAlong(Eye, Case[2], Case[3]))
	end))
end

-- fromMatrix's Z column near zero and near a unit vector
for _, Case in {
	{ "cross-1e-8", Vector3.new(1e-4, 0, 0), Vector3.new(0, 1e-4, 0) },
	{ "cross-1e-7", Vector3.new(1e-3, 0, 0), Vector3.new(0, 1e-4, 0) },
	{ "cross-2e-7", Vector3.new(2e-3, 0, 0), Vector3.new(0, 1e-4, 0) },
	{ "cross-1e-6", Vector3.new(1e-3, 0, 0), Vector3.new(0, 1e-3, 0) },
	{ "cross-1e-12", Vector3.new(1e-6, 0, 0), Vector3.new(0, 1e-6, 0) },
	{ "cross-1.000005", Vector3.new(1, 0, 0), Vector3.new(0, 1.000005, 0) },
	{ "cross-0.999995", Vector3.new(1, 0, 0), Vector3.new(0, 0.999995, 0) },
	{ "cross-1.0001", Vector3.new(1, 0, 0), Vector3.new(0, 1.0001, 0) },
	{ "cross-skew-near-1", Vector3.new(0.6, 0.8, 0), Vector3.new(-0.8, 0.6000001, 0) },
	{ "nan", Vector3.new(0 / 0, 0, 0), Vector3.yAxis },
	{ "inf", Vector3.new(math.huge, 0, 0), Vector3.yAxis },
} do
	print("frommatrix " .. Case[1] .. " " .. P.try(function()
		return CF(CFrame.fromMatrix(Eye, Case[2], Case[3]))
	end))
end

-- Content.fromAssetId: rounding, large ids, other arguments; fromObject's arguments
for _, Id in { 0.5, 2.5, -0.5, -1.5, -2.5, 0.49999999999999994, 1e20, -1e20, 2 ^ 63, -2 ^ 63, 2 ^ 53 + 2, 1e-300, -0 } do
	print("content-assetid " .. P.num(Id) .. " " .. P.try(function()
		return tostring(Content.fromAssetId(Id).Uri)
	end))
end
for _, Case in { { "string", "5" }, { "word", "abc" }, { "nil", nil }, { "true", true }, { "table", {} }, { "vector", Vector3.one } } do
	print("content-assetid-arg " .. Case[1] .. " " .. P.try(function()
		return (string.gsub(tostring(Content.fromAssetId(Case[2]).Uri), "table: [%wx]+", "table: <address>"))
	end))
end
print("content-assetid-none " .. P.try(function()
	return tostring(Content.fromAssetId().Uri)
end))
for _, Case in { { "string", "x" }, { "true", true }, { "table", {} }, { "vector", Vector3.one }, { "part", Instance.new("Part") } } do
	print("content-object " .. Case[1] .. " " .. P.try(function()
		return tostring(Content.fromObject(Case[2]))
	end))
end
print("content-object-none " .. P.try(function()
	return tostring(Content.fromObject())
end))

-- Font.fromEnum's refusals
for _, Case in { { "nil", nil }, { "number", 5 }, { "string", "Arial" }, { "material", Enum.Material.Wood }, { "enum", Enum.Font } } do
	print("font-fromenum " .. Case[1] .. " " .. P.try(function()
		return Font.fromEnum(Case[2]).Family
	end))
end
print("font-fromenum-none " .. P.try(function()
	return Font.fromEnum().Family
end))

-- every datatype's metatable
for _, Case in {
	{ "Vector2", function() return Vector2.new() end }, { "Vector2int16", function() return Vector2int16.new() end },
	{ "Vector3int16", function() return Vector3int16.new() end }, { "UDim", function() return UDim.new() end },
	{ "UDim2", function() return UDim2.new() end }, { "Rect", function() return Rect.new() end },
	{ "Ray", function() return Ray.new(Vector3.zero, Vector3.one) end }, { "Region3", function() return Region3.new(Vector3.zero, Vector3.one) end },
	{ "Region3int16", function() return Region3int16.new(Vector3int16.new(), Vector3int16.new(1, 1, 1)) end },
	{ "BrickColor", function() return BrickColor.new() end }, { "NumberRange", function() return NumberRange.new(1) end },
	{ "NumberSequence", function() return NumberSequence.new(1) end }, { "NumberSequenceKeypoint", function() return NumberSequenceKeypoint.new(0, 1) end },
	{ "ColorSequence", function() return ColorSequence.new(Color3.new()) end },
	{ "ColorSequenceKeypoint", function() return ColorSequenceKeypoint.new(0, Color3.new()) end },
	{ "PhysicalProperties", function() return PhysicalProperties.new(1, 1, 1) end }, { "TweenInfo", function() return TweenInfo.new() end },
	{ "Faces", function() return Faces.new() end }, { "Axes", function() return Axes.new() end },
	{ "Font", function() return Font.new("rbxasset://fonts/families/Arial.json") end }, { "Content", function() return Content.none end },
	{ "DateTime", function() return DateTime.now() end }, { "Random", function() return Random.new(1) end },
	{ "RaycastParams", function() return RaycastParams.new() end }, { "OverlapParams", function() return OverlapParams.new() end },
	{ "CatalogSearchParams", function() return CatalogSearchParams.new() end },
	{ "Connection", function() return workspace.Changed:Connect(function() end) end }, { "PathWaypoint", function() return PathWaypoint.new() end },
	{ "FloatCurveKey", function() return FloatCurveKey.new(0, 1, Enum.KeyInterpolationMode.Linear) end },
	{ "RotationCurveKey", function() return RotationCurveKey.new(0, CFrame.new(), Enum.KeyInterpolationMode.Linear) end },
	{ "SharedTable", function() return SharedTable.new() end }, { "buffer", function() return buffer.create(1) end },
	{ "thread", function() return coroutine.create(print) end }, { "function", function() return print end },
	{ "number", function() return 1 end }, { "boolean", function() return true end }, { "table", function() return {} end },
} do
	local Made, Value = pcall(Case[2])
	local Ok, Meta = pcall(getmetatable, Value)
	print("metatable " .. Case[1] .. " " .. (if not Made then "made E(" .. P.err(Value) .. ")" elseif not Ok then "E(" .. P.err(Meta) .. ")"
		else type(Meta) .. " " .. (if type(Meta) == "string" then Meta else "-")))
end

-- DateTime units past their range: milliseconds as 32-bit microseconds, the other units
local function Rolled(...)
	local Fields = table.pack(...)
	return P.try(function()
		return P.num(DateTime.fromUniversalTime(table.unpack(Fields, 1, Fields.n)).UnixTimestampMillis)
	end)
end
for _, Millis in { 0.5, -0.5, 1.5, 0.0004, 0.0015, -0.0015, 2147483, 2147483.647, 2147483.648, 2147484, 3000000, -2147483.648, -2147483.649,
	-2147484, -3000000, 1e15, -1e15, 0 / 0, math.huge, -math.huge } do
	print("rolled-millis " .. P.num(Millis) .. " " .. Rolled(2000, 1, 1, 0, 0, 0, Millis))
end
for _, Second in { 2147483647, 2147483648, 3e9, -2147483648, -2147483649, -3e9, 0 / 0, math.huge, 1.5, -1.5 } do
	print("rolled-second " .. P.num(Second) .. " " .. Rolled(2000, 1, 1, 0, 0, Second, 0))
end
for _, Minute in { 35791394, 35791395, 2147483648, -2147483649, 0 / 0 } do
	print("rolled-minute " .. P.num(Minute) .. " " .. Rolled(2000, 1, 1, 0, Minute, 0, 0))
end
for _, Hour in { 596523, 596524, 2147483648, -2147483649, 0 / 0, math.huge } do
	print("rolled-hour " .. P.num(Hour) .. " " .. Rolled(2000, 1, 1, Hour, 0, 0, 0))
end
for _, Fields in { { 2000, 1, 0 }, { 2000, 1, 32 }, { 2000, 0, 1 }, { 2000, 13, 1 }, { 2000, 2, 30 }, { 2000.9, 1.9, 1.9 }, { 0 / 0, 1, 1 },
	{ 2000, 0 / 0, 1 }, { 2000, 1, 0 / 0 }, { math.huge, 1, 1 }, { -math.huge, 1, 1 }, { 10000, 1, 1 }, { 2147483648, 1, 1 }, { 2000, 2147483649, 1 } } do
	print("rolled-date " .. table.concat(Fields, ",") .. " " .. Rolled(table.unpack(Fields)))
end

-- fromIsoDate: trailing text after each part, partial fields, offsets
for _, Text in { "2020-02-29T12:34:56junk", "2020-02-29T12:34junk", "2020-02-29T12:34:56.5junk", "2020-02-29T12:34Zjunk", "2020-02-29Zjunk",
	"2020-02-29Z", "2020-02-29 ", "2020-02-29T", "2020-02-29T12", "2020-02-29T12:", "2020-02-29T12:3", "2020-02-29T12:34:", "2020-02-29T12:34:5Z",
	"2020-02-29T1:34:56Z", "2020-02-29T12:34:56.Z", "2020-02-29T12:34:56.5", "2020-02-29T12:34:56,5Z", "2020-02-29T12:34:56+02:00 ",
	"2020-02-29T12:34:56+02", "2020-02-29T12:34:56+02:0", "2020-02-29T12:34:56+02:", "2020-02-29T12:34:56+23:59", "2020-02-29T12:34:56-23:59",
	"2020-02-29T12:34:56-24:00", "2020-02-29T12:34:56+00:59", "2020-02-29T12:34:56+00:60", "2020-02-29T12:34:56+00:99",
	"2020-02-29T12:34:56+05:30Z", "2020-02-29T12:34+01:00", "2020-02-29T12:34:56Z+01:00", "2020-02-29T99:99:99Z", "2020-02-29T00:00:99.999Z",
	"2020-12-31T24:00:00Z", "2020-02-29T12:60Z", "2020-02-29T12:99Z", "2020-02-29\t12:34Z", "2020-02-29x12:34Z", "\t2020-02-29", "\n2020-02-29",
	"2020-1-01", "2020-01-1", "202-01-01", "20200-01-01", "2020-00-10", "2020-13-01", "2020-01-00", "2020-01-32", "2020/01/01",
	"0000-01-01", "1399-12-31T23:59:59+00:00", "1400-01-01T00:59:59+01:00", "1400-01-01T01:00:00+01:00", "9999-12-31T22:59:59-01:00",
	"9999-12-31T23:00:00-01:00" } do
	local Ok, Time = pcall(DateTime.fromIsoDate, Text)
	print("iso " .. string.format("%q", Text) .. " " .. (if not Ok then "E(" .. P.err(Time) .. ")" elseif Time == nil then "nil" else P.num(Time.UnixTimestampMillis)))
end

-- the fallback's start in lookAt and lookAlong, and where fromMatrix makes its Z column a unit vector
for _, E in { 4.5e-7, 4.9999994e-7, 5e-7 } do
	print("lookat-threshold " .. P.f32(E) .. " " .. P.f32(CFrame.lookAlong(Vector3.zero, Vector3.new(E, 0, 1), Vector3.zAxis).RightVector.Y)
		.. " " .. P.f32(CFrame.lookAt(Vector3.zero, Vector3.new(E, 1, 0)).RightVector.Z))
end
for _, A in { 6e-7, 8e-7, 1e-6 } do
	print("frommatrix-threshold " .. P.f32(A) .. " " .. P.f32(CFrame.fromMatrix(Vector3.zero, Vector3.new(A, 0, 0), Vector3.yAxis).ZVector.Z)
		.. " " .. P.f32s(CFrame.fromMatrix(Vector3.zero, Vector3.xAxis, Vector3.new(0, A, A)).ZVector.Y))
end

-- fromIsoDate: punctuation after the seconds, offsets' forms and bounds, partial dates
local Base = "2020-02-29T12:34:56"
local IsoCases = {
	Base .. ",", Base .. ",5", Base .. ",Z", Base .. ";Z", Base .. ",junk", Base .. ".5,Z", Base .. ":5Z", Base .. " 5Z", Base .. "x+01:00", Base .. "junk+01:00", Base .. "Zx+01:00",
	Base .. "Z+01:00x", Base .. "Z-01:00", Base .. "z+01:00", Base .. ".5+01:00", Base .. ".5Z+01:00", Base .. "+12:00", Base .. "+13:00", Base .. "+14:00", Base .. "+14:01", Base .. "+15:00",
	Base .. "+18:00", Base .. "+19:00", Base .. "-12:00", Base .. "-13:00", Base .. "-14:00", Base .. "-14:01", Base .. "-15:00", Base .. "+12:59", Base .. "+2:00", Base .. "+02:00:00",
	Base .. "+02:00Z", Base .. "+02:00\t", Base .. "+02:00\n", Base .. "+02:00  ", Base .. "+02:30", Base .. "+02:5", Base .. "+0", Base .. "+", Base .. "-", Base .. "+ 2:00",
	Base .. "+02:00 x", Base .. "Z ", Base .. "\t", Base .. " ", Base .. " junk", Base .. "+02 :00", Base .. "+02x00", Base .. "+02-00",
	"2020-2-29", "2020-2-2", "2020-12-1", "2020-001-01", "2020--1-01", "2020-1-01T12:00Z", "2020-02-29T12Z", "2020-02-29T12x", "2020-02-29T12:34x",
	"2020-02-29TT12:34", "2020-02-29T12::34", "2020-02-29T12:34::56", "2020-02-29T12:34:56.1234", "2020-02-29T-1:00:00", "2020-02-29T+1:00:00",
	"2020-02-29T 1:00:00", "2020-+2-29", "2020- 2-29", "2020-02- 9", "2020-02-+9", "+2020-02-29", " 2020-02-29 ", "2020-02-29  ", "2020-02-29\t",
	"2020", "2020-", "2020-02", "2020-02-", "2020-02-2", "2020-1-1", "2020-02-29T12:34:56Z\t", "2020-02-29T12:34:56.5 ", "2020-02-29T12:34:56 +01:00",
	"2020-02-29T12:34:56Z +01:00", "2020-02-29T12:34:56.5x+01:00", "2020-02-29T12:34:56+01:00:30", "2020-02-29T12:34:56-00:30",
}
for _, Rest in { "+02-30", "+02-3x", "-x", "-Z", "- 5", "-junk", "+x", "+Z", ",x5", ", 5", ",,5", ":00", ":00Z", ":00:00", ":5", ":123", ".5:00", ":00+01:00",
	":00x", ":0", "+02:00-01:00", "+02:00:0", "+02:00:", "+02:", ",5:30", "-5", "+5", ",", ",x", ", ", "- ", "+ ", "-\t", ",\t", "+02:00:00:00", "+02:00:00x",
	"+02:00:5", ".5-", ".5,", "Z-", "Z,", "Z,5", "x-", "x,", "x,5", " -", " ,", " ,5", "-05", "-5:30", ",05:30", "+02 30", "+02:x", "+02-x", "-02-", "-02:" } do
	table.insert(IsoCases, Base .. Rest)
end
for _, Text in IsoCases do
	local Ok, Time = pcall(DateTime.fromIsoDate, Text)
	print("iso-more " .. string.format("%q", Text) .. " " .. (if not Ok then "E(" .. P.err(Time) .. ")" elseif Time == nil then "nil" else P.num(Time.UnixTimestampMillis)))
end
