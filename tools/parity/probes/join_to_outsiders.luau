-- Workspace:JoinToOutsiders (world/joints.rs join_to_outsiders, world/faces.rs): its argument errors; which welds it makes
-- (class, name, parent, parts, C0, C1) for a listed part B against outside parts, by mode, surface, face, gap, overlap, angle and
-- shape; which joints and constraints between two parts stop it; which parts are listed and which are outside; the order of a
-- listed part's welds. A client goes through its listed parts in an order of its own (it differs between a client and its
-- server), so each listed part's welds are printed apart. Everything it makes lives under one Folder and is cleaned up as it goes.
local W = workspace
local Root = Instance.new("Folder")
Root.Name = "JoinProbe"
Root.Parent = W
local Storage = game:GetService("ReplicatedStorage")
local Base = CFrame.new(0, 100, 0)
local All = Enum.JointCreationMode.All
local function N(V)
	if V == 0 then
		return if 1 / V < 0 then "-0" else "0"
	end
	return string.format("%.9g", V)
end
local function CF(C)
	local Parts = {}
	for _, V in { C:GetComponents() } do
		table.insert(Parts, N(V))
	end
	return table.concat(Parts, ",")
end
local function Brick(Name, At, Size, Props, Class, Parent)
	local B = Instance.new(Class or "Part")
	B.Name = Name
	B.Anchored = true
	B.Size = Size or Vector3.new(4, 1, 4)
	for K, V in Props or {} do
		B[K] = V
	end
	B.CFrame = At
	B.Parent = Parent or Root
	return B
end
local function Welds(Part, Detail)
	local Lines = {}
	for _, Child in Part:GetChildren() do
		if Child:IsA("JointInstance") then
			local Line = Child.ClassName .. " '" .. Child.Name .. "' " .. (Child.Part0 and Child.Part0.Name or "nil") .. "-" .. (Child.Part1 and Child.Part1.Name or "nil")
			if Detail then
				Line ..= " C0=" .. CF(Child.C0) .. " C1=" .. CF(Child.C1)
			end
			table.insert(Lines, Line)
		end
	end
	return if #Lines == 0 then "none" else table.concat(Lines, "; ")
end
local function Try(F)
	local Ok, Message = pcall(F)
	return if Ok then "ok" else "error " .. P.err(Message)
end
local function Clear()
	Root:ClearAllChildren()
end
-- B (listed) on top of A unless placed: the call's outcome and B's welds
local function Case(Label, BAt, BSize, BProps, BClass, Mode, ASize, AProps, AClass, AAt)
	Brick("A", AAt or Base, ASize, AProps, AClass)
	local B = Brick("B", BAt or Base * CFrame.new(0, 1, 0), BSize, BProps, BClass)
	local Result = Try(function()
		W:JoinToOutsiders({ B }, Mode or All)
	end)
	print(Label .. " -> " .. Result .. " | " .. Welds(B, true))
	Clear()
end
-- whether listed B gets a weld to A ("W"), none ("-"), or how many
local function Joins(AAt, ASize, BAt, BSize, AClass, BClass)
	Brick("A", AAt, ASize, nil, AClass)
	local B = Brick("B", BAt, BSize, nil, BClass)
	W:JoinToOutsiders({ B }, All)
	local Got = #B:GetChildren()
	Clear()
	return if Got == 0 then "-" elseif Got == 1 then "W" else tostring(Got)
end
local function Row(Label, Keys, F)
	local Out = {}
	for _, K in Keys do
		table.insert(Out, string.format("%g", K) .. ":" .. F(K))
	end
	print(Label .. " -> " .. table.concat(Out, " "))
end
local function Names(Part)
	local Out = {}
	for _, Child in Part:GetChildren() do
		if Child:IsA("Weld") then
			table.insert(Out, Child.Part1.Name)
		end
	end
	return table.concat(Out, ",")
end

