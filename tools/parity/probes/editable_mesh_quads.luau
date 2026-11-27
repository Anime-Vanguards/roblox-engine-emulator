-- EditableMesh faces of four vertices: AddFace, BatchAdd, Triangulate, what the per-face and per-vertex methods answer for them,
-- their automatic normals and splits, and ids of other kinds given as vertices (a playing client).
local AssetService = game:GetService("AssetService")
local function show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		if Value > 4294967296 then
			return string.format("%d:%d", Value // 4294967296, Value % 4294967296)
		end
		return P.num(Value)
	elseif Kind == "Vector3" then
		return "V3(" .. P.f32s(Value.X, Value.Y, Value.Z) .. ")"
	elseif Kind == "Vector2" then
		return "V2(" .. P.f32s(Value.X, Value.Y) .. ")"
	elseif Kind == "Color3" then
		return "C3(" .. P.f32s(Value.R, Value.G, Value.B) .. ")"
	elseif Kind == "table" then
		local Parts = {}
		for _, Item in Value do
			table.insert(Parts, show(Item))
		end
		return "{" .. table.concat(Parts, ",") .. "}"
	end
	return Kind .. ":" .. tostring(Value)
end
local Current
local function try(Label, Function)
	local Results = table.pack(pcall(Function))
	if Current then
		Current:Destroy()
		Current = nil
	end
	if Results[1] then
		local Out = {}
		for Index = 2, Results.n do
			table.insert(Out, show(Results[Index]))
		end
		print(Label .. " ok " .. table.concat(Out, " "))
	else
		print(Label .. " error " .. P.err(Results[2]))
	end
end
local function fresh(Positions)
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = {}
	for _, Position in Positions or { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1), Vector3.new(2, 2, 2) } do
		table.insert(V, Mesh:AddVertex(Position))
	end
	return Mesh, V
end
local function counts(Mesh)
	return #Mesh:GetVertices() .. "v " .. #Mesh:GetNormals() .. "n " .. #Mesh:GetUVs() .. "u " .. #Mesh:GetColors() .. "c " .. #Mesh:GetFaces() .. "f"
end
local function corners(Mesh, Face)
	return "v" .. show(Mesh:GetFaceVertices(Face)) .. " n" .. show(Mesh:GetFaceNormals(Face)) .. " u" .. show(Mesh:GetFaceUVs(Face)) .. " c" .. show(Mesh:GetFaceColors(Face))
end

-- AddFace of three and four vertices, and its argument checks
try("add-3", function()
	local Mesh, V = fresh()
	local F = Mesh:AddFace(V[1], V[2], V[3])
	return F, corners(Mesh, F), counts(Mesh)
end)
try("add-4", function()
	local Mesh, V = fresh()
	local F = Mesh:AddFace(V[1], V[2], V[3], V[4])
	return F, corners(Mesh, F), counts(Mesh)
end)
for _, Count in { 0, 1, 2, 5, 6 } do
	try("add-" .. Count, function()
		local Mesh, V = fresh()
		return Mesh:AddFace(table.unpack(V, 1, Count))
	end)
end
try("add-normal-id", function()
	local Mesh, V = fresh()
	return Mesh:AddFace(V[1], V[2], Mesh:AddNormal())
end)
try("add-gone", function()
	local Mesh, V = fresh()
	local F = Mesh:AddTriangle(V[1], V[2], V[6])
	Mesh:RemoveFace(F)
	Mesh:RemoveUnused()
	return Mesh:AddFace(V[1], V[2], V[6])
end)
try("add-zero", function()
	local Mesh, V = fresh()
	return Mesh:AddFace(V[1], V[2], 0)
end)
try("add-nil", function()
	local Mesh, V = fresh()
	return Mesh:AddFace(V[1], nil, V[3])
end)
try("add-string", function()
	local Mesh, V = fresh()
	return Mesh:AddFace(tostring(V[1]), V[2], V[3])
end)
try("add-text", function()
	local Mesh, V = fresh()
	return Mesh:AddFace("x", V[2], V[3])
end)
try("add-half", function()
	local Mesh, V = fresh()
	local F = Mesh:AddFace(V[1] + 0.5, V[2], V[3])
	return F, show(Mesh:GetFaceVertices(F))
end)
try("add-twice", function()
	local Mesh, V = fresh()
	local F = Mesh:AddFace(V[1], V[1], V[2], V[3])
	return F, corners(Mesh, F), counts(Mesh)
end)
try("add-table", function()
	local Mesh, V = fresh()
	return Mesh:AddFace({ V[1], V[2], V[3] })
end)
try("add-fixed", function()
	local Mesh, V = fresh()
	Mesh:AddTriangle(V[1], V[2], V[3])
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local W = Copy:GetVertices()
	local Ok, Message = pcall(Copy.AddFace, Copy, W[1], W[2], W[3])
	Copy:Destroy()
	return Ok, P.err(Message)
end)

