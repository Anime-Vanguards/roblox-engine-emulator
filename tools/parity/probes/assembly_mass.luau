-- An assembly's mass, centre of mass and velocities as each of its parts reads them (AssemblyRootPart, AssemblyMass,
-- GetMass, Mass, AssemblyCenterOfMass, AssemblyLinearVelocity, AssemblyAngularVelocity and the legacy Velocity and
-- RotVelocity): a lone part, an anchored part welded to unanchored ones, two anchored parts welded together with unanchored
-- ones hanging from each, Massless parts, in the Workspace, in a WorldModel and outside every world, just after an Anchored
-- or Massless write and a frame later, and after each kind of velocity write. Built 300 studs out with no gravity and no
-- collisions, so nothing moves between reads, and placed where the welds' offsets compose exactly (a client places a welded
-- part through its welds, ree where it was written).
P.sync()
local Gravity = workspace.Gravity
workspace.Gravity = 0
local Storage = game:GetService("ReplicatedStorage")
local Spot = 0
local function V(Vector)
	return P.f32s(Vector.X, Vector.Y, Vector.Z)
end
-- each case 20 studs from the last, its parts placed from a corner of its own
local function Corner()
	Spot += 1
	return CFrame.new(300 + 20 * Spot, 60, 40)
end
local function Part(Name, Parent, At, Size, Anchored, Massless)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Size = Size
	Made.Anchored = Anchored
	Made.Massless = Massless == true
	Made.CanCollide = false
	Made.CanTouch = false
	Made.CanQuery = false
	Made.CFrame = At
	Made.Parent = Parent
	return Made
end
-- a Weld that keeps both parts where they are
local function Weld(A, B)
	local Made = Instance.new("Weld")
	Made.Part0, Made.Part1 = A, B
	Made.C0 = A.CFrame:ToObjectSpace(B.CFrame)
	Made.Parent = A
	return Made
end
local function Show(Label, Parts)
	for _, Each in Parts do
		local Root = Each.AssemblyRootPart
		local Mass = Each.AssemblyMass
		local Centre = Each.AssemblyCenterOfMass
		print(
			Label,
			Each.Name,
			"root=" .. (if Root then Root.Name else "nil"),
			"mass=" .. P.num(Mass),
			"text=" .. tostring(Mass),
			"get=" .. P.num(Each:GetMass()),
			"own=" .. P.num(Each.Mass),
			"centre=" .. V(Centre),
			"centretext=" .. tostring(Centre),
			"linear=" .. V(Each.AssemblyLinearVelocity),
			"angular=" .. V(Each.AssemblyAngularVelocity),
			"velocity=" .. V(Each.Velocity),
			"rotvelocity=" .. V(Each.RotVelocity)
		)
	end
end
-- the reported case: an anchored Torso, an Arm welded beside it and a Hand welded below the Arm
local function Torso(Parent, Anchored)
	local At = Corner()
	local Model = Instance.new("Model")
	Model.Name = "Unit"
	local Body = Part("Torso", Model, At, Vector3.new(2, 2, 1), Anchored)
	local Arm = Part("Arm", Model, At * CFrame.new(1.5, 0, 0), Vector3.new(1, 2, 1), false)
	local Hand = Part("Hand", Arm, At * CFrame.new(1.5, -1.5, 0), Vector3.new(1, 1, 1), false)
	Weld(Body, Arm)
	Weld(Arm, Hand)
	Model.Parent = Parent
	return Model, { Body, Arm, Hand }
end

-- lone parts, anchored and not, then the other way round: read in the write's frame and a frame later
for _, Anchored in { true, false } do
	local Label = if Anchored then "lone-anchored" else "lone-free"
	local Lone = Part("Lone", workspace, Corner(), Vector3.new(2, 1, 4), Anchored)
	task.wait()
	Show(Label, { Lone })
	Lone.Anchored = not Anchored
	Show(Label .. " flipped", { Lone })
	task.wait()
	Show(Label .. " flipped-frame", { Lone })
	Lone:Destroy()
end

