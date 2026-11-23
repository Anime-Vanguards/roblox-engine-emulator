-- Nil holes in the ignore lists of Camera:GetLargestCutoffDistance and GetPartsObscuringTarget (world/camera_moves.rs and
-- world/camera.rs ignore_list): a hole is skipped wherever it stands, and an entry that is no Instance raises, before a hole
-- or after one. The camera looks at nothing, so a call that goes on returns 0 (no parts).
P.sync()
local Cam = Instance.new("Camera")
Cam.Parent = workspace
Cam.CFrame = CFrame.new(0, 5000, 0)
Cam.Focus = CFrame.new(0, 5000, -10)
local Folder = Instance.new("Folder")
local function Holes(Count, Last)
	local List = table.create(Count + 1)
	List[Count + 1] = Last
	return List
end
for _, Case in {
	{ "hole", { workspace, nil, workspace } },
	{ "holes first", Holes(10, Folder) },
	{ "hole then number", { workspace, nil, 5 } },
	{ "number then hole", { 5, nil, workspace } },
	{ "holes then number", Holes(10, 5) },
	{ "holes then vector", Holes(3, Vector3.one) },
} do
	local Name, List = Case[1], Case[2]
	print("cutoff", Name, #List, P.try(Cam.GetLargestCutoffDistance, Cam, List))
	print("obscuring", Name, #List, P.try(function()
		return #Cam:GetPartsObscuringTarget({}, List)
	end))
end
