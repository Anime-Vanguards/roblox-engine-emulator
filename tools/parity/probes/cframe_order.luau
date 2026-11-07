-- The float32 order of operations behind CFrame and the vector and colour operations: inputs chosen to tell apart the
-- formulas that fit the datatypes probe equally well (single-precision trig or double trig rounded, atan2 and acos in
-- floats or doubles, lookAlong's own form, Lerp's rotation, near-vertical lookAt, the third column of Orthonormalize,
-- the Euler orders, Lerp's alpha precision, ToHSV's hue, fromRGB's scale).
local Rng = P.rng(77)
local function V3(V)
	return P.f32s(V.X, V.Y, V.Z)
end
local function CF(C)
	return P.f32s(C:GetComponents())
end
-- 1 trig: R02 = sin A, R00 = cos A for angles where single and double trig round apart
for _, A in { 0.753655374, 2.42280459, -0.651295364, -0.656289637, 0.658208609, 0.48682332, -0.811513424, 0.833714724, -2.33957767, 2.35872865, -0.840261996, -1.0096271 } do
	print("trig " .. P.f32(A) .. " " .. CF(CFrame.Angles(0, A, 0)))
end
-- 2 atan2 and acos: angles between vectors, Euler and axis-angle decompositions
for Case = 1, 150 do
	local A = Vector3.new(Rng.real(-10, 10), Rng.real(-10, 10), Rng.real(-10, 10))
	local B = Vector3.new(Rng.real(-10, 10), Rng.real(-10, 10), Rng.real(-10, 10))
	local C = CFrame.Angles(Rng.real(-3, 3), Rng.real(-1.5, 1.5), Rng.real(-3, 3))
	local Axis, Theta = C:ToAxisAngle()
	print("many " .. Case .. " " .. P.f32s(A:Angle(B), Vector2.new(A.X, A.Y):Angle(Vector2.new(B.X, B.Y), true), C:ToEulerAnglesXYZ()) .. " " .. P.f32s(C:ToEulerAnglesYXZ()) .. " " .. V3(Axis) .. " " .. P.f32(Theta))
end
-- 3 lookAlong against lookAt(pos, pos + dir), far from the origin
local Far = Vector3.new(1000.123, -2000.5, 3000.25)
for _, D in { Vector3.new(0.3, 0.2, -0.9), Vector3.new(-0.1234567, 0.7654321, 0.5), Vector3.new(0.01, -0.99, 0.02) } do
	print("lookalong " .. CF(CFrame.lookAlong(Far, D)) .. " | " .. CF(CFrame.lookAt(Far, Far + D)))
end
-- 4 Lerp's rotation without the products: the identity and an axis-angle rotation
for Case = 1, 30 do
	local R = CFrame.fromAxisAngle(Vector3.new(Rng.real(-1, 1), Rng.real(-1, 1), Rng.real(-1, 1)), Rng.real(0.05, 3.1))
	local T = Rng.pick({ 0.1, 0.25, 0.3, 0.5, 0.7, 0.9, 1.5, -0.5 })
	print("lerp " .. Case .. " " .. P.f32(T) .. " " .. CF(CFrame.identity:Lerp(R, T)) .. " | " .. CF(R:Lerp(CFrame.identity, T)) .. " | " .. CF(R:Lerp(R * CFrame.Angles(0.2, 0, 0), T)))
end
-- 5 near-vertical lookAt and CFrame.new(pos, lookAt): where the fallback up vector starts
for _, E in { 0.2, 0.15, 0.14, 0.1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8 } do
	print("vertical " .. P.f32(E) .. " " .. CF(CFrame.lookAt(Vector3.zero, Vector3.new(E, 1, 0))) .. " | " .. CF(CFrame.new(Vector3.zero, Vector3.new(E, 1, 0))) .. " | " .. CF(CFrame.lookAt(Vector3.zero, Vector3.new(0, -1, E))))
end
-- 6 Orthonormalize: the third column's Gram-Schmidt form
for Case = 1, 8 do
	local M = CFrame.new(0, 0, 0, 1 + Rng.real(-0.5, 0.5), Rng.real(-0.5, 0.5), Rng.real(-0.5, 0.5), Rng.real(-0.5, 0.5), 1 + Rng.real(-0.5, 0.5), Rng.real(-0.5, 0.5), Rng.real(-0.5, 0.5), Rng.real(-0.5, 0.5), 1 + Rng.real(-0.5, 0.5))
	print("ortho " .. Case .. " " .. CF(M) .. " | " .. CF(M:Orthonormalize()))
end
-- 7 Euler orders the datatypes probe does not use
for _, Order in { Enum.RotationOrder.XZY, Enum.RotationOrder.YZX, Enum.RotationOrder.ZYX, Enum.RotationOrder.XYZ, Enum.RotationOrder.YXZ } do
	local C = CFrame.fromEulerAngles(0.7, -1.3, 2.1, Order)
	print("order " .. tostring(Order) .. " " .. CF(C) .. " | " .. P.f32s(C:ToEulerAngles(Order)))
end
-- 8 Lerp at alphas that separate single from double precision
for Case = 1, 20 do
	local A = Vector3.new(Rng.real(-10, 10), Rng.real(-10, 10), Rng.real(-10, 10))
	local B = Vector3.new(Rng.real(-10, 10), Rng.real(-10, 10), Rng.real(-10, 10))
	local T = Rng.pick({ 0.3, 0.7, 0.1, 1.7 })
	local L2 = Vector2.new(A.X, A.Y):Lerp(Vector2.new(B.X, B.Y), T)
	print("vlerp " .. Case .. " " .. P.f32(T) .. " " .. V3(A:Lerp(B, T)) .. " " .. P.f32s(L2.X, L2.Y))
end
-- 9 ToHSV's hue where green is the largest channel
for Case = 1, 40 do
	local G = Rng.real(0.5, 1)
	local Color = Color3.new(Rng.real(0, G), G, Rng.real(0, G))
	print("hsv " .. Case .. " " .. P.f32s(Color.R, Color.G, Color.B) .. " " .. P.f32s(Color:ToHSV()))
end
-- 10 fromRGB: n / 255 against n * (1 / 255)
print("rgb " .. P.f32s(Color3.fromRGB(196, 40, 3).R, Color3.fromRGB(196, 40, 3).G, Color3.fromRGB(98, 37, 209).B))
