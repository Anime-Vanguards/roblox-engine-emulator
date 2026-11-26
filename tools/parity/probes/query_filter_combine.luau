-- How a query combines FilterType / FilterDescendantsInstances with the ExcludeInstances / IncludeInstances lists: every
-- combination over a scene of three parts in a row (two in folders of their own), through GetPartBoundsInBox (what passes)
-- and Raycast (the first that passes), with the lists naming a part, its folder, nothing or nil.
local Origin = Vector3.new(400, 60, 400)
local FolderA = Instance.new("Folder")
FolderA.Name = "FA"
local FolderB = Instance.new("Folder")
FolderB.Name = "FB"
local Parts = {}
for Index, Name in { "A", "B", "C" } do
	local Part = Instance.new("Part")
	Part.Name = Name
	Part.Anchored = true
	Part.Size = Vector3.new(2, 2, 2)
	Part.Position = Origin + Vector3.new(Index * 5, 0, 0)
	Part.Parent = if Name == "A" then FolderA elseif Name == "B" then FolderB else workspace
	Parts[Name] = Part
end
FolderA.Parent = workspace
FolderB.Parent = workspace
task.wait()
print("fields " .. P.try(function()
	local Params = RaycastParams.new()
	return typeof(Params.ExcludeInstances) .. " " .. typeof(Params.IncludeInstances)
end) .. " | " .. P.try(function()
	local Params = OverlapParams.new()
	return typeof(Params.ExcludeInstances) .. " " .. typeof(Params.IncludeInstances)
end))
local Lists = {
	none = nil,
	empty = {},
	FA = { FolderA },
	FB = { FolderB },
	A = { Parts.A },
	B = { Parts.B },
	AC = { Parts.A, Parts.C },
}
local Filters = { "none", "FA", "A" }
local Excludes = { "nil", "empty", "FA", "B", "A" }
local Includes = { "nil", "empty", "FA", "AC", "FB" }
local function Apply(Params, Type, Filter, Exclude, Include)
	Params.FilterType = Type
	if Filter ~= "none" then
		Params.FilterDescendantsInstances = Lists[Filter]
	end
	if Exclude ~= "nil" then
		Params.ExcludeInstances = Lists[Exclude]
	end
	if Include ~= "nil" then
		Params.IncludeInstances = Lists[Include]
	end
end
local Box = CFrame.new(Origin + Vector3.new(10, 0, 0))
for _, Type in { Enum.RaycastFilterType.Exclude, Enum.RaycastFilterType.Include } do
	for _, Filter in Filters do
		for _, Exclude in Excludes do
			for _, Include in Includes do
				local Label = Type.Name .. " fdi=" .. Filter .. " ex=" .. Exclude .. " in=" .. Include
				local Overlap = P.try(function()
					local Params = OverlapParams.new()
					Apply(Params, Type, Filter, Exclude, Include)
					local Names = {}
					for _, Part in workspace:GetPartBoundsInBox(Box, Vector3.new(20, 4, 4), Params) do
						table.insert(Names, Part.Name)
					end
					table.sort(Names)
					return table.concat(Names, ",")
				end)
				local Ray = P.try(function()
					local Params = RaycastParams.new()
					Apply(Params, Type, Filter, Exclude, Include)
					local Hit = workspace:Raycast(Origin, Vector3.new(30, 0, 0), Params)
					return if Hit then Hit.Instance.Name else "none"
				end)
				print("combine " .. Label .. " box " .. Overlap .. " ray " .. Ray)
			end
		end
	end
end
FolderA:Destroy()
FolderB:Destroy()
Parts.C:Destroy()
