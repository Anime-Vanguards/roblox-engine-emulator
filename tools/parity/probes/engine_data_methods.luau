-- Engine methods with offline answers (parity package U): what each answers on a playing client and a server, its argument
-- casts and error texts, and the state it keeps. ContentProvider's keys and fetch statuses, CollectionService's GetCollection(s),
-- ContextActionService's tool icon and BindActivate, LogService's loggers, Stats' memory tags, Teams, ReplicatedFirst,
-- Lighting:GetMoonPhase, SoundService's listener, MaterialService's overrides and variants, Tool:Activate, TeleportOptions,
-- DataStore options, WireframeHandleAdornment, Beam, GeneratedFolder, Dialog, CustomEvent, Accessory and Makeup descriptions,
-- HumanoidDescription accessories, Model.PrimaryPart once its part leaves, GuiService's navigation defaults, RunService's step
-- bindings' casts.
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
			if type(A) == "number" and type(B) == "number" then
				return A < B
			end
			return tostring(A) < tostring(B)
		end)
		local Parts = {}
		for _, Key in Keys do
			table.insert(Parts, `{tostring(Key)}={Show(Value[Key], Depth + 1)}`)
		end
		return "{" .. table.concat(Parts, ", ") .. "}"
	elseif Kind == "Instance" then
		return `{Value.ClassName}({Value.Name})`
	elseif Kind == "string" then
		return string.format("%q", Value)
	elseif Kind == "number" then
		return P.num(Value)
	elseif Kind == "function" then
		return "function"
	end
	return `{Kind}({tostring(Value)})`
end
local function Try(Label, F, ...)
	local Results = table.pack(pcall(F, ...))
	local Parts = {}
	for Index = 2, Results.n do
		table.insert(Parts, if Results[1] then Show(Results[Index]) else P.err(Results[Index]))
	end
	print(`{Label}: {if Results[1] then "ok" else "error"} {table.concat(Parts, " | ")}`)
	return Results[1], Results[2]
end
local function M(Object, Name, ...)
	local Args = table.pack(...)
	return function()
		return Object[Name](Object, table.unpack(Args, 1, Args.n))
	end
end
local Players = game:GetService("Players")
local Player = Players.LocalPlayer

-- ContentProvider
local CP = game:GetService("ContentProvider")
Try("ListEncryptedAssets", M(CP, "ListEncryptedAssets"))
Try("UnregisterDefaultEncryptionKey", M(CP, "UnregisterDefaultEncryptionKey"))
for _, Name in { "Preload", "RegisterDefaultEncryptionKey", "RegisterDefaultSessionKey", "UnregisterEncryptedAsset" } do
	Try(Name .. "()", M(CP, Name))
	Try(Name .. "(id)", M(CP, Name, "rbxassetid://123"))
	Try(Name .. "(5)", M(CP, Name, 5))
end
for _, Name in { "RegisterEncryptedAsset", "RegisterSessionEncryptedAsset" } do
	Try(Name .. "()", M(CP, Name))
	Try(Name .. "(id, key)", M(CP, Name, "rbxassetid://123", "key"))
end
Try("ListEncryptedAssets after registering", M(CP, "ListEncryptedAssets"))
for _, Id in { "rbxassetid://123456", "", "rbxasset://textures/face.png" } do
	Try(`GetAssetFetchStatus({Id})`, M(CP, "GetAssetFetchStatus", Id))
end
for _, Id in { "rbxassetid://123456", "rbxassetid://5", "rbxasset://textures/face.png", "https://example.com/x.png", "", "abc", "rbxthumb://type=Asset&id=1&w=150&h=150" } do
	Try(`GetAssetFetchStatusChangedSignal({Id})`, function()
		local Signal = CP:GetAssetFetchStatusChangedSignal(Id)
		return typeof(Signal), tostring(Signal), Signal == CP:GetAssetFetchStatusChangedSignal(Id)
	end)
end

