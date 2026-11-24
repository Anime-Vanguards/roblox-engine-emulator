-- InstanceHandle: the library global (InstanceHandle.new), what a handle is and does (Get, members, writes, operators,
-- equality, table keys), what it holds once its instance is renamed, unparented or destroyed, and how attributes,
-- Bindables, JSON, properties and table copies take one.
local function Show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return P.num(Value)
	elseif Kind == "string" then
		return '"' .. P.text(Value) .. '"'
	elseif Kind == "nil" or Kind == "boolean" then
		return tostring(Value)
	elseif Kind == "table" then
		return "table"
	elseif Kind == "Instance" then
		return "Instance(" .. Value.Name .. ")"
	end
	return Kind .. "(" .. (string.gsub(tostring(Value), "0x%x+", "ADDR")) .. ")"
end
local function Try(Function, ...)
	local Results = table.pack(pcall(Function, ...))
	if Results[1] then
		local Parts = {}
		for Index = 2, Results.n do
			table.insert(Parts, Show(Results[Index]))
		end
		return "ok " .. table.concat(Parts, ", ")
	end
	local Message = Results[2]
	if type(Message) ~= "string" then
		return "error (" .. typeof(Message) .. ") " .. (string.gsub(string.split(tostring(Message), "\n")[1], "0x%x+", "ADDR"))
	end
	return "error " .. P.err(Message)
end
P.sync()

-- the global
print("global " .. typeof(InstanceHandle) .. " " .. type(InstanceHandle))
print("global-tostring " .. Try(function()
	return (string.gsub(tostring(InstanceHandle), "0x%x+", "ADDR"))
end))
print("global-frozen " .. Try(function()
	return table.isfrozen(InstanceHandle)
end))
print("global-metatable " .. Try(function()
	return getmetatable(InstanceHandle)
end))
local Keys = {}
pcall(function()
	for Key, Value in InstanceHandle do
		table.insert(Keys, tostring(Key) .. "=" .. typeof(Value))
	end
end)
table.sort(Keys)
print("global-keys " .. table.concat(Keys, ","))
for _, Key in { "new", "New", "none", "fromInstance", "Get", "Foo" } do
	print("global-member " .. Key .. " " .. Try(function()
		return InstanceHandle[Key]
	end))
end
print("global-write " .. Try(function()
	InstanceHandle.Foo = 1
end))
print("global-write-new " .. Try(function()
	InstanceHandle.new = 1
end))

-- the constructor
local Target = Instance.new("Part")
Target.Name = "ParityTarget"
Target.Parent = workspace
local Loose = Instance.new("Folder")
Loose.Name = "Loose"
local Gone = Instance.new("Folder")
Gone.Name = "Gone"
Gone:Destroy()
local Signal = Target.Changed
local Arguments = {
	{ "none", function() return InstanceHandle.new() end },
	{ "nil", function() return InstanceHandle.new(nil) end },
	{ "part", function() return InstanceHandle.new(Target) end },
	{ "loose", function() return InstanceHandle.new(Loose) end },
	{ "destroyed", function() return InstanceHandle.new(Gone) end },
	{ "game", function() return tostring(InstanceHandle.new(game)) == game.Name end },
	{ "service", function() return InstanceHandle.new(game:GetService("Lighting")) end },
	{ "number", function() return InstanceHandle.new(1) end },
	{ "numstr", function() return InstanceHandle.new("2") end },
	{ "string", function() return InstanceHandle.new("x") end },
	{ "bool", function() return InstanceHandle.new(true) end },
	{ "false", function() return InstanceHandle.new(false) end },
	{ "table", function() return InstanceHandle.new({}) end },
	{ "Vector3", function() return InstanceHandle.new(Vector3.one) end },
	{ "CFrame", function() return InstanceHandle.new(CFrame.identity) end },
	{ "EnumItem", function() return InstanceHandle.new(Enum.Material.Wood) end },
	{ "Enum", function() return InstanceHandle.new(Enum.Material) end },
	{ "function", function() return InstanceHandle.new(print) end },
	{ "thread", function() return InstanceHandle.new(coroutine.create(print)) end },
	{ "newproxy", function() return InstanceHandle.new(newproxy()) end },
	{ "buffer", function() return InstanceHandle.new(buffer.create(1)) end },
	{ "signal", function() return InstanceHandle.new(Signal) end },
	{ "handle", function() return InstanceHandle.new(InstanceHandle.new(Target)) end },
	{ "nil-handle", function() return InstanceHandle.new(InstanceHandle.new(nil)) end },
	{ "extra", function() return InstanceHandle.new(Target, 5) end },
	{ "nil-extra", function() return InstanceHandle.new(nil, Target) end },
	{ "colon", function() return InstanceHandle:new(Target) end },
	{ "colon-none", function() return InstanceHandle:new() end },
}
for _, Case in Arguments do
	print("new " .. Case[1] .. " " .. Try(Case[2]))
