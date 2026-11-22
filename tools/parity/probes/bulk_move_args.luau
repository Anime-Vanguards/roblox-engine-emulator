-- WorldRoot:BulkMoveTo's argument checks (world/mod.rs bulk_moves): the frame list first, an Array; then the part list, a list
-- of Instances whose entries are cast before the lengths are compared; values named in the casts as world/camera.rs cast_name
-- names them; and the mode
P.sync()
local Part = Instance.new("Part")
Part.Anchored = true
Part.CFrame = CFrame.new(200, 100, 200)
Part.Parent = workspace
local Cases = {
	{ "none", {} }, { "nil", { nil } }, { "nil nil", { nil, nil } }, { "list nil", { {}, nil } }, { "nil list", { nil, {} } },
	{ "list only", { {} } }, { "string string", { "x", "y" } }, { "Instance Instance", { workspace, workspace } },
	{ "string list", { "x", {} } }, { "list string", { {}, "x" } }, { "dictionary list", { { a = Part }, {} } },
	{ "dictionary dictionary", { { a = Part }, { a = CFrame.new() } } }, { "part, string", { { Part }, "x" } },
	{ "bad entry, string", { { 1 }, "x" } }, { "bad entry, dictionary", { { 1 }, { a = 1 } } },
	{ "part, no frames", { { Part }, {} } }, { "no parts, a frame", { {}, { CFrame.new() } } },
	{ "vector entry, no frames", { { Vector3.one }, {} } }, { "Instance list", { workspace, {} } }, { "Instance nil", { workspace, nil } },
	{ "nil Instance", { nil, workspace } }, { "KeyCode list", { Enum.KeyCode.A, {} } }, { "list KeyCode", { {}, Enum.KeyCode.A } },
	{ "Vector3 list", { Vector3.one, {} } }, { "function list", { print, {} } }, { "list function", { {}, print } },
	{ "hole", { { Part, nil, Part }, { CFrame.new(200, 101, 200), CFrame.new(), CFrame.new(200, 102, 200) } } },
}
for _, C in Cases do
	print("bulk", C[1], P.try(workspace.BulkMoveTo, workspace, table.unpack(C[2], 1, 3)))
end
for _, Entry in { 5, true, "x", workspace, Enum.KeyCode.A, Vector3.one, {}, print } do
	print("bulk frame entry", typeof(Entry), P.try(workspace.BulkMoveTo, workspace, { Part }, { Entry }))
end
for _, Mode in { "x", 0.5, 5, Enum.KeyCode.A, workspace, "FireCFrameChanged", 1, Enum.BulkMoveMode.FireAllEvents } do
	print("bulk mode", typeof(Mode), tostring(Mode), P.try(workspace.BulkMoveTo, workspace, { Part }, { CFrame.new(200, 100, 200) }, Mode))
end
print("bulk mode order", P.try(workspace.BulkMoveTo, workspace, "x", "y", "z"), P.try(workspace.BulkMoveTo, workspace, { 1 }, {}, "z"), P.try(workspace.BulkMoveTo, workspace, { Part }, {}, "z"))
