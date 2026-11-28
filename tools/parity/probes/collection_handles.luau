-- CollectionService:CreateCollection: the handle, its members, its count and when the count follows the tree
P.sync()
local CS = game:GetService("CollectionService")
local function show(Value)
	if typeof(Value) == "Instance" then
		return Value:GetFullName()
	end
	return typeof(Value) .. " " .. tostring(Value)
end
local Root = Instance.new("Folder")
Root.Name = "CollectionRoot"
Root.Parent = workspace
local Inner = Instance.new("Folder")
Inner.Name = "Inner"
Inner.Parent = Root
local function part(Name, Parent, Tag)
	local Made = Instance.new("Part")
	Made.Name = Name
	Made.Anchored = true
	if Tag then
		Made:AddTag(Tag)
	end
	Made.Parent = Parent
	return Made
end
local A = part("A", Root, "CTag")
local B = part("B", Inner, "CTag")
local C = part("C", Root, nil)
Root:AddTag("CTag")

local H = CS:CreateCollection(".CTag", Root)
print("type", typeof(H), tostring(H), H == H)
print("query", P.try(function() return show(H.Query) end))
print("root", P.try(function() return show(H.Root) end))
print("count now", P.try(function() return H:GetCount() end))
task.wait()
print("count after a wait", P.try(function() return H:GetCount() end))
for _, Name in { "Query", "Root", "GetCount", "Destroy", "Count", "Name", "ClassName", "Parent", "ItemAdded", "ItemRemoved", "Added", "Removed", "Changed", "CountChanged", "GetItems", "GetInstances", "GetTagged", "Items", "Instances", "Selector", "IsDestroyed", "Destroyed", "GetChildren", "Clone", "IsA", "Size", "Length", "query", "root", "getCount", "destroy" } do
	print("member", Name, P.try(function() return typeof((H :: any)[Name]) end))