end
-- the positions of a refused argument: called directly by pcall, and from a function's line
print("new-direct " .. Try(InstanceHandle.new, 1))
print("new-direct-none " .. Try(InstanceHandle.new))
print("new-line " .. Try(function()
	local Made = InstanceHandle.new("x")
	return Made
end))

-- a handle
local Handle = InstanceHandle.new(Target)
local Empty = InstanceHandle.new(nil)
print("handle typeof=" .. typeof(Handle) .. " type=" .. type(Handle) .. " tostring=" .. tostring(Handle))
print("empty typeof=" .. typeof(Empty) .. " type=" .. type(Empty) .. " tostring=" .. Try(tostring, Empty))
print("get " .. Try(function()
	local Got = Handle:Get()
	return Got, rawequal(Got, Target)
end))
print("get-empty " .. Try(function()
	return Empty:Get()
end))
print("get-count " .. Try(function()
	return select("#", Handle:Get()), select("#", Empty:Get())
end))
print("metatable " .. Try(getmetatable, Handle))
for _, Key in { "Get", "Set", "Value", "Instance", "Name", "Parent", "ClassName", "Target", "IsValid", "Clear", "Destroy", "Clone", "GetInstance", "IsA", "get", "Changed", "Object", "Ref" } do
	print("member " .. Key .. " " .. Try(function()
		return Handle[Key]
	end))
end
print("member-number " .. Try(function()
	return Handle[1]
end))
print("member-table " .. Try(function()
	return Handle[{}]
end))
print("member-empty " .. Try(function()
	return Empty.Name
end))
print("get-same " .. Try(function()
	return Handle.Get == Handle.Get, Handle.Get == Empty.Get, typeof(Handle.Get)
end))
-- Get's self and arguments
print("get-dot-none " .. Try(function()
	return Handle.Get()
end))
print("get-dot-self " .. Try(function()
	return Handle.Get(Handle)
end))
print("get-dot-other " .. Try(function()
	return Handle.Get(Empty)
end))
print("get-dot-number " .. Try(function()
	return Handle.Get(5)
end))
print("get-dot-instance " .. Try(function()
	return Handle.Get(Target)
end))
print("get-dot-vector " .. Try(function()
	return Handle.Get(Vector3.one)
end))
print("get-direct " .. Try(Handle.Get))
print("get-direct-number " .. Try(Handle.Get, 5))
print("get-extra " .. Try(function()
	return Handle:Get(1, 2)
end))
print("get-unknown-method " .. Try(function()
	return Handle:Set(Loose)
end))
print("get-unknown-method-2 " .. Try(function()
	return Handle:Foo()
end))
print("get-lower " .. Try(function()
	return Handle:get()
end))
-- writes
for _, Key in { "Get", "Name", "Value", "Foo" } do
	print("write " .. Key .. " " .. Try(function()
		Handle[Key] = 1
	end))
