--!nocheck
-- Rust engine prelude (native/src/engine/lua): the Luau side of the engine's script API.
--
-- 1. The call protocol: Rust functions return values normally, or SIGNAL followed by
--    (1, message, level) to raise a string error (level 1: the position of the script line that
--    called the engine function, 0: none) or (2) to yield the calling coroutine (the Rust side has
--    already registered its resumption). `wrap` turns such a function into a script function.
-- 2. typeof with registered proxy types.
-- 3. Script libraries over Lune's datatypes (port of runtime/engine/datatypes.luau ScriptLibrary,
--    constructor overrides and extras) and the small proxy types datatypes.luau implements in Luau
--    (Faces, Axes, Path2DControlPoint, parameter objects).
-- 4. Pieces of the shared global environment that are plain Luau (runtime/engine/globals.luau).

local R, Roblox = ...
local SIGNAL = R.SIGNAL

local error = error
local select = select
local yield = coroutine.yield
local NativeTypeof = typeof

-- (exactly the values the engine function returned: one that returns nothing returns no values, as Roblox's do,
-- `select("#", Instance.ClearAllChildren(Part))` is 0)
local function Pass(...)
	local First = ...
	if First == SIGNAL then
		local _, Kind, Message, Level = ...
		if Kind == 1 then
			if Level == 0 then
				error(Message, 0)
			end
			-- 1 = Pass, 2 = the wrapper, 3 = its caller
			error(Message, Level + 2)
		end
		return yield()
	end
	return ...
end

local function Wrap(Function)
	return function(...)
		return Pass(Function(...))
	end
end

-- Instance __index / __newindex: fixed-arity Rust functions returning (value, message, level)
local function WrapIndex(Function)
	return function(Self, Key)
		local Value, Message, Level = Function(Self, Key)
		if Message then
			error(Message, if Level == 0 then 0 else Level + 1)
		end
		return Value
	end
end

local function WrapNewIndex(Function)
	return function(Self, Key, Value)
		local Message, Level = Function(Self, Key, Value)
		if Message then
			error(Message, if Level == 0 then 0 else Level + 1)
		end
	end
end

-- typeof

local Registry = setmetatable({}, { __mode = "k" })

local function Typeof(Value)
	local Native = NativeTypeof(Value)
	-- Only tables and proxies are registered (engine userdata carry their __type)
	if Native == "table" or Native == "userdata" then
		return Registry[Value] or Native
	end
	-- In Roblox the native vector type is Vector3
	if Native == "vector" then
		return "Vector3"
	end
	return Native
end

local function Register(Value, TypeName)
	Registry[Value] = TypeName
	return Value
end

local Members = {} -- (a proxy's errors for keys its type has no member for: Members.ReadMiss / WriteMiss, below the traced functions)

local function ReadOnlyMember(_, Key)
	error(`{Key} cannot be assigned to`, 0)
end

local function Proxy(TypeName, Index, NewIndex, Extra)
	local Object = newproxy(true)
	local Meta = getmetatable(Object)
	Meta.__metatable = "The metatable is locked"
	Meta.__index = Index
	Meta.__newindex = NewIndex or ReadOnlyMember
	if Extra then
		for Key, Value in Extra do
			Meta[Key] = Value
		end
	end
	Register(Object, TypeName)
	return Object
end

-- Datatypes (datatypes.luau)

local Datatypes = {}
for _, Name in {
	"Axes", "BrickColor", "CFrame", "Color3", "ColorSequence", "ColorSequenceKeypoint", "Faces",
	"NumberRange", "NumberSequence", "NumberSequenceKeypoint", "PhysicalProperties", "Ray", "Rect",
	"Region3", "Region3int16", "UDim", "UDim2", "Vector2", "Vector2int16", "Vector3", "Vector3int16",
} do
	Datatypes[Name] = Roblox[Name]
end

local Color3 = Datatypes.Color3
local Vector2 = Datatypes.Vector2
local ToLune, ToNative, In, Out, VectorLibraries, VectorLibrary = R.Vector3(R, Datatypes, NativeTypeof) -- (lua/vector3.luau)
local UDim = Datatypes.UDim
local UDim2 = Datatypes.UDim2

-- Lune's two-value sequence constructors reuse the first value for the end keypoint
do
	local LuneColorSequence = Datatypes.ColorSequence
	local LuneNumberSequence = Datatypes.NumberSequence
	Datatypes.ColorSequence = setmetatable({
		new = function(First, Second)
			if NativeTypeof(First) == "Color3" then
				local Last = if NativeTypeof(Second) == "Color3" then Second else First
				return LuneColorSequence.new({
					Datatypes.ColorSequenceKeypoint.new(0, First),
					Datatypes.ColorSequenceKeypoint.new(1, Last),
				})
			end
			return LuneColorSequence.new(First)
		end,
	}, { __index = LuneColorSequence })
	Datatypes.NumberSequence = setmetatable({
		new = function(First, Second)
			if type(First) == "number" then
				local Last = if type(Second) == "number" then Second elseif Second == nil then First else error("NumberSequence.new() arg #2: Number expected.", 0)
				return LuneNumberSequence.new({
					Datatypes.NumberSequenceKeypoint.new(0, First),
					Datatypes.NumberSequenceKeypoint.new(1, Last),
				})
			end
			return LuneNumberSequence.new(First)
		end,
	}, { __index = LuneNumberSequence })
end

local function RobloxErrorMessage(Error, FunctionName)
	local Text = tostring(Error)
	Text = string.match(Text, "^(.-)\nstack traceback:") or Text
	Text = string.gsub(Text, "^runtime error: ", "")
	local Index, From, To = string.match(Text, "^bad argument #(%d+)[^:]*: error converting Lua (%w+) to ([%w_]+)")
	if Index then
		local Expected = To
		if string.match(To, "^[fiu]%d+$") then
			Expected = "number"
		elseif To == "userdata" then
			Expected = string.match(Text, "of type '([%w]+)'") or "userdata"
		end
		return `invalid argument #{Index} to '{FunctionName}' ({Expected} expected, got {From})`
	end
	return Text
end

local function PassResults(FunctionName, Ok, ...)
	if Ok then
		return ...
	end
	local Failure = ...
	if type(Failure) == "string" then
		error(Failure, 0)
	end
	error(RobloxErrorMessage(Failure, FunctionName), 0)
end

local function Float(Value)
	if type(Value) == "number" then
		return Value
	end
	return tonumber(Value) or 0
end

local function Int32(Value)
	local Number = Float(Value)
	if Number ~= Number or Number >= 2147483648 or Number < -2147483648 then
		return -2147483648
	end
	return if Number >= 0 then math.floor(Number) else math.ceil(Number)
end

local function VectorParts(Value)
	if NativeTypeof(Value) == "vector" then
		return Value.x, Value.y, Value.z
	end
	return Value.X, Value.Y, Value.Z
end

-- CFrame has no override here: Lune's CFrame takes and returns native vectors itself (vendor patch:
-- VectorArg, native_vector; CFrame.new(from, to) looks at the target), so a script's CFrame math
-- runs as native code, as in Roblox, and the library wrapper below only translates its errors.
-- (These comment lines stand where the CFrame.new override was: the prelude's line numbers, which
-- script error traces name, stay as they were.)
--
--
-- The other overrides pass a default or clamp what Lune's constructors would reject or wrap
-- (Float, Int32; the Roblox rules are in the constructors test of lua/datatype_tests.rs):
local ConstructorOverrides = {
	UDim = {
		new = function(Scale, Offset)
			return UDim.new(Float(Scale), Int32(Offset))
		end,
	},
	UDim2 = {
		new = function(...)
			local First = ...
			if NativeTypeof(First) == "UDim" then
				local Second = select(2, ...)
				return PassResults("new", pcall(UDim2.new, First, Second))
			end
			local XScale, XOffset, YScale, YOffset = ...
			return UDim2.new(Float(XScale), Int32(XOffset), Float(YScale), Int32(YOffset))
		end,
		fromOffset = function(X, Y)
			return UDim2.fromOffset(Int32(X), Int32(Y))
		end,
		fromScale = function(X, Y)
			return UDim2.fromScale(Float(X), Float(Y))
		end,
	},
	Vector2 = {
		new = function(X, Y)
			return Vector2.new(Float(X), Float(Y))
		end,
	},
	Vector3 = {
		new = function(X, Y, Z)
			return vector.create(Float(X), Float(Y), Float(Z))
		end,
	},
	Color3 = {
		new = function(Red, Green, Blue)
			return Color3.new(Float(Red), Float(Green), Float(Blue))
		end,
		fromRGB = function(Red, Green, Blue)
			return Color3.new(Float(Red) / 255, Float(Green) / 255, Float(Blue) / 255)
		end,
	},
}

