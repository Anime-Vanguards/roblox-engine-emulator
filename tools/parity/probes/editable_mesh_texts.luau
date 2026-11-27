-- EditableMesh's per-face batches' argument checks, MeshPart:ApplyMesh's checks, MeshPart.MeshSize without a mesh, and
-- CreateEditableMeshAsync's argument casts (a playing client).
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
	elseif Kind == "Instance" then
		return Value.ClassName
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
		local Out = { "n=" .. (Results.n - 1) }
		for Index = 2, Results.n do
			table.insert(Out, show(Results[Index]))
		end
		print(Label .. " ok " .. table.concat(Out, " "))
	else
		print(Label .. " error " .. P.err(Results[2]))
	end
end
local function scene()
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local V = {}
	for _, Position in { Vector3.new(0, 0, 0), Vector3.new(2, 0, 0), Vector3.new(0, 2, 0), Vector3.new(2, 2, 0) } do
		table.insert(V, Mesh:AddVertex(Position))
	end
	local F = { Mesh:AddTriangle(V[1], V[2], V[3]), Mesh:AddTriangle(V[2], V[4], V[3]) }
	return Mesh, V, F
end

-- BatchGetFaceAttributes
for _, Case in {
	{ "get-none" },
	{ "get-attr-only", "attr" },
	{ "get-vertex", Enum.MeshAttribute.Vertex, "faces" },
	{ "get-normal", Enum.MeshAttribute.Normal, "faces" },
	{ "get-color", Enum.MeshAttribute.Color, "faces" },
	{ "get-uv", Enum.MeshAttribute.UV, "faces" },
	{ "get-face", Enum.MeshAttribute.Face, "faces" },
	{ "get-name", "Normal", "faces" },
	{ "get-number", 1, "faces" },
	{ "get-other-enum", Enum.KeyCode.A, "faces" },
	{ "get-not-table", Enum.MeshAttribute.Normal, 5 },
	{ "get-empty", Enum.MeshAttribute.Normal, {} },
	{ "get-zero", Enum.MeshAttribute.Normal, { 0 } },
	{ "get-gone", Enum.MeshAttribute.Normal, "gone" },
	{ "get-vertex-id", Enum.MeshAttribute.Normal, "vertex" },
	{ "get-text", Enum.MeshAttribute.Normal, "text" },
	{ "get-numeric-text", Enum.MeshAttribute.Normal, "numeric" },
	{ "get-extra", Enum.MeshAttribute.Normal, "faces", 1 },
} do
	try(Case[1], function()
		local Mesh, V, F = scene()
		local Faces = Case[3]
		if Faces == "faces" then
			Faces = F
		elseif Faces == "gone" then
			Mesh:RemoveFace(F[2])
			Faces = { F[1], F[2] }
		elseif Faces == "vertex" then
			Faces = { F[1], V[1] }
		elseif Faces == "text" then
			Faces = { F[1], "x" }
		elseif Faces == "numeric" then
			Faces = { tostring(F[1]) }
		end
		if Case[2] == nil then
			return show(Mesh:BatchGetFaceAttributes())
		elseif Case[2] == "attr" then
			return show(Mesh:BatchGetFaceAttributes(Enum.MeshAttribute.Normal))
		elseif #Case == 4 then
			return show(Mesh:BatchGetFaceAttributes(Case[2], Faces, Case[4]))
		end
		return show(Mesh:BatchGetFaceAttributes(Case[2], Faces))
	end)
end

