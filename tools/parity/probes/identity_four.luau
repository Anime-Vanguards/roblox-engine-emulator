-- parity: identity=4
-- What code at the command bar's identity (4) may do that a game script may not, and which code keeps it (ree: `--identity 4`,
-- record/run.rs): it writes RunContext (cast first; a change fires Changed for Source, then for RunContext; a LocalScript's is
-- cast and stays as it is), reads and writes Source, makes a PluginAction and a DebuggerWatch but no Breakpoint nor Player. What
-- it spawns, defers, delays and wraps, and its handlers whoever fires them, keep it; a game script it starts, that script's
-- handlers and a module's chunk run at a game script's identity, a module's function at its caller's. The scripts it makes log
-- through attributes of a Folder (a client's job runs in a VM of its own).
local RS = game:GetService("ReplicatedStorage")
local IS_SERVER = game:GetService("RunService"):IsServer()
for _, Name in { "PluginAction", "DebuggerWatch", "Breakpoint", "Player" } do
	print("new", Name, P.try(function() return Instance.new(Name).ClassName end))
end
local S = Instance.new("Script")
print("write enum", P.try(function() S.RunContext = Enum.RunContext.Client return S.RunContext end))
print("write string", P.try(function() S.RunContext = "Server" return S.RunContext end))
print("write number", P.try(function() S.RunContext = 0 return S.RunContext end))
print("write bad", P.try(function() S.RunContext = "Nope" return S.RunContext end))
print("write table", P.try(function() S.RunContext = {} return S.RunContext end))
print("write plugin", P.try(function() S.RunContext = Enum.RunContext.Plugin return S.RunContext end))
print("write same", P.try(function() S.RunContext = S.RunContext return S.RunContext end))
print("source", P.try(function() S.Source = "return 1" return S.Source end))
local Changes = {}
S.Changed:Connect(function(Name) table.insert(Changes, Name .. "=" .. tostring(S.RunContext)) end)
S:GetPropertyChangedSignal("RunContext"):Connect(function() table.insert(Changes, "signal") end)
S:GetPropertyChangedSignal("Source"):Connect(function() table.insert(Changes, "source signal") end)
print("writes", P.try(function()
	S.RunContext = Enum.RunContext.Client
	S.RunContext = Enum.RunContext.Client
	S.RunContext = "Legacy"
end))
task.wait()
print("changed", table.concat(Changes, " "))
local LS = Instance.new("LocalScript")
local LocalChanges = {}
LS.Changed:Connect(function(Name) table.insert(LocalChanges, Name) end)
print("local write", P.try(function() LS.RunContext = Enum.RunContext.Server return LS.RunContext end))
print("local bad", P.try(function() LS.RunContext = "Nope" return LS.RunContext end))
task.wait()
print("local changed", #LocalChanges)

-- the threads this code makes, its handlers, a module it requires, and a game script it starts
local Log = Instance.new("Folder")
Log.Name = "IdentityLog"
Log.Parent = RS
local Bindable = Instance.new("BindableEvent")
Bindable.Name = "IdentityBindable"
Bindable.Parent = RS
local ATTEMPT = [[
local function attempt(Label)
	local Log = game:GetService("ReplicatedStorage").IdentityLog
	local Script = Instance.new("Script")
	local Ok, Problem = pcall(function() Script.RunContext = Enum.RunContext.Client end)
	Log:SetAttribute(string.gsub(Label, " ", "_"), if Ok then "ok" else (string.gsub(tostring(Problem), "^.-:%d+: ", "")))
end
]]
local function attempt(Label)
	local Script = Instance.new("Script")
	local Ok, Problem = pcall(function() Script.RunContext = Enum.RunContext.Client end)
	Log:SetAttribute(string.gsub(Label, " ", "_"), if Ok then "ok" else (string.gsub(tostring(Problem), "^.-:%d+: ", "")))
end
attempt("own")
task.spawn(function() attempt("spawn") end)
task.defer(function() attempt("defer") end)
task.delay(0, function() attempt("delay") end)
coroutine.wrap(function() attempt("coroutine") end)()
task.spawn(function() task.wait() attempt("spawn after wait") end)
Bindable.Event:Connect(function(From) attempt("own handler fired by " .. From) end)
Bindable:Fire("own")
local Module = Instance.new("ModuleScript")
Module.Name = "IdentityModule"
Module.Source = ATTEMPT .. 'attempt("module chunk")\nreturn attempt\n'
Module.Parent = RS
require(Module)("module function")
local Game = Instance.new(if IS_SERVER then "Script" else "LocalScript")
Game.Source = ATTEMPT .. [[
attempt("game script")
require(game:GetService("ReplicatedStorage").IdentityModule)("game script calls module")
local Bindable = game:GetService("ReplicatedStorage").IdentityBindable
Bindable.Event:Connect(function(From) attempt("game script handler fired by " .. From) end)
game:GetService("ReplicatedStorage").IdentityLog:SetAttribute("ready", true)
Bindable:Fire("game script")
]]
Game.Parent = if IS_SERVER then game:GetService("ServerScriptService") else game:GetService("ReplicatedFirst")
local Since = os.clock()
while not Log:GetAttribute("ready") and os.clock() - Since < 5 do
	task.wait()
end
Bindable:Fire("own")
task.wait(0.5)
local Names = {}
for Name in Log:GetAttributes() do
	table.insert(Names, Name)
end
table.sort(Names)
for _, Name in Names do
	print("thread", Name, Log:GetAttribute(Name))
end
Game:Destroy()
Module:Destroy()
Log:Destroy()
Bindable:Destroy()
