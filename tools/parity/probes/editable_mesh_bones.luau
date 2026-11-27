-- EditableMesh bones: their slots and ids, GetBoneByName, the setters' checks, AddBone's checks, their names' rules and the
-- FACS poses (a playing client).
local AssetService = game:GetService("AssetService")
local function show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		if Value > 4294967296 then
			return string.format("%d:%d", Value // 4294967296, Value % 4294967296)
		end
		return P.num(Value)
	elseif Kind == "CFrame" then
		local Parts = { Value:GetComponents() }
		for Index, Part in Parts do
			Parts[Index] = Part + 0
		end
		return "CF(" .. P.f32s(table.unpack(Parts)) .. ")"
	elseif Kind == "EnumItem" then
		return tostring(Value)
	elseif Kind == "table" then
		local Parts = {}
		if Value.n then
			for Index = 1, Value.n do
				table.insert(Parts, show(Value[Index]))
			end
			return "(" .. table.concat(Parts, ",") .. ")"
		end
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
local function bone(Mesh, Name, Position)
	return Mesh:AddBone({ Name = Name, CFrame = CFrame.new(Position or Vector3.zero), Virtual = false })
end
local function fresh(Count)
	local Mesh = AssetService:CreateEditableMesh()
	Current = Mesh
	local B = {}
	for Index = 1, Count or 3 do
		table.insert(B, bone(Mesh, "b" .. Index, Vector3.new(Index, 0, 0)))
	end
	return Mesh, B
end
local function names(Mesh)
	local Out = {}
	for _, B in Mesh:GetBones() do
		table.insert(Out, show(B) .. "=" .. Mesh:GetBoneName(B))
	end
	return table.concat(Out, " ")
end

-- slots: removal, reuse, the list's order
try("slots", function()
	local Mesh, B = fresh(4)
	Mesh:RemoveBone(B[2])
	local C = bone(Mesh, "c")
	Mesh:RemoveBone(B[4])
	Mesh:RemoveBone(B[1])
	local D = bone(Mesh, "d")
	local E = bone(Mesh, "e")
	local F = bone(Mesh, "f")
	return names(Mesh), C, D, E, F
end)
try("slots-again", function()
	local Mesh, B = fresh(1)
	Mesh:RemoveBone(B[1])
	local C = bone(Mesh, "c")
	Mesh:RemoveBone(C)
	local D = bone(Mesh, "d")
	return B[1], C, D
end)
try("removed-id", function()
	local Mesh, B = fresh(2)
	Mesh:RemoveBone(B[1])
	return Mesh:GetBoneName(B[1])
end)
try("remove-twice", function()
	local Mesh, B = fresh(2)
	Mesh:RemoveBone(B[1])
	Mesh:RemoveBone(B[1])
	return "removed"
end)
try("remove-parent", function()
	local Mesh, B = fresh(3)
	Mesh:SetBoneParent(B[2], B[1])
	Mesh:SetBoneParent(B[3], B[2])
	Mesh:RemoveBone(B[2])
	return Mesh:GetBoneParent(B[3]), Mesh:GetBoneParent(B[1])
end)
try("remove-weights", function()
	local Mesh, B = fresh(3)
	local V = Mesh:AddVertex(Vector3.zero)
	Mesh:SetVertexBones(V, { B[1], B[2], B[3] })
	Mesh:SetVertexBoneWeights(V, { 0.25, 0.5, 0.25 })
	Mesh:RemoveBone(B[2])
	return show(Mesh:GetVertexBones(V)), show(Mesh:GetVertexBoneWeights(V))
end)

-- GetBoneByName
try("by-name", function()
	local Mesh, B = fresh(3)
	return Mesh:GetBoneByName("b2"), B[2]
end)
try("by-name-missing", function()
	local Mesh = fresh(3)
	return Mesh:GetBoneByName("nope")
end)
try("by-name-empty-mesh", function()
	local Mesh = fresh(0)
	return Mesh:GetBoneByName("b1")
end)
try("by-name-twice", function()
	local Mesh, B = fresh(3)
	local Again = bone(Mesh, "b2")
	return Mesh:GetBoneByName("b2"), B[2], Again
end)
try("by-name-renamed", function()
	local Mesh, B = fresh(3)
	Mesh:SetBoneName(B[3], "b1")
	return Mesh:GetBoneByName("b1"), B[1], B[3]
end)
try("by-name-removed", function()
	local Mesh, B = fresh(3)
	local Again = bone(Mesh, "b1")
	Mesh:RemoveBone(B[1])
	return Mesh:GetBoneByName("b1"), Again
end)
try("by-name-nil", function()
	local Mesh = fresh(3)
	return Mesh:GetBoneByName(nil)
end)
try("by-name-number", function()
	local Mesh = fresh(0)
	bone(Mesh, "5")
	return Mesh:GetBoneByName(5)
end)
try("by-name-table", function()
	local Mesh = fresh(3)
	return Mesh:GetBoneByName({})
end)
try("by-name-case", function()
	local Mesh = fresh(3)
	return Mesh:GetBoneByName("B1")
end)
try("by-name-empty", function()
	local Mesh = fresh(0)
	bone(Mesh, "")
	return Mesh:GetBoneByName("")
end)

-- SetBoneName's checks
for _, Case in {
	{ "name-nil", nil },
	{ "name-number", 5 },
	{ "name-table", {} },
	{ "name-bool", true },
	{ "name-empty", "" },
	{ "name-long", string.rep("x", 300) },
	{ "name-nul", "a\0b" },
} do
	try(Case[1], function()
		local Mesh, B = fresh(2)
		Mesh:SetBoneName(B[1], Case[2])
		local Name = Mesh:GetBoneName(B[1])
		return #Name, string.sub(Name, 1, 8)
	end)
end
try("name-zero", function()
	local Mesh = fresh(2)
	Mesh:SetBoneName(0, "x")
	return "set"
end)
try("name-wrong-kind", function()
	local Mesh = fresh(2)
	Mesh:SetBoneName(Mesh:AddVertex(Vector3.zero), "x")
	return "set"
end)
try("name-gone", function()
	local Mesh, B = fresh(2)
	Mesh:RemoveBone(B[1])
	Mesh:SetBoneName(B[1], "x")
	return "set"
end)
try("name-fixed", function()
	local Mesh, B = fresh(2)
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Bones = Copy:GetBones()
	local Ok, Message = pcall(Copy.SetBoneName, Copy, Bones[1], "x")
	local Name = Copy:GetBoneName(Bones[1])
	Copy:Destroy()
	return Ok, P.err(Message), Name, B[1] == Bones[1]
end)

-- SetBoneIsVirtual, SetBoneParent, SetBoneCFrame
for _, Case in { { "virtual-true", true }, { "virtual-nil", nil }, { "virtual-number", 1 }, { "virtual-text", "x" } } do
	try(Case[1], function()
		local Mesh, B = fresh(2)
		Mesh:SetBoneIsVirtual(B[1], Case[2])
		return Mesh:GetBoneIsVirtual(B[1]), Mesh:GetBoneIsVirtual(B[2])
	end)
end
try("virtual-gone", function()
	local Mesh, B = fresh(2)
	Mesh:RemoveBone(B[1])
	Mesh:SetBoneIsVirtual(B[1], true)
	return "set"
end)
try("virtual-fixed", function()
	local Mesh = fresh(2)
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Bones = Copy:GetBones()
	local Ok, Message = pcall(Copy.SetBoneIsVirtual, Copy, Bones[1], true)
	local Value = Copy:GetBoneIsVirtual(Bones[1])
	Copy:Destroy()
	return Ok, P.err(Message), Value
end)
for _, Case in {
	{ "parent-zero", 0 },
	{ "parent-self", "self" },
	{ "parent-other", "other" },
	{ "parent-nil", nil },
	{ "parent-vertex", "vertex" },
	{ "parent-gone", "gone" },
	{ "parent-cycle", "cycle" },
} do
	try(Case[1], function()
		local Mesh, B = fresh(3)
		local Parent = Case[2]
		if Parent == "self" then
			Parent = B[1]
		elseif Parent == "other" then
			Parent = B[2]
		elseif Parent == "vertex" then
			Parent = Mesh:AddVertex(Vector3.zero)
		elseif Parent == "gone" then
			Mesh:RemoveBone(B[3])
			Parent = B[3]
		elseif Parent == "cycle" then
			Mesh:SetBoneParent(B[2], B[1])
			Parent = B[2]
		end
		Mesh:SetBoneParent(B[1], Parent)
		return Mesh:GetBoneParent(B[1]), Mesh:GetBoneParent(B[2])
	end)
end
try("parent-clear", function()
	local Mesh, B = fresh(3)
	Mesh:SetBoneParent(B[1], B[2])
	Mesh:SetBoneParent(B[1], 0)
	return Mesh:GetBoneParent(B[1])
end)
for _, Case in { { "cframe-nil", nil }, { "cframe-vector", Vector3.one }, { "cframe-number", 1 } } do
	try(Case[1], function()
		local Mesh, B = fresh(2)
		Mesh:SetBoneCFrame(B[1], Case[2])
		return Mesh:GetBoneCFrame(B[1])
	end)
end
try("cframe-round", function()
	local Mesh, B = fresh(1)
	local Turned = CFrame.new(1.25, 2.5, -3) * CFrame.Angles(0.3, -0.7, 1.1)
	Mesh:SetBoneCFrame(B[1], Turned)
	return Mesh:GetBoneCFrame(B[1])
end)

-- AddBone's checks
for _, Case in {
	{ "add-cframe-vector", { Name = "x", CFrame = Vector3.one, Virtual = false } },
	{ "add-virtual-number", { Name = "x", CFrame = CFrame.new(), Virtual = 1 } },
	{ "add-name-number", { Name = 5, CFrame = CFrame.new(), Virtual = false } },
	{ "add-nothing", {} },
	{ "add-extra", { Name = "x", CFrame = CFrame.new(), Virtual = true, Parent = 1 } },
	{ "add-empty-name", { Name = "", CFrame = CFrame.new(), Virtual = false } },
} do
	try(Case[1], function()
		local Mesh = fresh(0)
		local B = Mesh:AddBone(Case[2])
		return B, Mesh:GetBoneName(B), Mesh:GetBoneIsVirtual(B), Mesh:GetBoneParent(B)
	end)
end
try("add-nil", function()
	local Mesh = fresh(0)
	return Mesh:AddBone(nil)
end)
try("add-string", function()
	local Mesh = fresh(0)
	return Mesh:AddBone("x")
end)

-- FACS poses
local Units = Enum.FacsActionUnit:GetEnumItems()
try("facs-empty", function()
	local Mesh = fresh(2)
	return show(Mesh:GetFacsPoses()), show(Mesh:GetFacsCorrectivePoses()), Mesh:GetFacsPose(Units[1])
end)
try("facs-set", function()
	local Mesh, B = fresh(3)
	Mesh:SetFacsPose(Units[2], { B[1], B[3] }, { CFrame.new(1, 0, 0), CFrame.Angles(0, 0.5, 0) })
	Mesh:SetFacsPose(Units[1], { B[2] }, { CFrame.new(0, 2, 0) })
	local Ids, Frames = Mesh:GetFacsPose(Units[2])
	return show(Mesh:GetFacsPoses()), show(Ids), show(Frames), show(table.pack(Mesh:GetFacsPose(Units[3])))
end)
try("facs-set-again", function()
	local Mesh, B = fresh(3)
	Mesh:SetFacsPose(Units[2], { B[1], B[3] }, { CFrame.new(1, 0, 0), CFrame.new(2, 0, 0) })
	Mesh:SetFacsPose(Units[2], { B[2] }, { CFrame.new(3, 0, 0) })
	return show(table.pack(Mesh:GetFacsPose(Units[2]))), show(Mesh:GetFacsPoses())
end)
try("facs-bone-pose", function()
	local Mesh, B = fresh(3)
	Mesh:SetFacsBonePose(Units[4], B[2], CFrame.new(0, 0, 4))
	Mesh:SetFacsBonePose(Units[4], B[1], CFrame.new(0, 0, 5))
	Mesh:SetFacsBonePose(Units[4], B[2], CFrame.new(0, 0, 6))
	return show(table.pack(Mesh:GetFacsPose(Units[4]))), show(Mesh:GetFacsPoses())
end)
try("facs-clear", function()
	local Mesh, B = fresh(3)
	Mesh:SetFacsPose(Units[2], { B[1] }, { CFrame.new(1, 0, 0) })
	Mesh:SetFacsPose(Units[2], {}, {})
	return show(table.pack(Mesh:GetFacsPose(Units[2]))), show(Mesh:GetFacsPoses())
end)
for _, Case in {
	{ "facs-mismatch", "mismatch" },
	{ "facs-bad-bone", "vertex" },
	{ "facs-gone-bone", "gone" },
	{ "facs-bad-frame", "frame" },
	{ "facs-unit-name", "name" },
	{ "facs-unit-number", "number" },
	{ "facs-unit-other", "other" },
} do
	try(Case[1], function()
		local Mesh, B = fresh(3)
		local Unit, Ids, Frames = Units[2], { B[1] }, { CFrame.new() }
		if Case[2] == "mismatch" then
			Frames = { CFrame.new(), CFrame.new() }
		elseif Case[2] == "vertex" then
			Ids = { Mesh:AddVertex(Vector3.zero) }
		elseif Case[2] == "gone" then
			Mesh:RemoveBone(B[1])
		elseif Case[2] == "frame" then
			Frames = { Vector3.one }
		elseif Case[2] == "name" then
			Unit = Units[2].Name
		elseif Case[2] == "number" then
			Unit = Units[2].Value
		elseif Case[2] == "other" then
			Unit = Enum.KeyCode.A
		end
		Mesh:SetFacsPose(Unit, Ids, Frames)
		return show(table.pack(Mesh:GetFacsPose(Units[2])))
	end)
end
try("facs-corrective", function()
	local Mesh, B = fresh(3)
	Mesh:SetFacsCorrectivePose({ Units[3], Units[1] }, { B[2] }, { CFrame.new(0, 1, 0) })
	Mesh:SetFacsCorrectivePose({ Units[5], Units[2], Units[4] }, { B[1], B[3] }, { CFrame.new(1, 0, 0), CFrame.new(3, 0, 0) })
	return show(Mesh:GetFacsCorrectivePoses()), show(table.pack(Mesh:GetFacsCorrectivePose({ Units[1], Units[3] }))), show(table.pack(Mesh:GetFacsCorrectivePose({ Units[3], Units[1] }))),
		show(table.pack(Mesh:GetFacsCorrectivePose({ Units[2], Units[4], Units[5] }))), show(table.pack(Mesh:GetFacsCorrectivePose({ Units[1], Units[2] })))
end)
for _, Case in { { "corrective-one", 1 }, { "corrective-four", 4 }, { "corrective-same", "same" }, { "corrective-empty", 0 } } do
	try(Case[1], function()
		local Mesh, B = fresh(2)
		local Actions = if Case[2] == "same" then { Units[1], Units[1] } else table.move(Units, 1, Case[2], 1, {})
		Mesh:SetFacsCorrectivePose(Actions, { B[1] }, { CFrame.new() })
		return show(Mesh:GetFacsCorrectivePoses()), show(table.pack(Mesh:GetFacsCorrectivePose(Actions)))
	end)
end
try("facs-copy", function()
	local Mesh, B = fresh(2)
	Mesh:SetFacsPose(Units[2], { B[1] }, { CFrame.new(1, 0, 0) })
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = false })
	local Line = show(Copy:GetFacsPoses()) .. " " .. show(table.pack(Copy:GetFacsPose(Units[2])))
	Copy:Destroy()
	return Line
