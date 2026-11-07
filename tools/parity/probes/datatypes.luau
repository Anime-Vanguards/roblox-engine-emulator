-- Datatype arithmetic and precision over generated inputs: CFrame, Vector3, Vector2, Color3, UDim2 and friends.
local Rng = P.rng(2024)
local function V3(V)
	return P.f32s(V.X, V.Y, V.Z)
end
local function CF(C)
	return P.f32s(C:GetComponents())
end
local function Angle()
	return Rng.real(-math.pi, math.pi)
end
local function Vec(Scale)
	return Vector3.new(Rng.real(-Scale, Scale), Rng.real(-Scale, Scale), Rng.real(-Scale, Scale))
end
local function Frame()
	return CFrame.new(Vec(100)) * CFrame.fromEulerAnglesXYZ(Angle(), Angle(), Angle())
end

-- constructors and Euler orders
for Case = 1, 8 do
	local X, Y, Z = Angle(), Angle(), Angle()
	local Pos = Vec(50)
	print("cframe-ctor " .. Case .. " angles " .. CF(CFrame.Angles(X, Y, Z)) .. " | xyz " .. CF(CFrame.fromEulerAnglesXYZ(X, Y, Z)) .. " | yxz " .. CF(CFrame.fromEulerAnglesYXZ(X, Y, Z)) .. " | orient " .. CF(CFrame.fromOrientation(X, Y, Z)))
	print("cframe-ctor2 " .. Case .. " lookat " .. CF(CFrame.lookAt(Pos, Pos + Vec(10))) .. " | new2 " .. CF(CFrame.new(Pos, Pos + Vec(10))) .. " | axisangle " .. CF(CFrame.fromAxisAngle(Vec(1).Unit, X)) .. " | lookalong " .. P.try(function()
		return CF(CFrame.lookAlong(Pos, Vec(1)))
	end))
	local Euler = CFrame.fromEulerAngles(X, Y, Z, Enum.RotationOrder.ZXY)
	print("cframe-euler-orders " .. Case .. " " .. CF(Euler) .. " | " .. P.try(function()
		local A, B, C = Euler:ToEulerAngles(Enum.RotationOrder.ZXY)
		return P.f32s(A, B, C)
	end))
end
-- operations
for Case = 1, 10 do
	local A, B = Frame(), Frame()
	local Point = Vec(20)
	print("cframe-mul " .. Case .. " " .. CF(A * B))
	print("cframe-inverse " .. Case .. " " .. CF(A:Inverse()) .. " | toobject " .. CF(A:ToObjectSpace(B)) .. " | toworld " .. CF(A:ToWorldSpace(B)))
	print("cframe-points " .. Case .. " " .. V3(A * Point) .. " | " .. V3(A:PointToObjectSpace(Point)) .. " | " .. V3(A:PointToWorldSpace(Point)) .. " | " .. V3(A:VectorToObjectSpace(Point)) .. " | " .. V3(A:VectorToWorldSpace(Point)))
	print("cframe-lerp " .. Case .. " " .. CF(A:Lerp(B, 0.3)) .. " | " .. CF(A:Lerp(B, 0)) .. " | " .. CF(A:Lerp(B, 1)) .. " | " .. CF(A:Lerp(B, 1.5)))
	local X, Y, Z = A:ToEulerAnglesXYZ()
	local Yx, Yy, Yz = A:ToEulerAnglesYXZ()
	local Ox, Oy, Oz = A:ToOrientation()
	local Axis, Theta = A:ToAxisAngle()
	print("cframe-decompose " .. Case .. " xyz " .. P.f32s(X, Y, Z) .. " yxz " .. P.f32s(Yx, Yy, Yz) .. " orient " .. P.f32s(Ox, Oy, Oz) .. " axis " .. V3(Axis) .. " " .. P.f32(Theta))
	print("cframe-vectors " .. Case .. " look " .. V3(A.LookVector) .. " right " .. V3(A.RightVector) .. " up " .. V3(A.UpVector) .. " x " .. V3(A.XVector) .. " rot " .. CF(A.Rotation) .. " pos " .. V3(A.Position))
	print("cframe-add " .. Case .. " " .. CF(A + Point) .. " | " .. CF(A - Point) .. " | fuzzy " .. P.try(function()
		return tostring(A:FuzzyEq(A * CFrame.new(1e-6, 0, 0))) .. " " .. tostring(A:FuzzyEq(B))
	end))
	print("cframe-orthonormalize " .. Case .. " " .. P.try(function()
		local Skewed = CFrame.new(0, 0, 0, 1, 0.1, 0, 0, 1, 0.1, 0.1, 0, 1)
		return CF(Skewed:Orthonormalize())
	end))
