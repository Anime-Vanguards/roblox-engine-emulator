-- Spatial queries whose geometry is not finite, and the limits the shape casts check (world/query.rs): what each query
-- answers or raises for NaN and infinite origins, directions, sizes, radii and frames, for shape casts past their limits,
-- and for GetPartsObscuringTarget's points. The scene is 300 studs out: a client's Workspace holds the player's character
-- near the origin.
P.sync()
local Near = Instance.new("Folder")
Near.Parent = workspace
local function Part(Name, Position, Size)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = true
	Made.Size = Size
	Made.Position = Position
	Made.Parent = Near
	return Made
end
-- Wall: x 295..305, y 89.5..90.5, z 295..305; Side: x 329.5..330.5, y 95..105; High: y 129.5..130.5
Part("Wall", Vector3.new(300, 90, 300), Vector3.new(10, 1, 10))
Part("Side", Vector3.new(330, 100, 300), Vector3.new(1, 10, 10))
Part("High", Vector3.new(300, 130, 300), Vector3.new(10, 1, 10))
local Caster = Part("Caster", Vector3.new(300, 110, 300), Vector3.new(1, 1, 1))
local Big = Part("Big", Vector3.new(300, 400, 300), Vector3.new(600, 1, 2))
Big.CanCollide = false
Big.CanQuery = false
local NaN, Inf = 0 / 0, math.huge
local V = Vector3.new(300, 100, 300)
local Down = Vector3.new(0, -20, 0)
local function F(Value)
	return if Value == Value and math.abs(Value) ~= math.huge then string.format("%.2f", Value) else P.num(Value)
end
local function Describe(Value)
	if typeof(Value) == "RaycastResult" then
		return `{Value.Instance.Name} {F(Value.Position.X)} {F(Value.Position.Y)} {F(Value.Position.Z)} {F(Value.Distance)}`
	elseif type(Value) == "table" then
		local Names = {}
		for _, Item in Value do
			if typeof(Item) == "Instance" and Item:IsDescendantOf(Near) then
				table.insert(Names, Item.Name)
			end
		end
		table.sort(Names)
		return "{" .. table.concat(Names, ",") .. "}"
	elseif typeof(Value) == "Instance" then
		return Value.Name
	elseif typeof(Value) == "Vector3" then
		return `({F(Value.X)}, {F(Value.Y)}, {F(Value.Z)})`
	end
	return tostring(Value)
end
local function Try(Name, Call)
	local Results = table.pack(pcall(Call))
	if not Results[1] then
		print(Name, "error", P.err(Results[2]))
		return
	end
	local Parts = {}
	for Index = 2, Results.n do
		table.insert(Parts, Describe(Results[Index]))
	end
	print(Name, "ok", table.concat(Parts, " "))
