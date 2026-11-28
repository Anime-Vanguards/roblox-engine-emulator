-- What a game script gets of each service whose answers differ by service: FindService before any GetService, GetService,
-- FindService again, FindFirstChildOfClass and an index of the DataModel. Services Roblox keeps for its own scripts, Studio and plugins
-- are nil to all of them (some read nil through an index too), a client holds its starting services from the start (FindService finds
-- them), a few GetService makes are named "Instance", and a side refuses to make the network service it lacks.
local function Outcome(Ok, Value)
	if not Ok then
		return "err " .. (string.gsub(tostring(Value), 'DataModel "[^"]*"', 'DataModel "<game>"'))
	elseif Value == nil then
		return "nil"
	elseif typeof(Value) == "Instance" then
		return "inst " .. Value.ClassName .. "/" .. Value.Name
	end
	return typeof(Value)
end
for _, Name in {
	"AccountService", "AchievementService", "ActivityHistoryEventService", "AdService", "AnalyticsService",
	"AnimatedImageService", "AnimationClipProvider", "AnimationFromVideoCreatorService",
	"AnimationFromVideoCreatorStudioService", "AnnotationsService", "AppAgeSignalsService", "AppLifecycleObserverService",
	"AppRatingPromptService", "AppStorageService", "AppUpdateService", "AssetCounterService", "AssetDeliveryProxy",
	"AssetImportService", "AssetManagerService", "AssetQualityService", "AssetService", "AudioFocusService",
	"AuroraScriptService", "AuroraService", "AvatarChatService", "AvatarCreationService", "AvatarEditorService",
	"AvatarImportService", "AvatarSettings", "BackendReplicatedStorage", "BackendServerScriptService", "BackendServerStorage",
	"BadgeService", "BranchService", "BrowserService", "BugReporterService", "CSGDictionaryService",
	"CacheableContentProvider", "CallingService", "CalloutService", "CaptureService", "ChangeHistoryStreamingService", "Chat",
	"ClientStorageService", "CloudCRUDService", "CloudClientService", "CloudExecutionService", "ClusterPacketCache",
	"CollaboratorsService", "CollectionService", "CommerceService", "ConfigureServerService", "ConnectivityService",
	"ContentProvider", "ContextActionService", "ControllerService", "CookiesService", "CoreGui", "CorePackages",
	"CoreScriptDebuggingManagerHelper", "CoreScriptSyncService", "CreationDBService", "CrossDMScriptChangeListener",
	"DataStoreService", "Debris", "DebuggablePluginWatcher", "DebuggerConnectionManager", "DebuggerManager",
	"DebuggerUIService", "DeferredAssetManagerService", "DesignFoundationsService", "DeviceDisplayService", "DeviceIdService",
	"DraftsService", "DraggerService", "EditableService", "EditorSourceService", "EventIngestService", "ExampleV2Service",
	"ExperienceAuthService", "ExperienceService", "ExperienceStateCaptureService", "ExperienceStateRecordingService",
	"ExplorerServiceVisibilityService", "ExternalIdentityService", "FaceAnimatorService", "FacialAgeEstimationService",
	"FacialAnimationRecordingService", "FacialAnimationStreamingServiceV2", "FeatureRestrictionManager", "FileManagerService",
	"FileSyncReplicationService", "FlagStandService", "FlyweightService", "FriendService", "FriendsCallingService",
	"GameDownloaderService", "GamePassService", "GenerationService", "GongService", "GuiService", "GuidRegistryService",
	"HSRDataContentProvider", "HarmonyService", "HeapProfilerService", "HeatmapQueryService", "HeatmapService",
	"HiddenPlayerService", "Hopper", "HttpRbxApiService", "HttpService", "ILegacyStudioBridge", "IXPService",
	"ImageScreenCaptureService", "IncrementalPatchBuilder", "InputReplayService", "InsertService", "InstanceFileSyncService",
	"InstanceRegistryService", "IntegrityService", "IntentService", "InternalMessagingService",
	"InternalMessagingServiceVerifier", "InternalSyncService", "JointsService", "KeyboardService", "KeyframeSequenceProvider",
	"LanguageService", "LegacyStudioBridge", "Lighting", "LinkingService", "LiveScriptingService", "LiveSyncService",
	"LocalStorageService", "LocalizationService", "LodDataService", "LogReporterService", "LogService", "LoginService",
	"LuaWebService", "LuauExpressionService", "LuauScriptAnalyzerService", "MLModelDeliveryService", "MarketplaceService",
	"MaterialGenerationService", "MaterialService", "MemoryStoreService", "MeshContentProvider", "MessageBusService",
	"MessagingService", "MetaBreakpointManager", "MicroProfilerService", "ModelInfoService", "ModerationService",
	"MouseService", "NetworkClient", "NetworkServer", "NonReplicatedCSGDictionaryService", "NotificationService",
	"OmniRecommendationsService", "PackageService", "PackageUIService", "Packages", "PartyEmulatorService",
	"PathfindingService", "PerformanceControlService", "PermissionsService", "PhysicsService", "PinShortcutService",
	"PlaceAssetIdsService", "PlaceSettings", "PlaceStatsService", "PlatformCloudStorageService", "PlatformFriendsService",
	"PlatformLibraries", "PlayerEmulatorService", "PlayerHydrationService", "Players", "PluginConnectionService",
	"PluginDebugService", "PluginGuiService", "PluginPolicyService", "PointsService", "PolicyService",
	"ProceduralBehaviorSchedulerService", "ProcessInstancePhysicsService", "ProjectService", "ProximityPromptService",
	"PublishService", "RbxAnalyticsService", "RecommendationService", "RemoteCursorService", "RemoteDebuggerServer",
	"ReplicatedFirst", "ReplicatedStorage", "RequestOrchestratorService", "RibbonNotificationService",
	"RobloxPluginGuiService", "RobloxReplicatedStorage", "RobloxServerStorage", "RolloutValidationService",
	"RomarkRbxAnalyticsService", "RomarkService", "RtMessagingService", "RunService", "RuntimeContentService",
	"RuntimeScriptService", "SafetyService", "ScriptChangeService", "ScriptCloneWatcher", "ScriptCloneWatcherHelper",
	"ScriptCommitService", "ScriptContext", "ScriptDebuggerService", "ScriptEditorService", "ScriptProfilerService",
	"ScriptRegistrationService", "ScriptScannerService", "ScriptService", "Selection", "SerializationService",
	"ServerScriptService", "ServerStorage", "ServiceVisibilityService", "SessionService", "SharedTableRegistry",
	"SlimAnimationReplicationService", "SlimContentProvider", "SlimDebugSettings", "SlimReplicationService", "SlimService",
	"SmoothVoxelsUpgraderService", "SocialService", "SolidModelContentProvider", "SoundService", "SoundShimService",
	"SpawnerService", "StartPageService", "StarterGui", "StarterPack", "StarterPlayer", "StartupMessageService", "Stats",
	"StopWatchReporter", "StudioAssetService", "StudioCameraService", "StudioCaptureService", "StudioData",
	"StudioDeviceEmulatorService", "StudioDeviceSimulatorService", "StudioPublishService", "StudioScriptDebugEventListener",
	"StudioSdkService", "StudioService", "StudioTestService", "StudioUserService", "StudioWidgetsService",
	"SystemThemeService", "TeamCreateData", "TeamCreatePublishService", "TeamCreateService", "Teams", "TelemetryService",
	"TeleportService", "TemporaryCageMeshProvider", "TemporaryScriptService", "TestCoordinatorService", "TestService",
	"TextChatService", "TextService", "TextureGenerationService", "ThirdPartyUserService", "TimerService",
	"ToastNotificationService", "TouchInputService", "TraceRouteService", "TracerService", "TutorialService", "TweenService",
	"UGCAvatarService", "UnvalidatedAssetService", "UserInputService", "UserStorageService", "VRService",
	"VersionControlService", "VideoCaptureService", "VideoScreenCaptureService", "VideoService", "VirtualUser",
	"VisibilityCheckDispatcher", "VoiceChatService", "WebSocketService", "WebViewService", "WindowProtocolService",
	"Workspace", "WrapContentProvider",
} do
	local Find = Outcome(pcall(game.FindService, game, Name))
	local Get = Outcome(pcall(game.GetService, game, Name))
	local Again = Outcome(pcall(game.FindService, game, Name))
	local Child = Outcome(pcall(game.FindFirstChildOfClass, game, Name))
	local Index = Outcome(pcall(function()
		return (game :: any)[Name]
	end))
	print(Name, "|", Find, "|", Get, "|", Again, "|", Child, "|", Index)
end
