-- How a Parent write is refused: by class ("Cannot change Parent of type X", every write, the parent it has too), as an
-- instance ("The Parent property of X is locked, current parent: P, new parent Y", after the parent it has passed as no
-- change), by a settings root's capability, or not at all. Every service a game script gets on a client and a server alike,
-- then the instances the engine makes and locks or leaves unlocked, and Destroy of the ones locked by class.
P.sync()
local Players = game:GetService("Players")
local Me = Players.LocalPlayer
-- the place's and the player's names differ between engines
local function Escape(Text)
	return (string.gsub(Text, "%p", "%%%0"))
end
local function Clean(Text)
	local Game = Escape(game.Name)
	Text = string.gsub(Text, "current parent: " .. Game .. ",", "current parent: <game>,")
	Text = string.gsub(Text, "of " .. Game .. " is locked", "of <game> is locked")
	Text = string.gsub(Text, "new parent " .. Game .. "$", "new parent <game>")
	if Me then
		Text = string.gsub(Text, "Players%." .. Escape(Me.Name), "Players.<player>")
	end
	return Text
end
local function Try(Function)
	return Clean(P.try(Function))
end
local Target = Instance.new("Folder")
Target.Name = "Target"

-- the parent it has, another (nil, or Target when it has none), itself; a write that went through is put back
local function Writes(Label, Object, Movable)
	local Old = Object.Parent
	local Same = Try(function()
		Object.Parent = Old
	end)
	local Other = "skipped"
	if Movable then
		local New = if Old == nil then Target else nil
		Other = Try(function()
			Object.Parent = New
		end)
		if Object.Parent ~= Old then
			Other ..= " (moved: put back " .. Try(function()
				Object.Parent = Old
			end) .. ")"
		end
	end
	local Self = Try(function()
		Object.Parent = Object
	end)
	print(Label .. " | same " .. Same .. " | other " .. Other .. " | self " .. Self)
end

-- every service a client and a server both answer GetService with (a client's NetworkClient and a server's NetworkServer
-- and ClusterPacketCache aside)
for _, Name in {
	"AdService", "AnalyticsService", "AnimationClipProvider", "AnimationFromVideoCreatorService", "AssetDeliveryProxy",
	"AssetService", "AvatarCreationService", "AvatarEditorService", "BadgeService", "BrowserService", "CaptureService",
	"ChangeHistoryService", "Chat", "CollectionService", "CommerceService", "ConfigService", "ContentProvider",
	"ContextActionService", "CookiesService", "CoreGuiConfiguration", "CreatorStoreService", "DataStoreService",
	"Debris", "EncodingService", "ExperienceNotificationService", "FeatureMatchingService",
	"FeatureRestrictionManager", "FriendService", "GamePassService", "GamepadService", "GenerationService",
	"GenericChallengeService", "Geometry", "GeometryService", "GroupService", "GuiService", "GuidRegistryService",
	"HapticService", "HeapProfilerService", "HeightmapImporterService", "HttpRbxApiService", "HttpService",
	"IncrementalPatchBuilder", "InsertService", "InstanceExtensionsService", "InstanceRegistryService",
	"IntegrityService", "JointsService", "KeyboardService", "KeyframeSequenceProvider", "Lighting",
	"LocalizationService", "LogService", "MLService", "MarketplaceService", "MatchmakingService", "MaterialService",
	"MemStorageService", "MemoryStoreService", "MessagingService", "MicroProfilerService", "ModerationService",
	"MomentsService", "MouseService", "NotificationService", "OpenCloudService", "PackageService", "Packages",
	"PathfindingService", "PermissionsService", "PhysicsService", "PlacesService", "PlayerDataService",
	"PlayerViewService", "Players", "PluginManagementService", "PointsService", "PolicyService", "PopLatencyService",
	"Preloaded", "ProximityPromptService", "PublishService", "QueueService", "RecommendationService",
	"ReflectionService", "RemoteCommandService", "ReplicatedFirst", "ReplicatedStorage", "RunService",
	"SceneAnalysisService", "ScriptContext", "ScriptProfilerService", "ScriptService", "Selection",
	"SelectionHighlightManager", "ServerScriptService", "ServerStorage", "ServiceVisibilityService",
	"SessionCheckService", "SharedTableRegistry", "SlimContentProvider", "SocialService", "SoundService", "StarterGui",
	"StarterPack", "StarterPlayer", "StartupMessageService", "Stats", "StudioTestService", "StylingService", "Teams",
	"TeleportService", "TestService", "TextBoxService", "TextChatService", "TextService", "TimerService",
	"TouchInputService", "TraceRouteService", "TweenService", "UGCValidationService", "UIDragDetectorService",
	"UniqueIdLookupService", "UserInputService", "UserService", "VRService", "VRStatusService", "VideoCaptureService",
	"VideoScreenCaptureService", "VideoService", "VirtualInputManager", "VirtualUser", "VisibilityCheckDispatcher",
	"Visit", "VisualizationModeService", "VoiceChatInternal", "VoiceChatService", "Workspace",
} do
	local Ok, Service = pcall(game.GetService, game, Name)
	if not Ok or Service == nil then
		print(Name .. " get " .. P.try(game.GetService, game, Name))
	else
		Writes(Name, Service, true)
	end
