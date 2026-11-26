-- RaycastParams, OverlapParams and CatalogSearchParams members as a script writes them (prelude.luau): the instance lists'
-- casts (nested tables read in their place, holes skipped, what an error leaves), the other members' casts and checks,
-- AddToFilter, what new() does with a table, tostring, and what the queries make of nested lists and MaxParts. The scene is
-- 300 studs out.
P.sync()
local Root = Instance.new("Folder")
Root.Name = "Root"
Root.Parent = workspace
local function Part(Name, Position)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = true
	Made.Position = Position
	Made.Size = Vector3.new(10, 1, 10)
	Made.Parent = Root
	return Made
end
local A = Part("A", Vector3.new(300, 90, 300))
local B = Part("B", Vector3.new(300, 80, 300))
local Inner = Instance.new("Folder")
Inner.Name = "Inner"
Inner.Parent = Root

local function Names(List)
	if type(List) ~= "table" then
		return typeof(List) .. ":" .. tostring(List)
	end
	local Out = {}
	for Key, Value in List do
		local Text = if typeof(Value) == "Instance" then Value.Name elseif type(Value) == "table" then "{" .. Names(Value) .. "}" else typeof(Value) .. ":" .. tostring(Value)
		table.insert(Out, tostring(Key) .. "=" .. Text)
	end
	return "#" .. #List .. " " .. table.concat(Out, ",")
end
local function Hit(Params)
	local Result = workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -30, 0), Params)
	return if Result then Result.Instance.Name else "nil"
end

for _, Kind in { "RaycastParams", "OverlapParams" } do
	for _, Field in { "FilterDescendantsInstances", "ExcludeInstances", "IncludeInstances" } do
		local Cases = {
			{ "{1}", function() return { 1 } end },
			{ "{A,1}", function() return { A, 1 } end },
			{ "{x}", function() return { "x" } end },
			{ "{A,{B}}", function() return { A, { B } } end },
			{ "{{}}", function() return { {} } end },
			{ "{A,nil,B}", function() return { A, nil, B } end },
			{ "{[1]=A,[3]=B}", function() return { [1] = A, [3] = B } end },
			{ "{k=A}", function() return { k = A } end },
			{ "{A,A}", function() return { A, A } end },
			{ "{A,k=B}", function() return { A, k = B } end },
			{ "5", function() return 5 end },
			{ "x", function() return "x" end },
			{ "A", function() return A end },
			{ "true", function() return true end },
			{ "nil", function() return nil end },
			{ "{}", function() return {} end },
			{ "{V}", function() return { Vector3.zero } end },
			{ "{A,{B,{Inner}}}", function() return { A, { B, { Inner } } } end },
		}
		for _, Case in Cases do
			local Params = if Kind == "RaycastParams" then RaycastParams.new() else OverlapParams.new()
			Params[Field] = { B }
			local Ok, Why = pcall(function()
				Params[Field] = Case[2]()
			end)
			local ReadOk, Read = pcall(function()
				return Params[Field]
			end)
			print(Kind, Field, Case[1], if Ok then "ok" else "error " .. P.err(Why), "read", if ReadOk then Names(Read) else "error " .. P.err(Read))
		end
	end
end

-- what a query makes of nested and holed lists
for _, Case in {
	{ "exclude {{A}}", "Exclude", { { A } } },
	{ "exclude {B,{A}}", "Exclude", { B, { A } } },
	{ "exclude {A,nil,B}", "Exclude", { A, nil, B } },
	{ "exclude {[2]=A}", "Exclude", { [2] = A } },
	{ "include {{B}}", "Include", { { B } } },
	{ "include {{{B}}}", "Include", { { { B } } } },
} do
	local Params = RaycastParams.new()
	Params.FilterType = Enum.RaycastFilterType[Case[2]]
	local Ok, Why = pcall(function()
		Params.FilterDescendantsInstances = Case[3]
	end)
	print("query", Case[1], if Ok then "ok" else "error " .. P.err(Why), Hit(Params), Names(Params.FilterDescendantsInstances))
	local Params2 = RaycastParams.new()
	pcall(function()
		Params2.ExcludeInstances = Case[3]
	end)
	print("query-exclude-list", Case[1], Hit(Params2), Names(Params2.ExcludeInstances))