-- a quad's corners against a triangle's shared vertices, and the vertex lists
try("quad-share", function()
	local Mesh, V = fresh()
	local T = Mesh:AddTriangle(V[1], V[2], V[5])
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	return corners(Mesh, T), corners(Mesh, Q), counts(Mesh)
end)
try("quad-lists", function()
	local Mesh, V = fresh()
	local T = Mesh:AddTriangle(V[1], V[3], V[5])
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local Out = {}
	for Index = 1, 5 do
		table.insert(Out, "v" .. Index .. "=f" .. show(Mesh:GetVertexFaces(V[Index])) .. "n" .. show(Mesh:GetVertexNormals(V[Index])) .. "a" .. show(Mesh:GetAdjacentVertices(V[Index])))
	end
	return table.concat(Out, " "), show(Mesh:GetAdjacentFaces(Q)), show(Mesh:GetAdjacentFaces(T))
end)
try("quad-normals", function()
	local Mesh, V = fresh({ Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(2, 1, 0.5), Vector3.new(0, 1.5, 0) })
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local Out = {}
	for _, N in Mesh:GetFaceNormals(Q) do
		table.insert(Out, show(Mesh:GetNormal(N)))
	end
	return table.concat(Out, " ")
end)
try("quad-shared-normal", function()
	local Mesh, V = fresh({ Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(2, 1, 0.5), Vector3.new(0, 1.5, 0), Vector3.new(1, -1, 1) })
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local T = Mesh:AddTriangle(V[2], V[1], V[5])
	local Out = {}
	for _, N in Mesh:GetNormals() do
		table.insert(Out, show(Mesh:GetNormal(N)))
	end
	return table.concat(Out, " "), corners(Mesh, T)
end)
try("quad-batch-add", function()
	local Mesh, V = fresh()
	local Ids = Mesh:BatchAdd(Enum.MeshAttribute.Face, { { V[1], V[2], V[3], V[4] }, { V[1], V[2], V[5] } })
	return Ids, corners(Mesh, Ids[1]), corners(Mesh, Ids[2]), counts(Mesh), show(Mesh:GetFaces())
end)
try("quad-batch-get", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local T = Mesh:AddTriangle(V[1], V[2], V[5])
	return show(Mesh:BatchGetFaceAttributes(Enum.MeshAttribute.Vertex, { Q, T })), show(Mesh:BatchGetFaceAttributes(Enum.MeshAttribute.Normal, { Q }))
end)
try("quad-vertex-face", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	return Mesh:GetVertexFaceNormal(V[4], Q), show(Mesh:BatchGetVertexFaceAttributes(Enum.MeshAttribute.UV, { V[4], V[3] }, { Q, Q })), show(Mesh:BatchGetVertexAttributes(Enum.MeshAttribute.Color, { V[4] }))
end)

-- setting a quad's corners, and a triangle's to four
try("set-quad-4", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	Mesh:SetFaceVertices(Q, { V[2], V[3], V[4], V[5] })
	return corners(Mesh, Q), counts(Mesh)
end)
try("set-quad-3", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	Mesh:SetFaceVertices(Q, { V[2], V[3], V[4] })
	return corners(Mesh, Q), counts(Mesh)
end)
try("set-tri-4", function()
	local Mesh, V = fresh()
	local T = Mesh:AddTriangle(V[1], V[2], V[3])
	Mesh:SetFaceVertices(T, { V[1], V[2], V[3], V[4] })
	return corners(Mesh, T), counts(Mesh)
end)
try("set-quad-normals-4", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local N = { Mesh:AddNormal(Vector3.xAxis), Mesh:AddNormal(Vector3.yAxis), Mesh:AddNormal(Vector3.zAxis), Mesh:AddNormal(-Vector3.xAxis) }
	Mesh:SetFaceNormals(Q, N)
	return corners(Mesh, Q)
end)
try("set-quad-normals-3", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local N = { Mesh:AddNormal(Vector3.xAxis), Mesh:AddNormal(Vector3.yAxis), Mesh:AddNormal(Vector3.zAxis) }
	Mesh:SetFaceNormals(Q, N)
	return corners(Mesh, Q)
end)
try("set-quad-batch", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local U = { Mesh:AddUV(Vector2.new(0, 0)), Mesh:AddUV(Vector2.new(1, 0)), Mesh:AddUV(Vector2.new(1, 1)), Mesh:AddUV(Vector2.new(0, 1)) }
	Mesh:BatchSetFaceAttributes({ Q }, { U })
	return corners(Mesh, Q)
end)

