-- CollectionService:CreateCollection, fourth round: what OnChanged reports, OnItemAdded / OnItemRemoved, the callbacks' order
P.sync()
local CS = game:GetService("CollectionService")
local Root = Instance.new("Folder")
Root.Parent = workspace
local Log = {}
local function show(...)
	local Parts = {}
	for Index = 1, select("#", ...) do
		local Value = select(Index, ...)
		table.insert(Parts, if typeof(Value) == "Instance" then Value.Name else typeof(Value) .. ":" .. tostring(Value))
	end
	return table.concat(Parts, ",")
end
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
local A = part("A", Root, "Fourth")
local B = part("B", Root, "Fourth")
local H = CS:CreateCollection(".Fourth", Root)
for _, Name in { "OnAdded", "OnRemoved", "OnChanged", "OnItemAdded", "OnItemRemoved" } do
	(H :: any)[Name] = function(...)
		table.insert(Log, Name .. "(" .. show(...) .. ")")
	end
end
local function flush(Tag)
	task.wait()
	print(Tag, table.concat(Log, " "))
	table.clear(Log)
end
flush("after assigning")
A.Changed:Connect(function(Name)
	table.insert(Log, "A.Changed(" .. Name .. ")")
end)
A.Name = "A2"
flush("rename")
A.Transparency = 0.5
A.Transparency = 0.25
flush("two writes")
A:SetAttribute("Mark", 1)
flush("attribute")
A:AddTag("Other")
flush("other tag")
A.Transparency = 0.25
flush("same value")
local Child = Instance.new("Folder")
Child.Parent = A
flush("child")
local C = part("C", Root, nil)
C.Name = "C2"
flush("non-member write")
local D = part("D", Root, "Fourth")
D.Name = "D2"
flush("add then write")
B.Name = "B2"
B:RemoveTag("Fourth")
flush("write then remove")
local E = part("E", nil, "Fourth")
local F = part("F", nil, "Fourth")
E.Parent = Root
F.Parent = Root
flush("two added")
E.Parent = nil
F.Parent = nil
flush("two removed")
-- a second assignment replaces the first
local Handle: any = H
Handle.OnAdded = function(Instance)
	table.insert(Log, "second OnAdded(" .. Instance.Name .. ")")
end
E.Parent = Root
flush("replaced")
H:Destroy()
flush("destroyed")
Root:Destroy()