end
print("write-number " .. Try(function()
	Handle[1] = 1
end))
-- operators
print("op-add " .. Try(function()
	return Handle + 1
end))
print("op-concat " .. Try(function()
	return "x" .. Handle
end))
print("op-len " .. Try(function()
	return #Handle
end))
print("op-lt " .. Try(function()
	return Handle < Handle
end))
print("op-le " .. Try(function()
	return Handle <= Empty
end))
print("op-call " .. Try(function()
	return Handle()
end))
print("op-unm " .. Try(function()
	return -Handle
end))
print("op-iter " .. Try(function()
	for _ in Handle do
	end
end))
-- equality and identity
local Again = InstanceHandle.new(Target)
print("eq same=" .. tostring(Handle == Handle) .. " again=" .. tostring(Handle == Again) .. " rawequal=" .. tostring(rawequal(Handle, Again)) .. " instance=" .. tostring(Handle == Target) .. " empty=" .. tostring(Empty == InstanceHandle.new(nil)) .. " empty-rawequal=" .. tostring(rawequal(Empty, InstanceHandle.new())) .. " other=" .. tostring(Handle == InstanceHandle.new(Loose)) .. " vs-empty=" .. tostring(Handle == Empty) .. " ne=" .. tostring(Handle ~= Again))
print("eq number=" .. tostring(Handle == 1) .. " nil=" .. tostring(Handle == nil) .. " newproxy=" .. tostring(Handle == newproxy()))
local Map = { [Handle] = "first" }
print("key same=" .. tostring(Map[Handle]) .. " again=" .. tostring(Map[Again]) .. " instance=" .. tostring(Map[Target]))
local Set = {}
Set[InstanceHandle.new(Target)] = 1
Set[InstanceHandle.new(Target)] = 2
local Count = 0
for _ in Set do
	Count += 1
end
print("key count=" .. Count)
-- copies
local Copy = table.clone({ Handle, Key = Handle })
print("clone " .. tostring(rawequal(Copy[1], Handle)) .. " " .. tostring(rawequal(Copy.Key, Handle)))
print("frozen-copy " .. Try(function()
	local Frozen = table.freeze({ Handle })
	return rawequal(Frozen[1], Handle)
end))

-- the instance's fate
local Fate = Instance.new("Part")
Fate.Name = "Fate"
Fate.Parent = workspace
local FateHandle = InstanceHandle.new(Fate)
Fate.Name = "Renamed"
print("after-rename tostring=" .. tostring(FateHandle) .. " get=" .. Show(FateHandle:Get()))
Fate.Parent = nil
print("after-unparent tostring=" .. tostring(FateHandle) .. " get=" .. Show(FateHandle:Get()) .. " parent=" .. tostring(FateHandle:Get() and FateHandle:Get().Parent))
Fate:Destroy()
task.wait()
print("after-destroy tostring=" .. Try(tostring, FateHandle) .. " get=" .. Try(function()
	local Got = FateHandle:Get()
	return Got, rawequal(Got, Fate), Got and Got.Parent
end))
print("after-destroy-eq " .. tostring(FateHandle == InstanceHandle.new(Fate)))
local GoneHandle = InstanceHandle.new(Gone)
print("destroyed-made tostring=" .. Try(tostring, GoneHandle) .. " get=" .. Try(function()
	return GoneHandle:Get()
end))
local LooseHandle = InstanceHandle.new(Loose)
print("loose tostring=" .. tostring(LooseHandle) .. " get=" .. Show(LooseHandle:Get()))
local GameHandle = InstanceHandle.new(game)
print("game tostring=" .. Try(function()
	return tostring(GameHandle) == game.Name
end) .. " get=" .. Try(function()
	return GameHandle:Get() == game
end))

