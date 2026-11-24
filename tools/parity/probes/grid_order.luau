-- The order inside a spatial-grid cell (a client's broadphase): unanchored parts before anchored ones, each in
-- the order they came in; a part leaving a cell gives its place to the list's last; which changes file a part again at
-- the end (at the write itself, with no query between); a subtree's order coming in; welded unanchored parts; which of
-- exact ties the casts take; and a WorldModel's own grid under the same rules.
P.sync()
local Gravity = workspace.Gravity
workspace.Gravity = 0
local Root = Instance.new("Folder")
Root.Name = "GridOrder"
Root.Parent = workspace
local Mine = {}
local function Part(Name, At, Size, Parent)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = true
	Made.Size = Size or Vector3.new(0.2, 0.2, 0.2)
	Made.CFrame = if typeof(At) == "CFrame" then At else CFrame.new(At)
	Made.Parent = Parent or Root
	Mine[Made] = true
	return Made
end
local function Free(Name, At, Size)
	local Made = Part(Name, At, Size)
	Made.Anchored = false
	return Made
end
local function Index()
	workspace:GetPartBoundsInBox(CFrame.new(0, -10000, 0), Vector3.one)
end
local function Listed(List)
	local Names = {}
	for _, Found in List do
		if Mine[Found] then
			table.insert(Names, Found.Name)
		end
	end
	return table.concat(Names, " ")
end
local function Clear()
	Root:ClearAllChildren()
	table.clear(Mine)
end
local V = Vector3.new
local C0 = V(2004, 1004, 2004) -- mid-cell (cells 2000..2008)
local function Q(Label, Center, Size)
	print(`{Label}: {Listed(workspace:GetPartBoundsInBox(CFrame.new(Center), Size))}`)
end

-- 1. lists per cell: anchored and unanchored in cells along x; in one cell made in turn
Part("a1", C0)
Free("u2", C0 + V(8, 0, 0))
Part("a3", C0 + V(16, 0, 0))
Free("u4", C0 + V(24, 0, 0))
Index()
Q("cells along x: anchored, unanchored, anchored, unanchored", C0 + V(12, 0, 0), V(30, 1, 1))
Clear()
Part("a1", C0)
Free("u2", C0 + V(0.5, 0, 0))
Part("a3", C0 + V(1, 0, 0))
Free("u4", C0 + V(1.5, 0, 0))
Index()
Q("one cell, made a1 u2 a3 u4", C0, V(4, 1, 1))
Clear()

-- 2. changes to a2 of a1..a5 (anchored, one cell), then a query
local function Scenario(Label, Change)
	local A = {}
	for I = 1, 5 do
		A[I] = Part("a" .. I, C0 + V(-2 + I * 0.6, 0, 0))
	end
	Index()
	Change(A)
	Index()
	Q(Label, C0, V(6, 1, 1))
	Clear()
end
Scenario("nothing", function() end)
Scenario("a2 destroyed", function(A) A[2]:Destroy() end)
Scenario("a2 destroyed, then a1", function(A) A[2]:Destroy() Index() A[1]:Destroy() end)
Scenario("a2 moved within its cell", function(A) A[2].CFrame += V(0, 0, 0.3) end)
Scenario("a2 moved out, a query, back", function(A) A[2].CFrame += V(20, 0, 0) Index() A[2].CFrame -= V(20, 0, 0) end)
Scenario("a2 moved out and back, no query between", function(A) A[2].CFrame += V(20, 0, 0) A[2].CFrame -= V(20, 0, 0) end)
Scenario("a2 grown into the next cell too", function(A) A[2].Size = V(8, 0.2, 0.2) end)
Scenario("a2 turned in place", function(A) A[2].CFrame *= CFrame.Angles(0, math.rad(30), 0) end)
Scenario("a2 resized within its cells", function(A) A[2].Size = V(0.4, 0.4, 0.4) end)
Scenario("a2 resized to level 1", function(A) A[2].Size = V(30, 30, 30) end)
Scenario("a2 resized to level 1 and back", function(A) A[2].Size = V(30, 30, 30) A[2].Size = V(0.2, 0.2, 0.2) end)
Scenario("a2 unanchored", function(A) A[2].Anchored = false end)
Scenario("a2 unanchored and anchored again", function(A) A[2].Anchored = false A[2].Anchored = true end)
Scenario("a2 CanCollide off", function(A) A[2].CanCollide = false end)
Scenario("a2 CanTouch off", function(A) A[2].CanTouch = false end)
Scenario("a2 CanCollide and CanTouch off", function(A) A[2].CanCollide = false A[2].CanTouch = false end)
Scenario("a2 Transparency, Material, CollisionGroup, Massless, CustomPhysicalProperties, Shape, Locked", function(A)
	A[2].Transparency = 0.5
	A[2].Material = Enum.Material.Neon
	A[2].CollisionGroup = "Default"
	A[2].Massless = true
	A[2].CustomPhysicalProperties = PhysicalProperties.new(2, 0.3, 0.5)
	A[2].Shape = Enum.PartType.Ball
	A[2].Locked = true
end)
Scenario("a2 the same CFrame, Size and Anchored written", function(A) A[2].CFrame = A[2].CFrame A[2].Size = A[2].Size A[2].Anchored = true end)
Scenario("a2 to a folder in the Workspace", function(A)
	local Folder = Instance.new("Folder")
	Folder.Parent = Root
	A[2].Parent = Folder
end)
Scenario("a2 to nil and back, no query between", function(A) A[2].Parent = nil A[2].Parent = Root end)
Scenario("a2 welded to a4", function(A)
	local Weld = Instance.new("WeldConstraint")
	Weld.Part0 = A[2]
	Weld.Part1 = A[4]
	Weld.Parent = A[2]
end)
Scenario("an unanchored part welded to a2", function(A)
	local Other = Free("other", C0 + V(100, 0, 0))
	local Weld = Instance.new("WeldConstraint")
	Weld.Part0 = A[2]
	Weld.Part1 = Other
	Weld.Parent = A[2]
end)

-- 3. a part in two cells that grows into a third keeps its place in the first
do
	local X = Part("X", C0 + V(3.9, 0, 0)) -- cells 2000 and 2008
	Part("M", C0 + V(2, 0, 0))
	Index()
	Q("X (two cells) then M", C0 + V(4, 0, 0), V(15, 1, 1))
	X.Size = V(16, 0.2, 0.2)
	Index()
	Q("X grown into the cell below", C0 + V(4, 0, 0), V(15, 1, 1))
	Clear()
end

-- 4. a subtree coming in: T{a, M{b, c{d, f}}, e}
do
	local T = Instance.new("Folder")
	local function Sub(Name, Parent, X)
		return Part(Name, C0 + V(X, 0, 0), nil, Parent)
	end
	Sub("a", T, -0.9)
	local M = Instance.new("Model")
	M.Parent = T
	Sub("b", M, -0.5)
	local c = Sub("c", M, -0.1)
	Sub("d", c, 0.3)
	Sub("f", c, 0.7)
	Sub("e", T, 0.9)
	T.Parent = Root
	Index()
	Q("subtree T(a, M(b, c(d, f)), e) came in", C0, V(4, 1, 1))
	Clear()
end

-- 5. welded unanchored parts
do
	Part("a1", C0)
	local U2 = Free("u2", C0 + V(0.5, 0, 0))
	Part("a3", C0 + V(1, 0, 0))
	local U4 = Free("u4", C0 + V(1.5, 0, 0))
	local Weld = Instance.new("WeldConstraint")
	Weld.Part0 = U2
	Weld.Part1 = U4
	Weld.Parent = U2
	Free("u5", C0 + V(-0.5, 0, 0))
	Index()
	Q("a1 u2 a3 u4 (welded to u2) u5", C0, V(4, 1, 1))
	U2.Anchored = true
	Index()
	Q("then u2 anchored", C0, V(4, 1, 1))
	Clear()
end

-- 6. exact ties: coincident parts (or level tops) under casts from above
do
	local function Casts(Label, Make)
		Make()
		Index()
		local From = C0 + V(0, 10, 0)
		local Ray = workspace:Raycast(From, V(0, -20, 0))
		local Sphere = workspace:Spherecast(From, 0.5, V(0, -20, 0))
		local Block = workspace:Blockcast(CFrame.new(From), V(1, 1, 1), V(0, -20, 0))
		local Caster = Instance.new("Part")
		Caster.Anchored = true
		Caster.Size = V(1, 1, 1)
		Caster.CFrame = CFrame.new(From)
		local Shape = workspace:Shapecast(Caster, V(0, -20, 0))
		local Camera = Instance.new("Camera")
		Camera.CFrame = CFrame.new(From, C0)
		Camera.Parent = workspace
		local Names = {}
		for _, Found in Camera:GetPartsObscuringTarget({ C0 - V(0, 10, 0) }, {}) do
			table.insert(Names, Found.Name)
		end
		Camera:Destroy()
		local function Name(Result)
			return if Result then Result.Instance.Name else "nil"
		end
		print(`ties, {Label}: ray {Name(Ray)} sphere {Name(Sphere)} block {Name(Block)} shape {Name(Shape)} obscuring {table.concat(Names, ",")} box {Listed(workspace:GetPartBoundsInBox(CFrame.new(C0), V(0.5, 0.5, 0.5)))}`)
		Clear()
	end
	local function Coincident(Spec)
		return function()
			for _, S in Spec do
				local Made = Part(S[1], C0, V(2, 2, 2))
				Made.Anchored = S[2]
			end
		end
	end
	Casts("A B C anchored", Coincident({ { "A", true }, { "B", true }, { "C", true } }))
	Casts("A anchored, B unanchored, C anchored", Coincident({ { "A", true }, { "B", false }, { "C", true } }))
	Casts("A B unanchored, C anchored", Coincident({ { "A", false }, { "B", false }, { "C", true } }))
	Casts("A B unanchored", Coincident({ { "A", false }, { "B", false } }))
	Casts("A destroyed after a query", function()
		local A = Part("A", C0, V(2, 2, 2))
		Part("B", C0, V(2, 2, 2))
		Part("C", C0, V(2, 2, 2))
		Index()
		A:Destroy()
	end)
	Casts("A level 1 (top level with B), B level 0", function()
		Part("A", C0 + V(0, -14, 0), V(30, 30, 30))
		Part("B", C0, V(2, 2, 2))
	end)
	Casts("A level 0, B level 1 (top level with A)", function()
		Part("A", C0, V(2, 2, 2))
		Part("B", C0 + V(0, -14, 0), V(30, 30, 30))
	end)
end

-- 7. a WorldModel: its own grid, which the Workspace's queries do not see
do
	local World = Instance.new("WorldModel")
	World.Parent = workspace
	local B = V(2000, 1000, 1000)
	for Z = 1, 0, -1 do
		for Y = 1, 0, -1 do
			for X = 1, 0, -1 do
				Part(X .. Y .. Z, B + V(4 + 8 * X, 4 + 8 * Y, 4 + 8 * Z), nil, World)
			end
		end
	end
	print(`WorldModel, 2 x 2 x 2 cells made from the far corner: {Listed(World:GetPartBoundsInBox(CFrame.new(B + V(8, 8, 8)), V(15, 15, 15)))}`)
	print(`WorldModel, the Workspace's query there finds: {Listed(workspace:GetPartBoundsInBox(CFrame.new(B + V(8, 8, 8)), V(15, 15, 15)))}`)
	World:ClearAllChildren()
	for _, S in { 2048, 1024, 512, 256, 128, 64, 32, 1 } do
		Part("K" .. S, C0, V(S, S, S), World)
	end
	print(`WorldModel, levels, query 0.01: {Listed(World:GetPartBoundsInBox(CFrame.new(C0), V(0.01, 0.01, 0.01)))}`)
	print(`WorldModel, levels, query 100: {Listed(World:GetPartBoundsInBox(CFrame.new(C0), V(100, 100, 100)))}`)
	World:ClearAllChildren()
	local A = {}
	for I = 1, 5 do
		A[I] = Part("a" .. I, C0 + V(-2 + I * 0.6, 0, 0), nil, World)
	end
	World:GetPartBoundsInBox(CFrame.new(0, -10000, 0), Vector3.one)
	A[2]:Destroy()
	print(`WorldModel, a2 destroyed: {Listed(World:GetPartBoundsInBox(CFrame.new(C0), V(6, 1, 1)))}`)
	World:Destroy()
end
Root:Destroy()
workspace.Gravity = Gravity
