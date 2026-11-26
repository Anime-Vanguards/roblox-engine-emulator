-- parity: loopback
-- REPL-MOVE-COALESCE: what the client's copy does when the server moves an instance more than once in one frame (out of its
-- parent and back, through another replicated folder, out of the replicated containers and back, to nil and elsewhere,
-- with writes between, a new instance moved before it was ever sent, a child order after a move out and back, a model in
-- the Workspace, a move then a destroy, a move into a folder made in the frame), and the same out and back over frames:
-- the client's events (AncestryChanged, the Parent's and Name's Changed, ChildAdded / ChildRemoved of the folders), in
-- order, and where each copy ends.
local Storage = game:GetService("ReplicatedStorage")
local Steps = { "back", "through", "hidden", "chain", "nilThen", "hiddenThen", "written", "thrice", "fresh", "freshWritten", "order", "model", "apart", "doomed", "intoNew" }
--- server
P.player()
local Boxes = {}
for _, Name in { "Box", "Box2", "Box3" } do
	local Box = Instance.new("Folder")
	Box.Name = Name
	Box.Parent = Storage
	Boxes[Name] = Box
end
local Item = Instance.new("Folder")
Item.Name = "Item"
Item.Parent = Boxes.Box
local Inner = Instance.new("Folder")
Inner.Name = "Inner"
Inner.Parent = Item
local Doomed = Instance.new("Folder")
Doomed.Name = "Doomed"
Doomed.Parent = Boxes.Box
local Order = Instance.new("Folder")
Order.Name = "Order"
Order.Parent = Storage
for _, Name in { "A", "B", "C" } do
	local Child = Instance.new("Folder")
	Child.Name = Name
	Child.Parent = Order
end
local Model = Instance.new("Model")
Model.Name = "MoveModel"
local Part = Instance.new("Part")
Part.Name = "Body"
Part.Anchored = true
Part.Position = Vector3.new(0, 300, 0)
Part.Parent = Model
Model.Parent = workspace
local Hidden = game:GetService("ServerStorage")
local Moves = {
	back = function()
		Item.Parent = nil
		Item.Parent = Boxes.Box
	end,
	through = function()
		Item.Parent = Boxes.Box2
		Item.Parent = Boxes.Box
	end,
	hidden = function()
		Item.Parent = Hidden
		Item.Parent = Boxes.Box
	end,
	chain = function()
		Item.Parent = Boxes.Box2
		Item.Parent = Boxes.Box3
	end,
	nilThen = function()
		Item.Parent = nil
		Item.Parent = Boxes.Box2
	end,
	hiddenThen = function()
		Item.Parent = Hidden
		Item.Parent = Boxes.Box2
	end,
	written = function()
		Item.Parent = Boxes.Box2
		Item.Name = "Item2"
		Item.Parent = Boxes.Box
		Item.Name = "Item3"
	end,
	thrice = function()
		Item.Parent = Boxes.Box2
		Item.Parent = Boxes.Box
		Item.Parent = Boxes.Box3
	end,
	fresh = function()
		local Fresh = Instance.new("Folder")
		Fresh.Name = "Fresh"
		Fresh.Parent = Boxes.Box
		Fresh.Parent = Boxes.Box2
	end,
	freshWritten = function()
		local Fresh = Instance.new("Folder")
		Fresh.Name = "Fresh2"
		Fresh.Parent = Boxes.Box
		Fresh.Name = "Fresh3"
		Fresh.Parent = Boxes.Box2
	end,
	order = function()
		local A = Order:FindFirstChild("A")
		A.Parent = nil
		A.Parent = Order
	end,
	model = function()
		Model.Parent = nil
		Model.Parent = workspace
	end,
	apart = function()
		Item.Parent = nil
		task.wait()
		task.wait()
		Item.Parent = Boxes.Box
	end,
	doomed = function()
		Doomed.Parent = Boxes.Box2
		Doomed:Destroy()
	end,
	intoNew = function()
		local New = Instance.new("Folder")
		New.Name = "New"
		Item.Parent = New
		New.Parent = Boxes.Box3
	end,
}
P.meet("ready")
for _, Step in Steps do
	Item.Parent = Boxes.Box
	Item.Name = "Item"
	P.meet(Step .. " reset")
	P.meet(Step)
	Moves[Step]()
	P.meet(Step .. " seen")
end
--- client
local Boxes = {}
for _, Name in { "Box", "Box2", "Box3" } do
	Boxes[Name] = Storage:WaitForChild(Name)
end
local Item = Boxes.Box:WaitForChild("Item")
local Inner = Item:WaitForChild("Inner")
local Doomed = Boxes.Box:WaitForChild("Doomed")
local Order = Storage:WaitForChild("Order")
Order:WaitForChild("C")
local Model = workspace:WaitForChild("MoveModel")
local Events = {}
local function named(Instance)
	return if Instance then Instance.Name else "nil"
end
local function watch(Thing, Label)
	Thing.AncestryChanged:Connect(function(Child, Parent)
		table.insert(Events, Label .. " AncestryChanged " .. Child.Name .. " " .. named(Parent))
	end)
	Thing:GetPropertyChangedSignal("Parent"):Connect(function()
		table.insert(Events, Label .. " Parent")
	end)
end
watch(Item, "Item")
watch(Inner, "Inner")
watch(Model, "Model")
watch(Doomed, "Doomed")
Item:GetPropertyChangedSignal("Name"):Connect(function()
	table.insert(Events, "Item Name " .. Item.Name)
end)
Doomed.Destroying:Connect(function()
	table.insert(Events, "Doomed Destroying")
end)
for _, Box in { Boxes.Box, Boxes.Box2, Boxes.Box3, Order } do
	Box.ChildAdded:Connect(function(Child)
		table.insert(Events, Box.Name .. " ChildAdded " .. Child.Name .. (if Child == Item then " (same)" else ""))
	end)
	Box.ChildRemoved:Connect(function(Child)
		table.insert(Events, Box.Name .. " ChildRemoved " .. Child.Name)
	end)
end
workspace.ChildAdded:Connect(function(Child)
	if Child.Name == "MoveModel" then
		table.insert(Events, "Workspace ChildAdded MoveModel" .. (if Child == Model then " (same)" else ""))
	end
end)
workspace.ChildRemoved:Connect(function(Child)
	if Child.Name == "MoveModel" then
		table.insert(Events, "Workspace ChildRemoved MoveModel")
	end
end)
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
	local Names = {}
	for _, Child in Order:GetChildren() do
		table.insert(Names, Child.Name)
	end
	print(Step .. ": item in " .. named(Item.Parent) .. ", inner in " .. named(Inner.Parent) .. ", model in " .. named(Model.Parent)
		.. ", order " .. table.concat(Names, ",") .. " | " .. table.concat(Events, " / "))
	table.clear(Events)
	P.meet(Step .. " seen")
end
