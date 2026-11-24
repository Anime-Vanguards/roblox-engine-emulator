-- The order a box query walks the spatial grid's cells in (a client's broadphase): one 0.2-stud part at the
-- centre of each cell of a block, made in a shuffled order, named by its cell (x,y,z in cells of 8 studs), and one query
-- over the block; blocks across powers of two and zero, spreads, queries of growing size over one grid, coarse parts
-- in two clusters, the levels above the query's, the top level the walk starts at (raised the frame a part of level 6
-- comes in, lowered a frame after the last one leaves), and a query whose low edge lies on a cell edge.
P.sync()
local Root = Instance.new("Folder")
Root.Name = "GridWalk"
Root.Parent = workspace
local Mine = {}
local function Part(Name, At, Size)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = true
	Made.Size = Size or Vector3.new(0.2, 0.2, 0.2)
	Made.CFrame = if typeof(At) == "CFrame" then At else CFrame.new(At)
	Made.Parent = Root
	Mine[Made] = true
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
local function Span(Low, High)
	return workspace:GetPartBoundsInBox(CFrame.new((Low + High) / 2), High - Low)
end
local V = Vector3.new
local Rng = P.rng(7)
local function Shuffled(List)
	for I = #List, 2, -1 do
		local J = Rng.int(1, I)
		List[I], List[J] = List[J], List[I]
	end
	return List
end
local function Cells(X0, X1, Y0, Y1, Z0, Z1, Step)
	local Out = {}
	for X = X0, X1, Step or 1 do
		for Y = Y0, Y1, Step or 1 do
			for Z = Z0, Z1, Step or 1 do
				table.insert(Out, { X, Y, Z })
			end
		end
	end
	return Out
end
local function Fill(List)
	for _, C in Shuffled(List) do
		Part(`{C[1]},{C[2]},{C[3]}`, V(C[1] * 8 + 4, C[2] * 8 + 4, C[3] * 8 + 4))
	end
	Index()
end
-- a query from 1 stud inside cell (X0, Y0, Z0) to 1 stud inside cell (X1, Y1, Z1)
local function Over(X0, X1, Y0, Y1, Z0, Z1)
	return Listed(Span(V(X0 * 8 + 1, Y0 * 8 + 1, Z0 * 8 + 1), V(X1 * 8 + 7, Y1 * 8 + 7, Z1 * 8 + 7)))
end
local function Block(Label, X0, X1, Y0, Y1, Z0, Z1, Step)
	Fill(Cells(X0, X1, Y0, Y1, Z0, Z1, Step))
	print(`{Label}: {Over(X0, X1, Y0, Y1, Z0, Z1)}`)
	Clear()
end

-- 1. blocks of cells: across 512, 128 and 1024; across zero on every axis; negative; spreads; lines
Block("block across 512 128 1024", 510, 513, 126, 129, 1022, 1025)
Block("block across 0", -2, 1, 123, 126, -2, 1)
Block("block across y 0", 200, 201, -2, 1, 200, 201)
Block("block negative", -515, -510, 130, 131, -1027, -1022)
Block("spread 8 across 0", -16, 16, 130, 130, -16, 16, 8)
Block("spread 25", 125, 175, 125, 125, 125, 175, 25)
Block("spread 25 elsewhere", 100, 150, 130, 130, 300, 350, 25)
Block("line x", 1000, 1040, 130, 130, 200, 200, 4)
Block("line y across 0", 200, 200, -20, 20, 200, 200, 4)

-- 2. one 16 x 16 grid of cells (x and z 256..271), queries of several extents over it
Fill(Cells(256, 271, 125, 125, 256, 271))
for _, Q in { { 256, 263, 256, 263 }, { 257, 264, 257, 264 }, { 259, 267, 259, 267 }, { 260, 266, 260, 266 }, { 258, 262, 258, 270 }, { 261, 263, 261, 263 }, { 257, 271, 257, 271 } } do
	print(`grid x {Q[1]}..{Q[2]} z {Q[3]}..{Q[4]}: {Over(Q[1], Q[2], 125, 125, Q[3], Q[4])}`)
end
Clear()

