-- The spatial grid's limits (a client's broadphase): how far out each level finds parts (2^(26 + level) studs
-- on each axis, with float32 losses from 2^24 on), a box or radius clamped to 7,500 studs about its centre on each axis,
-- a ray cut at 15,000 studs from its origin, and physics past the far bound. Each line: one part per spot, alone, looked
-- for by a small box at it, F found or - not.
P.sync()
local Root = Instance.new("Folder")
Root.Name = "GridLimits"
Root.Parent = workspace
local function Part(Name, At, Size)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = true
	Made.Size = Size or Vector3.new(2, 2, 2)
	Made.CFrame = if typeof(At) == "CFrame" then At else CFrame.new(At)
	Made.Parent = Root
	return Made
end
local function Index()
	workspace:GetPartBoundsInBox(CFrame.new(0, -10000, 0), Vector3.one)
end
local function Has(List, Wanted)
	return table.find(List, Wanted) ~= nil
end
local V = Vector3.new
local function Marks(Spots, Size, Look)
	local Parts = {}
	for I, At in Spots do
		Parts[I] = Part("f" .. I, At, Size)
	end
	Index()
	local Out = {}
	for I, At in Spots do
		table.insert(Out, if Look(At, Parts[I]) then "F" else "-")
	end
	Root:ClearAllChildren()
	return table.concat(Out)
end
local function ByBox(At, Wanted)
	return Has(workspace:GetPartBoundsInBox(CFrame.new(At), Vector3.one), Wanted)
end
local T = 2 ^ 26

-- 1. the far bound for small parts, on each axis and side: 2^26 + -256 -64 -16 -8 -4 0 +8 +16 +64
local Fine = { -256, -64, -16, -8, -4, 0, 8, 16, 64 }
for _, Axis in { "x", "y", "z" } do
	for _, Sign in { 1, -1 } do
		local Spots = {}
		for I, O in Fine do
			local D = Sign * (T + O)
			Spots[I] = if Axis == "x" then V(D, 0, 300 + I * 40) elseif Axis == "y" then V(300 + I * 40, D, 300) else V(300 + I * 40, 0, D)
		end
		print(`far bound {Axis}{if Sign > 0 then "+" else "-"}, 2^26 -256 -64 -16 -8 -4 0 +8 +16 +64: {Marks(Spots, nil, ByBox)}`)
	end
end
-- 2. by level: parts of several sizes at 2^26, 1.5 * 2^26, 2^27 ... 1.5 * 2^34
local Far = {}
for K = 26, 34 do
	table.insert(Far, 2 ^ K)
	table.insert(Far, 1.5 * 2 ^ K)
end
for _, S in { 2, 8, 20, 30, 100, 200, 300, 600, 1200, 2048 } do
	local Spots = {}
	for I, D in Far do
		Spots[I] = V(D, 0, 300 + I * 3000)
	end
	print(`far by size {S}, 2^26 1.5x 2^27 ... 2^34 1.5x: {Marks(Spots, V(S, S, S), ByBox)}`)
end
-- 3. float32 losses below the bound: size 2 parts at 2^k + 0..31 (studs), and at 2^26 - 512 .. -4 by 4
for _, K in { 23, 24, 25 } do
	local Spots = {}
	for J = 0, 31 do
		Spots[J + 1] = V(2 ^ K + J, 0, 300 + (J % 2) * 40)
	end
	print(`below the bound, 2^{K} + 0..31: {Marks(Spots, nil, ByBox)}`)
end
do
	local Spots = {}
	for J = 0, 31 do
		Spots[J + 1] = V(T - 128 + J * 4, 0, 300 + (J % 2) * 40)
	end
	print(`below the bound, 2^26 - 128 .. -4 by 4: {Marks(Spots, nil, ByBox)}`)
end
-- 4. a 1000-stud part across the small parts' bound is found where it lies
do
	local Spots = { V(T + 400, 0, 300), V(T - 400, 0, 300) }
	local Out = {}
	for _, At in Spots do
		local Made = Part("big", At, V(1000, 1000, 1000))
		Index()
		local Line = {}
		for _, X in { T - 850, T - 300, T + 300, T + 850 } do
			table.insert(Line, if Has(workspace:GetPartBoundsInBox(CFrame.new(X, 0, 300), Vector3.one), Made) then "F" else "-")
		end
		table.insert(Out, table.concat(Line))
		Root:ClearAllChildren()
	end
	print(`1000-stud parts at 2^26 +400 and -400, looked for at -850 -300 +300 +850: {table.concat(Out, " ")}`)
end

-- 5. the long-query clamp: thin parts (0.01 along the axis) 7,495 .. 7,530 studs from a long box's centre
local C = V(100004, 1004, 4)
local function Clamp(Label, Query, Axis)
	local Unit = if Axis == "z" then V(0, 0, 1) else V(1, 0, 0)
	local Offsets = { 7000, 7495, 7499.5, 7500, 7500.5, 7501, 7510, 7530, 9000, -7500, -7501 }
	local Parts = {}
	for I, O in Offsets do
		Parts[I] = Part(tostring(O), C + Unit * O, if Axis == "z" then V(1, 1, 0.01) else V(0.01, 1, 1))
	end
	Index()
	local List = Query()
	local Got = {}
	for I, O in Offsets do
		if Has(List, Parts[I]) then
			table.insert(Got, tostring(O))
		end
	end
	print(`clamp, {Label}: {table.concat(Got, " ")}`)
	Root:ClearAllChildren()
