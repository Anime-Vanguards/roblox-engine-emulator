-- Every property a game script reaches whose Read or Write permit needs a capability it lacks, or that has no permit at all
-- (data/capability-properties.rs; a playing client's and a server's answers are the same): its read, a write of a value it takes, a
-- write of a table, and a read with a child of its name, on the class a client makes for it.
local function shown(Problem)
	local Text = P.err(Problem)
	-- (a full name differs between engines: the DataModel's; the member's own name stays)
	return (string.gsub(Text, " is not a valid member of .*$", " is not a valid member"))
end
local function value(Code)
	local Kind = string.sub(Code, 1, 1)
	if Kind == "b" then return true, false
	elseif Kind == "n" then return true, 0
	elseif Kind == "s" then return true, ""
	elseif Kind == "e" then
		local Ok, Items = pcall(function() return (Enum :: any)[string.sub(Code, 2)]:GetEnumItems() end)
		if Ok and Items[1] then return true, Items[1] end
		return false
	elseif Kind == "c" then return true, Color3.new()
	elseif Kind == "2" then return true, Vector2.zero
	elseif Kind == "3" then return true, Vector3.zero
	elseif Kind == "f" then return true, CFrame.identity
	elseif Kind == "t" then return true, Content.none
	elseif Kind == "r" then return true, NumberRange.new(0)
	elseif Kind == "x" then return true, Rect.new(0, 0, 0, 0)
	elseif Kind == "k" then return true, BrickColor.new(1)
	elseif Kind == "i" then return true, nil
	end
	return false