end

-- new(table)
print("new IgnoreWater", RaycastParams.new({ IgnoreWater = true }).IgnoreWater)
print("new Filter", Names(RaycastParams.new({ FilterDescendantsInstances = { A } }).FilterDescendantsInstances))
print("new Overlap MaxParts", OverlapParams.new({ MaxParts = 3 }).MaxParts)
print("new Overlap Exclude", Names(OverlapParams.new({ ExcludeInstances = { A } }).ExcludeInstances))
print("new Raycast Include", Names(RaycastParams.new({ IncludeInstances = { A } }).IncludeInstances))
print("new 5", P.try(function() return typeof(RaycastParams.new(5)) end))
print("new x", P.try(function() return typeof(OverlapParams.new("x")) end))
print("new nil,1", P.try(function() return typeof(RaycastParams.new(nil, 1)) end))
print("new bad key", P.try(function() return typeof(RaycastParams.new({ Nope = 1 })) end))
print("new FilterType", tostring(RaycastParams.new({ FilterType = Enum.RaycastFilterType.Include }).FilterType))
print("new hit", Hit(RaycastParams.new({ FilterDescendantsInstances = { A } })))

-- AddToFilter
for _, Case in {
	{ "5", function() return 5 end },
	{ "{5}", function() return { 5 } end },
	{ "{A,5}", function() return { A, 5 } end },
	{ "{A,{B}}", function() return { A, { B } } end },
	{ "nil", function() return nil end },
	{ "{}", function() return {} end },
	{ "{A,nil,B}", function() return { A, nil, B } end },
	{ "A", function() return A end },
	{ "x", function() return "x" end },
	{ "{k=A}", function() return { k = A } end },
} do
	local Params = RaycastParams.new()
	Params.FilterDescendantsInstances = { Inner }
	local Ok, Why = pcall(function()
		Params:AddToFilter(Case[2]())
	end)
	print("AddToFilter", Case[1], if Ok then "ok" else "error " .. P.err(Why), Names(Params.FilterDescendantsInstances))
end
local Params = OverlapParams.new()
print("AddToFilter none", P.try(function() Params:AddToFilter() end), Names(Params.FilterDescendantsInstances))

-- other members' writes
for _, Case in {
	{ "IgnoreWater", 5 }, { "IgnoreWater", "x" }, { "IgnoreWater", nil }, { "RespectCanCollide", 1 }, { "CollisionGroup", 5 }, { "CollisionGroup", nil },
	{ "CollisionGroup", "Nope" }, { "FilterType", "Include" }, { "FilterType", 1 }, { "FilterType", Enum.KeyCode.A }, { "FilterType", nil },
	{ "Margin", "x" }, { "Margin", "2" }, { "BruteForceAllSlow", 1 },
} do
	local Params = RaycastParams.new()
	local Ok, Why = pcall(function()
		Params[Case[1]] = Case[2]
	end)
	print("write RaycastParams", Case[1], tostring(Case[2]), if Ok then "ok" else "error " .. P.err(Why), tostring(Params[Case[1]]))
end
for _, Case in { { "MaxParts", "x" }, { "MaxParts", "3" }, { "MaxParts", 2.7 }, { "MaxParts", -1 }, { "Tolerance", "x" }, { "MaxParts", nil } } do
	local Params = OverlapParams.new()
	local Ok, Why = pcall(function()
		Params[Case[1]] = Case[2]
	end)
	print("write OverlapParams", Case[1], tostring(Case[2]), if Ok then "ok" else "error " .. P.err(Why), tostring(Params[Case[1]]))