-- 1. arguments: the mode is cast first, then the list
for _, C in {
	{ "()", {}, 0 }, { "({})", { {} }, 1 }, { "({}, nil)", { {}, nil }, 2 }, { "({}, 'Bogus')", { {}, "Bogus" }, 2 },
	{ "(5, 'Bogus')", { 5, "Bogus" }, 2 }, { "(5, All)", { 5, All }, 2 }, { "(nil, All)", { nil, All }, 2 },
	{ "('x', All)", { "x", All }, 2 }, { "(workspace, All)", { W, All }, 2 }, { "({1}, All)", { { 1 }, All }, 2 },
	{ "({workspace, 1}, All)", { { W, 1 }, All }, 2 }, { "({1}, None)", { { 1 }, Enum.JointCreationMode.None }, 2 },
	{ "({x=1}, All)", { { x = 1 }, All }, 2 }, { "({}, 3)", { {}, 3 }, 2 }, { "({}, -1)", { {}, -1 }, 2 }, { "({}, -1.5)", { {}, -1.5 }, 2 },
	{ "({}, -0.5)", { {}, -0.5 }, 2 }, { "({}, 1.5)", { {}, 1.5 }, 2 }, { "({}, 2.9)", { {}, 2.9 }, 2 }, { "({}, 0/0)", { {}, 0 / 0 }, 2 },
	{ "({}, math.huge)", { {}, math.huge }, 2 }, { "({}, '1')", { {}, "1" }, 2 }, { "({}, 'all')", { {}, "all" }, 2 },
	{ "({}, 'None')", { {}, "None" }, 2 }, { "({}, true)", { {}, true }, 2 }, { "({}, Material)", { {}, Enum.Material.Plastic }, 2 },
	{ "({}, Enum.JointCreationMode)", { {}, Enum.JointCreationMode }, 2 }, { "({}, {})", { {}, {} }, 2 }, { "({}, workspace)", { {}, W }, 2 },
	{ "({}, Vector3)", { {}, Vector3.zero }, 2 }, { "({}, print)", { {}, print }, 2 }, { "({}, None, 5)", { {}, Enum.JointCreationMode.None, 5 }, 3 },
} do
	print("JoinToOutsiders" .. C[1] .. " -> " .. Try(function()
		W:JoinToOutsiders(table.unpack(C[2], 1, C[3]))
	end))
end
print("dot call -> " .. Try(function()
	W.JoinToOutsiders({}, All)
end))

-- 2. modes and surfaces
for _, M in { { "All", All }, { "Surface", Enum.JointCreationMode.Surface }, { "None", Enum.JointCreationMode.None }, { "0.5", 0.5 }, { "-0.5", -0.5 }, { "1.5", 1.5 }, { "2.9", 2.9 }, { "'Surface'", "Surface" } } do
	Case("mode " .. M[1], nil, nil, nil, nil, M[2])
end
for _, S in { "Smooth", "Glue", "Weld", "Studs", "Inlet", "Universal", "Hinge", "Motor" } do
	for _, M in { "All", "Surface" } do
		Brick("A", Base, nil, { TopSurface = Enum.SurfaceType[S] })
		local B = Brick("B", Base * CFrame.new(0, 1, 0), nil, { BottomSurface = Enum.SurfaceType[S] })
		W:JoinToOutsiders({ B }, Enum.JointCreationMode[M])
		print("surfaces " .. S .. " " .. M .. " -> " .. Welds(B))
		Clear()
	end
end

-- 3. each face of B against A, sizes that halve and that do not, B turned
for _, F in { { "+X", Vector3.new(4, 0, 0) }, { "-X", Vector3.new(-4, 0, 0) }, { "+Y", Vector3.new(0, 1, 0) }, { "-Y", Vector3.new(0, -1, 0) }, { "+Z", Vector3.new(0, 0, 4) }, { "-Z", Vector3.new(0, 0, -4) } } do
	Case("B beside A at " .. F[1], Base + F[2])
end
for _, F in { { "+X", Vector3.new(3, 0, 0) }, { "-Y", Vector3.new(0, -3, 0) }, { "+Z", Vector3.new(0, 0, 5) } } do
	Case("B 2x3x5 beside A 4x3x5 at " .. F[1], Base + F[2], Vector3.new(2, 3, 5), nil, nil, nil, Vector3.new(4, 3, 5))
end
Case("B 0.7 on A 4x0.3x4", Base * CFrame.new(0, 0.5, 0), Vector3.new(0.7, 0.7, 0.7), nil, nil, nil, Vector3.new(4, 0.3, 4))
for _, T in { { "x90", CFrame.Angles(math.rad(90), 0, 0) }, { "y90", CFrame.Angles(0, math.rad(90), 0) }, { "z90", CFrame.Angles(0, 0, math.rad(90)) }, { "x180", CFrame.Angles(math.rad(180), 0, 0) }, { "y45", CFrame.Angles(0, math.rad(45), 0) } } do
	Case("B 2x2x2 turned " .. T[1] .. " on A", Base * CFrame.new(0, 1.5, 0) * T[2], Vector3.new(2, 2, 2))
end