end)
try("facs-remove-bone", function()
	local Mesh, B = fresh(3)
	Mesh:SetFacsPose(Units[2], { B[1], B[2] }, { CFrame.new(1, 0, 0), CFrame.new(2, 0, 0) })
	Mesh:RemoveBone(B[1])
	return show(table.pack(Mesh:GetFacsPose(Units[2])))
end)
try("facs-fixed", function()
	local Mesh = fresh(2)
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = true })
	local Bones = Copy:GetBones()
	local Ok, Message = pcall(Copy.SetFacsPose, Copy, Units[2], { Bones[1] }, { CFrame.new(1, 0, 0) })
	local Line = show(table.pack(Copy:GetFacsPose(Units[2])))
	Copy:Destroy()
	return Ok, P.err(Message), Line
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
		local Parts = { Value:GetComponents() }
		for Index, Part in Parts do
			Parts[Index] = Part + 0
		end
		return "CF(" .. P.f32s(table.unpack(Parts)) .. ")"
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

-- FACS poses on virtual bones
local Units = Enum.FacsActionUnit:GetEnumItems()
local function rig(Count)
	local Mesh = mesh({})
	local B = {}
	for Index = 1, Count do
		table.insert(B, Mesh:AddBone({ Name = "b" .. Index, CFrame = CFrame.new(Index, 0, 0), Virtual = Index ~= Count }))
	end
	return Mesh, B
