-- What the garbage collector frees and when: weak tables holding instances and tables, handlers that hold instances,
-- and Debris. collectgarbage("collect") is not allowed in Roblox, so the probe makes garbage until a cycle has run.
P.sync()
local RunService = game:GetService("RunService")
local function Churn()
	-- several full cycles' worth of garbage over a few frames
	for _ = 1, 6 do
		local Before = gcinfo()
		local Sink = nil
		for Index = 1, 40000 do
			Sink = { Index, Index * 2, tostring(Index) }
		end
		task.wait()
		if gcinfo() < Before then
			-- a cycle finished since the last check
		end
	end
end

local Weak = setmetatable({}, { __mode = "v" })
local WeakKeys = setmetatable({}, { __mode = "k" })

-- tables and functions
Weak.Table = {}
Weak.Closure = function() end
local Kept = {}
Weak.KeptTable = Kept
-- instances: in the DataModel with no Lua reference, parentless, destroyed, held by a connection, held by a property
do
	local InWorld = Instance.new("Part")
	InWorld.Name = "ParityWeakInWorld"
	InWorld.Parent = workspace
	Weak.InWorld = InWorld
	WeakKeys[InWorld] = "in-world"
end
do
	local Loose = Instance.new("Part")
	Weak.Loose = Loose
	WeakKeys[Loose] = "loose"
end
do
	local Destroyed = Instance.new("Part")
	Destroyed.Parent = workspace
	Destroyed:Destroy()
	Weak.Destroyed = Destroyed
end
do
	local HeldByHandler = Instance.new("Part")
	HeldByHandler.Changed:Connect(function()
		local _ = HeldByHandler.Name
	end)
	Weak.HeldByHandler = HeldByHandler
end
do
	local HeldByOther = Instance.new("Folder")
	local OtherEvent = Instance.new("BindableEvent")
	OtherEvent.Parent = workspace
	OtherEvent.Name = "ParityHolderEvent"
	OtherEvent.Event:Connect(function()
		local _ = HeldByOther.Name
	end)
	Weak.HeldByOther = HeldByOther
end
do
	local Pointed = Instance.new("Folder")
	local Pointer = Instance.new("ObjectValue")
	Pointer.Value = Pointed
	Pointer.Name = "ParityPointer"
	Pointer.Parent = workspace
	Weak.PointedByObjectValue = Pointed
end
do
	local LooseTree = Instance.new("Folder")
	local Child = Instance.new("Part")
	Child.Parent = LooseTree
	Weak.LooseTreeChild = Child
end
do
	local Signal = Instance.new("BindableEvent")
	local Connection = Signal.Event:Connect(function() end)
	Weak.Connection = Connection
	Weak.Signal = Signal.Event
end
do
	local Thread = task.delay(100, function() end)
	Weak.DelayedThread = Thread
	local Done = coroutine.create(function() end)
	coroutine.resume(Done)
	Weak.DeadThread = Done
end
do
	Weak.Vector3 = Vector3.new(1, 2, 3)
	Weak.CFrame = CFrame.new(1, 2, 3)
	Weak.String = "a string value " .. tostring(math.random())
	Weak.EnumItem = Enum.Material.Plastic
end

Churn()
Churn()
local Names = {}
for Name in Weak do
	table.insert(Names, Name)
end
table.sort(Names)
print("weak-values-left " .. table.concat(Names, ","))
local KeyNames = {}
for _, Value in WeakKeys do
	table.insert(KeyNames, Value)
end
table.sort(KeyNames)
print("weak-keys-left " .. table.concat(KeyNames, ","))
print("in-world-still-there " .. tostring(workspace:FindFirstChild("ParityWeakInWorld") ~= nil))
workspace.ParityWeakInWorld:Destroy()
workspace.ParityHolderEvent:Destroy()
workspace.ParityPointer:Destroy()

-- an instance key whose instance stays in the world: same key after the userdata could have been collected?
local Holder = Instance.new("Folder")
Holder.Name = "ParityIdentity"
Holder.Parent = workspace
local Cache = setmetatable({}, { __mode = "k" })
Cache[workspace.ParityIdentity] = true
Churn()
print("identity-cache-hit " .. tostring(Cache[workspace.ParityIdentity] == true))
Holder:Destroy()

-- Debris
local Debris = game:GetService("Debris")
local Frames = 0
local Counter = RunService.Heartbeat:Connect(function()
	Frames += 1
end)
local Results = {}
for _, Seconds in { 0, 0.02, 0.04, 0.06, -1 } do
	local Item = Instance.new("Part")
	Item.Parent = workspace
	local Start = Frames
	local Gone
	Item.Destroying:Connect(function()
		Gone = Frames - Start
	end)
	Debris:AddItem(Item, Seconds)
	for _ = 1, 8 do
		task.wait()
		if Gone then
			break
		end
	end
	table.insert(Results, tostring(Seconds) .. "->" .. tostring(Gone))
end
local Default = Instance.new("Part")
Default.Parent = workspace
Debris:AddItem(Default)
task.wait()
table.insert(Results, "default-after-1-frame " .. tostring(Default.Parent ~= nil))
Default:Destroy()
Counter:Disconnect()
print("debris " .. table.concat(Results, ","))
print("debris-errors " .. P.try(Debris.AddItem, Debris, {}, 1) .. " | " .. P.try(Debris.AddItem, Debris, nil, 1) .. " | " .. P.try(Debris.AddItem, Debris, Instance.new("Part"), "x"))
print("debris-maxitems " .. P.try(function()
	return Debris.MaxItems
end))