-- 4. how far apart: float32 world positions at several heights, and beside
for _, Y in { 0.5, 10, 50, 100 } do
	Row("gap y=" .. Y, { -0.011, -0.0101, -0.01, -0.0099, 0, 0.0099, 0.01, 0.0101, 0.011 }, function(D)
		return Joins(CFrame.new(0, Y, 0), nil, CFrame.new(0, Y + 1 + D, 0))
	end)
end
Row("gap beside +X", { -0.0101, -0.01, -0.0099, 0.0099, 0.01, 0.0101 }, function(D)
	return Joins(Base, nil, Base * CFrame.new(4 + D, 0, 0))
end)
Row("gap 40-stud faces", { 0.0098, 0.01 }, function(D)
	return Joins(Base, Vector3.new(40, 1, 40), Base * CFrame.new(0, 1 + D, 0), Vector3.new(40, 1, 40))
end)

-- 5. how much they overlap: B a cube of side s slid over A's +X edge (by overlap), by its centre, and the other way round
for _, S in { 0.05, 0.1, 0.12, 0.15, 0.2, 0.25, 1, 4 } do
	Row("overlap s=" .. S, { 0.05, 0.09, 0.095, 0.1, 0.105, 0.11, 0.12 }, function(O)
		if O > S then
			return "x"
		end
		return Joins(Base, nil, Base * CFrame.new(2 + S / 2 - O, 0.5 + S / 2, 0), Vector3.new(S, S, S))
	end)
end
for _, S in { 0.02, 0.05, 0.08, 0.2 } do
	Row("centre s=" .. S, { 1.89, 1.9, 1.91, 1.92, 1.93, 1.94, 1.95, 1.99, 2, 2.01 }, function(C)
		return Joins(Base, nil, Base * CFrame.new(C, 0.5 + S / 2, 0), Vector3.new(S, S, S))
	end)
	Row("centre of A s=" .. S, { 1.89, 1.9, 1.91, 1.92, 1.93, 1.94, 1.95, 1.99, 2, 2.01 }, function(C)
		return Joins(Base * CFrame.new(C, 0.5 - S / 2, 0), Vector3.new(S, S, S), Base * CFrame.new(0, 1, 0))
	end)
end
Row("corner s=1", { 0.095, 0.1, 0.105, 0.11 }, function(O)
	return Joins(Base, nil, Base * CFrame.new(2.5 - O, 1, 2.5 - O), Vector3.new(1, 1, 1))
end)
Row("thin B w x 1 x 2 on A", { 0.01, 0.05, 0.1, 0.2 }, function(Wd)
	return Joins(Base, nil, Base * CFrame.new(0, 1, 0), Vector3.new(Wd, 1, 2))
end)
Row("B 0.05 on A 0.05 offset", { 0, 0.01, 0.03 }, function(Off)
	return Joins(Base, Vector3.new(0.05, 0.05, 0.05), Base * CFrame.new(Off, 0.05, 0), Vector3.new(0.05, 0.05, 0.05))
end)
Row("B 0.2 on A 0.2 offset", { 0, 0.04, 0.08 }, function(Off)
	return Joins(Base, Vector3.new(0.2, 0.2, 0.2), Base * CFrame.new(Off, 0.2, 0), Vector3.new(0.2, 0.2, 0.2))
end)

