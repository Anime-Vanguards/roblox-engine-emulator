-- The resource suite's recording (tools/resources/suite.mjs, workload "record"): a scene that keeps the recorder busy.
-- A grid of parts drifts and spins every frame, a ring of models orbits, and a part is made and another destroyed
-- every few frames, so the recording holds moves, rotations, new instances and removals.
return function(Context)
	local RunService = game:GetService("RunService")

	local Grid = {}
	for X = 1, 12 do
		for Z = 1, 12 do
			local Part = Instance.new("Part")
			Part.Name = `Tile{X}_{Z}`
			Part.Anchored = true
			Part.Size = Vector3.new(3, 1, 3)
			Part.Color = Color3.fromHSV(((X * 12 + Z) % 36) / 36, 0.6, 0.9)
			Part.CFrame = CFrame.new(X * 4, 0, Z * 4)
			Part.Parent = workspace
			table.insert(Grid, Part)
		end
	end

	local Ring = {}
	for Index = 1, 16 do
		local Model = Instance.new("Model")
		Model.Name = `Orbiter{Index}`
		local Body = Instance.new("Part")
		Body.Name = "Body"
		Body.Anchored = true
		Body.Shape = Enum.PartType.Ball
		Body.Size = Vector3.new(2, 2, 2)
		Body.Parent = Model
		local Fin = Instance.new("WedgePart")
		Fin.Name = "Fin"
		Fin.Anchored = true
		Fin.Size = Vector3.new(0.5, 1.5, 2)
		Fin.Parent = Model
		Model.PrimaryPart = Body
		Model.Parent = workspace
		table.insert(Ring, Model)
	end

	local Spawned = {}
	local Frame = 0
	RunService.Heartbeat:Connect(function()
		Frame += 1
		local Time = Frame / 60
		for Index, Part in Grid do
			Part.CFrame = CFrame.new(Part.Position.X, math.sin(Time * 2 + Index * 0.3) * 2, Part.Position.Z) * CFrame.Angles(0, Time + Index * 0.1, 0)
		end
		for Index, Model in Ring do
			local Angle = Time + Index * math.pi / 8
			Model:PivotTo(CFrame.new(24 + math.cos(Angle) * 30, 8, 24 + math.sin(Angle) * 30) * CFrame.Angles(0, -Angle, 0))
		end
		if Frame % 4 == 0 then
			local Part = Instance.new("Part")
			Part.Name = `Spark{Frame}`
			Part.Anchored = true
			Part.Size = Vector3.new(0.5, 0.5, 0.5)
			Part.Material = Enum.Material.Neon
			Part.CFrame = CFrame.new(24, 4 + (Frame % 20), 24)
			Part.Parent = workspace
			table.insert(Spawned, Part)
			if #Spawned > 20 then
				table.remove(Spawned, 1):Destroy()
			end
		end
	end)
end
