-- CanQuery and CanCollide in the spatial queries (world/query.rs, world/broadphase.rs): which queries find a part with CanQuery
-- off while CanCollide is on, off with both off, and either order of writing them; RespectCanCollide; the Region3 queries,
-- GetPartsObscuringTarget, a WorldModel's queries and an Include filter naming such a part. The scene is 300 studs out.
P.sync()
local Root = Instance.new("Folder")
Root.Parent = workspace
local Cases = {
	{ "on-on", function(P) end },
	{ "Qoff", function(P) P.CanQuery = false end },
	{ "Coff,Qoff", function(P) P.CanCollide = false P.CanQuery = false end },
	{ "Qoff,Coff", function(P) P.CanQuery = false P.CanCollide = false end },
	{ "Coff", function(P) P.CanCollide = false end },
	{ "Coff,Qoff,Con", function(P) P.CanCollide = false P.CanQuery = false P.CanCollide = true end },
	{ "Qoff,Toff", function(P) P.CanQuery = false P.CanTouch = false end },
	{ "Coff,Qoff,unanch", function(P) P.CanCollide = false P.CanQuery = false end, true },
	{ "Qoff,unanch", function(P) P.CanQuery = false end, true },
}
local Parts = {}
for Index, Case in Cases do
	local Made = Instance.new("Part")
	Made.Name = Case[1]
	Made.Anchored = true
	Made.Size = Vector3.new(10, 1, 10)
	Made.Position = Vector3.new(300 + 40 * Index, 90, 300)
	Case[2](Made)
	Made.Parent = Root
	Parts[Index] = Made
end
-- unanchored ones held by a weld to an anchored stand far below
for Index, Case in Cases do
	if Case[3] then
		local Stand = Instance.new("Part")
		Stand.Anchored = true
		Stand.Size = Vector3.new(1, 1, 1)
		Stand.Position = Parts[Index].Position + Vector3.new(0, -40, 0)
		Stand.Parent = Root
		local Weld = Instance.new("WeldConstraint")
		Weld.Part0 = Stand
		Weld.Part1 = Parts[Index]
		Weld.Parent = Stand
		Parts[Index].Anchored = false
	end
end
local Caster = Instance.new("Part")
Caster.Anchored = true
Caster.Size = Vector3.new(1, 1, 1)
Caster.CanQuery = false
Caster.CanCollide = false
Caster.Position = Vector3.new(0, 200, 0)
Caster.Parent = Root
local Probe = Instance.new("Part")
Probe.Anchored = true
Probe.Size = Vector3.new(2, 2, 2)
Probe.CanQuery = false
Probe.CanCollide = false
Probe.Parent = Root
task.wait()

local function Name(Result)
	return if Result then Result.Instance.Name else "nil"
end
local function Mine(List, Part)
	for _, Item in List do
		if Item == Part then
			return "found"
		end
	end
	return "none"
end
for Index, Case in Cases do
	local Part = Parts[Index]
	local Top = Part.Position + Vector3.new(0, 10, 0)
	local Down = Vector3.new(0, -20, 0)
	local Respect = RaycastParams.new()
	Respect.RespectCanCollide = true
	local OverlapRespect = OverlapParams.new()
	OverlapRespect.RespectCanCollide = true
	Caster.CFrame = CFrame.new(Top)
	Probe.CFrame = CFrame.new(Part.Position)
	local Region = Region3.new(Part.Position - Vector3.new(1, 1, 1), Part.Position + Vector3.new(1, 1, 1))
	local Out = {
		"reads " .. tostring(Part.CanQuery) .. "/" .. tostring(Part.CanCollide) .. "/" .. tostring(Part.CanTouch),
		"ray " .. Name(workspace:Raycast(Top, Down)),
		"rayRespect " .. Name(workspace:Raycast(Top, Down, Respect)),
		"block " .. Name(workspace:Blockcast(CFrame.new(Top), Vector3.new(1, 1, 1), Down)),
		"sphere " .. Name(workspace:Spherecast(Top, 0.5, Down)),
		"shape " .. Name(workspace:Shapecast(Caster, Down)),
		"box " .. Mine(workspace:GetPartBoundsInBox(CFrame.new(Part.Position), Vector3.new(2, 2, 2)), Part),
		"boxRespect " .. Mine(workspace:GetPartBoundsInBox(CFrame.new(Part.Position), Vector3.new(2, 2, 2), OverlapRespect), Part),
		"radius " .. Mine(workspace:GetPartBoundsInRadius(Part.Position, 1), Part),
		"inpart " .. Mine(workspace:GetPartsInPart(Probe), Part),
		"fpor " .. tostring((workspace:FindPartOnRay(Ray.new(Top, Down)) or { Name = "nil" }).Name),
		"region " .. Mine(workspace:FindPartsInRegion3(Region, nil, 100), Part),
	}
	print(Case[1], table.concat(Out, " | "))