-- 6. angles: a turn about the normal, a tilt, both; a tilt of a big or small face, over a part of the other
local Degrees = { 0.3, 0.5, 0.57, 0.572, 0.573, 0.58, 0.6 }
Row("yaw B 4", Degrees, function(D)
	return Joins(Base, nil, Base * CFrame.new(0, 1, 0) * CFrame.Angles(0, math.rad(D), 0))
end)
Row("yaw A under B 0.1", Degrees, function(D)
	return Joins(Base * CFrame.Angles(0, math.rad(D), 0), nil, Base * CFrame.new(0, 1, 0), Vector3.new(0.1, 1, 0.1))
end)
Row("tilt B 0.1 on A 40", Degrees, function(D)
	return Joins(Base, Vector3.new(40, 1, 40), Base * CFrame.new(0, 0.55, 0) * CFrame.Angles(math.rad(D), 0, 0), Vector3.new(0.1, 0.1, 0.1))
end)
Row("tilt and yaw B 0.1", Degrees, function(D)
	return Joins(Base, nil, Base * CFrame.new(0, 1, 0) * CFrame.Angles(math.rad(D) * 0.6, math.rad(D) * 0.8, 0), Vector3.new(0.1, 1, 0.1))
end)
Row("yaw coarse B 2", { 1, 30, 45, 89.4, 89.5, 90, 135, 180, 270 }, function(D)
	return Joins(Base, nil, Base * CFrame.new(0, 1, 0) * CFrame.Angles(0, math.rad(D), 0), Vector3.new(2, 1, 2))
end)
local Tilts = { 0.1, 0.14, 0.15, 0.2, 0.25, 0.3, 0.55, 0.6 }
Row("tilt B 4 on A 40", Tilts, function(D)
	return Joins(Base, Vector3.new(40, 1, 40), Base * CFrame.new(0, 1, 0) * CFrame.Angles(math.rad(D), 0, 0))
end)
Row("tilt B 40 on A 4", Tilts, function(D)
	return Joins(Base, nil, Base * CFrame.new(0, 1, 0) * CFrame.Angles(math.rad(D), 0, 0), Vector3.new(40, 1, 40))
end)
Row("tilt A 40 under B 4", Tilts, function(D)
	return Joins(Base * CFrame.Angles(math.rad(D), 0, 0), Vector3.new(40, 1, 40), Base * CFrame.new(0, 1, 0))
end)
Row("tilt B 4 about its -Z edge on A 40", Tilts, function(D)
	return Joins(Base, Vector3.new(40, 1, 40), Base * CFrame.new(0, 0.5, -2) * CFrame.Angles(math.rad(D), 0, 0) * CFrame.new(0, 0.5, 2))
end)
Row("tilt B 4x1x1 about x", Tilts, function(D)
	return Joins(Base, Vector3.new(40, 1, 40), Base * CFrame.new(0, 1, 0) * CFrame.Angles(math.rad(D), 0, 0), Vector3.new(4, 1, 1))
end)
Row("tilt B 4 raised 0.005", Tilts, function(D)
	return Joins(Base, Vector3.new(40, 1, 40), Base * CFrame.new(0, 1.005, 0) * CFrame.Angles(math.rad(D), 0, 0))
end)
-- B 4 spans z 1..5 over A's z -2..2 (an overlap of 1): tilted about its edge over A, then about its far edge
Row("tilt B over A's edge about the overlap's edge", Tilts, function(D)
	return Joins(Base, nil, Base * CFrame.new(0, 0.5, 1) * CFrame.Angles(math.rad(D), 0, 0) * CFrame.new(0, 0.5, 2))
end)
Row("tilt B over A's edge about its far edge", Tilts, function(D)
	return Joins(Base, nil, Base * CFrame.new(0, 0.5, 5) * CFrame.Angles(math.rad(-D), 0, 0) * CFrame.new(0, 0.5, -2))
end)
Row("tilt A about its far edge under B's edge", Tilts, function(D)
	return Joins(Base * CFrame.new(0, 0.5, -2) * CFrame.Angles(math.rad(D), 0, 0) * CFrame.new(0, -0.5, 2), nil, Base * CFrame.new(0, 1, 3))
end)
Row("gap of a turned pair", { 0.0099, 0.0101 }, function(D)
	local T = CFrame.new(0, 100, 0) * CFrame.Angles(0.3, 0.7, 0.2)
	return Joins(T, nil, T * CFrame.new(0, 1 + D, 0))
end)
-- turned pairs: the weld's frames
for Index, Turn in { CFrame.Angles(0.3, 0.7, 0.2), CFrame.Angles(1.1, -0.4, 2.9), CFrame.Angles(0, 0, 0.001), CFrame.Angles(-2.2, 0.35, 1.4) } do
	local A = Brick("A", CFrame.new(40, 80, 0) * Turn)
	local B = Brick("B", A.CFrame * CFrame.new(0.3, 1, -0.6), Vector3.new(2, 1, 2))
	W:JoinToOutsiders({ B }, All)
	print("turned pair " .. Index .. " -> " .. Welds(B, true))
	Clear()
end

