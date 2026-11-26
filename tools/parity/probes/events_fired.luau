-- Events ree's source never fires (a static audit of the API dump's events) that a script's own actions may fire in a
-- client or a server: CollectionService's tag events, the DataModel's ServiceAdded and ItemChanged, Lighting's
-- LightingChanged, a Sound's Played / Resumed / Paused / Stopped, a HumanoidDescription's emote events, an input
-- context's and action's change events, a ParticleEmitter's emit and clear requests, the collision-group events and
-- ScriptContext's ErrorDetailed: whether each may be connected to, and what it fires with after the action.
local RunService = game:GetService("RunService")
local Server = RunService:IsServer()
P.sync()
local function Show(...)
	local Parts = {}
	for Index = 1, select("#", ...) do
		local Value = select(Index, ...)
		local Kind = typeof(Value)
		if Kind == "Instance" then
			table.insert(Parts, Value.ClassName .. "(" .. Value.Name .. ")")
		elseif Kind == "number" then
			table.insert(Parts, P.num(Value))
		elseif Kind == "string" then
			table.insert(Parts, '"' .. P.text(Value) .. '"')
		elseif Kind == "nil" or Kind == "boolean" then
			table.insert(Parts, tostring(Value))
		elseif Kind == "table" then
			table.insert(Parts, "table")
		else
			table.insert(Parts, Kind)
		end
	end
	return table.concat(Parts, " ")
end
local Fired = {}
local Connections = {}
-- (ItemChanged fires for every change in the place: only the probe's own instances and the Workspace's Gravity are kept)
local Own = {}
local function Watch(Label, Holder, Event)
	local Ok, Result = pcall(function()
		return Holder[Event]:Connect(function(...)
			if Event == "ItemChanged" then
				local Item, Name = ...
				if not (Own[Item] or (Item == workspace and Name == "Gravity")) then
					return
				end
			end
			table.insert(Fired, Label .. "(" .. Show(...) .. ")")
		end)
	end)
	if Ok then
		table.insert(Connections, Result)
	end
	print("connect " .. Label .. " " .. (if Ok then "ok" else "E(" .. P.err(Result) .. ")"))
end
local function Step(Label, Action)
	table.clear(Fired)
	local Ok, Message = pcall(Action)
	for _ = 1, 3 do
		task.wait()
	end
	print("step " .. Label .. (if Ok then "" else " E(" .. P.err(Message) .. ")") .. " " .. table.concat(Fired, ","))
end

-- CollectionService
local CollectionService = game:GetService("CollectionService")
for _, Event in { "TagAdded", "TagRemoved", "ItemAdded", "ItemRemoved" } do
	Watch("CollectionService." .. Event, CollectionService, Event)
end
local Tagged = Instance.new("Folder")
Tagged.Name = "Tagged"
local Other = Instance.new("Folder")
Other.Name = "Other"
Own[Tagged] = true
Own[Other] = true
Step("tag-loose", function()
	Tagged:AddTag("ParityEventTag")
end)
Step("tag-parented", function()
	Tagged.Parent = workspace
end)
Step("tag-second-instance", function()
	Other:AddTag("ParityEventTag")
	Other.Parent = workspace
end)
Step("tag-remove-one", function()
	Tagged:RemoveTag("ParityEventTag")
end)
Step("tag-remove-last", function()
	Other:RemoveTag("ParityEventTag")
end)
Step("tag-new-in-world", function()
	Other:AddTag("ParityEventTag2")
end)
Step("tag-destroy", function()
	Other:Destroy()
end)

-- the DataModel
Watch("game.ServiceAdded", game, "ServiceAdded")
Watch("game.ServiceRemoving", game, "ServiceRemoving")
Watch("game.ItemChanged", game, "ItemChanged")
Step("services", function()
	for _, Name in { "PathfindingService", "ProximityPromptService", "Debris", "TweenService", "PhysicsService", "TextService", "HapticService" } do
		game:GetService(Name)
	end
end)
Step("item-changed", function()
	Tagged.Name = "Renamed"
	workspace.Gravity = 150
	workspace.Gravity = 196.2
end)

