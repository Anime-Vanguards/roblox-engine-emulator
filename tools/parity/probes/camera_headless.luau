-- A server's cameras (world/camera.rs viewport_size): the start camera and a script's cameras read 1 x 1 at NearPlaneZ -0.5, a
-- camera the engine makes for a CurrentCamera that left the Workspace or was destroyed 800 x 600, one it makes for a
-- CurrentCamera written nil 1 x 1. (A headless client's read the same; ree's client has a screen, as a rendered client does, so
-- only a server prints them: camera_current_screen.luau has the rendered client's.)
P.sync()
if not game:GetService("RunService"):IsServer() then
	print("a client")
	return
end
local Start = workspace.CurrentCamera
local function V(Camera)
	if not Camera then
		return "nil"
	end
	return Camera.Name .. " " .. P.f32s(Camera.ViewportSize.X, Camera.ViewportSize.Y, Camera.NearPlaneZ, Camera.FieldOfView)
end
print("start", V(Start), Start and Start.Parent == workspace)
local Made = Instance.new("Camera")
print("new", V(Made))
Made.Parent = workspace
workspace.CurrentCamera = Made
print("made current", V(Made), "start", V(Start), Start.Parent)
task.wait()
task.wait()
print("made current frames", V(Made))
-- written nil
Made.Parent = nil
workspace.CurrentCamera = nil
print("nil", V(workspace.CurrentCamera))
task.wait()
local Engine = workspace.CurrentCamera
print("for nil", V(Engine), Engine ~= Made, Engine ~= Start, Engine and Engine.Parent == workspace)
-- destroyed
local Destroyed = Instance.new("Camera")
Destroyed.Parent = workspace
workspace.CurrentCamera = Destroyed
task.wait()
Destroyed:Destroy()
task.wait()
local First = workspace.CurrentCamera
print("for destroyed", V(First), First ~= Destroyed, First ~= Engine)
for _ = 1, 5 do
	task.wait()
end
print("for destroyed frames", V(First))
-- taken out of the Workspace
First.Parent = nil
task.wait()
local Second = workspace.CurrentCamera
print("for taken out", V(Second), Second ~= First)
-- put the session's start camera back
Start.Parent = workspace
workspace.CurrentCamera = Start
for _, Camera in { Second, First, Engine, Made } do
	if Camera then
		Camera:Destroy()
	end
end
task.wait()
print("restored", V(workspace.CurrentCamera), workspace.CurrentCamera == Start)