-- CollectionService
local CS = game:GetService("CollectionService")
local Tagged = Instance.new("Part")
Tagged:AddTag("ParityTag")
Tagged.Parent = workspace
Try("GetCollection()", M(CS, "GetCollection"))
Try("GetCollection(Part)", M(CS, "GetCollection", "Part"))
Try("GetCollection(tag)", M(CS, "GetCollection", "ParityTag"))
Try("GetCollections()", M(CS, "GetCollections"))
Try("GetCollections(part)", M(CS, "GetCollections", Tagged))
Try("GetCollections(5)", M(CS, "GetCollections", 5))

-- ContextActionService
local CAS = game:GetService("ContextActionService")
Try("GetCurrentLocalToolIcon", M(CAS, "GetCurrentLocalToolIcon"))
Try("BindActivate()", M(CAS, "BindActivate"))
Try("BindActivate(5)", M(CAS, "BindActivate", 5))
Try("BindActionToInputTypes()", M(CAS, "BindActionToInputTypes"))
Try("BindActionToInputTypes(name, 5, false)", M(CAS, "BindActionToInputTypes", "ParityAction", 5, false))
if not Player then
	Try("BindActivate(Gamepad1) on a server", M(CAS, "BindActivate", Enum.UserInputType.Gamepad1))
	Try("UnbindActivate(Gamepad1) on a server", M(CAS, "UnbindActivate", Enum.UserInputType.Gamepad1))
	Try("BindActionToInputTypes on a server", M(CAS, "BindActionToInputTypes", "ParityAction", function() end, false, Enum.KeyCode.E))
end

-- LogService
local LogService = game:GetService("LogService")
local Logged = {}
LogService.MessageOut:Connect(function(Message, Kind)
	if string.find(Message, "^PARITYLOG") then
		table.insert(Logged, `{Kind.Name} {Message}`)
	end
end)
Try("ClearOutput", M(LogService, "ClearOutput"))
Try("GetLogger()", M(LogService, "GetLogger"))
Try("GetLogHistoryAsync()", M(LogService, "GetLogHistoryAsync"))
if Player then
	Try("GetLogHistoryAsync(player)", M(LogService, "GetLogHistoryAsync", Player))
end
local Logger = LogService:GetLogger("ParityLogger")
Try("logger", function()
	return typeof(Logger), tostring(Logger), Logger.Name, rawequal(Logger, LogService:GetLogger("ParityLogger")), typeof(Logger.Info)
end)
Try("logger missing member", function() return Logger.Debug end)
Try("Logger:Info", M(Logger, "Info", "PARITYLOG info"))
Try("Logger:Warn", M(Logger, "Warn", "PARITYLOG warn"))
Try("Logger:Error", M(Logger, "Error", "PARITYLOG error"))
Try("Logger:Log()", M(Logger, "Log"))
Try("Logger:Log(text)", M(Logger, "Log", "PARITYLOG log"))
Try("Logger:Log(warning, text)", M(Logger, "Log", Enum.MessageType.MessageWarning, "PARITYLOG log warning"))
Try("Logger:Info()", M(Logger, "Info"))
Try("Logger:Info(a, b)", M(Logger, "Info", "PARITYLOG a", "b"))

-- Stats
local Stats = game:GetService("Stats")
local Tags = {}
for _, Item in Enum.DeveloperMemoryTag:GetEnumItems() do
	table.insert(Tags, `{Item.Name}={Stats:GetMemoryUsageMbForTag(Item)}`)
end
print("memory tags " .. table.concat(Tags, " "))
Try("GetMemoryUsageMbForTag()", M(Stats, "GetMemoryUsageMbForTag"))
Try("GetMemoryUsageMbForTag(0)", M(Stats, "GetMemoryUsageMbForTag", 0))
Try("GetMemoryUsageMbForTag(x)", M(Stats, "GetMemoryUsageMbForTag", "x"))
Try("GetTotalMemoryUsageMb", function() return typeof(Stats:GetTotalMemoryUsageMb()) end)