end

-- the new parent's name in a lock's message: its Name, not its full name
local Holder = Instance.new("Folder")
Holder.Name = "Holder"
Holder.Parent = game:GetService("ReplicatedStorage")
local Deep = Instance.new("Folder")
Deep.Name = "Deep"
Deep.Parent = Holder
print("named workspace " .. Try(function()
	workspace.Parent = Deep
end) .. " | datamodel " .. Try(function()
	game.Parent = Deep
end) .. " | lighting " .. Try(function()
	game:GetService("Lighting").Parent = Deep
end))

-- the DataModel, Terrain, the Camera, the user's settings
Writes("datamodel", game, true)
Writes("terrain", workspace.Terrain, true)
Writes("camera", workspace.CurrentCamera, false)
local Root = UserSettings()
Writes("usersettings", Root, true)
Writes("gamesettings", Root:GetService("UserGameSettings"), true)

-- a destroyed instance and a destroyed one's child
local Gone = Instance.new("Folder")
Gone.Name = "Gone"
Gone.Parent = workspace
Gone:Destroy()
Writes("destroyed", Gone, true)
local GoneParent = Instance.new("Folder")
GoneParent.Name = "GoneParent"
local GoneChild = Instance.new("Folder")
GoneChild.Name = "GoneChild"
GoneChild.Parent = GoneParent
GoneParent.Parent = workspace
GoneParent:Destroy()
Writes("destroyed-child", GoneChild, true)

-- the local player's: the Player and its Backpack have no lock, PlayerGui and PlayerScripts are locked by class, the
-- PlayerMouse as an instance
if Me then
	Writes("player", Me, false)
	Writes("backpack", Me:FindFirstChildOfClass("Backpack"), false)
	Writes("playergui", Me:FindFirstChildOfClass("PlayerGui"), true)
	Writes("playerscripts", Me:FindFirstChildOfClass("PlayerScripts"), true)
	Writes("mouse", Me:GetMouse(), true)
end

-- Destroy of what is locked by class: Terrain refuses, a player's PlayerGui and PlayerScripts go and keep their lock
local function DestroyOf(Label, Object)
	local Result = Try(function()
		Object:Destroy()
	end)
	print("destroy " .. Label .. " " .. Result .. " | parent " .. (if Object.Parent then Object.Parent.ClassName else "nil") .. " | write " .. Try(function()
		Object.Parent = Object.Parent
	end) .. " | back " .. Try(function()
		Object.Parent = Target
	end))
end
DestroyOf("terrain", workspace.Terrain)
if Me then
	DestroyOf("playerscripts", Me:FindFirstChildOfClass("PlayerScripts"))
	DestroyOf("playergui", Me:FindFirstChildOfClass("PlayerGui"))
end
Holder:Destroy()
