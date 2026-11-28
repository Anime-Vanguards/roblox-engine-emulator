-- CollectionService:CreateCollection, third round: the handle's On* callbacks
P.sync()
local CS = game:GetService("CollectionService")
local Root = Instance.new("Folder")
Root.Parent = workspace
local H = CS:CreateCollection(".Third", Root)
local Log = {}
for _, Name in { "OnAdded", "OnRemoved", "OnInstanceAdded", "OnInstanceRemoved", "OnItemAdded", "OnItemRemoved", "OnChanged", "OnCountChanged", "OnFoo", "On", "Onadded", "onAdded", "OnDestroyed", "OnDestroy" } do
	print("callback", Name, P.try(function()
		(H :: any)[Name] = function(...)
			local Parts = { Name }
			for Index = 1, select("#", ...) do
				local Value = select(Index, ...)
				table.insert(Parts, if typeof(Value) == "Instance" then Value.Name else typeof(Value) .. ":" .. tostring(Value))
			end
			table.insert(Log, table.concat(Parts, ","))
		end
	end), P.try(function() return typeof((H :: any)[Name]) end))
end
print("callback non-function", P.try(function() (H :: any).OnAdded = 5 end), P.try(function() (H :: any).OnAdded = nil end))
local A = Instance.new("Part")
A.Name = "A"
A:AddTag("Third")
task.defer(function() table.insert(Log, "defer-before") end)
A.Parent = Root
task.defer(function() table.insert(Log, "defer-after") end)
table.insert(Log, "now")
task.wait()
print("add", table.concat(Log, " | "))
table.clear(Log)
A:RemoveTag("Third")
table.insert(Log, "now=" .. H:GetCount())
task.wait()
print("remove", table.concat(Log, " | "))
table.clear(Log)
A:AddTag("Third")
A.Parent = nil
A.Parent = Root
task.wait()
print("flicker", table.concat(Log, " | "))
table.clear(Log)
H:Destroy()
task.wait()
print("destroy", table.concat(Log, " | "))
Root:Destroy()
