-- What each part of an assembly reads as its motion (AssemblyLinearVelocity, AssemblyAngularVelocity and the legacy Velocity
-- and RotVelocity) after its assembly changes in the frame a script set a velocity: two free parts welded either way round, a
-- free part welded to a moving anchored one that is then freed and anchored again, two welded pairs joined, a welded model
-- entering the Workspace, a weld disabled (each piece), a part anchored or given a RootPriority in a moving assembly (its own
-- motion, or none), and a part moved within one. Each motion is written to a part while it is its assembly's root, and nothing is
-- read before the change but where a case says so (a client's read of Velocity or RotVelocity keeps the motion it read in the
-- part). Built 300 studs out with no gravity and no collisions, so nothing moves between the writes and the reads.
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
-- each case 30 studs from the last
local function Case(Label, Parent)
	Spot += 1
	local Model = Instance.new("Model")
	Model.Name = Label
	Model.Parent = if Parent == false then nil else workspace
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
-- a Weld that keeps both parts where they are
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
local Big, Small = Vector3.new(2, 2, 2), Vector3.new(1, 1, 1)

-- 1. joins: each part takes the new root's motion
do
	local M, At = Case("join")
	local X, Y = Part("X", M, At, 0, Big), Part("Y", M, At, 4, Small)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 0.5, 0))
	Move(Y, Vector3.new(0, 0, 3), Vector3.new(0, 1, 0))
	Weld(X, Y)
	Motion("join x", { X, Y })
	M:Destroy()
	M, At = Case("join")
	X, Y = Part("X", M, At, 0, Big), Part("Y", M, At, 4, Small)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 0.5, 0))
	Move(Y, Vector3.new(0, 0, 3), Vector3.new(0, 1, 0))
	Weld(Y, X)
	Motion("join x-part1", { X, Y })
	M:Destroy()
	M, At = Case("join")
	X, Y = Part("X", M, At, 0, Small), Part("Y", M, At, 4, Big)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 0.5, 0))
	Move(Y, Vector3.new(0, 0, 3), Vector3.new(0, 1, 0))
	Weld(X, Y)
	Motion("join y", { X, Y })
	M:Destroy()
	-- a free part that never moved, welded to a moving one, and a moving one welded to a larger one that never moved
	M, At = Case("join")
	X, Y = Part("X", M, At, 0, Big), Part("Y", M, At, 4, Small)
	Move(X, Vector3.new(0, 2, 0), Vector3.new(0.25, 0, 0))
	Weld(X, Y)
	Motion("join still-y", { X, Y })
	M:Destroy()
	M, At = Case("join")
	X, Y = Part("X", M, At, 0, Big), Part("Y", M, At, 4, Small)
	Move(Y, Vector3.new(0, 2, 0), Vector3.new(0.25, 0, 0))
	Weld(X, Y)
	Motion("join still-x", { X, Y })
	M:Destroy()
end
-- an anchored part that moves (a conveyor) and a free part welded to it; the anchored part freed and anchored again
do
	local M, At = Case("conveyor")
	local X, Y = Part("X", M, At, 0, Big, true), Part("Y", M, At, 4, Small)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 0.5, 0))
	Move(Y, Vector3.new(0, 0, 3), Vector3.new(0, 1, 0))
	Weld(X, Y)
	Motion("conveyor welded", { X, Y })
	X.Anchored = false
	Motion("conveyor freed", { X, Y })
	X.Anchored = true
	Motion("conveyor anchored", { X, Y })
	M:Destroy()
end
-- two welded pairs, each moving from a write to its root, joined
do
	local M, At = Case("pairs")
	local X1, Y1 = Part("X1", M, At, 0, Big), Part("Y1", M, At, 2, Small)
	local X2, Y2 = Part("X2", M, At, 6, Big), Part("Y2", M, At, 4, Small)
	X2.RootPriority = 1
	Move(X1, Vector3.new(1, 2, 0), Vector3.new(0.25, 0, 0))
	Move(X2, Vector3.new(0, 0, -2), Vector3.new(0, 0, 0.75))
	Weld(X1, Y1)
	Weld(X2, Y2)
	Weld(Y1, Y2)
	Motion("pairs joined", { X1, Y1, X2, Y2 })
	M:Destroy()
end
-- a welded model entering the Workspace, its parts written while they were apart
do
	local M, At = Case("enter", false)
	local X, Y = Part("X", M, At, 0, Big), Part("Y", M, At, 4, Small)
	Move(X, Vector3.new(1, 0, 0), Vector3.zero)
	Move(Y, Vector3.new(0, 0, 3), Vector3.new(0, 1, 0))
	Weld(X, Y)
	M.Parent = workspace
	Motion("enter", { X, Y })
	M:Destroy()
end

-- 2. splits: the piece that keeps the root keeps the motion; a piece split off reads what its new root's own body holds
local function Trio(Label)
	local M, At = Case(Label)
	local X = Part("X", M, At, 0, Big)
	Move(X, Vector3.new(1, 0, 0), Vector3.new(0, 1, 0))
	local Y, Z = Part("Y", M, At, 2, Small), Part("Z", M, At, 4, Small)
	local XY, YZ = Weld(X, Y), Weld(Y, Z)
	return M, X, Y, Z, XY, YZ
end
do
	local M, X, Y, Z, XY, YZ = Trio("split")
	YZ.Enabled = false
	Motion("split leaf", { X, Y, Z })
	M:Destroy()
	M, X, Y, Z, XY, YZ = Trio("split")
	XY.Enabled = false
	Motion("split piece", { X, Y, Z })
	XY.Enabled = true
	Motion("split rejoined", { X, Y, Z })
	M:Destroy()
	-- a part that moved as a root of its own before it was welded: split off again, it reads its own motion
	M, X, Y, Z, XY, YZ = Trio("split")
	YZ:Destroy()
	XY:Destroy()
	Move(Z, Vector3.new(0, -1, 0), Vector3.new(0.5, 0, 0))
	Weld(X, Y)
	YZ = Weld(Y, Z)
	Motion("split own joined", { X, Y, Z })
	YZ.Parent = nil
	Motion("split own", { X, Y, Z })
	M:Destroy()
	-- read before the split: a client's read keeps the motion it read in the part
	M, X, Y, Z, XY, YZ = Trio("split")
	local _ = Z.Velocity
	YZ.Enabled = false
	Motion("split read", { X, Y, Z })
	M:Destroy()
end

-- 3. re-roots: a part anchored, or given a RootPriority, in a moving assembly gives it its own motion
do
	local M, X, Y, Z = Trio("reroot")
	Y.Anchored = true
	Motion("reroot anchored", { X, Y, Z })
	Y.Anchored = false
	Motion("reroot freed", { X, Y, Z })
	M:Destroy()
	M, X, Y, Z = Trio("reroot")
	Y.RootPriority = 5
	Motion("reroot priority", { X, Y, Z })
	M:Destroy()
	M, X, Y, Z = Trio("reroot")
	local _ = Y.RotVelocity
	Y.Anchored = true
	Motion("reroot read", { X, Y, Z })
	M:Destroy()
end

-- 4. a part moved within a moving assembly reads its velocity at its new place, and the assembly its centre's
do
	local M, X, Y, Z = Trio("moved")
	Z.Position = Z.Position + Vector3.new(0, 2, 0)
	Motion("moved z", { X, Y, Z })
	M:Destroy()
end

workspace.Gravity = Gravity
