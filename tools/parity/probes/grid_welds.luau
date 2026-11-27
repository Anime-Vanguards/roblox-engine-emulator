-- Welded parts in the spatial grid (world/broadphase.rs): the order GetPartBoundsInBox lists a model's parts when it comes in
-- with its welds (by weld kind, direction, holder and where the welds sit), welds made while the parts wait and after they were
-- taken in, welds that end, a member leaving and coming back, Anchored writes, an assembly's level, moves, a part leaving the
-- wait, and ray ties beside a welded part. Each scene 512 studs from the last, its parts in one 8-stud cell unless said.
do
	P.sync()
	local Base = 0
	local function Scene(Name, Specs, Welds, Options)
		Options = Options or {}
		Base += 1
		local Origin = Vector3.new(512 * Base, 64, 512)
		local Holder = Instance.new(Options.Folder and "Folder" or "Model")
		Holder.Name = Name
		local Parts = {}
		for Index, Spec in Specs do
			local Part = Instance.new("Part")
			Part.Name = Spec[1]
			Part.Size = Vector3.one * Spec[2]
			Part.Anchored = Spec[3] or false
			Part.CanCollide = false
			Part.CFrame = CFrame.new(Origin + (Spec[4] or Vector3.new(Index, 4, 4)))
			Part.Parent = Holder
			Parts[Spec[1]] = Part
		end
		local function List(Size, At)
			local Names = {}
			for _, Part in workspace:GetPartBoundsInBox(CFrame.new(Origin + (At or Vector3.new(4, 4, 4))), Size or Vector3.new(7.8, 7.8, 7.8)) do
				table.insert(Names, Part.Name)
			end
			return table.concat(Names, ",")
		end
		local Made = {}
		local function Weld(Pair)
			local W = Instance.new(Options.Kind or "WeldConstraint")
			W.Part0 = Parts[Pair[1]]
			W.Part1 = Parts[Pair[2]]
			if W:IsA("JointInstance") then
				W.C0 = W.Part0.CFrame:ToObjectSpace(W.Part1.CFrame)
			end
			W.Parent = Parts[Pair[Options.Under or 1]]
			Made[Pair[1] .. Pair[2]] = W
			return W
		end
		if not Options.After then
			for _, Pair in Welds do
				Weld(Pair)
			end
		end
		Holder.Parent = workspace
		if Options.After then
			print("welds1", Name, "in", List(Options.Size, Options.At))
			for _, Pair in Welds do
				Weld(Pair)
				if Options.Each then
					print("welds1", Name, "weld " .. Pair[1] .. Pair[2], List(Options.Size, Options.At))
				end
			end
		end
		print("welds1", Name, "listed", List(Options.Size, Options.At))
		if Options.Then then
			Options.Then(Parts, Made, function(Label)
				print("welds1", Name, Label, List(Options.Size, Options.At))
			end)
		end
		Holder:Destroy()
	end
	local function S(Names, Sizes)
		local Out = {}
		for Index, Name in Names do
			table.insert(Out, { Name, Sizes and Sizes[Index] or 1 })
		end
		return Out
	end
	-- pairs and chains in one cell, welded before they come in
	Scene("pair", S({ "a", "b" }), { { "a", "b" } })
	Scene("pair-rev", S({ "a", "b" }), { { "b", "a" } })
	Scene("pair-under-b", S({ "a", "b" }), { { "a", "b" } }, { Under = 2 })
	Scene("pair-big-a", S({ "a", "b" }, { 1.2, 0.8 }), { { "a", "b" } })
	Scene("pair-big-b", S({ "a", "b" }, { 0.8, 1.2 }), { { "a", "b" } })
	Scene("pair-x", S({ "a", "x", "b" }), { { "a", "b" } })
	Scene("pair-x-rev", S({ "a", "x", "b" }), { { "b", "a" } })
	Scene("chain3", S({ "a", "b", "c" }), { { "a", "b" }, { "b", "c" } })
	Scene("chain3-rev", S({ "a", "b", "c" }), { { "b", "c" }, { "a", "b" } })
	Scene("chain3-big-c", S({ "a", "b", "c" }, { 0.8, 0.8, 1.2 }), { { "a", "b" }, { "b", "c" } })
	Scene("chain3-big-a", S({ "a", "b", "c" }, { 1.2, 0.8, 0.8 }), { { "a", "b" }, { "b", "c" } })
	Scene("star", S({ "a", "b", "c", "d" }), { { "a", "b" }, { "a", "c" }, { "a", "d" } })
	Scene("star-d", S({ "a", "b", "c", "d" }), { { "d", "a" }, { "d", "b" }, { "d", "c" } })
	Scene("two", S({ "a1", "b1", "a2", "b2" }), { { "a1", "a2" }, { "b1", "b2" } })
	Scene("two-rev", S({ "a1", "b1", "a2", "b2" }), { { "b1", "b2" }, { "a1", "a2" } })
	Scene("model6", { { "m1", 0.75 }, { "m2", 1 }, { "m3", 0.5, true }, { "m4", 0.75 }, { "m5", 1 }, { "m6", 0.5 } }, { { "m1", "m2" }, { "m2", "m4" }, { "m3", "m5" }, { "m6", "m1" } })
	Scene("model6-folder", { { "m1", 0.75 }, { "m2", 1 }, { "m3", 0.5, true }, { "m4", 0.75 }, { "m5", 1 }, { "m6", 0.5 } }, { { "m1", "m2" }, { "m2", "m4" }, { "m3", "m5" }, { "m6", "m1" } }, { Folder = true })
	Scene("model6-weld", { { "m1", 0.75 }, { "m2", 1 }, { "m3", 0.5, true }, { "m4", 0.75 }, { "m5", 1 }, { "m6", 0.5 } }, { { "m1", "m2" }, { "m2", "m4" }, { "m3", "m5" }, { "m6", "m1" } }, { Kind = "Weld" })
	Scene("anchored", { { "x", 1 }, { "a", 1, true }, { "b", 1 }, { "y", 1 }, { "c", 1 } }, { { "b", "a" }, { "a", "c" } })
	-- welded after they came in
	Scene("after-pair", S({ "a", "b" }), { { "a", "b" } }, { After = true })
	Scene("after-pair-x", S({ "a", "x", "b" }), { { "a", "b" } }, { After = true })
	Scene("after-chain3", S({ "a", "b", "c" }), { { "a", "b" }, { "b", "c" } }, { After = true, Each = true })
	Scene("after-star", S({ "a", "b", "c", "d" }), { { "a", "b" }, { "a", "c" }, { "a", "d" } }, { After = true, Each = true })
	Scene("after-two", S({ "a1", "b1", "a2", "b2" }), { { "a1", "a2" }, { "b1", "b2" } }, { After = true, Each = true })
	-- level: a part welded to a far one, beside a lone part, a small query around them
	Scene("far", { { "p", 1, false, Vector3.new(3, 4, 4) }, { "r", 1, false, Vector3.new(5, 4, 4) }, { "q", 1, false, Vector3.new(203, 4, 4) } }, { { "p", "q" } }, { Size = Vector3.new(5, 2, 2), At = Vector3.new(4, 4, 4) })
	Scene("far-r-first", { { "r", 1, false, Vector3.new(5, 4, 4) }, { "p", 1, false, Vector3.new(3, 4, 4) }, { "q", 1, false, Vector3.new(203, 4, 4) } }, { { "p", "q" } }, { Size = Vector3.new(5, 2, 2), At = Vector3.new(4, 4, 4) })
	Scene("far-after", { { "p", 1, false, Vector3.new(3, 4, 4) }, { "r", 1, false, Vector3.new(5, 4, 4) }, { "q", 1, false, Vector3.new(203, 4, 4) } }, { { "p", "q" } }, { Size = Vector3.new(5, 2, 2), At = Vector3.new(4, 4, 4), After = true })
	-- across cells: the parts of one assembly in two cells, a lone part between
	Scene("cells", { { "a", 1, false, Vector3.new(2, 4, 4) }, { "x", 1, false, Vector3.new(10, 4, 4) }, { "b", 1, false, Vector3.new(18, 4, 4) } }, { { "a", "b" } }, { Size = Vector3.new(23.8, 7.8, 7.8), At = Vector3.new(12, 4, 4) })
	Scene("cells-rev", { { "b", 1, false, Vector3.new(18, 4, 4) }, { "x", 1, false, Vector3.new(10, 4, 4) }, { "a", 1, false, Vector3.new(2, 4, 4) } }, { { "a", "b" } }, { Size = Vector3.new(23.8, 7.8, 7.8), At = Vector3.new(12, 4, 4) })
	-- welds that end, and Anchored writes
	Scene("ends", S({ "a", "b", "c", "d" }), { { "a", "b" }, { "b", "c" }, { "c", "d" } }, {
		Then = function(Parts, Made, Show)
			Made.bc:Destroy()
			Show("bc-ends")
			Made.ab:Destroy()
			Show("ab-ends")
		end,
	})
	Scene("ends-cells", { { "m1", 0.75, false, Vector3.new(-6, 4, 4) }, { "m2", 1, false, Vector3.new(-2, 4, 4) }, { "m3", 0.5, true, Vector3.new(2, 4, 4) }, { "m4", 0.75, false, Vector3.new(6, 4, 4) }, { "m5", 1, false, Vector3.new(10, 4, 4) }, { "m6", 0.5, false, Vector3.new(14, 4, 4) } }, { { "m1", "m2" }, { "m2", "m4" }, { "m3", "m5" }, { "m6", "m1" } }, {
		Size = Vector3.new(40, 7.8, 7.8),
		At = Vector3.new(4, 4, 4),
		Then = function(Parts, Made, Show)
			Made.m2m4:Destroy()
			Show("weld-ends")
			Parts.m1.Anchored = true
			Show("anchored")
			Parts.m1.Anchored = false
			Show("unanchored")
			Parts.m4.Anchored = true
			Show("m4-anchored")
		end,
	})
	Scene("anchor-write", S({ "a", "b", "c", "x" }), { { "a", "b" }, { "b", "c" } }, {
		Then = function(Parts, Made, Show)
			Parts.b.Anchored = true
			Show("b-anchored")
			Parts.b.Anchored = false
			Show("b-free")
			Parts.c.Anchored = true
			Show("c-anchored")
		end,
	})
