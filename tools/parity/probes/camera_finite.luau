-- A Camera's CFrame writes that are not finite or not near a rotation (world/camera.rs): a CFrame or CoordinateFrame write with
-- a component that is not finite, or a rotation entry of 1.2 or more either way, leaves the camera as it was and fires
-- nothing; anything else is kept as written (scaled and skewed rotations too). Focus takes anything. PanUnits, TiltUnits and
-- ZoomToExtents refuse such a CFrame of their own; ZoomToExtents' Focus is the middle of the box's bounds, NaN for a NaN or
-- infinite size.
local RunService = game:GetService("RunService")
local NaN, Inf = 0 / 0, math.huge
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
end
local function M(R00, R01, R02, R10, R11, R12, R20, R21, R22)
	return CFrame.new(1, 2, 3, R00, R01, R02, R10, R11, R12, R20, R21, R22)
end
local Cases = {
	{ "nan x", CFrame.new(NaN, 2, 3) },
	{ "nan z", CFrame.new(1, 2, NaN) },
	{ "inf x", CFrame.new(Inf, 2, 3) },
	{ "-inf y", CFrame.new(1, -Inf, 3) },
	{ "1e39 x (inf as a float)", CFrame.new(1e39, 2, 3) },
	{ "3e38 x", CFrame.new(3e38, 2, 3) },
	{ "angles nan", CFrame.new(1, 2, 3) * CFrame.Angles(NaN, 0, 0) },
	{ "r00 nan", M(NaN, 0, 0, 0, 1, 0, 0, 0, 1) },
	{ "r01 nan", M(1, NaN, 0, 0, 1, 0, 0, 0, 1) },
	{ "r22 inf", M(1, 0, 0, 0, 1, 0, 0, 0, Inf) },
	{ "zero rotation", M(0, 0, 0, 0, 0, 0, 0, 0, 0) },
	{ "mirror", M(-1, 0, 0, 0, 1, 0, 0, 0, 1) },
	{ "turned", CFrame.new(1, 2, 3) * CFrame.Angles(0.3, 0.2, 0.1) },
	{ "lean 1e-6", M(1, 1e-6, 0, 0, 1, 0, 0, 0, 1) },
	{ "all ones", M(1, 1, 1, 1, 1, 1, 1, 1, 1) },
}
for _, S in { 1.1, 1.199, 1.19999993, 1.2, 1.5, 2, -1.19999993, -1.2 } do
	table.insert(Cases, { `r00 {S}`, M(S, 0, 0, 0, 1, 0, 0, 0, 1) })
end
for I = 1, 9 do
	local E = { 1, 0, 0, 0, 1, 0, 0, 0, 1 }
	E[I] = if I % 2 == 0 then -1.2 else 1.2
	table.insert(Cases, { `entry {I} {E[I]}`, M(table.unpack(E)) })
end
local function Components(Value)
	return P.f32s(Value:GetComponents())
end
for _, Which in { "current", "new", "loose" } do
	local Cam
	if Which == "current" then
		Cam = workspace.CurrentCamera
	else
		Cam = Instance.new("Camera")
		if Which == "new" then
			Cam.Parent = workspace
		end
	end
	Cam.CameraType = Enum.CameraType.Scriptable
	for _, Member in { "CFrame", "CoordinateFrame", "Focus" } do
		for _, Case in Cases do
			Cam[Member] = CFrame.new(5, 6, 7)
			Frames(1)
			local Fired = {}
			local Connection = Cam.Changed:Connect(function(Prop)
				if Prop == "CFrame" or Prop == "Focus" or Prop == "CoordinateFrame" then
					table.insert(Fired, Prop)
				end
			end)
			local Ok, Error = pcall(function()
				Cam[Member] = Case[2]
			end)
			local Read = Cam[Member]
			Frames(1)
			Connection:Disconnect()
			print(Which, Member, Case[1], if Ok then "ok" else P.err(Error), Components(Read), "fired", table.concat(Fired, ","))
		end
	end
	Cam.CFrame = CFrame.new(5, 6, 7)
	Cam.Focus = CFrame.new(NaN, 0, 0)
	Cam:PanUnits(1)
	print(Which, "PanUnits about a NaN focus", Components(Cam.CFrame))
	Cam.CFrame = CFrame.new(5, 6, 7)
	Cam.Focus = CFrame.new(Inf, 0, 0)
	print(Which, "TiltUnits about an infinite focus", Cam:TiltUnits(1), Components(Cam.CFrame))
	for _, Zoom in { { "NaN centre", CFrame.new(NaN, 0, 0), Vector3.new(4, 4, 4) }, { "infinite size", CFrame.new(1, 2, 3), Vector3.new(Inf, 4, 4) }, { "NaN size", CFrame.new(1, 2, 3), Vector3.new(NaN, 4, 4) }, { "huge size", CFrame.new(1, 2, 3), Vector3.new(1e38, 1e38, 1e38) } } do
		Cam.CFrame = CFrame.new(5, 6, 7)
		Cam.Focus = CFrame.new(0, 0, 0)
		Cam:ZoomToExtents(Zoom[2], Zoom[3])
		print(Which, "ZoomToExtents", Zoom[1], Components(Cam.CFrame), Components(Cam.Focus))
	end
	Cam.CFrame = CFrame.new(5, 6, 7)
	Cam.Focus = CFrame.new(0, 0, 0)
end
