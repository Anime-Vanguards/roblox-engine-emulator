-- EditableMesh's queries: GetFacesWith* / GetVerticesWith* (every attribute's lists as corners change), FindClosestVertex,
-- FindVerticesWithinSphere, FindClosestPointOnSurface, RaycastLocal, MergeVertices and IdDebugString (a playing client).
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
	elseif Kind == "table" then
		local Keys = {}
		for Key in Value do
			table.insert(Keys, Key)
		end
		table.sort(Keys, function(A, B)
			return tostring(A) < tostring(B)
		end)
		local Parts = {}
		if #Value == #Keys then
			for _, Item in ipairs(Value) do
				table.insert(Parts, show(Item))
			end
		else
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
-- two triangles and a quad over seven vertices, with set normals, uvs and colours on some corners
local function scene()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = {}
	for _, Position in { Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(0, 2, 0), Vector3.new(2, 2, 0), Vector3.new(0, 0, 2), Vector3.new(2, 0, 2), Vector3.new(5, 5, 5) } do
		table.insert(V, Mesh:AddVertex(Position))
	end
	local F = {
		Mesh:AddTriangle(V[1], V[2], V[3]),
		Mesh:AddTriangle(V[2], V[4], V[3]),
		Mesh:AddFace(V[1], V[5], V[6], V[2]),
	}
	return Mesh, V, F
end