-- BatchSetFaceAttributes
for _, Case in {
	{ "set-none" },
	{ "set-normals", "normals" },
	{ "set-uvs", "uvs" },
	{ "set-colors", "colors" },
	{ "set-vertices", "vertices" },
	{ "set-faces", "faces" },
	{ "set-two", "two" },
	{ "set-four", "four" },
	{ "set-mixed", "mixed" },
	{ "set-mixed-first", "mixed-first" },
	{ "set-length", "length" },
	{ "set-length-more", "length-more" },
	{ "set-not-table", "not-table" },
	{ "set-inner-not-table", "inner" },
	{ "set-zero-face", "zero-face" },
	{ "set-gone-face", "gone-face" },
	{ "set-gone-normal", "gone-normal" },
	{ "set-partial", "partial" },
	{ "set-empty", "empty" },
} do
	try(Case[1], function()
		local Mesh, V, F = scene()
		local N = { Mesh:AddNormal(Vector3.xAxis), Mesh:AddNormal(Vector3.yAxis), Mesh:AddNormal(Vector3.zAxis) }
		local U = { Mesh:AddUV(Vector2.new(0, 0)), Mesh:AddUV(Vector2.new(1, 0)), Mesh:AddUV(Vector2.new(0, 1)) }
		local C = { Mesh:AddColor(Color3.new(1, 0, 0), 1), Mesh:AddColor(Color3.new(0, 1, 0), 1), Mesh:AddColor(Color3.new(0, 0, 1), 1) }
		local K = Case[2]
		local Before = show(Mesh:GetFaceNormals(F[1]))
		if K == nil then
			Mesh:BatchSetFaceAttributes()
		elseif K == "normals" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { N })
		elseif K == "uvs" then
			Mesh:BatchSetFaceAttributes({ F[1], F[2] }, { U, { U[3], U[2], U[1] } })
		elseif K == "colors" then
			Mesh:BatchSetFaceAttributes({ F[2] }, { C })
		elseif K == "vertices" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { { V[4], V[2], V[3] } })
		elseif K == "faces" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { { F[1], F[2], F[1] } })
		elseif K == "two" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { { N[1], N[2] } })
		elseif K == "four" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { { N[1], N[2], N[3], N[1] } })
		elseif K == "mixed" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { { N[1], U[2], N[3] } })
		elseif K == "mixed-first" then
			Mesh:BatchSetFaceAttributes({ F[1], F[2] }, { N, U })
		elseif K == "length" then
			Mesh:BatchSetFaceAttributes({ F[1], F[2] }, { N })
		elseif K == "length-more" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { N, N })
		elseif K == "not-table" then
			Mesh:BatchSetFaceAttributes({ F[1] }, 5)
		elseif K == "inner" then
			Mesh:BatchSetFaceAttributes({ F[1] }, { 5 })
		elseif K == "zero-face" then
			Mesh:BatchSetFaceAttributes({ 0 }, { N })
		elseif K == "gone-face" then
			Mesh:RemoveFace(F[2])
			Mesh:BatchSetFaceAttributes({ F[1], F[2] }, { N, N })
		elseif K == "gone-normal" then
			local Gone = Mesh:AddNormal(Vector3.one)
			Mesh:RemoveUnused()
			Mesh:BatchSetFaceAttributes({ F[1] }, { { N[1], Gone, N[3] } })
		elseif K == "partial" then
			Mesh:BatchSetFaceAttributes({ F[1], F[2] }, { N, { N[1], 0, N[3] } })
		elseif K == "empty" then
			Mesh:BatchSetFaceAttributes({}, {})
		end
		return Before, show(Mesh:GetFaceNormals(F[1])), show(Mesh:GetFaceUVs(F[1])), show(Mesh:GetFaceVertices(F[1])), show(Mesh:GetFaceColors(F[2])), show(Mesh:GetFaceNormals(F[2]))
	end)
end
try("set-fixed", function()
	local Mesh, V, F = scene()
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Faces = Copy:GetFaces()
	local N = Copy:GetFaceNormals(Faces[2])
	local Ok, Message = pcall(Copy.BatchSetFaceAttributes, Copy, { Faces[1] }, { N })
	local Line = show(Copy:GetFaceNormals(Faces[1]))
	Copy:Destroy()
	return Ok, P.err(Message), Line
end)

-- MeshPart:ApplyMesh
local function part()
	local Mesh = scene()
	return AssetService:CreateMeshPartAsync(Content.fromObject(Mesh)), Mesh
end
for _, Case in {
	{ "apply-nil", nil },
	{ "apply-part", "part" },
	{ "apply-empty", "empty" },
	{ "apply-mesh", "mesh" },
	{ "apply-self", "self" },
	{ "apply-number", 5 },
	{ "apply-grown", "grown" },
	{ "apply-destroyed-mesh", "destroyed" },
} do
	try(Case[1], function()
		local Target = Instance.new("MeshPart")
		Target.Size = Vector3.new(3, 4, 5)
		local Source = Case[2]
		if Source == "part" then
			Source = Instance.new("Part")
		elseif Source == "empty" then
			Source = Instance.new("MeshPart")
		elseif Source == "mesh" then
			Source = part()
		elseif Source == "self" then
			Source = Target
		elseif Source == "grown" then
			local Mesh
			Source, Mesh = part()
			Mesh:SetPosition(Mesh:GetVertices()[1], Vector3.new(-3000, 0, 0))
		elseif Source == "destroyed" then
			local Mesh
			Source, Mesh = part()
			Mesh:Destroy()
		end
		Target:ApplyMesh(Source)
		return Target.MeshSize, Target.Size, Target.HasSkinnedMesh
	end)
