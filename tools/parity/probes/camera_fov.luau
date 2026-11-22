-- Camera.DiagonalFieldOfView and MaxAxisFieldOfView on a camera that is not the start camera (a 1 x 1 viewport): what they
-- read for each FieldOfView, what a write of either sets FieldOfView to (through the radians a client keeps), the order of
-- the three members' Changed events, and FieldOfViewMode (world/camera_fov.rs)
P.sync()
local Camera = Instance.new("Camera")
Camera.Parent = workspace
print("default", P.num(Camera.FieldOfView), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
for _, F in { 1, 2, 5, 10, 15, 20, 30, 33.3, 40, 45, 50, 60, 70, 71.7, 75, 80, 90, 100, 110, 119, 120 } do
	Camera.FieldOfView = F
	print("fov", P.num(F), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
end
for _, Prop in { "DiagonalFieldOfView", "MaxAxisFieldOfView" } do
	for _, W in table.clone({ 0.5, 1, 5, 10, 30, 50, 60, 70, 80, 88.88, 90, 100, 110, 120, 130, 140, 150, 160, 170, 179, 180, 200, 0, -10, 0 / 0, math.huge, "50", "x", true, {}, Vector3.one }) do
		Camera.FieldOfView = 70
		local Ok, Problem = pcall(function()
			(Camera :: any)[Prop] = W
		end)
		local Shown = if type(W) == "number" then P.num(W) else typeof(W)
		print("write", Prop, Shown, Ok, Ok and "" or P.err(Problem), P.num(Camera.FieldOfView), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
	end
end
-- a member written its own value
for _, F in { 30, 60, 90, 110 } do
	for _, Prop in { "DiagonalFieldOfView", "MaxAxisFieldOfView" } do
		Camera.FieldOfView = F;
		(Camera :: any)[Prop] = (Camera :: any)[Prop]
		print("self", P.num(F), Prop, P.num(Camera.FieldOfView), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
	end
end
-- Changed: the member written first, then FieldOfView, then the third; nothing for a write that changes nothing
Camera.FieldOfView = 70
task.wait()
local Log = {}
local Connections = {
	Camera.Changed:Connect(function(Name)
		table.insert(Log, Name)
	end),
}
for _, Prop in { "FieldOfView", "DiagonalFieldOfView", "MaxAxisFieldOfView" } do
	table.insert(Connections, Camera:GetPropertyChangedSignal(Prop):Connect(function()
		table.insert(Log, "signal " .. Prop)
	end))
end
for _, Step in { { "FieldOfView", 50 }, { "DiagonalFieldOfView", 60 }, { "MaxAxisFieldOfView", 60 }, { "FieldOfView", 60 }, { "MaxAxisFieldOfView", 60 }, { "DiagonalFieldOfView", 88.876533508300781 }, { "DiagonalFieldOfView", 0 / 0 }, { "MaxAxisFieldOfView", 500 }, { "MaxAxisFieldOfView", 500 } } do
	(Camera :: any)[Step[1]] = Step[2]
	task.wait()
	print("changed", Step[1], P.num(Step[2]), table.concat(Log, ", "))
	table.clear(Log)
end
-- FieldOfViewMode changes nothing a script reads
Camera.FieldOfViewMode = Enum.FieldOfViewMode.Diagonal
task.wait()
print("mode", table.concat(Log, ", "), P.num(Camera.FieldOfView), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
table.clear(Log)
Camera.FieldOfView = 40
Camera.FieldOfViewMode = Enum.FieldOfViewMode.MaxAxis
Camera.DiagonalFieldOfView = 100
task.wait()
print("mode writes", table.concat(Log, ", "), P.num(Camera.FieldOfView), P.num(Camera.DiagonalFieldOfView), P.num(Camera.MaxAxisFieldOfView))
for _, Connection in Connections do
	Connection:Disconnect()
end
-- a camera outside the Workspace answers the same
local Loose = Instance.new("Camera")
Loose.FieldOfView = 45
Loose.DiagonalFieldOfView = 50
print("loose", P.num(Loose.FieldOfView), P.num(Loose.DiagonalFieldOfView), P.num(Loose.MaxAxisFieldOfView))