-- attributes
local Holder = Instance.new("Folder")
Holder.Parent = workspace
local Changes = {}
Holder.AttributeChanged:Connect(function(Name)
	table.insert(Changes, Name)
end)
print("attr-set-instance " .. Try(Holder.SetAttribute, Holder, "Ref", Target))
local Read = Holder:GetAttribute("Ref")
print("attr-read typeof=" .. typeof(Read) .. " tostring=" .. tostring(Read) .. " get=" .. Try(function()
	return rawequal(Read:Get(), Target)
end) .. " eq-new=" .. tostring(Read == Handle) .. " rawequal-next=" .. tostring(rawequal(Read, Holder:GetAttribute("Ref"))))
print("attr-set-handle " .. Try(Holder.SetAttribute, Holder, "FromHandle", Handle) .. " " .. Try(function()
	local Back = Holder:GetAttribute("FromHandle")
	return typeof(Back), rawequal(Back, Handle), Back == Handle, rawequal(Back:Get(), Target)
end))
print("attr-set-empty " .. Try(Holder.SetAttribute, Holder, "FromEmpty", Empty) .. " " .. Try(function()
	local Back = Holder:GetAttribute("FromEmpty")
	return typeof(Back), Back ~= nil and Back:Get()
end))
print("attr-set-empty-listed " .. Try(function()
	local Listed = Holder:GetAttributes()
	local Names = {}
	for Name, Value in Listed do
		table.insert(Names, Name .. ":" .. typeof(Value))
	end
	table.sort(Names)
	return table.concat(Names, ",")
end))
print("attr-set-destroyed-handle " .. Try(Holder.SetAttribute, Holder, "FromGone", GoneHandle) .. " " .. Try(function()
	local Back = Holder:GetAttribute("FromGone")
	return typeof(Back), Back ~= nil and Back:Get()
end))
task.wait()
table.sort(Changes)
print("attr-changed " .. table.concat(Changes, ","))
table.clear(Changes)
pcall(Holder.SetAttribute, Holder, "FromHandle", Again)
pcall(Holder.SetAttribute, Holder, "Ref", Handle)
task.wait()
print("attr-same-again " .. table.concat(Changes, ","))
print("attr-empty-again " .. Try(Holder.SetAttribute, Holder, "FromEmpty", InstanceHandle.new()))
print("attr-ref-to-empty " .. Try(Holder.SetAttribute, Holder, "Ref", Empty) .. " " .. Try(function()
	return Holder:GetAttribute("Ref"):Get()
end))
print("attr-empty-to-instance " .. Try(Holder.SetAttribute, Holder, "FromEmpty", Target) .. " " .. Try(function()
	return Holder:GetAttribute("FromEmpty"):Get()
end))
task.wait()
table.sort(Changes)
print("attr-changed-2 " .. table.concat(Changes, ","))
table.clear(Changes)
print("attr-clone " .. Try(function()
	local Copy = Holder:Clone()
	local Ref = Copy:GetAttribute("Ref")
	return typeof(Ref), Ref:Get(), rawequal(Copy:GetAttribute("FromHandle"):Get(), Target)
end))
print("attr-instance-attr-of-handle " .. Try(function()
	return Holder:GetAttribute("Ref"):Get():GetAttribute("Nothing")
end))

-- Bindables
local Bindable = Instance.new("BindableEvent")
local Arrived = {}
Bindable.Event:Connect(function(...)
	Arrived = table.pack(...)
end)
local function Fired(...)
	Arrived = { n = -1 }
	local Ok, Problem = pcall(Bindable.Fire, Bindable, ...)
	task.wait()
	if not Ok then
		return "error " .. P.err(Problem)
	end
	local Parts = { "n=" .. tostring(Arrived.n) }
	for Index = 1, math.max(Arrived.n, 0) do
		local Value = Arrived[Index]
		if typeof(Value) == "InstanceHandle" then
			table.insert(Parts, "InstanceHandle rawequal=" .. tostring(rawequal(Value, Handle)) .. " eq=" .. tostring(Value == Handle) .. " get=" .. Show(Value:Get()))
		elseif typeof(Value) == "table" then
			local Inner = Value[1]
			local Kinds = {}
			for Key, Item in Value do
				table.insert(Kinds, typeof(Key) .. (if type(Key) == "string" then '"' .. P.text(Key) .. '"' else "") .. ":" .. typeof(Item))
			end
			table.sort(Kinds)
			table.insert(Parts, "table " .. typeof(Inner) .. (if typeof(Inner) == "InstanceHandle" then " rawequal=" .. tostring(rawequal(Inner, Handle)) .. " get=" .. Show(Inner:Get()) else "") .. " entries=" .. table.concat(Kinds, ","))
		else
			table.insert(Parts, Show(Value))
		end
	end
	return table.concat(Parts, " | ")
