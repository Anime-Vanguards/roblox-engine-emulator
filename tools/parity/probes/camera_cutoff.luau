-- Camera:GetLargestCutoffDistance on a camera that is not the start camera (a 1 x 1 viewport, its near plane 0.5 out): which
-- parts block its five rays, and when a part entering the Workspace is seen (world/camera_moves.rs). Distances to 3
-- decimals. The scenes are 200 studs out: a client's Workspace holds the player's character near the origin.
P.sync()
local O = Vector3.new(200, 100, 200)
local function R(Value)
	local S = string.format("%.3f", Value)
	return if S == "-0.000" then "0.000" else S
end
local RunService = game:GetService("RunService")
local Cam = Instance.new("Camera")
Cam.Parent = workspace
local Parts = {}
local function Part(Props)
	local Made = Instance.new("Part")
	Made.Anchored = true
	for _, Pair in Props do
		(Made :: any)[Pair[1]] = Pair[2]
	end
	table.insert(Parts, Made)
	return Made
end
local function Clear()
	for _, Made in Parts do
		Made:Destroy()
	end
	table.clear(Parts)
end
local function Cut(Name, Ignore)
	local Results = table.pack(pcall(Cam.GetLargestCutoffDistance, Cam, Ignore or {}))
	print("cut", Name, Results[1], if Results[1] then R(Results[2]) else P.err(Results[2]), Results.n)
end
Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 5, 30), O + Vector3.new(0, 5, 0))
Cam.Focus = CFrame.new(O + Vector3.new(0, 5, 0))
Cut("empty")
local Wall = Part({ { "Size", Vector3.new(60, 60, 1) }, { "CFrame", CFrame.new(O + Vector3.new(0, 5, 10)) }, { "Parent", workspace } })
task.wait()
Cut("wall")
Cut("wall ignored", { Wall })
local Model = Instance.new("Model")
Model.Parent = workspace
Wall.Parent = Model
Cut("wall in a model, the model ignored", { Model })
Cut("wall in a model, the model ignored twice", { Model, Model, Wall })
Wall.Parent = workspace
Model:Destroy()
for _, T in table.clone({ 0.5, 0.95, 0.9500001, 0.951, 1 }) do
	Wall.Transparency = T
	Cut("transparency " .. P.num(T))
end
Wall.Transparency = 0
Wall.CanCollide = false
Cut("cancollide off")
Wall.CanQuery = false
Cut("cancollide off, canquery off")
Wall.CanCollide = true
Cut("canquery off")
Wall.CanQuery = true
Wall.CanTouch = false
Cut("cantouch off")
Wall.CanTouch = true
-- moved, resized, removed: at once
Wall.CFrame = CFrame.new(O + Vector3.new(0, 5, 20))
Cut("moved")
Wall.Size = Vector3.new(60, 60, 3)
Cut("resized")
Wall.CFrame = CFrame.new(O + Vector3.new(0, 5, -10))
Cut("behind the focus")
Wall.CFrame = CFrame.new(O + Vector3.new(0, 5, 10))
Wall.Parent = nil
Cut("removed")
-- back in the Workspace: not seen until the spatial index takes it in
Wall.Parent = workspace
Cut("back")
task.wait()
Cut("back, a frame later")
Clear()
task.wait()
local Fresh = Part({ { "Size", Vector3.new(60, 60, 1) }, { "CFrame", CFrame.new(O + Vector3.new(0, 5, 12)) }, { "Parent", workspace } })
Cut("new")
Cut("new, again")
workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.new(0, 1, 0))
Cut("new, after a Raycast")
Clear()
task.wait()
Part({ { "Size", Vector3.new(60, 60, 1) }, { "CFrame", CFrame.new(O + Vector3.new(0, 5, 14)) }, { "Parent", workspace } })
workspace:GetPartBoundsInBox(CFrame.new(1000, 1000, 1000), Vector3.one)
Cut("new, after GetPartBoundsInBox")
Clear()
task.wait()
-- (task.wait resumes after the physics step: a Heartbeat in the same frame comes before the next one)
Part({ { "Size", Vector3.new(60, 60, 1) }, { "CFrame", CFrame.new(O + Vector3.new(0, 5, 16)) }, { "Parent", workspace } })
RunService.Heartbeat:Wait()
Cut("new, at the Heartbeat")
task.wait()
Cut("new, a frame later")
Clear()
task.wait()
Fresh = Part({ { "Size", Vector3.new(60, 60, 1) }, { "CFrame", CFrame.new(O + Vector3.new(0, 5, 18)) }, { "Parent", workspace } })
Fresh.CFrame = CFrame.new(O + Vector3.new(0, 5, 8))
Cut("new and moved")
task.wait()
Cut("new and moved, a frame later")
local Folder = Instance.new("Folder")
Folder.Parent = workspace
Fresh.Parent = Folder
Cut("moved into a folder in the Workspace")
Clear()
Folder:Destroy()
task.wait()