end
print("write Query", P.try(function() (H :: any).Query = ".X" end), show(H.Query))
print("write Root", P.try(function() (H :: any).Root = workspace end), show(H.Root))
print("write new", P.try(function() (H :: any).Foo = 1 end))
print("len", P.try(function() return #(H :: any) end))
print("call", P.try(function() return (H :: any)() end))
print("iterate", P.try(function()
	local N = 0
	for _ in H :: any do
		N += 1
	end
	return N
end))
print("concat", P.try(function() return "x" .. (H :: any) end))
print("index number", P.try(function() return (H :: any)[1] end))

-- method calls on the wrong self
print("count self nil", P.try(function() return H.GetCount(nil) end))
print("count self part", P.try(function() return H.GetCount(A :: any) end))
print("count self number", P.try(function() return H.GetCount(5 :: any) end))
print("destroy self part", P.try(function() return H.Destroy(A :: any) end))
print("count dot", P.try(function() return (H :: any).GetCount() end))

-- the count as the tree changes: when it follows
local D = part("D", Root, nil)
D:AddTag("CTag")
print("add tag", H:GetCount())
task.defer(function()
	print("add tag deferred", H:GetCount())
end)
local Added = CS:GetInstanceAddedSignal("CTag"):Connect(function(Instance)
	print("added signal", Instance.Name, H:GetCount())
end)
local E = part("E", Root, nil)
E:AddTag("CTag")
print("add tag before signal", H:GetCount())
task.wait()
print("add tag after wait", H:GetCount())
Added:Disconnect()
D:RemoveTag("CTag")
print("remove tag", H:GetCount())
task.defer(function()
	print("remove tag deferred", H:GetCount())
end)
task.wait()
print("remove tag after wait", H:GetCount())
E.Parent = nil
print("unparent", H:GetCount())
task.wait()
print("unparent after wait", H:GetCount())
E.Parent = Inner
print("reparent", H:GetCount())
task.wait()
print("reparent after wait", H:GetCount())
E:AddTag("Other")
E:RemoveTag("CTag")
E:AddTag("CTag")
print("off and on", H:GetCount())
task.wait()
print("off and on after wait", H:GetCount())
local F = part("F", nil, "CTag")
F.Parent = Root
F.Parent = nil
print("in and out", H:GetCount())
task.wait()
print("in and out after wait", H:GetCount())
B:Destroy()
print("destroy member", H:GetCount())
task.wait()
print("destroy member after wait", H:GetCount())

-- selectors: a collection's count against QueryDescendants
local Sel = Instance.new("Folder")
Sel.Name = "Selectors"
local Q1 = part("Alpha", Sel, "STag")
local Q2 = part("Beta", Sel, nil)
local M = Instance.new("Model")
M.Name = "Alpha"
M:AddTag("STag")
M.Parent = Sel
local Q3 = part("Gamma", M, "STag")
Q2:SetAttribute("Flag", true)
Q3:SetAttribute("Flag", 2)
Sel.Parent = workspace
for _, Query in { "Part", ".STag", "#Alpha", "Part.STag", "Model > Part", "[$Flag]", "[$Flag = 2]", "[Name = Beta]", "BasePart", "Model, Part", "*", ".STag:not(Model)", "Folder", "" } do
	local ok, Made = pcall(CS.CreateCollection, CS, Query, Sel)
	if not ok then
		print("selector", Query, "error", P.err(Made))
	else
		local Now = Made:GetCount()
		task.wait()
		local Q = P.try(function() return #Sel:QueryDescendants(Query) end)
		print("selector", Query, Now, Made:GetCount(), Q)
		Made:Destroy()
	end
end
for _, Query in { "[", "Part >", "#", "..x", ":foo", "Part[Name=]" } do
	print("bad selector", Query, P.try(function()
		local Made = CS:CreateCollection(Query, Sel)
		return Made:GetCount()
	end))
end

-- a property or attribute selector follows writes
local Named = CS:CreateCollection("#Renamed", Sel)
local Flagged = CS:CreateCollection("[$Mark]", Sel)
task.wait()
print("prop start", Named:GetCount(), Flagged:GetCount())
Q2.Name = "Renamed"
Q1:SetAttribute("Mark", 1)
print("prop write", Named:GetCount(), Flagged:GetCount())
task.wait()
print("prop write after wait", Named:GetCount(), Flagged:GetCount())
Q2.Name = "Beta"
Q1:SetAttribute("Mark", nil)
task.wait()
print("prop back after wait", Named:GetCount(), Flagged:GetCount())

-- arguments
print("no args", P.try(function() return show(CS:CreateCollection()) end))
print("query only", P.try(function()
	local Made = CS:CreateCollection(".NoSuchTag")
	return show(Made.Root), Made:GetCount()
end))
print("root nil", P.try(function() return show(CS:CreateCollection(".NoSuchTag", nil).Root) end))
print("query number", P.try(function() return show(CS:CreateCollection(5 :: any, Root).Query) end))
print("query table", P.try(function() return show(CS:CreateCollection({} :: any, Root)) end))
print("query nil", P.try(function() return show(CS:CreateCollection(nil :: any, Root)) end))
print("root number", P.try(function() return show(CS:CreateCollection(".X", 5 :: any)) end))
print("root string", P.try(function() return show(CS:CreateCollection(".X", "Workspace" :: any)) end))
print("root table", P.try(function() return show(CS:CreateCollection(".X", {} :: any)) end))
print("root parentless", P.try(function()
	local Loose = Instance.new("Folder")
	part("L", Loose, "LTag")
	local Made = CS:CreateCollection(".LTag", Loose)
	task.wait()
	return show(Made.Root), Made:GetCount()
end))
print("dot call", P.try(function() return show(CS.CreateCollection(".X" :: any, Root :: any)) end))
print("other self", P.try(function() return show(CS.CreateCollection(workspace :: any, ".X", Root)) end))

-- GetCollections
local G1 = CS:CreateCollection(".CTag", Root)
task.wait()
local Holding = CS:GetCollections(A)
print("collections of A", typeof(Holding), #Holding)
local Found = 0
for _, Handle in Holding do
	if Handle == H then
		Found += 1
	end
	if Handle == G1 then
		Found += 10
	end
end
print("collections found", Found)
print("collections of C", #CS:GetCollections(C), "root", #CS:GetCollections(Root))
print("collections bad", P.try(function() return #CS:GetCollections(5 :: any) end), P.try(function() return #CS:GetCollections() end))
G1:Destroy()
print("collections after destroy", #CS:GetCollections(A))

-- two handles over one query
local H2 = CS:CreateCollection(".CTag", Root)
print("two handles", H2 == H, rawequal(H2, H))
H2:Destroy()

-- Destroy
print("destroy", P.try(function() return H:Destroy() end))
print("count destroyed", P.try(function() return H:GetCount() end))
print("query destroyed", P.try(function() return show(H.Query) end))
print("root destroyed", P.try(function() return show(H.Root) end))
print("destroy again", P.try(function() return H:Destroy() end))
print("tostring destroyed", tostring(H), typeof(H))

-- the root destroyed
local R2 = Instance.new("Folder")
R2.Parent = workspace
part("R", R2, "RTag")
local H3 = CS:CreateCollection(".RTag", R2)
task.wait()
print("root alive", H3:GetCount())
R2:Destroy()
print("root gone", P.try(function() return H3:GetCount() end), P.try(function() return show(H3.Root) end))
task.wait()
print("root gone after wait", P.try(function() return H3:GetCount() end), P.try(function() return show(H3.Root) end))
H3:Destroy()

-- the Workspace by default
local W = CS:CreateCollection(".WTag")
local W1 = part("W1", workspace, "WTag")
task.wait()
print("workspace default", show(W.Root), W:GetCount())
W:Destroy()
W1:Destroy()
Root:Destroy()
Sel:Destroy()
