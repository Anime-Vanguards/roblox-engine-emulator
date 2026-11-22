-- Camera:PanUnits, TiltUnits and ZoomToExtents on cameras that are not the start camera, and Interpolate's checks
-- (world/camera_moves.rs). Positions and rotations are printed to 3 decimals: a client's last float steps there are not
-- all modelled.
P.sync()
local function R(...)
	local T = table.pack(...)
	for I = 1, T.n do
		local S = string.format("%.3f", T[I])
		T[I] = if S == "-0.000" then "0.000" else S
	end
	return table.concat(T, " ")
end
local function C(Frame)
	return R(Frame:GetComponents())
end
local function Direction(Yaw, Pitch)
	local Y, Pi = math.rad(Yaw), math.rad(Pitch)
	return Vector3.new(-math.sin(Y) * math.cos(Pi), math.sin(Pi), -math.cos(Y) * math.cos(Pi))
end
local Rng = P.rng(36)
local Cam = Instance.new("Camera")
Cam.Parent = workspace
for _, Method in { "PanUnits", "TiltUnits" } do
	for I = 1, 30 do
		local Pos = Vector3.new(Rng.real(-50, 50), Rng.real(-20, 40), Rng.real(-50, 50))
		local Yaw, Pitch = Rng.real(-180, 180), Rng.real(-89, 89)
		if I <= 6 then
			-- away from the ties, near the limits
			Yaw = Rng.pick({ 22, -23, 67, 90, 180, -135, 0, 44 })
			Pitch = Rng.pick({ 4, -6, 16, 74, 86, -86, 90, -90, 0 })
		end
		local Look = Direction(Yaw, Pitch)
		local Target = Pos + Look * Rng.real(0.5, 40)
		local Frame = CFrame.lookAt(Pos, Target)
		if I % 5 == 0 then
			Frame = Frame * CFrame.Angles(0, 0, Rng.real(-1, 1))
		end
		Cam.CFrame = Frame
		local Focus = if I % 3 == 0 then Target + Vector3.new(Rng.real(-5, 5), Rng.real(-5, 5), Rng.real(-5, 5)) else Target
		Cam.Focus = if I % 4 == 0 then CFrame.new(Focus) * CFrame.Angles(Rng.real(-1, 1), Rng.real(-1, 1), Rng.real(-1, 1)) else CFrame.new(Focus)
		if I == 6 then
			Cam.Focus = Cam.CFrame
		end
		local N = Rng.pick({ -3, -2, -1, 0, 1, 2, 3, 5, 9, -9 })
		local Out = table.pack((Cam :: any)[Method](Cam, N))
		print(Method, I, N, Out.n, tostring(Out[1]), C(Cam.CFrame), C(Cam.Focus))
	end
	-- repeated steps
	Cam.CFrame = CFrame.lookAt(Vector3.new(3, 4, 20), Vector3.new(0, 0, 0))
	Cam.Focus = CFrame.new(0, 0, 0)
	for Step = 1, 12 do
		local Out = table.pack((Cam :: any)[Method](Cam, 1))
		print(Method, "step", Step, Out.n, tostring(Out[1]), C(Cam.CFrame))
	end
	-- the argument: an int, rounded half away from zero; past the int's range its least
	for _, A in { 1.5, -1.5, 2.5, -2.5, 0.5, -0.5, 0.49, 0.51, "2", "2.5", " 3 ", "0x10", "1e1", true, false, 1e10, -1e10, 2 ^ 31, 2 ^ 31 - 1, 0 / 0, math.huge, -math.huge } do
		Cam.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 0))
		Cam.Focus = CFrame.new(0, 0, 0)
		local Results = table.pack(pcall((Cam :: any)[Method], Cam, A))
		local Look = Cam.CFrame.LookVector
		local Shown = if type(A) == "number" then P.num(A) else typeof(A) .. " " .. tostring(A)
		print(Method, "arg", Shown, Results[1], Results.n - 1, tostring(Results[2]), R(math.deg(math.atan2(-Look.X, -Look.Z)), math.deg(math.asin(Look.Y))))
	end
	for _, A in { "x", "", {}, { 1 }, { a = 1 }, Vector3.one, CFrame.new(), Enum.CameraType.Fixed, workspace } do
		print(Method, "cast", typeof(A), P.try((Cam :: any)[Method], Cam, A))
	end
	print(Method, "none", P.try((Cam :: any)[Method], Cam))
	print(Method, "nil", P.try((Cam :: any)[Method], Cam, nil))
	for _, Type in { "Fixed", "Custom", "Attach", "Watch", "Track", "Follow", "Orbital", "Scriptable" } do
		Cam.CameraType = (Enum.CameraType :: any)[Type]
		Cam.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 0))
		Cam.Focus = CFrame.new(0, 0, 0)
		print(Method, "type", Type, P.try((Cam :: any)[Method], Cam, 1), C(Cam.CFrame))
	end
	Cam.CameraType = Enum.CameraType.Fixed