-- random blocks between Focus and the camera
local Rng = P.rng(3600)
for I = 1, 30 do
	local Eye = O + Vector3.new(Rng.real(-20, 20), Rng.real(-10, 20), Rng.real(-20, 20))
	local Target = O + Vector3.new(Rng.real(-5, 5), Rng.real(-5, 5), Rng.real(-5, 5))
	Cam.CFrame = CFrame.lookAt(Eye, Eye + (Target - Eye).Unit * 10) * (if I % 3 == 0 then CFrame.Angles(0, 0, Rng.real(-1, 1)) else CFrame.identity)
	local FocusAt = if I % 4 == 0 then Target + Vector3.new(Rng.real(-3, 3), Rng.real(-3, 3), Rng.real(-3, 3)) else Target
	Cam.Focus = CFrame.new(FocusAt)
	Cam.FieldOfView = Rng.pick({ 70, 120, 30, 90 })
	for J = 1, Rng.int(1, 3) do
		local At = FocusAt:Lerp(Eye, Rng.real(0.05, 0.95)) + Vector3.new(Rng.real(-1, 1), Rng.real(-1, 1), Rng.real(-1, 1)) * (if J == 1 then 0.2 else 1)
		Part({ { "Size", Vector3.new(Rng.real(0.2, 4), Rng.real(0.2, 4), Rng.real(0.2, 4)) }, { "CFrame", CFrame.new(At) * CFrame.Angles(Rng.real(-3, 3), Rng.real(-3, 3), Rng.real(-3, 3)) }, { "Parent", workspace } })
	end
	task.wait()
	Cut("random " .. I)
	Clear()
end

-- the rays are parallel: from Focus moved by each near-plane corner's offset (0.866 each way at 120 degrees)
Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 0, 40), O)
Cam.Focus = CFrame.new(O)
Cam.FieldOfView = 120
for _, Offset in { Vector3.new(0, 0, 0), Vector3.new(0.6, 0.6, 0), Vector3.new(-0.85, 0.85, 0), Vector3.new(0.85, -0.85, 0), Vector3.new(0.85, 0, 0) } do
	for _, Z in { 1, 20, 39 } do
		Part({ { "Size", Vector3.new(0.1, 0.1, 0.1) }, { "CFrame", CFrame.new(O + Offset + Vector3.new(0, 0, Z)) }, { "Parent", workspace } })
		task.wait()
		Cut("corner " .. R(Offset.X) .. " " .. R(Offset.Y) .. " z " .. Z)
		Clear()
		Part({ { "Size", Vector3.new(0.1, 0.1, 0.1) }, { "CFrame", CFrame.new(O + Offset * (Z / 40) + Vector3.new(0, 0, Z)) }, { "Parent", workspace } })
		task.wait()
		Cut("converging " .. R(Offset.X) .. " " .. R(Offset.Y) .. " z " .. Z)
		Clear()
	end
end
-- Focus inside a part (a ray starting inside one does not hit it), the camera inside one
Part({ { "Size", Vector3.new(4, 4, 4) }, { "CFrame", CFrame.new(O) }, { "Parent", workspace } })
task.wait()
Cut("focus inside")
Parts[1].CFrame = CFrame.new(O + Vector3.new(0, 0, 40))
Cut("camera inside")
Clear()
Cam.FieldOfView = 70