end
print("bindable-fire " .. Fired(Handle))
print("bindable-fire-empty " .. Fired(Empty))
print("bindable-fire-table " .. Fired({ Handle, Key = Handle }))
print("bindable-fire-key " .. Fired({ [Handle] = 1 }))
print("bindable-fire-empty-key " .. Fired({ [Empty] = 1 }))
print("bindable-fire-destroyed-key " .. Fired({ [GoneHandle] = 1 }))
local Function = Instance.new("BindableFunction")
Function.OnInvoke = function(...)
	return ...
end
print("bindable-invoke " .. Try(function()
	local Back = Function:Invoke(Handle)
	return typeof(Back), rawequal(Back, Handle), Back == Handle, Back and Back:Get()
end))
print("bindable-invoke-table " .. Try(function()
	local Back = Function:Invoke({ Handle })
	return typeof(Back[1]), rawequal(Back[1], Handle)
end))

-- SharedTable, and the debug names
print("sharedtable " .. Try(function()
	local Shared = SharedTable.new({ Handle })
	return typeof(Shared[1])
end))
print("sharedtable-same " .. Try(function()
	local Shared = SharedTable.new({ Handle, Empty })
	return rawequal(Shared[1], Handle), Shared[1] == Handle, rawequal(Shared[1], Shared[1]), rawequal(Shared[1]:Get(), Target), Shared[2]:Get()
end))
print("sharedtable-clone " .. Try(function()
	local Shared = SharedTable.clone(SharedTable.new({ Handle }), true)
	return rawequal(Shared[1], Handle), Shared[1] == Handle
end))
print("sharedtable-set " .. Try(function()
	local Shared = SharedTable.new()
	Shared.Key = Handle
	return typeof(Shared.Key)
end))
print("debug-new " .. Try(function()
	return debug.info(InstanceHandle.new, "s"), debug.info(InstanceHandle.new, "n"), debug.info(InstanceHandle.new, "a")
end))
print("debug-get " .. Try(function()
	return debug.info(Handle.Get, "s"), debug.info(Handle.Get, "n"), debug.info(Handle.Get, "a")
end))
print("new-count " .. select("#", InstanceHandle.new(Target)) .. " " .. select("#", InstanceHandle.new()))

-- JSON and properties
local Http = game:GetService("HttpService")
print("json " .. Try(Http.JSONEncode, Http, { Handle }))
print("json-key " .. Try(Http.JSONEncode, Http, { Key = Handle }))
print("json-handle-key " .. Try(Http.JSONEncode, Http, { [Handle] = 1 }))
print("json-empty-key " .. Try(Http.JSONEncode, Http, { [Empty] = 1 }))
print("json-alone " .. Try(Http.JSONEncode, Http, Handle))
local Value = Instance.new("ObjectValue")
print("objectvalue " .. Try(function()
	Value.Value = Handle
	return Value.Value
end))
print("parent " .. Try(function()
	local Part = Instance.new("Part")
	Part.Parent = Handle
	return Part.Parent
end))
print("property-string " .. Try(function()
	local Part = Instance.new("Part")
	Part.Name = Handle
	return Part.Name
end))
print("tostring-concat " .. Try(function()
	return "[" .. tostring(Handle) .. "]"
end))
print("string-format " .. Try(function()
	return string.format("%s", Handle)
end))
Holder:Destroy()
Target:Destroy()
