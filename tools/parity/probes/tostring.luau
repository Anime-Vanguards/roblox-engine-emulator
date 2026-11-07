-- tostring of every datatype with fractional, negative, tiny and huge components (the number format each one prints),
-- and Color3's hex, arithmetic and constructors at their edges.
local function Show(Label, Make)
	local Made, Value = pcall(Make)
	if not Made then
		print(Label .. " E(" .. P.err(Value) .. ")")
		return
	end
	print(Label .. " " .. (string.gsub(tostring(Value), "0x%x+", "ADDR")))
end
local Third, Neg, Tiny, Huge = 1 / 3, -2.5, 1e-7, 1e20

Show("udim", function() return UDim.new(Third, 7) end)
Show("udim-neg", function() return UDim.new(-0.1, -3) end)
Show("udim-huge", function() return UDim.new(Huge, 2 ^ 31) end)
Show("udim2", function() return UDim2.new(Third, 7, Neg, -9) end)
Show("udim2-tiny", function() return UDim2.new(Tiny, 0, -0, 1) end)
Show("vector2", function() return Vector2.new(Third, Neg) end)
Show("vector2-edges", function() return Vector2.new(Tiny, Huge) end)
Show("vector2-negzero", function() return Vector2.new(-0, 0.1) end)
Show("vector3", function() return Vector3.new(Third, Neg, Tiny) end)
Show("vector2int16", function() return Vector2int16.new(-5, 32767) end)
Show("vector2int16-wrap", function() return Vector2int16.new(32768, -32769) end)
Show("vector3int16", function() return Vector3int16.new(-5, 32767, 7) end)
Show("vector3int16-wrap", function() return Vector3int16.new(32768, -32769, 65537) end)
Show("vector3int16-frac", function() return Vector3int16.new(1.5, -1.5, 2.7) end)
Show("cframe", function() return CFrame.new(Third, Neg, Huge) end)
Show("cframe-rot", function() return CFrame.new(0.1, -0, Tiny) * CFrame.Angles(0.1, 0.2, 0.3) end)
Show("color3", function() return Color3.new(Third, 0.1, 2 / 3) end)
Show("color3-range", function() return Color3.new(-1, 2.5, Huge) end)
Show("color3-tiny", function() return Color3.new(Tiny, 0.123456789, 1e-20) end)
Show("rect", function() return Rect.new(Third, Neg, 0.1, Huge) end)
Show("rect-vectors", function() return Rect.new(Vector2.new(Tiny, -0), Vector2.new(5, 6)) end)
Show("numberrange", function() return NumberRange.new(Third, 2.5) end)
Show("numberrange-one", function() return NumberRange.new(-0.1) end)
Show("numberrange-huge", function() return NumberRange.new(Tiny, Huge) end)
Show("numbersequence", function() return NumberSequence.new({ NumberSequenceKeypoint.new(0, Third, 0.1), NumberSequenceKeypoint.new(0.5, Tiny, Huge), NumberSequenceKeypoint.new(1, Neg, 0) }) end)
Show("numbersequence-one", function() return NumberSequence.new(Third) end)
Show("numbersequencekeypoint", function() return NumberSequenceKeypoint.new(0.25, Third, 0.5) end)
Show("colorsequence", function() return ColorSequence.new({ ColorSequenceKeypoint.new(0, Color3.new(Third, 0.1, 2 / 3)), ColorSequenceKeypoint.new(Third, Color3.new(Tiny, 1, 0.5)), ColorSequenceKeypoint.new(1, Color3.new(1, 0, 0.25)) }) end)
Show("colorsequencekeypoint", function() return ColorSequenceKeypoint.new(0.25, Color3.new(Third, 0.1, 2 / 3)) end)
Show("ray", function() return Ray.new(Vector3.new(Third, Neg, 0.1), Vector3.new(Huge, -0, Tiny)) end)
Show("region3", function() return Region3.new(Vector3.new(-Third, Neg, 0), Vector3.new(1, 2.25, 0.1)) end)
Show("region3int16", function() return Region3int16.new(Vector3int16.new(-1, 2, 3), Vector3int16.new(4, 5, 6)) end)
Show("tweeninfo", function() return TweenInfo.new(0.1, Enum.EasingStyle.Sine, Enum.EasingDirection.In, -1, true, Third) end)
Show("tweeninfo-default", function() return TweenInfo.new() end)
Show("axes-normals", function() return Axes.new(Enum.NormalId.Left, Enum.NormalId.Top) end)
Show("axes-all", function() return Axes.new(Enum.Axis.X, Enum.Axis.Y, Enum.Axis.Z) end)
Show("axes-none", function() return Axes.new() end)
Show("faces", function() return Faces.new(Enum.NormalId.Top, Enum.NormalId.Left, Enum.NormalId.Back) end)
Show("faces-none", function() return Faces.new() end)
Show("physicalproperties-material", function() return PhysicalProperties.new(Enum.Material.Wood) end)
Show("physicalproperties-five", function() return PhysicalProperties.new(Third, 0.1, 0.25, 2, 3) end)
Show("datetime-millis", function() return DateTime.fromUnixTimestampMillis(1700000000123) end)
Show("datetime-seconds", function() return DateTime.fromUnixTimestamp(1700000000) end)
Show("datetime-negative", function() return DateTime.fromUnixTimestampMillis(-1500) end)
Show("pathwaypoint", function() return PathWaypoint.new(Vector3.new(Third, 2, 3), Enum.PathWaypointAction.Jump, "label") end)
Show("font", function() return Font.new("rbxasset://fonts/families/Arial.json", Enum.FontWeight.Bold, Enum.FontStyle.Italic) end)
Show("raycastparams-set", function()
	local Params = RaycastParams.new()
	Params.FilterDescendantsInstances = { workspace }
	Params.FilterType = Enum.RaycastFilterType.Include
	Params.IgnoreWater = true
	Params.CollisionGroup = "Default"
	Params.RespectCanCollide = true
	return Params
end)
Show("overlapparams-set", function()
	local Params = OverlapParams.new()
	Params.MaxParts = 5
	Params.FilterDescendantsInstances = { workspace, game:GetService("Lighting") }
	return Params
end)
Show("random", function() return Random.new(5) end)
Show("brickcolor", function() return BrickColor.new(1001) end)
Show("enum", function() return Enum.Material end)

