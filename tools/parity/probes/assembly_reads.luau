-- What a script's read keeps in a part's body: a client keeps the motion a read of where a part is or how it moves works out (its
-- root's motion at the part) in the body of each part on the path from its assembly's root to it, zero included, as it is at the
-- read. Each case sets a motion, reads (or not), then splits or re-roots the assembly and prints what the piece reads: a part
-- split off reads its own body. Reads that keep it: Velocity, RotVelocity, AssemblyAngularVelocity, CFrame, Position,
-- Orientation, GetPivot, an index by a string, Clone, a tween made on Velocity; and a simulation step keeps an anchored moving
-- assembly's. Built 300 studs out with no gravity and no collisions, and placed so every motion is exact.
P.sync()
local Gravity = workspace.Gravity
workspace.Gravity = 0
local Spot = 0
local function V(Vector)
	return P.f32s(Vector.X, Vector.Y, Vector.Z)
end
local function Root(Each)
	local R = Each.AssemblyRootPart
	return if R then R.Name else "nil"
end
local function Motion(Label, Parts)
	for _, Each in Parts do
		print(Label, Each.Name, "root=" .. Root(Each), "linear=" .. V(Each.AssemblyLinearVelocity), "angular=" .. V(Each.AssemblyAngularVelocity), "velocity=" .. V(Each.Velocity), "rotvelocity=" .. V(Each.RotVelocity))
	end
end
-- (a re-rooted assembly's centre reads a float step off a client's, ASM-WELD-PRECISION; its parts' own motion is exact)
local function Own(Label, Parts)
	for _, Each in Parts do
		print(Label, Each.Name, "root=" .. Root(Each), "velocity=" .. V(Each.Velocity), "rotvelocity=" .. V(Each.RotVelocity))
	end
end
local function Case(Label)
	Spot += 1
	local Model = Instance.new("Model")
	Model.Name = Label
	Model.Parent = workspace
	return Model, CFrame.new(300 + 30 * Spot, 60, 40)
end
local function Part(Name, Model, At, X, Size, Anchored)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = Size or Vector3.new(1, 1, 1)
	Made.Anchored = Anchored == true
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = At * CFrame.new(X, 0, 0)
	Made.Parent = Model
	return Made
end
local function Weld(A, B)
	local Made = Instance.new("Weld")
	Made.Part0, Made.Part1 = A, B
	Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
	Made.Parent = A
	return Made
end
local function Move(Part, Linear, Angular)
	Part.AssemblyLinearVelocity = Linear
	Part.AssemblyAngularVelocity = Angular
end
local Big, Mid, Small = Vector3.new(2, 2, 2), Vector3.new(1.5, 1.5, 1.5), Vector3.new(1, 1, 1)
-- X (big, the root) moving, then Y and Z welded on in a chain
local function Trio(Label, YSize)
	local M, At = Case(Label)
	local X = Part("X", M, At, 0, Big)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 1, 0))
	local Y, Z = Part("Y", M, At, 2, YSize or Small), Part("Z", M, At, 4, Small)
	local XY, YZ = Weld(X, Y), Weld(Y, Z)
	return M, X, Y, Z, XY, YZ
end

-- 1. a read of the leaf Z, then Z split off: which reads keep the motion
for _, Read in {
	{ "none", function(Z) end },
	{ "velocity", function(Z) local _ = Z.Velocity end },
	{ "rotvelocity", function(Z) local _ = Z.RotVelocity end },
	{ "angular", function(Z) local _ = Z.AssemblyAngularVelocity end },
	{ "cframe", function(Z) local _ = Z.CFrame end },
	{ "position", function(Z) local _ = Z.Position end },
	{ "orientation", function(Z) local _ = Z.Orientation end },
	{ "rotation", function(Z) local _ = Z.Rotation end },
	{ "pivot", function(Z) local _ = Z:GetPivot() end },
	{ "index", function(Z) local _ = Z["Velocity"] end },
	{ "clone", function(Z) local _ = Z:Clone() end },
	{ "linear", function(Z) local _ = Z.AssemblyLinearVelocity end },
	{ "centre", function(Z) local _ = Z.AssemblyCenterOfMass end },
	{ "mass", function(Z) local _ = Z.AssemblyMass end },
	{ "rootpart", function(Z) local _ = Z.AssemblyRootPart end },
	{ "size", function(Z) local _ = Z.Size end },
	{ "anchored", function(Z) local _ = Z.Anchored end },
	{ "atposition", function(Z) local _ = Z:GetVelocityAtPosition(Vector3.zero) end },
	{ "signal", function(Z) Z:GetPropertyChangedSignal("Velocity"):Connect(function() end):Disconnect() end },
} do
	local M, X, Y, Z, XY, YZ = Trio("read")
	Read[2](Z)
	YZ.Enabled = false
	Motion("read " .. Read[1], { Z })
	M:Destroy()
end

-- 2. the path: Y-Z welded apart, X moved alone and welded to Y (the weld keeps nothing in Y or Z), then a read and X split off
for _, Read in {
	{ "none", function(Y, Z) end, "XY" },
	{ "leaf", function(Y, Z) local _ = Z.Velocity end, "XY" },
	{ "mid", function(Y, Z) local _ = Y.Velocity end, "XY" },
	{ "mid-leafsplit", function(Y, Z) local _ = Y.Velocity end, "YZ" },
} do
	local M, At = Case("path")
	local X, Y, Z = Part("X", M, At, 0, Big), Part("Y", M, At, 2, Mid), Part("Z", M, At, 4, Small)
	local YZ = Weld(Y, Z)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 1, 0))
	local XY = Weld(X, Y)
	Read[2](Y, Z)
	if Read[3] == "XY" then
		XY.Enabled = false
		Motion("path " .. Read[1], { Y, Z })
	else
		YZ.Enabled = false
		Motion("path " .. Read[1], { Z })
	end
	M:Destroy()
