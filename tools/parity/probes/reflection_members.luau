-- ReflectionService's member queries (GetPropertiesOfClass, GetMethodsOfClass, GetEventsOfClass) and its Security filters as a
-- game script gets them: the lists' order and owners, each kind of entry's fields and values, DefaultValues, freezing and new
-- values per call, the filter fields and their errors, the classes a set may not query, and what GetClass and GetClasses list
-- and permit under other sets. Whole views are checked by fingerprint (an FNV-1a hash of every entry's fields).
local RS = game:GetService("ReflectionService")
local LogService = game:GetService("LogService")

local function Keys(Table)
	local Out = {}
	for Key in Table do
		table.insert(Out, tostring(Key))
	end
	table.sort(Out)
	return table.concat(Out, ",")
end
-- a value as text: tables with their keys sorted, frozen ones marked F, SecurityCapabilities as SC<...>
local function Describe(Value)
	local Kind = typeof(Value)
	if Kind == "table" then
		local Parts = {}
		for Index = 1, #Value do
			table.insert(Parts, Describe(Value[Index]))
		end
		local Named = {}
		for Key in Value do
			if not (type(Key) == "number" and Key >= 1 and Key <= #Value and Key % 1 == 0) then
				table.insert(Named, Key)
			end
		end
		table.sort(Named, function(A, B)
			return tostring(A) < tostring(B)
		end)
		for _, Key in Named do
			table.insert(Parts, tostring(Key) .. "=" .. Describe(Value[Key]))
		end
		return (if table.isfrozen(Value) then "F" else "") .. "{" .. table.concat(Parts, ",") .. "}"
	elseif Kind == "string" then
		return string.format("%q", Value)
	elseif Kind == "SecurityCapabilities" then
		return "SC<" .. tostring(Value) .. ">"
	elseif Kind == "number" then
		return P.num(Value)
	end
	return Kind .. ":" .. tostring(Value)
end
local function Fnv(Text)
	local H = 0x811c9dc5
	for Index = 1, #Text do
		H = bit32.bxor(H, string.byte(Text, Index))
		H = (H * 403 + bit32.lshift(H, 24)) % 0x100000000
	end
	return string.format("%08x", H)
end
local function Names(List)
	if List == nil then
		return "nil"
	end
	local Out = {}
	for _, Entry in List do
		table.insert(Out, Entry.Name .. "@" .. Entry.Owner)
	end
	return #List .. " " .. table.concat(Out, ",")
end
local function Find(List, Name)
	for _, Entry in List or {} do
		if Entry.Name == Name then
			return Entry
		end
	end
	return nil
end
local function Try(F, ...)
	local Ok, Answer = pcall(F, ...)
	if Ok then
		return "ok " .. (if type(Answer) == "table" then "#" .. #Answer else tostring(Answer))
	end
	return "error " .. P.err(Answer)
end

local Items = Enum.SecurityCapability:GetEnumItems()
local All = SecurityCapabilities.new(table.unpack(Items))
local Current = SecurityCapabilities.fromCurrent()
local None = SecurityCapabilities.new()

-- lists: own members first, then each superclass's; a name a subclass declares again is listed once
print("props Instance", Names(RS:GetPropertiesOfClass("Instance")))
print("events Instance", Names(RS:GetEventsOfClass("Instance")))
print("props Folder", Names(RS:GetPropertiesOfClass("Folder")))
print("props Part own", Names(RS:GetPropertiesOfClass("Part", { ExcludeInherited = true })))
print("events BoolValue", Names(RS:GetEventsOfClass("BoolValue")))
local WorkspaceMethods = RS:GetMethodsOfClass("Workspace")
print("methods Workspace", #WorkspaceMethods, Find(WorkspaceMethods, "BreakJoints").Owner, Find(WorkspaceMethods, "breakJoints").Owner, WorkspaceMethods[1].Name, WorkspaceMethods[#WorkspaceMethods].Name)
for _, Class in { "Part", "Model", "Workspace", "Humanoid", "Player", "Players", "Sound", "Decal", "Frame", "TextLabel", "UserGameSettings", "GlobalSettings", "UserSettings", "NetworkClient", "NetworkServer", "Object", "LuaSettings" } do
	local Properties, Methods, Events = RS:GetPropertiesOfClass(Class), RS:GetMethodsOfClass(Class), RS:GetEventsOfClass(Class)
	print("count", Class, if Properties then #Properties else "nil", if Methods then #Methods else "nil", if Events then #Events else "nil")
end

-- entries
for _, Pair in { { "Part", "Size" }, { "Part", "Shape" }, { "Part", "shape" }, { "Part", "brickColor" }, { "Part", "Parent" }, { "Decal", "Texture" }, { "MeshPart", "MeshId" }, { "Model", "PrimaryPart" }, { "Workspace", "Gravity" }, { "Debris", "MaxItems" }, { "Player", "UserId" }, { "HttpService", "HttpEnabled" } } do
	print("prop", Pair[1] .. "." .. Pair[2], Describe(Find(RS:GetPropertiesOfClass(Pair[1]), Pair[2])))
end
for _, Pair in { { "Part", "FindFirstChild" }, { "Part", "WaitForChild" }, { "Part", "Destroy" }, { "Humanoid", "LoadAnimation" }, { "Player", "Kick" }, { "Selection", "Get" }, { "HttpService", "JSONEncode" }, { "Workspace", "GetPartBoundsInBox" } } do
	print("method", Pair[1] .. ":" .. Pair[2], Describe(Find(RS:GetMethodsOfClass(Pair[1]), Pair[2])))
end
for _, Pair in { { "Part", "Touched" }, { "Part", "Changed" }, { "Part", "childAdded" }, { "Humanoid", "Died" }, { "Players", "PlayerAdded" } } do
	print("event", Pair[1] .. "." .. Pair[2], Describe(Find(RS:GetEventsOfClass(Pair[1]), Pair[2])))
end
print("nodisplay", Describe(Find(RS:GetPropertiesOfClass("Decal", { ExcludeDisplay = true }), "Texture")), Describe(RS:GetEventsOfClass("Object", { ExcludeDisplay = true })))

-- DefaultValues: a value of each kind, new each call
local function DefaultOf(Class, Name, Parameter, Filter)
	for _, Entry in Find(RS:GetMethodsOfClass(Class, Filter), Name).Parameters do
		if Entry.Name == Parameter then
			return Entry.DefaultValue
		end
	end
	return "none"
end
local Ray1, Ray2 = DefaultOf("Workspace", "Raycast", "raycastParams"), DefaultOf("Workspace", "Raycast", "raycastParams")
print("default params", typeof(Ray1), rawequal(Ray1, Ray2), Ray1 == Ray2, tostring(Ray1), Ray1.FilterType, typeof(DefaultOf("Workspace", "GetPartBoundsInRadius", "overlapParams")), tostring(DefaultOf("Workspace", "GetPartBoundsInRadius", "overlapParams")))
-- (RaycastParams and OverlapParams are equal when every member is, a filter list item by item)
local Written, Filtered, Same = RaycastParams.new(), RaycastParams.new(), RaycastParams.new()
Written.IgnoreWater = true
Filtered.FilterDescendantsInstances = { workspace }
Same.FilterDescendantsInstances = { workspace }
local Overlap = OverlapParams.new()
Overlap.MaxParts = 3
print("params equal", RaycastParams.new() == RaycastParams.new(), Ray1 == Written, Ray1 == Filtered, Filtered == Same, OverlapParams.new() == OverlapParams.new(), OverlapParams.new() == Overlap, Ray1 == OverlapParams.new())
local Table1, Table2 = DefaultOf("AnalyticsService", "LogCustomEvent", "customFields"), DefaultOf("AnalyticsService", "LogCustomEvent", "customFields")
print("default table", typeof(Table1), rawequal(Table1, Table2), table.isfrozen(Table1), next(Table1))
local User = DefaultOf("SocialService", "CanSendGameInviteAsync", "recipientId")
print("default user", typeof(User), type(User), tostring(User), rawequal(User, DefaultOf("SocialService", "CanSendGameInviteAsync", "recipientId")), User == DefaultOf("SocialService", "CanSendGameInviteAsync", "recipientId"), Try(function()
	return User.Name
end))
print("default values", DefaultOf("BasePart", "AngularAccelerationToTorque", "angVelocity"), typeof(DefaultOf("ControlState", "AddVector2Field", "defaultValue")), DefaultOf("VirtualUser", "Button1Down", "camera"), rawequal(DefaultOf("AvatarEditorService", "GetOutfits", "outfitSource"), Enum.OutfitSource.All), DefaultOf("Part", "FindFirstChild", "recursive"), DefaultOf("AnalyticsService", "LogCustomEvent", "value"), DefaultOf("Player", "Kick", "message") == "", DefaultOf("Part", "FindFirstChild", "name"))

-- freezing, new tables and values each call
local A, B = RS:GetPropertiesOfClass("Decal"), RS:GetPropertiesOfClass("Decal")
local Texture = Find(A, "Texture")
local Method = Find(RS:GetMethodsOfClass("Part"), "FindFirstChild")
print("frozen", table.isfrozen(A), table.isfrozen(Texture), table.isfrozen(Texture.Type), table.isfrozen(Texture.Permits), table.isfrozen(Texture.Display), table.isfrozen(Method.Parameters), table.isfrozen(Method.Parameters[1]), table.isfrozen(Method.Parameters[1].Type), table.isfrozen(Method.ReturnType), table.isfrozen(Method.Permits), getmetatable(A), getmetatable(Texture))
print("new", A ~= B, A[1] ~= B[1], A[1].Type ~= B[1].Type, rawequal(A[1].Permits.Read, B[1].Permits.Read), A[1].Permits.Read == B[1].Permits.Read, typeof(A[1].Permits.Read), #RS:GetEventsOfClass("Part", { ExcludeInherited = true }), table.isfrozen(RS:GetEventsOfClass("Part", { ExcludeInherited = true })))
print("err", Try(function()
	A[1] = 1
end), Try(function()
	(Texture :: any).Name = "x"
end))

-- casts and errors
print("arg", Try(RS.GetPropertiesOfClass, RS), Try(RS.GetPropertiesOfClass, RS, nil), Try(RS.GetPropertiesOfClass, RS, 5), Try(RS.GetPropertiesOfClass, RS, workspace), Try(RS.GetPropertiesOfClass, RS, Enum.Material.Plastic), Try(RS.GetPropertiesOfClass, RS, true))
print("arg", Try(RS.GetPropertiesOfClass, RS, "Part", 5), Try(RS.GetPropertiesOfClass, RS, "Part", { 1 }), Try(RS.GetPropertiesOfClass, RS, "Part", "x"), Try(RS.GetPropertiesOfClass, RS, "Part", true), Try(RS.GetPropertiesOfClass, RS, "Part", { [1] = "x", ExcludeDisplay = true }))
print("arg", Try(RS.GetPropertiesOfClass, RS, nil, 5), Try(RS.GetPropertiesOfClass, RS, "Nope", 5), Try(RS.GetPropertiesOfClass, RS, "Nope", { ExcludeDisplay = 1 }), Try(RS.GetPropertiesOfClass, RS, "CoreGui", { ExcludeDisplay = 1 }), Try(RS.GetPropertiesOfClass, RS, "RomarkRbxAnalyticsService", { ExcludeDisplay = 1 }))
print("field", Try(RS.GetPropertiesOfClass, RS, "Part", { ExcludeInherited = 1 }), Try(RS.GetPropertiesOfClass, RS, "Part", { ExcludeDisplay = "x" }), Try(RS.GetPropertiesOfClass, RS, "Part", { Security = 1 }), Try(RS.GetMethodsOfClass, RS, "Part", { Security = {} }))
print("field", Try(RS.GetPropertiesOfClass, RS, "Part", { ExcludeInherited = 1, ExcludeDisplay = 1, Security = 1 }), Try(RS.GetEventsOfClass, RS, "Part", { ExcludeInherited = 1, ExcludeDisplay = 1 }), Try(RS.GetPropertiesOfClass, RS, "Part", { IsA = 1, Unknown = 1 }), Try(RS.GetPropertiesOfClass, RS, "Part", setmetatable({}, { __index = function() return 1 end })))
print("self", Try(RS.GetPropertiesOfClass), Try(RS.GetPropertiesOfClass, workspace, "Part"), Try(function()
	return RS.GetMethodsOfClass(nil, "Part")
end))
print("class filter", Try(RS.GetClasses, RS, { IsA = 1, ExcludeDisplay = 1, Security = 1 }), Try(RS.GetClasses, RS, { IsA = 1, ExcludeDisplay = 1 }), Try(RS.GetClass, RS, "Nope", { IsA = 1 }), Try(RS.GetClass, RS, "Nope", 5), Try(RS.GetClass, RS, "Part", { IsA = 1 }), Try(RS.GetClass, RS, "CoreGui", { IsA = 1 }), Try(RS.GetClass, RS, "UserGameSettings", { IsA = 1 }))
task.wait(1)
local Messages = {}
local Connection = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageWarning then
		table.insert(Messages, (string.split(Message, "\n")[1]))
	end
end)
RS:GetPropertiesOfClass(5)
RS:GetMethodsOfClass(true)
RS:GetEventsOfClass({})
task.wait(1)
Connection:Disconnect()
print("warnings", #Messages, table.concat(Messages, " | "))

-- Security: what a set lets a game script list and query
for _, Pair in { { "none", None }, { "basic", SecurityCapabilities.new(Enum.SecurityCapability.Basic) }, { "current", Current }, { "current-Basic", Current:Remove(Enum.SecurityCapability.Basic) }, { "current+Plugin", Current:Add(Enum.SecurityCapability.Plugin) }, { "current+WritePlayer", Current:Add(Enum.SecurityCapability.WritePlayer) }, { "current+RobloxScript", Current:Add(Enum.SecurityCapability.RobloxScript) }, { "unassigned", SecurityCapabilities.new(Enum.SecurityCapability.Unassigned) }, { "all", All } } do
	local Label, Security = Pair[1], Pair[2]
	local Parts = { "security", Label, #RS:GetClasses({ Security = Security }) }
	for _, Class in { "Part", "Workspace", "Player", "CoreGui", "Studio", "UserGameSettings", "NetworkClient" } do
		local Filter = { Security = Security }
		local Properties = RS:GetPropertiesOfClass(Class, Filter)
		table.insert(Parts, Class .. ":" .. (if RS:GetClass(Class, Filter) then "c" else "-") .. (if Properties then #Properties else "nil") .. "/" .. Try(RS.GetMethodsOfClass, RS, Class, Filter))
	end
	print(table.concat(Parts, " "))
end
local function PermitsText(Permits)
	local Out = {}
	for Action, Capabilities in Permits do
		table.insert(Out, Action .. "=" .. tostring(Capabilities))
	end
	table.sort(Out)
	return table.concat(Out, ";")
end
local Plugin = Current:Add(Enum.SecurityCapability.Plugin)
print("permits", PermitsText(RS:GetClass("Player", { Security = All }).Permits), "|", PermitsText(RS:GetClass("NetworkClient", { Security = All }).Permits), "|", PermitsText(Find(RS:GetMethodsOfClass("Selection", { Security = Plugin }), "Get").Permits), "|", PermitsText(Find(RS:GetMethodsOfClass("Selection"), "Get").Permits), "|", PermitsText(Find(RS:GetPropertiesOfClass("Workspace", { Security = All }), "StreamingEnabled").Permits))
print("hidden", Describe(Find(RS:GetPropertiesOfClass("Part", { Security = All }), "RobloxLocked")), Describe(Find(RS:GetPropertiesOfClass("Part", { Security = All }), "Attributes")))
print("all classes", #RS:GetClasses({ Security = All }), Describe(RS:GetClass("CoreGui", { Security = All })), Describe(RS:GetClass("Player", { Security = All, ExcludeDisplay = true })))
print("permits held", RS:GetClass("Part").Permits.New, RS:GetClass("Workspace").Permits.GetService, typeof(RS:GetClass("Players").Permits.GetService), RS:GetClass("Part").Permits.GetService)
print("subclasses", table.concat(RS:GetClass("CacheableContentProvider").Subclasses, ","), #RS:GetClass("Instance").Subclasses, #RS:GetClass("Object").Subclasses, table.find(RS:GetClass("Instance").Subclasses, "UserGameSettings") ~= nil, table.find(RS:GetClass("Instance").Subclasses, "RomarkRbxAnalyticsService") ~= nil)

-- whole views by fingerprint: every class's entries in the default view and with every capability, each kind apart
local function Entry(Value)
	local Kind = typeof(Value)
	if Kind == "table" then
		local Named = {}
		for Key in Value do
			table.insert(Named, Key)
		end
		table.sort(Named, function(A, B)
			return tostring(A) < tostring(B)
		end)
		local Parts = {}
		for _, Key in Named do
			table.insert(Parts, tostring(Key) .. "=" .. Entry(Value[Key]))
		end
		return "{" .. table.concat(Parts, ",") .. "}"
	elseif Kind == "SecurityCapabilities" or Kind == "EnumItem" or Kind == "RaycastParams" or Kind == "OverlapParams" or Kind == "User" then
		return Kind .. ":" .. tostring(Value)
	elseif Kind == "number" then
		return P.num(Value)
	end
	return Kind .. ":" .. tostring(Value)
end
local Candidates = {}
for _, Class in RS:GetClasses({ Security = All }) do
	table.insert(Candidates, Class.Name)
	for _, Subclass in Class.Subclasses or {} do
		table.insert(Candidates, Subclass)
	end
end
table.insert(Candidates, "GlobalSettings")
table.insert(Candidates, "UserSettings")
table.sort(Candidates)
for _, View in { { "default", Current }, { "all", All } } do
	local Label, Security = View[1], View[2]
	local Classes = {}
	for _, Class in RS:GetClasses({ Security = Security }) do
		table.insert(Classes, Entry(Class))
	end
	local Parts = { "view", Label, "classes", #Classes, Fnv(table.concat(Classes, "\n")) }
	for _, Pair in { { "properties", RS.GetPropertiesOfClass }, { "methods", RS.GetMethodsOfClass }, { "events", RS.GetEventsOfClass } } do
		local Lines, Answering, Previous = {}, 0, nil
		for _, Name in Candidates do
			if Name ~= Previous then
				Previous = Name
				local List = Pair[2](RS, Name, { Security = Security, ExcludeInherited = true })
				if List then
					Answering += 1
					table.insert(Lines, Name .. ":" .. Entry(List))
				end
			end
		end
		table.insert(Parts, Pair[1])
		table.insert(Parts, Answering)
		table.insert(Parts, Fnv(table.concat(Lines, "\n")))
	end
	print(table.concat(Parts, " "))
end
