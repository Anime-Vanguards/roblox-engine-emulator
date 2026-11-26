-- Every read-only property of the creatable classes (the API dump's properties a script may read but not write, each read
-- on one creatable class that has it, loose and in the Workspace) and of common services: what a fresh instance answers.
-- (Generated from data/reflection.json, 0.747.0.7471481; ClassName left out, and Stats' and ContentProvider's queue figures, which
-- measure the session.)
local function Show(Value)
	local Kind = typeof(Value)
	if Kind == "number" then
		return P.num(Value)
	elseif Kind == "string" then
		return '"' .. P.text(Value) .. '"'
	elseif Kind == "Vector3" then
		return "V3(" .. P.f32s(Value.X, Value.Y, Value.Z) .. ")"
	elseif Kind == "Vector2" then
		return "V2(" .. P.f32s(Value.X, Value.Y) .. ")"
	elseif Kind == "CFrame" then
		return "CF(" .. P.f32s(Value:GetComponents()) .. ")"
	elseif Kind == "Instance" then
		return "Instance(" .. Value.ClassName .. ")"
	elseif Kind == "nil" or Kind == "boolean" then
		return tostring(Value)
	end
	return Kind .. "(" .. (string.gsub(tostring(Value), "0x%x+", "ADDR")) .. ")"
end
local function Read(Holder, Name)
	local Ok, Value = pcall(function()
		return Holder[Name]
	end)
	return if Ok then Show(Value) else "E(" .. P.err(Value) .. ")"