local EnumItemName = R.EnumItemName
local EnumItemType = R.EnumItemType
local GetEnumItem = R.GetEnumItem

local LibraryExtras = {
	Color3 = {
		toHSV = function(Color)
			return Color:ToHSV()
		end,
	},
	Vector3 = {
		FromNormalId = function(Item)
			local Name = EnumItemName(Item)
			local Directions = {
				Right = vector.create(1, 0, 0),
				Left = vector.create(-1, 0, 0),
				Top = vector.create(0, 1, 0),
				Bottom = vector.create(0, -1, 0),
				Back = vector.create(0, 0, 1),
				Front = vector.create(0, 0, -1),
			}
			return Directions[Name or ""] or vector.zero
		end,
		FromAxis = function(Item)
			local Name = EnumItemName(Item)
			return if Name == "X" then vector.create(1, 0, 0) elseif Name == "Y" then vector.create(0, 1, 0) elseif Name == "Z" then vector.create(0, 0, 1) else vector.zero
		end,
	},
	CFrame = {
		lookAlong = function(...) -- (what it was given: an up passed as nil is no Vector3, none is the default)
			return Datatypes.CFrame.lookAlong(...)
		end,
	},
	BrickColor = {
		Random = function()
			return Datatypes.BrickColor.random()
		end,
	},
}
LibraryExtras.Vector3.fromNormalId = LibraryExtras.Vector3.FromNormalId
LibraryExtras.Vector3.fromAxis = LibraryExtras.Vector3.FromAxis; for Key, Value in VectorLibrary do ConstructorOverrides.Vector3[Key] = Value end -- (native zero, one, axes, min, max)

local ScriptLibraries = {}

