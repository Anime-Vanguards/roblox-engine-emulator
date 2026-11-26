-- parity: device=desktop rendered
-- The viewport a camera made the CurrentCamera gets (world/camera.rs): 1 x 1 and a near plane 0.5 out until the render step
-- after the PreRender handlers gives it the window and a near plane 0.1 out (ViewportSize's Changed, then NearPlaneZ's), for
-- good and wherever it is; the camera the engine makes has the window at once and its near plane 0.1 out from the next render
-- step; a camera with the window takes GuiService's inset in its screen forms. Each change is logged with the frame event
-- whose handlers ran last.
P.sync()
local RunService = game:GetService("RunService")
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
local function State(Cam)
	if Cam == nil then
		return "nil"
	end
	return `{Name(Cam)}[{if Cam.Parent then Cam.Parent.Name else "nil"} {P.f32s(Cam.ViewportSize.X, Cam.ViewportSize.Y)} {P.num(Cam.NearPlaneZ)}]`
end
local function Mark(Text)
	if Tracking then
		table.insert(Log, `{Phase}: {Text}`)
	end
end
local Watch = Start
local function Hook(Label)
	return function()
		Phase = Label
		local Now = `current {State(workspace.CurrentCamera)} watch {State(Watch)}`
		if Now ~= Last then
			Mark(Now)
			Last = Now
		end
	end
end
RunService:BindToRenderStep("cameraCurrentFirst", Enum.RenderPriority.First.Value, Hook("bindFirst"))
RunService:BindToRenderStep("cameraCurrentLast", Enum.RenderPriority.Last.Value, Hook("bindLast"))
RunService.RenderStepped:Connect(Hook("renderStepped"))
RunService.PreRender:Connect(Hook("preRender"))
RunService.PreAnimation:Connect(Hook("preAnimation"))
RunService.PostSimulation:Connect(Hook("postSimulation"))
RunService.Heartbeat:Connect(Hook("heartbeat"))
local function Track(Cam, Label)
	Names[Cam] = Label
	Cam.Changed:Connect(function(Prop)
		if Prop == "ViewportSize" or Prop == "NearPlaneZ" then
			Mark(`{Label} Changed {Prop}`)
		end
	end)
end
Track(Start, "Start")
workspace.ChildAdded:Connect(function(Child)
	if Child:IsA("Camera") and not Names[Child] then
		Made += 1
		Track(Child, `Made{Made}`)
		Mark(`made {State(Child)}`)
	end
end)
local function Frames(Count)
	for _ = 1, Count do
		RunService.Heartbeat:Wait()
	end
	Phase = "script"
end
local function Section(Label, Target, Write)
	Watch = Target
	Frames(1)
	Last = ""
	Tracking = true
	Phase = "script"
	Write()
	Mark(`wrote: current {State(workspace.CurrentCamera)} start {State(Start)} watch {State(Target)}`)
	Frames(3)
	Tracking = false
	print(Label, table.concat(Log, " | "))
	table.clear(Log)
	print(Label, "after: current", State(workspace.CurrentCamera), "start", State(Start), "watch", State(Target))
end

print("start", State(Start))
local New = Instance.new("Camera")
Track(New, "New")
New.Parent = workspace
Section("new in the workspace", New, function()
	workspace.CurrentCamera = New
end)
Section("back to the start camera", New, function()
	workspace.CurrentCamera = Start
end)
local Loose = Instance.new("Camera")
Track(Loose, "Loose")
Section("a camera in no parent", Loose, function()
	workspace.CurrentCamera = Loose
end)
local Current = workspace.CurrentCamera
Section("the engine's camera to nil", Current, function()
	Current.Parent = nil
end)

-- the screen forms of a camera that got the window (New: measured from below GuiService's inset) and of one that never did
-- (its screen forms are its viewport forms); the relations are printed, since the inset is the host's
local Inset = game:GetService("GuiService"):GetGuiInset()
local Never = Instance.new("Camera")
Never.Parent = workspace
for _, Pair in { { "New", New, Inset.Y, "at y + inset" }, { "never current", Never, 0, "at y" } } do
	local Cam, Shift, Shown = Pair[2], Pair[3], Pair[4]
	Cam.CFrame = CFrame.lookAt(Vector3.new(200, 105, 230), Vector3.new(200, 105, 200))
	for _, XY in { { 0, 0 }, { 300, 100 }, { 960, 482 } } do
		local X, Y = XY[1], XY[2]
		local S = Cam:ScreenPointToRay(X, Y, 2)
		local Shifted = Cam:ViewportPointToRay(X, Y + Shift, 2)
		print("screen", Pair[1], P.num(X), P.num(Y), "is the viewport ray", Shown, tostring(S.Origin == Shifted.Origin and S.Direction == Shifted.Direction))
	end
	local W = Vector3.new(203, 112, 190)
	local S = Cam:WorldToScreenPoint(W)
	local V = Cam:WorldToViewportPoint(W)
	print("screen", Pair[1], "point is the viewport point less", Shown, tostring(S == Vector3.new(V.X, V.Y - Shift, V.Z)), P.f32s(V.X, V.Y, V.Z))
end
