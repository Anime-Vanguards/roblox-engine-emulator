-- parity: identity=4
-- Whose globals getfenv(0) reads (a client's and a server's thread globals): scripts A and B and a module M of the game's VM; a
-- module's function on A's thread and the threads it makes there (task.spawn, task.defer, coroutine.wrap and create), handlers
-- connected in A and fired from B, an OnInvoke A set that B invokes, a Heartbeat handler, setfenv(0) and the threads it makes,
-- loadstring's chunk where a server has it. The scripts log through attributes (a client's job runs in a VM of its own).
local RS = game:GetService("ReplicatedStorage")
local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local IS_SERVER = RunService:IsServer()
local Log = Instance.new("Folder")
Log.Name = "E3Log"
Log:SetAttribute("n", 0)
Log.Parent = RS
local HELPER = 'local function L(s) local Log = game:GetService("ReplicatedStorage").E3Log local n = Log:GetAttribute("n") + 1 Log:SetAttribute("n", n) Log:SetAttribute("l" .. n, s) end\n'
local player = Players.LocalPlayer or Players:GetPlayers()[1]
if not player then
	-- (ree's server run has none: its harness lets one in, Context.Join; the real engine's job has no Context)
	player = Context.Join()
end
local PG = if IS_SERVER then game:GetService("ServerScriptService") else player:WaitForChild("PlayerGui")
local function make(name, body)
	local s = Instance.new(if IS_SERVER then "Script" else "LocalScript")
	s.Name = name
	s.Source = HELPER .. body
	return s
end
local BE = Instance.new("BindableEvent")
BE.Name = "E3Event"
BE.Parent = RS
local BF = Instance.new("BindableFunction")
BF.Name = "E3Function"
BF.Parent = RS
local IV = Instance.new("IntValue")
IV.Name = "E3Value"
IV.Parent = RS
local M = Instance.new("ModuleScript")
M.Name = "E3M"
M.Source = HELPER .. [[
_G.E3envs = _G.E3envs or {}
_G.E3envs.M = getfenv(1)
local M = {}
function M.who(e)
	e = e or getfenv(0)
	for k, v in _G.E3envs do
		if v == e then
			return k
		end
	end
	return if e == nil then "nil" else "other"
end
L("M chunk's thread " .. M.who())
function M.spawned(tag)
	task.spawn(function() L(tag .. " task.spawn in M " .. M.who()) end)
	task.defer(function() L(tag .. " task.defer in M " .. M.who()) end)
	L(tag .. " coroutine.wrap in M " .. coroutine.wrap(function() return M.who() end)())
	local co = coroutine.create(function() return M.who() end)
	L(tag .. " coroutine.create in M " .. select(2, coroutine.resume(co)))
end
function M.load(tag)
	local ok, chunk = pcall(loadstring, "return getfenv(1), getfenv(0)")
	if not ok or not chunk then
		L(tag .. " loadstring in M refused: " .. tostring(chunk))
		return
	end
	local one, zero = chunk()
	L(tag .. " loadstring in M: chunk env " .. M.who(one) .. ", getfenv(0) " .. M.who(zero))
end
return M
]]
M.Parent = RS
local A = make("A", [[
_G.E3envs = _G.E3envs or {}
_G.E3envs.A = getfenv(1)
local M = require(game:GetService("ReplicatedStorage"):WaitForChild("E3M"))
local RS = game:GetService("ReplicatedStorage")
L("A main " .. M.who() .. ", getfenv(0) == getfenv(1) " .. tostring(getfenv(0) == getfenv(1)))
M.spawned("A")
M.load("A")
RS.E3Event.Event:Connect(function(from) L("A's handler of E3Event fired by " .. from .. ": " .. M.who()) end)
RS.E3Value.Changed:Connect(function() L("A's handler of Changed: " .. M.who()) end)
RS.E3Function.OnInvoke = function(from) L("A's OnInvoke invoked by " .. from .. ": " .. M.who()) return true end
game:GetService("RunService").Heartbeat:Once(function() L("A's Heartbeat handler: " .. M.who()) end)
task.spawn(function()
	local T = setmetatable({}, { __index = getfenv(0) })
	_G.E3envs.T = T
	setfenv(0, T)
	L("A after setfenv(0, T): getfenv(0) " .. M.who() .. ", getfenv(1) " .. M.who(getfenv(1)))
	task.spawn(function() L("A's child of a setfenv(0) thread: " .. M.who()) end)
	M.load("A-T")
end)
RS:SetAttribute("E3Aready", true)
]])
local B = make("B", [[
_G.E3envs = _G.E3envs or {}
_G.E3envs.B = getfenv(1)
local RS = game:GetService("ReplicatedStorage")
local M = require(RS:WaitForChild("E3M"))
while not RS:GetAttribute("E3Aready") do task.wait() end
L("B main " .. M.who())
M.spawned("B")
RS.E3Event:Fire("B")
RS.E3Value.Value += 1
RS.E3Function:Invoke("B")
L("B fired")
]])
task.wait()
A.Parent = PG
task.wait(0.2)
B.Parent = PG
task.wait(0.5)
for i = 1, Log:GetAttribute("n") do
	print(Log:GetAttribute("l" .. i))
end
A:Destroy()
B:Destroy()
M:Destroy()
BE:Destroy()
BF:Destroy()
IV:Destroy()
RS:SetAttribute("E3Aready", nil)
Log:Destroy()