end
try("facs-units", function()
	return #Units, tostring(Units[1]), tostring(Units[2]), tostring(Units[#Units])
end)
try("facs-set", function()
	local Mesh, B = rig(4)
	Mesh:SetFacsPose(Units[2], { B[1], B[3] }, { CFrame.new(1, 0, 0), CFrame.Angles(0, 0.5, 0) })
	Mesh:SetFacsPose(Units[1], { B[2] }, { CFrame.new(0, 2, 0) })
	return show(Mesh:GetFacsPoses()), show(table.pack(Mesh:GetFacsPose(Units[2]))), show(table.pack(Mesh:GetFacsPose(Units[3])))
end)
try("facs-set-again", function()
	local Mesh, B = rig(4)
	Mesh:SetFacsPose(Units[2], { B[1], B[3] }, { CFrame.new(1, 0, 0), CFrame.new(2, 0, 0) })
	Mesh:SetFacsPose(Units[2], { B[2] }, { CFrame.new(3, 0, 0) })
	return show(table.pack(Mesh:GetFacsPose(Units[2]))), show(Mesh:GetFacsPoses())
end)
try("facs-bone-pose", function()
	local Mesh, B = rig(4)
	Mesh:SetFacsBonePose(Units[4], B[2], CFrame.new(0, 0, 4))
	Mesh:SetFacsBonePose(Units[4], B[1], CFrame.new(0, 0, 5))
	Mesh:SetFacsBonePose(Units[4], B[2], CFrame.new(0, 0, 6))
	return show(table.pack(Mesh:GetFacsPose(Units[4]))), show(Mesh:GetFacsPoses())
end)
try("facs-bone-pose-errors", function()
	local Mesh, B = rig(3)
	return caught(Mesh.SetFacsBonePose, Mesh, Units[1], B[3], CFrame.new()), caught(Mesh.SetFacsBonePose, Mesh, Units[1], B[1], nil),
		caught(Mesh.SetFacsBonePose, Mesh, Units[1], 0, CFrame.new()), caught(Mesh.SetFacsBonePose, Mesh, nil, B[1], CFrame.new()),
		caught(Mesh.SetFacsBonePose, Mesh, Units[1], B[1], Vector3.one)
end)
try("facs-clear", function()
	local Mesh, B = rig(3)
	Mesh:SetFacsPose(Units[2], { B[1] }, { CFrame.new(1, 0, 0) })
	Mesh:SetFacsPose(Units[2], {}, {})
	return show(table.pack(Mesh:GetFacsPose(Units[2]))), show(Mesh:GetFacsPoses())
end)
try("facs-identity", function()
	local Mesh, B = rig(3)
	Mesh:SetFacsPose(Units[2], { B[1] }, { CFrame.new() })
	return show(table.pack(Mesh:GetFacsPose(Units[2]))), show(Mesh:GetFacsPoses())
end)
for _, Case in {
	{ "facs-bad-frame", "frame" },
	{ "facs-unit-name", "name" },
	{ "facs-unit-number", "number" },
	{ "facs-unit-nil", "nil" },
	{ "facs-twice", "twice" },
	{ "facs-ids-not-table", "ids" },
	{ "facs-frames-not-table", "frames" },
	{ "facs-id-text", "text" },
} do
	try(Case[1], function()
		local Mesh, B = rig(3)
		local Unit, Ids, Frames = Units[2], { B[1] }, { CFrame.new(1, 0, 0) }
		if Case[2] == "frame" then
			Frames = { Vector3.one }
		elseif Case[2] == "name" then
			Unit = Units[2].Name
		elseif Case[2] == "number" then
			Unit = Units[2].Value
		elseif Case[2] == "nil" then
			Unit = nil
		elseif Case[2] == "twice" then
			Ids, Frames = { B[1], B[1] }, { CFrame.new(1, 0, 0), CFrame.new(2, 0, 0) }
		elseif Case[2] == "ids" then
			Ids = 5
		elseif Case[2] == "frames" then
			Frames = 5
		elseif Case[2] == "text" then
			Ids = { tostring(B[1]) }
		end
		local Result = caught(Mesh.SetFacsPose, Mesh, Unit, Ids, Frames)
		return Result, show(table.pack(Mesh:GetFacsPose(Units[2])))
	end)
