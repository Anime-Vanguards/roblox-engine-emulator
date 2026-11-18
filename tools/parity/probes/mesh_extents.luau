-- AssetService:CreateMeshPartAsync and the size of the EditableMesh it is given (a playing client): a mesh whose box, over
-- every vertex (a face's or not), is wider than 2048 studs on any axis raises, wherever the box lies; 2048 itself passes.
-- A mesh that grows past the limit after its part is made leaves the part as it was.
local AssetService = game:GetService("AssetService")
local Options = { CollisionFidelity = Enum.CollisionFidelity.Box, RenderFidelity = Enum.RenderFidelity.Precise }
-- A mesh whose box runs from Low to Low + Size: a triangle at its centre, and each far corner a zero-area triangle or,
-- Loose, a vertex no face uses
local function mesh(Low, Size, Loose)
	local Mesh = AssetService:CreateEditableMesh()
	local Centre = Low + Size / 2
	Mesh:AddTriangle(Mesh:AddVertex(Centre + Vector3.new(-4, 0, -4)), Mesh:AddVertex(Centre + Vector3.new(0, 0, 4)), Mesh:AddVertex(Centre + Vector3.new(4, 0, -4)))
	for _, Corner in { Low, Low + Size } do
		if Loose then
			Mesh:AddVertex(Corner)
		else
			Mesh:AddTriangle(Mesh:AddVertex(Corner), Mesh:AddVertex(Corner), Mesh:AddVertex(Corner))
		end
	end
	return Mesh
end
local function part(Mesh)
	return pcall(function()
		return AssetService:CreateMeshPartAsync(Content.fromObject(Mesh), Options)
	end)
end
local function try(Label, Low, Size, Loose)
	local Mesh = mesh(Low, Size, Loose)
	local Ok, Result = part(Mesh)
	if Ok then
		print(Label .. " ok " .. P.f32s(Result.MeshSize.X, Result.MeshSize.Y, Result.MeshSize.Z))
		Result:Destroy()
	else
		print(Label .. " error " .. P.err(Result))
	end
	Mesh:Destroy()
	-- (a client frees a destroyed mesh's share of its EditableMesh budget later; without the wait the meshes run it out)
	task.wait()
end
local function centred(Label, Size, Loose)
	try(Label, -Size / 2, Size, Loose)
end

-- about the origin
centred("2112x520x2112", Vector3.new(2112, 520, 2112))
centred("2048x520x2048", Vector3.new(2048, 520, 2048))
-- one axis alone
for _, Axis in { "X", "Y", "Z" } do
	local Unit = Vector3.FromAxis(Enum.Axis[Axis])
	centred("2112 on " .. Axis, (Vector3.one - Unit) * 520 + Unit * 2112)
	centred("2048 on " .. Axis, (Vector3.one - Unit) * 520 + Unit * 2048)
end
-- just past 2048
centred("2049x520x520", Vector3.new(2049, 520, 520))
centred("2048.01x520x520", Vector3.new(2048.01, 520, 520))
-- off the origin: the box's width counts, not how far its vertices are
try("0 to 1500 on X", Vector3.new(0, -260, -260), Vector3.new(1500, 520, 520))
try("5000 to 5100 on X", Vector3.new(5000, -50, -50), Vector3.new(100, 100, 100))
try("0 to 2048 on X", Vector3.new(0, -260, -260), Vector3.new(2048, 520, 520))
try("0 to 2112 on X", Vector3.new(0, -260, -260), Vector3.new(2112, 520, 520))
-- the far corners as vertices no face uses
centred("2112x520x520 loose", Vector3.new(2112, 520, 520), true)

-- grown past the limit after its part is made
do
	local Mesh = mesh(Vector3.new(-1000, -260, -260), Vector3.new(2000, 520, 520))
	local Ok, Part = part(Mesh)
	if Ok then
		local Far = Mesh:AddVertex(Vector3.new(3000, 0, 0))
		print("grown add " .. P.try(function()
			return Mesh:AddTriangle(Far, Far, Far) ~= nil
		end))
		task.wait()
		print("grown part " .. P.f32s(Part.MeshSize.X, Part.MeshSize.Y, Part.MeshSize.Z) .. " " .. P.f32s(Part.Size.X, Part.Size.Y, Part.Size.Z))
		Part:Destroy()
	else
		print("grown error " .. P.err(Part))
	end
	Mesh:Destroy()
end
