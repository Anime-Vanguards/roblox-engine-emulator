-- Cast edges (world/query.rs): a cast that touches a part only at its path's end or its start, rays along a face or an edge,
-- Blockcast's hit Position for boxes on and beside a face (off centre, coinciding edges, larger than the part, turned, of a
-- negative size) and Spherecast against a part's edges and corners. The scene is 300 studs out.
do
	P.sync()
	local Root = Instance.new("Folder")
	Root.Parent = workspace
	local function Part(Name, Position, Size, Shape)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Anchored = true
		if Shape then
			Made.Shape = Shape
		end
		Made.Size = Size
		Made.Position = Position
		Made.Parent = Root
		return Made
	end
	-- Wall: top face y = 90.5, x 295..305, z 295..305
	local Wall = Part("Wall", Vector3.new(300, 90, 300), Vector3.new(10, 1, 10))
	local function F(Value)
		return string.format("%.4f", Value)
	end
	local function Show(Result)
		if not Result then
			return "nil"
		end
		return `{Result.Instance.Name} pos {F(Result.Position.X)} {F(Result.Position.Y)} {F(Result.Position.Z)} n {F(Result.Normal.X)} {F(Result.Normal.Y)} {F(Result.Normal.Z)} d {F(Result.Distance)}`
	end
	local function Try(Label, F2)
		local Ok, Result = pcall(F2)
		print(Label, if Ok then Show(Result) else "error " .. P.err(Result))
	end

	-- Blockcast positions: sizes and offsets
	for _, Size in { Vector3.new(1, 1, 1), Vector3.new(2, 1, 4), Vector3.new(-1, 1, 1), Vector3.new(-1, -1, -1), Vector3.new(1, -1, 1), Vector3.new(-2, 1, -4), Vector3.new(0, 0, 0), Vector3.new(-1, 0, 0) } do
		Try(`block size {Size}`, function() return workspace:Blockcast(CFrame.new(300, 100, 300), Size, Vector3.new(0, -20, 0)) end)
		Try(`block size {Size} turned`, function() return workspace:Blockcast(CFrame.new(300, 100, 300) * CFrame.Angles(0, math.rad(30), 0), Size, Vector3.new(0, -20, 0)) end)
		Try(`block size {Size} offcentre`, function() return workspace:Blockcast(CFrame.new(304.8, 100, 300), Size, Vector3.new(0, -20, 0)) end)
	end
	-- Blockcast tilted onto the wall (an edge first)
	Try("block tilted", function() return workspace:Blockcast(CFrame.new(300, 100, 300) * CFrame.Angles(0, 0, math.rad(45)), Vector3.new(2, 2, 2), Vector3.new(0, -20, 0)) end)
	Try("block tilted neg", function() return workspace:Blockcast(CFrame.new(300, 100, 300) * CFrame.Angles(0, 0, math.rad(45)), Vector3.new(-2, 2, 2), Vector3.new(0, -20, 0)) end)
	Try("block side", function() return workspace:Blockcast(CFrame.new(290, 90, 300), Vector3.new(1, 1, 1), Vector3.new(10, 0, 0)) end)
	Try("block side neg", function() return workspace:Blockcast(CFrame.new(290, 90, 300), Vector3.new(-1, -1, -1), Vector3.new(10, 0, 0)) end)

	-- ends: the path's end exactly touching the top face (y 90.5)
	Try("ray end touch", function() return workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -9.5, 0)) end)
	Try("ray end past", function() return workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -9.50001, 0)) end)
	Try("ray start on face", function() return workspace:Raycast(Vector3.new(300, 90.5, 300), Vector3.new(0, -5, 0)) end)
	Try("ray start on face up", function() return workspace:Raycast(Vector3.new(300, 90.5, 300), Vector3.new(0, 5, 0)) end)
	Try("sphere end touch", function() return workspace:Spherecast(Vector3.new(300, 100, 300), 0.5, Vector3.new(0, -9, 0)) end)
	Try("sphere end past", function() return workspace:Spherecast(Vector3.new(300, 100, 300), 0.5, Vector3.new(0, -9.001, 0)) end)
	Try("sphere end short", function() return workspace:Spherecast(Vector3.new(300, 100, 300), 0.5, Vector3.new(0, -8.999, 0)) end)
	Try("block end touch", function() return workspace:Blockcast(CFrame.new(300, 100, 300), Vector3.new(1, 1, 1), Vector3.new(0, -9, 0)) end)
	Try("block end past", function() return workspace:Blockcast(CFrame.new(300, 100, 300), Vector3.new(1, 1, 1), Vector3.new(0, -9.001, 0)) end)
	Try("block end short", function() return workspace:Blockcast(CFrame.new(300, 100, 300), Vector3.new(1, 1, 1), Vector3.new(0, -8.999, 0)) end)
	Try("block start touch", function() return workspace:Blockcast(CFrame.new(300, 91, 300), Vector3.new(1, 1, 1), Vector3.new(0, -5, 0)) end)
	Try("block start touch up", function() return workspace:Blockcast(CFrame.new(300, 91, 300), Vector3.new(1, 1, 1), Vector3.new(0, 5, 0)) end)
	Try("sphere start touch", function() return workspace:Spherecast(Vector3.new(300, 91, 300), 0.5, Vector3.new(0, -5, 0)) end)
	Try("block start in", function() return workspace:Blockcast(CFrame.new(300, 90.9, 300), Vector3.new(1, 1, 1), Vector3.new(0, -5, 0)) end)
	local Caster = Part("Caster", Vector3.new(300, 100, 300), Vector3.new(1, 1, 1))
	Caster.CanQuery = false
	Caster.CanCollide = false
	Try("shape end touch", function() return workspace:Shapecast(Caster, Vector3.new(0, -9, 0)) end)
	Try("shape end past", function() return workspace:Shapecast(Caster, Vector3.new(0, -9.001, 0)) end)
	local Ball = Part("Ball", Vector3.new(300, 100, 300), Vector3.new(1, 1, 1), Enum.PartType.Ball)
	Ball.CanQuery = false
	Ball.CanCollide = false
	Try("shape ball end touch", function() return workspace:Shapecast(Ball, Vector3.new(0, -9, 0)) end)
	Try("shape ball end past", function() return workspace:Shapecast(Ball, Vector3.new(0, -9.001, 0)) end)
	-- side edges: a sphere grazing the wall's side face at its path's end
	Try("sphere side end touch", function() return workspace:Spherecast(Vector3.new(290, 90, 300), 1, Vector3.new(4, 0, 0)) end)
	Try("sphere side end past", function() return workspace:Spherecast(Vector3.new(290, 90, 300), 1, Vector3.new(4.01, 0, 0)) end)
	-- grazing along a face (parallel)
	Try("block graze", function() return workspace:Blockcast(CFrame.new(290, 91, 300), Vector3.new(1, 1, 1), Vector3.new(20, 0, 0)) end)
	Try("sphere graze", function() return workspace:Spherecast(Vector3.new(290, 91, 300), 0.5, Vector3.new(20, 0, 0)) end)
	Try("ray graze", function() return workspace:Raycast(Vector3.new(290, 90.5, 300), Vector3.new(20, 0, 0)) end)
	Root:Destroy()
