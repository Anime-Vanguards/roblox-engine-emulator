-- parity: loopback
-- REPL-MOVE-COALESCE: what the client's copy of a tree does when the server's instance comes back into the replicated
-- containers (from nil, from ServerStorage, in a new folder, in the same frame or frames later), and when it grew or lost
-- a descendant while it was out: each instance's AncestryChanged and Parent's Changed, and the folders' ChildAdded /
-- ChildRemoved, in order.
local Storage = game:GetService("ReplicatedStorage")
local Steps = { "back", "apart", "hidden", "intoNew", "grown", "shrunk", "moved" }
--- server
P.player()
local Box = Instance.new("Folder")
Box.Name = "Box"
Box.Parent = Storage
local Box2 = Instance.new("Folder")
Box2.Name = "Box2"
Box2.Parent = Storage
local Item = Instance.new("Folder")
Item.Name = "Item"
local Inner = Instance.new("Folder")
Inner.Name = "Inner"
Inner.Parent = Item
local Deep = Instance.new("Folder")
Deep.Name = "Deep"
Deep.Parent = Inner
local Inner2 = Instance.new("Folder")
Inner2.Name = "Inner2"
Inner2.Parent = Item
Item.Parent = Box
local Hidden = game:GetService("ServerStorage")
local Moves = {
	back = function()
		Item.Parent = nil
		Item.Parent = Box
	end,
	apart = function()
		Item.Parent = nil
		task.wait()
		task.wait()
		Item.Parent = Box
	end,
	hidden = function()
		Item.Parent = Hidden
		task.wait()
		task.wait()
		Item.Parent = Box
	end,
	intoNew = function()
		local New = Instance.new("Folder")
		New.Name = "New"
		Item.Parent = New
		New.Parent = Box2
	end,
	grown = function()
		Item.Parent = nil
		task.wait()
		local Extra = Instance.new("Folder")
		Extra.Name = "Extra"
		Extra.Parent = Inner
		task.wait()
		Item.Parent = Box
	end,
	shrunk = function()
		Item.Parent = nil
		task.wait()
		Inner2.Parent = nil
		task.wait()
		Item.Parent = Box
	end,
	moved = function()
		Item.Parent = nil
		task.wait()
		Deep.Parent = Item
		task.wait()
		Item.Parent = Box
	end,
}
P.meet("ready")
for _, Step in Steps do
	Item.Parent = Box
	P.meet(Step .. " reset")
	P.meet(Step)
	Moves[Step]()
	P.meet(Step .. " seen")
end
--- client
local Box = Storage:WaitForChild("Box")
local Box2 = Storage:WaitForChild("Box2")
local Item = Box:WaitForChild("Item")
local Inner = Item:WaitForChild("Inner")
local Deep = Inner:WaitForChild("Deep")
local Inner2 = Item:WaitForChild("Inner2")
local Events = {}
local function named(Instance)
	return if Instance then Instance.Name else "nil"
end
for _, Thing in { Item, Inner, Deep, Inner2 } do
	Thing.AncestryChanged:Connect(function(Child, Parent)
		table.insert(Events, Thing.Name .. " AncestryChanged " .. Child.Name .. " " .. named(Parent))
	end)
	Thing:GetPropertyChangedSignal("Parent"):Connect(function()
		table.insert(Events, Thing.Name .. " Parent")
	end)
end
for _, Folder in { Box, Box2, Item, Inner } do
	Folder.ChildAdded:Connect(function(Child)
		table.insert(Events, Folder.Name .. " ChildAdded " .. Child.Name)
	end)
	Folder.ChildRemoved:Connect(function(Child)
		table.insert(Events, Folder.Name .. " ChildRemoved " .. Child.Name)
	end)
end
P.meet("ready")
for _, Step in Steps do
	P.meet(Step .. " reset")
	for _ = 1, 5 do
		task.wait()
	end
	table.clear(Events)
	P.meet(Step)
	for _ = 1, 8 do
		task.wait()
	end
	local Kids = {}
	for _, Child in Item:GetDescendants() do
		table.insert(Kids, Child.Name .. "<" .. named(Child.Parent))
	end
	print(Step .. ": item in " .. named(Item.Parent) .. ", tree " .. table.concat(Kids, ",") .. " | " .. table.concat(Events, " / "))
	table.clear(Events)
	P.meet(Step .. " seen")
end