-- Triangulate
try("triangulate", function()
	local Mesh, V = fresh()
	local T = Mesh:AddTriangle(V[1], V[2], V[5])
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local Q2 = Mesh:AddFace(V[2], V[3], V[6], V[5])
	Mesh:Triangulate()
	local Out = {}
	for _, F in Mesh:GetFaces() do
		table.insert(Out, show(F) .. "=" .. corners(Mesh, F))
	end
	return T, Q, Q2, counts(Mesh), table.concat(Out, " ")
end)
try("triangulate-none", function()
	local Mesh, V = fresh()
	Mesh:AddTriangle(V[1], V[2], V[5])
	Mesh:Triangulate()
	return counts(Mesh), show(Mesh:GetFaces())
end)
try("triangulate-diagonal", function()
	-- a quad whose 1-3 diagonal is longer than its 2-4 one
	local Mesh, V = fresh({ Vector3.new(0, 0, 0), Vector3.new(3, 0, 0), Vector3.new(4, 1, 0), Vector3.new(0, 0.5, 0) })
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	Mesh:Triangulate()
	local Out = {}
	for _, F in Mesh:GetFaces() do
		table.insert(Out, show(Mesh:GetFaceVertices(F)))
	end
	return Q, table.concat(Out, " ")
end)
try("triangulate-fixed", function()
	local Mesh, V = fresh()
	Mesh:AddFace(V[1], V[2], V[3], V[4])
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Ok, Message = pcall(Copy.Triangulate, Copy)
	local Faces = #Copy:GetFaces()
	Copy:Destroy()
	return Ok, if Ok then Faces else P.err(Message)
end)

-- removing quads, and what shows them
try("quad-remove", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	Mesh:AddTriangle(V[1], V[2], V[5])
	Mesh:RemoveFace(Q)
	return counts(Mesh), show(Mesh:RemoveUnused()), counts(Mesh)
end)
try("quad-batch-remove", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local T = Mesh:AddTriangle(V[1], V[2], V[5])
	Mesh:BatchRemove({ Q })
	return counts(Mesh), show(Mesh:GetFaces()), T
end)
try("quad-part", function()
	local Mesh, V = fresh()
	Mesh:AddFace(V[1], V[2], V[3], V[4])
	local Part = AssetService:CreateMeshPartAsync(Content.fromObject(Mesh))
	return Part.MeshSize, Part.Size, Part.HasSkinnedMesh
end)
try("quad-size", function()
	local Mesh, V = fresh()
	Mesh:AddFace(V[1], V[2], V[3], V[4])
	return Mesh:GetSize(), Mesh:GetCenter()
end)
try("quad-copy", function()
	local Mesh, V = fresh()
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = false })
	local Faces = Copy:GetFaces()
	local Out = {}
	for _, F in Faces do
		table.insert(Out, show(F) .. "=" .. show(Copy:GetFaceVertices(F)))
	end
	Copy:Destroy()
	return Q, table.concat(Out, " ")
end)
try("quad-data-model", function()
	local Mesh, V = fresh()
	Mesh:AddFace(V[1], V[2], V[3], V[4])
	local _, Made = AssetService:CreateDataModelContentAsync(Content.fromObject(Mesh))
	local Copy = AssetService:CreateEditableMeshAsync(Made, { FixedSize = false })
	local Out = {}
	for _, F in Copy:GetFaces() do
		table.insert(Out, show(F) .. "=" .. show(Copy:GetFaceVertices(F)))
	end
	local Line = counts(Copy) .. " " .. table.concat(Out, " ")
	Copy:Destroy()
	return Line
end)