-- the reported case, then the Torso unanchored and anchored again, and the Hand anchored in the unanchored unit
do
	local Model, Parts = Torso(workspace, true)
	local Body, _, Hand = table.unpack(Parts)
	task.wait()
	Show("torso", Parts)
	Body.Anchored = false
	Show("torso unanchored", Parts)
	task.wait()
	Show("torso unanchored-frame", Parts)
	Hand.Anchored = true
	Show("torso hand-anchored", Parts)
	task.wait()
	Show("torso hand-anchored-frame", Parts)
	Hand.Anchored = false
	Body.Anchored = true
	Show("torso anchored", Parts)
	task.wait()
	Show("torso anchored-frame", Parts)
	Model:Destroy()
end

-- two anchored parts welded together, an unanchored one hanging from each: the anchored part that is not the root
do
	local At = Corner()
	local Model = Instance.new("Model")
	local A = Part("A", Model, At, Vector3.new(2, 2, 2), true)
	local B = Part("B", Model, At * CFrame.new(3, 0, 0), Vector3.new(4, 2, 2), true)
	local C = Part("C", Model, At * CFrame.new(0, 2, 0), Vector3.new(1, 2, 1), false)
	local D = Part("D", Model, At * CFrame.new(3, 2, 0), Vector3.new(1, 1, 3), false)
	Weld(A, B)
	Weld(A, C)
	Weld(B, D)
	Model.Parent = workspace
	task.wait()
	Show("two-anchored", { A, B, C, D })
	A.Anchored = false
	Show("two-anchored a-unanchored", { A, B, C, D })
	task.wait()
	Show("two-anchored a-unanchored-frame", { A, B, C, D })
	Model:Destroy()
end

-- Massless parts: alone, beside an unanchored part, all of an assembly, beside an anchored part, and anchored themselves
do
	local Cases = {
		{ "massless-lone-free", { { false, true } } },
		{ "massless-lone-anchored", { { true, true } } },
		{ "massless-leaf", { { false, false }, { false, true } } },
		{ "massless-all", { { false, true }, { false, true } } },
		{ "massless-anchored-root", { { true, false }, { false, true } } },
		{ "massless-anchored", { { true, true }, { false, false } } },
	}
	for _, Case in Cases do
		local At = Corner()
		local Model = Instance.new("Model")
		local Parts = {}
		for Index, Kind in Case[2] do
			local Made = Part("P" .. Index, Model, At * CFrame.new(2 * Index, 0, 0), Vector3.new(2, 1, 1 + Index), Kind[1], Kind[2])
			if Parts[1] then
				Weld(Parts[1], Made)
			end
			table.insert(Parts, Made)
		end
		Model.Parent = workspace
		task.wait()
		Show(Case[1], Parts)
		Model:Destroy()
	end
end

-- Massless parts in each world and outside them: a lone one, and two welded together, then the larger's partner anchored, the
-- larger made massful and the partner unanchored again, each read in the write's frame
do
	local World = Instance.new("WorldModel")
	World.Parent = Storage
	for _, Where in { { "massless-nil", nil }, { "massless-worldmodel", World }, { "massless-workspace", workspace } } do
		local Label, Parent = Where[1], Where[2]
		local Lone = Part("Lone", Parent, Corner(), Vector3.new(2, 1, 2), false, true)
		local A = Part("A", Parent, Corner(), Vector3.new(2, 1, 2), false, true)
		local B = Part("B", Parent, A.CFrame * CFrame.new(2, 0, 0), Vector3.new(2, 1, 3), false, true)
		Weld(A, B)
		task.wait()
		Show(Label, { Lone, A, B })
		A.Anchored = true
		Show(Label .. " a-anchored", { A, B })
		B.Massless = false
		Show(Label .. " b-massful", { A, B })
		A.Anchored = false
		Show(Label .. " a-unanchored", { A, B })
		Lone:Destroy()
		A:Destroy()
		B:Destroy()
	end
	World:Destroy()
end

-- other worlds: a WorldModel's parts, and parts outside every world (no parent, a Folder in ReplicatedStorage)
do
	local World = Instance.new("WorldModel")
	World.Parent = Storage
	local Folder = Instance.new("Folder")
	Folder.Parent = Storage
	for _, Where in { { "worldmodel", World }, { "nil", nil }, { "folder", Folder } } do
		local Label, Parent = Where[1], Where[2]
		for _, Anchored in { true, false } do
			local Model, Parts = Torso(Parent, Anchored)
			task.wait()
			local Case = Label .. (if Anchored then "-anchored" else "-free")
			Show(Case, Parts)
			Parts[1].Anchored = not Anchored
			Show(Case .. " flipped", Parts)
			task.wait()
			Show(Case .. " flipped-frame", Parts)
			Model:Destroy()
		end
	end
	World:Destroy()
	Folder:Destroy()
