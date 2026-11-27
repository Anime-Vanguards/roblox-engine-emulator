-- What ReflectionService answers a game script beyond its own default view, for data/reflection-security.json: every class
-- and member a Security filter can show, and the capabilities each class, property and permit needs.
-- Run it as a game script (identity 2) in a playing client: the default view depends on who asks, and data/reflection.json
-- is a game script's. It returns a JSON string; `node tools/reflection-security.mjs <that file>` checks it against
-- data/reflection.json and writes data/reflection-security.json (docs/engine.md "Data files").
--
-- What it reads (each measured to hold on a 0.747 client, notes in docs/engine.md "ReflectionService"): GetClasses and the
-- member queries with Security = every capability give every class and member there is; one capability taken out of that
-- set hides what needs it, so the capabilities a class's listing, its member queries and each property need are found one
-- capability at a time.

local HttpService = game:GetService("HttpService")
local ReflectionService = game:GetService("ReflectionService")

local Items = Enum.SecurityCapability:GetEnumItems()
local All = SecurityCapabilities.new(table.unpack(Items))

local Encode
function Encode(Value)
	local Kind = typeof(Value)
	if Kind == "nil" then
		return nil
	elseif Kind == "boolean" or Kind == "string" then
		return Value
	elseif Kind == "number" then
		if Value ~= Value then
			return { ["$t"] = "number", s = "nan" }
		elseif Value == math.huge then
			return { ["$t"] = "number", s = "inf" }
		elseif Value == -math.huge then
			return { ["$t"] = "number", s = "-inf" }
		end
		return Value
	elseif Kind == "EnumItem" then
		return { ["$t"] = "EnumItem", e = tostring(Value.EnumType), n = Value.Name }
	elseif Kind == "Vector2" then
		return { ["$t"] = Kind, v = { Value.X, Value.Y } }
	elseif Kind == "Vector3" then
		return { ["$t"] = Kind, v = { Value.X, Value.Y, Value.Z } }
	elseif Kind == "CFrame" then
		return { ["$t"] = Kind, v = { Value:GetComponents() } }
	elseif Kind == "table" then
		local Items_ = {}
		for Key, Item in Value do
			Items_[tostring(Key)] = Encode(Item)
		end
		return { ["$t"] = Kind, v = Items_ }
	end
	return { ["$t"] = Kind, s = tostring(Value) }
end

-- a table of text values (Permits' SecurityCapabilities as tostring gives them, Display's texts)
local function Text(Table)
	local Out = {}
	for Key, Item in Table or {} do
		Out[tostring(Key)] = tostring(Item)
	end
	return Out
end
local function TypeOf(TypeInfo)
	return { script = TypeInfo.ScriptType, engine = TypeInfo.EngineType, enum = TypeInfo.EnumType, instance = TypeInfo.InstanceType }
end
local function Parameters(List)
	local Result = {}
	for _, Parameter in List or {} do
		table.insert(Result, { name = Parameter.Name, type = TypeOf(Parameter.Type), default = Encode(Parameter.DefaultValue) })
	end
	return Result
end

-- every class: those GetClasses lists with every capability, their Subclasses (a class GetClasses never lists is still named
-- as a subclass: the settings classes under Instance), and the classes of the settings roots (settings(), which a game script
-- may not call, and UserSettings()), which no list names
local Classes = {}
local Names, Seen = {}, {}
local function Add(Name, Superclass)
	if Name and not Seen[Name] then
		Seen[Name] = { superclass = Superclass }
		table.insert(Names, Name)
	end
end
for _, Class in ReflectionService:GetClasses({ Security = All }) do
	table.insert(Classes, { name = Class.Name, superclass = Class.Superclass, serialized = Class.Serialized, display = Text(Class.Display), permits = Text(Class.Permits) })
	Add(Class.Name, Class.Superclass)
end
for _, Class in ReflectionService:GetClasses({ Security = All }) do
	for _, Subclass in Class.Subclasses or {} do
		Add(Subclass, Class.Name)
	end
end
-- (GlobalSettings and UserSettings are GenericSettings's, which no list gives subclasses for)
Add("GlobalSettings", "GenericSettings")
Add(UserSettings().ClassName, "GenericSettings")
table.sort(Names)
local Default = {}
for _, Class in ReflectionService:GetClasses() do
	Default[Class.Name] = true
end

local function Own(Name, Security)
	local Filter = { Security = Security, ExcludeInherited = true }
	return ReflectionService:GetPropertiesOfClass(Name, Filter), ReflectionService:GetMethodsOfClass(Name, Filter), ReflectionService:GetEventsOfClass(Name, Filter)
end

-- each class's own members with every capability, and the names a game script's default view lists
local Members = {}
for _, Name in Names do
	local Properties, Methods, Events = Own(Name, All)
	if Properties then
		local Entry = { properties = {}, methods = {}, events = {}, default = { properties = {}, methods = {}, events = {} } }
		for _, Property in Properties do
			table.insert(Entry.properties, { name = Property.Name, type = TypeOf(Property.Type), serialized = Property.Serialized, content = if Property.ContentType then Property.ContentType.Name else nil, permits = Text(Property.Permits), display = Text(Property.Display) })
		end
		for _, Method in Methods do
			table.insert(Entry.methods, { name = Method.Name, returns = TypeOf(Method.ReturnType or {}), parameters = Parameters(Method.Parameters), permits = Text(Method.Permits), display = Text(Method.Display) })
		end
		for _, Event in Events do
			table.insert(Entry.events, { name = Event.Name, parameters = Parameters(Event.Parameters), permits = Text(Event.Permits), display = Text(Event.Display) })
		end
		local DefaultProperties, DefaultMethods, DefaultEvents = Own(Name, nil)
		for Kind, List in { properties = DefaultProperties, methods = DefaultMethods, events = DefaultEvents } do
			for _, Member in List or {} do
				Entry.default[Kind][Member.Name] = Text(Member.Permits)
			end
		end
		Entry.answers = DefaultProperties ~= nil
		Members[Name] = Entry
	end
end

-- the capabilities each class's listing, its member queries and each own property need: what taking one out hides
local Gates = { list = {}, members = {}, visible = {} }
local function Note(Table, Key, Capability)
	Table[Key] = Table[Key] or {}
	table.insert(Table[Key], Capability)
end
for _, Item in Items do
	local Security = All:Remove(Item)
	local Listed = {}
	for _, Class in ReflectionService:GetClasses({ Security = Security, ExcludeDisplay = true }) do
		Listed[Class.Name] = true
	end
	for _, Class in Classes do
		if not Listed[Class.name] then
			Note(Gates.list, Class.name, Item.Name)
		end
	end
	for Name, Entry in Members do
		local Properties = ReflectionService:GetPropertiesOfClass(Name, { Security = Security, ExcludeInherited = true, ExcludeDisplay = true })
		if Properties == nil then
			Note(Gates.members, Name, Item.Name)
		else
			local Shown = {}
			for _, Property in Properties do
				Shown[Property.Name] = true
			end
			for _, Property in Entry.properties do
				if not Shown[Property.name] then
					Note(Gates.visible, Name .. "." .. Property.name, Item.Name)
				end
			end
		end
	end
end

local Superclasses = {}
for Name, Info in Seen do
	Superclasses[Name] = Info.superclass
end
return HttpService:JSONEncode({
	version = version(),
	capabilities = (function()
		local Out = {}
		for _, Item in Items do
			table.insert(Out, Item.Name)
		end
		return Out
	end)(),
	classes = Classes,
	default = Default,
	superclasses = Superclasses,
	members = Members,
	gates = Gates,
})
