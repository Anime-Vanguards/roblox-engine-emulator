-- GetService and FindService of strings that name no service: a client answers nil for a name it knows (a class's, a member's of
-- any class, an enum's, an enum item's, a type's) and raises "'<name>' is not a valid Service name" for any other, a script's own
-- strings included. Names from each kind, those rbx-dom's older database still lists that a client no longer knows, and
-- others. FindService first (it makes nothing), then GetService of the names no service class has.
local function Outcome(Method, Name)
	local Ok, Value = pcall(Method, game, Name)
	if not Ok then
		local Text = tostring(Value)
		if Text == "'" .. Name .. "' is not a valid Service name" then
			return "raise"
		end
		return "error " .. P.err(Text)
	elseif Value == nil then
		return "nil"
	end
	return typeof(Value) .. " " .. Value.ClassName
end
local function Check(Label, Names, Get)
	for _, Name in Names do
		local Line = Label .. " '" .. Name .. "' find " .. Outcome(game.FindService, Name)
		if Get then
			Line ..= " get " .. Outcome(game.GetService, Name)
		end
		print(Line)
	end
end

Check("class", {
	"Part", "Folder", "Model", "Instance", "BasePart", "Camera", "Terrain", "PlayerGui", "Backpack", "ModuleScript", "Player",
	"AuroraScript", "UserSettings", "GlobalSettings", "GenericSettings", "CoreScript", "Plugin", "PluginGui", "DockWidgetPluginGui",
	"StudioWidget", "Workspace",
}, true)
Check("class-gone", {
	"BulkImportService", "ChatbotUIService", "ConversationalAIAcceptanceService", "DataModelPatchService", "EmotesPages",
	"HandRigDescription", "LSPFileSyncService", "PlayerDataRecord", "PlayerDataRecordConfig", "SnippetService", "StreamingService",
}, true)
Check("member", {
	"Anchored", "Size", "Name", "Parent", "Changed", "GetService", "FindFirstChild", "Kick", "LoadCharacter", "LevelOfDetail",
	"AttributesSerialize", "Tags", "Auto Indent Rule", "workspace", "lighting", "Character", "PrimaryPart", "RunContext",
	"OnInvoke", "isA", "clone",
}, true)
Check("member-gone", {
	"GameSettingsAssetIDFace", "Pelvis", "LeftToesSize", "shap", "siz", "Readable", "Writable", "PrimalPhysicsSolver",
	"LoadFailureBehavior", "Waveform",
}, true)
Check("enum", { "Material", "KeyCode", "CoreGuiType", "ModelLevelOfDetail", "RunContext", "Font" }, true)
Check("enum-gone", {
	"Language", "CatalogTimedOptionFilter", "PlayerDataErrorState", "HandRigDescriptionSide", "MoverConstraintRootBehaviorMode",
	"PlayerDataLoadFailureBehavior",
}, true)
Check("item", { "Plastic", "Neon", "SmoothPlastic", "White", "Legacy", "Automatic", "Kick" }, true)
Check("item-gone", { "BlendNode", "FallbackToDefault", "FlushFailed", "TimedOptionOnly" }, true)
Check("type", {
	"string", "number", "bool", "int", "int64", "float", "double", "null", "nil", "function", "table", "thread", "buffer", "Tuple",
	"Array", "Dictionary", "Map", "Variant", "Content", "ProtectedString", "BinaryString", "SharedString", "Ray", "Region3",
	"NumberSequence", "ColorSequence", "PhysicalProperties", "DateTime", "RaycastParams", "OverlapParams", "EventInstance",
	"Path2DControlPoint", "SecurityCapabilities", "Signal", "Connection", "token", "Rect2D", "InstanceRef", "script", "settings",
	"stats", "time", "type", "x", "part", "folder", "roblox", "Vector3", "CFrame", "Color3", "UDim2", "BrickColor", "Object",
}, true)
Check("other", {
	"boolean", "void", "Objects", "RBXScriptSignal", "RBXScriptConnection", "EnumItem", "Enums", "userdata", "vector", "math",
	"print", "game", "shared", "_G", "plugin", "Bright red", "Black", "Medium stone grey", "Lighting ", "", " ", "Nope", "true",
	"false", "5", "{}", "a", "tostring", "runService", "Run Service", "Script Context", "PART", "Enum.Material", "rbxassetid://1",
	" HttpService", "httpservice", "require", "task", "wait", "typeof", "os", "debug", "Stack Begin",
}, true)

-- strings a script makes: before and after the engine holds them as an instance's name, an attribute and a tag
local Made = { "ProbeMadeName", "ProbeMadeAttribute", "ProbeMadeTag" }
Check("made-before", Made, false)
local Folder = Instance.new("Folder")
Folder.Name = "ProbeMadeName"
Folder:SetAttribute("ProbeMadeAttribute", 1)
Folder:AddTag("ProbeMadeTag")
Folder.Parent = workspace
Check("made-after", Made, true)
Folder:Destroy()
