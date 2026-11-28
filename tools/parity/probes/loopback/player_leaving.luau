-- parity: loopback players=4
-- Players leaving the server (src/services/players.rs), each its own way, its Character a Model the probe made or none: what
-- the server's scripts see (the events in the order they fire, the player's PlayerGui destroyed in the leave, what the player
-- and its character hold in the leaving thread and after, the player's Parent locked, the character destroyed a second after
-- the leave and the Player five, a kicked player staying about a second, an InvokeClient waiting on the player) and that the
-- client sees the players and characters gone. A game script cannot make a Player, so in the real engine the session's three
-- other clients (they run nothing) are the ones that leave; ree's loopback run has one client, so its server's harness lets the
-- three in itself (Context.Join; the real engine's job has no Context).
local Players = game:GetService("Players")
local Storage = game:GetService("ReplicatedStorage")
--- server
local Me = P.player()
local Leavers = {}
if Context then
	for Index = 1, 3 do
		table.insert(Leavers, Context.Join({ identity = { name = "Leaver" .. Index, userId = 100 + Index } }))
	end
else
	local Deadline = os.clock() + 120
	while #Players:GetPlayers() < 4 and os.clock() < Deadline do
		task.wait()
	end
	for _, Player in Players:GetPlayers() do
		if Player ~= Me then
			table.insert(Leavers, Player)
		end
	end
end
print("leavers " .. #Leavers)
local function Body(Index, Parent)
	local Model = Instance.new("Model")
	Model.Name = "Body" .. Index
	for Level = 1, 2 do
		local Part = Instance.new("Part")
		Part.Anchored = true
		Part.Position = Vector3.new(300 + Index * 20, Level * 4, 0)
		Part.Parent = Model
	end
	Model.Parent = Parent
	return Model
end
local Holder = Instance.new("Folder")
Holder.Name = "Holder"
Holder.Parent = Storage
local Call = Instance.new("RemoteFunction")
Call.Name = "ProbeCall"
Call.Parent = Storage

local function Where(Instance_)
	return if Instance_ == nil then "-" elseif Instance_.Parent == nil then "nil" else Instance_.Parent.Name
end
local function State(Player, Model)
	local Character = Player.Character
	return "Parent=" .. Where(Player) .. " Character=" .. (if Character == nil then "nil" elseif Character == Model then "body" else "other")
		.. " body.Parent=" .. Where(Model)
end
local function Locked(Player)
	local Name = Player.Name
	local Ok, Message = pcall(function()
		Player.Parent = Players
	end)
	if Ok then
		return "no"
	end
	local Start, Finish = string.find(Message, Name, 1, true)
	return if Start then string.sub(Message, 1, Start - 1) .. "<name>" .. string.sub(Message, Finish + 1) else Message
end

-- Each player gets its character (or none) just before it leaves: a real engine's own character leaves as it is replaced and
-- is destroyed a second on, which is waited out (and a character a script made, kept out of the Workspace a while, is
-- replaced by a new one of the engine's)
local function Leave(Label, Player, Index, Where, How)
	local Model = if Where then Body(Index, Where) else nil
	Player.Character = Model
	for _ = 1, 90 do
		task.wait()
	end
	local Order = {}
	local Connections = {}
	local function On(Signal, Handler)
		table.insert(Connections, Signal:Connect(Handler))
	end
	local function Note(Text)
		table.insert(Order, Text)
	end
	local Gui, Pack, Gear = Player:FindFirstChildOfClass("PlayerGui"), Player:FindFirstChildOfClass("Backpack"), Player:FindFirstChildOfClass("StarterGear")
	local function Tag(X)
		return if X == Player then "player" elseif Model ~= nil and X == Model then "body" elseif X == Gui then "gui" elseif X == Pack then "backpack"
			elseif X == Gear then "gear" else nil
	end
	for _, Ancestor in { { "player", Player }, { "Players", Players }, { "game", game }, { "Workspace", workspace }, { "ReplicatedStorage", Storage } } do
		On(Ancestor[2].DescendantRemoving, function(X)
			if Tag(X) then
				Note(Ancestor[1] .. ".DescendantRemoving " .. Tag(X))
			end
		end)
		On(Ancestor[2].ChildRemoved, function(X)
			if Tag(X) then
				Note(Ancestor[1] .. ".ChildRemoved " .. Tag(X))
			end
		end)
	end
	if Gui then
		On(Gui.Destroying, function()
			Note("gui Destroying")
		end)
	end
	On(Player.CharacterRemoving, function(X)
		Note("CharacterRemoving " .. (Tag(X) or "other"))
	end)
	On(Player:GetPropertyChangedSignal("Character"), function()
		Note("Character " .. (if Player.Character == nil then "nil" else "set"))
	end)
	On(Player.AncestryChanged, function(X, Parent)
		if X == Player then
			Note("player AncestryChanged " .. tostring(Parent))
		end
	end)
	On(Player.Changed, function(Property)
		if Property == "Parent" then
			Note("player Changed Parent")
		end
	end)
	On(Players.PlayerRemoving, function(X)
		if X == Player then
			Note("PlayerRemoving player")
		end
	end)
	if Model then
		On(Model.AncestryChanged, function(X, Parent)
			if X == Model then
				Note("body AncestryChanged " .. tostring(Parent))
			end
		end)
	end
	local Destroyed = {}
	On(Player.Destroying, function()
		Note("player Destroying")
		Destroyed.Player = time()
	end)
	if Model then
		On(Model.Destroying, function()
			Destroyed.Body = time()
		end)
	end
	local Invoked
	task.spawn(function()
		local Ok, Message = pcall(Call.InvokeClient, Call, Player)
		local Text, Named = tostring(Message), "Player " .. Player.Name .. " "
		local Start, Finish = string.find(Text, Named, 1, true)
		Invoked = tostring(Ok) .. " " .. (if Start then string.sub(Text, 1, Start - 1) .. "Player <name> " .. string.sub(Text, Finish + 1) else Text)
	end)
	task.wait()
	local Called = time()
	if How == "kick" then
		Player:Kick("bye")
	elseif How == "destroy" then
		Player:Destroy()
	else
		Player.Parent = nil
	end
	print(Label .. " " .. How .. ", in the leaving thread: " .. State(Player, Model))
	local Waited = 0
	while Player.Parent ~= nil and Waited < 300 do
		task.wait()
		Waited += 1
		if Waited == 10 then
			-- (a kicked player stays as it was, and a second Kick changes nothing)
			local Listed = table.find(Players:GetPlayers(), Player) ~= nil
			print(Label .. " ten frames on: " .. State(Player, Model) .. " listed=" .. tostring(Listed) .. " again=" .. tostring((pcall(Player.Kick, Player, "again"))))
		end
	end
	-- (the leave's frame; a kicked player leaves at the frame after the first that starts more than a second after the call: 61
	-- or 62 frames at 60 Hz in a real session, a frame landing on the second's edge, 62 in ree; in time, as a busy machine's
	-- frames run slower)
	local Left = time()
	if How == "kick" then
		print(Label .. " left about a second after the call: " .. tostring(Left - Called > 1 and Left - Called < 1.12))
	end
	task.wait()
	print(Label .. " order: " .. table.concat(Order, ", "))
	print(Label .. " after: " .. State(Player, Model))
	print(Label .. " locked: " .. Locked(Player))
	for _ = 1, 2 do
		task.wait(3)
	end
	local function Since(At)
		return if At then string.format("%.1f", At - Left) else "no"
	end
	print(Label .. " InvokeClient waiting at the leave: " .. tostring(Invoked))
	print(Label .. " destroyed: body " .. (if Model then Since(Destroyed.Body) else "-") .. ", player " .. (if How == "destroy" then "in the call" else Since(Destroyed.Player)))
	for _, Connection in Connections do
		Connection:Disconnect()
	end
end

Leave("no character", Leavers[1], 1, nil, "unparent")
Leave("character in the Workspace", Leavers[2], 2, workspace, "kick")
Leave("character in ReplicatedStorage", Leavers[3], 3, Holder, "destroy")
P.meet("left")
P.meet("seen")
--- client
P.meet("left")
local Others = 0
for _, Player in Players:GetPlayers() do
	if Player ~= Players.LocalPlayer then
		Others += 1
	end
end
local Bodies = 0
for _, Place in { workspace, Storage:FindFirstChild("Holder") } do
	for _, Child in Place:GetChildren() do
		if string.sub(Child.Name, 1, 4) == "Body" then
			Bodies += 1
		end
	end
end
print("other players " .. Others .. ", bodies " .. Bodies)
P.meet("seen")
