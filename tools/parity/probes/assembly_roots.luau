-- Which part roots each assembly (AssemblyRootPart), as a playing client links parts: an unanchored part welded to several
-- anchored ones (welds made, parented and given their parts in different orders, re-pointed, disabled, cloned, whole models
-- entering), chains, bridges and rings between two anchored parts, free stars with leaves anchored in either order, the
-- joint kinds' order against each other (every pair of Weld, ManualWeld, Snap, Glue, ManualGlue, Motor, Motor6D,
-- WeldConstraint, Rotate, RotateP, RotateV and VelocityMotor), Glue and ManualGlue against Welds (roots, mass, placing, moving),
-- where a joint must be to link (JointsService, a WorldModel, a Folder, ReplicatedStorage, no parent; parts entering after
-- their joint), and which joints place a cycle of welds whose C0 and C1 are written. Built 300 studs out with no gravity and
-- no collisions; each case's parts are destroyed before the next.
P.sync()
local Gravity = workspace.Gravity
workspace.Gravity = 0

-- I. several anchored parts, Glue, JointsService
do
	local JS = game:GetService("JointsService")
	local Spot = 0
	local function Corner()
		Spot += 1
		return CFrame.new(300 + 30 * Spot, 60, 40)
	end
	local Slot = { A = 1, B = 2, C = 3, D = 4, E = 5, U = 0, U1 = 6, U2 = 7, U3 = 8, U4 = 9, H = 0 }
	local function Part(Name, At, Anchored, Parent, Size)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Size = Size or Vector3.new(1, 1, 1)
		Made.Anchored = Anchored
		Made.CanCollide = false
		Made.CanTouch = false
		Made.CanQuery = false
		Made.CFrame = At * CFrame.new(2 * (Slot[Name] or tonumber(string.sub(Name, 2)) + 10), 0, 0)
		Made.Parent = Parent
		return Made
	end
	-- a joint that keeps both parts where they are; Parent "none" leaves it unparented
	local function Weld(A, B, Parent, Kind)
		local Made = Instance.new(Kind or "Weld")
		Made.Part0, Made.Part1 = A, B
		if Made:IsA("JointInstance") then
			Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
		end
		if Parent ~= "none" then
			Made.Parent = Parent or A
		end
		return Made
	end
	local function Root(Each)
		local R = Each.AssemblyRootPart
		return if R then R.Name else "nil"
	end
	local function Show(Label, Parts)
		local Out = {}
		for _, Each in Parts do
			table.insert(Out, Each.Name .. "=" .. Root(Each))
		end
		print(Label, table.concat(Out, " "))
	end
	local function Model(Label, Parent)
		local Made = Instance.new("Model")
		Made.Name = Label
		Made.Parent = Parent
		return Made
	end

	-- 1. an unanchored part welded to three anchored ones
	local function Fan(Label, Made, Order, Opts)
		Opts = Opts or {}
		local At = Corner()
		local M = Model(Label, if Opts.Outside then nil else workspace)
		local Parts = {}
		if not Opts.ULast then
			Parts.U = Part("U", At, Opts.Free == true, M)
			Parts.U.Anchored = false
		end
		for _, Name in Made do
			Parts[Name] = Part(Name, At, not Opts.Free, M)
		end
		if Opts.ULast then
			Parts.U = Part("U", At, false, M)
		end
		local List = { Parts.U, Parts.A, Parts.B, Parts.C }
		local Welds = {}
		for _, Name in Order do
			local W
			if Opts.Reverse then
				W = Weld(Parts[Name], Parts.U, Opts.WeldParent or Parts[Name], Opts.Kind)
			else
				W = Weld(Parts.U, Parts[Name], Opts.WeldParent or Parts.U, Opts.Kind)
			end
			Welds[Name] = W
			if not Opts.Quiet then
				Show(Label .. " +" .. Name, List)
			end
		end
		return M, Parts, Welds, List
	end
	do
		local M = Fan("fan-abc", { "A", "B", "C" }, { "A", "B", "C" })
		M:Destroy()
		M = Fan("fan-cba", { "A", "B", "C" }, { "C", "B", "A" })
		M:Destroy()
		M = Fan("fan-bac", { "A", "B", "C" }, { "B", "A", "C" })
		M:Destroy()
		M = Fan("fan-made-cba", { "C", "B", "A" }, { "A", "B", "C" })
		M:Destroy()
		M = Fan("fan-made-cba-weld-cba", { "C", "B", "A" }, { "C", "B", "A" })
		M:Destroy()
		M = Fan("fan-ulast", { "A", "B", "C" }, { "C", "B", "A" }, { ULast = true })
		M:Destroy()
		M = Fan("fan-reverse", { "A", "B", "C" }, { "C", "B", "A" }, { Reverse = true })
		M:Destroy()
		M = Fan("fan-wc", { "A", "B", "C" }, { "C", "B", "A" }, { Kind = "WeldConstraint" })
		M:Destroy()
		M = Fan("fan-motor6d", { "A", "B", "C" }, { "C", "B", "A" }, { Kind = "Motor6D" })
		M:Destroy()
	end
	-- welds made in one order, parented in another
	do
		local At = Corner()
		local M = Model("fan-parented", workspace)
		local U = Part("U", At, false, M)
		local A, B, C = Part("A", At, true, M), Part("B", At, true, M), Part("C", At, true, M)
		local WA, WB, WC = Weld(U, A, "none"), Weld(U, B, "none"), Weld(U, C, "none")
		WC.Parent = U
		Show("fan-parented +C", { U, A, B, C })
		WB.Parent = U
		Show("fan-parented +B", { U, A, B, C })
		WA.Parent = U
		Show("fan-parented +A", { U, A, B, C })
		-- the weld made first taken out and back
		WA.Parent = nil
		Show("fan-parented -A", { U, A, B, C })
		WA.Parent = U
		Show("fan-parented A-back", { U, A, B, C })
		-- disabled and enabled
		WC.Enabled = false
		Show("fan-parented C-off", { U, A, B, C })
		WC.Enabled = true
		Show("fan-parented C-on", { U, A, B, C })
		WB.Enabled = false
		Show("fan-parented B-off", { U, A, B, C })
		WB.Enabled = true
		Show("fan-parented B-on", { U, A, B, C })
		task.wait()
		Show("fan-parented frame", { U, A, B, C })
		-- re-pointed: A's weld to a fourth anchored part D
		local D = Part("D", At, true, M)
		WA.Part1 = D
		Show("fan-parented A-to-D", { U, A, B, C, D })
		WA.Part1 = A
		Show("fan-parented D-to-A", { U, A, B, C, D })
		M:Destroy()
	end
	-- the welds' Part1 set last, in an order of its own
	do
		local At = Corner()
		local M = Model("fan-part1", workspace)
		local U = Part("U", At, false, M)
		local A, B, C = Part("A", At, true, M), Part("B", At, true, M), Part("C", At, true, M)
		local Welds = {}
		for _, P in { A, B, C } do
			local W = Instance.new("Weld")
			W.Part0 = U
			W.C0 = U.CFrame:ToObjectSpace(P.CFrame)
			W.Parent = U
			Welds[P.Name] = W
		end
		for _, Name in { "B", "C", "A" } do
			Welds[Name].Part1 = M[Name]
			Show("fan-part1 +" .. Name, { U, A, B, C })
		end
		M:Destroy()
	end
	-- a whole model entering the Workspace at once
	for _, Order in { { "A", "B", "C" }, { "C", "B", "A" }, { "B", "C", "A" } } do
		local Label = "fan-model-" .. string.lower(table.concat(Order))
		local M, Parts, _, List = Fan(Label, { "A", "B", "C" }, Order, { Outside = true, Quiet = true })
		M.Parent = workspace
		Show(Label, List)
		M.Parent = nil
		M.Parent = workspace
		Show(Label .. " again", List)
		Parts.U.Parent = nil
		Parts.U.Parent = M
		Show(Label .. " u-again", List)
		M:Destroy()
	end
	-- the three free first, then anchored one by one, then freed
	do
		local M, Parts, _, List = Fan("fan-anchor", { "A", "B", "C" }, { "A", "B", "C" }, { Free = true, Quiet = true })
		Show("fan-anchor free", List)
		for _, Name in { "C", "A", "B" } do
			Parts[Name].Anchored = true
			Show("fan-anchor +" .. Name, List)
		end
		for _, Name in { "B", "C" } do
			Parts[Name].Anchored = false
			Show("fan-anchor -" .. Name, List)
		end
		M:Destroy()
	end

	-- 2. chains between two anchored parts
	local function Chain(Label, Names, Order)
		local At = Corner()
		local M = Model(Label, workspace)
		local Parts = {}
		for Index, Name in Names do
			Parts[Name] = Part(Name, At, Index == 1 or Index == #Names, M)
		end
		local List = {}
		for _, Name in Names do
			table.insert(List, Parts[Name])
		end
		for _, Pair in Order do
			Weld(Parts[Pair[1]], Parts[Pair[2]])
			Show(Label .. " " .. Pair[1] .. "-" .. Pair[2], List)
		end
		return M, Parts, List
	end
	do
		local M = Chain("chain", { "A", "U1", "U2", "B" }, { { "A", "U1" }, { "U1", "U2" }, { "U2", "B" } })
		M:Destroy()
		M = Chain("chain-rev", { "A", "U1", "U2", "B" }, { { "U2", "B" }, { "U1", "U2" }, { "A", "U1" } })
		M:Destroy()
		M = Chain("chain-mid", { "A", "U1", "U2", "B" }, { { "A", "U1" }, { "U2", "B" }, { "U1", "U2" } })
		M:Destroy()
		M = Chain("chain-mid2", { "A", "U1", "U2", "B" }, { { "U2", "B" }, { "A", "U1" }, { "U1", "U2" } })
		M:Destroy()
		M = Chain("chain-long", { "A", "U1", "U2", "U3", "U4", "B" }, { { "A", "U1" }, { "U1", "U2" }, { "U2", "U3" }, { "U3", "U4" }, { "U4", "B" } })
		M:Destroy()
		M = Chain("chain-long-rev", { "A", "U1", "U2", "U3", "U4", "B" }, { { "U4", "B" }, { "U3", "U4" }, { "U2", "U3" }, { "U1", "U2" }, { "A", "U1" } })
		M:Destroy()
	end
	-- a part welded straight to A and through U2 to B
	do
		local At = Corner()
		local M = Model("bridge", workspace)
		local A, B = Part("A", At, true, M), Part("B", At, true, M)
		local U1, U2 = Part("U1", At, false, M), Part("U2", At, false, M)
		Weld(U1, U2)
		Show("bridge u1-u2", { A, B, U1, U2 })
		Weld(U2, B)
		Show("bridge u2-b", { A, B, U1, U2 })
		Weld(U1, A)
		Show("bridge u1-a", { A, B, U1, U2 })
		M:Destroy()
	end

	-- 3. a free star: a hub and five leaves alike
	local function Star(Label, HubLast, Order)
		local At = Corner()
		local M = Model(Label, workspace)
		local Parts, List = {}, {}
		if not HubLast then
			Parts.H = Part("H", At, false, M)
		end
		for Index = 1, 5 do
			Parts["L" .. Index] = Part("L" .. Index, At, false, M)
		end
		if HubLast then
			Parts.H = Part("H", At, false, M)
		end
		List = { Parts.H, Parts.L1, Parts.L2, Parts.L3, Parts.L4, Parts.L5 }
		for _, Index in Order do
			Weld(Parts.H, Parts["L" .. Index])
		end
		Show(Label, List)
		return M, Parts, List
	end
	do
		local M, Parts, List = Star("star", false, { 1, 2, 3, 4, 5 })
		Parts.L5.Anchored = true
		Parts.L1.Anchored = true
		Show("star a5-a1", List)
		Parts.L5.Anchored = false
		Parts.L1.Anchored = false
		Show("star freed", List)
		Parts.L1.Anchored = true
		Parts.L5.Anchored = true
		Show("star a1-a5", List)
		Parts.L5.Anchored = false
		Show("star a1", List)
		Parts.L3.Anchored = true
		Show("star a1-a3", List)
		M:Destroy()
		M, Parts, List = Star("star-rev", false, { 5, 4, 3, 2, 1 })
		Parts.L5.Anchored = true
		Parts.L1.Anchored = true
		Show("star-rev a5-a1", List)
		Parts.L1.Anchored = false
		Parts.L5.Anchored = false
		Show("star-rev freed", List)
		M:Destroy()
		M, Parts, List = Star("star-mixed", false, { 2, 5, 1, 4, 3 })
		Parts.L5.Anchored = true
		Parts.L1.Anchored = true
		Parts.L3.Anchored = true
		Show("star-mixed a5-a1-a3", List)
		M:Destroy()
		M, Parts, List = Star("star-hublast", true, { 1, 2, 3, 4, 5 })
		M:Destroy()
		M, Parts, List = Star("star-hublast-rev", true, { 5, 4, 3, 2, 1 })
		M:Destroy()
	end
	-- two equal free parts made A, B and welded each way, then a third
	do
		local At = Corner()
		local M = Model("pair", workspace)
		local A, B, C = Part("A", At, false, M), Part("B", At, false, M), Part("C", At, false, M)
		Weld(B, A)
		Show("pair b-a", { A, B, C })
		Weld(C, A)
		Show("pair c-a", { A, B, C })
		M:Destroy()
	end

	-- 4. Glue
	local function Glued(Label, Kind, AAnchored, ASize, Offset)
		local At = Corner()
		local M = Model(Label, workspace)
		local A = Part("A", At, AAnchored, M, ASize)
		local B = Part("B", At, false, M)
		local G = Instance.new(Kind)
		G.Part0, G.Part1 = A, B
		G.C0 = Offset or A.CFrame:ToObjectSpace(B.CFrame)
		G.Parent = A
		return M, A, B, G
	end
	do
		for _, Kind in { "Glue", "ManualGlue", "Snap", "ManualWeld" } do
			local M, A, B, G = Glued("glue-" .. Kind, Kind, true)
			Show("glue " .. Kind .. " anchored", { A, B })
			G.Enabled = false
			Show("glue " .. Kind .. " off", { A, B })
			G.Enabled = true
			Show("glue " .. Kind .. " on", { A, B })
			A.Anchored = false
			Show("glue " .. Kind .. " freed", { A, B })
			M:Destroy()
			M, A, B, G = Glued("glue-" .. Kind, Kind, false, Vector3.new(2, 2, 2))
			Show("glue " .. Kind .. " free", { A, B })
			print("glue " .. Kind .. " mass", P.num(B.AssemblyMass), P.num(A.AssemblyMass))
			A.CFrame = A.CFrame * CFrame.new(0, 3, 0)
			print("glue " .. Kind .. " moved", P.f32s(B.Position.X, B.Position.Y, B.Position.Z))
			M:Destroy()
			-- a C0 that does not match where B is: does B move?
			M, A, B, G = Glued("glue-" .. Kind, Kind, true, nil, CFrame.new(0, 4, 0))
			print("glue " .. Kind .. " offset", P.f32s(B.Position.X - A.Position.X, B.Position.Y - A.Position.Y, B.Position.Z - A.Position.Z), Root(B))
			task.wait()
			print("glue " .. Kind .. " offset-frame", P.f32s(B.Position.X - A.Position.X, B.Position.Y - A.Position.Y, B.Position.Z - A.Position.Z), Root(B))
			M:Destroy()
		end
	end
	-- a part glued to one anchored part and welded to another
	do
		local At = Corner()
		local M = Model("glue-weld", workspace)
		local U = Part("U", At, false, M)
		local A, B = Part("A", At, true, M), Part("B", At, true, M)
		Weld(U, A)
		Show("glue-weld weld-a", { U, A, B })
		Weld(U, B, nil, "Glue")
		Show("glue-weld glue-b", { U, A, B })
		M:Destroy()
		M = Model("glue-weld2", workspace)
		U = Part("U", At, false, M)
		A, B = Part("A", At, true, M), Part("B", At, true, M)
		Weld(U, B, nil, "Glue")
		Show("glue-weld2 glue-b", { U, A, B })
		Weld(U, A)
		Show("glue-weld2 weld-a", { U, A, B })
		M:Destroy()
	end

	-- 5. JointsService
	local function Js(Label, Where, AAnchored, Kind)
		local At = Corner()
		local M = Model(Label, workspace)
		local A = Part("A", At, AAnchored ~= false, M)
		local B = Part("B", At, false, nil)
		local Parent = Where
		if Where == "folder" then
			Parent = Instance.new("Folder")
			Parent.Parent = M
		elseif Where == "a" then
			Parent = A
		elseif Where == "b" then
			Parent = B
		end
		local W = Weld(A, B, Parent, Kind)
		B.Parent = M
		Show(Label .. " entered", { A, B })
		return M, A, B, W
	end
	do
		for _, Where in { "js", "folder", "a", "b" } do
			local M, A, B, W = Js("enter-" .. Where, if Where == "js" then JS else Where)
			task.wait()
			Show("enter-" .. Where .. " frame", { A, B })
			W.Enabled = false
			W.Enabled = true
			Show("enter-" .. Where .. " toggled", { A, B })
			M:Destroy()
			W:Destroy()
		end
		local M, A, B, W = Js("enter-js-free", JS, false)
		M:Destroy()
		W:Destroy()
		M, A, B, W = Js("enter-js-reparent", JS)
		W.Parent = nil
		W.Parent = JS
		Show("enter-js-reparent back", { A, B })
		M:Destroy()
		W:Destroy()
		M, A, B, W = Js("enter-js-part1", JS)
		W.Part1 = nil
		W.Part1 = B
		Show("enter-js-part1 again", { A, B })
		M:Destroy()
		W:Destroy()
		M, A, B, W = Js("enter-js-c0", JS)
		W.C0 = W.C0
		W.C0 = W.C0 * CFrame.new(0, 0, 0)
		Show("enter-js-c0 written", { A, B })
		A.Anchored = false
		A.Anchored = true
		Show("enter-js-c0 reanchored", { A, B })
		M:Destroy()
		W:Destroy()
		M, A, B, W = Js("enter-js-wc", JS, true, "WeldConstraint")
		M:Destroy()
		W:Destroy()
	end
	-- both parts in the Workspace first, then the weld into JointsService; then a part out and back
	do
		local At = Corner()
		local M = Model("js-present", workspace)
		local A = Part("A", At, true, M)
		local B = Part("B", At, false, M)
		local W = Weld(A, B, JS)
		Show("js-present", { A, B })
		B.Parent = nil
		B.Parent = M
		Show("js-present b-back", { A, B })
		A.Parent = nil
		A.Parent = M
		Show("js-present a-back", { A, B })
		M.Parent = nil
		M.Parent = workspace
		Show("js-present model-back", { A, B })
		W:Destroy()
		M:Destroy()
	end
	-- the weld into JointsService while both parts are out, then the parts in
	do
		local At = Corner()
		local M = Model("js-out", nil)
		local A = Part("A", At, true, M)
		local B = Part("B", At, false, M)
		local W = Weld(A, B, JS)
		M.Parent = workspace
		Show("js-out model", { A, B })
		W:Destroy()
		M:Destroy()
		M = Model("js-out2", workspace)
		A = Part("A", At, true, nil)
		B = Part("B", At, false, nil)
		W = Weld(A, B, JS)
		A.Parent = M
		Show("js-out2 a-in", { A, B })
		B.Parent = M
		Show("js-out2 b-in", { A, B })
		W:Destroy()
		M:Destroy()
		-- a weld in a Folder of the Workspace, both parts in later
		M = Model("folder-out", workspace)
		local F = Instance.new("Folder")
		F.Parent = M
		A = Part("A", At, true, nil)
		B = Part("B", At, false, nil)
		W = Weld(A, B, F)
		A.Parent = M
		B.Parent = M
		Show("folder-out in", { A, B })
		M:Destroy()
	end
	-- JointsService welds into a WorldModel
	do
		local At = Corner()
		local World = Instance.new("WorldModel")
		World.Parent = workspace
		local A = Part("A", At, true, World)
		local B = Part("B", At, false, World)
		local W = Weld(A, B, JS)
		Show("js-worldmodel", { A, B })
		W:Destroy()
		World:Destroy()
	end
end

-- II. the order links are taken in, clones, cycles, where a joint links
do
	local JS = game:GetService("JointsService")
	local Storage = game:GetService("ReplicatedStorage")
	local Spot = 0
	local function Corner()
		Spot += 1
		return CFrame.new(300 + 30 * Spot, 60, 40)
	end
	local Slot = { A = 1, B = 2, C = 3, D = 4, U = 0, U1 = 6, U2 = 7, X = 0, Y = 8, Z = 9 }
	local function Part(Name, At, Anchored, Parent, Size)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Size = Size or Vector3.new(1, 1, 1)
		Made.Anchored = Anchored
		Made.CanCollide = false
		Made.CanTouch = false
		Made.CanQuery = false
		Made.CFrame = At * CFrame.new(2 * (Slot[Name] or tonumber(string.sub(Name, 2)) + 10), 0, 0)
		Made.Parent = Parent
		return Made
	end
	local function Weld(A, B, Parent, Kind)
		local Made = Instance.new(Kind or "Weld")
		Made.Part0, Made.Part1 = A, B
		if Made:IsA("JointInstance") then
			Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
		end
		if Parent ~= "none" then
			Made.Parent = Parent or A
		end
		return Made
	end
	local function Root(Each)
		local R = Each.AssemblyRootPart
		return if R then R.Name else "nil"
	end
	local function Show(Label, Parts)
		local Out = {}
		for _, Each in Parts do
			table.insert(Out, Each.Name .. "=" .. Root(Each))
		end
		print(Label, table.concat(Out, " "))
	end
	local function Model(Label, Parent)
		local Made = Instance.new("Model")
		Made.Name = Label
		Made.Parent = Parent
		return Made
	end
	local function Rel(Base, Each)
		local Offset = Base.CFrame:PointToObjectSpace(Each.Position)
		return Each.Name .. "@" .. P.f32s(Offset.X, Offset.Y, Offset.Z)
	end

	-- 1. the union's edges
	do
		-- joints of three kinds, made C, A, B
		local At = Corner()
		local M = Model("kinds", workspace)
		local U = Part("U", At, false, M)
		local A, B, C = Part("A", At, true, M), Part("B", At, true, M), Part("C", At, true, M)
		Weld(U, C, nil, "Motor6D")
		Weld(U, A, nil, "Weld")
		Weld(U, B, nil, "WeldConstraint")
		Show("kinds", { U, A, B, C })
		M:Destroy()
		-- two joints to A, the newest of all to A
		M = Model("twice", workspace)
		U = Part("U", At, false, M)
		A, B = Part("A", At, true, M), Part("B", At, true, M)
		local First = Weld(U, A)
		Weld(U, B)
		Show("twice b", { U, A, B })
		local Third = Weld(U, A)
		Show("twice a-again", { U, A, B })
		Third:Destroy()
		Show("twice a-again-gone", { U, A, B })
		First:Destroy()
		Show("twice a-first-gone", { U, A, B })
		M:Destroy()
		-- a bridge whose middle weld is the newest
		M = Model("bridge2", workspace)
		A, B = Part("A", At, true, M), Part("B", At, true, M)
		local U1, U2 = Part("U1", At, false, M), Part("U2", At, false, M)
		Weld(U1, A)
		Weld(U2, B)
		Show("bridge2 ends", { A, B, U1, U2 })
		Weld(U1, U2)
		Show("bridge2 middle", { A, B, U1, U2 })
		M:Destroy()
		-- two free parts each between A and B
		M = Model("ring", workspace)
		A, B = Part("A", At, true, M), Part("B", At, true, M)
		U1, U2 = Part("U1", At, false, M), Part("U2", At, false, M)
		Weld(A, U1)
		Weld(U2, B)
		Weld(U1, B)
		Weld(A, U2)
		Show("ring", { A, B, U1, U2 })
		M:Destroy()
		-- the newest joint's part unanchored
		M = Model("unroot", workspace)
		U = Part("U", At, false, M)
		A, B, C = Part("A", At, true, M), Part("B", At, true, M), Part("C", At, true, M)
		Weld(U, A)
		Weld(U, B)
		Weld(U, C)
		C.Anchored = false
		Show("unroot c", { U, A, B, C })
		B.Anchored = false
		Show("unroot b", { U, A, B, C })
		C.Anchored = true
		Show("unroot c-again", { U, A, B, C })
		M:Destroy()
	end
	-- clones: welds made A, B, C, each under its anchored part, the parts made U, C, B, A
	do
		local At = Corner()
		local M = Model("clone", nil)
		local U = Part("U", At, false, M)
		local C, B, A = Part("C", At, true, M), Part("B", At, true, M), Part("A", At, true, M)
		for _, Each in { A, B, C } do
			Weld(U, Each, Each)
		end
		M.Parent = workspace
		Show("clone original", { U, A, B, C })
		local Copy = M:Clone()
		M:Destroy()
		Copy.Parent = workspace
		Show("clone copy", { Copy.U, Copy.A, Copy.B, Copy.C })
		Copy:Destroy()
	end

	-- 2. a cycle of welds that disagree: where each part is put, with the order the welds were made in
	local function Cycle(Label, Order)
		local At = Corner()
		local M = Model(Label, workspace)
		local X = Part("X", At, false, M, Vector3.new(2, 2, 2))
		local Y = Part("Y", At, false, M)
		local Z = Part("Z", At, false, M)
		Y.CFrame = X.CFrame * CFrame.new(3, 0, 0)
		Z.CFrame = X.CFrame * CFrame.new(0, 3, 0)
		local Make = {
			xy = function()
				Weld(X, Y)
			end,
			yz = function()
				-- disagrees: puts Z 4 above Y
				local W = Instance.new("Weld")
				W.Part0, W.Part1 = Y, Z
				W.C0 = CFrame.new(0, 4, 0)
				W.Parent = Y
			end,
			xz = function()
				Weld(X, Z)
			end,
		}
		for _, Name in Order do
			Make[Name]()
			print(Label, "+" .. Name, Root(X), Rel(X, Y), Rel(X, Z))
		end
		task.wait()
		print(Label, "frame", Root(X), Rel(X, Y), Rel(X, Z))
		-- moved: the whole assembly follows its tree
		X.CFrame = X.CFrame * CFrame.new(0, 0, 5)
		print(Label, "moved", Rel(X, Y), Rel(X, Z))
		M:Destroy()
	end
	Cycle("cycle-xy-yz-xz", { "xy", "yz", "xz" })
	Cycle("cycle-xz-yz-xy", { "xz", "yz", "xy" })
	Cycle("cycle-yz-xy-xz", { "yz", "xy", "xz" })
	Cycle("cycle-xy-xz-yz", { "xy", "xz", "yz" })

	-- 3. where a joint must be to link two parts
	local function Where(Label, PartsIn, JointIn, Kind, Late)
		local At = Corner()
		local Holder = PartsIn
		local A = Part("A", At, true, if Late then nil else Holder)
		local B = Part("B", At, false, if Late then nil else Holder)
		local W = Weld(A, B, JointIn, Kind)
		if Late then
			A.Parent = Holder
			B.Parent = Holder
		end
		Show(Label, { A, B })
		if JointIn ~= JS then
			W.Parent = JS
			Show(Label .. " to-js", { A, B })
		end
		W:Destroy()
		A:Destroy()
		B:Destroy()
	end
	do
		local World = Instance.new("WorldModel")
		World.Parent = workspace
		local Away = Instance.new("WorldModel")
		Away.Parent = Storage
		local Folder = Instance.new("Folder")
		Folder.Parent = workspace
		for _, Kind in { "Weld", "WeldConstraint", "Motor6D", "Snap", "Glue" } do
			Where("where " .. Kind .. " ws-js", Folder, JS, Kind)
			Where("where " .. Kind .. " ws-js-late", Folder, JS, Kind, true)
			Where("where " .. Kind .. " ws-storage", Folder, Storage, Kind)
			Where("where " .. Kind .. " ws-world", Folder, World, Kind)
			Where("where " .. Kind .. " world-js", World, JS, Kind)
			Where("where " .. Kind .. " world-js-late", World, JS, Kind, true)
			Where("where " .. Kind .. " world-ws", World, Folder, Kind)
			Where("where " .. Kind .. " away-js", Away, JS, Kind)
			Where("where " .. Kind .. " ws-none", Folder, "none", Kind)
		end
		World:Destroy()
		Away:Destroy()
		Folder:Destroy()
	end
	-- a WeldConstraint in JointsService: toggled, its parts re-pointed, the parts moved out and back
	do
		local At = Corner()
		local M = Model("wcjs", workspace)
		local A = Part("A", At, true, M)
		local B = Part("B", At, false, M)
		local W = Weld(A, B, JS, "WeldConstraint")
		Show("wcjs", { A, B })
		W.Enabled = false
		W.Enabled = true
		Show("wcjs toggled", { A, B })
		W.Part1 = nil
		W.Part1 = B
		Show("wcjs repointed", { A, B })
		B.Parent = nil
		B.Parent = M
		Show("wcjs b-back", { A, B })
		W.Parent = A
		Show("wcjs under-a", { A, B })
		W.Parent = JS
		Show("wcjs back-js", { A, B })
		W:Destroy()
		M:Destroy()
	end
	-- many welds in JointsService from an entering part to anchored parts
	for _, N in { 1, 10, 200 } do
		local At = Corner()
		local M = Model("many", workspace)
		local Anchors = {}
		for Index = 1, N do
			local Q = Instance.new("Part")
			Q.Name = "Q" .. Index
			Q.Anchored = true
			Q.CanCollide, Q.CanTouch, Q.CanQuery = false, false, false
			Q.Size = Vector3.new(1, 1, 1)
			Q.CFrame = At * CFrame.new(0, 2 + Index, 0)
			Q.Parent = M
			Anchors[Index] = Q
		end
		local Pp = Part("U", At, false, nil)
		local Made = {}
		for Index = 1, N do
			Made[Index] = Weld(Pp, Anchors[Index], JS)
		end
		Pp.Parent = M
		print("many-js", N, Root(Pp))
		task.wait()
		print("many-js", N, "frame", Root(Pp))
		for _, W in Made do
			W:Destroy()
		end
		Pp:Destroy()
		M:Destroy()
	end
end

-- III. WeldConstraints, Motors and Motor6Ds among Welds; which joints place a cycle
do
	local Spot = 0
	local function Corner()
		Spot += 1
		return CFrame.new(300 + 30 * Spot, 60, 40)
	end
	local Slot = { A = 1, B = 2, C = 3, D = 4, U = 0, U1 = 6, U2 = 7, X = 0, Y = 8, Z = 9 }
	local function Part(Name, At, Anchored, Parent, Size)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Size = Size or Vector3.new(1, 1, 1)
		Made.Anchored = Anchored
		Made.CanCollide = false
		Made.CanTouch = false
		Made.CanQuery = false
		Made.CFrame = At * CFrame.new(2 * (Slot[Name] or tonumber(string.sub(Name, 2)) + 10), 0, 0)
		Made.Parent = Parent
		return Made
	end
	local function Weld(A, B, Parent, Kind)
		local Made = Instance.new(Kind or "Weld")
		Made.Part0, Made.Part1 = A, B
		if Made:IsA("JointInstance") then
			Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
		end
		if Parent ~= "none" then
			Made.Parent = Parent or A
		end
		return Made
	end
	local function Root(Each)
		local R = Each.AssemblyRootPart
		return if R then R.Name else "nil"
	end
	local function Show(Label, Parts)
		local Out = {}
		for _, Each in Parts do
			table.insert(Out, Each.Name .. "=" .. Root(Each))
		end
		print(Label, table.concat(Out, " "))
	end
	local function Model(Label, Parent)
		local Made = Instance.new("Model")
		Made.Name = Label
		Made.Parent = Parent
		return Made
	end
	local function Rel(Base, Each)
		local Offset = Base.CFrame:PointToObjectSpace(Each.Position)
		return Each.Name .. "@" .. P.f32s(Offset.X, Offset.Y, Offset.Z)
	end
	local function V(Vector)
		return P.f32s(Vector.X, Vector.Y, Vector.Z)
	end

	-- 1. WeldConstraints among JointInstances: an unanchored U joined to anchored parts by the kinds listed, made in that order
	local function Kinds(Label, List)
		local At = Corner()
		local M = Model(Label, workspace)
		local U = Part("U", At, false, M)
		local Parts = { U }
		for _, Entry in List do
			local Other = Part(Entry[1], At, true, M)
			table.insert(Parts, Other)
		end
		for Index, Entry in List do
			Weld(U, Parts[Index + 1], nil, Entry[2])
		end
		Show(Label, Parts)
		M:Destroy()
	end
	Kinds("kinds wc-weld", { { "A", "WeldConstraint" }, { "B", "Weld" } })
	Kinds("kinds weld-wc", { { "A", "Weld" }, { "B", "WeldConstraint" } })
	Kinds("kinds wc-wc-weld-wc", { { "A", "WeldConstraint" }, { "B", "WeldConstraint" }, { "C", "Weld" }, { "D", "WeldConstraint" } })
	Kinds("kinds weld-wc-wc", { { "A", "Weld" }, { "B", "WeldConstraint" }, { "C", "WeldConstraint" } })
	Kinds("kinds weld-motor", { { "A", "Weld" }, { "B", "Motor6D" } })
	Kinds("kinds motor-weld", { { "A", "Motor6D" }, { "B", "Weld" } })
	Kinds("kinds weld-snap-glue", { { "A", "Weld" }, { "B", "Snap" }, { "C", "Glue" } })
	Kinds("kinds glue-snap-weld", { { "A", "Glue" }, { "B", "Snap" }, { "C", "Weld" } })
	Kinds("kinds wc-motor-wc", { { "A", "WeldConstraint" }, { "B", "Motor6D" }, { "C", "WeldConstraint" } })
	-- a chain A -WC- U1 -Weld- U2 -Weld- B, the WeldConstraint made last
	do
		local At = Corner()
		local M = Model("wcchain", workspace)
		local A, U1, U2, B = Part("A", At, true, M), Part("U1", At, false, M), Part("U2", At, false, M), Part("B", At, true, M)
		Weld(U2, B)
		Weld(U1, U2)
		Weld(A, U1, nil, "WeldConstraint")
		Show("wcchain", { A, U1, U2, B })
		M:Destroy()
		M = Model("wcchain2", workspace)
		A, U1, U2, B = Part("A", At, true, M), Part("U1", At, false, M), Part("U2", At, false, M), Part("B", At, true, M)
		Weld(U2, B, nil, "WeldConstraint")
		Weld(U1, U2, nil, "WeldConstraint")
		Weld(A, U1)
		Show("wcchain2", { A, U1, U2, B })
		M:Destroy()
		-- WeldConstraint the oldest, a Weld across a WeldConstraint pair
		M = Model("wcchain3", workspace)
		A, U1, U2, B = Part("A", At, true, M), Part("U1", At, false, M), Part("U2", At, false, M), Part("B", At, true, M)
		Weld(U1, U2, nil, "WeldConstraint")
		Weld(A, U1, nil, "WeldConstraint")
		Weld(U2, B)
		Show("wcchain3", { A, U1, U2, B })
		M:Destroy()
	end

	-- 2. which joints place a cycle: three consistent welds, made in an order, then each one's C0 or C1 written in turn
	local function Cycle(Label, Order, Writes, XAnchored)
		local At = Corner()
		local M = Model(Label, workspace)
		local X = Part("X", At, XAnchored == true, M, Vector3.new(2, 2, 2))
		local Y = Part("Y", At, false, M)
		local Z = Part("Z", At, false, M)
		Y.CFrame = X.CFrame * CFrame.new(3, 0, 0)
		Z.CFrame = X.CFrame * CFrame.new(0, 3, 0)
		local Welds = {}
		local Pairs = { xy = { X, Y }, yz = { Y, Z }, xz = { X, Z }, zy = { Z, Y }, yx = { Y, X }, zx = { Z, X } }
		for _, Name in Order do
			Welds[Name] = Weld(Pairs[Name][1], Pairs[Name][2])
		end
		print(Label, "made", Root(X), Rel(X, Y), Rel(X, Z))
		for _, Name in Writes do
			local W = Welds[Name]
			W.C0 = W.C0 * CFrame.new(0, 0, 1)
			print(Label, "c0-" .. Name, Rel(X, Y), Rel(X, Z))
		end
		for _, Name in Writes do
			local W = Welds[Name]
			W.C1 = W.C1 * CFrame.new(0, 0.5, 0)
			print(Label, "c1-" .. Name, Rel(X, Y), Rel(X, Z))
		end
		task.wait()
		print(Label, "frame", Rel(X, Y), Rel(X, Z))
		M:Destroy()
	end
	Cycle("cyc-xy-yz-xz", { "xy", "yz", "xz" }, { "xy", "yz", "xz" })
	Cycle("cyc-xz-yz-xy", { "xz", "yz", "xy" }, { "xy", "yz", "xz" })
	Cycle("cyc-yz-xz-xy", { "yz", "xz", "xy" }, { "xy", "yz", "xz" })
	Cycle("cyc-xy-xz-yz", { "xy", "xz", "yz" }, { "xy", "yz", "xz" })
	Cycle("cyc-zy-yx-zx", { "zy", "yx", "zx" }, { "yx", "zy", "zx" })
	Cycle("cyc-anch-xy-yz-xz", { "xy", "yz", "xz" }, { "xy", "yz", "xz" }, true)
	Cycle("cyc-anch-xz-yz-xy", { "xz", "yz", "xy" }, { "xy", "yz", "xz" }, true)
end

-- IV. each pair of joint kinds
do
	local Spot = 0
	local function Corner()
		Spot += 1
		return CFrame.new(300 + 30 * Spot, 60, 40)
	end
	local Slot = { A = 1, B = 2, C = 3, D = 4, U = 0, X = 0, Y = 5, Z = 7 }
	local function Part(Name, At, Anchored, Parent, Size)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Size = Size or Vector3.new(1, 1, 1)
		Made.Anchored = Anchored
		Made.CanCollide = false
		Made.CanTouch = false
		Made.CanQuery = false
		Made.CFrame = At * CFrame.new(1.25 * (Slot[Name] or 0), 0.5 * (Slot[Name] or 0), 0)
		Made.Parent = Parent
		return Made
	end
	local function Weld(A, B, Parent, Kind)
		local Made = Instance.new(Kind or "Weld")
		Made.Part0, Made.Part1 = A, B
		if Made:IsA("JointInstance") then
			Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
		end
		if Parent ~= "none" then
			Made.Parent = Parent or A
		end
		return Made
	end
	local function Root(Each)
		local R = Each.AssemblyRootPart
		return if R then R.Name else "nil"
	end
	local function Show(Label, Parts)
		local Out = {}
		for _, Each in Parts do
			table.insert(Out, Each.Name .. "=" .. Root(Each))
		end
		print(Label, table.concat(Out, " "))
	end
	local function Model(Label, Parent)
		local Made = Instance.new("Model")
		Made.Name = Label
		Made.Parent = Parent
		return Made
	end
	local function V(Vector)
		return P.f32s(Vector.X, Vector.Y, Vector.Z)
	end

	-- 1. each kind's class: U joined to anchored A then B by the two kinds
	local Kinds = { "Weld", "ManualWeld", "Snap", "Glue", "ManualGlue", "Motor", "Motor6D", "WeldConstraint", "Rotate", "RotateP", "RotateV", "VelocityMotor" }
	for _, First in Kinds do
		for _, Second in Kinds do
			local At = Corner()
			local M = Model("k", workspace)
			local U = Part("U", At, false, M)
			local A, B = Part("A", At, true, M), Part("B", At, true, M)
			Weld(U, A, nil, First)
			Weld(U, B, nil, Second)
			Show("k " .. First .. "-" .. Second, { U })
			M:Destroy()
		end
	end
end

workspace.Gravity = Gravity