-- Lighting
local Lighting = game:GetService("Lighting")
Watch("Lighting.LightingChanged", Lighting, "LightingChanged")
Step("lighting-ambient", function()
	Lighting.Ambient = Color3.new(0.2, 0.3, 0.4)
end)
Step("lighting-clock", function()
	Lighting.ClockTime = 9
end)
Step("lighting-sky", function()
	local Sky = Instance.new("Sky")
	Sky.Parent = Lighting
end)
Step("lighting-brightness", function()
	Lighting.Brightness = 3
end)

-- a Sound
local Sound = Instance.new("Sound")
Sound.Name = "ParitySound"
Own[Sound] = true
Sound.Parent = workspace
for _, Event in { "Played", "Resumed", "Paused", "Stopped", "Ended", "Loaded", "DidLoop" } do
	Watch("Sound." .. Event, Sound, Event)
end
Step("sound-play", function()
	Sound:Play()
end)
Step("sound-pause", function()
	Sound:Pause()
end)
Step("sound-resume", function()
	Sound:Resume()
end)
Step("sound-stop", function()
	Sound:Stop()
end)
Step("sound-playing", function()
	Sound.Playing = true
end)
Step("sound-playing-off", function()
	Sound.Playing = false
end)

-- a HumanoidDescription
local Description = Instance.new("HumanoidDescription")
Watch("HumanoidDescription.EmotesChanged", Description, "EmotesChanged")
Watch("HumanoidDescription.EquippedEmotesChanged", Description, "EquippedEmotesChanged")
Step("emotes", function()
	Description:SetEmotes({ Salute = { 3360689775 } })
end)
Step("equipped-emotes", function()
	Description:SetEquippedEmotes({ "Salute" })
end)

-- input actions
local Context = Instance.new("InputContext")
local Action = Instance.new("InputAction")
Watch("InputContext.InputActionsChanged", Context, "InputActionsChanged")
Watch("InputAction.InputBindingsChanged", Action, "InputBindingsChanged")
Step("action-parent", function()
	Action.Parent = Context
end)
Step("binding-parent", function()
	local Binding = Instance.new("InputBinding")
	Binding.KeyCode = Enum.KeyCode.E
	Binding.Parent = Action
end)
Step("context-parent", function()
	Context.Parent = workspace
end)

-- a ParticleEmitter
local Holder = Instance.new("Part")
Holder.Anchored = true
Holder.Position = Vector3.new(600, 30, 600)
Holder.Parent = workspace
local Emitter = Instance.new("ParticleEmitter")
Emitter.Parent = Holder
Watch("ParticleEmitter.OnEmitRequested", Emitter, "OnEmitRequested")
Watch("ParticleEmitter.OnClearRequested", Emitter, "OnClearRequested")
Step("emit", function()
	Emitter:Emit(3)
end)
Step("clear", function()
	Emitter:Clear()
end)

-- collision groups
local PhysicsService = game:GetService("PhysicsService")
Watch("PhysicsService.CollisionGroupCollidableChanged", PhysicsService, "CollisionGroupCollidableChanged")
Watch("Workspace.CollisionGroupCollidableChanged", workspace, "CollisionGroupCollidableChanged")
Step("collidable", function()
	if Server then
		PhysicsService:RegisterCollisionGroup("ParityEventsA")
		PhysicsService:CollisionGroupSetCollidable("ParityEventsA", "Default", false)
	end
end)

-- ScriptContext
local ScriptContext = game:GetService("ScriptContext")
Watch("ScriptContext.ErrorDetailed", ScriptContext, "ErrorDetailed")
Step("script-error", function()
	task.spawn(function()
		error("parity event error", 0)
	end)
end)

for _, Connection in Connections do
	Connection:Disconnect()
end
Tagged:Destroy()
Sound:Destroy()
Holder:Destroy()
Context:Destroy()