local AssetService = game:GetService("AssetService")
local function show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		if Value > 4294967296 then
			return string.format("%d:%d", Value // 4294967296, Value % 4294967296)
		end
		return P.num(Value)
	elseif Kind == "Vector3" then
		return "V3(" .. P.f32s(Value.X, Value.Y, Value.Z) .. ")"
	elseif Kind == "CFrame" then
		return "CF(" .. P.f32s(Value:GetComponents()) .. ")"
	elseif Kind == "EnumItem" then
		return tostring(Value)
	elseif Kind == "table" then
		if Value.n then
			local Parts = {}
			for Index = 1, Value.n do
				table.insert(Parts, show(Value[Index]))
			end
			return "(" .. table.concat(Parts, ",") .. ")"
		end
		local Keys = {}
		for Key in Value do
			table.insert(Keys, Key)
		end
		local Parts = {}
		if #Value == #Keys then
			for _, Item in ipairs(Value) do
				table.insert(Parts, show(Item))
			end
		else
			table.sort(Keys, function(A, B)
				return tostring(A) < tostring(B)
			end)
			for _, Key in Keys do
				table.insert(Parts, show(Key) .. "=" .. show(Value[Key]))
			end
		end
		return "{" .. table.concat(Parts, ",") .. "}"
	end
	return Kind .. ":" .. tostring(Value)
end
local Current
local function try(Label, Function)
	local Results = table.pack(pcall(Function))
	if Current then
		Current:Destroy()
		Current = nil
	end
	if Results[1] then
		local Out = { "n=" .. (Results.n - 1) }
		for Index = 2, Results.n do
			table.insert(Out, show(Results[Index]))
		end
		print(Label .. " ok " .. table.concat(Out, " "))
	else
		print(Label .. " error " .. P.err(Results[2]))
	end
end
local function mesh(Positions)
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = {}
	for _, Position in Positions do
		table.insert(V, Mesh:AddVertex(Position))
	end
	return Mesh, V
end
local function corners(Mesh, Face)
	return "v" .. show(Mesh:GetFaceVertices(Face)) .. " n" .. show(Mesh:GetFaceNormals(Face)) .. " u" .. show(Mesh:GetFaceUVs(Face)) .. " c" .. show(Mesh:GetFaceColors(Face))
end
local Square = { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1), Vector3.new(2, 2, 2) }
local function caught(Function, ...)
	local Ok, Message = pcall(Function, ...)
	return if Ok then "ok" else P.err(Message)
end

-- AddFace and AddTriangle given an id of another kind
try("addface-normal", function()
	local Mesh, V = mesh(Square)
	local F = Mesh:AddFace(V[1], V[2], Mesh:AddNormal())
	return F, show(Mesh:GetFaceVertices(F))
end)
try("addface-face", function()
	local Mesh, V = mesh(Square)
	local T = Mesh:AddTriangle(V[1], V[2], V[3])
	local F = Mesh:AddFace(V[4], V[5], T)
	return F, show(Mesh:GetFaceVertices(F))
end)
try("addface-gone-kind", function()
	local Mesh, V = mesh(Square)
	return Mesh:AddFace(V[1], V[2], 3 * 4294967296 + 9)
end)
try("addtriangle-normal", function()
	local Mesh, V = mesh(Square)
	return Mesh:AddTriangle(V[1], V[2], Mesh:AddNormal())
end)
try("addtriangle-text", function()
	local Mesh, V = mesh(Square)
	return Mesh:AddTriangle(tostring(V[1]), V[2], V[3])
end)
try("addface-value", function()
	local Mesh, V = mesh(Square)
	local Method = Mesh.AddFace
	return Method(Mesh, V[1], nil, V[3])
end)

