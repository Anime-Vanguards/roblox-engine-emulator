-- Workspace:UnjoinFromOutsiders (world/joints.rs unjoin_from_outsiders): which joints it takes out, what it does to them, the
-- order each listed part's joints go in (and what sets that order: when a joint was made, connected or parented, what moves
-- it later, and the order one move takes several out of their parts' lists or puts several in), what a list's entries list
-- (nested and repeated entries too), the assemblies after, and a removed joint parented again. A client goes through its
-- listed parts in an order of its own (it changed between runs), so each listed part's joints are printed apart. What it makes
-- lives under one Folder (and JointsService, ReplicatedStorage) and is cleaned up as it goes.
local W = workspace
local JointsService = game:GetService("JointsService")
local Storage = game:GetService("ReplicatedStorage")
local Root = Instance.new("Folder")
Root.Name = "UnjoinProbe"
Root.Parent = W

local Made = {}
local function Brick(Name, Y, Parent, Anchored)
	local B = Instance.new("Part")
	B.Name = Name
	B.Anchored = if Anchored == nil then true else Anchored
	B.Size = Vector3.new(2, 1, 2)
	B.CFrame = CFrame.new(150, Y, 150)
	B.Parent = Parent or Root
	return B
end
local function J(Class, Name, P0, P1, Parent, Props)
	local X = Instance.new(Class)
	X.Name = Name
	X.Part0 = P0
	X.Part1 = P1
	for K, V in Props or {} do
		X[K] = V
	end
	X.Parent = Parent
	table.insert(Made, X)
	return X
end
local function Where(X)
	return if X.Parent then X.Parent:GetFullName() else "nil"
end
local function State()
	local Out = {}
	for _, X in Made do
		table.insert(Out, X.Name .. " in " .. Where(X))
	end
	return table.concat(Out, " ")
end
local function Unjoin(List)
	return P.try(function()
		W:UnjoinFromOutsiders(List)
	end)
end
local function Reset()
	for _, X in Made do
		X:Destroy()
	end
	Made = {}
	Root:ClearAllChildren()
end

-- 1. the order each listed part's joints go in, and the events
local Order = {}
local Watch = game.DescendantRemoving:Connect(function(D)
	if D:IsA("JointInstance") then
		table.insert(Order, D)
	end
end)
local function Setup()
	local A = Brick("A", 100)
	local B = Brick("B", 101)
	local C = Brick("C", 102)
	local D = Brick("D", 103)
	J("Weld", "j1", C, D, Root)
	J("Weld", "j2", B, A, A)
	J("ManualWeld", "j3", A, B, JointsService)
	J("Motor6D", "j4", B, D, B)
	J("Snap", "j5", D, C, D)
	J("Glue", "j6", B, C, B)
	J("ManualGlue", "j7", A, C, W)
	J("Motor", "j8", D, B, D)
	J("Glue", "j9", C, A, C)
	return A, B, C, D