-- Teams, ReplicatedFirst, Lighting
Try("RebalanceTeams", M(game:GetService("Teams"), "RebalanceTeams"))
Try("RemoveDefaultLoadingScreen", M(game:GetService("ReplicatedFirst"), "RemoveDefaultLoadingScreen"))
local Lighting = game:GetService("Lighting")
for _, Time in { 0, 12 } do
	Lighting.ClockTime = Time
	Try(`GetMoonPhase at {Time}`, M(Lighting, "GetMoonPhase"))
end
Lighting.ClockTime = 14

-- SoundService
local SS = game:GetService("SoundService")
Try("GetListener", M(SS, "GetListener"))
Try("SetListener()", M(SS, "SetListener"))
Try("SetListener(x)", M(SS, "SetListener", "x"))
Try("SetListener(CFrame)", M(SS, "SetListener", Enum.ListenerType.CFrame, CFrame.new(1, 2, 3)))
Try("GetListener after CFrame", M(SS, "GetListener"))
Try("SetListener(ObjectPosition, part)", M(SS, "SetListener", Enum.ListenerType.ObjectPosition, Tagged))
Try("GetListener after ObjectPosition", M(SS, "GetListener"))
Try("SetListener(CFrame, nil)", M(SS, "SetListener", Enum.ListenerType.CFrame))
Try("SetListener(CFrame, part)", M(SS, "SetListener", Enum.ListenerType.CFrame, Tagged))
Try("SetListener(Camera)", M(SS, "SetListener", Enum.ListenerType.Camera))
Try("GetListener after Camera", M(SS, "GetListener"))

-- MaterialService
local MS = game:GetService("MaterialService")
local Variant = Instance.new("MaterialVariant")
Variant.Name = "ParityShiny"
Variant.Parent = MS
local Deep = Instance.new("Folder")
Deep.Parent = MS
local Twin = Instance.new("MaterialVariant")
Twin.Name = "ParityShiny"
Twin.Parent = Deep
local Far = Instance.new("MaterialVariant")
Far.Name = "ParityFar"
Far.Parent = workspace
Try("GetMaterialVariant(Plastic, ParityShiny)", function()
	local Found = MS:GetMaterialVariant(Enum.Material.Plastic, "ParityShiny")
	return Found, Found == Variant, Found == Twin
end)
Try("GetMaterialVariant(Wood, ParityShiny)", M(MS, "GetMaterialVariant", Enum.Material.Wood, "ParityShiny"))
Try("GetMaterialVariant(Plastic, ParityFar)", M(MS, "GetMaterialVariant", Enum.Material.Plastic, "ParityFar"))
Try("GetMaterialVariant(Plastic)", M(MS, "GetMaterialVariant", Enum.Material.Plastic))
Try("GetMaterialVariant(x, name)", M(MS, "GetMaterialVariant", "x", "ParityShiny"))
Try("GetMaterialVariant(256, name)", M(MS, "GetMaterialVariant", 256, "ParityShiny"))
Try("GetBaseMaterialOverride()", M(MS, "GetBaseMaterialOverride"))
Try("SetBaseMaterialOverride(Plastic, ParityShiny)", M(MS, "SetBaseMaterialOverride", Enum.Material.Plastic, "ParityShiny"))
Try("SetBaseMaterialOverride(Plastic)", M(MS, "SetBaseMaterialOverride", Enum.Material.Plastic))
Try("SetBaseMaterialOverride(Wood, 5)", M(MS, "SetBaseMaterialOverride", Enum.Material.Wood, 5))
Try("SetBaseMaterialOverride(Air, x)", M(MS, "SetBaseMaterialOverride", Enum.Material.Air, "x"))
local Overrides = {}
for _, Item in Enum.Material:GetEnumItems() do
	local Ok, Name = pcall(MS.GetBaseMaterialOverride, MS, Item)
	table.insert(Overrides, `{Item.Name}={if Ok then Name else P.err(Name)}`)