end
try("facs-get-errors", function()
	local Mesh = rig(2)
	return caught(Mesh.GetFacsPose, Mesh, nil), caught(Mesh.GetFacsPose, Mesh, "Nope"), caught(Mesh.GetFacsCorrectivePose, Mesh, { Units[1] }),
		caught(Mesh.GetFacsCorrectivePose, Mesh, { Units[1], Units[1] }), caught(Mesh.GetFacsCorrectivePose, Mesh, nil)
end)
try("facs-corrective", function()
	local Mesh, B = rig(4)
	Mesh:SetFacsCorrectivePose({ Units[3], Units[1] }, { B[2] }, { CFrame.new(0, 1, 0) })
	Mesh:SetFacsCorrectivePose({ Units[5], Units[2], Units[4] }, { B[1], B[3] }, { CFrame.new(1, 0, 0), CFrame.new(3, 0, 0) })
	return show(Mesh:GetFacsCorrectivePoses()), show(table.pack(Mesh:GetFacsCorrectivePose({ Units[1], Units[3] }))), show(table.pack(Mesh:GetFacsCorrectivePose({ Units[3], Units[1] }))),
		show(table.pack(Mesh:GetFacsCorrectivePose({ Units[2], Units[4], Units[5] }))), show(table.pack(Mesh:GetFacsCorrectivePose({ Units[1], Units[2] })))
end)
for Index, Pair in { { 1, 2 }, { 1, 30 }, { 7, 8 }, { 20, 21 }, { 40, 41 }, { 1, 2, 3 } } do
	try("corrective-pair-" .. Index, function()
		local Mesh, B = rig(2)
		local Actions = {}
		for _, U in Pair do
			table.insert(Actions, Units[U])
		end
		Mesh:SetFacsCorrectivePose(Actions, { B[1] }, { CFrame.new(1, 0, 0) })
		return show(Mesh:GetFacsCorrectivePoses())
	end)