end
Clamp("box 20000 x 4 x 4", function() return workspace:GetPartBoundsInBox(CFrame.new(C), V(20000, 4, 4)) end)
Clamp("box 15000 x 4 x 4", function() return workspace:GetPartBoundsInBox(CFrame.new(C), V(15000, 4, 4)) end)
Clamp("box 14990 x 4 x 4", function() return workspace:GetPartBoundsInBox(CFrame.new(C), V(14990, 4, 4)) end)
Clamp("box 4 x 4 x 20000 along z", function() return workspace:GetPartBoundsInBox(CFrame.new(C), V(4, 4, 20000)) end, "z")
Clamp("box 20000 cube", function() return workspace:GetPartBoundsInBox(CFrame.new(C), V(20000, 20000, 20000)) end)
Clamp("radius 10000", function() return workspace:GetPartBoundsInRadius(C, 10000) end)
do
	-- a long box turned 45 degrees about y: parts along its own axis, found while within 7,500 on x and z
	local Turned = CFrame.new(C) * CFrame.Angles(0, math.rad(45), 0)
	local Offsets = { 9000, 10500, 10600, 10620, 11000 }
	local Parts = {}
	for I, O in Offsets do
		Parts[I] = Part(tostring(O), C + Turned.XVector * O, V(0.01, 0.01, 0.01))
	end
	Index()
	local List = workspace:GetPartBoundsInBox(Turned, V(40000, 4, 4))
	local Got = {}
	for I, O in Offsets do
		if Has(List, Parts[I]) then
			table.insert(Got, tostring(O))
		end
	end
	print(`clamp, box 40000 x 4 x 4 turned 45 about y, along its axis: {table.concat(Got, " ")}`)
	Root:ClearAllChildren()
end
do
	-- a 2000-stud rod past the clamp by D: its coarse cell still meets the clamped box
	local Got = {}
	for _, D in { 0, 1, 8 } do
		local Made = Part("rod", C + V(7500 + D + 1000, 0, 0), V(2000, 1, 1))
		Index()
		if Has(workspace:GetPartBoundsInBox(CFrame.new(C), V(20000, 4, 4)), Made) then
			table.insert(Got, tostring(D))
		end
		Root:ClearAllChildren()
	end
	print(`clamp, 2000-stud rods past it by 0 1 8, found: {table.concat(Got, " ")}`)
end

-- 6. the ray's cut: a 0.2-thick wall whose near face is D along +x from the origin of a 1,000,000-stud ray
do
	local Got = {}
	for _, D in { 7600, 14999.5, 14999.9, 15000, 15000.5, 20000 } do
		local Wall = Part("wall", C + V(D + 0.1, 0, 0), V(0.2, 4, 4))
		Index()
		local Hit = workspace:Raycast(C, V(1e6, 0, 0))
		local Legacy = workspace:FindPartOnRay(Ray.new(C, V(1e6, 0, 0)))
		table.insert(Got, `{D}:{if Hit and Hit.Instance == Wall then "hit" else "-"}/{if Legacy == Wall then "hit" else "-"}`)
		Root:ClearAllChildren()
	end
	print(`ray cut, Raycast/FindPartOnRay: {table.concat(Got, " ")}`)
	local Got2 = {}
	for _, D in { 14000, 16000 } do
		local Wall = Part("wall", C + V(-50000 + D, 0, 0), V(0.2, 4, 4))
		Index()
		local Hit = workspace:Raycast(C + V(-50000, 0, 0), V(1e6, 0, 0))
		table.insert(Got2, `{D}:{if Hit and Hit.Instance == Wall then "hit" else "-"}`)
		Root:ClearAllChildren()
	end
	print(`ray cut, from 50,000 studs back: {table.concat(Got2, " ")}`)
end

-- 7. rays down onto a 2-stud part far out (public #58 found a miss at 2e7 on 0.748)
do
	local Got = {}
	for _, X in { 1.6e7, 1.7e7, 2e7, 2e7 + 2, 3e7, 5e7, 7e7 } do
		local Made = Part("P", V(X, 0, 0), V(2, 2, 2))
		Index()
		local Hit = workspace:Raycast(V(X, 5, 0), V(0, -10, 0))
		table.insert(Got, `{string.format("%.0f", X)}:{if Hit and Hit.Instance == Made then "hit" else "-"}/{if Has(workspace:GetPartBoundsInBox(CFrame.new(X, 0, 0), Vector3.one), Made) then "F" else "-"}`)
		Root:ClearAllChildren()
	end
	print(`ray down/box at: {table.concat(Got, " ")}`)
end

-- 8. physics past the far bound: an unanchored block dropped onto a 16-stud floor, a second later
for _, X in { 1e6, 5e7, 1e8 } do
	local Floor = Part("floor", V(X, 1000, 300), V(16, 1, 16))
	local Block = Part("block", V(X, 1003, 300), V(2, 2, 2))
	Block.Anchored = false
	local Touched = 0
	Block.Touched:Connect(function()
		Touched += 1
	end)
	task.wait(1)
	print(`physics at x {X}: {if Block.Position.Y > 1000 then "on the floor" else "fell through"}, Touched {if Touched > 0 then "fired" else "never"}`)
	Floor:Destroy()
	Block:Destroy()
end
Root:Destroy()