end
do
	P.sync()
	local Root = Instance.new("Folder")
	Root.Parent = workspace
	local function Part(Name, CF, Size)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Anchored = true
		Made.Size = Size
		Made.CFrame = CF
		Made.Parent = Root
		return Made
	end
	-- Wall: top face y = 90.5, x 295..305, z 295..305
	Part("Wall", CFrame.new(300, 90, 300), Vector3.new(10, 1, 10))
	-- Small: a 2x1x2 block at (400, 90, 300); Tilted: a wall turned 30 degrees about y at (500, 90, 300)
	Part("Small", CFrame.new(400, 90, 300), Vector3.new(2, 1, 2))
	Part("Turned", CFrame.new(500, 90, 300) * CFrame.Angles(0, math.rad(30), 0), Vector3.new(10, 1, 6))
	local function F(Value)
		return string.format("%.4f", Value)
	end
	local function Show(Result)
		if not Result then
			return "nil"
		end
		return `{Result.Instance.Name} pos {F(Result.Position.X)} {F(Result.Position.Y)} {F(Result.Position.Z)} n {F(Result.Normal.X)} {F(Result.Normal.Y)} {F(Result.Normal.Z)} d {F(Result.Distance)}`
	end
	local function Try(Label, F2)
		local Ok, Result = pcall(F2)
		print(Label, if Ok then Show(Result) else "error " .. P.err(Result))
	end
	local Down = Vector3.new(0, -20, 0)
	-- down onto the wall, off centre on both axes, various sizes
	for _, Case in {
		{ 303, 302, Vector3.new(1, 1, 1) }, { 304.8, 304.6, Vector3.new(1, 1, 1) }, { 296, 303, Vector3.new(2, 1, 3) },
		{ 300, 300, Vector3.new(20, 1, 20) }, { 306, 300, Vector3.new(4, 1, 4) }, { 302, 301, Vector3.new(30, 2, 12) },
		{ 305.4, 300, Vector3.new(1, 1, 1) }, { 300, 300, Vector3.new(10, 1, 10) }, { 301, 300, Vector3.new(10, 1, 10) },
	} do
		Try(`down {Case[1]} {Case[2]} {Case[3]}`, function() return workspace:Blockcast(CFrame.new(Case[1], 100, Case[2]), Case[3], Down) end)
	end
	-- down onto the small block, bigger boxes
	for _, Case in { { 400, 300, Vector3.new(4, 1, 4) }, { 401.5, 299, Vector3.new(4, 1, 4) }, { 401, 300, Vector3.new(1, 1, 1) } } do
		Try(`small {Case[1]} {Case[2]} {Case[3]}`, function() return workspace:Blockcast(CFrame.new(Case[1], 100, Case[2]), Case[3], Down) end)
	end
	-- turned boxes onto the wall, off centre
	for _, Angle in { 15, 30, 45 } do
		Try(`turned box {Angle}`, function() return workspace:Blockcast(CFrame.new(302, 100, 301) * CFrame.Angles(0, math.rad(Angle), 0), Vector3.new(2, 1, 1), Down) end)
	end
	-- onto the turned wall
	Try("onto turned", function() return workspace:Blockcast(CFrame.new(502, 100, 301), Vector3.new(1, 1, 1), Down) end)
	Try("onto turned, box turned alike", function() return workspace:Blockcast(CFrame.new(502, 100, 301) * CFrame.Angles(0, math.rad(30), 0), Vector3.new(1, 1, 1), Down) end)
	-- side hits with offsets
	for _, Case in { { 90.3, 300 }, { 89.8, 302 }, { 90, 300 }, { 90.9, 300 }, { 89.6, 304.9 } } do
		Try(`side y {Case[1]} z {Case[2]}`, function() return workspace:Blockcast(CFrame.new(290, Case[1], Case[2]), Vector3.new(1, 1, 1), Vector3.new(10, 0, 0)) end)
	end
	Try("side tall", function() return workspace:Blockcast(CFrame.new(290, 90, 300), Vector3.new(1, 3, 1), Vector3.new(10, 0, 0)) end)
	-- negative sizes, more
	for _, Size in { Vector3.new(-1, 1, 1), Vector3.new(1, 1, -1), Vector3.new(-3, 1, 2), Vector3.new(1, -2, 1) } do
		Try(`neg {Size} at 302 301`, function() return workspace:Blockcast(CFrame.new(302, 100, 301), Size, Down) end)
		Try(`neg {Size} side`, function() return workspace:Blockcast(CFrame.new(290, 90.2, 301), Size, Vector3.new(10, 0, 0)) end)
	end
	-- spheres onto edges and corners of the wall
	for _, Case in {
		{ Vector3.new(290, 91, 300), Vector3.new(20, 0, 0) }, { Vector3.new(294.8, 100, 300), Down }, { Vector3.new(294.6, 100, 294.6), Down },
		{ Vector3.new(305.3, 100, 300), Down }, { Vector3.new(290, 90.8, 300), Vector3.new(20, 0, 0) }, { Vector3.new(290, 91.4, 290), Vector3.new(10, 0, 10) },
		{ Vector3.new(300, 100, 300), Down }, { Vector3.new(303, 100, 304), Down },
	} do
		Try(`sphere r1 {Case[1]} {Case[2]}`, function() return workspace:Spherecast(Case[1], 1, Case[2]) end)
		Try(`sphere r0.5 {Case[1]} {Case[2]}`, function() return workspace:Spherecast(Case[1], 0.5, Case[2]) end)
	end
	-- a ray ending in a part's face from inside a neighbour and a ray along an edge
	Try("ray along edge", function() return workspace:Raycast(Vector3.new(290, 90.5, 295), Vector3.new(20, 0, 0)) end)
	Try("ray at corner", function() return workspace:Raycast(Vector3.new(295, 100, 295), Down) end)
	Try("ray start on edge out", function() return workspace:Raycast(Vector3.new(295, 90.5, 300), Vector3.new(-5, 5, 0)) end)
	Root:Destroy()
end