end
local Classes = {
	{ "AdGui", "AdGui", { "Status" } },
	{ "AdPlacement", "AdPlacement", { "RewardId", "RewardImageContent", "RewardName", "Visible" } },
	{ "AdPortal", "AdPortal", { "Status" } },
	{ "AnimationClip", "AnimationGraphDefinition", { "Length" } },
	{ "AnimationConstraint", "AnimationConstraint", { "C0", "C1", "Part0", "Part1" } },
	{ "Animator", "Animator", { "EvaluationThrottled", "RootMotion", "RootMotionWeight" } },
	{ "AtmosphereSensor", "AtmosphereSensor", { "AirDensity", "RelativeWindVelocity" } },
	{ "Attachment", "Attachment", { "WorldRotation" } },
	{ "AudioAnalyzer", "AudioAnalyzer", { "PeakLevel", "RmsLevel" } },
	{ "AudioDeviceInput", "AudioDeviceInput", { "Active" } },
	{ "AudioPlayer", "AudioPlayer", { "IsPlaying", "IsReady", "TimeLength" } },
	{ "AudioRecorder", "AudioRecorder", { "IsRecording", "TimeLength" } },
	{ "AudioSpeechToText", "AudioSpeechToText", { "VoiceDetected" } },
	{ "AudioStreamReader", "AudioStreamReader", { "SampleFormat", "SampleRate" } },
	{ "AudioStreamWriter", "AudioStreamWriter", { "SampleFormat", "SampleRate" } },
	{ "AudioTextToSpeech", "AudioTextToSpeech", { "IsLoaded", "IsPlaying", "TimeLength" } },
	{ "BasePart", "Part", { "AssemblyCenterOfMass", "AssemblyMass", "AssemblyRootPart", "CenterOfMass", "CurrentPhysicalProperties", "ExtentsCFrame", "ExtentsSize", "Mass", "ReceiveAge", "ResizeIncrement", "ResizeableFaces", "SpecificGravity" } },
	{ "BaseScript", "Script", { "RunContext" } },
	{ "BaseWrap", "WrapLayer", { "CageMeshContent", "CageMeshId", "CageOrigin", "CageOriginWorld", "ImportOrigin", "ImportOriginWorld" } },
	{ "BillboardGui", "BillboardGui", { "CurrentDistance" } },
	{ "Bone", "Bone", { "TransformedCFrame", "TransformedWorldCFrame" } },
	{ "Camera", "Camera", { "NearPlaneZ", "ViewportSize" } },
	{ "Constraint", "HingeConstraint", { "Active" } },
	{ "ControllerBase", "AirController", { "Active" } },
	{ "CylindricalConstraint", "CylindricalConstraint", { "CurrentAngle", "WorldRotationAxis" } },
	{ "Decal", "Decal", { "EmissiveMaskContent", "MetalnessMapContent", "NormalMapContent", "RoughnessMapContent" } },
	{ "FeatureMatchingTable", "FeatureMatchingTable", { "SchemaName" } },
	{ "FloatCurve", "FloatCurve", { "Length" } },
	{ "FluidForceSensor", "FluidForceSensor", { "BuoyancyForce", "BuoyancyTorque", "CenterOfPressure", "Force", "Torque" } },
	{ "GuiBase2d", "Frame", { "AbsolutePosition", "AbsoluteRotation", "AbsoluteSize" } },
	{ "GuiObject", "Frame", { "GuiState" } },
	{ "HingeConstraint", "HingeConstraint", { "CurrentAngle" } },
	{ "Humanoid", "Humanoid", { "CollisionType", "FloorMaterial", "MoveDirection", "RootPart", "SeatPart" } },
	{ "ImageButton", "ImageButton", { "IsLoaded" } },
	{ "ImageLabel", "ImageLabel", { "IsLoaded" } },
	{ "InputAction", "InputAction", { "PreferredBinding" } },
	{ "InputActionLabel", "InputActionLabel", { "ResolvedImageContent", "ResolvedText" } },
	{ "JointInstance", "Weld", { "Active" } },
	{ "MarkerCurve", "MarkerCurve", { "Length" } },
	{ "MaterialVariant", "MaterialVariant", { "BaseMaterial" } },
	{ "MeshPart", "MeshPart", { "HasJointOffset", "HasSkinnedMesh", "JointOffset", "MeshContent", "MeshId", "RenderFidelity" } },
	{ "PartOperation", "PartOperation", { "RenderFidelity", "SmoothingAngle", "TriangleCount" } },
	{ "PhysicsActivitySensor", "PhysicsActivitySensor", { "Active" } },
	{ "ProceduralModel", "ProceduralModel", { "GenerationError" } },
	{ "RodConstraint", "RodConstraint", { "CurrentDistance" } },
	{ "RopeConstraint", "RopeConstraint", { "CurrentDistance" } },
	{ "RotationCurve", "RotationCurve", { "Length" } },
	{ "ScrollingFrame", "ScrollingFrame", { "AbsoluteCanvasSize", "AbsoluteWindowSize" } },
	{ "Seat", "Seat", { "Occupant" } },
	{ "SkateboardController", "SkateboardController", { "Steer", "Throttle" } },
	{ "SkateboardPlatform", "SkateboardPlatform", { "Controller", "ControllingHumanoid" } },
	{ "SlidingBallConstraint", "CylindricalConstraint", { "CurrentPosition" } },
	{ "Sound", "Sound", { "IsLoaded", "IsPaused", "IsPlaying", "PlaybackLoudness", "TimeLength", "isPlaying" } },
	{ "SpringConstraint", "SpringConstraint", { "CurrentLength" } },
	{ "StarterGear", "StarterGear", { "Loaded" } },
	{ "StyleQuery", "StyleQuery", { "IsActive" } },
	{ "StyleRule", "StyleRule", { "SelectorError" } },
	{ "SurfaceAppearance", "SurfaceAppearance", { "ColorMapContent", "EmissiveMaskContent", "MetalnessMapContent", "NormalMapContent", "RoughnessMapContent" } },
	{ "TerrainRegion", "TerrainRegion", { "IsSmooth", "SizeInCells" } },
	{ "TextBox", "TextBox", { "ContentText", "OpenTypeFeaturesError", "TextBounds", "TextFits" } },
	{ "TextButton", "TextButton", { "ContentText", "LocalizedText", "OpenTypeFeaturesError", "TextBounds", "TextFits" } },
	{ "TextChannel", "TextChannel", { "DirectChatRequester" } },
	{ "TextLabel", "TextLabel", { "ContentText", "LocalizedText", "OpenTypeFeaturesError", "TextBounds", "TextFits" } },
	{ "TorsionSpringConstraint", "TorsionSpringConstraint", { "CurrentAngle" } },
	{ "TriangleMeshPart", "MeshPart", { "CollisionFidelity", "CollisionPrecision", "FluidFidelity", "MeshSize" } },
	{ "Tween", "Tween", { "Instance", "TweenInfo" } },
	{ "TweenBase", "Tween", { "PlaybackState" } },
	{ "UIGridLayout", "UIGridLayout", { "AbsoluteCellCount", "AbsoluteCellSize" } },
	{ "UIGridStyleLayout", "UIListLayout", { "AbsoluteContentSize" } },
	{ "UIPageLayout", "UIPageLayout", { "CurrentPage" } },
	{ "ValueCurve", "ValueCurve", { "Length", "ValueType" } },
	{ "VehicleSeat", "VehicleSeat", { "AreHingesDetected", "Occupant" } },
	{ "VideoFrame", "VideoFrame", { "IsLoaded", "Resolution", "TimeLength" } },
	{ "VideoPlayer", "VideoPlayer", { "IsLoaded", "IsPlaying", "Resolution", "TimeLength" } },
	{ "WeldConstraint", "WeldConstraint", { "Active" } },
	{ "Wire", "Wire", { "Connected" } },
	{ "WrapLayer", "WrapLayer", { "BindOffset", "ReferenceMeshContent", "ReferenceMeshId", "ReferenceOrigin", "ReferenceOriginWorld", "RotateScaleDeformationMap", "ScaleOnlyDeformationMap", "ShrinkFactor", "TargetShaping" } },
	{ "WrapTarget", "WrapTarget", { "Stiffness" } },
}
local Holder = Instance.new("Model")
Holder.Name = "ReadOnlyHolder"
Holder.Parent = workspace
for _, Entry in Classes do
	local Owner, ClassName, Names = Entry[1], Entry[2], Entry[3]
	local Ok, Loose = pcall(Instance.new, ClassName)
	if not Ok then
		print("readonly " .. Owner .. " " .. ClassName .. " new E(" .. P.err(Loose) .. ")")
		continue
	end
	local Ok2, Placed = pcall(Instance.new, ClassName)
	local Parented = Ok2 and pcall(function()
		Placed.Parent = Holder
	end)
	for _, Name in Names do
		print("readonly " .. Owner .. "." .. Name .. " (" .. ClassName .. ") loose=" .. Read(Loose, Name) .. " world=" .. (if Parented then Read(Placed, Name) else "-"))
	end