end
do
	P.sync()
	local Base = 0
	local function Scene(Name, Specs, Welds, Options)
		Options = Options or {}
		Base += 1
		local Origin = Vector3.new(512 * Base, 64, 1024)
		local Holder = Instance.new("Model")
		Holder.Name = Name
		local Parts = {}
		for Index, Spec in Specs do
			local Part = Instance.new("Part")
			Part.Name = Spec[1]
			Part.Size = Spec.Size or Vector3.one * (Spec[2] or 1)
			Part.Anchored = Spec[3] or false
			Part.CanCollide = false
			Part.CFrame = CFrame.new(Origin + (Spec[4] or Vector3.new(Index, 4, 4)))
			Parts[Spec[1]] = Part
		end
		for _, Spec in (Options.ParentOrder or Specs) do
			Parts[typeof(Spec) == "table" and Spec[1] or Spec].Parent = Holder
		end
		local function List(Size, At)
			local Names = {}
			for _, Part in workspace:GetPartBoundsInBox(CFrame.new(Origin + (At or Vector3.new(4, 4, 4))), Size or Vector3.new(7.8, 7.8, 7.8)) do
				table.insert(Names, Part.Name)
			end
			return table.concat(Names, ",")
		end
		local Made = {}
		local function Weld(Pair)
			local W = Instance.new("WeldConstraint")
			W.Part0 = Parts[Pair[1]]
			W.Part1 = Parts[Pair[2]]
			W.Parent = Parts[Pair[3] or Pair[1]]
			Made[Pair[1] .. Pair[2]] = W
			return W
		end
		if not Options.After then
			for _, Pair in Welds do
				Weld(Pair)
			end
		end
		Holder.Parent = workspace
		local function Show(Label)
			print("welds2", Name, Label, List(Options.Size, Options.At))
		end
		if Options.After then
			Show("in")
			for _, Pair in Welds do
				Weld(Pair)
				Show("weld " .. Pair[1] .. Pair[2])
			end
		else
			Show("listed")
		end
		if Options.Then then
			Options.Then(Parts, Made, Show, Origin)
		end
		Holder:Destroy()
	end
	local function S(Names, Sizes)
		local Out = {}
		for Index, Name in Names do
			table.insert(Out, { Name, Sizes and Sizes[Index] or 1 })
		end
		return Out
	end
	-- made order against the order the parts came in (parented)
	Scene("chain-parented-cab", S({ "a", "b", "c" }), { { "a", "b" }, { "b", "c" } }, { ParentOrder = { "c", "a", "b" } })
	Scene("chain-parented-bca", S({ "a", "b", "c" }), { { "a", "b" }, { "b", "c" } }, { ParentOrder = { "b", "c", "a" } })
	Scene("pair-parented-ba", S({ "a", "b" }), { { "a", "b" } }, { ParentOrder = { "b", "a" } })
	Scene("pair-big-parented-ba", S({ "a", "b" }, { 1.2, 0.8 }), { { "a", "b" } }, { ParentOrder = { "b", "a" } })
	Scene("four-parented-dcba", S({ "a", "b", "c", "d" }), { { "a", "b" }, { "b", "c" }, { "c", "d" } }, { ParentOrder = { "d", "c", "b", "a" } })
	Scene("four-parented-bdac", S({ "a", "b", "c", "d" }), { { "a", "b" }, { "b", "c" }, { "c", "d" } }, { ParentOrder = { "b", "d", "a", "c" } })
	-- three: chains whose root is the middle or an end, triangles
	Scene("chain-root-mid", S({ "a", "b", "c" }, { 0.8, 1.2, 0.8 }), { { "a", "b" }, { "b", "c" } })
	Scene("chain-root-a", S({ "a", "b", "c" }, { 1.2, 0.8, 0.8 }), { { "c", "b" }, { "b", "a" } })
	Scene("triangle", S({ "a", "b", "c" }), { { "a", "b" }, { "b", "c" }, { "c", "a" } })
	Scene("three-x", S({ "a", "x", "b", "y", "c" }), { { "c", "a" }, { "b", "c" } })
	Scene("x-first", S({ "x", "a", "b", "c" }), { { "c", "b" }, { "b", "a" } })
	-- built after they came in, in other orders
	Scene("after-cd-ab-bc", S({ "a", "b", "c", "d" }), { { "c", "d" }, { "a", "b" }, { "b", "c" } }, { After = true })
	Scene("after-bc-ab", S({ "a", "b", "c" }), { { "b", "c" }, { "a", "b" } }, { After = true })
	Scene("after-ca-cb", S({ "a", "b", "c", "x" }), { { "c", "a" }, { "c", "b" } }, { After = true })
	Scene("after-big", S({ "a", "b", "c" }, { 0.8, 0.8, 1.2 }), { { "a", "b" }, { "b", "c" } }, { After = true })
	Scene("after-pair-big-first", S({ "a", "b", "x" }, { 1.2, 0.8, 1 }), { { "a", "b" } }, { After = true })
	Scene("after-pair-under", S({ "a", "x", "b" }), { { "a", "b", "b" } }, { After = true })
	Scene("after-pair-rev", S({ "a", "x", "b" }), { { "b", "a" } }, { After = true })
	-- moves: a welded part moved in its cell, beside lone parts
	Scene("move", S({ "x", "a", "b", "y" }), { { "a", "b" } }, {
		Then = function(Parts, Made, Show)
			Parts.a.CFrame = Parts.a.CFrame * CFrame.new(0, 0.5, 0)
			Show("a-moved")
			Parts.x.CFrame = Parts.x.CFrame * CFrame.new(0, 0.5, 0)
			Show("x-moved")
			Parts.b.CFrame = Parts.b.CFrame * CFrame.new(0, 0.5, 0)
			Show("b-moved")
		end,
	})
	Scene("move-out-back", S({ "x", "a", "b", "y" }), { { "a", "b" } }, {
		Then = function(Parts, Made, Show)
			local Home = Parts.a.CFrame
			Parts.a.CFrame = Home * CFrame.new(0, 40, 0)
			Parts.a.CFrame = Home
			Show("a-out-back")
		end,
	})
	-- levels: two small welded parts d studs apart, a lone part beside one, a small query around both
	for _, D in { 100, 147, 149, 200 } do
		Scene("level-" .. D, { { "p", 1, false, Vector3.new(3, 4, 4) }, { "r", 1, false, Vector3.new(5, 4, 4) }, { "q", 1, false, Vector3.new(3 + D, 4, 4) } }, { { "p", "q" } }, { Size = Vector3.new(5, 2, 2), At = Vector3.new(4, 4, 4) })
	end
	-- an assembly that sits in two cells and a lone part in the second
	Scene("span", { { "a", 1, false, Vector3.new(4, 4, 4) }, { "x", 1, false, Vector3.new(12, 4, 4) }, { "b", 1, false, Vector3.new(13, 4, 4) } }, { { "a", "b" } }, { Size = Vector3.new(6, 6, 6), At = Vector3.new(12, 4, 4) })
	Scene("span-b", { { "x", 1, false, Vector3.new(12, 4, 4) }, { "a", 1, false, Vector3.new(4, 4, 4) }, { "b", 1, false, Vector3.new(13, 4, 4) } }, { { "a", "b" } }, { Size = Vector3.new(6, 6, 6), At = Vector3.new(12, 4, 4) })
	-- ties: a ray onto two tops at the same height, a lone part and a welded one
	Scene("ray-tie", { { "x", 1, false, Vector3.new(4, 4, 4) }, { "a", 1, false, Vector3.new(4, 4, 4) }, { "b", 1, false, Vector3.new(4, 4, 104) } }, { { "a", "b" } }, {
		Then = function(Parts, Made, Show, Origin)
			local Hit = workspace:Raycast(Origin + Vector3.new(4, 10, 4), Vector3.new(0, -10, 0))
			print("welds2 ray-tie hit", Hit and Hit.Instance.Name)
			local Box = workspace:Blockcast(CFrame.new(Origin + Vector3.new(4, 10, 4)), Vector3.one * 0.5, Vector3.new(0, -10, 0))
			print("welds2 ray-tie block", Box and Box.Instance.Name)
		end,
	})
	Scene("ray-tie-near", { { "x", 1, false, Vector3.new(4, 4, 4) }, { "a", 1, false, Vector3.new(4, 4, 4) }, { "b", 1, false, Vector3.new(4, 4, 5.5) } }, { { "a", "b" } }, {
		Then = function(Parts, Made, Show, Origin)
			local Hit = workspace:Raycast(Origin + Vector3.new(4, 10, 4), Vector3.new(0, -10, 0))
			print("welds2 ray-tie-near hit", Hit and Hit.Instance.Name)
		end,
	})