-- a triangle given four vertices where the fourth has corners elsewhere; the setters' texts
try("set-tri-4-shared", function()
	local Mesh, V = mesh(Square)
	local A = Mesh:AddTriangle(V[4], V[5], V[6])
	local T = Mesh:AddTriangle(V[1], V[2], V[3])
	Mesh:SetFaceVertices(T, { V[1], V[2], V[3], V[4] })
	return corners(Mesh, A), corners(Mesh, T), #Mesh:GetNormals()
end)
try("set-quad-3-shared", function()
	local Mesh, V = mesh(Square)
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	Mesh:SetFaceVertices(Q, { V[5], V[2], V[6] })
	return corners(Mesh, Q), #Mesh:GetNormals()
end)
for _, Case in {
	{ "sfv-2", "SetFaceVertices", 2 },
	{ "sfv-5", "SetFaceVertices", 5 },
	{ "sfv-normal", "SetFaceVertices", "normal" },
	{ "sfv-gone", "SetFaceVertices", "gone" },
	{ "sfv-text", "SetFaceVertices", "text" },
	{ "sfv-numeric", "SetFaceVertices", "numeric" },
	{ "sfv-number", "SetFaceVertices", "number" },
	{ "sfv-zero", "SetFaceVertices", "zero" },
	{ "sfn-2", "SetFaceNormals", 2 },
	{ "sfn-4", "SetFaceNormals", 4 },
	{ "sfn-uv", "SetFaceNormals", "uv" },
	{ "sfn-gone", "SetFaceNormals", "gone" },
	{ "sfn-text", "SetFaceNormals", "text" },
	{ "sfn-number", "SetFaceNormals", "number" },
	{ "sfu-2", "SetFaceUVs", 2 },
	{ "sfc-4", "SetFaceColors", 4 },
} do
	try(Case[1], function()
		local Mesh, V = mesh(Square)
		local T = Mesh:AddTriangle(V[1], V[2], V[3])
		local Kind = string.match(Case[2], "^SetFace(%a+)s$")
		local Make = {
			Vertice = function(I)
				return V[I]
			end,
			Normal = function()
				return Mesh:AddNormal(Vector3.xAxis)
			end,
			UV = function()
				return Mesh:AddUV(Vector2.zero)
			end,
			Color = function()
				return Mesh:AddColor(Color3.new(1, 0, 0), 1)
			end,
		}
		local Ids = {}
		local Count = if type(Case[3]) == "number" then Case[3] else 3
		for Index = 1, Count do
			table.insert(Ids, Make[Kind](Index + 1))
		end
		if Case[3] == "normal" then
			Ids[2] = Mesh:AddNormal(Vector3.xAxis)
		elseif Case[3] == "uv" then
			Ids[2] = Mesh:AddUV(Vector2.zero)
		elseif Case[3] == "gone" then
			Mesh:RemoveUnused()
		elseif Case[3] == "text" then
			Ids[2] = "x"
		elseif Case[3] == "numeric" then
			Ids[2] = tostring(Ids[2])
		elseif Case[3] == "zero" then
			Ids[2] = 0
		end
		local Arg = if Case[3] == "number" then 5 else Ids
		local Result = caught((Mesh :: any)[Case[2]], Mesh, T, Arg)
		return Result, corners(Mesh, T)
	end)
end
try("quad-adjacent", function()
	local Mesh, V = mesh(Square)
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local T = Mesh:AddTriangle(V[2], V[1], V[5])
	local U = Mesh:AddTriangle(V[3], V[1], V[6])
	return show(Mesh:GetAdjacentFaces(Q)), show(Mesh:GetAdjacentFaces(T)), show(Mesh:GetAdjacentFaces(U))
end)
try("quad-vertex-face-set", function()
	local Mesh, V = mesh(Square)
	local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
	local N = Mesh:AddNormal(Vector3.yAxis)
	Mesh:BatchSetVertexFaceAttributes({ V[4] }, { Q }, { N })
	Mesh:SetVertexFaceUV(V[3], Q, Mesh:AddUV(Vector2.one))
	return corners(Mesh, Q), show(Mesh:GetVertexNormals(V[4]))
end)

-- automatic normals of quads (float bits)
for _, Case in {
	{ "auto-a", { Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(2, 1, 0.5), Vector3.new(0, 1.5, 0) } },
	{ "auto-b", { Vector3.new(0.1, -0.3, 0.7), Vector3.new(1.9, 0.2, -0.4), Vector3.new(2.3, 1.7, 0.9), Vector3.new(-0.2, 1.1, 0.3) } },
	{ "auto-c", { Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(-1, 0, 0.25), Vector3.new(0, -1, 0) } },
	{ "auto-d", { Vector3.new(0, 0, 0), Vector3.new(3, 0, 0), Vector3.new(3, 0.1, 2), Vector3.new(0, 0, 2) } },
} do
	try(Case[1], function()
		local Mesh, V = mesh(Case[2])
		local Q = Mesh:AddFace(V[1], V[2], V[3], V[4])
		local Out = {}
		for _, N in Mesh:GetFaceNormals(Q) do
			table.insert(Out, show(Mesh:GetNormal(N)))
		end
		return table.concat(Out, " ")
	end)