end
-- TiltUnits(0): true when the pitch moves to its step (a rolled camera looking level keeps it)
for _, Case in { { 0, 0 }, { 0, 0.5 }, { 20, 0 }, { 21, 0 }, { 81, 0 } } do
	Cam.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 20) + Direction(0, Case[1])) * CFrame.Angles(0, 0, Case[2])
	Cam.Focus = CFrame.new(0, 0, 0)
	print("tilt zero", Case[1], Case[2], P.try(Cam.TiltUnits, Cam, 0), C(Cam.CFrame))
end
-- Changed: CFrame, then its alias CoordinateFrame
do
	local Log = {}
	local Connection = Cam.Changed:Connect(function(Name)
		table.insert(Log, Name)
	end)
	Cam.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 0))
	Cam.Focus = CFrame.new(0, 0, 0)
	task.wait()
	table.clear(Log)
	Cam:PanUnits(1)
	task.wait()
	print("changed pan", table.concat(Log, ", "))
	table.clear(Log)
	Cam:TiltUnits(-2)
	task.wait()
	print("changed tilt", table.concat(Log, ", "))
	table.clear(Log)
	Cam:ZoomToExtents(CFrame.new(1, 2, 3), Vector3.new(4, 5, 6))
	task.wait()
	print("changed zoom", table.concat(Log, ", "))
	Connection:Disconnect()
end

-- ZoomToExtents: the rotation kept, backed off from the box's centre; Focus the centre
for I = 1, 24 do
	Cam.CFrame = CFrame.new(Rng.real(-50, 50), Rng.real(-20, 40), Rng.real(-50, 50)) * CFrame.Angles(Rng.real(-1.5, 1.5), Rng.real(-3, 3), if I % 4 == 0 then Rng.real(-1, 1) else 0)
	Cam.Focus = CFrame.new(Rng.real(-5, 5), Rng.real(-5, 5), Rng.real(-5, 5))
	Cam.FieldOfView = Rng.pick({ 30, 70, 90, 110, 1, 120 })
	local Box = CFrame.new(Rng.real(-30, 30), Rng.real(-30, 30), Rng.real(-30, 30)) * CFrame.Angles(Rng.real(-3, 3), Rng.real(-3, 3), Rng.real(-3, 3))
	local Size = Vector3.new(Rng.real(0, 30), Rng.real(0, 30), Rng.real(0, 30))
	if I % 6 == 0 then
		Size = Vector3.new(Rng.real(0, 0.6), Rng.real(0, 0.6), Rng.real(0, 0.6))
	elseif I % 6 == 1 then
		Size = Vector3.new(Rng.real(-10, 10), Rng.real(-10, 10), Rng.real(-10, 10))
	end
	local Out = table.pack(Cam:ZoomToExtents(Box, Size))
	print("zoom", I, Out.n, C(Cam.CFrame), C(Cam.Focus))
end
-- the smallest box: a radius under 0.5 counts as 0.5 (an unrotated box, the camera looking down -Z, 90 degrees)
Cam.FieldOfView = 90
for _, Size in { Vector3.zero, Vector3.new(0.4, 0.4, 0.4), Vector3.new(0.57, 0.57, 0.57), Vector3.new(0.58, 0.58, 0.58), Vector3.new(0.6, 0.6, 0.6), Vector3.new(0.98, 0, 0), Vector3.new(1.02, 0, 0), Vector3.new(-0.6, -0.6, -0.6), Vector3.new(-1, 0.2, 0.2) } do
	Cam.CFrame = CFrame.new()
	Cam:ZoomToExtents(CFrame.new(), Size)
	print("zoom small", R(Size.X, Size.Y, Size.Z), R(Cam.CFrame.Position.Z * math.sin(math.rad(45))))
end
Cam.FieldOfView = 70
for _, Case in { {}, { CFrame.new() }, { CFrame.new(), "x" }, { "x", "y" }, { Vector3.one, Vector3.one }, { CFrame.new(), CFrame.new() }, { nil, Vector3.one }, { CFrame.new(), Vector3.one, 5 }, { "1, 2, 3", Vector3.one }, { CFrame.new(), { 1, 2, 3 } } } do
	print("zoom arg", P.try(Cam.ZoomToExtents, Cam, table.unpack(Case, 1, 3)))