end
Try("ray", function() return workspace:Raycast(V, Down) end)
for _, Pair in { { "nan", NaN }, { "inf", Inf }, { "-inf", -Inf } } do
	local N, X = Pair[1], Pair[2]
	Try(`Raycast origin.x {N}`, function() return workspace:Raycast(Vector3.new(X, 100, 300), Down) end)
	Try(`Raycast origin.y {N}`, function() return workspace:Raycast(Vector3.new(300, X, 300), Down) end)
	Try(`Raycast direction.x {N}`, function() return workspace:Raycast(V, Vector3.new(X, -20, 0)) end)
	Try(`Raycast direction.y {N}`, function() return workspace:Raycast(V, Vector3.new(0, X, 0)) end)
	Try(`FindPartOnRay direction {N}`, function() return workspace:FindPartOnRay(Ray.new(V, Vector3.new(0, X, 0))) end)
	Try(`FindPartOnRay origin {N}`, function() return workspace:FindPartOnRay(Ray.new(Vector3.new(X, 100, 300), Down)) end)
	Try(`FindPartOnRayWithIgnoreList direction {N}`, function() return workspace:FindPartOnRayWithIgnoreList(Ray.new(V, Vector3.new(0, X, 0)), {}) end)
	Try(`Spherecast position {N}`, function() return workspace:Spherecast(Vector3.new(X, 100, 300), 1, Down) end)
	Try(`Spherecast radius {N}`, function() return workspace:Spherecast(V, X, Down) end)
	Try(`Spherecast direction {N}`, function() return workspace:Spherecast(V, 1, Vector3.new(0, X, 0)) end)
	Try(`Blockcast position {N}`, function() return workspace:Blockcast(CFrame.new(X, 100, 300), Vector3.one, Down) end)
	Try(`Blockcast rotation {N}`, function() return workspace:Blockcast(CFrame.new(300, 100, 300, X, 0, 0, 0, 1, 0, 0, 0, 1), Vector3.one, Down) end)
	Try(`Blockcast size {N}`, function() return workspace:Blockcast(CFrame.new(V), Vector3.new(X, 1, 1), Down) end)
	Try(`Blockcast direction {N}`, function() return workspace:Blockcast(CFrame.new(V), Vector3.one, Vector3.new(0, X, 0)) end)
	Try(`Shapecast direction {N}`, function() return workspace:Shapecast(Caster, Vector3.new(0, X, 0)) end)
	Try(`Shapecast cframe {N}`, function() return workspace:Shapecast(Caster, Down, nil, CFrame.new(X, 100, 300)) end)
	Try(`Shapecast size {N}`, function() return workspace:Shapecast(Caster, Down, nil, nil, Vector3.new(X, 1, 1)) end)
	Try(`GetPartBoundsInBox position {N}`, function() return workspace:GetPartBoundsInBox(CFrame.new(X, 90, 300), Vector3.one) end)
	Try(`GetPartBoundsInBox rotation {N}`, function() return workspace:GetPartBoundsInBox(CFrame.new(300, 90, 300, X, 0, 0, 0, 1, 0, 0, 0, 1), Vector3.one) end)
	Try(`GetPartBoundsInBox size.x on Wall {N}`, function() return workspace:GetPartBoundsInBox(CFrame.new(300, 90, 300), Vector3.new(X, 1, 1)) end)
	Try(`GetPartBoundsInBox size.x off Wall {N}`, function() return workspace:GetPartBoundsInBox(CFrame.new(320, 90, 300), Vector3.new(X, 1, 1)) end)
	Try(`GetPartBoundsInBox size.y {N}`, function() return workspace:GetPartBoundsInBox(CFrame.new(300, 110, 300), Vector3.new(1, X, 1)) end)
	Try(`GetPartBoundsInBox size {N}`, function() return workspace:GetPartBoundsInBox(CFrame.new(300, 90, 300), Vector3.new(X, X, X)) end)
	Try(`GetPartBoundsInBox turned size.x {N}`, function() return workspace:GetPartBoundsInBox(CFrame.new(320, 90, 300) * CFrame.Angles(0, math.rad(30), 0), Vector3.new(X, 1, 1)) end)
	Try(`GetPartBoundsInRadius radius {N}`, function() return workspace:GetPartBoundsInRadius(V, X) end)
	Try(`GetPartBoundsInRadius position {N}`, function() return workspace:GetPartBoundsInRadius(Vector3.new(X, 90, 300), 5) end)
	local Cam = Instance.new("Camera")
	Cam.CFrame = CFrame.new(300, 100, 300)
	Cam.Parent = workspace
	Try(`GetPartsObscuringTarget point.y {N}`, function() return Cam:GetPartsObscuringTarget({ Vector3.new(300, X, 300) }, {}) end)
	Try(`GetPartsObscuringTarget point.x {N}`, function() return Cam:GetPartsObscuringTarget({ Vector3.new(X, 80, 300) }, {}) end)
	Try(`GetPartsObscuringTarget {N} then Wall`, function() return Cam:GetPartsObscuringTarget({ Vector3.new(300, X, 300), Vector3.new(300, 80, 300) }, {}) end)
	Try(`GetPartsObscuringTarget Wall then {N}`, function() return Cam:GetPartsObscuringTarget({ Vector3.new(300, 80, 300), Vector3.new(X, 80, 300) }, {}) end)
	Cam:Destroy()
end
-- a ray's length: none (a direction whose length overflows a float finds nothing); the shape casts' 1,024 studs
for _, Length in { 16000, 1e6, 1e20 } do
	Try(`Raycast length {Length}`, function() return workspace:Raycast(V, Vector3.new(0, -Length, 0)) end)
end
for _, Length in { 1024, 1025, 15001 } do
	Try(`Spherecast distance {Length}`, function() return workspace:Spherecast(V, 0.5, Vector3.new(0, -Length, 0)) end)
	Try(`Blockcast distance {Length}`, function() return workspace:Blockcast(CFrame.new(V), Vector3.one, Vector3.new(0, -Length, 0)) end)
	Try(`Shapecast distance {Length}`, function() return workspace:Shapecast(Caster, Vector3.new(0, -Length, 0)) end)
end
Try("Spherecast distance nan", function() return workspace:Spherecast(V, 1, Vector3.new(NaN, -2000, 0)) end)
for _, Radius in { 0, 256, 256.001, 257, -1, -0.5 } do
	Try(`Spherecast radius {Radius}`, function() return workspace:Spherecast(V, Radius, Vector3.new(0, -5, 0)) end)