end
try("facs-copy", function()
	local Mesh, B = rig(3)
	Mesh:SetFacsPose(Units[2], { B[1] }, { CFrame.new(1, 0, 0) })
	Mesh:SetFacsCorrectivePose({ Units[1], Units[2] }, { B[2] }, { CFrame.new(0, 1, 0) })
	local Copy = AssetService:CreateEditableMeshAsync(Content.fromObject(Mesh), { FixedSize = false })
	local Line = show(Copy:GetFacsPoses()) .. " " .. show(table.pack(Copy:GetFacsPose(Units[2]))) .. " " .. show(Copy:GetFacsCorrectivePoses())
	Copy:Destroy()
	return Line
end)
try("facs-remove-bone", function()
	local Mesh, B = rig(3)
	Mesh:SetFacsPose(Units[2], { B[1], B[2] }, { CFrame.new(1, 0, 0), CFrame.new(2, 0, 0) })
	Mesh:RemoveBone(B[1])
	return show(table.pack(Mesh:GetFacsPose(Units[2])))
end)
try("facs-unvirtual", function()
	local Mesh, B = rig(3)
	Mesh:SetFacsPose(Units[2], { B[1] }, { CFrame.new(1, 0, 0) })
	Mesh:SetBoneIsVirtual(B[1], false)
	return show(table.pack(Mesh:GetFacsPose(Units[2])))
end)

