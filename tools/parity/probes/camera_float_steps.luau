-- parity: device=desktop rendered
-- Float steps to the bit (world/camera_fov.rs, world/camera_moves.rs): DiagonalFieldOfView and MaxAxisFieldOfView written on the
-- start camera (1920 x 1080) and a new one (1 x 1), FieldOfView read back after each; ZoomToExtents' CFrame and Focus from 40
-- random boxes on each camera (the box's bounds as low and high corners, their middle and half extents).
P.sync()
local Start = workspace.CurrentCamera
Start.CameraType = Enum.CameraType.Scriptable
local New = Instance.new("Camera")
New.Parent = workspace
task.wait()
local Degrees = {}
for D = 1, 179 do
	table.insert(Degrees, D)
end
for _, D in { 0.5, 1.5, 33.3, 44.5, 45.5, 59.5, 60.5, 88.88, 89.5, 90.5, 119.5, 120.5 } do
	table.insert(Degrees, D)
end
for _, Pair in { { "start", Start }, { "new", New } } do
	local Name, Cam = Pair[1], Pair[2]
	for _, D in Degrees do
		Cam.FieldOfView = 70
		Cam.DiagonalFieldOfView = D
		local A = Cam.FieldOfView
		Cam.FieldOfView = 70
		Cam.MaxAxisFieldOfView = D
		local B = Cam.FieldOfView
		print("write", Name, P.num(D), P.num(A), P.num(B))
	end
end
local Rng = P.rng(4242)
for _, Pair in { { "start", Start }, { "new", New } } do
	local Name, Cam = Pair[1], Pair[2]
	for I = 1, 40 do
		Cam.CFrame = CFrame.new(Rng.real(-50, 50), Rng.real(-20, 40), Rng.real(-50, 50)) * CFrame.Angles(Rng.real(-1.5, 1.5), Rng.real(-3, 3), if I % 4 == 0 then Rng.real(-1, 1) else 0)
		Cam.FieldOfView = Rng.pick({ 30, 70, 90, 110, 1, 120, 45, 61.3 })
		local Box = CFrame.new(Rng.real(-300, 300), Rng.real(-30, 30), Rng.real(-300, 300)) * CFrame.Angles(Rng.real(-3, 3), Rng.real(-3, 3), Rng.real(-3, 3))
		local Size = if I % 5 == 0 then Vector3.new(Rng.real(0, 0.6), Rng.real(0, 0.6), Rng.real(0, 0.6)) else Vector3.new(Rng.real(-10, 30), Rng.real(0, 30), Rng.real(0, 30))
		Cam:ZoomToExtents(Box, Size)
		local Focus = Cam.Focus.Position
		print("zoom", Name, I, P.f32s(Cam.CFrame:GetComponents()), "focus", P.f32s(Focus.X, Focus.Y, Focus.Z))
	end
end
Start.FieldOfView = 70