end
print("overrides " .. table.concat(Overrides, " "))
MS:SetBaseMaterialOverride(Enum.Material.Plastic, "Plastic")
MS:SetBaseMaterialOverride(Enum.Material.Wood, "Wood")

-- Tool:Activate / Deactivate
local Tool = Instance.new("Tool")
local Handle = Instance.new("Part")
Handle.Name = "Handle"
Handle.Parent = Tool
local ToolEvents = {}
for _, Name in { "Activated", "Deactivated" } do
	Tool[Name]:Connect(function() table.insert(ToolEvents, Name) end)
end
local function ToolCase(Label)
	Tool:Activate()
	Tool:Deactivate()
	task.wait()
	print(`tool {Label}: {table.concat(ToolEvents, ",")}`)
	table.clear(ToolEvents)
end
ToolCase("unparented")
Tool.Parent = workspace
ToolCase("in the Workspace")
local Rig = Instance.new("Model")
Instance.new("Humanoid").Parent = Rig
Rig.Parent = workspace
Tool.Parent = Rig
ToolCase("in a model with a Humanoid")
Tool.Enabled = false
ToolCase("disabled there")
Tool.Enabled = true
Tool.Parent = nil

-- TeleportOptions
local Options = Instance.new("TeleportOptions")
Try("GetTeleportData unset", M(Options, "GetTeleportData"))
Try("SetTeleportData(table)", M(Options, "SetTeleportData", { A = 1, B = { 2 } }))
Try("GetTeleportData table", function()
	local Data = Options:GetTeleportData()
	return Data, rawequal(Data, Options:GetTeleportData())
end)
Try("SetTeleportData(nil)", M(Options, "SetTeleportData", nil))
Try("SetTeleportData(mixed)", M(Options, "SetTeleportData", { 1, 2, K = 3 }))
Try("GetTeleportData mixed", M(Options, "GetTeleportData"))
Try("SetTeleportData(5)", M(Options, "SetTeleportData", 5))
Try("GetTeleportData 5", M(Options, "GetTeleportData"))
Try("SetTeleportData(function)", M(Options, "SetTeleportData", function() end))
Try("GetTeleportData function", M(Options, "GetTeleportData"))
local Cycle = {}
Cycle.C = Cycle
Try("SetTeleportData(cycle)", M(Options, "SetTeleportData", Cycle))

-- DataStore options
for _, Class in { "DataStoreIncrementOptions", "DataStoreSetOptions" } do
	local Object = Instance.new(Class)
	Try(Class .. " GetMetadata unset", M(Object, "GetMetadata"))
	Try(Class .. " SetMetadata(t)", M(Object, "SetMetadata", { A = 1, B = "x" }))
	Try(Class .. " GetMetadata", M(Object, "GetMetadata"))
	Try(Class .. " SetMetadata(5)", M(Object, "SetMetadata", 5))
	Try(Class .. " SetMetadata()", M(Object, "SetMetadata"))
	Try(Class .. " SetMetadata(array)", M(Object, "SetMetadata", { 1, 2 }))
end
local StoreOptions = Instance.new("DataStoreOptions")
Try("SetExperimentalFeatures({})", M(StoreOptions, "SetExperimentalFeatures", {}))
Try("SetExperimentalFeatures(t)", M(StoreOptions, "SetExperimentalFeatures", { v2 = true }))
Try("SetExperimentalFeatures(5)", M(StoreOptions, "SetExperimentalFeatures", 5))