end
-- a part beside the path keeps nothing
do
	local M, X, Y, Z = Trio("beside", Mid)
	local W = Part("W", M, CFrame.new(300 + 30 * Spot, 60, 40), -2, Small)
	local XW = Weld(X, W)
	local _ = Z.Velocity
	XW.Enabled = false
	Motion("beside", { W })
	M:Destroy()
end

-- 3. zero is kept: Z (or Y with Z) moving on its own, welded to a still assembly (free, or anchored), then read
for _, Read in { "none", "read" } do
	local M, At = Case("zero")
	local X = Part("X", M, At, 0, Big)
	local Y, Z = Part("Y", M, At, 2, Small), Part("Z", M, At, 4, Small)
	Weld(X, Y)
	Move(Z, Vector3.new(0, -1, 0), Vector3.new(0.5, 0, 0))
	local YZ = Weld(Y, Z)
	if Read == "read" then
		local _ = Z.Velocity
	end
	YZ.Enabled = false
	Motion("zero " .. Read, { Z })
	M:Destroy()
end
for _, Anchored in { false, true } do
	for _, Read in { "none", "leaf" } do
		local M, At = Case("zeropath")
		local X, Y, Z = Part("X", M, At, 0, Big, Anchored), Part("Y", M, At, 2, Mid), Part("Z", M, At, 4, Small)
		Weld(Y, Z)
		Move(Y, Vector3.new(0, -1, 0), Vector3.new(0.5, 0, 0))
		local XY = Weld(X, Y)
		if Read == "leaf" then
			local _ = Z.Velocity
		end
		XY.Enabled = false
		Motion("zeropath " .. (if Anchored then "anchored " else "free ") .. Read, { Y, Z })
		M:Destroy()
	end
end

-- 4. what is kept is the motion at the read: a later write to the root does not reach it, a read after the write keeps the new one
for _, Order in { "read-write", "write-read", "read-write-read", "read-write-readmid" } do
	local M, X, Y, Z, XY, YZ = Trio("when", Mid)
	if Order == "read-write" or Order == "read-write-read" or Order == "read-write-readmid" then
		local _ = Z.Velocity
	end
	Move(X, Vector3.new(0, 0, 5), Vector3.zero)
	if Order == "write-read" or Order == "read-write-read" then
		local _ = Z.Velocity
	elseif Order == "read-write-readmid" then
		local _ = Y.Velocity
	end
	YZ.Enabled = false
	Motion("when " .. Order, { Z })
	M:Destroy()
end

-- 5. an anchored root that moves (a conveyor): a read in the frame of the weld, and a simulation step between
for _, Read in { "none", "read", "frame" } do
	local M, At = Case("conveyor")
	local X, Y = Part("X", M, At, 0, Big, true), Part("Y", M, At, 4, Small)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 0.5, 0))
	local XY = Weld(X, Y)
	if Read == "read" then
		local _ = Y.Velocity
	elseif Read == "frame" then
		task.wait()
	end
	XY.Enabled = false
	Own("conveyor " .. Read, { Y })
	M:Destroy()
end

-- 6. re-rooted after a read: the part read anchored, or ranked above the root
for _, Change in { "anchor", "priority" } do
	for _, Read in { "none", "read" } do
		local M, X, Y, Z = Trio("reroot")
		if Read == "read" then
			local _ = Z.Velocity
		end
		if Change == "anchor" then
			Z.Anchored = true
		else
			Z.RootPriority = 5
		end
		Own("reroot " .. Change .. " " .. Read, { X, Z })
		M:Destroy()
	end
end

-- 7. a model's pivot (its PrimaryPart's), a tween made on Velocity (and played), a clone alone in the Workspace
do
	local M, X, Y, Z, XY, YZ = Trio("model")
	M.PrimaryPart = Z
	local _ = M:GetPivot()
	YZ.Enabled = false
	Motion("model pivot", { Z })
	M:Destroy()
end
for _, Kind in { "create", "play" } do
	local M, X, Y, Z, XY, YZ = Trio("tween")
	local T = game:GetService("TweenService"):Create(Z, TweenInfo.new(1), { Velocity = Vector3.new(0, 3, 0) })
	if Kind == "play" then
		T:Play()
	end
	YZ.Enabled = false
	Motion("tween " .. Kind, { Z })
	T:Cancel()
	M:Destroy()
end
do
	local M, X, Y, Z = Trio("clone")
	local C = Z:Clone()
	C.Name = "C"
	print("clone values " .. V(C.Velocity) .. " " .. V(C.RotVelocity))
	C.Parent = M
	Motion("clone", { C })
	M:Destroy()
end

-- 8. outside the Workspace (a model in nil) nothing moves, read or not
for _, Read in { "none", "read" } do
	Spot += 1
	local M, At = Instance.new("Model"), CFrame.new(300 + 30 * Spot, 60, 40)
	local X, Y, Z = Part("X", M, At, 0, Big), Part("Y", M, At, 2, Small), Part("Z", M, At, 4, Small)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 1, 0))
	Weld(X, Y)
	local YZ = Weld(Y, Z)
	if Read == "read" then
		local _ = Z.Velocity
	end
	YZ.Enabled = false
	M.Parent = workspace
	Motion("outside " .. Read, { Z })
	M:Destroy()
end

workspace.Gravity = Gravity