end

-- MeshPart.MeshSize of a part with no mesh
try("meshsize-new", function()
	local Part = Instance.new("MeshPart")
	local Before = Part.MeshSize
	Part.Size = Vector3.new(7, 8, 9)
	return Before, Part.MeshSize, Part.Size
end)
try("meshsize-part", function()
	local Part = part()
	local Before = Part.MeshSize
	Part.Size = Vector3.new(7, 8, 9)
	return Before, Part.MeshSize, Part.Size
end)
try("meshsize-clone", function()
	local Part = part()
	local Copy = Part:Clone()
	local Empty = Instance.new("MeshPart"):Clone()
	return Copy.MeshSize, Empty.MeshSize
end)

-- CreateEditableMeshAsync's argument casts in a thread that can wait
for _, Case in {
	{ "async-none" },
	{ "async-nil", "nil" },
	{ "async-number", 5 },
	{ "async-table", {} },
	{ "async-dict", { x = 1 } },
	{ "async-bool", true },
	{ "async-part", "part" },
	{ "async-none-content", "none" },
	{ "async-string-empty", "" },
	{ "async-options-number", "options" },
} do
	try(Case[1], function()
		local Source = Case[2]
		if Source == "part" then
			Source = Instance.new("Part")
		elseif Source == "none" then
			Source = Content.none
		end
		if #Case == 1 then
			return AssetService:CreateEditableMeshAsync()
		elseif Source == "nil" then
			return AssetService:CreateEditableMeshAsync(nil)
		elseif Source == "options" then
			local Mesh = scene()
			return AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), 5)
		end
		return AssetService:CreateEditableMeshAsync(Source)
	end)
end

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

-- BatchSetFaceAttributes leaves what it set before an error
try("batch-set-partial", function()
	local Mesh, V = mesh(Square)
	local F = { Mesh:AddTriangle(V[1], V[2], V[3]), Mesh:AddTriangle(V[2], V[4], V[3]) }
	local N = { Mesh:AddNormal(Vector3.xAxis), Mesh:AddNormal(Vector3.yAxis), Mesh:AddNormal(Vector3.zAxis) }
	local U = Mesh:AddUV(Vector2.one)
	return caught(Mesh.BatchSetFaceAttributes, Mesh, { F[1], F[2] }, { N, { N[1], 0, N[3] } }), corners(Mesh, F[1]), corners(Mesh, F[2]),
		caught(Mesh.BatchSetFaceAttributes, Mesh, { F[2], F[1] }, { { N[3], N[2], N[1] }, { N[1], U, N[3] } }), corners(Mesh, F[1]), corners(Mesh, F[2])
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

-- BatchSetFaceAttributes' other texts
try("bsfa-texts", function()
	local Mesh, V = mesh(Square)
	local F = Mesh:AddTriangle(V[1], V[2], V[3])
	local N = { Mesh:AddNormal(), Mesh:AddNormal(), Mesh:AddNormal() }
	return caught(Mesh.BatchSetFaceAttributes, Mesh, 5, { N }), caught(Mesh.BatchSetFaceAttributes, Mesh, { "x" }, { N }),
		caught(Mesh.BatchSetFaceAttributes, Mesh, { V[1] }, { N }), caught(Mesh.BatchSetFaceAttributes, Mesh, { F }, { { N[1], "x", N[3] } }),
		caught(Mesh.BatchSetFaceAttributes, Mesh, { F }, { { 0, N[2], N[3] } }), caught(Mesh.BatchSetFaceAttributes, Mesh, { F }, { { "x", N[2], N[3] } }),
		caught(Mesh.BatchSetFaceAttributes, Mesh, { tostring(F) }, { N }), caught(Mesh.BatchSetFaceAttributes, Mesh, { F }, { { tostring(N[1]), N[2], N[3] } }),
		caught(Mesh.BatchSetFaceAttributes, Mesh, {}, { N })
end)
