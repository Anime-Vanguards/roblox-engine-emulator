-- parity: identity=4
-- Scripts a script parents in at run time: where a client starts a LocalScript and a server a Script (RunContext Legacy), when
-- (its place in the deferred queue: after the Parent write's own events, the script's AncestryChanged among them; parented
-- after a wait, in Heartbeat and PreSimulation handlers, in a deferred thread, by a script that just started; an Enabled
-- write; into an Actor), a tree's scripts in its post-order, the thread a script starts on and its errors. The scripts run in
-- the game's VM (a client's job runs in a VM of its own), so they log through attributes of a Folder in ReplicatedStorage.
local RS = game:GetService("ReplicatedStorage")
local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
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
	player = Instance.new("Player")
	player.Parent = Players
end
local since = time()
while not player.Character and time() - since < 5 do
	task.wait()
end
if not player.Character then
	local model = Instance.new("Model")
	model.Name = player.Name
	local root = Instance.new("Part")
	root.Name = "HumanoidRootPart"
	root.Anchored = true
	root.Parent = model
	player.Character = model
	model.Parent = workspace
end
local character = player.Character
local function folder(parent, name, class)
	local f = Instance.new(class or "Folder")
	f.Name = name
	f.Parent = parent
	return f
end
local function make(name, body)
	local s = Instance.new(CLASS)
	s.Name = name
	s.Source = HELPER .. (body or 'L(script.Name .. " start")')
	return s
end
local function dump(title)
	print("== " .. title)
	for i = 1, Log:GetAttribute("n") do
		print(Log:GetAttribute("l" .. i))
	end
	Log:SetAttribute("n", 0)
end

-- 1. Where: one script a container, made in one stretch (the rest of each line names where it did not start)
local containers = if IS_SERVER
	then {
		{ "Workspace", function() return workspace end },
		{ "WorkspaceModel", function() return folder(workspace, "E3Model", "Model") end },
		{ "ServerScriptService", function() return game:GetService("ServerScriptService") end },
		{ "ServerScriptServiceFolder", function() return folder(game:GetService("ServerScriptService"), "E3Folder") end },
		{ "PlayerGui", function() return player:FindFirstChildOfClass("PlayerGui") end },
		{ "Backpack", function() return player:FindFirstChildOfClass("Backpack") end },
		{ "Character", function() return character end },
		{ "ServerStorage", function() return game:GetService("ServerStorage") end },
		{ "ReplicatedStorage", function() return RS end },
		{ "ReplicatedFirst", function() return game:GetService("ReplicatedFirst") end },
		{ "Lighting", function() return game:GetService("Lighting") end },
		{ "Players", function() return Players end },
		{ "Player", function() return player end },
		{ "StarterGui", function() return game:GetService("StarterGui") end },
		{ "StarterPack", function() return game:GetService("StarterPack") end },
	}
	else {
		{ "PlayerGui", function() return player:FindFirstChildOfClass("PlayerGui") end },
		{ "PlayerGuiScreenGui", function() return folder(player:FindFirstChildOfClass("PlayerGui"), "E3Screen", "ScreenGui") end },
		{ "Backpack", function() return player:FindFirstChildOfClass("Backpack") end },
		{ "BackpackTool", function() return folder(player:FindFirstChildOfClass("Backpack"), "E3Tool", "Tool") end },
		{ "Character", function() return character end },
		{ "CharacterPart", function() return character:FindFirstChild("HumanoidRootPart") end },
		{ "PlayerScripts", function() return player:FindFirstChildOfClass("PlayerScripts") end },
		{ "PlayerScriptsFolder", function() return folder(player:FindFirstChildOfClass("PlayerScripts"), "E3Folder") end },
		{ "ReplicatedFirst", function() return game:GetService("ReplicatedFirst") end },
		{ "ReplicatedFirstFolder", function() return folder(game:GetService("ReplicatedFirst"), "E3Folder") end },
		{ "Player", function() return player end },
		{ "StarterGear", function() return player:FindFirstChild("StarterGear") end },
		{ "Workspace", function() return workspace end },
		{ "WorkspaceModel", function() return folder(workspace, "E3Model", "Model") end },
		{ "ReplicatedStorage", function() return RS end },
		{ "Lighting", function() return game:GetService("Lighting") end },
		{ "Players", function() return Players end },
		{ "StarterGui", function() return game:GetService("StarterGui") end },
		{ "StarterPack", function() return game:GetService("StarterPack") end },
	}
