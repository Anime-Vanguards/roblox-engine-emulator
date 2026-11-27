-- Where Euler angles lock (CFrame:ToEulerAngles in each order, ToOrientation, ToEulerAnglesXYZ): the middle term at 1 - k float
-- steps, and an entry near 1 in a row that is no rotation's; a part's Orientation and Rotation (those angles to the thousandth of a
-- degree) around a quarter turn
P.sync()
local Orders = { "XYZ", "XZY", "YXZ", "YZX", "ZXY", "ZYX" }
local function Euler(C)
	local Parts = {}
	for _, Name in Orders do
		local X, Y, Z = C:ToEulerAngles(Enum.RotationOrder[Name])
		table.insert(Parts, Name .. " " .. P.f32s(X, Y, Z))
	end
	local Ox, Oy, Oz = C:ToOrientation()
	local Rx, Ry, Rz = C:ToEulerAnglesXYZ()
	return table.concat(Parts, " ; ") .. " ; ori " .. P.f32s(Ox, Oy, Oz) .. " ; xyz " .. P.f32s(Rx, Ry, Rz)
end
for _, K in { 1, 4, 6, 7, 8, 9, 12, 18 } do
	local E = 1 - K * 2 ^ -24
	local S = math.sqrt(1 - E * E)
	print("k" .. K .. " xyz", Euler(CFrame.new(0, 0, 0, S, 0, E, 0, 1, 0, -E, 0, S)))
	print("k" .. K .. " yxz", Euler(CFrame.new(0, 0, 0, 1, 0, 0, 0, S, -E, 0, E, S)))
	print("k" .. K .. " -xyz", Euler(CFrame.new(0, 0, 0, S, 0, -E, 0, 1, 0, E, 0, S)))
end
print("near-entry wide-row", Euler(CFrame.new(0, 0, 0, 0.5, 0.5, 0.9999999, 0, 1, 0, 0, 0, 1)))
print("far-entry empty-row", Euler(CFrame.new(0, 0, 0, 0, 0, 0.5, 0, 1, 0, 0, 0, 1)))
print("near-entry wide-col", Euler(CFrame.new(0, 0, 0, 1, 0, 0.9999999, 0, 1, 0.5, 0, 0, 0.5)))
print("far-entry empty-col", Euler(CFrame.new(0, 0, 0, 1, 0, 0.5, 0, 1, 0, 0, 0, 0)))
print("over-one", Euler(CFrame.new(0, 0, 0, 0, 0, 1.5, 0, 1, 0, -1, 0, 0)))
print("under-minus-one", Euler(CFrame.new(0, 0, 0, 0, 0, -1.5, 0, 1, 0, 1, 0, 0)))
-- a part's angles
local Part = Instance.new("Part")
Part.Anchored = true
Part.Parent = workspace
local function Row(Label, Cf)
	Part.CFrame = Cf
	local O, R = Part.Orientation, Part.Rotation
	print(Label, "|O", P.f32s(O.X, O.Y, O.Z), "|R", P.f32s(R.X, R.Y, R.Z))
end
for _, Deg in { 45, 89, 89.5, 89.74, 89.75, 89.9, 89.95, 89.99, 90, 90.01, 90.1, 91 } do
	Row("ox " .. Deg, CFrame.fromOrientation(math.rad(Deg), math.rad(30), math.rad(20)))
	Row("-ox " .. Deg, CFrame.fromOrientation(-math.rad(Deg), math.rad(30), math.rad(20)))
	Row("ry " .. Deg, CFrame.Angles(math.rad(10), math.rad(Deg), math.rad(25)))
	Row("-ry " .. Deg, CFrame.Angles(math.rad(10), -math.rad(Deg), math.rad(25)))
end
for _, Angles in { { 10.12345, 20.6789, 30.5555 }, { 0.00049, -0.00051, 0.0015 }, { 123.4565, -0.0004, 179.9996 }, { 1e-5, 2e-5, -3e-5 } } do
	Row("read " .. table.concat(Angles, ","), CFrame.fromOrientation(math.rad(Angles[1]), math.rad(Angles[2]), math.rad(Angles[3])))
end
Part:Destroy()