-- WireframeHandleAdornment, Beam, GeneratedFolder, Dialog
local Wire = Instance.new("WireframeHandleAdornment")
Try("AddLine()", M(Wire, "AddLine"))
Try("AddLine(a, b)", M(Wire, "AddLine", Vector3.zero, Vector3.one))
Try("AddLine(a, 5)", M(Wire, "AddLine", Vector3.zero, 5))
Try("AddLines({a, b, c})", M(Wire, "AddLines", { Vector3.zero, Vector3.one, Vector3.one }))
Try("AddLines({5})", M(Wire, "AddLines", { 5 }))
Try("AddLines(5)", M(Wire, "AddLines", 5))
Try("AddPath({a, b}, true)", M(Wire, "AddPath", { Vector3.zero, Vector3.one }, true))
Try("AddPath({a, b})", M(Wire, "AddPath", { Vector3.zero, Vector3.one }))
Try("AddText(a, t, 20)", M(Wire, "AddText", Vector3.zero, "hi", 20))
Try("AddText(a)", M(Wire, "AddText", Vector3.zero))
Try("AddText(a, 5)", M(Wire, "AddText", Vector3.zero, 5))
Try("Clear", M(Wire, "Clear"))
local Beam = Instance.new("Beam")
Try("SetTextureOffset()", M(Beam, "SetTextureOffset"))
Try("SetTextureOffset(0.5)", M(Beam, "SetTextureOffset", 0.5))
Try("SetTextureOffset(x)", M(Beam, "SetTextureOffset", "x"))
local Generated = Instance.new("GeneratedFolder")
Try("GeneratedFolder", function() return Generated.ClassName, Generated.Name end)
Try("SetPrimaryPart()", M(Generated, "SetPrimaryPart"))
Try("SetPrimaryPart(part)", M(Generated, "SetPrimaryPart", Instance.new("Part")))
Try("GetCurrentPlayers", M(Instance.new("Dialog"), "GetCurrentPlayers"))

-- CustomEvent and CustomEventReceiver
local Event = Instance.new("CustomEvent")
local Receiver = Instance.new("CustomEventReceiver")
Try("receiver value unattached", M(Receiver, "GetCurrentValue"))
Try("receivers none", M(Event, "GetAttachedReceivers"))
local Changes = {}
Receiver.SourceValueChanged:Connect(function(Value) table.insert(Changes, P.num(Value)) end)
Receiver.Source = Event
Try("receivers one", M(Event, "GetAttachedReceivers"))
for _, Value in { 0.25, -1, 2, 0 / 0, "0.75", true, 0.3 } do
	Try(`SetValue({Show(Value)})`, function()
		Event:SetValue(Value)
		return Receiver:GetCurrentValue()
	end)
end
Try("SetValue()", M(Event, "SetValue"))
Try("SetValue(x)", M(Event, "SetValue", "x"))
local Later = Instance.new("CustomEventReceiver")
Later.Source = Event
Try("a receiver attached later", M(Later, "GetCurrentValue"))
Receiver.Source = nil
Try("detached", M(Receiver, "GetCurrentValue"))
task.wait()
print("source value changes " .. table.concat(Changes, " "))

-- Accessory and Makeup descriptions
local Description = Instance.new("HumanoidDescription")
local Accessory = Instance.new("AccessoryDescription")
local Makeup = Instance.new("MakeupDescription")
Try("AccessoryDescription GetAppliedInstance unparented", M(Accessory, "GetAppliedInstance"))
Try("MakeupDescription GetAppliedInstance unparented", M(Makeup, "GetAppliedInstance"))
Accessory.Parent = Description
Makeup.Parent = Description
Try("AccessoryDescription GetAppliedInstance under a description", M(Accessory, "GetAppliedInstance"))
Try("MakeupDescription GetAppliedInstance under a description", M(Makeup, "GetAppliedInstance"))
Try("GetStyledPropertyChangedSignal(AssetId)", function()
	local Signal = Accessory:GetStyledPropertyChangedSignal("AssetId")
	return typeof(Signal), tostring(Signal)
end)
Try("GetStyledPropertyChangedSignal()", M(Accessory, "GetStyledPropertyChangedSignal"))
Try("GetStyledPropertyChangedSignal(Nope)", M(Accessory, "GetStyledPropertyChangedSignal", "Nope"))
Accessory.Parent = nil
Makeup.Parent = nil