end
for _, Side in { 512, 512.001, 513 } do
	Try(`Blockcast size.x {Side}`, function() return workspace:Blockcast(CFrame.new(V), Vector3.new(Side, 1, 1), Vector3.new(0, -5, 0)) end)
	Try(`Blockcast size.z {Side}`, function() return workspace:Blockcast(CFrame.new(V), Vector3.new(1, 1, Side), Vector3.new(0, -5, 0)) end)
end
Try("Blockcast size -600", function() return workspace:Blockcast(CFrame.new(V), Vector3.new(-600, 1, 1), Down) end)
-- a negative size off a part's face
Try("GetPartBoundsInBox size.x -4 beside Wall", function() return workspace:GetPartBoundsInBox(CFrame.new(306, 90, 300), Vector3.new(-4, 1, 1)) end)
Try("GetPartBoundsInBox size.x 4 beside Wall", function() return workspace:GetPartBoundsInBox(CFrame.new(306, 90, 300), Vector3.new(4, 1, 1)) end)
Try("Shapecast part over 512", function() return workspace:Shapecast(Big, Vector3.new(0, -150, 0)) end)
Try("Shapecast size over 512", function() return workspace:Shapecast(Caster, Down, nil, nil, Vector3.new(600, 1, 1)) end)
-- which check comes first
Try("Spherecast all three", function() return workspace:Spherecast(Vector3.new(NaN, 0, 0), NaN, Vector3.new(0, Inf, 0)) end)
Try("Spherecast position and direction", function() return workspace:Spherecast(Vector3.new(NaN, 0, 0), 1, Vector3.new(0, Inf, 0)) end)
Try("Spherecast position and distance", function() return workspace:Spherecast(Vector3.new(NaN, 0, 0), 1, Vector3.new(0, -2000, 0)) end)
Try("Spherecast radius and direction", function() return workspace:Spherecast(V, NaN, Vector3.new(0, Inf, 0)) end)
Try("Spherecast position and radius", function() return workspace:Spherecast(Vector3.new(NaN, 0, 0), NaN, Down) end)
Try("Spherecast negative radius and direction", function() return workspace:Spherecast(V, -1, Vector3.new(0, Inf, 0)) end)
Try("Spherecast radius over 256 and direction", function() return workspace:Spherecast(V, 300, Vector3.new(0, Inf, 0)) end)
Try("Spherecast radius over 256 and distance", function() return workspace:Spherecast(V, 300, Vector3.new(0, -2000, 0)) end)
Try("Blockcast all three", function() return workspace:Blockcast(CFrame.new(NaN, 0, 0), Vector3.new(NaN, 1, 1), Vector3.new(0, Inf, 0)) end)
Try("Blockcast frame and direction", function() return workspace:Blockcast(CFrame.new(NaN, 0, 0), Vector3.one, Vector3.new(0, Inf, 0)) end)
Try("Blockcast frame and distance", function() return workspace:Blockcast(CFrame.new(NaN, 0, 0), Vector3.one, Vector3.new(0, -2000, 0)) end)
Try("Blockcast size and direction", function() return workspace:Blockcast(CFrame.new(V), Vector3.new(NaN, 1, 1), Vector3.new(0, Inf, 0)) end)
Try("Blockcast frame and size", function() return workspace:Blockcast(CFrame.new(NaN, 0, 0), Vector3.new(NaN, 1, 1), Down) end)
Try("Blockcast size over 512 and direction", function() return workspace:Blockcast(CFrame.new(V), Vector3.new(600, 1, 1), Vector3.new(0, Inf, 0)) end)
Try("Blockcast size over 512 and frame", function() return workspace:Blockcast(CFrame.new(NaN, 0, 0), Vector3.new(600, 1, 1), Down) end)
Try("Blockcast size over 512 and distance", function() return workspace:Blockcast(CFrame.new(V), Vector3.new(600, 1, 1), Vector3.new(0, -2000, 0)) end)
Try("Shapecast frame and size", function() return workspace:Shapecast(Caster, Down, nil, CFrame.new(NaN, 0, 0), Vector3.new(NaN, 1, 1)) end)
Try("Shapecast frame and direction", function() return workspace:Shapecast(Caster, Vector3.new(0, Inf, 0), nil, CFrame.new(NaN, 0, 0)) end)
Try("Shapecast size and direction", function() return workspace:Shapecast(Caster, Vector3.new(0, Inf, 0), nil, nil, Vector3.new(NaN, 1, 1)) end)
Try("Shapecast size over 512 and distance", function() return workspace:Shapecast(Caster, Vector3.new(0, -2000, 0), nil, nil, Vector3.new(600, 1, 1)) end)
Try("Shapecast frame and distance", function() return workspace:Shapecast(Caster, Vector3.new(0, -2000, 0), nil, CFrame.new(NaN, 0, 0)) end)
