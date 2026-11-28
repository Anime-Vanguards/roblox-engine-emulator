-- CollectionService:CreateCollection, fifth round: OnChanged around parent changes, callbacks assigned twice, Destroy's OnRemoved
P.sync()
local CS = game:GetService("CollectionService")
local Root = Instance.new("Folder")
Root.Parent = workspace
local Inner = Instance.new("Folder")
Inner.Parent = Root
local Log = {}
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
local A = part("A", Root, "Fifth")
local Outside = part("O", nil, "Fifth")
local H = CS:CreateCollection(".Fifth", Root)
local Handle: any = H
for _, Name in { "OnAdded", "OnRemoved", "OnChanged" } do
	Handle[Name] = function(Instance, Property)
		table.insert(Log, Name .. "(" .. Instance.Name .. (if Property then "," .. tostring(Property) else "") .. ")")
	end
end
for _, Target in { A, Outside } do
	Target.Changed:Connect(function(Property)
		table.insert(Log, Target.Name .. ".Changed(" .. Property .. ")")
	end)
end
local function flush(Tag)
	task.wait()
	print(Tag, table.concat(Log, " "))
	table.clear(Log)
end
flush("start")
A.Parent = nil
A.Parent = Root
flush("member out and back")
Outside.Parent = Root
Outside.Parent = nil
Outside.Parent = Root
flush("outsider in, out, in")
A.Parent = Inner
A.Parent = Root
flush("moves inside")
A.Name = "A1"
A.Parent = nil
flush("write then out")
A.Parent = Root
A.Name = "A2"
A.Parent = nil
A.Parent = Root
flush("in, write, out, in")
A:RemoveTag("Fifth")
A:AddTag("Fifth")
A.Name = "A3"
flush("tag off and on, write")
for _, Name in { "OnAdded", "OnRemoved", "OnChanged", "OnItemAdded" } do
	print("assign again", Name, P.try(function() Handle[Name] = function() end end))
end
print("assign item twice", P.try(function() Handle.OnItemAdded = function() end end))
H:Destroy()
table.insert(Log, "now")
flush("destroy")
print("assign destroyed", P.try(function() Handle.OnItemRemoved = function() end end))
local H2 = CS:CreateCollection(".Fifth", Root)
local Handle2: any = H2
task.wait()
Handle2.OnRemoved = function(Instance)
	table.insert(Log, "second OnRemoved(" .. Instance.Name .. ")")
end
H2:Destroy()
flush("destroy, callback set late")
Root:Destroy()
Outside:Destroy()