end
do
	P.sync()
	local Base = 0
	local function Origin()
		Base += 1
		return Vector3.new(512 * Base, 64, 1536)
	end
	local function List(At, Size)
		local Names = {}
		for _, Part in workspace:GetPartBoundsInBox(CFrame.new(At + Vector3.new(4, 4, 4)), Size or Vector3.new(7.8, 7.8, 7.8)) do
			table.insert(Names, Part.Name)
		end
		return table.concat(Names, ",")
	end
	local function Part(Name, At, Parent, Size)
		local P1 = Instance.new("Part")
		P1.Name = Name
		P1.Size = Vector3.one * (Size or 1)
		P1.CanCollide = false
		P1.CFrame = CFrame.new(At)
		P1.Parent = Parent
		return P1
	end
	local function Weld(A, B, Parent)
		local W = Instance.new("WeldConstraint")
		W.Part0, W.Part1 = A, B
		W.Parent = Parent or A
		return W
	end
	-- 1: the wait: a part leaving it before the take-in
	do
		local O = Origin()
		local Ps = {}
		for I = 1, 5 do
			Ps[I] = Part("p" .. I, O + Vector3.new(I, 4, 4), workspace)
		end
		Ps[2].Parent = nil
		print("welds3 wait-leave", List(O))
		Ps[2].Parent = workspace
		print("welds3 wait-leave back", List(O))
		for _, P1 in Ps do P1:Destroy() end
	end
	-- 2: in the cells: a part leaving after the take-in
	do
		local O = Origin()
		local Ps = {}
		for I = 1, 5 do
			Ps[I] = Part("p" .. I, O + Vector3.new(I, 4, 4), workspace)
		end
		List(O)
		Ps[2].Parent = nil
		print("welds3 cell-leave", List(O))
		for _, P1 in Ps do P1:Destroy() end
	end
	-- 3: welds in a folder after the parts, and before them
	for _, Before in { false, true } do
		local O = Origin()
		local M = Instance.new("Model")
		local Welds = Instance.new("Folder")
		if Before then Welds.Parent = M end
		local A = Part("a", O + Vector3.new(1, 4, 4), M)
		local X = Part("x", O + Vector3.new(2, 4, 4), M)
		local B = Part("b", O + Vector3.new(3, 4, 4), M)
		local Y = Part("y", O + Vector3.new(4, 4, 4), M)
		local C = Part("c", O + Vector3.new(5, 4, 4), M)
		if not Before then Welds.Parent = M end
		Weld(C, A, Welds)
		Weld(B, C, Welds)
		M.Parent = workspace
		print("welds3 folder", Before, List(O))
		M:Destroy()
	end
	-- 4: splits: a chain of four losing its middle weld, roots at either end; a star losing its hub
	for _, Big in { "a", "d", "b" } do
		local O = Origin()
		local M = Instance.new("Model")
		local Ps = {}
		for I, Name in { "a", "b", "c", "d" } do
			Ps[Name] = Part(Name, O + Vector3.new(I, 4, 4), M, Name == Big and 1.2 or 0.8)
		end
		Weld(Ps.a, Ps.b)
		local Mid = Weld(Ps.b, Ps.c)
		Weld(Ps.c, Ps.d)
		local X = Part("x", O + Vector3.new(6, 4, 4), M)
		M.Parent = workspace
		print("welds3 split", Big, "in", List(O))
		Mid:Destroy()
		print("welds3 split", Big, "mid-ends", List(O))
		M:Destroy()
	end
	do
		local O = Origin()
		local M = Instance.new("Model")
		local Hub = Part("h", O + Vector3.new(1, 4, 4), M)
		local Leaves = {}
		for I, Name in { "a", "b", "c" } do
			Leaves[Name] = Part(Name, O + Vector3.new(1 + I, 4, 4), M)
		end
		for _, Name in { "a", "b", "c" } do
			Weld(Hub, Leaves[Name])
		end
		M.Parent = workspace
		print("welds3 star", "in", List(O))
		Hub.Parent = nil
		print("welds3 star", "hub-out", List(O))
		Hub.Parent = M
		print("welds3 star", "hub-back", List(O))
		M:Destroy()
	end
	-- 5: a unit growing past its level after the take-in, beside a lone part
	do
		local O = Origin()
		local A = Part("a", O + Vector3.new(3, 4, 4), workspace)
		local R = Part("r", O + Vector3.new(5, 4, 4), workspace)
		local Far = Part("f", O + Vector3.new(203, 4, 4), workspace)
		print("welds3 grow", "in", List(O, Vector3.new(5, 2, 2)))
		local W = Weld(A, Far)
		print("welds3 grow", "welded", List(O, Vector3.new(5, 2, 2)))
		W:Destroy()
		print("welds3 grow", "unwelded", List(O, Vector3.new(5, 2, 2)))
		A:Destroy() R:Destroy() Far:Destroy()
	end
	-- 6: a unit moved by its root, by a member, and a member moved out of the cell and back
	do
		local O = Origin()
		local X = Part("x", O + Vector3.new(1, 4, 4), workspace)
		local A = Part("a", O + Vector3.new(2, 4, 4), workspace)
		local B = Part("b", O + Vector3.new(3, 4, 4), workspace, 1.2)
		local Y = Part("y", O + Vector3.new(4, 4, 4), workspace)
		Weld(A, B)
		print("welds3 move", "in", List(O))
		B.CFrame = B.CFrame * CFrame.new(0, 0.3, 0)
		print("welds3 move", "root-moved", List(O))
		A.CFrame = A.CFrame * CFrame.new(0, 40, 0)
		print("welds3 move", "member-out", List(O))
		A.CFrame = A.CFrame * CFrame.new(0, -40, 0)
		print("welds3 move", "member-back", List(O))
		X:Destroy() A:Destroy() B:Destroy() Y:Destroy()
	end
	-- 7: anchoring: a member of a pair anchored and freed; a lone part welded to an anchored one after the take-in
	do
		local O = Origin()
		local X = Part("x", O + Vector3.new(1, 4, 4), workspace)
		local A = Part("a", O + Vector3.new(2, 4, 4), workspace)
		local B = Part("b", O + Vector3.new(3, 4, 4), workspace)
		local K = Part("k", O + Vector3.new(4, 4, 4), workspace)
		K.Anchored = true
		local Y = Part("y", O + Vector3.new(5, 4, 4), workspace)
		Weld(A, B)
		print("welds3 anchor", "in", List(O))
		A.Anchored = true
		print("welds3 anchor", "a-anchored", List(O))
		A.Anchored = false
		print("welds3 anchor", "a-free", List(O))
		Weld(Y, K)
		print("welds3 anchor", "y-to-k", List(O))
		Weld(B, K)
		print("welds3 anchor", "b-to-k", List(O))
		for _, P1 in { X, A, B, K, Y } do P1:Destroy() end
	end
