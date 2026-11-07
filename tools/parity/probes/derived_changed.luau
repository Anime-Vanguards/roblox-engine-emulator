-- The Changed events a part fires beside a write: which derived properties (Position, Orientation, Rotation, the
-- assembly's mass and centre of mass, Mass, BrickColor and Color) fire for each kind of write, in what order, with a
-- part outside the DataModel, in the Workspace, anchored or not, and welded to another.
P.sync()
local Log = {}
local function Watch(Part, Label)
	local Connections = {}
	table.insert(Connections, Part.Changed:Connect(function(Name)
		table.insert(Log, Label .. ":" .. tostring(Name))
	end))
	return function()
		for _, Connection in Connections do
			Connection:Disconnect()
		end
	end
end
local function Case(Label, Part, Write, Other)
	local Stop = Watch(Part, "a")
	local StopOther = Other and Watch(Other, "b")
	local Ok, Message = pcall(Write, Part)
	task.wait()
	task.wait()
	Stop()
	if StopOther then
		StopOther()
	end
	print(Label .. " " .. (if Ok then "" else "E(" .. P.err(Message) .. ") ") .. table.concat(Log, ","))
	table.clear(Log)
end
local function Fresh(Parent, Anchored)
	local Part = Instance.new("Part")
	Part.Anchored = Anchored ~= false
	Part.Parent = Parent
	task.wait()
	return Part
end
for _, Where in { "loose", "world" } do
	local Parent = if Where == "world" then workspace else nil
	Case(Where .. " size", Fresh(Parent), function(Part) Part.Size = Vector3.new(4, 2, 2) end)
	Case(Where .. " size-same-mass", Fresh(Parent), function(Part) Part.Size = Vector3.new(2, 2.4, 2) end)
	Case(Where .. " position", Fresh(Parent), function(Part) Part.Position = Vector3.new(1, 2, 3) end)
	Case(Where .. " position-rotated", Fresh(Parent), function(Part)
		Part.CFrame = CFrame.Angles(0, 1, 0)
		task.wait()
		table.clear(Log)
		Part.Position = Vector3.new(1, 2, 3)
	end)
	Case(Where .. " cframe-translate", Fresh(Parent), function(Part) Part.CFrame = CFrame.new(0, 5, 0) end)
	Case(Where .. " cframe-rotate", Fresh(Parent), function(Part) Part.CFrame = CFrame.Angles(0, 1, 0) end)
	Case(Where .. " orientation", Fresh(Parent), function(Part) Part.Orientation = Vector3.new(0, 45, 0) end)
	Case(Where .. " rotation", Fresh(Parent), function(Part) Part.Rotation = Vector3.new(0, 45, 0) end)
	Case(Where .. " pivotto", Fresh(Parent), function(Part) Part:PivotTo(CFrame.new(0, 3, 0)) end)
	Case(Where .. " color", Fresh(Parent), function(Part) Part.Color = Color3.new(1, 0, 0) end)
	Case(Where .. " color-same-brick", Fresh(Parent), function(Part) Part.Color = Color3.fromRGB(163, 162, 166) end)
	Case(Where .. " brickcolor", Fresh(Parent), function(Part) Part.BrickColor = BrickColor.new("Bright blue") end)
	Case(Where .. " brickcolor-same", Fresh(Parent), function(Part) Part.BrickColor = Part.BrickColor end)
	Case(Where .. " material", Fresh(Parent), function(Part) Part.Material = Enum.Material.Wood end)
	Case(Where .. " material-same-density", Fresh(Parent), function(Part) Part.Material = Enum.Material.SmoothPlastic end)
	Case(Where .. " physical", Fresh(Parent), function(Part) Part.CustomPhysicalProperties = PhysicalProperties.new(2, 0.3, 0.5) end)
	Case(Where .. " massless", Fresh(Parent), function(Part) Part.Massless = true end)
	Case(Where .. " anchor-off", Fresh(Parent), function(Part) Part.Anchored = false end)
	Case(Where .. " anchor-on", Fresh(Parent, false), function(Part) Part.Anchored = true end)
	Case(Where .. " transparency", Fresh(Parent), function(Part) Part.Transparency = 0.5 end)
end
-- two parts welded together: does the other part hear the assembly change?
do
	local A, B = Fresh(workspace, false), Fresh(workspace, false)
	A.Anchored = true
	local Weld = Instance.new("WeldConstraint")
	Weld.Part0, Weld.Part1 = A, B
	Weld.Parent = A
	task.wait()
	task.wait()
	Case("welded size", B, function(Part) Part.Size = Vector3.new(4, 2, 2) end, A)
	Case("welded position", B, function(Part) Part.Position = Vector3.new(0, 10, 0) end, A)
	Case("welded massless", B, function(Part) Part.Massless = true end, A)
	A:Destroy()
	B:Destroy()
end
-- a Model's parts and a MeshPart
do
	local Mesh = Instance.new("MeshPart")
	Mesh.Anchored = true
	task.wait()
	Case("meshpart size", Mesh, function(Part) Part.Size = Vector3.new(2, 2, 2) end)
	local Wedge = Instance.new("WedgePart")
	Wedge.Anchored = true
	task.wait()
	Case("wedge size", Wedge, function(Part) Part.Size = Vector3.new(2, 2, 2) end)
end
for _, Child in workspace:GetChildren() do
	if Child:IsA("BasePart") and Child.Name == "Part" then
		Child:Destroy()
	end
end
