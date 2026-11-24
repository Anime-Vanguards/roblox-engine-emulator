-- Random scenes against the spatial grid's order (a model of the grid's rules reproduced every line of a
-- 120-query run of this kind): parts of random size, place, turn and anchoring, made in one frame and taken in by one
-- query, then random query boxes, each line the names a box lists. The third scene comes in the frame the second one
-- (with parts of level 6) left, so its walk starts at level 6.
P.sync()
local Gravity = workspace.Gravity
workspace.Gravity = 0
local Root = Instance.new("Folder")
Root.Name = "GridRandom"
Root.Parent = workspace
local Mine = {}
local V = Vector3.new
local Rng = P.rng(20261124)
local function LogU(Low, High)
	return math.exp(Rng.real(math.log(Low), math.log(High)))
end
local function Turn()
	if Rng.real(0, 1) < 0.35 then
		return CFrame.Angles(Rng.real(0, 6.283), Rng.real(0, 6.283), Rng.real(0, 6.283))
	end
	return CFrame.identity
end
local function Scene(Label, Center, Spread, Count, MaxSize, Big)
	for I = 1, Count do
		local Size = V(LogU(0.2, MaxSize), LogU(0.2, MaxSize), LogU(0.2, MaxSize))
		if Big and I % 25 == 0 then
			Size = V(LogU(600, 2048), LogU(600, 2048), LogU(600, 2048))
		end
		local At = Center + V(Rng.real(-Spread, Spread), Rng.real(-Spread, Spread), Rng.real(-Spread, Spread))
		local Made = Instance.new("Part")
		Made.Name = "p" .. I
		Made.Size = Size
		Made.CFrame = CFrame.new(At) * Turn()
		Made.Anchored = Rng.real(0, 1) >= 0.3
		Made.Parent = Root
		Mine[Made] = true
	end
	workspace:GetPartBoundsInBox(CFrame.new(0, -10000, 0), Vector3.one)
	for I = 1, 15 do
		local Size = if I % 3 == 0 then V(LogU(1, 40), LogU(1, 40), LogU(1, 40)) else V(LogU(1, 300), LogU(1, 300), LogU(1, 300))
		local At = Center + V(Rng.real(-Spread, Spread), Rng.real(-Spread, Spread), Rng.real(-Spread, Spread))
		local Names = {}
		for _, Found in workspace:GetPartBoundsInBox(CFrame.new(At) * Turn(), Size) do
			if Mine[Found] then
				table.insert(Names, Found.Name)
			end
		end
		print(`{Label} query {I}: {table.concat(Names, " ")}`)
	end
	Root:ClearAllChildren()
	table.clear(Mine)
end
Scene("scene 1", V(5003.7, 1001.3, 4998.1), 150, 150, 60, false)
Scene("scene 2", V(-3001.9, 2003.3, 7002.9), 300, 150, 200, true)
Scene("scene 3", V(12007.3, 1000.7, -9003.1), 50, 200, 20, false)
Root:Destroy()
workspace.Gravity = Gravity