end
do
	P.sync()
	local Base = 0
	local function Origin()
		Base += 1
		return Vector3.new(512 * Base, 64, 2048)
	end
	local function List(At)
		local Names = {}
		for _, Part in workspace:GetPartBoundsInBox(CFrame.new(At + Vector3.new(4, 4, 4)), Vector3.new(7.8, 7.8, 7.8)) do
			table.insert(Names, Part.Name)
		end
		return table.concat(Names, ",")
	end
	-- a layout: a list of names in child order; "a", "x", "b" parts; "W" the weld a-b; "F(...)" a folder holding the listed
	local function Build(Name, Layout, Kind)
		local O = Origin()
		local M = Instance.new("Model")
		local Parts = {}
		local Welds = {}
		local Spot = 0
		local function Put(Item, Parent)
			if type(Item) == "table" then
				local F = Instance.new("Folder")
				F.Name = "F"
				for _, Inner in Item do
					Put(Inner, F)
				end
				F.Parent = Parent
			elseif Item == "W" or Item == "V" then
				-- made in its place, its parts set once they exist (a weld without parts links nothing)
				local W = Instance.new(Kind or "WeldConstraint")
				W.Parent = Parent
				table.insert(Welds, { Item, W })
			else
				Spot += 1
				local P1 = Instance.new("Part")
				P1.Name = Item
				P1.Size = Vector3.one
				P1.CanCollide = false
				P1.CFrame = CFrame.new(O + Vector3.new(Spot, 4, 4))
				P1.Parent = Parent
				Parts[Item] = P1
			end
		end
		for _, Item in Layout do
			Put(Item, M)
		end
		-- the welds made last, under their holders (a holder "a" etc. is the part of that name)
		for _, Entry in Welds do
			local W = Entry[2]
			if Entry[1] == "W" then
				W.Part0, W.Part1 = Parts.a, Parts.b
			else
				W.Part0, W.Part1 = Parts.b, Parts.c
			end
			if W:IsA("JointInstance") then
				W.C0 = W.Part0.CFrame:ToObjectSpace(W.Part1.CFrame)
			end
		end
		M.Parent = workspace
		print("welds4", Name, List(O))
		M:Destroy()
	end
	Build("ax-b-F", { "a", "x", "b", { "W" } })
	Build("F-axb", { { "W" }, "a", "x", "b" })
	Build("a-F-xb", { "a", { "W" }, "x", "b" })
	Build("ax-F-b", { "a", "x", { "W" }, "b" })
	Build("axbW", { "a", "x", "b", "W" })
	Build("Waxb", { "W", "a", "x", "b" })
	Build("axWb", { "a", "x", "W", "b" })
	Build("axb-weld", { "a", "x", "b", "W" }, "Weld")
	Build("Waxb-weld", { "W", "a", "x", "b" }, "Weld")
	-- welds a-b and b-c with lone x, y between
	Build("axbyc-FWV", { "a", "x", "b", "y", "c", { "W", "V" } })
	Build("FWV-axbyc", { { "W", "V" }, "a", "x", "b", "y", "c" })
	Build("axbyc-WV", { "a", "x", "b", "y", "c", "W", "V" })
	Build("WV-axbyc", { "W", "V", "a", "x", "b", "y", "c" })
	Build("aW-x-bV-y-c", { "a", "W", "x", "b", "V", "y", "c" })
end
