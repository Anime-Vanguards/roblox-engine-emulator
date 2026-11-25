-- The order a moved subtree's parts come into a spatial-grid cell's lists and leave them (a client's broadphase): by the
-- tree's shape (parts holding parts, Models and Folders between them, nesting), and by how the subtree comes in (made in
-- nil, through ReplicatedStorage, back from nil with or without a query between, back from ReplicatedStorage, a clone of
-- it) or goes out (to nil, destroyed, to ReplicatedStorage, into a WorldModel). All the parts sit in one cell, so a box
-- query lists them in the cell's order. A tree's text: a name, then its children in brackets; names starting F are
-- Folders, M Models, the rest anchored Parts.
P.sync()
local Gravity = workspace.Gravity
workspace.Gravity = 0
local Storage = game:GetService("ReplicatedStorage")
local Root = Instance.new("Folder")
Root.Name = "GridSubtree"
Root.Parent = workspace
local Mine = {}
local V = Vector3.new
local C0 = V(2004, 1004, 2004) -- mid-cell (cells 2000..2008)
local Slot = 0
local function Part(Name, Parent)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = true
	Made.Size = V(0.2, 0.2, 0.2)
	Slot += 1
	Made.CFrame = CFrame.new(C0 + V(-2.6 + (Slot % 18) * 0.3, 0, 0))
	Made.Parent = Parent
	Mine[Made] = true
	return Made
end
local function Index(World)
	(World or workspace):GetPartBoundsInBox(CFrame.new(0, -10000, 0), Vector3.one)
end
local function Listed(World)
	local Names = {}
	for _, Found in (World or workspace):GetPartBoundsInBox(CFrame.new(C0), V(6, 1, 1)) do
		if Mine[Found] then
			table.insert(Names, Found.Name)
		end
	end
	return table.concat(Names, " ")
end
-- a tree from its text, each instance parented as it is made (so a tree made in place comes in one instance at a time)
local function Build(Text, Parent)
	local At = 1
	local function Node(Into)
		local Name = string.match(Text, "^%w+", At)
		At += #Name
		local Made
		local Kind = string.sub(Name, 1, 1)
		if Kind == "F" or Kind == "M" then
			Made = Instance.new(if Kind == "F" then "Folder" else "Model")
			Made.Name = Name
			Made.Parent = Into
		else
			Made = Part(Name, Into)
		end
		if string.sub(Text, At, At) == "(" then
			At += 1
			while true do
				Node(Made)
				local Next = string.sub(Text, At, At)
				At += 1
				if Next == ")" then
					break
				end
			end
		end
		return Made
	end
	return Node(Parent)
end
local function Clear()
	Root:ClearAllChildren()
	table.clear(Mine)
	Slot = 0
end

-- 1. coming in: x is in the cell first, then the tree (made in nil) comes in as one move
local Trees = { "p(q)", "p(q,r)", "p(q(r))", "p(q(r),s)", "p(q,r(s),t)", "p(q(r(s)))", "M(p(q),r)", "F(p(q),r)", "p(F(q,r),s)", "F(p(q(r)),M(s(t)))" }
local Ins = {
	{ "from nil", function(T) T.Parent = Root end },
	{ "from ReplicatedStorage", function(T) T.Parent = Storage Index() T.Parent = Root end },
	{ "back from nil, a query between", function(T) T.Parent = Root Index() T.Parent = nil Index() T.Parent = Root end },
	{ "back from nil, no query between", function(T) T.Parent = Root Index() T.Parent = nil T.Parent = Root end },
	{ "back from ReplicatedStorage", function(T) T.Parent = Root Index() T.Parent = Storage Index() T.Parent = Root end },
	{ "a clone of it", function(T)
		T.Parent = Root
		Index()
		local Copy = T:Clone()
		T:Destroy()
		for _, Each in Copy:GetDescendants() do
			if Each:IsA("BasePart") then
				Mine[Each] = true
			end
		end
		if Copy:IsA("BasePart") then
			Mine[Copy] = true
		end
		Copy.Parent = Root
	end },
}
for _, Tree in Trees do
	for _, Mode in Ins do
		Part("x", Root)
		Index()
		Mode[2](Build(Tree, nil))
		Index()
		print(`in, {Tree} {Mode[1]}: {Listed()}`)
		Clear()
	end
end
do
	Part("x", Root)
	Index()
	local T = Part("p", Root)
	local Q = Part("q", nil)
	Part("r", Q)
	Q.Parent = T
	Index()
	print(`in, p, then q(r) into p: {Listed()}`)
	Clear()
end
do
	Part("x", Root)
	local T = Build("p(q)", Root)
	Part("y", Root)
	Index()
	local M = Instance.new("Model")
	M.Parent = Root
	T.Parent = M
	Index()
	print(`in, x p(q) y, then p into a Model in the Workspace: {Listed()}`)
	Clear()
end
do
	Part("x", Root)
	Index()
	local A = Build("p(q)", nil)
	local B = Build("r(s)", nil)
	A.Parent = Root
	B.Parent = Root
	Part("y", Root)
	Index()
	print(`in, p(q) then r(s) then y in one frame: {Listed()}`)
	Clear()
end
do
	Part("x", Root)
	Index()
	local T = Build("p(q(r),s)", nil)
	for _, Each in T:GetDescendants() do
		Each.Anchored = false
	end
	T.Anchored = false
	T.Parent = Root
	Index()
	print(`in, p(q(r),s) unanchored, from nil: {Listed()}`)
	T.Parent = nil
	Index()
	T.Parent = Root
	Index()
	print(`in, p(q(r),s) unanchored, back from nil: {Listed()}`)
	Clear()
end
do
	local World = Instance.new("WorldModel")
	World.Parent = workspace
	Part("x", World)
	Index(World)
	local T = Build("p(q(r),s)", nil)
	T.Parent = World
	Index(World)
	print(`in, a WorldModel, p(q(r),s) from nil: {Listed(World)}`)
	T.Parent = nil
	Index(World)
	T.Parent = World
	Index(World)
	print(`in, a WorldModel, p(q(r),s) back from nil: {Listed(World)}`)
	World:Destroy()
	Clear()
end

-- 2. going out: x, the tree made in place (one instance at a time), then a to e are in the cell in that order
local World = Instance.new("WorldModel")
World.Parent = workspace
local Outs = {
	{ "to nil", function(T) T.Parent = nil end },
	{ "destroyed", function(T) T:Destroy() end },
	{ "to ReplicatedStorage", function(T) T.Parent = Storage end },
	{ "into a WorldModel", function(T) T.Parent = World end },
}
for _, Tree in { "p(q)", "p(q,r)", "p(q(r))", "p(q(r),s)", "M(p(q),r)", "p(F(q,r),s)" } do
	for _, Mode in Outs do
		Part("x", Root)
		local T = Build(Tree, Root)
		for _, Name in { "a", "b", "c", "d", "e" } do
			Part(Name, Root)
		end
		Index()
		Mode[2](T)
		Index()
		print(`out, x {Tree} a b c d e, {Tree} {Mode[1]}: {Listed()}`)
		T:Destroy()
		Clear()
	end
end
World:Destroy()
Root:Destroy()
workspace.Gravity = Gravity
