-- parity: identity=4
-- Actor messages against the frame: where SendMessage's handlers run (serial bindings at the deferred flushes, one deferred step
-- after the send; parallel ones at the frame events' parallel points, after the event's flush and with its ConnectParallel
-- handlers), what task.desynchronize does in an Actor's serial code, the event re-entrancy cut, when a topic's bindings are taken.
-- A client lets a job write Source at identity 4; the workers are LocalScripts in the player's PlayerScripts. Lines are logged through a
-- SharedTable (no deferred step in between), each with the frames since its test began. Only one Actor logs in each parallel
-- round: a client runs Actors at the same time, in an order of its own.
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
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
local Warn = getfenv().warn
local Shared = SharedTable.new()
Registry:SetSharedTable("ParityActorMessages", Shared)
Shared.N = 0
Shared.F = 0
Shared.F0 = 0
Shared.Phase = ""
Shared.Mode = ""
Shared.Count = 0
local function Add(Text)
	local Index = SharedTable.increment(Shared, "N", 1) + 1
	Shared["L" .. Index] = `{Shared.F - Shared.F0}:{Text}`
end
local function Begin(Mode)
	Shared.F0 = Shared.F
	Shared.Mode = Mode
	Add("-- " .. Mode)
end
LogService.MessageOut:Connect(function(Message)
	if string.find(Message, "^PARITY") or string.find(Message, "re%-entrancy") then
		Add("log " .. Message)
	end
end)
local Folder = Instance.new("Folder")
Folder.Name = "ParityActorMessages"
local Template = Instance.new("LocalScript")
Template.Name = "Worker"
Template.Enabled = false
Template.Source = [==[
local RunService = game:GetService("RunService")
local Shared = game:GetService("SharedTableRegistry"):GetSharedTable("ParityActorMessages")
local Actor = script:GetActor()
local Name = Actor.Name
local function Add(Text)
	local Index = SharedTable.increment(Shared, "N", 1) + 1
	Shared["L" .. Index] = `{Shared.F - Shared.F0}:{Name} {Text}`
end
local function Other()
	return Actor.Parent:FindFirstChild(if Name == "A1" then "A2" else "A1")
end
Actor:BindToMessage("S", function(Tag)
	Add("S " .. Tag)
end)
Actor:BindToMessage("SDefer", function(Tag)
	Add("SDefer " .. Tag)
	task.defer(function()
		Add("SDefer's defer " .. Tag)
	end)
end)
Actor:BindToMessageParallel("P", function(Tag)
	Add(`P {Tag} at {Shared.Phase}`)
	task.synchronize()
	Add("P synchronized " .. Tag)
	task.desynchronize()
	Add("P desynchronized " .. Tag)
end)
Actor:BindToMessage("Chain", function()
	Add("Chain, sending on")
	Other():SendMessage("S", "chained from " .. Name)
	Actor:SendMessage("S", "chained to itself")
end)
Actor:BindToMessage("SendP", function(Tag)
	Add("SendP " .. Tag)
	Other():SendMessage("P", "from a serial handler")
	task.defer(function()
		Add("SendP's defer")
	end)
end)
Actor:BindToMessageParallel("PSend", function()
	Add("PSend parallel")
	Other():SendMessage("S", "sent desynchronized")
	Other():SendMessage("P", "sent desynchronized")
	task.synchronize()
	Add("PSend synchronized")
	Other():SendMessage("S", "sent synchronized")
	task.defer(function()
		Add("PSend's defer")
	end)
end)
Actor:BindToMessage("BindLate", function()
	Add("binding Late")
	Actor:BindToMessage("Late", function(Tag)
		Add("Late " .. Tag)
	end)
end)
local First = Actor:BindToMessage("Twice", function(Tag)
	Add("Twice first binding " .. Tag)
end)
local Second = Actor:BindToMessage("Twice", function(Tag)
	Add("Twice second binding " .. Tag)
end)
Actor:BindToMessage("DropSecond", function()
	Second:Disconnect()
	Add(`dropped the second: {First.Connected} {Second.Connected}`)
end)
Actor:BindToMessage("Both", function(Tag)
	Add("Both serial " .. Tag)
end)
Actor:BindToMessageParallel("Both", function(Tag)
	Add("Both parallel " .. Tag)
end)
Actor:BindToMessage("Both", function(Tag)
	Add("Both second serial " .. Tag)
end)
Actor:BindToMessage("Desync", function(Tag)
	Add("Desync " .. Tag)
	task.desynchronize()
	Add(`Desync desynchronized {Tag} at {Shared.Phase}`)
	task.synchronize()
	Add("Desync synchronized again " .. Tag)
end)
Actor:BindToMessage("Ping", function(Left)
	SharedTable.increment(Shared, "Count", 1)
	if Left > 0 then
		Actor:SendMessage("Ping", Left - 1)
	end
end)
Actor:BindToMessage("PingDefer", function(Left)
	SharedTable.increment(Shared, "Count", 1)
	if Left > 0 then
		task.defer(function()
			Actor:SendMessage("PingDefer", Left - 1)
		end)
	end
end)
Actor:BindToMessageParallel("PingP", function(Left)
	SharedTable.increment(Shared, "Count", 1)
	if Left > 0 then
		Actor:SendMessage("PingP", Left - 1)
	end
end)
Actor:BindToMessageParallel("PingSync", function(Left)
	SharedTable.increment(Shared, "Count", 1)
	task.synchronize()
	if Left > 0 then
		Actor:SendMessage("PingSync", Left - 1)
	end
end)
if Name == "A1" then
	RunService.Heartbeat:ConnectParallel(function()
		local Mode = Shared.Mode
		if Mode == "heartbeat" or Mode == "parallel sends" then
			Add("Heartbeat parallel handler")
			if Mode == "parallel sends" then
				Other():SendMessage("S", "from a parallel handler")
				Other():SendMessage("P", "from a parallel handler")
			end
			task.synchronize()
			Add("Heartbeat parallel handler synchronized")
		end
	end)
	RunService.Heartbeat:Connect(function()
		local Mode = Shared.Mode
		if Mode == "serial desynchronize" then
			Shared.Mode = "serial desynchronize, done"
			Add("serial Heartbeat handler desynchronizing")
			task.desynchronize()
			Add(`serial Heartbeat handler desynchronized at {Shared.Phase}`)
		elseif Mode == "lookup" then
			Shared.Mode = "lookup, done"
			task.defer(function()
				Actor:BindToMessage("Fresh", function(Tag)
					Add("Fresh " .. Tag)
				end)
				Add("bound Fresh in a deferred job")
			end)
		end
	end)
end
if Name == "D" then
	Add("top level desynchronizing")
	task.desynchronize()
	Add(`top level desynchronized at {Shared.Phase}`)
	task.synchronize()
	Add("top level synchronized")
end
]==]
Template.Parent = Folder
for _, Phase in { "PreRender", "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	-- (frames are counted at PreAnimation and PreRender is not logged: a headless client's render steps are not one a frame)
	RunService[Phase]:Connect(function()
		if Phase == "PreAnimation" then
			Shared.F += 1
		end
		Shared.Phase = Phase
		if Shared.Mode ~= "" and Shared.Mode ~= "quiet" and Phase ~= "PreRender" then
			Add("main " .. Phase)
		end
	end)
end
local SendFresh = false
local FreshTo
RunService.Heartbeat:Connect(function()
	if SendFresh then
		SendFresh = false
		Add("main Heartbeat handler sends Fresh")
		FreshTo:SendMessage("Fresh", "sent after the bind was deferred")
	end
end)
Folder.Parent = Home
local function MakeActor(Name)
	local Actor = Instance.new("Actor")
	Actor.Name = Name
	local Worker = Template:Clone()
	Worker.Parent = Actor
	Worker.Enabled = true
	Actor.Parent = Folder
	return Actor
end
local A1, A2 = MakeActor("A1"), MakeActor("A2")
FreshTo = A1
task.wait()
task.wait()
local function Settle()
	task.wait()
	task.wait()
	Shared.Mode = "quiet"
	task.wait()
	task.wait()
end

Begin("at a resumption")
A1:SendMessage("SDefer", "a")
A1:SendMessage("S", "b")
task.defer(function()
	Add("a defer there")
	task.defer(function()
		Add("what that defer deferred")
	end)
end)
Settle()

Begin("heartbeat")
A1:SendMessage("SDefer", "at the resumption")
local Connection
Connection = RunService.Heartbeat:Connect(function()
	Connection:Disconnect()
	Add("main Heartbeat handler sends and defers")
	A1:SendMessage("S", "from the Heartbeat handler")
	task.defer(function()
		Add("the Heartbeat handler's defer")
	end)
	Warn("PARITY from the Heartbeat handler")
end)
Warn("PARITY at the resumption")
Settle()

Begin("events")
local Pre
Pre = RunService.PreRender:Connect(function()
	Pre:Disconnect()
	Add("PreRender handler sends")
	A1:SendMessage("S", "from PreRender")
	A1:SendMessage("P", "from PreRender")
	task.defer(function()
		Add("PreRender's defer sends")
		A2:SendMessage("S", "from PreRender's defer")
	end)
end)
local Animation
Animation = RunService.PreAnimation:Connect(function()
	Animation:Disconnect()
	A2:SendMessage("P", "from PreAnimation")
end)
local Simulation
Simulation = RunService.PreSimulation:Connect(function()
	Simulation:Disconnect()
	A1:SendMessage("S", "from PreSimulation")
	A1:SendMessage("P", "from PreSimulation")
end)
local Post
Post = RunService.PostSimulation:Connect(function()
	Post:Disconnect()
	A2:SendMessage("P", "from PostSimulation")
end)
Settle()

Begin("chain")
A1:SendMessage("Chain")
Settle()

Begin("serial sends a parallel message")
A1:SendMessage("SendP", "x")
Settle()

Begin("parallel sends")
Settle()

Begin("parallel handler sends")
A1:SendMessage("PSend")
Settle()

Begin("unbound topics")
A1:SendMessage("Late", "before the bind")
A1:SendMessage("Nobody", 1)
task.wait()
A1:SendMessage("BindLate")
A1:SendMessage("Late", "sent with the bind")
task.wait()
A1:SendMessage("Late", "after the bind")
Settle()

Begin("two bindings")
A1:SendMessage("Twice", "1")
task.wait()
A1:SendMessage("DropSecond")
A1:SendMessage("Twice", "2")
task.wait()
A1:SendMessage("Twice", "3")
Settle()

Begin("serial and parallel bindings")
A2:SendMessage("Both", "b")
Settle()

Begin("top level desynchronize")
MakeActor("D")
Add("placed D")
Settle()

Begin("message handler desynchronize")
A1:SendMessage("Desync", "d")
Settle()

Begin("serial desynchronize")
Settle()

Begin("lookup")
SendFresh = true
Settle()

local function Count(Label, Send, Frames)
	Shared.Mode = "quiet"
	Shared.Count = 0
	Send()
	local Counts = {}
	for _ = 1, Frames do
		task.wait()
		table.insert(Counts, tostring(Shared.Count))
	end
	Add(`{Label}: {table.concat(Counts, " ")}`)
end
Begin("cuts")
Count("a chain of 100 messages, after each frame", function() A1:SendMessage("Ping", 99) end, 2)
Count("a chain of 100 messages and defers", function() A1:SendMessage("PingDefer", 99) end, 2)
Count("a chain of 100 parallel messages", function() A2:SendMessage("PingP", 99) end, 2)
Count("a chain of 12 sent synchronized", function() A2:SendMessage("PingSync", 11) end, 2)
task.wait()
Shared.Mode = ""
for Index = 1, Shared.N do
	print(Plain(Shared["L" .. Index]))
end