task.wait()
local made = {}
for _, entry in containers do
	local parent = entry[2]()
	if not parent then
		print("no container " .. entry[1])
		continue
	end
	local s = make("At" .. entry[1])
	s.Parent = parent
	table.insert(made, s)
end
L("made all")
task.wait(0.2)
dump("where")
for _, s in made do
	s:Destroy()
end

-- 2. When: against the writing thread, its task.defer / task.spawn / task.delay(0), a ChildAdded handler and the frame's events
local PG = if IS_SERVER then game:GetService("ServerScriptService") else player:FindFirstChildOfClass("PlayerGui")
local Frames, Window, Tag = 0, 0, ""
local function marker(phase)
	return function()
		if Window > 0 then
			L(Tag .. " M" .. Frames .. " " .. phase)
		end
	end
end
local markers = {
	RunService.PreAnimation:Connect(marker("PreAnimation")),
	RunService.PreSimulation:Connect(marker("PreSimulation")),
	RunService.PostSimulation:Connect(marker("PostSimulation")),
	RunService.Heartbeat:Connect(function()
		if Window > 0 then
			L(Tag .. " M" .. Frames .. " Heartbeat")
			Window -= 1
		end
		Frames += 1
	end),
}
local TIMED = 'L(script.Name .. " start") task.defer(function() L(script.Name .. " s-defer") end) task.wait() L(script.Name .. " s-resumed")'
local function begin(tag)
	Tag, Window, Frames = tag, 3, 0
end
local function settle()
	while Window > 0 do
		task.wait()
	end
	task.wait(0.1)
end
local function parentIt(tag, s, parent)
	L(tag .. " before")
	local c = parent.ChildAdded:Connect(function(child)
		L(tag .. " childadded " .. child.Name)
	end)
	task.defer(function() L(tag .. " defer-before") end)
	s.Parent = parent
	L(tag .. " after")
	task.defer(function() L(tag .. " defer-after") end)
	task.spawn(function() L(tag .. " spawn") end)
	task.delay(0, function() L(tag .. " delay0") end)
	task.defer(function() c:Disconnect() end)
