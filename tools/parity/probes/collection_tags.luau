-- The order GetTags (and CollectionService's GetTags and GetAllTags) lists tags in: many tags added in several orders,
-- case and digits mixed, removed and added again, copied by Clone. Then CollectionService's tag methods given one
-- argument (an instance or a text and no tag): what they answer and do to the service's own tags, with the warning
-- the engine logs.
local CollectionService = game:GetService("CollectionService")
local LogService = game:GetService("LogService")
P.sync()
local Seen = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageWarning and string.sub(Message, 1, 10) == "argument #" then
		table.insert(Seen, (string.split(Message, "\n")[1]))
	end
end)
local function Warned()
	task.wait()
	task.wait()
	local Text = table.concat(Seen, " | ")
	table.clear(Seen)
	return "[" .. Text .. "]"
end
local function Tags(Holder)
	-- (Instance's GetTags through another instance, so CollectionService's own GetTags is not the one called)
	return table.concat(game.GetTags(Holder), ",")
end
local function Own(List)
	-- the tags this probe made, in the order given (another script's tags left out)
	local Kept = {}
	for _, Tag in List do
		if string.sub(Tag, 1, 2) == "q-" then
			table.insert(Kept, Tag)
		end
	end
	return table.concat(Kept, ",")
end

-- orders
local Orders = {
	{ "b", "a" },
	{ "a", "b" },
	{ "c", "a", "b" },
	{ "B", "a", "C" },
	{ "a", "B", "c" },
	{ "10", "9", "2", "1", "100" },
	{ "zeta", "alpha", "Mid", "beta", "Alpha", "_under", "~tilde", "space x", "x space", "é", "e", "f" },
	{ "t01", "t02", "t03", "t04", "t05", "t06", "t07", "t08", "t09", "t10", "t11", "t12", "t13", "t14", "t15", "t16", "t17", "t18", "t19", "t20" },
	{ "t20", "t19", "t18", "t17", "t16", "t15", "t14", "t13", "t12", "t11", "t10", "t09", "t08", "t07", "t06", "t05", "t04", "t03", "t02", "t01" },
}
for Index, Order in Orders do
	local Holder = Instance.new("Folder")
	for _, Tag in Order do
		Holder:AddTag(Tag)
	end
	print("order " .. Index .. " " .. Tags(Holder))
	print("order-service " .. Index .. " " .. table.concat(CollectionService:GetTags(Holder), ","))
end

-- a random set, added in a random order twice
local Random = P.rng(7)
local Words = {}
for Index = 1, 40 do
	local Length = Random.int(1, 6)
	local Chars = {}
	for _ = 1, Length do
		table.insert(Chars, string.char(Random.int(97, 122)))
	end
	table.insert(Words, table.concat(Chars) .. Index)
end
local function Shuffled()
	local Copy = table.clone(Words)
	for Index = #Copy, 2, -1 do
		local Other = Random.int(1, Index)
		Copy[Index], Copy[Other] = Copy[Other], Copy[Index]
	end
	return Copy
end
local First, Second = Instance.new("Folder"), Instance.new("Folder")
for _, Tag in Shuffled() do
	First:AddTag(Tag)
end
for _, Tag in Shuffled() do
	Second:AddTag(Tag)
end
print("random-first " .. Tags(First))
print("random-second " .. Tags(Second))
print("random-same " .. tostring(Tags(First) == Tags(Second)))

-- removed and added again, added twice
local Again = Instance.new("Folder")
for _, Tag in { "c", "a", "b", "d" } do
	Again:AddTag(Tag)
end
Again:RemoveTag("a")
print("removed " .. Tags(Again))
Again:AddTag("a")
print("added-again " .. Tags(Again))
Again:AddTag("c")
print("added-twice " .. Tags(Again))
Again:RemoveTag("c")
Again:RemoveTag("d")
Again:AddTag("d")
Again:AddTag("c")
print("readded " .. Tags(Again))
print("clone " .. Tags(Again:Clone()))

-- GetAllTags and GetTagged over the probe's own tags
local Spread = {}
for Index, Tag in { "q-m", "q-c", "q-x", "q-a", "q-B" } do
	local Holder = Instance.new("Folder")
	Holder.Name = "spread" .. Index
	Holder.Parent = workspace
	Holder:AddTag(Tag)
	table.insert(Spread, Holder)
end
print("all-tags " .. Own(CollectionService:GetAllTags()))
local Shared = {}
for Index = 1, 6 do
	local Holder = Instance.new("Folder")
	Holder.Name = "shared" .. ({ "f", "b", "e", "a", "d", "c" })[Index]
	Holder.Parent = workspace
	Holder:AddTag("q-shared")
	table.insert(Shared, Holder)
end
local Names = {}
for _, Holder in CollectionService:GetTagged("q-shared") do
	table.insert(Names, Holder.Name)
end
-- (sorted: a client's order differs from one session to the next)
table.sort(Names)
print("tagged " .. table.concat(Names, ","))
for _, Holder in Spread do
	Holder:Destroy()
end
for _, Holder in Shared do
	Holder:Destroy()
end

-- one argument
local Folder = Instance.new("Folder")
Folder.Name = "ProbeFolder"
print("service-tags-start " .. Tags(CollectionService))
print("has-instance " .. P.try(function()
	return CollectionService:HasTag(Folder)
end) .. " " .. Warned())
print("add-instance " .. P.try(function()
	return CollectionService:AddTag(Folder)
end) .. " " .. Warned())
print("after-add-instance " .. Tags(CollectionService) .. " folder " .. Tags(Folder))
print("has-instance-2 " .. P.try(function()
	return CollectionService:HasTag(Folder)
end) .. " " .. Warned())
print("has-workspace " .. P.try(function()
	return CollectionService:HasTag(workspace)
end) .. " " .. Warned())
print("has-text " .. P.try(function()
	return CollectionService:HasTag("Instance")
end) .. " " .. Warned())
local Tagged = CollectionService:GetTagged("Instance")
print("tagged-instance " .. #Tagged .. " " .. tostring(Tagged[1] == CollectionService))
print("remove-workspace " .. P.try(function()
	return CollectionService:RemoveTag(workspace)
end) .. " " .. Warned())
print("after-remove " .. Tags(CollectionService))
print("add-text " .. P.try(function()
	return CollectionService:AddTag("q-solo")
end) .. " " .. Warned())
print("after-add-text " .. Tags(CollectionService))
print("has-text-2 " .. P.try(function()
	return CollectionService:HasTag("q-solo")
end) .. " " .. Warned())
print("add-number " .. P.try(function()
	return CollectionService:AddTag(5)
end) .. " " .. Warned())
print("add-vector " .. P.try(function()
	return CollectionService:AddTag(Vector3.new(1, 2, 3))
end) .. " " .. Warned())
print("after-add-number " .. Tags(CollectionService))
print("remove-text " .. P.try(function()
	return CollectionService:RemoveTag("q-solo")
end) .. " " .. Warned())
print("remove-number " .. P.try(function()
	return CollectionService:RemoveTag(5)
end) .. " " .. Warned())
print("remove-vector " .. P.try(function()
	return CollectionService:RemoveTag(Vector3.new(1, 2, 3))
end) .. " " .. Warned())
print("after-remove-all " .. Tags(CollectionService))
print("add-none " .. P.try(function()
	return CollectionService:AddTag()
end) .. " " .. Warned())
print("has-none " .. P.try(function()
	return CollectionService:HasTag()
end) .. " " .. Warned())
print("remove-none " .. P.try(function()
	return CollectionService:RemoveTag()
end) .. " " .. Warned())
print("add-nil " .. P.try(function()
	return CollectionService:AddTag(nil)
end) .. " " .. Warned())
print("has-nil " .. P.try(function()
	return CollectionService:HasTag(nil)
end) .. " " .. Warned())
print("add-instance-nil " .. P.try(function()
	return CollectionService:AddTag(Folder, nil)
end) .. " " .. Warned())
print("has-instance-nil " .. P.try(function()
	return CollectionService:HasTag(Folder, nil)
end) .. " " .. Warned())
print("gettags-instance " .. P.try(function()
	return table.concat(CollectionService:GetTags(Folder), ",")
end) .. " " .. Warned())
print("gettags-none " .. P.try(function()
	return table.concat(CollectionService:GetTags(), ",")
end) .. " " .. Warned())
print("gettags-text " .. P.try(function()
	return table.concat(CollectionService:GetTags("x"), ",")
end) .. " " .. Warned())
print("service-tags-end " .. Tags(CollectionService))
for _, Tag in game.GetTags(CollectionService) do
	CollectionService:RemoveTag(CollectionService, Tag)
end
Listener:Disconnect()