end
Holder:Destroy()
local Services = {
	{ "Workspace", { "AuthorityMode", "FallHeightEnabled", "FallenPartsDestroyHeight", "FilteringEnabled", "InterpolationThrottling", "StreamingEnabled", "Terrain" } },
	{ "Players", { "BubbleChat", "ClassicChat", "LocalPlayer", "MaxPlayers", "NumPlayers", "PreferredPlayers", "localPlayer", "numPlayers" } },
	{ "Lighting", { "LightingStyle", "PrioritizeLightingQuality" } },
	{ "UserInputService", { "AccelerometerEnabled", "GamepadEnabled", "GyroscopeEnabled", "KeyboardEnabled", "MouseEnabled", "OnScreenKeyboardPosition", "OnScreenKeyboardSize", "OnScreenKeyboardVisible", "PreferredInput", "TouchEnabled", "UserHeadCFrame", "VREnabled" } },
	{ "StarterPlayer", { "AllowCustomAnimations" } },
	{ "SoundService", { "CharacterSoundsUseNewApi" } },
	{ "HttpService", { "HttpEnabled" } },
	{ "GuiService", { "IsModalDialog", "IsWindows", "MenuIsOpen", "PreferredTextSize", "PreferredTransparency", "ReducedMotionEnabled", "TopbarInset", "ViewportDisplaySize" } },
	{ "ContentProvider", { "BaseUrl" } },
	{ "LocalizationService", { "RobloxLocaleId", "SystemLocaleId" } },
	{ "TextChatService", { "ChatTranslationEnabled", "ChatVersion", "CreateDefaultCommands", "CreateDefaultTextChannels" } },
	{ "VRService", { "ThirdPersonFollowCamEnabled", "VREnabled" } },
	{ "Chat", { "LoadDefaultChat" } },
	{ "Selection", { "SelectionThickness" } },
	{ "TestService", { "Enabled", "ErrorCount", "TestCount", "WarnCount" } },
	{ "Terrain", { "AssemblyCenterOfMass", "AssemblyMass", "AssemblyRootPart", "CenterOfMass", "CurrentPhysicalProperties", "Decoration", "ExtentsCFrame", "ExtentsSize", "GrassLength", "IsSmooth", "Mass", "MaxExtents", "ReceiveAge", "ResizeIncrement", "ResizeableFaces", "SpecificGravity" } },
}
for _, Entry in Services do
	local Ok, Service = pcall(function()
		return if Entry[1] == "Terrain" then workspace.Terrain else game:GetService(Entry[1])
	end)
	if not Ok or not Service then
		print("service " .. Entry[1] .. " absent " .. P.err(Service))
		continue
	end
	for _, Name in Entry[2] do
		print("service " .. Entry[1] .. "." .. Name .. " " .. Read(Service, Name))
	end
end
