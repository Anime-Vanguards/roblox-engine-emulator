-- CollectionService:CreateCollection, second round: the default root, where the count's updates fall in the deferred queue,
-- the root going away
P.sync()
local CS = game:GetService("CollectionService")
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
local Root = Instance.new("Folder")
Root.Name = "MoreRoot"
Root.Parent = workspace

-- the default root
local NoRoot = CS:CreateCollection(".MoreNone")
local NilRoot = CS:CreateCollection(".MoreNone", nil)
local WsRoot = CS:CreateCollection(".MoreNone", workspace)
print("defaults now", P.try(function() return NoRoot:GetCount() end), P.try(function() return NilRoot:GetCount() end), P.try(function() return WsRoot:GetCount() end))
task.defer(function()
	print("defaults deferred", P.try(function() return NoRoot:GetCount() end), P.try(function() return NilRoot:GetCount() end), P.try(function() return WsRoot:GetCount() end))
end)
task.wait()
print("defaults after a wait", P.try(function() return NoRoot:GetCount() end), P.try(function() return NilRoot:GetCount() end), P.try(function() return WsRoot:GetCount() end))
local OtherRoot = CS:CreateCollection(".MoreNone", Root)
task.wait()
print("own root after a wait", P.try(function() return OtherRoot:GetCount() end))
local Tagged = part("Tagged", workspace, "MoreWs")
local WsTagged = CS:CreateCollection(".MoreWs", workspace)
local NoRootTagged = CS:CreateCollection(".MoreWs")
print("workspace tagged now", P.try(function() return WsTagged:GetCount() end), P.try(function() return NoRootTagged:GetCount() end))
task.wait()
print("workspace tagged after a wait", P.try(function() return WsTagged:GetCount() end), P.try(function() return NoRootTagged:GetCount() end))
Tagged:Destroy()
pcall(function() WsTagged:Destroy() end)

-- members and writes
local H = CS:CreateCollection(".More", Root)
print("write function", P.try(function() (H :: any).Foo = function() end end), P.try(function() return typeof((H :: any).Foo) end))
print("write method", P.try(function() (H :: any).GetCount = function() end end))
print("write nil", P.try(function() (H :: any).Foo = nil end))

-- where the updates fall: around task.defer, in a deferred thread, against handlers connected before and after
local Log = {}
local function note(Tag)
	table.insert(Log, Tag .. "=" .. H:GetCount())
end
local BeforeConnection = CS:GetInstanceAddedSignal("More"):Connect(function()
	note("tag-handler")
end)
local ChildConnection = Root.ChildAdded:Connect(function()
	note("child-handler")
end)
task.defer(note, "defer-1")
local X = part("X", Root, "More")
task.defer(note, "defer-2")
note("now")
task.wait()
print("order", table.concat(Log, " "))
table.clear(Log)
task.defer(function()
	local Y = part("Y", Root, "More")
	note("inner-now")
	task.defer(note, "inner-defer")
end)
task.wait()
print("inner order", table.concat(Log, " "))
BeforeConnection:Disconnect()
ChildConnection:Disconnect()
local After = H:GetCount()
local Z = part("Z", nil, nil)
Z.Parent = Root
Z:AddTag("More")
table.clear(Log)
task.defer(note, "defer-z")
task.wait()
print("tag after parent", After, table.concat(Log, " "), H:GetCount())

-- moves inside the root, a clone, a rename under a name selector
local Inner = Instance.new("Folder")
Inner.Parent = Root
X.Parent = Inner
print("move inside", H:GetCount())
task.wait()
print("move inside after a wait", H:GetCount())
local Copy = X:Clone()
Copy.Parent = Root
print("clone", H:GetCount())
task.wait()
print("clone after a wait", H:GetCount())
local ByName = CS:CreateCollection("#Renamed", Root)
table.clear(Log)
task.defer(function() table.insert(Log, "before=" .. ByName:GetCount()) end)
X.Name = "Renamed"
task.defer(function() table.insert(Log, "after=" .. ByName:GetCount()) end)
task.wait()
print("rename order", table.concat(Log, " "), ByName:GetCount())
X.Name = "X"
print("rename back", ByName:GetCount())
ByName:Destroy()

-- selectors from the root
for _, Query in { "> Part", ">> Part", "Folder > Part", "> Folder > Part" } do
	print("relative", Query, P.try(function()
		local Made = CS:CreateCollection(Query, Root)
		local Count = Made:GetCount()
		Made:Destroy()
		return Count, #Root:QueryDescendants(Query)
	end))
end

-- GetCollections' order
local A = part("A", Root, "Ord")
local C1 = CS:CreateCollection(".Ord", Root)
local C2 = CS:CreateCollection("Part", Root)
local C3 = CS:CreateCollection("#A", Root)
task.wait()
local function order()
	local Out = {}
	for _, Handle in CS:GetCollections(A) do
		table.insert(Out, if Handle == C1 then "C1" elseif Handle == C2 then "C2" elseif Handle == C3 then "C3" else "?")
	end
	return table.concat(Out, ",")
end
print("collections order", order())
C2:Destroy()
print("collections order after destroy", order())
A:RemoveTag("Ord")
print("collections after untag", order())
task.wait()
print("collections after untag and wait", order())
C1:Destroy()
C3:Destroy()

-- the root unparented, then destroyed
local R = Instance.new("Folder")
R.Parent = workspace
part("RP", R, "RTag")
local RH = CS:CreateCollection(".RTag", R)
task.wait()
R.Parent = nil
print("root unparented", P.try(function() return RH:GetCount() end))
task.wait()
print("root unparented after a wait", P.try(function() return RH:GetCount() end))
R.Parent = workspace
task.wait()
print("root back", P.try(function() return RH:GetCount() end))
table.clear(Log)
task.defer(function() table.insert(Log, "before=" .. P.try(function() return RH:GetCount() end)) end)
R:Destroy()
task.defer(function() table.insert(Log, "after=" .. P.try(function() return RH:GetCount() end)) end)
print("root destroyed now", P.try(function() return RH:GetCount() end))
task.wait()
print("root destroyed order", table.concat(Log, " "))
local Gone = Instance.new("Folder")
Gone:Destroy()
print("destroyed root", P.try(function()
	local Made = CS:CreateCollection(".X", Gone)
	return Made:GetCount()
end))
H:Destroy()
pcall(function() NoRoot:Destroy() end)
pcall(function() NilRoot:Destroy() end)
pcall(function() WsRoot:Destroy() end)
pcall(function() OtherRoot:Destroy() end)
pcall(function() NoRootTagged:Destroy() end)
Root:Destroy()