end
try("auto-shared", function()
	local Mesh, V = mesh({ Vector3.new(0.1, -0.3, 0.7), Vector3.new(1.9, 0.2, -0.4), Vector3.new(2.3, 1.7, 0.9), Vector3.new(-0.2, 1.1, 0.3), Vector3.new(1.2, -1.4, 0.6), Vector3.new(2.8, 0.4, 1.3) })
	Mesh:AddFace(V[1], V[2], V[3], V[4])
	Mesh:AddTriangle(V[2], V[1], V[5])
	Mesh:AddFace(V[2], V[6], V[3], V[2])
	local Out = {}
	for _, N in Mesh:GetNormals() do
		table.insert(Out, show(N) .. "=" .. show(Mesh:GetNormal(N)))
	end
	return table.concat(Out, " ")
end)
try("auto-triangulated", function()
	local Mesh, V = mesh({ Vector3.new(0.1, -0.3, 0.7), Vector3.new(1.9, 0.2, -0.4), Vector3.new(2.3, 1.7, 0.9), Vector3.new(-0.2, 1.1, 0.3) })
	Mesh:AddFace(V[1], V[2], V[3], V[4])
	Mesh:Triangulate()
	local Out = {}
	for _, N in Mesh:GetNormals() do
		table.insert(Out, show(Mesh:GetNormal(N)))
	end
	return table.concat(Out, " ")
end)
try("snapshot-diagonal", function()
	local Mesh, V = mesh({ Vector3.new(0, 0, 0), Vector3.new(3, 0, 0), Vector3.new(4, 1, 0), Vector3.new(0, 0.5, 0) })
	Mesh:AddFace(V[1], V[2], V[3], V[4])
	local _, Made = AssetService:CreateDataModelContentAsync(Content.fromObject(Mesh))
	local Copy = AssetService:CreateEditableMeshAsync(Made, { FixedSize = false })
	local Out = {}
	for _, F in Copy:GetFaces() do
		local Positions = {}
		for _, W in Copy:GetFaceVertices(F) do
			table.insert(Positions, show(Copy:GetPosition(W)))
		end
		table.insert(Out, table.concat(Positions, ","))
	end
	Copy:Destroy()
	return table.concat(Out, " ")
end)

local function show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		if Value > 4294967296 then
			return string.format("%d:%d", Value // 4294967296, Value % 4294967296)
		end
		return P.num(Value)
	elseif Kind == "Vector3" then
		return "V3(" .. P.f32s(Value.X, Value.Y, Value.Z) .. ")"
	elseif Kind == "table" then
		local Keys = {}
		for Key in Value do
			table.insert(Keys, Key)
		end
		local Parts = {}
		if #Value == #Keys then
			for _, Item in ipairs(Value) do
				table.insert(Parts, show(Item))
			end
		else
			table.sort(Keys, function(A, B)
				return tostring(A) < tostring(B)
			end)
			for _, Key in Keys do
				table.insert(Parts, show(Key) .. "=" .. show(Value[Key]))
			end
		end
		return "{" .. table.concat(Parts, ",") .. "}"
	end
	return Kind .. ":" .. tostring(Value)
end
local Current
local function try(Label, Function)
	local Results = table.pack(pcall(Function))
	if Current then
		Current:Destroy()
		Current = nil
	end
	if Results[1] then
		local Out = { "n=" .. (Results.n - 1) }
		for Index = 2, Results.n do
			table.insert(Out, show(Results[Index]))
		end
		print(Label .. " ok " .. table.concat(Out, " "))
	else
		print(Label .. " error " .. P.err(Results[2]))
	end
end
local function mesh(Positions)
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = {}
	for _, Position in Positions do
		table.insert(V, Mesh:AddVertex(Position))
	end
	return Mesh, V
end
local function caught(Function, ...)
	local Ok, Message = pcall(Function, ...)
	return if Ok then "ok" else P.err(Message)
end
local Square = { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1), Vector3.new(2, 2, 2) }

