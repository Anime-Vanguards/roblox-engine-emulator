-- Workspace.CurrentCamera (world/camera.rs): a write takes at once, a camera outside the Workspace or a destroyed one too, and
-- when it changes the current camera every other Camera child of the Workspace is parented to nil (one in a Folder stays);
-- nil reads nil until the simulation step, after the PostSimulation handlers, makes the Workspace's first Camera child current
-- or puts a new Camera in the Workspace; a current camera that leaves the Workspace gives way there too; the errors. Each
-- event is logged with the frame event whose handlers ran last (those a server has too). The viewport each camera gets is
-- camera_current_screen.luau's (a headless client and a server have no window).
P.sync()
local RunService = game:GetService("RunService")
local Replicated = game:GetService("ReplicatedStorage")
local Start = workspace.CurrentCamera
local Log = {}
local Phase = "script"
local Tracking = false
local Last = ""
local Names = { [Start] = "Start" }
local Made = 0
local function Name(Cam)
	if Cam == nil then
		return "nil"
	end
	return Names[Cam] or "?"
end
local function Where(Cam)
	if Cam == nil then
		return "nil"
	end
	return `{Name(Cam)}<{if Cam.Parent then Names[Cam.Parent] or Cam.Parent.Name else "nil"}>`
end
local function Mark(Text)
	if Tracking then
		table.insert(Log, `{Phase}: {Text}`)
	end
end
local function Members(Cam)
	return `{Cam.Name} {Cam.ClassName} {P.f32s(Cam.CFrame:GetComponents())} focus {P.f32s(Cam.Focus:GetComponents())} fov {P.num(Cam.FieldOfView)}`
end
local function Hook(Label)
	return function()
		Phase = Label
		local Now = `current {Where(workspace.CurrentCamera)}`
		if Now ~= Last then
			Mark(Now)
			Last = Now
		end
	end
end
RunService.PreAnimation:Connect(Hook("preAnimation"))
RunService.PreSimulation:Connect(Hook("preSimulation"))
RunService.PostSimulation:Connect(Hook("postSimulation"))
RunService.Heartbeat:Connect(Hook("heartbeat"))
local function Track(Cam, Label)
	Names[Cam] = Label
	Cam:GetPropertyChangedSignal("Parent"):Connect(function()
		Mark(`{Label} Parent {if Cam.Parent then Names[Cam.Parent] or Cam.Parent.Name else "nil"}`)
	end)
	Cam.Destroying:Connect(function()
		Mark(`{Label} Destroying`)
	end)
end
Track(Start, "Start")
workspace.Changed:Connect(function(Prop)
	if Prop == "CurrentCamera" then
		Mark(`Workspace Changed CurrentCamera {Name(workspace.CurrentCamera)}`)
	end
end)
workspace:GetPropertyChangedSignal("CurrentCamera"):Connect(function()
	Mark(`Workspace CurrentCamera signal {Name(workspace.CurrentCamera)}`)
end)
workspace.ChildAdded:Connect(function(Child)
	if Child:IsA("Camera") then
		if not Names[Child] then
			Made += 1
			Track(Child, `Made{Made}`)
			Mark(`made {Members(Child)}`)
		end
		Mark(`Workspace ChildAdded {Name(Child)}`)
	end
end)
workspace.ChildRemoved:Connect(function(Child)
	if Child:IsA("Camera") then
		Mark(`Workspace ChildRemoved {Name(Child)}`)
	end
end)
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
	Phase = "script"
end
local Folder = Instance.new("Folder")
Folder.Name = "Folder"
Folder.Parent = workspace
Names[Folder] = "Folder"
Names[workspace] = "Workspace"
Names[Replicated] = "ReplicatedStorage"
local function Cameras()
	local List = {}
	for _, Holder in { workspace, Folder } do
		for _, Child in Holder:GetChildren() do
			if Child:IsA("Camera") then
				table.insert(List, Where(Child))
			end
		end
	end
	return table.concat(List, " ")
end
local function Section(Label, Write)
	Frames(1)
	Last = ""
	Tracking = true
	Phase = "script"
	local Ok, Error = pcall(Write)
	Mark(`wrote {if Ok then "ok" else P.err(Error)}: current {Where(workspace.CurrentCamera)} start {Where(Start)}`)
	Frames(3)
	Tracking = false
	print(Label, table.concat(Log, " | "))
	table.clear(Log)
	print(Label, "after: current", Where(workspace.CurrentCamera), "cameras", Cameras())
end
local function Camera(Label, Parent)
	local Cam = Instance.new("Camera")
	Track(Cam, Label)
	Cam.Parent = Parent
	return Cam
end

print("start", Where(Start), Start.Name)
local New = Camera("New", workspace)
Section("new in the workspace", function()
	workspace.CurrentCamera = New
end)
print("start parent write", pcall(function()
	Start.Parent = workspace
end))
Section("back to the start camera", function()
	workspace.CurrentCamera = Start
end)
New.Parent = workspace
Section("the current camera again", function()
	workspace.CurrentCamera = Start
end)
local Loose = Camera("Loose", nil)
Section("a camera in no parent", function()
	workspace.CurrentCamera = Loose
end)
local Rep = Camera("Rep", Replicated)
Section("a camera in ReplicatedStorage", function()
	workspace.CurrentCamera = Rep
end)
local Nested = Camera("Nested", Folder)
local Nested2 = Camera("Nested2", Folder)
local Side = Camera("Side", workspace)
Section("a camera in a Folder", function()
	workspace.CurrentCamera = Nested
end)
local A = Camera("A", workspace)
local B = Camera("B", workspace)
Section("nil beside two Workspace cameras", function()
	workspace.CurrentCamera = nil
end)
Section("nil beside cameras in a Folder only", function()
	workspace.CurrentCamera = nil
	A.Parent = Replicated
end)
local Current = workspace.CurrentCamera
Section("the current camera destroyed", function()
	Current:Destroy()
end)
Current = workspace.CurrentCamera
Section("the current camera into ReplicatedStorage", function()
	Current.Parent = Replicated
end)
Current = workspace.CurrentCamera
Section("the current camera to nil", function()
	Current.Parent = nil
end)
local Gone = Camera("Gone", workspace)
Gone:Destroy()
Section("a destroyed camera", function()
	workspace.CurrentCamera = Gone
end)
local InFolder = Camera("InFolder", Folder)
Section("a Folder's camera, then a Workspace camera", function()
	workspace.CurrentCamera = InFolder
	workspace.CurrentCamera = Camera("Top", workspace)
end)
for _, Case in { { "Part", Instance.new("Part") }, { "Workspace", workspace }, { "Folder", Folder }, { "string", "x" }, { "number", 5 }, { "boolean", true }, { "table", {} }, { "CFrame", CFrame.new() }, { "Vector3", Vector3.zero }, { "EnumItem", Enum.CameraType.Fixed }, { "function", print } } do
	print("assign", Case[1], pcall(function()
		workspace.CurrentCamera = Case[2]
	end))
end
print("end: current", Where(workspace.CurrentCamera), "cameras", Cameras())