-- 7. shapes and classes
local Up = Base * CFrame.new(0, 2.5, 0)
Case("B Ball", Up, Vector3.new(4, 4, 4), { Shape = Enum.PartType.Ball })
Case("B Cylinder end", Up * CFrame.Angles(0, 0, math.rad(90)), Vector3.new(4, 4, 4), { Shape = Enum.PartType.Cylinder })
Case("B Cylinder side", Up, Vector3.new(4, 4, 4), { Shape = Enum.PartType.Cylinder })
Case("B Cylinder end at A's corner", Base * CFrame.new(3.8, 2.5, 3.8) * CFrame.Angles(0, 0, math.rad(90)), Vector3.new(4, 4, 4), { Shape = Enum.PartType.Cylinder })
Case("B TrussPart", Up, Vector3.new(2, 4, 2), nil, "TrussPart")
Case("B MeshPart 2x6x3", Base * CFrame.new(0, 3.5, 0), Vector3.new(2, 6, 3), nil, "MeshPart")
Case("B Seat", nil, nil, nil, "Seat")
Case("B SpawnLocation", nil, nil, nil, "SpawnLocation")
Case("A Ball", Base * CFrame.new(0, 2.5, 0), nil, nil, nil, nil, Vector3.new(4, 4, 4), { Shape = Enum.PartType.Ball })
Case("A Cylinder side", Base * CFrame.new(0, 2.5, 0), nil, nil, nil, nil, Vector3.new(4, 4, 4), { Shape = Enum.PartType.Cylinder })
Case("A Cylinder end", Base * CFrame.new(0, 2.5, 0), nil, nil, nil, nil, Vector3.new(4, 4, 4), { Shape = Enum.PartType.Cylinder }, nil, Base * CFrame.Angles(0, 0, math.rad(90)))
Case("A TrussPart", Base * CFrame.new(0, 2.5, 0), Vector3.new(2, 1, 2), nil, nil, nil, Vector3.new(2, 4, 2), nil, "TrussPart")
-- a wedge or corner wedge's every face: B on top of A turned each of 32 ways (which face of A is up, from C1's position), and A
-- under a block B turned the same, by class and as a Part's Shape
local Turns = {}
for _, X in { 0, 90, 180, 270 } do
	for _, Y in { 0, 90, 180, 270 } do
		for _, Z in { 0, 90 } do
			table.insert(Turns, { X, Y, Z })
		end
	end
end
local function Up32(Label, AClass, AProps, BClass, BProps)
	local Out = {}
	for _, T in Turns do
		local Turn = CFrame.Angles(math.rad(T[1]), math.rad(T[2]), math.rad(T[3]))
		Brick("A", Base * Turn, Vector3.new(4, 4, 4), AProps, AClass)
		local B = Brick("B", Base * CFrame.new(0, 2.5, 0), Vector3.new(4, 1, 4), BProps, BClass)
		W:JoinToOutsiders({ B }, All)
		local X = B:FindFirstChildOfClass("Weld")
		table.insert(Out, if X then string.format("%d", math.round(X.C1.X)) .. "," .. string.format("%d", math.round(X.C1.Y)) .. "," .. string.format("%d", math.round(X.C1.Z)) else "-")
		Clear()
	end
	print("faces up of " .. Label .. " -> " .. table.concat(Out, " "))
end
Up32("WedgePart", "WedgePart")
Up32("Part Wedge", nil, { Shape = Enum.PartType.Wedge })
Up32("CornerWedgePart", "CornerWedgePart")
Up32("Part CornerWedge", nil, { Shape = Enum.PartType.CornerWedge })
local function Down32(Label, BClass, BProps)
	local Out = {}
	for _, T in Turns do
		local Turn = CFrame.Angles(math.rad(T[1]), math.rad(T[2]), math.rad(T[3]))
		Brick("A", Base, Vector3.new(40, 1, 40))
		local B = Brick("B", Base * CFrame.new(0, 2.5, 0) * Turn, Vector3.new(4, 4, 4), BProps, BClass)
		W:JoinToOutsiders({ B }, All)
		local X = B:FindFirstChildOfClass("Weld")
		table.insert(Out, if X then string.format("%d", math.round(X.C0.X)) .. "," .. string.format("%d", math.round(X.C0.Y)) .. "," .. string.format("%d", math.round(X.C0.Z)) else "-")
		Clear()
	end
	print("faces down of listed " .. Label .. " -> " .. table.concat(Out, " "))
end
Down32("WedgePart", "WedgePart")
Down32("CornerWedgePart", "CornerWedgePart")
-- a small B on each corner of a wedge's or corner wedge's triangle, turned up
for _, Class in { "WedgePart", "CornerWedgePart" } do
	for _, Up in { { "+X up", CFrame.Angles(0, 0, math.rad(90)) }, { "-Z up", CFrame.Angles(math.rad(90), 0, 0) } } do
		local Out = {}
		for _, At in { { -1.5, -1.5 }, { 1.5, 1.5 }, { -1.5, 1.5 }, { 1.5, -1.5 }, { 0, 0 } } do
			table.insert(Out, At[1] .. "," .. At[2] .. ":" .. Joins(Base * Up[2], Vector3.new(4, 4, 4), Base * CFrame.new(At[1], 2.25, At[2]), Vector3.new(0.5, 0.5, 0.5), Class))
		end
		print(Class .. " " .. Up[1] .. ", B 0.5 at x,z -> " .. table.concat(Out, " "))
	end