end
local Special = { Workspace = workspace, Terrain = workspace.Terrain, DataModel = game }
local Rows = {
	{ "Accessory", false, { "Instance.Attributes:s:w", "Instance.AttributesReplicate:s:w", "Instance.AttributesSerialize:-:w", "Instance.DataCost:n:w", "Instance.DefinesCapabilities:b:w", "Instance.HistoryId:-:w", "Instance.PropertyStatusStudio:ePropertyStatus:w", "Instance.RobloxLocked:b:w", "Instance.SerializedOverrides:-:w", "Instance.SourceAssetId:n:w", "Instance.Tags:-:w", "Instance.UniqueId:-:w" } },
	{ "AdGui", false, { "GuiBase2d.ActiveQueryNames:s:w", "GuiBase2d.ClippedRect:x:w", "GuiBase2d.IsNotOccluded:b:w", "GuiBase2d.RawRect2D:x:w", "GuiBase2d.ReplicatedInsertionOrder:n:w", "GuiBase2d.TotalGroupScale:n:w", "LayerCollector.InputSinkEnabled:eRolloutState:w", "LayerCollector.TabKeyboardNavigation:b:w" } },
	{ "AdPortal", false, { "AdPortal.PortalVersion:n:w" } },
	{ "AnalyticsService", true, { "AnalyticsService.ApiKey:s:w" } },
	{ "AnimationConstraint", false, { "AnimationConstraint.EnableSkinning:b:w" } },
	{ "AnimationGraphDefinition", false, { "AnimationClip.Guid:s:w", "AnimationClip.GuidBinaryString:-:w" } },
	{ "Animator", false, { "Animator.AnimTrackMetadata0:-:w", "Animator.AnimTrackMetadata1:-:w", "Animator.AnimTrackMetadata10:-:w", "Animator.AnimTrackMetadata11:-:w", "Animator.AnimTrackMetadata12:-:w", "Animator.AnimTrackMetadata13:-:w", "Animator.AnimTrackMetadata14:-:w", "Animator.AnimTrackMetadata15:-:w", "Animator.AnimTrackMetadata2:-:w", "Animator.AnimTrackMetadata3:-:w", "Animator.AnimTrackMetadata4:-:w", "Animator.AnimTrackMetadata5:-:w", "Animator.AnimTrackMetadata6:-:w", "Animator.AnimTrackMetadata7:-:w", "Animator.AnimTrackMetadata8:-:w", "Animator.AnimTrackMetadata9:-:w", "Animator.AnimTrackPlayState0:-:w", "Animator.AnimTrackPlayState1:-:w", "Animator.AnimTrackPlayState10:-:w", "Animator.AnimTrackPlayState11:-:w", "Animator.AnimTrackPlayState12:-:w", "Animator.AnimTrackPlayState13:-:w", "Animator.AnimTrackPlayState14:-:w", "Animator.AnimTrackPlayState15:-:w", "Animator.AnimTrackPlayState2:-:w", "Animator.AnimTrackPlayState3:-:w", "Animator.AnimTrackPlayState4:-:w", "Animator.AnimTrackPlayState5:-:w", "Animator.AnimTrackPlayState6:-:w", "Animator.AnimTrackPlayState7:-:w", "Animator.AnimTrackPlayState8:-:w", "Animator.AnimTrackPlayState9:-:w", "Animator.AnimTrackWeight0:-:w", "Animator.AnimTrackWeight1:-:w", "Animator.AnimTrackWeight10:-:w", "Animator.AnimTrackWeight11:-:w", "Animator.AnimTrackWeight12:-:w", "Animator.AnimTrackWeight13:-:w", "Animator.AnimTrackWeight14:-:w", "Animator.AnimTrackWeight15:-:w", "Animator.AnimTrackWeight2:-:w", "Animator.AnimTrackWeight3:-:w", "Animator.AnimTrackWeight4:-:w", "Animator.AnimTrackWeight5:-:w", "Animator.AnimTrackWeight6:-:w", "Animator.AnimTrackWeight7:-:w", "Animator.AnimTrackWeight8:-:w", "Animator.AnimTrackWeight9:-:w", "Animator.AnimationId0:n:w", "Animator.AnimationId1:n:w", "Animator.AnimationId10:n:w", "Animator.AnimationId11:n:w", "Animator.AnimationId12:n:w", "Animator.AnimationId13:n:w", "Animator.AnimationId14:n:w", "Animator.AnimationId15:n:w", "Animator.AnimationId2:n:w", "Animator.AnimationId3:n:w", "Animator.AnimationId4:n:w", "Animator.AnimationId5:n:w", "Animator.AnimationId6:n:w", "Animator.AnimationId7:n:w", "Animator.AnimationId8:n:w", "Animator.AnimationId9:n:w", "Animator.FacsReplicationData:-:w" } },
	{ "Annotation", false, { "Annotation.AuthorColor3:c:w", "Annotation.AuthorId:n:w", "Annotation.ChannelId:s:w", "Annotation.Contents:s:w", "Annotation.CreationTimeUnix:n:w", "Annotation.LastModifiedTimeUnix:n:w", "Annotation.LoadingReplies:b:w", "Annotation.MessageId:s:w", "Annotation.ReplyCount:n:w", "Annotation.Resolved:b:w", "Annotation.TaggedUsers:s:w" } },
	{ "AssetService", true, { "AssetService.AllowInsertFreeAssets:b:w" } },
	{ "AudioCompressor", false, { "AudioCompressor.Editor:b:w" } },
	{ "AudioDeviceInput", false, { "AudioDeviceInput.AccessList:-:w", "AudioDeviceInput.DictationEnabled:b:w", "AudioDeviceInput.EchoCancellation:b:w", "AudioDeviceInput.GainControl:b:w", "AudioDeviceInput.IsReady:b:w", "AudioDeviceInput.MutedByLocalUser:b:w", "AudioDeviceInput.NoiseSuppression:b:w", "AudioDeviceInput.Streaming:b:w", "AudioDeviceInput.Active:b:w" } },
	{ "AudioEmitter", false, { "AudioEmitter.AngleAttenuation:-:w", "AudioEmitter.DistanceAttenuation:-:w" } },
	{ "AudioEqualizer", false, { "AudioEqualizer.Editor:b:w" } },
	{ "AudioFilter", false, { "AudioFilter.Editor:b:w" } },
	{ "AudioLimiter", false, { "AudioLimiter.Editor:b:w" } },
	{ "AudioListener", false, { "AudioListener.AngleAttenuation:-:w", "AudioListener.DistanceAttenuation:-:w" } },
	{ "AudioPlayer", false, { "AudioPlayer.AssetRepresentation:eAssetRepresentation:w", "AudioPlayer.IsPlaying:b:w" } },
	{ "AudioRecorder", false, { "AudioRecorder.IsRecording:b:w" } },
	{ "AudioSpeechToText", false, { "AudioSpeechToText.DictationEnabled:b:w", "AudioSpeechToText.DisableVoiceDetection:b:w", "AudioSpeechToText.EnableVolumeCheck:b:w", "AudioSpeechToText.Locale:s:w", "AudioSpeechToText.Streaming:b:w", "AudioSpeechToText.UsageType:s:w", "AudioSpeechToText.VoiceDetectedOverride:b:w" } },
	{ "AudioTextToSpeech", false, { "AudioTextToSpeech.IsPlaying:b:w" } },
	{ "AvatarAccessoryRules", false, { "AvatarAccessoryRules.CustomAccessoryMode:eAvatarSettingsCustomAccessoryMode:w", "AvatarAccessoryRules.CustomBackAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomBackAccessoryId:n:w", "AvatarAccessoryRules.CustomFaceAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomFaceAccessoryId:n:w", "AvatarAccessoryRules.CustomFrontAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomFrontAccessoryId:n:w", "AvatarAccessoryRules.CustomHairAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomHairAccessoryId:n:w", "AvatarAccessoryRules.CustomHeadAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomHeadAccessoryId:n:w", "AvatarAccessoryRules.CustomNeckAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomNeckAccessoryId:n:w", "AvatarAccessoryRules.CustomShoulderAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomShoulderAccessoryId:n:w", "AvatarAccessoryRules.CustomWaistAccessoryEnabled:b:w", "AvatarAccessoryRules.CustomWaistAccessoryId:n:w" } },
	{ "AvatarAnimationRules", false, { "AvatarAnimationRules.AnimationClipsMode:eAvatarSettingsAnimationClipsMode:w", "AvatarAnimationRules.CustomClimbAnimationEnabled:b:w", "AvatarAnimationRules.CustomClimbAnimationId:n:w", "AvatarAnimationRules.CustomFallAnimationEnabled:b:w", "AvatarAnimationRules.CustomFallAnimationId:n:w", "AvatarAnimationRules.CustomIdleAlt1AnimationEnabled:b:w", "AvatarAnimationRules.CustomIdleAlt1AnimationId:n:w", "AvatarAnimationRules.CustomIdleAlt2AnimationEnabled:b:w", "AvatarAnimationRules.CustomIdleAlt2AnimationId:n:w", "AvatarAnimationRules.CustomIdleAnimationEnabled:b:w", "AvatarAnimationRules.CustomIdleAnimationId:n:w", "AvatarAnimationRules.CustomJumpAnimationEnabled:b:w", "AvatarAnimationRules.CustomJumpAnimationId:n:w", "AvatarAnimationRules.CustomRunAnimationEnabled:b:w", "AvatarAnimationRules.CustomRunAnimationId:n:w", "AvatarAnimationRules.CustomSwimAnimationEnabled:b:w", "AvatarAnimationRules.CustomSwimAnimationId:n:w", "AvatarAnimationRules.CustomSwimIdleAnimationEnabled:b:w", "AvatarAnimationRules.CustomSwimIdleAnimationId:n:w", "AvatarAnimationRules.CustomWalkAnimationEnabled:b:w", "AvatarAnimationRules.CustomWalkAnimationId:n:w" } },
	{ "AvatarBodyRules", false, { "AvatarBodyRules.AppearanceMode:eAvatarSettingsAppearanceMode:w", "AvatarBodyRules.CustomBodyBundleId:n:w", "AvatarBodyRules.CustomBodyType:eAvatarSettingsCustomBodyType:w", "AvatarBodyRules.CustomBodyTypeScale:r:w", "AvatarBodyRules.CustomEyebrowEnabled:b:w", "AvatarBodyRules.CustomEyebrowId:n:w", "AvatarBodyRules.CustomEyelashEnabled:b:w", "AvatarBodyRules.CustomEyelashId:n:w", "AvatarBodyRules.CustomFaceEnabled:b:w", "AvatarBodyRules.CustomFaceId:n:w", "AvatarBodyRules.CustomHeadEnabled:b:w", "AvatarBodyRules.CustomHeadId:n:w", "AvatarBodyRules.CustomHeadScale:r:w", "AvatarBodyRules.CustomHeight:r:w", "AvatarBodyRules.CustomHeightScale:r:w", "AvatarBodyRules.CustomLeftArmEnabled:b:w", "AvatarBodyRules.CustomLeftArmId:n:w", "AvatarBodyRules.CustomLeftLegEnabled:b:w", "AvatarBodyRules.CustomLeftLegId:n:w", "AvatarBodyRules.CustomMoodEnabled:b:w", "AvatarBodyRules.CustomMoodId:n:w", "AvatarBodyRules.CustomProportionsScale:r:w", "AvatarBodyRules.CustomRightArmEnabled:b:w", "AvatarBodyRules.CustomRightArmId:n:w", "AvatarBodyRules.CustomRightLegEnabled:b:w", "AvatarBodyRules.CustomRightLegId:n:w", "AvatarBodyRules.CustomTorsoEnabled:b:w", "AvatarBodyRules.CustomTorsoId:n:w", "AvatarBodyRules.CustomWidthScale:r:w", "AvatarBodyRules.KeepPlayerHead:b:w" } },
	{ "AvatarClothingRules", false, { "AvatarClothingRules.CustomClassicPantsAccessoryEnabled:b:w", "AvatarClothingRules.CustomClassicPantsAccessoryId:n:w", "AvatarClothingRules.CustomClassicShirtsAccessoryEnabled:b:w", "AvatarClothingRules.CustomClassicShirtsAccessoryId:n:w", "AvatarClothingRules.CustomClassicTShirtsAccessoryEnabled:b:w", "AvatarClothingRules.CustomClassicTShirtsAccessoryId:n:w", "AvatarClothingRules.CustomClothingMode:eAvatarSettingsCustomClothingMode:w", "AvatarClothingRules.CustomDressSkirtAccessoryEnabled:b:w", "AvatarClothingRules.CustomDressSkirtAccessoryId:n:w", "AvatarClothingRules.CustomJacketAccessoryEnabled:b:w", "AvatarClothingRules.CustomJacketAccessoryId:n:w", "AvatarClothingRules.CustomLeftShoesAccessoryEnabled:b:w", "AvatarClothingRules.CustomLeftShoesAccessoryId:n:w", "AvatarClothingRules.CustomPantsAccessoryEnabled:b:w", "AvatarClothingRules.CustomPantsAccessoryId:n:w", "AvatarClothingRules.CustomRightShoesAccessoryEnabled:b:w", "AvatarClothingRules.CustomRightShoesAccessoryId:n:w", "AvatarClothingRules.CustomShirtAccessoryEnabled:b:w", "AvatarClothingRules.CustomShirtAccessoryId:n:w", "AvatarClothingRules.CustomShortsAccessoryEnabled:b:w", "AvatarClothingRules.CustomShortsAccessoryId:n:w", "AvatarClothingRules.CustomSweaterAccessoryEnabled:b:w", "AvatarClothingRules.CustomSweaterAccessoryId:n:w", "AvatarClothingRules.CustomTShirtAccessoryEnabled:b:w", "AvatarClothingRules.CustomTShirtAccessoryId:n:w" } },
	{ "AvatarCollisionRules", false, { "AvatarCollisionRules.HitAndTouchDetectionMode:eAvatarSettingsHitAndTouchDetectionMode:w", "AvatarCollisionRules.LegacyCollisionMode:eAvatarSettingsLegacyCollisionMode:w" } },
	{ "AvatarRules", false, { "AvatarRules.AvatarType:eGameAvatarType:w" } },
	{ "BallSocketConstraint", false, { "BallSocketConstraint.EnableSkinning:b:w" } },
	{ "BoxHandleAdornment", false, { "BoxHandleAdornment.DoubleSided:b:w", "HandleAdornment.GizmoReference:i:w" } },
	{ "CanvasGroup", false, { "CanvasGroup.ResolutionScale:n:w", "GuiObject.SelectionRect2D:x:w", "GuiObject.Sink:eInputSink:w" } },
	{ "Chat", true, { "Chat.IsAutoMigrated:b:w", "Chat.ModerationMode:s:w", "Chat.LoadDefaultChat:b:w" } },
	{ "ConeHandleAdornment", false, { "ConeHandleAdornment.DoubleSided:b:w" } },
	{ "ControlState", false, { "ControlState.StateSchema:-:w" } },
	{ "CornerWedgePart", false, { "BasePart.CollisionGroupReplicate:s:w", "BasePart.PhysicsRepRootPart:i:w", "BasePart.PhysicsRepRootRef:-:w", "BasePart.ReplicationPV:-:w" } },
	{ "CylinderHandleAdornment", false, { "CylinderHandleAdornment.DoubleSided:b:w" } },
	{ "DataModel", true, { "DataModel.Environment:s:w", "DataModel.ForceR15:b:w", "DataModel.IsPioneerBuild:b:w", "DataModel.IsSFFlagsLoaded:b:w", "DataModel.PioneerSource:ePioneerSource:w", "DataModel.GameAvatarType:eGameAvatarType:w", "DataModel.R15CollisionType:eR15CollisionType:w" } },
	{ "DataStoreService", true, { "DataStoreService.AutomaticRetry:b:w", "DataStoreService.LegacyNamingScheme:b:w" } },
	{ "Decal", false, { "Decal.LocalizedTextureContent:t:w", "Decal.MetalnessMap:s:w", "Decal.NormalMap:s:w", "Decal.RoughnessMap:s:w", "Decal.TexturePack:s:w", "Decal.TexturePackContent:t:w", "Decal.TexturePackMetadata:s:w", "Decal.EmissiveMaskContent:t:w", "Decal.MetalnessMapContent:t:w", "Decal.NormalMapContent:t:w", "Decal.RoughnessMapContent:t:w" } },
	{ "FaceControls", false, { "FaceControls.InternalOverrideFACSData:-:w" } },
	{ "Folder", false, { "Folder.ReplicatedGuiInsertionOrder:n:w" } },
	{ "GamepadService", true, { "GamepadService.GamepadCursorEnabled:b:w" } },
	{ "GuiService", true, { "GuiService.CoreEffectFolder:i:w", "GuiService.CoreGuiFolder:i:w", "GuiService.CoreGuiNavigationEnabled:b", "GuiService.CoreUISafeAreaInsetBottomRight:2:w", "GuiService.CoreUISafeAreaInsetTopLeft:2:w", "GuiService.DisplayScalingMode:eDisplayScalingMode:w", "GuiService.FullscreenViewport:x:w", "GuiService.HardwareSafeAreaInsetBottomRight:2:w", "GuiService.HardwareSafeAreaInsetTopLeft:2:w", "GuiService.ScreenContextRawScreenScale:n:w", "GuiService.ScreenContextTopbarInset:x:w", "GuiService.SelectedCoreObject:i:w", "GuiService.ViewportSizeInMM:2:w" } },
	{ "HapticEffect", false, { "HapticEffect.WaveformData:-:w" } },
	{ "HiddenSurfaceRemovalAsset", false, { "HiddenSurfaceRemovalAsset.HSRData:-:w", "HiddenSurfaceRemovalAsset.HSRMeshIdData:-:w" } },
	{ "Highlight", false, { "Highlight.LineThickness:n:w", "Highlight.ReservedId:eReservedHighlightId:w" } },
	{ "HttpService", true, { "HttpService.HttpEnabled:b:w" } },
	{ "Humanoid", false, { "Humanoid.BodyPartsCollidableOnDeath:b:w", "Humanoid.InternalAnimationHipHeight:n:w", "Humanoid.InternalDisplayName:s:w", "Humanoid.OverrideDefaultCollisions:b:w", "Humanoid.RotationType:eRotationType:w", "Humanoid.CollisionType:eHumanoidCollisionType:w" } },
	{ "HumanoidDescription", false, { "HumanoidDescription.NumberEmotesLoaded:n:w", "HumanoidDescription.ResetIncludesBodyParts:b:w" } },
	{ "HumanoidRigDescription", false, { "HumanoidRigDescription.OriginOffset:f:w" } },
	{ "ImageButton", false, { "ImageButton.ContentImageSize:2:w", "ImageButton.LocalizedImageContent:t:w" } },
	{ "ImageHandleAdornment", false, { "ImageHandleAdornment.DoubleSided:b:w" } },
	{ "ImageLabel", false, { "ImageLabel.ContentImageSize:2:w", "ImageLabel.LocalizedImageContent:t:w" } },
	{ "InputAction", false, { "InputAction.BoolState:b:w", "InputAction.Direction1DState:n:w", "InputAction.Direction2DState:2:w", "InputAction.Direction3DState:3:w", "InputAction.Editor:b:w", "InputAction.ViewportPositionState:2:w" } },
	{ "InputBinding", false, { "InputBinding.Editor:b:w" } },
	{ "InputContext", false, { "InputContext.Editor:b:w" } },
	{ "IntersectOperation", false, { "TriangleMeshPart.AeroMeshData:-:w", "TriangleMeshPart.ConvexDecompHolder:-:w", "TriangleMeshPart.FluidFidelityInternal:eFluidFidelity:w", "TriangleMeshPart.InertiaMigrated:b:w", "TriangleMeshPart.PCDRequestId:n:w", "TriangleMeshPart.PhysicalConfigData:-:w", "TriangleMeshPart.UnscaledCofm:3:w", "TriangleMeshPart.UnscaledVolInertiaDiags:3:w", "TriangleMeshPart.UnscaledVolInertiaOffDiags:3:w", "TriangleMeshPart.UnscaledVolume:n:w", "TriangleMeshPart.CollisionFidelity:eCollisionFidelity:w", "TriangleMeshPart.CollisionPrecision:n:w", "TriangleMeshPart.FluidFidelity:eFluidFidelity:w" } },
	{ "KeyframeSequence", false, { "KeyframeSequence.AuthoredHipHeight:n:w" } },
	{ "Lighting", true, { "Lighting.Technology:eTechnology:w", "Lighting.LightingStyle:eLightingStyle:w", "Lighting.PrioritizeLightingQuality:b:w" } },
	{ "LocalScript", false, { "BaseScript.RunContext:eRunContext:w", "LuaSourceContainer.HasAssociatedDrafts:b:w", "LuaSourceContainer.IsDifferentFromFileSystem:b:w", "LuaSourceContainer.SandboxedSource:s:w", "LuaSourceContainer.isPlayerScript:b:w" } },
	{ "LocalizationService", true, { "LocalizationService.ForcePlayModeGameLocaleId:s:w", "LocalizationService.ForcePlayModeRobloxLocaleId:s:w", "LocalizationService.IsImageCaptureEnabled:b:w", "LocalizationService.IsTextScraperRunning:b:w", "LocalizationService.RobloxForcePlayModeGameLocaleId:s:w", "LocalizationService.RobloxForcePlayModeRobloxLocaleId:s:w", "LocalizationService.ShouldUseCloudTable:b:w", "LocalizationService.ShouldUseImageLocalizationTable:b:w" } },
	{ "MaterialService", true, { "MaterialService.AsphaltName:s:w", "MaterialService.BasaltName:s:w", "MaterialService.BrickName:s:w", "MaterialService.CardboardName:s:w", "MaterialService.CarpetName:s:w", "MaterialService.CeramicTilesName:s:w", "MaterialService.ClayRoofTilesName:s:w", "MaterialService.CobblestoneName:s:w", "MaterialService.ConcreteName:s:w", "MaterialService.CorrodedMetalName:s:w", "MaterialService.CrackedLavaName:s:w", "MaterialService.DiamondPlateName:s:w", "MaterialService.FabricName:s:w", "MaterialService.FoilName:s:w", "MaterialService.GlacierName:s:w", "MaterialService.GraniteName:s:w", "MaterialService.GrassName:s:w", "MaterialService.GroundName:s:w", "MaterialService.IceName:s:w", "MaterialService.LeafyGrassName:s:w", "MaterialService.LeatherName:s:w", "MaterialService.LimestoneName:s:w", "MaterialService.MarbleName:s:w", "MaterialService.MetalName:s:w", "MaterialService.MudName:s:w", "MaterialService.PavementName:s:w", "MaterialService.PebbleName:s:w", "MaterialService.PlasterName:s:w", "MaterialService.PlasticName:s:w", "MaterialService.RockName:s:w", "MaterialService.RoofShinglesName:s:w", "MaterialService.RubberName:s:w", "MaterialService.SaltName:s:w", "MaterialService.SandName:s:w", "MaterialService.SandstoneName:s:w", "MaterialService.SlateName:s:w", "MaterialService.SmoothPlasticName:s:w", "MaterialService.SnowName:s:w", "MaterialService.Use2022Materials:b:w", "MaterialService.WoodName:s:w", "MaterialService.WoodPlanksName:s:w" } },
	{ "MaterialVariant", false, { "MaterialVariant.ColorMap:s:w", "MaterialVariant.ColorMapContent:t:w", "MaterialVariant.EmissiveMaskContent:t:w", "MaterialVariant.MetalnessMap:s:w", "MaterialVariant.MetalnessMapContent:t:w", "MaterialVariant.NormalMap:s:w", "MaterialVariant.NormalMapContent:t:w", "MaterialVariant.RoughnessMap:s:w", "MaterialVariant.RoughnessMapContent:t:w", "MaterialVariant.BaseMaterial:eMaterial:w" } },
	{ "MeshPart", false, { "MeshPart.AlternateMeshHash:n:w", "MeshPart.InitialSize:3:w", "MeshPart.PhysicsData:-:w", "MeshPart.VertexCount:n:w", "MeshPart.RenderFidelity:eRenderFidelity:w" } },
	{ "MicroProfilerService", true, { "MicroProfilerService.ContextLabel:s:w" } },
	{ "Model", false, { "Model.LevelOfDetail:eModelLevelOfDetail:w", "Model.SlimHash:-:w" } },
	{ "ModuleScript", false, { "ModuleScript.Confidential:b:w", "ModuleScript.Source:s:w", "ModuleScript.UnrestrictedRequireAllowed:b:w" } },
	{ "Motor", false, { "Motor.ReplicateCurrentAngle:n:w" } },
	{ "Motor6D", false, { "Motor6D.EnableSkinning:b:w", "Motor6D.ReplicateCurrentAngle6D:3:w", "Motor6D.ReplicateCurrentOffset6D:3:w" } },
	{ "NegateOperation", false, { "NegateOperation.PreviousOperation:eNegateOperationHiddenHistory:w" } },
	{ "NotificationService", true, { "NotificationService.IsConnected:b:w", "NotificationService.IsLuaChatEnabled:b:w", "NotificationService.IsLuaGameDetailsEnabled:b:w", "NotificationService.SelectedTheme:s:w" } },
	{ "Packages", true, { "Packages.IsDehydrated:b:w", "Packages.OverridesRecalculated:b:w", "Packages.ShellPackagesCount:n:w", "Packages.SkippedInstancesCount:n:w" } },
	{ "PartOperation", false, { "PartOperation.AssetId:s:w", "PartOperation.ChildData:-:w", "PartOperation.ChildData2:-:w", "PartOperation.ComponentIndex:n:w", "PartOperation.Content:t:w", "PartOperation.DCDPropertyData:-:w", "PartOperation.FormFactor:eFormFactor:w", "PartOperation.InitialSize:3:w", "PartOperation.ManifoldMesh_DEPRECATED:-:w", "PartOperation.MeshData:-:w", "PartOperation.MeshData2:-:w", "PartOperation.OffCentered:b:w", "PartOperation.PhysicsData:-:w", "PartOperation.RenderFidelity:eRenderFidelity:w", "PartOperation.SmoothingAngle:n:w", "PartOperation.TriangleCount:n:w" } },
	{ "PartOperationAsset", false, { "PartOperationAsset.ChildData:-:w", "PartOperationAsset.MeshData:-:w" } },
	{ "Path2D", false, { "Path2D.PropertiesSerialize:-:w", "Path2D.SelectedControlPoint:n:w", "Path2D.SelectedControlPointData:-:w", "Path2D.Transparency:n:w" } },
	{ "Players", true, { "Players.BanningEnabled:b:w", "Players.MaxPlayersInternal:n:w", "Players.PreferredPlayersInternal:n:w", "Players.ServerGitHash:s:w", "Players.ServerLogPrefix:s:w", "Players.UseStrafingAnimations:b:w" } },
	{ "PolicyService", true, { "PolicyService.IsLuobuServer:eTriStateBoolean:w", "PolicyService.LuobuWhitelisted:eTriStateBoolean:w" } },
	{ "PyramidHandleAdornment", false, { "PyramidHandleAdornment.DoubleSided:b:w" } },
	{ "ReflectionMetadataClass", false, { "ReflectionMetadataClass.ServiceVisibility:eServiceVisibility:w" } },
	{ "RigidConstraint", false, { "RigidConstraint.EnableSkinning:b:w" } },
	{ "RunService", true, { "RunService.ClientGitHash:s:w", "RunService.FrameNumber:n:w", "RunService.RunState:eRunState:w" } },
	{ "ScreenGui", false, { "ScreenGui.IgnoresTitleBarReservation:b:w", "ScreenGui.OnTopOfCoreBlur:b:w" } },
	{ "Script", false, { "Script.Source:s:w" } },
	{ "ScriptContext", true, { "ScriptContext.ScriptsDisabled:b:w" } },
	{ "ScrollingFrame", false, { "ScrollingFrame.DraggingScrollBar:eDraggingScrollBar:w", "ScrollingFrame.HorizontalBarRect:x:w", "ScrollingFrame.MaxCanvasPosition:2:w", "ScrollingFrame.ScrollRate:n:w", "ScrollingFrame.ScrollVelocity:2:w", "ScrollingFrame.SmoothScroll:b:w", "ScrollingFrame.VerticalBarRect:x:w" } },
	{ "Selection", true, { "Selection.ActiveInstance:i:w", "Selection.RenderMode:eSelectionRenderMode:w", "Selection.SelectionBoxThickness:n:w", "Selection.SelectionLineThickness:n:w", "Selection.ShowActiveInstanceHighlight:b:w" } },
	{ "SelectionBox", false, { "SelectionBox.StudioSelectionBox:b:w" } },
	{ "ServiceVisibilityService", true, { "ServiceVisibilityService.HiddenServices:-:w", "ServiceVisibilityService.VisibleServices:-:w" } },
	{ "Sound", false, { "Sound.AssetRepresentation:eAssetRepresentation:w", "Sound.ChannelCount:n:w", "Sound.IsSpatial:b:w", "Sound.RollOffGain:n:w", "Sound.UsageContextPermission:eUsageContext:w" } },
	{ "SoundService", true, { "SoundService.AudioApiByDefault:eRolloutState:w", "SoundService.DefaultListenerLocation:eListenerLocation:w", "SoundService.IsNewExpForAudioApiByDefault:b:w", "SoundService.CharacterSoundsUseNewApi:eRolloutState:w" } },
	{ "SphereHandleAdornment", false, { "SphereHandleAdornment.DoubleSided:b:w" } },
	{ "StarterGui", true, { "StarterGui.ProcessUserInput:b:w", "StarterGui.StudioDefaultStyleSheet:i:w", "StarterGui.StudioInsertWidgetLayerCollectorAutoLinkStyleSheet:i:w" } },
	{ "StarterPlayer", true, { "StarterPlayer.AvatarJointUpgrade:eRolloutState:w", "StarterPlayer.CreateDefaultPlayerModule:b:w", "StarterPlayer.GameSettingsAvatar:eGameAvatarType:w", "StarterPlayer.GameSettingsR15Collision:eR15CollisionType:w", "StarterPlayer.PlayerModuleStatus:n:w", "StarterPlayer.UseImprovedCharacterEvents:eRolloutState:w", "StarterPlayer.AllowCustomAnimations:b:w" } },
	{ "StudioTestService", true, { "StudioTestService.EditModeActive:b:w" } },
	{ "StyleQuery", false, { "StyleQuery.AspectRatioRange:r:w", "StyleQuery.ConditionsSerialize:-:w", "StyleQuery.MaxSize:2:w", "StyleQuery.MinSize:2:w", "StyleQuery.PreferredInput:ePreferredInput:w", "StyleQuery.PreferredTextSize:ePreferredTextSize:w", "StyleQuery.ReducedMotionEnabled:b:w", "StyleQuery.ViewportDisplaySize:eDisplaySize:w" } },
	{ "SurfaceAppearance", false, { "SurfaceAppearance.ColorMap:s:w", "SurfaceAppearance.MetalnessMap:s:w", "SurfaceAppearance.NormalMap:s:w", "SurfaceAppearance.RoughnessMap:s:w", "SurfaceAppearance.TexturePack:s:w", "SurfaceAppearance.TexturePackContent:t:w", "SurfaceAppearance.ColorMapContent:t:w", "SurfaceAppearance.EmissiveMaskContent:t:w", "SurfaceAppearance.MetalnessMapContent:t:w", "SurfaceAppearance.NormalMapContent:t:w", "SurfaceAppearance.RoughnessMapContent:t:w" } },
	{ "SurfaceGui", false, { "SurfaceGui.HorizontalCurvature:n:w", "SurfaceGui.Shape:eSurfaceGuiShape:w" } },
	{ "Team", false, { "Team.ChildOrder:n:w" } },
	{ "Terrain", true, { "Terrain.AcquisitionMethod:eTerrainAcquisitionMethod:w", "Terrain.ExpandedTerrainResolved:b:w", "Terrain.LastUsedModificationMethod:eTerrainAcquisitionMethod:w", "Terrain.SmoothVoxelsUpgraded:b:w", "Terrain.VoxelGridAssetContentMap:-:w", "Terrain.Decoration:b:w", "Terrain.GrassLength:n:w" } },
	{ "TerrainDetail", false, { "TerrainDetail.ColorMap:s:w", "TerrainDetail.ColorMapContent:t:w", "TerrainDetail.EmissiveMaskContent:t:w", "TerrainDetail.MetalnessMap:s:w", "TerrainDetail.MetalnessMapContent:t:w", "TerrainDetail.NormalMap:s:w", "TerrainDetail.NormalMapContent:t:w", "TerrainDetail.RoughnessMap:s:w", "TerrainDetail.RoughnessMapContent:t:w" } },
	{ "TextBox", false, { "TextBox.Confidential:b:w", "TextBox.HasFocus:b:w", "TextBox.LocalizationMatchIdentifier:s:w", "TextBox.LocalizationMatchedSourceText:s:w", "TextBox.LocalizedPlaceholderText:s:w", "TextBox.ManualFocusRelease:b:w", "TextBox.OverlayNativeInput:b:w", "TextBox.ReturnKeyType:eReturnKeyType:w", "TextBox.ShouldEmitReturnEvents:b:w", "TextBox.ShouldEmitTabEvents:b:w", "TextBox.ShouldEmitUpAndDownArrowEvents:b:w", "TextBox.TextInputType:eTextInputType:w" } },
	{ "TextButton", false, { "TextButton.Confidential:b:w", "TextButton.LocalizationMatchIdentifier:s:w", "TextButton.LocalizationMatchedSourceText:s:w" } },
	{ "TextChannel", false, { "TextChannel.IsDefaultTextChannel:b:w" } },
	{ "TextChannelWindow", false, { "TextChannelWindow.IsRendering:b:w" } },
	{ "TextChatService", true, { "TextChatService.ChatTranslationFTUXShown:b:w", "TextChatService.ChatTranslationToggleEnabled:b:w", "TextChatService.HasSeenDeprecationDialog:b:w", "TextChatService.IsLegacyChatDisabled:b:w", "TextChatService.PlatformIntegratedChat:eRolloutState:w", "TextChatService.ChatTranslationEnabled:b:w", "TextChatService.ChatVersion:eChatVersion:w", "TextChatService.CreateDefaultCommands:b:w", "TextChatService.CreateDefaultTextChannels:b:w" } },
	{ "TextDocument", false, { "TextDocument.TextContent:t:w" } },
	{ "TextLabel", false, { "TextLabel.Confidential:b:w", "TextLabel.LocalizationMatchIdentifier:s:w", "TextLabel.LocalizationMatchedSourceText:s:w" } },
	{ "UserInputService", true, { "UserInputService.BottomBarSize:2:w", "UserInputService.LegacyInputEventsEnabled:b:w", "UserInputService.NavBarSize:2:w", "UserInputService.OnScreenKeyboardAnimationDuration:n:w", "UserInputService.OverrideMouseIconBehavior:eOverrideMouseIconBehavior:w", "UserInputService.RightBarSize:2:w", "UserInputService.StatusBarSize:2:w", "UserInputService.TouchScreenEnabled:b:w" } },
	{ "VRService", true, { "VRService.DidPointerHit:b:w", "VRService.LaserDistance:n:w", "VRService.PointerHitCFrame:f:w", "VRService.PresentationMode:eVRPresentationMode:w", "VRService.QuestASWState:b:w", "VRService.QuestDisplayRefreshRate:n:w", "VRService.VRDeviceAvailable:b:w", "VRService.VRDeviceName:s:w", "VRService.VRSessionState:eVRSessionState:w" } },
	{ "VideoCaptureService", true, { "VideoCaptureService.Active:b:w", "VideoCaptureService.CameraID:s:w" } },
	{ "VideoFrame", false, { "VideoFrame.InternalVideoUsage:eInternalVideoUsage:w", "VideoFrame.MaximumResolution:eVideoSampleSize:w" } },
	{ "VideoPlayer", false, { "VideoPlayer.InternalVideoUsage:eInternalVideoUsage:w", "VideoPlayer.MaximumResolution:eVideoSampleSize:w" } },
	{ "ViewportFrame", false, { "ViewportFrame.IsMirrored:b:w" } },
	{ "VirtualInputManager", true, { "VirtualInputManager.AdditionalLuaState:s:w" } },
	{ "VisualizationMode", false, { "VisualizationMode.Enabled:b:w", "VisualizationMode.Title:s:w", "VisualizationMode.ToolTip:s:w" } },
	{ "VisualizationModeCategory", false, { "VisualizationModeCategory.Enabled:b:w", "VisualizationModeCategory.Title:s:w" } },
	{ "VoiceChatInternal", true, { "VoiceChatInternal.VoiceChatState:eVoiceChatState:w" } },
	{ "VoiceChatService", true, { "VoiceChatService.DefaultDistanceAttenuation:eVoiceChatDistanceAttenuationType:w", "VoiceChatService.EnableDefaultVoice:b:w", "VoiceChatService.EnableVoiceVolumeControls:eRolloutState:w", "VoiceChatService.UseAudioApi:eAudioApiRollout:w", "VoiceChatService.UseNewAudioApi:b:w", "VoiceChatService.UseNewControlPaths:b:w", "VoiceChatService.UseNewJoinFlow:b:w", "VoiceChatService.UseStreamSwitching:b:w", "VoiceChatService.VoiceChatEnabledForPlaceOnRcc:b:w", "VoiceChatService.VoiceChatEnabledForUniverseOnRcc:b:w" } },
	{ "Weld", false, { "Weld.EnableSkinning:b:w" } },
	{ "Workspace", true, { "Workspace.InputSinkEnabled:eRolloutState:w", "Workspace.LuauTypeCheckMode:eLuauTypeCheckMode:w", "Workspace.NextGenerationPhysicsSolver:eRolloutState:w", "Workspace.NextGenerationReplicationAlias:eRolloutState:w", "Workspace.NextGenerationReplicationStorage:eRolloutState:w", "Workspace.PlayerScriptsUseInputActionSystem:eRolloutState:w", "Workspace.PlayerScriptsUseInputActionSystemAlias:eRolloutState:w", "Workspace.PlayerScriptsUseInputActionSystemStorage:eRolloutState:w", "Workspace.PredictiveStreamingMode:ePredictiveStreamingMode:w", "Workspace.ReplicationFrequency:eReplicationMode:w", "Workspace.ReplicationThroughput:eReplicationMode:w", "Workspace.SignalBehaviorAlias:eSignalBehavior:w", "Workspace.SignalBehaviorStorage:eSignalBehavior:w", "Workspace.StreamOutBehavior:eStreamOutBehavior:w", "Workspace.StreamingAdaptiveRadius:b:w", "Workspace.StreamingEnabledAlias:b:w", "Workspace.StreamingEnabledStorage:b:w", "Workspace.StreamingIntegrityMode:eStreamingIntegrityMode:w", "Workspace.StreamingMinRadius:n:w", "Workspace.StreamingTargetRadius:n:w", "Workspace.ThrottleLevel:n:w", "Workspace.UseFixedSimulationAlias:eRolloutState:w", "Workspace.UseFixedSimulationStorage:eRolloutState:w", "Workspace.WatermarkHash:s:w", "Workspace.AuthorityMode:eAuthorityMode:w", "Workspace.FallHeightEnabled:b:w", "Workspace.FallenPartsDestroyHeight:n:w", "Workspace.FilteringEnabled:b:w", "Workspace.InterpolationThrottling:eInterpolationThrottlingMode:w", "Workspace.StreamingEnabled:b:w" } },
	{ "WorkspaceAnnotation", false, { "WorkspaceAnnotation.Adornee:i:w", "WorkspaceAnnotation.AdorneeOffset:3:w" } },
	{ "WorldModel", false, { "WorldRoot.AutoSimulate:b:w", "WorldRoot.CollisionGroupData:-:w", "WorldRoot.GravityDirection:3:w", "WorldRoot.PhysicsStepTime:n:w", "WorldRoot.SimulationRate:n:w", "WorldRoot.Wind:n:w", "WorldRoot.WindDirection:3:w" } },
	{ "WrapDeformer", false, { "BaseWrap.HSRAssetId:s:w", "BaseWrap.HSRContent:t:w", "BaseWrap.HSRData:-:w", "BaseWrap.HSRMeshIdData:-:w", "BaseWrap.TemporaryCageMeshContent:t:w", "BaseWrap.TemporaryCageMeshId:s:w", "BaseWrap.CageMeshContent:t:w", "BaseWrap.CageMeshId:s:w", "BaseWrap.CageOrigin:f:w", "BaseWrap.ImportOrigin:f:w" } },
	{ "WrapLayer", false, { "WrapLayer.MaxSize:3:w", "WrapLayer.Offset:3:w", "WrapLayer.TemporaryReferenceId:s:w", "WrapLayer.TemporaryReferenceMeshContent:t:w", "WrapLayer.BindOffset:f:w", "WrapLayer.ReferenceMeshContent:t:w", "WrapLayer.ReferenceMeshId:s:w", "WrapLayer.ReferenceOrigin:f:w", "WrapLayer.RotateScaleDeformationMap:t:w", "WrapLayer.ScaleOnlyDeformationMap:t:w", "WrapLayer.ShrinkFactor:n:w", "WrapLayer.TargetShaping:n:w" } },
	{ "WrapTarget", false, { "WrapTarget.Stiffness:n:w" } },
}
for _, Row in Rows do
	local Made, Service, Properties = Row[1], Row[2], Row[3]
	local Instance_ = Special[Made] or (if Service then game:GetService(Made) else Instance.new(Made))
	for _, Entry in Properties do
		local Key, Code, Writes = string.match(Entry, "^([^:]+):([^:]+):?(w?)$")
		local Name = string.match(Key, "%.(.+)$")
		local Line = { Key }
		local ROk, RValue = pcall(function() return (Instance_ :: any)[Name] end)
		table.insert(Line, "r=" .. (if ROk then "ok:" .. typeof(RValue) else shown(RValue)))
		if Writes == "w" then
			local Has, Valid = false, nil
			if ROk and RValue ~= nil then Has, Valid = true, RValue else Has, Valid = value(Code) end
			if Has then
				local WOk, WProblem = pcall(function() (Instance_ :: any)[Name] = Valid end)
				table.insert(Line, "w=" .. (if WOk then "ok" else shown(WProblem)))
			end
			local BOk, BProblem = pcall(function() (Instance_ :: any)[Name] = {} end)
			table.insert(Line, "b=" .. (if BOk then "ok" else shown(BProblem)))
		end
		if not Service and not Special[Made] then
			local Child = Instance.new("Folder")
			Child.Name = Name
			if pcall(function() Child.Parent = Instance_ end) then
				local COk, CValue = pcall(function() return (Instance_ :: any)[Name] end)
				table.insert(Line, "c=" .. (if COk then (if CValue == Child then "child" else "value") else shown(CValue)))
			end
			Child:Destroy()
		end
		print(table.concat(Line, " "))
	end
end