-- (a vector to 4 decimals: the weights' and points' last float bits are a client's own, not followed)
local function approx(...)
	local Out = table.pack(...)
	for Index = 1, Out.n do
		local Value = Out[Index]
		if typeof(Value) == "Vector3" then
			local Text = string.format("V3~(%.4f %.4f %.4f)", Value.X + 0, Value.Y + 0, Value.Z + 0)
			Out[Index] = (string.gsub(Text, "%-(0%.0000)", "%1"))
		end
	end
	return table.unpack(Out, 1, Out.n)
end

-- GetFacesWith* and GetVerticesWith*
try("faces-with", function()
	local Mesh, V, F = scene()
	local N = Mesh:GetFaceNormals(F[2])
	local U = Mesh:GetFaceUVs(F[1])
	local C = Mesh:GetFaceColors(F[3])
	return show(Mesh:GetFacesWithAttribute(V[2])), show(Mesh:GetFacesWithAttribute(N[1])), show(Mesh:GetFacesWithAttribute(U[1])), show(Mesh:GetFacesWithAttribute(C[2])),
		show(Mesh:GetFacesWithNormal(N[1])), show(Mesh:GetFacesWithUV(U[2])), show(Mesh:GetFacesWithColor(C[4]))
end)
try("vertices-with", function()
	local Mesh, V, F = scene()
	local N = Mesh:GetFaceNormals(F[2])
	local U = Mesh:GetFaceUVs(F[1])
	local C = Mesh:GetFaceColors(F[3])
	return show(Mesh:GetVerticesWithAttribute(N[1])), show(Mesh:GetVerticesWithAttribute(U[3])), show(Mesh:GetVerticesWithAttribute(C[1])),
		show(Mesh:GetVerticesWithNormal(N[2])), show(Mesh:GetVerticesWithUV(U[1])), show(Mesh:GetVerticesWithColor(C[3]))
end)
try("with-shared", function()
	local Mesh, V, F = scene()
	local Shared = Mesh:AddNormal(Vector3.new(0, 0, 1))
	Mesh:SetFaceNormals(F[1], { Shared, Shared, Shared })
	Mesh:SetVertexFaceNormal(V[2], F[2], Shared)
	return show(Mesh:GetFacesWithNormal(Shared)), show(Mesh:GetVerticesWithNormal(Shared)), show(Mesh:GetFacesWithAttribute(Shared))
end)
try("with-unused", function()
	local Mesh = scene()
	local Lone = Mesh:AddNormal(Vector3.new(1, 0, 0))
	return show(Mesh:GetFacesWithNormal(Lone)), show(Mesh:GetVerticesWithNormal(Lone))
end)
for _, Case in {
	{ "faces-with-face", "GetFacesWithAttribute", 3 },
	{ "faces-with-zero", "GetFacesWithAttribute", 0 },
	{ "faces-with-bone", "GetFacesWithAttribute", "bone" },
	{ "faces-normal-of-uv", "GetFacesWithNormal", "uv" },
	{ "faces-uv-of-vertex", "GetFacesWithUV", "vertex" },
	{ "faces-color-of-face", "GetFacesWithColor", 3 },
	{ "faces-color-gone", "GetFacesWithColor", "gone-color" },
	{ "vertices-with-vertex", "GetVerticesWithAttribute", "vertex" },
	{ "vertices-with-face", "GetVerticesWithAttribute", 3 },
	{ "vertices-with-zero", "GetVerticesWithAttribute", 0 },
	{ "vertices-normal-of-color", "GetVerticesWithNormal", "color" },
	{ "vertices-uv-gone", "GetVerticesWithUV", "gone-uv" },
	{ "vertices-color-nil", "GetVerticesWithColor", "nil" },
	{ "vertices-color-text", "GetVerticesWithColor", "text" },
} do
	try(Case[1], function()
		local Mesh, V, F = scene()
		local Id = Case[3]
		if Id == 3 then
			Id = F[3]
		elseif Id == "bone" then
			Id = Mesh:AddBone({ Name = "b", CFrame = CFrame.new(), Virtual = false })
		elseif Id == "uv" then
			Id = Mesh:GetFaceUVs(F[1])[1]
		elseif Id == "vertex" then
			Id = V[1]
		elseif Id == "color" then
			Id = Mesh:GetFaceColors(F[1])[1]
		elseif Id == "gone-color" then
			Id = Mesh:AddColor(Color3.new(1, 0, 0), 1)
			Mesh:RemoveUnused()
		elseif Id == "gone-uv" then
			Id = Mesh:AddUV(Vector2.zero)
			Mesh:RemoveUnused()
		elseif Id == "nil" then
			Id = nil
		elseif Id == "text" then
			Id = "x"
		end
		return show((Mesh :: any)[Case[2]](Mesh, Id))
	end)
end

-- FindClosestVertex and FindVerticesWithinSphere
try("closest-vertex", function()
	local Mesh = scene()
	local Out = {}
	for _, Point in { Vector3.new(0.1, 0.1, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(9, 9, 9), Vector3.new(-3, 0, 0), Vector3.new(1, 1, 1) } do
		table.insert(Out, show(Mesh:FindClosestVertex(Point)))
	end
	return table.concat(Out, " ")
end)
try("closest-vertex-unused", function()
	local Mesh = scene()
	local Lone = Mesh:AddVertex(Vector3.new(10, 10, 10))
	return Lone, Mesh:FindClosestVertex(Vector3.new(11, 11, 11))
end)
try("closest-vertex-empty", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	return Mesh:FindClosestVertex(Vector3.zero)
end)
try("closest-vertex-nil", function()
	local Mesh = scene()
	return Mesh:FindClosestVertex(nil)
end)
try("closest-vertex-tie", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local A = Mesh:AddVertex(Vector3.new(1, 0, 0))
	local B = Mesh:AddVertex(Vector3.new(-1, 0, 0))
	local C = Mesh:AddVertex(Vector3.new(0, 1, 0))
	return A, B, C, Mesh:FindClosestVertex(Vector3.zero)
end)
try("closest-vertex-removed", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local A = Mesh:AddVertex(Vector3.new(0, 0, 0))
	local B = Mesh:AddVertex(Vector3.new(3, 0, 0))
	Mesh:RemoveUnused()
	local C = Mesh:AddVertex(Vector3.new(5, 0, 0))
	return A, B, C, Mesh:FindClosestVertex(Vector3.zero)
end)
for _, Case in {
	{ Vector3.new(0, 0, 0), 2 },
	{ Vector3.new(0, 0, 0), 2.0001 },
	{ Vector3.new(1, 1, 0), 1.5 },
	{ Vector3.new(1, 1, 1), 10 },
	{ Vector3.new(0, 0, 0), 0 },
	{ Vector3.new(0, 0, 0), -1 },
	{ Vector3.new(4, 4, 4), 1 },
} do
	try("within " .. show(Case[1]) .. " " .. P.num(Case[2]), function()
		local Mesh = scene()
		return show(Mesh:FindVerticesWithinSphere(Case[1], Case[2]))
	end)
end
try("within-nil", function()
	local Mesh = scene()
	return show(Mesh:FindVerticesWithinSphere(Vector3.zero))
end)

-- FindClosestPointOnSurface
for _, Point in {
	Vector3.new(0.5, 0.5, 1),
	Vector3.new(0.5, 0.5, -1),
	Vector3.new(1.5, 1.5, 0.25),
	Vector3.new(3, 1, 0),
	Vector3.new(1, -1, 1),
	Vector3.new(1, 0.3, 1.7),
	Vector3.new(5, 5, 5),
	Vector3.new(1, 1, 0),
} do
	try("surface " .. show(Point), function()
		local Mesh = scene()
		return approx(Mesh:FindClosestPointOnSurface(Point))
	end)
end
try("surface-empty", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	Mesh:AddVertex(Vector3.zero)
	return Mesh:FindClosestPointOnSurface(Vector3.one)
end)
try("surface-nil", function()
	local Mesh = scene()
	return Mesh:FindClosestPointOnSurface(nil)
end)

-- RaycastLocal
for _, Case in {
	{ Vector3.new(0.5, 0.5, 5), Vector3.new(0, 0, -10) },
	{ Vector3.new(0.5, 0.5, -5), Vector3.new(0, 0, 10) },
	{ Vector3.new(0.5, 0.5, 5), Vector3.new(0, 0, -2) },
	{ Vector3.new(0.5, 0.5, 5), Vector3.new(0, 0, -5) },
	{ Vector3.new(0.5, 0.5, 5), Vector3.new(0, 0, 10) },
	{ Vector3.new(1.5, 1.5, 3), Vector3.new(0, 0, -1) },
	{ Vector3.new(1, 5, 1), Vector3.new(0, -10, 0) },
	{ Vector3.new(1, -5, 1), Vector3.new(0, 10, 0) },
	{ Vector3.new(0.3, 0.7, 0), Vector3.new(0, 0, 1) },
	{ Vector3.new(9, 9, 9), Vector3.new(1, 0, 0) },
	{ Vector3.new(2, 2, 5), Vector3.new(0, 0, -10) },
	{ Vector3.new(0.5, 0.5, 5), Vector3.zero },
	{ Vector3.new(-1, 1, 1), Vector3.new(10, 0.1, -1) },
} do
	try("ray " .. show(Case[1]) .. " " .. show(Case[2]), function()
		local Mesh = scene()
		return approx(Mesh:RaycastLocal(Case[1], Case[2]))
	end)
end
try("ray-nil", function()
	local Mesh = scene()
	return Mesh:RaycastLocal(Vector3.zero)
end)

-- MergeVertices
try("merge", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = {
		Mesh:AddVertex(Vector3.new(0, 0, 0)),
		Mesh:AddVertex(Vector3.new(1, 0, 0)),
		Mesh:AddVertex(Vector3.new(0, 1, 0)),
		Mesh:AddVertex(Vector3.new(1, 0, 0)),
		Mesh:AddVertex(Vector3.new(0, 1, 0.05)),
		Mesh:AddVertex(Vector3.new(1, 1, 0)),
		Mesh:AddVertex(Vector3.new(7, 7, 7)),
	}
	local A = Mesh:AddTriangle(V[1], V[2], V[3])
	local B = Mesh:AddTriangle(V[4], V[6], V[5])
	local Map = Mesh:MergeVertices(0.1)
	return show(V), show(Map), show(Mesh:GetVertices()), show(Mesh:GetFaceVertices(A)), show(Mesh:GetFaceVertices(B)), #Mesh:GetNormals(), #Mesh:GetUVs()
end)
try("merge-zero", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = { Mesh:AddVertex(Vector3.new(0, 0, 0)), Mesh:AddVertex(Vector3.new(0, 0, 0)), Mesh:AddVertex(Vector3.new(0, 0, 0.001)) }
	local Map = Mesh:MergeVertices(0)
	return show(V), show(Map), show(Mesh:GetVertices())
end)
try("merge-chain", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = { Mesh:AddVertex(Vector3.new(0, 0, 0)), Mesh:AddVertex(Vector3.new(0.6, 0, 0)), Mesh:AddVertex(Vector3.new(1.2, 0, 0)) }
	local Map = Mesh:MergeVertices(1)
	return show(V), show(Map), show(Mesh:GetVertices()), show(Mesh:GetPosition(Mesh:GetVertices()[1]))
end)
try("merge-negative", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	Mesh:AddVertex(Vector3.zero)
	Mesh:AddVertex(Vector3.zero)
	return show(Mesh:MergeVertices(-1)), #Mesh:GetVertices()
end)
try("merge-nil", function()
	local Mesh = scene()
	return show(Mesh:MergeVertices())
end)
try("merge-fixed", function()
	local Mesh = scene()
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Ok, Map = pcall(Copy.MergeVertices, Copy, 0.1)
	Copy:Destroy()
	return Ok, if Ok then show(Map) else P.err(Map)
end)
try("merge-attrs", function()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = { Mesh:AddVertex(Vector3.new(0, 0, 0)), Mesh:AddVertex(Vector3.new(1, 0, 0)), Mesh:AddVertex(Vector3.new(0, 1, 0)), Mesh:AddVertex(Vector3.new(0, 0, 0)), Mesh:AddVertex(Vector3.new(0, -1, 0)) }
	local A = Mesh:AddTriangle(V[1], V[2], V[3])
	local B = Mesh:AddTriangle(V[4], V[5], V[2])
	local Before = show(Mesh:GetFaceNormals(A)) .. show(Mesh:GetFaceNormals(B))
	Mesh:MergeVertices(0.01)
	return Before, show(Mesh:GetFaceVertices(B)), show(Mesh:GetFaceNormals(A)), show(Mesh:GetFaceNormals(B)), show(Mesh:GetVertexFaces(V[1]))
end)

-- IdDebugString
try("debug", function()
	local Mesh, V, F = scene()
	local B = Mesh:AddBone({ Name = "b", CFrame = CFrame.new(), Virtual = false })
	local Out = {}
	for _, Id in { V[1], V[7], Mesh:GetFaceNormals(F[1])[1], Mesh:GetFaceColors(F[1])[1], Mesh:GetFaceUVs(F[1])[1], F[1], F[3], B } do
		table.insert(Out, Mesh:IdDebugString(Id))
	end
	return table.concat(Out, " ")
end)
try("debug-generations", function()
	local Mesh, V, F = scene()
	Mesh:RemoveFace(F[1])
	local G = Mesh:AddTriangle(V[1], V[2], V[3])
	Mesh:RemoveFace(G)
	local H = Mesh:AddTriangle(V[1], V[2], V[3])
	return Mesh:IdDebugString(G), Mesh:IdDebugString(H), Mesh:IdDebugString(F[1])
end)
for _, Value in { 0, 1, 7 * 4294967296 + 1, 4294967296 * 8 + 2, -1, 0.5, "x", "4294967297" } do
	try("debug " .. tostring(Value), function()
		local Mesh = scene()
		return Mesh:IdDebugString(Value)
	end)
end
try("debug-nil", function()
	local Mesh = scene()
	return Mesh:IdDebugString(nil)
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

-- the With* lists for every attribute, as corners change
local function study(Label, Mesh)
	for _, Kind in { "Normal", "UV", "Color" } do
		for _, Id in (Mesh :: any)["Get" .. Kind .. "s"](Mesh) do
			print(Label .. " " .. show(Id) .. " F" .. show(Mesh:GetFacesWithAttribute(Id)) .. " f" .. show((Mesh :: any)["GetFacesWith" .. Kind](Mesh, Id))
				.. " V" .. show(Mesh:GetVerticesWithAttribute(Id)) .. " v" .. show((Mesh :: any)["GetVerticesWith" .. Kind](Mesh, Id)))
		end
	end
	for _, Id in Mesh:GetVertices() do
		print(Label .. " " .. show(Id) .. " F" .. show(Mesh:GetFacesWithAttribute(Id)) .. " V" .. show(Mesh:GetVerticesWithAttribute(Id)))
	end
	for _, Id in Mesh:GetFaces() do
		print(Label .. " " .. show(Id) .. " V" .. show(Mesh:GetVerticesWithAttribute(Id)))
	end
end
local Six = { Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(0, 2, 0), Vector3.new(2, 2, 0), Vector3.new(0, 0, 2), Vector3.new(2, 0, 2) }
for _, Case in { "s1", "s2", "s3", "s4", "s5", "s6", "s7" } do
	local Ok, Message = pcall(function()
		local Mesh, V = mesh(Six)
		local F1 = Mesh:AddTriangle(V[1], V[2], V[3])
		local F2 = if Case == "s6" then Mesh:AddTriangle(V[4], V[5], V[6]) else Mesh:AddTriangle(V[2], V[4], V[3])
		local S = Mesh:AddNormal(Vector3.zAxis)
		if Case == "s2" then
			Mesh:SetVertexFaceNormal(V[2], F2, S)
		elseif Case == "s3" then
			Mesh:SetFaceNormals(F1, { S, S, S })
		elseif Case == "s4" or Case == "s5" then
			Mesh:AddFace(V[1], V[5], V[6], V[2])
			Mesh:SetFaceNormals(F1, { S, S, S })
			Mesh:SetVertexFaceNormal(V[2], F2, S)
			if Case == "s5" then
				Mesh:RemoveFace(F2)
			end
		elseif Case == "s6" then
			Mesh:SetFaceNormals(F1, { S, S, S })
			Mesh:SetVertexFaceNormal(V[4], F2, S)
		elseif Case == "s7" then
			Mesh:SetFaceVertices(F2, { V[4], V[5], V[6] })
		end
		study(Case, Mesh)
	end)
	if not Ok then
		print(Case .. " error " .. P.err(Message))
	end
	if Current then
		Current:Destroy()
		Current = nil
	end
end

-- FindClosestVertex and FindVerticesWithinSphere after a removal (listing order against slot order)
try("closest-order", function()
	local Mesh, V = mesh({ Vector3.new(0, 0, 3), Vector3.new(1, 0, 0), Vector3.new(-1, 0, 0), Vector3.new(0, 1, 0) })
	Mesh:AddTriangle(V[2], V[3], V[4])
	Mesh:RemoveUnused()
	return show(Mesh:GetVertices()), Mesh:FindClosestVertex(Vector3.zero), show(Mesh:FindVerticesWithinSphere(Vector3.zero, 1))
end)
try("closest-near-tie", function()
	local Mesh = mesh({ Vector3.new(1, 0, 0), Vector3.new(-1.0000001, 0, 0), Vector3.new(0, 0.99999994, 0) })
	return Mesh:FindClosestVertex(Vector3.new(0.1, 0, 0)), Mesh:FindClosestVertex(Vector3.zero), show(Mesh:FindVerticesWithinSphere(Vector3.zero, 0.99999994))
end)

-- FindClosestPointOnSurface and RaycastLocal on a skewed triangle and a quad (float bits, regions, ties)
local Skew = { Vector3.new(0.13, -0.27, 0.41), Vector3.new(2.71, 0.33, -0.19), Vector3.new(0.57, 1.93, 0.77), Vector3.new(3.1, 2.2, 1.3), Vector3.new(-0.6, 1.4, 1.9) }
local function skew()
	local Mesh, V = mesh(Skew)
	local F = { Mesh:AddTriangle(V[1], V[2], V[3]), Mesh:AddFace(V[2], V[4], V[5], V[3]) }
	return Mesh, V, F
end
for Index, Point in {
	Vector3.new(1, 0.5, 1), Vector3.new(1, 0.5, -1), Vector3.new(-1, -1, 0), Vector3.new(3, 0, 0), Vector3.new(0.3, 2.5, 0.5),
	Vector3.new(1.5, -0.5, 0), Vector3.new(1.8, 1.4, 0.2), Vector3.new(-0.5, 1, 0.5), Vector3.new(0.9, 0.4, 0.3), Vector3.new(2.5, 1.9, 1.1),
	Vector3.new(1.2, 2.4, 1.6), Vector3.new(0, 2, 3), Vector3.new(4, 3, 2), Vector3.new(1.7, 0.8, 0.27),
} do
	try("closest-skew-" .. Index, function()
		local Mesh = skew()
		return approx(Mesh:FindClosestPointOnSurface(Point))
	end)
end
for Index, Ray in {
	{ Vector3.new(1, 0.5, 3), Vector3.new(0.1, -0.05, -6) },
	{ Vector3.new(1, 0.5, -3), Vector3.new(-0.1, 0.05, 6) },
	{ Vector3.new(0.9, 0.4, 2), Vector3.new(0, 0, -4) },
	{ Vector3.new(2, 1.5, 5), Vector3.new(0, 0, -10) },
	{ Vector3.new(1.5, 1.8, 5), Vector3.new(0.2, 0.1, -10) },
	{ Vector3.new(-3, 0.5, 0.3), Vector3.new(10, 0.2, 0.1) },
	{ Vector3.new(1.7, 0.8, 4), Vector3.new(0, 0, -8) },
	{ Vector3.new(0.13, -0.27, 3), Vector3.new(0, 0, -6) },
	{ Vector3.new(1.42, 0.03, 3), Vector3.new(0, 0, -6) },
} do
	try("ray-skew-" .. Index, function()
		local Mesh = skew()
		return approx(Mesh:RaycastLocal(Ray[1], Ray[2]))
	end)
end
-- reach: a ray whose end is on the face, just past it, just short of it
for _, Length in { 4, 4.000001, 3.999999, 4.5 } do
	try("ray-reach " .. P.num(Length), function()
		local Mesh, V = mesh({ Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(0, 2, 0) })
		Mesh:AddTriangle(V[1], V[2], V[3])
		return approx(Mesh:RaycastLocal(Vector3.new(0.5, 0.5, 4), Vector3.new(0, 0, -Length)))
	end)
end
try("ray-two", function()
	local Mesh, V = mesh({ Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(0, 2, 0), Vector3.new(0, 0, 1), Vector3.new(2, 0, 1), Vector3.new(0, 2, 1) })
	Mesh:AddTriangle(V[1], V[2], V[3])
	Mesh:AddTriangle(V[4], V[5], V[6])
	return approx(Mesh:RaycastLocal(Vector3.new(0.5, 0.5, 5), Vector3.new(0, 0, -10)), Mesh:RaycastLocal(Vector3.new(0.5, 0.5, -5), Vector3.new(0, 0, 10)))
end)

-- MergeVertices
local function merge(Label, Positions, Faces, Tolerance)
	try(Label, function()
		local Mesh, V = mesh(Positions)
		local F = {}
		for _, Face in Faces do
			local Ids = {}
			for _, Index in Face do
				table.insert(Ids, V[Index])
			end
			table.insert(F, Mesh:AddFace(table.unpack(Ids)))
		end
		local Map = Mesh:MergeVertices(Tolerance)
		local Out = {}
		for _, Face in F do
			table.insert(Out, corners(Mesh, Face))
		end
		local Faces2 = {}
		for _, W in V do
			table.insert(Faces2, show(Mesh:GetVertexFaces(W)))
		end
		return show(Map), table.concat(Out, " "), table.concat(Faces2, ""), show(Mesh:RemoveUnused())
	end)
end
merge("merge-pair", { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 0), Vector3.new(0, -1, 0) }, { { 1, 2, 3 }, { 4, 5, 2 } }, 0.01)
merge("merge-later-first", { Vector3.new(0, -1, 0), Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 0) }, { { 5, 3, 4 }, { 2, 1, 3 } }, 0.01)
merge("merge-tol-eq", { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0.5, 0, 0), Vector3.new(0, -1, 0) }, { { 1, 2, 3 }, { 4, 5, 2 } }, 0.5)
merge("merge-tol-under", { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0.5, 0, 0), Vector3.new(0, -1, 0) }, { { 1, 2, 3 }, { 4, 5, 2 } }, 0.49)
merge("merge-chain", { Vector3.new(0, 0, 0), Vector3.new(0.6, 0, 0), Vector3.new(1.2, 0, 0), Vector3.new(0, 5, 0), Vector3.new(0, 6, 0), Vector3.new(0, 7, 0) }, { { 1, 4, 5 }, { 2, 5, 6 }, { 3, 6, 4 } }, 1)
merge("merge-same-face", { Vector3.new(0, 0, 0), Vector3.new(0.001, 0, 0), Vector3.new(0, 1, 0) }, { { 1, 2, 3 } }, 0.01)
merge("merge-quad", { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(1, 1, 0), Vector3.new(0, 1, 0), Vector3.new(1, 0, 0), Vector3.new(2, 0, 0), Vector3.new(2, 1, 0) }, { { 1, 2, 3, 4 }, { 5, 6, 7 } }, 0.01)
merge("merge-none", { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0) }, { { 1, 2, 3 } }, 0.01)
merge("merge-unused", { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 0) }, { { 1, 2, 3 } }, 0.01)
merge("merge-three", { Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 0), Vector3.new(0, -1, 0), Vector3.new(0, 0, 0), Vector3.new(-1, 0, 0) }, { { 1, 2, 3 }, { 4, 5, 2 }, { 6, 3, 7 } }, 0.01)
try("merge-fixed", function()
	local Mesh, V = mesh({ Vector3.new(0, 0, 0), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 0), Vector3.new(0, -1, 0) })
	Mesh:AddTriangle(V[1], V[2], V[3])
	Mesh:AddTriangle(V[4], V[5], V[2])
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Ok, Map = pcall(Copy.MergeVertices, Copy, 0.01)
	local Line = if Ok then show(Map) else P.err(Map)
	local Faces = {}
	for _, F in Copy:GetFaces() do
		table.insert(Faces, show(Copy:GetFaceVertices(F)))
	end
	Copy:Destroy()
	return Ok, Line, table.concat(Faces, " ")