end
print("cframe-identity " .. CF(CFrame.identity) .. " " .. tostring(CFrame.new() == CFrame.identity) .. " " .. tostring(CFrame.new(1, 2, 3) == CFrame.new(1, 2, 3)))
print("cframe-12 " .. CF(CFrame.new(1, 2, 3, 2, 0, 0, 0, 2, 0, 0, 0, 2)))
print("cframe-lookat-up " .. CF(CFrame.lookAt(Vector3.zero, Vector3.new(0, 10, 0))) .. " | " .. CF(CFrame.lookAt(Vector3.zero, Vector3.zero)))
print("cframe-nan " .. CF(CFrame.new(0 / 0, 0, 0)) .. " | " .. CF(CFrame.Angles(0 / 0, 0, 0)))
print("cframe-huge " .. CF(CFrame.new(1e20, -1e20, 1e-20) * CFrame.Angles(1e10, 0, 0)))
print("cframe-tostring " .. tostring(CFrame.Angles(0.1, 0.2, 0.3)))
print("cframe-components-count " .. select("#", CFrame.new():GetComponents()))

-- Vector3
for Case = 1, 10 do
	local A, B = Vec(10), Vec(10)
	local S = Rng.real(-5, 5)
	print("v3 " .. Case .. " add " .. V3(A + B) .. " sub " .. V3(A - B) .. " mul " .. V3(A * B) .. " scale " .. V3(A * S) .. " div " .. V3(A / S) .. " idiv " .. V3(A // 2) .. " neg " .. V3(-A))
	print("v3b " .. Case .. " mag " .. P.f32(A.Magnitude) .. " unit " .. V3(A.Unit) .. " dot " .. P.f32(A:Dot(B)) .. " cross " .. V3(A:Cross(B)) .. " lerp " .. V3(A:Lerp(B, 0.25)) .. " angle " .. P.f32(A:Angle(B)) .. " signed " .. P.f32(A:Angle(B, Vector3.yAxis)))
	print("v3c " .. Case .. " min " .. V3(A:Min(B)) .. " max " .. V3(A:Max(B)) .. " abs " .. V3(A:Abs()) .. " floor " .. V3(A:Floor()) .. " ceil " .. V3(A:Ceil()) .. " sign " .. V3(A:Sign()) .. " fuzzy " .. tostring(A:FuzzyEq(A + Vector3.new(1e-6, 0, 0))))
end
print("v3-zero-unit " .. V3(Vector3.zero.Unit) .. " mag " .. P.f32(Vector3.zero.Magnitude) .. " div0 " .. V3(Vector3.one / 0))
print("v3-precision " .. V3(Vector3.new(0.1, 0.2, 0.3)) .. " " .. V3(Vector3.new(1 / 3, 2 ^ 24 + 1, 1e-45)) .. " " .. tostring(Vector3.new(0.1, 0.2, 0.3) == Vector3.new(0.1, 0.2, 0.3)))
print("v3-fromnormalid " .. V3(Vector3.FromNormalId(Enum.NormalId.Back)) .. " axis " .. V3(Vector3.FromAxis(Enum.Axis.Y)))
print("v3-tostring " .. tostring(Vector3.new(1 / 3, -0.5, 1e-7)) .. " | " .. tostring(Vector3.new(123456789, 0, 0)))
print("v3-int16 " .. tostring(Vector3int16.new(40000, -40000, 1.9)) .. " | " .. tostring(Vector3int16.new(1, 2, 3) + Vector3int16.new(32767, 0, 0)))

-- Vector2
for Case = 1, 4 do
	local A = Vector2.new(Rng.real(-10, 10), Rng.real(-10, 10))
	local B = Vector2.new(Rng.real(-10, 10), Rng.real(-10, 10))
	print("v2 " .. Case .. " " .. P.f32s(A.Magnitude, A:Dot(B), A:Cross(B)) .. " unit " .. P.f32s(A.Unit.X, A.Unit.Y) .. " lerp " .. P.f32s(A:Lerp(B, 0.7).X, A:Lerp(B, 0.7).Y) .. " angle " .. P.try(function()
		return P.f32(A:Angle(B, true))
	end))
end

-- Color3
for Case = 1, 10 do
	local R, G, B = Rng.real(0, 1), Rng.real(0, 1), Rng.real(0, 1)
	local Color = Color3.new(R, G, B)
	local H, S, V = Color:ToHSV()
	local Other = Color3.new(Rng.real(0, 1), Rng.real(0, 1), Rng.real(0, 1))
	local Lerped = Color:Lerp(Other, 0.4)
	print("color " .. Case .. " hsv " .. P.f32s(H, S, V) .. " back " .. P.f32s(Color3.fromHSV(H, S, V).R, Color3.fromHSV(H, S, V).G, Color3.fromHSV(H, S, V).B) .. " lerp " .. P.f32s(Lerped.R, Lerped.G, Lerped.B) .. " hex " .. Color:ToHex())
end
local Rgb = Color3.fromRGB(255, 128, 1)
print("color-rgb " .. P.f32s(Rgb.R, Rgb.G, Rgb.B) .. " over " .. P.f32s(Color3.fromRGB(300, -5, 0).R, Color3.fromRGB(300, -5, 0).G) .. " new-over " .. P.f32s(Color3.new(2, -1, 0.5).R, Color3.new(2, -1, 0.5).G))
print("color-hex " .. P.try(function()
	local C = Color3.fromHex("#FF8000")
	return P.f32s(C.R, C.G, C.B) .. " " .. Color3.fromHex("abc"):ToHex()
end) .. " | " .. P.try(Color3.fromHex, "zzz"))
print("color-hsv-edges " .. P.f32s(Color3.new(0, 0, 0):ToHSV()) .. " | " .. P.f32s(Color3.new(1, 1, 1):ToHSV()) .. " | " .. P.f32s(Color3.new(1, 0, 0):ToHSV()) .. " | " .. P.f32s(Color3.fromHSV(1, 1, 1).R, Color3.fromHSV(1.5, 1, 1).G, Color3.fromHSV(-0.25, 1, 1).B))
print("color-tostring " .. tostring(Color3.new(1 / 3, 0.5, 1)) .. " eq " .. tostring(Color3.new(0.5, 0.5, 0.5) == Color3.new(0.5, 0.5, 0.5)))
print("color-arith " .. P.try(function()
	return tostring(Color3.new(0.5, 0, 0) + Color3.new(0.25, 0, 0))
end))
print("brickcolor " .. BrickColor.new("Bright red").Name .. " " .. BrickColor.new(21).Name .. " " .. BrickColor.new(1, 0, 0).Name .. " " .. BrickColor.new(Color3.new(0.3, 0.6, 0.2)).Name .. " " .. BrickColor.new("NotAColor").Name .. " " .. tostring(BrickColor.new("Bright red").Number) .. " " .. P.f32s(BrickColor.new("Bright red").Color.R, BrickColor.new("Bright red").Color.G))

-- UDim2 and UDim
for Case = 1, 5 do
	local A = UDim2.new(Rng.real(-1, 1), Rng.int(-100, 100), Rng.real(-1, 1), Rng.int(-100, 100))
	local B = UDim2.new(Rng.real(-1, 1), Rng.int(-100, 100), Rng.real(-1, 1), Rng.int(-100, 100))
	local L = A:Lerp(B, 0.35)
	print("udim2 " .. Case .. " add " .. tostring(A + B) .. " sub " .. tostring(A - B) .. " lerp " .. P.f32s(L.X.Scale, L.X.Offset, L.Y.Scale, L.Y.Offset))
end
print("udim2-offset-rounding " .. tostring(UDim2.new(0, 1.5, 0, -1.5)) .. " | " .. tostring(UDim2.fromOffset(2.7, -2.7)) .. " | " .. tostring(UDim.new(0, 0.5)) .. " | " .. tostring(UDim2.new(0, 2 ^ 31, 0, 0)))
print("udim2-lerp-offset " .. tostring(UDim2.fromOffset(0, 0):Lerp(UDim2.fromOffset(10, 3), 0.5)) .. " | " .. tostring(UDim2.fromOffset(0, 0):Lerp(UDim2.fromOffset(1, 1), 0.5)))
print("udim2-unm " .. P.try(function()
	return tostring(-UDim2.new(1, 2, 3, 4))
end) .. " udim-arith " .. P.try(function()
	return tostring(UDim.new(1, 2) + UDim.new(0.5, 3)) .. " " .. tostring(UDim.new(1, 2) - UDim.new(0.5, 3))
end))

-- Rect, NumberRange, NumberSequence, ColorSequence
print("rect " .. tostring(Rect.new(4, 3, 1, 2)) .. " w " .. P.f32(Rect.new(4, 3, 1, 2).Width) .. " h " .. P.f32(Rect.new(4, 3, 1, 2).Height))
print("numberrange " .. tostring(NumberRange.new(1)) .. " | " .. P.try(NumberRange.new, 2, 1) .. " | " .. tostring(NumberRange.new(1, 2)))
print("numbersequence " .. tostring(NumberSequence.new({ NumberSequenceKeypoint.new(0, 1), NumberSequenceKeypoint.new(0.5, 0.25, 0.1), NumberSequenceKeypoint.new(1, 0) })))
print("numbersequence-errors " .. P.try(NumberSequence.new, {}) .. " | " .. P.try(NumberSequence.new, { NumberSequenceKeypoint.new(0.5, 1), NumberSequenceKeypoint.new(1, 0) }))
print("colorsequence " .. tostring(ColorSequence.new(Color3.new(1, 0, 0), Color3.new(0, 1, 0))))
print("region3 " .. P.try(function()
	local Region = Region3.new(Vector3.new(-1, -2, -3), Vector3.new(3, 2, 1))
	return CF(Region.CFrame) .. " size " .. V3(Region.Size) .. " expand " .. V3(Region:ExpandToGrid(4).Size)
end))
print("ray " .. P.try(function()
	local R = Ray.new(Vector3.new(0, 0, 0), Vector3.new(10, 0, 0))
	return V3(R:ClosestPoint(Vector3.new(5, 5, 0))) .. " " .. P.f32(R:Distance(Vector3.new(5, 5, 0))) .. " unit " .. V3(R.Unit.Direction)
end))
print("tweeninfo " .. tostring(TweenInfo.new(1, Enum.EasingStyle.Back, Enum.EasingDirection.InOut, 2, true, 0.5)) .. " " .. P.try(TweenInfo.new, -1) .. " " .. P.try(function()
	local Info = TweenInfo.new()
	return P.num(Info.Time) .. " " .. tostring(Info.EasingStyle) .. " " .. tostring(Info.EasingDirection) .. " " .. Info.RepeatCount .. " " .. tostring(Info.Reverses) .. " " .. P.num(Info.DelayTime)
end))
print("physicalproperties " .. P.try(function()
	local Props = PhysicalProperties.new(Enum.Material.Wood)
	return P.f32s(Props.Density, Props.Friction, Props.Elasticity, Props.FrictionWeight, Props.ElasticityWeight)
end))
