-- parity: device=desktop rendered
-- Camera:Interpolate on the CurrentCamera: a Focus write ends a tween at the next render step and InterpolationFinished fires
-- after its PreRender handlers' deferred work; CameraType leaving Scriptable cancels a tween unfired; a new call replaces a
-- tween, which never fires (world/camera_moves.rs). The tweens are long and the probe prints no camera values: roblox-sim's
-- clients do not run the step that moves the camera.
P.sync()
local RunService = game:GetService("RunService")
local Cam = workspace.CurrentCamera
Cam.CameraType = Enum.CameraType.Scriptable
Cam.CFrame = CFrame.lookAt(Vector3.new(0, 10, 30), Vector3.new(0, 0, 0))
Cam.Focus = CFrame.new(0, 0, 0)
task.wait()
local Log = {}
local Render = 0
local Recording = false
local function Mark(Text)
	if Recording then
		table.insert(Log, Text)
	end
end
RunService:BindToRenderStep("cameraInterpolateProbe", Enum.RenderPriority.First.Value, function()
	Render += 1
end)
local Fired = 0
local Connections = {
	RunService.PreRender:Connect(function()
		Mark("preRender")
		task.defer(Mark, "deferred by preRender")
	end),
	Cam.InterpolationFinished:Connect(function(...)
		Fired += 1
		Mark("finished " .. select("#", ...))
	end),
}
local End, EndFocus = CFrame.lookAt(Vector3.new(30, 10, 0), Vector3.new(0, 0, 0)), CFrame.new(0, 0, 0)
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
end

-- a Focus write ends it
print("call", select("#", Cam:Interpolate(End, EndFocus, 1000)))
Frames(3)
Recording = true
local Before = Render
Cam.Focus = CFrame.new(3, 3, 3)
Mark("wrote Focus")
Frames(3)
Recording = false
print("focus write", table.concat(Log, ", "), "fired", Fired)
for Index, Line in Log do
	if string.sub(Line, 1, 8) == "finished" then
		print("finished in the render step after the write", Render > Before)
		break
	end
end
table.clear(Log)
print("focus as written", Cam.Focus == CFrame.new(3, 3, 3))
Cam.Focus = CFrame.new(4, 4, 4)
Frames(3)
print("a second write fires nothing", Fired)

-- CameraType leaving Scriptable cancels it; back to Scriptable it stays gone
Fired = 0
Cam:Interpolate(End, EndFocus, 1000)
Frames(2)
Cam.CameraType = Enum.CameraType.Fixed
Frames(2)
Cam.CameraType = Enum.CameraType.Scriptable
Cam.Focus = CFrame.new(5, 5, 5)
Frames(3)
print("cancelled", Fired)

-- a new call replaces it: one InterpolationFinished
Fired = 0
Cam:Interpolate(End, EndFocus, 1000)
Frames(2)
Cam:Interpolate(CFrame.lookAt(Vector3.new(0, 30, 10), Vector3.new(0, 0, 0)), CFrame.new(0, 1, 0), 1000)
Frames(2)
Cam.Focus = CFrame.new(6, 6, 6)
Frames(3)
print("replaced", Fired)

-- a tween on a camera that is not the CurrentCamera never fires
local Other = Instance.new("Camera")
Other.Parent = workspace
Other.CameraType = Enum.CameraType.Scriptable
local OtherFired = 0
Other.InterpolationFinished:Connect(function()
	OtherFired += 1
end)
print("other", P.try(Other.Interpolate, Other, End, EndFocus, 0.05))
Frames(6)
Other.Focus = CFrame.new(1, 1, 1)
Frames(3)
print("other fired", OtherFired)
for _, Connection in Connections do
	Connection:Disconnect()
end
RunService:UnbindFromRenderStep("cameraInterpolateProbe")
