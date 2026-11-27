-- parity: identity=4
-- A script's environment and require, as a client and a server make them: the environment's own keys and its locked metatable,
-- require as one global function that finds the calling script (the nearest Luau function's environment's `script`, at the call),
-- what each require-by-string path answers, and what a module whose source does not compile logs. Writing Source needs identity
-- 4 in a real engine.
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
local Folder = Instance.new("Folder")
Folder.Name = "ParityEnv"
Folder.Parent = ReplicatedStorage
local function Clean(Text)
	return (string.gsub(tostring(Text), "0x%x+", "ADDR"))
end
local function Make(Name, Source, Parent, Class)
	local Script = Instance.new(Class or "ModuleScript")
	Script.Name = Name
	Script.Source = Source
	Script.Parent = Parent or Folder
	return Script
end
local function Try(Function, ...)
	local Results = table.pack(pcall(Function, ...))
	for Index = 2, Results.n do
		Results[Index] = Clean(Results[Index])
	end
	return (if Results[1] then "ok " else "error ") .. table.concat(Results, " ", 2, Results.n)
end

-- the environment
local EnvA = require(Make("EnvA", "local R = require return { env = getfenv(1), req = R, thread = getfenv(0), keys = function() local K = {} for k in next, getfenv(1) do K[#K + 1] = tostring(k) end return table.concat(K, ',') end } -- env-a"))
local EnvB = require(Make("EnvB", "return { env = getfenv(1), req = require } -- env-b"))
local Env = EnvA.env
local Keys = {}
for Key in next, EnvB.env do
	table.insert(Keys, tostring(Key))
end
print("keys " .. EnvA.keys() .. " | " .. table.concat(Keys, ","))
local Meta = getmetatable(Env)
print("metatable " .. type(Meta) .. " " .. Clean(Meta))
print("setmetatable " .. Try(setmetatable, Env, {}) .. " | nil " .. Try(setmetatable, Env, nil))
print("frozen " .. tostring(table.isfrozen(Env)) .. " write " .. Try(function()
	Env.ParityWritten = 1
	return rawget(Env, "ParityWritten")
end))
print("raw require " .. type(rawget(Env, "require")) .. " script " .. Clean(rawget(Env, "script")) .. " _G " .. type(rawget(Env, "_G")) .. " shared " .. type(rawget(Env, "shared")) .. " game " .. type(rawget(Env, "game")))
print("require one " .. tostring(rawequal(EnvA.req, EnvB.req)) .. " through-index " .. tostring(rawequal(Env.require, EnvA.req)) .. " here " .. tostring(rawequal(require, EnvA.req)))
print("_G one " .. tostring(rawequal(rawget(Env, "_G"), rawget(EnvB.env, "_G"))) .. " here " .. tostring(rawequal(rawget(Env, "_G"), _G)) .. " shared one " .. tostring(rawequal(rawget(Env, "shared"), rawget(EnvB.env, "shared"))) .. " here " .. tostring(rawequal(rawget(Env, "shared"), shared)))
print("chunk thread-env " .. tostring(rawequal(EnvA.thread, Env)))
print("require " .. Clean(require) .. " " .. tostring(debug.info(EnvA.req, "s")) .. " " .. tostring(debug.info(EnvA.req, "n")))