end
-- (each listed part's joints in the order they went; a joint between two listed parts stays, so it is under one part only)
local function Grouped(Listed)
	local Names = {}
	for _, Part in Listed do
		if not table.find(Names, Part.Name) then
			table.insert(Names, Part.Name)
		end
	end
	table.sort(Names)
	local Out = {}
	for _, PartName in Names do
		local Mine = {}
		for _, X in Order do
			if (X.Part0 and X.Part0.Name == PartName) or (X.Part1 and X.Part1.Name == PartName) then
				table.insert(Mine, X.Name)
			end
		end
		table.insert(Out, PartName .. ":" .. table.concat(Mine, ","))
	end
	return table.concat(Out, " ")
end
local function Run(Label, Listed, Events)
	-- (the last case's cleanup has fired its events before this one counts)
	task.wait()
	Order = {}
	local Seen = {}
	if Events then
		for _, X in Made do
			X.AncestryChanged:Connect(function(_, Parent)
				table.insert(Seen, X.Name .. ">" .. (if Parent then Parent.Name else "nil"))
			end)
		end
	end
	print("order", Label, "->", Unjoin(Listed), "now", State())
	task.wait()
	print("order", Label, "removing ->", Grouped(Listed), "count", #Order)
	if Events then
		table.sort(Seen)
		print("order", Label, "events ->", table.concat(Seen, ","))
	end
	Reset()
end
do
	local A, B, C, D = Setup()
	Run("{B,C}", { B, C }, true)
	A, B, C, D = Setup()
	Run("{C,B}", { C, B })
	A, B, C, D = Setup()
	Run("{A}", { A })
	A, B, C, D = Setup()
	Run("{D,A}", { D, A })
	A, B, C, D = Setup()
	Run("{B,C,B}", { B, C, B })
	task.wait()
end
Watch:Disconnect()

-- 2. which joints count
do
	local A = Brick("A", 100)
	local B = Brick("B", 101)
	local World = Instance.new("WorldModel")
	World.Parent = Root
	local Mw = Brick("M", 120, World)
	local Nw = Brick("N", 121, World)
	local Kept = Instance.new("Folder")
	Kept.Parent = Storage
	local S1 = Brick("S1", 130, Kept)
	J("Weld", "in-worldmodel", B, A, World)
	J("Weld", "disabled", B, A, B, { Enabled = false })
	J("Weld", "in-storage", B, A, Storage)
	J("Weld", "loose", B, A, nil)
	J("Weld", "half", B, nil, B)
	J("Weld", "self", B, B, B)
	J("Weld", "to-worldmodel-part", B, Mw, B)
	J("Weld", "to-storage-part", B, S1, B)
	J("Glue", "glue-to-storage-part", S1, B, B)
	J("Weld", "deep", B, A, Root)
	J("Rotate", "rotate", B, A, B)
	J("RotateP", "rotatep", B, A, B)
	J("RotateV", "rotatev", B, A, B)
	J("VelocityMotor", "velocitymotor", B, A, B)
	J("Motor", "motor", B, A, B)
	J("Weld", "wm-pair", Nw, Mw, World)
	local WC = Instance.new("WeldConstraint")
	WC.Name = "weldconstraint"
	WC.Part0 = B
	WC.Part1 = A
	WC.Parent = B
	table.insert(Made, WC)
	local Rigid = Instance.new("RigidConstraint")
	Rigid.Name = "rigidconstraint"
	local A0 = Instance.new("Attachment")
	A0.Parent = A
	local A1 = Instance.new("Attachment")
	A1.Parent = B
	Rigid.Attachment0 = A0
	Rigid.Attachment1 = A1
	Rigid.Parent = B
	table.insert(Made, Rigid)
	print("kinds {B} ->", Unjoin({ B }), "now", State())
	print("kinds {N} in worldmodel ->", Unjoin({ Nw }), "now", State())
	Reset()
	Kept:Destroy()
end

-- 3. what a list lists: a model's parts, a part's child part, nested and repeated entries, a folder, the Workspace, the game,
-- a destroyed part
local function Tree()
	local A = Brick("A", 100)
	local Model = Instance.new("Model")
	Model.Name = "Model"
	Model.Parent = Root
	local B = Brick("B", 101, Model)
	local C = Brick("C", 102, B)
	local E = Brick("E", 103)
	J("Weld", "B-A", B, A, B)
	J("Weld", "C-A", C, A, C)
	J("Weld", "B-C", B, C, B)
	J("Weld", "E-C", E, C, E)
	J("Weld", "E-A", E, A, E)
	return A, Model, B, C, E
end
do
	local A, Model, B, C, E = Tree()
	print("listed {game} ->", Unjoin({ game }), "now", State())
	print("listed {workspace} ->", Unjoin({ W }), "now", State())
	print("listed {Root} ->", Unjoin({ Root }), "now", State())
	print("listed {B} ->", Unjoin({ B }), "now", State())
	Reset()
	for _, Case in {
		{ "{Model}", function(Model, B, C, E) return { Model } end },
		{ "{Model,B}", function(Model, B, C, E) return { Model, B } end },
		{ "{C,Model}", function(Model, B, C, E) return { C, Model } end },
		{ "{B,Model,C,B}", function(Model, B, C, E) return { B, Model, C, B } end },
		{ "{Model,E}", function(Model, B, C, E) return { Model, E } end },
		{ "{C,E}", function(Model, B, C, E) return { C, E } end },
		{ "{C}", function(Model, B, C, E) return { C } end },
	} do
		A, Model, B, C, E = Tree()
		print("listed", Case[1], "->", Unjoin(Case[2](Model, B, C, E)), "now", State())
		Reset()
	end
	A = Brick("A", 100)
	B = Brick("B", 101)
	J("Weld", "A-B", A, B, A)
	B:Destroy()
	print("listed destroyed ->", Unjoin({ B }), "now", State())
	Reset()
end

-- 4. the assemblies after: an unanchored part welded to an anchored one
do
	local A = Brick("A", 100)
	local B = Brick("B", 101, nil, false)
	J("Weld", "w", B, A, B)
	print("assembly before ->", B.AssemblyRootPart.Name)
	print("assembly unjoin ->", Unjoin({ B }), "root", B.AssemblyRootPart.Name, "parent", Where(Made[1]))
	-- (taken out, not destroyed: it can be parented again, and links its parts up again)
	local Weld = Made[1]
	print("assembly parented again ->", P.try(function()
		Weld.Parent = B
	end), "parent", Where(Weld), "parts", Weld.Part0 and Weld.Part0.Name, Weld.Part1 and Weld.Part1.Name, "root", B.AssemblyRootPart.Name)
	print("assembly unjoin again ->", Unjoin({ B }), "root", B.AssemblyRootPart.Name, "parent", Where(Weld))
	B.Anchored = true
	Reset()
end

-- 5. what orders a part's joints: each case lists one part, B (A in one), whose welds x1, x2, ... (made in that order) join it to
-- A, and prints the order they went in
local Removing = {}
local Watch2 = game.DescendantRemoving:Connect(function(D)
	if D:IsA("JointInstance") then
		table.insert(Removing, D.Name)
	end
end)
local function Joined(X, A, B)
	for _, Weld in X do
		Weld.Part0, Weld.Part1 = B, A
	end
	for _, Weld in X do
		Weld.Parent = B
	end
	task.wait()
end
local function Listing(Label, Count, Build, ListA)
	task.wait()
	local A, B, C = Brick("A", 100), Brick("B", 101), Brick("C", 102)
	local X = {}
	for Index = 1, Count do
		local Weld = Instance.new("Weld")
		Weld.Name = "x" .. Index
		X[Index] = Weld
		table.insert(Made, Weld)
	end
	Build(X, A, B, C)
	task.wait()
	Removing = {}
	local Result = Unjoin({ if ListA then A else B })
	task.wait()
	print("list", Label, "->", Result, table.concat(Removing, ","))
	Reset()
end
Listing("made 1-3, connected 3-1, parented 1-3", 3, function(X, A, B)
	for Index = 3, 1, -1 do
		X[Index].Part0, X[Index].Part1 = B, A
	end
	for Index = 1, 3 do
		X[Index].Parent = B
	end
end)
Listing("made 1-3, connected 1-3, parented 3-1", 3, function(X, A, B)
	for Index = 1, 3 do
		X[Index].Part0, X[Index].Part1 = B, A
	end
	for Index = 3, 1, -1 do
		X[Index].Parent = B
	end
end)
Listing("made 1-3, parented 1-3, connected 3-1", 3, function(X, A, B)
	for Index = 1, 3 do
		X[Index].Parent = B
	end
	for Index = 3, 1, -1 do
		X[Index].Part0, X[Index].Part1 = B, A
	end
end)
Listing("x1 out and back", 3, function(X, A, B)
	Joined(X, A, B)
	X[1].Parent = nil
	X[1].Parent = B
end)
Listing("x1 out, a frame, back", 3, function(X, A, B)
	Joined(X, A, B)
	X[1].Parent = nil
	task.wait()
	X[1].Parent = B
end)
Listing("x1 Part1 to C and back", 3, function(X, A, B, C)
	Joined(X, A, B)
	X[1].Part1 = C
	X[1].Part1 = A
end)
Listing("x1 Part0 nil, a frame, back", 3, function(X, A, B)
	Joined(X, A, B)
	X[1].Part0 = nil
	task.wait()
	X[1].Part0 = B
end)
Listing("x1 disabled, a frame, enabled", 3, function(X, A, B)
	Joined(X, A, B)
	X[1].Enabled = false
	task.wait()
	X[1].Enabled = true
end)
Listing("x1 Part0 and Part1 swapped", 3, function(X, A, B)
	Joined(X, A, B)
	X[1].Part0, X[1].Part1 = A, B
end)
Listing("x1 into JointsService", 3, function(X, A, B)
	Joined(X, A, B)
	X[1].Parent = JointsService
end)
Listing("x1 C0 changed", 3, function(X, A, B)
	Joined(X, A, B)
	X[1].C0 = CFrame.new(0, 1, 0)
end)
Listing("of 4, x2 out and back", 4, function(X, A, B)
	Joined(X, A, B)
	X[2].Parent = nil
	X[2].Parent = B
end)
Listing("of 4, x1 out, x2 out, x2 back, x1 back", 4, function(X, A, B)
	Joined(X, A, B)
	X[1].Parent = nil
	X[2].Parent = nil
	X[2].Parent = B
	X[1].Parent = B
end)
Listing("of 4, x3 to ReplicatedStorage and back", 4, function(X, A, B)
	Joined(X, A, B)
	X[3].Parent = Storage
	X[3].Parent = B
end)
Listing("of 4, x2 Part1 to C, A listed", 4, function(X, A, B, C)
	Joined(X, A, B)
	X[2].Part1 = C
end, true)
Listing("of 4, A to ReplicatedStorage and back", 4, function(X, A, B)
	Joined(X, A, B)
	A.Parent = Storage
	A.Parent = Root
end)
Listing("of 4, A into a WorldModel and back", 4, function(X, A, B)
	Joined(X, A, B)
	local World = Instance.new("WorldModel")
	World.Parent = Root
	A.Parent = World
	A.Parent = Root
end)

-- 6. one move that takes several joints out of their parts' lists, or puts several in: each case's welds x1, x2, ... join B
-- to the parts Others names, get their parts in index order (or Order's) and are parented in index order (to B, or where
-- Places says), and the case prints B's list as UnjoinFromOutsiders({B}) takes it out
local Dropped = {}
local function New(Name, Parent)
	local F = Instance.new("Folder")
	F.Name = Name
	F.Parent = Parent
	table.insert(Dropped, F)
	return F
end
local function Moved(Label, Others, Build)
	task.wait()
	local Q = { A = Brick("A", 100), B = Brick("B", 101), C = Brick("C", 102), D = Brick("D", 103) }
	local X = {}
	for Index = 1, #Others do
		local Weld = Instance.new("Weld")
		Weld.Name = "x" .. Index
		X[Index] = Weld
		table.insert(Made, Weld)
	end
	local function Name(Order)
		for _, Index in Order or X do
			Index = if type(Index) == "number" then Index else table.find(X, Index)
			X[Index].Part0, X[Index].Part1 = Q.B, Q[Others[Index]]
		end
	end
	local function Put(Places)
		for Index, Weld in X do
			if Weld.Parent == nil then
				Weld.Parent = Places and Places[Index] or Q.B
			end
		end
	end
	Build(X, Q, Name, Put)
	task.wait()
	Removing = {}
	local Result = Unjoin({ Q.B })
	task.wait()
	print("moved", Label, "->", Result, table.concat(Removing, ","))
	for _, F in Dropped do
		F:Destroy()
	end
	Dropped = {}
	for _, Part in Q do
		Part:Destroy()
	end
	Reset()
end
-- (leaving: the moved instance first, then the rest from the bottom up, a part's active joints the last named first)
Moved("A out, x1 x3 to A, named and parented 1-5", { "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
	Name()
	Put()
	Q.A.Parent = Storage
end)
Moved("A out, x1 x3 to A, named 3 1 2 4 5, parented 1-5", { "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
	Name({ 3, 1, 2, 4, 5 })
	Put()
	Q.A.Parent = Storage
end)
Moved("A [x1] out, x1 x3 to A", { "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
	Name()
	Put({ Q.A })
	Q.A.Parent = Storage
end)
Moved("Folder [A [x1]] out, x1 x3 to A", { "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
	Q.A.Parent = New("Fo", Root)
	Name()
	Put({ Q.A })
	Q.A.Parent.Parent = Storage
end)
Moved("Folder [x2, x3] out", { "A", "A", "A", "A", "A" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Root)
	Put({ nil, Fo, Fo })
	Name()
	Fo.Parent = nil
end)
Moved("Folder [x3, x2] out", { "A", "A", "A", "A", "A" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Root)
	X[3].Parent = Fo
	Put({ nil, Fo })
	Name()
	Fo.Parent = nil
end)
Moved("Folder [x2, Sub [x3]] out", { "A", "A", "A", "A", "A" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Root)
	Name()
	Put({ nil, Fo })
	X[3].Parent = New("Sub", Fo)
	Fo.Parent = nil
end)
Moved("Folder [x2 [x3]] out", { "A", "A", "A", "A", "A" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Root)
	Name()
	Put({ nil, Fo })
	X[3].Parent = X[2]
	Fo.Parent = nil
end)
Moved("Folder [A, C] out, x1 to A, x3 to C", { "A", "D", "C", "D", "D", "D" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Root)
	Q.A.Parent = Fo
	Q.C.Parent = Fo
	Name()
	Put()
	Fo.Parent = Storage
end)
Moved("Folder [C, A] out, x1 to A, x3 to C", { "A", "D", "C", "D", "D", "D" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Root)
	Q.C.Parent = Fo
	Q.A.Parent = Fo
	Name()
	Put()
	Fo.Parent = Storage
end)
Moved("Folder [A [x3]] out, x1 to A, x3 to C", { "A", "D", "C", "D", "D", "D" }, function(X, Q, Name, Put)
	Q.A.Parent = New("Fo", Root)
	Name()
	Put({ nil, nil, Q.A })
	Q.A.Parent.Parent = Storage
end)
Moved("A [x3] out, x1 to A, x3 to C", { "A", "D", "C", "D", "D", "D" }, function(X, Q, Name, Put)
	Name()
	Put({ nil, nil, Q.A })
	Q.A.Parent = Storage
end)
Moved("A into a Folder of the Workspace", { "A", "A", "A", "A" }, function(X, Q, Name, Put)
	Name()
	Put()
	Q.A.Parent = New("Fo", Root)
end)
Moved("x2 into a Folder of the Workspace", { "A", "A", "A", "A" }, function(X, Q, Name, Put)
	Name()
	Put()
	X[2].Parent = New("Fo", Root)
end)
-- (re-pointing, while out of the DataModel: a joint names a part anew when the side naming it is set, not its other side)
Moved("A out, x1's Part0 to D and back first", { "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
	Name()
	X[1].Part0 = Q.D
	X[1].Part0 = Q.B
	Put()
	Q.A.Parent = Storage
end)
Moved("A out, x1's Part1 to C and back first", { "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
	Name()
	X[1].Part1 = Q.C
	X[1].Part1 = Q.A
	Put()
	Q.A.Parent = Storage
end)
Moved("A out, x1's parts swapped first", { "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
	Name()
	X[1].Part0, X[1].Part1 = Q.A, Q.B
	Put()
	Q.A.Parent = Storage
end)
-- (entering: the same order, and an instance not reached yet counts as where it was: a joint below a part that comes back
-- joins when it is reached)
Moved("Folder [x2, x3] in", { "A", "A", "A" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Storage)
	Name()
	Put({ nil, Fo, Fo })
	Fo.Parent = Root
end)
Moved("Folder [x3, x2] in", { "A", "A", "A" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Storage)
	Name()
	X[3].Parent = Fo
	Put({ nil, Fo })
	Fo.Parent = Root
end)
Moved("Folder [x3, Sub [x2]] in", { "A", "A", "A" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Storage)
	Name()
	X[3].Parent = Fo
	Put({ nil, New("Sub", Fo) })
	Fo.Parent = Root
end)
Moved("Folder [A, C] in, x2 x3 to A, x4 x5 to C", { "D", "A", "A", "C", "C" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Storage)
	Q.A.Parent = Fo
	Q.C.Parent = Fo
	Name()
	Put()
	Fo.Parent = Root
end)
Moved("Folder [C, A] in, x2 x3 to A, x4 x5 to C", { "D", "A", "A", "C", "C" }, function(X, Q, Name, Put)
	local Fo = New("Fo", Storage)
	Q.C.Parent = Fo
	Q.A.Parent = Fo
	Name()
	Put()
	Fo.Parent = Root
end)
Moved("Folder [A [x3]] in, x2 to A, x3 to C", { "D", "A", "C" }, function(X, Q, Name, Put)
	Q.A.Parent = New("Fo", Storage)
	Name()
	Put({ nil, nil, Q.A })
	Q.A.Parent.Parent = Root
end)
Moved("A [x2, x3] in, x2 x3 to A", { "D", "A", "A" }, function(X, Q, Name, Put)
	Q.A.Parent = Storage
	Name()
	Put({ nil, Q.A, Q.A })
	Q.A.Parent = Root
end)
Moved("A [x3, x2] in, x2 x3 to A", { "D", "A", "A" }, function(X, Q, Name, Put)
	Q.A.Parent = Storage
	Name()
	X[3].Parent = Q.A
	Put({ nil, Q.A })
	Q.A.Parent = Root
end)
Moved("A [x3] in, x2 x3 to A", { "D", "A", "A" }, function(X, Q, Name, Put)
	Q.A.Parent = Storage
	Name()
	Put({ nil, nil, Q.A })
	Q.A.Parent = Root
end)
Moved("A [x2] in, x2 x3 to A", { "D", "A", "A" }, function(X, Q, Name, Put)
	Q.A.Parent = Storage
	Name()
	Put({ nil, Q.A })
	Q.A.Parent = Root
end)
Moved("Folder [A [x2]] in, x2 x3 to A", { "D", "A", "A" }, function(X, Q, Name, Put)
	Q.A.Parent = New("Fo", Storage)
	Name()
	Put({ nil, Q.A })
	Q.A.Parent.Parent = Root
end)
Watch2:Disconnect()
Root:Destroy()
