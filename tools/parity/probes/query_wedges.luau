-- Wedges, corner wedges and trusses in queries (world/query.rs, world/convex.rs): Raycast, Spherecast, Blockcast and Shapecast
-- against each (hit position, normal and distance), GetPartsInPart, ArePartsTouchingOthers and FindPartOnRay meeting their own
-- shapes, GetPartBoundsInBox, GetPartBoundsInRadius and FindPartsInRegion3 their boxes; a TrussPart's Size; a wedge's and a corner
-- wedge's mass, centre of mass and inertia. Numbers to three decimals (a client's contacts carry iteration errors of 2e-5).
do
	P.sync()
	workspace.Gravity = 0
	local Root = Instance.new("Folder")
	Root.Parent = workspace
	local function F(Value)
		return string.format("%.3f", Value)
	end
	local function V(A)
		return `{F(A.X)} {F(A.Y)} {F(A.Z)}`
	end
	local function Make(Name, Class, Position, Size, Shape, Turn)
		local Made = Instance.new(Class)
		Made.Name = Name
		Made.Anchored = true
		if Shape then
			Made.Shape = Shape
		end
		Made.Size = Size
		Made.CFrame = CFrame.new(Position) * (Turn or CFrame.identity)
		Made.Parent = Root
		return Made
	end
	local function Show(Result)
		if not Result then
			return "nil"
		end
		return `{Result.Instance.Name} pos {V(Result.Position)} n {V(Result.Normal)} d {F(Result.Distance)}`
	end
	local function Names(List)
		local Out = {}
		for _, Each in List do
			table.insert(Out, Each.Name)
		end
		table.sort(Out)
		return "{" .. table.concat(Out, ",") .. "}"
	end

	-- a TrussPart's Size
	local T = Instance.new("TrussPart")
	print("truss new", V(T.Size))
	for _, Size in { Vector3.new(4, 6, 8), Vector3.new(2, 4, 6), Vector3.new(4, 4, 2), Vector3.new(2, 4, 4), Vector3.new(6, 6, 6), Vector3.new(3, 3, 1),
		Vector3.new(0.5, 0.5, 0.5), Vector3.new(2, 2, 2.5), Vector3.new(1, 9, 9), Vector3.new(7, 5, 7), Vector3.new(2, 2, 3000), Vector3.new(-4, 2, 2),
		Vector3.new(0, 0, 0), Vector3.new(1.9, 2, 2), Vector3.new(1.5, 3, 2), Vector3.new(1.5, 1.8, 2), Vector3.new(2, 1.5, 2), Vector3.new(600, 3, 2) } do
		T.Size = Size
		print("truss", V(Size), V(T.Size))
	end
	T:Destroy()

	-- rays
	local Parts = {
		Make("W", "WedgePart", Vector3.new(300, 100, 300), Vector3.new(4, 6, 8)),
		Make("C", "CornerWedgePart", Vector3.new(320, 100, 300), Vector3.new(4, 6, 8)),
		Make("T", "TrussPart", Vector3.new(340, 100, 300), Vector3.new(2, 2, 8)),
		Make("WS", "Part", Vector3.new(360, 100, 300), Vector3.new(4, 6, 8), Enum.PartType.Wedge),
		Make("CS", "Part", Vector3.new(380, 100, 300), Vector3.new(4, 6, 8), Enum.PartType.CornerWedge),
		Make("WT", "WedgePart", Vector3.new(400, 100, 300), Vector3.new(4, 6, 8), nil, CFrame.Angles(0.3, 0.5, 0.7)),
		Make("CT", "CornerWedgePart", Vector3.new(420, 100, 300), Vector3.new(4, 6, 8), nil, CFrame.Angles(0.3, 0.5, 0.7)),
		Make("T3", "TrussPart", Vector3.new(460, 100, 300), Vector3.new(10, 2, 2)),
	}
	for _, Part in Parts do
		local C = Part.Position
		local H = Part.Size / 2
		local function Ray(Label, From, Direction)
			print("ray", Part.Name, Label, Show(workspace:Raycast(C + From, Direction)))
		end
		for _, X in { -0.75, 0, 0.75 } do
			for _, Z in { -0.875, 0, 0.5 } do
				Ray(`down {X} {Z}`, Vector3.new(X * H.X, H.Y + 10, Z * H.Z), Vector3.new(0, -40, 0))
			end
		end
		for _, Y in { -0.8, 0, 0.3, 0.8 } do
			Ray(`front {Y}`, Vector3.new(0.75 * H.X, Y * H.Y, -H.Z - 10), Vector3.new(0, 0, 40))
			Ray(`back {Y}`, Vector3.new(0.75 * H.X, Y * H.Y, H.Z + 10), Vector3.new(0, 0, -40))
		end
		for _, Y in { -0.7, 0.7 } do
			for _, Z in { -0.75, 0.75 } do
				Ray(`left {Y} {Z}`, Vector3.new(-H.X - 10, Y * H.Y, Z * H.Z), Vector3.new(40, 0, 0))
				Ray(`right {Y} {Z}`, Vector3.new(H.X + 10, Y * H.Y, Z * H.Z), Vector3.new(-40, 0, 0))
			end
		end
		Ray("up", Vector3.new(0.3 * H.X, -H.Y - 10, 0.4 * H.Z), Vector3.new(0, 40, 0))
		for _, Off in { -2, 0, 2 } do
			Ray(`diag {Off}`, Vector3.new(0.2, 10 + Off, -10), Vector3.new(0, -30, 30))
		end
	end
	-- on edges and in faces' planes
	local W, C = Parts[1], Parts[2]
	for _, Case in { { "in-box down", W, Vector3.new(0, 2.5, -3), Vector3.new(0, -10, 0) }, { "in-wedge", W, Vector3.new(0, -2, 2), Vector3.new(0, -10, 0) },
		{ "slope side edge", W, Vector3.new(2, 10, 0), Vector3.new(0, -20, 0) }, { "slope top edge", W, Vector3.new(0, 10, 4), Vector3.new(0, -20, 0) },
		{ "front bottom edge", W, Vector3.new(0, -10, -4), Vector3.new(0, 20, 0) }, { "along slope", W, Vector3.new(-12, 0, 0), Vector3.new(40, 0, 0) },
		{ "ridge", C, Vector3.new(1, 10, -2), Vector3.new(0, -20, 0) }, { "right face plane", C, Vector3.new(2, 10, -2), Vector3.new(0, -20, 0) },
		{ "front slope edge", C, Vector3.new(1, 1.5, -10), Vector3.new(0, 0, 20) }, { "along z slope", C, Vector3.new(-12, 0, 0), Vector3.new(40, 0, 0) },
		{ "apex down", C, Vector3.new(2, 10, -4), Vector3.new(0, -20, 0) }, { "end at apex", C, Vector3.new(-8, 13, -12), Vector3.new(10, -10, 8) } } do
		print("edge", Case[1], Show(workspace:Raycast(Case[2].Position + Case[3], Case[4])))
	end
	-- a truss entered through an edge, and touched along one
	local Truss = Parts[3]
	for _, Case in { { "top front", Vector3.new(0.2, 10, -11), Vector3.new(0, -30, 30) }, { "bottom front", Vector3.new(0.2, 8, -11), Vector3.new(0, -30, 30) },
		{ "side", Vector3.new(11, 8, -14), Vector3.new(-25, -20, 25) } } do
		print("truss edge", Case[1], Show(workspace:Raycast(Truss.Position + Case[2], Case[3])))
	end
	-- a block's face at a float position: how a client reads the distance
	local B = Make("B", "Part", Vector3.new(300, 140, 300), Vector3.new(6.4, 2, 2))
	print("block distance", Show(workspace:Raycast(Vector3.new(286, 140, 300), Vector3.new(40, 0, 0))))
	B:Destroy()

	-- shape casts
	local Ball = Make("qball", "Part", Vector3.new(0, 500, 0), Vector3.new(1, 1, 1), Enum.PartType.Ball)
	local Block = Make("qblock", "Part", Vector3.new(0, 520, 0), Vector3.new(1, 1, 1))
	local Moving = Make("qwedge", "WedgePart", Vector3.new(0, 540, 0), Vector3.new(1, 2, 2))
	local Params = RaycastParams.new()
	Params.FilterType = Enum.RaycastFilterType.Exclude
	Params.FilterDescendantsInstances = { Ball, Block, Moving }
	for _, Part in { W, C, Truss, Parts[6] } do
		local Center = Part.Position
		local H = Part.Size / 2
		local function Cast(Kind, Label, From, Direction)
			local Result
			if Kind == "sphere" then
				Result = workspace:Spherecast(Center + From, 1, Direction, Params)
			elseif Kind == "block" then
				Result = workspace:Blockcast(CFrame.new(Center + From), Vector3.one, Direction, Params)
			elseif Kind == "shapeball" then
				Result = workspace:Shapecast(Ball, Direction, Params, CFrame.new(Center + From), Vector3.one)
			else
				Result = workspace:Shapecast(Moving, Direction, Params, CFrame.new(Center + From))
			end
			print(Kind, Part.Name, Label, Show(Result))
		end
		for _, Kind in { "sphere", "block", "shapeball", "shapewedge" } do
			for _, Z in { -0.75, 0.25, 0.75 } do
				Cast(Kind, `down {Z}`, Vector3.new(0.1, H.Y + 6, Z * H.Z), Vector3.new(0, -20, 0))
			end
			for _, Y in { -0.5, 0, 0.5 } do
				Cast(Kind, `front {Y}`, Vector3.new(0.1, Y * H.Y, -H.Z - 6), Vector3.new(0, 0, 20))
				Cast(Kind, `side {Y}`, Vector3.new(-H.X - 6, Y * H.Y, -0.3 * H.Z), Vector3.new(20, 0, 0))
			end
			Cast(Kind, "diag", Vector3.new(0.2, 8, -8), Vector3.new(0, -20, 20))
		end
	end

	-- overlap queries at points above a slope (in the box, off the shape) and in the shape
	local Overlap = OverlapParams.new()
	Overlap.FilterType = Enum.RaycastFilterType.Exclude
	Overlap.FilterDescendantsInstances = { Ball, Block, Moving }
	for _, Part in { W, C, Truss, Parts[6] } do
		local Center = Part.Position
		local H = Part.Size / 2
		for _, Z in { -0.8, 0, 0.8 } do
			for _, Y in { 0.6, 0.9 } do
				local At = Center + Part.CFrame:VectorToWorldSpace(Vector3.new(0.5 * H.X, Y * H.Y, Z * H.Z))
				Block.Size = Vector3.one * 0.2
				Block.CFrame = CFrame.new(At)
				Ball.Size = Vector3.one * 0.2
				Ball.CFrame = CFrame.new(At)
				print("overlap", Part.Name, Z, Y, Names(workspace:GetPartBoundsInBox(CFrame.new(At), Vector3.one * 0.2, Overlap)), Names(workspace:GetPartBoundsInRadius(At, 0.1, Overlap)),
					Names(workspace:GetPartsInPart(Block, Overlap)), Names(workspace:GetPartsInPart(Ball, Overlap)))
			end
		end
		-- the shape as GetPartsInPart's part
		local Probe = Make("probe", "Part", Center, Vector3.one * 0.2)
		local Seen = {}
		local Only = OverlapParams.new()
		Only.FilterType = Enum.RaycastFilterType.Include
		Only.FilterDescendantsInstances = { Probe }
		for _, Z in { -0.8, -0.4, 0, 0.4, 0.8 } do
			for _, Y in { 0.6, 0.9 } do
				for _, X in { -0.8, 0.8 } do
					Probe.CFrame = CFrame.new(Center + Part.CFrame:VectorToWorldSpace(Vector3.new(X * H.X, Y * H.Y, Z * H.Z)))
					table.insert(Seen, #workspace:GetPartsInPart(Part, Only))
				end
			end
		end
		print("as-query", Part.Name, table.concat(Seen, ""))
		Probe:Destroy()
	end

	-- a turned wedge's Region3: its box's world box, not its corners'
	local WT = Parts[6]
	local H = WT.Size / 2
	local Lo, Hi = WT.Position, WT.Position
	for _, Sx in { -1, 1 } do
		for _, Sy in { -1, 1 } do
			for _, Sz in { -1, 1 } do
				local At = WT.CFrame * Vector3.new(Sx * H.X, Sy * H.Y, Sz * H.Z)
				Lo, Hi = Lo:Min(At), Hi:Max(At)
			end
		end
	end
	for _, Gap in { 2, 0.05, -0.05 } do
		-- slabs below the box's world box along x, and above it along y (the wedge's corners end 2 studs below its box's top)
		local Below = Region3.new(Lo - Vector3.new(10, 10, 10), Vector3.new(Lo.X - Gap, Hi.Y + 10, Hi.Z + 10))
		local Above = Region3.new(Vector3.new(Lo.X - 10, Hi.Y + Gap, Lo.Z - 10), Hi + Vector3.new(10, 10, 10))
		print("region", Gap, #workspace:FindPartsInRegion3(Below, nil, 10), #workspace:FindPartsInRegion3(Above, nil, 10))
	end

	-- touching: a block above the slope, in the box but off the wedge; on the slope; a ball by a corner wedge's slope
	local W2 = Make("W2", "WedgePart", Vector3.new(500, 100, 300), Vector3.new(4, 6, 8))
	local C2 = Make("C2", "CornerWedgePart", Vector3.new(520, 100, 300), Vector3.new(4, 6, 8))
	for _, Case in { { "off", "Part", Vector3.new(500, 102.5, 297) }, { "on", "Part", Vector3.new(500, 99, 300) }, { "edge", "Part", Vector3.new(500, 100.2, 300) },
		{ "ball off", "Ball", Vector3.new(500, 102.5, 297) }, { "ball on", "Ball", Vector3.new(500, 100.1, 300) }, { "corner off", "Part", Vector3.new(518.5, 101.5, 298) },
		{ "corner on", "Part", Vector3.new(521, 99, 299) } } do
		local Piece = Make("piece", "Part", Case[3], Vector3.new(0.4, 0.4, 0.4), Case[2] == "Ball" and Enum.PartType.Ball or nil)
		Piece.CanCollide = true
		print("touching", Case[1], workspace:ArePartsTouchingOthers({ Piece }))
		Piece:Destroy()
	end
	-- FindPartOnRay through a wedge's box above its slope
	for _, Case in { { Vector3.new(500, 110, 297), Vector3.new(0, -20, 0) }, { Vector3.new(490, 102.5, 302), Vector3.new(20, 0, 0) } } do
		local Hit, At, Normal = workspace:FindPartOnRay(Ray.new(Case[1], Case[2]))
		print("findpart", Hit and Hit.Name or "nil", V(At), V(Normal))
	end

	-- mass, centre of mass and inertia
	local Spot = 0
	for _, Size in { Vector3.new(2, 4, 6), Vector3.new(6, 2, 2) } do
		for _, Kind in { { "WedgePart" }, { "CornerWedgePart" }, { "Part", Enum.PartType.Wedge }, { "Part", Enum.PartType.CornerWedge } } do
			for _, Turn in { CFrame.identity, CFrame.Angles(0.3, 0.5, 0.7) } do
				Spot += 1
				local A = Make("mass", Kind[1], Vector3.new(300 + 20 * Spot, 300, 300), Size, Kind[2], Turn)
				A.Anchored = false
				A.CanCollide = false
				local Out = { "mass", Kind[1], tostring(Kind[2]), V(Size), F(A.Mass), V(A.AssemblyCenterOfMass - A.Position) }
				for _, Axis in { Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1) } do
					table.insert(Out, V(A:AngularAccelerationToTorque(Axis)))
				end
				print(table.concat(Out, " "))
				A:Destroy()
			end
		end
	end
	Root:Destroy()
	workspace.Gravity = 196.2
end