end
Root:Destroy()
do
	P.sync()
	local Root = Instance.new("Folder")
	Root.Parent = workspace
	local function Part(Name, Position)
		local Made = Instance.new("Part")
		Made.Name = Name
		Made.Anchored = true
		Made.Position = Position
		Made.Size = Vector3.new(4, 1, 4)
		Made.Parent = Root
		return Made
	end
	local A = Part("A", Vector3.new(300, 90, 300))
	local B = Part("B", Vector3.new(300, 80, 300))
	local C = Part("C", Vector3.new(300, 70, 300))

	local function Value(V)
		return typeof(V) .. ":" .. (if typeof(V) == "table" then "#" .. #V else tostring(V))
	end
	for _, Kind in { "RaycastParams", "OverlapParams" } do
		for _, Case in {
			{ "CollisionGroup", 5 }, { "CollisionGroup", 5.5 }, { "CollisionGroup", true }, { "CollisionGroup", Vector3.one }, { "CollisionGroup", {} }, { "CollisionGroup", A },
			{ "RespectCanCollide", nil }, { "RespectCanCollide", {} }, { "RespectCanCollide", 0 }, { "RespectCanCollide", "" },
			{ "BruteForceAllSlow", nil }, { "BruteForceAllSlow", "x" },
			{ "FilterType", Enum.RaycastFilterType.Include }, { "FilterType", Enum.RaycastFilterType.Whitelist }, { "FilterType", Enum.RaycastFilterType.Blacklist },
			{ "FilterType", 0 }, { "FilterType", "Exclude" }, { "FilterType", {} },
			{ "IgnoreWater", 0 }, { "IgnoreWater", {} },
			{ "Margin", nil }, { "Margin", true }, { "Margin", 1e40 }, { "Margin", -1 },
			{ "SizeGradient", "3" }, { "SizeGradient", nil },
			{ "MaxParts", true }, { "MaxParts", 1e10 }, { "MaxParts", -2.5 }, { "MaxParts", 0 / 0 }, { "MaxParts", {} }, { "MaxParts", "2.9" }, { "MaxParts", 2 ^ 31 },
			{ "Tolerance", nil }, { "Tolerance", "1.5" }, { "Tolerance", true },
		} do
			local Params = if Kind == "RaycastParams" then RaycastParams.new() else OverlapParams.new()
			local Ok, Why = pcall(function()
				Params[Case[1]] = Case[2]
			end)
			local ReadOk, Read = pcall(function()
				return Params[Case[1]]
			end)
			print(Kind, Case[1], Value(Case[2]), if Ok then "ok" else "error " .. P.err(Why), "read", if ReadOk then Value(Read) else "error " .. P.err(Read))
		end
	end

	-- MaxParts in a query: -1, 1, 2
	for _, Count in { -1, 1, 2, 0 } do
		local Params = OverlapParams.new()
		Params.MaxParts = Count
		print("MaxParts", Count, #workspace:GetPartBoundsInBox(CFrame.new(300, 80, 300), Vector3.new(2, 40, 2), Params))
	end
	-- FilterType Whitelist / Blacklist
	for _, Item in { Enum.RaycastFilterType.Whitelist, Enum.RaycastFilterType.Blacklist } do
		local Params = RaycastParams.new()
		Params.FilterType = Item
		Params.FilterDescendantsInstances = { A }
		local Result = workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -50, 0), Params)
		print("FilterType", tostring(Params.FilterType), Result and Result.Instance.Name)
	end
	-- deep nesting and a cycle
	do
		local Deep = { A }
		for _ = 1, 50 do
			Deep = { Deep }
		end
		local Params = RaycastParams.new()
		print("deep 51", P.try(function() Params.FilterDescendantsInstances = Deep end), #Params.FilterDescendantsInstances)
		local Deeper = { A }
		for _ = 1, 300 do
			Deeper = { Deeper }
		end
		print("deep 301", P.try(function() Params.FilterDescendantsInstances = Deeper end), #Params.FilterDescendantsInstances)
	end
	do
		local Cycle = { A }
		Cycle[2] = Cycle
		local Params = RaycastParams.new()
		print("cycle", P.try(function() Params.FilterDescendantsInstances = Cycle end), #Params.FilterDescendantsInstances)
	end
	-- the list keeps the instances it read before a bad entry; the new() table; writes of the old lists after errors
	do
		local Params = RaycastParams.new()
		Params.IncludeInstances = { A }
		pcall(function() Params.IncludeInstances = 5 end)
		local Result = workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -50, 0), Params)
		print("include after error", Value(Params.IncludeInstances), Result and Result.Instance.Name)
		Params.IncludeInstances = nil
		Result = workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -50, 0), Params)
		print("include nil", Value(Params.IncludeInstances), Result and Result.Instance.Name)
		pcall(function() Params.IncludeInstances = { B, 1, A } end)
		Result = workspace:Raycast(Vector3.new(300, 100, 300), Vector3.new(0, -50, 0), Params)
		print("include partial", Value(Params.IncludeInstances), Result and Result.Instance.Name)
	end
	-- a list read twice is two tables; the list holds a copy
	do
		local Params = RaycastParams.new()
		local List = { A }
		Params.FilterDescendantsInstances = List
		table.insert(List, B)
		print("copy", #Params.FilterDescendantsInstances, Params.FilterDescendantsInstances == Params.FilterDescendantsInstances)
		local Read = Params.FilterDescendantsInstances
		table.insert(Read, C)
		print("read copy", #Params.FilterDescendantsInstances)
	end
	-- tostring with lists
	do
		local Params = RaycastParams.new()
		Params.FilterDescendantsInstances = { A, { B } }
		print("tostring nested", tostring(Params))
		Params.ExcludeInstances = { A }
		Params.CollisionGroup = 5
		print("tostring exclude", tostring(Params))
		local Overlap = OverlapParams.new()
		Overlap.MaxParts = 3
		Overlap.IncludeInstances = { A, B }
		print("tostring overlap", tostring(Overlap))
	end
	-- AddToFilter: nested, holes, cycle, and the copy
	do
		local Params = RaycastParams.new()
		Params:AddToFilter({ A, { B, { C } } })
		print("add nested", #Params.FilterDescendantsInstances)
		local Cycle = { A }
		Cycle[2] = Cycle
		print("add cycle", P.try(function() Params:AddToFilter(Cycle) end), #Params.FilterDescendantsInstances)
		print("add 2", P.try(function() Params:AddToFilter(A, B) end), #Params.FilterDescendantsInstances)
		print("add dot", P.try(function() Params.AddToFilter(Params, A) end), #Params.FilterDescendantsInstances)
		print("add wrong self", P.try(function() Params.AddToFilter(OverlapParams.new(), A) end), #Params.FilterDescendantsInstances)
		print("add no self", P.try(function() Params.AddToFilter(A) end))
		print("add 5 self", P.try(function() Params.AddToFilter(5, A) end))
	end
	Root:Destroy()
end
do
	local A = Instance.new("Folder")
	A.Name = "A"
	local function Nest(Depth)
		local List = { A }
		for _ = 2, Depth do
			List = { List }
		end
		return List
	end
	local Params = RaycastParams.new()
	local Last
	for _, Depth in { 100, 256, 297, 298, 300 } do
		Params.FilterDescendantsInstances = {}
		local Text = P.try(function() Params.FilterDescendantsInstances = Nest(Depth) end)
		print("depth", Depth, Text, #Params.FilterDescendantsInstances)
	end
	for _, Depth in { 100, 298, 299 } do
		local Fresh = RaycastParams.new()
		print("add depth", Depth, P.try(function() Fresh:AddToFilter(Nest(Depth)) end), #Fresh.FilterDescendantsInstances)
	end
	do
		local Shared = { A }
		print("dag", P.try(function() Params.FilterDescendantsInstances = { Shared, Shared } end), #Params.FilterDescendantsInstances)
		print("dag nested", P.try(function() Params.FilterDescendantsInstances = { Shared, { Shared } } end), #Params.FilterDescendantsInstances)
		local Self = { A }
		Self[2] = { Self }
		print("cycle deep", P.try(function() Params.FilterDescendantsInstances = Self end), #Params.FilterDescendantsInstances)
		print("exclude dag", P.try(function() Params.ExcludeInstances = { Shared, Shared } end), #Params.ExcludeInstances)
	end
	for _, Case in { { "Tolerance", 0.05 }, { "Tolerance", 0.0500001 }, { "Tolerance", -0.01 }, { "Tolerance", 0 / 0 }, { "Tolerance", "0.01" } } do
		local Overlap = OverlapParams.new()
		print("overlap", Case[1], tostring(Case[2]), P.try(function() Overlap[Case[1]] = Case[2] end), Overlap[Case[1]])
	end
	for _, Case in { { "SizeGradient", -1 }, { "SizeGradient", 1e40 }, { "SizeGradient", 0 / 0 }, { "Margin", 0 }, { "Margin", 0 / 0 }, { "Margin", 1e30 }, { "Margin", 2.5 } } do
		local Ray = RaycastParams.new()
		print("ray", Case[1], tostring(Case[2]), P.try(function() Ray[Case[1]] = Case[2] end), Ray[Case[1]])
	end
	-- tostring of numbers set and the lists' text
	do
		local Ray = RaycastParams.new()
		Ray.Margin = 1 / 3
		Ray.SizeGradient = 2.5
		Ray.FilterDescendantsInstances = { A }
		print("tostring", tostring(Ray))
		local Overlap = OverlapParams.new()
		Overlap.MaxParts = -7
		Overlap.Tolerance = 0.01
		print("tostring", tostring(Overlap))
	end
	-- CatalogSearchParams members
	for _, Case in { { "MaxPrice", 1e12 }, { "MaxPrice", -5 }, { "MinPrice", 0 / 0 }, { "CreatorId", -3 }, { "CreatorId", 2.7 }, { "CreatorId", "x" }, { "MinPrice", true }, { "MinPrice", nil } } do
		local Catalog = CatalogSearchParams.new()
		print("catalog", Case[1], tostring(Case[2]), P.try(function() Catalog[Case[1]] = Case[2] end), Catalog[Case[1]])
	end
end
do
	local function Show(Label, Build)
		local Ok, Text = pcall(function()
			local Params = CatalogSearchParams.new()
			Build(Params)
			return tostring(Params)
		end)
		print(Label, if Ok then Text else "error " .. P.err(Text))
	end
	Show("new", function() end)
	Show("MinPrice 5", function(P) P.MinPrice = 5 end)
	Show("MinPrice 0", function(P) P.MinPrice = 0 end)
	Show("MaxPrice 100", function(P) P.MaxPrice = 100 end)
	Show("MaxPrice default", function(P) P.MaxPrice = 2147483647 end)
	Show("CreatorId 7", function(P) P.CreatorId = 7 end)
	Show("CreatorId 0", function(P) P.CreatorId = 0 end)
	Show("all", function(P) P.CreatorId = 7 P.MaxPrice = 100 P.MinPrice = 5 end)
	Show("all reversed", function(P) P.MinPrice = 5 P.MaxPrice = 100 P.CreatorId = 7 end)
	Show("MinPrice 5 then 0", function(P) P.MinPrice = 5 P.MinPrice = 0 end)
	Show("MinPrice 2.5", function(P) P.MinPrice = 2.5 end)
	Show("MinPrice -1", function(P) P.MinPrice = -1 end)
	Show("MinPrice x", function(P) P.MinPrice = "x" end)
	Show("MinPrice '3'", function(P) P.MinPrice = "3" end)
	Show("CreatorId 1e12", function(P) P.CreatorId = 1e12 end)
	Show("MaxPrice 1e12", function(P) P.MaxPrice = 1e12 end)
	Show("keyword+min", function(P) P.SearchKeyword = "hat" P.MinPrice = 5 end)
	Show("Limit 10", function(P) P.Limit = 10 end)
	Show("IncludeOffSale", function(P) P.IncludeOffSale = true end)
	Show("CreatorName", function(P) P.CreatorName = "x" end)
	Show("AssetTypes", function(P) P.AssetTypes = { Enum.AvatarAssetType.Hat, Enum.AvatarAssetType.Shirt } end)
	Show("BundleTypes", function(P) P.BundleTypes = { Enum.BundleType.BodyParts } end)
	local Params = CatalogSearchParams.new()
	Params.MinPrice = 5
	print("read", Params.MinPrice, Params.MaxPrice, Params.CreatorId)
	Show("new({MinPrice=5})", function() end)
	print("new table", tostring(CatalogSearchParams.new({ MinPrice = 5 })))
end