end

-- 8. properties that make no difference
for _, Props in { { "unanchored", { Anchored = false } }, { "Locked", { Locked = true } }, { "Massless", { Massless = true } }, { "CanCollide false", { CanCollide = false } }, { "CanQuery false", { CanQuery = false } }, { "CanTouch false", { CanTouch = false } }, { "Transparency 1", { Transparency = 1 } } } do
	W.Gravity = 0
	Brick("A", Base, nil, Props[2])
	local B = Brick("B", Base * CFrame.new(0, 1, 0), nil, Props[2])
	W:JoinToOutsiders({ B }, All)
	print("both " .. Props[1] .. " -> " .. Welds(B))
	Clear()
	W.Gravity = 196.2
end
do
	Brick("A", Base)
	local B = Brick("B", Base * CFrame.new(0, 1, 0))
	W:JoinToOutsiders({ B }, All)
	local X = B:FindFirstChildOfClass("Weld")
	print("the weld -> Enabled " .. tostring(X.Enabled) .. " Archivable " .. tostring(X.Archivable) .. " parent " .. X.Parent.Name)
	Clear()
end

-- 9. which parts are listed and which are outside
do
	local A = Brick("A", Base, Vector3.new(8, 1, 4))
	local B = Brick("B", Base * CFrame.new(-2, 1, 0))
	local C = Brick("C", Base * CFrame.new(2, 1, 0))
	W:JoinToOutsiders({ C, B }, All)
	print("two listed on A -> B: " .. Welds(B) .. " | C: " .. Welds(C) .. " | A: " .. Welds(A))
	Clear()
	Brick("A", Base)
	local Model = Instance.new("Model")
	Model.Parent = Root
	B = Brick("B", Base * CFrame.new(0, 1, 0), nil, nil, nil, Model)
	W:JoinToOutsiders({ Model }, All)
	print("a Model listed -> " .. Welds(B))
	Clear()
	Brick("A", Base)
	local Pa = Brick("P", Base * CFrame.new(0, 20, 0))
	B = Brick("B", Base * CFrame.new(0, 1, 0), nil, nil, nil, Pa)
	W:JoinToOutsiders({ Pa }, All)
	print("a listed part's child part -> " .. Welds(B))
	Clear()
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	Brick("A", Base, nil, nil, nil, B)
	W:JoinToOutsiders({ B }, All)
	print("the other part below the listed one -> " .. Welds(B))
	Clear()
	for _, Where in { "both", "A", "B" } do
		local World = Instance.new("WorldModel")
		World.Parent = Root
		Brick("A", Base, nil, nil, nil, if Where ~= "B" then World else nil)
		B = Brick("B", Base * CFrame.new(0, 1, 0), nil, nil, nil, if Where ~= "A" then World else nil)
		W:JoinToOutsiders({ B }, All)
		print("in a WorldModel: " .. Where .. " -> " .. Welds(B))
		Clear()
	end
	A = Brick("A", Base, nil, nil, nil, Storage)
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	W:JoinToOutsiders({ B }, All)
	print("A in ReplicatedStorage -> " .. Welds(B))
	A:Destroy()
	Clear()
	Brick("A", Base)
	B = Brick("B", Base * CFrame.new(0, 1, 0), nil, nil, nil, Storage)
	W:JoinToOutsiders({ B }, All)
	print("B in ReplicatedStorage -> " .. Welds(B))
	B:Destroy()
	Clear()
	Brick("A", Base)
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	W:JoinToOutsiders({ B, nil, B, Root }, All)
	print("listed again, and its Folder -> " .. Welds(B))
	Clear()
	A = Brick("A", Base)
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	W:JoinToOutsiders({ A }, All)
	print("A listed, B outside -> A: " .. Welds(A, true) .. " | B: " .. Welds(B))
	Clear()
	Brick("A", Base, Vector3.new(8, 1, 8))
	Brick("C", Base * CFrame.new(3, 1.5, 0), Vector3.new(2, 2, 8))
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	W:JoinToOutsiders({ B }, All)
	print("B on A and beside C -> " .. Welds(B, true))
	Clear()
	print("listing Terrain -> " .. Try(function()
		W:JoinToOutsiders({ W.Terrain }, All)
	end) .. " | " .. Welds(W.Terrain))
end

