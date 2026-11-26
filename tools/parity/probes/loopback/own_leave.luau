-- parity: loopback
-- The server takes its client's player out (Parent = nil; src/services/players.rs, src/loopback/mod.rs cut): what that client
-- sees of its own leave (its character's removal, CharacterRemoving, Character nil, its PlayerGui destroyed, its PlayerRemoving
-- and AncestryChanged), what the server sent it in the leave's frame and after, and what it can do once cut off (FireServer,
-- an InvokeServer waiting and a new one, its frames). The server publishes its lines first: nothing reaches the client after
-- the leave. It sends every frame before the leave: a real server whose connection to the client was idle dropped the leave's
-- frame in about half the measured leaves, and none while it was busy.
local Players = game:GetService("Players")
local Storage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
--- server
Players.CharacterAutoLoads = false
local Me = P.player()
local Tick = Instance.new("RemoteEvent")
Tick.Name = "ProbeTick"
Tick.Parent = Storage
local Call = Instance.new("RemoteFunction")
Call.Name = "ProbeCall"
Call.OnServerInvoke = function(_, Wait)
	if Wait then
		task.wait(3)
	end
	return "answered"
end
Call.Parent = Storage
local Body = Instance.new("Model")
Body.Name = "Body"
local Part = Instance.new("Part")
Part.Anchored = true
Part.Position = Vector3.new(400, 4, 0)
Part.Parent = Body
Body.Parent = workspace
Me.Character = Body
P.meet("ready")
print("children " .. #Me:GetChildren() .. ", Character " .. tostring(Me.Character))
-- (the server's lines go to the client now, while it hears the server)
__LoopbackFinish(__LoopbackLines, __LoopbackRealPrint)
P.meet("published")
local Ticking = RunService.Heartbeat:Connect(function()
	if Me.Parent == Players then
		Tick:FireClient(Me, "tick")
	end
end)
for _ = 1, 30 do
	task.wait()
end
Tick:FireClient(Me, "before the call")
Me.Parent = nil
Tick:FireClient(Me, "after the call")
task.wait()
Tick:FireAllClients("the next frame")
task.wait(2)
Ticking:Disconnect()
--- client
local Me = Players.LocalPlayer
local Tick = Storage:WaitForChild("ProbeTick", 30)
local Call = Storage:WaitForChild("ProbeCall", 30)
local Order = {}
local Heard = {}
local Ticks = 0
local function Note(Text)
	table.insert(Order, Text)
end
Tick.OnClientEvent:Connect(function(Text)
	if Text == "tick" then
		Ticks += 1
	else
		table.insert(Heard, Text)
	end
end)
P.meet("ready")
local Body = workspace:WaitForChild("Body", 30)
local Gui = Me:FindFirstChildOfClass("PlayerGui")
Body.AncestryChanged:Connect(function(X, Parent)
	if X == Body then
		Note("body AncestryChanged " .. tostring(Parent))
	end
end)
Me.CharacterRemoving:Connect(function(X)
	Note("CharacterRemoving " .. (if X == Body then "body" else X.Name))
end)
Me:GetPropertyChangedSignal("Character"):Connect(function()
	Note("Character " .. tostring(Me.Character))
end)
if Gui then
	Gui.Destroying:Connect(function()
		Note("PlayerGui Destroying")
	end)
end
Players.PlayerRemoving:Connect(function(X)
	Note("PlayerRemoving " .. (if X == Me then "me" else "other"))
end)
Me.AncestryChanged:Connect(function(X, Parent)
	if X == Me then
		Note("me AncestryChanged " .. tostring(Parent))
	end
end)
Me.Changed:Connect(function(Property)
	if Property == "Parent" then
		Note("me Changed Parent")
	end
end)
P.meet("published")
local Pending = "waiting"
task.spawn(function()
	local Ok, Answer = pcall(Call.InvokeServer, Call, true)
	Pending = tostring(Ok) .. " " .. tostring(Answer)
end)
local Deadline = os.clock() + 20
while Me.Parent ~= nil and os.clock() < Deadline do
	task.wait()
end
task.wait(0.2)
print("own leave: " .. table.concat(Order, ", "))
print("heard after the ticks: " .. table.concat(Heard, ", ") .. "; ticks " .. tostring(Ticks > 20))
local Fired = pcall(Tick.FireServer, Tick, "after the cut")
local Later = "waiting"
task.spawn(function()
	local Ok, Answer = pcall(Call.InvokeServer, Call, false)
	Later = tostring(Ok) .. " " .. tostring(Answer)
end)
local Frames = 0
local Beat = RunService.Heartbeat:Connect(function()
	Frames += 1
end)
task.wait(3.5)
Beat:Disconnect()
print("after the cut: FireServer " .. tostring(Fired) .. ", InvokeServer waiting " .. Pending .. ", a new one " .. Later
	.. ", frames go on " .. tostring(Frames > 30) .. ", Parent " .. tostring(Me.Parent) .. ", Character " .. tostring(Me.Character)
	.. ", players " .. #Players:GetPlayers())