-- Color3:ToHex: rounding, clamping and case
local Hexes = {}
for _, Color in { Color3.new(0.5, 0.5, 0.5), Color3.new(Third, 2 / 3, 1), Color3.new(0.0019, 0.002, 0.998), Color3.new(-0.1, 1.5, 0.5), Color3.fromRGB(13, 254, 177), Color3.new(1 / 255 * 0.5, 1 / 255 * 1.5, 1 / 255 * 2.5) } do
	table.insert(Hexes, Color:ToHex())
end
print("tohex " .. table.concat(Hexes, " "))
-- Color3.fromRGB at its edges, read back exactly
local function C3(Color)
	return P.f32s(Color.R, Color.G, Color.B)
end
print("fromrgb " .. C3(Color3.fromRGB(300, -5, 127.5)) .. " | " .. C3(Color3.fromRGB(255, 128, 1)) .. " | " .. C3(Color3.fromRGB(1.4, 1.6, 254.5)))
print("fromhex " .. P.try(function() return C3(Color3.fromHex("#ABC")) end) .. " | " .. P.try(function() return C3(Color3.fromHex("abcdef")) end)
	.. " | " .. P.try(Color3.fromHex, "ZZZ") .. " | " .. P.try(Color3.fromHex, "#12") .. " | " .. P.try(Color3.fromHex, "1234567") .. " | " .. P.try(Color3.fromHex, "")
	.. " | " .. P.try(Color3.fromHex, " #ABC") .. " | " .. P.try(Color3.fromHex, "#GGGGGG") .. " | " .. P.try(Color3.fromHex, 5))
-- Color3 arithmetic and comparison
local A, B = Color3.new(0.5, 0.25, 1), Color3.new(0.25, 0.5, 0)
print("color3-ops " .. P.try(function() return A + B end) .. " | " .. P.try(function() return A - B end) .. " | " .. P.try(function() return A * B end)
	.. " | " .. P.try(function() return A * 2 end) .. " | " .. P.try(function() return A / 2 end) .. " | " .. P.try(function() return -A end)
	.. " | " .. tostring(A == Color3.new(0.5, 0.25, 1)) .. " " .. tostring(A == B))
print("color3-lerp " .. C3(A:Lerp(B, Third)) .. " | " .. C3(A:Lerp(B, 2)) .. " | " .. P.try(function() return A:Lerp(B) end))
print("color3-new " .. C3(Color3.new()) .. " | " .. C3(Color3.new(0.5)) .. " | " .. P.try(Color3.new, "0.5", 1, 1) .. " | " .. P.try(Color3.new, "x", 1, 1))
