-- Rigid assemblies as units of the spatial grid (world/broadphase.rs settle): the order GetPartBoundsInBox lists welded parts
-- in, all in one 8-stud cell, as welds join and split them (a join keeps the root's unit, a split keeps the root's piece), as parts
-- come in with their welds (each link when the last of its joint and parts comes in, a part's links newest named first), as parts
-- leave (the root too), as Anchored writes split and join assemblies, and as writes of Size, Massless, RootPriority or Name move a
-- unit of several to the end. Each scene is an op script, one op per ';':
--   p NAME [size] [A]   a Part, unparented (anchored with A); parts sit 0.4 studs apart in the scene's cell
--   m / f / wm NAME     a Model, Folder or WorldModel
--   w NAME P0 P1        a WeldConstraint with Part0 and Part1 set ('-' for nil), unparented; s NAME P0 P1 sets them again
--   > CHILD PARENT      a Parent write (ws: the scene's folder in the Workspace, nil)
--   q TAG               GetPartBoundsInBox over the cell, printed (q - takes the wait in, printing nothing)
--   x NAME              Destroy; a NAME 0|1 Anchored; e NAME 0|1 a weld's Enabled; sz NAME n Size; ml NAME 0|1 Massless;
--   nm NAME new         Name; rp NAME n RootPriority; mat, cpp, shp, tr, col, cg NAME: Material, CustomPhysicalProperties, Shape,
--                       Transparency, Color, CollisionGroup; cc / ct NAME 0|1 CanCollide / CanTouch; o NAME moved out and back
P.sync()
local Base = 0
local function Run(Script)
	Base += 1
	local Origin = Vector3.new(512 * Base, 64, 3584)
	local Root = Instance.new("Folder")
	Root.Name = "A3"
	Root.Parent = workspace
	local Things = {}
	local Spot = 0
	local function Get(Name)
		if Name == "ws" then return Root end
		if Name == "nil" or Name == "-" then return nil end
		local Thing = Things[Name]
		if not Thing then error("no " .. Name, 0) end
		return Thing
	end
	for Op in string.gmatch(Script, "[^;]+") do
		local T = {}
		for Word in string.gmatch(Op, "%S+") do
			table.insert(T, Word)
		end
		local Code = T[1]
		if Code == "p" then
			Spot += 1
			local Part = Instance.new("Part")
			Part.Name = T[2]
			Part.Size = Vector3.one * (tonumber(T[3]) or 1)
			Part.Anchored = T[4] == "A"
			Part.CanCollide = false
			Part.CFrame = CFrame.new(Origin + Vector3.new(1 + 0.4 * Spot, 4, 4))
			Things[T[2]] = Part
		elseif Code == "m" or Code == "f" or Code == "wm" then
			local Holder = Instance.new(Code == "m" and "Model" or Code == "wm" and "WorldModel" or "Folder")
			Holder.Name = T[2]
			Things[T[2]] = Holder
		elseif Code == "w" then
			local Weld = Instance.new("WeldConstraint")
			Weld.Name = T[2]
			Weld.Part0 = Get(T[3])
			Weld.Part1 = Get(T[4])
			Things[T[2]] = Weld
		elseif Code == "s" then
			local Weld = Get(T[2])
			Weld.Part0 = Get(T[3])
			Weld.Part1 = Get(T[4])
		elseif Code == ">" then
			Get(T[2]).Parent = Get(T[3])
		elseif Code == "q" then
			local Names = {}
			for _, Part in workspace:GetPartBoundsInBox(CFrame.new(Origin + Vector3.new(4, 4, 4)), Vector3.new(7.8, 7.8, 7.8)) do
				table.insert(Names, Part.Name)
			end
			local Tag = table.concat(T, " ", 2)
			if Tag ~= "-" then
				print(Tag .. " " .. table.concat(Names, ","))
			end
		elseif Code == "x" then
			Get(T[2]):Destroy()
			Things[T[2]] = nil
		elseif Code == "a" then
			Get(T[2]).Anchored = T[3] == "1"
		elseif Code == "sz" then
			Get(T[2]).Size = Vector3.one * tonumber(T[3])
		elseif Code == "nm" then
			Get(T[2]).Name = T[3]
			Things[T[3]] = Get(T[2])
		elseif Code == "mat" then
			Get(T[2]).Material = Enum.Material.Neon
		elseif Code == "cpp" then
			Get(T[2]).CustomPhysicalProperties = PhysicalProperties.new(2, 0.3, 0.5)
		elseif Code == "shp" then
			Get(T[2]).Shape = Enum.PartType.Ball
		elseif Code == "rp" then
			Get(T[2]).RootPriority = tonumber(T[3])
		elseif Code == "tr" then
			Get(T[2]).Transparency = 0.5
		elseif Code == "cc" then
			Get(T[2]).CanCollide = T[3] == "1"
		elseif Code == "ct" then
			Get(T[2]).CanTouch = T[3] == "1"
		elseif Code == "cg" then
			Get(T[2]).CollisionGroup = "Default"
		elseif Code == "col" then
			Get(T[2]).Color = Color3.new(1, 0, 0)
		elseif Code == "ml" then
			Get(T[2]).Massless = T[3] == "1"
		elseif Code == "e" then
			Get(T[2]).Enabled = T[3] == "1"
		elseif Code == "o" or Code == "out" or Code == "back" then
			local Part = Get(T[2])
			if Code ~= "back" then
				Part.CFrame = Part.CFrame * CFrame.new(0, 40, 0)
			end
			if Code ~= "out" then
				Part.CFrame = Part.CFrame * CFrame.new(0, -40, 0)
			end
		else
			error("op " .. Code, 0)
		end
	end
	Root:Destroy()
end
Run("m M; p p0 1; p p1 1; p p2 1; p p3 1; p p4 1; > p0 M; > p1 M; > p2 M; > p3 M; > p4 M; w W p3 p1; > W M; > M ws; q A1")
Run("m M; p p0 1; p p1 1; p p2 1; p p3 1; p p4 1; > p0 M; > p1 M; > p2 M; > p3 M; > p4 M; w W p1 p3; > W M; > M ws; q A2")
Run("m M; p p0; p p1; p p2; p p3; p p4; > p0 M; > p3 M; > p2 M; > p1 M; > p4 M; w W p1 p3; > W M; > M ws; q A3")
Run("m M; p p0 1; p p1 1.2; p p2 1; p p3 1; p p4 1; > p0 M; > p1 M; > p2 M; > p3 M; > p4 M; w W p3 p1; > W M; > M ws; q A4")
Run("m M; p p0 1; p p1 1; p p2 1; p p3 1.2; p p4 1; > p0 M; > p1 M; > p2 M; > p3 M; > p4 M; w W p1 p3; > W M; > M ws; q A5")
Run("m M; p p0; p p1; p p2; p p3; p p4; > p0 M; > p3 M; > p2 M; > p1 M; > p4 M; w W p3 p1; > W M; > M ws; q A6")
Run("m M; p s0 1; p s1 1; p s2 1; p b1 1; p b2 1; p s3 1; > s0 M; > s1 M; > s2 M; > b1 M; > b2 M; > s3 M; w W1 b1 b2; > W1 M; w W2 s1 b1; > W2 M; > M ws; q B1")
Run("m M; p s0 1; p s1 1; p s2 1; p b1 1; p b2 1; p s3 1; > s0 M; > s1 M; > s2 M; > b1 M; > b2 M; > s3 M; w W1 b1 b2; > W1 M; w W2 b1 s1; > W2 M; > M ws; q B2")
Run("m M; p s0 1; p s1 1; p c1 1; p c2 1; p s2 1; p b1 1; p b2 1; p b3 1; p s3 1; > s0 M; > s1 M; > c1 M; > c2 M; > s2 M; > b1 M; > b2 M; > b3 M; > s3 M; w W1 b1 b2; > W1 M; w W2 b2 b3; > W2 M; w W3 c1 c2; > W3 M; w W4 c1 b1; > W4 M; > M ws; q B3")
Run("m M; p s0 1; p s1 1; p c1 1; p c2 1; p s2 1; p b1 1; p b2 1; p b3 1; p s3 1; > s0 M; > s1 M; > c1 M; > c2 M; > s2 M; > b1 M; > b2 M; > b3 M; > s3 M; w W1 b1 b2; > W1 M; w W2 b2 b3; > W2 M; w W3 c1 c2; > W3 M; w W4 b1 c1; > W4 M; > M ws; q B4")
Run("m M; f F; > F M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 b c; > W1 F; w W2 c a; > W2 F; > M ws; q C1")
Run("m M; f F; > F M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 a c; > W1 F; w W2 b c; > W2 F; > M ws; q C2")
Run("m M; f F; > F M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 c a; > W1 F; w W2 c b; > W2 F; > M ws; q C3")
Run("m M; f F; > F M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 - -; w W2 - -; > W1 F; > W2 F; s W2 b c; s W1 c a; > M ws; q C4")
Run("m M; f F; > F M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 - -; w W2 - -; > W2 F; > W1 F; s W1 c a; s W2 b c; > M ws; q C5")
Run("p a; > a ws; m M; p x; p b; > x M; > b M; w W a b; > W b; > M ws; q D1")
Run("p a; > a ws; p z; > z ws; q -; m M; p x; p b; > x M; > b M; w W a b; > W b; > M ws; q D2")
Run("p a; > a ws; p z; > z ws; m M; p x; p b; > x M; > b M; w W b a; > W b; > M ws; q D3")
Run("p a; > a ws; p b; > b ws; p c; > c ws; p d; > d ws; q -; w W b a; > W b; q E1")
Run("p a; > a ws; p b; > b ws; p c; > c ws; p d; > d ws; q -; p e; > e ws; w W a b; > W a; q F1")
Run("m M; p a 1; p b 1; p c 1; > a M; > b M; > c M; w W a b; > W a; > M ws; q -; p e; > e ws; x W; q G1")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; x W; q H1")
Run("m M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; x b; q H2")
Run("p a; > a ws; p x; > x ws; q -; m M; p b; > b M; w W a b; > W b; > M ws; q I1")
Run("p a; > a ws; p x; > x ws; p y; > y ws; q -; m M; p b; > b M; w W b a; > W b; > M ws; q I2")
Run("m M; p a 1; p x 1; p b 1; > a M; > x M; > b M; w W a b; > W a; > M ws; a a 1; q J1")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W - -; > W M; > M ws; s W a b; q K1")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W M; > M ws; e W 0; e W 1; q K2")
Run("p a 1.2; > a ws; p b; > b ws; p c; > c ws; p d; > d ws; q -; w W a b; > W a; q E2")
Run("p a 1.2; p b; p c; p d; > b ws; > a ws; > c ws; > d ws; q -; w W a b; > W a; q E3")
Run("p a; p b 1.2; p c; p d; > b ws; > a ws; > c ws; > d ws; q -; w W a b; > W a; q E4")
Run("p a 1.2; > a ws; p x; > x ws; q -; m M; p b; > b M; w W a b; > W b; > M ws; q I3")
Run("p a 1.2; > a ws; p x; > x ws; q -; p b; > b ws; w W a b; > W a; q I4")
Run("p a 1.2; > a ws; p x; > x ws; p y; > y ws; q -; m M; p b; > b M; w W b a; > W b; > M ws; q I5")
Run("p a; > a ws; m M; p x; p b; > x M; > b M; w W a b; > W M; > M ws; q D4")
Run("p a 1.2; > a ws; p z; > z ws; m M; p x; p b; > x M; > b M; w W b a; > W b; > M ws; q D5")
Run("m M; p a 1; p b 1; p c 1; p d 1; p x 1; > a M; > b M; > c M; > d M; > x M; w W1 a b; > W1 a; w W2 b c; > W2 b; w W3 c d; > W3 c; > M ws; x W2; q S1")
Run("m M; p p 1; p a 1; p b 1; p q 1; > p M; > a M; > b M; > q M; w W a b; > W a; > M ws; x W; q S2")
Run("m M; p p 1; p a 1.2; p b 1; p q 1; > p M; > a M; > b M; > q M; w W a b; > W a; > M ws; x W; q S3")
Run("m M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; x c; q H3")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; x b; q H4")
Run("m M; p a 1; p x 1; p b 1; p y 1; p c 1; p z 1; > a M; > x M; > b M; > y M; > c M; > z M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; x W2; q H5")
Run("p a; > a ws; p x; > x ws; p b; > b ws; q -; m M; p c; > c M; w W a b; > W M; > M ws; q Y1")
Run("p a; > a ws; p x; > x ws; p b; > b ws; m M; p c; > c M; w W a b; > W M; > M ws; q Y2")
Run("p a; > a ws; p x; > x ws; p b; > b ws; p y; > y ws; m M; p c; > c M; w Wa a c; > Wa M; w Wb b c; > Wb M; > M ws; q Z1")
Run("p a; > a ws; p x; > x ws; p b; > b ws; p y; > y ws; q -; m M; p c; > c M; w Wa a c; > Wa M; w Wb b c; > Wb M; > M ws; q Z2")
Run("p a; > a ws; p x; > x ws; p b; > b ws; p y; > y ws; p c; w Wa a c; w Wb b c; > Wa c; > Wb c; > c ws; q Z3")
Run("p a; > a ws; p b; > b ws; p c; > c ws; p d; > d ws; w W a b; > W a; q W1")
Run("p a; > a ws; p b; > b ws; p c; > c ws; p d; > d ws; w W b a; > W b; q W2")
Run("p a 1.2; > a ws; p b; > b ws; p c; > c ws; p d; > d ws; w W a b; > W a; q W3")
Run("m M; p a 1; p b 1; p c 1; p d 1; > a M; > b M; > c M; > d M; > M ws; w W a b; > W a; q W4")
Run("p a; > a ws; p b; > b ws; p c; > c ws; p d; > d ws; w W - -; > W ws; s W a b; q W5")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 c b; > W1 c; w W2 b a; > W2 b; > M ws; x b; q H6")
Run("m M; p c; p b 1.2; p a; p x; p y; > a M; > x M; > b M; > y M; > c M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; x b; q H7")
Run("m M; p h 1.2; p a 1; p b 1; p c 1; p x 1; > h M; > a M; > b M; > c M; > x M; w Wa h a; > Wa h; w Wb h b; > Wb h; w Wc h c; > Wc h; > M ws; x h; q H8")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; q -; x b; q H9")
Run("m M; p a 1; p b 1; p x 1; p c 1.2; p d 1; p y 1; > a M; > b M; > x M; > c M; > d M; > y M; w W1 a b; > W1 a; w W2 b c; > W2 b; w W3 c d; > W3 c; > M ws; x c; q H10")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W2 b c; > W2 b; w W1 a b; > W1 a; > M ws; x b; q H11")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 b a; > W1 b; w W2 c b; > W2 c; > M ws; x b; q H12")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; > b nil; q H13")
Run("m M; p h 1.2; p a 1; p b 1; p c 1; p x 1; > h M; > a M; > b M; > c M; > x M; w Wc h c; > Wc h; w Wb h b; > Wb h; w Wa h a; > Wa h; > M ws; x h; q H14")
Run("m M; p c; p b; p a; p h 1.2; p x; > h M; > a M; > b M; > c M; > x M; w Wa h a; > Wa h; w Wb h b; > Wb h; w Wc h c; > Wc h; > M ws; x h; q H15")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q P1 in; sz a 1.2; q P1 a-grown; sz a 1; q P1 a-back")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q P2 in; ml b 1; q P2 b-massless")
Run("m M; p a 1; p x 1; p b 1; p c 1; p y 1; > a M; > x M; > b M; > c M; > y M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; q P3 in; sz a 1.2; q P3 a-grown")
Run("wm V; > V ws; m M; p a 1; p x 1; p b 1; > a M; > x M; > b M; w W a b; > W a; > M V; p y; > y ws; > M ws; q V1")
Run("wm V; > V ws; p y; > y ws; q -; m M; p a 1; p x 1; p b 1; > a M; > x M; > b M; w W a b; > W a; > M V; > M ws; q V2")
Run("m M; p A 1 A; p u; p B 1 A; p x; > A M; > u M; > B M; > x M; w W1 A u; > W1 M; w W2 u B; > W2 M; > M ws; q N1 in; x W2; q N1 w2-ends; x W1; q N1 w1-ends")
Run("m M; p A 1 A; p u; p B 1 A; p x; > A M; > u M; > B M; > x M; w W1 u B; > W1 M; w W2 A u; > W2 M; > M ws; q N2 in; x W2; q N2 w2-ends")
Run("m M; p A 1 A; p u; p v; p x; > A M; > u M; > v M; > x M; w W1 A u; > W1 M; w W2 u v; > W2 M; > M ws; q N3 in; a v 1; q N3 v-anchored; a v 0; q N3 v-free; a A 0; q N3 A-free")
Run("p x; > x ws; m M; p A 1 A; p u; > A M; > u M; w W A u; > W M; > M ws; q N4 in; a A 0; q N4 A-free; a u 1; q N4 u-anchored")
Run("m M; p a 1; p x 1; p b 1; p c 1; p y 1; > a M; > x M; > b M; > c M; > y M; w W a b; > W a; > M ws; q R1 in; s W a c; q R1 to-c")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q R2 in; e W 0; q R2 off; e W 1; q R2 on")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; e W 0; q R3 off; e W 1; q R3 on")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q T1 in; > W nil; q T1 out; > W a; q T1 back")
Run("m M; p a 1; p x 1; p b 1; p y 1; p c 1; > a M; > x M; > b M; > y M; > c M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; q T2 in; > b nil; q T2 b-out; > b M; q T2 b-back")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q P4 in; sz a 1.1; q P4 a-grown")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q P5 in; ml a 1; q P5 a-massless")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q P6 in; nm a Torso; q P6 named")
Run("m M; p a 1; p x 1; p b 1; p c 1; p y 1; > a M; > x M; > b M; > c M; > y M; w W1 a b; > W1 a; w W2 b c; > W2 b; > M ws; q P7 in; sz b 1.1; q P7 b-grown; sz c 1.2; q P7 c-grown")
Run("m M; p a 1; p x 1; p b 1; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; sz a 1.2; q P8 grown-waiting")
Run("m M; p x 1; p a 1; p b 1.2; p y 1; > x M; > a M; > b M; > y M; w W a b; > W a; > M ws; q P9 in; sz b 1.1; q P9 b-shrunk")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q1 in; mat a; q Q1 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q2 in; cpp a; q Q2 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q3 in; shp a; q Q3 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q4 in; rp a -1; q Q4 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q5 in; tr a; q Q5 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q6 in; cc a 0; q Q6 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q7 in; ct a 0; q Q7 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q8 in; cg a; q Q8 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q9 in; col a; q Q9 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q10 in; cc a 0; ct a 0; q Q10 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q11 in; mat b; q Q11 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q12 in; sz a 1; q Q12 after")
Run("m M; p a 1; p x 1; p b 1.2; p y 1; > a M; > x M; > b M; > y M; w W a b; > W a; > M ws; q Q13 in; ml b 0; q Q13 after")
Run("m M; p x 1; p a 1; p y 1; > x M; > a M; > y M; > M ws; q Q14 in; ml a 1; sz a 1.1; mat a; nm a Torso; q Q14 after")