-- 10. what is between the two already: a joint or a constraint stops it, unless disabled or out of the Workspace
for _, J in {
	"Weld", "Weld swapped", "Weld disabled", "Weld out of the DataModel", "Weld in ReplicatedStorage", "Weld in JointsService",
	"Weld in a WorldModel", "Weld Part1 nil", "Weld A to A", "Weld through a third part", "joined twice", "WeldConstraint",
	"WeldConstraint disabled", "WeldConstraint through a third part", "Motor6D", "Motor6D disabled", "Motor", "Snap", "Glue",
	"Glue disabled", "ManualWeld", "ManualGlue", "Rotate", "Rotate disabled", "RotateP", "RotateV", "VelocityMotor",
	"HingeConstraint", "HingeConstraint disabled", "HingeConstraint in ReplicatedStorage", "HingeConstraint one attachment",
	"HingeConstraint attachment in a Folder", "BallSocketConstraint", "RopeConstraint", "RodConstraint", "SpringConstraint",
	"PrismaticConstraint", "CylindricalConstraint", "UniversalConstraint", "PlaneConstraint", "AlignPosition", "AlignOrientation",
	"TorsionSpringConstraint", "RigidConstraint", "NoCollisionConstraint", "Weld to a part in ReplicatedStorage",
} do
	local A = Brick("A", Base)
	local B = Brick("B", Base * CFrame.new(0, 1, 0))
	local Class = string.match(J, "^(%S+)")
	local Made
	local function Make(Name, P0, P1, Parent)
		local X = Instance.new(Name)
		if X:IsA("JointInstance") or X:IsA("WeldConstraint") or X:IsA("NoCollisionConstraint") then
			X.Part0, X.Part1 = P0, P1
		else
			local A0 = Instance.new("Attachment")
			A0.Parent = P0
			local A1 = Instance.new("Attachment")
			A1.Parent = P1
			X.Attachment0, X.Attachment1 = A0, A1
		end
		X.Parent = Parent or P0
		return X
	end
	if J == "Weld through a third part" or J == "WeldConstraint through a third part" then
		local Far = Brick("Far", Base * CFrame.new(0, 20, 0), Vector3.new(1, 1, 1))
		Make(Class, A, Far)
		Make(Class, Far, B)
	elseif J == "joined twice" then
		W:JoinToOutsiders({ B }, All)
	elseif J == "Weld swapped" then
		Make("Weld", B, A, Root)
	elseif J == "Weld out of the DataModel" then
		Make("Weld", A, B).Parent = nil
	elseif J == "Weld in ReplicatedStorage" or J == "HingeConstraint in ReplicatedStorage" then
		Made = Make(Class, A, B, Storage)
	elseif J == "Weld in JointsService" then
		Made = Make("Weld", A, B, game:GetService("JointsService"))
	elseif J == "Weld in a WorldModel" then
		local M = Instance.new("WorldModel")
		M.Parent = Root
		Make("Weld", A, B, M)
	elseif J == "Weld Part1 nil" then
		Make("Weld", A, nil)
	elseif J == "Weld A to A" then
		Make("Weld", A, A)
	elseif J == "HingeConstraint one attachment" then
		Make("HingeConstraint", A, B).Attachment1 = nil
	elseif J == "HingeConstraint attachment in a Folder" then
		local X = Make("HingeConstraint", A, B)
		local F = Instance.new("Folder")
		F.Parent = A
		X.Attachment0.Parent = F
	elseif J == "Weld to a part in ReplicatedStorage" then
		local Far = Brick("Far", Base * CFrame.new(0, 20, 0), Vector3.new(1, 1, 1), nil, nil, Storage)
		Make("Weld", B, Far)
		Made = Far
	elseif string.find(J, " disabled$") then
		Make(Class, A, B).Enabled = false
	else
		Make(Class, A, B)
	end
	W:JoinToOutsiders({ B }, All)
	print("between them " .. J .. " -> " .. Welds(B))
	if Made then
		Made:Destroy()
	end
	Clear()
end

-- 11. the order of a listed part's welds: the order the other parts came into the Workspace
local At = { A = Base, C = Base * CFrame.new(0, 2, 0), D = Base * CFrame.new(4, 1, 0), E = Base * CFrame.new(-4, 1, 0), F = Base * CFrame.new(0, 1, 4) }
for _, Order in { { "A", "C", "D" }, { "D", "C", "A" }, { "F", "E", "D", "C", "A" } } do
	local B = Brick("B", Base * CFrame.new(0, 1, 0))
	for _, Name in Order do
		Brick(Name, At[Name])
	end
	W:JoinToOutsiders({ B }, All)
	print("made " .. table.concat(Order, ",") .. " -> " .. Names(B))
	Clear()