end
-- toggled later, without another write of CanQuery
local Later = Parts[2]
Later.CanCollide = false
task.wait()
print("Qoff then Coff later", Name(workspace:Raycast(Later.Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0))), Mine(workspace:GetPartBoundsInBox(CFrame.new(Later.Position), Vector3.new(2, 2, 2)), Later))
Later.CanCollide = true
task.wait()
print("and Con again", Name(workspace:Raycast(Later.Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0))), Mine(workspace:GetPartBoundsInBox(CFrame.new(Later.Position), Vector3.new(2, 2, 2)), Later))
Root:Destroy()
do
	P.sync()
	local Root = Instance.new("Folder")
	Root.Parent = workspace
	local function Part(Name, Position, Size, Query, Collide)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Anchored = true
		Made.Size = Size
		Made.Position = Position
		Made.CanQuery = Query
		Made.CanCollide = Collide
		Made.Parent = Root
		return Made
	end
	local QoffCon = Part("QoffCon", Vector3.new(300, 90, 300), Vector3.new(10, 1, 10), false, true)
	local QoffCoff = Part("QoffCoff", Vector3.new(400, 90, 300), Vector3.new(10, 1, 10), false, false)
	task.wait()
	local function Names(List)
		local Out = {}
		for _, Item in List do
			if Item:IsDescendantOf(Root) then
				table.insert(Out, Item.Name)
			end
		end
		table.sort(Out)
		return table.concat(Out, ",")
	end
	-- ArePartsTouchingOthers, GetTouchingParts
	local Toucher = Part("Toucher", Vector3.new(300, 91, 300), Vector3.new(1, 1, 1), true, true)
	local Toucher2 = Part("Toucher2", Vector3.new(400, 91, 300), Vector3.new(1, 1, 1), true, true)
	print("touching", workspace:ArePartsTouchingOthers({ Toucher }), workspace:ArePartsTouchingOthers({ Toucher2 }))
	-- GetPartsObscuringTarget with a camera
	local Camera = workspace.CurrentCamera
	if Camera then
		Camera.CameraType = Enum.CameraType.Scriptable
		Camera.CFrame = CFrame.lookAt(Vector3.new(300, 120, 300), Vector3.new(300, 60, 300))
		task.wait()
		print("obscuring", Names(Camera:GetPartsObscuringTarget({ Vector3.new(300, 60, 300) }, {})))
		Camera.CFrame = CFrame.lookAt(Vector3.new(400, 120, 300), Vector3.new(400, 60, 300))
		task.wait()
		print("obscuring off", Names(Camera:GetPartsObscuringTarget({ Vector3.new(400, 60, 300) }, {})))
	end
	-- a query from a Model's WorldModel
	local World = Instance.new("WorldModel")
	World.Parent = Root
	local Inside = Part("Inside", Vector3.new(0, 0, 0), Vector3.new(4, 1, 4), false, true)
	Inside.Parent = World
	task.wait()
	local Hit = World:Raycast(Vector3.new(0, 10, 0), Vector3.new(0, -20, 0))
	print("worldmodel ray", Hit and Hit.Instance.Name, Names(World:GetPartBoundsInBox(CFrame.new(), Vector3.new(2, 2, 2))))
	-- params lists naming the part
	local Params = RaycastParams.new()
	Params.FilterType = Enum.RaycastFilterType.Include
	Params.FilterDescendantsInstances = { QoffCon }
	Hit = workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -20, 0), Params)
	print("include ray", Hit and Hit.Instance.Name)
	Root:Destroy()
end
