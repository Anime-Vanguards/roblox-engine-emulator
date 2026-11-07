-- ReflectionService:GetClasses and GetClass as a game script gets them: the list's order and freezing, entries' keys and
-- values, the classes a script cannot get, the filter and its errors, and what Permits holds.
local RS = game:GetService("ReflectionService")

local function Keys(Table)
	local Out = {}
	for Key in Table do
		table.insert(Out, tostring(Key))
	end
	table.sort(Out)
	return table.concat(Out, ",")
end

local List = RS:GetClasses()
local Sorted, Frozen = true, true
for Index, Entry in List do
	if Index > 1 and not (List[Index - 1].Name < Entry.Name) then
		Sorted = false
	end
	if not table.isfrozen(Entry) then
		Frozen = false
	end
end
print("list", Sorted, Frozen, table.isfrozen(List), RS:GetClasses() ~= List, RS:GetClasses()[1] ~= List[1], List[1].Name)
print("count", #List, #RS:GetClasses({ IsA = "Instance" }))

local function Show(Name, Filter)
	local Entry = RS:GetClass(Name, Filter)
	if Entry == nil then
		print("class", Name, "nil")
		return
	end
	local Subclasses = if Entry.Subclasses then table.concat(Entry.Subclasses, ",") .. " " .. tostring(table.isfrozen(Entry.Subclasses)) else "-"
	local Display = if Entry.Display
		then Keys(Entry.Display) .. " " .. tostring(Entry.Display.Category) .. " " .. tostring(Entry.Display.DeprecationMessage) .. " " .. tostring(table.isfrozen(Entry.Display))
		else "-"
	print("class", Name, Keys(Entry), Entry.Superclass, Entry.Serialized, Subclasses, Display)
end
for _, Name in { "Part", "Accessory", "BasePlayerGui", "Glue", "BodyMover", "LogService", "ReflectionService", "Frame", "ModuleScript", "Workspace", "EditableImage" } do
	Show(Name)
end
for _, Name in { "Player", "CoreGui", "UserGameSettings", "PluginGui", "part", "Nope", "" } do
	Show(Name)
end
Show("Part", { IsA = "BasePart" })
Show("Part", { IsA = "GuiObject" })
Show("Part", { ExcludeDisplay = true })
Show("Part", { Unknown = 1, isA = "GuiObject" })

local function Names(Filter)
	local Out = {}
	for _, Entry in RS:GetClasses(Filter) do
		table.insert(Out, Entry.Name)
	end
	return table.concat(Out, ",")
end
print("IsA BasePart", Names({ IsA = "BasePart" }))
print("IsA GuiObject", Names({ IsA = "GuiObject" }))
print("IsA BasePlayerGui", Names({ IsA = "BasePlayerGui" }))
print("IsA none", #RS:GetClasses({ IsA = "Nope" }), #RS:GetClasses({ IsA = "basepart" }), #RS:GetClasses({ IsA = "" }), #RS:GetClasses({ IsA = "Player" }), #RS:GetClasses({ IsA = "CoreGui" }))
local NoDisplay = RS:GetClasses({ ExcludeDisplay = true })
print("ExcludeDisplay", #NoDisplay == #List, NoDisplay[1].Display, Keys(NoDisplay[1]), RS:GetClasses({ ExcludeDisplay = false })[1].Display ~= nil)

print("err", P.try(RS.GetClasses, RS, 5))
print("err", P.try(RS.GetClasses, RS, true))
print("err", P.try(RS.GetClasses, RS, "BasePart"))
print("err", P.try(RS.GetClasses, RS, { "BasePart" }))
print("err", P.try(RS.GetClasses, RS, { 1, IsA = "BasePart" }))
print("err", P.try(RS.GetClasses, RS, { IsA = 5 }))
print("err", P.try(RS.GetClasses, RS, { ExcludeDisplay = 1 }))
print("err", P.try(RS.GetClasses, RS, { Security = "x" }))
print("err", P.try(RS.GetClass, RS))
print("err", P.try(RS.GetClass, RS, nil))
print("err", P.try(RS.GetClass, RS, 5))
print("err", P.try(RS.GetClass, RS, "Part", 5))
print("err", P.try(RS.GetClasses))
print("err", P.try(function()
	List[1] = 1
end))
print("err", P.try(function()
	(List[1] :: any).Name = "x"
end))

local Permits = RS:GetClass("Part").Permits
print("Permits", typeof(Permits), table.isfrozen(Permits))
print("Permits held", Permits.New ~= nil, RS:GetClass("Workspace").Permits.GetService ~= nil, next(RS:GetClass("BasePart").Permits))