-- 3. levels inside the walk: coarse and fine parts in two places, one query over both
do
	local Y, Z = 1004, 1004
	Part("L4", V(1088, Y, Z), V(300, 300, 300))
	Part("L3", V(1056, Y, Z), V(200, 200, 200))
	Part("A1", V(1040, Y, Z), V(30, 30, 30))
	Part("A0", V(1040, Y, Z))
	Part("B1", V(1110, Y, Z), V(30, 30, 30))
	Part("B0", V(1092, Y, Z))
	Index()
	for _, Q in { { 1028, 1093 }, { 1028, 1120 }, { 1036, 1100 } } do
		print(`coarse and fine, query x {Q[1]}..{Q[2]}: {Listed(Span(V(Q[1], Y - 1, Z - 1), V(Q[2], Y + 1, Z + 1)))}`)
	end
	Clear()
	for K = 0, 1 do
		local At = V(2004 + 200 * K, 1004, 2004)
		Part(`c{K}L0`, At)
		Part(`c{K}L1`, At, V(30, 30, 30))
		Part(`c{K}L2`, At, V(100, 100, 100))
	end
	Index()
	print(`two clusters, query 400: {Listed(Span(V(1904, 1003.5, 2003.5), V(2304, 1004.5, 2004.5)))}`)
	Clear()
end

-- 4. the levels above the query's: four parts of level 4 about a corner, after an L3 query
do
	local Y = 1028
	for _, C in { { 9, 9 }, { 9, 8 }, { 8, 9 }, { 8, 8 } } do
		Part(`P4({C[1]},{C[2]})`, V(C[1] * 128 + 64, Y, C[2] * 128 + 64), V(100, 2048, 100))
	end
	Part("L3", V(1152, Y, 1152), V(0.2, 2048, 300))
	Part("L0", V(1150, Y, 1150))
	Index()
	print(`level-4 cells after a level-3 query: {Listed(workspace:GetPartBoundsInBox(CFrame.new(1152, Y, 1152), V(64, 1, 64)))}`)
	Clear()
end

-- 5. the top level: a pattern of parts (x, z offsets 0 1 2 4 8 16 32 48 cells) whose order tells the walk's blocks,
-- queried with a level-6 query at the start, with a level-6 part far away, the frame it leaves, a frame later
local Patterns = 0
local function Pattern(Label, Offsets)
	Offsets = Offsets or { 0, 1, 2, 4, 8, 16, 32, 48 }
	Patterns += 1
	local Base = 256 + Patterns * 256
	local Last = Offsets[#Offsets]
	local List = {}
	for _, X in Offsets do
		for _, Z in Offsets do
			table.insert(List, { Base + X, 125, Base + Z })
		end
	end
	local Made = {}
	for _, C in Shuffled(List) do
		table.insert(Made, Part(`{C[1] - Base},{C[3] - Base}`, V(C[1] * 8 + 4, C[2] * 8 + 4, C[3] * 8 + 4)))
	end
	Index()
	print(`{Label}: {Listed(Span(V(Base * 8 + 1, 1001, Base * 8 + 1), V((Base + Last) * 8 + 7, 1007, (Base + Last) * 8 + 7)))}`)
	for _, Done in Made do
		Done:Destroy()
		Mine[Done] = nil
	end
end
task.wait()
task.wait()
Pattern("top, at the start")
Pattern("top, at the start, a level-7 query", { 0, 1, 2, 4, 8, 16, 32, 64, 96 })
local Far = V(-20000, 1000, -20000)
local Six = Part("six", Far, V(2048, 2048, 2048))
Index()
Pattern("top, a level-6 part far away this frame")
task.wait()
task.wait()
Pattern("top, a level-6 part far away, two frames on")
Six:Destroy()
Pattern("top, the level-6 part destroyed this frame")
task.wait()
Pattern("top, a frame after it went")
local Seven = Part("seven", CFrame.new(Far) * CFrame.Angles(math.rad(45), math.rad(45), 0), V(2048, 2048, 2048))
Index()
Pattern("top, a level-7 part far away this frame")
Pattern("top, a level-7 part far away this frame, a level-7 query", { 0, 1, 2, 4, 8, 16, 32, 64, 96 })
Seven:Destroy()
task.wait()
task.wait()

-- 6. a query's low edge on a cell edge also takes the cell below: M (one cell) made before P (that cell and the one below)
do
	local E = 2048
	Part("M", V(E + 8, 1004, 1004), V(0.2, 200, 0.2))
	Part("P", V(E, 1004, 1004), V(0.2, 200, 0.2))
	Index()
	print(`level 1, query from {E}: {Listed(Span(V(E, 1003, 1003), V(E + 12, 1005, 1005)))}`)
	print(`level 1, query from {E}.001: {Listed(Span(V(E + 0.001, 1003, 1003), V(E + 12, 1005, 1005)))}`)
	Clear()
	Part("M", V(E + 4, 1004, 1004))
	Part("P", V(E, 1004, 1004))
	Index()
	print(`level 0, query from {E}: {Listed(Span(V(E, 1003, 1003), V(E + 6, 1005, 1005)))}`)
	print(`level 0, query from {E}.001: {Listed(Span(V(E + 0.001, 1003, 1003), V(E + 6, 1005, 1005)))}`)
	Clear()
end
Root:Destroy()