end)
try("merge-text", function()
	local Mesh = mesh({ Vector3.zero })
	return show(Mesh:MergeVertices("x"))
end)

-- IdDebugString of generations
for _, Generation in { 1, 2, 254, 255, 256, 257, 65535 } do
	try("debug-gen " .. P.num(Generation), function()
		local Mesh = mesh({})
		return Mesh:IdDebugString(((Generation * 8) + 5) * 4294967296 + 3)
	end)
end
try("debug-kinds", function()
	local Mesh = mesh({})
	local Out = {}
	for Kind = 0, 7 do
		table.insert(Out, Mesh:IdDebugString(Kind * 4294967296 + 12))
	end
	return table.concat(Out, " ")
end)
try("debug-slot-zero", function()
	local Mesh = mesh({})
	return Mesh:IdDebugString(4294967296), Mesh:IdDebugString(5 * 4294967296)
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

-- MergeVertices: its map, and which vertex each one goes to
local function merge(Label, Positions, Faces, Tolerance)
	try(Label, function()
		local Mesh, V = mesh(Positions)
		local F = {}
		for _, Face in Faces do
			local Ids = {}
			for _, Index in Face do
				table.insert(Ids, V[Index])
			end
			table.insert(F, Mesh:AddFace(table.unpack(Ids)))
		end
		local Map = Mesh:MergeVertices(Tolerance)
		local Out = {}
		for _, Face in F do
			table.insert(Out, show(Mesh:GetFaceVertices(Face)))
		end
		return show(Map), table.concat(Out, " ")
	end)
end
local O, X, Y = Vector3.zero, Vector3.xAxis, Vector3.yAxis
merge("merge-four", { O, X, Y, O, -Y, O, -X, O, Vector3.new(0, 0, 1) }, { { 1, 2, 3 }, { 4, 5, 2 }, { 6, 3, 7 }, { 8, 9, 2 } }, 0.01)
merge("merge-two-threes", { O, X, Y, O, -Y, O, -X, Vector3.new(5, 0, 0), Vector3.new(5, 0, 0), Vector3.new(5, 0, 0), Vector3.new(6, 0, 0) }, { { 1, 2, 3 }, { 4, 5, 2 }, { 6, 3, 7 }, { 8, 11, 2 }, { 9, 11, 3 }, { 10, 11, 1 } }, 0.01)
merge("merge-three-one-unused", { O, X, Y, O, -Y, O }, { { 1, 2, 3 }, { 4, 5, 2 } }, 0.01)
merge("merge-three-one-face", { O, X, O, O }, { { 1, 2, 3 }, { 4, 2, 1 } }, 0.01)
merge("merge-target", { O, Vector3.new(0.5, 0, 0), Vector3.new(0.8, 0, 0), Vector3.new(0, 5, 0), Vector3.new(0, 6, 0), Vector3.new(0, 7, 0) }, { { 1, 4, 5 }, { 2, 5, 6 }, { 3, 6, 4 } }, 1)
merge("merge-target-far-first", { O, Vector3.new(0.9, 0, 0), Vector3.new(0.5, 0, 0), Vector3.new(0, 5, 0), Vector3.new(0, 6, 0), Vector3.new(0, 7, 0) }, { { 1, 4, 5 }, { 2, 5, 6 }, { 3, 6, 4 } }, 1)
merge("merge-pair-again", { O, X, Y, O, -Y }, { { 1, 2, 3 }, { 4, 5, 2 } }, 0.01)
merge("merge-pairs-two", { O, X, Y, O, -Y, Vector3.new(5, 0, 0), Vector3.new(5, 0, 0), Vector3.new(6, 0, 0) }, { { 1, 2, 3 }, { 4, 5, 2 }, { 6, 8, 1 }, { 7, 8, 2 } }, 0.01)
try("merge-after", function()
	local Mesh, V = mesh({ O, X, Y, O, -Y, O, -X })
	Mesh:AddTriangle(V[1], V[2], V[3])
	Mesh:AddTriangle(V[4], V[5], V[2])
	Mesh:AddTriangle(V[6], V[3], V[7])
	local First = Mesh:MergeVertices(0.01)
	local Second = Mesh:MergeVertices(0.01)
	return show(First), show(Second), show(Mesh:GetVertices())
end)

-- sorted lists when generations differ from slots
try("sorted-generations", function()
	local Mesh, V = mesh({ Vector3.new(0, 0, 3), Vector3.new(1, 0, 0), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1) })
	Mesh:AddTriangle(V[2], V[3], V[4])
	Mesh:RemoveUnused()
	local New = Mesh:AddVertex(Vector3.new(-1, 0, 0))
	local F = Mesh:AddTriangle(New, V[2], V[3])
	local N = Mesh:AddNormal(Vector3.zAxis)
	Mesh:SetFaceNormals(F, { N, N, N })
	return New, show(Mesh:GetVerticesWithAttribute(N)), show(Mesh:GetVerticesWithNormal(N)), Mesh:FindClosestVertex(Vector3.zero), show(Mesh:FindVerticesWithinSphere(Vector3.zero, 1)), show(Mesh:GetVertices())
end)