end

-- velocities written: to a lone anchored part (a conveyor's) and to each part of the anchored unit, read in the write's frame
-- and (for what is anchored) a frame later; to an unanchored unit, read in the write's frame
do
	local Lone = Part("Lone", workspace, Corner(), Vector3.new(2, 1, 4), true)
	task.wait()
	Lone.AssemblyLinearVelocity = Vector3.new(1, 2, 3)
	Lone.AssemblyAngularVelocity = Vector3.new(0, 0.5, 0)
	Show("velocity-lone", { Lone })
	task.wait()
	Show("velocity-lone frame", { Lone })
	Lone:Destroy()
	local Model, Parts = Torso(workspace, true)
	local Body, Arm = Parts[1], Parts[2]
	task.wait()
	Body.AssemblyLinearVelocity = Vector3.new(4, 0, 0)
	Show("velocity-torso root", Parts)
	Arm.AssemblyLinearVelocity = Vector3.new(0, 0, 2)
	Arm.AssemblyAngularVelocity = Vector3.new(0, 1, 0)
	Show("velocity-torso arm", Parts)
	task.wait()
	Show("velocity-torso frame", Parts)
	Model:Destroy()
	Model, Parts = Torso(workspace, false)
	task.wait()
	Parts[2].AssemblyLinearVelocity = Vector3.new(0, 0, 2)
	Show("velocity-free linear", Parts)
	Parts[3].AssemblyAngularVelocity = Vector3.new(0, 1, 0)
	Show("velocity-free angular", Parts)
	Model:Destroy()
end

-- each kind of velocity write in turn, to one part or another of the unit, unanchored and anchored, read in the write's frame:
-- the assembly's velocity, its spin, the legacy Velocity (each part's own) and RotVelocity
for _, Anchored in { false, true } do
	local Label = if Anchored then "writes-anchored" else "writes-free"
	local Model, Parts = Torso(workspace, Anchored)
	local Body, Arm, Hand = table.unpack(Parts)
	task.wait()
	Hand.AssemblyLinearVelocity = Vector3.new(1, 2, 3)
	Show(Label .. " hand-linear", Parts)
	Arm.AssemblyAngularVelocity = Vector3.new(0.5, -1, 2)
	Show(Label .. " arm-angular", Parts)
	Body.AssemblyLinearVelocity = Vector3.new(-3, 0.25, 7)
	Show(Label .. " torso-linear", Parts)
	Hand.AssemblyAngularVelocity = Vector3.new(1, 1, 1)
	Show(Label .. " hand-angular", Parts)
	Arm.Velocity = Vector3.new(0, 5, 0)
	Show(Label .. " arm-velocity", Parts)
	Hand.RotVelocity = Vector3.new(0, 0, 3)
	Show(Label .. " hand-rotvelocity", Parts)
	Model:Destroy()
end

-- two anchored parts welded together: a velocity written to what hangs from one and a spin to the other, then the first
-- unanchored (its assembly joins the other's)
do
	local At = Corner()
	local Model = Instance.new("Model")
	local A = Part("A", Model, At, Vector3.new(2, 2, 2), true)
	local B = Part("B", Model, At * CFrame.new(3, 0, 0), Vector3.new(4, 2, 2), true)
	local C = Part("C", Model, At * CFrame.new(0, 2, 0), Vector3.new(1, 2, 1), false)
	local D = Part("D", Model, At * CFrame.new(3, 2, 0), Vector3.new(1, 1, 3), false)
	Weld(A, B)
	Weld(A, C)
	Weld(B, D)
	Model.Parent = workspace
	task.wait()
	C.AssemblyLinearVelocity = Vector3.new(1, 0, 0)
	Show("velocity-two-anchored c-linear", { A, B, C, D })
	B.AssemblyAngularVelocity = Vector3.new(0, 2, 0)
	Show("velocity-two-anchored b-angular", { A, B, C, D })
	A.Anchored = false
	Show("velocity-two-anchored a-unanchored", { A, B, C, D })
	Model:Destroy()
end

-- how a client prints what is not a number
print("text", tostring(math.huge), tostring(-math.huge), tostring(0 / 0), tostring(Vector3.new(math.huge, -math.huge, 0 / 0)))
workspace.Gravity = Gravity