-- which script a path is resolved from
local F1 = Instance.new("Folder")
F1.Name = "F1"
F1.Parent = Folder
local F2 = Instance.new("Folder")
F2.Name = "F2"
F2.Parent = Folder
Make("Sib", "return 'F1-sib' -- env-s1", F1)
Make("Sib", "return 'F2-sib' -- env-s2", F2)
local Other = Make("Other", "return 'F2-other' -- env-o", F2)
local CallerModule = Make("Caller", table.concat({
	"local R = require",
	"local M = {}",
	"function M.call(p) return require(p) end",
	"function M.upvalue(p) return R(p) end",
	"function M.pcalled(p) return select(2, pcall(require, p)) end",
	"function M.set(s) script = s end",
	"function M.rawset(s) rawset(getfenv(1), 'script', s) end",
	"function M.make() return function(p) return R(p) end end",
	"function M.wrapped(p) return coroutine.wrap(R)(p) end",
	"function M.spawned(p, Out) task.spawn(function() Out.v = select(2, pcall(R, p)) end) end",
	"function M.bare(p) local co = coroutine.create(R) return select(2, coroutine.resume(co, p)) end",
	"return M -- env-caller",
}, "\n"), F1)
Make("Child", "return 'caller-child' -- env-child", CallerModule)
local Caller = require(CallerModule)
print("caller call " .. Try(Caller.call, "./Sib") .. " | upvalue " .. Try(Caller.upvalue, "./Sib") .. " | pcalled " .. Try(Caller.pcalled, "./Sib"))
print("caller wrapped " .. P.err(Try(Caller.wrapped, "./Sib")))
print("caller bare-thread " .. Try(Caller.bare, "./Sib") .. " | instance " .. Try(Caller.bare, Other) .. " | nothing " .. Try(Caller.bare))
local Out = {}
Caller.spawned("./Sib", Out)
task.wait()
print("caller spawned " .. Clean(Out.v))
local Moved = Caller.make()
setfenv(Moved, setmetatable({ script = Other }, { __index = getfenv(Moved) }))
print("setfenv script " .. Try(Moved, "./Sib"))
local Bare = Caller.make()
setfenv(Bare, {})
print("setfenv empty " .. Try(Bare, "./Sib") .. " | instance " .. Try(Bare, Other))
for _, Path in { "@self", "@self/x", "@game/ReplicatedStorage/ParityEnv/F1/Sib" } do
	print("setfenv empty [" .. Path .. "] " .. Try(Bare, Path))
end
local Indexed = Caller.make()
setfenv(Indexed, setmetatable({}, { __index = { script = Other } }))
print("setfenv indexed " .. Try(Indexed, "./Sib"))
local Text = Caller.make()
setfenv(Text, { script = "not a script" })
print("setfenv string " .. Try(Text, "./Sib"))
local Folderish = Caller.make()
setfenv(Folderish, { script = F2 })
print("setfenv folder " .. Try(Folderish, "./Sib"))
Caller.set(Other)
print("script written " .. Try(Caller.call, "./Sib") .. " | upvalue " .. Try(Caller.upvalue, "./Sib"))
Caller.rawset(CallerModule)
print("script rawset " .. Try(Caller.call, "./Sib"))
Caller.set(nil)
print("script nil " .. Try(Caller.call, "./Sib") .. " | instance " .. Try(Caller.call, Other))
Caller.set(CallerModule)
local Loose = Instance.new("ModuleScript")
Loose.Name = "Loose"
Loose.Source = "local R = require return { call = function(p) return R(p) end } -- env-loose"
local LooseCall = require(Loose).call
for _, Path in { "./Sib", "../Sib", "@self", "@self/x", "@game/ReplicatedStorage/ParityEnv/F1/Sib" } do
	print("parentless [" .. Path .. "] " .. Try(LooseCall, Path))
end

-- paths, from the module F1.Caller
Make("Lower", "return 'F1-lower' -- env-lo", F1)
local Plain = Instance.new("Folder")
Plain.Name = "Plain"
Plain.Parent = F1
local Inner = Make("Inner", "return 'F1-inner' -- env-inner", F1)
Make("init", "return 'inner-init' -- env-init", Inner)
for _, Path in {
	"./Sib", "./Missing", "../F2/Sib", "../Missing/Sib", "./Sib/", "./Sib.lua", "./Sib.luau", "./sib", "./LOWER", "./Plain", "./Plain/X",
	"./", ".", "", "..", "../", "./Inner", "./Inner/init", "@self/Child", "@self/Missing", "@game", "@game/ReplicatedStorage/ParityEnv/F2/Sib",
	"@ReplicatedStorage", "@Unknown/x", "@", "/Sib", "F2/Sib", "./../F2/Sib", ".//Sib", "../../../../../../../../../x", "./Sib/../Sib",
	"@Self/Child", "@SELF/Child", "@Game/ReplicatedStorage/ParityEnv/F1/Sib", "@game/", "@game//ReplicatedStorage/ParityEnv/F1/Sib", "@self/..",
	"@self/../Sib", "@game/..", "@plugin", "@self/Child/", "./Sib/./", "@game/Workspace", "./Child", "@self/Child/x", "@game/NotAService/x",
	".\\Sib", "@game\\ReplicatedStorage\\ParityEnv\\F1\\Sib", "./Sib\\", "@game/x/..", "@game/ReplicatedStorage/../..",
} do
	print("path [" .. Path .. "] " .. Try(Caller.call, Path))