end

-- Interpolate's checks, from the last argument to the first, then the duration, then CameraType
local A, B = CFrame.new(1, 2, 3), CFrame.new(4, 5, 6)
local Cases = { {}, { A }, { A, B }, { A, nil, 1 }, { nil, B, 1 }, { A, B, "x" }, { A, "y", "x" }, { "z", "y", "x" }, { Vector3.one, B, 1 }, { A, Vector3.one, 1 }, { A, B, 0 }, { A, B, -1 }, { A, B, -0 }, { "z", B, 0 }, { A, B, {} }, { A, B, "0" }, { A, B, true } }
for _, Case in Cases do
	print("interpolate fixed", P.try(Cam.Interpolate, Cam, table.unpack(Case, 1, 3)))
end
for _, Type in { "Custom", "Attach", "Watch", "Track", "Follow", "Orbital" } do
	Cam.CameraType = (Enum.CameraType :: any)[Type]
	print("interpolate type", Type, P.try(Cam.Interpolate, Cam, A, B, 1))
end
Cam.CameraType = Enum.CameraType.Scriptable
for _, Case in { { A, B, 0 }, { A, B, "x" }, { A, B, 1000 } } do
	print("interpolate scriptable", P.try(Cam.Interpolate, Cam, table.unpack(Case, 1, 3)))
end
Cam.CameraType = Enum.CameraType.Fixed

-- a camera outside the Workspace pans and zooms too
local Loose = Instance.new("Camera")
Loose.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 0))
Loose.Focus = CFrame.new(0, 0, 0)
print("loose pan", P.try(Loose.PanUnits, Loose, 1), C(Loose.CFrame))
print("loose tilt", P.try(Loose.TiltUnits, Loose, 1), C(Loose.CFrame))
print("loose zoom", P.try(Loose.ZoomToExtents, Loose, CFrame.new(), Vector3.one), C(Loose.CFrame))