-- bone names: the longest, and a name another bone has
for _, Length in { 99, 100, 127, 128, 255, 256 } do
	try("bone-name-len " .. Length, function()
		local Mesh = rig(1)
		local Ok, Message = pcall(Mesh.AddBone, Mesh, { Name = string.rep("y", Length), CFrame = CFrame.new(), Virtual = false })
		return Ok, if Ok then "added" else P.err(Message)
	end)
end
try("bone-name-own", function()
	local Mesh, B = rig(2)
	Mesh:SetBoneName(B[1], "b1")
	return Mesh:GetBoneName(B[1])
end)
try("bone-name-removed", function()
	local Mesh, B = rig(2)
	Mesh:RemoveBone(B[1])
	return Mesh:AddBone({ Name = "b1", CFrame = CFrame.new(), Virtual = false })
end)
try("bone-name-unicode", function()
	local Mesh, B = rig(2)
	Mesh:SetBoneName(B[1], "héllo")
	return Mesh:GetBoneName(B[1]), Mesh:GetBoneByName("héllo")
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

-- bone names: the longest, in bytes or characters
for _, Case in { { "ascii-101", string.rep("y", 101) }, { "ascii-110", string.rep("y", 110) }, { "ascii-126", string.rep("y", 126) }, { "e-50", string.rep("é", 50) }, { "e-51", string.rep("é", 51) } } do
	try("bone-len-" .. Case[1], function()
		local Mesh = mesh({})
		local Ok, Message = pcall(Mesh.AddBone, Mesh, { Name = Case[2], CFrame = CFrame.new(), Virtual = false })
		return Ok, if Ok then "added" else P.err(Message)
	end)
end
try("bone-set-len", function()
	local Mesh = mesh({})
	local B = Mesh:AddBone({ Name = "a", CFrame = CFrame.new(), Virtual = false })
	return caught(Mesh.SetBoneName, Mesh, B, string.rep("y", 100)), caught(Mesh.SetBoneName, Mesh, B, string.rep("y", 101))
end)
try("bone-by-name-table", function()
	local Mesh = mesh({})
	Mesh:AddBone({ Name = "{}", CFrame = CFrame.new(), Virtual = false })
	return Mesh:GetBoneByName({}), Mesh:GetBoneByName(true)
end)
