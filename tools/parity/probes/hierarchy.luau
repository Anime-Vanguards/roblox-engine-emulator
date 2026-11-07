-- The order of hierarchy events when subtrees move, and what Destroy and ClearAllChildren do.
P.sync()
local Log = {}
local function Note(Text)
	table.insert(Log, Text)
end
local function Flush(Label)
	print(Label .. " " .. table.concat(Log, ","))
	table.clear(Log)
end
local Watched = {}
local function Watch(Instance)
	table.insert(Watched, Instance.ChildAdded:Connect(function(Child)
		Note(Instance.Name .. ".ChildAdded(" .. Child.Name .. ")")
	end))
	table.insert(Watched, Instance.ChildRemoved:Connect(function(Child)
		Note(Instance.Name .. ".ChildRemoved(" .. Child.Name .. ")")
	end))
	table.insert(Watched, Instance.DescendantAdded:Connect(function(Child)
		Note(Instance.Name .. ".DescendantAdded(" .. Child.Name .. ")")
	end))
	table.insert(Watched, Instance.DescendantRemoving:Connect(function(Child)
		Note(Instance.Name .. ".DescendantRemoving(" .. Child.Name .. ")")
	end))
	table.insert(Watched, Instance.AncestryChanged:Connect(function(Child, Parent)
		Note(Instance.Name .. ".AncestryChanged(" .. Child.Name .. "," .. (Parent and Parent.Name or "nil") .. ")")
	end))
	table.insert(Watched, Instance.Destroying:Connect(function()
		Note(Instance.Name .. ".Destroying")
	end))
	table.insert(Watched, Instance:GetPropertyChangedSignal("Parent"):Connect(function()
		Note(Instance.Name .. ".Parent")
	end))
end
local function Make(Name, Parent)
	local Folder = Instance.new("Folder")
	Folder.Name = Name
	Folder.Parent = Parent
	return Folder
end

-- a tree: Root > A > (A1, A2 > A21), B; Root is in the workspace
local Root = Make("Root", workspace)
local A = Make("A", Root)
local A1 = Make("A1", A)
local A2 = Make("A2", A)
local A21 = Make("A21", A2)
local B = Make("B", Root)
task.wait()
for _, Instance in { Root, A, A1, A2, A21, B } do
	Watch(Instance)
end

-- 1. move A under B
A.Parent = B
task.wait()
Flush("move-subtree")

-- 2. take A out of the tree, then put it back
A.Parent = nil
task.wait()
Flush("remove-subtree")
A.Parent = Root
task.wait()
Flush("insert-subtree")

-- 3. a bulk reparent: three children moved one after another in one batch
local Box = Make("Box", nil)
local Items = {}
for Index = 1, 3 do
	Items[Index] = Make("Item" .. Index, Box)
end
Box.Parent = Root
task.wait()
Flush("box-in")
for _, Item in Items do
	Item.Parent = B
end
task.wait()
Flush("bulk-move")

-- 4. Destroy: event order, Parent locked, children destroyed, connections gone
A.Destroying:Connect(function()
	Note("A.Destroying sees parent " .. tostring(A.Parent and A.Parent.Name))
end)
A:Destroy()
Note("after-destroy parent " .. tostring(A.Parent) .. " A1 parent " .. tostring(A1.Parent) .. " A21 parent " .. tostring(A21.Parent))
task.wait()
Flush("destroy")
print("destroyed-parent-write " .. P.try(function()
	A.Parent = Root
end))
print("destroyed-child-parent-write " .. P.try(function()
	A21.Parent = Root
end))
print("destroyed-name-write " .. P.try(function()
	A.Name = "Still"
end) .. " " .. A.Name)
print("destroy-twice " .. P.try(A.Destroy, A))
print("destroyed-findfirstchild " .. tostring(A:FindFirstChild("A1")) .. " children " .. #A:GetChildren())
local Probe = Instance.new("BindableEvent")
Probe.Parent = A1
print("parent-into-destroyed " .. tostring(Probe.Parent))

-- 5. Destroying of a whole tree: which instances fire, in what order
local T = Make("T", Root)
local T1 = Make("T1", T)
local T11 = Make("T11", T1)
local T2 = Make("T2", T)
task.wait()
for _, Instance in { T, T1, T11, T2 } do
	Instance.Destroying:Connect(function()
		Note(Instance.Name .. ".Destroying parent=" .. tostring(Instance.Parent and Instance.Parent.Name))
	end)
end
T:Destroy()
Note("returned")
task.wait()
Flush("destroy-tree")

-- 6. ClearAllChildren
local C = Make("C", Root)
for Index = 1, 3 do
	Make("C" .. Index, C)
end
task.wait()
Watch(C)
C:ClearAllChildren()
task.wait()
Flush("clear-all-children")
print("clear-all-children-left " .. #C:GetChildren())

-- 7. parenting errors
local Self = Make("Self", nil)
print("parent-to-self " .. P.try(function()
	Self.Parent = Self
end))
local Outer = Make("Outer", nil)
local Inner = Make("Inner", Outer)
print("parent-to-descendant " .. P.try(function()
	Outer.Parent = Inner
end))
print("parent-to-number " .. P.try(function()
	Outer.Parent = 5
end))
print("parent-to-service " .. P.try(function()
	game:GetService("Lighting").Parent = workspace
end))
print("parent-of-datamodel " .. (function()
	-- the DataModel's name is the place's: left out
	local Result = P.try(function()
		game.Parent = workspace
	end)
	return if #game.Name > 0 then (string.gsub(Result, (string.gsub(game.Name, "%p", "%%%0")), "<game>")) else Result
end)())
print("destroy-service " .. P.try(function()
	game:GetService("Lighting"):Destroy()
end))

-- 8. reparenting from inside a ChildAdded handler
local Shuffle = Make("Shuffle", Root)
local Other = Make("Other", Root)
task.wait()
Shuffle.ChildAdded:Connect(function(Child)
	Note("Shuffle.ChildAdded(" .. Child.Name .. ") parent now " .. tostring(Child.Parent and Child.Parent.Name))
	if Child.Name == "Moved" then
		Child.Parent = Other
	end
end)
Other.ChildAdded:Connect(function(Child)
	Note("Other.ChildAdded(" .. Child.Name .. ")")
end)
local Moved = Make("Moved", Shuffle)
task.wait()
task.wait()
Flush("reparent-in-handler")
-- an instance added and removed before its event runs
local Gone = Make("Gone", Shuffle)
Gone.Parent = nil
task.wait()
Flush("added-then-removed")

for _, Connection in Watched do
	Connection:Disconnect()
end
Root:Destroy()