-- an id of another kind given as a vertex
local W = 4294967296
for _, Case in {
	{ "color", 3 * W + 1 }, { "uv", 4 * W + 1 }, { "bone", 6 * W + 1 }, { "kind0", 1 }, { "kind7", 7 * W + 1 }, { "vertex-gen1", 9 * W + 1 },
	{ "normal-gen1", 10 * W + 1 }, { "face", 5 * W + 1 }, { "normal-slot2", 2 * W + 2 }, { "color-slot9", 3 * W + 9 }, { "kind0-slot0", 0 },
} do
	try("kind-" .. Case[1], function()
		local Mesh, V = mesh(Square)
		local F = Mesh:AddTriangle(V[4], V[5], Case[2])
		return F, show(Mesh:GetFaceVertices(F))
	end)
	try("kind-face-" .. Case[1], function()
		local Mesh, V = mesh(Square)
		local F = Mesh:AddFace(V[4], V[5], Case[2])
		return F, show(Mesh:GetFaceVertices(F))
	end)
end
try("kind-batch", function()
	local Mesh, V = mesh(Square)
	return caught(Mesh.BatchAdd, Mesh, Enum.MeshAttribute.Face, { { V[4], V[5], 2 * W + 1 } })
end)
try("kind-set-face-vertices", function()
	local Mesh, V = mesh(Square)
	local F = Mesh:AddTriangle(V[1], V[2], V[3])
	return caught(Mesh.SetFaceVertices, Mesh, F, { V[4], V[5], 2 * W + 1 }), show(Mesh:GetFaceVertices(F))
end)

-- AddFace's check order
try("addface-count-text", function()
	local Mesh, V = mesh(Square)
	return caught(Mesh.AddFace, Mesh, V[1], "x"), caught(Mesh.AddFace, Mesh, V[1], V[2]), caught(Mesh.AddFace, Mesh, V[1], V[2], nil),
		caught(Mesh.AddFace, Mesh, V[1], V[2], V[3], nil), caught(Mesh.AddFace, Mesh, { V[1] }, V[2], V[3]), caught(Mesh.AddFace, Mesh, V[1], { V[2] }, V[3])
end)
try("addface-fixed-order", function()
	local Mesh, V = mesh(Square)
	Mesh:AddTriangle(V[1], V[2], V[3])
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Line = { caught(Copy.AddFace, Copy, V[1], "x", V[3]), caught(Copy.AddFace, Copy, V[1], V[2]), caught(Copy.AddFace, Copy, { V[1] }) }
	Copy:Destroy()
	return table.concat(Line, " | ")
end)

-- a normal several faces share: triangles and quads in turn (the sum's order)
local Fan = { Vector3.new(0, 0, 0), Vector3.new(1.3, 0.2, 0.1), Vector3.new(0.9, 1.1, -0.3), Vector3.new(-0.4, 1.2, 0.2), Vector3.new(-1.1, 0.3, 0.4), Vector3.new(-0.7, -0.9, -0.2), Vector3.new(0.5, -1.2, 0.3), Vector3.new(1.4, -0.6, -0.1), Vector3.new(0.2, 2.1, 0.5) }
for _, Case in {
	{ "fan-tqt", { { 1, 2, 3 }, { 1, 3, 9, 4 }, { 1, 4, 5 } } },
	{ "fan-qtq", { { 1, 2, 3, 9 }, { 1, 9, 4 }, { 1, 4, 5, 6 } } },
	{ "fan-ttqq", { { 1, 2, 3 }, { 1, 3, 4 }, { 1, 4, 5, 6 }, { 1, 6, 7, 8 } } },
	{ "fan-qqtt", { { 1, 2, 3, 9 }, { 1, 9, 4, 5 }, { 1, 5, 6 }, { 1, 6, 7 } } },
	{ "fan-tqtqt", { { 1, 2, 3 }, { 1, 3, 9, 4 }, { 1, 4, 5 }, { 1, 5, 6, 7 }, { 1, 7, 8 } } },
	{ "fan-ttt", { { 1, 2, 3 }, { 1, 3, 4 }, { 1, 4, 5 }, { 1, 5, 6 } } },
} do
	try(Case[1], function()
		local Mesh, V = mesh(Fan)
		for _, Face in Case[2] do
			local Ids = {}
			for _, Index in Face do
				table.insert(Ids, V[Index])
			end
			Mesh:AddFace(table.unpack(Ids))
		end
		local Out = {}
		for _, N in Mesh:GetNormals() do
			table.insert(Out, show(Mesh:GetNormal(N)))
		end
		return table.concat(Out, " ")
	end)
end
