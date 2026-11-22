--!nocheck
-- Vector3 as the native vector type (loaded by the prelude; lua/globals.rs).
--
-- In Roblox a Vector3 IS the native vector type (type() "vector", typeof() "Vector3"): the `vector`
-- library and Vector3 work on the same values, and libraries rely on it (Vide's spring keeps every
-- spring's state as vectors and returns a Vector3 spring's vector as its value; with a userdata
-- Vector3 its step raised on the first Vector3 spring, and Vide steps every spring of a VM in one
-- loop, so all of them stopped). Lune's Vector3 is a userdata, so scripts never see it: Vector3
-- values are native vectors (arithmetic, x / y / z, equality and tostring are the VM's), their
-- other members come from the vector type's metatable below. Lune's CFrame takes and returns
-- vectors itself (vendor patch: VectorArg, native_vector); its Ray / Region3 take and return them
-- through their libraries and metatables (ToLune / ToNative).
--
-- Measured in Studio 0.740 (the same script in both): Unit equals vector.normalize and Magnitude
-- vector.magnitude bit for bit, tostring prints each component as the VM prints a number
-- ("0.10000000149011612, 0.3333333432674408, 10000000000"), vectors are table keys by value,
-- getmetatable is locked.
--
-- (A file of its own so the prelude keeps its line numbers: script error traces name them.)

local R, Datatypes, NativeTypeof = ...

local LuneVector3New = Datatypes.Vector3.new
local CreateVector = vector.create

local function ToLune(Value)
	if type(Value) == "vector" then
		return LuneVector3New(Value.x, Value.y, Value.z)
	end
	return Value
end

local function ToNative(Value)
	if type(Value) == "userdata" and NativeTypeof(Value) == "Vector3" then
		return CreateVector(Value.X, Value.Y, Value.Z)
	end
	return Value
end

local function In(...)
	local Count = select("#", ...)
	if Count == 0 then
		return
	elseif Count == 1 then
		return ToLune((...))
	elseif Count == 2 then
		local A, B = ...
		return ToLune(A), ToLune(B)
	elseif Count == 3 then
		local A, B, C = ...
		return ToLune(A), ToLune(B), ToLune(C)
	end
	local Values = table.pack(...)
	for Index = 1, Count do
		Values[Index] = ToLune(Values[Index])
	end
	return table.unpack(Values, 1, Count)
end

local function Out(...)
	local Count = select("#", ...)
	if Count == 0 then
		return
	elseif Count == 1 then
		return ToNative((...))
	elseif Count == 2 then
		local A, B = ...
		return ToNative(A), ToNative(B)
	elseif Count == 3 then
		local A, B, C = ...
		return ToNative(A), ToNative(B), ToNative(C)
	end
	local Values = table.pack(...)
	for Index = 1, Count do
		Values[Index] = ToNative(Values[Index])
	end
	return table.unpack(Values, 1, Count)
end

-- A member read from a Lune datatype: its methods take and return vectors
local ConvertedMethods = setmetatable({}, { __mode = "k" })

local function Converted(Value)
	if type(Value) == "function" then
		local Method = ConvertedMethods[Value]
		if Method == nil then
			Method = function(...)
				return Out(Value(In(...)))
			end
			ConvertedMethods[Value] = Method
		end
		return Method
	end
	return ToNative(Value)
end

-- Members the VM does not provide itself, over the `vector` library
local Magnitude = vector.magnitude
local Normalize = vector.normalize
local Dot = vector.dot
local Cross = vector.cross

local Charge, LerpCost, FuzzyEqCost, Atan2f = R.Charge, R.VectorLerpCost, R.VectorFuzzyEqCost, R.Atan2f

-- A method's arguments as a playing client checks them (Studio 0.747, tools/parity/probes/datatype_errors.luau), from the
-- calling line: self named as a `.` call names it (only a `.` call can pass a wrong one), a later argument as a `:` call
-- reports it ("invalid argument #2 (Vector3 expected, got number)")
local function TypeName(Value)
	return if type(Value) == "vector" then "Vector3" else NativeTypeof(Value)
end

-- (the message for argument Index when it is no Vector3; the method raises it at level 2 for `.` and level 3 past
-- `__namecall` for `:`)
local function VectorProblem(Value, Index, Count, Method, Dotted)
	if type(Value) == "vector" then
		return nil
	end
	local To = if Index == 1 or Dotted then ` to '{Method}'` else ""
	if Index > Count then
		return `missing argument #{Index}{To} (Vector3 expected)`
	end
	return `invalid argument #{Index}{To} (Vector3 expected, got {TypeName(Value)})`
end

local function MakeLerp(Dotted)
	local Level = if Dotted then 2 else 3
	local To3 = if Dotted then " to 'Lerp'" else ""
	return function(...)
		local Count = select("#", ...)
		local Self, Goal, Alpha = ...
		local Problem = VectorProblem(Self, 1, Count, "Lerp", Dotted) or VectorProblem(Goal, 2, Count, "Lerp", Dotted)
		if Problem then
			error(Problem, Level)
		end
		local Number = if type(Alpha) == "number" then Alpha elseif type(Alpha) == "string" then tonumber(Alpha) else nil
		if Number == nil then
			error(if Count < 3 then `missing argument #3{To3} (number expected)` else `invalid argument #3{To3} (number expected, got {TypeName(Alpha)})`, Level)
		end
		Charge(LerpCost)
		return Self + (Goal - Self) * Number
	end
end

local function MakeFuzzyEq(Dotted)
	local Level = if Dotted then 2 else 3
	return function(...)
		local Count = select("#", ...)
		local Self, Other, Epsilon = ...
		local Problem = VectorProblem(Self, 1, Count, "FuzzyEq", Dotted) or VectorProblem(Other, 2, Count, "FuzzyEq", Dotted)
		if Problem then
			error(Problem, Level)
		end
		Charge(FuzzyEqCost)
		local Difference = vector.abs(Self - Other)
		local Limit = (if type(Epsilon) == "number" or type(Epsilon) == "string" then tonumber(Epsilon) else nil) or 1e-5
		return Difference.x <= Limit and Difference.y <= Limit and Difference.z <= Limit
	end
end

local function MakeAngle(Dotted)
	local Level = if Dotted then 2 else 3
	return function(...)
		local Count = select("#", ...)
		local Self, Other, Axis = ...
		if Count < 2 then
			error(`Invalid number of arguments: {Count}, Vector3.Angle expect 2 or 3`, 0)
		end
		local Problem = VectorProblem(Self, 1, Count, "Angle", Dotted) or VectorProblem(Other, 2, Count, "Angle", Dotted)
			or (if Count >= 3 then VectorProblem(Axis, 3, Count, "Angle", Dotted) else nil)
		if Problem then
			error(Problem, Level)
		end
		local Normal = Cross(Self, Other)
		local Angle = Atan2f(Magnitude(Normal), Dot(Self, Other)) -- atan2f in floats, as a playing client (cframe_order probe)
		if Axis ~= nil and Dot(Normal, Axis) < 0 then
			return -Angle
		end
		return Angle
	end
end

local LerpDot, LerpColon = MakeLerp(true), MakeLerp(false)

-- Abs, Ceil, Floor, Sign, Dot, Cross, Max and Min are the `vector` library's functions behind C functions of their own
-- names that check the arguments first (lua/vector_methods.rs)
local Native = R.VectorMethods

local VectorMethods = {
	Dot = Native.Dot,
	Cross = Native.Cross,
	Abs = Native.Abs,
	Ceil = Native.Ceil,
	Floor = Native.Floor,
	Sign = Native.Sign,
	Max = Native.Max,
	Min = Native.Min,
	Lerp = Native.Lerp,
	lerp = Native.Lerp,
	FuzzyEq = MakeFuzzyEq(true),
	Angle = MakeAngle(true),
}

local ColonExtras = {
	FuzzyEq = MakeFuzzyEq(false),
	Angle = MakeAngle(false),
}

-- The VM answers a vector's own members: x / y / z in either case (a key the compiler could not fold, `v[axis]`,
-- included) and Magnitude / Unit in either case (luaV_vectormember in patches/luau/05-vector-members.patch: the work clock charges them
-- Studio's 10-12 ns, where a function here cost a 15 ns host call). Methods are a table lookup, as in Studio
-- (a:Dot(b) 9-10 ns); the table's own metatable raises Studio's error for any other member, from the script's line.
R.SetVectorMetatable({
	__index = setmetatable(VectorMethods, {
		__index = function(_, Key)
			-- (a key that is no string is the client's Luau check, from the calling line: `Vector3.one[1]` raises "invalid
			-- argument #2 (string expected, got number)" in Studio 0.747, tools/parity/probes/datatype_errors.luau)
			if type(Key) ~= "string" then
				error(`invalid argument #2 (string expected, got {TypeName(Key)})`, 2)
			end
			error(`{Key} is not a valid member of Vector3`, 0)
		end,
	}),
	__namecall = R.MakeVectorNamecall(ColonExtras),
	-- an assignment raises Studio's "X cannot be assigned to" for any string or number key (a vector is a value), any other
	-- key the client's Luau check from the calling line (Studio 0.747, tools/parity/probes/datatype_errors.luau)
	__newindex = function(_, Key)
		if type(Key) ~= "string" and type(Key) ~= "number" then
			error(`invalid argument #2 (string expected, got {TypeName(Key)})`, 2)
		end
		error(`{Key} cannot be assigned to`, 0)
	end,
	__metatable = "The metatable is locked",
	-- The VM names a vector by this table's __type in its own errors, as a playing client does (Studio 0.748): "attempt to
	-- get length of a Vector3 value", "invalid argument #1 to 'floor' (number expected, got Vector3)"; type() still answers
	-- "vector", as there
	__type = "Vector3",
})

-- Lune's Ray / Region3 values: members and operators through ToLune / ToNative; Aliases
-- maps the deprecated names Roblox still answers to their current ones
local function ConvertMetatable(Sample, Aliases)
	local Meta = R.RawMetatable(Sample)
	local Index = rawget(Meta, "__index")
	if type(Index) == "function" then
		rawset(Meta, "__index", function(Self, Key)
			return Converted(Index(Self, Aliases and Aliases[Key] or Key))
		end)
	elseif type(Index) == "table" then
		rawset(Meta, "__index", function(_, Key)
			return Converted(Index[Aliases and Aliases[Key] or Key])
		end)
	end
	for _, Key in { "__add", "__sub", "__mul", "__div" } do
		local Operator = rawget(Meta, Key)
		if type(Operator) == "function" then
			rawset(Meta, Key, function(A, B)
				return ToNative(Operator(ToLune(A), ToLune(B)))
			end)
		end
	end
end

-- Lune's CFrame / Ray / Region3 all take and return native vectors directly (vendor patch: VectorArg, native_vector)
local VectorLibraries = {}

-- The script library's own members (the prelude's ScriptLibrary takes these before Lune's)
local Library = {
	zero = vector.zero,
	one = vector.one,
	xAxis = CreateVector(1, 0, 0),
	yAxis = CreateVector(0, 1, 0),
	zAxis = CreateVector(0, 0, 1),
	min = vector.min,
	max = vector.max,
}

-- Vector3.FromNormalId and FromAxis (and their lowercase names) take the item as a playing client's reflection casts it
-- (Studio 0.747, tools/parity/probes/datatype_errors.luau): an item of the enum, its value as a number (1 is Top, Y,
-- truncated: 1.5 too) or its name;
-- none or nil is "Argument 1 missing or nil", anything else "Unable to cast string to NormalId", without a position
local EnumItemName, EnumItemType, GetEnumItem = R.EnumItemName, R.EnumItemType, R.GetEnumItem

local function CastName(Value)
	local Kind = type(Value)
	if Kind == "number" then
		return "double"
	elseif Kind == "boolean" then
		return "bool"
	elseif Kind == "table" then
		return if next(Value) == nil or #Value > 0 then "Array" else "Dictionary"
	elseif Kind == "function" then
		return "Function"
	elseif Kind == "vector" then
		return "Vector3"
	end
	local Type = EnumItemType(Value) or NativeTypeof(Value)
	return if Type == "CFrame" then "CoordinateFrame" else Type
end

-- (the items' values, as Roblox's API dump lists them)
local NormalIdNames = { [0] = "Right", "Top", "Back", "Left", "Bottom", "Front" }
local AxisNames = { [0] = "X", "Y", "Z" }

local function EnumArgument(Value, EnumName, Names)
	if Value == nil then
		error("Argument 1 missing or nil", 0)
	end
	local Name = if EnumItemType(Value) == EnumName then EnumItemName(Value)
		elseif type(Value) == "number" then Names[if Value >= 0 then math.floor(Value) else math.ceil(Value)] -- (1.5 is 1)
		elseif type(Value) == "string" and GetEnumItem(EnumName, Value) ~= nil then Value
		else nil
	if Name == nil then
		error(`Unable to cast {CastName(Value)} to {EnumName}`, 0)
	end
	return Name
end

local Directions = {
	Right = CreateVector(1, 0, 0),
	Left = CreateVector(-1, 0, 0),
	Top = CreateVector(0, 1, 0),
	Bottom = CreateVector(0, -1, 0),
	Back = CreateVector(0, 0, 1),
	Front = CreateVector(0, 0, -1),
}
local Axes = { X = CreateVector(1, 0, 0), Y = CreateVector(0, 1, 0), Z = CreateVector(0, 0, 1) }

function Library.FromNormalId(Item)
	return Directions[EnumArgument(Item, "NormalId", NormalIdNames)]
end

function Library.FromAxis(Item)
	return Axes[EnumArgument(Item, "Axis", AxisNames)]
end

Library.fromNormalId = Library.FromNormalId
Library.fromAxis = Library.FromAxis

return ToLune, ToNative, In, Out, VectorLibraries, Library