end

-- a module whose source does not compile: the log, ScriptContext.Error, later requires
local Log = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if string.find(Message, "ParityEnv") or string.find(Message, "Requested") then
		table.insert(Log, Kind.Name .. ":" .. string.gsub(Message, "\n", " / "))
	end
end)
local Errors = {}
local ErrorListener = ScriptContext.Error:Connect(function(Message, Trace, Script)
	table.insert(Errors, tostring(Message) .. " [" .. string.gsub(tostring(Trace), "\n", " / ") .. "] " .. (if Script then Script:GetFullName() else "nil"))
end)
local function Flush(Label)
	task.wait()
	task.wait()
	print(Label .. " log " .. table.concat(Log, " | "))
	print(Label .. " errors " .. table.concat(Errors, " | "))
	table.clear(Log)
	table.clear(Errors)
end
local Bad = Make("BadSyntax", "local x = 1\nlocal = 2 -- env-bad")
print("syntax " .. Try(require, Bad))
Flush("syntax")
print("syntax again " .. Try(require, Bad))
Bad.Source = "return 'fixed' -- env-fixed"
print("syntax fixed " .. Try(require, Bad))
Flush("syntax again")
print("syntax twin " .. Try(require, Make("BadTwin", "local x = 1\nlocal = 2 -- env-bad")))
Flush("syntax twin")
print("empty " .. Try(require, Make("Empty", "")))
Make("BadInner", "return ( -- env-bad-inner")
print("syntax inner " .. Try(require, Make("Outer", "return require(script.Parent.BadInner) -- env-outer")))
Flush("syntax inner")
ErrorListener:Disconnect()
Listener:Disconnect()

-- an Actor's script: its own _G and shared, its environment
if RunService:IsClient() then
	shared.ParityMark = "main-shared"
	_G.ParityMark = "main-G"
	local Gui = Instance.new("Folder")
	Gui.Name = "ParityEnvGui"
	Gui.Parent = game:GetService("Players").LocalPlayer:WaitForChild("PlayerGui")
	local Actor = Instance.new("Actor")
	local Inside = Make("ActorScript", table.concat({
		"script:SetAttribute('Shared', tostring(shared.ParityMark)) script:SetAttribute('G', tostring(_G.ParityMark))",
		"local E = getfenv(1) local K = {} for k in next, E do K[#K + 1] = tostring(k) end",
		"script:SetAttribute('Keys', table.concat(K, ',')) script:SetAttribute('Meta', type(getmetatable(E)) .. ' ' .. tostring(getmetatable(E)))",
		"script:SetAttribute('ReqOne', tostring(rawequal(require(script.M).req, require)))",
		"-- env-actor",
	}, "\n"), Actor, "LocalScript")
	Make("M", "return { req = require } -- env-actor-m", Inside)
	Actor.Parent = Gui
	local function Seen(Name)
		for _ = 1, 30 do
			local Value = Inside:GetAttribute(Name)
			if Value ~= nil then
				return Value
			end
			task.wait()
		end
		return "never"
	end
	print("actor shared " .. Seen("Shared") .. " _G " .. Seen("G") .. " keys " .. Seen("Keys") .. " metatable " .. Seen("Meta") .. " require one " .. Seen("ReqOne"))
	print("actor after shared " .. tostring(shared.ParityMark) .. " _G " .. tostring(_G.ParityMark))
	Gui:Destroy()
end
Folder:Destroy()
