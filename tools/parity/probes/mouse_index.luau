-- The PlayerMouse's members and the spatial index (world/mouse.rs): whether reading one takes a part that just entered the
-- Workspace into the index, as a Raycast does, seen through Camera:GetLargestCutoffDistance, which does not see such a part
-- until the index has it (camera_cutoff.luau). Also whether Target sees such a part at once. The scenes are 200 studs out.
P.sync()
local O = Vector3.new(200, 100, 200)
local function R(Value)
	local S = string.format("%.3f", Value)
	return if S == "-0.000" then "0.000" else S
end
local M = game:GetService("Players").LocalPlayer:GetMouse()
local Cam = Instance.new("Camera")
Cam.Parent = workspace
Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 5, 30), O + Vector3.new(0, 5, 0))
Cam.Focus = CFrame.new(O + Vector3.new(0, 5, 0))
local Parts = {}
local function Wall(Z)
	local Made = Instance.new("Part")
	Made.Anchored = true
	Made.Size = Vector3.new(60, 60, 1)
	Made.CFrame = CFrame.new(O + Vector3.new(0, 5, Z))
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
local function Cut(Name)
	print("cut", Name, R(Cam:GetLargestCutoffDistance({})))
end
Cut("empty")
Wall(12)
Cut("new")
Clear()
task.wait()
Wall(12)
workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.new(0, 1, 0))
Cut("new, after a Raycast")
for _, Member in { "X", "UnitRay", "Origin", "Icon", "TargetFilter", "Hit", "hit", "Target", "target", "TargetSurface" } do
	Clear()
	task.wait()
	Wall(12)
	local _ = (M :: any)[Member]
	Cut("new, after Mouse." .. Member)
end
-- with a TargetFilter, and with the new part as the TargetFilter
Clear()
task.wait()
local Folder = Instance.new("Folder")
Folder.Parent = workspace
M.TargetFilter = Folder
Wall(12)
local _ = M.Hit
Cut("new, after Mouse.Hit with a TargetFilter")
Clear()
task.wait()
M.TargetFilter = Wall(12)
local _ = M.Hit
Cut("new, after Mouse.Hit, the new part the TargetFilter")
M.TargetFilter = nil
Folder:Destroy()
Clear()
task.wait()
-- a part entering the Workspace across the mouse's ray is its Target at once, and the index then has it
local Ray = M.UnitRay
local Across = Instance.new("Part")
Across.Anchored = true
Across.Size = Vector3.new(100, 100, 1)
Across.CFrame = CFrame.lookAt(Ray.Origin + Ray.Direction * 50, Ray.Origin + Ray.Direction * 51)
Across.Parent = workspace
table.insert(Parts, Across)
Wall(12)
print("target across, new", M.Target == Across, M.TargetSurface)
Cut("new, after Mouse.Target hit a new part")
workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.new(0, 1, 0))
print("target across, after a Raycast", M.Target == Across, M.TargetSurface)
Clear()
task.wait()
Across = Instance.new("Part")
Across.Anchored = true
Across.Size = Vector3.new(100, 100, 1)
Across.CFrame = CFrame.lookAt(Ray.Origin + Ray.Direction * 50, Ray.Origin + Ray.Direction * 51)
Across.Parent = workspace
table.insert(Parts, Across)
print("target across, new again", M.Target == Across)
task.wait()
Ray = M.UnitRay
Across.CFrame = CFrame.lookAt(Ray.Origin + Ray.Direction * 50, Ray.Origin + Ray.Direction * 51)
print("target across, a frame later", M.Target == Across, M.TargetSurface)
Across.CFrame = CFrame.lookAt(Ray.Origin + Ray.Direction * 40, Ray.Origin + Ray.Direction * 39)
print("target across, moved and turned", M.Target == Across, M.TargetSurface)
Across.Parent = nil
print("target across, removed", M.Target == Across)
Across.Parent = workspace
print("target across, back", M.Target == Across)
local _ = M.Hit
print("target across, back, after Mouse.Hit", M.Target == Across)
Across.CanQuery = false
print("target across, back, canquery off", M.Target == Across)
Clear()
task.wait()
Cut("cleared")