end
task.wait(0.2)
begin("A")
parentIt("A", make("A_s", TIMED), PG)
L("A yield")
task.wait()
L("A resumed")
settle()
begin("B")
local hb
hb = RunService.Heartbeat:Connect(function()
	hb:Disconnect()
	parentIt("B", make("B_s", TIMED), PG)
	L("B handler end")
end)
settle()
begin("C")
local ps
ps = RunService.PreSimulation:Connect(function()
	ps:Disconnect()
	parentIt("C", make("C_s", TIMED), PG)
	L("C handler end")
end)
settle()
begin("D")
task.defer(function()
	parentIt("D", make("D_s", TIMED), PG)
	L("D defer end")
end)
L("D yield")
task.wait()
settle()
begin("E")
local inner = make("E_inner", TIMED)
inner.Parent = RS
parentIt("E", make("E_outer", 'L("E_outer start") local i = game:GetService("ReplicatedStorage").E_inner i.Parent = script.Parent L("E_outer parented") task.defer(function() L("E_outer defer") end)'), PG)
L("E yield")
task.wait()
settle()
begin("F")
local f1, f2 = make("F_one", TIMED), make("F_two", TIMED)
f2.Enabled = false
f1.Parent = PG
f2.Parent = PG
L("F parented")
f2.Enabled = true
L("F enabled")
task.wait()
settle()
-- (an Actor's scripts: a LocalScript in the PlayerScripts on a client, a Script in the Workspace on a server)
local actorHome = if IS_SERVER then workspace else player:FindFirstChildOfClass("PlayerScripts")
local actor = Instance.new("Actor")
actor.Parent = actorHome
task.wait(0.1)
begin("G")
parentIt("G", make("G_s", TIMED), actor)
L("G yield")
task.wait()
settle()
begin("H")
local actor2 = Instance.new("Actor")
make("H_s", TIMED).Parent = actor2
parentIt("H", actor2, actorHome)
L("H yield")
task.wait()
settle()
for _, c in markers do
	c:Disconnect()
end
dump("when")
for _, s in PG:GetChildren() do
	if s:IsA("BaseScript") and s.Name:match("^%u_") then
		s:Destroy()
	end
end
actor:Destroy()
actor2:Destroy()

-- 3. The start among the Parent write's events
task.wait(0.1)
do
	local s = make("Ev")
	local f = Instance.new("Folder")
	f.Name = "EvFolder"
	s.Parent = f
	local cons = {}
	local function on(signal, text)
		table.insert(cons, signal:Connect(function(x) L(text .. " " .. tostring(typeof(x) == "Instance" and x.Name or x)) end))
	end
	on(PG.ChildAdded, "PG.ChildAdded")
	on(PG.DescendantAdded, "PG.DescendantAdded")
	on(game.DescendantAdded, "game.DescendantAdded")
	on(s.AncestryChanged, "s.AncestryChanged")
	on(f.AncestryChanged, "f.AncestryChanged")
	on(f.Changed, "f.Changed")
	on(s.Changed, "s.Changed")
	task.defer(function() L("defer-before") end)
	f.Parent = PG
	task.defer(function() L("defer-after") end)
	task.wait()
	for _, c in cons do
		c:Disconnect()
	end
	f:Destroy()
end
-- a tree's scripts: a folder holding a, b (holding b1, b2), c, made in that order
task.wait(0.1)
do
	local f = Instance.new("Folder")
	make("Ta").Parent = f
	local b = make("Tb")
	b.Parent = f
	make("Tb1").Parent = b
	make("Tb2").Parent = b
	make("Tc").Parent = f
	f.Parent = PG
	task.wait()
	f:Destroy()
end
dump("order")

-- 4. The thread a script starts on, and its errors (a stack of its own; a source that does not compile is logged once, with
-- no stack and no ScriptContext.Error)
local function clean(text)
	return (string.gsub(string.gsub(text, "\n", " | "), "[%w_.]+%.E3T", "<path>.E3T"))
end
local messages = LogService.MessageOut:Connect(function(message, kind)
	if string.find(message, "E3T", 1, true) then
		L("log " .. kind.Name .. " " .. clean(message))
	end
end)
local errors = ScriptContext.Error:Connect(function(message, trace, culprit)
	if string.find(message, "E3T", 1, true) or (culprit and string.find(culprit.Name, "E3T", 1, true)) then
		L("ScriptContext.Error " .. clean(message) .. " || " .. clean(trace) .. " || " .. tostring(culprit and culprit.Name))
	end
end)
task.wait()
make("E3T1", [[
L("E3T1 status " .. coroutine.status(coroutine.running()) .. ", below it " .. select("#", debug.info(2, "s")) .. " values")
L("E3T1 traceback " .. select(2, string.gsub(debug.traceback(), "\n", "")) .. " lines")
error("E3T1 boom")
]]).Parent = PG
task.wait(0.1)
make("E3T2", [[
task.wait()
error("E3T2 boom after a wait")
]]).Parent = PG
task.wait(0.1)
make("E3T3", "local x = = 1 -- E3T3").Parent = PG
task.wait(0.2)
messages:Disconnect()
errors:Disconnect()
dump("thread")
for _, s in PG:GetChildren() do
	if s.Name:match("^E3T") then
		s:Destroy()
	end
end
Log:Destroy()
