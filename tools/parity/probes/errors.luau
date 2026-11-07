-- Error messages (the VM's and the engine's), error levels, tracebacks, and what scripts see of them.
local function Catch(Function, ...)
	local Ok, Message = pcall(Function, ...)
	if Ok then
		return "no error"
	end
	if type(Message) ~= "string" then
		return "(" .. typeof(Message) .. ") " .. tostring(if type(Message) == "table" then Message.Code else Message)
	end
	return P.err(Message)
end
local Part = Instance.new("Part")
local Cases = {
	{ "error-string", function() error("plain") end },
	{ "error-level0", function() error("plain", 0) end },
	{ "error-level2", function() local function Inner() error("up", 2) end Inner() end },
	{ "error-table", function() error({ Code = 7 }) end },
	{ "error-number", function() error(42) end },
	{ "error-nil", function() error(nil) end },
	{ "error-noarg", function() error() end },
	{ "error-instance", function() error(Part) end },
	{ "call-nil", function() local Missing = nil Missing() end },
	{ "call-field", function() local T = {} T.Missing() end },
	{ "call-method", function() local T = {} T:Missing() end },
	{ "call-number", function() local N = 5 N() end },
	{ "index-nil", function() local Missing = nil return Missing.Field end },
	{ "index-nil-local", function() local Thing return Thing.Field end },
	{ "index-number", function() local N = 5 return N.Field end },
	{ "index-global", function() return UndefinedGlobal.Field end },
	{ "newindex-nil", function() local Missing = nil Missing.Field = 1 end },
	{ "arith-nil", function() local Missing = nil return Missing + 1 end },
	{ "arith-table", function() return {} + 1 end },
	{ "arith-string", function() return "abc" * 2 end },
	{ "arith-bool", function() return true - 1 end },
	{ "concat-nil", function() local Missing = nil return "a" .. Missing end },
	{ "concat-table", function() return "a" .. {} end },
	{ "compare-mixed", function() return 1 < "2" end },
	{ "compare-tables", function() return {} < {} end },
	{ "compare-nil", function() return nil < 1 end },
	{ "len-number", function() return #5 end },
	{ "len-nil", function() local Missing = nil return #Missing end },
	{ "unm-table", function() return -{} end },
	{ "for-step0", function() for _ = 1, 2, 0 do end end },
	{ "for-string", function() for _ = "a", 2 do end end },
	{ "iterate-number", function() for _ in 5 do end end },
	{ "iterate-nil", function() for _ in nil do end end },
	{ "stack-overflow", function() local function F() return 1 + F() end return F() end },
	{ "assert-false", function() assert(false) end },
	{ "assert-message", function() assert(nil, "custom message") end },
	{ "assert-table", function() assert(false, { Code = 1 }) end },
	{ "tostring-none", function() return tostring() end },
	{ "setmetatable-bad", function() setmetatable(1, {}) end },
	{ "select-negative", function() return select(-5, 1) end },
	{ "select-zero", function() return select(0, 1) end },
	{ "string-method-nil", function() return string.upper(nil) end },
	{ "math-floor-string", function() return math.floor("x") end },
	{ "rawset-nil-key", function() rawset({}, nil, 1) end },
	{ "table-nan-key", function() local T = {} T[0 / 0] = 1 end },
	{ "table-nil-key", function() local T = {} T[nil] = 1 end },
	{ "ipairs-nil", function() for _ in ipairs(nil) do end end },
	{ "pairs-nil", function() for _ in pairs(nil) do end end },
	{ "next-bad-key", function() return next({}, "nope") end },
	{ "coroutine-resume-dead", function() local C = coroutine.create(function() end) coroutine.resume(C) local Ok, Message = coroutine.resume(C) error(Message, 0) end },
	{ "wrap-dead", function() local W = coroutine.wrap(function() end) W() W() end },
	{ "wrap-error", function() local W = coroutine.wrap(function() error("inner") end) W() end },
	{ "string-rep-too-large", function() return string.rep("xy", 2 ^ 30) end },
	-- engine errors
	{ "invalid-member", function() return Part.NotAMember end },
	{ "invalid-member-write", function() Part.NotAMember = 1 end },
	{ "wrong-type-write", function() Part.Size = 5 end },
	{ "wrong-type-write-string", function() Part.Anchored = "yes" end },
	{ "readonly-write", function() Part.ClassName = "X" end },
	{ "readonly-write-abs", function() local Frame = Instance.new("Frame") Frame.AbsoluteSize = Vector2.zero end },
	{ "enum-wrong", function() Part.Material = "NotAMaterial" end },
	{ "enum-wrong-type", function() Part.Material = Enum.SortOrder.Name end },
	{ "enum-number-wrong", function() Part.Material = 999999 end },
	{ "method-on-wrong-class", function() return Part:GetPlayers() end },
	{ "method-with-dot", function() return Part.FindFirstChild("x") end },
	{ "method-bad-self", function() return Part.FindFirstChild(5, "x") end },
	{ "method-missing-arg", function() return Part:FindFirstChild() end },
	{ "method-wrong-arg", function() return Part:IsA(5) end },
	{ "instance-new-bad", function() return Instance.new("NotAClass") end },
	{ "instance-new-service", function() return Instance.new("Workspace") end },
	{ "instance-new-abstract", function() return Instance.new("BasePart") end },
	{ "instance-new-number", function() return Instance.new(5) end },
	{ "getservice-bad", function() return game:GetService("NotAService") end },
	{ "vector3-bad-arg", function() return Vector3.new("a") end },
	{ "vector3-arith-bad", function() return Vector3.new() + 1 end },
	{ "vector3-index", function() return Vector3.new().W end },
	{ "vector3-newindex", function() local V = Vector3.new() V.X = 1 end },
	{ "cframe-bad-arg", function() return CFrame.new("a") end },
	{ "cframe-mul-bad", function() return CFrame.new() * 5 end },
	{ "udim2-bad", function() return UDim2.new("a") end },
	{ "color3-index", function() return Color3.new().A end },
	{ "enum-index", function() return Enum.Material.NotThere end },
	{ "enums-index", function() return Enum.NotAnEnum end },
	{ "enumitem-newindex", function() Enum.Material.Plastic.Value = 1 end },
	{ "clone-locked-parent", function() local P2 = Instance.new("Part") P2:Destroy() P2.Parent = workspace end },
	{ "tween-bad-property", function() return game:GetService("TweenService"):Create(Part, TweenInfo.new(1), { NotAProperty = 1 }) end },
	{ "tween-bad-type", function() return game:GetService("TweenService"):Create(Part, TweenInfo.new(1), { Size = 5 }) end },
	{ "tween-untweenable", function() return game:GetService("TweenService"):Create(Part, TweenInfo.new(1), { Name = "x" }) end },
	{ "require-nonmodule", function() return require(Part) end },
	{ "require-number", function() return require(5) end },
	{ "require-nil", function() return require(nil) end },
	{ "fire-nonbindable", function() return Part.Changed:Fire() end },
	{ "connect-dot", function() return Part.Changed.Connect(function() end) end },
	{ "signal-wait-dot", function() return Part.Changed.Wait() end },
	{ "attribute-bad", function() Part:SetAttribute("A", {}) end },
	{ "getattribute-dot", function() return Part.GetAttribute("A") end },
}
for _, Case in Cases do
	print("error " .. Case[1] .. " | " .. Catch(Case[2]))
end

-- xpcall and the traceback a handler sees
local Traceback
xpcall(function()
	local function Deep()
		error("deep error")
	end
	Deep()
end, function(Message)
	Traceback = debug.traceback(Message, 1)
end)
print("xpcall-traceback " .. P.trace(Traceback))
print("traceback-plain " .. P.trace(debug.traceback("msg")))
print("traceback-level " .. P.trace(debug.traceback("msg", 2)))
print("traceback-nonstring " .. P.try(function()
	return type(debug.traceback({}))
end) .. " " .. P.try(function()
	return type(debug.traceback(5))
end))
print("traceback-thread " .. P.try(function()
	local Thread = coroutine.create(function()
		coroutine.yield()
	end)
	coroutine.resume(Thread)
	return P.trace(debug.traceback(Thread))
end))
print("xpcall-args " .. P.try(function()
	return xpcall(function(A, B)
		return A + B
	end, print, 2, 3)
end))
print("xpcall-handler-error " .. P.try(function()
	local Ok, Result = xpcall(function()
		error("first")
	end, function()
		error("in handler")
	end)
	return tostring(Ok) .. " " .. P.err(Result)
end))
print("pcall-nonfunction " .. P.try(function()
	return pcall(5)
end))
print("pcall-yield " .. P.try(function()
	return pcall(function()
		task.wait()
		return "yielded"
	end)
end))
print("error-rethrow-position " .. Catch(function()
	local Ok, Message = pcall(error, "inner")
	error(Message)
end))
print("error-in-tostring " .. Catch(function()
	return tostring(setmetatable({}, { __tostring = function()
		error("tostring failed")
	end }))
end))
print("debug-info " .. P.try(function()
	local function Named() end
	local Source, Line, Name, Arity, Variadic = debug.info(Named, "slna")
	return tostring(Name) .. " " .. tostring(Arity) .. " " .. tostring(Variadic) .. " " .. tostring(Line - (debug.info(1, "l") - P.line(1)) >= 0)
end) .. " | " .. P.try(function()
	return debug.info(tostring, "s") .. " " .. debug.info(tostring, "n") .. " " .. tostring(debug.info(tostring, "l"))
end) .. " | " .. P.try(function()
	return tostring(debug.info(100, "s"))
end))
print("debug-other " .. tostring(debug.getinfo) .. " " .. tostring(debug.sethook) .. " " .. type(debug.profilebegin) .. " " .. type(debug.setmemorycategory) .. " " .. type(debug.resetmemorycategory) .. " " .. type(debug.dumpcodesize))
print("getfenv " .. P.try(function()
	return type(getfenv(1)) .. " " .. type(getfenv(0)) .. " script " .. tostring(getfenv(1).script ~= nil)
end))
print("loadstring " .. P.try(function()
	return type(loadstring)
end) .. " " .. P.try(function()
	return loadstring("return 1")
end))
print("newproxy " .. P.try(function()
	local Proxy = newproxy(true)
	getmetatable(Proxy).__tostring = function()
		return "proxy!"
	end
	return typeof(Proxy) .. " " .. tostring(Proxy)
end))
print("globals " .. table.concat((function()
	local Names = {}
	for _, Name in { "shared", "_G", "_VERSION", "workspace", "Workspace", "game", "Game", "script", "plugin", "settings", "UserSettings", "tick", "time", "elapsedTime", "wait", "delay", "spawn", "ypcall", "printidentity", "stats", "version", "PluginManager", "LoadLibrary", "collectgarbage", "gcinfo", "require", "unpack", "Stats", "warn", "DebuggerManager", "utf8", "buffer", "vector", "task", "bit32" } do
		local Value = getfenv(1)[Name]
		table.insert(Names, Name .. "=" .. type(Value))
	end
	return Names
end)(), " "))
print("_VERSION " .. tostring(_VERSION))
print("collectgarbage " .. P.try(function()
	return type(collectgarbage("count"))
end) .. " | " .. P.try(collectgarbage) .. " | " .. P.try(collectgarbage, "collect") .. " | " .. P.try(collectgarbage, "step"))
print("gcinfo-type " .. type(gcinfo()))
print("time-funcs " .. type(tick()) .. " " .. type(time()) .. " " .. type(elapsedTime()) .. " time-positive " .. tostring(time() >= 0))
print("ypcall " .. P.try(function()
	return ypcall(function()
		return 1
	end)
end))
print("settings " .. P.try(function()
	return typeof(settings())
end) .. " | " .. P.try(function()
	return typeof(UserSettings())
end))