-- the argument
print("cut arg none", P.try(Cam.GetLargestCutoffDistance, Cam))
print("cut arg nil", P.try(Cam.GetLargestCutoffDistance, Cam, nil))
print("cut arg number", P.try(Cam.GetLargestCutoffDistance, Cam, 5))
print("cut arg string", P.try(Cam.GetLargestCutoffDistance, Cam, "x"))
print("cut arg instance", P.try(Cam.GetLargestCutoffDistance, Cam, workspace))
print("cut arg mixed", P.try(Cam.GetLargestCutoffDistance, Cam, { workspace, 5 }))
print("cut arg dictionary", P.try(Cam.GetLargestCutoffDistance, Cam, { a = workspace }))
local Loose = Instance.new("Camera")
print("cut loose", P.try(Loose.GetLargestCutoffDistance, Loose, {}))

-- GetPartsObscuringTarget's arguments: the ignore list first (none given "Argument 2 missing or nil", any value but a table
-- "Expected table argument, got <type()>"), then the points as an Array (named as world/camera.rs cast_name names values);
-- and GetLargestCutoffDistance's ignore list, checked the same way
do
	local function Try(F, ...)
		local Out = table.pack(pcall(F, ...))
		if not Out[1] then
			return "error " .. P.err(Out[2])
		end
		local Parts = { "ok" }
		for I = 2, Out.n do
			local V = Out[I]
			table.insert(Parts, if type(V) == "table" then "table " .. #V elseif type(V) == "number" then R(V) else typeof(V))
		end
		return table.concat(Parts, " ")
	end
	local Kinds = {
		{ "string", "x" }, { "true", true }, { "number", 5 }, { "array", { 1 } }, { "empty-table", {} }, { "dictionary", { a = 1 } },
		{ "Instance", workspace }, { "Vector3", Vector3.one }, { "Vector2", Vector2.one }, { "CFrame", CFrame.new() },
		{ "KeyCode", Enum.KeyCode.A }, { "Enum", Enum.KeyCode }, { "function", print }, { "thread", coroutine.create(function() end) },
		{ "buffer", buffer.create(1) }, { "userdata", newproxy() }, { "RBXScriptSignal", workspace.Changed },
	}
	local Obscurer = Instance.new("Camera")
	Obscurer.CFrame = CFrame.new(O + Vector3.new(0, 50, 0))
	Obscurer.Parent = workspace
	local V = O + Vector3.new(0, 50, -5)
	local F = Obscurer.GetPartsObscuringTarget
	for _, K in Kinds do
		local Name, X = K[1], K[2]
		print("obscuring cast", Name, "points", Try(F, Obscurer, X, {}), "ignore", Try(F, Obscurer, { V }, X), "both", Try(F, Obscurer, X, X))
		print("cut cast", Name, Try(Cam.GetLargestCutoffDistance, Cam, X))
	end
	print("obscuring args", "none", Try(F, Obscurer), "nil nil", Try(F, Obscurer, nil, nil), "points only", Try(F, Obscurer, { V }), "nil ignore", Try(F, Obscurer, nil, {}), "extra", Try(F, Obscurer, { V }, {}, workspace))
	print("obscuring ignore entries", "mixed", Try(F, Obscurer, { V }, { workspace, 5 }), "hole", Try(F, Obscurer, { V }, { workspace, nil, workspace }), "vector", Try(F, Obscurer, { V }, { Vector3.one }), "enum", Try(F, Obscurer, { V }, { Enum.KeyCode.A }), "string", Try(F, Obscurer, { V }, { "x" }))
	print("obscuring order", "bad points and entry", Try(F, Obscurer, "x", { 1 }), "dictionary points, bad ignore", Try(F, Obscurer, { a = V }, "x"), "bad points, nil ignore", Try(F, Obscurer, workspace, nil))
	local Current = workspace.CurrentCamera
	print("obscuring current", "none", Try(Current.GetPartsObscuringTarget, Current), "Instance", Try(Current.GetPartsObscuringTarget, Current, {}, workspace))
end
