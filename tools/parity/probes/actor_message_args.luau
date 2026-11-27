-- parity: identity=4
-- Actor messages' arguments and errors: what each argument arrives as (copied as BindableEvent:Fire copies), the topic's cast,
-- BindToMessage's and SendMessage's argument errors, which code may bind (the Actor's VM: its scripts and the modules they
-- require, wherever those are), a handler's error, yield and thread, an Actor outside the DataModel, a disabled script.
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
local Registry = game:GetService("SharedTableRegistry")
-- (the workers are LocalScripts in the player's PlayerScripts, where a client runs them: a game script may not write RunContext)
local Home = game:GetService("Players").LocalPlayer:WaitForChild("PlayerScripts")
local HomeName = Home:GetFullName()
local function Plain(Text)
	local Start, Finish = string.find(Text, HomeName, 1, true)
	while Start do
		Text = string.sub(Text, 1, Start - 1) .. "PlayerScripts" .. string.sub(Text, Finish + 1)
		Start, Finish = string.find(Text, HomeName, 1, true)
	end
	return Text
end
local Shared = SharedTable.new()
Registry:SetSharedTable("ParityActorArgs", Shared)
Shared.N = 0
local function Add(Text)
	local Index = SharedTable.increment(Shared, "N", 1) + 1
	Shared["L" .. Index] = Text
end
local Outside = Instance.new("ModuleScript")
Outside.Name = "ParityOutsideBinder"
Outside.Source = [==[
return function(Actor, Add)
	local Ok, Message = pcall(function()
		return Actor:BindToMessage("FromModule", function() Add("the outside module's binding ran") end)
	end)
	Add(`a module outside the Actor binds: {Ok} {Message}; its GetActor {script:GetActor()}`)
end
]==]
Outside.Parent = game:GetService("ReplicatedStorage")
local Folder = Instance.new("Folder")
Folder.Name = "ParityActorArgs"
local Template = Instance.new("LocalScript")
Template.Name = "Worker"
Template.Enabled = false
Template.Source = [==[
local Shared = game:GetService("SharedTableRegistry"):GetSharedTable("ParityActorArgs")
local Actor = script:GetActor()
local Name = Actor.Name
local function Add(Text)
	local Index = SharedTable.increment(Shared, "N", 1) + 1
	Shared["L" .. Index] = `{Name} {Text}`
end
local function Show(Value, Depth)
	Depth = Depth or 0
	local Kind = typeof(Value)
	if Kind == "table" then
		if Depth > 3 then
			return "{...}"
		end
		local Keys = {}
		for Key in Value do
			table.insert(Keys, Key)
		end
		table.sort(Keys, function(A, B)
			return tostring(A) < tostring(B)
		end)
		local Parts = {}
		for _, Key in Keys do
			table.insert(Parts, `[{typeof(Key)} {tostring(Key)}]={Show(Value[Key], Depth + 1)}`)
		end
		return `table(meta={getmetatable(Value) ~= nil} frozen={table.isfrozen(Value)}){"{"}{table.concat(Parts, "; ")}{"}"}`
	elseif Kind == "Instance" then
		return `Instance({Value:GetFullName()})`
	elseif Kind == "SharedTable" then
		return `SharedTable(same={Value == Shared})`
	elseif Kind == "buffer" then
		return `buffer({buffer.len(Value)} {buffer.readu8(Value, 1)})`
	elseif Kind == "string" then
		return string.format("%q", Value)
	elseif Kind == "function" then
		return `function(called={Value()})`
	end
	return `{Kind}({tostring(Value)})`
end
Actor:BindToMessage("Arg", function(A)
	if type(A) == "table" then
		Add(`the older binding's table is its own: {A.Mutated == nil}`)
	end
end)
Actor:BindToMessage("Arg", function(...)
	local Parts = {}
	for Index = 1, select("#", ...) do
		Parts[Index] = Show((select(Index, ...)))
	end
	local A, B = ...
	local Notes = ""
	if type(A) == "table" and rawequal(A, B) then
		Notes ..= " (the same table twice)"
	end
	Add(`arg n={select("#", ...)} {table.concat(Parts, " | ")}{Notes}`)
	if type(A) == "table" then
		A.Mutated = true
	end
end)
for _, Topic in { "5", "{}", "true", "1, 2, 3" } do
	Actor:BindToMessage(Topic, function(Tag)
		Add(`topic "{Topic}" got {Tag}`)
	end)
end
Actor:BindToMessage("Fail", function()
	Add("Fail raising")
	error("a handler failed on purpose")
end)
Actor:BindToMessage("FailTable", function()
	error({ "a table" })
end)
Actor:BindToMessage("Yield", function(Tag)
	Add("Yield starts " .. Tag)
	task.wait()
	Add("Yield resumed " .. Tag)
end)
Actor:BindToMessage("Quick", function(Tag)
	Add("Quick " .. Tag)
end)
local Threads = {}
Actor:BindToMessage("Thread", function(Tag)
	local Thread = coroutine.running()
	local Seen = table.find(Threads, Thread)
	if not Seen then
		table.insert(Threads, Thread)
	end
	Add(`Thread {Tag} on thread #{Seen or #Threads}, debug.info(2) {debug.info(2, "s") or "nil"}, {coroutine.status(Thread)}`)
	if Tag == "yields" then
		task.wait()
	end
end)
Actor:BindToMessageParallel("ParallelBind", function()
	local Ok, Message = pcall(function()
		return Actor:BindToMessage("BoundInParallel", function(Tag) Add("BoundInParallel " .. Tag) end)
	end)
	Add(`a parallel handler binds: {Ok} {Message}`)
end)
local function Try(Label, F)
	local Ok, Message = pcall(F)
	Add(`{Label}: {Ok} {Message}`)
end
if Name == "A1" then
	Try("bind nil topic", function() return Actor:BindToMessage(nil, function() end) end)
	Try("bind number topic", function() return Actor:BindToMessage(7, function() end) end)
	Try("bind nil callback", function() return Actor:BindToMessage("t", nil) end)
	Try("bind number callback", function() return Actor:BindToMessage("t", 5) end)
	Try("bind table callback", function() return Actor:BindToMessage("t", {}) end)
	Try("bind dictionary callback", function() return Actor:BindToMessage("t", { a = 1 }) end)
	Try("bind string callback", function() return Actor:BindToMessage("t", "f") end)
	Try("bind nothing", function() return Actor:BindToMessage() end)
	Try("bindparallel nil topic", function() return Actor:BindToMessageParallel(nil, function() end) end)
	Try("bindparallel nil callback", function() return Actor:BindToMessageParallel("t", nil) end)
	local Other = Actor.Parent:FindFirstChild("A2")
	Try("bind on another Actor", function() return Other:BindToMessage("x", function() end) end)
	Try("bindparallel on another Actor", function() return Other:BindToMessageParallel("x", function() end) end)
	Try("bind on the Workspace", function() return workspace:BindToMessage("x", function() end) end)
	Try("bind dot call", function() return Actor.BindToMessage(workspace, "x", function() end) end)
	local Connection = Actor:BindToMessage("Connected", function() end)
	Add(`a binding: {typeof(Connection)} {Connection.Connected}`)
	Connection:Disconnect()
	Add(`disconnected: {Connection.Connected}`)
end
require(game:GetService("ReplicatedStorage"):WaitForChild("ParityOutsideBinder"))(Actor, Add)
if Actor.Parent:IsA("Actor") then
	Try("an inner Actor's script binds on the outer", function()
		return Actor.Parent:BindToMessage("FromInner", function() end)
	end)
end
local Inside = Actor:FindFirstChild("InsideBinder")
if Inside then
	require(Inside)(Actor, Add)
end
Add("ready")
]==]
Template.Parent = Folder
local InsideTemplate = Instance.new("ModuleScript")
InsideTemplate.Name = "InsideBinder"
InsideTemplate.Source = [==[
return function(Actor, Add)
	local Ok, Message = pcall(function()
		return Actor:BindToMessage("FromInside", function() Add("the inside module's binding ran") end)
	end)
	Add(`a module inside the Actor binds: {Ok} {Message}`)
end
]==]
LogService.MessageOut:Connect(function(Message, Kind)
	if string.find(Message, "on purpose") or string.find(Message, "no output from Luau") or string.find(Message, "expects a string") then
		Add(`log {Kind.Name} {string.gsub(Message, "\n.*", "")}`)
	end
end)
ScriptContext.Error:Connect(function(Message, Trace, Script)
	Add(`ScriptContext.Error {Message} | {string.gsub(Trace, "\n", "/")} | {if Script then Script:GetFullName() else "nil"}`)
end)
Folder.Parent = Home
local function MakeActor(Name, Parent)
	local Actor = Instance.new("Actor")
	Actor.Name = Name
	local Worker = Template:Clone()
	Worker.Parent = Actor
	Worker.Enabled = true
	Actor.Parent = Parent
	return Actor
end
local A1 = Instance.new("Actor")
A1.Name = "A1"
local Worker1 = Template:Clone()
Worker1.Parent = A1
Worker1.Enabled = true
InsideTemplate:Clone().Parent = A1
A1.Parent = Folder
local A2 = MakeActor("A2", Folder)
MakeActor("Inner", A1)
task.wait()
local function Try(Label, F)
	local Ok, Message = pcall(F)
	Add(`{Label}: {Ok} {P.err(Message)}`)
end
Try("this script requires the inside module", function()
	require(A1.InsideBinder)(A1, Add)
end)
A1:SendMessage("FromModule")
A1:SendMessage("FromInside")
task.wait()
task.wait()
local function Send(Label, ...)
	local Args = table.pack(...)
	local Ok, Message = pcall(function()
		return A1:SendMessage("Arg", table.unpack(Args, 1, Args.n))
	end)
	Add(`send {Label}: {Ok}{if Ok then "" else " " .. P.err(Message)}`)
end
Send("nils", nil, 1, nil)
Send("mixed", { 1, 2, Key = "v" })
Send("holes", { [1] = "a", [3] = "c" })
Send("nested", { x = { y = 1 } })
Send("metatable", setmetatable({ a = 1 }, { __index = { b = 2 } }))
Send("frozen", table.freeze({ 1 }))
local Twice = { 1 }
Send("twice", Twice, Twice)
local Cycle = {}
Cycle.Self = Cycle
Send("cycle", Cycle)
Send("function", function() return "yes" end)
Send("thread", coroutine.create(function() end))
Send("function in a table", { 1, F = function() end })
Send("instance", workspace)
Send("datatypes", Vector3.new(1, 2, 3), CFrame.new(1, 2, 3), Color3.new(1, 0, 0), Enum.Material.Plastic, UDim2.fromScale(1, 0))
local Buffer = buffer.create(4)
buffer.writeu8(Buffer, 1, 7)
Send("buffer", Buffer)
Send("sharedtable", Shared)
Send("keys", { [1.5] = "a", [workspace] = "w" })
Send("boolean key", { [true] = 1 })
Send("userdata", newproxy(true))
local Event = Instance.new("BindableEvent")
Event.Event:Connect(function(Value)
	Add(`a BindableEvent in this VM passes a userdata as {typeof(Value)}`)
end)
Event:Fire(newproxy(true))
local Listed = Instance.new("BindableEvent")
Listed.Event:Connect(function(Value)
	local Keys = {}
	for Key, Item in Value do
		table.insert(Keys, `{Key}={typeof(Item)}`)
	end
	table.sort(Keys)
	Add(`a BindableEvent passes userdata in a table as {table.concat(Keys, " ")}`)
end)
Listed:Fire({ 1, newproxy(true), 3 })
Listed:Fire({ A = newproxy(true), B = 2 })
local Later = { V = 1 }
Send("changed after the send", Later)
Later.V = 2
Send("many", 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20)
Send("a zero byte", "a\0b")
Send("vector", vector.create(1, 2, 3))
task.wait()
Add(`after delivery, the sender's table: Mutated {Later.Mutated} V {Later.V}`)
Try("send nothing", function() A1:SendMessage() end)
Try("send nil", function() A1:SendMessage(nil) end)
Try("send dot call", function() A1.SendMessage(workspace, "x") end)
Try("send's results", function() return select("#", A1:SendMessage("Quick", "results")) end)
Try("bind from this script", function() return A1:BindToMessage("x", function() end) end)
Try("bindparallel from this script", function() return A1:BindToMessageParallel("x", function() end) end)
Try("bind on a new Actor", function() return Instance.new("Actor"):BindToMessage("x", function() end) end)
Try("send to a new Actor", function() Instance.new("Actor"):SendMessage("x") end)
A1:SendMessage(5, "a number")
A1:SendMessage({}, "a table")
A1:SendMessage(true, "a boolean")
A1:SendMessage(Vector3.new(1, 2, 3), "a vector")
task.wait()
task.wait()
A1:SendMessage("Fail")
A1:SendMessage("Quick", "after the failing handler")
A1:SendMessage("FailTable")
task.wait()
task.wait()
A1:SendMessage("Yield", "y1")
A1:SendMessage("Quick", "q1")
A1:SendMessage("Yield", "y2")
task.wait()
task.wait()
A1:SendMessage("Thread", "a")
A1:SendMessage("Thread", "b")
task.wait()
A1:SendMessage("Thread", "yields")
A1:SendMessage("Thread", "c")
task.wait()
task.wait()
A1:SendMessage("Thread", "d")
task.wait()
A1:SendMessage("ParallelBind")
task.wait()
A1:SendMessage("BoundInParallel", "after")
task.wait()
task.wait()
-- the Actor leaves the DataModel between the send and the handler; a disabled script
A2:SendMessage("Quick", "sent before the Actor left")
A2.Parent = nil
Try("send to an Actor that left", function() A2:SendMessage("Quick", "x") end)
task.wait()
A2.Parent = Folder
task.wait()
A2:SendMessage("Quick", "back in the DataModel")
task.wait()
local Worker2 = A2:FindFirstChild("Worker")
Worker2.Enabled = false
A2:SendMessage("Quick", "to a disabled script")
task.wait()
Worker2.Enabled = true
task.wait()
A2:SendMessage("Quick", "enabled again")
task.wait()
task.wait()
for Index = 1, Shared.N do
	print(Plain(Shared["L" .. Index]))
end