-- HumanoidDescription accessories
local Fields = { "HatAccessory", "HairAccessory", "FaceAccessory", "NeckAccessory", "ShouldersAccessory", "FrontAccessory", "BackAccessory", "WaistAccessory" }
local function FieldsOf()
	local Parts = {}
	for _, Field in Fields do
		table.insert(Parts, Description[Field])
	end
	return table.concat(Parts, "|")
end
Try("GetAccessories(false) empty", M(Description, "GetAccessories", false))
Try("GetAccessories()", M(Description, "GetAccessories"))
for Index, Field in Fields do
	Description[Field] = tostring(100 + Index)
end
Description.HatAccessory = "1, 2,x,,3"
Try("GetAccessories(true) all rigid", M(Description, "GetAccessories", true))
Try("GetAccessories(false) all rigid", M(Description, "GetAccessories", false))
Try("SetAccessories(list, true)", M(Description, "SetAccessories", {
	{ AccessoryType = Enum.AccessoryType.Hat, AssetId = 11 },
	{ AccessoryType = Enum.AccessoryType.Shirt, AssetId = 14, IsLayered = true, Order = 3, Puffiness = 0.5 },
	{ AccessoryType = Enum.AccessoryType.Hair, AssetId = 13 },
	{ AccessoryType = Enum.AccessoryType.Hat, AssetId = 12 },
	{ AccessoryType = Enum.AccessoryType.Jacket, AssetId = 15, IsLayered = true, Order = 1 },
	{ AccessoryType = Enum.AccessoryType.Sweater, AssetId = 16, IsLayered = true },
	{ AccessoryType = Enum.AccessoryType.Back, AssetId = 17, Position = Vector3.new(1, 2, 3), Rotation = Vector3.new(0, 90, 0), Scale = Vector3.new(2, 2, 2) },
	{ AccessoryType = Enum.AccessoryType.Unknown, AssetId = 18 },
	{ AccessoryType = Enum.AccessoryType.TShirt, AssetId = 19 },
	{ AccessoryType = Enum.AccessoryType.Face, AssetId = "20" },
	{ AccessoryType = Enum.AccessoryType.Neck, AssetId = 21.5 },
	5,
}, true))
Try("fields after", FieldsOf)
Try("GetAccessories(true) after", M(Description, "GetAccessories", true))
Try("GetAccessories(false) after", M(Description, "GetAccessories", false))
Description.HairAccessory = "31,32"
Try("GetAccessories(true) after a Hair write", M(Description, "GetAccessories", true))
Try("SetAccessories(layered, false)", M(Description, "SetAccessories", { { AccessoryType = Enum.AccessoryType.Pants, AssetId = 41, IsLayered = true, Order = 2 }, { AccessoryType = Enum.AccessoryType.Hat, AssetId = 42 } }, false))
Try("GetAccessories(true) after layered", M(Description, "GetAccessories", true))
Try("SetAccessories(rigid with an Order)", M(Description, "SetAccessories", { { AccessoryType = Enum.AccessoryType.Hat, AssetId = 51, Order = 9 } }, true))
Try("GetAccessories(true) rigid with an Order", M(Description, "GetAccessories", true))
Try("SetAccessories(layered and rigid hat)", M(Description, "SetAccessories", { { AccessoryType = Enum.AccessoryType.Hat, AssetId = 52, IsLayered = true, Order = 1 }, { AccessoryType = Enum.AccessoryType.Hat, AssetId = 53 } }, true))
Try("GetAccessories(true) layered and rigid hat", M(Description, "GetAccessories", true))
Try("SetAccessories(nil, true)", M(Description, "SetAccessories", nil, true))
Try("SetAccessories(5, true)", M(Description, "SetAccessories", 5, true))
Try("SetAccessories({})", M(Description, "SetAccessories", {}))
Try("SetAccessories(dictionary, true)", M(Description, "SetAccessories", { A = {} }, true))
Try("GetAccessories(5)", function() return #Description:GetAccessories(5) end)

-- Model.PrimaryPart once its part leaves the model
local Model = Instance.new("Model")
local A = Instance.new("Part")
A.Parent = Model
Model.PrimaryPart = A
Model.Parent = workspace
local PrimaryChanges = 0
Model:GetPropertyChangedSignal("PrimaryPart"):Connect(function() PrimaryChanges += 1 end)
local Inner = Instance.new("Folder")
Inner.Parent = Model
A.Parent = Inner
Try("PrimaryPart moved deeper", function() return Model.PrimaryPart end)
Inner.Parent = workspace
Try("PrimaryPart after its folder left", function() return Model.PrimaryPart end)
Inner.Parent = Model
Try("PrimaryPart after the folder came back", function() return Model.PrimaryPart end)
Model.PrimaryPart = A
A:Destroy()
Try("PrimaryPart after Destroy", function() return Model.PrimaryPart end)
local B = Instance.new("Part")
B.Parent = Model
Model.PrimaryPart = B
Model.Parent = nil
Model.Parent = workspace
Try("PrimaryPart after the model left and came back", function() return Model.PrimaryPart end)
local Sub = Instance.new("Model")
local C = Instance.new("Part")
C.Parent = Sub
Sub.PrimaryPart = C
Sub.Parent = Model
Model.PrimaryPart = C
C.Parent = Model
Try("PrimaryParts after the part left the inner model for the outer", function() return Model.PrimaryPart, Sub.PrimaryPart end)
local Outside = Instance.new("Part")
Model.PrimaryPart = Outside
Try("PrimaryPart set to a part outside", function() return Model.PrimaryPart end)
Outside.Parent = Model
Outside.Parent = nil
Try("PrimaryPart after that part joined and left", function() return Model.PrimaryPart end)
task.wait()
print("PrimaryPart changes", PrimaryChanges)

-- GuiService
local GuiService = game:GetService("GuiService")
Try("navigation defaults", function() return GuiService.GuiNavigationEnabled, GuiService.AutoSelectGuiEnabled end)
GuiService.GuiNavigationEnabled = false
Try("navigation after a write", function() return GuiService.GuiNavigationEnabled, GuiService.AutoSelectGuiEnabled end)
GuiService.GuiNavigationEnabled = true

-- RunService's step bindings
local RunService = game:GetService("RunService")
Try("BindToSimulation()", M(RunService, "BindToSimulation"))
Try("BindToSimulation(5)", M(RunService, "BindToSimulation", 5))
Try("BindToSimulation(f, x)", M(RunService, "BindToSimulation", function() end, "x"))
Try("BindToAnimation()", M(RunService, "BindToAnimation"))
Try("IsResimulating", M(RunService, "IsResimulating"))
local Steps = {}
local Watch = false
local Pre = RunService.PreSimulation:Connect(function()
	if Watch then
		table.insert(Steps, "PreSimulation")
		task.defer(function() table.insert(Steps, "PreSimulation's defer") end)
	end
end)
local Bound = {}
local function Bind(Method, Label, Priority)
	table.insert(Bound, RunService[Method](RunService, function(Delta, ...)
		if Watch then
			table.insert(Steps, `{Label} {string.format("%.4f", Delta)} {select("#", ...)}`)
			task.defer(function() table.insert(Steps, Label .. "'s defer") end)
		end
	end, Enum.StepFrequency.Hz60, Priority))
end
Bind("BindToSimulation", "simulation", nil)
Bind("BindToSimulation", "simulation first", 1)
Bind("BindToSimulation", "simulation last", 3000)
Bind("BindToAnimation", "animation", nil)
task.wait()
Watch = true
task.wait()
Watch = false
Pre:Disconnect()
for _, Connection in Bound do
	Connection:Disconnect()
end
print("step bindings " .. table.concat(Steps, " | "))
task.wait()
print("logged " .. table.concat(Logged, " | "))
