-- parity: identity=4
-- What stops a running script and what a stop takes: Enabled writes, moves out and back, between places it runs, clones, Destroy;
-- what it had waiting (its loop, a spawned thread, a connection, deferred and delayed work, a WaitForChild, an event's Wait, a
-- handler that yielded, a thread a module's function made for it) against what goes on (a module's own thread, what a script
-- makes after it disabled itself); its connections by the stop's cause; the source and environment of a restart. The scripts run
-- in the game's VM and log through attributes (a client's job runs in a VM of its own).
local RS = game:GetService("ReplicatedStorage")
local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local IS_SERVER = RunService:IsServer()
local CLASS = if IS_SERVER then "Script" else "LocalScript"
local Log = Instance.new("Folder")
Log.Name = "E3Log"
Log:SetAttribute("n", 0)
Log.Parent = RS
local function L(s)
	local n = Log:GetAttribute("n") + 1
	Log:SetAttribute("n", n)
	Log:SetAttribute("l" .. n, s)
end
local HELPER = 'local function L(s) local Log = game:GetService("ReplicatedStorage").E3Log local n = Log:GetAttribute("n") + 1 Log:SetAttribute("n", n) Log:SetAttribute("l" .. n, s) end\n'
-- (a playing session has a player with a character; ree's harness makes them where it has none)
local player = Players.LocalPlayer or Players:GetPlayers()[1]
if not player then
	-- (ree's server run has none: its harness lets one in, Context.Join; the real engine's job has no Context)
	player = Context.Join()
end
local since = time()
while not player.Character and time() - since < 5 do
	task.wait()
end
if not player.Character then
	local model = Instance.new("Model")
	model.Name = player.Name
	player.Character = model
	model.Parent = workspace
end
local CHARACTER = player.Character
local PG = if IS_SERVER then game:GetService("ServerScriptService") else player:FindFirstChildOfClass("PlayerGui")
local PACK = player:FindFirstChildOfClass("Backpack")
local OFF = RS
local function make(name, body)
	local s = Instance.new(CLASS)
	s.Name = name
	s.Source = HELPER .. body
	return s
end
local Window = 0
local beat = RunService.Heartbeat:Connect(function()
	if Window > 0 then
		L("HB")
		Window -= 1
	end
end)
local function frames(n)
	Window = n
	while Window > 0 do
		task.wait()
	end
end
local function dump(title)
	print("== " .. title)
	for i = 1, Log:GetAttribute("n") do
		print(Log:GetAttribute("l" .. i))
	end
	Log:SetAttribute("n", 0)
end
-- a script that keeps running: a loop, a spawned loop, a Heartbeat connection; each line names the run
local LIVE = [[
local run = (script:GetAttribute("runs") or 0) + 1
script:SetAttribute("runs", run)
local me = script.Name .. "#" .. run
L(me .. " start")
task.spawn(function() while true do task.wait() L(me .. " spawned") end end)
game:GetService("RunService").Heartbeat:Connect(function() L(me .. " conn") end)
while true do task.wait() L(me .. " loop") end
]]
task.wait(0.1)

-- 1. Enabled writes
do
	local s = make("En", LIVE)
	s.Parent = PG
	frames(2)
	L("Enabled false")
	s.Enabled = false
	frames(2)
	L("Enabled true")
	s.Enabled = true
	frames(2)
	L("Enabled false, true")
	s.Enabled = false
	s.Enabled = true
	frames(2)
	L("Enabled true again")
	s.Enabled = true
	frames(1)
	s:Destroy()
	frames(1)
end
dump("enabled")

-- 2. Moves
do
	local s = make("Mv", LIVE)
	s.Parent = PG
	frames(2)
	L("to a place it does not run")
	s.Parent = OFF
	frames(2)
	L("back")
	s.Parent = PG
	frames(2)
	L("to the Backpack")
	s.Parent = PACK
	frames(2)
	L("to nil and back")
	s.Parent = nil
	s.Parent = PACK
	frames(2)
	L("into a folder there")
	local f = Instance.new("Folder")
	f.Parent = PACK
	s.Parent = f
	frames(2)
	L("the folder to nil")
	f.Parent = nil
	frames(2)
	L("to the character")
	s.Parent = CHARACTER
	frames(2)
	s:Destroy()
	frames(1)
end
dump("moves")

-- 3. Clones and Destroy
do
	local s = make("Cl", LIVE)
	s.Parent = PG
	frames(1)
	L("a clone of it")
	local c = s:Clone()
	c.Name = "ClCopy"
	c.Parent = PG
	frames(2)
	local d = make("ClOff", 'L("ClOff start")')
	d.Enabled = false
	d.Parent = OFF
	L("a clone of a disabled one")
	local e = d:Clone()
	e.Parent = PG
	frames(1)
	L("enabled")
	e.Enabled = true
	frames(1)
	L("Destroy")
	s:Destroy()
	c:Destroy()
	frames(2)
	d:Destroy()
	e:Destroy()
end
dump("clones")

-- 4. A script that finished, out and back, its Enabled off and on; before its turn: parented and moved off, its Source
-- written, parented twice, disabled, destroyed
do
	local s = make("Fin", 'L("Fin start")')
	s.Parent = PG
	frames(1)
	L("out and back")
	s.Parent = OFF
	frames(1)
	s.Parent = PG
	frames(1)
	L("Enabled off, on")
	s.Enabled = false
	frames(1)
	s.Enabled = true
	frames(1)
	s:Destroy()
	local a = make("Bout", 'L("Bout start")')
	a.Parent = PG
	a.Parent = OFF
	local b = make("Bsrc", 'L("Bsrc start, first source")')
	b.Parent = PG
	b.Source = HELPER .. 'L("Bsrc start, second source")'
	local c = make("Btwice", 'L("Btwice start")')
	c.Parent = PG
	c.Parent = PACK
	c.Parent = PG
	local d = make("Bdis", 'L("Bdis start")')
	d.Parent = PG
	d.Enabled = false
	local e = make("Bdestroy", 'L("Bdestroy start")')
	e.Parent = PG
	e:Destroy()
	frames(1)
	L("a restart runs the Source as it is then")
	b.Enabled = false
	b.Enabled = true
	frames(1)
	for _, x in { a, b, c, d } do
		x:Destroy()
	end
end
dump("restarts")

-- 5. What a stop takes and what it leaves
local M = Instance.new("ModuleScript")
M.Name = "E3M"
M.Source = HELPER .. [[
local ticks = 0
task.spawn(function() while ticks < 8 do task.wait() ticks += 1 L("the module's own thread") end end)
local M = {}
function M.loop(tag)
	task.spawn(function() while true do task.wait() L(tag .. ": a thread a module's function made") end end)
end
return M
]]
M.Parent = RS
local Signal = Instance.new("BindableEvent")
Signal.Name = "E3Signal"
Signal.Parent = RS
do
	local s = make("S", [[
local RS = game:GetService("ReplicatedStorage")
local M = require(RS.E3M)
_G.E3env = _G.E3env or getfenv(1)
L("S start, a fresh environment " .. tostring(_G.E3env ~= getfenv(1)) .. ", a global the last run set " .. tostring(LastRun))
LastRun = true
M.loop("S")
RS.E3Signal.Event:Connect(function() L("S's handler runs") task.wait() L("S's handler resumed") end)
task.spawn(function() RS:WaitForChild("E3Later") L("S's WaitForChild returned") end)
task.spawn(function() RS.E3Signal.Event:Wait() L("S's Wait returned") end)
task.defer(function() while true do task.wait() L("S's deferred loop") end end)
while true do task.wait() L("S's loop") end
]])
	s.Parent = PG
	frames(1)
	L("fire, then stop a frame on")
	Signal:Fire()
	frames(1)
	s.Enabled = false
	L("stopped")
	local later = Instance.new("Folder")
	later.Name = "E3Later"
	later.Parent = RS
	Signal:Fire()
	frames(2)
	L("restart")
	s.Enabled = true
	frames(2)
	s:Destroy()
	frames(10)
	later:Destroy()
end
do
	local d = make("D", [[
task.defer(function() L("D's deferred work") end)
task.delay(0, function() L("D's delay(0)") end)
script:SetAttribute("queued", true)
while true do task.wait() L("D's loop") end
]])
	d.Parent = PG
	-- (stopped in the same stretch as its start: from a deferred thread queued after it)
	task.defer(function()
		L("D queued " .. tostring(d:GetAttribute("queued")))
		d.Enabled = false
		L("D stopped")
	end)
	frames(2)
	d:Destroy()
	local e = make("E", [[
task.defer(function() L("E's deferred work from before") end)
task.spawn(function() task.wait() L("E's spawned thread from before") end)
game:GetService("RunService").Heartbeat:Once(function() L("E's handler from before") end)
script.Enabled = false
L("E after disabling itself")
task.spawn(function() L("E's spawned thread from after") task.wait() L("E's spawned thread from after, resumed") end)
task.defer(function() L("E's deferred work from after") end)
game:GetService("RunService").Heartbeat:Once(function() L("E's handler from after") end)
task.wait()
L("E resumed")
task.wait()
L("E resumed again")
]])
	e.Parent = PG
	frames(3)
	e:Destroy()
end
dump("what a stop takes")
M:Destroy()

-- 6. A stopped script's connections, seen from another script of the VM, by the stop's cause
for index = 1, 4 do
	local event = Instance.new("BindableEvent")
	event.Name = "E3Event" .. index
	event.Parent = RS
end
local B = [[
local RS = game:GetService("ReplicatedStorage")
local me = {}
_G[script.Name] = me
local Event = RS["E3Event" .. string.sub(script.Name, 2)]
me.hb = game:GetService("RunService").Heartbeat:Connect(function() end)
me.be = Event.Event:Connect(function(tag) L(script.Name .. "'s handler runs: " .. tag) end)
me.main = coroutine.running()
me.waiter = task.spawn(function() Event.Event:Wait() L(script.Name .. "'s Wait returned") end)
me.sleeper = task.delay(10, function() L(script.Name .. "'s delay ran") end)
while true do task.wait() end
]]
for index = 1, 4 do
	make("B" .. index, B).Parent = PG
end
task.wait(0.2)
make("W", [[
local RS = game:GetService("ReplicatedStorage")
for index, cause in { "Enabled false", "Destroy", "Parent nil", "a move off" } do
	local name = "B" .. index
	local B = script.Parent:FindFirstChild(name)
	local me = _G[name]
	local function state(when)
		L(cause .. ", " .. when .. ": hb " .. tostring(me.hb.Connected) .. ", be " .. tostring(me.be.Connected) .. ", main "
			.. coroutine.status(me.main) .. ", waiter " .. coroutine.status(me.waiter) .. ", sleeper " .. coroutine.status(me.sleeper))
	end
	state("before")
	if cause == "Enabled false" then
		B.Enabled = false
	elseif cause == "Destroy" then
		B:Destroy()
	elseif cause == "Parent nil" then
		B.Parent = nil
	else
		B.Parent = RS
	end
	state("right after")
	RS["E3Event" .. index]:Fire(cause)
	state("right after a fire")
	task.defer(function() state("deferred") end)
	task.wait()
	state("next frame")
	task.wait(0.1)
end
script:SetAttribute("done", true)
]]).Parent = PG
local w = PG:WaitForChild("W")
while not w:GetAttribute("done") do
	task.wait()
end
dump("connections")
beat:Disconnect()
for _, x in PG:GetChildren() do
	if x:IsA("BaseScript") and (x.Name == "W" or string.match(x.Name, "^B%d$")) then
		x:Destroy()
	end
end
for index = 1, 4 do
	RS["E3Event" .. index]:Destroy()
	local held = RS:FindFirstChild("B" .. index)
	if held then
		held:Destroy()
	end
end
Signal:Destroy()
Log:Destroy()
