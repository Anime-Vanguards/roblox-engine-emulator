-- Instance.new of the classes a game script may not make, or makes though they are services: a class whose New permit needs a
-- capability the script lacks, a service whose GetService permit does, the classes refused with words of their own, and the
-- services Instance.new makes all the same (each class of the API dump, the security dump and rbx-dom's database was tried;
-- these are the ones a client answers for otherwise than "Unable to create", and a few that answer so)
for _, Name in {
	"AnimationPlayer", "AuroraScript", "AvatarAbilityRules", "Breakpoint", "CSGDictionaryService", "CustomLog", "DebuggerWatch",
	"ExplorerFilter", "FlyweightService", "FollowRotationBoneBehavior", "FollowTwistBoneBehavior", "HeightmapImporterService",
	"InternalSyncItem", "MemoryStoreService", "Noise", "NonReplicatedCSGDictionaryService", "ParabolaAdornment", "Player",
	"PluginAction", "PluginCapabilities", "ProximityPromptService", "RTAnimationTracker", "RealtimeMedia", "RelativeGui",
	"StandalonePluginScripts", "StateMachineDefinition", "StudioAttachment", "StudioCallout", "TestService", "TrackerStreamAnimation",
	"VideoDeviceInput", "VirtualInputManager", "Part", "Folder", "Workspace", "CoreGui", "NetworkClient", "NotAClass",
} do
	print(Name, P.try(function()
		local Made = Instance.new(Name)
		local Class, Parent = Made.ClassName, Made.Parent
		pcall(Made.Destroy, Made)
		return Class, Parent
	end))
end
-- a made service is a plain instance of its class, outside the DataModel
local Made = Instance.new("TestService")
print("made service", Made.Name, Made.Parent, Made:IsA("Instance"), game:GetService("TestService") ~= Made)
print("made service parent", P.try(function() Made.Parent = workspace end), P.try(function() Made:Destroy() end))
