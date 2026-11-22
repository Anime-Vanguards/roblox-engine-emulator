-- parity: device=desktop rendered
-- The start camera's members that depend on its viewport (1920 x 1080 here): DiagonalFieldOfView and MaxAxisFieldOfView
-- read and written, ZoomToExtents' distance (the wider half angle) and GetLargestCutoffDistance's corner rays
-- (world/camera_fov.rs, world/camera_moves.rs)
P.sync()
local Camera = workspace.CurrentCamera
Camera.CameraType = Enum.CameraType.Scriptable
task.wait()
print("viewport", P.f32s(Camera.ViewportSize.X, Camera.ViewportSize.Y), P.num(Camera.NearPlaneZ))
for _, F in { 1, 2, 5, 10, 15, 20, 30, 33.3, 40, 45, 50, 60, 70, 71.7, 75, 80, 90, 100, 110, 119, 120 } do
	Camera.FieldOfView = F
	print("fov", P.num(F), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
end
for _, Prop in { "DiagonalFieldOfView", "MaxAxisFieldOfView" } do
	for _, W in { 0.5, 1, 2, 5, 10, 20, 30, 40, 50, 60, 70, 80, 88.88, 90, 100, 110, 120, 130, 140, 150, 160, 170, 179, 180 } do
		Camera.FieldOfView = 70;
		(Camera :: any)[Prop] = W
		print("write", Prop, P.num(W), P.num(Camera.FieldOfView), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
	end
end
Camera.FieldOfView = 70

-- numbers to 3 decimals (a client's last float steps are not all modelled), -0 as 0
local function R(...)
	local T = table.pack(...)
	for I = 1, T.n do
		local S = string.format("%.3f", T[I])
		T[I] = if S == "-0.000" then "0.000" else S
	end
	return table.concat(T, " ")
end
local Rng = P.rng(3636)
for I = 1, 20 do
	Camera.CFrame = CFrame.new(Rng.real(-50, 50), Rng.real(-20, 40), Rng.real(-50, 50)) * CFrame.Angles(Rng.real(-1.5, 1.5), Rng.real(-3, 3), if I % 4 == 0 then Rng.real(-1, 1) else 0)
	Camera.FieldOfView = Rng.pick({ 30, 70, 90, 110, 1, 120, 45 })
	local Box = CFrame.new(Rng.real(-30, 30), Rng.real(-30, 30), Rng.real(-30, 30)) * CFrame.Angles(Rng.real(-3, 3), Rng.real(-3, 3), Rng.real(-3, 3))
	local Size = if I % 5 == 0 then Vector3.new(Rng.real(0, 0.6), Rng.real(0, 0.6), Rng.real(0, 0.6)) else Vector3.new(Rng.real(-10, 30), Rng.real(0, 30), Rng.real(0, 30))
	Camera:ZoomToExtents(Box, Size)
	print("zoom", I, R(Camera.CFrame:GetComponents()), R(Camera.Focus.Position.X, Camera.Focus.Position.Y, Camera.Focus.Position.Z))
end
Camera.FieldOfView = 70

local Parts = {}
local function Part(Size, At)
	local Made = Instance.new("Part")
	Made.Anchored = true
	Made.Size = Size
	Made.CFrame = At
	Made.Parent = workspace
	table.insert(Parts, Made)
	return Made
end
local function Clear()
	for _, Made in Parts do
		Made:Destroy()
	end
	table.clear(Parts)
end
-- (200 studs out: a client's Workspace holds the player's character near the origin)
local O = Vector3.new(200, 100, 200)
local function Sync()
	workspace:Raycast(Vector3.new(10000, 10000, 10000), Vector3.new(0, 1, 0))
end
Camera.CFrame = CFrame.lookAt(O + Vector3.new(0, 5, 30), O + Vector3.new(0, 5, 0))
Camera.Focus = CFrame.new(O + Vector3.new(0, 5, 0))
Part(Vector3.new(60, 60, 1), CFrame.new(O + Vector3.new(0, 5, 10)))
Sync()
print("cutoff wall", R(Camera:GetLargestCutoffDistance({})))
Clear()
-- a plate 0.1 thick with an edge each side of the corner rays: the near plane is 0.1 out, 0.0700 high and 0.1245 wide
-- each way
Camera.CFrame = CFrame.lookAt(O + Vector3.new(0, 0, 40), O)
Camera.Focus = CFrame.new(O)
for _, X in { 0.115, 0.122, 0.127, 0.134 } do
	-- reaching out from X, then in from X
	Part(Vector3.new(0.1, 10, 0.1), CFrame.new(O + Vector3.new(X + 0.05, 0, 2)))
	Sync()
	print("cutoff plate from x", P.num(X), R(Camera:GetLargestCutoffDistance({})))
	Clear()
	Part(Vector3.new(0.1, 10, 0.1), CFrame.new(O + Vector3.new(X - 0.05, 0, 2)))
	Sync()
	print("cutoff plate to x", P.num(X), R(Camera:GetLargestCutoffDistance({})))
	Clear()
end
for _, Y in { 0.065, 0.069, 0.071, 0.075 } do
	Part(Vector3.new(10, 0.1, 0.1), CFrame.new(O + Vector3.new(0, Y + 0.05, 2)))
	Sync()
	print("cutoff plate from y", P.num(Y), R(Camera:GetLargestCutoffDistance({})))
	Clear()
end
