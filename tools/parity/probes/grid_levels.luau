-- The spatial queries' grid (a client's broadphase), its levels and cells: the level each part is filed on (one marker
-- part per level at one point, made biggest first, then the parts under test; a tiny box query there lists each level's
-- cell in turn), the size where a shape moves up a level (bisected), the margin a part's box is grown by on each level,
-- for turned parts, the cell size of each level (parts whose low edge steps along x, made from the top down: a run in
-- made order is one cell), and the level a query starts its walk at. Scenes are 1,000 studs and more from the origin.
P.sync()
local Root = Instance.new("Folder")
Root.Name = "GridLevels"
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
local function Box(At, Size)
	return workspace:GetPartBoundsInBox(if typeof(At) == "CFrame" then At else CFrame.new(At), Size)
end
local function Span(Low, High)
	return workspace:GetPartBoundsInBox(CFrame.new((Low + High) / 2), High - Low)
end
local V, Cube = Vector3.new, function(S)
	return Vector3.new(S, S, S)
end
local function F32(X)
	return Vector3.new(X, 0, 0).X
end

-- 1. the level of cubes, rods and plates: markers K<size> (each level's), then the parts, at one point
local P0 = V(4100, 1028, 4100) -- 4 studs past a cell edge on every level
local function Markers()
	for _, S in { 2048, 1024, 512, 256, 128, 64, 32, 16, 1 } do
		Part("K" .. S, P0, Cube(S))
	end
end
Markers()
Index()
print(`markers: {Listed(Box(P0, Cube(0.01)))}`)
for _, S in { 1, 8, 16, 20.5, 21, 32, 56, 60, 128, 144, 280, 300, 512, 600, 1024, 1200, 2048 } do
	Part("c" .. S, P0, Cube(S))
end
Index()
print(`cubes: {Listed(Box(P0, Cube(0.01)))}`)
Clear()
Markers()
for _, S in { 128, 200, 1024, 2048 } do
	Part("rod" .. S, P0, V(S, 1, 1))
end
for _, S in { 36, 40, 128, 200, 400, 512, 1024, 2048 } do
	Part("plate" .. S, P0, V(S, 1, S))
end
Index()
print(`rods and plates: {Listed(Box(P0, Cube(0.01)))}`)
Clear()

-- 2. the size where a shape moves up a level, bisected on float32 sizes
Markers()
local MarkerLevel = { K1 = 0, K16 = 0, K32 = 1, K64 = 2, K128 = 2, K256 = 3, K512 = 4, K1024 = 5, K2048 = 6 }
local function LevelOf(Size)
	local Made = Part("T", P0, Size)
	Index()
	local Level = -1
	for _, Found in Box(P0, Cube(0.01)) do
		if Found == Made then
			break
		end
		if Mine[Found] and MarkerLevel[Found.Name] then
			Level = MarkerLevel[Found.Name]
		end
	end
	Made:Destroy()
	Mine[Made] = nil
	return Level
end
for _, Family in {
	{ "cube", function(S) return Cube(S) end },
	{ "rod", function(S) return V(S, 1, 1) end },
	{ "thin rod", function(S) return V(S, 0.1, 0.1) end },
	{ "plate", function(S) return V(S, 1, S) end },
	{ "box", function(S) return V(S, 2, 5) end },
} do
	local Line = {}
	for Level = 0, 5 do
		local Low, High = 0.01, 2048
		if LevelOf(Family[2](High)) <= Level then
			break
		end
		for _ = 1, 40 do
			local Mid = F32((Low + High) / 2)
			if Mid == Low or Mid == High then
				break
			end
			if LevelOf(Family[2](Mid)) <= Level then
				Low = Mid
			else
				High = Mid
			end
		end
		table.insert(Line, `{Level}|{Level + 1} {P.f32(Low)}..{P.f32(High)}`)
	end
	print(`{Family[1]} level steps: {table.concat(Line, ", ")}`)
end
Clear()

-- 3. turned parts: a 2048 cube turned 45 degrees about x and y is on the level above 2048 cubes
Part("K2048", P0, Cube(2048))
Part("K1024", P0, Cube(1024))
Part("K1", P0, Cube(1))
Part("turned2048", CFrame.new(P0) * CFrame.Angles(math.rad(45), math.rad(45), 0), Cube(2048))
Part("turned20", CFrame.new(P0) * CFrame.Angles(0, math.rad(45), 0), Cube(20))
Part("turned16", CFrame.new(P0) * CFrame.Angles(0, math.rad(45), 0), Cube(16))
Index()
for _, S in { 0.01, 300, 600 } do
	print(`turned parts, query {S}: {Listed(Box(P0, Cube(S)))}`)
end
Clear()

-- 4. margins: parts of one level whose low x edge is D past a cell edge, markers A (the cell below) and B (the cell at the
-- edge) made first; the part is in A's cell while it lists before B. Bisected per level; then turned parts at small
-- coordinates (float32 steps of 2e-6), per axis, as the excess over 0.3
local Base = V(40960, 4100, 40960)
for Level, Family in {
	V(0.2, 0.2, 0.2), V(0.2, 200, 0.2), V(0.2, 1500, 0.2), V(0.2, 2048, 300), V(0.2, 2048, 2048), V(150, 2048, 2048), V(2048, 2048, 2048),
} do
	local Cell = 8 * 2 ^ (Level - 1)
	local function At(LowX)
		return V(LowX + Family.X / 2, Base.Y, Base.Z)
	end
	Part("A", At(Base.X - Cell + 1), Family)
	Part("B", At(Base.X + Cell / 4), Family)
	local function InA(D)
		local Made = Part("T", At(Base.X + D), Family)
		Index()
		local S = Listed(Span(V(Base.X - Cell + 0.5, Base.Y - 0.01, Base.Z - 0.01), V(Base.X + Cell - 0.5, Base.Y + 0.01, Base.Z + 0.01)))
		Made:Destroy()
		Mine[Made] = nil
		return S == "A T B"
	end
	local Low, High = 0, Cell / 4
	for _ = 1, 30 do
		local Mid = F32((Low + High) / 2)
		if Mid == Low or Mid == High then
			break
		end
		if InA(Mid) then
			Low = Mid
		else
			High = Mid
		end
	end
	print(`margin L{Level - 1}: {P.f32(Low)}..{P.f32(High)}`)
	Clear()
end
do
	local Small = V(16, 1028, 4100)
	for _, Case in {
		{ "none", V(4, 1, 1), CFrame.identity },
		{ "y30", V(4, 1, 1), CFrame.Angles(0, math.rad(30), 0) },
		{ "y45", V(4, 1, 1), CFrame.Angles(0, math.rad(45), 0) },
		{ "y90", V(4, 1, 1), CFrame.Angles(0, math.rad(90), 0) },
		{ "xyz", V(1.5, 3, 2), CFrame.Angles(math.rad(20), math.rad(50), math.rad(70)) },
	} do
		local Size, Turn = Case[2], Case[3]
		local Out = {}
		for Axis = 1, 3 do
			local Unit = if Axis == 1 then V(1, 0, 0) elseif Axis == 2 then V(0, 1, 0) else V(0, 0, 1)
			local Edge = Small:Dot(Unit)
			local E = math.floor(Edge / 8) * 8 + 8
			local Rows = { Turn.XVector, Turn.YVector, Turn.ZVector }
			local Half = (math.abs(Rows[1]:Dot(Unit)) * Size.X + math.abs(Rows[2]:Dot(Unit)) * Size.Y + math.abs(Rows[3]:Dot(Unit)) * Size.Z) / 2
			Part("A", Small + Unit * (E - 4 - Edge))
			Part("B", Small + Unit * (E + 4 - Edge))
			local function InA(D)
				local Made = Part("T", CFrame.new(Small + Unit * (E + D + Half - Edge)) * Turn, Size)
				Index()
				local Lo = Small + Unit * (E - 7 - Edge) - (V(1, 1, 1) - Unit) * 0.01
				local Hi = Small + Unit * (E + 7 - Edge) + (V(1, 1, 1) - Unit) * 0.01
				local S = Listed(Span(Lo, Hi))
				Made:Destroy()
				Mine[Made] = nil
				return S == "A T B"
			end
			local Low, High = 0, 2
			for _ = 1, 26 do
				local Mid = (Low + High) / 2
				if InA(Mid) then
					Low = Mid
				else
					High = Mid
				end
			end
			table.insert(Out, string.format("%.4f", Low - 0.3))
			Clear()
		end
		print(`turned margin {Case[1]}, excess over 0.3 on x y z: {table.concat(Out, " ")}`)
	end
end

-- 5. cell sizes: per level, parts whose low x edge steps by an eighth of a cell, made from the highest down
for Level, Family in {
	V(0.2, 0.2, 0.2), V(0.2, 200, 0.2), V(0.2, 1500, 0.2), V(0.2, 2048, 300), V(0.2, 2048, 2048), V(150, 2048, 2048), V(2048, 2048, 2048),
} do
	local Step = 2 ^ (Level - 1)
	for J = 16, -4, -1 do
		local O = J * Step + 0.5
		Part(string.format("%g", O), V(Base.X + O + Family.X / 2, Base.Y, Base.Z), Family)
	end
	Index()
	print(`cells L{Level - 1}, low edges by {Step}: {Listed(Span(V(Base.X - 5 * Step - 1, Base.Y - 0.01, Base.Z - 0.01), V(Base.X + 17 * Step + 2, Base.Y + 0.01, Base.Z + 0.01)))}`)
	Clear()
end

-- 6. the level a query starts at: the first marker it names
for _, S in { 2048, 1024, 512, 256, 128, 64, 32, 1 } do
	Part("K" .. S, P0, Cube(S))
end
Index()
for _, S in { 8, 8.001, 16, 16.001, 32, 32.01, 64, 64.01, 128, 128.01, 256, 256.01, 512.01 } do
	print(`query cube {S}: {Listed(Box(P0, Cube(S)))}`)
end
for _, S in { V(20, 1, 1), V(1, 1, 20), V(12, 12, 12), V(40, 4, 4), V(300, 1, 1) } do
	print(`query box {S}: {Listed(Box(P0, S))}`)
end
for _, L in { 10, 12, 20, 22.7 } do
	print(`query {L}x1x1 turned 45 about y: {Listed(Box(CFrame.new(P0) * CFrame.Angles(0, math.rad(45), 0), V(L, 1, 1)))}`)
end
for _, R in { 4, 4.01, 8, 8.01, 100 } do
	print(`query radius {R}: {Listed(workspace:GetPartBoundsInRadius(P0, R))}`)
end
for _, L in { 7.5, 8, 15.4, 16 } do
	local Probe = Instance.new("Part")
	Probe.Anchored = true
	Probe.Size = V(L, 0.5, 0.5)
	Probe.CFrame = CFrame.new(P0)
	Probe.Parent = workspace
	print(`query GetPartsInPart {L}x0.5x0.5: {Listed(workspace:GetPartsInPart(Probe))}`)
	Probe:Destroy()
end
Root:Destroy()