-- The argument casts of the projections, SetRoll, SetCameraPanMode, ZoomToExtents, Interpolate and PanUnits, a line for each
-- value and method (each argument in turn, the others valid): an Instance is named Instance, an EnumItem by its enum, a
-- function Function, a signal EventInstance, an Enum, Enums, thread or plain userdata null (world/camera.rs cast_name). A ray's
-- zero components print as 0 (their sign is not modelled).
do
	local function N(X)
		return if X == 0 then "0" else P.f32(X)
	end
	local function Show(V)
		local T = typeof(V)
		if T == "Ray" then
			return "Ray " .. table.concat({ N(V.Origin.X), N(V.Origin.Y), N(V.Origin.Z), N(V.Direction.X), N(V.Direction.Y), N(V.Direction.Z) }, " ")
		elseif T == "Vector3" then
			return table.concat({ N(V.X), N(V.Y), N(V.Z) }, " ")
		end
		return T .. " " .. tostring(V)
	end
	local function Try(F, ...)
		local Out = table.pack(pcall(F, ...))
		if not Out[1] then
			return "error " .. P.err(Out[2])
		end
		local Parts = { "ok" }
		for I = 2, Out.n do
			table.insert(Parts, Show(Out[I]))
		end
		return table.concat(Parts, " ")
	end
	local Kinds = {
		{ "string", "x" }, { "numeric-string", "0.25" }, { "true", true }, { "false", false }, { "array", { 1 } },
		{ "empty-table", {} }, { "dictionary", { a = 1 } }, { "Instance", workspace }, { "Vector3", Vector3.one },
		{ "Vector2", Vector2.one }, { "CFrame", CFrame.new() }, { "Color3", Color3.new() }, { "KeyCode", Enum.KeyCode.A },
		{ "CameraPanMode", Enum.CameraPanMode.EdgeBump }, { "Enum", Enum.KeyCode }, { "Enums", Enum }, { "function", print },
		{ "thread", coroutine.create(function() end) }, { "buffer", buffer.create(1) }, { "userdata", newproxy() },
		{ "RBXScriptSignal", workspace.Changed }, { "nan", 0 / 0 }, { "inf", math.huge }, { "-0", -0 }, { "1e40", 1e40 },
	}
	local Caster = Instance.new("Camera")
	Caster.CameraType = Enum.CameraType.Scriptable
	Caster.CFrame = CFrame.lookAt(Vector3.new(0, 5, 30), Vector3.new(0, 5, 0))
	Caster.Parent = workspace
	-- (Fixed: Interpolate casts its arguments before it checks CameraType)
	local Still = Instance.new("Camera")
	Still.Parent = workspace
	local Zoomer = Instance.new("Camera")
	Zoomer.Parent = workspace
	local From, To = CFrame.new(1, 2, 3), CFrame.new(4, 5, 6)
	local W = Vector3.new(1, 6, 0)
	for _, K in Kinds do
		local Name, X = K[1], K[2]
		for _, M in { "ScreenPointToRay", "ViewportPointToRay" } do
			local F = (Caster :: any)[M]
			print("cast", Name, M, "x", Try(F, Caster, X, 0.25), "y", Try(F, Caster, 0.25, X), "depth", Try(F, Caster, 0.25, 0.75, X))
		end
		print("cast", Name, "SetRoll", Try(Caster.SetRoll, Caster, X), P.num(Caster:GetRoll()))
		Caster:SetRoll(0)
		print("cast", Name, "WorldToScreenPoint", Try(Caster.WorldToScreenPoint, Caster, X), "WorldToViewportPoint", Try(Caster.WorldToViewportPoint, Caster, X))
		print("cast", Name, "SetCameraPanMode", Try(Caster.SetCameraPanMode, Caster, X))
		print("cast", Name, "ZoomToExtents", Try(Zoomer.ZoomToExtents, Zoomer, X, Vector3.one), "size", Try(Zoomer.ZoomToExtents, Zoomer, CFrame.new(), X))
		print("cast", Name, "Interpolate", Try(Still.Interpolate, Still, X, To, 1), "focus", Try(Still.Interpolate, Still, From, X, 1), "duration", Try(Still.Interpolate, Still, From, To, X))
		print("cast", Name, "PanUnits", Try(Still.PanUnits, Still, X))
	end
	-- missing, nil and extra arguments; the rays check theirs from the last to the first
	for _, M in { "ScreenPointToRay", "ViewportPointToRay" } do
		local F = (Caster :: any)[M]
		print("args", M, "none", Try(F, Caster), "x only", Try(F, Caster, 1), "nil nil", Try(F, Caster, nil, nil), "nil y", Try(F, Caster, nil, 1))
		print("args", M, "x nil", Try(F, Caster, 1, nil), "depth nil", Try(F, Caster, 0.25, 0.75, nil), "extra", Try(F, Caster, 0.25, 0.75, 1, "extra", workspace))
		print("args", M, "x and y bad", Try(F, Caster, "a", workspace), "x and depth bad", Try(F, Caster, workspace, 1, Enum.KeyCode.A), "all bad", Try(F, Caster, workspace, "a", Enum.KeyCode.A))
	end
	print("args SetRoll", "none", Try(Caster.SetRoll, Caster), "nil", Try(Caster.SetRoll, Caster, nil), "extra", Try(Caster.SetRoll, Caster, 0.5, workspace), P.num(Caster:GetRoll()))
	Caster:SetRoll(0)
	for _, M in { "WorldToScreenPoint", "WorldToViewportPoint" } do
		local F = (Caster :: any)[M]
		print("args", M, "none", Try(F, Caster), "nil", Try(F, Caster, nil), "extra", Try(F, Caster, W, workspace, "x"))
	end
	print("args SetCameraPanMode", "none", Try(Caster.SetCameraPanMode, Caster), "nil", Try(Caster.SetCameraPanMode, Caster, nil), "extra", Try(Caster.SetCameraPanMode, Caster, Enum.CameraPanMode.Classic, workspace))
	-- a fractional number takes the item it truncates to
	print("cast SetCameraPanMode fractional", Try(Caster.SetCameraPanMode, Caster, 0.5), Try(Caster.SetCameraPanMode, Caster, 1.99), Try(Caster.SetCameraPanMode, Caster, -0.99), Try(Caster.SetCameraPanMode, Caster, 2.5))
	-- the CurrentCamera casts as any camera does
	local Current = workspace.CurrentCamera
	print("cast current", Try(Current.ScreenPointToRay, Current, workspace, 1), Try(Current.ViewportPointToRay, Current, 1, Enum.KeyCode.A), Try(Current.WorldToScreenPoint, Current, workspace), Try(Current.WorldToViewportPoint, Current, Enum.KeyCode.A), Try(Current.SetRoll, Current, workspace), Try(Current.SetCameraPanMode, Current, workspace))
end