end
do
	local B = Brick("B", Base * CFrame.new(0, 1, 0))
	local Made = {}
	for _, Name in { "A", "C", "D" } do
		Made[Name] = Brick(Name, At[Name], nil, nil, nil, Storage)
	end
	for _, Name in { "D", "A", "C" } do
		Made[Name].Parent = Root
	end
	W:JoinToOutsiders({ B }, All)
	print("made A,C,D, parented D,A,C -> " .. Names(B))
	Clear()
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	for _, Name in { "A", "C", "D" } do
		Made[Name] = Brick(Name, At[Name])
	end
	Made.A.Parent = nil
	Made.A.Parent = Root
	W:JoinToOutsiders({ B }, All)
	print("made A,C,D, A out and back -> " .. Names(B))
	Clear()
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	for _, Name in { "A", "C", "D" } do
		Made[Name] = Brick(Name, At[Name] + Vector3.new(0, 500, 0))
	end
	task.wait()
	for _, Name in { "D", "A", "C" } do
		Made[Name].CFrame = At[Name]
	end
	Made.A.Size = Vector3.new(4, 1, 4.5)
	Made.A.Size = Vector3.new(4, 1, 4)
	W:JoinToOutsiders({ B }, All)
	print("made A,C,D far, a frame, moved in D,A,C -> " .. Names(B))
	Clear()
	local F1 = Instance.new("Folder")
	F1.Parent = Root
	local F2 = Instance.new("Folder")
	F2.Parent = Root
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	Made.A = Brick("A", At.A, nil, nil, nil, F1)
	Brick("C", At.C, nil, nil, nil, F1)
	Brick("D", At.D, nil, nil, nil, F2)
	Made.A.Parent = F2
	W:JoinToOutsiders({ B }, All)
	print("made A,C,D, A to another Folder -> " .. Names(B))
	Clear()
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	Made.A = Brick("A", At.A)
	Brick("C", At.C)
	Brick("D", At.D)
	local M = Instance.new("WorldModel")
	M.Parent = Root
	Made.A.Parent = M
	Made.A.Parent = Root
	W:JoinToOutsiders({ B }, All)
	print("made A,C,D, A into a WorldModel and back -> " .. Names(B))
	Clear()
	-- a subtree at once: the moved part first, then each part after its own descendants
	local Fo = Instance.new("Folder")
	local D = Brick("D", At.D, nil, nil, nil, Fo)
	Brick("A", At.A, nil, nil, nil, D)
	local Sub = Instance.new("Folder")
	Sub.Parent = Fo
	Brick("E", At.E, nil, nil, nil, Sub)
	Brick("C", At.C, nil, nil, nil, Fo)
	B = Brick("B", Base * CFrame.new(0, 1, 0))
	Fo.Parent = Root
	W:JoinToOutsiders({ B }, All)
	print("Folder{D{A},Sub{E},C} at once -> " .. Names(B))
	Clear()
	for _, Pair in { { "A", "C" }, { "C", "A" } } do
		local Pp = Brick("P", At[Pair[1]] + Vector3.new(0, 300, 0), nil, nil, nil, Storage)
		Pp.CFrame = At[Pair[1]]
		Brick("Q", At[Pair[2]], nil, nil, nil, Pp)
		B = Brick("B", Base * CFrame.new(0, 1, 0))
		Pp.Parent = Root
		W:JoinToOutsiders({ B }, All)
		print("P at " .. Pair[1] .. "'s place with child Q at " .. Pair[2] .. "'s, at once -> " .. Names(B))
		Clear()
	end
	-- 25 parts on a big listed part, made in a scrambled order, the third taken out and back
	B = Brick("B", Base * CFrame.new(0, 1, 0), Vector3.new(40, 1, 40))
	local Order = {}
	for I = 1, 25 do
		Order[I] = (I * 7) % 25 + 1
	end
	local Ring = {}
	for _, I in Order do
		local X, Z = (I - 1) % 5, (I - 1) // 5
		Ring[I] = Brick("R" .. I, Base * CFrame.new(-16 + 8 * X, 2, -16 + 8 * Z), Vector3.new(2, 1, 2))
	end
	Ring[Order[3]].Parent = nil
	Ring[Order[3]].Parent = Root
	W:JoinToOutsiders({ B }, All)
	print("25 made " .. table.concat(Order, ",") .. ", the third out and back -> " .. Names(B))
	Clear()
end
Root:Destroy()