local function ScriptLibrary(Name)
	local Existing = ScriptLibraries[Name]
	if Existing then
		return Existing
	end
	local Library = Datatypes[Name]
	local Extras = LibraryExtras[Name] or {}
	local Overrides = ConstructorOverrides[Name] or {}
	local Wrapped, Converts = {}, VectorLibraries[Name]
	local function Resolve(_, Key)
		local Value = Overrides[Key]
		if Value ~= nil then
			return Value
		end
		Value = Extras[Key]
		if Value == nil then
			Value = Library[Key]
		end
		if type(Value) ~= "function" then
			return Value
		end
		local Cached = Wrapped[Key]
		if Cached == nil then
			Cached = if Converts then function(...) return Out(PassResults(Key, pcall(Value, In(...)))) end else function(...)
				return PassResults(Key, pcall(Value, ...))
			end
			Wrapped[Key] = Cached
		end
		return Cached
	end
	-- Members resolve once, then come from a plain __index table (the VM's fast path)
	local Members = {}
	setmetatable(Members, {
		__index = function(_, Key)
			local Value = Resolve(nil, Key)
			if Value ~= nil then
				rawset(Members, Key, Value)
			end
			return Value
		end,
	})
	local Result = setmetatable({}, {
		__index = Members,
		__newindex = function() -- (a library is a frozen table in a client: Studio 0.747, datatype_errors.luau)
			error("attempt to modify a readonly table", 2)
		end,
		__metatable = "The metatable is locked",
	})
	ScriptLibraries[Name] = Result
	return Result
end

-- Faces / Axes (Lune's versions only accept Lune's own EnumItems)

local function FlagSet(TypeName, Names, Resolve, Shown)
	local Library = {}
	local Info = setmetatable({}, { __mode = "k" })
	local Meta = {
		__eq = function(A, B)
			local X, Y = Info[A], Info[B]
			if X == nil or Y == nil then
				return false
			end
			for _, Name in Names do
				if X[Name] ~= Y[Name] then
					return false
				end
			end
			return true
		end,
		__tostring = function(Self) -- (the names in Shown's order: Roblox prints Faces by NormalId value, Axes as X, Y, Z only)
			local Parts = {}
			for _, Name in Shown or Names do
				if Info[Self][Name] then
					table.insert(Parts, Name)
				end
			end
			return table.concat(Parts, ", ")
		end,
	}
	function Library.new(...)
		local Flags = {}
		for _, Name in Names do
			Flags[Name] = false
		end
		for Index = 1, select("#", ...) do
			local Item = select(Index, ...)
			for _, Name in Resolve(Item) do
				Flags[Name] = true
			end
		end
		local Object = Proxy(TypeName, function(Self, Key)
			local Value = Info[Self][Key]
			if Value == nil then
				Members.ReadMiss(TypeName, Key, true)
			end
			return Value
		end, function(Self, Key) Members.WriteMiss(TypeName, Key, Info[Self][Key] ~= nil) end, Meta) -- (every flag is read-only)
		Info[Object] = Flags
		return Object
	end
	return Library
end

local FaceNames = { "Top", "Bottom", "Left", "Right", "Back", "Front" }
local AxisFaces = { X = { "Left", "Right" }, Y = { "Top", "Bottom" }, Z = { "Back", "Front" } }

local Faces = FlagSet("Faces", FaceNames, function(Item)
	local Name = EnumItemName(Item)
	return if Name and EnumItemType(Item) == "NormalId" then { Name } else {}
end, { "Right", "Top", "Back", "Left", "Bottom", "Front" })

local Axes = FlagSet("Axes", { "X", "Y", "Z", "Top", "Bottom", "Left", "Right", "Back", "Front" }, function(Item)
	local Name = EnumItemName(Item)
	local TypeName = EnumItemType(Item)
	if TypeName == "Axis" and Name then
		return { Name, table.unpack(AxisFaces[Name] or {}) }
	elseif TypeName == "NormalId" and Name then
		for Axis, List in AxisFaces do
			if table.find(List, Name) then
				return { Name, Axis }
			end
		end
	end
	return {}
end, { "X", "Y", "Z" })

-- datatypes.luau replaces Lune's Faces / Axes with these before the script libraries wrap them
Datatypes.Faces = Faces
Datatypes.Axes = Axes

local Path2DControlPoint = {}
-- (Path2DControlPoint.new and its values are defined below the traced functions, beside the other
-- additions; these lines keep the count of the ones that built it here: rur's golden traces record
-- prelude line numbers.)
--
--
--
--
--
--
--
--
--
--
--

local function ParamsType(TypeName, Defaults, ReadOnly, Meta) -- (Defaults has every member; its key, quoted, for any other key)
	return {
		new = function(Initial)
			local Data = table.clone(Defaults)
			for Key, Value in Initial or {} do
				Data[Key] = Value
			end
			return Proxy(TypeName, function(_, Key)
				if Defaults[Key] == nil then Members.ReadMiss(TypeName, Key, true) end return Data[Key]
			end, function(_, Key, Value)
				if ReadOnly or Defaults[Key] == nil then Members.WriteMiss(TypeName, Key, Defaults[Key] ~= nil) end Data[Key] = Value
			end, Meta and Meta(Data))
		end,
	}
end

local function InertLibrary(TypeName)
	return {
		new = function(...)
			local Fields = table.pack(...)
			return Proxy(TypeName, function(_, Key)
				return Fields[Key]
			end)
		end,
	}
end

-- Shared environment pieces (globals.luau)

local function MakeStubTree(Name)
	local Cache = {}
	return setmetatable({}, {
		__index = function(_, Key)
			if Key == "GetFFlag" or Key == "GetFVariable" or Key == "GetFInt" then
				return function()
					return nil
				end
			end
			local Existing = Cache[Key]
			if Existing == nil then
				Existing = MakeStubTree(`{Name}.{Key}`)
				Cache[Key] = Existing
			end
			return Existing
		end,
		__newindex = function(_, Key, Value)
			Cache[Key] = Value
		end,
		__tostring = function()
			return Name
		end,
		__call = function()
			return MakeStubTree(`{Name}()`)
		end,
	})
end

-- Roblox's utf8 additions: graphemes, and the normalization functions (lua/normalize.rs: a client's rules, not Unicode's)
local Utf8Table = table.clone(utf8)
local function Extends(Code)
	return (Code >= 0x0300 and Code <= 0x036F)
		or (Code >= 0x1AB0 and Code <= 0x1AFF)
		or (Code >= 0x1DC0 and Code <= 0x1DFF)
		or (Code >= 0x20D0 and Code <= 0x20FF)
		or (Code >= 0xFE00 and Code <= 0xFE0F)
		or (Code >= 0xFE20 and Code <= 0xFE2F)
		or (Code >= 0x1F3FB and Code <= 0x1F3FF)
		or (Code >= 0xE0020 and Code <= 0xE007F)
		or (Code >= 0xE0100 and Code <= 0xE01EF)
		or Code == 0x200D
end
Utf8Table.graphemes = function(Text, First, Last)
	local Clusters = {}
	local Previous = nil
	local Joining = false
	local Flags = 0
	for Position, Code in utf8.codes(string.sub(Text, 1, Last or #Text)) do
		if Position < (First or 1) then
			continue
		end
		local Continue = Previous ~= nil and (Extends(Code) or Joining or (Previous == 13 and Code == 10))
		if Code >= 0x1F1E6 and Code <= 0x1F1FF then
			Continue = Continue or (Flags % 2 == 1)
			Flags += 1
		else
			Flags = 0
		end
		if Continue and #Clusters > 0 then
			Clusters[#Clusters][2] = Position + #utf8.char(Code) - 1
		else
			table.insert(Clusters, { Position, Position + #utf8.char(Code) - 1 })
		end
		Joining = Code == 0x200D
		Previous = Code
	end
	local Index = 0
	return function()
		Index += 1
		local Cluster = Clusters[Index]
		if Cluster then
			return Cluster[1], Cluster[2]
		end
		return nil
	end
end
-- (Rust, lua/normalize.rs: nfc composes each code point with the one before it in the output only, nfd writes full
-- canonical decompositions in place, unordered, a U+0000 after a singleton's; measured in a 0.747 client and server,
-- tools/parity/probes/utf8_normalize.luau. As many lines as the identity stubs had: the lines below keep their
-- numbers, which tracebacks through the prelude record)
Utf8Table.nfcnormalize = Wrap(R.NfcNormalize)
Utf8Table.nfdnormalize = Wrap(R.NfdNormalize)

local DebugTable = table.clone(debug)
DebugTable.profilebegin = function() end
DebugTable.profileend = function() end
DebugTable.setmemorycategory = function() end
DebugTable.resetmemorycategory = function() end
DebugTable.dumpcodesize = function() end
DebugTable.getmemorycategory = function()
	return "default"
end

-- task.wait / wait / Signal:Wait / WaitForChild yield in Luau after the Rust side registered the
-- thread (or refused to: Pass raises it, lua/refs.rs); everything else is a Rust function wrapped for string errors.
local ScheduleWait = R.ScheduleWait
local ScheduleLegacyWait = R.ScheduleLegacyWait

local Task = {
	spawn = Wrap(R.TaskSpawn),
	defer = Wrap(R.TaskDefer),
	delay = Wrap(R.TaskDelay),
	wait = function(Duration)
		Pass(ScheduleWait(Duration))
		return yield()
	end,
	-- (the running thread, and one waiting on a coroutine it resumed, cannot be cancelled: Roblox raises this without a
	-- position; measured in a playing client, Studio 0.744, tools/parity/probes/task_order.luau)
	cancel = function(...)
		local Thread = ...
		if type(Thread) == "thread" then
			local Status = coroutine.status(Thread)
			if Status == "running" or Status == "normal" then
				error("cannot cancel thread", 0)
			end
		end
		return Pass(R.TaskCancel(...))
	end,
	synchronize = function() if Pass(R.ParallelPark(true)) then yield() end end, -- (a parallel handler's thread leaves the phase: actors.rs)
	desynchronize = function() if Pass(R.ParallelPark(false)) then yield() end end,
}

local function LegacyWait(Duration)
	Pass(ScheduleLegacyWait(Duration))
	return yield()
end

local function Traceback(Thread)
	return debug.traceback(Thread)
end

-- print / warn (output.luau Stringify)
local Output = R.Output
local function Stringify(...)
	local Count = select("#", ...)
	local Parts = table.create(Count)
	for Index = 1, Count do
		Parts[Index] = tostring((select(Index, ...)))
	end
	return table.concat(Parts, " ")
end
local function Print(...)
	Output("print", Stringify(...))
end
local function Warn(...)
	Output("warn", Stringify(...))
end

-- require (modules.luau Require): the Rust side keeps the module records; the chunk runs here so
-- a module may yield while it loads. RequireBegin answers
--   0, value            the module's (cached) result
--   1, message, level   an error (level 1: the requiring script's line, 0: none)
--   2                   loading on another thread: wait (resumed when it finishes), then retry
--   3, chunk, key       run the chunk, then report through RequireFinish
local RequireBegin = R.RequireBegin
local RequireFinish = R.RequireFinish
local pcall = pcall
local function Require(Target, Caller, Vm)
	while true do
		local Status, A, B = RequireBegin(Target, Caller, Vm)
		if Status == 0 then
			return A
		elseif Status == 1 then
			if B == 0 then
				error(A, 0)
			end
			-- 1 = Require, 2 = the script's require function, 3 = the requiring line
			error(A, 3)
		elseif Status == 2 then
			yield()
		else
			local Finished, Message, Level = RequireFinish(B, table.pack(pcall(A)), Vm)
			if Finished then
				return Message
			end
			if Level == 0 then
				error(Message, 0)
			end
			error(Message, 3)
		end
	end
end

-- Engine.Call's thread body: the callback's results or failure (message + traceback) land in Box
local function CallBody(Box, Callback, ...)
	local Packed = table.pack(xpcall(Callback, function(Message)
		return { Message = Message, Trace = debug.traceback(nil, 2) }
	end, ...))
	if Packed[1] then
		Box.Results = Packed
	else
		Box.Failure = Packed[2]
		Box.Message = tostring(Packed[2].Message)
	end
end

local VmGlobals = {} -- an Actor's _G, by its key (modules.rs vm_key): its VM's own, as in Studio
local function Environment(Shared, Script, Vm)
	local Env = {
		script = Script,
		require = function(Target)
			return Require(Target, Script, Vm)
		end,
	}
	if Vm ~= nil then
		local Globals = VmGlobals[Vm]
		if Globals == nil then
			Globals = {}
			VmGlobals[Vm] = Globals
		end
		Env._G = Globals
	end
	return setmetatable(Env, { __index = Shared })
end

-- RaycastParams / OverlapParams:AddToFilter(Instance | {Instance}) adds to FilterDescendantsInstances
-- (documented), as a new list: a table the script assigned earlier is not changed under it. (Kept below
-- the traced functions: rur's golden traces record prelude line numbers.)
local FilterMethods = {
	AddToFilter = function(Data, Instances)
		local List = table.clone(Data.FilterDescendantsInstances or {})
		if Typeof(Instances) == "Instance" then
			table.insert(List, Instances)
		elseif type(Instances) == "table" then
			for _, Item in Instances do
				if Typeof(Item) ~= "Instance" then
					error(`Unable to cast {Typeof(Item)} to Instance`, 3)
				end
				table.insert(List, Item)
			end
		else
			error(`Unable to cast {Typeof(Instances)} to Instance`, 3)
		end
		Data.FilterDescendantsInstances = List
	end,
}

-- RaycastParams / OverlapParams. Their instance lists, FilterDescendantsInstances and Roblox's spatial query filters of April
-- 2026, ExcludeInstances and IncludeInstances ({Instance}?, nil until assigned), hold a copy of the table assigned and answer
-- a copy, as Roblox's array properties do: changing a table afterwards changes no filter (a script assigns it again, as
-- Roblox's engineer advises in the announcement's thread). The queries read what is stored through ParamsData
-- (world/query.rs), not through the proxy.
local ParamsData = setmetatable({}, { __mode = "k" })
local FilterLists = { FilterDescendantsInstances = true, ExcludeInstances = true, IncludeInstances = true }
local function ListCopy(Value)
	return if type(Value) == "table" then table.move(Value, 1, #Value, 1, table.create(#Value)) else Value
end
-- `Fields`: every member a script reads and writes besides AddToFilter, with the default a new one reads (FilterType's,
-- Exclude, is set at the first call, once the enums exist; the two filter lists of April 2026 are nil). Any other key raises
-- unquoted ("Nope is not a valid member of RaycastParams"), a key that is not a string too ("invalid argument #2 (string
-- expected, got number)"): measured in a 0.747 playing client and server (tools/parity/probes/proxy_values.luau).
local function FilterParamsType(TypeName, Fields, Meta)
	local Known = { FilterType = true, ExcludeInstances = true, IncludeInstances = true }
	for Key in Fields do
		Known[Key] = true
	end
	local Defaults
	return {
		new = function(Initial)
			if not Defaults then
				Defaults = table.clone(Fields)
				Defaults.FilterType = GetEnumItem("RaycastFilterType", "Exclude")
			end
			local Data = table.clone(Defaults)
			for Key, Value in Initial or {} do
				Data[Key] = if FilterLists[Key] then ListCopy(Value) else Value
			end
			local Object = Proxy(TypeName, function(_, Key)
				local Method = FilterMethods[Key]
				if Method then
					return function(_Self, ...)
						return Method(Data, ...)
					end
				elseif not Known[Key] then
					Members.ReadMiss(TypeName, Key, false)
				end
				local Value = Data[Key]
				return if FilterLists[Key] then ListCopy(Value) else Value
			end, function(_, Key, Value)
				if not Known[Key] then
					Members.WriteMiss(TypeName, Key, false)
				end
				Data[Key] = if FilterLists[Key] then ListCopy(Value) else Value
			end, Meta and Meta(Data))
			ParamsData[Object] = Data
			return Object
		end,
	}
end

-- coroutine.create / wrap make a thread, which Studio charges the script for as task.spawn does (cost.rs THREAD)
local Coroutine = table.clone(coroutine)
do
	local Charge, Thread = R.Charge, R.ThreadCost
	local Create, NativeWrap = coroutine.create, coroutine.wrap
	Coroutine.create = function(Function)
		Charge(Thread)
		return Create(Function)
	end
	Coroutine.wrap = function(Function)
		Charge(Thread)
		return NativeWrap(Function)
	end
end

-- tostring of the parameter objects as a playing client prints them (Studio 0.744): Roblox's fields in its order, an
-- instance list as {Name} for one instance and {N instances...} for more, an empty enum list as nothing, numbers with 9
-- significant digits. (A list of enum items with members is printed by name, comma-separated: not measured.)
local function ParamsText(TypeName, Fields)
	return function(Data)
		return {
			__tostring = function()
				local Parts = {}
				for _, Field in Fields do
					local Name, Value = Field[1], Data[Field[1]]
					if Value == nil then
						Value = Field[2]
					end
					local Text
					if type(Value) == "table" and Name == "FilterDescendantsInstances" then
						Text = if #Value == 0 then "{}" elseif #Value == 1 then `\{{Value[1]}\}` else `\{{#Value} instances...\}`
					elseif type(Value) == "table" then
						local Names = {}
						for _, Item in Value do
							table.insert(Names, EnumItemName(Item) or tostring(Item))
						end
						Text = table.concat(Names, ", ")
					elseif type(Value) == "number" then
						Text = string.format("%.9g", Value)
					else
						Text = EnumItemName(Value) or tostring(Value)
					end
					table.insert(Parts, `{Name}={Text}`)
				end
				return `{TypeName}\{{table.concat(Parts, ", ")}\}`
			end,
		}
	end
end
local RaycastParamsText = ParamsText("RaycastParams", { { "IgnoreWater", false }, { "Margin", 0 }, { "SizeGradient", 0 }, { "BruteForceAllSlow", false },
	{ "RespectCanCollide", false }, { "CollisionGroup", "Default" }, { "FilterDescendantsInstances", {} } })
local OverlapParamsText = ParamsText("OverlapParams", { { "MaxParts", 0 }, { "Tolerance", 0 }, { "BruteForceAllSlow", false }, { "RespectCanCollide", false },
	{ "CollisionGroup", "Default" }, { "FilterDescendantsInstances", {} } })
-- CatalogSearchParams.new(): Roblox's defaults as its tostring shows them (made at the first call, once the enums exist)
local CatalogSearchParamsType
local CatalogSearchParamsLibrary = {
	new = function(Initial)
		if not CatalogSearchParamsType then
			local Fields = { { "SearchKeyword", "" }, { "CategoryFilter", GetEnumItem("CatalogCategoryFilter", "None") }, { "SortType", GetEnumItem("CatalogSortType", "Relevance") },
				{ "SortAggregation", GetEnumItem("CatalogSortAggregation", "AllTime") }, { "SalesTypeFilter", GetEnumItem("SalesTypeFilter", "All") }, { "CreatorName", "" },
				{ "Limit", 30 }, { "CreatorType", GetEnumItem("CreatorTypeFilter", "All") }, { "BundleTypes", {} }, { "AssetTypes", {} }, { "IncludeOffSale", false } }
			-- (and three members its tostring leaves out at their defaults: a 0.747 client, tools/parity/probes/proxy_values.luau)
			local Defaults = { MinPrice = 0, MaxPrice = 2147483647, CreatorId = 0 }
			for _, Field in Fields do
				Defaults[Field[1]] = Field[2]
			end
			CatalogSearchParamsType = ParamsType("CatalogSearchParams", Defaults, false, ParamsText("CatalogSearchParams", Fields))
		end
		return CatalogSearchParamsType.new(Initial)
	end,
}
-- PathWaypoint.new(Position, Action, Label): tostring "{0.333333343, 2, 3}, Jump, label" (Studio 0.744); its three members
-- are read-only ("Label member of PathWaypoint is read-only and cannot be assigned to": a 0.747 client and server,
-- tools/parity/probes/proxy_values.luau). Made at the first call, once the enums exist.
local PathWaypointType
local PathWaypointLibrary = {
	new = function(Position, Action, Label)
		if not PathWaypointType then
			PathWaypointType = ParamsType("PathWaypoint", { Position = vector.zero, Action = GetEnumItem("PathWaypointAction", "Walk"), Label = "" }, true, function(Data)
				return {
					__tostring = function()
						local Position = Data.Position
						return `\{{string.format("%.9g", Position.X)}, {string.format("%.9g", Position.Y)}, {string.format("%.9g", Position.Z)}\}, {EnumItemName(Data.Action) or tostring(Data.Action)}, {Data.Label}`
					end,
				}
			end)
		end
		return PathWaypointType.new({ Position = Position or vector.zero, Action = Action or GetEnumItem("PathWaypointAction", "Walk"), Label = Label or "" })
	end,
}
-- PhysicalProperties.new(Enum.Material.X) as in Roblox (a playing client, Studio 0.744: Wood is 0.35, 0.48, 0.2, 1, 1,
-- 0.25): Lune's constructor takes only its own EnumItems, so the material goes by name
do
	local LunePhysical = Datatypes.PhysicalProperties
	local LuneMaterials = Roblox.Enum.Material
	ConstructorOverrides.PhysicalProperties = {
		new = function(...)
			local First = ...
			local Kind = if select("#", ...) == 1 then EnumItemType(First) else nil
			if Kind ~= nil then
				if Kind ~= "Material" then error("PhysicalProperties.new with 1 argument expects Enum.Material inputs", 0) end
				return PassResults("new", pcall(LunePhysical.new, LuneMaterials[EnumItemName(First)]))
			end
			return PassResults("new", pcall(LunePhysical.new, ...))
		end,
	}
end

-- Path2DControlPoint: three UDim2 fields a script reads and writes, compared by value. Its fields live in
-- Path2DPoints, where the engine reads them (path2d.rs). Measured in a playing client (Studio 0.744) and in Studio
-- 0.747 (tools/parity/probes/path2d.luau): new takes no argument, a Position, or all three; a field takes a UDim2 only.
local Path2DPoints = setmetatable({}, { __mode = "k" })
local Path2DPointMeta = {
	__tostring = function(Self)
		local Fields = Path2DPoints[Self]
		return `Path2DControlPoint \{ Position = {Fields.Position}, LeftTangent = {Fields.LeftTangent}, RightTangent = {Fields.RightTangent} \}`
	end,
	__eq = function(Left, Right)
		local A, B = Path2DPoints[Left], Path2DPoints[Right]
		return A ~= nil and B ~= nil and A.Position == B.Position and A.LeftTangent == B.LeftTangent and A.RightTangent == B.RightTangent
	end,
}
local function Path2DPointIndex(Self, Key)
	local Value = Path2DPoints[Self][Key]
	if Value == nil then
		Members.ReadMiss("Path2DControlPoint", Key, true)
	end
	return Value
end
local function Path2DPointNewIndex(Self, Key, Value)
	local Fields = Path2DPoints[Self]
	if Fields[Key] == nil then
		Members.WriteMiss("Path2DControlPoint", Key, false)
	elseif Typeof(Value) ~= "UDim2" then
		error(`invalid argument #3 (UDim2 expected, got {Typeof(Value)})`, 2)
	end
	Fields[Key] = Value
end
function Path2DControlPoint.new(...)
	local Count = select("#", ...)
	local Fields = { Position = UDim2.new(), LeftTangent = UDim2.new(), RightTangent = UDim2.new() }
	if Count > 0 then
		local Position, LeftTangent, RightTangent = ...
		if Typeof(Position) ~= "UDim2" then
			error(`invalid argument #1 to 'new' (UDim2 expected, got {Typeof(Position)})`, 2)
		elseif Count ~= 1 and Count ~= 3 then
			error("Trying to create a Path2DControlPoint with wrong number of arguments", 0)
		end
		Fields.Position = Position
		if Count == 3 then
			if Typeof(LeftTangent) ~= "UDim2" then
				error(`invalid argument #2 to 'new' (UDim2 expected, got {Typeof(LeftTangent)})`, 2)
			elseif Typeof(RightTangent) ~= "UDim2" then
				error(`invalid argument #3 to 'new' (UDim2 expected, got {Typeof(RightTangent)})`, 2)
			end
			Fields.LeftTangent, Fields.RightTangent = LeftTangent, RightTangent
		end
	end
	local Point = Proxy("Path2DControlPoint", Path2DPointIndex, Path2DPointNewIndex, Path2DPointMeta)
	Path2DPoints[Point] = Fields
	return Point
end

-- Keys a proxy type has no member for, as a client words them (a 0.747 playing client and server,
-- tools/parity/probes/proxy_values.luau). A read of a string names the key, quoted ("'Nope' is not a valid member of
-- RaycastResult") or, for RaycastParams, OverlapParams and SecurityCapabilities, unquoted; the quoting types read a number key
-- as its text ("'1.5' is not a valid member of Axes") where the others take only a string. A write names the key unquoted (a
-- number as its text), and a member a script may only read as "X member of Axes is read-only and cannot be assigned to". Any
-- other key raises "invalid argument #2 (string expected, got boolean)" at the script's line: each is called by the type's
-- __index or __newindex itself, so level 3 is the script's.
function Members.ReadMiss(TypeName, Key, Quoted)
	local Kind = type(Key)
	if Kind == "string" or (Quoted and Kind == "number") then
		error(if Quoted then `'{Key}' is not a valid member of {TypeName}` else `{Key} is not a valid member of {TypeName}`, 0)
	end
	error(`invalid argument #2 (string expected, got {Typeof(Key)})`, 3)
end
function Members.WriteMiss(TypeName, Key, ReadOnly)
	local Kind = type(Key)
	if Kind == "string" or Kind == "number" then
		error(if ReadOnly then `{Key} member of {TypeName} is read-only and cannot be assigned to` else `{Key} is not a valid member of {TypeName}`, 0)
	end
	error(`invalid argument #2 (string expected, got {Typeof(Key)})`, 3)
end

-- RaycastResult (world/query.rs builds one from a hit's fields): five read-only members, and tostring as a client prints it,
-- "RaycastResult{Part @ 0, 900.5, 0; normal = 0, 1, 0; material = Plastic}": the part as it prints then (its Name), each
-- vector's floats with 9 significant digits, the material's name. Two are equal when every member is (two casts that hit
-- alike). Measured in a 0.747 playing client and server (tools/parity/probes/proxy_values.luau).
local ResultMembers = { Instance = true, Position = true, Normal = true, Distance = true, Material = true }
local ResultData = setmetatable({}, { __mode = "k" })
local function Digits9(Vector)
	local X, Y, Z = VectorParts(Vector)
	return `{string.format("%.9g", X)}, {string.format("%.9g", Y)}, {string.format("%.9g", Z)}`
end
local ResultMeta = {
	__tostring = function(Self)
		local Data = ResultData[Self]
		return `RaycastResult\{{tostring(Data.Instance)} @ {Digits9(Data.Position)}; normal = {Digits9(Data.Normal)}; material = {EnumItemName(Data.Material) or tostring(Data.Material)}\}`
	end,
	__eq = function(Left, Right)
		local A, B = ResultData[Left], ResultData[Right]
		if A == nil or B == nil then
			return false
		end
		for Key in ResultMembers do
			if A[Key] ~= B[Key] then
				return false
			end
		end
		return true
	end,
}
local function RaycastResult(Data)
	local Object = Proxy("RaycastResult", function(_, Key)
		if not ResultMembers[Key] then
			Members.ReadMiss("RaycastResult", Key, true)
		end
		return Data[Key]
	end, function(_, Key)
		Members.WriteMiss("RaycastResult", Key, ResultMembers[Key] ~= nil)
	end, ResultMeta)
	ResultData[Object] = Data
	return Object
end

-- DockWidgetPluginGuiInfo.new(InitialDockState, InitialEnabled, InitialEnabledShouldOverrideRestore, FloatingXSize,
-- FloatingYSize, MinWidth, MinHeight): a value with no member a script can read or write. tostring prints the fields
-- ("InitialDockState:Right InitialEnabled:0 ... MinHeight:0", the flags as 1 or 0), and two with the same fields are equal.
-- Every argument given is checked, nil too, and those past the seventh are ignored: the state an InitialDockState item or its
-- value as a number (truncated; Right when left out), the flags booleans, the sizes numbers held as 32-bit integers (truncated;
-- -2147483648 out of range or NaN). Measured in a 0.747 playing client and server (tools/parity/probes/proxy_values.luau).
local DockStates = { "Top", "Bottom", "Left", "Right", "Float" } -- (by value + 1)
local DockErrors = {
	"DockWidgetPluginGuiInfo.new first argument expects InitialDockState input",
	"DockWidgetPluginGuiInfo.new second arg should be a boolean for InitialEnabled.",
	"DockWidgetPluginGuiInfo.new third arg should be a boolean for InitialEnabledShouldOverrideRestore.",
	"DockWidgetPluginGuiInfo.new fourth argument expects an int for FloatingXSize.",
	"DockWidgetPluginGuiInfo.new fifth argument expects an int for FloatingYSize.",
	"DockWidgetPluginGuiInfo.new sixth argument expects an int for MinWidth.",
	"DockWidgetPluginGuiInfo.new seventh argument expects an int for MinHeight.",
}
local DockInfo = setmetatable({}, { __mode = "k" })
local DockMeta = {
	__tostring = function(Self)
		local Fields = DockInfo[Self]
		return `InitialDockState:{Fields[1]} InitialEnabled:{if Fields[2] then 1 else 0} InitialEnabledShouldOverrideRestore:{if Fields[3] then 1 else 0} FloatingXSize:{Fields[4]} FloatingYSize:{Fields[5]} MinWidth:{Fields[6]} MinHeight:{Fields[7]}`
	end,
	__eq = function(Left, Right)
		local A, B = DockInfo[Left], DockInfo[Right]
		if A == nil or B == nil then
			return false
		end
		for Index = 1, 7 do
			if A[Index] ~= B[Index] then
				return false
			end
		end
		return true
	end,
}
local function DockIndex(_, Key)
	Members.ReadMiss("DockWidgetPluginGuiInfo", Key, true)
end
local function DockNewIndex(_, Key)
	Members.WriteMiss("DockWidgetPluginGuiInfo", Key, false)
end
local DockWidgetPluginGuiInfoLibrary = table.freeze({
	new = function(...)
		local Fields = { "Right", false, false, 0, 0, 0, 0 }
		for Index = 1, math.min(select("#", ...), 7) do
			local Value = select(Index, ...)
			local Kind = type(Value)
			if Index == 1 then
				local State
				if Kind == "number" then
					State = DockStates[(if Value >= 0 then math.floor(Value) else math.ceil(Value)) + 1]
				elseif EnumItemType(Value) == "InitialDockState" then
					State = EnumItemName(Value)
				end
				if State == nil then
					error(DockErrors[1], 0)
				end
				Fields[1] = State
			elseif Index <= 3 then
				if Kind ~= "boolean" then
					error(DockErrors[Index], 0)
				end
				Fields[Index] = Value
			else
				if Kind ~= "number" then
					error(DockErrors[Index], 0)
				end
				Fields[Index] = Int32(Value) + 0 -- (-0 as 0)
			end
		end
		local Object = Proxy("DockWidgetPluginGuiInfo", DockIndex, DockNewIndex, DockMeta)
		DockInfo[Object] = Fields
		return Object
	end,
})

-- SecurityCapabilities: a set of Enum.SecurityCapability items. tostring names them joined by " | " in the client's own
-- order (CapabilityOrder, not the items' values); AssetRequire is the set's LoadUnownedAsset and Avatar its AvatarAppearance
-- (each prints as, and is contained as, the other). new(...) makes one; fromCurrent() a game script's, the same on a client
-- and a server, whatever it is given; Contains(...) answers whether every item given is held (true for none); Add(...) and
-- Remove(...) answer a new set. Two are equal when they hold the same items. An argument that is no item raises "Invalid
-- value for enum 'SecurityCapability'", an item of another enum "Invalid type 'Material' for enum 'SecurityCapability'"; a
-- method called on another value "invalid argument #1 to 'Contains' (SecurityCapabilities expected, got nil)" at the script's
-- line; other members raise unquoted. Measured in a 0.747 playing client and server (tools/parity/probes/proxy_values.luau).
-- A plugin's set was not measured: in the edit run context fromCurrent reports itself and answers a game script's.
local CapabilityOrder = {
	"Plugin", "LocalUser", "WritePlayer", "RobloxScript", "RobloxEngine", "RunClientScript", "RunServerScript", "AccessOutsideWrite",
	"Unassigned", "LoadUnownedAsset", "LoadString", "ScriptGlobals", "CreateInstances", "Basic", "Audio", "DataStore", "Network",
	"Physics", "UI", "CSG", "Chat", "Animation", "AvatarAppearance", "Input", "Environment", "RemoteEvent", "LegacySound",
	"Players", "CapabilityControl", "AssetRead", "AssetManagement", "DynamicGeneration", "PlatformAvatarEditing",
	"AssetCreateUpdate", "Capture", "SensitiveInput", "Monetization", "LoadOwnedAsset", "Social", "ServerCommunication",
	"Logging", "PromptExternalPurchase", "Groups", "Teleport", "Consequences", "Material", "AvatarBehavior", "RemoteCommand",
	"InternalTest", "PluginOrOpenCloud", "Assistant",
}
local CapabilityAliases = { AssetRequire = "LoadUnownedAsset", Avatar = "AvatarAppearance" }
local CapabilityRank = {}
for Rank, Name in CapabilityOrder do
	CapabilityRank[Name] = Rank
end
-- (a game script's: RunClientScript to AvatarBehavior, every capability but the first five and the last four)
local ScriptCapabilities = {}
for Rank = CapabilityRank.RunClientScript, CapabilityRank.AvatarBehavior do
	ScriptCapabilities[Rank] = true
end
local CapabilitySets = setmetatable({}, { __mode = "k" })
local function CapabilityRankOf(Item)
	local TypeName = EnumItemType(Item)
	if TypeName == "SecurityCapability" then
		local Name = EnumItemName(Item)
		Name = CapabilityAliases[Name] or Name
		local Rank = CapabilityRank[Name]
		if Rank == nil then
			-- (an item newer than the measured order goes after the others)
			table.insert(CapabilityOrder, Name)
			Rank = #CapabilityOrder
			CapabilityRank[Name] = Rank
		end
		return Rank
	elseif TypeName then
		error(`Invalid type '{TypeName}' for enum 'SecurityCapability'`, 0)
	end
	error("Invalid value for enum 'SecurityCapability'", 0)
end
local function CapabilityRanks(...)
	local Count = select("#", ...)
	local Ranks = table.create(Count)
	for Index = 1, Count do
		Ranks[Index] = CapabilityRankOf((select(Index, ...)))
	end
	return Ranks
end
local CapabilityMethods = {}
local CapabilityMeta = {
	__tostring = function(Self)
		local Set = CapabilitySets[Self]
		local Names = {}
		for Rank, Name in CapabilityOrder do
			if Set[Rank] then
				table.insert(Names, Name)
			end
		end
		return table.concat(Names, " | ")
	end,
	__eq = function(Left, Right)
		local A, B = CapabilitySets[Left], CapabilitySets[Right]
		if A == nil or B == nil then
			return false
		end
		for Rank in A do
			if not B[Rank] then
				return false
			end
		end
		for Rank in B do
			if not A[Rank] then
				return false
			end
		end
		return true
	end,
}
local function CapabilityIndex(_, Key)
	local Method = CapabilityMethods[Key]
	if Method == nil then
		Members.ReadMiss("SecurityCapabilities", Key, false)
	end
	return Method
end
local function CapabilityNewIndex(_, Key)
	Members.WriteMiss("SecurityCapabilities", Key, false)
end
local function Capabilities(Set)
	local Object = Proxy("SecurityCapabilities", CapabilityIndex, CapabilityNewIndex, CapabilityMeta)
	CapabilitySets[Object] = Set
	return Object
end
-- (a method's set; called by the method itself, so level 3 is the script's line: a table's function, which the compiler
-- does not inline into the method as it would a local one)
function Members.CapabilitySet(Self, Method)
	local Set = if Self == nil then nil else CapabilitySets[Self]
	if Set == nil then
		error(`invalid argument #1 to '{Method}' (SecurityCapabilities expected, got {Typeof(Self)})`, 3)
	end
	return Set
end
function CapabilityMethods.Contains(Self, ...)
	local Set = Members.CapabilitySet(Self, "Contains")
	for _, Rank in CapabilityRanks(...) do
		if not Set[Rank] then
			return false
		end
	end
	return true
end
function CapabilityMethods.Add(Self, ...)
	local Set = table.clone(Members.CapabilitySet(Self, "Add"))
	for _, Rank in CapabilityRanks(...) do
		Set[Rank] = true
	end
	return Capabilities(Set)
end
function CapabilityMethods.Remove(Self, ...)
	local Set = table.clone(Members.CapabilitySet(Self, "Remove"))
	for _, Rank in CapabilityRanks(...) do
		Set[Rank] = nil
	end
	return Capabilities(Set)
end
local SecurityCapabilitiesLibrary = table.freeze({
	new = function(...)
		local Set = {}
		for _, Rank in CapabilityRanks(...) do
			Set[Rank] = true
		end
		return Capabilities(Set)
	end,
	fromCurrent = function()
		if R.EditMode then
			R.Missing("SecurityCapabilities", "fromCurrent")
		end
		return Capabilities(table.clone(ScriptCapabilities))
	end,
})

-- ValueCurveKey.new(Time, Value, Interpolation?): two or three arguments ("ValueCurveKey.new expects 2 or 3 input arguments").
-- Time is what reads as a number (a number string too; anything else 0), kept as a float; Value anything but nil ("Failed to
-- get value for second argument in ValueCurveKey"), a table or buffer copied as an event's argument is (lua/bindable.rs) and
-- again at every read; Interpolation a KeyInterpolationMode item ("Invalid value for enum 'KeyInterpolationMode'" for nil,
-- a name or a number, "Invalid type 'Material' for enum ..." for another enum's), Constant when left out. tostring is empty.
-- Every member is written: Time as the constructor reads it, nil after a value that does not read as a number; Value anything
-- but nil ("Failed to get input value for setValue of ValueCurveKey"); Interpolation an item, a mode other than Cubic dropping
-- the right tangent; the tangents (nil until set) ignore nil and keep a float, NaN for what does not read as a number, the
-- right one on a Cubic key only ("Attempting to set a right tangent value on a curve key for which the interpolation mode is
-- not Cubic"). Two are equal when every member is (a key is itself; never with a nil Time, or a table or function Value). Other keys raise
-- quoted. Measured in a 0.747 playing client and server (tools/parity/probes/proxy_values.luau).
local KeyInfo = setmetatable({}, { __mode = "k" })
local KeyMembers = { Time = true, Value = true, Interpolation = true, LeftTangent = true, RightTangent = true }
local KeyModes -- (Constant and Cubic, at the first call, once the enums exist)
local function KeyFloat(Value)
	local Number = tonumber(Value)
	return if Number then vector.create(Number, 0, 0).x else nil
end
-- (Height: the frame the copy starts above, as Roblox's: new's arguments, a write's object, key and value, a read's result)
local function KeyCopy(Value, Height)
	local Kind = type(Value)
	if Kind ~= "table" and Kind ~= "buffer" then
		return Value
	end
	local Copy, _, Message = R.Copied(Height, Value)
	if Copy == SIGNAL then
		error(Message, 0)
	end
	return Copy
end
local function KeyMode(Value)
	local TypeName = EnumItemType(Value)
	if TypeName == "KeyInterpolationMode" then
		return Value
	elseif TypeName then
		error(`Invalid type '{TypeName}' for enum 'KeyInterpolationMode'`, 0)
	end
	error("Invalid value for enum 'KeyInterpolationMode'", 0)
end
local KeyMeta = {
	__tostring = function()
		return ""
	end,
	__eq = function(Left, Right)
		if rawequal(Left, Right) then
			return true
		end
		local A, B = KeyInfo[Left], KeyInfo[Right]
		if A == nil or B == nil or A.Time == nil or B.Time == nil then
			return false
		end
		local Kind = type(A.Value)
		if Kind == "table" or Kind == "function" or Kind == "buffer" then
			return false
		end
		return A.Time == B.Time and A.Value == B.Value and A.Interpolation == B.Interpolation and A.LeftTangent == B.LeftTangent and A.RightTangent == B.RightTangent
	end,
}
local function KeyIndex(Self, Key)
	if not KeyMembers[Key] then
		Members.ReadMiss("ValueCurveKey", Key, true)
	end
	local Info = KeyInfo[Self]
	return if Key == "Value" then KeyCopy(Info.Value, 1) else Info[Key]
end
local function KeyNewIndex(Self, Key, Value)
	local Info = KeyInfo[Self]
	if Key == "Time" then
		Info.Time = KeyFloat(Value)
	elseif Key == "Value" then
		if Value == nil then
			error("Failed to get input value for setValue of ValueCurveKey", 0)
		end
		Info.Value = KeyCopy(Value, 3)
	elseif Key == "Interpolation" then
		Info.Interpolation = KeyMode(Value)
		if Info.Interpolation ~= KeyModes.Cubic then
			Info.RightTangent = nil
		end
	elseif Key == "LeftTangent" or Key == "RightTangent" then
		if Value == nil then
			return
		elseif Key == "RightTangent" and Info.Interpolation ~= KeyModes.Cubic then
			error("Attempting to set a right tangent value on a curve key for which the interpolation mode is not Cubic", 0)
		end
		Info[Key] = KeyFloat(Value) or 0 / 0
	else
		Members.WriteMiss("ValueCurveKey", Key, false)
	end
end
local ValueCurveKeyLibrary = table.freeze({
	new = function(...)
		local Count = select("#", ...)
		if Count ~= 2 and Count ~= 3 then
			error("ValueCurveKey.new expects 2 or 3 input arguments", 0)
		end
		if not KeyModes then
			KeyModes = { Constant = GetEnumItem("KeyInterpolationMode", "Constant"), Cubic = GetEnumItem("KeyInterpolationMode", "Cubic") }
		end
		local Time, Value, Mode = ...
		if Value == nil then
			error("Failed to get value for second argument in ValueCurveKey", 0)
		end
		local Info = { Time = KeyFloat(Time) or 0, Value = KeyCopy(Value, Count), Interpolation = if Count == 3 then KeyMode(Mode) else KeyModes.Constant }
		local Object = Proxy("ValueCurveKey", KeyIndex, KeyNewIndex, KeyMeta)
		KeyInfo[Object] = Info
		return Object
	end,
})

-- require of a module that must load, from a thread that cannot yield (modules.rs require_begin): the module loads on a
-- thread of its own, as in a client. RequireBegin answers with its outcome (0, value or 1, message, level) when it loads
-- without yielding; when it yields, with `thread is not yieldable` while it goes on loading, and a later require gets its
-- result (measured in a 0.747 playing client and server: tools/parity/probes/yield_refused_require.luau). (Below the
-- traced functions: rur's golden traces record prelude line numbers.)
local function RequireApart(Target, Caller, Vm)
	local Outcome
	local Loader = coroutine.create(function()
		while true do
			local Status, A, B = RequireBegin(Target, Caller, Vm)
			if Status == 2 then
				yield()
			elseif Status == 3 then
				local Finished, Message, Level = RequireFinish(B, table.pack(pcall(A)), Vm)
				Outcome = if Finished then { 0, Message } else { 1, Message, Level }
				return
			else
				Outcome = { Status, A, B }
				return
			end
		end
	end)
	local Resumed, Problem = coroutine.resume(Loader)
	if not Resumed then
		error(Problem, 0)
	end
	if Outcome then
		return Outcome[1], Outcome[2], Outcome[3]
	end
	return 1, "thread is not yieldable", 0
end

return {
	Wrap = Wrap,
	WrapIndex = WrapIndex,
	WrapNewIndex = WrapNewIndex,
	Pass = Pass,
	Typeof = Typeof,
	Register = Register,
	TypeRegistry = Registry,
	Proxy = Proxy,
	ScriptLibrary = ScriptLibrary,
	Datatypes = Datatypes,
	Faces = Faces,
	Axes = Axes,
	Path2DControlPoint = Path2DControlPoint,
	Path2DPoints = Path2DPoints,
	RaycastParams = FilterParamsType("RaycastParams", { FilterDescendantsInstances = {}, IgnoreWater = false, RespectCanCollide = false, CollisionGroup = "Default",
		BruteForceAllSlow = false, Margin = 0, SizeGradient = 0 }, RaycastParamsText),
	OverlapParams = FilterParamsType("OverlapParams", { FilterDescendantsInstances = {}, MaxParts = 0, RespectCanCollide = false, CollisionGroup = "Default",
		BruteForceAllSlow = false, Tolerance = 0 }, OverlapParamsText),
	ParamsData = ParamsData,
	RaycastResult = RaycastResult,
	CatalogSearchParams = CatalogSearchParamsLibrary,
	PathWaypoint = PathWaypointLibrary,
	DockWidgetPluginGuiInfo = DockWidgetPluginGuiInfoLibrary,
	SecurityCapabilities = SecurityCapabilitiesLibrary,
	ValueCurveKey = ValueCurveKeyLibrary,
	InertLibrary = InertLibrary,
	MakeStubTree = MakeStubTree,
	Utf8 = Utf8Table,
	Debug = DebugTable,
	Coroutine = Coroutine,
	Os = table.clone(os),
	Task = Task,
	LegacyWait = LegacyWait,
	Traceback = Traceback,
	VectorParts = VectorParts,
	GetEnumItem = GetEnumItem,
	Print = Print,
	Warn = Warn,
	Stringify = Stringify,
	Require = Require,
	RequireApart = RequireApart,
	Environment = Environment,
	CallBody = CallBody,
}
